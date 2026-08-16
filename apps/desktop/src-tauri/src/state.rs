use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Context;
use parking_lot::{Mutex, RwLock};
use riviu_core::db::Database;
use riviu_core::{
    AppEvent, BackgroundStreamLease, DeviceControlPlane, DeviceRegistry, DeviceWorkCoordinator,
    DeviceWorkOwner, EventBus, FlowArtifactStore, FlowId, FlowRuntime, FlowRuntimeDeps, Frame,
    JobQueue, JobStatus, NurtureEngine, StreamBudgetManager, StreamSettings, UiSession,
    UiSessionContext, STREAM_FPS,
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
fn configured_desktop_stream_capacity() -> usize {
    match std::env::var("RIVIU_STREAM_CAPACITY") {
        Ok(raw) => match raw.trim().parse::<usize>() {
            Ok(value) if (1..=MAX_DESKTOP_STREAM_CAPACITY).contains(&value) => value,
            _ => {
                log::warn!(
                    "invalid RIVIU_STREAM_CAPACITY={raw:?}; using default {DEFAULT_DESKTOP_STREAM_CAPACITY}"
                );
                DEFAULT_DESKTOP_STREAM_CAPACITY
            }
        },
        Err(_) => DEFAULT_DESKTOP_STREAM_CAPACITY,
    }
}

/// Build the stream budget, falling back rather than panicking.
///
/// `configured_desktop_stream_capacity` accepts up to 100 but the budget
/// manager hard-caps concurrent producers at 2 (AGENTS.md 3.5/3.12), so
/// `RIVIU_STREAM_CAPACITY=3` used to panic the app at startup through an
/// `expect`. The env var's own contract is to fail closed to the default, and
/// that is what a farm-sized value gets now, with the reason logged.
fn desktop_stream_budget() -> StreamBudgetManager {
    let requested = configured_desktop_stream_capacity();
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
    overlay_sessions: AsyncMutex<HashMap<String, UiSessionContext>>,
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

        let db = Arc::new(Database::open(data.join("riviu.db"))?);
        let sidecar_root = resolve_sidecar_root(resource_dir.as_deref());
        let credentials = CredentialStore::system()?;
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
        let (android, android_unavailable) =
            match riviu_android_driver::detect_driver(&android_config).await {
                Ok(driver) => {
                    // JPEG evidence still lands in StreamHub. H.264 view samples
                    // go to ViewHub and never become a Frame.
                    driver.set_frame_sink(Arc::new(bundle.streams.clone()));
                    driver.set_view_sink(
                        Arc::clone(&view_hub) as Arc<dyn riviu_android_driver::ViewSink>
                    );
                    backends.push(("android".to_string(), driver.clone()));
                    (Some(driver), None)
                }
                Err(reason) => (None, Some(reason)),
            };
        let fleet: Arc<dyn riviu_core::driver::DeviceDriver> =
            Arc::new(riviu_core::driver_multiplex::MultiplexDriver::new(backends));

        let control = Arc::new(DeviceControlPlane::new_with_capability_registry(
            fleet,
            Arc::new(DeviceWorkCoordinator::new()),
            // Keep both connected phone tiles live. Core flow fixtures retain
            // the conservative default; the desktop has two USB devices in
            // its live fleet and the driver supports two producers.
            Arc::new(desktop_stream_budget()),
            bundle.interaction_capabilities.clone(),
        ));
        let signing = SigningService::with_credentials(sidecar_root.join("signer"), credentials);

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

        // Recovery must see an authoritative initial fleet snapshot. Flow target
        // selection and startup reconciliation both fail closed on absent devices.
        let devices = control
            .list_devices()
            .await
            .context("initial metadata device scan failed before Flow recovery")?;
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
            agent_token_configured,
            active_agent_artifact_id,
            active_agent_artifact_version,
            active_agent_bundle_id,
            stream_settings: Arc::new(RwLock::new(StreamSettings::default())),
            artifacts_dir,
            legacy_wda_bundle: sidecar_root.join("wda").join("Riviumanagersphone.ipa"),
            nurture: NurtureRuntime::new(),
            nurture_engine,
            flow_mutations: FlowMutationCoordinator::default(),
            overlay_sessions: AsyncMutex::new(HashMap::new()),
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
    /// The map lock is held across `open_manual_session` so two begins cannot
    /// race into DeviceBusy on the same UDID.
    pub async fn begin_overlay_session(&self, udid: &str) -> Result<(), CommandError> {
        let mut sessions = self.overlay_sessions.lock().await;
        if sessions.contains_key(udid) {
            return Ok(());
        }
        let context = self
            .control
            .open_manual_session(udid, DeviceWorkOwner::ManualControl)
            .await
            .map_err(CommandError::from)?;
        sessions.insert(udid.to_string(), context);
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
        self.control
            .close_manual_session(context)
            .map_err(CommandError::from)?;
        Ok(())
    }

    pub async fn overlay_ui_session(&self, udid: &str) -> Option<Arc<dyn UiSession>> {
        let sessions = self.overlay_sessions.lock().await;
        let context = sessions.get(udid)?;
        self.control.session(context).ok()
    }

    /// Release every overlay lease before `shutdown_cleanup` waits for
    /// `lifecycle.outstanding() == 0`. A held ManualControl deadlocks that wait.
    pub async fn close_all_overlay_sessions(&self) {
        let contexts: Vec<UiSessionContext> = {
            let mut sessions = self.overlay_sessions.lock().await;
            sessions.drain().map(|(_, context)| context).collect()
        };
        for context in contexts {
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
                                    continue;
                                }
                            };
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
                            .map(|mut d| {
                                if let Some(prev) = existing.iter().find(|e| e.udid == d.udid) {
                                    if d.stream_url.is_none() {
                                        d.stream_url = prev.stream_url.clone();
                                    }
                                    if !d.wda_ready {
                                        d.wda_ready = prev.wda_ready;
                                    }
                                    d.wda_expires_at = prev.wda_expires_at.or(d.wda_expires_at);
                                    if matches!(prev.status, riviu_core::DeviceStatus::Busy) {
                                        d.status = prev.status.clone();
                                    }
                                    if d.last_error.is_none() {
                                        d.last_error = prev.last_error.clone();
                                    }
                                    d.tile_stream_state = prev.tile_stream_state;
                                }
                                d
                            })
                            .collect();
                        registry.upsert_many(merged);
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
        let app_frames = app.clone();
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
                            let payload = serde_json::json!({
                                "type": "streamFrame",
                                "udid": udid,
                                "fps": per_device_fps,
                            });
                            let _ = app_frames.emit("riviu://event", payload);
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
                    if let Ok(Some(body)) = db.get_script(&s.script_name) {
                        if let Ok(script) = riviu_script_engine::parse_script(&body) {
                            let _ = jobs.enqueue(script, s.udids.clone()).await;
                            let _ = db.log_op("schedule.run", &s.name);
                        }
                    }
                    s.last_run_at = Some(now.to_rfc3339());
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
                let duration =
                    Duration::from_secs(settings.schedule_duration_minutes.max(1) as u64 * 60);
                let started = nurture
                    .start_many(
                        app_nurture.clone(),
                        nurture_engine.clone(),
                        udids,
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
