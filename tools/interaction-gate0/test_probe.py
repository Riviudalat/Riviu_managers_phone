import base64
import contextlib
import hashlib
import importlib.util
import io
import json
import os
import plistlib
import socket
import sys
import tempfile
import threading
import time
import unittest
import urllib.request
import warnings
import zipfile
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from types import SimpleNamespace
from unittest import mock

from PIL import Image


HERE = Path(__file__).resolve().parent
SPEC = importlib.util.spec_from_file_location("interaction_gate0_probe", HERE / "probe.py")
probe = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
sys.modules[SPEC.name] = probe
SPEC.loader.exec_module(probe)


TOKEN = "fixture-token-0123456789-abcdefghijk"
UDID = "00008020-001C2D3E4F50002E"
HEADER = "X-Fixture-Token"


def generated_jpeg(width=750, height=1334, color=(21, 46, 77)):
    image = Image.new("RGB", (width, height), color)
    stream = io.BytesIO()
    image.save(stream, "JPEG", quality=70)
    return stream.getvalue()


def generated_rail_jpeg():
    image = Image.new("RGB", (750, 1334), (20, 30, 40))
    for logical_y in (312, 377, 443, 511):
        pixel_y = logical_y * 2
        for y in range(pixel_y - 10, pixel_y + 11):
            for x in range(675, 705):
                image.putpixel((x, y), (245, 245, 245))
    stream = io.BytesIO()
    image.save(stream, "JPEG", quality=95)
    return stream.getvalue()


class FixtureState:
    def __init__(self):
        self.events = []
        self.requests = []
        self.jpeg = generated_jpeg()
        self.jpegs = [
            generated_jpeg(),
            generated_jpeg(color=(33, 57, 81)),
            generated_jpeg(color=(45, 68, 92)),
        ]
        self.clipboard = b"prior-fixture"
        self.keep_streaming = threading.Event()
        self.keep_streaming.set()


class FixtureHandler(BaseHTTPRequestHandler):
    server_version = "Gate0Fixture/1"

    def log_message(self, _format, *_args):
        pass

    def _authorized(self):
        return self.headers.get(HEADER) == TOKEN

    def _json(self, status, payload):
        body = json.dumps(payload, separators=(",", ":")).encode("ascii")
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self):
        state = self.server.fixture_state
        if self.path == "/status":
            self._json(200, {"value": {"sessionId": "fixture-session"}})
            return
        if not self._authorized():
            self._json(401, {"value": {"error": "unauthorized"}})
            return
        if self.path == "/protected":
            self._json(200, {"value": {"locked": False}})
            return
        if self.path == "/wda/locked":
            self._json(200, {"value": False})
            return
        if self.path == "/wda/activeAppInfo":
            self._json(
                200,
                {"value": {"bundleId": "com.ss.iphone.ugc.Ame", "pid": 4142}},
            )
            return
        if self.path == "/wda/deviceOrientation":
            self._json(200, {"value": "UIDeviceOrientationPortrait"})
            return
        if self.path == "/mjpeg":
            state.events.append("mjpeg-connect")
            boundary = b"--fixture"
            parts = []
            for jpeg in state.jpegs:
                parts.append(
                    boundary
                    + b"\r\nContent-Type: image/jpeg\r\nContent-Length: "
                    + str(len(jpeg)).encode("ascii")
                    + b"\r\n\r\n"
                    + jpeg
                    + b"\r\n"
                )
            part = b"".join(parts) + boundary + b"--\r\n"
            self.send_response(200)
            self.send_header("Content-Type", "multipart/x-mixed-replace; boundary=fixture")
            self.send_header("Content-Length", str(len(part)))
            self.end_headers()
            self.wfile.write(part)
            return
        if self.path == "/mjpeg-live":
            self.send_response(200)
            self.send_header("Content-Type", "multipart/x-mixed-replace; boundary=fixture")
            self.end_headers()
            self.wfile.write(b"--fixture\r\nContent-Type: image/jpeg\r\n\r\n")
            self.wfile.flush()
            time.sleep(1.5)
            return
        if self.path == "/mjpeg-owned":
            self.send_response(200)
            self.send_header("Content-Type", "multipart/x-mixed-replace; boundary=fixture")
            self.end_headers()
            try:
                ordinal = 0
                while state.keep_streaming.is_set():
                    jpeg = state.jpegs[ordinal % len(state.jpegs)]
                    ordinal += 1
                    self.wfile.write(
                        b"--fixture\r\nContent-Type: image/jpeg\r\n\r\n"
                        + jpeg
                        + b"\r\n"
                    )
                    self.wfile.flush()
                    time.sleep(0.03)
            except (BrokenPipeError, ConnectionAbortedError, ConnectionResetError):
                pass
            return
        self._json(404, {"value": {"error": "unknown"}})

    def do_POST(self):
        state = self.server.fixture_state
        length = int(self.headers.get("Content-Length", "0"))
        raw_payload = self.rfile.read(length)
        if not self._authorized():
            self._json(401, {"value": {"error": "unauthorized"}})
            return
        payload = json.loads(raw_payload or b"{}")
        state.requests.append((self.path, payload))
        if self.path == "/session":
            state.events.append("session")
            self._json(200, {"sessionId": "fixture-session", "value": {}})
            return
        if self.path == "/url":
            self._json(200, {"value": None})
            return
        if self.path == "/wda/setPasteboard":
            state.clipboard = base64.b64decode(payload["content"], validate=True)
            self._json(200, {"value": None})
            return
        if self.path == "/wda/getPasteboard":
            self._json(
                200,
                {"value": base64.b64encode(state.clipboard).decode("ascii")},
            )
            return
        if self.path == "/wda/swipe":
            self._json(200, {"value": None})
            return
        self._json(404, {"value": {"error": "unknown"}})


class FixtureServer:
    def __init__(self):
        self.state = FixtureState()
        self.server = ThreadingHTTPServer(("127.0.0.1", 0), FixtureHandler)
        self.server.fixture_state = self.state
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)

    @property
    def base_url(self):
        return f"http://127.0.0.1:{self.server.server_port}"

    @property
    def port(self):
        return self.server.server_port

    def start(self):
        self.thread.start()
        return self

    def close(self):
        self.state.keep_streaming.clear()
        self.server.shutdown()
        self.server.server_close()
        self.thread.join(timeout=2)


class ProbeFixtureTests(unittest.TestCase):
    def setUp(self):
        self.fixture = FixtureServer().start()

    def tearDown(self):
        self.fixture.close()

    def test_protected_control_and_mjpeg_auth_are_401_401_200(self):
        with warnings.catch_warnings(record=True) as caught:
            warnings.simplefilter("always", ResourceWarning)
            for path in ("/protected", "/mjpeg"):
                statuses = probe.prove_auth_triplet(
                    self.fixture.base_url + path, HEADER, TOKEN, timeout_seconds=1.0
                )
                self.assertEqual((401, 401, 200), statuses)
        self.assertEqual([], [item for item in caught if item.category is ResourceWarning])

    def test_live_mjpeg_auth_checks_status_without_waiting_for_stream_eof(self):
        started = time.monotonic()
        statuses = probe.prove_stream_auth_triplet(
            self.fixture.base_url + "/mjpeg-live",
            HEADER,
            TOKEN,
            timeout_seconds=1.0,
        )
        self.assertEqual((401, 401, 200), statuses)
        self.assertLess(time.monotonic() - started, 1.0)

    def test_open_url_bodies_are_exact_for_direct_short_and_photo(self):
        urls = (
            "https://www.tiktok.com/@fixture/video/7411111111111111111",
            "https://vt.tiktok.com/fixture/",
            "https://www.tiktok.com/@fixture/photo/7422222222222222222",
        )
        for url in urls:
            probe.post_open_url(
                self.fixture.base_url,
                "/url",
                HEADER,
                TOKEN,
                url,
                "com.ss.iphone.ugc.Ame",
                timeout_seconds=1.0,
            )
        self.assertEqual(
            [
                (
                    "/url",
                    {"url": url, "bundleId": "com.ss.iphone.ugc.Ame", "idleTimeoutMs": 0},
                )
                for url in urls
            ],
            self.fixture.state.requests,
        )

    def test_short_link_resolution_never_leaves_https_tiktok_hosts(self):
        with mock.patch.object(urllib.request, "build_opener") as build_opener:
            with self.assertRaisesRegex(probe.ProbeError, "TargetUrlUnsupported"):
                probe._resolve_tiktok_post("https://example.invalid/redirect")
        build_opener.assert_not_called()

        redirects = probe._BoundedRedirect(maximum=2)
        self.assertEqual(
            "https://www.tiktok.com/@fixture/video/7411111111111111111",
            redirects._next_url(
                "https://vt.tiktok.com/fixture/",
                "https://www.tiktok.com/@fixture/video/7411111111111111111",
            ),
        )
        with self.assertRaisesRegex(probe.ProbeError, "TargetUrlUnsupported"):
            redirects._next_url(
                "https://www.tiktok.com/@fixture/video/7411111111111111111",
                "https://example.invalid/collect",
            )

        bounded = probe._BoundedRedirect(maximum=1)
        bounded._next_url("https://vt.tiktok.com/a/", "https://vt.tiktok.com/b/")
        with self.assertRaisesRegex(probe.ProbeError, "TargetResolveRedirectLimit"):
            bounded._next_url("https://vt.tiktok.com/b/", "https://vt.tiktok.com/c/")

    def test_each_interaction_route_proves_missing_wrong_and_correct_auth(self):
        get_statuses = probe.prove_route_auth_triplet(
            "GET",
            self.fixture.base_url + "/wda/activeAppInfo",
            HEADER,
            TOKEN,
            None,
            timeout_seconds=1.0,
        )
        set_statuses = probe.prove_route_auth_triplet(
            "POST",
            self.fixture.base_url + "/wda/setPasteboard",
            HEADER,
            TOKEN,
            {
                "content": base64.b64encode(b"route-auth").decode("ascii"),
                "contentType": "plaintext",
            },
            timeout_seconds=1.0,
        )
        self.assertEqual((401, 401, 200), get_statuses)
        self.assertEqual((401, 401, 200), set_statuses)

    def test_unauthorized_posts_return_401_after_the_request_body_is_received(self):
        body = {"value": "x" * 65_536}
        for _ in range(40):
            status, _ = probe._request(
                "POST",
                self.fixture.base_url + "/session",
                HEADER,
                None,
                body,
                timeout_seconds=1.0,
            )
            self.assertEqual(401, status)

    def test_mjpeg_health_requires_multiple_frames_on_one_connection(self):
        frames = probe.read_mjpeg_sequence(
            self.fixture.base_url + "/mjpeg",
            HEADER,
            TOKEN,
            minimum_frames=3,
            timeout_seconds=1.0,
        )
        self.assertEqual(3, len(frames))
        self.assertGreater(len({hashlib.sha256(frame).digest() for frame in frames}), 1)

    def test_owned_mjpeg_reader_binds_frames_to_one_generation(self):
        reader = probe.MjpegReader(
            self.fixture.base_url + "/mjpeg-owned", HEADER, TOKEN, generation=9
        )
        try:
            batch = reader.start(minimum_frames=3, timeout_seconds=1.0)
            self.assertEqual(9, batch.generation)
            self.assertEqual(1, batch.start_sequence)
            self.assertEqual(3, batch.end_sequence)
            self.assertGreater(
                len({hashlib.sha256(frame).digest() for frame in batch.frames}), 1
            )
        finally:
            reader.stop()

    def test_owned_mjpeg_reader_rejects_a_bounded_or_ended_response(self):
        reader = probe.MjpegReader(
            self.fixture.base_url + "/mjpeg", HEADER, TOKEN, generation=9
        )
        try:
            with self.assertRaisesRegex(probe.ProbeError, "MjpegReaderFailed"):
                reader.start(minimum_frames=3, timeout_seconds=1.0)
        finally:
            reader.stop()

    def test_owned_mjpeg_reader_rejects_stale_buffered_frames(self):
        reader = probe.MjpegReader(
            self.fixture.base_url + "/mjpeg-owned", HEADER, TOKEN, generation=9
        )
        stale_at = time.monotonic() - probe.MAX_STREAM_GAP_SECONDS - 1.0
        with reader._condition:
            reader._frames = [
                (1, stale_at, generated_jpeg(color=(10, 20, 30))),
                (2, stale_at + 0.02, generated_jpeg(color=(20, 30, 40))),
                (3, stale_at + 0.04, generated_jpeg(color=(30, 40, 50))),
            ]
            reader._sequence = 3

        with self.assertRaisesRegex(probe.ProbeError, "MjpegInterFrameGapExceeded"):
            reader.wait_for_frames(0, minimum_frames=3, timeout_seconds=0.1)

    def test_session_is_created_before_mjpeg_and_first_decoded_frame(self):
        result = probe.start_session_then_mjpeg(
            self.fixture.base_url,
            self.fixture.base_url + "/mjpeg",
            HEADER,
            TOKEN,
            timeout_seconds=1.0,
        )
        self.assertEqual(["session", "mjpeg-connect"], self.fixture.state.events)
        self.assertEqual(["session", "mjpeg-connect", "first-frame"], result.events)
        self.assertEqual((750, 1334), result.frame_size)
        self.assertEqual(hashlib.sha256(self.fixture.state.jpeg).hexdigest(), result.frame_sha256)

    def test_clipboard_decode_accepts_64k_and_rejects_one_byte_more(self):
        accepted = probe.decode_bounded_clipboard(base64.b64encode(b"a" * 65536).decode("ascii"))
        self.assertEqual(65536, len(accepted))
        with self.assertRaisesRegex(probe.ProbeError, "ClipboardTooLarge"):
            probe.decode_bounded_clipboard(base64.b64encode(b"a" * 65537).decode("ascii"))

        probe.require_controlled_clipboard_fixture(probe.CLIPBOARD_FIXTURE_VALUE)
        with self.assertRaisesRegex(probe.ProbeError, "ClipboardFixtureMissing"):
            probe.require_controlled_clipboard_fixture(b"personal-clipboard")

    def test_background_safe_clipboard_mode_never_foregrounds_agent(self):
        self.assertEqual(
            ["verify-tiktok", "clipboard-set", "clipboard-read", "verify-tiktok"],
            probe.clipboard_access_sequence("targetBackgroundSafe"),
        )

    def test_agent_foreground_clipboard_mode_rebuilds_session_before_stream(self):
        sequence = probe.clipboard_access_sequence("agentForegroundRequired")
        self.assertEqual(
            [
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
            ],
            sequence,
        )
        self.assertLess(sequence.index("session"), sequence.index("mjpeg-connect"))

    def test_active_bundle_and_pid_must_both_be_stable(self):
        before = {"bundleId": "com.ss.iphone.ugc.Ame", "pid": 4142}
        after = {"bundleId": "com.ss.iphone.ugc.Ame", "pid": 4142}
        proof = probe.prove_stable_app_identity(before, after, "com.ss.iphone.ugc.Ame")
        self.assertEqual(4142, proof.pid)
        with self.assertRaisesRegex(probe.ProbeError, "ActivePidMismatch"):
            probe.prove_stable_app_identity(before, {**after, "pid": 4143}, before["bundleId"])
        with self.assertRaisesRegex(probe.ProbeError, "ActiveBundleMismatch"):
            probe.prove_stable_app_identity(before, after, "com.apple.mobilesafari")

    def test_geometry_rejects_orientation_bounds_and_scale_mismatch(self):
        good = probe.GeometryEvidence(375, 667, 750, 1334, "portrait")
        self.assertEqual((2.0, 2.0), probe.validate_geometry(good))
        cases = (
            (probe.GeometryEvidence(375, 667, 750, 1334, "landscape"), "UnsupportedOrientation"),
            (probe.GeometryEvidence(390, 844, 780, 1688, "portrait"), "UnsupportedBounds"),
            (probe.GeometryEvidence(375, 667, 751, 1334, "portrait"), "UnsupportedScale"),
        )
        for evidence, code in cases:
            with self.subTest(code=code), self.assertRaisesRegex(probe.ProbeError, code):
                probe.validate_geometry(evidence)

    def test_mobilegestalt_geometry_uses_device_metrics_without_wda_hierarchy(self):
        evidence = probe.geometry_from_mobilegestalt(
            {
                "main-screen-width": 750,
                "main-screen-height": 1334,
                "main-screen-scale": 2,
            },
            "UIDeviceOrientationPortrait",
            (750, 1334),
        )
        self.assertEqual(probe.GeometryEvidence(375, 667, 750, 1334, "portrait"), evidence)
        with self.assertRaisesRegex(probe.ProbeError, "UnsupportedOrientation"):
            probe.geometry_from_mobilegestalt(
                {
                    "main-screen-width": 750,
                    "main-screen-height": 1334,
                    "main-screen-scale": 2,
                },
                "UIDeviceOrientationLandscapeLeft",
                (750, 1334),
            )

    def test_share_detector_requires_a_white_rail_chain(self):
        x, y = probe.find_share_control(generated_rail_jpeg())
        self.assertAlmostEqual(0.919, x, places=3)
        self.assertAlmostEqual(511 / 667, y, places=2)
        with self.assertRaisesRegex(probe.ProbeError, "ShareControlNotFound"):
            probe.find_share_control(generated_jpeg())

    def test_share_sheet_cleanup_requires_the_feed_rail_to_return(self):
        adapter = probe.MacGateAdapter(
            UDID,
            TOKEN,
            "com.fixture.agent",
            "com.ss.iphone.ugc.Ame",
            probe.EXPECTED_LOGICAL_BOUNDS,
        )
        adapter.generation = 1
        adapter.share_sheet_open = True
        adapter.latest_frame = generated_rail_jpeg()
        adapter._geometry = probe.GeometryEvidence(375, 667, 750, 1334, "portrait")
        modal_batch = probe.StreamBatch(1, 11, 13, [generated_jpeg()] * 3, 0.03)
        feed_batch = probe.StreamBatch(1, 21, 23, [generated_rail_jpeg()] * 3, 0.03)
        with mock.patch.object(adapter, "stream_boundary", return_value=10), mock.patch.object(
            adapter, "_native_tap_correct", return_value=20
        ) as dismiss, mock.patch.object(
            adapter, "wait_stream_frames", side_effect=[modal_batch, feed_batch]
        ) as wait:
            adapter.dismiss_share_sheet_if_needed()
        self.assertFalse(adapter.share_sheet_open)
        self.assertIn("share-sheet-dismissed", adapter.trace)
        dismiss.assert_called_once()
        self.assertEqual(2, wait.call_count)

    def test_ambiguous_share_request_is_cleaned_as_possibly_executed(self):
        adapter = probe.MacGateAdapter(
            UDID,
            TOKEN,
            "com.fixture.agent",
            "com.ss.iphone.ugc.Ame",
            probe.EXPECTED_LOGICAL_BOUNDS,
        )
        adapter.generation = 1
        adapter.session_id = "fixture-session"
        adapter.latest_frame = generated_rail_jpeg()
        geometry = probe.GeometryEvidence(375, 667, 750, 1334, "portrait")
        with mock.patch.object(adapter, "geometry", return_value=geometry), mock.patch.object(
            adapter, "_native_tap", side_effect=probe.ProbeError("ControlRouteTimeout")
        ):
            with self.assertRaisesRegex(probe.ProbeError, "ControlRouteTimeout"):
                adapter.tap_share_and_copy_link_once()
        self.assertTrue(adapter.share_sheet_open)

    def test_final_health_samples_its_boundary_after_identity_and_geometry(self):
        adapter = probe.MacGateAdapter(
            UDID,
            TOKEN,
            "com.fixture.agent",
            "com.ss.iphone.ugc.Ame",
            probe.EXPECTED_LOGICAL_BOUNDS,
        )
        adapter.generation = 1
        adapter.session_id = "fixture-session"
        adapter.latest_frame = generated_rail_jpeg()
        final_batch = probe.StreamBatch(
            1,
            41,
            43,
            [
                generated_rail_jpeg(),
                generated_jpeg(color=(31, 41, 51)),
                generated_rail_jpeg(),
            ],
            0.03,
        )
        call_order = []

        def identity(_pid):
            call_order.append("identity")

        def geometry(_frame):
            call_order.append("geometry")
            return probe.GeometryEvidence(375, 667, 750, 1334, "portrait")

        def boundary():
            call_order.append("boundary")
            return 40

        def wait(after_sequence):
            call_order.append(f"wait:{after_sequence}")
            return final_batch

        with mock.patch.object(
            adapter, "_status_session_id", return_value="fixture-session"
        ), mock.patch.object(
            adapter, "prove_tiktok_identity", side_effect=identity
        ), mock.patch.object(
            adapter, "geometry", side_effect=geometry
        ), mock.patch.object(
            adapter, "stream_boundary", side_effect=boundary
        ), mock.patch.object(
            adapter, "wait_stream_frames", side_effect=wait
        ):
            self.assertEqual(final_batch, adapter.require_healthy(1234))

        self.assertEqual(
            ["identity", "geometry", "boundary", "wait:40", "geometry"],
            call_order,
        )

    def test_copy_link_ocr_requires_a_confident_normalized_label(self):
        result = probe.select_copy_link_observation(
            [
                {"text": "Send to", "confidence": 0.99, "x": 0.1, "y": 0.6, "width": 0.2, "height": 0.05},
                {"text": "Copy Link", "confidence": 0.91, "x": 0.61, "y": 0.71, "width": 0.22, "height": 0.06},
            ]
        )
        self.assertAlmostEqual(0.72, result[0])
        self.assertAlmostEqual(0.74, result[1])
        with self.assertRaisesRegex(probe.ProbeError, "CopyLinkControlNotFound"):
            probe.select_copy_link_observation(
                [{"text": "Copy Link", "confidence": 0.2, "x": 0.6, "y": 0.7, "width": 0.2, "height": 0.05}]
            )

    def test_detector_attestation_is_derived_from_both_source_files(self):
        contract = probe.detector_attestation()
        self.assertEqual("rail-white-chain-v1", contract["shareDetectorVersion"])
        self.assertEqual("macos-vision-text-v1", contract["copyLinkDetectorVersion"])
        for field in ("probeSha256", "visionHelperSha256", "detectorSetSha256"):
            self.assertRegex(contract[field], r"^[0-9a-f]{64}$")

    def test_live_dependency_tuple_requires_the_exact_mac_lock_versions(self):
        versions = dict(probe.EXPECTED_LIVE_DEPENDENCIES)
        attestation = probe.live_dependency_attestation(versions.__getitem__)
        self.assertEqual("10.1.0", attestation["pymobiledevice3Version"])
        self.assertEqual("11.3.0", attestation["pillowVersion"])
        self.assertRegex(attestation["requirementsMacSha256"], r"^[0-9a-f]{64}$")

        versions["Pillow"] = "11.3.1"
        with self.assertRaisesRegex(probe.ProbeError, "LiveDependencyVersionMismatch"):
            probe.live_dependency_attestation(versions.__getitem__)

    def test_probe_source_has_no_tiktok_hierarchy_or_session_window_routes(self):
        source = (HERE / "probe.py").read_text(encoding="utf-8")
        for forbidden in (
            "/session/{self.session_id}/element",
            "/session/{self.session_id}/window/size",
            "/session/{self.session_id}/orientation",
            'f"/session/{self.session_id}"',
        ):
            self.assertNotIn(forbidden, source)

    def test_mac_adapter_uses_protected_sessionless_geometry_and_native_tap(self):
        adapter = probe.MacGateAdapter(
            UDID,
            TOKEN,
            "com.fixture.agent",
            "com.ss.iphone.ugc.Ame",
            probe.EXPECTED_LOGICAL_BOUNDS,
        )
        adapter.TOKEN_HEADER = HEADER
        adapter.control_relay = SimpleNamespace(local_port=self.fixture.port)
        adapter.generation = 1
        adapter.mjpeg_reader = SimpleNamespace(generation=1, sequence=23)
        self.assertEqual("fixture-session", adapter.create_session())
        self.assertEqual(False, adapter._control_with_auth("GET", "/wda/locked", None, "locked"))
        self.assertEqual("fixture-session", adapter._status_session_id())
        identity = adapter.active_identity(prove_auth=True)
        self.assertEqual(4142, identity["pid"])
        with mock.patch.object(
            probe,
            "_device_display_metrics",
            new=mock.AsyncMock(
                return_value={
                "main-screen-width": 750,
                "main-screen-height": 1334,
                "main-screen-scale": 2,
                }
            ),
        ):
            geometry = adapter.geometry(generated_jpeg(), prove_auth=True)
        self.assertEqual(
            23,
            adapter._native_tap(0.919, 511 / 667, geometry, "nativeSwipeFixture"),
        )
        adapter.mjpeg_reader.sequence = 31
        self.assertEqual(
            31,
            adapter.open_url(
                "https://www.tiktok.com/@fixture/video/7411111111111111111"
            ),
        )
        self.assertTrue(adapter.route_auth)
        self.assertTrue(
            all(statuses == [401, 401, 200] for statuses in adapter.route_auth.values())
        )
        for edge in ((1.0, 0.5), (0.5, 1.0)):
            with self.assertRaisesRegex(probe.ProbeError, "TapCoordinateInvalid"):
                adapter._native_tap(*edge, geometry, "nativeSwipeEdge")

    def test_cleanup_steps_continue_after_an_earlier_failure(self):
        events = []

        def failed():
            events.append("relay")
            raise RuntimeError("fixture")

        def completed():
            events.append("terminate")

        with self.assertRaisesRegex(probe.ProbeError, "CleanupFailed"):
            probe.run_cleanup_steps((failed, completed))
        self.assertEqual(["relay", "terminate"], events)

    def test_cleanup_closes_server_port(self):
        extra = FixtureServer().start()
        port = extra.port
        probe.cleanup_and_verify_ports([extra], [("127.0.0.1", port)], timeout_seconds=1.0)
        with socket.socket() as sock:
            self.assertNotEqual(0, sock.connect_ex(("127.0.0.1", port)))

    def test_artifact_hash_and_manifest_hash_are_derived(self):
        with tempfile.TemporaryDirectory() as raw:
            root = Path(raw)
            ipa = root / "agent.ipa"
            ipa.write_bytes(b"fixture-ipa")
            manifest = root / "agent-manifest.json"
            manifest.write_text(
                json.dumps({"sha256": hashlib.sha256(ipa.read_bytes()).hexdigest()}),
                encoding="ascii",
            )
            proof = probe.verify_artifact_manifest(ipa, manifest)
            self.assertEqual(hashlib.sha256(ipa.read_bytes()).hexdigest(), proof.ipa_sha256)
            self.assertEqual(hashlib.sha256(manifest.read_bytes()).hexdigest(), proof.manifest_sha256)

    def test_ipa_executable_identity_is_read_from_the_hashed_payload(self):
        with tempfile.TemporaryDirectory() as raw:
            root = Path(raw)
            ipa = root / "agent.ipa"
            info = {
                "CFBundleIdentifier": "com.fixture.agent",
                "CFBundleShortVersionString": "1.2",
                "CFBundleVersion": "34",
                "CFBundleExecutable": "FixtureRunner",
            }
            with zipfile.ZipFile(ipa, "w") as archive:
                archive.writestr(
                    "Payload/Fixture.app/Info.plist",
                    plistlib.dumps(info, fmt=plistlib.FMT_BINARY),
                )
            identity = probe.inspect_ipa_identity(
                ipa,
                {
                    "payloadApp": "Fixture.app",
                    "bundleId": "com.fixture.agent",
                    "bundleVersion": "1.2",
                    "bundleBuild": "34",
                },
            )
            self.assertEqual("FixtureRunner", identity.executable_name)
            with self.assertRaisesRegex(probe.ProbeError, "ArtifactInfoPlistIdentityMismatch"):
                probe.inspect_ipa_identity(
                    ipa,
                    {
                        "payloadApp": "Fixture.app",
                        "bundleId": "com.fixture.other",
                        "bundleVersion": "1.2",
                        "bundleBuild": "34",
                    },
                )

    def test_fresh_install_identity_must_match_the_hashed_ipa_payload(self):
        identity = probe.IpaIdentity(
            "Fixture.app", "com.fixture.agent", "1.2", "34", "FixtureRunner"
        )
        installed = {
            "Path": "/private/containers/Bundle/Application/id/Fixture.app",
            "CFBundleIdentifier": "com.fixture.agent",
            "CFBundleShortVersionString": "1.2",
            "CFBundleVersion": "34",
            "CFBundleExecutable": "FixtureRunner",
            "SignerIdentity": "Fixture Signer",
            "ApplicationType": "User",
        }
        proof = probe._validate_fresh_install_identity(
            installed, identity, {"signerIdentity": "Fixture Signer"}
        )
        self.assertTrue(proof["freshInstall"])
        self.assertEqual("Fixture.app", proof["payloadApp"])

        installed["CFBundleExecutable"] = "DifferentRunner"
        with self.assertRaisesRegex(probe.ProbeError, "FreshInstallIdentityMismatch"):
            probe._validate_fresh_install_identity(
                installed, identity, {"signerIdentity": "Fixture Signer"}
            )

    def test_redaction_rejects_token_udid_and_decoded_json_leaves(self):
        for raw in (
            TOKEN.encode("ascii"),
            UDID.encode("ascii"),
            json.dumps({"nested": TOKEN}).encode("ascii"),
            json.dumps({"nested": UDID}).encode("ascii"),
        ):
            with self.subTest(raw=raw[:12]), self.assertRaisesRegex(
                probe.ProbeError, "RedactionViolation"
            ):
                probe.verify_redaction([raw], [TOKEN, UDID])

    def test_report_publication_rolls_back_both_files_on_second_replace_failure(self):
        with tempfile.TemporaryDirectory() as raw:
            root = Path(raw)
            json_path = root / "gate-0.json"
            md_path = root / "gate-0.md"
            json_path.write_bytes(b"old-json")
            md_path.write_bytes(b"old-md")
            calls = 0

            def fail_second(source, destination):
                nonlocal calls
                calls += 1
                if calls == 2:
                    raise OSError("replace fixture failure")
                os.replace(source, destination)

            report = probe.fixture_report({"ipaSha256": "a" * 64})
            with self.assertRaisesRegex(probe.ProbeError, "ReportPublishFailed"):
                probe.publish_reports(report, "fixture markdown\n", root, [TOKEN, UDID], fail_second)
            self.assertEqual(b"old-json", json_path.read_bytes())
            self.assertEqual(b"old-md", md_path.read_bytes())

    def test_report_publication_recovers_an_interrupted_journal(self):
        with tempfile.TemporaryDirectory() as raw:
            root = Path(raw)
            (root / "gate-0.json").write_bytes(b"partial-new-json")
            (root / "gate-0.md").write_bytes(b"old-md")
            transaction = root / ".gate0-publish-fixture"
            transaction.mkdir()
            (transaction / "prior-0").write_bytes(b"old-json")
            (transaction / "prior-1").write_bytes(b"old-md")
            journal = {
                "schemaVersion": 1,
                "transaction": transaction.name,
                "hadPrior": [True, True],
                "priorSha256": [
                    hashlib.sha256(b"old-json").hexdigest(),
                    hashlib.sha256(b"old-md").hexdigest(),
                ],
                "stagedSha256": [
                    hashlib.sha256(b"partial-new-json").hexdigest(),
                    hashlib.sha256(b"new-md").hexdigest(),
                ],
                "state": "replacing",
            }
            (root / ".gate0-publish-journal.json").write_text(
                json.dumps(journal), encoding="ascii"
            )
            probe.recover_report_publication(root)
            self.assertEqual(b"old-json", (root / "gate-0.json").read_bytes())
            self.assertEqual(b"old-md", (root / "gate-0.md").read_bytes())
            self.assertFalse((root / ".gate0-publish-journal.json").exists())

    def test_report_publication_recovers_after_process_death_between_replaces(self):
        class SimulatedProcessDeath(BaseException):
            pass

        with tempfile.TemporaryDirectory() as raw:
            root = Path(raw)
            (root / "gate-0.json").write_bytes(b"old-json")
            (root / "gate-0.md").write_bytes(b"old-md")

            def checkpoint(label):
                if label == "json-replaced":
                    raise SimulatedProcessDeath()

            report = probe.fixture_report({"ipaSha256": "a" * 64})
            with self.assertRaises(SimulatedProcessDeath):
                probe.publish_reports(
                    report,
                    "fixture markdown\n",
                    root,
                    [TOKEN, UDID],
                    checkpoint=checkpoint,
                )
            probe.recover_report_publication(root)
            self.assertEqual(b"old-json", (root / "gate-0.json").read_bytes())
            self.assertEqual(b"old-md", (root / "gate-0.md").read_bytes())

    def test_fixture_report_never_emits_pass_or_a_qualification(self):
        report = probe.fixture_report({"ipaSha256": "a" * 64})
        encoded = json.dumps(report, sort_keys=True)
        self.assertEqual("FIXTURE_ONLY", report["environment"])
        self.assertEqual("FIXTURE_ONLY", report["gateStatus"])
        self.assertFalse(report["productionQualified"])
        self.assertEqual([], report["qualifications"])
        self.assertNotIn('"PASS"', encoded)

    def test_windows_cli_publishes_only_hashes_and_has_no_bypass_flags(self):
        with tempfile.TemporaryDirectory() as raw:
            root = Path(raw)
            ipa = root / "agent.ipa"
            ipa.write_bytes(b"fixture-ipa")
            manifest = root / "agent-manifest.json"
            manifest.write_text(
                json.dumps({"sha256": hashlib.sha256(ipa.read_bytes()).hexdigest()}),
                encoding="ascii",
            )
            report_dir = root / "report"
            argv = [
                "--udid",
                UDID,
                "--ipa",
                str(ipa),
                "--agent-manifest",
                str(manifest),
                "--token-env",
                "GATE0_TEST_TOKEN",
                "--tiktok-bundle",
                "com.ss.iphone.ugc.Ame",
                "--direct-url",
                "https://www.tiktok.com/@fixture/video/7411111111111111111",
                "--photo-url",
                "https://www.tiktok.com/@fixture/photo/7422222222222222222",
                "--short-url",
                "https://vt.tiktok.com/fixture/",
                "--report-dir",
                str(report_dir),
            ]
            prior = os.environ.get("GATE0_TEST_TOKEN")
            os.environ["GATE0_TEST_TOKEN"] = TOKEN
            try:
                with mock.patch.object(probe.platform, "system", return_value="Windows"), \
                    contextlib.redirect_stdout(io.StringIO()):
                    self.assertEqual(2, probe.main(argv))
            finally:
                if prior is None:
                    del os.environ["GATE0_TEST_TOKEN"]
                else:
                    os.environ["GATE0_TEST_TOKEN"] = prior
            published = (report_dir / "gate-0.json").read_bytes()
            self.assertNotIn(TOKEN.encode("ascii"), published)
            self.assertNotIn(UDID.encode("ascii"), published)
            self.assertEqual("PENDING_MAC_DEVICE", json.loads(published)["gateStatus"])
            with contextlib.redirect_stderr(io.StringIO()), self.assertRaises(SystemExit):
                probe.build_parser().parse_args(argv + ["--samples", "1"])


if __name__ == "__main__":
    unittest.main()
