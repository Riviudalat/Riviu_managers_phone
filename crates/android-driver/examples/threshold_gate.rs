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
//! **A pass has to land before it can count, and the first version of this gate did not.** It
//! force-stopped TikTok, slept 2 s, ran a plain launch, slept 3 s, and only then fired the
//! deep link — into a splash screen. AGENTS.md §9.19 measured TikTok reaching foreground
//! **15.86 / 19.71 / 19.42 s** after `am force-stop`, once **26.9 s**, which is why production
//! waits 40 s. Every "+0" this gate reported was taken through that hole, including the one on
//! the operator's own post. The three passes that really moved a number were run by hand as a
//! single shell command — `am force-stop …; am start -a VIEW -d <url>` — where
//! ActivityManager queues the intent and TikTok handles it as its launch intent on the way up.
//! That is what [`open_on`] now does, and [`run_pass`] no longer counts a phone unless the
//! phone can be shown to be looking at the post.
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
    open_target_by_hierarchy, read_post_caption, read_post_counters, read_view_count, TargetArrival,
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

/// How long a cold TikTok gets to reach the post before a phone is written off for this pass.
///
/// AGENTS.md §9.19 measured the app reaching foreground 15.86 / 19.71 / 19.42 s after
/// `am force-stop`, and once 26.9 s. Production uses 40 s for exactly this, so this uses 40 s
/// too rather than inventing a second number for the same event.
const ARRIVAL_WINDOW: Duration = Duration::from_secs(40);

fn flag(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|arg| arg == name)
        .and_then(|at| args.get(at + 1))
        .cloned()
}

/// The URL is about to go into a shell command, so it is checked rather than trusted.
///
/// Not defence against a hostile operator — defence against a paste that silently changes what
/// the command does. `device_shell` hands the string to `/system/bin/sh` on the phone.
fn safe_url(url: &str) -> anyhow::Result<&str> {
    let allowed = |c: char| {
        c.is_ascii_alphanumeric()
            || matches!(
                c,
                ':' | '/' | '.' | '-' | '_' | '?' | '=' | '&' | '@' | '~' | '%'
            )
    };
    anyhow::ensure!(
        url.starts_with("https://") && url.chars().all(allowed),
        "refusing {url:?}: a post URL has to be https and free of shell metacharacters"
    );
    Ok(url)
}

/// Force-stop, then fire the deep link **in the same command**.
///
/// `am start -a VIEW` against a stopped app is a cold start whose launch intent *is* the link,
/// so ActivityManager queues it and TikTok handles it when it comes up. Splitting that into
/// stop / launch / link — with sleeps between — is what fired the link at a splash screen; see
/// the module header for the measurement.
async fn open_on(driver: &AndroidDriver, serial: &str, url: &str) -> anyhow::Result<()> {
    let package = TIKTOK.as_str();
    driver
        .device_shell(
            serial,
            &format!(
                "am force-stop {package}; am start -a android.intent.action.VIEW \
                 -c android.intent.category.BROWSABLE -d '{url}' -p {package}"
            ),
        )
        .await
        .map(|_| ())
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
    let (reader, url) = (&head[0], safe_url(&head[1])?);
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
    // Same one-command cold start as the watchers, and the same 40 s. A phone left on a profile
    // or a search page has no author label to take a baseline from, and the arrival check
    // correctly refuses — which reads as a broken gate when it is really a phone parked
    // somewhere.
    open_on(&driver, reader, url).await?;
    tokio::time::sleep(ARRIVAL_WINDOW).await;
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

    // The post's own caption, read once, from the phone that is definitely on the post. It is
    // what every watcher is checked against afterwards — the counters are what a threshold is
    // moving, so they cannot identify a post, and the caption does not change.
    let caption = read_post_caption(&session)
        .await
        .ok_or_else(|| anyhow::anyhow!("reader could not read the post's caption"))?;
    println!(
        "  caption = {:?}",
        caption.chars().take(60).collect::<String>()
    );

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
        // what keeps a saturated post from being farmed all night for zero — but only a pass
        // that *landed* is evidence of anything, which is what `confirmed` below is for.
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
        let confirmed = run_pass(&driver, &watchers, url, &caption, watch_secs).await;
        if confirmed == 0 {
            println!("  ! không máy nào xác nhận đang ở bài — lượt này không đo được gì, dừng lại");
            return Ok(());
        }
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

/// One pass: every watcher opens the post and stays on it. Returns how many were *shown* to be.
///
/// The number that matters is the confirmed one, and the first version of this counted the
/// wrong thing entirely: it reported how many `open_url_in_app` calls returned `Ok`, which only
/// says ActivityManager accepted an intent. That is not a fact about any screen, and it read as
/// "10/10 phones opened the post" on passes where the link had gone to a splash screen. A phone
/// is counted here only when its own caption matches the post's.
async fn run_pass(
    driver: &AndroidDriver,
    watchers: &[String],
    url: &str,
    caption: &str,
    watch_secs: u64,
) -> usize {
    // Fired at every phone first, so the fleet cold-starts in parallel rather than in series.
    let mut fired = 0usize;
    for serial in watchers {
        match open_on(driver, serial, url).await {
            Ok(()) => fired += 1,
            Err(error) => println!("  {serial}: intent không gửi được: {error:#}"),
        }
    }
    println!("  {fired}/{} máy nhận được intent", watchers.len());
    tokio::time::sleep(ARRIVAL_WINDOW).await;

    // **Proof of watching, not of asking.** Checked before the dwell, so a phone counted here
    // was on the post for the whole of it.
    let mut confirmed = 0usize;
    for serial in watchers {
        let Ok(session) = driver.open_session(serial).await else {
            println!("  {serial}: no session");
            continue;
        };
        let foreground = session
            .active_app_bundle()
            .await
            .is_ok_and(|bundle| bundle == *TIKTOK);
        if !foreground {
            println!("  {serial}: TikTok không ở foreground sau {ARRIVAL_WINDOW:?}");
            continue;
        }
        match read_post_caption(&session).await {
            Some(seen) if seen == caption => confirmed += 1,
            Some(seen) => println!(
                "  {serial}: đang ở bài khác ({:?})",
                seen.chars().take(40).collect::<String>()
            ),
            None => println!("  {serial}: không đọc được caption, không tính"),
        }
    }
    println!(
        "  {confirmed}/{} máy xác nhận đang ở đúng bài",
        watchers.len()
    );
    tokio::time::sleep(Duration::from_secs(watch_secs)).await;
    confirmed
}
