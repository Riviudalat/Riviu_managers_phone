from __future__ import annotations

import importlib.util
import json
import sys
import tempfile
import unittest
from pathlib import Path


AGENT_ROOT = Path(__file__).resolve().parents[1]
PROBE_PATH = AGENT_ROOT / "Scripts" / "probe_tiktok_comment.py"
PROMOTE_PATH = AGENT_ROOT / "Scripts" / "promote_text_candidate.py"


def load_module(name: str, path: Path):
    spec = importlib.util.spec_from_file_location(name, path)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"failed to load {path}")
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


probe = load_module("riviu_probe_tiktok_comment", PROBE_PATH)
promote = load_module("riviu_promote_text_candidate", PROMOTE_PATH)


class TikTokCommentProbeTests(unittest.TestCase):
    def test_comment_text_must_be_real_and_contentful(self):
        self.assertEqual(probe._validate_comment_text("  Cô nhảy dễ thương quá ạ "), "Cô nhảy dễ thương quá ạ")
        for value in ("", "abc", "Riviu test", "fixture comment", "sample comment"):
            with self.subTest(value=value):
                with self.assertRaises(probe.ProbeError):
                    probe._validate_comment_text(value)

    def test_control_client_tracks_session_rotation_from_route_envelope(self):
        client = probe.ControlClient("http://127.0.0.1:18100", "t" * 32)
        client._remember_session({"sessionId": "sid-before"})
        client._remember_session({"value": {"sessionId": "sid-after"}})
        self.assertEqual(client.session_id, "sid-after")

    def test_promotion_requires_live_frame_backed_send_evidence(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            frames = root / "frames"
            frames.mkdir()
            frame_paths = {}
            for name in ("before", "drawer", "armed", "sent"):
                path = frames / f"{name}.jpg"
                path.write_bytes(b"\xff\xd8fixture\xff\xd9")
                frame_paths[name] = str(path)
            evidence = {
                "environment": "LIVE_MAC_DEVICE",
                "gateStatus": "PASS",
                "targetBundle": probe.TARGET_BUNDLE,
                "sessionCreatedFresh": True,
                "commentText": "Cô nhảy dễ thương quá ạ",
                "composerArmed": True,
                "composerClearedAfterSend": True,
                "operatorConfirmedCommentVisible": True,
                "frames": frame_paths,
            }
            evidence_path = root / "evidence.json"
            evidence_path.write_text(json.dumps(evidence), encoding="utf-8")
            candidate_ipa = root / "RiviuAgent-candidate.ipa"
            candidate_ipa.write_bytes(b"candidate")
            candidate_manifest = root / "candidate-manifest.json"
            candidate_manifest.write_text(
                json.dumps(
                    {
                        "artifactId": "riviu-agent-ios-candidate",
                        "artifactVersion": "0.1.0",
                        "gateStatus": "PASS",
                        "protocolVersion": 2,
                        "ipa": candidate_ipa.name,
                        "features": ["stream", "tap", "swipe", "clipboard"],
                    }
                ),
                encoding="utf-8",
            )
            output_manifest = root / "text-manifest.json"
            output_ipa = root / "RiviuAgent-text.ipa"
            result = promote.promote(
                candidate_manifest,
                evidence_path,
                output_ipa,
                output_manifest,
            )
            self.assertIn("text", result["features"])
            self.assertEqual(result["gateStatus"], "PASS")
            self.assertTrue(output_ipa.is_file())

    def test_promotion_rejects_fixture_only_evidence(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            evidence_path = root / "evidence.json"
            evidence_path.write_text(
                json.dumps({"environment": "FIXTURE_ONLY", "gateStatus": "PASS"}),
                encoding="utf-8",
            )
            with self.assertRaises(promote.PromotionError):
                promote._validate_evidence(json.loads(evidence_path.read_text()), evidence_path)


if __name__ == "__main__":
    unittest.main()
