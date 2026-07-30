#!/usr/bin/env python3
"""Riviu pymobiledevice3 sidecar.

Requires: pip install pymobiledevice3
Falls back to structured errors when the library is missing.
"""

from __future__ import annotations

import argparse
import asyncio
import contextlib
import importlib.metadata
import io
import json
import os
import struct
import subprocess
import sys
import time
from pathlib import Path
from typing import Optional


SIDECAR_PROTOCOL_VERSION = 2
PYMOBILEDEVICE3_PROCESS_CONTROL_VERSION = "10.1.0"
VERIFIED_PROCESS_CONTROL_CONTRACT = "verifiedProcessControl"


def _windows_kill_on_close_job(process: subprocess.Popen):
    """Keep a relay child tied to this proxy even if the proxy is force-killed."""
    if sys.platform != "win32":
        return None

    import ctypes
    from ctypes import wintypes

    class JobObjectBasicLimitInformation(ctypes.Structure):
        _fields_ = [
            ("PerProcessUserTimeLimit", ctypes.c_longlong),
            ("PerJobUserTimeLimit", ctypes.c_longlong),
            ("LimitFlags", wintypes.DWORD),
            ("MinimumWorkingSetSize", ctypes.c_size_t),
            ("MaximumWorkingSetSize", ctypes.c_size_t),
            ("ActiveProcessLimit", wintypes.DWORD),
            ("Affinity", ctypes.c_size_t),
            ("PriorityClass", wintypes.DWORD),
            ("SchedulingClass", wintypes.DWORD),
        ]

    class IoCounters(ctypes.Structure):
        _fields_ = [
            ("ReadOperationCount", ctypes.c_ulonglong),
            ("WriteOperationCount", ctypes.c_ulonglong),
            ("OtherOperationCount", ctypes.c_ulonglong),
            ("ReadTransferCount", ctypes.c_ulonglong),
            ("WriteTransferCount", ctypes.c_ulonglong),
            ("OtherTransferCount", ctypes.c_ulonglong),
        ]

    class JobObjectExtendedLimitInformation(ctypes.Structure):
        _fields_ = [
            ("BasicLimitInformation", JobObjectBasicLimitInformation),
            ("IoInfo", IoCounters),
            ("ProcessMemoryLimit", ctypes.c_size_t),
            ("JobMemoryLimit", ctypes.c_size_t),
            ("PeakProcessMemoryUsed", ctypes.c_size_t),
            ("PeakJobMemoryUsed", ctypes.c_size_t),
        ]

    kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
    kernel32.CreateJobObjectW.argtypes = [ctypes.c_void_p, wintypes.LPCWSTR]
    kernel32.CreateJobObjectW.restype = wintypes.HANDLE
    kernel32.SetInformationJobObject.argtypes = [
        wintypes.HANDLE,
        ctypes.c_int,
        ctypes.c_void_p,
        wintypes.DWORD,
    ]
    kernel32.SetInformationJobObject.restype = wintypes.BOOL
    kernel32.AssignProcessToJobObject.argtypes = [wintypes.HANDLE, wintypes.HANDLE]
    kernel32.AssignProcessToJobObject.restype = wintypes.BOOL
    kernel32.CloseHandle.argtypes = [wintypes.HANDLE]
    kernel32.CloseHandle.restype = wintypes.BOOL

    job = kernel32.CreateJobObjectW(None, None)
    if not job:
        return None

    info = JobObjectExtendedLimitInformation()
    info.BasicLimitInformation.LimitFlags = 0x00002000
    configured = kernel32.SetInformationJobObject(
        job,
        9,
        ctypes.byref(info),
        ctypes.sizeof(info),
    )
    child_handle = getattr(process, "_handle", None)
    assigned = child_handle is not None and kernel32.AssignProcessToJobObject(
        job, wintypes.HANDLE(child_handle)
    )
    if not configured or not assigned:
        kernel32.CloseHandle(job)
        return None
    return job


def _windows_close_handle(handle) -> None:
    if sys.platform != "win32" or handle is None:
        return

    import ctypes
    from ctypes import wintypes

    kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
    kernel32.CloseHandle.argtypes = [wintypes.HANDLE]
    kernel32.CloseHandle.restype = wintypes.BOOL
    kernel32.CloseHandle(handle)


def emit(obj) -> None:
    print(json.dumps(obj), flush=True)


def emit_frame(jpeg: bytes) -> None:
    sys.stdout.buffer.write(struct.pack(">I", len(jpeg)))
    sys.stdout.buffer.write(jpeg)
    sys.stdout.buffer.flush()


def try_import():
    try:
        from pymobiledevice3 import usbmux  # noqa: F401
        return True
    except Exception as exc:  # pragma: no cover
        return False


def verified_process_control_ready() -> bool:
    try:
        if (
            importlib.metadata.version("pymobiledevice3")
            != PYMOBILEDEVICE3_PROCESS_CONTROL_VERSION
        ):
            return False
        from pymobiledevice3.services.dvt.instruments.dvt_provider import DvtProvider
        from pymobiledevice3.services.dvt.instruments.process_control import (
            ProcessControl,
        )

        return isinstance(DvtProvider, type) and isinstance(ProcessControl, type)
    except Exception:  # pragma: no cover - dependency shape varies by host
        return False


def cmd_ping(_: argparse.Namespace) -> int:
    ok = try_import()
    process_control_ready = ok and verified_process_control_ready()
    emit(
        {
            "ok": True,
            "pymobiledevice3": ok,
            "sidecarProtocolVersion": SIDECAR_PROTOCOL_VERSION,
            "contracts": (
                [VERIFIED_PROCESS_CONTROL_CONTRACT] if process_control_ready else []
            ),
        }
    )
    return 0 if ok else 2


async def _list_devices() -> list[dict]:
    from pymobiledevice3.usbmux import list_devices
    from pymobiledevice3.lockdown import create_using_usbmux

    devices = []
    for dev in await list_devices():
        udid = getattr(dev, "serial", None) or getattr(dev, "udid", "")
        info = {
            "udid": udid,
            "name": "iPhone",
            "model": "Unknown",
            "iosVersion": "?",
            "connection": "wifi"
            if str(getattr(dev, "connection_type", "")).lower() == "network"
            else "usb",
            "battery": None,
        }
        lockdown = None
        try:
            lockdown = await create_using_usbmux(serial=udid)
            info["name"] = lockdown.display_name or info["name"]
            info["model"] = lockdown.product_type or info["model"]
            info["iosVersion"] = lockdown.product_version or info["iosVersion"]
        except Exception as exc:
            info["pairingError"] = str(exc)
        finally:
            if lockdown is not None:
                await lockdown.close()
        devices.append(info)
    return devices


def cmd_list(_: argparse.Namespace) -> int:
    if not try_import():
        emit({"devices": [], "error": "pymobiledevice3 not installed"})
        return 0
    try:
        devices = asyncio.run(_list_devices())
        emit({"devices": devices})
        return 0
    except Exception as exc:
        emit({"devices": [], "error": str(exc)})
        return 1


def cmd_install(args: argparse.Namespace) -> int:
    if not try_import():
        print("pymobiledevice3 not installed", file=sys.stderr)
        return 1
    from pymobiledevice3.services.installation_proxy import InstallationProxyService
    from pymobiledevice3.lockdown import create_using_usbmux

    async def _run() -> None:
        lockdown = await create_using_usbmux(serial=args.udid)
        try:
            async with InstallationProxyService(lockdown=lockdown) as proxy:
                await proxy.install_from_local(args.ipa, developer=True)
        finally:
            await lockdown.close()

    asyncio.run(_run())
    emit({"ok": True})
    return 0


def cmd_is_installed(args: argparse.Namespace) -> int:
    if not try_import():
        print("pymobiledevice3 not installed", file=sys.stderr)
        return 1
    from pymobiledevice3.services.installation_proxy import InstallationProxyService
    from pymobiledevice3.lockdown import create_using_usbmux

    async def _run() -> Optional[dict]:
        lockdown = await create_using_usbmux(serial=args.udid)
        try:
            async with InstallationProxyService(lockdown=lockdown) as proxy:
                apps = await proxy.get_apps(bundle_identifiers=[args.bundle_id])
                return apps.get(args.bundle_id)
        finally:
            await lockdown.close()

    app = asyncio.run(_run())
    emit(
        {
            "ok": True,
            "installed": app is not None,
            "bundleId": args.bundle_id,
            "version": app.get("CFBundleShortVersionString") if app else None,
            "build": app.get("CFBundleVersion") if app else None,
            "applicationType": app.get("ApplicationType") if app else None,
            "path": app.get("Path") if app else None,
            "signerIdentity": app.get("SignerIdentity") if app else None,
        }
    )
    return 0


def _interaction_app_identity(bundle_id: str, app: Optional[dict], *, agent: bool):
    if app is None:
        return None
    identity = {
        "bundleId": bundle_id,
        "version": app.get("CFBundleShortVersionString"),
        "build": app.get("CFBundleVersion"),
    }
    if agent:
        identity.update(
            {
                "executableName": app.get("CFBundleExecutable"),
                "signerIdentity": app.get("SignerIdentity"),
            }
        )
    return identity


def cmd_inspect_device_capabilities(args: argparse.Namespace) -> int:
    """Read device and installed-app identity without starting runtime services."""
    if not try_import():
        print("pymobiledevice3 not installed", file=sys.stderr)
        return 1
    from pymobiledevice3.services.installation_proxy import InstallationProxyService

    async def _run() -> dict:
        has_rsd_host = args.rsd_host is not None
        has_rsd_port = args.rsd_port is not None
        if has_rsd_host != has_rsd_port:
            raise ValueError("rsd-host and rsd-port must be provided together")

        provider = None
        try:
            if has_rsd_host:
                from pymobiledevice3.remote.remote_service_discovery import (
                    RemoteServiceDiscoveryService,
                )

                provider = RemoteServiceDiscoveryService((args.rsd_host, args.rsd_port))
                await provider.connect()
                transport = "rsdTransport"
            else:
                from pymobiledevice3.lockdown import create_using_usbmux

                provider = await create_using_usbmux(serial=args.udid, autopair=False)
                transport = "legacyUsbmuxTransport"

            provider_udid = getattr(provider, "udid", None)
            if not provider_udid:
                raise RuntimeError("metadata provider did not report a UDID")
            bundle_ids = [args.target_bundle_id, args.agent_bundle_id]
            async with InstallationProxyService(lockdown=provider) as proxy:
                apps = await proxy.get_apps(bundle_identifiers=bundle_ids)
            return {
                "ok": True,
                "udid": provider_udid,
                "productType": getattr(provider, "product_type", None),
                "iosVersion": getattr(provider, "product_version", None),
                "transport": transport,
                "targetApp": _interaction_app_identity(
                    args.target_bundle_id,
                    apps.get(args.target_bundle_id),
                    agent=False,
                ),
                "agentApp": _interaction_app_identity(
                    args.agent_bundle_id,
                    apps.get(args.agent_bundle_id),
                    agent=True,
                ),
            }
        finally:
            if provider is not None:
                await provider.close()

    emit(asyncio.run(_run()))
    return 0


def cmd_uninstall(args: argparse.Namespace) -> int:
    if not try_import():
        print("pymobiledevice3 not installed", file=sys.stderr)
        return 1
    from pymobiledevice3.services.installation_proxy import InstallationProxyService
    from pymobiledevice3.lockdown import create_using_usbmux

    async def _run() -> None:
        lockdown = await create_using_usbmux(serial=args.udid)
        try:
            async with InstallationProxyService(lockdown=lockdown) as proxy:
                await proxy.uninstall(args.bundle_id)
        finally:
            await lockdown.close()

    asyncio.run(_run())
    emit({"ok": True})
    return 0


async def _take_screenshot(udid: str) -> bytes:
    from pymobiledevice3.lockdown import create_using_usbmux
    from pymobiledevice3.services.screenshot import ScreenshotService

    lockdown = await create_using_usbmux(serial=udid)
    try:
        async with ScreenshotService(lockdown=lockdown) as screenshot:
            return await screenshot.take_screenshot()
    finally:
        await lockdown.close()


def cmd_screenshot(args: argparse.Namespace) -> int:
    if not try_import():
        print("pymobiledevice3 not installed", file=sys.stderr)
        return 1
    data = asyncio.run(_take_screenshot(args.udid))
    with open(args.out, "wb") as f:
        f.write(data)
    emit({"ok": True, "path": args.out})
    return 0


def _png_to_jpeg(raw: bytes, quality: int, max_width: int = 540) -> bytes:
    from PIL import Image

    with Image.open(io.BytesIO(raw)) as img:
        rgb = img.convert("RGB")
        if rgb.width > max_width:
            ratio = max_width / float(rgb.width)
            rgb = rgb.resize(
                (max_width, max(1, int(rgb.height * ratio))),
                Image.Resampling.BILINEAR,
            )
        out = io.BytesIO()
        rgb.save(out, format="JPEG", quality=max(30, min(95, quality)), optimize=False)
        return out.getvalue()


async def _stream_screenshots(
    udid: str, fps: int, quality: int, max_frames: Optional[int]
) -> None:
    from pymobiledevice3.lockdown import create_using_usbmux
    from pymobiledevice3.services.screenshot import ScreenshotService

    interval = 1.0 / max(1, fps)
    lockdown = await create_using_usbmux(serial=udid)
    frames = 0
    try:
        async with ScreenshotService(lockdown=lockdown) as screenshot:
            while max_frames is None or frames < max_frames:
                started = time.monotonic()
                raw = await screenshot.take_screenshot()
                # Keep encode cheap — lockdown screenshots can't exceed ~1–2 FPS anyway
                jpeg = await asyncio.to_thread(_png_to_jpeg, raw, quality, 390)
                emit_frame(jpeg)
                frames += 1
                elapsed = time.monotonic() - started
                delay = interval - elapsed
                if delay > 0:
                    await asyncio.sleep(delay)
    finally:
        await lockdown.close()


def _extract_mjpeg_frames(buffer: bytearray):
    """Yield complete JPEG frames from an MJPEG / multipart byte buffer."""
    while True:
        soi = buffer.find(b"\xff\xd8")
        if soi < 0:
            buffer.clear()
            return
        if soi > 0:
            del buffer[:soi]
        eoi = buffer.find(b"\xff\xd9", 2)
        if eoi < 0:
            return
        frame = bytes(buffer[: eoi + 2])
        del buffer[: eoi + 2]
        yield frame


async def _wait_device_port(udid: str, port: int, timeout: float = 45.0) -> bool:
    from pymobiledevice3 import usbmux
    from pymobiledevice3.exceptions import ConnectionFailedError

    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        device = await usbmux.select_device(udid)
        if device is not None:
            try:
                sock = await device.connect(port)
                try:
                    close = getattr(sock, "close", None)
                    if close is not None:
                        result = close()
                        if asyncio.iscoroutine(result):
                            await result
                except Exception:
                    pass
                return True
            except ConnectionFailedError:
                pass
            except Exception:
                pass
        await asyncio.sleep(0.25)
    return False


def _rt_auth_request(token: str) -> bytes:
    return (
        "GET /wda/locked HTTP/1.1\r\n"
        "Host: 127.0.0.1\r\n"
        f"X-RT-Token: {token}\r\n"
        "Connection: close\r\n\r\n"
    ).encode()


def _http_response_is_ok(first_chunk: bytes) -> bool:
    status_line = first_chunk.split(b"\r\n", 1)[0]
    parts = status_line.split()
    return len(parts) >= 2 and parts[1] == b"200"


async def _device_http_ready(udid: str, port: int, token: str, timeout: float = 3.0) -> bool:
    """Validate a token-protected RT-MMO route directly over usbmux."""
    from pymobiledevice3 import usbmux

    device = await usbmux.select_device(udid)
    if device is None:
        return False
    sock = None
    try:
        sock = await device.connect(port)
        sock.setblocking(False)
        loop = asyncio.get_running_loop()
        request = _rt_auth_request(token)
        await asyncio.wait_for(loop.sock_sendall(sock, request), timeout=timeout)
        first = await asyncio.wait_for(loop.sock_recv(sock, 4096), timeout=timeout)
        return _http_response_is_ok(first)
    except Exception:
        return False
    finally:
        if sock is not None:
            with contextlib.suppress(Exception):
                sock.close()


async def _start_wda_xctest(udid: str, bundle_id: str) -> asyncio.Task:
    """Keep XCUITest runner alive in the background (WDA HTTP starts inside the test)."""
    from pymobiledevice3.lockdown import create_using_usbmux
    from pymobiledevice3.services.dvt.testmanaged.xcuitest import TestConfig, XCUITestService

    lockdown = await create_using_usbmux(serial=udid)
    cfg = await TestConfig.create_for(lockdown, runner_bundle_id=bundle_id)
    service = XCUITestService(lockdown)

    async def _run() -> None:
        try:
            await service.run(cfg)
        finally:
            try:
                await lockdown.close()
            except Exception:
                pass

    return asyncio.create_task(_run(), name=f"wda-xctest-{udid}")


def _free_local_port() -> int:
    """Grab an OS-assigned free 127.0.0.1 port.

    Every stream runs in its own process (one per device). Fixed local ports
    (9100 / 18101) collided across devices — the 2nd device failed to bind and
    dropped to slow screenshots. Ephemeral ports keep each device independent and
    never overlap the Rust WDA-control range (18100–18115).
    """
    import socket

    s = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    try:
        s.bind(("127.0.0.1", 0))
        return int(s.getsockname()[1])
    finally:
        s.close()


async def _configure_mjpeg(udid: str, fps: int, quality: int) -> None:
    """Create a brief WDA session and raise MJPEG framerate/quality."""
    import json
    from pymobiledevice3.tcp_forwarder import UsbmuxTcpForwarder

    local = _free_local_port()
    listening = asyncio.Event()
    forwarder = UsbmuxTcpForwarder(udid, 8100, local, listening_event=listening)
    task = asyncio.create_task(forwarder.start(address="127.0.0.1"))
    try:
        await asyncio.wait_for(listening.wait(), timeout=5)

        async def once(method: str, path: str, body: Optional[dict] = None) -> dict:
            reader, writer = await asyncio.open_connection("127.0.0.1", local)
            payload = b""
            headers = (
                f"{method} {path} HTTP/1.1\r\n"
                f"Host: 127.0.0.1\r\n"
                f"Connection: close\r\n"
            )
            if body is not None:
                raw = json.dumps(body).encode()
                headers += (
                    "Content-Type: application/json\r\n"
                    f"Content-Length: {len(raw)}\r\n\r\n"
                )
                payload = raw
            else:
                headers += "\r\n"
            writer.write(headers.encode() + payload)
            await writer.drain()
            chunks: list[bytes] = []
            while True:
                chunk = await reader.read(65536)
                if not chunk:
                    break
                chunks.append(chunk)
            writer.close()
            with contextlib.suppress(Exception):
                await writer.wait_closed()
            raw = b"".join(chunks)
            _, _, body_b = raw.partition(b"\r\n\r\n")
            try:
                return json.loads(body_b.decode("utf-8", "replace") or "{}")
            except Exception:
                return {}

        sess = await once(
            "POST",
            "/session",
            {
                "capabilities": {"alwaysMatch": {}},
                "desiredCapabilities": {},
            },
        )
        sid = sess.get("sessionId")
        if not sid and isinstance(sess.get("value"), dict):
            sid = sess["value"].get("sessionId")
        if not sid:
            return
        await once(
            "POST",
            f"/session/{sid}/appium/settings",
            {
                "settings": {
                    "mjpegServerFramerate": max(1, min(30, fps)),
                    "mjpegServerScreenshotQuality": max(10, min(95, quality)),
                    "mjpegScalingFactor": 50,
                }
            },
        )
    finally:
        forwarder.stop()
        task.cancel()
        with contextlib.suppress(asyncio.CancelledError):
            await task


async def _stream_mjpeg(
    udid: str,
    local_port: int,
    device_port: int,
    max_frames: Optional[int],
) -> None:
    """Forward device MJPEG port and emit length-prefixed JPEG frames."""
    from pymobiledevice3.tcp_forwarder import UsbmuxTcpForwarder

    listening = asyncio.Event()
    forwarder = UsbmuxTcpForwarder(udid, device_port, local_port, listening_event=listening)
    fwd_task = asyncio.create_task(forwarder.start(address="127.0.0.1"), name="mjpeg-forward")
    try:
        await asyncio.wait_for(listening.wait(), timeout=10)
        await asyncio.sleep(0.2)
        reader, writer = await asyncio.open_connection("127.0.0.1", local_port)
        writer.write(
            b"GET / HTTP/1.1\r\n"
            b"Host: 127.0.0.1\r\n"
            b"Connection: keep-alive\r\n"
            b"\r\n"
        )
        await writer.drain()

        buf = bytearray()
        frames = 0
        while max_frames is None or frames < max_frames:
            chunk = await reader.read(65536)
            if not chunk:
                raise RuntimeError("MJPEG connection closed")
            buf.extend(chunk)
            for jpeg in _extract_mjpeg_frames(buf):
                emit_frame(jpeg)
                frames += 1
                if max_frames is not None and frames >= max_frames:
                    break
        writer.close()
        with contextlib.suppress(Exception):
            await writer.wait_closed()
    finally:
        forwarder.stop()
        fwd_task.cancel()
        with contextlib.suppress(asyncio.CancelledError):
            await fwd_task


def _start_wda_tidevice(udid: str, bundle_id: str):
    """Launch WDA via tidevice xctest (works when DVT SSL is healthy)."""
    import shutil

    tidevice = shutil.which("tidevice")
    cmd = [
        sys.executable,
        "-m",
        "tidevice",
        "-u",
        udid,
        "xctest",
        "-B",
        bundle_id,
    ]
    if tidevice:
        cmd = [tidevice, "-u", udid, "xctest", "-B", bundle_id]
    try:
        return subprocess.Popen(
            cmd,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
    except Exception as exc:
        print(f"tidevice launch failed: {exc}", file=sys.stderr, flush=True)
        return None


async def _stream_auto(
    udid: str,
    fps: int,
    quality: int,
    max_frames: Optional[int],
    mode: str,
    wda_bundle: str,
    mjpeg_port: int,
    wda_port: int = 8100,
) -> None:
    """Prefer WDA MJPEG (smooth); fall back to lockdown screenshots (~1 FPS).

    Does NOT start or kill the WDA XCTest runner — that lifecycle belongs to
    `wda-proxy` / the Rust control plane so nurture + stream share one agent.
    """
    _ = wda_bundle  # kept for CLI compat; runner is owned elsewhere
    use_mjpeg = mode in ("auto", "mjpeg")
    if use_mjpeg:
        ready = await _wait_device_port(udid, mjpeg_port, timeout=3.0)
        if not ready and await _wait_device_port(udid, wda_port, timeout=1.0):
            # WDA HTTP up but MJPEG port not open yet — brief wait.
            ready = await _wait_device_port(udid, mjpeg_port, timeout=8.0)
        if ready:
            # MJPEG quality is configured once by wda-proxy — avoid creating a
            # second WDA session here (that invalidates nurture mid-gesture).
            print("streaming via WDA MJPEG", file=sys.stderr, flush=True)
            try:
                await _stream_mjpeg(udid, _free_local_port(), mjpeg_port, max_frames)
                return
            except Exception as exc:
                print(f"MJPEG stream failed: {exc}", file=sys.stderr, flush=True)
                if mode == "mjpeg":
                    raise
        elif mode == "mjpeg":
            raise RuntimeError(
                f"WDA MJPEG (port {mjpeg_port}) unreachable.\n"
                "Trust developer on iPhone, keep Riviumanagersphone installed, "
                "then Prepare again."
            )
        else:
            print(
                "WDA not ready — falling back to screenshot stream (~1 FPS)",
                file=sys.stderr,
                flush=True,
            )

    await _stream_screenshots(udid, fps, quality, max_frames)


def cmd_stream(args: argparse.Namespace) -> int:
    if not try_import():
        print("pymobiledevice3 not installed", file=sys.stderr)
        return 1
    try:
        asyncio.run(
            _stream_auto(
                args.udid,
                args.fps,
                args.quality,
                args.max_frames,
                args.mode,
                args.wda_bundle,
                args.mjpeg_port,
                getattr(args, "wda_port", 8100),
            )
        )
        return 0
    except KeyboardInterrupt:
        return 0
    except Exception as exc:
        print(f"stream failed: {exc}", file=sys.stderr)
        return 1


def cmd_syslog(args: argparse.Namespace) -> int:
    emit({"log": f"[{args.udid}] syslog via pymobiledevice3 — connect device and use os_trace_relay for live tails\n" * max(1, args.lines // 10 + 1)})
    return 0


def cmd_launch(args: argparse.Namespace) -> int:
    """Launch an app through the same bounded DVT path used by RT-MMO."""
    try:
        asyncio.run(_launch_app_with_environment(args.udid, args.bundle_id, {}))
        emit({"ok": True, "via": "dvt"})
        return 0
    except Exception as exc:
        emit({"ok": False, "error": f"dvt: {exc}"})
        return 1


async def _launch_app_with_environment(
    udid: str,
    bundle_id: str,
    environment: dict[str, str],
) -> int:
    """Launch through DVT so environment secrets never enter child argv."""
    if not try_import():
        raise RuntimeError("pymobiledevice3 not installed")
    from pymobiledevice3.lockdown import create_using_usbmux
    from pymobiledevice3.services.dvt.instruments.process_control import ProcessControl
    from pymobiledevice3.services.dvt.instruments.dvt_provider import DvtProvider

    lockdown = await create_using_usbmux(serial=udid)
    try:
        async with DvtProvider(lockdown) as dvt:
            async with ProcessControl(dvt) as process_control:
                return await process_control.launch(
                    bundle_id,
                    kill_existing=False,
                    environment=environment,
                )
    finally:
        await lockdown.close()


TERMINATE_TIMEOUT_SECONDS = 5.0
TERMINATE_CLEANUP_TIMEOUT_SECONDS = 0.5
TERMINATE_POLL_SECONDS = 0.1


async def _await_before(deadline: float, operation):
    remaining = deadline - asyncio.get_running_loop().time()
    if remaining <= 0:
        raise TimeoutError("app process-control deadline expired")
    try:
        return await asyncio.wait_for(operation(), timeout=remaining)
    except asyncio.TimeoutError as exc:
        raise TimeoutError("app process-control deadline expired") from exc


def _checked_process_pid(value) -> Optional[int]:
    if value is None:
        return None
    if (
        isinstance(value, bool)
        or not isinstance(value, int)
        or value < 0
        or value > (1 << 64) - 1
    ):
        raise RuntimeError("process lookup returned an invalid PID")
    if value == 0:
        return None
    return value


async def _with_bounded_process_control(udid: str, operation):
    if not try_import():
        raise RuntimeError("pymobiledevice3 not installed")
    from pymobiledevice3.lockdown import create_using_usbmux
    from pymobiledevice3.services.dvt.instruments.process_control import ProcessControl
    from pymobiledevice3.services.dvt.instruments.dvt_provider import DvtProvider

    loop = asyncio.get_running_loop()
    deadline = loop.time() + TERMINATE_TIMEOUT_SECONDS
    lockdown = None
    dvt_context = None
    process_context = None
    dvt_entered = False
    process_entered = False
    error_info = (None, None, None)
    try:
        lockdown = await _await_before(
            deadline, lambda: create_using_usbmux(serial=udid)
        )
        dvt_context = DvtProvider(lockdown)
        dvt = await _await_before(deadline, dvt_context.__aenter__)
        dvt_entered = True
        process_context = ProcessControl(dvt)
        process_control = await _await_before(deadline, process_context.__aenter__)
        process_entered = True
        return await operation(process_control, deadline)
    except BaseException as error:
        error_info = (type(error), error, error.__traceback__)
        raise
    finally:
        cleanup_deadline = loop.time() + TERMINATE_CLEANUP_TIMEOUT_SECONDS
        cleanup_operations = []
        if process_entered:
            cleanup_operations.append(
                lambda: process_context.__aexit__(*error_info)
            )
        if dvt_entered:
            cleanup_operations.append(lambda: dvt_context.__aexit__(*error_info))
        if lockdown is not None:
            cleanup_operations.append(lockdown.close)

        cleanup_errors = []
        for cleanup in cleanup_operations:
            try:
                await _await_before(cleanup_deadline, cleanup)
            except BaseException as cleanup_error:
                cleanup_errors.append(cleanup_error)
        if cleanup_errors and error_info[1] is None:
            raise cleanup_errors[0]
        if cleanup_errors:
            print(f"app process cleanup error: {cleanup_errors[0]}", file=sys.stderr)


async def _inspect_app_process(udid: str, bundle_id: str) -> dict:
    async def inspect(process_control, deadline: float) -> dict:
        raw_pid = await _await_before(
            deadline,
            lambda: process_control.process_identifier_for_bundle_identifier(
                bundle_id
            ),
        )
        pid = _checked_process_pid(raw_pid)
        return {
            "ok": True,
            "bundleId": bundle_id,
            "pid": pid,
            "running": pid is not None,
        }

    return await _with_bounded_process_control(udid, inspect)


async def _terminate_app_verified(udid: str, bundle_id: str) -> dict:
    async def terminate(process_control, deadline: float) -> dict:
        raw_pid = await _await_before(
            deadline,
            lambda: process_control.process_identifier_for_bundle_identifier(
                bundle_id
            ),
        )
        old_pid = _checked_process_pid(raw_pid)
        if old_pid is None:
            return {
                "ok": True,
                "bundleId": bundle_id,
                "oldPid": None,
                "running": False,
            }

        await _await_before(deadline, lambda: process_control.kill(old_pid))
        while True:
            raw_current = await _await_before(
                deadline,
                lambda: process_control.process_identifier_for_bundle_identifier(
                    bundle_id
                ),
            )
            if _checked_process_pid(raw_current) is None:
                return {
                    "ok": True,
                    "bundleId": bundle_id,
                    "oldPid": old_pid,
                    "running": False,
                }
            await _await_before(
                deadline, lambda: asyncio.sleep(TERMINATE_POLL_SECONDS)
            )

    return await _with_bounded_process_control(udid, terminate)


def cmd_terminate(args: argparse.Namespace) -> int:
    try:
        emit(asyncio.run(_terminate_app_verified(args.udid, args.bundle_id)))
        return 0
    except Exception as exc:
        emit({"ok": False, "error": str(exc)})
        return 1


def cmd_app_process(args: argparse.Namespace) -> int:
    try:
        emit(asyncio.run(_inspect_app_process(args.udid, args.bundle_id)))
        return 0
    except Exception as exc:
        emit({"ok": False, "error": str(exc)})
        return 1


def cmd_reboot(args: argparse.Namespace) -> int:
    if not try_import():
        print("pymobiledevice3 not installed", file=sys.stderr)
        return 1
    from pymobiledevice3.lockdown import create_using_usbmux
    from pymobiledevice3.services.diagnostics import DiagnosticsService

    async def _run() -> None:
        lockdown = await create_using_usbmux(serial=args.udid)
        try:
            async with DiagnosticsService(lockdown=lockdown) as diag:
                await diag.restart()
        except TypeError:
            # older sync API
            DiagnosticsService(lockdown=lockdown).restart()
        finally:
            await lockdown.close()

    try:
        asyncio.run(_run())
        emit({"ok": True})
        return 0
    except Exception as exc:
        emit({"ok": False, "error": str(exc)})
        return 1


def cmd_tunnel(args: argparse.Namespace) -> int:
    emit({"ok": True, "udid": args.udid, "message": "Start `pymobiledevice3 remote tunnel` / start-tunnel separately for iOS 17+"})
    return 0


def cmd_start_wda(args: argparse.Namespace) -> int:
    """Ensure WDA HTTP is listening on the device (port 8100). Does not keep a proxy."""
    bundle = args.bundle_id

    async def _run() -> dict:
        # Skip MJPEG configure here — it opens a short-lived forward and can hang the command.
        if await _wait_device_port(args.udid, 8100, timeout=1.5):
            return {"ok": True, "udid": args.udid, "alreadyRunning": True, "port": 8100}

        proc = _start_wda_tidevice(args.udid, bundle)
        ok = await _wait_device_port(args.udid, 8100, timeout=45)
        if not ok:
            if proc is not None and proc.poll() is None:
                proc.terminate()
            try:
                task = await _start_wda_xctest(args.udid, bundle)
                ok = await _wait_device_port(args.udid, 8100, timeout=40)
                if not ok:
                    task.cancel()
                    with contextlib.suppress(asyncio.CancelledError):
                        await task
                    return {
                        "ok": False,
                        "udid": args.udid,
                        "error": (
                            "Không start được WDA.\n"
                            "Trust developer trên iPhone, giữ app Riviumanagersphone, "
                            "rồi Prepare lại."
                        ),
                    }
                # Keep XCUITest task alive by parking — caller should use wda-proxy instead.
                # Detach: don't cancel task; process exit will kill it. Prefer tidevice path.
            except Exception as exc:
                return {"ok": False, "udid": args.udid, "error": str(exc)}
        return {
            "ok": True,
            "udid": args.udid,
            "port": 8100,
            "bundleId": bundle,
            "via": "tidevice" if proc is not None else "xcuitest",
        }

    try:
        result = asyncio.run(_run())
        emit(result)
        return 0 if result.get("ok") else 1
    except Exception as exc:
        emit({"ok": False, "error": str(exc)})
        return 1


def _which(name: str) -> Optional[str]:
    import site
    from shutil import which

    resolved = which(name)
    if resolved or sys.platform != "win32":
        return resolved

    scripts_dir = Path(site.getusersitepackages()).parent / "Scripts"
    candidates = [name] if Path(name).suffix else [f"{name}.exe", f"{name}.cmd", name]
    for candidate in candidates:
        executable = scripts_dir / candidate
        if executable.is_file():
            return str(executable)
    return None


def cmd_wda_forward(args: argparse.Namespace) -> int:
    """Keep USB relay alive: localhost:local_port -> device:8100.

    Prefers `tidevice relay` (stable). Falls back to pymobiledevice3 UsbmuxTcpForwarder.
    """
    local = int(args.local_port)
    device_port = int(args.device_port)
    udid = args.udid

    tidevice = _which("tidevice")
    if tidevice:
        # tidevice relay LOCAL REMOTE
        proc = subprocess.Popen(
            [tidevice, "-u", udid, "relay", str(local), str(device_port)],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
        # Wait until HTTP /status answers (or process dies).
        deadline = time.monotonic() + 25
        ready = False
        while time.monotonic() < deadline:
            if proc.poll() is not None:
                emit({"ok": False, "error": f"tidevice relay exited early ({proc.returncode})"})
                return 1
            try:
                import urllib.request

                with urllib.request.urlopen(
                    f"http://127.0.0.1:{local}/status", timeout=1.5
                ) as resp:
                    if resp.status == 200:
                        ready = True
                        break
            except Exception:
                time.sleep(0.35)
        if not ready:
            proc.terminate()
            emit(
                {
                    "ok": False,
                    "error": (
                        f"relay {local}->device:{device_port} lên nhưng WDA /status không trả lời. "
                        "Bấm Start/Agent để mở WDA trên iPhone."
                    ),
                }
            )
            return 1
        emit(
            {
                "ok": True,
                "udid": udid,
                "localPort": local,
                "devicePort": device_port,
                "via": "tidevice-relay",
            }
        )
        try:
            proc.wait()
        except KeyboardInterrupt:
            proc.terminate()
        return 0 if (proc.returncode in (0, None, -15, -9)) else 1

    if not try_import():
        emit({"ok": False, "error": "tidevice/pymobiledevice3 not installed"})
        return 1

    async def _run() -> int:
        from pymobiledevice3.tcp_forwarder import UsbmuxTcpForwarder

        listening = asyncio.Event()
        forwarder = UsbmuxTcpForwarder(
            udid, device_port, local, listening_event=listening
        )
        task = asyncio.create_task(
            forwarder.start(address="127.0.0.1"), name=f"wda-fwd-{udid}"
        )
        try:
            await asyncio.wait_for(listening.wait(), timeout=10)
        except Exception as exc:
            forwarder.stop()
            task.cancel()
            with contextlib.suppress(asyncio.CancelledError):
                await task
            emit({"ok": False, "error": f"wda forward failed: {exc}"})
            return 1

        # Wait for HTTP readiness
        ready = False
        for _ in range(40):
            try:
                reader, writer = await asyncio.wait_for(
                    asyncio.open_connection("127.0.0.1", local), timeout=1.0
                )
                writer.write(
                    b"GET /status HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n"
                )
                await writer.drain()
                data = await asyncio.wait_for(reader.read(512), timeout=2.0)
                writer.close()
                with contextlib.suppress(Exception):
                    await writer.wait_closed()
                if b"200" in data[:32] or b"value" in data:
                    ready = True
                    break
            except Exception:
                await asyncio.sleep(0.25)
        if not ready:
            forwarder.stop()
            task.cancel()
            with contextlib.suppress(asyncio.CancelledError):
                await task
            emit({"ok": False, "error": "forward up but WDA /status silent"})
            return 1

        emit(
            {
                "ok": True,
                "udid": udid,
                "localPort": local,
                "devicePort": device_port,
                "via": "pymobiledevice3",
            }
        )
        try:
            while True:
                await asyncio.sleep(3600)
                if task.done():
                    break
        except asyncio.CancelledError:
            pass
        finally:
            forwarder.stop()
            task.cancel()
            with contextlib.suppress(asyncio.CancelledError):
                await task
        return 0

    try:
        return asyncio.run(_run())
    except KeyboardInterrupt:
        return 0
    except Exception as exc:
        emit({"ok": False, "error": str(exc)})
        return 1


def _tune_mjpeg_http(local_port: int, fps: int = 24, quality: int = 55) -> None:
    """Best-effort MJPEG settings via an already-open local relay. Deletes the temp session."""
    import json
    import urllib.request

    def once(method: str, path: str, body: Optional[dict] = None) -> dict:
        data = None if body is None else json.dumps(body).encode()
        req = urllib.request.Request(
            f"http://127.0.0.1:{local_port}{path}",
            data=data,
            method=method,
        )
        if body is not None:
            req.add_header("Content-Type", "application/json")
        with urllib.request.urlopen(req, timeout=6) as resp:
            raw = resp.read().decode()
            return json.loads(raw) if raw else {}

    sess = once(
        "POST",
        "/session",
        {"capabilities": {"alwaysMatch": {}}, "desiredCapabilities": {}},
    )
    sid = sess.get("sessionId")
    if not sid and isinstance(sess.get("value"), dict):
        sid = sess["value"].get("sessionId")
    if not sid:
        return
    once(
        "POST",
        f"/session/{sid}/appium/settings",
        {
            "settings": {
                "mjpegServerFramerate": max(1, min(30, fps)),
                "mjpegServerScreenshotQuality": max(10, min(95, quality)),
                "mjpegScalingFactor": 50,
            }
        },
    )
    try:
        once("DELETE", f"/session/{sid}")
    except Exception:
        pass


def cmd_wda_proxy(args: argparse.Namespace) -> int:
    """Own one backend-specific WDA relay and bootstrap its agent when needed."""
    import signal

    tidevice = _which("tidevice")
    if not tidevice:
        emit({"ok": False, "error": "tidevice not found — pip install -U tidevice"})
        return 1

    backend = getattr(args, "backend", "stock")
    rt_mmo = backend == "rt-mmo"
    local = int(args.local_port)
    requested_bundle = getattr(args, "bundle_id", None)
    if rt_mmo and requested_bundle in (None, "com.riviu.managersphone.agent.xctrunner"):
        bundle = "com.mrph.svc"
    else:
        bundle = requested_bundle or "com.riviu.managersphone.agent.xctrunner"
    requested_device_port = getattr(args, "device_port", None)
    device_port = int(
        requested_device_port
        if requested_device_port is not None
        else (8906 if rt_mmo else 8100)
    )
    requested_mjpeg_port = getattr(args, "mjpeg_port", None)
    mjpeg_port = int(
        requested_mjpeg_port
        if requested_mjpeg_port is not None
        else (9093 if rt_mmo else 9100)
    )
    token = str(getattr(args, "token", "") or "")
    udid = args.udid
    restart = bool(getattr(args, "restart_wda", False))
    bootstrap_only = bool(getattr(args, "bootstrap_only", False))

    if rt_mmo and not token:
        emit(
            {
                "ok": False,
                "error": "RT-MMO requires RIVIU_RTMMO_TOKEN for FARM_KEY and HTTP readiness",
            }
        )
        return 1
    if bootstrap_only and not rt_mmo:
        emit({"ok": False, "error": "--bootstrap-only is only valid for RT-MMO"})
        return 1

    xctest = None
    relay = None
    relay_job = None
    own_xctest = False
    cleaned = False

    async def wait_port(timeout: float) -> bool:
        return await _wait_device_port(udid, device_port, timeout=timeout)

    async def wait_mjpeg(timeout: float) -> bool:
        return await _wait_device_port(udid, mjpeg_port, timeout=timeout)

    def wait_until_port_closes(attempts: int = 6) -> bool:
        for _ in range(attempts):
            try:
                if not asyncio.run(wait_port(1.5)):
                    return True
            except Exception:
                return False
            time.sleep(1.0)
        return False

    def _stop(proc: Optional[subprocess.Popen]) -> None:
        if proc is None or proc.poll() is not None:
            return
        proc.terminate()
        try:
            proc.wait(timeout=3)
        except Exception:
            proc.kill()

    def _kill_agent_bundle() -> None:
        """Best-effort tear-down of the selected agent before an explicit restart."""
        names = (
            (bundle,)
            if rt_mmo
            else (bundle, "com.facebook.WebDriverAgentRunner.xctrunner")
        )
        for name in names:
            try:
                subprocess.run(
                    [tidevice, "-u", udid, "kill", name],
                    capture_output=True,
                    timeout=12,
                )
            except Exception:
                pass

    def cleanup(_signum=None, _frame=None) -> None:
        nonlocal cleaned, relay_job
        if cleaned:
            return
        cleaned = True
        _stop(relay)
        _windows_close_handle(relay_job)
        relay_job = None
        # Only tear down XCTest we started — leave stream's WDA alone.
        if own_xctest:
            _stop(xctest)
        if _signum is not None:
            raise SystemExit(0)

    signal.signal(signal.SIGTERM, cleanup)
    signal.signal(signal.SIGINT, cleanup)

    try:
        already = asyncio.run(wait_port(5.0))
    except Exception:
        already = False

    if rt_mmo and already and not restart:
        try:
            complete_runtime = asyncio.run(wait_mjpeg(3.0)) and asyncio.run(
                _device_http_ready(udid, device_port, token, timeout=3.0)
            )
        except Exception:
            complete_runtime = False
        if not complete_runtime:
            # A control-only agent was launched without the required runtime
            # environment. Reusing it makes the stream fall back to ~1 FPS and
            # leaves frame-based action confirmation unreliable.
            _kill_agent_bundle()
            if not wait_until_port_closes():
                emit(
                    {
                        "ok": False,
                        "error": f"RT-MMO device port {device_port} did not close after kill",
                    }
                )
                return 1
            time.sleep(2.0)
            already = False

    if restart:
        # Recovery path. Always kill the selected runner bundle, even when its
        # port is already closed: the failure this exists for is a runner whose XCTest
        # thread is blocked, and that process can be stuck with its port shut.
        # Only killing "if the port is open" left those alive, and the next
        # `tidevice xctest` then timed out waiting for a port the zombie owned.
        # Kill once, then wait for the port to close. Repeating the kill was
        # worse than useless: every `tidevice kill` opens its own Instruments
        # connection, and hammering it left the channel busy so the following
        # `tidevice xctest` could not start at all.
        _kill_agent_bundle()
        if not wait_until_port_closes():
            emit(
                {
                    "ok": False,
                    "error": f"RT-MMO device port {device_port} did not close after kill",
                }
            )
            return 1
        # Let iOS reap the process before asking for a new one.
        time.sleep(2.0)
        already = False

    if not already:
        if rt_mmo:
            launch_environment = {
                "USE_PORT": str(device_port),
                "MJPEG_SERVER_PORT": str(mjpeg_port),
                "FARM_KEY": token,
            }
            ok = False
            last_launch_error = ""
            # Some installations need one second launch after the first app
            # process exits before binding HTTP. Keep this bounded at one retry.
            for attempt in range(2):
                try:
                    asyncio.run(
                        _launch_app_with_environment(
                            udid,
                            bundle,
                            launch_environment,
                        )
                    )
                except Exception as exc:
                    last_launch_error = str(exc)
                try:
                    ok = asyncio.run(wait_port(35.0))
                except Exception as exc:
                    last_launch_error = str(exc)
                    ok = False
                if ok:
                    try:
                        ok = asyncio.run(
                            _device_http_ready(
                                udid,
                                device_port,
                                token,
                                timeout=3.0,
                            )
                        )
                    except Exception as exc:
                        last_launch_error = str(exc)
                        ok = False
                    if not ok and not last_launch_error:
                        last_launch_error = "protected /wda/locked auth probe failed"
                if ok:
                    try:
                        ok = asyncio.run(wait_mjpeg(15.0))
                    except Exception as exc:
                        last_launch_error = str(exc)
                        ok = False
                    if not ok:
                        last_launch_error = (
                            f"RT-MMO MJPEG port {mjpeg_port} did not open"
                        )
                if ok:
                    break
                if attempt == 0:
                    _kill_agent_bundle()
                    if not wait_until_port_closes():
                        last_launch_error = (
                            f"RT-MMO device port {device_port} did not close before retry"
                        )
                        break
                    time.sleep(2.0)
            if not ok:
                cleanup()
                detail = f": {last_launch_error}" if last_launch_error else ""
                emit(
                    {
                        "ok": False,
                        "error": f"timeout waiting for RT-MMO on device:{device_port}{detail}",
                    }
                )
                return 1
        else:
            # Stream may own stock WDA later; start XCTest only when :8100 is absent.
            xctest = subprocess.Popen(
                [tidevice, "-u", udid, "xctest", "-B", bundle],
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
            )
            own_xctest = True
            try:
                # A cold start after a kill can take a while on this device; the
                # recovery path needs the longer window or it gives up just before
                # the runner comes up.
                ok = asyncio.run(wait_port(75.0 if restart else 55.0))
            except Exception:
                ok = False
            if not ok:
                cleanup()
                emit({"ok": False, "error": "timeout chờ WDA trên device:8100 (xctest)"})
                return 1
            # If another process already owns the runner, our xctest may exit with 0
            # while :8100 stays up — treat as reuse, don't fail the proxy.
            if xctest.poll() is not None:
                own_xctest = False
                xctest = None

    if bootstrap_only:
        emit(
            {
                "ok": True,
                "udid": udid,
                "devicePort": device_port,
                "mjpegPort": mjpeg_port,
                "backend": backend,
                "bundleId": bundle,
                "restarted": restart,
                "bootstrapOnly": True,
            }
        )
        cleanup()
        return 0

    relay = subprocess.Popen(
        [tidevice, "-u", udid, "relay", str(local), str(device_port)],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )
    if sys.platform == "win32":
        relay_job = _windows_kill_on_close_job(relay)
        if relay_job is None:
            cleanup()
            emit({"ok": False, "error": "failed to own relay with a Windows Job Object"})
            return 1

    readiness_path = "/wda/locked" if rt_mmo else "/status"
    deadline = time.monotonic() + 25
    ready = False
    while time.monotonic() < deadline:
        if own_xctest and xctest is not None and xctest.poll() is not None:
            # Runner died — but stream may have taken over. Keep going if :8100 lives.
            rc = xctest.returncode
            try:
                still = asyncio.run(wait_port(1.5))
            except Exception:
                still = False
            if still:
                own_xctest = False
                xctest = None
            else:
                cleanup()
                emit({"ok": False, "error": f"xctest exited early ({rc})"})
                return 1
        if relay.poll() is not None:
            cleanup()
            emit({"ok": False, "error": f"relay exited early ({relay.returncode})"})
            return 1
        try:
            import urllib.request

            request = urllib.request.Request(
                f"http://127.0.0.1:{local}{readiness_path}", method="GET"
            )
            if rt_mmo:
                request.add_header("X-RT-Token", token)
            with urllib.request.urlopen(request, timeout=1.5) as resp:
                if resp.status == 200:
                    ready = True
                    break
        except Exception:
            time.sleep(0.4)

    if not ready:
        cleanup()
        emit(
            {
                "ok": False,
                "error": f"relay up but WDA {readiness_path} did not return HTTP 200",
            }
        )
        return 1

    emit(
        {
            "ok": True,
            "udid": udid,
            "localPort": local,
            "devicePort": device_port,
            "mjpegPort": mjpeg_port,
            "backend": backend,
            "via": "tidevice-relay" + ("+xctest" if own_xctest else "-reuse"),
            "bundleId": bundle,
            "ownedXctest": own_xctest,
            "restarted": restart,
        }
    )
    try:
        while True:
            if relay.poll() is not None:
                break
            if own_xctest and xctest is not None and xctest.poll() is not None:
                break
            time.sleep(1.0)
    except (KeyboardInterrupt, SystemExit):
        pass
    finally:
        cleanup()
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(prog="riviu_pmd")
    sub = parser.add_subparsers(dest="cmd", required=True)

    sub.add_parser("ping")

    sub.add_parser("list")

    p = sub.add_parser("install")
    p.add_argument("--udid", required=True)
    p.add_argument("--ipa", required=True)

    p = sub.add_parser("is-installed")
    p.add_argument("--udid", required=True)
    p.add_argument("--bundle-id", required=True)

    p = sub.add_parser("inspect-device-capabilities")
    p.add_argument("--udid", required=True)
    p.add_argument(
        "--target-bundle-id",
        default="com.ss.iphone.ugc.Ame",
    )
    p.add_argument("--agent-bundle-id", required=True)
    p.add_argument("--rsd-host", default=None)
    p.add_argument("--rsd-port", type=int, default=None)

    p = sub.add_parser("uninstall")
    p.add_argument("--udid", required=True)
    p.add_argument("--bundle-id", required=True)

    p = sub.add_parser("screenshot")
    p.add_argument("--udid", required=True)
    p.add_argument("--out", required=True)

    p = sub.add_parser("stream")
    p.add_argument("--udid", required=True)
    p.add_argument("--fps", type=int, default=24)
    p.add_argument("--quality", type=int, default=55)
    p.add_argument("--max-frames", type=int, default=None)
    p.add_argument(
        "--mode",
        choices=["auto", "mjpeg", "screenshot"],
        default="auto",
        help="auto: WDA MJPEG if available else screenshot",
    )
    p.add_argument(
        "--wda-bundle",
        default="com.riviu.managersphone.agent.xctrunner",
    )
    p.add_argument("--mjpeg-port", type=int, default=9100)
    p.add_argument("--wda-port", type=int, default=8100)

    p = sub.add_parser("syslog")
    p.add_argument("--udid", required=True)
    p.add_argument("--lines", type=int, default=100)

    p = sub.add_parser("launch")
    p.add_argument("--udid", required=True)
    p.add_argument("--bundle-id", required=True)

    p = sub.add_parser("terminate")
    p.add_argument("--udid", required=True)
    p.add_argument("--bundle-id", required=True)

    p = sub.add_parser("app-process")
    p.add_argument("--udid", required=True)
    p.add_argument("--bundle-id", required=True)

    p = sub.add_parser("reboot")
    p.add_argument("--udid", required=True)

    p = sub.add_parser("tunnel")
    p.add_argument("--udid", required=True)

    p = sub.add_parser("start-wda")
    p.add_argument("--udid", required=True)
    p.add_argument(
        "--bundle-id",
        default="com.riviu.managersphone.agent.xctrunner",
    )

    p = sub.add_parser("wda-forward")
    p.add_argument("--udid", required=True)
    p.add_argument("--local-port", type=int, default=18100)
    p.add_argument("--device-port", type=int, default=8100)

    p = sub.add_parser("wda-proxy")
    p.add_argument("--udid", required=True)
    p.add_argument("--local-port", type=int, default=18100)
    p.add_argument("--backend", choices=["stock", "rt-mmo"], default="stock")
    p.add_argument("--device-port", type=int, default=None)
    p.add_argument("--mjpeg-port", type=int, default=None)
    p.set_defaults(token=os.environ.get("RIVIU_RTMMO_TOKEN", ""))
    p.add_argument(
        "--bundle-id",
        default=None,
    )
    p.add_argument(
        "--restart-wda",
        action="store_true",
        help="Kill existing WDA runner before start (recovery from wedged /status)",
    )
    p.add_argument(
        "--bootstrap-only",
        action="store_true",
        help="Prepare the RT-MMO agent and exit without starting a local relay",
    )

    args = parser.parse_args()
    dispatch = {
        "ping": cmd_ping,
        "list": cmd_list,
        "install": cmd_install,
        "is-installed": cmd_is_installed,
        "inspect-device-capabilities": cmd_inspect_device_capabilities,
        "uninstall": cmd_uninstall,
        "screenshot": cmd_screenshot,
        "stream": cmd_stream,
        "syslog": cmd_syslog,
        "launch": cmd_launch,
        "terminate": cmd_terminate,
        "app-process": cmd_app_process,
        "reboot": cmd_reboot,
        "tunnel": cmd_tunnel,
        "start-wda": cmd_start_wda,
        "wda-forward": cmd_wda_forward,
        "wda-proxy": cmd_wda_proxy,
    }
    return dispatch[args.cmd](args)


if __name__ == "__main__":
    raise SystemExit(main())
