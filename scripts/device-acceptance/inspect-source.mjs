// Read-only check of a publish source before preflight. Promoted from
// target/acceptance-20261010/kit/inspect-source.mjs.
// IPC: list_devices, list_device_metas, google_sheets_status, publish_scan_folder(--source of the
// template). Writes <report-dir>-inspect/*.json and exits 1 unless the scanned bundle ids equal
// the template's mapping. run-publish.py refuses to submit without this summary.
import path from 'node:path';
import {DEFAULT_PORT, connect, writeJson, readJson, parseArgs, helpAndExit, resolvePath, check} from './ipc.mjs';

const USAGE = `
Kiểm nguồn đăng bài (chỉ đọc) và so bundle id với template.
  node scripts/device-acceptance/inspect-source.mjs --template <publish-template.json> [--port 47182]
Ghi <report-dir>-inspect/{roster,metas,sheet,scan,summary}.json. Mẫu: examples/publish-template.example.json.`;

const argv = process.argv.slice(2);
helpAndExit(argv, USAGE);
const args = parseArgs(argv, {options: ['template', 'port']});
const template = readJson(resolvePath(args.template ?? ''));
check(template?.mapping?.length, 'thiếu --template có mapping');
const flag = name => template.common[template.common.indexOf(name) + 1];
const sourceRoot = flag('--source');
const OUT = `${flag('--report-dir')}-inspect`;
check(sourceRoot && flag('--report-dir'), 'template thiếu --source/--report-dir');
const c = await connect(args.port ?? DEFAULT_PORT);
const call = async (command, a = {}) => {
  try { return {ok: true, value: await c.invoke(command, a)}; }
  catch (error) { return {ok: false, error: String(error.message ?? error)}; }
};
try {
  const roster = await call('list_devices');
  const metas = await call('list_device_metas');
  const sheet = await call('google_sheets_status');
  const scan = await call('publish_scan_folder', {sourceRoot});
  // Sheet status is reduced to non-secret fields before it touches disk.
  const sheetSafe = {ok: sheet.ok, error: sheet.error, connected: sheet.value?.connected, active: sheet.value?.active, sheetUrl: sheet.value?.sheetUrl};
  for (const [name, value] of Object.entries({roster, metas, sheet: sheetSafe, scan})) writeJson(path.join(OUT, `${name}.json`), value);
  const scanned = (scan.value?.bundles ?? []).map(b => ({name: b.name, id: b.id, images: b.images?.length, kind: b.mediaKind}));
  const expected = template.mapping.map(m => m.bundleId);
  const match = JSON.stringify(scanned.map(b => b.id).sort()) === JSON.stringify([...expected].sort());
  const connected = new Set((roster.value ?? []).filter(d => ['connected', 'ready'].includes(d.status)).map(d => d.udid));
  const summary = {
    scan: {ok: scan.ok, error: scan.error, notices: scan.value?.notices?.length, bundles: scanned},
    bundleIdsMatchTemplate: match,
    publishPhones: template.mapping.map(m => ({number: m.number, udid: m.udid, connected: connected.has(m.udid),
      metaHandle: (metas.value ?? []).find(x => x.udid === m.udid)?.handle ?? null})),
    sheet: {ok: sheet.ok, connected: sheet.value?.connected, active: sheet.value?.active},
  };
  writeJson(path.join(OUT, 'summary.json'), summary);
  console.log(JSON.stringify(summary));
  if (!scan.ok || !match) process.exitCode = 1;
} finally { await c.close(); }
