import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { createHash } from 'node:crypto';
import { parseArgs, runAcceptance, connectIPC } from './publish_acceptance.mjs';

// Mock only Tauri IPC. The command runner, safety decisions and files are real.
const source = path.resolve('fixture-source');
const base = ['--udids', 'phone-b,phone-a', '--source', source,
  '--bundle-ids', 'bundle-b,bundle-a', '--sheet-id', 'sheet_fixture', '--sheet-gid', '0'];
const postUrl = 'https://www.tiktok.com/@fixture/photo/123456789';
const sheet = { version: 2, spreadsheetId: 'sheet_fixture', sheetGid: 0,
  reportingEpoch: 'epoch-1', internalReporting: true };
function environment(t) {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'publish-acceptance-'));
  t.after(() => fs.rmSync(dir, { recursive: true, force: true }));
  const devScope = path.join(dir, 'dev-acceptance-scope.json');
  const activationId = 'fixture-activation-20260922';
  const initialDevScope = { activationId, campaignIds: [], deviceIds: ['phone-b', 'phone-a'],
    capabilities: { publishVerification: false, sheetDelivery: false } };
  fs.writeFileSync(devScope, JSON.stringify(initialDevScope) + '\n');
  const options = (mode = 'inspect', more = []) => parseArgs([
    '--mode', mode, '--protocol', 'legacy', '--report-dir', dir, ...base,
    ...(['preflight', 'submit'].includes(mode) ? ['--dev-scope', devScope] : []), ...more]);
  const phoneOnlyOptions = (mode = 'inspect', more = []) => parseArgs([
    '--mode', mode, '--protocol', 'legacy', '--report-dir', dir, '--udids', 'phone-b,phone-a', '--source', source,
    '--bundle-ids', 'bundle-b,bundle-a', '--sheet-disabled', 'true', '--dev-scope', devScope, ...more]);
  const calls = [];
  let created;
  let handoffFailure = false;
  let lostHandoff = false;
  let lostCreate = false;
  let lostExecute = false;
  let accountMismatch = false;
  const detail = { campaign: { id: 'campaign-fixture', requestId: '', sourceRoot: source,
    state: 'verifying', visibility: 'public', cleanupPolicy: 'keepImportedAssets',
    assignments: [{ bundleId: 'managed-b', udid: 'phone-b', ordinal: 0 }, { bundleId: 'managed-a', udid: 'phone-a', ordinal: 1 }],
    createdAt: '2026-09-17T00:00:00Z', updatedAt: '2026-09-17T00:01:00Z' },
    bundles: [], events: [], assignments: ['phone-b', 'phone-a'].map((udid, ordinal) => ({
      id: `assignment-${ordinal}`, campaignId: 'campaign-fixture', bundleId: `managed-${ordinal}`,
      ordinal, udid, publicationId: `assignment-${ordinal}`, state: 'verifying',
      dispatch: { phase: 'compose', state: 'finished', queuedAtMs: 100, startedAtMs: 200, owner: null, reason: null, revision: 2 },
      effectIntent: JSON.stringify({ expectedAccount: 'fixture', submittedAt: '2026-09-17T00:00:30Z' }),
      evidenceJson: JSON.stringify({ post: { state: 'submitted', verdict: 'Submitted', publicationVerified: false,
        expectedAccount: 'fixture', submittedAt: '2026-09-17T00:00:30Z' },
        verificationStatus: { state: 'pending', checkedAt: '2026-09-17T00:02:00Z', nextCheckAt: '2026-09-17T00:07:00Z' } }),
      sheetDelivery: { state: 'pending', attempts: 0, lastError: null, nextAttemptAtMs: null, updatedAt: '2026-09-17T00:00:30Z' }, errorCode: null,
    })) };
  const submittedAssignments = structuredClone(detail.assignments);
  const preflight = { inputDigest: 'digest-fixture', canExecute: true, sheetConfigured: true,
    sheetEnabled: true, sheetDelivery: sheet, issues: [],
    targetSnapshot: { udids: ['phone-b', 'phone-a'] },
    assignments: ['phone-b', 'phone-a'].map((udid, ordinal) => ({ udid, ordinal,
      bundleId: ordinal === 0 ? 'bundle-b' : 'bundle-a', media: 'pass', composer: 'pass', soundPicker: 'pass',
      storage: 'pass', requiredBytes: 123, availableBytes: 1000, issues: [], packageName: 'fixture.tiktok', version: '1', locale: 'en' })) };
  const invoke = async (command, args = {}) => {
    calls.push({ command, args: structuredClone(args) });
    switch (command) {
      case 'list_devices': return [
        { udid: 'phone-a', platform: 'android', status: 'ready', name: 'A' },
        { udid: 'phone-b', platform: 'android', status: 'ready', name: 'B' },
        { udid: 'excluded', platform: 'android', status: 'disconnected', name: 'Off' }];
      case 'list_device_metas': return [{ udid: 'phone-a', number: 42, handle: 'fixture' }, { udid: 'phone-b', number: 7, handle: 'fixture' }, { udid: 'metadata-only', number: 99 }];
      case 'google_sheets_status': return { configured: true, connected: true, active: true, writerId: 'writer-fixture',
        selectedFileId: 'sheet_fixture', sheetUrl: 'https://docs.google.com/spreadsheets/d/sheet_fixture/edit#gid=0',
        phase: 'idle', pickerConfigured: true, clientId: 'fixture-client' };
      case 'publish_sheet_check': return { sheetUrl: args.sheetUrl, spreadsheetId: 'sheet_fixture', sheetGid: 0,
        readable: true, connectionVerified: true, reportingReady: true, reportingEpoch: 'epoch-1',
        layout: 'internal', columns: ['Link'], message: 'ready' };
      case 'publish_preflight': return structuredClone(preflight);
      case 'operation_prepare_devices': {
        const result = { operationId: 'handoff-fixture', state: handoffFailure ? 'needsAttention' : 'closed',
          devices: ['phone-a', 'phone-b'].map(udid => ({ udid,
            closed: !handoffFailure || udid !== 'phone-a',
            message: handoffFailure && udid === 'phone-a' ? 'busy' : 'closed' })),
          stopMarker: null };
        if (lostHandoff) { lostHandoff = false; throw new Error('handoff ACK lost'); }
        return result;
      }
      case 'interaction_read_account': return { udid: args.udid, expectedHandle: 'fixture',
        observedHandle: accountMismatch ? 'other' : 'fixture', status: accountMismatch ? 'mismatch' : 'matched', checkedAt: '2026-09-22T00:00:00Z',
        snapshotSha256: 'a'.repeat(64) };
      case 'publish_create_campaign': {
        // A lost response is AFTER server commit; same requestId must recover this row.
        const intent = JSON.parse(fs.readFileSync(path.join(dir, 'create-intent.json'), 'utf8'));
        assert.equal(intent.args.requestId, args.requestId, 'intent exists before create');
        assert.match(intent.requestFingerprint, /^[a-f0-9]{64}$/);
        if (created) assert.deepEqual(args, created, 'replay must preserve the whole request');
        created = structuredClone(args);
        detail.campaign.requestId = args.requestId;
        detail.campaign.assignments.forEach((a, i) => { a.bundleId = `${args.requestId}:${args.bundleIds[i]}`; });
        detail.assignments.forEach((a, i) => {
          a.bundleId = `${args.requestId}:${args.bundleIds[i]}`;
          a.state = 'queued'; a.dispatch = null; a.effectIntent = null; a.evidenceJson = null;
        });
        detail.campaign.state = 'queued';
        if (lostCreate) { lostCreate = false; throw new Error('create ACK lost'); }
        return structuredClone(detail.campaign);
      }
      case 'publish_execute': {
        if (created) assert.deepEqual(JSON.parse(fs.readFileSync(devScope, 'utf8')),
          { activationId, campaignIds: ['campaign-fixture'], deviceIds: ['phone-b', 'phone-a'],
            capabilities: { publishVerification: true, sheetDelivery: true, publishCleanup: true } }, 'scope opens verification, Sheet and cleanup before Execute');
        const intent = JSON.parse(fs.readFileSync(path.join(dir, 'execute-intent.json'), 'utf8'));
        assert.equal(intent.campaignId, args.campaignId, 'intent exists before Execute');
        assert.deepEqual(intent.assignmentIds, ['assignment-0', 'assignment-1']);
        detail.campaign.state = 'verifying';
        detail.assignments.forEach((a, i) => {
          const submitted = submittedAssignments[i];
          a.state = submitted.state; a.dispatch = submitted.dispatch;
          a.effectIntent = submitted.effectIntent; a.evidenceJson = submitted.evidenceJson;
        });
        if (lostExecute) { lostExecute = false; throw new Error('execute ACK lost'); }
        return { campaignId: args.campaignId, status: 'partial', retryScope: 'linkAndSheet', issues: [], detail };
      }
      case 'publish_get': assert.equal(args.campaignId, 'campaign-fixture'); return structuredClone(detail);
      default: assert.fail(`unsafe or unexpected IPC: ${command}`);
    }
  };
  return { dir, devScope, initialDevScope, options, phoneOnlyOptions, invoke, calls, detail, preflight,
    failHandoff: () => { handoffFailure = true; }, loseHandoff: () => { lostHandoff = true; },
    mismatchAccount: () => { accountMismatch = true; },
    loseCreate: () => { lostCreate = true; }, loseExecute: () => { lostExecute = true; } };
}

// Break caught: inspect starts preflight (device/network work), refresh or any mutation.
test('default inspect reads full roster metadata only and never invents machine numbers', async t => {
  const e = environment(t);
  const options = parseArgs(['--report-dir', e.dir, '--udids', 'phone-b,phone-a']);
  const result = await runAcceptance(options, { invoke: e.invoke });
  assert.equal(result.report.mode, 'inspect');
  assert.deepEqual(e.calls.map(c => c.command), ['list_devices', 'list_device_metas']);
  assert.deepEqual(result.report.roster.selected.map(r => [r.udid, r.number]), [['phone-b', 7], ['phone-a', 42]]);
  assert.deepEqual(result.report.roster.excluded.map(r => r.udid).sort(), ['excluded', 'metadata-only']);
  assert.equal(result.report.acceptance, 'notEvaluated');
  assert.equal(fs.existsSync(path.join(e.dir, 'create-intent.json')), false);
});

// Break caught: permissive argument parsing turns a typo/remote endpoint into a write.
test('arguments reject ambiguous, duplicate, remote and unconfirmed inputs before IPC', () => {
  for (const args of [ ['--bogus', 'x'], ['--mode', 'wat'], ['--mode', 'inspect', '--mode', 'submit'],
    ['--udids', 'a,a'], ['--sheet-gid', '0oops'], ['--sheet-gid', '-1'], ['--cdp', 'http://example.com:9277'],
    ['--cdp', 'http://127.0.0.1:9277/redirect'], ['--mode', 'submit'], ['--confirm', 'yes'] ]) {
    assert.throws(() => parseArgs(['--report-dir', 'out', ...args]));
  }
});

test('preflight binds explicit mapping and Sheet identity but never creates', async t => {
  const e = environment(t);
  const result = await runAcceptance(e.options('preflight'), { invoke: e.invoke });
  assert.equal(result.exitCode, 0);
  assert.match(result.report.confirmation, /^[a-f0-9]{64}$/);
  const request = e.calls.find(c => c.command === 'publish_preflight').args.request;
  assert.deepEqual(request.udids, ['phone-b', 'phone-a']);
  assert.deepEqual(request.bundleIds, ['bundle-b', 'bundle-a']);
  assert.deepEqual(request.targetRef, { type: 'explicit', udids: ['phone-b', 'phone-a'] });
  assert.equal(request.sheetEnabled, true);
  assert.equal(request.deleteAfterPublish, true);
  assert.ok(e.calls.some(c => c.command === 'google_sheets_status'));
  assert.ok(e.calls.some(c => c.command === 'publish_sheet_check'));
  assert.equal(e.calls.some(c => c.command === 'publish_create_campaign'), false);
});

test('manual acceptance preflight authorizes its exact durable start ID before IPC', async t => {
  const e = environment(t);
  const invoke = async (command, args) => {
    if (command !== 'publish_preflight') return e.invoke(command, args);
    const scope = JSON.parse(fs.readFileSync(e.devScope, 'utf8'));
    const allowed = typeof args?.requestId === 'string'
      && scope.campaignIds.length === 1 && scope.campaignIds[0] === args.requestId
      && scope.deviceIds.join(',') === 'phone-b,phone-a';
    const response = await e.invoke(command, args);
    return { ...response, canExecute: allowed,
      startBlock: allowed ? null : 'AcceptanceScopeDenied' };
  };
  const options = e.options('preflight');
  options.protocol = 'start';
  const result = await runAcceptance(options, { invoke });
  assert.equal(result.exitCode, 0, result.report.error);
  const requestId = e.calls.find(call => call.command === 'publish_preflight').args.requestId;
  assert.match(requestId, /^[a-f0-9-]{36}$/);
  assert.equal(JSON.parse(fs.readFileSync(e.devScope, 'utf8')).campaignIds[0], requestId);
});

test('new acceptance defaults to Start while historical observe stays readable', () => {
  const args = ['--report-dir', path.resolve('out'), ...base,
    '--dev-scope', path.resolve('scope.json')];
  assert.equal(parseArgs(['--mode', 'preflight', ...args]).protocol, 'start');
  assert.equal(parseArgs(['--mode', 'submit', ...args, '--confirm', 'a'.repeat(64)]).protocol, 'start');
  assert.equal(parseArgs(['--mode', 'observe', '--report-dir', path.resolve('out'),
    '--udids', 'phone-b,phone-a', '--campaign-id', 'old-campaign']).protocol, 'legacy');
});

test('start protocol saves intent before Start and only reads status after lost ACK', async t => {
  const e = environment(t);
  let releaseAccount, accountGate, accountFailure = false, accountSettled = false;
  const accountStarts = [];
  let status;
  let loseAck = true;
  let statusUnavailable = true;
  const invoke = async (command, args) => {
    if (command === 'interaction_read_account' && accountGate) {
      accountStarts.push(args.udid);
      if (args.udid === 'phone-a') {
        await accountGate;
        accountSettled = true;
      }
      const reading = await e.invoke(command, args);
      if (accountFailure && args.udid === 'phone-b') throw new Error('account read refused');
      return reading;
    }
    if (command === 'publish_preflight') {
      const response = await e.invoke(command, args);
      response.assignments.forEach((row, i) => { row.udid = args.request.udids[i]; });
      return response;
    }
    if (command === 'publish_start') {
      assert.equal(accountSettled, true, 'Start waits for every account read');
      e.calls.push({ command, args: structuredClone(args) });
      const intent = JSON.parse(fs.readFileSync(path.join(e.dir, 'start-intent.json'), 'utf8'));
      assert.equal(intent.requestId, args.requestId);
      assert.deepEqual(args.request, intent.request);
      assert.equal(JSON.parse(fs.readFileSync(e.devScope, 'utf8')).campaignIds[0],
        args.requestId);
      status = { requestId: args.requestId, reservedCampaignId: args.requestId,
        campaignId: args.requestId, inputDigest: args.approvedInputDigest,
        state: 'queued', stage: 'queued', error: null };
      e.detail.campaign.id = args.requestId;
      e.detail.campaign.requestId = args.requestId;
      e.detail.assignments.forEach((a, index) => {
        a.udid = args.request.udids[index];
        a.campaignId = args.requestId;
        a.bundleId = `${args.requestId}:${args.request.bundleIds[index]}`;
      });
      if (loseAck) { loseAck = false; throw new Error('start ACK lost'); }
      return structuredClone(status);
    }
    if (command === 'publish_start_status') {
      e.calls.push({ command, args: structuredClone(args) });
      if (statusUnavailable) { statusUnavailable = false; return null; }
      return structuredClone(status);
    }
    if (command === 'publish_get') {
      e.calls.push({ command, args: structuredClone(args) });
      assert.equal(args.campaignId, status.campaignId);
      return structuredClone(e.detail);
    }
    return e.invoke(command, args);
  };
  const preflight = e.options('preflight');
  preflight.protocol = 'start';
  preflight.udids = ['phone-a', 'phone-b'];
  const policy = JSON.parse(fs.readFileSync(e.devScope, 'utf8'));
  policy.deviceIds = preflight.udids;
  fs.writeFileSync(e.devScope, JSON.stringify(policy));
  const prepared = await runAcceptance(preflight, { invoke });
  assert.equal(prepared.exitCode, 0, prepared.report.error);
  const submit = e.options('submit', ['--confirm', prepared.report.confirmation]);
  submit.protocol = 'start';
  submit.udids = preflight.udids;
  accountGate = new Promise(resolve => { releaseAccount = resolve; });
  accountFailure = true;
  let returned = false;
  const failedRun = runAcceptance(submit, { invoke }).then(result => { returned = true; return result; });
  await new Promise(resolve => setImmediate(resolve));
  // Drain the blocked read even when an assertion exposes the serial baseline.
  const startedWhileBlocked = [...accountStarts];
  assert.equal(returned, false, 'a failed sibling must drain the slow read before returning');
  assert.equal(e.calls.some(call => call.command === 'publish_start'), false);
  releaseAccount();
  const failed = await failedRun;
  assert.deepEqual(startedWhileBlocked, ['phone-a', 'phone-b'], 'B starts before slow A resolves');
  assert.equal(failed.exitCode, 1);
  assert.equal(accountSettled, true);
  assert.equal(fs.existsSync(path.join(e.dir, 'start-intent.json')), false);
  accountFailure = false;
  accountStarts.length = 0;
  accountSettled = false;
  accountGate = new Promise(resolve => { releaseAccount = resolve; });
  const firstRun = runAcceptance(submit, { invoke });
  await new Promise(resolve => setImmediate(resolve));
  assert.deepEqual(accountStarts, ['phone-a', 'phone-b']);
  assert.equal(e.calls.some(call => call.command === 'publish_start'), false);
  releaseAccount();
  const first = await firstRun;
  accountGate = null;
  assert.equal(first.exitCode, 2, first.report.error);
  assert.equal(first.report.acceptance, 'ackUnknown');
  assert.equal(e.calls.filter(call => call.command === 'publish_start').length, 1);
  const second = await runAcceptance(submit, { invoke });
  assert.equal(second.exitCode, 2, second.report.error);
  assert.equal(e.calls.filter(call => call.command === 'publish_start').length, 1);
  assert.equal(e.calls.some(call => call.command === 'publish_create_campaign'
    || call.command === 'publish_execute'), false);
  assert.equal(second.report.start, 'alreadyIntendedObserveOnly');
  const observe = e.options('observe', ['--campaign-id', status.campaignId]);
  observe.udids = preflight.udids;
  const observed = await runAcceptance(observe, { invoke });
  assert.equal(observed.exitCode, 2, observed.report.error);
  assert.equal(e.calls.filter(call => call.command === 'publish_start').length, 1);
  assert.equal(observed.report.campaignId, status.campaignId);
  const wrong = e.options('observe', ['--campaign-id', 'foreign-campaign']);
  const refused = await runAcceptance(wrong, { invoke });
  assert.equal(refused.exitCode, 1);
  assert.equal(e.calls.filter(call => call.command === 'publish_start').length, 1);
  let clock = 0;
  const statusReadsBefore = e.calls.filter(call => call.command === 'publish_start_status').length;
  const waiting = e.options('observe', ['--campaign-id', status.campaignId,
    '--wait-seconds', '10', '--poll-seconds', '5']);
  waiting.udids = preflight.udids;
  const waited = await runAcceptance(waiting, {
    invoke, now: () => clock, sleep: async ms => { clock += ms; },
  });
  assert.equal(waited.exitCode, 2);
  assert.equal(waited.report.acceptance, 'pendingDeadline');
  assert.equal(e.calls.filter(call => call.command === 'publish_start_status').length
    - statusReadsBefore, 3);
  assert.equal(e.calls.filter(call => call.command === 'publish_start').length, 1);
  e.detail.assignments[0].bundleId = `${status.requestId}:other-bundle`;
  const altered = await runAcceptance(observe, { invoke });
  assert.equal(altered.exitCode, 1);
  assert.match(altered.report.error, /mapping bài-máy/);
  assert.equal(e.calls.filter(call => call.command === 'publish_start').length, 1);
  e.detail.assignments[0].bundleId = status.requestId + ':bundle-b';
  e.detail.assignments[1].id = e.detail.assignments[0].id;
  const duplicated = await runAcceptance(observe, { invoke });
  assert.equal(duplicated.exitCode, 1);
  assert.match(duplicated.report.error, /mapping bài-máy/);
  assert.equal(e.calls.filter(call => call.command === 'publish_start').length, 1);
});

test('active Sheet can preflight without a last Picker selection', async t => {
  const e = environment(t);
  const invoke = async (command, args) => {
    const value = await e.invoke(command, args);
    if (command === 'google_sheets_status') value.selectedFileId = null;
    return value;
  };
  const result = await runAcceptance(e.options('preflight'), { invoke });
  assert.equal(result.exitCode, 0);
  assert.equal(result.report.preflight.sheetDelivery.spreadsheetId, 'sheet_fixture');
  assert.ok(e.calls.some(call => call.command === 'publish_sheet_check'));
});

test('Sheet-enabled scoped acceptance keeps handoff, cleanup and Sheet delivery together', async t => {
  const e = environment(t);
  const prepared = await runAcceptance(e.options('preflight'), { invoke: e.invoke });
  assert.equal(prepared.exitCode, 0);
  const submitted = await runAcceptance(e.options('submit', ['--confirm', prepared.report.confirmation]), { invoke: e.invoke });
  assert.equal(submitted.exitCode, 2);
  assert.deepEqual(JSON.parse(fs.readFileSync(e.devScope, 'utf8')),
    { activationId: 'fixture-activation-20260922', campaignIds: ['campaign-fixture'],
      deviceIds: ['phone-b', 'phone-a'], capabilities: { publishVerification: true, sheetDelivery: true, publishCleanup: true } });
  assert.equal(e.calls.filter(c => c.command === 'operation_prepare_devices').length, 1);
  assert.equal(e.calls.filter(c => c.command === 'interaction_read_account').length, 2);
  const created = e.calls.find(c => c.command === 'publish_create_campaign');
  assert.equal(created.args.sheetEnabled, true);
  assert.equal(created.args.deleteAfterPublish, true);
  assert.ok(e.calls.findIndex(c => c.command === 'interaction_read_account')
    < e.calls.findIndex(c => c.command === 'publish_create_campaign'));
});

test('an unassigned phone freezes its observed account and refuses a changed account before Create', async t => {
  for (const changedAfterApproval of [false, true]) {
    const e = environment(t);
    let changed = false;
    const invoke = async (command, args) => {
      const value = await e.invoke(command, args);
      if (command === 'list_device_metas') value.find(row => row.udid === 'phone-b').handle = '';
      if (command === 'interaction_read_account' && args.udid === 'phone-b') {
        return { ...value, expectedHandle: '', observedHandle: changed ? 'other' : 'fixture', status: 'unassigned' };
      }
      return value;
    };
    const prepared = await runAcceptance(e.options('preflight'), { invoke });
    assert.equal(prepared.exitCode, 0, prepared.report.error);
    const approved = JSON.parse(fs.readFileSync(path.join(e.dir, 'preflight.json'), 'utf8')).approval;
    assert.equal(approved.accounts['phone-b'], 'fixture');
    assert.equal(approved.metadataAccounts['phone-b'], '');
    assert.equal(e.calls.some(call => call.command === 'publish_create_campaign'), false);

    changed = changedAfterApproval;
    const submitted = await runAcceptance(e.options('submit', ['--confirm', prepared.report.confirmation]), { invoke });
    assert.equal(submitted.exitCode, changed ? 1 : 2);
    assert.equal(e.calls.filter(call => call.command === 'publish_create_campaign').length, changed ? 0 : 1);
    assert.equal(e.calls.filter(call => call.command === 'publish_execute').length, changed ? 0 : 1);
  }
});

test('real Android inspect refuses mock or absent devices before any effect command', async t => {
  const e = environment(t);
  const options = e.options('inspect', ['--real-android', 'true']);
  const invoke = async (command, args) => {
    if (command === 'list_devices') return [{ udid: 'phone-b', platform: 'ios', connection: 'mock', status: 'ready' }];
    return e.invoke(command, args);
  };
  const result = await runAcceptance(options, { invoke });
  assert.equal(result.exitCode, 1);
  assert.match(result.report.error, /Android USB thật/);
  assert.equal(e.calls.some(call => /preflight|create|execute/.test(call.command)), false);
});

test('real Android inspect accepts the exact connected USB roster without effects', async t => {
  const e = environment(t);
  const options = e.options('inspect', ['--real-android', 'true']);
  const invoke = async (command, args) => {
    if (command === 'list_devices') return [
      { udid: 'phone-a', platform: 'android', connection: 'usb', status: 'connected' },
      { udid: 'phone-b', platform: 'android', connection: 'usb', status: 'ready' },
    ];
    return e.invoke(command, args);
  };
  const result = await runAcceptance(options, { invoke });
  assert.equal(result.exitCode, 0);
  assert.equal(result.report.acceptance, 'notEvaluated');
  assert.equal(e.calls.some(call => /preflight|create|execute/.test(call.command)), false);
});

test('Sheet-disabled preflight and submit cannot create new campaigns', t => {
  const e = environment(t);
  assert.throws(() => e.phoneOnlyOptions('preflight'), /bắt buộc ghi Sheet/);
  assert.throws(() => e.phoneOnlyOptions('submit', ['--confirm', 'a'.repeat(64)]), /bắt buộc ghi Sheet/);
  assert.equal(fs.existsSync(path.join(e.dir, 'create-intent.json')), false);
});

test('historical phone-only campaign remains observable without Sheet or device commands', async t => {
  const e = environment(t);
  const options = e.phoneOnlyOptions('observe', ['--campaign-id', 'campaign-fixture']);
  const approvalScope = { cdp: options.cdp, pageUrl: options.pageUrl, source: options.source,
    contentSnapshot: options.contentSnapshot, udids: options.udids, bundleIds: options.bundleIds,
    sheetId: null, sheetGid: null, sheetDisabled: true, devScope: e.devScope };
  const digest = value => createHash('sha256').update(JSON.stringify(value)).digest('hex');
  const binding = { path: e.devScope,
    contentHash: createHash('sha256').update(fs.readFileSync(e.devScope)).digest('hex'),
    activationId: e.initialDevScope.activationId };
  const approval = { scope: approvalScope, sheetDisabled: true, writer: null, sheetDelivery: null,
    devScope: binding, request: { sheetEnabled: false } };
  const confirmation = digest(approval);
  fs.writeFileSync(path.join(e.dir, 'preflight.json'), JSON.stringify({ approval, confirmation }));
  const args = { requestId: 'legacy-request', bundleIds: options.bundleIds, udids: options.udids };
  fs.writeFileSync(path.join(e.dir, 'create-intent.json'), JSON.stringify({
    confirmation, sheetDisabled: true, args, requestFingerprint: digest({ sheetDisabled: true, args }),
  }));
  const campaign = e.detail.campaign;
  campaign.requestId = args.requestId;
  campaign.assignments.forEach((row, index) => { row.bundleId = `${args.requestId}:${args.bundleIds[index]}`; });
  fs.writeFileSync(path.join(e.dir, 'campaign.json'), JSON.stringify(campaign));
  fs.writeFileSync(e.devScope, JSON.stringify({ activationId: binding.activationId,
    campaignIds: [campaign.id], deviceIds: options.udids,
    capabilities: { publishVerification: true } }));
  for (const row of e.detail.assignments) {
    row.state = 'succeeded';
    row.bundleId = `${args.requestId}:${args.bundleIds[row.ordinal]}`;
    row.evidenceJson = JSON.stringify({ post: { postUrl, publicationVerified: true, state: 'posted' } });
    row.sheetDelivery = null;
  }
  const result = await runAcceptance(options, { invoke: e.invoke });
  assert.equal(result.exitCode, 0);
  assert.equal(result.report.acceptance, 'phoneOnly/sheetDisabled');
  assert.ok(e.calls.every(call => ['list_devices', 'list_device_metas', 'publish_get'].includes(call.command)));
});

test('Sheet-enabled scoped submit reuses handoff, proof, request and Execute intent exactly once', async t => {
  const e = environment(t);
  const prepared = await runAcceptance(e.options('preflight'), { invoke: e.invoke });
  const submitted = await runAcceptance(e.options('submit', ['--confirm', prepared.report.confirmation]), { invoke: e.invoke });
  assert.equal(submitted.exitCode, 2);
  const intent = JSON.parse(fs.readFileSync(path.join(e.dir, 'create-intent.json'), 'utf8'));
  assert.equal(intent.args.sheetEnabled, true);
  assert.equal(intent.args.deleteAfterPublish, true);
  const resumed = await runAcceptance(e.options('submit', ['--confirm', prepared.report.confirmation]), { invoke: e.invoke });
  assert.equal(resumed.exitCode, 2);
  assert.equal(e.calls.filter(c => c.command === 'publish_execute').length, 1);
  assert.equal(e.calls.filter(c => c.command === 'operation_prepare_devices').length, 1);
  assert.equal(e.calls.filter(c => c.command === 'interaction_read_account').length, 2);
  assert.ok(e.calls.findIndex(c => c.command === 'operation_prepare_devices')
    < e.calls.findIndex(c => c.command === 'publish_create_campaign'));
  assert.ok(e.calls.findIndex(c => c.command === 'interaction_read_account')
    < e.calls.findIndex(c => c.command === 'publish_create_campaign'));
});

test('Sheet-enabled handoff failure is durable and blocks campaign creation', async t => {
  const e = environment(t);
  const prepared = await runAcceptance(e.options('preflight'), { invoke: e.invoke });
  e.failHandoff();
  const options = e.options('submit', ['--confirm', prepared.report.confirmation]);
  const first = await runAcceptance(options, { invoke: e.invoke });
  const second = await runAcceptance(options, { invoke: e.invoke });
  assert.equal(first.exitCode, 1);
  assert.equal(second.exitCode, 1);
  assert.equal(e.calls.filter(c => c.command === 'operation_prepare_devices').length, 1);
  assert.equal(e.calls.some(c => c.command === 'publish_create_campaign'), false);
  assert.equal(fs.existsSync(path.join(e.dir, 'create-intent.json')), false);
});

test('Sheet-enabled lost handoff ACK is never replayed and never creates a campaign', async t => {
  const e = environment(t);
  const prepared = await runAcceptance(e.options('preflight'), { invoke: e.invoke });
  e.loseHandoff();
  const options = e.options('submit', ['--confirm', prepared.report.confirmation]);
  const first = await runAcceptance(options, { invoke: e.invoke });
  const second = await runAcceptance(options, { invoke: e.invoke });
  assert.equal(first.exitCode, 2);
  assert.equal(second.exitCode, 2);
  assert.equal(e.calls.filter(c => c.command === 'operation_prepare_devices').length, 1);
  assert.equal(e.calls.some(c => c.command === 'publish_create_campaign'), false);
});

test('Sheet-enabled account mismatch after handoff blocks campaign creation', async t => {
  const e = environment(t);
  const prepared = await runAcceptance(e.options('preflight'), { invoke: e.invoke });
  e.mismatchAccount();
  const result = await runAcceptance(e.options('submit', [
    '--confirm', prepared.report.confirmation,
  ]), { invoke: e.invoke });
  assert.equal(result.exitCode, 1);
  assert.equal(e.calls.some(c => c.command === 'operation_prepare_devices'), true);
  assert.equal(e.calls.some(c => c.command === 'publish_create_campaign'), false);
  assert.equal(fs.existsSync(path.join(e.dir, 'create-intent.json')), false);
});

test('new campaigns require Sheet target and absolute manual scope', () => {
  const common = ['--mode', 'preflight', '--report-dir', 'out', '--udids', 'phone-a',
    '--source', source, '--bundle-ids', 'bundle-a'];
  assert.throws(() => parseArgs([...common, '--sheet-disabled', 'false']));
  assert.throws(() => parseArgs([...common, '--sheet-disabled', 'true', '--sheet-id', 'sheet_fixture', '--sheet-gid', '0']));
  assert.throws(() => parseArgs(common));
  assert.throws(() => parseArgs(['--mode', 'preflight', '--report-dir', 'out',
    '--udids', 'phone-a,phone-b,phone-c', '--source', source, '--bundle-ids', 'bundle-a,bundle-b,bundle-c',
    '--sheet-disabled', 'true']));
  assert.throws(() => parseArgs([...common, '--sheet-disabled', 'true', '--dev-scope', 'relative.json']));
  assert.throws(() => parseArgs([...common, '--sheet-id', 'sheet_fixture', '--sheet-gid', '0',
    '--dev-scope', 'relative.json']));
  assert.throws(() => parseArgs([...common, '--sheet-id', 'sheet_fixture', '--sheet-gid', '0']),
    /--dev-scope/);
  assert.equal(parseArgs([...common, '--sheet-id', 'sheet_fixture', '--sheet-gid', '0',
    '--dev-scope', path.resolve('scope.json')]).sheetDisabled, false);
});

test('Sheet-enabled scope rejects drift and never creates or executes', async t => {
  const e = environment(t);
  const prepared = await runAcceptance(e.options('preflight'), { invoke: e.invoke });
  fs.writeFileSync(e.devScope, JSON.stringify(e.initialDevScope, null, 2) + '\n');
  const before = e.calls.length;
  const result = await runAcceptance(e.options('submit', ['--confirm', prepared.report.confirmation]), { invoke: e.invoke });
  assert.equal(result.exitCode, 1);
  assert.equal(e.calls.slice(before).some(c => ['publish_create_campaign', 'publish_execute'].includes(c.command)), false);
});

test('Sheet-enabled scope replacement failure persists campaign receipt but never executes', async t => {
  const e = environment(t);
  const prepared = await runAcceptance(e.options('preflight'), { invoke: e.invoke });
  const rename = fs.renameSync;
  fs.renameSync = (from, to) => {
    if (to === e.devScope) throw new Error('scope replace failed');
    return rename(from, to);
  };
  let result;
  try {
    result = await runAcceptance(e.options('submit', ['--confirm', prepared.report.confirmation]), { invoke: e.invoke });
  } finally { fs.renameSync = rename; }
  assert.equal(result.exitCode, 1);
  assert.equal(fs.existsSync(path.join(e.dir, 'campaign.json')), true);
  assert.equal(e.calls.some(c => c.command === 'publish_create_campaign'), true);
  assert.equal(e.calls.some(c => c.command === 'publish_execute'), false);
  assert.deepEqual(JSON.parse(fs.readFileSync(e.devScope, 'utf8')), e.initialDevScope);
});

test('Sheet-enabled preflight accepts only the exact inactive DevAcceptancePolicy schema', async t => {
  const e = environment(t);
  for (const policy of [
    { campaignIds: ['other'], deviceIds: ['phone-b', 'phone-a'] },
    { campaignIds: [], deviceIds: ['phone-a', 'phone-b'] },
    { campaignIds: [], deviceIds: ['phone-b', 'phone-a'], capabilities: { publishVerification: true } },
    { campaignIds: [], deviceIds: ['phone-b', 'phone-a'], capabilities: {}, extra: true },
  ]) {
    fs.writeFileSync(e.devScope, JSON.stringify(policy) + '\n');
    const result = await runAcceptance(e.options('preflight'), { invoke: e.invoke });
    assert.equal(result.exitCode, 1);
    assert.equal(fs.existsSync(path.join(e.dir, 'preflight.json')), false);
  }
});

test('failed preflight never removes selected devices to pretend complete coverage', async t => {
  const e = environment(t);
  e.preflight.canExecute = false;
  e.preflight.issues = [{ code: 'busy', udid: 'phone-a', message: 'held by upload' }];
  const result = await runAcceptance(e.options('preflight'), { invoke: e.invoke });
  assert.equal(result.exitCode, 1);
  assert.equal(result.report.roster.requestedCount, 2);
  assert.equal(result.report.confirmation, undefined);
  assert.equal(e.calls.some(c => c.command === 'publish_create_campaign'), false);
});

test('lost create ACK resumes same durable request and never creates a different campaign', async t => {
  const e = environment(t);
  const prepared = await runAcceptance(e.options('preflight'), { invoke: e.invoke });
  const options = e.options('submit', ['--confirm', prepared.report.confirmation]);
  e.loseCreate();
  const first = await runAcceptance(options, { invoke: e.invoke });
  assert.equal(first.exitCode, 2);
  assert.equal(e.calls.some(c => c.command === 'publish_execute'), false);
  const firstIntent = fs.readFileSync(path.join(e.dir, 'create-intent.json'), 'utf8');
  const resumed = await runAcceptance(options, { invoke: e.invoke });
  assert.equal(resumed.exitCode, 2, 'submitted pending is not complete');
  assert.equal(fs.readFileSync(path.join(e.dir, 'create-intent.json'), 'utf8'), firstIntent);
  assert.equal(e.calls.filter(c => c.command === 'publish_create_campaign').length, 2);
  assert.equal(e.calls.filter(c => c.command === 'publish_execute').length, 1);
  assert.deepEqual(resumed.report.counts, { requested: 2, enqueued: 2, submitted: 2, verified: 0, sheetSent: 0, urlReadback: 0, mediaCleaned: 0 });
});

test('lost execute ACK and subsequent submit only observe; persisted intent is never replayed', async t => {
  const e = environment(t);
  const prepared = await runAcceptance(e.options('preflight'), { invoke: e.invoke });
  const options = e.options('submit', ['--confirm', prepared.report.confirmation]);
  e.loseExecute();
  await runAcceptance(options, { invoke: e.invoke });
  const previous = e.calls.length;
  const result = await runAcceptance(options, { invoke: e.invoke });
  assert.equal(result.exitCode, 2);
  assert.ok(e.calls.slice(previous).some(c => c.command === 'publish_get'));
  assert.equal(e.calls.filter(c => c.command === 'publish_execute').length, 1);
  assert.equal(e.calls.filter(c => c.command === 'publish_create_campaign').length, 1);
});

test('changed scope or confirmation cannot reuse a saved create intent', async t => {
  const e = environment(t);
  const prepared = await runAcceptance(e.options('preflight'), { invoke: e.invoke });
  const good = e.options('submit', ['--confirm', prepared.report.confirmation]);
  e.loseCreate();
  await runAcceptance(good, { invoke: e.invoke });
  const before = e.calls.length;
  const changed = { ...good, udids: ['other', 'phone-a'] };
  assert.equal((await runAcceptance(changed, { invoke: e.invoke })).exitCode, 1);
  assert.equal((await runAcceptance({ ...good, confirm: '0'.repeat(64) }, { invoke: e.invoke })).exitCode, 1);
  assert.equal(e.calls.slice(before).some(c => /create|execute/.test(c.command)), false);
});

test('a successful-looking state or URL never substitutes for canonical proof', async t => {
  const e = environment(t);
  e.detail.assignments[0].state = 'succeeded';
  e.detail.assignments[0].evidenceJson = JSON.stringify({ post: { postUrl, publicationVerified: false } });
  const result = await runAcceptance(e.options('observe', ['--campaign-id', 'campaign-fixture']), { invoke: e.invoke });
  assert.equal(result.report.counts.verified, 0);
  assert.notEqual(result.exitCode, 0);
  assert.ok(e.calls.every(c => ['list_devices', 'list_device_metas', 'publish_get'].includes(c.command)));
});

test('Sheet sent remains distinct from authenticated URL readback when the cell is unavailable', async t => {
  const e = environment(t);
  for (const row of e.detail.assignments) {
    row.state = 'succeeded';
    row.evidenceJson = JSON.stringify({ post: { postUrl, publicationVerified: true, state: 'posted',
      expectedAccount: 'fixture', submittedAt: '2026-09-17T00:00:30Z' } });
    row.sheetDelivery.state = 'sent';
  }
  const result = await runAcceptance(e.options('observe', ['--campaign-id', 'campaign-fixture']), { invoke: e.invoke });
  assert.equal(result.report.counts.verified, 2);
  assert.equal(result.report.counts.sheetSent, 2);
  assert.equal(result.report.counts.urlReadback, 0);
  assert.equal(result.report.rows[0].urlReadback, 'unsupported');
  assert.equal(result.exitCode, 3);
});

test('existing submission without local Execute intent is observed, never executed again', async t => {
  const e = environment(t);
  const prepared = await runAcceptance(e.options('preflight'), { invoke: e.invoke });
  const invoke = async (cmd, args) => {
    const response = await e.invoke(cmd, args);
    if (cmd === 'publish_get') {
      response.assignments[0].state = 'verifying';
      response.assignments[0].effectIntent = JSON.stringify({ effectIntent: 'post', expectedAccount: 'fixture', submittedAt: '2026-09-17T00:00:30Z' });
      response.assignments[0].evidenceJson = JSON.stringify({ post: { state: 'submitted', verdict: 'Submitted' } });
    }
    return response;
  };
  const result = await runAcceptance(e.options('submit', ['--confirm', prepared.report.confirmation]), { invoke });
  assert.equal(e.calls.some(c => c.command === 'publish_execute'), false);
  assert.equal(fs.existsSync(path.join(e.dir, 'execute-intent.json')), false);
  assert.equal(result.report.execute, 'existingWorkObserveOnly');
});

test('report write remains under lock even when persistence fails', async t => {
  const e = environment(t);
  const originalRename = fs.renameSync;
  let sawReport = false;
  fs.renameSync = (from, to) => {
    if (to === path.join(e.dir, 'report.json')) {
      sawReport = true;
      assert.equal(fs.existsSync(path.join(e.dir, 'harness.lock')), true, 'report replacement owns the same lock');
      throw new Error('report disk failure');
    }
    return originalRename(from, to);
  };
  try { await assert.rejects(runAcceptance(e.options(), { invoke: e.invoke }), /report disk failure/); }
  finally { fs.renameSync = originalRename; }
  assert.equal(sawReport, true);
  assert.equal(fs.existsSync(path.join(e.dir, 'harness.lock')), false, 'failed report still releases lock');
});

test('tampered durable request is rejected before any create replay', async t => {
  const e = environment(t);
  const prepared = await runAcceptance(e.options('preflight'), { invoke: e.invoke });
  const options = e.options('submit', ['--confirm', prepared.report.confirmation]);
  e.loseCreate();
  await runAcceptance(options, { invoke: e.invoke });
  const file = path.join(e.dir, 'create-intent.json');
  const intent = JSON.parse(fs.readFileSync(file, 'utf8'));
  intent.args.udids = ['foreign-phone', 'phone-a'];
  fs.writeFileSync(file, JSON.stringify(intent));
  const before = e.calls.length;
  assert.equal((await runAcceptance(options, { invoke: e.invoke })).exitCode, 1);
  assert.equal(e.calls.slice(before).some(c => /create|execute/.test(c.command)), false);
});

test('foreign content in a create receipt cannot be executed on otherwise matching machines', async t => {
  const e = environment(t);
  const prepared = await runAcceptance(e.options('preflight'), { invoke: e.invoke });
  const invoke = async (cmd, args) => {
    const response = await e.invoke(cmd, args);
    if (cmd === 'publish_create_campaign') response.assignments[0].bundleId = 'foreign-content';
    return response;
  };
  const result = await runAcceptance(e.options('submit', ['--confirm', prepared.report.confirmation]), { invoke });
  assert.equal(result.exitCode, 1);
  assert.equal(e.calls.some(c => c.command === 'publish_execute'), false);
});

test('empty or altered assignment identity on get blocks execute', async t => {
  const e = environment(t);
  const prepared = await runAcceptance(e.options('preflight'), { invoke: e.invoke });
  const invoke = async (cmd, args) => {
    const response = await e.invoke(cmd, args);
    if (cmd === 'publish_get') response.assignments[0].id = '';
    return response;
  };
  const result = await runAcceptance(e.options('submit', ['--confirm', prepared.report.confirmation]), { invoke });
  assert.equal(result.exitCode, 1);
  assert.equal(e.calls.some(c => c.command === 'publish_execute'), false);
});

test('intent without a Submitted receipt is not reported as a submitted post', async t => {
  const e = environment(t);
  e.detail.assignments[0].evidenceJson = null;
  const result = await runAcceptance(e.options('observe', ['--campaign-id', 'campaign-fixture']), { invoke: e.invoke });
  assert.equal(result.report.counts.submitted, 1);
  assert.equal(result.report.rows[0].submitted, false);
});

test('blocked verification review is not mislabeled as ordinary pending', async t => {
  const e = environment(t);
  e.detail.assignments[0].evidenceJson = JSON.stringify({ post: { state: 'submitted' }, verificationStatus: { state: 'needsReview', reasonCode: 'missingIdentity' } });
  const result = await runAcceptance(e.options('observe', ['--campaign-id', 'campaign-fixture']), { invoke: e.invoke });
  assert.equal(result.exitCode, 1);
});

test('persistence error before execute leaves campaign but never dispatches', async t => {
  const e = environment(t);
  const prepared = await runAcceptance(e.options('preflight'), { invoke: e.invoke });
  const invoke = async (cmd, args) => {
    const response = await e.invoke(cmd, args);
    if (cmd === 'publish_get') fs.mkdirSync(path.join(e.dir, 'execute-intent.json'), { recursive: true });
    return response;
  };
  const result = await runAcceptance(e.options('submit', ['--confirm', prepared.report.confirmation]), { invoke });
  assert.equal(result.exitCode, 1);
  assert.ok(fs.existsSync(path.join(e.dir, 'campaign.json')));
  assert.equal(e.calls.some(c => c.command === 'publish_execute'), false);
});

test('a live harness lock blocks every IPC rather than racing another submit', async t => {
  const e = environment(t);
  fs.writeFileSync(path.join(e.dir, 'harness.lock'), 'another owner');
  const result = await runAcceptance(e.options(), { invoke: e.invoke });
  assert.equal(result.exitCode, 1);
  assert.deepEqual(e.calls, []);
});

test('connection settings reject credentials or nonloopback page selectors', () => {
  for (const url of ['https://example.com/app', 'http://user:pass@localhost:5173/']) {
    assert.throws(() => parseArgs(['--report-dir', 'out', '--udids', 'a', '--page-url', url]));
  }
});

test('observe deadline only reads persisted status and never commands device verification', async t => {
  const e = environment(t);
  let clock = 1000;
  const result = await runAcceptance(e.options('observe', ['--campaign-id', 'campaign-fixture', '--wait-seconds', '10', '--poll-seconds', '5']),
    { invoke: e.invoke, now: () => clock, sleep: async ms => { clock += ms; } });
  assert.equal(result.exitCode, 2);
  assert.equal(result.report.acceptance, 'pendingDeadline');
  assert.equal(e.calls.filter(c => c.command === 'publish_get').length, 3);
  assert.ok(e.calls.every(c => ['list_devices', 'list_device_metas', 'publish_get'].includes(c.command)));
});

test('invalid or changed writer epoch prevents create even with approved hash', async t => {
  const e = environment(t);
  const prepared = await runAcceptance(e.options('preflight'), { invoke: e.invoke });
  const invoke = async (cmd, args) => {
    const response = await e.invoke(cmd, args);
    return cmd === 'publish_sheet_check' ? { ...response, reportingEpoch: 'epoch-2' } : response;
  };
  const result = await runAcceptance(e.options('submit', ['--confirm', prepared.report.confirmation]), { invoke });
  assert.equal(result.exitCode, 1);
  assert.equal(fs.existsSync(path.join(e.dir, 'create-intent.json')), false);
});

test('CDP refuses ambiguous pages and disconnects without touching any page', async () => {
  let disconnected = false;
  const page = { url: () => 'http://localhost:5173/', evaluate: () => assert.fail('ambiguous IPC') };
  const chromium = { connectOverCDP: async () => ({ contexts: () => [{ pages: () => [page, page] }],
    close: async () => { disconnected = true; } }) };
  await assert.rejects(connectIPC({ cdp: 'http://127.0.0.1:9277' }, chromium), /WebView/);
  assert.equal(disconnected, true);
});

test('CDP adapter only invokes allowlisted production IPC and never starts a browser', async () => {
  const seen = [];
  let rejected = false;
  const page = { url: () => 'http://tauri.localhost/', evaluate: async (fn, args) => {
    seen.push(args);
    if (!args) return true;
    const previous = globalThis.window;
    globalThis.window = { __TAURI_INTERNALS__: { invoke: async () => {
      if (rejected) throw { code: 'pending_publish_guard', message: 'Pending publication requires reconciliation',
        token: 'secret-token', credentials: { password: 'secret-password' }, details: 'private-details' };
      return { campaign: { id: args.args.campaignId } };
    } } };
    try { return await fn(args); }
    finally { globalThis.window = previous; }
  } };
  let disconnected = false;
  const chromium = { connectOverCDP: async () => ({ contexts: () => [{ pages: () => [page] }],
    close: async () => { disconnected = true; } }) };
  const ipc = await connectIPC({ cdp: 'http://127.0.0.1:9277' }, chromium);
  assert.deepEqual(await ipc.invoke('publish_get', { campaignId: 'fixture' }), { campaign: { id: 'fixture' } });
  rejected = true;
  await assert.rejects(ipc.invoke('interaction_read_account', { udid: 'phone-a', token: 'input-secret' }), error => {
    const safe = JSON.parse(error.message);
    assert.deepEqual(safe, { command: 'interaction_read_account', udid: 'phone-a',
      code: 'pending_publish_guard', message: 'Pending publication requires reconciliation' });
    assert.doesNotMatch(error.message, /secret|password|credentials|details/);
    return true;
  });
  assert.throws(() => ipc.invoke('terminate_app', {}));
  await ipc.close();
  assert.equal(disconnected, true);
  assert.deepEqual(seen[1], { command: 'publish_get', args: { campaignId: 'fixture' } });
});

test('an armed execute intent alone forbids create even when campaign receipt is missing', async t => {
  const e = environment(t);
  const prepared = await runAcceptance(e.options('preflight'), { invoke: e.invoke });
  const opts = e.options('submit', ['--confirm', prepared.report.confirmation]);
  await runAcceptance(opts, { invoke: e.invoke });
  fs.unlinkSync(path.join(e.dir, 'campaign.json'));
  const previous = e.calls.length;
  const result = await runAcceptance(opts, { invoke: e.invoke });
  assert.equal(result.exitCode, 1);
  assert.equal(e.calls.slice(previous).some(c => /create|execute/.test(c.command)), false);
});

test('empty assignments and failed rows are blocked, not vacuous success or endless pending', async t => {
  const e = environment(t);
  e.detail.assignments = [];
  assert.equal((await runAcceptance(e.options('observe', ['--campaign-id', 'campaign-fixture']), { invoke: e.invoke })).exitCode, 1);
  e.detail.assignments = [{ id: 'failed', campaignId: 'campaign-fixture', udid: 'phone-b', state: 'failedBeforeDispatch', errorCode: 'pickerRefused' }];
  assert.equal((await runAcceptance(e.options('observe', ['--campaign-id', 'campaign-fixture']), { invoke: e.invoke })).exitCode, 1);
});

test('tampered create intent cannot disable Sheet or verified media cleanup', async t => {
  for (const field of ['sheetEnabled', 'deleteAfterPublish']) {
    const e = environment(t);
    const prepared = await runAcceptance(e.options('preflight'), { invoke: e.invoke });
    const options = e.options('submit', ['--confirm', prepared.report.confirmation]);
    e.loseCreate();
    await runAcceptance(options, { invoke: e.invoke });
    const file = path.join(e.dir, 'create-intent.json');
    const intent = JSON.parse(fs.readFileSync(file, 'utf8'));
    intent.args[field] = false;
    intent.requestFingerprint = createHash('sha256').update(JSON.stringify(intent.args)).digest('hex');
    fs.writeFileSync(file, JSON.stringify(intent));
    const before = e.calls.length;
    assert.equal((await runAcceptance(options, { invoke: e.invoke })).exitCode, 1);
    assert.equal(e.calls.slice(before).some(call => ['publish_create_campaign', 'publish_execute'].includes(call.command)), false);
  }
});

test('phone-only observe rejects a foreign Sheet-enabled campaign despite the CLI flag', async t => {
  const e = environment(t);
  for (const row of e.detail.assignments) {
    row.state = 'succeeded';
    row.evidenceJson = JSON.stringify({ post: { postUrl, publicationVerified: true, state: 'posted' } });
  }
  const result = await runAcceptance(e.phoneOnlyOptions('observe', [
    '--campaign-id', 'campaign-fixture',
  ]), { invoke: e.invoke });
  assert.equal(result.exitCode, 1);
  assert.equal(result.report.acceptance, 'blockedFailed');
  assert.notEqual(result.report.acceptance, 'phoneOnly/sheetDisabled');
});

test('observe keeps polling an active retry whose prior failure projection has not settled', async t => {
  const e = environment(t);
  let clock = 1000, gets = 0;
  const invoke = async (cmd, args) => {
    const result = await e.invoke(cmd, args);
    if (cmd === 'publish_get' && ++gets === 1) {
      Object.assign(result.assignments[0], { state: 'failedBeforeDispatch', effectIntent: null,
        evidenceJson: null, errorCode: 'post_refused_before_dispatch',
        dispatch: { phase: 'compose', state: 'running', revision: 3 } });
    }
    return result;
  };
  const result = await runAcceptance(e.options('observe', ['--campaign-id', 'campaign-fixture', '--wait-seconds', '10', '--poll-seconds', '5']),
    { invoke, now: () => clock, sleep: async ms => { clock += ms; } });
  assert.equal(gets, 3);
  assert.equal(result.exitCode, 2);
  assert.equal(result.report.acceptance, 'pendingDeadline');
  assert.ok(e.calls.every(c => ['list_devices', 'list_device_metas', 'publish_get'].includes(c.command)));
});

test('a terminal retry failure remains failed and is never polled as active work', async t => {
  const e = environment(t);
  Object.assign(e.detail.assignments[0], { state: 'failedBeforeDispatch', effectIntent: null,
    evidenceJson: null, dispatch: { phase: 'compose', state: 'finished', revision: 3 } });
  const result = await runAcceptance(e.options('observe', ['--campaign-id', 'campaign-fixture', '--wait-seconds', '10']), { invoke: e.invoke });
  assert.equal(result.exitCode, 1);
  assert.equal(e.calls.filter(c => c.command === 'publish_get').length, 1);
});

test('a changed assigned account after approval blocks create and Execute', async t => {
  const e = environment(t);
  const approved = await runAcceptance(e.options('preflight'), { invoke: e.invoke });
  const before = e.calls.length;
  const invoke = async (command, args) => {
    const value = await e.invoke(command, args);
    if (command === 'list_device_metas') value[0].handle = 'changed.after.approval';
    return value;
  };
  const result = await runAcceptance(e.options('submit', ['--confirm', approved.report.confirmation]), { invoke });
  assert.equal(result.exitCode, 1);
  assert.equal(e.calls.slice(before).some(c => /create|execute/.test(c.command)), false);
});

test('acceptance needs matching canonical proof, durable receipt and authenticated cell', async t => {
  const e = environment(t);
  for (const row of e.detail.assignments) {
    row.state = 'succeeded';
    row.evidenceJson = JSON.stringify({ post: { postUrl, publicationVerified: true, state: 'posted' } });
    row.sheetDelivery = { state: 'sent', revision: 4 };
  }
  const invoke = async (command, args) => command === 'publish_sheet_readback'
    ? { assignmentId: args.assignmentId, url: postUrl, revision: 4, epoch: 'epoch-1', range: 'gid=0:D2', checkedAt: '2026-09-19T00:00:00Z',
      receipt: { publicationId: args.assignmentId, postUrl, revision: 4, reportingEpoch: 'epoch-1' } }
    : e.invoke(command, args);
  const result = await runAcceptance(e.options('observe', ['--campaign-id', 'campaign-fixture']), { invoke });
  assert.equal(result.exitCode, 0);
  assert.equal(result.report.acceptance, 'passed');
  assert.equal(result.report.counts.urlReadback, 2);
  e.detail.campaign.cleanupPolicy = 'deleteImportedAssetsAfterVerified';
  const pendingCleanup = await runAcceptance(e.options('observe', ['--campaign-id', 'campaign-fixture']), { invoke });
  assert.equal(pendingCleanup.exitCode, 2);
  assert.equal(pendingCleanup.report.acceptance, 'pendingCleanup');
  for (const row of e.detail.assignments) {
    row.evidenceJson = JSON.stringify({ post: { postUrl, publicationVerified: true, state: 'posted', importId: `import-${row.id}` },
      cleanup: { state: 'cleaned', importId: `import-${row.id}` } });
  }
  const cleaned = await runAcceptance(e.options('observe', ['--campaign-id', 'campaign-fixture']), { invoke });
  assert.equal(cleaned.exitCode, 0);
  assert.equal(cleaned.report.counts.mediaCleaned, 2);
  e.detail.assignments[0].evidenceJson = JSON.stringify({ postUrl, publicationVerified: true, state: 'posted',
    priorEvidenceJson: JSON.stringify({ nativeImport: { state: 'imported', importId: 'import-assignment-0' } }),
    cleanup: { state: 'cleaned', importId: 'import-assignment-0', source: 'verified_deferred' } });
  const recovered = await runAcceptance(e.options('observe', ['--campaign-id', 'campaign-fixture']), { invoke });
  assert.equal(recovered.exitCode, 0, 'verified deferred cleanup from an uncertain Post keeps its native import proof');
  assert.equal(recovered.report.counts.mediaCleaned, 2);
  const wrongAccount = async (command, args) => {
    const value = await invoke(command, args);
    if (command === 'list_device_metas') for (const row of value) row.handle = 'different.account';
    return value;
  };
  assert.notEqual((await runAcceptance(e.options('observe', ['--campaign-id', 'campaign-fixture']), { invoke: wrongAccount })).exitCode, 0);
  const wrong = async (command, args) => {
    const value = await invoke(command, args);
    if (command === 'publish_sheet_readback') value.receipt.publicationId = 'other';
    return value;
  };
  assert.notEqual((await runAcceptance(e.options('observe', ['--campaign-id', 'campaign-fixture']), { invoke: wrong })).exitCode, 0);
});

// Owns scheduled harness routing, not the production clock or scheduler engine.
test('schedule reservation freezes scope and lost Start ACK observes deadline cancel and late start without replay', async t => {
  for (const scenario of ['deadline', 'cancel', 'late-start']) {
    const e = environment(t), at = '2099-10-03T12:00:00';
    const opts = (mode, extra = []) => parseArgs(['--mode', mode, '--protocol', 'start',
      '--report-dir', e.dir, ...base, '--dev-scope', e.devScope, '--run-at', at,
      '--schedule-case', scenario, ...extra]);
    const reserved = await runAcceptance(opts('reserve'), { invoke: () => assert.fail('reserve must be offline') });
    assert.equal(reserved.exitCode, 0, reserved.report.error);
    const reservation = JSON.parse(fs.readFileSync(path.join(e.dir, 'preflight-request.json'), 'utf8'));
    const frozen = JSON.parse(fs.readFileSync(e.devScope, 'utf8'));
    // Root assembles static scope before starting the controller; include a sibling ID/device.
    frozen.campaignIds = [reservation.requestId, 'approved-sibling'];
    frozen.deviceIds.push('sibling-phone');
    frozen.capabilities = { publishVerification: true, sheetDelivery: true, publishCleanup: true, publishSchedule: true };
    fs.writeFileSync(e.devScope, JSON.stringify(frozen));
    const frozenBytes = fs.readFileSync(e.devScope);
    let status, starts = 0, cancels = 0, clock = new Date('2099-10-03T11:59:00').getTime();
    const invoke = async (command, args) => {
      if (command === 'publish_start') {
        starts++;
        assert.equal(args.requestId, reservation.requestId);
        assert.equal(args.request.runAt, at);
        assert.equal(args.confirmed, true);
        status = { requestId: args.requestId, reservedCampaignId: args.requestId,
          campaignId: args.requestId, inputDigest: args.approvedInputDigest, state: 'queued', revision: 3 };
        e.detail.campaign = { ...e.detail.campaign, id: args.requestId, requestId: args.requestId,
          runAt: at, state: 'scheduled' };
        e.detail.events = [{ revision: 1, kind: 'created', payloadJson: JSON.stringify({
          ...args.request, requestId: args.requestId, executionConfirmed: true }) },
        { revision: 4, kind: 'state', payloadJson: JSON.stringify({ state: 'scheduled' }) }];
        for (const [i, row] of e.detail.assignments.entries()) Object.assign(row, {
          campaignId: args.requestId, bundleId: `${args.requestId}:${args.request.bundleIds[i]}`,
          state: 'scheduled', effectIntent: null, evidenceJson: null, dispatch: null });
        throw new Error('Start ACK lost after commit');
      }
      if (command === 'publish_start_status') return structuredClone(status);
      if (command === 'publish_get') return structuredClone(e.detail);
      if (command === 'publish_cancel') {
        cancels++; e.detail.campaign.state = 'cancelled';
        e.detail.events.push({ revision: 5, kind: 'state', payloadJson: JSON.stringify({ state: 'cancelled' }) });
        // Production cancel keeps untouched scheduled assignment rows.
        throw new Error('Cancel ACK lost after commit');
      }
      assert.ok(!['publish_execute', 'publish_create_campaign'].includes(command));
      return e.invoke(command, args);
    };
    const prepared = await runAcceptance(opts('preflight'), { invoke, now: () => clock });
    assert.equal(prepared.exitCode, 0, prepared.report.error);
    const submit = opts('submit', ['--confirm', prepared.report.confirmation]);
    const first = await runAcceptance(submit, { invoke, now: () => clock });
    assert.equal(first.exitCode, 2, first.report.error);
    assert.equal(first.report.start, 'ackUnknownNeverReplay');
    assert.equal(first.report.schedule.revision, 3);
    assert.equal(first.report.schedule.latestCampaignEventRevision, 4);
    const deadline = await runAcceptance(opts('observe', ['--wait-seconds', '10', '--poll-seconds', '5']),
      { invoke, now: () => clock, sleep: async ms => { clock += ms; } });
    assert.equal(deadline.report.acceptance, 'pendingDeadline');
    await runAcceptance(submit, { invoke, now: () => clock });
    assert.equal(starts, 1, 'lost Start ACK is never replayed');
    if (scenario === 'cancel') {
      const cancelled = await runAcceptance(opts('cancel', ['--confirm', prepared.report.confirmation]), { invoke, now: () => clock });
      assert.equal(cancelled.exitCode, 2, 'cancellation must be observed through the due window');
      assert.equal(cancelled.report.cancel, 'ackUnknownNeverReplay');
      await runAcceptance(opts('cancel', ['--confirm', prepared.report.confirmation]), { invoke, now: () => clock });
      assert.equal(cancels, 1, 'lost Cancel ACK is never replayed');
    } else {
      // Supply a production startup receipt. The harness does not emulate a scheduler.
      e.detail.campaign.state = 'missed'; e.detail.campaign.errorCode = scenario === 'deadline' ? 'schedule_capacity_deadline' : 'app_opened_after_deadline';
      // Native miss_publish_schedule currently increments DB revision without appending an event.
      e.detail.assignments.forEach(row => { row.state = 'missed'; });
    }
    clock = new Date(at).getTime() + 31000;
    const observed = await runAcceptance(opts('observe'), { invoke, now: () => clock });
    assert.equal(observed.report.acceptance, scenario === 'cancel' ? 'cancelledNoPublication' : scenario === 'deadline' ? 'deadlineNoPublication' : 'lateStartNoPublication');
    assert.equal(observed.exitCode, 0, observed.report.error);
    assert.equal(observed.report.schedule.revision, 3);
    assert.equal(observed.report.schedule.latestCampaignEventRevision, scenario === 'cancel' ? 5 : 4);
    e.detail.assignments[0].effectIntent = JSON.stringify({ effectIntent: 'post' });
    assert.equal((await runAcceptance(opts('observe'), { invoke, now: () => clock })).exitCode, 1);
    assert.equal(starts, 1);
    assert.deepEqual(fs.readFileSync(e.devScope), frozenBytes, 'running process scope bytes stay frozen');
  }
});
