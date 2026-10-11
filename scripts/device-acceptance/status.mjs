// One-shot status of publish / interaction / nurture on the acceptance controller. Read-only IPC.
// Replaces the per-run target/acceptance-20261010/kit/status-d.sh.
import {DEFAULT_PORT, connect, parseArgs, helpAndExit, numberOf} from './ipc.mjs';

const USAGE = `
Tóm tắt một lần trạng thái Đăng bài / Tương tác / Nuôi (chỉ đọc).
  node scripts/device-acceptance/status.mjs [--publish <campaignId,...>] [--interaction <campaignId,...>] [--port 47182]
Luôn in phiên Nuôi đang chạy và owner đang giữ máy.`;

const argv = process.argv.slice(2);
helpAndExit(argv, USAGE);
const args = parseArgs(argv, {options: ['publish', 'interaction', 'port']});
const list = v => (v ?? '').split(',').filter(Boolean);
const tally = rows => rows.reduce((m, k) => (m[k] = (m[k] ?? 0) + 1, m), {});
const c = await connect(args.port ?? DEFAULT_PORT);
try {
  for (const campaignId of list(args.publish)) {
    const d = await c.invoke('publish_get', {campaignId});
    console.log(`== publish ${campaignId}`, JSON.stringify(tally(d.assignments.map(a => a.state))));
    for (const a of d.assignments) console.log(`  ${numberOf(a.udid)} ${a.state}${a.errorCode ? ' ' + a.errorCode : ''}`);
  }
  for (const campaignId of list(args.interaction)) {
    const d = await c.invoke('interaction_get', {campaignId});
    console.log(`== interaction ${campaignId} ${d.summary?.state} ${d.summary?.errorCode ?? ''}`);
    const counts = tally(d.assignments.map(a => `${a.state} ${(a.errorCode ?? '').slice(0, 90)}`.trim()));
    for (const [k, v] of Object.entries(counts).sort((a, b) => b[1] - a[1])) console.log(`  ${v} × ${k}`);
  }
  const nurture = (await c.invoke('nurture_session_status')).filter(s => s.running);
  console.log(`== nurture đang chạy: ${nurture.length}`);
  for (const s of nurture) console.log(`  ${numberOf(s.udid)} video=${s.videosDone} like=${s.likes} save=${s.saves} ${s.phase ?? ''}`);
  const owners = (await c.invoke('list_device_work_states')).filter(r => r.currentOwner);
  console.log(`== owner đang giữ máy: ${owners.length}`, JSON.stringify(tally(owners.map(r => r.currentOwner))));
} finally { await c.close(); }
