//! Stage two different bundles onto two different phones, and check each got only its own.
//!
//! ```text
//! cargo run -p riviu-android-driver --example publish_two_bundles -- <serial-a> <serial-b>
//! ```
//!
//! **Stops at staging. Nothing is imported into a gallery and nothing is posted.** The files
//! land in the dot-prefixed staging directory the media scanner ignores, and the run removes
//! them again on the way out.
//!
//! This is the check AGENTS.md §9.83 asked for by name: the defect it records sent *every*
//! bundle to *every* phone, so a campaign that paired two folders with two phones could post
//! one account's images under another account's caption, irreversibly. Unit tests pin which
//! bundle is chosen for which assignment; only a phone can say what actually arrived on it.
//!
//! The layout below is built by hand to match `publish_commands::stage_one_bundle` exactly —
//! `<campaign>/.transfer/<ordinal>/<bundle-name>/<images>` — because the layout *is* what is
//! being tested.

use std::path::{Path, PathBuf};

use riviu_android_driver::AndroidDriver;
use riviu_core::driver::DeviceDriver;

#[path = "common/mod.rs"]
mod common;

const CAMPAIGN: &str = "a1-live-check";

fn write_bundle(root: &Path, bundle: &str, ordinal: u32, files: &[(&str, &[u8])]) -> PathBuf {
    // `stage_one_bundle`'s shape: a per-ordinal scratch root holding one directory named
    // after the bundle. The `.transfer` parent is dot-prefixed so it can never be mistaken
    // for a real bundle.
    let staged = root
        .join(".transfer")
        .join(ordinal.to_string())
        .join(bundle);
    std::fs::create_dir_all(&staged).expect("create the staged bundle directory");
    for (name, bytes) in files {
        std::fs::write(staged.join(name), bytes).expect("write a staged image");
    }
    root.join(".transfer").join(ordinal.to_string())
}

async fn staged_names(driver: &AndroidDriver, serial: &str, scope: &str) -> String {
    driver
        .device_shell(
            serial,
            &format!("ls -1 /sdcard/Pictures/.riviu-publish/{scope} 2>/dev/null || echo '(none)'"),
        )
        .await
        .map(|outcome| outcome.stdout.trim().replace('\n', " "))
        .unwrap_or_else(|error| format!("<unreadable: {error}>"))
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (Some(phone_a), Some(phone_b)) = (args.first(), args.get(1)) else {
        println!("usage: publish_two_bundles <serial-a> <serial-b>   (staging only)");
        return Ok(());
    };

    let root = std::env::temp_dir().join("riviu-a1-live-check");
    let _ = std::fs::remove_dir_all(&root);
    // Distinct content per bundle, so "which bundle landed here" is answerable from the
    // bytes and not only from the file names.
    let source_a = write_bundle(
        &root,
        "bundle-a",
        0,
        &[("a-one.jpg", b"AAAA-one"), ("a-two.jpg", b"AAAA-two")],
    );
    let source_b = write_bundle(
        &root,
        "bundle-b",
        1,
        &[("b-one.jpg", b"BBBB-one"), ("b-two.jpg", b"BBBB-two")],
    );

    let driver = AndroidDriver::new(&common::repo_config())?;
    let scope_a = format!("{CAMPAIGN}-0");
    let scope_b = format!("{CAMPAIGN}-1");

    let mut failures = 0;
    for (serial, scope, source) in [
        (phone_a, &scope_a, &source_a),
        (phone_b, &scope_b, &source_b),
    ] {
        print!("staging {} -> {serial} ... ", source.display());
        match driver.stage_publish_media(serial, "", scope, source).await {
            Ok(evidence) => println!("ok: {evidence}"),
            Err(error) => {
                failures += 1;
                println!("FAILED: {error:#}");
            }
        }
    }

    println!("\n--- what is actually on each phone ---");
    println!(
        "  {phone_a} [{scope_a}]: {}",
        staged_names(&driver, phone_a, &scope_a).await
    );
    println!(
        "  {phone_a} [{scope_b}]: {}",
        staged_names(&driver, phone_a, &scope_b).await
    );
    println!(
        "  {phone_b} [{scope_a}]: {}",
        staged_names(&driver, phone_b, &scope_a).await
    );
    println!(
        "  {phone_b} [{scope_b}]: {}",
        staged_names(&driver, phone_b, &scope_b).await
    );

    println!("\n--- cleaning up ---");
    for serial in [phone_a, phone_b] {
        for scope in [&scope_a, &scope_b] {
            let _ = driver
                .device_shell(
                    serial,
                    &format!("rm -rf /sdcard/Pictures/.riviu-publish/{scope}"),
                )
                .await;
        }
    }
    let _ = std::fs::remove_dir_all(&root);
    println!("done ({failures} staging failure(s))");
    Ok(())
}
