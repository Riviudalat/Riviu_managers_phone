// Retry publish assignments that failed BEFORE dispatch. Promoted from
// target/acceptance-20261010/kit/retry-campaign.p47182.mjs.
//
// A row is eligible only when ALL hold, read fresh from the controller:
//   state === 'failedBeforeDispatch', effectIntent == null, its ordinal and udid match the
//   frozen scope, and publish_recovery_capabilities says retryBeforePost.allowed.
// One ineligible row refuses the whole batch before anything is sent. The intent file is written
// exclusively BEFORE publish_retry_assignment; each row's result file is exclusive too.
// `uncertain`, `verifying` or any row with an effect intent is never retried: observe it.
import path from 'node:path';
import {randomUUID} from 'node:crypto';
import {DEFAULT_PORT, connect, readJson, writeJson, now, parseArgs, helpAndExit, resolvePath, check} from './ipc.mjs';

const USAGE = `
Thử lại dòng Đăng bài hỏng trước khi gửi (failedBeforeDispatch, chưa có effect intent).
  node scripts/device-acceptance/retry-publish.mjs <campaignId> <số thứ tự 1-based,...> <label>
       --scope <device-scope.json> --out <dir> [--port 47182]
Số thứ tự là vị trí trong scope (máy 1 = deviceIds[0]); mẫu số là số máy của scope.
Một dòng không đủ điều kiện thì không gửi dòng nào.`;

const argv = process.argv.slice(2);
helpAndExit(argv, USAGE);
const args = parseArgs(argv, {options: ['scope', 'out', 'port']});
const [campaignId, numbers, label] = args._;
check(/^[A-Za-z0-9_-]{1,128}$/.test(campaignId ?? ''), 'campaignId không hợp lệ');
check(/^[a-z0-9-]+$/.test(label ?? ''), 'nhãn phải là [a-z0-9-]');
check(args.out, 'thiếu --out');
const scope = readJson(resolvePath(args.scope ?? ''));
check(scope?.deviceIds?.length, 'thiếu --scope');
const total = scope.deviceIds.length;
const requested = (numbers ?? '').split(',').map(Number);
check(requested.every(n => Number.isInteger(n) && n >= 1 && n <= total) && new Set(requested).size === requested.length,
  'danh sách số thứ tự không hợp lệ');
check(scope.campaignIds?.[0] === campaignId, 'scope đã đổi: campaignIds[0] khác campaignId');
const OUT = resolvePath(args.out);

const c = await connect(args.port ?? DEFAULT_PORT);
try {
  const detail = await c.invoke('publish_get', {campaignId});
  const capabilities = await c.invoke('publish_recovery_capabilities', {campaignId});
  const intents = requested.map(number => {
    const row = detail.assignments[number - 1];
    const cap = capabilities.find(x => x.assignmentId === row?.id);
    check(row && row.ordinal === number - 1 && row.udid === scope.deviceIds[number - 1] && row.state === 'failedBeforeDispatch'
      && row.effectIntent == null && cap?.retryBeforePost?.allowed === true, `Máy ${number} không đủ điều kiện; không gửi retry nào`);
    return {number, udid: row.udid, assignmentId: row.id, expectedRevision: cap.revision, requestId: randomUUID(), confirmed: true};
  });
  writeJson(path.join(OUT, `retry-${label}-intent.json`), {at: now(), campaignId, frozenDenominator: total, intents}, true);
  const rows = await Promise.all(intents.map(async input => {
    let result;
    try {
      result = {ok: true, value: await c.invoke('publish_retry_assignment', {assignmentId: input.assignmentId,
        expectedRevision: input.expectedRevision, requestId: input.requestId, confirmed: true})};
    } catch (error) { result = {ok: false, ackUnknownNeverReplay: true, error: String(error.message ?? error)}; }
    const row = {...input, result, at: now()};
    writeJson(path.join(OUT, `retry-${label}-${input.number}.json`), row, true);
    return row;
  }));
  writeJson(path.join(OUT, `retry-${label}-receipt.json`), {campaignId, frozenDenominator: total, rows});
  console.log(JSON.stringify(rows.map(r => ({number: r.number, ok: r.result.ok, error: r.result.error}))));
} finally { await c.close(); }
