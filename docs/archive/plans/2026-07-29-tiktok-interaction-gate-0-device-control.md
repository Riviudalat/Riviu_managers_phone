# TikTok Interaction Gate 0 Device Control Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Establish one shared device owner, one producer-counted stream budget, install-only Interaction preflight, and typed fail-closed URL/clipboard/identity capabilities before any campaign can touch an iPhone.

**Architecture:** Keep pure coordination and capability types in `riviu-core`; expose bounded lifecycle primitives through `DeviceDriver`; implement those primitives with the existing per-UDID supervisor in `riviu-ios-driver`; inject the same coordinator and stream budget into desktop, Nurture, Script, Repair, and manual paths. A separate qualification registry binds driver behavior to an exact Agent/transport/iOS/TikTok/layout/geometry tuple and starts empty.

**Tech Stack:** Rust 2021, Tokio synchronization, async-trait, serde, reqwest, Tauri 2, Python 3.9+, pymobiledevice3 10.1.0, Pillow 11.3.0, WDA HTTP/MJPEG, Vitest.

---

## Preconditions And Stop Rules

- Read `AGENTS.md` sections 2, 3.7, 3.8, and 3.9 before every WDA lifecycle edit.
- Use design commit `10433fb` as the behavioral source.
- Keep `sidecars/wda/RiviuAgent.ipa`, `sidecars/wda/agent-manifest.json`, and `sidecars/wda/WebDriverAgent/**` byte-identical.
- Gate 0 does not enable a campaign command. The capability registry begins with no qualified production tuple.
- Gate 0 may qualify the URL, clipboard, geometry, and reference Copy Link contract, but it never qualifies the G2 Rust identity/Watch executor. `TargetIdentityCopyLink` and Watch remain production-disabled until G2 publishes its separate exact-tuple runtime gate.
- A `/status` or `/wda/locked` success alone never qualifies automation, URL opening, clipboard, identity, or geometry.
- Interaction-specific code must not call `preflight_agent()`, `repair_agent()`, `prove_agent_ready_locked()`, or `ensure_stream()` because those generic paths may create a session and stream.
- Use request-local reqwest deadlines. Do not wrap WDA HTTP calls in `tokio::time::timeout`.

## File Map

**Create**

- `crates/core/src/device_work.rs`: shared per-UDID ownership and typed Busy result.
- `crates/core/src/stream_budget.rs`: producer budget, background revocation/transfer, sampler state.
- `crates/core/src/device_control.rs`: composition boundary that couples work ownership to stream reservations.
- `crates/core/src/device_capabilities.rs`: qualification tuple and typed UI capabilities.
- `crates/ios-driver/src/interaction_runtime.rs`: concrete install-only inspect/repair and session-before-stream orchestration.
- `sidecars/wda/interaction-capabilities.json`: reviewed allowlist; initially empty.
- `sidecars/wda/interaction-capabilities.schema.json`: strict registry schema.
- `tools/interaction-gate0/probe.py`: Mac/device URL, clipboard, geometry, and lifecycle probe.
- `tools/interaction-gate0/test_probe.py`: fixture HTTP/MJPEG and report transaction tests.
- `docs/re/interaction-gate0/README.md`: evidence contract and current gate state.

**Modify**

- `crates/core/src/lib.rs`
- `crates/core/src/driver.rs`
- `crates/core/src/job_queue.rs`
- `crates/core/src/nurture/mod.rs`
- `crates/core/src/nurture/recovery.rs`
- `crates/core/src/types.rs`
- `crates/ios-driver/src/lib.rs`
- `crates/ios-driver/src/pmd.rs`
- `crates/ios-driver/src/wda.rs`
- `crates/ios-driver/src/mock.rs`
- `crates/ios-driver/src/stream.rs`
- `sidecars/pymobiledevice3/riviu_pmd.py`
- `apps/desktop/src-tauri/src/state.rs`
- `apps/desktop/src-tauri/src/commands.rs`
- `apps/desktop/src-tauri/src/agent_commands.rs`
- `apps/desktop/src-tauri/src/nurture_commands.rs`
- `apps/desktop/src-tauri/src/bin/live_nurture_test.rs`
- `apps/desktop/src/types.ts`
- `apps/desktop/src/components/DeviceTile.tsx`
- `AGENTS.md`

---

### Task 1: Add The Shared Per-UDID Work Coordinator

**Files:**
- Create: `crates/core/src/device_work.rs`
- Modify: `crates/core/src/lib.rs`

- [ ] **Step 1: Write failing ownership tests**

Cover every owner named by the design, non-blocking Busy behavior, FIFO wait for Script/Nurture where requested, token validation, release on cancellation, and independence across UDIDs.

```rust
#[tokio::test]
async fn interaction_excludes_every_screen_changing_owner() {
    let coordinator = DeviceWorkCoordinator::new();
    let lease = coordinator
        .try_acquire("iphone-a", DeviceWorkOwner::Interaction)
        .expect("interaction lease");

    for owner in [
        DeviceWorkOwner::Nurture,
        DeviceWorkOwner::Script,
        DeviceWorkOwner::Repair,
        DeviceWorkOwner::ManualControl,
        DeviceWorkOwner::GroupSync,
    ] {
        let busy = coordinator.try_acquire("iphone-a", owner).unwrap_err();
        assert_eq!(busy.current_owner, DeviceWorkOwner::Interaction);
    }
    assert!(coordinator.try_acquire("iphone-b", DeviceWorkOwner::ManualControl).is_ok());
    drop(lease);
}
```

- [ ] **Step 2: Run the focused test and confirm RED**

```powershell
cargo test -p riviu-core device_work -- --nocapture
```

Expected: module/type import failure.

- [ ] **Step 3: Implement typed ownership**

Use one coordinator instance, a per-UDID Tokio semaphore, and synchronous metadata that never remains locked across `.await`.

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DeviceWorkOwner {
    Nurture,
    Interaction,
    Script,
    Repair,
    ManualControl,
    GroupSync,
}

pub struct DeviceWorkLease {
    udid: String,
    owner: DeviceWorkOwner,
    token: Uuid,
    _permit: OwnedSemaphorePermit,
    state: Arc<CoordinatorState>,
}
```

`try_acquire` must return a typed `DeviceBusy { udid, requested_owner, current_owner }`; it must not queue Interaction actor batches. `acquire` is permitted only for existing workflows whose current behavior intentionally queues. Gate G3's `interaction_open_on_device` command uses `DeviceWorkOwner::ManualControl`, matching the approved API contract; do not add a second owner category for that command.

- [ ] **Step 4: Run focused tests and confirm GREEN**

- [ ] **Step 5: Commit**

```powershell
git add crates/core/src/device_work.rs crates/core/src/lib.rs
git commit -m "feat(core): add shared device work coordinator"
```

---

### Task 2: Build A Producer-Counted Stream Budget State Machine

**Files:**
- Create: `crates/core/src/stream_budget.rs`
- Modify: `crates/core/src/lib.rs`

- [ ] **Step 1: Write failing state-machine tests**

Test default `1`, hard maximum `2`, background rotation, 5-second turn timeout, 30-second failure backoff, foreground priority, token checking, and cleanup. The key test must start with a background producer consuming budget `1`.

```rust
#[tokio::test]
async fn foreground_retags_budget_one_background_without_double_producer() {
    let budget = StreamBudgetManager::new(1).unwrap();
    let bg = budget.reserve_background("tile-a").expect("background");
    budget.mark_running(bg.token()).unwrap();

    let transfer = budget
        .begin_foreground_transfer("tile-b", DeviceWorkOwner::Interaction)
        .expect("revocation decision");
    assert_eq!(budget.running_producer_count(), 1);
    assert_eq!(transfer.revoked_udid(), Some("tile-a"));

    let fg = budget.complete_transfer(transfer, StreamStopProof::confirmed()).unwrap();
    assert_eq!(budget.reserved_capacity(), 1);
    assert_eq!(budget.running_producer_count(), 0);
    budget.mark_running(fg.token()).unwrap();
    assert_eq!(budget.running_producer_count(), 1);
}
```

- [ ] **Step 2: Confirm RED**

```powershell
cargo test -p riviu-core stream_budget -- --nocapture
```

- [ ] **Step 3: Implement explicit transitions**

Use these persisted-in-memory states, not a bare semaphore:

```rust
enum ProducerState {
    BackgroundReserved,
    BackgroundRunning,
    Revoking,
    ForegroundReserved,
    ForegroundRunning,
    Stopping,
    FailedBackoff { until: Instant },
}
```

`begin_foreground_transfer` marks the victim `Revoking` atomically. The caller stops the child and advances `StreamHub` generation outside the state mutex. `complete_transfer` retags the same capacity only after a matching `StreamStopProof`. No start is legal in `Revoking`. A failed stop keeps the capacity occupied and fails closed.

- [ ] **Step 4: Add property tests for invariants**

For generated sequences of reserve/start/revoke/stop/release operations, assert:

```text
running_producer_count <= configured_limit <= 2
one running producer per UDID
one capacity token has exactly one owner
foreground start requires Background stop proof when transferred
stale token cannot mutate current state
```

- [ ] **Step 5: Confirm GREEN and commit**

```powershell
cargo test -p riviu-core stream_budget -- --nocapture
git add crates/core/src/stream_budget.rs crates/core/src/lib.rs
git commit -m "feat(core): add bounded stream producer manager"
```

---

### Task 3: Define Fail-Closed Device Capabilities And Lifecycle Boundaries

**Files:**
- Create: `crates/core/src/device_capabilities.rs`
- Modify: `crates/core/src/driver.rs`
- Modify: `crates/core/src/lib.rs`
- Modify: `crates/core/src/types.rs`

- [ ] **Step 1: Write serialization and default-deny tests**

Require full qualification equality and prove that changing one dimension removes support: artifact SHA, agent/protocol, driver adapter, transport, route contract, clipboard mode, iOS, TikTok version/build, detector/layout, bounds, scale, or orientation.

```rust
#[test]
fn capability_does_not_inherit_across_tiktok_builds() {
    let registry = fixture_registry();
    let mut actual = fixture_snapshot();
    assert!(registry.negotiate(&actual).ui.open_url.is_some());
    actual.tiktok.build = "changed".into();
    assert!(registry.negotiate(&actual).ui.open_url.is_none());
}
```

- [ ] **Step 2: Confirm RED**

```powershell
cargo test -p riviu-core device_capabilities -- --nocapture
```

- [ ] **Step 3: Add exact types**

```rust
pub enum ActiveTransport { LegacyUsbmuxTransport, RsdTransport, Mock }
pub enum ScreenOrientation { Portrait, PortraitUpsideDown, LandscapeLeft, LandscapeRight }
pub enum ClipboardAccessMode { TargetBackgroundSafe, AgentForegroundRequired }
pub enum RouteScope { Sessionless, Session }
pub enum RouteMethod { Get, Post }

pub struct ProtectedRouteContract {
    pub contract_id: String,
    pub method: RouteMethod,
    pub scope: RouteScope,
    pub path: String,
    pub auth_header_name: String,
    pub body_schema_id: String,
    pub request_timeout_ms: u32,
}

pub struct QualifiedGeometry {
    pub logical_width: f64,
    pub logical_height: f64,
    pub pixel_width: u32,
    pub pixel_height: u32,
    pub scale_x: f64,
    pub scale_y: f64,
    pub orientation: ScreenOrientation,
}

pub struct InstalledAgentIdentity {
    pub bundle_id: String,
    pub version: String,
    pub build: String,
    pub executable_name: String,
    pub signer_identity_sha256: String,
}

pub struct InstalledTargetIdentity {
    pub bundle_id: String,
    pub version: String,
    pub build: String,
}

pub struct DeviceCapabilitySnapshot {
    pub installed_agent: InstalledAgentIdentity,
    pub selected_artifact_sha256: String,
    pub agent_version: String,
    pub protocol_version: u32,
    pub driver_adapter_version: String,
    pub transport: ActiveTransport,
    pub product_type: String,
    pub ios_version: String,
    pub target_app: InstalledTargetIdentity,
    pub protected_auth_ready: bool,
    pub geometry: Option<QualifiedGeometry>,
}

pub struct DeviceControllerCapabilities {
    pub snapshot: DeviceCapabilitySnapshot,
    pub ui: UiCapabilities,
}

pub struct UiCapabilities {
    pub open_url: Option<OpenUrlCapability>,
    pub clipboard: Option<ClipboardCapability>,
    pub target_identity_copy_link: Option<TargetIdentityCapability>,
}

pub struct OpenUrlCapability {
    pub route: ProtectedRouteContract,
    pub target_bundle_id: String,
    pub live_report_sha256: String,
}

pub struct ClipboardCapability {
    pub mode: ClipboardAccessMode,
    pub set_route: ProtectedRouteContract,
    pub get_route: ProtectedRouteContract,
    pub maximum_decoded_bytes: u32,
    pub live_report_sha256: String,
}

pub struct TargetIdentityCapability {
    pub open_url_contract_id: String,
    pub clipboard_contract_id: String,
    pub share_detector_version: String,
    pub copy_link_detector_version: String,
    pub detector_set_sha256: String,
    pub layout_id: String,
    pub geometry: QualifiedGeometry,
    pub live_report_sha256: String,
}

pub struct AgentInstallProof {
    pub installed: InstalledAgentIdentity,
    pub artifact_sha256: String,
    pub protected_auth_ready: bool,
    pub session_created: bool,
    pub stream_started: bool,
}

pub struct StreamStopProof {
    pub old_generation: u64,
    pub new_generation: u64,
    pub child_stopped: bool,
}
```

The base qualification key contains the selected artifact SHA, installed Agent identity, agent/protocol/driver-adapter versions, transport, iOS range, installed TikTok version/build, every protected route contract ID and schema, detector/layout IDs, logical bounds, expected frame dimensions, measured scales, orientation, and live report SHA. Gate 0 accepts only `375x667` portrait until a later reviewed registry entry exists. Route structs contain header names but never token values. Every digest is lowercase 64-character SHA-256; every dimension/scale is finite and positive; `maximum_decoded_bytes` is exactly 65,536 and `open_url.request_timeout_ms` is exactly 10,000. Do not infer capabilities from Agent manifest features.
An install-only proof is valid only when protected auth is true and both lifecycle booleans are false; do not overload `AgentState::Ready`, which still means auth + session + MJPEG readiness for existing callers.

- [ ] **Step 4: Extend `DeviceDriver` with safe defaults**

Add defaults that return `UnsupportedCapability`; existing drivers must continue compiling while implementations are added task by task.

```rust
async fn inspect_interaction_device(&self, udid: &str)
    -> anyhow::Result<DeviceCapabilitySnapshot>;
async fn repair_agent_install_only(&self, udid: &str)
    -> anyhow::Result<AgentInstallProof>;
async fn stop_owned_stream(&self, udid: &str)
    -> anyhow::Result<StreamStopProof>;
async fn start_stream_after_session(&self, udid: &str)
    -> anyhow::Result<StreamStartProof>;
async fn start_interaction_session(
    &self,
    udid: &str,
    bundle_id: &str,
    kind: InteractionSessionKind,
) -> anyhow::Result<Box<dyn UiSession>>;
```

Add typed `UiSession` methods for `open_url`, `set_clipboard`, `get_clipboard`, and full `active_app_identity { bundle_id, pid }`. Clipboard reads enforce a caller-provided maximum of 64 KiB after base64 decode.

- [ ] **Step 5: Confirm GREEN and commit**

```powershell
cargo test -p riviu-core device_capabilities -- --nocapture
git add crates/core/src/device_capabilities.rs crates/core/src/driver.rs crates/core/src/lib.rs crates/core/src/types.rs
git commit -m "feat(core): define interaction device capabilities"
```

---

### Task 4: Add Non-Mutating Device Bridge Inspection

**Files:**
- Modify: `sidecars/pymobiledevice3/riviu_pmd.py`
- Modify: `crates/ios-driver/src/pmd.rs`
- Modify: `crates/ios-driver/src/mock.rs`

- [ ] **Step 1: Write failing Python and Rust tests**

Add a sidecar command test showing that `inspect-device-capabilities` only opens lockdown/installation metadata services and never launches, terminates, installs, creates a relay, creates a session, or starts MJPEG.

Expected JSON keys:

```json
{
  "ok": true,
  "udid": "FIXTURE",
  "productType": "iPhone10,1",
  "iosVersion": "16.7.15",
  "transport": "legacyUsbmuxTransport",
  "targetApp": {
    "bundleId": "com.ss.iphone.ugc.Ame",
    "version": "TARGET_VERSION",
    "build": "TARGET_BUILD"
  },
  "agentApp": {
    "bundleId": "AGENT_BUNDLE_ID",
    "version": "AGENT_BUNDLE_VERSION",
    "build": "AGENT_BUNDLE_BUILD",
    "executableName": "AGENT_EXECUTABLE",
    "signerIdentity": "AGENT_SIGNER_IDENTITY"
  }
}
```

- [ ] **Step 2: Confirm RED**

```powershell
python -m unittest sidecars.pymobiledevice3.test_rtmmo_lifecycle -v
cargo test -p riviu-ios-driver interaction_inspect -- --nocapture
```

- [ ] **Step 3: Implement metadata inspection**

Reuse one `InstallationProxyService.get_apps` call for TikTok and installed-Agent
metadata and the current lockdown/RSD selection for `ActiveTransport`. Verify the
selected IPA checksum before emitting a snapshot. The sidecar reports the UDID from
the connected provider, and Rust rejects a different UDID, a missing/null app, blank
identity fields, or unknown response fields. Hash the installed signer identity at
the Rust boundary. Legacy lockdown inspection must set `autopair=false`, so inspect
cannot create pairing state or a Trust prompt. Do not put the token in argv or output. This metadata-only command
always returns `protected_auth_ready=false` and `geometry=None`; those proofs come
from separate protected runtime operations. The manifest's `375x667` is not proof.
The Gate 0 `DeviceDriver` path selects legacy usbmux for the current iOS 16 fixture.
RSD is an explicit low-level endpoint primitive only; a later per-UDID transport
adapter must own and pass that endpoint before the control plane can select RSD.

- [ ] **Step 4: Implement mock snapshots and negative dimensions**

Mock fixtures must include one exact qualified iPhone-8-like profile and at least one unsupported geometry/newer-device profile.

- [ ] **Step 5: Confirm GREEN and commit**

```powershell
git add sidecars/pymobiledevice3/riviu_pmd.py sidecars/pymobiledevice3/test_rtmmo_lifecycle.py crates/ios-driver/src/pmd.rs crates/ios-driver/src/mock.rs
git commit -m "feat(driver): inspect interaction device metadata"
```

---

### Task 5: Split Install-Only Repair From Generic Readiness

**Files:**
- Create: `crates/ios-driver/src/interaction_runtime.rs`
- Modify: `crates/ios-driver/src/lib.rs`
- Modify: `crates/ios-driver/src/pmd.rs`
- Modify: `crates/ios-driver/src/mock.rs`

- [ ] **Step 1: Write a call-log test that forbids session and MJPEG**

```rust
#[tokio::test]
async fn install_only_repair_never_proves_session_or_stream() {
    let driver = fixture_driver_with_missing_agent();
    driver.repair_agent_install_only("fixture").await.unwrap();
    assert_eq!(
        driver.calls(),
        ["verify_artifact", "inspect", "install", "inspect", "launch_auth", "protected_health"]
    );
    assert_eq!(driver.session_creates(), 0);
    assert_eq!(driver.stream_starts(), 0);
}
```

Also prove auth/session/MJPEG faults do not cause uninstall/reinstall.

- [ ] **Step 2: Confirm RED**

```powershell
cargo test -p riviu-ios-driver install_only -- --nocapture
```

- [ ] **Step 3: Implement `repair_install_only_locked`**

Refactor only common checksum/metadata/install helpers. Keep generic `repair_agent_locked()` behavior unchanged for existing Agent Repair callers. The new path may install only for `InstallMissing` or `RepairVersionMismatch`, then must prove installed identity and `/wda/locked` auth without `POST /session` or MJPEG.

- [ ] **Step 4: Add secret and process-ownership assertions**

Verify the token remains environment-only and every terminated process matches the existing PID/command fingerprint rules.

- [ ] **Step 5: Confirm GREEN and commit**

```powershell
cargo test -p riviu-ios-driver install_only -- --nocapture
git add crates/ios-driver/src/interaction_runtime.rs crates/ios-driver/src/lib.rs crates/ios-driver/src/pmd.rs crates/ios-driver/src/mock.rs
git commit -m "feat(driver): add install-only interaction repair"
```

---

### Task 6: Add Explicit Stream Stop And Session-Before-Stream Primitives

**Files:**
- Modify: `crates/ios-driver/src/pmd.rs`
- Modify: `crates/ios-driver/src/stream.rs`
- Modify: `crates/ios-driver/src/mock.rs`
- Modify: `crates/ios-driver/src/interaction_runtime.rs`

- [ ] **Step 1: Write lifecycle order tests**

Cover ordinary and fresh-text modes. The ordinary-mode log is:

```text
stop old producer
clear/increment generation
foreground TikTok
create/attach approved session
reserve stream generation
start MJPEG
first decoded frame
```

Fresh-text keeps the live-confirmed RT-MMO order from `AGENTS.md` because bootstrap
foregrounds the Agent: `stop old producer -> clear/increment generation -> bootstrap
fresh Agent -> foreground TikTok -> POST /session new -> reserve stream generation ->
start MJPEG -> first decoded frame`. Do not move bootstrap after foreground TikTok.

The tests must fail if generic preflight, generic repair, window-size probing, or stream-before-session occurs.

- [ ] **Step 2: Confirm RED**

```powershell
cargo test -p riviu-ios-driver interaction_lifecycle -- --nocapture
```

- [ ] **Step 3: Implement explicit primitives**

`stop_owned_stream` must stop the exact child, wait boundedly, invalidate the old
driver-held session, call `StreamHub::clear`, and return
`{ old_generation, new_generation, child_stopped: true }`. Session invalidation is
required before the subsequent install-only check; that path rejects any live
producer/session instead of mutating it outside the control plane.
`start_stream_after_session` requires a driver-held session and a foreground
reservation token; it must not create or probe a session itself.
`child_stopped=true` is valid only after the owned child exits within the bounded
wait; a timed-out child remains owned and returns an unconfirmed proof so stream
capacity cannot be released. Ordinary mode requires the protected relay established
by install-only readiness to still be live before foregrounding TikTok; it must not
cold-launch the Agent afterward. Fresh-text on a non-unified profile fails closed.
Readiness status moves to session-pending after stop, stream-pending after session,
and `Ready` only after the first JPEG decodes.

Do not reuse `fresh_text_session_locked` error recovery as-is because it restores an ordinary session and stream. Add an Interaction variant whose failure cleanup tears down the failed transition without opening a replacement producer.

- [ ] **Step 4: Test stale reader rejection**

Extend existing `StreamHub` tests so bytes buffered by the old reader after stop cannot repopulate latest frame or broadcast into the new generation.

- [ ] **Step 5: Confirm GREEN and commit**

```powershell
cargo test -p riviu-ios-driver interaction_lifecycle -- --nocapture
cargo test -p riviu-ios-driver stream::tests -- --nocapture
git add crates/ios-driver/src/pmd.rs crates/ios-driver/src/stream.rs crates/ios-driver/src/mock.rs crates/ios-driver/src/interaction_runtime.rs
git commit -m "feat(driver): expose ordered interaction stream lifecycle"
```

---

### Task 7: Compose The Control Plane And Integrate Every Existing Workflow

**Files:**
- Create: `crates/core/src/device_control.rs`
- Modify: `crates/core/src/lib.rs`
- Modify: `crates/core/src/job_queue.rs`
- Modify: `crates/core/src/nurture/mod.rs`
- Modify: `crates/core/src/nurture/recovery.rs`
- Modify: `apps/desktop/src-tauri/src/state.rs`
- Modify: `apps/desktop/src-tauri/src/commands.rs`
- Modify: `apps/desktop/src-tauri/src/agent_commands.rs`
- Modify: `apps/desktop/src-tauri/src/nurture_commands.rs`
- Modify: `apps/desktop/src-tauri/src/bin/live_nurture_test.rs`

- [ ] **Step 1: Add failing cross-owner tests**

Required cases:

- Script owns device -> manual tap returns typed Busy without creating a session.
- Nurture owns device -> Repair returns Busy.
- Interaction owns device -> Group Sync skips that UDID and reports it.
- Manual owns device -> Interaction `try_acquire` returns `SkippedUnavailable` input.
- Different UDIDs run concurrently.
- Cancellation drops the lease and leaves no owner metadata.
- Cancelling a future that owns `UiWithStreamContext` queues cleanup while retaining the device lease and stream capacity until the exact producer stops and generation advances.
- A new owner remains Busy during queued cleanup and succeeds only after `StreamStopProof` is committed.

- [ ] **Step 2: Confirm RED**

```powershell
cargo test -p riviu-core shared_device_owner -- --nocapture
cargo test -p riviu-managers-phone shared_device_owner -- --nocapture
```

- [ ] **Step 3: Inject one control plane from `AppState::bootstrap`**

Create one `DeviceControlPlane` that owns the driver, `DeviceWorkCoordinator`, and `StreamBudgetManager`. Its API is the only high-level path that can combine device ownership with producer capacity:

```rust
pub struct DeviceControlPlane {
    driver: Arc<dyn DeviceDriver>,
    work: Arc<DeviceWorkCoordinator>,
    streams: Arc<StreamBudgetManager>,
    cleanup_tx: tokio::sync::mpsc::UnboundedSender<DeviceCleanupTicket>,
}

impl DeviceControlPlane {
    pub async fn try_acquire_exclusive(
        &self,
        udid: &str,
        owner: DeviceWorkOwner,
    ) -> Result<DeviceExclusiveContext, DeviceControlError>;

    pub async fn reserve_ui_capacity(
        &self,
        context: &DeviceExclusiveContext,
    ) -> Result<ForegroundStreamReservation, DeviceControlError>;

    pub async fn inspect_interaction_device(
        &self,
        context: &DeviceExclusiveContext,
    ) -> Result<DeviceCapabilitySnapshot, DeviceControlError>;

    pub async fn repair_agent_install_only(
        &self,
        context: &DeviceExclusiveContext,
    ) -> Result<AgentInstallProof, DeviceControlError>;

    pub async fn foreground_target_app(
        &self,
        context: &DeviceExclusiveContext,
        bundle_id: &str,
    ) -> Result<ForegroundAppProof, DeviceControlError>;

    pub async fn start_interaction_session(
        &self,
        context: DeviceExclusiveContext,
        bundle_id: &str,
        kind: InteractionSessionKind,
    ) -> Result<UiSessionContext, DeviceControlError>;

    pub async fn start_reserved_stream(
        &self,
        context: UiSessionContext,
        reservation: ForegroundStreamReservation,
    ) -> Result<UiWithStreamContext, DeviceControlError>;

    pub async fn close_ui_context(
        &self,
        context: UiWithStreamContext,
    ) -> Result<DeviceReleaseProof, DeviceControlError>;
}
```

`try_acquire_exclusive` parks this UDID's background producer before returning. Every subsequent method validates the same UDID, owner, and unforgeable lease token. `reserve_ui_capacity` performs the atomic background transfer. `start_interaction_session` consumes the exclusive context and returns a session-bearing context, so `start_reserved_stream` cannot be called without the approved session. It calls only `start_stream_after_session`; the type transition itself enforces session-before-MJPEG. Release stops the matching producer before releasing capacity, then makes the tile eligible for later sampling.

Implement cancellation-safe ownership explicitly. Each context stores its lease and reservation inside `Option`s. `close_ui_context` takes them, stops the exact producer, advances `StreamHub` generation, applies the matching `StreamStopProof`, releases capacity, and drops the device lease in that order. `UiWithStreamContext::drop` moves the same cleanup ticket to the control plane's dedicated cleanup worker; it never drops the lease/permit directly. While the ticket is queued or stopping, coordinator metadata remains `CleanupPending` under the original owner and new acquisition returns typed Busy. The worker is started with `DeviceControlPlane`, joined during orderly desktop/harness shutdown, and tested with cancellation at every lifecycle await. A closed cleanup channel is a fatal control-plane state that keeps ownership/capacity quarantined; it must not mark the producer stopped. Windows process-tree shutdown remains the final process-exit cleanup, not the in-process ownership algorithm.

Keep the driver's field private. Neither `DeviceExclusiveContext`, `UiSessionContext`, nor `UiWithStreamContext` exposes `Arc<dyn DeviceDriver>`; existing workflows and later Interaction executors call only control-plane methods that verify their context token. Add compile-fail/doc tests or module-privacy tests showing an unowned caller cannot inspect, repair, foreground an app, create a session, start a stream, or recover one through this high-level boundary.

Remove `JobQueue.device_locks`. `JobQueue::new`, `NurtureEngine::new`, manual commands, Agent commands, and the background sampler receive the same `Arc<DeviceControlPlane>`. Replace direct `ensure_stream` calls in Nurture startup/recovery with owned reserve/restart operations. The live harness constructs one control plane before any child process or session work.

- [ ] **Step 4: Guard desktop commands**

Acquire the correct owner before `prepare_device`, `device_tap`, `device_swipe`, `device_type_text`, `device_home`, `group_input`, Agent preflight/repair, Script, and Nurture. Return a stable Tauri error object/code such as `DeviceBusy` rather than only localized text.

- [ ] **Step 5: Confirm GREEN and commit**

```powershell
cargo test --workspace
git add crates/core/src/device_control.rs crates/core/src/lib.rs crates/core/src/job_queue.rs crates/core/src/nurture/mod.rs crates/core/src/nurture/recovery.rs apps/desktop/src-tauri/src/state.rs apps/desktop/src-tauri/src/commands.rs apps/desktop/src-tauri/src/agent_commands.rs apps/desktop/src-tauri/src/nurture_commands.rs apps/desktop/src-tauri/src/bin/live_nurture_test.rs
git commit -m "refactor(runtime): share device ownership across workflows"
```

---

### Task 8: Replace Eager Background Streams With The Budgeted Sampler

**Files:**
- Modify: `apps/desktop/src-tauri/src/state.rs`
- Modify: `apps/desktop/src-tauri/src/commands.rs`
- Modify: `crates/ios-driver/src/mock.rs`
- Modify: `apps/desktop/src/types.ts`
- Modify: `apps/desktop/src/components/DeviceTile.tsx`

- [ ] **Step 1: Write failing sampler tests with a fake clock/driver**

Prove startup and refresh do not call `ensure_stream` for every device, only one producer runs at budget `1`, a tile yields after one fresh frame or 5 seconds, a failure backs off 30 seconds, foreground demand preempts the sampler, and the poller never reopens a foreground-owned or parked stream.

- [ ] **Step 2: Confirm RED**

```powershell
cargo test -p riviu-managers-phone background_stream -- --nocapture
```

- [ ] **Step 3: Implement sampler and tile state**

Delete the eager `ensure_stream` loops from bootstrap, `refresh_devices`, and the existing 3-second poll. Drive starts/stops through `StreamBudgetManager` only. Expose tile state:

```ts
type TileStreamState = "live" | "sampling" | "parked" | "stale" | "error";
```

Keep the last frame while parked and show a compact state indicator; do not blank or stretch the tile.

- [ ] **Step 4: Make mock producers obey the same lifetime**

Replace the unconditional all-device mock stream loop with start/stop handles so mock mode tests producer accounting honestly.

- [ ] **Step 5: Confirm GREEN, frontend build, and commit**

```powershell
cargo test -p riviu-managers-phone background_stream -- --nocapture
npm --prefix apps/desktop test -- --run
npm --prefix apps/desktop run build
git add apps/desktop/src-tauri/src/state.rs apps/desktop/src-tauri/src/commands.rs crates/ios-driver/src/mock.rs apps/desktop/src/types.ts apps/desktop/src/components/DeviceTile.tsx
git commit -m "feat(desktop): budget background device streams"
```

---

### Task 9: Implement Profile-Specific Open URL, Clipboard, And App Identity

**Files:**
- Modify: `crates/ios-driver/src/wda.rs`
- Modify: `crates/ios-driver/src/pmd.rs`
- Modify: `crates/ios-driver/src/mock.rs`
- Modify: `crates/core/src/driver.rs`

- [ ] **Step 1: Write exact HTTP contract tests**

For each supported fixture profile, assert method, session scope, auth header, body, response parsing, and request-local timeout. Open URL body must be exact:

```json
{
  "url": "https://www.tiktok.com/@fixture/video/123",
  "bundleId": "com.ss.iphone.ugc.Ame",
  "idleTimeoutMs": 0
}
```

Clipboard routes use the profile's proven session scope and exact base64 schema. `activeAppInfo` must return both bundle ID and PID. Add negative tests for unknown route contracts, oversized clipboard values, invalid base64, wrong/missing auth, Safari fallback, and positive idle timeout.

- [ ] **Step 2: Confirm RED**

```powershell
cargo test -p riviu-ios-driver interaction_http_contract -- --nocapture
```

- [ ] **Step 3: Implement adapter tables, not backend conditionals at call sites**

Add route contracts to the negotiated capability entry and have `WdaClient` render the exact URL. Do not expose production RT-MMO support until the live probe has established its `/url` method/scope/body/auth behavior.

- [ ] **Step 4: Implement both clipboard modes as guarded transitions**

`TargetBackgroundSafe` never foregrounds Agent. `AgentForegroundRequired` must stop/clear stream, foreground Agent, verify stable Agent PID, perform clipboard operation, foreground TikTok, verify TikTok bundle/PID, create the profile-approved final session, then start a new stream generation and confirm a fresh frame. No session created before the final Agent/TikTok switch is trusted.

- [ ] **Step 5: Confirm GREEN and commit**

```powershell
cargo test -p riviu-ios-driver interaction_http_contract -- --nocapture
git add crates/core/src/driver.rs crates/ios-driver/src/wda.rs crates/ios-driver/src/pmd.rs crates/ios-driver/src/mock.rs
git commit -m "feat(driver): add qualified URL and clipboard adapters"
```

---

### Task 10: Load A Strict, Empty-By-Default Qualification Registry

**Files:**
- Create: `sidecars/wda/interaction-capabilities.json`
- Create: `sidecars/wda/interaction-capabilities.schema.json`
- Modify: `crates/ios-driver/src/lib.rs`
- Modify: `crates/ios-driver/src/interaction_runtime.rs`
- Modify: `crates/ios-driver/src/mock.rs`

- [ ] **Step 1: Write registry parser and negotiation tests**

Reject unknown keys, duplicate qualification keys, malformed SHA-256, overlapping ambiguous entries, unknown transport/clipboard mode/orientation, missing evidence hash, and dimensions outside positive finite bounds. Changing any runtime tuple field must disable support.

- [ ] **Step 2: Confirm RED**

```powershell
cargo test -p riviu-ios-driver interaction_capability_registry -- --nocapture
```

- [ ] **Step 3: Add an empty production registry**

```json
{
  "schemaVersion": 1,
  "driverAdapterVersion": "interaction-v1",
  "qualifications": []
}
```

Tests may load a fixture-only registry from memory. Never put `FIXTURE_ONLY` in the production file and call it qualified.

Use this complete in-memory fixture shape for parser and one-field-drift tests; the production file above remains empty:

```json
{
  "schemaVersion": 1,
  "driverAdapterVersion": "interaction-v1",
  "qualifications": [{
    "qualificationId": "fixture-g0",
    "environment": "FIXTURE_ONLY",
    "base": {
      "agentArtifactSha256": "0000000000000000000000000000000000000000000000000000000000000000",
      "agentBundleId": "com.fixture.agent",
      "agentBundleVersion": "fixture-bundle-version",
      "agentBundleBuild": "fixture-bundle-build",
      "agentExecutableName": "FixtureRunner",
      "agentSignerIdentitySha256": "3333333333333333333333333333333333333333333333333333333333333333",
      "agentVersion": "fixture-1",
      "protocolVersion": 1,
      "transport": "legacyUsbmuxTransport",
      "productType": "iPhone10,1",
      "iosMinInclusive": "16.7.15",
      "iosMaxInclusive": "16.7.15",
      "tiktokBundleId": "com.ss.iphone.ugc.Ame",
      "tiktokVersion": "fixture-version",
      "tiktokBuild": "fixture-build",
      "geometry": {
        "logicalWidth": 375.0,
        "logicalHeight": 667.0,
        "pixelWidth": 750,
        "pixelHeight": 1334,
        "scaleX": 2.0,
        "scaleY": 2.0,
        "orientation": "portrait"
      }
    },
    "openUrl": {
      "contractId": "fixture-open-url-v1",
      "method": "post",
      "scope": "sessionless",
      "path": "/fixture/url",
      "authHeaderName": "X-Fixture-Token",
      "bodySchemaId": "open-url-body-v1",
      "requestTimeoutMs": 10000,
      "targetBundleId": "com.ss.iphone.ugc.Ame"
    },
    "clipboard": {
      "contractId": "fixture-clipboard-v1",
      "mode": "targetBackgroundSafe",
      "setRoute": {
        "method": "post", "scope": "sessionless", "path": "/fixture/clipboard/set",
        "authHeaderName": "X-Fixture-Token", "bodySchemaId": "clipboard-set-base64-v1",
        "requestTimeoutMs": 10000
      },
      "getRoute": {
        "method": "post", "scope": "sessionless", "path": "/fixture/clipboard/get",
        "authHeaderName": "X-Fixture-Token", "bodySchemaId": "clipboard-get-base64-v1",
        "requestTimeoutMs": 10000
      },
      "maximumDecodedBytes": 65536
    },
    "targetIdentityCopyLink": {
      "openUrlContractId": "fixture-open-url-v1",
      "clipboardContractId": "fixture-clipboard-v1",
      "shareDetectorVersion": "share-v1",
      "copyLinkDetectorVersion": "copy-link-v1",
      "detectorSetSha256": "1111111111111111111111111111111111111111111111111111111111111111",
      "layoutId": "iphone8-portrait-v1"
    },
    "liveReportSha256": "2222222222222222222222222222222222222222222222222222222222222222"
  }]
}
```

The schema requires a bounded request-local timeout on every route and rejects tokens or arbitrary header values anywhere in the file. `targetIdentityCopyLink` must reference the sibling route contract IDs exactly; it cannot embed a second route or geometry definition. Runtime negotiation combines this entry with the single `base.geometry` value to construct `TargetIdentityCapability`.

- [ ] **Step 4: Expose negotiation without enabling commands**

`DriverBundle` may expose a capability inspector, but desktop Interaction commands still do not exist in Gate 0. Existing Nurture remains independent from the Interaction allowlist.

- [ ] **Step 5: Confirm GREEN and commit**

```powershell
cargo test -p riviu-ios-driver interaction_capability_registry -- --nocapture
git add sidecars/wda/interaction-capabilities.json sidecars/wda/interaction-capabilities.schema.json crates/ios-driver/src/lib.rs crates/ios-driver/src/interaction_runtime.rs crates/ios-driver/src/mock.rs
git commit -m "feat(driver): fail closed on interaction capabilities"
```

---

### Task 11: Build The Mac/Device Gate 0 Probe

**Files:**
- Create: `tools/interaction-gate0/probe.py`
- Create: `tools/interaction-gate0/test_probe.py`
- Create: `docs/re/interaction-gate0/README.md`

- [ ] **Step 1: Write fixture probe tests**

Use local fake control/MJPEG servers and generated JPEGs. Test auth `401/401/200`, exact URL bodies, direct/short/photo cases, first-frame ordering, clipboard byte limits, both clipboard modes, PID/bundle proofs, geometry/orientation mismatch, report rollback on atomic replace failure, cleanup, and redaction.

- [ ] **Step 2: Confirm RED**

```powershell
python -m unittest discover -s tools/interaction-gate0 -p test_probe.py -v
```

- [ ] **Step 3: Implement a fixed live matrix**

The probe accepts explicit `--udid`, `--ipa`, `--agent-manifest`, `--token-env`, TikTok bundle, direct video URL, photo URL, and short URL. It derives all installed/transport/app/geometry values from the device and artifacts. It must not accept CLI switches that lower sample counts or bypass identity.

Required sequence per case:

```text
verify artifact/manifest SHA
inspect installed Agent and TikTok metadata
prove old producer stopped and generation advanced
foreground TikTok -> session -> MJPEG first frame
open exact URL
prove TikTok foreground and fresh target frame
exercise selected clipboard mode with sentinel/readback
open Share -> Copy Link once -> read/resolve/compare ID and kind
prove session and stream remain healthy
stop reader/relay/processes and prove ports closed
```

The probe stores only redacted tuple data, request labels/statuses, hashes, timing, and selected outcome frames. Raw token, clipboard prior bytes, and unredacted UDID are forbidden.

This is a transport/geometry/reference-contract probe. Its Copy Link result does not
attest the later Rust executor implementation and cannot by itself map
`PlannedActionKind::TargetIdentityCopyLink` or Watch to `Ready`. Gate G2 must run
the production `FrameVerifiedTikTokActionExecutor` against the same exact tuple and
publish the separate runtime capability before desktop execution is enabled.

- [ ] **Step 4: Implement transactional report publication**

Write JSON and Markdown to temp files, run redaction checks, then atomically replace both or restore both previous files. Do not publish a capability entry automatically.

- [ ] **Step 5: Confirm fixture GREEN and commit**

```powershell
python -m unittest discover -s tools/interaction-gate0 -p test_probe.py -v
git add tools/interaction-gate0/probe.py tools/interaction-gate0/test_probe.py docs/re/interaction-gate0/README.md
git commit -m "test(interaction): add Gate 0 live qualification probe"
```

---

### Task 12: Run Gate 0 On Mac And Review The Qualification Entry

**Files:**
- Create after PASS: `docs/re/interaction-gate0/gate-0.json`
- Create after PASS: `docs/re/interaction-gate0/gate-0.md`
- Modify after review: `sidecars/wda/interaction-capabilities.json`
- Modify: `AGENTS.md`

- [ ] **Step 1: Verify the Mac environment without touching the device**

```bash
export PATH="$HOME/Library/Python/3.9/bin:$PATH"
: "${RIVIU_GATE0_UDID:?set the live device UDID}"
: "${RIVIU_GATE0_DIRECT_URL:?set the controlled direct-video URL}"
: "${RIVIU_GATE0_PHOTO_URL:?set the controlled photo URL}"
: "${RIVIU_GATE0_SHORT_URL:?set the controlled short URL}"
python3 -c 'import pymobiledevice3, PIL; print(pymobiledevice3.__version__, PIL.__version__)'
shasum -a 256 sidecars/wda/RiviuAgent.ipa sidecars/wda/agent-manifest.json
```

Expected production hashes remain those recorded in `AGENTS.md`.

- [ ] **Step 2: Eliminate competing XCTest/device owners**

Close the desktop/harness and 3uTools before running the probe. Kill only the exact known competing bundle/process using the existing safe commands. Keep the phone unlocked and TikTok signed into the fixture account.

- [ ] **Step 3: Run the fixed live probe**

```bash
RIVIU_RTMMO_TOKEN="$(security find-generic-password -s riviu-managers-phone -a agent-auth-token -w)" \
python3 tools/interaction-gate0/probe.py \
  --udid "$RIVIU_GATE0_UDID" \
  --ipa sidecars/wda/RiviuAgent.ipa \
  --agent-manifest sidecars/wda/agent-manifest.json \
  --token-env RIVIU_RTMMO_TOKEN \
  --tiktok-bundle com.ss.iphone.ugc.Ame \
  --direct-url "$RIVIU_GATE0_DIRECT_URL" \
  --photo-url "$RIVIU_GATE0_PHOTO_URL" \
  --short-url "$RIVIU_GATE0_SHORT_URL" \
  --report-dir docs/re/interaction-gate0
```

Expected: exact lifecycle ordering, all three targets identified by copied link, protected clipboard route proven in exactly one declared mode, portrait geometry proven, cleanup complete, redaction PASS.

- [ ] **Step 4: Stop on any tuple uncertainty**

Leave `qualifications` empty if the URL route contract, clipboard mode, TikTok build, transport, bounds/scale/orientation, Share/Copy Link detector, target identity, cleanup, or redaction is not proven. Record the typed failure in `AGENTS.md`; do not weaken the key.

- [ ] **Step 5: Review and add exactly one qualification entry after PASS**

The reviewed entry must be derived from `gate-0.json`, include its SHA-256, and match exact device/app/artifact/adapter data. Run parser tests again. This is the only step that can expose the G0 `open_url`, clipboard, and reference `target_identity_copy_link` contracts for that tuple. It does not add the G2 production-runtime capability and therefore does not enable a campaign executor.

- [ ] **Step 6: Run complete regression gates**

```bash
cargo fmt --all -- --check
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
npm --prefix apps/desktop test -- --run
npm --prefix apps/desktop run build
python3 -m unittest discover -s tools/interaction-gate0 -p test_probe.py -v
```

- [ ] **Step 7: Update `AGENTS.md` and commit evidence/allowlist**

Record the exact qualification tuple, commands, results, disabled dimensions, and rollback action. Stage only the reviewed registry, evidence, and `AGENTS.md` hunk.

```bash
git add sidecars/wda/interaction-capabilities.json \
  docs/re/interaction-gate0/gate-0.json \
  docs/re/interaction-gate0/gate-0.md
git add -p AGENTS.md
git commit -m "test(interaction): qualify Gate 0 device control tuple"
```

## Gate 0 Completion Criteria

Gate 0 is complete only when all of the following are true:

- all screen-changing desktop workflows use the same per-UDID owner;
- all real and mock MJPEG producers use the same budget;
- budget `1` transfer cannot deadlock or run two producers;
- generic preflight/repair behavior remains covered and Interaction uses install-only paths;
- session-before-stream is proven by call log and live trace;
- open URL has an exact protected route/body/session contract and no Safari fallback;
- clipboard mode has stable PID/bundle proof and 64 KiB enforcement;
- direct video, photo, and short URL identity succeed through Copy Link on the exact tuple;
- unsupported geometry/orientation/TikTok build/transport fail before a coordinate action;
- the production IPA/manifest hashes remain unchanged;
- full Rust/frontend/Python regressions pass;
- `AGENTS.md` names the Gate 0 commit and next plan.

Do not create or register `interaction_start` until these criteria pass.
