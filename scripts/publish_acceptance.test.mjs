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
    '--mode', mode, '--report-dir', dir, ...base, ...more]);
  const phoneOnlyOptions = (mode = 'inspect', more = []) => parseArgs([
    '--mode', mode, '--report-dir', dir, '--udids', 'phone-b,phone-a', '--source', source,
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
        if (created?.sheetEnabled === false) assert.deepEqual(JSON.parse(fs.readFileSync(devScope, 'utf8')),
          { activationId, campaignIds: ['campaign-fixture'], deviceIds: ['phone-b', 'phone-a'],
            capabilities: { publishVerification: true } }, 'scope opens verification before Execute');
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
  assert.ok(e.calls.some(c => c.command === 'google_sheets_status'));
  assert.ok(e.calls.some(c => c.command === 'publish_sheet_check'));
  assert.equal(e.calls.some(c => c.command === 'publish_create_campaign'), false);
});

test('phone-only preflight is explicit, durable and never contacts Google or Sheet IPC', async t => {
  const e = environment(t);
  Object.assign(e.preflight, { sheetConfigured: false, sheetEnabled: false, sheetDelivery: null });
  const result = await runAcceptance(e.phoneOnlyOptions('preflight'), { invoke: e.invoke });
  assert.equal(result.exitCode, 0);
  assert.equal(result.report.acceptance, 'preflightOnly');
  assert.equal(result.report.acceptanceScope, 'phoneOnly');
  assert.equal(result.report.sheetAcceptance, 'sheetDisabled');
  assert.ok(e.calls.every(c => !['google_sheets_status', 'publish_sheet_check', 'publish_sheet_readback'].includes(c.command)));
  const request = e.calls.find(c => c.command === 'publish_preflight').args.request;
  assert.equal(request.sheetEnabled, false);
  const prepared = JSON.parse(fs.readFileSync(path.join(e.dir, 'preflight.json'), 'utf8'));
  assert.equal(prepared.approval.scope.sheetDisabled, true);
  assert.equal(prepared.approval.sheetDisabled, true);
  assert.equal(prepared.approval.writer, null);
  assert.equal(prepared.approval.sheetDelivery, null);
  assert.equal(prepared.approval.devScope.path, e.devScope);
  assert.match(prepared.approval.devScope.contentHash, /^[a-f0-9]{64}$/);
});

test('phone-only preflight accepts the backend omitting an optional Sheet target', async t => {
  const e = environment(t);
  e.preflight.sheetEnabled = false;
  delete e.preflight.sheetDelivery;
  const result = await runAcceptance(e.phoneOnlyOptions('preflight'), { invoke: e.invoke });
  assert.equal(result.exitCode, 0);
  const prepared = JSON.parse(fs.readFileSync(path.join(e.dir, 'preflight.json'), 'utf8'));
  assert.equal(prepared.approval.sheetDelivery, null);
});

test('phone-only submit fingerprints the disabled Sheet decision and reports bounded success only', async t => {
  const e = environment(t);
  Object.assign(e.preflight, { sheetConfigured: false, sheetEnabled: false, sheetDelivery: null });
  const prepared = await runAcceptance(e.phoneOnlyOptions('preflight'), { invoke: e.invoke });
  const submitted = await runAcceptance(e.phoneOnlyOptions('submit', ['--confirm', prepared.report.confirmation]), { invoke: e.invoke });
  assert.equal(submitted.exitCode, 2);
  const intent = JSON.parse(fs.readFileSync(path.join(e.dir, 'create-intent.json'), 'utf8'));
  assert.equal(intent.sheetDisabled, true);
  assert.equal(intent.requestFingerprint, createHash('sha256')
    .update(JSON.stringify({ sheetDisabled: true, args: intent.args })).digest('hex'));
  assert.ok(e.calls.every(c => !['google_sheets_status', 'publish_sheet_check', 'publish_sheet_readback'].includes(c.command)));
  assert.deepEqual(JSON.parse(fs.readFileSync(e.devScope, 'utf8')),
    { activationId: 'fixture-activation-20260922', campaignIds: ['campaign-fixture'], deviceIds: ['phone-b', 'phone-a'],
      capabilities: { publishVerification: true } });
  const resumed = await runAcceptance(e.phoneOnlyOptions('submit', ['--confirm', prepared.report.confirmation]), { invoke: e.invoke });
  assert.equal(resumed.exitCode, 2);
  assert.equal(e.calls.filter(c => c.command === 'publish_execute').length, 1,
    'an already-active scope never authorizes Execute replay');
  assert.equal(e.calls.filter(c => c.command === 'operation_prepare_devices').length, 1,
    'a persisted handoff receipt is never replayed');
  assert.ok(e.calls.findIndex(c => c.command === 'operation_prepare_devices')
    < e.calls.findIndex(c => c.command === 'publish_create_campaign'),
  'handoff must precede campaign creation');
  assert.equal(e.calls.filter(c => c.command === 'interaction_read_account').length, 2);
  assert.ok(e.calls.findIndex(c => c.command === 'interaction_read_account')
    < e.calls.findIndex(c => c.command === 'publish_create_campaign'),
  'fresh account proof must precede campaign creation');

  for (const row of e.detail.assignments) {
    row.state = 'succeeded';
    row.evidenceJson = JSON.stringify({ post: { postUrl, publicationVerified: true, state: 'posted' } });
    row.sheetDelivery = null;
  }
  const observed = await runAcceptance(e.phoneOnlyOptions('observe', ['--campaign-id', 'campaign-fixture']), { invoke: e.invoke });
  assert.equal(observed.exitCode, 0);
  assert.equal(observed.report.acceptance, 'phoneOnly/sheetDisabled');
  assert.equal(observed.report.acceptanceScope, 'phoneOnly');
  assert.equal(observed.report.sheetAcceptance, 'sheetDisabled');
  assert.equal(observed.report.deliveryEvidence, 'sheetDisabled');
  assert.notEqual(observed.report.acceptance, 'passed');
  assert.ok(e.calls.every(c => c.command !== 'publish_sheet_readback'));
});

test('phone-only handoff failure is durable and blocks campaign creation', async t => {
  const e = environment(t);
  Object.assign(e.preflight, { sheetConfigured: false, sheetEnabled: false, sheetDelivery: null });
  const prepared = await runAcceptance(e.phoneOnlyOptions('preflight'), { invoke: e.invoke });
  e.failHandoff();
  const options = e.phoneOnlyOptions('submit', ['--confirm', prepared.report.confirmation]);
  const first = await runAcceptance(options, { invoke: e.invoke });
  const second = await runAcceptance(options, { invoke: e.invoke });
  assert.equal(first.exitCode, 1);
  assert.equal(second.exitCode, 1);
  assert.equal(e.calls.filter(c => c.command === 'operation_prepare_devices').length, 1);
  assert.equal(e.calls.some(c => c.command === 'publish_create_campaign'), false);
  assert.equal(fs.existsSync(path.join(e.dir, 'create-intent.json')), false);
});

test('phone-only lost handoff ACK is never replayed and never creates a campaign', async t => {
  const e = environment(t);
  Object.assign(e.preflight, { sheetConfigured: false, sheetEnabled: false, sheetDelivery: null });
  const prepared = await runAcceptance(e.phoneOnlyOptions('preflight'), { invoke: e.invoke });
  e.loseHandoff();
  const options = e.phoneOnlyOptions('submit', ['--confirm', prepared.report.confirmation]);
  const first = await runAcceptance(options, { invoke: e.invoke });
  const second = await runAcceptance(options, { invoke: e.invoke });
  assert.equal(first.exitCode, 2);
  assert.equal(second.exitCode, 2);
  assert.equal(e.calls.filter(c => c.command === 'operation_prepare_devices').length, 1);
  assert.equal(e.calls.some(c => c.command === 'publish_create_campaign'), false);
});

test('phone-only account mismatch after handoff blocks campaign creation', async t => {
  const e = environment(t);
  Object.assign(e.preflight, { sheetConfigured: false, sheetEnabled: false, sheetDelivery: null });
  const prepared = await runAcceptance(e.phoneOnlyOptions('preflight'), { invoke: e.invoke });
  e.mismatchAccount();
  const result = await runAcceptance(e.phoneOnlyOptions('submit', [
    '--confirm', prepared.report.confirmation,
  ]), { invoke: e.invoke });
  assert.equal(result.exitCode, 1);
  assert.equal(e.calls.some(c => c.command === 'operation_prepare_devices'), true);
  assert.equal(e.calls.some(c => c.command === 'publish_create_campaign'), false);
  assert.equal(fs.existsSync(path.join(e.dir, 'create-intent.json')), false);
});

test('Sheet-disabled mode rejects ambiguous false values, Sheet targets and implicit omission', () => {
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
    '--dev-scope', path.resolve('scope.json')]));
});

test('phone-only scope rejects drift and never creates or executes', async t => {
  const e = environment(t);
  Object.assign(e.preflight, { sheetConfigured: false, sheetEnabled: false, sheetDelivery: null });
  const prepared = await runAcceptance(e.phoneOnlyOptions('preflight'), { invoke: e.invoke });
  fs.writeFileSync(e.devScope, JSON.stringify(e.initialDevScope, null, 2) + '\n');
  const before = e.calls.length;
  const result = await runAcceptance(e.phoneOnlyOptions('submit', ['--confirm', prepared.report.confirmation]), { invoke: e.invoke });
  assert.equal(result.exitCode, 1);
  assert.equal(e.calls.slice(before).some(c => ['publish_create_campaign', 'publish_execute'].includes(c.command)), false);
});

test('phone-only scope replacement failure persists campaign receipt but never executes', async t => {
  const e = environment(t);
  Object.assign(e.preflight, { sheetConfigured: false, sheetEnabled: false, sheetDelivery: null });
  const prepared = await runAcceptance(e.phoneOnlyOptions('preflight'), { invoke: e.invoke });
  const rename = fs.renameSync;
  fs.renameSync = (from, to) => {
    if (to === e.devScope) throw new Error('scope replace failed');
    return rename(from, to);
  };
  let result;
  try {
    result = await runAcceptance(e.phoneOnlyOptions('submit', ['--confirm', prepared.report.confirmation]), { invoke: e.invoke });
  } finally { fs.renameSync = rename; }
  assert.equal(result.exitCode, 1);
  assert.equal(fs.existsSync(path.join(e.dir, 'campaign.json')), true);
  assert.equal(e.calls.some(c => c.command === 'publish_create_campaign'), true);
  assert.equal(e.calls.some(c => c.command === 'publish_execute'), false);
  assert.deepEqual(JSON.parse(fs.readFileSync(e.devScope, 'utf8')), e.initialDevScope);
});

test('phone-only preflight accepts only the exact inactive DevAcceptancePolicy schema', async t => {
  const e = environment(t);
  Object.assign(e.preflight, { sheetConfigured: false, sheetEnabled: false, sheetDelivery: null });
  for (const policy of [
    { campaignIds: ['other'], deviceIds: ['phone-b', 'phone-a'] },
    { campaignIds: [], deviceIds: ['phone-a', 'phone-b'] },
    { campaignIds: [], deviceIds: ['phone-b', 'phone-a'], capabilities: { publishVerification: true } },
    { campaignIds: [], deviceIds: ['phone-b', 'phone-a'], capabilities: {}, extra: true },
  ]) {
    fs.writeFileSync(e.devScope, JSON.stringify(policy) + '\n');
    const result = await runAcceptance(e.phoneOnlyOptions('preflight'), { invoke: e.invoke });
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
  assert.deepEqual(resumed.report.counts, { requested: 2, enqueued: 2, submitted: 2, verified: 0, sheetSent: 0, urlReadback: 0 });
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

test('Sheet sent remains distinct from supplementary URL readback, private readback is unsupported', async t => {
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
  const page = { url: () => 'http://tauri.localhost/', evaluate: async (fn, args) => {
    seen.push(args);
    return args ? { campaign: { id: args.args.campaignId } } : true;
  } };
  let disconnected = false;
  const chromium = { connectOverCDP: async () => ({ contexts: () => [{ pages: () => [page] }],
    close: async () => { disconnected = true; } }) };
  const ipc = await connectIPC({ cdp: 'http://127.0.0.1:9277' }, chromium);
  assert.deepEqual(await ipc.invoke('publish_get', { campaignId: 'fixture' }), { campaign: { id: 'fixture' } });
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
