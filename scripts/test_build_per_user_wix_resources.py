from __future__ import annotations

import html
import json
import re
import sys
import tempfile
import unittest
import xml.etree.ElementTree as ET
from pathlib import Path

from scripts.build_per_user_wix_resources import (
    COMPONENT_GROUP_ID,
    collect_resources,
    render_fragment,
    write_outputs,
)


class PerUserWixResourcesTests(unittest.TestCase):
    def test_windows_floor_search_and_condition_are_in_the_linked_fragment(self) -> None:
        ns = {"w": "http://schemas.microsoft.com/wix/2006/wi"}
        document = ET.fromstring(render_fragment([], win64=True))
        fragment = next(
            item for item in document.findall("w:Fragment", ns)
            if item.find(f'w:ComponentGroup[@Id="{COMPONENT_GROUP_ID}"]', ns) is not None
        )
        search = fragment.find('w:Property[@Id="RIVIU_WINDOWS_BUILD"]/w:RegistrySearch', ns)
        self.assertIsNotNone(search)
        self.assertEqual(search.attrib["Root"], "HKLM")
        self.assertEqual(search.attrib["Key"], r"SOFTWARE\Microsoft\Windows NT\CurrentVersion")
        self.assertEqual(search.attrib["Name"], "CurrentBuildNumber")
        self.assertEqual(search.attrib["Type"], "raw")
        self.assertEqual(search.attrib["Win64"], "yes")
        condition = fragment.find("w:Condition", ns)
        self.assertIsNotNone(condition)
        nsis = (Path(__file__).parents[1] / "scripts/windows-installer-hooks.nsh").read_text(encoding="utf-8")
        self.assertIn(condition.attrib["Message"], nsis)
        floor = re.search(r"AtLeastBuild\} (\d+)", nsis)
        self.assertIsNotNone(floor)
        self.assertIn(f" >= {floor.group(1)}", condition.text)

    @unittest.skipUnless(sys.platform == "win32", "requires the native Windows Installer evaluator")
    def test_native_msi_floor_is_numeric_and_allows_maintenance(self) -> None:
        # A tiny in-memory install session exercises the actual MSI evaluator,
        # including missing, invalid and lexicographically misleading builds.
        # IGNOREMACHINESTATE=1 prevents this handle changing machine state.
        import ctypes
        from ctypes import byref, c_uint, c_void_p, c_wchar_p

        msi = ctypes.WinDLL("msi")
        msi.MsiOpenDatabaseW.argtypes = [c_wchar_p, c_void_p, ctypes.POINTER(c_uint)]
        msi.MsiDatabaseOpenViewW.argtypes = [c_uint, c_wchar_p, ctypes.POINTER(c_uint)]
        msi.MsiOpenPackageExW.argtypes = [c_wchar_p, c_uint, ctypes.POINTER(c_uint)]
        msi.MsiSetPropertyW.argtypes = [c_uint, c_wchar_p, c_wchar_p]
        msi.MsiEvaluateConditionW.argtypes = [c_uint, c_wchar_p]
        msi.MsiGetSummaryInformationW.argtypes = [c_uint, c_wchar_p, c_uint, ctypes.POINTER(c_uint)]
        msi.MsiSummaryInfoSetPropertyW.argtypes = [c_uint, c_uint, c_uint, ctypes.c_int, c_void_p, c_wchar_p]
        ole = ctypes.OleDLL("ole32")
        self.assertIn(ole.CoInitialize(None), [0, 1])
        handles = []
        try:
            with tempfile.TemporaryDirectory() as temporary:
                database_path = str(Path(temporary) / "condition-fixture.msi")
                database = c_uint()
                self.assertEqual(msi.MsiOpenDatabaseW(database_path, c_void_p(3), byref(database)), 0)
                handles.append(database)
                queries = [
                    "CREATE TABLE `Property` (`Property` CHAR(72) NOT NULL, "
                    "`Value` CHAR(0) LOCALIZABLE PRIMARY KEY `Property`)"
                ]
                for key, value in {
                    "ProductCode": "{51F8A337-9819-46FD-8377-58A53EC1BD3A}",
                    "ProductLanguage": "1033", "ProductName": "Riviu condition fixture",
                    "ProductVersion": "1.0.0", "Manufacturer": "Fixture",
                }.items():
                    queries.append(f"INSERT INTO `Property` (`Property`, `Value`) VALUES ('{key}', '{value}')")
                for query in queries:
                    view = c_uint()
                    self.assertEqual(msi.MsiDatabaseOpenViewW(database, query, byref(view)), 0)
                    try:
                        self.assertEqual(msi.MsiViewExecute(view, 0), 0)
                    finally:
                        msi.MsiCloseHandle(view)
                summary = c_uint()
                self.assertEqual(msi.MsiGetSummaryInformationW(database, None, 3, byref(summary)), 0)
                try:
                    for prop, kind, number, text in [
                        (7, 30, 0, "x64;1033"),
                        (9, 30, 0, "{5863FFCB-878A-4FF8-87A0-A2B86B72497B}"),
                        (14, 3, 200, None),
                    ]:
                        self.assertEqual(msi.MsiSummaryInfoSetPropertyW(summary, prop, kind, number, None, text), 0)
                    self.assertEqual(msi.MsiSummaryInfoPersist(summary), 0)
                finally:
                    msi.MsiCloseHandle(summary)
                self.assertEqual(msi.MsiDatabaseCommit(database), 0)
                msi.MsiCloseHandle(handles.pop())
                session = c_uint()
                self.assertEqual(msi.MsiOpenPackageExW(database_path, 1, byref(session)), 0)
                handles.append(session)
                try:
                    condition = ET.fromstring(render_fragment([], win64=True)).find(
                        ".//{http://schemas.microsoft.com/wix/2006/wi}Condition"
                    )
                    self.assertIsNotNone(condition)
                    expression = condition.text.strip()
                    for build, expected in [
                        ("", 0), ("invalid", 0), ("#18362", 0), ("18362.0", 0),
                        ("9999", 0), ("17763", 0), ("18361", 0), ("18362", 1),
                        ("19045", 1), ("26200", 1), ("100000", 1),
                    ]:
                        with self.subTest(build=build):
                            self.assertEqual(msi.MsiSetPropertyW(session, "Installed", ""), 0)
                            self.assertEqual(msi.MsiSetPropertyW(session, "RIVIU_WINDOWS_BUILD", build), 0)
                            self.assertEqual(msi.MsiEvaluateConditionW(session, expression), expected)
                            self.assertEqual(msi.MsiSetPropertyW(session, "Installed", "1"), 0)
                            self.assertEqual(msi.MsiEvaluateConditionW(session, expression), 1)
                finally:
                    msi.MsiCloseHandle(handles.pop())
        finally:
            for handle in reversed(handles):
                msi.MsiCloseHandle(handle)
            ole.CoUninitialize()

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
