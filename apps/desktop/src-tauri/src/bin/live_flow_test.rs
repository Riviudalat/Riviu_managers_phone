use std::collections::{HashMap, HashSet};
use std::ffi::OsString;
use std::io::{Cursor, Write};
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
    DeviceDriver, DeviceInfo, DeviceRegistry, DeviceStatus, DeviceWorkCoordinator,
    FlowAggregateState, FlowArtifactStore, FlowDocumentV2, FlowRunDetail, FlowRuntime,
    FlowRuntimeDeps, FlowTargetSelection, Frame, FrameSource, FrameStream, GenerationFrame,
    GenerationFrameEvent, GenerationFrameSource, GenerationFrameStream, ImageCoordinateTarget,
    InstalledAgentIdentity, InstalledTargetIdentity, InteractionSessionKind, ProcessAbsenceProof,
    QualifiedGeometry, ScreenOrientation, StreamBudgetManager, StreamHandoffProof,
    StreamStartProof, StreamStopProof, SwipeGesture, TapPoint, UiSession,
};
use riviu_script_engine::{compile_flow, CompiledRevision};
use serde_json::{json, Value};
use uuid::Uuid;

const SETTINGS: &str = "com.apple.Preferences";
const SPRINGBOARD: &str = "com.apple.springboard";
const WAIT_DEADLINE: Duration = Duration::from_secs(30);

#[tokio::main]
async fn main() {
    let exit_code = match parse_args(std::env::args_os().skip(1)) {
        Ok(ParseOutcome::Help) => {
            print_usage();
            0
        }
        Err(error) => {
            eprintln!("{error}");
            print_usage();
            1
        }
        Ok(ParseOutcome::Run(args)) => run(args).await,
    };
    std::process::exit(exit_code);
}

async fn run(args: Args) -> i32 {
    let raw = match std::fs::read_to_string(&args.flow) {
        Ok(raw) => raw,
        Err(error) => {
            return config_failure(
                &args,
                "FlowReadFailed",
                format!("read {}: {error}", args.flow.display()),
                None,
            );
        }
    };
    let document: FlowDocumentV2 = match serde_json::from_str(&raw) {
        Ok(document) => document,
        Err(error) => {
            return config_failure(
                &args,
                "FlowJsonInvalid",
                format!("parse {}: {error}", args.flow.display()),
                None,
            );
        }
    };
    let compiled = match compile_flow(&document, &release_one_catalog()) {
        Ok(compiled) => compiled,
        Err(errors) => {
            return config_failure(
                &args,
                "FlowCompileFailed",
                "Flow document did not compile".to_string(),
                Some(json!(errors)),
            );
        }
    };

    if !args.mock {
        let summary = json!({
            "schemaVersion": 1,
            "environment": "LIVE_MAC_DEVICE",
            "mode": "real",
            "status": "pendingConfiguration",
            "flowId": document.id,
            "flowRevision": document.revision,
            "planSha256": compiled.sha256,
            "targetUdids": args.udids,
            "error": {
                "code": "LiveFlowDriverPendingConfiguration",
                "message": "Real-device Flow composition requires the Mac Device Bridge and qualified Agent runtime"
            }
        });
        let _ = write_jsonl(&args.jsonl, &summary).map_err(|error| eprintln!("{error:#}"));
        return 1;
    }

    if args.udids.len() > 2 {
        return config_failure(
            &args,
            "MockStreamLimitExceeded",
            "mock mode supports at most two selected UDIDs".to_string(),
            None,
        );
    }
    if let Err(error) = verify_mock_coordinate_profiles(&document) {
        return config_failure(
            &args,
            "MockGeometryProfileMismatch",
            error.to_string(),
            None,
        );
    }

    run_mock(args, document, compiled).await
}

async fn run_mock(args: Args, document: FlowDocumentV2, compiled: CompiledRevision) -> i32 {
    let fixture = match MockHarness::new(&args.udids, mock_snapshot()).await {
        Ok(fixture) => fixture,
        Err(error) => {
            return config_failure(&args, "MockRuntimeSetupFailed", error.to_string(), None);
        }
    };

    let execution = execute_mock(&fixture, &document, &compiled, &args.udids).await;
    let flow_shutdown = fixture.runtime.shutdown().await;
    let control_shutdown = fixture.control.shutdown_cleanup().await;
    let active_contexts = fixture.active_context_count();
    let reserved_streams = fixture.streams.reserved_capacity();
    let running_streams = fixture.streams.running_producer_count();
    let quarantined = fixture.control.cleanup_quarantine_count();

    let (run_id, aggregate_state, device_runs, attempts, artifacts, uncertain_attempts) =
        match execution.as_ref() {
            Ok(detail) => (
                Some(detail.run.id),
                Some(detail.run.state),
                json!(detail.device_runs),
                detail.attempts.len(),
                json!(detail.artifacts),
                detail
                    .attempts
                    .iter()
                    .filter(|attempt| {
                        matches!(
                            attempt.state,
                            riviu_core::FlowAttemptState::Uncertain
                                | riviu_core::FlowAttemptState::Interrupted
                        )
                    })
                    .count(),
            ),
            Err(_) => (None, None, json!([]), 0, json!([]), 0),
        };

    let execution_succeeded = execution.as_ref().is_ok_and(|detail| {
        detail.run.state == FlowAggregateState::Succeeded
            && detail.device_runs.len() == args.udids.len()
            && detail
                .device_runs
                .iter()
                .all(|device| device.state.is_success())
            && uncertain_attempts == 0
    });
    let cleanup_succeeded = flow_shutdown.is_ok()
        && control_shutdown.is_ok()
        && active_contexts == 0
        && reserved_streams == 0
        && running_streams == 0
        && quarantined == 0;
    let route_succeeded = fixture.driver.screenshot_calls() == 0
        && fixture.driver.ui_screenshot_calls() == 0
        && fixture.driver.max_reserved_capacity() <= fixture.streams.configured_limit();
    let succeeded = execution_succeeded && cleanup_succeeded && route_succeeded;

    let summary = json!({
        "schemaVersion": 1,
        "environment": "MOCK_FIXTURE",
        "mode": "mock",
        "status": if succeeded { "succeeded" } else { "failed" },
        "flowId": document.id,
        "flowRevision": document.revision,
        "planSha256": compiled.sha256,
        "runId": run_id,
        "aggregateState": aggregate_state,
        "targetUdids": args.udids,
        "deviceRuns": device_runs,
        "attemptCount": attempts,
        "uncertainAttemptCount": uncertain_attempts,
        "artifacts": artifacts,
        "legacyDriverScreenshotCalls": fixture.driver.screenshot_calls(),
        "uiScreenshotCalls": fixture.driver.ui_screenshot_calls(),
        "streamBudget": {
            "configuredLimit": fixture.streams.configured_limit(),
            "maxObservedReserved": fixture.driver.max_reserved_capacity(),
            "reservedAfterCleanup": reserved_streams,
            "runningAfterCleanup": running_streams
        },
        "cleanup": {
            "flowShutdown": result_label(&flow_shutdown),
            "controlShutdown": result_label(&control_shutdown),
            "activeContexts": active_contexts,
            "quarantinedContexts": quarantined
        },
        "error": execution.as_ref().err().map(ToString::to_string),
        "flowShutdownError": flow_shutdown.as_ref().err().map(ToString::to_string),
        "controlShutdownError": control_shutdown.as_ref().err().map(ToString::to_string)
    });

    if let Err(error) = write_jsonl(&args.jsonl, &summary) {
        eprintln!("{error:#}");
        return 2;
    }
    if succeeded {
        0
    } else {
        2
    }
}

async fn execute_mock(
    fixture: &MockHarness,
    document: &FlowDocumentV2,
    compiled: &CompiledRevision,
    udids: &[String],
) -> anyhow::Result<FlowRunDetail> {
    fixture
        .database
        .save_flow_revision(None, document, &compiled.plan, &compiled.sha256)?;
    let revision = fixture
        .database
        .get_flow_revision(document.id, Some(document.revision))?
        .ok_or_else(|| anyhow::anyhow!("persisted Flow revision disappeared"))?;
    anyhow::ensure!(
        revision.plan_hash == compiled.sha256,
        "reloaded plan hash changed"
    );
    anyhow::ensure!(
        revision.compiled_plan == compiled.plan,
        "reloaded plan changed"
    );
    let run = fixture
        .runtime
        .enqueue(
            revision,
            FlowTargetSelection::Selected {
                udids: udids.to_vec(),
            },
        )
        .await?;
    fixture.wait_terminal(run.id).await
}

fn verify_mock_coordinate_profiles(document: &FlowDocumentV2) -> anyhow::Result<()> {
    let profile_id =
        qualified_geometry_profile_id(&mock_snapshot()).map_err(|error| anyhow::anyhow!(error))?;
    for node in &document.nodes {
        match node.kind {
            ActionKind::Swipe => {
                for field in ["from", "to"] {
                    let target: ImageCoordinateTarget = serde_json::from_value(
                        node.config
                            .get(field)
                            .cloned()
                            .ok_or_else(|| anyhow::anyhow!("Swipe.{field} is absent"))?,
                    )?;
                    anyhow::ensure!(
                        target.profile_id == profile_id,
                        "Swipe.{field} profile does not match the mock snapshot"
                    );
                }
            }
            ActionKind::Tap if node.config.get("point").is_some() => {
                let target: ImageCoordinateTarget =
                    serde_json::from_value(node.config["point"].clone())?;
                anyhow::ensure!(
                    target.profile_id == profile_id,
                    "Tap.point profile does not match the mock snapshot"
                );
            }
            _ => {}
        }
    }
    Ok(())
}

fn result_label<T, E>(result: &Result<T, E>) -> &'static str {
    if result.is_ok() {
        "pass"
    } else {
        "fail"
    }
}

fn config_failure(args: &Args, code: &str, message: String, details: Option<Value>) -> i32 {
    let summary = json!({
        "schemaVersion": 1,
        "environment": if args.mock { "MOCK_FIXTURE" } else { "LIVE_MAC_DEVICE" },
        "mode": if args.mock { "mock" } else { "real" },
        "status": "configurationError",
        "targetUdids": args.udids,
        "error": { "code": code, "message": message, "details": details }
    });
    if let Err(error) = write_jsonl(&args.jsonl, &summary) {
        eprintln!("{error:#}");
    }
    1
}

fn write_jsonl(path: &Path, summary: &Value) -> anyhow::Result<()> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent)?;
    }
    let mut file = std::fs::File::create(path)?;
    serde_json::to_writer(&mut file, summary)?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    Ok(())
}

#[derive(Debug)]
struct Args {
    flow: PathBuf,
    udids: Vec<String>,
    jsonl: PathBuf,
    mock: bool,
}

enum ParseOutcome {
    Run(Args),
    Help,
}

fn parse_args(values: impl Iterator<Item = OsString>) -> Result<ParseOutcome, String> {
    let mut values = values.peekable();
    let mut flow = None;
    let mut udids = Vec::new();
    let mut jsonl = None;
    let mut mock = false;
    while let Some(value) = values.next() {
        let flag = value
            .to_str()
            .ok_or_else(|| "arguments must be valid UTF-8 flags".to_string())?;
        match flag {
            "--help" | "-h" => return Ok(ParseOutcome::Help),
            "--flow" => set_once(
                &mut flow,
                PathBuf::from(required_value(&mut values, "--flow")?),
                "--flow",
            )?,
            "--udid" => {
                let udid = required_value(&mut values, "--udid")?
                    .into_string()
                    .map_err(|_| "--udid must be valid UTF-8".to_string())?;
                if udid.trim().is_empty() || udid.trim() != udid {
                    return Err("--udid must be non-empty without surrounding whitespace".into());
                }
                udids.push(udid);
            }
            "--jsonl" => set_once(
                &mut jsonl,
                PathBuf::from(required_value(&mut values, "--jsonl")?),
                "--jsonl",
            )?,
            "--mock" if !mock => mock = true,
            "--mock" => return Err("--mock may be supplied once".into()),
            _ => return Err(format!("unknown argument {flag}")),
        }
    }
    let flow = flow.ok_or_else(|| "--flow is required".to_string())?;
    let jsonl = jsonl.ok_or_else(|| "--jsonl is required".to_string())?;
    if udids.is_empty() {
        return Err("at least one --udid is required".into());
    }
    let unique = udids.iter().collect::<HashSet<_>>();
    if unique.len() != udids.len() {
        return Err("--udid values must be unique".into());
    }
    Ok(ParseOutcome::Run(Args {
        flow,
        udids,
        jsonl,
        mock,
    }))
}

fn required_value(
    values: &mut impl Iterator<Item = OsString>,
    flag: &str,
) -> Result<OsString, String> {
    values
        .next()
        .ok_or_else(|| format!("{flag} requires a value"))
}

fn set_once<T>(slot: &mut Option<T>, value: T, flag: &str) -> Result<(), String> {
    if slot.replace(value).is_some() {
        Err(format!("{flag} may be supplied once"))
    } else {
        Ok(())
    }
}

fn print_usage() {
    eprintln!(
        "Usage: live_flow_test --flow <flow.json> --udid <UDID> [--udid <UDID> ...] --jsonl <output.jsonl> [--mock]"
    );
}

struct MockHarness {
    runtime: FlowRuntime,
    database: Arc<Database>,
    driver: Arc<FixtureDriver>,
    control: Arc<DeviceControlPlane>,
    work: Arc<DeviceWorkCoordinator>,
    streams: Arc<StreamBudgetManager>,
    udids: Vec<String>,
}

impl MockHarness {
    async fn new(udids: &[String], snapshot: DeviceCapabilitySnapshot) -> anyhow::Result<Self> {
        let events = riviu_core::EventBus::new(128);
        let registry = DeviceRegistry::new(events.clone());
        registry.upsert_many(udids.iter().map(|udid| mock_device(udid)).collect());
        let database = Arc::new(Database::open(
            std::env::temp_dir().join(format!("riviu-live-flow-mock-{}.db", Uuid::new_v4())),
        )?);
        let artifacts = FlowArtifactStore::new(
            std::env::temp_dir().join(format!("riviu-live-flow-mock-artifacts-{}", Uuid::new_v4())),
        )?;
        let work = Arc::new(DeviceWorkCoordinator::new());
        let streams = Arc::new(StreamBudgetManager::new(2)?);
        let driver = Arc::new(FixtureDriver::new(udids, snapshot, streams.clone()));
        let control = Arc::new(DeviceControlPlane::new(
            driver.clone(),
            work.clone(),
            streams.clone(),
        ));
        let frames = Arc::new(FixtureFrames::new(udids));
        let runtime = FlowRuntime::new(FlowRuntimeDeps {
            database: database.clone(),
            events,
            registry,
            control: control.clone(),
            frames,
            artifacts,
        });
        runtime.recover_startup().await?;
        Ok(Self {
            runtime,
            database,
            driver,
            control,
            work,
            streams,
            udids: udids.to_vec(),
        })
    }

    async fn wait_terminal(&self, run_id: Uuid) -> anyhow::Result<FlowRunDetail> {
        let deadline = tokio::time::Instant::now() + WAIT_DEADLINE;
        loop {
            if let Some(detail) = self.database.get_flow_run(run_id)? {
                if detail.run.state.is_terminal() {
                    return Ok(detail);
                }
            }
            if tokio::time::Instant::now() >= deadline {
                anyhow::bail!("Flow run did not become terminal before the harness deadline");
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
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
            max_reserved_capacity: AtomicUsize::new(0),
        }
    }

    fn require_udid(&self, udid: &str) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.udids.iter().any(|candidate| candidate == udid),
            "unknown mock UDID {udid}"
        );
        Ok(())
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

    fn max_reserved_capacity(&self) -> usize {
        self.max_reserved_capacity.load(Ordering::SeqCst)
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

    fn supports_verified_app_termination(&self, _udid: &str) -> bool {
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
        Self {
            inner: Arc::new(Mutex::new(
                udids
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
                    .collect(),
            )),
        }
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
        os_version: "16.7.15".into(),
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
        platform: riviu_core::DevicePlatform::Ios,
        os_version: "16.7.15".into(),
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
        .expect("encode mock JPEG");
    Arc::new(bytes.into_inner())
}
