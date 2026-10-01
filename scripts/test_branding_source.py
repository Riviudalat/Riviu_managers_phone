"""Branding path migration tests; no icon generation, signing or device I/O."""
import hashlib
import importlib.util
import json
from pathlib import Path
import sys
import tempfile
import unittest

from scripts import collect_desktop_ci_artifacts as collector
from sidecars.wda import build_and_install

ROOT = Path(__file__).resolve().parents[1]
CANONICAL = ROOT / "apps/desktop/public/logo.jpg"


class BrandingSourceTests(unittest.TestCase):
    def test_canonical_logo_keeps_the_pinned_bytes(self):
        lock = json.loads((ROOT / "sidecars/wda/legacy-wda-source-lock.json").read_text())
        self.assertEqual(hashlib.sha256(CANONICAL.read_bytes()).hexdigest(), lock["logoSha256"])
        self.assertEqual(collector.BRANDING_LOGO, CANONICAL)
        self.assertEqual(build_and_install.DEVELOPMENT_LOGO, CANONICAL)
        self.assertFalse((ROOT / "logo.jpg").exists())

    def test_tauri_maps_canonical_source_to_unchanged_packaged_location(self):
        path = ROOT / "apps/desktop/src-tauri/tauri.conf.json"
        resources = json.loads(path.read_text(encoding="utf8"))["bundle"]["resources"]
        sources = [source for source, target in resources.items() if target == "sidecars/wda/logo.jpg"]
        self.assertEqual(sources, ["../public/logo.jpg"])
        self.assertEqual((path.parent / sources[0]).resolve(), CANONICAL)
        self.assertNotIn("../../../logo.jpg", resources)

    def test_packaged_signer_does_not_need_the_checkout(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary) / "sidecars/wda"
            root.mkdir(parents=True)
            script = root / "build_and_install.py"
            script.write_bytes((ROOT / "sidecars/wda/build_and_install.py").read_bytes())
            logo = root / "logo.jpg"
            logo.write_bytes(CANONICAL.read_bytes())
            name = "packaged_branding_fixture"
            spec = importlib.util.spec_from_file_location(name, script)
            assert spec is not None and spec.loader is not None
            module = importlib.util.module_from_spec(spec)
            sys.modules[name] = module
            try:
                spec.loader.exec_module(module)
                self.assertEqual(module.LOGO, logo)
                self.assertFalse(module.DEVELOPMENT_LOGO.exists())
            finally:
                sys.modules.pop(name, None)

    def test_candidate_build_uses_the_same_brand_source(self):
        path = ROOT / "sidecars/wda/riviu-agent/Scripts/build_candidate.py"
        spec = importlib.util.spec_from_file_location("candidate_brand_fixture", path)
        assert spec is not None and spec.loader is not None
        module = importlib.util.module_from_spec(spec)
        sys.modules[spec.name] = module
        try:
            spec.loader.exec_module(module)
            self.assertEqual(module.BRAND_LOGO, CANONICAL)
        finally:
            sys.modules.pop(spec.name, None)


if __name__ == "__main__":
    unittest.main()
