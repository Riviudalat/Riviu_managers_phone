import assert from "node:assert/strict";
import { existsSync, mkdtempSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { delimiter, join } from "node:path";
import test from "node:test";

import {
  MOBILE_MCP_INTEGRITY,
  MOBILE_MCP_VERSION,
  REPOSITORY_ROOT,
  assertSupportedNode,
  assertStdioOnly,
  buildAdbCandidates,
  buildServerEnvironment,
  readPinnedPackage,
  stageAdbShim,
} from "./run_mobile_mcp.mjs";

test("the reviewed mobile-mcp artifact is pinned as a dev-only dependency", () => {
  const pinned = readPinnedPackage(REPOSITORY_ROOT);
  assert.equal(pinned.installed.version, MOBILE_MCP_VERSION);
  assert.match(MOBILE_MCP_INTEGRITY, /^sha512-/);
});

test("the launcher rejects unsupported Node versions", () => {
  assert.throws(() => assertSupportedNode("19.9.0"), /Node\.js 20 or newer/);
  assert.doesNotThrow(() => assertSupportedNode("20.0.0"));
});

test("adb discovery follows Riviu's development precedence", () => {
  const root = mkdtempSync(join(tmpdir(), "riviu-mobile-mcp-"));
  const pathA = join(root, "path-a");
  const pathB = join(root, "path-b");
  mkdirSync(pathA);
  mkdirSync(pathB);
  writeFileSync(join(pathA, "adb.exe"), "fixture");
  const candidates = buildAdbCandidates({
    env: {
      RIVIU_ADB_PATH: join(root, "direct", "adb.exe"),
      ANDROID_SDK_ROOT: join(root, "sdk"),
      ANDROID_HOME: join(root, "home"),
      PATH: `${pathA}${delimiter}${pathB}`,
    },
    platform: "win32",
    repositoryRoot: root,
  });
  assert.deepEqual(
    candidates.slice(0, 5).map(({ origin }) => origin),
    ["RIVIU_ADB_PATH", "ANDROID_SDK_ROOT", "ANDROID_HOME", "PATH", "PATH"],
  );
  assert.equal(candidates.at(-1).origin, "bundled");
});

test("the child server is local, legacy and telemetry-free", () => {
  const env = buildServerEnvironment(
    { MOBILEMCP_ALLOW_UNSAFE_URLS: "1", UNRELATED: "kept" },
    "C:/fixture-sdk",
  );
  assert.equal(env.MOBILEMCP_DISABLE_TELEMETRY, "1");
  assert.equal(env.MOBILEWRIGHT_DISABLE_TELEMETRY, "1");
  assert.equal(env.MOBILEMCP_LEGACY_ROBOT, "1");
  assert.equal(env.MOBILEMCP_ALLOW_UNSAFE_URLS, undefined);
  assert.equal(env.ANDROID_HOME, "C:/fixture-sdk");
  assert.equal(env.UNRELATED, "kept");
  assert.throws(() => assertStdioOnly(["--listen", "3000"]), /stdio only/);
  assert.throws(() => assertStdioOnly(["--listen=3000"]), /stdio only/);
  assert.doesNotThrow(() => assertStdioOnly(["--stdio"]));
});

test("adb shims are immutable and content-addressed", () => {
  const root = mkdtempSync(join(tmpdir(), "riviu-mobile-mcp-shim-"));
  const source = join(root, "source-adb.exe");
  writeFileSync(source, "first");
  const first = stageAdbShim({ path: source }, root);
  const repeated = stageAdbShim({ path: source }, root);
  const adbName = process.platform === "win32" ? "adb.exe" : "adb";
  assert.equal(first, repeated);
  assert.equal(existsSync(join(first, "platform-tools", adbName)), true);

  writeFileSync(source, "second");
  const changed = stageAdbShim({ path: source }, root);
  assert.notEqual(first, changed);
});

test("Tauri does not redistribute the development MCP server", () => {
  const tauri = JSON.parse(
    readFileSync(
      join(REPOSITORY_ROOT, "apps", "desktop", "src-tauri", "tauri.conf.json"),
      "utf8",
    ),
  );
  assert.equal(JSON.stringify(tauri).includes("mobile-mcp"), false);
});
