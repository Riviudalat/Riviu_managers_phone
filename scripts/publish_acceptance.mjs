import fs from 'node:fs';
import path from 'node:path';
import { createHash, randomUUID } from 'node:crypto';
import { pathToFileURL } from 'node:url';
import { setTimeout as delay } from 'node:timers/promises';

const MODES = ['inspect', 'preflight', 'submit', 'observe'];
const FLAGS = ['mode', 'report-dir', 'udids', 'source', 'bundle-ids', 'sheet-id', 'sheet-gid',
  'sheet-disabled', 'dev-scope', 'cdp', 'page-url', 'campaign-id', 'confirm', 'wait-seconds', 'poll-seconds', 'content-snapshot'];
const check = (condition, message) => { if (!condition) throw new Error(message); };
const hash = value => createHash('sha256').update(JSON.stringify(value)).digest('hex');
const hashBytes = value => createHash('sha256').update(value).digest('hex');
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
function plainObject(value) {
  return value !== null && typeof value === 'object' && !Array.isArray(value)
    && Object.getPrototypeOf(value) === Object.prototype;
}
function readDevScope(file) {
  const stat = fs.lstatSync(file);
  check(stat.isFile() && !stat.isSymbolicLink(), '--dev-scope phải là regular file, không nhận symlink');
  const raw = fs.readFileSync(file);
  let value;
  try { value = JSON.parse(raw.toString('utf8')); }
  catch { throw new Error('--dev-scope không phải JSON hợp lệ'); }
  check(plainObject(value), '--dev-scope phải là JSON object');
  return { raw, value, mode: stat.mode };
}
function exactKeys(value, expected) {
  return Object.keys(value).sort().join('\0') === [...expected].sort().join('\0');
}
function validateInactiveDevScope(file, udids) {
  const current = readDevScope(file), policy = current.value;
  check(Object.keys(policy).every(key => ['activationId', 'campaignIds', 'deviceIds', 'capabilities'].includes(key))
    && Object.hasOwn(policy, 'activationId') && Object.hasOwn(policy, 'campaignIds')
    && Object.hasOwn(policy, 'deviceIds'),
  '--dev-scope có field ngoài DevAcceptancePolicy');
  check(typeof policy.activationId === 'string' && /^[A-Za-z0-9_-]{16,128}$/.test(policy.activationId),
    '--dev-scope activationId is invalid');
  check(Array.isArray(policy.campaignIds) && policy.campaignIds.length === 0,
    '--dev-scope preflight cần campaignIds rỗng');
  check(Array.isArray(policy.deviceIds) && JSON.stringify(policy.deviceIds) === JSON.stringify(udids),
    '--dev-scope deviceIds không khớp đúng thứ tự máy yêu cầu');
  if (Object.hasOwn(policy, 'capabilities')) {
    check(plainObject(policy.capabilities)
      && Object.keys(policy.capabilities).every(key => ['publishVerification', 'sheetDelivery'].includes(key))
      && Object.values(policy.capabilities).every(value => value === false),
    '--dev-scope preflight chỉ nhận capability false hoặc absent');
  }
  return current;
}
function activeDevScope(campaignId, udids, activationId) {
  return { activationId, campaignIds: [campaignId], deviceIds: [...udids],
    capabilities: { publishVerification: true } };
}
function isActiveDevScope(policy, campaignId, udids, activationId) {
  return plainObject(policy) && exactKeys(policy, ['activationId', 'campaignIds', 'deviceIds', 'capabilities'])
    && policy.activationId === activationId
    && JSON.stringify(policy.campaignIds) === JSON.stringify([campaignId])
    && JSON.stringify(policy.deviceIds) === JSON.stringify(udids)
    && plainObject(policy.capabilities)
    && exactKeys(policy.capabilities, ['publishVerification'])
    && policy.capabilities.publishVerification === true;
}
function inspectDevScope(file, udids) {
  const current = validateInactiveDevScope(file, udids);
  return { path: file, contentHash: hashBytes(current.raw), activationId: current.value.activationId };
}
function validateDevScopeForSubmit(binding, campaign, udids) {
  check(binding?.path && binding.contentHash && /^[a-f0-9]{64}$/.test(binding.contentHash)
    && /^[A-Za-z0-9_-]{16,128}$/.test(binding.activationId ?? ''),
    'Preflight thiếu binding --dev-scope');
  const current = readDevScope(binding.path);
  if (hashBytes(current.raw) === binding.contentHash) {
    validateInactiveDevScope(binding.path, udids);
    return 'inactiveApproved';
  }
  check(campaign?.id && isActiveDevScope(current.value, campaign.id, udids, binding.activationId),
    '--dev-scope đã đổi sau preflight');
  return 'activeForCampaign';
}
function activateDevScope(binding, campaignId, udids) {
  const lockFile = `${binding.path}.publish-acceptance.lock`;
  let lock;
  try { lock = fs.openSync(lockFile, 'wx'); }
  catch (error) { throw new Error(`--dev-scope đang khóa hoặc không ghi được: ${error.code}`); }
  try {
    fs.writeFileSync(lock, String(process.pid)); fs.fsyncSync(lock);
    const current = readDevScope(binding.path);
    if (isActiveDevScope(current.value, campaignId, udids, binding.activationId)) return hashBytes(current.raw);
    check(hashBytes(current.raw) === binding.contentHash, '--dev-scope đã đổi trước khi mở campaign');
    validateInactiveDevScope(binding.path, udids);
    const next = activeDevScope(campaignId, udids, binding.activationId);
    const temp = `${binding.path}.${randomUUID()}.tmp`;
    let fd, replaced = false;
    try {
      try {
        fd = fs.openSync(temp, 'wx', current.mode & 0o777);
        fs.writeFileSync(fd, JSON.stringify(next, null, 2) + '\n');
        fs.fsyncSync(fd);
      } finally {
        if (fd !== undefined) fs.closeSync(fd);
      }
      fs.renameSync(temp, binding.path);
      replaced = true;
    } finally {
      if (!replaced) fs.rmSync(temp, { force: true });
    }
    const verified = readDevScope(binding.path);
    check(isActiveDevScope(verified.value, campaignId, udids, binding.activationId), '--dev-scope replace không đọc lại được');
    return hashBytes(verified.raw);
  } finally {
    try { fs.closeSync(lock); }
    finally { fs.unlinkSync(lockFile); }
  }
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
  check(flags['sheet-disabled'] === undefined || flags['sheet-disabled'] === 'true',
    '--sheet-disabled only accepts the explicit value true');
  const sheetDisabled = flags['sheet-disabled'] === 'true';
  check(MODES.includes(mode), 'mode phải là inspect, preflight, submit hoặc observe');
  check(flags['report-dir'], 'Thiếu --report-dir');
  const cdp = new URL(flags.cdp ?? 'http://127.0.0.1:9277');
  check(cdp.protocol === 'http:' && ['127.0.0.1', '[::1]'].includes(cdp.hostname)
    && cdp.port && cdp.pathname === '/' && !cdp.search && !cdp.hash && !cdp.username && !cdp.password,
  '--cdp chỉ nhận HTTP loopback bằng địa chỉ IP và cổng, không path/credential');
  check(!flags['dev-scope'] || path.isAbsolute(flags['dev-scope']), '--dev-scope phải là đường dẫn tuyệt đối');
  const options = { mode, reportDir: path.resolve(flags['report-dir']), cdp: cdp.origin,
    pageUrl: flags['page-url'] ?? null, udids: list(flags.udids, '--udids'),
    source: flags.source ? path.resolve(flags.source) : null,
    contentSnapshot: flags['content-snapshot'] ? read(path.resolve(flags['content-snapshot'])) : null,
    bundleIds: flags['bundle-ids'] ? list(flags['bundle-ids'], '--bundle-ids') : [],
    sheetId: flags['sheet-id'] ?? null, sheetGid: flags['sheet-gid'] === undefined ? null
      : integer(flags['sheet-gid'], 0, 2147483647, '--sheet-gid'),
    sheetDisabled, devScope: flags['dev-scope'] ? path.normalize(flags['dev-scope']) : null,
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
  check(!options.sheetDisabled || !options.sheetId, '--sheet-disabled true không nhận --sheet-id/--sheet-gid');
  check(!options.devScope || options.sheetDisabled, '--dev-scope chỉ dùng cùng --sheet-disabled true');
  check(!options.campaignId || /^[A-Za-z0-9_-]{1,128}$/.test(options.campaignId), 'Campaign ID không hợp lệ');
  check(mode === 'submit' ? /^[a-f0-9]{64}$/.test(options.confirm ?? '') : !options.confirm,
    'submit cần --confirm SHA-256 từ preflight; mode khác không nhận --confirm');
  if (mode === 'submit' || mode === 'preflight') {
    check(options.source && options.bundleIds.length === options.udids.length,
      'preflight/submit cần --source và --bundle-ids một-một với --udids');
    check(options.sheetDisabled || options.sheetId,
      'preflight/submit cần đích Sheet; chỉ bỏ qua khi có --sheet-disabled true');
    check(!options.sheetDisabled || options.udids.length <= 2,
      'Canary --sheet-disabled true chỉ nhận tối đa 2 publication');
    check(!options.campaignId, 'Không submit/preflight campaign cũ; dùng observe');
  }
  check(mode === 'observe' || (options.waitSeconds === 0 && flags['poll-seconds'] === undefined),
    'Chỉ observe nhận thời gian chờ; chờ không kích hoạt kiểm tra thiết bị');
  return options;
}
function scope(options) {
  return { cdp: options.cdp, pageUrl: options.pageUrl, source: options.source,
    contentSnapshot: options.contentSnapshot, udids: options.udids, bundleIds: options.bundleIds,
    sheetId: options.sheetId, sheetGid: options.sheetGid, ...(options.sheetDisabled ? { sheetDisabled: true } : {}),
    ...(options.devScope ? { devScope: options.devScope } : {}) };
}
function requestFingerprint(args, sheetDisabled) {
  return hash(sheetDisabled ? { sheetDisabled: true, args } : args);
}
function validateHandoff(result, udids) {
  check(result?.operationId && result.state === 'closed' && Array.isArray(result.devices),
    'Handoff did not confirm every selected device was closed');
  check(result.devices.length === udids.length
    && new Set(result.devices.map(row => row.udid)).size === udids.length
    && udids.every(udid => result.devices.some(row => row.udid === udid && row.closed === true)),
  'Handoff left at least one device unreleased; refusing Create/Post');
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
  const sheetDisabled = options.sheetDisabled === true;
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
      sheetSent: !sheetDisabled && a.sheetDelivery?.state === 'sent',
      sheetState: sheetDisabled ? 'disabled' : a.sheetDelivery?.state ?? 'notReported',
      postUrl: canonical ? post.postUrl : null, urlReadback: 'unsupported',
      verification: evidence.verificationStatus ?? null };
  });
  for (const row of sheetDisabled ? [] : rows.filter(r => r.verified && r.sheetSent)) {
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
  const allPhoneVerified = rows.length > 0 && rows.every(r => r.verified && r.accountState === 'matched');
  if (sheetDisabled) {
    const accepted = exact && !failed && allPhoneVerified;
    return { rows, counts, campaignId: detail.campaign.id,
      acceptance: accepted ? 'phoneOnly/sheetDisabled' : !exact || failed ? 'blockedFailed' : 'pending',
      acceptanceScope: 'phoneOnly', sheetAcceptance: 'sheetDisabled',
      exitCode: accepted ? 0 : !exact || failed ? 1 : 2, scopeMatches: exact,
      deliveryEvidence: 'sheetDisabled',
      urlReadback: { status: 'notApplicable', reason: 'Sheet was explicitly disabled for this bounded phone-only acceptance.' } };
  }
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
  let report = { mode: options.mode, at: new Date(now()).toISOString(), acceptance: 'notEvaluated',
    ...(options.sheetDisabled ? { acceptanceScope: 'phoneOnly', sheetAcceptance: 'sheetDisabled' } : {}) };
  let exitCode = 0;
  try {
    fs.writeFileSync(lock, String(process.pid)); fs.fsyncSync(lock);
    const intent = read(file('create-intent.json'));
    const prepared = read(file('preflight.json'));
    if (options.mode === 'submit') {
      check(prepared && hash(prepared.approval) === prepared.confirmation
        && prepared.confirmation === options.confirm, 'Confirmation/preflight không khớp');
      check(hash(scope(options)) === hash(prepared.approval.scope), 'Phạm vi khác preflight đã duyệt');
      if (options.sheetDisabled) check(prepared.approval.sheetDisabled === true
        && prepared.approval.writer === null && prepared.approval.sheetDelivery === null,
      'Preflight phone-only không ghi nhận Sheet disabled');
      if (intent) check(intent.confirmation === options.confirm
        && (intent.sheetDisabled === true) === options.sheetDisabled
        && requestFingerprint(intent.args, intent.sheetDisabled === true) === intent.requestFingerprint,
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
    if (options.mode === 'submit' && options.devScope) {
      check(prepared.approval.devScope?.path === options.devScope,
        '--dev-scope không khớp path đã duyệt');
      report.devScope = { ...prepared.approval.devScope,
        state: validateDevScopeForSubmit(prepared.approval.devScope, campaign, options.udids) };
    }
    if (options.mode === 'preflight') {
      check(!intent && !campaign && !read(file('execute-intent.json')), 'Report directory đã có lượt tạo; không thay preflight');
      const devScope = options.devScope ? inspectDevScope(options.devScope, options.udids) : null;
      if (devScope) report.devScope = { ...devScope, state: 'inactiveApproved' };
      const writer = options.sheetDisabled ? null : await checkSheet(invoke, options);
      const request = { sourceRoot: options.source, bundleIds: options.bundleIds, udids: options.udids,
        targetRef: { type: 'explicit', udids: options.udids }, runAt: null, captionOverrides: options.contentSnapshot?.captionOverrides ?? {},
        soundPolicy: options.contentSnapshot?.soundPolicy ?? { kind: 'default' }, sheetEnabled: !options.sheetDisabled, deleteAfterPublish: false };
      const preflight = await invoke('publish_preflight', { request });
      report.preflight = preflight;
      const target = preflight.sheetDelivery ?? null;
      if (options.sheetDisabled) {
        check(preflight.canExecute === true && preflight.sheetEnabled === false && target === null,
          'Preflight phone-only phải xác nhận sheetEnabled=false và sheetDelivery=null');
      } else {
        check(preflight.canExecute === true && preflight.sheetEnabled === true && target?.version === 2
          && target.spreadsheetId === writer.spreadsheetId && target.sheetGid === writer.sheetGid
          && target.reportingEpoch === writer.reportingEpoch, 'Preflight bị chặn hoặc sai đích Sheet/epoch');
      }
      check(preflight.assignments?.length === options.udids.length && preflight.assignments.every((a, i) =>
        a.udid === options.udids[i] && a.bundleId === options.bundleIds[i] && a.ordinal === i), 'Preflight thay đổi mapping đã yêu cầu');
      check(options.udids.every(id => /^[a-z0-9_.]{1,24}$/.test(currentAccounts[id])), 'Thiếu nick gán trong roster nghiệm thu');
      const approval = { scope: scope(options), ...(options.sheetDisabled ? { sheetDisabled: true } : {}),
        ...(devScope ? { devScope } : {}), accounts: currentAccounts, request,
        inputDigest: preflight.inputDigest, sheetDelivery: target, writer };
      report.confirmation = hash(approval);
      write(file('preflight.json'), { approval, confirmation: report.confirmation });
      report.acceptance = 'preflightOnly';
    } else if (options.mode === 'submit') {
      let durable = intent;
      if (!durable) {
        check(!campaign && !read(file('execute-intent.json')), 'Có receipt không có create intent; không tạo lại');
        if (!options.sheetDisabled) {
          const writer = await checkSheet(invoke, options);
          check(hash(writer) === hash(prepared.approval.writer), 'Writer/target/epoch đã đổi; không tạo');
        }
        if (options.sheetDisabled) {
          const handoffIntent = read(file('handoff-intent.json'));
          const handoff = read(file('handoff.json'));
          const expectedHandoff = {
            confirmation: options.confirm,
            scopeFingerprint: hash(scope(options)),
            udids: options.udids,
          };
          const handoffIdentity = value => ({ confirmation: value.confirmation,
            scopeFingerprint: value.scopeFingerprint, udids: value.udids });
          if (handoff) {
            check(handoffIntent && hash(handoffIdentity(handoffIntent)) === hash(expectedHandoff),
              'Handoff intent does not match the approved preflight');
            validateHandoff(handoff.result, options.udids);
            report.handoff = 'alreadyAcknowledged';
          } else if (handoffIntent) {
            check(hash(handoffIdentity(handoffIntent)) === hash(expectedHandoff),
              'Handoff intent does not match the approved preflight');
            exitCode = 2;
            throw new Error('Handoff ACK unknown; refusing replay and Create/Post');
          } else {
            write(file('handoff-intent.json'), { ...expectedHandoff, at: new Date(now()).toISOString() }, true);
            exitCode = 2;
            const result = await invoke('operation_prepare_devices', { udids: options.udids });
            exitCode = 1;
            write(file('handoff.json'), { result }, true);
            validateHandoff(result, options.udids);
            report.handoff = 'acknowledged';
          }
          let accountProof = read(file('account-proof.json'));
          if (!accountProof) {
            const readings = [];
            for (const udid of options.udids) {
              const reading = await invoke('interaction_read_account', { udid });
              check(reading?.udid === udid && reading.status === 'matched'
                && handle(reading.expectedHandle) === handle(accounts[udid])
                && handle(reading.observedHandle) === handle(accounts[udid])
                && reading.snapshotSha256,
              `Account preflight did not match approved metadata for ${udid}`);
              readings.push({ udid, expectedHandle: handle(reading.expectedHandle),
                observedHandle: handle(reading.observedHandle), status: reading.status,
                checkedAt: reading.checkedAt, snapshotSha256: reading.snapshotSha256 });
            }
            accountProof = { readings, scopeFingerprint: hash(scope(options)) };
            write(file('account-proof.json'), accountProof, true);
          }
          check(accountProof.scopeFingerprint === hash(scope(options))
            && accountProof.readings?.length === options.udids.length
            && options.udids.every(udid => accountProof.readings.some(reading => reading.udid === udid
              && reading.status === 'matched' && reading.observedHandle === handle(accounts[udid]))),
          'Persisted account proof does not match the approved scope');
        }
        const args = { ...prepared.approval.request, requestId: randomUUID(), confirmed: true,
          approvedInputDigest: prepared.approval.inputDigest };
        durable = { confirmation: options.confirm, ...(options.sheetDisabled ? { sheetDisabled: true } : {}),
          requestFingerprint: requestFingerprint(args, options.sheetDisabled), args };
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
      if (options.devScope) {
        report.devScope = { ...prepared.approval.devScope, state: 'activeForCampaign',
          activeContentHash: activateDevScope(prepared.approval.devScope, campaign.id, options.udids) };
      }
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
        if (options.sheetDisabled) {
          check(prepared && hash(prepared.approval) === prepared.confirmation
            && prepared.approval.sheetDisabled === true
            && prepared.approval.request?.sheetEnabled === false
            && hash(scope(options)) === hash(prepared.approval.scope),
          'Observe phone-only requires its exact approved Sheet-disabled preflight');
          check(intent && intent.sheetDisabled === true
            && intent.confirmation === prepared.confirmation
            && requestFingerprint(intent.args, true) === intent.requestFingerprint,
          'Observe phone-only requires its exact durable create intent');
          check(campaign?.id === campaignId,
            'Observe phone-only campaign does not match its local receipt');
          validateCampaign(campaign, intent);
          check(prepared.approval.devScope?.path === options.devScope
            && validateDevScopeForSubmit(prepared.approval.devScope, campaign, options.udids)
              === 'activeForCampaign',
          'Observe phone-only requires the active approved dev scope');
        }
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

const ALLOWED_IPC = new Set(['list_devices', 'list_device_metas', 'operation_prepare_devices', 'interaction_read_account', 'publish_get', 'google_sheets_status',
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
