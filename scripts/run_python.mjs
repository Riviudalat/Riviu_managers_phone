#!/usr/bin/env node

import { spawnSync } from "node:child_process";

const arguments_ = process.argv.slice(2);
if (arguments_.length === 0) {
  console.error("usage: run_python.mjs <script> [arguments...]");
  process.exit(2);
}

const candidates = process.platform === "win32"
  ? [["py", ["-3"]], ["python", []]]
  : [["python3", []], ["python", []]];

for (const [program, prefix] of candidates) {
  const result = spawnSync(program, [...prefix, ...arguments_], {
    stdio: "inherit",
  });
  if (result.error?.code === "ENOENT") continue;
  if (result.error) {
    console.error(`${program}: ${result.error.message}`);
    process.exit(1);
  }
  process.exit(result.status ?? 1);
}

console.error("Python 3 was not found; install it before building from source.");
process.exit(1);
