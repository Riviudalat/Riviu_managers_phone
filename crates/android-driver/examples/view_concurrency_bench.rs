//! What a fleet-wide recovery ceiling should be, measured instead of guessed.
//!
//! AGENTS.md 9.67 records that restarting a view producer on "packets arrive but nothing
//! paints" is a positive feedback loop — 33 producer starts at two phones, **291** at
//! twenty — and concludes that the frontend detector may only restart again once there is a
//! **fleet-wide concurrency ceiling** on recovery. It does not say what that ceiling is,
//! and picking one from nowhere is precisely how the constants this fleet keeps outgrowing
//! (`BROADCAST_CAP` 8, then 128) were chosen against a bench of two.
//!
//! This measures it. A view start is not cheap and its cost is not local: `spawn_view`
//! wakes the display, pushes/verifies the server JAR, kills leftovers, prunes forwards,
//! adds an `adb forward` and spawns an `app_process`, and **every one of those goes through
//! the single adb server**. So the question is not "how long does one restart take" but
//! "how many at once before they start costing each other time".
//!
//! ```text
//! # stop the desktop app first (AGENTS.md 9.60: it SIGTERMs any scrcpy server it finds)
//! cargo run -p riviu-android-driver --example view_concurrency_bench -- 1 2 4 8 20
//! ```
//!
//! Each argument is a concurrency level. For each level the bench starts a view on every
//! connected phone with at most that many starts in flight, records **time to the first
//! keyframe** per phone, then stops them all and moves on. What to read: the level at which
//! the per-phone median stops tracking the level-1 median is the point where starts are
//! queueing on adb rather than running.
//!
//! **What it found, 16/08/2026, twenty Galaxy S8/S8+ (AGENTS.md 9.72): no such level.** p50
//! was 11.4-11.5 s at every concurrency from 1 to 20 and wall time fell exactly linearly,
//! 230.0 s to 14.9 s. One adb server takes twenty concurrent scrcpy spawns without slowing
//! any of them. Two things follow, and both contradicted what was believed when this was
//! written: a ceiling cannot be justified as protecting adb throughput, and a **clean** start
//! reaches its first keyframe in ~11.5 s rather than the ~44 s of 9.64 — that figure is the
//! restart path inside a loaded app and does not describe this one.
//!
//! It is kept rather than deleted because the answer is a property of a particular box, and
//! the next farm may not answer the same way.
//!
//! It starts and stops real producers on real phones and touches nothing else.

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use riviu_android_driver::{AndroidDriver, ViewKind, ViewPacket, ViewPreset, ViewSink};
use riviu_core::driver::DeviceDriver;

#[path = "common/mod.rs"]
mod common;

/// Longest a single start is waited for before it is called a failure.
///
/// 9.64 measured one restart end to end at ~44 s (17:51:54 -> 17:52:45) with nothing else
/// competing. This has to leave room for a level where they *are* competing, or the bench
/// would report a timeout as an answer — an overrun has to be a real finding, not the
/// deadline showing through. Measured worst case turned out to be 14.9 s.
const START_DEADLINE: Duration = Duration::from_secs(180);

/// A sink that answers one question per device: when did the first keyframe land.
///
/// Deliberately not `ViewHub`. The hub coalesces, replays and fans out to a WebSocket, and
/// none of that is on the path being measured — mixing it in would put the desktop's own
/// scheduling into a number that is supposed to be about adb.
struct FirstKeyframeSink {
    generations: Mutex<HashMap<String, u64>>,
    first_key_at: Mutex<HashMap<String, Instant>>,
    packets: AtomicUsize,
}

impl FirstKeyframeSink {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            generations: Mutex::new(HashMap::new()),
            first_key_at: Mutex::new(HashMap::new()),
            packets: AtomicUsize::new(0),
        })
    }

    fn first_key_at(&self, udid: &str) -> Option<Instant> {
        self.first_key_at.lock().get(udid).copied()
    }

    fn reset(&self) {
        self.first_key_at.lock().clear();
        self.packets.store(0, Ordering::Release);
    }
}

impl ViewSink for FirstKeyframeSink {
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
        self.first_key_at.lock().remove(udid);
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
        self.packets.fetch_add(1, Ordering::Relaxed);
        if packet.kind == ViewKind::H264 && packet.key {
            self.first_key_at
                .lock()
                .entry(packet.udid.clone())
                .or_insert_with(Instant::now);
        }
        true
    }
}

/// One phone's result at one concurrency level.
struct Sample {
    udid: String,
    /// `None` when no keyframe arrived inside [`START_DEADLINE`].
    to_first_key: Option<Duration>,
}

fn percentile(sorted: &[Duration], fraction: f64) -> Duration {
    if sorted.is_empty() {
        return Duration::ZERO;
    }
    let index = ((sorted.len() - 1) as f64 * fraction).round() as usize;
    sorted[index]
}

async fn run_level(
    driver: &Arc<AndroidDriver>,
    sink: &Arc<FirstKeyframeSink>,
    serials: &[String],
    concurrency: usize,
) -> Vec<Sample> {
    sink.reset();
    let gate = Arc::new(tokio::sync::Semaphore::new(concurrency));
    let mut tasks = Vec::with_capacity(serials.len());
    for serial in serials {
        let driver = Arc::clone(driver);
        let sink = Arc::clone(sink);
        let gate = Arc::clone(&gate);
        let serial = serial.clone();
        tasks.push(tokio::spawn(async move {
            // The permit is held across the whole start AND the wait for the first
            // keyframe, because that is what a recovery permit would have to do: a
            // producer that has spawned but not yet produced is still consuming the
            // resource this ceiling exists to protect.
            let _permit = gate.acquire().await.expect("semaphore is never closed");
            let started = Instant::now();
            if let Err(error) = driver.start_view_stream(&serial, ViewPreset::Tile).await {
                eprintln!("  {serial}: start failed: {error:#}");
                return Sample {
                    udid: serial,
                    to_first_key: None,
                };
            }
            loop {
                if let Some(at) = sink.first_key_at(&serial) {
                    return Sample {
                        udid: serial,
                        to_first_key: Some(at.duration_since(started)),
                    };
                }
                if started.elapsed() >= START_DEADLINE {
                    return Sample {
                        udid: serial,
                        to_first_key: None,
                    };
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }));
    }
    let mut samples = Vec::with_capacity(tasks.len());
    for task in tasks {
        match task.await {
            Ok(sample) => samples.push(sample),
            Err(error) => eprintln!("  a start task panicked: {error}"),
        }
    }
    samples
}

#[tokio::main(flavor = "multi_thread")]
async fn main() -> anyhow::Result<()> {
    let levels: Vec<usize> = std::env::args()
        .skip(1)
        .filter_map(|raw| raw.trim().parse::<usize>().ok())
        .filter(|level| *level > 0)
        .collect();
    let levels = if levels.is_empty() {
        vec![1, 2, 4, 8, 20]
    } else {
        levels
    };

    // Point at the repo's own sidecars unless the operator has said otherwise. Without
    // this every start fails with "no scrcpy server configured" and the bench reports a
    // configuration mistake as if it were a measurement.
    let sidecars = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../sidecars");
    let mut config = common::repo_config();
    if std::env::var_os("RIVIU_SCRCPY_SERVER").is_none() {
        let server = sidecars.join("android/noarch/scrcpy-server");
        anyhow::ensure!(
            server.is_file(),
            "no scrcpy server at {}; set RIVIU_SCRCPY_SERVER",
            server.display()
        );
        config.scrcpy_server = Some(server);
    }
    let driver = riviu_android_driver::detect_driver(&config)
        .await
        .map_err(|reason| anyhow::anyhow!("no usable adb on this host: {reason}"))?;
    let sink = FirstKeyframeSink::new();
    driver.set_view_sink(sink.clone() as Arc<dyn ViewSink>);

    let serials: Vec<String> = driver
        .list_devices()
        .await?
        .into_iter()
        .map(|device| device.udid)
        .filter(|serial| !serial.is_empty())
        .collect();
    anyhow::ensure!(!serials.is_empty(), "no phones are connected");
    println!("{} phones connected", serials.len());
    println!(
        "deadline per start {} s; levels {levels:?}\n",
        START_DEADLINE.as_secs()
    );

    for level in levels {
        let level = level.min(serials.len());
        println!("--- concurrency {level} ---");
        let wall = Instant::now();
        let samples = run_level(&driver, &sink, &serials, level).await;
        let wall = wall.elapsed();

        let mut ok: Vec<Duration> = samples.iter().filter_map(|s| s.to_first_key).collect();
        ok.sort();
        let failed: Vec<&str> = samples
            .iter()
            .filter(|s| s.to_first_key.is_none())
            .map(|s| s.udid.as_str())
            .collect();
        if ok.is_empty() {
            println!("  every start failed or timed out\n");
        } else {
            println!(
                "  first keyframe: min {:.1}s  p50 {:.1}s  p90 {:.1}s  max {:.1}s   \
                 ({}/{} phones, wall {:.1}s)",
                ok[0].as_secs_f64(),
                percentile(&ok, 0.5).as_secs_f64(),
                percentile(&ok, 0.9).as_secs_f64(),
                ok[ok.len() - 1].as_secs_f64(),
                ok.len(),
                samples.len(),
                wall.as_secs_f64()
            );
        }
        if !failed.is_empty() {
            println!("  no keyframe: {}", failed.join(", "));
        }
        println!();

        driver.stop_all_views().await;
        // Let the phones and the adb server settle, or the next level measures the
        // teardown of the previous one.
        tokio::time::sleep(Duration::from_secs(5)).await;
    }

    driver.stop_all_views().await;
    Ok(())
}
