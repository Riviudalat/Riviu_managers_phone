#!/usr/bin/env node

import { spawn, spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import {
  copyFileSync,
  mkdirSync,
  readFileSync,
  statSync,
} from "node:fs";
import { delimiter, dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

export const MOBILE_MCP_VERSION = "1.0.2";
export const MOBILE_MCP_INTEGRITY =
  "sha512-EVWH2dLOL69Ur1HZn7g5oPEhVnCbpyoZdKLKUAjQWh5JGn9i10OU6TrS+onInzUeRWFJY18M7Cnha4Q+LgxzKA==";

const SCRIPT_DIR = dirname(fileURLToPath(import.meta.url));
export const REPOSITORY_ROOT = resolve(SCRIPT_DIR, "..");

function executableName(platform = process.platform) {
  return platform === "win32" ? "adb.exe" : "adb";
}

function isFile(path) {
  try {
    return statSync(path).isFile();
  } catch {
    return false;
  }
}

export function assertSupportedNode(version = process.versions.node) {
  const major = Number.parseInt(version.split(".", 1)[0], 10);
  if (!Number.isInteger(major) || major < 20) {
    throw new Error(`mobile-mcp requires Node.js 20 or newer; found ${version}`);
  }
}

export function readPinnedPackage(repositoryRoot = REPOSITORY_ROOT) {
  const desktopRoot = join(repositoryRoot, "apps", "desktop");
  const manifest = JSON.parse(readFileSync(join(desktopRoot, "package.json"), "utf8"));
  const lock = JSON.parse(readFileSync(join(desktopRoot, "package-lock.json"), "utf8"));
  const installed = JSON.parse(
    readFileSync(
      join(desktopRoot, "node_modules", "@mobilenext", "mobile-mcp", "package.json"),
      "utf8",
    ),
  );
  const lockEntry = lock.packages?.["node_modules/@mobilenext/mobile-mcp"];

  if (manifest.devDependencies?.["@mobilenext/mobile-mcp"] !== MOBILE_MCP_VERSION) {
    throw new Error(`package.json must pin @mobilenext/mobile-mcp to ${MOBILE_MCP_VERSION}`);
  }
  if (manifest.dependencies?.["@mobilenext/mobile-mcp"] !== undefined) {
    throw new Error("mobile-mcp must remain a development-only dependency");
  }
  if (lockEntry?.version !== MOBILE_MCP_VERSION || lockEntry?.integrity !== MOBILE_MCP_INTEGRITY) {
    throw new Error("package-lock.json does not contain the reviewed mobile-mcp artifact");
  }
  if (installed.version !== MOBILE_MCP_VERSION) {
    throw new Error(`installed mobile-mcp is ${installed.version}; expected ${MOBILE_MCP_VERSION}`);
  }

  const entrypoint = join(
    desktopRoot,
    "node_modules",
    "@mobilenext",
    "mobile-mcp",
    "lib",
    "index.js",
  );
  if (!isFile(entrypoint)) {
    throw new Error(`mobile-mcp entrypoint is missing: ${entrypoint}`);
  }
  return { desktopRoot, entrypoint, installed };
}

export function buildAdbCandidates({
  env = process.env,
  platform = process.platform,
  repositoryRoot = REPOSITORY_ROOT,
} = {}) {
  const name = executableName(platform);
  const candidates = [];
  const add = (path, origin) => {
    if (!path) return;
    const normalized = resolve(path);
    if (!candidates.some((candidate) => candidate.path.toLowerCase() === normalized.toLowerCase())) {
      candidates.push({ path: normalized, origin });
    }
  };

  add(env.RIVIU_ADB_PATH?.trim(), "RIVIU_ADB_PATH");
  for (const [key, origin] of [
    ["ANDROID_SDK_ROOT", "ANDROID_SDK_ROOT"],
    ["ANDROID_HOME", "ANDROID_HOME"],
  ]) {
    const root = env[key]?.trim();
    if (root) add(join(root, "platform-tools", name), origin);
  }
  for (const entry of (env.PATH ?? "").split(delimiter)) {
    if (entry.trim()) add(join(entry.trim(), name), "PATH");
  }
  if (platform === "win32") {
    add(join(repositoryRoot, "sidecars", "android", "win-x86_64", name), "bundled");
  }
  return candidates;
}

export function resolveAdb(options = {}) {
  for (const candidate of buildAdbCandidates(options)) {
    if (!isFile(candidate.path)) continue;
    const probe = spawnSync(candidate.path, ["version"], {
      encoding: "utf8",
      timeout: 10_000,
      windowsHide: true,
    });
    if (probe.status === 0) {
      return {
        ...candidate,
        version: `${probe.stdout ?? ""}${probe.stderr ?? ""}`.trim().split(/\r?\n/)[0],
      };
    }
  }
  return null;
}

function copyIfPresent(source, destination) {
  if (!isFile(source)) return;
  mkdirSync(dirname(destination), { recursive: true });
  if (isFile(destination) && sha256(destination) === sha256(source)) return;
  copyFileSync(source, destination);
}

function sha256(path) {
  return createHash("sha256").update(readFileSync(path)).digest("hex");
}

export function stageAdbShim(adb, repositoryRoot = REPOSITORY_ROOT) {
  const digest = sha256(adb.path);
  const sdkRoot = join(
    repositoryRoot,
    "target",
    "mobile-mcp",
    `android-sdk-${digest.slice(0, 16)}`,
  );
  const platformTools = join(sdkRoot, "platform-tools");
  const destination = join(platformTools, executableName());
  mkdirSync(platformTools, { recursive: true });
  copyIfPresent(adb.path, destination);
  if (process.platform === "win32") {
    for (const library of ["AdbWinApi.dll", "AdbWinUsbApi.dll"]) {
      copyIfPresent(join(dirname(adb.path), library), join(platformTools, library));
    }
  }
  return sdkRoot;
}

export function buildServerEnvironment(base, sdkRoot) {
  const env = { ...base };
  if (sdkRoot) {
    env.ANDROID_HOME = sdkRoot;
    env.ANDROID_SDK_ROOT = sdkRoot;
  }
  env.MOBILEMCP_DISABLE_TELEMETRY = "1";
  env.MOBILEWRIGHT_DISABLE_TELEMETRY = "1";
  env.MOBILEMCP_LEGACY_ROBOT = "1";
  delete env.MOBILEMCP_ALLOW_UNSAFE_URLS;
  return env;
}

export function assertStdioOnly(args) {
  if (args.some((arg) => arg === "--listen" || arg.startsWith("--listen="))) {
    throw new Error("Riviu's mobile-mcp launcher permits stdio only; network listeners are disabled");
  }
}

export function integrationStatus(repositoryRoot = REPOSITORY_ROOT) {
  assertSupportedNode();
  const pkg = readPinnedPackage(repositoryRoot);
  const adb = resolveAdb({ repositoryRoot });
  return {
    status: adb ? "ready" : "package_ready_adb_missing",
    mobileMcpVersion: pkg.installed.version,
    transport: "stdio",
    telemetry: "disabled",
    robot: "legacy",
    productionRuntime: "not_bundled",
    adb: adb
      ? { origin: adb.origin, path: adb.path, version: adb.version }
      : null,
  };
}

async function main() {
  const args = process.argv.slice(2);
  const check = args.includes("--check");
  const requireAdb = args.includes("--require-adb");
  const forwarded = args.filter((arg) => arg !== "--check" && arg !== "--require-adb");
  assertStdioOnly(forwarded);

  const status = integrationStatus();
  if (check) {
    console.log(JSON.stringify(status, null, 2));
    if (requireAdb && !status.adb) process.exitCode = 2;
    return;
  }
  if (requireAdb && !status.adb) {
    throw new Error("No usable adb was found; set RIVIU_ADB_PATH or install Android platform-tools");
  }

  const { entrypoint } = readPinnedPackage();
  const sdkRoot = status.adb ? stageAdbShim(status.adb) : null;
  const child = spawn(process.execPath, [entrypoint, "--stdio", ...forwarded], {
    env: buildServerEnvironment(process.env, sdkRoot),
    stdio: "inherit",
    windowsHide: true,
  });
  child.once("error", (error) => {
    console.error(error.message);
    process.exitCode = 1;
  });
  child.once("exit", (code, signal) => {
    process.exitCode = code ?? (signal ? 1 : 0);
  });
}

if (resolve(process.argv[1] ?? "") === fileURLToPath(import.meta.url)) {
  main().catch((error) => {
    console.error(error instanceof Error ? error.message : String(error));
    process.exitCode = 1;
  });
}
