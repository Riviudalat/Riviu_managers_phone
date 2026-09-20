"""Bundle NSIS and MSI from one Tauri build, preserving each updater's bundle identity."""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import time

MARKER = b"__TAURI_BUNDLE_TYPE_VAR_"


def pristine_binary(data: bytes) -> tuple[bytes, int]:
    if data.count(MARKER) != 1:
        raise ValueError("Expected exactly one Tauri bundle marker")
    offset = data.index(MARKER) + len(MARKER)
    if data[offset:offset + 3] not in (b"UNK", b"NSS", b"MSI"):
        raise ValueError("Unknown Tauri bundle marker; build the app again")
    return data[:offset] + b"UNK" + data[offset + 3:], offset


def restore_binary(path: Path, pristine: bytes, offset: int) -> None:
    current = path.read_bytes()
    normalized, current_offset = pristine_binary(current)
    if current_offset != offset or normalized != pristine:
        raise ValueError("App changed outside its bundle marker; refusing to overwrite it")
    if current != pristine:
        path.write_bytes(pristine)


def bundle(
    executable: Path,
    desktop: Path,
    target: str,
    configs: list[Path],
    msi_config: Path,
    report_path: Path,
    *,
    run=subprocess.run,
) -> dict:
    """No compiler, frontend build, resource override or signing-key change here."""
    pristine, offset = pristine_binary(executable.read_bytes())
    report = {
        "executable": str(executable),
        "compiledSha256": hashlib.sha256(pristine).hexdigest(),
        "markerOffset": offset,
        "steps": [],
    }
    report_path.parent.mkdir(parents=True, exist_ok=True)
    cli = desktop / "node_modules/@tauri-apps/cli/tauri.js"
    if not cli.is_file():
        raise FileNotFoundError("Install the locked frontend dependencies before bundling")
    for kind in ("nsis", "msi"):
        restore_binary(executable, pristine, offset)
        overlays = configs + ([msi_config] if kind == "msi" else [])
        command = ["node", str(cli), "bundle", "--ci", "--target", target, "--bundles", kind]
        for config in overlays:
            command.extend(("--config", str(config)))
        started = time.monotonic()
        step = {"kind": kind, "command": command, "exit": None}
        try:
            result = run(command, cwd=desktop, check=False)
            step["exit"] = result.returncode
            if result.returncode:
                raise subprocess.CalledProcessError(result.returncode, command)
        finally:
            step["seconds"] = round(time.monotonic() - started, 2)
            report["steps"].append(step)
            # Tauri may leave NSS in the input file (also after an interrupted bundle).
            # Restore its original bytes before the next bundler can mistake MSI for NSIS.
            try:
                restore_binary(executable, pristine, offset)
            finally:
                report_path.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    return report


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--desktop-dir", type=Path, default=Path("apps/desktop"))
    parser.add_argument("--target", required=True)
    parser.add_argument("--executable", type=Path, required=True)
    parser.add_argument("--config", type=Path, action="append", default=[])
    parser.add_argument("--msi-config", type=Path, required=True)
    parser.add_argument("--report", type=Path, required=True)
    args = parser.parse_args()
    if os.name != "nt" or not args.target.endswith("windows-msvc"):
        parser.error("This command packages Windows installers only")
    report = bundle(
        args.executable.resolve(), args.desktop_dir.resolve(), args.target,
        [config.resolve() for config in args.config], args.msi_config.resolve(),
        args.report.resolve(),
    )
    print(json.dumps(report))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
