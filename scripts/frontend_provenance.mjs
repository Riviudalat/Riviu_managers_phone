// One frontend build, followed by a content receipt; Cargo only verifies it.
import { createHash } from "node:crypto";
import { existsSync, lstatSync, readFileSync, readdirSync, renameSync, rmSync, writeFileSync } from "node:fs";
import { resolve, relative, join } from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";

const desktop = process.cwd();
const script = fileURLToPath(import.meta.url);
const receiptName = "frontend-provenance.json";
const configuration = ["index.html", "package.json", "package-lock.json", "vite.config.ts",
  "tsconfig.json", "tsconfig.app.json", "tsconfig.node.json",
  ".env", ".env.local", ".env.production", ".env.production.local"];
const hash = (value) => createHash("sha256").update(value).digest("hex");
const text = (bytes) => bytes.toString("utf8").replace(/\r\n/g, "\n");

function files(path) {
  if (!existsSync(path)) return [];
  const stat = lstatSync(path);
  if (stat.isSymbolicLink()) throw new Error(`frontend provenance cannot follow symlink: ${path}`);
  return stat.isDirectory()
    ? readdirSync(path).sort().flatMap((name) => files(join(path, name))) : [path];
}

function inputs() {
  const paths = [...files(join(desktop, "src")), ...files(join(desktop, "public")),
    ...configuration.map((name) => join(desktop, name)).filter(existsSync), script];
  const entries = paths.map((path) => {
    const bytes = readFileSync(path);
    const normalized = /\.(?:tsx?|jsx?|mjs|cjs|css|html|json|svg|md|txt)$/.test(path)
      || relative(desktop, path).startsWith(".env") ? text(bytes) : bytes;
    return [relative(desktop, path).replaceAll("\\", "/"), hash(normalized)];
  }).sort(([a], [b]) => a < b ? -1 : a > b ? 1 : 0);
  const environment = Object.fromEntries(Object.keys(process.env).filter((name) => name.startsWith("VITE_"))
    .sort().map((name) => [name, hash(process.env[name]) ]));
  environment.NODE_ENV = hash(process.env.NODE_ENV || "production");
  return { entries, environment };
}

function outputs(dist) {
  if (!existsSync(join(dist, "index.html"))) throw new Error("frontend dist has no index.html");
  return files(dist).filter((path) => relative(dist, path) !== receiptName)
    .map((path) => [relative(dist, path).replaceAll("\\", "/"), hash(readFileSync(path))]);
}

function configuredDist() {
  const tauri = join(desktop, "src-tauri");
  let build = JSON.parse(readFileSync(join(tauri, "tauri.conf.json"), "utf8")).build || {};
  const platform = ({ windows: "windows", macos: "macos", linux: "linux", android: "android", ios: "ios" })[process.env.CARGO_CFG_TARGET_OS];
  if (platform && existsSync(join(tauri, `tauri.${platform}.conf.json`))) {
    build = { ...build, ...JSON.parse(readFileSync(join(tauri, `tauri.${platform}.conf.json`), "utf8")).build };
  }
  if (process.env.TAURI_CONFIG) build = { ...build, ...JSON.parse(process.env.TAURI_CONFIG).build };
  const dist = build.frontendDist;
  if (typeof dist !== "string") throw new Error("frontend provenance requires a directory frontendDist");
  if (/^[a-z]:[\\/]/i.test(dist)) throw new Error("Use a relative frontendDist; absolute Windows paths can be interpreted as URLs");
  if (/^https?:\/\//.test(dist)) return null; // Remote URL builds do not embed dist.
  if (/^[a-z][a-z\d+.-]*:/i.test(dist)) throw new Error("unsupported frontendDist URL");
  return resolve(tauri, dist);
}

function verify(dist, cargo) {
  const saved = JSON.parse(readFileSync(join(dist, receiptName), "utf8"));
  const current = inputs();
  if (cargo) {
    for (const path of [script, join(desktop, "src"), join(desktop, "public"), dist,
      ...configuration.map((name) => join(desktop, name))]) console.log(`cargo:rerun-if-changed=${path}`);
    for (const name of new Set([...Object.keys(saved.inputs?.environment || {}), ...Object.keys(current.environment)])) {
      console.log(`cargo:rerun-if-env-changed=${name}`);
    }
  }
  if (saved.schemaVersion !== 1 || JSON.stringify(saved.inputs) !== JSON.stringify(current)
    || JSON.stringify(saved.outputs) !== JSON.stringify(outputs(dist))) {
    throw new Error("frontend provenance is stale");
  }
}

function build() {
  const dist = join(desktop, "dist");
  const receipt = join(dist, receiptName);
  rmSync(receipt, { force: true });
  const before = inputs();
  for (const [entry, args] of [["typescript/bin/tsc", ["-b"]], ["vite/bin/vite.js", ["build"]]]) {
    const result = spawnSync(process.execPath, [join(desktop, "node_modules", entry), ...args], { stdio: "inherit" });
    if (result.error) throw result.error;
    if (result.status !== 0) throw new Error(`frontend ${entry} failed (exit ${result.status})`);
  }
  if (JSON.stringify(before) !== JSON.stringify(inputs())) throw new Error("frontend inputs changed during build");
  const content = { schemaVersion: 1, inputs: before, outputs: outputs(dist) };
  const temporary = `${receipt}.${process.pid}.tmp`;
  try {
    writeFileSync(temporary, JSON.stringify(content, null, 2) + "\n", { flag: "wx" });
    renameSync(temporary, receipt);
  } finally { rmSync(temporary, { force: true }); }
}

try {
  const mode = process.argv[2];
  if (mode === "build") build();
  else if (mode === "verify") verify(join(desktop, "dist"), false);
  else if (mode === "verify-cargo") {
    const dist = configuredDist();
    if (dist) verify(dist, true);
  } else throw new Error("expected build, verify, or verify-cargo");
} catch (error) {
  console.error(`Frontend provenance check failed: ${error.message}. Run npm run build in apps/desktop before embedding frontend assets.`);
  process.exitCode = 1;
}
