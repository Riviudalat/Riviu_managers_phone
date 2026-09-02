#!/usr/bin/env python3
"""Reject vendor artifacts, endpoints and excluded command surfaces from shipping code."""

from __future__ import annotations

import argparse
import json
from pathlib import Path


BLOCKED_RUNTIME = {
    b"api.xiaowei.xin": "vendor endpoint",
    b"xiaowei.run": "vendor endpoint",
    b"xiaowei-manage-backend": "vendor endpoint",
    b"com.xiaowei.assistant": "vendor package",
    b"com.xiaowei.aoamanager": "vendor package",
    b"com.android.xwkeyboard": "vendor package",
    b"cloud_phone_": "commercial command family",
    b"cloud_virtual_": "commercial command family",
    b"exec_autojs": "arbitrary-script command",
    b"stop_autojs": "arbitrary-script command",
    b"merge_adb_auth_key": "ADB key import command",
    b"install_magisk": "privilege modification command",
    b"install_xwdb": "privilege modification command",
    b"open_api_ws_connect": "vendor OpenAPI transport",
}

BLOCKED_NAMES = {
    "xiaowei.exe",
    "assistant.apk",
    "hidmanager.apk",
    "xwkeyboard.apk",
    "bridge.exe",
    "xwdb",
}

TEXT_EXTENSIONS = {
    ".css", ".html", ".js", ".json", ".map", ".mjs", ".rs", ".ts", ".tsx", ".vue"
}


def files_under(path: Path):
    if path.is_file():
        yield path
    elif path.is_dir():
        yield from (candidate for candidate in path.rglob("*") if candidate.is_file())


def inspect(paths: list[tuple[str, Path]]) -> list[dict[str, str]]:
    findings: list[dict[str, str]] = []
    for surface, root in paths:
        for path in files_under(root):
            lowered_name = path.name.lower()
            if lowered_name in BLOCKED_NAMES:
                findings.append({"surface": surface, "path": str(path), "reason": "vendor artifact name"})
            try:
                payload = path.read_bytes().lower()
            except OSError as error:
                findings.append({"surface": surface, "path": str(path), "reason": f"unreadable: {error}"})
                continue
            for needle, reason in BLOCKED_RUNTIME.items():
                if needle in payload:
                    findings.append({"surface": surface, "path": str(path), "reason": reason})
            if surface == "frontend" and path.suffix.lower() in TEXT_EXTENSIONS and b"xiaowei" in payload:
                findings.append({"surface": surface, "path": str(path), "reason": "vendor branding in operator UI"})
    return findings


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--runtime", action="append", type=Path, default=[])
    parser.add_argument("--frontend", action="append", type=Path, default=[])
    parser.add_argument("--installer", action="append", type=Path, default=[])
    args = parser.parse_args()
    paths = (
        [("runtime", path) for path in args.runtime]
        + [("frontend", path) for path in args.frontend]
        + [("installer", path) for path in args.installer]
    )
    findings = inspect(paths)
    print(json.dumps({"ok": not findings, "filesets": len(paths), "findings": findings}, ensure_ascii=False))
    return 1 if findings else 0


if __name__ == "__main__":
    raise SystemExit(main())
