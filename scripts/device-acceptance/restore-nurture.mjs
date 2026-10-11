// Restore the nurture settings saved by run-nurture.mjs (<out>/nurture-settings-before.json).
// Refuses while any nurture session is running. Saves against the latest revision (CAS).
// Promoted from target/acceptance-20261010/kit/restore-nurture.mjs.
import path from 'node:path';
import {DEFAULT_PORT, connect, readJson, writeJson, now, parseArgs, helpAndExit, resolvePath, check} from './ipc.mjs';

const USAGE = `
Khôi phục cấu hình Nuôi đã lưu trước vòng chạy.
  node scripts/device-acceptance/restore-nurture.mjs --out <dir của run-nurture> [--port 47182]
Từ chối khi còn phiên Nuôi đang chạy.`;

const argv = process.argv.slice(2);
helpAndExit(argv, USAGE);
const args = parseArgs(argv, {options: ['out', 'port']});
check(args.out, 'thiếu --out');
const OUT = resolvePath(args.out);
const before = readJson(path.join(OUT, 'nurture-settings-before.json'));
check(before, `${OUT}/nurture-settings-before.json không có; không có gì để khôi phục`);
const c = await connect(args.port ?? DEFAULT_PORT);
try {
  const running = (await c.invoke('nurture_session_status')).filter(s => s.running).map(s => s.udid);
  check(!running.length, `nurture còn chạy trên ${running.join(',')}; dừng trước`);
  const latest = await c.invoke('nurture_get_settings');
  const restored = await c.invoke('nurture_save_settings', {settings: before, expectedRevision: latest.revision});
  const keys = ['numVideos', 'numRounds', 'likeEnabled', 'likeProb', 'saveEnabled', 'saveProb', 'commentEnabled', 'commentProb',
    'followEnabled', 'followProb', 'frenzyEnabled', 'frenzyProb', 'watchMin', 'watchMax', 'scheduleEnabled', 'staggerDelayMin', 'staggerDelayMax', 'feedSource'];
  const drift = keys.filter(k => JSON.stringify(restored[k]) !== JSON.stringify(before[k]));
  const record = {at: now(), fromRevision: latest.revision, toRevision: restored.revision, drift};
  writeJson(path.join(OUT, `restore-${Date.now()}.json`), record);
  console.log(JSON.stringify(record));
  if (drift.length) process.exitCode = 1;
} finally { await c.close(); }
