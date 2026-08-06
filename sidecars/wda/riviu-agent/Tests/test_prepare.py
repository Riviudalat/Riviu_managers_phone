from __future__ import annotations

import base64
import hashlib
import importlib.util
import io
import json
import os
import plistlib
import stat
import tarfile
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
PREPARE_PATH = ROOT / "Scripts" / "prepare.py"
REPOSITORY_ROOT = ROOT.parents[2]
REAL_ARCHIVE = (
    REPOSITORY_ROOT
    / "target"
    / "rtmmo-re"
    / "baselines"
    / "appium-webdriveragent-15.1.4.tgz"
)


def load_prepare_module():
    spec = importlib.util.spec_from_file_location("riviu_agent_prepare", PREPARE_PATH)
    if spec is None or spec.loader is None:
        raise RuntimeError("cannot load prepare module")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def sri_sha512(data: bytes) -> str:
    digest = hashlib.sha512(data).digest()
    return "sha512-" + base64.b64encode(digest).decode("ascii")


class PrepareFixture:
    def __init__(self) -> None:
        self.temp = tempfile.TemporaryDirectory(prefix="riviu-agent-prepare-")
        self.root = Path(self.temp.name)
        self.archive = self.root / "baseline.tgz"
        self.upstream_lock = self.root / "upstream-lock.json"
        self.project_lock = self.root / "baseline-lock.json"
        self.output = self.root / "output"
        self.patch_root = self.root / "patch-root"
        self.patch_root.mkdir()

    def close(self) -> None:
        self.temp.cleanup()

    def write_archive(
        self,
        files: dict[str, bytes],
        *,
        modes: dict[str, int] | None = None,
        symlink: str | None = None,
    ) -> bytes:
        with tarfile.open(self.archive, "w:gz") as archive:
            for name, contents in files.items():
                info = tarfile.TarInfo(name)
                info.size = len(contents)
                info.mode = (modes or {}).get(name, 0o644)
                archive.addfile(info, io.BytesIO(contents))
            if symlink is not None:
                info = tarfile.TarInfo(symlink)
                info.type = tarfile.SYMTYPE
                info.linkname = "package/source.m"
                archive.addfile(info)
        return self.archive.read_bytes()

    def write_locks(
        self,
        archive_bytes: bytes,
        *,
        project_version: str = "15.1.4",
        project_git_head: str = "a" * 40,
        integrity: str | None = None,
        patches: list[dict[str, str]] | None = None,
        baseline_source_sha256: str | None = None,
        output_source_sha256: str | None = None,
    ) -> None:
        upstream = {
            "package": "appium-webdriveragent",
            "version": "15.1.4",
            "gitHead": "a" * 40,
            "tarball": "https://registry.invalid/baseline.tgz",
            "integrity": sri_sha512(archive_bytes),
        }
        self.upstream_lock.write_text(json.dumps(upstream), encoding="utf-8")
        project = {
            "schemaVersion": 1,
            "upstreamLock": self.upstream_lock.name,
            "package": "appium-webdriveragent",
            "version": project_version,
            "gitHead": project_git_head,
            "archiveSha256": hashlib.sha256(archive_bytes).hexdigest(),
            "integrity": integrity or upstream["integrity"],
            "baselineSourceSha256": baseline_source_sha256,
            "outputSourceSha256": output_source_sha256,
            "patches": patches or [],
        }
        self.project_lock.write_text(json.dumps(project), encoding="utf-8")


class PrepareTests(unittest.TestCase):
    def setUp(self) -> None:
        self.fixture = PrepareFixture()

    def tearDown(self) -> None:
        self.fixture.close()

    @staticmethod
    def package_files(version: str = "15.1.4") -> dict[str, bytes]:
        package = json.dumps(
            {"name": "appium-webdriveragent", "version": version}
        ).encode("utf-8")
        return {
            "package/package.json": package,
            "package/WebDriverAgentLib/Source.m": b"@interface Fixture\n@end\n",
            "package/README.md": b"fixture\n",
        }

    def prepare(self):
        module = load_prepare_module()
        return module.prepare_source(
            self.fixture.archive,
            self.fixture.project_lock,
            self.fixture.output,
        )

    def test_extracts_valid_archive_and_reports_deterministic_digest(self) -> None:
        archive_bytes = self.fixture.write_archive(self.package_files())
        self.fixture.write_locks(archive_bytes)

        first = self.prepare()
        first_digest = first.output_source_sha256
        second = self.prepare()

        self.assertEqual(first_digest, second.output_source_sha256)
        self.assertTrue((self.fixture.output / "WebDriverAgentLib/Source.m").is_file())
        self.assertEqual(first.patch_count, 0)

    def test_archive_executable_mode_changes_source_digest(self) -> None:
        files = self.package_files()
        script = "package/Scripts/build.sh"
        files[script] = b"#!/bin/sh\nexit 0\n"
        archive_bytes = self.fixture.write_archive(files, modes={script: 0o755})
        self.fixture.write_locks(archive_bytes)
        executable_digest = self.prepare().baseline_source_sha256

        archive_bytes = self.fixture.write_archive(files, modes={script: 0o644})
        self.fixture.write_locks(archive_bytes)
        regular_digest = self.prepare().baseline_source_sha256

        self.assertNotEqual(executable_digest, regular_digest)

    @unittest.skipUnless(os.name == "posix", "POSIX file modes are required")
    def test_posix_extraction_preserves_normalized_tar_modes(self) -> None:
        files = self.package_files()
        script = "package/Scripts/build.sh"
        source = "package/WebDriverAgentLib/Source.m"
        files[script] = b"#!/bin/sh\nexit 0\n"
        archive_bytes = self.fixture.write_archive(
            files,
            modes={script: 0o711, source: 0o600},
        )
        self.fixture.write_locks(archive_bytes)

        self.prepare()

        self.assertEqual(
            stat.S_IMODE((self.fixture.output / "Scripts" / "build.sh").stat().st_mode),
            0o755,
        )
        self.assertEqual(
            stat.S_IMODE(
                (self.fixture.output / "WebDriverAgentLib" / "Source.m").stat().st_mode
            ),
            0o644,
        )

    @unittest.skipUnless(os.name == "posix", "POSIX file modes are required")
    def test_source_digest_changes_when_executable_bit_changes(self) -> None:
        module = load_prepare_module()
        with tempfile.TemporaryDirectory(prefix="riviu-agent-mode-digest-") as temp:
            root = Path(temp)
            script = root / "build.sh"
            script.write_bytes(b"#!/bin/sh\nexit 0\n")
            script.chmod(0o644)
            regular_digest = module.source_documents_sha256(root)

            script.chmod(0o755)
            executable_digest = module.source_documents_sha256(root)

        self.assertNotEqual(regular_digest, executable_digest)

    def test_source_digest_includes_property_lists(self) -> None:
        module = load_prepare_module()
        with tempfile.TemporaryDirectory(prefix="riviu-agent-digest-") as temp:
            root = Path(temp)
            source = root / "Source.m"
            info = root / "Info.plist"
            source.write_bytes(b"@interface Fixture\n@end\n")
            info.write_bytes(b"<plist><dict><key>Fixture</key><string>one</string></dict></plist>\n")
            before = module.source_documents_sha256(root)

            info.write_bytes(b"<plist><dict><key>Fixture</key><string>two</string></dict></plist>\n")
            after = module.source_documents_sha256(root)

        self.assertNotEqual(before, after)

    def test_source_digest_includes_xcode_project_build_inputs(self) -> None:
        module = load_prepare_module()
        with tempfile.TemporaryDirectory(prefix="riviu-agent-digest-") as temp:
            root = Path(temp)
            project = root / "WebDriverAgent.xcodeproj" / "project.pbxproj"
            project.parent.mkdir()
            project.write_bytes(b"PRODUCT_NAME = Before;\n")
            before = module.source_documents_sha256(root)

            project.write_bytes(b"PRODUCT_NAME = After;\n")
            after = module.source_documents_sha256(root)

        self.assertNotEqual(before, after)

    def test_rejects_sha512_integrity_mismatch(self) -> None:
        archive_bytes = self.fixture.write_archive(self.package_files())
        self.fixture.write_locks(archive_bytes, integrity="sha512-" + "A" * 88)

        with self.assertRaisesRegex(Exception, "integrity"):
            self.prepare()
        self.assertFalse(self.fixture.output.exists())

    def test_rejects_package_version_mismatch(self) -> None:
        archive_bytes = self.fixture.write_archive(self.package_files(version="15.1.3"))
        self.fixture.write_locks(archive_bytes)

        with self.assertRaisesRegex(Exception, "version"):
            self.prepare()

    def test_rejects_project_git_head_mismatch_with_upstream_lock(self) -> None:
        archive_bytes = self.fixture.write_archive(self.package_files())
        self.fixture.write_locks(archive_bytes, project_git_head="b" * 40)

        with self.assertRaisesRegex(Exception, "gitHead"):
            self.prepare()

    def test_rejects_parent_path_before_extracting(self) -> None:
        files = self.package_files()
        files["package/../../outside"] = b"x"
        archive_bytes = self.fixture.write_archive(files)
        self.fixture.write_locks(archive_bytes)

        with self.assertRaisesRegex(Exception, "unsafe archive path"):
            self.prepare()
        self.assertFalse((self.fixture.root / "outside").exists())

    def test_rejects_absolute_archive_path(self) -> None:
        files = self.package_files()
        files["/absolute"] = b"x"
        archive_bytes = self.fixture.write_archive(files)
        self.fixture.write_locks(archive_bytes)

        with self.assertRaisesRegex(Exception, "unsafe archive path"):
            self.prepare()

    def test_rejects_symlink_member(self) -> None:
        archive_bytes = self.fixture.write_archive(
            self.package_files(), symlink="package/link.m"
        )
        self.fixture.write_locks(archive_bytes)

        with self.assertRaisesRegex(Exception, "unsupported archive member"):
            self.prepare()

    def test_rejects_patch_hash_mismatch_before_apply(self) -> None:
        archive_bytes = self.fixture.write_archive(self.package_files())
        patch = self.fixture.patch_root / "change.patch"
        patch.write_text("not a patch\n", encoding="utf-8")
        self.fixture.write_locks(
            archive_bytes,
            patches=[{"path": "patch-root/change.patch", "sha256": "0" * 64}],
        )

        with self.assertRaisesRegex(Exception, "patch checksum"):
            self.prepare()

    def test_applies_valid_patch_inside_prepared_tree(self) -> None:
        archive_bytes = self.fixture.write_archive(self.package_files())
        patch = self.fixture.patch_root / "change.patch"
        patch_bytes = (
            b"diff --git a/WebDriverAgentLib/Source.m b/WebDriverAgentLib/Source.m\n"
            b"--- a/WebDriverAgentLib/Source.m\n"
            b"+++ b/WebDriverAgentLib/Source.m\n"
            b"@@ -1,2 +1,2 @@\n"
            b"-@interface Fixture\n"
            b"+@interface PatchedFixture\n"
            b" @end\n"
        )
        patch.write_bytes(patch_bytes)
        self.fixture.write_locks(
            archive_bytes,
            patches=[
                {
                    "path": "patch-root/change.patch",
                    "sha256": hashlib.sha256(patch_bytes).hexdigest(),
                }
            ],
        )

        result = self.prepare()

        source = (self.fixture.output / "WebDriverAgentLib" / "Source.m").read_text(
            encoding="utf-8"
        )
        self.assertIn("PatchedFixture", source)
        self.assertNotEqual(result.baseline_source_sha256, result.output_source_sha256)

    def test_applies_patch_when_output_is_nested_under_repository(self) -> None:
        archive_bytes = self.fixture.write_archive(self.package_files())
        patch = self.fixture.patch_root / "change.patch"
        patch_bytes = (
            b"diff --git a/WebDriverAgentLib/Source.m b/WebDriverAgentLib/Source.m\n"
            b"--- a/WebDriverAgentLib/Source.m\n"
            b"+++ b/WebDriverAgentLib/Source.m\n"
            b"@@ -1,2 +1,2 @@\n"
            b"-@interface Fixture\n"
            b"+@interface NestedPatchedFixture\n"
            b" @end\n"
        )
        patch.write_bytes(patch_bytes)
        self.fixture.write_locks(
            archive_bytes,
            patches=[
                {
                    "path": "patch-root/change.patch",
                    "sha256": hashlib.sha256(patch_bytes).hexdigest(),
                }
            ],
        )

        with tempfile.TemporaryDirectory(
            prefix="riviu-agent-nested-", dir=REPOSITORY_ROOT / "target"
        ) as temp:
            nested_output = Path(temp) / "output"
            module = load_prepare_module()
            result = module.prepare_source(
                self.fixture.archive, self.fixture.project_lock, nested_output
            )
            source = (nested_output / "WebDriverAgentLib" / "Source.m").read_text(
                encoding="utf-8"
            )

        self.assertIn("NestedPatchedFixture", source)
        self.assertNotEqual(result.baseline_source_sha256, result.output_source_sha256)

    def test_rejects_baseline_source_digest_mismatch(self) -> None:
        archive_bytes = self.fixture.write_archive(self.package_files())
        self.fixture.write_locks(archive_bytes, baseline_source_sha256="0" * 64)

        with self.assertRaisesRegex(Exception, "baseline source digest"):
            self.prepare()

    def test_rejects_output_source_digest_mismatch(self) -> None:
        archive_bytes = self.fixture.write_archive(self.package_files())
        self.fixture.write_locks(archive_bytes, output_source_sha256="0" * 64)

        with self.assertRaisesRegex(Exception, "output source digest"):
            self.prepare()
        self.assertFalse(self.fixture.output.exists())


class RealOverlayTests(unittest.TestCase):
    def prepare_real_source(self, output: Path):
        module = load_prepare_module()
        return module.prepare_source(REAL_ARCHIVE, ROOT / "baseline-lock.json", output)

    def test_auth_overlay_is_applied_at_connection_boundary(self) -> None:
        with tempfile.TemporaryDirectory(prefix="riviu-agent-overlay-") as temp:
            output = Path(temp) / "source"
            result = self.prepare_real_source(output)
            server = (
                output / "WebDriverAgentLib" / "Routing" / "FBWebServer.m"
            ).read_text(encoding="utf-8")

            expected_patch_count = len(
                json.loads((ROOT / "baseline-lock.json").read_text(encoding="utf-8"))[
                    "patches"
                ]
            )
            self.assertEqual(result.patch_count, expected_patch_count)
            self.assertIn("FBRiviuConstantTimeDataEqual", server)
            self.assertIn("FBRiviuRequestIsAuthenticationExempt", server)
            self.assertIn("RIVIU_AGENT_TOKEN", server)
            self.assertIn("X-Riviu-Token", server)
            self.assertIn("tokenData.length < 32", server)
            self.assertIn("httpResponseForMethod:(NSString *)method URI:(NSString *)path", server)
            self.assertLess(
                server.index("FBRiviuRequireAgentToken"),
                server.index("self.server = [[RoutingHTTPServer alloc] init]"),
            )

            comparison_start = server.index("BOOL FBRiviuConstantTimeDataEqual")
            comparison_end = server.index("BOOL FBRiviuRequestIsAuthenticationExempt")
            comparison = server[comparison_start:comparison_end]
            self.assertNotIn("isEqualToString", comparison)

    def test_overlay_preserves_the_pinned_sources_lf_line_endings(self) -> None:
        with tempfile.TemporaryDirectory(prefix="riviu-agent-overlay-") as temp:
            output = Path(temp) / "source"
            self.prepare_real_source(output)

            for relative in (
                "WebDriverAgentLib/Commands/FBSessionCommands.m",
                "WebDriverAgentLib/Commands/FBElementCommands.m",
                "WebDriverAgentLib/Routing/FBWebServer.h",
                "WebDriverAgentLib/Routing/FBWebServer.m",
                "WebDriverAgentTests/UnitTests/FBRouteTests.m",
            ):
                self.assertNotIn(b"\r\n", (output / relative).read_bytes(), relative)

    def test_auth_overlay_exposes_only_the_project2_identity(self) -> None:
        with tempfile.TemporaryDirectory(prefix="riviu-agent-overlay-") as temp:
            output = Path(temp) / "source"
            self.prepare_real_source(output)
            session = (
                output
                / "WebDriverAgentLib"
                / "Commands"
                / "FBSessionCommands.m"
            ).read_text(encoding="utf-8")
            route_tests = (
                output
                / "WebDriverAgentTests"
                / "UnitTests"
                / "FBRouteTests.m"
            ).read_text(encoding="utf-8")

            self.assertIn('@"/riviu/health"', session)
            self.assertIn('@"riviuAgent"', session)
            for feature in ('@"stream"', '@"tap"', '@"swipe"', '@"clipboard"'):
                self.assertIn(feature, session)
            self.assertIn('RIVIU_AGENT_TEXT_CAPABLE', session)
            self.assertIn('RIVIU_AGENT_MEDIA_CAPABLE', session)
            self.assertIn("testRiviuStatusRouteIsTheOnlyAuthenticationExemption", route_tests)
            self.assertIn("testRiviuAuthenticationTruthTable", route_tests)

    def test_native_input_overlay_uses_direct_sessionless_event_records(self) -> None:
        with tempfile.TemporaryDirectory(prefix="riviu-agent-overlay-") as temp:
            output = Path(temp) / "source"
            self.prepare_real_source(output)
            element_commands = (
                output
                / "WebDriverAgentLib"
                / "Commands"
                / "FBElementCommands.m"
            ).read_text(encoding="utf-8")
            route_tests = (
                output
                / "WebDriverAgentTests"
                / "UnitTests"
                / "FBRouteTests.m"
            ).read_text(encoding="utf-8")

            self.assertIn(
                '[[FBRoute POST:@"/wda/tap"].withoutSession '
                'respondWithTarget:self action:@selector(handleRiviuNativeTap:)]',
                element_commands,
            )
            self.assertIn(
                '[[FBRoute POST:@"/wda/swipe"].withoutSession '
                'respondWithTarget:self action:@selector(handleRiviuNativeSwipe:)]',
                element_commands,
            )
            handler_start = element_commands.index(
                "+ (id<FBResponsePayload>)handleRiviuNativeTap:"
            )
            handler_end = element_commands.index(
                "+ (id<FBResponsePayload>)handlePinch:", handler_start
            )
            handlers = element_commands[handler_start:handler_end]
            for required in (
                "XCPointerEventPath",
                "XCSynthesizedEventRecord",
                "FBXCTestDaemonsProxy",
                "initForTouchAtPoint",
                "moveToPoint",
                "liftUpAtOffset",
                "synthesizeEventWithRecord",
                "riviuInterfaceOrientation",
                "timeout:5.0",
            ):
                self.assertIn(required, handlers)
            self.assertIn("isfinite", element_commands)
            self.assertIn(
                "[arguments isKindOfClass:NSDictionary.class]", element_commands
            )
            for forbidden in (
                "/actions",
                "XCUICoordinate",
                "pressForDuration:thenDragToCoordinate:",
                "fb_waitUntilStable",
                "fb_activeApplication",
            ):
                self.assertNotIn(forbidden, handlers)
            self.assertIn("testRiviuNativeGestureRoutesDoNotRequireSession", route_tests)
            self.assertIn("testRiviuNativeInputNumberValidation", route_tests)
            self.assertIn("testRiviuNativeInputRequiresAnExactDictionaryBody", route_tests)

            proxy_header = (
                output / "WebDriverAgentLib" / "Utilities" / "FBXCTestDaemonsProxy.h"
            ).read_text(encoding="utf-8")
            proxy_source = (
                output / "WebDriverAgentLib" / "Utilities" / "FBXCTestDaemonsProxy.m"
            ).read_text(encoding="utf-8")
            self.assertIn("timeout:(NSTimeInterval)timeout", proxy_header)
            self.assertIn("eventSucceeded = result", proxy_source)
            self.assertIn("spinUntilTrue", proxy_source)
            self.assertIn("Event synthesis returned an unsuccessful result", proxy_source)

    def test_stream_overlay_is_loopback_authenticated_and_health_aware(self) -> None:
        with tempfile.TemporaryDirectory(prefix="riviu-agent-overlay-") as temp:
            output = Path(temp) / "source"
            self.prepare_real_source(output)
            tcp_header = (
                output / "WebDriverAgentLib" / "Routing" / "FBTCPSocket.h"
            ).read_text(encoding="utf-8")
            tcp_source = (
                output / "WebDriverAgentLib" / "Routing" / "FBTCPSocket.m"
            ).read_text(encoding="utf-8")
            mjpeg = (
                output / "WebDriverAgentLib" / "Utilities" / "FBMjpegServer.m"
            ).read_text(encoding="utf-8")
            server = (
                output / "WebDriverAgentLib" / "Routing" / "FBWebServer.m"
            ).read_text(encoding="utf-8")
            session = (
                output / "WebDriverAgentLib" / "Commands" / "FBSessionCommands.m"
            ).read_text(encoding="utf-8")

            self.assertIn("didClient:(GCDAsyncSocket *)client sendData:(NSData *)data", tcp_header)
            self.assertIn('acceptOnInterface:@"localhost"', tcp_source)
            self.assertIn("FBRiviuRequestIsAuthorized", mjpeg)
            self.assertIn("FBRiviuAgentTokenHeaderName", mjpeg)
            self.assertIn("disconnectAfterWriting", mjpeg)
            self.assertIn("FBRiviuSetAgentStreamReady(YES)", server)
            self.assertIn("FBRiviuSetAgentStreamReady(NO)", server)
            self.assertIn("FBRiviuAgentStreamIsReady()", session)

    def test_attestation_overlay_declares_typed_runner_info_plist_values(self) -> None:
        with tempfile.TemporaryDirectory(prefix="riviu-agent-overlay-") as temp:
            output = Path(temp) / "source"
            result = self.prepare_real_source(output)
            info_path = output / "WebDriverAgentRunner" / "Info.plist"
            info = plistlib.loads(info_path.read_bytes())

            expected_patch_count = len(
                json.loads((ROOT / "baseline-lock.json").read_text(encoding="utf-8"))[
                    "patches"
                ]
            )
            self.assertEqual(expected_patch_count, result.patch_count)
            self.assertEqual(
                "$(RIVIU_AGENT_SOURCE_SHA256)", info["RiviuAgentSourceSHA256"]
            )
            self.assertEqual(
                "$(RIVIU_AGENT_XCCONFIG_SHA256)",
                info["RiviuAgentXcconfigSHA256"],
            )
            self.assertIs(type(info["RiviuAgentProtocolVersion"]), int)
            self.assertEqual(2, info["RiviuAgentProtocolVersion"])
            self.assertEqual(
                "$(RIVIU_AGENT_OBJECTIVE_C_UNIT_TESTS)",
                info["RiviuAgentObjectiveCUnitTests"],
            )
            self.assertEqual(
                "$(RIVIU_AGENT_XCODE_VERSION)", info["RiviuAgentXcodeVersion"]
            )
            self.assertEqual(
                "$(RIVIU_AGENT_XCODE_BUILD)", info["RiviuAgentXcodeBuild"]
            )

    def test_clipboard_overlay_enforces_the_exact_runtime_body_schema(self) -> None:
        with tempfile.TemporaryDirectory(prefix="riviu-agent-overlay-") as temp:
            output = Path(temp) / "source"
            self.prepare_real_source(output)
            custom_commands = (
                output / "WebDriverAgentLib" / "Commands" / "FBCustomCommands.m"
            ).read_text(encoding="utf-8")
            route_tests = (
                output / "WebDriverAgentTests" / "UnitTests" / "FBRouteTests.m"
            ).read_text(encoding="utf-8")

            self.assertIn("riviuPasteboardArguments", custom_commands)
            self.assertIn("isSubsetOfSet:allowedKeys", custom_commands)
            self.assertIn("Cannot decode the pasteboard content from base64", custom_commands)
            self.assertIn("testRiviuPasteboardSetRequiresExactSchema", route_tests)
            self.assertIn("testRiviuPasteboardGetRequiresExactSchema", route_tests)


if __name__ == "__main__":
    unittest.main()
