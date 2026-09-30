import json
from pathlib import Path
import subprocess
import tempfile
import unittest
import contextlib
import shutil
import uuid

from scripts.bundle_windows_installers import MARKER, bundle, pristine_binary


@contextlib.contextmanager
def fixture_directory():
    # Inherit the test runner's writable directory ACL. Windows restricted tokens
    # cannot access TemporaryDirectory's owner-only security descriptor.
    parent = Path(tempfile.gettempdir()).resolve()
    root = parent / f"riviu-bundle-fixture-{uuid.uuid4()}"
    root.mkdir()
    try:
        yield root
    finally:
        if root.resolve().parent != parent:
            raise RuntimeError("fixture path left the test temporary directory")
        shutil.rmtree(root)


class BundleWindowsInstallersTests(unittest.TestCase):
    def run_fixture(self, fail=False, corrupt=False, frontend_ready=True):
        with fixture_directory() as folder:
            root = Path(folder)
            cli = root / "node_modules/@tauri-apps/cli/tauri.js"
            cli.parent.mkdir(parents=True)
            cli.write_text("fixture")
            app = root / "app.exe"
            # Also exercise a previous interrupted NSIS bundle as the input.
            app.write_bytes(b"MZ-fixture" + MARKER + b"NSS\0payload")
            canonical, offset = pristine_binary(app.read_bytes())
            commands = []

            def run(command, **_kwargs):
                self.assertEqual(app.read_bytes(), canonical)
                if "--verify-frontend" in command:
                    Path(command[-1]).write_text(json.dumps({
                        "status": "ready" if frontend_ready else "missing",
                        "frontendKind": "embeddedDirectory",
                    }))
                    return subprocess.CompletedProcess(command, 0)
                self.assertIn("bundle", command)
                self.assertNotIn("build", command)
                kind = command[command.index("--bundles") + 1]
                commands.append(command)
                tag = b"NSS" if kind == "nsis" else b"MSI"
                data = canonical[:offset] + tag + canonical[offset + 3:]
                app.write_bytes(data + b"changed" if corrupt else data)
                return subprocess.CompletedProcess(command, 1 if fail else 0)

            operation = lambda: bundle(app, root, "x86_64-pc-windows-msvc", [root / "common.json"], root / "msi.json", root / "report.json", run=run)
            if not frontend_ready:
                with self.assertRaisesRegex(ValueError, "embedded frontend"):
                    operation()
                self.assertEqual(commands, [])
                self.assertEqual(app.read_bytes(), canonical)
                return
            if corrupt:
                with self.assertRaisesRegex(ValueError, "changed outside"):
                    operation()
                self.assertTrue(app.read_bytes().endswith(b"changed"))
            elif fail:
                with self.assertRaises(subprocess.CalledProcessError):
                    operation()
                self.assertEqual(app.read_bytes(), canonical)
                self.assertEqual(len(commands), 1)
            else:
                report = operation()
                self.assertEqual(app.read_bytes(), canonical)
                self.assertEqual([s["kind"] for s in report["steps"]], ["nsis", "msi"])
                self.assertNotIn(str(root / "msi.json"), commands[0])
                self.assertIn(str(root / "msi.json"), commands[1])
            self.assertTrue(json.loads((root / "report.json").read_text())["steps"])

    def test_each_installer_receives_pristine_binary_and_only_msi_has_its_overlay(self):
        self.run_fixture()

    def test_failed_bundle_restores_binary_and_never_runs_next_installer(self):
        self.run_fixture(fail=True)

    def test_unexpected_app_change_is_preserved_and_refused(self):
        self.run_fixture(corrupt=True)

    def test_missing_embedded_frontend_never_starts_a_bundler(self):
        self.run_fixture(frontend_ready=False)

    def test_ambiguous_or_unknown_marker_refuses(self):
        for data in [b"MZ", MARKER + b"BAD", MARKER + b"UNK" + MARKER + b"UNK"]:
            with self.assertRaises(ValueError):
                pristine_binary(data)


if __name__ == "__main__":
    unittest.main()
