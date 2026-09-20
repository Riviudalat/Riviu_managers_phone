import fs from 'node:fs';
import path from 'node:path';
import { createHash, randomUUID } from 'node:crypto';
import { pathToFileURL } from 'node:url';
import { setTimeout as delay } from 'node:timers/promises';

const MODES = ['inspect', 'preflight', 'submit', 'observe'];
const FLAGS = ['mode', 'report-dir', 'udids', 'source', 'bundle-ids', 'sheet-id', 'sheet-gid',
  'cdp', 'page-url', 'campaign-id', 'confirm', 'wait-seconds', 'poll-seconds', 'content-snapshot'];
const check = (condition, message) => { if (!condition) throw new Error(message); };
const hash = value => createHash('sha256').update(JSON.stringify(value)).digest('hex');
const handle = value => typeof value === 'string' ? value.trim().replace(/^@/, '').toLowerCase() : '';
const read = file => {
  try { return JSON.parse(fs.readFileSync(file, 'utf8')); }
  catch (error) { if (error.code === 'ENOENT') return null; throw error; }
};
function write(file, value, exclusive = false) {
  const target = exclusive ? file : `${file}.${randomUUID()}.tmp`;
  const fd = fs.openSync(target, 'wx');
  try { fs.writeFileSync(fd, JSON.stringify(value, null, 2) + '\n'); fs.fsyncSync(fd); }
  finally { fs.closeSync(fd); }
  if (!exclusive) fs.renameSync(target, file);
}
function list(value, label) {
  const items = value?.split(',') ?? [];
  check(items.length > 0 && items.length <= 100 && items.every(s => s && s.trim() === s && !/[\r\n\0]/.test(s))
    && new Set(items).size === items.length, `${label}: cần 1–100 ID khác nhau, không để trống`);
  return items;
}
function integer(value, min, max, label) {
  check(/^\d+$/.test(value), `${label}: cần số nguyên`);
  const n = Number(value);
  check(Number.isSafeInteger(n) && n >= min && n <= max, `${label}: ngoài phạm vi ${min}–${max}`);
  return n;
}
export function parseArgs(argv) {
  const flags = {};
  for (let i = 0; i < argv.length; i += 2) {
    const name = argv[i].slice(2), value = argv[i + 1];
    check(argv[i].startsWith('--') && FLAGS.includes(name) && !Object.hasOwn(flags, name)
      && typeof value === 'string' && value.length > 0 && !value.startsWith('--'), `Đối số không hợp lệ: ${argv[i]}`);
    flags[name] = value;
  }
  const mode = flags.mode ?? 'inspect';
  check(MODES.includes(mode), 'mode phải là inspect, preflight, submit hoặc observe');
  check(flags['report-dir'], 'Thiếu --report-dir');
  const cdp = new URL(flags.cdp ?? 'http://127.0.0.1:9277');
  check(cdp.protocol === 'http:' && ['127.0.0.1', '[::1]'].includes(cdp.hostname)
    && cdp.port && cdp.pathname === '/' && !cdp.search && !cdp.hash && !cdp.username && !cdp.password,
  '--cdp chỉ nhận HTTP loopback bằng địa chỉ IP và cổng, không path/credential');
  const options = { mode, reportDir: path.resolve(flags['report-dir']), cdp: cdp.origin,
    pageUrl: flags['page-url'] ?? null, udids: list(flags.udids, '--udids'),
    source: flags.source ? path.resolve(flags.source) : null,
    contentSnapshot: flags['content-snapshot'] ? read(path.resolve(flags['content-snapshot'])) : null,
    bundleIds: flags['bundle-ids'] ? list(flags['bundle-ids'], '--bundle-ids') : [],
    sheetId: flags['sheet-id'] ?? null, sheetGid: flags['sheet-gid'] === undefined ? null
      : integer(flags['sheet-gid'], 0, 2147483647, '--sheet-gid'),
    campaignId: flags['campaign-id'] ?? null, confirm: flags.confirm ?? null,
    waitSeconds: integer(flags['wait-seconds'] ?? '0', 0, 86400, '--wait-seconds'),
    pollSeconds: integer(flags['poll-seconds'] ?? '10', 5, 3600, '--poll-seconds') };
  if (options.pageUrl) {
    const page = new URL(options.pageUrl);
    check(['http:', 'https:'].includes(page.protocol)
      && ['localhost', '127.0.0.1', '[::1]', 'tauri.localhost'].includes(page.hostname)
      && !page.username && !page.password, '--page-url phải là WebView loopback không credential');
  }
  check(!options.sheetId || /^[A-Za-z0-9_-]{1,128}$/.test(options.sheetId), 'Sheet ID không hợp lệ');
  check(Boolean(options.sheetId) === (options.sheetGid !== null), '--sheet-id và --sheet-gid phải đi cùng nhau');
  check(!options.campaignId || /^[A-Za-z0-9_-]{1,128}$/.test(options.campaignId), 'Campaign ID không hợp lệ');
  check(mode === 'submit' ? /^[a-f0-9]{64}$/.test(options.confirm ?? '') : !options.confirm,
    'submit cần --confirm SHA-256 từ preflight; mode khác không nhận --confirm');
  if (mode === 'submit' || mode === 'preflight') {
    check(options.source && options.bundleIds.length === options.udids.length && options.sheetId,
      'preflight/submit cần --source, --bundle-ids một-một với --udids và đích Sheet');
    check(!options.campaignId, 'Không submit/preflight campaign cũ; dùng observe');
  }
  check(mode === 'observe' || (options.waitSeconds === 0 && flags['poll-seconds'] === undefined),
    'Chỉ observe nhận thời gian chờ; chờ không kích hoạt kiểm tra thiết bị');
  return options;
}
function scope(options) {
  return { cdp: options.cdp, pageUrl: options.pageUrl, source: options.source,
    contentSnapshot: options.contentSnapshot, udids: options.udids, bundleIds: options.bundleIds, sheetId: options.sheetId, sheetGid: options.sheetGid };
}
function roster(devices, metas, udids) {
  const ids = [...new Set([...devices.map(d => d.udid), ...metas.map(m => m.udid)])];
  const rows = ids.map(udid => ({ udid, number: metas.find(m => m.udid === udid)?.number ?? null,
    assignedHandle: handle(metas.find(m => m.udid === udid)?.handle),
    name: devices.find(d => d.udid === udid)?.name ?? null,
    status: devices.find(d => d.udid === udid)?.status ?? 'absent',
    platform: devices.find(d => d.udid === udid)?.platform ?? null }));
  return { requestedCount: udids.length, inventoryCount: rows.length,
    selected: udids.map(udid => rows.find(r => r.udid === udid) ?? { udid, number: null, status: 'absent' }),
    excluded: rows.filter(r => !udids.includes(r.udid)).map(r => ({ ...r, reason: 'notRequested' })) };
}
async function checkSheet(invoke, options) {
  const status = await invoke('google_sheets_status');
  const url = `https://docs.google.com/spreadsheets/d/${options.sheetId}/edit#gid=${options.sheetGid}`;
  check(status.connected === true && status.active === true && status.writerId
    && status.selectedFileId === options.sheetId, 'OAuth direct chưa sẵn sàng hoặc sai writer/Sheet');
  // This command may persist validated connection settings. Never called by inspect/observe.
  const result = await invoke('publish_sheet_check', { sheetUrl: url });
  check(result.connectionVerified === true && result.reportingReady === true && result.reportingEpoch
    && result.spreadsheetId === options.sheetId && result.sheetGid === options.sheetGid,
  'Sheet check chưa xác minh đúng target/epoch hoặc đang reset');
  return { writerId: status.writerId, spreadsheetId: result.spreadsheetId,
    sheetGid: result.sheetGid, reportingEpoch: result.reportingEpoch };
}
function validateCampaign(campaign, intent) {
  check(campaign?.id && campaign.requestId === intent.args.requestId, 'Create receipt sai requestId/campaign');
  check(campaign.assignments?.length === intent.args.udids.length
    && campaign.assignments.every((a, i) => a.udid === intent.args.udids[i] && a.ordinal === i
      && a.bundleId === `${intent.args.requestId}:${intent.args.bundleIds[i]}`),
  'Create receipt sai phạm vi máy; không Execute');
}
function parseEvidence(raw) {
  if (!raw) return {};
  try { return JSON.parse(raw); } catch { return {}; }
}
async function summarize(detail, options, invoke, accounts) {
  check(detail?.campaign?.id && Array.isArray(detail.assignments), 'Không đọc được campaign');
  const rows = detail.assignments.map(a => {
    const evidence = parseEvidence(a.evidenceJson), post = evidence.post ?? evidence;
    const canonical = typeof post.postUrl === 'string'
      && /^https:\/\/www\.tiktok\.com\/@[A-Za-z0-9_.]+\/(photo|video)\/\d+$/.test(post.postUrl);
    const verified = post.publicationVerified === true && canonical;
    const assigned = handle(accounts[a.udid]);
    const expected = handle(parseEvidence(a.effectIntent).expectedAccount);
    const author = canonical ? handle(new URL(post.postUrl).pathname.split('/')[1]) : '';
    const accountState = !assigned || !expected ? 'unknown'
      : assigned !== expected || (author && assigned !== author) ? 'mismatch' : 'matched';
    const submitted = verified || post.state === 'submitted' || post.verdict === 'Submitted'
      || post.state === 'posted' || post.verdict === 'Posted';
    const retryInProgress = a.state === 'failedBeforeDispatch' && !a.effectIntent
      && a.dispatch?.phase === 'compose' && ['queued', 'running'].includes(a.dispatch?.state);
    return { assignmentId: a.id, publicationId: a.publicationId ?? null, udid: a.udid,
      state: a.state, error: a.errorCode ?? null, retryInProgress, enqueued: Boolean(a.dispatch), submitted, verified,
      assignedAccount: assigned, submittedAccount: expected, accountState,
      sheetSent: a.sheetDelivery?.state === 'sent', sheetState: a.sheetDelivery?.state ?? 'notReported',
      postUrl: canonical ? post.postUrl : null, urlReadback: 'unsupported',
      verification: evidence.verificationStatus ?? null };
  });
  for (const row of rows.filter(r => r.verified && r.sheetSent)) {
    const assignment = detail.assignments.find(a => a.id === row.assignmentId);
    if (!Number.isSafeInteger(assignment.sheetDelivery?.revision)) continue;
    try {
      const proof = await invoke('publish_sheet_readback', { assignmentId: row.assignmentId, expectedRevision: assignment.sheetDelivery.revision });
      check(proof.assignmentId === row.assignmentId && proof.url === row.postUrl
        && proof.revision === assignment.sheetDelivery.revision && proof.epoch && proof.range && proof.checkedAt
        && proof.receipt?.publicationId === (row.publicationId || row.assignmentId)
        && proof.receipt?.postUrl === row.postUrl && proof.receipt?.revision === proof.revision
        && proof.receipt?.reportingEpoch === proof.epoch, 'Sheet receipt/readback mismatch');
      row.urlReadback = 'matched'; row.sheetProof = proof;
    } catch (error) { row.urlReadback = 'pending'; row.readbackError = String(error); }
  }
  const counts = { requested: options.udids.length, enqueued: rows.filter(r => r.enqueued).length,
    submitted: rows.filter(r => r.submitted).length, verified: rows.filter(r => r.verified).length,
    sheetSent: rows.filter(r => r.sheetSent).length, urlReadback: rows.filter(r => r.urlReadback === 'matched').length };
  const exact = rows.length === options.udids.length && new Set(rows.map(r => r.udid)).size === rows.length
    && rows.every(r => options.udids.includes(r.udid));
  const failed = rows.some(r => (r.state === 'failedBeforeDispatch' && !r.retryInProgress)
    || ['uncertain', 'cancelled', 'missed'].includes(r.state)
    || ['failed', 'superseded'].includes(r.sheetState) || r.verification?.state === 'needsReview'
    || r.accountState === 'mismatch');
  // publish_get exposes settlement state, not the writer's raw ACK/target receipt or Sheet cells.
  // Neither a URL nor 'sent' alone is a supplementary end-to-end readback.
  const allSent = rows.length > 0 && rows.every(r => r.verified && r.sheetSent && r.accountState === 'matched');
  return { rows, counts, campaignId: detail.campaign.id,
    acceptance: exact && !failed && allSent && counts.urlReadback === rows.length ? 'passed' : !exact || failed ? 'blockedFailed' : allSent ? 'readbackUnsupported' : 'pending',
    exitCode: exact && !failed && allSent && counts.urlReadback === rows.length ? 0 : !exact || failed ? 1 : allSent ? 3 : 2,
    scopeMatches: exact, deliveryEvidence: counts.urlReadback === rows.length ? 'canonicalReceiptAndCell' : 'backendSettlementOnly',
    urlReadback: { status: allSent && counts.urlReadback === rows.length ? 'matched' : 'pending', reason: 'Cần canonical post proof, receipt đúng revision/epoch và ô Sheet khớp.' } };
}

/** Same runner for CLI and tests. Only invoke is replaced in tests; files are durable. */
export async function runAcceptance(options, { invoke, sleep = delay, now = Date.now }) {
  fs.mkdirSync(options.reportDir, { recursive: true });
  const file = name => path.join(options.reportDir, name);
  let lock;
  try { lock = fs.openSync(file('harness.lock'), 'wx'); }
  catch (error) { return { exitCode: 1, report: { mode: options.mode, acceptance: 'blockedFailed',
    error: `Report directory đang khóa hoặc không ghi được: ${error.code}; không tự xóa lock` } }; }
  let report = { mode: options.mode, at: new Date(now()).toISOString(), acceptance: 'notEvaluated' };
  let exitCode = 0;
  try {
    fs.writeFileSync(lock, String(process.pid)); fs.fsyncSync(lock);
    const intent = read(file('create-intent.json'));
    const prepared = read(file('preflight.json'));
    if (options.mode === 'submit') {
      check(prepared && hash(prepared.approval) === prepared.confirmation
        && prepared.confirmation === options.confirm, 'Confirmation/preflight không khớp');
      check(hash(scope(options)) === hash(prepared.approval.scope), 'Phạm vi khác preflight đã duyệt');
      if (intent) check(intent.confirmation === options.confirm && hash(intent.args) === intent.requestFingerprint,
        'Create intent không khớp confirmation/fingerprint');
    }
    const devices = await invoke('list_devices');
    const metas = await invoke('list_device_metas');
    report.roster = roster(devices, metas, options.udids);
    const currentAccounts = Object.fromEntries(report.roster.selected.map(r => [r.udid, r.assignedHandle ?? '']));
    const accounts = prepared?.approval.accounts ?? currentAccounts;
    report.accountSnapshot = prepared?.approval.accounts ? 'approvedPreflight' : 'currentMetadata';
    if (options.mode === 'submit' && prepared?.approval.accounts) {
      check(options.udids.every(id => handle(accounts[id]) === currentAccounts[id]),
        'Nick gán đã đổi sau preflight; không Create/Execute');
    }
    let campaign = read(file('campaign.json'));
    if (options.mode === 'preflight') {
      check(!intent && !campaign && !read(file('execute-intent.json')), 'Report directory đã có lượt tạo; không thay preflight');
      const writer = await checkSheet(invoke, options);
      const request = { sourceRoot: options.source, bundleIds: options.bundleIds, udids: options.udids,
        targetRef: { type: 'explicit', udids: options.udids }, runAt: null, captionOverrides: options.contentSnapshot?.captionOverrides ?? {},
        soundPolicy: options.contentSnapshot?.soundPolicy ?? { kind: 'default' }, sheetEnabled: true, deleteAfterPublish: false };
      const preflight = await invoke('publish_preflight', { request });
      report.preflight = preflight;
      const target = preflight.sheetDelivery;
      check(preflight.canExecute === true && preflight.sheetEnabled === true && target?.version === 2
        && target.spreadsheetId === writer.spreadsheetId && target.sheetGid === writer.sheetGid
        && target.reportingEpoch === writer.reportingEpoch, 'Preflight bị chặn hoặc sai đích Sheet/epoch');
      check(preflight.assignments?.length === options.udids.length && preflight.assignments.every((a, i) =>
        a.udid === options.udids[i] && a.bundleId === options.bundleIds[i] && a.ordinal === i), 'Preflight thay đổi mapping đã yêu cầu');
      check(options.udids.every(id => /^[a-z0-9_.]{1,24}$/.test(currentAccounts[id])), 'Thiếu nick gán trong roster nghiệm thu');
      const approval = { scope: scope(options), accounts: currentAccounts, request, inputDigest: preflight.inputDigest, sheetDelivery: target, writer };
      report.confirmation = hash(approval);
      write(file('preflight.json'), { approval, confirmation: report.confirmation });
      report.acceptance = 'preflightOnly';
    } else if (options.mode === 'submit') {
      let durable = intent;
      if (!durable) {
        check(!campaign && !read(file('execute-intent.json')), 'Có receipt không có create intent; không tạo lại');
        const writer = await checkSheet(invoke, options);
        check(hash(writer) === hash(prepared.approval.writer), 'Writer/target/epoch đã đổi; không tạo');
        const args = { ...prepared.approval.request, requestId: randomUUID(), confirmed: true,
          approvedInputDigest: prepared.approval.inputDigest };
        durable = { confirmation: options.confirm, requestFingerprint: hash(args), args };
        write(file('create-intent.json'), durable, true);
      }
      if (!campaign) {
        check(!read(file('execute-intent.json')), 'Execute intent đã có nhưng mất campaign receipt; chỉ observe bằng ID, không create');
        // An exception may be an ACK lost after commit. Retain exact request, never generate another UUID.
        exitCode = 2;
        campaign = await invoke('publish_create_campaign', durable.args);
        exitCode = 1;
        validateCampaign(campaign, durable);
        write(file('campaign.json'), campaign, true);
      }
      validateCampaign(campaign, durable);
      const before = await invoke('publish_get', { campaignId: campaign.id });
      check(before?.campaign?.id === campaign.id, 'Campaign đọc lại không khớp receipt');
      const summary = await summarize(before, options, invoke, accounts);
      check(summary.scopeMatches && new Set(before.assignments.map(a => a.id)).size === before.assignments.length
        && before.assignments.every((a, i) => a.id && a.campaignId === campaign.id
          && a.udid === campaign.assignments[i].udid && a.ordinal === i
          && a.bundleId === campaign.assignments[i].bundleId),
      'Assignments không khớp identity đã duyệt; không Execute');
      const executeIntent = read(file('execute-intent.json'));
      if (executeIntent) {
        check(executeIntent.campaignId === campaign.id && executeIntent.requestFingerprint === durable.requestFingerprint,
          'Execute intent sai campaign/fingerprint');
        report.execute = 'alreadyIntendedObserveOnly';
      } else if (!['queued', 'ready'].includes(before.campaign.state)
        || before.assignments.some(a => !['queued', 'ready'].includes(a.state) || a.effectIntent || a.dispatch)
        || summary.rows.some(a => a.submitted || a.verified)) {
        // Another caller or an earlier lost local receipt may already own this work.
        // Even without our intent, server evidence never authorizes another Execute.
        report.execute = 'existingWorkObserveOnly';
      } else {
        const execution = { campaignId: campaign.id, assignmentIds: before.assignments.map(a => a.id),
          requestFingerprint: durable.requestFingerprint, at: new Date(now()).toISOString() };
        write(file('execute-intent.json'), execution, true);
        try {
          const response = await invoke('publish_execute', { campaignId: campaign.id, confirmed: true });
          write(file('execution-ack.json'), response);
          report.execute = 'acknowledged';
        } catch (error) { report.execute = 'ackUnknownNeverReplay'; report.executeError = String(error); }
      }
      const detail = await invoke('publish_get', { campaignId: campaign.id });
      write(file('detail.json'), detail);
      const result = await summarize(detail, options, invoke, accounts);
      exitCode = result.exitCode; Object.assign(report, result);
    } else {
      const campaignId = options.campaignId ?? campaign?.id;
      check(options.mode !== 'observe' || campaignId, 'observe cần --campaign-id hoặc campaign.json');
      if (campaignId) {
        const deadline = now() + options.waitSeconds * 1000;
        do {
          const detail = await invoke('publish_get', { campaignId });
          check(detail?.campaign?.id === campaignId, 'Campaign đọc lại không khớp ID');
          write(file('detail.json'), detail);
          const result = await summarize(detail, options, invoke, accounts);
          Object.assign(report, result); exitCode = result.exitCode;
          if (options.mode !== 'observe' || exitCode !== 2 || now() >= deadline) break;
          await sleep(Math.min(options.pollSeconds * 1000, deadline - now()));
        } while (true);
        if (exitCode === 2) report.acceptance = 'pendingDeadline';
      }
    }
  } catch (error) {
    if (exitCode !== 2) exitCode = 1;
    report.acceptance = exitCode === 2 ? 'ackUnknown' : 'blockedFailed';
    report.error = String(error);
  } finally {
    try {
      report.exitCode = exitCode;
      write(file('report.json'), report);
    } finally {
      fs.closeSync(lock);
      fs.unlinkSync(file('harness.lock'));
    }
  }
  return { exitCode, report };
}

const ALLOWED_IPC = new Set(['list_devices', 'list_device_metas', 'publish_get', 'google_sheets_status',
  'publish_sheet_readback', 'publish_sheet_check', 'publish_preflight', 'publish_create_campaign', 'publish_execute']);
export async function connectIPC(options, chromium) {
  const browser = await chromium.connectOverCDP(options.cdp, { timeout: 15000 });
  try {
    const pages = browser.contexts().flatMap(c => c.pages()).filter(page => {
      if (options.pageUrl) return page.url() === options.pageUrl;
      const url = new URL(page.url());
      return ['http:', 'https:'].includes(url.protocol) && ['localhost', '127.0.0.1', 'tauri.localhost'].includes(url.hostname);
    });
    check(pages.length === 1, 'Cần đúng một WebView ứng dụng; dùng --page-url để chọn tường minh');
    const page = pages[0];
    check(await page.evaluate(() => typeof window.__TAURI_INTERNALS__?.invoke === 'function'), 'WebView chưa có Tauri IPC');
    return {
      invoke: (command, args = {}) => {
        check(ALLOWED_IPC.has(command), 'IPC ngoài phạm vi harness');
        return page.evaluate(async ({ command, args }) => window.__TAURI_INTERNALS__.invoke(command, args), { command, args });
      },
      // For connectOverCDP, close disconnects this client; it does not close the existing app/pages.
      close: () => browser.close(),
    };
  } catch (error) { await browser.close(); throw error; }
}

if (process.argv[1] && import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href) {
  let transport;
  try {
    const options = parseArgs(process.argv.slice(2));
    const { chromium } = await import('../apps/desktop/node_modules/playwright/index.mjs');
    transport = await connectIPC(options, chromium);
    const result = await runAcceptance(options, transport);
    console.log(JSON.stringify(result.report, null, 2));
    process.exitCode = result.exitCode;
  } catch (error) { console.error(String(error)); process.exitCode = 1; }
  finally { await transport?.close(); }
}
