#!/usr/bin/env python3
"""Live integration test for USB iPhone discovery."""

import json
import subprocess
import sys
from pathlib import Path


sidecar = Path(__file__).with_name("riviu_pmd.py")
result = subprocess.run(
    [sys.executable, str(sidecar), "list"],
    capture_output=True,
    text=True,
    check=False,
)

assert result.returncode == 0, result.stderr or result.stdout
payload = json.loads(result.stdout)
assert payload["devices"], f"expected a trusted USB iPhone, got: {payload}"
assert payload["devices"][0]["udid"], payload
print(json.dumps(payload, indent=2))
