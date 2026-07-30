#!/usr/bin/env python3
"""Gate 0 transport, URL, clipboard, geometry, and lifecycle probe.

The fixture surface is importable without iOS dependencies. Device-only imports
are delayed until the live Mac path is selected.
"""

from __future__ import annotations

import argparse
import asyncio
import base64
import binascii
import contextlib
import hashlib
import importlib.metadata
import io
import json
import math
import os
import platform
import plistlib
import re
import shutil
import socket
import subprocess
import sys
import tempfile
import threading
import time
import urllib.error
import urllib.parse
import urllib.request
import unicodedata
import zipfile
from pathlib import Path, PurePosixPath
from typing import Any, Callable, Iterable, NamedTuple, Sequence


MAX_CLIPBOARD_BYTES = 65_536
MAX_HTTP_BYTES = 16 * 1024 * 1024
MAX_MJPEG_BYTES = 16 * 1024 * 1024
REQUEST_TIMEOUT_SECONDS = 10.0
MAX_STREAM_GAP_SECONDS = 2.0
EXPECTED_LOGICAL_BOUNDS = (375, 667)
FIXTURE_ENVIRONMENT = "FIXTURE_ONLY"
LIVE_ENVIRONMENT = "LIVE_MAC_DEVICE"
PENDING_ENVIRONMENT = "PENDING_MAC_DEVICE"
SHA256_RE = re.compile(r"^[0-9a-f]{64}$")
SHARE_DETECTOR_VERSION = "rail-white-chain-v1"
COPY_LINK_DETECTOR_VERSION = "macos-vision-text-v1"
LAYOUT_ID = "tiktok-iphone8-portrait-share-sheet-v1"
VISION_HELPER = Path(__file__).resolve().with_name("vision_ocr.swift")
REQUIREMENTS_MAC = (
    Path(__file__).resolve().parents[2]
    / "sidecars"
    / "wda"
    / "riviu-agent"
    / "requirements-mac.txt"
)
EXPECTED_LIVE_DEPENDENCIES = {
    "pymobiledevice3": "10.1.0",
    "Pillow": "11.3.0",
}
CLIPBOARD_FIXTURE_VALUE = b"RIVIU_GATE0_CLIPBOARD_FIXTURE_V1"


class ProbeError(RuntimeError):
    """A typed, operator-visible gate failure."""


class StartupProof(NamedTuple):
    events: list[str]
    frame_size: tuple[int, int]
    frame_sha256: str


class AppIdentityProof(NamedTuple):
    bundle_id: str
    pid: int


class GeometryEvidence(NamedTuple):
    logical_width: int
    logical_height: int
    frame_width: int
    frame_height: int
    orientation: str


class ArtifactProof(NamedTuple):
    ipa_sha256: str
    manifest_sha256: str
    manifest: dict[str, Any]


class IpaIdentity(NamedTuple):
    payload_app: str
    bundle_id: str
    version: str
    build: str
    executable_name: str


class PostIdentity(NamedTuple):
    content_id: str
    post_kind: str


class StreamBatch(NamedTuple):
    generation: int
    start_sequence: int
    end_sequence: int
    frames: list[bytes]
    maximum_gap_seconds: float


class UsbmuxRelay:
    """A bounded local relay whose live dependency is imported on its thread."""

    def __init__(self, udid: str, device_port: int) -> None:
        self.udid = udid
        self.device_port = int(device_port)
        self.local_port = _free_local_port()
        self._ready = threading.Event()
        self._stop = threading.Event()
        self._error: Exception | None = None
        self._thread: threading.Thread | None = None

    def start(self, timeout_seconds: float = 10.0) -> None:
        if self._thread is not None:
            return
        self._thread = threading.Thread(target=self._thread_main, daemon=True)
        self._thread.start()
        if not self._ready.wait(timeout_seconds):
            self.stop()
            raise ProbeError(f"RelayStartTimeout: devicePort={self.device_port}")
        if self._error is not None:
            raise ProbeError(f"RelayStartFailed: devicePort={self.device_port}")

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
            self.udid,
            self.device_port,
            self.local_port,
            listening_event=listening,
        )
        task = asyncio.create_task(forwarder.start(address="127.0.0.1"))
        try:
            await asyncio.wait_for(listening.wait(), timeout=10.0)
            self._ready.set()
            while not self._stop.is_set():
                if task.done():
                    await task
                    raise ProbeError(f"RelayExited: devicePort={self.device_port}")
                await asyncio.sleep(0.05)
        finally:
            forwarder.stop()
            task.cancel()
            with contextlib.suppress(asyncio.CancelledError):
                await task

    def stop(self) -> None:
        self._stop.set()
        thread = self._thread
        if thread is None:
            return
        thread.join(timeout=5.0)
        if thread.is_alive():
            raise ProbeError(f"RelayStopTimeout: devicePort={self.device_port}")
        self._thread = None


class MjpegReader:
    """One authenticated MJPEG connection owned by one producer generation."""

    def __init__(
        self,
        url: str,
        header_name: str,
        token: str,
        *,
        generation: int,
    ) -> None:
        if isinstance(generation, bool) or not isinstance(generation, int) or generation < 1:
            raise ProbeError("MjpegGenerationInvalid")
        self.url = url
        self.header_name = header_name
        self.token = token
        self.generation = generation
        self._condition = threading.Condition()
        self._stop = threading.Event()
        self._thread: threading.Thread | None = None
        self._response: Any = None
        self._error: Exception | None = None
        self._frames: list[tuple[int, float, bytes]] = []
        self._sequence = 0
        self._dimensions: tuple[int, int] | None = None

    @property
    def sequence(self) -> int:
        with self._condition:
            return self._sequence

    @property
    def latest_frame(self) -> bytes:
        with self._condition:
            if not self._frames:
                raise ProbeError("MjpegFirstFrameMissing")
            return self._frames[-1][2]

    def start(
        self,
        *,
        minimum_frames: int = 3,
        timeout_seconds: float = REQUEST_TIMEOUT_SECONDS,
    ) -> StreamBatch:
        if self._thread is not None:
            raise ProbeError("MjpegReaderAlreadyStarted")
        self._thread = threading.Thread(target=self._thread_main, daemon=True)
        self._thread.start()
        return self.wait_for_frames(
            0, minimum_frames=minimum_frames, timeout_seconds=timeout_seconds
        )

    def _thread_main(self) -> None:
        request = urllib.request.Request(
            self.url,
            headers={
                self.header_name: self.token,
                "Accept": "multipart/x-mixed-replace",
            },
            method="GET",
        )
        response = None
        try:
            response = urllib.request.urlopen(request, timeout=MAX_STREAM_GAP_SECONDS)
            with self._condition:
                self._response = response
                self._condition.notify_all()
            if response.status != 200:
                raise ProbeError(f"MjpegHttpStatus: {response.status}")
            if response.headers.get_content_type().lower() != "multipart/x-mixed-replace":
                raise ProbeError("MjpegContentTypeInvalid")
            if response.headers.get("Content-Length") is not None:
                raise ProbeError("MjpegStreamBounded")
            buffer = bytearray()
            while not self._stop.is_set():
                chunk = response.read1(65_536)
                if not chunk:
                    raise ProbeError("MjpegStreamEnded")
                buffer.extend(chunk)
                while True:
                    start = buffer.find(b"\xff\xd8")
                    if start < 0:
                        if len(buffer) > 8_192:
                            raise ProbeError("MjpegHeaderTooLarge")
                        break
                    if start > 8_192:
                        raise ProbeError("MjpegHeaderTooLarge")
                    end = buffer.find(b"\xff\xd9", start + 2)
                    if end < 0:
                        if len(buffer) - start > MAX_MJPEG_BYTES:
                            raise ProbeError("MjpegFrameTooLarge")
                        if start:
                            del buffer[:start]
                        break
                    frame = bytes(buffer[start : end + 2])
                    dimensions = _jpeg_dimensions(frame)
                    del buffer[: end + 2]
                    now = time.monotonic()
                    with self._condition:
                        if self._dimensions is None:
                            self._dimensions = dimensions
                        elif dimensions != self._dimensions:
                            raise ProbeError("MjpegGeometryChanged")
                        if self._frames and now - self._frames[-1][1] > MAX_STREAM_GAP_SECONDS:
                            raise ProbeError("MjpegInterFrameGapExceeded")
                        self._sequence += 1
                        self._frames.append((self._sequence, now, frame))
                        if len(self._frames) > 128:
                            self._frames = self._frames[-128:]
                        self._condition.notify_all()
        except Exception as exc:
            if not self._stop.is_set():
                with self._condition:
                    self._error = exc
                    self._condition.notify_all()
        finally:
            if response is not None:
                with contextlib.suppress(Exception):
                    response.close()
            with self._condition:
                self._response = None
                self._condition.notify_all()

    def wait_for_frames(
        self,
        after_sequence: int,
        *,
        minimum_frames: int = 3,
        timeout_seconds: float = REQUEST_TIMEOUT_SECONDS,
    ) -> StreamBatch:
        if (
            isinstance(after_sequence, bool)
            or not isinstance(after_sequence, int)
            or after_sequence < 0
            or isinstance(minimum_frames, bool)
            or not isinstance(minimum_frames, int)
            or minimum_frames < 1
            or minimum_frames > 64
            or not math.isfinite(timeout_seconds)
            or timeout_seconds <= 0
        ):
            raise ProbeError("MjpegWaitInvalid")
        deadline = time.monotonic() + timeout_seconds
        with self._condition:
            while True:
                if self._error is not None:
                    raise ProbeError("MjpegReaderFailed") from self._error
                available = [item for item in self._frames if item[0] > after_sequence]
                if len(available) >= minimum_frames:
                    selected = available[:minimum_frames]
                    if time.monotonic() - selected[-1][1] > MAX_STREAM_GAP_SECONDS:
                        raise ProbeError("MjpegInterFrameGapExceeded")
                    frames = [item[2] for item in selected]
                    if minimum_frames > 1 and len(
                        {hashlib.sha256(frame).digest() for frame in frames}
                    ) < 2:
                        raise ProbeError("MjpegStreamStalled")
                    gaps = [
                        selected[index][1] - selected[index - 1][1]
                        for index in range(1, len(selected))
                    ]
                    maximum_gap = max(gaps, default=0.0)
                    if maximum_gap > MAX_STREAM_GAP_SECONDS:
                        raise ProbeError("MjpegInterFrameGapExceeded")
                    return StreamBatch(
                        self.generation,
                        selected[0][0],
                        selected[-1][0],
                        frames,
                        maximum_gap,
                    )
                if (
                    self._frames
                    and time.monotonic() - self._frames[-1][1] > MAX_STREAM_GAP_SECONDS
                ):
                    raise ProbeError("MjpegInterFrameGapExceeded")
                remaining = deadline - time.monotonic()
                if remaining <= 0:
                    raise ProbeError("MjpegFrameSequenceTimeout")
                self._condition.wait(min(remaining, 0.1))

    def stop(self) -> None:
        self._stop.set()
        with self._condition:
            response = self._response
        if response is not None:
            with contextlib.suppress(Exception):
                response.close()
        thread = self._thread
        if thread is None:
            return
        thread.join(timeout=5.0)
        if thread.is_alive():
            raise ProbeError("MjpegReaderStopTimeout")
        self._thread = None


def _free_local_port() -> int:
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as listener:
        listener.bind(("127.0.0.1", 0))
        return int(listener.getsockname()[1])


def _run_async(awaitable: Any, timeout_seconds: float, label: str) -> Any:
    async def bounded() -> Any:
        return await asyncio.wait_for(awaitable, timeout_seconds)

    try:
        return asyncio.run(bounded())
    except asyncio.TimeoutError as exc:
        raise ProbeError(f"DeadlineExceeded: {label}") from exc


async def _with_process_control(udid: str, operation: Callable[[Any], Any]) -> Any:
    from pymobiledevice3.lockdown import create_using_usbmux
    from pymobiledevice3.services.dvt.instruments.dvt_provider import DvtProvider
    from pymobiledevice3.services.dvt.instruments.process_control import ProcessControl

    lockdown = await create_using_usbmux(serial=udid, autopair=False)
    try:
        async with DvtProvider(lockdown) as dvt:
            async with ProcessControl(dvt) as process_control:
                return await operation(process_control)
    finally:
        await lockdown.close()


async def _device_display_metrics(udid: str) -> dict[str, Any]:
    from pymobiledevice3.lockdown import create_using_usbmux
    from pymobiledevice3.services.diagnostics import DiagnosticsService

    lockdown = await create_using_usbmux(serial=udid, autopair=False)
    try:
        async with DiagnosticsService(lockdown=lockdown) as diagnostics:
            return await diagnostics.mobilegestalt(
                [
                    "main-screen-width",
                    "main-screen-height",
                    "main-screen-scale",
                ]
            )
    finally:
        await lockdown.close()


async def _device_port_is_open(udid: str, port: int, timeout_seconds: float = 5.0) -> bool:
    from pymobiledevice3 import usbmux
    from pymobiledevice3.exceptions import ConnectionFailedError

    device = await asyncio.wait_for(usbmux.select_device(udid), timeout_seconds)
    if device is None:
        raise ProbeError("DeviceDisconnected")
    connection = None
    try:
        connection = await asyncio.wait_for(device.connect(port), timeout_seconds)
        return True
    except ConnectionFailedError:
        return False
    finally:
        if connection is not None:
            close = getattr(connection, "close", None)
            if close is not None:
                result = close()
                if hasattr(result, "__await__"):
                    await result


def _strict_json(raw: bytes, label: str) -> Any:
    def unique_pairs(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
        result: dict[str, Any] = {}
        for key, value in pairs:
            if key in result:
                raise ProbeError(f"DuplicateJsonKey: {label} contains {key!r}")
            result[key] = value
        return result

    try:
        return json.loads(raw.decode("utf-8"), object_pairs_hook=unique_pairs)
    except ProbeError:
        raise
    except (UnicodeDecodeError, json.JSONDecodeError) as exc:
        raise ProbeError(f"InvalidJson: {label}") from exc


def _sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    try:
        with path.open("rb") as source:
            for chunk in iter(lambda: source.read(1024 * 1024), b""):
                digest.update(chunk)
    except OSError as exc:
        raise ProbeError(f"ArtifactReadFailed: {path.name}") from exc
    return digest.hexdigest()


def verify_artifact_manifest(ipa_path: Path, manifest_path: Path) -> ArtifactProof:
    ipa_path = Path(ipa_path)
    manifest_path = Path(manifest_path)
    try:
        raw_manifest = manifest_path.read_bytes()
    except OSError as exc:
        raise ProbeError("ArtifactManifestReadFailed") from exc
    manifest = _strict_json(raw_manifest, "agent manifest")
    if not isinstance(manifest, dict):
        raise ProbeError("ArtifactManifestInvalid: root must be an object")
    expected_ipa_sha = manifest.get("sha256")
    if not isinstance(expected_ipa_sha, str) or not SHA256_RE.fullmatch(expected_ipa_sha):
        raise ProbeError("ArtifactManifestInvalid: sha256")
    actual_ipa_sha = _sha256_file(ipa_path)
    if actual_ipa_sha != expected_ipa_sha:
        raise ProbeError("ArtifactShaMismatch")
    return ArtifactProof(
        ipa_sha256=actual_ipa_sha,
        manifest_sha256=hashlib.sha256(raw_manifest).hexdigest(),
        manifest=manifest,
    )


def inspect_ipa_identity(ipa_path: Path, manifest: dict[str, Any]) -> IpaIdentity:
    payload_app = _required_text(manifest, "payloadApp", "artifact.payloadApp")
    if Path(payload_app).name != payload_app or not payload_app.endswith(".app"):
        raise ProbeError("ArtifactManifestInvalid: payloadApp")
    info_path = f"Payload/{payload_app}/Info.plist"
    try:
        with zipfile.ZipFile(ipa_path, "r") as archive:
            matches = [entry for entry in archive.infolist() if entry.filename == info_path]
            if len(matches) != 1 or matches[0].file_size > 1024 * 1024:
                raise ProbeError("ArtifactInfoPlistInvalid")
            raw_plist = archive.read(matches[0])
    except ProbeError:
        raise
    except (OSError, KeyError, zipfile.BadZipFile, RuntimeError) as exc:
        raise ProbeError("ArtifactInfoPlistReadFailed") from exc
    try:
        info = plistlib.loads(raw_plist)
    except plistlib.InvalidFileException as exc:
        raise ProbeError("ArtifactInfoPlistInvalid") from exc
    if not isinstance(info, dict):
        raise ProbeError("ArtifactInfoPlistInvalid")
    identity = IpaIdentity(
        payload_app,
        _required_text(info, "CFBundleIdentifier", "ipa.CFBundleIdentifier"),
        _required_text(
            info, "CFBundleShortVersionString", "ipa.CFBundleShortVersionString"
        ),
        _required_text(info, "CFBundleVersion", "ipa.CFBundleVersion"),
        _required_text(info, "CFBundleExecutable", "ipa.CFBundleExecutable"),
    )
    expected = (
        _required_text(manifest, "bundleId", "artifact.bundleId"),
        _required_text(manifest, "bundleVersion", "artifact.bundleVersion"),
        _required_text(manifest, "bundleBuild", "artifact.bundleBuild"),
    )
    if identity[1:4] != expected:
        raise ProbeError("ArtifactInfoPlistIdentityMismatch")
    return identity


def _request(
    method: str,
    url: str,
    header_name: str,
    header_value: str | None,
    body: dict[str, Any] | None,
    timeout_seconds: float,
) -> tuple[int, bytes]:
    if not math.isfinite(timeout_seconds) or timeout_seconds <= 0:
        raise ProbeError("InvalidDeadline")
    encoded = None
    headers = {"Accept": "application/json", "Connection": "close"}
    if header_value is not None:
        headers[header_name] = header_value
    if body is not None:
        encoded = json.dumps(body, ensure_ascii=True, separators=(",", ":")).encode("ascii")
        headers["Content-Type"] = "application/json"
    request = urllib.request.Request(url, data=encoded, headers=headers, method=method)
    try:
        with urllib.request.urlopen(request, timeout=timeout_seconds) as response:
            raw = response.read(MAX_HTTP_BYTES + 1)
            status = int(response.status)
    except urllib.error.HTTPError as exc:
        try:
            raw = exc.read(MAX_HTTP_BYTES + 1)
            status = int(exc.code)
        finally:
            exc.close()
    except (OSError, TimeoutError, urllib.error.URLError) as exc:
        raise ProbeError(f"HttpRequestFailed: {method} {urllib.parse.urlsplit(url).path}") from exc
    if len(raw) > MAX_HTTP_BYTES:
        raise ProbeError("HttpResponseTooLarge")
    return status, raw


def prove_auth_triplet(
    url: str,
    header_name: str,
    token: str,
    *,
    timeout_seconds: float = REQUEST_TIMEOUT_SECONDS,
) -> tuple[int, int, int]:
    wrong = "x" * max(32, len(token))
    if wrong == token:
        wrong = "y" * len(wrong)
    statuses = (
        _request("GET", url, header_name, None, None, timeout_seconds)[0],
        _request("GET", url, header_name, wrong, None, timeout_seconds)[0],
        _request("GET", url, header_name, token, None, timeout_seconds)[0],
    )
    if statuses != (401, 401, 200):
        raise ProbeError(f"ProtectedAuthMismatch: {statuses}")
    return statuses


def _route_auth_exchange(
    method: str,
    url: str,
    header_name: str,
    token: str,
    body: dict[str, Any] | None,
    *,
    timeout_seconds: float = REQUEST_TIMEOUT_SECONDS,
) -> tuple[tuple[int, int, int], bytes]:
    method = method.upper()
    if method not in {"GET", "POST"}:
        raise ProbeError("RouteAuthMethodUnsupported")
    if method == "GET" and body is not None:
        raise ProbeError("RouteAuthBodyInvalid")
    wrong = "x" * max(32, len(token))
    if wrong == token:
        wrong = "y" * len(wrong)
    missing_status, _ = _request(
        method, url, header_name, None, body, timeout_seconds
    )
    wrong_status, _ = _request(
        method, url, header_name, wrong, body, timeout_seconds
    )
    correct_status, correct_raw = _request(
        method, url, header_name, token, body, timeout_seconds
    )
    statuses = (missing_status, wrong_status, correct_status)
    if statuses[:2] != (401, 401):
        route = urllib.parse.urlsplit(url).path
        raise ProbeError(f"ProtectedRouteAuthMismatch: {method} {route} {statuses}")
    if statuses[2] != 200:
        route = urllib.parse.urlsplit(url).path
        raise ProbeError(
            f"ProtectedRouteUnavailableInForeground: {method} {route} HTTP {statuses[2]}"
        )
    return statuses, correct_raw


def prove_route_auth_triplet(
    method: str,
    url: str,
    header_name: str,
    token: str,
    body: dict[str, Any] | None,
    *,
    timeout_seconds: float = REQUEST_TIMEOUT_SECONDS,
) -> tuple[int, int, int]:
    statuses, _ = _route_auth_exchange(
        method,
        url,
        header_name,
        token,
        body,
        timeout_seconds=timeout_seconds,
    )
    return statuses


def _request_status_only(
    url: str,
    header_name: str,
    header_value: str | None,
    timeout_seconds: float,
) -> int:
    request = urllib.request.Request(url, method="GET")
    if header_value is not None:
        request.add_header(header_name, header_value)
    try:
        response = urllib.request.urlopen(request, timeout=timeout_seconds)
    except urllib.error.HTTPError as exc:
        try:
            return int(exc.code)
        finally:
            exc.close()
    except (OSError, TimeoutError, urllib.error.URLError) as exc:
        raise ProbeError("StreamAuthRequestFailed") from exc
    try:
        return int(response.status)
    finally:
        response.close()


def prove_stream_auth_triplet(
    url: str,
    header_name: str,
    token: str,
    *,
    timeout_seconds: float = REQUEST_TIMEOUT_SECONDS,
) -> tuple[int, int, int]:
    wrong = "x" * max(32, len(token))
    if wrong == token:
        wrong = "y" * len(wrong)
    statuses = (
        _request_status_only(url, header_name, None, timeout_seconds),
        _request_status_only(url, header_name, wrong, timeout_seconds),
        _request_status_only(url, header_name, token, timeout_seconds),
    )
    if statuses != (401, 401, 200):
        raise ProbeError(f"ProtectedStreamAuthMismatch: {statuses}")
    return statuses


def post_open_url(
    control_base: str,
    route_path: str,
    header_name: str,
    token: str,
    target_url: str,
    target_bundle_id: str,
    *,
    timeout_seconds: float = REQUEST_TIMEOUT_SECONDS,
) -> None:
    if not route_path.startswith("/"):
        raise ProbeError("OpenUrlRouteInvalid")
    body = {
        "url": target_url,
        "bundleId": target_bundle_id,
        "idleTimeoutMs": 0,
    }
    status, _ = _request(
        "POST",
        control_base.rstrip("/") + route_path,
        header_name,
        token,
        body,
        timeout_seconds,
    )
    if status != 200:
        raise ProbeError(f"OpenUrlFailed: HTTP {status}")


def _jpeg_dimensions(jpeg: bytes) -> tuple[int, int]:
    if len(jpeg) < 4 or not jpeg.startswith(b"\xff\xd8") or not jpeg.endswith(b"\xff\xd9"):
        raise ProbeError("MjpegInvalidJpeg")
    offset = 2
    while offset + 4 <= len(jpeg):
        if jpeg[offset] != 0xFF:
            offset += 1
            continue
        marker = jpeg[offset + 1]
        offset += 2
        if marker in (0xD8, 0xD9) or 0xD0 <= marker <= 0xD7:
            continue
        if offset + 2 > len(jpeg):
            break
        length = int.from_bytes(jpeg[offset : offset + 2], "big")
        if length < 2 or offset + length > len(jpeg):
            raise ProbeError("MjpegInvalidJpeg")
        if marker in {
            0xC0,
            0xC1,
            0xC2,
            0xC3,
            0xC5,
            0xC6,
            0xC7,
            0xC9,
            0xCA,
            0xCB,
            0xCD,
            0xCE,
            0xCF,
        }:
            if length < 7:
                raise ProbeError("MjpegInvalidJpeg")
            height = int.from_bytes(jpeg[offset + 3 : offset + 5], "big")
            width = int.from_bytes(jpeg[offset + 5 : offset + 7], "big")
            if width <= 0 or height <= 0:
                raise ProbeError("MjpegInvalidJpeg")
            return width, height
        offset += length
    raise ProbeError("MjpegMissingDimensions")


def read_mjpeg_sequence(
    url: str,
    header_name: str,
    token: str,
    *,
    minimum_frames: int,
    timeout_seconds: float,
) -> list[bytes]:
    if (
        isinstance(minimum_frames, bool)
        or not isinstance(minimum_frames, int)
        or minimum_frames < 1
        or minimum_frames > 64
    ):
        raise ProbeError("MjpegFrameCountInvalid")
    if not math.isfinite(timeout_seconds) or timeout_seconds <= 0:
        raise ProbeError("InvalidDeadline")
    request = urllib.request.Request(
        url,
        headers={header_name: token, "Accept": "multipart/x-mixed-replace"},
        method="GET",
    )
    buffer = bytearray()
    frames: list[bytes] = []
    total_read = 0
    deadline = time.monotonic() + timeout_seconds
    try:
        with urllib.request.urlopen(request, timeout=timeout_seconds) as response:
            if response.status != 200:
                raise ProbeError(f"MjpegHttpStatus: {response.status}")
            content_type = response.headers.get_content_type().lower()
            if content_type != "multipart/x-mixed-replace":
                raise ProbeError("MjpegContentTypeInvalid")
            while total_read <= MAX_MJPEG_BYTES and time.monotonic() < deadline:
                chunk = response.read(min(65_536, MAX_MJPEG_BYTES + 1 - total_read))
                if not chunk:
                    break
                total_read += len(chunk)
                buffer.extend(chunk)
                while True:
                    start = buffer.find(b"\xff\xd8")
                    if start < 0:
                        if len(buffer) > 2:
                            del buffer[:-2]
                        break
                    end = buffer.find(b"\xff\xd9", start + 2)
                    if end < 0:
                        if start:
                            del buffer[:start]
                        break
                    frame = bytes(buffer[start : end + 2])
                    _jpeg_dimensions(frame)
                    frames.append(frame)
                    del buffer[: end + 2]
                    if len(frames) >= minimum_frames:
                        dimensions = {_jpeg_dimensions(item) for item in frames}
                        if len(dimensions) != 1:
                            raise ProbeError("MjpegGeometryChanged")
                        if minimum_frames > 1 and len(
                            {hashlib.sha256(item).digest() for item in frames}
                        ) < 2:
                            raise ProbeError("MjpegStreamStalled")
                        return frames
    except ProbeError:
        raise
    except (OSError, TimeoutError, urllib.error.URLError) as exc:
        raise ProbeError("MjpegReadFailed") from exc
    if total_read > MAX_MJPEG_BYTES:
        raise ProbeError("MjpegFrameTooLarge")
    raise ProbeError(
        "MjpegFirstFrameMissing" if not frames else "MjpegFrameSequenceIncomplete"
    )


def _read_first_jpeg(
    url: str,
    header_name: str,
    token: str,
    timeout_seconds: float,
) -> bytes:
    return read_mjpeg_sequence(
        url,
        header_name,
        token,
        minimum_frames=1,
        timeout_seconds=timeout_seconds,
    )[0]


def start_session_then_mjpeg(
    control_base: str,
    mjpeg_url: str,
    header_name: str,
    token: str,
    *,
    timeout_seconds: float = REQUEST_TIMEOUT_SECONDS,
) -> StartupProof:
    events: list[str] = []
    status, raw = _request(
        "POST",
        control_base.rstrip("/") + "/session",
        header_name,
        token,
        {"capabilities": {"alwaysMatch": {}}},
        timeout_seconds,
    )
    if status != 200:
        raise ProbeError(f"SessionCreateFailed: HTTP {status}")
    payload = _strict_json(raw, "session response")
    session_id = None
    if isinstance(payload, dict):
        value = payload.get("value")
        if isinstance(value, dict):
            session_id = value.get("sessionId")
        session_id = session_id or payload.get("sessionId")
    if not isinstance(session_id, str) or not session_id:
        raise ProbeError("SessionIdMissing")
    events.append("session")
    frame = _read_first_jpeg(mjpeg_url, header_name, token, timeout_seconds)
    events.extend(("mjpeg-connect", "first-frame"))
    return StartupProof(events, _jpeg_dimensions(frame), hashlib.sha256(frame).hexdigest())


def decode_bounded_clipboard(value: str, maximum: int = MAX_CLIPBOARD_BYTES) -> bytes:
    if maximum != MAX_CLIPBOARD_BYTES:
        raise ProbeError("ClipboardLimitMustBe65536")
    if not isinstance(value, str):
        raise ProbeError("ClipboardInvalidBase64")
    if len(value) > 4 * ((maximum + 2) // 3):
        raise ProbeError("ClipboardTooLarge")
    try:
        decoded = base64.b64decode(value, validate=True)
    except (ValueError, binascii.Error) as exc:
        raise ProbeError("ClipboardInvalidBase64") from exc
    if len(decoded) > maximum:
        raise ProbeError("ClipboardTooLarge")
    return decoded


def require_controlled_clipboard_fixture(value: bytes) -> None:
    if value != CLIPBOARD_FIXTURE_VALUE:
        raise ProbeError("ClipboardFixtureMissing")


def clipboard_access_sequence(mode: str) -> list[str]:
    if mode == "targetBackgroundSafe":
        return ["verify-tiktok", "clipboard-set", "clipboard-read", "verify-tiktok"]
    if mode == "agentForegroundRequired":
        return [
            "stop-stream",
            "advance-generation",
            "foreground-agent",
            "verify-agent-pid-before",
            "clipboard-set",
            "clipboard-read",
            "verify-agent-pid-after",
            "foreground-tiktok",
            "verify-tiktok-pid",
            "session",
            "mjpeg-connect",
            "first-frame",
        ]
    raise ProbeError("ClipboardModeUnsupported")


def _identity_fields(value: dict[str, Any]) -> tuple[str, int]:
    if not isinstance(value, dict):
        raise ProbeError("ActiveIdentityInvalid")
    bundle = value.get("bundleId") or value.get("bundleID")
    pid = value.get("pid") or value.get("processId")
    if not isinstance(bundle, str) or not bundle:
        raise ProbeError("ActiveBundleMissing")
    if not isinstance(pid, int) or isinstance(pid, bool) or pid <= 0:
        raise ProbeError("ActivePidMissing")
    return bundle, pid


def prove_stable_app_identity(
    before: dict[str, Any], after: dict[str, Any], expected_bundle: str
) -> AppIdentityProof:
    before_bundle, before_pid = _identity_fields(before)
    after_bundle, after_pid = _identity_fields(after)
    if before_bundle != expected_bundle or after_bundle != expected_bundle:
        raise ProbeError("ActiveBundleMismatch")
    if before_pid != after_pid:
        raise ProbeError("ActivePidMismatch")
    return AppIdentityProof(expected_bundle, before_pid)


def validate_geometry(evidence: GeometryEvidence) -> tuple[float, float]:
    if evidence.orientation != "portrait":
        raise ProbeError("UnsupportedOrientation")
    if (evidence.logical_width, evidence.logical_height) != EXPECTED_LOGICAL_BOUNDS:
        raise ProbeError("UnsupportedBounds")
    if min(evidence.frame_width, evidence.frame_height) <= 0:
        raise ProbeError("UnsupportedScale")
    scale_x = evidence.frame_width / evidence.logical_width
    scale_y = evidence.frame_height / evidence.logical_height
    if not all(math.isfinite(value) and value > 0 for value in (scale_x, scale_y)):
        raise ProbeError("UnsupportedScale")
    if abs(scale_x - scale_y) > 1e-9 or scale_x not in (1.0, 2.0, 3.0):
        raise ProbeError("UnsupportedScale")
    return scale_x, scale_y


def geometry_from_mobilegestalt(
    metrics: dict[str, Any],
    orientation: str,
    frame_size: tuple[int, int],
) -> GeometryEvidence:
    if orientation != "UIDeviceOrientationPortrait":
        raise ProbeError("UnsupportedOrientation")
    if not isinstance(metrics, dict):
        raise ProbeError("UnsupportedGeometry")
    values = tuple(
        metrics.get(key)
        for key in ("main-screen-width", "main-screen-height", "main-screen-scale")
    )
    if any(
        isinstance(value, bool)
        or not isinstance(value, (int, float))
        or not math.isfinite(float(value))
        or float(value) <= 0
        for value in values
    ):
        raise ProbeError("UnsupportedGeometry")
    physical_width, physical_height, scale = (float(value) for value in values)
    frame_width, frame_height = frame_size
    if (
        isinstance(frame_width, bool)
        or isinstance(frame_height, bool)
        or not isinstance(frame_width, int)
        or not isinstance(frame_height, int)
        or frame_width <= 0
        or frame_height <= 0
    ):
        raise ProbeError("UnsupportedGeometry")
    if abs(physical_width - frame_width) > 1e-9 or abs(physical_height - frame_height) > 1e-9:
        raise ProbeError("UnsupportedScale")
    logical_width = physical_width / scale
    logical_height = physical_height / scale
    if abs(logical_width - round(logical_width)) > 1e-9 or abs(
        logical_height - round(logical_height)
    ) > 1e-9:
        raise ProbeError("UnsupportedScale")
    return GeometryEvidence(
        int(round(logical_width)),
        int(round(logical_height)),
        frame_width,
        frame_height,
        "portrait",
    )


def find_share_control(jpeg: bytes) -> tuple[float, float]:
    try:
        from PIL import Image

        with Image.open(io.BytesIO(jpeg)) as source:
            image = source.convert("RGB")
    except (OSError, ValueError) as exc:
        raise ProbeError("ShareFrameInvalid") from exc
    width, height = image.size
    if width <= 0 or height <= 0:
        raise ProbeError("ShareFrameInvalid")
    x0 = int((0.919 - 0.04) * width)
    x1 = min(width, int((0.919 + 0.04) * width))
    y0 = int(0.28 * height)
    y1 = min(height, int(0.85 * height))
    if x1 <= x0 or y1 <= y0:
        raise ProbeError("ShareControlNotFound")
    minimum_white = max(1, math.ceil((x1 - x0) * 0.35))
    runs: list[tuple[int, int]] = []
    start: int | None = None
    pixels = image.load()
    for y in range(y0, y1):
        white = 0
        for x in range(x0, x1):
            red, green, blue = pixels[x, y]
            low = min(red, green, blue)
            high = max(red, green, blue)
            if low > 190 and high - low < 40:
                white += 1
        glyph = white >= minimum_white
        if glyph and start is None:
            start = y
        elif not glyph and start is not None:
            runs.append((start, y))
            start = None
    if start is not None:
        runs.append((start, y1))
    minimum_height = max(1, int(0.0045 * height))
    centres = [
        (start_y + end_y) / 2.0 / height
        for start_y, end_y in runs
        if end_y - start_y >= minimum_height
    ]
    best: list[float] = []
    minimum_pitch = 55.0 / 667.0
    maximum_pitch = 80.0 / 667.0
    for index, centre in enumerate(centres):
        chain = [centre]
        for candidate in centres[index + 1 :]:
            gap = candidate - chain[-1]
            if minimum_pitch <= gap <= maximum_pitch:
                chain.append(candidate)
        if len(chain) > len(best):
            best = chain
    if len(best) < 3:
        raise ProbeError("ShareControlNotFound")
    return 0.919, best[-1]


def select_copy_link_observation(observations: Any) -> tuple[float, float]:
    if not isinstance(observations, list) or not observations or len(observations) > 256:
        raise ProbeError("CopyLinkOcrInvalid")
    matches: list[tuple[float, float, float]] = []
    required_fields = {"text", "confidence", "x", "y", "width", "height"}
    for observation in observations:
        if not isinstance(observation, dict) or set(observation) != required_fields:
            raise ProbeError("CopyLinkOcrInvalid")
        text = observation["text"]
        numeric = tuple(
            observation[field] for field in ("confidence", "x", "y", "width", "height")
        )
        if not isinstance(text, str) or any(
            isinstance(value, bool)
            or not isinstance(value, (int, float))
            or not math.isfinite(float(value))
            for value in numeric
        ):
            raise ProbeError("CopyLinkOcrInvalid")
        confidence, x, y, width, height = (float(value) for value in numeric)
        if (
            confidence < 0.55
            or x < 0
            or y < 0
            or width <= 0
            or height <= 0
            or x + width > 1
            or y + height > 1
        ):
            continue
        label = " ".join(unicodedata.normalize("NFKC", text).casefold().split())
        if label in {"copy link", "sao ch\u00e9p li\u00ean k\u1ebft"}:
            matches.append((confidence, x + width / 2.0, y + height / 2.0))
    if len(matches) != 1:
        raise ProbeError("CopyLinkControlNotFound")
    _, center_x, center_y = matches[0]
    return center_x, center_y


def _vision_environment() -> dict[str, str]:
    return {
        key: os.environ[key]
        for key in ("PATH", "HOME", "TMPDIR", "LANG", "LC_ALL")
        if key in os.environ
    }


def run_vision_copy_link(jpeg: bytes) -> tuple[float, float, str]:
    if platform.system() != "Darwin":
        raise ProbeError("VisionRequiresMac")
    if not VISION_HELPER.is_file():
        raise ProbeError("VisionHelperMissing")
    with tempfile.TemporaryDirectory(prefix="riviu-gate0-vision-") as raw:
        image_path = Path(raw) / "frame.jpg"
        image_path.write_bytes(jpeg)
        try:
            completed = subprocess.run(
                ["xcrun", "swift", str(VISION_HELPER), str(image_path)],
                check=True,
                capture_output=True,
                timeout=30,
                env=_vision_environment(),
            )
        except (OSError, subprocess.SubprocessError) as exc:
            raise ProbeError("VisionOcrFailed") from exc
    observations = _strict_json(completed.stdout, "Vision OCR output")
    point = select_copy_link_observation(observations)
    canonical = json.dumps(
        observations, ensure_ascii=True, sort_keys=True, separators=(",", ":")
    ).encode("ascii")
    return point[0], point[1], hashlib.sha256(canonical).hexdigest()


def detector_attestation() -> dict[str, Any]:
    if not VISION_HELPER.is_file():
        raise ProbeError("VisionHelperMissing")
    probe_sha = _sha256_file(Path(__file__).resolve())
    helper_sha = _sha256_file(VISION_HELPER)
    contract = {
        "shareDetectorVersion": SHARE_DETECTOR_VERSION,
        "copyLinkDetectorVersion": COPY_LINK_DETECTOR_VERSION,
        "layoutId": LAYOUT_ID,
        "probeSha256": probe_sha,
        "visionHelperSha256": helper_sha,
        "visionLanguages": ["en-US", "vi-VN"],
        "visionRequestRevision": 3,
        "visionRecognitionLevel": "accurate",
        "visionLanguageCorrection": True,
        "minimumConfidence": 0.55,
    }
    canonical = json.dumps(
        contract, ensure_ascii=True, sort_keys=True, separators=(",", ":")
    ).encode("ascii")
    return {**contract, "detectorSetSha256": hashlib.sha256(canonical).hexdigest()}


def live_dependency_attestation(
    version_getter: Callable[[str], str] = importlib.metadata.version,
) -> dict[str, str]:
    if not REQUIREMENTS_MAC.is_file():
        raise ProbeError("LiveDependencyLockMissing")
    locked = {
        line.strip()
        for line in REQUIREMENTS_MAC.read_text(encoding="utf-8").splitlines()
        if line.strip() and not line.lstrip().startswith("#")
    }
    expected_locks = {
        f"{distribution}=={version}"
        for distribution, version in EXPECTED_LIVE_DEPENDENCIES.items()
    }
    if locked != expected_locks:
        raise ProbeError("LiveDependencyLockMismatch")
    installed: dict[str, str] = {}
    for distribution, expected in EXPECTED_LIVE_DEPENDENCIES.items():
        try:
            actual = version_getter(distribution)
        except importlib.metadata.PackageNotFoundError as exc:
            raise ProbeError("LiveDependencyMissing") from exc
        if actual != expected:
            raise ProbeError("LiveDependencyVersionMismatch")
        installed[distribution] = actual
    return {
        "pymobiledevice3Version": installed["pymobiledevice3"],
        "pillowVersion": installed["Pillow"],
        "requirementsMacSha256": _sha256_file(REQUIREMENTS_MAC),
        "pythonVersion": platform.python_version(),
    }


def host_vision_attestation() -> dict[str, str]:
    macos_version = platform.mac_ver()[0]
    if platform.system() != "Darwin" or not macos_version:
        raise ProbeError("VisionHostUnsupported")
    try:
        completed = subprocess.run(
            ["xcrun", "swift", "--version"],
            check=True,
            capture_output=True,
            timeout=10,
            env=_vision_environment(),
        )
    except (OSError, subprocess.SubprocessError) as exc:
        raise ProbeError("VisionHostInspectionFailed") from exc
    if not completed.stdout or len(completed.stdout) > 16_384:
        raise ProbeError("VisionHostInspectionFailed")
    return {
        "macOsVersion": macos_version,
        "swiftVersionSha256": hashlib.sha256(completed.stdout).hexdigest(),
        **live_dependency_attestation(),
    }


def run_cleanup_steps(steps: Sequence[Callable[[], Any]]) -> None:
    errors: list[str] = []
    for step in steps:
        try:
            step()
        except Exception as exc:
            errors.append(type(exc).__name__)
    if errors:
        raise ProbeError(f"CleanupFailed: errors={errors!r}")


def _port_open(host: str, port: int, timeout: float = 0.1) -> bool:
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as client:
        client.settimeout(timeout)
        return client.connect_ex((host, port)) == 0


def cleanup_and_verify_ports(
    resources: Sequence[Any],
    ports: Sequence[tuple[str, int]],
    *,
    timeout_seconds: float = 5.0,
) -> None:
    cleanup_errors: list[str] = []
    for resource in reversed(resources):
        try:
            close = getattr(resource, "close", None) or getattr(resource, "stop", None)
            if close is not None:
                close()
            else:
                shutdown = getattr(resource, "shutdown")
                shutdown()
                server_close = getattr(resource, "server_close", None)
                if server_close is not None:
                    server_close()
        except Exception as exc:  # cleanup must continue through every resource
            cleanup_errors.append(type(exc).__name__)
    deadline = time.monotonic() + timeout_seconds
    still_open = list(ports)
    while still_open and time.monotonic() < deadline:
        still_open = [(host, port) for host, port in still_open if _port_open(host, port)]
        if still_open:
            time.sleep(0.05)
    if cleanup_errors or still_open:
        raise ProbeError(
            f"CleanupFailed: errors={cleanup_errors!r} openPortCount={len(still_open)}"
        )


def _secret_variants(secret: str) -> set[bytes]:
    raw = secret.encode("utf-8")
    variants = {
        raw,
        raw.hex().encode("ascii"),
        raw.hex().upper().encode("ascii"),
        base64.b64encode(raw),
        base64.urlsafe_b64encode(raw),
        json.dumps(secret, ensure_ascii=True)[1:-1].encode("ascii"),
        secret.encode("utf-16-le"),
        secret.encode("utf-16-be"),
    }
    variants.update(value.rstrip(b"=") for value in tuple(variants) if value.endswith(b"="))
    return {value for value in variants if value}


def _json_string_leaves(value: Any) -> Iterable[str]:
    if isinstance(value, str):
        yield value
    elif isinstance(value, dict):
        for key, child in value.items():
            yield key
            yield from _json_string_leaves(child)
    elif isinstance(value, list):
        for child in value:
            yield from _json_string_leaves(child)


def verify_redaction(payloads: Sequence[bytes], secrets: Sequence[str]) -> None:
    secret_values = [secret for secret in secrets if secret]
    patterns = set().union(*(_secret_variants(secret) for secret in secret_values))
    for payload in payloads:
        if not isinstance(payload, bytes):
            raise ProbeError("RedactionInputInvalid")
        if any(pattern in payload for pattern in patterns):
            raise ProbeError("RedactionViolation: raw bytes")
        stripped = payload.lstrip()
        if not stripped.startswith((b"{", b"[")):
            continue
        decoded = _strict_json(payload, "published report")
        for leaf in _json_string_leaves(decoded):
            if any(secret in leaf for secret in secret_values):
                raise ProbeError("RedactionViolation: decoded JSON leaf")


def fixture_report(artifact: dict[str, Any]) -> dict[str, Any]:
    return {
        "schemaVersion": 1,
        "environment": FIXTURE_ENVIRONMENT,
        "gateStatus": FIXTURE_ENVIRONMENT,
        "productionQualified": False,
        "qualifications": [],
        "artifact": dict(artifact),
        "outcomes": {
            "fixtureContractsExercised": True,
            "registryModified": False,
        },
    }


def _write_synced(path: Path, content: bytes) -> None:
    with path.open("wb") as destination:
        destination.write(content)
        destination.flush()
        os.fsync(destination.fileno())


def _sync_directory(path: Path) -> None:
    if platform.system() == "Windows":
        return
    descriptor = os.open(path, os.O_RDONLY)
    try:
        os.fsync(descriptor)
    finally:
        os.close(descriptor)


def recover_report_publication(report_dir: Path) -> None:
    report_dir = Path(report_dir)
    journal_path = report_dir / ".gate0-publish-journal.json"
    if not journal_path.exists():
        return
    try:
        journal = _strict_json(journal_path.read_bytes(), "report publication journal")
        required = {
            "schemaVersion",
            "transaction",
            "hadPrior",
            "priorSha256",
            "stagedSha256",
            "state",
        }
        if not isinstance(journal, dict) or set(journal) != required:
            raise ProbeError("ReportRecoveryJournalInvalid")
        transaction_name = journal.get("transaction")
        had_prior = journal.get("hadPrior")
        prior_hashes = journal.get("priorSha256")
        staged_hashes = journal.get("stagedSha256")
        state = journal.get("state")
        if (
            journal.get("schemaVersion") != 1
            or not isinstance(transaction_name, str)
            or not transaction_name.startswith(".gate0-publish-")
            or Path(transaction_name).name != transaction_name
            or not isinstance(had_prior, list)
            or len(had_prior) != 2
            or any(type(value) is not bool for value in had_prior)
            or not isinstance(prior_hashes, list)
            or len(prior_hashes) != 2
            or not isinstance(staged_hashes, list)
            or len(staged_hashes) != 2
            or any(not isinstance(value, str) or not SHA256_RE.fullmatch(value) for value in staged_hashes)
            or any(
                (had_prior[index] and (
                    not isinstance(prior_hashes[index], str)
                    or not SHA256_RE.fullmatch(prior_hashes[index])
                ))
                or (not had_prior[index] and prior_hashes[index] is not None)
                for index in range(2)
            )
            or state not in {"replacing", "committed"}
        ):
            raise ProbeError("ReportRecoveryJournalInvalid")
        transaction = report_dir / transaction_name
        if transaction.resolve().parent != report_dir.resolve() or not transaction.is_dir():
            raise ProbeError("ReportRecoveryJournalInvalid")
        destinations = (report_dir / "gate-0.json", report_dir / "gate-0.md")

        def pair_matches(hashes: Sequence[str]) -> bool:
            return all(
                destination.is_file() and _sha256_file(destination) == hashes[ordinal]
                for ordinal, destination in enumerate(destinations)
            )

        if state == "replacing" or not pair_matches(staged_hashes):
            for ordinal, destination in enumerate(destinations):
                if had_prior[ordinal]:
                    prior = transaction / f"prior-{ordinal}"
                    if not prior.is_file():
                        raise ProbeError("ReportRecoverySnapshotMissing")
                    if _sha256_file(prior) != prior_hashes[ordinal]:
                        raise ProbeError("ReportRecoverySnapshotMismatch")
                    restoration = transaction / f"restore-{ordinal}"
                    _write_synced(restoration, prior.read_bytes())
                    os.replace(restoration, destination)
                else:
                    destination.unlink(missing_ok=True)
            restored = all(
                (
                    destination.is_file()
                    and _sha256_file(destination) == prior_hashes[ordinal]
                )
                if had_prior[ordinal]
                else not destination.exists()
                for ordinal, destination in enumerate(destinations)
            )
            if not restored:
                raise ProbeError("ReportRecoveryVerificationFailed")
        journal_path.unlink()
        _sync_directory(report_dir)
        shutil.rmtree(transaction)
        _sync_directory(report_dir)
    except ProbeError:
        raise
    except OSError as exc:
        raise ProbeError("ReportRecoveryFailed") from exc


def publish_reports(
    report: dict[str, Any],
    markdown: str,
    report_dir: Path,
    secrets: Sequence[str],
    replace: Callable[[str | os.PathLike[str], str | os.PathLike[str]], Any] = os.replace,
    checkpoint: Callable[[str], Any] | None = None,
) -> tuple[Path, Path]:
    report_dir = Path(report_dir)
    json_bytes = (json.dumps(report, ensure_ascii=True, indent=2, sort_keys=True) + "\n").encode(
        "ascii"
    )
    markdown_bytes = markdown.encode("ascii")
    verify_redaction([json_bytes, markdown_bytes], secrets)
    report_dir.mkdir(parents=True, exist_ok=True)
    recover_report_publication(report_dir)
    destinations = (report_dir / "gate-0.json", report_dir / "gate-0.md")
    transaction = Path(tempfile.mkdtemp(prefix=".gate0-publish-", dir=report_dir))
    staged = (transaction / "gate-0.json", transaction / "gate-0.md")
    journal_path = report_dir / ".gate0-publish-journal.json"
    journal_staging = transaction / "journal-staging"
    had_prior: list[bool] = []
    prior_hashes: list[str | None] = []
    staged_hashes = [hashlib.sha256(json_bytes).hexdigest(), hashlib.sha256(markdown_bytes).hexdigest()]
    mark = checkpoint or (lambda _label: None)
    try:
        for ordinal, destination in enumerate(destinations):
            exists = destination.exists()
            had_prior.append(exists)
            if exists:
                prior = destination.read_bytes()
                prior_hashes.append(hashlib.sha256(prior).hexdigest())
                _write_synced(transaction / f"prior-{ordinal}", prior)
            else:
                prior_hashes.append(None)
        _write_synced(staged[0], json_bytes)
        _write_synced(staged[1], markdown_bytes)
        journal = {
            "schemaVersion": 1,
            "transaction": transaction.name,
            "hadPrior": had_prior,
            "priorSha256": prior_hashes,
            "stagedSha256": staged_hashes,
            "state": "replacing",
        }
        _write_synced(
            journal_staging,
            json.dumps(journal, ensure_ascii=True, separators=(",", ":")).encode("ascii"),
        )
        os.replace(journal_staging, journal_path)
        _sync_directory(report_dir)
        mark("journal-replacing-synced")
        replace(staged[0], destinations[0])
        _sync_directory(report_dir)
        mark("json-replaced")
        replace(staged[1], destinations[1])
        _sync_directory(report_dir)
        mark("markdown-replaced")
        if any(
            not destination.is_file() or _sha256_file(destination) != staged_hashes[ordinal]
            for ordinal, destination in enumerate(destinations)
        ):
            raise ProbeError("ReportPublishVerificationFailed")
        mark("destinations-verified")
        journal["state"] = "committed"
        _write_synced(
            journal_staging,
            json.dumps(journal, ensure_ascii=True, separators=(",", ":")).encode("ascii"),
        )
        os.replace(journal_staging, journal_path)
        _sync_directory(report_dir)
        mark("journal-committed-synced")
        committed = _strict_json(journal_path.read_bytes(), "committed publication journal")
        if committed != journal:
            raise ProbeError("ReportPublishJournalMismatch")
        journal_path.unlink()
        _sync_directory(report_dir)
        mark("journal-removed-synced")
    except Exception as exc:
        try:
            if journal_path.exists():
                recover_report_publication(report_dir)
            else:
                shutil.rmtree(transaction, ignore_errors=True)
        except Exception as rollback_error:
            raise ProbeError("ReportPublishFailed: rollback failed") from rollback_error
        raise ProbeError("ReportPublishFailed: prior files restored") from exc
    shutil.rmtree(transaction)
    _sync_directory(report_dir)
    return destinations


def _redacted_device_hash(udid: str) -> str:
    return hashlib.sha256(udid.encode("utf-8")).hexdigest()


def _pending_report(args: argparse.Namespace, artifact: ArtifactProof) -> dict[str, Any]:
    return {
        "schemaVersion": 1,
        "environment": PENDING_ENVIRONMENT,
        "gateStatus": PENDING_ENVIRONMENT,
        "productionQualified": False,
        "qualifications": [],
        "artifact": {
            "ipaSha256": artifact.ipa_sha256,
            "manifestSha256": artifact.manifest_sha256,
        },
        "tuple": {
            "deviceSha256": _redacted_device_hash(args.udid),
            "targetBundleSha256": hashlib.sha256(args.tiktok_bundle.encode("utf-8")).hexdigest(),
            "targetUrlSha256": {
                "direct": hashlib.sha256(args.direct_url.encode("utf-8")).hexdigest(),
                "photo": hashlib.sha256(args.photo_url.encode("utf-8")).hexdigest(),
                "short": hashlib.sha256(args.short_url.encode("utf-8")).hexdigest(),
            },
        },
        "outcomes": {
            "code": "PENDING_MAC_DEVICE",
            "registryModified": False,
            "cleanup": "not-started",
        },
    }


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        description="Run the fixed TikTok Interaction Gate 0 device matrix"
    )
    parser.add_argument("--udid", required=True)
    parser.add_argument("--ipa", type=Path, required=True)
    parser.add_argument("--agent-manifest", type=Path, required=True)
    parser.add_argument("--token-env", required=True)
    parser.add_argument("--tiktok-bundle", required=True)
    parser.add_argument("--direct-url", required=True)
    parser.add_argument("--photo-url", required=True)
    parser.add_argument("--short-url", required=True)
    parser.add_argument("--report-dir", type=Path, required=True)
    return parser


def _validate_fresh_install_identity(
    installed: dict[str, Any],
    ipa_identity: IpaIdentity,
    manifest: dict[str, Any],
) -> dict[str, Any]:
    installed_path = installed.get("Path")
    payload_app = (
        PurePosixPath(installed_path).name
        if isinstance(installed_path, str) and installed_path
        else None
    )
    expected = {
        "CFBundleIdentifier": ipa_identity.bundle_id,
        "CFBundleShortVersionString": ipa_identity.version,
        "CFBundleVersion": ipa_identity.build,
        "CFBundleExecutable": ipa_identity.executable_name,
        "SignerIdentity": manifest.get("signerIdentity"),
    }
    if payload_app != ipa_identity.payload_app or any(
        installed.get(key) != value for key, value in expected.items()
    ):
        raise ProbeError("FreshInstallIdentityMismatch")
    return {
        "freshInstall": True,
        "identityMatch": True,
        "payloadApp": payload_app,
        "applicationType": installed.get("ApplicationType", "unknown"),
    }


def _fresh_install_hashed_agent(
    udid: str,
    ipa_path: Path,
    artifact: ArtifactProof,
    ipa_identity: IpaIdentity,
) -> dict[str, Any]:
    if _sha256_file(ipa_path) != artifact.ipa_sha256:
        raise ProbeError("ArtifactChangedBeforeFreshInstall")

    async def install() -> dict[str, Any]:
        from pymobiledevice3.lockdown import create_using_usbmux
        from pymobiledevice3.services.installation_proxy import InstallationProxyService

        provider = await create_using_usbmux(serial=udid, autopair=False)
        try:
            if getattr(provider, "udid", None) != udid:
                raise ProbeError("DeviceMetadataIdentityMismatch")
            async with InstallationProxyService(lockdown=provider) as proxy:
                before = await proxy.get_apps(
                    bundle_identifiers=[ipa_identity.bundle_id]
                )
                if ipa_identity.bundle_id in before:
                    await proxy.uninstall(ipa_identity.bundle_id)
                await proxy.install_from_local(str(ipa_path), developer=True)
                after = await proxy.get_apps(
                    bundle_identifiers=[ipa_identity.bundle_id]
                )
                installed = after.get(ipa_identity.bundle_id)
                if not isinstance(installed, dict):
                    raise ProbeError("FreshInstallMissing")
                return installed
        finally:
            await provider.close()

    installed = _run_async(install(), 180.0, "fresh install hashed Agent")
    if _sha256_file(ipa_path) != artifact.ipa_sha256:
        raise ProbeError("ArtifactChangedDuringFreshInstall")
    return _validate_fresh_install_identity(
        installed, ipa_identity, artifact.manifest
    )


def _inspect_device(args: argparse.Namespace, agent_bundle: str) -> dict[str, Any]:
    repository = Path(__file__).resolve().parents[2]
    sidecar = repository / "sidecars" / "pymobiledevice3" / "riviu_pmd.py"
    command = [
        sys.executable,
        str(sidecar),
        "inspect-device-capabilities",
        "--udid",
        args.udid,
        "--target-bundle-id",
        args.tiktok_bundle,
        "--agent-bundle-id",
        agent_bundle,
    ]
    try:
        completed = subprocess.run(
            command,
            check=True,
            capture_output=True,
            text=True,
            timeout=30,
            env={key: value for key, value in os.environ.items() if key != args.token_env},
        )
    except (OSError, subprocess.SubprocessError) as exc:
        raise ProbeError("DeviceMetadataInspectionFailed") from exc
    metadata = _strict_json(completed.stdout.encode("utf-8"), "device metadata")
    if not isinstance(metadata, dict) or metadata.get("ok") is not True:
        raise ProbeError("DeviceMetadataInspectionFailed")
    if metadata.get("udid") != args.udid:
        raise ProbeError("DeviceMetadataIdentityMismatch")
    return metadata


def _response_value(raw: bytes, label: str) -> Any:
    payload = _strict_json(raw, label)
    return payload.get("value") if isinstance(payload, dict) else None


def _validate_tiktok_resolution_url(url: str) -> urllib.parse.SplitResult:
    try:
        parsed = urllib.parse.urlsplit(url)
    except ValueError as exc:
        raise ProbeError("TargetUrlInvalid") from exc
    host = (parsed.hostname or "").lower().rstrip(".")
    try:
        port = parsed.port
    except ValueError as exc:
        raise ProbeError("TargetUrlUnsupported") from exc
    if (
        parsed.scheme.lower() != "https"
        or parsed.username is not None
        or parsed.password is not None
        or port not in (None, 443)
        or not (host == "tiktok.com" or host.endswith(".tiktok.com"))
    ):
        raise ProbeError("TargetUrlUnsupported")
    return parsed


def _normalize_tiktok_post(url: str) -> PostIdentity:
    parsed = _validate_tiktok_resolution_url(url)
    match = re.search(r"/(video|photo)/(\d+)(?:/|$)", parsed.path)
    if match is None:
        raise ProbeError("TargetUrlUnsupported")
    return PostIdentity(match.group(2), match.group(1))


class _BoundedRedirect(urllib.request.HTTPRedirectHandler):
    def __init__(self, maximum: int = 5) -> None:
        super().__init__()
        self.maximum = maximum
        self.count = 0

    def _next_url(self, request_url: str, new_url: str) -> str:
        self.count += 1
        if self.count > self.maximum:
            raise ProbeError("TargetResolveRedirectLimit")
        resolved = urllib.parse.urljoin(request_url, new_url)
        _validate_tiktok_resolution_url(resolved)
        return resolved

    def redirect_request(self, request, fp, code, msg, headers, newurl):
        resolved = self._next_url(request.full_url, newurl)
        return super().redirect_request(request, fp, code, msg, headers, resolved)


def _resolve_tiktok_post(url: str) -> tuple[PostIdentity, str]:
    _validate_tiktok_resolution_url(url)
    try:
        return _normalize_tiktok_post(url), url
    except ProbeError:
        pass
    opener = urllib.request.build_opener(_BoundedRedirect())
    request = urllib.request.Request(
        url,
        headers={"User-Agent": "Riviu-Gate0/1", "Range": "bytes=0-0"},
        method="GET",
    )
    try:
        with opener.open(request, timeout=REQUEST_TIMEOUT_SECONDS) as response:
            final_url = response.geturl()
            response.read(1)
    except ProbeError:
        raise
    except (OSError, TimeoutError, urllib.error.URLError) as exc:
        raise ProbeError("TargetResolveFailed") from exc
    return _normalize_tiktok_post(final_url), final_url


def interaction_route_contracts(target_bundle_id: str) -> dict[str, Any]:
    common = {
        "authHeaderName": "X-RT-Token",
        "requestTimeoutMs": 10_000,
    }
    return {
        "protectedHealth": {
            **common,
            "method": "get",
            "scope": "sessionless",
            "path": "/wda/locked",
            "bodySchemaId": "none",
        },
        "createSession": {
            **common,
            "method": "post",
            "scope": "sessionless",
            "path": "/session",
            "bodySchemaId": "w3c-empty-capabilities-v1",
        },
        "openUrl": {
            **common,
            "contractId": "rtmmo-open-url-v1",
            "method": "post",
            "scope": "sessionless",
            "path": "/url",
            "bodySchemaId": "open-url-body-v1",
            "targetBundleId": target_bundle_id,
        },
        "clipboardSet": {
            **common,
            "method": "post",
            "scope": "sessionless",
            "path": "/wda/setPasteboard",
            "bodySchemaId": "clipboard-set-base64-v1",
        },
        "clipboardGet": {
            **common,
            "method": "post",
            "scope": "sessionless",
            "path": "/wda/getPasteboard",
            "bodySchemaId": "clipboard-get-base64-v1",
        },
        "activeApp": {
            **common,
            "method": "get",
            "scope": "sessionless",
            "path": "/wda/activeAppInfo",
            "bodySchemaId": "none",
        },
        "deviceOrientation": {
            **common,
            "method": "get",
            "scope": "sessionless",
            "path": "/wda/deviceOrientation",
            "bodySchemaId": "none",
        },
        "nativeSwipe": {
            **common,
            "method": "post",
            "scope": "sessionless",
            "path": "/wda/swipe",
            "bodySchemaId": "rtmmo-native-swipe-v1",
        },
        "statusWitness": {
            "method": "get",
            "scope": "sessionless",
            "path": "/status",
            "authHeaderName": "none",
            "bodySchemaId": "none",
            "requestTimeoutMs": 10_000,
        },
        "mjpeg": {
            **common,
            "method": "get",
            "scope": "sessionless",
            "path": "/",
            "bodySchemaId": "multipart-mjpeg-v1",
        },
    }


def route_contract_sha256(contracts: dict[str, Any]) -> str:
    canonical = json.dumps(
        contracts, ensure_ascii=True, sort_keys=True, separators=(",", ":")
    ).encode("ascii")
    return hashlib.sha256(canonical).hexdigest()


class MacGateAdapter:
    CONTROL_PORT = 8906
    MJPEG_PORT = 9093
    TOKEN_HEADER = "X-RT-Token"

    def __init__(
        self,
        udid: str,
        token: str,
        agent_bundle: str,
        tiktok_bundle: str,
        logical_bounds: tuple[int, int],
    ) -> None:
        self.udid = udid
        self.token = token
        self.agent_bundle = agent_bundle
        self.tiktok_bundle = tiktok_bundle
        self.logical_bounds = logical_bounds
        self.control_relay: UsbmuxRelay | None = None
        self.mjpeg_relay: UsbmuxRelay | None = None
        self.mjpeg_reader: MjpegReader | None = None
        self.session_id: str | None = None
        self.agent_pid: int | None = None
        self.tiktok_pid: int | None = None
        self.latest_frame: bytes | None = None
        self.latest_frame_sequence = 0
        self.generation = 0
        self.trace: list[str] = []
        self.route_auth: dict[str, list[int]] = {}
        self._auth_counts: dict[str, int] = {}
        self._geometry: GeometryEvidence | None = None
        self._observed_agent_pids: set[int] = set()
        self.share_sheet_open = False

    @property
    def control_base(self) -> str:
        if self.control_relay is None:
            raise ProbeError("ControlRelayMissing")
        return f"http://127.0.0.1:{self.control_relay.local_port}"

    @property
    def mjpeg_url(self) -> str:
        if self.mjpeg_relay is None:
            raise ProbeError("MjpegRelayMissing")
        return f"http://127.0.0.1:{self.mjpeg_relay.local_port}/"

    def _control(
        self, method: str, path: str, body: dict[str, Any] | None = None
    ) -> Any:
        status, raw = _request(
            method,
            self.control_base + path,
            self.TOKEN_HEADER,
            self.token,
            body,
            REQUEST_TIMEOUT_SECONDS,
        )
        if status != 200:
            raise ProbeError(f"ControlRouteFailed: {method} {path} HTTP {status}")
        return _response_value(raw, f"{method} {path}")

    def _auth_label(self, prefix: str) -> str:
        count = self._auth_counts.get(prefix, 0) + 1
        self._auth_counts[prefix] = count
        return prefix if count == 1 else f"{prefix}{count}"

    def _control_with_auth(
        self,
        method: str,
        path: str,
        body: dict[str, Any] | None,
        auth_prefix: str,
        *,
        return_payload: bool = False,
    ) -> Any:
        statuses, raw = _route_auth_exchange(
            method,
            self.control_base + path,
            self.TOKEN_HEADER,
            self.token,
            body,
            timeout_seconds=REQUEST_TIMEOUT_SECONDS,
        )
        self.route_auth[self._auth_label(auth_prefix)] = list(statuses)
        if return_payload:
            return _strict_json(raw, f"{method} {path}")
        return _response_value(raw, f"{method} {path}")

    def _process_id(self, bundle_id: str) -> int | None:
        async def identify(process_control):
            return await process_control.process_identifier_for_bundle_identifier(bundle_id)

        value = _run_async(
            _with_process_control(self.udid, identify), 30.0, "process identity"
        )
        if not value:
            return None
        if isinstance(value, bool) or not isinstance(value, int) or value <= 0:
            raise ProbeError("ProcessIdentityInvalid")
        return value

    def _terminate_agent(self, expected_pid: int | None = None) -> int | None:
        async def terminate(process_control):
            pid = await process_control.process_identifier_for_bundle_identifier(
                self.agent_bundle
            )
            if expected_pid is not None and pid != expected_pid:
                raise ProbeError("AgentPidDriftBeforeTerminate")
            if pid:
                await process_control.kill(pid)
            return pid

        value = _run_async(
            _with_process_control(self.udid, terminate), 30.0, "agent terminate"
        )
        if value is None:
            return None
        if isinstance(value, bool) or not isinstance(value, int) or value <= 0:
            raise ProbeError("ProcessIdentityInvalid")
        return value

    def _wait_process_absent(self, old_pid: int, timeout_seconds: float = 20.0) -> None:
        deadline = time.monotonic() + timeout_seconds
        while time.monotonic() < deadline:
            current = self._process_id(self.agent_bundle)
            if current is None:
                return
            if current != old_pid:
                raise ProbeError("AgentPidReplacedDuringStop")
            time.sleep(0.2)
        raise ProbeError("AgentPidDidNotExit")

    def _prove_process_stable(self, bundle_id: str, expected_pid: int) -> None:
        first = self._process_id(bundle_id)
        time.sleep(0.1)
        second = self._process_id(bundle_id)
        if first != expected_pid or second != expected_pid:
            raise ProbeError("ForegroundPidUnstable")

    def _wait_device_ports(self, expected_open: bool, timeout_seconds: float) -> None:
        deadline = time.monotonic() + timeout_seconds
        while time.monotonic() < deadline:
            states = [
                _run_async(
                    _device_port_is_open(self.udid, port),
                    6.0,
                    f"device port {port}",
                )
                for port in (self.CONTROL_PORT, self.MJPEG_PORT)
            ]
            if all(state is expected_open for state in states):
                return
            time.sleep(0.2)
        raise ProbeError(
            "DevicePortsDidNotOpen" if expected_open else "DevicePortsDidNotClose"
        )

    def _launch(self, bundle_id: str, *, environment: dict[str, str], kill_existing: bool) -> int:
        async def launch(process_control):
            activated = await process_control.launch(
                bundle_id,
                kill_existing=kill_existing,
                environment=environment,
            )
            observed = await process_control.process_identifier_for_bundle_identifier(bundle_id)
            return activated, observed

        activated, observed = _run_async(
            _with_process_control(self.udid, launch), 30.0, f"foreground {bundle_id}"
        )
        if (
            isinstance(activated, bool)
            or not isinstance(activated, int)
            or activated <= 0
            or activated != observed
        ):
            raise ProbeError("ForegroundPidUnstable")
        return activated

    def prepare_fresh_install(self) -> dict[str, Any]:
        self.stop_relays()
        old_pid = self._process_id(self.agent_bundle)
        if old_pid is not None:
            terminated = self._terminate_agent(old_pid)
            if terminated != old_pid:
                raise ProbeError("AgentInstallPidMismatch")
            self._wait_process_absent(old_pid)
        self._wait_device_ports(False, 20.0)
        return {
            "oldProcessTerminated": old_pid is not None,
            "devicePortsClosedBeforeInstall": True,
        }

    def begin_case(self) -> dict[str, Any]:
        self.stop_relays()
        old_pid = self._process_id(self.agent_bundle)
        if old_pid is not None:
            self._terminate_agent(old_pid)
            self._wait_process_absent(old_pid)
        self._wait_device_ports(False, 20.0)
        self.generation += 1
        self.trace = [
            "old-producer-stopped" if old_pid is not None else "old-producer-absent",
            "ports-closed",
            "generation-advanced",
        ]
        self.route_auth = {}
        self._auth_counts = {}
        self._geometry = None
        self.share_sheet_open = False
        self.latest_frame = None
        self.latest_frame_sequence = 0
        environment = {
            "USE_IP": "127.0.0.1",
            "USE_PORT": str(self.CONTROL_PORT),
            "MJPEG_SERVER_PORT": str(self.MJPEG_PORT),
            "FARM_KEY": self.token,
            "WDA_PRODUCT_BUNDLE_IDENTIFIER": self.agent_bundle,
        }
        self.agent_pid = self._launch(
            self.agent_bundle, environment=environment, kill_existing=False
        )
        if self.agent_pid == old_pid or self.agent_pid in self._observed_agent_pids:
            raise ProbeError("AgentColdPidNotDistinct")
        self._prove_process_stable(self.agent_bundle, self.agent_pid)
        self._observed_agent_pids.add(self.agent_pid)
        deadline = time.monotonic() + 45.0
        while time.monotonic() < deadline:
            if _run_async(
                _device_port_is_open(self.udid, self.CONTROL_PORT),
                6.0,
                "control port",
            ):
                break
            time.sleep(0.25)
        else:
            raise ProbeError("AgentControlPortMissing")
        self.control_relay = UsbmuxRelay(self.udid, self.CONTROL_PORT)
        self.control_relay.start()
        locked = self._control_with_auth("GET", "/wda/locked", None, "protectedHealth")
        if not isinstance(locked, bool):
            raise ProbeError("ProtectedHealthInvalid")
        if locked:
            raise ProbeError("DeviceLocked")
        self.trace.append("protected-auth")
        return {
            "generation": self.generation,
            "oldPidWitnessed": old_pid is not None,
            "newPidSha256": hashlib.sha256(str(self.agent_pid).encode("ascii")).hexdigest(),
        }

    def foreground_tiktok(self) -> int:
        prior = self._process_id(self.tiktok_bundle)
        pid = self._launch(self.tiktok_bundle, environment={}, kill_existing=False)
        if prior is not None and pid != prior:
            raise ProbeError("TikTokPidChangedOnForeground")
        self._prove_process_stable(self.tiktok_bundle, pid)
        self.tiktok_pid = pid
        self.trace.append("foreground-tiktok")
        return pid

    def create_session(self) -> str:
        payload = self._control_with_auth(
            "POST",
            "/session",
            {"capabilities": {"alwaysMatch": {}, "firstMatch": [{}]}},
            "createSession",
            return_payload=True,
        )
        session_id = payload.get("sessionId") if isinstance(payload, dict) else None
        value = payload.get("value") if isinstance(payload, dict) else None
        if not session_id and isinstance(value, dict):
            session_id = value.get("sessionId")
        if not isinstance(session_id, str) or not session_id:
            raise ProbeError("SessionIdMissing")
        self.session_id = session_id
        self.trace.append("session")
        return session_id

    def start_stream(self, *, prove_auth: bool) -> StreamBatch:
        if self.session_id is None:
            raise ProbeError("SessionBeforeMjpegViolation")
        self.stop_stream()
        self.mjpeg_relay = UsbmuxRelay(self.udid, self.MJPEG_PORT)
        self.mjpeg_relay.start()
        if prove_auth:
            statuses = prove_stream_auth_triplet(
                self.mjpeg_url, self.TOKEN_HEADER, self.token
            )
            self.route_auth[self._auth_label("mjpeg")] = list(statuses)
        self.trace.append("mjpeg-connect")
        reader = MjpegReader(
            self.mjpeg_url,
            self.TOKEN_HEADER,
            self.token,
            generation=self.generation,
        )
        self.mjpeg_reader = reader
        batch = reader.start(
            minimum_frames=3, timeout_seconds=REQUEST_TIMEOUT_SECONDS
        )
        if batch.generation != self.generation:
            raise ProbeError("MjpegGenerationMismatch")
        self.latest_frame = batch.frames[-1]
        self.latest_frame_sequence = batch.end_sequence
        self.trace.append("first-frame")
        self.trace.append("stream-continuous")
        return batch

    def wait_stream_frames(self, after_sequence: int | None = None) -> StreamBatch:
        reader = self.mjpeg_reader
        if reader is None or reader.generation != self.generation:
            raise ProbeError("MjpegGenerationMismatch")
        if after_sequence is None:
            after_sequence = self.latest_frame_sequence
        batch = reader.wait_for_frames(
            after_sequence,
            minimum_frames=3,
            timeout_seconds=REQUEST_TIMEOUT_SECONDS,
        )
        if batch.generation != self.generation:
            raise ProbeError("MjpegGenerationMismatch")
        self.latest_frame = batch.frames[-1]
        self.latest_frame_sequence = batch.end_sequence
        return batch

    def stream_boundary(self) -> int:
        reader = self.mjpeg_reader
        if reader is None or reader.generation != self.generation:
            raise ProbeError("MjpegGenerationMismatch")
        sequence = reader.sequence
        if sequence < self.latest_frame_sequence:
            raise ProbeError("MjpegSequenceRegressed")
        return sequence

    def stop_stream(self) -> None:
        errors: list[Exception] = []
        if self.mjpeg_reader is not None:
            try:
                self.mjpeg_reader.stop()
                self.mjpeg_reader = None
            except Exception as exc:
                errors.append(exc)
        if self.mjpeg_relay is not None:
            try:
                self.mjpeg_relay.stop()
                self.mjpeg_relay = None
            except Exception as exc:
                errors.append(exc)
        if errors:
            raise ProbeError("StreamStopFailed") from errors[0]

    def active_identity(self, *, prove_auth: bool = False) -> dict[str, Any]:
        value = (
            self._control_with_auth(
                "GET", "/wda/activeAppInfo", None, "activeApp"
            )
            if prove_auth
            else self._control("GET", "/wda/activeAppInfo")
        )
        if not isinstance(value, dict):
            raise ProbeError("ActiveIdentityInvalid")
        return value

    def prove_tiktok_identity(
        self, expected_pid: int, *, prove_auth: bool = False
    ) -> AppIdentityProof:
        before = self.active_identity(prove_auth=prove_auth)
        time.sleep(0.1)
        after = self.active_identity()
        proof = prove_stable_app_identity(before, after, self.tiktok_bundle)
        if proof.pid != expected_pid or self._process_id(self.tiktok_bundle) != expected_pid:
            raise ProbeError("ActivePidMismatch")
        return proof

    def geometry(self, frame: bytes, *, prove_auth: bool = False) -> GeometryEvidence:
        if self.session_id is None:
            raise ProbeError("SessionMissing")
        orientation = (
            self._control_with_auth(
                "GET", "/wda/deviceOrientation", None, "deviceOrientation"
            )
            if prove_auth
            else self._control("GET", "/wda/deviceOrientation")
        )
        if not isinstance(orientation, str):
            raise ProbeError("UnsupportedGeometry")
        metrics = _run_async(
            _device_display_metrics(self.udid), 20.0, "device display metrics"
        )
        evidence = geometry_from_mobilegestalt(
            metrics, orientation, _jpeg_dimensions(frame)
        )
        if (evidence.logical_width, evidence.logical_height) != self.logical_bounds:
            raise ProbeError("UnsupportedBounds")
        validate_geometry(evidence)
        if self._geometry is not None and evidence != self._geometry:
            raise ProbeError("GeometryDrift")
        self._geometry = evidence
        return evidence

    def open_url(self, url: str) -> int:
        body = {
            "url": url,
            "bundleId": self.tiktok_bundle,
            "idleTimeoutMs": 0,
        }
        self._control_with_auth(
            "POST", "/url", body, "openUrl"
        )
        self.trace.append("open-url")
        return self.stream_boundary()

    def get_clipboard(self, *, prove_auth: bool = False) -> bytes:
        body = {"contentType": "plaintext"}
        value = (
            self._control_with_auth(
                "POST", "/wda/getPasteboard", body, "clipboardGet"
            )
            if prove_auth
            else self._control("POST", "/wda/getPasteboard", body)
        )
        return decode_bounded_clipboard(value)

    def set_clipboard(self, value: bytes, *, prove_auth: bool = False) -> None:
        if len(value) > MAX_CLIPBOARD_BYTES:
            raise ProbeError("ClipboardTooLarge")
        body = {
            "content": base64.b64encode(value).decode("ascii"),
            "contentType": "plaintext",
        }
        if prove_auth:
            self._control_with_auth(
                "POST", "/wda/setPasteboard", body, "clipboardSet"
            )
        else:
            self._control("POST", "/wda/setPasteboard", body)

    def foreground_agent_stable(self) -> int:
        before = self._process_id(self.agent_bundle)
        if before is None:
            raise ProbeError("AgentPidMissing")
        after = self._launch(self.agent_bundle, environment={}, kill_existing=False)
        if before != after:
            raise ProbeError("AgentPidChangedOnForeground")
        return after

    def clipboard_transition(
        self,
        sentinel: bytes,
        mode: str,
        *,
        write: bool,
        prove_auth: bool = False,
    ) -> bytes:
        if mode == "targetBackgroundSafe":
            before = self.active_identity()
            if write:
                self.set_clipboard(sentinel, prove_auth=prove_auth)
            value = self.get_clipboard(prove_auth=prove_auth)
            after = self.active_identity()
            prove_stable_app_identity(before, after, self.tiktok_bundle)
            return value
        if mode != "agentForegroundRequired":
            raise ProbeError("ClipboardModeUnsupported")
        self.stop_stream()
        self.generation += 1
        self.trace.append("clipboard-generation-advanced")
        agent_before = self.foreground_agent_stable()
        agent_identity_before = self.active_identity()
        prove_stable_app_identity(
            agent_identity_before, self.active_identity(), self.agent_bundle
        )
        if _identity_fields(agent_identity_before)[1] != agent_before:
            raise ProbeError("AgentPidChangedDuringClipboard")
        if write:
            self.set_clipboard(sentinel, prove_auth=prove_auth)
        value = self.get_clipboard(prove_auth=prove_auth)
        agent_after = self._process_id(self.agent_bundle)
        if agent_after != agent_before:
            raise ProbeError("AgentPidChangedDuringClipboard")
        agent_identity_after = self.active_identity()
        prove_stable_app_identity(
            agent_identity_before, agent_identity_after, self.agent_bundle
        )
        tiktok_pid = self.foreground_tiktok()
        self.prove_tiktok_identity(tiktok_pid)
        self.create_session()
        self.start_stream(prove_auth=True)
        self.prove_tiktok_identity(tiktok_pid)
        return value

    def _native_tap(
        self,
        x_fraction: float,
        y_fraction: float,
        geometry: GeometryEvidence,
        auth_prefix: str,
    ) -> int:
        if not all(
            math.isfinite(value) and 0 <= value < 1
            for value in (x_fraction, y_fraction)
        ):
            raise ProbeError("TapCoordinateInvalid")
        from_x = x_fraction * geometry.logical_width
        from_y = y_fraction * geometry.logical_height
        to_x = min(float(geometry.logical_width), from_x + 1.0)
        to_y = min(float(geometry.logical_height), from_y + 1.0)
        self._control_with_auth(
            "POST",
            "/wda/swipe",
            {
                "fromX": from_x,
                "fromY": from_y,
                "toX": to_x,
                "toY": to_y,
                "delay": 0.05,
            },
            auth_prefix,
        )
        return self.stream_boundary()

    def _native_tap_correct(
        self,
        x_fraction: float,
        y_fraction: float,
        geometry: GeometryEvidence,
    ) -> int:
        if not all(
            math.isfinite(value) and 0 <= value < 1
            for value in (x_fraction, y_fraction)
        ):
            raise ProbeError("TapCoordinateInvalid")
        from_x = x_fraction * geometry.logical_width
        from_y = y_fraction * geometry.logical_height
        self._control(
            "POST",
            "/wda/swipe",
            {
                "fromX": from_x,
                "fromY": from_y,
                "toX": from_x + 1.0,
                "toY": from_y + 1.0,
                "delay": 0.05,
            },
        )
        return self.stream_boundary()

    def dismiss_share_sheet_if_needed(self) -> None:
        if not self.share_sheet_open:
            return
        if self._geometry is None:
            raise ProbeError("ShareSheetCleanupEvidenceMissing")
        observation_boundary = self.stream_boundary()
        observation = self.wait_stream_frames(observation_boundary)
        try:
            find_share_control(observation.frames[-1])
            self.share_sheet_open = False
            return
        except ProbeError:
            pass
        boundary = self._native_tap_correct(0.05, 0.10, self._geometry)
        batch = self.wait_stream_frames(boundary)
        find_share_control(batch.frames[-1])
        self.share_sheet_open = False
        self.trace.append("share-sheet-dismissed")

    def tap_share_and_copy_link_once(self) -> dict[str, Any]:
        if self.session_id is None or self.latest_frame is None:
            raise ProbeError("FreshTargetFrameMissing")
        geometry = self.geometry(self.latest_frame)
        share_x, share_y = find_share_control(self.latest_frame)
        self.share_sheet_open = True
        after_share = self._native_tap(
            share_x, share_y, geometry, "nativeSwipeShare"
        )
        self.trace.append("share-tap")
        share_batch = self.wait_stream_frames(after_share)
        share_frame = share_batch.frames[-1]
        copy_geometry = self.geometry(share_frame)
        copy_x, copy_y, ocr_digest = run_vision_copy_link(share_frame)
        self.trace.append("identity-copy-intent-issued")
        after_copy = self._native_tap(
            copy_x, copy_y, copy_geometry, "nativeSwipeCopyLink"
        )
        self.trace.append("copy-link-tap")
        post_copy = self.wait_stream_frames(after_copy)
        find_share_control(post_copy.frames[-1])
        self.share_sheet_open = False
        return {
            "sharePoint": [round(share_x, 6), round(share_y, 6)],
            "copyPoint": [round(copy_x, 6), round(copy_y, 6)],
            "ocrObservationSha256": ocr_digest,
            "shareSheetFrameSha256": hashlib.sha256(share_frame).hexdigest(),
            "postCopyFrameSha256": hashlib.sha256(post_copy.frames[-1]).hexdigest(),
        }

    def _status_session_id(self) -> str:
        status, raw = _request(
            "GET",
            self.control_base + "/status",
            self.TOKEN_HEADER,
            None,
            None,
            REQUEST_TIMEOUT_SECONDS,
        )
        if status != 200:
            raise ProbeError("SessionStatusWitnessFailed")
        payload = _strict_json(raw, "GET /status")
        candidates: list[Any] = []
        if isinstance(payload, dict):
            candidates.append(payload.get("sessionId"))
            value = payload.get("value")
            if isinstance(value, dict):
                candidates.append(value.get("sessionId"))
        session_id = next(
            (
                candidate
                for candidate in candidates
                if isinstance(candidate, str) and candidate
            ),
            None,
        )
        if session_id is None:
            raise ProbeError("SessionStatusWitnessMissing")
        return session_id

    def require_healthy(self, expected_tiktok_pid: int) -> StreamBatch:
        if self.session_id is None:
            raise ProbeError("SessionMissing")
        if self._status_session_id() != self.session_id:
            raise ProbeError("SessionStatusWitnessMismatch")
        self.prove_tiktok_identity(expected_tiktok_pid)
        if self.latest_frame is None:
            raise ProbeError("FreshTargetFrameMissing")
        self.geometry(self.latest_frame)
        health_boundary = self.stream_boundary()
        batch = self.wait_stream_frames(health_boundary)
        self.geometry(batch.frames[-1])
        self.trace.append("final-health")
        return batch

    def stop_relays(self) -> None:
        errors: list[Exception] = []
        try:
            self.stop_stream()
        except Exception as exc:
            errors.append(exc)
        for name in ("control_relay",):
            relay = getattr(self, name)
            if relay is not None:
                try:
                    relay.stop()
                    setattr(self, name, None)
                except Exception as exc:
                    errors.append(exc)
        if errors:
            raise ProbeError("RelayCleanupFailed") from errors[0]

    def cleanup_case(self) -> None:
        local_ports = [
            relay.local_port
            for relay in (self.mjpeg_relay, self.control_relay)
            if relay is not None
        ]
        expected_pid = self.agent_pid

        def invalidate_generation() -> None:
            self.generation += 1
            self.trace.append("cleanup-generation-invalidated")

        def terminate_agent() -> None:
            terminated = self._terminate_agent(expected_pid)
            if expected_pid is not None and terminated != expected_pid:
                raise ProbeError("AgentCleanupPidMismatch")
            if expected_pid is not None:
                self._wait_process_absent(expected_pid)

        def verify_local_ports() -> None:
            deadline = time.monotonic() + 5.0
            remaining = list(local_ports)
            while remaining and time.monotonic() < deadline:
                remaining = [port for port in remaining if _port_open("127.0.0.1", port)]
                if remaining:
                    time.sleep(0.05)
            if remaining:
                raise ProbeError("LocalRelayPortsDidNotClose")

        def clear_state() -> None:
            self.session_id = None
            self.agent_pid = None
            self.tiktok_pid = None
            self.latest_frame = None
            self.latest_frame_sequence = 0
            self._geometry = None
            self.share_sheet_open = False

        run_cleanup_steps(
            (
                self.dismiss_share_sheet_if_needed,
                self.stop_relays,
                invalidate_generation,
                terminate_agent,
                lambda: self._wait_device_ports(False, 20.0),
                verify_local_ports,
                clear_state,
            )
        )
        self.trace.append("cleanup-ports-closed")


def _required_text(mapping: dict[str, Any], key: str, label: str) -> str:
    value = mapping.get(key)
    if not isinstance(value, str) or not value.strip():
        raise ProbeError(f"QualificationFieldMissing: {label}")
    return value


def _require_complete_auth_matrix(matrix: dict[str, list[int]]) -> None:
    required_prefixes = {
        "protectedHealth",
        "createSession",
        "mjpeg",
        "openUrl",
        "activeApp",
        "deviceOrientation",
        "clipboardSet",
        "clipboardGet",
        "nativeSwipeShare",
        "nativeSwipeCopyLink",
    }
    present = {
        re.sub(r"\d+$", "", label)
        for label, statuses in matrix.items()
        if statuses == [401, 401, 200]
    }
    if not required_prefixes.issubset(present):
        raise ProbeError("ProtectedRouteEvidenceIncomplete")
    if any(statuses != [401, 401, 200] for statuses in matrix.values()):
        raise ProbeError("ProtectedRouteEvidenceInvalid")


def _require_complete_tuple(value: Any, path: str = "tuple") -> None:
    if isinstance(value, dict):
        if not value:
            raise ProbeError(f"QualificationTupleIncomplete: {path}")
        for key, child in value.items():
            _require_complete_tuple(child, f"{path}.{key}")
    elif isinstance(value, list):
        if not value:
            raise ProbeError(f"QualificationTupleIncomplete: {path}")
        for index, child in enumerate(value):
            _require_complete_tuple(child, f"{path}[{index}]")
    elif value is None or (isinstance(value, str) and not value.strip()):
        raise ProbeError(f"QualificationTupleIncomplete: {path}")


def _live_mac_report(
    args: argparse.Namespace, artifact: ArtifactProof, token: str
) -> dict[str, Any]:
    agent_bundle = _required_text(artifact.manifest, "bundleId", "artifact.bundleId")
    artifact_version = _required_text(
        artifact.manifest, "artifactVersion", "artifact.artifactVersion"
    )
    bundle_version = _required_text(
        artifact.manifest, "bundleVersion", "artifact.bundleVersion"
    )
    bundle_build = _required_text(
        artifact.manifest, "bundleBuild", "artifact.bundleBuild"
    )
    signer_identity = _required_text(
        artifact.manifest, "signerIdentity", "artifact.signerIdentity"
    )
    protocol_version = artifact.manifest.get("protocolVersion")
    if (
        isinstance(protocol_version, bool)
        or not isinstance(protocol_version, int)
        or protocol_version < 1
    ):
        raise ProbeError("ArtifactManifestInvalid: protocolVersion")
    ipa_identity = inspect_ipa_identity(args.ipa, artifact.manifest)
    manifest_contract = {
        "controlPort": MacGateAdapter.CONTROL_PORT,
        "mjpegPort": MacGateAdapter.MJPEG_PORT,
        "logicalWidth": EXPECTED_LOGICAL_BOUNDS[0],
        "logicalHeight": EXPECTED_LOGICAL_BOUNDS[1],
    }
    if any(artifact.manifest.get(key) != value for key, value in manifest_contract.items()):
        raise ProbeError("ArtifactManifestRuntimeContractMismatch")
    detector_before = detector_attestation()
    host_attestation = host_vision_attestation()
    route_contracts = interaction_route_contracts(args.tiktok_bundle)
    planned = {
        "direct": _resolve_tiktok_post(args.direct_url),
        "photo": _resolve_tiktok_post(args.photo_url),
        "short": _resolve_tiktok_post(args.short_url),
    }
    if planned["direct"][0].post_kind != "video" or planned["photo"][0].post_kind != "photo":
        raise ProbeError("TargetFixtureKindMismatch")

    adapter = MacGateAdapter(
        args.udid,
        token,
        agent_bundle,
        args.tiktok_bundle,
        EXPECTED_LOGICAL_BOUNDS,
    )
    fresh_install = {
        **adapter.prepare_fresh_install(),
        **_fresh_install_hashed_agent(
            args.udid, args.ipa, artifact, ipa_identity
        ),
    }
    metadata = _inspect_device(args, agent_bundle)
    agent_app = metadata.get("agentApp")
    target_app = metadata.get("targetApp")
    if not isinstance(agent_app, dict):
        raise ProbeError("AgentNotInstalled")
    if not isinstance(target_app, dict):
        raise ProbeError("TikTokNotInstalled")
    agent_executable = _required_text(
        agent_app, "executableName", "installedAgent.executableName"
    )
    if agent_executable != ipa_identity.executable_name:
        raise ProbeError("InstalledAgentExecutableMismatch")
    product_type = _required_text(metadata, "productType", "device.productType")
    ios_version = _required_text(metadata, "iosVersion", "device.iosVersion")
    target_version = _required_text(target_app, "version", "targetApp.version")
    target_build = _required_text(target_app, "build", "targetApp.build")
    installed_agent_contract = {
        "bundleId": agent_bundle,
        "version": bundle_version,
        "build": bundle_build,
        "signerIdentity": signer_identity,
    }
    if any(agent_app.get(key) != value for key, value in installed_agent_contract.items()):
        raise ProbeError("InstalledAgentIdentityMismatch")
    if target_app.get("bundleId") != args.tiktok_bundle:
        raise ProbeError("InstalledTikTokIdentityMismatch")
    if metadata.get("transport") != "legacyUsbmuxTransport":
        raise ProbeError("TransportUnsupported")
    signer_identity_sha256 = hashlib.sha256(signer_identity.encode("utf-8")).hexdigest()
    cases: list[dict[str, Any]] = []
    selected_clipboard_mode: str | None = None
    qualified_geometry: GeometryEvidence | None = None
    resolved_urls_for_redaction = [value[1] for value in planned.values()]
    overall_started = time.monotonic()
    failure_code: str | None = None
    cleanup_state = "PASS"
    for label, target_url in (
        ("direct", args.direct_url),
        ("photo", args.photo_url),
        ("short", args.short_url),
    ):
        case_started = time.monotonic()
        prior_clipboard: bytes | None = None
        case_mode: str | None = None
        restore_mode: str | None = None
        clipboard_write_attempted = False
        evidence: dict[str, Any] = {"label": label, "outcome": "FAIL"}
        try:
            lifecycle = adapter.begin_case()
            expected_identity = planned[label][0]
            expected_url = planned[label][1]
            tiktok_pid = adapter.foreground_tiktok()
            adapter.create_session()
            opening_batch = adapter.start_stream(prove_auth=True)
            opening_frame = opening_batch.frames[-1]
            if adapter.trace.index("session") > adapter.trace.index("mjpeg-connect"):
                raise ProbeError("SessionBeforeMjpegViolation")
            open_boundary = adapter.open_url(target_url)

            target_batch = adapter.wait_stream_frames(open_boundary)
            fresh_frame = target_batch.frames[-1]
            opening_digest = hashlib.sha256(opening_frame).digest()
            if all(
                hashlib.sha256(frame).digest() == opening_digest
                for frame in target_batch.frames
            ):
                raise ProbeError("FreshTargetFrameMissing")

            active_proof = adapter.prove_tiktok_identity(tiktok_pid, prove_auth=True)
            geometry = adapter.geometry(fresh_frame, prove_auth=True)
            scales = validate_geometry(geometry)
            if qualified_geometry is None:
                qualified_geometry = geometry
            elif geometry != qualified_geometry:
                raise ProbeError("GeometryDrift")

            sentinel = (
                f"riviu-gate0:{label}:{hashlib.sha256(os.urandom(32)).hexdigest()}"
            ).encode("ascii")
            try:
                prior_clipboard = adapter.clipboard_transition(
                    b"", "targetBackgroundSafe", write=False
                )
                require_controlled_clipboard_fixture(prior_clipboard)
                restore_mode = "targetBackgroundSafe"
                clipboard_write_attempted = True
                sentinel_read = adapter.clipboard_transition(
                    sentinel,
                    "targetBackgroundSafe",
                    write=True,
                    prove_auth=True,
                )
                if sentinel_read != sentinel:
                    raise ProbeError("ClipboardSentinelMismatch")
                case_mode = "targetBackgroundSafe"
            except ProbeError as background_error:
                background_message = str(background_error)
                background_code = background_message.split(":", 1)[0]
                mode_specific = background_code == "ClipboardSentinelMismatch" or (
                    background_code
                    in {"ControlRouteFailed", "ProtectedRouteUnavailableInForeground"}
                    and "Pasteboard" in background_message
                )
                if not mode_specific:
                    raise
                restore_mode = "agentForegroundRequired"
                if prior_clipboard is None:
                    prior_clipboard = adapter.clipboard_transition(
                        b"", "agentForegroundRequired", write=False
                    )
                    require_controlled_clipboard_fixture(prior_clipboard)
                clipboard_write_attempted = True
                sentinel_read = adapter.clipboard_transition(
                    sentinel,
                    "agentForegroundRequired",
                    write=True,
                    prove_auth=True,
                )
                case_mode = "agentForegroundRequired"
            if selected_clipboard_mode is None:
                selected_clipboard_mode = case_mode
            elif selected_clipboard_mode != case_mode:
                raise ProbeError("ClipboardModeDrift")
            if sentinel_read != sentinel:
                raise ProbeError("ClipboardSentinelMismatch")
            evidence["clipboardFixture"] = "controlled-plaintext-v1"
            if adapter.latest_frame is None:
                raise ProbeError("FreshTargetFrameMissing")
            validate_geometry(adapter.geometry(adapter.latest_frame))
            identity_evidence = adapter.tap_share_and_copy_link_once()
            copied = adapter.clipboard_transition(b"", case_mode, write=False)
            if copied == sentinel:
                raise ProbeError("TargetIdentitySentinelUnchanged")
            try:
                copied_text = copied.decode("utf-8")
            except UnicodeDecodeError as exc:
                raise ProbeError("TargetIdentityClipboardNotUtf8") from exc
            copied_identity, copied_url = _resolve_tiktok_post(copied_text)
            resolved_urls_for_redaction.append(copied_url)
            if copied_identity != expected_identity:
                raise ProbeError("TargetUnverified")
            if adapter.tiktok_pid is None:
                raise ProbeError("ActivePidMissing")
            final_batch = adapter.require_healthy(adapter.tiktok_pid)
            _require_complete_auth_matrix(adapter.route_auth)
            evidence.update(
                {
                    "outcome": "PASS",
                    "durationMs": round((time.monotonic() - case_started) * 1000),
                    "lifecycle": lifecycle,
                    "routeAuth": dict(sorted(adapter.route_auth.items())),
                    "clipboardMode": case_mode,
                    "geometry": {
                        "logicalWidth": geometry.logical_width,
                        "logicalHeight": geometry.logical_height,
                        "pixelWidth": geometry.frame_width,
                        "pixelHeight": geometry.frame_height,
                        "scaleX": scales[0],
                        "scaleY": scales[1],
                        "orientation": geometry.orientation,
                    },
                    "scale": [scales[0], scales[1]],
                    "activePidSha256": hashlib.sha256(
                        str(active_proof.pid).encode("ascii")
                    ).hexdigest(),
                    "plannedIdentitySha256": hashlib.sha256(
                        f"{expected_identity.post_kind}:{expected_identity.content_id}".encode()
                    ).hexdigest(),
                    "copiedIdentitySha256": hashlib.sha256(
                        f"{copied_identity.post_kind}:{copied_identity.content_id}".encode()
                    ).hexdigest(),
                    "resolvedInputUrlSha256": hashlib.sha256(expected_url.encode()).hexdigest(),
                    "copiedUrlSha256": hashlib.sha256(copied_url.encode()).hexdigest(),
                    "openingFrameSha256": hashlib.sha256(opening_frame).hexdigest(),
                    "targetFrameSha256": hashlib.sha256(fresh_frame).hexdigest(),
                    "stream": {
                        "generation": opening_batch.generation,
                        "openingSequence": [
                            opening_batch.start_sequence,
                            opening_batch.end_sequence,
                        ],
                        "targetSequence": [
                            target_batch.start_sequence,
                            target_batch.end_sequence,
                        ],
                        "finalSequence": [
                            final_batch.start_sequence,
                            final_batch.end_sequence,
                        ],
                        "maximumGapMs": round(
                            max(
                                opening_batch.maximum_gap_seconds,
                                target_batch.maximum_gap_seconds,
                                final_batch.maximum_gap_seconds,
                            )
                            * 1000
                        ),
                    },
                    "identity": identity_evidence,
                    "trace": list(adapter.trace),
                }
            )
        except Exception as exc:
            message = str(exc)
            code = message.split(":", 1)[0]
            failure_code = code if re.fullmatch(r"[A-Za-z][A-Za-z0-9]+", code) else "LiveProbeFailed"
            evidence.update(
                {
                    "outcome": "FAIL",
                    "durationMs": round((time.monotonic() - case_started) * 1000),
                    "code": failure_code,
                    "trace": list(adapter.trace),
                }
            )
        finally:
            cleanup_failed = False
            if clipboard_write_attempted:
                if prior_clipboard is None or restore_mode is None:
                    cleanup_failed = True
                    evidence["plaintextClipboardRestored"] = False
                else:
                    try:
                        adapter.clipboard_transition(
                            prior_clipboard, restore_mode, write=True
                        )
                        restored = adapter.clipboard_transition(
                            b"", restore_mode, write=False
                        )
                        evidence["plaintextClipboardRestored"] = restored == prior_clipboard
                        if restored != prior_clipboard:
                            cleanup_failed = True
                    except Exception:
                        cleanup_failed = True
                        evidence["plaintextClipboardRestored"] = False
            try:
                adapter.cleanup_case()
            except Exception:
                cleanup_failed = True
            evidence["routeAuth"] = dict(sorted(adapter.route_auth.items()))
            evidence["trace"] = list(adapter.trace)
            if cleanup_failed:
                cleanup_state = "FAIL"
                failure_code = failure_code or "CleanupFailed"
                evidence["outcome"] = "FAIL"
                evidence["code"] = evidence.get("code", "CleanupFailed")
            cases.append(evidence)
        if failure_code is not None:
            break

    detector_after = detector_attestation()
    if failure_code is None and detector_after != detector_before:
        failure_code = "DetectorSourceChangedDuringProbe"
    if failure_code is None and (
        len(cases) != 3
        or any(case.get("outcome") != "PASS" for case in cases)
        or selected_clipboard_mode is None
        or qualified_geometry is None
    ):
        failure_code = "QualificationEvidenceIncomplete"

    if qualified_geometry is not None and selected_clipboard_mode is not None:
        scale_x, scale_y = validate_geometry(qualified_geometry)
        open_url_contract = dict(route_contracts["openUrl"])
        clipboard_set_contract = dict(route_contracts["clipboardSet"])
        clipboard_get_contract = dict(route_contracts["clipboardGet"])
        qualification_tuple: dict[str, Any] = {
            "deviceSha256": _redacted_device_hash(args.udid),
            "driverAdapterVersion": "interaction-v1",
            "base": {
                "agentArtifactSha256": artifact.ipa_sha256,
                "agentBundleId": agent_bundle,
                "agentBundleVersion": bundle_version,
                "agentBundleBuild": bundle_build,
                "agentExecutableName": agent_executable,
                "agentSignerIdentitySha256": signer_identity_sha256,
                "agentVersion": artifact_version,
                "protocolVersion": protocol_version,
                "transport": metadata["transport"],
                "productType": product_type,
                "iosMinInclusive": ios_version,
                "iosMaxInclusive": ios_version,
                "tiktokBundleId": args.tiktok_bundle,
                "tiktokVersion": target_version,
                "tiktokBuild": target_build,
                "geometry": {
                    "logicalWidth": qualified_geometry.logical_width,
                    "logicalHeight": qualified_geometry.logical_height,
                    "pixelWidth": qualified_geometry.frame_width,
                    "pixelHeight": qualified_geometry.frame_height,
                    "scaleX": scale_x,
                    "scaleY": scale_y,
                    "orientation": qualified_geometry.orientation,
                },
            },
            "openUrl": open_url_contract,
            "clipboard": {
                "contractId": "rtmmo-clipboard-v1",
                "mode": selected_clipboard_mode,
                "setRoute": clipboard_set_contract,
                "getRoute": clipboard_get_contract,
                "maximumDecodedBytes": MAX_CLIPBOARD_BYTES,
            },
            "targetIdentityCopyLink": {
                "openUrlContractId": open_url_contract["contractId"],
                "clipboardContractId": "rtmmo-clipboard-v1",
                "shareDetectorVersion": detector_before["shareDetectorVersion"],
                "copyLinkDetectorVersion": detector_before["copyLinkDetectorVersion"],
                "detectorSetSha256": detector_before["detectorSetSha256"],
                "layoutId": detector_before["layoutId"],
            },
            "detectorAttestation": detector_before,
            "referenceProbeHost": host_attestation,
            "routeContracts": route_contracts,
            "routeContractSha256": route_contract_sha256(route_contracts),
        }
    else:
        qualification_tuple = {
            "deviceSha256": _redacted_device_hash(args.udid),
            "qualificationState": "incomplete",
        }

    if failure_code is None:
        _require_complete_tuple(qualification_tuple)
    gate_status = "PASS" if failure_code is None and cleanup_state == "PASS" else "FAIL"
    report = {
        "schemaVersion": 1,
        "environment": LIVE_ENVIRONMENT,
        "gateStatus": gate_status,
        "productionQualified": False,
        "qualifications": [],
        "artifact": {
            "ipaSha256": artifact.ipa_sha256,
            "manifestSha256": artifact.manifest_sha256,
            "artifactVersion": artifact_version,
            "payloadApp": ipa_identity.payload_app,
            "executableName": ipa_identity.executable_name,
            "installProof": fresh_install,
        },
        "tuple": qualification_tuple,
        "timing": {"totalMs": round((time.monotonic() - overall_started) * 1000)},
        "cases": cases,
        "outcomes": {
            "code": failure_code or "PASS",
            "cleanup": cleanup_state,
            "registryModified": False,
        },
    }
    verify_redaction(
        [json.dumps(report, ensure_ascii=True).encode("ascii")],
        [
            token,
            args.udid,
            args.direct_url,
            args.photo_url,
            args.short_url,
            *resolved_urls_for_redaction,
        ],
    )
    return report


def _markdown(report: dict[str, Any]) -> str:
    return (
        "# TikTok Interaction Gate 0\n\n"
        f"- Environment: `{report['environment']}`\n"
        f"- Gate status: `{report['gateStatus']}`\n"
        f"- Production qualified: `{str(report['productionQualified']).lower()}`\n"
        f"- Outcome: `{report['outcomes']['code']}`\n"
        "- Capability registry modified: `false`\n"
    )


def main(argv: Sequence[str] | None = None) -> int:
    args = build_parser().parse_args(argv)
    args.report_dir.mkdir(parents=True, exist_ok=True)
    recover_report_publication(args.report_dir)
    token = os.environ.get(args.token_env)
    if not token:
        raise ProbeError(f"TokenEnvironmentMissing: {args.token_env}")
    artifact = verify_artifact_manifest(args.ipa, args.agent_manifest)
    if platform.system() != "Darwin":
        report = _pending_report(args, artifact)
        exit_code = 2
    else:
        report = _live_mac_report(args, artifact, token)
        exit_code = 0 if report.get("gateStatus") == "PASS" else 2
    publish_reports(
        report,
        _markdown(report),
        args.report_dir,
        [
            token,
            args.udid,
            args.direct_url,
            args.photo_url,
            args.short_url,
        ],
    )
    print(
        json.dumps(
            {
                "gateStatus": report["gateStatus"],
                "reports": ["gate-0.json", "gate-0.md"],
                "productionQualified": False,
            },
            separators=(",", ":"),
        )
    )
    return exit_code


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except ProbeError as error:
        print(json.dumps({"gateStatus": "FAIL", "error": str(error)}), file=sys.stderr)
        raise SystemExit(2)
