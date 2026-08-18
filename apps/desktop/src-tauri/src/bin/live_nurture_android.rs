//! Run the nurture engine on several real Android phones at once, and report what each did.
//!
//! ```text
//! RIVIU_ADB_PATH=… cargo run -p riviu-managers-phone --bin live_nurture_android -- \
//!   --devices 4 --minutes 2 --videos 3
//! ```
//!
//! **Do not run this while the desktop app is open.** Two processes competing for the same
//! phones is the contention this project spent a week removing.
//!
//! Comments are off (`comment_prob = 0`) and cannot be switched on here: writing a comment
//! needs an AI key and puts text on a real account, which is not a thing a measurement
//! harness should do. That promise is kept by *persisting* the settings rather than by
//! passing them — see the note at the call, and the follow that happened before it did.
//! What it does exercise is everything underneath — an exclusive lease, a
//! stream-budget slot, a UI session, TikTok in the foreground, watch and swipe and like —
//! per device, concurrently. That is the part that decides whether the feature works for a
//! whole fleet or only for the first couple of phones.
//!
//! It exists because the answer turned out to be "the first couple": every session holds a
//! foreground slot for its whole run, and the desktop's budget was two.

use std::io::Write;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::{Duration, Instant};

use app_lib::interaction_ocr::DesktopFrameTextSource;
use riviu_core::db::Database;
use riviu_core::driver::DeviceDriver;
use riviu_core::{
    DeviceControlPlane, DeviceWorkCoordinator, NurtureEngine, NurtureSettings, StreamBudgetManager,
};
use riviu_ios_driver::StreamHub;
use uuid::Uuid;

/// Print and flush. Output is piped to a file here, where Rust block-buffers stdout — a run
/// that is killed part way would otherwise report nothing at all about what it had learned.
fn say(line: &str) {
    println!("{line}");
    let _ = std::io::stdout().flush();
}

struct Args {
    devices: usize,
    /// Restrict the run to named serials. Without it the harness takes the first N the
    /// driver reports, which is what "the whole fleet" means.
    only: Vec<String>,
    minutes: u64,
    videos: u32,
    /// Percent chance of liking each watched post.
    ///
    /// Configurable because the like path is the one that needs a *sample* to say anything:
    /// at the default rate a two-video run produces one or two attempts across the whole
    /// fleet, which cannot distinguish "confirmation works" from "we got lucky". Raising it
    /// is the only way to measure the confirmation, and a like is an action this feature
    /// exists to perform — unlike a comment, which is why that one stays at zero.
    like_prob: u32,
    /// Percent chance of following the author of a watched post.
    ///
    /// Off by default and separate from `--like-prob` because the two are not the same kind
    /// of action: a like can be taken back and leaves nothing behind, a follow is a lasting
    /// relationship on a real account. Configurable at all only because the path cannot be
    /// verified without performing one — and then it is worth doing on one phone rather than
    /// on twenty.
    follow_prob: u32,
}

fn parse_args() -> Args {
    let mut args = Args {
        devices: 2,
        only: Vec::new(),
        minutes: 2,
        videos: 3,
        like_prob: 30,
        follow_prob: 0,
    };
    let raw: Vec<String> = std::env::args().skip(1).collect();
    let mut index = 0;
    while index + 1 < raw.len() {
        let value = raw[index + 1].parse::<u64>().unwrap_or(0);
        match raw[index].as_str() {
            "--devices" => args.devices = value as usize,
            "--minutes" => args.minutes = value,
            "--videos" => args.videos = value as u32,
            "--like-prob" => args.like_prob = value.min(100) as u32,
            "--follow-prob" => args.follow_prob = value.min(100) as u32,
            "--only" => args.only = raw[index + 1].split(',').map(str::to_string).collect(),
            _ => {}
        }
        index += 2;
    }
    args
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = parse_args();

    let android = Arc::new(riviu_android_driver::AndroidDriver::new(
        &riviu_android_driver::AndroidDriverConfig::default(),
    )?);
    let streams = Arc::new(StreamHub::new());
    android.set_frame_sink(Arc::new(streams.as_ref().clone()));

    let devices = android.list_devices().await?;
    let targets: Vec<String> = devices
        .iter()
        .filter(|device| device.status != riviu_core::DeviceStatus::Disconnected)
        .filter(|device| args.only.is_empty() || args.only.contains(&device.udid))
        .take(args.devices)
        .map(|device| device.udid.clone())
        .collect();
    anyhow::ensure!(!targets.is_empty(), "no usable Android device is connected");

    // The number under test. Every nurture session holds one for its whole run, so this is
    // the ceiling on how many phones can nurture at once — whatever the fleet size is.
    let budget = StreamBudgetManager::new(
        std::env::var("RIVIU_STREAM_CAPACITY")
            .ok()
            .and_then(|raw| raw.trim().parse::<usize>().ok())
            .unwrap_or(targets.len()),
    );
    let budget = match budget {
        Ok(budget) => budget,
        Err(error) => {
            println!("stream budget refused: {error}");
            println!("  -> that refusal is the finding: the fleet cannot exceed it.");
            return Ok(());
        }
    };

    let scratch = std::env::temp_dir().join(format!("riviu-live-nurture-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&scratch)?;
    let database = Arc::new(Database::open(scratch.join("riviu.db"))?);
    let control = Arc::new(DeviceControlPlane::new(
        android.clone(),
        Arc::new(DeviceWorkCoordinator::new()),
        Arc::new(budget),
    ));
    let engine = NurtureEngine::new(
        database.clone(),
        control.clone(),
        Arc::new(streams.as_ref().clone()),
        scratch.join("artifacts"),
    )
    .with_frame_text_source(Arc::new(DesktopFrameTextSource));

    let settings = NurtureSettings {
        num_videos: args.videos,
        num_rounds: 1,
        like_prob: args.like_prob,
        // Not configurable, and deliberately: a comment is text on a real account.
        comment_prob: 0,
        follow_prob: args.follow_prob,
        frenzy_prob: 0,
        watch_min: 2.0,
        watch_max: 4.0,
        stagger_delay_min: 1,
        stagger_delay_max: 3,
        ..Default::default()
    };

    // **Write them down before running, or they are not the settings that run.**
    //
    // `run_session` re-reads the stored settings row once per post — that is the desktop's
    // live-tuning mechanism, where "Lưu" in the panel *is* how a running session is
    // retuned — and what it reads overwrites what was passed in. A fresh database answers
    // with `NurtureSettings::default()`, whose `follow_prob` is 3.
    //
    // Measured, not reasoned: on 18/08/2026 ce0717171c2a64d50d followed an author during a
    // run whose settings said `follow_prob: 0`. A follow is a real relationship on a real
    // account, and the same channel governs `comment_prob`, which this harness promises in
    // its own header never to switch on. Persisting first makes both promises true.
    database.save_nurture_settings(&settings)?;

    println!(
        "nurturing {} device(s) for up to {} minute(s), {} video(s) each\n",
        targets.len(),
        args.minutes,
        args.videos
    );

    let started = Instant::now();
    let mut running = Vec::new();
    for (index, udid) in targets.iter().cloned().enumerate() {
        let engine = engine.clone();
        let settings = settings.clone();
        let stop = Arc::new(AtomicBool::new(false));
        running.push(tokio::spawn(async move {
            // Staggered the way the desktop staggers, so twenty phones do not all reach for
            // a lease in the same millisecond.
            tokio::time::sleep(Duration::from_millis(index as u64 * 700)).await;
            let began = Instant::now();
            say(&format!("  [{udid}] session starting"));
            // Keep the trail. A session that ends "0/2 video" has already said *why* in
            // its intermediate status lines, and throwing them away leaves only the number.
            let trail = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
            let collector = Arc::clone(&trail);
            let outcome = engine
                .run_session(
                    &udid,
                    settings,
                    stop,
                    Some(Duration::from_secs(args.minutes * 60)),
                    move |status| {
                        let mut seen = collector.lock().expect("status trail");
                        if seen.last().map(String::as_str) != Some(status.last_message.as_str()) {
                            seen.push(status.last_message.clone());
                        }
                    },
                )
                .await;
            let trail = trail.lock().expect("status trail").clone();
            match &outcome {
                Ok(status) => {
                    say(&format!(
                        "  [{udid}] done in {:.0}s: videos={} likes={} — {}",
                        began.elapsed().as_secs_f64(),
                        status.videos_done,
                        status.likes,
                        status.last_message
                    ));
                    if status.videos_done == 0 {
                        for line in &trail {
                            say(&format!("      [{udid}] {line}"));
                        }
                    } else {
                        // The carousel is the thing under test, and a session that met a
                        // photo post and *finished* says so nowhere else: the trail is only
                        // dumped when a session ends at zero. That is exactly backwards for
                        // proving a traversal no longer eats the session, so the photo lines
                        // come out either way.
                        for line in trail.iter().filter(|line| line.contains("bài ảnh")) {
                            say(&format!("      [{udid}] {line}"));
                        }
                    }
                }
                Err(error) => say(&format!(
                    "  [{udid}] FAILED after {:.0}s: {error:#}",
                    began.elapsed().as_secs_f64()
                )),
            }
            (udid, began.elapsed(), outcome)
        }));
    }

    let mut worked = 0;
    let mut watched_any = 0;
    say(&format!(
        "{:<22} {:>7} {:>7} {:>7}  outcome",
        "device", "secs", "videos", "likes"
    ));
    for handle in running {
        let (udid, elapsed, outcome) = handle.await?;
        match outcome {
            Ok(status) => {
                if status.videos_done > 0 {
                    watched_any += 1;
                }
                worked += 1;
                println!(
                    "{udid:<22} {:>7.0} {:>7} {:>7}  {}",
                    elapsed.as_secs_f64(),
                    status.videos_done,
                    status.likes,
                    status.last_message
                );
            }
            Err(error) => println!(
                "{udid:<22} {:>7.0} {:>7} {:>7}  ERROR: {error:#}",
                elapsed.as_secs_f64(),
                0,
                0
            ),
        }
    }
    println!(
        "\n{worked}/{} session(s) returned, {watched_any} watched at least one video, in {:.0}s",
        targets.len(),
        started.elapsed().as_secs_f64()
    );
    let _ = std::fs::remove_dir_all(&scratch);
    Ok(())
}
