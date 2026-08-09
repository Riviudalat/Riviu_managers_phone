use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use async_trait::async_trait;
use chrono::{Duration as ChronoDuration, Utc};
use image::{ImageBuffer, Rgb};
use parking_lot::{Mutex, RwLock as SyncRwLock};
use riviu_core::{
    ActiveTransport, AgentInstallProof, AgentSettings, AgentState, AgentStatus, AppProcessState,
    ConnectionKind, DeviceCapabilitySnapshot, DeviceDriver, DeviceInfo, DeviceStatus,
    InstalledAgentIdentity, InstalledTargetIdentity, InteractionSessionKind, ProcessAbsenceProof,
    QualifiedElementLocator, QualifiedGeometry, ScreenOrientation, StreamHandoffProof,
    StreamStartProof, StreamStopProof, SwipeGesture, TapPoint, TileStreamState, UiSession,
    STREAM_FPS,
};
use tokio::sync::{oneshot, RwLock};
use tokio::task::JoinHandle;

use crate::interaction_runtime::InteractionLifecycleRegistry;
use crate::stream::{encode_rgb_jpeg, StreamHub};

struct MockStreamProducer {
    generation: u64,
    stop_tx: Option<oneshot::Sender<()>>,
    task: Option<JoinHandle<()>>,
}

impl MockStreamProducer {
    async fn stop(mut self) {
        if let Some(stop_tx) = self.stop_tx.take() {
            let _ = stop_tx.send(());
        }
        let Some(mut task) = self.task.take() else {
            return;
        };
        if tokio::time::timeout(Duration::from_secs(1), &mut task)
            .await
            .is_err()
        {
            task.abort();
            let _ = task.await;
        }
    }
}

impl Drop for MockStreamProducer {
    fn drop(&mut self) {
        if let Some(stop_tx) = self.stop_tx.take() {
            let _ = stop_tx.send(());
        }
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

#[derive(Clone)]
pub struct MockIosDriver {
    devices: Arc<RwLock<Vec<DeviceInfo>>>,
    streams: StreamHub,
    taps: Arc<Mutex<HashMap<String, Vec<TapPoint>>>>,
    typed_text: Arc<Mutex<HashMap<String, String>>>,
    agent_settings: Arc<SyncRwLock<AgentSettings>>,
    agent_statuses: Arc<SyncRwLock<HashMap<String, AgentStatus>>>,
    agent_preflight_calls: Arc<AtomicUsize>,
    agent_repair_calls: Arc<AtomicUsize>,
    mock_repair_failures: Arc<SyncRwLock<HashSet<String>>>,
    ordinary_session_calls: Arc<AtomicUsize>,
    fresh_text_session_calls: Arc<AtomicUsize>,
    stream_restart_calls: Arc<AtomicUsize>,
    window_size_calls: Arc<AtomicUsize>,
    interaction_calls: Arc<Mutex<Vec<&'static str>>>,
    interaction_lifecycle: InteractionLifecycleRegistry,
    mock_interaction_session_failures: Arc<SyncRwLock<HashSet<String>>>,
    mock_stream_failures: Arc<SyncRwLock<HashSet<String>>>,
    mock_static_streams: Arc<SyncRwLock<HashSet<String>>>,
    mock_stream_producers: Arc<Mutex<HashMap<String, MockStreamProducer>>>,
    mock_verified_app_termination: Arc<AtomicBool>,
    mock_app_processes: Arc<SyncRwLock<HashMap<(String, String), u64>>>,
}

impl MockIosDriver {
    pub fn new() -> Self {
        let now = Utc::now();
        let devices = vec![
            DeviceInfo {
                udid: "MOCK-IPHONE-01".into(),
                name: "iPhone Mock 01".into(),
                model: "iPhone10,1".into(),
                ios_version: "16.7.15".into(),
                connection: ConnectionKind::Mock,
                status: DeviceStatus::Ready,
                battery: Some(86),
                wda_ready: true,
                wda_expires_at: Some(now + ChronoDuration::days(5)),
                stream_url: None,
                tile_stream_state: TileStreamState::Parked,
                last_error: None,
            },
            DeviceInfo {
                udid: "MOCK-IPHONE-02".into(),
                name: "iPhone Mock 02".into(),
                model: "iPhone15,2".into(),
                ios_version: "18.2".into(),
                connection: ConnectionKind::Mock,
                status: DeviceStatus::Ready,
                battery: Some(62),
                wda_ready: true,
                wda_expires_at: Some(now + ChronoDuration::days(2)),
                stream_url: None,
                tile_stream_state: TileStreamState::Parked,
                last_error: None,
            },
            DeviceInfo {
                udid: "MOCK-IPHONE-03".into(),
                name: "iPhone Mock 03".into(),
                model: "iPhone13,2".into(),
                ios_version: "16.7".into(),
                connection: ConnectionKind::Mock,
                status: DeviceStatus::Preparing,
                battery: Some(41),
                wda_ready: false,
                wda_expires_at: Some(now - ChronoDuration::days(1)),
                stream_url: None,
                tile_stream_state: TileStreamState::Error,
                last_error: Some("WDA signing expired".into()),
            },
        ];
        let streams = StreamHub::new();
        Self {
            devices: Arc::new(RwLock::new(devices)),
            streams: streams.clone(),
            taps: Arc::new(Mutex::new(HashMap::new())),
            typed_text: Arc::new(Mutex::new(HashMap::new())),
            agent_settings: Arc::new(SyncRwLock::new(AgentSettings::default())),
            agent_statuses: Arc::new(SyncRwLock::new(HashMap::new())),
            agent_preflight_calls: Arc::new(AtomicUsize::new(0)),
            agent_repair_calls: Arc::new(AtomicUsize::new(0)),
            mock_repair_failures: Arc::new(SyncRwLock::new(HashSet::new())),
            ordinary_session_calls: Arc::new(AtomicUsize::new(0)),
            fresh_text_session_calls: Arc::new(AtomicUsize::new(0)),
            stream_restart_calls: Arc::new(AtomicUsize::new(0)),
            window_size_calls: Arc::new(AtomicUsize::new(0)),
            interaction_calls: Arc::new(Mutex::new(Vec::new())),
            interaction_lifecycle: InteractionLifecycleRegistry::default(),
            mock_interaction_session_failures: Arc::new(SyncRwLock::new(HashSet::new())),
            mock_stream_failures: Arc::new(SyncRwLock::new(HashSet::new())),
            mock_static_streams: Arc::new(SyncRwLock::new(HashSet::new())),
            mock_stream_producers: Arc::new(Mutex::new(HashMap::new())),
            mock_verified_app_termination: Arc::new(AtomicBool::new(false)),
            mock_app_processes: Arc::new(SyncRwLock::new(HashMap::new())),
        }
    }

    fn start_mock_stream_producer(&self, udid: &str, generation: u64) -> anyhow::Result<bool> {
        if self.streams.generation(udid) != generation {
            anyhow::bail!("mock stream generation became stale before start");
        }

        let frame_index = match udid {
            "MOCK-IPHONE-01" => 0,
            "MOCK-IPHONE-02" => 1,
            _ => 2,
        };
        let first_frame = render_mock_frame(udid, frame_index, 0).and_then(|frame| {
            image::load_from_memory(&frame).context("decode first mock stream frame")?;
            Ok(frame)
        })?;

        let mut producers = self.mock_stream_producers.lock();
        if let Some(producer) = producers.get(udid) {
            if producer.generation == generation
                && producer
                    .task
                    .as_ref()
                    .is_some_and(|task| !task.is_finished())
            {
                return Ok(self.streams.latest(udid).is_some());
            }
            anyhow::bail!("mock stream producer must be explicitly stopped before restart");
        }

        let streams = self.streams.clone();
        let udid_owned = udid.to_string();
        let static_stream = self.mock_static_streams.read().contains(udid);
        let (stop_tx, mut stop_rx) = oneshot::channel();
        let task = tokio::spawn(async move {
            let mut tick: u64 = 1;
            let frame_interval = Duration::from_millis(1000 / STREAM_FPS as u64);
            loop {
                tokio::select! {
                    _ = &mut stop_rx => break,
                    _ = tokio::time::sleep(frame_interval) => {
                        if static_stream {
                            continue;
                        }
                        let Ok(jpeg) = render_mock_frame(&udid_owned, frame_index, tick) else {
                            break;
                        };
                        if !streams.publish_if_current(&udid_owned, generation, jpeg) {
                            break;
                        }
                        tick = tick.wrapping_add(1);
                    }
                }
            }
        });
        producers.insert(
            udid.to_string(),
            MockStreamProducer {
                generation,
                stop_tx: Some(stop_tx),
                task: Some(task),
            },
        );
        if !self
            .streams
            .publish_if_current(udid, generation, first_frame)
        {
            producers.remove(udid);
            anyhow::bail!("mock stream generation became stale during start");
        }
        Ok(true)
    }

    async fn stop_mock_stream_producer(&self, udid: &str) {
        let producer = self.mock_stream_producers.lock().remove(udid);
        if let Some(producer) = producer {
            producer.stop().await;
        }
    }

    fn apply_mock_stream_state(&self, devices: &mut [DeviceInfo]) {
        let producers = self.mock_stream_producers.lock();
        for device in devices {
            let running = producers.contains_key(&device.udid);
            device.stream_url = running.then(|| format!("mock://{}", device.udid));
            if running {
                device.tile_stream_state = TileStreamState::Live;
            } else if device.tile_stream_state == TileStreamState::Live {
                device.tile_stream_state = TileStreamState::Parked;
            }
        }
    }

    pub fn stream_hub(&self) -> StreamHub {
        self.streams.clone()
    }

    pub fn set_mock_agent_status(&self, status: AgentStatus) {
        self.agent_statuses
            .write()
            .insert(status.udid.clone(), status);
    }

    pub fn agent_preflight_calls(&self) -> usize {
        self.agent_preflight_calls.load(Ordering::Relaxed)
    }

    pub fn set_mock_repair_failure(&self, udid: &str, should_fail: bool) {
        let mut failures = self.mock_repair_failures.write();
        if should_fail {
            failures.insert(udid.to_string());
        } else {
            failures.remove(udid);
        }
    }

    pub fn set_mock_stream_failure(&self, udid: &str, should_fail: bool) {
        let mut failures = self.mock_stream_failures.write();
        if should_fail {
            failures.insert(udid.to_string());
        } else {
            failures.remove(udid);
        }
    }

    pub fn set_mock_stream_static(&self, udid: &str, should_be_static: bool) {
        let mut streams = self.mock_static_streams.write();
        if should_be_static {
            streams.insert(udid.to_string());
        } else {
            streams.remove(udid);
        }
    }

    pub fn set_mock_verified_app_termination(&self, supported: bool) {
        self.mock_verified_app_termination
            .store(supported, Ordering::SeqCst);
    }

    pub fn set_mock_app_process(&self, udid: &str, bundle_id: &str, pid: Option<u64>) {
        assert!(pid != Some(0), "mock app PID must be positive");
        let key = (udid.to_string(), bundle_id.to_string());
        let mut processes = self.mock_app_processes.write();
        if let Some(pid) = pid {
            processes.insert(key, pid);
        } else {
            processes.remove(&key);
        }
    }

    pub fn agent_repair_calls(&self) -> usize {
        self.agent_repair_calls.load(Ordering::Relaxed)
    }

    pub fn ordinary_session_calls(&self) -> usize {
        self.ordinary_session_calls.load(Ordering::Relaxed)
    }

    pub fn fresh_text_session_calls(&self) -> usize {
        self.fresh_text_session_calls.load(Ordering::Relaxed)
    }

    pub fn stream_restart_calls(&self) -> usize {
        self.stream_restart_calls.load(Ordering::Relaxed)
    }

    fn interaction_snapshot(udid: &str) -> anyhow::Result<DeviceCapabilitySnapshot> {
        let (product_type, ios_version, target_version, target_build, geometry) = match udid {
            "MOCK-IPHONE-01" => (
                "iPhone10,1",
                "16.7.15",
                "35.0.0",
                "350001",
                Some(QualifiedGeometry {
                    logical_width: 375.0,
                    logical_height: 667.0,
                    pixel_width: 750,
                    pixel_height: 1334,
                    scale_x: 2.0,
                    scale_y: 2.0,
                    orientation: ScreenOrientation::Portrait,
                }),
            ),
            "MOCK-IPHONE-02" => ("iPhone15,2", "18.2", "36.0.0", "360001", None),
            "MOCK-IPHONE-03" => ("iPhone13,2", "16.7", "34.0.0", "340001", None),
            _ => anyhow::bail!("device not found"),
        };

        Ok(DeviceCapabilitySnapshot {
            installed_agent: InstalledAgentIdentity {
                bundle_id: "com.mrph.svc".into(),
                version: "1.0".into(),
                build: "1".into(),
                executable_name: "FixtureRunner".into(),
                signer_identity_sha256:
                    "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
            },
            selected_artifact_sha256:
                "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
            agent_version: "2026.07.28.3".into(),
            protocol_version: 1,
            driver_adapter_version: "interaction-v1".into(),
            transport: ActiveTransport::Mock,
            product_type: product_type.into(),
            ios_version: ios_version.into(),
            target_app: InstalledTargetIdentity {
                bundle_id: "com.ss.iphone.ugc.Ame".into(),
                version: target_version.into(),
                build: target_build.into(),
            },
            protected_auth_ready: udid != "MOCK-IPHONE-03",
            geometry,
        })
    }

    fn target_snapshot(
        udid: &str,
        target_bundle_id: &str,
    ) -> anyhow::Result<DeviceCapabilitySnapshot> {
        let mut snapshot = Self::interaction_snapshot(udid)?;
        snapshot.target_app.bundle_id = target_bundle_id.to_string();
        Ok(snapshot)
    }
}

impl Default for MockIosDriver {
    fn default() -> Self {
        Self::new()
    }
}

fn render_mock_frame(name: &str, index: usize, tick: u64) -> anyhow::Result<Vec<u8>> {
    let (width, height) = if index == 0 {
        (750u32, 1334u32)
    } else {
        (390u32, 844u32)
    };
    let mut img: ImageBuffer<Rgb<u8>, Vec<u8>> = ImageBuffer::new(width, height);
    let base: [[u8; 3]; 3] = [[24, 28, 42], [32, 24, 40], [20, 36, 34]];
    let palette = base[index % 3];
    let pulse = ((tick % 48) as u8).saturating_mul(2);
    for y in 0..height {
        for x in 0..width {
            let edge = x < 8 || x > width - 9 || y < 8 || y > height - 9;
            let bar = y > 60 && y < 120;
            let color = if edge {
                Rgb([40, 44, 58])
            } else if bar {
                Rgb([
                    palette[0].saturating_add(pulse / 2),
                    palette[1].saturating_add(30),
                    palette[2].saturating_add(60),
                ])
            } else {
                Rgb([
                    palette[0].saturating_add((y / 20) as u8),
                    palette[1],
                    palette[2].saturating_add((x / 30) as u8),
                ])
            };
            img.put_pixel(x, y, color);
        }
    }
    // Simple status strip representing "live @ 24fps".
    let progress_width = width.saturating_sub(40).max(1);
    for x in 20..(20 + (tick % progress_width as u64) as u32) {
        for y in height.saturating_sub(64)..height.saturating_sub(44) {
            img.put_pixel(x, y, Rgb([80, 200, 140]));
        }
    }
    let _ = name; // name shown in UI chrome, not burned into pixels for perf
    encode_rgb_jpeg(&img, 70)
}

struct MockUiSession {
    udid: String,
    taps: Arc<Mutex<HashMap<String, Vec<TapPoint>>>>,
    typed_text: Arc<Mutex<HashMap<String, String>>>,
    supports_text_input: bool,
    supports_accessibility_readback: bool,
    window_size_calls: Arc<AtomicUsize>,
}

#[async_trait]
impl UiSession for MockUiSession {
    async fn tap(&self, point: TapPoint) -> anyhow::Result<()> {
        self.taps
            .lock()
            .entry(self.udid.clone())
            .or_default()
            .push(point);
        Ok(())
    }

    async fn swipe(&self, gesture: SwipeGesture) -> anyhow::Result<()> {
        let _ = gesture;
        Ok(())
    }

    async fn type_text(&self, text: &str) -> anyhow::Result<()> {
        self.typed_text
            .lock()
            .insert(self.udid.clone(), text.to_string());
        Ok(())
    }

    fn supports_text_input(&self) -> bool {
        self.supports_text_input
    }

    async fn read_text(
        &self,
        locator: &QualifiedElementLocator,
        _request_timeout: Duration,
    ) -> anyhow::Result<String> {
        anyhow::ensure!(
            self.supports_accessibility_readback,
            "mock accessibility read-back is unavailable"
        );
        anyhow::ensure!(
            !locator.value.trim().is_empty() && locator.value.trim() == locator.value,
            "mock locator is invalid"
        );
        Ok(self
            .typed_text
            .lock()
            .get(&self.udid)
            .cloned()
            .unwrap_or_default())
    }

    fn supports_accessibility_readback(&self) -> bool {
        self.supports_accessibility_readback
    }

    async fn home(&self) -> anyhow::Result<()> {
        Ok(())
    }

    async fn find_and_tap(&self, _accessibility_id: &str) -> anyhow::Result<()> {
        Ok(())
    }

    async fn assert_visible(&self, _accessibility_id: &str) -> anyhow::Result<()> {
        Ok(())
    }

    async fn healthy(&self) -> bool {
        true
    }

    async fn window_size(&self) -> anyhow::Result<(f64, f64)> {
        self.window_size_calls.fetch_add(1, Ordering::Relaxed);
        if self.udid == "MOCK-IPHONE-01" {
            Ok((375.0, 667.0))
        } else {
            Ok((390.0, 844.0))
        }
    }

    async fn launch_app_foreground(&self, _bundle_id: &str) -> anyhow::Result<()> {
        Ok(())
    }

    async fn active_app_bundle(&self) -> anyhow::Result<String> {
        Ok("com.ss.iphone.ugc.Ame".into())
    }

    fn stream_url(&self) -> Option<String> {
        Some(format!("mock://{}", self.udid))
    }
}

#[async_trait]
impl DeviceDriver for MockIosDriver {
    fn agent_settings(&self) -> AgentSettings {
        self.agent_settings.read().clone()
    }

    fn set_agent_settings(&self, settings: AgentSettings) {
        *self.agent_settings.write() = settings;
    }

    fn cached_agent_status(&self, udid: &str) -> AgentStatus {
        if let Some(status) = self.agent_statuses.read().get(udid) {
            return status.clone();
        }
        AgentStatus {
            udid: udid.to_string(),
            state: AgentState::Ready,
            artifact_id: "riviu-agent-mock".to_string(),
            artifact_version: "1.0.0".to_string(),
            bundle_id: "com.mrph.svc".to_string(),
            protocol_version: 1,
            features: vec![
                "stream".to_string(),
                "tap".to_string(),
                "swipe".to_string(),
                "text".to_string(),
                "clipboard".to_string(),
                "pushMedia".to_string(),
            ],
            installed_version: Some("1.0".to_string()),
            installed_build: Some("1".to_string()),
            auth_ready: true,
            mjpeg_ready: true,
            session_ready: true,
            message: None,
        }
    }

    async fn preflight_agent(&self, udid: &str) -> anyhow::Result<AgentStatus> {
        self.agent_preflight_calls.fetch_add(1, Ordering::Relaxed);
        Ok(self.cached_agent_status(udid))
    }

    async fn repair_agent(&self, udid: &str) -> anyhow::Result<AgentStatus> {
        self.agent_repair_calls.fetch_add(1, Ordering::Relaxed);
        if self.mock_repair_failures.read().contains(udid) {
            let mut status = self.cached_agent_status(udid);
            status.state = AgentState::Error;
            status.auth_ready = false;
            status.mjpeg_ready = false;
            status.session_ready = false;
            status.message = Some("Mock Agent repair failed".to_string());
            self.set_mock_agent_status(status);
            anyhow::bail!("mock Agent repair failed");
        }
        Ok(self.cached_agent_status(udid))
    }

    async fn inspect_interaction_device(
        &self,
        udid: &str,
    ) -> anyhow::Result<DeviceCapabilitySnapshot> {
        Self::interaction_snapshot(udid)
    }

    async fn inspect_device_for_target(
        &self,
        udid: &str,
        target_bundle_id: &str,
    ) -> anyhow::Result<DeviceCapabilitySnapshot> {
        Self::target_snapshot(udid, target_bundle_id)
    }

    async fn repair_agent_install_only(&self, udid: &str) -> anyhow::Result<AgentInstallProof> {
        if self.mock_repair_failures.read().contains(udid) {
            anyhow::bail!("mock Agent install-only auth failed");
        }
        let snapshot = Self::interaction_snapshot(udid)?;
        if !snapshot.protected_auth_ready {
            anyhow::bail!("mock Agent protected auth is unavailable");
        }
        let proof = AgentInstallProof {
            installed: snapshot.installed_agent,
            artifact_sha256: snapshot.selected_artifact_sha256,
            protected_auth_ready: true,
            session_created: false,
            stream_started: false,
        };
        proof.validate_install_only()?;
        Ok(proof)
    }

    async fn stop_owned_stream(&self, udid: &str) -> anyhow::Result<StreamStopProof> {
        self.interaction_calls.lock().push("stop old producer");
        self.stop_mock_stream_producer(udid).await;
        let (old_generation, new_generation) = self.streams.clear_and_advance(udid);
        self.interaction_calls
            .lock()
            .push("clear/increment generation");
        self.interaction_lifecycle
            .record_stopped(udid, new_generation);
        Ok(StreamStopProof {
            old_generation,
            new_generation,
            child_stopped: true,
        })
    }

    async fn confirm_interaction_stream_stopped(
        &self,
        udid: &str,
    ) -> anyhow::Result<StreamHandoffProof> {
        if self.mock_stream_producers.lock().contains_key(udid) {
            anyhow::bail!("interaction handoff still owns an MJPEG producer");
        }
        let generation = self.streams.generation(udid);
        self.interaction_lifecycle.record_stopped(udid, generation);
        Ok(StreamHandoffProof { generation })
    }

    async fn start_interaction_session(
        &self,
        udid: &str,
        _bundle_id: &str,
        kind: InteractionSessionKind,
    ) -> anyhow::Result<Box<dyn UiSession>> {
        let generation = self.streams.generation(udid);
        let reservation = self
            .interaction_lifecycle
            .begin_session(udid, generation, kind)?;
        if kind == InteractionSessionKind::FreshText {
            self.interaction_calls.lock().push("bootstrap fresh Agent");
        }
        self.interaction_calls.lock().push("foreground TikTok");
        self.interaction_calls
            .lock()
            .push("create/attach approved session");

        if self.mock_interaction_session_failures.read().contains(udid) {
            self.interaction_lifecycle.clear(udid);
            anyhow::bail!("mock interaction session failed");
        }
        if let Err(error) = self.interaction_lifecycle.complete_session(&reservation) {
            self.interaction_lifecycle.clear(udid);
            return Err(error);
        }

        Ok(Box::new(MockUiSession {
            udid: udid.to_string(),
            taps: self.taps.clone(),
            typed_text: self.typed_text.clone(),
            supports_text_input: kind == InteractionSessionKind::FreshText,
            supports_accessibility_readback: kind == InteractionSessionKind::FreshText,
            window_size_calls: self.window_size_calls.clone(),
        }))
    }

    async fn foreground_target_app_and_start_interaction_session(
        &self,
        udid: &str,
        bundle_id: &str,
        kind: InteractionSessionKind,
    ) -> anyhow::Result<Box<dyn UiSession>> {
        self.start_interaction_session(udid, bundle_id, kind).await
    }

    async fn start_stream_after_session(&self, udid: &str) -> anyhow::Result<StreamStartProof> {
        let generation = self.streams.generation(udid);
        let reservation = self
            .interaction_lifecycle
            .reserve_stream(udid, generation)?;
        self.interaction_calls
            .lock()
            .push("reserve stream generation");
        self.interaction_calls.lock().push("start MJPEG");

        let first_frame_observed =
            match self.start_mock_stream_producer(udid, reservation.generation()) {
                Ok(observed) => observed,
                Err(error) => {
                    self.stop_mock_stream_producer(udid).await;
                    self.streams.clear(udid);
                    self.interaction_lifecycle.clear(udid);
                    return Err(error);
                }
            };
        if !first_frame_observed {
            self.stop_mock_stream_producer(udid).await;
            self.streams.clear(udid);
            self.interaction_lifecycle.clear(udid);
            anyhow::bail!("mock interaction stream did not produce a decoded frame");
        }
        self.interaction_calls.lock().push("first decoded frame");
        if let Err(error) = self.interaction_lifecycle.complete_stream(&reservation) {
            self.stop_mock_stream_producer(udid).await;
            self.streams.clear(udid);
            self.interaction_lifecycle.clear(udid);
            return Err(error);
        }

        Ok(StreamStartProof {
            generation: reservation.generation(),
            first_frame_observed,
            stream_url: format!("mock://{udid}"),
        })
    }

    fn supports_text_comments(&self, _udid: &str) -> bool {
        true
    }

    fn supports_verified_app_termination(&self, _udid: &str) -> bool {
        self.mock_verified_app_termination.load(Ordering::SeqCst)
    }

    async fn inspect_app_process(
        &self,
        udid: &str,
        bundle_id: &str,
    ) -> anyhow::Result<AppProcessState> {
        if !self.supports_verified_app_termination(udid) {
            anyhow::bail!("verified app termination is disabled for this mock driver");
        }
        let pid = self
            .mock_app_processes
            .read()
            .get(&(udid.to_string(), bundle_id.to_string()))
            .copied();
        Ok(AppProcessState {
            bundle_id: bundle_id.to_string(),
            pid,
            running: pid.is_some(),
        })
    }

    fn requires_fresh_text_session(&self, _udid: &str) -> bool {
        true
    }

    async fn list_devices(&self) -> anyhow::Result<Vec<DeviceInfo>> {
        let mut devices = self.devices.read().await.clone();
        self.apply_mock_stream_state(&mut devices);
        Ok(devices)
    }

    async fn refresh_device(&self, udid: &str) -> anyhow::Result<DeviceInfo> {
        let mut device = self
            .devices
            .read()
            .await
            .iter()
            .find(|d| d.udid == udid)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("device not found"))?;
        self.apply_mock_stream_state(std::slice::from_mut(&mut device));
        Ok(device)
    }

    async fn install_app(&self, _udid: &str, path: &Path) -> anyhow::Result<()> {
        if !path.exists() {
            anyhow::bail!("IPA not found: {}", path.display());
        }
        Ok(())
    }

    async fn uninstall_app(&self, _udid: &str, _bundle_id: &str) -> anyhow::Result<()> {
        Ok(())
    }

    async fn screenshot(&self, udid: &str, dest: &Path) -> anyhow::Result<PathBuf> {
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let jpeg = self
            .streams
            .latest(udid)
            .map(|frame| frame.to_vec())
            .unwrap_or_else(|| {
                let index = match udid {
                    "MOCK-IPHONE-01" => 0,
                    "MOCK-IPHONE-02" => 1,
                    _ => 2,
                };
                render_mock_frame(udid, index, 0).unwrap_or_default()
            });
        // store as jpeg bytes with .png extension for simplicity in mock; write jpeg
        let out = if dest.extension().and_then(|e| e.to_str()) == Some("png") {
            dest.with_extension("jpg")
        } else {
            dest.to_path_buf()
        };
        std::fs::write(&out, jpeg)?;
        Ok(out)
    }

    async fn syslog_tail(&self, udid: &str, lines: usize) -> anyhow::Result<String> {
        let mut out = String::new();
        for i in 0..lines {
            out.push_str(&format!(
                "[{udid}] mock syslog line {i}: SpringBoard ready\n"
            ));
        }
        Ok(out)
    }

    async fn launch_app(&self, _udid: &str, _bundle_id: &str) -> anyhow::Result<()> {
        self.interaction_calls
            .lock()
            .push("foreground target via bridge");
        Ok(())
    }

    async fn terminate_app(
        &self,
        udid: &str,
        bundle_id: &str,
    ) -> anyhow::Result<ProcessAbsenceProof> {
        if !self.supports_verified_app_termination(udid) {
            anyhow::bail!("verified app termination is disabled for this mock driver");
        }
        let old_pid = self
            .mock_app_processes
            .write()
            .remove(&(udid.to_string(), bundle_id.to_string()));
        Ok(ProcessAbsenceProof {
            bundle_id: bundle_id.to_string(),
            old_pid,
        })
    }

    async fn reboot(&self, udid: &str) -> anyhow::Result<()> {
        let mut list = self.devices.write().await;
        if let Some(d) = list.iter_mut().find(|d| d.udid == udid) {
            d.status = DeviceStatus::Preparing;
        }
        Ok(())
    }

    async fn start_ui_session(&self, udid: &str) -> anyhow::Result<Box<dyn UiSession>> {
        self.ordinary_session_calls.fetch_add(1, Ordering::Relaxed);
        Ok(Box::new(MockUiSession {
            udid: udid.to_string(),
            taps: self.taps.clone(),
            typed_text: self.typed_text.clone(),
            supports_text_input: false,
            supports_accessibility_readback: false,
            window_size_calls: self.window_size_calls.clone(),
        }))
    }

    async fn start_fresh_text_session(
        &self,
        udid: &str,
        _bundle_id: &str,
    ) -> anyhow::Result<Box<dyn UiSession>> {
        self.fresh_text_session_calls
            .fetch_add(1, Ordering::Relaxed);
        Ok(Box::new(MockUiSession {
            udid: udid.to_string(),
            taps: self.taps.clone(),
            typed_text: self.typed_text.clone(),
            supports_text_input: true,
            supports_accessibility_readback: true,
            window_size_calls: self.window_size_calls.clone(),
        }))
    }

    async fn ensure_stream(&self, udid: &str) -> anyhow::Result<String> {
        self.stream_restart_calls.fetch_add(1, Ordering::Relaxed);
        if self.mock_stream_failures.read().contains(udid) {
            anyhow::bail!("mock stream start failed");
        }
        self.interaction_lifecycle.clear(udid);
        self.start_mock_stream_producer(udid, self.streams.generation(udid))?;
        Ok(format!("mock://{udid}"))
    }

    async fn prepare_device(&self, udid: &str) -> anyhow::Result<()> {
        let mut list = self.devices.write().await;
        if let Some(d) = list.iter_mut().find(|d| d.udid == udid) {
            d.status = DeviceStatus::Ready;
            d.wda_ready = true;
            d.wda_expires_at = Some(Utc::now() + ChronoDuration::days(7));
            d.last_error = None;
            d.stream_url = None;
            d.tile_stream_state = TileStreamState::Parked;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use riviu_core::{ElementLocatorStrategy, InteractionSessionKind};

    async fn wait_for_mock_frame(driver: &MockIosDriver, udid: &str) -> riviu_core::Frame {
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if let Some(frame) = driver.streams.latest(udid) {
                    return frame;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("mock producer did not publish a frame")
    }

    #[tokio::test]
    async fn mock_stream_lifecycle_startup_list_and_refresh_are_producer_free() {
        let driver = MockIosDriver::new();

        let listed = driver.list_devices().await.expect("list mock devices");
        let refreshed = driver
            .refresh_device("MOCK-IPHONE-01")
            .await
            .expect("refresh mock device");
        tokio::time::sleep(Duration::from_millis(100)).await;

        assert!(driver.mock_stream_producers.lock().is_empty());
        assert!(driver.streams.latest("MOCK-IPHONE-01").is_none());
        assert!(listed.iter().all(|device| device.stream_url.is_none()));
        assert_eq!(refreshed.stream_url, None);
        assert_eq!(refreshed.tile_stream_state, TileStreamState::Parked);
    }

    #[tokio::test]
    async fn mock_stream_lifecycle_start_owns_exactly_one_current_generation() {
        let driver = MockIosDriver::new();
        let generation = driver.streams.generation("MOCK-IPHONE-01");

        driver
            .ensure_stream("MOCK-IPHONE-01")
            .await
            .expect("start mock stream");
        wait_for_mock_frame(&driver, "MOCK-IPHONE-01").await;
        driver
            .ensure_stream("MOCK-IPHONE-01")
            .await
            .expect("reuse mock stream");

        {
            let producers = driver.mock_stream_producers.lock();
            assert_eq!(producers.len(), 1);
            assert_eq!(
                producers
                    .get("MOCK-IPHONE-01")
                    .map(|producer| producer.generation),
                Some(generation)
            );
        }
        assert_eq!(driver.streams.generation("MOCK-IPHONE-01"), generation);
        driver
            .stop_owned_stream("MOCK-IPHONE-01")
            .await
            .expect("cleanup mock stream");
    }

    #[tokio::test]
    async fn mock_stream_lifecycle_stop_prevents_all_later_frames() {
        let driver = MockIosDriver::new();
        driver
            .ensure_stream("MOCK-IPHONE-01")
            .await
            .expect("start mock stream");
        wait_for_mock_frame(&driver, "MOCK-IPHONE-01").await;

        let proof = driver
            .stop_owned_stream("MOCK-IPHONE-01")
            .await
            .expect("stop mock stream");
        let mut frames = driver.streams.subscribe();

        assert!(proof.child_stopped);
        assert!(driver.mock_stream_producers.lock().is_empty());
        assert!(driver.streams.latest("MOCK-IPHONE-01").is_none());
        assert!(
            tokio::time::timeout(Duration::from_millis(100), frames.recv())
                .await
                .is_err(),
            "stopped mock producer published another frame"
        );
        let refreshed = driver
            .refresh_device("MOCK-IPHONE-01")
            .await
            .expect("refresh stopped device");
        assert_eq!(refreshed.stream_url, None);
        assert_eq!(refreshed.tile_stream_state, TileStreamState::Parked);
    }

    #[tokio::test]
    async fn mock_stream_lifecycle_restart_uses_the_advanced_generation() {
        let driver = MockIosDriver::new();
        driver
            .ensure_stream("MOCK-IPHONE-01")
            .await
            .expect("start first mock stream");
        wait_for_mock_frame(&driver, "MOCK-IPHONE-01").await;
        let first_generation = driver
            .mock_stream_producers
            .lock()
            .get("MOCK-IPHONE-01")
            .expect("first producer")
            .generation;

        let stopped = driver
            .stop_owned_stream("MOCK-IPHONE-01")
            .await
            .expect("stop first mock stream");
        driver
            .ensure_stream("MOCK-IPHONE-01")
            .await
            .expect("restart mock stream");
        wait_for_mock_frame(&driver, "MOCK-IPHONE-01").await;
        let second_generation = driver
            .mock_stream_producers
            .lock()
            .get("MOCK-IPHONE-01")
            .expect("second producer")
            .generation;

        assert_eq!(stopped.old_generation, first_generation);
        assert_eq!(stopped.new_generation, second_generation);
        assert_ne!(first_generation, second_generation);
        assert_eq!(driver.mock_stream_producers.lock().len(), 1);
        driver
            .stop_owned_stream("MOCK-IPHONE-01")
            .await
            .expect("cleanup restarted mock stream");
    }

    #[tokio::test]
    async fn mock_agent_status_is_ready_and_text_capable() {
        let driver = MockIosDriver::new();

        let status = driver
            .preflight_agent("mock-device")
            .await
            .expect("mock preflight");

        assert_eq!(status.state, AgentState::Ready);
        assert!(status.auth_ready);
        assert!(status.mjpeg_ready);
        assert!(status.session_ready);
        assert!(status.features.iter().any(|feature| feature == "text"));
        assert!(driver.supports_text_comments("fixture-udid"));
    }

    #[tokio::test]
    async fn mock_agent_settings_round_trip() {
        let driver = MockIosDriver::new();
        driver.set_agent_settings(AgentSettings { auto_repair: false });

        assert_eq!(
            driver.agent_settings(),
            AgentSettings { auto_repair: false }
        );
    }

    #[tokio::test]
    async fn mock_records_ordinary_fresh_and_stream_session_calls_separately() {
        let driver = MockIosDriver::new();

        let ordinary = driver
            .start_ui_session("mock-device")
            .await
            .expect("ordinary session");
        let fresh = driver
            .start_fresh_text_session("mock-device", "com.ss.iphone.ugc.Ame")
            .await
            .expect("fresh text session");
        driver.ensure_stream("mock-device").await.expect("stream");

        assert_eq!(driver.ordinary_session_calls(), 1);
        assert_eq!(driver.fresh_text_session_calls(), 1);
        assert_eq!(driver.stream_restart_calls(), 1);
        assert!(!ordinary.supports_text_input());
        assert!(!ordinary.supports_accessibility_readback());
        assert!(fresh.supports_text_input());
        assert!(fresh.supports_accessibility_readback());
        fresh
            .type_text("Tiếng Việt chính xác")
            .await
            .expect("mock type text");
        assert_eq!(
            fresh
                .read_text(
                    &QualifiedElementLocator {
                        strategy: ElementLocatorStrategy::AccessibilityId,
                        value: "SearchField".into(),
                    },
                    Duration::from_secs(1),
                )
                .await
                .expect("mock read text"),
            "Tiếng Việt chính xác"
        );
    }

    #[tokio::test]
    async fn interaction_inspect_returns_qualified_and_newer_mock_profiles_without_side_effects() {
        let driver = MockIosDriver::new();
        let counters_before = (
            driver.agent_preflight_calls(),
            driver.agent_repair_calls(),
            driver.ordinary_session_calls(),
            driver.fresh_text_session_calls(),
            driver.stream_restart_calls(),
        );

        let qualified = driver
            .inspect_interaction_device("MOCK-IPHONE-01")
            .await
            .expect("qualified fixture");
        let newer = driver
            .inspect_interaction_device("MOCK-IPHONE-02")
            .await
            .expect("newer fixture");
        let listed = driver
            .refresh_device("MOCK-IPHONE-01")
            .await
            .expect("listed fixture");

        assert_eq!(qualified.product_type, "iPhone10,1");
        assert_eq!(qualified.ios_version, "16.7.15");
        assert_eq!(listed.model, qualified.product_type);
        assert_eq!(listed.ios_version, qualified.ios_version);
        assert_eq!(qualified.transport, ActiveTransport::Mock);
        assert_eq!(
            qualified.geometry.as_ref().map(|value| (
                value.logical_width,
                value.logical_height,
                value.pixel_width,
                value.pixel_height,
            )),
            Some((375.0, 667.0, 750, 1334))
        );
        assert_eq!(newer.product_type, "iPhone15,2");
        assert_eq!(newer.ios_version, "18.2");
        assert_eq!(newer.geometry, None);
        let mock_frame = render_mock_frame("iPhone Mock 01", 0, 0).expect("mock JPEG");
        let decoded = image::load_from_memory(&mock_frame).expect("decode mock JPEG");
        assert_eq!((decoded.width(), decoded.height()), (750, 1334));
        assert_eq!(
            counters_before,
            (
                driver.agent_preflight_calls(),
                driver.agent_repair_calls(),
                driver.ordinary_session_calls(),
                driver.fresh_text_session_calls(),
                driver.stream_restart_calls(),
            )
        );
    }

    #[tokio::test]
    async fn mock_install_only_repair_returns_proof_without_session_or_stream() {
        let driver = MockIosDriver::new();
        let lifecycle_before = (
            driver.ordinary_session_calls(),
            driver.fresh_text_session_calls(),
            driver.stream_restart_calls(),
        );

        let proof = driver
            .repair_agent_install_only("MOCK-IPHONE-01")
            .await
            .expect("mock install-only proof");

        proof.validate_install_only().expect("valid mock proof");
        assert_eq!(proof.installed.bundle_id, "com.mrph.svc");
        assert_eq!(
            lifecycle_before,
            (
                driver.ordinary_session_calls(),
                driver.fresh_text_session_calls(),
                driver.stream_restart_calls(),
            )
        );
    }

    #[tokio::test]
    async fn interaction_lifecycle_orders_ordinary_session_before_stream() {
        let driver = MockIosDriver::new();

        let stop = driver
            .stop_owned_stream("MOCK-IPHONE-01")
            .await
            .expect("stop old producer");
        let _session = driver
            .start_interaction_session(
                "MOCK-IPHONE-01",
                "com.ss.iphone.ugc.Ame",
                InteractionSessionKind::Ordinary,
            )
            .await
            .expect("ordinary interaction session");
        let started = driver
            .start_stream_after_session("MOCK-IPHONE-01")
            .await
            .expect("reserved interaction stream");

        assert_eq!(
            driver.interaction_calls.lock().as_slice(),
            [
                "stop old producer",
                "clear/increment generation",
                "foreground TikTok",
                "create/attach approved session",
                "reserve stream generation",
                "start MJPEG",
                "first decoded frame",
            ]
        );
        assert!(stop.child_stopped);
        assert!(stop.new_generation > stop.old_generation);
        assert_eq!(started.generation, stop.new_generation);
        assert!(started.first_frame_observed);
        assert_eq!(driver.agent_preflight_calls(), 0);
        assert_eq!(driver.agent_repair_calls(), 0);
        assert_eq!(driver.window_size_calls.load(Ordering::Relaxed), 0);
        assert_eq!(driver.ordinary_session_calls(), 0);
        assert_eq!(driver.stream_restart_calls(), 0);
    }

    #[tokio::test]
    async fn mock_interaction_stream_handoff_records_an_idle_generation() {
        let driver = MockIosDriver::new();
        let generation = driver.streams.generation("MOCK-IPHONE-01");

        driver
            .confirm_interaction_stream_stopped("MOCK-IPHONE-01")
            .await
            .expect("idle handoff");
        driver
            .start_interaction_session(
                "MOCK-IPHONE-01",
                "com.ss.iphone.ugc.Ame",
                InteractionSessionKind::Ordinary,
            )
            .await
            .expect("session after handoff");

        assert_eq!(driver.streams.generation("MOCK-IPHONE-01"), generation);
        assert!(driver.mock_stream_producers.lock().is_empty());
    }

    #[tokio::test]
    async fn mock_interaction_stream_handoff_rejects_a_running_producer() {
        let driver = MockIosDriver::new();
        driver
            .ensure_stream("MOCK-IPHONE-01")
            .await
            .expect("running mock stream");

        let error = driver
            .confirm_interaction_stream_stopped("MOCK-IPHONE-01")
            .await
            .expect_err("running producer must block handoff");

        assert!(error.to_string().contains("MJPEG producer"));
        driver
            .stop_owned_stream("MOCK-IPHONE-01")
            .await
            .expect("cleanup mock stream");
    }

    #[tokio::test]
    async fn interaction_lifecycle_bootstraps_fresh_profile_before_foreground_and_session() {
        let driver = MockIosDriver::new();

        driver
            .stop_owned_stream("MOCK-IPHONE-01")
            .await
            .expect("stop old producer");
        let _session = driver
            .start_interaction_session(
                "MOCK-IPHONE-01",
                "com.ss.iphone.ugc.Ame",
                InteractionSessionKind::FreshText,
            )
            .await
            .expect("fresh interaction session");
        driver
            .start_stream_after_session("MOCK-IPHONE-01")
            .await
            .expect("reserved interaction stream");

        assert_eq!(
            driver.interaction_calls.lock().as_slice(),
            [
                "stop old producer",
                "clear/increment generation",
                "bootstrap fresh Agent",
                "foreground TikTok",
                "create/attach approved session",
                "reserve stream generation",
                "start MJPEG",
                "first decoded frame",
            ]
        );
        assert_eq!(driver.agent_preflight_calls(), 0);
        assert_eq!(driver.agent_repair_calls(), 0);
        assert_eq!(driver.window_size_calls.load(Ordering::Relaxed), 0);
        assert_eq!(driver.ordinary_session_calls(), 0);
        assert_eq!(driver.stream_restart_calls(), 0);
    }

    #[tokio::test]
    async fn interaction_lifecycle_requires_session_reservation_and_never_restores_on_failure() {
        let driver = MockIosDriver::new();

        let error = driver
            .start_stream_after_session("MOCK-IPHONE-01")
            .await
            .expect_err("stream-before-session must fail");
        assert!(error.to_string().contains("session reservation"));

        driver
            .stop_owned_stream("MOCK-IPHONE-01")
            .await
            .expect("stop old producer");
        driver
            .mock_interaction_session_failures
            .write()
            .insert("MOCK-IPHONE-01".to_string());
        let error = match driver
            .start_interaction_session(
                "MOCK-IPHONE-01",
                "com.ss.iphone.ugc.Ame",
                InteractionSessionKind::FreshText,
            )
            .await
        {
            Ok(_) => panic!("fixture session failure unexpectedly succeeded"),
            Err(error) => error,
        };

        assert!(error.to_string().contains("interaction session"));
        assert_eq!(driver.ordinary_session_calls(), 0);
        assert_eq!(driver.stream_restart_calls(), 0);
        assert_eq!(driver.agent_preflight_calls(), 0);
        assert_eq!(driver.agent_repair_calls(), 0);
        assert_eq!(driver.window_size_calls.load(Ordering::Relaxed), 0);
        assert!(!driver
            .interaction_lifecycle
            .has_session_reservation("MOCK-IPHONE-01"));
        assert!(!driver
            .interaction_calls
            .lock()
            .iter()
            .any(|call| call.contains("restore")));
    }

    #[tokio::test]
    async fn verified_process_control_is_explicit_and_preserves_pid_state() {
        let driver = MockIosDriver::new();
        let udid = "MOCK-IPHONE-01";
        let bundle_id = "com.fixture.app";

        assert!(!driver.supports_verified_app_termination("fixture-udid"));
        assert!(driver.inspect_app_process(udid, bundle_id).await.is_err());
        assert!(driver.terminate_app(udid, bundle_id).await.is_err());
        driver.set_mock_verified_app_termination(true);
        driver.set_mock_app_process(udid, bundle_id, Some(42));
        assert!(driver.supports_verified_app_termination("fixture-udid"));
        assert_eq!(
            driver
                .inspect_app_process(udid, bundle_id)
                .await
                .expect("running process"),
            riviu_core::AppProcessState {
                bundle_id: bundle_id.to_string(),
                pid: Some(42),
                running: true,
            }
        );

        assert_eq!(
            driver
                .terminate_app(udid, bundle_id)
                .await
                .expect("verified terminate"),
            riviu_core::ProcessAbsenceProof {
                bundle_id: bundle_id.to_string(),
                old_pid: Some(42),
            }
        );
        assert_eq!(
            driver
                .inspect_app_process(udid, bundle_id)
                .await
                .expect("absent after terminate")
                .pid,
            None
        );
        assert_eq!(
            driver
                .terminate_app(udid, bundle_id)
                .await
                .expect("idempotent absent terminate")
                .old_pid,
            None
        );
    }

    #[tokio::test]
    async fn target_qualified_inspection_preserves_the_requested_bundle() {
        let driver = MockIosDriver::new();

        let snapshot = driver
            .inspect_device_for_target("MOCK-IPHONE-01", "com.apple.Preferences")
            .await
            .expect("target-qualified mock inspection");

        assert_eq!(snapshot.target_app.bundle_id, "com.apple.Preferences");
        assert_eq!(snapshot.product_type, "iPhone10,1");
        assert!(snapshot.protected_auth_ready);
    }

    #[tokio::test]
    async fn direct_mock_launch_records_the_bridge_foreground() {
        let driver = MockIosDriver::new();

        driver
            .launch_app("MOCK-IPHONE-01", "com.apple.Preferences")
            .await
            .expect("mock bridge launch");

        assert_eq!(
            driver.interaction_calls.lock().as_slice(),
            ["foreground target via bridge"]
        );
    }

    #[tokio::test]
    async fn combined_mock_session_foregrounds_the_target_once() {
        let driver = MockIosDriver::new();
        driver
            .confirm_interaction_stream_stopped("MOCK-IPHONE-01")
            .await
            .expect("idle interaction handoff");

        driver
            .foreground_target_app_and_start_interaction_session(
                "MOCK-IPHONE-01",
                "com.apple.Preferences",
                InteractionSessionKind::Ordinary,
            )
            .await
            .expect("combined mock foreground and session");

        assert_eq!(
            driver.interaction_calls.lock().as_slice(),
            ["foreground TikTok", "create/attach approved session"]
        );
    }
}
