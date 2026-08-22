#!/usr/bin/env python3
"""Build and attest the self-contained desktop iOS sidecar runtime."""

from __future__ import annotations

import argparse
import hashlib
import importlib.metadata
import json
import os
import platform
import shutil
import stat
import subprocess
import sys
from pathlib import Path

from packaging.requirements import Requirement
from packaging.utils import canonicalize_name


REPOSITORY_ROOT = Path(__file__).resolve().parents[1]
SIDECAR_ROOT = REPOSITORY_ROOT / "sidecars" / "pymobiledevice3"
SIDECAR_SOURCE = SIDECAR_ROOT / "riviu_pmd.py"
SIGNER_SOURCE = REPOSITORY_ROOT / "sidecars" / "signer" / "riviu_signer.py"
REQUIREMENTS = SIDECAR_ROOT / "requirements.txt"
BUILD_REQUIREMENTS = SIDECAR_ROOT / "requirements-build.txt"
REQUIREMENTS_LOCK = SIDECAR_ROOT / "requirements-lock.txt"
PYINSTALLER_RUNTIME_HOOK = SIDECAR_ROOT / "pyinstaller_runtime_hook.py"
BUILD_INSTALL_SOURCE = REPOSITORY_ROOT / "sidecars" / "wda" / "build_and_install.py"
LEGACY_WDA_SOURCE_LOCK = (
    REPOSITORY_ROOT / "sidecars" / "wda" / "legacy-wda-source-lock.json"
)
DEFAULT_OUTPUT = REPOSITORY_ROOT / "target" / "desktop-sidecar"
DEFAULT_WORK = REPOSITORY_ROOT / "target" / "pyinstaller-work"
DEFAULT_CONFIG = REPOSITORY_ROOT / "target" / "tauri-sidecar.conf.json"


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def normalized_platform() -> str:
    if sys.platform == "win32":
        return "windows"
    if sys.platform == "darwin":
        return "macos"
    raise RuntimeError(f"desktop sidecar packaging is unsupported on {sys.platform}")


def normalized_architecture() -> str:
    machine = platform.machine().lower()
    if machine in {"amd64", "x86_64"}:
        return "x86_64"
    if machine in {"arm64", "aarch64"}:
        return "aarch64"
    raise RuntimeError(f"desktop sidecar packaging is unsupported on {machine}")


def dependency_closure() -> dict[str, str]:
    locked: dict[str, Requirement] = {}
    for line_number, raw_line in enumerate(
        REQUIREMENTS_LOCK.read_text(encoding="utf-8").splitlines(), start=1
    ):
        line = raw_line.strip()
        if not line or line.startswith("#"):
            continue
        requirement = Requirement(line)
        name = canonicalize_name(requirement.name)
        if name in locked:
            raise RuntimeError(
                f"duplicate dependency lock entry {name!r} at line {line_number}"
            )
        locked[name] = requirement

    installed_distributions: dict[str, importlib.metadata.Distribution] = {}
    for distribution in importlib.metadata.distributions():
        raw_name = distribution.metadata.get("Name")
        if not raw_name:
            continue
        installed_distributions[canonicalize_name(raw_name)] = distribution

    roots = ("pymobiledevice3", "tidevice", "pyinstaller")
    reachable: set[str] = set()
    pending = list(roots)
    while pending:
        name = pending.pop()
        if name in reachable:
            continue
        distribution = installed_distributions.get(name)
        if distribution is None:
            raise RuntimeError(f"required build dependency is missing: {name}")
        reachable.add(name)
        for raw_requirement in distribution.requires or ():
            requirement = Requirement(raw_requirement)
            if requirement.marker is not None and not requirement.marker.evaluate():
                continue
            dependency_name = canonicalize_name(requirement.name)
            if dependency_name not in reachable:
                pending.append(dependency_name)

    installed: dict[str, str] = {}
    for name in sorted(reachable):
        distribution = installed_distributions[name]
        requirement = locked.get(name)
        if requirement is None:
            raise RuntimeError(
                f"reachable build dependency {name}=={distribution.version} "
                "is missing from requirements-lock.txt"
            )
        if requirement.marker is not None and not requirement.marker.evaluate():
            raise RuntimeError(
                f"reachable build dependency {name} is inactive in requirements-lock.txt"
            )
        if distribution.version not in requirement.specifier:
            raise RuntimeError(
                f"installed build dependency {name}=={distribution.version} does not "
                f"match lock {requirement.specifier}"
            )
        installed[name] = distribution.version

    active_locked = {
        name: requirement
        for name, requirement in locked.items()
        if requirement.marker is None or requirement.marker.evaluate()
    }
    missing = sorted(set(active_locked) - reachable)
    unexpected = sorted(reachable - set(active_locked))
    if missing or unexpected:
        raise RuntimeError(
            "reachable dependency closure does not exactly match the active lock: "
            f"missing={missing!r}, unexpected={unexpected!r}"
        )
    return dict(sorted(installed.items()))


def run_checked(
    command: list[str],
    *,
    timeout: int = 900,
    capture_output: bool = True,
    env: dict[str, str] | None = None,
) -> subprocess.CompletedProcess:
    try:
        return subprocess.run(
            command,
            cwd=REPOSITORY_ROOT,
            capture_output=capture_output,
            text=True,
            timeout=timeout,
            check=True,
            env=env,
        )
    except (subprocess.CalledProcessError, subprocess.TimeoutExpired) as error:
        stdout = getattr(error, "stdout", "") or ""
        stderr = getattr(error, "stderr", "") or ""
        raise RuntimeError(
            f"sidecar build command failed: {command!r}; "
            f"stdout={stdout[-2000:]!r}; stderr={stderr[-2000:]!r}"
        ) from error


def smoke_runtime(entrypoint: Path) -> tuple[dict, dict]:
    diagnostic_env = dict(os.environ)
    diagnostic_env["RIVIU_SIDECAR_CONTRACT_DIAGNOSTICS"] = "1"
    ping = run_checked(
        [str(entrypoint), "ping"], timeout=90, env=diagnostic_env
    )
    payload = json.loads(ping.stdout.strip().splitlines()[-1])
    required_contract = "verifiedProcessControl"
    if not (
        payload.get("ok") is True
        and payload.get("pymobiledevice3") is True
        and payload.get("sidecarProtocolVersion") == 2
        and required_contract in payload.get("contracts", [])
    ):
        diagnostics = (ping.stderr or "")[-2000:]
        raise RuntimeError(
            f"frozen sidecar ping contract failed: {payload!r}; "
            f"diagnostics={diagnostics!r}"
        )

    tidevice = run_checked(
        [str(entrypoint), "__tidevice", "--version"],
        timeout=90,
    )
    expected_tidevice = importlib.metadata.version("tidevice")
    if tidevice.stdout.strip() != expected_tidevice:
        raise RuntimeError(
            "embedded tidevice version mismatch: "
            f"expected {expected_tidevice!r}, got {tidevice.stdout.strip()!r}"
        )

    signer = run_checked(
        [str(entrypoint), "__script", str(SIGNER_SOURCE), "--help"],
        timeout=90,
    )
    if "sign-install-wda" not in signer.stdout:
        raise RuntimeError("embedded signer command smoke test failed")

    signer_error_json = "NOT_APPLICABLE"
    if sys.platform == "win32":
        signer_failure = subprocess.run(
            [
                str(entrypoint),
                "__script",
                str(SIGNER_SOURCE),
                "sign-install-wda",
                "--udid",
                "FIXTURE_ONLY",
            ],
            cwd=REPOSITORY_ROOT,
            # Credentials by environment, never argv — the signer stopped accepting
            # `--apple-id`/`--password` because a Windows command line is readable by every
            # process running as the same user. Passed here anyway so this smoke test keeps
            # exercising the same path the desktop uses.
            env={
                **os.environ,
                "RIVIU_APPLE_ID": "fixture@example.test",
                "RIVIU_APPLE_PASSWORD": "FIXTURE_ONLY",
            },
            capture_output=True,
            text=True,
            encoding="utf-8",
            errors="strict",
            timeout=90,
        )
        error_lines = [
            line
            for line in signer_failure.stdout.splitlines()
            if line.strip().startswith("{")
        ]
        error_payload = json.loads(error_lines[-1]) if error_lines else {}
        if not (
            signer_failure.returncode == 1
            and error_payload.get("ok") is False
            and "Traceback" not in signer_failure.stderr
        ):
            raise RuntimeError("embedded signer did not return structured UTF-8 JSON")
        signer_error_json = "PASS"

    signing_resources = run_checked(
        [str(entrypoint), "__script", str(BUILD_INSTALL_SOURCE), "--self-test"],
        timeout=90,
    )
    signing_payload = json.loads(signing_resources.stdout.strip().splitlines()[-1])
    if not (
        signing_payload.get("ok") is True
        and signing_payload.get("kind") == "packagedSigningResources"
        and signing_payload.get("workspaceOutsideResources") is True
    ):
        raise RuntimeError(
            f"embedded signing resource self-test failed: {signing_payload!r}"
        )
    payload["signerErrorJson"] = signer_error_json
    return payload, signing_payload


def tree_attestation(root: Path, *, ignored: set[str] | None = None) -> dict:
    ignored = ignored or set()
    entries: list[tuple[str, str, int, int, str]] = []
    total_bytes = 0
    for current, directory_names, file_names in os.walk(root, followlinks=False):
        current_path = Path(current)
        for name in list(directory_names):
            path = current_path / name
            if path.is_symlink():
                target = os.readlink(path)
                entries.append(
                    (
                        path.relative_to(root).as_posix(),
                        "symlink",
                        stat.S_IMODE(path.lstat().st_mode),
                        len(target.encode("utf-8")),
                        target,
                    )
                )
                directory_names.remove(name)
        for name in file_names:
            path = current_path / name
            relative = path.relative_to(root).as_posix()
            if relative in ignored:
                continue
            mode = stat.S_IMODE(path.lstat().st_mode)
            if path.is_symlink():
                target = os.readlink(path)
                entries.append(
                    (
                        relative,
                        "symlink",
                        mode,
                        len(target.encode("utf-8")),
                        target,
                    )
                )
                continue
            size = path.stat().st_size
            entries.append((relative, "file", mode, size, sha256_file(path)))
            total_bytes += size

    tree = hashlib.sha256()
    for relative, kind, mode, size, value in sorted(entries):
        tree.update(relative.encode("utf-8"))
        tree.update(b"\0")
        tree.update(kind.encode("ascii"))
        tree.update(b"\0")
        tree.update(f"{mode:o}".encode("ascii"))
        tree.update(b"\0")
        tree.update(str(size).encode("ascii"))
        tree.update(b"\0")
        tree.update(value.encode("utf-8"))
        tree.update(b"\n")
    return {
        "fileCount": len(entries),
        "payloadBytes": total_bytes,
        "treeSha256": tree.hexdigest(),
    }


def payload_attestation(
    runtime_dir: Path,
    entrypoint: Path,
    dependencies: dict[str, str],
    signing_resources: dict,
) -> dict:
    measured_tree = tree_attestation(
        runtime_dir, ignored={"runtime-manifest.json"}
    )

    return {
        "schemaVersion": 1,
        "kind": "pyinstaller-onedir",
        "platform": normalized_platform(),
        "architecture": normalized_architecture(),
        "pythonVersion": platform.python_version(),
        "dependencies": {
            "pymobiledevice3": importlib.metadata.version("pymobiledevice3"),
            "tidevice": importlib.metadata.version("tidevice"),
            "pyinstaller": importlib.metadata.version("pyinstaller"),
        },
        "entrypoint": entrypoint.relative_to(runtime_dir).as_posix(),
        "entrypointSha256": sha256_file(entrypoint),
        "sourceSha256": sha256_file(SIDECAR_SOURCE),
        "requirementsSha256": sha256_file(REQUIREMENTS),
        "buildRequirementsSha256": sha256_file(BUILD_REQUIREMENTS),
        "requirementsLockSha256": sha256_file(REQUIREMENTS_LOCK),
        "runtimeHookSha256": sha256_file(PYINSTALLER_RUNTIME_HOOK),
        "signerSourceSha256": sha256_file(SIGNER_SOURCE),
        "buildInstallSourceSha256": sha256_file(BUILD_INSTALL_SOURCE),
        "legacyWdaSourceLockSha256": sha256_file(LEGACY_WDA_SOURCE_LOCK),
        "dependencyClosure": dependencies,
        "signingResources": {
            "sourceVersion": signing_resources["sourceVersion"],
            "sourceTreeSha256": signing_resources["sourceTreeSha256"],
            "logoSha256": signing_resources["logoSha256"],
            "iconSetTreeSha256": signing_resources["iconSetTreeSha256"],
            "workspaceOutsideResources": True,
        },
        **measured_tree,
    }


def build_runtime(output_root: Path, work_root: Path) -> tuple[Path, dict]:
    runtime_dir = output_root / "riviu-pmd"
    if runtime_dir.exists():
        shutil.rmtree(runtime_dir)
    if work_root.exists():
        shutil.rmtree(work_root)
    output_root.mkdir(parents=True, exist_ok=True)
    work_root.mkdir(parents=True, exist_ok=True)

    dependencies = dependency_closure()
    command = [
        sys.executable,
        "-m",
        "PyInstaller",
        "--noconfirm",
        "--clean",
        "--onedir",
        "--noupx",
        "--name",
        "riviu-pmd",
        "--distpath",
        str(output_root),
        "--workpath",
        str(work_root / "build"),
        "--specpath",
        str(work_root / "spec"),
        "--collect-data",
        "pymobiledevice3",
        "--collect-data",
        "tidevice",
        "--copy-metadata",
        "pymobiledevice3",
        "--copy-metadata",
        "tidevice",
        "--runtime-hook",
        str(PYINSTALLER_RUNTIME_HOOK),
        "--exclude-module",
        "IPython",
        str(SIDECAR_SOURCE),
    ]
    run_checked(command, capture_output=False)

    entrypoint = runtime_dir / ("riviu-pmd.exe" if sys.platform == "win32" else "riviu-pmd")
    if not entrypoint.is_file():
        raise RuntimeError(f"PyInstaller entrypoint missing: {entrypoint}")
    if sys.platform != "win32":
        entrypoint.chmod(entrypoint.stat().st_mode | 0o111)

    ping, signing_resources = smoke_runtime(entrypoint)
    manifest = payload_attestation(
        runtime_dir, entrypoint, dependencies, signing_resources
    )
    manifest["smoke"] = {
        "ping": "PASS",
        "embeddedTidevice": "PASS",
        "embeddedSigner": "PASS",
        "embeddedSigningResources": "PASS",
        "embeddedSignerErrorJson": ping["signerErrorJson"],
        "contracts": ping["contracts"],
    }
    with (runtime_dir / "runtime-manifest.json").open(
        "w", encoding="utf-8", newline="\n"
    ) as handle:
        handle.write(json.dumps(manifest, indent=2, sort_keys=True) + "\n")
    return runtime_dir, manifest


def write_tauri_config(path: Path, runtime_dir: Path) -> None:
    source = runtime_dir.resolve().as_posix()
    if sys.platform == "darwin":
        config: dict = {
            "bundle": {
                "macOS": {
                    "files": {
                        "Resources/sidecars/pymobiledevice3/runtime": source,
                    },
                    "signingIdentity": os.environ.get("APPLE_SIGNING_IDENTITY", "-"),
                }
            }
        }
    else:
        config = {
            "bundle": {
                "resources": {
                    source + "/": "sidecars/pymobiledevice3/runtime/",
                }
            }
        }
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("w", encoding="utf-8", newline="\n") as handle:
        handle.write(json.dumps(config, indent=2, sort_keys=True) + "\n")


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--output", type=Path, default=DEFAULT_OUTPUT)
    parser.add_argument("--work", type=Path, default=DEFAULT_WORK)
    parser.add_argument("--tauri-config", type=Path, default=DEFAULT_CONFIG)
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    runtime_dir, manifest = build_runtime(args.output.resolve(), args.work.resolve())
    write_tauri_config(args.tauri_config.resolve(), runtime_dir)
    print(
        json.dumps(
            {
                "ok": True,
                "runtime": str(runtime_dir),
                "tauriConfig": str(args.tauri_config.resolve()),
                "payloadBytes": manifest["payloadBytes"],
                "treeSha256": manifest["treeSha256"],
            },
            sort_keys=True,
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
