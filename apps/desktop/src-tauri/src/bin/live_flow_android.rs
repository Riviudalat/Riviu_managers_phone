//! Run one real Flow, on one real Android phone, through the shipped runtime.
//!
//! ```text
//! RIVIU_ADB_PATH=… cargo run -p riviu-managers-phone --bin live_flow_android -- <serial>
//! RIVIU_ADB_PATH=… cargo run -p riviu-managers-phone --bin live_flow_android -- <serial> \
//!   --package com.riviu.fixture.autoswipe --auto-swipe
//! ```
//!
//! This exists because B1's fix cannot be proved anywhere else. `AndroidDriver` had no
//! `inspect_device_for_target`, so the trait default answered `unsupported` and Flow
//! refused at preflight on every Android device — while the UI went on listing all of them
//! as valid targets. Unit tests can prove the snapshot is assembled correctly from fixed
//! facts; only a phone proves the facts can be read off it, and only the real `FlowRuntime`
//! proves preflight then admits the device and a node actually dispatches.
//!
//! What it runs by default is deliberately the smallest flow that exercises the whole gate:
//! `Start → LaunchApp(TikTok) → End`. `LaunchApp` is `ResourceClass::UiSession`, so the
//! run needs an exclusive lease, a live agent and a geometry profile — everything a
//! bridge-only flow would skip. Its side effect is opening an app that is already on the
//! phone, and its evidence is `ActiveAppEquals`, which the driver reads back from
//! `mCurrentFocus`. Nothing is typed or posted.
//!
//! `--tap` inserts an image-coordinate Tap in the middle, which is the *other* thing that
//! could not work on Android. The runtime refuses to dispatch a coordinate unless the live
//! frame matches the device's qualified geometry, and this driver's minicap producer ran at
//! half scale, so the check could never pass. Adding the flag is how that is proved rather
//! than asserted — the target is built from the device's own snapshot, so a failure here is
//! the runtime frame disagreeing, never the plan. `--auto-swipe` requires an explicit
//! `--package` fixture and inserts three bounded custom swipes instead of a tap.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use riviu_core::db::Database;
use riviu_core::{
    release_one_catalog, ActionKind, DeviceControlPlane, DeviceDriver, DeviceRegistry,
    DeviceWorkCoordinator, EventBus, EvidenceSpec, FlowAggregateState, FlowArtifactStore,
    FlowDocumentV2, FlowEdge, FlowNode, FlowRuntime, FlowRuntimeDeps, FlowTargetSelection,
    ScreenOrientation,
};
use riviu_ios_driver::StreamHub;
use riviu_script_engine::compile_flow;
use serde_json::json;
use uuid::Uuid;

const WAIT_DEADLINE: Duration = Duration::from_secs(120);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LiveMode {
    LaunchOnly,
    Tap,
    AutoSwipe,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct LiveArgs {
    serial: String,
    package: Option<String>,
    mode: LiveMode,
}

fn parse_args(arguments: impl IntoIterator<Item = String>) -> anyhow::Result<LiveArgs> {
    let mut arguments = arguments.into_iter();
    let mut serial = None;
    let mut package = None;
    let mut mode = LiveMode::LaunchOnly;
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--package" => {
                anyhow::ensure!(package.is_none(), "--package may only be specified once");
                let value = arguments
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("--package requires an application id"))?;
                anyhow::ensure!(
                    !value.is_empty() && !value.starts_with("--"),
                    "--package requires an application id"
                );
                package = Some(value);
            }
            "--tap" => {
                anyhow::ensure!(mode == LiveMode::LaunchOnly, "choose one live action mode");
                mode = LiveMode::Tap;
            }
            "--auto-swipe" => {
                anyhow::ensure!(mode == LiveMode::LaunchOnly, "choose one live action mode");
                mode = LiveMode::AutoSwipe;
            }
            value if value.starts_with("--") => anyhow::bail!("unknown option {value}"),
            value => {
                anyhow::ensure!(serial.is_none(), "only one device serial is accepted");
                serial = Some(value.to_string());
            }
        }
    }
    let serial = serial.ok_or_else(|| anyhow::anyhow!("device serial is required"))?;
    anyhow::ensure!(
        mode != LiveMode::AutoSwipe || package.is_some(),
        "--auto-swipe requires an explicit --package fixture"
    );
    Ok(LiveArgs {
        serial,
        package,
        mode,
    })
}

struct ScratchDirectory {
    path: Option<PathBuf>,
}

impl ScratchDirectory {
    fn create(path: PathBuf) -> std::io::Result<Self> {
        std::fs::create_dir_all(&path)?;
        Ok(Self { path: Some(path) })
    }

    fn path(&self) -> &Path {
        self.path.as_deref().expect("scratch directory is active")
    }

    fn cleanup(mut self) -> std::io::Result<()> {
        let path = self.path.take().expect("scratch directory is active");
        std::fs::remove_dir_all(path)
    }
}

impl Drop for ScratchDirectory {
    fn drop(&mut self) {
        if let Some(path) = self.path.take() {
            let _ = std::fs::remove_dir_all(path);
        }
    }
}

fn require_succeeded(state: FlowAggregateState) -> anyhow::Result<()> {
    anyhow::ensure!(
        state == FlowAggregateState::Succeeded,
        "live flow ended in {state:?}"
    );
    Ok(())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = match parse_args(std::env::args().skip(1)) {
        Ok(args) => args,
        Err(error) => {
            eprintln!(
                "usage: live_flow_android <serial> [--package <application-id>] [--tap | --auto-swipe]\n{error}"
            );
            std::process::exit(2);
        }
    };
    let serial = args.serial;

    let config = riviu_android_driver::AndroidDriverConfig::default();
    let android = Arc::new(riviu_android_driver::AndroidDriver::new(&config)?);
    let streams = Arc::new(StreamHub::new());
    android.set_frame_sink(Arc::new(streams.as_ref().clone()));

    let target = match args.package {
        Some(package) => package,
        None => android.resolve_tiktok_package(&serial).await?,
    };
    println!("serial {serial}\ntarget {target}\n");

    let events = EventBus::new(128);
    let registry = DeviceRegistry::new(events.clone());
    let control = Arc::new(DeviceControlPlane::new(
        android.clone(),
        Arc::new(DeviceWorkCoordinator::new()),
        Arc::new(riviu_core::StreamBudgetManager::new(2)?),
    ));

    // Only this serial reaches the registry: `resolve_targets` fails closed on absent
    // devices, and a run aimed at one phone must not depend on the rest of the fleet.
    let devices = control.list_devices().await?;
    let device = devices
        .iter()
        .find(|device| device.udid == serial)
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("{serial} is not connected"))?;
    registry.upsert_many(vec![device]);

    let scratch = ScratchDirectory::create(
        std::env::temp_dir().join(format!("riviu-live-flow-android-{}", Uuid::new_v4())),
    )?;
    let database = Arc::new(Database::open(scratch.path().join("riviu.db"))?);
    let runtime = FlowRuntime::new(FlowRuntimeDeps {
        database: database.clone(),
        events,
        registry,
        control: control.clone(),
        frames: Arc::new(streams.as_ref().clone()),
        artifacts: FlowArtifactStore::new(scratch.path().join("flows"))?,
    });
    let run_result: anyhow::Result<FlowAggregateState> = async {
        runtime.recover_startup().await?;

        // Built with the typed model rather than JSON: node and edge ids are UUIDs, the
        // document carries an `entryNodeId`, and getting any of that wrong here would look
        // like a Flow failure rather than a harness one.
        let mut launch = FlowNode::new(ActionKind::LaunchApp, json!({ "bundleId": target }));
        // Required, and the point: the run is only a success if the phone comes back and says
        // TikTok is in front. On Android that read is `mCurrentFocus`.
        launch.postcondition = Some(EvidenceSpec::ActiveAppEquals {
            bundle_id: target.clone(),
        });
        let mut document = FlowDocumentV2::empty("android-live-preflight");
        let start = document.entry_node_id;
        let end = document
            .nodes
            .iter()
            .find(|node| node.kind == ActionKind::End)
            .map(|node| node.id)
            .expect("empty document has an End");
        let launch_id = launch.id;
        document.nodes.push(launch);
        document.edges = vec![
            FlowEdge::flow(start, launch_id),
            FlowEdge::flow(launch_id, end),
        ];
        document.revision = 1;

        if args.mode == LiveMode::Tap {
            let snapshot = android.inspect_device_for_target(&serial, &target).await?;
            let geometry = snapshot.geometry.as_ref().expect("qualified geometry");
            let profile_id = riviu_core::qualified_geometry_profile_id(&snapshot)
                .map_err(|reason| anyhow::anyhow!("geometry profile: {reason}"))?;
            println!(
                "tap target  {}x{} {:?}",
                geometry.pixel_width, geometry.pixel_height, geometry.orientation
            );
            // Middle of the screen: on a TikTok feed that is the video itself, so the tap
            // pauses or resumes playback and nothing is posted, followed or liked.
            let mut tap = FlowNode::new(
                ActionKind::Tap,
                json!({
                    "point": {
                        "x": f64::from(geometry.pixel_width) / 2.0,
                        "y": f64::from(geometry.pixel_height) / 2.0,
                        "imageWidth": geometry.pixel_width,
                        "imageHeight": geometry.pixel_height,
                        "orientation": match geometry.orientation {
                            ScreenOrientation::Portrait => "portrait",
                            ScreenOrientation::PortraitUpsideDown => "portraitUpsideDown",
                            ScreenOrientation::LandscapeLeft => "landscapeLeft",
                            ScreenOrientation::LandscapeRight => "landscapeRight",
                        },
                        "profileId": profile_id,
                    }
                }),
            );
            // The only evidence Tap allows, and it is stated in *frame* pixels -- which is the
            // second reason the producer's scale matters. A rectangle authored against the
            // device's geometry is out of bounds on a half-scale frame.
            tap.postcondition = Some(EvidenceSpec::FrameRegionChanged {
                x: geometry.pixel_width / 4,
                y: geometry.pixel_height / 4,
                width: geometry.pixel_width / 2,
                height: geometry.pixel_height / 2,
                minimum_distance: 1,
            });
            let tap_id = tap.id;
            document.nodes.push(tap);
            document.edges = vec![
                FlowEdge::flow(start, launch_id),
                FlowEdge::flow(launch_id, tap_id),
                FlowEdge::flow(tap_id, end),
            ];
        } else if args.mode == LiveMode::AutoSwipe {
            let snapshot = android.inspect_device_for_target(&serial, &target).await?;
            let geometry = snapshot.geometry.as_ref().expect("qualified geometry");
            println!(
                "auto-swipe frame  {}x{} {:?}",
                geometry.pixel_width, geometry.pixel_height, geometry.orientation
            );
            let mut auto_swipe = FlowNode::new(
                ActionKind::AutoSwipe,
                json!({
                    "preset": "custom",
                    "count": 3,
                    "from": { "x": 0.5, "y": 0.78 },
                    "to": { "x": 0.5, "y": 0.28 },
                    "gestureDurationMs": 350,
                    "pauseMinMs": 1_200,
                    "pauseMaxMs": 2_500,
                    "jitterPercent": 2,
                }),
            );
            auto_swipe.postcondition = Some(EvidenceSpec::FrameDigestChanged {
                minimum_distance: 1,
            });
            let auto_swipe_id = auto_swipe.id;
            document.nodes.push(auto_swipe);
            document.edges = vec![
                FlowEdge::flow(start, launch_id),
                FlowEdge::flow(launch_id, auto_swipe_id),
                FlowEdge::flow(auto_swipe_id, end),
            ];
        }

        let compiled = compile_flow(&document, &release_one_catalog())
            .map_err(|errors| anyhow::anyhow!("flow did not compile: {errors:?}"))?;
        println!(
            "compiled  ui_session={} exclusive={} capabilities={:?}\n",
            compiled.plan.context_plan.requires_ui_session,
            compiled.plan.context_plan.requires_exclusive,
            compiled.plan.required_capabilities
        );

        // Saved first, and that is a rule rather than a formality: `insert_flow_run` refuses a
        // revision that is not already in `flow_revisions` with a matching plan hash, so a run
        // can only ever execute a plan the database has committed.
        let revision =
            database.save_flow_revision(None, &document, &compiled.plan, &compiled.sha256)?;

        let run = runtime
            .enqueue(
                revision,
                FlowTargetSelection::Selected {
                    udids: vec![serial.clone()],
                },
            )
            .await?;
        println!("run {}", run.id);

        let deadline = tokio::time::Instant::now() + WAIT_DEADLINE;
        let detail = loop {
            if let Some(detail) = database.get_flow_run(run.id)? {
                if detail.run.state.is_terminal() {
                    break detail;
                }
            }
            if tokio::time::Instant::now() >= deadline {
                anyhow::bail!("the run never reached a terminal state");
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        };

        println!("\nstate {:?}", detail.run.state);
        if let Some(error) = &detail.run.error {
            println!("error {error:?}");
        }
        for device_run in &detail.device_runs {
            println!(
                "\ndevice {}  {:?}  {}",
                device_run.udid,
                device_run.state,
                device_run
                    .error
                    .as_ref()
                    .map(|e| format!("{e:?}"))
                    .unwrap_or_default()
            );
            for attempt in &detail.attempts {
                if attempt.device_run_id == device_run.id {
                    println!(
                        "  node {:<10} {:?}  {}",
                        attempt.node_id,
                        attempt.state,
                        attempt
                            .error
                            .as_ref()
                            .map(|e| format!("{e:?}"))
                            .unwrap_or_default()
                    );
                }
            }
        }
        Ok(detail.run.state)
    }
    .await;

    let runtime_shutdown = runtime.shutdown().await;
    let control_shutdown = control.shutdown_cleanup().await;
    drop(runtime);
    drop(database);
    let scratch_cleanup = scratch.cleanup();

    let state = run_result?;
    runtime_shutdown?;
    control_shutdown?;
    scratch_cleanup?;
    require_succeeded(state)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arguments_preserve_launch_only_and_tap_modes() {
        assert_eq!(
            parse_args(["serial-1"].map(String::from)).unwrap(),
            LiveArgs {
                serial: "serial-1".to_string(),
                package: None,
                mode: LiveMode::LaunchOnly,
            }
        );
        assert_eq!(
            parse_args(["serial-1", "--tap"].map(String::from))
                .unwrap()
                .mode,
            LiveMode::Tap
        );
    }

    #[test]
    fn auto_swipe_requires_an_explicit_fixture_package() {
        let parsed = parse_args(
            [
                "serial-1",
                "--package",
                "com.riviu.fixture.autoswipe",
                "--auto-swipe",
            ]
            .map(String::from),
        )
        .unwrap();
        assert_eq!(
            parsed.package.as_deref(),
            Some("com.riviu.fixture.autoswipe")
        );
        assert_eq!(parsed.mode, LiveMode::AutoSwipe);
        assert!(parse_args(["serial-1", "--auto-swipe"].map(String::from)).is_err());
        assert!(parse_args(["serial-1", "--tap", "--auto-swipe"].map(String::from)).is_err());
    }

    #[test]
    fn only_a_succeeded_terminal_run_is_accepted() {
        assert!(require_succeeded(FlowAggregateState::Succeeded).is_ok());
        for state in [
            FlowAggregateState::Partial,
            FlowAggregateState::Failed,
            FlowAggregateState::Cancelled,
        ] {
            assert!(require_succeeded(state).is_err(), "{state:?}");
        }
    }

    #[test]
    fn scratch_cleanup_removes_the_complete_tree() {
        let path = std::env::temp_dir().join(format!("riviu-live-flow-cleanup-{}", Uuid::new_v4()));
        let scratch = ScratchDirectory::create(path.clone()).expect("create scratch directory");
        std::fs::create_dir_all(scratch.path().join("flows")).expect("create nested directory");
        std::fs::write(scratch.path().join("flows").join("artifact"), b"proof")
            .expect("write scratch artifact");

        scratch.cleanup().expect("cleanup scratch directory");

        assert!(!path.exists());
    }
}
