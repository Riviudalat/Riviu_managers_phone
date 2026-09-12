"""Read-only APK regressions. Requires SDK build-tools; never invokes adb."""
import hashlib
import json
import os
from pathlib import Path
import re
import struct
import subprocess
import unittest
import xml.etree.ElementTree as ET
import zipfile


ROOT = Path(__file__).resolve().parent
REPO = ROOT.parents[1]
APK = REPO / "sidecars/android/noarch/riviu-agent.apk"
ANDROID = "{http://schemas.android.com/apk/res/android}"
# The installed 0.4.0 helper uses this certificate; a new key would reject upgrades.
SIGNER_SHA256 = "0ecb2f06620b2d0f2fcc2a71cede204819a20ddcc1210573bc8ab3f4b48ff813"


def run(*command: str) -> str:
    result = subprocess.run(command, capture_output=True, text=True, encoding="utf-8", errors="replace")
    if result.returncode:
        raise AssertionError(f"command failed ({result.returncode}): {command[0]}\n{result.stdout}\n{result.stderr}")
    return result.stdout


class HelperPackageTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        sdk = os.environ.get("ANDROID_HOME") or os.environ.get("ANDROID_SDK_ROOT")
        if not sdk:
            raise RuntimeError("Set ANDROID_HOME or ANDROID_SDK_ROOT to verify the actual APK")
        candidates = sorted((Path(sdk) / "build-tools").glob("*"), reverse=True)
        cls.tools = next((path for path in candidates if (path / "aapt.exe").is_file()), None)
        if cls.tools is None:
            raise RuntimeError("Android build-tools with aapt/apksigner are required")
        cls.badging = run(str(cls.tools / "aapt.exe"), "dump", "badging", str(APK))
        cls.manifest_dump = run(str(cls.tools / "aapt.exe"), "dump", "xmltree", str(APK), "AndroidManifest.xml")
        cls.manifest = ET.parse(ROOT / "app/src/main/AndroidManifest.xml").getroot()

    def test_apk_exposes_named_launcher_and_pinned_protocol_version(self) -> None:
        gradle = (ROOT / "app/build.gradle").read_text(encoding="utf-8")
        version = re.search(r'versionName\s+"([^"]+)"', gradle).group(1)
        code = re.search(r"versionCode\s+(\d+)", gradle).group(1)
        protocol = (ROOT / "app/src/main/java/com/riviu/agent/Protocol.java").read_text(encoding="utf-8")
        self.assertIn(f"name='com.riviu.agent' versionCode='{code}' versionName='{version}'", self.badging)
        self.assertIn("application-label:'Riviu Helper'", self.badging)
        self.assertIn("launchable-activity: name='com.riviu.agent.MainActivity'", self.badging)
        self.assertIn(f'AGENT_VERSION = "{version}"', protocol)
        self.assertIn('features.put("launcher")', protocol)
        with zipfile.ZipFile(APK) as archive:
            dex = archive.read("classes.dex")
            self.assertIn(b"Lcom/riviu/agent/MainActivity;", dex)
            self.assertIn(b"launcher", dex)
            self.assertIn(version.encode(), dex)

    def test_launcher_is_read_only_and_service_access_contract_is_retained(self) -> None:
        application = self.manifest.find("application")
        activity = application.find("activity")
        self.assertEqual(activity.get(ANDROID + "name"), ".MainActivity")
        self.assertEqual(activity.get(ANDROID + "exported"), "true")
        self.assertEqual(application.get(ANDROID + "icon"), "@mipmap/ic_launcher")
        services = {node.get(ANDROID + "name"): node for node in application.findall("service")}
        self.assertEqual(services[".AgentService"].get(ANDROID + "permission"), "android.permission.DUMP")
        self.assertEqual(services[".RiviuIme"].get(ANDROID + "permission"), "android.permission.BIND_INPUT_METHOD")
        self.assertRegex(self.manifest_dump, r'(?s)E: service.*?"\.AgentService".*?android:permission[^\n]+"android.permission.DUMP"')
        self.assertRegex(self.manifest_dump, r'(?s)E: service.*?"\.RiviuIme".*?android:permission[^\n]+"android.permission.BIND_INPUT_METHOD"')
        activity_source = (ROOT / "app/src/main/java/com/riviu/agent/MainActivity.java").read_text(encoding="utf-8")
        for forbidden in ("startService(", "startForegroundService(", "Settings.Secure", "InputMethodManager", "EXTRA_TOKEN"):
            self.assertNotIn(forbidden, activity_source)
        self.assertIn("AgentService.localStatus()", activity_source)
        self.assertIn("handler.removeCallbacks(refresh)", activity_source)
        service = (ROOT / "app/src/main/java/com/riviu/agent/AgentService.java").read_text(encoding="utf-8")
        self.assertIn("!active.server.isRunning()", service)
        self.assertIn("PendingIntent.FLAG_IMMUTABLE", service)
        self.assertIn(".setSmallIcon(R.drawable.ic_notification)", service)

    def test_apk_contains_density_adaptive_and_notification_brand_assets(self) -> None:
        with zipfile.ZipFile(APK) as archive:
            names = archive.namelist()
            for qualifier in ("v26", "v33"):
                self.assertIn(f"res/mipmap-anydpi-{qualifier}/ic_launcher.xml", names)
                self.assertIn(f"res/mipmap-anydpi-{qualifier}/ic_launcher_round.xml", names)
            for density, scale in (("mdpi", 1), ("hdpi", 1.5), ("xhdpi", 2), ("xxhdpi", 3), ("xxxhdpi", 4)):
                for name, size in (("ic_launcher", 48), ("ic_launcher_foreground", 108), ("ic_launcher_monochrome", 108)):
                    member = next(path for path in names if path.startswith(f"res/mipmap-{density}") and path.endswith(f"/{name}.png"))
                    image = archive.read(member)
                    self.assertEqual(image[:8], b"\x89PNG\r\n\x1a\n")
                    self.assertEqual(struct.unpack(">II", image[16:24]), (round(size * scale),) * 2)
                self.assertTrue(any(path.startswith(f"res/drawable-{density}") and path.endswith("/ic_notification.png") for path in names))
            self.assertIn("res/drawable-nodpi-v4/riviu_logo.png", names)

    def test_shipped_bytes_match_manifest_and_notice(self) -> None:
        data = APK.read_bytes()
        digest = hashlib.sha256(data).hexdigest()
        manifest = json.loads((REPO / "sidecars/android/android-tools-manifest.json").read_text(encoding="utf-8"))
        entry = next(item for item in manifest["files"] if item.get("role") == "riviuAgentApk")
        self.assertEqual(entry["bytes"], len(data))
        self.assertEqual(entry["sha256"], digest)
        notice = (REPO / "NOTICE").read_text(encoding="utf-8")
        self.assertIn(digest, notice)
        self.assertIn(f"bytes  {len(data)}", notice)

    def test_apk_signature_preserves_upgrade_identity_and_alignment(self) -> None:
        signer = run(str(self.tools / "apksigner.bat"), "verify", "--verbose", "--print-certs", str(APK))
        self.assertIn("Verified using v2 scheme (APK Signature Scheme v2): true", signer)
        self.assertIn(f"Signer #1 certificate SHA-256 digest: {SIGNER_SHA256}", signer)
        run(str(self.tools / "zipalign.exe"), "-c", "4", str(APK))


if __name__ == "__main__":
    unittest.main(verbosity=2)
