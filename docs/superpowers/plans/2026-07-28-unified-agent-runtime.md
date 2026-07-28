# Riviu Unified Agent Runtime Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the live-confirmed trusted text agent the single default iPhone agent in the desktop product, with OS-keyring credentials, verified artifacts, per-device readiness/repair, visible UI state, and reliable comment-session recovery.

**Architecture:** Resolve secrets and persisted settings once in the Tauri composition root, then pass a fully explicit `DriverConfig` into `riviu-ios-driver`. The driver owns a manifest-verified `RiviuAgent.ipa`, a per-UDID lifecycle lock, and cached `AgentStatus`; it never chooses stock WDA from missing environment variables. Existing fresh-session and session-before-stream behavior stays intact and is hardened around preflight, repair, and repeated text-channel failures.

**Tech Stack:** Rust 2021, Tokio, Tauri 2, React 19, TypeScript 6, SQLite/rusqlite, keyring 3 native backends, Python 3/pymobiledevice3, Vitest, NSIS.

---

## Scope And Baseline

This plan implements milestone 1, `Unified Agent Runtime`, from the approved design in
`docs/superpowers/specs/2026-07-28-riviu-unified-iphone-control-design.md`.
Milestones 2-6 will add the broader capability control plane, system controls,
apps/transfers, backup/diagnostics, and RSD support. MDM remains deferred.

The current working tree already contains the live-confirmed RT text path:

- RT control on device port 8906 and MJPEG on 9093.
- Protected requests using `X-RT-Token` supplied to the sidecar through process env.
- Fresh text session after TikTok is foregrounded.
- Whole-comment `/wda/keys` payload and frame-confirmed Send state.
- Recovery that replaces both the feed session and watcher handle.

Do not reset, re-clone, or replace those changes. Before every commit below, inspect
`git diff --cached --check` and stage only the named implementation files; leave
`docs/claude/chat-latest*` and any unrelated user changes untouched.

Hard invariants from `AGENTS.md` remain in force:

- Never enable `autoDismissAlerts`.
- Stock sessions are primed with `snapshotMaxDepth: 1`.
- Create/prime the session before starting MJPEG.
- Do not wrap WDA requests in an outer Tokio timeout.
- Recycle transport only for a classified transport failure.
- Stop stream before an agent restart and clear cached frames.
- Do not run the desktop app and live harness against the same UDID concurrently.

## Public Runtime Contract

The milestone exposes these product concepts:

```rust
AgentSettings { auto_repair }
AgentStatus {
    udid, state, artifact_id, artifact_version, bundle_id,
    protocol_version, features, installed_version, installed_build,
    auth_ready, mjpeg_ready, session_ready, message
}
```

The token never appears in these types, SQLite, command arguments, logs, traces, or
frontend state. A bundled manifest supplies artifact identity; protected-route,
MJPEG, and session probes supply runtime readiness.

### Task 1: Add The Versioned Riviu Agent Artifact

**Files:**
- Create: `crates/ios-driver/src/agent.rs`
- Modify: `crates/ios-driver/src/lib.rs`
- Modify: `crates/ios-driver/Cargo.toml`
- Modify: `Cargo.toml`
- Create: `sidecars/wda/agent-manifest.json`
- Rename: `sidecars/wda/idbagent.ipa` -> `sidecars/wda/RiviuAgent.ipa`
- Modify: `apps/desktop/src-tauri/tauri.conf.json`
- Modify: `apps/desktop/src-tauri/src/state.rs`

- [ ] **Step 1: Write failing manifest and checksum tests**

Add tests to `crates/ios-driver/src/agent.rs` before the implementation:

```rust
#[test]
fn bundled_manifest_matches_the_bundled_ipa() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../sidecars/wda");
    let artifact = AgentArtifact::load(root.join("agent-manifest.json"))
        .expect("load bundled agent");

    assert_eq!(artifact.manifest.bundle_id, "com.mrph.svc");
    assert_eq!(artifact.manifest.protocol_version, 1);
    assert!(artifact.manifest.features.iter().any(|f| f == "text"));
    artifact.verify_checksum().expect("matching SHA-256");
}

#[test]
fn checksum_mismatch_is_rejected_before_install() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../sidecars/wda");
    let mut artifact = AgentArtifact::load(root.join("agent-manifest.json")).unwrap();
    artifact.manifest.sha256 = "00".into();
    let error = artifact.verify_checksum().unwrap_err().to_string();
    assert!(error.contains("checksum"));
}
```

- [ ] **Step 2: Run the tests and confirm they fail for missing artifact types**

Run:

```powershell
cargo test -p riviu-ios-driver agent::tests -- --nocapture
```

Expected: compile failure because `AgentArtifact` and `agent.rs` are not implemented.

- [ ] **Step 3: Define the manifest and verified artifact types**

Add `sha2 = "0.10"` to workspace dependencies and consume it from
`crates/ios-driver/Cargo.toml`. Implement:

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentManifest {
    pub artifact_id: String,
    pub artifact_version: String,
    pub bundle_id: String,
    pub bundle_version: String,
    pub bundle_build: String,
    pub protocol_version: u32,
    pub ipa: String,
    pub sha256: String,
    pub control_port: u16,
    pub mjpeg_port: u16,
    pub logical_width: u32,
    pub logical_height: u32,
    pub features: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct AgentArtifact {
    pub manifest: AgentManifest,
    pub ipa_path: PathBuf,
}
```

`AgentArtifact::load` must reject an absolute `ipa`, `..` path components, empty
feature names, a protocol version other than `SUPPORTED_AGENT_PROTOCOL = 1`, and a
missing IPA. `verify_checksum` streams the IPA through `Sha256` and compares
case-insensitively without logging file contents. Add
`unsupported_protocol_is_rejected` to the test module.

- [ ] **Step 4: Rename the binary without modifying its signed contents**

Run after resolving both paths and confirming they remain under the workspace:

```powershell
$root = (Resolve-Path .).Path
$source = Join-Path $root 'sidecars\wda\idbagent.ipa'
$dest = Join-Path $root 'sidecars\wda\RiviuAgent.ipa'
if ((Test-Path -LiteralPath $source) -and -not (Test-Path -LiteralPath $dest)) {
  Move-Item -LiteralPath $source -Destination $dest
}
```

Create `sidecars/wda/agent-manifest.json`:

```json
{
  "artifactId": "riviu-agent-ios-legacy-20260728",
  "artifactVersion": "2026.07.28.1",
  "bundleId": "com.mrph.svc",
  "bundleVersion": "1.0",
  "bundleBuild": "1",
  "protocolVersion": 1,
  "ipa": "RiviuAgent.ipa",
  "sha256": "8a24847099495ff70b998522692c43f00dd16b90f698bda6953a73f5d33002ea",
  "controlPort": 8906,
  "mjpegPort": 9093,
  "logicalWidth": 375,
  "logicalHeight": 667,
  "features": ["stream", "tap", "swipe", "text", "clipboard", "pushMedia"]
}
```

- [ ] **Step 5: Package the manifest and renamed IPA**

Replace the `idbagent.ipa` resource with `agent-manifest.json` and
`RiviuAgent.ipa` in `tauri.conf.json`. Update
`state.rs::tauri_resources_map_to_clean_sidecar_layout` to assert both resources
and keep stock `Riviumanagersphone.ipa` only as a legacy rollback artifact.

- [ ] **Step 6: Run focused verification**

Run:

```powershell
cargo test -p riviu-ios-driver agent::tests -- --nocapture
cargo test -p riviu-managers-phone state::tests::tauri_resources_map_to_clean_sidecar_layout
Get-FileHash sidecars/wda/RiviuAgent.ipa -Algorithm SHA256
```

Expected: both Rust test groups pass and the printed hash equals the manifest.

- [ ] **Step 7: Commit the artifact contract**

```powershell
git add Cargo.toml crates/ios-driver/Cargo.toml crates/ios-driver/src/agent.rs crates/ios-driver/src/lib.rs sidecars/wda/agent-manifest.json sidecars/wda/RiviuAgent.ipa apps/desktop/src-tauri/tauri.conf.json apps/desktop/src-tauri/src/state.rs
git diff --cached --check
git commit -m "feat: add versioned Riviu agent artifact"
```

### Task 2: Put Agent Credentials In The Native OS Store

**Files:**
- Create: `crates/signing/src/credentials.rs`
- Modify: `crates/signing/src/lib.rs`
- Modify: `crates/signing/Cargo.toml`
- Modify: `crates/core/src/types.rs`
- Modify: `crates/core/src/db.rs`
- Modify: `apps/desktop/src-tauri/src/commands.rs`

- [ ] **Step 1: Write failing credential-store tests**

Use an in-memory `CredentialBackend` fixture and add:

```rust
#[test]
fn existing_agent_token_wins_over_legacy_environment_value() { /* assert stored */ }

#[test]
fn first_run_migrates_a_legacy_token_once() { /* assert saved */ }

#[test]
fn first_run_generates_a_256_bit_token_when_env_is_empty() { /* 64 hex chars */ }

#[test]
fn apple_id_and_agent_token_use_distinct_accounts() { /* no collision */ }

#[test]
fn backend_errors_are_not_reported_as_missing_credentials() { /* assert error */ }
```

Also add DB tests:

```rust
#[test]
fn agent_settings_round_trip_without_secret_fields() { /* autoRepair only */ }

#[test]
fn invalid_agent_settings_json_is_not_silently_defaulted() { /* assert error */ }
```

- [ ] **Step 2: Confirm the new tests fail**

```powershell
cargo test -p riviu-signing credentials -- --nocapture
cargo test -p riviu-core agent_settings -- --nocapture
```

Expected: compile failures for the missing credential/settings APIs.

- [ ] **Step 3: Enable real native keyring backends**

Replace the featureless `keyring = "3"` dependency with target-specific entries:

```toml
[target.'cfg(target_os = "windows")'.dependencies]
keyring = { version = "3", features = ["windows-native"] }

[target.'cfg(target_os = "macos")'.dependencies]
keyring = { version = "3", features = ["apple-native"] }

[target.'cfg(target_os = "linux")'.dependencies]
keyring = { version = "3", features = ["linux-native-sync-persistent"] }
```

Add `uuid = { workspace = true }` to `crates/signing/Cargo.toml`. The current
featureless dependency can select keyring's mock backend, so this change is part of
the acceptance criteria rather than optional cleanup.

- [ ] **Step 4: Implement an injectable credential store**

Create `credentials.rs` with this interface:

```rust
pub trait CredentialBackend: Send + Sync {
    fn get(&self, account: &str) -> anyhow::Result<Option<String>>;
    fn set(&self, account: &str, value: &str) -> anyhow::Result<()>;
    fn delete(&self, account: &str) -> anyhow::Result<()>;
}

#[derive(Clone)]
pub struct CredentialStore {
    backend: Arc<dyn CredentialBackend>,
}

impl CredentialStore {
    pub fn system() -> anyhow::Result<Self>;
    pub fn agent_token_or_create(&self, legacy_env: Option<&str>) -> anyhow::Result<String>;
    pub fn has_agent_token(&self) -> anyhow::Result<bool>;
}
```

Use service `riviu-managers-phone` and account `agent-auth-token`. Generate a
64-character lowercase hex token by concatenating two UUID v4 values without
hyphens. Existing keyring value wins; a nonblank legacy env value is imported only
when the keyring is empty; otherwise generate and persist. Do not add a token getter
to a Tauri command.

Refactor `SigningService` to accept the same store:

```rust
pub fn with_credentials(sidecar_dir: PathBuf, credentials: CredentialStore) -> Self;
```

Apple email/password keep their current accounts, but keyring errors now propagate
instead of being converted to empty strings.

- [ ] **Step 5: Persist only non-secret settings**

Add to `crates/core/src/types.rs`:

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentSettings {
    #[serde(default = "default_true")]
    pub auto_repair: bool,
}

fn default_true() -> bool { true }

impl Default for AgentSettings {
    fn default() -> Self { Self { auto_repair: true } }
}
```

Add strict DB methods using key `agent.settings.v1`:

```rust
pub fn get_agent_settings(&self) -> anyhow::Result<AgentSettings>;
pub fn save_agent_settings(&self, settings: &AgentSettings) -> anyhow::Result<()>;
```

Malformed JSON returns an error with context. Never use `unwrap_or_default` on a
stored value and never persist `token`, `hasToken`, or the legacy token env value.
Change `SigningService::apple_id_config` and the existing `get_apple_id` Tauri command
to return `Result`; keyring access failures must reach the UI instead of looking like
an empty Apple ID.

- [ ] **Step 6: Verify secrets and settings**

```powershell
cargo test -p riviu-signing -- --nocapture
cargo test -p riviu-core agent_settings -- --nocapture
rg -n "agent-auth-token|RIVIU_RTMMO_TOKEN|token" crates/core/src/db.rs crates/core/src/types.rs
```

Expected: tests pass; the final search finds no persisted agent secret field.

- [ ] **Step 7: Commit credential persistence**

```powershell
git add crates/signing/Cargo.toml crates/signing/src/credentials.rs crates/signing/src/lib.rs crates/core/src/types.rs crates/core/src/db.rs apps/desktop/src-tauri/src/commands.rs Cargo.lock
git diff --cached --check
git commit -m "feat: persist Riviu agent credentials in OS keyring"
```

### Task 3: Make Driver Selection Explicit At The Composition Root

**Files:**
- Create: `crates/ios-driver/src/config.rs`
- Modify: `crates/ios-driver/src/lib.rs`
- Modify: `crates/ios-driver/src/wda.rs`
- Modify: `crates/ios-driver/src/pmd.rs`
- Create: `apps/desktop/src-tauri/src/agent_runtime.rs`
- Modify: `apps/desktop/src-tauri/src/lib.rs`
- Modify: `apps/desktop/src-tauri/src/state.rs`
- Modify: `apps/desktop/src-tauri/src/bin/live_nurture_test.rs`
- Modify: `crates/ios-driver/examples/nurture_live.rs`

- [ ] **Step 1: Write failing explicit-config tests**

Add tests that prove:

```rust
#[test]
fn agent_token_debug_is_redacted() {
    let token = AgentToken::new("fixture-token").unwrap();
    assert_eq!(format!("{token:?}"), "AgentToken([REDACTED])");
}

#[test]
fn unified_config_uses_manifest_ports_bundle_and_ipa() { /* exact manifest fields */ }

#[test]
fn empty_agent_token_is_rejected() { /* constructor error */ }

#[test]
fn degraded_driver_keeps_unified_profile_instead_of_stock() { /* backend remains unified */ }
```

In `apps/desktop/src-tauri/src/agent_runtime.rs`, add resolver tests proving that a
new install resolves the bundled unified artifact, that the legacy token env is used
only as keyring input, and that `RIVIU_WDA_BACKEND=stock` does not switch the desktop
product to stock.

- [ ] **Step 2: Confirm config tests fail**

```powershell
cargo test -p riviu-ios-driver config -- --nocapture
cargo test -p riviu-managers-phone agent_runtime -- --nocapture
```

- [ ] **Step 3: Add redacted typed driver configuration**

Implement in `config.rs`:

```rust
#[derive(Clone)]
pub struct AgentToken(String);

#[derive(Clone)]
pub struct UnifiedAgentConfig {
    pub token: AgentToken,
    pub artifact: AgentArtifact,
    pub settings: AgentSettings,
}

#[derive(Clone)]
pub enum DriverTarget {
    Mock,
    Real(UnifiedAgentConfig),
    LegacyStock,
}

#[derive(Clone)]
pub struct DriverConfig {
    pub sidecar_root: PathBuf,
    pub state_dir: PathBuf,
    pub target: DriverTarget,
}
```

`LegacyStock` is available only to explicit harness/tests and is never selected by
desktop bootstrap. `AgentToken` implements custom redacted `Debug`, no `Serialize`,
and exposes its value only through a crate-private method used to set the child
process environment.

Change the entry point to:

```rust
pub async fn create_driver(config: DriverConfig) -> anyhow::Result<DriverBundle>;
```

- [ ] **Step 4: Remove hidden environment reads from the driver library**

Change `PmdIosDriver::probe` and `PmdIosDriver::degraded` to accept resolved config.
Delete driver-library reads of:

- `RIVIU_WDA_BACKEND`
- `RIVIU_RTMMO_TOKEN`
- `RIVIU_RTMMO_IPA`

Replace `WdaProfile::select` with constructors from `DriverTarget`. The unified
constructor gets bundle, ports, logical size, IPA path, and feature declarations from
the verified manifest. Sidecar child processes still receive the token through
`Command::env`; it never enters `args`.

- [ ] **Step 5: Resolve runtime configuration in Tauri bootstrap**

Implement `resolve_desktop_agent_runtime` in `agent_runtime.rs`:

```rust
pub struct ResolvedAgentRuntime {
    pub driver_config: DriverConfig,
    pub settings: AgentSettings,
    pub token_configured: bool,
}
```

At `AppState::bootstrap`, use this exact order:

1. Open SQLite.
2. Resolve the runtime sidecar root.
3. Construct `CredentialStore::system()`.
4. Read `AgentSettings`.
5. Import or generate the token in keyring.
6. Load and verify `sidecars/wda/agent-manifest.json`.
7. Build explicit unified `DriverConfig`.
8. Call `create_driver(config)`.
9. Construct `SigningService::with_credentials` for the legacy signer.

The desktop may degrade to an empty real-device list when Python is missing, but the
degraded driver retains the unified profile and reports bridge/agent errors. It never
constructs stock WDA as a fallback.

- [ ] **Step 6: Keep environment parsing only at binary boundaries**

Update `live_nurture_test.rs` and `examples/nurture_live.rs` to build
`DriverConfig` explicitly. Their boundary may read `RIVIU_RTMMO_TOKEN` and an optional
`RIVIU_RTMMO_IPA`, then construct `AgentToken` and `AgentArtifact`. This preserves
headless diagnostics without reintroducing environment selection inside the library.

- [ ] **Step 7: Run config and existing transport tests**

```powershell
cargo test -p riviu-ios-driver -- --nocapture
cargo test -p riviu-managers-phone agent_runtime -- --nocapture
rg -n "std::env::var.*RIVIU_(WDA_BACKEND|RTMMO_TOKEN|RTMMO_IPA)" crates/ios-driver/src
```

Expected: tests pass and the final search returns no matches.

- [ ] **Step 8: Commit explicit configuration**

```powershell
git add crates/ios-driver/src/config.rs crates/ios-driver/src/lib.rs crates/ios-driver/src/wda.rs crates/ios-driver/src/pmd.rs apps/desktop/src-tauri/src/agent_runtime.rs apps/desktop/src-tauri/src/lib.rs apps/desktop/src-tauri/src/state.rs apps/desktop/src-tauri/src/bin/live_nurture_test.rs crates/ios-driver/examples/nurture_live.rs
git diff --cached --check
git commit -m "refactor: resolve unified agent configuration explicitly"
```

### Task 4: Add Installed-App Metadata And Agent Status Types

**Files:**
- Modify: `sidecars/pymobiledevice3/riviu_pmd.py`
- Modify: `sidecars/pymobiledevice3/test_rtmmo_lifecycle.py`
- Modify: `crates/core/src/types.rs`
- Modify: `crates/core/src/driver.rs`
- Modify: `crates/ios-driver/src/agent.rs`
- Modify: `crates/ios-driver/src/mock.rs`

- [ ] **Step 1: Write failing sidecar inventory tests**

Mock `InstallationProxyService.get_apps` with an entry containing
`CFBundleShortVersionString`, `CFBundleVersion`, and `ApplicationType`. Extend the
`is-installed` test to expect:

```json
{
  "ok": true,
  "installed": true,
  "bundleId": "com.mrph.svc",
  "version": "1.0",
  "build": "1",
  "applicationType": "User"
}
```

Add a missing-app case with all metadata fields `null`.

- [ ] **Step 2: Run and observe the metadata test failure**

```powershell
python -m unittest sidecars.pymobiledevice3.test_rtmmo_lifecycle -v
```

- [ ] **Step 3: Return installation metadata from the sidecar**

Change `_run` in `cmd_is_installed` to return the matching app dictionary rather than
a boolean, then emit the exact fields above. Keep the command name for compatibility.
Do not enumerate all installed apps when a bundle filter is supplied.

- [ ] **Step 4: Define serializable agent status types**

Add to `crates/core/src/types.rs`:

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum AgentState {
    Unknown,
    Missing,
    RepairRequired,
    Starting,
    Ready,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentStatus {
    pub udid: String,
    pub state: AgentState,
    pub artifact_id: String,
    pub artifact_version: String,
    pub bundle_id: String,
    pub protocol_version: u32,
    pub features: Vec<String>,
    pub installed_version: Option<String>,
    pub installed_build: Option<String>,
    pub auth_ready: bool,
    pub mjpeg_ready: bool,
    pub session_ready: bool,
    pub message: Option<String>,
}
```

Extend `DeviceDriver` with:

```rust
fn agent_settings(&self) -> AgentSettings;
fn set_agent_settings(&self, settings: AgentSettings);
fn cached_agent_status(&self, udid: &str) -> AgentStatus;
async fn preflight_agent(&self, udid: &str) -> anyhow::Result<AgentStatus>;
async fn repair_agent(&self, udid: &str) -> anyhow::Result<AgentStatus>;
fn supports_text_comments(&self) -> bool;
```

The mock driver returns a deterministic `Ready` status and supports text comments.

- [ ] **Step 5: Add the pure install decision function**

In `agent.rs`, define:

```rust
pub enum AgentInstallDecision {
    Reuse,
    InstallMissing,
    RepairVersionMismatch,
    ReportRepairRequired,
}
```

`decide_install(manifest, installed, auto_repair)` returns `Reuse` only when bundle,
version, and build match. Add unit tests for missing, matching, mismatched, and
`auto_repair=false` cases. Runtime probes still decide whether a matching install is
actually ready.

- [ ] **Step 6: Verify types and sidecar contract**

```powershell
python -m unittest sidecars.pymobiledevice3.test_rtmmo_lifecycle -v
cargo test -p riviu-ios-driver agent::tests -- --nocapture
cargo test -p riviu-core --lib
```

- [ ] **Step 7: Commit inventory and status contracts**

```powershell
git add sidecars/pymobiledevice3/riviu_pmd.py sidecars/pymobiledevice3/test_rtmmo_lifecycle.py crates/core/src/types.rs crates/core/src/driver.rs crates/ios-driver/src/agent.rs crates/ios-driver/src/mock.rs
git diff --cached --check
git commit -m "feat: expose unified agent readiness contract"
```

### Task 5: Implement Per-Device Preflight, Repair, And Lifecycle Rollback

**Files:**
- Modify: `crates/ios-driver/src/pmd.rs`
- Modify: `crates/ios-driver/src/wda.rs`
- Modify: `crates/ios-driver/src/stream.rs`
- Modify: `sidecars/pymobiledevice3/riviu_pmd.py`
- Modify: `sidecars/pymobiledevice3/test_rtmmo_lifecycle.py`

- [ ] **Step 1: Add failing lifecycle tests**

Cover these state transitions before editing production code:

```text
missing + autoRepair       -> install -> protected auth -> MJPEG -> session -> Ready
matching + healthy         -> no reinstall -> Ready
matching + auth failure    -> repair once -> Ready or Error
matching + MJPEG failure   -> repair once -> Ready or Error
version mismatch + off     -> RepairRequired, no install
repair failure             -> Error, no stock fallback
fresh-session failure      -> stream cache cleared and ordinary UI stream restored
hard recycle               -> stream stops before relay/agent restart
ordinary unified session   -> supports_text_input is false
fresh unified session      -> supports_text_input is true
```

Use pure transition tests in Rust plus mocked subprocess/service tests in
`test_rtmmo_lifecycle.py`; do not use a physical device for the red phase.

- [ ] **Step 2: Confirm lifecycle tests fail**

```powershell
cargo test -p riviu-ios-driver lifecycle -- --nocapture
python -m unittest sidecars.pymobiledevice3.test_rtmmo_lifecycle -v
```

- [ ] **Step 3: Replace the bundle-exists cache with status-aware inspection**

In `PmdIosDriver`, replace `agent_checked: HashSet<String>` with:

```rust
agent_statuses: Arc<Mutex<HashMap<String, AgentStatus>>>,
agent_settings: Arc<RwLock<AgentSettings>>,
artifact: AgentArtifact,
```

Split the current install helper into these lock-scoped operations:

```rust
async fn inspect_agent_locked(&self, udid: &str) -> Result<InstalledAgentInfo>;
async fn install_bundled_agent_locked(&self, udid: &str) -> Result<()>;
async fn preflight_agent_locked(&self, udid: &str, owned: &mut DeviceOwned)
    -> Result<AgentStatus>;
async fn repair_agent_locked(&self, udid: &str, owned: &mut DeviceOwned)
    -> Result<AgentStatus>;
```

Every path validates the local checksum before installation. A matching installed
version is reused only after protected auth, MJPEG, and session readiness pass.
Update the cached status after every transition.

- [ ] **Step 4: Make repair use the established lifecycle order**

`repair_agent_locked` performs this exact sequence under the existing per-UDID lock:

1. Set `Starting` status.
2. Stop MJPEG and clear `StreamHub` frames.
3. Drop cached WDA sessions.
4. Stop the owned relay.
5. Uninstall only `manifest.bundle_id` when a mismatched/broken install exists.
6. Install the checksum-verified `RiviuAgent.ipa`.
7. Launch with `USE_PORT`, `MJPEG_SERVER_PORT`, and `FARM_KEY` in child env.
8. Prove auth through protected `GET /wda/locked`.
9. Create/attach the session.
10. Start MJPEG and wait for a nonempty frame.
11. Re-read installed metadata and publish `Ready`.

On failure, publish `Error` with a secret-free message and return the error. Do not
start stock WDA.

- [ ] **Step 5: Harden failure cleanup and session transitions**

Change `recycle_locked` to call `teardown_stream_locked` before relay teardown.
Wrap `fresh_text_session_locked` in an explicit transition result: on failure, clear
the half-created session/proxy, restore a session-before-stream UI channel on a
best-effort basis, leave the status `Error`, and return the original error with cleanup
context. All spawned commands keep `kill_on_drop(true)` and internal request deadlines.

An ordinary status-attached unified session is screen-control capable but must set
`PmdUiSession::supports_text_input=false`. Only a session returned by
`start_fresh_text_session` sets it true. This prevents a recovered ordinary session
from being mistaken for TikTok's armed text channel.

Keep these existing behaviors unchanged:

- RT readiness uses protected `/wda/locked`, not unauthenticated `/status`.
- Stock cached-session liveness never triggers a transport recycle.
- RT tap/swipe/key payloads retain their live-confirmed routes.

- [ ] **Step 6: Implement the new `DeviceDriver` methods**

`cached_agent_status` is read-only and cheap. `preflight_agent` and `repair_agent`
take the same UDID slot lock used by session, stream, install, and recycle. Both return
the final cached `AgentStatus`; `supports_text_comments` is true only for a unified
runtime configuration, never for `LegacyStock` or degraded stock. Per-device readiness
still comes from `preflight_agent` and is not inferred from this configuration flag.

- [ ] **Step 7: Run lifecycle regression tests**

```powershell
cargo test -p riviu-ios-driver -- --nocapture
python -m unittest sidecars.pymobiledevice3.test_rtmmo_lifecycle -v
```

Expected: all tests pass, including prior route/header/session-order tests.

- [ ] **Step 8: Commit lifecycle management**

```powershell
git add crates/ios-driver/src/pmd.rs crates/ios-driver/src/wda.rs crates/ios-driver/src/stream.rs sidecars/pymobiledevice3/riviu_pmd.py sidecars/pymobiledevice3/test_rtmmo_lifecycle.py
git diff --cached --check
git commit -m "feat: preflight and repair Riviu agent per device"
```

### Task 6: Expose Agent Runtime Commands Through Tauri

**Files:**
- Create: `apps/desktop/src-tauri/src/agent_commands.rs`
- Modify: `apps/desktop/src-tauri/src/lib.rs`
- Modify: `apps/desktop/src-tauri/src/state.rs`
- Modify: `apps/desktop/src-tauri/src/commands.rs`

- [ ] **Step 1: Write failing command-model tests**

Add serialization tests for a ready/error `AgentStatus` and command tests using the
mock driver:

```rust
#[tokio::test]
async fn repair_command_returns_the_verified_status() { /* Ready */ }

#[test]
fn runtime_view_never_serializes_a_token() { /* inspect JSON keys and values */ }

#[test]
fn saving_auto_repair_updates_db_and_live_driver_settings() { /* both equal */ }
```

- [ ] **Step 2: Confirm command tests fail**

```powershell
cargo test -p riviu-managers-phone agent_commands -- --nocapture
```

- [ ] **Step 3: Implement the command surface**

Create these Tauri commands:

```rust
agent_get_settings() -> AgentRuntimeView
agent_save_settings(settings: AgentSettings) -> AgentRuntimeView
agent_list_statuses(udids: Vec<String>) -> Vec<AgentStatus>
agent_preflight(udid: String) -> AgentStatus
agent_repair(udid: String) -> AgentStatus
agent_bulk_repair(udids: Vec<String>) -> Vec<AgentStatus>
```

`AgentRuntimeView` contains `settings`, `token_configured`, `activeArtifactId`, and
`activeArtifactVersion`. It contains no token. Bulk repair remains sequential on each
UDID from this command; concurrency across different devices remains the future fleet
scheduler's responsibility because this Mac is confirmed safe at only 1-2 streams.

Register all commands in `lib.rs`. `agent_save_settings` writes SQLite first, then
calls `driver.set_agent_settings` so the new auto-repair policy applies immediately.

- [ ] **Step 4: Retire stock signing from the primary flow**

Rename `AppState::wda_bundle` to `legacy_wda_bundle`. Keep `resign_wda` and Apple ID
commands callable for the rollback/debug path, but no product Agent button may call
them. Add a code comment stating that they are legacy stock tooling and do not provide
trusted TikTok text.

- [ ] **Step 5: Verify the Tauri contract**

```powershell
cargo test -p riviu-managers-phone agent_commands -- --nocapture
cargo test -p riviu-managers-phone state::tests -- --nocapture
```

- [ ] **Step 6: Commit the command surface**

```powershell
git add apps/desktop/src-tauri/src/agent_commands.rs apps/desktop/src-tauri/src/lib.rs apps/desktop/src-tauri/src/state.rs apps/desktop/src-tauri/src/commands.rs
git diff --cached --check
git commit -m "feat: expose Riviu agent status and repair commands"
```

### Task 7: Fail Fast On Missing Text Capability And Recover Repeated Non-Arming

**Files:**
- Modify: `crates/core/src/nurture/actions.rs`
- Modify: `crates/core/src/nurture/mod.rs`
- Modify: `crates/core/src/nurture/recovery.rs`
- Modify: `crates/core/src/driver.rs`
- Modify: `crates/ios-driver/src/mock.rs`
- Modify: `apps/desktop/src-tauri/src/nurture_commands.rs`

- [ ] **Step 1: Add failing nurture tests**

Add tests for these product rules:

```rust
#[tokio::test]
async fn comment_enabled_job_stops_before_feed_when_text_capability_is_missing() {}

#[tokio::test]
async fn comment_job_with_unready_agent_is_rejected_before_it_is_reported_started() {}

#[tokio::test]
async fn two_consecutive_text_not_armed_results_refresh_the_fresh_session() {}

#[tokio::test]
async fn successful_text_comment_resets_the_non_armed_streak() {}

#[tokio::test]
async fn text_not_sent_is_not_retried_because_delivery_is_ambiguous() {}
```

The mock driver records ordinary/fresh-session calls and stream restarts. Assert that
recovery replaces both the mutable feed session and `SessionHandle` watcher session.

- [ ] **Step 2: Run the focused tests and observe failure**

```powershell
cargo test -p riviu-core comment_enabled_job -- --nocapture
cargo test -p riviu-core text_not_armed -- --nocapture
```

- [ ] **Step 3: Add a text-capability preflight at session startup**

In `nurture_start`, load settings before spawning tasks. When `comment_prob > 0`, call
`driver.preflight_agent` for every requested UDID and require `AgentState::Ready`
before calling `start_many`. If any device is not ready, reject the command before any
device is reported started and include each failed UDID with its concise repair reason.

At the engine boundary, before mood creation or feed actions, also check
`driver.supports_text_comments()`. If false, stop the session with an actionable status
that instructs the desktop to run Agent Repair. This second check protects headless
callers that do not use the Tauri command. Do not enter the emoji path as an implicit
fallback. Keep emoji reaction code available only for an explicitly selected reaction
flow in a later milestone.

As defense in depth, change `do_comment` to return `TextChannelUnavailable` when its
session reports `supports_text_input=false`; do not call `do_emoji_comment` from that
branch.

- [ ] **Step 4: Refresh after repeated frame-confirmed non-arming**

Maintain `text_not_armed_streak` in `run_session`:

- Increment only for `CommentResult::TextNotArmed`.
- Reset on `TextSent` and when a fresh session is successfully installed.
- At two consecutive failures, synthesize a session-class error and run the existing
  fresh-session recovery path.
- Restart stream after the fresh session and replace both session references.
- Do not retry the same comment after `TextNotSent`, because the post may already have
  reached TikTok.

- [ ] **Step 5: Run nurture and recovery regression tests**

```powershell
cargo test -p riviu-core nurture -- --nocapture
cargo test -p riviu-core comment -- --nocapture
```

- [ ] **Step 6: Commit comment preflight and recovery**

```powershell
git add crates/core/src/nurture/actions.rs crates/core/src/nurture/mod.rs crates/core/src/nurture/recovery.rs crates/core/src/driver.rs crates/ios-driver/src/mock.rs apps/desktop/src-tauri/src/nurture_commands.rs
git diff --cached --check
git commit -m "fix: require and recover trusted text comments"
```

### Task 8: Replace The Desktop Agent Button And Add Readiness UI

**Files:**
- Modify: `apps/desktop/package.json`
- Modify: `apps/desktop/package-lock.json`
- Modify: `apps/desktop/src/types.ts`
- Modify: `apps/desktop/src/api.ts`
- Create: `apps/desktop/src/agentStatus.ts`
- Create: `apps/desktop/src/agentStatus.test.ts`
- Modify: `apps/desktop/src/components/SettingsPanel.tsx`
- Modify: `apps/desktop/src/App.tsx`
- Modify: `apps/desktop/src/index.css`

- [ ] **Step 1: Add Vitest and failing status-view tests**

Install the existing-package-manager-compatible test dependency:

```powershell
Set-Location apps/desktop
npm install --save-dev vitest
Set-Location ../..
```

Add `"test": "vitest run"` to package scripts. In `agentStatus.test.ts`, test that:

- `ready` maps to `San sang` and enables text comments.
- `repairRequired` maps to `Can sua Agent`.
- `error` preserves a concise message.
- a bulk repair summary counts ready/error devices without including UDIDs in toast
  headings.

Run `npm test` and confirm failure before implementing `agentStatus.ts`.

- [ ] **Step 2: Add frontend types and API wrappers**

Mirror Rust camelCase fields exactly:

```ts
export type AgentState =
  | "unknown"
  | "missing"
  | "repairRequired"
  | "starting"
  | "ready"
  | "error";

export interface AgentSettings { autoRepair: boolean }
export interface AgentStatus { /* every Rust AgentStatus field */ }
export interface AgentRuntimeView {
  settings: AgentSettings;
  tokenConfigured: boolean;
  activeArtifactId: string;
  activeArtifactVersion: string;
}
```

Add wrappers for all six `agent_*` Tauri commands. No API returns or accepts the
token.

- [ ] **Step 3: Build a quiet operational Agent section in Settings**

Replace the current primary stock-WDA instructions with:

- Active artifact and protocol version.
- Credential state as `Stored in OS credential store`.
- `Auto repair` checkbox.
- One compact row per connected device showing state, installed build, auth, MJPEG,
  and session readiness.
- `Check` and `Repair` commands with disabled/in-progress states.

Keep Apple ID signing in a separate `Legacy stock agent` section below the active
runtime. Do not present it as required for comment support. Use the existing panel,
table, chip, button, and 8px-radius styles; do not nest cards.

- [ ] **Step 4: Route the Control Center Agent command to unified repair**

In `App.tsx`, replace `bulkResignWda` with `agentBulkRepair`. Before submit, confirm
the exact selected/connected device count. Reload device and agent status after the
command, then report the number ready and the number needing attention.

Change the no-stream banner to direct the user to `Agent` repair. No primary UI path
calls `resign_wda` or installs `Riviumanagersphone.ipa`.

- [ ] **Step 5: Run frontend verification**

```powershell
Set-Location apps/desktop
npm test
npm run lint
npm run build
Set-Location ../..
```

Expected: Vitest, oxlint, TypeScript, and Vite all pass.

- [ ] **Step 6: Commit the readiness UI**

```powershell
git add apps/desktop/package.json apps/desktop/package-lock.json apps/desktop/src/types.ts apps/desktop/src/api.ts apps/desktop/src/agentStatus.ts apps/desktop/src/agentStatus.test.ts apps/desktop/src/components/SettingsPanel.tsx apps/desktop/src/App.tsx apps/desktop/src/index.css
git diff --cached --check
git commit -m "feat: manage the unified agent from desktop UI"
```

### Task 9: Verify, Live-Test, Package, And Install

**Files:**
- Modify: `AGENTS.md`
- Create: `docs/live-agent-runtime-2026-07-28.md`

- [ ] **Step 1: Update handoff documentation before final verification**

Document in `AGENTS.md`:

- Desktop always resolves the unified agent through DB + native keyring.
- Driver code does not read backend/token/IPA environment variables.
- The stock IPA and re-sign commands are legacy rollback/debug paths.
- Agent readiness requires checksum, installed metadata, protected auth, session, and
  MJPEG frame evidence.
- Agent repair and all session/stream transitions use the per-UDID lock.
- Repeated `TextNotArmed` refreshes the fresh session after two occurrences.
- Future milestones 2-6 remain pending; MDM remains phase 3/deferred.

Start `docs/live-agent-runtime-2026-07-28.md` with the exact commands below and fill
in actual pass counts, package path, installed executable path, and live JSONL path.

- [ ] **Step 2: Run format, Rust, Python, and frontend gates**

```powershell
cargo fmt --all -- --check
cargo test --workspace
python -m unittest sidecars.pymobiledevice3.test_device_discovery sidecars.pymobiledevice3.test_stream_frames sidecars.pymobiledevice3.test_rtmmo_lifecycle -v
Set-Location apps/desktop
npm test
npm run lint
npm run build
Set-Location ../..
```

Expected: every command exits 0. Record actual test counts in the live report.

- [ ] **Step 3: Build the headless live harness and desktop installer**

```powershell
cargo build -p riviu-managers-phone --bin live_nurture_test --release
Set-Location apps/desktop
npm run tauri:build -- --bundles nsis
Set-Location ../..
$installer = Get-ChildItem target\release\bundle\nsis\*.exe |
  Sort-Object LastWriteTime -Descending |
  Select-Object -First 1
if (-not $installer) { throw 'NSIS installer was not produced' }
$installer | Format-List FullName,Length,LastWriteTime
```

- [ ] **Step 4: Stop only the exact installed desktop executable before USB testing**

```powershell
$installedExe = Join-Path $env:LOCALAPPDATA 'Riviumanagersphone\riviu-managers-phone.exe'
Get-Process | Where-Object { $_.Path -eq $installedExe } | ForEach-Object {
  Stop-Process -Id $_.Id
  Wait-Process -Id $_.Id -ErrorAction SilentlyContinue
}
```

This avoids broad process killing and prevents desktop/harness USB contention.

- [ ] **Step 5: Run the live text-comment gate on the connected test iPhone**

```powershell
$deviceJson = python sidecars/pymobiledevice3/riviu_pmd.py list |
  Select-Object -Last 1 |
  ConvertFrom-Json
$udid = $deviceJson.devices |
  Where-Object { $_.connection -eq 'usb' } |
  Select-Object -First 1 -ExpandProperty udid
if (-not $udid) { throw 'No USB iPhone is connected' }

tidevice -u $udid kill notes.3u
tidevice -u $udid kill com.riviu.managersphone.agent.xctrunner
tidevice -u $udid launch com.ss.iphone.ugc.Ame

if ([string]::IsNullOrWhiteSpace($env:RIVIU_RTMMO_TOKEN)) {
  $env:RIVIU_RTMMO_TOKEN = ([guid]::NewGuid().ToString('N') + [guid]::NewGuid().ToString('N'))
}
$run = Join-Path $env:TEMP ("riviu-agent-live-" + (Get-Date -Format 'yyyyMMdd-HHmmss'))
New-Item -ItemType Directory -Path $run | Out-Null
$env:RIVIU_FRAME_DUMP = Join-Path $run 'frames'
$env:RIVIU_WDA_TRACE = Join-Path $run 'trace.jsonl'

.\target\release\live_nurture_test.exe `
  --udid $udid `
  --minutes 6 `
  --videos 4 `
  --like-prob 0 `
  --comment-prob 100 `
  --follow-prob 0 `
  --watch-min 4 `
  --watch-max 8 `
  --steady chatty `
  --jsonl (Join-Path $run 'summary.jsonl')

$harnessExit = $LASTEXITCODE
if ($harnessExit -ne 0) { throw "Live harness exited $harnessExit" }
$rows = Get-Content (Join-Path $run 'summary.jsonl') | ForEach-Object { $_ | ConvertFrom-Json }
$summary = $rows | Where-Object { $_.kind -eq 'run' } | Select-Object -First 1
if (-not $summary) { throw 'Live JSONL has no run summary' }
if ($summary.comments -lt 1) { throw 'Live gate posted no verified text comment' }
if ($summary.videos -lt 1) { throw 'Live gate processed no video' }
if ($summary.summary -match '^(failed|partial)') { throw "Unexpected summary: $($summary.summary)" }
```

Expected: at least one frame-confirmed text comment, no stock WDA selection, and no
more than one heavy recovery. Record the generated `$run` path without copying tokens
or request headers into the report.

- [ ] **Step 6: Install the NSIS package and verify packaged resources**

```powershell
Start-Process -FilePath $installer.FullName -ArgumentList '/S' -Wait
if (-not (Test-Path -LiteralPath $installedExe)) { throw 'Installed executable missing' }
$installRoot = Split-Path $installedExe
if (-not (Test-Path (Join-Path $installRoot 'sidecars\wda\RiviuAgent.ipa'))) {
  throw 'Packaged RiviuAgent.ipa missing'
}
if (-not (Test-Path (Join-Path $installRoot 'sidecars\wda\agent-manifest.json'))) {
  throw 'Packaged agent manifest missing'
}
Start-Process -FilePath $installedExe -WorkingDirectory $installRoot
```

Open Settings and verify the connected device reaches `Ready`; the Agent button must
repair/preflight `RiviuAgent.ipa` and the active runtime must show protocol 1.

- [ ] **Step 7: Final secret and stock-flow audit**

```powershell
rg -n "bulkResignWda|resign_wda" apps/desktop/src/App.tsx apps/desktop/src/components
rg -n "std::env::(var|var_os).*RIVIU_(WDA_BACKEND|RTMMO_IPA|RTMMO_TOKEN)" crates/ios-driver/src
@'
import json, os, pathlib, sqlite3
db = pathlib.Path(os.environ['APPDATA']) / 'riviu-managers-phone' / 'riviu.db'
with sqlite3.connect(db) as conn:
    rows = conn.execute('select key, value from settings').fetchall()
for key, value in rows:
    assert 'token' not in key.lower(), key
    if key == 'agent.settings.v1':
        assert set(json.loads(value)) <= {'autoRepair'}
'@ | python -
rg -n "X-RT-Token|agent-auth-token" "$env:APPDATA\riviu-managers-phone" "$env:LOCALAPPDATA\Riviumanagersphone" -g '*.json' -g '*.log' 2>$null
git status --short
```

Expected: no primary frontend stock-signing call, no driver-library env reads (the
child-process env name used by `Command::env` remains), no plaintext token in
app-owned files, and only intentional working-tree changes.

- [ ] **Step 8: Commit documentation and final integration fixes**

```powershell
git add AGENTS.md docs/live-agent-runtime-2026-07-28.md
git diff --cached --check
git commit -m "docs: record unified agent runtime verification"
```

## Milestone 1 Done Criteria

- A clean desktop launch uses `RiviuAgent.ipa` without manually setting backend/token
  environment variables.
- The token is created/imported in the native OS credential store and is absent from
  SQLite, argv, logs, traces, and frontend state.
- Local IPA checksum, installed bundle metadata, protected auth, session, and MJPEG
  frame are all represented in per-device status.
- Agent Check/Repair is available from Settings and the Control Center Agent button;
  stock re-sign is not part of the primary workflow.
- Comment-enabled nurture refuses an untrusted text channel and refreshes a fresh
  session after two consecutive frame-confirmed non-arming outcomes.
- Workspace tests pass, a live text comment passes on the connected iPhone, the NSIS
  package contains the manifest/artifact, and the rebuilt desktop app is installed.

After this milestone is accepted, write the next implementation plan for milestone 2,
`Capability Control Plane`, before adding Screen/Apps/Files/Backup commands.
