use std::collections::{HashMap, HashSet};
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use image::{DynamicImage, ImageFormat, Rgb, RgbImage};
use parking_lot::Mutex;
use riviu_core::db::Database;
use riviu_core::{
    qualified_geometry_profile_id, release_one_catalog, ActionKind, ActiveTransport, AgentState,
    AgentStatus, AppProcessState, ConnectionKind, DeviceCapabilitySnapshot, DeviceControlPlane,
    DeviceDriver, DeviceInfo, DeviceRegistry, DeviceStatus, DeviceWorkCoordinator, DeviceWorkOwner,
    FlowAggregateState, FlowArtifactStore, FlowAttemptState, FlowDeviceRunState, FlowDocumentV2,
    FlowRunDetail, FlowRuntime, FlowRuntimeDeps, FlowTargetSelection, Frame, FrameSource,
    FrameStream, GenerationFrame, GenerationFrameEvent, GenerationFrameSource,
    GenerationFrameStream, ImageCoordinateTarget, InstalledAgentIdentity, InstalledTargetIdentity,
    InteractionSessionKind, ProcessAbsenceProof, QualifiedGeometry, ScreenOrientation,
    StreamBudgetManager, StreamHandoffProof, StreamStartProof, StreamStopProof, SwipeGesture,
    TapPoint, UiSession,
};
use riviu_script_engine::compile_flow;
use sha2::{Digest, Sha256};
use uuid::Uuid;

const SETTINGS: &str = "com.apple.Preferences";
const SPRINGBOARD: &str = "com.apple.springboard";
const MOCK_UDIDS: [&str; 2] = ["MOCK-IPHONE-01", "MOCK-IPHONE-02"];
const TERMINAL_WAIT_TIMEOUT: Duration = Duration::from_secs(30);
const EXPECTED_PLAN_SHA256: &str =
    "0cade80dae4ddbf361413ea582680a4c849c9698490bc176e9ec3344247f68b2";

#[tokio::test]
async fn release_one_fixture_runs_two_devices_without_shared_attempts() {
    let document: FlowDocumentV2 =
        serde_json::from_str(include_str!("../../../docs/fixtures/flow-release-one.json"))
            .expect("fixture JSON");
    let snapshot = mock_snapshot();
    let profile_id = qualified_geometry_profile_id(&snapshot).expect("profile ID");
    let swipe = document
        .nodes
        .iter()
        .find(|node| node.kind == ActionKind::Swipe)
        .expect("fixture Swipe");
    let from: ImageCoordinateTarget =
        serde_json::from_value(swipe.config.get("from").cloned().expect("Swipe.from"))
            .expect("typed Swipe.from");
    let to: ImageCoordinateTarget =
        serde_json::from_value(swipe.config.get("to").cloned().expect("Swipe.to"))
            .expect("typed Swipe.to");
    assert_eq!(from.profile_id, profile_id);
    assert_eq!(to.profile_id, profile_id);

    let compiled = compile_flow(&document, &release_one_catalog()).expect("compile");
    assert_eq!(compiled.sha256, EXPECTED_PLAN_SHA256);
    let fixture = MockFlowRuntimeFixture::new(&MOCK_UDIDS, snapshot)
        .await
        .expect("mock runtime");
    fixture
        .database
        .save_flow_revision(None, &document, &compiled.plan, &compiled.sha256)
        .expect("persist immutable revision");
    let revision = fixture
        .database
        .get_flow_revision(document.id, Some(document.revision))
        .expect("reload revision")
        .expect("saved revision");
    assert_eq!(revision.plan_hash, compiled.sha256);
    assert_eq!(revision.compiled_plan, compiled.plan);

    let run = fixture
        .runtime
        .enqueue(
            revision,
            FlowTargetSelection::Selected {
                udids: MOCK_UDIDS.iter().map(|udid| (*udid).to_string()).collect(),
            },
        )
        .await
        .expect("enqueue");
    assert_eq!(run.plan_sha256, compiled.sha256);
    let detail = fixture.wait_terminal(run.id).await.expect("terminal run");

    assert_eq!(detail.run.state, FlowAggregateState::Succeeded);
    assert_eq!(detail.run.plan_sha256, compiled.sha256);
    assert_eq!(detail.device_runs.len(), 2);
    assert!(detail
        .device_runs
        .iter()
        .all(|device| device.state == FlowDeviceRunState::Succeeded));
    assert!(detail.device_runs.iter().all(|device| {
        device.release_proof.as_ref().is_some_and(|proof| {
            proof.udid == device.udid
                && proof.owner == DeviceWorkOwner::Script
                && proof.had_session
                && proof.had_stream
        })
    }));

    let attempt_ids = detail
        .attempts
        .iter()
        .map(|attempt| attempt.id)
        .collect::<HashSet<_>>();
    assert_eq!(
        detail.attempts.len(),
        document.nodes.len() * MOCK_UDIDS.len()
    );
    assert_eq!(attempt_ids.len(), detail.attempts.len());
    for device in &detail.device_runs {
        assert_eq!(
            detail
                .attempts
                .iter()
                .filter(|attempt| attempt.device_run_id == device.id)
                .count(),
            document.nodes.len()
        );
    }

    assert_eq!(detail.artifacts.len(), 2);
    let expected_artifact_hash = fixture.frames.post_sha256();
    assert!(detail.artifacts.iter().all(|artifact| {
        artifact.kind == "jpeg"
            && artifact.label == "release-one"
            && artifact.sha256 == expected_artifact_hash
            && artifact.size > 0
    }));

    let terminate_attempts = detail
        .attempts
        .iter()
        .filter(|attempt| attempt.action_kind == ActionKind::TerminateApp)
        .filter(|attempt| attempt.state == FlowAttemptState::Succeeded)
        .collect::<Vec<_>>();
    assert_eq!(terminate_attempts.len(), 2);
    assert!(terminate_attempts.iter().all(|attempt| {
        let evidence = attempt
            .evidence_result
            .as_ref()
            .expect("Terminate evidence");
        evidence.get("kind").and_then(serde_json::Value::as_str) == Some("processAbsent")
            && evidence.get("matched").and_then(serde_json::Value::as_bool) == Some(true)
    }));

    assert_eq!(fixture.driver.screenshot_calls(), 0);
    assert_eq!(fixture.driver.ui_screenshot_calls(), 0);
    assert_eq!(fixture.driver.terminate_calls(), 2);
    assert!(fixture.driver.max_reserved_capacity() > 0);
    assert!(
        fixture.driver.max_reserved_capacity() <= fixture.streams.configured_limit(),
        "observed stream budget exceeded its configured limit"
    );

    fixture.shutdown().await.expect("joined cleanup");
    assert_eq!(fixture.active_context_count(), 0);
    assert_eq!(fixture.streams.reserved_capacity(), 0);
    assert_eq!(fixture.streams.running_producer_count(), 0);
    assert_eq!(fixture.control.cleanup_quarantine_count(), 0);
}

struct MockFlowRuntimeFixture {
    runtime: FlowRuntime,
    database: Arc<Database>,
    driver: Arc<FixtureDriver>,
    frames: Arc<FixtureFrames>,
    control: Arc<DeviceControlPlane>,
    work: Arc<DeviceWorkCoordinator>,
    streams: Arc<StreamBudgetManager>,
    udids: Vec<String>,
}

impl MockFlowRuntimeFixture {
    async fn new(udids: &[&str], snapshot: DeviceCapabilitySnapshot) -> anyhow::Result<Self> {
        let udids = udids
            .iter()
            .map(|udid| (*udid).to_string())
            .collect::<Vec<_>>();
        let events = riviu_core::EventBus::new(128);
        let registry = DeviceRegistry::new(events.clone());
        registry.upsert_many(udids.iter().map(|udid| mock_device(udid)).collect());

        let database_path =
            std::env::temp_dir().join(format!("riviu-flow-release-one-{}.db", Uuid::new_v4()));
        let database = Arc::new(Database::open(database_path)?);
        let artifact_root = std::env::temp_dir().join(format!(
            "riviu-flow-release-one-artifacts-{}",
            Uuid::new_v4()
        ));
        let artifacts = FlowArtifactStore::new(artifact_root)?;
        let work = Arc::new(DeviceWorkCoordinator::new());
        let streams = Arc::new(StreamBudgetManager::new(2)?);
        let driver = Arc::new(FixtureDriver::new(&udids, snapshot, streams.clone()));
        let control = Arc::new(DeviceControlPlane::new(
            driver.clone(),
            work.clone(),
            streams.clone(),
        ));
        let frames = Arc::new(FixtureFrames::new(&udids));
        let runtime = FlowRuntime::new(FlowRuntimeDeps {
            database: database.clone(),
            events,
            registry,
            control: control.clone(),
            frames: frames.clone(),
            artifacts,
        });
        runtime.recover_startup().await?;

        Ok(Self {
            runtime,
            database,
            driver,
            frames,
            control,
            work,
            streams,
            udids,
        })
    }

    async fn wait_terminal(&self, run_id: Uuid) -> anyhow::Result<FlowRunDetail> {
        let deadline = tokio::time::Instant::now() + TERMINAL_WAIT_TIMEOUT;
        loop {
            if let Some(detail) = self.database.get_flow_run(run_id)? {
                if detail.run.state.is_terminal() {
                    return Ok(detail);
                }
            }
            if tokio::time::Instant::now() >= deadline {
                anyhow::bail!(
                    "flow run did not become terminal before the {} second deadline",
                    TERMINAL_WAIT_TIMEOUT.as_secs()
                );
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    async fn shutdown(&self) -> anyhow::Result<()> {
        self.runtime.shutdown().await?;
        self.control.shutdown_cleanup().await?;
        Ok(())
    }

    fn active_context_count(&self) -> usize {
        self.udids
            .iter()
            .filter(|udid| self.work.current_owner(udid).is_some())
            .count()
    }
}

struct FixtureDriver {
    udids: Vec<String>,
    snapshot: DeviceCapabilitySnapshot,
    active_apps: Arc<Mutex<HashMap<String, String>>>,
    processes: Mutex<HashMap<(String, String), u64>>,
    generations: Mutex<HashMap<String, u64>>,
    streams: Arc<StreamBudgetManager>,
    screenshot_calls: AtomicUsize,
    ui_screenshot_calls: Arc<AtomicUsize>,
    terminate_calls: AtomicUsize,
    max_reserved_capacity: AtomicUsize,
}

impl FixtureDriver {
    fn new(
        udids: &[String],
        snapshot: DeviceCapabilitySnapshot,
        streams: Arc<StreamBudgetManager>,
    ) -> Self {
        Self {
            udids: udids.to_vec(),
            snapshot,
            active_apps: Arc::new(Mutex::new(HashMap::new())),
            processes: Mutex::new(HashMap::new()),
            generations: Mutex::new(udids.iter().map(|udid| (udid.clone(), 1)).collect()),
            streams,
            screenshot_calls: AtomicUsize::new(0),
            ui_screenshot_calls: Arc::new(AtomicUsize::new(0)),
            terminate_calls: AtomicUsize::new(0),
            max_reserved_capacity: AtomicUsize::new(0),
        }
    }

    fn observe_stream_budget(&self) {
        self.max_reserved_capacity
            .fetch_max(self.streams.reserved_capacity(), Ordering::SeqCst);
    }

    fn screenshot_calls(&self) -> usize {
        self.screenshot_calls.load(Ordering::SeqCst)
    }

    fn ui_screenshot_calls(&self) -> usize {
        self.ui_screenshot_calls.load(Ordering::SeqCst)
    }

    fn terminate_calls(&self) -> usize {
        self.terminate_calls.load(Ordering::SeqCst)
    }

    fn max_reserved_capacity(&self) -> usize {
        self.max_reserved_capacity.load(Ordering::SeqCst)
    }

    fn require_udid(&self, udid: &str) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.udids.iter().any(|candidate| candidate == udid),
            "unknown mock UDID {udid}"
        );
        Ok(())
    }
}

#[async_trait]
impl DeviceDriver for FixtureDriver {
    fn cached_agent_status(&self, udid: &str) -> AgentStatus {
        AgentStatus {
            udid: udid.to_string(),
            state: AgentState::Ready,
            artifact_id: "fixture-agent".into(),
            artifact_version: "1.0".into(),
            bundle_id: "com.mrph.svc".into(),
            protocol_version: 1,
            features: vec!["stream".into(), "tap".into(), "swipe".into()],
            installed_version: Some("1.0".into()),
            installed_build: Some("1".into()),
            auth_ready: true,
            mjpeg_ready: true,
            session_ready: false,
            message: None,
        }
    }

    async fn inspect_device_for_target(
        &self,
        udid: &str,
        target_bundle_id: &str,
    ) -> anyhow::Result<DeviceCapabilitySnapshot> {
        self.require_udid(udid)?;
        anyhow::ensure!(target_bundle_id == SETTINGS, "unexpected target bundle");
        Ok(self.snapshot.clone())
    }

    fn supports_verified_app_termination(&self) -> bool {
        true
    }

    async fn inspect_app_process(
        &self,
        udid: &str,
        bundle_id: &str,
    ) -> anyhow::Result<AppProcessState> {
        self.require_udid(udid)?;
        let pid = self
            .processes
            .lock()
            .get(&(udid.to_string(), bundle_id.to_string()))
            .copied();
        Ok(AppProcessState {
            bundle_id: bundle_id.to_string(),
            pid,
            running: pid.is_some(),
        })
    }

    async fn confirm_interaction_stream_stopped(
        &self,
        udid: &str,
    ) -> anyhow::Result<StreamHandoffProof> {
        self.require_udid(udid)?;
        Ok(StreamHandoffProof {
            generation: *self.generations.lock().get(udid).expect("known generation"),
        })
    }

    async fn start_interaction_session(
        &self,
        udid: &str,
        _bundle_id: &str,
        _kind: InteractionSessionKind,
    ) -> anyhow::Result<Box<dyn UiSession>> {
        self.require_udid(udid)?;
        self.observe_stream_budget();
        Ok(Box::new(FixtureSession {
            udid: udid.to_string(),
            active_apps: self.active_apps.clone(),
            ui_screenshot_calls: self.ui_screenshot_calls.clone(),
        }))
    }

    async fn start_stream_after_session(&self, udid: &str) -> anyhow::Result<StreamStartProof> {
        self.require_udid(udid)?;
        self.observe_stream_budget();
        let generation = *self.generations.lock().get(udid).expect("known generation");
        Ok(StreamStartProof {
            generation,
            first_frame_observed: true,
            stream_url: format!("mock://{udid}/generation/{generation}"),
        })
    }

    async fn stop_owned_stream(&self, udid: &str) -> anyhow::Result<StreamStopProof> {
        self.require_udid(udid)?;
        let mut generations = self.generations.lock();
        let generation = generations.get_mut(udid).expect("known generation");
        let old_generation = *generation;
        *generation += 1;
        Ok(StreamStopProof {
            old_generation,
            new_generation: *generation,
            child_stopped: true,
        })
    }

    async fn read_active_app_bundle(&self, udid: &str) -> anyhow::Result<String> {
        self.require_udid(udid)?;
        self.active_apps
            .lock()
            .get(udid)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("mock active app is absent"))
    }

    async fn list_devices(&self) -> anyhow::Result<Vec<DeviceInfo>> {
        Ok(self.udids.iter().map(|udid| mock_device(udid)).collect())
    }

    async fn refresh_device(&self, udid: &str) -> anyhow::Result<DeviceInfo> {
        self.require_udid(udid)?;
        Ok(mock_device(udid))
    }

    async fn install_app(&self, _udid: &str, _path: &Path) -> anyhow::Result<()> {
        Ok(())
    }

    async fn uninstall_app(&self, _udid: &str, _bundle_id: &str) -> anyhow::Result<()> {
        Ok(())
    }

    async fn screenshot(&self, _udid: &str, _dest: &Path) -> anyhow::Result<PathBuf> {
        self.screenshot_calls.fetch_add(1, Ordering::SeqCst);
        anyhow::bail!("Flow must use generation-qualified stream frames")
    }

    async fn syslog_tail(&self, _udid: &str, _lines: usize) -> anyhow::Result<String> {
        Ok(String::new())
    }

    async fn launch_app(&self, udid: &str, bundle_id: &str) -> anyhow::Result<()> {
        self.require_udid(udid)?;
        self.observe_stream_budget();
        self.active_apps
            .lock()
            .insert(udid.to_string(), bundle_id.to_string());
        let index = self
            .udids
            .iter()
            .position(|candidate| candidate == udid)
            .expect("known UDID");
        self.processes.lock().insert(
            (udid.to_string(), bundle_id.to_string()),
            1_000 + u64::try_from(index).expect("fixture index"),
        );
        Ok(())
    }

    async fn terminate_app(
        &self,
        udid: &str,
        bundle_id: &str,
    ) -> anyhow::Result<ProcessAbsenceProof> {
        self.require_udid(udid)?;
        self.terminate_calls.fetch_add(1, Ordering::SeqCst);
        let old_pid = self
            .processes
            .lock()
            .remove(&(udid.to_string(), bundle_id.to_string()));
        Ok(ProcessAbsenceProof {
            bundle_id: bundle_id.to_string(),
            old_pid,
        })
    }

    async fn reboot(&self, _udid: &str) -> anyhow::Result<()> {
        Ok(())
    }

    async fn start_ui_session(&self, _udid: &str) -> anyhow::Result<Box<dyn UiSession>> {
        anyhow::bail!("Flow must use the interaction session transition")
    }

    fn invalidate_ui_session(&self, _udid: &str) {}

    async fn ensure_stream(&self, _udid: &str) -> anyhow::Result<String> {
        anyhow::bail!("Flow must use start_stream_after_session")
    }

    async fn prepare_device(&self, _udid: &str) -> anyhow::Result<()> {
        Ok(())
    }
}

struct FixtureSession {
    udid: String,
    active_apps: Arc<Mutex<HashMap<String, String>>>,
    ui_screenshot_calls: Arc<AtomicUsize>,
}

#[async_trait]
impl UiSession for FixtureSession {
    async fn tap(&self, _point: TapPoint) -> anyhow::Result<()> {
        Ok(())
    }

    async fn swipe(&self, _gesture: SwipeGesture) -> anyhow::Result<()> {
        Ok(())
    }

    async fn type_text(&self, _text: &str) -> anyhow::Result<()> {
        Ok(())
    }

    async fn home(&self) -> anyhow::Result<()> {
        self.active_apps
            .lock()
            .insert(self.udid.clone(), SPRINGBOARD.to_string());
        Ok(())
    }

    async fn find_and_tap(&self, _accessibility_id: &str) -> anyhow::Result<()> {
        Ok(())
    }

    async fn assert_visible(&self, _accessibility_id: &str) -> anyhow::Result<()> {
        Ok(())
    }

    async fn launch_app_foreground(&self, bundle_id: &str) -> anyhow::Result<()> {
        self.active_apps
            .lock()
            .insert(self.udid.clone(), bundle_id.to_string());
        Ok(())
    }

    async fn active_app_bundle(&self) -> anyhow::Result<String> {
        self.active_apps
            .lock()
            .get(&self.udid)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("mock active app is absent"))
    }

    async fn screenshot_png(&self) -> anyhow::Result<Vec<u8>> {
        self.ui_screenshot_calls.fetch_add(1, Ordering::SeqCst);
        anyhow::bail!("Flow must not poll the UI screenshot route")
    }

    fn stream_url(&self) -> Option<String> {
        Some(format!("mock://{}/stream", self.udid))
    }
}

#[derive(Clone)]
struct FixtureFrames {
    inner: Arc<Mutex<HashMap<String, FixtureFrameState>>>,
    post: Frame,
}

#[derive(Clone)]
struct FixtureFrameState {
    latest: GenerationFrame,
    post: GenerationFrame,
}

impl FixtureFrames {
    fn new(udids: &[String]) -> Self {
        let baseline = jpeg(24);
        let post = jpeg(224);
        let inner = udids
            .iter()
            .map(|udid| {
                (
                    udid.clone(),
                    FixtureFrameState {
                        latest: GenerationFrame {
                            generation: 1,
                            sequence: 1,
                            bytes: baseline.clone(),
                        },
                        post: GenerationFrame {
                            generation: 1,
                            sequence: 2,
                            bytes: post.clone(),
                        },
                    },
                )
            })
            .collect();
        Self {
            inner: Arc::new(Mutex::new(inner)),
            post,
        }
    }

    fn post_sha256(&self) -> String {
        format!("{:x}", Sha256::digest(self.post.as_ref()))
    }
}

struct FixtureFrameStream {
    frame: Option<Frame>,
}

#[async_trait]
impl FrameStream for FixtureFrameStream {
    async fn next(&mut self) -> Option<Frame> {
        self.frame.take()
    }
}

impl FrameSource for FixtureFrames {
    fn subscribe(&self, udid: &str) -> Box<dyn FrameStream> {
        Box::new(FixtureFrameStream {
            frame: self.latest(udid),
        })
    }

    fn latest(&self, udid: &str) -> Option<Frame> {
        self.inner
            .lock()
            .get(udid)
            .map(|state| state.latest.bytes.clone())
    }
}

struct FixtureGenerationStream {
    inner: Arc<Mutex<HashMap<String, FixtureFrameState>>>,
    udid: String,
    generation: u64,
    watermark: Option<u64>,
    delivered: bool,
}

#[async_trait]
impl GenerationFrameStream for FixtureGenerationStream {
    async fn next(&mut self) -> GenerationFrameEvent {
        if self.delivered {
            return GenerationFrameEvent::Closed;
        }
        self.delivered = true;
        let mut frames = self.inner.lock();
        let Some(state) = frames.get_mut(&self.udid) else {
            return GenerationFrameEvent::Closed;
        };
        if state.latest.generation != self.generation {
            return GenerationFrameEvent::Advanced {
                expected: self.generation,
                actual: state.latest.generation,
            };
        }
        if self
            .watermark
            .is_some_and(|watermark| state.post.sequence <= watermark)
        {
            return GenerationFrameEvent::Closed;
        }
        state.latest = state.post.clone();
        GenerationFrameEvent::Frame(state.latest.clone())
    }
}

impl GenerationFrameSource for FixtureFrames {
    fn subscribe_generation(&self, udid: &str, generation: u64) -> Box<dyn GenerationFrameStream> {
        let watermark = self
            .inner
            .lock()
            .get(udid)
            .filter(|state| state.latest.generation == generation)
            .map(|state| state.latest.sequence);
        Box::new(FixtureGenerationStream {
            inner: self.inner.clone(),
            udid: udid.to_string(),
            generation,
            watermark,
            delivered: false,
        })
    }

    fn latest_in_generation(&self, udid: &str, generation: u64) -> Option<GenerationFrame> {
        self.inner
            .lock()
            .get(udid)
            .filter(|state| state.latest.generation == generation)
            .map(|state| state.latest.clone())
    }
}

fn mock_snapshot() -> DeviceCapabilitySnapshot {
    DeviceCapabilitySnapshot {
        installed_agent: InstalledAgentIdentity {
            bundle_id: "com.mrph.svc".into(),
            version: "1.0".into(),
            build: "1".into(),
            executable_name: "fixture-agent".into(),
            signer_identity_sha256: "22".repeat(32),
        },
        selected_artifact_sha256: "33".repeat(32),
        agent_version: "1.0".into(),
        protocol_version: 1,
        driver_adapter_version: "fixture-driver-1".into(),
        transport: ActiveTransport::Mock,
        product_type: "iPhone10,1".into(),
        ios_version: "16.7.15".into(),
        target_app: InstalledTargetIdentity {
            bundle_id: SETTINGS.into(),
            version: "1".into(),
            build: "1".into(),
        },
        protected_auth_ready: true,
        geometry: Some(QualifiedGeometry {
            logical_width: 375.0,
            logical_height: 667.0,
            pixel_width: 375,
            pixel_height: 667,
            scale_x: 1.0,
            scale_y: 1.0,
            orientation: ScreenOrientation::Portrait,
        }),
    }
}

fn mock_device(udid: &str) -> DeviceInfo {
    DeviceInfo {
        udid: udid.to_string(),
        name: format!("Fixture {udid}"),
        model: "iPhone 8".into(),
        ios_version: "16.7.15".into(),
        connection: ConnectionKind::Mock,
        status: DeviceStatus::Ready,
        battery: Some(100),
        wda_ready: true,
        wda_expires_at: None,
        stream_url: None,
        tile_stream_state: Default::default(),
        last_error: None,
    }
}

fn jpeg(color: u8) -> Frame {
    let image = RgbImage::from_pixel(375, 667, Rgb([color, color, color]));
    let mut bytes = Cursor::new(Vec::new());
    DynamicImage::ImageRgb8(image)
        .write_to(&mut bytes, ImageFormat::Jpeg)
        .expect("encode fixture JPEG");
    Arc::new(bytes.into_inner())
}
