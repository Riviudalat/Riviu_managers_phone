"""Execute the production MAC shell program against a fake ip command only."""
import json
from pathlib import Path
import re
import shutil
import subprocess
import unittest


class MacChangeTest(unittest.TestCase):
    def run_fixture(self, down=0, address=0, up=0):
        source = (Path(__file__).resolve().parents[1] / "crates/android-driver/src/driver/device_ops.rs").read_text(encoding="utf-8")
        body = source[source.index("fn mac_change_command"):]
        program = json.loads('"' + re.search(r'format!\("((?:\\.|[^"\\])*)"\)', body).group(1) + '"').replace("{mac}", "02:00:00:00:00:01")
        git_bash = Path("C:/Program Files/Git/bin/bash.exe")
        shell = str(git_bash) if git_bash.exists() else shutil.which("bash")
        fixture = f'''ip() {{
case "$4" in
  down) echo down; return {down};;
  address) echo address; return {address};;
  up) echo up; return {up};;
  *) return 99;;
esac
}}
'''
        return subprocess.run([shell, "-c", fixture + program], capture_output=True, text=True)

    def test_address_failure_is_retained_and_interface_is_restored(self):
        result = self.run_fixture(address=7)
        self.assertEqual(result.returncode, 7)
        self.assertEqual(result.stdout.splitlines(), ["down", "address", "up"])

    def test_down_failure_prevents_address_change_but_restores_interface(self):
        result = self.run_fixture(down=8)
        self.assertEqual(result.returncode, 8)
        self.assertEqual(result.stdout.splitlines(), ["down", "up"])

    def test_up_failure_is_not_reported_as_success(self):
        self.assertEqual(self.run_fixture(up=9).returncode, 9)

    def test_success_requires_every_step(self):
        self.assertEqual(self.run_fixture().returncode, 0)


if __name__ == "__main__":
    unittest.main()
