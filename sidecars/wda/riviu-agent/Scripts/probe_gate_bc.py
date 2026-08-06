#!/usr/bin/env python3
"""Attest and probe the standalone Riviu Agent candidate on a Mac/iPhone."""

from __future__ import annotations

import argparse
import asyncio
import base64
import binascii
import contextlib
import hashlib
import http.client
import importlib.metadata
import io
import json
import math
import os
import plistlib
import re
import shutil
import socket
import subprocess
import sys
import tempfile
import threading
import time
import zipfile
from dataclasses import dataclass
from datetime import datetime, timezone
from pathlib import Path, PurePosixPath
from typing import Any, Callable, Protocol


AGENT_ROOT = Path(__file__).resolve().parents[1]
REPOSITORY_ROOT = AGENT_ROOT.parents[2]
DEFAULT_OUTPUT = REPOSITORY_ROOT / "docs" / "re" / "riviu-agent" / "candidate-probes.json"
DEFAULT_LOCK = AGENT_ROOT / "baseline-lock.json"
DEFAULT_XCCONFIG = AGENT_ROOT / "Config" / "RiviuAgent.xcconfig"
DEFAULT_ARCHIVE = (
    REPOSITORY_ROOT / "target" / "rtmmo-re" / "baselines" / "appium-webdriveragent-15.1.4.tgz"
)

TOKEN_ENVIRONMENT = "RIVIU_AGENT_TOKEN"
TOKEN_HEADER = "X-Riviu-Token"
CONTROL_DEVICE_PORT = 8916
MJPEG_DEVICE_PORT = 9094
EXPECTED_FEATURES = ["stream", "tap", "swipe", "clipboard"]
EXPECTED_ORDER = ["launch", "health", "foreground", "session", "mjpeg"]
EXPECTED_ARTIFACT_ID = "riviu-agent-ios-candidate"
EXPECTED_PROTOCOL_VERSION = 2
EXPECTED_AGENT_VERSION = "0.1.0"
EXPECTED_BUNDLE_ID = "com.riviu.managersphone.agent.xctrunner"
EXPECTED_PAYLOAD_APP = "WebDriverAgentRunner-Runner.app"
EXPECTED_EXECUTABLE = "WebDriverAgentRunner-Runner"
EXPECTED_ATTESTATION_BUNDLE = "PlugIns/WebDriverAgentRunner.xctest"
SETTINGS_BUNDLE_ID = "com.apple.Preferences"
SETTINGS_FOREGROUND_SETTLE_SECONDS = 5.0
SETTINGS_BACK_TAP = {"x": 20.0, "y": 50.0}
SETTINGS_ROOT_RETRIES = 6
SETTINGS_SEARCH_PULL_DOWN = {
    "fromX": 187.0,
    "fromY": 180.0,
    "toX": 187.0,
    "toY": 600.0,
    "delay": 0.25,
}
SETTINGS_SEARCH_RETRIES = 3
SETTINGS_POST_ACTION_SETTLE_SECONDS = 1.0
ACTIVE_APP_SETTLE_TIMEOUT_SECONDS = 5.0
ACTIVE_APP_SETTLE_POLL_SECONDS = 0.25
MAX_HTTP_BODY_BYTES = 16 * 1024 * 1024
MAX_MJPEG_BUFFER_BYTES = 16 * 1024 * 1024
MAX_TOKEN_SCAN_FILE_BYTES = 128 * 1024 * 1024
MAX_TOKEN_SCAN_TOTAL_BYTES = 1024 * 1024 * 1024
MAX_TOKEN_SCAN_FILES = 50_000
TOKEN_PREFLIGHT_FIELDS = (
    "manifestTokenScanClean",
    "ipaTokenScanClean",
    "sourceTokenScanClean",
    "xcconfigTokenScanClean",
    "argvTokenScanClean",
)
TOKEN_GATE_FIELDS = TOKEN_PREFLIGHT_FIELDS + (
    "logTokenScanClean",
    "reportTokenScanClean",
)

# These are acceptance thresholds, not caller-configurable defaults.
GATE_COLD_LAUNCHES = 5
GATE_TAP_SUCCESSES = 50
GATE_SWIPE_SUCCESSES = 20
GATE_STREAM_SECONDS = 300.0
GATE_MIN_STREAM_FPS = 1.0
GATE_MAX_FRAME_GAP_SECONDS = 2.0
GATE_MAX_STREAM_RECONNECTS = 1
GATE_CONTROL_INTERVAL_SECONDS = 5.0
GATE_MIN_CONTROL_CHECKS = math.ceil(GATE_STREAM_SECONDS / GATE_CONTROL_INTERVAL_SECONDS)
GATE_MAX_CONTROL_CYCLE_SECONDS = 5.0
GATE_MAX_CONTROL_COMPLETION_GAP_SECONDS = 5.5
GATE_MAX_CONTROL_SCHEDULE_LATENESS_SECONDS = 0.5
GATE_MIN_VISUAL_DELTA = 6.0
GATE_VISUAL_CAUSAL_MARGIN = 3.0
GATE_ADDITIONAL_CONTROL_FRAMES_PER_ACTION = 3
GATE_CONTROL_FRAME_SAMPLES_PER_ACTION = GATE_ADDITIONAL_CONTROL_FRAMES_PER_ACTION + 1
INSTALL_OPERATION_TIMEOUT_SECONDS = 180.0
# installation_proxy can return before deviceprocesscontrolservice registers the app
POST_INSTALL_SETTLE_SECONDS = 60.0
PROCESS_CONTROL_TIMEOUT_SECONDS = 30.0
# A freshly installed XCTest runner can return transient DTX code 2 while the
# device process-control service registers its application record.
DVT_LAUNCH_ATTEMPTS = 8
DVT_LAUNCH_RETRY_DELAY_SECONDS = 1.5
DEVICE_PORT_PROBE_TIMEOUT_SECONDS = 5.0
LIVE_ENVIRONMENT = "LIVE_MAC_DEVICE"
SUPPLEMENTAL_ENVIRONMENT = "SUPPLEMENTAL_MAC_DEVICE"
FIXTURE_ENVIRONMENT = "FIXTURE_ONLY"

CONTENT_REGION = (0.05, 0.08, 0.95, 0.92)


class ProbeError(RuntimeError):
    pass


@dataclass(frozen=True, repr=False)
class SecretToken:
    _value: str

    def __post_init__(self) -> None:
        if len(self._value.encode("utf-8")) < 32:
            raise ProbeError("RIVIU_AGENT_TOKEN must contain at least 256 bits")

    def reveal(self) -> str:
        return self._value

    def variants(self) -> set[str]:
        raw = self._value.encode("utf-8")
        encoded = {
            self._value,
            raw.hex(),
            raw.hex().upper(),
            base64.b64encode(raw).decode("ascii"),
            base64.urlsafe_b64encode(raw).decode("ascii"),
            json.dumps(self._value, ensure_ascii=True)[1:-1],
        }
        encoded.update(value.rstrip("=") for value in tuple(encoded) if value.endswith("="))
        return encoded

    def byte_variants(self) -> tuple[bytes, ...]:
        encoded = {value.encode("utf-8") for value in self.variants() if value}
        encoded.add(self._value.encode("utf-16-le"))
        encoded.add(self._value.encode("utf-16-be"))
        return tuple(sorted(encoded, key=lambda value: (len(value), value)))

    @staticmethod
    def redacted() -> str:
        return "<redacted>"

    def __repr__(self) -> str:
        return self.redacted()

    def __str__(self) -> str:
        return self.redacted()


class SecretScanningWriter:
    def __init__(self, destination: Any, token: SecretToken) -> None:
        self.destination = destination
        self.patterns = tuple(sorted(token.variants(), key=lambda value: (len(value), value)))
        self.maximum_pattern = max(len(pattern) for pattern in self.patterns)
        self.tail = ""
        self.compromised = False
        self._lock = threading.Lock()

    def write(self, value: str) -> int:
        if not isinstance(value, str):
            raise TypeError("process output must be text")
        with self._lock:
            if self.compromised:
                raise ProbeError("process output token guard is compromised")
            candidate = self.tail + value
            if any(pattern and pattern in candidate for pattern in self.patterns):
                self.tail = ""
                self.compromised = True
                raise ProbeError("process output contains an agent-token representation")
            retained = max(0, self.maximum_pattern - 1)
            safe_length = max(0, len(candidate) - retained)
            if safe_length:
                self.destination.write(candidate[:safe_length])
            self.tail = candidate[safe_length:]
        return len(value)

    def flush(self) -> None:
        self.destination.flush()

    def finish(self) -> None:
        with self._lock:
            if not self.compromised and self.tail:
                self.destination.write(self.tail)
            self.tail = ""
        self.destination.flush()

    def __getattr__(self, name: str) -> Any:
        return getattr(self.destination, name)


@contextlib.contextmanager
def guard_process_output(token: SecretToken):
    original_stdout = sys.stdout
    original_stderr = sys.stderr
    guarded_stdout = SecretScanningWriter(original_stdout, token)
    guarded_stderr = SecretScanningWriter(original_stderr, token)
    sys.stdout = guarded_stdout
    sys.stderr = guarded_stderr
    try:
        yield
    finally:
        sys.stdout = original_stdout
        sys.stderr = original_stderr
        guarded_stdout.finish()
        guarded_stderr.finish()


@dataclass(frozen=True)
class CandidateArtifact:
    manifest_sha256: str
    ipa_sha256: str
    source_sha256: str
    xcconfig_sha256: str
    ipa_path: Path
    manifest_path: Path
    artifact_version: str
    bundle_id: str
    bundle_version: str
    bundle_build: str
    payload_app: str
    executable: str
    attestation_bundle: str
    signature_identifier: str
    signer_identity: str
    signer_team_id: str
    xcode_version: str
    xcode_build: str
    features: tuple[str, ...]

    def evidence(self) -> dict[str, Any]:
        return {
            "artifactId": EXPECTED_ARTIFACT_ID,
            "artifactVersion": self.artifact_version,
            "manifestSha256": self.manifest_sha256,
            "ipaSha256": self.ipa_sha256,
            "sourceSha256": self.source_sha256,
            "xcconfigSha256": self.xcconfig_sha256,
            "ipaMetadataBound": True,
            "bundleId": self.bundle_id,
            "bundleVersion": self.bundle_version,
            "bundleBuild": self.bundle_build,
            "payloadApp": self.payload_app,
            "executable": self.executable,
            "attestationBundle": self.attestation_bundle,
            "signatureIdentifier": self.signature_identifier,
            "signerIdentitySha256": _sha256_text(self.signer_identity),
            "signerTeamId": self.signer_team_id,
            "xcode": {"version": self.xcode_version, "build": self.xcode_build},
            "protocolVersion": EXPECTED_PROTOCOL_VERSION,
            "features": list(self.features),
        }


@dataclass(frozen=True)
class HttpResult:
    status: int
    payload: Any


@dataclass(frozen=True)
class VisualFrame:
    width: int
    height: int
    samples: bytes


def _sha256_path(path: Path, label: str) -> str:
    digest = hashlib.sha256()
    try:
        with path.open("rb") as stream:
            while chunk := stream.read(1024 * 1024):
                digest.update(chunk)
    except OSError as exc:
        raise ProbeError(f"{label} could not be hashed") from exc
    return digest.hexdigest()


def _sha256_file(path: Path) -> str:
    return _sha256_path(path, "candidate IPA")


def _sha256_text(value: str) -> str:
    return hashlib.sha256(value.encode("utf-8")).hexdigest()


def _stream_token_clean(
    stream: Any,
    patterns: tuple[bytes, ...],
    *,
    maximum_bytes: int,
) -> bool:
    longest = max(len(pattern) for pattern in patterns)
    tail = b""
    consumed = 0
    while True:
        chunk = stream.read(1024 * 1024)
        if not chunk:
            return True
        consumed += len(chunk)
        if consumed > maximum_bytes:
            raise ProbeError("token scan input exceeds its size limit")
        candidate = tail + chunk
        if any(pattern in candidate for pattern in patterns):
            return False
        tail = candidate[-(longest - 1) :] if longest > 1 else b""


def _path_token_clean(path: Path, patterns: tuple[bytes, ...]) -> bool:
    try:
        size = path.stat().st_size
        if size > MAX_TOKEN_SCAN_FILE_BYTES:
            raise ProbeError("token scan file exceeds its size limit")
        with path.open("rb") as stream:
            return _stream_token_clean(stream, patterns, maximum_bytes=size)
    except ProbeError:
        raise
    except OSError as exc:
        raise ProbeError("token scan input could not be read") from exc


def _ipa_token_clean(ipa_path: Path, patterns: tuple[bytes, ...]) -> bool:
    total = 0
    try:
        with zipfile.ZipFile(ipa_path) as archive:
            entries = archive.infolist()
            if len(entries) > MAX_TOKEN_SCAN_FILES:
                raise ProbeError("token scan IPA contains too many entries")
            for entry in entries:
                if any(pattern in entry.filename.encode("utf-8") for pattern in patterns):
                    return False
                if entry.flag_bits & 0x1:
                    raise ProbeError("token scan IPA contains an encrypted entry")
                if entry.file_size > MAX_TOKEN_SCAN_FILE_BYTES:
                    raise ProbeError("token scan IPA entry exceeds its size limit")
                total += entry.file_size
                if total > MAX_TOKEN_SCAN_TOTAL_BYTES:
                    raise ProbeError("token scan IPA exceeds its total size limit")
                if entry.is_dir():
                    continue
                with archive.open(entry) as stream:
                    if not _stream_token_clean(
                        stream, patterns, maximum_bytes=entry.file_size
                    ):
                        return False
    except ProbeError:
        raise
    except (OSError, UnicodeError, zipfile.BadZipFile) as exc:
        raise ProbeError("candidate IPA token scan failed") from exc
    return True


def _source_token_clean(source: Path, patterns: tuple[bytes, ...]) -> bool:
    if not source.is_dir():
        raise ProbeError("prepared source for token scan is missing")
    files = sorted(path for path in source.rglob("*") if path.is_file())
    if len(files) > MAX_TOKEN_SCAN_FILES:
        raise ProbeError("prepared source token scan contains too many files")
    total = 0
    for path in files:
        relative = path.relative_to(source).as_posix().encode("utf-8")
        if any(pattern in relative for pattern in patterns):
            return False
        try:
            size = path.stat().st_size
        except OSError as exc:
            raise ProbeError("prepared source token scan failed") from exc
        total += size
        if total > MAX_TOKEN_SCAN_TOTAL_BYTES:
            raise ProbeError("prepared source token scan exceeds its total size limit")
        if not _path_token_clean(path, patterns):
            return False
    return True


def scan_token_preflight(
    *,
    artifact: CandidateArtifact,
    token: SecretToken,
    prepared_source: Path,
    xcconfig: Path = DEFAULT_XCCONFIG,
    argv: list[str] | tuple[str, ...],
) -> dict[str, bool]:
    patterns = token.byte_variants()
    argv_bytes = "\0".join(str(value) for value in argv).encode("utf-8")
    xcconfig = Path(xcconfig)
    if _sha256_path(xcconfig, "candidate xcconfig") != artifact.xcconfig_sha256:
        raise ProbeError("candidate xcconfig SHA-256 does not match its manifest")
    return {
        "manifestTokenScanClean": _path_token_clean(artifact.manifest_path, patterns),
        "ipaTokenScanClean": _ipa_token_clean(artifact.ipa_path, patterns),
        "sourceTokenScanClean": _source_token_clean(Path(prepared_source), patterns),
        "xcconfigTokenScanClean": _path_token_clean(xcconfig, patterns),
        "argvTokenScanClean": not any(pattern in argv_bytes for pattern in patterns),
    }


def token_preflight_is_clean(evidence: dict[str, Any]) -> bool:
    return set(evidence) == set(TOKEN_PREFLIGHT_FIELDS) and all(
        evidence.get(field) is True for field in TOKEN_PREFLIGHT_FIELDS
    )


def prepare_locked_source_for_scan(
    artifact: CandidateArtifact,
    output: Path,
    *,
    archive: Path = DEFAULT_ARCHIVE,
    lock: Path = DEFAULT_LOCK,
) -> None:
    environment = os.environ.copy()
    environment.pop(TOKEN_ENVIRONMENT, None)
    command = [
        sys.executable,
        str(AGENT_ROOT / "Scripts" / "prepare.py"),
        "--archive",
        str(archive),
        "--lock",
        str(lock),
        "--output",
        str(output),
    ]
    try:
        result = subprocess.run(
            command,
            cwd=REPOSITORY_ROOT,
            env=environment,
            capture_output=True,
            text=True,
            shell=False,
            timeout=300.0,
        )
    except subprocess.TimeoutExpired as exc:
        raise ProbeError("locked source reconstruction exceeded its deadline") from exc
    if result.returncode != 0:
        raise ProbeError("locked source reconstruction failed")
    lines = [line for line in result.stdout.splitlines() if line.strip()]
    if not lines:
        raise ProbeError("locked source reconstruction returned no attestation")
    try:
        payload = json.loads(lines[-1])
    except json.JSONDecodeError as exc:
        raise ProbeError("locked source reconstruction attestation is invalid") from exc
    if (
        not isinstance(payload, dict)
        or payload.get("ok") is not True
        or payload.get("outputSourceSha256") != artifact.source_sha256
        or not Path(output).is_dir()
    ):
        raise ProbeError("locked source reconstruction does not match the candidate")


def installed_team_id(metadata: dict[str, Any]) -> str:
    candidates: set[str] = set()

    def add(value: Any) -> None:
        if isinstance(value, str) and value.strip():
            candidates.add(value.strip())

    add(metadata.get("TeamIdentifier"))
    add(metadata.get("com.apple.developer.team-identifier"))
    entitlements = metadata.get("Entitlements")
    if isinstance(entitlements, dict):
        add(entitlements.get("com.apple.developer.team-identifier"))
        application_identifier = entitlements.get("application-identifier")
        if isinstance(application_identifier, str) and "." in application_identifier:
            add(application_identifier.split(".", 1)[0])
    application_identifier = metadata.get("ApplicationIdentifierEntitlement")
    if isinstance(application_identifier, str) and "." in application_identifier:
        add(application_identifier.split(".", 1)[0])
    if len(candidates) != 1:
        raise ProbeError("installed candidate team identity is missing or inconsistent")
    return next(iter(candidates))


def _json_without_duplicate_keys(raw: bytes, label: str) -> dict[str, Any]:
    def pairs_hook(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
        result: dict[str, Any] = {}
        for key, value in pairs:
            if key in result:
                raise ProbeError(f"{label} contains a duplicate JSON key")
            result[key] = value
        return result

    try:
        parsed = json.loads(raw.decode("utf-8"), object_pairs_hook=pairs_hook)
    except (UnicodeError, json.JSONDecodeError) as exc:
        raise ProbeError(f"{label} is not valid UTF-8 JSON") from exc
    if not isinstance(parsed, dict):
        raise ProbeError(f"{label} root must be an object")
    return parsed


def _required_string(value: dict[str, Any], key: str, label: str) -> str:
    candidate = value.get(key)
    if not isinstance(candidate, str) or not candidate.strip():
        raise ProbeError(f"{label} field {key} must be a nonblank string")
    return candidate.strip()


def _required_sha256(value: dict[str, Any], key: str, label: str) -> str:
    candidate = _required_string(value, key, label).lower()
    if re.fullmatch(r"[0-9a-f]{64}", candidate) is None:
        raise ProbeError(f"{label} field {key} must be a SHA-256 digest")
    return candidate


def _verify_ipa_metadata_binding(
    ipa_path: Path,
    *,
    payload_app: str,
    executable: str,
    bundle_id: str,
    bundle_version: str,
    bundle_build: str,
    attestation_bundle: str,
    source_sha256: str,
    xcconfig_sha256: str,
    xcode_version: str,
    xcode_build: str,
) -> None:
    app_root = f"Payload/{payload_app}"
    info_name = f"{app_root}/Info.plist"
    attestation_info_name = f"{app_root}/{attestation_bundle}/Info.plist"
    executable_name = f"{app_root}/{executable}"
    try:
        with zipfile.ZipFile(ipa_path) as archive:
            names = archive.namelist()
            if len(names) != len(set(names)):
                raise ProbeError("candidate IPA contains duplicate archive entries")
            if (
                info_name not in names
                or executable_name not in names
                or attestation_info_name not in names
            ):
                raise ProbeError("candidate IPA payload identity is incomplete")
            info_entry = archive.getinfo(info_name)
            attestation_entry = archive.getinfo(attestation_info_name)
            if info_entry.file_size > 1024 * 1024 or attestation_entry.file_size > 1024 * 1024:
                raise ProbeError("candidate IPA Info.plist exceeds its size limit")
            raw_info = archive.read(info_entry)
            raw_attestation = archive.read(attestation_entry)
    except ProbeError:
        raise
    except (OSError, KeyError, zipfile.BadZipFile) as exc:
        raise ProbeError("candidate IPA could not be inspected") from exc
    try:
        info = plistlib.loads(raw_info)
    except plistlib.InvalidFileException as exc:
        raise ProbeError("candidate IPA Info.plist is invalid") from exc
    if not isinstance(info, dict):
        raise ProbeError("candidate IPA Info.plist root must be a dictionary")
    try:
        attestation = plistlib.loads(raw_attestation)
    except plistlib.InvalidFileException as exc:
        raise ProbeError("candidate IPA attestation Info.plist is invalid") from exc
    if not isinstance(attestation, dict):
        raise ProbeError("candidate IPA attestation Info.plist root must be a dictionary")
    outer_expected = {
        "CFBundleIdentifier": bundle_id,
        "CFBundleShortVersionString": bundle_version,
        "CFBundleVersion": bundle_build,
        "CFBundleExecutable": executable,
    }
    attestation_expected = {
        "RiviuAgentSourceSHA256": source_sha256,
        "RiviuAgentXcconfigSHA256": xcconfig_sha256,
        "RiviuAgentObjectiveCUnitTests": "PASS",
        "RiviuAgentXcodeVersion": xcode_version,
        "RiviuAgentXcodeBuild": xcode_build,
    }
    if any(info.get(key) != value for key, value in outer_expected.items()) or any(
        attestation.get(key) != value for key, value in attestation_expected.items()
    ):
        raise ProbeError("candidate IPA metadata does not match its manifest and baseline lock")
    if attestation.get("RiviuAgentProtocolVersion") != EXPECTED_PROTOCOL_VERSION:
        raise ProbeError("candidate IPA protocol metadata does not match protocol v2")


def load_candidate_artifact(
    manifest_path: Path, *, lock_path: Path = DEFAULT_LOCK
) -> CandidateArtifact:
    manifest_path = Path(manifest_path)
    try:
        raw_manifest = manifest_path.read_bytes()
        lock = _json_without_duplicate_keys(Path(lock_path).read_bytes(), "baseline lock")
    except OSError as exc:
        raise ProbeError("candidate manifest or baseline lock could not be read") from exc
    manifest = _json_without_duplicate_keys(raw_manifest, "candidate manifest")

    exact_values = {
        "schemaVersion": 1,
        "artifactId": EXPECTED_ARTIFACT_ID,
        "gateStatus": "PENDING_MAC_DEVICE",
        "objectiveCUnitTests": "PASS",
        "protocolVersion": EXPECTED_PROTOCOL_VERSION,
        "controlPort": CONTROL_DEVICE_PORT,
        "mjpegPort": MJPEG_DEVICE_PORT,
        "logicalWidth": 375,
        "logicalHeight": 667,
    }
    for key, expected in exact_values.items():
        if manifest.get(key) != expected:
            raise ProbeError(f"candidate manifest field {key} does not match the gate contract")
    manifest_features = manifest.get("features")
    expected_feature_prefix = list(EXPECTED_FEATURES)
    if (
        not isinstance(manifest_features, list)
        or manifest_features[: len(expected_feature_prefix)] != expected_feature_prefix
        or any(not isinstance(feature, str) for feature in manifest_features)
        or any(feature not in {"pushMedia"} for feature in manifest_features[len(expected_feature_prefix) :])
        or len(manifest_features) != len(set(manifest_features))
    ):
        raise ProbeError("candidate manifest feature set does not match protocol v2")

    source_sha256 = _required_sha256(manifest, "sourceSha256", "candidate manifest")
    expected_source_sha256 = _required_sha256(lock, "outputSourceSha256", "baseline lock")
    if source_sha256 != expected_source_sha256:
        raise ProbeError("candidate source digest does not match the locked overlay")
    xcconfig_sha256 = _required_sha256(
        manifest, "xcconfigSha256", "candidate manifest"
    )
    expected_xcconfig_sha256 = _required_sha256(
        lock, "xcconfigSha256", "baseline lock"
    )
    if xcconfig_sha256 != expected_xcconfig_sha256:
        raise ProbeError("candidate xcconfig digest does not match the baseline lock")

    relative_ipa = Path(_required_string(manifest, "ipa", "candidate manifest"))
    if relative_ipa.is_absolute() or relative_ipa.suffix.lower() != ".ipa":
        raise ProbeError("candidate manifest IPA must be a relative .ipa path")
    if not relative_ipa.parts or any(part in {"", ".", ".."} for part in relative_ipa.parts):
        raise ProbeError("candidate manifest IPA path is unsafe")
    root = manifest_path.parent.resolve(strict=False)
    ipa_path = (root / relative_ipa).resolve(strict=False)
    try:
        ipa_path.relative_to(root)
    except ValueError as exc:
        raise ProbeError("candidate manifest IPA leaves its artifact directory") from exc
    if not ipa_path.is_file():
        raise ProbeError("candidate IPA referenced by the manifest is missing")
    ipa_sha256 = _required_sha256(manifest, "sha256", "candidate manifest")
    if _sha256_file(ipa_path) != ipa_sha256:
        raise ProbeError("candidate IPA SHA-256 does not match its manifest")

    xcode = manifest.get("xcode")
    if not isinstance(xcode, dict):
        raise ProbeError("candidate manifest xcode field must be an object")
    bundle_id = _required_string(manifest, "bundleId", "candidate manifest")
    signature_identifier = _required_string(
        manifest, "signatureIdentifier", "candidate manifest"
    )
    if signature_identifier != bundle_id:
        raise ProbeError("candidate signature identifier does not match its bundle ID")
    payload_app = _required_string(manifest, "payloadApp", "candidate manifest")
    executable = _required_string(manifest, "executable", "candidate manifest")
    attestation_bundle = _required_string(
        manifest, "attestationBundle", "candidate manifest"
    )
    if (
        PurePosixPath(payload_app).name != payload_app
        or not payload_app.endswith(".app")
        or PurePosixPath(executable).name != executable
    ):
        raise ProbeError("candidate payload or executable name is unsafe")
    if (
        bundle_id != EXPECTED_BUNDLE_ID
        or payload_app != EXPECTED_PAYLOAD_APP
        or executable != EXPECTED_EXECUTABLE
        or attestation_bundle != EXPECTED_ATTESTATION_BUNDLE
    ):
        raise ProbeError("candidate identity is outside the Riviu artifact policy")
    artifact_version = _required_string(manifest, "artifactVersion", "candidate manifest")
    bundle_version = _required_string(manifest, "bundleVersion", "candidate manifest")
    bundle_build = _required_string(manifest, "bundleBuild", "candidate manifest")
    signer_identity = _required_string(manifest, "signerIdentity", "candidate manifest")
    signer_team_id = _required_string(manifest, "signerTeamId", "candidate manifest")
    xcode_version = _required_string(xcode, "version", "candidate manifest xcode")
    xcode_build = _required_string(xcode, "build", "candidate manifest xcode")
    _verify_ipa_metadata_binding(
        ipa_path,
        payload_app=payload_app,
        executable=executable,
        bundle_id=bundle_id,
        bundle_version=bundle_version,
        bundle_build=bundle_build,
        attestation_bundle=attestation_bundle,
        source_sha256=source_sha256,
        xcconfig_sha256=xcconfig_sha256,
        xcode_version=xcode_version,
        xcode_build=xcode_build,
    )

    return CandidateArtifact(
        manifest_sha256=hashlib.sha256(raw_manifest).hexdigest(),
        ipa_sha256=ipa_sha256,
        source_sha256=source_sha256,
        xcconfig_sha256=xcconfig_sha256,
        ipa_path=ipa_path,
        manifest_path=manifest_path.resolve(strict=False),
        artifact_version=artifact_version,
        bundle_id=bundle_id,
        bundle_version=bundle_version,
        bundle_build=bundle_build,
        payload_app=payload_app,
        executable=executable,
        attestation_bundle=attestation_bundle,
        signature_identifier=signature_identifier,
        signer_identity=signer_identity,
        signer_team_id=signer_team_id,
        xcode_version=xcode_version,
        xcode_build=xcode_build,
        features=tuple(manifest_features),
    )


def decode_visual_frame(
    frame: bytes, region: tuple[float, float, float, float] = CONTENT_REGION
) -> VisualFrame:
    try:
        from PIL import Image, UnidentifiedImageError
    except ImportError as exc:
        raise ProbeError("Pillow is required to decode MJPEG evidence") from exc
    try:
        with Image.open(io.BytesIO(frame)) as image:
            if image.format != "JPEG":
                raise ProbeError("MJPEG frame is not a JPEG image")
            image.load()
            width, height = image.size
            if width < 32 or height < 32 or width > 4096 or height > 4096:
                raise ProbeError("MJPEG frame dimensions are outside the gate bounds")
            left = max(0, min(width - 1, round(region[0] * width)))
            top = max(0, min(height - 1, round(region[1] * height)))
            right = max(left + 1, min(width, round(region[2] * width)))
            bottom = max(top + 1, min(height, round(region[3] * height)))
            resampling = getattr(Image, "Resampling", Image).BILINEAR
            samples = image.convert("L").crop((left, top, right, bottom)).resize(
                (32, 32), resampling
            )
            return VisualFrame(width=width, height=height, samples=samples.tobytes())
    except ProbeError:
        raise
    except (OSError, UnidentifiedImageError, ValueError) as exc:
        raise ProbeError("MJPEG frame failed JPEG decode") from exc


def visual_distance(left: VisualFrame, right: VisualFrame) -> float:
    if len(left.samples) != len(right.samples):
        raise ProbeError("visual samples have incompatible dimensions")
    return sum(abs(a - b) for a, b in zip(left.samples, right.samples)) / len(left.samples)


class ControlClient:
    def __init__(self, host: str, port: int, token: SecretToken, timeout: float) -> None:
        self.host = host
        self.port = int(port)
        self.token = token
        self.timeout = float(timeout)

    def _wrong_token(self) -> str:
        candidate = "x" * 32
        return "y" * 32 if candidate == self.token.reveal() else candidate

    def request(
        self,
        method: str,
        path: str,
        body: dict[str, Any] | None = None,
        *,
        auth: str = "correct",
    ) -> HttpResult:
        if auth not in {"missing", "wrong", "correct"}:
            raise ProbeError(f"unsupported auth mode: {auth}")
        headers = {"Connection": "close", "Accept": "application/json"}
        if auth == "correct":
            headers[TOKEN_HEADER] = self.token.reveal()
        elif auth == "wrong":
            headers[TOKEN_HEADER] = self._wrong_token()
        encoded = None
        if body is not None:
            encoded = json.dumps(body, ensure_ascii=True, separators=(",", ":")).encode("ascii")
            headers["Content-Type"] = "application/json"
            headers["Content-Length"] = str(len(encoded))

        connection = http.client.HTTPConnection(self.host, self.port, timeout=self.timeout)
        try:
            connection.request(method, path, body=encoded, headers=headers)
            response = connection.getresponse()
            raw = response.read(MAX_HTTP_BODY_BYTES + 1)
        except (OSError, TimeoutError, http.client.HTTPException) as exc:
            raise ProbeError(f"HTTP {method} {path} failed") from exc
        finally:
            connection.close()
        if len(raw) > MAX_HTTP_BODY_BYTES:
            raise ProbeError(f"HTTP {method} {path} response exceeds size limit")
        try:
            payload = json.loads(raw.decode("utf-8")) if raw else None
        except (UnicodeError, json.JSONDecodeError) as exc:
            raise ProbeError(f"HTTP {method} {path} returned invalid JSON") from exc
        return HttpResult(response.status, payload)

    def require_ok(
        self, method: str, path: str, body: dict[str, Any] | None = None
    ) -> HttpResult:
        result = self.request(method, path, body)
        if result.status != 200:
            raise ProbeError(f"HTTP {method} {path} returned {result.status}")
        return result

    def create_session(self) -> str:
        result = self.require_ok(
            "POST", "/session", {"capabilities": {"alwaysMatch": {}, "firstMatch": [{}]}}
        )
        payload = result.payload if isinstance(result.payload, dict) else {}
        session_id = payload.get("sessionId")
        value = payload.get("value")
        if not session_id and isinstance(value, dict):
            session_id = value.get("sessionId")
        if not isinstance(session_id, str) or not session_id:
            raise ProbeError("POST /session omitted a session ID")
        return session_id

    def require_session(self, session_id: str) -> None:
        self.require_ok("GET", f"/session/{session_id}")

    @staticmethod
    def _element_id(result: HttpResult) -> str:
        value = result.payload.get("value") if isinstance(result.payload, dict) else None
        if not isinstance(value, dict):
            raise ProbeError("element lookup omitted its value object")
        element_id = value.get("element-6066-11e4-a52e-4f735466cecf") or value.get("ELEMENT")
        if not isinstance(element_id, str) or not element_id:
            raise ProbeError("element lookup omitted an element ID")
        return element_id

    def find_element(self, session_id: str, element_type: str) -> str:
        return self._element_id(
            self.require_ok(
                "POST",
                f"/session/{session_id}/element",
                {"using": "class name", "value": element_type},
            )
        )

    def read_element_text(self, session_id: str, element_id: str) -> str:
        result = self.require_ok("GET", f"/session/{session_id}/element/{element_id}/text")
        value = result.payload.get("value") if isinstance(result.payload, dict) else None
        if not isinstance(value, str):
            raise ProbeError("element text response omitted its string value")
        return value

    def require_active_bundle(self, session_id: str, bundle_id: str) -> int:
        result = self.require_ok("GET", f"/session/{session_id}/wda/activeAppInfo")
        value = result.payload.get("value") if isinstance(result.payload, dict) else None
        if not isinstance(value, dict) or value.get("bundleId") != bundle_id:
            raise ProbeError("active application does not match the gesture surface")
        pid = value.get("pid")
        if isinstance(pid, bool) or not isinstance(pid, int) or pid <= 0:
            raise ProbeError("active application response omitted a valid PID")
        return pid

    def read_element_rect(self, session_id: str, element_id: str) -> dict[str, float]:
        result = self.require_ok("GET", f"/session/{session_id}/element/{element_id}/rect")
        value = result.payload.get("value") if isinstance(result.payload, dict) else None
        if not isinstance(value, dict):
            raise ProbeError("element rect response omitted its value object")
        rect: dict[str, float] = {}
        for key in ("x", "y", "width", "height"):
            number = value.get(key)
            if isinstance(number, bool) or not isinstance(number, (int, float)):
                raise ProbeError("element rect response contains a non-number")
            converted = float(number)
            if not math.isfinite(converted):
                raise ProbeError("element rect response contains a non-finite number")
            rect[key] = converted
        if rect["width"] <= 0 or rect["height"] <= 0:
            raise ProbeError("element rect response contains an empty rectangle")
        return rect

    def read_element_attribute(self, session_id: str, element_id: str, name: str) -> str:
        if not re.fullmatch(r"[A-Za-z][A-Za-z0-9_-]*", name):
            raise ProbeError("element attribute name is invalid")
        result = self.require_ok(
            "GET", f"/session/{session_id}/element/{element_id}/attribute/{name}"
        )
        value = result.payload.get("value") if isinstance(result.payload, dict) else None
        if not isinstance(value, str):
            raise ProbeError("element attribute response omitted its string value")
        return value

    def set_clipboard(self, value: bytes) -> None:
        self.require_ok(
            "POST",
            "/wda/setPasteboard",
            {
                "content": base64.b64encode(value).decode("ascii"),
                "contentType": "plaintext",
            },
        )

    def get_clipboard(self) -> bytes:
        result = self.require_ok(
            "POST", "/wda/getPasteboard", {"contentType": "plaintext"}
        )
        value = result.payload.get("value") if isinstance(result.payload, dict) else None
        if not isinstance(value, str):
            raise ProbeError("clipboard response omitted base64 value")
        try:
            return base64.b64decode(value, validate=True)
        except (ValueError, binascii.Error) as exc:
            raise ProbeError("clipboard response contains invalid base64") from exc


def _extract_jpegs(buffer: bytearray) -> list[bytes]:
    frames: list[bytes] = []
    while True:
        start = buffer.find(b"\xff\xd8")
        if start < 0:
            if len(buffer) > 1:
                del buffer[:-1]
            return frames
        if start > 0:
            del buffer[:start]
        end = buffer.find(b"\xff\xd9", 2)
        if end < 0:
            return frames
        frames.append(bytes(buffer[: end + 2]))
        del buffer[: end + 2]


def _mjpeg_request(token_value: str | None, connection: str = "close") -> bytes:
    parts = [
        "GET / HTTP/1.1",
        "Host: 127.0.0.1",
        f"Connection: {connection}",
    ]
    if token_value is not None:
        parts.append(f"{TOKEN_HEADER}: {token_value}")
    return ("\r\n".join(parts) + "\r\n\r\n").encode("utf-8")


def _read_http_header(connection: socket.socket, timeout: float) -> tuple[int, bytearray]:
    deadline = time.monotonic() + timeout
    buffer = bytearray()
    while time.monotonic() < deadline:
        try:
            chunk = connection.recv(65536)
        except socket.timeout:
            continue
        if not chunk:
            break
        buffer.extend(chunk)
        if len(buffer) > MAX_MJPEG_BUFFER_BYTES:
            raise ProbeError("MJPEG response exceeds buffer limit")
        separator = buffer.find(b"\r\n\r\n")
        if separator >= 0:
            status_line = bytes(buffer[:separator]).split(b"\r\n", 1)[0]
            match = re.match(rb"HTTP/\d(?:\.\d)?\s+(\d{3})\b", status_line)
            if match is None:
                raise ProbeError("MJPEG returned an invalid HTTP status line")
            del buffer[: separator + 4]
            return int(match.group(1)), buffer
    raise ProbeError("MJPEG did not return a complete HTTP header")


def read_mjpeg_status(
    host: str, port: int, *, token_value: str | None, request_timeout: float
) -> int:
    connection = socket.create_connection((host, int(port)), timeout=request_timeout)
    try:
        connection.settimeout(min(0.25, request_timeout))
        connection.sendall(_mjpeg_request(token_value))
        status, _ = _read_http_header(connection, request_timeout)
        return status
    finally:
        with contextlib.suppress(OSError):
            connection.close()


def read_mjpeg_frames(
    host: str,
    port: int,
    *,
    token: SecretToken,
    duration_seconds: float,
    request_timeout: float,
) -> list[bytes]:
    if duration_seconds <= 0:
        raise ProbeError("MJPEG duration must be positive")
    deadline = time.monotonic() + duration_seconds
    connection = socket.create_connection((host, int(port)), timeout=request_timeout)
    frames: list[bytes] = []
    try:
        connection.settimeout(min(0.25, request_timeout))
        connection.sendall(_mjpeg_request(token.reveal()))
        status, buffer = _read_http_header(connection, request_timeout)
        if status != 200:
            raise ProbeError(f"MJPEG returned HTTP {status}")
        while time.monotonic() < deadline:
            for frame in _extract_jpegs(buffer):
                decode_visual_frame(frame)
                frames.append(frame)
            try:
                chunk = connection.recv(65536)
            except socket.timeout:
                continue
            if not chunk:
                break
            buffer.extend(chunk)
            if len(buffer) > MAX_MJPEG_BUFFER_BYTES:
                raise ProbeError("MJPEG response exceeds buffer limit")
        for frame in _extract_jpegs(buffer):
            decode_visual_frame(frame)
            frames.append(frame)
    finally:
        with contextlib.suppress(OSError):
            connection.close()
    if not frames:
        raise ProbeError("MJPEG produced no valid JPEG frame")
    return frames


class MjpegSampler:
    """Maintain one authenticated post-session reader with bounded reconnects."""

    def __init__(
        self,
        host: str,
        port: int,
        token: SecretToken,
        request_timeout: float,
        max_reconnects: int = GATE_MAX_STREAM_RECONNECTS,
        stall_timeout: float = GATE_MAX_FRAME_GAP_SECONDS,
    ) -> None:
        self.host = host
        self.port = int(port)
        self.token = token
        self.request_timeout = float(request_timeout)
        self.max_reconnects = int(max_reconnects)
        self.stall_timeout = float(stall_timeout)
        if self.stall_timeout <= 0:
            raise ProbeError("MJPEG stall timeout must be positive")
        self._condition = threading.Condition()
        self._stop = threading.Event()
        self._thread: threading.Thread | None = None
        self._active_socket: socket.socket | None = None
        self._sequence = 0
        self._latest: bytes | None = None
        self._timestamps: list[float] = []
        self._invalid_frames = 0
        self._connections = 0
        self._reconnects = 0
        self._fatal_error: str | None = None

    @property
    def frame_count(self) -> int:
        with self._condition:
            return self._sequence

    @property
    def invalid_frame_count(self) -> int:
        with self._condition:
            return self._invalid_frames

    @property
    def reconnect_count(self) -> int:
        with self._condition:
            return self._reconnects

    @property
    def frame_timestamps(self) -> list[float]:
        with self._condition:
            return list(self._timestamps)

    def start(self) -> None:
        if self._thread is not None:
            return
        self._thread = threading.Thread(target=self._run, daemon=True)
        self._thread.start()

    def stop(self) -> None:
        self._stop.set()
        with self._condition:
            connection = self._active_socket
            self._condition.notify_all()
        if connection is not None:
            with contextlib.suppress(OSError):
                connection.shutdown(socket.SHUT_RDWR)
            with contextlib.suppress(OSError):
                connection.close()
        thread = self._thread
        if thread is not None:
            thread.join(timeout=2.0)
            if thread.is_alive():
                raise ProbeError("MJPEG sampler thread did not stop")
            self._thread = None

    def snapshot(self, timeout: float) -> tuple[int, bytes]:
        deadline = time.monotonic() + timeout
        with self._condition:
            while self._latest is None and not self._stop.is_set() and self._fatal_error is None:
                remaining = deadline - time.monotonic()
                if remaining <= 0:
                    break
                self._condition.wait(remaining)
            if self._latest is None:
                raise ProbeError(self._fatal_error or "MJPEG sampler produced no valid JPEG")
            return self._sequence, self._latest

    def wait_next(self, sequence: int, timeout: float) -> tuple[int, bytes]:
        deadline = time.monotonic() + timeout
        with self._condition:
            while self._sequence <= sequence and not self._stop.is_set():
                if self._fatal_error is not None:
                    raise ProbeError(self._fatal_error)
                remaining = deadline - time.monotonic()
                if remaining <= 0:
                    raise ProbeError("timed out waiting for a new MJPEG frame")
                self._condition.wait(remaining)
            if self._latest is None or self._sequence <= sequence:
                raise ProbeError("MJPEG sampler stopped before the next frame")
            return self._sequence, self._latest

    def maximum_frame_gap(self, start: float, end: float) -> float:
        timestamps = [stamp for stamp in self.frame_timestamps if start <= stamp <= end]
        points = [start, *timestamps, end]
        return max((right - left for left, right in zip(points, points[1:])), default=end - start)

    def assert_healthy(self) -> None:
        with self._condition:
            if self._fatal_error is not None:
                raise ProbeError(self._fatal_error)
            if self._invalid_frames:
                raise ProbeError("MJPEG sampler received an invalid JPEG frame")
            if self._reconnects > self.max_reconnects:
                raise ProbeError("MJPEG sampler exceeded its reconnect budget")

    def _publish(self, frame: bytes) -> None:
        try:
            decode_visual_frame(frame)
        except ProbeError:
            with self._condition:
                self._invalid_frames += 1
                self._condition.notify_all()
            return
        with self._condition:
            self._sequence += 1
            self._latest = frame
            self._timestamps.append(time.monotonic())
            self._condition.notify_all()

    def _record_fatal(self, message: str) -> None:
        with self._condition:
            self._fatal_error = message
            self._condition.notify_all()

    def _run(self) -> None:
        while not self._stop.is_set():
            with self._condition:
                if self._connections > 0:
                    self._reconnects += 1
                    if self._reconnects > self.max_reconnects:
                        self._fatal_error = "MJPEG sampler exceeded its reconnect budget"
                        self._condition.notify_all()
                        return
            connection: socket.socket | None = None
            try:
                connection = socket.create_connection(
                    (self.host, self.port), timeout=self.request_timeout
                )
                connection.settimeout(min(0.5, self.request_timeout))
                with self._condition:
                    self._active_socket = connection
                connection.sendall(_mjpeg_request(self.token.reveal(), "keep-alive"))
                status, buffer = _read_http_header(connection, self.request_timeout)
                if status != 200:
                    raise ProbeError(f"MJPEG sampler returned HTTP {status}")
                with self._condition:
                    self._connections += 1
                last_data = time.monotonic()
                while not self._stop.is_set():
                    for frame in _extract_jpegs(buffer):
                        self._publish(frame)
                    try:
                        chunk = connection.recv(65536)
                    except socket.timeout:
                        if time.monotonic() - last_data >= self.stall_timeout:
                            raise ProbeError("MJPEG sampler stalled")
                        continue
                    if not chunk:
                        break
                    last_data = time.monotonic()
                    buffer.extend(chunk)
                    if len(buffer) > MAX_MJPEG_BUFFER_BYTES:
                        raise ProbeError("MJPEG sampler exceeded its buffer limit")
            except Exception as exc:
                if not self._stop.is_set() and self._connections == 0:
                    self._record_fatal(f"MJPEG sampler could not establish a stream: {type(exc).__name__}")
                    return
            finally:
                with self._condition:
                    self._active_socket = None
                if connection is not None:
                    with contextlib.suppress(OSError):
                        connection.close()
            if not self._stop.is_set():
                time.sleep(0.05)


@dataclass(frozen=True)
class ProbeConfig:
    cold_launches: int = GATE_COLD_LAUNCHES
    tap_attempts: int = GATE_TAP_SUCCESSES
    swipe_attempts: int = GATE_SWIPE_SUCCESSES
    stream_seconds: float = GATE_STREAM_SECONDS
    request_timeout: float = 10.0
    port_close_timeout: float = 20.0
    action_settle_seconds: float = 0.35
    action_observation_seconds: float = 1.0
    control_interval_seconds: float = GATE_CONTROL_INTERVAL_SECONDS
    foreground_bundle: str = SETTINGS_BUNDLE_ID
    wait_for_trust: bool = False
    reuse_trusted_install: bool = False
    swipe_from_x: float = 187.0
    swipe_from_y: float = 500.0
    swipe_to_x: float = 187.0
    swipe_to_y: float = 180.0
    swipe_delay: float = 0.25

    def __post_init__(self) -> None:
        for name in ("cold_launches", "tap_attempts", "swipe_attempts"):
            if getattr(self, name) < 1:
                raise ProbeError(f"{name} must be at least 1")
        for name in (
            "stream_seconds",
            "request_timeout",
            "port_close_timeout",
            "action_observation_seconds",
            "control_interval_seconds",
        ):
            if getattr(self, name) <= 0:
                raise ProbeError(f"{name} must be positive")
        if self.action_settle_seconds < 0:
            raise ProbeError("action_settle_seconds must not be negative")
        if self.wait_for_trust and self.reuse_trusted_install:
            raise ProbeError(
                "wait_for_trust and reuse_trusted_install are mutually exclusive"
            )


def wait_for_manual_trust() -> None:
    """Pause after fresh-install so the user can approve the developer profile."""
    print(
        "Fresh install is complete. On this iPhone open Settings > General > "
        "VPN & Device Management, select the Apple Development profile, and "
        "tap Trust/Verify. Press Enter here after the approval is complete.",
        flush=True,
    )
    try:
        input()
    except EOFError as exc:
        raise ProbeError(
            "manual trust checkpoint requires an interactive terminal"
        ) from exc


def validate_live_config(config: ProbeConfig) -> None:
    if config.foreground_bundle != SETTINGS_BUNDLE_ID:
        raise ProbeError("live Gate C gesture surface must be Settings")
    requirements = {
        "cold_launches": GATE_COLD_LAUNCHES,
        "tap_attempts": GATE_TAP_SUCCESSES,
        "swipe_attempts": GATE_SWIPE_SUCCESSES,
        "stream_seconds": GATE_STREAM_SECONDS,
    }
    lowered = [name for name, minimum in requirements.items() if getattr(config, name) < minimum]
    if lowered:
        raise ProbeError("live Gate B/C configuration is below fixed acceptance thresholds")
    if config.control_interval_seconds > GATE_CONTROL_INTERVAL_SECONDS:
        raise ProbeError("live Gate C control interval exceeds the fixed cadence")


class DeviceAdapter(Protocol):
    evidence_environment: str

    @property
    def control_address(self) -> tuple[str, int]: ...

    @property
    def mjpeg_address(self) -> tuple[str, int]: ...

    def prepare_candidate(
        self,
        artifact: CandidateArtifact,
        timeout: float,
        *,
        reuse_trusted_install: bool = False,
    ) -> dict[str, Any]: ...

    def terminate_candidate(self) -> int | None: ...

    def wait_candidate_ports_closed(self, timeout: float) -> bool: ...

    def launch_candidate(self, environment: dict[str, str]) -> int: ...

    def candidate_process_id(self) -> int | None: ...

    def start_control_relay(self) -> None: ...

    def start_mjpeg_relay(self) -> None: ...

    def foreground(self, bundle_id: str) -> None: ...

    def foreground_candidate_without_restart(self) -> int: ...

    def stop_relays(self) -> None: ...


def empty_measurements() -> dict[str, Any]:
    return {
        "candidateFreshInstalled": False,
        "installedIdentityMatch": False,
        "manualTrustPauseRequested": False,
        "manualTrustPauseCompleted": False,
        "cleanupVerified": False,
        "coldLaunchSuccesses": 0,
        "coldLaunchProcessWitnesses": [],
        "sessionFingerprints": [],
        "sessionCommandSuccesses": 0,
        "coldLaunchOrder": [],
        "statusIdentitySuccesses": 0,
        "authStatusesByLaunch": [],
        "mjpegAuthStatusesByLaunch": [],
        "firstJpegCount": 0,
        "gestureControlSamples": 0,
        "gestureControlFrames": 0,
        "settingsActiveChecks": 0,
        "tapCausalChanges": 0,
        "tapSemanticToggles": 0,
        "swipeCausalChanges": 0,
        "swipeForwardCausalChanges": 0,
        "swipeReverseCausalChanges": 0,
        "minimumTapVisualMargin": None,
        "minimumSwipeVisualMargin": None,
        "streamFrames": 0,
        "streamInvalidFrames": 0,
        "streamReconnects": 0,
        "streamMaxFrameGapSeconds": None,
        "streamObservedSeconds": 0.0,
        "streamControlChecks": 0,
        "streamSessionChecks": 0,
        "streamMaxControlCycleSeconds": None,
        "streamMaxControlCompletionGapSeconds": None,
        "streamMaxControlScheduleLatenessSeconds": None,
        **{field: False for field in TOKEN_GATE_FIELDS},
        "clipboardAgentForegroundPidStable": False,
        "clipboardAgentForegroundIdentityVerified": False,
        "clipboardByteExact": 0,
        "unicodeKeysReadBack": False,
        "failures": [],
    }


def _environment_gate(environment: str, checks: tuple[bool, ...], failures: Any) -> str:
    if environment != LIVE_ENVIRONMENT:
        if environment == SUPPLEMENTAL_ENVIRONMENT:
            return "SUPPLEMENTAL_ONLY"
        return FIXTURE_ENVIRONMENT
    return "PASS" if all(checks) and not failures else "FAIL"


def evaluate_gate_b(measurements: dict[str, Any], environment: str) -> str:
    orders = measurements.get("coldLaunchOrder", [])
    sessions = measurements.get("sessionFingerprints", [])
    successes = measurements.get("coldLaunchSuccesses", 0)
    expected_auth = {"missing": 401, "wrong": 401, "correct": 200}
    auth_by_launch = measurements.get("authStatusesByLaunch", [])
    mjpeg_auth_by_launch = measurements.get("mjpegAuthStatusesByLaunch", [])
    process_witnesses = measurements.get("coldLaunchProcessWitnesses", [])
    valid_process_witnesses = (
        isinstance(process_witnesses, list)
        and len(process_witnesses) == successes
        and all(
            isinstance(witness, dict)
            and set(witness)
            == {
                "oldProcessObserved",
                "processAbsentBeforeLaunch",
                "newProcessVerified",
                "newPidFingerprint",
            }
            and type(witness.get("oldProcessObserved")) is bool
            and (index == 0 or witness.get("oldProcessObserved") is True)
            and witness.get("processAbsentBeforeLaunch") is True
            and witness.get("newProcessVerified") is True
            and isinstance(witness.get("newPidFingerprint"), str)
            and len(witness["newPidFingerprint"]) == 16
            for index, witness in enumerate(process_witnesses)
        )
        and len({witness["newPidFingerprint"] for witness in process_witnesses})
        == len(process_witnesses)
    )
    checks = (
        measurements.get("candidateFreshInstalled") is True,
        measurements.get("installedIdentityMatch") is True,
        measurements.get("cleanupVerified") is True,
        successes >= GATE_COLD_LAUNCHES,
        valid_process_witnesses,
        len(sessions) == successes,
        len(set(sessions)) == len(sessions),
        measurements.get("sessionCommandSuccesses", 0) == successes,
        measurements.get("statusIdentitySuccesses", 0) == successes,
        len(orders) == successes,
        all(order == EXPECTED_ORDER for order in orders),
        len(auth_by_launch) == successes,
        all(matrix == expected_auth for matrix in auth_by_launch),
        len(mjpeg_auth_by_launch) == successes,
        all(matrix == expected_auth for matrix in mjpeg_auth_by_launch),
        measurements.get("firstJpegCount", 0) == successes,
        all(measurements.get(field) is True for field in TOKEN_GATE_FIELDS),
    )
    return _environment_gate(environment, checks, measurements.get("failures"))


def evaluate_gate_c(measurements: dict[str, Any], environment: str) -> str:
    minimum_frames = math.ceil(GATE_STREAM_SECONDS * GATE_MIN_STREAM_FPS)
    max_gap = measurements.get("streamMaxFrameGapSeconds")
    gesture_attempts = GATE_TAP_SUCCESSES + GATE_SWIPE_SUCCESSES

    def at_most(key: str, maximum: float) -> bool:
        value = measurements.get(key)
        return not isinstance(value, bool) and isinstance(value, (int, float)) and value <= maximum

    checks = (
        measurements.get("cleanupVerified") is True,
        measurements.get("gestureControlSamples", 0) >= gesture_attempts,
        measurements.get("gestureControlFrames", 0)
        >= gesture_attempts * GATE_CONTROL_FRAME_SAMPLES_PER_ACTION,
        measurements.get("settingsActiveChecks", 0) >= gesture_attempts * 2,
        measurements.get("tapCausalChanges", 0) >= GATE_TAP_SUCCESSES,
        measurements.get("tapSemanticToggles", 0) >= GATE_TAP_SUCCESSES,
        measurements.get("swipeCausalChanges", 0) >= GATE_SWIPE_SUCCESSES,
        measurements.get("swipeForwardCausalChanges", 0) >= GATE_SWIPE_SUCCESSES // 2,
        measurements.get("swipeReverseCausalChanges", 0) >= GATE_SWIPE_SUCCESSES // 2,
        measurements.get("streamFrames", 0) >= minimum_frames,
        measurements.get("streamInvalidFrames", 0) == 0,
        measurements.get("streamReconnects", GATE_MAX_STREAM_RECONNECTS + 1)
        <= GATE_MAX_STREAM_RECONNECTS,
        isinstance(max_gap, (int, float)) and max_gap <= GATE_MAX_FRAME_GAP_SECONDS,
        measurements.get("streamObservedSeconds", 0.0) + 0.001 >= GATE_STREAM_SECONDS,
        measurements.get("streamControlChecks", 0) >= GATE_MIN_CONTROL_CHECKS,
        measurements.get("streamSessionChecks", 0) >= GATE_MIN_CONTROL_CHECKS,
        at_most("streamMaxControlCycleSeconds", GATE_MAX_CONTROL_CYCLE_SECONDS),
        at_most(
            "streamMaxControlCompletionGapSeconds",
            GATE_MAX_CONTROL_COMPLETION_GAP_SECONDS,
        ),
        at_most(
            "streamMaxControlScheduleLatenessSeconds",
            GATE_MAX_CONTROL_SCHEDULE_LATENESS_SECONDS,
        ),
        measurements.get("clipboardAgentForegroundPidStable") is True,
        measurements.get("clipboardAgentForegroundIdentityVerified") is True,
        measurements.get("clipboardByteExact", 0) >= 2,
        measurements.get("unicodeKeysReadBack") is True,
    )
    return _environment_gate(environment, checks, measurements.get("failures"))


def evaluate_gate(measurements: dict[str, Any], environment: str) -> str:
    gate_b = evaluate_gate_b(measurements, environment)
    gate_c = evaluate_gate_c(measurements, environment)
    if environment != LIVE_ENVIRONMENT:
        if environment == SUPPLEMENTAL_ENVIRONMENT:
            return "SUPPLEMENTAL_ONLY"
        return FIXTURE_ENVIRONMENT
    return "PASS" if gate_b == "PASS" and gate_c == "PASS" else "FAIL"


_CLASSIC_UDID = re.compile(r"(?i)\b[0-9a-f]{40}\b")
_MODERN_UDID = re.compile(r"(?i)\b[0-9a-f]{8}-[0-9a-f]{16}\b")
_WINDOWS_HOME = re.compile(r"(?i)[A-Z]:\\Users\\[^\\\s]+")
_UNIX_HOME = re.compile(r"/Users/[^/\s]+")


def sanitize_message(value: str, token: SecretToken, extra_secrets: tuple[str, ...] = ()) -> str:
    result = value
    for secret in sorted(token.variants() | {item for item in extra_secrets if item}, key=len, reverse=True):
        result = result.replace(secret, SecretToken.redacted())
    result = _CLASSIC_UDID.sub("<redacted-device-id>", result)
    result = _MODERN_UDID.sub("<redacted-device-id>", result)
    result = _WINDOWS_HOME.sub("<home>", result)
    result = _UNIX_HOME.sub("<home>", result)
    return result


def _report_strings(value: Any):
    if isinstance(value, str):
        yield value
    elif isinstance(value, dict):
        for key, child in value.items():
            yield str(key)
            yield from _report_strings(child)
    elif isinstance(value, (list, tuple)):
        for child in value:
            yield from _report_strings(child)


def serialize_report(report: dict[str, Any], token: SecretToken) -> str:
    for value in _report_strings(report):
        if any(variant and variant in value for variant in token.variants()):
            raise ProbeError("probe report contains an agent-token representation")
        if _CLASSIC_UDID.search(value) or _MODERN_UDID.search(value):
            raise ProbeError("probe report contains a device identifier")
        if _WINDOWS_HOME.search(value) or _UNIX_HOME.search(value):
            raise ProbeError("probe report contains a user home path")
    serialized = json.dumps(report, ensure_ascii=True, indent=2, sort_keys=True) + "\n"
    if any(variant and variant in serialized for variant in token.variants()):
        raise ProbeError("probe report contains an agent-token representation")
    if _CLASSIC_UDID.search(serialized) or _MODERN_UDID.search(serialized):
        raise ProbeError("probe report contains a device identifier")
    if _WINDOWS_HOME.search(serialized) or _UNIX_HOME.search(serialized):
        raise ProbeError("probe report contains a user home path")
    return serialized


def attest_report_token_scan(report: dict[str, Any], token: SecretToken) -> None:
    measurements = report.get("measurements")
    environment = report.get("environment")
    if not isinstance(measurements, dict) or not isinstance(environment, str):
        raise ProbeError("probe report cannot be finalized")
    measurements["reportTokenScanClean"] = False
    report["gateB"] = evaluate_gate_b(measurements, environment)
    report["gateC"] = evaluate_gate_c(measurements, environment)
    report["gateStatus"] = evaluate_gate(measurements, environment)
    serialize_report(report, token)
    measurements["reportTokenScanClean"] = True
    report["gateB"] = evaluate_gate_b(measurements, environment)
    report["gateC"] = evaluate_gate_c(measurements, environment)
    report["gateStatus"] = evaluate_gate(measurements, environment)
    serialize_report(report, token)


def record_control_cadence_sample(
    measurements: dict[str, Any],
    *,
    scheduled: float,
    started: float,
    completed: float,
    previous_completion: float,
) -> float:
    if started < scheduled or completed < started or previous_completion > completed:
        raise ProbeError("control cadence timestamps are inconsistent")
    cycle = completed - started
    completion_gap = completed - previous_completion
    schedule_lateness = started - scheduled
    values = {
        "streamMaxControlCycleSeconds": cycle,
        "streamMaxControlCompletionGapSeconds": completion_gap,
        "streamMaxControlScheduleLatenessSeconds": schedule_lateness,
    }
    for key, value in values.items():
        previous = measurements.get(key)
        maximum = value if previous is None else max(float(previous), value)
        measurements[key] = round(maximum, 3)
    if cycle > GATE_MAX_CONTROL_CYCLE_SECONDS:
        raise ProbeError("stream control check exceeded its cycle deadline")
    if completion_gap > GATE_MAX_CONTROL_COMPLETION_GAP_SECONDS:
        raise ProbeError("stream control completion gap exceeded its deadline")
    if schedule_lateness > GATE_MAX_CONTROL_SCHEDULE_LATENESS_SECONDS:
        raise ProbeError("stream control check started after its schedule deadline")
    return completed


class ProbeRunner:
    def __init__(
        self,
        *,
        adapter: DeviceAdapter,
        artifact: CandidateArtifact,
        config: ProbeConfig,
        token: SecretToken,
        token_preflight: dict[str, bool] | None = None,
    ) -> None:
        self.adapter = adapter
        self.artifact = artifact
        self.config = config
        self.token = token
        self.token_preflight = dict(token_preflight or {})
        self._settings_post_action_settle_seconds = 0.0

    def _validate_identity(self, result: HttpResult) -> None:
        if result.status != 200 or not isinstance(result.payload, dict):
            raise ProbeError("protected health did not return HTTP 200 JSON")
        value = result.payload.get("value")
        artifact = getattr(self, "artifact", None)
        features = list(artifact.features) if artifact is not None else list(EXPECTED_FEATURES)
        expected = {
            "agentVersion": EXPECTED_AGENT_VERSION,
            "protocolVersion": EXPECTED_PROTOCOL_VERSION,
            "features": features,
            "logicalWidth": 375,
            "logicalHeight": 667,
            "state": "ready",
        }
        if value != expected:
            raise ProbeError("protected health identity does not match ready protocol v2")

    def _validate_status_identity(self, result: HttpResult) -> None:
        if result.status != 200 or not isinstance(result.payload, dict):
            raise ProbeError("discovery status did not return HTTP 200 JSON")
        value = result.payload.get("value")
        identity = value.get("riviuAgent") if isinstance(value, dict) else None
        artifact = getattr(self, "artifact", None)
        features = list(artifact.features) if artifact is not None else list(EXPECTED_FEATURES)
        expected = {
            "agentVersion": EXPECTED_AGENT_VERSION,
            "protocolVersion": EXPECTED_PROTOCOL_VERSION,
            "features": features,
            "logicalWidth": 375,
            "logicalHeight": 667,
            "state": "ready",
        }
        if identity != expected:
            raise ProbeError("discovery status identity does not match ready protocol v2")

    @staticmethod
    def _element_region(rect: dict[str, float]) -> tuple[float, float, float, float]:
        left = rect["x"]
        top = rect["y"]
        right = left + rect["width"]
        bottom = top + rect["height"]
        if left < 0 or top < 0 or right > 375 or bottom > 667:
            raise ProbeError("gesture target is outside the logical screen")
        padding = 12.0
        return (
            max(0.0, left - padding) / 375.0,
            max(0.0, top - padding) / 667.0,
            min(375.0, right + padding) / 375.0,
            min(667.0, bottom + padding) / 667.0,
        )

    def _measure_visual_action(
        self,
        sampler: MjpegSampler,
        action: Callable[[], None],
        region: tuple[float, float, float, float],
        measurements: dict[str, Any],
    ) -> tuple[bool, float]:
        cursor, start_frame = sampler.snapshot(self.config.request_timeout)
        control_frames = [decode_visual_frame(start_frame, region)]
        for _ in range(GATE_ADDITIONAL_CONTROL_FRAMES_PER_ACTION):
            cursor, frame = sampler.wait_next(cursor, self.config.request_timeout)
            control_frames.append(decode_visual_frame(frame, region))
        idle_distance = max(
            visual_distance(left, right)
            for index, left in enumerate(control_frames)
            for right in control_frames[index + 1 :]
        )
        measurements["gestureControlSamples"] += 1
        measurements["gestureControlFrames"] += len(control_frames)
        action()
        if self.config.action_settle_seconds:
            time.sleep(self.config.action_settle_seconds)
        deadline = time.monotonic() + self.config.action_observation_seconds
        baseline = control_frames[-1]
        maximum = 0.0
        observed = 0
        while time.monotonic() < deadline:
            remaining = deadline - time.monotonic()
            try:
                cursor, candidate = sampler.wait_next(cursor, min(self.config.request_timeout, remaining))
            except ProbeError:
                break
            maximum = max(maximum, visual_distance(baseline, decode_visual_frame(candidate, region)))
            observed += 1
            if maximum >= max(GATE_MIN_VISUAL_DELTA, idle_distance + GATE_VISUAL_CAUSAL_MARGIN):
                break
        if observed == 0:
            raise ProbeError("gesture probe observed no post-action JPEG")
        margin = maximum - idle_distance
        return maximum >= max(GATE_MIN_VISUAL_DELTA, idle_distance + GATE_VISUAL_CAUSAL_MARGIN), margin

    @staticmethod
    def _record_minimum(measurements: dict[str, Any], key: str, value: float) -> None:
        previous = measurements[key]
        measurements[key] = round(value if previous is None else min(previous, value), 3)

    def _ensure_settings_root(self, client: ControlClient, session_id: str) -> None:
        """Return Settings to its root page before querying a switch.

        iOS restores the last Settings page after a cold launch. Trust/profile
        pages have no switch, so a bounded native back tap makes the gesture
        surface deterministic without weakening the action thresholds.
        """
        client.require_active_bundle(session_id, self.config.foreground_bundle)
        for _ in range(SETTINGS_ROOT_RETRIES):
            try:
                client.find_element(session_id, "XCUIElementTypeSwitch")
                return
            except ProbeError as error:
                message = str(error)
                if "returned 404" not in message or "/element" not in message:
                    raise
                client.require_ok("POST", "/wda/tap", SETTINGS_BACK_TAP)
                time.sleep(0.75)
        raise ProbeError("Settings root did not expose an XCUIElementTypeSwitch")

    def _measure_gestures(
        self,
        client: ControlClient,
        session_id: str,
        sampler: MjpegSampler,
        measurements: dict[str, Any],
    ) -> None:
        for _ in range(self.config.tap_attempts):
            client.require_active_bundle(session_id, self.config.foreground_bundle)
            measurements["settingsActiveChecks"] += 1
            switch_id = client.find_element(session_id, "XCUIElementTypeSwitch")
            rect = client.read_element_rect(session_id, switch_id)
            before_value = client.read_element_attribute(session_id, switch_id, "value")
            tap_x = rect["x"] + rect["width"] / 2.0
            tap_y = rect["y"] + rect["height"] / 2.0
            changed, margin = self._measure_visual_action(
                sampler,
                lambda: client.require_ok(
                    "POST", "/wda/tap", {"x": tap_x, "y": tap_y}
                ),
                self._element_region(rect),
                measurements,
            )
            client.require_active_bundle(session_id, self.config.foreground_bundle)
            measurements["settingsActiveChecks"] += 1
            after_value = client.read_element_attribute(session_id, switch_id, "value")
            self._record_minimum(measurements, "minimumTapVisualMargin", margin)
            if changed:
                measurements["tapCausalChanges"] += 1
            if {before_value, after_value} == {"0", "1"}:
                measurements["tapSemanticToggles"] += 1
            if self._settings_post_action_settle_seconds:
                time.sleep(self._settings_post_action_settle_seconds)

        for index in range(self.config.swipe_attempts):
            client.require_active_bundle(session_id, self.config.foreground_bundle)
            measurements["settingsActiveChecks"] += 1
            reverse = index % 2 == 1
            from_x = self.config.swipe_to_x if reverse else self.config.swipe_from_x
            from_y = self.config.swipe_to_y if reverse else self.config.swipe_from_y
            to_x = self.config.swipe_from_x if reverse else self.config.swipe_to_x
            to_y = self.config.swipe_from_y if reverse else self.config.swipe_to_y
            changed, margin = self._measure_visual_action(
                sampler,
                lambda: client.require_ok(
                    "POST",
                    "/wda/swipe",
                    {
                        "fromX": from_x,
                        "fromY": from_y,
                        "toX": to_x,
                        "toY": to_y,
                        "delay": self.config.swipe_delay,
                    },
                ),
                CONTENT_REGION,
                measurements,
            )
            client.require_active_bundle(session_id, self.config.foreground_bundle)
            measurements["settingsActiveChecks"] += 1
            self._record_minimum(measurements, "minimumSwipeVisualMargin", margin)
            if changed:
                measurements["swipeCausalChanges"] += 1
                direction_key = (
                    "swipeReverseCausalChanges" if reverse else "swipeForwardCausalChanges"
                )
                measurements[direction_key] += 1
            if self._settings_post_action_settle_seconds:
                time.sleep(self._settings_post_action_settle_seconds)

    def _wait_for_active_bundle(
        self,
        client: ControlClient,
        session_id: str,
        bundle_id: str,
        expected_pid: int,
    ) -> int:
        deadline = time.monotonic() + ACTIVE_APP_SETTLE_TIMEOUT_SECONDS
        last_error: ProbeError | None = None
        while time.monotonic() < deadline:
            try:
                active_pid = client.require_active_bundle(session_id, bundle_id)
                if active_pid == expected_pid:
                    return active_pid
                last_error = ProbeError(
                    "candidate foreground identity returned an unexpected PID"
                )
            except ProbeError as error:
                if "active application does not match" not in str(error):
                    raise
                last_error = error
            time.sleep(ACTIVE_APP_SETTLE_POLL_SECONDS)
        if last_error is not None:
            raise last_error
        raise ProbeError("candidate foreground identity did not settle")

    def _ensure_settings_search(self, client: ControlClient, session_id: str) -> None:
        """Reveal the iOS 16 Settings search field before Unicode read-back."""
        client.require_active_bundle(session_id, self.config.foreground_bundle)
        for _ in range(SETTINGS_SEARCH_RETRIES):
            try:
                client.find_element(session_id, "XCUIElementTypeSearchField")
                return
            except ProbeError as error:
                message = str(error)
                if "returned 404" not in message or "/element" not in message:
                    raise
                client.require_ok("POST", "/wda/swipe", SETTINGS_SEARCH_PULL_DOWN)
                time.sleep(0.75)
        raise ProbeError("Settings search field did not appear after pull-down")

    @staticmethod
    def _measure_clipboard(client: ControlClient, measurements: dict[str, Any]) -> None:
        samples = (
            b"Riviu clipboard ASCII probe",
            "Riviu clipboard Unicode \u0111\u01b0\u1ee3c \U0001f525".encode("utf-8"),
        )
        for sample in samples:
            client.set_clipboard(sample)
            if client.get_clipboard() == sample:
                measurements["clipboardByteExact"] += 1

    @staticmethod
    def _measure_unicode_keys(
        client: ControlClient, session_id: str, measurements: dict[str, Any]
    ) -> None:
        sample = "Riviu Unicode \u0111\u01b0\u1ee3c \U0001f525"
        element_id = client.find_element(session_id, "XCUIElementTypeSearchField")
        client.require_ok("POST", f"/session/{session_id}/element/{element_id}/click")
        client.require_ok("POST", f"/session/{session_id}/element/{element_id}/clear")
        client.require_ok(
            "POST", f"/session/{session_id}/wda/keys", {"value": list(sample)}
        )
        measurements["unicodeKeysReadBack"] = (
            client.read_element_text(session_id, element_id) == sample
        )

    def _monitor_stream(
        self,
        client: ControlClient,
        session_id: str,
        sampler: MjpegSampler,
        measurements: dict[str, Any],
    ) -> None:
        started = time.monotonic()
        deadline = started + self.config.stream_seconds
        initial_frames = sampler.frame_count
        initial_invalid = sampler.invalid_frame_count
        next_check = started
        previous_completion = started
        while time.monotonic() < deadline:
            now = time.monotonic()
            if now >= next_check:
                check_started = now
                self._validate_identity(client.require_ok("GET", "/riviu/health"))
                measurements["streamControlChecks"] += 1
                client.require_session(session_id)
                measurements["streamSessionChecks"] += 1
                previous_completion = record_control_cadence_sample(
                    measurements,
                    scheduled=next_check,
                    started=check_started,
                    completed=time.monotonic(),
                    previous_completion=previous_completion,
                )
                next_check += self.config.control_interval_seconds
            time.sleep(min(0.1, max(0.0, deadline - time.monotonic())))
        ended = time.monotonic()
        final_gap = ended - previous_completion
        previous_gap = measurements["streamMaxControlCompletionGapSeconds"]
        measurements["streamMaxControlCompletionGapSeconds"] = round(
            final_gap if previous_gap is None else max(float(previous_gap), final_gap), 3
        )
        if final_gap > GATE_MAX_CONTROL_COMPLETION_GAP_SECONDS:
            raise ProbeError("stream control completion gap exceeded at stream end")
        sampler.assert_healthy()
        measurements["streamObservedSeconds"] = round(ended - started, 3)
        measurements["streamFrames"] = sampler.frame_count - initial_frames
        measurements["streamInvalidFrames"] = sampler.invalid_frame_count - initial_invalid
        measurements["streamReconnects"] = sampler.reconnect_count
        measurements["streamMaxFrameGapSeconds"] = round(
            sampler.maximum_frame_gap(started, ended), 3
        )

    def _mjpeg_auth_matrix(self, sampler: MjpegSampler) -> dict[str, int]:
        host, port = self.adapter.mjpeg_address
        wrong = "x" * 32
        if wrong == self.token.reveal():
            wrong = "y" * 32
        statuses = {
            "missing": read_mjpeg_status(
                host, port, token_value=None, request_timeout=self.config.request_timeout
            ),
            "wrong": read_mjpeg_status(
                host, port, token_value=wrong, request_timeout=self.config.request_timeout
            ),
        }
        sampler.start()
        sampler.snapshot(self.config.request_timeout)
        statuses["correct"] = 200
        return statuses

    def run(self) -> dict[str, Any]:
        environment = getattr(self.adapter, "evidence_environment", FIXTURE_ENVIRONMENT)
        if environment == LIVE_ENVIRONMENT:
            validate_live_config(self.config)
        self._settings_post_action_settle_seconds = (
            SETTINGS_POST_ACTION_SETTLE_SECONDS if environment == LIVE_ENVIRONMENT else 0.0
        )
        measurements = empty_measurements()
        if self.token_preflight:
            if set(self.token_preflight) != set(TOKEN_PREFLIGHT_FIELDS) or not all(
                type(value) is bool for value in self.token_preflight.values()
            ):
                raise ProbeError("token preflight evidence has an invalid shape")
            measurements.update(self.token_preflight)
        device_evidence: dict[str, Any] = {}
        client: ControlClient | None = None
        active_session: str | None = None
        sampler: MjpegSampler | None = None
        previous_pid: int | None = None
        try:
            device_evidence = self.adapter.prepare_candidate(
                self.artifact,
                self.config.port_close_timeout,
                reuse_trusted_install=self.config.reuse_trusted_install,
            )
            measurements["candidateFreshInstalled"] = device_evidence.get("freshInstall") is True
            measurements["installedIdentityMatch"] = device_evidence.get("identityMatch") is True
            measurements["manualTrustPauseRequested"] = self.config.wait_for_trust
            if self.config.wait_for_trust:
                wait_for_manual_trust()
                measurements["manualTrustPauseCompleted"] = True
            for index in range(self.config.cold_launches):
                self.adapter.stop_relays()
                old_pid = self.adapter.terminate_candidate()
                if not self.adapter.wait_candidate_ports_closed(self.config.port_close_timeout):
                    raise ProbeError(
                        "candidate process or device ports remained active after terminate"
                    )
                if previous_pid is not None and old_pid != previous_pid:
                    raise ProbeError(
                        "candidate PID changed before the next cold launch"
                    )
                launch_environment = {
                    "USE_PORT": str(CONTROL_DEVICE_PORT),
                    "MJPEG_SERVER_PORT": str(MJPEG_DEVICE_PORT),
                    "USE_IP": "127.0.0.1",
                    "WDA_PRODUCT_BUNDLE_IDENTIFIER": self.artifact.bundle_id,
                    TOKEN_ENVIRONMENT: self.token.reveal(),
                }
                if "text" in self.artifact.features:
                    launch_environment["RIVIU_AGENT_TEXT_CAPABLE"] = "1"
                if "pushMedia" in self.artifact.features:
                    launch_environment["RIVIU_AGENT_MEDIA_CAPABLE"] = "1"
                new_pid = self.adapter.launch_candidate(launch_environment)
                if isinstance(new_pid, bool) or not isinstance(new_pid, int) or new_pid <= 0:
                    raise ProbeError("candidate launch omitted a verified process ID")
                order = ["launch"]
                self.adapter.start_control_relay()
                client = ControlClient(
                    *self.adapter.control_address, self.token, self.config.request_timeout
                )
                status = client.request("GET", "/status", auth="missing")
                self._validate_status_identity(status)
                measurements["statusIdentitySuccesses"] += 1
                measurements["authStatusesByLaunch"].append({
                    "missing": client.request("GET", "/riviu/health", auth="missing").status,
                    "wrong": client.request("GET", "/riviu/health", auth="wrong").status,
                    "correct": client.request("GET", "/riviu/health", auth="correct").status,
                })
                self._validate_identity(client.require_ok("GET", "/riviu/health"))
                order.append("health")

                self.adapter.foreground(self.config.foreground_bundle)
                if environment == LIVE_ENVIRONMENT:
                    # Settings restores its last page; let XCTest publish a fresh
                    # accessibility snapshot before the element probe begins.
                    time.sleep(SETTINGS_FOREGROUND_SETTLE_SECONDS)
                order.append("foreground")
                active_session = client.create_session()
                client.require_session(active_session)
                measurements["sessionFingerprints"].append(_sha256_text(active_session)[:16])
                measurements["sessionCommandSuccesses"] += 1
                order.append("session")

                self.adapter.start_mjpeg_relay()
                sampler = MjpegSampler(
                    *self.adapter.mjpeg_address,
                    self.token,
                    self.config.request_timeout,
                    GATE_MAX_STREAM_RECONNECTS,
                )
                measurements["mjpegAuthStatusesByLaunch"].append(
                    self._mjpeg_auth_matrix(sampler)
                )
                measurements["firstJpegCount"] += 1
                order.append("mjpeg")
                observed_pid = self.adapter.candidate_process_id()
                if (
                    isinstance(observed_pid, bool)
                    or not isinstance(observed_pid, int)
                    or observed_pid != new_pid
                ):
                    raise ProbeError(
                        "candidate PID changed before cold-launch readiness completed"
                    )
                measurements["coldLaunchProcessWitnesses"].append(
                    {
                        "oldProcessObserved": old_pid is not None,
                        "processAbsentBeforeLaunch": True,
                        "newProcessVerified": observed_pid == new_pid,
                        "newPidFingerprint": _sha256_text(str(new_pid))[:16],
                    }
                )
                measurements["coldLaunchOrder"].append(order)
                measurements["coldLaunchSuccesses"] += 1
                previous_pid = new_pid

                if index == self.config.cold_launches - 1:
                    if environment in {LIVE_ENVIRONMENT, SUPPLEMENTAL_ENVIRONMENT}:
                        self._ensure_settings_root(client, active_session)
                    self._measure_gestures(client, active_session, sampler, measurements)
                    if environment in {LIVE_ENVIRONMENT, SUPPLEMENTAL_ENVIRONMENT}:
                        self._ensure_settings_search(client, active_session)
                    self._measure_unicode_keys(client, active_session, measurements)
                    self._monitor_stream(client, active_session, sampler, measurements)
                    candidate_pid = self.adapter.foreground_candidate_without_restart()
                    if (
                        isinstance(candidate_pid, bool)
                        or not isinstance(candidate_pid, int)
                        or candidate_pid <= 0
                    ):
                        raise ProbeError("candidate foreground activation omitted its stable PID")
                    measurements["clipboardAgentForegroundPidStable"] = True
                    active_pid = self._wait_for_active_bundle(
                        client,
                        active_session,
                        self.artifact.bundle_id,
                        candidate_pid,
                    )
                    if active_pid != candidate_pid:
                        raise ProbeError(
                            "candidate foreground identity does not match its stable PID"
                        )
                    measurements["clipboardAgentForegroundIdentityVerified"] = True
                    self._measure_clipboard(client, measurements)

                client.require_ok("DELETE", f"/session/{active_session}")
                active_session = None
                sampler.stop()
                sampler = None
                self.adapter.stop_relays()
                client = None
        except Exception as exc:
            measurements["failures"].append(
                sanitize_message(str(exc), self.token, (self.artifact.bundle_id,))
            )
        finally:
            cleanup_errors: list[str] = []
            if client is not None and active_session is not None:
                try:
                    client.require_ok("DELETE", f"/session/{active_session}")
                except Exception as exc:
                    cleanup_errors.append(type(exc).__name__)
            if sampler is not None:
                try:
                    sampler.stop()
                except Exception as exc:
                    cleanup_errors.append(type(exc).__name__)
            try:
                self.adapter.stop_relays()
            except Exception as exc:
                cleanup_errors.append(type(exc).__name__)
            try:
                terminated_pid = self.adapter.terminate_candidate()
                if previous_pid is not None and terminated_pid != previous_pid:
                    cleanup_errors.append("candidate PID changed before final cleanup")
                if not self.adapter.wait_candidate_ports_closed(self.config.port_close_timeout):
                    cleanup_errors.append("ports remained open")
            except Exception as exc:
                cleanup_errors.append(type(exc).__name__)
            if cleanup_errors:
                measurements["failures"].append("cleanup verification failed")
            else:
                measurements["cleanupVerified"] = True

        report = {
            "schemaVersion": 2,
            "generatedAt": datetime.now(timezone.utc).isoformat(),
            "environment": environment,
            "gateB": evaluate_gate_b(measurements, environment),
            "gateC": evaluate_gate_c(measurements, environment),
            "gateStatus": evaluate_gate(measurements, environment),
            "candidate": self.artifact.evidence(),
            "device": device_evidence,
            "measurements": measurements,
            "requirements": {
                "coldLaunches": GATE_COLD_LAUNCHES,
                "tapCausalSuccesses": GATE_TAP_SUCCESSES,
                "swipeCausalSuccesses": GATE_SWIPE_SUCCESSES,
                "streamSeconds": GATE_STREAM_SECONDS,
                "minimumStreamFps": GATE_MIN_STREAM_FPS,
                "maximumFrameGapSeconds": GATE_MAX_FRAME_GAP_SECONDS,
                "maximumStreamReconnects": GATE_MAX_STREAM_RECONNECTS,
                "minimumControlChecks": GATE_MIN_CONTROL_CHECKS,
                "controlIntervalSeconds": GATE_CONTROL_INTERVAL_SECONDS,
                "maximumControlCycleSeconds": GATE_MAX_CONTROL_CYCLE_SECONDS,
                "maximumControlCompletionGapSeconds": GATE_MAX_CONTROL_COMPLETION_GAP_SECONDS,
                "maximumControlScheduleLatenessSeconds": GATE_MAX_CONTROL_SCHEDULE_LATENESS_SECONDS,
                "additionalControlFramesPerAction": GATE_ADDITIONAL_CONTROL_FRAMES_PER_ACTION,
                "controlFrameSamplesPerAction": GATE_CONTROL_FRAME_SAMPLES_PER_ACTION,
                "visualEvidence": {
                    "surface": "Settings",
                    "activeBundleCheckedBeforeAndAfter": True,
                    "tapSemanticTarget": "XCUIElementTypeSwitch",
                    "metric": "meanAbsoluteLumaDelta32x32",
                    "control": "maximum pairwise delta across preceding no-action frames",
                    "minimumDelta": GATE_MIN_VISUAL_DELTA,
                    "causalMargin": GATE_VISUAL_CAUSAL_MARGIN,
                },
            },
        }
        attest_report_token_scan(report, self.token)
        return report


def _free_local_port() -> int:
    listener = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    try:
        listener.bind(("127.0.0.1", 0))
        return int(listener.getsockname()[1])
    finally:
        listener.close()


class UsbmuxRelay:
    def __init__(self, udid: str, device_port: int) -> None:
        self.udid = udid
        self.device_port = int(device_port)
        self.local_port = _free_local_port()
        self._ready = threading.Event()
        self._stop = threading.Event()
        self._error: Exception | None = None
        self._thread: threading.Thread | None = None

    def start(self, timeout: float = 10.0) -> None:
        if self._thread is not None:
            return
        self._thread = threading.Thread(target=self._thread_main, daemon=True)
        self._thread.start()
        if not self._ready.wait(timeout):
            self.stop()
            raise ProbeError(f"usbmux relay {self.device_port} did not become ready")
        if self._error is not None:
            raise ProbeError(f"usbmux relay {self.device_port} failed")

    def _thread_main(self) -> None:
        try:
            asyncio.run(self._run())
        except Exception as exc:
            self._error = exc
            self._ready.set()

    async def _run(self) -> None:
        from pymobiledevice3.tcp_forwarder import UsbmuxTcpForwarder

        listening = asyncio.Event()
        forwarder = UsbmuxTcpForwarder(
            self.udid, self.device_port, self.local_port, listening_event=listening
        )
        task = asyncio.create_task(forwarder.start(address="127.0.0.1"))
        try:
            await asyncio.wait_for(listening.wait(), timeout=10.0)
            self._ready.set()
            while not self._stop.is_set():
                if task.done():
                    await task
                    raise ProbeError(f"usbmux relay {self.device_port} exited")
                await asyncio.sleep(0.05)
        finally:
            forwarder.stop()
            task.cancel()
            with contextlib.suppress(asyncio.CancelledError):
                await task

    def stop(self) -> None:
        self._stop.set()
        thread = self._thread
        if thread is not None:
            thread.join(timeout=5.0)
            if thread.is_alive():
                raise ProbeError(f"usbmux relay {self.device_port} thread did not stop")
            self._thread = None


async def _with_process_control(udid: str, operation):
    from pymobiledevice3.lockdown import create_using_usbmux
    from pymobiledevice3.services.dvt.instruments.dvt_provider import DvtProvider
    from pymobiledevice3.services.dvt.instruments.process_control import ProcessControl

    lockdown = await create_using_usbmux(serial=udid)
    try:
        async with DvtProvider(lockdown) as dvt:
            async with ProcessControl(dvt) as process_control:
                return await operation(process_control)
    finally:
        await lockdown.close()


def run_async_bounded(awaitable, timeout: float, label: str):
    if timeout <= 0:
        close = getattr(awaitable, "close", None)
        if close is not None:
            close()
        raise ProbeError(f"{label} timeout must be positive")

    async def bounded():
        return await asyncio.wait_for(awaitable, timeout=timeout)

    try:
        return asyncio.run(bounded())
    except asyncio.TimeoutError as exc:
        raise ProbeError(f"{label} exceeded its deadline") from exc


async def _device_port_is_open(
    udid: str, port: int, timeout: float = DEVICE_PORT_PROBE_TIMEOUT_SECONDS
) -> bool:
    from pymobiledevice3 import usbmux
    from pymobiledevice3.exceptions import ConnectionFailedError

    try:
        device = await asyncio.wait_for(usbmux.select_device(udid), timeout=timeout)
    except asyncio.TimeoutError as exc:
        raise ProbeError("USB device lookup exceeded its deadline") from exc
    if device is None:
        raise ProbeError("USB device is not connected")
    connection = None
    try:
        connection = await asyncio.wait_for(device.connect(port), timeout=timeout)
        return True
    except ConnectionFailedError:
        return False
    except asyncio.TimeoutError as exc:
        raise ProbeError("device port probe exceeded its deadline") from exc
    finally:
        if connection is not None:
            close = getattr(connection, "close", None)
            if close is not None:
                result = close()
                if asyncio.iscoroutine(result):
                    await result


class MacDeviceAdapter:

    def __init__(
        self,
        *,
        udid: str,
        artifact: CandidateArtifact,
        reuse_trusted_install: bool = False,
    ) -> None:
        if sys.platform != "darwin":
            raise ProbeError("live Gate B/C probe requires macOS")
        requirements = {"pymobiledevice3": "10.1.0", "Pillow": "11.3.0"}
        for package, expected in requirements.items():
            try:
                actual = importlib.metadata.version(package)
            except importlib.metadata.PackageNotFoundError as exc:
                raise ProbeError(f"{package} {expected} is required") from exc
            if actual != expected:
                raise ProbeError(f"{package} {expected} is required")
        self.udid = udid
        self.artifact = artifact
        self.candidate_bundle = artifact.bundle_id
        self.reuse_trusted_install = reuse_trusted_install
        self.evidence_environment = (
            SUPPLEMENTAL_ENVIRONMENT if reuse_trusted_install else LIVE_ENVIRONMENT
        )
        self.control_relay: UsbmuxRelay | None = None
        self.mjpeg_relay: UsbmuxRelay | None = None

    @property
    def control_address(self) -> tuple[str, int]:
        if self.control_relay is None:
            raise ProbeError("control relay is not running")
        return "127.0.0.1", self.control_relay.local_port

    @property
    def mjpeg_address(self) -> tuple[str, int]:
        if self.mjpeg_relay is None:
            raise ProbeError("MJPEG relay is not running")
        return "127.0.0.1", self.mjpeg_relay.local_port

    def prepare_candidate(
        self,
        artifact: CandidateArtifact,
        timeout: float,
        *,
        reuse_trusted_install: bool = False,
    ) -> dict[str, Any]:
        if artifact != self.artifact:
            raise ProbeError("adapter candidate does not match the attested artifact")
        if reuse_trusted_install != self.reuse_trusted_install:
            raise ProbeError("candidate reuse mode does not match the adapter")
        if _sha256_file(artifact.ipa_path) != artifact.ipa_sha256:
            raise ProbeError("candidate IPA changed after manifest attestation")
        self.stop_relays()
        self.terminate_candidate()
        if not self.wait_candidate_ports_closed(timeout):
            raise ProbeError("candidate ports remained open before candidate install")

        async def install_or_reuse() -> tuple[dict[str, Any], str, str]:
            from pymobiledevice3.lockdown import create_using_usbmux
            from pymobiledevice3.services.installation_proxy import InstallationProxyService

            lockdown = await create_using_usbmux(serial=self.udid)
            try:
                product_type = lockdown.product_type or "unknown"
                product_version = lockdown.product_version or "unknown"
                async with InstallationProxyService(lockdown=lockdown) as proxy:
                    before = await proxy.get_apps(bundle_identifiers=[artifact.bundle_id])
                    if self.reuse_trusted_install:
                        installed = before.get(artifact.bundle_id)
                        if not isinstance(installed, dict):
                            raise ProbeError(
                                "trusted candidate is not installed; run a fresh-install probe first"
                            )
                        await proxy.install_from_local(
                            str(artifact.ipa_path),
                            cmd="Upgrade",
                            options={"ApplicationType": "User"},
                            developer=True,
                        )
                    else:
                        if artifact.bundle_id in before:
                            await proxy.uninstall(artifact.bundle_id)
                        await proxy.install_from_local(str(artifact.ipa_path), developer=True)
                    after = await proxy.get_apps(bundle_identifiers=[artifact.bundle_id])
                    installed = after.get(artifact.bundle_id)
                    if not isinstance(installed, dict):
                        raise ProbeError("candidate install was not returned by installation proxy")
                    return installed, product_type, product_version
            finally:
                await lockdown.close()

        installed, product_type, product_version = run_async_bounded(
            install_or_reuse(),
            INSTALL_OPERATION_TIMEOUT_SECONDS,
            "candidate install or trusted upgrade",
        )
        time.sleep(POST_INSTALL_SETTLE_SECONDS)
        installed_path = installed.get("Path")
        payload_name = (
            PurePosixPath(installed_path).name if isinstance(installed_path, str) else None
        )
        installed_signer_team_id = installed_team_id(installed)
        comparisons = {
            "bundleId": installed.get("CFBundleIdentifier") == artifact.bundle_id,
            "bundleVersion": installed.get("CFBundleShortVersionString")
            == artifact.bundle_version,
            "bundleBuild": installed.get("CFBundleVersion") == artifact.bundle_build,
            "payloadApp": payload_name == artifact.payload_app,
            "executable": installed.get("CFBundleExecutable") == artifact.executable,
            "signerIdentity": installed.get("SignerIdentity") == artifact.signer_identity,
            "signerTeamId": installed_signer_team_id == artifact.signer_team_id,
        }
        if not all(comparisons.values()):
            raise ProbeError("installed candidate identity does not match the attested IPA")
        return {
            "freshInstall": not self.reuse_trusted_install,
            "installationMode": (
                "trusted_upgrade" if self.reuse_trusted_install else "fresh_install"
            ),
            "identityMatch": True,
            "productType": product_type,
            "iOSVersion": product_version,
            "bundleId": artifact.bundle_id,
            "bundleVersion": artifact.bundle_version,
            "bundleBuild": artifact.bundle_build,
            "payloadApp": artifact.payload_app,
            "applicationType": installed.get("ApplicationType", "unknown"),
            "signerIdentitySha256": _sha256_text(artifact.signer_identity),
            "signerTeamId": installed_signer_team_id,
        }

    def terminate_candidate(self) -> int | None:
        async def terminate(process_control):
            pid = await process_control.process_identifier_for_bundle_identifier(
                self.candidate_bundle
            )
            if pid:
                await process_control.kill(pid)
                return int(pid)
            return None

        result = run_async_bounded(
            _with_process_control(self.udid, terminate),
            PROCESS_CONTROL_TIMEOUT_SECONDS,
            "candidate terminate",
        )
        if result is not None and (
            isinstance(result, bool) or not isinstance(result, int) or result <= 0
        ):
            raise ProbeError("candidate terminate returned an invalid process ID")
        return result

    def candidate_process_id(self) -> int | None:
        async def identify(process_control):
            return await process_control.process_identifier_for_bundle_identifier(
                self.candidate_bundle
            )

        result = run_async_bounded(
            _with_process_control(self.udid, identify),
            PROCESS_CONTROL_TIMEOUT_SECONDS,
            "candidate process lookup",
        )
        if not result:
            return None
        if isinstance(result, bool) or not isinstance(result, int) or result <= 0:
            raise ProbeError("candidate process lookup returned an invalid process ID")
        return result

    def wait_candidate_ports_closed(self, timeout: float) -> bool:
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            remaining = max(0.05, deadline - time.monotonic())
            probe_timeout = min(DEVICE_PORT_PROBE_TIMEOUT_SECONDS, remaining)
            control_open = run_async_bounded(
                _device_port_is_open(self.udid, CONTROL_DEVICE_PORT, probe_timeout),
                probe_timeout + 0.5,
                "candidate control-port probe",
            )
            mjpeg_open = run_async_bounded(
                _device_port_is_open(self.udid, MJPEG_DEVICE_PORT, probe_timeout),
                probe_timeout + 0.5,
                "candidate MJPEG-port probe",
            )
            process_id = self.candidate_process_id()
            if process_id is None and not control_open and not mjpeg_open:
                return True
            time.sleep(0.25)
        return False

    def launch_candidate(self, environment: dict[str, str]) -> int:
        async def launch(process_control):
            activated = await process_control.launch(
                self.candidate_bundle,
                kill_existing=False,
                environment=environment,
            )
            observed = await process_control.process_identifier_for_bundle_identifier(
                self.candidate_bundle
            )
            return activated, observed

        for attempt in range(DVT_LAUNCH_ATTEMPTS):
            try:
                activated, observed = run_async_bounded(
                    _with_process_control(self.udid, launch),
                    PROCESS_CONTROL_TIMEOUT_SECONDS,
                    "candidate launch",
                )
                break
            except Exception as exc:
                transient = (
                    "deviceprocesscontrolservice" in str(exc)
                    and "failed to launch" in str(exc)
                )
                if not transient or attempt == DVT_LAUNCH_ATTEMPTS - 1:
                    raise
                time.sleep(DVT_LAUNCH_RETRY_DELAY_SECONDS)
        if (
            isinstance(activated, bool)
            or not isinstance(activated, int)
            or activated <= 0
            or observed != activated
        ):
            raise ProbeError("candidate DVT launch did not produce a stable process ID")
        deadline = time.monotonic() + 45.0
        while time.monotonic() < deadline:
            if run_async_bounded(
                _device_port_is_open(self.udid, CONTROL_DEVICE_PORT),
                DEVICE_PORT_PROBE_TIMEOUT_SECONDS + 0.5,
                "candidate launch port probe",
            ):
                return activated
            time.sleep(0.25)
        raise ProbeError("candidate control port did not open after DVT launch")

    def start_control_relay(self) -> None:
        if self.control_relay is None:
            self.control_relay = UsbmuxRelay(self.udid, CONTROL_DEVICE_PORT)
            self.control_relay.start()

    def start_mjpeg_relay(self) -> None:
        if self.mjpeg_relay is None:
            self.mjpeg_relay = UsbmuxRelay(self.udid, MJPEG_DEVICE_PORT)
            self.mjpeg_relay.start()

    def foreground(self, bundle_id: str) -> None:
        async def launch(process_control):
            return await process_control.launch(bundle_id, kill_existing=True, environment={})

        run_async_bounded(
            _with_process_control(self.udid, launch),
            PROCESS_CONTROL_TIMEOUT_SECONDS,
            "gesture-surface launch",
        )

    def foreground_candidate_without_restart(self) -> int:
        async def foreground(process_control):
            before = await process_control.process_identifier_for_bundle_identifier(
                self.candidate_bundle
            )
            if not before:
                raise ProbeError("candidate was not running before foreground activation")
            activated = await process_control.launch(
                self.candidate_bundle, kill_existing=False, environment={}
            )
            after = await process_control.process_identifier_for_bundle_identifier(
                self.candidate_bundle
            )
            if activated != before or after != before:
                raise ProbeError("candidate PID changed while moving agent to foreground")
            return int(before)

        result = run_async_bounded(
            _with_process_control(self.udid, foreground),
            PROCESS_CONTROL_TIMEOUT_SECONDS,
            "candidate foreground activation",
        )
        if isinstance(result, bool) or not isinstance(result, int) or result <= 0:
            raise ProbeError("candidate foreground activation returned an invalid PID")
        return result

    def stop_relays(self) -> None:
        errors: list[Exception] = []
        if self.mjpeg_relay is not None:
            try:
                self.mjpeg_relay.stop()
            except Exception as exc:
                errors.append(exc)
            self.mjpeg_relay = None
        if self.control_relay is not None:
            try:
                self.control_relay.stop()
            except Exception as exc:
                errors.append(exc)
            self.control_relay = None
        if errors:
            raise ProbeError("one or more usbmux relay threads did not stop")


def _write_text_atomic(path: Path, contents: str) -> None:
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
        with contextlib.suppress(OSError):
            os.close(descriptor)
        temporary.unlink(missing_ok=True)
        raise


def _verify_evidence_redaction(paths: list[Path]) -> None:
    command = ["cargo", "run", "-q", "-p", "rtmmo-re", "--", "verify-redaction"]
    for path in paths:
        command.extend(["--input", str(path)])
    environment = os.environ.copy()
    environment.pop(TOKEN_ENVIRONMENT, None)
    try:
        result = subprocess.run(
            command,
            cwd=REPOSITORY_ROOT,
            env=environment,
            capture_output=True,
            text=True,
            shell=False,
            timeout=60.0,
        )
    except subprocess.TimeoutExpired as exc:
        raise ProbeError("redaction verifier exceeded its deadline") from exc
    if result.returncode != 0:
        raise ProbeError("redaction verifier rejected staged Gate B/C evidence")


def write_evidence(
    output: Path,
    report: dict[str, Any],
    token: SecretToken,
    *,
    verifier: Callable[[list[Path]], None] = _verify_evidence_redaction,
) -> None:
    output = Path(output)
    if output.suffix.lower() != ".json" or output.name.casefold() in {
        "gate-b.md",
        "gate-c.md",
    }:
        raise ProbeError("Gate B/C evidence output must use a distinct .json filename")
    output.parent.mkdir(parents=True, exist_ok=True)
    serialized = serialize_report(report, token)
    documents = {
        output.name: serialized,
        "gate-b.md": (
            "# Riviu Agent GATE-B\n\n"
            f"Status: `{report['gateB']}`\n\nEvidence: `{output.name}`\n"
        ),
        "gate-c.md": (
            "# Riviu Agent GATE-C\n\n"
            f"Status: `{report['gateC']}`\n\nEvidence: `{output.name}`\n"
        ),
    }
    with tempfile.TemporaryDirectory(prefix=".riviu-agent-evidence-", dir=output.parent) as temp:
        transaction = Path(temp)
        staging = transaction / "new"
        backups = transaction / "old"
        staging.mkdir()
        backups.mkdir()
        staged: list[Path] = []
        for name, contents in documents.items():
            path = staging / name
            _write_text_atomic(path, contents)
            staged.append(path)
        verifier(staged)
        publications = [
            (
                path,
                output if path.name == output.name else output.parent / path.name,
                backups / path.name,
            )
            for path in staged
        ]
        for _source, destination, backup in publications:
            if destination.exists():
                if not destination.is_file():
                    raise ProbeError("Gate B/C evidence destination is not a file")
                shutil.copyfile(destination, backup)
        try:
            for source, destination, _backup in publications:
                os.replace(source, destination)
        except OSError as exc:
            restoration_failed = False
            for _source, destination, backup in publications:
                try:
                    if backup.is_file():
                        os.replace(backup, destination)
                    else:
                        destination.unlink(missing_ok=True)
                except OSError:
                    restoration_failed = True
            if restoration_failed:
                raise ProbeError("Gate B/C evidence publication and rollback failed") from exc
            raise ProbeError("Gate B/C evidence publication was rolled back") from exc


def _parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--udid", required=True)
    parser.add_argument("--manifest", type=Path, required=True)
    parser.add_argument("--cold-launches", type=int, default=GATE_COLD_LAUNCHES)
    parser.add_argument("--tap-attempts", type=int, default=GATE_TAP_SUCCESSES)
    parser.add_argument("--swipe-attempts", type=int, default=GATE_SWIPE_SUCCESSES)
    parser.add_argument("--stream-seconds", type=float, default=GATE_STREAM_SECONDS)
    parser.add_argument("--request-timeout", type=float, default=10.0)
    parser.add_argument(
        "--wait-for-trust",
        action="store_true",
        help="pause after fresh-install for manual Apple Development profile approval",
    )
    parser.add_argument(
        "--reuse-trusted-install",
        action="store_true",
        help="upgrade the current trusted install without uninstalling (supplemental only)",
    )
    parser.add_argument("--output", type=Path, default=DEFAULT_OUTPUT)
    return parser


def main() -> int:
    args = _parser().parse_args()
    raw_token = os.environ.get(TOKEN_ENVIRONMENT, "")
    try:
        token = SecretToken(raw_token)
        artifact = load_candidate_artifact(args.manifest)
        with tempfile.TemporaryDirectory(prefix="riviu-agent-token-source-") as temp:
            prepared_source = Path(temp) / "source"
            prepare_locked_source_for_scan(artifact, prepared_source)
            token_preflight = scan_token_preflight(
                artifact=artifact,
                token=token,
                prepared_source=prepared_source,
                argv=sys.argv,
            )
        if not token_preflight_is_clean(token_preflight):
            raise ProbeError("agent token preflight rejected a candidate input")
        config = ProbeConfig(
            cold_launches=args.cold_launches,
            tap_attempts=args.tap_attempts,
            swipe_attempts=args.swipe_attempts,
            stream_seconds=args.stream_seconds,
            request_timeout=args.request_timeout,
            foreground_bundle=SETTINGS_BUNDLE_ID,
            wait_for_trust=args.wait_for_trust,
            reuse_trusted_install=args.reuse_trusted_install,
        )
        validate_live_config(config)
        adapter = MacDeviceAdapter(
            udid=args.udid,
            artifact=artifact,
            reuse_trusted_install=args.reuse_trusted_install,
        )
        with guard_process_output(token):
            report = ProbeRunner(
                adapter=adapter,
                artifact=artifact,
                config=config,
                token=token,
                token_preflight=token_preflight,
            ).run()
            report["measurements"]["logTokenScanClean"] = True
            attest_report_token_scan(report, token)
            write_evidence(args.output, report, token)
    except ProbeError as exc:
        token_for_redaction = SecretToken(raw_token) if len(raw_token.encode("utf-8")) >= 32 else None
        message = (
            sanitize_message(str(exc), token_for_redaction, (args.udid, str(args.manifest)))
            if token_for_redaction is not None
            else "Gate B/C setup failed"
        )
        print(json.dumps({"ok": False, "error": message}, ensure_ascii=True, sort_keys=True))
        return 1
    print(
        json.dumps(
            {
                "ok": report["gateStatus"] == "PASS",
                "gateStatus": report["gateStatus"],
                "output": args.output.name,
            },
            ensure_ascii=True,
            sort_keys=True,
        )
    )
    return 0 if report["gateStatus"] == "PASS" else 2


if __name__ == "__main__":
    raise SystemExit(main())
