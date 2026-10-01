"""Local, single-run evidence protocol. Confirmation never contacts a device."""
from __future__ import annotations

import hashlib
import json
import os
import uuid
from pathlib import Path


class EvidenceError(RuntimeError):
    pass


def digest(path: Path) -> str:
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def atomic_json(path: Path, value: dict) -> None:
    temporary = path.with_name(path.name + "." + uuid.uuid4().hex + ".tmp")
    try:
        with temporary.open("x", encoding="utf8") as stream:
            json.dump(value, stream, ensure_ascii=False, indent=2)
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(temporary, path)
    finally:
        temporary.unlink(missing_ok=True)


def claim(output: Path, frames: Path, identity: dict) -> dict:
    if not output.is_absolute() or not frames.is_absolute():
        raise EvidenceError("Use absolute evidence and frames paths")
    output.parent.mkdir(parents=True, exist_ok=True)
    intent = output.with_suffix(output.suffix + ".intent.json")
    if output.exists() or frames.exists():
        raise EvidenceError("Evidence/frames already exist; inspect the prior run, never replay Send")
    value = {"schemaVersion": 2, "runId": str(uuid.uuid4()), "identity": identity,
             "output": str(output), "framesDirectory": str(frames), "state": "claimed"}
    try:
        with intent.open("x", encoding="utf8") as stream:
            json.dump(value, stream, ensure_ascii=False)
            stream.flush()
            os.fsync(stream.fileno())
    except FileExistsError as exc:
        raise EvidenceError("Run intent exists, even if incomplete; execution is refused") from exc
    # Never remove the intent after failure. A different directory is not a safe retry.
    frames.mkdir(parents=True, exist_ok=False)
    return value


def pending(output: Path, intent: dict, evidence: dict) -> dict:
    evidence.update(schemaVersion=2, runId=intent["runId"], identity=intent["identity"],
                    gateStatus="PENDING_OPERATOR", operatorConfirmedCommentVisible=False)
    evidence["frameHashes"] = {name: digest(Path(path)) for name, path in evidence["frames"].items()}
    evidence["intentSha256"] = digest(output.with_suffix(output.suffix + ".intent.json"))
    atomic_json(output, evidence)
    return evidence


def validate(output: Path, evidence: dict, run_id: str) -> None:
    intent_path = output.with_suffix(output.suffix + ".intent.json")
    intent = json.loads(intent_path.read_text(encoding="utf8"))
    if (evidence.get("schemaVersion") != 2 or evidence.get("runId") != run_id
            or intent.get("runId") != run_id or evidence.get("identity") != intent.get("identity")
            or intent.get("output") != str(output)
            or evidence.get("intentSha256") != digest(intent_path)):
        raise EvidenceError("Evidence/intent identity mismatch")
    identity = evidence.get("identity", {})
    if not all(identity.get(k) for k in ("deviceId", "candidateSha256", "manifestSha256", "targetBundle", "commentSha256")):
        raise EvidenceError("Incomplete candidate/device binding")
    if hashlib.sha256(evidence.get("commentText", "").encode("utf8")).hexdigest() != identity["commentSha256"]:
        raise EvidenceError("Comment identity mismatch")
    if evidence.get("targetBundle") != identity["targetBundle"]:
        raise EvidenceError("Target identity mismatch")
    root = Path(intent["framesDirectory"]).resolve(strict=True)
    for name in ("before", "drawer", "armed", "sent"):
        path = Path(evidence["frames"][name])
        if path.is_symlink() or path.resolve(strict=True).parent != root:
            raise EvidenceError("Frame is outside the run")
        if digest(path) != evidence["frameHashes"].get(name):
            raise EvidenceError("Frame hash mismatch")
    if not all(evidence.get(k) is True for k in ("sessionCreatedFresh", "composerArmed", "composerClearedAfterSend")):
        raise EvidenceError("Insufficient post-send evidence")


def confirm(output: Path, run_id: str) -> dict:
    # The lock is fail-closed after a crash; no send is ever retried by this module.
    lock = output.with_suffix(output.suffix + ".confirm.lock")
    try:
        with lock.open("x"):
            pass
    except FileExistsError as exc:
        raise EvidenceError("Confirmation is already active or interrupted") from exc
    try:
        evidence = json.loads(output.read_text(encoding="utf8"))
        validate(output, evidence, run_id)
        if evidence.get("gateStatus") not in ("PENDING_OPERATOR", "OPERATOR_CONFIRMED"):
            raise EvidenceError("Only a complete pending observation can be confirmed")
        evidence.update(gateStatus="OPERATOR_CONFIRMED", operatorConfirmedCommentVisible=True)
        atomic_json(output, evidence)
        return evidence
    finally:
        lock.unlink(missing_ok=True)
