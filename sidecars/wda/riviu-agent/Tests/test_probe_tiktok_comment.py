from __future__ import annotations

import importlib.util
import json
import sys
import tempfile
import unittest
from unittest import mock
import hashlib
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
            frame_paths = {}
            for name in ("before", "drawer", "armed", "sent"):
                path = frames / f"{name}.jpg"
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
            identity = {"deviceId": "fixture-device", "targetBundle": probe.TARGET_BUNDLE,
                        "commentSha256": hashlib.sha256(evidence["commentText"].encode()).hexdigest(),
                        "candidateSha256": probe.comment_evidence.digest(candidate_ipa),
                        "manifestSha256": probe.comment_evidence.digest(candidate_manifest)}
            intent = probe.comment_evidence.claim(evidence_path, frames, identity)
            for path in frame_paths.values():
                Path(path).write_bytes(bytes([255, 216]) + b"fixture" + bytes([255, 217]))
            probe.comment_evidence.pending(evidence_path, intent, evidence)
            with mock.patch.object(probe, "ControlClient", side_effect=AssertionError("no transport")), mock.patch.object(probe.subprocess, "run", side_effect=AssertionError("no subprocess")):
                probe.comment_evidence.confirm(evidence_path, intent["runId"])
                probe.comment_evidence.confirm(evidence_path, intent["runId"])
            output_manifest = root / "text-manifest.json"
            output_ipa = root / "RiviuAgent-text.ipa"
            with self.assertRaises(promote.PromotionError):
                promote.promote(candidate_manifest, evidence_path, output_ipa, output_manifest)
            self.assertFalse(output_ipa.exists())
            self.assertFalse(output_manifest.exists())
            self.assertEqual(json.loads(evidence_path.read_text(encoding="utf8"))["gateStatus"], "OPERATOR_CONFIRMED")
            with mock.patch.object(probe, "ControlClient", side_effect=AssertionError("transport")), mock.patch.object(probe.subprocess, "run", side_effect=AssertionError("sidecar")), mock.patch.object(probe.os.environ, "get", side_effect=lambda key, default=None: (_ for _ in ()).throw(AssertionError("token access")) if key == probe.TOKEN_ENV else default), mock.patch.object(probe.sys, "argv", ["probe", "--confirm-evidence", str(evidence_path), "--run-id", intent["runId"], "--operator-confirmed-comment-visible"]):
                self.assertEqual(probe.main(), 0)
            with self.assertRaises(probe.comment_evidence.EvidenceError):
                probe.comment_evidence.confirm(evidence_path, "wrong-run")
            Path(frame_paths["sent"]).write_bytes(b"tampered")
            with self.assertRaises(probe.comment_evidence.EvidenceError):
                probe.comment_evidence.confirm(evidence_path, intent["runId"])


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

    def test_existing_or_partial_intent_refuses_without_creating_frames(self):
        for content in ("", "{", '{"state":"claimed"}'):
            with self.subTest(content=content), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                output = root / "evidence.json"
                output.with_suffix(".json.intent.json").write_text(content)
                with self.assertRaises(probe.comment_evidence.EvidenceError):
                    probe.comment_evidence.claim(output, root / "frames", {})
                self.assertFalse((root / "frames").exists())

    def test_only_one_invocation_claims_a_run(self):
        from concurrent.futures import ThreadPoolExecutor
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            def attempt():
                try:
                    probe.comment_evidence.claim(root / "evidence.json", root / "frames", {})
                    return True
                except (probe.comment_evidence.EvidenceError, FileExistsError):
                    return False
            with ThreadPoolExecutor(max_workers=2) as workers:
                self.assertEqual(sum(workers.map(lambda _: attempt(), range(2))), 1)

    def test_lost_ack_after_send_does_not_allow_a_second_execution(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            ipa = root / "candidate.ipa"
            ipa.write_bytes(b"fixture-ipa")
            manifest = root / "candidate.json"
            manifest.write_text(json.dumps({"ipa":ipa.name,"sha256":probe.comment_evidence.digest(ipa)}))
            config = probe.ProbeConfig("fixture-phone", "http://127.0.0.1:1", 9094,
                "Nội dung đã duyệt", (1,1), (2,2), (3,3), root/"proof.json", root/"frames",
                root/"fake-sidecar", False, manifest, True, True)
            client = mock.Mock()
            client.fresh_session.return_value = "session"
            client.session_id = "session"
            def tap(x, y):
                if (x, y) == (3, 3):
                    raise probe.ProbeError("lost Send ACK")
            client.tap.side_effect = tap
            with mock.patch.dict(probe.os.environ, {probe.TOKEN_ENV:"t"*32}), mock.patch.object(probe, "ControlClient", return_value=client), mock.patch.object(probe.subprocess, "run", return_value=mock.Mock(stdout='{"ok": true}')), mock.patch.object(probe, "_capture_frame", return_value=root/"unused.jpg"), mock.patch.object(probe, "_send_button_redness", return_value=1), mock.patch.object(probe.time, "sleep"):
                with self.assertRaises(probe.ProbeError):
                    probe.run(config)
                self.assertEqual(client.tap.call_count, 3)
                with self.assertRaises(probe.comment_evidence.EvidenceError):
                    probe.run(config)
                self.assertEqual(client.tap.call_count, 3)
            self.assertTrue(config.output.with_suffix(".json.intent.json").exists())

    def test_preconfirmation_refused_before_any_device_work(self):
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory)
            config=probe.ProbeConfig("fixture", "http://127.0.0.1:1", 9094, "Nội dung thật",
                (1,1),(2,2),(3,3),root/"proof.json",root/"frames",root/"sidecar",True,root/"missing.json",True,True)
            with mock.patch.object(probe.subprocess,"run",side_effect=AssertionError("device work")), mock.patch.object(probe,"ControlClient",side_effect=AssertionError("transport")):
                with self.assertRaises(probe.ProbeError):
                    probe.run(config)
            self.assertFalse(config.output.exists())

    def test_historical_evidence_cannot_be_promoted_again(self):
        with tempfile.TemporaryDirectory() as directory:
            with self.assertRaises(promote.PromotionError):
                promote._validate_evidence({"gateStatus":"PASS", "environment":"LIVE_MAC_DEVICE"}, Path(directory)/"old.json")


if __name__ == "__main__":
    unittest.main()
