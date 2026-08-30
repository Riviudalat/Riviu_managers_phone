//! Drive a phone to TikTok's edit step and dump it, so the last three publish labels can be
//! **measured** instead of guessed.
//!
//! ```text
//! cargo run -p riviu-android-driver --example composer_scout -- <serial> --album "<tên album>" [--images 3]
//! ```
//!
//! # Why this exists
//!
//! `ComposerNext`, `ComposerCaption` and `PostButton` have never been read off a phone, on any
//! build, so `tiktok_composer` refuses to publish everywhere. They live two screens inside the
//! composer, behind a picker that has to have images selected before it will move — which is
//! why nobody has dumped them by hand: getting there and dumping the screen are a race a person
//! loses.
//!
//! So the walk that the publish path uses is what takes the measurement.
//! [`riviu_core::tiktok_composer::reach_edit_step`] is the *same code*, with the publishing tail
//! not called — not a copy of it, which would drift and would quietly lose a verification.
//!
//! # It cannot publish, structurally
//!
//! It calls `reach_edit_step`, which never taps the edit step's Next and never looks for a Post
//! button. There is no argument to this program that makes it post, and no sequence of failures
//! inside it that does either. Everything it touches is reversible: Back walks out and nothing
//! has left the phone.
//!
//! # What `--album` should be
//!
//! The name of an album that already exists in that phone's picker and holds at least
//! `--images` photos. For a campaign that has been transferred, it is the `importId`. For a
//! first measuring run it can be any album the phone already has — the point is to reach the
//! edit step, not to reach a particular set of images.

use std::sync::atomic::AtomicBool;

use riviu_android_driver::AndroidDriver;
use riviu_core::driver::{DeviceDriver, UiSession};
use riviu_core::tiktok_composer::{
    reach_edit_step, CarouselRequest, Composer, ComposerPlan, Screen, REQUIRED_TO_PUBLISH,
};
use riviu_core::tiktok_labels::{controls_for, TikTokControl};

#[path = "common/mod.rs"]
mod common;

fn say(line: &str) {
    use std::io::Write;
    println!("{line}");
    let _ = std::io::stdout().flush();
}

/// Every `content-desc`, `text` and `resource-id` on the screen, with its bounds.
///
/// Bounds included because two of the three controls being hunted are buttons whose strings
/// may turn out to be absent — the delete row on the own-post page was exactly that — and a
/// rectangle is the difference between "measure it later" and "there is nothing to measure".
fn dump_elements(source: &str) {
    for chunk in source.split('<').skip(1) {
        let Some(end) = chunk.find('>') else { continue };
        let attributes = &chunk[..end];
        let read = |key: &str| -> String {
            let needle = format!(" {key}=\"");
            let Some(start) = attributes.find(&needle) else {
                return String::new();
            };
            let rest = &attributes[start + needle.len()..];
            rest.find('"')
                .map(|to| rest[..to].to_string())
                .unwrap_or_default()
        };
        let class = read("class");
        if class.is_empty() || class == "hierarchy" {
            continue;
        }
        let desc = read("content-desc");
        let text = read("text");
        let id = read("resource-id");
        if desc.is_empty() && text.is_empty() && id.is_empty() {
            continue;
        }
        say(&format!(
            "  {class:<44} desc={desc:?} text={text:?} id={id:?} bounds={} clickable={}",
            read("bounds"),
            read("clickable")
        ));
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(serial) = args.first().filter(|arg| !arg.starts_with("--")) else {
        say("usage: composer_scout <serial> --album \"<tên album>\" [--images 3]");
        return Ok(());
    };
    // The value after a flag, unless that value is itself a flag — `--images --album x` used
    // to read `--album` as the count and then default silently.
    let value = |flag: &str| -> Option<String> {
        args.iter()
            .position(|arg| arg == flag)
            .and_then(|at| args.get(at + 1))
            .filter(|value| !value.starts_with("--"))
            .cloned()
    };
    let Some(album) = value("--album") else {
        say("--album is required: the name of an album this phone's picker already shows");
        return Ok(());
    };
    // **A value this cannot read is refused, not defaulted.** `--images abc`, `--images -1`
    // and `--images --album …` all became three, which is the wrong number silently — and the
    // number decides how many cells get tapped on a real phone.
    let images: usize = match value("--images") {
        None => 3,
        Some(raw) => match raw.parse::<usize>() {
            Ok(count) if (1..=12).contains(&count) => count,
            _ => {
                say(&format!(
                    "--images {raw:?} không đọc được; cần một số từ 1 đến 12"
                ));
                return Ok(());
            }
        },
    };

    let driver = AndroidDriver::new(&common::repo_config())?;
    let package = driver.resolve_tiktok_package(serial).await?;
    let session = driver.open_session(serial).await?;
    let language = session.ui_language().await.unwrap_or_default();
    let version = session.app_version(&package).await.unwrap_or_default();
    say(&format!(
        "serial   {serial}\npackage  {package}\nlanguage {language}\nversion  {version}\nalbum    {album:?}\nimages   {images}\n"
    ));

    let Some(labels) = controls_for(&package, &language, &version) else {
        say("no measured label set for this (package, language) — run label_scout first");
        return Ok(());
    };
    say(&format!("labels   {}", labels.provenance()));

    // **The refusal is printed whether or not it fires**, because the list is the point of the
    // trip: it is what this run is going out to close.
    let still_missing: Vec<TikTokControl> = ComposerPlan::missing_to_publish(&labels);
    say(&format!(
        "\nchưa đo (cần {} nhãn để đăng được): {:?}",
        REQUIRED_TO_PUBLISH.len(),
        still_missing
    ));

    let plan = match ComposerPlan::resolve(&labels) {
        Ok(plan) => plan,
        Err(refusal) => {
            say(&format!("\nkhông đi được tới bước chỉnh sửa: {refusal}"));
            return Ok(());
        }
    };

    let (width, height) = riviu_core::screen::measured_screen_size(&session).await?;
    let Some(screen) = Screen::new(width, height) else {
        say(&format!("screen {width}x{height} is not a screen"));
        return Ok(());
    };
    say(&format!("screen   {width}x{height}\n"));

    driver.launch_app(serial, &package).await?;
    tokio::time::sleep(std::time::Duration::from_secs(4)).await;

    let request = CarouselRequest {
        album: &album,
        images,
        // Never typed: `reach_edit_step` stops before the screen the caption lives on. Present
        // because the request describes a carousel, and a measuring run should carry the same
        // shape a real one does.
        caption: "",
        screen,
    };
    let stop = AtomicBool::new(false);
    let mut composer = Composer::new(&session, plan, |element: &riviu_core::ElementBox| {
        element.centre()
    });

    let verdict = reach_edit_step(&mut composer, &request, &stop).await;
    match &verdict {
        Ok(reached) => say(&format!("\nđi tới: {reached:?} — {}", reached.reason())),
        Err(error) => say(&format!("\nlỗi giữa đường: {error}")),
    }

    // **Dump before backing out, whatever happened.** Even a walk that stopped early left the
    // phone on a screen worth reading — a picker that would not arm is exactly the thing the
    // next measurement needs to see.
    say("\n--- màn hình đang hiện ---");
    match session.agent().source().await {
        Ok(source) => {
            let path = std::path::Path::new("target").join("composer-scout.xml");
            let _ = std::fs::create_dir_all("target");
            let _ = std::fs::write(&path, &source);
            dump_elements(&source);
            say(&format!("\n(XML đầy đủ: {})", path.display()));
        }
        Err(error) => say(&format!("  (không đọc được hierarchy: {error})")),
    }

    // The caller owns the walk-back — see `reach_edit_step`.
    let out = composer.leave().await;
    say(&format!(
        "\nlùi về feed: {}",
        if out {
            "xong"
        } else {
            "CHƯA — kiểm tra máy"
        }
    ));
    verdict?;
    // **Not backing out is a failure of this run**, even when the walk itself went fine. It
    // used to print the warning and exit 0, so a script saw success while the phone sat on the
    // edit step with a carousel selected — and the next thing to run started from there.
    anyhow::ensure!(
        out,
        "không lùi được về feed: máy còn đang ở trong composer với ảnh đã chọn. Kiểm tra tay          trước khi chạy tiếp bất cứ thứ gì trên máy này."
    );
    Ok(())
}
