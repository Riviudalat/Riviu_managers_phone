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


class AppleCredentialTests(unittest.TestCase):
    """The Apple ID and its app-specific password must never reach a command line.

    On Windows every process running as the same user can read another process's command line,
    and EDR/Sysmon record it as a matter of course — so `--password <secret>` handed the value
    to the whole box and made the OS credential store pointless. These two tests pin the shape
    rather than the wording: one says the parser refuses to accept the flags at all, the other
    says the child Xcode process is not handed the secret it does not need.
    """

    def test_the_parser_no_longer_accepts_credentials_as_flags(self):
        for flag, value in (("--password", "hunter2"), ("--apple-id", "a@b.test")):
            with self.subTest(flag=flag):
                with (
                    patch.object(
                        riviu_signer.sys,
                        "argv",
                        ["riviu_signer.py", "sign-install-wda", "--udid", "u", flag, value],
                    ),
                    patch("sys.stderr", new_callable=io.StringIO),
                ):
                    # argparse exits 2 on an unrecognised flag. A future edit that re-adds the
                    # flag "just to keep the old call working" turns this red.
                    with self.assertRaises(SystemExit) as raised:
                        riviu_signer.main()
                    self.assertEqual(raised.exception.code, 2)

    def test_the_child_xcode_process_does_not_inherit_the_credentials(self):
        completed = SimpleNamespace(returncode=1, stdout='{"ok": false, "error": "x"}', stderr="")
        with (
            patch.object(
                riviu_signer.sys,
                "argv",
                ["riviu_signer.py", "sign-install-wda", "--udid", "fixture-udid"],
            ),
            patch.dict(
                riviu_signer.os.environ,
                {"RIVIU_APPLE_ID": "a@b.test", "RIVIU_APPLE_PASSWORD": "hunter2"},
            ),
            patch.object(riviu_signer.subprocess, "run", return_value=completed) as run,
            patch("sys.stdout", new_callable=io.StringIO),
        ):
            self.assertEqual(riviu_signer.main(), 1)

        child_environment = run.call_args.kwargs["env"]
        self.assertNotIn("RIVIU_APPLE_PASSWORD", child_environment)
        self.assertNotIn("RIVIU_APPLE_ID", child_environment)
        # And nothing smuggled it into argv on the way.
        self.assertNotIn("hunter2", " ".join(run.call_args.args[0]))


if __name__ == "__main__":
    unittest.main()
