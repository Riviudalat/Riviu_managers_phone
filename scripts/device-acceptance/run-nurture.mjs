// One Nurture (Nuôi) round on the phones in <config>. Promoted from
// target/acceptance-20261010/kit/run-nurture.mjs.
//   1. nurture_get_settings -> <out>/nurture-settings-before.json (written once, never overwritten;
//      apiKey there is the backend sentinel, never the key)
//   2. nurture_save_settings {settings: {...current, ...overrides}, expectedRevision} BEFORE start
//      (under RIVIU_DEV_MANUAL_ACCEPTANCE=1 a running session ignores later settings changes) and
//      read back to confirm
//   3. <out>/intent.json (exclusive) -> nurture_start {udids, durationMinutes}
//   4. every pollSeconds: nurture_session_status + nurture_session_log -> <out>/poll-NNN.json,
//      <out>/log-<udid>.json, until every started phone stopped or maxWatchMinutes
//   5. nurture_stop; final status; restore the original settings when the revision has not moved
// Never-replay: an existing <out>/intent.json means this only watches/stops/restores.
import fs from 'node:fs';
import path from 'node:path';
import {DEFAULT_PORT, connect, readJson, writeJson, sleep, now, parseArgs, helpAndExit, resolvePath, check} from './ipc.mjs';

const USAGE = `
Chạy một vòng Nuôi và theo dõi tới khi xong, rồi dừng và khôi phục cấu hình.
  node scripts/device-acceptance/run-nurture.mjs --config <nurture.json> --out <dir>
       [--fleet-scope <scope.json>] [--exclude-scope <scope.json>] [--port 47182]
  (NURTURE_OUT=<dir> thay được --out.) Mẫu config: examples/nurture.example.json.
Nếu <out>/intent.json đã có thì chỉ theo dõi/dừng/khôi phục, không lưu cấu hình và không start lại.`;

const argv = process.argv.slice(2);
helpAndExit(argv, USAGE);
const args = parseArgs(argv, {options: ['config', 'out', 'fleet-scope', 'exclude-scope', 'port']});
const config = readJson(resolvePath(args.config ?? ''));
check(config, 'thiếu --config');
const outArg = args.out ?? process.env.NURTURE_OUT;
check(outArg, 'thiếu --out (thư mục riêng cho vòng này)');
const OUT = resolvePath(outArg);
const udids = config.udids;
check(Array.isArray(udids) && udids.length && new Set(udids).size === udids.length, 'udids phải khác rỗng và không trùng');
const fleet = args['fleet-scope'] ? readJson(resolvePath(args['fleet-scope'])) : null;
if (fleet) check(udids.every(u => fleet.deviceIds.includes(u)), 'có udid nuôi nằm ngoài fleet scope');
const exclude = args['exclude-scope'] ? readJson(resolvePath(args['exclude-scope'])) : null;
if (exclude) check(udids.every(u => !exclude.deviceIds.includes(u)), 'udid nuôi trùng scope bị loại trừ (vd. scope publish)');
const minutes = config.durationMinutes, pollMs = config.pollSeconds * 1000, maxMs = config.maxWatchMinutes * 60000;
check(Number.isInteger(minutes) && minutes > 0 && minutes <= 60, 'durationMinutes 1..60');
check(pollMs >= 5000 && maxMs > 0, 'pollSeconds >= 5 và maxWatchMinutes > 0');
fs.mkdirSync(OUT, {recursive: true});
const beforeFile = path.join(OUT, 'nurture-settings-before.json');
const intentFile = path.join(OUT, 'intent.json');
const runFile = path.join(OUT, 'run.json');
const run = readJson(runFile) ?? {createdAt: now()};
const save = () => writeJson(runFile, run);

const c = await connect(args.port ?? DEFAULT_PORT);
try {
  let intent = readJson(intentFile);
  if (!intent) {
    const status0 = await c.invoke('nurture_session_status');
    const busy = status0.filter(s => udids.includes(s.udid) && s.running);
    check(busy.length === 0, `nurture đang chạy trên ${busy.map(s => s.udid).join(',')}`);
    const current = await c.invoke('nurture_get_settings');
    if (!fs.existsSync(beforeFile)) writeJson(beforeFile, current, true);
    const unknown = Object.keys(config.settings).filter(k => !Object.hasOwn(current, k));
    check(unknown.length === 0, `khóa settings không có trong NurtureSettings: ${unknown.join(',')}`);
    const settings = {...current, ...config.settings};
    const saved = await c.invoke('nurture_save_settings', {settings, expectedRevision: current.revision});
    const readBack = await c.invoke('nurture_get_settings');
    const drift = Object.entries(config.settings).filter(([k, v]) => JSON.stringify(readBack[k]) !== JSON.stringify(v)).map(([k]) => k);
    run.saved = {revisionBefore: current.revision, revisionAfter: saved.revision, readBackRevision: readBack.revision, drift, at: now()};
    save();
    check(drift.length === 0 && readBack.revision === saved.revision, `cấu hình đã lưu đọc lại không khớp: ${drift.join(',')}`);
    intent = {udids, durationMinutes: minutes, settingsRevision: saved.revision, at: now()};
    writeJson(intentFile, intent, true);
    try {
      run.started = await c.invoke('nurture_start', {udids, durationMinutes: minutes});
      run.start = 'acknowledged';
    } catch (error) {
      run.start = 'ackUnknownNeverReplay';
      run.startError = String(error.message ?? error);
    }
    run.startReturnedAt = now();
    save();
  } else {
    run.resumedWatchAt = now();
    save();
  }
  const startedAt = Date.parse(intent.at);
  const tracked = Array.isArray(run.started) && run.started.length ? run.started : intent.udids;
  const seenRunning = new Set(run.seenRunning ?? []);
  const deadline = startedAt + maxMs;
  let poll = run.polls ?? 0;
  for (;;) {
    let rows = [];
    try {
      rows = (await c.invoke('nurture_session_status')).filter(s => udids.includes(s.udid));
      for (const udid of udids) writeJson(path.join(OUT, `log-${udid}.json`), await c.invoke('nurture_session_log', {udid}));
      writeJson(path.join(OUT, `poll-${String(++poll).padStart(3, '0')}.json`), {at: now(), status: rows});
    } catch (error) {
      run.lastReadError = {at: now(), error: String(error.message ?? error)};
    }
    for (const s of rows) if (s.running) seenRunning.add(s.udid);
    const finished = udid => rows.some(s => s.udid === udid && !s.running
      && (seenRunning.has(udid) || (s.startedAt && Date.parse(s.startedAt) >= startedAt - 60000)));
    run.polls = poll; run.seenRunning = [...seenRunning];
    run.progress = rows.map(s => ({udid: s.udid, running: s.running, videos: s.videosDone, likes: s.likes, saves: s.saves,
      comments: s.comments, follows: s.follows, phase: s.phase, outcome: s.outcome, last: s.lastMessage?.slice(0, 120)}));
    save();
    console.log(JSON.stringify({at: now(), poll, running: rows.filter(s => s.running).length,
      done: tracked.filter(finished).length, of: tracked.length}));
    if (tracked.every(finished) || Date.now() >= deadline) break;
    await sleep(pollMs);
  }
  run.stopRequestedAt = now();
  try { await c.invoke('nurture_stop', {udids}); run.stop = 'acknowledged'; }
  catch (error) { run.stop = 'error'; run.stopError = String(error.message ?? error); }
  save();
  await sleep(15000);
  run.finalStatus = (await c.invoke('nurture_session_status')).filter(s => udids.includes(s.udid));
  writeJson(path.join(OUT, 'final-status.json'), run.finalStatus);
  save();
  // Restore only when nothing is still running here and nobody else changed the settings row.
  const before = readJson(beforeFile);
  const latest = await c.invoke('nurture_get_settings');
  if (run.finalStatus.some(s => s.running)) {
    run.restore = 'skippedStillRunning; chạy restore-nurture.mjs sau khi máy dừng';
  } else if (latest.revision !== intent.settingsRevision) {
    run.restore = `skippedRevisionMoved ${intent.settingsRevision}->${latest.revision}; xem lại rồi chạy restore-nurture.mjs`;
  } else {
    const restored = await c.invoke('nurture_save_settings', {settings: before, expectedRevision: latest.revision});
    run.restore = {revision: restored.revision, at: now()};
  }
  save();
  console.log(JSON.stringify({start: run.start, started: run.started, stop: run.stop, restore: run.restore,
    final: run.finalStatus.map(s => ({udid: s.udid, running: s.running, videos: s.videosDone, likes: s.likes, saves: s.saves, outcome: s.outcome}))}));
} finally { await c.close(); }
