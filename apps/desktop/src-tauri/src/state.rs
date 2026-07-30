use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use base64::{engine::general_purpose::STANDARD as B64, Engine};
use parking_lot::RwLock;
use riviu_core::db::Database;
use riviu_core::{
    BackgroundStreamLease, DeviceControlPlane, DeviceRegistry, DeviceWorkCoordinator, EventBus,
    JobQueue, NurtureEngine, StreamBudgetManager, StreamSettings, STREAM_FPS,
};
use riviu_ios_driver::{create_driver, DriverMode, DriverTarget, StreamHub};
use riviu_signing::{CredentialStore, SigningService};
use tauri::{AppHandle, Emitter};
use tokio::sync::Notify;

use crate::agent_runtime::{resolve_desktop_agent_runtime, ResolvedAgentRuntime};
use crate::nurture_commands::NurtureRuntime;

pub struct AppState {
    pub registry: DeviceRegistry,
    pub events: EventBus,
    pub control: Arc<DeviceControlPlane>,
    pub streams: StreamHub,
    pub driver_mode: DriverMode,
    pub jobs: JobQueue,
    pub db: Arc<Database>,
    pub signing: SigningService,
    pub agent_token_configured: bool,
    pub active_agent_artifact_id: String,
    pub active_agent_artifact_version: String,
    pub stream_settings: Arc<RwLock<StreamSettings>>,
    pub artifacts_dir: PathBuf,
    pub legacy_wda_bundle: PathBuf,
    pub nurture: NurtureRuntime,
    pub nurture_engine: NurtureEngine,
    background_stop: Arc<AtomicBool>,
    background_stopped: Arc<AtomicBool>,
    background_stopped_notify: Arc<Notify>,
    background_shutdown_error: Arc<RwLock<Option<String>>>,
}

struct ActiveBackgroundSample {
    lease: BackgroundStreamLease,
    baseline_digest: Option<u64>,
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
    active: Option<ActiveBackgroundSample>,
    cursor: usize,
}

impl BackgroundStreamSampler {
    fn new(control: Arc<DeviceControlPlane>, streams: StreamHub, registry: DeviceRegistry) -> Self {
        Self {
            control,
            streams,
            registry,
            active: None,
            cursor: 0,
        }
    }

    async fn tick(&mut self) -> SamplerTick {
        if let Some(sample) = self.active.take() {
            let udid = sample.lease.udid().to_string();
            if !self.control.background_stream_is_current(&sample.lease) {
                set_stream_parked(&self.registry, &udid);
                return SamplerTick::Preempted(udid);
            }

            let fresh = self
                .streams
                .latest(&udid)
                .map(|frame| Some(frame_digest(&frame)) != sample.baseline_digest)
                .unwrap_or(false);
            let turn_due = self
                .control
                .background_turn_due(&sample.lease)
                .unwrap_or(true);
            if fresh || turn_due {
                match self.control.stop_background_stream(&sample.lease).await {
                    Ok(_) if fresh => {
                        set_stream_parked(&self.registry, &udid);
                        return SamplerTick::Parked(udid);
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
                        return SamplerTick::Stale(udid);
                    }
                    Err(error) => {
                        set_stream_state(
                            &self.registry,
                            &udid,
                            riviu_core::TileStreamState::Error,
                            Some(error.to_string()),
                        );
                        self.active = Some(sample);
                        return SamplerTick::Failed(udid);
                    }
                }
            }

            self.active = Some(sample);
            return SamplerTick::Sampling(udid);
        }

        let devices = self.registry.list();
        for offset in 0..devices.len() {
            let index = (self.cursor + offset) % devices.len();
            let device = &devices[index];
            if !background_sample_candidate(device, &self.control.cached_agent_status(&device.udid))
            {
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
            let baseline_digest = self
                .streams
                .latest(&device.udid)
                .map(|frame| frame_digest(&frame));
            match self.control.start_background_stream(&lease).await {
                Ok(url) => {
                    if let Some(mut current) = self.registry.get(&device.udid) {
                        current.stream_url = Some(url);
                        current.status = riviu_core::DeviceStatus::Ready;
                        current.tile_stream_state = riviu_core::TileStreamState::Sampling;
                        current.last_error = None;
                        self.registry.upsert(current);
                    }
                    self.active = Some(ActiveBackgroundSample {
                        lease,
                        baseline_digest,
                    });
                    self.cursor = (index + 1) % devices.len().max(1);
                    return SamplerTick::Started(device.udid.clone());
                }
                Err(error) => {
                    set_stream_state(
                        &self.registry,
                        &device.udid,
                        riviu_core::TileStreamState::Error,
                        Some(error.to_string()),
                    );
                    self.cursor = (index + 1) % devices.len().max(1);
                    return SamplerTick::Failed(device.udid.clone());
                }
            }
        }
        SamplerTick::Idle
    }

    async fn stop(&mut self) -> Result<(), riviu_core::DeviceControlError> {
        let Some(sample) = self.active.take() else {
            return Ok(());
        };
        if self.control.background_stream_is_current(&sample.lease) {
            if let Err(error) = self.control.stop_background_stream(&sample.lease).await {
                self.active = Some(sample);
                return Err(error);
            }
        }
        set_stream_parked(&self.registry, sample.lease.udid());
        Ok(())
    }
}

impl AppState {
    pub async fn bootstrap(resource_dir: Option<PathBuf>) -> anyhow::Result<Self> {
        let data = dirs::data_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("riviu-managers-phone");
        std::fs::create_dir_all(&data)?;
        let artifacts_dir = data.join("artifacts");
        std::fs::create_dir_all(&artifacts_dir)?;

        let db = Arc::new(Database::open(data.join("riviu.db"))?);
        let sidecar_root = resolve_sidecar_root(resource_dir.as_deref());
        let credentials = CredentialStore::system()?;
        let legacy_token = std::env::var("RIVIU_RTMMO_TOKEN").ok();
        let mock_requested = std::env::var("RIVIU_MOCK_DEVICES")
            .map(|value| value == "1" || value.eq_ignore_ascii_case("true"))
            .unwrap_or(false);
        let ResolvedAgentRuntime {
            driver_config,
            settings: resolved_agent_settings,
            token_configured: agent_token_configured,
        } = resolve_desktop_agent_runtime(
            sidecar_root.clone(),
            data.clone(),
            &db,
            &credentials,
            legacy_token.as_deref(),
            mock_requested,
        )?;
        let (active_agent_artifact_id, active_agent_artifact_version) = match &driver_config.target
        {
            DriverTarget::Real(config) => (
                config.artifact.manifest.artifact_id.clone(),
                config.artifact.manifest.artifact_version.clone(),
            ),
            DriverTarget::Mock => ("riviu-agent-mock".to_string(), "1.0.0".to_string()),
            DriverTarget::LegacyStock => ("legacy-stock-wda".to_string(), String::new()),
        };
        let bundle = create_driver(driver_config).await?;
        bundle.driver.set_agent_settings(resolved_agent_settings);
        let control = Arc::new(DeviceControlPlane::new_with_capability_registry(
            bundle.driver.clone(),
            Arc::new(DeviceWorkCoordinator::new()),
            Arc::new(StreamBudgetManager::default()),
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
        );

        let state = Self {
            registry,
            events,
            control,
            streams: bundle.streams,
            driver_mode: bundle.mode,
            jobs,
            db,
            signing,
            agent_token_configured,
            active_agent_artifact_id,
            active_agent_artifact_version,
            stream_settings: Arc::new(RwLock::new(StreamSettings::default())),
            artifacts_dir,
            legacy_wda_bundle: sidecar_root.join("wda").join("Riviumanagersphone.ipa"),
            nurture: NurtureRuntime::new(),
            nurture_engine,
            background_stop: Arc::new(AtomicBool::new(false)),
            background_stopped: Arc::new(AtomicBool::new(false)),
            background_stopped_notify: Arc::new(Notify::new()),
            background_shutdown_error: Arc::new(RwLock::new(None)),
        };

        // Initial scan is metadata-only. The budgeted sampler owns every
        // background producer after background tasks start.
        if let Ok(devices) = state.control.list_devices().await {
            state.registry.upsert_many(devices);
        }

        Ok(state)
    }

    pub fn spawn_background_tasks(&self, app: AppHandle) {
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

        // Forward stream frames @ 24 FPS pacing to UI
        let streams = self.streams.clone();
        let app_frames = app.clone();
        tauri::async_runtime::spawn(async move {
            let mut rx = streams.subscribe();
            let min_gap = Duration::from_millis(1000 / STREAM_FPS as u64);
            let mut last_emit = std::collections::HashMap::<String, std::time::Instant>::new();
            loop {
                match rx.recv().await {
                    Ok((udid, jpeg)) => {
                        let now = std::time::Instant::now();
                        let allow = last_emit
                            .get(&udid)
                            .map(|t| now.duration_since(*t) >= min_gap)
                            .unwrap_or(true);
                        if !allow {
                            continue;
                        }
                        last_emit.insert(udid.clone(), now);
                        let payload = serde_json::json!({
                            "type": "streamFrame",
                            "udid": udid,
                            "jpegBase64": B64.encode(jpeg.as_slice()),
                            "fps": STREAM_FPS,
                        });
                        let _ = app_frames.emit("riviu://event", payload);
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(_) => break,
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

        // TikTok nurture schedule ticks
        let db = self.db.clone();
        let nurture = self.nurture.clone();
        let nurture_engine = self.nurture_engine.clone();
        let registry = self.registry.clone();
        let app_nurture = app.clone();
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

fn background_sample_candidate(
    device: &riviu_core::DeviceInfo,
    status: &riviu_core::AgentStatus,
) -> bool {
    !matches!(
        device.status,
        riviu_core::DeviceStatus::Disconnected
            | riviu_core::DeviceStatus::Pairing
            | riviu_core::DeviceStatus::Preparing
            | riviu_core::DeviceStatus::Busy
    ) && matches!(
        status.state,
        riviu_core::AgentState::Unknown
            | riviu_core::AgentState::Ready
            | riviu_core::AgentState::Error
    )
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

fn frame_digest(frame: &[u8]) -> u64 {
    let mut digest = 0xcbf2_9ce4_8422_2325_u64 ^ frame.len() as u64;
    for byte in frame.iter().step_by((frame.len() / 512).max(1)) {
        digest ^= *byte as u64;
        digest = digest.wrapping_mul(0x100_0000_01b3);
    }
    digest
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

    async fn sampler_fixture() -> (
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
            Arc::new(StreamBudgetManager::default()),
        ));
        let registry = DeviceRegistry::new(EventBus::new(32));
        registry.upsert_many(control.list_devices().await.expect("list mock devices"));
        let sampler = BackgroundStreamSampler::new(control.clone(), streams, registry.clone());
        (driver, control, registry, sampler)
    }

    #[tokio::test]
    async fn background_stream_sampler_rotates_one_producer_after_a_fresh_frame() {
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
            SamplerTick::Parked("MOCK-IPHONE-01".to_string())
        );
        assert_eq!(control.reserved_stream_capacity(), 0);
        assert_eq!(
            sampler.tick().await,
            SamplerTick::Started("MOCK-IPHONE-02".to_string())
        );
        assert_eq!(control.reserved_stream_capacity(), 1);
        assert_eq!(driver.stream_restart_calls(), 2);

        sampler.stop().await.expect("stop sampler producer");
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
        let sample = sampler.active.as_mut().expect("active sampler turn");
        sample.baseline_digest = sampler
            .streams
            .latest(sample.lease.udid())
            .map(|frame| frame_digest(&frame));
        let wait = sample
            .lease
            .turn_deadline()
            .saturating_duration_since(Instant::now());
        tokio::time::sleep(wait + Duration::from_millis(25)).await;

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
            ios_version: "fixture".to_string(),
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
            riviu_core::AgentState::Starting,
        ] {
            status.state = state;
            assert!(!background_sample_candidate(&device, &status));
        }

        status.state = riviu_core::AgentState::Error;
        assert!(background_sample_candidate(&device, &status));

        status.state = riviu_core::AgentState::Ready;
        assert!(background_sample_candidate(&device, &status));
        device.status = riviu_core::DeviceStatus::Preparing;
        assert!(!background_sample_candidate(&device, &status));
        device.status = riviu_core::DeviceStatus::Busy;
        assert!(!background_sample_candidate(&device, &status));
    }

    #[test]
    fn background_stream_state_keeps_cached_frame_url_only_while_sampling() {
        let events = riviu_core::EventBus::new(8);
        let registry = riviu_core::DeviceRegistry::new(events);
        let mut device = riviu_core::DeviceInfo {
            udid: "fixture-state".to_string(),
            name: "fixture".to_string(),
            model: "fixture".to_string(),
            ios_version: "fixture".to_string(),
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
            (
                "../../../sidecars/wda/build_and_install.py",
                "sidecars/wda/build_and_install.py",
            ),
            (
                "../../../sidecars/wda/agent-manifest.json",
                "sidecars/wda/agent-manifest.json",
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
                "../../../sidecars/wda/RiviuAgent.ipa",
                "sidecars/wda/RiviuAgent.ipa",
            ),
            (
                "../../../sidecars/wda/Riviumanagersphone.ipa",
                "sidecars/wda/Riviumanagersphone.ipa",
            ),
        ];

        assert_eq!(resources.len(), expected.len());
        for (source, target) in expected {
            assert_eq!(resources.get(source).and_then(|v| v.as_str()), Some(target));
            assert!(!target.contains("_up_"));
        }
    }
}
