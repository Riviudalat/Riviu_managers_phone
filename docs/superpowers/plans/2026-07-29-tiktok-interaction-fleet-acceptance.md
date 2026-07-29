# TikTok Interaction Fleet Acceptance Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Deliver Gate G5 by proving the G0-G3 Interaction release on two real iPhones, qualifying only independently supported actions and stream budgets, smoke-testing the packaged desktop app, and publishing rollback-ready evidence without changing the production Agent artifacts.

**Architecture:** A fixed parent harness starts isolated worker processes against the existing `InteractionCampaignEngine`, `DeviceControlPlane`, capability registry, and G2/G4 action adapters. A separate verifier accepts only a complete two-device live report, checks event ordering, producer accounting, restart/no-replay behavior, package resources, cleanup, and redaction, stages sanitized evidence after rollback, and publishes the evidence trio only after the final regression passes; it never creates runtime capability claims. G4 actions are included only when both live device tuples already expose that action through the reviewed registry, so an unqualified G4 action remains disabled without blocking the first G0-G3 release.

**Tech Stack:** Rust 2021, Tokio, Tauri 2, React 19, TypeScript 6, SQLite/WAL, Python 3.9+, pymobiledevice3 10.1.0, Pillow 11.3.0, Playwright/Chromium, WDA/MJPEG over usbmux, macOS packaging tools.

---

## Preconditions And Stop Rules

- Execute only after G0, G1, G2, and G3 source gates pass, including the exact G2 `interaction_runtime` entry that attests the production identity/Watch executor on both live tuples. G4 source may be present, but each Save/Repost/Direct Message capability remains independently optional and fail-closed.
- Read `AGENTS.md` sections 2, 3.9, 3.12, and 4 before touching device lifecycle, streams, the capability registry, or live harnesses.
- Use the approved design commit `10433fb` and the first five plans in `2026-07-29-tiktok-interaction-roadmap.md` as the contract. Resolve a type conflict in its owning earlier gate before changing the acceptance harness.
- Keep `sidecars/wda/RiviuAgent.ipa` SHA-256 `8a24847099495ff70b998522692c43f00dd16b90f698bda6953a73f5d33002ea` and `sidecars/wda/agent-manifest.json` SHA-256 `e98a549af4c061556effd36424e7732219e1a6d262bcf1f259279975024b6e1a` byte-identical.
- Do not run the desktop app, Nurture harness, G0/G2/G4 probe, 3uTools, or another XCTest owner concurrently with the G5 live harness.
- Kill only exact known bundles/PIDs with existing fingerprint checks. Never use broad `pkill`.
- `FIXTURE_ONLY`, one device, lowered counts, missing package proof, or dirty cleanup can exercise branches but can never produce G5 PASS.
- The live-required release budget is `1`. Budget `2` is a separate measured promotion inside G5: failure leaves the configured/default budget at `1`; any producer overrun, leaked process, or dirty port is a G5 failure rather than a clean non-qualification.
- Do not add a parallel runtime release flag or qualification registry. Runtime support continues to come only from `sidecars/wda/interaction-capabilities.json` and its action-specific live evidence.
- Project 2 candidate B0/B/C/text gates remain separate. G5 packages the current reviewed production artifact and never switches the desktop to an unqualified candidate.

Tasks 1-5 are implementation tasks and each ends with its own focused commit. Tasks 6-13 make no implementation edits and no commits: their raw reports and sanitized publication staging stay under ignored `target/interaction-gate5/`, while every path under `docs/re/interaction-gate5/` remains unchanged. Task 14 runs the complete final regression and redaction checks against that staging directory before it may transactionally replace the published evidence trio. If an acceptance run exposes an implementation defect, stop, fix and commit that defect in its owning G0-G4 component, then restart G5 from Task 6 and capture a new baseline; do not patch code inside a live-evidence task.

## Fixed Acceptance Matrix

The harness compiles these values as constants; no CLI option or environment variable may lower them:

```rust
const REQUIRED_DEVICE_COUNT: usize = 2;
const TARGETS_PER_KIND: usize = 2;
const TARGET_KINDS: usize = 3; // direct video, photo, resolved short link
const ALL_ASSIGNMENTS: usize = 12; // 6 targets x 2 actors
const ROUND_ROBIN_ASSIGNMENTS: usize = 6;
const UNICODE_COMMENTS_PER_DEVICE: usize = 2;
const LIKE_TRANSITIONS_PER_DEVICE: usize = 1;
const FOLLOW_TRANSITIONS_PER_DEVICE: usize = 1;
const CANCELLATION_CASES: usize = 1;
const BUSY_PARTIAL_CASES: usize = 1;
const REQUIRED_BASE_LIVE_CRASH_CASES: usize = 2;
const REQUIRED_DM_PREPARED_FIXTURE_CRASH_CASES: usize = 1;
const CLEANUP_DEADLINE_SECONDS: u64 = 20;
```

Required cases:

| Case | Required result |
|---|---|
| budget-1 All | 12 terminal assignments, both actors used, max producer count 1 |
| budget-1 RoundRobin | 6 terminal assignments, deterministic 3/3 actor split, max producer count 1 |
| budget-2 diagnostic | two simultaneous device batches, max producer count 2; PASS promotes budget 2, clean non-pass keeps budget 1 |
| identity | two distinct direct videos, two photos, two short links; copied ID/kind match on both devices |
| G2 effects | Like, Follow, and two Unicode Comments per device with frame evidence |
| cancellation | cancel during a persisted pre-intent delay; no later target/effect opens |
| busy/partial | one held `ManualControl` lease yields `SkippedUnavailable`; other actor succeeds; no redistribution |
| restart after plan commit | crash before `Preparing`; restart reuses seeds/policies/timing, and the DM resolved-recipient payload is still absent |
| restart after DM preparation | fixture always; live only when Direct Message is Ready on both tuples: crash after durable resolved-recipient payload/evidence but before effect intent, then reload the exact bytes without resampling or replacement |
| restart after effect | one real Comment Send tap after committed intent, worker termination, restart => `Uncertain`, tap count 1, no replay |
| optional G4 | run a fixed per-device case only for an action already Ready on both exact live tuples |
| package | release app launches with mock devices, Interaction panel opens, packaged Agent/manifest hashes match source |
| rollback | registry revocation/restore and N-1 desktop/source rollback drill leave history and production artifacts intact |

## File Map

**Create**

- `apps/desktop/src-tauri/src/bin/live_interaction_fleet_acceptance.rs`: parent/worker live harness and fixed two-device matrix.
- `apps/desktop/src-tauri/src/bin/interaction_acceptance_worker.rs`: isolated crashable worker using production engine composition.
- `crates/core/src/interaction/acceptance_audit.rs`: test-only/feature-gated ordered audit sink with no secrets.
- `crates/core/tests/interaction_fleet_acceptance.rs`: fixture matrix, producer, cancellation, partial, and restart tests.
- `tools/interaction-gate5/verify_report.py`: strict evaluator, package verifier, target-only evidence stager, transactional trio publisher, and rollback helper.
- `tools/interaction-gate5/test_verify_report.py`: fixture, tamper, cleanup, redaction, and rollback transaction tests.
- `tools/interaction-gate5/package_smoke.py`: macOS packaged-app launch, UI activation/screenshot, resource, and cleanup probe.
- `tools/interaction-gate5/test_package_smoke.py`: package-layout and report fixture tests.
- `docs/re/interaction-gate5/README.md`: evidence schema and current `PENDING_MAC_TWO_DEVICE` state.
- after Task 14 final regression and atomic publication: `docs/re/interaction-gate5/gate-5.json` and `docs/re/interaction-gate5/gate-5.md`.

**Modify**

- `crates/core/Cargo.toml`
- `crates/core/src/interaction/mod.rs`
- `crates/core/src/interaction/dispatcher.rs`
- `crates/core/src/interaction/store.rs`
- `crates/core/src/interaction/artifacts.rs`
- `crates/core/src/device_control.rs`
- `crates/core/src/interaction/progress.rs`
- `crates/core/src/interaction/device_batch_executor.rs`
- `crates/core/src/tiktok_actions/identity.rs`
- `crates/core/src/tiktok_actions/verified.rs`
- `crates/core/src/tiktok_actions/comment.rs`
- `apps/desktop/src-tauri/Cargo.toml`
- `apps/desktop/e2e/interaction-workflow.spec.ts`
- `AGENTS.md`

**Must Remain Byte-Identical**

- `sidecars/wda/RiviuAgent.ipa`
- `sidecars/wda/agent-manifest.json`
- `sidecars/wda/WebDriverAgent/**`
- `sidecars/wda/riviu-agent/**`

---

### Task 1: Define The Strict G5 Report And Verifier

**Files:**
- Create: `tools/interaction-gate5/verify_report.py`
- Create: `tools/interaction-gate5/test_verify_report.py`
- Create: `docs/re/interaction-gate5/README.md`

- [ ] **Step 1: Write failing verifier tests**

Build reports in temporary directories. Test exact device/count requirements, distinct hashed devices, tuple equality, every fixed case, chronological event ordering, max producer counts, action-specific G4 inclusion, Nurture attachment, rollback evidence, cleanup, package resource hashes, registry hash, and transactional publication. Reject raw UDIDs, URLs, handles, comment text, tokens, clipboard bytes, proxy credentials/userinfo, local absolute paths, and source error chains from JSON and Markdown.

```python
class VerifyGate5Tests(unittest.TestCase):
    def test_one_device_never_passes(self):
        report = passing_report()
        report["devices"] = report["devices"][:1]
        result = verify_report.evaluate(report)
        self.assertEqual("FAIL", result["gateStatus"])
        self.assertIn("device_count", result["failures"])

    def test_fixture_environment_never_passes(self):
        report = passing_report()
        report["environment"] = "FIXTURE_ONLY"
        self.assertEqual("FIXTURE_ONLY", verify_report.evaluate(report)["gateStatus"])

    def test_budget_two_clean_nonqualification_keeps_release_budget_one(self):
        report = passing_report()
        report["streamBudgets"]["budget2"]["status"] = "NOT_QUALIFIED_CLEAN"
        report["streamBudgets"]["budget2"]["maxRunningProducers"] = 1
        result = verify_report.evaluate(report)
        self.assertEqual("PASS", result["gateStatus"])
        self.assertEqual(1, result["qualifiedStreamBudget"])

    def test_post_intent_restart_requires_one_tap_and_zero_replay(self):
        report = passing_report()
        crash = report["restarts"]["afterCommentEffect"]
        crash["effectTapCount"] = 2
        self.assertIn("effect_replayed", verify_report.evaluate(report)["failures"])

    def test_plan_commit_cannot_contain_a_prepared_dm_recipient(self):
        report = passing_report()
        crash = report["restarts"]["afterPlanCommit"]
        crash["preparedRecipientPresent"] = True
        crash["preparedRecipientPayloadLength"] = 16
        failures = verify_report.evaluate(report)["failures"]
        self.assertIn("recipient_prepared_at_plan_commit", failures)

    def test_dm_preparation_restart_reloads_exact_bytes_once(self):
        report = passing_report()
        crash = report["restarts"]["afterDmPreparation"]
        crash["afterPayloadSha256"] = "b" * 64
        crash["recipientSelectionCount"] = 2
        failures = verify_report.evaluate(report)["failures"]
        self.assertIn("dm_preparation_replaced", failures)
        self.assertIn("dm_recipient_resampled", failures)

    def test_staging_never_writes_the_publication_directory(self):
        with tempfile.TemporaryDirectory() as root:
            root = Path(root)
            staging = root / "target" / "interaction-gate5" / "publication"
            published = root / "docs" / "re" / "interaction-gate5"
            verify_report.stage_report(passing_report(), staging)
            self.assertFalse(published.exists())

    def test_failed_third_replacement_restores_the_entire_prior_trio(self):
        with tempfile.TemporaryDirectory() as root:
            fixture = publication_fixture(Path(root))
            before = fixture.read_published_bytes_or_absence()
            with self.assertRaises(OSError):
                fixture.publish_with_failure_on_replace(ordinal=3)
            self.assertEqual(before, fixture.read_published_bytes_or_absence())

    def test_nurture_and_rollback_are_required_before_pass(self):
        for required in ("nurture", "rollback"):
            report = passing_report()
            del report[required]
            self.assertIn(f"missing_{required}", verify_report.evaluate(report)["failures"])
```

- [ ] **Step 2: Run RED**

```powershell
python -m unittest discover -s tools/interaction-gate5 -p "test_verify_report.py" -v
```

Expected: import failure because `verify_report.py` does not exist.

- [ ] **Step 3: Implement a closed schema and evaluator**

`verify_report.py` accepts schema version `1` and rejects unknown/missing keys. Use this top-level contract:

```python
REQUIRED_TOP_LEVEL = {
    "schemaVersion", "environment", "sourceCommit", "startedAt", "finishedAt",
    "productionArtifacts", "capabilityRegistry", "devices", "matrix",
    "streamBudgets", "cancellation", "busyPartial", "restarts", "optionalG4",
    "proxy", "credentials", "nurture", "package", "rollback", "cleanup",
    "redaction"
}

REQUIRED_IPA_SHA = "8a24847099495ff70b998522692c43f00dd16b90f698bda6953a73f5d33002ea"
REQUIRED_MANIFEST_SHA = "e98a549af4c061556effd36424e7732219e1a6d262bcf1f259279975024b6e1a"
```

Every device has only `udidSha256`, product/iOS/TikTok/Agent/transport/geometry tuple, G0 qualification ID, G2 runtime qualification ID/report hash, and match booleans. Matrix rows store target/account hashes, expected/observed content ID hashes, post kind, typed states, event timestamps, evidence hashes, and counts. The verifier recomputes aggregate counts from rows; it never trusts summary counters alone. A G0 reference identity contract without the matching G2 production-runtime entry fails before any live matrix action.

`restarts` is also closed. `afterPlanCommit` must record equal before/after request and plan digests, equal root/assignment seeds, sampled policies, Watch/action/target delays, `preparedRecipientPresent=false`, `preparedRecipientPayloadLength=0`, and `preparedRecipientEvidenceLength=0`; payload/evidence hash keys are forbidden because runtime UI selection has not happened. `afterDmPreparation` records a typed live-readiness disposition plus equal before/after SHA-256 and byte lengths for both canonical prepared payload and prepared evidence, equal nonzero preparation revisions, `recipientSelectionCount=1`, `payloadReplaceCount=0`, `evidenceReplaceCount=0`, `effectIntentCount=0`, and `effectTapCount=0`. The mandatory fixture executes the full PASS shape. Live `READY_BOTH` must execute that same shape, while live `NOT_READY`/`NOT_READY_FLEET` forbids byte/hash/revision keys and requires every navigation/preparation/intent/tap count to be zero. The verifier rejects a missing preparation crash fixture, any prepared payload at plan commit, any byte/hash/revision drift after DM preparation, and every resample or replacement attempt.

Budget 2 accepts only `PASS` or `NOT_QUALIFIED_CLEAN`. `PASS` requires observed overlap on two different hashed devices and `maxRunningProducers == 2`; clean non-qualification requires max `<=1`, zero overrun, and complete cleanup. Any `> configuredLimit`, stale-generation publication, control/session fault, reader/relay/process leak, or port still open fails G5.

For each optional G4 action, compare the report's exact two live tuples against the loaded capability-registry snapshot. `READY_BOTH` requires the fixed fleet case; `NOT_READY` requires zero action attempts. A one-device-ready action is `NOT_READY_FLEET` and remains disabled for this fleet report. `nurture` requires an exit-0 summary, at least one processed video, trace/frame-set hashes, session-before-stream, and cleanup. `rollback` requires registry restoration, an N-1 boot report, additive database-history preservation, and unchanged production artifact hashes.

- [ ] **Step 4: Implement raw-byte/decoded-leaf redaction, target staging, and atomic publish**

Reject duplicate JSON keys with `object_pairs_hook`, scan raw bytes before decoding, then recursively scan every string leaf. Implement `attach-nurture` as an atomic rewrite that accepts the ignored Nurture summary/trace/frame directory, validates exit and lifecycle evidence, retains only counts and SHA-256 values, and deletes no input until its replacement report fsyncs. Implement `preflight` to require every non-rollback section while returning `PENDING_ROLLBACK` and writing no docs. Implement `drill-revocation` against a temporary registry only, and `finish-rollback` to compare registry bytes, N-1 boot evidence, pre/post copied-database row identities, and fixed artifact hashes. Implement `finalize` to bind exactly one PASS rollback report to the same source/artifact/registry/database hashes. `stage` accepts only an ignored `target/interaction-gate5/` output directory, writes/fsyncs sanitized JSON and Markdown there, and never writes `docs/re`; `render-readme` derives a staged README from the reviewed repository template without modifying that template. `verify-staged` re-evaluates the merged report, registry, exact staged bytes, and redaction after the final regression.

`publish-staged` handles `gate-5.json`, `gate-5.md`, and `README.md` as one recoverable transaction. Before touching destinations it re-evaluates the finalized input, current registry/source/artifact bindings, and exact staged hashes. It snapshots each old destination as bytes or an explicit absence marker, writes/fsyncs same-filesystem incoming files, replaces all three, fsyncs the directory, then verifies destination bytes against staging. Any replacement or post-replacement verification failure restores every old byte, removes destinations previously absent, fsyncs again, verifies restoration, and removes incoming files. It leaves a target-side rollback transaction until `seal-publication` runs after commit; `rollback-publication` restores the old trio if any later pre-commit check or commit fails. None of these commands writes the production registry, and no command before `publish-staged` changes a path under `docs/re/interaction-gate5/`.

The CLI is closed to these exact subcommands; tests invoke every parser branch and reject aliases or unknown flags:

```text
baseline
check-section
check-package
attach-nurture
merge
preflight
check-registry-unchanged
drill-revocation
finish-rollback
finalize
stage
render-readme
verify-staged
publish-staged
rollback-publication
seal-publication
verify-published
```

```python
EVIDENCE_NAMES = ("gate-5.json", "gate-5.md", "README.md")

def publish_triplet_with_rollback(staging: Path, output_dir: Path, transaction: Path) -> None:
    old = snapshot_destinations_to_transaction(output_dir, transaction, EVIDENCE_NAMES)
    try:
        replace_and_fsync_all(staging, output_dir, EVIDENCE_NAMES)
        verify_exact_staged_bytes(staging, output_dir, EVIDENCE_NAMES)
    except BaseException:
        restore_bytes_or_remove_absent(old, output_dir)
        fsync_directory(output_dir)
        verify_restored_bytes_or_absence(old, output_dir)
        raise
```

- [ ] **Step 5: Run GREEN and commit**

```powershell
python -m unittest discover -s tools/interaction-gate5 -p "test_verify_report.py" -v
git add tools/interaction-gate5/verify_report.py tools/interaction-gate5/test_verify_report.py docs/re/interaction-gate5/README.md
git diff --cached --name-only
git commit -m "test(interaction): define fleet acceptance report"
```

---

### Task 2: Add A Secret-Free Ordered Acceptance Audit

**Files:**
- Create: `crates/core/src/interaction/acceptance_audit.rs`
- Modify: `crates/core/Cargo.toml`
- Modify: `crates/core/src/interaction/mod.rs`
- Modify: `crates/core/src/interaction/dispatcher.rs`
- Modify: `crates/core/src/interaction/store.rs`
- Modify: `crates/core/src/interaction/artifacts.rs`
- Modify: `crates/core/src/device_control.rs`
- Modify: `crates/core/src/interaction/progress.rs`
- Modify: `crates/core/src/interaction/device_batch_executor.rs`
- Modify: `crates/core/src/tiktok_actions/identity.rs`
- Modify: `crates/core/src/tiktok_actions/verified.rs`
- Modify: `crates/core/src/tiktok_actions/comment.rs`
- Modify: `apps/desktop/src-tauri/Cargo.toml`
- Create: `crates/core/tests/interaction_fleet_acceptance.rs`

- [ ] **Step 1: Write failing ordering and redaction tests**

```rust
#[test]
fn audit_rejects_effect_tap_before_intent() {
    let audit = AcceptanceAudit::new_in_memory();
    let action = ActionRunId::new_v4();
    audit.record(AuditEvent::EffectTap {
        action_id: action,
        ordinal: 1,
        at: Utc::now(),
    }).unwrap();
    let error = audit.finish().unwrap_err();
    assert_eq!(error.code(), "effect_tap_without_intent");
}

#[test]
fn audit_rejects_comment_as_a_desired_state_tap() {
    let audit = AcceptanceAudit::new_in_memory();
    let action = ActionRunId::new_v4();
    audit.record(AuditEvent::DesiredStateTap {
        action_id: action,
        kind: PlannedActionKind::Comment,
        ordinal: 1,
        at: Utc::now(),
    }).unwrap();
    assert_eq!(audit.finish().unwrap_err().code(), "non_idempotent_desired_state_tap");
}

#[test]
fn audit_serialization_contains_hashes_not_sensitive_values() {
    let report = fixture_audit().sanitized_report().unwrap();
    let json = serde_json::to_string(&report).unwrap();
    assert!(json.contains("udidSha256"));
    assert!(!json.contains("fixture-raw-udid"));
    assert!(!json.contains("fixture comment text"));
}
```

Test one owner per UDID, producer reserve/start/stop/generation order, session-before-stream, plan commit before preparation, resolved-recipient preparation before Direct Message intent, identity intent-before-Copy-Link, prepared-comment/intent-before-Send, cancellation before next action, and worker-process witness events. Reject duplicate tap ordinals and non-monotonic timestamps.

- [ ] **Step 2: Run RED**

```powershell
cargo test -p riviu-core --features interaction-acceptance-audit --test interaction_fleet_acceptance audit -- --nocapture
```

Expected: FAIL because `riviu-core` has no `interaction-acceptance-audit` feature and the audit module/types do not exist.

- [ ] **Step 3: Implement a feature-gated audit port**

Add the feature at both manifests so the acceptance binaries forward one exact feature into core:

In `crates/core/Cargo.toml`, add:

```toml
[features]
default = []
interaction-acceptance-audit = []
```

In `apps/desktop/src-tauri/Cargo.toml`, replace the existing two-line feature table with:

```toml
[features]
default = []
interaction-acceptance-audit = ["riviu-core/interaction-acceptance-audit"]
```

```rust
pub trait AcceptanceAuditSink: Send + Sync {
    fn record(&self, event: AuditEvent) -> Result<(), AcceptanceAuditError>;
}

#[derive(Clone)]
pub enum AuditEvent {
    OwnerAcquired { udid_hash: String, owner: DeviceWorkOwner, at: DateTime<Utc> },
    ProducerState { udid_hash: String, generation: u64, state: AuditedProducerState, at: DateTime<Utc> },
    SessionReady { udid_hash: String, session_hash: String, at: DateTime<Utc> },
    PlanCommitted { campaign_id: CampaignId, request_sha256: String, plan_sha256: String, at: DateTime<Utc> },
    ActionPrepared {
        action_id: ActionRunId,
        payload_sha256: String,
        payload_length: u64,
        evidence_sha256: Option<String>,
        preparation_revision: i64,
        resolved_recipient: bool,
        at: DateTime<Utc>,
    },
    IdentityIntent { assignment_id: AssignmentId, at: DateTime<Utc> },
    CopyLinkTap { assignment_id: AssignmentId, ordinal: u32, at: DateTime<Utc> },
    EffectIntent { action_id: ActionRunId, at: DateTime<Utc> },
    DesiredStateTap { action_id: ActionRunId, kind: PlannedActionKind, ordinal: u32, at: DateTime<Utc> },
    EffectTap { action_id: ActionRunId, ordinal: u32, at: DateTime<Utc> },
    WorkerWitness { worker_hash: String, phase: WorkerPhase, at: DateTime<Utc> },
}
```

Compile the concrete recorder only under `#[cfg(any(test, feature = "interaction-acceptance-audit"))]`; keep `AcceptanceAuditSink` and `NoopAcceptanceAudit` available to the normal composition without recording. The desktop feature forwards only to `riviu-core`, and production desktop composition always injects `NoopAcceptanceAudit`; normal events/logs do not gain this detail. The sink receives already hashed device/session/worker IDs and typed ordinals. It has no method accepting headers, URLs, clipboard bytes, comment text, proxy data, or arbitrary error strings.

Instrument existing boundaries by dependency injection, not by duplicating them: `DeviceControlPlane` transition hooks emit owner/producer/session events; `dispatcher.rs` emits synchronously after `commit_plan` returns and before dispatch can advance; `InteractionProgress` emits after durable prepared-payload, identity-intent, and effect-intent commits; `device_batch_executor.rs` passes the sink into the existing identity and action facades; `identity.rs` emits `CopyLinkTap`, `verified.rs` emits `DesiredStateTap` for Like/Follow, and `comment.rs` emits `EffectTap` immediately before the real Send tap. `ActionPrepared` receives only canonical payload length/SHA-256, the referenced artifact SHA-256 already present in the typed `EvidenceRef`, revision, and `resolved_recipient`; it never receives recipient text or raw payload/evidence bytes. Under `interaction-acceptance-audit` only, `InteractionStore` exposes a read-only raw `prepared_payload_json` byte accessor and `ArtifactStore` exposes a containment/checksum-validating evidence-byte accessor; neither accessor logs, serializes, or returns those bytes to the parent/report. The restarted worker uses them for in-process byte equality before producing only hashes/lengths. `EffectTap` always requires a preceding durable `EffectIntent`; idempotent desired-state taps are instead cross-checked against G2/G4 frame evidence and never fabricate an effect intent. Any G4 facade present later consumes the same split through `device_batch_executor.rs`: Save uses `DesiredStateTap`, while Repost/Direct Message use `EffectTap`; absent G4 source remains disabled and needs no conditional file edit. Audit failure aborts before a not-yet-issued gesture; after issued intent it yields typed Uncertain and never replays.

- [ ] **Step 4: Run GREEN and commit**

```powershell
cargo test -p riviu-core --features interaction-acceptance-audit --test interaction_fleet_acceptance audit -- --nocapture
git add crates/core/Cargo.toml crates/core/src/interaction/acceptance_audit.rs crates/core/src/interaction/mod.rs crates/core/src/interaction/dispatcher.rs crates/core/src/interaction/store.rs crates/core/src/interaction/artifacts.rs crates/core/src/device_control.rs crates/core/src/interaction/progress.rs crates/core/src/interaction/device_batch_executor.rs crates/core/src/tiktok_actions/identity.rs crates/core/src/tiktok_actions/verified.rs crates/core/src/tiktok_actions/comment.rs crates/core/tests/interaction_fleet_acceptance.rs apps/desktop/src-tauri/Cargo.toml
git diff --cached --name-only
git commit -m "test(core): audit interaction side-effect ordering"
```

---

### Task 3: Build The Fixed Parent And Worker Harness

**Files:**
- Create: `apps/desktop/src-tauri/src/bin/live_interaction_fleet_acceptance.rs`
- Create: `apps/desktop/src-tauri/src/bin/interaction_acceptance_worker.rs`
- Modify: `crates/core/tests/interaction_fleet_acceptance.rs`

- [ ] **Step 1: Write failing fixture orchestration tests**

Use mock devices, a temporary SQLite database, paused time, deterministic target resolver, current G0 `DeviceControlPlane`, and the real G1 dispatcher/recovery reducers. Assert the exact All/RoundRobin rows, budget-1/budget-2 accounting, partial behavior, and crash state transitions.

```rust
#[tokio::test(start_paused = true)]
async fn budget_one_all_uses_both_actors_without_two_producers() {
    let fixture = fleet_fixture(1).with_two_devices().with_six_targets();
    let report = fixture.run_all().await.unwrap();
    assert_eq!(report.assignments.len(), 12);
    assert_eq!(report.distinct_actor_count(), 2);
    assert_eq!(report.max_running_producers, 1);
    assert_eq!(report.producer_overruns, 0);
}

#[tokio::test]
async fn busy_actor_is_not_redistributed() {
    let fixture = fleet_fixture(1).with_manual_owner("device-a").await;
    let report = fixture.run_busy_partial().await.unwrap();
    assert_eq!(report.actor("device-a").status, ActorRunStatus::SkippedUnavailable);
    assert_eq!(report.actor("device-b").status, ActorRunStatus::Succeeded);
    assert_eq!(report.redistributed_assignments, 0);
    assert_eq!(report.campaign_status, CampaignStatus::Partial);
}

#[test]
fn command_parser_exposes_only_the_fixed_gate_sections() {
    assert!(matches!(parse_command(args_for_inspect()), Ok(HarnessCommand::Inspect(_))));
    assert!(matches!(parse_command(args_for_base()), Ok(HarnessCommand::Base(_))));
    assert!(parse_command(["lower-count", "1"]).is_err());
}
```

- [ ] **Step 2: Run RED**

```powershell
cargo test -p riviu-core --features interaction-acceptance-audit --test interaction_fleet_acceptance harness -- --nocapture
cargo test -p riviu-managers-phone --features interaction-acceptance-audit --bin live_interaction_fleet_acceptance -- --nocapture
```

- [ ] **Step 3: Define fixed subcommands and the external fixture contract**

The first argument is exactly one of `inspect`, `base`, `faults`, or `integrations`; a hand-written parser follows the existing harness style and rejects missing, duplicate, or unknown flags. `inspect` accepts only `--udid`, `--registry`, `--ipa`, `--agent-manifest`, and `--output`. The three fleet commands accept only `--udid-a`, `--udid-b`, `--fixture-matrix`, `--work-dir`, and `--output`, and reject equal UDIDs. No subcommand accepts counts, thresholds, timeout relaxation, action skipping, device reduction, or stream-budget overrides. Tokens and AI keys are read only from `RIVIU_RTMMO_TOKEN` and `RIVIU_AI_API_KEY` at the binary boundary.

```rust
enum HarnessCommand {
    Inspect(InspectArgs),
    Base(FleetRunArgs),
    Faults(FleetRunArgs),
    Integrations(FleetRunArgs),
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase")]
enum WorkerMode {
    Normal,
    ExitAfterPlanCommit,
    ExitAfterDmRecipientPrepared,
    ExitAfterCommentEffectTap,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum WorkerPhase {
    PlanCommitted,
    DmRecipientPrepared,
    CommentEffectTapForwarded,
}

struct InspectArgs {
    udid: String,
    registry: PathBuf,
    ipa: PathBuf,
    agent_manifest: PathBuf,
    output: PathBuf,
}

struct FleetRunArgs {
    udid_a: String,
    udid_b: String,
    fixture_matrix: PathBuf,
    work_dir: PathBuf,
    output: PathBuf,
}
```

The fixture file used by fleet commands must contain exactly two entries for each identity kind, reset metadata for Like/Follow, and Unicode Comment inputs; it stays outside Git.

```rust
#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct FleetFixtureMatrix {
    direct_videos: [IdentityFixture; TARGETS_PER_KIND],
    photos: [IdentityFixture; TARGETS_PER_KIND],
    short_links: [IdentityFixture; TARGETS_PER_KIND],
    like_transitions: [ResettableFixture; REQUIRED_DEVICE_COUNT],
    follow_transitions: [ResettableFixture; REQUIRED_DEVICE_COUNT],
    unicode_comments: [[CommentFixture; UNICODE_COMMENTS_PER_DEVICE]; REQUIRED_DEVICE_COUNT],
}
```

Every fixture carries raw URL/account/reset data only in the input process. The parent validates distinct expected content IDs, hashes sensitive fields immediately for reporting, and passes raw values to a worker through inherited anonymous stdin, never argv or a temporary JSON file. Worker stdout is framed canonical JSON audit data; stderr is guarded and redacted before retention.

- [ ] **Step 4: Implement parent-owned lifecycle and matrix execution**

The parent installs `riviu_ios_driver::install_process_tree_guard()` before any child on Windows and creates a new process group plus explicit child registry ownership on macOS. `inspect` verifies source artifact/manifest/registry hashes, acquires `DeviceWorkOwner::Interaction`, calls only `DeviceControlPlane::inspect_interaction_device`, emits a sanitized `DeviceCapabilitySnapshot`, releases ownership, and exits; it never repairs, installs, launches, creates a session, starts MJPEG, or executes a gesture. Each fleet command verifies the same source artifacts/registry, preflights both devices through that identical inspect function, then runs cases one at a time except the budget-2 overlap case.

Use the existing engine APIs only:

```rust
struct FleetHarness {
    control: Arc<DeviceControlPlane>,
    service: Arc<DesktopInteractionService>,
    audit: Arc<dyn AcceptanceAuditSink>,
    workers: WorkerRegistry,
}

impl FleetHarness {
    async fn run_campaign(&self, request: InteractionCampaignRequest) -> anyhow::Result<CampaignSummary>;
    async fn await_terminal(&self, id: CampaignId, deadline: Duration) -> anyhow::Result<CampaignDetail>;
    async fn prove_cleanup(&self, udids: [&str; 2]) -> anyhow::Result<CleanupEvidence>;
}
```

The parent never calls `DeviceDriver`, WDA, a stream relay, or SQLite mutations directly. It submits G1 commands, observes paged state, and uses G0 inspection/cleanup proof. It snapshots every action capability before constructing required requests; missing G0 contracts, missing/mismatched G2 `interaction_runtime`, or a missing required G2 side-effect capability fails before device action. For G4, absence records `NOT_READY` with zero attempts.

- [ ] **Step 5: Implement worker modes without weakening production state transitions**

`interaction_acceptance_worker` accepts a single inherited stdin envelope with mode `normal`, `exitAfterPlanCommit`, `exitAfterDmRecipientPrepared`, or `exitAfterCommentEffectTap`. It uses the production composition root and audit sink. The three crash modes call `std::process::abort()` only after the audit sink has fsynced the named witness; they do not catch errors, fabricate terminal state, or kill a device process. `DmRecipientPrepared` is emitted only after the canonical resolved-recipient payload and its preparation evidence are durably committed, and strictly before an effect-intent row exists.

Before campaign execution, the acceptance sink arms one exact one-shot barrier for the selected phase. The engine task records and fsyncs the matching audit event, then blocks inside that hook without returning to the next state transition; `wait_for_witness` releases only the worker control task, which verifies the forbidden next row/count is absent and aborts the process. Therefore `PlanCommitted` cannot race `action_prepared`, and resolved `ActionPrepared` cannot race `issue_effect_intent`. `CommentEffectTapForwarded` uses the corresponding post-driver-forward hook so the real tap count is already one before its barrier fires. These barriers compile only with `interaction-acceptance-audit` and never alter normal production composition.

```rust
match envelope.mode {
    WorkerMode::Normal => run_claimed_campaign(envelope).await?,
    WorkerMode::ExitAfterPlanCommit => {
        wait_for_witness(WorkerPhase::PlanCommitted).await?;
        std::process::abort();
    }
    WorkerMode::ExitAfterDmRecipientPrepared => {
        wait_for_witness(WorkerPhase::DmRecipientPrepared).await?;
        assert_no_effect_intent().await?;
        std::process::abort();
    }
    WorkerMode::ExitAfterCommentEffectTap => {
        wait_for_witness(WorkerPhase::CommentEffectTapForwarded).await?;
        std::process::abort();
    }
}
```

The parent records old PID disappearance, closes/reclaims exact children/transports through the existing registry, starts a new worker against the same database, runs `recover_startup`, and queries final state. It never edits rows to simulate a crash outcome.

- [ ] **Step 6: Run fixture GREEN and commit**

```powershell
cargo test -p riviu-core --features interaction-acceptance-audit --test interaction_fleet_acceptance -- --nocapture
cargo test -p riviu-managers-phone --features interaction-acceptance-audit --bin live_interaction_fleet_acceptance -- --nocapture
cargo test -p riviu-managers-phone --features interaction-acceptance-audit --bin interaction_acceptance_worker -- --nocapture
git add apps/desktop/src-tauri/src/bin/live_interaction_fleet_acceptance.rs apps/desktop/src-tauri/src/bin/interaction_acceptance_worker.rs crates/core/tests/interaction_fleet_acceptance.rs
git diff --cached --name-only
git commit -m "test(interaction): add fixed two-device fleet harness"
```

---

### Task 4: Prove Cancellation, Crash Recovery, And No Replay In Fixtures

**Files:**
- Modify: `apps/desktop/src-tauri/src/bin/live_interaction_fleet_acceptance.rs`
- Modify: `apps/desktop/src-tauri/src/bin/interaction_acceptance_worker.rs`
- Modify: `crates/core/tests/interaction_fleet_acceptance.rs`
- Modify: `tools/interaction-gate5/test_verify_report.py`

- [ ] **Step 1: Write failing deterministic fault tests**

```rust
#[tokio::test(start_paused = true)]
async fn cancellation_during_persisted_delay_opens_no_next_action() {
    let fixture = Arc::new(fleet_fixture(1).with_action_delay_ms(10_000));
    let worker_fixture = Arc::clone(&fixture);
    let run = tokio::spawn(async move { worker_fixture.run_cancellation_case().await });
    tokio::time::advance(Duration::from_millis(500)).await;
    fixture.request_cancel().await.unwrap();
    tokio::time::advance(Duration::from_secs(1)).await;
    let report = run.await.unwrap().unwrap();
    assert_eq!(report.effect_taps_after_cancel, 0);
    assert_eq!(report.next_target_open_count, 0);
}

#[tokio::test]
async fn restart_after_plan_commit_preserves_plan_without_dm_preparation() {
    let fixture = crash_fixture(WorkerMode::ExitAfterPlanCommit).await;
    fixture.run_worker_and_observe_abort().await.unwrap();
    let before = fixture.load_immutable_plan_material().await.unwrap();
    let recovered = fixture.restart_and_recover().await.unwrap();
    let after = fixture.load_immutable_plan_material().await.unwrap();
    assert_eq!(before, after); // seeds, policies, Watch/action/target timing
    assert_eq!(recovered.prepared_recipient_payload_count, 0);
    assert_eq!(recovered.prepared_recipient_evidence_count, 0);
}

#[tokio::test]
async fn restart_after_real_effect_tap_never_replays_comment() {
    let fixture = crash_fixture(WorkerMode::ExitAfterCommentEffectTap).await;
    fixture.run_worker_and_observe_abort().await.unwrap();
    let recovered = fixture.restart_and_recover().await.unwrap();
    assert_eq!(recovered.action_status, ActionStatus::Uncertain);
    assert_eq!(recovered.effect_tap_count, 1);
    assert_eq!(recovered.replay_count, 0);
}

#[tokio::test]
async fn restart_after_dm_preparation_reloads_exact_bytes_without_resampling() {
    let fixture = crash_fixture(WorkerMode::ExitAfterDmRecipientPrepared).await;
    fixture.run_worker_and_observe_abort().await.unwrap();
    let before = fixture.read_durable_dm_preparation_bytes().await.unwrap();
    let recovered = fixture.restart_and_recover().await.unwrap();
    let after = fixture.read_durable_dm_preparation_bytes().await.unwrap();
    assert_eq!(before.payload, after.payload);
    assert_eq!(before.evidence, after.evidence);
    assert_eq!(recovered.recipient_selection_count, 1);
    assert_eq!(recovered.payload_replace_count, 0);
    assert_eq!(recovered.effect_intent_count, 0);
}
```

Add pre-plan cancellation, cancellation after issued intent, lost dispatch heartbeat, restart after plan commit, restart after durable DM preparation, restart after identity intent with zero/one observed Copy Link tap, and cleanup-worker cancellation. Assert that only the named harness modes may abort. The plan-commit fixture includes a selected Direct Message action with persisted policy/assignment seed but no prepared payload. The DM preparation crash uses a deterministic visible-recipient fixture in every source-gate run; the live harness executes the same case only when Direct Message is Ready on both exact tuples.

- [ ] **Step 2: Run RED**

```powershell
cargo test -p riviu-core --features interaction-acceptance-audit --test interaction_fleet_acceptance cancellation -- --nocapture
cargo test -p riviu-core --features interaction-acceptance-audit --test interaction_fleet_acceptance restart -- --nocapture
```

- [ ] **Step 3: Implement bounded cancellation observation**

Use G2's persisted-delay cancellation poller and G1 cancellation command unchanged. The harness records the committed cancel revision, last action event before cancel, first terminal aggregate after cancel, and count of subsequent open/effect events. It does not use timing alone as proof.

`exitAfterPlanCommit` must recover to the same campaign/request hash, root planner seed, assignment seeds, sampled policies, Watch duration, action delays, and target delays. The planned Direct Message action row already exists with `prepared_payload_json=NULL`; prove that no resolved-recipient payload/evidence or preparation revision exists at either side of recovery. Recipient choice belongs to the first durable runtime preparation, not immutable plan construction. No second plan, assignment, or action row is legal.

`exitAfterDmRecipientPrepared` snapshots the exact canonical payload bytes from the durable row and the exact recipient-evidence artifact bytes referenced by that payload, restarts through `recover_startup`, reloads both without calling recipient discovery, and compares the byte arrays before hashing/redacting them for the report. Recovery freezes this non-issued Direct Message as `Interrupted`; it must leave the preparation revision unchanged, make zero CAS replacement attempts, perform no second recipient selection, create no effect intent, and forward no tap. `exitAfterCommentEffectTap` must recover issued intent to `Uncertain`, freeze later work as `Interrupted`, and reject Retry Failed for that assignment.

- [ ] **Step 4: Add verifier mutations for every forbidden replay**

```python
def test_restart_plan_must_reuse_all_sampled_values(self):
    report = passing_report()
    report["restarts"]["afterPlanCommit"]["beforeDigest"] = "a" * 64
    report["restarts"]["afterPlanCommit"]["afterDigest"] = "b" * 64
    self.assertIn("plan_resampled", verify_report.evaluate(report)["failures"])

def test_restart_plan_must_not_prepare_a_recipient(self):
    report = passing_report()
    crash = report["restarts"]["afterPlanCommit"]
    crash["preparedRecipientPresent"] = True
    crash["preparedRecipientEvidenceLength"] = 8
    self.assertIn("recipient_prepared_at_plan_commit", verify_report.evaluate(report)["failures"])

def test_restart_dm_preparation_must_preserve_payload_and_evidence(self):
    report = passing_report()
    crash = report["restarts"]["afterDmPreparation"]
    crash["afterEvidenceSha256"] = "f" * 64
    crash["payloadReplaceCount"] = 1
    crash["afterPreparationRevision"] += 1
    failures = verify_report.evaluate(report)["failures"]
    self.assertIn("dm_preparation_replaced", failures)
    self.assertIn("dm_preparation_rewrite_attempted", failures)

def test_cancelled_case_must_have_zero_post_cancel_effects(self):
    report = passing_report()
    report["cancellation"]["effectTapsAfterCancel"] = 1
    self.assertIn("effect_after_cancel", verify_report.evaluate(report)["failures"])
```

- [ ] **Step 5: Run GREEN and commit**

```powershell
cargo test -p riviu-core --features interaction-acceptance-audit --test interaction_fleet_acceptance cancellation -- --nocapture
cargo test -p riviu-core --features interaction-acceptance-audit --test interaction_fleet_acceptance restart -- --nocapture
python -m unittest discover -s tools/interaction-gate5 -p "test_verify_report.py" -v
git add apps/desktop/src-tauri/src/bin/live_interaction_fleet_acceptance.rs apps/desktop/src-tauri/src/bin/interaction_acceptance_worker.rs crates/core/tests/interaction_fleet_acceptance.rs tools/interaction-gate5/test_verify_report.py
git diff --cached --name-only
git commit -m "test(interaction): prove cancellation and no replay"
```

---

### Task 5: Build The Packaged Desktop Smoke Probe

**Files:**
- Create: `tools/interaction-gate5/package_smoke.py`
- Create: `tools/interaction-gate5/test_package_smoke.py`
- Modify: `apps/desktop/e2e/interaction-workflow.spec.ts`

- [ ] **Step 1: Write failing package-layout and report tests**

```python
class PackageSmokeTests(unittest.TestCase):
    def test_embedded_production_artifacts_match_source(self):
        fixture = packaged_app_fixture()
        result = package_smoke.inspect_bundle(fixture.app, fixture.repo)
        self.assertEqual(REQUIRED_IPA_SHA, result["embeddedIpaSha256"])
        self.assertEqual(REQUIRED_MANIFEST_SHA, result["embeddedManifestSha256"])

    def test_nonblank_window_capture_is_required(self):
        report = passing_package_report()
        report["screenshot"]["nonBackgroundPixelRatio"] = 0.0
        self.assertIn("blank_window", package_smoke.evaluate(report)["failures"])
```

Test missing executable, missing sidecar/resource, mismatched embedded resource, early process exit, no window, blank capture, Interaction button absent, panel overflow, crash log, token in argv/log, and cleanup failure.

- [ ] **Step 2: Run RED**

```powershell
python -m unittest discover -s tools/interaction-gate5 -p "test_package_smoke.py" -v
```

- [ ] **Step 3: Implement deterministic bundle inspection**

On macOS require the exact bundle `target/release/bundle/macos/Riviumanagersphone.app`, executable `Contents/MacOS/Riviumanagersphone`, and packaged resources under `Contents/Resources/sidecars`. Recompute hashes from source and bundle bytes; codesign verification and package hash are reported separately.

```python
def inspect_bundle(app: Path, repo: Path) -> dict:
    executable = app / "Contents/MacOS/Riviumanagersphone"
    resources = app / "Contents/Resources/sidecars/wda"
    return {
        "bundleSha256": hash_tree(app),
        "executableSha256": sha256_file(executable),
        "embeddedIpaSha256": sha256_file(resources / "RiviuAgent.ipa"),
        "embeddedManifestSha256": sha256_file(resources / "agent-manifest.json"),
        "sourceIpaMatch": files_equal(resources / "RiviuAgent.ipa", repo / "sidecars/wda/RiviuAgent.ipa"),
        "sourceManifestMatch": files_equal(resources / "agent-manifest.json", repo / "sidecars/wda/agent-manifest.json"),
    }
```

- [ ] **Step 4: Implement live launch and UI capture without a production smoke route**

The CLI accepts exactly `run` and `boot-only`. `run` requires `--app`, `--repo-root`, and `--output` and performs bundle inspection plus full window/UI capture. `boot-only` requires `--app` and `--output`, uses the caller's isolated `HOME`, proves bounded startup/database compatibility and cleanup, and does not require the Interaction button because the N-1 bundle predates G3. Both modes reject all unknown flags.

Launch the bundle executable directly as an owned child so the environment and PID are deterministic. Do not delegate launch to another app launcher, and do not add a Tauri command, URL route, build flag, or production environment branch for smoke mode.

```python
executable = app / "Contents/MacOS/Riviumanagersphone"
launch_cwd = repo if repo is not None else app.parent
environment = os.environ.copy()
for secret_name in ("RIVIU_RTMMO_TOKEN", "RIVIU_AI_API_KEY", "RIVIU_AGENT_TOKEN"):
    environment.pop(secret_name, None)
environment["RIVIU_MOCK_DEVICES"] = "1"
process = subprocess.Popen(
    [str(executable)],
    cwd=str(launch_cwd),
    env=environment,
    stdin=subprocess.DEVNULL,
    stdout=subprocess.PIPE,
    stderr=subprocess.PIPE,
    start_new_session=True,
)
```

Drain both pipes on dedicated bounded readers, redact before retaining diagnostics, and terminate the exact process group during cleanup. Use `osascript` System Events to wait for the `Riviumanagersphone` window, obtain bounds, click the visible `Tương tác` toolbar button, and verify the panel text through accessibility. Capture that exact window rectangle with `/usr/sbin/screencapture -x -R`. Pillow requires real dimensions, at least 2% non-background pixels, and no uniform/transparent image. The script stores only capture SHA/dimensions/pixel ratio in the publishable report and deletes the PNG during cleanup. If macOS Accessibility or Screen Recording permission is missing, return `PENDING_MAC_PERMISSION`, never PASS.

- [ ] **Step 5: Extend the G3 browser gate for packaged-release states**

Add Playwright assertions for the production-minimum 1100x700 viewport, G2 Ready/G4 Deferred capabilities, two devices, 500 targets, longest typed error, and panel reopen. Verify screenshots with Playwright and keep them under ignored `apps/desktop/test-results/`.

```ts
test("release-sized workflow keeps device context visible", async ({ page }) => {
  await page.setViewportSize({ width: 1100, height: 700 });
  await page.goto("/interaction-harness.html?fixture=release");
  await page.getByRole("button", { name: "Tương tác" }).click();
  await expect(page.locator(".interaction-panel")).toBeVisible();
  await expect(page.locator(".fixture-device-grid")).toBeVisible();
  expect(await page.locator(".interaction-panel").evaluate(
    (node) => node.scrollWidth <= node.clientWidth,
  )).toBe(true);
});
```

- [ ] **Step 6: Run fixture GREEN and commit**

```powershell
python -m unittest discover -s tools/interaction-gate5 -p "test_package_smoke.py" -v
Push-Location apps/desktop
npx playwright test e2e/interaction-workflow.spec.ts --project=chromium
Pop-Location
git add tools/interaction-gate5/package_smoke.py tools/interaction-gate5/test_package_smoke.py apps/desktop/e2e/interaction-workflow.spec.ts
git diff --cached --name-only
git commit -m "test(desktop): add packaged interaction smoke probe"
```

---

### Task 6: Run Complete Fixture And Static Release Gates

**Files:**
- Read/verify only: repository source, tests, production artifacts, and Git scope

This task makes no source edit and no commit. A failure returns to the owning implementation task for a focused fix/commit, after which Task 6 restarts from Step 1.

- [ ] **Step 1: Verify the acceptance binaries expose no threshold controls**

```powershell
$source = Get-Content apps/desktop/src-tauri/src/bin/live_interaction_fleet_acceptance.rs -Raw
$forbidden = @('--count', '--minimum', '--skip', '--no-crash', '--one-device', '--budget-limit')
foreach ($flag in $forbidden) {
  if ($source.Contains($flag)) { throw "lowerable G5 flag: $flag" }
}
```

- [ ] **Step 2: Run every fixture verifier and harness**

```powershell
python -m unittest discover -s tools/interaction-gate5 -p "test_*.py" -v
cargo test -p riviu-core --features interaction-acceptance-audit --test interaction_fleet_acceptance -- --nocapture
cargo test -p riviu-managers-phone --features interaction-acceptance-audit --bin live_interaction_fleet_acceptance -- --nocapture
cargo test -p riviu-managers-phone --features interaction-acceptance-audit --bin interaction_acceptance_worker -- --nocapture
```

Expected: fixture orchestration and all verifier branches pass, including the mandatory crash immediately after durable DM preparation; generated reports remain `FIXTURE_ONLY`.

- [ ] **Step 3: Run the complete workspace and desktop gates**

```powershell
cargo fmt --all -- --check
cargo test --workspace
cargo clippy --workspace --all-targets --all-features -- -D warnings
npm --prefix apps/desktop test -- --run
npm --prefix apps/desktop run lint
npm --prefix apps/desktop run build
Push-Location apps/desktop
npx playwright test e2e/interaction-workflow.spec.ts --project=chromium
Pop-Location
```

- [ ] **Step 4: Recompute immutable production hashes**

```powershell
$expectedIpa = '8a24847099495ff70b998522692c43f00dd16b90f698bda6953a73f5d33002ea'
$expectedManifest = 'e98a549af4c061556effd36424e7732219e1a6d262bcf1f259279975024b6e1a'
if (((Get-FileHash sidecars/wda/RiviuAgent.ipa -Algorithm SHA256).Hash.ToLowerInvariant()) -ne $expectedIpa) { throw 'production IPA changed' }
if (((Get-FileHash sidecars/wda/agent-manifest.json -Algorithm SHA256).Hash.ToLowerInvariant()) -ne $expectedManifest) { throw 'production manifest changed' }
```

- [ ] **Step 5: Inspect scope before moving to Mac**

```powershell
git status --short
git diff --check
git diff --stat
```

Expected: the isolated implementation worktree is clean after Tasks 1-5 commits, and no production Agent, stock WDA, or candidate source file changed for G5. If unrelated work exists in the shared checkout, create/use a clean isolated worktree before Task 7 instead of carrying it into live evidence.

---

### Task 7: Prepare Two Mac Devices And Capture The Immutable Baseline

**Files:**
- Create in ignored work area: `target/interaction-gate5/baseline.json`

- [ ] **Step 1: Validate required environment without printing secrets**

```bash
export PATH="$HOME/Library/Python/3.9/bin:$PATH"
: "${RIVIU_GATE5_UDID_A:?set first live UDID}"
: "${RIVIU_GATE5_UDID_B:?set second live UDID}"
: "${RIVIU_GATE5_FIXTURES:?set controlled fixture matrix path}"
: "${RIVIU_AI_API_KEY:?load AI key into the environment}"
test "$RIVIU_GATE5_UDID_A" != "$RIVIU_GATE5_UDID_B"
test -f "$RIVIU_GATE5_FIXTURES"
python3 -c 'import pymobiledevice3, PIL; assert pymobiledevice3.__version__ == "10.1.0"; assert PIL.__version__ == "11.3.0"'
```

Load `RIVIU_RTMMO_TOKEN` from the existing Keychain credential into an environment variable only for each command. Never echo it, write it into the fixture file, or append it to a shell trace.

- [ ] **Step 2: Require a clean implementation checkout and record source identity**

```bash
test -z "$(git status --porcelain)"
mkdir -p target/interaction-gate5/live
git rev-parse HEAD > target/interaction-gate5/source-commit.txt
shasum -a 256 sidecars/wda/RiviuAgent.ipa sidecars/wda/agent-manifest.json sidecars/wda/interaction-capabilities.json
```

Expected: the first two hashes equal the fixed production values. Stop if the implementation checkout is dirty; run live gates from an isolated worktree rather than hiding changes.

- [ ] **Step 3: Build the exact release harnesses used for inspection and live runs**

```bash
cargo build -p riviu-managers-phone --features interaction-acceptance-audit \
  --bin live_interaction_fleet_acceptance \
  --bin interaction_acceptance_worker --release
```

Expected: both binaries are produced from the clean source commit recorded in Step 2.

- [ ] **Step 4: Confirm both devices, trust, Developer Mode, and tuple readiness**

```bash
tidevice list
RIVIU_RTMMO_TOKEN="$(security find-generic-password -s riviu-managers-phone -a agent-auth-token -w)" \
./target/release/live_interaction_fleet_acceptance inspect \
  --udid "$RIVIU_GATE5_UDID_A" \
  --registry sidecars/wda/interaction-capabilities.json \
  --ipa sidecars/wda/RiviuAgent.ipa \
  --agent-manifest sidecars/wda/agent-manifest.json \
  --output target/interaction-gate5/device-a-inspect.json
RIVIU_RTMMO_TOKEN="$(security find-generic-password -s riviu-managers-phone -a agent-auth-token -w)" \
./target/release/live_interaction_fleet_acceptance inspect \
  --udid "$RIVIU_GATE5_UDID_B" \
  --registry sidecars/wda/interaction-capabilities.json \
  --ipa sidecars/wda/RiviuAgent.ipa \
  --agent-manifest sidecars/wda/agent-manifest.json \
  --output target/interaction-gate5/device-b-inspect.json
```

The G5 `inspect` subcommand implemented in Task 3 uses only G0's non-mutating inspection API. Both devices must be USB-connected, unlocked, trusted, Developer Mode enabled, signed into controlled TikTok fixture accounts, and matched to reviewed G0 plus G2 `interaction_runtime`/action capability entries. Different qualified tuples are allowed only when each independently exposes the runtime gate and required G2 actions; coordinates are never inherited.

- [ ] **Step 5: Remove competing owners exactly**

Close the desktop and every live harness. Then stop only known competing bundles on both devices:

```bash
for udid in "$RIVIU_GATE5_UDID_A" "$RIVIU_GATE5_UDID_B"; do
  tidevice -u "$udid" kill notes.3u || true
  tidevice -u "$udid" kill com.mrph.svc || true
  tidevice -u "$udid" kill com.riviu.managersphone.agent.xctrunner || true
done
```

Do not kill TikTok, usbmuxd, Python, tidevice, or XCTest processes by a broad process name. The harness/supervisor owns the exact processes it starts.

- [ ] **Step 6: Write the immutable baseline**

```bash
python3 tools/interaction-gate5/verify_report.py baseline \
  --source-commit "$(git rev-parse HEAD)" \
  --registry sidecars/wda/interaction-capabilities.json \
  --ipa sidecars/wda/RiviuAgent.ipa \
  --manifest sidecars/wda/agent-manifest.json \
  --device-inspect target/interaction-gate5/device-a-inspect.json \
  --device-inspect target/interaction-gate5/device-b-inspect.json \
  --output target/interaction-gate5/baseline.json
```

Expected: baseline contains only hashed device identities and exact non-secret tuples; its status is `PENDING_LIVE_MATRIX`.

---

### Task 8: Run All, RoundRobin, G2 Effects, And Stream Budgets Live

**Files:**
- Create in ignored work area: `target/interaction-gate5/live/base-report.json`

- [ ] **Step 1: Run the fixed budget-1 release matrix**

```bash
RIVIU_RTMMO_TOKEN="$(security find-generic-password -s riviu-managers-phone -a agent-auth-token -w)" \
./target/release/live_interaction_fleet_acceptance base \
  --udid-a "$RIVIU_GATE5_UDID_A" \
  --udid-b "$RIVIU_GATE5_UDID_B" \
  --fixture-matrix "$RIVIU_GATE5_FIXTURES" \
  --work-dir target/interaction-gate5/live \
  --output target/interaction-gate5/live/base-report.json
```

There is no count, action-skip, device-count, or timeout-relaxation flag. The `base` mode always runs budget-1 All, budget-1 RoundRobin, six-kind identity coverage, Like/Follow, Unicode Comments, and the budget-2 diagnostic.

- [ ] **Step 2: Inspect required budget-1 and distribution results**

```bash
python3 tools/interaction-gate5/verify_report.py check-section \
  --input target/interaction-gate5/live/base-report.json \
  --section budget1-all
python3 tools/interaction-gate5/verify_report.py check-section \
  --input target/interaction-gate5/live/base-report.json \
  --section budget1-round-robin
```

Expected: All has 12 terminal assignments and both actors; RoundRobin has six assignments split 3/3 in persisted target order; producer max is one; no actor is silently redistributed.

- [ ] **Step 3: Inspect identity and G2 action evidence**

```bash
python3 tools/interaction-gate5/verify_report.py check-section \
  --input target/interaction-gate5/live/base-report.json \
  --section identity-g2
```

Expected: direct/photo/short copied content hashes and kinds match planned targets on both devices; Like/Follow transitions have desired-state frames; four Unicode Comments have stored payload, armed Send, intent-before-tap, sent evidence, and no raw text in the report.

- [ ] **Step 4: Classify budget 2 without weakening budget 1**

```bash
python3 tools/interaction-gate5/verify_report.py check-section \
  --input target/interaction-gate5/live/base-report.json \
  --section budget2
```

If PASS, report `qualifiedStreamBudget=2` for this exact host/device/USB topology evidence but retain product default `1`. If `NOT_QUALIFIED_CLEAN`, keep qualified budget `1` and continue. Any overrun, stale frame publication, transport/control fault, dirty cleanup, or failure to return both devices to idle stops G5 and must be repaired before rerunning the complete base matrix.

- [ ] **Step 5: Confirm section cleanup before fault cases**

For both hashed devices, the report must show watcher stopped, stream reader stopped, current generation invalidated, relay/proxy children terminated, control and MJPEG device ports closed, device owner absent, producer capacity zero, and no Riviu-owned clipboard sentinel. A successful Copy Link target URL may remain in clipboard; the report records only sentinel ownership/match booleans and hashes.

---

### Task 9: Run Cancellation, Busy Partial, And Crash Recovery Live

**Files:**
- Create in ignored work area: `target/interaction-gate5/live/fault-report.json`

- [ ] **Step 1: Run the fixed fault matrix**

```bash
RIVIU_RTMMO_TOKEN="$(security find-generic-password -s riviu-managers-phone -a agent-auth-token -w)" \
./target/release/live_interaction_fleet_acceptance faults \
  --udid-a "$RIVIU_GATE5_UDID_A" \
  --udid-b "$RIVIU_GATE5_UDID_B" \
  --fixture-matrix "$RIVIU_GATE5_FIXTURES" \
  --work-dir target/interaction-gate5/live \
  --output target/interaction-gate5/live/fault-report.json
```

The `faults` mode always runs one persisted-delay cancellation, one held-ManualControl partial campaign, one worker abort after plan commit, and one worker abort immediately after forwarding a real Comment Send tap. It also evaluates the Direct Message preparation crash: `READY_BOTH` runs it against each qualified tuple; `NOT_READY` or `NOT_READY_FLEET` records the matching registry state and zero navigation/preparation/effect attempts. The mandatory fixture gate has already run this third crash branch regardless of live readiness.

- [ ] **Step 2: Verify cancellation and Busy semantics**

```bash
python3 tools/interaction-gate5/verify_report.py check-section \
  --input target/interaction-gate5/live/fault-report.json \
  --section cancellation-busy
```

Expected: cancellation is durably requested during the stored delay, the pending action becomes `Skipped/CancelledBeforeStart`, no next target/effect starts, and aggregate terminal state is correct. The busy actor reports current `ManualControl`, zero navigation, `SkippedUnavailable`, no redistribution; the other actor succeeds and campaign is Partial.

- [ ] **Step 3: Verify plan restart and immutable sampling**

```bash
python3 tools/interaction-gate5/verify_report.py check-section \
  --input target/interaction-gate5/live/fault-report.json \
  --section restart-plan
```

Expected: old worker PID disappears; exact owned child fingerprints are reclaimed; the same request/plan digest, root seed, assignment seeds, sampled policies, Watch/action/target timing load once; `preparedRecipientPresent=false`, prepared payload/evidence lengths are zero, their hashes are absent, and no duplicate plan/assignment/action row exists.

- [ ] **Step 4: Verify durable Direct Message preparation recovery**

```bash
python3 tools/interaction-gate5/verify_report.py check-section \
  --input target/interaction-gate5/live/fault-report.json \
  --section restart-dm-prepared
```

Expected for `READY_BOTH`: each worker abort occurs after one durable resolved-recipient preparation and before effect intent; restart reads the same preparation revision and exact payload/evidence bytes, whose before/after SHA-256 and lengths match; recipient selection count is 1, replacement counts are 0, status is `Interrupted`, and intent/tap counts are 0. For `NOT_READY`/`NOT_READY_FLEET`, the registry disposition matches both exact tuples and every preparation/navigation/effect count is 0. Raw recipient or evidence bytes never enter the report.

- [ ] **Step 5: Verify post-intent no replay**

```bash
python3 tools/interaction-gate5/verify_report.py check-section \
  --input target/interaction-gate5/live/fault-report.json \
  --section restart-effect
```

Expected: prepared Comment payload committed, effect intent committed, exactly one real Send tap forwarded, worker abort witnessed, restart classifies action/assignment Uncertain, later pending work Interrupted, retry eligibility false, and effect replay count zero. HTTP ACK is not success evidence.

- [ ] **Step 6: Require complete cleanup after every abort**

The restarted parent must reclaim only registry-matched children, prove old PIDs gone, close both device ports, clear readers/watchers, leave no owner/producer, and retain no raw child stderr. The cleanup count must match the two required base live aborts plus the DM-preparation aborts actually attempted under `READY_BOTH`. Failure is G5 FAIL even when database recovery classification is correct.

---

### Task 10: Verify Proxy Truthfulness, Credentials, Nurture, And Optional G4

**Files:**
- Create in ignored work area: `target/interaction-gate5/live/integration-report.json`
- Create then remove after hashing: `target/interaction-gate5/nurture-summary.jsonl`
- Create then remove after hashing: `target/interaction-gate5/nurture-trace.jsonl`
- Create then remove after hashing: `target/interaction-gate5/nurture-frames/**`

- [ ] **Step 1: Run the fixed integration section**

```bash
RIVIU_RTMMO_TOKEN="$(security find-generic-password -s riviu-managers-phone -a agent-auth-token -w)" \
./target/release/live_interaction_fleet_acceptance integrations \
  --udid-a "$RIVIU_GATE5_UDID_A" \
  --udid-b "$RIVIU_GATE5_UDID_B" \
  --fixture-matrix "$RIVIU_GATE5_FIXTURES" \
  --work-dir target/interaction-gate5/live \
  --output target/interaction-gate5/live/integration-report.json
```

- [ ] **Step 2: Verify proxy annotations without claiming iPhone application**

The harness creates a loopback TCP endpoint, creates a temporary proxy catalog row through G3 commands, assigns it to one device, records desktop reachability, records manual confirmation, edits the proxy revision, and proves both annotations invalidate. It then reassigns/unassigns, proves assignment revision changes, deletes the temporary row through the same command boundary, and verifies the row/assignment are absent. No network setting, profile, VPN, MDM command, or iPhone public-IP request runs.

```text
endpoint check: unchecked -> reachable -> invalidated
iPhone state: manual_required -> manually_confirmed -> manual_required
capability: unsupported_unsupervised throughout
forbidden labels/states: applied, device_ip_verified
```

The publishable report contains only proxy ID/revision hashes, typed states, loopback boolean, and latency; no host, port, username, password, exported URL, or raw device ID.

- [ ] **Step 3: Verify credential and process-boundary redaction**

Sample parent/worker/proxy process argv through the platform process inspector while cases run. Require token absent from argv, report, stderr, SQLite string leaves, evidence, and packaged files. Require desktop `agent_token_configured=true` from the existing credential service without exposing the value. AI key is environment-only at the binary boundary and absent from every retained artifact.

Clipboard checks require maximum retained prior bytes 65,536, no raw prior/current bytes in evidence, no owned sentinel after failure/crash cleanup, and correct TargetBackgroundSafe/AgentForegroundRequired PID/bundle witnesses for the qualified tuple.

- [ ] **Step 4: Run every already-qualified G4 action, and no others**

The harness reads the exact registry snapshot captured in baseline. For Save, Repost, and Direct Message:

- Ready on both live tuples: run the fixed G4 fleet case once per device, preserving G4 intent/evidence/no-replay rules.
- Ready on only one tuple: record `NOT_READY_FLEET`, make zero attempts, and keep the UI capability disabled for the fleet report.
- Ready on neither: record `NOT_READY`, make zero attempts.

This step never promotes a G4 action and never turns a fixture result into Ready.

- [ ] **Step 5: Run the existing Nurture live regression sequentially**

After the integration harness proves both devices idle and exits, run Nurture on exactly one device, never concurrently with G5 or desktop:

```bash
cargo build -p riviu-managers-phone --bin live_nurture_test --release
tidevice -u "$RIVIU_GATE5_UDID_A" kill notes.3u || true
RIVIU_WDA_BACKEND=rt-mmo \
RIVIU_RTMMO_TOKEN="$(security find-generic-password -s riviu-managers-phone -a agent-auth-token -w)" \
RIVIU_FRAME_DUMP=target/interaction-gate5/nurture-frames \
RIVIU_WDA_TRACE=target/interaction-gate5/nurture-trace.jsonl \
./target/release/live_nurture_test \
  --udid "$RIVIU_GATE5_UDID_A" \
  --minutes 5 --videos 20 \
  --like-prob 30 --comment-prob 10 --follow-prob 3 \
  --watch-min 4 --watch-max 12 \
  --jsonl target/interaction-gate5/nurture-summary.jsonl
```

Expected exit `0`, at least one processed video, no fallback-coordinate use, session-before-stream, and clean shutdown.

- [ ] **Step 6: Bind Nurture evidence into the integration report atomically**

```bash
python3 tools/interaction-gate5/verify_report.py attach-nurture \
  --input target/interaction-gate5/live/integration-report.json \
  --summary target/interaction-gate5/nurture-summary.jsonl \
  --trace target/interaction-gate5/nurture-trace.jsonl \
  --frame-dir target/interaction-gate5/nurture-frames \
  --output target/interaction-gate5/live/integration-report.json && \
cargo run -q -p rtmmo-re -- verify-redaction \
  --input target/interaction-gate5/nurture-summary.jsonl \
  --input target/interaction-gate5/nurture-trace.jsonl && \
rm -rf target/interaction-gate5/nurture-frames \
  target/interaction-gate5/nurture-summary.jsonl \
  target/interaction-gate5/nurture-trace.jsonl
```

`attach-nurture` must first fsync and atomically replace the enriched report. It retains only aggregate counts, trace/summary/frame-set SHA-256 values, lifecycle booleans, and cleanup proof; a missing input, malformed JSONL, nonzero harness result, zero processed video, or failed redaction leaves the original report unchanged.

- [ ] **Step 7: Verify the integration report section**

```bash
python3 tools/interaction-gate5/verify_report.py check-section \
  --input target/interaction-gate5/live/integration-report.json \
  --section integration
```

Expected: proxy state is truthful, secrets are absent, optional actions match existing registry readiness exactly, and Nurture parity/cleanup are recorded.

---

### Task 11: Build And Smoke-Test The Packaged Desktop App

**Files:**
- Create in ignored work area: `target/interaction-gate5/package-report.json`

- [ ] **Step 1: Run final frontend tests and release package build**

```bash
npm --prefix apps/desktop test -- --run
npm --prefix apps/desktop run lint
npm --prefix apps/desktop run build
(
  cd apps/desktop
  npx playwright test e2e/interaction-workflow.spec.ts --project=chromium
)
npm --prefix apps/desktop run tauri:build
```

Expected: Tauri produces `target/release/bundle/macos/Riviumanagersphone.app`. Do not package from a dirty checkout or substitute a dev server build.

- [ ] **Step 2: Verify codesign, executable, and embedded resources**

```bash
APP=target/release/bundle/macos/Riviumanagersphone.app
test -x "$APP/Contents/MacOS/Riviumanagersphone"
codesign --verify --deep --strict "$APP"
shasum -a 256 \
  "$APP/Contents/Resources/sidecars/wda/RiviuAgent.ipa" \
  "$APP/Contents/Resources/sidecars/wda/agent-manifest.json"
cmp sidecars/wda/RiviuAgent.ipa "$APP/Contents/Resources/sidecars/wda/RiviuAgent.ipa"
cmp sidecars/wda/agent-manifest.json "$APP/Contents/Resources/sidecars/wda/agent-manifest.json"
```

- [ ] **Step 3: Run packaged launch, accessibility, screenshot, and cleanup smoke**

```bash
python3 tools/interaction-gate5/package_smoke.py run \
  --app target/release/bundle/macos/Riviumanagersphone.app \
  --repo-root . \
  --output target/interaction-gate5/package-report.json
```

Expected: signed app remains alive through bootstrap with mock devices, one app window exists, `Tương tác` opens the real panel, UI capture is nonblank/contained, token is absent from argv/logs, packaged resources match source, and the exact app process plus owned children are gone after cleanup. `PENDING_MAC_PERMISSION` must be resolved and rerun; it is not PASS.

- [ ] **Step 4: Verify package report**

```bash
python3 tools/interaction-gate5/verify_report.py check-package \
  --input target/interaction-gate5/package-report.json \
  --source-ipa sidecars/wda/RiviuAgent.ipa \
  --source-manifest sidecars/wda/agent-manifest.json
```

Expected: package status PASS, exact source/resource equality, nonblank UI evidence, and clean process exit.

---

### Task 12: Merge And Preflight G5 Evidence

**Files:**
- Create in ignored work area: `target/interaction-gate5/merged-pre-rollback.json`
- Create in ignored work area: `target/interaction-gate5/preflight.json`

- [ ] **Step 1: Merge only reports bound to one baseline and source commit**

```bash
python3 tools/interaction-gate5/verify_report.py merge \
  --baseline target/interaction-gate5/baseline.json \
  --base target/interaction-gate5/live/base-report.json \
  --faults target/interaction-gate5/live/fault-report.json \
  --integrations target/interaction-gate5/live/integration-report.json \
  --package target/interaction-gate5/package-report.json \
  --output target/interaction-gate5/merged-pre-rollback.json
```

Reject differing source commits, production hashes, registry hashes, fixture-matrix hash, device hashes/tuples, or timestamps that overlap another harness/desktop owner. Merge copies rows, not trusted aggregate PASS fields, and writes the required closed-schema `rollback` object only as `{ "status": "PENDING_ROLLBACK" }`; no other command may synthesize rollback success.

- [ ] **Step 2: Require every non-rollback section without publishing PASS**

```bash
python3 tools/interaction-gate5/verify_report.py preflight \
  --input target/interaction-gate5/merged-pre-rollback.json \
  --registry sidecars/wda/interaction-capabilities.json \
  --output target/interaction-gate5/preflight.json
```

Expected: `environment=LIVE_MAC_TWO_DEVICE`, `gateStatus=PENDING_ROLLBACK`, `qualifiedStreamBudget` is 1 or 2 under the rules above, every non-rollback matrix count is exact, optional actions mirror existing readiness, Nurture/package/cleanup/redaction pass, and no file under `docs/re/interaction-gate5/` is created or changed.

- [ ] **Step 3: Run independent redaction verification on the preflight artifacts**

```bash
cargo run -q -p rtmmo-re -- verify-redaction \
  --input target/interaction-gate5/merged-pre-rollback.json \
  --input target/interaction-gate5/preflight.json
python3 -m unittest discover -s tools/interaction-gate5 -p "test_*.py" -v
```

- [ ] **Step 4: Recompute artifacts, registry, and source after preflight**

```bash
test "$(shasum -a 256 sidecars/wda/RiviuAgent.ipa | awk '{print $1}')" = "8a24847099495ff70b998522692c43f00dd16b90f698bda6953a73f5d33002ea"
test "$(shasum -a 256 sidecars/wda/agent-manifest.json | awk '{print $1}')" = "e98a549af4c061556effd36424e7732219e1a6d262bcf1f259279975024b6e1a"
test "$(git rev-parse HEAD)" = "$(cat target/interaction-gate5/source-commit.txt)"
python3 tools/interaction-gate5/verify_report.py check-registry-unchanged \
  --baseline target/interaction-gate5/baseline.json \
  --registry sidecars/wda/interaction-capabilities.json
```

G5 does not add a capability entry. If an action or tuple is missing, return to its owning G0/G2/G4 live gate, qualify it there, then rerun all G5 sections against a new baseline. Task 12 intentionally cannot produce `gateStatus=PASS`; Task 13 may bind rollback evidence and create a sanitized PASS staging set under ignored `target/`, but only Task 14 may publish it under `docs/re/` after the final regression.

---

### Task 13: Drill Capability And N-1 Desktop Rollback

**Files:**
- Create in ignored work area: `target/interaction-gate5/rollback/**`
- Create in ignored work area: `target/interaction-gate5/rollback-report.json`
- Create in ignored work area: `target/interaction-gate5/merged.json`
- Create in ignored work area: `target/interaction-gate5/publication/gate-5.json`
- Create in ignored work area: `target/interaction-gate5/publication/gate-5.md`
- Create in ignored work area: `target/interaction-gate5/publication/README.md`

- [ ] **Step 1: Snapshot the reviewed registry and current database**

```bash
mkdir -p target/interaction-gate5/rollback
cp sidecars/wda/interaction-capabilities.json target/interaction-gate5/rollback/registry-before.json
cp "$HOME/Library/Application Support/riviu-managers-phone/riviu.db" target/interaction-gate5/rollback/riviu-current.db
shasum -a 256 target/interaction-gate5/rollback/registry-before.json \
  target/interaction-gate5/rollback/riviu-current.db
```

- [ ] **Step 2: Drill one action revocation through a transactional temporary registry**

Choose Comment because it is required by G5 and already qualified by G2. `drill-revocation` copies the reviewed registry, appends a report-bound Comment revocation to the copy, loads it through the production registry parser, and runs the capability provider against both recorded tuples.

```bash
python3 tools/interaction-gate5/verify_report.py drill-revocation \
  --source-registry sidecars/wda/interaction-capabilities.json \
  --temporary-registry target/interaction-gate5/rollback/registry-revoked.json \
  --report target/interaction-gate5/merged-pre-rollback.json \
  --action comment \
  --output target/interaction-gate5/rollback/revocation-report.json
cargo test -p riviu-ios-driver interaction_capability_registry -- --nocapture
```

Expected: temporary negotiation reports Comment `LiveQualificationRevoked`, Open/identity/Like/Follow remain unchanged, no action is attempted, and the production registry still matches the Step 1 snapshot.

- [ ] **Step 3: Build the N-1 desktop in an isolated worktree**

Set the reviewed commit immediately before the G3 desktop workflow was merged:

```bash
: "${RIVIU_GATE5_N1_COMMIT:?set reviewed pre-G3 commit}"
git merge-base --is-ancestor "$RIVIU_GATE5_N1_COMMIT" HEAD
git worktree add --detach target/interaction-gate5/rollback/n1-worktree "$RIVIU_GATE5_N1_COMMIT"
(
  cd target/interaction-gate5/rollback/n1-worktree
  npm --prefix apps/desktop ci
  npm --prefix apps/desktop run build
  cargo test --workspace
  npm --prefix apps/desktop run tauri:build
)
```

- [ ] **Step 4: Launch N-1 against an isolated copy of the additive database**

```bash
N1_HOME="$PWD/target/interaction-gate5/rollback/n1-home"
mkdir -p "$N1_HOME/Library/Application Support/riviu-managers-phone"
cp target/interaction-gate5/rollback/riviu-current.db \
  "$N1_HOME/Library/Application Support/riviu-managers-phone/riviu.db"
HOME="$N1_HOME" RIVIU_MOCK_DEVICES=1 \
python3 tools/interaction-gate5/package_smoke.py boot-only \
  --app target/interaction-gate5/rollback/n1-worktree/target/release/bundle/macos/Riviumanagersphone.app \
  --output target/interaction-gate5/rollback/n1-package-report.json
```

Expected: N-1 starts, reads its existing tables/settings, ignores additive Interaction/proxy annotation tables, does not delete campaign history, and exits cleanly. The probe uses the copied database only.

- [ ] **Step 5: Remove the rollback worktree and verify restoration**

```bash
git worktree remove target/interaction-gate5/rollback/n1-worktree
test "$(shasum -a 256 sidecars/wda/interaction-capabilities.json | awk '{print $1}')" = \
  "$(shasum -a 256 target/interaction-gate5/rollback/registry-before.json | awk '{print $1}')"
cmp sidecars/wda/interaction-capabilities.json target/interaction-gate5/rollback/registry-before.json
test "$(shasum -a 256 sidecars/wda/RiviuAgent.ipa | awk '{print $1}')" = "8a24847099495ff70b998522692c43f00dd16b90f698bda6953a73f5d33002ea"
test "$(shasum -a 256 sidecars/wda/agent-manifest.json | awk '{print $1}')" = "e98a549af4c061556effd36424e7732219e1a6d262bcf1f259279975024b6e1a"
```

- [ ] **Step 6: Build a rollback report bound to the same baseline**

```bash
python3 tools/interaction-gate5/verify_report.py finish-rollback \
  --baseline target/interaction-gate5/baseline.json \
  --registry-before target/interaction-gate5/rollback/registry-before.json \
  --registry-current sidecars/wda/interaction-capabilities.json \
  --revocation target/interaction-gate5/rollback/revocation-report.json \
  --n1-commit "$RIVIU_GATE5_N1_COMMIT" \
  --n1-package target/interaction-gate5/rollback/n1-package-report.json \
  --database-before target/interaction-gate5/rollback/riviu-current.db \
  --database-after "$PWD/target/interaction-gate5/rollback/n1-home/Library/Application Support/riviu-managers-phone/riviu.db" \
  --output target/interaction-gate5/rollback-report.json
```

Expected: PASS requires the temporary revocation to disable Comment without a device attempt, the production registry to match byte-for-byte, the N-1 package to boot and exit cleanly, every preexisting Interaction/proxy table and row identity to remain present in the copied database, and both production artifact hashes to remain fixed.

- [ ] **Step 7: Bind rollback evidence and stage the sanitized evidence trio**

```bash
set -Eeuo pipefail
STAGING=target/interaction-gate5/publication
trap 'rm -rf "$STAGING"; rm -f target/interaction-gate5/merged.json' ERR INT TERM
python3 tools/interaction-gate5/verify_report.py finalize \
  --input target/interaction-gate5/merged-pre-rollback.json \
  --rollback target/interaction-gate5/rollback-report.json \
  --output target/interaction-gate5/merged.json
rm -rf "$STAGING"
python3 tools/interaction-gate5/verify_report.py stage \
  --input target/interaction-gate5/merged.json \
  --registry sidecars/wda/interaction-capabilities.json \
  --output-dir "$STAGING"
python3 tools/interaction-gate5/verify_report.py render-readme \
  --input "$STAGING/gate-5.json" \
  --template docs/re/interaction-gate5/README.md \
  --output "$STAGING/README.md"
python3 tools/interaction-gate5/verify_report.py verify-staged \
  --input target/interaction-gate5/merged.json \
  --registry sidecars/wda/interaction-capabilities.json \
  --staging-dir "$STAGING"
cargo run -q -p rtmmo-re -- verify-redaction \
  --input "$STAGING/README.md" \
  --input "$STAGING/gate-5.json" \
  --input "$STAGING/gate-5.md"
test -z "$(git status --porcelain -- docs/re/interaction-gate5)"
trap - ERR INT TERM
```

Expected: `environment=LIVE_MAC_TWO_DEVICE`, staged `gateStatus=PASS`, rollback PASS, and all three sanitized files appear together only under ignored `target/interaction-gate5/publication/`. Every path under `docs/re/interaction-gate5/` remains byte-identical to HEAD. Task 13 does not expose a new PASS to users or Git.

---

### Task 14: Run Final Regression, Update Handoff, And Commit Evidence

**Files:**
- Read from ignored work area: `target/interaction-gate5/merged.json`
- Read from ignored work area: `target/interaction-gate5/publication/**`
- Create after PASS: `docs/re/interaction-gate5/gate-5.json`
- Create after PASS: `docs/re/interaction-gate5/gate-5.md`
- Modify: `docs/re/interaction-gate5/README.md`
- Modify: `AGENTS.md`

- [ ] **Step 1: Run fresh final verification**

```bash
set -Eeuo pipefail
STAGING=target/interaction-gate5/publication
trap 'rm -rf "$STAGING"; rm -f target/interaction-gate5/merged.json' ERR INT TERM
cargo fmt --all -- --check
cargo test --workspace
cargo clippy --workspace --all-targets --all-features -- -D warnings
npm --prefix apps/desktop test -- --run
npm --prefix apps/desktop run lint
npm --prefix apps/desktop run build
python3 -m unittest discover -s tools/interaction-gate5 -p "test_*.py" -v
python3 tools/interaction-gate5/verify_report.py verify-staged \
  --input target/interaction-gate5/merged.json \
  --registry sidecars/wda/interaction-capabilities.json \
  --staging-dir "$STAGING"
cargo run -q -p rtmmo-re -- verify-redaction \
  --input "$STAGING/README.md" \
  --input "$STAGING/gate-5.json" \
  --input "$STAGING/gate-5.md"
test -z "$(git status --porcelain -- docs/re/interaction-gate5)"
trap - ERR INT TERM
```

Any command failure removes both the staged PASS trio and finalized PASS merge, then stops while `docs/re/interaction-gate5/` is still unchanged. Recreate them through Task 13 after fixing the failure; never reuse a partial staging directory or an earlier finalized report.

- [ ] **Step 2: Publish the staged trio through one recoverable transaction**

```bash
set -Eeuo pipefail
TX=target/interaction-gate5/publication-transaction
rollback_publication() {
  trap - ERR INT TERM
  python3 tools/interaction-gate5/verify_report.py rollback-publication \
    --transaction "$TX" \
    --output-dir docs/re/interaction-gate5
}
trap rollback_publication ERR INT TERM
rm -rf "$TX"
python3 tools/interaction-gate5/verify_report.py publish-staged \
  --input target/interaction-gate5/merged.json \
  --registry sidecars/wda/interaction-capabilities.json \
  --staging-dir target/interaction-gate5/publication \
  --output-dir docs/re/interaction-gate5 \
  --transaction "$TX"
python3 tools/interaction-gate5/verify_report.py verify-published \
  --json docs/re/interaction-gate5/gate-5.json \
  --markdown docs/re/interaction-gate5/gate-5.md \
  --readme docs/re/interaction-gate5/README.md \
  --registry sidecars/wda/interaction-capabilities.json
cargo run -q -p rtmmo-re -- verify-redaction \
  --input docs/re/interaction-gate5/README.md \
  --input docs/re/interaction-gate5/gate-5.json \
  --input docs/re/interaction-gate5/gate-5.md
trap - ERR INT TERM
```

`publish-staged` must restore all prior bytes and remove any newly created destination when any one of the three replacements or validations fails. Keep `$TX` intact after success so a later staging or commit failure can restore the pre-publication trio. Do not manually copy any staged file into `docs/re`.

- [ ] **Step 3: Update `AGENTS.md` with measured facts only**

First preserve the exact pre-edit handoff bytes in ignored target state:

```bash
cp AGENTS.md target/interaction-gate5/AGENTS.before-publication.md
```

Record:

- G0-G5 commit range and exact test commands/counts;
- both hashed device tuples and fixture/report hashes, never raw UDIDs/URLs/handles/text;
- qualified stream budget (`1` or `2`) and why budget 2 was or was not qualified;
- action-by-action readiness, with unqualified G4 actions still disabled;
- All/RoundRobin, cancellation, Busy/Partial, all three restart/no-replay branches, Nurture, proxy, credential, package, cleanup, and redaction results;
- production IPA/manifest hashes unchanged;
- Project 2 candidate remains separate until its own live text/open-URL/clipboard gates pass;
- rollback commands, N-1 commit/package hash, and additive-history behavior.

Do not write PASS for a missing live section or translate `NOT_QUALIFIED_CLEAN` into budget-2 support. If this edit or any later pre-commit check fails, restore `AGENTS.md` from the ignored snapshot and invoke `rollback-publication` before stopping so neither handoff nor evidence leaves a new visible PASS without a commit.

- [ ] **Step 4: Stage only evidence and the reviewed handoff hunk**

```bash
git add docs/re/interaction-gate5/README.md \
  docs/re/interaction-gate5/gate-5.json \
  docs/re/interaction-gate5/gate-5.md
git add -p AGENTS.md
git diff --cached --name-only
EXPECTED_PATHS="$(printf '%s\n' \
  AGENTS.md \
  docs/re/interaction-gate5/README.md \
  docs/re/interaction-gate5/gate-5.json \
  docs/re/interaction-gate5/gate-5.md | sort)"
ACTUAL_PATHS="$(git diff --cached --name-only | sort)"
if ! test "$ACTUAL_PATHS" = "$EXPECTED_PATHS" || ! git diff --cached --check; then
  git restore --staged -- docs/re/interaction-gate5 AGENTS.md
  cp target/interaction-gate5/AGENTS.before-publication.md AGENTS.md
  python3 tools/interaction-gate5/verify_report.py rollback-publication \
    --transaction target/interaction-gate5/publication-transaction \
    --output-dir docs/re/interaction-gate5
  exit 1
fi
```

Expected: exactly the three G5 evidence paths plus only the G5 checkpoint hunk from `AGENTS.md`. No raw live file, fixture matrix, screenshot, trace, database, token, Agent artifact, registry, or unrelated dirty file is staged.

If either scope check fails, the command block restores the exact pre-edit handoff and prior published trio or prior absence. Fix the scope cause, then retry Task 14 from final verification against the intact staging set; rerun Task 13 only when staging itself was removed or invalidated.

- [ ] **Step 5: Commit the verified gate and seal publication**

```bash
if ! git commit -m "test(interaction): qualify two-device fleet release"; then
  git restore --staged -- docs/re/interaction-gate5 AGENTS.md
  cp target/interaction-gate5/AGENTS.before-publication.md AGENTS.md
  python3 tools/interaction-gate5/verify_report.py rollback-publication \
    --transaction target/interaction-gate5/publication-transaction \
    --output-dir docs/re/interaction-gate5
  exit 1
fi
python3 tools/interaction-gate5/verify_report.py seal-publication \
  --transaction target/interaction-gate5/publication-transaction
rm -f target/interaction-gate5/AGENTS.before-publication.md
```

- [ ] **Step 6: Verify the commit and cleanup one final time**

```bash
git show --stat --oneline HEAD
git status --short
python3 tools/interaction-gate5/verify_report.py verify-published \
  --json docs/re/interaction-gate5/gate-5.json \
  --markdown docs/re/interaction-gate5/gate-5.md \
  --readme docs/re/interaction-gate5/README.md \
  --registry sidecars/wda/interaction-capabilities.json
```

Expected: evidence still validates from committed bytes and no G5 child/relay/reader/device port remains active.

---

## Gate G5 Completion Criteria

- [ ] Exactly two distinct live iPhones pass the required G0 contracts, G2 production-runtime, and G2 action tuple checks.
- [ ] Budget-1 All and RoundRobin pass with correct assignment counts, actor use, and max one producer.
- [ ] Budget 2 is either independently PASS or cleanly not qualified; default remains 1 and no overrun is accepted.
- [ ] Direct video, photo, and resolved short-link identity match copied content ID/kind on both devices.
- [ ] Like, Follow, and Unicode Comment have frame-derived evidence; ACK alone never counts.
- [ ] Cancellation starts no later action; Busy is typed, immediate, partial-and-continue, and never redistributed.
- [ ] Plan-commit restart preserves seeds/policies/timing while proving the DM recipient payload is absent; the prepared-DM fixture always reloads identical payload/evidence bytes exactly once with no intent, replacement, or resampling, and the live branch does the same only for `READY_BOTH` or records a zero-attempt typed disposition.
- [ ] Post-intent Comment has one effect tap, Uncertain state, and zero replay.
- [ ] G4 actions run only when already Ready on both exact tuples; absent actions remain disabled without blocking G0-G3 release.
- [ ] Proxy evidence distinguishes desktop reachability from manual iPhone state and never claims applied/device-IP verified.
- [ ] Tokens, AI keys, clipboard bytes, proxy secrets, raw identifiers/content, paths, and source errors are absent from published evidence.
- [ ] Existing Nurture live regression passes sequentially and all streams/processes/ports clean up.
- [ ] Packaged desktop app is signed/launchable, opens Interaction, has nonblank contained UI, and embeds byte-identical production Agent resources.
- [ ] Production IPA/manifest and candidate/stock source trees remain unchanged.
- [ ] Capability revocation and N-1 desktop rollback drills pass without deleting additive history.
- [ ] JSON/Markdown/README staging survives final regression and redaction before atomic publication; replacement failure restores the prior trio or absence, and committed evidence is reproducible.

## Failure And Retry Rules

- A required base, fault, package, cleanup, or redaction failure makes G5 FAIL. Repair the first typed cause and rerun every G5 section from a new baseline.
- Budget-2 performance may be `NOT_QUALIFIED_CLEAN`; keep budget 1. Producer overrun or dirty cleanup is never downgraded to non-qualification.
- A missing G4 capability remains disabled. Qualify it only through G4, then rerun the complete G5 baseline/matrix if fleet exposure is desired.
- A new iPhone/iOS/TikTok/Agent/transport/geometry tuple fails closed and returns to G0/G2/G4 qualification. It never inherits iPhone 8 coordinates or an older TikTok detector.
- A Task 14 final-regression failure removes target staging plus the finalized PASS merge and leaves `docs/re` unchanged. A publication, pre-commit, or commit failure restores the prior three published paths byte-for-byte (or removes newly created paths) before the run stops; a failed run must not leave a new visible PASS.
- A published report is immutable evidence. Reruns create a new complete transaction; never edit PASS fields or splice rows across source/registry/fixture baselines.
