from __future__ import annotations

import tempfile
import unittest
from pathlib import Path

from scripts import build_xiaowei_parity_matrix as matrix
from scripts import check_xiaowei_provenance as gate


class XiaoweiProvenanceTests(unittest.TestCase):
    def test_matrix_is_complete_unique_and_uses_only_reviewed_states(self):
        self.assertEqual(len(matrix.COMMANDS), 158)
        self.assertEqual(len(set(matrix.COMMANDS)), 158)
        allowed = {"existing", "implement", "commercial-excluded", "security-excluded", "not-applicable"}
        self.assertEqual({matrix.status_for(command)[0] for command in matrix.COMMANDS} - allowed, set())

    def test_gate_rejects_vendor_endpoint_package_command_and_artifact(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / "runtime.rs").write_text(
                'const P: &str = "com.xiaowei.assistant"; const C: &str = "exec_autojs";',
                encoding="utf-8",
            )
            (root / "assistant.apk").write_bytes(b"fixture")
            findings = gate.inspect([("runtime", root)])
            reasons = {finding["reason"] for finding in findings}
            self.assertIn("vendor package", reasons)
            self.assertIn("arbitrary-script command", reasons)
            self.assertIn("vendor artifact name", reasons)

    def test_frontend_brand_is_rejected_but_clean_runtime_passes(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / "app.js").write_text('console.log("Xiaowei")', encoding="utf-8")
            self.assertTrue(gate.inspect([("frontend", root)]))
            (root / "app.js").write_text('console.log("Riviu")', encoding="utf-8")
            self.assertEqual(gate.inspect([("runtime", root)]), [])


if __name__ == "__main__":
    unittest.main()
