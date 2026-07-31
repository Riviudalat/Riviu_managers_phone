from __future__ import annotations

import io
import json
import unittest
from types import SimpleNamespace
from unittest.mock import patch

from sidecars.signer import riviu_signer


class EmbeddedRuntimeTests(unittest.TestCase):
    def test_windows_embedded_runtime_stays_hidden_and_reenters_build_script(self):
        completed = SimpleNamespace(
            returncode=1,
            stdout='{"ok": false, "error": "fixture"}\n',
            stderr="",
        )
        with (
            patch.object(
                riviu_signer.sys,
                "argv",
                [
                    "riviu_signer.py",
                    "sign-install-wda",
                    "--udid",
                    "fixture-udid",
                    "--apple-id",
                    "fixture@example.test",
                    "--password",
                    "fixture-password",
                ],
            ),
            patch.object(riviu_signer.sys, "platform", "win32"),
            patch.dict(
                riviu_signer.os.environ,
                {"RIVIU_EMBEDDED_PYTHON_RUNTIME": "riviu-pmd.exe"},
            ),
            patch.object(
                riviu_signer.subprocess,
                "run",
                return_value=completed,
            ) as run,
            patch("sys.stdout", new_callable=io.StringIO),
        ):
            self.assertEqual(riviu_signer.main(), 1)

        command = run.call_args.args[0]
        options = run.call_args.kwargs
        self.assertEqual(command[0:2], ["riviu-pmd.exe", "__script"])
        self.assertEqual(command[2], str(riviu_signer.BUILD_INSTALL))
        self.assertEqual(options["creationflags"], 0x08000000)
        self.assertEqual(options["encoding"], "utf-8")
        self.assertEqual(options["errors"], "replace")
        self.assertEqual(options["env"]["PYTHONUTF8"], "1")
        self.assertEqual(options["env"]["PYTHONIOENCODING"], "utf-8")

    def test_structured_error_is_ascii_safe_for_a_frozen_windows_console(self):
        completed = SimpleNamespace(
            returncode=1,
            stdout=json.dumps({"ok": False, "error": "fixture \ufffd lỗi"}),
            stderr="",
        )
        output_bytes = io.BytesIO()
        output = io.TextIOWrapper(output_bytes, encoding="cp1252")
        try:
            with (
                patch.object(
                    riviu_signer.sys,
                    "argv",
                    [
                        "riviu_signer.py",
                        "sign-install-wda",
                        "--udid",
                        "fixture-udid",
                        "--apple-id",
                        "fixture@example.test",
                        "--password",
                        "fixture-password",
                    ],
                ),
                patch.object(riviu_signer.subprocess, "run", return_value=completed),
                patch("sys.stdout", output),
            ):
                self.assertEqual(riviu_signer.main(), 1)
            output.flush()
            payload = json.loads(output_bytes.getvalue().decode("cp1252"))
            self.assertEqual(payload["error"], "fixture \ufffd lỗi")
        finally:
            output.detach()


if __name__ == "__main__":
    unittest.main()
