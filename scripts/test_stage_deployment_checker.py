from __future__ import annotations

import tempfile
import unittest
from pathlib import Path

from scripts import stage_deployment_checker as stage


class StageDeploymentCheckerTests(unittest.TestCase):
    def test_windows_binary_gets_the_target_suffix_before_exe(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            source = root / "target" / "riviu-deployment-check.exe"
            source.parent.mkdir()
            source.write_bytes(b"MZ-fixture")

            staged = stage.stage_binary(
                source,
                root / "binaries",
                "x86_64-pc-windows-msvc",
            )

            self.assertEqual(
                staged.name,
                "riviu-deployment-check-x86_64-pc-windows-msvc.exe",
            )
            self.assertEqual(staged.read_bytes(), b"MZ-fixture")

    def test_non_windows_binary_has_no_exe_suffix(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            source = root / "riviu-deployment-check"
            source.write_bytes(b"fixture")

            staged = stage.stage_binary(
                source,
                root / "binaries",
                "aarch64-apple-darwin",
            )

            self.assertEqual(
                staged.name,
                "riviu-deployment-check-aarch64-apple-darwin",
            )

    def test_missing_input_is_refused(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            with self.assertRaisesRegex(FileNotFoundError, "checker binary is missing"):
                stage.stage_binary(
                    root / "missing.exe",
                    root / "binaries",
                    "x86_64-pc-windows-msvc",
                )


if __name__ == "__main__":
    unittest.main()
