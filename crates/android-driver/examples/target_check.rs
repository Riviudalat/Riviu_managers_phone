//! Ask a phone which of a list of TikTok links it can actually open.
//!
//! Read-only: it opens each link and reads the tree. No tap, no text, nothing posted.
//!
//! ```text
//! RIVIU_ADB_PATH=… RIVIU_TIKTOK_PACKAGE=com.ss.android.ugc.trill \
//!   cargo run -p riviu-android-driver --example target_check -- <serial> <url> [<url> …]
//! ```
//!
//! Why this exists rather than checking links from the host: **an HTTP fetch cannot tell
//! you.** Measured 11/08/2026 — `curl -L` returns `200` and
//! `<title>TikTok - Make Your Day</title>` for both a live post and a dead one, serves a
//! captcha shell instead of post data, and reported `Video currently unavailable` for a post
//! that opened perfectly well on the device. The phone is the only authority.
//!
//! The verdict per link is the shipped [`open_target_by_hierarchy`], so a link this says
//! `ARRIVED` for is a link a campaign can use, and one it refuses is one a campaign would
//! refuse — with the same reason code.

use std::sync::atomic::AtomicBool;
use std::sync::LazyLock;
use std::time::{Duration, Instant};

use riviu_android_driver::{AndroidDriver, AndroidDriverConfig};
use riviu_core::driver::{DeviceDriver, UiSession};
use riviu_core::interaction_hierarchy::{open_target_by_hierarchy, TargetArrival};
use riviu_core::tiktok_labels;

static TIKTOK: LazyLock<String> = LazyLock::new(|| {
    std::env::var("RIVIU_TIKTOK_PACKAGE")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "com.ss.android.ugc.trill".to_string())
});

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() < 2 {
        println!("usage: target_check <serial> <url> [<url> …]   (read-only)");
        return Ok(());
    }
    let serial = &args[0];
    // Flags filtered out, or `--dump-text` gets opened as if it were a link.
    let urls: Vec<String> = args[1..]
        .iter()
        .filter(|arg| !arg.starts_with("--"))
        .cloned()
        .collect();
    if urls.is_empty() {
        println!("usage: target_check <serial> <url> [<url> …] [--dump-text]");
        return Ok(());
    }

    let driver = AndroidDriver::new(&AndroidDriverConfig::default())?;
    driver.launch_app(serial, TIKTOK.as_str()).await?;
    tokio::time::sleep(Duration::from_secs(10)).await;
    let session = driver.open_session(serial).await?;
    let language = session.ui_language().await.unwrap_or_default();
    let app_version = session
        .app_version(TIKTOK.as_str())
        .await
        .unwrap_or_default();
    let labels =
        tiktok_labels::controls_for(TIKTOK.as_str(), &language, &app_version).ok_or_else(|| {
            anyhow::anyhow!("no measured labels for {} + {language:?}", TIKTOK.as_str())
        })?;
    println!("device {serial}: {language:?} app {app_version:?}");
    println!("checking {} link(s), read-only\n", urls.len());

    let stop = AtomicBool::new(false);
    let mut arrived = Vec::new();
    let mut refused = Vec::new();
    for (index, url) in urls.iter().enumerate() {
        let handle = url
            .split('@')
            .nth(1)
            .and_then(|rest| rest.split('/').next())
            .unwrap_or_default()
            .to_string();
        // Foregrounded and proved before every link, because between two links the phone
        // can end up anywhere — measured: after ~30 s it had gone back to the launcher, and
        // the arrival check then correctly blamed `com.miui.home`.
        let _ = session.launch_app_foreground(TIKTOK.as_str()).await;
        for _ in 0..20 {
            if session.active_app_bundle().await.unwrap_or_default() == TIKTOK.as_str() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }

        let started = Instant::now();
        let verdict =
            open_target_by_hierarchy(&session, labels, TIKTOK.as_str(), url, &handle, &stop).await;
        let ms = started.elapsed().as_millis();
        match verdict {
            Ok(TargetArrival::Identified { author_label }) => {
                println!(
                    "  {:>2}. ARRIVED  @{handle}  Identified ({author_label})  {ms} ms",
                    index + 1
                );
                arrived.push(url.clone());
            }
            Ok(TargetArrival::Structural) => {
                println!(
                    "  {:>2}. ARRIVED  @{handle}  Structural  {ms} ms",
                    index + 1
                );
                arrived.push(url.clone());
            }
            Err(refusal) => {
                println!(
                    "  {:>2}. REFUSED  @{handle}  {}  {ms} ms",
                    index + 1,
                    refusal.code()
                );
                refused.push((url.clone(), refusal.code().to_string()));
            }
        }
    }

    // `--dump-text` prints every `TextView`'s text on whatever is on screen after the last
    // link. It exists for one question: a photo carousel shows `1 / 7` on screen, and if
    // that is in the tree then "swipe through all of it" is a **known** number rather than
    // a guess — which is the difference between sampling a carousel and reading it.
    if args.iter().any(|arg| arg == "--dump-text") {
        println!("\n== every TextView on screen (for the carousel counter) ==");
        match session
            .locate_all_described(riviu_core::ElementQuery::ClassName(
                "android.widget.TextView",
            ))
            .await
        {
            Ok(found) => {
                for element in &found {
                    if let Some(text) = element.description.as_deref() {
                        println!(
                            "  [{:>4},{:>4} {:>4}x{:<4}] {text:?}",
                            element.x as i64,
                            element.y as i64,
                            element.width as i64,
                            element.height as i64
                        );
                    }
                }
                println!("  ({} TextView(s))", found.len());
            }
            Err(error) => println!("  FAILED: {error:#}"),
        }
    }

    println!("\n== summary ==");
    println!("  usable: {} of {}", arrived.len(), urls.len());
    for url in &arrived {
        println!("    {url}");
    }
    if !refused.is_empty() {
        println!("  refused:");
        for (url, code) in &refused {
            println!("    [{code}] {url}");
        }
        println!(
            "\n  A `target_open_screen_unchanged` refusal means TikTok took the intent and\n  \
             left the feed alone — the post is deleted, private, or region-blocked. It is\n  \
             the link, not the phone."
        );
    }
    Ok(())
}
