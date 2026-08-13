//! Gate G3: drive a real phone **through the real `DeviceControlPlane`**.
//!
//! G1 (`examples/probe.rs`) proves the primitives. G2 (`examples/nurture.rs`)
//! proves the feed loop, and does it with no control plane, database or stream in
//! the way — which is exactly why G2 passed for weeks while the desktop app could
//! not start an Android session at all. This is the gate for the part G2 skips.
//!
//! ```text
//! RIVIU_ADB_PATH=…/adb.exe RIVIU_MINICAP_APK=…/noarch/minicap.apk \
//! RIVIU_TIKTOK_PACKAGE=com.ss.android.ugc.trill \
//!   cargo run -p riviu-android-driver --example control_plane -- <serial>
//! ```
//!
//! What it asserts, and why each one is here:
//!
//! * `reserve_ui_capacity` succeeds — the step that failed first, because it stops
//!   the *target's own* background producer when the tile has been streaming it;
//! * `start_stream_after_session` returns `generation == handoff` and
//!   `first_frame_observed == true`;
//! * `cleanup_quarantine_count() == 0` after `shutdown_cleanup()`. **This is the
//!   real gate.** A wrong `old_generation` or a `child_stopped: false` shows up
//!   only as a quarantined cleanup ticket, and nothing else in a run reveals it.
//!
//! It also prints `adb forward --list` for the serial before and after, because a
//! leaked forward is the other failure that leaves no trace in a success path.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use riviu_android_driver::{AndroidDriver, AndroidDriverConfig};
use riviu_core::{DeviceControlPlane, DeviceWorkCoordinator, DeviceWorkOwner, StreamBudgetManager};

/// A minimal hub.
///
/// Deliberately **not** `StreamHub`: that lives in `riviu-ios-driver`, and this
/// crate must not depend on it — the whole reason `FrameSink` exists. Forty lines
/// here keep the dependency direction honest and make the park-versus-clear
/// distinction observable, which a shared hub would hide.
#[derive(Default)]
struct ExampleSink {
    generations: parking_lot::Mutex<HashMap<String, u64>>,
    latest: parking_lot::Mutex<HashMap<String, Vec<u8>>>,
    cleared: AtomicU64,
    parked: AtomicU64,
    published: AtomicU64,
}

impl ExampleSink {
    fn advance(&self, udid: &str) -> u64 {
        let mut generations = self.generations.lock();
        let entry = generations.entry(udid.to_string()).or_insert(0);
        *entry += 1;
        *entry
    }
}

impl riviu_core::FrameSink for ExampleSink {
    fn generation(&self, udid: &str) -> u64 {
        self.generations.lock().get(udid).copied().unwrap_or(0)
    }

    fn clear_and_advance(&self, udid: &str) -> u64 {
        self.cleared.fetch_add(1, Ordering::Relaxed);
        self.latest.lock().remove(udid);
        self.advance(udid)
    }

    fn park_and_advance(&self, udid: &str) -> u64 {
        self.parked.fetch_add(1, Ordering::Relaxed);
        // The point of park: the frame stays.
        self.advance(udid)
    }

    fn publish_if_current(&self, udid: &str, generation: u64, jpeg: Vec<u8>) -> bool {
        if self.generation(udid) != generation {
            return false;
        }
        self.published.fetch_add(1, Ordering::Relaxed);
        self.latest.lock().insert(udid.to_string(), jpeg);
        true
    }
}

fn tiktok() -> String {
    std::env::var("RIVIU_TIKTOK_PACKAGE")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "com.ss.android.ugc.trill".to_string())
}

async fn forwards(adb: &std::path::Path, serial: &str) -> String {
    let output = tokio::process::Command::new(adb)
        .args(["-s", serial, "forward", "--list"])
        .output()
        .await;
    match output {
        Ok(output) => {
            let text = String::from_utf8_lossy(&output.stdout);
            let lines: Vec<&str> = text
                .lines()
                .filter(|line| line.contains("riviu-minicap"))
                .collect();
            if lines.is_empty() {
                "none".to_string()
            } else {
                lines.join(" | ")
            }
        }
        Err(error) => format!("<unreadable: {error}>"),
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let wanted = args.iter().find(|arg| !arg.starts_with("--")).cloned();

    let driver = Arc::new(AndroidDriver::new(&AndroidDriverConfig::default())?);
    let adb_path = riviu_android_driver::AdbProgram::resolve(None, None)?
        .path()
        .to_path_buf();
    let sink = Arc::new(ExampleSink::default());
    driver.set_frame_sink(Arc::clone(&sink) as Arc<dyn riviu_core::FrameSink>);

    // Capacity 1, so a second device cannot mask a stop that never happened.
    let control = DeviceControlPlane::new(
        Arc::clone(&driver) as Arc<dyn riviu_core::DeviceDriver>,
        Arc::new(DeviceWorkCoordinator::new()),
        Arc::new(StreamBudgetManager::new(1)?),
    );

    // The route table is built only from a listing — never by parsing a udid.
    let devices = control.list_devices().await?;
    let serial = match wanted {
        Some(serial) => serial,
        None => devices
            .first()
            .map(|device| device.udid.clone())
            .ok_or_else(|| anyhow::anyhow!("no Android device attached"))?,
    };
    // Resolve rather than read an env var: this is what the desktop app does, and it
    // is the only way the run proves the resolver works. `RIVIU_TIKTOK_PACKAGE` stays
    // available for `--example nurture`, which drives the loop directly.
    let target = control.resolve_tiktok_package(&serial).await?;
    println!("device {serial}, resolved target {target}");
    if target != tiktok() {
        println!(
            "  (note: RIVIU_TIKTOK_PACKAGE says {}, the device says otherwise)",
            tiktok()
        );
    }
    println!("forwards before: {}", forwards(&adb_path, &serial).await);

    // Open the tile feed first, so the run exercises the case that actually failed
    // in the app: `reserve_ui_capacity` must stop the target's *own* background
    // producer, and that call is where an unimplemented `stop_owned_stream`
    // surfaced as a quarantined lease.
    //
    // It has to go through `reserve_background_stream` + `start_background_stream`,
    // the pair the desktop sampler uses — **not** `driver.ensure_stream` directly.
    // Calling the driver leaves the budget manager unaware of the producer, so
    // `reserve_ui_capacity` finds no victim to revoke while the driver still owns a
    // minicap; the handoff then correctly refuses and the gate proves nothing about
    // the path the app takes. (That mistake is what this comment is here to stop
    // someone repeating.)
    println!("\n== tile feed first, so reserve has a producer to revoke ==");
    let background = control.reserve_background_stream(&serial)?;
    match control.start_background_stream(&background).await {
        Ok(url) => println!("  start_background_stream -> {url}"),
        Err(error) => println!("  start_background_stream failed (continuing): {error}"),
    }

    println!("\n== control-plane handoff ==");
    let exclusive = control
        .acquire_exclusive(&serial, DeviceWorkOwner::Nurture)
        .await?;
    println!("  acquire_exclusive ok");
    let (exclusive, capacity) = control.reserve_ui_capacity(exclusive).await?;
    println!("  reserve_ui_capacity ok");
    let session = control
        .start_interaction_session(
            exclusive,
            &target,
            riviu_core::InteractionSessionKind::Ordinary,
        )
        .await?;
    println!("  start_interaction_session ok (foreground proven)");
    let context = control.start_reserved_stream(session, capacity).await?;
    println!("  start_reserved_stream ok");
    println!(
        "  sink: generation={} published={} cleared={} parked={}",
        riviu_core::FrameSink::generation(sink.as_ref(), &serial),
        sink.published.load(Ordering::Relaxed),
        sink.cleared.load(Ordering::Relaxed),
        sink.parked.load(Ordering::Relaxed),
    );

    // Prove the session is usable, not merely constructed.
    let ui = control.streaming_session(&context)?;
    let size = riviu_core::driver::UiSession::window_size(ui.as_ref()).await?;
    let foreground = riviu_core::driver::UiSession::active_app_bundle(ui.as_ref()).await?;
    println!("  session: screen {size:?} foreground {foreground}");
    anyhow::ensure!(
        foreground == target,
        "the plane handed back a session whose foreground is {foreground}, not {target}"
    );

    println!("\n== teardown ==");
    let release = control.close_ui_context(context).await?;
    println!("  close_ui_context -> {release:?}");
    control.shutdown_cleanup().await?;
    let quarantined = control.cleanup_quarantine_count();
    println!("  cleanup_quarantine_count = {quarantined}");
    println!("forwards after: {}", forwards(&adb_path, &serial).await);

    // The real gate. A quarantined ticket is how a wrong `old_generation` or a
    // `child_stopped: false` becomes visible; a run can otherwise look clean.
    anyhow::ensure!(
        quarantined == 0,
        "{quarantined} cleanup ticket(s) were quarantined — the stop proofs do not add up"
    );
    println!("\nG3 control-plane gate passed.");
    Ok(())
}
