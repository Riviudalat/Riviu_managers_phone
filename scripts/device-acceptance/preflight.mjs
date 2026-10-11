// Per-phone preflight before a device round. READ-ONLY by default.
//
// For every serial in the scope file:
//   1. adb transport state (`adb devices`; the same adb the controller resolves: RIVIU_ADB_PATH,
//      local config `adb`, else PATH. A client of another version restarts the adb server.)
//   2. baseline through the controller (`device_baseline_read`): auto-rotate off, lock screen
//      off. The controller never sends `locksettings` to a phone with a PIN (baseline.rs).
//   3. load (`device_load_states` once, then `device_load_check` per phone: one read-only sample)
//   4. helper maintenance still pending (`agent_helper_maintenance_pending`, read-only)
//   5. focused window (`dumpsys window | grep mCurrentFocus`) to spot a leftover dialog.
//      Screencap is fine; never run `uiautomator dump` while the controller runs.
// Effects only behind explicit flags, each recorded in an intent file (exclusive) first:
//   --apply-baseline         device_baseline_apply {autoRotateOff, lockScreenDisabled} on drifted phones
//   --cleanup-overloaded     device_load_cleanup on phones whose sample is not healthy
//   --helper-recover <s,..>  pending -> prepare -> execute {confirmed, qualifyReplacement} on the named serials
// A phone that fails a check is reported `skipped` with its reason; it stays in the denominator.
import path from 'node:path';
import {execFile} from 'node:child_process';
import {promisify} from 'node:util';
import {OUT_ROOT, DEFAULT_PORT, CONFIG, connect, readJson, writeJson, now, parseArgs, helpAndExit, resolvePath, check, numberOf} from './ipc.mjs';

const USAGE = `
Preflight từng máy trước vòng nghiệm thu (mặc định chỉ đọc).
  node scripts/device-acceptance/preflight.mjs <label> <device-scope.json> [--port 47182] [--out DIR]
       [--expect-focus <regex>] [--apply-baseline] [--cleanup-overloaded] [--helper-recover <serial,...>]
Ghi <out>/preflight-<label>.json (mặc định out: target/device-acceptance/preflight).
Kết quả đọc là trạng thái TRƯỚC effect; sau --apply-baseline/--cleanup/--helper-recover chạy lại với nhãn mới.
Exit 1 nếu có máy không đạt. Máy không đạt ghi 'skipped' kèm lý do, không bỏ khỏi mẫu số.`;

const argv = process.argv.slice(2);
helpAndExit(argv, USAGE);
const args = parseArgs(argv, {flags: ['apply-baseline', 'cleanup-overloaded'],
  options: ['port', 'out', 'expect-focus', 'helper-recover']});
const [label, scopeArg] = args._;
check(/^[A-Za-z0-9-]+$/.test(label ?? ''), 'nhãn phải là [A-Za-z0-9-]');
const scope = readJson(resolvePath(scopeArg ?? ''));
check(Array.isArray(scope?.deviceIds) && scope.deviceIds.length, 'thiếu file scope có deviceIds');
const serials = scope.deviceIds;
const OUT = resolvePath(args.out) ?? path.join(OUT_ROOT, 'preflight');
const reportFile = path.join(OUT, `preflight-${label}.json`);
check(!readJson(reportFile), `${reportFile} đã có; chọn nhãn mới`);
const expectFocus = new RegExp(args['expect-focus'] ?? CONFIG.expectFocus ?? 'com\\.(ss\\.android\\.ugc\\.trill|zhiliaoapp\\.musically|ss\\.android\\.ugc\\.aweme)|[Ll]auncher');
const recover = args['helper-recover'] ? args['helper-recover'].split(',') : [];
check(recover.every(s => serials.includes(s)), '--helper-recover chỉ nhận serial trong scope');
const ADB = process.env.RIVIU_ADB_PATH ?? CONFIG.adb ?? 'adb';
const run = promisify(execFile);
const adb = async (...a) => (await run(ADB, a, {timeout: 20000, windowsHide: true, encoding: 'utf8'})).stdout;
const safe = async fn => { try { return {ok: true, value: await fn()}; } catch (e) { return {ok: false, error: String(e.message ?? e).slice(0, 300)}; } };

const transport = {};
for (const line of (await adb('devices')).split(/\r?\n/).slice(1)) {
  const [serial, state] = line.trim().split(/\s+/);
  if (serial) transport[serial] = state;
}

const c = await connect(args.port ?? DEFAULT_PORT);
const report = {label, at: now(), scope: resolvePath(scopeArg), effects: {applyBaseline: !!args['apply-baseline'],
  cleanupOverloaded: !!args['cleanup-overloaded'], helperRecover: recover}, phones: []};
try {
  const loadStates = await safe(() => c.invoke('device_load_states'));
  for (const udid of serials) {
    const row = {udid, number: numberOf(udid), transport: transport[udid] ?? 'absent', reasons: []};
    if (row.transport !== 'device') row.reasons.push(`adb: ${row.transport}`);
    const baseline = await safe(() => c.invoke('device_baseline_read', {udid}));
    const pick = s => baseline.value?.settings?.find(x => x.setting === s);
    row.baseline = baseline.ok ? {error: baseline.value?.error, rotate: pick('autoRotateOff'), lock: pick('lockScreenDisabled')} : {error: baseline.error};
    for (const [name, r] of [['xoay màn hình', row.baseline.rotate], ['khóa màn hình', row.baseline.lock]])
      if (r?.status !== 'ok') row.reasons.push(`${name}: ${r?.status ?? 'không đọc được'}${r?.detail ? ' (' + r.detail + ')' : ''}`);
    row.loadState = loadStates.value?.find(s => s.udid === udid) ?? null;
    const load = await safe(() => c.invoke('device_load_check', {udid}));
    row.load = load.ok ? {verdict: load.value.verdict, message: load.value.message, sample: load.value.sample} : {error: load.error};
    const kind = load.value?.verdict?.kind;
    if (kind !== 'healthy') row.reasons.push(`tải máy: ${kind ?? 'không đọc được'}${load.value?.verdict?.reasons ? ' (' + load.value.verdict.reasons.join(', ') + ')' : ''}`);
    const pending = await safe(() => c.invoke('agent_helper_maintenance_pending', {udid}));
    row.helperPending = pending.ok ? (pending.value ? {maintenanceId: pending.value.plan.maintenanceId,
      observationOnly: pending.value.observationOnly, submitted: pending.value.plan.submitted.length} : null) : {error: pending.error};
    if (row.helperPending) row.reasons.push('helper: còn bảo trì đang chờ');
    if (row.transport === 'device') {
      const focus = await safe(() => adb('-s', udid, 'shell', 'dumpsys window | grep mCurrentFocus'));
      row.focus = focus.ok ? focus.value.trim().split(/\r?\n/)[0] : `? ${focus.error}`;
      if (!focus.ok || !expectFocus.test(row.focus)) row.reasons.push(`cửa sổ đang focus lạ: ${row.focus}`);
    }
    report.phones.push(row);
  }

  // Effects: only what a flag names, each behind its own exclusive intent.
  const effectsDir = path.join(OUT, `preflight-${label}-effects`);
  if (args['apply-baseline']) {
    for (const row of report.phones) {
      const settings = [['autoRotateOff', row.baseline.rotate], ['lockScreenDisabled', row.baseline.lock]]
        .filter(([, r]) => r?.status === 'drift').map(([s]) => s);
      if (!settings.length || row.transport !== 'device') continue;
      writeJson(path.join(effectsDir, `baseline-${row.udid}.intent.json`), {at: now(), udid: row.udid, settings}, true);
      row.baselineApply = await safe(() => c.invoke('device_baseline_apply', {udid: row.udid, settings}));
    }
  }
  if (args['cleanup-overloaded']) {
    for (const row of report.phones) {
      if (!row.load?.verdict || row.load.verdict.kind === 'healthy' || row.transport !== 'device') continue;
      writeJson(path.join(effectsDir, `cleanup-${row.udid}.intent.json`), {at: now(), udid: row.udid, verdict: row.load.verdict.kind}, true);
      row.cleanup = await safe(() => c.invoke('device_load_cleanup', {udid: row.udid}));
    }
  }
  for (const udid of recover) {
    const row = report.phones.find(r => r.udid === udid);
    const pending = await safe(() => c.invoke('agent_helper_maintenance_pending', {udid}));
    const plan = pending.ok && pending.value ? {ok: true, value: pending.value.plan} : (pending.ok ? await safe(() => c.invoke('agent_helper_maintenance_prepare', {udid})) : pending);
    row.helperRecovery = {plan: plan.ok ? {maintenanceId: plan.value.maintenanceId, submitted: plan.value.submitted.length} : {error: plan.error}};
    if (!plan.ok) continue;
    const reconcileOnly = pending.value?.observationOnly ?? false;
    writeJson(path.join(effectsDir, `helper-${udid}.intent.json`), {at: now(), udid, maintenanceId: plan.value.maintenanceId, reconcileOnly}, true);
    row.helperRecovery.execute = await safe(() => c.invoke('agent_helper_maintenance_execute',
      {plan: plan.value, confirmed: true, reconcileOnly, qualifyReplacement: true}));
  }
} finally { await c.close(); }

for (const row of report.phones) row.verdict = row.reasons.length ? 'skipped' : 'ok';
writeJson(reportFile, report, true);
for (const row of report.phones)
  console.log(`${String(row.number).padEnd(20)} ${row.verdict.padEnd(8)} ${row.reasons.join('; ')}`);
const bad = report.phones.filter(r => r.verdict !== 'ok').length;
console.log(`${report.phones.length - bad}/${report.phones.length} máy đạt. Báo cáo: ${reportFile}`);
if (bad) process.exitCode = 1;
