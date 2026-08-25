#!/usr/bin/env python3
"""Sign/install Riviumanagersphone on-device agent.

Delegates to build_and_install.py (needs full Xcode + Apple Development cert).
Falls back to structured errors the desktop UI can show.
"""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
from datetime import datetime, timedelta, timezone
from pathlib import Path

WDA_DIR = Path(__file__).resolve().parents[1] / "wda"
BUILD_INSTALL = WDA_DIR / "build_and_install.py"
DEFAULT_IPA = WDA_DIR / "Riviumanagersphone.ipa"


def main() -> int:
    parser = argparse.ArgumentParser()
    sub = parser.add_subparsers(dest="cmd", required=True)
    p = sub.add_parser("sign-install-wda")
    p.add_argument("--udid", required=True)
    p.add_argument("--wda", required=False, default=str(DEFAULT_IPA))
    p.add_argument("--team-id", required=False, default=None)
    args = parser.parse_args()

    # Apple ID and its app-specific password arrive in the **environment**, never in argv.
    # On Windows a process command line is readable by any process running as the same user
    # (and is routinely captured by EDR/Sysmon), so `--password <secret>` handed the value to
    # every other process on the box — defeating the OS credential store the desktop went to
    # the trouble of using. Same reason, same fix as `ios-driver/src/supervisor.rs`, which
    # passes its fingerprint by environment with that reasoning written next to it.
    #
    # Both are read but unused: the certificate is currently owned by the Xcode Personal Team
    # after the operator adds the same Apple ID in Xcode. They are kept for the anisette
    # free-sign flow that would need them.
    _ = os.environ.get("RIVIU_APPLE_ID", "")
    _ = os.environ.get("RIVIU_APPLE_PASSWORD", "")
    _ = args.wda

    embedded_runtime = os.environ.get("RIVIU_EMBEDDED_PYTHON_RUNTIME")
    if embedded_runtime:
        cmd = [
            embedded_runtime,
            "__script",
            str(BUILD_INSTALL),
            "--udid",
            args.udid,
        ]
    else:
        cmd = [sys.executable, str(BUILD_INSTALL), "--udid", args.udid]
    if args.team_id:
        cmd.extend(["--team-id", args.team_id])

    child_environment = os.environ.copy()
    # `build_and_install.py` drives Xcode and never needs the Apple credentials, so it does not
    # get them. Repo precedent: the WDA build gate hands its subprocess a copy of the
    # environment with `RIVIU_AGENT_TOKEN` removed for exactly this reason — a secret should
    # reach one process, not every descendant of it.
    child_environment.pop("RIVIU_APPLE_PASSWORD", None)
    child_environment.pop("RIVIU_APPLE_ID", None)
    child_environment["PYTHONUTF8"] = "1"
    child_environment["PYTHONIOENCODING"] = "utf-8"
    options = {
        "capture_output": True,
        "text": True,
        "encoding": "utf-8",
        "errors": "replace",
        "env": child_environment,
    }
    if sys.platform == "win32":
        options["creationflags"] = 0x08000000
    result = subprocess.run(cmd, **options)
    payload = None
    for line in reversed((result.stdout or "").strip().splitlines()):
        line = line.strip()
        if line.startswith("{") and line.endswith("}"):
            try:
                payload = json.loads(line)
                break
            except json.JSONDecodeError:
                continue

    if payload is None:
        payload = {
            "ok": False,
            "error": (result.stderr or result.stdout or "build_and_install failed").strip(),
        }

    if not payload.get("ok"):
        print(json.dumps(payload, ensure_ascii=True))
        return 1

    expires = datetime.now(timezone.utc) + timedelta(days=7)
    payload["expiresAt"] = expires.isoformat().replace("+00:00", "Z")
    payload.setdefault(
        "message",
        "Signed and installed Riviumanagersphone agent (orange R) on device.",
    )
    print(json.dumps(payload, ensure_ascii=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
