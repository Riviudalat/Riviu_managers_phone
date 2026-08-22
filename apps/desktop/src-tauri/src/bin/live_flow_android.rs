//! Run one real Flow, on one real Android phone, through the shipped runtime.
//!
//! ```text
//! RIVIU_ADB_PATH=… cargo run -p riviu-managers-phone --bin live_flow_android -- <serial>
//! ```
//!
//! This exists because B1's fix cannot be proved anywhere else. `AndroidDriver` had no
//! `inspect_device_for_target`, so the trait default answered `unsupported` and Flow
//! refused at preflight on every Android device — while the UI went on listing all of them
//! as valid targets. Unit tests can prove the snapshot is assembled correctly from fixed
//! facts; only a phone proves the facts can be read off it, and only the real `FlowRuntime`
//! proves preflight then admits the device and a node actually dispatches.
//!
//! What it runs is deliberately the smallest flow that exercises the whole gate:
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
//! the runtime frame disagreeing, never the plan.

use std::sync::Arc;
use std::time::Duration;

use riviu_core::db::Database;
use riviu_core::{
    release_one_catalog, ActionKind, DeviceControlPlane, DeviceDriver, DeviceRegistry,
    DeviceWorkCoordinator, EventBus, EvidenceSpec, FlowArtifactStore, FlowDocumentV2, FlowEdge,
    FlowNode, FlowRuntime, FlowRuntimeDeps, FlowTargetSelection, ScreenOrientation,
};
use riviu_ios_driver::StreamHub;
use riviu_script_engine::compile_flow;
use serde_json::json;
use uuid::Uuid;

const WAIT_DEADLINE: Duration = Duration::from_secs(120);

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let Some(serial) = std::env::args().nth(1) else {
        eprintln!("usage: live_flow_android <serial>");
        std::process::exit(2);
    };

    let config = riviu_android_driver::AndroidDriverConfig::default();
    let android = Arc::new(riviu_android_driver::AndroidDriver::new(&config)?);
    let streams = Arc::new(StreamHub::new());
    android.set_frame_sink(Arc::new(streams.as_ref().clone()));

    let target = android.resolve_tiktok_package(&serial).await?;
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

    let scratch = std::env::temp_dir().join(format!("riviu-live-flow-android-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&scratch)?;
    let database = Arc::new(Database::open(scratch.join("riviu.db"))?);
    let runtime = FlowRuntime::new(FlowRuntimeDeps {
        database: database.clone(),
        events,
        registry,
        control,
        frames: Arc::new(streams.as_ref().clone()),
        artifacts: FlowArtifactStore::new(scratch.join("flows"))?,
    });
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

    if std::env::args().any(|arg| arg == "--tap") {
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
    Ok(())
}
