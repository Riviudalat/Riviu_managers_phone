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

use riviu_android_driver::AndroidDriver;
use riviu_core::driver::{DeviceDriver, UiSession};
use riviu_core::interaction_hierarchy::{
    open_target_by_hierarchy, read_author_label, read_post_caption, read_post_counters,
    read_post_now, PostCounters, TargetArrival,
};
use riviu_core::interaction_threshold::{plan_thresholds, PostNow, PostTargets};
use riviu_core::tiktok_labels;

#[path = "common/mod.rs"]
mod common;

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

    let driver = AndroidDriver::new(&common::repo_config())?;
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
    // confirmed **seven**. One read a different post's caption, one never reached the foreground
    // inside 40 s, and two `com.zhiliaoapp.musically` phones could not have their caption read at
    // all.
    //
    // **Confirmed, not landed** — and the distinction is one I got wrong first time round. A
    // later logcat comparison showed the deep link reaching `AppLinkHandlerV2` on a phone that
    // confirmed and on a phone that did not, both ending on `MainActivity`, so TikTok pushes the
    // post *inside* that activity and the foreground component says nothing about which post is
    // showing. The unconfirmed four may well have been on the post with a caption this build
    // reads differently. Estimating from eleven would still overstate the rate — an estimate that
    // flatters the fleet is how a threshold ends up chasing a number it will not reach — but
    // estimating from seven understates it by however many of those four were really there.
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
        let Some(author) = reading.author.clone() else {
            anyhow::bail!(
                "reader could not read the post's author label, \n                 so no watcher could be checked against it"
            );
        };
        let identity = PostIdentity {
            author,
            likes: now.likes,
            comments: now.comments,
        };
        println!("== pass {} on {} phones ==", pass + 1, watchers.len());
        let tally = run_pass(&driver, &watchers, url, &identity, watch_secs).await;
        // The **lower** bound drives the estimate: it is the only number that is evidence. The
        // upper bound is printed next to it so the error bar is not invisible.
        landing = Some(tally.confirmed);
        if tally.upper() == 0 {
            println!(
                "  ! không máy nào xác nhận đang ở bài, và không máy nào để ngỏ — lượt này không \
                 đo được gì, dừng lại"
            );
            return Ok(());
        }
        if tally.upper() > tally.confirmed {
            println!(
                "  (ước lượng dùng {} máy chắc chắn; tối đa có thể là {} nếu mấy máy không đọc \
                 được caption vẫn ở trên bài)",
                tally.confirmed,
                tally.upper()
            );
        }
    }
    Ok(())
}

/// What one pass could be shown to have done, split by *why* a phone did not count.
///
/// One number was not enough, and lumping them cost a wrong conclusion. "The caption says a
/// different post" is a phone that was somewhere else. "The caption could not be read" is a
/// phone this gate cannot speak about — it may have been on the post the whole time with a
/// caption this build renders in a class nobody has measured. Reporting both as "did not land"
/// turned an unknown into a claim.
#[derive(Debug, Default, Clone, Copy)]
struct PassTally {
    /// Caption matched the post's. The only number that is evidence.
    confirmed: usize,
    /// Caption read, and it belonged to something else.
    wrong_post: usize,
    /// TikTok was up but no caption came back — unknown, not negative.
    unreadable: usize,
    /// The app never reached the foreground inside the arrival window.
    absent: usize,
}

impl PassTally {
    /// The most phones that could have been on the post: confirmed plus the ones we cannot
    /// speak about. Printed beside `confirmed` so the estimate's error bar is visible.
    fn upper(&self) -> usize {
        self.confirmed + self.unreadable
    }
}

/// What identifies a post **across phones**.
///
/// **Not the caption**, and getting that wrong cost two reports to the operator. Measured
/// 24/08/2026 with the same post open on four phones at once: three read
/// `Một list gọn để lên Đà Lạt mà không phải…` and one read
/// `A compact list to go to Da Lat without struggling to choose…` — TikTok localises the caption
/// to the account's language — and every one of them was **truncated with an ellipsis** at a
/// length that depends on the screen. A caption identifies a post only *within one phone*, which
/// is exactly how `read_view_count` uses it and exactly not what a pass needs.
///
/// The author and the post's own counters are properties of the **post**, not of the viewer: the
/// same on every phone at a given moment, in every language, untruncated.
struct PostIdentity {
    /// The **reader's** author label, compared against each watcher's own.
    ///
    /// Not the URL's handle through `author_matches_handle`: measured 24/08/2026, that returns
    /// `false` for this very account, because `.lt.gi.mang.v` abbreviates each word of
    /// `Đà Lạt Gói Mang Về` instead of taking a prefix, and the comparison is run-containment.
    /// Two phones reading the same account's nickname get the same string, which is all this
    /// needs — and it needs no heuristic at all.
    author: String,
    likes: Option<u32>,
    comments: Option<u32>,
}

/// How far a counter may drift between the reader's reading and a watcher's, and still be the
/// same post.
///
/// A pass is about ninety seconds and the post is live, so a like or a comment can land inside
/// it — the reader's numbers are from before the pass. Three is wide enough for that and narrow
/// enough that two *different* posts by the same author would have to agree on both counters to
/// within three to be confused, which the author check has already made unlikely.
const COUNT_DRIFT: u32 = 3;

impl PostIdentity {
    /// Whether what this phone is showing is the same post.
    fn matches(&self, author_label: Option<&str>, seen: &PostCounters) -> Option<bool> {
        let author = author_label?;
        if !author.trim().eq_ignore_ascii_case(self.author.trim()) {
            return Some(false);
        }
        Some(near(self.likes, seen.likes) && near(self.comments, seen.comments))
    }
}

/// Two readings of the same counter, allowing for the post moving under us.
///
/// `None` on either side is not a mismatch: a build that cannot state a number says nothing about
/// which post this is, and treating silence as disagreement would fail every phone on a build
/// where the counted-like control was never measured.
fn near(reader: Option<u32>, seen: Option<u32>) -> bool {
    match (reader, seen) {
        (Some(a), Some(b)) => a.abs_diff(b) <= COUNT_DRIFT,
        _ => true,
    }
}

/// One reading of the post: its numbers, and the caption that identifies it.
struct Reading {
    now: PostNow,
    /// The author's nickname as **this** phone renders it, read right after arrival.
    ///
    /// Taken here rather than from `TargetArrival::Identified`, because arrival at this account
    /// resolves `Structural`: its handle abbreviates the nickname and `author_matches_handle`
    /// therefore does not fire. The label is still on the rail and still readable.
    author: Option<String>,
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
    let author = read_author_label(session, labels).await;
    // The same function the desktop's `interaction_measure_post` calls, not a second copy of the
    // same reads in a different order — a gate that measures a re-implementation proves nothing
    // about the product.
    let now = read_post_now(session, labels, screen, true, stop).await;
    Ok(Reading {
        now,
        caption,
        author,
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
    identity: &PostIdentity,
    watch_secs: u64,
) -> PassTally {
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
    let mut tally = PassTally::default();
    for serial in watchers {
        let Ok(session) = driver.open_session(serial).await else {
            println!("  {serial}: no session");
            tally.absent += 1;
            continue;
        };
        let wanted = package_for(driver, serial).await;
        let foreground = session
            .active_app_bundle()
            .await
            .is_ok_and(|bundle| bundle == wanted);
        if !foreground {
            println!("  {serial}: {wanted} không ở foreground sau {ARRIVAL_WINDOW:?}");
            tally.absent += 1;
            continue;
        }
        // Labels per phone: the fleet runs two packages and two UI languages, and an author
        // label is read through whichever pair this phone is.
        let language = session.ui_language().await.unwrap_or_default();
        let app_version = session.app_version(&wanted).await.unwrap_or_default();
        let Some(labels) = tiktok_labels::controls_for(&wanted, &language, &app_version) else {
            tally.unreadable += 1;
            println!("  {serial}: chưa đo nhãn cho {wanted} + {language:?}");
            continue;
        };
        let author = read_author_label(&session, labels).await;
        let seen = read_post_counters(&session, labels).await;
        match identity.matches(author.as_deref(), &seen) {
            Some(true) => tally.confirmed += 1,
            Some(false) => {
                tally.wrong_post += 1;
                println!(
                    "  {serial}: bài khác — tác giả {:?}, tim {:?}, bình luận {:?}",
                    author.as_deref().unwrap_or("?"),
                    seen.likes,
                    seen.comments
                );
            }
            // The author label is the one thing this cannot do without. No label is not a
            // negative: TikTok pushes the post inside `MainActivity`, so a phone whose rail this
            // build labels differently looks exactly like one on the feed.
            None => {
                tally.unreadable += 1;
                println!("  {serial}: không đọc được nhãn tác giả — không kết luận được");
            }
        }
    }
    println!(
        "  {}/{} máy xác nhận đúng bài · {} bài khác · {} không đọc được · {} không lên foreground",
        tally.confirmed,
        watchers.len(),
        tally.wrong_post,
        tally.unreadable,
        tally.absent
    );
    tokio::time::sleep(Duration::from_secs(watch_secs)).await;
    tally
}
