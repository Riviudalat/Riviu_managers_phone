use async_trait::async_trait;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::device_capabilities::{
    validate_clipboard_read_limit, AgentInstallProof, ClipboardAccessMode,
    DeviceCapabilitySnapshot, UiCapabilities,
};
use crate::flow::QualifiedElementLocator;
use crate::stream_budget::StreamStopProof;
use crate::types::{
    ActiveAppIdentity, AgentSettings, AgentStatus, AndroidInstallDeviceSpec, AppInstallResult,
    AppInstallStatus, DeviceAppInstallRequest, DeviceInfo, HardwareKey, InstalledApp,
    InteractionSessionKind, ShellOutcome, StreamHandoffProof, StreamStartProof, SwipeGesture,
    SwipePath, TapPoint,
};

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("capability {capability} is not supported by this driver")]
pub struct UnsupportedCapability {
    pub capability: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessAbsenceProof {
    pub bundle_id: String,
    pub old_pid: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppProcessState {
    pub bundle_id: String,
    pub pid: Option<u64>,
    pub running: bool,
}

/// What a media export found on the phone, and what of it actually landed.
///
/// Both numbers, because one of them alone is a sentence with no meaning. `pull_media`
/// used to return only the files that arrived, so a phone with five hundred photos of which
/// twenty copied reported "20" — the same answer it gives for a phone that only has twenty.
/// The operator has no way to tell those apart, and the second is the one where nothing is
/// wrong.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MediaPullReport {
    /// Files written on this host, verified to exist with a non-zero length.
    pub fetched: Vec<std::path::PathBuf>,
    /// How many media files the phone reported having.
    pub found: usize,
}

impl MediaPullReport {
    /// Found but did not arrive. Zero on a healthy export and on an empty gallery alike.
    pub fn missed(&self) -> usize {
        self.found.saturating_sub(self.fetched.len())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GuardedClipboardOperation {
    Set {
        content_type: String,
        bytes: Vec<u8>,
    },
    Get {
        maximum_decoded_bytes: usize,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GuardedClipboardOutput {
    Written,
    Value {
        content_type: String,
        bytes: Vec<u8>,
    },
}

#[derive(Debug, Clone, Default)]
pub struct GuardedClipboardProgress {
    state: Arc<Mutex<GuardedClipboardProgressState>>,
}

#[derive(Debug, Clone, Default)]
struct GuardedClipboardProgressState {
    stop: Option<StreamStopProof>,
    stream: Option<StreamStartProof>,
}

impl GuardedClipboardProgress {
    pub fn record_stop(&self, proof: StreamStopProof) {
        self.state.lock().stop = Some(proof);
    }

    pub fn record_stream(&self, proof: StreamStartProof) {
        self.state.lock().stream = Some(proof);
    }

    pub fn snapshot(&self) -> (Option<StreamStopProof>, Option<StreamStartProof>) {
        let state = self.state.lock();
        (state.stop, state.stream.clone())
    }
}

pub struct GuardedClipboardTransition {
    pub output: GuardedClipboardOutput,
    pub stop: Option<StreamStopProof>,
    pub agent: Option<ActiveAppIdentity>,
    pub target: ActiveAppIdentity,
    pub final_session: Option<Box<dyn UiSession>>,
    pub stream: Option<StreamStartProof>,
}

impl std::fmt::Debug for GuardedClipboardTransition {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("GuardedClipboardTransition")
            .field("output", &self.output)
            .field("stop", &self.stop)
            .field("agent", &self.agent)
            .field("target", &self.target)
            .field("has_final_session", &self.final_session.is_some())
            .field("stream", &self.stream)
            .finish()
    }
}

fn unsupported<T>(capability: &'static str) -> anyhow::Result<T> {
    Err(UnsupportedCapability { capability }.into())
}

/// Why a UI command failed. The nurture engine's recovery ladder turns on this
/// distinction: a rejected command means the runner is alive and only the
/// command was wrong, while a broken socket means the relay itself needs
/// attention. Collapsing both into "WDA unhealthy" is what produced the
/// 2–3 minute recycle spirals in earlier live tests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UiErrorKind {
    /// The socket never completed: connection refused/reset, relay wedged.
    /// The only class that justifies touching the transport.
    Transport,
    /// Accepted but no answer inside the deadline.
    Timeout,
    /// The agent says this session is gone — recreate the session, nothing else.
    Session,
    /// The agent answered with an error status. The runner is healthy.
    Http,
    /// Anything not produced by the UI transport.
    Other,
}

impl UiErrorKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            UiErrorKind::Transport => "transport",
            UiErrorKind::Timeout => "timeout",
            UiErrorKind::Session => "session",
            UiErrorKind::Http => "http",
            UiErrorKind::Other => "other",
        }
    }

    /// Does this class mean the command definitely did not reach the device?
    /// Retrying one of these is safe; retrying a timeout may double-apply it.
    pub fn is_safe_to_retry(&self) -> bool {
        matches!(self, UiErrorKind::Transport | UiErrorKind::Session)
    }
}

/// A classified UI transport failure.
#[derive(Debug, Clone)]
pub struct UiError {
    pub kind: UiErrorKind,
    /// The command that failed, e.g. `tap` or `actions.swipe`.
    pub op: String,
    pub message: String,
}

impl UiError {
    pub fn new(kind: UiErrorKind, op: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            kind,
            op: op.into(),
            message: message.into(),
        }
    }
}

impl std::fmt::Display for UiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} [{}]: {}", self.op, self.kind.as_str(), self.message)
    }
}

impl std::error::Error for UiError {}

/// Classify any error coming out of a [`UiSession`] call. Errors raised by the
/// driver carry a [`UiError`]; anything else is [`UiErrorKind::Other`].
pub fn ui_error_kind(err: &anyhow::Error) -> UiErrorKind {
    err.downcast_ref::<UiError>()
        .map(|e| e.kind)
        .unwrap_or(UiErrorKind::Other)
}

#[async_trait]
pub trait DeviceDriver: Send + Sync {
    fn agent_settings(&self) -> AgentSettings {
        AgentSettings::default()
    }
    fn set_agent_settings(&self, _settings: AgentSettings) {}
    fn cached_agent_status(&self, udid: &str) -> AgentStatus {
        AgentStatus::unknown(udid)
    }
    async fn preflight_agent(&self, udid: &str) -> anyhow::Result<AgentStatus> {
        Ok(self.cached_agent_status(udid))
    }
    async fn repair_agent(&self, _udid: &str) -> anyhow::Result<AgentStatus> {
        anyhow::bail!("agent repair is not supported by this driver")
    }
    async fn inspect_interaction_device(
        &self,
        _udid: &str,
    ) -> anyhow::Result<DeviceCapabilitySnapshot> {
        unsupported("inspectInteractionDevice")
    }
    async fn inspect_device_for_target(
        &self,
        _udid: &str,
        _target_bundle_id: &str,
    ) -> anyhow::Result<DeviceCapabilitySnapshot> {
        unsupported("inspectDeviceForTarget")
    }
    async fn set_negotiated_interaction_capabilities(
        &self,
        _udid: &str,
        capabilities: UiCapabilities,
    ) -> anyhow::Result<()> {
        if capabilities == UiCapabilities::default() {
            Ok(())
        } else {
            unsupported("setNegotiatedInteractionCapabilities")
        }
    }
    async fn repair_agent_install_only(&self, _udid: &str) -> anyhow::Result<AgentInstallProof> {
        unsupported("repairAgentInstallOnly")
    }
    async fn stop_owned_stream(&self, _udid: &str) -> anyhow::Result<StreamStopProof> {
        unsupported("stopOwnedStream")
    }
    /// Stop a bounded background producer while retaining the last decoded
    /// frame for the desktop tile. The generation still advances so buffered
    /// bytes from the parked producer cannot be published after the stop.
    async fn park_owned_stream(&self, udid: &str) -> anyhow::Result<StreamStopProof> {
        self.stop_owned_stream(udid).await
    }
    async fn start_stream_after_session(&self, _udid: &str) -> anyhow::Result<StreamStartProof> {
        unsupported("startStreamAfterSession")
    }
    /// Confirms that the per-device driver owns no MJPEG producer and records
    /// the current generation as the session handoff point. This must not stop
    /// or start a producer.
    async fn confirm_interaction_stream_stopped(
        &self,
        _udid: &str,
    ) -> anyhow::Result<StreamHandoffProof> {
        unsupported("confirmInteractionStreamStopped")
    }
    async fn read_active_app_bundle(&self, _udid: &str) -> anyhow::Result<String> {
        unsupported("readActiveAppBundle")
    }
    async fn start_interaction_session(
        &self,
        _udid: &str,
        _bundle_id: &str,
        _kind: InteractionSessionKind,
    ) -> anyhow::Result<Box<dyn UiSession>> {
        unsupported("startInteractionSession")
    }
    /// Performs the first durable Flow Launch effect and creates its session.
    /// Backends whose fresh-session bootstrap foregrounds the Agent override
    /// this method so the target application is foregrounded exactly once.
    async fn foreground_target_app_and_start_interaction_session(
        &self,
        udid: &str,
        bundle_id: &str,
        kind: InteractionSessionKind,
    ) -> anyhow::Result<Box<dyn UiSession>> {
        self.launch_app(udid, bundle_id).await?;
        self.start_interaction_session(udid, bundle_id, kind).await
    }
    #[allow(clippy::too_many_arguments)]
    async fn guarded_clipboard_transition(
        &self,
        _udid: &str,
        _agent_bundle_id: &str,
        _target_bundle_id: &str,
        _final_session_kind: InteractionSessionKind,
        _mode: ClipboardAccessMode,
        _operation: GuardedClipboardOperation,
        _progress: GuardedClipboardProgress,
    ) -> anyhow::Result<GuardedClipboardTransition> {
        unsupported("guardedClipboardTransition")
    }
    /// Whether this device has a trusted text channel.
    ///
    /// Takes a udid because a fleet can be mixed: with an iOS and an Android
    /// backend behind one multiplexer, a fleet-wide answer would report one
    /// platform's capability for the other's device.
    fn supports_text_comments(&self, _udid: &str) -> bool {
        false
    }
    /// Whether this device's sessions can report element geometry.
    ///
    /// A **pre-flight prediction**, answerable without opening a session, so callers
    /// can gate a picker or choose a strategy before touching the phone. The session's
    /// own [`UiSession::supports_element_bounds`] stays the runtime authority; if the
    /// two disagree, follow the session and record the mismatch rather than silently
    /// taking the other path.
    ///
    /// Named for the property the code actually depends on rather than for a platform:
    /// "android implies hierarchy" is an inference that can be wrong, while "this
    /// device reports bounds" is the thing being relied on.
    fn reports_element_bounds(&self, _udid: &str) -> bool {
        false
    }
    /// Which TikTok build this device can be driven against.
    ///
    /// The default is the iOS bundle — for a backend with one fixed app id that is a
    /// fact, not a guess. Android has two regional builds whose labels differ, so it
    /// must read the device (`crate::tiktok_target`).
    async fn resolve_tiktok_package(&self, _udid: &str) -> anyhow::Result<String> {
        Ok(crate::tiktok_target::IOS_TIKTOK_BUNDLE.to_string())
    }
    /// The `(package, versionName, locale)` triple a label lookup is keyed by.
    ///
    /// Read from the device rather than assumed, because the catalogue is keyed on all
    /// three and two of them move underneath the fleet: TikTok updates itself, and a
    /// resource id measured on one `versionName` is not the same control on the next. A
    /// caller that has only the package is answering a question about the *package* — which
    /// is the right shape for a refusal ("no set for this package is complete") and the
    /// wrong shape for a claim ("this phone is ready").
    ///
    /// The default refuses rather than inventing a version: a backend that cannot read the
    /// installed build must not hand back an empty string that `controls_for` would then
    /// treat as "unmeasured" — the two are different answers and only the device knows.
    async fn tiktok_build(&self, _udid: &str) -> anyhow::Result<(String, String, String)> {
        anyhow::bail!("backend này không đọc được (gói, versionName, locale) của TikTok")
    }
    /// Bytes available on the filesystem used to stage publish media.
    async fn available_storage_bytes(&self, _udid: &str) -> anyhow::Result<u64> {
        unsupported("availableStorageBytes")
    }
    fn supports_verified_app_termination(&self, _udid: &str) -> bool {
        false
    }
    async fn inspect_app_process(
        &self,
        _udid: &str,
        _bundle_id: &str,
    ) -> anyhow::Result<AppProcessState> {
        unsupported("inspectAppProcess")
    }
    /// Run one shell script on the device and return its combined output.
    ///
    /// An operator escape hatch, not an automation seam — nothing in this codebase may
    /// call it to get work done, because a string typed by a person is the one input no
    /// contract can describe. Defaulted to a refusal so a backend without a device shell
    /// says so instead of returning empty output that reads as a command that did nothing.
    async fn device_shell(&self, _udid: &str, _script: &str) -> anyhow::Result<ShellOutcome> {
        unsupported("deviceShell")
    }

    /// Turn the screen to `rotation` (0, 1, 2, 3 = 0°, 90°, 180°, 270°) and report what
    /// the device actually ended up at.
    ///
    /// Returns the rotation the device reports **after** the attempt, which is often not
    /// the one asked for: measured 14/08/2026 on both fleet phones, a portrait-locked
    /// foreground app wins over every mechanism tried, so a caller that assumed success
    /// would tell the operator the screen turned when it did not.
    async fn set_screen_rotation(&self, _udid: &str, _rotation: u8) -> anyhow::Result<u8> {
        unsupported("setScreenRotation")
    }

    /// Every app the phone reports as present, tagged user or system.
    ///
    /// Defaults to a refusal rather than an empty list, and the difference is the whole
    /// point: an empty `Vec` from a backend that cannot enumerate is indistinguishable
    /// from a phone with nothing on it, which is a lie the UI would render as fact.
    ///
    /// Deliberately **no** companion `supports_list_installed_apps`. That shape exists
    /// for pre-flight predictions that pick a strategy before touching a phone; here the
    /// call is already the cheapest possible probe and its failure is already typed.
    async fn list_installed_apps(&self, _udid: &str) -> anyhow::Result<Vec<InstalledApp>> {
        unsupported("listInstalledApps")
    }
    /// Full device backup into `dest`/<udid> via Mobilebackup2. Long-running.
    async fn backup_device(&self, _udid: &str, _dest: &Path) -> anyhow::Result<()> {
        unsupported("backupDevice")
    }
    /// Restore a Mobilebackup2 backup from `src` (the directory containing the
    /// per-device backup). Long-running; typically reboots the device.
    async fn restore_device(&self, _udid: &str, _src: &Path) -> anyhow::Result<()> {
        unsupported("restoreDevice")
    }
    async fn list_devices(&self) -> anyhow::Result<Vec<DeviceInfo>>;
    async fn refresh_device(&self, udid: &str) -> anyhow::Result<DeviceInfo>;
    async fn install_app(&self, udid: &str, path: &Path) -> anyhow::Result<()>;
    /// Install one Android package set. A one-member set is intentionally distinct from
    /// `install_app`: iOS continues to receive IPA files through that established path.
    async fn install_app_set(&self, _udid: &str, _paths: &[PathBuf]) -> anyhow::Result<()> {
        unsupported("installAppSet")
    }
    async fn android_install_device_spec(
        &self,
        _udid: &str,
    ) -> anyhow::Result<AndroidInstallDeviceSpec> {
        unsupported("androidInstallDeviceSpec")
    }
    /// Materialize one `.apks` selection for a normalized device spec. The
    /// caller groups equal specs, so this host-only Bundletool operation runs
    /// once per group rather than once per phone.
    async fn extract_app_container_for_spec(
        &self,
        _udid: &str,
        _path: &Path,
        _spec: &AndroidInstallDeviceSpec,
        _destination: &Path,
    ) -> anyhow::Result<Vec<PathBuf>> {
        unsupported("extractAppContainerForSpec")
    }
    /// Install a verified Android package set and classify the result at the
    /// process-spawn boundary. Android overrides this method with Package
    /// Manager readback; the default preserves compatibility for third-party
    /// drivers while refusing to describe an unverified failure as retryable.
    async fn install_app_set_checked(
        &self,
        udid: &str,
        request: &DeviceAppInstallRequest,
    ) -> anyhow::Result<AppInstallResult> {
        if let Some(gate) = &request.effect_gate {
            if !gate.claim_effect() {
                return Ok(AppInstallResult {
                    udid: udid.to_string(),
                    status: AppInstallStatus::CancelledBeforeDispatch,
                    effect_started: false,
                    observed_version_name: None,
                    observed_version_code: None,
                    detail: Some("install cancelled before process spawn".to_string()),
                });
            }
        }
        match self.install_app_set(udid, &request.apk_paths).await {
            Ok(()) => Ok(AppInstallResult {
                udid: udid.to_string(),
                status: AppInstallStatus::Uncertain,
                effect_started: true,
                observed_version_name: None,
                observed_version_code: None,
                detail: Some("driver did not provide package/version readback".to_string()),
            }),
            Err(error) => Ok(AppInstallResult {
                udid: udid.to_string(),
                status: AppInstallStatus::Uncertain,
                effect_started: true,
                observed_version_name: None,
                observed_version_code: None,
                detail: Some(error.to_string()),
            }),
        }
    }
    /// Install a platform-specific app container. Android uses this for `.apks`, where
    /// extracting device-compatible splits requires the connected device's specification.
    async fn install_app_container(&self, _udid: &str, _path: &Path) -> anyhow::Result<()> {
        unsupported("installAppContainer")
    }
    /// Stage a verified publish tree in the Agent sandbox. This is deliberately
    /// separate from `install_app`; media must never be sent through installd.
    async fn stage_publish_media(
        &self,
        _udid: &str,
        _agent_bundle_id: &str,
        _campaign_id: &str,
        _source_root: &Path,
    ) -> anyhow::Result<serde_json::Value> {
        unsupported("stagePublishMedia")
    }
    /// Whether the selected Agent advertises the native media prepare route.
    /// Staging remains available for older/production Agents, but callers must
    /// only invoke the protected route when this capability is present.
    fn supports_push_media(&self, _udid: &str) -> bool {
        false
    }
    /// Ask the Agent to validate the staged campaign and return its import
    /// proof. This is deliberately separate from TikTok posting.
    async fn prepare_publish_media(
        &self,
        _udid: &str,
        _campaign_id: &str,
        _manifest_sha256: &str,
    ) -> anyhow::Result<serde_json::Value> {
        unsupported("preparePublishMedia")
    }
    /// Import the validated campaign into the device photo library so TikTok's
    /// native photo composer can select the assets.
    async fn import_publish_media(
        &self,
        _udid: &str,
        _campaign_id: &str,
        _manifest_sha256: &str,
    ) -> anyhow::Result<serde_json::Value> {
        unsupported("importPublishMedia")
    }
    /// Remove only the photo assets created for a verified campaign.
    async fn cleanup_publish_media(
        &self,
        _udid: &str,
        _import_id: &str,
    ) -> anyhow::Result<serde_json::Value> {
        unsupported("cleanupPublishMedia")
    }
    /// Copy the phone's own photos and videos off it, into `dest_dir`.
    ///
    /// The other direction from the publish pipeline above, and deliberately not built on
    /// it: that path exists to put a *campaign* on a phone and knows about manifests and
    /// import ids. This one knows nothing about what it is fetching — it is the operator
    /// asking for whatever the camera roll currently holds.
    ///
    /// Returns what was found and what arrived, so a caller can report a count rather
    /// than a shrug -- and can tell a small gallery from a large one that mostly failed.
    /// An empty gallery is an `Ok` report with both numbers zero, not an error: nothing
    /// went wrong, there was simply nothing there.
    ///
    /// Defaults to a refusal that names itself rather than to an empty success, the same
    /// rule as every capability above — "no media" and "this backend cannot fetch media"
    /// must not look alike to a caller.
    async fn pull_media(&self, _udid: &str, _dest_dir: &Path) -> anyhow::Result<MediaPullReport> {
        unsupported("pullMedia")
    }
    async fn uninstall_app(&self, udid: &str, bundle_id: &str) -> anyhow::Result<()>;
    async fn screenshot(&self, udid: &str, dest: &Path) -> anyhow::Result<PathBuf>;
    async fn syslog_tail(&self, udid: &str, lines: usize) -> anyhow::Result<String>;
    async fn launch_app(&self, udid: &str, bundle_id: &str) -> anyhow::Result<()>;
    async fn terminate_app(
        &self,
        udid: &str,
        bundle_id: &str,
    ) -> anyhow::Result<ProcessAbsenceProof>;
    async fn reboot(&self, udid: &str) -> anyhow::Result<()>;
    /// Creates only a control session. Implementations must not start, stop, or
    /// replace an MJPEG producer; stream lifecycle belongs to DeviceControlPlane.
    async fn start_ui_session(&self, udid: &str) -> anyhow::Result<Box<dyn UiSession>>;
    /// Open a control session that can coexist with a live background stream.
    ///
    /// Default is [`Self::start_ui_session`]. iOS overrides this: a new
    /// `POST /session` under a live MJPEG wedges the runner, so a live
    /// producer must reuse the cached session or fail closed. Android's
    /// uiautomator2 session is independent of minicap, so the default is fine.
    async fn open_control_session(&self, udid: &str) -> anyhow::Result<Box<dyn UiSession>> {
        self.start_ui_session(udid).await
    }
    /// Whether comment-enabled jobs need the target app foregrounded before a
    /// newly-created trusted text session. Stock WDA keeps its existing order.
    fn requires_fresh_text_session(&self, _udid: &str) -> bool {
        false
    }
    /// Prepare a text-capable UI session for `bundle_id`. Backends without a
    /// special text lifecycle use the ordinary session path.
    async fn start_fresh_text_session(
        &self,
        udid: &str,
        _bundle_id: &str,
    ) -> anyhow::Result<Box<dyn UiSession>> {
        self.start_ui_session(udid).await
    }
    /// Is a usable UI session already cached for this device? Callers use it
    /// only to report honestly whether they reused an agent or started one.
    async fn ui_session_cached(&self, _udid: &str) -> bool {
        false
    }
    /// Drop cached WDA session so the next `start_ui_session` opens a fresh one.
    fn invalidate_ui_session(&self, _udid: &str) {}
    /// Hard-recycle USB relay + device WDA runner (wedged Agent recovery).
    async fn recycle_ui_transport(&self, _udid: &str) {}
    async fn ensure_stream(&self, udid: &str) -> anyhow::Result<String>;
    /// Legacy preparation hook. This must remain install/auth-only and must not
    /// create a control session or MJPEG producer.
    async fn prepare_device(&self, udid: &str) -> anyhow::Result<()>;
    /// Terminate and reap every host process spawned and still owned by this backend.
    ///
    /// DeviceControlPlane invokes this only after active contexts and its cleanup worker have
    /// drained. The default owns no persistent process. Implementations must target retained
    /// child handles, never a process name or a fleet-wide kill command.
    async fn shutdown_owned_processes(&self) -> anyhow::Result<()> {
        Ok(())
    }
}

/// One immutable accessibility-tree read, ordered within a live UI session.
///
/// This is intentionally separate from the cheap element locators. Public actions that need
/// ancestry/resource-id proof can pay for one full source read immediately before their effect;
/// ordinary feed observation must keep using [`UiSession::locate`]. A generation is assigned by
/// the session only after a source read succeeds, so `0` can never be mistaken for evidence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HierarchySourceSnapshot {
    pub generation: u64,
    pub xml: String,
}

#[async_trait]
pub trait UiSession: Send + Sync {
    async fn tap(&self, point: TapPoint) -> anyhow::Result<()>;
    /// Tap the way a finger does, through the UI hierarchy rather than by
    /// synthesising HID events.
    ///
    /// Text fields need this: a W3C `/actions` tap posts touch events that iOS
    /// does not turn into `becomeFirstResponder`, so tapping TikTok's comment
    /// box with it opens no keyboard and everything typed afterwards is lost.
    /// Slower than [`Self::tap`], so it is only for focus-sensitive targets.
    async fn tap_native(&self, point: TapPoint) -> anyhow::Result<()> {
        self.tap(point).await
    }
    async fn swipe(&self, gesture: SwipeGesture) -> anyhow::Result<()>;
    /// Draw a swipe as a **path**, if this backend can.
    ///
    /// The default collapses it to its two endpoints, so a backend that has no multi-point
    /// gesture API is not broken by callers that plan one — it simply gets the straight line
    /// it always got. Android overrides this because the W3C pointer protocol it speaks
    /// takes any number of moves with individual durations, which is what makes a curved,
    /// accelerating gesture free.
    ///
    /// Why a second method rather than a richer [`SwipeGesture`]: that type is persisted in
    /// flow scripts and carried in evidence, and every stored script would have to grow a
    /// path it does not have. See [`SwipePath`].
    async fn swipe_path(&self, path: SwipePath) -> anyhow::Result<()> {
        self.swipe(path.as_gesture()).await
    }
    /// [`Self::swipe_path`] with the points in stream/screenshot pixel space.
    ///
    /// The overlay measures a drag in the encoded frame it is painting, not in device
    /// pixels, so a path from the UI needs the same scaling [`Self::swipe_image`] does.
    /// Kept as its own method rather than making the caller scale, because the scale
    /// factor lives in the session (it knows the screen size) and a caller that guesses it
    /// produces a gesture that is subtly the wrong shape rather than an error.
    ///
    /// The default collapses to first-point -> last-point, so a backend that cannot draw a
    /// path keeps working and simply loses the curve.
    async fn swipe_path_image(
        &self,
        path: SwipePath,
        image_w: f64,
        image_h: f64,
    ) -> anyhow::Result<()> {
        let gesture = path.as_gesture();
        self.swipe_image(
            gesture.from,
            gesture.to,
            image_w,
            image_h,
            gesture.duration_ms,
        )
        .await
    }

    /// Tap using coordinates in stream/screenshot pixel space.
    async fn tap_image(&self, x: f64, y: f64, image_w: f64, image_h: f64) -> anyhow::Result<()> {
        let _ = (image_w, image_h);
        self.tap(TapPoint { x, y }).await
    }
    /// Swipe using coordinates in stream/screenshot pixel space.
    async fn swipe_image(
        &self,
        from: TapPoint,
        to: TapPoint,
        image_w: f64,
        image_h: f64,
        duration_ms: u64,
    ) -> anyhow::Result<()> {
        let _ = (image_w, image_h);
        self.swipe(SwipeGesture {
            from,
            to,
            duration_ms,
        })
        .await
    }
    async fn type_text(&self, text: &str) -> anyhow::Result<()>;
    /// Type **as real key events**, the way a keyboard would, appending at the cursor.
    ///
    /// [`Self::type_text`] writes through accessibility (`ACTION_SET_TEXT` on Android), which
    /// replaces the whole field and — measured 24/08/2026 — is invisible to the app's own
    /// input watchers: setting a comment box to `@name` never opened TikTok's mention picker,
    /// while the same characters injected as key events opened it and filtered it to real
    /// accounts. Anything that has to make the app *react to typing* needs this path.
    ///
    /// **ASCII only.** The Android implementation shells out to `input text`, which is killed
    /// outright by diacritics — the reason `type_text` exists in the first place. Callers put
    /// Vietnamese through `type_text` and use this only for things like an `@handle`.
    ///
    /// Unsupported by default: a session that cannot inject real keys must say so rather than
    /// silently fall back to the accessibility path, because the caller is asking for the one
    /// property that path does not have.
    async fn type_keys(&self, _text: &str) -> anyhow::Result<()> {
        unsupported("typeKeys")
    }
    /// Whether this session's text injection is accepted by the foreground
    /// app. Stock XCTest WDA reports successful key requests that TikTok drops;
    /// the standalone RT-MMO backend supplies the trusted text channel.
    fn supports_text_input(&self) -> bool {
        false
    }
    async fn home(&self) -> anyhow::Result<()>;
    /// Press a hardware-looking key from the desktop overlay.
    ///
    /// Home and Back delegate to the methods above. Everything else is
    /// unsupported unless a backend maps it — iOS has no volume HID here,
    /// and inventing one is the same class of bug as tapping uncalibrated
    /// coordinates.
    async fn press_hardware_key(&self, key: HardwareKey) -> anyhow::Result<()> {
        match key {
            HardwareKey::Home => self.home().await,
            HardwareKey::Back => self.back().await,
            HardwareKey::Recents
            | HardwareKey::VolumeUp
            | HardwareKey::VolumeDown
            | HardwareKey::Power
            | HardwareKey::Notification => unsupported("pressHardwareKey"),
        }
    }
    /// Lock (screen off) or unlock the device — xiaowei "锁屏 / 解锁", batched over a fleet.
    ///
    /// Default unsupported, so every mock and minimal backend inherits it untouched. iOS
    /// maps this to WDA `/wda/lock` and `/wda/unlock`. Android sleeps with `KEYCODE_SLEEP`
    /// and wakes with `KEYCODE_WAKEUP`, then best-effort dismisses a swipe-only keyguard; a
    /// phone with a secure PIN stays at its lock screen, which is the honest outcome rather
    /// than a pretended unlock.
    async fn set_locked(&self, _locked: bool) -> anyhow::Result<()> {
        unsupported("setLocked")
    }
    /// Go back one step, the way the platform's own back gesture would.
    ///
    /// Default unsupported because iOS has no system-wide back: there, leaving a
    /// sheet is an edge swipe or a close button at a calibrated position. Android
    /// has `KEYCODE_BACK`, and the comment flow needs it — pressing Home instead
    /// would close a drawer by leaving TikTok altogether.
    async fn back(&self) -> anyhow::Result<()> {
        unsupported("back")
    }
    async fn find_and_tap(&self, accessibility_id: &str) -> anyhow::Result<()>;
    async fn assert_visible(&self, accessibility_id: &str) -> anyhow::Result<()>;
    /// Dismiss a visible iOS system alert if present. Default: unsupported.
    async fn dismiss_alert(&self) -> anyhow::Result<()> {
        anyhow::bail!("dismiss_alert not supported")
    }
    /// Cheap liveness probe (WDA `/status`). Default: assume healthy.
    async fn healthy(&self) -> bool {
        true
    }
    /// Screen size in points. Default: unknown.
    async fn window_size(&self) -> anyhow::Result<(f64, f64)> {
        anyhow::bail!("window_size not supported")
    }
    /// Bring app to foreground (WDA). Default: unsupported.
    async fn launch_app_foreground(&self, _bundle_id: &str) -> anyhow::Result<()> {
        anyhow::bail!("launch_app_foreground not supported")
    }
    /// Stop the app and start it again, so it comes back in a fresh state.
    ///
    /// Distinct from [`Self::launch_app_foreground`], which only raises what is already
    /// running — measured 18/08/2026 on ce051715081fe20f03, whose TikTok would not leave
    /// one card: launching it repeatedly showed the same card every time, and a
    /// force-stop followed by a launch moved the feed on the first swipe afterwards.
    ///
    /// The default is that plain raise, because it is the most a backend without process
    /// control can offer and it is never *worse* than doing nothing. A backend that can
    /// stop a process should override this and say so.
    async fn restart_app(&self, bundle_id: &str) -> anyhow::Result<()> {
        self.launch_app_foreground(bundle_id).await
    }
    /// Bundle id of the frontmost app (`GET /wda/activeAppInfo`). Default: unsupported.
    async fn active_app_bundle(&self) -> anyhow::Result<String> {
        anyhow::bail!("active_app_bundle not supported")
    }
    async fn open_url(&self, _url: &str) -> anyhow::Result<()> {
        unsupported("openUrl")
    }
    /// Open a URL, requiring it to be handled by **one named app**.
    ///
    /// Exists because [`Self::open_url`] is not enough on Android, and that was measured
    /// rather than reasoned about. A bare `VIEW` intent for `https://www.tiktok.com/…`
    /// resolves to `com.android.intentresolver.ResolverActivity` — the app chooser —
    /// because TikTok and Chrome both claim the domain. Naming the package resolves it
    /// to TikTok's own `AppLinkHandlerV2` instead:
    ///
    /// ```text
    /// resolve-activity -a VIEW -d <url>                    -> ResolverActivity
    /// resolve-activity -a VIEW -c BROWSABLE -d <url> <pkg>  -> AppLinkHandlerV2
    /// ```
    ///
    /// A campaign that opens a link into a browser and then types would post nothing, or
    /// post somewhere unintended; the arrival check catches that
    /// (`interaction_hierarchy::ArrivalRefusal::WrongApp`), but not resolving to a chooser
    /// in the first place is better than detecting it afterwards.
    ///
    /// The default delegates, which is correct for a backend where the URL scheme has one
    /// possible handler.
    async fn open_url_in_app(&self, url: &str, _bundle_id: &str) -> anyhow::Result<()> {
        self.open_url(url).await
    }
    async fn set_clipboard(&self, _content_type: &str, _bytes: &[u8]) -> anyhow::Result<()> {
        unsupported("setClipboard")
    }
    async fn get_clipboard(
        &self,
        maximum_decoded_bytes: usize,
    ) -> anyhow::Result<(String, Vec<u8>)> {
        validate_clipboard_read_limit(maximum_decoded_bytes)?;
        unsupported("getClipboard")
    }
    async fn active_app_identity(&self) -> anyhow::Result<ActiveAppIdentity> {
        unsupported("activeAppIdentity")
    }
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
    /// Where a control actually is on screen, and what state it is in.
    ///
    /// This is the one primitive the hierarchy-driven nurture loop needs and
    /// `read_text` cannot give: not the label's text but its *rectangle*, so the
    /// existing touch-jitter planner can pick a human-looking point inside the
    /// real control instead of multiplying a calibrated fraction. Default is
    /// unsupported, because iOS cannot answer it — `snapshotMaxDepth` is pinned
    /// at 1 there (AGENTS.md 2.3), which is exactly why that backend locates
    /// controls by pixels.
    async fn locate(&self, _query: ElementQuery<'_>) -> anyhow::Result<Option<ElementBox>> {
        unsupported("locate")
    }
    /// Locate one element together with optional boolean state exposed by its hierarchy node.
    ///
    /// Existing callers deliberately stay on [`Self::locate`], so adding Save state does not add
    /// two attribute round trips to Like, Comment, Follow, or list scans. Backends without such
    /// attributes preserve the geometry and report both states as unknown.
    async fn locate_stateful(
        &self,
        query: ElementQuery<'_>,
    ) -> anyhow::Result<Option<StatefulElementBox>> {
        Ok(self.locate(query).await?.map(|element| StatefulElementBox {
            element,
            checked: None,
            selected: None,
        }))
    }
    /// **Every** element matching the query, not just the first.
    ///
    /// Exists because some controls are deliberately not unique: a comment drawer has
    /// one Reply button per row, and choosing among them is a geometric question, not
    /// a matching one. Picking the first would post a reply under whichever comment
    /// happened to be highest.
    ///
    /// Implementations should skip the per-element attribute reads that
    /// [`Self::locate`] performs unless they are cheap — this can be called against a
    /// list of dozens of rows, and `locate` already costs several round trips each.
    async fn locate_all(&self, _query: ElementQuery<'_>) -> anyhow::Result<Vec<ElementBox>> {
        unsupported("locateAll")
    }
    /// [`Self::locate_all`], but with each element's label actually read.
    ///
    /// A separate method because it is **materially more expensive** and most callers
    /// must not pay for it. `locate_all` returns geometry only — measured at ~90–170 ms
    /// per element for the rectangle alone — and reading a label costs a further round
    /// trip on top of that, per element.
    ///
    /// It exists because one caller genuinely needs the text and cannot get it any
    /// other way: reading back *which comment* is at a rectangle. Selecting a comment
    /// row by geometry needs an author label that is not empty
    /// (`crate::interaction_hierarchy`), and there is no primitive for "describe the
    /// element at this rectangle" — the query is the only handle a caller has.
    ///
    /// Call it once per send, never inside a poll loop. The default delegates, so a
    /// backend that has no cheaper `locate_all` loses nothing.
    async fn locate_all_described(
        &self,
        query: ElementQuery<'_>,
    ) -> anyhow::Result<Vec<ElementBox>> {
        self.locate_all(query).await
    }
    /// Read one full accessibility tree for an effect-bound identity proof.
    ///
    /// The default refuses. A caller must never replace it with independently queried elements:
    /// those can come from different cards while TikTok is animating between feed items.
    async fn hierarchy_source_snapshot(&self) -> anyhow::Result<HierarchySourceSnapshot> {
        unsupported("hierarchySourceSnapshot")
    }
    /// Whether [`Self::locate_description`] answers instead of refusing.
    ///
    /// Separate from [`Self::supports_accessibility_readback`] on purpose: a
    /// backend could read text without being able to report geometry, and the
    /// nurture loop needs geometry to tap.
    fn supports_element_bounds(&self) -> bool {
        false
    }
    /// The UI language, for picking a measured label set.
    ///
    /// `None` means "unknown", which callers must treat as refuse-to-guess:
    /// `content-desc` is translated on some builds, so the wrong language
    /// produces locators that silently match nothing
    /// (`crate::tiktok_labels`).
    async fn ui_language(&self) -> Option<String> {
        None
    }
    /// The installed `versionName` of an app, for picking a measured label set.
    ///
    /// Needed for the same reason as [`Self::ui_language`] and for the mirror-image
    /// reason: some measured labels are unresolved Android resource references, which
    /// are language-independent but **reassigned on every app rebuild**. Measured, two
    /// phones on the same package and language: the comment drawer's Send button is
    /// `@2131823284` on TikTok 46.3.3 and `@2131823293` on 46.4.3
    /// (`crate::tiktok_labels`).
    ///
    /// `None` means "unknown", which callers treat as unmeasured — those controls
    /// refuse rather than tapping an id that may now belong to a different button.
    /// Backends whose labels are all real strings have nothing to answer here.
    async fn app_version(&self, _bundle_id: &str) -> Option<String> {
        None
    }
    /// Raw screen capture via the UI channel (WDA `GET /screenshot`).
    /// Cheap (~0.3s over USB) unlike the pymobiledevice3 path.
    async fn screenshot_png(&self) -> anyhow::Result<Vec<u8>> {
        anyhow::bail!("screenshot_png not supported")
    }
    fn stream_url(&self) -> Option<String>;
}

/// How to find an element in a hierarchy that can be queried.
///
/// Two strategies because two are needed and no more: TikTok's action rail is
/// labelled, and its comment input is not — that field carries a placeholder in
/// `text` and an empty `content-desc`, so only its class identifies it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ElementQuery<'a> {
    /// Match `content-desc`. `exact` is false for labels that embed a value: a
    /// comment label carries its own count (`… 697 bình luận`), so an exact match
    /// can never hit one.
    Description { value: &'a str, exact: bool },
    /// Match the widget class, e.g. `android.widget.EditText`.
    ClassName(&'a str),
    /// Match the **suffix** of the fully-qualified `resource-id`.
    ///
    /// Measured 24/08/2026, and the measurement is the whole reason this exists. TikTok's caption
    /// node is `com.bytedance.tux.input.TuxTextLayoutView` on `com.ss.android.ugc.trill` and
    /// **`X.1BOr`** on `com.zhiliaoapp.musically` — the class name is obfuscated and changes with
    /// the build — while *both* carry a `resource-id` ending `:id/desc`. Keying the caption on the
    /// class therefore read it on one build and returned nothing on the other, which is what made
    /// four of twenty phones unconfirmable during a pass and got recorded as "the deep link did
    /// not navigate" when the post had been open the whole time.
    ///
    /// A suffix rather than the whole id because the id embeds the package, and the package is
    /// exactly what differs between the two builds.
    ///
    /// **A literal, not a pattern** — the driver escapes it before building the match.
    ResourceIdSuffix(&'a str),
    /// Match the node's rendered `text`, not its `content-desc`.
    ///
    /// Needed because the two are not interchangeable: TikTok's action rail is
    /// described, while the Reply button inside its comment drawer carries an
    /// **empty** `content-desc` and puts `Trả lời` in `text`. A comment body is the
    /// same — its content *is* its text.
    Text { value: &'a str, exact: bool },
}

impl<'a> ElementQuery<'a> {
    pub fn description_contains(value: &'a str) -> Self {
        Self::Description {
            value,
            exact: false,
        }
    }
}

/// A control's on-screen rectangle, in the same coordinate space as
/// [`UiSession::tap`], plus the label and state it was found with.
///
/// The label comes back with the geometry because one hierarchy query already
/// carries it, and the loop needs it: the comment control's own text
/// (`… 697 bình luận`) is what distinguishes one post from the next, which is
/// how a swipe is proved to have advanced without looking at pixels.
#[derive(Debug, Clone, PartialEq)]
pub struct ElementBox {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    /// The element's label, as read by **whichever method produced this box**.
    ///
    /// Not always `content-desc`, and the difference matters: [`UiSession::locate`] reads
    /// `content-desc`, [`UiSession::locate_all_described`] reads the rendered `text`, and
    /// [`UiSession::locate_all`] reads neither and leaves this `None`. That is measured,
    /// not sloppy — a TikTok comment row carries its author and body in `text` with an
    /// empty `content-desc`, while the action rail is the other way round, so a single
    /// attribute cannot serve both.
    ///
    /// The consequence for callers: **never compare two `description`s that came from
    /// different methods.** They are labels from different attributes and disagreeing is
    /// the normal case.
    pub description: Option<String>,
    /// Whether the control is enabled.
    ///
    /// Carried because it is a *proof*, not decoration: TikTok's comment Send
    /// button exists in the drawer the whole time and flips from
    /// `enabled=false` to `enabled=true` when the field holds text. That
    /// transition is the same "send is armed" evidence the iOS engine has to
    /// detect by pixel colour, available here as an attribute.
    pub enabled: bool,
    /// Whether the control accepts a tap.
    ///
    /// A second armed-flag, carried for the same reason as [`Self::enabled`] and
    /// **not** interchangeable with it. Measured 29/08/2026 on the build sixteen of
    /// the twenty phones run (`com.ss.android.ugc.trill` 38.3.2), on TikTok's image
    /// picker: the `Next` button that advances out of the picker reads
    ///
    /// ```text
    ///   nothing selected   clickable=false  enabled=true
    ///   one image selected clickable=true   enabled=true
    /// ```
    ///
    /// so `enabled` is constant across the transition and proves nothing there, while
    /// the comment drawer's Send button on another build moves `enabled` and not this.
    /// Two different apps' idea of "armed", and a caller must ask for the one its own
    /// screen was measured to move.
    ///
    /// # Only [`UiSession::locate`] fills this in
    ///
    /// [`UiSession::locate_all`] and [`UiSession::locate_all_described`] leave it
    /// `false`, exactly as they leave `enabled` at a constant: both skip the
    /// per-element attribute round trips on purpose, and they are called against
    /// list screens where the cost is per row. Do not filter a list by this field —
    /// you would drop every row.
    ///
    /// # Unreadable means `false`, which is the opposite of `enabled`'s default
    ///
    /// `enabled` defaults to `true` when the attribute cannot be read, because
    /// reporting a live Send button as unarmed would silently drop every comment. The
    /// safe direction here is the other one: this flag gates a **post**, and reading
    /// "armed" from a failed attribute read would tap `Next` with nothing selected —
    /// or, further along, publish. Unknown therefore refuses.
    pub clickable: bool,
}

/// One hierarchy element plus the optional selection signals carried by its XML node.
///
/// `None` is materially different from `false`: absent means the backend did not measure a
/// state and cannot authorize a toggle. Kept beside, rather than inside, [`ElementBox`] so the
/// existing element contract and every Like/Comment/Follow fixture remain unchanged.
#[derive(Debug, Clone, PartialEq)]
pub struct StatefulElementBox {
    pub element: ElementBox,
    pub checked: Option<bool>,
    pub selected: Option<bool>,
}

impl ElementBox {
    pub fn centre(&self) -> TapPoint {
        TapPoint {
            x: self.x + self.width / 2.0,
            y: self.y + self.height / 2.0,
        }
    }

    /// Half-extents for the touch planner, shrunk so jitter stays clear of the
    /// edge.
    ///
    /// A rectangle read off the hierarchy is the control's *hit area*; tapping
    /// its last pixel row is a coin flip on rounding. 40% of the half-extent,
    /// floored at one point, keeps every planned tap comfortably inside.
    pub fn jitter_radius(&self) -> (f64, f64) {
        (
            (self.width / 2.0 * 0.4).max(1.0),
            (self.height / 2.0 * 0.4).max(1.0),
        )
    }
}
