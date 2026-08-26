//! Read the labels a phone's TikTok actually shows, so a label set can be *measured*.
//!
//! ```text
//! cargo run -p riviu-android-driver --example label_scout -- <serial> [--tap "Home"] [--raw]
//! ```
//!
//! Read-only except for `--tap`, which taps one control by its exact `content-desc` so the
//! next dump can be taken on a different screen. Nothing is liked, posted or sent.
//!
//! This exists because `tiktok_labels` refuses anything it has not measured, and refusing is
//! the right answer — a guessed translation produces a locator that silently matches nothing.
//! The consequence is that adding a (package, language) pair is a *measurement*, and this is
//! the instrument. Measured 18/08/2026: sixteen of eighteen phones on this fleet run
//! `com.ss.android.ugc.trill` with an English UI, a pair that had never been read.

use std::collections::BTreeSet;
use std::io::Write;

use riviu_android_driver::AndroidDriver;
use riviu_core::driver::{DeviceDriver, UiSession};

#[path = "common/mod.rs"]
mod common;

/// Pull every `content-desc` and `text` out of a uiautomator XML dump.
///
/// Deliberately a scan rather than a parser: the dump is one line of XML with escaped
/// attributes, the question is only "which strings does this screen offer", and a scan
/// cannot fail on a shape it did not expect.
/// Print and flush: this output is read from a pipe, where Rust block-buffers stdout, and a
/// run that is cut short would otherwise report nothing at all about what it had read.
fn say(line: &str) {
    println!("{line}");
    let _ = std::io::stdout().flush();
}

fn attributes(dump: &str, key: &str) -> BTreeSet<String> {
    let needle = format!("{key}=\"");
    let mut found = BTreeSet::new();
    let mut rest = dump;
    while let Some(at) = rest.find(&needle) {
        rest = &rest[at + needle.len()..];
        let Some(end) = rest.find('"') else { break };
        let value = rest[..end].trim();
        if !value.is_empty() {
            found.insert(
                value
                    .replace("&amp;", "&")
                    .replace("&lt;", "<")
                    .replace("&gt;", ">")
                    .replace("&quot;", "\""),
            );
        }
        rest = &rest[end..];
    }
    found
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(serial) = args.first() else {
        println!("usage: label_scout <serial> [--tap \"<content-desc>\"] [--raw]");
        return Ok(());
    };

    let driver = AndroidDriver::new(&common::repo_config())?;
    let package = driver.resolve_tiktok_package(serial).await?;
    let locale = driver
        .device_shell(
            serial,
            "getprop persist.sys.locale; getprop ro.product.locale",
        )
        .await
        .map(|outcome| {
            outcome
                .stdout
                .trim()
                .lines()
                .next()
                .unwrap_or("")
                .to_string()
        })
        .unwrap_or_default();
    println!("serial   {serial}\npackage  {package}\nlocale   {locale}\n");

    let session = driver.open_session(serial).await?;
    // **`--no-launch` exists because launching is not free of consequences.** Bringing
    // TikTok to the front resets its feed to the top card, so two dumps taken either side
    // of a swipe both read the *same* card and a swipe that worked looks like one that did
    // not. Any measurement of what a gesture changed has to skip this.
    if !args.iter().any(|arg| arg == "--no-launch") {
        driver.launch_app(serial, &package).await?;
        tokio::time::sleep(std::time::Duration::from_secs(4)).await;
    }

    // A vertical swipe, in the same direction and shape the nurture loop uses, so that
    // "does this phone advance at all" can be asked without running a whole session.
    if args.iter().any(|arg| arg == "--swipe") {
        let (width, height) = session.window_size().await.unwrap_or((1_080.0, 2_220.0));
        say(&format!("swiping up on {width}x{height}"));
        session
            .swipe(riviu_core::types::SwipeGesture {
                from: riviu_core::types::TapPoint {
                    x: width / 2.0,
                    y: height * 0.78,
                },
                to: riviu_core::types::TapPoint {
                    x: width / 2.0,
                    y: height * 0.22,
                },
                duration_ms: 250,
            })
            .await
            .unwrap_or_else(|error| say(&format!("  (swipe failed: {error})")));
        tokio::time::sleep(std::time::Duration::from_secs(3)).await;
    }

    if let Some(index) = args.iter().position(|arg| arg == "--tap") {
        if let Some(target) = args.get(index + 1) {
            say(&format!("tapping {target:?}"));
            session
                .find_and_tap(target)
                .await
                .unwrap_or_else(|error| say(&format!("  (tap failed: {error})")));
            tokio::time::sleep(std::time::Duration::from_secs(3)).await;
        }
    }

    let dump = session.agent().source().await?;
    if args.iter().any(|arg| arg == "--raw") {
        say(&dump);
        return Ok(());
    }

    say("--- content-desc ---");
    for value in attributes(&dump, "content-desc") {
        say(&format!("  {value}"));
    }
    println!("\n--- text ---");
    for value in attributes(&dump, "text") {
        say(&format!("  {value}"));
    }
    Ok(())
}
