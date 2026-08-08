#!/usr/bin/env python3
"""Collect and verify desktop CI artifacts without modifying release inputs."""

from __future__ import annotations

import argparse
from dataclasses import dataclass
import hashlib
import json
import os
import plistlib
import shutil
import stat
import subprocess
import sys
import tempfile
import time
import tomllib
from pathlib import Path
from typing import Any

from packaging.markers import default_environment
from packaging.requirements import Requirement
from packaging.utils import canonicalize_name


REPOSITORY_ROOT = Path(__file__).resolve().parents[1]
WDA_ROOT = REPOSITORY_ROOT / "sidecars" / "wda"
PRODUCTION_MANIFEST = WDA_ROOT / "agent-manifest.json"
SIDECAR_SOURCE = REPOSITORY_ROOT / "sidecars" / "pymobiledevice3" / "riviu_pmd.py"
SIDECAR_REQUIREMENTS = (
    REPOSITORY_ROOT / "sidecars" / "pymobiledevice3" / "requirements.txt"
)
SIDECAR_BUILD_REQUIREMENTS = (
    REPOSITORY_ROOT / "sidecars" / "pymobiledevice3" / "requirements-build.txt"
)
SIDECAR_REQUIREMENTS_LOCK = (
    REPOSITORY_ROOT / "sidecars" / "pymobiledevice3" / "requirements-lock.txt"
)
SIDECAR_RUNTIME_HOOK = (
    REPOSITORY_ROOT / "sidecars" / "pymobiledevice3" / "pyinstaller_runtime_hook.py"
)
SIGNER_SOURCE = REPOSITORY_ROOT / "sidecars" / "signer" / "riviu_signer.py"
BUILD_INSTALL_SOURCE = REPOSITORY_ROOT / "sidecars" / "wda" / "build_and_install.py"
LEGACY_WDA_SOURCE_LOCK = (
    REPOSITORY_ROOT / "sidecars" / "wda" / "legacy-wda-source-lock.json"
)
LEGACY_WDA_SOURCE = REPOSITORY_ROOT / "sidecars" / "wda" / "WebDriverAgent"
LEGACY_WDA_ICONSET = REPOSITORY_ROOT / "sidecars" / "wda" / "AppIcon.appiconset"
BRANDING_LOGO = REPOSITORY_ROOT / "logo.jpg"
TAURI_CONFIG = REPOSITORY_ROOT / "apps" / "desktop" / "src-tauri" / "tauri.conf.json"
TAURI_CARGO_MANIFEST = (
    REPOSITORY_ROOT / "apps" / "desktop" / "src-tauri" / "Cargo.toml"
)
DESKTOP_PACKAGE_JSON = REPOSITORY_ROOT / "apps" / "desktop" / "package.json"
RUNTIME_RESOURCE_DESTINATION = "sidecars/pymobiledevice3/runtime/"
MACOS_RUNTIME_CONTENTS_DESTINATION = (
    "Resources/" + RUNTIME_RESOURCE_DESTINATION.rstrip("/")
)
CANONICAL_PRODUCTION_SHA256 = {
    "sidecars/wda/RiviuAgent.ipa": (
        "8a24847099495ff70b998522692c43f00dd16b90f698bda6953a73f5d33002ea"
    ),
    "sidecars/wda/agent-manifest.json": (
        "e98a549af4c061556effd36424e7732219e1a6d262bcf1f259279975024b6e1a"
    ),
}
EXPECTED_RELEASE_PYTHON_VERSION = "3.12.10"

TARGETS = {
    "x86_64-pc-windows-msvc": {
        "platform": "windows",
        "architecture": "x86_64",
        "installer_suffixes": (".msi", ".exe"),
    },
    "aarch64-apple-darwin": {
        "platform": "macos",
        "architecture": "aarch64",
        "installer_suffixes": (".dmg",),
    },
    "x86_64-apple-darwin": {
        "platform": "macos",
        "architecture": "x86_64",
        "installer_suffixes": (".dmg",),
    },
}
RELEASE_LABEL_TARGETS = {
    "windows-x64": "x86_64-pc-windows-msvc",
    "macos-arm64": "aarch64-apple-darwin",
    "macos-x64": "x86_64-apple-darwin",
}


class ArtifactError(RuntimeError):
    """Raised when a build output fails its release contract."""


def release_marker_environment() -> dict[str, str]:
    environment = default_environment()
    environment["python_full_version"] = EXPECTED_RELEASE_PYTHON_VERSION
    environment["python_version"] = ".".join(
        EXPECTED_RELEASE_PYTHON_VERSION.split(".")[:2]
    )
    return environment


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def tree_entries(root: Path, *, ignored: set[str] | None = None) -> list[dict[str, Any]]:
    ignored = ignored or set()
    entries: list[dict[str, Any]] = []
    for current, directory_names, file_names in os.walk(root, followlinks=False):
        current_path = Path(current)
        for name in list(directory_names):
            path = current_path / name
            if path.is_symlink():
                target = os.readlink(path)
                entries.append(
                    {
                        "path": path.relative_to(root).as_posix(),
                        "kind": "symlink",
                        "mode": stat.S_IMODE(path.lstat().st_mode),
                        "size": len(target.encode("utf-8")),
                        "value": target,
                    }
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
                    {
                        "path": relative,
                        "kind": "symlink",
                        "mode": mode,
                        "size": len(target.encode("utf-8")),
                        "value": target,
                    }
                )
            else:
                entries.append(
                    {
                        "path": relative,
                        "kind": "file",
                        "mode": mode,
                        "size": path.stat().st_size,
                        "value": sha256_file(path),
                    }
                )
    return sorted(entries, key=lambda entry: entry["path"])


def tree_attestation(root: Path, *, ignored: set[str] | None = None) -> dict[str, Any]:
    entries = tree_entries(root, ignored=ignored)
    tree = hashlib.sha256()
    payload_bytes = 0
    for entry in entries:
        tree.update(entry["path"].encode("utf-8"))
        tree.update(b"\0")
        tree.update(entry["kind"].encode("ascii"))
        tree.update(b"\0")
        tree.update(f"{entry['mode']:o}".encode("ascii"))
        tree.update(b"\0")
        tree.update(str(entry["size"]).encode("ascii"))
        tree.update(b"\0")
        tree.update(entry["value"].encode("utf-8"))
        tree.update(b"\n")
        if entry["kind"] == "file":
            payload_bytes += entry["size"]
    return {
        "fileCount": len(entries),
        "payloadBytes": payload_bytes,
        "treeSha256": tree.hexdigest(),
    }


def production_sha256(path: Path) -> str:
    if path != PRODUCTION_MANIFEST:
        return sha256_file(path)
    canonical_lf = path.read_bytes().replace(b"\r\n", b"\n")
    return hashlib.sha256(canonical_lf).hexdigest()


def load_json(path: Path) -> dict[str, Any]:
    if not path.is_file():
        raise ArtifactError(f"JSON file is missing: {path}")

    def reject_duplicate(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
        result: dict[str, Any] = {}
        for key, value in pairs:
            if key in result:
                raise ArtifactError(f"duplicate JSON key {key!r} in {path}")
            result[key] = value
        return result

    try:
        value = json.loads(path.read_text(encoding="utf-8"), object_pairs_hook=reject_duplicate)
    except (OSError, UnicodeDecodeError, json.JSONDecodeError) as error:
        raise ArtifactError(f"invalid JSON in {path}: {error}") from error
    if not isinstance(value, dict):
        raise ArtifactError(f"JSON root must be an object: {path}")
    return value


def write_json(path: Path, value: dict[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_suffix(path.suffix + ".tmp")
    temporary.write_text(
        json.dumps(value, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
        newline="\n",
    )
    temporary.replace(path)


def production_paths() -> list[Path]:
    paths = sorted(path for path in WDA_ROOT.glob("*.ipa") if path.is_file())
    if PRODUCTION_MANIFEST.is_file():
        paths.append(PRODUCTION_MANIFEST)
    if not paths:
        raise ArtifactError(f"no production WDA inputs found under {WDA_ROOT}")
    return paths


def production_snapshot(*, require_canonical: bool = False) -> dict[str, Any]:
    files = []
    for path in production_paths():
        files.append(
            {
                "path": path.relative_to(REPOSITORY_ROOT).as_posix(),
                "bytes": path.stat().st_size,
                "sha256": production_sha256(path),
            }
        )
    snapshot = {"schemaVersion": 1, "files": files}
    if require_canonical:
        for relative, expected in CANONICAL_PRODUCTION_SHA256.items():
            source = REPOSITORY_ROOT / relative
            actual = production_sha256(source) if source.is_file() else None
            if actual != expected:
                raise ArtifactError(
                    f"canonical production SHA-256 mismatch for {relative}: "
                    f"expected {expected}, got {actual}"
                )
    return snapshot


def snapshot_command(args: argparse.Namespace) -> dict[str, Any]:
    snapshot = production_snapshot(require_canonical=args.require_canonical_production)
    write_json(args.output.resolve(), snapshot)
    return {
        "ok": True,
        "command": "snapshot-production",
        "output": str(args.output.resolve()),
        "fileCount": len(snapshot["files"]),
    }


def verify_version_command(args: argparse.Namespace) -> dict[str, Any]:
    tauri_version = load_json(TAURI_CONFIG).get("version")
    npm_version = load_json(DESKTOP_PACKAGE_JSON).get("version")
    try:
        cargo_document = tomllib.loads(
            TAURI_CARGO_MANIFEST.read_text(encoding="utf-8")
        )
        cargo_version = cargo_document["package"]["version"]
    except (OSError, UnicodeDecodeError, tomllib.TOMLDecodeError, KeyError) as error:
        raise ArtifactError(f"invalid desktop Cargo version: {error}") from error
    versions = {
        "tauri": tauri_version,
        "npm": npm_version,
        "cargo": cargo_version,
    }
    if any(not isinstance(version, str) or not version for version in versions.values()):
        raise ArtifactError(f"desktop version fields must be non-empty strings: {versions!r}")
    if len(set(versions.values())) != 1:
        raise ArtifactError(f"desktop version fields do not match: {versions!r}")
    version = cargo_version
    if args.tag is not None and args.tag != f"v{version}":
        raise ArtifactError(
            f"release tag mismatch: expected v{version!s}, got {args.tag!r}"
        )
    return {
        "ok": True,
        "command": "verify-version",
        "version": version,
        "tag": args.tag,
    }


def verify_production_snapshot(path: Path) -> dict[str, Any]:
    expected = load_json(path)
    actual = production_snapshot()
    if expected != actual:
        raise ArtifactError(
            "production WDA inputs changed between snapshot and artifact collection"
        )
    return actual


def verify_runtime(runtime_dir: Path, target: str) -> dict[str, Any]:
    target_contract = TARGETS[target]
    manifest_path = runtime_dir / "runtime-manifest.json"
    manifest = load_json(manifest_path)

    expected_fields = {
        "schemaVersion": 1,
        "kind": "pyinstaller-onedir",
        "platform": target_contract["platform"],
        "architecture": target_contract["architecture"],
    }
    for key, expected in expected_fields.items():
        if manifest.get(key) != expected:
            raise ArtifactError(
                f"runtime manifest {key} mismatch: expected {expected!r}, "
                f"got {manifest.get(key)!r}"
            )

    python_version = manifest.get("pythonVersion")
    if python_version != EXPECTED_RELEASE_PYTHON_VERSION:
        raise ArtifactError(
            "runtime Python version mismatch: expected exact "
            f"{EXPECTED_RELEASE_PYTHON_VERSION!r}, got {python_version!r}"
        )

    smoke = manifest.get("smoke")
    if (
        not isinstance(smoke, dict)
        or smoke.get("ping") != "PASS"
        or smoke.get("embeddedTidevice") != "PASS"
        or smoke.get("embeddedSigner") != "PASS"
        or smoke.get("embeddedSigningResources") != "PASS"
    ):
        raise ArtifactError(f"runtime smoke attestation is incomplete: {smoke!r}")
    expected_error_json = (
        "PASS" if target == "x86_64-pc-windows-msvc" else "NOT_APPLICABLE"
    )
    if smoke.get("embeddedSignerErrorJson") != expected_error_json:
        raise ArtifactError("runtime signer error JSON smoke attestation is incomplete")

    expected_dependencies = {
        "pymobiledevice3": "10.1.0",
        "tidevice": "0.12.11",
        "pyinstaller": "6.21.0",
    }
    if manifest.get("dependencies") != expected_dependencies:
        raise ArtifactError(
            "runtime dependency versions do not match the release lock: "
            f"{manifest.get('dependencies')!r}"
        )

    active_locks: dict[str, Requirement] = {}
    marker_environment = release_marker_environment()
    for line_number, raw_line in enumerate(
        SIDECAR_REQUIREMENTS_LOCK.read_text(encoding="utf-8").splitlines(), start=1
    ):
        line = raw_line.strip()
        if not line or line.startswith("#"):
            continue
        requirement = Requirement(line)
        name = canonicalize_name(requirement.name)
        if name in active_locks:
            raise ArtifactError(
                f"duplicate dependency lock entry {name!r} at line {line_number}"
            )
        if requirement.marker is None or requirement.marker.evaluate(
            environment=marker_environment
        ):
            active_locks[name] = requirement

    closure = manifest.get("dependencyClosure")
    if not isinstance(closure, dict) or not closure:
        raise ArtifactError("runtime dependency closure is missing")
    canonical_closure: dict[str, str] = {}
    for raw_name, version in closure.items():
        if not isinstance(raw_name, str) or not isinstance(version, str):
            raise ArtifactError("runtime dependency closure must map names to versions")
        name = canonicalize_name(raw_name)
        if name in canonical_closure:
            raise ArtifactError(f"duplicate canonical runtime dependency: {name}")
        requirement = active_locks.get(name)
        if requirement is None or version not in requirement.specifier:
            raise ArtifactError(
                f"runtime dependency {raw_name}=={version} is not locked for this platform"
            )
        canonical_closure[name] = version
    if set(canonical_closure) != set(active_locks):
        missing = sorted(set(active_locks) - set(canonical_closure))
        unexpected = sorted(set(canonical_closure) - set(active_locks))
        raise ArtifactError(
            "runtime dependency closure does not exactly match the active lock: "
            f"missing={missing!r}, unexpected={unexpected!r}"
        )
    for name, version in expected_dependencies.items():
        if canonical_closure.get(name) != version:
            raise ArtifactError(
                f"runtime dependency closure is missing {name}=={version}"
            )

    source_lock = load_json(LEGACY_WDA_SOURCE_LOCK)
    signing_resources = manifest.get("signingResources")
    expected_signing_resources = {
        "sourceVersion": source_lock.get("version"),
        "sourceTreeSha256": source_lock.get("treeSha256"),
        "logoSha256": source_lock.get("logoSha256"),
        "iconSetTreeSha256": source_lock.get("iconSetTreeSha256"),
        "workspaceOutsideResources": True,
    }
    if signing_resources != expected_signing_resources:
        raise ArtifactError(
            "runtime signing resource attestation does not match the pinned source lock"
        )

    expected_inputs = {
        "sourceSha256": SIDECAR_SOURCE,
        "requirementsSha256": SIDECAR_REQUIREMENTS,
        "buildRequirementsSha256": SIDECAR_BUILD_REQUIREMENTS,
        "requirementsLockSha256": SIDECAR_REQUIREMENTS_LOCK,
        "runtimeHookSha256": SIDECAR_RUNTIME_HOOK,
        "signerSourceSha256": SIGNER_SOURCE,
        "buildInstallSourceSha256": BUILD_INSTALL_SOURCE,
        "legacyWdaSourceLockSha256": LEGACY_WDA_SOURCE_LOCK,
    }
    for field, source in expected_inputs.items():
        expected = sha256_file(source)
        if manifest.get(field) != expected:
            raise ArtifactError(
                f"runtime input {field} mismatch: expected {expected!r}, "
                f"got {manifest.get(field)!r}"
            )

    entrypoint_value = manifest.get("entrypoint")
    if not isinstance(entrypoint_value, str):
        raise ArtifactError("runtime manifest entrypoint must be a string")
    entrypoint = (runtime_dir / entrypoint_value).resolve()
    try:
        entrypoint.relative_to(runtime_dir.resolve())
    except ValueError as error:
        raise ArtifactError("runtime manifest entrypoint escapes the runtime directory") from error
    if not entrypoint.is_file():
        raise ArtifactError(f"runtime entrypoint is missing: {entrypoint}")
    if sha256_file(entrypoint) != manifest.get("entrypointSha256"):
        raise ArtifactError("runtime entrypoint SHA-256 does not match its manifest")

    measured = tree_attestation(
        runtime_dir, ignored={"runtime-manifest.json"}
    )
    for key, actual in measured.items():
        if manifest.get(key) != actual:
            raise ArtifactError(
                f"runtime payload {key} mismatch: expected {manifest.get(key)!r}, "
                f"measured {actual!r}"
            )
    return manifest


def verify_overlay(overlay_path: Path, runtime_dir: Path, target: str) -> dict[str, Any]:
    overlay = load_json(overlay_path)
    bundle = overlay.get("bundle")
    if not isinstance(bundle, dict):
        raise ArtifactError("Tauri overlay is missing bundle configuration")

    if target.endswith("apple-darwin"):
        macos = bundle.get("macOS")
        if not isinstance(macos, dict) or not macos.get("signingIdentity"):
            raise ArtifactError("macOS Tauri overlay is missing a signing identity")
        expected_macos = {
            "files": {
                MACOS_RUNTIME_CONTENTS_DESTINATION: runtime_dir.resolve().as_posix()
            },
            "signingIdentity": macos["signingIdentity"],
        }
        if bundle != {"macOS": expected_macos}:
            raise ArtifactError(
                "macOS Tauri overlay must map the exact runtime with macOS.files "
                "so PyInstaller symlinks are preserved"
            )
    else:
        expected_source = runtime_dir.resolve().as_posix() + "/"
        expected_bundle = {
            "resources": {expected_source: RUNTIME_RESOURCE_DESTINATION}
        }
        if bundle != expected_bundle:
            raise ArtifactError(
                "Tauri overlay does not map exactly the attested runtime directory "
                f"to {RUNTIME_RESOURCE_DESTINATION!r}"
            )
    return overlay


def find_installers(bundle_dir: Path, target: str) -> list[Path]:
    if not bundle_dir.is_dir():
        raise ArtifactError(f"Tauri bundle directory is missing: {bundle_dir}")
    suffixes = TARGETS[target]["installer_suffixes"]
    installers = sorted(
        path
        for path in bundle_dir.rglob("*")
        if path.is_file() and path.suffix.lower() in suffixes
    )

    suffix_counts = {suffix: 0 for suffix in suffixes}
    for path in installers:
        suffix_counts[path.suffix.lower()] += 1
    missing = [suffix for suffix, count in suffix_counts.items() if count == 0]
    if missing:
        raise ArtifactError(
            f"Tauri bundle is missing installer type(s) {missing!r} under {bundle_dir}"
        )

    return installers


def read_log_tail(path: Path, *, limit: int = 2000) -> str:
    """Best-effort tail of a tool log; absence is itself worth reporting."""
    try:
        # msiexec writes UTF-16 on some locales; never let decoding mask the
        # failure we are trying to explain.
        return path.read_text(encoding="utf-8", errors="replace")[-limit:]
    except OSError as error:
        return f"<unreadable: {error}>"


def run_checked(command: list[str], *, timeout: int = 120) -> subprocess.CompletedProcess[str]:
    try:
        return subprocess.run(
            command,
            cwd=REPOSITORY_ROOT,
            capture_output=True,
            text=True,
            timeout=timeout,
            check=True,
        )
    except (subprocess.CalledProcessError, subprocess.TimeoutExpired) as error:
        stdout = getattr(error, "stdout", "") or ""
        stderr = getattr(error, "stderr", "") or ""
        # msiexec reports everything through its exit code and writes nothing to
        # either stream, so omitting the code left its failures undiagnosable.
        code = getattr(error, "returncode", None)
        detail = "timed out" if code is None else f"exit {code}"
        raise ArtifactError(
            f"packaged artifact command failed ({detail}): {command!r}; "
            f"stdout={stdout[-1000:]!r}; stderr={stderr[-1000:]!r}"
        ) from error


def assert_same_tree(source: Path, packaged: Path, label: str) -> None:
    if not source.is_dir() or not packaged.is_dir():
        raise ArtifactError(f"{label} tree is missing: {source} -> {packaged}")
    if tree_entries(source) != tree_entries(packaged):
        raise ArtifactError(
            f"packaged {label} tree does not preserve files, symlinks and modes"
        )


def assert_same_file(source: Path, packaged: Path) -> None:
    if not packaged.is_file():
        raise ArtifactError(f"packaged resource is missing: {packaged}")
    if source.stat().st_size != packaged.stat().st_size or sha256_file(source) != sha256_file(
        packaged
    ):
        raise ArtifactError(
            f"packaged resource does not match source: {source} -> {packaged}"
        )


def verify_packaged_resources(
    sidecars_root: Path, runtime_dir: Path, runtime_manifest: dict[str, Any]
) -> dict[str, Any]:
    packaged_pmd_root = sidecars_root / "pymobiledevice3"
    packaged_runtime = packaged_pmd_root / "runtime"
    packaged_entrypoint = packaged_runtime / runtime_manifest["entrypoint"]
    packaged_signer = sidecars_root / "signer" / "riviu_signer.py"
    packaged_wda_root = sidecars_root / "wda"
    packaged_build_install = packaged_wda_root / "build_and_install.py"

    assert_same_file(SIDECAR_SOURCE, packaged_pmd_root / "riviu_pmd.py")
    assert_same_file(
        SIDECAR_REQUIREMENTS,
        packaged_pmd_root / "requirements.txt",
    )
    assert_same_file(
        SIDECAR_REQUIREMENTS_LOCK,
        packaged_pmd_root / "requirements-lock.txt",
    )
    assert_same_file(SIGNER_SOURCE, packaged_signer)
    assert_same_file(BUILD_INSTALL_SOURCE, packaged_build_install)
    assert_same_file(
        LEGACY_WDA_SOURCE_LOCK,
        packaged_wda_root / "legacy-wda-source-lock.json",
    )
    assert_same_file(BRANDING_LOGO, packaged_wda_root / "logo.jpg")
    assert_same_tree(
        LEGACY_WDA_SOURCE,
        packaged_wda_root / "WebDriverAgent",
        "pinned WebDriverAgent source",
    )
    assert_same_tree(
        LEGACY_WDA_ICONSET,
        packaged_wda_root / "AppIcon.appiconset",
        "WDA icon set",
    )
    for source in production_paths():
        assert_same_file(source, sidecars_root / "wda" / source.name)

    assert_same_tree(runtime_dir, packaged_runtime, "frozen runtime")

    ping = run_checked([str(packaged_entrypoint), "ping"], timeout=90)
    try:
        ping_payload = json.loads(ping.stdout.strip().splitlines()[-1])
    except (IndexError, json.JSONDecodeError) as error:
        raise ArtifactError(
            f"packaged sidecar ping returned invalid JSON: {ping.stdout!r}"
        ) from error
    if not (
        ping_payload.get("ok") is True
        and ping_payload.get("pymobiledevice3") is True
        and ping_payload.get("sidecarProtocolVersion") == 2
        and "verifiedProcessControl" in ping_payload.get("contracts", [])
    ):
        raise ArtifactError(f"packaged sidecar ping contract failed: {ping_payload!r}")

    tidevice = run_checked(
        [str(packaged_entrypoint), "__tidevice", "--version"], timeout=90
    )
    expected_tidevice = runtime_manifest.get("dependencies", {}).get("tidevice")
    if tidevice.stdout.strip() != expected_tidevice:
        raise ArtifactError(
            "packaged tidevice version mismatch: "
            f"expected {expected_tidevice!r}, got {tidevice.stdout.strip()!r}"
        )

    signer = run_checked(
        [str(packaged_entrypoint), "__script", str(packaged_signer), "--help"],
        timeout=90,
    )
    if "sign-install-wda" not in signer.stdout:
        raise ArtifactError("packaged signer command smoke test failed")

    signer_error_json = "NOT_APPLICABLE"
    if sys.platform == "win32":
        signer_failure = subprocess.run(
            [
                str(packaged_entrypoint),
                "__script",
                str(packaged_signer),
                "sign-install-wda",
                "--udid",
                "FIXTURE_ONLY",
                "--apple-id",
                "fixture@example.test",
                "--password",
                "FIXTURE_ONLY",
            ],
            cwd=REPOSITORY_ROOT,
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
        try:
            error_payload = json.loads(error_lines[-1])
        except (IndexError, json.JSONDecodeError) as error:
            raise ArtifactError(
                "packaged signer error path returned invalid JSON"
            ) from error
        if not (
            signer_failure.returncode == 1
            and error_payload.get("ok") is False
            and "Traceback" not in signer_failure.stderr
        ):
            raise ArtifactError("packaged signer error path was not structured")
        signer_error_json = "PASS"

    signing_resources = run_checked(
        [
            str(packaged_entrypoint),
            "__script",
            str(packaged_build_install),
            "--self-test",
        ],
        timeout=90,
    )
    try:
        signing_payload = json.loads(signing_resources.stdout.strip().splitlines()[-1])
    except (IndexError, json.JSONDecodeError) as error:
        raise ArtifactError(
            "packaged signing resource self-test returned invalid JSON"
        ) from error
    expected_signing = runtime_manifest.get("signingResources")
    if not isinstance(expected_signing, dict):
        raise ArtifactError("runtime signing resource attestation is missing")
    if not (
        signing_payload.get("ok") is True
        and signing_payload.get("kind") == "packagedSigningResources"
        and signing_payload.get("sourceVersion")
        == expected_signing.get("sourceVersion")
        and signing_payload.get("sourceTreeSha256")
        == expected_signing.get("sourceTreeSha256")
        and signing_payload.get("logoSha256") == expected_signing.get("logoSha256")
        and signing_payload.get("iconSetTreeSha256")
        == expected_signing.get("iconSetTreeSha256")
        and signing_payload.get("workspaceOutsideResources") is True
    ):
        raise ArtifactError(
            f"packaged signing resource contract failed: {signing_payload!r}"
        )

    return {
        "resourceTree": "PASS",
        "ping": "PASS",
        "embeddedTidevice": "PASS",
        "embeddedSigner": "PASS",
        "embeddedSigningResources": "PASS",
        "embeddedSignerErrorJson": signer_error_json,
        "runtimeTreeSha256": runtime_manifest["treeSha256"],
    }


def verify_windows_desktop_executable(
    install_root: Path, target: str
) -> dict[str, Any]:
    try:
        cargo_document = tomllib.loads(
            TAURI_CARGO_MANIFEST.read_text(encoding="utf-8")
        )
        binary_name = cargo_document["package"]["name"]
    except (OSError, UnicodeDecodeError, tomllib.TOMLDecodeError, KeyError) as error:
        raise ArtifactError(f"invalid desktop Cargo binary name: {error}") from error
    if (
        not isinstance(binary_name, str)
        or not binary_name
        or Path(binary_name).name != binary_name
    ):
        raise ArtifactError("desktop Cargo package name must be a portable string")
    expected_name = f"{binary_name}.exe".casefold()
    candidates = sorted(
        path
        for path in install_root.rglob("*.exe")
        if path.is_file() and path.name.casefold() == expected_name
    )
    if len(candidates) != 1:
        raise ArtifactError(
            f"installed package must contain exactly one {binary_name}.exe, "
            f"found {len(candidates)}"
        )
    executable = candidates[0]
    with executable.open("rb") as handle:
        dos_header = handle.read(64)
        if len(dos_header) != 64 or dos_header[:2] != b"MZ":
            raise ArtifactError(f"desktop executable has an invalid DOS header: {executable}")
        pe_offset = int.from_bytes(dos_header[0x3C:0x40], "little")
        if pe_offset < 64 or pe_offset > 16 * 1024 * 1024:
            raise ArtifactError(f"desktop executable has an invalid PE offset: {executable}")
        handle.seek(pe_offset)
        pe_header = handle.read(6)
    if len(pe_header) != 6 or pe_header[:4] != b"PE\0\0":
        raise ArtifactError(f"desktop executable has an invalid PE signature: {executable}")
    machine = int.from_bytes(pe_header[4:6], "little")
    expected_machines = {"x86_64-pc-windows-msvc": (0x8664, "x86_64")}
    expected_machine, architecture = expected_machines[target]
    if machine != expected_machine:
        raise ArtifactError(
            f"desktop executable machine mismatch: expected 0x{expected_machine:04x}, "
            f"got 0x{machine:04x}"
        )
    return {
        "name": executable.name,
        "bytes": executable.stat().st_size,
        "sha256": sha256_file(executable),
        "architecture": architecture,
    }


def verify_windows_package(
    installers: list[Path], bundle_dir: Path, runtime_dir: Path, runtime: dict[str, Any]
) -> dict[str, Any]:
    msi_installers = [path for path in installers if path.suffix.lower() == ".msi"]
    nsis_installers = [path for path in installers if path.suffix.lower() == ".exe"]
    if len(msi_installers) != 1:
        raise ArtifactError(f"expected exactly one MSI installer, found {len(msi_installers)}")
    if len(nsis_installers) != 1:
        raise ArtifactError(
            f"expected exactly one NSIS installer, found {len(nsis_installers)}"
        )

    bundle_dir.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(
        prefix="desktop-msi-extract-", dir=bundle_dir.parent
    ) as temporary:
        extract_root = Path(temporary).resolve()
        # msiexec prints nothing; /L*v is the only way to learn why it refused.
        msi_log = extract_root / "msiexec-administrative.log"
        command = [
            "msiexec.exe",
            "/a",
            str(msi_installers[0]),
            "/qn",
            f"TARGETDIR={extract_root}",
            "/L*v",
            str(msi_log),
        ]
        try:
            run_checked(command, timeout=300)
        except ArtifactError as error:
            raise ArtifactError(
                f"{error}; msiexec log tail={read_log_tail(msi_log)!r}"
            ) from error
        sidecars_roots = [
            path
            for path in extract_root.rglob("sidecars")
            if (path / "pymobiledevice3" / "riviu_pmd.py").is_file()
        ]
        if len(sidecars_roots) != 1:
            raise ArtifactError(
                "administratively extracted MSI must contain exactly one sidecars resource root"
            )
        evidence = verify_packaged_resources(sidecars_roots[0], runtime_dir, runtime)
        msi_desktop = verify_windows_desktop_executable(
            extract_root, "x86_64-pc-windows-msvc"
        )
    evidence["msiAdministrativeExtract"] = "PASS"

    with tempfile.TemporaryDirectory(
        prefix="desktop-nsis-install-", dir=bundle_dir.parent
    ) as temporary:
        install_root = (Path(temporary) / "installed").resolve()
        run_checked(
            [str(nsis_installers[0]), "/S", f"/D={install_root}"], timeout=300
        )
        sidecars_roots = [
            path
            for path in install_root.rglob("sidecars")
            if (path / "pymobiledevice3" / "riviu_pmd.py").is_file()
        ]
        if len(sidecars_roots) != 1:
            raise ArtifactError(
                "silently installed NSIS must contain exactly one sidecars resource root"
            )
        nsis_evidence = verify_packaged_resources(
            sidecars_roots[0], runtime_dir, runtime
        )
        nsis_desktop = verify_windows_desktop_executable(
            install_root, "x86_64-pc-windows-msvc"
        )
        if nsis_evidence != {
            key: evidence[key]
            for key in nsis_evidence
        }:
            raise ArtifactError("MSI and NSIS packaged resource evidence diverged")
        for field in ("name", "architecture"):
            if nsis_desktop[field] != msi_desktop[field]:
                raise ArtifactError(
                    f"MSI and NSIS desktop executable {field} values diverged"
                )
        uninstallers = sorted(install_root.rglob("uninstall*.exe"))
        if len(uninstallers) != 1:
            raise ArtifactError("NSIS silent install did not produce one uninstaller")
        run_checked([str(uninstallers[0]), "/S"], timeout=300)
    evidence["nsisSilentInstall"] = "PASS"
    evidence["desktopExecutable"] = "PASS"
    evidence["desktopArchitecture"] = msi_desktop["architecture"]
    evidence["desktopExecutableBytes"] = {
        "msi": msi_desktop["bytes"],
        "nsis": nsis_desktop["bytes"],
    }
    evidence["desktopExecutableSha256"] = {
        "msi": msi_desktop["sha256"],
        "nsis": nsis_desktop["sha256"],
    }
    return evidence


@dataclass(frozen=True)
class DmgAttachment:
    image_path: Path
    dev_entry: str


def parse_dmg_attachments(
    payload: object, image_path: Path
) -> set[DmgAttachment]:
    if not isinstance(payload, dict):
        raise ArtifactError("hdiutil info plist root must be a dictionary")
    images = payload.get("images", [])
    if not isinstance(images, list):
        raise ArtifactError("hdiutil info plist images must be an array")

    canonical_image_path = image_path.resolve()
    attachments: set[DmgAttachment] = set()
    for image in images:
        if not isinstance(image, dict):
            continue
        raw_image_path = image.get("image-path")
        if (
            not isinstance(raw_image_path, str)
            or Path(raw_image_path).resolve() != canonical_image_path
        ):
            continue
        entities = image.get("system-entities", [])
        if not isinstance(entities, list):
            raise ArtifactError(
                f"hdiutil info has invalid system entities for {canonical_image_path}"
            )
        dev_entry = next(
            (
                entity["dev-entry"]
                for entity in entities
                if isinstance(entity, dict)
                and isinstance(entity.get("dev-entry"), str)
            ),
            None,
        )
        if dev_entry is None:
            raise ArtifactError(
                f"hdiutil info has no dev-entry for attached image "
                f"{canonical_image_path}"
            )
        attachments.add(DmgAttachment(canonical_image_path, dev_entry))
    return attachments


def inspect_dmg_attachments(image_path: Path) -> set[DmgAttachment]:
    try:
        info = subprocess.run(
            ["hdiutil", "info", "-plist"],
            cwd=REPOSITORY_ROOT,
            capture_output=True,
            text=True,
            timeout=30,
        )
    except (OSError, subprocess.TimeoutExpired) as error:
        raise ArtifactError(f"failed to inspect mounted DMGs: {error}") from error
    if info.returncode != 0:
        raise ArtifactError(
            f"failed to inspect mounted DMGs: {info.stderr[-1000:]!r}"
        )
    try:
        payload = plistlib.loads(info.stdout.encode("utf-8"))
    except plistlib.InvalidFileException as error:
        raise ArtifactError("hdiutil info returned an invalid plist") from error
    return parse_dmg_attachments(payload, image_path)


def parse_dmg_attach_result(
    stdout: str, image_path: Path
) -> tuple[DmgAttachment, list[Path]]:
    try:
        payload = plistlib.loads(stdout.encode("utf-8"))
    except plistlib.InvalidFileException as error:
        raise ArtifactError("hdiutil returned an invalid attachment plist") from error
    if not isinstance(payload, dict):
        raise ArtifactError("hdiutil attachment plist root must be a dictionary")
    entities = payload.get("system-entities", [])
    if not isinstance(entities, list):
        raise ArtifactError("hdiutil attachment system entities must be an array")
    dev_entry = next(
        (
            entity["dev-entry"]
            for entity in entities
            if isinstance(entity, dict)
            and isinstance(entity.get("dev-entry"), str)
        ),
        None,
    )
    if dev_entry is None:
        raise ArtifactError("hdiutil attachment plist has no dev-entry")
    mount_points = [
        Path(entity["mount-point"]).resolve()
        for entity in entities
        if isinstance(entity, dict) and isinstance(entity.get("mount-point"), str)
    ]
    return DmgAttachment(image_path.resolve(), dev_entry), mount_points


def detach_dmg_with_retry(attachment: DmgAttachment) -> ArtifactError | None:
    last_failure = "unknown detach failure"
    for attempt in range(3):
        command = ["hdiutil", "detach"]
        if attempt == 2:
            command.append("-force")
        command.append(attachment.dev_entry)
        try:
            detached = subprocess.run(
                command,
                cwd=REPOSITORY_ROOT,
                capture_output=True,
                text=True,
                timeout=120,
            )
            if detached.returncode == 0:
                last_failure = "detach returned success but attachment remains"
            else:
                last_failure = detached.stderr[-1000:]
        except (OSError, subprocess.TimeoutExpired) as error:
            last_failure = str(error)
        try:
            if attachment not in inspect_dmg_attachments(attachment.image_path):
                return None
        except ArtifactError as error:
            last_failure = (
                f"{last_failure}; attachment state re-query failed: {error}"
            )
        if attempt < 2:
            time.sleep(1)
    return ArtifactError(f"failed to detach DMG after 3 attempts: {last_failure!r}")


def verify_macos_package(
    bundle_dir: Path, target: str, runtime_dir: Path, runtime: dict[str, Any]
) -> dict[str, Any]:
    dmg_installers = [path for path in find_installers(bundle_dir, target) if path.suffix == ".dmg"]
    if len(dmg_installers) != 1:
        raise ArtifactError(f"expected exactly one DMG installer, found {len(dmg_installers)}")
    mount_point = Path(
        tempfile.mkdtemp(prefix="desktop-dmg-mount-", dir=bundle_dir.parent)
    ).resolve()
    attach_attempted = False
    baseline_attachments: set[DmgAttachment] = set()
    owned_attachments: set[DmgAttachment] = set()
    evidence: dict[str, Any] | None = None
    primary_error: BaseException | None = None
    cleanup_error: ArtifactError | None = None
    try:
        baseline_attachments = inspect_dmg_attachments(dmg_installers[0])
        attach_attempted = True
        attachment = run_checked(
            [
                "hdiutil",
                "attach",
                "-readonly",
                "-nobrowse",
                "-mountpoint",
                str(mount_point),
                "-plist",
                str(dmg_installers[0]),
            ],
            timeout=300,
        )
        owned_attachment, mount_points = parse_dmg_attach_result(
            attachment.stdout, dmg_installers[0]
        )
        if owned_attachment not in baseline_attachments:
            owned_attachments.add(owned_attachment)
        if mount_points != [mount_point]:
            raise ArtifactError(
                "DMG did not use the exact isolated mount point: "
                f"expected {[mount_point]!r}, got {mount_points!r}"
            )
        app_bundles = sorted(path for path in mount_point.rglob("*.app") if path.is_dir())
        if len(app_bundles) != 1:
            raise ArtifactError(
                f"mounted DMG must contain exactly one app bundle, found {len(app_bundles)}"
            )
        app = app_bundles[0]
        resources_root = app / "Contents" / "Resources"
        sidecars_root = resources_root / "sidecars"
        evidence = verify_packaged_resources(sidecars_root, runtime_dir, runtime)

        info_path = app / "Contents" / "Info.plist"
        try:
            info = plistlib.loads(info_path.read_bytes())
        except (OSError, plistlib.InvalidFileException) as error:
            raise ArtifactError(f"invalid packaged Info.plist: {info_path}") from error
        executable_name = info.get("CFBundleExecutable")
        if not isinstance(executable_name, str) or not executable_name:
            raise ArtifactError("packaged Info.plist has no CFBundleExecutable")
        app_executable = app / "Contents" / "MacOS" / executable_name
        sidecar_executable = (
            sidecars_root / "pymobiledevice3" / "runtime" / runtime["entrypoint"]
        )
        expected_architecture = (
            "arm64" if target.startswith("aarch64-") else "x86_64"
        )
        for executable in (app_executable, sidecar_executable):
            architectures = run_checked(
                ["lipo", "-archs", str(executable)]
            ).stdout.split()
            if architectures != [expected_architecture]:
                raise ArtifactError(
                    f"packaged executable architecture mismatch for {executable}: "
                    f"expected {[expected_architecture]!r}, got {architectures!r}"
                )
        run_checked(["codesign", "--verify", "--deep", "--strict", str(app)])
        evidence["appArchitecture"] = expected_architecture
        evidence["sidecarArchitecture"] = expected_architecture
        evidence["codeSignature"] = "PASS"
        evidence["dmgMountedReadOnly"] = "PASS"
    except BaseException as error:
        primary_error = error
    finally:
        cleanup_failures: list[ArtifactError] = []
        current_attachments: set[DmgAttachment] | None = None
        if attach_attempted:
            try:
                current_attachments = inspect_dmg_attachments(dmg_installers[0])
                owned_attachments.update(
                    current_attachments - baseline_attachments
                )
            except ArtifactError as error:
                cleanup_failures.append(error)
        for owned_attachment in sorted(
            owned_attachments, key=lambda item: item.dev_entry
        ):
            if (
                current_attachments is not None
                and owned_attachment not in current_attachments
            ):
                continue
            detach_error = detach_dmg_with_retry(owned_attachment)
            if detach_error is not None:
                cleanup_failures.append(detach_error)
        if len(cleanup_failures) == 1:
            cleanup_error = cleanup_failures[0]
        elif cleanup_failures:
            cleanup_error = ArtifactError(
                "DMG cleanup failed: "
                + "; ".join(str(error) for error in cleanup_failures)
            )
        if cleanup_error is None:
            shutil.rmtree(mount_point, ignore_errors=True)

    if primary_error is not None:
        if cleanup_error is not None:
            raise primary_error.with_traceback(primary_error.__traceback__) from cleanup_error
        raise primary_error.with_traceback(primary_error.__traceback__)
    if cleanup_error is not None:
        raise cleanup_error
    if evidence is None:
        raise ArtifactError("DMG verification produced no evidence")
    return evidence


def verify_packaged_bundle(
    installers: list[Path],
    bundle_dir: Path,
    target: str,
    runtime_dir: Path,
    runtime: dict[str, Any],
) -> dict[str, Any]:
    if target == "x86_64-pc-windows-msvc":
        return verify_windows_package(installers, bundle_dir, runtime_dir, runtime)
    return verify_macos_package(bundle_dir, target, runtime_dir, runtime)


def copy_release_file(source: Path, output_dir: Path, name: str) -> dict[str, Any]:
    destination = output_dir / name
    if destination.exists():
        raise ArtifactError(f"duplicate release artifact name: {destination.name}")
    shutil.copy2(source, destination)
    return {
        "file": destination.name,
        "sourceName": source.name,
        "bytes": destination.stat().st_size,
        "sha256": sha256_file(destination),
    }


def validate_source_commit(value: str) -> str:
    normalized = value.strip().lower()
    if len(normalized) != 40 or any(
        character not in "0123456789abcdef" for character in normalized
    ):
        raise ArtifactError("source commit must be an exact 40-character Git SHA")
    return normalized


def collect_command(args: argparse.Namespace) -> dict[str, Any]:
    target = args.target
    source_commit = validate_source_commit(args.source_commit)
    requested_runtime_dir = args.runtime.resolve()
    bundle_dir = args.bundle_dir.resolve()
    staged_runtime_dir = (
        bundle_dir.parent / "sidecars" / "pymobiledevice3" / "runtime"
    )
    runtime_dir = requested_runtime_dir
    runtime_was_staged = False
    if not (runtime_dir / "runtime-manifest.json").is_file():
        runtime_dir = staged_runtime_dir
        runtime_was_staged = True
    overlay_path = args.tauri_config.resolve()
    output_dir = args.output.resolve()
    production = verify_production_snapshot(args.production_baseline.resolve())
    runtime = verify_runtime(runtime_dir, target)
    verify_overlay(overlay_path, requested_runtime_dir, target)
    installers = find_installers(bundle_dir, target)
    packaged = verify_packaged_bundle(
        installers, bundle_dir, target, runtime_dir, runtime
    )

    if output_dir.exists():
        shutil.rmtree(output_dir)
    output_dir.mkdir(parents=True)

    copied_installers = []
    for installer in installers:
        copied_installers.append(
            copy_release_file(installer, output_dir, f"{args.label}--{installer.name}")
        )

    runtime_manifest_name = f"{args.label}-runtime-manifest.json"
    overlay_name = f"{args.label}-tauri-overlay.json"
    copy_release_file(
        runtime_dir / "runtime-manifest.json", output_dir, runtime_manifest_name
    )
    copy_release_file(overlay_path, output_dir, overlay_name)

    artifact_manifest_name = f"{args.label}-artifact-manifest.json"
    artifact_manifest = {
        "schemaVersion": 2,
        "label": args.label,
        "target": target,
        "sourceCommit": source_commit,
        "runtime": {
            "manifest": runtime_manifest_name,
            "platform": runtime["platform"],
            "architecture": runtime["architecture"],
            "pythonVersion": runtime["pythonVersion"],
            "treeSha256": runtime["treeSha256"],
            "tauriStagedSource": runtime_was_staged,
        },
        "tauriOverlay": {
            "file": overlay_name,
            "sha256": sha256_file(output_dir / overlay_name),
            "resourceDestination": RUNTIME_RESOURCE_DESTINATION,
        },
        "productionInputs": production["files"],
        "packagedVerification": packaged,
        "installers": copied_installers,
    }
    write_json(output_dir / artifact_manifest_name, artifact_manifest)

    checksum_name = f"{args.label}-SHA256SUMS"
    checksum_files = sorted(
        path for path in output_dir.iterdir() if path.is_file() and path.name != checksum_name
    )
    checksum_text = "".join(
        f"{sha256_file(path)}  {path.name}\n" for path in checksum_files
    )
    (output_dir / checksum_name).write_text(
        checksum_text, encoding="ascii", newline="\n"
    )
    return {
        "ok": True,
        "command": "collect",
        "label": args.label,
        "target": target,
        "output": str(output_dir),
        "installerCount": len(copied_installers),
    }


def parse_checksum_file(path: Path) -> list[tuple[str, str]]:
    entries: list[tuple[str, str]] = []
    for line_number, raw_line in enumerate(
        path.read_text(encoding="ascii").splitlines(), start=1
    ):
        digest, separator, name = raw_line.partition("  ")
        if (
            not separator
            or len(digest) != 64
            or any(character not in "0123456789abcdef" for character in digest)
            or not name
            or Path(name).name != name
        ):
            raise ArtifactError(f"invalid checksum line {line_number} in {path}")
        entries.append((digest, name))
    if not entries:
        raise ArtifactError(f"checksum file is empty: {path}")
    return entries


def verify_release_command(args: argparse.Namespace) -> dict[str, Any]:
    root = args.root.resolve()
    source_commit = validate_source_commit(args.source_commit)
    if not root.is_dir():
        raise ArtifactError(f"downloaded artifact directory is missing: {root}")

    all_files = [path for path in root.rglob("*") if path.is_file()]
    basenames: dict[str, Path] = {}
    for path in all_files:
        previous = basenames.get(path.name)
        if previous is not None:
            raise ArtifactError(
                f"release asset basename collision: {previous} and {path}"
            )
        basenames[path.name] = path

    verified_files = 0
    covered_files: set[Path] = set()
    for label in args.labels:
        expected_target = RELEASE_LABEL_TARGETS.get(label)
        if expected_target is None:
            raise ArtifactError(f"unknown desktop release label: {label!r}")
        checksum_matches = list(root.rglob(f"{label}-SHA256SUMS"))
        manifest_matches = list(root.rglob(f"{label}-artifact-manifest.json"))
        if len(checksum_matches) != 1 or len(manifest_matches) != 1:
            raise ArtifactError(
                f"expected one checksum and artifact manifest for {label!r}"
            )
        checksum_path = checksum_matches[0]
        manifest_path = manifest_matches[0]
        manifest = load_json(manifest_path)
        if manifest.get("schemaVersion") != 2:
            raise ArtifactError(f"artifact manifest schema mismatch in {manifest_path}")
        if manifest.get("sourceCommit") != source_commit:
            raise ArtifactError(
                f"artifact source commit mismatch in {manifest_path}: "
                f"expected {source_commit}, got {manifest.get('sourceCommit')!r}"
            )
        if manifest.get("label") != label:
            raise ArtifactError(f"artifact manifest label mismatch in {manifest_path}")
        if manifest.get("target") != expected_target:
            raise ArtifactError(f"artifact manifest target mismatch in {manifest_path}")
        packaged = manifest.get("packagedVerification")
        common_package_gates = (
            "resourceTree",
            "ping",
            "embeddedTidevice",
            "embeddedSigner",
            "embeddedSigningResources",
        )
        if not isinstance(packaged, dict) or any(
            packaged.get(gate) != "PASS" for gate in common_package_gates
        ):
            raise ArtifactError(
                f"artifact manifest lacks packaged resource verification: {manifest_path}"
            )
        if expected_target == "x86_64-pc-windows-msvc":
            if (
                packaged.get("msiAdministrativeExtract") != "PASS"
                or packaged.get("nsisSilentInstall") != "PASS"
                or packaged.get("embeddedSignerErrorJson") != "PASS"
                or packaged.get("desktopExecutable") != "PASS"
                or packaged.get("desktopArchitecture") != "x86_64"
            ):
                raise ArtifactError(
                    f"Windows installer verification gate is missing in {manifest_path}"
                )
        else:
            expected_architecture = (
                "arm64" if expected_target.startswith("aarch64-") else "x86_64"
            )
            if (
                packaged.get("codeSignature") != "PASS"
                or packaged.get("dmgMountedReadOnly") != "PASS"
                or packaged.get("appArchitecture") != expected_architecture
                or packaged.get("sidecarArchitecture") != expected_architecture
            ):
                raise ArtifactError(
                    f"macOS signature or architecture gate is missing in {manifest_path}"
                )
        installers = manifest.get("installers")
        if not isinstance(installers, list) or not installers:
            raise ArtifactError(f"artifact manifest has no installers: {manifest_path}")

        entries = parse_checksum_file(checksum_path)
        checksummed_names = {name for _, name in entries}
        if len(checksummed_names) != len(entries):
            raise ArtifactError(f"duplicate checksum filename in {checksum_path}")
        sibling_names = {
            path.name
            for path in checksum_path.parent.iterdir()
            if path.is_file() and path != checksum_path
        }
        if checksummed_names != sibling_names:
            raise ArtifactError(
                f"checksum coverage mismatch in {checksum_path}: "
                f"expected {sorted(sibling_names)!r}, got {sorted(checksummed_names)!r}"
            )
        for expected_digest, name in entries:
            candidate = checksum_path.parent / name
            if not candidate.is_file():
                raise ArtifactError(f"checksummed release asset is missing: {candidate}")
            if sha256_file(candidate) != expected_digest:
                raise ArtifactError(f"release asset checksum mismatch: {candidate}")
            covered_files.add(candidate.resolve())
            verified_files += 1
        covered_files.add(checksum_path.resolve())

        installer_names: set[str] = set()
        for installer in installers:
            if not isinstance(installer, dict):
                raise ArtifactError(f"invalid installer record in {manifest_path}")
            name = installer.get("file")
            if not isinstance(name, str) or Path(name).name != name:
                raise ArtifactError(f"invalid installer filename in {manifest_path}")
            candidate = checksum_path.parent / name
            if (
                name not in checksummed_names
                or not candidate.is_file()
                or installer.get("bytes") != candidate.stat().st_size
                or installer.get("sha256") != sha256_file(candidate)
            ):
                raise ArtifactError(
                    f"installer record does not match release asset: {candidate}"
                )
            installer_names.add(name)
        expected_suffixes = set(TARGETS[expected_target]["installer_suffixes"])
        actual_suffixes = {Path(name).suffix.lower() for name in installer_names}
        if actual_suffixes != expected_suffixes:
            raise ArtifactError(
                f"installer types mismatch for {label}: expected "
                f"{sorted(expected_suffixes)!r}, got {sorted(actual_suffixes)!r}"
            )

    all_resolved = {path.resolve() for path in all_files}
    if covered_files != all_resolved:
        unexpected = sorted(str(path) for path in all_resolved - covered_files)
        missing = sorted(str(path) for path in covered_files - all_resolved)
        raise ArtifactError(
            f"release artifact set coverage mismatch: unexpected={unexpected!r}, "
            f"missing={missing!r}"
        )

    return {
        "ok": True,
        "command": "verify-release",
        "labels": args.labels,
        "sourceCommit": source_commit,
        "assetCount": len(all_files),
        "verifiedFiles": verified_files,
    }


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    subparsers = parser.add_subparsers(dest="command", required=True)

    snapshot = subparsers.add_parser("snapshot-production")
    snapshot.add_argument("--output", type=Path, required=True)
    snapshot.add_argument("--require-canonical-production", action="store_true")
    snapshot.set_defaults(handler=snapshot_command)

    verify_version = subparsers.add_parser("verify-version")
    verify_version.add_argument("--tag")
    verify_version.set_defaults(handler=verify_version_command)

    collect = subparsers.add_parser("collect")
    collect.add_argument("--target", choices=sorted(TARGETS), required=True)
    collect.add_argument("--label", required=True)
    collect.add_argument("--bundle-dir", type=Path, required=True)
    collect.add_argument("--runtime", type=Path, required=True)
    collect.add_argument("--tauri-config", type=Path, required=True)
    collect.add_argument("--production-baseline", type=Path, required=True)
    collect.add_argument("--output", type=Path, required=True)
    collect.add_argument("--source-commit", required=True)
    collect.set_defaults(handler=collect_command)

    verify_release = subparsers.add_parser("verify-release")
    verify_release.add_argument("--root", type=Path, required=True)
    verify_release.add_argument("--labels", nargs="+", required=True)
    verify_release.add_argument("--source-commit", required=True)
    verify_release.set_defaults(handler=verify_release_command)
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    try:
        result = args.handler(args)
    except (ArtifactError, OSError) as error:
        print(json.dumps({"ok": False, "error": str(error)}, sort_keys=True), file=sys.stderr)
        return 1
    print(json.dumps(result, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
