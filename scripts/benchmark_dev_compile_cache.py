"""Measure cold/warm compiler-cache builds of the actual desktop application.

Run after tests/builds finish and the dev executable is closed. Only the core
entrypoint mtime is advanced to make Cargo dispatch the identical compiler input;
its bytes and original mtime are restored. No release profile or installer runs.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import socket
import subprocess
import time


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--sccache", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--features", default="")
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[1]
    cache = args.sccache.resolve(strict=True)
    output = args.output.resolve()
    output.parent.mkdir(parents=True, exist_ok=True)
    source = root / "crates/core/src/lib.rs"
    before = source.stat()
    digest = hashlib.sha256(source.read_bytes()).hexdigest()
    with socket.socket() as listener:
        listener.bind(("127.0.0.1", 0))
        port = listener.getsockname()[1]
    env = dict(os.environ)
    env.update(RUSTC_WRAPPER=str(cache), SCCACHE_CACHE_SIZE="8G",
               SCCACHE_DIR=str(output.parent / "compiler-cache"),
               SCCACHE_SERVER_PORT=str(port))
    runs = []
    command = ["cargo", "build", "--locked", "-p", "riviu-managers-phone", "--timings"]
    if args.features:
        command.extend(["--features", args.features])
    try:
        for label in ["cold", "warm"]:
            os.utime(source, None)
            started = time.perf_counter()
            stdout = output.with_name(output.stem + f"-{label}.stdout.txt")
            stderr = output.with_name(output.stem + f"-{label}.stderr.txt")
            with stdout.open("wb") as out, stderr.open("wb") as err:
                result = subprocess.run(command, cwd=root, env=env, stdout=out, stderr=err)
            run = dict(label=label, command=command, exit=result.returncode,
                       seconds=time.perf_counter() - started, stdout=str(stdout), stderr=str(stderr))
            runs.append(run)
            stats = subprocess.run([str(cache), "--show-stats", "--stats-format", "json"],
                                   cwd=root, env=env, capture_output=True)
            stats_path = output.with_name(output.stem + f"-{label}.stats.json")
            stats_path.write_bytes(stats.stdout)
            run["statsExit"] = stats.returncode
            run["statsPath"] = str(stats_path)
            print(json.dumps(run), flush=True)
            if result.returncode or stats.returncode:
                raise RuntimeError(f"{label} build/cache stats failed")
            assert hashlib.sha256(source.read_bytes()).hexdigest() == digest
    finally:
        os.utime(source, ns=(before.st_atime_ns, before.st_mtime_ns))
        restored = hashlib.sha256(source.read_bytes()).hexdigest()
        report = dict(target=str(root), source=str(source), beforeHash=digest,
                      afterHash=restored, cacheLimit="8G", runs=runs,
                      comparison="Actual application cold/warm cache builds, including uncached links")
        output.write_text(json.dumps(report, indent=2), encoding="utf-8")
        subprocess.run([str(cache), "--stop-server"], cwd=root, env=env, capture_output=True)
    assert restored == digest


if __name__ == "__main__":
    main()
