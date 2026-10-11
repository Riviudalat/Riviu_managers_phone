// Read-only native UI capture over CDP: page screenshots per size, command palette and a tile's
// context menu opened then closed with Escape, and the canvas count (live tiles) over 8 s.
// Clicks only navigation and menu launchers; no device actions.
// Promoted from target/acceptance-20261010/kit/native-ui-shots.mjs.
import fs from 'node:fs';
import path from 'node:path';
import {OUT_ROOT, DEFAULT_PORT, connect, parseArgs, helpAndExit, resolvePath} from './ipc.mjs';

const USAGE = `
Chụp UI thật của controller qua CDP (chỉ đọc).
  SIZES=1280x720,1366x768 node scripts/device-acceptance/native-ui-shots.mjs [outdir] [--port 47182]
SIZES mặc định 820x560,1366x768,1920x1080. outdir mặc định target/device-acceptance/native-ui.`;

const argv = process.argv.slice(2);
helpAndExit(argv, USAGE);
const args = parseArgs(argv, {options: ['port']});
const OUT = resolvePath(args._[0]) ?? path.join(OUT_ROOT, 'native-ui');
fs.mkdirSync(OUT, {recursive: true});
const sizes = (process.env.SIZES || '820x560,1366x768,1920x1080').split(',').map(s => s.split('x').map(Number));
if (sizes.some(([w, h]) => !(w >= 320 && h >= 240))) throw new Error('SIZES phải dạng WxH,WxH');
const c = await connect(args.port ?? DEFAULT_PORT);
const page = c.page;
const shot = async name => { await page.waitForTimeout(2500); await page.screenshot({path: path.join(OUT, `${name}.png`)}); console.log('shot', name); };
const tileCanvases = () => page.evaluate(() => [...document.querySelectorAll('canvas')].length);
try {
  for (const [w, h] of sizes) {
    await page.setViewportSize({width: w, height: h});
    const nav = page.getByRole('link', {name: /Thiết bị|Control Center|Bảng điều khiển/}).first();
    if (await nav.count()) await nav.click().catch(() => {});
    await shot(`devices-${w}`);
    console.log(`canvases@${w}`, await tileCanvases());
  }
  await page.setViewportSize({width: 1366, height: 768});
  await page.keyboard.press('Control+K'); await shot('palette-1366');
  await page.keyboard.press('Escape');
  const tile = page.locator('[tabindex][data-udid]').first();
  if (await tile.count()) { await tile.click({button: 'right'}); await shot('menu-1366'); await page.keyboard.press('Escape'); }
  const before = await tileCanvases(); await page.waitForTimeout(8000); console.log('canvases stable', before, await tileCanvases());
} finally {
  // setViewportSize(null) is rejected by Playwright and left the app stuck at the last
  // emulated size inside the real window; clear the CDP override explicitly.
  await page.context().newCDPSession(page).then((s) => s.send('Emulation.clearDeviceMetricsOverride')).catch(() => {});
  await c.close();
}
