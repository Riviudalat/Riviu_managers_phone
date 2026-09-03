from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import Mock, patch

from scripts import build_desktop_sidecar as sidecar_builder
from scripts import collect_desktop_ci_artifacts as artifacts


def active_dependency_closure() -> dict[str, str]:
    closure: dict[str, str] = {}
    for raw_line in artifacts.SIDECAR_REQUIREMENTS_LOCK.read_text(
        encoding="utf-8"
    ).splitlines():
        line = raw_line.strip()
        if not line or line.startswith("#"):
            continue
        requirement = artifacts.Requirement(line)
        if requirement.marker is not None and not requirement.marker.evaluate(
            environment=artifacts.release_marker_environment()
        ):
            continue
        specifiers = list(requirement.specifier)
        if len(specifiers) != 1 or specifiers[0].operator != "==":
            raise AssertionError(f"test requires an exact dependency lock: {line}")
        closure[artifacts.canonicalize_name(requirement.name)] = specifiers[0].version
    return closure


class ArtifactContractTests(unittest.TestCase):
    def test_windows_collection_requires_staged_android_package_tools(self):
        with self.assertRaisesRegex(
            artifacts.ArtifactError, "Windows collection requires"
        ):
            artifacts.android_package_tools_for_target(
                "x86_64-pc-windows-msvc", None
            )

        self.assertIsNone(
            artifacts.android_package_tools_for_target("aarch64-apple-darwin", None)
        )

    def test_windows_workflow_stages_tools_into_both_installers_and_collector(self):
        workflow = (
            artifacts.REPOSITORY_ROOT / ".github" / "workflows" / "desktop-ci-cd.yml"
        ).read_text(encoding="utf-8")
        self.assertIn("Stage pinned Android package tools", workflow)
        self.assertIn("if: runner.os == 'Windows'", workflow)
        self.assertGreaterEqual(
            workflow.count("target/tauri-android-package-tools.conf.json"), 4
        )
        self.assertIn(
            "android_package_tools_args=(--android-package-tools target/android-package-tools)",
            workflow,
        )

    def test_expected_database_version_tracks_the_latest_migration(self):
        source = (
            artifacts.REPOSITORY_ROOT / "crates" / "core" / "src" / "db" / "migrations.rs"
        ).read_text(encoding="utf-8")
        versions = [int(value) for value in artifacts.re.findall(r"version:\s*(\d+),", source)]
        self.assertEqual(artifacts.EXPECTED_DATABASE_VERSION, max(versions))

    def test_android_package_tools_verify_complete_tree_and_pins(self):
        source = artifacts.REPOSITORY_ROOT / "target" / "android-package-tools"
        if not source.is_dir():
            self.skipTest("local staged package tools are not present")
        evidence = artifacts.verify_android_package_tools(source)
        self.assertEqual(evidence["bundletoolVersion"], "1.18.3")
        self.assertEqual(evidence["jreVersion"], "21.0.12.1+1")
        self.assertGreater(evidence["fileCount"], 100)
        self.assertEqual(
            evidence["treeSha256"], artifacts.ANDROID_PACKAGE_TOOLS_TREE_SHA256
        )

    def test_android_package_tools_reject_an_extra_unmanifested_file(self):
        source = artifacts.REPOSITORY_ROOT / "target" / "android-package-tools"
        if not source.is_dir():
            self.skipTest("local staged package tools are not present")
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary) / "tools"
            artifacts.shutil.copytree(source, root)
            (root / "extra.dll").write_bytes(b"extra")
            with self.assertRaisesRegex(artifacts.ArtifactError, "manifest is incomplete"):
                artifacts.verify_android_package_tools(root, execute=False)
            (root / "extra.dll").unlink()
            manifest_path = root / artifacts.ANDROID_PACKAGE_TOOLS_MANIFEST_NAME
            manifest = artifacts.load_json(manifest_path)
            manifest["treeSha256"] = "0" * 64
            manifest_path.write_text(json.dumps(manifest), encoding="utf-8")
            with self.assertRaisesRegex(artifacts.ArtifactError, "tree attestation"):
                artifacts.verify_android_package_tools(root, execute=False)

    @staticmethod
    def _materialize_windows_install(command: list[str]) -> Path:
        if command[0] == "msiexec.exe":
            install_root = Path(
                next(value for value in command if value.startswith("INSTALLDIR=")).split(
                    "=", 1
                )[1]
            )
        else:
            install_root = Path(
                next(value for value in command if value.startswith("/D=")).split(
                    "=", 1
                )[1]
            )
        sidecars = install_root / "sidecars" / "pymobiledevice3"
        sidecars.mkdir(parents=True)
        (sidecars / "riviu_pmd.py").write_text("fixture", encoding="utf-8")
        (install_root / "riviu-deployment-check.exe").write_bytes(b"MZ")
        return install_root

    def test_msi_verify_failure_still_uninstalls_and_removes_the_install_root(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            msi = root / "Riviu.msi"
            nsis = root / "Riviu-setup.exe"
            msi.write_bytes(b"msi")
            nsis.write_bytes(b"nsis")
            commands: list[list[str]] = []
            installed_root: Path | None = None

            def fake_run(command: list[str], *, timeout: int = 120):
                nonlocal installed_root
                commands.append(command)
                if command[0] == "msiexec.exe" and "/i" in command:
                    installed_root = self._materialize_windows_install(command)
                elif command[0] == "msiexec.exe" and "/x" in command:
                    assert installed_root is not None
                    artifacts.shutil.rmtree(installed_root)
                return subprocess.CompletedProcess(command, 0, "", "")

            with (
                patch.object(artifacts, "run_checked", side_effect=fake_run),
                patch.object(
                    artifacts,
                    "verify_packaged_resources",
                    side_effect=artifacts.ArtifactError("MSI verification failed"),
                ),
            ):
                with self.assertRaisesRegex(
                    artifacts.ArtifactError, "MSI verification failed"
                ):
                    artifacts.verify_windows_package(
                        [msi, nsis], root / "bundle", root / "runtime", {}
                    )

            self.assertTrue(any("/x" in command for command in commands), commands)
            self.assertIsNotNone(installed_root)
            self.assertFalse(installed_root.exists())

    def test_nsis_verify_failure_still_uninstalls_and_removes_the_install_root(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            msi = root / "Riviu.msi"
            nsis = root / "Riviu-setup.exe"
            msi.write_bytes(b"msi")
            nsis.write_bytes(b"nsis")
            commands: list[list[str]] = []
            nsis_root: Path | None = None

            def fake_run(command: list[str], *, timeout: int = 120):
                nonlocal nsis_root
                commands.append(command)
                if command[0] == "msiexec.exe" and "/i" in command:
                    self._materialize_windows_install(command)
                elif command[0] == "msiexec.exe" and "/x" in command:
                    install_root = Path(
                        next(
                            value
                            for prior in commands
                            for value in prior
                            if value.startswith("INSTALLDIR=")
                        ).split("=", 1)[1]
                    )
                    artifacts.shutil.rmtree(install_root)
                elif command[0] == str(nsis):
                    nsis_root = self._materialize_windows_install(command)
                    (nsis_root / "uninstall.exe").write_bytes(b"MZ")
                elif Path(command[0]).name == "uninstall.exe":
                    assert nsis_root is not None
                    artifacts.shutil.rmtree(nsis_root)
                return subprocess.CompletedProcess(command, 0, "", "")

            with (
                patch.object(artifacts, "run_checked", side_effect=fake_run),
                patch.object(
                    artifacts, "verify_packaged_resources", return_value={"tree": "ok"}
                ),
                patch.object(
                    artifacts,
                    "verify_windows_desktop_executable",
                    return_value={
                        "name": "riviu-managers-phone.exe",
                        "architecture": "x86_64",
                        "bytes": 1,
                        "sha256": "0" * 64,
                    },
                ),
                patch.object(
                    artifacts,
                    "run_installed_deployment_check",
                    side_effect=[
                        {"schemaVersion": 1, "overall": "pass"},
                        artifacts.ArtifactError("NSIS verification failed"),
                    ],
                ),
                patch.object(artifacts, "run_installed_app_smoke"),
            ):
                with self.assertRaisesRegex(
                    artifacts.ArtifactError, "NSIS verification failed"
                ):
                    artifacts.verify_windows_package(
                        [msi, nsis], root / "bundle", root / "runtime", {}
                    )

            self.assertTrue(
                any(Path(command[0]).name == "uninstall.exe" for command in commands),
                commands,
            )
            self.assertIsNotNone(nsis_root)
            self.assertFalse(nsis_root.exists())

    def test_cleanup_failure_is_chained_below_the_primary_failure(self):
        def operation():
            raise artifacts.ArtifactError("primary verification failure")

        def cleanup():
            raise artifacts.ArtifactError("cleanup failure")

        with self.assertRaisesRegex(
            artifacts.ArtifactError, "primary verification failure"
        ) as raised:
            artifacts.run_with_cleanup_preserving_primary(operation, cleanup)

        self.assertIsNotNone(raised.exception.__cause__)
        self.assertIn("cleanup failure", str(raised.exception.__cause__))

    def test_sidecar_build_failure_keeps_bounded_child_diagnostics(self):
        child_error = subprocess.CalledProcessError(
            1,
            ["fixture"],
            output="prefix-" + "x" * 2500,
            stderr="fixture stderr",
        )
        with patch.object(
            sidecar_builder.subprocess, "run", side_effect=child_error
        ):
            with self.assertRaisesRegex(
                RuntimeError, "sidecar build command failed"
            ) as raised:
                sidecar_builder.run_checked(["fixture"])

        message = str(raised.exception)
        self.assertIn("fixture stderr", message)
        self.assertNotIn("prefix-", message)
        self.assertLess(len(message), 4200)

    def test_runtime_overlay_preserves_macos_symlinks_and_keeps_windows_resources(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            runtime = root / "runtime"
            runtime.mkdir()

            windows_overlay = root / "windows.json"
            with patch.object(sidecar_builder.sys, "platform", "win32"):
                sidecar_builder.write_tauri_config(windows_overlay, runtime)
            windows = artifacts.verify_overlay(
                windows_overlay, runtime, "x86_64-pc-windows-msvc"
            )
            self.assertEqual(
                windows["bundle"],
                {
                    "resources": {
                        runtime.resolve().as_posix() + "/": (
                            artifacts.RUNTIME_RESOURCE_DESTINATION
                        )
                    }
                },
            )

            macos_overlay = root / "macos.json"
            with (
                patch.object(sidecar_builder.sys, "platform", "darwin"),
                patch.dict(os.environ, {"APPLE_SIGNING_IDENTITY": "-"}),
            ):
                sidecar_builder.write_tauri_config(macos_overlay, runtime)
            macos = artifacts.verify_overlay(
                macos_overlay, runtime, "aarch64-apple-darwin"
            )
            self.assertEqual(
                macos["bundle"],
                {
                    "macOS": {
                        "files": {
                            artifacts.MACOS_RUNTIME_CONTENTS_DESTINATION: (
                                runtime.resolve().as_posix()
                            )
                        },
                        "signingIdentity": "-",
                    }
                },
            )

    def test_invalid_dmg_plist_preserves_primary_error_when_cleanup_fails(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            bundle_dir = root / "bundle" / "dmg"
            bundle_dir.mkdir(parents=True)
            dmg = bundle_dir / "fixture.dmg"
            dmg.write_bytes(b"fixture")
            dev_entry = "/dev/disk17"
            attachment = subprocess.CompletedProcess(
                args=["hdiutil"], returncode=0, stdout="not-a-plist", stderr=""
            )
            no_attachments = self._hdiutil_info([])
            owned_attachment = self._hdiutil_info(
                [self._dmg_image(dmg, dev_entry)]
            )
            detach_failure = subprocess.CompletedProcess(
                args=["hdiutil"], returncode=1, stdout="", stderr="detach failed"
            )
            with (
                patch.object(artifacts, "find_installers", return_value=[dmg]),
                patch.object(artifacts, "run_checked", return_value=attachment),
                patch.object(
                    artifacts.subprocess,
                    "run",
                    side_effect=[
                        no_attachments,
                        owned_attachment,
                        detach_failure,
                        owned_attachment,
                        detach_failure,
                        owned_attachment,
                        detach_failure,
                        owned_attachment,
                    ],
                ) as system_command,
                patch.object(artifacts.time, "sleep"),
            ):
                with self.assertRaisesRegex(
                    artifacts.ArtifactError, "invalid attachment plist"
                ) as raised:
                    artifacts.verify_macos_package(
                        bundle_dir, "aarch64-apple-darwin", root, {}
                    )

            detach_commands = [
                call.args[0]
                for call in system_command.call_args_list
                if call.args[0][1] == "detach"
            ]
            self.assertEqual(len(detach_commands), 3)
            self.assertEqual(detach_commands[0], ["hdiutil", "detach", dev_entry])
            self.assertEqual(
                detach_commands[-1], ["hdiutil", "detach", "-force", dev_entry]
            )
            self.assertIsInstance(raised.exception.__cause__, artifacts.ArtifactError)
            self.assertIn("failed to detach DMG after 3 attempts", str(raised.exception.__cause__))

    def test_partial_dmg_attach_without_mountpoint_detaches_new_dev_entry(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            bundle_dir = root / "bundle" / "dmg"
            bundle_dir.mkdir(parents=True)
            dmg = bundle_dir / "fixture.dmg"
            dmg.write_bytes(b"fixture")
            mount_point = root / "owned-mount"
            mount_point.mkdir()
            dev_entry = "/dev/disk18"
            no_attachments = self._hdiutil_info([])
            partial_attachment = self._hdiutil_info(
                [
                    self._dmg_image(
                        bundle_dir / "nested" / ".." / dmg.name,
                        dev_entry,
                    )
                ]
            )
            detached = subprocess.CompletedProcess(
                args=["hdiutil"], returncode=0, stdout="", stderr=""
            )
            detached_state = self._hdiutil_info([])
            attach_error = artifacts.ArtifactError("attach command failed")
            with (
                patch.object(artifacts, "find_installers", return_value=[dmg]),
                patch.object(artifacts, "run_checked", side_effect=attach_error),
                patch.object(
                    artifacts.tempfile, "mkdtemp", return_value=str(mount_point)
                ),
                patch.object(
                    artifacts.subprocess,
                    "run",
                    side_effect=[
                        no_attachments,
                        partial_attachment,
                        detached,
                        detached_state,
                    ],
                ) as system_command,
            ):
                with self.assertRaisesRegex(
                    artifacts.ArtifactError, "attach command failed"
                ) as raised:
                    artifacts.verify_macos_package(
                        bundle_dir, "aarch64-apple-darwin", root, {}
                    )

            self.assertIs(raised.exception, attach_error)
            self.assertEqual(system_command.call_count, 4)
            self.assertEqual(
                system_command.call_args_list[0].args[0],
                ["hdiutil", "info", "-plist"],
            )
            self.assertEqual(
                system_command.call_args_list[2].args[0],
                ["hdiutil", "detach", dev_entry],
            )

    def test_dmg_attach_at_different_mountpoint_detaches_by_dev_entry(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            bundle_dir = root / "bundle" / "dmg"
            bundle_dir.mkdir(parents=True)
            dmg = bundle_dir / "fixture.dmg"
            dmg.write_bytes(b"fixture")
            requested_mount = root / "requested-mount"
            requested_mount.mkdir()
            actual_mount = root / "different-mount"
            dev_entry = "/dev/disk19"
            attachment = subprocess.CompletedProcess(
                args=["hdiutil"],
                returncode=0,
                stdout=artifacts.plistlib.dumps(
                    {
                        "system-entities": [
                            {
                                "dev-entry": dev_entry,
                                "mount-point": str(actual_mount),
                            }
                        ]
                    }
                ).decode("utf-8"),
                stderr="",
            )
            no_attachments = self._hdiutil_info([])
            owned_attachment = self._hdiutil_info(
                [self._dmg_image(dmg, dev_entry, actual_mount)]
            )
            detached = subprocess.CompletedProcess(
                args=["hdiutil"], returncode=0, stdout="", stderr=""
            )
            detached_state = self._hdiutil_info([])
            with (
                patch.object(artifacts, "find_installers", return_value=[dmg]),
                patch.object(artifacts, "run_checked", return_value=attachment),
                patch.object(
                    artifacts.tempfile, "mkdtemp", return_value=str(requested_mount)
                ),
                patch.object(
                    artifacts.subprocess,
                    "run",
                    side_effect=[
                        no_attachments,
                        owned_attachment,
                        detached,
                        detached_state,
                    ],
                ) as system_command,
            ):
                with self.assertRaisesRegex(
                    artifacts.ArtifactError, "exact isolated mount point"
                ):
                    artifacts.verify_macos_package(
                        bundle_dir, "aarch64-apple-darwin", root, {}
                    )

            detach_commands = [
                call.args[0]
                for call in system_command.call_args_list
                if call.args[0][1] == "detach"
            ]
            self.assertEqual(detach_commands, [["hdiutil", "detach", dev_entry]])

    def test_failed_dmg_detach_accepts_confirmed_absence(self):
        with tempfile.TemporaryDirectory() as temporary:
            image_path = Path(temporary) / "fixture.dmg"
            image_path.write_bytes(b"fixture")
            attachment = artifacts.DmgAttachment(
                image_path.resolve(), "/dev/disk20"
            )
            absent = self._hdiutil_info([])
            nonzero = subprocess.CompletedProcess(
                args=["hdiutil"], returncode=1, stdout="", stderr="busy"
            )
            for failure in (
                nonzero,
                subprocess.TimeoutExpired(["hdiutil", "detach"], 120),
            ):
                with self.subTest(failure=type(failure).__name__):
                    with (
                        patch.object(
                            artifacts.subprocess,
                            "run",
                            side_effect=[failure, absent],
                        ) as system_command,
                        patch.object(artifacts.time, "sleep") as sleep,
                    ):
                        self.assertIsNone(
                            artifacts.detach_dmg_with_retry(attachment)
                        )

                    self.assertEqual(system_command.call_count, 2)
                    sleep.assert_not_called()

    def test_successful_dmg_detach_retries_until_absence_is_confirmed(self):
        with tempfile.TemporaryDirectory() as temporary:
            image_path = Path(temporary) / "fixture.dmg"
            image_path.write_bytes(b"fixture")
            attachment = artifacts.DmgAttachment(
                image_path.resolve(), "/dev/disk21"
            )
            detached = subprocess.CompletedProcess(
                args=["hdiutil"], returncode=0, stdout="", stderr=""
            )
            present = self._hdiutil_info(
                [self._dmg_image(image_path, attachment.dev_entry)]
            )
            absent = self._hdiutil_info([])
            with (
                patch.object(
                    artifacts.subprocess,
                    "run",
                    side_effect=[detached, present, detached, absent],
                ) as system_command,
                patch.object(artifacts.time, "sleep") as sleep,
            ):
                self.assertIsNone(artifacts.detach_dmg_with_retry(attachment))

            self.assertEqual(system_command.call_count, 4)
            sleep.assert_called_once_with(1)

    @staticmethod
    def _dmg_image(
        image_path: Path,
        dev_entry: str,
        mount_point: Path | None = None,
    ) -> dict[str, object]:
        entity = {"dev-entry": dev_entry}
        if mount_point is not None:
            entity["mount-point"] = str(mount_point)
        return {
            "image-path": str(image_path),
            "system-entities": [entity],
        }

    @staticmethod
    def _hdiutil_info(images: list[dict[str, object]]) -> subprocess.CompletedProcess[str]:
        return subprocess.CompletedProcess(
            args=["hdiutil", "info", "-plist"],
            returncode=0,
            stdout=artifacts.plistlib.dumps({"images": images}).decode("utf-8"),
            stderr="",
        )

    def test_windows_desktop_executable_requires_a_valid_x64_pe(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            executable = root / "riviu-managers-phone.exe"
            payload = bytearray(128)
            payload[:2] = b"MZ"
            payload[0x3C:0x40] = (64).to_bytes(4, "little")
            payload[64:68] = b"PE\0\0"
            payload[68:70] = (0x8664).to_bytes(2, "little")
            executable.write_bytes(payload)

            evidence = artifacts.verify_windows_desktop_executable(
                root, "x86_64-pc-windows-msvc"
            )
            self.assertEqual(evidence["architecture"], "x86_64")
            self.assertEqual(evidence["name"], executable.name)

            payload[68:70] = (0x014C).to_bytes(2, "little")
            executable.write_bytes(payload)
            with self.assertRaisesRegex(artifacts.ArtifactError, "machine mismatch"):
                artifacts.verify_windows_desktop_executable(
                    root, "x86_64-pc-windows-msvc"
                )

    def test_production_manifest_uses_the_canonical_lf_digest(self):
        self.assertEqual(
            artifacts.production_sha256(artifacts.PRODUCTION_MANIFEST),
            artifacts.CANONICAL_PRODUCTION_SHA256[
                "sidecars/wda/agent-manifest.json"
            ],
        )

    def test_canonical_production_snapshot_accepts_the_windows_checkout(self):
        snapshot = artifacts.production_snapshot(require_canonical=True)

        self.assertEqual(snapshot["schemaVersion"], 1)
        self.assertIn(
            "sidecars/wda/agent-manifest.json",
            {entry["path"] for entry in snapshot["files"]},
        )

    def test_duplicate_json_keys_are_rejected(self):
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "duplicate.json"
            path.write_text('{"value": 1, "value": 2}\n', encoding="utf-8")

            with self.assertRaisesRegex(artifacts.ArtifactError, "duplicate JSON key"):
                artifacts.load_json(path)

    def test_desktop_version_contract_matches_and_rejects_a_wrong_tag(self):
        # Asks the repo what the version is instead of naming it. The earlier version
        # asserted "0.1.0" literally, so bumping to 0.1.1 broke a test about tag matching
        # for a reason that had nothing to do with tag matching. What is worth pinning is
        # the property: whatever the four files agree on, that tag is accepted and any
        # other is refused.
        version = artifacts.verify_version_command(argparse.Namespace(tag=None))["version"]
        result = artifacts.verify_version_command(argparse.Namespace(tag=f"v{version}"))
        self.assertEqual(result["version"], version)

        with self.assertRaisesRegex(artifacts.ArtifactError, "tag mismatch"):
            artifacts.verify_version_command(argparse.Namespace(tag="v9.9.9"))

    def test_the_release_overlay_is_inside_the_version_contract(self):
        # The overlay decides what the shipped binary reports at runtime, and it sat outside
        # the check. A mismatch there is invisible until after a release, when latest.json
        # advertises a version no installed copy can ever claim to have reached.
        overlay = artifacts.load_json(artifacts.TAURI_FULL_CONFIG)
        version = artifacts.verify_version_command(argparse.Namespace(tag=None))["version"]

        self.assertEqual(overlay.get("version"), version)

    def test_source_commit_requires_an_exact_git_object_id(self):
        commit = "A" * 40
        self.assertEqual(artifacts.validate_source_commit(commit), "a" * 40)

        for invalid in ("a" * 39, "a" * 41, "g" * 40, "refs/heads/main"):
            with self.subTest(invalid=invalid):
                with self.assertRaisesRegex(
                    artifacts.ArtifactError, "exact 40-character Git SHA"
                ):
                    artifacts.validate_source_commit(invalid)

    def test_tree_attestation_includes_symlink_target_when_supported(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / "first").write_bytes(b"same")
            (root / "second").write_bytes(b"same")
            link = root / "selected"
            try:
                os.symlink("first", link)
            except OSError as error:
                self.skipTest(f"symlink creation unavailable: {error}")
            first = artifacts.tree_attestation(root)
            link.unlink()
            os.symlink("second", link)
            second = artifacts.tree_attestation(root)

            self.assertNotEqual(first["treeSha256"], second["treeSha256"])

    def test_runtime_tree_is_recomputed_instead_of_trusting_the_manifest(self):
        with tempfile.TemporaryDirectory() as temporary:
            runtime = Path(temporary)
            entrypoint = runtime / "riviu-pmd.exe"
            entrypoint.write_bytes(b"fixture-runtime")
            digest = artifacts.sha256_file(entrypoint)
            relative = entrypoint.relative_to(runtime).as_posix()
            measured = artifacts.tree_attestation(runtime)
            source_lock = artifacts.load_json(artifacts.LEGACY_WDA_SOURCE_LOCK)
            manifest = {
                "schemaVersion": 1,
                "kind": "pyinstaller-onedir",
                "platform": "windows",
                "architecture": "x86_64",
                "pythonVersion": "3.12.10",
                "dependencies": {
                    "pymobiledevice3": "10.1.0",
                    "tidevice": "0.12.11",
                    "pyinstaller": "6.21.0",
                },
                "dependencyClosure": active_dependency_closure(),
                "entrypoint": relative,
                "entrypointSha256": digest,
                "sourceSha256": artifacts.sha256_file(artifacts.SIDECAR_SOURCE),
                "requirementsSha256": artifacts.sha256_file(
                    artifacts.SIDECAR_REQUIREMENTS
                ),
                "buildRequirementsSha256": artifacts.sha256_file(
                    artifacts.SIDECAR_BUILD_REQUIREMENTS
                ),
                "requirementsLockSha256": artifacts.sha256_file(
                    artifacts.SIDECAR_REQUIREMENTS_LOCK
                ),
                "runtimeHookSha256": artifacts.sha256_file(
                    artifacts.SIDECAR_RUNTIME_HOOK
                ),
                "signerSourceSha256": artifacts.sha256_file(
                    artifacts.SIGNER_SOURCE
                ),
                "buildInstallSourceSha256": artifacts.sha256_file(
                    artifacts.BUILD_INSTALL_SOURCE
                ),
                "legacyWdaSourceLockSha256": artifacts.sha256_file(
                    artifacts.LEGACY_WDA_SOURCE_LOCK
                ),
                "signingResources": {
                    "sourceVersion": source_lock["version"],
                    "sourceTreeSha256": source_lock["treeSha256"],
                    "logoSha256": source_lock["logoSha256"],
                    "iconSetTreeSha256": source_lock["iconSetTreeSha256"],
                    "workspaceOutsideResources": True,
                },
                **measured,
                "smoke": {
                    "ping": "PASS",
                    "embeddedTidevice": "PASS",
                    "embeddedSigner": "PASS",
                    "embeddedSigningResources": "PASS",
                    "embeddedSignerErrorJson": "PASS",
                },
            }
            (runtime / "runtime-manifest.json").write_text(
                json.dumps(manifest), encoding="utf-8"
            )

            verified = artifacts.verify_runtime(
                runtime, "x86_64-pc-windows-msvc"
            )
            self.assertEqual(verified["treeSha256"], measured["treeSha256"])

            manifest["pythonVersion"] = "3.12.11"
            (runtime / "runtime-manifest.json").write_text(
                json.dumps(manifest), encoding="utf-8"
            )
            with self.assertRaisesRegex(
                artifacts.ArtifactError, "expected exact"
            ):
                artifacts.verify_runtime(runtime, "x86_64-pc-windows-msvc")

            manifest["pythonVersion"] = artifacts.EXPECTED_RELEASE_PYTHON_VERSION
            (runtime / "runtime-manifest.json").write_text(
                json.dumps(manifest), encoding="utf-8"
            )

            entrypoint.write_bytes(b"tampered-runtime")
            with self.assertRaisesRegex(
                artifacts.ArtifactError, "entrypoint SHA-256"
            ):
                artifacts.verify_runtime(runtime, "x86_64-pc-windows-msvc")

    def test_msi_log_decodes_utf16_and_reports_the_failing_lines(self):
        # msiexec writes UTF-16 and buries the cause thousands of lines above
        # the exit, so a byte tail shows only the shutdown sequence.
        noise = "\n".join(f"MSI (s): Property change {index}" for index in range(400))
        log = (
            "=== Verbose logging started ===\n"
            + noise
            + "\nMSI (s): Note: 1: 2262 2: Error 3: -2147287038\n"
            + "MSI (s): Error 1310: Error writing to file C:\\nope\\riviu.exe\n"
            + noise
            + "\nMSI (c): MainEngineThread is returning 1603\n"
            "=== Verbose logging stopped ===\n"
        )
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "msiexec.log"
            path.write_text(log, encoding="utf-16")

            summary = artifacts.read_msi_log(path)

        self.assertIn("Error 1310", summary)
        self.assertIn("returning 1603", summary)
        # Decoding must not leave UTF-16 padding behind.
        self.assertNotIn("\x00", summary)
        # Property-change chatter is not what a reader needs.
        self.assertNotIn("Property change 200", summary)

    def test_shallow_temp_dir_stays_far_from_the_max_path_ceiling(self):
        # The deepest payload entry is ~135 characters of relative path, so the
        # scratch root has to leave room for it under Windows' 260-char limit.
        deep = Path(tempfile.gettempdir()) / ("nested" * 12)
        with artifacts.shallow_temp_dir("rv-test-", deep) as temporary:
            self.assertTrue(temporary.is_dir())
            self.assertLess(len(str(temporary)), 60)
            created = temporary

        self.assertFalse(created.exists())

    def test_shallow_temp_dir_falls_back_when_the_drive_root_is_closed(self):
        real_mkdtemp = tempfile.mkdtemp

        def deny_drive_root(*args, **kwargs):
            # Refuse only the drive-root attempt; TemporaryDirectory needs the
            # real mkdtemp for the fallback to work at all.
            directory = kwargs.get("dir")
            if directory is not None and Path(directory) == Path(Path(directory).anchor):
                raise OSError("denied")
            return real_mkdtemp(*args, **kwargs)

        with tempfile.TemporaryDirectory() as parent:
            fallback_parent = Path(parent) / "fallback"
            with patch.object(artifacts.tempfile, "mkdtemp", side_effect=deny_drive_root):
                with artifacts.shallow_temp_dir("rv-test-", fallback_parent) as temporary:
                    self.assertTrue(temporary.is_dir())
                    # The fallback must still be usable, not silently skipped.
                    self.assertTrue(
                        str(temporary).startswith(str(fallback_parent))
                    )

    def test_every_job_running_the_collector_installs_what_it_imports(self):
        """Catch the class, not the instance.

        The release job runs this very script and installed nothing, so it died on
        `ModuleNotFoundError: packaging`. It only runs on a tag push, so nothing exercised
        it until the first tag in the repository — the failure mode is a job that is never
        rehearsed. Asserting the structure is the only thing that runs on every push.
        """
        workflow = (
            artifacts.REPOSITORY_ROOT / ".github" / "workflows" / "desktop-ci-cd.yml"
        ).read_text(encoding="utf-8")
        script = "collect_desktop_ci_artifacts.py"

        # Split into jobs by their two-space-indented keys, so "runs the script" and
        # "installs a dependency" are compared within one job rather than across the file.
        # The `on:` triggers land in here as pseudo-jobs, which is harmless: they never
        # mention the script, so they can never be offenders. Verified by deleting the fix
        # and watching this report exactly `['release']`.
        jobs: dict[str, list[str]] = {}
        current: str | None = None
        for line in workflow.splitlines():
            if line and not line.startswith(" ") and not line.startswith("#"):
                current = None
            elif line.startswith("  ") and not line.startswith("   ") and line.rstrip().endswith(":"):
                current = line.strip().rstrip(":")
                jobs[current] = []
            elif current is not None:
                jobs[current].append(line)

        offenders = []
        for name, body in jobs.items():
            text = "\n".join(body)
            if script in text and "pip install" not in text:
                offenders.append(name)

        self.assertEqual(
            offenders,
            [],
            f"job(s) {offenders!r} run {script} without installing its dependencies",
        )

    def test_the_packaging_pin_the_release_job_reads_is_really_in_the_lock(self):
        # The release job greps the pin out of the lock instead of repeating it. If the
        # lock ever stops naming packaging that way, the grep yields nothing and pip is
        # called with an empty argument — so the shape of that line is worth pinning.
        lock = artifacts.SIDECAR_REQUIREMENTS_LOCK.read_text(encoding="utf-8")
        pins = [
            line
            for line in lock.splitlines()
            if line.startswith("packaging==")
        ]

        self.assertEqual(len(pins), 1, pins)

    def test_msi_log_absence_is_reported_rather_than_raising(self):
        with tempfile.TemporaryDirectory() as temporary:
            summary = artifacts.read_msi_log(Path(temporary) / "missing.log")

        self.assertIn("unreadable", summary)


def fake_signature(comment: str = "signature from tauri secret key") -> str:
    import base64

    return base64.b64encode(
        f"untrusted comment: {comment}\nRWRaNhHELnCx0000\n".encode("ascii")
    ).decode("ascii")


class UpdaterManifestTests(unittest.TestCase):
    """The `latest.json` contract, which nothing on this machine can retry once shipped."""

    def test_a_name_with_a_space_is_renamed_the_way_github_would_rename_it(self):
        self.assertEqual(
            artifacts.release_asset_name("windows-x64--Riviu Full_0.1.0-setup.exe"),
            "windows-x64--Riviu.Full_0.1.0-setup.exe",
        )

    def test_a_name_github_would_leave_alone_is_returned_unchanged(self):
        for name in (
            "windows-x64--Riviu_0.1.0_x64-setup.exe",
            "macos-arm64--Riviu.app.tar.gz.sig",
        ):
            self.assertEqual(artifacts.release_asset_name(name), name)

    def test_a_name_that_would_lose_a_leading_period_is_refused_not_trimmed(self):
        # Trimming would put back the very mismatch this function exists to remove.
        with self.assertRaises(artifacts.ArtifactError):
            artifacts.release_asset_name(" leading-space.exe")

    def test_a_signature_that_is_not_base64_is_refused(self):
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "x.sig"
            path.write_text("untrusted comment: raw minisign\n", encoding="ascii")
            with self.assertRaises(artifacts.ArtifactError):
                artifacts.read_updater_signature(path)

    def test_base64_that_is_not_minisign_is_refused(self):
        import base64

        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "x.sig"
            path.write_text(
                base64.b64encode(b"hello there").decode("ascii"), encoding="ascii"
            )
            with self.assertRaises(artifacts.ArtifactError):
                artifacts.read_updater_signature(path)

    def test_an_archive_without_a_signature_beside_it_fails_the_release(self):
        with tempfile.TemporaryDirectory() as temporary:
            bundle = Path(temporary) / "bundle" / "nsis"
            bundle.mkdir(parents=True)
            (bundle / "Riviu_0.1.0_x64-setup.exe").write_bytes(b"installer")
            with self.assertRaises(artifacts.ArtifactError) as caught:
                artifacts.find_updater_artifacts(
                    Path(temporary) / "bundle", "x86_64-pc-windows-msvc"
                )

        # The message has to name both suspects; the operator cannot see the runner.
        self.assertIn("createUpdaterArtifacts", str(caught.exception))
        self.assertIn("TAURI_SIGNING_PRIVATE_KEY", str(caught.exception))

    def test_two_candidate_archives_refuse_rather_than_pick_one(self):
        with tempfile.TemporaryDirectory() as temporary:
            bundle = Path(temporary) / "bundle"
            (bundle / "nsis").mkdir(parents=True)
            (bundle / "other").mkdir(parents=True)
            for relative in ("nsis/a-setup.exe", "other/b-setup.exe"):
                (bundle / relative).write_bytes(b"installer")
                (bundle / f"{relative}.sig").write_text(
                    fake_signature(), encoding="ascii"
                )
            with self.assertRaises(artifacts.ArtifactError):
                artifacts.find_updater_artifacts(bundle, "x86_64-pc-windows-msvc")

    def write_label(
        self,
        root: Path,
        label: str,
        *,
        archives: dict[str, str],
        signature: str | None = None,
        platform_override: tuple[str, str] | None = None,
    ) -> Path:
        """Lay out one label's release directory the way `collect` would.

        `archives` maps each platform key to the asset name serving it, so the Windows
        case can share one file across two keys exactly as the real collector does.
        """
        target = artifacts.RELEASE_LABEL_TARGETS[label]
        directory = root / f"desktop-{label}"
        directory.mkdir(parents=True, exist_ok=True)
        signature_text = fake_signature() if signature is None else signature
        entries = []
        for key, _pattern in artifacts.UPDATER_ARCHIVES[target]:
            archive = archives[key]
            path = directory / archive
            if not path.exists():
                path.write_bytes(archive.encode("utf-8"))
            signature_name = f"{archive}.sig"
            (directory / signature_name).write_text(signature_text, encoding="ascii")
            entries.append(
                {
                    "platform": key,
                    "archive": archive,
                    "archiveBytes": path.stat().st_size,
                    "archiveSha256": artifacts.sha256_file(path),
                    "signatureFile": signature_name,
                    "signature": signature_text,
                }
            )
        if platform_override is not None:
            index, value = platform_override
            entries[int(index)]["platform"] = value
        manifest_name = f"{label}-artifact-manifest.json"
        manifest = {
            "schemaVersion": 2,
            "label": label,
            "target": target,
            "updater": entries,
        }
        artifacts.write_json(directory / manifest_name, manifest)
        names = sorted(
            path.name
            for path in directory.iterdir()
            if path.is_file() and not path.name.endswith("SHA256SUMS")
        )
        (directory / f"{label}-SHA256SUMS").write_text(
            "".join(
                f"{artifacts.sha256_file(directory / name)}  {name}\n" for name in names
            ),
            encoding="ascii",
            newline="\n",
        )
        return directory

    WINDOWS_ARCHIVES = {
        "windows-x86_64-nsis": "windows-x64--Riviu-setup.exe",
        "windows-x86_64-msi": "windows-x64--Riviu_en-US.msi",
        "windows-x86_64": "windows-x64--Riviu-setup.exe",
    }

    def complete_release(self, root: Path) -> None:
        self.write_label(root, "windows-x64", archives=dict(self.WINDOWS_ARCHIVES))
        self.write_label(
            root,
            "macos-arm64",
            archives={"darwin-aarch64": "macos-arm64--Riviu.app.tar.gz"},
        )
        self.write_label(
            root, "macos-x64", archives={"darwin-x86_64": "macos-x64--Riviu.app.tar.gz"}
        )

    def build(self, root: Path, output: Path, **overrides):
        arguments = {
            "root": root,
            "labels": ["windows-x64", "macos-arm64", "macos-x64"],
            "repo": "Riviudalat/Riviu_managers_phone",
            "tag": "v0.1.1",
            "version": "0.1.1",
            "output": output,
            "notes": None,
            "pub_date": "2026-08-13T00:00:00Z",
        }
        arguments.update(overrides)
        return artifacts.build_updater_manifest_command(
            argparse.Namespace(**arguments)
        )

    def test_every_platform_url_is_pinned_to_the_tag_not_to_latest(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            self.complete_release(root)
            output = root / "out" / "latest.json"
            self.build(root, output)
            manifest = json.loads(output.read_text(encoding="utf-8"))

        self.assertEqual(manifest["version"], "0.1.1")
        self.assertEqual(
            sorted(manifest["platforms"]),
            [
                "darwin-aarch64",
                "darwin-x86_64",
                "windows-x86_64",
                "windows-x86_64-msi",
                "windows-x86_64-nsis",
            ],
        )
        for platform, entry in manifest["platforms"].items():
            # A client that already holds the manifest must reach an immutable asset,
            # so every URL names the tag. Only the endpoint itself may say "latest".
            self.assertIn("/releases/download/v0.1.1/", entry["url"], platform)
            self.assertNotIn("/latest/", entry["url"])
            self.assertTrue(entry["signature"])

    def test_windows_points_at_the_nsis_installer_it_can_actually_apply(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            self.complete_release(root)
            output = root / "out" / "latest.json"
            self.build(root, output)
            manifest = json.loads(output.read_text(encoding="utf-8"))

        # The bare key is the answer for a bundle type the plugin cannot identify, and it
        # points at NSIS because installMode: currentUser lets that one run without UAC.
        self.assertTrue(
            manifest["platforms"]["windows-x86_64"]["url"].endswith("-setup.exe")
        )
        self.assertTrue(
            manifest["platforms"]["windows-x86_64-nsis"]["url"].endswith("-setup.exe")
        )

    def test_an_msi_install_updates_with_an_msi_and_not_with_the_nsis_build(self):
        # The plugin looks up {os}-{arch}-{installer} and falls back to {os}-{arch}. With
        # only the bare key, an MSI-installed copy falls through and installs the NSIS build
        # over itself: one app, two uninstall entries, two registry identities. So the -msi
        # key has to exist and has to point at the MSI.
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            self.complete_release(root)
            output = root / "out" / "latest.json"
            self.build(root, output)
            manifest = json.loads(output.read_text(encoding="utf-8"))

        self.assertTrue(
            manifest["platforms"]["windows-x86_64-msi"]["url"].endswith(".msi"),
            manifest["platforms"]["windows-x86_64-msi"]["url"],
        )

    def test_every_key_the_plugin_could_ask_for_is_present(self):
        # Mirrors the plugin's own lookup rather than a list written here: whatever keys
        # UPDATER_ARCHIVES declares, the manifest must serve all of them.
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            self.complete_release(root)
            output = root / "out" / "latest.json"
            self.build(root, output)
            manifest = json.loads(output.read_text(encoding="utf-8"))

        self.assertEqual(set(manifest["platforms"]), set(artifacts.REQUIRED_UPDATER_KEYS))

    def test_a_missing_platform_fails_instead_of_shipping_a_partial_manifest(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            self.write_label(root, "windows-x64", archives=dict(self.WINDOWS_ARCHIVES))
            with self.assertRaises(artifacts.ArtifactError) as caught:
                self.build(root, root / "out" / "latest.json", labels=["windows-x64"])

        self.assertIn("darwin-aarch64", str(caught.exception))

    def test_a_tag_that_disagrees_with_the_version_is_refused(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            self.complete_release(root)
            with self.assertRaises(artifacts.ArtifactError):
                self.build(root, root / "out" / "latest.json", tag="v0.1.2")

    def test_a_signature_edited_after_collection_is_caught_before_upload(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            self.complete_release(root)
            signature = root / "desktop-windows-x64" / "windows-x64--Riviu-setup.exe.sig"
            signature.write_text(fake_signature("tampered"), encoding="ascii")
            with self.assertRaises(artifacts.ArtifactError) as caught:
                self.build(root, root / "out" / "latest.json")

        self.assertIn("differs from", str(caught.exception))

    def test_an_archive_whose_bytes_changed_is_caught_before_upload(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            self.complete_release(root)
            archive = root / "desktop-macos-arm64" / "macos-arm64--Riviu.app.tar.gz"
            archive.write_bytes(b"a different archive entirely")
            with self.assertRaises(artifacts.ArtifactError) as caught:
                self.build(root, root / "out" / "latest.json")

        self.assertIn("does not match asset", str(caught.exception))

    def test_an_asset_name_github_would_rewrite_is_refused_before_it_becomes_a_url(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            self.complete_release(root)
            self.write_label(
                root,
                "windows-x64",
                archives={
                    "windows-x86_64-nsis": "windows-x64--Riviu Full-setup.exe",
                    "windows-x86_64-msi": "windows-x64--Riviu_en-US.msi",
                    "windows-x86_64": "windows-x64--Riviu Full-setup.exe",
                },
            )
            with self.assertRaises(artifacts.ArtifactError) as caught:
                self.build(root, root / "out" / "latest.json")

        self.assertIn("renamed by GitHub", str(caught.exception))

    def test_a_label_claiming_the_wrong_platform_is_refused(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            self.complete_release(root)
            self.write_label(
                root,
                "macos-x64",
                archives={"darwin-x86_64": "macos-x64--Riviu.app.tar.gz"},
                platform_override=("0", "darwin-aarch64"),
            )
            with self.assertRaises(artifacts.ArtifactError) as caught:
                self.build(root, root / "out" / "latest.json")

        self.assertIn("platform keys mismatch", str(caught.exception))

    def test_an_updater_asset_outside_the_checksum_file_is_refused(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            self.complete_release(root)
            directory = root / "desktop-windows-x64"
            checksums = directory / "windows-x64-SHA256SUMS"
            kept = [
                line
                for line in checksums.read_text(encoding="ascii").splitlines(True)
                if not line.endswith("-setup.exe.sig\n")
            ]
            checksums.write_text("".join(kept), encoding="ascii", newline="\n")
            with self.assertRaises(artifacts.ArtifactError) as caught:
                self.build(root, root / "out" / "latest.json")

        self.assertIn("not a checksummed release asset", str(caught.exception))



class DeclaredResourcesAreVerified(unittest.TestCase):
    """The drift that failed all three platform builds, caught in a second instead.

    `assert_every_sidecar_resource_is_verified` already existed and already worked -- it is what
    found the problem. But it only runs inside a full bundle build, so the answer arrived from
    CI twenty minutes after the push. This is the same comparison against the same two sources
    of truth, with no bundle needed.
    """

    def test_every_declared_resource_is_named(self):
        artifacts.assert_every_sidecar_resource_is_verified(artifacts.VERIFIED_SIDECAR_TARGETS)

    def test_the_list_does_not_name_resources_the_config_stopped_shipping(self):
        """The other direction: a stale entry is a check nobody performs any more.

        Not fatal the way a missing one is -- an extra name cannot let something ship
        unverified -- but it makes the list read as covering more than it does.
        """
        resources = artifacts.load_json(artifacts.TAURI_CONFIG).get("bundle", {}).get("resources", {})
        declared = {
            target.rstrip("/")
            for target in resources.values()
            if isinstance(target, str) and target.startswith("sidecars/")
        }
        stale = sorted(artifacts.VERIFIED_SIDECAR_TARGETS - declared)
        self.assertEqual(
            stale, [], "these are named as verified but no longer declared in bundle.resources"
        )

    def test_windows_bundle_declares_the_deployment_checker_and_notice(self):
        config = artifacts.load_json(artifacts.TAURI_FULL_CONFIG)
        bundle = config.get("bundle", {})

        self.assertEqual(
            bundle.get("externalBin"),
            ["binaries/riviu-deployment-check"],
        )
        self.assertEqual(
            artifacts.load_json(artifacts.TAURI_CONFIG)["bundle"]["resources"].get(
                "../../../NOTICE"
            ),
            "NOTICE",
        )

    def test_windows_installers_are_scoped_to_the_current_user(self):
        config = artifacts.load_json(artifacts.TAURI_CONFIG)
        windows = config["bundle"]["windows"]
        self.assertEqual(windows["nsis"]["installMode"], "currentUser")

        template_path = artifacts.TAURI_CONFIG.parent / windows["wix"]["template"]
        template = template_path.read_text(encoding="utf-8")
        self.assertIn('InstallScope="perUser"', template)
        self.assertIn('InstallPrivileges="limited"', template)
        self.assertIn('<Directory Id="LocalAppDataFolder">', template)
        self.assertNotIn('InstallScope="perMachine"', template)
        self.assertNotIn('Root="HKLM" Key="Software\\Classes', template)

    def test_windows_msi_is_installed_not_administratively_extracted(self):
        command = artifacts.windows_msi_install_command(
            Path("Riviu.msi"), Path(r"C:\clean\Riviu"), Path("install.log")
        )

        self.assertIn("/i", command)
        self.assertNotIn("/a", command)
        self.assertIn(r"INSTALLDIR=C:\clean\Riviu", command)

    def test_deployment_checker_command_is_headless_and_records_the_installer(self):
        command = artifacts.deployment_checker_command(
            Path("riviu-deployment-check.exe"),
            Path("report.json"),
            Path("Riviu.msi"),
            "production",
        )

        self.assertEqual(
            command,
            [
                "riviu-deployment-check.exe",
                "--profile",
                "production",
                "--report",
                "report.json",
                "--installer",
                "Riviu.msi",
            ],
        )

    def test_deployment_report_is_bound_to_profile_installer_and_installed_files(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            install_root = root / "installed"
            install_root.mkdir()
            installer = root / "Riviu.msi"
            checker = install_root / "riviu-deployment-check.exe"
            app = install_root / "riviu-managers-phone.exe"
            installer.write_bytes(b"installer")
            checker.write_bytes(b"checker")
            app.write_bytes(b"app")
            payload = {
                "schemaVersion": 1,
                "profile": "internal",
                "overall": "pass",
                "adbOrigin": "Bundled",
                "installDir": str(install_root),
                "installerPath": str(installer),
                "installerSha256": artifacts.sha256_file(installer),
                "checkerSha256": artifacts.sha256_file(checker),
                "appSha256": artifacts.sha256_file(app),
            }
            artifacts.validate_deployment_report_binding(
                payload, install_root, installer, checker, app, "internal"
            )

            corruptions = {
                "profile": "production",
                "installDir": str(root / "other-install"),
                "installerPath": str(root / "other.msi"),
                "installerSha256": "0" * 64,
                "checkerSha256": "1" * 64,
                "appSha256": "2" * 64,
            }
            for field, wrong_value in corruptions.items():
                with self.subTest(field=field):
                    bad = dict(payload)
                    bad[field] = wrong_value
                    with self.assertRaises(artifacts.ArtifactError):
                        artifacts.validate_deployment_report_binding(
                            bad, install_root, installer, checker, app, "internal"
                        )

    def test_installed_tree_provenance_scan_reads_extracted_payloads(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            payload = root / "renamed-helper.bin"
            payload.write_bytes(b"connect api.xiaowei.xin")
            with self.assertRaisesRegex(artifacts.ArtifactError, "clean-room"):
                artifacts.verify_clean_room_installed_tree(root)
            payload.write_bytes(b"Riviu packaged fixture")
            artifacts.verify_clean_room_installed_tree(root)

    def test_direct_script_entrypoint_can_load_the_provenance_gate(self):
        script = artifacts.REPOSITORY_ROOT / "scripts" / "collect_desktop_ci_artifacts.py"
        completed = subprocess.run(
            [sys.executable, str(script), "verify-version"],
            cwd=artifacts.REPOSITORY_ROOT,
            capture_output=True,
            text=True,
            timeout=30,
            check=False,
        )
        self.assertEqual(completed.returncode, 0, completed.stderr)

    def test_installed_app_smoke_uses_mock_data_and_exits_by_itself(self):
        command = artifacts.installed_app_smoke_command(
            Path("riviu-managers-phone.exe"),
            Path("startup.json"),
            Path(r"C:\clean\data"),
        )

        self.assertEqual(
            command,
            [
                "riviu-managers-phone.exe",
                "--deployment-smoke",
                "startup.json",
                "--data-dir",
                r"C:\clean\data",
            ],
        )

    def test_installed_app_smoke_requires_tauri_and_frontend_readiness(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            report = root / "startup.json"
            data_dir = root / "data"
            with (
                patch.object(
                    artifacts,
                    "verify_windows_desktop_executable",
                    return_value={"name": "riviu-managers-phone.exe"},
                ),
                patch.object(artifacts, "run_checked"),
            ):
                (root / "riviu-managers-phone.exe").write_bytes(b"fixture")
                report.write_text(
                    json.dumps({
                        "schemaVersion": 1,
                        "status": "ready",
                        "mode": "mock",
                        "databaseVersion": 18,
                    }),
                    encoding="utf-8",
                )
                with self.assertRaisesRegex(
                    artifacts.ArtifactError, "did not become ready"
                ):
                    artifacts.run_installed_app_smoke(root, report, data_dir)

    def test_installed_app_smoke_rejects_the_previous_database_schema(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            report = root / "startup.json"
            data_dir = root / "data"
            (root / "riviu-managers-phone.exe").write_bytes(b"fixture")
            report.write_text(
                json.dumps(
                    {
                        "schemaVersion": 1,
                        "status": "ready",
                        "mode": "mock",
                        "tauriReady": True,
                        "frontendReady": True,
                        "databaseVersion": 18,
                    }
                ),
                encoding="utf-8",
            )
            with (
                patch.object(
                    artifacts,
                    "verify_windows_desktop_executable",
                    return_value={"name": "riviu-managers-phone.exe"},
                ),
                patch.object(artifacts, "run_checked"),
            ):
                with self.assertRaisesRegex(
                    artifacts.ArtifactError, "did not become ready"
                ):
                    artifacts.run_installed_app_smoke(root, report, data_dir)

    def test_installed_app_smoke_parent_removes_child_scratch_after_exit(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            report = root / "startup.json"
            data_dir = root / "data"

            def run_child(_command, *, timeout):
                self.assertEqual(timeout, 180)
                data_dir.mkdir()
                (data_dir / "riviu.db").write_bytes(b"fixture")
                report.write_text(
                    json.dumps(
                        {
                            "schemaVersion": 1,
                            "status": "ready",
                            "mode": "mock",
                            "tauriReady": True,
                            "frontendReady": True,
                            "databaseVersion": artifacts.EXPECTED_DATABASE_VERSION,
                        }
                    ),
                    encoding="utf-8",
                )

            with (
                patch.object(
                    artifacts,
                    "verify_windows_desktop_executable",
                    return_value={"name": "riviu-managers-phone.exe"},
                ),
                patch.object(artifacts, "run_checked", side_effect=run_child),
            ):
                (root / "riviu-managers-phone.exe").write_bytes(b"fixture")
                payload = artifacts.run_installed_app_smoke(root, report, data_dir)

            self.assertEqual(payload["status"], "ready")
            self.assertFalse(data_dir.exists())

    def test_nsis_uninstall_waits_for_its_self_delete_child(self):
        install_root = Mock()
        install_root.exists.side_effect = [True, False]
        checker = Path("riviu-deployment-check.exe")
        with (
            patch.object(
                artifacts,
                "find_deployment_checker_if_present",
                side_effect=[[checker], []],
            ),
            patch.object(artifacts.time, "sleep") as sleep,
        ):
            artifacts.wait_for_nsis_uninstall(
                install_root, timeout_seconds=1.0, poll_seconds=0.01
            )

        sleep.assert_called_once_with(0.01)

    def test_windows_deployment_reports_survive_temporary_install_cleanup(self):
        packaged = {
            "_deploymentReportPayloads": {
                "msi": {
                    "schemaVersion": 1,
                    "profile": "internal",
                    "host": {"build": "22631"},
                    "overall": "warning",
                },
                "nsis": {
                    "schemaVersion": 1,
                    "profile": "internal",
                    "host": {"build": "19045"},
                    "overall": "pass",
                },
            }
        }
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary)
            records = artifacts.write_deployment_reports(
                packaged, output, "windows-x64"
            )

            self.assertNotIn("_deploymentReportPayloads", packaged)
            self.assertEqual(set(records), {"msi", "nsis"})
            for kind, record in records.items():
                report = artifacts.load_json(output / record["file"])
                self.assertEqual(report["overall"], records[kind]["overall"])
                self.assertEqual(report["profile"], records[kind]["profile"])
                self.assertEqual(
                    artifacts.sha256_file(output / record["file"]),
                    record["sha256"],
                )

    def test_windows_release_contract_requires_actual_install_checker_and_smoke_gates(self):
        packaged = {
            "msiSilentInstall": "PASS",
            "msiDeploymentCheck": "PASS",
            "msiStartupSmoke": "PASS",
            "nsisSilentInstall": "PASS",
            "nsisDeploymentCheck": "PASS",
            "nsisStartupSmoke": "PASS",
            "embeddedSignerErrorJson": "PASS",
            "desktopExecutable": "PASS",
            "desktopArchitecture": "x86_64",
            "deploymentReports": {
                "msi": {"profile": "production", "overall": "pass"},
                "nsis": {"profile": "production", "overall": "pass"},
            },
        }

        artifacts.require_windows_package_gates(packaged, Path("artifact-manifest.json"))

        production_warning = dict(packaged)
        production_warning["deploymentReports"] = {
            "msi": {"profile": "production", "overall": "warning"},
            "nsis": {"profile": "production", "overall": "warning"},
        }
        artifacts.require_windows_package_gates(
            production_warning, Path("artifact-manifest.json")
        )

        for missing in (
            "msiSilentInstall",
            "msiDeploymentCheck",
            "msiStartupSmoke",
            "nsisSilentInstall",
            "nsisDeploymentCheck",
            "nsisStartupSmoke",
        ):
            with self.subTest(missing=missing):
                drifted = dict(packaged)
                drifted.pop(missing)
                with self.assertRaisesRegex(
                    artifacts.ArtifactError,
                    "Windows installer verification gate is missing",
                ):
                    artifacts.require_windows_package_gates(
                        drifted, Path("artifact-manifest.json")
                    )

        internal = dict(packaged)
        internal["deploymentReports"] = {
            "msi": {"profile": "internal", "overall": "warning"},
            "nsis": {"profile": "internal", "overall": "warning"},
        }
        with self.assertRaisesRegex(
            artifacts.ArtifactError, "production deployment report"
        ):
            artifacts.require_windows_package_gates(
                internal, Path("artifact-manifest.json")
            )

    def test_release_tags_collect_with_the_production_deployment_profile(self):
        workflow = (artifacts.REPOSITORY_ROOT / ".github/workflows/desktop-ci-cd.yml").read_text(
            encoding="utf-8"
        )

        self.assertIn("--deployment-profile", workflow)
        self.assertIn(
            "startsWith(github.ref, 'refs/tags/v') && 'production' || 'internal'",
            workflow,
        )
        self.assertIn("snapshot-yt-dlp", workflow)

    def test_ytdlp_manifest_is_generated_from_the_downloaded_binary(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            binary = root / "yt-dlp.exe"
            binary.write_bytes(b"downloaded yt-dlp")
            completed = subprocess.CompletedProcess(
                [str(binary), "--version"], 0, "2026.09.01\n", ""
            )
            with patch.object(artifacts, "run_checked", return_value=completed):
                manifest = artifacts.ytdlp_manifest(binary)

            self.assertEqual(manifest["schemaVersion"], 1)
            self.assertEqual(manifest["path"], "yt-dlp.exe")
            self.assertEqual(manifest["bytes"], binary.stat().st_size)
            self.assertEqual(manifest["sha256"], artifacts.sha256_file(binary))
            self.assertEqual(manifest["version"], "2026.09.01")

    def test_release_verifier_reads_the_deployment_report_instead_of_trusting_its_summary(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            manifest_path = root / "windows-x64-artifact-manifest.json"
            manifest_path.write_text("{}", encoding="utf-8")
            reports = {}
            for kind in ("msi", "nsis"):
                report_path = root / f"{kind}-deployment-check.json"
                report_path.write_text(
                    json.dumps(
                        {
                            "schemaVersion": 1,
                            "profile": "internal" if kind == "msi" else "production",
                            "overall": "warning" if kind == "msi" else "pass",
                        }
                    ),
                    encoding="utf-8",
                )
                reports[kind] = {
                    "file": report_path.name,
                    "sha256": artifacts.sha256_file(report_path),
                    "profile": "production",
                    "overall": "pass",
                }
            packaged = {
                "msiSilentInstall": "PASS",
                "msiDeploymentCheck": "PASS",
                "msiStartupSmoke": "PASS",
                "nsisSilentInstall": "PASS",
                "nsisDeploymentCheck": "PASS",
                "nsisStartupSmoke": "PASS",
                "embeddedSignerErrorJson": "PASS",
                "desktopExecutable": "PASS",
                "desktopArchitecture": "x86_64",
                "deploymentReports": reports,
            }

            with self.assertRaisesRegex(
                artifacts.ArtifactError, "production deployment report"
            ):
                artifacts.require_windows_package_gates(packaged, manifest_path)

if __name__ == "__main__":
    unittest.main()
