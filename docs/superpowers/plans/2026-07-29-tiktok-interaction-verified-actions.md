# TikTok Interaction Verified Actions Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Deliver Gate G2: execute a qualified target through open URL, mandatory Copy Link identity proof, Watch, Like, Follow, and Comment with durable intent and frame-derived evidence, while preserving Nurture behavior.

**Architecture:** The Gate 0 device adapter owns the per-UDID lease, stream permit, ordered session/MJPEG lifecycle, and clipboard transitions. `riviu-core` owns a reusable TikTok action facade over `UiSession` plus `FrameSource`; the campaign batch executor supplies an identity-confirmed context and records every state change through the Gate G1 `InteractionProgress` port. Nurture is migrated onto the same Like/Follow/Comment primitives so there is one implementation of evidence thresholds and text recovery.

**Tech Stack:** Rust 2021, Tokio, async-trait, image 0.25, rusqlite through `InteractionStore`, Tauri 2 composition, Python 3.9+ live probes, pymobiledevice3 10.1.0, Pillow 11.3.0, XCTest/WDA, MJPEG.

---

## Execution Preconditions

- Complete and merge `docs/superpowers/plans/2026-07-29-tiktok-interaction-gate-0-device-control.md` and `docs/superpowers/plans/2026-07-29-tiktok-interaction-campaign-core.md` first. G0 must expose the shared `DeviceWorkCoordinator`, producer-counted `StreamBudget`, install-only repair, ordered Interaction session/stream primitives, clipboard modes, and an exact qualification registry. G1 must expose `InteractionBatchExecutor`, `TikTokActionExecutor`, `InteractionProgress`, `PreparedActionPayload`, `ActionOutcome`, and `EvidenceRef`.
- Reconcile the G1 names to the canonical set used by its final task: `PlannedActionKind`, `ActionStatus`, `AssignmentStatus`, and `CampaignStatus`. Do not introduce parallel `ActionKind` or `ActionState` enums.
- Execute in an isolated worktree after the current dirty Project 2/runtime work is committed to an integration baseline. Stage only paths named by the current task.
- Keep `sidecars/wda/RiviuAgent.ipa` and `sidecars/wda/agent-manifest.json` byte-identical. This plan adds no Agent route and does not advertise a manifest feature.
- Keep `sidecars/wda/interaction-capabilities.json` empty for G2 action entries until the fixed Mac/device gate for that exact tuple passes. Fixture tests never qualify production.
- Re-read `AGENTS.md` before every lifecycle or recovery task. Session must precede MJPEG; Interaction must not call generic `preflight_agent()`, `repair_agent()`, or a helper that silently creates a session/stream.

## File Map

**Create**

- `crates/core/src/tiktok_actions/mod.rs`: public facade and shared action contracts.
- `crates/core/src/tiktok_actions/frame_probe.rs`: decoded current-frame reads, digest/timestamp evidence, bounded predicate waits, and geometry checks.
- `crates/core/src/tiktok_actions/identity.rs`: Share/Copy Link locators and the mandatory sentinel/read-back identity state machine.
- `crates/core/src/tiktok_actions/verified.rs`: Watch, Like, and Follow desired-state executors.
- `crates/core/src/tiktok_actions/comment.rs`: deterministic comment preparation and prepared-text-only send executor.
- `crates/core/src/interaction/device_batch_executor.rs`: G0 lease/lifecycle adapter implementing G1 `InteractionBatchExecutor` and `TikTokActionExecutor` composition.
- `crates/core/tests/tiktok_verified_actions.rs`: stream/session fakes and frame-confirmation tests.
- `crates/core/tests/interaction_device_batch.rs`: lifecycle, identity, recovery, cancellation, and durable-intent integration tests.
- `crates/core/tests/fixtures/interaction/manifest.json`: fixture provenance, dimensions, detector versions, and SHA-256 values.
- `crates/core/tests/fixtures/interaction/feed-followed-unliked.jpg`: positively located rail without a Follow badge.
- `crates/core/tests/fixtures/interaction/feed-followed-liked.jpg`: positively located rail with a filled heart and no Follow badge.
- `crates/core/tests/fixtures/interaction/share-copy-link-video.jpg`: qualified video Share drawer.
- `crates/core/tests/fixtures/interaction/share-copy-link-photo.jpg`: qualified photo Share drawer.
- `crates/core/tests/fixtures/interaction/share-closed.jpg`: negative Share fixture.
- `crates/core/src/templates/interaction/share-drawer-v1.png`: fixture-derived Share drawer anchor.
- `crates/core/src/templates/interaction/copy-link-v1.png`: fixture-derived Copy Link control anchor.
- `apps/desktop/src-tauri/src/bin/live_interaction_verified_actions.rs`: fixed Gate G2 real-device harness.
- `tools/interaction-gate2/verify_report.py`: schema, threshold, tuple, cleanup, and redaction verifier.
- `tools/interaction-gate2/test_verify_report.py`: fixture report tests.
- `docs/re/interaction-gate2/README.md`: fixed live procedure and report contract.

**Modify**

- `Cargo.toml`: enable Tokio's `test-util` feature for deterministic paused-clock Watch tests.
- `crates/core/src/lib.rs`: export `tiktok_actions`.
- `crates/core/src/screen.rs`: return coordinates for both rail layouts and add Save/Share positions for later gates.
- `crates/core/src/screen_match.rs`: expose ROI/template helpers used by Share locators without duplicating NCC.
- `crates/core/src/openai_client.rs`: accept an explicit campaign instruction while preserving Nurture generation.
- `crates/core/src/interaction/types.rs`: add typed prepared-comment, identity, action-result, and evidence payloads.
- `crates/core/src/interaction/progress.rs`: persist identity shadow state and prepared payload before UI.
- `crates/core/src/interaction/store.rs`: implement the corresponding transactional CAS updates.
- `crates/core/src/interaction/executor.rs`: expose the concrete batch/action input contracts consumed here.
- `crates/core/src/nurture/actions.rs`: delegate Like, Follow, and text Comment to the shared facade.
- `crates/core/src/nurture/mod.rs`: use the coordinate-returning rail locator and shared text-health policy.
- `crates/core/src/nurture/recovery.rs`: call the shared fresh-text context replacement helper.
- `crates/ios-driver/src/interaction_runtime.rs`: expose one owned context that can replace session, stream generation, feed, and watcher handle atomically.
- `crates/ios-driver/src/mock.rs`: deterministic G2 call log and lifecycle failures.
- `apps/desktop/src-tauri/src/state.rs`: construct the qualified batch executor but keep it disabled without registry evidence.
- `sidecars/wda/interaction-capabilities.schema.json`: add independently keyed Like, Follow, and Comment qualification entries.
- `sidecars/wda/interaction-capabilities.json`: add entries only in the live PASS task.
- `AGENTS.md`: record G2 invariants and actual gate status.

---

### Task 1: Reconcile The G1 Action Port And Extract Frame Observation

**Files:**
- Create: `crates/core/src/tiktok_actions/mod.rs`
- Create: `crates/core/src/tiktok_actions/frame_probe.rs`
- Create: `crates/core/tests/tiktok_verified_actions.rs`
- Modify: `crates/core/src/lib.rs`
- Modify: `crates/core/src/interaction/types.rs`
- Modify: `crates/core/src/interaction/executor.rs`
- Modify: `crates/core/src/interaction/progress.rs`

- [ ] **Step 1: Add failing contract and frame-probe tests**

Add tests that prove a facade result cannot be constructed from a gesture ACK alone, a decoded frame carries both a digest and observation time, a predicate accepts only a frame newer than the supplied digest, cancellation exits without a tap, and geometry mismatch fails before session use:

```rust
#[tokio::test]
async fn wait_after_rejects_the_baseline_and_accepts_a_new_matching_frame() {
    let frames = Arc::new(TestFrames::new("iphone-a", jpeg(10)));
    let probe = FrameProbe::new("iphone-a", frames.clone(), geometry_375x667());
    let baseline = probe.latest().unwrap();
    frames.publish("iphone-a", jpeg(20));

    let observed = probe
        .wait_after(baseline.digest, Duration::from_secs(1), &stop_false(), |_| true)
        .await
        .unwrap();

    assert_ne!(observed.digest, baseline.digest);
    assert_eq!(observed.logical_bounds, (375.0, 667.0));
}

#[test]
fn action_outcome_requires_typed_evidence() {
    let outcome = VerifiedEffect::NotConfirmed {
        code: ActionOutcomeCode::FrameTransitionMissing,
        evidence: vec![fixture_frame_evidence()],
    };
    assert!(!outcome.is_positive());
}
```

- [ ] **Step 2: Run the focused test and verify RED**

```powershell
cargo test -p riviu-core --test tiktok_verified_actions frame_probe -- --nocapture
```

Expected: compilation fails because `tiktok_actions`, `FrameProbe`, `VerifiedEffect`, and the G2 outcome codes do not exist.

- [ ] **Step 3: Add the exact shared contracts**

Create `tiktok_actions/mod.rs` with these contracts. Keep transport errors separate from evidence outcomes so the batch executor can apply the G0 recovery policy without turning ambiguity into a retry:

```rust
pub mod frame_probe;

#[derive(Debug, Clone, PartialEq)]
pub enum VerifiedEffect<T> {
    Applied(T),
    AlreadySatisfied(T),
    NotConfirmed { code: ActionOutcomeCode, evidence: Vec<FrameEvidence> },
    Uncertain { code: ActionOutcomeCode, evidence: Vec<FrameEvidence> },
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FrameEvidence {
    pub digest: String,
    pub observed_at: chrono::DateTime<chrono::Utc>,
    pub pixel_width: u32,
    pub pixel_height: u32,
    pub logical_width: f64,
    pub logical_height: f64,
    pub detector_version: String,
}

pub use crate::device_capabilities::QualifiedGeometry;

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
}

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
}

impl From<ActionOutcomeCode> for ActionExecutionError {
    fn from(code: ActionOutcomeCode) -> Self { Self::Outcome(code) }
}

impl From<anyhow::Error> for ActionExecutionError {
    fn from(error: anyhow::Error) -> Self { Self::Ui(error) }
}
```

Reuse G1's existing `UnsupportedGeometry`, `TargetChanged`, `TargetUnverified`,
`TargetIdentityAmbiguous`, `TextNotArmed`, and `TextNotSent` variants. Add only the G2 variants that are
missing: `FrameUnavailable`, `FrameTransitionMissing`, `RailNotLocated`,
`ClipboardRestoreFailed`, `LikeNotConfirmed`, `FollowNotConfirmed`,
`CommentGenerationFailed`, `CommentExistingDraft`, and
`TextChannelUnavailable`. Do not duplicate enum variants or add a free-form
result-code path.

Canonicalize the G1 progress-port identifiers while touching the contract: methods that address assignments take `&AssignmentId`, and methods that address actions take `&ActionRunId`. Those aliases are UUIDs, so downstream code must not stringify them or invent parallel string IDs.

Complete the immutable G1 execution DTO with the already persisted preparation inputs; G2 Comment and G4 recipient selection must not query mutable campaign defaults or invent runtime randomness:

```rust
pub struct AssignmentExecution {
    pub assignment_id: AssignmentId,
    pub account: AccountBinding,
    pub target: ResolvedTikTokTarget,
    pub effective_proxy: EffectiveProxySnapshot,
    pub effective_settings: InteractionDefaults,
    pub assignment_seed: u64,
    pub capabilities: CapabilitySnapshot,
    pub target_delay_after_ms: u32,
    pub actions: Vec<PreparedAction>,
}
```

The dispatcher hydrates `effective_settings`, `assignment_seed`, and
`target_delay_after_ms` from the immutable assignment snapshot, while every
`PreparedAction` retains G1's persisted `delay_before_ms` and
`watch_duration_ms`. The batch/action facade borrows these values; it never
re-merges current defaults or samples timing again.

Reconcile the G1 action port once so preparation that needs the immutable
assignment snapshot is implementable without a side cache or mutable-default
lookup. Add `assignment` to the existing method; update G1 fakes and the disabled
adapter in the same commit, and do not add a second trait:

```rust
#[async_trait::async_trait]
pub trait TikTokActionExecutor: Send + Sync {
    async fn open_and_verify_target(
        &self,
        session: &mut UiWithStreamContext,
        assignment: &AssignmentExecution,
        progress: Arc<dyn InteractionProgress>,
    ) -> Result<VerifiedTargetContext, ExecutorError>;

    async fn execute_action(
        &self,
        session: &mut UiWithStreamContext,
        assignment: &AssignmentExecution,
        context: &VerifiedTargetContext,
        action: &PreparedAction,
        progress: Arc<dyn InteractionProgress>,
    ) -> Result<ActionOutcome, ExecutorError>;
}
```

`assignment.assignment_id` must equal `context.assignment_id`, and the supplied
`action_run_id` must belong to that assignment; reject either mismatch before
frame or session use. G2 Comment and G4 Direct Message preparation consume this
borrowed snapshot. They never retain it beyond the call or reload settings through
another repository API.

Also reconcile G1's Comment retry constant with the live-confirmed two-strike health policy: permit three total pre-intent action attempts, comprising two consecutive `TextNotArmed` observations, one fresh-context replacement, and exactly one post-refresh attempt. All attempts reuse the same prepared payload; no retry is legal after effect intent.

- [ ] **Step 4: Implement `FrameProbe` without WDA screenshots**

`FrameProbe::latest()` decodes only `FrameSource::latest`; `wait_after()` subscribes/coalesces or polls the same source at 120 ms, checks cancellation, rejects the baseline digest, verifies pixel dimensions and portrait bounds on every candidate, and returns `UnsupportedGeometry` before any coordinate can be derived. Use SHA-256 for persisted evidence digests; keep the existing cheap FNV digest only for in-process change detection.

```rust
pub struct ObservedFrame {
    pub jpeg: Frame,
    pub image: image::RgbImage,
    pub digest: u64,
    pub evidence: FrameEvidence,
}

impl FrameProbe {
    pub fn latest(&self) -> Result<ObservedFrame, ActionProbeError>;

    pub async fn wait_after<F>(
        &self,
        previous: u64,
        timeout: Duration,
        stop: &AtomicBool,
        predicate: F,
    ) -> Result<ObservedFrame, ActionProbeError>
    where
        F: FnMut(&image::RgbImage) -> bool + Send;
}
```

- [ ] **Step 5: Run contract tests and full core compile**

```powershell
cargo test -p riviu-core --test tiktok_verified_actions frame_probe -- --nocapture
cargo test -p riviu-core --lib --no-run
```

Expected: frame-probe tests pass; G1 fakes compile against the single canonical enum set.

- [ ] **Step 6: Commit the observation boundary**

```powershell
git add crates/core/src/lib.rs crates/core/src/interaction/types.rs crates/core/src/interaction/executor.rs crates/core/src/interaction/progress.rs crates/core/src/tiktok_actions/mod.rs crates/core/src/tiktok_actions/frame_probe.rs crates/core/tests/tiktok_verified_actions.rs
git diff --cached --name-only
git commit -m "feat(core): define verified TikTok action evidence"
```

---

### Task 2: Replace Boolean Rail Presence With A Coordinate Locator

**Files:**
- Modify: `crates/core/src/screen.rs`
- Modify: `crates/core/tests/real_frames.rs`
- Modify: `crates/core/tests/tiktok_verified_actions.rs`
- Create: `crates/core/tests/fixtures/interaction/feed-followed-unliked.jpg`
- Create: `crates/core/tests/fixtures/interaction/feed-followed-liked.jpg`
- Create: `crates/core/tests/fixtures/interaction/manifest.json`

- [ ] **Step 1: Add followed/unfollowed and negative fixture tests**

Capture the two followed-author frames from the same qualified 375x667 portrait tuple through the existing MJPEG stream. Record source report, TikTok version/build, pixel dimensions, and SHA-256 in `manifest.json`; do not put an account name or UDID in it. Add tests for both known layouts, filled/unfilled heart, existing feed fixtures, LIVE preview, mid-swipe, and blank frames:

```rust
#[test]
fn followed_author_rail_returns_current_coordinates_without_a_badge() {
    let image = load_interaction_fixture("feed-followed-unliked.jpg");
    let located = screen::locate_action_rail(&image).expect("positive white-chain rail");
    assert_eq!(located.anchor, RailAnchor::WhiteIconChain);
    assert!(located.matched_icons >= 3);
    assert!(located.rail.located);
}

#[test]
fn live_and_mid_swipe_frames_never_return_coordinates() {
    for name in ["feed-live-card.jpg", "feed-mid-swipe.jpg"] {
        assert!(screen::locate_action_rail(&load(name)).is_none(), "{name}");
    }
}
```

- [ ] **Step 2: Run the tests and verify RED**

```powershell
cargo test -p riviu-core --test real_frames action_rail -- --nocapture
cargo test -p riviu-core --test tiktok_verified_actions rail -- --nocapture
```

Expected: the new coordinate-returning locator and `RailAnchor` are unresolved.

- [ ] **Step 3: Extend `ActionRail` for all rail-owned actions**

Keep normalized frame coordinates and derive every icon from a positively scored layout. The Save/Share fields are added now so Gate G4 reuses the same rail proof:

```rust
const FOLLOW_TO_LIKE: f64 = 51.0 / 667.0;
const FOLLOW_TO_COMMENT: f64 = 113.0 / 667.0;
const FOLLOW_TO_SAVE: f64 = 181.0 / 667.0;
const FOLLOW_TO_SHARE: f64 = 248.0 / 667.0;

pub struct ActionRail {
    pub x: f64,
    pub follow_y: f64,
    pub like_y: f64,
    pub comment_y: f64,
    pub save_y: f64,
    pub share_y: f64,
    pub located: bool,
}

pub enum RailAnchor { FollowBadge, WhiteIconChain }

pub struct LocatedActionRail {
    pub rail: ActionRail,
    pub anchor: RailAnchor,
    pub layout: u8,
    pub matched_icons: u8,
}
```

- [ ] **Step 4: Implement two-template white-chain scoring**

Retain the red-badge fast path. When it is absent, score the measured white-run centers against both layout candidates (`follow_y=223/667` and `259/667`) at expected Like, Comment, Save, and Share offsets. Accept at least two matched icons only when the winning layout exceeds the other by one match or by a 6-point residual margin; a filled red heart may be missing from the white chain. An ambiguous tie returns `None`. `rail_icons_present()` becomes `locate_action_rail(img).is_some()`. Keep `ActionRail::fallback()` only for compatibility until Task 7 removes its Nurture use; Interaction never calls it.

- [ ] **Step 5: Verify fixture hashes and GREEN**

```powershell
cargo test -p riviu-core --test real_frames -- --nocapture
cargo test -p riviu-core --test tiktok_verified_actions rail -- --nocapture
cargo test -p riviu-core screen:: -- --nocapture
```

Expected: both followed states and both layouts return current coordinates; LIVE, transition, and ambiguous fixtures return none; existing heart thresholds remain unchanged.

- [ ] **Step 6: Commit the locator and fixtures**

```powershell
git add crates/core/src/screen.rs crates/core/tests/real_frames.rs crates/core/tests/tiktok_verified_actions.rs crates/core/tests/fixtures/interaction/feed-followed-unliked.jpg crates/core/tests/fixtures/interaction/feed-followed-liked.jpg crates/core/tests/fixtures/interaction/manifest.json
git diff --cached --name-only
git commit -m "feat(core): locate complete TikTok action rail"
```

---

### Task 3: Implement The Mandatory Copy Link Identity State Machine

**Files:**
- Create: `crates/core/src/tiktok_actions/identity.rs`
- Modify: `crates/core/src/screen.rs`
- Modify: `crates/core/src/screen_match.rs`
- Modify: `crates/core/src/interaction/types.rs`
- Modify: `crates/core/src/interaction/progress.rs`
- Modify: `crates/core/src/interaction/store.rs`
- Create: `crates/core/src/interaction/device_batch_executor.rs`
- Modify: `crates/core/src/tiktok_actions/mod.rs`
- Modify: `crates/core/tests/tiktok_verified_actions.rs`
- Create: `crates/core/tests/interaction_device_batch.rs`
- Modify: `crates/core/tests/fixtures/interaction/manifest.json`
- Create: `crates/core/tests/fixtures/interaction/share-copy-link-video.jpg`
- Create: `crates/core/tests/fixtures/interaction/share-copy-link-photo.jpg`
- Create: `crates/core/tests/fixtures/interaction/share-closed.jpg`
- Create: `crates/core/src/templates/interaction/share-drawer-v1.png`
- Create: `crates/core/src/templates/interaction/copy-link-v1.png`

- [ ] **Step 1: Add pure locator tests before device orchestration**

Capture Share fixtures through MJPEG and derive the exact `share-drawer-v1.png` and `copy-link-v1.png` crops under `crates/core/src/templates/interaction/`. Extend the fixture manifest with every JPEG/template SHA-256 and crop rectangle. Test that `locate_share_drawer`, `locate_copy_link`, `locate_share_dismiss`, and `share_closed` accept video/photo drawers and reject feed, comments, popup, and cropped/rotated inputs. `locate_share_dismiss` returns a point in the drawer's observed dimmed backdrop, outside every Share method tile; it never uses a fixed screen point. Every result must include current-frame coordinates and detector version.

```rust
#[test]
fn copy_link_locator_is_specific_to_a_qualified_share_drawer() {
    let frame = load_interaction_fixture("share-copy-link-video.jpg");
    let drawer = locate_share_drawer(&frame).expect("share drawer");
    let copy = locate_copy_link(&frame, &drawer).expect("copy link");
    assert!(copy.ncc >= 0.86);
    assert_eq!(copy.detector_version, "share-copy-link-v1");
}
```

- [ ] **Step 2: Run locator tests and verify RED**

```powershell
cargo test -p riviu-core --test tiktok_verified_actions identity_locator -- --nocapture
```

Expected: identity locators and templates do not exist.

- [ ] **Step 3: Define bounded identity contracts**

Add these serializable evidence types and a mutable context port. The concrete G0 adapter must replace the session and stream handles inside the same lease when the clipboard mode requires foregrounding the Agent:

```rust
pub const MAX_IDENTITY_CLIPBOARD_BYTES: usize = 64 * 1024;

pub struct PriorClipboard {
    bytes: Vec<u8>,
    pub content_type: String,
    pub sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TargetIdentityEvidence {
    pub identity_action_run_id: ActionRunId,
    pub identity_attempt_no: u32,
    pub planned_content_id: String,
    pub copied_content_id: String,
    pub planned_kind: TikTokPostKind,
    pub copied_kind: TikTokPostKind,
    pub copied_url_sha256: String,
    pub sentinel_sha256: String,
    pub prior_type: String,
    pub prior_len: u32,
    pub prior_sha256: String,
    pub clipboard_mode: ClipboardAccessMode,
    pub frame: FrameEvidence,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum TargetIdentityResult {
    Confirmed { evidence: TargetIdentityEvidence },
    Unverified { code: ActionOutcomeCode, frames: Vec<FrameEvidence> },
    Uncertain { code: ActionOutcomeCode, frames: Vec<FrameEvidence> },
}

#[async_trait::async_trait]
pub trait IdentityUiContext: Send {
    fn session(&self) -> Arc<dyn UiSession>;
    fn probe(&self) -> &FrameProbe;
    async fn set_clipboard(&mut self, content_type: &str, bytes: &[u8]) -> anyhow::Result<()>;
    async fn get_clipboard(&mut self, max_bytes: usize) -> anyhow::Result<(String, Vec<u8>)>;
    async fn restore_target_after_clipboard(&mut self) -> anyhow::Result<()>;
}
```

Validate this closed result contract at construction and persistence boundaries: `Unverified` accepts only `TargetUnverified`, `Uncertain` accepts only `TargetIdentityAmbiguous`, and Confirmed evidence must carry the same current identity action ID/attempt as the `PreparedAction`. Reject every other status/code or stale-attempt combination before an aggregate can change.

The byte field stays private, the type implements neither `Serialize` nor `Debug`, and `Drop` calls `self.bytes.fill(0)`. Evidence stores only type, length, and hashes.

Create the initial `DeviceIdentityContext` in `interaction/device_batch_executor.rs` here. It owns only the already-acquired G0 lease/runtime handles needed to implement `IdentityUiContext`; it must not own another coordinator, mutex, semaphore, or SQLite connection. Task 8 extends this context with atomic text-handle replacement, and Task 9 completes its batch orchestration.

Add `pub mod identity;` to `tiktok_actions/mod.rs` only in this task, after `identity.rs` and its templates exist.

- [ ] **Step 4: Make identity intent durable through the G1 identity-action CAS**

The current `TargetIdentityCopyLink` action row's `identity_copy_intent` is the durable append-only no-replay source of truth; the assignment's `current_identity_attempt_no` and `identity_copy_intent` are its required current projection. Preserve the G1 split: call `InteractionProgress::issue_identity_copy_intent(identity_action_id)` immediately before the Copy Link tap; never call `issue_effect_intent` for identity because G1 restricts that method to Comment, Repost, and Direct Message. The CAS requires that exact highest identity attempt and assignment projection to be `Running/None`, changes both intent values to `Issued` in one transaction, and commits before device I/O. It never mutates an earlier Confirmed attempt. Completing the current identity action updates derived `identity_state` and the action/assignment/campaign aggregates together. Add rollback, projection-mismatch, stale-attempt-ID, and concurrent-CAS tests.

```rust
progress.action_started(&identity_action_id).await?;
progress.issue_identity_copy_intent(&identity_action_id).await?;
if context.session().tap(copy_link.point).await.is_err() {
    let frames = context.probe().latest().ok()
        .map(|observed| observed.evidence)
        .into_iter()
        .collect();
    return Ok(TargetIdentityResult::Uncertain {
        code: ActionOutcomeCode::TargetIdentityAmbiguous,
        frames,
    });
}
```

Do not keep a SQLite transaction open across the tap. The batch executor converts the returned frame metadata into bounded `EvidenceRef` artifacts and immediately calls `action_finished`; it does not trust this result as already persisted. If the tap or subsequent read is ambiguous after the committed intent, finish the action as `Uncertain/TargetIdentityAmbiguous`; do not tap Copy Link again. A completed read that deterministically yields a stale sentinel, unsupported URL, exhausted resolution, content mismatch, or post-kind mismatch is `Failed/TargetUnverified`, never `Uncertain`.

- [ ] **Step 5: Implement sentinel, read-back, comparison, and cleanup**

`verify_target_identity` receives the current identity `PreparedAction` and performs exactly this sequence: read and retain prior bytes within 64 KiB; store only metadata; write a namespaced random sentinel containing assignment/action-attempt IDs and verify read-back; when the clipboard mode foregrounds the Agent, call `restore_target_after_clipboard()` and await the replacement session plus first current-generation frame before locating Share; locate/tap Share from the current rail; commit that action row's identity-copy intent; tap Copy Link once; read the clipboard; when that read foregrounds the Agent, call `restore_target_after_clipboard()` again before returning the context to optional actions; require a new HTTPS TikTok URL; resolve through the existing bounded resolver; normalize; compare content ID and post kind; close Share if it is still present. On failure before identity success, best-effort restore prior bytes and verify the prior hash, then restore the target context if clipboard access changed foreground state. Report `ClipboardRestoreFailed` alongside the primary code without replacing it. On success, do not restore the prior value: leave the verified target URL in the clipboard exactly as TikTok supplied it.

Opening is a separate durable pre-intent loop owned by the batch coordinator. Before each `open_url`, append an `interaction_open_attempts` row through `start_opening_attempt`; finish it from a newer target-frame result. At most two attempts are permitted for the current identity action (initial plus one retry), and attempt 2 is legal only for G1's typed pre-intent retry reasons while `identity_copy_intent=None`. A successful open proceeds to identity; an exhausted failure finishes the identity action with the underlying deterministic code and runs zero optional effects. Neither identity code nor cleanup may call `open_url` after intent issuance. A process restart turns a Running opening row into `Interrupted` and does not resume it. Either terminal no-intent case may later be selected by G1's explicit `PreCopyRetryable` branch, which appends a new Pending/None identity action before the next batch; no prior Confirmed identity anchor is required and the old identity/opening rows remain immutable.

Export the bounded close primitive for later Share actions:

```rust
pub async fn close_share_drawer(
    session: &dyn UiSession,
    probe: &FrameProbe,
    gestures: &tokio::sync::Mutex<()>,
    stop: &AtomicBool,
) -> Result<FrameEvidence, ActionExecutionError>;
```

It re-detects the current drawer, derives the dimmed-backdrop point from that observation, taps once under the gesture mutex, and requires a newer `share_closed` frame. Identity treats close failure as cleanup evidence without overwriting a confirmed/mismatched primary result. Gate G4 reuses this primitive and does not introduce another close gesture contract.

Add fake call-log tests for stale sentinel, non-HTTPS value, redirect exhaustion, content mismatch, Copy Link tap error, read timeout after intent, cancellation at every boundary, restore failure, and successful target-URL retention. Assert deterministic mismatches are `Failed/TargetUnverified`, tap/read ambiguity is `Uncertain/TargetIdentityAmbiguous`, and optional action call counts remain zero for every non-success identity result. Add opening tests proving exactly two append-only attempts for one eligible pre-intent failure, rejection of attempt 3, rejection after identity intent, and zero automatic calls after a restarted Running opening attempt.

Add `cleanup_owned_sentinel` for process-loss recovery. On the first acquired batch lease for each UDID after startup, before creating the final action session, read the clipboard through the qualified mode and clear it only when its decoded value matches the exact `riviu-interaction-v1:<campaign-id>:<attempt-id>:<nonce>` grammar and the referenced attempt is stale. Verify the cleared value, persist only old hash/length/type plus cleanup outcome, and never clear arbitrary clipboard data. For `AgentForegroundRequired`, perform this before the final TikTok/session/MJPEG transition so no pre-cleanup session is reused. Add crash-fixture tests for owned, malformed, current-attempt, and unrelated clipboard values.

- [ ] **Step 6: Run identity and transaction tests**

```powershell
cargo test -p riviu-core --test tiktok_verified_actions identity -- --nocapture
cargo test -p riviu-core --test interaction_device_batch identity -- --nocapture
cargo test -p riviu-core --test interaction_transitions -- --nocapture
```

Expected: Copy Link is tapped at most once per immutable identity action attempt, raw clipboard bytes never enter serialized evidence, only exact content ID plus post-kind equality produces confirmed identity, and deterministic mismatch remains distinct from ambiguous tap/read outcome.

- [ ] **Step 7: Commit the identity executor**

```powershell
git add crates/core/src/screen.rs crates/core/src/screen_match.rs crates/core/src/tiktok_actions/mod.rs crates/core/src/tiktok_actions/identity.rs crates/core/src/interaction/types.rs crates/core/src/interaction/progress.rs crates/core/src/interaction/store.rs crates/core/src/interaction/device_batch_executor.rs crates/core/tests/tiktok_verified_actions.rs crates/core/tests/interaction_device_batch.rs crates/core/tests/fixtures/interaction/manifest.json crates/core/tests/fixtures/interaction/share-copy-link-video.jpg crates/core/tests/fixtures/interaction/share-copy-link-photo.jpg crates/core/tests/fixtures/interaction/share-closed.jpg crates/core/src/templates/interaction/share-drawer-v1.png crates/core/src/templates/interaction/copy-link-v1.png
git diff --cached --name-only
git commit -m "feat(core): verify TikTok targets through Copy Link"
```

---

### Task 4: Add Frame-Verified Watch

**Files:**
- Modify: `Cargo.toml`
- Create: `crates/core/src/tiktok_actions/verified.rs`
- Modify: `crates/core/src/tiktok_actions/mod.rs`
- Modify: `crates/core/tests/tiktok_verified_actions.rs`
- Modify: `crates/core/src/interaction/types.rs`

- [ ] **Step 1: Write duration, readiness, and cancellation tests**

Use a paused Tokio clock and coalescing frame fake. Require at least one target-ready sample per second, monotonic elapsed duration, and a final ready frame. Test a popup, absent rail for two consecutive samples, stream closure, geometry drift, and cancellation.

```rust
#[tokio::test(start_paused = true)]
async fn watch_succeeds_only_after_the_planned_elapsed_duration() {
    let fixture = verified_fixture().with_ready_frames_every(Duration::from_secs(1));
    let task = fixture.watch(Duration::from_secs(5));
    tokio::time::advance(Duration::from_secs(4)).await;
    assert!(!task.is_finished());
    tokio::time::advance(Duration::from_secs(1)).await;
    assert!(matches!(task.await.unwrap(), VerifiedEffect::Applied(_)));
}
```

- [ ] **Step 2: Verify RED**

```powershell
cargo test -p riviu-core --test tiktok_verified_actions watch -- --nocapture
```

- [ ] **Step 3: Implement `watch_target`**

Add `test-util` to the existing workspace Tokio feature list, then add `pub mod verified;` only after `verified.rs` exists. This keeps Task 1 compilable and makes `#[tokio::test(start_paused = true)]` plus `tokio::time::advance` deterministic.

```toml
tokio = { version = "1", features = [
  "rt-multi-thread", "macros", "sync", "time", "test-util",
  "io-util", "process", "fs", "net", "signal",
] }
```

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WatchEvidence {
    pub planned_ms: u64,
    pub elapsed_ms: u64,
    pub samples: Vec<FrameEvidence>,
}

pub async fn watch_target(
    probe: &FrameProbe,
    duration: Duration,
    stop: &AtomicBool,
) -> Result<VerifiedEffect<WatchEvidence>, ActionExecutionError>;
```

Target-ready means a current qualified portrait frame classified as `Feed` with `locate_action_rail` success. Store digest/timestamp evidence only; do not persist a screenshot for each sample. A transient single miss may recover on the next sample; two consecutive misses return `TargetChanged` and stop all later side effects for that assignment.

- [ ] **Step 4: Verify GREEN and commit**

```powershell
cargo test -p riviu-core --test tiktok_verified_actions watch -- --nocapture
git add Cargo.toml crates/core/src/tiktok_actions/mod.rs crates/core/src/tiktok_actions/verified.rs crates/core/src/interaction/types.rs crates/core/tests/tiktok_verified_actions.rs
git diff --cached --name-only
git commit -m "feat(core): verify target watch duration from MJPEG"
```

---

### Task 5: Add Typed Like And Follow Desired-State Executors

**Files:**
- Modify: `crates/core/src/tiktok_actions/verified.rs`
- Modify: `crates/core/src/interaction/types.rs`
- Modify: `crates/core/tests/tiktok_verified_actions.rs`
- Modify: `crates/core/tests/real_frames.rs`

- [ ] **Step 1: Write Like state-transition tests**

Cover already red, outline-to-red, tap ACK without red, rail disappearing, stale same frame, gesture error before a completed request, and geometry drift. Assert no tap for already-liked or unknown rail.

```rust
#[tokio::test]
async fn like_ack_without_red_heart_is_not_confirmed() {
    let fixture = verified_fixture().with_frames([unliked_frame(), unliked_frame_2()]);
    let result = fixture.actions.like(&fixture.context()).await.unwrap();
    assert!(matches!(result, VerifiedEffect::NotConfirmed {
        code: ActionOutcomeCode::LikeNotConfirmed, ..
    }));
    assert_eq!(fixture.session.tap_count(), 1);
}
```

- [ ] **Step 2: Write Follow state-transition tests**

Cover badge present-to-absent, positive white-chain rail with no badge, unknown rail, tap ACK with badge still present, and a post-tap unrelated frame. Only a positively located rail can produce `AlreadySatisfied`.

```rust
#[tokio::test]
async fn missing_badge_on_an_unknown_rail_is_not_already_satisfied() {
    let fixture = verified_fixture().with_frame(blank_feed_frame());
    let result = fixture.actions.follow(&fixture.context()).await.unwrap();
    assert!(matches!(result, VerifiedEffect::NotConfirmed {
        code: ActionOutcomeCode::RailNotLocated, ..
    }));
    assert_eq!(fixture.session.tap_count(), 0);
}
```

- [ ] **Step 3: Verify RED**

```powershell
cargo test -p riviu-core --test tiktok_verified_actions like -- --nocapture
cargo test -p riviu-core --test tiktok_verified_actions follow -- --nocapture
```

- [ ] **Step 4: Implement both executors over current-frame coordinates**

```rust
pub struct ActionContext<'a> {
    pub probe: &'a FrameProbe,
    pub session: &'a dyn UiSession,
    pub gestures: &'a tokio::sync::Mutex<()>,
    pub stop: &'a AtomicBool,
    /// The immutable seed persisted on this assignment; never runtime randomness.
    pub assignment_seed: u64,
}

pub async fn like(
    context: &ActionContext<'_>,
) -> Result<VerifiedEffect<LikeEvidence>, ActionExecutionError>;

pub async fn follow(
    context: &ActionContext<'_>,
) -> Result<VerifiedEffect<FollowEvidence>, ActionExecutionError>;
```

Each method decodes one current frame, obtains `LocatedActionRail`, reads desired state, derives point coordinates from the same frame and qualified geometry, then locks only the gesture. Like requires absolute redness above `LIKE_FILLED_REDNESS` on a newer frame. Follow requires the badge to disappear while the rail remains positively locatable. Persist before/best redness, rail anchor/layout/matched-icons, tap point, and before/after frame evidence; gesture ACK is not an outcome.

- [ ] **Step 5: Run focused and existing frame tests**

```powershell
cargo test -p riviu-core --test tiktok_verified_actions like -- --nocapture
cargo test -p riviu-core --test tiktok_verified_actions follow -- --nocapture
cargo test -p riviu-core --test real_frames -- --nocapture
```

Expected: all desired-state and negative cases pass; no test counts a gesture ACK as success.

- [ ] **Step 6: Commit Like and Follow**

```powershell
git add crates/core/src/tiktok_actions/verified.rs crates/core/src/interaction/types.rs crates/core/tests/tiktok_verified_actions.rs crates/core/tests/real_frames.rs
git diff --cached --name-only
git commit -m "feat(core): verify TikTok like and follow states"
```

---

### Task 6: Prepare And Persist One Deterministic Comment Before UI

**Files:**
- Create: `crates/core/src/tiktok_actions/comment.rs`
- Modify: `crates/core/src/tiktok_actions/mod.rs`
- Modify: `crates/core/src/openai_client.rs`
- Modify: `crates/core/src/interaction/types.rs`
- Modify: `crates/core/src/interaction/progress.rs`
- Modify: `crates/core/src/interaction/store.rs`
- Modify: `crates/core/tests/tiktok_verified_actions.rs`
- Modify: `crates/core/tests/interaction_transitions.rs`

- [ ] **Step 1: Write preparation ordering and restart tests**

Use an injected `CommentGenerator` and recording `InteractionProgress`. Test vision success, deterministic fallback, empty fallback, invalid generated text, progress failure, and a restart with an existing prepared payload. Assert no session method is called in this task.

```rust
#[tokio::test]
async fn prepared_text_is_durable_before_any_ui_call() {
    let fixture = comment_fixture().with_generated_text("fixture comment");
    let prepared = fixture.prepare().await.unwrap();
    assert_eq!(prepared.text, "fixture comment");
    assert_eq!(fixture.progress.prepared_count(), 1);
    assert!(fixture.session.call_log().is_empty());
}

#[tokio::test]
async fn fallback_selection_is_stable_for_the_assignment_seed() {
    let pool = vec!["one".into(), "two".into(), "three".into()];
    let a = deterministic_fallback(&pool, 41).unwrap();
    let b = deterministic_fallback(&pool, 41).unwrap();
    assert_eq!(a, b);
}
```

- [ ] **Step 2: Verify RED**

```powershell
cargo test -p riviu-core --test tiktok_verified_actions comment_prepare -- --nocapture
cargo test -p riviu-core --test interaction_transitions prepared_payload -- --nocapture
```

Expected: prepared-comment types, generator port, and store behavior are missing.

- [ ] **Step 3: Add the typed prepared payload**

Extend the G1 tagged `PreparedActionPayload` with a Comment variant; do not store an arbitrary JSON object:

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", content = "value", rename_all = "camelCase")]
pub enum PreparedActionPayload {
    NoPayload,
    Comment(PreparedCommentPayload),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PreparedCommentPayload {
    pub text: String,
    pub source: PreparedCommentSource,
    pub model: String,
    pub base_url_host: String,
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub usd: f64,
    pub target_frame_sha256: String,
    pub instruction_sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum PreparedCommentSource { Vision, DeterministicFallback }
```

Gate G2 preserves G1's optional execution-payload semantics, `NoPayload` variant, and adjacent-tag serialization, then adds only Comment. Gate G4 adds its reviewed `PreparedDirectMessagePayload` variant when recipient policy, resolution evidence, and pre-intent CAS semantics are implemented; G2 has no Direct Message payload variant.

`PreparedCommentPayload::validate` rejects blank/oversized text, control characters,
non-finite or negative `usd`, blank model names, a `base_url_host` containing scheme,
userinfo, path, query, or fragment, and digests that are not exact lowercase
64-character ASCII hex strings. Validate both newly generated and reloaded payloads
before any UI call;
JSON serialization failure is a typed preparation failure and never writes a partial
payload.

- [ ] **Step 4: Refactor generation around an explicit request**

Add a reusable request without changing the existing Nurture API:

```rust
pub struct VisionCommentRequest<'a> {
    pub settings: &'a NurtureSettings,
    pub jpeg: &'a [u8],
    pub instruction: &'a str,
}

pub async fn generate_campaign_comment(
    request: VisionCommentRequest<'_>,
) -> anyhow::Result<VisionCommentResult>;
```

Move the shared body construction and sanitization behind one private function. `generate_vision_comment(settings, jpeg, direction)` continues to build the current Nurture instruction and delegates. Never include API keys, full base URLs with credentials, or raw model responses in errors/events.

- [ ] **Step 5: Implement `prepare_and_persist_comment`**

```rust
#[async_trait::async_trait]
pub trait CommentGenerator: Send + Sync {
    async fn generate(&self, input: CommentGenerationInput<'_>)
        -> anyhow::Result<GeneratedComment>;
}

pub async fn prepare_and_persist_comment(
    action: &PreparedAction,
    stable_frame: &ObservedFrame,
    instruction: &str,
    fallback_pool: &[String],
    selection_seed: u64,
    generator: &dyn CommentGenerator,
    progress: &dyn InteractionProgress,
) -> Result<PreparedCommentPayload, ActionExecutionError>;
```

Match the finalized G1 optional execution payload exactly: `Some(PreparedActionPayload::Comment(existing))` validates and reuses it without calling the generator; `None` generates once; `Some(NoPayload)` or any non-Comment variant is a typed invalid plan for a Comment action and is never overwritten. On generation failure choose `pool[(selection_seed % pool.len() as u64) as usize]` only after rejecting an empty pool; validate non-blank sanitized text; call `progress.action_prepared(&action.action_run_id, Some(&PreparedActionPayload::Comment(payload.clone())))`; only then return the payload. Empty fallback plus generation failure becomes `CommentGenerationFailed` and opens no UI. The store writes `prepared_payload_json` only when null or byte-equivalent; a different second payload is an invalid transition. Add `pub mod comment;` to `tiktok_actions/mod.rs` in this task, after the file exists.

- [ ] **Step 6: Run preparation and persistence tests**

```powershell
cargo test -p riviu-core --test tiktok_verified_actions comment_prepare -- --nocapture
cargo test -p riviu-core --test interaction_transitions prepared_payload -- --nocapture
cargo test -p riviu-core openai_client::tests -- --nocapture
```

Expected: preparation is deterministic and durable, restart reuses exact text, and all UI call counts stay zero.

- [ ] **Step 7: Commit preparation**

```powershell
git add crates/core/src/tiktok_actions/mod.rs crates/core/src/tiktok_actions/comment.rs crates/core/src/openai_client.rs crates/core/src/interaction/types.rs crates/core/src/interaction/progress.rs crates/core/src/interaction/store.rs crates/core/tests/tiktok_verified_actions.rs crates/core/tests/interaction_transitions.rs
git diff --cached --name-only
git commit -m "feat(core): persist comments before TikTok UI"
```

---

### Task 7: Send Only Stored Comment Text With Durable Intent

**Files:**
- Modify: `crates/core/src/tiktok_actions/comment.rs`
- Modify: `crates/core/src/nurture/actions.rs`
- Modify: `crates/core/src/nurture/mod.rs`
- Modify: `crates/core/src/screen.rs`
- Modify: `crates/core/tests/tiktok_verified_actions.rs`
- Modify: `crates/core/tests/real_frames.rs`

- [ ] **Step 1: Write exact send-order tests**

The recording fake must observe this order: current rail, drawer tap, drawer open, empty composer, focus, type exact stored bytes, newer armed frame, durable intent commit, Send tap, newer unarmed drawer frame, close drawer. Add tests for existing draft, missing drawer, untrusted text session, typing ACK without armed state, intent commit failure, Send tap error after intent, and armed state that never disarms.

```rust
#[tokio::test]
async fn intent_is_committed_after_armed_evidence_and_before_send_tap() {
    let fixture = send_fixture().with_armed_then_unarmed_frames();
    let result = fixture.send(stored_comment("fixture text")).await.unwrap();
    assert!(matches!(result, CommentDelivery::Sent(_)));
    assert_eq!(fixture.log(), [
        "open_drawer", "focus", "type:fixture text", "frame:armed",
        "intent:issued", "tap:send", "frame:unarmed", "close_drawer",
    ]);
}

#[tokio::test]
async fn failure_after_intent_is_uncertain_and_send_is_not_repeated() {
    let fixture = send_fixture().fail_send_transport_after_dispatch();
    let result = fixture.send(stored_comment("fixture text")).await.unwrap();
    assert!(matches!(result, CommentDelivery::Uncertain { .. }));
    assert_eq!(fixture.session.send_tap_count(), 1);
}
```

- [ ] **Step 2: Verify RED**

```powershell
cargo test -p riviu-core --test tiktok_verified_actions comment_send -- --nocapture
```

- [ ] **Step 3: Define the narrow intent journal and delivery result**

```rust
#[async_trait::async_trait]
pub trait EffectIntentJournal: Send + Sync {
    async fn issue(&self, action_id: &ActionRunId) -> Result<(), ActionExecutionError>;
}

pub enum CommentDelivery {
    Sent(CommentEvidence),
    TextChannelUnavailable(Vec<FrameEvidence>),
    ExistingDraft(Vec<FrameEvidence>),
    NoDrawer(Vec<FrameEvidence>),
    TextNotArmed(Vec<FrameEvidence>),
    Uncertain { code: ActionOutcomeCode, evidence: Vec<FrameEvidence> },
}

pub async fn send_prepared_comment(
    context: &ActionContext<'_>,
    action_id: &ActionRunId,
    prepared: &PreparedCommentPayload,
    journal: &dyn EffectIntentJournal,
) -> Result<CommentDelivery, ActionExecutionError>;
```

`CampaignIntentJournal` delegates to G1 `InteractionProgress::issue_effect_intent`. `NurtureIntentJournal` records no campaign state but still keeps the exact method boundary; it is available only from `nurture`, not as the Interaction default.

- [ ] **Step 4: Extract the current proven drawer flow**

Move the frame-confirmed text path from `nurture/actions.rs` into `send_prepared_comment` without changing the live-confirmed timings or coordinates. Continue using RT-MMO sessionless gesture behavior through `UiSession::tap`, never W3C actions or element finding. The executor must:

1. reject a session whose `supports_text_input()` is false;
2. open from a newly located current rail;
3. wait for a non-loading drawer, then require `CommentDrawer::Open` before typing;
4. type only `prepared.text` and require a newer `SendArmed` frame;
5. commit intent before Send;
6. tap Send once and require a newer `CommentDrawer::Open` frame;
7. classify every failure after intent as `Uncertain/TextNotSent` and close UI best-effort.

Remove the current `tracing::info!(..., prepared.text)` statement. Logs/events may report source, token counts, cost, and text SHA-256, but never the raw prepared text.

- [ ] **Step 5: Keep Nurture behavior through an adapter**

Refactor `NurtureEngine::do_comment` into: generate its in-memory `PreparedCommentPayload`, call shared `send_prepared_comment`, map results back to the existing public session counters/cost row, and retain the existing `TextNotArmed`/`TextNotSent` meanings. Preserve all current regression tests, including existing draft, no emoji fallback after text failure, send error cleanup, and cost persistence after confirmed send.

- [ ] **Step 6: Run comment and Nurture tests**

```powershell
cargo test -p riviu-core --test tiktok_verified_actions comment_send -- --nocapture
cargo test -p riviu-core nurture::actions::tests -- --nocapture
cargo test -p riviu-core --test real_frames comment -- --nocapture
```

Expected: exact send ordering passes, post-intent ambiguity never retries, and every pre-existing Nurture comment regression remains green.

- [ ] **Step 7: Commit the shared comment sender**

```powershell
git add crates/core/src/tiktok_actions/comment.rs crates/core/src/nurture/actions.rs crates/core/src/nurture/mod.rs crates/core/src/screen.rs crates/core/tests/tiktok_verified_actions.rs crates/core/tests/real_frames.rs
git diff --cached --name-only
git commit -m "feat(core): share frame-verified comment delivery"
```

---

### Task 8: Share Text Health And Replace The Complete Owned Context

**Files:**
- Modify: `crates/core/src/tiktok_actions/comment.rs`
- Modify: `crates/core/src/nurture/mod.rs`
- Modify: `crates/core/src/nurture/recovery.rs`
- Modify: `crates/core/src/screen_watch.rs`
- Modify: `crates/core/src/interaction/device_batch_executor.rs`
- Modify: `crates/ios-driver/src/interaction_runtime.rs`
- Modify: `crates/ios-driver/src/mock.rs`
- Modify: `crates/core/tests/interaction_device_batch.rs`

- [ ] **Step 1: Write the two-strike and no-retry tests**

Retain the current Nurture tests and add Interaction tests. Two consecutive `TextNotArmed` results must cause exactly one refresh inside the current `DeviceExclusive` lease. The call log must show stream stop/generation advance, TikTok foreground, fresh text session, MJPEG start/first frame, then both executor and watcher handle replacement. `TextNotSent` must not call refresh or send again.

For Interaction, each `TextNotArmed` finishes its current action row and
`schedule_bounded_action_retry` creates the next distinct `ActionRunId` before more
UI. Assert all three rows retain byte-equal prepared payload,
`delay_before_ms`/`watch_duration_ms`, ordinal, and sampled decision. Add restart
cases after strike one and strike two: health is reconstructed from the durable
attempt chain, strike two still causes exactly one refresh before attempt three, and
the CAS rejects a fourth row. An issued intent or `TextNotSent` never creates a retry
row.

```rust
#[tokio::test]
async fn second_text_not_armed_replaces_session_feed_and_watcher_in_one_lease() {
    let fixture = device_batch_fixture().comment_results([
        CommentDelivery::TextNotArmed(vec![]),
        CommentDelivery::TextNotArmed(vec![]),
        CommentDelivery::Sent(sent_evidence()),
    ]);
    fixture.run().await.unwrap();
    assert_eq!(fixture.driver.lifecycle_log(), [
        "stop_stream:g1", "foreground:tiktok", "fresh_text_session:s2",
        "start_stream:g2", "first_frame:g2", "replace_executor:s2",
        "replace_watcher:s2",
    ]);
    assert_eq!(fixture.coordinator.acquire_count(), 1);
}
```

- [ ] **Step 2: Verify RED**

```powershell
cargo test -p riviu-core --test interaction_device_batch text_recovery -- --nocapture
cargo test -p riviu-core nurture::tests::two_consecutive_text_not_armed -- --nocapture
```

- [ ] **Step 3: Extract `TextDeliveryHealth`**

```rust
pub enum TextRecoveryDecision { Continue, RefreshFreshContext, DoNotRetry }

#[derive(Default)]
pub struct TextDeliveryHealth { consecutive_not_armed: u8 }

impl TextDeliveryHealth {
    pub fn observe(&mut self, result: &CommentDelivery) -> TextRecoveryDecision;
    pub fn fresh_context_installed(&mut self);
}
```

The threshold is fixed at two. `Sent` resets it; every non-`TextNotArmed` terminal result resets it; `Uncertain/TextNotSent` returns `DoNotRetry`. The Interaction adapter initializes health from the ordered durable attempt chain, validates contiguous attempt numbers and the same prepared payload, and then observes the current result; it does not reset strikes merely because the process restarted. The two pre-refresh failures plus the single post-refresh attempt are the three total pre-intent attempts reconciled in Task 1; a second refresh or fourth attempt is rejected by the progress CAS. Nurture may construct the default in-memory health because it does not own campaign retry rows.

- [ ] **Step 4: Add one context replacement operation**

The concrete G0-owned context exposes one method; callers do not re-acquire a device lease or stream permit:

```rust
#[async_trait::async_trait]
pub trait FreshTextContextRefresher: Send {
    async fn refresh_fresh_text_context(
        &mut self,
        bundle_id: &str,
    ) -> anyhow::Result<FreshContextProof>;
}

pub struct FreshContextProof {
    pub session: Arc<dyn UiSession>,
    pub stream_generation: u64,
    pub first_frame_sha256: String,
}
```

`interaction_runtime.rs` implements the exact G0 order and updates the executor session plus `SessionHandle` only after the first new-generation JPEG is decoded. On any failure it clears both handles and stops the partial producer; it does not open an ordinary fallback session. `nurture/recovery.rs` calls the same replacement helper while retaining its existing soft/hard transport budgets.

- [ ] **Step 5: Run recovery tests**

```powershell
cargo test -p riviu-core --test interaction_device_batch text_recovery -- --nocapture
cargo test -p riviu-core nurture::recovery::tests -- --nocapture
cargo test -p riviu-ios-driver interaction_lifecycle -- --nocapture
```

Expected: session-before-stream ordering and generation invalidation pass; no nested coordinator acquisition occurs; `TextNotSent` remains terminal and un-retried.

- [ ] **Step 6: Commit shared text recovery**

```powershell
git add crates/core/src/tiktok_actions/comment.rs crates/core/src/nurture/mod.rs crates/core/src/nurture/recovery.rs crates/core/src/screen_watch.rs crates/core/src/interaction/device_batch_executor.rs crates/ios-driver/src/interaction_runtime.rs crates/ios-driver/src/mock.rs crates/core/tests/interaction_device_batch.rs
git diff --cached --name-only
git commit -m "feat(driver): replace complete fresh text context"
```

---

### Task 9: Implement The G0-Owned Device Batch Executor

**Files:**
- Modify: `crates/core/src/interaction/device_batch_executor.rs`
- Modify: `crates/core/src/interaction/mod.rs`
- Modify: `crates/core/src/interaction/executor.rs`
- Modify: `crates/core/src/interaction/types.rs`
- Modify: `crates/ios-driver/src/interaction_runtime.rs`
- Modify: `crates/ios-driver/src/mock.rs`
- Modify: `crates/core/tests/interaction_device_batch.rs`

- [ ] **Step 1: Write a complete fake call-log test**

Seed two targets for one UDID. Require one non-blocking `DeviceExclusive` lease, install-only inspection/repair, one atomic `UiWithStream` upgrade at a time, and identity on each target. A selected Comment makes the final post-identity context fresh-text. Assert there is no call to generic preflight/repair, no session before the foreground transition, no MJPEG before session, and no optional action before identity success.

```rust
#[tokio::test]
async fn batch_uses_one_owner_and_runs_identity_before_optional_effects() {
    let fixture = device_batch_fixture()
        .with_clipboard_mode(ClipboardAccessMode::TargetBackgroundSafe)
        .with_two_targets()
        .with_comment_on_second();
    let report = fixture.run().await.unwrap();
    assert_eq!(report.processed_assignment_ids.len(), 2);
    assert_eq!(fixture.driver.call_log(), [
        "try_owner:interaction", "park_background", "inspect", "upgrade:ui_stream",
        "bootstrap:fresh_text", "foreground:tiktok", "session:fresh_text",
        "mjpeg:first_frame", "open:target-1",
        "identity:target-1", "watch:target-1", "like:target-1", "close_transient",
        "open:target-2", "identity:target-2", "comment:target-2",
        "close_transient", "release", "restore_background_if_budgeted",
    ]);
}
```

Add a separate `AgentForegroundRequired` case proving each sentinel/read-back switch
returns to TikTok with PID/bundle evidence and constructs the final ordinary or fresh
text session from that target's persisted sampled actions before starting its new
MJPEG generation. No session that predates the Agent foreground switch may execute
an optional action.

Add one explicit-retry case seeded with identity attempt 1 Confirmed and a retryable
Failed Like. The G1 retry transaction must append identity attempt 2 as Pending/None
plus Like attempt 2. Add a second case whose first identity attempt exhausted Opening
or was interrupted before Copy Link with `identity_copy_intent=None`; operator retry
must work without a prior Confirmed anchor and append a fresh identity attempt plus
all still-selected no-intent work. Each batch
call log must show opening attempt 1 for the new identity attempt, one new Copy Link
proof, and only then optional effects; it must never construct `VerifiedTargetContext`
from an old row. Add blocked counterparts for an issued deterministic
`Failed/TargetUnverified` and `Uncertain/TargetIdentityAmbiguous`, both with zero
new open/gesture calls.

Add paused-clock timing tests using the immutable G1 execution snapshot. Give the
first target two actions with distinct `delay_before_ms`, give its Watch action an
exact `watch_duration_ms`, and give both assignments distinct
`target_delay_after_ms`. Assert each action delay finishes before that action starts,
Watch receives exactly the persisted duration, the first target delay occurs only
after its transient UI is closed and before the second target opens, and the final
target delay is never awaited. Advance the clock partway through a delay and request
cancellation; the pending action must become `Skipped/CancelledBeforeStart` and its
assignment must reduce to `Cancelled` without opening UI. Reload a
durable retry row and prove it reuses the same three timing fields without invoking a
sampler:

```rust
#[tokio::test(start_paused = true)]
async fn persisted_timing_is_exact_and_target_delay_is_between_targets_only() {
    let fixture = device_batch_fixture()
        .with_two_targets()
        .with_persisted_timing(
            [17, 23],
            [Some(5_000), None],
            [41, 9_999],
        );
    let run = tokio::spawn(fixture.run());
    fixture.clock().advance_until_idle().await;
    run.await.unwrap().unwrap();

    assert_eq!(fixture.actions.observed_delay_before_ms(), [17, 23]);
    assert_eq!(fixture.actions.observed_watch_duration_ms(), [5_000]);
    assert_eq!(fixture.driver.observed_target_delays_ms(), [41]);
    assert_eq!(fixture.sampler.call_count(), 0);
}
```

- [ ] **Step 2: Add fail-closed capability, timing, and cancellation tests**

Test busy/offline owner, unavailable stream capacity, missing open URL, missing identity, missing Required Like/Follow/Comment, unsupported geometry, Probability-selected unsupported action, cancellation before lease, cancellation during every persisted delay, cancellation between actions, target mismatch, assignment/context mismatch, and an action row owned by another assignment. The two ownership mismatches perform zero frame/session calls. Missing mandatory capability skips the assignment before optional side effects. A missing Probability-selected action is skipped while independent supported actions continue and the assignment becomes Partial. Invalid timing snapshots fail closed: action and inter-target delays must be `0..=60_000`, Watch requires `Some(1_000..=300_000)`, every non-Watch action requires `None`, and dispatch never substitutes defaults or samples again. Also reject a batch with two current identity attempts, an assignment current-attempt/intent projection that differs from its highest identity row, a current identity row whose intent is already Issued, a retry batch that omits its new Pending identity attempt, or an opening history whose attempt numbers/budget are invalid.

- [ ] **Step 3: Verify RED**

```powershell
cargo test -p riviu-core --test interaction_device_batch -- --nocapture
```

- [ ] **Step 4: Implement `CoordinatedInteractionBatchExecutor`**

```rust
pub struct CoordinatedInteractionBatchExecutor {
    pub control: Arc<DeviceControlPlane>,
    pub actions: Arc<dyn TikTokActionExecutor>,
}

#[async_trait::async_trait]
impl InteractionBatchExecutor for CoordinatedInteractionBatchExecutor {
    async fn execute_device_batch(
        &self,
        batch: DeviceBatchExecution,
        progress: Arc<dyn InteractionProgress>,
        cancellation: Arc<dyn CancellationProbe>,
    ) -> Result<BatchExecutionReport, ExecutorError>;
}
```

Implementation order is fixed through Gate 0's single `DeviceControlPlane`: non-blocking `try_acquire_exclusive`; park/invalidate the device's background producer; inspect metadata/auth/transport/geometry without session/stream; install-only repair only for missing/mismatched app; `reserve_ui_capacity` for the atomic upgrade; foreground TikTok; create the profile-approved session; `start_reserved_stream` and await first frame; start/finish durable Opening attempt 1 and, only for an eligible pre-intent result, attempt 2; execute the highest Pending identity action; Watch/Like/Follow/Comment highest attempts in persisted ordinal order; persist each outcome; close transient UI; release and restore only an eligible budgeted tile. Do not retain parallel coordinator, semaphore, stream-budget, or runtime fields in this executor. Keep no SQLite transaction open during any of these awaits.

For every assignment, load the highest action attempt per ordinal, verify the assignment projection matches the mandatory current identity row, and require that row first. `action_started(identity_action_run_id)` precedes Opening. Each `open_url` is bracketed by `start_opening_attempt`/`finish_opening_attempt`, with `MAX_OPENING_ATTEMPTS_PER_IDENTITY=2`; the retry is an in-process, same-lease action only while both current intent values remain None. Opening success does not satisfy identity. Only a newly Confirmed current identity attempt constructs `VerifiedTargetContext { identity_action_run_id, identity_attempt_no, .. }`. A prior Confirmed attempt remains audit history and cannot authorize retried effects. Recovery never invokes this loop for a Running/Interrupted opening row; a later explicit operator retry may append a new identity attempt when the interrupted row remained no-intent.

The adapter consumes G1 timing fields verbatim. Immediately before every planned
action, call one `sleep_persisted_delay(action.delay_before_ms, ...)` that uses a
monotonic deadline and polls the existing `CancellationProbe` in bounded slices;
zero means no sleep. It never adds jitter, samples, rounds, catches up, or applies the
delay twice on an in-process retry. A durable retry is a new G1 action row and uses
the copied persisted value from that row. Pass
`Duration::from_millis(action.watch_duration_ms.unwrap() as u64)` to `watch_target`
for Watch and reject a missing value; reject a value on every other action. While
Watch is running, a scoped cancellation poller drives its existing stop flag and is
joined before the next action, so cancellation is observed during the full watch
rather than only at its boundaries. After closing and persisting one target, await
`assignment.target_delay_after_ms` only when another assignment remains; cancellation
during that wait prevents the next target from opening. The final assignment's delay
is intentionally ignored. All delay/cancellation polling occurs without a database
transaction or mutable store guard held.

- [ ] **Step 5: Implement `FrameVerifiedTikTokActionExecutor`**

Map `PreparedAction.kind` to the shared facade. Reject any call whose `VerifiedTargetContext.identity_evidence` is empty, whose identity action ID/attempt is not the current Confirmed row, whose assignment/action ownership does not match, or whose current geometry differs from the qualified tuple. A matched identity and a ready feed are separate facts: after identity cleanup, require a newer current-generation Feed frame with a positively located rail before constructing the optional-action context. If Share remains open, invoke the one bounded `close_share_drawer` primitive; if that still lacks `share_closed` plus feed/rail evidence, retain the identity result for audit but finish/skip all optional actions for that assignment with typed `TargetChanged` and zero action taps. Use `ScreenWatcher::run_suppressible()` around Share and Comment, but keep classification running. Convert `TargetIdentityResult`, `VerifiedEffect`, and `CommentDelivery` exhaustively to G1 outcomes: deterministic identity mismatch becomes `Failed/TargetUnverified`, tap/read ambiguity becomes `Uncertain/TargetIdentityAmbiguous`, and `Uncertain` is never flattened to Failed.

For Comment, call `prepare_and_persist_comment` before opening its drawer. Pass the
current stable MJPEG frame, the immutable
`assignment.effective_settings.ai_instruction` and
`assignment.effective_settings.fallback_comments`, and a domain-separated selection
seed derived by SHA-256 from `assignment.assignment_seed` and
`b"comment-fallback-v1"`. `FrameVerifiedTikTokActionExecutor` owns an injected
`Arc<dyn CommentGenerator>`; the batch layer does not query desktop settings or
downcast the trait object. After preparation has committed, call
`send_prepared_comment` with those exact stored bytes. A restart with
`Some(Comment(...))` bypasses generation and fallback selection completely.

- [ ] **Step 6: Run batch and campaign-core regression tests**

```powershell
cargo test -p riviu-core --test interaction_device_batch -- --nocapture
cargo test -p riviu-core --test interaction_dispatcher -- --nocapture
cargo test -p riviu-core --test interaction_recovery -- --nocapture
```

Expected: one shared owner controls each batch; capacity comes only from G0; every initial/retry batch confirms its own current identity attempt before optional actions; Opening retry is bounded and durable; state aggregation remains durable.

- [ ] **Step 7: Commit the batch executor**

```powershell
git add crates/core/src/interaction/device_batch_executor.rs crates/core/src/interaction/mod.rs crates/core/src/interaction/executor.rs crates/core/src/interaction/types.rs crates/ios-driver/src/interaction_runtime.rs crates/ios-driver/src/mock.rs crates/core/tests/interaction_device_batch.rs
git diff --cached --name-only
git commit -m "feat(core): execute qualified TikTok device batches"
```

---

### Task 10: Migrate Nurture To The Shared Rail And Action Facade

**Files:**
- Modify: `crates/core/src/nurture/actions.rs`
- Modify: `crates/core/src/nurture/mod.rs`
- Modify: `crates/core/src/nurture/recovery.rs`
- Modify: `crates/core/tests/real_frames.rs`
- Modify: `apps/desktop/src-tauri/src/bin/live_nurture_test.rs`

- [ ] **Step 1: Add Nurture parity tests before deleting duplicate code**

Retain current Like/Comment/recovery tests and add Follow outcome tests. Use the same frame/session fixtures to execute the old-result adapter and the shared facade, then compare counters, no-tap cases, evidence thresholds, and text-recovery decisions.

- [ ] **Step 2: Replace last-known/fallback coordinates**

Change the feed loop to call `screen::locate_action_rail` on the current frame. A positive white-chain result handles an already-followed author. An ambiguous/absent result watches/swipes without Like/Follow/Comment taps. Remove runtime use of `ActionRail::fallback()` and the mutable last-known rail.

- [ ] **Step 3: Delete duplicate Like/Follow/Comment mechanics**

Keep thin Nurture mapping methods if they make logging/counters readable, but all point derivation, frame waits, desired-state tests, drawer handling, armed/disarmed checks, and text-health decisions must call `tiktok_actions`. Keep Nurture's random human delays around facade invocation; Interaction uses its persisted assignment seed.

- [ ] **Step 4: Run Nurture and workspace core tests**

```powershell
cargo test -p riviu-core nurture:: -- --nocapture
cargo test -p riviu-core --test real_frames -- --nocapture
cargo test -p riviu-core --lib -- --nocapture
cargo build -p riviu-managers-phone --bin live_nurture_test
```

Expected: all prior Nurture behavior tests pass, followed-author rail is current-frame located, and no production path uses fallback coordinates.

- [ ] **Step 5: Commit the migration**

```powershell
git add crates/core/src/nurture/actions.rs crates/core/src/nurture/mod.rs crates/core/src/nurture/recovery.rs crates/core/tests/real_frames.rs apps/desktop/src-tauri/src/bin/live_nurture_test.rs
git diff --cached --name-only
git commit -m "refactor(core): reuse verified TikTok actions in nurture"
```

---

### Task 11: Add Independent G2 Capability Keys And Desktop Composition

**Files:**
- Modify: `crates/core/src/device_capabilities.rs`
- Modify: `sidecars/wda/interaction-capabilities.schema.json`
- Modify: `sidecars/wda/interaction-capabilities.json`
- Modify: `crates/ios-driver/src/interaction_runtime.rs`
- Modify: `apps/desktop/src-tauri/src/state.rs`
- Modify: `apps/desktop/src/types.ts`
- Modify: `crates/core/tests/interaction_device_batch.rs`

- [ ] **Step 1: Write qualification equality and default-deny tests**

Add one G2 production-runtime capability plus independent slots for Like, Follow, and Comment. The runtime key binds the production executor version, G0 identity-contract ID, identity detector-set digest, and Gate G2 live-report SHA to the exact base tuple; it is the only key that can make `TargetIdentityCopyLink` and Watch ready. Each side-effect key extends that same tuple with detector set/version; Comment also includes text-route contract and comment-composer detector version. Change one field at a time and assert only the affected action is unavailable. An empty registry must construct `ExecutionDisabled`, not a partially active executor.

Also test the finalized G1 `InteractionCapabilities` provider: its `CapabilitySnapshot.actions` map contains every `PlannedActionKind`. G0 open/clipboard/reference-identity contracts without the G2 runtime key leave `TargetIdentityCopyLink` and Watch `Deferred/GateNotQualified`; the exact G2 runtime key makes those two ready, independently qualified G2 side effects map to `Ready`, and Save/Repost/DirectMessage remain explicit `Deferred/GateNotQualified`. A missing key is an invariant error, never implicit support.

- [ ] **Step 2: Verify RED**

```powershell
cargo test -p riviu-core device_capabilities::tests::g2 -- --nocapture
cargo test -p riviu-ios-driver interaction_capability_registry -- --nocapture
```

- [ ] **Step 3: Extend the typed capability contract**

```rust
pub struct UiCapabilities {
    pub open_url: Option<OpenUrlCapability>,
    pub clipboard: Option<ClipboardCapability>,
    pub target_identity_copy_link: Option<TargetIdentityCapability>,
    pub interaction_runtime: Option<VerifiedInteractionRuntimeCapability>,
    pub like: Option<VerifiedActionCapability>,
    pub follow: Option<VerifiedActionCapability>,
    pub comment: Option<VerifiedActionCapability>,
}

pub struct VerifiedInteractionRuntimeCapability {
    pub executor_version: String,
    pub identity_contract_id: String,
    pub identity_detector_set_sha256: String,
    pub live_report_sha256: String,
}
```

Gate G2 does not add Save, Repost, or Direct Message fields or schema entries; Gate G4 extends this struct and the same registry when those contracts exist. Until then, the G1 `CapabilitySnapshot` reports those planned kinds as typed `Deferred/GateNotQualified`. Runtime status must report a typed reason such as `gate_not_qualified`, `unsupported_geometry`, or `text_route_unqualified`; it must not infer support from `AgentStatus.features`.

- [ ] **Step 4: Compose but do not over-enable**

`AppState::bootstrap` may construct the G0-backed adapter object when the exact base tuple is qualified, but it keeps production execution disabled until `interaction_runtime` matches that same tuple and verified Gate G2 report. Without it, `TargetIdentityCopyLink` and Watch remain deferred and `interaction_start`/`interaction_schedule` return typed `gate_not_qualified` before a lease or device call; campaign drafts and monitoring remain usable. For each claimed batch, the executor reads the persisted planned action set plus each policy origin and negotiates every corresponding action capability against the same live tuple before acquiring device work. A missing or mismatched `Required` capability skips that assignment before any optional side effect with typed `SkippedUnsupported`. A missing capability for a Probability-selected action skips only that action, continues independent supported actions, and makes the assignment `Partial`; `Off` and false probability samples remain `NotPlanned`. Do not expose a real-device start path merely because fixture tests pass.

Construct one `FrameVerifiedTikTokActionExecutor` with an
`Arc<dyn CommentGenerator>` adapter over the existing OpenAI-compatible client. The
adapter owns only provider credentials/model/pricing configuration; every call takes
the campaign's immutable `ai_instruction` and `fallback_comments` from the
`AssignmentExecution` argument added in Task 1. It never captures mutable campaign
defaults in `AppState`, and an unavailable provider still reaches the persisted
fallback path without disabling independent Like/Follow actions.

Implement the existing G1 port, not a second snapshot API:

```rust
#[async_trait::async_trait]
impl InteractionCapabilities for RegistryBackedInteractionCapabilities {
    async fn snapshot(&self, udid: &str) -> Result<CapabilitySnapshot, ExecutorError>;
}
```

The implementation derives the current device/app/artifact tuple through G0 inspection, negotiates the single strict registry once, and converts the resulting `UiCapabilities` into the complete G1 action map. It does not acquire a screen-changing lease, start a session, or start MJPEG.

- [ ] **Step 5: Verify GREEN and commit**

```powershell
cargo test -p riviu-core device_capabilities::tests::g2 -- --nocapture
cargo test -p riviu-ios-driver interaction_capability_registry -- --nocapture
cargo test -p riviu-managers-phone state::tests -- --nocapture
git add crates/core/src/device_capabilities.rs sidecars/wda/interaction-capabilities.schema.json sidecars/wda/interaction-capabilities.json crates/ios-driver/src/interaction_runtime.rs apps/desktop/src-tauri/src/state.rs apps/desktop/src/types.ts crates/core/tests/interaction_device_batch.rs
git diff --cached --name-only
git commit -m "feat(desktop): compose fail-closed verified actions"
```

Expected: the production JSON still has no G2 PASS entries at this checkpoint.

---

### Task 12: Build And Run The Fixed Gate G2 Mac/Device Probe

**Files:**
- Create: `apps/desktop/src-tauri/src/bin/live_interaction_verified_actions.rs`
- Create: `tools/interaction-gate2/verify_report.py`
- Create: `tools/interaction-gate2/test_verify_report.py`
- Create: `docs/re/interaction-gate2/README.md`
- Create after PASS: `docs/re/interaction-gate2/gate-2.json`
- Create after PASS: `docs/re/interaction-gate2/gate-2.md`
- Modify after PASS: `sidecars/wda/interaction-capabilities.json`
- Modify: `AGENTS.md`

- [ ] **Step 1: Write report-verifier fixture tests**

Reject `FIXTURE_ONLY`, missing exact tuple fields, lowered sample counts, reused identity across targets or explicit retries, Copy Link count other than one per identity attempt, Opening count outside one-or-two with no durable attempt rows, absent frame hashes, HTTP-only evidence, raw token/UDID/clipboard bytes/comment text, missing cleanup, changed production IPA/manifest hashes, or recovery/ambiguity cases whose exact harness fault descriptor is absent. Accept only `LIVE_MAC_DEVICE` with all fixed cases present. Add failpoint tests proving publication snapshots the original registry and every evidence destination exactly once, rolls all of them back when any replace, registry parse, focused check, staging, or simulated commit step fails, and seals the transaction only after a successful commit. A second qualification entry in the same transaction must not overwrite the original snapshot.

```powershell
python -m unittest discover -s tools/interaction-gate2 -p "test_*.py" -v
```

Expected before implementation: import/module failure.

- [ ] **Step 2: Implement the fixed harness**

The harness accepts device/secrets only through environment inputs `UDID`, `RIVIU_RTMMO_TOKEN`, and `RIVIU_AI_API_KEY`. Its only CLI inputs are `--fixture-matrix` (normally `$RIVIU_GATE2_FIXTURES`) and `--report`; neither flag carries a token or raw device identifier. URLs, expected content IDs/post kinds, expected starting state, and controlled account labels stay in the controlled JSON matrix and outside source. The verifier requires the exact schema and minimum array sizes below; counts are compiled constants, not CLI overrides:

```json
{
  "schemaVersion": 1,
  "identity": {
    "video": [], "photo": [], "short": [], "openingRetry": [],
    "deterministicMismatch": [], "postIntentAmbiguous": []
  },
  "like": { "unliked": [], "alreadyLiked": [] },
  "follow": { "unfollowed": [], "alreadyFollowed": [] },
  "comment": { "unicode": [], "textNotArmedRecovery": [], "postIntentAmbiguous": [] }
}
```

```rust
const IDENTITY_CASES_PER_KIND: usize = 3;
const OPENING_RETRY_CASES: usize = 1;
const IDENTITY_MISMATCH_CASES: usize = 1;
const IDENTITY_AMBIGUOUS_CASES: usize = 1;
const LIKE_TRANSITIONS: usize = 10;
const FOLLOW_TRANSITIONS: usize = 6;
const UNICODE_COMMENTS: usize = 5;
const TEXT_NOT_ARMED_RECOVERIES: usize = 1;
const POST_INTENT_AMBIGUOUS_CASES: usize = 1;
```

Require at least three distinct targets in each normal identity-kind array, one Opening-retry case, one deterministic identity-mismatch case, one post-intent identity-ambiguity case, ten `unliked`, one `alreadyLiked`, six `unfollowed`, one `alreadyFollowed`, five `unicode`, one `textNotArmedRecovery`, and one Comment `postIntentAmbiguous` case. Reject duplicate planned content IDs across normal identity cases. The mismatch case must finish `Failed/TargetUnverified`; the identity ambiguity case must finish `Uncertain/TargetIdentityAmbiguous`; both run zero optional effects. Published evidence stores only URL/account-label SHA-256 values, never raw URLs or labels.

For every normal identity/action target: open URL, Copy Link identity, watch at least five seconds, run only the requested test action, record before/after frames, and close transient UI. The deterministic mismatch and post-intent identity-ambiguity cases stop immediately after their typed identity outcome and prove zero Watch/optional-effect calls. Include an already-liked and already-followed desired-state case and background-stream budget accounting. Use MJPEG only for observation.

Make the otherwise nondeterministic fault cases explicit in the harness, not dependent on device luck:

- `OpeningRetryDecorator` withholds the first `open_url` before forwarding any device request, finishes durable Opening attempt 1 as `Failed/DeviceUnavailable`, appends attempt 2 with `retryReason=TransportBeforeOpen`, then delegates attempt 2. Attempt 2 must reach a newer target frame and the same identity action must perform exactly one Copy Link tap. A third Opening attempt or an Opening call after identity intent fails the case.
- `IdentityReadAmbiguityDecorator` delegates exactly one Copy Link tap, then withholds the first post-intent clipboard result until the fixed deadline. The action must finish `Uncertain/TargetIdentityAmbiguous`, remain retry-ineligible, and run zero optional effects.
- `TextNotArmedSessionDecorator` returns the same successful text ACK shape without forwarding exactly the first two pre-intent `type_text` calls. The real MJPEG composer therefore remains disarmed twice. It disables itself after the fresh-context generation changes, then the third attempt delegates exact Unicode bytes to the real fresh session and must send successfully.
- `PostSendFrameLossDecorator` delegates exactly one real Send tap, records that intent preceded it, then withholds only subsequent frames from the action waiter until its fixed deadline. The action must finish `Uncertain/TextNotSent`, emit `effectTapCount=1`, and remain retry-ineligible. Cleanup reads from the underlying stream directly so it can still prove process/port shutdown.

The report records only the fixed descriptors `open_transport_before_request_v1`, `identity_post_tap_read_loss_v1`, `text_ack_without_delivery_v1`, and `post_send_frame_loss_v1`, affected action/opening-attempt IDs, generation changes, and bounded timestamps. The verifier rejects any other injection, injection during normal cases, or use of injected cases as frame-success evidence.

The verifier also implements `publish-and-qualify`, `verify-published`, `rollback-publication`, and `seal-publication`. `publish-and-qualify` recomputes the sanitized report, derives only passing exact-tuple entries, snapshots the pre-publication registry plus prior evidence bytes/absence into one ignored transaction directory, atomically replaces the evidence pair and registry, and retains that transaction until the Git commit succeeds. Every later failure path uses `rollback-publication`; repeated action qualification within one transaction never replaces its original snapshot.

- [ ] **Step 3: Verify the harness compiles and fixture verifier passes**

```powershell
cargo build -p riviu-managers-phone --bin live_interaction_verified_actions
python -m unittest discover -s tools/interaction-gate2 -p "test_*.py" -v
```

Expected: Windows compilation and synthetic verifier tests pass; report status remains `PENDING_MAC_DEVICE`.

- [ ] **Step 4: Run on the Mac with all other XCTest owners stopped**

```bash
test -n "$UDID"
test -n "$RIVIU_RTMMO_TOKEN"
test -n "$RIVIU_AI_API_KEY"
test -f "$RIVIU_GATE2_FIXTURES"
tidevice -u "$UDID" kill notes.3u || true
tidevice -u "$UDID" kill com.mrph.svc || true
tidevice -u "$UDID" kill com.riviu.managersphone.agent.xctrunner || true
cargo run -p riviu-managers-phone --release --bin live_interaction_verified_actions -- \
  --fixture-matrix "$RIVIU_GATE2_FIXTURES" \
  --report target/interaction-gate2/gate-2.json
python3 tools/interaction-gate2/verify_report.py \
  --input target/interaction-gate2/gate-2.json \
  --production-ipa sidecars/wda/RiviuAgent.ipa \
  --production-manifest sidecars/wda/agent-manifest.json
```

Expected: direct video, photo, and short link identities match copied content IDs; Like/Follow desired-state transitions and Unicode Comment evidence pass; two-strike recovery replaces both handles; cleanup leaves no owned relay/stream/Agent child; redaction passes.

- [ ] **Step 5: Run complete regressions before publication or qualification**

```bash
REGISTRY_BEFORE="$(shasum -a 256 sidecars/wda/interaction-capabilities.json | awk '{print $1}')"
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace -- --nocapture
npm --prefix apps/desktop test -- --run
npm --prefix apps/desktop run build
test "$(shasum -a 256 sidecars/wda/RiviuAgent.ipa | awk '{print $1}')" = "8a24847099495ff70b998522692c43f00dd16b90f698bda6953a73f5d33002ea"
test "$(shasum -a 256 sidecars/wda/agent-manifest.json | awk '{print $1}')" = "e98a549af4c061556effd36424e7732219e1a6d262bcf1f259279975024b6e1a"
python3 -m unittest discover -s tools/interaction-gate2 -p "test_*.py" -v
git diff --check
test "$(shasum -a 256 sidecars/wda/interaction-capabilities.json | awk '{print $1}')" = "$REGISTRY_BEFORE"
```

Expected: all commands pass while `docs/re/interaction-gate2/` and `sidecars/wda/interaction-capabilities.json` remain byte-identical to their pre-gate state; production Agent hashes remain `8a24847099495ff70b998522692c43f00dd16b90f698bda6953a73f5d33002ea` and `e98a549af4c061556effd36424e7732219e1a6d262bcf1f259279975024b6e1a`.

- [ ] **Step 6: Publish evidence and qualify through one retained transaction**

The verifier derives registry entries from the verified report SHA-256. Add `interaction_runtime` only when every production identity-kind case, Watch timing case, lifecycle order, and cleanup check passes through the real Rust executor; a reference G0 probe is insufficient. A Like failure leaves Like absent while an independent Follow PASS may qualify Follow; Comment requires its own text and recovery PASS. No side-effect entry is usable without the matching runtime entry. Never hand-edit a PASS field.

```bash
set -Eeuo pipefail
TX=target/interaction-gate2/publication-transaction
rollback_gate2() {
  trap - ERR INT TERM
  if test -d "$TX"; then
    python3 tools/interaction-gate2/verify_report.py rollback-publication \
      --transaction "$TX"
  fi
}
trap rollback_gate2 ERR INT TERM
rm -rf "$TX"
python3 tools/interaction-gate2/verify_report.py publish-and-qualify \
  --input target/interaction-gate2/gate-2.json \
  --production-ipa sidecars/wda/RiviuAgent.ipa \
  --production-manifest sidecars/wda/agent-manifest.json \
  --output-dir docs/re/interaction-gate2 \
  --registry sidecars/wda/interaction-capabilities.json \
  --transaction "$TX"
python3 tools/interaction-gate2/verify_report.py verify-published \
  --output-dir docs/re/interaction-gate2 \
  --registry sidecars/wda/interaction-capabilities.json
cargo test -p riviu-ios-driver interaction_capability_registry -- --nocapture
trap - ERR INT TERM
```

Expected: evidence and only the passing exact-tuple capabilities are visible together, the retained transaction still contains the one original snapshot, and any command failure restores both evidence and registry before exiting.

- [ ] **Step 7: Update handoff and commit actual gate state**

Before editing, run `mkdir -p target/interaction-gate2 && cp AGENTS.md target/interaction-gate2/AGENTS.before-publication.md`. Then record in `AGENTS.md`: commands/counts, exact qualification tuple, action-by-action PASS/PENDING status, session-before-stream evidence, no-retry classifications, remaining G4 actions disabled, and rollback commit/artifacts. If editing, staging, or commit fails, restore the snapshot and invoke the retained publication rollback before stopping.

```bash
set -Eeuo pipefail
TX=target/interaction-gate2/publication-transaction
HANDOFF_BEFORE=target/interaction-gate2/AGENTS.before-publication.md
rollback_gate2_commit() {
  trap - ERR INT TERM
  git restore --staged -- apps/desktop/src-tauri/src/bin/live_interaction_verified_actions.rs tools/interaction-gate2 docs/re/interaction-gate2 sidecars/wda/interaction-capabilities.json AGENTS.md || true
  if test -f "$HANDOFF_BEFORE"; then cp "$HANDOFF_BEFORE" AGENTS.md; fi
  if test -d "$TX"; then
    python3 tools/interaction-gate2/verify_report.py rollback-publication \
      --transaction "$TX"
  fi
}
trap rollback_gate2_commit ERR INT TERM
test -f "$HANDOFF_BEFORE"
git add apps/desktop/src-tauri/src/bin/live_interaction_verified_actions.rs tools/interaction-gate2 docs/re/interaction-gate2 sidecars/wda/interaction-capabilities.json
git add -p AGENTS.md
git diff --cached --name-only
git diff --cached --check
git commit -m "test(interaction): qualify verified TikTok actions"
trap - ERR INT TERM
python3 tools/interaction-gate2/verify_report.py seal-publication \
  --transaction "$TX"
rm -f "$HANDOFF_BEFORE"
```

## Gate G2 Completion Criteria

- G0 and G1 contracts are reused without a second device lock, stream semaphore, or SQLite writer.
- Production `TargetIdentityCopyLink` and Watch stay disabled until the exact G2 runtime entry attests the real Rust executor; G0 reference evidence alone never enables them.
- Every initial or explicit-retry batch that reaches an optional side effect first passes its own new disclosed Copy Link identity attempt; prior Confirmed identity is audit history only.
- Opening has exactly two durable attempts maximum per identity action, retries only before identity intent, and never resumes automatically after process loss.
- Watch uses elapsed time plus target-ready frame samples; Like and Follow use current-frame coordinates and desired-state evidence.
- Comment text is stored before opening UI; Send intent is committed after armed evidence and before one Send tap; `TextNotSent` is Uncertain and not replayed.
- Two consecutive `TextNotArmed` results replace session, stream generation, executor feed, and watcher handle inside the existing lease.
- Nurture uses the same rail/action/comment primitives and all existing regressions pass.
- G2 evidence and capability entries are promoted only after full regression, share one retained rollback transaction, and seal that transaction only after the reviewed commit succeeds.
- Like, Follow, and Comment are independently enabled only for exact reviewed live tuples; Save, Repost, and Direct Message remain disabled for Gate G4.
- Production IPA/manifest remain byte-identical and the rollback path is documented.

## Assumptions And Risks

- G0/G1 are hard prerequisites, including the live Gate 0 open/identity tuple. This plan does not compensate for a missing shared lease, stream budget, strict capability registry, or durable action journal.
- Rail and Share detectors are qualified per exact geometry/layout tuple. A new iPhone, orientation, TikTok build, or detector set fails closed until fixture and live qualification are repeated.
- Gate G2 requires controlled fixture posts/accounts that can be reset to the declared Like/Follow/comment states; an uncontrolled production feed cannot satisfy the fixed matrix.
- Comment generation requires the configured AI runtime. Its prepared output is persisted once, but provider availability and cost remain operational dependencies for Comment only.
- Clipboard behavior must match the exact qualified `ClipboardAccessMode`. `AgentForegroundRequired` always replaces the target session/stream context after clipboard access.
- Production Agent artifacts remain unchanged. Windows fixture results and a pending Mac/device gate do not enable a production capability entry.
