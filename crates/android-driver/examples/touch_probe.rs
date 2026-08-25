//! Does a touch on the scrcpy control socket actually move the phone?
//!
//! Written because the first end-to-end try did not: a drag in the overlay left the feed
//! exactly where it was, and with the gesture crossing a browser, a Tauri command, a driver
//! and a socket there was no way to tell which half was wrong from the outside.
//!
//! So this exercises the Rust half alone. It starts one producer, injects DOWN, a path of
//! MOVEs and UP directly through [`AndroidDriver::inject_touch`], and photographs the
//! phone's own framebuffer over `adb exec-out screencap` before, **during** (button still
//! down) and after. Nothing from the desktop app is involved, so a pass here means the
//! message format, the socket and the coordinate handling are right and the fault is above
//! them; a failure here means the opposite, and either way it is one answer instead of four
//! suspects.
//!
//! ```text
//! # stop the desktop app first (AGENTS.md 9.60: it SIGTERMs any scrcpy server it finds)
//! cargo run -p riviu-android-driver --example touch_probe -- <serial>
//! ```
//!
//! It streams one phone and swipes on it. On a feed that is a scroll; on a home screen it
//! may be nothing at all, so run it against something that visibly moves.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use riviu_android_driver::{
    AndroidDriver, AndroidDriverConfig, TouchAction, ViewKind, ViewPacket, ViewPreset, ViewSink,
};
use riviu_core::driver::DeviceDriver;

const FIRST_KEYFRAME_DEADLINE: Duration = Duration::from_secs(90);

/// Enough of a sink to start a producer, and enough to report the frame size the touches
/// will be scaled into — which is the number most likely to be wrong.
struct SizeSink {
    generations: Mutex<HashMap<String, u64>>,
    size: Mutex<Option<(u32, u32)>>,
    keyed: Mutex<bool>,
}

impl SizeSink {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            generations: Mutex::new(HashMap::new()),
            size: Mutex::new(None),
            keyed: Mutex::new(false),
        })
    }
}

impl ViewSink for SizeSink {
    fn generation(&self, udid: &str) -> u64 {
        self.generations.lock().get(udid).copied().unwrap_or(0)
    }

    fn advance(&self, udid: &str) -> u64 {
        let mut generations = self.generations.lock();
        let next = generations
            .get(udid)
            .copied()
            .unwrap_or(0)
            .saturating_add(1);
        generations.insert(udid.to_string(), next);
        next
    }

    fn publish(&self, packet: ViewPacket) -> bool {
        let current = self
            .generations
            .lock()
            .get(&packet.udid)
            .copied()
            .unwrap_or(0);
        if packet.generation != current {
            return false;
        }
        *self.size.lock() = Some((packet.width, packet.height));
        if packet.kind == ViewKind::H264 && packet.key {
            *self.keyed.lock() = true;
        }
        true
    }
}

/// The phone's own framebuffer, hashed. Not decoded — the question is only whether it
/// changed, and two identical screens hash identically whatever is on them.
fn phone_frame(adb: &std::path::Path, serial: &str) -> anyhow::Result<(usize, u64)> {
    let out = std::process::Command::new(adb)
        .args(["-s", serial, "exec-out", "screencap", "-p"])
        .output()?;
    anyhow::ensure!(out.status.success(), "screencap failed");
    // FNV-1a, because a dependency for this would be silly.
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in &out.stdout {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    Ok((out.stdout.len(), hash))
}

#[tokio::main(flavor = "multi_thread")]
async fn main() -> anyhow::Result<()> {
    let sidecars = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../sidecars");
    let mut config = AndroidDriverConfig::default();
    if std::env::var_os("RIVIU_SCRCPY_SERVER").is_none() {
        let server = sidecars.join("android/noarch/scrcpy-server");
        anyhow::ensure!(
            server.is_file(),
            "no scrcpy server at {}; set RIVIU_SCRCPY_SERVER",
            server.display()
        );
        config.scrcpy_server = Some(server);
    }
    let driver: Arc<AndroidDriver> = riviu_android_driver::detect_driver(&config)
        .await
        .map_err(|reason| anyhow::anyhow!("no usable adb on this host: {reason}"))?;
    // The driver keeps its own resolved adb private; the screenshots here are a side check
    // rather than part of the path being probed, so they take the same env var the other
    // examples do and fall back to whatever is on PATH.
    let adb = std::path::PathBuf::from(
        std::env::var_os("RIVIU_ADB_PATH").unwrap_or_else(|| "adb".into()),
    );

    let serials: Vec<String> = driver
        .list_devices()
        .await?
        .into_iter()
        .map(|device| device.udid)
        .filter(|serial| !serial.is_empty())
        .collect();
    anyhow::ensure!(!serials.is_empty(), "no phones are connected");
    let serial = std::env::args()
        .nth(1)
        .unwrap_or_else(|| serials[0].clone());
    anyhow::ensure!(
        serials.contains(&serial),
        "{serial} is not connected; have {serials:?}"
    );
    println!("probing {serial}");

    let sink = SizeSink::new();
    driver.set_view_sink(sink.clone() as Arc<dyn ViewSink>);
    driver
        .start_view_stream(&serial, ViewPreset::Overlay)
        .await?;

    let waiting = Instant::now();
    while !*sink.keyed.lock() {
        anyhow::ensure!(
            waiting.elapsed() < FIRST_KEYFRAME_DEADLINE,
            "no keyframe within {FIRST_KEYFRAME_DEADLINE:?}"
        );
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    let (width, height) = sink.size.lock().expect("a frame was published");
    println!(
        "streaming at {width}x{height} after {:?}",
        waiting.elapsed()
    );

    // Coordinates in the picture's own space, which is what `inject_touch` takes. Up the
    // middle, from low to high: a scroll on anything that scrolls.
    let x = f64::from(width) / 2.0;
    let from_y = f64::from(height) * 0.80;
    let to_y = f64::from(height) * 0.25;
    let image_w = f64::from(width);
    let image_h = f64::from(height);

    let before = phone_frame(&adb, &serial)?;
    println!("phone before      {:>9} bytes  {:016x}", before.0, before.1);

    driver
        .inject_touch(&serial, TouchAction::Down, x, from_y, image_w, image_h)
        .await?;

    let steps = 20;
    let mut midway = None;
    for step in 1..=steps {
        let y = from_y + (to_y - from_y) * f64::from(step) / f64::from(steps);
        driver
            .inject_touch(&serial, TouchAction::Move, x, y, image_w, image_h)
            .await?;
        tokio::time::sleep(Duration::from_millis(90)).await;
        if step == steps / 2 {
            // Still down. Anything that has moved by here moved without a release, which is
            // the entire claim being tested.
            let shot = phone_frame(&adb, &serial)?;
            println!("phone mid-drag    {:>9} bytes  {:016x}", shot.0, shot.1);
            midway = Some(shot);
        }
    }

    let held = phone_frame(&adb, &serial)?;
    println!("phone still held  {:>9} bytes  {:016x}", held.0, held.1);

    driver
        .inject_touch(&serial, TouchAction::Up, x, to_y, image_w, image_h)
        .await?;
    tokio::time::sleep(Duration::from_millis(1200)).await;
    let after = phone_frame(&adb, &serial)?;
    println!("phone released    {:>9} bytes  {:016x}", after.0, after.1);

    let moved_while_held =
        midway.map(|shot| shot.1 != before.1).unwrap_or(false) || held.1 != before.1;
    println!(
        "\nmoved while the finger was still down: {}",
        if moved_while_held { "YES" } else { "NO" }
    );
    if !moved_while_held {
        println!(
            "A phone playing video changes on its own, so NO here is conclusive and YES is not.\n\
             Confirm a YES by eye before believing it."
        );
    }

    driver.stop_view_stream(&serial).await;
    Ok(())
}
