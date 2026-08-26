//! Gate: does an `@handle` become a **real mention** on a real phone?
//!
//! The interaction feature used to prepend `@name` as plain characters. TikTok renders those
//! grey, links nothing and notifies nobody — measured on a live post on 24/08/2026, which is
//! the report that started this. A real mention only exists if it was *picked out of TikTok's
//! own suggestion list*, and this gate proves the production path does that.
//!
//! It calls [`send_root_by_hierarchy`] — the same function the campaign runner calls, not a
//! re-implementation — so a pass here is a pass for the product. What it exercises:
//!
//! 1. the body goes in through `set_text`, the only path that carries Vietnamese;
//! 2. each handle goes in as **real key events**, which is what opens and filters the picker
//!    (accessibility text does not: the app never sees a keystroke);
//! 3. the row whose text *is* the handle is tapped, and only that row.
//!
//! ```text
//! RIVIU_TIKTOK_PACKAGE=com.ss.android.ugc.trill \
//!   cargo run -p riviu-android-driver --example mention_gate -- \
//!   <serial> <url> "<comment body>" <handle> [<handle> …]
//! ```
//!
//! **POSTS A PUBLIC COMMENT.** One, from the given phone, on the given post.

use std::sync::atomic::AtomicBool;
use std::sync::LazyLock;
use std::time::Duration;

use riviu_android_driver::AndroidDriver;
use riviu_core::driver::{DeviceDriver, UiSession};
use riviu_core::interaction_hierarchy::{
    open_target_by_hierarchy, send_root_by_hierarchy, TargetArrival,
};
use riviu_core::tiktok_labels::{self, TikTokControl};

#[path = "common/mod.rs"]
mod common;

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
    if args.len() < 3 {
        println!(
            "usage: mention_gate <serial> <url> \"<body>\" <handle> [<handle> …]\n\
             \n\
             POSTS ONE PUBLIC COMMENT on <url> from <serial>, tagging each <handle> by\n\
             picking it out of TikTok's suggestion list."
        );
        return Ok(());
    }
    let (serial, url, body) = (&args[0], &args[1], &args[2]);
    let handles: Vec<String> = args[3.min(args.len())..]
        .iter()
        .map(|h| h.trim_start_matches('@').to_string())
        .collect();
    // The post's own author, which is what the arrival check compares against.
    let handle = url
        .split('@')
        .nth(1)
        .and_then(|rest| rest.split('/').next())
        .unwrap_or_default()
        .to_string();

    let driver = AndroidDriver::new(&common::repo_config())?;
    println!("== opening {serial} ==");
    driver.launch_app(serial, TIKTOK.as_str()).await?;
    tokio::time::sleep(Duration::from_secs(8)).await;
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
    let screen = session.window_size().await?;
    println!("  {language:?} app {app_version:?} screen {screen:?}");
    anyhow::ensure!(
        labels.label(TikTokControl::CommentSend).is_some(),
        "no measured Send control for app {app_version:?}"
    );

    let stop = AtomicBool::new(false);
    println!("== arriving at the post ==");
    match open_target_by_hierarchy(&session, labels, TIKTOK.as_str(), url, &handle, &stop).await {
        Ok(TargetArrival::Identified { author_label }) => {
            println!("  arrival Identified ({author_label})")
        }
        Ok(TargetArrival::Structural) => println!("  arrival Structural"),
        Err(refusal) => anyhow::bail!(
            "arrival refused ({}): {}",
            refusal.code(),
            refusal.message()
        ),
    }

    // What the post says about itself right now — the numbers any threshold is measured
    // against. Read before the send so the "before" is honest.
    let before = riviu_core::interaction_hierarchy::read_post_counters(&session, labels).await;
    println!(
        "== counters: likes={:?} comments={:?}{} ==",
        before.likes,
        before.comments,
        if before.approximate {
            " (rounded by TikTok)"
        } else {
            ""
        }
    );
    if handles.is_empty() {
        println!("(no handles given — stopping before the send)");
        return Ok(());
    }

    println!("== sending, tagging {handles:?} ==");
    let outcome =
        send_root_by_hierarchy(&session, labels, screen, body, &handles, &stop, String::new)
            .await?;
    println!(
        "  verdict = {:?} ({})",
        outcome.verdict,
        outcome.verdict.reason()
    );
    match &outcome.mention_note {
        Some(note) => println!("  tags   = {note}"),
        None => println!("  tags   = (none reported — the picker never ran)"),
    }
    match &outcome.identity {
        Some(identity) => println!(
            "  posted = {:?} by {:?}",
            identity.text, identity.author_label
        ),
        None => println!("  posted = (could not read the comment back)"),
    }
    Ok(())
}
