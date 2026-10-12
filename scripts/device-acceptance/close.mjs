// Graceful close of the acceptance controller, then wait for its process to exit. Never kills.
// Promoted from target/acceptance-20261010/kit/close.p47182.mjs.
//
// Refuses (no close) when:
//   * a named publish campaign is still composing/transferring (a state outside DRAINED), or
//   * any device owner other than the publish Script owner is active (nurture/interaction/manual...), or
//   * a nurture session still reports running.
// Never force-stops a composer; the app's graceful shutdown drains retained Script contexts.
import fs from 'node:fs';
import path from 'node:path';
import {OUT_ROOT, DEFAULT_PORT, connect, readJson, writeJson, now, sleep, parseArgs, helpAndExit, resolvePath, check} from './ipc.mjs';

const USAGE = `
Đóng controller nghiệm thu một cách graceful rồi chờ process thoát (không bao giờ kill).
  node scripts/device-acceptance/close.mjs <label> [campaignId,...] [--publish-dir DIR] [--port 47182]
       [--out-dir DIR] [--controller controller-<label>.json] [--wait-seconds 300]
campaignId mặc định lấy requestId trong <publish-dir>/start-intent.json.
Từ chối khi publish còn compose/transfer, còn owner khác 'script', hoặc nurture còn chạy.
Ghi owners-before-<label>-close.json, campaigns-before-<label>-close.json, close-for-<label>.json (exclusive).`;

const argv = process.argv.slice(2);
helpAndExit(argv, USAGE);
const args = parseArgs(argv, {options: ['publish-dir', 'port', 'out-dir', 'controller', 'wait-seconds']});
const [label, ids] = args._;
check(/^[a-z0-9-]+$/.test(label ?? ''), 'nhãn phải là [a-z0-9-]');
const port = Number(args.port ?? DEFAULT_PORT);
const OUT = resolvePath(args['out-dir']) ?? path.join(OUT_ROOT, 'controllers');
function discoverCampaignIds(publishDir) {
  if (!publishDir) return [];
  const dir = resolvePath(publishDir);
  const intent = readJson(path.join(dir, 'start-intent.json'));
  if (intent?.requestId) return [intent.requestId];
  const campaign = readJson(path.join(dir, 'campaign.json'));
  if (campaign?.id) return [campaign.id];
  const preflight = readJson(path.join(dir, 'preflight.json'));
  if (preflight?.approval?.requestId) return [preflight.approval.requestId];
  return [];
}
const campaignIds = ids ? ids.split(',') : discoverCampaignIds(args['publish-dir']);
// The start intent, when the publish dir has one, gives the requested phone count.
const intent = args['publish-dir'] ? readJson(path.join(resolvePath(args['publish-dir']), 'start-intent.json')) : null;
check(campaignIds.every(id => /^[A-Za-z0-9_-]{1,128}$/.test(id)), 'campaign id không hợp lệ');
const waitSeconds = Number(args['wait-seconds'] ?? 300);
check(Number.isInteger(waitSeconds) && waitSeconds >= 0 && waitSeconds <= 3600, '--wait-seconds 0..3600');

/** The controller record whose pid we wait on: explicit, else the newest one on this port. */
function controllerRecord() {
  if (args.controller) return readJson(resolvePath(args.controller));
  if (!fs.existsSync(OUT)) return null;
  const records = fs.readdirSync(OUT).filter(f => /^controller-.*\.json$/.test(f))
    .map(f => ({f, at: fs.statSync(path.join(OUT, f)).mtimeMs})).sort((a, b) => b.at - a.at)
    .map(({f}) => readJson(path.join(OUT, f))).filter(r => r?.port === port);
  return records[0] ?? null;
}
const alive = pid => { try { process.kill(pid, 0); return true; } catch (e) { return e.code === 'EPERM'; } };

const DRAINED = ['failedBeforeDispatch', 'succeeded', 'verifying', 'uncertain', 'queued', 'cancelled', 'missed'];
const record = controllerRecord();
const c = await connect(port);
try {
  const campaigns = [];
  for (const campaignId of campaignIds) {
    const detail = await c.invoke('publish_get', {campaignId});
    check(detail, `publish_get không trả gì cho ${campaignId}`);
    const expected = intent?.requestId === campaignId ? intent.request.udids.length : detail.assignments.length;
    const active = detail.assignments.filter(r => !DRAINED.includes(r.state));
    campaigns.push({campaignId, assignments: detail.assignments.length, expected,
      states: detail.assignments.map(r => ({udid: r.udid, state: r.state, errorCode: r.errorCode}))});
    check(detail.assignments.length === expected && !active.length,
      `Chiến dịch ${campaignId}: còn compose/transfer (${active.map(r => r.state).join(',')}); không đóng`);
  }
  const owners = await c.invoke('list_device_work_states');
  const nurture = await c.invoke('nurture_session_status');
  writeJson(path.join(OUT, `owners-before-${label}-close.json`), owners);
  writeJson(path.join(OUT, `campaigns-before-${label}-close.json`), campaigns);
  check(!owners.some(r => r.currentOwner && r.currentOwner !== 'script'), 'Còn owner khác chiến dịch đang giữ máy; không đóng');
  check(!nurture.some(s => s.running), 'Nurture còn chạy; dừng bằng run-nurture.mjs trước; không đóng');
  // Intent before effect: the record exists even if the page dies while closing.
  writeJson(path.join(OUT, `close-for-${label}.json`), {at: now(), requested: true, campaignIds, postIntentsPreserved: true,
    deviceOwnersBeforeDrain: owners.filter(r => r.currentOwner), gracefulDrainRequired: true, pid: record?.pid ?? null}, true);
  try { await c.invoke('plugin:window|close', {label: 'main'}); }
  catch (error) { console.log('ACK đóng chưa rõ (trang có thể đã tắt): ' + String(error.message ?? error).slice(0, 160)); }
  console.log('Đã yêu cầu đóng graceful; chiến dịch và kết quả đã gửi vẫn được giữ');
} finally { await c.close().catch(() => {}); }

if (!record?.pid) {
  console.log('Không có bản ghi controller cho cổng này: tự kiểm process đã thoát trước khi launch lại.');
} else {
  const deadline = Date.now() + waitSeconds * 1000;
  while (alive(record.pid) && Date.now() < deadline) await sleep(2000);
  if (alive(record.pid)) {
    console.log(`pid ${record.pid} vẫn chạy sau ${waitSeconds} s. KHÔNG kill: chờ thêm hoặc xem log controller.`);
    process.exitCode = 2;
  } else console.log(`pid ${record.pid} đã thoát`);
}
