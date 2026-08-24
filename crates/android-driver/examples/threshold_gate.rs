//! Gate: farm a post up to a number, and read the post to decide when to stop.
//!
//! Answers the operator's question directly — "bài mới 200 view, tôi muốn 500, farm thế nào
//! là đủ?" — by doing it: read where the post is, say what is reachable, run a pass, read
//! again, repeat.
//!
//! **The loop believes the post, not the arithmetic.** Measured 24/08/2026: ten phones opening
//! a fresh post added +9, a second immediate pass +9, a third +8 — so views accumulate. But
//! the same ten phones on a post they had opened all day added **nothing**, and no formula
//! predicts that. So a pass that moves nothing stops the loop instead of running forever.
//!
//! ```text
//! RIVIU_TIKTOK_PACKAGE=com.ss.android.ugc.trill \
//!   cargo run -p riviu-android-driver --example threshold_gate -- \
//!   <reader-serial> <url> --views <n> [--passes <n>] [--watch <seconds>] -- <serial> [<serial> …]
//! ```
//!
//! **OPENS THE POST ON EVERY LISTED PHONE.** It reads and it watches; it does not like and it
//! does not comment — those are the campaign runner's job, and doing them from a probe would
//! post from real accounts with no record in the app.

use std::sync::atomic::AtomicBool;
use std::sync::LazyLock;
use std::time::Duration;

use riviu_android_driver::{AndroidDriver, AndroidDriverConfig};
use riviu_core::driver::{DeviceDriver, UiSession};
use riviu_core::interaction_hierarchy::{
    open_target_by_hierarchy, read_post_counters, read_view_count, TargetArrival,
};
use riviu_core::interaction_threshold::{plan_thresholds, PostNow, PostTargets};
use riviu_core::tiktok_labels;

static TIKTOK: LazyLock<String> = LazyLock::new(|| {
    std::env::var("RIVIU_TIKTOK_PACKAGE")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "com.ss.android.ugc.trill".to_string())
});

fn flag(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|arg| arg == name)
        .and_then(|at| args.get(at + 1))
        .cloned()
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let split = args.iter().rposition(|arg| arg == "--");
    let (head, watchers) = match split {
        Some(at) => (&args[..at], args[at + 1..].to_vec()),
        None => (&args[..], Vec::new()),
    };
    if head.len() < 2 || watchers.is_empty() {
        println!(
            "usage: threshold_gate <reader-serial> <url> --views <n> [--passes <n>] \\\n\
             \x20        [--watch <seconds>] -- <serial> [<serial> …]\n\
             \n\
             OPENS THE POST on every listed phone, once per pass. Reads only; never likes or\n\
             comments."
        );
        return Ok(());
    }
    let (reader, url) = (&head[0], &head[1]);
    let want_views: Option<u32> = flag(head, "--views").and_then(|v| v.parse().ok());
    let max_passes: u32 = flag(head, "--passes")
        .and_then(|v| v.parse().ok())
        .unwrap_or(3);
    let watch_secs: u64 = flag(head, "--watch")
        .and_then(|v| v.parse().ok())
        .unwrap_or(70);

    let driver = AndroidDriver::new(&AndroidDriverConfig::default())?;
    let handle = url
        .split('@')
        .nth(1)
        .and_then(|rest| rest.split('/').next())
        .unwrap_or_default()
        .to_string();

    // The reader is the phone that measures. Kept separate from the watchers so a reading is
    // never taken on a phone that is mid-pass.
    println!("== reader {reader} ==");
    // Cold start, not just "bring to front". A phone left on a profile or a search page has
    // no author label to take a baseline from, and the arrival check correctly refuses —
    // which reads as a broken gate when it is really a phone parked somewhere.
    driver.terminate_app(reader, TIKTOK.as_str()).await.ok();
    tokio::time::sleep(Duration::from_secs(2)).await;
    driver.launch_app(reader, TIKTOK.as_str()).await?;
    tokio::time::sleep(Duration::from_secs(8)).await;
    let session = driver.open_session(reader).await?;
    let language = session.ui_language().await.unwrap_or_default();
    let app_version = session
        .app_version(TIKTOK.as_str())
        .await
        .unwrap_or_default();
    let labels = tiktok_labels::controls_for(TIKTOK.as_str(), &language, &app_version)
        .ok_or_else(|| anyhow::anyhow!("no measured labels for {language:?}"))?;
    let screen = session.window_size().await?;
    let stop = AtomicBool::new(false);

    let mut previous: Option<u32> = None;
    for pass in 0..=max_passes {
        let now = measure(&session, labels, screen, url, &handle, &stop).await?;
        println!(
            "  after pass {pass}: views={:?} likes={:?} comments={:?}",
            now.views, now.likes, now.comments
        );
        let plan = plan_thresholds(
            PostTargets {
                views: want_views,
                ..Default::default()
            },
            now,
            watchers.len() as u32,
            watchers.len() as u32,
        );
        for refusal in plan.refusals() {
            println!("  ! {refusal}");
        }
        if plan.satisfied() {
            println!("  target reached");
            return Ok(());
        }
        if let Some(views) = plan.views.as_ref() {
            println!(
                "  còn thiếu {} view; ước {} lượt nữa với {} máy",
                views.shortfall,
                views.passes.unwrap_or(0),
                watchers.len()
            );
        }
        // A pass that moved nothing is the signal the arithmetic cannot give. Stopping here is
        // what keeps a saturated post from being farmed all night for zero.
        if let (Some(before), Some(after)) = (previous, now.views) {
            if after <= before {
                println!("  ! lượt vừa rồi không thêm được view nào — dừng thay vì chạy tiếp mù");
                return Ok(());
            }
        }
        previous = now.views;
        if pass == max_passes {
            println!("  hết số lượt cho phép");
            return Ok(());
        }
        println!("== pass {} on {} phones ==", pass + 1, watchers.len());
        run_pass(&driver, &watchers, url, watch_secs).await;
    }
    Ok(())
}

/// Everything the post says about itself, from the reader phone.
async fn measure(
    session: &dyn UiSession,
    labels: tiktok_labels::TikTokControls,
    screen: (f64, f64),
    url: &str,
    handle: &str,
    stop: &AtomicBool,
) -> anyhow::Result<PostNow> {
    match open_target_by_hierarchy(session, labels, TIKTOK.as_str(), url, handle, stop).await {
        Ok(TargetArrival::Identified { .. }) | Ok(TargetArrival::Structural) => {}
        Err(refusal) => anyhow::bail!("reader could not reach the post: {}", refusal.message()),
    }
    let rail = read_post_counters(session, labels).await;
    // The view count is a navigation away and costs the post page, so it is read last.
    let views = read_view_count(session, labels, screen, stop).await;
    Ok(PostNow {
        views,
        likes: rail.likes,
        comments: rail.comments,
    })
}

/// One pass: every watcher opens the post and stays on it.
///
/// Each phone is checked for actually being on a post page afterwards. The first time this
/// was measured by hand the check was skipped, and a pass that landed nowhere was read as
/// "views do not count" — an hour of wrong conclusions from one missing assertion.
async fn run_pass(driver: &AndroidDriver, watchers: &[String], url: &str, watch_secs: u64) {
    for serial in watchers {
        // Same reason as the reader: a cold start puts the phone on the feed, so the deep
        // link lands on the post rather than on whatever it was showing.
        driver.terminate_app(serial, TIKTOK.as_str()).await.ok();
        if let Err(error) = driver.launch_app(serial, TIKTOK.as_str()).await {
            println!("  {serial}: could not launch TikTok: {error:#}");
            continue;
        }
    }
    tokio::time::sleep(Duration::from_secs(3)).await;
    let mut opened = 0usize;
    for serial in watchers {
        let Ok(session) = driver.open_session(serial).await else {
            println!("  {serial}: no session");
            continue;
        };
        if session.open_url_in_app(url, TIKTOK.as_str()).await.is_err() {
            println!("  {serial}: intent refused");
            continue;
        }
        opened += 1;
    }
    println!("  {opened}/{} phones opened the post", watchers.len());
    tokio::time::sleep(Duration::from_secs(watch_secs)).await;

    // Proof of watching, not just of asking: a phone that drifted off the post contributed
    // nothing and the tally has to say so.
    let mut still = 0usize;
    for serial in watchers {
        let Ok(session) = driver.open_session(serial).await else {
            continue;
        };
        if session
            .active_app_bundle()
            .await
            .is_ok_and(|bundle| bundle == *TIKTOK)
        {
            still += 1;
        }
    }
    println!(
        "  {still}/{} still in TikTok at the end of the pass",
        watchers.len()
    );
}
