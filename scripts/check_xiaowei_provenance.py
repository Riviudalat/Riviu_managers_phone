#!/usr/bin/env python3
"""Reject vendor artifacts, endpoints and excluded command surfaces from shipping code."""

from __future__ import annotations

import argparse
import json
from pathlib import Path

try:
    from scripts import build_xiaowei_parity_matrix as parity_matrix
except ImportError:
    import build_xiaowei_parity_matrix as parity_matrix


BLOCKED_RUNTIME = {
    b"api.xiaowei.xin": "vendor endpoint",
    b"xiaowei.run": "vendor endpoint",
    b"xiaowei-manage-backend": "vendor endpoint",
    b"com.xiaowei.assistant": "vendor package",
    b"com.xiaowei.aoamanager": "vendor package",
    b"com.android.xwkeyboard": "vendor package",
    b"cloud_phone_": "commercial command family",
    b"cloud_virtual_": "commercial command family",
    b"client_event_reporting": "commercial telemetry command",
    b"client_login": "commercial account command",
    b"client_logout": "commercial account command",
    b"client_pwd_reset": "commercial account command",
    b"client_refund": "commercial payment command",
    b"client_register": "commercial account command",
    b"get_activation_pay_list": "commercial activation command",
    b"get_brand_cloud_service": "commercial service command",
    b"get_brand_price_list": "commercial payment command",
    b"get_pay_result": "commercial payment command",
    b"pay_receipt": "commercial payment command",
    b"start_updater": "vendor updater command",
    b"sync_client_stat": "commercial telemetry command",
    b"voucher_": "commercial voucher command family",
    b"exec_autojs": "arbitrary-script command",
    b"stop_autojs": "arbitrary-script command",
    b"check_hid_app_installed": "vendor HID command",
    b"install_hid_app": "vendor HID command",
    b"remove_hid_driver": "vendor HID command",
    b"switch_accessible_mode": "vendor Accessibility transport command",
    b"switch_hid_model": "vendor HID command",
    b"usb_to_tcp": "vendor USB transport command",
    b"merge_adb_auth_key": "ADB key import command",
    b":32991": "vendor bridge port",
    b"install_magisk": "privilege modification command",
    b"install_xwdb": "privilege modification command",
    b"open_api_ws_connect": "vendor OpenAPI transport",
}

BLOCKED_NAMES = {
    "xiaowei.exe",
    "assistant.apk",
    "aoamanager.apk",
    "hidmanager.apk",
    "xwkeyboard.apk",
    "bridge.exe",
    "adbkey",
    "adbkey.pub",
    "xwdb",
}

TEXT_EXTENSIONS = {
    ".css", ".html", ".js", ".json", ".map", ".mjs", ".rs", ".ts", ".tsx", ".vue"
}


def excluded_commands() -> set[str]:
    return {
        command
        for command in parity_matrix.COMMANDS
        if parity_matrix.status_for(command)[0]
        in {"commercial-excluded", "security-excluded"}
    }


def _token_character(value: int) -> bool:
    return value == ord("_") or ord("0") <= value <= ord("9") or ord("a") <= value <= ord("z")


def matched_excluded_command_tokens(path: Path) -> list[tuple[bytes, str]]:
    tokens = {
        command.encode("ascii"): f"{parity_matrix.status_for(command)[0]} command"
        for command in excluded_commands()
    }
    longest = max(map(len, tokens), default=1)
    matches: dict[bytes, str] = {}
    carry = b""

    def scan(payload: bytes, *, at_eof: bool) -> None:
        for token, reason in tokens.items():
            start = 0
            while (index := payload.find(token, start)) >= 0:
                end = index + len(token)
                left_ok = index == 0 or not _token_character(payload[index - 1])
                right_known = end < len(payload) or at_eof
                right_ok = end == len(payload) or not _token_character(payload[end])
                wire_literal = True
                if token == b"activate":
                    delimiters = b"\"'\0"
                    wire_literal = (
                        index > 0
                        and end < len(payload)
                        and payload[index - 1] in delimiters
                        and payload[end] in delimiters
                    )
                if left_ok and right_known and right_ok and wire_literal:
                    matches[token] = reason
                start = index + 1

    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            payload = (carry + chunk).lower()
            scan(payload, at_eof=False)
            carry = payload[-(longest + 1) :]
    scan(carry, at_eof=True)
    return list(matches.items())


def files_under(path: Path):
    if path.is_file():
        yield path
    elif path.is_dir():
        yield from (candidate for candidate in path.rglob("*") if candidate.is_file())


def matched_needles(path: Path) -> list[tuple[bytes, str]]:
    lowered = tuple((needle.lower(), reason) for needle, reason in BLOCKED_RUNTIME.items())
    longest = max(len(needle) for needle, _ in lowered)
    matches: dict[bytes, str] = {}
    carry = b""
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            payload = carry + chunk.lower()
            for needle, reason in lowered:
                if needle in payload:
                    matches[needle] = reason
            carry = payload[-(longest - 1) :] if longest > 1 else b""
    for needle, reason in matched_excluded_command_tokens(path):
        matches.setdefault(needle, reason)
    return list(matches.items())


def contains_ascii_case_insensitive(path: Path, needle: bytes) -> bool:
    overlap = max(0, len(needle) - 1)
    carry = b""
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            payload = carry + chunk.lower()
            if needle.lower() in payload:
                return True
            carry = payload[-overlap:] if overlap else b""
    return False


def inspect(paths: list[tuple[str, Path]]) -> list[dict[str, str]]:
    findings: list[dict[str, str]] = []
    if not paths:
        return [
            {
                "surface": "gate",
                "path": "",
                "reason": "no scan targets were provided",
            }
        ]
    for surface, root in paths:
        if not root.exists():
            findings.append(
                {
                    "surface": surface,
                    "path": str(root),
                    "reason": "scan target does not exist",
                }
            )
            continue
        files = list(files_under(root))
        if not files:
            findings.append(
                {
                    "surface": surface,
                    "path": str(root),
                    "reason": "scan target contains no files",
                }
            )
            continue
        for path in files:
            lowered_name = path.name.lower()
            if lowered_name in BLOCKED_NAMES or "xiaowei" in lowered_name:
                findings.append({"surface": surface, "path": str(path), "reason": "vendor artifact name"})
            try:
                matches = matched_needles(path)
            except OSError as error:
                findings.append({"surface": surface, "path": str(path), "reason": f"unreadable: {error}"})
                continue
            for _needle, reason in matches:
                findings.append({"surface": surface, "path": str(path), "reason": reason})
            if surface == "frontend" and path.suffix.lower() in TEXT_EXTENSIONS:
                try:
                    branded = contains_ascii_case_insensitive(path, b"xiaowei")
                except OSError:
                    branded = False
            else:
                branded = False
            if branded:
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
