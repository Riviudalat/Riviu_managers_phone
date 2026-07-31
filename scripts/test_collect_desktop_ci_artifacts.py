from __future__ import annotations

import argparse
import json
import os
import subprocess
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

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

    def test_invalid_dmg_plist_still_detaches_the_owned_mountpoint(self):
        with tempfile.TemporaryDirectory() as temporary:
            bundle_dir = Path(temporary) / "bundle" / "dmg"
            bundle_dir.mkdir(parents=True)
            dmg = bundle_dir / "fixture.dmg"
            dmg.write_bytes(b"fixture")
            attachment = subprocess.CompletedProcess(
                args=["hdiutil"], returncode=0, stdout="not-a-plist", stderr=""
            )
            detach_failure = subprocess.CompletedProcess(
                args=["hdiutil"], returncode=1, stdout="", stderr="detach failed"
            )
            with (
                patch.object(artifacts, "find_installers", return_value=[dmg]),
                patch.object(artifacts, "run_checked", return_value=attachment),
                patch.object(
                    artifacts.subprocess, "run", return_value=detach_failure
                ) as detach,
                patch.object(artifacts.time, "sleep"),
            ):
                with self.assertRaisesRegex(
                    artifacts.ArtifactError, "invalid attachment plist"
                ) as raised:
                    artifacts.verify_macos_package(
                        bundle_dir, "aarch64-apple-darwin", Path(temporary), {}
                    )

            self.assertEqual(detach.call_count, 3)
            self.assertIn("-force", detach.call_args.args[0])
            self.assertIsInstance(raised.exception.__cause__, artifacts.ArtifactError)
            self.assertIn("failed to detach DMG after 3 attempts", str(raised.exception.__cause__))

    def test_partial_dmg_attach_discovers_and_detaches_the_mountpoint(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            bundle_dir = root / "bundle" / "dmg"
            bundle_dir.mkdir(parents=True)
            dmg = bundle_dir / "fixture.dmg"
            dmg.write_bytes(b"fixture")
            mount_point = root / "owned-mount"
            mount_point.mkdir()
            info = subprocess.CompletedProcess(
                args=["hdiutil"],
                returncode=0,
                stdout=artifacts.plistlib.dumps(
                    {
                        "images": [
                            {
                                "system-entities": [
                                    {"mount-point": str(mount_point)}
                                ]
                            }
                        ]
                    }
                ).decode("utf-8"),
                stderr="",
            )
            detached = subprocess.CompletedProcess(
                args=["hdiutil"], returncode=0, stdout="", stderr=""
            )
            attach_error = artifacts.ArtifactError("attach command failed")
            with (
                patch.object(artifacts, "find_installers", return_value=[dmg]),
                patch.object(artifacts, "run_checked", side_effect=attach_error),
                patch.object(
                    artifacts.tempfile, "mkdtemp", return_value=str(mount_point)
                ),
                patch.object(
                    artifacts.subprocess, "run", side_effect=[info, detached]
                ) as system_command,
            ):
                with self.assertRaisesRegex(
                    artifacts.ArtifactError, "attach command failed"
                ) as raised:
                    artifacts.verify_macos_package(
                        bundle_dir, "aarch64-apple-darwin", root, {}
                    )

            self.assertIs(raised.exception, attach_error)
            self.assertEqual(system_command.call_count, 2)
            self.assertEqual(
                system_command.call_args_list[0].args[0],
                ["hdiutil", "info", "-plist"],
            )
            self.assertEqual(
                system_command.call_args_list[1].args[0],
                ["hdiutil", "detach", str(mount_point.resolve())],
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
        result = artifacts.verify_version_command(argparse.Namespace(tag="v0.1.0"))
        self.assertEqual(result["version"], "0.1.0")

        with self.assertRaisesRegex(artifacts.ArtifactError, "tag mismatch"):
            artifacts.verify_version_command(argparse.Namespace(tag="v9.9.9"))

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


if __name__ == "__main__":
    unittest.main()
