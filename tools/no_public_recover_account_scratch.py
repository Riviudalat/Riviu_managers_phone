"""Read-only preflight for the exact account-inspect scratch recovery.

Approval is ordered: prove/close the exact session and owned stream, THEN stop the
scratch process. This binary exposes no proven quarantined-context extraction/close
entry point and receipts contain no session/producer identity. Therefore this script
never terminates, closes, launches, archives, clears fences, or calls a device.
"""
from __future__ import annotations

import argparse
import ctypes
from ctypes import wintypes
import hashlib
import json
import os
from pathlib import Path
import sys
from typing import Any

ACTIVATION = "ddea5d06-9991-48de-8557-a13e39195aea"
REQUEST = "50b74ffd-1ae1-4b68-bf43-9d5c2a75be21"
PID = 35488
START_TICKS = "639264891585141270"
SERIAL = "ce031713b0c610ab0c"
EXE_SHA256 = "95224ddbc5167b5aeab5b73acf16c43e5edfecae88a7f7b28f0ac7e3670b87cb"
PACKAGE = "com.zhiliaoapp.musically"
REPO = Path(__file__).resolve().parents[1]
LIVE = REPO / "target/no-public-live"
RUN = LIVE / f"{ACTIVATION}.run" / f"run-{REQUEST}"
FILETIME_TO_DOTNET = 504911232000000000


def require(value: bool, message: str) -> None:
    if not value:
        raise RuntimeError(message)


def digest(path: Path) -> str:
    value = hashlib.sha256()
    with path.open("rb") as source:
        for block in iter(lambda: source.read(1024 * 1024), b""):
            value.update(block)
    return value.hexdigest()


def read_json(path: Path) -> dict[str, Any]:
    value = json.loads(path.read_text(encoding="utf-8-sig"))
    require(isinstance(value, dict), "invalid evidence object")
    return value


def validate_evidence() -> tuple[Path, dict[str, str]]:
    process_path = LIVE / f"{ACTIVATION}.process.json"
    record = read_json(process_path)
    require(record.get("activationId") == ACTIVATION and record.get("pid") == PID
            and record.get("exeSha256") == EXE_SHA256 and record.get("publicEffectsAllowed") is False,
            "scratch launch identity differs")
    exe = Path(record["exe"]).resolve(strict=True)
    require(exe == (REPO / "target/debug/riviu-managers-phone.exe").resolve(strict=True), "executable path differs")
    require(digest(exe) == EXE_SHA256, "executable bytes differ")
    require(Path(record["dataRoot"]).resolve() == RUN.parent.resolve(), "scratch data root differs")
    scope_path = Path(record["scope"]).resolve(strict=True)
    require(scope_path == (LIVE / f"{ACTIVATION}.scope.json").resolve(strict=True), "scope path differs")
    scope = read_json(scope_path)
    devices = scope.get("deviceScopes")
    require(scope.get("activationId") == ACTIVATION and isinstance(devices, list) and len(devices) == 1,
            "scope identity differs")
    assert isinstance(devices, list)
    require(isinstance(devices[0], dict), "invalid device scope")
    require(devices[0].get("udid") == SERIAL and devices[0].get("package") == PACKAGE,
            "scope device/package differs")
    files = [process_path, scope_path, RUN / "intent.json", RUN / "status.json"]
    for path in files[2:]:
        value = read_json(path)
        require(value.get("activationId") == ACTIVATION and value.get("requestId") == REQUEST
                and value.get("udid") == SERIAL and value.get("kind") == "inspect"
                and value.get("inputDigest") == PACKAGE and value.get("publicEffectsAllowed") is False,
                "inspect receipt identity differs")
    status = read_json(files[3])
    require(status.get("state") == "needsAttention" and status.get("phase") == "readingAccount",
            "inspect attention state differs")
    require(status.get("outcome", {}).get("error") ==
            "no-public prepare blocked (ComposerUnproved); cleanup=NotCreated", "inspect blocker differs")
    return exe, {str(path): digest(path) for path in files}


def process_identity() -> dict[str, Any]:
    require(os.name == "nt", "Windows identity API required")
    api = ctypes.WinDLL("kernel32", use_last_error=True)
    api.OpenProcess.argtypes = [wintypes.DWORD, wintypes.BOOL, wintypes.DWORD]
    api.OpenProcess.restype = wintypes.HANDLE
    api.CloseHandle.argtypes = [wintypes.HANDLE]
    api.QueryFullProcessImageNameW.argtypes = [wintypes.HANDLE, wintypes.DWORD, wintypes.LPWSTR, ctypes.POINTER(wintypes.DWORD)]
    api.GetProcessTimes.argtypes = [wintypes.HANDLE] + [ctypes.POINTER(wintypes.FILETIME)] * 4
    api.WaitForSingleObject.argtypes = [wintypes.HANDLE, wintypes.DWORD]
    api.WaitForSingleObject.restype = wintypes.DWORD
    # Read-only QUERY_LIMITED + SYNCHRONIZE; no PROCESS_TERMINATE capability.
    handle = api.OpenProcess(0x1000 | 0x100000, False, PID)
    require(bool(handle), "exact process absent or unreadable")
    try:
        require(api.WaitForSingleObject(handle, 0) == 0x102, "pinned process already exited")
        size = wintypes.DWORD(32768)
        path = ctypes.create_unicode_buffer(size.value)
        require(bool(api.QueryFullProcessImageNameW(handle, 0, path, ctypes.byref(size))), "image identity unreadable")
        created, exited, kernel, user = (wintypes.FILETIME() for _ in range(4))
        require(bool(api.GetProcessTimes(handle, ctypes.byref(created), ctypes.byref(exited), ctypes.byref(kernel), ctypes.byref(user))),
                "process incarnation unreadable")
        ticks = ((created.dwHighDateTime << 32) | created.dwLowDateTime) + FILETIME_TO_DOTNET
        return {"pid": PID, "path": str(Path(path.value).resolve()), "startTicks": str(ticks)}
    finally:
        api.CloseHandle(handle)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.parse_args()  # No execution/termination mode exists.
    exe, evidence_hashes = validate_evidence()
    identity = process_identity()
    require(Path(identity["path"]) == exe and identity["startTicks"] == START_TICKS, "process incarnation differs")
    report = {
        "activationId": ACTIVATION, "requestId": REQUEST, "deviceId": SERIAL,
        "process": identity, "exeSha256": EXE_SHA256, "identityVerified": True,
        "state": "recoveryBlocked", "publicEffectsAllowed": False,
        "sessionCloseProved": False, "ownedProducerCloseProved": False,
        "sessionId": None, "producerIdentity": None,
        "scratchStopAllowed": False, "scratchStopped": False,
        "deviceCommandsIssued": 0, "fencesCleared": False, "receiptsModified": False,
        "evidenceSha256": evidence_hashes,
        "reason": "exact live session/producer identity and native quarantined-context close entry point unavailable",
        "next": "retain this process/fences; obtain explicit exact-controller recovery capability or revised operator authorization; do not DELETE sessions, kill borrowed runners, or bypass the controller",
    }
    print(json.dumps(report, indent=2))
    return 2  # Blocked, not successful cleanup.


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (RuntimeError, OSError, ValueError, KeyError) as failure:
        print(json.dumps({"state": "recoveryBlocked", "identityVerified": False,
                          "reason": str(failure) if isinstance(failure, RuntimeError) else type(failure).__name__,
                          "scratchStopped": False, "deviceCommandsIssued": 0, "fencesCleared": False}), file=sys.stderr)
        raise SystemExit(2)
