# Riviu Flow V2 Runtime Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Execute immutable Flow revisions with per-device durability, exact stream-generation evidence, contained artifacts, conservative retry/recovery, and joined shutdown.

**Architecture:** `FlowRuntime` in core owns run orchestration and persists every state transition before crossing a side-effect boundary. It acquires one device per worker, upgrades control-plane typestate monotonically, and reads frames only through a generation-aware extension of `FrameSource`.

**Tech Stack:** Rust 2021, Tokio, rusqlite, image, sha2, Tauri event bus, Python asyncio, pymobiledevice3 10.1.0 DVT.

---

### Task 1: Add Exact-Generation Frame Access

**Files:**
- Modify: `crates/core/src/frame_source.rs`
- Modify: `crates/core/src/lib.rs`
- Modify: `crates/ios-driver/src/stream.rs`
- Test: `crates/ios-driver/src/stream.rs`

- [x] **Step 1: Write failing generation tests**

Add tests proving a generation subscriber never emits a buffered frame from
generation N after `clear_and_advance`, immediately reports the N -> N+1 advance,
and that `latest_in_generation` returns `None` for the old generation. Also prove a
closed hub returns `Closed` instead of hanging.

```rust
#[tokio::test]
async fn generation_subscription_rejects_buffered_old_frames() {
    let hub = StreamHub::new();
    let old = hub.generation("fixture");
    let mut old_stream = hub.subscribe_generation("fixture", old);
    assert!(hub.publish_if_current("fixture", old, vec![1, 2, 3]));
    let (_, new) = hub.clear_and_advance("fixture");
    assert!(hub.latest_in_generation("fixture", old).is_none());
    assert_eq!(
        old_stream.next().await,
        GenerationFrameEvent::Advanced { expected: old, actual: new },
    );
    assert!(hub.publish_if_current("fixture", new, vec![9, 8, 7]));
    let latest = hub.latest_in_generation("fixture", new).expect("new frame");
    assert_eq!(&*latest.bytes, &[9, 8, 7]);
}
```

- [x] **Step 2: Run tests red**

```powershell
cargo test -p riviu-ios-driver stream::tests::generation -- --nocapture
```

Expected: FAIL because generation-aware traits are absent.

- [x] **Step 3: Define the generation-aware core contract**

Append to `frame_source.rs`:

```rust
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GenerationFrame {
    pub generation: u64,
    pub bytes: Frame,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GenerationFrameEvent {
    Frame(GenerationFrame),
    Advanced { expected: u64, actual: u64 },
    Closed,
}

#[async_trait]
pub trait GenerationFrameStream: Send {
    async fn next(&mut self) -> GenerationFrameEvent;
}

pub trait GenerationFrameSource: FrameSource {
    fn subscribe_generation(
        &self,
        udid: &str,
        generation: u64,
    ) -> Box<dyn GenerationFrameStream>;

    fn latest_in_generation(&self, udid: &str, generation: u64) -> Option<GenerationFrame>;
}
```

Export all four types from `crates/core/src/lib.rs`. Do not add a default
implementation that falls back to unqualified `latest()`.

- [x] **Step 4: Implement StreamHub generation fan-out**

Add a second broadcast sender carrying a private `HubGenerationEvent` enum with
`Frame { udid, generation, bytes }` and `Advanced { udid, generation }` variants.
Publish `Frame` from both `publish` and `publish_if_current` using the current
generation. `clear_and_advance` increments state first and then broadcasts exactly
one `Advanced` marker for that UDID before returning. Implement a filtering
`HubGenerationStream` and `GenerationFrameSource` for `StreamHub`. Keep the existing
raw subscription API unchanged for desktop tiles and Nurture.

```rust
#[derive(Clone)]
enum HubGenerationEvent {
    Frame { udid: String, generation: u64, bytes: Frame },
    Advanced { udid: String, generation: u64 },
}

struct HubGenerationStream {
    udid: String,
    generation: u64,
    state: std::sync::Arc<parking_lot::RwLock<HubState>>,
    rx: broadcast::Receiver<HubGenerationEvent>,
}

#[async_trait]
impl GenerationFrameStream for HubGenerationStream {
    async fn next(&mut self) -> GenerationFrameEvent {
        loop {
            let actual = self.state.read().generations
                .get(&self.udid).copied().unwrap_or(0);
            if actual > self.generation {
                return GenerationFrameEvent::Advanced {
                    expected: self.generation,
                    actual,
                };
            }
            match self.rx.recv().await {
                Ok(HubGenerationEvent::Frame { udid, generation, bytes })
                    if udid == self.udid && generation == self.generation => {
                    let actual = self.state.read().generations
                        .get(&self.udid).copied().unwrap_or(0);
                    if actual > self.generation {
                        return GenerationFrameEvent::Advanced {
                            expected: self.generation,
                            actual,
                        };
                    }
                    return GenerationFrameEvent::Frame(GenerationFrame { generation, bytes });
                }
                Ok(HubGenerationEvent::Advanced { udid, generation })
                    if udid == self.udid && generation > self.generation =>
                    return GenerationFrameEvent::Advanced {
                        expected: self.generation,
                        actual: generation,
                    },
                Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) =>
                    return GenerationFrameEvent::Closed,
            }
        }
    }
}

impl GenerationFrameSource for StreamHub {
    fn subscribe_generation(
        &self,
        udid: &str,
        generation: u64,
    ) -> Box<dyn GenerationFrameStream> {
        Box::new(HubGenerationStream {
            udid: udid.to_string(),
            generation,
            state: self.state.clone(),
            rx: self.generation_tx.subscribe(),
        })
    }

    fn latest_in_generation(&self, udid: &str, generation: u64) -> Option<GenerationFrame> {
        let state = self.state.read();
        (state.generations.get(udid).copied().unwrap_or(0) == generation)
            .then(|| state.latest.get(udid).cloned())
            .flatten()
            .map(|bytes| GenerationFrame { generation, bytes })
    }
}
```

Add `generation_tx` to `StreamHub`; every accepted publish sends the raw and
generation-qualified event while holding the same state generation used for the
latest cache. The advance marker is part of the public behavioral contract: a
subscriber for an invalidated generation must become observable immediately even
when no frame from the replacement stream has arrived.

- [x] **Step 5: Run tests and commit**

```powershell
cargo fmt --all
cargo test -p riviu-ios-driver stream::tests -- --nocapture
cargo test -p riviu-core frame_source -- --nocapture
git add crates/core/src/frame_source.rs crates/core/src/lib.rs crates/ios-driver/src/stream.rs
git commit -m "feat(flow): qualify frames by stream generation"
```

Expected: old-generation frames are never observable through the Flow contract.

### Task 2: Add Crash-Consistent Artifact Storage

**Files:**
- Create: `crates/core/src/flow/artifact_store.rs`
- Modify: `crates/core/src/flow/mod.rs`
- Test: `crates/core/src/flow/artifact_store.rs`

- [x] **Step 1: Write failing containment and failpoint tests**

Cover labels `../x`, `a/b`, `CON`, control bytes, and unsupported extensions. Cover valid JPEG/PNG decode, hash/size, temp-file cleanup, rename success, simulated DB rejection cleanup, stale temp cleanup, orphan final quarantine, and missing/hash-mismatched committed files.

```rust
fn temp_artifact_root() -> std::path::PathBuf {
    let root = std::env::temp_dir().join(format!(
        "riviu-flow-artifacts-{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&root).expect("create artifact root");
    root
}

#[test]
fn artifact_labels_never_become_paths() {
    let root = temp_artifact_root();
    let store = FlowArtifactStore::new(&root).expect("store");
    for label in ["../x", "a/b", "CON", "bad\u{0001}name", "shot.exe"] {
        assert!(store.validate_label(label, "jpeg").is_err(), "accepted {label:?}");
    }
    let prepared = store.prepare_image(
        uuid::Uuid::from_u128(1),
        uuid::Uuid::from_u128(2),
        uuid::Uuid::from_u128(3),
        "screen.png",
        "png",
        include_bytes!("../../tests/fixtures/feed-rail-variant.png"),
    ).expect("prepared image");
    assert!(!prepared.relative_path.to_string_lossy().contains("screen.png"));
    assert_eq!(prepared.sha256.len(), 64);
    std::fs::remove_dir_all(root).expect("remove artifact root");
}
```

- [x] **Step 2: Run tests red**

```powershell
cargo test -p riviu-core flow::artifact_store -- --nocapture
```

Expected: FAIL because `FlowArtifactStore` is absent.

- [x] **Step 3: Add artifact types**

Reuse the `sha2` dependency added by F0. Define:

```rust
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PreparedArtifact {
    pub id: uuid::Uuid,
    pub relative_path: std::path::PathBuf,
    pub kind: String,
    pub size: u64,
    pub sha256: String,
    temp_path: std::path::PathBuf,
    final_path: std::path::PathBuf,
}

#[derive(Clone)]
pub struct FlowArtifactStore {
    root: std::path::PathBuf,
    quarantine: std::path::PathBuf,
}
```

- [x] **Step 4: Implement prepare, publish, rollback, and reconcile**

`prepare_image(run_id, device_run_id, attempt_id, label, bytes)` must create
`<root>/.staging/<uuid>.tmp`, write and `sync_all`, decode with
`image::load_from_memory`, compute SHA-256, and choose
`<root>/<run_uuid>/<device_run_uuid>/<attempt_uuid>/<artifact_uuid>.<ext>`.
Every path component except the fixed extension is a server-generated UUID; raw
UDID and label never enter a path. `publish_file` atomically renames within the same
root and returns the final relative path. `rollback_file` removes the final or temp
path. `reconcile` accepts the rows loaded from SQLite, removes stale staging files,
moves orphan finals into `<root>/.quarantine`, and returns typed failures for
missing/hash-mismatched rows.

Its `validate_label` method delegates to F0's `validate_artifact_label`; do not
create a second path/extension validator in the artifact module.

Expose this exact API so the executor can enforce file/DB ordering:

```rust
impl FlowArtifactStore {
    pub fn new(root: impl AsRef<std::path::Path>) -> anyhow::Result<Self>;
    pub fn validate_label(&self, label: &str, format: &str) -> anyhow::Result<()>;
    pub fn prepare_image(
        &self,
        run_id: uuid::Uuid,
        device_run_id: uuid::Uuid,
        attempt_id: uuid::Uuid,
        label: &str,
        format: &str,
        bytes: &[u8],
    ) -> anyhow::Result<PreparedArtifact>;
    pub fn publish_file(&self, artifact: &PreparedArtifact) -> anyhow::Result<std::path::PathBuf>;
    pub fn rollback_file(&self, artifact: &PreparedArtifact) -> anyhow::Result<()>;
    pub fn reconcile(
        &self,
        rows: &[FlowArtifactRecord],
    ) -> anyhow::Result<Vec<ArtifactReconciliationFailure>>;
}
```

Define `ArtifactReconciliationFailure { artifact_id, code }`, where code is exactly
`Missing`, `HashMismatch`, or `QuarantinedOrphan`. The executor calls
`rollback_file` on every database failure; `reconcile` never marks an attempt
successful itself.

Use canonicalized parent checks before every rename/remove; no method accepts an absolute destination or a user label.

- [x] **Step 5: Run tests and commit**

```powershell
cargo fmt --all
cargo test -p riviu-core flow::artifact_store -- --nocapture
git add crates/core/src/flow
git commit -m "feat(flow): publish contained crash-consistent artifacts"
```

### Task 3: Persist Runs, Device Runs, Attempts, And Events

**Files:**
- Create: `crates/core/src/db/flow_runs.rs`
- Modify: `crates/core/src/db.rs`
- Modify: `crates/core/src/flow/model.rs`
- Test: `crates/core/src/db/flow_runs.rs`

- [ ] **Step 1: Write transition tests**

Test run creation with exact selection snapshot, independent device states, the full legal attempt transition graph, rejection of skipped transitions, commit of `IntentCommitted` before `EffectDispatched`, atomic artifact-row plus `Succeeded`, monotonic event revisions, and startup classification of every nonterminal state.

```rust
#[test]
fn an_attempt_cannot_skip_the_committed_intent_boundary() {
    let (database, path, attempt_id) = flow_run_fixture();
    let error = database.transition_attempt(
        attempt_id,
        FlowAttemptState::Queued,
        FlowAttemptState::EffectDispatched,
        AttemptTransitionPatch::default(),
    ).expect_err("skipped intent must fail");
    assert!(error.to_string().contains("StateConflict"));
    database.transition_attempt(
        attempt_id,
        FlowAttemptState::Queued,
        FlowAttemptState::IntentCommitted,
        AttemptTransitionPatch {
            canonical_input: Some(serde_json::json!({"point":{"x":10,"y":20}})),
            ..Default::default()
        },
    ).expect("commit intent");
    std::fs::remove_file(path).expect("remove run database");
}
```

- [ ] **Step 2: Run tests red**

```powershell
cargo test -p riviu-core db::flow_runs -- --nocapture
```

Expected: FAIL because run projections and repository operations are absent.

- [ ] **Step 3: Add durable enums and projections**

Add exact serde camelCase enums to `model.rs`:

```rust
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum FlowAttemptState {
    Queued,
    IntentCommitted,
    EffectDispatched,
    Verifying,
    Succeeded,
    FailedBeforeDispatch,
    FailedVerified,
    Uncertain,
    Cancelled,
    Interrupted,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum FlowAggregateState { Queued, Running, Succeeded, Partial, Failed, Cancelled }

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum FlowDeviceRunState {
    Queued,
    Preflight,
    Running,
    Succeeded,
    Failed,
    Skipped,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "mode", rename_all = "camelCase")]
pub enum FlowTargetSelection {
    One { udid: String },
    Selected { udids: Vec<String> },
    AllEligible,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FlowSelectionSnapshot {
    pub requested: FlowTargetSelection,
    pub target_udids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FlowErrorRecord {
    pub code: String,
    pub message: String,
    pub node_id: Option<NodeId>,
    pub field: Option<String>,
    pub udid: Option<String>,
    pub attempt_id: Option<uuid::Uuid>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FlowContextReleaseProof {
    pub udid: String,
    pub owner: crate::DeviceWorkOwner,
    pub had_session: bool,
    pub had_stream: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FlowRunRecord {
    pub id: uuid::Uuid,
    pub flow_id: FlowId,
    pub flow_revision: u64,
    pub plan_sha256: String,
    pub selection: FlowSelectionSnapshot,
    pub state: FlowAggregateState,
    pub event_revision: u64,
    pub error: Option<FlowErrorRecord>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FlowDeviceRunRecord {
    pub id: uuid::Uuid,
    pub run_id: uuid::Uuid,
    pub udid: String,
    pub state: FlowDeviceRunState,
    pub capability_snapshot: Option<crate::DeviceCapabilitySnapshot>,
    pub release_proof: Option<FlowContextReleaseProof>,
    pub error: Option<FlowErrorRecord>,
    pub started_at: Option<chrono::DateTime<chrono::Utc>>,
    pub finished_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FlowNodeAttemptRecord {
    pub id: uuid::Uuid,
    pub device_run_id: uuid::Uuid,
    pub node_id: NodeId,
    pub action_kind: ActionKind,
    pub attempt_no: u32,
    pub side_effect_class: SideEffectClass,
    pub state: FlowAttemptState,
    pub canonical_input: Option<serde_json::Value>,
    pub evidence_baseline: Option<serde_json::Value>,
    pub evidence_result: Option<serde_json::Value>,
    pub retry_allowed: bool,
    pub error: Option<FlowErrorRecord>,
    pub started_at: Option<chrono::DateTime<chrono::Utc>>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
    pub finished_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FlowArtifactRecord {
    pub id: uuid::Uuid,
    pub attempt_id: uuid::Uuid,
    pub relative_path: String,
    pub label: String,
    pub kind: String,
    pub size: u64,
    pub sha256: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FlowEventRecord {
    pub id: i64,
    pub run_id: uuid::Uuid,
    pub revision: u64,
    pub kind: String,
    pub payload: serde_json::Value,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FlowRunDetail {
    pub run: FlowRunRecord,
    pub device_runs: Vec<FlowDeviceRunRecord>,
    pub attempts: Vec<FlowNodeAttemptRecord>,
    pub artifacts: Vec<FlowArtifactRecord>,
}
```

Add `is_terminal()` to attempt/device/aggregate states and `is_success()` to
`FlowDeviceRunState`; the F3 integration test uses those methods. Store enums with
their serde camelCase spelling exactly as migration 2's `CHECK` values.

- [ ] **Step 4: Implement repository transitions**

In `flow_runs.rs`, implement `create_flow_run`, `create_flow_device_run`,
`create_flow_attempt`, `list_flow_runs`, `get_flow_run`, `transition_attempt`,
`record_nonterminal_attempt_error`,
`publish_artifact_and_succeed`, `record_retry_safe_reconciliation`,
`mark_device_terminal`, `recompute_run_projection`, and `load_nonterminal_attempts`.
Each public mutation opens and closes its own
transaction; reads open a bounded connection. `transition_attempt` uses
`UPDATE ... WHERE state = ?` and returns `StateConflict` when zero rows change.

Use these exact mutation inputs and repository signatures:

```rust
#[derive(Debug, Clone, Default)]
pub struct AttemptTransitionPatch {
    pub canonical_input: Option<serde_json::Value>,
    pub evidence_baseline: Option<serde_json::Value>,
    pub evidence_result: Option<serde_json::Value>,
    pub error: Option<FlowErrorRecord>,
}
```

Implement the following as inherent `Database` methods with real bodies; this is a
signature contract, not a semicolon-only Rust `impl`:

```text
    pub fn create_flow_run(
        &self,
        revision: &FlowRevisionRecord,
        selection: FlowSelectionSnapshot,
    ) -> anyhow::Result<FlowRunRecord>;
    pub fn create_flow_device_run(
        &self,
        run_id: uuid::Uuid,
        udid: &str,
    ) -> anyhow::Result<FlowDeviceRunRecord>;
    pub fn create_flow_attempt(
        &self,
        device_run_id: uuid::Uuid,
        node: &CompiledFlowNode,
        side_effect_class: SideEffectClass,
        attempt_no: u32,
    ) -> anyhow::Result<FlowNodeAttemptRecord>;
    pub fn list_flow_runs(&self, limit: usize) -> anyhow::Result<Vec<FlowRunRecord>>;
    pub fn get_flow_run(&self, run_id: uuid::Uuid) -> anyhow::Result<Option<FlowRunDetail>>;
    pub fn transition_attempt(
        &self,
        attempt_id: uuid::Uuid,
        expected: FlowAttemptState,
        next: FlowAttemptState,
        patch: AttemptTransitionPatch,
    ) -> anyhow::Result<FlowNodeAttemptRecord>;
    pub fn record_nonterminal_attempt_error(
        &self,
        attempt_id: uuid::Uuid,
        expected: FlowAttemptState,
        error: FlowErrorRecord,
    ) -> anyhow::Result<FlowNodeAttemptRecord>;
    pub fn publish_artifact_and_succeed(
        &self,
        attempt_id: uuid::Uuid,
        artifact: &FlowArtifactRecord,
    ) -> anyhow::Result<FlowNodeAttemptRecord>;
    pub fn record_retry_safe_reconciliation(
        &self,
        attempt_id: uuid::Uuid,
        evidence_result: serde_json::Value,
    ) -> anyhow::Result<FlowNodeAttemptRecord>;
    pub fn mark_device_terminal(
        &self,
        device_run_id: uuid::Uuid,
        expected: &[FlowDeviceRunState],
        next: FlowDeviceRunState,
        error: Option<FlowErrorRecord>,
        release_proof: FlowContextReleaseProof,
    ) -> anyhow::Result<FlowDeviceRunRecord>;
    pub fn recompute_run_projection(
        &self,
        run_id: uuid::Uuid,
    ) -> anyhow::Result<FlowRunRecord>;
    pub fn load_nonterminal_attempts(&self) -> anyhow::Result<Vec<FlowNodeAttemptRecord>>;
```

Validate list limits as 1..=200. Every mutating transaction increments the parent
run event_revision and inserts its flow_events row before commit. Implement
append_flow_event as a private helper receiving the current rusqlite Transaction;
never publish state and its durable invalidation in separate transactions.
Generic transitions never write retry_safe. record_retry_safe_reconciliation accepts
only an idempotentSet attempt already in failedVerified, persists the re-read proof,
sets retry_safe=1, and appends the event in the same transaction. Artifact publication
requires artifact.attempt_id to equal its argument and Verifying to be the current state.
`record_nonterminal_attempt_error` performs a guarded same-state update only for
`IntentCommitted`, `EffectDispatched`, or `Verifying`, increments the event revision,
and never makes a retry decision. It exists so an infrastructure failure can be
recorded before the reconciler classifies the durable side effect.

The only legal side-effect path is:

```rust
fn legal_transition(from: FlowAttemptState, to: FlowAttemptState) -> bool {
    use FlowAttemptState::*;
    matches!(
        (from, to),
        (Queued, IntentCommitted)
            | (Queued, Cancelled)
            | (IntentCommitted, EffectDispatched)
            | (IntentCommitted, FailedBeforeDispatch)
            | (IntentCommitted, Cancelled)
            | (EffectDispatched, Verifying)
            | (EffectDispatched, FailedBeforeDispatch)
            | (EffectDispatched, Uncertain)
            | (EffectDispatched, Interrupted)
            | (EffectDispatched, Cancelled)
            | (Verifying, Succeeded)
            | (Verifying, FailedVerified)
            | (Verifying, Uncertain)
            | (Verifying, Interrupted)
            | (Verifying, Cancelled)
            | (Queued, Interrupted)
            | (Interrupted, Queued)
    )
}
```

`transition_attempt` additionally receives `SideEffectClass`. It permits
`Interrupted` from dispatched/verifying states only for `SideEffectClass::None`;
it likewise permits `Cancelled` from those states only for `SideEffectClass::None`;
otherwise cancellation or restart must persist `Uncertain`. It permits
`FailedBeforeDispatch` after dispatch only when the classified transport proof says
the request never reached the device.

`recompute_run_projection` uses these exact rules: `Succeeded` when at least one
device succeeded and every other device is `Succeeded` or `Skipped`; `Partial` when
at least one succeeded and another failed/cancelled; `Cancelled` when every
non-skipped device cancelled; otherwise terminal `Failed`. `Skipped` never hides a
failure and a run with zero non-skipped devices is `Failed` with `NoEligibleDevice`.

- [ ] **Step 5: Run tests and commit**

```powershell
cargo fmt --all
cargo test -p riviu-core db::flow_runs -- --nocapture
git add crates/core/src/db crates/core/src/flow/model.rs
git commit -m "feat(flow): persist durable run and attempt states"
```

### Task 4: Replace False-Success App Termination

**Files:**
- Create: `sidecars/pymobiledevice3/test_app_control.py`
- Modify: `sidecars/pymobiledevice3/riviu_pmd.py`
- Modify: `crates/core/src/driver.rs`
- Modify: `crates/core/src/device_control.rs`
- Modify: `crates/core/src/job_queue.rs`
- Modify: `crates/core/src/flow/catalog.rs`
- Modify: `crates/core/src/flow/mod.rs`
- Modify: `crates/script-engine/src/flow.rs`
- Modify: `crates/ios-driver/src/pmd.rs`
- Modify: `crates/ios-driver/src/mock.rs`
- Test: `crates/core/src/device_control.rs`
- Test: `crates/core/src/job_queue.rs`
- Test: `crates/ios-driver/src/pmd.rs`

- [ ] **Step 1: Write Python tests for running and absent processes**

Use fake `DvtProvider` and `ProcessControl` modules. Assert a running bundle resolves PID 42, calls `kill(42)` once, polls to PID 0, and emits `{ok:true, oldPid:42, running:false}`. Assert an absent bundle emits `{ok:true, oldPid:null, running:false}` without calling kill. Assert a PID that remains present until the 5-second deadline exits nonzero with `ok:false`.

```python
class FakeProcessControl:
    def __init__(self, provider):
        self.provider = provider
        self.pids = [42, 0]
        self.killed = []

    async def __aenter__(self):
        return self

    async def __aexit__(self, exc_type, exc, tb):
        return False

    async def process_identifier_for_bundle_identifier(self, bundle_id):
        self.bundle_id = bundle_id
        return self.pids.pop(0)

    async def kill(self, pid):
        self.killed.append(pid)

def test_verified_terminate_kills_exact_pid_and_observes_absence(self):
    with app_control_modules(FakeProcessControl) as state:
        result = asyncio.run(riviu_pmd._terminate_app_verified("fixture", "com.fixture.app"))
    self.assertEqual(result, {
        "ok": True,
        "bundleId": "com.fixture.app",
        "oldPid": 42,
        "running": False,
    })
    self.assertEqual(state.process_control.killed, [42])
```

Define the fixture without touching the user's installed package:

```python
@contextlib.contextmanager
def app_control_modules(process_control_type):
    state = types.SimpleNamespace(process_control=None)

    class FakeLockdown:
        async def close(self):
            return None

    async def create_using_usbmux(serial):
        state.serial = serial
        return FakeLockdown()

    class FakeDvtProvider:
        def __init__(self, lockdown):
            self.lockdown = lockdown

        async def __aenter__(self):
            return self

        async def __aexit__(self, exc_type, exc, tb):
            return False

    def capture_process_control(provider):
        state.process_control = process_control_type(provider)
        return state.process_control

    lockdown_module = types.ModuleType("pymobiledevice3.lockdown")
    lockdown_module.create_using_usbmux = create_using_usbmux
    dvt_module = types.ModuleType(
        "pymobiledevice3.services.dvt.instruments.dvt_provider"
    )
    dvt_module.DvtProvider = FakeDvtProvider
    process_module = types.ModuleType(
        "pymobiledevice3.services.dvt.instruments.process_control"
    )
    process_module.ProcessControl = capture_process_control
    modules = {
        "pymobiledevice3.lockdown": lockdown_module,
        "pymobiledevice3.services.dvt.instruments.dvt_provider": dvt_module,
        "pymobiledevice3.services.dvt.instruments.process_control": process_module,
    }
    with mock.patch.object(riviu_pmd, "try_import", return_value=True), \
         mock.patch.dict(sys.modules, modules), \
         mock.patch.object(riviu_pmd, "TERMINATE_TIMEOUT_SECONDS", 0.02), \
         mock.patch.object(riviu_pmd, "TERMINATE_POLL_SECONDS", 0.001):
        yield state
```

Import `asyncio`, `contextlib`, `sys`, `types`, `unittest`, and
`unittest.mock as mock`. Put the test methods on `AppControlTests(unittest.TestCase)`.
Use separate fake subclasses whose PID sequences are `[42, 0]`, `[0]`, and an
unbounded `42`. Add one delayed fake for each awaited boundary: lockdown creation,
DVT enter, ProcessControl enter, initial PID lookup, kill, polling lookup, context
exit, and lockdown close. With a patched 20 ms operation deadline and 10 ms cleanup
deadline, every stuck test must finish in under 250 ms. Assert a cleanup fault does
not replace the primary operation error.

- [ ] **Step 2: Run tests red**

```powershell
python -m unittest sidecars.pymobiledevice3.test_app_control -v
```

Expected: FAIL because `cmd_terminate` returns the best-effort stub.

- [ ] **Step 3: Implement bounded DVT termination**

Add an async helper beside `_launch_app_with_environment`. One monotonic operation
deadline covers setup, lookup, termination, and absence polling. Every await receives
the remaining duration; entering an async context through plain `async with` is
forbidden here because its hidden `__aenter__` would be unbounded. Cleanup uses a
separate short deadline, always runs, and never replaces a primary error:

```python
async def _await_before(deadline: float, operation):
    remaining = deadline - asyncio.get_running_loop().time()
    if remaining <= 0:
        raise TimeoutError("terminate deadline expired")
    return await asyncio.wait_for(operation(), timeout=remaining)


async def _terminate_app_verified(udid: str, bundle_id: str) -> dict:
    if not try_import():
        raise RuntimeError("pymobiledevice3 not installed")
    from pymobiledevice3.lockdown import create_using_usbmux
    from pymobiledevice3.services.dvt.instruments.process_control import ProcessControl
    from pymobiledevice3.services.dvt.instruments.dvt_provider import DvtProvider

    loop = asyncio.get_running_loop()
    deadline = loop.time() + TERMINATE_TIMEOUT_SECONDS
    cleanup_deadline = None
    lockdown = None
    dvt_context = None
    process_context = None
    error_info = (None, None, None)
    try:
        lockdown = await _await_before(
            deadline, lambda: create_using_usbmux(serial=udid)
        )
        dvt_context = DvtProvider(lockdown)
        dvt = await _await_before(deadline, dvt_context.__aenter__)
        process_context = ProcessControl(dvt)
        process_control = await _await_before(deadline, process_context.__aenter__)
        pid = await _await_before(
            deadline,
            lambda: process_control.process_identifier_for_bundle_identifier(bundle_id),
        )
        if not pid:
            return {"ok": True, "bundleId": bundle_id, "oldPid": None, "running": False}
        await _await_before(deadline, lambda: process_control.kill(pid))
        while True:
            current = await _await_before(
                deadline,
                lambda: process_control.process_identifier_for_bundle_identifier(bundle_id),
            )
            if not current:
                return {"ok": True, "bundleId": bundle_id, "oldPid": pid, "running": False}
            await _await_before(
                deadline,
                lambda: asyncio.sleep(TERMINATE_POLL_SECONDS),
            )
    except BaseException as error:
        error_info = (type(error), error, error.__traceback__)
        raise
    finally:
        cleanup_deadline = loop.time() + TERMINATE_CLEANUP_TIMEOUT_SECONDS
        cleanup_errors = []
        for operation in [
            None if process_context is None else
                lambda: process_context.__aexit__(*error_info),
            None if dvt_context is None else
                lambda: dvt_context.__aexit__(*error_info),
            None if lockdown is None else lockdown.close,
        ]:
            if operation is None:
                continue
            try:
                await _await_before(cleanup_deadline, operation)
            except BaseException as cleanup_error:
                cleanup_errors.append(cleanup_error)
        if cleanup_errors and error_info[1] is None:
            raise cleanup_errors[0]
        if cleanup_errors:
            print(f"terminate cleanup error: {cleanup_errors[0]}", file=sys.stderr)
```

Define `TERMINATE_TIMEOUT_SECONDS = 5.0`, `TERMINATE_CLEANUP_TIMEOUT_SECONDS =
0.5`, and `TERMINATE_POLL_SECONDS = 0.1` beside the helper. `cmd_terminate` uses
those production defaults.

`cmd_terminate` must `asyncio.run` this helper, emit its result, and emit `{ok:false,error}` plus return 1 on exception.

- [ ] **Step 4: Make Rust require verified output and shared ownership**

Change `DeviceDriver::terminate_app` to return a typed `ProcessAbsenceProof` containing
the exact bundle ID and optional old PID. `PmdIosDriver::terminate_app` retains the
`run_json` value and requires `ok == true`, `running == false`, and an exact matching
`bundleId`; missing/mismatched fields are protocol errors. Add a fixture sidecar test
that returns the old best-effort payload and assert Rust rejects it.

Add `DeviceDriver::supports_verified_app_termination() -> bool` with a default of
false. `PmdIosDriver` returns true only with this corrected protocol, and the mock is
explicitly configurable. `DeviceControlPlane::driver_contract_ids()` maps true to
the stable internal contract ID `verifiedProcessControl`; Task 6 uses that set when
building static Flow capabilities instead of assuming every driver can terminate.

```rust
let value = self
    .run_json(&["terminate", "--udid", udid, "--bundle-id", bundle_id])
    .await?;
let ok = value.get("ok").and_then(serde_json::Value::as_bool);
let running = value.get("running").and_then(serde_json::Value::as_bool);
let observed_bundle = value.get("bundleId").and_then(serde_json::Value::as_str);
match (ok, running, observed_bundle) {
    (Some(true), Some(false), Some(observed)) if observed == bundle_id =>
        Ok(ProcessAbsenceProof {
            bundle_id: observed.to_string(),
            old_pid: value.get("oldPid").and_then(serde_json::Value::as_u64),
        }),
    _ => anyhow::bail!("terminate sidecar omitted verified process absence"),
}
```

Both `DeviceControlPlane::terminate_app(&DeviceExclusiveContext, ...)` and
`terminate_session_app(&UiSessionContext, ...)` return this proof after validating
their owned context. Flow calls the exclusive variant for its bridge-only Terminate
node; the legacy JobQueue keeps using the session variant. No caller may invoke the
sidecar directly. Add a concurrency test that starts legacy Terminate and Flow
Terminate for the same UDID and proves only one enters the recording driver until
the first owned context is released; different UDIDs may proceed independently.

Add a read-only `app-process` sidecar command using the same bounded setup/cleanup
helper. It returns `{ok:true,bundleId,pid,running}` and never calls `kill`.
`DeviceDriver::inspect_app_process` and ownership-checked control-plane methods map
it to `AppProcessState`. This is the only `ReadProcess` reconciliation route: after
a crash at `EffectDispatched`, the runtime compares it with the PID persisted in the
pre-effect baseline. Absent proves `ProcessAbsent`; the exact same positive PID
proves non-delivery and may mark retry-safe; a different positive PID is
`Uncertain`. It does not redispatch Terminate while reconciling.

- [ ] **Step 5: Enable the typed Terminate action**

Add the Task F0 model's Terminate definition to `release_one_catalog()` with
`disabled_reason: None`, config `{bundleId}`, capability `app.terminate`,
`ResourceClass::Bridge`, `SideEffectClass::IdempotentSet`,
`EvidenceRequirement::Process`, `EvidenceKind::ProcessAbsent`,
`ReconciliationPolicy::ReadProcess`, and `RetryPolicy::IdempotentAfterRead`.
Update the compiler's F0 feature gate to accept only this now-catalogued definition
and compile `TerminateApp { bundle_id }`. Runtime Task 6 dispatches it through the
exclusive context and persists the returned `ProcessAbsent` evidence. Add compiler
and catalog tests proving raw actions remain excluded and Terminate is now enabled.
Replace F0's temporary `catalog_never_exposes_raw_transport_actions_or_terminate`
assertion with two final assertions: raw kinds remain absent, and Terminate has the
exact process evidence/reconciliation contract. Do not leave an F0-stage test that
must fail in the final workspace.

Update `import_legacy_v1` in the same `flow.rs`: replace F0's temporary Terminate
diagnostic test and match arm with a semantics-preserving mapping from
`ScriptAction::TerminateApp { bundle_id }` to typed Terminate config plus
`EvidenceSpec::ProcessAbsent { bundle_id }`. Keep the original v1 JSON untouched;
all other unsupported diagnostics stay unchanged.

- [ ] **Step 6: Run tests and commit**

```powershell
python -m unittest sidecars.pymobiledevice3.test_app_control -v
cargo test -p riviu-ios-driver pmd -- --nocapture
cargo test -p riviu-core device_control job_queue -- --nocapture
cargo test -p riviu-script-engine flow -- --nocapture
git add sidecars/pymobiledevice3/riviu_pmd.py sidecars/pymobiledevice3/test_app_control.py crates/core/src/driver.rs crates/core/src/device_control.rs crates/core/src/job_queue.rs crates/core/src/flow/catalog.rs crates/core/src/flow/mod.rs crates/script-engine/src/flow.rs crates/ios-driver/src/pmd.rs crates/ios-driver/src/mock.rs
git commit -m "fix(device): verify app termination through DVT"
```

### Task 5: Implement Typed Evidence Verification

**Files:**
- Create: `crates/core/src/flow/cancellation.rs`
- Create: `crates/core/src/flow/evidence.rs`
- Modify: `crates/core/src/flow/mod.rs`
- Modify: `crates/core/src/driver.rs`
- Modify: `crates/ios-driver/src/wda.rs`
- Modify: `crates/ios-driver/src/pmd.rs`
- Modify: `crates/ios-driver/src/mock.rs`
- Test: `crates/core/src/flow/evidence.rs`

- [ ] **Step 1: Write evidence tests**

Test frame region delta with the repository JPEG fixtures, stale generation
rejection, active-app equality, Terminate `ProcessAbsent` proof with exact bundle,
artifact decode/hash, accessibility visibility, exact Unicode text read-back, and
injected WDA ACK with unchanged frame returning `EvidenceMismatch`.

```rust
#[tokio::test]
async fn gesture_ack_without_matching_frame_evidence_is_not_success() {
    let frames = TestGenerationFrames::single_generation(7, fixture_jpeg());
    let cancellation = FlowCancellation::default();
    let baseline = capture_baseline(
        &frames,
        "fixture",
        7,
        &EvidenceSpec::FrameRegionChanged {
            x: 0, y: 0, width: 20, height: 20, minimum_distance: 8,
        },
        tokio::time::Instant::now() + std::time::Duration::from_secs(1),
        &cancellation,
    ).await.expect("baseline");
    frames.publish(7, fixture_jpeg());
    let error = verify_frame_postcondition(
        &frames,
        "fixture",
        7,
        &baseline,
        tokio::time::Instant::now() + std::time::Duration::from_secs(1),
        &cancellation,
    )
        .await.expect_err("unchanged frame must fail");
    assert_eq!(error.code(), "EvidenceMismatch");
}
```

- [ ] **Step 2: Run tests red**

```powershell
cargo test -p riviu-core flow::evidence -- --nocapture
```

Expected: FAIL because evidence capture/verifiers are absent.

- [ ] **Step 3: Add qualified read-back methods**

Add default-unsupported methods to `UiSession`:

```rust
async fn read_text(
    &self,
    _locator: &QualifiedElementLocator,
    _request_timeout: std::time::Duration,
) -> anyhow::Result<String> {
    unsupported("readText")
}

fn supports_accessibility_readback(&self) -> bool {
    false
}
```

Implement them in the mock. In `WdaClient`, resolve exactly one element with POST
`/session/{sid}/element` using only `"accessibility id"` or `"class name"` mapped
from the typed enum plus its value, extract
the W3C or legacy element ID, then GET
`/session/{sid}/element/{elementId}/text` and require a string `value`. Add request
contract tests for both calls. `PmdUiSession` advertises this method only when it
was created as `FreshText` on `WdaBackend::RtMmo`; stock remains false and its
snapshot depth remains 1. The runtime rejects `accessibility.readText` and
`accessibility.visible` before dispatch when the live session flag is false.
For WDA read-back, recompute the remaining verifier duration before element lookup
and again before GET text, then pass that value to `WdaClient::send`; the deadline
stays on each request.

- [ ] **Step 4: Implement baseline and verifier types**

First define the shared cancellation primitive in `flow/cancellation.rs`; Task 6 and
Task 7 reuse it rather than defining another token:

```rust
#[derive(Clone, Default)]
pub(crate) struct FlowCancellation {
    cancelled: std::sync::Arc<std::sync::atomic::AtomicBool>,
    notify: std::sync::Arc<tokio::sync::Notify>,
}

impl FlowCancellation {
    pub(crate) fn cancel(&self) {
        self.cancelled.store(true, std::sync::atomic::Ordering::Release);
        self.notify.notify_waiters();
    }

    pub(crate) fn is_cancelled(&self) -> bool {
        self.cancelled.load(std::sync::atomic::Ordering::Acquire)
    }

    pub(crate) async fn cancelled(&self) {
        loop {
            let changed = self.notify.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            if self.is_cancelled() {
                return;
            }
            changed.await;
        }
    }
}
```

Export it only within `riviu-core::flow`. Then define `EvidenceBaseline`,
`EvidenceResult`, and `EvidenceError`.
`capture_baseline` must require a `GenerationFrameSource` frame matching the
context generation for Tap/Swipe/Type. Decode images and compute mean absolute
luma delta for the configured region. `verify` dispatches only known
`EvidenceSpec` variants. `TextReadBackEquals` uses both its typed locator and the
exact expected value from the F0 model. Reject read-back when the session does not
advertise the qualified accessibility contract. Both `capture_baseline` and every
postcondition verifier receive an absolute deadline plus `FlowCancellation`; all
frame waits use one `tokio::select!` over `stream.next()`, cancellation, and
`sleep_until(deadline)`. Map `GenerationFrameEvent::Advanced` immediately to
`StaleGeneration`, `Closed` to `StreamClosed`, and never wait for a replacement
generation. Device-backed verifiers check cancellation/deadline immediately before
and after each request. New read-back calls pass the remaining duration into the WDA
request; existing active-app calls retain their shorter request-local deadline. No
verifier wraps and cancels an in-flight WDA request with `tokio::time::timeout`.
`ProcessAbsent` validates the action's `ProcessAbsenceProof` or the read-only
`AppProcessState` from reconciliation against the exact configured bundle; a
sidecar success boolean without that typed identity is a mismatch. Before Terminate
commits intent, `capture_baseline` reads and persists `EvidenceBaseline::Process`.
During normal verification, require the returned proof's bundle and `old_pid` to
match that baseline (including the already-absent `None` case).

```rust
pub enum EvidenceBaseline {
    None,
    Process {
        bundle_id: String,
        pid: Option<u64>,
    },
    Frame {
        generation: u64,
        jpeg_sha256: String,
        image: image::RgbImage,
    },
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EvidenceResult {
    pub kind: EvidenceKind,
    pub matched: bool,
    pub observed_sha256: String,
    pub measurement: serde_json::Value,
}

#[derive(Debug, thiserror::Error)]
pub enum EvidenceError {
    #[error("frame generation changed")]
    StaleGeneration,
    #[error("frame stream closed")]
    StreamClosed,
    #[error("evidence deadline expired")]
    Timeout,
    #[error("evidence verification was cancelled")]
    Cancelled,
    #[error("evidence did not match")]
    Mismatch,
    #[error("evidence capability is unavailable: {0}")]
    Unsupported(&'static str),
    #[error("evidence input is invalid: {0}")]
    Invalid(String),
}

impl EvidenceError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::StaleGeneration => "StaleGeneration",
            Self::StreamClosed => "StreamClosed",
            Self::Timeout => "EvidenceTimeout",
            Self::Cancelled => "Cancelled",
            Self::Mismatch => "EvidenceMismatch",
            Self::Unsupported(_) => "EvidenceUnsupported",
            Self::Invalid(_) => "EvidenceInvalid",
        }
    }
}

pub async fn verify_postcondition(
    source: &dyn GenerationFrameSource,
    session: Option<&dyn UiSession>,
    udid: &str,
    generation: u64,
    specification: &EvidenceSpec,
    baseline: &EvidenceBaseline,
    deadline: tokio::time::Instant,
    cancellation: &FlowCancellation,
) -> Result<EvidenceResult, EvidenceError>;
```

The optional session is `None` only for bridge/pure-desktop evidence such as
`ProcessAbsent` and artifact reconciliation. A verifier that needs active-app,
accessibility, text, or frames returns `EvidenceUnsupported` when its owned context
does not supply the required session/stream; it never creates one implicitly.

Add a deterministic test for each exit path: matching frame, generation advance,
closed stream, deadline, and cancellation. Each test must complete within its own
outer 250 ms test timeout so a verifier regression cannot hang `shutdown()`.

- [ ] **Step 5: Run tests and commit**

```powershell
cargo fmt --all
cargo test -p riviu-core flow::evidence -- --nocapture
cargo test -p riviu-ios-driver wda -- --nocapture
cargo test -p riviu-ios-driver mock -- --nocapture
git add crates/core/src/flow crates/core/src/driver.rs crates/ios-driver/src/wda.rs crates/ios-driver/src/pmd.rs crates/ios-driver/src/mock.rs crates/script-engine/src/flow.rs
git commit -m "feat(flow): verify actions through typed evidence"
```

### Task 6: Build Monotonic Per-Device Execution

**Files:**
- Create: `crates/core/src/flow/device_context.rs`
- Create: `crates/core/src/flow/executor.rs`
- Modify: `crates/core/src/flow/mod.rs`
- Modify: `crates/core/src/driver.rs`
- Modify: `crates/core/src/device_control.rs`
- Test: `crates/core/src/flow/executor.rs`

- [ ] **Step 1: Write typestate and ACK-without-evidence tests**

Define a local `RecordingFlowDriver` in `flow/executor.rs`'s test module that
implements `DeviceDriver`; core tests must not import `riviu-ios-driver` and create a
dependency cycle. Test Exclusive -> Session -> Stream upgrade order, fresh-text
session selection, stream capacity reserved before session, MJPEG after session, no
reacquire between nodes, exactly one launch call, one close on success/failure/
cancel, Launch active-app proof, bridge-only Terminate process-absence proof,
Screenshot from stream rather than `screenshot_png`, and Tap ACK with unchanged
frame ending `FailedVerified` rather than `Succeeded`.

```rust
#[tokio::test]
async fn text_flow_upgrades_once_and_starts_stream_after_fresh_session() {
    let fixture = executor_fixture_with_text_flow();
    fixture.executor.run_device(fixture.device_run_id, fixture.plan.clone())
        .await.expect("device run");
    assert_eq!(
        fixture.driver.operations(),
        vec![
            "park", "reserveStream", "launch:com.apple.Preferences",
            "session:freshText", "startStream", "typeText", "readText",
            "stopStream", "closeSession", "release",
        ]
    );
    let replacement = fixture.work.try_acquire(
        "fixture",
        DeviceWorkOwner::Repair,
    ).expect("flow released the device");
    drop(replacement);
}
```

- [ ] **Step 2: Run tests red**

```powershell
cargo test -p riviu-core flow::executor -- --nocapture
```

Expected: FAIL because Flow device context/executor are absent.

- [ ] **Step 3: Add explicit context close support**

Add this generic control-plane proof and return it from `close_exclusive_context`
and `close_session_context`:

```rust
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ContextReleaseProof {
    pub udid: String,
    pub owner: DeviceWorkOwner,
    pub had_session: bool,
    pub had_stream: bool,
}
```

Keep `close_ui_context` as the only streaming close path and map its existing
`DeviceReleaseProof` into `ContextReleaseProof`. The Flow executor converts that
generic value into Task 3's persisted `FlowContextReleaseProof`. Preserve drop
cleanup as the panic/cancellation backstop.

- [ ] **Step 4: Add target-qualified capability inspection**

Add this typed driver method without changing the existing Interaction wrapper:

```rust
async fn inspect_device_for_target(
    &self,
    _udid: &str,
    _target_bundle_id: &str,
) -> anyhow::Result<DeviceCapabilitySnapshot> {
    unsupported("inspectDeviceForTarget")
}
```

`PmdIosDriver` must call the existing sidecar `inspect-device-capabilities` with
the supplied target bundle and parse the returned installed target against that
same value. Its existing `inspect_interaction_device` becomes a thin call to
`inspect_device_for_target(udid, INTERACTION_TARGET_BUNDLE_ID)`. The mock returns
a fixture snapshot containing the requested target bundle.

Expose only this ownership-checked control-plane method to Flow:

```rust
pub async fn inspect_flow_device(
    &self,
    context: &DeviceExclusiveContext,
    target_bundle_id: &str,
) -> Result<DeviceCapabilitySnapshot, DeviceControlError>;
```

It validates a non-empty bundle ID and the exclusive context, then calls the driver.
It does not clear or apply `UiCapabilities`; those remain Interaction-owned. Derive
the coordinate profile only through `qualified_geometry_profile_id`.

Build a `FlowDevicePreflight` from this snapshot plus `cached_agent_status`.
For UI plans, require `state=Ready`, protected auth, matching installed target
bundle, non-empty Agent identity, and every static capability. Map manifest features
`stream`, `tap`, `swipe`, and `text` to the same Flow IDs; add `app.launch` and
`ui.home` only after the protected exact-target inspection succeeds. Add
`app.terminate` only when the DVT adapter reports the verified process-control
contract installed by Task 4. A bridge-only Terminate preflight does not require an
Agent session or initial Launch, but it still requires the exact target to be
installed and `app.terminate` present. Defer `accessibility.visible` and
`accessibility.readText` until the live session flags from Task 5 are available.

```rust
struct FlowDevicePreflight {
    snapshot: DeviceCapabilitySnapshot,
    agent_status: Option<AgentStatus>,
    profile_id: String,
    capability_ids: std::collections::BTreeSet<String>,
}

fn static_flow_capability_ids(
    agent: Option<&AgentStatus>,
    driver_contracts: &std::collections::BTreeSet<String>,
) -> std::collections::BTreeSet<String> {
    let features: std::collections::BTreeSet<&str> =
        agent.into_iter().flat_map(|value| value.features.iter().map(String::as_str)).collect();
    let mut ids = std::collections::BTreeSet::new();
    if agent.is_some() {
        ids.extend(["app.launch".to_string(), "ui.home".to_string()]);
    }
    if driver_contracts.contains("verifiedProcessControl") {
        ids.insert("app.terminate".to_string());
    }
    for (feature, capability) in [
        ("stream", "stream"),
        ("tap", "ui.tap"),
        ("swipe", "ui.swipe"),
        ("text", "ui.text"),
    ] {
        if features.contains(feature) {
            ids.insert(capability.to_string());
        }
    }
    ids
}
```

When comparing with `plan.required_capabilities`, exclude only the two deferred
accessibility IDs from this first comparison. Check each immediately after session
creation and before the first node requiring that capability is moved to
`IntentCommitted`.

- [ ] **Step 5: Implement monotonic context owner**

```rust
enum FlowDeviceContext {
    Exclusive(DeviceExclusiveContext),
    Session(UiSessionContext),
    Streaming(UiWithStreamContext),
    Closed,
}

impl FlowDeviceContext {
    fn level(&self) -> u8 {
        match self {
            Self::Exclusive(_) => 1,
            Self::Session(_) => 2,
            Self::Streaming(_) => 3,
            Self::Closed => 4,
        }
    }
}

```

Use Task 5's single `FlowCancellation`; do not create an executor-local flag.

Every upgrade consumes the previous variant. Do not foreground an app inside a
generic context-upgrade helper: the first Launch App is a real durable node attempt,
not setup outside the attempt state machine. For any UI-session plan, reserve stream
capacity while Exclusive when required, but leave the context Exclusive until that
Launch attempt has persisted `EffectDispatched` and invoked
`foreground_target_app` exactly once. Then consume it through
`start_interaction_session` using `InteractionSessionKind::FreshText` when text is
required, otherwise Ordinary. Only after session creation succeeds may
`start_reserved_stream` run. Do not implement downgrade.

- [ ] **Step 6: Implement action execution order**

For a UI-session plan, call `inspect_flow_device` under Exclusive with the compiled
`initial_bundle_id` before any application effect; persist the snapshot and static
capability IDs on the device run. For a bridge-only Terminate plan, preflight the
bundle carried by that node without inventing an initial Launch target. For each
ordinary node: persist `IntentCommitted` with baseline, persist `EffectDispatched`,
invoke the typed action, persist `Verifying`, verify postcondition, then persist
terminal state.

The first Launch of a UI plan is consumed by one special instance of that same
durable sequence: capture its baseline; persist `IntentCommitted`; persist
`EffectDispatched`; call `foreground_target_app` once; create the ordinary/fresh
session; start the reserved stream only if required; persist `Verifying`; verify the
active app through the new session; persist `Succeeded`; then continue iteration at
the node after Launch. It must not enter the generic dispatcher again. If session or
stream creation fails, record the typed diagnostic while the Launch attempt remains
`EffectDispatched`, then invoke the same bounded `ReadActiveApp` reconciler used at
startup. It proves success/non-delivery or marks `Uncertain`; a crash before that
classification leaves the durable state for startup recovery. Never blindly dispatch
a second launch. Check deferred accessibility/read-back capabilities after session
creation and before the first node that requires them reaches `IntentCommitted`.

Later Launch nodes foreground through the session/stream method matching the current
context and verify active app. Terminate dispatches through the exclusive/session/
stream owned method matching the current monotonic context and produces
`ProcessAbsent`; it never bypasses `DeviceControlPlane`. Before Tap/Swipe, compare
the compiled image
dimensions/orientation/profile with the runtime frame/profile and fail
`GeometryMismatch` before dispatch. Execute matching coordinates through
`tap_image`/`swipe_image`. Screenshot reads `latest_in_generation`, then uses Task
2's artifact protocol, rechecks `latest_in_generation` for the same generation
before the DB transaction, and removes the prepared file with `StaleGeneration` if
the generation was invalidated. Task 3 then performs atomic DB publication. Wait
sleeps in <=250 ms
cancellation slices. Assert Visible is read-only and available only when the
session reports read-back support.

Dispatch only the compiler-owned enum; a kind/config mismatch is persisted as
`CompiledPlanCorrupt` without a device call:

```rust
enum ActionOutput {
    None,
    ProcessAbsent(ProcessAbsenceProof),
    Screenshot { label: String, format: String },
}

#[derive(Debug, thiserror::Error)]
enum FlowExecutionError {
    #[error("compiled action config does not match node {node_id}")]
    CompiledPlanCorrupt { node_id: NodeId },
    #[error("flow was cancelled")]
    Cancelled,
    #[error(transparent)]
    Device(#[from] DeviceControlError),
    #[error(transparent)]
    Evidence(#[from] EvidenceError),
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

async fn cancellable_wait(
    duration_ms: u64,
    cancellation: &FlowCancellation,
) -> Result<(), FlowExecutionError> {
    let deadline = tokio::time::Instant::now()
        + std::time::Duration::from_millis(duration_ms);
    while tokio::time::Instant::now() < deadline {
        if cancellation.is_cancelled() {
            return Err(FlowExecutionError::Cancelled);
        }
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        let slice = remaining.min(std::time::Duration::from_millis(250));
        tokio::select! {
            _ = tokio::time::sleep(slice) => {}
            _ = cancellation.cancelled() => {}
        }
    }
    if cancellation.is_cancelled() {
        Err(FlowExecutionError::Cancelled)
    } else {
        Ok(())
    }
}

match (&node.kind, &node.config) {
    (ActionKind::Start | ActionKind::End, CompiledActionConfig::Empty) => Ok(ActionOutput::None),
    (ActionKind::LaunchApp, CompiledActionConfig::LaunchApp { bundle_id }) => {
        context.foreground_app(control, bundle_id).await?;
        Ok(ActionOutput::None)
    }
    (ActionKind::TerminateApp, CompiledActionConfig::TerminateApp { bundle_id }) => {
        let proof = context.terminate_app(control, bundle_id).await?;
        Ok(ActionOutput::ProcessAbsent(proof))
    }
    (ActionKind::Wait, CompiledActionConfig::Wait { duration_ms }) => {
        cancellable_wait(*duration_ms, cancellation).await?;
        Ok(ActionOutput::None)
    }
    (ActionKind::Tap, CompiledActionConfig::Tap { target }) => {
        match target {
            CompiledTapTarget::Point { target } =>
                context.session(control)?.tap_image(
                    target.x, target.y,
                    f64::from(target.image_width), f64::from(target.image_height),
                ).await?,
            CompiledTapTarget::AccessibilityId { value } =>
                context.session(control)?.find_and_tap(value).await?,
        }
        Ok(ActionOutput::None)
    }
    (ActionKind::Swipe, CompiledActionConfig::Swipe { from, to, duration_ms }) => {
        context.session(control)?.swipe_image(
            TapPoint { x: from.x, y: from.y },
            TapPoint { x: to.x, y: to.y },
            f64::from(from.image_width), f64::from(from.image_height),
            *duration_ms,
        ).await?;
        Ok(ActionOutput::None)
    }
    (ActionKind::TypeText, CompiledActionConfig::TypeText { text, .. }) => {
        context.session(control)?.type_text(text).await?;
        Ok(ActionOutput::None)
    }
    (ActionKind::Screenshot, CompiledActionConfig::Screenshot { label, format }) =>
        Ok(ActionOutput::Screenshot { label: label.clone(), format: format.clone() }),
    (ActionKind::Home, CompiledActionConfig::Empty) => {
        context.session(control)?.home().await?;
        Ok(ActionOutput::None)
    }
    (ActionKind::AssertVisible, CompiledActionConfig::AssertVisible { accessibility_id }) => {
        context.session(control)?.assert_visible(accessibility_id).await?;
        Ok(ActionOutput::None)
    }
    _ => Err(FlowExecutionError::CompiledPlanCorrupt { node_id: node.id }),
}
```

- [ ] **Step 7: Run tests and commit**

```powershell
cargo fmt --all
cargo test -p riviu-core flow::executor -- --nocapture
cargo test -p riviu-core device_control -- --nocapture
git add crates/core/src/flow crates/core/src/driver.rs crates/core/src/device_control.rs
git commit -m "feat(flow): execute nodes through monotonic device contexts"
```

### Task 7: Add Multi-Device Runtime, Cancellation, Recovery, And Shutdown

**Files:**
- Create: `crates/core/src/flow/runtime.rs`
- Modify: `crates/core/src/flow/mod.rs`
- Modify: `crates/core/src/events.rs`
- Test: `crates/core/src/flow/runtime.rs`

- [ ] **Step 1: Write runtime tests**

Test One failure, Selected partial, AllEligible skipped/zero eligible, two-device
independent histories, no A lease while waiting for B, cancellation during acquire/
Wait/in-flight effect, recovery of every nonterminal attempt, prohibited retry for
Uncertain Tap/Swipe/Type, eligible retry for FailedBeforeDispatch, generation
advance during evidence, and `shutdown()` joining all workers within a fixed bound.

```rust
#[tokio::test]
async fn selected_devices_keep_independent_attempt_histories() {
    let fixture = runtime_fixture(&["iphone-a", "iphone-b"]);
    fixture.driver.fail_tap_for("iphone-a");
    let run = fixture.runtime.enqueue(
        fixture.revision.clone(),
        FlowTargetSelection::Selected {
            udids: vec!["iphone-a".into(), "iphone-b".into()],
        },
    ).await.expect("enqueue");
    fixture.runtime.wait_terminal(run.id).await.expect("terminal");
    let detail = fixture.database.get_flow_run(run.id).expect("load").expect("run");
    assert_eq!(detail.device_runs.len(), 2);
    assert_ne!(detail.device_runs[0].state, detail.device_runs[1].state);
    assert!(detail.attempts.iter().all(|attempt| attempt.device_run_id != uuid::Uuid::nil()));
    fixture.runtime.shutdown().await.expect("shutdown");
    fixture.control.shutdown_cleanup().await.expect("control cleanup");
}
```

Define the test-only waiter used above:

```rust
#[cfg(test)]
impl FlowRuntime {
    async fn wait_terminal(&self, run_id: uuid::Uuid) -> anyhow::Result<FlowRunDetail> {
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            if let Some(detail) = self.inner.database.get_flow_run(run_id)? {
                if detail.run.state.is_terminal() {
                    return Ok(detail);
                }
            }
            if tokio::time::Instant::now() >= deadline {
                anyhow::bail!("flow run did not become terminal before test deadline");
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    }
}
```

- [ ] **Step 2: Run tests red**

```powershell
cargo test -p riviu-core flow::runtime -- --nocapture
```

Expected: FAIL because `FlowRuntime` is absent.

- [ ] **Step 3: Implement runtime ownership**

Define `FlowRuntime` with `Arc<Database>`, `EventBus`, `DeviceRegistry`, `Arc<DeviceControlPlane>`, `Arc<dyn GenerationFrameSource>`, `FlowArtifactStore`, cancellation set/Notify, task map, stopping flag, and shutdown mutex. Mirror `JobQueue::stop_all`/`shutdown`, but create one task per run and one joined child per selected device. Each child acquires only its own UDID.

```rust
#[derive(Clone)]
pub struct FlowRuntime {
    inner: std::sync::Arc<FlowRuntimeInner>,
}

struct FlowRuntimeInner {
    database: std::sync::Arc<Database>,
    events: EventBus,
    registry: DeviceRegistry,
    control: std::sync::Arc<DeviceControlPlane>,
    frames: std::sync::Arc<dyn GenerationFrameSource>,
    artifacts: FlowArtifactStore,
    cancellations: parking_lot::Mutex<std::collections::HashMap<uuid::Uuid, FlowCancellation>>,
    tasks: tokio::sync::Mutex<std::collections::HashMap<
        uuid::Uuid,
        tokio::task::JoinHandle<anyhow::Result<()>>,
    >>,
    stopping: std::sync::atomic::AtomicBool,
    shutdown_lock: tokio::sync::Mutex<()>,
}

pub struct FlowRuntimeDeps {
    pub database: std::sync::Arc<Database>,
    pub events: EventBus,
    pub registry: DeviceRegistry,
    pub control: std::sync::Arc<DeviceControlPlane>,
    pub frames: std::sync::Arc<dyn GenerationFrameSource>,
    pub artifacts: FlowArtifactStore,
}

```

Implement inherent methods with these signatures: `new(FlowRuntimeDeps) -> Self`,
`recover_startup(&self) -> anyhow::Result<()>`, async
`enqueue(&self, FlowRevisionRecord, FlowTargetSelection) -> anyhow::Result<FlowRunRecord>`,
`cancel_run(&self, Uuid) -> anyhow::Result<()>`, async
`retry_attempt(&self, Uuid) -> anyhow::Result<FlowNodeAttemptRecord>`, `stop_all(&self)`,
and async `shutdown(&self) -> anyhow::Result<()>`.

`shutdown` takes `shutdown_lock`, calls `stop_all`, drains the task map, and awaits
every handle. All frame waits select cancellation/deadline, all WDA calls retain
request-local deadlines, and all sidecar process-control calls have the Task 4
deadline. Use a 30-second shutdown deadline, longer than any action request plus
cleanup. If that deadline expires, abort every still-running Tokio task, await each
aborted handle, record `ShutdownDeadlineExceeded`, and return the first error only
after no worker handle remains. This last-resort abort occurs only after the request
deadlines should already have fired; never wrap an individual WDA request in
`tokio::time::timeout`.

- [ ] **Step 4: Implement selection and retry semantics**

Resolve One/Selected from requested IDs and AllEligible from one `DeviceRegistry`
snapshot. Reject empty/duplicate Selected IDs and unknown requested IDs. AllEligible
includes only USB/Mock devices whose snapshot status is Connected, Ready, or Busy;
sort exact UDIDs lexicographically, persist them in `FlowSelectionSnapshot` before
spawning, and never add a device discovered later. One ineligible fails; Selected
records per-device failure and becomes Partial only when another device succeeds
(otherwise Failed); AllEligible records Skipped reasons
and fails when zero qualify. `retry_attempt` must call the action reconciler and
return `RetryNotAllowed` for Uncertain Tap/Swipe/Type.

An allowed retry inserts `attempt_no + 1` for the same device/node without changing
the old row, reacquires only that device, repeats exact-target preflight, and executes
the new attempt through the normal intent/evidence path. If it succeeds, continue
the remaining Queued successors in the same immutable plan; if it does not, leave
successors Queued and recompute the aggregate. Never retry the whole multi-device
run as a side effect of one device retry.

```rust
#[derive(Debug, thiserror::Error)]
enum FlowSelectionError {
    #[error("selected devices are empty")]
    Empty,
    #[error("a selected device is unknown")]
    UnknownDevice,
    #[error("selected devices contain a duplicate")]
    Duplicate,
    #[error("no eligible device exists")]
    NoEligibleDevice,
}

fn resolve_targets(
    registry: &DeviceRegistry,
    selection: &FlowTargetSelection,
) -> Result<Vec<String>, FlowSelectionError> {
    let devices = registry.list();
    let known: std::collections::BTreeSet<&str> =
        devices.iter().map(|device| device.udid.as_str()).collect();
    let mut targets = match selection {
        FlowTargetSelection::One { udid } => vec![udid.clone()],
        FlowTargetSelection::Selected { udids } if !udids.is_empty() => udids.clone(),
        FlowTargetSelection::Selected { .. } => return Err(FlowSelectionError::Empty),
        FlowTargetSelection::AllEligible => devices.into_iter()
            .filter(|device| matches!(device.connection, ConnectionKind::Usb | ConnectionKind::Mock))
            .filter(|device| matches!(
                device.status,
                DeviceStatus::Connected | DeviceStatus::Ready | DeviceStatus::Busy
            ))
            .map(|device| device.udid)
            .collect(),
    };
    if !matches!(selection, FlowTargetSelection::AllEligible)
        && targets.iter().any(|udid| !known.contains(udid.as_str()))
    {
        return Err(FlowSelectionError::UnknownDevice);
    }
    targets.sort();
    let before = targets.len();
    targets.dedup();
    if targets.len() != before {
        return Err(FlowSelectionError::Duplicate);
    }
    if targets.is_empty() {
        return Err(FlowSelectionError::NoEligibleDevice);
    }
    Ok(targets)
}

fn retry_is_allowed(
    attempt: &FlowNodeAttemptRecord,
    reconciler_proved_retry_safe: bool,
) -> bool {
    attempt.state == FlowAttemptState::FailedBeforeDispatch
        || (attempt.side_effect_class == SideEffectClass::IdempotentSet
            && attempt.state == FlowAttemptState::FailedVerified
            && reconciler_proved_retry_safe)
}
```

- [ ] **Step 5: Implement startup recovery and events**

Before returning FlowNodeAttemptRecord, read retry_safe and assign retry_allowed
through retry_is_allowed. New attempts always persist retry_safe=0. Only a successful
idempotent-set reconciler transaction may set retry_safe=1, and that transaction
must append its proof event before the projection exposes a Retry control.

Load nonterminal attempts before accepting new runs. Convert IntentCommitted to
FailedBeforeDispatch. Reconcile EffectDispatched/Verifying; unresolved results become
Uncertain. A Queued attempt is safe to reclaim only when every predecessor in the
immutable linear plan is Succeeded and no attempt on that device is Uncertain; this
rule applies equally to read-only and side-effect nodes because Queued precedes the
intent boundary. Add `FlowUpdated` and `FlowRunUpdated` AppEvent variants carrying
IDs and monotonic revisions, not whole mutable projections.

Reconciliation dispatches strictly by the compiled action definition: Launch/Home
use `ReadActiveApp`, Terminate uses the read-only `ReadProcess` plus its persisted
pre-effect PID (absent = success, same PID = proved non-delivery, different PID =
`Uncertain`), frame actions use
their persisted baseline plus the same generation, Type Text uses `ReadText`, and
Screenshot uses `ReadArtifact`. A first Launch left at `EffectDispatched` because
session creation failed is inspected without foregrounding the app again. A
Terminate left there is queried without calling kill. Only an exact typed proof may
advance or mark retry-safe; otherwise the attempt becomes `Uncertain`.

For a reclaimable Interrupted read-only attempt, first persist Interrupted -> Queued
with an event, then follow the ordinary Queued -> IntentCommitted path. An ambiguous
side effect is never stored as Interrupted, so this reset cannot redispatch Tap,
Swipe, Type Text, or any other ambiguous effect.

```rust
enum RecoveryTarget {
    ReclaimIfPredecessorsSucceeded,
    Terminal(FlowAttemptState),
    Reconcile,
    AlreadyTerminal,
}

fn recovery_target(attempt: &FlowNodeAttemptRecord) -> RecoveryTarget {
    match attempt.state {
        FlowAttemptState::Queued | FlowAttemptState::Interrupted =>
            RecoveryTarget::ReclaimIfPredecessorsSucceeded,
        FlowAttemptState::IntentCommitted =>
            RecoveryTarget::Terminal(FlowAttemptState::FailedBeforeDispatch),
        FlowAttemptState::EffectDispatched | FlowAttemptState::Verifying =>
            RecoveryTarget::Reconcile,
        FlowAttemptState::Succeeded
        | FlowAttemptState::FailedBeforeDispatch
        | FlowAttemptState::FailedVerified
        | FlowAttemptState::Uncertain
        | FlowAttemptState::Cancelled => RecoveryTarget::AlreadyTerminal,
    }
}
```

Add these variants directly to existing `AppEvent` (which already owns the serde
tag), then emit only after the corresponding database transaction commits:

```rust
FlowUpdated { flow_id: FlowId, revision: u64 },
FlowRunUpdated { run_id: uuid::Uuid, revision: u64 },
```

- [ ] **Step 6: Run tests and commit**

```powershell
cargo fmt --all
cargo test -p riviu-core flow::runtime -- --nocapture
cargo test -p riviu-core job_queue -- --nocapture
git add crates/core/src/flow crates/core/src/events.rs
git commit -m "feat(flow): orchestrate durable multi-device runs"
```

### Task 8: Close Runtime Gate F1

**Files:**
- Modify: `AGENTS.md`
- Modify: `docs/superpowers/plans/2026-07-30-riviu-flow-v2-runtime.md`

- [ ] **Step 1: Run the full gate**

```powershell
python -m unittest sidecars.pymobiledevice3.test_app_control -v
cargo fmt --all -- --check
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
git diff --check
```

Expected: every command exits 0; production IPA/manifest hashes remain the values recorded in `AGENTS.md`.

- [ ] **Step 2: Record and commit F1**

Mark all F1 checkboxes complete. Record commit range, test counts, termination contract, disabled TikTok nodes, next plan, and rollback commit in `AGENTS.md`.

```powershell
git add AGENTS.md docs/superpowers/plans/2026-07-30-riviu-flow-v2-runtime.md
git commit -m "docs(flow): record runtime gate F1"
```
