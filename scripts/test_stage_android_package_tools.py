from __future__ import annotations

import hashlib
import stat
import tempfile
import unittest
from pathlib import Path
import zipfile

from scripts import stage_android_package_tools as tools


class AndroidPackageToolsStageTests(unittest.TestCase):
    def test_tool_versions_are_checked_exactly(self):
        tools.verify_tool_versions(
            'openjdk version "21.0.12.1"\nOpenJDK Runtime Environment Temurin-21.0.12.1+1',
            "1.18.3",
        )
        with self.assertRaisesRegex(tools.StageError, "Java version"):
            tools.verify_tool_versions("openjdk version 21.0.11", "1.18.3")
        with self.assertRaisesRegex(tools.StageError, "Bundletool version"):
            tools.verify_tool_versions(
                "OpenJDK Runtime Environment Temurin-21.0.12.1+1",
                "1.18.30",
            )

    def test_hash_and_size_are_both_required_before_extract(self):
        with tempfile.TemporaryDirectory() as temporary:
            payload = Path(temporary) / "artifact"
            payload.write_bytes(b"same-size-bad")
            expected = tools.PinnedArtifact(
                "artifact", "https://invalid.example/artifact", payload.stat().st_size, "0" * 64
            )
            with self.assertRaisesRegex(tools.StageError, "SHA-256 mismatch"):
                tools.verify_pinned(payload, expected)

    def test_jre_zip_rejects_traversal_and_symlinks(self):
        for name, configure in (
            ("root/../../escape", lambda info: None),
            ("root/link", lambda info: setattr(info, "external_attr", (stat.S_IFLNK | 0o777) << 16)),
        ):
            with self.subTest(name=name), tempfile.TemporaryDirectory() as temporary:
                archive = Path(temporary) / "jre.zip"
                with zipfile.ZipFile(archive, "w") as output:
                    info = zipfile.ZipInfo(name)
                    configure(info)
                    output.writestr(info, b"payload")
                with self.assertRaises(tools.StageError):
                    tools.extract_jre(archive, Path(temporary) / "out")

    def test_jre_zip_rejects_windows_alternate_data_stream_paths(self):
        with tempfile.TemporaryDirectory() as temporary:
            archive = Path(temporary) / "jre.zip"
            with zipfile.ZipFile(archive, "w") as output:
                output.writestr("root/bin/java.exe", b"java")
                output.writestr("root/bin/java.exe:hidden", b"payload")
            with self.assertRaisesRegex(tools.StageError, "non-portable"):
                tools.extract_jre(archive, Path(temporary) / "out")

    def test_jre_zip_rejects_duplicate_entries(self):
        with tempfile.TemporaryDirectory() as temporary:
            archive = Path(temporary) / "jre.zip"
            with zipfile.ZipFile(archive, "w") as output:
                output.writestr("root/bin/java.exe", b"first")
                with self.assertWarns(UserWarning):
                    output.writestr("root/bin/java.exe", b"second")
            with self.assertRaisesRegex(tools.StageError, "duplicate entry"):
                tools.extract_jre(archive, Path(temporary) / "out")

    def test_jre_zip_rejects_case_colliding_windows_entries(self):
        with tempfile.TemporaryDirectory() as temporary:
            archive = Path(temporary) / "jre.zip"
            with zipfile.ZipFile(archive, "w") as output:
                output.writestr("root/bin/java.exe", b"first")
                output.writestr("ROOT/BIN/JAVA.EXE", b"second")
            with self.assertRaisesRegex(tools.StageError, "case-colliding entry"):
                tools.extract_jre(archive, Path(temporary) / "out")

    def test_complete_tree_manifest_rejects_missing_changed_and_extra_files(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / "bundletool.jar").write_bytes(b"jar")
            (root / "jre" / "bin").mkdir(parents=True)
            (root / "jre" / "bin" / "java.exe").write_bytes(b"java")
            manifest = {"files": tools.tree_files(root)}
            tools.verify_tree_manifest(root, manifest)
            (root / "extra.dll").write_bytes(b"extra")
            with self.assertRaisesRegex(tools.StageError, "complete manifest"):
                tools.verify_tree_manifest(root, manifest)

    def test_manifest_entries_are_stable_and_hash_exact_bytes(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / "z").write_bytes(b"z")
            (root / "a").write_bytes(b"a")
            entries = tools.tree_files(root)
            self.assertEqual([entry["path"] for entry in entries], ["a", "z"])
            self.assertEqual(entries[0]["sha256"], hashlib.sha256(b"a").hexdigest())
            self.assertEqual(
                tools.tree_sha256(entries),
                tools.tree_sha256(list(reversed(entries))),
            )
            changed = [dict(entry) for entry in entries]
            changed[0]["bytes"] = 2
            self.assertNotEqual(tools.tree_sha256(entries), tools.tree_sha256(changed))


if __name__ == "__main__":
    unittest.main()
