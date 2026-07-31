from __future__ import annotations

import argparse
import json
import os
import tempfile
import unittest
from pathlib import Path

from scripts import collect_desktop_ci_artifacts as artifacts


class ArtifactContractTests(unittest.TestCase):
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
                "dependencyClosure": {
                    "pymobiledevice3": "10.1.0",
                    "tidevice": "0.12.11",
                    "pyinstaller": "6.21.0",
                },
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

            entrypoint.write_bytes(b"tampered-runtime")
            with self.assertRaisesRegex(
                artifacts.ArtifactError, "entrypoint SHA-256"
            ):
                artifacts.verify_runtime(runtime, "x86_64-pc-windows-msvc")


if __name__ == "__main__":
    unittest.main()
