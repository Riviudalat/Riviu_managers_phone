from __future__ import annotations

import base64
import asyncio
import copy
import hashlib
import importlib.util
import io
import json
import os
import plistlib
import socket
import socketserver
import sys
import tempfile
import threading
import time
import unittest
import zipfile
from dataclasses import replace
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from unittest import mock

from PIL import Image, ImageDraw


AGENT_ROOT = Path(__file__).resolve().parents[1]
MODULE_PATH = AGENT_ROOT / "Scripts" / "probe_gate_bc.py"
BASELINE_LOCK_PATH = AGENT_ROOT / "baseline-lock.json"
XCCONFIG_PATH = AGENT_ROOT / "Config" / "RiviuAgent.xcconfig"
W3C_ELEMENT_KEY = "element-6066-11e4-a52e-4f735466cecf"
UNICODE_SAMPLE = "Riviu Unicode \u0111\u01b0\u1ee3c \U0001f525"


def load_probe_module():
    spec = importlib.util.spec_from_file_location("riviu_probe_gate_bc", MODULE_PATH)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"failed to load module: {MODULE_PATH}")
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


def make_jpeg(taps: int, swipes: int) -> bytes:
    """Build a decodable JPEG whose content changes only after an action."""
    image = Image.new("RGB", (64, 96), (18, 24, 32))
    draw = ImageDraw.Draw(image)
    tap_color = ((taps * 67) % 256, 220, (taps * 31) % 256)
    swipe_color = (230, (swipes * 79) % 256, (swipes * 43) % 256)
    draw.rectangle((4, 6, 59, 43), fill=tap_color)
    draw.rectangle((4, 52, 59, 89), fill=swipe_color)
    draw.line((0, taps % 96, 63, taps % 96), fill=(255, 255, 255), width=2)
    draw.line((swipes % 64, 0, swipes % 64, 95), fill=(0, 0, 0), width=2)
    output = io.BytesIO()
    image.save(output, format="JPEG", quality=95, subsampling=0)
    return output.getvalue()


class ControlState:
    def __init__(self, token: str) -> None:
        self.token = token
        self.session_counter = 0
        self.active_session: str | None = None
        self.active_bundle = "com.riviu.managersphone.agent.xctrunner"
        self.active_pid = 101
        self.clipboard = b""
        self.candidate_foreground = False
        self.screenshot_counter = 0
        self.taps = 0
        self.swipes = 0
        self.search_focused = False
        self.search_text = ""
        self.switch_value = "0"
        self.switch_find_count = 0
        self.unicode_events: list[str] = []
        self.keys_payloads: list[list[str]] = []
        self.protected_health_checks = 0
        self.session_control_checks = 0
        self.mjpeg_auth_attempts: list[str | None] = []
        self.mjpeg_frames_sent = 0
        self.mjpeg_invalid = False
        self.mjpeg_disconnect_budget = 0
        self.mjpeg_stall = False
        self.lock = threading.Lock()

    def jpeg(self) -> bytes:
        with self.lock:
            if self.mjpeg_invalid:
                return b"\xff\xd8not-a-jpeg\xff\xd9"
            return make_jpeg(self.taps, self.swipes)


class ControlHandler(BaseHTTPRequestHandler):
    server_version = "Fixture/2"

    def log_message(self, _format, *_args):
        return

    @property
    def state(self) -> ControlState:
        return self.server.state  # type: ignore[attr-defined]

    def _authorized(self) -> bool:
        return self.headers.get("X-Riviu-Token") == self.state.token

    def _json(self, status: int, payload: dict) -> None:
        body = json.dumps(payload).encode("utf-8")
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.send_header("Connection", "close")
        self.end_headers()
        self.wfile.write(body)

    def _body(self) -> dict:
        length = int(self.headers.get("Content-Length", "0"))
        return json.loads(self.rfile.read(length) or b"{}")

    def _session_path(self, suffix: str = "") -> str:
        if self.state.active_session is None:
            return "/session/none" + suffix
        return f"/session/{self.state.active_session}" + suffix

    def do_GET(self):
        if self.path == "/status":
            self._json(
                200,
                {
                    "value": {
                        "riviuAgent": {
                            "agentVersion": "0.1.0",
                            "protocolVersion": 2,
                            "features": ["stream", "tap", "swipe", "clipboard"],
                            "logicalWidth": 375,
                            "logicalHeight": 667,
                            "state": "ready",
                        }
                    }
                },
            )
            return
        if not self._authorized():
            self._json(401, {"value": {"error": "unauthorized"}})
            return
        if self.path == "/riviu/health":
            self.state.protected_health_checks += 1
            self._json(
                200,
                {
                    "value": {
                        "agentVersion": "0.1.0",
                        "protocolVersion": 2,
                        "features": ["stream", "tap", "swipe", "clipboard"],
                        "logicalWidth": 375,
                        "logicalHeight": 667,
                        "state": "ready",
                    }
                },
            )
            return
        if self.path == self._session_path():
            self.state.session_control_checks += 1
            self._json(200, {"value": {"sessionId": self.state.active_session}})
            return
        if self.path == self._session_path("/wda/activeAppInfo"):
            self._json(
                200,
                {
                    "value": {
                        "bundleId": self.state.active_bundle,
                        "pid": self.state.active_pid,
                        "name": "Fixture",
                        "processArguments": [],
                    }
                },
            )
            return
        if self.path == self._session_path("/element/switch/rect"):
            self._json(200, {"value": {"x": 310, "y": 190, "width": 51, "height": 31}})
            return
        if self.path == self._session_path("/element/switch/attribute/value"):
            self._json(200, {"value": self.state.switch_value})
            return
        if self.path == self._session_path("/element/search/text"):
            self.state.unicode_events.append("readback")
            self._json(200, {"value": self.state.search_text})
            return
        if self.path == "/screenshot":
            self.state.screenshot_counter += 1
            frame = make_jpeg(self.state.taps, self.state.swipes)
            self._json(200, {"value": base64.b64encode(frame).decode("ascii")})
            return
        self._json(404, {"value": {"error": "unknown command"}})

    def do_POST(self):
        if not self._authorized():
            self._json(401, {"value": {"error": "unauthorized"}})
            return
        body = self._body()
        if self.path == "/session":
            self.state.session_counter += 1
            session_id = f"session-{self.state.session_counter}"
            self.state.active_session = session_id
            self.state.search_focused = False
            self.state.search_text = ""
            self._json(200, {"sessionId": session_id, "value": {"sessionId": session_id}})
            return
        if self.path == "/wda/tap":
            with self.state.lock:
                self.state.taps += 1
                self.state.switch_value = "1" if self.state.switch_value == "0" else "0"
            self._json(200, {"value": None})
            return
        if self.path == "/wda/swipe":
            with self.state.lock:
                self.state.swipes += 1
            self._json(200, {"value": None})
            return
        if self.path == "/wda/setPasteboard":
            if self.state.candidate_foreground:
                self.state.clipboard = base64.b64decode(body["content"], validate=True)
            self._json(200, {"value": None})
            return
        if self.path == "/wda/getPasteboard":
            content = base64.b64encode(self.state.clipboard).decode("ascii")
            self._json(200, {"value": content})
            return
        if self.path == self._session_path("/element"):
            using = body.get("using")
            value = body.get("value")
            if using == "class name" and value == "XCUIElementTypeSwitch":
                self.state.switch_find_count += 1
                self._json(200, {"value": {"ELEMENT": "switch", W3C_ELEMENT_KEY: "switch"}})
                return
            self.state.unicode_events.append(f"find:{using}:{value}")
            if using != "class name" or value != "XCUIElementTypeSearchField":
                self._json(404, {"value": {"error": "no such element"}})
                return
            self._json(200, {"value": {"ELEMENT": "search", W3C_ELEMENT_KEY: "search"}})
            return
        if self.path == self._session_path("/element/search/click"):
            self.state.search_focused = True
            self.state.unicode_events.append("focus")
            self._json(200, {"value": None})
            return
        if self.path == self._session_path("/element/search/clear"):
            if not self.state.search_focused:
                self._json(400, {"value": {"error": "element not interactable"}})
                return
            self.state.search_text = ""
            self.state.unicode_events.append("clear")
            self._json(200, {"value": None})
            return
        if self.path == self._session_path("/wda/keys"):
            values = body.get("value")
            if not self.state.search_focused or not isinstance(values, list):
                self._json(400, {"value": {"error": "element not interactable"}})
                return
            self.state.keys_payloads.append(values)
            self.state.search_text += "".join(values)
            self.state.unicode_events.append("keys")
            self._json(200, {"value": None})
            return
        self._json(404, {"value": {"error": "unknown command"}})

    def do_DELETE(self):
        if not self._authorized():
            self._json(401, {"value": {"error": "unauthorized"}})
            return
        if self.path == self._session_path():
            self.state.active_session = None
        self._json(200, {"value": None})


class ThreadingMjpegServer(socketserver.ThreadingTCPServer):
    allow_reuse_address = True
    daemon_threads = True


class MjpegHandler(socketserver.BaseRequestHandler):
    def handle(self):
        request = bytearray()
        while b"\r\n\r\n" not in request and len(request) <= 8192:
            chunk = self.request.recv(4096)
            if not chunk:
                return
            request.extend(chunk)
        lines = bytes(request).split(b"\r\n")
        headers: dict[str, str] = {}
        for line in lines[1:]:
            if b":" not in line:
                continue
            name, value = line.split(b":", 1)
            headers[name.decode("ascii", "replace").strip().lower()] = value.decode(
                "utf-8", "replace"
            ).strip()

        state = self.server.state  # type: ignore[attr-defined]
        supplied_token = headers.get("x-riviu-token")
        state.mjpeg_auth_attempts.append(supplied_token)
        if supplied_token != state.token:
            self.request.sendall(
                b"HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            )
            return

        self.request.sendall(
            b"HTTP/1.1 200 OK\r\n"
            b"Content-Type: multipart/x-mixed-replace; boundary=frame\r\n"
            b"Connection: keep-alive\r\n\r\n"
        )
        try:
            connection_frames = 0
            while True:
                if state.mjpeg_stall:
                    time.sleep(0.5)
                    continue
                frame = state.jpeg()
                self.request.sendall(
                    b"--frame\r\nContent-Type: image/jpeg\r\nContent-Length: "
                    + str(len(frame)).encode("ascii")
                    + b"\r\n\r\n"
                    + frame
                    + b"\r\n"
                )
                state.mjpeg_frames_sent += 1
                connection_frames += 1
                if connection_frames >= 2 and state.mjpeg_disconnect_budget > 0:
                    state.mjpeg_disconnect_budget -= 1
                    return
                time.sleep(0.005)
        except (BrokenPipeError, ConnectionResetError, OSError):
            return


class FixtureServers:
    def __init__(self, token: str) -> None:
        self.state = ControlState(token)
        self.http = ThreadingHTTPServer(("127.0.0.1", 0), ControlHandler)
        self.http.state = self.state  # type: ignore[attr-defined]
        self.mjpeg = ThreadingMjpegServer(("127.0.0.1", 0), MjpegHandler)
        self.mjpeg.state = self.state  # type: ignore[attr-defined]
        self.threads = [
            threading.Thread(target=self.http.serve_forever, daemon=True),
            threading.Thread(target=self.mjpeg.serve_forever, daemon=True),
        ]

    def __enter__(self):
        for thread in self.threads:
            thread.start()
        return self

    def __exit__(self, *_args):
        self.http.shutdown()
        self.mjpeg.shutdown()
        self.http.server_close()
        self.mjpeg.server_close()
        for thread in self.threads:
            thread.join(timeout=2)


class FakeAdapter:
    evidence_environment = "FIXTURE_ONLY"

    def __init__(
        self,
        fixture: FixtureServers,
        *,
        fail_foreground: bool = False,
        candidate_foreground_pid: int | None = None,
    ) -> None:
        self.fixture = fixture
        self.fail_foreground = fail_foreground
        self.events: list[str] = []
        self.launch_environments: list[dict[str, str]] = []
        self.prepared_artifact = None
        self.candidate_foreground_pid = candidate_foreground_pid
        self.current_candidate_pid: int | None = None
        self.next_candidate_pid = 1000

    @property
    def control_address(self):
        return self.fixture.http.server_address

    @property
    def mjpeg_address(self):
        return self.fixture.mjpeg.server_address

    def prepare_candidate(self, artifact, _timeout, *, reuse_trusted_install=False):
        self.events.append("prepare")
        self.prepared_artifact = artifact
        self.reuse_trusted_install = reuse_trusted_install
        self.evidence_environment = (
            "SUPPLEMENTAL_MAC_DEVICE" if reuse_trusted_install else "FIXTURE_ONLY"
        )
        return {
            "freshInstall": not reuse_trusted_install,
            "installationMode": (
                "trusted_upgrade" if reuse_trusted_install else "fresh_install"
            ),
            "identityMatch": True,
            "bundleId": artifact.bundle_id,
            "bundleVersion": artifact.bundle_version,
            "bundleBuild": artifact.bundle_build,
            "payloadApp": artifact.payload_app,
            "signerTeamId": artifact.signer_team_id,
            "iosVersion": "16.7.15",
            "productType": "iPhone10,1",
        }

    def terminate_candidate(self):
        self.events.append("terminate")
        old_pid = self.current_candidate_pid
        self.current_candidate_pid = None
        return old_pid

    def wait_candidate_ports_closed(self, _timeout):
        self.events.append("ports-closed")
        return self.current_candidate_pid is None

    def launch_candidate(self, environment):
        self.events.append("launch")
        self.launch_environments.append(dict(environment))
        self.fixture.state.candidate_foreground = True
        self.next_candidate_pid += 1
        self.current_candidate_pid = self.next_candidate_pid
        self.fixture.state.active_pid = self.current_candidate_pid
        return self.current_candidate_pid

    def candidate_process_id(self):
        self.events.append("candidate-process-id")
        return self.current_candidate_pid

    def start_control_relay(self):
        self.events.append("control-relay")

    def start_mjpeg_relay(self):
        self.events.append("mjpeg-relay")

    def foreground(self, _bundle_id):
        self.events.append("foreground")
        self.fixture.state.active_bundle = _bundle_id
        self.fixture.state.candidate_foreground = False
        if self.fail_foreground:
            raise RuntimeError("fixture foreground failed")

    def foreground_candidate_without_restart(self):
        self.events.append("candidate-foreground-pid-stable")
        self.fixture.state.active_bundle = self.prepared_artifact.bundle_id
        self.fixture.state.candidate_foreground = True
        return self.candidate_foreground_pid or self.current_candidate_pid

    def stop_relays(self):
        self.events.append("cleanup")


class ProbeGateTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.probe = load_probe_module()
        cls.source_digest = json.loads(BASELINE_LOCK_PATH.read_text(encoding="ascii"))[
            "outputSourceSha256"
        ]
        cls.xcconfig_digest = hashlib.sha256(XCCONFIG_PATH.read_bytes()).hexdigest()

    def _candidate_manifest(self, root: Path) -> Path:
        artifact_root = root / "artifact"
        artifact_root.mkdir(parents=True)
        ipa = artifact_root / "RiviuAgent-candidate.ipa"
        info = {
            "CFBundleIdentifier": "com.riviu.managersphone.agent.xctrunner",
            "CFBundleShortVersionString": "0.1.0",
            "CFBundleVersion": "1",
            "CFBundleExecutable": "WebDriverAgentRunner-Runner",
        }
        attestation = {
            "RiviuAgentSourceSHA256": self.source_digest,
            "RiviuAgentXcconfigSHA256": self.xcconfig_digest,
            "RiviuAgentProtocolVersion": 2,
            "RiviuAgentObjectiveCUnitTests": "PASS",
            "RiviuAgentXcodeVersion": "16.4",
            "RiviuAgentXcodeBuild": "16F6",
        }
        with zipfile.ZipFile(ipa, "w") as archive:
            root = "Payload/WebDriverAgentRunner-Runner.app"
            archive.writestr(f"{root}/Info.plist", plistlib.dumps(info))
            archive.writestr(f"{root}/WebDriverAgentRunner-Runner", b"fixture executable")
            archive.writestr(
                f"{root}/PlugIns/WebDriverAgentRunner.xctest/Info.plist",
                plistlib.dumps(attestation),
            )
        manifest = {
            "schemaVersion": 1,
            "artifactId": "riviu-agent-ios-candidate",
            "artifactVersion": "0.1.0",
            "gateStatus": "PENDING_MAC_DEVICE",
            "bundleId": "com.riviu.managersphone.agent.xctrunner",
            "bundleVersion": "0.1.0",
            "bundleBuild": "1",
            "payloadApp": "WebDriverAgentRunner-Runner.app",
            "executable": "WebDriverAgentRunner-Runner",
            "attestationBundle": "PlugIns/WebDriverAgentRunner.xctest",
            "signatureIdentifier": "com.riviu.managersphone.agent.xctrunner",
            "signerIdentity": "Apple Development: Fixture Builder (ABCDE12345)",
            "signerTeamId": "ABCDE12345",
            "protocolVersion": 2,
            "ipa": ipa.name,
            "sha256": hashlib.sha256(ipa.read_bytes()).hexdigest(),
            "sourceSha256": self.source_digest,
            "xcconfigSha256": self.xcconfig_digest,
            "controlPort": 8916,
            "mjpegPort": 9094,
            "logicalWidth": 375,
            "logicalHeight": 667,
            "features": ["stream", "tap", "swipe", "clipboard"],
            "objectiveCUnitTests": "PASS",
            "xcode": {"version": "16.4", "build": "16F6"},
        }
        path = artifact_root / "candidate-manifest.json"
        path.write_text(json.dumps(manifest), encoding="ascii")
        return path

    @staticmethod
    def _raw_mjpeg_status(address, token: str | None) -> int:
        with socket.create_connection(address, timeout=1.0) as connection:
            request = ["GET / HTTP/1.1", "Host: 127.0.0.1"]
            if token is not None:
                request.append(f"X-Riviu-Token: {token}")
            request.extend(["Connection: close", "", ""])
            connection.sendall("\r\n".join(request).encode("ascii"))
            status_line = connection.recv(128).split(b"\r\n", 1)[0]
        return int(status_line.split()[1])

    def _passing_measurements(self) -> dict:
        measurements = self.probe.empty_measurements()
        sessions = [f"session-{index}" for index in range(5)]
        measurements.update(
            {
                "candidateFreshInstalled": True,
                "installedIdentityMatch": True,
                "cleanupVerified": True,
                "coldLaunchSuccesses": 5,
                "coldLaunchProcessWitnesses": [
                    {
                        "oldProcessObserved": index > 0,
                        "processAbsentBeforeLaunch": True,
                        "newProcessVerified": True,
                        "newPidFingerprint": f"{index:016d}",
                    }
                    for index in range(5)
                ],
                "sessionFingerprints": sessions,
                "sessionCommandSuccesses": 5,
                "coldLaunchOrder": [
                    ["launch", "health", "foreground", "session", "mjpeg"]
                    for _ in sessions
                ],
                "statusIdentitySuccesses": 5,
                "authStatusesByLaunch": [
                    {"missing": 401, "wrong": 401, "correct": 200} for _ in sessions
                ],
                "mjpegAuthStatusesByLaunch": [
                    {"missing": 401, "wrong": 401, "correct": 200} for _ in sessions
                ],
                "firstJpegCount": 5,
                "gestureControlSamples": 70,
                "gestureControlFrames": 280,
                "settingsActiveChecks": 140,
                "tapCausalChanges": 50,
                "tapSemanticToggles": 50,
                "swipeCausalChanges": 20,
                "swipeForwardCausalChanges": 10,
                "swipeReverseCausalChanges": 10,
                "streamFrames": 300,
                "streamObservedSeconds": 300.0,
                "streamMaxFrameGapSeconds": 2.0,
                "streamReconnects": 1,
                "streamInvalidFrames": 0,
                "streamControlChecks": 60,
                "streamSessionChecks": 60,
                "streamMaxControlCycleSeconds": 5.0,
                "streamMaxControlCompletionGapSeconds": 5.5,
                "streamMaxControlScheduleLatenessSeconds": 0.5,
                "manifestTokenScanClean": True,
                "ipaTokenScanClean": True,
                "sourceTokenScanClean": True,
                "xcconfigTokenScanClean": True,
                "argvTokenScanClean": True,
                "logTokenScanClean": True,
                "reportTokenScanClean": True,
                "clipboardAgentForegroundPidStable": True,
                "clipboardAgentForegroundIdentityVerified": True,
                "clipboardByteExact": 2,
                "unicodeKeysReadBack": True,
                "failures": [],
            }
        )
        return measurements

    def test_secret_never_renders_or_serializes(self):
        token_value = "s" * 32
        secret = self.probe.SecretToken(token_value)
        self.assertEqual("<redacted>", repr(secret))
        encodings = {
            "raw": token_value,
            "base64": base64.b64encode(token_value.encode("utf-8")).decode("ascii"),
            "hex": token_value.encode("utf-8").hex(),
            "windowsPath": r"C:\Users\fixture-user\private\trace.jsonl",
            "macPath": "/Users/fixture-user/private/trace.jsonl",
            "udid": "0123456789abcdef0123456789abcdef01234567",
        }

        for label, value in encodings.items():
            with self.subTest(label=label):
                try:
                    serialized = self.probe.serialize_report({"value": value}, secret)
                except self.probe.ProbeError:
                    continue
                self.assertNotIn(token_value, serialized)
                self.assertNotIn(encodings["base64"], serialized)
                self.assertNotIn(encodings["hex"], serialized)
                self.assertNotIn("fixture-user", serialized)
                self.assertNotIn(encodings["udid"], serialized)

    def test_token_preflight_scans_manifest_ipa_source_xcconfig_and_argv(self):
        token_value = "~" * 32
        token = self.probe.SecretToken(token_value)
        standard_base64 = base64.b64encode(token_value.encode("utf-8")).decode()
        urlsafe_base64 = base64.urlsafe_b64encode(token_value.encode("utf-8")).decode()
        self.assertNotEqual(standard_base64, urlsafe_base64)
        expected_failures = {
            "manifest": "manifestTokenScanClean",
            "ipa": "ipaTokenScanClean",
            "source": "sourceTokenScanClean",
            "xcconfig": "xcconfigTokenScanClean",
            "argv": "argvTokenScanClean",
        }
        for surface, expected_field in expected_failures.items():
            with self.subTest(surface=surface), tempfile.TemporaryDirectory() as tmp:
                root = Path(tmp)
                manifest_path = self._candidate_manifest(root)
                artifact = self.probe.load_candidate_artifact(manifest_path)
                source = root / "prepared-source"
                source.mkdir()
                (source / "clean.m").write_text("void fixture(void) {}\n", encoding="ascii")
                xcconfig = root / "RiviuAgent.xcconfig"
                xcconfig.write_bytes(XCCONFIG_PATH.read_bytes())
                argv = ["probe_gate_bc.py", "--manifest", str(manifest_path)]

                if surface == "manifest":
                    manifest = json.loads(manifest_path.read_text(encoding="ascii"))
                    manifest["ignoredAuditValue"] = token_value.encode("utf-8").hex()
                    manifest_path.write_text(json.dumps(manifest), encoding="ascii")
                elif surface == "ipa":
                    with zipfile.ZipFile(artifact.ipa_path, "a") as archive:
                        archive.writestr(
                            "Payload/WebDriverAgentRunner-Runner.app/token-audit.bin",
                            standard_base64,
                        )
                elif surface == "source":
                    (source / "generated.xcconfig").write_text(token_value, encoding="ascii")
                elif surface == "xcconfig":
                    xcconfig.write_text(token_value, encoding="ascii")
                    artifact = replace(
                        artifact,
                        xcconfig_sha256=hashlib.sha256(xcconfig.read_bytes()).hexdigest(),
                    )
                else:
                    argv.append(urlsafe_base64)

                evidence = self.probe.scan_token_preflight(
                    artifact=artifact,
                    token=token,
                    prepared_source=source,
                    xcconfig=xcconfig,
                    argv=argv,
                )

                self.assertFalse(evidence[expected_field])
                self.assertFalse(self.probe.token_preflight_is_clean(evidence))

    def test_clean_token_preflight_records_all_required_surfaces(self):
        token = self.probe.SecretToken("clean-token-fixture-0123456789abcdef")
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            manifest_path = self._candidate_manifest(root)
            artifact = self.probe.load_candidate_artifact(manifest_path)
            source = root / "prepared-source"
            source.mkdir()
            (source / "clean.m").write_text("void fixture(void) {}\n", encoding="ascii")
            xcconfig = root / "RiviuAgent.xcconfig"
            xcconfig.write_bytes(XCCONFIG_PATH.read_bytes())

            evidence = self.probe.scan_token_preflight(
                artifact=artifact,
                token=token,
                prepared_source=source,
                xcconfig=xcconfig,
                argv=["probe_gate_bc.py", "--manifest", str(manifest_path)],
            )

        self.assertTrue(self.probe.token_preflight_is_clean(evidence))
        self.assertEqual(
            {
                "manifestTokenScanClean",
                "ipaTokenScanClean",
                "sourceTokenScanClean",
                "xcconfigTokenScanClean",
                "argvTokenScanClean",
            },
            set(evidence),
        )

    def test_token_preflight_rejects_xcconfig_not_bound_to_manifest(self):
        token = self.probe.SecretToken("clean-token-fixture-0123456789abcdef")
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            manifest_path = self._candidate_manifest(root)
            artifact = self.probe.load_candidate_artifact(manifest_path)
            source = root / "prepared-source"
            source.mkdir()
            (source / "clean.m").write_text("void fixture(void) {}\n", encoding="ascii")
            xcconfig = root / "RiviuAgent.xcconfig"
            xcconfig.write_text("// different but token-free\n", encoding="ascii")

            with self.assertRaisesRegex(self.probe.ProbeError, "xcconfig SHA-256"):
                self.probe.scan_token_preflight(
                    artifact=artifact,
                    token=token,
                    prepared_source=source,
                    xcconfig=xcconfig,
                    argv=["probe_gate_bc.py", "--manifest", str(manifest_path)],
                )

    def test_locked_source_reconstruction_removes_token_from_child_environment(self):
        token_value = "environment-token-fixture-0123456789abcdef"
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            artifact = self.probe.load_candidate_artifact(self._candidate_manifest(root))
            output = root / "prepared-source"
            output.mkdir()
            completed = self.probe.subprocess.CompletedProcess(
                ["python", "prepare.py"],
                0,
                json.dumps(
                    {
                        "ok": True,
                        "outputSourceSha256": artifact.source_sha256,
                    }
                ),
                "",
            )
            with mock.patch.dict(
                self.probe.os.environ,
                {self.probe.TOKEN_ENVIRONMENT: token_value},
            ), mock.patch.object(
                self.probe.subprocess, "run", return_value=completed
            ) as run:
                self.probe.prepare_locked_source_for_scan(artifact, output)

        child_environment = run.call_args.kwargs["env"]
        self.assertNotIn(self.probe.TOKEN_ENVIRONMENT, child_environment)
        self.assertNotIn(token_value, " ".join(map(str, run.call_args.args[0])))
        self.assertIs(False, run.call_args.kwargs["shell"])
        self.assertGreater(run.call_args.kwargs["timeout"], 0)

    def test_output_guard_blocks_a_token_split_across_writes(self):
        token_value = "split-output-token-0123456789abcdef"
        token = self.probe.SecretToken(token_value)
        destination = io.StringIO()
        guarded = self.probe.SecretScanningWriter(destination, token)

        guarded.write("ordinary log line\n" + token_value[:17])
        with self.assertRaises(self.probe.ProbeError):
            guarded.write(token_value[17:] + "\n")
        guarded.finish()

        self.assertNotIn(token_value, destination.getvalue())

    def test_control_client_auth_matrix_and_unicode_clipboard(self):
        token = "t" * 32
        with FixtureServers(token) as fixture:
            host, port = fixture.http.server_address
            client = self.probe.ControlClient(host, port, self.probe.SecretToken(token), 2.0)
            missing = client.request("GET", "/riviu/health", auth="missing")
            wrong = client.request("GET", "/riviu/health", auth="wrong")
            correct = client.request("GET", "/riviu/health", auth="correct")
            fixture.state.candidate_foreground = True
            sample = "Hay qua \U0001f525".encode("utf-8")
            client.set_clipboard(sample)
            read_back = client.get_clipboard()

        self.assertEqual((401, 401, 200), (missing.status, wrong.status, correct.status))
        self.assertEqual(sample, read_back)

    def test_mjpeg_requires_token_and_returns_real_decodable_jpeg(self):
        token = "m" * 32
        with FixtureServers(token) as fixture:
            self.assertEqual(401, self._raw_mjpeg_status(fixture.mjpeg.server_address, None))
            self.assertEqual(401, self._raw_mjpeg_status(fixture.mjpeg.server_address, "x" * 32))
            frames = self.probe.read_mjpeg_frames(
                *fixture.mjpeg.server_address,
                token=self.probe.SecretToken(token),
                duration_seconds=0.05,
                request_timeout=1.0,
            )

        self.assertGreaterEqual(len(frames), 1)
        for frame in frames:
            with Image.open(io.BytesIO(frame)) as image:
                image.load()
                self.assertEqual("JPEG", image.format)
                self.assertEqual((64, 96), image.size)

    def test_mjpeg_reader_rejects_wrong_token(self):
        with FixtureServers("m" * 32) as fixture:
            with self.assertRaises(self.probe.ProbeError):
                self.probe.read_mjpeg_frames(
                    *fixture.mjpeg.server_address,
                    token=self.probe.SecretToken("x" * 32),
                    duration_seconds=0.05,
                    request_timeout=1.0,
                )

    def test_decoder_rejects_marker_only_payload(self):
        with self.assertRaises(self.probe.ProbeError):
            self.probe.decode_visual_frame(b"\xff\xd8not-a-jpeg\xff\xd9")

    def test_sampler_reconnects_once_and_continues_with_valid_frames(self):
        token = self.probe.SecretToken("j" * 32)
        with FixtureServers(token.reveal()) as fixture:
            fixture.state.mjpeg_disconnect_budget = 1
            sampler = self.probe.MjpegSampler(
                *fixture.mjpeg.server_address,
                token,
                request_timeout=1.0,
                max_reconnects=1,
            )
            sampler.start()
            deadline = time.monotonic() + 3.0
            while time.monotonic() < deadline and (
                sampler.reconnect_count < 1 or sampler.frame_count < 4
            ):
                time.sleep(0.02)
            sampler.assert_healthy()
            sampler.stop()

        self.assertEqual(1, sampler.reconnect_count)
        self.assertGreaterEqual(sampler.frame_count, 4)
        self.assertIsNone(sampler._thread)

    def test_sampler_rejects_invalid_frames_and_stops_its_thread(self):
        token = self.probe.SecretToken("k" * 32)
        with FixtureServers(token.reveal()) as fixture:
            fixture.state.mjpeg_invalid = True
            sampler = self.probe.MjpegSampler(
                *fixture.mjpeg.server_address,
                token,
                request_timeout=0.3,
                max_reconnects=1,
            )
            sampler.start()
            deadline = time.monotonic() + 2.0
            while time.monotonic() < deadline and sampler.invalid_frame_count == 0:
                time.sleep(0.02)
            with self.assertRaises(self.probe.ProbeError):
                sampler.assert_healthy()
            sampler.stop()

        self.assertGreater(sampler.invalid_frame_count, 0)
        self.assertIsNone(sampler._thread)

    def test_sampler_stall_exhausts_bounded_reconnect_budget(self):
        token = self.probe.SecretToken("l" * 32)
        with FixtureServers(token.reveal()) as fixture:
            fixture.state.mjpeg_stall = True
            sampler = self.probe.MjpegSampler(
                *fixture.mjpeg.server_address,
                token,
                request_timeout=0.2,
                max_reconnects=1,
                stall_timeout=0.25,
            )
            sampler.start()
            deadline = time.monotonic() + 3.0
            while time.monotonic() < deadline and sampler.reconnect_count <= 1:
                time.sleep(0.02)
            with self.assertRaises(self.probe.ProbeError):
                sampler.assert_healthy()
            sampler.stop()

        self.assertGreater(sampler.reconnect_count, 1)
        self.assertIsNone(sampler._thread)

    def test_manifest_loader_verifies_artifact_identity_and_hash(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = self._candidate_manifest(Path(tmp))
            artifact = self.probe.load_candidate_artifact(path)
            expected_ipa_sha = hashlib.sha256(artifact.ipa_path.read_bytes()).hexdigest()

        self.assertEqual("com.riviu.managersphone.agent.xctrunner", artifact.bundle_id)
        self.assertEqual("0.1.0", artifact.bundle_version)
        self.assertEqual("1", artifact.bundle_build)
        self.assertEqual("WebDriverAgentRunner-Runner.app", artifact.payload_app)
        self.assertEqual("ABCDE12345", artifact.signer_team_id)
        self.assertEqual(self.source_digest, artifact.source_sha256)
        self.assertEqual(self.xcconfig_digest, artifact.xcconfig_sha256)
        self.assertEqual(
            expected_ipa_sha,
            artifact.ipa_sha256,
        )

    def test_manifest_loader_binds_source_tests_and_xcode_to_ipa_metadata(self):
        mutations = {
            "source": ("RiviuAgentSourceSHA256", "f" * 64),
            "xcconfig": ("RiviuAgentXcconfigSHA256", "e" * 64),
            "unit tests": ("RiviuAgentObjectiveCUnitTests", "UNKNOWN"),
            "protocol": ("RiviuAgentProtocolVersion", 1),
            "protocol string": ("RiviuAgentProtocolVersion", "2"),
            "Xcode": ("RiviuAgentXcodeVersion", "15.0"),
        }
        for label, (key, value) in mutations.items():
            with self.subTest(label=label), tempfile.TemporaryDirectory() as tmp:
                path = self._candidate_manifest(Path(tmp))
                manifest = json.loads(path.read_text(encoding="ascii"))
                ipa = path.parent / manifest["ipa"]
                rewritten = ipa.with_suffix(".new.ipa")
                with zipfile.ZipFile(ipa) as source, zipfile.ZipFile(rewritten, "w") as target:
                    for entry in source.infolist():
                        payload = source.read(entry.filename)
                        if entry.filename.endswith(
                            "/PlugIns/WebDriverAgentRunner.xctest/Info.plist"
                        ):
                            info = plistlib.loads(payload)
                            info[key] = value
                            payload = plistlib.dumps(info)
                        target.writestr(entry, payload)
                rewritten.replace(ipa)
                manifest["sha256"] = hashlib.sha256(ipa.read_bytes()).hexdigest()
                path.write_text(json.dumps(manifest), encoding="ascii")
                with self.assertRaises(self.probe.ProbeError):
                    self.probe.load_candidate_artifact(path)

    def test_manifest_loader_rejects_hash_source_features_and_ports_mismatch(self):
        mutations = {
            "IPA SHA-256": ("sha256", "0" * 64),
            "source digest": ("sourceSha256", "b" * 64),
            "xcconfig digest": ("xcconfigSha256", "e" * 64),
            "features": ("features", ["stream", "tap", "swipe", "clipboard", "text"]),
            "control port": ("controlPort", 8906),
            "MJPEG port": ("mjpegPort", 9093),
        }
        for label, (field, value) in mutations.items():
            with self.subTest(label=label), tempfile.TemporaryDirectory() as tmp:
                path = self._candidate_manifest(Path(tmp))
                manifest = json.loads(path.read_text(encoding="ascii"))
                manifest[field] = value
                path.write_text(json.dumps(manifest), encoding="ascii")
                with self.assertRaises(self.probe.ProbeError):
                    self.probe.load_candidate_artifact(path)

    def test_manifest_loader_binds_xcconfig_digest_to_baseline_lock(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            path = self._candidate_manifest(root)
            lock = json.loads(BASELINE_LOCK_PATH.read_text(encoding="ascii"))
            lock["xcconfigSha256"] = "e" * 64
            lock_path = root / "baseline-lock.json"
            lock_path.write_text(json.dumps(lock), encoding="ascii")

            with self.assertRaisesRegex(self.probe.ProbeError, "xcconfig"):
                self.probe.load_candidate_artifact(path, lock_path=lock_path)

    def test_manifest_loader_rejects_ipa_path_traversal(self):
        for unsafe_path in ("../outside.ipa", "nested/../../outside.ipa"):
            with self.subTest(ipa=unsafe_path), tempfile.TemporaryDirectory() as tmp:
                root = Path(tmp)
                path = self._candidate_manifest(root)
                outside = root / "outside.ipa"
                outside.write_bytes(b"fixture signed candidate")
                manifest = json.loads(path.read_text(encoding="ascii"))
                manifest["ipa"] = unsafe_path
                manifest["sha256"] = hashlib.sha256(outside.read_bytes()).hexdigest()
                path.write_text(json.dumps(manifest), encoding="ascii")
                with self.assertRaises(self.probe.ProbeError):
                    self.probe.load_candidate_artifact(path)

    def test_manifest_loader_enforces_the_riviu_candidate_identity_policy(self):
        mutations = (
            {
                "bundleId": "com.example.other.xctrunner",
                "signatureIdentifier": "com.example.other.xctrunner",
            },
            {"payloadApp": "OtherRunner.app"},
            {"executable": "OtherRunner"},
        )
        for mutation in mutations:
            with self.subTest(mutation=mutation), tempfile.TemporaryDirectory() as tmp:
                path = self._candidate_manifest(Path(tmp))
                manifest = json.loads(path.read_text(encoding="ascii"))
                manifest.update(mutation)
                path.write_text(json.dumps(manifest), encoding="ascii")
                with self.assertRaises(self.probe.ProbeError):
                    self.probe.load_candidate_artifact(path)

    def test_gate_evaluators_use_fixed_thresholds_not_lowered_config(self):
        config = self.probe.ProbeConfig(
            cold_launches=1,
            tap_attempts=1,
            swipe_attempts=1,
            stream_seconds=0.01,
        )
        passing = self._passing_measurements()

        with self.assertRaises(self.probe.ProbeError):
            self.probe.validate_live_config(config)
        with self.assertRaises(self.probe.ProbeError):
            self.probe.validate_live_config(
                self.probe.ProbeConfig(foreground_bundle="com.example.spoof")
            )
        self.assertEqual(
            "PASS", self.probe.evaluate_gate_b(passing, self.probe.LIVE_ENVIRONMENT)
        )
        self.assertEqual(
            "PASS", self.probe.evaluate_gate_c(passing, self.probe.LIVE_ENVIRONMENT)
        )
        self.assertEqual(
            "PASS", self.probe.evaluate_gate(passing, self.probe.LIVE_ENVIRONMENT)
        )

        gate_b_below = {
            "cold launches": ("coldLaunchSuccesses", 4),
            "fresh sessions": (
                "sessionFingerprints",
                [f"session-{index}" for index in range(4)],
            ),
            "first JPEGs": ("firstJpegCount", 4),
            "status identities": ("statusIdentitySuccesses", 4),
            "cold process witnesses": (
                "coldLaunchProcessWitnesses",
                passing["coldLaunchProcessWitnesses"][:4],
            ),
        }
        gate_b_below.update(
            {
                field: (field, False)
                for field in (
                    "manifestTokenScanClean",
                    "ipaTokenScanClean",
                    "sourceTokenScanClean",
                    "xcconfigTokenScanClean",
                    "argvTokenScanClean",
                    "logTokenScanClean",
                    "reportTokenScanClean",
                )
            }
        )
        for label, (field, value) in gate_b_below.items():
            with self.subTest(gate="B", label=label):
                measurements = copy.deepcopy(passing)
                measurements[field] = value
                self.assertEqual(
                    "FAIL",
                    self.probe.evaluate_gate_b(
                        measurements, self.probe.LIVE_ENVIRONMENT
                    ),
                )

        missing_token_measurement = copy.deepcopy(passing)
        missing_token_measurement.pop("xcconfigTokenScanClean")
        self.assertEqual(
            "FAIL",
            self.probe.evaluate_gate_b(
                missing_token_measurement, self.probe.LIVE_ENVIRONMENT
            ),
        )

        for field in ("authStatusesByLaunch", "mjpegAuthStatusesByLaunch"):
            with self.subTest(gate="B", label=field):
                measurements = copy.deepcopy(passing)
                measurements[field][2]["wrong"] = 200
                self.assertEqual(
                    "FAIL",
                    self.probe.evaluate_gate_b(
                        measurements, self.probe.LIVE_ENVIRONMENT
                    ),
                )

        for label, mutate in (
            (
                "duplicate process PID witness",
                lambda value: value[1].update(
                    newPidFingerprint=value[0]["newPidFingerprint"]
                ),
            ),
            (
                "missing prior process witness",
                lambda value: value[1].update(oldProcessObserved=False),
            ),
        ):
            with self.subTest(gate="B", label=label):
                measurements = copy.deepcopy(passing)
                mutate(measurements["coldLaunchProcessWitnesses"])
                self.assertEqual(
                    "FAIL",
                    self.probe.evaluate_gate_b(
                        measurements, self.probe.LIVE_ENVIRONMENT
                    ),
                )

        gate_c_below = {
            "gesture controls": ("gestureControlSamples", 69),
            "gesture control frames": ("gestureControlFrames", 279),
            "Settings active checks": ("settingsActiveChecks", 139),
            "tap": ("tapCausalChanges", 49),
            "tap semantic": ("tapSemanticToggles", 49),
            "swipe": ("swipeCausalChanges", 19),
            "swipe forward": ("swipeForwardCausalChanges", 9),
            "swipe reverse": ("swipeReverseCausalChanges", 9),
            "duration": ("streamObservedSeconds", 299.99),
            "cadence": ("streamFrames", 299),
            "gap": ("streamMaxFrameGapSeconds", 2.001),
            "reconnect": ("streamReconnects", 2),
            "health checks": ("streamControlChecks", 59),
            "session checks": ("streamSessionChecks", 59),
            "control cycle deadline": ("streamMaxControlCycleSeconds", 5.001),
            "control completion gap": (
                "streamMaxControlCompletionGapSeconds",
                5.501,
            ),
            "control schedule lateness": (
                "streamMaxControlScheduleLatenessSeconds",
                0.501,
            ),
            "clipboard agent foreground": ("clipboardAgentForegroundPidStable", False),
            "clipboard foreground identity": (
                "clipboardAgentForegroundIdentityVerified",
                False,
            ),
            "Unicode readback": ("unicodeKeysReadBack", False),
        }
        for label, (field, value) in gate_c_below.items():
            with self.subTest(gate="C", label=label):
                measurements = copy.deepcopy(passing)
                measurements[field] = value
                self.assertEqual(
                    "FAIL",
                    self.probe.evaluate_gate_c(
                        measurements, self.probe.LIVE_ENVIRONMENT
                    ),
                )

    def test_fixture_runner_is_fixture_only_and_uses_verified_artifact(self):
        token = "r" * 32
        with tempfile.TemporaryDirectory() as tmp, FixtureServers(token) as fixture:
            artifact = self.probe.load_candidate_artifact(
                self._candidate_manifest(Path(tmp))
            )
            adapter = FakeAdapter(fixture)
            config = self.probe.ProbeConfig(
                cold_launches=2,
                tap_attempts=2,
                swipe_attempts=2,
                stream_seconds=0.05,
                request_timeout=2.0,
                action_settle_seconds=0.0,
            )
            report = self.probe.ProbeRunner(
                adapter=adapter,
                config=config,
                token=self.probe.SecretToken(token),
                artifact=artifact,
            ).run()

        self.assertEqual("FIXTURE_ONLY", report["gateStatus"])
        self.assertNotEqual("PASS", report["gateStatus"])
        self.assertIs(artifact, adapter.prepared_artifact)
        self.assertEqual(2, len(set(report["measurements"]["sessionFingerprints"])))
        witnesses = report["measurements"]["coldLaunchProcessWitnesses"]
        self.assertEqual(2, len(witnesses))
        self.assertFalse(witnesses[0]["oldProcessObserved"])
        self.assertTrue(witnesses[1]["oldProcessObserved"])
        self.assertEqual(2, len({item["newPidFingerprint"] for item in witnesses}))
        self.assertEqual(2, report["measurements"]["tapCausalChanges"])
        self.assertEqual(2, report["measurements"]["swipeCausalChanges"])
        self.assertTrue(report["measurements"]["unicodeKeysReadBack"])
        self.assertEqual(
            {"missing": 401, "wrong": 401, "correct": 200},
            report["measurements"]["mjpegAuthStatusesByLaunch"][-1],
        )
        self.assertEqual(2, report["measurements"]["statusIdentitySuccesses"])
        self.assertEqual(2, len(report["measurements"]["authStatusesByLaunch"]))
        self.assertEqual(2, len(report["measurements"]["mjpegAuthStatusesByLaunch"]))
        self.assertEqual(2, report["measurements"]["tapSemanticToggles"])
        self.assertEqual(2, fixture.state.switch_find_count)
        self.assertEqual(8, report["measurements"]["settingsActiveChecks"])
        self.assertEqual(3, report["requirements"]["additionalControlFramesPerAction"])
        self.assertEqual(4, report["requirements"]["controlFrameSamplesPerAction"])
        self.assertNotIn("controlFramesPerAction", report["requirements"])
        self.assertEqual(
            "XCUIElementTypeSwitch",
            report["requirements"]["visualEvidence"]["tapSemanticTarget"],
        )
        self.assertGreaterEqual(report["measurements"]["streamControlChecks"], 1)
        self.assertGreaterEqual(report["measurements"]["streamSessionChecks"], 1)
        self.assertLessEqual(
            report["measurements"]["streamMaxControlCycleSeconds"], 5.0
        )
        self.assertLessEqual(
            report["measurements"]["streamMaxControlCompletionGapSeconds"], 5.5
        )
        self.assertTrue(report["measurements"]["clipboardAgentForegroundPidStable"])
        self.assertTrue(
            report["measurements"]["clipboardAgentForegroundIdentityVerified"]
        )
        self.assertTrue(report["measurements"]["reportTokenScanClean"])
        self.assertFalse(report["measurements"]["logTokenScanClean"])
        self.assertIn("candidate-foreground-pid-stable", adapter.events)
        self.assertEqual(
            ["launch", "health", "foreground", "session", "mjpeg"],
            report["measurements"]["coldLaunchOrder"][0],
        )
        expected_environment = {
            "USE_PORT": "8916",
            "MJPEG_SERVER_PORT": "9094",
            "RIVIU_AGENT_TOKEN": token,
            "WDA_PRODUCT_BUNDLE_IDENTIFIER": artifact.bundle_id,
            "USE_IP": "127.0.0.1",
        }
        self.assertTrue(adapter.launch_environments)
        self.assertTrue(all(item == expected_environment for item in adapter.launch_environments))
        self.assertEqual(
            [
                "find:class name:XCUIElementTypeSearchField",
                "focus",
                "clear",
                "keys",
                "readback",
            ],
            fixture.state.unicode_events,
        )
        self.assertEqual([list(UNICODE_SAMPLE)], fixture.state.keys_payloads)
        self.assertEqual(UNICODE_SAMPLE, fixture.state.search_text)
        self.assertEqual(0, fixture.state.screenshot_counter)
        self.assertEqual(["cleanup", "terminate", "ports-closed"], adapter.events[-3:])

    def test_manual_trust_pause_runs_after_fresh_install(self):
        token = "t" * 32
        with tempfile.TemporaryDirectory() as tmp, FixtureServers(token) as fixture:
            artifact = self.probe.load_candidate_artifact(
                self._candidate_manifest(Path(tmp))
            )
            adapter = FakeAdapter(fixture)
            config = self.probe.ProbeConfig(
                cold_launches=1,
                tap_attempts=1,
                swipe_attempts=1,
                stream_seconds=0.05,
                request_timeout=2.0,
                action_settle_seconds=0.0,
                wait_for_trust=True,
            )
            with mock.patch.object(self.probe, "wait_for_manual_trust") as pause:
                report = self.probe.ProbeRunner(
                    adapter=adapter,
                    config=config,
                    token=self.probe.SecretToken(token),
                    artifact=artifact,
                ).run()

        pause.assert_called_once_with()
        self.assertTrue(report["measurements"]["manualTrustPauseRequested"])
        self.assertTrue(report["measurements"]["manualTrustPauseCompleted"])
        self.assertEqual("FIXTURE_ONLY", report["gateStatus"])

    def test_manual_trust_pause_requires_interactive_input(self):
        with mock.patch("builtins.input", side_effect=EOFError):
            with self.assertRaisesRegex(
                self.probe.ProbeError, "interactive terminal"
            ):
                self.probe.wait_for_manual_trust()

    def test_reuse_trusted_install_is_supplemental_only(self):
        token = "u" * 32
        with tempfile.TemporaryDirectory() as tmp, FixtureServers(token) as fixture:
            artifact = self.probe.load_candidate_artifact(
                self._candidate_manifest(Path(tmp))
            )
            adapter = FakeAdapter(fixture)
            adapter.evidence_environment = "SUPPLEMENTAL_MAC_DEVICE"
            config = self.probe.ProbeConfig(
                cold_launches=1,
                tap_attempts=1,
                swipe_attempts=1,
                stream_seconds=0.05,
                request_timeout=2.0,
                action_settle_seconds=0.0,
                reuse_trusted_install=True,
            )
            report = self.probe.ProbeRunner(
                adapter=adapter,
                config=config,
                token=self.probe.SecretToken(token),
                artifact=artifact,
            ).run()

        self.assertTrue(adapter.reuse_trusted_install)
        self.assertFalse(report["measurements"]["candidateFreshInstalled"])
        self.assertEqual("trusted_upgrade", report["device"]["installationMode"])
        self.assertEqual("SUPPLEMENTAL_ONLY", report["gateStatus"])

    def test_trust_pause_and_reuse_modes_cannot_be_combined(self):
        with self.assertRaisesRegex(
            self.probe.ProbeError, "mutually exclusive"
        ):
            self.probe.ProbeConfig(
                wait_for_trust=True,
                reuse_trusted_install=True,
            )

    def test_cold_launch_rejects_pid_change_before_readiness_is_counted(self):
        class RelaunchingAdapter(FakeAdapter):
            def candidate_process_id(self):
                observed = super().candidate_process_id()
                return None if observed is None else observed + 1

        token = "p" * 32
        with tempfile.TemporaryDirectory() as tmp, FixtureServers(token) as fixture:
            artifact = self.probe.load_candidate_artifact(
                self._candidate_manifest(Path(tmp))
            )
            report = self.probe.ProbeRunner(
                adapter=RelaunchingAdapter(fixture),
                config=self.probe.ProbeConfig(
                    cold_launches=1,
                    tap_attempts=1,
                    swipe_attempts=1,
                    stream_seconds=0.01,
                    request_timeout=2.0,
                    action_settle_seconds=0.0,
                ),
                token=self.probe.SecretToken(token),
                artifact=artifact,
            ).run()

        self.assertEqual(0, report["measurements"]["coldLaunchSuccesses"])
        self.assertEqual([], report["measurements"]["coldLaunchProcessWitnesses"])
        self.assertTrue(
            any(
                "candidate PID changed before cold-launch readiness" in failure
                for failure in report["measurements"]["failures"]
            )
        )

    def test_cold_launch_rejects_pid_change_before_next_cycle_teardown(self):
        class RelaunchingBetweenCyclesAdapter(FakeAdapter):
            def __init__(self, fixture):
                super().__init__(fixture)
                self.terminate_calls = 0

            def terminate_candidate(self):
                self.terminate_calls += 1
                if self.terminate_calls == 2 and self.current_candidate_pid is not None:
                    self.current_candidate_pid += 1
                return super().terminate_candidate()

        token = "q" * 32
        with tempfile.TemporaryDirectory() as tmp, FixtureServers(token) as fixture:
            artifact = self.probe.load_candidate_artifact(
                self._candidate_manifest(Path(tmp))
            )
            report = self.probe.ProbeRunner(
                adapter=RelaunchingBetweenCyclesAdapter(fixture),
                config=self.probe.ProbeConfig(
                    cold_launches=2,
                    tap_attempts=1,
                    swipe_attempts=1,
                    stream_seconds=0.01,
                    request_timeout=2.0,
                    action_settle_seconds=0.0,
                ),
                token=self.probe.SecretToken(token),
                artifact=artifact,
            ).run()

        self.assertEqual(1, report["measurements"]["coldLaunchSuccesses"])
        self.assertEqual(1, len(report["measurements"]["coldLaunchProcessWitnesses"]))
        self.assertTrue(
            any(
                "candidate PID changed before the next cold launch" in failure
                for failure in report["measurements"]["failures"]
            )
        )

    def test_cold_launch_rejects_pid_change_before_final_cleanup(self):
        class RelaunchingBeforeCleanupAdapter(FakeAdapter):
            def __init__(self, fixture):
                super().__init__(fixture)
                self.terminate_calls = 0

            def terminate_candidate(self):
                self.terminate_calls += 1
                if self.terminate_calls == 2 and self.current_candidate_pid is not None:
                    self.current_candidate_pid += 1
                return super().terminate_candidate()

        token = "v" * 32
        with tempfile.TemporaryDirectory() as tmp, FixtureServers(token) as fixture:
            artifact = self.probe.load_candidate_artifact(
                self._candidate_manifest(Path(tmp))
            )
            report = self.probe.ProbeRunner(
                adapter=RelaunchingBeforeCleanupAdapter(fixture),
                config=self.probe.ProbeConfig(
                    cold_launches=1,
                    tap_attempts=1,
                    swipe_attempts=1,
                    stream_seconds=0.01,
                    request_timeout=2.0,
                    action_settle_seconds=0.0,
                ),
                token=self.probe.SecretToken(token),
                artifact=artifact,
            ).run()

        self.assertEqual(1, report["measurements"]["coldLaunchSuccesses"])
        self.assertFalse(report["measurements"]["cleanupVerified"])
        self.assertIn(
            "cleanup verification failed", report["measurements"]["failures"]
        )

    def test_control_cadence_rejects_a_delayed_check_instead_of_catching_up(self):
        measurements = self.probe.empty_measurements()
        completion = self.probe.record_control_cadence_sample(
            measurements,
            scheduled=0.0,
            started=0.0,
            completed=0.1,
            previous_completion=0.0,
        )
        self.assertEqual(0.1, completion)

        with self.assertRaisesRegex(self.probe.ProbeError, "cycle deadline"):
            self.probe.record_control_cadence_sample(
                measurements,
                scheduled=5.0,
                started=5.0,
                completed=14.0,
                previous_completion=completion,
            )

    def test_clipboard_requires_the_foreground_identity_to_match_the_stable_pid(self):
        token = "p" * 32
        with tempfile.TemporaryDirectory() as tmp, FixtureServers(token) as fixture:
            artifact = self.probe.load_candidate_artifact(
                self._candidate_manifest(Path(tmp))
            )
            adapter = FakeAdapter(fixture, candidate_foreground_pid=202)
            report = self.probe.ProbeRunner(
                adapter=adapter,
                artifact=artifact,
                token=self.probe.SecretToken(token),
                config=self.probe.ProbeConfig(
                    cold_launches=1,
                    tap_attempts=1,
                    swipe_attempts=1,
                    stream_seconds=0.01,
                    request_timeout=1.0,
                    action_settle_seconds=0.0,
                ),
            ).run()

        self.assertFalse(
            report["measurements"]["clipboardAgentForegroundIdentityVerified"]
        )
        self.assertTrue(report["measurements"]["failures"])

    def test_runner_cleanup_terminates_candidate_after_failure(self):
        token = "f" * 32
        with tempfile.TemporaryDirectory() as tmp, FixtureServers(token) as fixture:
            artifact = self.probe.load_candidate_artifact(
                self._candidate_manifest(Path(tmp))
            )
            adapter = FakeAdapter(fixture, fail_foreground=True)
            report = self.probe.ProbeRunner(
                adapter=adapter,
                config=self.probe.ProbeConfig(
                    cold_launches=1,
                    tap_attempts=1,
                    swipe_attempts=1,
                    stream_seconds=0.01,
                    request_timeout=1.0,
                    action_settle_seconds=0.0,
                ),
                token=self.probe.SecretToken(token),
                artifact=artifact,
            ).run()

        self.assertNotEqual("PASS", report["gateStatus"])
        self.assertTrue(report["measurements"]["failures"])
        self.assertEqual(["cleanup", "terminate", "ports-closed"], adapter.events[-3:])

    def test_unicode_ack_without_readback_prevents_gate_c_pass(self):
        config = self.probe.ProbeConfig(
            cold_launches=1,
            tap_attempts=1,
            swipe_attempts=1,
            stream_seconds=0.01,
        )
        measurements = self._passing_measurements()
        measurements["unicodeKeysAccepted"] = True
        measurements["unicodeKeysReadBack"] = False

        self.assertEqual(
            "FAIL",
            self.probe.evaluate_gate_c(measurements, self.probe.LIVE_ENVIRONMENT),
        )

    def test_evidence_is_verified_before_any_existing_report_is_replaced(self):
        token = self.probe.SecretToken("v" * 32)
        report = {"gateB": "FIXTURE_ONLY", "gateC": "FIXTURE_ONLY"}
        with tempfile.TemporaryDirectory() as tmp:
            output = Path(tmp) / "candidate-probes.json"
            output.write_text("existing evidence\n", encoding="ascii")

            def reject(_paths):
                raise self.probe.ProbeError("fixture verifier rejected evidence")

            with self.assertRaises(self.probe.ProbeError):
                self.probe.write_evidence(output, report, token, verifier=reject)

            self.assertEqual("existing evidence\n", output.read_text(encoding="ascii"))
            self.assertFalse((output.parent / "gate-b.md").exists())
            self.assertFalse((output.parent / "gate-c.md").exists())

    def test_evidence_verifier_receives_json_and_both_gate_documents(self):
        token = self.probe.SecretToken("w" * 32)
        report = {"gateB": "FIXTURE_ONLY", "gateC": "FIXTURE_ONLY"}
        verified_names = []
        with tempfile.TemporaryDirectory() as tmp:
            output = Path(tmp) / "candidate-probes.json"

            def accept(paths):
                verified_names.extend(sorted(path.name for path in paths))
                self.assertTrue(all(path.exists() for path in paths))

            self.probe.write_evidence(output, report, token, verifier=accept)

            self.assertTrue(output.exists())
            self.assertTrue((output.parent / "gate-b.md").exists())
            self.assertTrue((output.parent / "gate-c.md").exists())
        self.assertEqual(
            ["candidate-probes.json", "gate-b.md", "gate-c.md"], verified_names
        )

    def test_evidence_publication_rolls_back_the_whole_set_on_replace_failure(self):
        token = self.probe.SecretToken("transaction-token-fixture-0123456789")
        report = {"gateB": "FIXTURE_ONLY", "gateC": "FIXTURE_ONLY"}
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            output = root / "candidate-probes.json"
            old = {
                output: "old json\n",
                root / "gate-b.md": "old gate b\n",
                root / "gate-c.md": "old gate c\n",
            }
            for path, contents in old.items():
                path.write_text(contents, encoding="ascii")
            real_replace = os.replace
            publication_count = 0

            def fail_second_publication(source, destination):
                nonlocal publication_count
                source_path = Path(source)
                destination_path = Path(destination)
                is_publication = (
                    source_path.parent.name == "new"
                    and destination_path.parent == root
                )
                if is_publication:
                    publication_count += 1
                    if publication_count == 2:
                        raise OSError("fixture second publication failed")
                return real_replace(source, destination)

            with mock.patch.object(
                self.probe.os, "replace", side_effect=fail_second_publication
            ), self.assertRaises(self.probe.ProbeError):
                self.probe.write_evidence(
                    output, report, token, verifier=lambda _paths: None
                )

            self.assertEqual(
                {path: path.read_text(encoding="ascii") for path in old}, old
            )

    def test_evidence_output_rejects_markdown_name_collisions(self):
        token = self.probe.SecretToken("z" * 32)
        report = {"gateB": "FIXTURE_ONLY", "gateC": "FIXTURE_ONLY"}
        with tempfile.TemporaryDirectory() as tmp:
            for name in ("gate-b.md", "gate-c.md", "GATE-B.MD", "report.txt"):
                with self.subTest(name=name), self.assertRaises(self.probe.ProbeError):
                    self.probe.write_evidence(Path(tmp) / name, report, token)

    def test_evidence_redaction_subprocess_has_a_deadline(self):
        with tempfile.TemporaryDirectory() as tmp:
            evidence = Path(tmp) / "candidate-probes.json"
            evidence.write_text("{}", encoding="ascii")
            expired = self.probe.subprocess.TimeoutExpired("cargo", 60)
            with mock.patch.object(self.probe.subprocess, "run", side_effect=expired):
                with self.assertRaises(self.probe.ProbeError):
                    self.probe._verify_evidence_redaction([evidence])

    def test_evidence_redaction_subprocess_removes_runtime_token_environment(self):
        token_value = "verifier-environment-token-0123456789abcdef"
        with tempfile.TemporaryDirectory() as tmp:
            evidence = Path(tmp) / "candidate-probes.json"
            evidence.write_text("{}", encoding="ascii")
            completed = self.probe.subprocess.CompletedProcess(
                ["cargo", "run"], 0, "", ""
            )
            with mock.patch.dict(
                self.probe.os.environ,
                {self.probe.TOKEN_ENVIRONMENT: token_value},
            ), mock.patch.object(
                self.probe.subprocess, "run", return_value=completed
            ) as run:
                self.probe._verify_evidence_redaction([evidence])

        child_environment = run.call_args.kwargs["env"]
        self.assertNotIn(self.probe.TOKEN_ENVIRONMENT, child_environment)
        self.assertNotIn(token_value, child_environment.values())

    def test_mac_adapter_source_uses_manifest_and_keeps_token_out_of_argv(self):
        source = MODULE_PATH.read_text(encoding="utf-8")
        self.assertIn("ProcessControl", source)
        self.assertIn("environment=environment", source)
        self.assertIn('add_argument("--manifest"', source)
        self.assertIn("candidate IPA changed after manifest attestation", source)
        self.assertIn("candidate PID changed while moving agent to foreground", source)
        self.assertNotIn('add_argument("--bundle-id"', source)
        self.assertNotIn('add_argument("--token"', source)
        self.assertNotIn("RIVIU_AGENT_TOKEN=", source)
        self.assertIn('"output": args.output.name', source)
        self.assertNotIn('"output": str(args.output)', source)

    def test_installed_team_id_is_derived_from_device_metadata(self):
        self.assertEqual(
            "ABCDE12345",
            self.probe.installed_team_id(
                {"ApplicationIdentifierEntitlement": "ABCDE12345.com.example.agent"}
            ),
        )
        self.assertEqual(
            "ABCDE12345",
            self.probe.installed_team_id(
                {"Entitlements": {"com.apple.developer.team-identifier": "ABCDE12345"}}
            ),
        )
        with self.assertRaises(self.probe.ProbeError):
            self.probe.installed_team_id(
                {
                    "TeamIdentifier": "ABCDE12345",
                    "ApplicationIdentifierEntitlement": "FGHIJ67890.com.example.agent",
                }
            )

    def test_status_identity_requires_the_complete_ready_candidate_identity(self):
        runner = object.__new__(self.probe.ProbeRunner)
        complete = self.probe.HttpResult(
            200,
            {
                "value": {
                    "riviuAgent": {
                        "agentVersion": "0.1.0",
                        "protocolVersion": 2,
                        "features": ["stream", "tap", "swipe", "clipboard"],
                        "logicalWidth": 375,
                        "logicalHeight": 667,
                        "state": "ready",
                    }
                }
            },
        )
        runner._validate_status_identity(complete)
        invalid = copy.deepcopy(complete.payload)
        invalid["value"]["riviuAgent"]["protocolVersion"] = 1
        with self.assertRaises(self.probe.ProbeError):
            runner._validate_status_identity(self.probe.HttpResult(200, invalid))

    def test_mac_foreground_candidate_requires_a_stable_existing_pid(self):
        class FakeProcessControl:
            def __init__(self, pids):
                self.pids = iter(pids)

            async def process_identifier_for_bundle_identifier(self, _bundle):
                return next(self.pids)

            async def launch(self, _bundle, *, kill_existing, environment):
                self.assertions = (kill_existing, environment)
                return 42

        adapter = object.__new__(self.probe.MacDeviceAdapter)
        adapter.udid = "fixture-device"
        adapter.candidate_bundle = "com.example.agent"

        stable = FakeProcessControl([42, 42])

        async def with_stable(_udid, operation):
            return await operation(stable)

        with mock.patch.object(self.probe, "_with_process_control", with_stable):
            self.assertEqual(42, adapter.foreground_candidate_without_restart())
        self.assertEqual((False, {}), stable.assertions)

        changed = FakeProcessControl([42, 43])

        async def with_changed(_udid, operation):
            return await operation(changed)

        with mock.patch.object(self.probe, "_with_process_control", with_changed):
            with self.assertRaises(self.probe.ProbeError):
                adapter.foreground_candidate_without_restart()

        target = FakeProcessControl([])

        async def with_target(_udid, operation):
            return await operation(target)

        with mock.patch.object(self.probe, "_with_process_control", with_target):
            adapter.foreground("com.apple.Preferences")
        self.assertEqual((True, {}), target.assertions)

    def test_async_device_operations_have_a_real_deadline(self):
        async def hangs():
            await asyncio.sleep(1.0)

        started = time.monotonic()
        with self.assertRaises(self.probe.ProbeError):
            self.probe.run_async_bounded(hangs(), 0.02, "fixture operation")
        self.assertLess(time.monotonic() - started, 0.5)

    def test_device_port_probe_only_treats_connection_refused_as_closed(self):
        from pymobiledevice3.exceptions import ConnectionFailedError

        class FakeDevice:
            def __init__(self, error):
                self.error = error

            async def connect(self, _port):
                raise self.error

        async def closed_device(_udid):
            return FakeDevice(ConnectionFailedError("fixture closed"))

        async def broken_device(_udid):
            return FakeDevice(RuntimeError("fixture usb failure"))

        with mock.patch("pymobiledevice3.usbmux.select_device", closed_device):
            self.assertFalse(asyncio.run(self.probe._device_port_is_open("fixture", 8916)))
        with mock.patch("pymobiledevice3.usbmux.select_device", broken_device):
            with self.assertRaises(RuntimeError):
                asyncio.run(self.probe._device_port_is_open("fixture", 8916))


if __name__ == "__main__":
    unittest.main()
