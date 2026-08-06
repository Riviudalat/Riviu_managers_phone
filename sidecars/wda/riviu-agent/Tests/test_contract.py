import json
import unittest
from pathlib import Path


CONTRACT_PATH = Path(__file__).resolve().parents[1] / "Contracts" / "control-v2.json"
NATIVE_INPUT_PATH = (
    Path(__file__).resolve().parents[1] / "Contracts" / "native-input-v1.json"
)
MEDIA_CONTRACT_PATH = (
    Path(__file__).resolve().parents[1] / "Contracts" / "media-v1.json"
)


class ControlContractTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.raw = CONTRACT_PATH.read_text(encoding="utf-8")
        cls.contract = json.loads(cls.raw)
        cls.routes = {
            (route["method"], route["path"]): route
            for route in cls.contract["routes"]
        }

    def route(self, method, path):
        return self.routes[(method, path)]

    def assert_number_property(self, route, name, *, minimum=None, maximum=None):
        schema = route["requestSchema"]
        self.assertIn(name, schema["required"])
        prop = schema["properties"][name]
        self.assertEqual("number", prop["type"])
        self.assertIs(True, prop["finite"])
        if minimum is not None:
            self.assertEqual(minimum, prop["minimum"])
        if maximum is not None:
            self.assertEqual(maximum, prop["maximum"])

    def test_identity_network_and_features_are_exact(self):
        identity = self.contract["identity"]
        self.assertEqual(1, self.contract["schemaVersion"])
        self.assertEqual(2, identity["protocolVersion"])
        self.assertEqual("0.1.0", identity["agentVersion"])
        self.assertEqual(["stream", "tap", "swipe", "clipboard"], identity["features"])
        self.assertEqual({"width": 375, "height": 667}, identity["logicalSize"])

        network = self.contract["network"]
        self.assertEqual(8916, network["controlPort"])
        self.assertEqual(9094, network["mjpegPort"])
        self.assertNotEqual(8906, network["controlPort"])
        self.assertNotEqual(9093, network["mjpegPort"])

    def test_auth_names_and_truth_table_are_exact(self):
        auth = self.contract["authentication"]
        self.assertEqual("RIVIU_AGENT_TOKEN", auth["tokenEnvironment"])
        self.assertEqual("X-Riviu-Token", auth["header"])
        self.assertEqual(256, auth["minimumTokenBits"])
        self.assertIs(False, auth["tokenValueEmbedded"])
        self.assertEqual(
            {"missing": 401, "wrong": 401, "correct": 200},
            auth["statusExpectations"],
        )

    def test_wda_response_envelope_allows_active_or_absent_session(self):
        envelope = self.contract["wdaResponseEnvelope"]
        self.assertEqual(["value"], envelope["required"])
        self.assertEqual(
            {"oneOf": [{"type": "string", "minLength": 1}, {"type": "null"}]},
            envelope["properties"]["sessionId"],
        )
        self.assertIs(True, envelope["additionalProperties"])

        for route in self.contract["routes"]:
            schema = route.get("responseSchema")
            if schema is None or schema.get("additionalProperties", True):
                continue
            self.assertIn(
                "sessionId",
                schema.get("properties", {}),
                f'{route["method"]} {route["path"]} rejects the WDA sessionId envelope field',
            )

    def test_only_get_status_is_auth_exempt(self):
        exempt = [
            (route["method"], route["path"])
            for route in self.contract["routes"]
            if route["auth"] == "exempt"
        ]
        self.assertEqual([("GET", "/status")], exempt)
        self.assertEqual(
            [{"method": "GET", "path": "/status"}],
            self.contract["authentication"]["exemptions"],
        )
        for route in self.contract["routes"]:
            self.assertIn(route["auth"], {"exempt", "protected"})

    def test_route_set_is_complete_and_unique(self):
        expected = {
            ("GET", "/status"),
            ("GET", "/riviu/health"),
            ("GET", "/wda/locked"),
            ("POST", "/session"),
            ("GET", "/session/{sessionId}"),
            ("DELETE", "/session/{sessionId}"),
            ("POST", "/wda/tap"),
            ("POST", "/wda/swipe"),
            ("POST", "/wda/setPasteboard"),
            ("POST", "/wda/getPasteboard"),
            ("GET", "/screenshot"),
            ("POST", "/session/{sessionId}/element"),
            ("POST", "/session/{sessionId}/element/{elementId}/click"),
            ("POST", "/session/{sessionId}/element/{elementId}/clear"),
            ("GET", "/session/{sessionId}/element/{elementId}/text"),
            ("GET", "/session/{sessionId}/element/{elementId}/rect"),
            ("GET", "/session/{sessionId}/element/{elementId}/attribute/value"),
            ("GET", "/session/{sessionId}/wda/activeAppInfo"),
            ("POST", "/session/{sessionId}/wda/keys"),
        }
        pairs = [(route["method"], route["path"]) for route in self.contract["routes"]]
        self.assertEqual(len(pairs), len(set(pairs)))
        self.assertEqual(expected, set(pairs))

    def test_health_and_status_publish_candidate_identity(self):
        health = self.route("GET", "/riviu/health")
        self.assertEqual("protected", health["auth"])
        self.assertEqual("none", health["session"])
        self.assertEqual(
            [
                "agentVersion",
                "protocolVersion",
                "features",
                "logicalWidth",
                "logicalHeight",
                "state",
            ],
            health["responseSchema"]["properties"]["value"]["required"],
        )
        examples = health["responseExamples"]
        self.assertEqual(
            ["stream", "tap", "swipe", "clipboard"],
            examples["ready"]["value"]["features"],
        )
        self.assertEqual("ready", examples["ready"]["value"]["state"])
        self.assertEqual(
            ["tap", "swipe", "clipboard"],
            examples["degraded"]["value"]["features"],
        )
        self.assertEqual("degraded", examples["degraded"]["value"]["state"])
        self.assertNotIn("stream", examples["degraded"]["value"]["features"])

        alternatives = health["responseSchema"]["properties"]["value"]["oneOf"]
        states = {
            alternative["properties"]["state"]["const"]: alternative
            for alternative in alternatives
        }
        self.assertEqual({"ready", "degraded"}, set(states))
        self.assertEqual(
            ["stream", "tap", "swipe", "clipboard"],
            states["ready"]["properties"]["features"]["const"],
        )
        self.assertEqual(
            ["tap", "swipe", "clipboard"],
            states["degraded"]["properties"]["features"]["const"],
        )

        status = self.route("GET", "/status")
        self.assertEqual("none", status["session"])
        self.assertEqual("riviuAgent", status["identityKey"])
        status_identity = status["responseSchema"]["properties"]["value"]["properties"][
            "riviuAgent"
        ]
        self.assertEqual(
            [
                "agentVersion",
                "protocolVersion",
                "features",
                "logicalWidth",
                "logicalHeight",
                "state",
            ],
            status_identity["required"],
        )
        self.assertEqual(2, status_identity["properties"]["protocolVersion"]["const"])
        self.assertEqual("0.1.0", status_identity["properties"]["agentVersion"]["const"])

    def test_session_server_schema_and_live_probe_policy_are_distinct(self):
        create = self.route("POST", "/session")
        self.assertEqual("none", create["session"])
        capabilities = create["requestSchema"]["properties"]["capabilities"]
        self.assertIn("capabilities", create["requestSchema"]["required"])
        self.assertNotIn("required", capabilities)
        first_match = capabilities["properties"]["firstMatch"]
        self.assertEqual("array", first_match["type"])
        self.assertEqual("object", first_match["items"]["type"])
        self.assertIs(True, create["fresh"])

        policy = self.contract["lifecycle"]["liveProbeFreshSessionRequest"]
        self.assertEqual("capabilities.firstMatch", policy["requiredField"])
        self.assertEqual(1, policy["minimumItems"])
        self.assertEqual("policy", policy["enforcement"])

    def test_session_and_element_routes_are_scoped_to_unicode_readback_probe(self):
        active = self.route("GET", "/session/{sessionId}")
        close = self.route("DELETE", "/session/{sessionId}")
        keys = self.route("POST", "/session/{sessionId}/wda/keys")
        find = self.route("POST", "/session/{sessionId}/element")
        click = self.route("POST", "/session/{sessionId}/element/{elementId}/click")
        clear = self.route("POST", "/session/{sessionId}/element/{elementId}/clear")
        text = self.route("GET", "/session/{sessionId}/element/{elementId}/text")
        self.assertEqual("required", active["session"])
        self.assertEqual("activeSessionControlProbe", active["purpose"])
        for route in (keys, click, clear, text):
            self.assertEqual("required", route["session"])
            self.assertEqual("unicodeReadbackProbeOnly", route["purpose"])
        self.assertEqual("required", find["session"])
        self.assertEqual("probeElementLookupOnly", find["purpose"])
        self.assertEqual("required", close["session"])
        self.assertEqual(["value"], keys["requestSchema"]["required"])
        self.assertEqual("array", keys["requestSchema"]["properties"]["value"]["type"])
        self.assertEqual("string", keys["requestSchema"]["properties"]["value"]["items"]["type"])
        self.assertEqual(["using", "value"], find["requestSchema"]["required"])
        self.assertEqual("string", text["responseSchema"]["properties"]["value"]["type"])

        active = self.route("GET", "/session/{sessionId}/wda/activeAppInfo")
        rect = self.route("GET", "/session/{sessionId}/element/{elementId}/rect")
        value = self.route(
            "GET", "/session/{sessionId}/element/{elementId}/attribute/value"
        )
        self.assertEqual("required", active["session"])
        self.assertEqual("settingsActiveProbeOnly", active["purpose"])
        for route in (rect, value):
            self.assertEqual("required", route["session"])
            self.assertEqual("gestureSemanticProbeOnly", route["purpose"])

    def test_gesture_schemas_require_finite_numbers_and_bounded_delay(self):
        tap = self.route("POST", "/wda/tap")
        self.assertEqual("none", tap["session"])
        self.assert_number_property(tap, "x")
        self.assert_number_property(tap, "y")

        swipe = self.route("POST", "/wda/swipe")
        self.assertEqual("none", swipe["session"])
        for name in ("fromX", "fromY", "toX", "toY"):
            self.assert_number_property(swipe, name)
        self.assert_number_property(swipe, "delay", minimum=0, maximum=5)

    def test_clipboard_contract_is_base64_and_byte_exact(self):
        set_route = self.route("POST", "/wda/setPasteboard")
        get_route = self.route("POST", "/wda/getPasteboard")
        self.assertEqual("none", set_route["session"])
        self.assertEqual("none", get_route["session"])

        set_schema = set_route["requestSchema"]
        self.assertEqual(["content"], set_schema["required"])
        self.assertEqual("string", set_schema["properties"]["content"]["type"])
        self.assertEqual("base64", set_schema["properties"]["content"]["encoding"])
        self.assertEqual(
            ["ignoreUnknownCharacters"],
            set_schema["properties"]["content"]["decoderOptions"],
        )
        self.assertEqual("string", set_schema["properties"]["contentType"]["type"])
        self.assertIs(False, set_schema["additionalProperties"])
        self.assertIs(False, get_route["requestSchema"]["additionalProperties"])

        self.assertEqual("base64", get_route["responseSchema"]["properties"]["value"]["encoding"])
        self.assertIs(True, get_route["responseSchema"]["properties"]["value"]["byteExact"])
        self.assertIs(True, self.contract["lifecycle"]["clipboardRuntimeSchemaValidation"])

    def test_media_contract_is_implemented_but_not_promoted_by_default(self):
        media = json.loads(MEDIA_CONTRACT_PATH.read_text(encoding="utf-8"))
        self.assertEqual("candidate-route", media["status"])
        self.assertEqual("pushMedia", media["feature"])
        self.assertFalse(media["promotionGate"]["advertiseFeatureBeforeGate"])
        self.assertEqual(["size", "sha256"], media["transport"]["readback"])
        self.assertEqual(
            {
                "POST /riviu/media/v1/prepare",
                "GET /riviu/media/v1/prepare/{importId}",
                "POST /riviu/media/v1/import",
                "DELETE /riviu/media/v1/import/{importId}",
            },
            {f'{route["method"]} {route["path"]}' for route in media["routes"]},
        )

    def test_lifecycle_and_error_semantics_are_explicit(self):
        lifecycle = self.contract["lifecycle"]
        self.assertIs(True, lifecycle["targetForegroundBeforeFreshSession"])
        self.assertIs(True, lifecycle["freshSessionBeforeMjpegConnect"])
        self.assertIs(True, lifecycle["firstValidJpegDefinesStreamReady"])
        self.assertIs(True, lifecycle["clipboardProbeAgentForegroundPidStable"])
        self.assertIs(
            True, lifecycle["clipboardProbeAgentForegroundIdentityVerified"]
        )
        self.assertEqual("com.apple.Preferences", lifecycle["gestureSurfaceBundle"])
        self.assertIs(True, lifecycle["resetGestureSurfaceBeforeSession"])
        self.assertEqual(
            {"invalidBody": 400, "authFailure": 401, "success": 200},
            self.contract["httpStatus"],
        )

    def test_mjpeg_is_loopback_only_and_uses_the_control_token_header(self):
        mjpeg = self.contract["network"]["mjpeg"]
        self.assertEqual("localhost", mjpeg["bindInterface"])
        self.assertIs(True, mjpeg["loopbackOnly"])
        self.assertEqual("GET", mjpeg["request"]["method"])
        self.assertEqual("/", mjpeg["request"]["path"])
        self.assertEqual("X-Riviu-Token", mjpeg["authentication"]["header"])
        self.assertIs(True, mjpeg["authentication"]["required"])
        self.assertEqual(
            {"missing": 401, "wrong": 401, "correct": 200},
            mjpeg["authentication"]["statusExpectations"],
        )

    def test_forbidden_vendor_and_future_feature_strings_are_absent(self):
        for forbidden in ('"text"', "pushMedia", "FARM_KEY", "X-RT-Token"):
            self.assertNotIn(forbidden, self.raw)


class NativeInputContractTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.contract = json.loads(NATIVE_INPUT_PATH.read_text(encoding="utf-8"))
        cls.routes = {
            (route["method"], route["path"]): route
            for route in cls.contract["routes"]
        }

    def test_routes_are_sessionless_and_use_distinct_handlers(self):
        self.assertEqual(
            {("POST", "/wda/tap"), ("POST", "/wda/swipe")},
            set(self.routes),
        )
        tap = self.routes[("POST", "/wda/tap")]
        swipe = self.routes[("POST", "/wda/swipe")]
        self.assertIs(False, tap["requiresSession"])
        self.assertIs(False, swipe["requiresSession"])
        self.assertEqual("handleRiviuNativeTap:", tap["handler"])
        self.assertEqual("handleRiviuNativeSwipe:", swipe["handler"])

    def test_validation_and_direct_synthesis_are_locked(self):
        validation = self.contract["validation"]
        self.assertEqual(["x", "y"], validation["tap"]["finiteNumbers"])
        self.assertEqual(
            ["fromX", "fromY", "toX", "toY", "delay"],
            validation["swipe"]["finiteNumbers"],
        )
        self.assertEqual({"minimum": 0, "maximum": 5}, validation["swipe"]["delay"])
        synthesis = self.contract["synthesis"]
        self.assertEqual("directEventRecord", synthesis["strategy"])
        self.assertEqual("XCUIDevice.sharedDevice.orientation", synthesis["orientationSource"])
        self.assertEqual(
            {
                "portrait": "portrait",
                "portraitUpsideDown": "portraitUpsideDown",
                "landscapeLeft": "landscapeRight",
                "landscapeRight": "landscapeLeft",
                "unknown": "portrait",
                "faceUp": "portrait",
                "faceDown": "portrait",
            },
            synthesis["orientationMapping"],
        )
        self.assertIs(False, synthesis["readsAccessibilityHierarchy"])
        self.assertEqual(
            ["XCPointerEventPath", "XCSynthesizedEventRecord", "FBXCTestDaemonsProxy"],
            synthesis["apis"],
        )
        self.assertEqual(
            "FBXCTestDaemonsProxy.synthesizeEventWithRecord:timeout:error:",
            synthesis["dispatchApi"],
        )
        self.assertEqual(5, synthesis["deadlineSeconds"])
        self.assertIs(True, synthesis["requiresBooleanSuccess"])

    def test_evidence_and_forbidden_paths_are_explicit(self):
        evidence = self.contract["evidence"]
        self.assertEqual(
            ["handleHCTap:", "handleHCSwipe:", "hcEmit:offsets:tag:"],
            evidence["oracleSelectors"],
        )
        self.assertEqual(
            [
                "XCPointerEventPath",
                "XCSynthesizedEventRecord",
                "FBXCTestDaemonsProxy.synthesizeEventWithRecord:error:",
            ],
            evidence["baselineApis"],
        )
        self.assertEqual(
            [
                "/actions",
                "XCUICoordinate",
                "pressForDuration:thenDragToCoordinate:",
                "fb_waitUntilStable",
            ],
            self.contract["forbiddenPaths"],
        )


if __name__ == "__main__":
    unittest.main()
