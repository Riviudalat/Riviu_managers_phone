from __future__ import annotations

import json
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from sidecars.wda import build_and_install


class PackagedSigningResourceTests(unittest.TestCase):
    def test_build_workspaces_are_stable_and_distinct_per_device(self):
        original = (
            build_and_install.BUILD_ROOT,
            build_and_install.WDA_SRC,
            build_and_install.DERIVED,
            build_and_install.PRODUCT_DIR,
        )
        try:
            build_and_install.configure_build_workspace("fixture-a")
            first = build_and_install.BUILD_ROOT
            build_and_install.configure_build_workspace("fixture-b")
            second = build_and_install.BUILD_ROOT
            build_and_install.configure_build_workspace("fixture-a")
            repeated = build_and_install.BUILD_ROOT
        finally:
            (
                build_and_install.BUILD_ROOT,
                build_and_install.WDA_SRC,
                build_and_install.DERIVED,
                build_and_install.PRODUCT_DIR,
            ) = original

        self.assertNotEqual(first, second)
        self.assertEqual(first, repeated)
        self.assertEqual(first.parent, build_and_install.WORK_ROOT / "devices")

    def test_checked_in_signing_resources_match_the_pinned_lock(self):
        lock = build_and_install.verify_resource_bundle()

        self.assertEqual(lock["package"], "appium-webdriveragent")
        self.assertEqual(lock["version"], "16.0.0")
        self.assertEqual(
            build_and_install.source_tree_sha256(
                build_and_install.SOURCE_TEMPLATE
            ),
            lock["treeSha256"],
        )

    def test_workspace_copy_is_verified_and_stays_outside_resources(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            resources = root / "resources"
            source = resources / "WebDriverAgent"
            (source / "WebDriverAgent.xcodeproj").mkdir(parents=True)
            (source / "package.json").write_text(
                json.dumps(
                    {"name": "appium-webdriveragent", "version": "16.0.0"}
                ),
                encoding="utf-8",
            )
            (source / "fixture.txt").write_text("pinned\n", encoding="utf-8")
            (resources / "logo.jpg").write_bytes(b"fixture-logo")
            (resources / "AppIcon.appiconset").mkdir()
            lock_path = resources / "legacy-wda-source-lock.json"
            lock_path.write_text(
                json.dumps(
                    {
                        "schemaVersion": 1,
                        "package": "appium-webdriveragent",
                        "version": "16.0.0",
                        "treeSha256": build_and_install.source_tree_sha256(source),
                        "logoSha256": build_and_install.hashlib.sha256(
                            (resources / "logo.jpg").read_bytes()
                        ).hexdigest(),
                        "iconSetTreeSha256": build_and_install.source_tree_sha256(
                            resources / "AppIcon.appiconset"
                        ),
                    }
                ),
                encoding="utf-8",
            )
            work = root / "user-cache"
            destination = work / "WebDriverAgent"

            with (
                patch.object(build_and_install, "RESOURCE_ROOT", resources),
                patch.object(build_and_install, "SOURCE_TEMPLATE", source),
                patch.object(build_and_install, "SOURCE_LOCK", lock_path),
                patch.object(build_and_install, "LOGO", resources / "logo.jpg"),
                patch.object(
                    build_and_install,
                    "ICONSET",
                    resources / "AppIcon.appiconset",
                ),
                patch.object(build_and_install, "WORK_ROOT", work),
                patch.object(build_and_install, "WDA_SRC", destination),
            ):
                build_and_install.ensure_wda_checkout()
                self.assertEqual(
                    build_and_install.source_tree_sha256(destination),
                    build_and_install.source_tree_sha256(source),
                )
                (destination / "fixture.txt").write_text(
                    "working-copy-change\n", encoding="utf-8"
                )
                self.assertEqual(
                    (source / "fixture.txt").read_text(encoding="utf-8"),
                    "pinned\n",
                )

                (source / "fixture.txt").write_text("tampered\n", encoding="utf-8")
                with self.assertRaisesRegex(RuntimeError, "integrity mismatch"):
                    build_and_install.verify_resource_bundle()


if __name__ == "__main__":
    unittest.main()
