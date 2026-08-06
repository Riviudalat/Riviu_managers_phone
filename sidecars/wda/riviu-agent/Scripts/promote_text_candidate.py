#!/usr/bin/env python3
"""Promote a frame-backed live comment proof into a separate text artifact."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import shutil
from pathlib import Path
from typing import Any


TARGET_BUNDLE = "com.ss.iphone.ugc.Ame"
PLACEHOLDER_WORDS = ("riviu test", "fixture", "placeholder", "sample comment")


class PromotionError(RuntimeError):
    pass


def _sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def _read_json(path: Path) -> dict[str, Any]:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as exc:
        raise PromotionError(f"invalid JSON: {path}") from exc
    if not isinstance(value, dict):
        raise PromotionError(f"JSON root must be an object: {path}")
    return value


def _validate_evidence(evidence: dict[str, Any], evidence_path: Path) -> str:
    if evidence.get("environment") != "LIVE_MAC_DEVICE":
        raise PromotionError("text promotion requires LIVE_MAC_DEVICE evidence")
    if evidence.get("gateStatus") != "PASS":
        raise PromotionError("text promotion requires a passing comment gate")
    if evidence.get("targetBundle") != TARGET_BUNDLE:
        raise PromotionError("comment evidence targets a different application")
    for field in (
        "sessionCreatedFresh",
        "composerArmed",
        "composerClearedAfterSend",
        "operatorConfirmedCommentVisible",
    ):
        if evidence.get(field) is not True:
            raise PromotionError(f"comment evidence is missing {field}=true")
    text = evidence.get("commentText")
    if not isinstance(text, str) or len(text.encode("utf-8")) < 4:
        raise PromotionError("comment evidence has no real UTF-8 comment text")
    lowered = text.casefold()
    if any(marker in lowered for marker in PLACEHOLDER_WORDS):
        raise PromotionError("comment evidence contains a probe placeholder")
    frames = evidence.get("frames")
    if not isinstance(frames, dict):
        raise PromotionError("comment evidence has no frame set")
    for name in ("before", "drawer", "armed", "sent"):
        value = frames.get(name)
        if not isinstance(value, str) or not Path(value).is_file():
            raise PromotionError(f"comment evidence is missing frame {name}")
    return _sha256(evidence_path)


def promote(
    candidate_manifest_path: Path,
    evidence_path: Path,
    output_ipa: Path,
    output_manifest: Path,
) -> dict[str, Any]:
    candidate = _read_json(candidate_manifest_path)
    evidence = _read_json(evidence_path)
    evidence_sha256 = _validate_evidence(evidence, evidence_path)
    candidate_ipa = candidate_manifest_path.parent / str(candidate.get("ipa", ""))
    if not candidate_ipa.is_file():
        raise PromotionError(f"candidate IPA is missing: {candidate_ipa}")
    if candidate.get("protocolVersion") != 2:
        raise PromotionError("text promotion requires protocol version 2")

    features = list(candidate.get("features", []))
    if "text" not in features:
        features.append("text")
    manifest = dict(candidate)
    manifest.update(
        {
            "artifactId": "riviu-agent-ios-text",
            "artifactVersion": "0.2.0-text",
            "gateStatus": "PASS",
            "ipa": output_ipa.name,
            "sha256": "",
            "features": features,
            "textGate": {
                "environment": evidence["environment"],
                "evidenceSha256": evidence_sha256,
                "commentText": evidence["commentText"],
                "commentFrameConfirmed": True,
            },
        }
    )

    output_ipa.parent.mkdir(parents=True, exist_ok=True)
    temporary_ipa = output_ipa.with_suffix(output_ipa.suffix + ".tmp")
    shutil.copyfile(candidate_ipa, temporary_ipa)
    os.replace(temporary_ipa, output_ipa)
    manifest["sha256"] = _sha256(output_ipa)

    output_manifest.parent.mkdir(parents=True, exist_ok=True)
    temporary_manifest = output_manifest.with_suffix(output_manifest.suffix + ".tmp")
    temporary_manifest.write_text(
        json.dumps(manifest, ensure_ascii=False, indent=2) + "\n", encoding="utf-8"
    )
    os.replace(temporary_manifest, output_manifest)
    return manifest


def _parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--candidate-manifest", type=Path, required=True)
    parser.add_argument("--evidence", type=Path, required=True)
    parser.add_argument(
        "--output-ipa", type=Path, default=Path("sidecars/wda/RiviuAgent-text.ipa")
    )
    parser.add_argument(
        "--output-manifest", type=Path, default=Path("sidecars/wda/text-manifest.json")
    )
    return parser


def main() -> int:
    args = _parser().parse_args()
    try:
        manifest = promote(
            args.candidate_manifest,
            args.evidence,
            args.output_ipa,
            args.output_manifest,
        )
    except PromotionError as exc:
        print(json.dumps({"ok": False, "error": str(exc)}, ensure_ascii=True))
        return 1
    print(
        json.dumps(
            {
                "ok": True,
                "artifactId": manifest["artifactId"],
                "manifest": str(args.output_manifest),
                "ipa": str(args.output_ipa),
                "features": manifest["features"],
            },
            ensure_ascii=True,
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
