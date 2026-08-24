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

/// Fallback only. **Not** the package for the fleet — see [`package_for`].
static TIKTOK_FALLBACK: LazyLock<String> = LazyLock::new(|| {
    std::env::var("RIVIU_TIKTOK_PACKAGE")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "com.ss.android.ugc.trill".to_string())
});

/// Which TikTok **this phone** has.
///
/// Measured 24/08/2026 on this farm: eleven of fourteen phones carry
/// `com.ss.android.ugc.trill` and **three carry `com.zhiliaoapp.musically`**. This gate used to
/// take one package from the environment and use it for every phone, so on those three
/// `am force-stop com.ss.android.ugc.trill` stopped nothing, `am start -p …trill` started
/// nothing, and the foreground check compared against a package that is not installed. All
/// three were then reported as "TikTok không ở foreground sau 40s" — a phone problem, when it
/// was a configuration one. Production never had this: `DeviceDriver::resolve_tiktok_package`
/// resolves per device and caches, and the campaign runner goes through it.
async fn package_for(driver: &AndroidDriver, serial: &str) -> String {
    driver
        .resolve_tiktok_package(serial)
        .await
        .unwrap_or_else(|_| TIKTOK_FALLBACK.clone())
}

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
///
/// `KEYCODE_WAKEUP` first, because a sleeping screen is one of the ways a pass silently does not
/// land: measured 24/08/2026, `ce0517151215a00304` sat in `UnintentionalLcdOn` through a whole
/// pass. Waking is idempotent on a screen that is already on, and it does **not** unlock — a
/// locked phone still fails the confirmation, which is the honest outcome rather than a silent
/// one.
async fn open_on(driver: &AndroidDriver, serial: &str, url: &str) -> anyhow::Result<()> {
    let package = package_for(driver, serial).await;
    driver
        .device_shell(
            serial,
            &format!(
                "input keyevent KEYCODE_WAKEUP; am force-stop {package}; \
                 am start -a android.intent.action.VIEW \
                 -c android.intent.category.BROWSABLE -d '{url}' -p {package}"
            ),
        )
        .await
        .map(|_| ())
}

/// Force-stop, then a **plain** launch: the app comes up on the feed, not on the post.
///
/// For the reader, and the difference is the whole reason it exists. `open_target_by_hierarchy`
/// decides it arrived by watching the author label *change* — so a phone already sitting on the
/// target post gives it nothing to observe and it refuses with `target_open_screen_unchanged`,
/// which reads as "the post is gone". That is exactly what happened on the second measurement of
/// the first real run: `read_view_count` had walked the reader post → profile → tile → post, so
/// it was already there. The feed is the only starting screen from which arriving is visible.
async fn cold_feed(driver: &AndroidDriver, serial: &str) -> anyhow::Result<()> {
    let package = package_for(driver, serial).await;
    driver
        .device_shell(
            serial,
            &format!(
                "input keyevent KEYCODE_WAKEUP; am force-stop {package}; \
                 monkey -p {package} -c android.intent.category.LAUNCHER 1"
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
    let package = package_for(&driver, reader).await;
    println!("  package = {package}");
    // Onto the **feed**, not onto the post: `measure` fires the link itself and decides it
    // arrived by watching the author label change. See `cold_feed`.
    cold_feed(&driver, reader).await?;
    tokio::time::sleep(ARRIVAL_WINDOW).await;
    let session = driver.open_session(reader).await?;
    let language = session.ui_language().await.unwrap_or_default();
    let app_version = session.app_version(&package).await.unwrap_or_default();
    let labels = tiktok_labels::controls_for(&package, &language, &app_version)
        .ok_or_else(|| anyhow::anyhow!("no measured labels for {package} + {language:?}"))?;
    let screen = session.window_size().await?;
    let stop = AtomicBool::new(false);

    // The post's own caption comes out of the first reading, taken while the reader is still on
    // the post — it is what every watcher is checked against afterwards, because the counters are
    // what a threshold is moving and so cannot identify a post, while a caption does not change.
    let mut caption: Option<String> = None;
    let mut previous: Option<u32> = None;
    // How many phones the last pass could be **shown** to have put on the post.
    //
    // The estimate uses this rather than the number of phones handed in, because those are
    // different numbers and the difference is large: measured 24/08/2026, eleven attached phones
    // put **seven** on the post — one landed on a different post of the same author, one never
    // reached the foreground inside 40 s, and two run `com.zhiliaoapp.musically`, where the VIEW
    // intent opens the app on its feed instead of the post. Estimating from eleven would overstate
    // the rate by more than a third, and an estimate that flatters the fleet is how a threshold
    // ends up chasing a number it will not reach.
    let mut landing: Option<usize> = None;
    for pass in 0..=max_passes {
        // **Cold onto the feed before every reading, without exception.** `measure` decides it
        // arrived by watching the author label change, and `read_view_count` leaves the reader on
        // the post it just matched — so a second reading from there sees no transition and
        // refuses with `target_open_screen_unchanged`, which reads as "the post is gone". That is
        // what ended the first real run of this gate, and skipping the cold start on pass 0 alone
        // was not enough: the caption read left the reader on the post just as surely.
        cold_feed(&driver, reader).await?;
        tokio::time::sleep(ARRIVAL_WINDOW).await;
        let reading = measure(&session, labels, &package, screen, url, &handle, &stop).await?;
        let now = reading.now;
        if caption.is_none() {
            caption = reading.caption.clone();
            match caption.as_deref() {
                Some(text) => println!(
                    "  caption = {:?}",
                    text.chars().take(60).collect::<String>()
                ),
                None => {
                    println!("  ! không đọc được caption của bài — không xác nhận được máy nào")
                }
            }
        }
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
            landing.unwrap_or(watchers.len()) as u32,
            landing.unwrap_or(watchers.len()) as u32,
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
                "  còn thiếu {} view; ước {} lượt nữa với {} máy{}",
                views.shortfall,
                views.passes.unwrap_or(0),
                landing.unwrap_or(watchers.len()),
                match landing {
                    Some(_) => format!(" đã xác nhận tới bài, trên {} máy nối vào", watchers.len()),
                    None => String::new(),
                }
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
        let Some(caption) = caption.as_deref() else {
            anyhow::bail!(
                "no caption to check the watchers against, so a pass could not be measured"
            );
        };
        println!("== pass {} on {} phones ==", pass + 1, watchers.len());
        let confirmed = run_pass(&driver, &watchers, url, caption, watch_secs).await;
        landing = Some(confirmed);
        if confirmed == 0 {
            println!("  ! không máy nào xác nhận đang ở bài — lượt này không đo được gì, dừng lại");
            return Ok(());
        }
    }
    Ok(())
}

/// One reading of the post: its numbers, and the caption that identifies it.
struct Reading {
    now: PostNow,
    /// Read right after arrival, before `read_view_count` navigates off the post.
    caption: Option<String>,
}

/// Everything the post says about itself, from the reader phone.
#[allow(clippy::too_many_arguments)]
async fn measure(
    session: &dyn UiSession,
    labels: tiktok_labels::TikTokControls,
    package: &str,
    screen: (f64, f64),
    url: &str,
    handle: &str,
    stop: &AtomicBool,
) -> anyhow::Result<Reading> {
    match open_target_by_hierarchy(session, labels, package, url, handle, stop).await {
        Ok(TargetArrival::Identified { .. }) | Ok(TargetArrival::Structural) => {}
        Err(refusal) => anyhow::bail!("reader could not reach the post: {}", refusal.message()),
    }
    // Before anything navigates: `read_view_count` walks off to the profile grid.
    let caption = read_post_caption(session).await;
    let rail = read_post_counters(session, labels).await;
    // The view count is a navigation away and costs the post page, so it is read last.
    let views = read_view_count(session, labels, screen, stop).await;
    Ok(Reading {
        now: PostNow {
            views,
            likes: rail.likes,
            comments: rail.comments,
        },
        caption,
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
        let wanted = package_for(driver, serial).await;
        let foreground = session
            .active_app_bundle()
            .await
            .is_ok_and(|bundle| bundle == wanted);
        if !foreground {
            println!("  {serial}: {wanted} không ở foreground sau {ARRIVAL_WINDOW:?}");
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
