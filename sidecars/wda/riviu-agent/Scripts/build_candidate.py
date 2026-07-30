#!/usr/bin/env python3
"""Prepare, build, inspect, and package a Riviu Agent candidate on macOS."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import plistlib
import re
import shutil
import stat
import subprocess
import sys
import tempfile
import zipfile
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Callable, Iterable


AGENT_ROOT = Path(__file__).resolve().parents[1]
REPO_ROOT = AGENT_ROOT.parents[2]
PREPARE_SCRIPT = AGENT_ROOT / "Scripts" / "prepare.py"
DEFAULT_LOCK = AGENT_ROOT / "baseline-lock.json"
DEFAULT_ARCHIVE = (
    REPO_ROOT / "target" / "rtmmo-re" / "baselines" / "appium-webdriveragent-15.1.4.tgz"
)
DEFAULT_SOURCE = REPO_ROOT / "target" / "riviu-agent" / "source"
DEFAULT_DERIVED_DATA = REPO_ROOT / "target" / "riviu-agent" / "derived-data"
DEFAULT_ARTIFACTS_ROOT = REPO_ROOT / "target" / "riviu-agent" / "artifacts"
DEFAULT_XCCONFIG = AGENT_ROOT / "Config" / "RiviuAgent.xcconfig"

ARTIFACT_ID = "riviu-agent-ios-candidate"
DEFAULT_ARTIFACT_VERSION = "0.1.0"
GATE_STATUS = "PENDING_MAC_DEVICE"
CANDIDATE_RUNNER_BUNDLE_ID = "com.riviu.managersphone.agent.xctrunner"
ATTESTATION_BUNDLE = "PlugIns/WebDriverAgentRunner.xctest"
PROTOCOL_VERSION = 2
FEATURES = ("stream", "tap", "swipe", "clipboard")
CONTROL_PORT = 8916
MJPEG_PORT = 9094
LOGICAL_WIDTH = 375
LOGICAL_HEIGHT = 667
XCODE26_RUNTIME_CLOSURE = (
    "Testing.framework/Testing",
    "_Testing_Foundation.framework/_Testing_Foundation",
    "lib_TestingInterop.dylib",
    "libXCTestSwiftSupport.dylib",
)
REQUIRED_TOOLS = ("xcodebuild", "security", "codesign", "xcrun")
ZIP_TIMESTAMP = (1980, 1, 1, 0, 0, 0)
COMMAND_TIMEOUT_SECONDS = 60
PREPARE_TIMEOUT_SECONDS = 300
XCODE_BUILD_TIMEOUT_SECONDS = 1800


class BuildError(RuntimeError):
    pass


@dataclass(frozen=True)
class XcodeVersion:
    version: str
    build: str


@dataclass(frozen=True)
class BundleIdentity:
    bundle_id: str
    bundle_version: str
    bundle_build: str
    executable: str
    attestation_bundle: str
    signature_identifier: str
    signer_identity: str
    signer_team_id: str
    source_sha256: str
    xcconfig_sha256: str
    protocol_version: int
    objective_c_unit_tests: str
    xcode: XcodeVersion


@dataclass(frozen=True)
class DerivedDataLayout:
    root: Path
    unit_tests: Path
    runner: Path


def parse_xcode_version(output: str) -> XcodeVersion:
    version_match = re.search(r"(?m)^Xcode\s+(\S+)\s*$", output)
    build_match = re.search(r"(?m)^Build version\s+(\S+)\s*$", output)
    if version_match is None or build_match is None:
        raise BuildError("could not parse complete Xcode version output")
    return XcodeVersion(version_match.group(1), build_match.group(1))


def _required_plist_string(info: dict[str, Any], key: str) -> str:
    value = info.get(key)
    if not isinstance(value, str) or not value.strip():
        raise BuildError(f"app Info.plist field {key} must be a nonblank string")
    return value.strip()


def _first_codesign_value(output: str, key: str) -> str:
    match = re.search(rf"(?m)^{re.escape(key)}=(.+?)\s*$", output)
    if match is None or not match.group(1).strip():
        raise BuildError(f"codesign output is missing {key}")
    return match.group(1).strip()


def capture_bundle_identity(
    app: Path,
    codesign_output: str,
    *,
    expected_source_sha256: str,
    expected_xcconfig_sha256: str,
    expected_xcode: XcodeVersion,
) -> BundleIdentity:
    app = Path(app)
    if not app.is_dir() or app.suffix != ".app":
        raise BuildError(f"candidate app does not exist: {app}")
    info_path = app / "Info.plist"
    try:
        with info_path.open("rb") as stream:
            info = plistlib.load(stream)
    except (OSError, plistlib.InvalidFileException) as exc:
        raise BuildError(f"failed to read candidate Info.plist: {info_path}") from exc
    if not isinstance(info, dict):
        raise BuildError("candidate Info.plist root must be a dictionary")

    bundle_id = _required_plist_string(info, "CFBundleIdentifier")
    bundle_version = _required_plist_string(info, "CFBundleShortVersionString")
    bundle_build = _required_plist_string(info, "CFBundleVersion")
    executable = _required_plist_string(info, "CFBundleExecutable")
    if bundle_id != CANDIDATE_RUNNER_BUNDLE_ID:
        raise BuildError(
            "candidate bundle identifier mismatch: "
            f"actual={bundle_id}, expected={CANDIDATE_RUNNER_BUNDLE_ID}"
        )
    if Path(executable).name != executable or "/" in executable or "\\" in executable:
        raise BuildError("candidate CFBundleExecutable must be a file name")
    if not (app / executable).is_file():
        raise BuildError(f"candidate executable is missing: {executable}")

    attestation_info_path = app / Path(ATTESTATION_BUNDLE) / "Info.plist"
    try:
        with attestation_info_path.open("rb") as stream:
            attestation = plistlib.load(stream)
    except (OSError, plistlib.InvalidFileException) as exc:
        raise BuildError(
            f"failed to read signed attestation Info.plist: {attestation_info_path}"
        ) from exc
    if not isinstance(attestation, dict):
        raise BuildError("signed attestation Info.plist root must be a dictionary")

    expected_source = _validate_sha256(expected_source_sha256, "prepared source digest")
    source_sha256 = _required_plist_string(attestation, "RiviuAgentSourceSHA256")
    if source_sha256 != _validate_sha256(source_sha256, "signed source digest"):
        raise BuildError("signed source digest must use canonical lowercase SHA-256 hex")
    if source_sha256 != expected_source:
        raise BuildError(
            "signed source SHA-256 mismatch: "
            f"actual={source_sha256}, expected={expected_source}"
        )

    expected_xcconfig = _validate_sha256(
        expected_xcconfig_sha256, "locked xcconfig digest"
    )
    xcconfig_sha256 = _required_plist_string(
        attestation, "RiviuAgentXcconfigSHA256"
    )
    if xcconfig_sha256 != _validate_sha256(
        xcconfig_sha256, "signed xcconfig digest"
    ):
        raise BuildError("signed xcconfig digest must use canonical lowercase SHA-256 hex")
    if xcconfig_sha256 != expected_xcconfig:
        raise BuildError(
            "signed xcconfig SHA-256 mismatch: "
            f"actual={xcconfig_sha256}, expected={expected_xcconfig}"
        )

    protocol_version = attestation.get("RiviuAgentProtocolVersion")
    if type(protocol_version) is not int or protocol_version != PROTOCOL_VERSION:
        raise BuildError(
            f"app Info.plist field RiviuAgentProtocolVersion must be integer {PROTOCOL_VERSION}"
        )
    objective_c_unit_tests = attestation.get("RiviuAgentObjectiveCUnitTests")
    if objective_c_unit_tests != "PASS":
        raise BuildError("app Info.plist field RiviuAgentObjectiveCUnitTests must be PASS")
    xcode = XcodeVersion(
        _required_plist_string(attestation, "RiviuAgentXcodeVersion"),
        _required_plist_string(attestation, "RiviuAgentXcodeBuild"),
    )
    if xcode != expected_xcode:
        raise BuildError(
            "signed Xcode mismatch: "
            f"actual={xcode.version}/{xcode.build}, "
            f"expected={expected_xcode.version}/{expected_xcode.build}"
        )

    signature_identifier = _first_codesign_value(codesign_output, "Identifier")
    signer_identity = _first_codesign_value(codesign_output, "Authority")
    signer_team_id = _first_codesign_value(codesign_output, "TeamIdentifier")
    if signature_identifier != bundle_id:
        raise BuildError(
            "bundle/signature identifier mismatch: "
            f"plist={bundle_id}, signature={signature_identifier}"
        )
    return BundleIdentity(
        bundle_id=bundle_id,
        bundle_version=bundle_version,
        bundle_build=bundle_build,
        executable=executable,
        attestation_bundle=ATTESTATION_BUNDLE,
        signature_identifier=signature_identifier,
        signer_identity=signer_identity,
        signer_team_id=signer_team_id,
        source_sha256=source_sha256,
        xcconfig_sha256=xcconfig_sha256,
        protocol_version=protocol_version,
        objective_c_unit_tests=objective_c_unit_tests,
        xcode=xcode,
    )


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    try:
        with Path(path).open("rb") as stream:
            while True:
                chunk = stream.read(1024 * 1024)
                if not chunk:
                    break
                digest.update(chunk)
    except OSError as exc:
        raise BuildError(f"failed to hash file: {path}") from exc
    return digest.hexdigest()


def source_documents_sha256(root: Path) -> str:
    documents: list[tuple[str, int, bytes]] = []
    try:
        for path in Path(root).rglob("*"):
            if path.is_file():
                raw_mode = stat.S_IMODE(path.stat().st_mode)
                normalized_mode = 0o755 if raw_mode & 0o111 else 0o644
                documents.append(
                    (
                        path.relative_to(root).as_posix(),
                        normalized_mode,
                        path.read_bytes(),
                    )
                )
    except OSError as exc:
        raise BuildError("prepared source tree could not be hashed") from exc
    documents.sort(key=lambda item: item[0])
    digest = hashlib.sha256()
    for relative, mode, contents in documents:
        path_bytes = relative.encode("utf-8")
        digest.update(len(path_bytes).to_bytes(8, "little"))
        digest.update(path_bytes)
        digest.update(mode.to_bytes(4, "little"))
        digest.update(len(contents).to_bytes(8, "little"))
        digest.update(hashlib.sha256(contents).digest())
    return digest.hexdigest()


def require_source_digest(root: Path, expected: str, phase: str) -> None:
    expected = _validate_sha256(expected, "prepared source digest")
    if source_documents_sha256(root) != expected:
        raise BuildError(f"prepared source tree changed during {phase}")


def require_xcconfig_digest(path: Path, expected: str, phase: str) -> None:
    expected = _validate_sha256(expected, "locked xcconfig digest")
    if sha256_file(path) != expected:
        raise BuildError(f"candidate xcconfig changed during {phase}")


def _reject_duplicate_json_keys(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
    result: dict[str, Any] = {}
    for key, value in pairs:
        if key in result:
            raise BuildError(f"baseline lock contains duplicate key: {key}")
        result[key] = value
    return result


def load_locked_xcconfig_sha256(lock_path: Path) -> str:
    try:
        lock = json.loads(
            Path(lock_path).read_text(encoding="utf-8"),
            object_pairs_hook=_reject_duplicate_json_keys,
        )
    except OSError as exc:
        raise BuildError(f"failed to read baseline lock: {lock_path}") from exc
    except json.JSONDecodeError as exc:
        raise BuildError("baseline lock is invalid JSON") from exc
    if not isinstance(lock, dict):
        raise BuildError("baseline lock root must be an object")
    value = lock.get("xcconfigSha256")
    if not isinstance(value, str):
        raise BuildError("baseline lock field xcconfigSha256 must be a SHA-256 digest")
    digest = _validate_sha256(value, "baseline lock field xcconfigSha256")
    if value != digest:
        raise BuildError("baseline lock field xcconfigSha256 must use lowercase SHA-256 hex")
    return digest


def safe_artifact_relative_path(path: Path, artifact_root: Path) -> str:
    resolved_root = Path(artifact_root).resolve(strict=False)
    resolved_path = Path(path).resolve(strict=False)
    try:
        relative = resolved_path.relative_to(resolved_root)
    except ValueError as exc:
        raise BuildError("IPA must be inside artifact root") from exc
    if not relative.parts or any(part in {"", ".", ".."} for part in relative.parts):
        raise BuildError("IPA path is not a safe relative artifact path")
    if relative.suffix.lower() != ".ipa":
        raise BuildError("candidate artifact must use the .ipa extension")
    return relative.as_posix()


def _validate_sha256(value: str, label: str) -> str:
    normalized = value.lower()
    if len(normalized) != 64 or any(character not in "0123456789abcdef" for character in normalized):
        raise BuildError(f"{label} must be a SHA-256 hex digest")
    return normalized


def generate_candidate_manifest(
    *,
    artifact_version: str,
    artifact_root: Path,
    ipa_path: Path,
    app_name: str,
    identity: BundleIdentity,
) -> dict[str, Any]:
    if not artifact_version.strip():
        raise BuildError("artifact version must be nonblank")
    if Path(app_name).name != app_name or not app_name.endswith(".app"):
        raise BuildError("payload app name must be a safe .app file name")
    ipa_path = Path(ipa_path)
    if not ipa_path.is_file():
        raise BuildError(f"candidate IPA does not exist: {ipa_path}")
    return {
        "schemaVersion": 1,
        "artifactId": ARTIFACT_ID,
        "artifactVersion": artifact_version,
        "gateStatus": GATE_STATUS,
        "bundleId": identity.bundle_id,
        "bundleVersion": identity.bundle_version,
        "bundleBuild": identity.bundle_build,
        "payloadApp": app_name,
        "executable": identity.executable,
        "attestationBundle": identity.attestation_bundle,
        "signatureIdentifier": identity.signature_identifier,
        "signerIdentity": identity.signer_identity,
        "signerTeamId": identity.signer_team_id,
        "protocolVersion": identity.protocol_version,
        "ipa": safe_artifact_relative_path(ipa_path, artifact_root),
        "sha256": sha256_file(ipa_path),
        "sourceSha256": identity.source_sha256,
        "xcconfigSha256": identity.xcconfig_sha256,
        "controlPort": CONTROL_PORT,
        "mjpegPort": MJPEG_PORT,
        "logicalWidth": LOGICAL_WIDTH,
        "logicalHeight": LOGICAL_HEIGHT,
        "features": list(FEATURES),
        "objectiveCUnitTests": identity.objective_c_unit_tests,
        "xcode": {"version": identity.xcode.version, "build": identity.xcode.build},
    }


def _tree_entries(app: Path) -> list[tuple[str, Path | None, str]]:
    prefix = f"Payload/{app.name}"
    entries: list[tuple[str, Path | None, str]] = [
        ("Payload/", None, "directory"),
        (f"{prefix}/", app, "directory"),
    ]

    def visit(directory: Path, archive_directory: str) -> None:
        for child in sorted(directory.iterdir(), key=lambda item: item.name):
            archive_name = f"{archive_directory}/{child.name}"
            if child.is_symlink():
                entries.append((archive_name, child, "symlink"))
            elif child.is_dir():
                entries.append((f"{archive_name}/", child, "directory"))
                visit(child, archive_name)
            elif child.is_file():
                entries.append((archive_name, child, "file"))
            else:
                raise BuildError(f"unsupported app bundle entry: {child}")

    visit(app, prefix)
    entries.sort(key=lambda item: item[0])
    names = [name for name, _path, _kind in entries]
    if len(names) != len(set(names)):
        raise BuildError("candidate package contains duplicate archive entries")
    return entries


def _zip_info(name: str, mode: int, kind: str) -> zipfile.ZipInfo:
    info = zipfile.ZipInfo(name, ZIP_TIMESTAMP)
    info.create_system = 3
    info.flag_bits |= 0x800
    if kind == "directory":
        unix_mode = stat.S_IFDIR | (mode or 0o755)
        info.external_attr = (unix_mode << 16) | 0x10
        info.compress_type = zipfile.ZIP_STORED
    elif kind == "symlink":
        unix_mode = stat.S_IFLNK | (mode or 0o777)
        info.external_attr = unix_mode << 16
        info.compress_type = zipfile.ZIP_STORED
    else:
        unix_mode = stat.S_IFREG | (mode or 0o644)
        info.external_attr = unix_mode << 16
        info.compress_type = zipfile.ZIP_DEFLATED
    return info


def package_candidate_ipa(app: Path, destination: Path) -> list[str]:
    app = Path(app)
    destination = Path(destination)
    if not app.is_dir() or app.suffix != ".app":
        raise BuildError(f"candidate app does not exist: {app}")
    if destination.suffix.lower() != ".ipa":
        raise BuildError("candidate artifact must use the .ipa extension")
    entries = _tree_entries(app)
    destination.parent.mkdir(parents=True, exist_ok=True)
    descriptor, temporary_name = tempfile.mkstemp(
        prefix=f".{destination.name}.", suffix=".tmp", dir=destination.parent
    )
    os.close(descriptor)
    temporary = Path(temporary_name)
    try:
        with zipfile.ZipFile(
            temporary, mode="w", compression=zipfile.ZIP_DEFLATED, compresslevel=9
        ) as archive:
            for name, path, kind in entries:
                mode = 0o755 if path is None else stat.S_IMODE(path.lstat().st_mode)
                info = _zip_info(name, mode, kind)
                if kind == "directory":
                    archive.writestr(info, b"")
                elif kind == "symlink":
                    assert path is not None
                    archive.writestr(info, os.readlink(path).encode("utf-8"))
                else:
                    assert path is not None
                    with path.open("rb") as source, archive.open(info, "w") as output:
                        shutil.copyfileobj(source, output, length=1024 * 1024)
        os.replace(temporary, destination)
    except BaseException:
        temporary.unlink(missing_ok=True)
        raise
    return [name for name, _path, _kind in entries]


def validate_build_requirements(
    *,
    team_id: str,
    udid: str,
    platform_name: str | None = None,
    tool_lookup: Callable[[str], str | None] = shutil.which,
) -> dict[str, str]:
    if not team_id.strip():
        raise BuildError("Apple development team ID is required")
    if not udid.strip():
        raise BuildError("device UDID is required")
    host = sys.platform if platform_name is None else platform_name
    if host != "darwin":
        raise BuildError("candidate Xcode build requires macOS")
    resolved: dict[str, str] = {}
    missing: list[str] = []
    for tool in REQUIRED_TOOLS:
        path = tool_lookup(tool)
        if path is None:
            missing.append(tool)
        else:
            resolved[tool] = path
    if missing:
        raise BuildError(f"missing required Apple toolchain: {', '.join(missing)}")
    return resolved


def create_fresh_derived_data_layout(requested_root: Path) -> DerivedDataLayout:
    requested_root = Path(requested_root)
    try:
        requested_root.mkdir(parents=True, exist_ok=True)
        root = Path(tempfile.mkdtemp(prefix="invocation-", dir=requested_root))
        unit_tests = root / "unit-tests"
        runner = root / "runner"
        unit_tests.mkdir()
        runner.mkdir()
    except OSError as exc:
        raise BuildError(f"failed to create fresh DerivedData under {requested_root}") from exc
    return DerivedDataLayout(root=root, unit_tests=unit_tests, runner=runner)


def make_xcodebuild_command(
    *,
    source: Path,
    derived_data: Path,
    xcconfig: Path,
    team_id: str,
    udid: str,
    source_sha256: str,
    xcconfig_sha256: str,
    xcode: XcodeVersion,
) -> list[str]:
    source_digest = _validate_sha256(source_sha256, "prepared source digest")
    xcconfig_digest = _validate_sha256(xcconfig_sha256, "locked xcconfig digest")
    return [
        "xcodebuild",
        "build-for-testing",
        "-allowProvisioningUpdates",
        "-allowProvisioningDeviceRegistration",
        "-project",
        str(Path(source) / "WebDriverAgent.xcodeproj"),
        "-scheme",
        "WebDriverAgentRunner",
        "-destination",
        f"id={udid}",
        "-derivedDataPath",
        str(derived_data),
        "-xcconfig",
        str(xcconfig),
        "CODE_SIGN_STYLE=Automatic",
        f"DEVELOPMENT_TEAM={team_id}",
        f"RIVIU_AGENT_SOURCE_SHA256={source_digest}",
        f"RIVIU_AGENT_XCCONFIG_SHA256={xcconfig_digest}",
        "RIVIU_AGENT_OBJECTIVE_C_UNIT_TESTS=PASS",
        f"RIVIU_AGENT_XCODE_VERSION={xcode.version}",
        f"RIVIU_AGENT_XCODE_BUILD={xcode.build}",
        "COMPILER_INDEX_STORE_ENABLE=NO",
        "OTHER_CFLAGS=$(inherited) -Wno-error=poison-system-directories",
    ]


def make_xcode_unit_test_command(
    *, source: Path, derived_data: Path, xcconfig: Path, team_id: str, udid: str
) -> list[str]:
    return [
        "xcodebuild",
        "test",
        "-allowProvisioningUpdates",
        "-allowProvisioningDeviceRegistration",
        "-project",
        str(Path(source) / "WebDriverAgent.xcodeproj"),
        "-scheme",
        "WebDriverAgentLib",
        "-only-testing:UnitTests",
        "-destination",
        f"id={udid}",
        "-derivedDataPath",
        str(derived_data),
        "-xcconfig",
        str(xcconfig),
        "CODE_SIGN_STYLE=Automatic",
        f"DEVELOPMENT_TEAM={team_id}",
        "COMPILER_INDEX_STORE_ENABLE=NO",
        "OTHER_CFLAGS=$(inherited) -Wno-error=poison-system-directories",
    ]


def _run(
    command: Iterable[str],
    *,
    cwd: Path | None = None,
    label: str,
    timeout_seconds: int,
) -> subprocess.CompletedProcess[str]:
    if type(timeout_seconds) is not int or timeout_seconds <= 0:
        raise BuildError(f"{label} timeout must be a positive integer")
    argv = list(command)
    try:
        result = subprocess.run(
            argv,
            cwd=cwd,
            capture_output=True,
            text=True,
            shell=False,
            timeout=timeout_seconds,
        )
    except subprocess.TimeoutExpired as exc:
        raise BuildError(f"{label} timed out after {timeout_seconds} seconds") from exc
    if result.returncode != 0:
        detail = "\n".join(part for part in (result.stdout, result.stderr) if part).strip()
        raise BuildError(f"{label} failed ({result.returncode}): {detail[-4000:]}")
    return result


def _run_prepare(archive: Path, lock: Path, output: Path) -> str:
    result = _run(
        [
            sys.executable,
            str(PREPARE_SCRIPT),
            "--archive",
            str(archive),
            "--lock",
            str(lock),
            "--output",
            str(output),
        ],
        cwd=REPO_ROOT,
        label="source preparation",
        timeout_seconds=PREPARE_TIMEOUT_SECONDS,
    )
    lines = [line for line in result.stdout.splitlines() if line.strip()]
    if not lines:
        raise BuildError("source preparation returned no result")
    try:
        payload = json.loads(lines[-1])
    except json.JSONDecodeError as exc:
        raise BuildError("source preparation returned invalid JSON") from exc
    if not isinstance(payload, dict) or payload.get("ok") is not True:
        raise BuildError("source preparation did not report success")
    digest = payload.get("outputSourceSha256")
    if not isinstance(digest, str):
        raise BuildError("source preparation omitted outputSourceSha256")
    return _validate_sha256(digest, "prepared source digest")


def _locate_runner_app(derived_data: Path) -> Path:
    preferred = (
        Path(derived_data)
        / "Build"
        / "Products"
        / "Debug-iphoneos"
        / "WebDriverAgentRunner-Runner.app"
    )
    if preferred.is_dir():
        return preferred
    candidates = sorted(
        path for path in Path(derived_data).rglob("WebDriverAgentRunner-Runner.app") if path.is_dir()
    )
    if not candidates:
        raise BuildError(f"build succeeded but runner app was not found under {derived_data}")
    if len(candidates) != 1:
        rendered = ", ".join(str(path) for path in candidates)
        raise BuildError(f"build produced multiple runner apps: {rendered}")
    return candidates[0]


def _inspect_signature(app: Path) -> str:
    _run(
        ["codesign", "--verify", "--deep", "--strict", str(app)],
        label="codesign verify",
        timeout_seconds=COMMAND_TIMEOUT_SECONDS,
    )
    result = _run(
        ["codesign", "-d", "--verbose=4", str(app)],
        label="codesign inspect",
        timeout_seconds=COMMAND_TIMEOUT_SECONDS,
    )
    return "\n".join(part for part in (result.stdout, result.stderr) if part)


def _xcode_major_version(xcode: XcodeVersion) -> int:
    match = re.match(r"^(\d+)(?:\.|$)", xcode.version)
    if match is None:
        raise BuildError(f"unsupported Xcode version: {xcode.version}")
    return int(match.group(1))


def resolve_iphoneos_platform_path() -> Path:
    result = _run(
        ["xcrun", "--sdk", "iphoneos", "--show-sdk-platform-path"],
        label="iPhoneOS platform query",
        timeout_seconds=COMMAND_TIMEOUT_SECONDS,
    )
    rendered = result.stdout.strip()
    if not rendered:
        raise BuildError("xcrun returned an empty iPhoneOS platform path")
    platform = Path(rendered)
    if not platform.is_dir():
        raise BuildError(f"iPhoneOS platform path does not exist: {platform}")
    return platform


def ensure_xcode_runtime_dependencies(
    app: Path, platform: Path, xcode: XcodeVersion
) -> None:
    if _xcode_major_version(xcode) < 26:
        return

    frameworks = Path(app) / "Frameworks"
    required_from_build = (
        "Testing.framework/Testing",
        "libXCTestSwiftSupport.dylib",
    )
    for relative in required_from_build:
        if not (frameworks / relative).is_file():
            raise BuildError(f"Xcode 26 runner is missing runtime dependency: {relative}")

    platform = Path(platform)
    interop_source = (
        platform / "Developer" / "usr" / "lib" / "lib_TestingInterop.dylib"
    )
    foundation_source = (
        platform
        / "Developer"
        / "Library"
        / "Frameworks"
        / "_Testing_Foundation.framework"
    )
    foundation_binary = foundation_source / "_Testing_Foundation"
    if not interop_source.is_file():
        raise BuildError(f"active Xcode platform is missing {interop_source}")
    if not foundation_source.is_dir() or not foundation_binary.is_file():
        raise BuildError(f"active Xcode platform is missing {foundation_binary}")

    shutil.copy2(interop_source, frameworks / "lib_TestingInterop.dylib")
    foundation_destination = frameworks / "_Testing_Foundation.framework"
    if foundation_destination.is_symlink() or foundation_destination.is_file():
        foundation_destination.unlink()
    elif foundation_destination.exists():
        shutil.rmtree(foundation_destination)
    shutil.copytree(foundation_source, foundation_destination, symlinks=True)

    for relative in XCODE26_RUNTIME_CLOSURE:
        if not (frameworks / relative).is_file():
            raise BuildError(f"Xcode 26 runtime closure is incomplete: {relative}")


def resign_candidate_tree(app: Path, signer_identity: str) -> None:
    if not signer_identity.strip():
        raise BuildError("candidate signing identity must be nonblank")
    app = Path(app)
    frameworks = app / "Frameworks"
    dependencies = []
    if frameworks.is_dir():
        dependencies = sorted(
            (
                path
                for path in frameworks.iterdir()
                if path.suffix in {".framework", ".dylib"}
            ),
            key=lambda path: path.name,
        )
    plug_ins = app / "PlugIns"
    test_bundles = (
        sorted(plug_ins.glob("*.xctest"), key=lambda path: path.name)
        if plug_ins.is_dir()
        else []
    )
    for path in [*dependencies, *test_bundles, app]:
        _run(
            [
                "codesign",
                "--force",
                "--sign",
                signer_identity,
                "--preserve-metadata=entitlements,flags",
                str(path),
            ],
            label=f"codesign {path.name}",
            timeout_seconds=COMMAND_TIMEOUT_SECONDS,
        )


def finalize_runtime_closure(app: Path, xcode: XcodeVersion) -> str:
    if _xcode_major_version(xcode) >= 26:
        initial_signature = _inspect_signature(app)
        signer_identity = _first_codesign_value(initial_signature, "Authority")
        platform = resolve_iphoneos_platform_path()
        ensure_xcode_runtime_dependencies(app, platform, xcode)
        resign_candidate_tree(app, signer_identity)
    return _inspect_signature(app)


def _ensure_signing_identity() -> None:
    result = _run(
        ["security", "find-identity", "-v", "-p", "codesigning"],
        label="signing identity lookup",
        timeout_seconds=COMMAND_TIMEOUT_SECONDS,
    )
    output = f"{result.stdout}\n{result.stderr}"
    if "0 valid identities found" in output:
        raise BuildError("no valid code-signing identity is available")


def _safe_artifact_version(value: str) -> str:
    if re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9._-]*", value) is None or ".." in value:
        raise BuildError("artifact version contains unsafe path characters")
    return value


def _write_json_atomic(path: Path, payload: dict[str, Any]) -> None:
    contents = json.dumps(payload, ensure_ascii=True, indent=2, sort_keys=True) + "\n"
    path.parent.mkdir(parents=True, exist_ok=True)
    descriptor, temporary_name = tempfile.mkstemp(
        prefix=f".{path.name}.", suffix=".tmp", dir=path.parent
    )
    temporary = Path(temporary_name)
    try:
        with os.fdopen(descriptor, "w", encoding="ascii", newline="\n") as stream:
            stream.write(contents)
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(temporary, path)
    except BaseException:
        try:
            os.close(descriptor)
        except OSError:
            pass
        temporary.unlink(missing_ok=True)
        raise


def build_candidate(args: argparse.Namespace) -> tuple[Path, Path]:
    if not args.team_id.strip():
        raise BuildError("Apple development team ID is required")
    if not args.udid.strip():
        raise BuildError("device UDID is required")
    version = _safe_artifact_version(args.artifact_version)
    xcconfig_digest = load_locked_xcconfig_sha256(args.lock)
    source_digest = _run_prepare(args.archive, args.lock, args.source)
    require_source_digest(args.source, source_digest, "source preparation")
    validate_build_requirements(team_id=args.team_id, udid=args.udid)
    for required_path, label in (
        (args.xcconfig, "candidate xcconfig"),
        (args.source / "WebDriverAgent.xcodeproj", "prepared Xcode project"),
    ):
        if not required_path.exists():
            raise BuildError(f"{label} does not exist: {required_path}")
    require_xcconfig_digest(args.xcconfig, xcconfig_digest, "build preflight")

    xcode_result = _run(
        ["xcodebuild", "-version"],
        label="Xcode version query",
        timeout_seconds=COMMAND_TIMEOUT_SECONDS,
    )
    xcode = parse_xcode_version(xcode_result.stdout)
    _ensure_signing_identity()
    derived_data = create_fresh_derived_data_layout(args.derived_data)
    unit_test_command = make_xcode_unit_test_command(
        source=args.source,
        derived_data=derived_data.unit_tests,
        xcconfig=args.xcconfig,
        team_id=args.team_id,
        udid=args.udid,
    )
    _run(
        unit_test_command,
        cwd=args.source,
        label="Objective-C unit tests",
        timeout_seconds=XCODE_BUILD_TIMEOUT_SECONDS,
    )
    require_source_digest(args.source, source_digest, "Objective-C unit tests")
    require_xcconfig_digest(args.xcconfig, xcconfig_digest, "Objective-C unit tests")
    command = make_xcodebuild_command(
        source=args.source,
        derived_data=derived_data.runner,
        xcconfig=args.xcconfig,
        team_id=args.team_id,
        udid=args.udid,
        source_sha256=source_digest,
        xcconfig_sha256=xcconfig_digest,
        xcode=xcode,
    )
    _run(
        command,
        cwd=args.source,
        label="Xcode candidate build",
        timeout_seconds=XCODE_BUILD_TIMEOUT_SECONDS,
    )
    require_source_digest(args.source, source_digest, "runner build")
    require_xcconfig_digest(args.xcconfig, xcconfig_digest, "runner build")

    app = _locate_runner_app(derived_data.runner)
    signature_output = finalize_runtime_closure(app, xcode)
    require_source_digest(args.source, source_digest, "runtime finalization")
    require_xcconfig_digest(args.xcconfig, xcconfig_digest, "runtime finalization")
    identity = capture_bundle_identity(
        app,
        signature_output,
        expected_source_sha256=source_digest,
        expected_xcconfig_sha256=xcconfig_digest,
        expected_xcode=xcode,
    )
    if identity.signer_team_id != args.team_id:
        raise BuildError(
            "signed app team does not match requested team: "
            f"actual={identity.signer_team_id}, requested={args.team_id}"
        )

    artifact_dir = args.artifacts_root / version
    ipa = artifact_dir / "RiviuAgent-candidate.ipa"
    package_candidate_ipa(app, ipa)
    manifest = generate_candidate_manifest(
        artifact_version=version,
        artifact_root=artifact_dir,
        ipa_path=ipa,
        app_name=app.name,
        identity=identity,
    )
    manifest_path = artifact_dir / "candidate-manifest.json"
    _write_json_atomic(manifest_path, manifest)
    return ipa, manifest_path


def _parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--udid", required=True)
    parser.add_argument("--team-id", required=True)
    parser.add_argument("--artifact-version", default=DEFAULT_ARTIFACT_VERSION)
    parser.add_argument("--archive", type=Path, default=DEFAULT_ARCHIVE)
    parser.add_argument("--lock", type=Path, default=DEFAULT_LOCK)
    parser.add_argument("--source", type=Path, default=DEFAULT_SOURCE)
    parser.add_argument("--derived-data", type=Path, default=DEFAULT_DERIVED_DATA)
    parser.add_argument("--artifacts-root", type=Path, default=DEFAULT_ARTIFACTS_ROOT)
    parser.add_argument("--xcconfig", type=Path, default=DEFAULT_XCCONFIG)
    return parser


def main() -> int:
    args = _parser().parse_args()
    try:
        ipa, manifest = build_candidate(args)
    except BuildError as exc:
        print(json.dumps({"ok": False, "error": str(exc)}, ensure_ascii=True, sort_keys=True))
        return 1
    print(
        json.dumps(
            {"ok": True, "gateStatus": GATE_STATUS, "ipa": str(ipa), "manifest": str(manifest)},
            ensure_ascii=True,
            sort_keys=True,
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
