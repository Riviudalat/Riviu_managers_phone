from __future__ import annotations

import tempfile
import subprocess
import sys
import unittest
from pathlib import Path
import csv

from scripts import build_xiaowei_parity_matrix as matrix
from scripts import check_xiaowei_provenance as gate


class XiaoweiProvenanceTests(unittest.TestCase):
    def test_gate_runs_as_a_script_from_the_repository_root(self):
        repository = Path(__file__).parents[1]
        with tempfile.TemporaryDirectory() as temporary:
            clean = Path(temporary) / "clean"
            clean.mkdir()
            (clean / "runtime.rs").write_text("const PRODUCT: &str = \"Riviu\";", encoding="utf-8")
            completed = subprocess.run(
                [
                    sys.executable,
                    str(repository / "scripts" / "check_xiaowei_provenance.py"),
                    "--runtime",
                    str(clean),
                ],
                cwd=repository,
                capture_output=True,
                text=True,
                check=False,
            )
            self.assertEqual(completed.returncode, 0, completed.stderr)
            self.assertIn('"ok": true', completed.stdout)

    def test_invocation_without_scan_targets_fails_closed(self):
        self.assertEqual(
            gate.inspect([]),
            [
                {
                    "surface": "gate",
                    "path": "",
                    "reason": "no scan targets were provided",
                }
            ],
        )

    def test_matrix_is_complete_unique_and_uses_only_reviewed_states(self):
        self.assertEqual(len(matrix.COMMANDS), 158)
        self.assertEqual(len(set(matrix.COMMANDS)), 158)
        allowed = {"existing", "implement", "commercial-excluded", "security-excluded", "not-applicable"}
        self.assertEqual({matrix.status_for(command)[0] for command in matrix.COMMANDS} - allowed, set())

    def test_every_excluded_matrix_command_is_an_exact_runtime_token_guard(self):
        expected = {
            command
            for command in matrix.COMMANDS
            if matrix.status_for(command)[0]
            in {"commercial-excluded", "security-excluded"}
        }
        self.assertEqual(gate.excluded_commands(), expected)
        with tempfile.TemporaryDirectory() as temporary:
            binary = Path(temporary) / "runtime.bin"
            binary.write_bytes(b'{"command":"activate"}\0{"command":"adb_command"}')
            reasons = {item[1] for item in gate.matched_needles(binary)}
            self.assertIn("commercial-excluded command", reasons)
            self.assertIn("security-excluded command", reasons)
            binary.write_bytes(b"reactivate activate_window")
            self.assertEqual(gate.matched_needles(binary), [])
            binary.write_bytes(b"\0activate\0")
            self.assertEqual(gate.matched_needles(binary), [])

    def test_checked_in_matrix_is_exact_generator_output(self):
        with tempfile.TemporaryDirectory() as temporary:
            generated = Path(temporary) / "matrix.csv"
            matrix.build(generated)
            checked_in = Path(__file__).parents[1] / "docs" / "provenance" / "xiaowei-parity-matrix.csv"
            with generated.open(encoding="utf-8", newline="") as actual:
                with checked_in.open(encoding="utf-8", newline="") as expected:
                    self.assertEqual(list(csv.reader(actual)), list(csv.reader(expected)))

    def test_matrix_rows_survive_checkout_line_endings_without_hiding_cell_changes(self):
        with tempfile.TemporaryDirectory() as temporary:
            generated = Path(temporary) / "matrix.csv"
            matrix.build(generated)
            raw = generated.read_bytes()
            windows = Path(temporary) / "windows.csv"
            windows.write_bytes(raw.replace(b"\n", b"\r\n"))
            with generated.open(encoding="utf-8", newline="") as actual:
                with windows.open(encoding="utf-8", newline="") as expected:
                    self.assertEqual(list(csv.reader(actual)), list(csv.reader(expected)))
            rows = list(csv.reader(raw.decode("utf-8").splitlines()))
            changed = [row[:] for row in rows]
            changed[1][-1] += " changed"
            self.assertNotEqual(rows, changed)

    def test_missing_scan_target_fails_closed(self):
        with tempfile.TemporaryDirectory() as temporary:
            missing = Path(temporary) / "frontend-dist-that-was-never-built"
            self.assertEqual(
                gate.inspect([("frontend", missing)]),
                [
                    {
                        "surface": "frontend",
                        "path": str(missing),
                        "reason": "scan target does not exist",
                    }
                ],
            )

    def test_empty_scan_target_fails_closed(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            self.assertEqual(
                gate.inspect([("installer", root)]),
                [
                    {
                        "surface": "installer",
                        "path": str(root),
                        "reason": "scan target contains no files",
                    }
                ],
            )

    def test_gate_rejects_vendor_endpoint_package_command_and_artifact(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / "runtime.rs").write_text(
                'const P: &str = "com.xiaowei.assistant"; const C: &str = "exec_autojs"; '
                'const LOGIN: &str = "client_login"; const USB: &str = "usb_to_tcp";',
                encoding="utf-8",
            )
            (root / "assistant.apk").write_bytes(b"fixture")
            findings = gate.inspect([("runtime", root)])
            reasons = {finding["reason"] for finding in findings}
            self.assertIn("vendor package", reasons)
            self.assertIn("arbitrary-script command", reasons)
            self.assertIn("commercial account command", reasons)
            self.assertIn("vendor USB transport command", reasons)

    def test_gate_rejects_vendor_bridge_key_material_and_renamed_binary(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / "xiaowei-helper.dll").write_bytes(b"connect :32991")
            (root / "adbkey").write_bytes(b"private fixture")
            findings = gate.inspect([("installer", root)])
            reasons = {finding["reason"] for finding in findings}
            self.assertEqual(
                reasons,
                {"vendor artifact name", "vendor bridge port"},
            )

    def test_frontend_brand_is_rejected_but_clean_runtime_passes(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / "app.js").write_text('console.log("Xiaowei")', encoding="utf-8")
            self.assertTrue(gate.inspect([("frontend", root)]))
            (root / "app.js").write_text('console.log("Riviu")', encoding="utf-8")
            self.assertEqual(gate.inspect([("runtime", root)]), [])

    def test_streaming_scan_finds_a_pattern_across_chunk_boundary(self):
        with tempfile.TemporaryDirectory() as temporary:
            binary = Path(temporary) / "installer.exe"
            binary.write_bytes(b"x" * (1024 * 1024 - 4) + b"exec" + b"_autojs")
            self.assertIn(
                "arbitrary-script command",
                {item[1] for item in gate.matched_needles(binary)},
            )


if __name__ == "__main__":
    unittest.main()
