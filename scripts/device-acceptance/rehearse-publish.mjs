// End-to-end no-public publish rehearsal on N Android phones.
//
// Launches a no-public rehearsal controller, runs the full picker/sound/caption
// preparation on every scoped phone IN PARALLEL, then reports per-phone per-stage
// results and shuts the controller down. No Post, no Sheet, no paid AI.
//
// Prerequisites (the coordinator owns these):
//   1. A debug build from build-native.py (the exe hash is recorded).
//   2. A prepared scope JSON with deviceScopes (udid, package, expectedAccount,
//      sourceRoot, bundleId, publishFingerprint, soundPolicy).
//      Set allowInstalledRunner:true and allowWarmLaunch:true so the no-public
//      controller can use the existing UiAutomator and bring TikTok to foreground.
//   3. Phones connected via ADB with TikTok installed and UiAutomator running
//      (from a prior production session or manual setup).
//   4. Test media on disk (from tools/no_public_fixture.py or the publish source).
//
// usage: node scripts/device-acceptance/rehearse-publish.mjs <label> <scope.json>
//            --exe <path> [--port 9377] [--out DIR] [--timeout 360] [--ready-wait 120]
import fs from 'node:fs';
import path from 'node:path';
import crypto from 'node:crypto';
import {execFile, spawn} from 'node:child_process';
import {promisify} from 'node:util';
import {MAIN, OUT_ROOT, CONFIG, connect, readJson, writeJson, now, sleep, parseArgs, helpAndExit, resolvePath, check, numberOf} from './ipc.mjs';

const USAGE = `
Chạy thử đăng bài không công khai (no-public) trên N máy Android song song.
  node scripts/device-acceptance/rehearse-publish.mjs <label> <scope.json> --exe <path>
       [--port 9377] [--out DIR] [--timeout 360] [--ready-wait 120]
scope.json: file JSON no-public (activationId, deviceScopes[...]).
Mỗi deviceScope cần: udid, package, expectedAccount, sourceRoot, bundleId,
  publishFingerprint, soundPolicy, allowInstalledRunner, allowWarmLaunch.
Đầu ra: <out>/rehearsal-<label>.json + trạng thái từng máy.
Không Post, không Sheet, không AI trả phí. Toàn bộ draft và media được dọn dẹp.`;

const argv = process.argv.slice(2);
helpAndExit(argv, USAGE);
const args = parseArgs(argv, {options: ['exe', 'port', 'out', 'timeout', 'ready-wait']});
const [label, scopeArg] = args._;
check(/^[A-Za-z0-9][A-Za-z0-9-]{0,63}$/.test(label ?? ''), 'nhãn phải là [A-Za-z0-9][A-Za-z0-9-]');
const exePath = resolvePath(args.exe ?? '');
check(exePath && fs.existsSync(exePath), '--exe bắt buộc, phải trỏ đến file debug exe');
const scopePath = resolvePath(scopeArg ?? '');
check(scopePath && fs.existsSync(scopePath), 'thiếu file scope JSON');
const PORT = Number(args.port ?? 9377);
check(Number.isInteger(PORT) && PORT >= 1024 && PORT <= 65535, 'cổng CDP không hợp lệ');
const OUT = resolvePath(args.out) ?? path.join(OUT_ROOT, 'rehearsal');
const TIMEOUT_S = Number(args.timeout ?? 360);
const READY_WAIT_S = Number(args['ready-wait'] ?? 120);
const reportFile = path.join(OUT, `rehearsal-${label}.json`);
check(!readJson(reportFile), `${reportFile} đã có; chọn nhãn mới`);

const scope = readJson(scopePath);
check(scope?.activationId && Array.isArray(scope.deviceScopes) && scope.deviceScopes.length,
  'scope cần activationId và deviceScopes[]');
const serials = scope.deviceScopes.map(d => d.udid);
check(new Set(serials).size === serials.length, 'serial phải không trùng');
for (const d of scope.deviceScopes) {
  check(d.udid && d.package && d.expectedAccount && d.sourceRoot && d.bundleId,
    `deviceScope ${d.udid} thiếu trường bắt buộc (package/expectedAccount/sourceRoot/bundleId)`);
  check(d.publishFingerprint, `deviceScope ${d.udid} thiếu publishFingerprint`);
  check(d.soundPolicy !== undefined, `deviceScope ${d.udid} thiếu soundPolicy`);
}

const exeHash = crypto.createHash('sha256').update(fs.readFileSync(exePath)).digest('hex');
const run = promisify(execFile);
const alive = pid => { try { process.kill(pid, 0); return true; } catch { return false; } };

// Ensure no existing Riviu controller is running.
const existing = await run('powershell', ['-NoProfile', '-Command',
  "@(Get-CimInstance Win32_Process|Where-Object {$_.Name -like 'riviu*.exe' -and $_.ExecutablePath -notlike '*deps*'}).Count"],
  {timeout: 30000, windowsHide: true, encoding: 'utf8'});
check(existing.stdout.trim() === '0', `đã có riviu*.exe đang chạy (${existing.stdout.trim()}); đóng trước`);

// Prepare the data root.
const dataRoot = path.join(MAIN, 'target', 'no-public-rehearsal', `${label}.run`);
check(!fs.existsSync(dataRoot), `${dataRoot} đã có; chọn nhãn mới`);
fs.mkdirSync(path.dirname(dataRoot), {recursive: true});

// Launch the no-public controller.
const env = {...process.env};
for (const key of ['RIVIU_MOCK_DEVICES', 'RIVIU_MOCK_DATA_DIR', 'RIVIU_UI_SMOKE', 'RIVIU_UI_SMOKE_DIR',
  'WEBVIEW2_USER_DATA_FOLDER', 'WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS', 'WEBVIEW2_BROWSER_EXECUTABLE_FOLDER',
  'WEBVIEW2_PIPE_FOR_SCRIPT_DEBUGGER']) delete env[key];
if ((env.RIVIU_ANDROID_OBSERVATION_MODE ?? 'legacy') !== 'legacy')
  throw new Error('Discovery requires legacy observation mode');
Object.assign(env, {
  RIVIU_ANDROID_OBSERVATION_MODE: 'legacy',
  RIVIU_NO_PUBLIC_REHEARSAL: '1',
  RIVIU_NO_PUBLIC_SCOPE: scopePath,
  RIVIU_NO_PUBLIC_DIR: dataRoot,
  RIVIU_DEV_BACKGROUND: '1',
  RIVIU_DEV_CDP_PORT: String(PORT),
});
const stdoutLog = path.join(OUT, `rehearsal-${label}.stdout.log`);
const stderrLog = path.join(OUT, `rehearsal-${label}.stderr.log`);
fs.mkdirSync(OUT, {recursive: true});
const stdoutFd = fs.openSync(stdoutLog, 'wx');
const stderrFd = fs.openSync(stderrLog, 'wx');
const child = spawn(exePath, [], {env, stdio: ['ignore', stdoutFd, stderrFd], detached: true, windowsHide: true});
fs.closeSync(stdoutFd);
fs.closeSync(stderrFd);
child.unref();

const processRecord = {activationId: scope.activationId, pid: child.pid, exe: exePath, exeSha256: exeHash,
  port: PORT, scope: scopePath, dataRoot, publicEffectsAllowed: false, startedAt: now()};
writeJson(path.join(OUT, `rehearsal-${label}.process.json`), processRecord, true);
console.log(`Controller khởi động: pid=${child.pid}, port=${PORT}`);

// Wait for CDP + phones ready.
let c;
const connectDeadline = Date.now() + READY_WAIT_S * 1000;
while (Date.now() < connectDeadline) {
  try { c = await connect(PORT); break; }
  catch { await sleep(2000); }
  if (!alive(child.pid)) throw new Error('Controller đã thoát trước khi CDP sẵn sàng; xem log');
}
check(c, `Không kết nối được CDP sau ${READY_WAIT_S} s`);

let report;
try {
  // Verify no-public activation.
  const status = await c.invoke('no_public_status');
  check(status.publicEffectsAllowed === false && status.activationId === scope.activationId,
    'Activation không khớp hoặc publicEffectsAllowed !== false');
  console.log(`Activation: ${status.activationId}, scope: ${status.scopeDeviceIds.length} máy`);

  // Wait for phones to reach "ready" (UiAutomator forward attached).
  const waitReady = async () => {
    const deadline = Date.now() + READY_WAIT_S * 1000;
    while (Date.now() < deadline) {
      const devices = await c.invoke('list_devices');
      const ready = serials.filter(s => devices.find(d => d.udid === s && d.status === 'ready'));
      const connected = serials.filter(s => devices.find(d => d.udid === s && (d.status === 'connected' || d.status === 'ready')));
      console.log(`Máy sẵn sàng: ${ready.length}/${serials.length} (kết nối: ${connected.length}/${serials.length})`);
      if (ready.length === serials.length) return devices;
      await sleep(3000);
    }
    const devices = await c.invoke('list_devices');
    return devices;
  };
  const devices = await waitReady();
  const notReady = serials.filter(s => !devices.find(d => d.udid === s && d.status === 'ready'));
  if (notReady.length) console.log(`Cảnh báo: ${notReady.length} máy chưa ready: ${notReady.map(numberOf).join(', ')}`);

  // Verify metadata per phone.
  const metaResults = {};
  for (const udid of serials) {
    try {
      metaResults[udid] = await c.invoke('no_public_metadata', {udid});
    } catch (error) {
      metaResults[udid] = {error: String(error.message ?? error).slice(0, 200)};
    }
  }

  // Send no_public_prepare_publish for all phones in parallel.
  const requestIds = {};
  const startTimes = {};
  for (const udid of serials) requestIds[udid] = crypto.randomUUID();
  console.log(`\nBắt đầu rehearsal ${serials.length} máy song song...`);

  const startPromises = serials.map(async udid => {
    startTimes[udid] = Date.now();
    try {
      return await c.invoke('no_public_prepare_publish', {udid, requestId: requestIds[udid]});
    } catch (error) {
      return {diagnosticCommandError: true, code: error?.code ?? 'Unknown',
        message: String(error?.message ?? error).slice(0, 300)};
    }
  });
  const startResults = await Promise.all(startPromises);
  for (let i = 0; i < serials.length; i++) {
    const udid = serials[i];
    const r = startResults[i];
    if (r.diagnosticCommandError) {
      console.log(`  ${numberOf(udid)}: lỗi khởi tạo - ${r.code}: ${r.message}`);
    } else {
      console.log(`  ${numberOf(udid)}: ${r.state} (phase: ${r.phase})`);
    }
  }

  // Poll all run statuses until settled, tracking phase transitions.
  const phaseTimeline = {};
  for (const udid of serials) phaseTimeline[udid] = [];
  const lastPhase = {};

  const pollDeadline = Date.now() + TIMEOUT_S * 1000;
  const RUNNING = new Set(serials.filter((_, i) => !startResults[i].diagnosticCommandError));
  while (RUNNING.size > 0 && Date.now() < pollDeadline) {
    await sleep(1500);
    for (const udid of [...RUNNING]) {
      try {
        const status = await c.invoke('no_public_run_status', {requestId: requestIds[udid]});
        if (!status) continue;
        if (status.phase !== lastPhase[udid]) {
          const elapsed = Date.now() - startTimes[udid];
          phaseTimeline[udid].push({phase: status.phase, elapsedMs: elapsed, at: now()});
          lastPhase[udid] = status.phase;
        }
        if (status.state !== 'running') {
          RUNNING.delete(udid);
          const elapsed = Date.now() - startTimes[udid];
          console.log(`  ${numberOf(udid)}: ${status.state} (${(elapsed / 1000).toFixed(1)} s)`);
        }
      } catch { /* retry next poll */ }
    }
  }
  if (RUNNING.size) console.log(`Timeout: ${RUNNING.size} máy chưa xong: ${[...RUNNING].map(numberOf).join(', ')}`);

  // Collect final statuses.
  const finalStatuses = {};
  for (const udid of serials) {
    try {
      finalStatuses[udid] = await c.invoke('no_public_run_status', {requestId: requestIds[udid]});
    } catch (error) {
      finalStatuses[udid] = {error: String(error.message ?? error).slice(0, 200)};
    }
  }

  // Build the report.
  const phones = serials.map((udid, i) => {
    const start = startResults[i];
    const final_ = finalStatuses[udid];
    const timeline = phaseTimeline[udid];
    const elapsed = Date.now() - startTimes[udid];

    // Extract stage timing from timeline.
    const stageTiming = {};
    for (let j = 0; j < timeline.length; j++) {
      const entry = timeline[j];
      const next = timeline[j + 1];
      stageTiming[entry.phase] = {
        startMs: entry.elapsedMs,
        durationMs: next ? next.elapsedMs - entry.elapsedMs : elapsed - entry.elapsedMs,
      };
    }

    // Extract outcome details.
    const outcome = final_?.outcome;
    const preparation = outcome?.preparation;
    const sound = preparation?.prepared?.sound;
    const soundRoute = sound?.route ?? sound?.method ?? null;
    const soundMs = stageTiming.preparedBeforePost?.startMs
      ? (stageTiming.preparedBeforePost.startMs - (stageTiming.composerStarting?.startMs ?? 0))
      : null;

    return {
      udid, number: numberOf(udid),
      startError: start.diagnosticCommandError ? {code: start.code, message: start.message} : null,
      metadata: metaResults[udid],
      state: final_?.state ?? 'unknown',
      phase: final_?.phase ?? 'unknown',
      blocker: outcome?.blocker ?? final_?.blocker ?? null,
      failurePhase: outcome?.failurePhase ?? null,
      failureCode: outcome?.failureCode ?? null,
      preflightDetail: outcome?.preflightDetail ?? null,
      mediaCleanup: outcome?.mediaCleanup ?? null,
      cleanupSafe: outcome?.cleanupSafe ?? null,
      account: preparation?.prepared?.account ?? null,
      importId: outcome?.importId ?? null,
      imageCount: preparation?.prepared?.imageCount ?? null,
      soundRoute: soundRoute,
      soundConfirmed: sound?.confirmed ?? null,
      caption: preparation?.prepared?.exactCaption ?? null,
      prePostReproof: preparation?.prepared?.hierarchySha256 ?? null,
      totalMs: elapsed,
      stageTiming,
      timeline,
    };
  });

  report = {
    label, at: now(), scope: scopePath, activation: scope.activationId,
    exe: exePath, exeSha256: exeHash, port: PORT,
    phonesTotal: serials.length,
    preparedAndCleaned: phones.filter(p => p.state === 'preparedAndCleaned').length,
    blocked: phones.filter(p => p.state === 'blocked').length,
    needsAttention: phones.filter(p => p.state === 'needsAttention').length,
    failed: phones.filter(p => p.startError).length,
    publicEffectsAllowed: false,
    phones,
  };
  writeJson(reportFile, report, true);

  // Summary table.
  console.log('\n=== Kết quả tổng hợp ===');
  console.log(`  Tổng: ${serials.length} | Đạt: ${report.preparedAndCleaned} | Blocked: ${report.blocked} | NeedsAttention: ${report.needsAttention} | Lỗi: ${report.failed}`);
  console.log('');
  for (const p of phones) {
    const stages = [];
    if (p.account) stages.push(`tài khoản: ${p.account}`);
    if (p.importId) stages.push(`import: ${p.importId.slice(0, 30)}`);
    if (p.soundRoute) stages.push(`âm thanh: ${p.soundRoute}`);
    if (p.soundConfirmed) stages.push('sound ✓');
    if (p.caption) stages.push(`caption: ${p.caption.slice(0, 40)}`);
    if (p.prePostReproof) stages.push('pre-Post ✓');
    if (p.mediaCleanup) stages.push(`cleanup: ${p.mediaCleanup}`);
    const status = p.startError ? `LỖI: ${p.startError.code}` : p.state;
    const time = `${(p.totalMs / 1000).toFixed(1)} s`;
    const blocker = p.blocker ? ` [${p.blocker}]` : '';
    console.log(`  ${String(p.number).padEnd(20)} ${status.padEnd(22)} ${time.padStart(8)}${blocker}`);
    if (stages.length) console.log(`    ${stages.join(' | ')}`);
    if (p.failureCode) console.log(`    lỗi: ${p.failurePhase} → ${p.failureCode} (${p.preflightDetail ?? ''})`);
  }
} finally {
  // Shutdown the controller gracefully.
  try {
    if (c) {
      console.log('\nĐang đóng controller...');
      await c.invoke('no_public_shutdown', {activationId: scope.activationId});
    }
  } catch { /* page may have closed already */ }
  try { if (c) await c.close().catch(() => {}); } catch { /* ignore */ }
  // Wait for process to exit.
  const exitDeadline = Date.now() + 30000;
  while (alive(child.pid) && Date.now() < exitDeadline) await sleep(1000);
  if (alive(child.pid)) console.log(`Cảnh báo: pid ${child.pid} vẫn chạy sau 30 s`);
  else console.log(`Controller pid ${child.pid} đã thoát`);
}

console.log(`\nBáo cáo: ${reportFile}`);
if (report) {
  if (report.needsAttention > 0 || report.failed > 0) process.exitCode = 1;
} else {
  process.exitCode = 1;
}
