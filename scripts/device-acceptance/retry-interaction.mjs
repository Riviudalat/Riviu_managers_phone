// Retry interaction assignments that failed BEFORE any effect. Promoted from
// target/acceptance-20261010/kit/retry-interaction.mjs and wait-then-retry.sh.
//
// Two filters, both required: the backend's own (retryable_assignments never re-sends
// Succeeded, Uncertain or Sending; interaction_campaign.rs) and this script's, which keeps only
// `failed` rows whose errorCode matches a pre-effect message (--match). A row that may have
// tapped anything is never selected. With --wait-idle the script first waits until the
// campaign is no longer `running`; it refuses to retry a running campaign.
import path from 'node:path';
import {DEFAULT_PORT, invokeOnce, writeJson, now, sleep, parseArgs, helpAndExit, resolvePath, check} from './ipc.mjs';

const PRE_EFFECT = 'chưa tương tác|Chưa chứng minh được tài khoản';
const USAGE = `
Thử lại các dòng Tương tác hỏng TRƯỚC effect (không bao giờ dòng đã gửi/chưa rõ).
  node scripts/device-acceptance/retry-interaction.mjs <campaignId> --out <dir> [--match <regex>]
       [--wait-idle <giây>] [--dry-run] [--port 47182]
--match mặc định: '${PRE_EFFECT}'. Chỉ thêm mẫu khi đã chứng minh lỗi đó xảy ra trước mọi tap.
Ghi <out>/retry-intent-<ms>.json (exclusive) trước interaction_retry.`;

const argv = process.argv.slice(2);
helpAndExit(argv, USAGE);
const args = parseArgs(argv, {flags: ['dry-run'], options: ['out', 'match', 'wait-idle', 'port']});
const [campaignId] = args._;
check(/^[A-Za-z0-9_-]{1,128}$/.test(campaignId ?? ''), 'campaignId không hợp lệ');
check(args.out, 'thiếu --out');
const port = args.port ?? DEFAULT_PORT;
const match = new RegExp(args.match ?? PRE_EFFECT);
const waitSeconds = Number(args['wait-idle'] ?? 0);
check(Number.isInteger(waitSeconds) && waitSeconds >= 0 && waitSeconds <= 7200, '--wait-idle 0..7200');

let detail = await invokeOnce('interaction_get', {campaignId}, port);
const deadline = Date.now() + waitSeconds * 1000;
while (detail.summary?.state === 'running' && Date.now() < deadline) {
  await sleep(20000);
  detail = await invokeOnce('interaction_get', {campaignId}, port);
}
check(detail.summary?.state !== 'running', 'chiến dịch còn running; không retry (dùng --wait-idle)');
const ids = detail.assignments.filter(a => a.state === 'failed' && match.test(a.errorCode ?? '')).map(a => a.id);
console.log(JSON.stringify({state: detail.summary?.state, selected: ids.length, match: match.source}));
if (args['dry-run'] || !ids.length) process.exit(0);
writeJson(path.join(resolvePath(args.out), `retry-intent-${Date.now()}.json`), {at: now(), campaignId, match: match.source, ids}, true);
let res;
try { res = {ok: true, value: await invokeOnce('interaction_retry', {campaignId, assignmentIds: ids}, port)}; }
catch (e) { res = {ok: false, ackUnknownNeverReplay: true, error: String(e.message ?? e)}; }
console.log(JSON.stringify({retried: ids.length, res}));
