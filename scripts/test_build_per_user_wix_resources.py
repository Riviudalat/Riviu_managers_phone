from __future__ import annotations

import html
import json
import re
import tempfile
import unittest
from pathlib import Path

from scripts.build_per_user_wix_resources import COMPONENT_GROUP_ID, collect_resources, write_outputs


class PerUserWixResourcesTests(unittest.TestCase):
    def test_webview_download_action_fits_windows_installer_target(self) -> None:
        template = (
            Path(__file__).parents[1]
            / "apps"
            / "desktop"
            / "src-tauri"
            / "wix"
            / "main-per-user.wxs"
        ).read_text(encoding="utf-8")
        match = re.search(
            r'<CustomAction Id="DownloadAndInvokeBootstrapper"[^>]*ExeCommand="([^"]*)"',
            template,
        )
        self.assertIsNotNone(match)
        command = html.unescape(match.group(1))
        self.assertLessEqual(len(command), 255)
        self.assertIn("curl.exe -fL --retry 3", command)
        self.assertIn("/silent /install", command)
        self.assertIn("del /q", command)
        self.assertIn("exit /b !r!", command)
        registry_key = r'Key="Software\\{{@root/manufacturer}}\\{{@root/product_name}}\Components"'
        self.assertEqual(template.count(registry_key), 2)
        self.assertNotIn("{{../manufacturer}}", template)

    def test_generates_registry_keypaths_and_directory_cleanup(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            resources = root / "payload"
            (resources / "nested").mkdir(parents=True)
            (resources / "root.txt").write_text("root", encoding="utf-8")
            (resources / "nested" / "child.txt").write_text("child", encoding="utf-8")
            config = root / "tauri.json"
            config.write_text(
                json.dumps({"bundle": {"resources": {"payload/": "sidecars/runtime/"}}}),
                encoding="utf-8",
            )
            fragment = root / "generated" / "resources.wxs"
            overlay = root / "generated" / "overlay.json"

            count, _digest = write_outputs([config], fragment, overlay, target="x86_64-pc-windows-msvc")

            self.assertEqual(count, 2)
            xml = fragment.read_text(encoding="utf-8")
            self.assertEqual(xml.count('<File Id="'), 2)
            self.assertEqual(xml.count('KeyPath="no"'), 2)
            self.assertEqual(xml.count('Root="HKCU"'), 5)
            self.assertEqual(xml.count('KeyPath="yes"'), 5)
            self.assertEqual(xml.count('<RemoveFolder '), 3)
            self.assertEqual(xml.count('<ComponentRef Id="'), 5)
            generated_overlay = json.loads(overlay.read_text(encoding="utf-8"))
            self.assertEqual(generated_overlay["bundle"]["resources"], [])
            self.assertEqual(
                generated_overlay["bundle"]["windows"]["wix"]["componentGroupRefs"],
                [COMPONENT_GROUP_ID],
            )

    def test_conflicting_resource_destinations_are_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            first = root / "first.txt"
            second = root / "second.txt"
            first.write_text("first", encoding="utf-8")
            second.write_text("second", encoding="utf-8")
            base = root / "base.json"
            overlay = root / "overlay.json"
            base.write_text(json.dumps({"bundle": {"resources": {"first.txt": "same.txt"}}}), encoding="utf-8")
            overlay.write_text(json.dumps({"bundle": {"resources": {"second.txt": "same.txt"}}}), encoding="utf-8")

            with self.assertRaisesRegex(ValueError, "duplicate resource destination"):
                collect_resources([base, overlay])

    def test_absolute_and_traversing_destinations_are_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            source = root / "payload.txt"
            source.write_text("payload", encoding="utf-8")

            for destination in ("/absolute", r"C:\absolute", r"\\server\share", "../escape", "."):
                with self.subTest(destination=destination):
                    config = root / "tauri.json"
                    config.write_text(
                        json.dumps({"bundle": {"resources": {"payload.txt": destination}}}),
                        encoding="utf-8",
                    )
                    with self.assertRaisesRegex(ValueError, "unsafe resource destination"):
                        collect_resources([config])


if __name__ == "__main__":
    unittest.main()
