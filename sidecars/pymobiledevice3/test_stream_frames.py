#!/usr/bin/env python3
"""Assert stream command emits at least one JPEG frame."""

import struct
import subprocess
import sys
from pathlib import Path

sidecar = Path(__file__).with_name("riviu_pmd.py")
# Discover first device
list_proc = subprocess.run(
    [sys.executable, str(sidecar), "list"],
    capture_output=True,
    text=True,
    check=False,
)
assert list_proc.returncode == 0, list_proc.stderr
import json

devices = json.loads(list_proc.stdout)["devices"]
assert devices, "no USB iPhone connected for stream test"
udid = devices[0]["udid"]

proc = subprocess.Popen(
    [
        sys.executable,
        str(sidecar),
        "stream",
        "--udid",
        udid,
        "--fps",
        "5",
        "--quality",
        "60",
        "--max-frames",
        "2",
    ],
    stdout=subprocess.PIPE,
    stderr=subprocess.PIPE,
)
assert proc.stdout is not None
for i in range(2):
    header = proc.stdout.read(4)
    assert len(header) == 4, proc.stderr.read().decode()
    (length,) = struct.unpack(">I", header)
    assert 1000 < length < 2_000_000, length
    frame = proc.stdout.read(length)
    assert len(frame) == length
    assert frame[:2] == b"\xff\xd8", "expected JPEG SOI"
    print(f"frame{i} jpeg_bytes={length}")
proc.wait(timeout=30)
assert proc.returncode == 0, proc.stderr.read().decode()
print(f"ok udid={udid}")
