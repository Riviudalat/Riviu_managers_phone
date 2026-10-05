import assert from "node:assert/strict";
import { mkdtempSync, mkdirSync, readFileSync, writeFileSync, copyFileSync, existsSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { resolve, join } from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";
import test from "node:test";

const repository = fileURLToPath(new URL("../", import.meta.url));

function fixture(t) {
  const root = mkdtempSync(join(tmpdir(), "riviu-frontend-proof-"));
  t.after(() => rmSync(root, { recursive: true, force: true }));
  const desktop = join(root, "apps/desktop");
  function put(name, value) {
    const path = join(desktop, name);
    mkdirSync(resolve(path, ".."), { recursive: true });
    writeFileSync(path, value);
  }
  put("package.json", readFileSync(join(repository, "apps/desktop/package.json")));
  put("package-lock.json", "{}\n");
  put("src/App.tsx", "export const label = 'Original';\n");
  put("index.html", '<div id="root"></div>\n');
  put("vite.config.ts", "export default {};\n");
  put("tsconfig.json", "{}\n");
  const compiler = `const fs=require('node:fs');
const kind=process.argv[1].includes('typescript')?'tsc':'vite';
fs.appendFileSync('build-calls.txt',kind+'\\n');
if(fs.existsSync('fail-build')) process.exit(23);
if(kind==='vite') { fs.mkdirSync('dist/assets',{recursive:true}); fs.writeFileSync('dist/index.html','<div id="root"></div><script src="./assets/app.js"></script>'); fs.copyFileSync('src/App.tsx','dist/assets/app.js'); }
`;
  put("node_modules/typescript/bin/tsc", compiler);
  put("node_modules/vite/bin/vite.js", compiler);
  for (const [name, entry] of [["tsc", "typescript/bin/tsc"], ["vite", "vite/bin/vite.js"]]) {
    put(`node_modules/.bin/${name}.cmd`, `@echo off\r\n"${process.execPath}" "%~dp0../${entry}" %*\r\n`);
    put(`node_modules/.bin/${name}`, `#!/bin/sh\nexec "${process.execPath}" "$(dirname "$0")/../${entry}" "$@"\n`);
    if (process.platform !== "win32") spawnSync("chmod", ["+x", join(desktop, `node_modules/.bin/${name}`)]);
  }
  mkdirSync(join(root, "scripts"));
  const script = join(repository, "scripts/frontend_provenance.mjs");
  if (existsSync(script)) copyFileSync(script, join(root, "scripts/frontend_provenance.mjs"));
  const run = (command, args) => spawnSync(command, args, { cwd: desktop, encoding: "utf8" });
  const build = () => process.platform === "win32"
    ? run(process.env.ComSpec || "cmd.exe", ["/d", "/s", "/c", "npm run build"])
    : run("npm", ["run", "build"]);
  const verify = () => run(process.execPath, [join(root, "scripts/frontend_provenance.mjs"), "verify"]);
  return { desktop, put, build, verify };
}

test("successful frontend build proves its inputs; stale dist is rejected without a second build", (t) => {
  const f = fixture(t);
  const built = f.build();
  assert.equal(built.status, 0, built.stdout + built.stderr);
  assert.ok(existsSync(join(f.desktop, "dist/frontend-provenance.json")),
    "successful npm build must record frontend provenance before Cargo can reuse dist");
  assert.equal(f.verify().status, 0);
  // Equivalent Windows checkout line endings do not invalidate content provenance.
  f.put("src/App.tsx", "export const label = 'Original';\r\n");
  assert.equal(f.verify().status, 0);
  f.put("src/App.tsx", "export const label = 'Recovery button';\n");
  const stale = f.verify();
  assert.notEqual(stale.status, 0);
  assert.match(stale.stderr, /frontend.*stale|stale.*frontend/i);
  assert.equal(readFileSync(join(f.desktop, "build-calls.txt"), "utf8"), "tsc\nvite\n");
});

test("a failed rebuild cannot leave a valid old provenance receipt", (t) => {
  const f = fixture(t);
  assert.equal(f.build().status, 0);
  assert.ok(existsSync(join(f.desktop, "dist/frontend-provenance.json")),
    "successful npm build must record frontend provenance");
  f.put("fail-build", "fail");
  assert.notEqual(f.build().status, 0);
  assert.notEqual(f.verify().status, 0);
});
