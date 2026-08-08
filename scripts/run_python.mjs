#!/usr/bin/env node

import { spawnSync } from "node:child_process";

const arguments_ = process.argv.slice(2);
if (arguments_.length === 0) {
  console.error("usage: run_python.mjs <script> [arguments...]");
  process.exit(2);
}

// 3.12 first, because that is the version the README installs the requirements
// into and the one CI pins. `py -3` / bare `python3` mean "newest Python 3", so
// on a machine with a newer interpreter they select one with no dependencies.
const candidates = process.platform === "win32"
  ? [["py", ["-3.12"]], ["py", ["-3"]], ["python", []]]
  : [["python3.12", []], ["python3", []], ["python", []]];

// Probe before running: `py` exists on most Windows machines but exits non-zero
// when the requested version is absent, so dispatching the real command first
// would report that failure instead of moving on to the next candidate.
const tried = [];
for (const [program, prefix] of candidates) {
  const label = [program, ...prefix].join(" ");
  const probe = spawnSync(program, [...prefix, "-c", "pass"], { stdio: "ignore" });
  if (probe.error?.code === "ENOENT") {
    tried.push(`${label} (not installed)`);
    continue;
  }
  if (probe.error) {
    tried.push(`${label} (${probe.error.message})`);
    continue;
  }
  if (probe.status !== 0) {
    tried.push(`${label} (exit ${probe.status})`);
    continue;
  }

  const result = spawnSync(program, [...prefix, ...arguments_], { stdio: "inherit" });
  if (result.error) {
    console.error(`${label}: ${result.error.message}`);
    process.exit(1);
  }
  process.exit(result.status ?? 1);
}

console.error(
  `Python 3 was not found; install it before building from source. Tried: ${tried.join(", ")}`,
);
process.exit(1);
