use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Context;
use parking_lot::{Mutex, RwLock};
use riviu_core::db::Database;
use riviu_core::{
    AppEvent, BackgroundStreamLease, DeviceControlPlane, DeviceExclusiveContext, DeviceRegistry,
    DeviceWorkCoordinator, DeviceWorkOwner, EventBus, FlowArtifactStore, FlowId, FlowRuntime,
    FlowRuntimeDeps, Frame, JobQueue, JobStatus, NurtureEngine, StreamBudgetManager,
    StreamSettings, UiSession, UiSessionContext, STREAM_FPS,
};
use riviu_ios_driver::{create_driver, DriverMode, DriverTarget, StreamHub};
use riviu_signing::{CredentialStore, SigningService};
use tauri::{AppHandle, Emitter};
use tokio::sync::{Mutex as AsyncMutex, Notify};

use crate::agent_runtime::{resolve_desktop_agent_runtime_with_candidate, ResolvedAgentRuntime};
use crate::command_error::CommandError;
use crate::nurture_commands::NurtureRuntime;

const DEFAULT_DESKTOP_STREAM_CAPACITY: usize = 2;
const MAX_DESKTOP_STREAM_CAPACITY: usize = 100;
/// UI preview is deliberately a separate budget from the raw stream consumed
/// by popup watchers and action confirmation. At two phones this preserves
/// 24 FPS per tile; at 20-100 phones it degrades evenly instead of flooding
/// the WebView with thousands of base64 events per second.
const PREVIEW_TOTAL_FPS: u32 = 240;
const PREVIEW_MAX_FPS_PER_DEVICE: u32 = STREAM_FPS;
const PREVIEW_TICK: Duration = Duration::from_millis(4);
const PREVIEW_IDLE_EVICTION: Duration = Duration::from_secs(10);
/// How often the keeper states whether the frontend's paint evidence is arriving at all.
///
/// Rare enough not to be noise across a long run, frequent enough that opening the log after
/// a report of "the picture is stuck" answers immediately whether the fine rule was even
/// running. Every 15 ticks of the 2 s loop.
const VIEW_EVIDENCE_LOG_EVERY: Duration = Duration::from_secs(30);
/// How far back [`AppState::busy_reason`] looks for unfinished jobs.
///
/// `list_jobs` orders newest first, and anything queued or running is recent by nature,
/// so a bounded scan answers the question without reading the whole table. The residual:
/// a job stuck in `Running` for longer than the last 500 records would fall outside the
/// window and be missed. Left bounded rather than unbounded because this runs on every
/// update check.
const BUSY_JOB_SCAN_LIMIT: usize = 500;

fn preview_fps_for_device_count(count: usize) -> u32 {
    (PREVIEW_TOTAL_FPS / count.max(1) as u32).clamp(1, PREVIEW_MAX_FPS_PER_DEVICE)
}

/// Keep the current two-phone behavior as the default while allowing a farm
/// deployment to opt into one producer per connected phone. Invalid values
/// fail closed to the default instead of creating an accidental USB storm.
/// What the operator asked for, or `None` if they said nothing usable.
///
/// `None` rather than the default, so the caller can tell "not configured" from
/// "configured as 2" — the first is sized from the fleet, the second is an instruction.
fn configured_desktop_stream_capacity() -> Option<usize> {
    let raw = std::env::var("RIVIU_STREAM_CAPACITY").ok()?;
    match raw.trim().parse::<usize>() {
        Ok(value) if (1..=MAX_DESKTOP_STREAM_CAPACITY).contains(&value) => Some(value),
        _ => {
            log::warn!("invalid RIVIU_STREAM_CAPACITY={raw:?}; sizing from the fleet instead");
            None
        }
    }
}

/// Build the stream budget, falling back rather than panicking.
///
/// `configured_desktop_stream_capacity` accepts up to 100 but the budget
/// manager hard-caps concurrent producers at 2 (AGENTS.md 3.5/3.12), so
/// `RIVIU_STREAM_CAPACITY=3` used to panic the app at startup through an
/// `expect`. The env var's own contract is to fail closed to the default, and
/// that is what a farm-sized value gets now, with the reason logged.
/// The stream budget this desktop runs with.
///
/// `fleet_size` is how many phones the first scan found. The budget is sized to it so a
/// two-phone bench gets two and a twenty-phone farm gets twenty — the operator's explicit
/// `RIVIU_STREAM_CAPACITY` still wins, and a fleet of zero (nothing plugged in yet, or a
/// scan that failed) falls back to the conservative default rather than reserving for
/// phones that may never arrive.
fn desktop_stream_budget(fleet_size: usize) -> StreamBudgetManager {
    let requested = configured_desktop_stream_capacity().unwrap_or_else(|| {
        fleet_size.clamp(DEFAULT_DESKTOP_STREAM_CAPACITY, MAX_DESKTOP_STREAM_CAPACITY)
    });
    match StreamBudgetManager::new(requested) {
        Ok(manager) => manager,
        Err(error) => {
            log::warn!(
                "RIVIU_STREAM_CAPACITY={requested} is above the stream budget ceiling ({error}); \
                 using {DEFAULT_DESKTOP_STREAM_CAPACITY}"
            );
            StreamBudgetManager::new(DEFAULT_DESKTOP_STREAM_CAPACITY)
                .expect("the default desktop stream capacity is within the ceiling")
        }
    }
}

/// Which lease a device command is running under.
///
/// Both arms hand out the same thing to the control plane — a live lease it can validate —
/// so the caller stops caring which one it got. The difference is only in the release:
/// `Owned` drops with the command, `Overlay` outlives it.
pub enum DeviceLease {
    /// The overlay already holds this phone; the command rides that lease.
    Overlay(Arc<UiSessionContext>),
    /// Nothing held it, so this command took its own and gives it back on drop.
    Owned(DeviceExclusiveContext),
}

/// So a command can keep writing `&context` and not care which arm it got.
impl<'a> From<&'a DeviceLease> for riviu_core::DeviceLeaseRef<'a> {
    fn from(lease: &'a DeviceLease) -> Self {
        match lease {
            DeviceLease::Overlay(context) => context.as_ref().into(),
            DeviceLease::Owned(context) => context.into(),
        }
    }
}

/// Whether a command that has to take its own lease should park the live preview.
///
/// Only consulted on the `Owned` path: an overlay that is already open has a running stream
/// by definition, and parking it underneath the operator would be absurd.
#[derive(Clone, Copy)]
pub enum LeaseStream {
    /// Keep the preview running — the point of the action is to watch it happen.
    Keep,
    /// Park it; the action is long or disruptive enough that the stream is in the way.
    Park,
}

pub struct AppState {
    pub registry: DeviceRegistry,
    pub events: EventBus,
    pub control: Arc<DeviceControlPlane>,
    pub streams: StreamHub,
    pub driver_mode: DriverMode,
    /// Set when the device sidecar failed to start; the UI shows it so an empty
    /// device list is never mistaken for "nothing plugged in".
    pub driver_degraded_reason: Option<String>,
    /// Asked fresh, not stored: why the **last** listing was empty.
    ///
    /// `driver_degraded_reason` above is a boot snapshot and cannot change. This one can,
    /// which is the point — an operator who installs Apple Devices while the app is open
    /// should stop being told it is missing, and a listing that succeeds clears it.
    pub driver_list_error: Option<Arc<dyn Fn() -> Option<String> + Send + Sync>>,
    /// Why the Android backend is absent, when it is. `None` means it joined
    /// the fleet. Kept separate from `driver_degraded_reason`, which is about
    /// the iOS sidecar: "no adb on this machine" and "the iOS sidecar died"
    /// are different facts and must not be reported as one.
    pub android_unavailable_reason: Option<String>,
    /// Concrete Android backend, kept so view start/stop/retune do not go
    /// through `DeviceDriver` or `StreamBudgetManager`.
    pub android: Option<Arc<riviu_android_driver::AndroidDriver>>,
    pub view_hub: Arc<crate::view_hub::ViewHub>,
    /// What the frontend last reported painting, per device.
    ///
    /// Written by the `view_report_paint` command and read by the keeper. This is the half
    /// of the evidence the host cannot gather for itself: bytes arriving prove a phone is
    /// talking, not that anything came out of the decoder.
    pub view_paint: Arc<crate::view_watchdog::ViewPaintLedger>,
    /// The fleet-wide ceiling every producer restart passes through, automatic or not.
    ///
    /// On `AppState` because it is the one object both the keeper task and every Tauri
    /// command can reach, and because a ceiling that only one of the two respects is not a
    /// ceiling — which is exactly the state AGENTS.md 9.67 measured at 291 restarts.
    pub view_recovery: Arc<crate::view_watchdog::ViewRecoveryGate>,
    pub jobs: JobQueue,
    pub flows: FlowRuntime,
    pub flow_artifacts: FlowArtifactStore,
    /// The same store mechanism as Flow's, rooted separately. Interaction had
    /// no artifact storage at all: `interaction_artifacts.relative_path` has
    /// been in the schema since the first migration and was never written, so a
    /// campaign left behind hashes of frames nobody kept.
    pub interaction_artifacts: FlowArtifactStore,
    pub db: Arc<Database>,
    pub signing: SigningService,
    /// The OS credential store, for secrets that must not sit in the SQLite file.
    pub secrets: CredentialStore,
    pub agent_token_configured: bool,
    pub active_agent_artifact_id: String,
    pub active_agent_artifact_version: String,
    pub active_agent_bundle_id: String,
    pub stream_settings: Arc<RwLock<StreamSettings>>,
    pub artifacts_dir: PathBuf,
    pub legacy_wda_bundle: PathBuf,
    pub nurture: NurtureRuntime,
    pub nurture_engine: NurtureEngine,
    pub(crate) flow_mutations: FlowMutationCoordinator,
    /// One ManualControl session per UDID while the overlay is open.
    /// Gestures reuse it; closing the overlay is the only release.
    /// The lease the control overlay holds, per device, for as long as it is open.
    ///
    /// `Arc` rather than the bare context because the overlay's own actions now *borrow* this
    /// lease instead of asking for a second one (see [`AppState::device_lease`]). A borrower
    /// holds its clone for the whole run of a command — an export or a reboot is not
    /// instant — and it has to outlive an overlay the operator closes half-way through.
    /// Dropping a `UiSessionContext` releases the lease and the activity permit and nothing
    /// else, so "the last holder releases it" is already the semantics rather than a new one.
    overlay_sessions: AsyncMutex<HashMap<String, Arc<UiSessionContext>>>,
    /// One gate per device, so opening an overlay serialises against itself and nothing else.
    ///
    /// Held only while a `begin` is in flight for that phone. The map of gates is locked just
    /// long enough to clone an `Arc` out, never across device I/O — the mistake this exists to
    /// undo. Bounded by the number of distinct serials the app has seen, which is the fleet.
    overlay_gates: AsyncMutex<HashMap<String, Arc<AsyncMutex<()>>>>,
    command_admission: Arc<CommandAdmissionState>,
    background_stop: Arc<AtomicBool>,
    background_stopped: Arc<AtomicBool>,
    background_stopped_notify: Arc<Notify>,
    background_shutdown_error: Arc<RwLock<Option<String>>>,
}

struct CommandAdmissionState {
    accepting_work: AtomicBool,
    in_flight: AtomicUsize,
    changed: Notify,
}

#[derive(Default)]
pub(crate) struct FlowMutationCoordinator {
    event_revision: Mutex<u64>,
}

impl FlowMutationCoordinator {
    pub(crate) fn commit<T, E>(
        &self,
        events: &EventBus,
        persist: impl FnOnce() -> Result<(T, FlowId), E>,
    ) -> Result<T, E> {
        let mut revision = self.event_revision.lock();
        let (result, flow_id) = persist()?;
        *revision = revision
            .checked_add(1)
            .expect("Flow invalidation revision overflow");
        events.emit(AppEvent::FlowUpdated {
            flow_id,
            revision: *revision,
        });
        Ok(result)
    }
}

pub(crate) struct CommandAdmission {
    state: Arc<CommandAdmissionState>,
}

impl CommandAdmissionState {
    fn new(accepting_work: bool) -> Self {
        Self {
            accepting_work: AtomicBool::new(accepting_work),
            in_flight: AtomicUsize::new(0),
            changed: Notify::new(),
        }
    }

    fn start_accepting(&self) {
        self.accepting_work.store(true, Ordering::Release);
        self.changed.notify_waiters();
    }

    fn ensure_accepting_work(self: &Arc<Self>) -> Result<CommandAdmission, CommandError> {
        if !self.accepting_work.load(Ordering::Acquire) {
            return Err(CommandError::application_shutting_down());
        }
        self.in_flight.fetch_add(1, Ordering::AcqRel);
        if !self.accepting_work.load(Ordering::Acquire) {
            self.finish_one();
            return Err(CommandError::application_shutting_down());
        }
        Ok(CommandAdmission {
            state: self.clone(),
        })
    }

    fn reject_new_work(&self) {
        self.accepting_work.store(false, Ordering::Release);
        self.changed.notify_waiters();
    }

    async fn wait_until_drained(&self) {
        loop {
            let changed = self.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            if self.in_flight.load(Ordering::Acquire) == 0 {
                return;
            }
            changed.await;
        }
    }

    fn finish_one(&self) {
        if self.in_flight.fetch_sub(1, Ordering::AcqRel) == 1 {
            self.changed.notify_waiters();
        }
    }
}

impl Drop for CommandAdmission {
    fn drop(&mut self) {
        self.state.finish_one();
    }
}

struct ActiveBackgroundSample {
    lease: BackgroundStreamLease,
    baseline_sequence: Option<u64>,
    last_frame_at: Instant,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum SamplerTick {
    Idle,
    Started(String),
    Sampling(String),
    Parked(String),
    Stale(String),
    Preempted(String),
    Failed(String),
}

struct BackgroundStreamSampler {
    control: Arc<DeviceControlPlane>,
    streams: StreamHub,
    registry: DeviceRegistry,
    active: Vec<ActiveBackgroundSample>,
    cursor: usize,
}

impl BackgroundStreamSampler {
    fn new(control: Arc<DeviceControlPlane>, streams: StreamHub, registry: DeviceRegistry) -> Self {
        Self {
            control,
            streams,
            registry,
            active: Vec::new(),
            cursor: 0,
        }
    }

    async fn tick(&mut self) -> SamplerTick {
        let samples = std::mem::take(&mut self.active);
        let mut retained = Vec::with_capacity(samples.len());
        let mut first_event = None;
        let mut terminal_change = false;
        let multi_stream = self.control.configured_stream_capacity() > 1;

        for mut sample in samples {
            let udid = sample.lease.udid().to_string();
            if !self.control.background_stream_is_current(&sample.lease) {
                set_stream_parked(&self.registry, &udid);
                terminal_change = true;
                if first_event.is_none() {
                    first_event = Some(SamplerTick::Preempted(udid));
                }
                continue;
            }

            let latest_sequence = self.streams.latest_frame_sequence(&udid);
            let fresh = latest_sequence
                .map(|sequence| Some(sequence) != sample.baseline_sequence)
                .unwrap_or(false);
            let turn_due = self
                .control
                .background_turn_due(&sample.lease)
                .unwrap_or(true);
            // Keep the producer alive for its whole bounded turn. The old
            // path stopped as soon as the first frame arrived, which left the
            // tile with a static snapshot and made a working MJPEG stream look
            // like "No stream" to the operator. A foreground job still
            // preempts this lease through the ownership check above.
            // With two desktop tile slots, a healthy producer stays live past
            // the bookkeeping deadline. A producer that stops advancing still
            // gets recycled at the next deadline.
            if fresh {
                sample.baseline_sequence = latest_sequence;
                sample.last_frame_at = Instant::now();
            }
            let stream_watchdog_expired = sample.last_frame_at.elapsed() >= Duration::from_secs(5);
            if turn_due && (!multi_stream || (!fresh && stream_watchdog_expired)) {
                if !fresh {
                    log::warn!(
                        "background stream stale: udid={} baseline_sequence={:?} latest_sequence={:?} last_frame_age_ms={}",
                        udid,
                        sample.baseline_sequence,
                        latest_sequence,
                        sample.last_frame_at.elapsed().as_millis(),
                    );
                }
                match self.control.stop_background_stream(&sample.lease).await {
                    Ok(_) if fresh => {
                        set_stream_parked(&self.registry, &udid);
                        terminal_change = true;
                        if first_event.is_none() {
                            first_event = Some(SamplerTick::Parked(udid));
                        }
                    }
                    Ok(_) => {
                        set_stream_state(
                            &self.registry,
                            &udid,
                            riviu_core::TileStreamState::Stale,
                            Some(
                                "stream sampler did not receive a fresh frame within 5 seconds"
                                    .to_string(),
                            ),
                        );
                        terminal_change = true;
                        if first_event.is_none() {
                            first_event = Some(SamplerTick::Stale(udid));
                        }
                    }
                    Err(error) => {
                        set_stream_state(
                            &self.registry,
                            &udid,
                            riviu_core::TileStreamState::Error,
                            Some(error.to_string()),
                        );
                        retained.push(sample);
                        if first_event.is_none() {
                            first_event = Some(SamplerTick::Failed(udid));
                        }
                    }
                }
            } else {
                set_stream_state(
                    &self.registry,
                    &udid,
                    riviu_core::TileStreamState::Live,
                    None,
                );
                retained.push(sample);
                if first_event.is_none() {
                    first_event = Some(SamplerTick::Sampling(udid));
                }
            }
        }
        self.active = retained;

        // A stop/preemption is left visible for one tick; the next tick starts
        // replacements. This preserves a truthful handoff state and prevents
        // a failed producer from being hidden by an immediate restart.
        if !terminal_change && self.active.len() < self.control.configured_stream_capacity() {
            let devices = self.registry.list();
            while self.active.len() < self.control.configured_stream_capacity() {
                let mut started = false;
                for offset in 0..devices.len() {
                    let index = (self.cursor + offset) % devices.len();
                    let device = &devices[index];
                    if !background_sample_candidate(
                        device,
                        &self.control.cached_agent_status(&device.udid),
                    ) {
                        continue;
                    }
                    let Ok(lease) = self.control.reserve_background_stream(&device.udid) else {
                        continue;
                    };
                    set_stream_state(
                        &self.registry,
                        &device.udid,
                        riviu_core::TileStreamState::Sampling,
                        None,
                    );
                    let baseline_sequence = self.streams.latest_frame_sequence(&device.udid);
                    match self.control.start_background_stream(&lease).await {
                        Ok(url) => {
                            if let Some(mut current) = self.registry.get(&device.udid) {
                                current.stream_url = Some(url);
                                current.status = riviu_core::DeviceStatus::Ready;
                                // ensure_stream returns only after the unified agent has
                                // produced its first frame. Keep the readiness banner
                                // truthful even after a normal producer handoff.
                                current.wda_ready = true;
                                current.tile_stream_state = riviu_core::TileStreamState::Sampling;
                                current.last_error = None;
                                self.registry.upsert(current);
                            }
                            self.active.push(ActiveBackgroundSample {
                                lease,
                                baseline_sequence,
                                last_frame_at: Instant::now(),
                            });
                            self.cursor = (index + 1) % devices.len().max(1);
                            if first_event.is_none() {
                                first_event = Some(SamplerTick::Started(device.udid.clone()));
                            }
                            started = true;
                        }
                        Err(error) => {
                            set_stream_state(
                                &self.registry,
                                &device.udid,
                                riviu_core::TileStreamState::Error,
                                Some(error.to_string()),
                            );
                            self.cursor = (index + 1) % devices.len().max(1);
                            if first_event.is_none() {
                                first_event = Some(SamplerTick::Failed(device.udid.clone()));
                            }
                        }
                    }
                    break;
                }
                if !started {
                    break;
                }
            }
        }
        first_event.unwrap_or(SamplerTick::Idle)
    }

    async fn stop(&mut self) -> Result<(), riviu_core::DeviceControlError> {
        let samples = std::mem::take(&mut self.active);
        let mut first_error = None;
        for sample in samples {
            if self.control.background_stream_is_current(&sample.lease) {
                if let Err(error) = self.control.stop_background_stream(&sample.lease).await {
                    self.active.push(sample);
                    if first_error.is_none() {
                        first_error = Some(error);
                    }
                    continue;
                }
            }
            set_stream_parked(&self.registry, sample.lease.udid());
        }
        first_error.map_or(Ok(()), Err)
    }
}

/// Bridges `riviu-core`'s [`SecretStore`] onto the OS credential store.
///
/// The seam exists because `crates/core` must not depend on `riviu-signing` — core stays
/// unaware of keyrings the same way it stays unaware of the driver crates. This is the one
/// place the two meet, and it is four lines.
struct KeyringSecrets {
    credentials: CredentialStore,
}

impl KeyringSecrets {
    fn new(credentials: CredentialStore) -> Self {
        Self { credentials }
    }
}

impl riviu_core::db::SecretStore for KeyringSecrets {
    fn get_secret(&self, name: &str) -> anyhow::Result<Option<String>> {
        self.credentials.app_secret(name)
    }

    fn set_secret(&self, name: &str, value: &str) -> anyhow::Result<()> {
        self.credentials.set_app_secret(name, value)
    }
}

impl AppState {
    pub async fn bootstrap(resource_dir: Option<PathBuf>) -> anyhow::Result<Self> {
        let mock_requested = std::env::var("RIVIU_MOCK_DEVICES")
            .map(|value| value == "1" || value.eq_ignore_ascii_case("true"))
            .unwrap_or(false);
        let data = resolve_desktop_data_dir(
            mock_requested,
            std::env::var_os("RIVIU_MOCK_DATA_DIR").map(PathBuf::from),
        )?;
        std::fs::create_dir_all(&data)?;
        let artifacts_dir = data.join("artifacts");
        std::fs::create_dir_all(&artifacts_dir)?;

        let sidecar_root = resolve_sidecar_root(resource_dir.as_deref());
        let credentials = CredentialStore::system()?;
        // The database gets somewhere to put secrets that is not the SQLite file. Opened after
        // the credential store precisely so it can be handed one: the AI API key used to sit in
        // the settings blob in cleartext, readable by any process running as the operator.
        let db = Arc::new(
            Database::open(data.join("riviu.db"))?
                .with_secrets(Arc::new(KeyringSecrets::new(credentials.clone()))),
        );
        let legacy_token = std::env::var("RIVIU_RTMMO_TOKEN").ok();
        let ResolvedAgentRuntime {
            driver_config,
            settings: resolved_agent_settings,
            token_configured: agent_token_configured,
        } = resolve_desktop_agent_runtime_with_candidate(
            sidecar_root.clone(),
            data.clone(),
            &db,
            &credentials,
            legacy_token.as_deref(),
            mock_requested,
            true,
        )?;
        let (active_agent_artifact_id, active_agent_artifact_version, active_agent_bundle_id) =
            match &driver_config.target {
                DriverTarget::Real(config) => (
                    config.artifact.manifest.artifact_id.clone(),
                    config.artifact.manifest.artifact_version.clone(),
                    config.artifact.manifest.bundle_id.clone(),
                ),
                DriverTarget::Mock => (
                    "riviu-agent-mock".to_string(),
                    "1.0.0".to_string(),
                    "com.riviu.managersphone.agent.xctrunner".to_string(),
                ),
                DriverTarget::LegacyStock => (
                    "legacy-stock-wda".to_string(),
                    String::new(),
                    "com.riviu.managersphone.agent.xctrunner".to_string(),
                ),
            };
        let bundle = create_driver(driver_config).await?;
        bundle.driver.set_agent_settings(resolved_agent_settings);

        // One plane over both platforms. The Android backend joins only when
        // `adb` is actually usable, so a machine with no Android tooling gets
        // no backend rather than one that is permanently degraded — and the
        // reason is kept so the UI can say which it is.
        let mut backends: Vec<(String, Arc<dyn riviu_core::driver::DeviceDriver>)> =
            vec![("ios".to_string(), bundle.driver.clone())];
        // Verified bundled tools, offered at the **lowest** priority so an operator's
        // own adb or `RIVIU_MINICAP_APK` still wins. A corrupt bundle costs its own
        // tool and nothing else; see `android_tools`.
        let android_tools = crate::android_tools::AndroidTools::load(&sidecar_root);
        for problem in &android_tools.problems {
            log::warn!("bundled Android tools: {problem}");
        }
        let view_hub = crate::view_hub::ViewHub::new();
        let android_config = riviu_android_driver::AndroidDriverConfig {
            bundled_adb_path: android_tools.adb_path.clone(),
            bundled_minicap_apk: android_tools.minicap_apk.clone(),
            bundled_scrcpy_server: android_tools.scrcpy_server.clone(),
            bundled_riviu_agent_apk: android_tools.riviu_agent_apk.clone(),
            bundled_agent_server_apk: android_tools.agent_server_apk.clone(),
            bundled_agent_test_apk: android_tools.agent_test_apk.clone(),
            ..Default::default()
        };
        // Restored from the database rather than rebuilt from `Default`, which is what made
        // every quality and frame-rate choice last exactly as long as the app was open.
        //
        // Warn-and-default rather than propagate: the helper is strict so a corrupt row is
        // reported, but refusing to open the app over a stream-quality setting would trade a
        // cosmetic loss for a total one. The clamp is applied on the way in as well as on
        // the way out, so a value stored by an older build cannot make `get_stream_settings`
        // report a rate the encoder will never run at.
        let stream_settings = {
            let mut settings = db.get_stream_settings().unwrap_or_else(|error| {
                log::warn!("stored stream settings could not be read ({error:#}); using defaults");
                StreamSettings::default()
            });
            settings.fps = crate::commands::clamp_stream_fps(settings.fps);
            settings
        };

        let (android, android_unavailable) =
            match riviu_android_driver::detect_driver(&android_config).await {
                Ok(driver) => {
                    // JPEG evidence still lands in StreamHub. H.264 view samples
                    // go to ViewHub and never become a Frame.
                    driver.set_frame_sink(Arc::new(bundle.streams.clone()));
                    driver.set_view_sink(
                        Arc::clone(&view_hub) as Arc<dyn riviu_android_driver::ViewSink>
                    );
                    // The restored choice has to reach the driver before the first producer
                    // spawns: `spawn_view` reads `view_tuning` and takes no argument for it,
                    // so without this the settings panel would report the operator's values
                    // while every tile started at the compiled-in defaults until they saved
                    // again. That is the UI-and-encoder disagreement of AGENTS.md 9.59.
                    driver.set_view_tuning(
                        stream_settings.grid_quality.clone(),
                        stream_settings.focus_quality.clone(),
                        stream_settings.fps,
                    );
                    backends.push(("android".to_string(), driver.clone()));
                    (Some(driver), None)
                }
                Err(reason) => (None, Some(reason)),
            };
        let fleet: Arc<dyn riviu_core::driver::DeviceDriver> =
            Arc::new(riviu_core::driver_multiplex::MultiplexDriver::new(backends));

        // **How many phones may hold a producer at once — sized from the fleet that is
        // actually plugged in.**
        //
        // This was the constant 2, chosen when the desktop was a two-iPhone dev box and
        // described as "the desktop shows at most two tiles". Every nurture session holds a
        // *foreground* slot for its whole run, so on a twenty-phone farm that constant was
        // not bounding previews — it was refusing eighteen of twenty sessions outright with
        // `CapacityExhausted`. Measured 18/08/2026 before changing it: six concurrent
        // Android sessions all ran.
        //
        // Counting first costs one device scan, and it is the scan the registry needs a few
        // lines below anyway, so it is done here and reused rather than run twice. A scan
        // that fails falls back to the old constant rather than guessing high: an unknown
        // fleet is not a licence to start twenty producers.
        let initial_devices = fleet.list_devices().await.unwrap_or_else(|error| {
            log::warn!(
                "initial device scan failed ({error:#}); stream budget falls back to the default"
            );
            Vec::new()
        });
        let control = Arc::new(DeviceControlPlane::new_with_capability_registry(
            fleet,
            Arc::new(DeviceWorkCoordinator::new()),
            Arc::new(desktop_stream_budget(initial_devices.len())),
            bundle.interaction_capabilities.clone(),
        ));
        let signing =
            SigningService::with_credentials(sidecar_root.join("signer"), credentials.clone());

        let events = EventBus::new(512);
        let registry = DeviceRegistry::new(events.clone());

        let jobs = JobQueue::new(
            db.clone(),
            events.clone(),
            registry.clone(),
            control.clone(),
            artifacts_dir.clone(),
        );

        // The engine reads the screen from the frame stream the app already
        // runs for the device tiles, so it never has to ask WDA for a
        // screenshot. `StreamHub` is the FrameSource implementation.
        let nurture_engine = NurtureEngine::new(
            db.clone(),
            control.clone(),
            Arc::new(bundle.streams.clone()),
            artifacts_dir.clone(),
        )
        .with_frame_text_source(Arc::new(crate::interaction_ocr::DesktopFrameTextSource));

        // Recovery must see an authoritative initial fleet snapshot. Flow target selection
        // and startup reconciliation both fail closed on absent devices. Reuses the scan the
        // stream budget was sized from — two scans of twenty phones is two seconds of
        // startup for one answer.
        let devices = if initial_devices.is_empty() {
            control
                .list_devices()
                .await
                .context("initial metadata device scan failed before Flow recovery")?
        } else {
            initial_devices
        };
        registry.upsert_many(devices);

        let command_admission = Arc::new(CommandAdmissionState::new(false));
        let flow_artifacts = FlowArtifactStore::new(artifacts_dir.join("flows"))?;
        let interaction_artifacts = FlowArtifactStore::new(artifacts_dir.join("interactions"))?;
        let flows = FlowRuntime::new(FlowRuntimeDeps {
            database: db.clone(),
            events: events.clone(),
            registry: registry.clone(),
            control: control.clone(),
            frames: Arc::new(bundle.streams.clone()),
            artifacts: flow_artifacts.clone(),
        });
        flows.recover_startup().await?;
        let committed_artifacts = db.list_committed_flow_artifacts()?;
        for failure in flow_artifacts.reconcile(&committed_artifacts)? {
            log::warn!(
                "Flow artifact reconciliation {:?}: {}",
                failure.code,
                failure.artifact_id
            );
        }
        command_admission.start_accepting();

        let state = Self {
            registry,
            events,
            control,
            streams: bundle.streams,
            driver_mode: bundle.mode,
            driver_degraded_reason: bundle.degraded_reason,
            driver_list_error: bundle.list_error,
            android_unavailable_reason: android_unavailable,
            android,
            view_hub,
            view_paint: crate::view_watchdog::ViewPaintLedger::new(),
            view_recovery: crate::view_watchdog::ViewRecoveryGate::new(),
            jobs,
            flows,
            flow_artifacts,
            interaction_artifacts,
            db,
            signing,
            secrets: credentials,
            agent_token_configured,
            active_agent_artifact_id,
            active_agent_artifact_version,
            active_agent_bundle_id,
            stream_settings: Arc::new(RwLock::new(stream_settings)),
            artifacts_dir,
            legacy_wda_bundle: sidecar_root.join("wda").join("Riviumanagersphone.ipa"),
            nurture: NurtureRuntime::new(),
            nurture_engine,
            flow_mutations: FlowMutationCoordinator::default(),
            overlay_sessions: AsyncMutex::new(HashMap::new()),
            overlay_gates: AsyncMutex::new(HashMap::new()),
            command_admission,
            background_stop: Arc::new(AtomicBool::new(false)),
            background_stopped: Arc::new(AtomicBool::new(false)),
            background_stopped_notify: Arc::new(Notify::new()),
            background_shutdown_error: Arc::new(RwLock::new(None)),
        };

        Ok(state)
    }

    /// Open one ManualControl session for the overlay, or reuse the one already held.
    ///
    /// **Serialised per device, never across the fleet.** The map lock used to be held across
    /// `open_manual_session`, with a comment reasoning only about one UDID — but
    /// `overlay_sessions` is one map for every phone, and that open is not quick: on Android
    /// it can reach `install_agent_apks`, which allows 180 s per APK, or an instrumentation
    /// restart. Everything else that touches the map waited behind it: every gesture (through
    /// `overlay_ui_session`), `group_input`, `end_overlay_session`, and — since the lease
    /// borrow landed — every one of the eleven device rows through [`Self::device_lease`].
    /// Opening one freshly plugged phone could freeze device work on all twenty.
    ///
    /// The property that comment wanted is still here, and it is the only one that was ever
    /// needed: two begins on the *same* phone cannot race into DeviceBusy, because they
    /// serialise on that phone's own gate. Two begins on different phones no longer meet.
    pub async fn begin_overlay_session(&self, udid: &str) -> Result<(), CommandError> {
        let gate = {
            let mut gates = self.overlay_gates.lock().await;
            Arc::clone(
                gates
                    .entry(udid.to_string())
                    .or_insert_with(|| Arc::new(AsyncMutex::new(()))),
            )
        };
        let _opening = gate.lock().await;

        if self.overlay_sessions.lock().await.contains_key(udid) {
            return Ok(());
        }
        // No map lock held here, and that is the whole point of the gate above.
        let context = self
            .control
            .open_manual_session(udid, DeviceWorkOwner::ManualControl)
            .await
            .map_err(CommandError::from)?;
        self.overlay_sessions
            .lock()
            .await
            .insert(udid.to_string(), Arc::new(context));
        Ok(())
    }

    pub async fn end_overlay_session(&self, udid: &str) -> Result<(), CommandError> {
        let context = {
            let mut sessions = self.overlay_sessions.lock().await;
            sessions.remove(udid)
        };
        let Some(context) = context else {
            return Ok(());
        };
        // A command may still be riding this lease — a reboot, an export, an APK install.
        // Whoever is last to let go releases it; closing the overlay must not yank the phone
        // out from under work the operator started from that very overlay.
        let Some(context) = Arc::into_inner(context) else {
            return Ok(());
        };
        self.control
            .close_manual_session(context)
            .map_err(CommandError::from)?;
        Ok(())
    }

    pub async fn overlay_ui_session(&self, udid: &str) -> Option<Arc<dyn UiSession>> {
        let sessions = self.overlay_sessions.lock().await;
        let context = sessions.get(udid)?;
        self.control.session(context.as_ref()).ok()
    }

    /// The lease a command should run under: the overlay's if it has one, otherwise its own.
    ///
    /// **This is what makes ten rows of the control overlay work at all.** Rotate, install
    /// APK, import, export, adb, change keyboard, reboot, backup, restore and screenshot each
    /// used to take their own exclusive lease — on a phone this process was already holding
    /// open, because the overlay is the only place those rows exist. Every one of them was
    /// refused `DeviceBusy`. See AGENTS.md 9.83.
    ///
    /// The map is the *only* thing consulted. A phone held by nurture, a flow, a script or a
    /// repair is still refused, with the real owner named — a lease is only ever lent by the
    /// UI that opened it, never taken from someone else.
    pub async fn device_lease(
        &self,
        udid: &str,
        owner: DeviceWorkOwner,
        stream: LeaseStream,
    ) -> Result<DeviceLease, CommandError> {
        if let Some(context) = self.overlay_sessions.lock().await.get(udid).cloned() {
            return Ok(DeviceLease::Overlay(context));
        }
        let context = match stream {
            LeaseStream::Keep => {
                self.control
                    .try_acquire_exclusive_keeping_stream(udid, owner)
                    .await
            }
            LeaseStream::Park => self.control.try_acquire_exclusive(udid, owner).await,
        }
        .map_err(CommandError::from)?;
        Ok(DeviceLease::Owned(context))
    }

    /// Release every overlay lease before `shutdown_cleanup` waits for
    /// `lifecycle.outstanding() == 0`. A held ManualControl deadlocks that wait.
    pub async fn close_all_overlay_sessions(&self) {
        let contexts: Vec<Arc<UiSessionContext>> = {
            let mut sessions = self.overlay_sessions.lock().await;
            sessions.drain().map(|(_, context)| context).collect()
        };
        for context in contexts {
            // Every borrower holds a `CommandAdmission`, and shutdown drains those before
            // reaching here, so nothing should still be riding a lease at this point. If one
            // is, dropping our `Arc` is still correct — the borrower releases it — and the
            // wait below would be the thing to notice, not this.
            let Some(context) = Arc::into_inner(context) else {
                log::warn!("an overlay lease was still borrowed at shutdown");
                continue;
            };
            if let Err(error) = self.control.close_manual_session(context) {
                log::error!("overlay session close failed during shutdown: {error}");
            }
        }
    }

    /// What is running that an update would interrupt, or `None` when the fleet is idle.
    ///
    /// A sentence, not a bool. Installing an update replaces the running binary, and this
    /// process holds the WDA relays, XCTest runners and device leases that only its own
    /// shutdown releases — so an operator deciding whether to take an update now needs to
    /// know *what* they would be cutting off, not merely that something is there.
    ///
    /// Nurture sessions and the job queue are both consulted. Flow runs are not, and that
    /// is the remaining gap: `FlowRuntime` exposes no liveness query, its runs live in the
    /// database, and inventing one here would mean a second source of truth for "is this
    /// flow alive". So "idle" means "no nurture session and no unfinished job", not
    /// "nothing at all" — named rather than implied.
    ///
    /// An unreadable job queue counts as busy. Failing closed is the whole point: the cost
    /// of a wrong "idle" is cutting a live session off mid-run to swap the binary, while the
    /// cost of a wrong "busy" is an operator who waits and asks again.
    pub(crate) fn busy_reason(&self) -> Option<String> {
        let mut reasons: Vec<String> = Vec::new();
        let sessions = self
            .nurture
            .list_status()
            .into_iter()
            .filter(|status| status.running)
            .count();
        if sessions > 0 {
            reasons.push(format!("{sessions} phiên Nuôi TT đang chạy"));
        }
        match self.jobs.list_jobs(BUSY_JOB_SCAN_LIMIT) {
            Ok(jobs) => {
                let unfinished = jobs
                    .iter()
                    .filter(|job| matches!(job.status, JobStatus::Queued | JobStatus::Running))
                    .count();
                if unfinished > 0 {
                    reasons.push(format!("{unfinished} việc trong hàng đợi chưa xong"));
                }
            }
            Err(error) => reasons.push(format!("không đọc được hàng đợi việc ({error})")),
        }
        (!reasons.is_empty()).then(|| {
            format!(
                "{} — dừng hết trước khi cập nhật, vì bản cài mới thay thế tiến trình \
                 đang giữ session và lease của các máy",
                reasons.join("; ")
            )
        })
    }

    /// The Android backend, or the reason there isn't one.
    ///
    /// Twenty-two commands opened with the same three lines and the same flat sentence,
    /// "Android không khả dụng" — which is the one thing the operator already knows by the
    /// time they see it. `android_unavailable_reason` has held the actual cause the whole
    /// time ("adb not on PATH", "the sidecar died"), two fields away, and no command read it.
    pub(crate) fn require_android(
        &self,
    ) -> Result<&Arc<riviu_android_driver::AndroidDriver>, CommandError> {
        self.android
            .as_ref()
            .ok_or_else(|| match &self.android_unavailable_reason {
                Some(reason) => {
                    CommandError::operation(format!("Android không khả dụng: {reason}"))
                }
                None => CommandError::operation("Android không khả dụng"),
            })
    }

    pub(crate) fn ensure_accepting_work(&self) -> Result<CommandAdmission, CommandError> {
        self.command_admission.ensure_accepting_work()
    }

    pub(crate) fn reject_new_work(&self) {
        self.command_admission.reject_new_work();
    }

    pub(crate) async fn wait_for_mutating_commands(&self) {
        self.command_admission.wait_until_drained().await;
    }

    pub fn spawn_background_tasks(&self, app: AppHandle) {
        let view_hub = self.view_hub.clone();
        tauri::async_runtime::spawn(async move {
            if let Err(error) = view_hub.listen().await {
                log::error!("view websocket failed to bind: {error:#}");
            }
        });

        // Local automation API (Giai đoạn B, xiaowei "openapi"). Off unless the operator
        // turned it on and a token exists; it binds loopback only. Startup-configured on
        // purpose — a config change takes effect next launch, so the socket has one owner and
        // the lifecycle stays a single spawn rather than a bind/unbind dance we could not
        // verify without the running app.
        {
            let config = crate::local_api::load_config(&self.db, &self.secrets);
            if config.enabled && !config.token.is_empty() {
                let app = app.clone();
                tauri::async_runtime::spawn(async move {
                    if let Err(error) =
                        crate::local_api::serve(app, config.port, config.token).await
                    {
                        log::error!("local API failed to start: {error:#}");
                    }
                });
            }
        }

        if let Some(android) = self.android.clone() {
            let registry = self.registry.clone();
            let background_stop = self.background_stop.clone();
            let view_hub = self.view_hub.clone();
            let view_paint = self.view_paint.clone();
            let view_recovery = self.view_recovery.clone();
            tauri::async_runtime::spawn(async move {
                let mut interval = tokio::time::interval(Duration::from_secs(2));
                // Retune history stays task-local: it is a per-preset floor, not a recovery,
                // and it does not consume the fleet ceiling because it is not triggered by a
                // fault. Restart history moved into `ViewRecoveryGate`, which is where it has
                // to live now that operator commands share the same accounting.
                let mut view_retunes: HashMap<String, (Instant, u32)> = HashMap::new();
                let mut last_evidence_log = Instant::now() - VIEW_EVIDENCE_LOG_EVERY;
                loop {
                    interval.tick().await;
                    if background_stop.load(Ordering::Acquire) {
                        android.stop_all_views().await;
                        return;
                    }
                    // Say out loud whether the fine rule has anything to work with. When
                    // nobody is reporting paints the watchdog correctly falls back to the
                    // byte rule -- and that fallback is invisible, so a dead reporting path
                    // and a healthy one produce exactly the same (empty) log. AGENTS.md 9.66
                    // is three diagnosis rounds lost to precisely that shape.
                    if last_evidence_log.elapsed() >= VIEW_EVIDENCE_LOG_EVERY {
                        last_evidence_log = Instant::now();
                        let fresh = view_paint.fresh_count(Instant::now());
                        let android_devices = registry
                            .list()
                            .iter()
                            .filter(|device| device.platform == riviu_core::DevicePlatform::Android)
                            .count();
                        log::info!(
                            "view watchdog evidence: {fresh}/{android_devices} android devices \
                             reporting painted frames; {} of {} recovery slots in use",
                            view_recovery.in_flight(),
                            view_recovery.limit()
                        );
                    }
                    let mut starts = Vec::new();
                    for device in registry.list() {
                        if device.platform != riviu_core::DevicePlatform::Android {
                            continue;
                        }
                        if matches!(
                            device.status,
                            riviu_core::DeviceStatus::Disconnected
                                | riviu_core::DeviceStatus::Pairing
                                | riviu_core::DeviceStatus::Preparing
                                | riviu_core::DeviceStatus::Busy
                        ) {
                            continue;
                        }
                        if android.view_start_in_flight(&device.udid) {
                            continue;
                        }
                        // Reconcile toward the preset the operator asked for, rather than
                        // accepting either one as "running".
                        //
                        // The bug this fixes was observed live: opening an overlay races the
                        // keeper for the exclusive start claim, `view_set_preset` loses and
                        // returns "a scrcpy view start is already in flight", and the
                        // frontend logs that to the console and never asks again. The
                        // overlay then spends its whole life on the tile encode -- 232x480
                        // stretched across 760 px, which is the broken picture the operator
                        // reported. Making the desire authoritative here means a lost race
                        // costs one keeper tick instead of the entire session.
                        let desired = android.desired_view_preset(&device.udid);
                        let running = android.view_is_running(&device.udid, desired).await;
                        let wrong_preset = !running
                            && (android
                                .view_is_running(
                                    &device.udid,
                                    riviu_android_driver::ViewPreset::Tile,
                                )
                                .await
                                || android
                                    .view_is_running(
                                        &device.udid,
                                        riviu_android_driver::ViewPreset::Overlay,
                                    )
                                    .await);
                        if wrong_preset {
                            // A retune, not a failure, so it does not go through the fault
                            // backoff -- but it does need its own floor, or a phone that
                            // cannot encode the larger preset would be retuned every tick.
                            let record = view_retunes.get(&device.udid).copied();
                            if !crate::view_watchdog::view_restart_is_due(
                                record.map(|(_, n): (Instant, u32)| n).unwrap_or(0),
                                record.map(|(at, _)| at.elapsed()),
                                crate::view_watchdog::VIEW_RETUNE_FLOOR,
                                crate::view_watchdog::VIEW_RESTART_MAX_BACKOFF,
                            ) {
                                continue;
                            }
                            let attempts = record.map(|(_, n)| n).unwrap_or(0) + 1;
                            view_retunes.insert(device.udid.clone(), (Instant::now(), attempts));
                            log::info!(
                                "android view for {} is running the wrong preset; retuning to \
                                 {desired:?} (attempt {attempts})",
                                device.udid
                            );
                        } else if running {
                            view_retunes.remove(&device.udid);
                        }
                        // The merged decision. One predicate over both kinds of evidence --
                        // bytes off the wire, and frames the frontend says it drew -- so a
                        // device cannot be judged healthy by one rule and broken by another,
                        // which is what two independent watchdogs were doing to each other.
                        if running {
                            let paint = view_paint.sample(&device.udid);
                            let report_age =
                                paint.as_ref().map(|report| report.reported_at.elapsed());
                            let reporting = report_age
                                .map(|age| age < crate::view_watchdog::VIEW_PAINT_REPORT_STALE)
                                .unwrap_or(false);
                            let verdict = crate::view_watchdog::view_verdict(
                                false,
                                view_hub.last_packet_age(&device.udid),
                                paint.as_ref(),
                                report_age,
                            );
                            let fault = match verdict {
                                crate::view_watchdog::ViewVerdict::Restart(fault) => fault,
                                // Starting cannot occur here: in-flight starts were skipped
                                // above.
                                _ => {
                                    // Healthy clears the backoff, but only on the coarse
                                    // rule's terms. With nobody reporting paints, "bytes are
                                    // arriving" is the whole of what healthy can mean and
                                    // clearing on it is the keeper's original, safe
                                    // behaviour. With a live reporter it is NOT enough: a
                                    // stream that painted two frames after a restart and
                                    // stopped reads as healthy for one tick, and treating
                                    // that as recovery is exactly what made every stall log
                                    // "attempt 1" while the loop ran 33 times. In that regime
                                    // only sustained painting clears it, via `note_painted`.
                                    if !reporting {
                                        view_recovery.forget(&device.udid);
                                    }
                                    // Painting again means the next stall is a new problem
                                    // and gets the cheap cure first.
                                    view_recovery.clear_keyframe_ask(&device.udid);
                                    continue;
                                }
                            };
                            // Packets arriving and nothing painting is what a decoder that
                            // lost its GOP looks like, and a fresh IDR is exactly what such a
                            // decoder needs — so ask for one before tearing anything down.
                            // One byte against ~11.5 s of black tile.
                            if fault == crate::view_watchdog::ViewFault::PaintStalled {
                                match view_recovery.keyframe_step(
                                    &device.udid,
                                    crate::view_watchdog::VIEW_KEYFRAME_GRACE,
                                ) {
                                    crate::view_watchdog::KeyframeStep::Ask => {
                                        match android.request_keyframe(&device.udid).await {
                                            Ok(true) => log::info!(
                                                "android view for {} drew nothing; asked for a \
                                                 keyframe before restarting",
                                                device.udid
                                            ),
                                            Ok(false) => {}
                                            Err(error) => log::warn!(
                                                "could not ask {} for a keyframe: {error:#}",
                                                device.udid
                                            ),
                                        }
                                        continue;
                                    }
                                    // Asked recently. Let the cheap cure finish rather than
                                    // stacking a teardown on top of it.
                                    crate::view_watchdog::KeyframeStep::Wait => continue,
                                    crate::view_watchdog::KeyframeStep::Escalate => {}
                                }
                            }
                            // Capacity and backoff in one place, shared with every operator
                            // command. A refusal is not an error -- the next tick asks again,
                            // and the device keeps whatever tile state it already had rather
                            // than flickering into Sampling for work that is not happening.
                            // The frame count at the moment of the attempt, so "it recovered"
                            // can later mean SUSTAINED painting rather than a single frame.
                            let frames = paint.as_ref().map(|report| report.frames).unwrap_or(0);
                            let Some(permit) = view_recovery.try_admit(
                                &device.udid,
                                frames,
                                crate::view_watchdog::VIEW_RESTART_BACKOFF,
                                crate::view_watchdog::VIEW_RESTART_MAX_BACKOFF,
                            ) else {
                                continue;
                            };
                            // Name the cause, because for the first two weeks of this
                            // path there was only ever one line and it was the same
                            // whether the encoder had died or the phone had simply gone
                            // to sleep — and it is nearly always the second, which the
                            // restart now fixes by waking the screen. An anonymous
                            // "published nothing" sent the reader looking at scrcpy.
                            let display = match android.display_is_awake(&device.udid).await {
                                Some(true) => "display awake",
                                Some(false) => "display asleep",
                                None => "display state unreadable",
                            };
                            // The counters come from a Web Worker, which vite does not
                            // forward to the terminal (AGENTS.md 9.66) -- so printing them
                            // here, on the line that says what is being done about them, is
                            // the only place they are readable at all.
                            let evidence = paint
                                .as_ref()
                                .map(|report| {
                                    format!(
                                        " [gen={} received={} frames={} since_paint={}ms]",
                                        report.generation,
                                        report.received,
                                        report.frames,
                                        report.since_paint.as_millis()
                                    )
                                })
                                .unwrap_or_default();
                            log::warn!(
                                "android view for {} {} ({display}){evidence}; restarting \
                                 scrcpy ({} of {} recovery slots in use)",
                                device.udid,
                                fault.reason(),
                                view_recovery.in_flight(),
                                view_recovery.limit()
                            );
                            let android = android.clone();
                            let registry = registry.clone();
                            let view_paint = view_paint.clone();
                            let udid = device.udid.clone();
                            starts.push(tokio::spawn(async move {
                                let _ = crate::view_watchdog::restart_android_view(
                                    &android,
                                    &registry,
                                    &view_paint,
                                    &udid,
                                    permit,
                                )
                                .await;
                            }));
                            continue;
                        }
                        // Not running at all: a first start, not a recovery. It takes NO
                        // permit. Nothing is being torn down, so there is no working picture
                        // to risk -- and the bench says twenty concurrent clean starts cost
                        // no more per start than one does, while gating them made a cold
                        // start of the fleet take 55 s instead of 15 s (AGENTS.md 9.72).
                        // `view_start_in_flight`, checked above, is what stops two of these
                        // racing for the same device.
                        let android = android.clone();
                        let registry = registry.clone();
                        let udid = device.udid.clone();
                        starts.push(tokio::spawn(async move {
                            let _ = crate::view_watchdog::start_android_view(
                                &android, &registry, &udid,
                            )
                            .await;
                        }));
                    }
                    for start in starts {
                        let _ = start.await;
                    }
                }
            });
        }

        // One sampler owns the bounded background stream budget. It yields a
        // device after the first fresh frame or five seconds.
        let control = self.control.clone();
        let streams = self.streams.clone();
        let registry = self.registry.clone();
        let background_stop = self.background_stop.clone();
        let background_stopped = self.background_stopped.clone();
        let background_stopped_notify = self.background_stopped_notify.clone();
        let background_shutdown_error = self.background_shutdown_error.clone();
        let sweep_view_hub = self.view_hub.clone();
        tauri::async_runtime::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_millis(250));
            let mut last_scan = Instant::now() - Duration::from_secs(3);
            let mut sampler =
                BackgroundStreamSampler::new(control.clone(), streams, registry.clone());
            loop {
                interval.tick().await;
                if background_stop.load(Ordering::Acquire) {
                    if let Err(error) = sampler.stop().await {
                        *background_shutdown_error.write() = Some(error.to_string());
                    }
                    background_stopped.store(true, Ordering::Release);
                    background_stopped_notify.notify_one();
                    break;
                }
                if last_scan.elapsed() >= Duration::from_secs(3) {
                    last_scan = Instant::now();
                    if let Ok(devices) = control.list_devices().await {
                        // Preserve stream URLs / WDA flags from registry when PMD returns fresh list
                        let existing = registry.list();
                        let merged = devices
                            .into_iter()
                            .map(
                                |device| match existing.iter().find(|e| e.udid == device.udid) {
                                    Some(previous) => merge_scanned_device(device, previous),
                                    None => device,
                                },
                            )
                            .collect();
                        registry.upsert_many(merged);
                        // This scan is the only place a device DEPARTURE is observed —
                        // `upsert_many` replaces the roster wholesale, so nothing else is
                        // ever told a phone was unplugged. The hub needs to hear it now that
                        // each device owns a channel: without this it keeps one
                        // fully-allocated ring per udid it has ever seen, and the rings are
                        // allocated eagerly.
                        //
                        // Departure only, never a producer restart or a status flap: a phone
                        // whose view is merely being restarted is still there, and closing
                        // its channel would make every client tear down a canvas that is
                        // about to be repainted.
                        let live: std::collections::HashSet<String> = registry
                            .list()
                            .into_iter()
                            .map(|device| device.udid)
                            .collect();
                        for udid in sweep_view_hub.known_devices() {
                            if !live.contains(&udid) {
                                log::debug!("view hub forgetting {udid}: it left the fleet");
                                sweep_view_hub.forget(&udid);
                            }
                        }
                    }
                }

                let _ = sampler.tick().await;
            }
        });

        // Forward a bounded latest-frame preview to the UI. The raw StreamHub
        // feed stays lossless enough for automation; this path only keeps one
        // Arc per device and publishes JPEG **bytes** onto ViewHub — no
        // base64, no `<img src=data:...>`.
        let streams = self.streams.clone();
        let view_hub = self.view_hub.clone();
        tauri::async_runtime::spawn(async move {
            let mut rx = streams.subscribe();
            let mut ticker = tokio::time::interval(PREVIEW_TICK);
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            let mut latest = HashMap::<String, Frame>::new();
            let mut last_received = HashMap::<String, Instant>::new();
            let mut next_due = HashMap::<String, Instant>::new();
            let mut order = Vec::<String>::new();
            let mut cursor = 0usize;
            let mut last_eviction = Instant::now();
            loop {
                tokio::select! {
                    biased;
                    _ = ticker.tick() => {
                        let now = Instant::now();
                        // A stopped/parked producer should not consume a slot
                        // forever after its last frame. The frontend keeps its
                        // cached last image, so eviction only affects the
                        // scheduler's active-rate calculation.
                        if now.duration_since(last_eviction) >= Duration::from_secs(1) {
                            last_eviction = now;
                            order.retain(|udid| {
                                last_received.get(udid).is_some_and(|seen| {
                                    now.duration_since(*seen) <= PREVIEW_IDLE_EVICTION
                                })
                            });
                            latest.retain(|udid, _| order.iter().any(|id| id == udid));
                            next_due.retain(|udid, _| order.iter().any(|id| id == udid));
                            if !order.is_empty() {
                                cursor %= order.len();
                            } else {
                                cursor = 0;
                            }
                        }

                        let count = order.len();
                        if count == 0 {
                            continue;
                        }
                        let per_device_fps = preview_fps_for_device_count(count);
                        let gap = Duration::from_nanos(1_000_000_000u64 / per_device_fps as u64);
                        // Emit at most one frame per scheduler tick. The
                        // round-robin cursor prevents the first device from
                        // monopolising the global preview budget.
                        for _ in 0..count {
                            let index = cursor % count;
                            cursor = (cursor + 1) % count;
                            let udid = &order[index];
                            if next_due.get(udid).is_some_and(|due| *due > now) {
                                continue;
                            }
                            let Some(frame) = latest.get(udid).cloned() else {
                                continue;
                            };
                            next_due.insert(udid.clone(), now + gap);
                            view_hub.publish_jpeg(udid, frame.as_slice().to_vec());
                            break;
                        }
                    }
                    message = rx.recv() => match message {
                        Ok((udid, frame)) => {
                            let now = Instant::now();
                            if !latest.contains_key(&udid) {
                                order.push(udid.clone());
                                next_due.insert(udid.clone(), now);
                            }
                            latest.insert(udid.clone(), frame);
                            last_received.insert(udid, now);
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                        Err(_) => break,
                    }
                }
            }
        });

        // Forward app events
        let mut ev_rx = self.events.subscribe();
        let app_ev = app.clone();
        tauri::async_runtime::spawn(async move {
            loop {
                match ev_rx.recv().await {
                    Ok(event) => {
                        let _ = app_ev.emit("riviu://event", event);
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(_) => break,
                }
            }
        });

        // WDA expiry warnings
        let registry = self.registry.clone();
        let events = self.events.clone();
        tauri::async_runtime::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(60));
            loop {
                interval.tick().await;
                for d in registry.list() {
                    if let Some(exp) = d.wda_expires_at {
                        let days = (exp - chrono::Utc::now()).num_days();
                        if days <= 2 {
                            events.emit(riviu_core::AppEvent::WdaExpiryWarning {
                                udid: d.udid,
                                days_remaining: days,
                            });
                        }
                    }
                }
            }
        });

        // Idle popup sweep. Clears TikTok's onboarding pages and modals off phones nobody
        // is driving — see `crate::idle_sweeper` for why it can never compete with real
        // work, park a stream, or touch a phone that is not in TikTok.
        tauri::async_runtime::spawn(
            crate::idle_sweeper::IdleSweeper::new(
                self.control.clone(),
                self.registry.clone(),
                self.nurture.log(),
            )
            .run(),
        );

        // Local schedule runner
        let db = self.db.clone();
        let jobs = self.jobs.clone();
        let command_admission = self.command_admission.clone();
        tauri::async_runtime::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(30));
            loop {
                interval.tick().await;
                let Ok(schedules) = db.list_schedules() else {
                    continue;
                };
                let now = chrono::Utc::now();
                for mut s in schedules {
                    if !s.enabled {
                        continue;
                    }
                    let due = s
                        .next_run_at
                        .as_ref()
                        .and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok())
                        .map(|t| t.with_timezone(&chrono::Utc) <= now)
                        .unwrap_or(true);
                    if !due {
                        continue;
                    }
                    let Ok(_admission) = command_admission.ensure_accepting_work() else {
                        break;
                    };
                    // **Both failures used to fall through in silence, and the timestamps
                    // moved anyway.** A schedule whose script had been renamed or deleted
                    // advanced `last_run_at` on every tick while enqueueing nothing, so the
                    // page showed a job that had run two minutes ago and would run again in
                    // an hour — for as long as the app stayed open.
                    //
                    // `last_run_at` now marks a run that happened, and `last_error` says
                    // why one did not. `next_run_at` still advances either way: a schedule
                    // that stopped ticking would look paused, and it is not — it is trying
                    // once an interval and failing, which is a different thing.
                    let outcome = match db.get_script(&s.script_name) {
                        Ok(Some(body)) => match riviu_script_engine::parse_script(&body) {
                            Ok(script) => match jobs.enqueue(script, s.udids.clone()).await {
                                Ok(_) => Ok(()),
                                Err(error) => Err(format!("không xếp được tác vụ: {error}")),
                            },
                            Err(error) => Err(format!(
                                "kịch bản `{}` không đọc được: {error}",
                                s.script_name
                            )),
                        },
                        Ok(None) => Err(format!(
                            "không còn kịch bản tên `{}` — có thể đã bị xoá hoặc đổi tên",
                            s.script_name
                        )),
                        Err(error) => Err(format!(
                            "không đọc được kịch bản `{}`: {error}",
                            s.script_name
                        )),
                    };
                    match outcome {
                        Ok(()) => {
                            s.last_run_at = Some(now.to_rfc3339());
                            s.last_error = None;
                            let _ = db.log_op("schedule.run", &s.name);
                        }
                        Err(reason) => {
                            log::warn!("lịch `{}` không chạy được: {reason}", s.name);
                            let _ = db.log_op("schedule.failed", &format!("{}: {reason}", s.name));
                            s.last_error = Some(reason);
                        }
                    }
                    s.next_run_at = Some(
                        (now + chrono::Duration::minutes(s.every_minutes as i64)).to_rfc3339(),
                    );
                    let _ = db.upsert_schedule(&s);
                }
            }
        });

        // One-time publish schedules are intentionally conservative: opening
        // the desktop after the deadline marks the campaign missed instead
        // of surprise-posting. A campaign that is due while this process is
        // open runs the same transfer -> native composer -> frame verification
        // transaction as the manual Post button.
        let publish_db = self.db.clone();
        let publish_control = self.control.clone();
        let publish_frames = Arc::new(self.streams.clone());
        let publish_agent_bundle = self.active_agent_bundle_id.clone();
        let publish_admission = self.command_admission.clone();
        let publish_background_stop = self.background_stop.clone();
        let publish_started_at = chrono::Local::now().naive_local();
        tauri::async_runtime::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(15));
            loop {
                interval.tick().await;
                if publish_background_stop.load(Ordering::Acquire) {
                    break;
                }
                let Ok(campaigns) = publish_db.list_publish_campaigns(200) else {
                    continue;
                };
                let now = chrono::Local::now().naive_local();
                for campaign in campaigns {
                    if campaign.state != riviu_core::PublishCampaignState::Scheduled {
                        continue;
                    }
                    let Some(raw_run_at) = campaign.run_at.as_deref() else {
                        let _ = publish_db.update_publish_campaign_state(
                            &campaign.id,
                            riviu_core::PublishCampaignState::FailedBeforeDispatch,
                            Some("missing_run_at"),
                        );
                        continue;
                    };
                    let Ok(run_at) =
                        chrono::NaiveDateTime::parse_from_str(raw_run_at, "%Y-%m-%dT%H:%M")
                            .or_else(|_| {
                                chrono::NaiveDateTime::parse_from_str(
                                    raw_run_at,
                                    "%Y-%m-%dT%H:%M:%S",
                                )
                            })
                    else {
                        let _ = publish_db.update_publish_campaign_state(
                            &campaign.id,
                            riviu_core::PublishCampaignState::FailedBeforeDispatch,
                            Some("invalid_run_at"),
                        );
                        continue;
                    };
                    if run_at < publish_started_at {
                        let _ = publish_db.update_publish_campaign_state(
                            &campaign.id,
                            riviu_core::PublishCampaignState::Missed,
                            Some("app_opened_after_deadline"),
                        );
                        let _ = publish_db.log_op("publish.missed", &campaign.id);
                        continue;
                    }
                    if run_at > now {
                        continue;
                    }
                    let Ok(_admission) = publish_admission.ensure_accepting_work() else {
                        break;
                    };
                    if let Err(error) = crate::publish_commands::transfer_publish_campaign_inner(
                        publish_control.clone(),
                        publish_db.clone(),
                        publish_agent_bundle.clone(),
                        campaign.id.clone(),
                    )
                    .await
                    {
                        let _ = publish_db.log_op(
                            "publish.schedule.error",
                            &format!("{}: {error}", campaign.id),
                        );
                        continue;
                    }
                    if let Err(error) = crate::publish_commands::post_publish_campaign_inner(
                        publish_control.clone(),
                        publish_db.clone(),
                        publish_frames.clone(),
                        campaign.id.clone(),
                    )
                    .await
                    {
                        let _ = publish_db.log_op(
                            "publish.schedule.post_error",
                            &format!("{}: {error}", campaign.id),
                        );
                    }
                }
            }
        });

        // TikTok nurture schedule ticks
        let db = self.db.clone();
        let nurture = self.nurture.clone();
        let nurture_engine = self.nurture_engine.clone();
        let registry = self.registry.clone();
        let app_nurture = app.clone();
        let command_admission = self.command_admission.clone();
        let nurture_control = self.control.clone();
        tauri::async_runtime::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(30));
            loop {
                interval.tick().await;
                let Ok(settings) = db.get_nurture_settings() else {
                    continue;
                };
                if !settings.schedule_enabled {
                    continue;
                }
                let every = settings.schedule_every_minutes.max(1) as i64;
                let now = chrono::Utc::now();
                let due = match db.get_setting("nurture.schedule.next_run_at") {
                    Ok(Some(raw)) => chrono::DateTime::parse_from_rfc3339(&raw)
                        .map(|t| t.with_timezone(&chrono::Utc) <= now)
                        .unwrap_or(true),
                    _ => true,
                };
                if !due {
                    continue;
                }
                let Ok(_admission) = command_admission.ensure_accepting_work() else {
                    break;
                };
                let mut udids = settings.schedule_udids.clone();
                if udids.is_empty() {
                    udids = registry
                        .list()
                        .into_iter()
                        .filter(|d| {
                            !matches!(
                                d.status,
                                riviu_core::DeviceStatus::Disconnected
                                    | riviu_core::DeviceStatus::Error
                            )
                        })
                        .map(|d| d.udid)
                        .collect();
                }
                if udids.is_empty() {
                    let _ = db.set_setting(
                        "nurture.schedule.next_run_at",
                        &(now + chrono::Duration::minutes(every)).to_rfc3339(),
                    );
                    continue;
                }
                // **The same gate the manual start treats as mandatory.** This path went
                // straight to `start_many`, so a scheduled run began on phones whose text
                // agent was not ready and then failed every comment it tried, once an hour,
                // with nothing anywhere saying why. The manual button had refused those
                // phones outright; the schedule did not know to ask.
                let preflight = crate::nurture_commands::preflight_comment_job(
                    &nurture_control,
                    &udids,
                    &settings,
                )
                .await;
                if !preflight.skipped.is_empty() {
                    log::warn!(
                        "lịch nuôi TT bỏ qua {} máy: {}",
                        preflight.skipped.len(),
                        preflight.skipped.join("; ")
                    );
                    let _ = db.log_op("nurture.schedule.skipped", &preflight.skipped.join("; "));
                }
                if preflight.ready.is_empty() {
                    // Recorded rather than retried immediately: the next tick is an hour
                    // away and the reason is now written down, which is the difference
                    // between a schedule that is failing and one that looks idle.
                    let _ = db.log_op("nurture.schedule.blocked", &preflight.refusal());
                    let _ = db.set_setting(
                        "nurture.schedule.next_run_at",
                        &(now + chrono::Duration::minutes(every)).to_rfc3339(),
                    );
                    continue;
                }
                let duration =
                    Duration::from_secs(settings.schedule_duration_minutes.max(1) as u64 * 60);
                let started = nurture
                    .start_many(
                        app_nurture.clone(),
                        nurture_engine.clone(),
                        preflight.ready,
                        settings,
                        Some(duration),
                    )
                    .await;
                if !started.is_empty() {
                    let _ = db.log_op("nurture.schedule", &format!("{} devices", started.len()));
                }
                let _ = db.set_setting(
                    "nurture.schedule.next_run_at",
                    &(now + chrono::Duration::minutes(every)).to_rfc3339(),
                );
            }
        });
    }

    pub async fn shutdown_android_views(&self) {
        if let Some(android) = &self.android {
            android.stop_all_views().await;
        }
    }

    pub async fn shutdown_background_sampler(&self) -> anyhow::Result<()> {
        let stopped = self.background_stopped_notify.notified();
        self.background_stop.store(true, Ordering::Release);
        if !self.background_stopped.load(Ordering::Acquire) {
            stopped.await;
        }
        if let Some(error) = self.background_shutdown_error.read().clone() {
            anyhow::bail!("background stream sampler cleanup failed: {error}");
        }
        Ok(())
    }
}

fn resolve_desktop_data_dir(
    mock_requested: bool,
    mock_override: Option<PathBuf>,
) -> anyhow::Result<PathBuf> {
    if let Some(path) = mock_override {
        anyhow::ensure!(
            mock_requested,
            "RIVIU_MOCK_DATA_DIR requires RIVIU_MOCK_DEVICES=1"
        );
        anyhow::ensure!(path.is_absolute(), "RIVIU_MOCK_DATA_DIR must be absolute");
        return Ok(path);
    }
    Ok(dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("riviu-managers-phone"))
}

fn background_sample_candidate(
    device: &riviu_core::DeviceInfo,
    status: &riviu_core::AgentStatus,
) -> bool {
    if device.platform == riviu_core::DevicePlatform::Android {
        return false;
    }
    !matches!(
        device.status,
        riviu_core::DeviceStatus::Disconnected
            | riviu_core::DeviceStatus::Pairing
            | riviu_core::DeviceStatus::Preparing
            | riviu_core::DeviceStatus::Busy
    ) && (matches!(
        status.state,
        riviu_core::AgentState::Unknown
            | riviu_core::AgentState::Ready
            | riviu_core::AgentState::Error
    ) || (status.state == riviu_core::AgentState::Starting && device.wda_ready))
}

pub(crate) fn mark_android_view_live(registry: &DeviceRegistry, udid: &str) {
    set_stream_state(registry, udid, riviu_core::TileStreamState::Live, None);
}

/// A restart has been admitted and is under way.
///
/// Set only once a permit is actually held, never on the deferral path: a device the gate
/// declined has not started anything, and flickering its tile into Sampling would report
/// work that is not happening.
pub(crate) fn set_stream_sampling(registry: &DeviceRegistry, udid: &str) {
    set_stream_state(registry, udid, riviu_core::TileStreamState::Sampling, None);
}

pub(crate) fn set_stream_error(registry: &DeviceRegistry, udid: &str, error: String) {
    set_stream_state(
        registry,
        udid,
        riviu_core::TileStreamState::Error,
        Some(error),
    );
}

fn set_stream_parked(registry: &DeviceRegistry, udid: &str) {
    set_stream_state(registry, udid, riviu_core::TileStreamState::Parked, None);
}

/// Fold what only the registry knows into the device the three-second scan just read.
///
/// `list_devices` builds each `DeviceInfo` from scratch off the wire, so anything recorded
/// by another path — a stream URL, a WDA flag, the tile's stream state — is absent from it
/// and has to be carried, or `upsert_many` replacing the roster wholesale would erase it
/// every three seconds.
///
/// **`last_error` is carried only while the state that explains it is still `Error`**, and
/// that condition is the fix rather than a detail. It used to be carried whenever the fresh
/// scan had none, which made every error immortal: `probe_device` writes `None` on a
/// successful probe, so a phone that failed one adb call and answered every one after it
/// kept the sentence for the life of the process — painting a failure panel over live video.
///
/// The old rule also disagreed with itself. A stream failure sets `status = Error`, and
/// `status` is *not* carried here, so three seconds later the tile read "Ready" with an
/// error string beneath it. Tying the message to `tile_stream_state` keeps the two together:
/// a stream failure holds its reason for as long as it holds the state, and
/// `set_stream_state` clears both the moment it goes Live, Sampling or Parked. A probe
/// failure is not carried at all — if the probe is still failing this scan wrote its own
/// error, and if it is not, this scan just disproved the old one.
fn merge_scanned_device(
    mut device: riviu_core::DeviceInfo,
    previous: &riviu_core::DeviceInfo,
) -> riviu_core::DeviceInfo {
    // **Nothing is carried onto a device the scan could not reach.**
    //
    // Everything below this line exists to survive `list_devices` rebuilding each row from
    // scratch. None of it should survive the scan reporting the phone as unreachable: a
    // `Disconnected` row is what `unusable_device` produces for an adb state of `offline`
    // or `recovery`, and it is already complete and already honest — `wda_ready: false`, no
    // stream, and a sentence saying which cable to check.
    //
    // Carrying the old flags over it turned that into the opposite claim. `wda_ready` came
    // back as `true`, the grid derives readiness from `wdaReady || status == "ready"`, and a
    // phone in the middle of a forty-second reboot was painted green and counted in "N sẵn
    // sàng" while its own explanation sat in a field nothing displayed. The old tile
    // state came back with it, so the frame from before the reboot stayed on screen with no
    // overlay over it. This became reachable the moment offline devices stopped being
    // dropped from the roster — the fix that gave them a row is what made the lie visible.
    if matches!(device.status, riviu_core::DeviceStatus::Disconnected) {
        return device;
    }

    if device.stream_url.is_none() {
        device.stream_url = previous.stream_url.clone();
    }
    if !device.wda_ready {
        device.wda_ready = previous.wda_ready;
    }
    device.wda_expires_at = previous.wda_expires_at.or(device.wda_expires_at);
    if matches!(previous.status, riviu_core::DeviceStatus::Busy) {
        device.status = previous.status.clone();
    }
    device.tile_stream_state = previous.tile_stream_state;
    if device.last_error.is_none()
        && previous.tile_stream_state == riviu_core::TileStreamState::Error
    {
        device.last_error = previous.last_error.clone();
    }
    device
}

fn set_stream_state(
    registry: &DeviceRegistry,
    udid: &str,
    state: riviu_core::TileStreamState,
    error: Option<String>,
) {
    if let Some(mut device) = registry.get(udid) {
        device.tile_stream_state = state;
        if state == riviu_core::TileStreamState::Live {
            device.wda_ready = true;
        }
        if !matches!(
            state,
            riviu_core::TileStreamState::Live | riviu_core::TileStreamState::Sampling
        ) {
            device.stream_url = None;
        }
        if state == riviu_core::TileStreamState::Error {
            device.status = riviu_core::DeviceStatus::Error;
        } else if !matches!(
            device.status,
            riviu_core::DeviceStatus::Disconnected
                | riviu_core::DeviceStatus::Pairing
                | riviu_core::DeviceStatus::Preparing
                | riviu_core::DeviceStatus::Busy
        ) {
            device.status = riviu_core::DeviceStatus::Ready;
        }
        device.last_error = error;
        registry.upsert(device);
    }
}

fn resolve_sidecar_root(resource_dir: Option<&Path>) -> PathBuf {
    let configured = std::env::var_os("RIVIU_SIDECAR_ROOT").map(PathBuf::from);
    resolve_sidecar_root_from(
        configured,
        resource_dir,
        Path::new(env!("CARGO_MANIFEST_DIR")),
    )
}

fn resolve_sidecar_root_from(
    configured: Option<PathBuf>,
    resource_dir: Option<&Path>,
    manifest_dir: &Path,
) -> PathBuf {
    if let Some(configured) = configured {
        return configured;
    }
    if let Some(resource_dir) = resource_dir {
        let packaged = resource_dir.join("sidecars");
        if packaged
            .join("pymobiledevice3")
            .join("riviu_pmd.py")
            .is_file()
        {
            return packaged;
        }
    }
    // Dev: repo sidecars/ relative to CARGO_MANIFEST_DIR (apps/desktop/src-tauri)
    manifest_dir
        .join("../../..")
        .join("sidecars")
        .canonicalize()
        .unwrap_or_else(|_| manifest_dir.join("../../../sidecars"))
}

#[cfg(test)]
mod tests {

    use super::*;

    #[test]
    fn the_stream_budget_follows_the_fleet_that_is_plugged_in() {
        // The constant this replaced was 2, described as "the desktop shows at most two
        // tiles". Every nurture session holds a foreground slot for its whole run, so on a
        // twenty-phone farm that constant refused eighteen of twenty sessions outright.
        assert_eq!(desktop_stream_budget(20).configured_limit(), 20);
        assert_eq!(desktop_stream_budget(6).configured_limit(), 6);
        // A two-phone bench is unchanged, and a fleet of zero -- nothing plugged in yet, or
        // a scan that failed -- does not reserve for phones that may never arrive.
        assert_eq!(desktop_stream_budget(2).configured_limit(), 2);
        assert_eq!(
            desktop_stream_budget(0).configured_limit(),
            DEFAULT_DESKTOP_STREAM_CAPACITY
        );
    }

    #[test]
    fn preview_budget_is_smooth_for_small_fleet_and_bounded_for_large_fleet() {
        assert_eq!(preview_fps_for_device_count(0), 24);
        assert_eq!(preview_fps_for_device_count(2), 24);
        assert_eq!(preview_fps_for_device_count(20), 12);
        assert_eq!(preview_fps_for_device_count(100), 2);
    }

    #[test]
    fn mock_data_override_is_absolute_and_never_available_to_real_devices() {
        let absolute = std::env::temp_dir().join("riviu-mock-data-dir-fixture");
        assert_eq!(
            resolve_desktop_data_dir(true, Some(absolute.clone()))
                .expect("mock override is accepted"),
            absolute
        );

        let production_error = resolve_desktop_data_dir(false, Some(std::env::temp_dir()))
            .expect_err("real-device startup rejects the override");
        assert!(production_error
            .to_string()
            .contains("requires RIVIU_MOCK_DEVICES=1"));

        let relative_error = resolve_desktop_data_dir(true, Some(PathBuf::from("fixture")))
            .expect_err("relative override is rejected");
        assert!(relative_error.to_string().contains("must be absolute"));
    }

    #[tokio::test]
    async fn command_admission_drains_winner_and_rejects_shutdown_contender() {
        let admission = Arc::new(CommandAdmissionState::new(false));
        let startup_error = match admission.ensure_accepting_work() {
            Ok(_) => panic!("startup must reject mutating commands"),
            Err(error) => error,
        };
        assert_eq!(startup_error.code, "ApplicationShuttingDown");

        admission.start_accepting();
        let winner = admission
            .ensure_accepting_work()
            .expect("admit command before shutdown");
        admission.reject_new_work();

        let contender = match admission.ensure_accepting_work() {
            Ok(_) => panic!("shutdown contender must be rejected"),
            Err(error) => error,
        };
        assert_eq!(contender.code, "ApplicationShuttingDown");

        let drain_state = admission.clone();
        let mut drain = tokio::spawn(async move { drain_state.wait_until_drained().await });
        tokio::task::yield_now().await;
        assert!(!drain.is_finished());

        drop(winner);
        tokio::time::timeout(Duration::from_secs(1), &mut drain)
            .await
            .expect("drain waiter completed")
            .expect("join drain waiter");
    }

    #[tokio::test]
    async fn shutdown_signal_releases_an_admitted_retry_before_admission_drain() {
        let admission = Arc::new(CommandAdmissionState::new(true));
        let permit = admission
            .ensure_accepting_work()
            .expect("admit retry before shutdown");
        let (stop_sender, stop_receiver) = tokio::sync::oneshot::channel();
        let retry = tokio::spawn(async move {
            let _permit = permit;
            stop_receiver.await.expect("runtime stop signal");
        });

        admission.reject_new_work();
        stop_sender.send(()).expect("signal runtime before drain");
        tokio::time::timeout(Duration::from_secs(1), admission.wait_until_drained())
            .await
            .expect("admitted retry released after runtime stop");
        retry.await.expect("join admitted retry");
    }

    #[test]
    fn flow_mutation_coordinator_emits_commits_in_strict_revision_order() {
        const WRITERS: usize = 8;
        let coordinator = Arc::new(FlowMutationCoordinator::default());
        let events = EventBus::new(WRITERS);
        let mut receiver = events.subscribe();
        let barrier = Arc::new(std::sync::Barrier::new(WRITERS));
        let handles = (0..WRITERS)
            .map(|_| {
                let coordinator = Arc::clone(&coordinator);
                let events = events.clone();
                let barrier = Arc::clone(&barrier);
                std::thread::spawn(move || {
                    barrier.wait();
                    coordinator
                        .commit(&events, || Ok::<_, ()>(((), uuid::Uuid::nil())))
                        .expect("commit Flow mutation")
                })
            })
            .collect::<Vec<_>>();
        for handle in handles {
            handle.join().expect("join Flow mutation writer");
        }

        let emitted = (0..WRITERS)
            .map(
                |_| match receiver.try_recv().expect("Flow mutation event") {
                    AppEvent::FlowUpdated { revision, .. } => revision,
                    _ => panic!("unexpected event kind"),
                },
            )
            .collect::<Vec<_>>();
        assert_eq!(emitted, (1..=WRITERS as u64).collect::<Vec<_>>());
    }

    async fn sampler_fixture() -> (
        riviu_ios_driver::MockIosDriver,
        Arc<DeviceControlPlane>,
        DeviceRegistry,
        BackgroundStreamSampler,
    ) {
        sampler_fixture_with_limit(1).await
    }

    async fn sampler_fixture_with_limit(
        limit: usize,
    ) -> (
        riviu_ios_driver::MockIosDriver,
        Arc<DeviceControlPlane>,
        DeviceRegistry,
        BackgroundStreamSampler,
    ) {
        let driver = riviu_ios_driver::MockIosDriver::new();
        let streams = driver.stream_hub();
        let control = Arc::new(DeviceControlPlane::new(
            Arc::new(driver.clone()),
            Arc::new(DeviceWorkCoordinator::new()),
            Arc::new(StreamBudgetManager::new(limit).expect("valid stream capacity")),
        ));
        let registry = DeviceRegistry::new(EventBus::new(32));
        registry.upsert_many(control.list_devices().await.expect("list mock devices"));
        let sampler = BackgroundStreamSampler::new(control.clone(), streams, registry.clone());
        (driver, control, registry, sampler)
    }

    #[tokio::test]
    async fn background_stream_sampler_keeps_a_live_producer_for_its_turn() {
        let (driver, control, registry, mut sampler) = sampler_fixture().await;
        assert_eq!(driver.stream_restart_calls(), 0);

        assert_eq!(
            sampler.tick().await,
            SamplerTick::Started("MOCK-IPHONE-01".to_string())
        );
        assert_eq!(control.reserved_stream_capacity(), 1);
        assert_eq!(driver.stream_restart_calls(), 1);
        assert_eq!(
            registry
                .get("MOCK-IPHONE-01")
                .expect("first tile")
                .tile_stream_state,
            riviu_core::TileStreamState::Sampling
        );

        assert_eq!(
            sampler.tick().await,
            SamplerTick::Sampling("MOCK-IPHONE-01".to_string())
        );
        assert_eq!(control.reserved_stream_capacity(), 1);
        assert_eq!(
            registry
                .get("MOCK-IPHONE-01")
                .expect("first tile remains live")
                .tile_stream_state,
            riviu_core::TileStreamState::Live
        );

        sampler.stop().await.expect("stop sampler producer");
        assert_eq!(control.reserved_stream_capacity(), 0);
        control.shutdown_cleanup().await.expect("shutdown control");
    }

    #[tokio::test]
    async fn background_stream_sampler_keeps_both_desktop_tiles_live() {
        let (driver, control, registry, mut sampler) = sampler_fixture_with_limit(2).await;

        assert_eq!(
            sampler.tick().await,
            SamplerTick::Started("MOCK-IPHONE-01".to_string())
        );
        assert_eq!(control.reserved_stream_capacity(), 2);
        assert_eq!(driver.stream_restart_calls(), 2);

        assert!(matches!(sampler.tick().await, SamplerTick::Sampling(_)));
        for udid in ["MOCK-IPHONE-01", "MOCK-IPHONE-02"] {
            assert_eq!(
                registry.get(udid).expect("desktop tile").tile_stream_state,
                riviu_core::TileStreamState::Live
            );
        }
        tokio::time::sleep(Duration::from_secs(6)).await;
        assert!(matches!(sampler.tick().await, SamplerTick::Sampling(_)));
        assert_eq!(control.reserved_stream_capacity(), 2);

        sampler.stop().await.expect("stop desktop producers");
        assert_eq!(control.reserved_stream_capacity(), 0);
        control.shutdown_cleanup().await.expect("shutdown control");
    }

    #[tokio::test]
    async fn background_stream_sampler_does_not_recycle_at_a_healthy_turn_boundary() {
        let (driver, control, _registry, mut sampler) = sampler_fixture_with_limit(2).await;
        driver.set_mock_stream_static("MOCK-IPHONE-01", true);
        driver.set_mock_stream_static("MOCK-IPHONE-02", true);

        assert!(matches!(sampler.tick().await, SamplerTick::Started(_)));
        assert_eq!(driver.stream_restart_calls(), 2);

        // Move beyond the five-second bookkeeping deadline, then model a
        // frame observed just before the sampler tick. The producer is quiet
        // at this exact instant, but it is not stalled and must remain live.
        tokio::time::sleep(Duration::from_secs(5) + Duration::from_millis(25)).await;
        for sample in &mut sampler.active {
            sample.last_frame_at = Instant::now();
        }
        assert!(matches!(sampler.tick().await, SamplerTick::Sampling(_)));
        assert_eq!(driver.stream_restart_calls(), 2);

        sampler.stop().await.expect("stop desktop producers");
        control.shutdown_cleanup().await.expect("shutdown control");
    }

    #[tokio::test]
    async fn background_stream_sampler_does_not_reopen_during_foreground_preemption() {
        let (driver, control, registry, mut sampler) = sampler_fixture().await;
        assert!(matches!(sampler.tick().await, SamplerTick::Started(_)));

        let exclusive = control
            .try_acquire_exclusive("MOCK-IPHONE-02", riviu_core::DeviceWorkOwner::Interaction)
            .await
            .expect("foreground owner");
        let (exclusive, capacity) = control
            .reserve_ui_capacity(exclusive)
            .await
            .expect("preempt background producer");

        assert_eq!(
            sampler.tick().await,
            SamplerTick::Preempted("MOCK-IPHONE-01".to_string())
        );
        assert_eq!(driver.stream_restart_calls(), 1);
        assert_eq!(
            registry
                .get("MOCK-IPHONE-01")
                .expect("preempted tile")
                .tile_stream_state,
            riviu_core::TileStreamState::Parked
        );
        assert!(matches!(
            control.reserve_background_stream("MOCK-IPHONE-02"),
            Err(riviu_core::DeviceControlError::BackgroundStreamBlocked { .. })
        ));

        drop(capacity);
        drop(exclusive);
        control.shutdown_cleanup().await.expect("shutdown control");
    }

    #[tokio::test]
    async fn background_stream_sampler_backs_off_a_failed_device_for_thirty_seconds() {
        let (driver, control, registry, mut sampler) = sampler_fixture().await;
        driver.set_mock_stream_failure("MOCK-IPHONE-01", true);

        assert_eq!(
            sampler.tick().await,
            SamplerTick::Failed("MOCK-IPHONE-01".to_string())
        );
        assert!(matches!(
            control.reserve_background_stream("MOCK-IPHONE-01"),
            Err(riviu_core::DeviceControlError::StreamBudget(
                riviu_core::StreamBudgetError::FailedBackoff { remaining, .. }
            )) if remaining <= Duration::from_secs(30) && remaining > Duration::from_secs(29)
        ));
        assert_eq!(
            registry
                .get("MOCK-IPHONE-01")
                .expect("failed tile")
                .tile_stream_state,
            riviu_core::TileStreamState::Error
        );

        assert_eq!(
            sampler.tick().await,
            SamplerTick::Started("MOCK-IPHONE-02".to_string())
        );
        sampler.stop().await.expect("stop second producer");
        control.shutdown_cleanup().await.expect("shutdown control");
    }

    #[tokio::test]
    async fn background_stream_sampler_marks_a_five_second_turn_without_a_new_frame_stale() {
        let (driver, control, registry, mut sampler) = sampler_fixture().await;
        driver.set_mock_stream_static("MOCK-IPHONE-01", true);
        assert!(matches!(sampler.tick().await, SamplerTick::Started(_)));
        let sample = sampler.active.first_mut().expect("active sampler turn");
        sample.baseline_sequence = sampler.streams.latest_frame_sequence(sample.lease.udid());
        // The bounded turn starts when the producer is marked running, not
        // when its reservation is created; include a full turn after the
        // first frame so slow bootstrap cannot make this assertion early.
        tokio::time::sleep(Duration::from_secs(5) + Duration::from_millis(25)).await;

        assert_eq!(
            sampler.tick().await,
            SamplerTick::Stale("MOCK-IPHONE-01".to_string())
        );
        assert_eq!(control.reserved_stream_capacity(), 0);
        assert_eq!(
            registry
                .get("MOCK-IPHONE-01")
                .expect("stale tile")
                .tile_stream_state,
            riviu_core::TileStreamState::Stale
        );
        control.shutdown_cleanup().await.expect("shutdown control");
    }

    #[test]
    fn background_stream_sampler_skips_busy_and_repair_states() {
        let mut device = riviu_core::DeviceInfo {
            udid: "fixture".to_string(),
            name: "fixture".to_string(),
            model: "fixture".to_string(),
            platform: riviu_core::DevicePlatform::Ios,
            os_version: "fixture".to_string(),
            connection: riviu_core::ConnectionKind::Mock,
            status: riviu_core::DeviceStatus::Ready,
            battery: None,
            wda_ready: true,
            wda_expires_at: None,
            stream_url: None,
            tile_stream_state: riviu_core::TileStreamState::Parked,
            last_error: None,
        };
        let mut status = riviu_core::AgentStatus::unknown("fixture");
        assert!(background_sample_candidate(&device, &status));

        for state in [
            riviu_core::AgentState::Missing,
            riviu_core::AgentState::RepairRequired,
        ] {
            status.state = state;
            assert!(!background_sample_candidate(&device, &status));
        }

        status.state = riviu_core::AgentState::Starting;
        assert!(background_sample_candidate(&device, &status));
        device.wda_ready = false;
        assert!(!background_sample_candidate(&device, &status));
        device.wda_ready = true;

        status.state = riviu_core::AgentState::Error;
        assert!(background_sample_candidate(&device, &status));

        status.state = riviu_core::AgentState::Ready;
        assert!(background_sample_candidate(&device, &status));
        device.status = riviu_core::DeviceStatus::Preparing;
        assert!(!background_sample_candidate(&device, &status));
        device.status = riviu_core::DeviceStatus::Busy;
        assert!(!background_sample_candidate(&device, &status));

        device.platform = riviu_core::DevicePlatform::Android;
        device.status = riviu_core::DeviceStatus::Ready;
        status.state = riviu_core::AgentState::Ready;
        assert!(
            !background_sample_candidate(&device, &status),
            "Android tiles must not take a minicap budget slot"
        );
    }

    fn scanned(udid: &str) -> riviu_core::DeviceInfo {
        riviu_core::DeviceInfo {
            udid: udid.to_string(),
            name: "fixture".to_string(),
            model: "fixture".to_string(),
            platform: riviu_core::DevicePlatform::Android,
            os_version: "9".to_string(),
            connection: riviu_core::ConnectionKind::Usb,
            status: riviu_core::DeviceStatus::Connected,
            battery: None,
            wda_ready: false,
            wda_expires_at: None,
            stream_url: None,
            tile_stream_state: riviu_core::TileStreamState::default(),
            last_error: None,
        }
    }

    #[test]
    fn a_phone_that_stopped_answering_is_not_still_called_ready() {
        // The reboot case, which is the common one: a phone that was Ready is restarted
        // from the tile menu and spends about forty seconds in adb `offline`. The scan
        // builds an honest row for it. The merge used to hand back the readiness flag from
        // before the reboot, and the grid shows a green "Running" chip for anything with
        // `wda_ready` — over a phone that cannot answer a single call.
        let mut previous = scanned("fixture");
        previous.wda_ready = true;
        previous.status = riviu_core::DeviceStatus::Ready;
        previous.stream_url = Some("http://127.0.0.1:9100/stream".to_string());
        previous.tile_stream_state = riviu_core::TileStreamState::Live;

        let mut offline = scanned("fixture");
        offline.status = riviu_core::DeviceStatus::Disconnected;
        offline.last_error = Some("adb sees this device but it is not answering".to_string());

        let merged = merge_scanned_device(offline, &previous);

        assert!(!merged.wda_ready, "an unreachable phone is not ready");
        assert_eq!(merged.status, riviu_core::DeviceStatus::Disconnected);
        assert_eq!(
            merged.stream_url, None,
            "a stream URL for a phone that is gone is an address nothing answers"
        );
        assert_ne!(
            merged.tile_stream_state,
            riviu_core::TileStreamState::Live,
            "the tile must not keep claiming live video off a phone that stopped answering"
        );
        assert!(
            merged.last_error.is_some(),
            "the reason the scan wrote is the one thing that must survive"
        );
    }

    #[test]
    fn a_phone_that_is_merely_between_probes_keeps_what_it_earned() {
        // The other direction, so the guard above cannot be widened by accident: an
        // ordinary scan carries the flags that no scan can see, which is the whole reason
        // this function exists.
        let mut previous = scanned("fixture");
        previous.wda_ready = true;
        previous.stream_url = Some("http://127.0.0.1:9100/stream".to_string());

        let merged = merge_scanned_device(scanned("fixture"), &previous);

        assert!(merged.wda_ready);
        assert_eq!(
            merged.stream_url.as_deref(),
            Some("http://127.0.0.1:9100/stream")
        );
    }

    #[test]
    fn a_failure_the_scan_disproved_stops_being_shown() {
        // One failed adb probe used to mark a phone forever. `probe_device` writes `None`
        // on every later success, and the merge put the old sentence straight back, so the
        // tile kept a failure panel over live video until the app was restarted. Three
        // seconds is the interval; "for the life of the process" was the duration.
        let mut previous = scanned("fixture");
        previous.last_error = Some("adb -s fixture shell timed out".to_string());
        previous.status = riviu_core::DeviceStatus::Error;

        let merged = merge_scanned_device(scanned("fixture"), &previous);

        assert_eq!(merged.last_error, None);
    }

    #[test]
    fn a_failure_that_is_still_true_keeps_its_reason() {
        // Two ways it stays. The scan finding its own fault is the obvious one; a stream
        // that is sitting in `Error` is the one the old rule got wrong in the other
        // direction -- it kept the message but dropped `status`, so the tile read "Ready"
        // with an error underneath it.
        let mut still_failing = scanned("fixture");
        still_failing.last_error = Some("adb -s fixture shell timed out".to_string());
        let merged = merge_scanned_device(still_failing, &scanned("fixture"));
        assert_eq!(
            merged.last_error.as_deref(),
            Some("adb -s fixture shell timed out")
        );

        let mut stream_failed = scanned("fixture");
        stream_failed.tile_stream_state = riviu_core::TileStreamState::Error;
        stream_failed.last_error = Some("no fresh frame".to_string());
        let merged = merge_scanned_device(scanned("fixture"), &stream_failed);
        assert_eq!(merged.last_error.as_deref(), Some("no fresh frame"));
        assert_eq!(
            merged.tile_stream_state,
            riviu_core::TileStreamState::Error,
            "the reason and the state it explains have to travel together"
        );
    }

    #[test]
    fn a_live_stream_clears_the_reason_along_with_the_state() {
        // `set_stream_state` is the other half: it writes the state and the reason in one
        // move, so the moment a producer goes live the merge has nothing left to carry.
        let events = riviu_core::EventBus::new(8);
        let registry = riviu_core::DeviceRegistry::new(events);
        let mut failed = scanned("fixture");
        failed.tile_stream_state = riviu_core::TileStreamState::Error;
        failed.last_error = Some("no fresh frame".to_string());
        registry.upsert(failed);

        mark_android_view_live(&registry, "fixture");

        let recovered = registry.get("fixture").expect("device");
        assert_eq!(
            recovered.tile_stream_state,
            riviu_core::TileStreamState::Live
        );
        assert_eq!(recovered.last_error, None);
        assert_eq!(
            merge_scanned_device(scanned("fixture"), &recovered).last_error,
            None
        );
    }

    #[test]
    fn background_stream_state_keeps_cached_frame_url_only_while_sampling() {
        let events = riviu_core::EventBus::new(8);
        let registry = riviu_core::DeviceRegistry::new(events);
        let mut device = riviu_core::DeviceInfo {
            udid: "fixture-state".to_string(),
            name: "fixture".to_string(),
            model: "fixture".to_string(),
            platform: riviu_core::DevicePlatform::Ios,
            os_version: "fixture".to_string(),
            connection: riviu_core::ConnectionKind::Mock,
            status: riviu_core::DeviceStatus::Ready,
            battery: None,
            wda_ready: true,
            wda_expires_at: None,
            stream_url: Some("mock://fixture-state".to_string()),
            tile_stream_state: riviu_core::TileStreamState::Live,
            last_error: None,
        };
        registry.upsert(device.clone());

        set_stream_state(
            &registry,
            &device.udid,
            riviu_core::TileStreamState::Sampling,
            None,
        );
        device = registry.get(&device.udid).expect("sampling device");
        assert_eq!(
            device.tile_stream_state,
            riviu_core::TileStreamState::Sampling
        );
        assert_eq!(device.stream_url.as_deref(), Some("mock://fixture-state"));

        set_stream_state(
            &registry,
            &device.udid,
            riviu_core::TileStreamState::Stale,
            Some("no fresh frame".to_string()),
        );
        device = registry.get(&device.udid).expect("stale device");
        assert_eq!(device.tile_stream_state, riviu_core::TileStreamState::Stale);
        assert_eq!(device.stream_url, None);
        assert_eq!(device.status, riviu_core::DeviceStatus::Ready);
        assert_eq!(device.last_error.as_deref(), Some("no fresh frame"));
    }

    #[test]
    fn live_stream_marks_agent_ready_after_first_frame() {
        let events = riviu_core::EventBus::new(8);
        let registry = riviu_core::DeviceRegistry::new(events);
        registry.upsert(riviu_core::DeviceInfo {
            udid: "fixture-ready".to_string(),
            name: "fixture".to_string(),
            model: "fixture".to_string(),
            platform: riviu_core::DevicePlatform::Ios,
            os_version: "fixture".to_string(),
            connection: riviu_core::ConnectionKind::Mock,
            status: riviu_core::DeviceStatus::Connected,
            battery: None,
            wda_ready: false,
            wda_expires_at: None,
            stream_url: None,
            tile_stream_state: riviu_core::TileStreamState::Sampling,
            last_error: None,
        });

        set_stream_state(
            &registry,
            "fixture-ready",
            riviu_core::TileStreamState::Live,
            None,
        );

        assert!(
            registry
                .get("fixture-ready")
                .expect("live device")
                .wda_ready
        );
    }

    #[test]
    fn packaged_app_resolves_sidecars_from_runtime_resource_dir() {
        let resource_dir = std::env::temp_dir().join(format!(
            "riviu-sidecar-resource-test-{}",
            uuid::Uuid::new_v4()
        ));
        let packaged_sidecars = resource_dir.join("sidecars");
        let pmd_dir = packaged_sidecars.join("pymobiledevice3");
        std::fs::create_dir_all(&pmd_dir).expect("create packaged sidecar fixture");
        std::fs::write(pmd_dir.join("riviu_pmd.py"), b"# fixture\n")
            .expect("write packaged sidecar fixture");

        let actual = resolve_sidecar_root_from(
            None,
            Some(&resource_dir),
            Path::new(env!("CARGO_MANIFEST_DIR")),
        );
        std::fs::remove_dir_all(&resource_dir).expect("remove packaged sidecar fixture");

        assert_eq!(actual, packaged_sidecars);
    }

    #[test]
    fn development_app_falls_back_to_repo_sidecars() {
        let missing_resource_dir = std::env::temp_dir().join(format!(
            "riviu-missing-resource-test-{}",
            uuid::Uuid::new_v4()
        ));
        let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
        let expected = manifest_dir
            .join("../../..")
            .join("sidecars")
            .canonicalize()
            .expect("repo sidecars directory");

        let actual = resolve_sidecar_root_from(None, Some(&missing_resource_dir), manifest_dir);

        assert_eq!(actual, expected);
    }

    #[test]
    fn tauri_resources_map_to_clean_sidecar_layout() {
        let config: serde_json::Value =
            serde_json::from_str(include_str!("../tauri.conf.json")).expect("valid Tauri config");
        let resources = config["bundle"]["resources"]
            .as_object()
            .expect("bundle.resources must be a source-to-target map");

        let expected = [
            (
                "../../../sidecars/pymobiledevice3/requirements.txt",
                "sidecars/pymobiledevice3/requirements.txt",
            ),
            (
                "../../../sidecars/pymobiledevice3/requirements-lock.txt",
                "sidecars/pymobiledevice3/requirements-lock.txt",
            ),
            (
                "../../../sidecars/pymobiledevice3/riviu_pmd.py",
                "sidecars/pymobiledevice3/riviu_pmd.py",
            ),
            (
                "../../../sidecars/signer/requirements.txt",
                "sidecars/signer/requirements.txt",
            ),
            (
                "../../../sidecars/signer/riviu_signer.py",
                "sidecars/signer/riviu_signer.py",
            ),
            ("../../../logo.jpg", "sidecars/wda/logo.jpg"),
            (
                "../../../sidecars/wda/AppIcon.appiconset/",
                "sidecars/wda/AppIcon.appiconset/",
            ),
            (
                "../../../sidecars/wda/WebDriverAgent/",
                "sidecars/wda/WebDriverAgent/",
            ),
            (
                "../../../sidecars/wda/build_and_install.py",
                "sidecars/wda/build_and_install.py",
            ),
            (
                "../../../sidecars/wda/legacy-wda-source-lock.json",
                "sidecars/wda/legacy-wda-source-lock.json",
            ),
            (
                "../../../sidecars/wda/agent-manifest.json",
                "sidecars/wda/agent-manifest.json",
            ),
            (
                "../../../sidecars/wda/candidate-manifest.json",
                "sidecars/wda/candidate-manifest.json",
            ),
            (
                "../../../sidecars/wda/text-manifest.json",
                "sidecars/wda/text-manifest.json",
            ),
            (
                "../../../sidecars/wda/interaction-capabilities.json",
                "sidecars/wda/interaction-capabilities.json",
            ),
            (
                "../../../sidecars/wda/interaction-capabilities.schema.json",
                "sidecars/wda/interaction-capabilities.schema.json",
            ),
            (
                "../../../sidecars/wda/interaction_vision_ocr.swift",
                "sidecars/wda/interaction_vision_ocr.swift",
            ),
            (
                "../../../sidecars/wda/RiviuAgent.ipa",
                "sidecars/wda/RiviuAgent.ipa",
            ),
            (
                "../../../sidecars/wda/RiviuAgent-candidate.ipa",
                "sidecars/wda/RiviuAgent-candidate.ipa",
            ),
            (
                "../../../sidecars/wda/RiviuAgent-text.ipa",
                "sidecars/wda/RiviuAgent-text.ipa",
            ),
            (
                "../../../sidecars/wda/Riviumanagersphone.ipa",
                "sidecars/wda/Riviumanagersphone.ipa",
            ),
            // One tree entry, not one per file. A tree is what gives *completeness*:
            // `assert_same_tree` in the CI collector compares the whole directory, so a
            // sixth bundled tool cannot appear unchecked. Five separate file entries would
            // give five assertions and no such property.
            ("../../../sidecars/android/", "sidecars/android/"),
        ];

        assert_eq!(resources.len(), expected.len());
        for (source, target) in expected {
            assert_eq!(resources.get(source).and_then(|v| v.as_str()), Some(target));
            assert!(!target.contains("_up_"));
        }

        // Every resource lands under `sidecars/`, which is the whole point of the
        // source-to-target map: Tauri's default for a `../..` source is a mangled `_up_`
        // path, and `resolve_sidecar_root` only knows how to find things under this one
        // prefix. A target that escaped it would be shipped and never found.
        for (source, target) in resources {
            let target = target
                .as_str()
                .unwrap_or_else(|| panic!("target for {source} must be a string"));
            assert!(
                target.starts_with("sidecars/"),
                "{source} -> {target} escapes the sidecar root"
            );
        }
    }
}
