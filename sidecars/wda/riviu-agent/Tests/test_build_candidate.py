from __future__ import annotations

import hashlib
import importlib.util
import inspect
import json
import os
import plistlib
import sys
import tempfile
import unittest
import zipfile
from pathlib import Path
from unittest import mock


AGENT_ROOT = Path(__file__).resolve().parents[1]
MODULE_PATH = AGENT_ROOT / "Scripts" / "build_candidate.py"
CONFIG_PATH = AGENT_ROOT / "Config" / "RiviuAgent.xcconfig"

spec = importlib.util.spec_from_file_location("riviu_build_candidate", MODULE_PATH)
if spec is None or spec.loader is None:
    raise RuntimeError(f"failed to load module spec: {MODULE_PATH}")
build_candidate = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = build_candidate
spec.loader.exec_module(build_candidate)


class BuildCandidateTests(unittest.TestCase):
    SOURCE_SHA256 = "a" * 64
    XCCONFIG_SHA256 = "c" * 64

    def setUp(self) -> None:
        self.xcode = build_candidate.XcodeVersion("16.4", "16F6")

    def _make_app(
        self,
        root: Path,
        *,
        info_overrides: dict[str, object] | None = None,
        outer_info_overrides: dict[str, object] | None = None,
    ) -> Path:
        app = root / "WebDriverAgentRunner-Runner.app"
        app.mkdir()
        info = {
            "CFBundleIdentifier": "com.riviu.managersphone.agent.xctrunner",
            "CFBundleShortVersionString": "0.1.0",
            "CFBundleVersion": "7",
            "CFBundleExecutable": "WebDriverAgentRunner-Runner",
        }
        attestation = {
            "RiviuAgentSourceSHA256": self.SOURCE_SHA256,
            "RiviuAgentXcconfigSHA256": self.XCCONFIG_SHA256,
            "RiviuAgentProtocolVersion": 2,
            "RiviuAgentObjectiveCUnitTests": "PASS",
            "RiviuAgentXcodeVersion": "16.4",
            "RiviuAgentXcodeBuild": "16F6",
        }
        if info_overrides is not None:
            attestation.update(info_overrides)
        if outer_info_overrides is not None:
            info.update(outer_info_overrides)
        with (app / "Info.plist").open("wb") as stream:
            plistlib.dump(info, stream, sort_keys=True)
        attestation_bundle = app / "PlugIns" / "WebDriverAgentRunner.xctest"
        attestation_bundle.mkdir(parents=True)
        with (attestation_bundle / "Info.plist").open("wb") as stream:
            plistlib.dump(attestation, stream, sort_keys=True)
        executable = app / info["CFBundleExecutable"]
        executable.write_bytes(b"candidate executable")
        executable.chmod(0o755)
        return app

    @staticmethod
    def _codesign_output() -> str:
        return "\n".join(
            [
                "Executable=/tmp/WebDriverAgentRunner-Runner.app/WebDriverAgentRunner-Runner",
                "Identifier=com.riviu.managersphone.agent.xctrunner",
                "Authority=Apple Development: Fixture Builder (ABCDE12345)",
                "Authority=Apple Worldwide Developer Relations Certification Authority",
                "TeamIdentifier=ABCDE12345",
            ]
        )

    def test_parses_xcode_version_and_build(self) -> None:
        parsed = build_candidate.parse_xcode_version("Xcode 16.4\nBuild version 16F6\n")

        self.assertEqual("16.4", parsed.version)
        self.assertEqual("16F6", parsed.build)

    def test_source_digest_guard_rejects_a_tree_mutation(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            source = Path(tmp) / "source"
            source.mkdir()
            document = source / "project.pbxproj"
            document.write_text("locked\n", encoding="ascii")
            expected = build_candidate.source_documents_sha256(source)

            build_candidate.require_source_digest(source, expected, "fixture preflight")
            document.write_text("mutated\n", encoding="ascii")

            with self.assertRaisesRegex(build_candidate.BuildError, "source tree changed"):
                build_candidate.require_source_digest(source, expected, "fixture postflight")

    def test_xcconfig_digest_guard_rejects_a_file_mutation(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            xcconfig = Path(tmp) / "RiviuAgent.xcconfig"
            xcconfig.write_text("SETTING = locked\n", encoding="ascii")
            expected = build_candidate.sha256_file(xcconfig)

            build_candidate.require_xcconfig_digest(xcconfig, expected, "fixture preflight")
            xcconfig.write_text("SETTING = mutated\n", encoding="ascii")

            with self.assertRaisesRegex(build_candidate.BuildError, "xcconfig changed"):
                build_candidate.require_xcconfig_digest(
                    xcconfig, expected, "fixture postflight"
                )

    def test_loads_xcconfig_digest_from_lock(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            lock = Path(tmp) / "baseline-lock.json"
            lock.write_text(
                json.dumps({"xcconfigSha256": self.XCCONFIG_SHA256}),
                encoding="ascii",
            )

            self.assertEqual(
                self.XCCONFIG_SHA256,
                build_candidate.load_locked_xcconfig_sha256(lock),
            )

    def test_candidate_build_guards_source_before_and_after_xcodebuild(self) -> None:
        source = inspect.getsource(build_candidate.build_candidate)

        self.assertGreaterEqual(source.count("require_source_digest("), 3)
        self.assertGreaterEqual(source.count("require_xcconfig_digest("), 3)
        self.assertLess(
            source.index("finalize_runtime_closure("),
            source.index("package_candidate_ipa("),
        )

    def test_rejects_incomplete_xcode_version(self) -> None:
        with self.assertRaisesRegex(build_candidate.BuildError, "Xcode version"):
            build_candidate.parse_xcode_version("Xcode 16.4\n")

    def test_captures_bundle_and_signature_identity_from_actual_app(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            app = self._make_app(Path(tmp))

            identity = build_candidate.capture_bundle_identity(
                app,
                self._codesign_output(),
                expected_source_sha256=self.SOURCE_SHA256,
                expected_xcconfig_sha256=self.XCCONFIG_SHA256,
                expected_xcode=self.xcode,
            )

        self.assertEqual("com.riviu.managersphone.agent.xctrunner", identity.bundle_id)
        self.assertEqual("0.1.0", identity.bundle_version)
        self.assertEqual("7", identity.bundle_build)
        self.assertEqual("WebDriverAgentRunner-Runner", identity.executable)
        self.assertEqual(
            "Apple Development: Fixture Builder (ABCDE12345)", identity.signer_identity
        )
        self.assertEqual("ABCDE12345", identity.signer_team_id)
        self.assertEqual("com.riviu.managersphone.agent.xctrunner", identity.signature_identifier)
        self.assertEqual(self.SOURCE_SHA256, identity.source_sha256)
        self.assertEqual(self.XCCONFIG_SHA256, identity.xcconfig_sha256)
        self.assertEqual(2, identity.protocol_version)
        self.assertEqual("PASS", identity.objective_c_unit_tests)
        self.assertEqual(self.xcode, identity.xcode)
        self.assertEqual(
            "PlugIns/WebDriverAgentRunner.xctest", identity.attestation_bundle
        )

    def test_rejects_attestation_copied_only_to_outer_runner_app(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            app = self._make_app(Path(tmp))
            nested_info = app / "PlugIns" / "WebDriverAgentRunner.xctest" / "Info.plist"
            attestation = plistlib.loads(nested_info.read_bytes())
            outer_info = plistlib.loads((app / "Info.plist").read_bytes())
            outer_info.update(attestation)
            (app / "Info.plist").write_bytes(plistlib.dumps(outer_info))
            nested_info.unlink()

            with self.assertRaisesRegex(build_candidate.BuildError, "attestation Info.plist"):
                build_candidate.capture_bundle_identity(
                    app,
                    self._codesign_output(),
                    expected_source_sha256=self.SOURCE_SHA256,
                    expected_xcconfig_sha256=self.XCCONFIG_SHA256,
                    expected_xcode=self.xcode,
                )

    def test_rejects_bundle_and_signature_identifier_mismatch(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            app = self._make_app(Path(tmp))
            output = self._codesign_output().replace(
                "Identifier=com.riviu.managersphone.agent.xctrunner",
                "Identifier=com.example.other",
            )

            with self.assertRaisesRegex(build_candidate.BuildError, "identifier mismatch"):
                build_candidate.capture_bundle_identity(
                    app,
                    output,
                    expected_source_sha256=self.SOURCE_SHA256,
                    expected_xcconfig_sha256=self.XCCONFIG_SHA256,
                    expected_xcode=self.xcode,
                )

    def test_rejects_non_candidate_runner_bundle(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            app = self._make_app(
                Path(tmp),
                outer_info_overrides={"CFBundleIdentifier": "com.example.actual.xctrunner"},
            )
            output = self._codesign_output().replace(
                "Identifier=com.riviu.managersphone.agent.xctrunner",
                "Identifier=com.example.actual.xctrunner",
            )

            with self.assertRaisesRegex(build_candidate.BuildError, "candidate bundle"):
                build_candidate.capture_bundle_identity(
                    app,
                    output,
                    expected_source_sha256=self.SOURCE_SHA256,
                    expected_xcconfig_sha256=self.XCCONFIG_SHA256,
                    expected_xcode=self.xcode,
                )

    def test_rejects_mismatched_signed_source_digest(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            app = self._make_app(
                Path(tmp), info_overrides={"RiviuAgentSourceSHA256": "b" * 64}
            )

            with self.assertRaisesRegex(build_candidate.BuildError, "source SHA-256 mismatch"):
                build_candidate.capture_bundle_identity(
                    app,
                    self._codesign_output(),
                    expected_source_sha256=self.SOURCE_SHA256,
                    expected_xcconfig_sha256=self.XCCONFIG_SHA256,
                    expected_xcode=self.xcode,
                )

    def test_rejects_mismatched_signed_xcconfig_digest(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            app = self._make_app(
                Path(tmp), info_overrides={"RiviuAgentXcconfigSHA256": "d" * 64}
            )

            with self.assertRaisesRegex(build_candidate.BuildError, "xcconfig SHA-256 mismatch"):
                build_candidate.capture_bundle_identity(
                    app,
                    self._codesign_output(),
                    expected_source_sha256=self.SOURCE_SHA256,
                    expected_xcconfig_sha256=self.XCCONFIG_SHA256,
                    expected_xcode=self.xcode,
                )

    def test_rejects_non_integer_signed_protocol(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            app = self._make_app(
                Path(tmp), info_overrides={"RiviuAgentProtocolVersion": "2"}
            )

            with self.assertRaisesRegex(build_candidate.BuildError, "integer 2"):
                build_candidate.capture_bundle_identity(
                    app,
                    self._codesign_output(),
                    expected_source_sha256=self.SOURCE_SHA256,
                    expected_xcconfig_sha256=self.XCCONFIG_SHA256,
                    expected_xcode=self.xcode,
                )

    def test_rejects_unsigned_objective_c_test_claim(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            app = self._make_app(
                Path(tmp), info_overrides={"RiviuAgentObjectiveCUnitTests": "FAIL"}
            )

            with self.assertRaisesRegex(build_candidate.BuildError, "must be PASS"):
                build_candidate.capture_bundle_identity(
                    app,
                    self._codesign_output(),
                    expected_source_sha256=self.SOURCE_SHA256,
                    expected_xcconfig_sha256=self.XCCONFIG_SHA256,
                    expected_xcode=self.xcode,
                )

    def test_rejects_mismatched_signed_xcode_identity(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            app = self._make_app(
                Path(tmp), info_overrides={"RiviuAgentXcodeBuild": "16F7"}
            )

            with self.assertRaisesRegex(build_candidate.BuildError, "Xcode mismatch"):
                build_candidate.capture_bundle_identity(
                    app,
                    self._codesign_output(),
                    expected_source_sha256=self.SOURCE_SHA256,
                    expected_xcconfig_sha256=self.XCCONFIG_SHA256,
                    expected_xcode=self.xcode,
                )

    def test_generates_candidate_manifest_from_measured_values(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            artifact_root = Path(tmp) / "artifacts" / "0.1.0"
            artifact_root.mkdir(parents=True)
            ipa = artifact_root / "RiviuAgent-candidate.ipa"
            ipa.write_bytes(b"signed candidate bytes")
            app = self._make_app(Path(tmp))
            identity = build_candidate.capture_bundle_identity(
                app,
                self._codesign_output(),
                expected_source_sha256=self.SOURCE_SHA256,
                expected_xcconfig_sha256=self.XCCONFIG_SHA256,
                expected_xcode=self.xcode,
            )

            manifest = build_candidate.generate_candidate_manifest(
                artifact_version="0.1.0",
                artifact_root=artifact_root,
                ipa_path=ipa,
                app_name=app.name,
                identity=identity,
            )

        self.assertEqual("PENDING_MAC_DEVICE", manifest["gateStatus"])
        self.assertEqual(2, manifest["protocolVersion"])
        self.assertEqual(["stream", "tap", "swipe", "clipboard"], manifest["features"])
        self.assertEqual("RiviuAgent-candidate.ipa", manifest["ipa"])
        self.assertEqual(hashlib.sha256(b"signed candidate bytes").hexdigest(), manifest["sha256"])
        self.assertEqual("com.riviu.managersphone.agent.xctrunner", manifest["bundleId"])
        self.assertEqual("ABCDE12345", manifest["signerTeamId"])
        self.assertEqual({"version": "16.4", "build": "16F6"}, manifest["xcode"])
        self.assertEqual("PASS", manifest["objectiveCUnitTests"])
        self.assertEqual(self.SOURCE_SHA256, manifest["sourceSha256"])
        self.assertEqual(self.XCCONFIG_SHA256, manifest["xcconfigSha256"])
        self.assertEqual(
            "PlugIns/WebDriverAgentRunner.xctest", manifest["attestationBundle"]
        )
        serialized = json.dumps(manifest, sort_keys=True)
        self.assertNotIn("RIVIU_AGENT_TOKEN", serialized)
        self.assertNotIn("X-Riviu-Token", serialized)

    def test_rejects_ipa_outside_artifact_directory(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            outside = root / "outside.ipa"
            outside.write_bytes(b"x")

            with self.assertRaisesRegex(build_candidate.BuildError, "inside artifact root"):
                build_candidate.safe_artifact_relative_path(outside, root / "artifacts")

    def test_packages_ipa_in_sorted_deterministic_order(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            app = self._make_app(root)
            (app / "z.txt").write_text("z", encoding="utf-8")
            (app / "a.txt").write_text("a", encoding="utf-8")
            nested = app / "Nested"
            nested.mkdir()
            (nested / "b.txt").write_text("b", encoding="utf-8")
            first = root / "first.ipa"
            second = root / "second.ipa"

            first_names = build_candidate.package_candidate_ipa(app, first)
            for path in app.rglob("*"):
                os.utime(path, (2_000_000_000, 2_000_000_000))
            second_names = build_candidate.package_candidate_ipa(app, second)

            self.assertEqual(sorted(first_names), first_names)
            self.assertEqual(first_names, second_names)
            self.assertEqual(first.read_bytes(), second.read_bytes())
            with zipfile.ZipFile(first) as archive:
                self.assertEqual(first_names, archive.namelist())
                self.assertTrue(all(item.date_time == (1980, 1, 1, 0, 0, 0) for item in archive.infolist()))

    def test_packages_prefix_siblings_in_global_archive_order(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            app = self._make_app(root)
            directory = app / "Assets"
            directory.mkdir()
            (directory / "icon.png").write_bytes(b"png")
            (app / "Assets.car").write_bytes(b"car")
            destination = root / "candidate.ipa"

            names = build_candidate.package_candidate_ipa(app, destination)

            self.assertEqual(sorted(names), names)
            with zipfile.ZipFile(destination) as archive:
                self.assertEqual(names, archive.namelist())

    def test_rejects_missing_team_before_tool_lookup(self) -> None:
        with self.assertRaisesRegex(build_candidate.BuildError, "team ID"):
            build_candidate.validate_build_requirements(
                team_id=" ",
                udid="fixture-device",
                platform_name="darwin",
                tool_lookup=lambda _name: "/usr/bin/tool",
            )

    def test_rejects_missing_device_before_tool_lookup(self) -> None:
        with self.assertRaisesRegex(build_candidate.BuildError, "device UDID"):
            build_candidate.validate_build_requirements(
                team_id="ABCDE12345",
                udid="",
                platform_name="darwin",
                tool_lookup=lambda _name: "/usr/bin/tool",
            )

    def test_rejects_non_macos_host(self) -> None:
        with self.assertRaisesRegex(build_candidate.BuildError, "macOS"):
            build_candidate.validate_build_requirements(
                team_id="ABCDE12345",
                udid="fixture-device",
                platform_name="win32",
                tool_lookup=lambda _name: "/usr/bin/tool",
            )

    def test_rejects_missing_required_toolchain(self) -> None:
        available = {"xcodebuild", "security", "codesign"}

        with self.assertRaisesRegex(build_candidate.BuildError, "xcrun"):
            build_candidate.validate_build_requirements(
                team_id="ABCDE12345",
                udid="fixture-device",
                platform_name="darwin",
                tool_lookup=lambda name: f"/usr/bin/{name}" if name in available else None,
            )

    def test_xcodebuild_command_uses_explicit_list_arguments(self) -> None:
        command = build_candidate.make_xcodebuild_command(
            source=Path("/tmp/source"),
            derived_data=Path("/tmp/derived"),
            xcconfig=Path("/tmp/RiviuAgent.xcconfig"),
            team_id="ABCDE12345",
            udid="fixture-device",
            source_sha256=self.SOURCE_SHA256,
            xcconfig_sha256=self.XCCONFIG_SHA256,
            xcode=self.xcode,
        )

        self.assertEqual(["xcodebuild", "build-for-testing"], command[:2])
        self.assertIn("DEVELOPMENT_TEAM=ABCDE12345", command)
        self.assertIn("id=fixture-device", command)
        self.assertIn(str(Path("/tmp/RiviuAgent.xcconfig")), command)
        self.assertIn(f"RIVIU_AGENT_SOURCE_SHA256={self.SOURCE_SHA256}", command)
        self.assertIn(f"RIVIU_AGENT_XCCONFIG_SHA256={self.XCCONFIG_SHA256}", command)
        self.assertNotIn("RIVIU_AGENT_PROTOCOL_VERSION=2", command)
        self.assertIn("RIVIU_AGENT_OBJECTIVE_C_UNIT_TESTS=PASS", command)
        self.assertIn("RIVIU_AGENT_XCODE_VERSION=16.4", command)
        self.assertIn("RIVIU_AGENT_XCODE_BUILD=16F6", command)
        self.assertNotIn("RIVIU_AGENT_TOKEN", " ".join(command))

    def test_xcode26_runtime_closure_embeds_missing_device_libraries(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            app = self._make_app(root)
            frameworks = app / "Frameworks"
            (frameworks / "Testing.framework").mkdir(parents=True)
            (frameworks / "Testing.framework" / "Testing").write_bytes(b"testing")
            (frameworks / "libXCTestSwiftSupport.dylib").write_bytes(b"swift")
            platform = root / "iPhoneOS.platform"
            interop = platform / "Developer" / "usr" / "lib" / "lib_TestingInterop.dylib"
            interop.parent.mkdir(parents=True)
            interop.write_bytes(b"interop")
            foundation = (
                platform
                / "Developer"
                / "Library"
                / "Frameworks"
                / "_Testing_Foundation.framework"
            )
            foundation.mkdir(parents=True)
            (foundation / "_Testing_Foundation").write_bytes(b"foundation")

            build_candidate.ensure_xcode_runtime_dependencies(
                app, platform, build_candidate.XcodeVersion("26.0", "17A1")
            )

            self.assertEqual(
                b"interop", (frameworks / "lib_TestingInterop.dylib").read_bytes()
            )
            self.assertEqual(
                b"foundation",
                (
                    frameworks
                    / "_Testing_Foundation.framework"
                    / "_Testing_Foundation"
                ).read_bytes(),
            )
            for relative in build_candidate.XCODE26_RUNTIME_CLOSURE:
                self.assertTrue((frameworks / relative).is_file(), relative)

    def test_xcode26_runtime_closure_rejects_missing_build_dependency(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            app = self._make_app(root)
            frameworks = app / "Frameworks"
            frameworks.mkdir()
            (frameworks / "libXCTestSwiftSupport.dylib").write_bytes(b"swift")
            platform = root / "iPhoneOS.platform"
            (platform / "Developer" / "usr" / "lib").mkdir(parents=True)
            (platform / "Developer" / "Library" / "Frameworks").mkdir(parents=True)

            with self.assertRaisesRegex(build_candidate.BuildError, "Testing.framework"):
                build_candidate.ensure_xcode_runtime_dependencies(
                    app, platform, build_candidate.XcodeVersion("26.0", "17A1")
                )

    def test_resigns_dependencies_then_xctest_then_outer_app(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            app = self._make_app(Path(tmp))
            frameworks = app / "Frameworks"
            (frameworks / "Testing.framework").mkdir(parents=True)
            (frameworks / "libXCTestSwiftSupport.dylib").write_bytes(b"swift")
            calls: list[list[str]] = []

            def fake_run(command, **_kwargs):
                calls.append(list(command))
                return build_candidate.subprocess.CompletedProcess(command, 0, "", "")

            with mock.patch.object(build_candidate, "_run", side_effect=fake_run):
                build_candidate.resign_candidate_tree(
                    app, "Apple Development: Fixture Builder (ABCDE12345)"
                )

        signed_paths = [Path(command[-1]) for command in calls]
        self.assertEqual(
            [
                frameworks / "Testing.framework",
                frameworks / "libXCTestSwiftSupport.dylib",
                app / "PlugIns" / "WebDriverAgentRunner.xctest",
                app,
            ],
            signed_paths,
        )
        self.assertTrue(
            all("--preserve-metadata=entitlements,flags" in command for command in calls)
        )

    def test_runtime_finalization_deep_verifies_after_resigning(self) -> None:
        app = Path("/tmp/WebDriverAgentRunner-Runner.app")
        events: list[str] = []

        with mock.patch.object(
            build_candidate,
            "_inspect_signature",
            side_effect=lambda _app: events.append("inspect") or self._codesign_output(),
        ), mock.patch.object(
            build_candidate,
            "resolve_iphoneos_platform_path",
            side_effect=lambda: events.append("platform") or Path("/tmp/iPhoneOS.platform"),
        ), mock.patch.object(
            build_candidate,
            "ensure_xcode_runtime_dependencies",
            side_effect=lambda *_args: events.append("embed"),
        ), mock.patch.object(
            build_candidate,
            "resign_candidate_tree",
            side_effect=lambda *_args: events.append("resign"),
        ):
            signature = build_candidate.finalize_runtime_closure(
                app, build_candidate.XcodeVersion("26.0", "17A1")
            )

        self.assertEqual(["inspect", "platform", "embed", "resign", "inspect"], events)
        self.assertEqual(self._codesign_output(), signature)

    def test_xcode_unit_test_command_runs_only_the_unit_target(self) -> None:
        command = build_candidate.make_xcode_unit_test_command(
            source=Path("/tmp/source"),
            derived_data=Path("/tmp/unit-derived"),
            xcconfig=Path("/tmp/RiviuAgent.xcconfig"),
            team_id="ABCDE12345",
            udid="fixture-device",
        )

        self.assertEqual(["xcodebuild", "test"], command[:2])
        self.assertIn("WebDriverAgentLib", command)
        self.assertIn("-only-testing:UnitTests", command)
        self.assertIn("id=fixture-device", command)
        self.assertNotIn("RIVIU_AGENT_TOKEN", " ".join(command))

    def test_xcconfig_pins_candidate_identity_without_credentials(self) -> None:
        text = CONFIG_PATH.read_text(encoding="ascii")

        self.assertIn("PRODUCT_BUNDLE_IDENTIFIER = com.riviu.managersphone.agent", text)
        self.assertIn("MARKETING_VERSION = 0.1.0", text)
        self.assertIn("CURRENT_PROJECT_VERSION = 1", text)
        self.assertIn("IPHONEOS_DEPLOYMENT_TARGET = 13.0", text)
        self.assertIn("INFOPLIST_EXPAND_BUILD_SETTINGS = YES", text)
        self.assertNotIn("INFOPLIST_KEY_RiviuAgent", text)
        self.assertNotIn("DEVELOPMENT_TEAM", text)
        self.assertNotIn("TOKEN", text)

    def test_fresh_derived_data_layout_never_reuses_requested_root(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            requested = Path(tmp) / "derived-data"
            requested.mkdir()
            stale = requested / "stale-marker"
            stale.write_text("old", encoding="ascii")

            first = build_candidate.create_fresh_derived_data_layout(requested)
            second = build_candidate.create_fresh_derived_data_layout(requested)

            self.assertNotEqual(first.root, second.root)
            self.assertEqual(requested, first.root.parent)
            self.assertEqual(requested, second.root.parent)
            self.assertNotEqual(requested, first.unit_tests)
            self.assertNotEqual(requested, first.runner)
            self.assertNotEqual(first.unit_tests, first.runner)
            self.assertEqual(first.root / "unit-tests", first.unit_tests)
            self.assertEqual(first.root / "runner", first.runner)
            self.assertTrue(stale.is_file())

    def test_run_forwards_explicit_timeout(self) -> None:
        completed = build_candidate.subprocess.CompletedProcess(["fixture"], 0, "ok", "")
        with mock.patch.object(
            build_candidate.subprocess, "run", return_value=completed
        ) as run:
            actual = build_candidate._run(
                ["fixture"], label="fixture command", timeout_seconds=17
            )

        self.assertIs(completed, actual)
        self.assertEqual(17, run.call_args.kwargs["timeout"])

    def test_run_reports_concise_timeout(self) -> None:
        expired = build_candidate.subprocess.TimeoutExpired(["fixture"], timeout=23)
        with mock.patch.object(build_candidate.subprocess, "run", side_effect=expired):
            with self.assertRaisesRegex(
                build_candidate.BuildError, r"^fixture command timed out after 23 seconds$"
            ):
                build_candidate._run(
                    ["fixture"], label="fixture command", timeout_seconds=23
                )


if __name__ == "__main__":
    unittest.main()
