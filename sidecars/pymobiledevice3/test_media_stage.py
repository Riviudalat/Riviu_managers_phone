import hashlib
import tempfile
import unittest
from unittest import mock
from pathlib import Path

from sidecars.pymobiledevice3 import riviu_pmd


class MediaStageManifestTests(unittest.TestCase):
    def test_manifest_is_sorted_and_hashes_each_managed_file(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "bundle-2").mkdir()
            (root / "bundle-1").mkdir()
            (root / "bundle-2" / "02.png").write_bytes(b"two")
            (root / "bundle-2" / "01.png").write_bytes(b"one")
            (root / "bundle-2" / "caption.txt").write_text("caption", encoding="utf-8")
            (root / "bundle-1" / "01.jpg").write_bytes(b"jpg")
            (root / "bundle-1" / "caption.txt").write_text("caption", encoding="utf-8")
            (root / ".DS_Store").write_bytes(b"ignored")

            manifest = riviu_pmd._media_file_manifest(root)

            self.assertEqual(
                [(entry["bundle"], entry["file"]) for entry in manifest],
                [
                    ("bundle-1", "01.jpg"),
                    ("bundle-1", "caption.txt"),
                    ("bundle-2", "01.png"),
                    ("bundle-2", "02.png"),
                    ("bundle-2", "caption.txt"),
                ],
            )
            one = next(entry for entry in manifest if entry["file"] == "01.png")
            self.assertEqual(one["sha256"], hashlib.sha256(b"one").hexdigest())
            self.assertEqual(one["kind"], "image")

    def test_manifest_rejects_unapproved_files(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            bundle = root / "bundle"
            bundle.mkdir()
            (bundle / "01.mp4").write_bytes(b"video")
            with self.assertRaisesRegex(ValueError, "unsupported media file"):
                riviu_pmd._media_file_manifest(root)

    def test_campaign_id_validation_happens_before_device_access(self):
        class Args:
            udid = "fixture"
            agent_bundle_id = "com.riviu.agent"
            campaign_id = "../escape"
            source_root = "/tmp/missing"

        with mock.patch.object(riviu_pmd, "try_import", return_value=True):
            emitted = []
            with mock.patch.object(riviu_pmd, "emit", emitted.append):
                self.assertEqual(riviu_pmd.cmd_media_stage(Args()), 2)
        self.assertEqual(emitted, [])


if __name__ == "__main__":
    unittest.main()
