# TikTok Interaction New Action Gates Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add independently qualified Save, Repost, allowlisted Direct Message, and random-visible Direct Message actions with frame-derived state proof, durable side-effect intent, pinned local OCR, exact recipient matching, evidence artifacts, and action-specific rollback.

**Architecture:** Extend the G2 `tiktok_actions` facade instead of adding a second automation engine. Save remains an idempotent desired-state transition; Repost and Direct Message commit `effect_intent=issued` immediately before the final side-effect tap and become `Uncertain` after ambiguous completion. Direct Message uses an injectable local recognizer in `riviu-core`, an `ocrs` adapter in the Tauri composition root, and the existing Gate 0 qualification registry; every action starts disabled and is enabled only for an exact live-qualified capability tuple.

**Tech Stack:** Rust 2021 with Rust 1.89.0 minimum for the OCR build, Tokio, async-trait, image 0.25, ocrs 0.12.2, RTen 0.24.0, serde/serde_json, SHA-256, Tauri 2 resources, Python 3.9+ verification tools, pymobiledevice3 10.1.0, Pillow 11.3.0, MJPEG, SQLite through `InteractionStore`.

---

## Execution Preconditions

- Complete G0 through G3 in the roadmap before starting: `2026-07-29-tiktok-interaction-gate-0-device-control.md`, `2026-07-29-tiktok-interaction-campaign-core.md`, `2026-07-29-tiktok-interaction-verified-actions.md`, and `2026-07-29-tiktok-interaction-desktop-workflow.md`.
- Execute in an isolated worktree created with `using-git-worktrees`. The current shared checkout contains unrelated Project 2/runtime work; stage only the paths named by each commit step.
- Re-read `AGENTS.md` and the approved design at commit `10433fb` before every device or capability task. Preserve session-before-MJPEG, the shared coordinator lease, the shared stream budget, generation-safe frames, request-local WDA deadlines, and exact profile selection.
- G2 must expose `FrameProbe`, `ActionContext`, `VerifiedEffect`, `QualifiedGeometry`, `LocatedActionRail`, `IdentityUiContext`, `InteractionProgress`, `PreparedActionPayload`, `ActionOutcomeCode`, `EvidenceRef`, `TikTokActionExecutor`, and `InteractionBatchExecutor`. Its canonical `TikTokActionExecutor::execute_action` receives `&AssignmentExecution` so preparation can consume the immutable effective settings and `assignment_seed`; G4 reuses that signature. Its `UiCapabilities.interaction_runtime` exact-tuple gate must already attest the production Rust identity/Watch executor, and every G4 capability extends rather than replaces that base runtime qualification. Reconcile names once at the start; do not create parallel action/state enums.
- Production `sidecars/wda/RiviuAgent.ipa` and `sidecars/wda/agent-manifest.json` must remain byte-identical. Their accepted SHA-256 values are `8a24847099495ff70b998522692c43f00dd16b90f698bda6953a73f5d33002ea` and `e98a549af4c061556effd36424e7732219e1a6d262bcf1f259279975024b6e1a`.
- `sidecars/wda/interaction-capabilities.json` remains the single Interaction qualification registry. This plan extends it; it does not create a second registry or infer support from `AgentStatus.features`.
- Fixture runs and generated reports use `FIXTURE_ONLY` and never produce a production capability entry. Only a fixed `LIVE_MAC_DEVICE` run can produce `PASS`.
- Direct Message remains disabled when OCR model files are missing, fail checksum, fail to load, fail fixture accuracy, or fail the exact live tuple. Save and Repost remain independently eligible.
- All frame reads come from `FrameSource`/MJPEG. Do not add `UiSession::screenshot_png`, `DeviceDriver::screenshot`, or polling `GET /screenshot` to these actions or probes.

## Locked OCR Decision

The recipient recognizer is local and limited to ASCII handles. Display labels are retained only for operator presentation and never participate in matching.

```text
engine crate: ocrs 0.12.2
engine source commit: 2dbc1f840e47d45630ef6060499138bf597a9f65
runtime crate: rten 0.24.0
minimum Rust: 1.89.0
model source revision: df0edd170279ab971b53e094c627255a87e1a503
model license: CC-BY-SA-4.0
model-set digest: fa3c0f3aedb139813d434fe9bdd9d12ce1685cca0f26af46e006ca8ce583ef14
```

The upstream library does not expose a calibrated recognition posterior. `RecipientLocator` therefore records a clearly named `consensus_confidence_bps`: it recognizes three deterministic preprocessing variants and accepts a handle only when all three normalize to the same exact ASCII `@handle`, yielding `10000`. Two-of-three yields `6667`, one-of-three yields `3333`, and neither value is tappable. The fixture and live gates validate this consensus rule; the field must not be described as a neural-model probability.

## File Map

**Create**

- `sidecars/ocr/model-lock.json`: immutable engine/model source, sizes, checksums, aggregate digest, and license metadata.
- `sidecars/ocr/fetch_models.py`: bounded atomic model acquisition and offline verification.
- `sidecars/ocr/test_fetch_models.py`: lock, checksum, size-cap, and atomic-replace tests.
- `sidecars/ocr/ATTRIBUTION.md`: ocrs and model attribution/license record.
- `sidecars/ocr/models/.gitkeep`: retained model directory; downloaded `.rten` files remain ignored.
- `apps/desktop/src-tauri/src/interaction_ocr.rs`: checksum-validating `ocrs` adapter and typed availability state.
- `crates/core/src/tiktok_actions/save.rs`: Save detector and bounded desired-state executor.
- `crates/core/src/tiktok_actions/share.rs`: Repost/Remove Repost, Direct Message submit-state locators, and share action executors.
- `crates/core/src/tiktok_actions/recipient.rs`: recognizer port, exact handle normalization, consensus, ambiguity rejection, and deterministic random-visible selection.
- `crates/core/tests/tiktok_new_actions.rs`: fake-session/frame/progress tests for all G4 action semantics.
- `crates/core/tests/interaction_new_action_persistence.rs`: prepared-recipient, intent ordering, crash, retry, and evidence tests.
- `crates/core/tests/fixtures/interaction/g4/manifest.json`: fixture provenance, labels, split, dimensions, tuple, and hashes.
- `crates/core/tests/fixtures/interaction/g4/save-unsaved-calibration.jpg`
- `crates/core/tests/fixtures/interaction/g4/save-saved-calibration.jpg`
- `crates/core/tests/fixtures/interaction/g4/save-unsaved-holdout-a.jpg`
- `crates/core/tests/fixtures/interaction/g4/save-saved-holdout-a.jpg`
- `crates/core/tests/fixtures/interaction/g4/save-unsaved-holdout-b.jpg`
- `crates/core/tests/fixtures/interaction/g4/save-saved-holdout-b.jpg`
- `crates/core/tests/fixtures/interaction/g4/save-ambiguous.jpg`
- `crates/core/tests/fixtures/interaction/g4/repost-calibration.jpg`
- `crates/core/tests/fixtures/interaction/g4/remove-repost-calibration.jpg`
- `crates/core/tests/fixtures/interaction/g4/repost-holdout-a.jpg`
- `crates/core/tests/fixtures/interaction/g4/remove-repost-holdout-a.jpg`
- `crates/core/tests/fixtures/interaction/g4/repost-ambiguous.jpg`
- `crates/core/tests/fixtures/interaction/g4/dm-visible-three-a.jpg`
- `crates/core/tests/fixtures/interaction/g4/dm-visible-three-b.jpg`
- `crates/core/tests/fixtures/interaction/g4/dm-search-empty-calibration.jpg`
- `crates/core/tests/fixtures/interaction/g4/dm-search-empty-holdout.jpg`
- `crates/core/tests/fixtures/interaction/g4/dm-allowlist-search-result-a.jpg`
- `crates/core/tests/fixtures/interaction/g4/dm-allowlist-search-result-b.jpg`
- `crates/core/tests/fixtures/interaction/g4/dm-duplicate-handle.jpg`
- `crates/core/tests/fixtures/interaction/g4/dm-low-consensus.jpg`
- `crates/core/tests/fixtures/interaction/g4/dm-submit-ready-calibration.jpg`
- `crates/core/tests/fixtures/interaction/g4/dm-submit-ready-holdout.jpg`
- `crates/core/tests/fixtures/interaction/g4/dm-send-confirmed-calibration.jpg`
- `crates/core/tests/fixtures/interaction/g4/dm-send-confirmed-holdout.jpg`
- `crates/core/tests/fixtures/interaction/g4/dm-send-ambiguous.jpg`
- `crates/core/src/templates/interaction/bookmark-unsaved-v1.png`
- `crates/core/src/templates/interaction/bookmark-saved-v1.png`
- `crates/core/src/templates/interaction/repost-v1.png`
- `crates/core/src/templates/interaction/remove-repost-v1.png`
- `crates/core/src/templates/interaction/dm-search-v1.png`
- `crates/core/src/templates/interaction/dm-submit-contract-v1.png`
- `crates/core/src/templates/interaction/dm-submit-ready-v1.png`
- `crates/core/src/templates/interaction/dm-send-confirmed-v1.png`
- `crates/core/src/templates/interaction/detector-set-g4-v1.json`
- `tools/interaction-gate4/fixture_manifest.py`: fixture import, crop extraction, threshold calibration, and manifest validation.
- `tools/interaction-gate4/test_fixture_manifest.py`: generated-fixture and calibration tests.
- `tools/interaction-gate4/requirements.txt`: exact `Pillow==11.3.0` dependency for capture and detector generation.
- `tools/interaction-gate4/verify_report.py`: fixed-threshold report verification, transactional publication, promotion, and revocation.
- `tools/interaction-gate4/test_verify_report.py`: fixture/live separation, tuple, intent, evidence, promotion, rollback, and redaction tests.
- `apps/desktop/src-tauri/src/bin/live_interaction_new_action_gates.rs`: fixed G4 real-device harness.
- `docs/re/interaction-gate4/README.md`: operator procedure, artifact contract, and rollback commands.

**Modify**

- `Cargo.toml`: pin `ocrs` and `rten` workspace dependencies.
- `Cargo.lock`: lock the exact OCR/runtime transitive dependency graph generated by Rust 1.89.0.
- `.gitignore`: ignore only downloaded OCR model binaries and local G4 raw captures.
- `apps/desktop/src-tauri/Cargo.toml`: raise the desktop MSRV to 1.89.0 and add the pinned OCR dependencies.
- `apps/desktop/src-tauri/tauri.conf.json`: package the OCR lock, attribution, and both verified model files.
- `apps/desktop/src-tauri/src/lib.rs`: register the OCR runtime module.
- `apps/desktop/src-tauri/src/state.rs`: load OCR as ready/unavailable without failing desktop bootstrap and inject it into G4 actions.
- `crates/core/src/types.rs`: make logical `TapPoint` comparable and own shared checked normalized point/rectangle DTOs.
- `crates/core/src/tiktok_actions/mod.rs`: export G4 modules and add a scoped watcher-suppression guard.
- `crates/core/src/tiktok_actions/frame_probe.rs`: retain a byte-bounded recent JPEG cache long enough to materialize evidence by SHA-256 without another device read.
- `crates/core/src/interaction/types.rs`: recipient modes, prepared payloads, typed outcome codes, and evidence DTOs.
- `crates/core/src/interaction/progress.rs`: permit a pre-intent prepared-payload CAS for resolved recipient evidence.
- `crates/core/src/interaction/artifacts.rs`: persist exact recipient crops and crash-reconcile failed pre-intent attachment cleanup.
- `crates/core/src/interaction/store.rs`: persist the prepared-payload CAS, artifact discard state, and issued-intent retry exclusion.
- `crates/core/src/interaction/device_batch_executor.rs`: execute Save/Repost/Direct Message in the approved order and close Share between methods.
- `crates/core/src/device_capabilities.rs`: independently typed Save, Repost, and Direct Message capabilities.
- `crates/ios-driver/src/interaction_runtime.rs`: parse and negotiate the extended strict registry; bind OCR readiness only to Direct Message.
- `crates/ios-driver/src/mock.rs`: expose deterministic G4 capabilities and ordered session/frame/tap logs only in tests.
- `sidecars/wda/interaction-capabilities.schema.json`: strict per-action qualification and revocation schema.
- `sidecars/wda/interaction-capabilities.json`: add reviewed live entries only after each corresponding PASS.
- `AGENTS.md`: record OCR/model packaging, G4 intent/evidence rules, exact live results, disabled tuples, and rollback commands.

**Do Not Modify**

- `sidecars/wda/RiviuAgent.ipa`
- `sidecars/wda/agent-manifest.json`
- `sidecars/wda/riviu-agent/**`
- `crates/ios-driver/src/wda.rs` route/session logic
- `crates/core/src/nurture/actions.rs` behavior beyond regressions already shared by G2
- MDM, supervision, proxy-apply, or account-switching code

---

### Task 1: Verify The G0-G3 Baseline And Freeze G4 As Disabled

**Files:**
- Inspect: `crates/core/src/tiktok_actions/mod.rs`
- Inspect: `crates/core/src/interaction/types.rs`
- Inspect: `crates/core/src/interaction/progress.rs`
- Inspect: `crates/core/src/interaction/device_batch_executor.rs`
- Inspect: `crates/core/src/device_capabilities.rs`
- Inspect: `sidecars/wda/interaction-capabilities.json`
- Inspect: `sidecars/wda/interaction-capabilities.schema.json`

- [ ] **Step 1: Confirm the canonical prerequisite symbols**

Run:

```powershell
rg -n "pub (struct|enum|trait) (FrameProbe|ActionContext|VerifiedEffect|QualifiedGeometry|PreparedActionPayload|ActionOutcomeCode|EvidenceRef)|trait (InteractionProgress|TikTokActionExecutor|InteractionBatchExecutor)" crates/core/src
rg -n "save_y|share_y|locate_action_rail" crates/core/src/screen.rs
```

Expected: every named contract has exactly one production definition; `ActionRail` includes `save_y` and `share_y`; Interaction uses `locate_action_rail` rather than `ActionRail::fallback()`.

- [ ] **Step 2: Confirm production actions are still fail-closed**

Run:

```powershell
python -c "import json; p=json.load(open('sidecars/wda/interaction-capabilities.json', encoding='utf-8')); assert p['schemaVersion']==1; blocked={'save','repost','directMessage'}; assert all(blocked.isdisjoint(q.get('actions', {})) for q in p['qualifications']); print('G4_DEFAULT_DENY', len(p['qualifications']))"
```

Expected: the registry parses and prints `G4_DEFAULT_DENY <existing-count>` before a G4 live promotion. Existing G0-G3 qualification entries are allowed and must remain intact.

- [ ] **Step 3: Record baseline hashes and tests**

Run:

```powershell
Get-FileHash sidecars/wda/RiviuAgent.ipa -Algorithm SHA256
Get-FileHash sidecars/wda/agent-manifest.json -Algorithm SHA256
cargo test --workspace
npm --prefix apps/desktop test -- --run
```

Expected: production hashes match the preconditions; all existing G0-G3 and Nurture tests pass.

- [ ] **Step 4: Confirm the read-only checkpoint**

```powershell
git status --short -- crates/core sidecars/wda apps/desktop
```

Expected: this task introduced no diff and therefore creates no commit. Any changed
prerequisite contract is reconciled in its owning G0-G3 plan before Task 2 starts;
do not hide it in an empty G4 baseline commit.

---

### Task 2: Pin, Fetch, Verify, And Attribute The Local OCR Models

**Files:**
- Create: `sidecars/ocr/model-lock.json`
- Create: `sidecars/ocr/fetch_models.py`
- Create: `sidecars/ocr/test_fetch_models.py`
- Create: `sidecars/ocr/ATTRIBUTION.md`
- Create: `sidecars/ocr/models/.gitkeep`
- Modify: `.gitignore`

- [ ] **Step 1: Write failing lock and verifier tests**

Create `sidecars/ocr/test_fetch_models.py` with focused tests using temporary files, not network calls:

```python
import copy
import hashlib
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import fetch_models


class ModelLockTests(unittest.TestCase):
    def test_locked_model_set_is_exact(self):
        lock = fetch_models.load_lock(Path(__file__).with_name("model-lock.json"))
        self.assertEqual("ocrs", lock["engine"]["crate"])
        self.assertEqual("0.12.2", lock["engine"]["version"])
        self.assertEqual(
            "fa3c0f3aedb139813d434fe9bdd9d12ce1685cca0f26af46e006ca8ce583ef14",
            fetch_models.model_set_digest(lock),
        )

    def test_verify_rejects_wrong_bytes(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "model.rten"
            path.write_bytes(b"wrong")
            entry = {"bytes": 5, "sha256": hashlib.sha256(b"right").hexdigest()}
            with self.assertRaisesRegex(ValueError, "SHA-256"):
                fetch_models.verify_file(path, entry)

    def test_atomic_install_never_replaces_with_oversized_data(self):
        with tempfile.TemporaryDirectory() as tmp:
            final = Path(tmp) / "model.rten"
            final.write_bytes(b"trusted")
            with self.assertRaisesRegex(ValueError, "byte limit"):
                fetch_models.install_stream(
                    iter([b"x" * 9]), final,
                    {"bytes": 8, "sha256": hashlib.sha256(b"x" * 8).hexdigest()},
                )
            self.assertEqual(b"trusted", final.read_bytes())

    def test_lock_rejects_duplicate_unknown_and_unsafe_fields(self):
        for raw in [
            '{"schemaVersion":1,"schemaVersion":1}',
            '{"schemaVersion":1,"unknown":true}',
        ]:
            with self.subTest(raw=raw), self.assertRaises(ValueError):
                fetch_models.parse_lock(raw)

        lock = copy.deepcopy(fetch_models.load_lock(Path(__file__).with_name("model-lock.json")))
        lock["models"][0]["name"] = "../text-detection.rten"
        with self.assertRaisesRegex(ValueError, "model name"):
            fetch_models.validate_lock(lock)

        lock = copy.deepcopy(fetch_models.load_lock(Path(__file__).with_name("model-lock.json")))
        lock["models"][0]["url"] = "http://example.invalid/model.rten"
        with self.assertRaisesRegex(ValueError, "pinned HTTPS"):
            fetch_models.validate_lock(lock)

    def test_self_consistent_substituted_model_lock_is_rejected(self):
        lock = copy.deepcopy(fetch_models.load_lock(Path(__file__).with_name("model-lock.json")))
        lock["models"][0]["sha256"] = "0" * 64
        lock["modelSetDigest"] = fetch_models.model_set_digest(lock)
        with self.assertRaisesRegex(ValueError, "unsupported OCR engine lock"):
            fetch_models.validate_lock(lock)


if __name__ == "__main__":
    unittest.main()
```

- [ ] **Step 2: Run the tests and verify RED**

Run:

```powershell
python -m unittest discover -s sidecars/ocr -p "test_fetch_models.py" -v
```

Expected: import or file-not-found failure because the lock and verifier do not exist.

- [ ] **Step 3: Add the exact immutable model lock**

Create `sidecars/ocr/model-lock.json` exactly as follows:

```json
{
  "schemaVersion": 1,
  "engine": {
    "crate": "ocrs",
    "version": "0.12.2",
    "sourceCommit": "2dbc1f840e47d45630ef6060499138bf597a9f65",
    "runtimeCrate": "rten",
    "runtimeVersion": "0.24.0",
    "minimumRust": "1.89.0",
    "license": "MIT OR Apache-2.0"
  },
  "modelSetId": "ocrs-hiertext-2024-01-30",
  "modelSetDigest": "fa3c0f3aedb139813d434fe9bdd9d12ce1685cca0f26af46e006ca8ce583ef14",
  "sourceRevision": "df0edd170279ab971b53e094c627255a87e1a503",
  "license": "CC-BY-SA-4.0",
  "models": [
    {
      "name": "text-detection.rten",
      "url": "https://huggingface.co/robertknight/ocrs/resolve/df0edd170279ab971b53e094c627255a87e1a503/text-detection-ssfbcj81.rten",
      "bytes": 2523564,
      "sha256": "614aafabf27c94d386f7aa036c967c2e47e4b9938fa11531ca8f5698c1ca4c36"
    },
    {
      "name": "text-recognition.rten",
      "url": "https://huggingface.co/robertknight/ocrs/resolve/df0edd170279ab971b53e094c627255a87e1a503/text-rec-checkpoint-s52qdbqt.rten",
      "bytes": 9716444,
      "sha256": "606d9a0414c6b73c99df75b707c11c70d1c8b12e1d4f900922e185fc37bfca65"
    }
  ]
}
```

- [ ] **Step 4: Implement bounded atomic acquisition and offline verification**

Create `sidecars/ocr/fetch_models.py` with this complete boundary:

```python
from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import tempfile
import urllib.request
from pathlib import Path
from typing import Iterable
from urllib.parse import urlparse

CHUNK = 1024 * 1024
MAX_MODEL_BYTES = 16 * 1024 * 1024


TOP_KEYS = {
    "schemaVersion", "engine", "modelSetId", "modelSetDigest",
    "sourceRevision", "license", "models",
}
ENGINE_KEYS = {
    "crate", "version", "sourceCommit", "runtimeCrate",
    "runtimeVersion", "minimumRust", "license",
}
MODEL_KEYS = {"name", "url", "bytes", "sha256"}
MODEL_NAMES = {"text-detection.rten", "text-recognition.rten"}
ENGINE_SOURCE_COMMIT = "2dbc1f840e47d45630ef6060499138bf597a9f65"
MODEL_SET_ID = "ocrs-hiertext-2024-01-30"
MODEL_LICENSE = "CC-BY-SA-4.0"
MODEL_SOURCE_REVISION = "df0edd170279ab971b53e094c627255a87e1a503"
MODEL_SET_DIGEST = "fa3c0f3aedb139813d434fe9bdd9d12ce1685cca0f26af46e006ca8ce583ef14"
EXPECTED_MODELS = {
    "text-detection.rten": {
        "url": "https://huggingface.co/robertknight/ocrs/resolve/df0edd170279ab971b53e094c627255a87e1a503/text-detection-ssfbcj81.rten",
        "bytes": 2523564,
        "sha256": "614aafabf27c94d386f7aa036c967c2e47e4b9938fa11531ca8f5698c1ca4c36",
    },
    "text-recognition.rten": {
        "url": "https://huggingface.co/robertknight/ocrs/resolve/df0edd170279ab971b53e094c627255a87e1a503/text-rec-checkpoint-s52qdbqt.rten",
        "bytes": 9716444,
        "sha256": "606d9a0414c6b73c99df75b707c11c70d1c8b12e1d4f900922e185fc37bfca65",
    },
}


def _unique_object(pairs: list[tuple[str, object]]) -> dict:
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError(f"duplicate OCR lock key: {key}")
        result[key] = value
    return result


def parse_lock(raw: str) -> dict:
    try:
        lock = json.loads(raw, object_pairs_hook=_unique_object)
    except (json.JSONDecodeError, TypeError) as error:
        raise ValueError("invalid OCR model lock JSON") from error
    validate_lock(lock)
    return lock


def _exact_keys(value: object, expected: set[str], label: str) -> dict:
    if not isinstance(value, dict) or set(value) != expected:
        raise ValueError(f"invalid {label} keys")
    return value


def validate_lock(lock: object) -> None:
    root = _exact_keys(lock, TOP_KEYS, "OCR lock")
    engine = _exact_keys(root["engine"], ENGINE_KEYS, "OCR engine")
    if type(root["schemaVersion"]) is not int or root["schemaVersion"] != 1 \
            or engine["crate"] != "ocrs" \
            or engine["version"] != "0.12.2" \
            or engine["sourceCommit"] != ENGINE_SOURCE_COMMIT \
            or engine["runtimeCrate"] != "rten" \
            or engine["runtimeVersion"] != "0.24.0" \
            or engine["minimumRust"] != "1.89.0" \
            or engine["license"] != "MIT OR Apache-2.0" \
            or root["modelSetId"] != MODEL_SET_ID \
            or root["sourceRevision"] != MODEL_SOURCE_REVISION \
            or root["modelSetDigest"] != MODEL_SET_DIGEST \
            or root["license"] != MODEL_LICENSE:
        raise ValueError("unsupported OCR engine lock")
    if not isinstance(root["sourceRevision"], str) \
            or not isinstance(engine["sourceCommit"], str) \
            or not isinstance(root["modelSetDigest"], str) \
            or not re.fullmatch(r"[0-9a-f]{40}", root["sourceRevision"]) \
            or not re.fullmatch(r"[0-9a-f]{40}", engine["sourceCommit"]) \
            or not re.fullmatch(r"[0-9a-f]{64}", root["modelSetDigest"]):
        raise ValueError("invalid OCR source or digest")
    models = root["models"]
    if not isinstance(models, list) or len(models) != 2:
        raise ValueError("unsupported OCR model lock")
    seen = set()
    for raw_entry in models:
        entry = _exact_keys(raw_entry, MODEL_KEYS, "OCR model")
        name = entry["name"]
        if not isinstance(name, str) or not isinstance(entry["url"], str) \
                or not isinstance(entry["sha256"], str):
            raise ValueError("invalid OCR model field type")
        parsed = urlparse(entry["url"])
        if name not in MODEL_NAMES or Path(name).name != name or name in seen:
            raise ValueError("invalid OCR model name")
        if parsed.scheme != "https" or parsed.netloc != "huggingface.co" \
                or root["sourceRevision"] not in parsed.path \
                or parsed.query or parsed.fragment:
            raise ValueError("model URL is not pinned HTTPS")
        if type(entry["bytes"]) is not int or not 0 < entry["bytes"] <= MAX_MODEL_BYTES \
                or not re.fullmatch(r"[0-9a-f]{64}", entry["sha256"]):
            raise ValueError("invalid OCR model size or digest")
        if EXPECTED_MODELS.get(name) != {
            "url": entry["url"],
            "bytes": entry["bytes"],
            "sha256": entry["sha256"],
        }:
            raise ValueError("OCR lock does not match exact pinned model")
        seen.add(name)
    if seen != MODEL_NAMES:
        raise ValueError("incomplete OCR model set")
    lines = [f"{m['name']}:{m['sha256']}\n" for m in sorted(models, key=lambda x: x["name"])]
    if hashlib.sha256("".join(lines).encode("utf-8")).hexdigest() != root["modelSetDigest"]:
        raise ValueError("OCR model-set digest mismatch")


def load_lock(path: Path) -> dict:
    return parse_lock(path.read_text(encoding="utf-8"))


def model_set_digest(lock: dict) -> str:
    lines = [f"{m['name']}:{m['sha256']}\n" for m in sorted(lock["models"], key=lambda x: x["name"])]
    return hashlib.sha256("".join(lines).encode("utf-8")).hexdigest()


def verify_file(path: Path, entry: dict) -> None:
    data_len = path.stat().st_size
    if data_len != entry["bytes"]:
        raise ValueError(f"model byte length mismatch: {path.name}")
    digest = hashlib.sha256(path.read_bytes()).hexdigest()
    if digest != entry["sha256"]:
        raise ValueError(f"model SHA-256 mismatch: {path.name}")


def install_stream(chunks: Iterable[bytes], final: Path, entry: dict) -> None:
    final.parent.mkdir(parents=True, exist_ok=True)
    fd, raw_tmp = tempfile.mkstemp(prefix=f".{final.name}.", suffix=".partial", dir=final.parent)
    tmp = Path(raw_tmp)
    total = 0
    digest = hashlib.sha256()
    try:
        with os.fdopen(fd, "wb") as out:
            for chunk in chunks:
                total += len(chunk)
                if total > min(MAX_MODEL_BYTES, entry["bytes"]):
                    raise ValueError("model download exceeded byte limit")
                digest.update(chunk)
                out.write(chunk)
            out.flush()
            os.fsync(out.fileno())
        if total != entry["bytes"]:
            raise ValueError("model download byte length mismatch")
        if digest.hexdigest() != entry["sha256"]:
            raise ValueError("model download SHA-256 mismatch")
        os.replace(tmp, final)
        try:
            dir_fd = os.open(final.parent, os.O_RDONLY)
            try:
                os.fsync(dir_fd)
            finally:
                os.close(dir_fd)
        except OSError:
            # Windows does not expose directory fsync through Python.
            pass
    finally:
        tmp.unlink(missing_ok=True)


def fetch(entry: dict, model_dir: Path) -> None:
    final = model_dir / entry["name"]
    if final.exists():
        verify_file(final, entry)
        return
    request = urllib.request.Request(entry["url"], headers={"User-Agent": "Riviu-model-fetch/1"})
    with urllib.request.urlopen(request, timeout=30) as response:
        install_stream(iter(lambda: response.read(CHUNK), b""), final, entry)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", type=Path, default=Path(__file__).resolve().parent)
    parser.add_argument("--verify", action="store_true")
    args = parser.parse_args()
    lock = load_lock(args.root / "model-lock.json")
    if model_set_digest(lock) != lock["modelSetDigest"]:
        raise ValueError("OCR model-set digest mismatch")
    for entry in lock["models"]:
        path = args.root / "models" / entry["name"]
        if args.verify:
            verify_file(path, entry)
        else:
            fetch(entry, args.root / "models")
    print(f"OCR_MODEL_SET_OK {lock['modelSetId']} {lock['modelSetDigest']}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
```

- [ ] **Step 5: Add attribution and narrow ignore rules**

Create `sidecars/ocr/ATTRIBUTION.md` stating that ocrs source is MIT OR Apache-2.0 and the pinned pretrained model set is CC-BY-SA-4.0, with the exact source revision and model URLs from the lock. Add only these patterns to `.gitignore`:

```gitignore
/sidecars/ocr/models/*.rten
/target/interaction-gate4/raw/
```

Keep `sidecars/ocr/models/.gitkeep` tracked.

- [ ] **Step 6: Fetch, verify, and run tests**

Run:

```powershell
python sidecars/ocr/fetch_models.py
python sidecars/ocr/fetch_models.py --verify
python -m unittest discover -s sidecars/ocr -p "test_fetch_models.py" -v
```

Expected: both files match their exact byte lengths/SHA-256, the aggregate digest is exact, and all tests pass. A network failure leaves no trusted final file.

- [ ] **Step 7: Commit the model supply-chain contract**

```powershell
git add .gitignore sidecars/ocr/model-lock.json sidecars/ocr/fetch_models.py sidecars/ocr/test_fetch_models.py sidecars/ocr/ATTRIBUTION.md sidecars/ocr/models/.gitkeep
git diff --cached --name-only
git commit -m "build(ocr): pin local recipient models"
```

Expected staged paths: exactly the six paths above; downloaded `.rten` files are absent from the index.

---

### Task 3: Build A Provenance-Checked Real-Frame Fixture Corpus

**Files:**
- Create: `tools/interaction-gate4/fixture_manifest.py`
- Create: `tools/interaction-gate4/test_fixture_manifest.py`
- Create: `tools/interaction-gate4/requirements.txt`
- Create: `crates/core/tests/fixtures/interaction/g4/manifest.json`
- Create: the 25 G4 JPEG fixtures listed in the File Map
- Create: the eight G4 templates and generated detector set listed in the File Map
- Modify: `tools/interaction-gate0/probe.py`

Create `tools/interaction-gate4/requirements.txt` with exactly:

```text
Pillow==11.3.0
```

- [ ] **Step 1: Write failing manifest validation tests**

Create tests that reject a non-JPEG, wrong SHA-256, wrong `750x1334` pixel size, missing `375x667` portrait geometry, duplicate image hash, unrecognized state label, a fixture with a UDID/account token in metadata, or an incomplete calibration/holdout matrix:

```python
def test_manifest_requires_separate_calibration_and_holdout(self):
    manifest = valid_manifest()
    manifest["fixtures"] = [entry("save.unsaved", "calibration")]
    with self.assertRaisesRegex(ValueError, "fixture matrix"):
        fixture_manifest.validate_manifest(manifest, self.root)


def test_manifest_rejects_identifying_metadata(self):
    manifest = valid_manifest()
    manifest["fixtures"][0]["udid"] = "device-secret"
    with self.assertRaisesRegex(ValueError, "forbidden metadata"):
        fixture_manifest.validate_manifest(manifest, self.root)
```

The accepted state labels are exact: `save.unsaved`, `save.saved`, `save.ambiguous`, `repost.available`, `repost.remove`, `repost.ambiguous`, `dm.visible`, `dm.searchEmpty`, `dm.searchResult`, `dm.duplicate`, `dm.lowConsensus`, `dm.submitReady`, `dm.sent`, and `dm.sentAmbiguous`. Every `dm.visible`, `dm.submitReady`, and `dm.sent` fixture also carries the exact `submitMode`. The detector builder emits a submit contract only when `dm.visible`, `dm.submitReady`, and `dm.sent` each have separate calibration and holdout proof for that same mode. For `RecipientTapSends`, `dm.submitReady` is the positively qualified pre-tap recipient shell; for `SelectThenSend`, it is the post-selection shell with the same recipient selected and the Send affordance armed. `dm.searchEmpty` is the focused search field with its qualified empty placeholder and no entered handle; it requires one calibration and one holdout frame. If the controlled TikTok tuple exposes only one mode, the other mode remains absent/default-deny until a separate real-frame corpus and live gate are added; fake unit tests never add that capability.

- [ ] **Step 2: Run fixture tests and verify RED**

```powershell
python -m pip install -r tools/interaction-gate4/requirements.txt
python -m unittest discover -s tools/interaction-gate4 -p "test_fixture_manifest.py" -v
```

Expected: import/file failures because the manifest tool and corpus are absent.

- [ ] **Step 3: Add a Gate 0 `capture-frame` subcommand that reads MJPEG only**

Extend the proven Gate 0 probe lifecycle with a capture mode after its session and first MJPEG frame are established. The reusable function is exact and transport-agnostic:

```python
def capture_frame(reader, output: Path, expected_width: int, expected_height: int) -> dict:
    jpeg = reader.next_jpeg(timeout=5.0)
    with Image.open(io.BytesIO(jpeg)) as image:
        image.load()
        if image.format != "JPEG" or image.size != (expected_width, expected_height):
            raise ProbeError(f"unexpected fixture frame {image.format} {image.size}")
    output.parent.mkdir(parents=True, exist_ok=True)
    temporary = output.with_suffix(output.suffix + ".partial")
    temporary.write_bytes(jpeg)
    temporary.replace(output)
    return {
        "file": output.name,
        "sha256": hashlib.sha256(jpeg).hexdigest(),
        "pixelWidth": expected_width,
        "pixelHeight": expected_height,
    }
```

The command accepts `--capture-label` from the fixed label set and `--capture-output`; it derives Agent/iOS/TikTok/layout/geometry/orientation and the required pixel dimensions from the live inspected tuple, then passes those dimensions to `capture_frame`. It does not accept those values as user claims. This first G4 corpus validator still admits only the qualified `750x1334`/`375x667` portrait tuple; a later device class needs a separate reviewed corpus entry instead of changing defaults in this capture helper. It writes no token, raw UDID, account handle, or prior clipboard bytes.

- [ ] **Step 4: Implement deterministic manifest and template generation**

`fixture_manifest.py import` reads a raw capture plus the Gate 0 tuple sidecar, verifies JPEG bytes, stores a sanitized record, and copies the frame atomically. `fixture_manifest.py build-detectors` extracts the named crops, computes NCC distributions, chooses the midpoint between the lowest calibration-positive and highest calibration-negative score, and rejects any state pair with an NCC margin below `0.08`. It writes `detector-set-g4-v1.json` with exact crop boxes, thresholds, template hashes, the measured full-frame normalized `recipientBand`, `recipientPassIou=0.60`, `maxStablePointDelta=0.02`, and `detectorSetVersion="g4-detectors-v1"`. OCR boxes from the 2x pass are mapped back to original crop coordinates before IoU; the holdout evaluator rejects the set if either locked geometry value produces a false eligible tile.

Use this generated schema:

```json
{
  "schemaVersion": 1,
  "fixtureEnvironment": "LIVE_MAC_DEVICE_CAPTURE",
  "logicalWidth": 375,
  "logicalHeight": 667,
  "pixelWidth": 750,
  "pixelHeight": 1334,
  "orientation": "portrait",
  "fixtures": []
}
```

The generated holdout evaluator must require exact classification for every holdout and zero positive matches for `save.ambiguous`, `repost.ambiguous`, `dm.duplicate`, `dm.lowConsensus`, `dm.sentAmbiguous`, G2 `share-closed.jpg`, `feed-live-card.jpg`, and `feed-mid-swipe.jpg`. The `dm.searchEmpty` detector must accept its calibration/holdout pair and reject both `dm.searchResult` frames plus every duplicate/low-consensus frame; a generic focused-field rectangle is not sufficient proof that no prior query is present. The mode-specific submit-contract detector must accept both `dm.visible` frames, while `dm-submit-ready-v1` and `dm-send-confirmed-v1` each use one calibration frame and a disjoint holdout frame. One JPEG hash can occupy only one split and cannot qualify two predicates by alias.

- [ ] **Step 5: Capture and import the fixed state matrix**

Set required device inputs in environment variables so secrets and fixture values do not enter source:

```bash
export RIVIU_RTMMO_TOKEN="$(security find-generic-password -s riviu-managers-phone -a agent-auth-token -w)"
export RIVIU_WDA_BACKEND=rt-mmo
export RIVIU_RTMMO_IPA="$PWD/sidecars/wda/RiviuAgent.ipa"
export RIVIU_GATE4_UDID="$(idevice_id -l | head -n 1)"
: "${RIVIU_GATE4_SAVE_URL:?set the controlled Unsaved/Saved fixture URL}"
: "${RIVIU_GATE4_REPOST_URL:?set the controlled Repost/Remove-repost fixture URL}"
: "${RIVIU_GATE4_DM_URL:?set the controlled DM fixture URL}"
: "${RIVIU_GATE4_DM_ALLOWLIST_HANDLE:?set the exact ASCII @handle visible on the DM fixture}"
: "${RIVIU_GATE4_DM_RANDOM_HANDLES:?set comma-separated exact ASCII @handles visible on the DM fixture}"
```

The operator supplies these values from a controlled fixture account; the repository contains no production defaults. For each exact filename in the File Map, put the phone into the named state, run Gate 0 `capture-frame`, and import it with its exact state/split. Capture `save-ambiguous.jpg` only while the rail is still positively locatable but neither bookmark template wins; a missing rail is a different negative. Capture the two `dm-search-empty-*` frames only after focusing a newly opened Share search field and before any `/wda/keys` call. Capture the two `dm-submit-ready-*` frames immediately before the final side effect according to the manifest's one exact submit mode, and capture the two `dm-send-confirmed-*` frames in separate completed-send observations. The manifest records each detector crop and split. The import command stores only controlled-fixture expected normalized handles/display labels in `manifest.json`, so OCR tests read labels from provenance instead of hard-coding an account. Capture no personal feed, message, or account screen. Run `build-detectors` only after all 25 images are present.

- [ ] **Step 6: Verify the corpus and generated assets**

```powershell
python tools/interaction-gate4/fixture_manifest.py validate --manifest crates/core/tests/fixtures/interaction/g4/manifest.json
python tools/interaction-gate4/fixture_manifest.py build-detectors --manifest crates/core/tests/fixtures/interaction/g4/manifest.json --output crates/core/src/templates/interaction/detector-set-g4-v1.json
python -m unittest discover -s tools/interaction-gate4 -p "test_fixture_manifest.py" -v
```

Expected: every file/hash/tuple passes, calibration margins are at least `0.08`, every holdout is exact, and all ambiguous/negative frames are rejected.

- [ ] **Step 7: Commit the corpus and calibration tool**

```powershell
git add tools/interaction-gate0/probe.py tools/interaction-gate4/fixture_manifest.py tools/interaction-gate4/test_fixture_manifest.py tools/interaction-gate4/requirements.txt crates/core/tests/fixtures/interaction/g4 crates/core/src/templates/interaction/bookmark-unsaved-v1.png crates/core/src/templates/interaction/bookmark-saved-v1.png crates/core/src/templates/interaction/repost-v1.png crates/core/src/templates/interaction/remove-repost-v1.png crates/core/src/templates/interaction/dm-search-v1.png crates/core/src/templates/interaction/dm-submit-contract-v1.png crates/core/src/templates/interaction/dm-submit-ready-v1.png crates/core/src/templates/interaction/dm-send-confirmed-v1.png crates/core/src/templates/interaction/detector-set-g4-v1.json
git diff --cached --name-only
git commit -m "test(interaction): add G4 real-frame fixtures"
```

---

### Task 4: Define G4 Typed Outcomes, Recipient Policies, And Durable Preparation

**Files:**
- Modify: `crates/core/src/types.rs`
- Modify: `crates/core/src/interaction/types.rs`
- Modify: `crates/core/src/interaction/progress.rs`
- Modify: `crates/core/src/interaction/store.rs`
- Create: `crates/core/tests/interaction_new_action_persistence.rs`

- [ ] **Step 1: Write failing normalization and persistence tests**

```rust
#[test]
fn allowlist_normalization_is_exact_ascii_and_deduplicated() {
    assert_eq!(
        normalize_recipient_handles([" @Fixture.User ", "@fixture.user"]),
        Ok(vec!["@fixture.user".to_string()])
    );
    assert!(normalize_recipient_handles(["fixture user"]).is_err());
    assert!(normalize_recipient_handles(["@t\u{00ea}n"]).is_err());
    assert!(normalize_recipient_allowlist([
        RecipientAllowlistEntry {
            normalized_handle: "@fixture.user".into(),
            display_label: Some("Fixture One".into()),
        },
        RecipientAllowlistEntry {
            normalized_handle: "@FIXTURE.USER".into(),
            display_label: Some("Fixture Two".into()),
        },
    ]).is_err());
}

#[tokio::test]
async fn resolved_recipient_is_committed_before_effect_intent() {
    let fixture = running_dm_action(RecipientMode::RandomVisible).await;
    fixture.progress.action_prepared(
        &fixture.action_id,
        Some(&PreparedActionPayload::DirectMessage(fixture.resolved_payload())),
    ).await.unwrap();
    fixture.progress.issue_effect_intent(&fixture.action_id).await.unwrap();
    let row = fixture.store.get_action(&fixture.action_id).await.unwrap();
    assert_eq!(row.effect_intent, EffectIntent::Issued);
    assert_eq!(row.prepared_payload.unwrap(), fixture.resolved_payload_json());
}
```

Add tests proving an allowlist handle is selected once from the persisted assignment seed, random-visible starts unresolved, an identical prepared-payload replay after a lost database ACK is idempotent, a different payload cannot change after resolution or intent, `Uncertain` and issued intent are retry-ineligible, and a failed prepared-payload transaction leaves the action unchanged.

- [ ] **Step 2: Run focused tests and verify RED**

```powershell
cargo test -p riviu-core --test interaction_new_action_persistence -- --nocapture
```

Expected: unresolved recipient/payload/evidence types and missing pre-intent CAS behavior.

- [ ] **Step 3: Add the exact typed contract**

Reuse G1's canonical `RecipientPolicy`, `RecipientAllowlistEntry { normalized_handle, display_label }`, and `RecipientMode`; do not redefine or shadow them. Extend `interaction/types.rs` only with these G4 types and exact outcome codes:

```rust
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum DirectMessageSubmitMode { RecipientTapSends, SelectThenSend }

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedRecipient {
    pub normalized_handle: String,
    pub display_label: Option<String>,
    pub consensus_confidence_bps: u16,
    pub point_x: f64,
    pub point_y: f64,
    pub locator_version: String,
    pub model_set_digest: String,
    pub crop: EvidenceRef,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PreparedDirectMessagePayload {
    pub mode: RecipientMode,
    pub planned_handle: Option<String>,
    pub planned_display_label: Option<String>,
    pub resolved_recipient: Option<ResolvedRecipient>,
    pub submit_mode: DirectMessageSubmitMode,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", content = "value", rename_all = "camelCase")]
pub enum PreparedActionPayload {
    NoPayload,
    Comment(PreparedCommentPayload),
    DirectMessage(PreparedDirectMessagePayload),
}
```

Add `Copy` and `PartialEq` to the existing `crates/core/src/types.rs::TapPoint` derive so detector observations/evidence can compare logical points and an observation can retain its point after a gesture call; do not change its `x`/`y` serialization or coordinate semantics:

```rust
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TapPoint {
    pub x: f64,
    pub y: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NormalizedPoint {
    pub x: f64,
    pub y: f64,
}

impl NormalizedPoint {
    pub fn checked(x: f64, y: f64) -> Option<Self> {
        (x.is_finite() && y.is_finite() && (0.0..=1.0).contains(&x)
            && (0.0..=1.0).contains(&y)).then_some(Self { x, y })
    }

    pub fn to_logical(self, logical_width: f64, logical_height: f64) -> TapPoint {
        TapPoint { x: self.x * logical_width, y: self.y * logical_height }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NormalizedRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl NormalizedRect {
    pub fn checked(x: f64, y: f64, width: f64, height: f64) -> Option<Self> {
        let values = [x, y, width, height];
        if !values.into_iter().all(f64::is_finite)
            || x < 0.0 || y < 0.0 || width <= 0.0 || height <= 0.0
            || x + width > 1.0 || y + height > 1.0
        {
            return None;
        }
        Some(Self { x, y, width, height })
    }

    pub fn pixel_bounds(
        self,
        (image_width, image_height): (u32, u32),
    ) -> Option<(u32, u32, u32, u32)> {
        let left = (self.x * image_width as f64).floor() as u32;
        let top = (self.y * image_height as f64).floor() as u32;
        let right = ((self.x + self.width) * image_width as f64).ceil() as u32;
        let bottom = ((self.y + self.height) * image_height as f64).ceil() as u32;
        (left < right && top < bottom && right <= image_width && bottom <= image_height)
            .then_some((left, top, right - left, bottom - top))
    }
}
```

Keep `NormalizedPoint`/`NormalizedRect` in the shared `core::types` module because both the recipient locator and capability registry consume them; `device_capabilities` must not depend on the action executor module.

Extend `ActionOutcomeCode` with `BookmarkStateAmbiguous`, `SaveNotConfirmed`, `RepostStateAmbiguous`, `RepostNotConfirmed`, `OcrUnavailable`, `RecipientNotFound`, `RecipientSearchNotEmpty`, `RecipientAmbiguous`, `RecipientLowConfidence`, `RecipientChanged`, `DirectMessageNotConfirmed`, `DirectMessageSubmitContractMismatch`, and `EvidencePersistenceFailed`.

`PreparedDirectMessagePayload::validate` requires canonical planned/resolved handles, matching planned and resolved handle in allowlist mode, `None` planned fields in random-visible mode, `consensus_confidence_bps == 10000` for a resolved recipient, finite normalized points in `0.0..=1.0`, the exact non-empty locator version, a lowercase 64-character model-set digest, and an owning `EvidenceKind::RecipientMatch` reference. Reject non-finite JSON numbers before serialization and verify the crop belongs to the same assignment/action in the store transaction.

- [ ] **Step 4: Use one canonical handle normalizer and deterministic selector**

```rust
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum RecipientValidationError {
    #[error("recipient handle is not canonical ASCII")]
    Handle,
    #[error("one normalized handle has conflicting display labels")]
    ConflictingDisplayLabel,
    #[error("allowlist recipient mode requires at least one handle")]
    EmptyAllowlist,
}

pub fn normalize_recipient_handle(raw: &str) -> Result<String, RecipientValidationError> {
    let trimmed = raw.trim();
    let body = trimmed.strip_prefix('@').ok_or(RecipientValidationError::Handle)?;
    if body.is_empty() || body.len() > 64
        || !body.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'.')
    {
        return Err(RecipientValidationError::Handle);
    }
    Ok(format!("@{}", body.to_ascii_lowercase()))
}

pub fn normalize_recipient_handles<I, S>(
    values: I,
) -> Result<Vec<String>, RecipientValidationError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let normalized = values.into_iter()
        .map(|value| normalize_recipient_handle(value.as_ref()))
        .collect::<Result<BTreeSet<_>, _>>()?;
    Ok(normalized.into_iter().collect())
}

pub fn normalize_recipient_allowlist<I>(
    values: I,
) -> Result<Vec<RecipientAllowlistEntry>, RecipientValidationError>
where
    I: IntoIterator<Item = RecipientAllowlistEntry>,
{
    let mut normalized = BTreeMap::<String, Option<String>>::new();
    for entry in values {
        let handle = normalize_recipient_handle(&entry.normalized_handle)?;
        match normalized.get(&handle) {
            Some(label) if label != &entry.display_label => {
                return Err(RecipientValidationError::ConflictingDisplayLabel);
            }
            Some(_) => {}
            None => { normalized.insert(handle, entry.display_label); }
        }
    }
    Ok(normalized.into_iter().map(|(normalized_handle, display_label)| {
        RecipientAllowlistEntry { normalized_handle, display_label }
    }).collect())
}

pub fn seeded_index(seed: u64, namespace: &[u8], len: usize) -> Option<usize> {
    if len == 0 {
        return None;
    }
    let mut digest = Sha256::new();
    digest.update(seed.to_be_bytes());
    digest.update(namespace);
    let bytes: [u8; 8] = digest.finalize()[..8].try_into().expect("8-byte digest prefix");
    Some((u64::from_be_bytes(bytes) % len as u64) as usize)
}

pub fn prepare_direct_message_payload(
    policy: &RecipientPolicy,
    assignment_seed: u64,
    submit_mode: DirectMessageSubmitMode,
) -> Result<PreparedDirectMessagePayload, RecipientValidationError> {
    match &policy.mode {
        RecipientMode::Allowlist => {
            let entries = normalize_recipient_allowlist(policy.allowlist.clone())?;
            let index = seeded_index(assignment_seed, b"dm-allowlist-v1", entries.len())
                .ok_or(RecipientValidationError::EmptyAllowlist)?;
            let chosen = &entries[index];
            Ok(PreparedDirectMessagePayload {
                mode: RecipientMode::Allowlist,
                planned_handle: Some(chosen.normalized_handle.clone()),
                planned_display_label: chosen.display_label.clone(),
                resolved_recipient: None,
                submit_mode,
            })
        }
        RecipientMode::RandomVisible => Ok(PreparedDirectMessagePayload {
            mode: RecipientMode::RandomVisible,
            planned_handle: None,
            planned_display_label: None,
            resolved_recipient: None,
            submit_mode,
        }),
    }
}
```

G1 has already normalized, sorted, and deduplicated `AssignmentExecution.effective_settings.recipient_policy` and persisted `AssignmentExecution.assignment_seed`. Replace any private G1 handle-normalization body with calls to these exported helpers in this same commit, so request validation, planning, OCR comparison, and retry preparation share one implementation rather than parallel rules. The batch adapter commits this base payload once after `action_started` and before OCR/UI, while a restart reuses an existing byte-equivalent payload. Display labels are presentation-only and never participate in OCR matching; neither mode can panic on a zero-length set.

- [ ] **Step 5: Implement the pre-intent prepared-payload CAS**

`InteractionProgress::action_prepared` may create the unresolved base payload while the action is `Running`, return idempotently for the same payload, or replace it once while `effect_intent=EffectIntent::None` to add `resolved_recipient`. It rejects a first Direct Message payload that is already resolved, a changed planned handle, changed planned display label, changed mode, changed submit contract, a second different resolved recipient, any terminal state, and any issued intent. Run this through the serialized blocking writer; no SQLite call crosses `.await`. Add the following Direct Message arms to G1/G2's existing transition table after action-kind and payload validation; retain the existing `NoPayload` and Comment arms instead of replacing the whole match:

```rust
match (current.effect_intent, current.status, current.prepared_payload.as_ref(), next) {
    (EffectIntent::None, ActionStatus::Running,
     None,
     PreparedActionPayload::DirectMessage(new))
        if new.resolved_recipient.is_none() => update_payload(new),
    (EffectIntent::None, ActionStatus::Running,
     Some(PreparedActionPayload::DirectMessage(old)),
     PreparedActionPayload::DirectMessage(new))
        if old == new => Ok(PreparedMutation::Unchanged),
    (EffectIntent::None, ActionStatus::Running,
     Some(PreparedActionPayload::DirectMessage(old)),
     PreparedActionPayload::DirectMessage(new))
        if old.mode == new.mode
            && old.planned_handle == new.planned_handle
            && old.planned_display_label == new.planned_display_label
            && old.submit_mode == new.submit_mode
            && old.resolved_recipient.is_none()
            && new.resolved_recipient.is_some() => update_payload(new),
    _ => return Err(ProgressError::IllegalTransition),
}
```

- [ ] **Step 6: Verify GREEN and commit**

```powershell
cargo test -p riviu-core --test interaction_new_action_persistence -- --nocapture
cargo test -p riviu-core --test interaction_transitions -- --nocapture
git add crates/core/src/types.rs crates/core/src/interaction/types.rs crates/core/src/interaction/progress.rs crates/core/src/interaction/store.rs crates/core/tests/interaction_new_action_persistence.rs
git diff --cached --name-only
git commit -m "feat(interaction): persist G4 action preparation"
```

---

### Task 5: Implement The Save Detector And Desired-State Executor

**Files:**
- Create: `crates/core/src/tiktok_actions/save.rs`
- Modify: `crates/core/src/tiktok_actions/mod.rs`
- Create: `crates/core/tests/tiktok_new_actions.rs`
- Modify: `crates/core/tests/interaction_recovery.rs`

- [ ] **Step 1: Write failing detector and action tests**

Cover every save calibration/holdout image, already-saved, unsaved-to-saved, a newer frame remaining unsaved, ambiguous pre-state, geometry drift, stale frames, cancellation, and a tap timeout whose next persisted action attempt first re-reads Saved. Assert no tap for ambiguous/already-saved and at most one tap per action invocation. Add an integration case through G1's existing `schedule_bounded_action_retry` proving at most three total taps across the immutable attempt chain for one assignment/action ordinal; each durable retry has a distinct `ActionRunId`, and G4 must not own a nested retry loop.

```rust
#[tokio::test]
async fn save_taps_at_most_once_in_one_action_attempt() {
    let fixture = new_action_fixture([
        frame("save-unsaved-holdout-a.jpg"),
        frame("save-saved-holdout-a.jpg"),
    ]);
    let result = save(&fixture.context()).await.unwrap();
    assert!(matches!(result, VerifiedEffect::Applied(_)));
    assert_eq!(fixture.session.tap_count(), 1);
}

#[tokio::test]
async fn ambiguous_bookmark_never_taps() {
    let fixture = new_action_fixture([frame("save-ambiguous.jpg")]);
    let result = save(&fixture.context()).await.unwrap();
    assert!(matches!(result, VerifiedEffect::NotConfirmed {
        code: ActionOutcomeCode::BookmarkStateAmbiguous, ..
    }));
    assert_eq!(fixture.session.tap_count(), 0);
}
```

- [ ] **Step 2: Verify RED**

```powershell
cargo test -p riviu-core --test tiktok_new_actions save -- --nocapture
```

Expected: `tiktok_actions::save` and bookmark state types are unresolved.

- [ ] **Step 3: Add fixture-backed bookmark observations**

```rust
pub const SAVE_DETECTOR_VERSION: &str = "bookmark-state-v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BookmarkState { Unsaved, Saved, Ambiguous }

#[derive(Debug, Clone, PartialEq)]
pub struct BookmarkObservation {
    pub state: BookmarkState,
    pub unsaved_ncc: f64,
    pub saved_ncc: f64,
    pub point: TapPoint,
    pub detector_version: &'static str,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SaveEvidence {
    pub before: FrameEvidence,
    pub after: Option<FrameEvidence>,
    pub initial_state: BookmarkState,
    pub final_state: BookmarkState,
    pub effect_tap_count: u8,
    pub tap_point: Option<TapPoint>,
}

impl SaveEvidence {
    fn already(before: FrameEvidence, initial: BookmarkObservation) -> Self {
        Self {
            before,
            after: None,
            initial_state: initial.state,
            final_state: initial.state,
            effect_tap_count: 0,
            tap_point: None,
        }
    }

    fn changed(
        before: FrameEvidence,
        after: FrameEvidence,
        initial: BookmarkObservation,
        final_observation: BookmarkObservation,
        tap_point: TapPoint,
    ) -> Self {
        Self {
            before,
            after: Some(after),
            initial_state: initial.state,
            final_state: final_observation.state,
            effect_tap_count: 1,
            tap_point: Some(tap_point),
        }
    }
}

pub(crate) fn detector_evidence(frame: &ObservedFrame, detector_version: &str) -> FrameEvidence {
    let mut evidence = frame.evidence.clone();
    evidence.detector_version = detector_version.to_owned();
    evidence
}

pub fn detect_bookmark_state(
    image: &RgbImage,
    rail: &LocatedActionRail,
) -> BookmarkObservation;
```

Place `detector_evidence` once in `tiktok_actions/mod.rs` and import it from `save.rs` and `share.rs`; the snippets in later tasks refer to this one helper rather than defining module-local copies.

Load template bytes and thresholds from `detector-set-g4-v1.json`. Crop around the current `rail.rail.save_y`; require the winning state to meet its generated threshold and exceed the other state by the generated margin. Any miss/tie is `Ambiguous`. Do not use fixed iPhone points when `locate_action_rail` failed.

Add one shared, narrow gesture helper to the G2 `ActionContext`. It takes a point already derived from the current qualified frame, acquires only `context.gestures`, calls `UiSession::tap`, and maps the driver error through G2's canonical transport-error conversion. It never takes the coordinator lease, creates a session, reads a frame, or treats the gesture ACK as evidence:

```rust
impl ActionContext<'_> {
    pub async fn tap_verified_point(&self, point: TapPoint) -> Result<(), ActionExecutionError> {
        let _gesture = self.gestures.lock().await;
        self.session.tap(point).await.map_err(ActionExecutionError::Ui)
    }
}
```

- [ ] **Step 4: Implement one-tap Save semantics under G1's persisted retry owner**

```rust
pub async fn save(
    context: &ActionContext<'_>,
) -> Result<VerifiedEffect<SaveEvidence>, ActionExecutionError> {
    let original = context.probe.latest()?;
    let rail = locate_action_rail(&original.image).ok_or(ActionOutcomeCode::RailNotLocated)?;
    let initial = detect_bookmark_state(&original.image, &rail);
    if initial.state == BookmarkState::Saved {
        return Ok(VerifiedEffect::AlreadySatisfied(SaveEvidence::already(
            detector_evidence(&original, SAVE_DETECTOR_VERSION), initial,
        )));
    }
    if initial.state == BookmarkState::Ambiguous {
        return Ok(VerifiedEffect::NotConfirmed {
            code: ActionOutcomeCode::BookmarkStateAmbiguous,
            evidence: vec![detector_evidence(&original, SAVE_DETECTOR_VERSION)],
        });
    }
    let original_evidence = detector_evidence(&original, SAVE_DETECTOR_VERSION);
    let tap_point = initial.point;
    context.tap_verified_point(tap_point).await?;
    let after = match context.probe.wait_after(
        original.digest,
        Duration::from_secs(3),
        context.stop,
        |image| locate_action_rail(image).is_some_and(|current_rail| {
            detect_bookmark_state(image, &current_rail).state == BookmarkState::Saved
        }),
    ).await {
        Ok(after) => after,
        Err(ActionProbeError::Deadline) => return Ok(VerifiedEffect::NotConfirmed {
            code: ActionOutcomeCode::SaveNotConfirmed,
            evidence: vec![original_evidence],
        }),
        Err(error) => return Err(error.into()),
    };
    let Some(current_rail) = locate_action_rail(&after.image) else {
        return Ok(VerifiedEffect::NotConfirmed {
            code: ActionOutcomeCode::SaveNotConfirmed,
            evidence: vec![detector_evidence(&after, SAVE_DETECTOR_VERSION)],
        });
    };
    let observed = detect_bookmark_state(&after.image, &current_rail);
    if observed.state == BookmarkState::Saved {
        Ok(VerifiedEffect::Applied(SaveEvidence::changed(
            original_evidence,
            detector_evidence(&after, SAVE_DETECTOR_VERSION),
            initial,
            observed,
            tap_point,
        )))
    } else {
        Ok(VerifiedEffect::NotConfirmed {
            code: ActionOutcomeCode::SaveNotConfirmed,
            evidence: vec![detector_evidence(&after, SAVE_DETECTOR_VERSION)],
        })
    }
}
```

One invocation performs zero or one tap. A retry is scheduled only by G1's durable max-three-attempt policy, and each new invocation starts by re-reading current state; a prior tap that succeeded after a timeout therefore becomes `AlreadySatisfied` with no second tap. A gesture ACK is timing only, never success evidence. `detector_evidence` stores metadata only, while full-frame retention remains policy-driven in `ArtifactStore`.

- [ ] **Step 5: Verify all fixture and transition cases**

```powershell
cargo test -p riviu-core --test tiktok_new_actions save -- --nocapture
cargo test -p riviu-core --test real_frames -- --nocapture
cargo test -p riviu-core --test interaction_recovery save_retry_budget -- --nocapture
```

Expected: every holdout classifies exactly; ambiguous/negative frames produce no tap; transitions require a newer saved frame; one invocation never exceeds one tap; one persisted attempt chain never exceeds three taps across retry/restart.

- [ ] **Step 6: Commit Save**

```powershell
git add crates/core/src/tiktok_actions/mod.rs crates/core/src/tiktok_actions/save.rs crates/core/tests/tiktok_new_actions.rs crates/core/tests/interaction_recovery.rs
git diff --cached --name-only
git commit -m "feat(interaction): add frame-verified Save"
```

---

### Task 6: Implement Repost With Durable Intent And Ambiguous Completion

**Files:**
- Create: `crates/core/src/tiktok_actions/share.rs`
- Modify: `crates/core/src/tiktok_actions/mod.rs`
- Modify: `crates/core/tests/tiktok_new_actions.rs`
- Modify: `crates/core/tests/interaction_new_action_persistence.rs`

- [ ] **Step 1: Write failing Repost state and intent tests**

Cover `Repost`, `Remove repost`, ambiguous/tied detector scores, Share failing to open, intent commit failure, tap error after intent, verified Remove repost after tap, confirmation timeout, cancellation before and after intent, and process restart after intent. For every early-return/error branch, assert the public wrapper attempts idempotent Share cleanup and restores the exact prior watcher-suppression flag; an already closed Share performs no cleanup tap. Assert the final Repost tap is exactly once after commit and never retried.

```rust
#[tokio::test]
async fn repost_intent_is_visible_before_the_only_effect_tap() {
    let fixture = repost_fixture().with_post_state("repost-holdout-a.jpg");
    let result = repost(&fixture.context(), &fixture.action_id, fixture.progress.as_ref())
        .await.unwrap();
    assert!(matches!(result, VerifiedEffect::Applied(_)));
    assert_eq!(fixture.session.effect_tap_count(), 1);
    assert!(fixture.session.intent_was_issued_at_effect_tap());
}

#[tokio::test]
async fn ambiguous_result_after_intent_is_uncertain_and_not_retried() {
    let fixture = repost_fixture().with_post_state("repost-ambiguous.jpg");
    let result = repost(&fixture.context(), &fixture.action_id, fixture.progress.as_ref())
        .await.unwrap();
    assert!(matches!(result, VerifiedEffect::Uncertain {
        code: ActionOutcomeCode::RepostNotConfirmed, ..
    }));
    assert_eq!(fixture.session.effect_tap_count(), 1);
}
```

- [ ] **Step 2: Verify RED**

```powershell
cargo test -p riviu-core --test tiktok_new_actions repost -- --nocapture
cargo test -p riviu-core --test interaction_new_action_persistence repost -- --nocapture
```

- [ ] **Step 3: Add strict share-method observations and scoped suppression**

```rust
pub const REPOST_DETECTOR_VERSION: &str = "repost-state-v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RepostState { Available, RemoveAvailable, Ambiguous }

pub struct ShareMethodObservation {
    pub state: RepostState,
    pub repost_ncc: f64,
    pub remove_ncc: f64,
    pub point: TapPoint,
    pub detector_version: &'static str,
}

pub fn locate_repost_state(
    image: &RgbImage,
) -> Result<ShareMethodObservation, ActionExecutionError>;

#[derive(Debug, Clone, PartialEq)]
pub struct RepostEvidence {
    pub before: FrameEvidence,
    pub after: Option<FrameEvidence>,
    pub initial_state: RepostState,
    pub final_state: RepostState,
    pub effect_tap_count: u8,
    pub tap_point: Option<TapPoint>,
}

impl RepostEvidence {
    fn existing(before: FrameEvidence, observation: ShareMethodObservation) -> Self {
        Self {
            before,
            after: None,
            initial_state: observation.state,
            final_state: observation.state,
            effect_tap_count: 0,
            tap_point: None,
        }
    }

    fn changed(
        before: FrameEvidence,
        after: FrameEvidence,
        observation: ShareMethodObservation,
    ) -> Self {
        Self {
            before,
            after: Some(after),
            initial_state: observation.state,
            final_state: RepostState::RemoveAvailable,
            effect_tap_count: 1,
            tap_point: Some(observation.point),
        }
    }
}

pub struct UiSuppressionGuard<'a> {
    flag: &'a AtomicBool,
    previous: bool,
}

impl Drop for UiSuppressionGuard<'_> {
    fn drop(&mut self) { self.flag.store(self.previous, Ordering::Release); }
}
```

Extend G2's `ActionContext` with `pub watcher_suppressed: &'a AtomicBool`, borrowing the same per-session `Arc<AtomicBool>` already passed to `ScreenWatcher::run_suppressible`; do not allocate a second flag in the action facade. `ActionContext::suppress_watcher()` captures the prior value from that field with `swap(true, Ordering::AcqRel)` and returns this guard, so a nested scope cannot accidentally re-enable watcher taps. Classification continues through `ScreenWatcher::run_suppressible`; only watcher taps pause. `locate_repost_state` uses the current G2 Share drawer bounds and G4 templates/config. A tied or below-threshold result is `Ambiguous`.

Keep Share mechanics in this module with explicit contracts:

```rust
pub enum RepostConfirmation {
    Confirmed(ObservedFrame),
    Ambiguous(Vec<FrameEvidence>),
}

async fn open_share_from_current_rail(
    context: &ActionContext<'_>,
) -> Result<ObservedFrame, ActionExecutionError>;

async fn close_share(
    context: &ActionContext<'_>,
) -> Result<(), ActionExecutionError>;

async fn confirm_repost(
    context: &ActionContext<'_>,
    effect_frame_digest: u64,
) -> RepostConfirmation;
```

`open_share_from_current_rail` reads a current frame, validates geometry, locates the G2 rail, taps its derived Share point once, and requires a newer fixture-backed Share drawer frame. `close_share` is an idempotent `ActionContext` adapter: a current G2 `share_closed` frame returns `Ok(())` without a tap; a positively located drawer delegates to G2's exported `identity::close_share_drawer(session, probe, gestures, stop)`; an unknown state returns a typed cleanup error. It adds no coordinate, gesture, or confirmation rule and is best-effort after the primary result is known. `confirm_repost` accepts two stable newer `Remove repost` frames or the separately qualified success toast; timeout, a tied state, or transport/frame loss after intent returns `Ambiguous` evidence rather than a retryable driver error.

- [ ] **Step 4: Implement the no-repeat Repost flow**

```rust
async fn repost_inner(
    context: &ActionContext<'_>,
    action_id: &ActionRunId,
    progress: &dyn InteractionProgress,
) -> Result<VerifiedEffect<RepostEvidence>, ActionExecutionError> {
    let opened = open_share_from_current_rail(context).await?;
    let before = locate_repost_state(&opened.image)?;
    match before.state {
        RepostState::RemoveAvailable => {
            return Ok(VerifiedEffect::AlreadySatisfied(RepostEvidence::existing(
                detector_evidence(&opened, REPOST_DETECTOR_VERSION), before,
            )));
        }
        RepostState::Ambiguous => {
            let evidence = vec![detector_evidence(&opened, REPOST_DETECTOR_VERSION)];
            return Ok(VerifiedEffect::NotConfirmed {
                code: ActionOutcomeCode::RepostStateAmbiguous,
                evidence,
            });
        }
        RepostState::Available => {}
    }
    progress.issue_effect_intent(action_id).await?;
    let effect_digest = opened.digest;
    let result = if context.tap_verified_point(before.point).await.is_err() {
        VerifiedEffect::Uncertain {
            code: ActionOutcomeCode::RepostNotConfirmed,
            evidence: vec![detector_evidence(&opened, REPOST_DETECTOR_VERSION)],
        }
    } else {
        match confirm_repost(context, effect_digest).await {
            RepostConfirmation::Confirmed(after) => {
                VerifiedEffect::Applied(RepostEvidence::changed(
                    detector_evidence(&opened, REPOST_DETECTOR_VERSION),
                    detector_evidence(&after, REPOST_DETECTOR_VERSION),
                    before,
                ))
            }
            RepostConfirmation::Ambiguous(evidence) => VerifiedEffect::Uncertain {
                code: ActionOutcomeCode::RepostNotConfirmed,
                evidence,
            },
        }
    };
    Ok(result)
}

pub async fn repost(
    context: &ActionContext<'_>,
    action_id: &ActionRunId,
    progress: &dyn InteractionProgress,
) -> Result<VerifiedEffect<RepostEvidence>, ActionExecutionError> {
    let _suppressed = context.suppress_watcher();
    let primary = repost_inner(context, action_id, progress).await;
    let cleanup = close_share(context).await;
    if cleanup.is_err() {
        tracing::warn!(action_id = %action_id, code = "share_cleanup_failed");
    }
    primary
}
```

Confirmation may close and reopen Share read-only, then require stable `Remove repost`, or accept the separately qualified success toast. The public wrapper attempts idempotent Share cleanup and restores the prior watcher-suppression flag on every inner return, including open/wait/detector errors. Cleanup failure emits only the typed `share_cleanup_failed` code and never replaces a primary Applied/AlreadySatisfied/NotConfirmed/Uncertain result. The batch executor's G2 target-ready frame/rail check blocks the next side effect when cleanup did not actually restore the feed. `confirm_repost` catches every cancellation/transport/frame error after `issue_effect_intent` and returns `RepostConfirmation::Ambiguous` with bounded evidence. The executor never invokes this action again for the same action row.

- [ ] **Step 5: Verify GREEN and commit**

```powershell
cargo test -p riviu-core --test tiktok_new_actions repost -- --nocapture
cargo test -p riviu-core --test interaction_new_action_persistence repost -- --nocapture
cargo test -p riviu-core --test interaction_recovery -- --nocapture
git add crates/core/src/tiktok_actions/mod.rs crates/core/src/tiktok_actions/share.rs crates/core/tests/tiktok_new_actions.rs crates/core/tests/interaction_new_action_persistence.rs
git diff --cached --name-only
git commit -m "feat(interaction): add intent-safe Repost"
```

---

### Task 7: Add The Injectable Recipient Locator And Exact Selection Rules

**Files:**
- Create: `crates/core/src/tiktok_actions/recipient.rs`
- Modify: `crates/core/src/tiktok_actions/mod.rs`
- Modify: `crates/core/tests/tiktok_new_actions.rs`

- [ ] **Step 1: Write failing recognizer, exact-match, and random-selection tests**

Use an injected fake recognizer. Cover exact case-folded `@handle`, a matching display label with a different handle, missing `@`, Unicode, two tiles with the same handle, two-of-three consensus, crop outside the qualified recipient band, coordinate drift across two frames, allowlist not found, and deterministic random selection regardless of OCR return order.

```rust
#[tokio::test]
async fn allowlist_matches_only_the_exact_recognized_handle() {
    let locator = locator_with_passes([
        recognized("@fixture.user", tile(0)),
        recognized("Fixture User", tile(1)),
    ]);
    let result = locator.locate_allowlisted(&share_frame(), "@fixture.user").await.unwrap();
    assert_eq!(result.normalized_handle, "@fixture.user");
    assert_eq!(result.consensus_confidence_bps, 10000);
}

#[tokio::test]
async fn random_visible_is_stable_under_recognizer_order() {
    let first = locator_with_handles(["@z", "@a", "@m"])
        .locate_random(&share_frame(), 42).await.unwrap();
    let second = locator_with_handles(["@m", "@z", "@a"])
        .locate_random(&share_frame(), 42).await.unwrap();
    assert_eq!(first.normalized_handle, second.normalized_handle);
    assert_eq!(first.point, second.point);
}
```

- [ ] **Step 2: Verify RED**

```powershell
cargo test -p riviu-core --test tiktok_new_actions recipient -- --nocapture
```

- [ ] **Step 3: Define the local recognizer port and bounded output**

```rust
pub const RECIPIENT_LOCATOR_VERSION: &str = "recipient-locator-v1";
pub const RECIPIENT_PREPROCESSING_VERSION: &str = "recipient-preprocess-v1";

fn normalized_point_delta(left: NormalizedPoint, right: NormalizedPoint) -> f64 {
    (left.x - right.x).hypot(left.y - right.y)
}

#[derive(Debug, thiserror::Error)]
pub enum TextRecognitionError {
    #[error("OCR input pixels are invalid")]
    InvalidInput,
    #[error("OCR engine execution failed")]
    Engine,
    #[error("OCR returned invalid geometry")]
    InvalidGeometry,
    #[error("OCR output exceeded its fixed bound")]
    OutputBound,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum OcrAvailabilityCode {
    LockMissing,
    LockInvalid,
    ModelMissing,
    ModelPathUntrusted,
    ModelSizeMismatch,
    ModelDigestMismatch,
    EngineLoadFailed,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RecognizedText {
    pub text: String,
    pub bounds: NormalizedRect,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RecipientCandidate {
    pub normalized_handle: String,
    pub display_label: Option<String>,
    pub consensus_confidence_bps: u16,
    pub normalized_point: NormalizedPoint,
    pub point: TapPoint,
    pub crop: NormalizedRect,
    pub locator_version: &'static str,
    pub model_set_digest: String,
}

impl RecipientCandidate {
    pub fn to_persisted(&self, crop: EvidenceRef) -> ResolvedRecipient {
        ResolvedRecipient {
            normalized_handle: self.normalized_handle.clone(),
            display_label: self.display_label.clone(),
            consensus_confidence_bps: self.consensus_confidence_bps,
            point_x: self.normalized_point.x,
            point_y: self.normalized_point.y,
            locator_version: self.locator_version.to_owned(),
            model_set_digest: self.model_set_digest.clone(),
            crop,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RecognizerDescriptor {
    pub engine: String,
    pub engine_version: String,
    pub engine_source_commit: String,
    pub runtime: String,
    pub runtime_version: String,
    pub model_set_id: String,
    pub model_set_digest: String,
    pub model_source_revision: String,
}

pub trait LocalTextRecognizer: Send + Sync {
    fn descriptor(&self) -> &RecognizerDescriptor;
    fn recognize(&self, image: &RgbImage) -> Result<Vec<RecognizedText>, TextRecognitionError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecipientError {
    OcrUnavailable,
    NotFound,
    SearchNotEmpty,
    Ambiguous,
    LowConfidence,
    Changed,
    UnsupportedGeometry,
}

pub struct RecipientLocator {
    recognizer: Arc<dyn LocalTextRecognizer>,
    ocr_slots: Arc<tokio::sync::Semaphore>,
    recipient_band: NormalizedRect,
    geometry: QualifiedGeometry,
}

impl RecipientLocator {
    pub async fn locate_allowlisted(
        &self,
        frame: &ObservedFrame,
        expected: &str,
    ) -> Result<RecipientCandidate, RecipientError>;

    pub async fn locate_random(
        &self,
        frame: &ObservedFrame,
        seed: u64,
    ) -> Result<RecipientCandidate, RecipientError>;

    pub async fn relocate_same(
        &self,
        frame: &ObservedFrame,
        prior: &RecipientCandidate,
    ) -> Result<RecipientCandidate, RecipientError>;

    async fn recognize_variants(
        &self,
        crop: RgbImage,
    ) -> Result<[Vec<RecognizedText>; 3], RecipientError> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
        let permit = tokio::time::timeout_at(
            deadline,
            self.ocr_slots.clone().acquire_owned(),
        ).await.map_err(|_| RecipientError::OcrUnavailable)?
            .map_err(|_| RecipientError::OcrUnavailable)?;
        let recognizer = self.recognizer.clone();
        let mut task = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            let [original, contrasted, enlarged] = recipient_preprocessing_variants(&crop)
                .map_err(|_| RecipientError::OcrUnavailable)?;
            Ok::<_, RecipientError>([
                recognizer.recognize(&original).map_err(|_| RecipientError::OcrUnavailable)?,
                recognizer.recognize(&contrasted).map_err(|_| RecipientError::OcrUnavailable)?,
                recognizer.recognize(&enlarged).map_err(|_| RecipientError::OcrUnavailable)?,
            ])
        });
        tokio::time::timeout_at(deadline, &mut task).await
            .map_err(|_| RecipientError::OcrUnavailable)?
            .map_err(|_| RecipientError::OcrUnavailable)?
    }
}
```

The adapter returns at most 64 words, each at most 128 UTF-8 bytes. `RecipientLocator` crops only the qualified Share recipient/search region and uses `recognize_variants` for every recognition request. It maps 2x boxes back to crop coordinates, then maps every accepted box through `recipient_band` into full-frame `NormalizedRect`/`NormalizedPoint`. It stores that normalized point for persistence/drift checks and separately derives `RecipientCandidate.point` in logical 375x667 coordinates with `normalized_point.to_logical(geometry.logical_width, geometry.logical_height)` for `UiSession::tap`; Task 9 never feeds a 0..1 coordinate directly to the driver. `AppState` constructs exactly one process-wide `Arc<Semaphore>::new(1)` and clones it into every device locator; tests inject the same shared instance. If the caller deadline expires, the detached blocking closure retains the permit until it really finishes, so timed-out work cannot build an unbounded queue behind `OcrEngine`'s mutex. Failure to acquire within the same deadline returns `OcrUnavailable` before a tap. Group pass results only when normalized handles are equal and their rectangles overlap by the detector-set's calibrated IoU threshold; never merge by display label or nearest text alone.

Make preprocessing one exported pure function so fixture tests and production use identical bytes. `autocontrast_rgb` converts to luma using `image::imageops::grayscale`, stretches the observed non-empty luma range to `0..=255` with integer arithmetic (and returns the unchanged grayscale image when `max == min`), then expands back to RGB. The third pass is a 2x Lanczos3 resize of that autocontrasted image; checked multiplication rejects dimensions over `4096x4096` as `TextRecognitionError::OutputBound` before allocating:

```rust
pub fn recipient_preprocessing_variants(
    image: &RgbImage,
) -> Result<[RgbImage; 3], TextRecognitionError> {
    let original = image.clone();
    let contrasted = autocontrast_rgb(image);
    let width = image.width().checked_mul(2).filter(|value| *value <= 4096)
        .ok_or(TextRecognitionError::OutputBound)?;
    let height = image.height().checked_mul(2).filter(|value| *value <= 4096)
        .ok_or(TextRecognitionError::OutputBound)?;
    let enlarged = image::imageops::resize(
        &contrasted, width, height, image::imageops::FilterType::Lanczos3,
    );
    Ok([original, contrasted, enlarged])
}

fn autocontrast_rgb(image: &RgbImage) -> RgbImage {
    let gray = image::imageops::grayscale(image);
    let (min, max) = gray.pixels().fold((u8::MAX, u8::MIN), |(low, high), pixel| {
        (low.min(pixel[0]), high.max(pixel[0]))
    });
    RgbImage::from_fn(gray.width(), gray.height(), |x, y| {
        let source = gray.get_pixel(x, y)[0];
        let value = if max == min {
            source
        } else {
            (((source - min) as u16 * 255) / (max - min) as u16) as u8
        };
        image::Rgb([value, value, value])
    })
}
```

- [ ] **Step 4: Implement consensus, ambiguity rejection, and deterministic random mode**

```rust
fn consensus(pass_values: &[Option<String>; 3]) -> (Option<String>, u16) {
    let mut counts = BTreeMap::<String, u16>::new();
    for value in pass_values.iter().flatten() {
        *counts.entry(value.clone()).or_default() += 1;
    }
    let Some((value, count)) = counts.into_iter().max_by_key(|(_, count)| *count) else {
        return (None, 0);
    };
    (Some(value), match count { 3 => 10000, 2 => 6667, _ => 3333 })
}

fn choose_random_visible(mut candidates: Vec<RecipientCandidate>, seed: u64) -> Result<RecipientCandidate, RecipientError> {
    candidates.retain(|c| c.consensus_confidence_bps == 10000);
    candidates.sort_by(|a, b| {
        a.normalized_handle.cmp(&b.normalized_handle)
            .then_with(|| a.point.y.total_cmp(&b.point.y))
            .then_with(|| a.point.x.total_cmp(&b.point.x))
    });
    if candidates.is_empty() { return Err(RecipientError::NotFound); }
    let index = seeded_index(seed, b"dm-random-visible-v1", candidates.len())
        .ok_or(RecipientError::NotFound)?;
    Ok(candidates.remove(index))
}
```

Require the same exact handle and Euclidean delta between `normalized_point` values no greater than `0.02` in two distinct MJPEG observations: the source frame and one frame whose digest is strictly newer. Duplicate exact matches, any accepted candidate sharing a tile, low consensus, or cross-frame disagreement returns a typed no-tap result.

- [ ] **Step 5: Verify fixture negatives and GREEN**

```powershell
cargo test -p riviu-core --test tiktok_new_actions recipient -- --nocapture
```

Expected: exact-handle and deterministic-selection tests pass; duplicate/low-consensus/off-band inputs never return a tappable recipient.

- [ ] **Step 6: Commit the pure locator**

```powershell
git add crates/core/src/tiktok_actions/mod.rs crates/core/src/tiktok_actions/recipient.rs crates/core/tests/tiktok_new_actions.rs
git diff --cached --name-only
git commit -m "feat(interaction): locate exact share recipients"
```

---

### Task 8: Load And Package The OCR Engine Without Weakening Other Actions

**Files:**
- Modify: `Cargo.toml`
- Modify: `Cargo.lock`
- Modify: `apps/desktop/src-tauri/Cargo.toml`
- Modify: `apps/desktop/src-tauri/tauri.conf.json`
- Create: `apps/desktop/src-tauri/src/interaction_ocr.rs`
- Modify: `apps/desktop/src-tauri/src/lib.rs`
- Modify: `apps/desktop/src-tauri/src/state.rs`

- [ ] **Step 1: Write failing model-loader and real-crop tests**

In `interaction_ocr.rs`, add tests for missing lock, missing model, wrong length/hash, engine load failure, exact descriptor values, and OCR on both `dm-allowlist-search-result` holdouts. The two real crops must yield the exact normalized fixture handle in all three preprocessing variants.

```rust
#[test]
fn pinned_models_recognize_the_real_allowlist_holdouts() {
    let OcrRuntime::Ready(runtime) = OcrRuntime::load(fixture_ocr_root()) else {
        panic!("pinned OCR runtime must load");
    };
    for name in ["dm-allowlist-search-result-a.jpg", "dm-allowlist-search-result-b.jpg"] {
        let (image, expected_handle) = load_g4_fixture_with_expected_handle(name);
        for variant in recipient_preprocessing_variants(&image).expect("bounded variants") {
            let words = runtime.recognize(&variant).expect("OCR words");
            assert!(words.iter().any(|word| matches!(
                normalize_recipient_handle(&word.text),
                Ok(ref normalized) if normalized == &expected_handle
            )), "{name}");
        }
    }
}
```

Use these test helpers; they verify the image SHA from `crates/core/tests/fixtures/interaction/g4/manifest.json` and read that fixture's `expectedHandle` instead of inferring an account from the filename:

```rust
#[cfg(test)]
fn fixture_ocr_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../sidecars/ocr")
}

#[cfg(test)]
fn load_g4_fixture_with_expected_handle(name: &str) -> (RgbImage, String) {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../crates/core/tests/fixtures/interaction/g4");
    let manifest: serde_json::Value = serde_json::from_slice(
        &std::fs::read(root.join("manifest.json")).expect("fixture manifest"),
    ).expect("valid fixture manifest");
    let entry = manifest["fixtures"].as_array().expect("fixture array").iter()
        .find(|item| item["file"].as_str() == Some(name))
        .expect("fixture entry");
    let bytes = std::fs::read(root.join(name)).expect("fixture bytes");
    assert_eq!(sha256_hex(&bytes), entry["sha256"].as_str().expect("fixture SHA"));
    let expected = normalize_recipient_handle(
        entry["expectedHandle"].as_str().expect("expected handle"),
    ).expect("canonical fixture handle");
    let image = image::load_from_memory_with_format(&bytes, image::ImageFormat::Jpeg)
        .expect("fixture JPEG").into_rgb8();
    let detector: serde_json::Value = serde_json::from_slice(
        &std::fs::read(PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../../crates/core/src/templates/interaction/detector-set-g4-v1.json"))
            .expect("detector set"),
    ).expect("valid detector set");
    let raw = &detector["recipientBand"];
    let band = NormalizedRect::checked(
        raw["x"].as_f64().expect("band x"),
        raw["y"].as_f64().expect("band y"),
        raw["width"].as_f64().expect("band width"),
        raw["height"].as_f64().expect("band height"),
    ).expect("checked recipient band");
    let (x, y, width, height) = band.pixel_bounds(image.dimensions()).expect("band pixels");
    (image::imageops::crop_imm(&image, x, y, width, height).to_image(), expected)
}
```

`recipient_preprocessing_variants` is the exported pure helper used by `RecipientLocator`, so tests exercise the exact original/autocontrast/2x-Lanczos inputs.

- [ ] **Step 2: Pin dependencies and verify the declared MSRV failure first**

Add:

```toml
# root Cargo.toml [workspace.dependencies]
ocrs = "=0.12.2"
rten = { version = "=0.24.0", default-features = false, features = ["rten_format"] }

# apps/desktop/src-tauri/Cargo.toml [package]
rust-version = "1.89.0"

# apps/desktop/src-tauri/Cargo.toml [dependencies]
ocrs = { workspace = true }
rten = { workspace = true }
sha2 = { workspace = true }
thiserror = { workspace = true }
image = { version = "=0.25.10", default-features = false, features = ["jpeg"] }
```

`interaction_ocr.rs` names both `image::RgbImage` and `thiserror::Error`, so these are direct desktop dependencies rather than relying on `riviu-core` or `ocrs` to expose transitive crates. `image 0.25.10` is the exact patch already selected by the reviewed workspace lock.

Run:

```powershell
rustup toolchain install 1.88.0 1.89.0 --profile minimal
cargo +1.88.0 check -p riviu-managers-phone --lib
cargo +1.89.0 check -p riviu-managers-phone --lib
```

Expected: the 1.88 command is rejected by the package MSRV before compilation. The 1.89 command proceeds to the unresolved `interaction_ocr` module/tests.

- [ ] **Step 3: Implement checksum-first model loading and local recognition**

```rust
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};

use riviu_core::tiktok_actions::recipient::{
    LocalTextRecognizer, OcrAvailabilityCode, RecognizedText, RecognizerDescriptor,
    TextRecognitionError,
};
use sha2::{Digest, Sha256};

const OCR_LOCK_MAX_BYTES: u64 = 64 * 1024;
const OCR_MODEL_MAX_BYTES: u64 = 16 * 1024 * 1024;

#[derive(Debug, thiserror::Error)]
#[error("local OCR is unavailable: {code:?}")]
pub struct OcrLoadError {
    code: OcrAvailabilityCode,
}

impl OcrLoadError {
    fn new(code: OcrAvailabilityCode) -> Self { Self { code } }
    fn code(&self) -> OcrAvailabilityCode { self.code }
    fn public_message(&self) -> String { self.to_string() }
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct OcrModelLock {
    schema_version: u32,
    engine: OcrEngineLock,
    model_set_id: String,
    model_set_digest: String,
    source_revision: String,
    license: String,
    models: Vec<OcrModelEntry>,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct OcrEngineLock {
    #[serde(rename = "crate")]
    crate_name: String,
    version: String,
    source_commit: String,
    runtime_crate: String,
    runtime_version: String,
    minimum_rust: String,
    license: String,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct OcrModelEntry {
    name: String,
    url: String,
    bytes: u64,
    sha256: String,
}

pub struct OcrsTextRecognizer {
    descriptor: RecognizerDescriptor,
    engine: parking_lot::Mutex<ocrs::OcrEngine>,
}

pub enum OcrRuntime {
    Ready(Arc<OcrsTextRecognizer>),
    Unavailable { code: OcrAvailabilityCode, message: String },
}

impl OcrRuntime {
    pub fn load(root: &Path) -> Self {
        match OcrsTextRecognizer::load(root) {
            Ok(value) => Self::Ready(Arc::new(value)),
            Err(error) => Self::Unavailable {
                code: error.code(),
                message: error.public_message(),
            },
        }
    }
}

impl OcrsTextRecognizer {
    pub fn load(root: &Path) -> Result<Self, OcrLoadError> {
        let lock_bytes = read_plain_bounded_child(
            root, Path::new("model-lock.json"), OCR_LOCK_MAX_BYTES,
            OcrAvailabilityCode::LockMissing,
            OcrAvailabilityCode::LockInvalid,
        )?;
        let lock: OcrModelLock = serde_json::from_slice(&lock_bytes)
            .map_err(|_| OcrLoadError::new(OcrAvailabilityCode::LockInvalid))?;
        validate_exact_lock(&lock)?;
        let detection = read_locked_model(root, &lock, "text-detection.rten")?;
        let recognition = read_locked_model(root, &lock, "text-recognition.rten")?;
        let detection_model = rten::Model::load(detection)
            .map_err(|_| OcrLoadError::new(OcrAvailabilityCode::EngineLoadFailed))?;
        let recognition_model = rten::Model::load(recognition)
            .map_err(|_| OcrLoadError::new(OcrAvailabilityCode::EngineLoadFailed))?;
        let engine = ocrs::OcrEngine::new(ocrs::OcrEngineParams {
            detection_model: Some(detection_model),
            recognition_model: Some(recognition_model),
            ..Default::default()
        }).map_err(|_| OcrLoadError::new(OcrAvailabilityCode::EngineLoadFailed))?;
        Ok(Self {
            descriptor: RecognizerDescriptor {
                engine: "ocrs".into(),
                engine_version: "0.12.2".into(),
                engine_source_commit: lock.engine.source_commit.clone(),
                runtime: "rten".into(),
                runtime_version: "0.24.0".into(),
                model_set_id: lock.model_set_id,
                model_set_digest: lock.model_set_digest,
                model_source_revision: lock.source_revision,
            },
            engine: parking_lot::Mutex::new(engine),
        })
    }
}

impl LocalTextRecognizer for OcrsTextRecognizer {
    fn descriptor(&self) -> &RecognizerDescriptor { &self.descriptor }

    fn recognize(&self, image: &RgbImage) -> Result<Vec<RecognizedText>, TextRecognitionError> {
        let engine = self.engine.lock();
        let source = ocrs::ImageSource::from_bytes(image.as_raw(), image.dimensions())
            .map_err(|_| TextRecognitionError::InvalidInput)?;
        let input = engine.prepare_input(source)
            .map_err(|_| TextRecognitionError::Engine)?;
        let words = engine.detect_words(&input)
            .map_err(|_| TextRecognitionError::Engine)?;
        let lines = engine.find_text_lines(&input, &words);
        let text_lines = engine.recognize_text(&input, &lines)
            .map_err(|_| TextRecognitionError::Engine)?;
        map_bounded_words(text_lines, image.dimensions())
    }
}

fn map_bounded_words(
    lines: Vec<Option<ocrs::TextLine>>,
    (width, height): (u32, u32),
) -> Result<Vec<RecognizedText>, TextRecognitionError> {
    use ocrs::TextItem;

    if width == 0 || height == 0 {
        return Err(TextRecognitionError::InvalidInput);
    }
    let mut output = Vec::new();
    'lines: for line in lines.into_iter().flatten() {
        for word in line.words() {
            if output.len() == 64 { break 'lines; }
            let text = word.to_string();
            if text.trim().is_empty() || text.len() > 128 { continue; }
            let rect = word.bounding_rect();
            if rect.left() < 0 || rect.top() < 0
                || rect.right() > width as i32 || rect.bottom() > height as i32
            {
                return Err(TextRecognitionError::InvalidGeometry);
            }
            let bounds = NormalizedRect::checked(
                rect.left() as f64 / width as f64,
                rect.top() as f64 / height as f64,
                rect.width() as f64 / width as f64,
                rect.height() as f64 / height as f64,
            ).ok_or(TextRecognitionError::InvalidGeometry)?;
            output.push(RecognizedText { text, bounds });
        }
    }
    Ok(output)
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(64);
    for byte in Sha256::digest(bytes) {
        write!(&mut output, "{byte:02x}").expect("writing to String cannot fail");
    }
    output
}

fn validate_exact_lock(lock: &OcrModelLock) -> Result<(), OcrLoadError> {
    let engine = &lock.engine;
    if lock.schema_version != 1
        || engine.crate_name != "ocrs"
        || engine.version != "0.12.2"
        || engine.source_commit != "2dbc1f840e47d45630ef6060499138bf597a9f65"
        || engine.runtime_crate != "rten"
        || engine.runtime_version != "0.24.0"
        || engine.minimum_rust != "1.89.0"
        || engine.license != "MIT OR Apache-2.0"
        || lock.model_set_id != "ocrs-hiertext-2024-01-30"
        || lock.model_set_digest != "fa3c0f3aedb139813d434fe9bdd9d12ce1685cca0f26af46e006ca8ce583ef14"
        || lock.source_revision != "df0edd170279ab971b53e094c627255a87e1a503"
        || lock.license != "CC-BY-SA-4.0"
        || lock.models.len() != 2
    {
        return Err(OcrLoadError::new(OcrAvailabilityCode::LockInvalid));
    }

    let mut by_name = BTreeMap::new();
    for model in &lock.models {
        if by_name.insert(model.name.as_str(), model).is_some() {
            return Err(OcrLoadError::new(OcrAvailabilityCode::LockInvalid));
        }
    }
    let expected = [
        (
            "text-detection.rten",
            "https://huggingface.co/robertknight/ocrs/resolve/df0edd170279ab971b53e094c627255a87e1a503/text-detection-ssfbcj81.rten",
            2_523_564,
            "614aafabf27c94d386f7aa036c967c2e47e4b9938fa11531ca8f5698c1ca4c36",
        ),
        (
            "text-recognition.rten",
            "https://huggingface.co/robertknight/ocrs/resolve/df0edd170279ab971b53e094c627255a87e1a503/text-rec-checkpoint-s52qdbqt.rten",
            9_716_444,
            "606d9a0414c6b73c99df75b707c11c70d1c8b12e1d4f900922e185fc37bfca65",
        ),
    ];
    for (name, url, bytes, digest) in expected {
        let Some(actual) = by_name.get(name) else {
            return Err(OcrLoadError::new(OcrAvailabilityCode::LockInvalid));
        };
        if actual.url != url || actual.bytes != bytes || actual.sha256 != digest {
            return Err(OcrLoadError::new(OcrAvailabilityCode::LockInvalid));
        }
    }
    let aggregate = by_name.values().fold(String::new(), |mut value, model| {
        writeln!(&mut value, "{}:{}", model.name, model.sha256)
            .expect("writing to String cannot fail");
        value
    });
    if sha256_hex(aggregate.as_bytes()) != lock.model_set_digest {
        return Err(OcrLoadError::new(OcrAvailabilityCode::LockInvalid));
    }
    Ok(())
}

fn is_link_or_reparse(metadata: &fs::Metadata) -> bool {
    if metadata.file_type().is_symlink() { return true; }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0400;
        return metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0;
    }
    #[cfg(not(windows))]
    false
}

fn read_plain_bounded_child(
    root: &Path,
    relative: &Path,
    limit: u64,
    missing: OcrAvailabilityCode,
    invalid: OcrAvailabilityCode,
) -> Result<Vec<u8>, OcrLoadError> {
    if relative.is_absolute()
        || relative.components().any(|part| !matches!(part, std::path::Component::Normal(_)))
    {
        return Err(OcrLoadError::new(OcrAvailabilityCode::ModelPathUntrusted));
    }
    let canonical_root = fs::canonicalize(root).map_err(|_| OcrLoadError::new(missing))?;
    let mut current = canonical_root.clone();
    for part in relative.components() {
        let std::path::Component::Normal(part) = part else { unreachable!() };
        current.push(part);
        let metadata = fs::symlink_metadata(&current)
            .map_err(|_| OcrLoadError::new(missing))?;
        if is_link_or_reparse(&metadata) {
            return Err(OcrLoadError::new(OcrAvailabilityCode::ModelPathUntrusted));
        }
    }
    let canonical = fs::canonicalize(&current).map_err(|_| OcrLoadError::new(missing))?;
    if !canonical.starts_with(&canonical_root) {
        return Err(OcrLoadError::new(OcrAvailabilityCode::ModelPathUntrusted));
    }
    let metadata = fs::metadata(&canonical).map_err(|_| OcrLoadError::new(missing))?;
    if !metadata.is_file() || metadata.len() > limit {
        return Err(OcrLoadError::new(invalid));
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    File::open(&canonical).map_err(|_| OcrLoadError::new(missing))?
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| OcrLoadError::new(invalid))?;
    if bytes.len() as u64 != metadata.len() || bytes.len() as u64 > limit {
        return Err(OcrLoadError::new(invalid));
    }
    Ok(bytes)
}

fn read_locked_model(
    root: &Path,
    lock: &OcrModelLock,
    name: &str,
) -> Result<Vec<u8>, OcrLoadError> {
    let entry = lock.models.iter().find(|entry| entry.name == name)
        .ok_or_else(|| OcrLoadError::new(OcrAvailabilityCode::LockInvalid))?;
    let bytes = read_plain_bounded_child(
        root,
        &Path::new("models").join(name),
        OCR_MODEL_MAX_BYTES,
        OcrAvailabilityCode::ModelMissing,
        OcrAvailabilityCode::ModelSizeMismatch,
    )?;
    if bytes.len() as u64 != entry.bytes {
        return Err(OcrLoadError::new(OcrAvailabilityCode::ModelSizeMismatch));
    }
    if sha256_hex(&bytes) != entry.sha256 {
        return Err(OcrLoadError::new(OcrAvailabilityCode::ModelDigestMismatch));
    }
    Ok(bytes)
}
```

`validate_exact_lock` compares every Task 2 engine/model/source/license/version constant, requires exactly the two model names, recomputes the sorted aggregate digest, and rejects duplicate model names. `read_plain_bounded_child` canonicalizes the root and child, rejects a symlink or Windows reparse-point in every component, requires canonical containment, checks the size before reading, reads through `File::take(limit + 1)`, and rejects a short/long read. `read_locked_model` additionally requires the locked byte count and SHA-256, returning those same verified `Vec<u8>` bytes to `Model::load`; it never reopens the path. Unit-test each error code and every `map_bounded_words` bound. Never truncate inside a UTF-8 sequence or fabricate confidence from OCR output.

Canonicalize both model paths beneath the packaged OCR root, reject symlink/reparse-point components, read each bounded file once, verify its byte length and SHA-256 from `model-lock.json`, then pass those same verified `Vec<u8>` values to `rten::Model::load`. Do not verify one path read and reopen it with `Model::load_file`. Construct `OcrEngine` with `OcrEngineParams { detection_model: Some(...), recognition_model: Some(...), ..Default::default() }`. Use `parking_lot::Mutex<OcrEngine>` and never log a crop or recognized handle from the loader. Bootstrap converts any loader failure into `OcrRuntime::Unavailable`; the desktop, Save, Repost, Nurture, and non-DM campaigns continue to start.

- [ ] **Step 4: Package the exact verified resources**

Add these Tauri resource mappings:

```json
"../../../sidecars/ocr/model-lock.json": "sidecars/ocr/model-lock.json",
"../../../sidecars/ocr/ATTRIBUTION.md": "sidecars/ocr/ATTRIBUTION.md",
"../../../sidecars/ocr/models/text-detection.rten": "sidecars/ocr/models/text-detection.rten",
"../../../sidecars/ocr/models/text-recognition.rten": "sidecars/ocr/models/text-recognition.rten"
```

`AppState::bootstrap` resolves `sidecar_root.join("ocr")`, constructs `OcrRuntime`, creates one `Arc<tokio::sync::Semaphore>::new(1)` for the process, and injects both the optional `Arc<dyn LocalTextRecognizer>` and that shared semaphore into every G4 locator. Missing/untrusted models map only Direct Message to `Unavailable/OcrUnavailable`.

- [ ] **Step 5: Verify loader, MSRV, and package inputs**

```powershell
python sidecars/ocr/fetch_models.py --verify
cargo +1.89.0 test -p riviu-managers-phone interaction_ocr -- --nocapture
cargo +1.89.0 check --workspace
python -c "import json,pathlib; c=json.load(open('apps/desktop/src-tauri/tauri.conf.json')); r=c['bundle']['resources']; p=[(src,dst) for src,dst in r.items() if 'sidecars/ocr' in src]; assert len(p)==4, p; assert {dst for _,dst in p}=={'sidecars/ocr/model-lock.json','sidecars/ocr/ATTRIBUTION.md','sidecars/ocr/models/text-detection.rten','sidecars/ocr/models/text-recognition.rten'}; [pathlib.Path('apps/desktop/src-tauri').joinpath(src).resolve(strict=True) for src,_ in p]; print('OCR_RESOURCES_OK')"
```

Expected: both real crops are recognized exactly, the pinned toolchain builds, all four OCR resources exist, and model verification passes.

- [ ] **Step 6: Commit OCR runtime and packaging**

```powershell
git add Cargo.toml Cargo.lock apps/desktop/src-tauri/Cargo.toml apps/desktop/src-tauri/tauri.conf.json apps/desktop/src-tauri/src/interaction_ocr.rs apps/desktop/src-tauri/src/lib.rs apps/desktop/src-tauri/src/state.rs
git diff --cached --name-only
git commit -m "feat(desktop): package pinned local OCR"
```

---

### Task 9: Implement Allowlisted And Random-Visible Direct Message

**Files:**
- Modify: `crates/core/src/tiktok_actions/mod.rs`
- Modify: `crates/core/src/tiktok_actions/share.rs`
- Modify: `crates/core/src/tiktok_actions/recipient.rs`
- Modify: `crates/core/src/interaction/artifacts.rs`
- Modify: `crates/core/src/interaction/store.rs`
- Modify: `crates/core/tests/tiktok_new_actions.rs`
- Modify: `crates/core/tests/interaction_new_action_persistence.rs`
- Modify: `crates/core/tests/interaction_artifacts.rs`

- [ ] **Step 1: Write failing Direct Message flow tests**

Cover visible allowlist exact match, offscreen allowlist search using a trusted fresh text session, focused empty-search proof before typing, a pre-filled or ambiguous search field that performs zero `/wda/keys` calls, missing text support, search ACK without exact result, random-visible with at least two candidates, deterministic selection, recipient disappearing before tap, a still-Running random-visible action re-entered before intent whose persisted handle is absent while another handle is visible, `RecipientTapSends`, `SelectThenSend`, submit-contract mismatch before selection, prepared-payload CAS failure with recipient-crop cleanup, a crash after crop `put` but before the payload CAS, intent commit failure, post-send toast success, ambiguous post-send state, cancellation before/after intent, and crash recovery. For every early-return/error branch, assert the public wrapper attempts idempotent Share cleanup and restores the exact prior watcher-suppression flag. Assert startup freezes the non-issued action first and then removes its unreferenced crop, missing/ambiguous/low-consensus OCR causes zero recipient/Send effect taps, re-entry never samples a replacement, and no second effect tap occurs after intent. An allowlist search may already have performed its separately audited Share/search-focus navigation tap before a later OCR rejection.

```rust
#[tokio::test]
async fn random_recipient_is_persisted_before_the_send_tap() {
    let fixture = dm_fixture(
        RecipientMode::RandomVisible,
        DirectMessageSubmitMode::RecipientTapSends,
    );
    let result = direct_message(
        &fixture.context(), &fixture.action_id, fixture.progress.as_ref(),
        fixture.locator.clone(), fixture.artifacts.as_ref(), fixture.prepared_payload(),
    ).await.unwrap();
    assert!(matches!(result, VerifiedEffect::Applied(_)));
    assert!(fixture.session.recipient_was_prepared_at_effect_tap());
    assert!(fixture.session.intent_was_issued_at_effect_tap());
    assert_eq!(fixture.session.effect_tap_count(), 1);
}

#[tokio::test]
async fn low_consensus_never_effect_taps_or_issues_intent() {
    let fixture = dm_fixture_with_frame("dm-low-consensus.jpg");
    let result = fixture.run().await.unwrap();
    assert!(matches!(result, VerifiedEffect::NotConfirmed {
        code: ActionOutcomeCode::RecipientLowConfidence, ..
    }));
    assert_eq!(fixture.session.effect_tap_count(), 0);
    assert_eq!(fixture.progress.intent_count(), 0);
}

#[tokio::test]
async fn prefilled_search_field_never_types_or_selects() {
    let fixture = dm_allowlist_fixture()
        .with_post_focus_frame("dm-allowlist-search-result-a.jpg");
    let result = fixture.run().await.unwrap();
    assert!(matches!(result, VerifiedEffect::NotConfirmed {
        code: ActionOutcomeCode::RecipientSearchNotEmpty, ..
    }));
    assert_eq!(fixture.session.type_text_count(), 0);
    assert_eq!(fixture.session.effect_tap_count(), 0);
    assert_eq!(fixture.progress.intent_count(), 0);
}
```

- [ ] **Step 2: Verify RED**

```powershell
cargo test -p riviu-core --test tiktok_new_actions direct_message -- --nocapture
cargo test -p riviu-core --test interaction_new_action_persistence direct_message -- --nocapture
```

- [ ] **Step 3: Implement allowlist search and random-visible resolution**

Allowlist uses the already persisted `planned_handle`. First locate it among visible tiles. If absent, require `session.supports_text_input()`, positively locate `dm-search-v1`, focus it, prove the focused field is empty from a newer frame, type the exact persisted handle once, and wait for a still newer result frame. Never match the display label. Random-visible never opens search; it uses only currently visible unanimous candidates.

Implement this canonical entry point; the snippets below are consecutive blocks inside it:

```rust
async fn direct_message_inner(
    context: &ActionContext<'_>,
    action_id: &ActionRunId,
    progress: &dyn InteractionProgress,
    locator: Arc<RecipientLocator>,
    artifacts: &ArtifactStore,
    mut prepared: PreparedDirectMessagePayload,
) -> Result<VerifiedEffect<DirectMessageEvidence>, ActionExecutionError>;
```

Carry lifecycle proof into the existing action context instead of inferring it from `UiSession::supports_text_input()`:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextChannelTrust { Ordinary, Fresh }

// Add to G2 ActionContext:
pub text_channel_trust: TextChannelTrust,

#[derive(Debug, Clone, PartialEq)]
pub struct DirectMessageEvidence {
    pub before: FrameEvidence,
    pub after: Option<FrameEvidence>,
    pub recipient: ResolvedRecipient,
    pub submit_mode: DirectMessageSubmitMode,
    pub effect_tap_count: u8,
}

pub struct LocatedRecipient {
    pub frame: ObservedFrame,
    pub candidate: RecipientCandidate,
}

async fn search_exact_handle(
    context: &ActionContext<'_>,
    locator: &RecipientLocator,
    expected: &str,
) -> Result<LocatedRecipient, RecipientError>;

fn recipient_outcome_code(error: RecipientError) -> ActionOutcomeCode {
    match error {
        RecipientError::OcrUnavailable => ActionOutcomeCode::OcrUnavailable,
        RecipientError::NotFound => ActionOutcomeCode::RecipientNotFound,
        RecipientError::SearchNotEmpty => ActionOutcomeCode::RecipientSearchNotEmpty,
        RecipientError::Ambiguous => ActionOutcomeCode::RecipientAmbiguous,
        RecipientError::LowConfidence => ActionOutcomeCode::RecipientLowConfidence,
        RecipientError::Changed => ActionOutcomeCode::RecipientChanged,
        RecipientError::UnsupportedGeometry => ActionOutcomeCode::UnsupportedGeometry,
    }
}

fn recipient_not_confirmed<T>(
    error: RecipientError,
    frame: &ObservedFrame,
) -> VerifiedEffect<T> {
    VerifiedEffect::NotConfirmed {
        code: recipient_outcome_code(error),
        evidence: vec![detector_evidence(frame, RECIPIENT_LOCATOR_VERSION)],
    }
}
```

`search_exact_handle` first requires `text_channel_trust == Fresh` and `session.supports_text_input()`. It positively locates the search field from the current frame and passes that observation's derived logical point to `context.tap_verified_point`, which preserves the profile-selected `UiSession::tap` route under the shared gesture mutex. In particular, RT-MMO continues to use its sessionless `/wda/swipe` tap and this flow never switches to `/wda/tap` or W3C actions. It then waits for a newer frame that the qualified detector classifies exactly as `dm.searchEmpty`. Only that proven-empty state permits one `type_text(expected)` call. A pre-filled, stale, or ambiguous field returns `RecipientError::SearchNotEmpty` with zero `/wda/keys` calls; this plan does not invent a clear-text route that G2 never qualified. After typing, require another newer exact-result frame and return that result frame inside `LocatedRecipient`. Gesture or `/wda/keys` ACK alone does not produce a candidate.

```rust
let frame = open_share_from_current_rail(context).await?;
let existing = prepared.resolved_recipient.clone();
let located = if let Some(persisted) = existing.as_ref() {
    let visible = locator.locate_allowlisted(&frame, &persisted.normalized_handle).await;
    let found = match (prepared.mode.clone(), visible) {
        (_, Ok(candidate)) => LocatedRecipient { frame, candidate },
        (RecipientMode::Allowlist, Err(RecipientError::NotFound)) => {
            match search_exact_handle(context, locator.as_ref(), &persisted.normalized_handle).await {
                Ok(found) => found,
                Err(error) => return Ok(recipient_not_confirmed(error, &frame)),
            }
        }
        (_, Err(error)) => return Ok(recipient_not_confirmed(error, &frame)),
    };
    let old_point = NormalizedPoint::checked(persisted.point_x, persisted.point_y)
        .ok_or(ActionOutcomeCode::RecipientChanged)?;
    if normalized_point_delta(old_point, found.candidate.normalized_point) > 0.02 {
        return Ok(recipient_not_confirmed(RecipientError::Changed, &found.frame));
    }
    found
} else {
    match prepared.mode.clone() {
        RecipientMode::Allowlist => {
            let Some(expected) = prepared.planned_handle.as_deref() else {
                return Ok(recipient_not_confirmed(RecipientError::NotFound, &frame));
            };
            match locator.locate_allowlisted(&frame, expected).await {
                Ok(candidate) => LocatedRecipient { frame, candidate },
                Err(RecipientError::NotFound) => {
                    match search_exact_handle(context, locator.as_ref(), expected).await {
                        Ok(found) => found,
                        Err(error) => return Ok(recipient_not_confirmed(error, &frame)),
                    }
                }
                Err(error) => return Ok(recipient_not_confirmed(error, &frame)),
            }
        }
        RecipientMode::RandomVisible => match locator.locate_random(
            &frame, context.assignment_seed,
        ).await {
            Ok(candidate) => LocatedRecipient { frame, candidate },
            Err(error) => return Ok(recipient_not_confirmed(error, &frame)),
        },
    }
};
let source_frame = located.frame;
let resolved = located.candidate;
```

The `existing` branch is mandatory whenever the still-Running action re-enters recipient location before intent (for example after a recoverable pre-send UI/context refresh): it never calls `locate_random`, never changes the persisted handle/crop, and random-visible never falls through to search. It does not broaden G1's durable retry allowlist: a process restart freezes a non-issued Direct Message attempt as `Interrupted`, and no new DM attempt row is scheduled automatically. A model/locator digest mismatch is rejected by capability preflight before this function. The allowlist branch may search only for the already persisted exact handle.

The batch preflight conservatively requests a fresh text session whenever any sampled allowlisted Direct Message exists. It preserves `foreground TikTok -> fresh session -> MJPEG -> first frame`; no search attempts an ordinary session.

- [ ] **Step 4: Commit recipient evidence before the final side effect**

Encode only `resolved.crop` from its exact `source_frame` as deterministic JPEG quality 90, write it through `ArtifactStore::put(ArtifactOwner::Action(action_id.clone()), ArtifactKind::FrameEvidence, ..., "image/jpeg")`, and convert returned metadata to `EvidenceRef { kind: RecipientMatch }`. `search_exact_handle` must return the newer search-result frame together with its candidate; never crop the initial Share frame with coordinates learned from a later frame. Build `ResolvedRecipient` and call `progress.action_prepared` before a final tap. Re-locate the same exact handle in a newer frame and require normalized point drift at most `0.02`; never choose a replacement recipient within the same assignment. If the prepared-payload CAS fails, call a new idempotent `ArtifactStore::discard_if_unreferenced(artifact_id)` and verify both file and metadata cleanup; a cleanup error is attached without replacing the primary transition error.

Extend G2's `ActionExecutionError` rather than collapsing artifact failures into `Ui`:

```rust
#[derive(Debug, thiserror::Error)]
pub enum ActionExecutionError {
    #[error(transparent)]
    Probe(#[from] ActionProbeError),
    #[error("UI operation failed")]
    Ui(#[source] anyhow::Error),
    #[error("progress transition failed")]
    Progress(#[from] ProgressError),
    #[error("invalid or unconfirmed action state: {0:?}")]
    Outcome(ActionOutcomeCode),
    #[error("evidence crop encoding failed")]
    EvidenceEncoding,
    #[error("artifact operation failed")]
    Artifact(#[from] ArtifactError),
    #[error("prepared-payload transition failed; artifact cleanup: {cleanup:?}")]
    PreparedArtifact {
        #[source]
        primary: ProgressError,
        cleanup: Option<ArtifactError>,
    },
}

impl ActionExecutionError {
    fn prepared_artifact(primary: ProgressError, cleanup: Option<ArtifactError>) -> Self {
        Self::PreparedArtifact { primary, cleanup }
    }
}

fn encode_normalized_crop_jpeg(
    image: &RgbImage,
    bounds: NormalizedRect,
    quality: u8,
) -> Result<Vec<u8>, ActionExecutionError> {
    if !(1..=100).contains(&quality) {
        return Err(ActionExecutionError::EvidenceEncoding);
    }
    let (x, y, width, height) = bounds.pixel_bounds(image.dimensions())
        .ok_or(ActionExecutionError::EvidenceEncoding)?;
    let crop = image::imageops::crop_imm(image, x, y, width, height).to_image();
    let mut bytes = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes, quality)
        .encode_image(&image::DynamicImage::ImageRgb8(crop))
        .map_err(|_| ActionExecutionError::EvidenceEncoding)?;
    Ok(bytes)
}
```

Add `ArtifactStorageState::Discarding` and this exact API to G1's existing store. The first serialized writer transaction loads the owner, parses all prepared payload/evidence JSON for that action, returns `StillReferenced` when any reference exists, otherwise CASes `stored -> discarding`; this makes later reference validation reject the artifact. Delete the canonical file outside SQLite, then a second writer transaction removes the still-discarding metadata row. `AlreadyAbsent` is success. Startup recovery first applies G1's action freeze, then finishes `discarding` rows and scans `stored` action-owned artifacts. A stored artifact whose owning action is terminal and whose ID appears in neither prepared payload nor outcome evidence is moved through the same `discarding -> delete -> remove row` sequence. It never deletes a referenced artifact or one owned by a still-Running action. This explicitly covers a crash after recipient-crop `put` but before `action_prepared`, while a lost ACK after a committed CAS yields `StillReferenced` and retains the crop. No SQLite transaction spans file I/O:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiscardArtifactResult { Removed, AlreadyAbsent, StillReferenced }

impl ArtifactStore {
    pub async fn discard_if_unreferenced(
        &self,
        artifact_id: &str,
    ) -> Result<DiscardArtifactResult, ArtifactError>;
}
```

```rust
if existing.is_none() {
    let crop_jpeg = encode_normalized_crop_jpeg(&source_frame.image, resolved.crop, 90)?;
    let saved_crop = artifacts.put(
        ArtifactOwner::Action(action_id.clone()),
        ArtifactKind::FrameEvidence,
        &crop_jpeg,
        "image/jpeg",
    ).await?;
    let crop_ref = EvidenceRef {
        artifact_id: saved_crop.id.clone(),
        kind: EvidenceKind::RecipientMatch,
        sha256: saved_crop.sha256.clone(),
        observed_at: source_frame.evidence.observed_at,
    };
    prepared.resolved_recipient = Some(resolved.to_persisted(crop_ref));
    if let Err(primary) = progress.action_prepared(
        action_id,
        Some(&PreparedActionPayload::DirectMessage(prepared.clone())),
    ).await {
        let cleanup = artifacts.discard_if_unreferenced(&saved_crop.id).await;
        return Err(ActionExecutionError::prepared_artifact(primary, cleanup.err()));
    }
}
let confirmation_frame = context.probe.wait_after(
    source_frame.digest,
    Duration::from_secs(3),
    context.stop,
    |_| true,
).await?;
let confirmed = match locator.relocate_same(&confirmation_frame, &resolved).await {
    Ok(value) => value,
    Err(error) => return Ok(recipient_not_confirmed(error, &confirmation_frame)),
};
```

- [ ] **Step 5: Implement both typed submit contracts and gate each one independently**

```rust
pub enum DirectMessageConfirmation {
    Confirmed(ObservedFrame),
    Ambiguous(Vec<FrameEvidence>),
}

pub enum SelectedRecipientSend {
    Ready { frame: ObservedFrame, point: TapPoint },
    NotConfirmed { code: ActionOutcomeCode, frame: ObservedFrame },
}

async fn wait_for_selected_recipient_send(
    context: &ActionContext<'_>,
    locator: &RecipientLocator,
    previous_digest: u64,
    expected: &ResolvedRecipient,
) -> Result<SelectedRecipientSend, ActionExecutionError>;

fn verify_submit_contract(
    image: &RgbImage,
    mode: DirectMessageSubmitMode,
) -> Result<(), ActionOutcomeCode>;

async fn confirm_direct_message(
    context: &ActionContext<'_>,
    effect_frame_digest: u64,
    expected: &ResolvedRecipient,
) -> DirectMessageConfirmation;

// `confirmed.point` was derived from this exact frame in Step 4. Do not fetch a
// third frame and combine its submit-shell proof with a stale recipient point.
let effect_baseline = confirmation_frame;
if let Err(code) = verify_submit_contract(&effect_baseline.image, prepared.submit_mode) {
    return Ok(VerifiedEffect::NotConfirmed {
        code,
        evidence: vec![detector_evidence(
            &effect_baseline, DM_SUBMIT_CONTRACT_DETECTOR_VERSION,
        )],
    });
}
let mut effect_frame_digest = effect_baseline.digest;
let mut effect_frame_evidence = detector_evidence(
    &effect_baseline, DM_SUBMIT_CONTRACT_DETECTOR_VERSION,
);
let tap_result = match prepared.submit_mode {
    DirectMessageSubmitMode::RecipientTapSends => {
        effect_frame_evidence = detector_evidence(
            &effect_baseline, DM_SUBMIT_READY_DETECTOR_VERSION,
        );
        progress.issue_effect_intent(action_id).await?;
        context.tap_verified_point(confirmed.point).await
    }
    DirectMessageSubmitMode::SelectThenSend => {
        context.tap_verified_point(confirmed.point).await?;
        let selected = wait_for_selected_recipient_send(
            context,
            locator.as_ref(),
            effect_baseline.digest,
            prepared.resolved_recipient.as_ref().expect("recipient persisted"),
        ).await?;
        let (selected_frame, send) = match selected {
            SelectedRecipientSend::Ready { frame, point } => (frame, point),
            SelectedRecipientSend::NotConfirmed { code, frame } => {
                return Ok(VerifiedEffect::NotConfirmed {
                    code,
                    evidence: vec![detector_evidence(
                        &frame, DM_SUBMIT_READY_DETECTOR_VERSION,
                    )],
                })
            }
        };
        effect_frame_digest = selected_frame.digest;
        effect_frame_evidence = detector_evidence(
            &selected_frame, DM_SUBMIT_READY_DETECTOR_VERSION,
        );
        progress.issue_effect_intent(action_id).await?;
        context.tap_verified_point(send).await
    }
};

let resolved = prepared.resolved_recipient.clone().expect("recipient persisted");
if tap_result.is_err() {
    return Ok(VerifiedEffect::Uncertain {
        code: ActionOutcomeCode::DirectMessageNotConfirmed,
        evidence: vec![effect_frame_evidence],
    });
}

let result = match confirm_direct_message(context, effect_frame_digest, &resolved).await {
    DirectMessageConfirmation::Confirmed(after) => {
        Ok(VerifiedEffect::Applied(DirectMessageEvidence {
            before: detector_evidence(&effect_baseline, RECIPIENT_LOCATOR_VERSION),
            after: Some(detector_evidence(&after, DM_CONFIRM_DETECTOR_VERSION)),
            recipient: resolved,
            submit_mode: prepared.submit_mode,
            effect_tap_count: 1,
        }))
    }
    DirectMessageConfirmation::Ambiguous(evidence) => Ok(VerifiedEffect::Uncertain {
        code: ActionOutcomeCode::DirectMessageNotConfirmed,
        evidence,
    }),
};
result
```

The block above is the final expression of `direct_message_inner`. Wrap all of its
returns at the public boundary:

```rust
pub async fn direct_message(
    context: &ActionContext<'_>,
    action_id: &ActionRunId,
    progress: &dyn InteractionProgress,
    locator: Arc<RecipientLocator>,
    artifacts: &ArtifactStore,
    prepared: PreparedDirectMessagePayload,
) -> Result<VerifiedEffect<DirectMessageEvidence>, ActionExecutionError> {
    let _suppressed = context.suppress_watcher();
    let primary = direct_message_inner(
        context, action_id, progress, locator, artifacts, prepared,
    ).await;
    let cleanup = close_share(context).await;
    if cleanup.is_err() {
        tracing::warn!(action_id = %action_id, code = "share_cleanup_failed");
    }
    primary
}
```

Define `DM_SUBMIT_CONTRACT_DETECTOR_VERSION = "dm-submit-contract-v1"`, `DM_SUBMIT_READY_DETECTOR_VERSION = "dm-submit-ready-v1"`, and `DM_CONFIRM_DETECTOR_VERSION = "dm-send-confirm-v1"`. Before either recipient tap, `verify_submit_contract` must positively match the mode-specific `dm-submit-contract-v1.png` and distinguish the qualified `RecipientTapSends` layout from the qualified `SelectThenSend` shell; missing/tied/opposite state returns `DirectMessageSubmitContractMismatch` with zero tap and no intent. For `RecipientTapSends`, that same frame must also satisfy the mode's `dm.submitReady` contract before intent. For `SelectThenSend`, `wait_for_selected_recipient_send` advances across strictly newer digests until the fixed three-second deadline. On each returned frame it invokes the same async `RecipientLocator` for the persisted exact handle, requires normalized-point drift at most `0.02`, then requires `dm-submit-ready-v1.png` and derives Send from that frame. `OcrUnavailable`, duplicate/low-consensus, changed recipient, unsupported geometry, or a conflicting submit shell returns typed `NotConfirmed` immediately; `NotFound` may wait for the next digest until the deadline. Cancellation or frame/transport loss before intent remains `ActionExecutionError`. This prevents a drifted tap-to-send layout from sending during the nominal selection step and gives the second mode a real detector instead of an inferred button or synchronous non-OCR check. `confirm_direct_message` requires two newer stable frames with the separately calibrated and held-out sent toast/state and carries the already persisted recipient crop/hash in `DirectMessageEvidence`; all timeout/cancellation/transport/frame errors after intent become `Ambiguous`. The public wrapper owns watcher suppression and attempts idempotent Share cleanup on every inner return; cleanup failure logs only the typed code, does not change the primary outcome, and the next action remains blocked by G2's current feed/rail readiness check.

- [ ] **Step 6: Verify GREEN, retry exclusion, and evidence bounds**

```powershell
cargo test -p riviu-core --test tiktok_new_actions direct_message -- --nocapture
cargo test -p riviu-core --test interaction_new_action_persistence direct_message -- --nocapture
cargo test -p riviu-core --test interaction_recovery -- --nocapture
cargo test -p riviu-core --test interaction_artifacts -- --nocapture
```

Expected: every pre-intent OCR failure produces zero effect taps; both submit contracts require evidence; issued/uncertain sends are never retryable; crop paths remain managed/relative.

- [ ] **Step 7: Commit Direct Message**

```powershell
git add crates/core/src/tiktok_actions/mod.rs crates/core/src/tiktok_actions/share.rs crates/core/src/tiktok_actions/recipient.rs crates/core/src/interaction/artifacts.rs crates/core/src/interaction/store.rs crates/core/tests/tiktok_new_actions.rs crates/core/tests/interaction_new_action_persistence.rs crates/core/tests/interaction_artifacts.rs
git diff --cached --name-only
git commit -m "feat(interaction): add qualified direct sharing"
```

---

### Task 10: Wire G4 Actions Into The Batch Executor Without Cross-Action Leakage

**Files:**
- Modify: `crates/core/src/interaction/device_batch_executor.rs`
- Modify: `crates/core/src/interaction/types.rs`
- Modify: `crates/core/src/tiktok_actions/frame_probe.rs`
- Modify: `crates/core/tests/interaction_device_batch.rs`
- Modify: `crates/core/tests/interaction_artifacts.rs`

- [ ] **Step 1: Write failing order, identity, and partial-result tests**

Test the exact order `Identity -> Watch -> Like -> Follow -> Comment -> Save -> Repost -> DirectMessage`; no G4 action before confirmed identity; Share closes between Repost and Direct Message; a failed Required G4 action makes the assignment partial/failed according to the G1 reducer; an Off/probability-false G4 action remains `NotPlanned`; and a capability-disabled actor is `SkippedUnsupported` without a device tap.

Also prove evidence materialization is real rather than metadata-only: successful Save/Repost store only named detector crops plus timing, failed/not-confirmed/uncertain outcomes may store at most one full JPEG per action, every `EvidenceRef.sha256` matches the managed file, and serialized events contain no JPEG bytes. Feed more than twelve rejected predicate frames and prove they are not retained; attempt to pin a thirteenth returned frame or exceed 16 MiB and require a typed bound error before any further tap. Current-action pinned frames remain materializable until `action_finished`, and a missing digest fails the evidence commit instead of fabricating a reference or polling WDA.

```rust
#[tokio::test]
async fn new_actions_run_only_after_identity_and_save_precedes_share_methods() {
    let fixture = batch_fixture().with_all_actions();
    fixture.execute().await.unwrap();
    assert_eq!(fixture.call_log(), [
        "identity", "watch", "like", "follow", "comment", "save",
        "open-share", "repost", "close-share", "open-share", "direct-message", "close-share",
    ]);
}
```

- [ ] **Step 2: Verify RED**

```powershell
cargo test -p riviu-core --test interaction_device_batch g4 -- --nocapture
```

- [ ] **Step 3: Dispatch through the shared facade and progress port**

Extend the existing `FrameVerifiedTikTokActionExecutor::execute_action` match inside the canonical `CoordinatedInteractionBatchExecutor`; do not add another dispatcher or revive the superseded `QualifiedInteractionBatchExecutor` name. Its `execute_device_batch` result remains G1's `BatchExecutionReport`. All progress calls use `&ActionRunId`/`&AssignmentId`, never `&str`. Add exactly these arms:

- `Save`: require G1/G2's no-payload representation, call Task 5 `save`, and map `Applied`, `AlreadySatisfied`, and `NotConfirmed` exhaustively.
- `Repost`: require no payload, pass `action.action_run_id` and the existing `InteractionProgress` to Task 6, and preserve `Uncertain` exactly.
- `DirectMessage`: after `action_started`, if `action.payload` is `None`, call Task 4 `prepare_direct_message_payload` with the assignment's immutable effective recipient policy and `assignment_seed`, plus the currently negotiated submit mode, then commit it through `action_prepared` before OCR or Share UI. Reuse `Some(PreparedActionPayload::DirectMessage(...))` byte-for-byte on restart; reject `Some(NoPayload)`, Comment, or a payload whose submit mode differs from the exact capability. When a resolved recipient is already present, also require its locator version and model-set digest to equal the negotiated Direct Message capability before opening Share. Pass the cloned persisted payload plus the same G1 `ArtifactStore` to Task 9.

Keep the existing G2 conversion/persistence block as the only place that converts `VerifiedEffect` to `ActionOutcome`, stores bounded artifacts through `ArtifactStore`, produces `EvidenceRef`, and calls `progress.action_finished(&action.action_run_id, ...)`. Extend its exhaustive typed mapping for `SaveEvidence`, `RepostEvidence`, and `DirectMessageEvidence`; do not create `into_action_outcome`, `evidence_refs`, or free-form fallback helpers. Before every action, check persisted cancellation and target-ready identity. A transport error remains an executor error before intent and becomes the action-specific `Uncertain` result after Repost/DM intent as defined above.

Extend `FrameProbe` with a `Mutex<VecDeque<CachedEvidenceFrame>>` keyed by `FrameEvidence.digest`, capped at both twelve frames and 16 MiB. Twelve covers the longest qualified allowlist `SelectThenSend` path, including two sent-confirmation frames and the final Share-close proof; eight would deterministically reject that ninth distinct frame before cleanup. `begin_action_evidence(action_run_id)` starts one cache scope; only frames actually returned by `latest`/`wait_after` are pinned, while rejected predicate candidates are immediately discardable. Pinned frames for the current action are never evicted; exceeding either bound fails typed before another action tap. `jpeg_for_evidence(action_run_id, &FrameEvidence)` returns only an exact SHA-keyed clone. An `ActionEvidenceScopeGuard` calls `clear_action_evidence(action_run_id)` on success, cancellation, conversion error, or unwind, after synchronous access needed for artifact persistence has finished. The conversion block uses detector-set named crop rectangles for successful Save/Repost/DM confirmation, the already persisted recipient crop for DM identity, and one cached full frame only for failed/uncertain/not-confirmed retention. Cache miss is typed `EvidenceFrameEvicted` and never triggers `GET /screenshot`, DVT screenshot, or a fresh stream read.

```rust
#[derive(Debug, thiserror::Error)]
pub enum ActionProbeError {
    #[error("current frame is unavailable")]
    FrameUnavailable,
    #[error("current frame is not a valid JPEG")]
    Decode,
    #[error("frame geometry is outside the qualified tuple")]
    UnsupportedGeometry,
    #[error("frame wait was cancelled")]
    Cancelled,
    #[error("frame predicate deadline elapsed")]
    Deadline,
    #[error("another action already owns the evidence cache")]
    EvidenceScopeActive,
    #[error("the current action returned more than twelve evidence frames")]
    EvidenceFrameLimit,
    #[error("the current action returned more than 16 MiB of evidence frames")]
    EvidenceByteLimit,
    #[error("the requested evidence frame is not retained for this action")]
    EvidenceFrameEvicted,
}

struct CachedEvidenceFrame {
    action_run_id: ActionRunId,
    evidence: FrameEvidence,
    jpeg: Frame,
}

const MAX_ACTION_EVIDENCE_FRAMES: usize = 12;
const MAX_ACTION_EVIDENCE_BYTES: usize = 16 * 1024 * 1024;

struct ActionEvidenceScopeGuard<'a> {
    probe: &'a FrameProbe,
    action_run_id: ActionRunId,
}

impl Drop for ActionEvidenceScopeGuard<'_> {
    fn drop(&mut self) {
        self.probe.clear_action_evidence(&self.action_run_id);
    }
}

impl FrameProbe {
    pub fn begin_action_evidence(
        &self,
        action_run_id: ActionRunId,
    ) -> Result<ActionEvidenceScopeGuard<'_>, ActionProbeError>;
    pub fn jpeg_for_evidence(
        &self,
        action_run_id: &ActionRunId,
        evidence: &FrameEvidence,
    ) -> Result<Frame, ActionProbeError>;
    pub fn clear_action_evidence(&self, action_run_id: &ActionRunId);
}
```

`FrameProbe::latest` and the successful return path of `wait_after` call one private `pin_returned_frame(&ObservedFrame)` function; the predicate loop never calls it for rejected candidates. That function reads the current active `ActionRunId` under the cache mutex, deduplicates by `(action_run_id, evidence.digest)`, checks both prospective bounds with `checked_add`, and only then pushes the `Frame` clone. `jpeg_for_evidence` requires the active action ID and both the evidence SHA and geometry metadata to match the cached frame. `clear_action_evidence` removes only rows for the exact active ID and clears ownership atomically under the same mutex. Begin a scope before invoking the action and keep the guard alive through outcome conversion and synchronous JPEG lookup; perform async `ArtifactStore::put` only after cloning the bounded bytes out of the cache.

- [ ] **Step 4: Preserve typed uncertainty through assignment aggregation**

After issued Repost/DM intent, a cancellation, transport loss, frame loss, process crash, crop-encoding error, cache miss, or artifact-write error finishes/freezes the action as `Uncertain`; use `ActionOutcomeCode::EvidencePersistenceFailed` for the last three and never replay the effect. The same evidence failure for idempotent Save becomes `NotConfirmed/EvidencePersistenceFailed`; G1 may run a bounded retry whose first step re-reads Saved and performs no second tap. A pre-intent evidence failure remains an executor failure, and unfinished non-issued later actions become `Interrupted`. The assignment becomes `Uncertain` if it has no positive action, otherwise `Partial`, using the existing reducer. Persist the typed outcome even when its evidence list is empty, emit the artifact error code without a path/content, and ensure no cleanup action overwrites the primary result.

- [ ] **Step 5: Verify GREEN and commit**

```powershell
cargo test -p riviu-core --test interaction_device_batch g4 -- --nocapture
cargo test -p riviu-core --test interaction_transitions -- --nocapture
cargo test -p riviu-core --test interaction_recovery -- --nocapture
cargo test -p riviu-core --test interaction_artifacts g4_evidence -- --nocapture
git add crates/core/src/interaction/device_batch_executor.rs crates/core/src/interaction/types.rs crates/core/src/tiktok_actions/frame_probe.rs crates/core/tests/interaction_device_batch.rs crates/core/tests/interaction_artifacts.rs
git diff --cached --name-only
git commit -m "feat(interaction): execute independently gated G4 actions"
```

---

### Task 11: Extend Exact-Tuple Capability Negotiation And Action-Specific Revocation

**Files:**
- Modify: `crates/core/src/device_capabilities.rs`
- Modify: `crates/ios-driver/src/interaction_runtime.rs`
- Modify: `crates/ios-driver/src/mock.rs`
- Modify: `apps/desktop/src-tauri/src/state.rs`
- Modify: `sidecars/wda/interaction-capabilities.schema.json`
- Modify: `sidecars/wda/interaction-capabilities.json`

- [ ] **Step 1: Write failing default-deny and tuple-drift tests**

Test Save, Repost, allowlisted DM, and random-visible DM separately. Change one field at a time: artifact SHA, protocol, driver adapter, transport, route contract, iOS, TikTok version/build/locale, layout, G2 production-runtime qualification ID/report SHA, detector-set digest, geometry, orientation, OCR engine/version/source commit, OCR runtime/name/version, model source revision/set ID/digest, preprocessing version, consensus rule, recipient band, recipient-locator version, submit mode, submit-contract detector version, submit-ready detector version, send-confirmation detector version, and evidence report hash. Missing or drifting G2 runtime qualification disables every G4 action; an action-specific field disables only that affected capability. Test action-specific revocation precedence.

```rust
#[test]
fn ocr_model_drift_disables_only_direct_message() {
    let registry = fixture_g4_registry();
    let mut runtime = fixture_g4_runtime();
    runtime.ocr.model_set_digest = "changed".into();
    let capabilities = registry.negotiate(&runtime);
    assert!(capabilities.ui.save.is_some());
    assert!(capabilities.ui.repost.is_some());
    assert!(capabilities.ui.direct_message.is_none());
}
```

- [ ] **Step 2: Verify RED**

```powershell
cargo test -p riviu-core device_capabilities::tests::g4 -- --nocapture
cargo test -p riviu-ios-driver interaction_capability_registry::g4 -- --nocapture
```

- [ ] **Step 3: Add independently typed capabilities**

Extend G1's closed `CapabilityUnavailableCode` with exactly `OcrUnavailable` and `LiveQualificationRevoked`. The former is used only when an otherwise matching Direct Message tuple lacks a checksum-valid, loadable local recognizer; the latter is used only when an exact action revocation wins. Other tuple drift retains `DeviceTupleUnqualified`, and an action with no reviewed entry retains `GateNotQualified`.

```rust
pub struct UiCapabilities {
    pub open_url: Option<OpenUrlCapability>,
    pub clipboard: Option<ClipboardCapability>,
    pub target_identity_copy_link: Option<TargetIdentityCapability>,
    pub interaction_runtime: Option<VerifiedInteractionRuntimeCapability>,
    pub like: Option<VerifiedActionCapability>,
    pub follow: Option<VerifiedActionCapability>,
    pub comment: Option<VerifiedActionCapability>,
    pub save: Option<VerifiedActionCapability>,
    pub repost: Option<RepostCapability>,
    pub direct_message: Option<DirectMessageCapability>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum OcrRuntimeSnapshot {
    Ready { descriptor: RecognizerDescriptor },
    Unavailable { code: OcrAvailabilityCode },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum RepostConfirmationContract {
    StableRemoveRepost,
    QualifiedSuccessToast,
}

pub struct RepostCapability {
    pub base: VerifiedActionCapability,
    pub confirmation_contract: RepostConfirmationContract,
}

pub struct DirectMessageCapability {
    pub base: VerifiedActionCapability,
    pub tiktok_locale: String,
    pub recipient_locator_version: String,
    pub ocr_engine: String,
    pub ocr_engine_version: String,
    pub ocr_engine_source_commit: String,
    pub ocr_runtime: String,
    pub ocr_runtime_version: String,
    pub model_source_revision: String,
    pub model_set_id: String,
    pub model_set_digest: String,
    pub preprocessing_version: String,
    pub consensus_rule_version: String,
    pub recipient_band: NormalizedRect,
    pub submit_mode: DirectMessageSubmitMode,
    pub submit_contract_detector_version: String,
    pub submit_ready_detector_version: String,
    pub send_confirmation_detector_version: String,
    pub allowlist: bool,
    pub random_visible: bool,
}
```

A `VerifiedActionCapability` carries detector-set version/digest and live report SHA. Repost additionally carries its confirmation contract. Direct Message additionally carries locale, OCR, locator, submit mode, and independently qualified recipient modes.

- [ ] **Step 4: Extend the strict registry schema and empty/default data**

Each qualification retains the exact G0 base key plus the exact G2 `interactionRuntime` qualification ID/report SHA and adds optional `actions.save`, `actions.repost`, and `actions.directMessage`. Add top-level `revocations`, keyed by `qualificationId + action`, with UTC time, reason code, and source report SHA. Reject unknown fields, duplicate entries/revocations, malformed hashes, an action entry without its G0 identity capability and matching G2 runtime gate, DM without OCR/submit-mode evidence, and a revocation for an unknown qualification.

Preserve every checked-in G0-G3 qualification byte-for-byte at the semantic JSON value level. Before Task 13, add only a schema-compatible empty `revocations` array when the existing registry does not already have one; do not replace `qualifications` with an empty array, reorder existing entries, or remove existing Like/Follow/Comment evidence. Add a regression test that loads the pre-migration registry fixture, writes it through the extended parser, and proves all pre-existing qualification objects are unchanged.

- [ ] **Step 5: Bind runtime OCR availability only to Direct Message**

`OcrRuntimeSnapshot` is a core-owned DTO in `device_capabilities.rs`; `ios-driver` must not import the desktop-only `interaction_ocr` module. `OcrRuntime` exposes a redacted conversion that returns `Ready { descriptor }` or `Unavailable { code }`. During `AppState::bootstrap`, desktop passes that snapshot into `RegistryBackedInteractionCapabilities`; `interaction_runtime.rs` combines it with the device tuple while building `UiCapabilities`. Test the ready, each typed unavailable, and descriptor-drift paths through this composition boundary.

Registry equality plus local checksum/load readiness is required for DM. Save/Repost do not depend on OCR. A manifest feature, fixture registry, or action HTTP ACK cannot make any G4 field ready.

- [ ] **Step 6: Verify GREEN and commit schema support**

```powershell
cargo test -p riviu-core device_capabilities::tests::g4 -- --nocapture
cargo test -p riviu-ios-driver interaction_capability_registry::g4 -- --nocapture
git add crates/core/src/device_capabilities.rs crates/ios-driver/src/interaction_runtime.rs crates/ios-driver/src/mock.rs apps/desktop/src-tauri/src/state.rs sidecars/wda/interaction-capabilities.schema.json sidecars/wda/interaction-capabilities.json
git diff --cached --name-only
git commit -m "feat(interaction): negotiate G4 action capabilities"
```

---

### Task 12: Build The Fixed G4 Live Harness And Transactional Verifier

**Files:**
- Create: `apps/desktop/src-tauri/src/bin/live_interaction_new_action_gates.rs`
- Create: `tools/interaction-gate4/verify_report.py`
- Create: `tools/interaction-gate4/test_verify_report.py`
- Create: `docs/re/interaction-gate4/README.md`

- [ ] **Step 1: Write failing verifier tests against fixture reports**

Generate reports in temporary directories. Prove `FIXTURE_ONLY` never passes; one action-local failure does not block a different action's candidate; lowered counts are rejected; intent after tap is rejected; missing crop/hash/tuple/cleanup is rejected; a DM allowlist case without a newer `dm.searchEmpty` frame or with more than one production `/wda/keys` call is rejected; a pre-filled-search negative that types or taps an effect is rejected; OCR/model/locale drift rejects DM; an ambiguous Repost/DM with more than one effect tap is rejected; token, raw UDID, fixture URL, recipient handle/display label, and clipboard bytes are rejected from published JSON/Markdown; partial JSON/Markdown/candidate publication rolls back all destinations. Also prove that any production-artifact, G0 base, G2 runtime, global lifecycle/cleanup, or redaction failure forces overall `FAIL` and suppresses every candidate even when individual action rows pass. Prove the final report bytes do not embed their own SHA-256, every candidate fragment binds the recomputed final JSON SHA, and changing either file invalidates promotion. Promotion tests must prove a multi-action transaction snapshots the original registry exactly once, never overwrites that snapshot on its second/third action, restores the original bytes after any promote/parser/package/staging/commit failpoint, and seals only after commit. Sanitized reports retain only hashes plus match booleans for fixture URLs/handles.

```python
def test_fixture_report_never_qualifies(self):
    report = passing_report(environment="FIXTURE_ONLY")
    self.assertEqual("FIXTURE_ONLY", verify_report.evaluate(report)["gateStatus"])


def test_dm_requires_prepared_recipient_then_intent_then_tap(self):
    report = passing_report()
    case = report["actions"]["directMessage"]["cases"][0]
    case["preparedSequence"], case["intentSequence"], case["effectTapSequence"] = 3, 2, 1
    self.assertIn("dm_event_order", verify_report.evaluate(report)["failures"])
```

- [ ] **Step 2: Verify RED**

```powershell
python -m unittest discover -s tools/interaction-gate4 -p "test_verify_report.py" -v
cargo test -p riviu-managers-phone --bin live_interaction_new_action_gates -- --nocapture
```

- [ ] **Step 3: Implement a fixed, non-lowerable live matrix**

The harness accepts only UDID, the three fixture URLs, one allowlist handle, expected random-visible handles, report directory, and optional action filter. Counts are constants, not CLI flags:

```rust
const SAVE_CYCLES: usize = 5;
const REPOST_CYCLES: usize = 5;
const REPOST_AMBIGUITY_CASES: usize = 1;
const DM_ALLOWLIST_SENDS: usize = 3;
const DM_PREFILLED_SEARCH_REJECTIONS: usize = 1;
const DM_RANDOM_SENDS: usize = 3;
const DM_AMBIGUITY_CASES: usize = 1;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct EffectOrderEvidence {
    base_prepared_sequence: Option<u64>,
    prepared_sequence: Option<u64>,
    intent_sequence: Option<u64>,
    effect_tap_sequence: u64,
    prepared_at: Option<DateTime<Utc>>,
    intent_at: Option<DateTime<Utc>>,
    effect_tap_at: DateTime<Utc>,
    effect_tap_count: u32,
}
```

Every device case acquires the shared `DeviceWorkCoordinator`, uses install-only inspect/repair, creates the profile-approved session before MJPEG, receives a first frame, opens and Copy-Link-verifies the exact target, executes one action, closes UI, releases the owned stream, and proves cleanup. Wrap progress and session boundaries with one audit sink whose `AtomicU64` assigns a strictly increasing sequence after each durable prepared/intent commit is observed and immediately before the adapter forwards a real effect tap. For Direct Message, `prepared_sequence` is assigned only after the resolved-recipient payload with its owning crop reference commits; the earlier unresolved base-payload event is reported separately and never satisfies the order gate. Ordering gates compare these sequence fields, not wall-clock timestamps; timestamps remain diagnostic only. Every gesture is tagged `setup`, `productionNavigation`, `productionEffect`, or `cleanup`, so Save/Repost state reset and DM pre-filled-query setup cannot inflate production effect counts. The adapter logs no request headers or token values.

All three allowlist-send cases must start with the planned handle absent from the visible recipient tiles, then record a newer `dm.searchEmpty` frame hash before exactly one production `/wda/keys` call and a still newer exact-result frame. Add one no-send pre-filled negative: after a harness setup call types a different controlled ASCII query into a proven-empty field, invoke the production search path for the planned handle. It must return `RecipientSearchNotEmpty`, perform zero additional `/wda/keys` calls, issue no intent, and make no recipient/Send effect tap. The audit adapter labels setup versus production calls; restart the controlled TikTok case afterward rather than relying on an unqualified clear operation.

Make both ambiguity cases deterministic with a harness-only `PostEffectEvidenceDropper`: after the journal records intent and the audit adapter forwards exactly one real effect tap, it temporarily withholds only frames that satisfy the Repost/DM confirmation predicate until the production timeout expires. It never substitutes a fixture frame, blocks the gesture, or changes driver responses. The action must finish `Uncertain` with one effect tap and zero replay. Then disable the dropper, inspect the real state read-only, and clean up the controlled fixture. Report the drop window/timing and `faultInjection="post_effect_evidence_drop"`; the verifier rejects this marker on ordinary success cases.

- [ ] **Step 4: Encode the exact acceptance thresholds**

`verify_report.py` requires:

```text
environment == LIVE_MAC_DEVICE
production IPA and manifest SHA-256 unchanged
exact G0 base qualification tuple present
exact matching G2 interactionRuntime qualification ID and live-report SHA present
fixture corpus and detector-set hashes exact
Save: 5/5 Unsaved -> Saved, 5/5 AlreadySatisfied no-tap checks, 5/5 cleanup to Unsaved
Save retry ownership: <=1 desired-state tap per invocation and <=3 across one persisted attempt chain for the same assignment/action ordinal
Repost: 5/5 Available -> Remove repost with intentSequence < effectTapSequence, 5/5 AlreadySatisfied no-tap checks, 5/5 cleanup
Repost ambiguity: 1/1 Uncertain, intentSequence < effectTapSequence, exactly one effect tap, no retry
DM allowlist: 3/3 handle initially offscreen, fresh text session, newer dm.searchEmpty proof, exactly one production keys call, exact planned-handle hash match, preparedSequence < intentSequence < effectTapSequence, sent evidence
DM pre-filled search: 1/1 RecipientSearchNotEmpty, zero additional production keys calls, zero intent, zero effect tap
DM random-visible: 3/3 >=2 eligible tiles, persisted seeded-handle hash match, preparedSequence < intentSequence < effectTapSequence, sent evidence
DM ambiguity: 1/1 Uncertain after a real send tap, exactly one effect tap, no retry
DM submit mode: every case matches one detector-backed mode from the live tuple; pre-effect `dm.submitReady` and post-effect `dm.sent` evidence use that mode's exact three detector versions; evidence for one mode never qualifies the other
DM wrong-recipient count == 0
all accepted OCR candidates consensusConfidenceBps == 10000
OCR release-build p95 <= 2000 ms and maximum <= 3000 ms across fixture/live crops
zero WDA screenshot calls
all evidence paths relative, hashes valid, and retention metadata present
all sessions/streams/readers/relays stopped and both device ports closed at cleanup
redaction PASS
```

Fixture reports may exercise every branch but return `gateStatus=FIXTURE_ONLY`. Common gates are production-artifact integrity, the exact G0 base tuple, the matching G2 runtime gate, global lifecycle/cleanup, and redaction. Failure of any common gate forces overall `FAIL` and suppresses every candidate. Only after all common gates pass is a structurally valid live report `PASS` when all requested actions pass, `PARTIAL` when at least one requested action passes and at least one has an action-local failure, or `FAIL` when none pass. `verify` accepts and publishes a valid `PARTIAL` report without turning a failed action into a candidate; `promote --action` remains the per-action enforcement boundary.

- [ ] **Step 5: Implement atomic report publication and candidate generation**

The verifier first serializes the final sanitized `gate-4.json` without a `liveReportSha256` field, computes SHA-256 over those exact final bytes, then derives one candidate registry fragment per passing action with that digest in `liveReportSha256`. It derives no fragments when any common gate failed. The report may contain the stable `qualificationId` used by the operator, but never embeds its own digest or a digest of a candidate that points back to it. Write JSON, Markdown, and candidate fragments into a transaction directory, run raw-byte plus decoded-leaf redaction on every file, then publish JSON/Markdown with rollback on partial replacement. Candidate fragments are atomically stored under `target/interaction-gate4/candidates/<report-sha256>/<action>.json`; `promote` recomputes the report hash and candidate fields from evidence and requires byte-equivalence with that fragment when it exists. A missing target fragment is regenerated only after full report validation and common-gate success. Candidate entries are not inserted into production automatically.

Promotion syntax is exact:

```bash
python3 tools/interaction-gate4/verify_report.py promote \
  --report docs/re/interaction-gate4/gate-4.json \
  --registry sidecars/wda/interaction-capabilities.json \
  --action save \
  --transaction target/interaction-gate4/promotion-transaction
```

Promotion validates the full report again, rejects a dirty/mismatched report or candidate, and requires a retained transaction directory. The first action snapshots the original registry bytes and digest exactly once; later actions validate that snapshot and append their journal entries without replacing it. Each call writes a temp file, fsyncs it, and atomically replaces the registry. `rollback-promotion --transaction ...` restores the one original snapshot after any later failure, while `seal-promotion` removes rollback state only after the reviewed Git commit succeeds. Repeat `promote` separately for `repost` and `directMessage` inside the same transaction.

- [ ] **Step 6: Implement action-specific revocation and rollback proof**

The concrete command below is the Direct Message case and is valid only after that action has a `PASS` candidate; verifier tests repeat the same contract for Save and Repost.

```bash
python3 tools/interaction-gate4/verify_report.py rollback \
  --registry sidecars/wda/interaction-capabilities.json \
  --qualification-id "$(python3 -c 'import json; p=json.load(open("docs/re/interaction-gate4/gate-4.json", encoding="utf-8")); print(p["actions"]["directMessage"]["qualificationId"])')" \
  --action directMessage \
  --reason live_regression \
  --source-report docs/re/interaction-gate4/gate-4.json
```

The command appends a report-hash-bound revocation record transactionally; it never deletes campaign history or edits the Agent artifact. Runtime negotiation immediately reports only that action as `Unavailable/LiveQualificationRevoked`. Add a test that rollback followed by parser reload disables DM while Save/Repost remain ready. Implement `restore-registry --snapshot ... --registry ...` as the byte-exact, temp-write/fsync/atomic-replace helper used only by the live rollback drill; it validates the snapshot as a complete registry before replacement. This drill helper does not seal, replace, or weaken the separate pre-promotion transaction snapshot.

- [ ] **Step 7: Verify fixture harness and verifier GREEN**

```powershell
python -m unittest discover -s tools/interaction-gate4 -p "test_verify_report.py" -v
cargo test -p riviu-managers-phone --bin live_interaction_new_action_gates -- --nocapture
```

Expected: fixture matrix covers all branches but never yields PASS; transactional failure tests restore all prior files.

- [ ] **Step 8: Commit harness and verifier**

```powershell
git add apps/desktop/src-tauri/src/bin/live_interaction_new_action_gates.rs tools/interaction-gate4/verify_report.py tools/interaction-gate4/test_verify_report.py docs/re/interaction-gate4/README.md
git diff --cached --name-only
git commit -m "test(interaction): add fixed G4 live gates"
```

---

### Task 13: Run Live Gates, Promote Independently, And Drill Rollback

**Files:**
- Create after live run: `docs/re/interaction-gate4/gate-4.json`
- Create after live run: `docs/re/interaction-gate4/gate-4.md`
- Modify after reviewed PASS: `sidecars/wda/interaction-capabilities.json`
- Modify: `AGENTS.md`

- [ ] **Step 1: Verify the Mac environment and immutable artifacts**

```bash
export PATH="$HOME/Library/Python/3.9/bin:$PATH"
: "${RIVIU_GATE4_UDID:?set the live device UDID}"
: "${RIVIU_GATE4_SAVE_URL:?set the controlled Save fixture URL}"
: "${RIVIU_GATE4_REPOST_URL:?set the controlled Repost fixture URL}"
: "${RIVIU_GATE4_DM_URL:?set the controlled DM fixture URL}"
: "${RIVIU_GATE4_DM_ALLOWLIST_HANDLE:?set the exact allowlisted @handle}"
: "${RIVIU_GATE4_DM_RANDOM_HANDLES:?set comma-separated expected visible @handles}"
rustup toolchain install 1.89.0
python3 -m pip install -r sidecars/pymobiledevice3/requirements.txt
python3 -m pip install -r tools/interaction-gate4/requirements.txt
python3 -c 'import PIL; assert PIL.__version__ == "11.3.0"; print("PILLOW_OK")'
python3 sidecars/ocr/fetch_models.py
python3 sidecars/ocr/fetch_models.py --verify
shasum -a 256 sidecars/wda/RiviuAgent.ipa sidecars/wda/agent-manifest.json
```

Expected: Python dependencies import, Rust 1.89.0 exists, OCR files match the lock, and production hashes match the preconditions.

- [ ] **Step 2: Remove competing device owners**

Close the desktop, all nurture/live harnesses, 3uTools, and other XCTest runners. Then kill only the exact known competing bundles:

```bash
tidevice -u "$RIVIU_GATE4_UDID" kill notes.3u || true
tidevice -u "$RIVIU_GATE4_UDID" kill com.mrph.svc || true
tidevice -u "$RIVIU_GATE4_UDID" kill com.riviu.managersphone.agent.xctrunner || true
```

Keep the phone unlocked, portrait, on the qualified TikTok build, and signed into the controlled fixture account.

- [ ] **Step 3: Build the release harness with pinned OCR**

```bash
cargo +1.89.0 build -p riviu-managers-phone --bin live_interaction_new_action_gates --release
```

Expected: release build succeeds; debug OCR performance is not used for qualification.

- [ ] **Step 4: Run the fixed live matrix**

```bash
RIVIU_WDA_BACKEND=rt-mmo \
RIVIU_RTMMO_IPA="$PWD/sidecars/wda/RiviuAgent.ipa" \
RIVIU_RTMMO_TOKEN="$(security find-generic-password -s riviu-managers-phone -a agent-auth-token -w)" \
./target/release/live_interaction_new_action_gates \
  --udid "$RIVIU_GATE4_UDID" \
  --save-url "$RIVIU_GATE4_SAVE_URL" \
  --repost-url "$RIVIU_GATE4_REPOST_URL" \
  --dm-url "$RIVIU_GATE4_DM_URL" \
  --allowlist-handle "$RIVIU_GATE4_DM_ALLOWLIST_HANDLE" \
  --expected-random-handles "$RIVIU_GATE4_DM_RANDOM_HANDLES" \
  --report-dir target/interaction-gate4/live
```

Expected: the harness has no sample-count switches, always finalizes per-action results and cleanup evidence, and exits `0` only when every requested action passes its fixed matrix. A nonzero gate result still leaves a complete raw report for Step 5; it never leaves a half-written report.

- [ ] **Step 5: Verify and publish sanitized evidence**

```bash
python3 tools/interaction-gate4/verify_report.py verify \
  --input target/interaction-gate4/live/report.json \
  --output-dir docs/re/interaction-gate4
cargo run -q -p rtmmo-re -- verify-redaction \
  --input docs/re/interaction-gate4/gate-4.json \
  --input docs/re/interaction-gate4/gate-4.md
```

Expected: `environment=LIVE_MAC_DEVICE`, every per-action status is explicit, `gateStatus` is `PASS`, `PARTIAL`, or `FAIL` by the fixed rule in Task 12, report publication is atomic, cleanup is clean, and redaction passes. Only a per-action `PASS` can be promoted.

- [ ] **Step 6: Run complete regressions before mutating the capability registry**

```bash
REGISTRY_BEFORE="$(shasum -a 256 sidecars/wda/interaction-capabilities.json | awk '{print $1}')"
cargo +1.89.0 fmt --all -- --check
cargo +1.89.0 test --workspace
cargo +1.89.0 clippy --workspace --all-targets -- -D warnings
npm --prefix apps/desktop test -- --run
npm --prefix apps/desktop run build
python3 -m unittest discover -s sidecars/ocr -p "test_fetch_models.py" -v
python3 -m unittest discover -s tools/interaction-gate4 -p "test_fixture_manifest.py" -v
python3 -m unittest discover -s tools/interaction-gate4 -p "test_verify_report.py" -v
npm --prefix apps/desktop run tauri:build -- --debug
shasum -a 256 sidecars/wda/RiviuAgent.ipa sidecars/wda/agent-manifest.json
test "$(shasum -a 256 sidecars/wda/interaction-capabilities.json | awk '{print $1}')" = "$REGISTRY_BEFORE"
```

Expected: all Rust/frontend/Python tests pass, Tauri packages the verified OCR model set, both production artifact hashes remain unchanged, and the capability registry is still byte-identical to its pre-G4 state.

- [ ] **Step 7: Promote passing actions and drill rollback inside one retained transaction**

Run every passing action through the same transaction. The first `promote` call creates the only original-registry snapshot; later calls must preserve it. Any promote, parser, revocation-drill, or restoration failure rolls the entire registry back to that original snapshot.

```bash
set -Eeuo pipefail
TX=target/interaction-gate4/promotion-transaction
REGISTRY=sidecars/wda/interaction-capabilities.json
rollback_gate4() {
  trap - ERR INT TERM
  if test -d "$TX"; then
    python3 tools/interaction-gate4/verify_report.py rollback-promotion \
      --transaction "$TX" \
      --registry "$REGISTRY"
  fi
}
trap rollback_gate4 ERR INT TERM
rm -rf "$TX"
PASS_ACTIONS="$(python3 -c 'import json; p=json.load(open("docs/re/interaction-gate4/gate-4.json", encoding="utf-8")); print(" ".join(name for name in ("save", "repost", "directMessage") if p["actions"][name]["status"] == "PASS"))')"
test -n "$PASS_ACTIONS"
for action in $PASS_ACTIONS; do
  python3 tools/interaction-gate4/verify_report.py promote \
    --report docs/re/interaction-gate4/gate-4.json \
    --registry "$REGISTRY" \
    --action "$action" \
    --transaction "$TX"
  cargo test -p riviu-ios-driver interaction_capability_registry::g4 -- --nocapture
done

cp "$REGISTRY" target/interaction-gate4/registry-pre-drill.json
PRE_DRILL_SHA="$(shasum -a 256 target/interaction-gate4/registry-pre-drill.json | awk '{print $1}')"
DRILL_ACTION="$(python3 -c 'import json; p=json.load(open("docs/re/interaction-gate4/gate-4.json", encoding="utf-8")); print(next(name for name in ("directMessage", "repost", "save") if p["actions"][name]["status"] == "PASS"))')"
QUALIFICATION_ID="$(DRILL_ACTION="$DRILL_ACTION" python3 -c 'import json,os; p=json.load(open("docs/re/interaction-gate4/gate-4.json", encoding="utf-8")); print(p["actions"][os.environ["DRILL_ACTION"]]["qualificationId"])')"
python3 tools/interaction-gate4/verify_report.py rollback \
  --registry "$REGISTRY" \
  --qualification-id "$QUALIFICATION_ID" \
  --action "$DRILL_ACTION" \
  --reason rollback_drill \
  --source-report docs/re/interaction-gate4/gate-4.json
cargo test -p riviu-ios-driver interaction_capability_registry::g4 -- --nocapture
python3 tools/interaction-gate4/verify_report.py restore-registry \
  --snapshot target/interaction-gate4/registry-pre-drill.json \
  --registry "$REGISTRY"
test "$(shasum -a 256 "$REGISTRY" | awk '{print $1}')" = "$PRE_DRILL_SHA"
cargo test -p riviu-ios-driver interaction_capability_registry::g4 -- --nocapture
trap - ERR INT TERM
```

Expected: the exact live tuple exposes only independently passing actions, the drill temporarily exposes `Unavailable/LiveQualificationRevoked` only for `$DRILL_ACTION`, restoration is byte-exact, and the retained transaction still holds the untouched pre-promotion registry. No passing action or later promote overwrites that snapshot.

- [ ] **Step 8: Verify the promoted registry and packaged resources with rollback armed**

```bash
set -Eeuo pipefail
TX=target/interaction-gate4/promotion-transaction
REGISTRY=sidecars/wda/interaction-capabilities.json
rollback_gate4() {
  trap - ERR INT TERM
  if test -d "$TX"; then
    python3 tools/interaction-gate4/verify_report.py rollback-promotion \
      --transaction "$TX" \
      --registry "$REGISTRY"
  fi
}
trap rollback_gate4 ERR INT TERM
python3 tools/interaction-gate4/verify_report.py verify-promoted \
  --report docs/re/interaction-gate4/gate-4.json \
  --registry "$REGISTRY" \
  --transaction "$TX"
cargo test -p riviu-ios-driver interaction_capability_registry::g4 -- --nocapture
npm --prefix apps/desktop run tauri:build -- --debug
shasum -a 256 sidecars/wda/RiviuAgent.ipa sidecars/wda/agent-manifest.json
trap - ERR INT TERM
```

Expected: focused negotiation and the packaged desktop consume the promoted entries, both production artifact hashes remain unchanged, and any failure restores the complete pre-promotion registry before exiting.

- [ ] **Step 9: Update the handoff record**

Before editing, run `mkdir -p target/interaction-gate4 && cp AGENTS.md target/interaction-gate4/AGENTS.before-promotion.md`. Add a dated G4 section to `AGENTS.md` recording:

```text
- OCR engine/model set IDs, exact SHA-256 values, model license, MSRV, and packaged paths.
- Exact detector-set digest and fixture manifest hash.
- Exact live capability tuple and report SHA for each promoted action.
- Repost/DM intent-before-tap and no-repeat invariant.
- Direct Message consensus confidence semantics and exact-handle-only rule.
- Any action still disabled and its typed first failing gate.
- Action-specific rollback command and successful rollback-drill evidence.
- Production IPA/manifest hashes remained unchanged.
```

If the handoff edit or any following staging/commit check fails, restore that snapshot and run `rollback-promotion` against the retained transaction before stopping.

- [ ] **Step 10: Commit reviewed evidence and qualifications**

```bash
set -Eeuo pipefail
TX=target/interaction-gate4/promotion-transaction
REGISTRY=sidecars/wda/interaction-capabilities.json
HANDOFF_BEFORE=target/interaction-gate4/AGENTS.before-promotion.md
rollback_gate4_commit() {
  trap - ERR INT TERM
  git restore --staged -- "$REGISTRY" docs/re/interaction-gate4 AGENTS.md || true
  if test -f "$HANDOFF_BEFORE"; then cp "$HANDOFF_BEFORE" AGENTS.md; fi
  if test -d "$TX"; then
    python3 tools/interaction-gate4/verify_report.py rollback-promotion \
      --transaction "$TX" \
      --registry "$REGISTRY"
  fi
}
trap rollback_gate4_commit ERR INT TERM
test -f "$HANDOFF_BEFORE"
git add "$REGISTRY" docs/re/interaction-gate4/gate-4.json docs/re/interaction-gate4/gate-4.md
git add -p AGENTS.md
git diff --cached --name-only
git diff --cached --check
git commit -m "test(interaction): qualify G4 action tuple"
trap - ERR INT TERM
python3 tools/interaction-gate4/verify_report.py seal-promotion \
  --transaction "$TX"
rm -f "$HANDOFF_BEFORE" target/interaction-gate4/registry-pre-drill.json
git show --stat --oneline HEAD
```

If only a subset passed, stage and describe only those capability entries; keep failing actions disabled with their evidence report.

---

## G4 Completion Criteria

- Save, Repost, and Direct Message remain independently default-deny and independently revocable.
- Save never taps without a positively located rail and unambiguous Unsaved state; success requires a newer Saved frame; one invocation has at most one tap and only G1 owns the persisted max-three retry budget.
- Repost distinguishes `Repost` from `Remove repost`, commits intent before its only effect tap, and maps ambiguous completion to `Uncertain` without replay.
- Direct Message accepts only unanimous, exact normalized ASCII `@handle` OCR from two stable frames; display labels never match.
- Allowlist selection is sampled/persisted once; random-visible selection is sorted and seeded deterministically, persisted before tap, and never changes on retry.
- Allowlist search uses a trusted fresh text session and the profile-approved session-before-MJPEG lifecycle; it types only after a newer frame proves `dm.searchEmpty`, and never assumes an unqualified clear-text primitive.
- DM submit mode is part of the exact capability tuple; confirmation requires a qualified post-send frame/toast and resolved recipient evidence.
- OCR engine, model files, licenses, checksums, aggregate digest, MSRV, and Tauri resources are pinned and verified.
- Fixture/negative gates have zero false taps; fixture-only evidence cannot qualify production.
- Every action evidence file is bounded, hashed, relatively addressed, retained by the existing artifact policy, and omitted from broadcast events.
- Runtime tuple drift or revocation disables the affected action before a coordinate or text side effect.
- Any production-artifact, G0 base, G2 runtime, global cleanup, or redaction failure forces G4 `FAIL` and suppresses every capability candidate.
- Full regression precedes production promotion; all passing actions share one retained original-registry snapshot, every post-promotion failure restores it, and the transaction seals only after commit.
- Nurture, G0-G3 actions, shared device ownership, stream budget, and recovery regressions pass.
- Production Agent IPA/manifest remain byte-identical.

## Assumptions And Risks

- The controlled TikTok build visibly exposes exact ASCII `@handle` text in either the recipient lane or search results. If it exposes only display labels, Direct Message stays disabled for that tuple.
- The pinned ocrs model supports Latin text only; this is intentional because matching is limited to exact ASCII handles. It is not used for localized UI labels.
- ocrs 0.12.2 requires RTen 0.24.0 and Rust 1.89.0. The plan makes that build requirement explicit and tests it; it does not silently depend on the developer's newer default toolchain.
- TikTok can change Share/Repost/DM layout or submit semantics independently of app version. Detector/template/model/locale/submit-mode fields are therefore part of the exact qualification key, and drift fails closed.
- Repost and Direct Message are non-idempotent after intent. Ambiguous completion is operationally visible and requires a new explicit campaign rather than automatic retry.
- Model files add approximately 12 MiB to the packaged desktop resources. Build and runtime checksum verification prevent partial or substituted packaging.
