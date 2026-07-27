#!/usr/bin/env python3
"""Sign/install Riviumanagersphone on-device agent.

Delegates to build_and_install.py (needs full Xcode + Apple Development cert).
Falls back to structured errors the desktop UI can show.
"""

from __future__ import annotations

import argparse
import json
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
    p.add_argument("--apple-id", required=True)
    p.add_argument("--password", required=True)
    p.add_argument("--wda", required=False, default=str(DEFAULT_IPA))
    p.add_argument("--team-id", required=False, default=None)
    args = parser.parse_args()

    # Password is stored for future anisette free-sign; Xcode Personal Team
    # currently owns the certificate after user adds the same Apple ID in Xcode.
    _ = args.password
    _ = args.apple_id
    _ = args.wda

    cmd = [sys.executable, str(BUILD_INSTALL), "--udid", args.udid]
    if args.team_id:
        cmd.extend(["--team-id", args.team_id])

    result = subprocess.run(cmd, capture_output=True, text=True)
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
        print(json.dumps(payload, ensure_ascii=False))
        return 1

    expires = datetime.now(timezone.utc) + timedelta(days=7)
    payload["expiresAt"] = expires.isoformat().replace("+00:00", "Z")
    payload.setdefault(
        "message",
        "Signed and installed Riviumanagersphone agent (orange R) on device.",
    )
    print(json.dumps(payload, ensure_ascii=False))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
