# TikTok Interaction Campaign Core Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the durable campaign-domain, planning, persistence, dispatch, recovery, pagination, artifact-metadata, and Tauri API core for target-driven TikTok interactions without enabling unqualified device actions.

**Architecture:** `riviu-core` owns immutable campaign plans and treats SQLite as the only runnable-work source. Pure planner and aggregation modules are separated from a serialized blocking `InteractionStore`; a durable dispatcher claims database rows and calls Gate 0's injected `InteractionBatchExecutor`, whose shared `DeviceControlPlane` implementation alone owns device leases and stream capacity.

**Tech Stack:** Rust 2021, Tokio, rusqlite/SQLite WAL, serde/serde_json, chrono, uuid, rand, sha2, Tauri 2 commands/events, TypeScript 6, Vitest.

---

## Execution Preconditions

- Execute from an isolated worktree created with the `using-git-worktrees` skill after the current Project 2/runtime/spec work is committed to an integration baseline. The present shared checkout is dirty; do not stage, reset, or overwrite its unrelated changes.
- Read `AGENTS.md` and `docs/superpowers/specs/2026-07-29-tiktok-interaction-campaign-design.md` before every task. If their session-before-stream, capability, proxy, or persistence invariants changed, stop and reconcile this plan first.
- Gate 0 must be complete and passing before Task 1 begins. This plan consumes its injected `InteractionBatchExecutor`/shared `DeviceControlPlane` contract and does not introduce a second lock, semaphore, or capacity counter. Re-run Gate 0's control-plane tests before Task 10. This plan does not implement `open_url`, Copy Link identity proof, stream-budget transfer, verified TikTok actions, or production capability enablement.
- Production `sidecars/wda/RiviuAgent.ipa` and `sidecars/wda/agent-manifest.json` remain untouched.
- Each commit command below stages only the listed paths in the isolated worktree. Run `git diff --cached --name-only` before every commit and remove any path not listed for that task from the index without changing its working-tree contents.

## File Map

**Create**

- `crates/core/src/interaction/mod.rs`: public campaign-core exports and service composition.
- `crates/core/src/interaction/types.rs`: IDs, requests, immutable snapshots, states, result codes, page DTOs, and canonical request hashing.
- `crates/core/src/interaction/links.rs`: strict TikTok URL parsing, bounded redirects, normalization, and per-line results.
- `crates/core/src/interaction/planner.rs`: pure actor/target expansion and one-time probability sampling.
- `crates/core/src/interaction/aggregate.rs`: exhaustive assignment and campaign state aggregation.
- `crates/core/src/interaction/schema.rs`: numbered additive SQLite migration and indexes.
- `crates/core/src/interaction/store.rs`: serialized blocking repository, transactions, claims, transitions, idempotency, and pagination.
- `crates/core/src/interaction/cursor.rs`: versioned opaque keyset cursor codec.
- `crates/core/src/interaction/progress.rs`: executor-to-store transition and evidence port.
- `crates/core/src/interaction/artifacts.rs`: managed artifact metadata, atomic finalization, retention, quota, and reconciliation.
- `crates/core/src/interaction/executor.rs`: stable Gate 0 batch, verified-action, progress, and capability ports.
- `crates/core/src/interaction/scheduler.rs`: due-schedule materialization and per-device batch ordering.
- `crates/core/src/interaction/dispatcher.rs`: durable dispatch claims, orchestration, and crash audit.
- `crates/core/tests/interaction_schema.rs`: migration, PRAGMA, constraints, and existing-database tests.
- `crates/core/tests/interaction_links.rs`: direct/short URL, redirect, normalization, and revalidation tests.
- `crates/core/tests/interaction_store.rs`: request idempotency and concurrent-writer tests.
- `crates/core/tests/interaction_plan_store.rs`: immutable actor/assignment/action plan transaction tests.
- `crates/core/tests/interaction_transitions.rs`: progress, intent, cancel, retry, and aggregation-transaction tests.
- `crates/core/tests/interaction_pagination.rs`: stable bounded keyset pagination tests.
- `crates/core/tests/interaction_dispatcher.rs`: claim, schedule, batching, and disabled-executor tests.
- `crates/core/tests/interaction_recovery.rs`: persisted crash-point audit tests.
- `crates/core/tests/interaction_artifacts.rs`: path, hash, retention, quota, and crash-reconciliation tests.
- `crates/core/tests/interaction_campaign_e2e.rs`: public-service fake-executor durability acceptance tests.
- `apps/desktop/src-tauri/src/interaction_commands.rs`: thin Tauri command boundary and command-helper tests.
- `apps/desktop/src/interactionApi.test.ts`: TypeScript invoke-contract tests.

**Modify**

- `Cargo.toml`: expose no new workspace package; reuse the existing workspace `sha2` dependency.
- `crates/core/Cargo.toml`: add `sha2.workspace = true`.
- `crates/core/src/lib.rs`: register and re-export the interaction core.
- `crates/core/src/db.rs`: configure every connection and invoke the numbered interaction migration.
- `crates/core/src/events.rs`: add revisioned `InteractionUpdated` events.
- `apps/desktop/src-tauri/src/state.rs`: own the store/dispatcher and start its background loop.
- `apps/desktop/src-tauri/src/lib.rs`: register interaction commands.
- `apps/desktop/src/types.ts`: mirror public interaction DTOs.
- `apps/desktop/src/api.ts`: expose interaction command wrappers.
- `AGENTS.md`: record durable-dispatch, aggregation, SQLite, proxy-source, and production-disable invariants after implementation passes.

**Do Not Modify In This Plan**

- `crates/core/src/nurture/**`
- `crates/core/src/screen.rs`
- `crates/core/src/driver.rs`
- `crates/ios-driver/**`
- `sidecars/wda/**`
- `apps/desktop/src/App.tsx` or any visual component

---

### Task 1: Define The Versioned Campaign Domain

**Files:**
- Create: `crates/core/src/interaction/mod.rs`
- Create: `crates/core/src/interaction/types.rs`
- Modify: `crates/core/src/lib.rs`
- Modify: `crates/core/Cargo.toml`

- [ ] **Step 1: Add failing serialization and normalization tests**

Create minimal `crates/core/src/interaction/mod.rs` containing `pub mod types;`, add `pub mod interaction;` to `crates/core/src/lib.rs`, then place these tests in `crates/core/src/interaction/types.rs` under `#[cfg(test)]`. Do not add the production types yet, so the focused run compiles the test module and fails on unresolved names:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probability_boundaries_canonicalize_before_hashing() {
        assert_eq!(ActionPolicy::Probability { percent: 0 }.canonical(), ActionPolicy::Off);
        assert_eq!(
            ActionPolicy::Probability { percent: 100 }.canonical(),
            ActionPolicy::Required
        );
        assert!(ActionPolicy::Probability { percent: 101 }.validate().is_err());
    }

    #[test]
    fn request_hash_ignores_request_id_but_covers_actor_order() {
        let mut first = fixture_resolved_request("request-a", vec!["actor-a", "actor-b"]);
        let mut retried = first.clone();
        retried.request.request_id = "request-b".into();
        assert_eq!(first.normalized_hash().unwrap(), retried.normalized_hash().unwrap());

        first.request.actor_selection = ActorSelection::Explicit {
            account_ids: vec!["actor-b".into(), "actor-a".into()],
        };
        assert_ne!(first.normalized_hash().unwrap(), retried.normalized_hash().unwrap());
    }

    #[test]
    fn camel_case_state_contract_is_stable() {
        assert_eq!(
            serde_json::to_string(&AssignmentStatus::WaitingCapacity).unwrap(),
            "\"waitingCapacity\""
        );
        assert_eq!(
            serde_json::to_string(&CampaignStatus::Interrupted).unwrap(),
            "\"interrupted\""
        );
    }
}
```

- [ ] **Step 2: Run the focused test and verify RED**

Run:

```powershell
cargo test -p riviu-core interaction::types::tests -- --nocapture
```

Expected: compilation fails on unresolved `ActionPolicy`, request, and state types; a zero-test/pass result means the module was not registered correctly.

- [ ] **Step 3: Complete the core type contract and exports**

Complete `crates/core/src/interaction/mod.rs`:

```rust
pub mod types;

pub use types::*;
```

Each later task adds its module declaration and re-export only when that file is created; Task 1 must compile with `types.rs` alone.

Create `crates/core/src/interaction/types.rs` with the following public contract. Keep enum spellings exact because SQLite JSON and Tauri use them:

```rust
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

pub type AccountId = String;
pub type CampaignId = Uuid;
pub type TargetId = Uuid;
pub type AssignmentId = Uuid;
pub type ActionRunId = Uuid;
pub type OpeningAttemptId = Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ActionPolicy {
    Off,
    Required,
    Probability { percent: u8 },
}

impl ActionPolicy {
    pub fn validate(&self) -> anyhow::Result<()> {
        if matches!(self, Self::Probability { percent } if *percent > 100) {
            anyhow::bail!("probability percent must be 0..=100");
        }
        Ok(())
    }

    pub fn canonical(&self) -> Self {
        match self {
            Self::Probability { percent: 0 } => Self::Off,
            Self::Probability { percent: 100 } => Self::Required,
            value => value.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum DistributionMode { All, RoundRobin }

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "mode", rename_all = "camelCase")]
pub enum ActorSelection {
    AllOnline,
    Explicit { account_ids: Vec<AccountId> },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "mode", rename_all = "camelCase")]
pub enum ScheduleMode {
    RunNow,
    Once { at: DateTime<Utc> },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum TikTokPostKind { Video, Photo }

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum AccountState { Active, Disabled }

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum AccountPlatform { TikTok }

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RecipientAllowlistEntry {
    pub normalized_handle: String,
    pub display_label: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum RecipientMode { Allowlist, RandomVisible }

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RecipientPolicy {
    pub mode: RecipientMode,
    pub allowlist: Vec<RecipientAllowlistEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "camelCase")]
pub enum PlannedActionKind {
    TargetIdentityCopyLink,
    Watch,
    Like,
    Follow,
    Comment,
    Save,
    Repost,
    DirectMessage,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum PlannedDecision { NotPlanned, Pending }

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum EffectIntent { None, Issued }

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum IdentityCopyIntent { None, Issued }

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum IdentityState { Pending, Confirmed, Unverified, Ambiguous }

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ActionStatus {
    NotPlanned, Pending, Running, Succeeded, AlreadySatisfied, NotConfirmed,
    Uncertain, Failed, Skipped, Interrupted,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum AssignmentStatus {
    Queued, WaitingCapacity, Preparing, Session, Stream, Opening, Verifying, Acting,
    Succeeded, Partial, Failed, SkippedUnavailable, SkippedUnsupported, Cancelled,
    Interrupted, Uncertain,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum CampaignStatus {
    Scheduled, Queued, Running, Succeeded, Partial, Failed, Cancelled, Interrupted, Missed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum CampaignResultCode {
    NoEligibleActors, NoRunnableAssignments, NoSupportedAssignments,
    TooManyAssignments, Cancelled, ProcessLost, AmbiguousOutcome,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum AssignmentResultCode {
    TargetChanged, TargetUnverified, TargetIdentityAmbiguous, UnsupportedCapability,
    UnsupportedGeometry, DeviceUnavailable, CancelledBeforeStart, ProcessLost,
    NoPositiveOutcome,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum RetryBlockedReason {
    AssignmentStatusIneligible,
    IdentityIssuedUnconfirmed,
    TargetUnverified,
    TargetIdentityAmbiguous,
    EffectIntentIssued,
    FinalNotConfirmed,
    NoRetryableActions,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ActorExclusionCode {
    Offline, DeviceBusy, AccountDisabled, IdentityUnavailable,
    AccountSwitchUnsupported, UnsupportedCapability,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ProxySnapshotSource { None, DeviceDefault }

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ProxyApplyCapability { UnsupportedUnsupervised, SupportedManaged }

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ProxyConfigurationState { Unassigned, ManualRequired, AppliedVerified }

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ProxyCheckState { Untested, Passed, Failed, Invalidated }

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct EffectiveProxySnapshot {
    pub proxy_id: Option<String>,
    pub configuration_revision: Option<String>,
    pub source: ProxySnapshotSource,
    pub apply_capability: ProxyApplyCapability,
    pub configuration_state: ProxyConfigurationState,
    pub endpoint_check: ProxyCheckState,
    pub manually_confirmed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ActorRunStatus {
    Selected, Eligible, Running, Succeeded, Partial, Failed, Interrupted, Uncertain,
    SkippedUnavailable, SkippedUnsupported,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AccountBinding {
    pub id: AccountId,
    pub platform: AccountPlatform,
    pub device_udid: String,
    pub slot_key: String,
    pub username: Option<String>,
    pub state: AccountState,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedTikTokTarget {
    pub original_url: String,
    pub normalized_url: String,
    pub resolved_url: String,
    pub target_key: String,
    pub content_id: String,
    pub author: Option<String>,
    pub kind: TikTokPostKind,
    #[serde(default)]
    pub overrides: InteractionOverrides,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TikTokTargetInput {
    pub url: String,
    #[serde(default)]
    pub overrides: InteractionOverrides,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct InteractionOverrides {
    pub watch_min_seconds: Option<u32>,
    pub watch_max_seconds: Option<u32>,
    pub watch: Option<ActionPolicy>,
    pub like: Option<ActionPolicy>,
    pub follow: Option<ActionPolicy>,
    pub comment: Option<ActionPolicy>,
    pub save: Option<ActionPolicy>,
    pub repost: Option<ActionPolicy>,
    pub direct_message: Option<ActionPolicy>,
    pub comment_instruction: Option<String>,
    pub recipient_policy: Option<RecipientPolicy>,
    pub action_delay_min_ms: Option<u32>,
    pub action_delay_max_ms: Option<u32>,
    pub target_delay_min_ms: Option<u32>,
    pub target_delay_max_ms: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct InteractionDefaults {
    pub watch_min_seconds: u32,
    pub watch_max_seconds: u32,
    pub watch: ActionPolicy,
    pub like: ActionPolicy,
    pub follow: ActionPolicy,
    pub comment: ActionPolicy,
    pub save: ActionPolicy,
    pub repost: ActionPolicy,
    pub direct_message: ActionPolicy,
    pub ai_instruction: String,
    pub fallback_comments: Vec<String>,
    pub recipient_policy: RecipientPolicy,
    pub action_delay_min_ms: u32,
    pub action_delay_max_ms: u32,
    pub target_delay_min_ms: u32,
    pub target_delay_max_ms: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct InteractionCampaignRequest {
    pub request_id: String,
    pub actor_selection: ActorSelection,
    pub distribution: DistributionMode,
    pub schedule: ScheduleMode,
    pub defaults: InteractionDefaults,
    pub targets: Vec<TikTokTargetInput>,
}

/// Internal-only input produced by `TikTokLinkResolver`; never deserialized
/// directly from a Tauri request.
#[derive(Debug, Clone)]
pub struct ResolvedCampaignInput {
    pub(in crate::interaction) request: InteractionCampaignRequest,
    pub(in crate::interaction) targets: Vec<ResolvedTikTokTarget>,
}

impl ResolvedCampaignInput {
    pub(in crate::interaction) fn normalized_hash(&self) -> anyhow::Result<String> {
        let mut canonical = self.request.clone();
        canonical.request_id.clear();
        canonical.defaults.watch = canonical.defaults.watch.canonical();
        canonical.defaults.like = canonical.defaults.like.canonical();
        canonical.defaults.follow = canonical.defaults.follow.canonical();
        canonical.defaults.comment = canonical.defaults.comment.canonical();
        canonical.defaults.save = canonical.defaults.save.canonical();
        canonical.defaults.repost = canonical.defaults.repost.canonical();
        canonical.defaults.direct_message = canonical.defaults.direct_message.canonical();
        canonical.targets.clear();
        let canonical_targets: Vec<_> = self.targets.iter()
            .map(|target| (&target.normalized_url, &target.overrides))
            .collect();
        let bytes = serde_json::to_vec(&(canonical, canonical_targets))?;
        Ok(Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PlannedAction {
    pub ordinal: u32,
    pub kind: PlannedActionKind,
    pub decision: PlannedDecision,
    pub delay_before_ms: u32,
    pub watch_duration_ms: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CampaignSummary {
    pub id: CampaignId,
    pub status: CampaignStatus,
    pub result_code: Option<CampaignResultCode>,
    pub revision: i64,
    pub target_count: u32,
    pub actor_count: u32,
    pub assignment_count: u32,
    pub terminal_assignment_count: u32,
    pub positive_assignment_count: u32,
    pub negative_assignment_count: u32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Page<T> { pub items: Vec<T>, pub next_cursor: Option<String> }
```

Add `pub mod interaction;` and the needed `pub use` entries to `crates/core/src/lib.rs`. Add `sha2 = { workspace = true }` to `crates/core/Cargo.toml`.

- [ ] **Step 4: Add complete request validation and fixtures**

Add `InteractionCampaignRequest::validate()` enforcing request ID non-empty, 1-500 nonblank target URLs, watch bounds `1..=300` with min <= max, pacing bounds `0..=60_000 ms` with min <= max, all probability policies in `0..=100`, and Comment credential-or-fallback prerequisite represented by a `comment_runtime_available: bool` validation argument. Normalize exact ASCII `@handle` allowlist entries, deduplicate and sort them canonically by handle, and require at least one only when effective Direct Message mode is Allowlist and that action can run. Content-ID deduplication occurs only after the backend resolver produces `ResolvedCampaignInput`; callers cannot supply content IDs, post kinds, authors, normalized URLs, or resolved URLs. Add `fixture_request` only inside the test module; production code must not contain fixture defaults.
Two duplicate handles with different display labels are a typed conflict rather than a first/last-wins merge; display labels never participate in locator matching.
Implement product defaults exactly as Watch Required, 4-12 seconds, 600-1,800 ms between planned actions, and 1,500-4,000 ms between targets; every other action defaults Off and the dormant recipient policy is an empty Allowlist.

- [ ] **Step 5: Run focused tests and verify GREEN**

Run:

```powershell
cargo test -p riviu-core interaction::types::tests -- --nocapture
```

Expected: all interaction type tests pass; existing core tests remain compiled.

- [ ] **Step 6: Commit the domain contract**

```powershell
git add crates/core/Cargo.toml crates/core/src/lib.rs crates/core/src/interaction/mod.rs crates/core/src/interaction/types.rs
git diff --cached --name-only
git commit -m "feat(core): define interaction campaign domain"
```

Expected staged paths: exactly the four paths above.

---

### Task 1A: Parse And Resolve TikTok Targets On The Backend

**Files:**
- Create: `crates/core/src/interaction/links.rs`
- Create: `crates/core/tests/interaction_links.rs`
- Modify: `crates/core/src/interaction/mod.rs`
- Modify: `crates/core/src/interaction/types.rs`

- [ ] **Step 1: Write the failing direct/short-link matrix**

Use a scripted redirect transport rather than the network. Cover direct video/photo,
`vt.tiktok.com`, `vm.tiktok.com`, `/t/<code>`, blank lines, tracking parameters,
duplicates, malformed URLs, HTTP, userinfo, custom ports, lookalike hosts, profile,
LIVE, music, shop, search, missing Location, unsupported redirect status, off-domain
redirect, loop, hop six, and total deadline.

```rust
#[tokio::test]
async fn resolves_short_links_and_deduplicates_by_content_id() {
    let transport = ScriptedRedirects::new([
        ("https://vt.tiktok.com/abc/", 302,
         Some("https://www.tiktok.com/@creator/video/7657447099239271697?_t=tracking")),
    ]);
    let resolver = TikTokLinkResolver::new(transport);
    let result = resolver.parse_lines(
        "https://vt.tiktok.com/abc/\n\
         https://www.tiktok.com/@creator/video/7657447099239271697"
    ).await;

    assert_eq!(result.lines.len(), 2);
    assert_eq!(result.valid_targets.len(), 1);
    assert_eq!(result.valid_targets[0].target_key,
               "content:7657447099239271697");
    assert_eq!(result.valid_targets[0].kind, TikTokPostKind::Video);
}

#[tokio::test]
async fn validates_every_redirect_hop_before_following_it() {
    let transport = ScriptedRedirects::new([
        ("https://vt.tiktok.com/abc/", 302,
         Some("https://TARGET.example/video/7657447099239271697")),
    ]);
    let error = TikTokLinkResolver::new(transport)
        .resolve_one("https://vt.tiktok.com/abc/")
        .await
        .unwrap_err();
    assert_eq!(error.code, TargetParseErrorCode::UnsupportedHost);
}
```

- [ ] **Step 2: Run the test and verify RED**

```powershell
cargo test -p riviu-core --test interaction_links -- --nocapture
```

Expected: unresolved `interaction::links` imports.

- [ ] **Step 3: Define the per-line DTO and transport boundary**

Add the exact public result contract to `types.rs` and the HTTP seam to `links.rs`:

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum TargetParseErrorCode {
    InvalidUrl,
    UnsupportedScheme,
    UnsupportedHost,
    UserinfoNotAllowed,
    CustomPortNotAllowed,
    UnsupportedTargetKind,
    RedirectMissingLocation,
    RedirectStatusRejected,
    RedirectLoop,
    RedirectLimit,
    ResolutionTimeout,
    ResolutionFailed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum TargetLineOutcome {
    Valid { target: ResolvedTikTokTarget, duplicate: bool },
    Error { code: TargetParseErrorCode, message: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ParsedTargetLine {
    pub line_number: usize,
    pub original: String,
    pub outcome: TargetLineOutcome,
}

pub struct ParseLinksResult {
    pub lines: Vec<ParsedTargetLine>,
    pub valid_targets: Vec<ResolvedTikTokTarget>,
}

#[async_trait]
pub trait RedirectTransport: Send + Sync {
    async fn request_without_redirect(
        &self,
        url: &reqwest::Url,
        remaining: Duration,
    ) -> Result<RedirectResponse, TargetParseError>;
}
```

The production transport uses one `reqwest::Client` with automatic redirects disabled.
It drops the response without retaining the body.
`TargetParseError::Display` and tracing fields expose only the typed code/hop number;
they never print the submitted URL, `Location`, query, or userinfo. The original line
exists only in the command result for the local operator and is never copied to logs.
Add `pub mod links;` and the public parser/result re-exports to `interaction/mod.rs`
in this task, after `links.rs` exists.

- [ ] **Step 4: Implement structural parsing and bounded redirects**

The resolver algorithm is fixed:

```rust
const MAX_REDIRECTS: usize = 5;
const RESOLUTION_DEADLINE: Duration = Duration::from_secs(10);
const REDIRECT_STATUSES: &[u16] = &[301, 302, 303, 307, 308];

fn validate_tiktok_url(url: &reqwest::Url) -> Result<(), TargetParseError> {
    if url.scheme() != "https" { return Err(TargetParseError::scheme()); }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(TargetParseError::userinfo());
    }
    if url.port().is_some() { return Err(TargetParseError::port()); }
    let host = url.host_str().ok_or_else(TargetParseError::host)?;
    if host != "tiktok.com" && !host.ends_with(".tiktok.com") {
        return Err(TargetParseError::host());
    }
    Ok(())
}
```

At each hop, validate the current URL before sending and the joined `Location` URL
before following. Track canonical hop URLs in a set, reject hop six, calculate every
request timeout from one monotonic 10-second deadline, and accept only the five
status codes above. A final direct path must have exactly
`/@<nonempty>/video/<ASCII digits>` or `/@<nonempty>/photo/<ASCII digits>` segments.
Strip the fragment and the fixed tracking-key allowlist; preserve target order and
deduplicate output by `content:<content-id>`.

- [ ] **Step 5: Make request resolution an internal trust boundary**

Add this service method and use it from preview, start, and schedule creation:

```rust
impl<T: RedirectTransport> TikTokLinkResolver<T> {
    pub async fn resolve_request(
        &self,
        request: InteractionCampaignRequest,
    ) -> Result<ResolvedCampaignInput, RequestResolutionError>;
}
```

`resolve_request` revalidates every `TikTokTargetInput.url`, carries its overrides to
the resolved target, rejects a start/schedule request if any submitted target no longer
resolves, rejects zero valid or more than 500 deduplicated targets, and computes the
request hash only from the resulting canonical URLs plus ordered settings/actors. Two
inputs resolving to the same content ID deduplicate only when their overrides are
identical; conflicting overrides return typed `DuplicateTargetConflict` instead of
silently choosing one.
Scheduled dispatch resolves each stored original URL again and requires the same
content ID and post kind before device acquisition.

- [ ] **Step 6: Run GREEN and commit**

```powershell
cargo test -p riviu-core --test interaction_links -- --nocapture
cargo test -p riviu-core interaction::types::tests -- --nocapture
git add crates/core/src/interaction/links.rs crates/core/src/interaction/mod.rs crates/core/src/interaction/types.rs crates/core/tests/interaction_links.rs
git diff --cached --name-only
git commit -m "feat(core): resolve TikTok campaign targets"
```

---

### Task 2: Implement Deterministic Distribution And One-Time Sampling

**Files:**
- Create: `crates/core/src/interaction/planner.rs`
- Modify: `crates/core/src/interaction/mod.rs`
- Modify: `crates/core/src/interaction/types.rs`

- [ ] **Step 1: Write failing All and RoundRobin planner tests**

Create `planner.rs` with only its test module and add `pub mod planner;` to `interaction/mod.rs` before the RED run, so Cargo cannot silently skip the new file.

```rust
#[test]
fn all_is_actor_then_target_cartesian_order() {
    let plan = expect_ready(planner().plan(input(DistributionMode::All, 7)).unwrap());
    let pairs: Vec<_> = plan.assignments.iter()
        .map(|a| (a.account_id.as_str(), a.target_key.as_str()))
        .collect();
    assert_eq!(pairs, vec![
        ("actor-a", "content:1"), ("actor-a", "content:2"),
        ("actor-b", "content:1"), ("actor-b", "content:2"),
    ]);
}

#[test]
fn round_robin_preserves_target_order() {
    let plan = expect_ready(planner().plan(input(DistributionMode::RoundRobin, 7)).unwrap());
    let pairs: Vec<_> = plan.assignments.iter()
        .map(|a| (a.account_id.as_str(), a.target_key.as_str()))
        .collect();
    assert_eq!(pairs, vec![("actor-a", "content:1"), ("actor-b", "content:2")]);
}
```

- [ ] **Step 2: Run planner tests and verify RED**

Run:

```powershell
cargo test -p riviu-core interaction::planner::tests -- --nocapture
```

Expected: unresolved `InteractionPlanner`, `PlanInput`, and `PlannedAssignmentDraft`.

- [ ] **Step 3: Implement the pure expansion contract**

Create the following core in `crates/core/src/interaction/planner.rs`:

```rust
use rand::{rngs::StdRng, Rng, SeedableRng};
use super::types::*;

pub const MAX_ASSIGNMENTS: usize = 10_000;

#[derive(Debug, Clone)]
pub struct ActorSnapshot {
    pub account: AccountBinding,
    pub effective_proxy: EffectiveProxySnapshot,
}

#[derive(Debug, Clone)]
pub struct PlanInput {
    pub campaign_id: CampaignId,
    pub actors: Vec<ActorSnapshot>,
    pub targets: Vec<ResolvedTikTokTarget>,
    pub defaults: InteractionDefaults,
    pub distribution: DistributionMode,
    pub seed: u64,
}

#[derive(Debug, Clone)]
pub struct PlannedAssignmentDraft {
    pub ordinal: u32,
    pub account_id: AccountId,
    pub udid_snapshot: String,
    pub effective_proxy: EffectiveProxySnapshot,
    pub target_key: String,
    pub effective_settings: InteractionDefaults,
    pub assignment_seed: u64,
    pub target_delay_after_ms: u32,
    pub actions: Vec<PlannedAction>,
}

#[derive(Debug, Clone)]
pub struct PlannedCampaignDraft {
    pub campaign_id: CampaignId,
    pub actors: Vec<ActorSnapshot>,
    pub assignments: Vec<PlannedAssignmentDraft>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanRejection {
    NoEligibleActors,
    TooManyAssignments { projected: usize, maximum: usize },
}

#[derive(Debug, Clone)]
pub enum PlanOutcome {
    Ready(PlannedCampaignDraft),
    Rejected {
        campaign_id: CampaignId,
        actors: Vec<ActorSnapshot>,
        reason: PlanRejection,
    },
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum PlanError {
    #[error("invalid planner input: {code}")]
    InvalidInput { code: &'static str },
}

#[derive(Default)]
pub struct InteractionPlanner;

impl InteractionPlanner {
    pub fn plan(&self, input: PlanInput) -> Result<PlanOutcome, PlanError> {
        if input.actors.is_empty() {
            return Ok(PlanOutcome::Rejected {
                campaign_id: input.campaign_id,
                actors: input.actors,
                reason: PlanRejection::NoEligibleActors,
            });
        }
        let pairs: Vec<(usize, usize)> = match input.distribution {
            DistributionMode::All => (0..input.actors.len())
                .flat_map(|actor| (0..input.targets.len()).map(move |target| (actor, target)))
                .collect(),
            DistributionMode::RoundRobin => (0..input.targets.len())
                .map(|target| (target % input.actors.len(), target))
                .collect(),
        };
        if pairs.len() > MAX_ASSIGNMENTS {
            return Ok(PlanOutcome::Rejected {
                campaign_id: input.campaign_id,
                actors: input.actors,
                reason: PlanRejection::TooManyAssignments {
                    projected: pairs.len(),
                    maximum: MAX_ASSIGNMENTS,
                },
            });
        }

        let mut root_rng = StdRng::seed_from_u64(input.seed);
        let assignments = pairs.into_iter().enumerate().map(|(ordinal, (ai, ti))| {
            let actor = &input.actors[ai];
            let target = &input.targets[ti];
            let assignment_seed = root_rng.gen::<u64>();
            let effective = merge_settings(&input.defaults, &target.overrides);
            let actions = sample_actions(&effective, assignment_seed);
            let target_delay_after_ms = seeded_range_ms(
                assignment_seed,
                b"target-delay-after-v1",
                effective.target_delay_min_ms,
                effective.target_delay_max_ms,
            );
            PlannedAssignmentDraft {
                ordinal: ordinal as u32,
                account_id: actor.account.id.clone(),
                udid_snapshot: actor.account.device_udid.clone(),
                effective_proxy: actor.effective_proxy.clone(),
                target_key: target.target_key.clone(),
                actions,
                effective_settings: effective,
                assignment_seed,
                target_delay_after_ms,
            }
        }).collect();
        Ok(PlanOutcome::Ready(PlannedCampaignDraft {
            campaign_id: input.campaign_id,
            actors: input.actors,
            assignments,
        }))
    }
}
```

- [ ] **Step 4: Write failing merge and stable-sampling tests**

```rust
#[test]
fn override_inherit_and_probability_are_stable() {
    let first = expect_ready(planner().plan(input(DistributionMode::All, 42)).unwrap());
    let second = expect_ready(planner().plan(input(DistributionMode::All, 42)).unwrap());
    assert_eq!(first.assignments[0].actions, second.assignments[0].actions);
    assert_eq!(first.assignments[0].effective_settings.like, ActionPolicy::Required);
    assert!(first.assignments[0].actions.iter().any(|a|
        a.kind == PlannedActionKind::TargetIdentityCopyLink && a.decision == PlannedDecision::Pending));
    assert_eq!(first.assignments[0].target_delay_after_ms,
               second.assignments[0].target_delay_after_ms);
    assert!(first.assignments[0].actions.iter().all(|a|
        a.delay_before_ms <= first.assignments[0].effective_settings.action_delay_max_ms));
    let watch = first.assignments[0].actions.iter()
        .find(|a| a.kind == PlannedActionKind::Watch)
        .unwrap();
    assert!(watch.watch_duration_ms.is_some_and(|ms|
        ms >= first.assignments[0].effective_settings.watch_min_seconds * 1_000
            && ms <= first.assignments[0].effective_settings.watch_max_seconds * 1_000));
}

#[test]
fn false_probability_is_not_planned_not_skipped() {
    let actions = sample_actions(&defaults_with_like(ActionPolicy::Off), 1);
    let like = actions.iter().find(|a| a.kind == PlannedActionKind::Like).unwrap();
    assert_eq!(like.decision, PlannedDecision::NotPlanned);
}
```

- [ ] **Step 5: Implement complete merge and sampling helpers**

Implement `merge_settings` by replacing only `Some` override fields across watch bounds, policies, comment instruction, recipient policy, action pacing, and target pacing, then validate the complete effective settings and canonicalize every policy. Implement `sample_actions` with domain-separated deterministic draws from `assignment_seed`, fixed action ordinal order `TargetIdentityCopyLink, Watch, Like, Follow, Comment, Save, Repost, DirectMessage`, unconditional Pending for identity, and exactly one probability draw per optional action. `Off` and false rolls become `NotPlanned`; they never become `Skipped`.

Persist pacing and Watch duration instead of sampling either at execution time. `TargetIdentityCopyLink` and every `NotPlanned` action have `delay_before_ms=0`; each later `Pending` action receives one value in the effective `action_delay_*` range, and each assignment receives `target_delay_after_ms` from that target's merged effective range. A selected Watch receives `watch_duration_ms=Some(value)` sampled inclusively from the effective 1-300 second bounds; every other action and an unselected Watch use `None`. Use separate SHA-256 namespaces for policy, Watch duration, action delay, target delay, and later recipient selection so adding a draw in one domain cannot change another domain. Tests cover inclusive equal bounds, per-target override application, min/max validation, same-seed equality, changed-seed variation, and exact persistence across retry/restart.

G1 persists normalized `RecipientPolicy` plus the assignment seed but leaves DirectMessage capability `Deferred/GateNotQualified`; it does not invent a second recipient-intent type. The G4 plan extends `PreparedActionPayload`, uses the domain-separated seed to select the recipient exactly once during the first durable preparation, and preserves the committed choice across re-entry, retries, and restart.

Add a capacity test asserting `PlanOutcome::Rejected` preserves the eligible actor snapshot and projected count without allocating assignment/action vectors. The dispatcher retains the full availability snapshot, including excluded actors, and supplies both groups to `PlanCommit`; this lets the store persist campaign actors and terminal `Failed/TooManyAssignments` atomically while creating no assignment or device work.

- [ ] **Step 6: Run planner tests and verify GREEN**

```powershell
cargo test -p riviu-core interaction::planner::tests -- --nocapture
```

Expected: Cartesian order, round robin, cap, merge, mandatory identity, and repeated-seed tests all pass.

- [ ] **Step 7: Commit the planner**

```powershell
git add crates/core/src/interaction/mod.rs crates/core/src/interaction/planner.rs crates/core/src/interaction/types.rs
git diff --cached --name-only
git commit -m "feat(core): plan deterministic interaction assignments"
```

---

### Task 3: Make State Aggregation Exhaustive And Pure

**Files:**
- Create: `crates/core/src/interaction/aggregate.rs`
- Modify: `crates/core/src/interaction/mod.rs`
- Modify: `crates/core/src/interaction/types.rs`

- [ ] **Step 1: Write the failing aggregation truth table**

Create `aggregate.rs` with only its test module and add `pub mod aggregate;` to `interaction/mod.rs` before the RED run.

```rust
#[test]
fn campaign_aggregation_covers_every_terminal_mix() {
    let cases = [
        (vec![positive()], vec![], false, CampaignStatus::Succeeded, None),
        (vec![positive(), failed()], vec![], false, CampaignStatus::Partial, None),
        (vec![positive()], vec![], true, CampaignStatus::Partial, None),
        (vec![failed()], vec![], false, CampaignStatus::Failed, Some(CampaignResultCode::NoRunnableAssignments)),
        (vec![interrupted()], vec![], false, CampaignStatus::Interrupted, Some(CampaignResultCode::ProcessLost)),
        (vec![uncertain()], vec![], false, CampaignStatus::Interrupted, Some(CampaignResultCode::AmbiguousOutcome)),
        (vec![uncertain()], vec![], true, CampaignStatus::Interrupted, Some(CampaignResultCode::AmbiguousOutcome)),
        (vec![positive(), uncertain()], vec![], false, CampaignStatus::Partial, None),
        (vec![skipped_unsupported()], vec![], false, CampaignStatus::Failed, Some(CampaignResultCode::NoSupportedAssignments)),
        (vec![positive()], vec![ActorRunStatus::SkippedUnavailable], false, CampaignStatus::Partial, None),
        (vec![], vec![ActorRunStatus::SkippedUnavailable], false, CampaignStatus::Failed, Some(CampaignResultCode::NoEligibleActors)),
        (vec![], vec![], true, CampaignStatus::Cancelled, Some(CampaignResultCode::Cancelled)),
    ];
    for (assignments, actors, cancelled, status, code) in cases {
        let actual = aggregate_campaign(&CampaignAggregateInput { assignments, actors, cancelled, missed: false });
        assert_eq!((actual.status, actual.result_code), (status, code));
    }
}

#[test]
fn not_planned_actions_do_not_make_an_assignment_partial() {
    let result = aggregate_assignment(true, &[
        action(ActionStatus::Succeeded),
        action(ActionStatus::NotPlanned),
    ]).unwrap();
    assert_eq!(result.status, AssignmentStatus::Succeeded);
    assert!(result.positive);
}

#[test]
fn identity_only_is_positive_only_when_no_optional_action_was_planned() {
    assert!(aggregate_assignment(true, &[identity_success()]).unwrap().positive);
    assert!(!aggregate_assignment(true, &[identity_success(), action(ActionStatus::Failed)]).unwrap().positive);
}

#[test]
fn latest_attempt_replaces_prior_attempt_for_the_same_action_ordinal() {
    let result = aggregate_assignment(true, &[
        attempt(2, 1, ActionStatus::Failed),
        attempt(2, 2, ActionStatus::Succeeded),
    ]).unwrap();
    assert_eq!(result.status, AssignmentStatus::Succeeded);
}
```

- [ ] **Step 2: Run aggregation tests and verify RED**

```powershell
cargo test -p riviu-core interaction::aggregate::tests -- --nocapture
```

Expected: unresolved aggregation inputs/functions.

- [ ] **Step 3: Implement assignment and campaign reducers**

Define `AssignmentAggregateInput`, `AssignmentAggregate`, `CampaignAggregateInput`, and `CampaignAggregate`. Implement these precedence rules exactly:

```rust
pub fn aggregate_campaign(input: &CampaignAggregateInput) -> CampaignAggregate {
    if input.missed {
        return CampaignAggregate::new(CampaignStatus::Missed, None);
    }
    if input.assignments.iter().any(|a| !a.terminal)
        || input.actors.iter().any(|a| matches!(a,
            ActorRunStatus::Selected | ActorRunStatus::Eligible | ActorRunStatus::Running))
    {
        return CampaignAggregate::new(CampaignStatus::Running, None);
    }
    let positive = input.assignments.iter().filter(|a| a.positive).count();
    let actor_unavailable = input.actors.iter().any(|s|
        *s == ActorRunStatus::SkippedUnavailable);
    let actor_unsupported = input.actors.iter().any(|s|
        *s == ActorRunStatus::SkippedUnsupported);
    let actor_skip = actor_unavailable || actor_unsupported;
    let actor_uncertain = input.actors.iter().any(|s| *s == ActorRunStatus::Uncertain);
    let actor_interrupted = input.actors.iter().any(|s| *s == ActorRunStatus::Interrupted);
    let actor_negative = input.actors.iter().any(|s| matches!(s,
        ActorRunStatus::Partial | ActorRunStatus::Failed | ActorRunStatus::Interrupted |
        ActorRunStatus::Uncertain | ActorRunStatus::SkippedUnavailable |
        ActorRunStatus::SkippedUnsupported));
    let interrupted = input.assignments.iter().any(|a| matches!(a.status,
        AssignmentStatus::Interrupted | AssignmentStatus::Uncertain)) || actor_interrupted || actor_uncertain;
    let uncertain = input.assignments.iter().any(|a| a.status == AssignmentStatus::Uncertain)
        || actor_uncertain;
    let unsupported_only = !input.assignments.is_empty() && input.assignments.iter().all(|a|
        a.status == AssignmentStatus::SkippedUnsupported);
    let negative = actor_negative || input.assignments.iter().any(|a| !a.positive);

    if uncertain && positive == 0 {
        CampaignAggregate::new(CampaignStatus::Interrupted, Some(CampaignResultCode::AmbiguousOutcome))
    } else if input.cancelled {
        if positive == 0 {
            CampaignAggregate::new(CampaignStatus::Cancelled, Some(CampaignResultCode::Cancelled))
        } else {
            CampaignAggregate::new(CampaignStatus::Partial, None)
        }
    } else if positive > 0 && !negative {
        CampaignAggregate::new(CampaignStatus::Succeeded, None)
    } else if positive > 0 {
        CampaignAggregate::new(CampaignStatus::Partial, None)
    } else if interrupted {
        CampaignAggregate::new(CampaignStatus::Interrupted, Some(CampaignResultCode::ProcessLost))
    } else if unsupported_only {
        CampaignAggregate::new(CampaignStatus::Failed, Some(CampaignResultCode::NoSupportedAssignments))
    } else if input.assignments.is_empty() && actor_unsupported && !actor_unavailable {
        CampaignAggregate::new(CampaignStatus::Failed, Some(CampaignResultCode::NoSupportedAssignments))
    } else if input.assignments.is_empty() && actor_skip {
        CampaignAggregate::new(CampaignStatus::Failed, Some(CampaignResultCode::NoEligibleActors))
    } else {
        CampaignAggregate::new(CampaignStatus::Failed, Some(CampaignResultCode::NoRunnableAssignments))
    }
}
```

`aggregate_assignment` groups action runs by immutable `action_ordinal` and evaluates only the highest `attempt_no`; earlier attempts remain audit history and do not make a later success `Partial`. It must ignore `NotPlanned`, treat latest `Pending`/`Running` as nonterminal, require the latest identity attempt to succeed, count `Succeeded`/`AlreadySatisfied` as positive, return `Partial` for mixed optional outcomes, `Uncertain` before `Interrupted`, and never infer success from an HTTP/gesture acknowledgement field. The identity mapping is exact: `Failed/TargetUnverified` produces assignment `Failed/TargetUnverified`, while `Uncertain/TargetIdentityAmbiguous` produces assignment `Uncertain/TargetIdentityAmbiguous`. A retry-created Pending identity attempt therefore makes the assignment nonterminal even when an earlier identity attempt was Confirmed. Reject a persisted invariant with two non-terminal attempts for one ordinal instead of choosing one silently.

- [ ] **Step 4: Add an enum-coverage test instead of a wildcard arm**

Write one test constructing every `ActionStatus`, `AssignmentStatus`, and `ActorRunStatus` variant. Keep reducer matches exhaustive so adding a state causes a compile error or a failing truth-table case.

- [ ] **Step 5: Run aggregation and full core tests**

```powershell
cargo test -p riviu-core interaction::aggregate::tests -- --nocapture
cargo test -p riviu-core
```

Expected: all new truth-table cases and all existing core tests pass.

- [ ] **Step 6: Commit the reducers**

```powershell
git add crates/core/src/interaction/aggregate.rs crates/core/src/interaction/mod.rs crates/core/src/interaction/types.rs
git diff --cached --name-only
git commit -m "feat(core): aggregate interaction outcomes deterministically"
```

---

### Task 4: Add Configured SQLite Connections And Numbered Schema

**Files:**
- Create: `crates/core/src/interaction/schema.rs`
- Create: `crates/core/tests/interaction_schema.rs`
- Modify: `crates/core/src/db.rs`
- Modify: `crates/core/src/interaction/mod.rs`

- [ ] **Step 1: Write failing migration and PRAGMA tests**

```rust
#[test]
fn existing_database_gets_interaction_schema_and_durable_pragmas() {
    let path = fixture_path("schema-upgrade");
    let old = rusqlite::Connection::open(&path).unwrap();
    old.execute("CREATE TABLE legacy_fixture (id TEXT PRIMARY KEY)", []).unwrap();
    drop(old);

    let db = Database::open(&path).unwrap();
    let conn = db.configured_connection().unwrap();
    let mode: String = conn.query_row("PRAGMA journal_mode", [], |r| r.get(0)).unwrap();
    let foreign_keys: i64 = conn.query_row("PRAGMA foreign_keys", [], |r| r.get(0)).unwrap();
    let synchronous: i64 = conn.query_row("PRAGMA synchronous", [], |r| r.get(0)).unwrap();
    let busy_timeout: i64 = conn.query_row("PRAGMA busy_timeout", [], |r| r.get(0)).unwrap();

    assert_eq!(mode.to_ascii_lowercase(), "wal");
    assert_eq!(foreign_keys, 1);
    assert_eq!(synchronous, 2); // FULL
    assert_eq!(busy_timeout, 5_000);
    assert!(table_exists(&conn, "interaction_dispatch"));
    assert!(table_exists(&conn, "legacy_fixture"));
}

#[test]
fn migration_is_idempotent() {
    let path = fixture_path("schema-idempotent");
    Database::open(&path).unwrap();
    Database::open(&path).unwrap();
    let conn = rusqlite::Connection::open(path).unwrap();
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM schema_migrations WHERE version = 2026072901", [], |r| r.get(0)
    ).unwrap();
    assert_eq!(count, 1);
}
```

- [ ] **Step 2: Run the schema test and verify RED**

```powershell
cargo test -p riviu-core --test interaction_schema -- --nocapture
```

Expected: compilation fails because `configured_connection` and interaction tables do not exist.

- [ ] **Step 3: Configure every database connection**

Change `Database::open`/`conn` in `crates/core/src/db.rs` so WAL is enabled once before migration and every returned connection applies:

```rust
pub fn configured_connection(&self) -> anyhow::Result<Connection> {
    let conn = Connection::open(&self.path)?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    conn.pragma_update(None, "synchronous", "FULL")?;
    conn.busy_timeout(std::time::Duration::from_secs(5))?;
    Ok(conn)
}
```

At initialization, call `PRAGMA journal_mode=WAL` outside a transaction, then use `configured_connection()` for all existing methods. Do not change existing table semantics.

- [ ] **Step 4: Add the complete numbered interaction migration**

Create `schema.rs` with version `2026072901`. The SQL must create `schema_migrations` plus the ten campaign tables from the design and the append-only `interaction_open_attempts` table required by the reviewed retry contract, with foreign keys, required unique constraints, and indexes for campaign status/revision, dispatch state/availability, actor UDID/state, assignment campaign/device/state/ordinal, target campaign/ordinal, action attempt order, opening attempt order, schedule due/state, and artifact retention/purge state. Use `TransactionBehavior::Immediate`, insert the migration version only after every DDL statement succeeds, and roll back the complete migration on error.

In the same additive migration, add `proxies.configuration_revision TEXT` only when the column is absent and backfill existing blank rows with generated UUID values. Update `Database::upsert_proxy` to write a new revision on every create/edit without exposing it in the current credential-bearing UI DTO. This supplies the non-secret revision required by immutable proxy snapshots; do not derive a stored revision by hashing a proxy password.

Required columns include:

```sql
interaction_campaigns(
  id TEXT PRIMARY KEY, request_id TEXT NOT NULL UNIQUE, request_hash TEXT NOT NULL,
  request_json TEXT NOT NULL, status TEXT NOT NULL, result_code TEXT,
  target_count INTEGER NOT NULL DEFAULT 0, actor_count INTEGER NOT NULL DEFAULT 0,
  assignment_count INTEGER NOT NULL DEFAULT 0,
  terminal_assignment_count INTEGER NOT NULL DEFAULT 0,
  positive_assignment_count INTEGER NOT NULL DEFAULT 0,
  negative_assignment_count INTEGER NOT NULL DEFAULT 0,
  revision INTEGER NOT NULL DEFAULT 0, cancel_requested_at TEXT,
  planner_seed TEXT, planned_at TEXT, created_at TEXT NOT NULL, updated_at TEXT NOT NULL
);
interaction_campaign_actors(
  id TEXT PRIMARY KEY, campaign_id TEXT NOT NULL, account_id TEXT NOT NULL,
  actor_ordinal INTEGER NOT NULL, account_snapshot_json TEXT NOT NULL,
  udid_snapshot TEXT NOT NULL, status TEXT NOT NULL, effective_proxy_id TEXT,
  effective_proxy_snapshot_json TEXT NOT NULL,
  capability_snapshot_json TEXT, error_code TEXT, revision INTEGER NOT NULL DEFAULT 0,
  created_at TEXT NOT NULL, updated_at TEXT NOT NULL,
  UNIQUE(campaign_id, account_id),
  FOREIGN KEY(campaign_id) REFERENCES interaction_campaigns(id) ON DELETE CASCADE
);
interaction_targets(
  id TEXT PRIMARY KEY, campaign_id TEXT NOT NULL, target_ordinal INTEGER NOT NULL,
  original_url TEXT NOT NULL, normalized_url TEXT NOT NULL, resolved_url TEXT NOT NULL,
  target_key TEXT NOT NULL, content_id TEXT NOT NULL, author TEXT, kind TEXT NOT NULL,
  overrides_json TEXT NOT NULL, resolution_status TEXT NOT NULL, resolution_error TEXT,
  execution_resolved_url TEXT, execution_content_id TEXT, execution_kind TEXT,
  execution_resolution_status TEXT, execution_error_code TEXT, execution_checked_at TEXT,
  UNIQUE(campaign_id, target_key),
  FOREIGN KEY(campaign_id) REFERENCES interaction_campaigns(id) ON DELETE CASCADE
);
interaction_assignments(
  id TEXT PRIMARY KEY, campaign_id TEXT NOT NULL, account_id TEXT NOT NULL,
  target_id TEXT NOT NULL, parent_assignment_id TEXT, assignment_ordinal INTEGER NOT NULL,
  udid_snapshot TEXT NOT NULL, effective_proxy_id TEXT,
  effective_proxy_snapshot_json TEXT NOT NULL, effective_settings_json TEXT NOT NULL,
  sampled_actions_json TEXT NOT NULL, assignment_seed TEXT NOT NULL,
  target_delay_after_ms INTEGER NOT NULL CHECK(
    target_delay_after_ms >= 0 AND target_delay_after_ms <= 60000),
  identity_state TEXT NOT NULL,
  current_identity_attempt_no INTEGER NOT NULL CHECK(current_identity_attempt_no >= 1),
  identity_copy_intent TEXT NOT NULL,
  status TEXT NOT NULL, result_code TEXT, revision INTEGER NOT NULL DEFAULT 0,
  created_at TEXT NOT NULL, updated_at TEXT NOT NULL,
  UNIQUE(campaign_id, account_id, target_id),
  FOREIGN KEY(campaign_id) REFERENCES interaction_campaigns(id) ON DELETE CASCADE,
  FOREIGN KEY(target_id) REFERENCES interaction_targets(id) ON DELETE CASCADE,
  FOREIGN KEY(parent_assignment_id) REFERENCES interaction_assignments(id) ON DELETE RESTRICT
);
interaction_action_runs(
  id TEXT PRIMARY KEY, assignment_id TEXT NOT NULL, action_ordinal INTEGER NOT NULL,
  action_kind TEXT NOT NULL, attempt_no INTEGER NOT NULL CHECK(attempt_no >= 1),
  decision TEXT NOT NULL,
  delay_before_ms INTEGER NOT NULL CHECK(
    delay_before_ms >= 0 AND delay_before_ms <= 60000),
  watch_duration_ms INTEGER CHECK(watch_duration_ms IS NULL OR
    (watch_duration_ms >= 1000 AND watch_duration_ms <= 300000)),
  status TEXT NOT NULL, identity_copy_intent TEXT NOT NULL,
  effect_intent TEXT NOT NULL, prepared_payload_json TEXT,
  outcome_code TEXT, evidence_json TEXT, revision INTEGER NOT NULL DEFAULT 0,
  started_at TEXT, completed_at TEXT, updated_at TEXT NOT NULL,
  UNIQUE(assignment_id, action_ordinal, attempt_no),
  FOREIGN KEY(assignment_id) REFERENCES interaction_assignments(id) ON DELETE CASCADE
);
interaction_open_attempts(
  id TEXT PRIMARY KEY, assignment_id TEXT NOT NULL, identity_action_run_id TEXT NOT NULL,
  attempt_no INTEGER NOT NULL CHECK(attempt_no >= 1), status TEXT NOT NULL,
  retry_reason TEXT,
  outcome_code TEXT, revision INTEGER NOT NULL DEFAULT 0,
  started_at TEXT NOT NULL, completed_at TEXT,
  UNIQUE(identity_action_run_id, attempt_no),
  FOREIGN KEY(assignment_id) REFERENCES interaction_assignments(id) ON DELETE CASCADE,
  FOREIGN KEY(identity_action_run_id) REFERENCES interaction_action_runs(id) ON DELETE CASCADE
);
interaction_dispatch(
  campaign_id TEXT PRIMARY KEY, state TEXT NOT NULL, available_at TEXT NOT NULL,
  claim_owner TEXT, claim_started_at TEXT, revision INTEGER NOT NULL DEFAULT 0,
  FOREIGN KEY(campaign_id) REFERENCES interaction_campaigns(id) ON DELETE CASCADE
);
```

Create the remaining four tables with this exact first-version shape:

```sql
tiktok_accounts(
  id TEXT PRIMARY KEY, platform TEXT NOT NULL, device_udid TEXT NOT NULL,
  slot_key TEXT NOT NULL, username TEXT, display_label TEXT NOT NULL,
  state TEXT NOT NULL, is_default INTEGER NOT NULL DEFAULT 0 CHECK(is_default IN (0,1)),
  created_at TEXT NOT NULL, updated_at TEXT NOT NULL,
  UNIQUE(platform, device_udid, slot_key)
);
interaction_artifacts(
  id TEXT PRIMARY KEY, campaign_id TEXT NOT NULL, assignment_id TEXT,
  action_run_id TEXT, evidence_kind TEXT NOT NULL, relative_path TEXT,
  sha256 TEXT NOT NULL, mime_type TEXT NOT NULL, byte_len INTEGER NOT NULL,
  payload_json TEXT,
  storage_state TEXT NOT NULL, retention_class TEXT NOT NULL,
  pinned INTEGER NOT NULL DEFAULT 0 CHECK(pinned IN (0,1)),
  created_at TEXT NOT NULL, purge_after TEXT NOT NULL, purged_at TEXT,
  last_verified_at TEXT,
  FOREIGN KEY(campaign_id) REFERENCES interaction_campaigns(id) ON DELETE CASCADE,
  FOREIGN KEY(assignment_id) REFERENCES interaction_assignments(id) ON DELETE CASCADE,
  FOREIGN KEY(action_run_id) REFERENCES interaction_action_runs(id) ON DELETE CASCADE
);
interaction_schedules(
  id TEXT PRIMARY KEY, campaign_id TEXT NOT NULL UNIQUE, scheduled_at TEXT NOT NULL,
  state TEXT NOT NULL, claim_owner TEXT, claim_started_at TEXT,
  dispatched_at TEXT, missed_at TEXT, revision INTEGER NOT NULL DEFAULT 0,
  created_at TEXT NOT NULL, updated_at TEXT NOT NULL,
  FOREIGN KEY(campaign_id) REFERENCES interaction_campaigns(id) ON DELETE CASCADE
);
interaction_retry_requests(
  request_id TEXT PRIMARY KEY, request_hash TEXT NOT NULL, campaign_id TEXT NOT NULL,
  assignment_ids_json TEXT NOT NULL, result_json TEXT NOT NULL,
  created_at TEXT NOT NULL,
  FOREIGN KEY(campaign_id) REFERENCES interaction_campaigns(id) ON DELETE CASCADE
);
```

Store all timestamps as RFC 3339 UTC text. Add non-negative `CHECK` constraints for byte/count/ordinal/revision fields, require artifact paths to be relative at the Rust boundary, and create the exact indexes named in the paragraph above. `interaction_assignments.current_identity_attempt_no` plus `identity_copy_intent` are the current materialized projection required by the approved assignment contract; every identity transition updates them in the same transaction as the highest `TargetIdentityCopyLink` row and rejects a mismatch as a persisted invariant error. The append-only action row remains the historical no-replay source. `identity_copy_intent` is `None` on every non-identity action; only a Running current identity row may CAS it to `Issued`. Each opening row must reference an identity action from the same assignment. Add a partial unique index on `(platform, device_udid) WHERE is_default=1`, while the slot-key uniqueness permits future additional accounts. Test every foreign key, uniqueness constraint, Boolean check, current-identity projection invariant, identity/opening ownership constraint, query index, and the one-default-account invariant.

- [ ] **Step 5: Test rollback on a forced migration failure**

Add a test-only migration runner that applies an invalid statement after creating a sentinel table. Assert neither sentinel nor version row remains. This proves the numbered migration is atomic rather than merely repeatable.
Also seed a pre-migration proxy, assert migration backfills a nonblank revision, then call `upsert_proxy` with changed credentials and assert the revision changes while no campaign-facing DTO contains those credentials.

- [ ] **Step 6: Run schema and existing DB tests**

```powershell
cargo test -p riviu-core --test interaction_schema -- --nocapture
cargo test -p riviu-core db:: -- --nocapture
```

Expected: PRAGMAs, schema, constraints, migration rollback, idempotency, and existing database tests pass.

- [ ] **Step 7: Commit the schema**

```powershell
git add crates/core/src/db.rs crates/core/src/interaction/mod.rs crates/core/src/interaction/schema.rs crates/core/tests/interaction_schema.rs
git diff --cached --name-only
git commit -m "feat(core): add durable interaction schema"
```

### Task 5: Build the serialized repository and request idempotency boundary

**Files:**
- Create: `crates/core/src/interaction/store.rs`
- Create: `crates/core/tests/interaction_store.rs`
- Modify: `crates/core/src/interaction/mod.rs`

- [ ] **Step 1: Write failing tests for one-writer behavior and idempotent campaign creation**

Create `interaction_store.rs` tests that open a temporary database and execute two concurrent creates with the same request:

```rust
#[tokio::test]
async fn duplicate_request_id_with_same_hash_returns_existing_campaign() {
    let store = test_store().await;
    let request = valid_resolved_request("req-001");
    let (left, right) = tokio::join!(
        store.create_campaign(request.clone()),
        store.create_campaign(request.clone()),
    );
    let left = left.unwrap();
    let right = right.unwrap();
    assert_eq!(left.summary.id, right.summary.id);
    assert_ne!(left.changed, right.changed);
    assert_eq!(store.count_campaigns().await.unwrap(), 1);
}

#[tokio::test]
async fn duplicate_request_id_with_different_hash_is_a_conflict() {
    let store = test_store().await;
    store.create_campaign(valid_resolved_request("req-001")).await.unwrap();
    let mut changed_request = valid_request("req-001");
    changed_request.defaults.comment = ActionPolicy::Probability { percent: 77 };
    let changed = fixture_resolver().resolve_request(changed_request).await.unwrap();
    assert!(matches!(
        store.create_campaign(changed).await,
        Err(StoreError::IdempotencyConflict { request_id }) if request_id == "req-001"
    ));
}
```

Also race 25 distinct requests and assert all rows exist without `SQLITE_BUSY`. The test must fail because no interaction repository exists.

```powershell
cargo test -p riviu-core --test interaction_store -- --nocapture
```

Expected RED: unresolved `InteractionStore`/`StoreError` imports.

- [ ] **Step 2: Define explicit command and repository errors**

In `store.rs`, add typed errors for validation, not found, idempotency conflict, invalid transition, capacity, serialization, and SQLite failure. Do not expose raw SQL strings or token-bearing values in `Display` output.

```rust
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("invalid interaction request: {code}")]
    Validation { code: &'static str },
    #[error("interaction object not found: {entity}/{id}")]
    NotFound { entity: &'static str, id: String },
    #[error("request id already exists with different content: {request_id}")]
    IdempotencyConflict { request_id: String },
    #[error("invalid {entity} transition from {from} to {to}")]
    InvalidTransition { entity: &'static str, from: String, to: String },
    #[error("interaction capacity exceeded: {code}")]
    Capacity { code: &'static str },
    #[error("interaction payload serialization failed")]
    Serialization(#[from] serde_json::Error),
    #[error("interaction database operation failed")]
    Database(#[from] rusqlite::Error),
    #[error("interaction database access failed")]
    DatabaseAccess(#[source] anyhow::Error),
    #[error("interaction blocking task failed")]
    RuntimeJoin,
}

impl StoreError {
    fn join(_: tokio::task::JoinError) -> Self { Self::RuntimeJoin }
}

#[derive(Debug, Clone)]
pub struct CommittedMutation<T> {
    pub value: T,
    pub summary: CampaignSummary,
    pub changed_assignment_ids: Vec<String>,
    pub changed: bool,
}
```

- [ ] **Step 3: Implement a serialized blocking writer**

Implement `InteractionStore` with `Arc<Database>` and a dedicated `Arc<parking_lot::Mutex<()>>`. Every public async mutation must use `tokio::task::spawn_blocking`, acquire that mutex, open a configured connection, and use an immediate transaction. Reads also run in `spawn_blocking`, but do not take the writer mutex. Never retain `rusqlite::Connection`, `Transaction`, statement, or row across an `.await`.

```rust
#[derive(Clone)]
pub struct InteractionStore {
    db: Arc<Database>,
    writer: Arc<parking_lot::Mutex<()>>,
}

impl InteractionStore {
    async fn write<T, F>(&self, operation: F) -> Result<T, StoreError>
    where
        T: Send + 'static,
        F: FnOnce(&rusqlite::Transaction<'_>) -> Result<T, StoreError> + Send + 'static,
    {
        let db = Arc::clone(&self.db);
        let writer = Arc::clone(&self.writer);
        tokio::task::spawn_blocking(move || {
            let _guard = writer.lock();
            let mut conn = db.configured_connection().map_err(StoreError::DatabaseAccess)?;
            let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
            let value = operation(&tx)?;
            tx.commit()?;
            Ok(value)
        })
        .await
        .map_err(StoreError::join)?
    }
}
```

`parking_lot` is already a `riviu-core` dependency. Use that existing dependency and do not hold its mutex across `.await`.

- [ ] **Step 4: Implement atomic request creation**

`create_campaign(resolved: ResolvedCampaignInput)` accepts only the internal resolver output, validates its cross-field invariants, computes `request_hash` from its canonical targets, and performs all of the following in one transaction. The public Tauri service must call `TikTokLinkResolver::resolve_request` before this method; `InteractionStore` does not expose an overload accepting raw or caller-resolved target metadata.

1. Look up `request_id`; return the existing summary only when hashes match.
2. Insert `interaction_campaigns` with immutable `request_json`, revision `0`, and state `Queued` for `RunNow` or `Scheduled` for `Once`.
3. Insert all resolved targets in request order.
4. For `RunNow`, insert `interaction_dispatch(state='queued', available_at=now)`.
5. For `Once`, validate RFC 3339 UTC, reject a timestamp in the past, and insert `interaction_schedules(state='pending')` without a dispatch row.

Do not acquire devices, leases, stream budget, or call a driver in this method. Add an injected `Clock` trait with a system implementation and a deterministic test clock; database time must not decide observable behavior.
Same-hash replay never inserts/reopens dispatch or schedule rows, even when the existing campaign is terminal; the caller must use the explicit retry command or a new request ID.

- [ ] **Step 5: Add deterministic current-default account persistence**

Add `ensure_default_account(udid)` using ID `device:{udid}:default`, display label derived from the device, `is_default=true`, `state=active`, and no proxy copy. Use `INSERT ... ON CONFLICT(id) DO NOTHING` so later account metadata is never overwritten by device discovery. Assert repeated calls preserve a user-edited label. Add `load_effective_proxy_snapshot(udid)`: read only canonical `device_meta.proxy_id` plus the proxy's non-secret `configuration_revision`, persist only ID/revision/source/non-secret state, and snapshot current devices as `applyCapability=unsupportedUnsupervised` plus `configurationState=manualRequired`, never `appliedVerified`. Editing the proxy row or device assignment must change the effective revision/ID and invalidate endpoint/manual annotations; no host, username, password, or URL userinfo enters campaign JSON/evidence/logs.
The snapshot is audit context only in this phase: it neither blocks Interaction nor claims that traffic used the proxy.

- [ ] **Step 6: Persist versioned interaction defaults through the same writer**

Implement `get_defaults()`/`save_defaults()` using the existing `settings` table key `interaction.defaults.v1`. Validate before write, perform the upsert through `InteractionStore::write`, and return a typed serialization error for malformed stored JSON instead of silently replacing it. Defaults contain no AI credential or proxy secret. Test valid round-trip, malformed JSON, and concurrent save/create operations.

- [ ] **Step 7: Run the repository tests**

```powershell
cargo test -p riviu-core --test interaction_store -- --nocapture
cargo test -p riviu-core --test interaction_schema -- --nocapture
```

Expected: concurrent writers serialize, duplicate requests are stable, conflicts are typed, schedule creation is atomic, and no existing DB tests regress.

- [ ] **Step 8: Commit the repository boundary**

```powershell
git add crates/core/src/interaction/mod.rs crates/core/src/interaction/store.rs crates/core/tests/interaction_store.rs
git diff --cached --name-only
git commit -m "feat(core): add interaction campaign repository"
```

### Task 6: Persist immutable plans and actor snapshots atomically

**Files:**
- Modify: `crates/core/src/interaction/store.rs`
- Modify: `crates/core/src/interaction/planner.rs`
- Create: `crates/core/tests/interaction_plan_store.rs`

- [ ] **Step 1: Write the failing all-or-nothing plan test**

Use a store test hook that fails after the first assignment insert:

```rust
#[tokio::test]
async fn failed_plan_commit_leaves_no_partial_actor_assignment_or_action_rows() {
    let store = test_store_with_failpoint("plan.after_first_assignment").await;
    let campaign = seed_queued_campaign(&store, 2, 2).await;
    let plan = build_plan_for(&store, &campaign).await;

    assert!(store.commit_plan(plan).await.is_err());
    assert_eq!(store.count_campaign_actors(&campaign.id).await.unwrap(), 0);
    assert_eq!(store.count_assignments(&campaign.id).await.unwrap(), 0);
    assert_eq!(store.count_action_runs(&campaign.id).await.unwrap(), 0);
    assert_eq!(store.get_campaign(&campaign.id).await.unwrap().status, CampaignStatus::Queued);
}
```

Add a concurrent double-plan test; exactly one caller must commit and the other must return the already persisted immutable plan, not generate a second seed.

```powershell
cargo test -p riviu-core --test interaction_plan_store -- --nocapture
```

Expected RED: missing `commit_plan` and plan persistence DTOs.

- [ ] **Step 2: Add persistence input structures**

Define a `PlanCommit` that contains the campaign revision read by the dispatcher, the planner seed, all eligible and excluded actor snapshots, planned assignments, and action decisions. The store owns IDs and ordinals so callers cannot insert inconsistent foreign keys.

Derive the root `u64` planner seed from SHA-256 over the fixed domain separator `riviu-interaction-plan-v1`, campaign ID, and persisted request hash. Do not use process RNG at dispatch time. This makes competing planners/recovery produce the same immutable sampling before the first plan commit; per-assignment seeds still come from the seeded `StdRng` in Task 2.

```rust
pub struct PlanCommit {
    pub campaign_id: CampaignId,
    pub expected_revision: i64,
    pub planner_seed: String,
    pub actors: Vec<PlannedActorSnapshot>,
    pub assignments: Vec<PlannedAssignmentDraft>,
}

pub struct PlannedActorSnapshot {
    pub account: AccountBinding,
    pub effective_proxy: EffectiveProxySnapshot,
    pub status: ActorSnapshotState,
    pub exclusion_code: Option<String>,
}
```

Excluded actors are persisted in `interaction_campaign_actors`; they are not silently discarded. Their exclusion reason feeds terminal campaign classification when no assignment is runnable.

- [ ] **Step 3: Implement a compare-and-swap plan transaction**

`commit_plan()` must:

1. Re-read the campaign and require `status=Queued`, `planned_at IS NULL`, and the expected revision.
2. Recheck the already-persisted target count is at most 500. For a `TooManyAssignments` plan outcome, persist all eligible/excluded actor snapshots, set campaign `Failed/TooManyAssignments`, make dispatch terminal, and create zero assignments/actions in this transaction.
3. Re-read canonical `device_meta.proxy_id` plus current proxy configuration revision for every actor and reject any ID/revision mismatch; never compare or persist secret proxy fields in the plan.
4. Persist the root deterministic `planner_seed` on the campaign, actor snapshots, assignment order, each decimal `u64` `assignment_seed`, effective settings, sampled Watch duration, sampled target/action delays, and one `attempt_no=1` action row per planned action. The assignment begins with `identity_state=Pending`, `current_identity_attempt_no=1`, and `identity_copy_intent=None`; the initial identity action carries the same intent. All non-identity actions also store `None` and can never change that field. Validate `watch_duration_ms` is non-null only for a selected Watch and lies within the persisted effective Watch bounds. Persist the root seed even when `TooManyAssignments` creates no assignment rows.
5. Set campaign `planned_at`, keep its public state `Queued`, increment revision, and set dispatch to `ready` in the same transaction. The first assignment transition to `Preparing` moves the campaign to `Running`; there is no hidden `Dispatching` campaign state.

If a plan already exists, load it and compare its canonical digest to the proposed plan. Return it only when equal; otherwise return `InvalidTransition` rather than replacing rows. No update API may mutate targets, actors, settings, probabilities, payloads, or action decisions after this commit. Add an assertion that a rejected-capacity plan leaves durable actor rows but no assignment/action rows.

- [ ] **Step 4: Test the current one-account invariant and future-ready account list**

Verify a current device with one default account produces one actor snapshot. Verify the planner/store accept multiple distinct `AccountBinding` rows without schema changes, while enforcing `UNIQUE(campaign_id, account_id, target_id)`. Add a pure planner test showing two future accounts on one UDID remain distinct snapshots and share the same serialized device queue.

Keep the current production provider stricter than the future-ready schema: `AllOnline`, `interaction_list_accounts`, preview, start, and schedule expose only rows with `is_default=1`. An explicit non-default account ID is retained in the availability audit as `AccountSwitchUnsupported` and creates zero runnable assignments/device work. Treat a missing/disabled default binding as a typed actor exclusion, not as a reason to invent a username. Persist configured username/label as operator metadata only; neither proves the TikTok login nor authorizes account switching. This phase never reads credentials or switches accounts. Add store/service tests for the default-only list, explicit non-default rejection, and zero device calls.

- [ ] **Step 5: Run planner persistence tests**

```powershell
cargo test -p riviu-core --test interaction_plan_store -- --nocapture
cargo test -p riviu-core --test interaction_planner -- --nocapture
```

Expected: failpoint rollback is clean, plan CAS is deterministic, caps are enforced, actor exclusions are durable, and the existing planner tests stay green.

- [ ] **Step 6: Commit immutable plan persistence**

```powershell
git add crates/core/src/interaction/store.rs crates/core/src/interaction/planner.rs crates/core/tests/interaction_plan_store.rs
git diff --cached --name-only
git commit -m "feat(core): persist immutable interaction plans"
```

### Task 7: Make action transitions, aggregation, cancellation, and retry durable

**Files:**
- Create: `crates/core/src/interaction/progress.rs`
- Modify: `crates/core/src/interaction/store.rs`
- Modify: `crates/core/src/interaction/mod.rs`
- Modify: `crates/core/src/interaction/types.rs`
- Create: `crates/core/tests/interaction_transitions.rs`

- [ ] **Step 1: Write failing tests for effect intent and atomic aggregation**

Start with the irreversible boundary test:

```rust
#[tokio::test]
async fn effect_intent_is_committed_before_the_side_effect_callback_runs() {
    let (store, action) = seeded_running_action(PlannedActionKind::Comment).await;
    store.issue_effect_intent(&action.id).await.unwrap();

    // This read represents the observation made at the final-gesture boundary.
    let reloaded = store.get_action(&action.id).await.unwrap();
    assert_eq!(reloaded.effect_intent, EffectIntent::Issued);
}
```

Do not literally keep a SQLite transaction open while the callback awaits. Implement this as two phases: a committed CAS to `Issued`, followed by the callback. Add a failpoint between updating an action and aggregating its assignment; assert the transaction rolls back both changes.

```powershell
cargo test -p riviu-core --test interaction_transitions -- --nocapture
```

Expected RED: unresolved progress port/transition methods.

- [ ] **Step 2: Define the executor-to-store progress contract**

Create a narrow progress port. Verified action implementations may report only typed state and evidence through this port; they do not write SQLite directly.

```rust
#[async_trait::async_trait]
pub trait InteractionProgress: Send + Sync {
    async fn assignment_progress(
        &self,
        assignment_id: &AssignmentId,
        update: AssignmentProgressUpdate,
    ) -> Result<(), ProgressError>;
    async fn start_opening_attempt(
        &self,
        identity_action_id: &ActionRunId,
        reason: Option<OpeningRetryReason>,
    ) -> Result<OpeningAttempt, ProgressError>;
    async fn finish_opening_attempt(
        &self,
        attempt_id: &OpeningAttemptId,
        outcome: OpeningAttemptOutcome,
    ) -> Result<(), ProgressError>;
    async fn issue_identity_copy_intent(&self, identity_action_id: &ActionRunId) -> Result<(), ProgressError>;
    async fn action_prepared(
        &self,
        action_id: &ActionRunId,
        prepared_payload: Option<&PreparedActionPayload>,
    ) -> Result<(), ProgressError>;
    async fn action_started(&self, action_id: &ActionRunId) -> Result<(), ProgressError>;
    async fn issue_effect_intent(&self, action_id: &ActionRunId) -> Result<(), ProgressError>;
    async fn action_finished(
        &self,
        action_id: &ActionRunId,
        outcome: &ActionOutcome,
        evidence: &[EvidenceRef],
    ) -> Result<(), ProgressError>;
    async fn schedule_bounded_action_retry(
        &self,
        action_id: &ActionRunId,
        reason: ActionRetryReason,
    ) -> Result<ActionRunId, ProgressError>;
}
```

Keep the shared contracts in `crates/core/src/interaction/types.rs`: `PlannedActionKind`, `PreparedActionPayload`, `ActionOutcome`, `ActionOutcomeCode`, `EvidenceRef`, `EffectIntent`, `IdentityCopyIntent`, `OpeningAttempt`, `OpeningAttemptOutcome`, `OpeningRetryReason`, `AssignmentProgressUpdate`, `ActionStatus`, and `AssignmentStatus`. `AssignmentProgressUpdate` permits only the intermediate states plus typed `SkippedUnavailable`, `SkippedUnsupported`, and `Cancelled` outcomes; success/partial/failure/interrupted/uncertain remain reducer or recovery results. `ActionOutcome` must be a tagged enum or validated struct; arbitrary executor strings must not become database states.

Use this closed payload/evidence shape so later verified-action plans extend a reviewed enum rather than pass arbitrary JSON:

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", content = "value", rename_all = "camelCase")]
pub enum PreparedActionPayload {
    NoPayload,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ActionOutcomeCode {
    FrameConfirmed,
    DesiredStateAlreadyPresent,
    TargetChanged,
    TargetUnverified,
    TargetIdentityAmbiguous,
    UnsupportedCapability,
    UnsupportedGeometry,
    DeviceUnavailable,
    TextNotArmed,
    TextNotSent,
    GestureFailed,
    CancelledBeforeStart,
    ProcessLost,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ActionRetryReason {
    DesiredStateUnconfirmed,
    TransportBeforeGesture,
    TextNotArmed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum OpeningRetryReason {
    TransportBeforeOpen,
    TargetFrameUnconfirmed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum OpeningAttemptOutcome {
    Confirmed,
    Failed {
        code: ActionOutcomeCode,
        retry_reason: Option<OpeningRetryReason>,
    },
    Interrupted { code: ActionOutcomeCode },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum OpeningAttemptStatus { Running, Confirmed, Failed, Interrupted }

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct OpeningAttempt {
    pub id: OpeningAttemptId,
    pub assignment_id: AssignmentId,
    pub identity_action_run_id: ActionRunId,
    pub attempt_no: u32,
    pub status: OpeningAttemptStatus,
    pub retry_reason: Option<OpeningRetryReason>,
    pub outcome_code: Option<ActionOutcomeCode>,
    pub started_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum ActionOutcome {
    Succeeded { code: ActionOutcomeCode },
    AlreadySatisfied { code: ActionOutcomeCode },
    NotConfirmed { code: ActionOutcomeCode },
    Uncertain { code: ActionOutcomeCode },
    Failed { code: ActionOutcomeCode },
    Skipped { code: ActionOutcomeCode },
    Interrupted { code: ActionOutcomeCode },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum EvidenceKind {
    TargetIdentity,
    BeforeCrop,
    AfterCrop,
    FullFrame,
    TextReadBack,
    RecipientMatch,
    Timing,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct EvidenceRef {
    pub artifact_id: String,
    pub kind: EvidenceKind,
    pub sha256: String,
    pub observed_at: DateTime<Utc>,
}
```

Add validation tables that permit only meaningful status/code pairs and verify every evidence reference belongs to the same assignment/action before committing it. Deterministic stale/mismatched identity is `Failed/TargetUnverified`; uncertainty over whether the Copy Link tap or read-back completed is `Uncertain/TargetIdentityAmbiguous`. The ambiguous pair is valid only for a `TargetIdentityCopyLink` row whose identity intent is Issued; success likewise requires Issued. The two failure classes are never interchangeable. `PreparedActionPayload` starts closed with `NoPayload`; G2 adds the typed Comment payload and G4 adds the typed DirectMessage payload in their own reviewed commits. Arbitrary JSON is never an accepted payload.

- [ ] **Step 3: Implement legal compare-and-swap transitions**

Create store-backed `InteractionProgress` methods with explicit transition tables:

```text
Action:     Pending -> Running -> Succeeded|AlreadySatisfied|NotConfirmed|Uncertain|Failed|Skipped|Interrupted
Assignment: Queued -> WaitingCapacity -> Preparing -> Session -> Stream -> Opening -> Verifying -> Acting -> Succeeded|Partial|Failed|SkippedUnavailable|SkippedUnsupported|Cancelled|Interrupted|Uncertain
Campaign:   Scheduled|Queued -> Running -> Succeeded|Partial|Failed|Cancelled|Interrupted|Missed
```

Every mutation must match the expected state and revision where the row has a revision column. `issue_effect_intent()` is idempotent for an already-issued intent but rejects terminal actions. `action_finished()` updates the action, recomputes its assignment, recomputes that account's campaign-actor outcome, then recomputes campaign status/counters/result and increments its revision in one immediate transaction.

`issue_identity_copy_intent(identity_action_id)` is a CAS on that exact Running `TargetIdentityCopyLink` attempt and commits immediately before its one Copy Link tap. It changes that row and the matching assignment's current `identity_copy_intent` projection from `None` to `Issued` in one transaction; an issued identity attempt is immutable thereafter. A crash after issuance makes that action and assignment `Uncertain/TargetIdentityAmbiguous`, and neither recovery nor operator retry reuses that attempt. `action_started()` moves Pending to Running but authorizes no gesture by itself. `action_prepared()` creates a payload only while Running, or returns idempotently for byte-equivalent stored data; a different payload is rejected. `issue_effect_intent()` accepts only Comment, Repost, or DirectMessage action kinds, and Comment/DirectMessage require their later gate-specific typed payload first. A terminal success/ambiguous outcome for any of those kinds is rejected unless intent is already issued. Add negative tests for every guard, including a stale action ID whose attempt does not match the assignment projection.

`start_opening_attempt()` implements the only in-run Opening retry budget: `MAX_OPENING_ATTEMPTS_PER_IDENTITY = 2` (initial plus one retry). It appends an attempt before each `open_url` call and permits attempt 2 only after attempt 1 is terminal `Failed` with `retry_reason=Some(TransportBeforeOpen|TargetFrameUnconfirmed)`, while the referenced current identity attempt and assignment projection are both `None`. `finish_opening_attempt()` stores the deterministic terminal `ActionOutcomeCode` separately from the optional retry classification; this makes the `OpeningAttemptOutcome` type, SQLite columns, and retry decision agree. Prior opening rows are immutable. Once identity intent is Issued, a second open is rejected. A Running opening row found at restart becomes `Interrupted`; startup never appends or dispatches its successor automatically.

`schedule_bounded_action_retry()` creates the next durable attempt row before another device action. Allow only Like/Follow/Save re-read retries and Comment `TextNotArmed` before send intent; preserve ordinal, sampled decision, `delay_before_ms`, `watch_duration_ms`, and prepared payload. Encode reviewed maximum attempts as constants (three total attempts for Comment: two pre-recovery strikes plus one post-refresh attempt; three for Like/Follow/Save), reject issued intent/uncertain outcomes, and never mutate the previous attempt. Add boundary and concurrent-CAS tests. The same tests prove Opening stops after two durable attempts, after process loss, and after identity intent issuance.

- [ ] **Step 4: Implement durable cooperative cancellation**

`request_cancel(campaign_id)` atomically sets `cancel_requested_at`, increments revision, and leaves running work visible. Dispatcher and executor query a `CancellationProbe` before each assignment and before every not-yet-issued action. Pending work becomes `Skipped(result_code='cancelled_before_start')`; an action with issued intent is never relabeled as definitely cancelled. Aggregate to `Cancelled` only when nothing remains running and no uncertain result outranks cancellation.

For an unplanned `Scheduled` campaign, the same transaction marks its schedule cancelled and the campaign terminal `Cancelled`, with no dispatch row. For an unplanned run-now campaign, mark its dispatch terminal. Test that a due-tick racing cancellation wins by revision CAS on exactly one terminal outcome and never creates runnable cancelled work.

Test duplicate cancellation, cancellation before planning, during an assignment, and after terminal completion. Terminal campaigns return their existing summary without revision churn.

- [ ] **Step 5: Implement explicit retry request idempotency**

Add `retry_campaign(RetryCampaignRequest)` with `retry_request_id` and canonical hash over campaign ID plus sorted/deduplicated assignment IDs. Reject an empty set and IDs from another campaign. In one transaction:

1. Return the existing retry result when ID and hash match; reject a hash conflict.
2. Require every selected assignment to be terminal `Partial|Failed|Interrupted`, its affected actor projection not to be `Uncertain|SkippedUnavailable|SkippedUnsupported`, the campaign to be terminal `Partial|Failed|Interrupted`, and `cancel_requested_at IS NULL`; a cancelled `Partial` campaign is not retryable. Then classify the latest identity action as either `Confirmed`, or `PreCopyRetryable` when it is terminal `Failed|Interrupted` with `identity_copy_intent=None`. Any issued-but-unconfirmed, `Uncertain/TargetIdentityAmbiguous`, Running, or deterministic post-Copy mismatch is ineligible. The same `TargetUnverified` code is retryable only when the persisted intent is `None`; code alone never decides this boundary.
3. For `Confirmed`, select the highest attempt of each eligible optional action that ended `Failed` or `Interrupted` and whose effect intent was never issued. For `PreCopyRetryable`, select every originally planned optional action whose latest row was never issued and did not reach a positive terminal result, including work left Pending/Interrupted because identity never opened; `NotPlanned` remains untouched.
4. Append exactly one new mandatory identity action attempt with the same ordinal/decision/timing, `attempt_no + 1`, `Pending`, and `identity_copy_intent=None`, then append each selected optional action attempt with its immutable decision, timing, and prepared payload. Prior identity/effect/opening rows are never updated.
5. Requeue only the selected assignments, atomically set each assignment projection to the new `current_identity_attempt_no`, `identity_copy_intent=None`, and `identity_state=Pending`, then recompute each affected actor to `Eligible` while it owns queued retry work and CAS the campaign from `Partial|Failed|Interrupted` to `Queued`. Preserve every unselected positive assignment/action and every unaffected actor byte-for-byte, recompute counters from the resulting rows, and insert/update dispatch. The new batch must finish its new identity attempt before any retried optional action.

There is no `Issued -> None` update on an action row. The assignment projection moves atomically to a newly inserted Pending identity row with `None`; every preceding action row, including an Issued Confirmed row, remains immutable forever.

For the operator `interaction_retry` command, `Uncertain`, final optional-action `NotConfirmed` after the bounded in-run budget, issued action intent, and issued-but-unconfirmed identity are ineligible. A no-intent pre-Copy failure and a Failed/Interrupted optional effect after Confirmed identity are eligible through the two branches above, but neither reuses an old identity attempt. Add separate tests for no-intent exhausted Opening, no-intent process loss, issued deterministic `TargetUnverified`, `TargetIdentityAmbiguous`, issued effect, failed effect after Confirmed identity, new identity-attempt ordering, and two concurrent retry submissions. Add a mixed-action test in which one positive optional action plus one retryable failure reduces the assignment to `Partial`, and a mixed-device test in which one actor remains `Succeeded` while a selected failed actor and the `Partial` campaign alone reopen; both must append new rows without replaying the positive work. Pair them with a cancelled `Partial` campaign and an actor with an unresolved uncertain projection; both must return a typed retry block and append nothing.
If the validated selection contains zero retryable actions, persist the retry request with a typed `NoRetryableActions` result and return an idempotent no-op (`changed=false`); do not requeue the campaign.

This transaction is the sole reviewed exception to the forward-only display graph: it CASes selected terminal `Partial|Failed|Interrupted` assignments and a terminal `Partial|Failed|Interrupted` campaign back to `Queued`, plus affected terminal actor rows back to `Eligible`, only while appending the new identity/effect attempts above. No generic progress method may perform a terminal-to-running transition, and a `Succeeded`, `Uncertain`, `Cancelled`, or skipped row is never reopened.

- [ ] **Step 6: Run transition and reducer tests**

```powershell
cargo test -p riviu-core --test interaction_transitions -- --nocapture
cargo test -p riviu-core --test interaction_aggregation -- --nocapture
```

Expected: illegal transitions fail without partial writes, effect intent is visible before callbacks, aggregation is transactional, and retry/cancel operations are idempotent.

- [ ] **Step 7: Commit durable progress handling**

```powershell
git add crates/core/src/interaction/mod.rs crates/core/src/interaction/progress.rs crates/core/src/interaction/store.rs crates/core/src/interaction/types.rs crates/core/tests/interaction_transitions.rs
git diff --cached --name-only
git commit -m "feat(core): persist interaction progress and retries"
```

### Task 8: Add bounded cursor pagination and post-commit revisions

**Files:**
- Create: `crates/core/src/interaction/cursor.rs`
- Modify: `crates/core/src/interaction/progress.rs`
- Modify: `crates/core/src/interaction/store.rs`
- Modify: `crates/core/src/interaction/mod.rs`
- Modify: `crates/core/src/interaction/types.rs`
- Create: `crates/core/tests/interaction_pagination.rs`

- [ ] **Step 1: Write failing stable-pagination tests**

Seed records with identical timestamps and page through them while inserting a newer row between requests:

```rust
#[tokio::test]
async fn campaign_cursor_has_no_duplicate_or_gap_during_newer_inserts() {
    let store = seeded_store_with_campaigns(125, fixed_time()).await;
    let first = store.list_campaigns(None, 50).await.unwrap();
    seed_newest_campaign(&store).await;
    let second = store.list_campaigns(first.next_cursor.as_deref(), 50).await.unwrap();
    let third = store.list_campaigns(second.next_cursor.as_deref(), 50).await.unwrap();

    let ids = pages(&[first, second, third]);
    assert_eq!(ids.len(), 125);
    assert_eq!(ids.iter().collect::<HashSet<_>>().len(), 125);
}
```

Add equivalent tests for targets, assignments, action runs, and artifacts. Assert default limit 50, maximum 200, zero/201 rejected, malformed cursor rejected, cross-resource cursor rejected, and no SQL fragment can enter through a cursor.

```powershell
cargo test -p riviu-core --test interaction_pagination -- --nocapture
```

Expected RED: cursor codec and paged repository methods are missing.

- [ ] **Step 2: Implement opaque versioned cursors**

Encode a small JSON payload with URL-safe base64 using the existing `base64` dependency:

```rust
#[derive(Serialize, Deserialize)]
struct CursorV1 {
    version: u8,
    resource: CursorResource,
    scope_id: Option<String>,
    filter_hash: String,
    position: CursorPosition,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
enum CursorPosition {
    Campaign { created_at: String, row_id: String },
    Target { ordinal: u32, row_id: String },
    Assignment { ordinal: u32, row_id: String },
    ActionRun { action_ordinal: u32, attempt_no: u32, row_id: String },
    Artifact { created_at: String, row_id: String },
}
```

Use a deterministic `(created_at DESC, id DESC)` keyset predicate for campaigns/artifacts, `(target_ordinal ASC, id ASC)` for targets, `(assignment_ordinal ASC, id ASC)` for assignments, and `(action_ordinal ASC, attempt_no ASC, id ASC)` for action runs. Hash the canonical status/filter object into `filter_hash`; the cursor includes resource, campaign scope, and filter hash, so it cannot be replayed against another collection or changed filter. Decode into typed values and bind all values as SQL parameters. Never use `OFFSET` for these APIs.
Reject an encoded cursor longer than 2,048 bytes, unknown version/resource/position combinations, non-canonical timestamps, negative/out-of-range ordinals, trailing JSON, and decoded payloads that do not re-encode canonically.

- [ ] **Step 3: Implement summary and detail repository queries**

Add:

```rust
pub async fn list_campaigns(&self, filter: &CampaignListFilter, cursor: Option<&str>, limit: usize) -> Result<Page<CampaignSummary>, StoreError>;
pub async fn get_campaign(&self, id: &CampaignId) -> Result<CampaignDetail, StoreError>;
pub async fn list_targets(&self, id: &CampaignId, cursor: Option<&str>, limit: usize) -> Result<Page<TargetView>, StoreError>;
pub async fn list_assignments(&self, id: &CampaignId, filter: &AssignmentListFilter, cursor: Option<&str>, limit: usize) -> Result<Page<AssignmentView>, StoreError>;
pub async fn get_assignment(&self, id: &AssignmentId) -> Result<AssignmentView, StoreError>;
pub async fn list_action_runs(&self, assignment_id: &AssignmentId, cursor: Option<&str>, limit: usize) -> Result<Page<ActionRunView>, StoreError>;
pub async fn list_artifacts(&self, filter: &ArtifactListFilter, cursor: Option<&str>, limit: usize) -> Result<Page<ArtifactView>, StoreError>;
```

`CampaignListFilter` accepts a bounded typed set of statuses. `AssignmentListFilter` accepts bounded typed status/account/target/UDID sets. `ArtifactListFilter` requires campaign scope and optionally narrows assignment, action run, evidence kind, or storage state; reject empty strings and excessive values before SQL construction. `CampaignSummary` contains configuration summary, counts, and top-level state only. Do not load nested assignment/action histories for campaign list rows. Detail children stay paged and preserve immutable snapshot fields.

`AssignmentView` exposes `retry_eligible`, `retry_blocked_reason: Option<RetryBlockedReason>`, `current_identity_attempt_no`, `current_identity_intent`, and `identity_state` from the assignment projection only after verifying it equals the highest identity row. The closed reason distinguishes `IdentityIssuedUnconfirmed`, deterministic post-Copy `TargetUnverified`, `TargetIdentityAmbiguous`, `EffectIntentIssued`, and the other declared cases; a terminal no-intent pre-Copy failure can remain backend-authorized for retry. No free-form database string reaches the client. `ActionRunView` exposes `attempt_no`, `identity_copy_intent`, and a bounded `opening_attempts: Vec<OpeningAttempt>` populated only for that identity action (maximum two), so the UI can show immutable identity/opening history without inferring it from assignment status. Opening attempts are backend audit data and are never flattened into retry eligibility on the client.

- [ ] **Step 4: Return post-commit revision envelopes**

Every mutation returns `CommittedMutation<T> { value, summary, changed_assignment_ids, changed }`, populated by re-reading inside the transaction just before commit. `changed=false` is reserved for a truly idempotent no-op such as a same-hash duplicate request. The caller may emit an event only after `write()` reports a successful commit and only when `changed=true`. Add a failpoint on commit and assert no envelope is returned.

Define `InteractionUpdateSink::publish(&CommittedInteractionUpdate)` in `progress.rs` and inject it into the store-backed progress/service composition. The sink is invoked after `write().await` returns, so command, dispatcher, recovery, and executor progress mutations all follow the same event rule. Tests use a recording sink and assert rollback/no-op paths publish nothing; Task 11 adapts it to `EventBus`.

- [ ] **Step 5: Run pagination tests**

```powershell
cargo test -p riviu-core --test interaction_pagination -- --nocapture
```

Expected: all five resource types paginate deterministically under insertion, enforce bounds/scope, and return only committed revisions.

- [ ] **Step 6: Commit pagination**

```powershell
git add crates/core/src/interaction/cursor.rs crates/core/src/interaction/mod.rs crates/core/src/interaction/progress.rs crates/core/src/interaction/store.rs crates/core/src/interaction/types.rs crates/core/tests/interaction_pagination.rs
git diff --cached --name-only
git commit -m "feat(core): add interaction cursor pagination"
```

### Task 9: Store evidence artifacts with retention and crash reconciliation

**Files:**
- Create: `crates/core/src/interaction/artifacts.rs`
- Modify: `crates/core/src/interaction/store.rs`
- Modify: `crates/core/src/interaction/mod.rs`
- Create: `crates/core/tests/interaction_artifacts.rs`

- [ ] **Step 1: Write failing artifact lifecycle tests**

Use a temporary artifact root and a deterministic clock. Also cover an artifact over the fixed 16 MiB per-file limit, quota enforcement immediately after a successful finalize, and a quota blocked only by active/pinned files:

```rust
#[tokio::test]
async fn successful_artifact_is_purged_after_14_days_but_metadata_and_hash_remain() {
    let fixture = artifact_fixture().await;
    let saved = fixture.store.put(
        ArtifactOwner::Action(fixture.succeeded_action_id.clone()),
        ArtifactKind::FrameEvidence,
        b"jpeg-fixture",
        "image/jpeg",
    ).await.unwrap();

    fixture.clock.advance(Duration::days(15));
    let report = fixture.store.enforce_retention().await.unwrap();
    assert_eq!(report.files_removed, 1);
    assert!(!saved.absolute_path.exists());
    let metadata = fixture.db.get_artifact(&saved.id).await.unwrap();
    assert_eq!(metadata.sha256, sha256_hex(b"jpeg-fixture"));
    assert_eq!(metadata.storage_state, ArtifactStorageState::Purged);
}
```

Add tests for failed/uncertain/not-confirmed retention at 30 days, active and pinned artifacts never purged, a configurable test quota, oldest-eligible-first eviction, partial temp files, missing files, checksum mismatch, and a database insert failure after file creation.

```powershell
cargo test -p riviu-core --test interaction_artifacts -- --nocapture
```

Expected RED: unresolved `ArtifactStore` and artifact state types.

- [ ] **Step 2: Implement canonical artifact paths and atomic writes**

`ArtifactStore` receives an absolute canonical root, `InteractionStore`, clock, and retention policy. Generate paths internally as `<root>/<campaign>/<assignment>/<artifact-id>.<extension>`; reject caller filenames and any path that resolves outside the root. Reject file payloads over `MAX_ARTIFACT_BYTES = 16 * 1024 * 1024` before creating a temp file. Write to a sibling `*.partial`, call `sync_all`, atomically rename, sync the parent directory where supported, compute SHA-256 and byte length, then insert metadata. If metadata commit fails, delete the newly renamed unreferenced file.
Reject symlink/reparse-point components below the managed root and open existing files only after canonical containment is rechecked; reconciliation never follows directory links.
Run hashing, sync, rename, directory walks, and deletion in `spawn_blocking`; no blocking filesystem call or SQLite call runs on a Tokio worker, and no database transaction spans file I/O.

Add the reserved typed comment output alongside generic evidence metadata:

```rust
pub struct CommentArtifact {
    pub target_key: String,
    pub account_id: AccountId,
    pub configured_account_handle: Option<String>,
    pub normalized_text: String,
    pub text_sha256: String,
    pub sent_at: DateTime<Utc>,
    pub platform_comment_id: Option<String>,
    pub screenshot: EvidenceRef,
}
```

Validate it against the owning assignment/action and store the typed JSON as artifact metadata only after Comment is frame-confirmed. It reserves later reply linkage but is not live account proof and does not make `parent_assignment_id` executable. Current planner always writes `parent_assignment_id=NULL`; store validates any future non-null parent belongs to the same campaign and rejects self-parenting.
`put_typed_metadata()` canonicalizes the JSON, hashes those bytes, uses `storage_state='metadataOnly'`, `relative_path=NULL`, and never purges the payload/hash under the file quota; only its referenced screenshot follows file retention.

```rust
pub struct ArtifactMetadata {
    pub id: String,
    pub campaign_id: String,
    pub assignment_id: Option<String>,
    pub action_run_id: Option<String>,
    pub kind: ArtifactKind,
    pub relative_path: Option<String>,
    pub sha256: String,
    pub byte_len: u64,
    pub media_type: String,
    pub storage_state: ArtifactStorageState,
    pub pinned: bool,
    pub created_at: DateTime<Utc>,
    pub purge_after: DateTime<Utc>,
}
```

Store only relative paths in SQLite. `open_artifact()` re-canonicalizes under the configured root and returns a typed `Purged`/`Missing` result rather than leaking filesystem errors through Tauri.

- [ ] **Step 3: Implement retention and the 5 GiB quota**

Default policy is success 14 days, failed/uncertain/not-confirmed 30 days, and total retained file bytes at most 5 GiB. Cleanup first purges expired eligible files, then evicts oldest eligible files until under quota. It must never purge artifacts for non-terminal campaigns or rows marked pinned. Use a recoverable two-phase file transition: commit `storage_state='purging'` with the original relative path, delete the canonical file outside SQLite, then commit `relative_path=NULL`, `storage_state='purged'`, and `purged_at`. Never hold a SQLite transaction during filesystem deletion.

Run quota enforcement after every successful file finalization as well as during startup and the 24-hour maintenance pass. If active/pinned files alone prevent reaching the cap, return and emit typed `ArtifactQuotaBlocked` counts, reject subsequent full-frame files until eligible space exists, and continue accepting bounded metadata-only evidence. Never silently grow past the cap or purge active/pinned evidence.

- [ ] **Step 4: Reconcile files and metadata after crashes**

At startup and before quota accounting:

1. Remove stale `*.partial` files older than the configured grace period.
2. Finish `purging` rows deterministically: an absent file becomes `purged`; a present file is retried, while metadata for any other missing file becomes `missing` with hash/size preserved.
3. Hash existing files and mark mismatches `corrupt`; never serve corrupt data.
4. Remove unreferenced files only after the grace period and only inside the canonical root.

Produce a `ReconciliationReport` with counts and artifact IDs, but no file content. Re-running reconciliation must be idempotent.

- [ ] **Step 5: Run artifact tests**

```powershell
cargo test -p riviu-core --test interaction_artifacts -- --nocapture
```

Expected: atomic-write failures leave neither trusted metadata nor orphan data, retention windows and quota are exact, and reconciliation is idempotent.

- [ ] **Step 6: Commit artifact management**

```powershell
git add crates/core/src/interaction/artifacts.rs crates/core/src/interaction/mod.rs crates/core/src/interaction/store.rs crates/core/tests/interaction_artifacts.rs
git diff --cached --name-only
git commit -m "feat(core): manage interaction evidence artifacts"
```

### Task 10: Implement the durable dispatcher and scheduler with fake actions

**Files:**
- Create: `crates/core/src/interaction/executor.rs`
- Create: `crates/core/src/interaction/scheduler.rs`
- Create: `crates/core/src/interaction/dispatcher.rs`
- Modify: `crates/core/src/interaction/links.rs`
- Modify: `crates/core/src/interaction/mod.rs`
- Modify: `crates/core/src/interaction/store.rs`
- Create: `crates/core/tests/interaction_dispatcher.rs`
- Create: `crates/core/tests/interaction_recovery.rs`

- [ ] **Step 1: Re-prove the completed Gate 0 ownership boundary**

```powershell
cargo test -p riviu-core device_work -- --nocapture
cargo test -p riviu-core stream_budget -- --nocapture
cargo test -p riviu-core shared_device_owner -- --nocapture
cargo test -p riviu-managers-phone shared_device_owner -- --nocapture
```

Expected: Gate 0 ownership, producer accounting, and cross-owner tests pass before dispatcher code exists. If any fails, repair Gate 0 in its own plan/commit before continuing; campaign core must not compensate with local synchronization.

- [ ] **Step 2: Write the failing device-queue and capacity test**

Use Gate 0's fake control plane and a controlled batch executor that records lease ownership:

```rust
#[tokio::test]
async fn scheduler_uses_gate_zero_control_plane_for_device_and_capacity_outcomes() {
    let fixture = dispatcher_fixture().with_control_plane_capacity(2).await;
    fixture.seed_assignments([
        assignment("a1", "udid-a"),
        assignment("a2", "udid-a"),
        assignment("b1", "udid-b"),
        assignment("c1", "udid-c"),
    ]).await;

    fixture.dispatcher.drain_ready_once().await.unwrap();

    assert_eq!(fixture.control_plane.max_owners_for("udid-a"), 1);
    assert_eq!(fixture.control_plane.max_stream_producers(), 2);
    assert_eq!(fixture.control_plane.lease_acquisitions_for("udid-a"), 1);
    assert_eq!(fixture.executor.context_id_for("a1"),
               fixture.executor.context_id_for("a2"));
    assert_eq!(fixture.store.get_assignment("c1").await.unwrap().status,
               AssignmentStatus::WaitingCapacity);
}
```

This test counts device batches/producers, not assignments: `a1` and `a2` share one `udid-a` lease/context, `b1` consumes the second producer, and only the third UDID waits at capacity two. Add tests proving the dispatcher never invokes a batch unless the Gate 0 executor reports a committed control-plane lease, waiting work remains durable, and the executor receives device groups in persisted ordinal order. Assert the dispatcher contains no local per-UDID mutex, device semaphore, or stream-capacity counter.

```powershell
cargo test -p riviu-core --test interaction_dispatcher -- --nocapture
```

Expected RED: dispatcher/executor contracts do not exist.

- [ ] **Step 3: Define narrow runtime ports**

The campaign core must not depend on WDA coordinates or TikTok detectors. Define these contracts in `executor.rs`:

```rust
#[async_trait::async_trait]
pub trait ActorAvailabilityProvider: Send + Sync {
    async fn snapshot(&self, selection: &ActorSelection) -> Result<Vec<ActorAvailability>, ExecutorError>;
}

#[async_trait::async_trait]
pub trait CancellationProbe: Send + Sync {
    async fn is_cancel_requested(&self, campaign_id: &CampaignId) -> Result<bool, ExecutorError>;
}

#[async_trait::async_trait]
pub trait InteractionBatchExecutor: Send + Sync {
    async fn execute_device_batch(
        &self,
        batch: DeviceBatchExecution,
        progress: Arc<dyn InteractionProgress>,
        cancellation: Arc<dyn CancellationProbe>,
    ) -> Result<BatchExecutionReport, ExecutorError>;
}

#[async_trait::async_trait]
pub trait InteractionCapabilities: Send + Sync {
    async fn snapshot(&self, udid: &str) -> Result<CapabilitySnapshot, ExecutorError>;
}

#[async_trait::async_trait]
pub trait InteractionTargetResolver: Send + Sync {
    async fn resolve_for_execution(
        &self,
        target: &ResolvedTikTokTarget,
    ) -> Result<ExecutionTargetResolution, ExecutorError>;
}

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
        context: &VerifiedTargetContext,
        action: &PreparedAction,
        progress: Arc<dyn InteractionProgress>,
    ) -> Result<ActionOutcome, ExecutorError>;
}
```

Use a closed capability contract:

```rust
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum CapabilityUnavailableCode {
    GateNotQualified,
    DeviceTupleUnqualified,
    MissingOpenUrl,
    MissingTargetIdentity,
    UnsupportedGeometry,
    TextRouteUnqualified,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum CapabilityState {
    Ready,
    Unavailable { code: CapabilityUnavailableCode },
    Deferred { code: CapabilityUnavailableCode },
}

pub struct CapabilitySnapshot {
    pub qualification_id: String,
    pub actions: BTreeMap<PlannedActionKind, CapabilityState>,
}

pub struct ActorAvailability {
    pub account: AccountBinding,
    pub device_online: bool,
    pub effective_proxy: EffectiveProxySnapshot,
    pub capabilities: CapabilitySnapshot,
    pub exclusion: Option<ActorExclusionCode>,
}
```

Snapshot every planned kind explicitly. An absent map key is a persisted invariant error, not implicit support or implicit Off. G1's production-disabled provider reports `TargetIdentityCopyLink`, Watch, Like, Follow, Comment, Save, Repost, and DirectMessage as `Deferred/GateNotQualified`; G2 later promotes identity/Watch only through its production-runtime gate and promotes Like/Follow/Comment independently, while G4 owns its three actions.

Define the referenced execution DTOs as immutable snapshots, not database handles:

```rust
pub struct DeviceBatchExecution {
    pub campaign_id: CampaignId,
    pub udid: String,
    pub assignments: Vec<AssignmentExecution>,
}

pub struct AssignmentExecution {
    pub assignment_id: AssignmentId,
    pub account: AccountBinding,
    pub target: ResolvedTikTokTarget,
    pub effective_proxy: EffectiveProxySnapshot,
    pub capabilities: CapabilitySnapshot,
    pub effective_settings: InteractionDefaults,
    pub assignment_seed: u64,
    pub target_delay_after_ms: u32,
    pub actions: Vec<PreparedAction>,
}

pub struct PreparedAction {
    pub action_run_id: ActionRunId,
    pub ordinal: u32,
    pub attempt_no: u32,
    pub kind: PlannedActionKind,
    pub identity_copy_intent: IdentityCopyIntent,
    pub delay_before_ms: u32,
    pub watch_duration_ms: Option<u32>,
    pub payload: Option<PreparedActionPayload>,
}

pub struct VerifiedTargetContext {
    pub assignment_id: AssignmentId,
    pub identity_action_run_id: ActionRunId,
    pub identity_attempt_no: u32,
    pub content_id: String,
    pub kind: TikTokPostKind,
    pub identity_evidence: Vec<EvidenceRef>,
}
```

`UiWithStreamContext` is the Gate 0 type, not a new campaign lock. `VerifiedTargetContext` may be constructed only after the latest device Copy Link attempt matches the immutable content ID/kind; it contains no driver, lease, stream handle, or arbitrary resolved metadata. The qualified batch adapter retains the Gate 0 context and passes a mutable borrow to `TikTokActionExecutor` only while it owns it. Repository loading must select only the highest attempt for each ordinal, require exactly one Pending/Running current identity row for runnable work, verify its attempt/intent equal the assignment projection, and copy `attempt_no`, `identity_copy_intent`, assignment seed, effective settings, `delay_before_ms`, `watch_duration_ms`, and `target_delay_after_ms` into these DTOs verbatim; dispatch and retry never resample planner-owned values. A retry batch never hydrates an old Confirmed identity as its current context. G4 may resolve a Direct Message recipient once during the first durable preparation, but only from the persisted seed/policy, and it must reload rather than replace any committed recipient.

Name the later production adapter `CoordinatedInteractionBatchExecutor`; it composes exactly one `Arc<DeviceControlPlane>` with one `Arc<dyn TikTokActionExecutor>`. This G1 plan defines its contract and disabled implementation, while the verified-actions plan supplies the qualified adapter behavior and live evidence.

`ExecutionTargetResolution` is exactly `Verified { resolved_url, content_id, kind }`, `Changed { observed_content_id, observed_kind }`, or `Unverified { code }`. `BatchExecutionReport` contains only processed assignment IDs and a typed batch completion code; it cannot grant ownership. The injected Gate 0 adapter persists `WaitingCapacity` through `InteractionProgress` while it still holds `DeviceExclusiveContext`, persists `SkippedUnavailable` when `try_acquire_exclusive` fails, and does not return until it has released the matching control-plane context. `CapabilitySnapshot` is a typed map from `PlannedActionKind` to `CapabilityState` (`Ready`, `Unavailable { code }`, `Deferred { code }`); it is a preflight signal, not success evidence. Verified action plans implement `TikTokActionExecutor` behind the Gate 0 batch adapter. This core slice supplies fake implementations only under `#[cfg(test)]` plus a fail-closed disabled batch adapter; there is no second allocator.

Implement `InteractionTargetResolver` for an adapter around Task 1A's `TikTokLinkResolver`; execution uses the identical redirect policy and compares returned content ID/kind against the immutable snapshot. Do not create a second URL parser or trust a caller-provided execution result.

- [ ] **Step 4: Implement atomic dispatch claims**

Add store methods using revision CAS:

```rust
pub async fn claim_next_dispatch(&self, owner: &str, now: DateTime<Utc>) -> Result<Option<DispatchClaim>, StoreError>;
pub async fn renew_dispatch_claim(&self, claim: &DispatchClaim, now: DateTime<Utc>) -> Result<DispatchClaim, StoreError>;
pub async fn release_dispatch_claim(&self, claim: DispatchClaim, next: DispatchRelease) -> Result<(), StoreError>;
```

`claim_next_dispatch` selects one due `queued|ready` row ordered by `(available_at, campaign_id)`, then updates `state='claimed'`, owner, start time, and revision in the same immediate transaction. A stale or wrong-owner release/renew fails. Do not use a process-memory queue as canonical state.

Use fixed constants `DISPATCH_CLAIM_TTL = 30 seconds` and `DISPATCH_RENEW_INTERVAL = 10 seconds`. While a batch call is active, a heartbeat renews only the matching owner/revision. Losing the claim sets the durable cancellation probe for that worker; it finishes the current atomic gesture/verification but must not begin another action. Tests use a fake clock to prove a 30-second-old claim is not stale, a strictly older claim is audited, and wrong-owner heartbeats never revive it.

- [ ] **Step 5: Implement deterministic scheduling**

`InteractionScheduler::materialize_due(now)` claims due `Once` schedules and, in one revision-CAS transaction, either changes campaign `Scheduled -> Queued`, marks the schedule dispatched, and inserts its dispatch row, or changes both schedule/campaign to terminal `Missed` when lateness is greater than 60 seconds. Reject invalid/past timestamps at request time. Exactly 60 seconds late is eligible; 60 seconds plus one tick is missed. The scheduler only materializes durable rows; it performs no network or device work.

Use a clock-driven loop with `tokio::sync::watch::Receiver<bool>` for shutdown, but expose `tick(now)` for deterministic tests. Startup always calls `tick()` before sleeping so missed wakeups are handled.
Use a lossy `Notify` only to wake early and a fixed one-second idle poll as backstop; receiving a wake never carries campaign data, and dropping all wakes cannot lose SQLite work.

- [ ] **Step 6: Implement planner-to-executor dispatch**

`InteractionDispatcher::dispatch_claim()` follows this order:

1. Load the immutable request and persisted cancellation flag; a cancelled unplanned campaign terminates before network resolution.
2. For every run-now or scheduled campaign, resolve every stored target through `InteractionTargetResolver` before actor/device acquisition. Use at most eight HTTP resolutions concurrently, restore target ordinal order before persistence, and retain Task 1A's per-target deadline. Persist the execution-time result without overwriting the original snapshot; `Changed` and `Unverified` make only that target's future assignments fail with typed `TargetChanged`/`TargetUnverified`.
3. Recheck persisted cancellation, then for an unplanned campaign snapshot actor availability/identity/proxy/capabilities at actual start, plan, and commit the plan transaction. Preserve request order for explicit accounts; sort `AllOnline` by `(device_udid, slot_key, account_id)` so snapshots are deterministic. Scheduled `AllOnline` selection is therefore resolved at execution time.
4. Group persisted assignments by UDID in ordinal order and submit one `DeviceBatchExecution` at a time per group to the injected Gate 0 `InteractionBatchExecutor`.
5. Let that executor acquire the shared `DeviceControlPlane` lease and atomically upgrade stream capacity; the dispatcher acquires no device lock or capacity permit.
6. Accept `WaitingCapacity`, `SkippedUnavailable`, and the transition to `Preparing` only through `InteractionProgress` calls made by the executor after the corresponding Gate 0 control-plane result. The dispatcher never synthesizes these states from timing or task count.
7. Execute through the progress contract, aggregate, and release the durable claim.

The dispatcher may hold task handles solely for shutdown/observation; it has no ownership map. SQLite reconstructs campaign work after restart, while Gate 0's control plane remains the sole authority for live ownership and stream capacity.
Every claim/plan/progress transaction commits before awaiting target HTTP, `InteractionBatchExecutor`, control-plane lease, stream transfer, driver call, frame evidence, or artifact file I/O.

- [ ] **Step 7: Write crash-state audit tests before implementing recovery**

Seed each relevant persisted crash point and restart a new dispatcher instance:

```rust
#[tokio::test]
async fn restart_after_issued_intent_marks_action_uncertain_and_does_not_replay_it() {
    let fixture = recovery_fixture(ActionStatus::Running, EffectIntent::Issued).await;
    let restarted = fixture.restart_dispatcher().await;
    restarted.recover_startup().await.unwrap();

    let action = fixture.store.get_action(&fixture.action_id).await.unwrap();
    assert_eq!(action.status, ActionStatus::Uncertain);
    assert_eq!(fixture.executor.call_count(&fixture.action_id), 0);
}
```

Cover queued, claimed-before-plan, planned/waiting, preparing with no intent, running with no intent, issued intent, terminal action before aggregate commit, cancellation pending, and stale schedule claim.

Add identity/opening-specific cases: an issued current identity action without a terminal identity result becomes `Uncertain/TargetIdentityAmbiguous` and is never replayed; a Running opening attempt becomes immutable `Interrupted` and no successor is appended at startup; an assignment that entered `Preparing` but whose current identity action still has `identity_copy_intent=None` becomes `Interrupted` with Copy Link call count zero after restart; prior Confirmed identity rows remain unchanged in all three cases.

- [ ] **Step 8: Implement startup crash reconciliation**

`recover_startup()` runs before normal dispatch:

- expired dispatch claims without any started action return to `queued`;
- persisted planned work waiting for capacity returns to `ready`;
- if any assignment in a campaign reached `Preparing` or later, freeze the whole unfinished campaign for explicit retry: every unfinished no-intent action/assignment becomes `Interrupted`, every unfinished issued-intent action becomes `Uncertain`, and no remaining `Pending` action is dispatched automatically;
- a Running opening attempt becomes `Interrupted`, consumes that durable opening attempt, and is never retried automatically; the two-attempt budget can advance only inside a live claimed batch before identity intent;
- issued identity-copy intent on the current action without a terminal result makes the assignment `Uncertain/TargetIdentityAmbiguous` and is never reissued;
- terminal actions missing parent aggregates are reduced transactionally;
- requested cancellation is finalized cooperatively;
- stale scheduler claims are re-evaluated against the fixed 60-second rule.

Publish an audit result with counts/campaign IDs. Recovery must be idempotent and must never convert uncertain work to success or automatically continue a partially completed campaign.

- [ ] **Step 9: Run dispatcher, recovery, and full core tests**

```powershell
cargo test -p riviu-core --test interaction_dispatcher -- --nocapture
cargo test -p riviu-core --test interaction_recovery -- --nocapture
cargo test -p riviu-core --lib -- --nocapture
```

Expected: Gate 0 determines device serialization/capacity, schedules obey fixed lateness, claims survive restart, opening retries obey their durable two-attempt budget, and every crash point reaches the specified durable state without replaying issued identity/effect intents.

- [ ] **Step 10: Commit dispatcher and recovery**

```powershell
git add crates/core/src/interaction/dispatcher.rs crates/core/src/interaction/executor.rs crates/core/src/interaction/links.rs crates/core/src/interaction/mod.rs crates/core/src/interaction/scheduler.rs crates/core/src/interaction/store.rs crates/core/tests/interaction_dispatcher.rs crates/core/tests/interaction_recovery.rs
git diff --cached --name-only
git commit -m "feat(core): add durable interaction dispatcher"
```

### Task 11: Expose a thin Tauri and TypeScript campaign API

**Files:**
- Create: `apps/desktop/src-tauri/src/interaction_commands.rs`
- Modify: `apps/desktop/src-tauri/src/state.rs`
- Modify: `apps/desktop/src-tauri/src/lib.rs`
- Modify: `crates/core/src/events.rs`
- Modify: `apps/desktop/src/types.ts`
- Modify: `apps/desktop/src/api.ts`
- Create: `apps/desktop/src/interactionApi.test.ts`

- [ ] **Step 1: Write failing Rust command-helper tests**

Keep command logic in plain async helpers so it is testable without a Tauri process:

Create `interaction_commands.rs` with the test module and add `mod interaction_commands;` to desktop `lib.rs` before implementing helpers; this guarantees the RED run compiles the new module.

```rust
#[tokio::test]
async fn start_returns_committed_revision_and_emits_exactly_once() {
    let fixture = command_fixture(DriverMode::Mock).await;
    let result = start_impl(&fixture.service, valid_start_request("req-1"))
        .await
        .unwrap();
    assert_eq!(result.value.status, CampaignStatus::Queued);
    let event = fixture.events.recv().await.unwrap();
    assert_eq!(event.summary.id, result.summary.id);
    assert_eq!(event.summary.revision, result.summary.revision);
    assert!(fixture.events.try_recv().is_err());
}

#[tokio::test]
async fn real_runtime_without_qualified_executor_fails_before_persisting() {
    let fixture = command_fixture(DriverMode::Pymobiledevice3).await;
    let error = start_impl(&fixture.service, valid_start_request("req-1"))
        .await
        .unwrap_err();
    assert_eq!(error.code, "interaction_execution_unavailable");
    assert_eq!(fixture.store.count_campaigns().await.unwrap(), 0);
}
```

Also assert same-ID retries return the same campaign/revision without a duplicate event, hash conflicts map to a stable command code, and all errors serialize without database internals.

```powershell
cargo test -p riviu-managers-phone interaction_commands -- --nocapture
```

Expected RED: unresolved `DesktopInteractionService` and command helper functions.

- [ ] **Step 2: Add the interaction service to desktop state**

Define `DesktopInteractionService` in `interaction_commands.rs`, then compose its resolver, `InteractionStore`, `ArtifactStore`, scheduler, and dispatcher during `AppState::bootstrap()`. Use the existing `db`, `events`, `DeviceControlPlane`, and `driver_mode`, with the managed root fixed to `artifacts_dir.join("interaction")` so reconciliation can never touch Nurture/job artifacts. Production gets `ExecutionDisabled` until the qualified action executor lands; mock mode may install the deterministic fake executor for desktop development.

```rust
pub struct AppState {
    // existing fields
    pub interactions: DesktopInteractionService,
}
```

Call `recover_startup()` before spawning the dispatch loop. Start one dispatcher task from `spawn_background_tasks`; it uses the durable store and must not race another process-local dispatcher owner. Give each process a UUID claim owner and renew/release claims through the store.

During bootstrap, run artifact reconciliation before retention accounting. Spawn one low-frequency retention task from the same state owner (first run after startup, then every 24 hours); it calls the two-phase artifact store and emits only redacted counts/errors. Do not delete files from a Tauri command or while a campaign is non-terminal.

- [ ] **Step 3: Add stable post-commit events**

Extend `AppEvent` in `crates/core/src/events.rs`:

```rust
InteractionUpdated {
    summary: CampaignSummary,
    changed_assignment_ids: Vec<String>,
},
```

The desktop service emits it only after a successful store commit and only when `summary.revision` is newer than the revision already emitted in this process. UI consumers must treat the summary as an invalidation hint and reload detail via paged queries. Do not include full campaign trees, artifacts, comments, or tokens in events.

- [ ] **Step 4: Add the command boundary**

Implement and register these Tauri commands:

```text
interaction_parse_links
interaction_preview
interaction_start
interaction_schedule
interaction_get
interaction_list
interaction_list_targets
interaction_list_assignments
interaction_get_assignment
interaction_list_action_runs
interaction_cancel
interaction_retry
interaction_get_defaults
interaction_save_defaults
interaction_list_accounts
```

`interaction_parse_links` calls the backend `TikTokLinkResolver` and returns every nonblank line with its typed outcome plus the content-ID-deduplicated valid targets. `interaction_preview`, `start`, and `schedule` all re-resolve `TikTokTargetInput` values through the same service; none trusts resolved metadata from TypeScript. Preview then plans against an actor availability snapshot but performs no inserts, leases, or actions. `start` and `schedule` accept a client-generated request ID. Read commands accept opaque cursor and optional limit, applying 50/200 defaults at the Rust boundary as well as in the repository.

Before persistence, `start` and any currently determinable explicit-actor schedule compute the provisional assignment count from a non-reserving availability snapshot and reject more than 10,000. The execution-time planner remains authoritative and rechecks after the actual snapshot, especially for scheduled `AllOnline`; preview never reserves devices or guarantees future eligibility.

Match the approved argument surface exactly: `interaction_parse_links(raw_text)`, `interaction_list(cursor, limit, status_filter)`, `interaction_list_assignments(campaign_id, cursor, limit, filters)`, `interaction_retry(campaign_id, assignment_ids, retry_request_id)`, and `interaction_list_accounts(udids)`. Tauri request structs use `#[serde(rename_all = "camelCase", deny_unknown_fields)]`; command helpers convert them to domain filters before calling the repository. `interaction_start` accepts only `RunNow`; `interaction_schedule` accepts only `Once` and rejects cross-mode requests before persistence.

`interaction_list_accounts(udids)` returns only the current default binding for each
requested device, including its enabled/disabled state. The storage and planner
types remain multi-slot-ready, but no public command exposes a non-default actor
until a later account-switch capability is implemented and qualified.

Return `InteractionCommandError { code, message, retryable }` as a serializable Tauri error. Map every resolver/validation/store/control-plane-disabled case explicitly; never fall back to `format!("{error:?}")`, SQLite text, filesystem paths, URLs, proxy fields, or token-bearing source errors.

`interaction_open_on_device` remains for the desktop workflow plan as manual navigation under Gate 0 ownership only. It opens the immutable target and returns a frame-generation witness, but it performs no Copy Link proof, creates no identity/opening attempt, and mutates no campaign state. This core plan does not ship a placeholder implementation.

- [ ] **Step 5: Write failing TypeScript invoke-contract tests**

Mock `@tauri-apps/api/core` and assert exact command names and payload casing:

```ts
it("lists assignment pages with an opaque cursor", async () => {
  vi.mocked(invoke).mockResolvedValue({ items: [], nextCursor: null });
  await listInteractionAssignments("campaign-1", { statuses: ["failed"] }, "cursor-1", 50);
  expect(invoke).toHaveBeenCalledWith("interaction_list_assignments", {
    campaignId: "campaign-1",
    filters: { statuses: ["failed"] },
    cursor: "cursor-1",
    limit: 50,
  });
});

it("starts with the caller request id intact", async () => {
  vi.mocked(invoke).mockResolvedValue(campaignSummaryFixture);
  await startInteractionCampaign(startFixture("request-1"));
  expect(invoke).toHaveBeenCalledWith("interaction_start", {
    request: expect.objectContaining({ requestId: "request-1" }),
  });
});

it("parses draft lines only through the backend", async () => {
  vi.mocked(invoke).mockResolvedValue(parseLinksFixture);
  await parseInteractionLinks("https://vt.tiktok.com/FIXTURE/\ninvalid");
  expect(invoke).toHaveBeenCalledWith("interaction_parse_links", {
    rawText: "https://vt.tiktok.com/FIXTURE/\ninvalid",
  });
});
```

Expected first run: imports/wrappers do not exist.

```powershell
npm --prefix apps/desktop test -- interactionApi.test.ts
```

Expected RED: missing interaction wrappers/DTO exports.

- [ ] **Step 6: Mirror DTOs and implement API wrappers**

Add TypeScript discriminated unions mirroring the Rust camelCase serde contract. Reuse one generic `Page<T>` and keep IDs as strings. Mirror `TargetUnverified` and `TargetIdentityAmbiguous` as distinct closed result codes, and include the backend-projected retry eligibility/current identity-attempt fields without recomputing them in TypeScript. Do not represent result codes or capability states as unbounded display strings; retain their code/detail split.

Implement typed wrappers in `api.ts`, including `null` for absent cursor and no client-side request hashing. Add an `onInteractionUpdated` listener that exposes `CampaignSummary` plus changed assignment IDs and no nested graph.

- [ ] **Step 7: Run desktop boundary tests and builds**

```powershell
cargo test -p riviu-managers-phone interaction_commands -- --nocapture
cargo check -p riviu-managers-phone
npm --prefix apps/desktop test -- interactionApi.test.ts
npm --prefix apps/desktop run build
```

Expected: command helpers enforce production disablement before persistence, duplicate calls/events are idempotent, invoke payloads match Rust names, and both desktop targets compile.

- [ ] **Step 8: Commit the desktop API boundary**

```powershell
git add apps/desktop/src-tauri/src/interaction_commands.rs apps/desktop/src-tauri/src/lib.rs apps/desktop/src-tauri/src/state.rs crates/core/src/events.rs apps/desktop/src/api.ts apps/desktop/src/types.ts apps/desktop/src/interactionApi.test.ts
git diff --cached --name-only
git commit -m "feat(desktop): expose interaction campaign api"
```

### Task 12: Prove end-to-end durability, document invariants, and stop at the qualified boundary

**Files:**
- Create: `crates/core/tests/interaction_campaign_e2e.rs`
- Modify: `AGENTS.md`

- [ ] **Step 1: Write the failing fake-executor acceptance test**

Drive the service through the same public methods used by Tauri:

```rust
#[tokio::test]
async fn run_now_campaign_survives_restart_and_finishes_from_verified_fake_outcomes() {
    let fixture = e2e_fixture().with_devices(["udid-a", "udid-b"]).await;
    let created = fixture.service.start(valid_request("req-e2e")).await.unwrap();
    fixture.dispatcher.tick_once().await.unwrap();
    fixture.crash_after_first_terminal_action().await;

    let restarted = fixture.restart().await;
    restarted.dispatcher.recover_startup().await.unwrap();
    restarted.dispatcher.drain_ready_once().await.unwrap();

    let detail = restarted.store.get_campaign(&created.summary.id).await.unwrap();
    assert_eq!(detail.status, CampaignStatus::Interrupted);
    assert_eq!(detail.result_code, Some(CampaignResultCode::ProcessLost));
    assert_eq!(restarted.executor.replayed_issued_effect_count(), 0);
}
```

This assertion intentionally proves there is no automatic continuation after partial progress. Add separate clean-run acceptance cases for `Succeeded`, `Partial`, `Failed`, `Cancelled`, and `Uncertain`, plus a scheduled run and an idempotent explicit retry. The retry cases cover both a `Partial` assignment with one positive action plus one retryable Failed effect and a `Partial` two-device campaign with one untouched `Succeeded` actor plus one selected failed actor. Assert one transaction appends identity attempt 2 Pending/None before effect attempt 2 only for selected work, the next batch cannot execute the effect until identity attempt 2 succeeds, the successful action/device is never dispatched again, and all attempt-1 rows remain byte-equivalent.

- [ ] **Step 2: Add load and restart acceptance tests**

Generate 500 targets and up to 10,000 assignments with a seeded planner. Page through every entity, cancel during capacity wait, restart with stale claims, and enforce the artifact quota. Assert:

- planner output digest is identical across runs with the same input/seed;
- no assignment/action is duplicated;
- Gate 0 reports one screen-changing owner per UDID;
- Gate 0's running producer count never exceeds its configured stream budget;
- pagination returns every row once;
- every terminal campaign has a deterministic aggregate/result code;
- recovery never replays an issued identity/effect intent or advances a Running Opening attempt.

Keep the 10,000-row test ignored in the normal suite only if measured runtime is excessive; if ignored, add it to the explicit pre-merge command below and document its measured duration in the test comment.

- [ ] **Step 3: Run formatting, lint, focused tests, and full workspace verification**

```powershell
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test -p riviu-core --test interaction_campaign_e2e -- --nocapture
cargo test -p riviu-core --test interaction_campaign_e2e max_plan_capacity -- --ignored --nocapture
cargo test --workspace -- --nocapture
npm --prefix apps/desktop test
npm --prefix apps/desktop run lint
npm --prefix apps/desktop run build
```

Expected: all commands pass. Gate 0 is present, but real-device `start` remains typed-disabled until the verified-action gates install a qualified Gate 0-backed executor; fake outcomes are test evidence only and never enable production capability flags.

- [ ] **Step 4: Inspect the final diff for scope and secrets**

```powershell
git status --short
git diff --check
git diff --name-only HEAD
rg -n "RIVIU_RTMMO_TOKEN|RIVIU_AGENT_TOKEN|X-RT-Token|X-Riviu-Token|a99f4bd9" crates/core/src/interaction crates/core/tests/interaction_* apps/desktop/src-tauri/src/interaction_commands.rs apps/desktop/src/interactionApi.test.ts
```

Expected: only files listed in this plan changed; `git diff --check` is empty; secret/fixture identifier scan returns no matches. Do not stage unrelated Project 2, WDA, docs, or frontend-page changes.

- [ ] **Step 5: Update the handoff invariants in `AGENTS.md`**

Record, with the actual test counts and no aspirational PASS claims:

- SQLite is canonical for campaign/dispatch state and every mutation uses the serialized blocking writer;
- request/retry idempotency and immutable planned snapshots;
- proxy is snapshotted from `device_meta.proxy_id` only;
- Gate 0 alone enforces one screen-changing owner per UDID and the shared producer budget; dispatcher owns neither;
- per-attempt identity intent plus effect intent before side effects, append-only retry identity rows, bounded durable Opening attempts, and no replay of uncertain/issued actions;
- pagination limits/cursors and artifact retention/quota;
- dispatcher recovery matrix and fixed schedule lateness;
- Gate 0 remains the active control plane, while real Interaction execution stays disabled until verified-action qualification is wired.

Label the checkpoint `G1` only after every Step 3 command passes, record the resulting commit IDs, name `docs/superpowers/plans/2026-07-29-tiktok-interaction-verified-actions.md` as the next plan, and include the reverse-order commit reverts. Do not copy fixture-executor results into a live qualification table.

Do not claim comment, save, repost, DM, or any real TikTok action is complete based on fake-executor tests.

- [ ] **Step 6: Commit the verified campaign core**

```powershell
git add crates/core/tests/interaction_campaign_e2e.rs
git add -p AGENTS.md
git diff --cached --name-only
git commit -m "test(core): verify interaction campaign durability"
```

- [ ] **Step 7: Capture final rollback and handoff evidence**

```powershell
git log --oneline --decorate -12
git status --short
git diff HEAD^ -- AGENTS.md
```

Expected: the task commits are independently revertible in reverse order; the worktree contains no changes beyond pre-existing unrelated files. Hand the branch to the verified-action plan with these stable contracts: `InteractionBatchExecutor`, `TikTokActionExecutor`, `InteractionProgress`, `InteractionCapabilities`, `InteractionTargetResolver`, `PreparedActionPayload`, `ActionOutcome`, and `EvidenceRef`.
