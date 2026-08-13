//! Gate G2: run the real hierarchy nurture loop against a real phone.
//!
//! G1 (`examples/probe.rs`) proves the primitives. This proves the *loop* — the
//! one in `riviu_core::nurture::run_hierarchy_session`, reached through the same
//! public entry point the desktop app uses, with no control plane, database, or
//! stream in the way. Nothing here reimplements the session: if the pacing or the
//! like confirmation changes in core, this run changes with it.
//!
//! **What this gate deliberately does not cover**, and where to find it: the
//! control-plane handoff. Bypassing it is this gate's value — the loop is tested in
//! isolation — but it is also why this passed for weeks while the desktop app could
//! not start an Android session at all. `examples/control_plane.rs` (G3) is the
//! gate for that. Do not "fix" this one by routing it through the plane.
//!
//! ```text
//! RIVIU_ADB_PATH=…/adb.exe RIVIU_TIKTOK_PACKAGE=com.ss.android.ugc.trill \
//!   cargo run -p riviu-android-driver --example nurture -- <serial> --videos 3
//! ```
//!
//! **This touches a real account.** It watches, swipes, and — unless `--no-like`
//! is passed — likes posts, because that is the behaviour being verified. Follows
//! are off unless `--follow` is passed: a follow is visible to another person and
//! is not needed to prove the loop works.
//!
//! Commenting is off unless `--comment <text>` is given, and then the *given text*
//! is posted — no AI is involved here on purpose, so a drawer failure cannot be
//! confused with a generation failure. The desktop app supplies the real grounded
//! generator instead.

use std::sync::atomic::AtomicBool;
use std::sync::LazyLock;
use std::time::{Duration, Instant};

use riviu_android_driver::{AndroidDriver, AndroidDriverConfig};
use riviu_core::driver::DeviceDriver;
use riviu_core::nurture::{
    run_hierarchy_session, CommentTextSource, HierarchySession, PreparedComment,
};
use riviu_core::types::{NurtureSessionStatus, NurtureSettings};

static TIKTOK: LazyLock<String> = LazyLock::new(|| {
    std::env::var("RIVIU_TIKTOK_PACKAGE")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "com.ss.android.ugc.trill".to_string())
});

/// Read a `--flag value` pair.
fn numeric_arg(args: &[String], flag: &str) -> Option<u32> {
    let index = args.iter().position(|arg| arg == flag)?;
    args.get(index + 1)?.parse().ok()
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let videos = numeric_arg(&args, "--videos").unwrap_or(3);
    let like_prob = if args.iter().any(|arg| arg == "--no-like") {
        0
    } else {
        numeric_arg(&args, "--like").unwrap_or(60)
    };
    let follow_prob = if args.iter().any(|arg| arg == "--follow") {
        numeric_arg(&args, "--follow-prob").unwrap_or(20)
    } else {
        0
    };
    // `--comment <text>` posts that exact text. Explicit rather than a probability,
    // because a probe that sometimes comments is a probe that cannot be trusted to
    // have exercised the drawer.
    let comment_text = args
        .iter()
        .position(|arg| arg == "--comment")
        .and_then(|index| args.get(index + 1))
        .cloned();
    let serial = args
        .iter()
        .find(|arg| {
            !arg.starts_with("--")
                && arg.parse::<u32>().is_err()
                && Some(arg.as_str()) != comment_text.as_deref()
        })
        .cloned();

    let driver = AndroidDriver::new(&AndroidDriverConfig::default())?;
    let devices = driver.list_devices().await?;
    let serial = match serial {
        Some(serial) => serial,
        None => devices
            .first()
            .map(|device| device.udid.clone())
            .ok_or_else(|| anyhow::anyhow!("no Android device attached"))?,
    };
    println!("device {serial}, target {}", TIKTOK.as_str());
    println!("videos={videos} like_prob={like_prob} follow_prob={follow_prob}");

    let session = driver.open_session(&serial).await?;
    let screen = riviu_core::driver::UiSession::window_size(&session).await?;
    println!("screen {screen:?}");

    // A source that always says the same thing. `comment_prob` has to be non-zero
    // as well, or the action roll never selects a comment.
    struct FixedComment(String);
    #[async_trait::async_trait]
    impl CommentTextSource for FixedComment {
        async fn comment_for_post(
            &self,
            _settings: &riviu_core::types::NurtureSettings,
        ) -> Option<PreparedComment> {
            Some(PreparedComment {
                text: self.0.clone(),
                usd: 0.0,
                attempt_id: None,
            })
        }
    }
    let comment_source = comment_text.clone().map(FixedComment);
    let comment_prob = if comment_text.is_some() { 100 } else { 0 };
    if let Some(text) = &comment_text {
        println!("comment text: {text:?} (will be POSTED)");
    }

    let settings = NurtureSettings {
        bundle_id: TIKTOK.clone(),
        num_videos: videos,
        num_rounds: 1,
        like_prob,
        comment_prob,
        follow_prob,
        // Short dwells: this is a functional check, not a session that has to look
        // like a person. A real run uses the operator's configured window.
        watch_min: 2.0,
        watch_max: 4.0,
        // Off, so the run is not skipped for being inside the operator's configured
        // quiet hours.
        night_start: 0,
        night_end: 0,
        ..NurtureSettings::default()
    };

    let stop = AtomicBool::new(false);
    let mut status = NurtureSessionStatus {
        udid: serial.clone(),
        running: true,
        videos_done: 0,
        swipe_attempts: 0,
        like_attempts: 0,
        comment_attempts: 0,
        follow_attempts: 0,
        likes: 0,
        comments: 0,
        follows: 0,
        last_message: "bắt đầu".into(),
        session_usd: 0.0,
    };
    let started = Instant::now();
    let report = move |status: &mut NurtureSessionStatus, message: String| {
        println!("  [{:>6.1}s] {message}", started.elapsed().as_secs_f64());
        status.last_message = message;
    };

    println!("\n== run_hierarchy_session ==");
    // A timed run ignores the video count entirely — `total_videos` becomes
    // unbounded and the clock decides, which is the pixel engine's rule and not
    // something to diverge from here. So pass a duration only when one was asked
    // for, or `--videos 4` silently means "run for the whole timeout".
    let max_duration = numeric_arg(&args, "--seconds").map(|s| Duration::from_secs(s as u64));
    println!(
        "bound: {}",
        match max_duration {
            Some(limit) => format!("{}s (video count ignored)", limit.as_secs()),
            None => format!("{videos} video"),
        }
    );
    let outcome = run_hierarchy_session(
        &session,
        screen,
        &settings,
        // The package this gate was told to drive. The loop no longer derives it from
        // `settings.bundle_id`, which holds an iOS bundle by default.
        TIKTOK.as_str(),
        started,
        max_duration,
        &stop,
        &mut status,
        &report,
        comment_source
            .as_ref()
            .map(|source| source as &dyn CommentTextSource),
        // No live source: this gate has no database behind it, so the session keeps the
        // numbers it was started with — which is exactly what a gate should do.
        None,
    )
    .await;

    println!("\n== result ==");
    match &outcome {
        HierarchySession::Ran(verdict) => println!("  outcome = {}", verdict.as_str()),
        HierarchySession::NotSupported => {
            println!("  this backend cannot report element bounds — nothing ran")
        }
        HierarchySession::Refused => println!("  refused: {}", status.last_message),
    }
    println!(
        "  {}/{} video, tim {}/{}, follow {}/{}, {:.0}s",
        status.videos_done,
        status.swipe_attempts,
        status.likes,
        status.like_attempts,
        status.follows,
        status.follow_attempts,
        started.elapsed().as_secs_f64()
    );

    // A run that swiped without ever proving a card change is a failure even if
    // nothing errored, and saying so here is the point of a gate.
    if matches!(outcome, HierarchySession::Ran(_)) && status.videos_done == 0 {
        anyhow::bail!("the loop never proved a single card change");
    }
    Ok(())
}
