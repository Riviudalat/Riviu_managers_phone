// Shared CDP/IPC helper for the device-acceptance kit, and a one-call CLI.
//
// Every script here talks to ONE acceptance controller (launched by launch.py) over the
// WebView's CDP port and calls window.__TAURI_INTERNALS__.invoke, the same path the UI uses, so
// admission and leases still apply. Promoted from target/acceptance-20261010/kit/ipc.p47182.mjs
// and call.p47182.mjs; the port and output root are now parameters.
//
// Safety properties (keep them):
//   * an IPC error exposes only {command, code, message}; arguments and rejected objects are
//     never serialized, because they may carry Sheet passwords or cookies;
//   * intent files are written exclusively (`writeJson(file, value, true)` refuses to overwrite);
//   * the CLI refuses known effect commands unless --allow-effect is given: the runners own them.
//
// CLI: node ipc.mjs [--port N] [--allow-effect] <command> ['<json args>']
//      prints {"ok":true,"value":...} or {"ok":false,"error":{command,code,message}}; exit 1 on error.
import fs from 'node:fs';
import path from 'node:path';
import {execFileSync} from 'node:child_process';
import {fileURLToPath, pathToFileURL} from 'node:url';

export const KIT = path.dirname(fileURLToPath(import.meta.url));
export const REPO = path.resolve(KIT, '..', '..');

/** The main checkout: worktrees share its target/ (see scripts/dev_cache_common.ps1). */
function mainCheckout() {
  try {
    const common = execFileSync('git', ['-C', REPO, 'rev-parse', '--path-format=absolute', '--git-common-dir'],
      {encoding: 'utf8', stdio: ['ignore', 'pipe', 'ignore']}).trim().split(/\r?\n/).pop();
    if (path.basename(common) === '.git') return path.dirname(common);
  } catch { /* not a git checkout: fall back to this tree */ }
  return REPO;
}
export const MAIN = mainCheckout();

export const sleep = ms => new Promise(resolve => setTimeout(resolve, ms));
export const now = () => new Date().toISOString();
export const readJson = file => {
  try { return JSON.parse(fs.readFileSync(file, 'utf8').replace(/^﻿/, '')); }
  catch (error) { if (error.code === 'ENOENT') return null; throw error; }
};
/** Atomic write; `exclusive` refuses to overwrite (intent files). */
export function writeJson(file, value, exclusive = false) {
  fs.mkdirSync(path.dirname(file), {recursive: true});
  if (exclusive) { fs.writeFileSync(file, JSON.stringify(value, null, 2) + '\n', {flag: 'wx'}); return; }
  const temp = `${file}.${process.pid}.tmp`;
  fs.writeFileSync(temp, JSON.stringify(value, null, 2) + '\n');
  fs.renameSync(temp, file);
}

/** Git-ignored machine config (local.config.json); see local.config.example.json. */
export const CONFIG = readJson(path.join(KIT, 'local.config.json')) ?? {};
const resolveFrom = (base, p) => (p ? path.resolve(base, p) : null);
/** Outputs stay under the main checkout's target/ unless the local config says otherwise. */
export const OUT_ROOT = resolveFrom(MAIN, CONFIG.outRoot) ?? path.join(MAIN, 'target', 'device-acceptance');
export const DEFAULT_PORT = Number(process.env.RIVIU_CDP_PORT ?? CONFIG.cdpPort ?? 47182);
/** Resolve a path argument: relative to the current directory, like any CLI. */
export const resolvePath = p => (p ? path.resolve(p) : null);
/** "Riviu 31" style display name from the local number map, else the serial. */
export const numberOf = udid => CONFIG.numbers?.[udid] ?? udid;

export function checkPort(port) {
  const n = Number(port);
  if (!Number.isInteger(n) || n < 1024 || n > 65535) throw new Error(`cổng CDP không hợp lệ: ${port}`);
  return n;
}

export const check = (ok, message) => { if (!ok) throw new Error(message); };

async function loadChromium() {
  // A worktree may not have run `npm ci`; the main checkout's install is the same lockfile.
  for (const root of [REPO, MAIN]) {
    const entry = path.join(root, 'apps', 'desktop', 'node_modules', 'playwright', 'index.mjs');
    if (fs.existsSync(entry)) return (await import(pathToFileURL(entry).href)).chromium;
  }
  throw new Error('không tìm thấy playwright trong apps/desktop/node_modules; chạy npm ci --prefix apps/desktop');
}

/** Connect to the single WebView page of the controller on 127.0.0.1:<port>. */
export async function connect(port = DEFAULT_PORT) {
  const chromium = await loadChromium();
  const browser = await chromium.connectOverCDP(`http://127.0.0.1:${checkPort(port)}`, {timeout: 15000});
  const pages = browser.contexts().flatMap(c => c.pages());
  if (pages.length !== 1) { await browser.close(); throw new Error(`cần đúng một trang WebView, thấy ${pages.length}`); }
  const page = pages[0];
  const invoke = (command, args = {}) => page.evaluate(async ({command, args}) => {
    try { return {ok: true, value: await window.__TAURI_INTERNALS__.invoke(command, args)}; }
    catch (e) {
      const known = e && typeof e.code === 'string' && typeof e.message === 'string';
      return {ok: false, error: {command, code: known ? e.code : 'ipc_rejected', message: known ? e.message : String(e?.message ?? e)}};
    }
  }, {command, args}).then(r => { if (!r.ok) throw new Error(JSON.stringify(r.error)); return r.value; });
  return {invoke, page, browser, close: () => browser.close()};
}

/** Reconnect per call: survives a WebView reload during long watches. */
export async function invokeOnce(command, args = {}, port = DEFAULT_PORT) {
  const c = await connect(port);
  try { return await c.invoke(command, args); } finally { await c.close(); }
}

/**
 * Commands with a phone, network or Sheet effect. The runners in this folder own them, each
 * behind its intent file; a hand call needs --allow-effect so it is never a reflex.
 */
export const EFFECT_COMMANDS = new Set([
  'publish_start', 'publish_retry_assignment', 'publish_cancel', 'interaction_start_thread', 'interaction_retry',
  'interaction_cancel', 'nurture_start', 'nurture_stop', 'nurture_save_settings', 'agent_helper_maintenance_execute',
  'device_baseline_apply', 'device_load_cleanup', 'plugin:window|close',
]);

/** Minimal flag parser: `--name value`, `--flag`, positionals. Unknown flags are refused. */
export function parseArgs(argv, {flags = [], options = []} = {}) {
  const out = {_: []};
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    if (!a.startsWith('--')) { out._.push(a); continue; }
    const name = a.slice(2);
    if (flags.includes(name)) out[name] = true;
    else if (options.includes(name)) {
      if (i + 1 >= argv.length) throw new Error(`thiếu giá trị cho --${name}`);
      out[name] = argv[++i];
    } else throw new Error(`cờ không hỗ trợ: ${a}`);
  }
  return out;
}

/** Print usage and exit 0 when --help/-h is present. */
export function helpAndExit(argv, usage) {
  if (argv.includes('--help') || argv.includes('-h')) { console.log(usage.trim()); process.exit(0); }
}

const USAGE = `
Gọi một lệnh IPC qua controller nghiệm thu (CDP 127.0.0.1:<port>).
  node scripts/device-acceptance/ipc.mjs [--port 47182] [--allow-effect] <command> ['<json args>']
Ví dụ: node scripts/device-acceptance/ipc.mjs list_devices
Lệnh có effect (publish_start, nurture_start, ...) bị từ chối nếu thiếu --allow-effect;
hãy dùng runner tương ứng, vì runner ghi intent trước effect.`;

if (process.argv[1] && import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href) {
  const argv = process.argv.slice(2);
  helpAndExit(argv, USAGE);
  const args = parseArgs(argv, {flags: ['allow-effect'], options: ['port']});
  const [command, raw = '{}'] = args._;
  check(command, 'thiếu tên lệnh; xem --help');
  if (EFFECT_COMMANDS.has(command) && !args['allow-effect'])
    throw new Error(`${command} có effect; dùng runner tương ứng hoặc thêm --allow-effect có chủ đích`);
  const port = args.port ?? DEFAULT_PORT;
  let result;
  try { result = {ok: true, value: await invokeOnce(command, JSON.parse(raw), port)}; }
  catch (error) {
    let parsed = null;
    try { parsed = JSON.parse(error.message); } catch { /* not an IPC error */ }
    result = {ok: false, error: parsed?.command ? parsed : {command, code: 'cdp_error', message: String(error.message ?? error)}};
  }
  console.log(JSON.stringify(result));
  if (!result.ok) process.exitCode = 1;
}
