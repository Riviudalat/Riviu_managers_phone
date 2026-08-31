//! Publish exactly ONE carousel on ONE phone, through the production chain — the M7 trip.
//!
//! ```text
//! cargo run -p riviu-android-driver --example m7_post_once -- <serial> --source <dir> --caption "<text>" [--yes]
//! ```
//!
//! # This is the one example that CAN post, and it says so at the top
//!
//! Every other instrument in this directory is structurally unable to publish, and that is
//! their whole design. M7 — measuring the route from a just-published carousel back to its
//! own post page, the share control there, and the copy-link row — cannot exist without one
//! real, undeletable post on a real account. This tool makes exactly that post, once,
//! through the same production code a campaign uses on the phone side:
//! `stage → prepare → import` (the transfer that builds the campaign's own album) and then
//! [`riviu_core::tiktok_composer::publish_carousel`] — not a copy of the walk, the walk.
//!
//! # Refusal is the default
//!
//! Without `--yes` it does everything up to and including printing the full plan — files
//! with their hashes, the caption verbatim, the album id the import will create — and then
//! refuses with a non-zero exit. `--yes` is the operator's signature on that exact plan.
//!
//! # What it deliberately does not do
//!
//! No campaign row, no assignment, no outbox: the desktop DB layer is `pub(crate)` in the
//! app and stays exercised by its own tests and by the first real campaign. This trip's job
//! is the phone and the post; wiring the link capture into the campaign path is the step
//! AFTER the route it unlocks has been measured (`share_scout`).

use std::path::PathBuf;
use std::sync::atomic::AtomicBool;

use riviu_android_driver::AndroidDriver;
use riviu_core::driver::{DeviceDriver, UiSession};
use riviu_core::tiktok_composer::{publish_carousel, CarouselRequest, ComposerPlan, Screen};
use riviu_core::tiktok_labels::controls_for;

#[path = "common/mod.rs"]
mod common;

fn say(line: &str) {
    use std::io::Write;
    println!("{line}");
    let _ = std::io::stdout().flush();
}

/// Every flag this tool defines. A `--word` outside this list is refused — a typo must not
/// quietly run a different trip, least of all in the one tool that can post.
const KNOWN_FLAGS: &[&str] = &["--source", "--caption", "--yes"];

fn refuse_unknown_flags(args: &[String]) -> Result<(), String> {
    let unknown: Vec<&str> = args
        .iter()
        .map(String::as_str)
        .filter(|arg| arg.starts_with("--") && !KNOWN_FLAGS.contains(arg))
        .collect();
    if unknown.is_empty() {
        return Ok(());
    }
    Err(format!(
        "không hiểu cờ {unknown:?}; tool này chỉ có {KNOWN_FLAGS:?}"
    ))
}

/// One required value flag: present exactly once, with a usable value.
fn required_value(args: &[String], flag: &str) -> Result<String, String> {
    let mut occurrences = args.iter().enumerate().filter(|(_, arg)| *arg == flag);
    let Some((at, _)) = occurrences.next() else {
        return Err(format!("{flag} là bắt buộc"));
    };
    if occurrences.next().is_some() {
        return Err(format!(
            "{flag} xuất hiện nhiều lần — không đoán lần nào là thật"
        ));
    }
    match args.get(at + 1) {
        Some(value) if !value.starts_with("--") && !value.trim().is_empty() => Ok(value.clone()),
        _ => Err(format!("{flag} cần một giá trị đi ngay sau nó")),
    }
}

/// A boolean switch: off, on, or passed twice — refused like every other repeat.
fn switch(args: &[String], flag: &str) -> Result<bool, String> {
    match args.iter().filter(|arg| *arg == flag).count() {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(format!(
            "{flag} xuất hiện nhiều lần — không đoán lần nào là thật"
        )),
    }
}

fn refuse_usage(complaint: &str) -> anyhow::Error {
    say(complaint);
    anyhow::anyhow!("dòng lệnh bị từ chối — xem thông báo ở trên")
}

const CAMPAIGN: &str = "m7-route-measure";

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(serial) = args.first().filter(|arg| !arg.starts_with("--")) else {
        return Err(refuse_usage(
            "usage: m7_post_once <serial> --source <dir> --caption \"<text>\" [--yes]",
        ));
    };
    if let Err(complaint) = refuse_unknown_flags(&args) {
        return Err(refuse_usage(&complaint));
    }
    let source = match required_value(&args, "--source") {
        Ok(value) => PathBuf::from(value),
        Err(complaint) => return Err(refuse_usage(&complaint)),
    };
    let caption = match required_value(&args, "--caption") {
        Ok(value) => value,
        Err(complaint) => return Err(refuse_usage(&complaint)),
    };
    let yes = match switch(&args, "--yes") {
        Ok(on) => on,
        Err(complaint) => return Err(refuse_usage(&complaint)),
    };

    // The files, hashed, before anything talks to a phone: the plan the operator signs is
    // the plan that ships, byte for byte.
    anyhow::ensure!(
        source.is_dir(),
        "--source không phải thư mục: {}",
        source.display()
    );
    let mut files: Vec<PathBuf> = std::fs::read_dir(&source)?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| path.is_file())
        .collect();
    files.sort();
    anyhow::ensure!(
        (1..=12).contains(&files.len()),
        "cần 1..=12 ảnh trong --source; thấy {}",
        files.len()
    );
    say(&format!("KẾ HOẠCH ĐĂNG — một bài, một máy\n  máy      {serial}\n  album    (đặt sau khi stage — in ở dưới)\n  caption  {caption:?}"));
    for path in &files {
        let bytes = std::fs::read(path)?;
        let sha = riviu_core::frame_sha256(&bytes);
        say(&format!(
            "  ảnh      {} ({} byte, sha256 {})",
            path.file_name().and_then(|n| n.to_str()).unwrap_or("?"),
            bytes.len(),
            &sha[..12]
        ));
    }

    let driver = AndroidDriver::new(&common::repo_config())?;
    let package = driver.resolve_tiktok_package(serial).await?;
    let session = driver.open_session(serial).await?;
    let language = session.ui_language().await.unwrap_or_default();
    let version = session.app_version(&package).await.unwrap_or_default();
    say(&format!("  package  {package} / {language} / {version}"));
    let Some(labels) = controls_for(&package, &language, &version) else {
        anyhow::bail!("bộ nhãn chưa đo cho ({package}, {language}, {version}) — không đăng");
    };
    let plan = ComposerPlan::resolve(&labels)
        .map_err(|refusal| anyhow::anyhow!("composer chưa đo đủ: {refusal}"))?;
    anyhow::ensure!(
        plan.can_publish(),
        "build này không có đuôi đăng đo đủ — không đăng"
    );
    let (width, height) = riviu_core::screen::measured_screen_size(&session).await?;
    let Some(screen) = Screen::new(width, height) else {
        anyhow::bail!("kích thước màn hình đọc ra không dùng được: {width}x{height}");
    };

    if !yes {
        say("\nCHƯA có --yes: kế hoạch in ở trên, KHÔNG đăng gì. Chạy lại kèm --yes để ký.");
        anyhow::bail!("từ chối theo mặc định — thiếu --yes");
    }

    // The production transfer: stage the files, prove them, import into the album the walk
    // will select by name. Each step's own JSON is printed — it is the receipt.
    let staged = driver
        .stage_publish_media(serial, "", CAMPAIGN, &source)
        .await?;
    say(&format!("\nstage    {staged}"));
    let sha = staged
        .get("manifestSha256")
        .and_then(|value| value.as_str())
        .map(str::to_string)
        .ok_or_else(|| anyhow::anyhow!("stage không trả manifestSha256"))?;
    let prepared = driver.prepare_publish_media(serial, CAMPAIGN, &sha).await?;
    say(&format!("prepare  {prepared}"));
    let imported = driver.import_publish_media(serial, CAMPAIGN, &sha).await?;
    say(&format!("import   {imported}"));
    let album = imported
        .get("importId")
        .and_then(|value| value.as_str())
        .map(str::to_string)
        .ok_or_else(|| anyhow::anyhow!("import không trả importId"))?;
    say(&format!("album    {album}"));

    driver.launch_app(serial, &package).await?;
    tokio::time::sleep(std::time::Duration::from_secs(4)).await;

    let request = CarouselRequest {
        album: &album,
        images: files.len(),
        caption: &caption,
        screen,
    };
    let stop = AtomicBool::new(false);
    let verdict = publish_carousel(
        &session,
        plan,
        |element: &riviu_core::ElementBox| element.centre(),
        &request,
        &stop,
    )
    .await?;
    say(&format!("\nkết quả: {verdict:?} — {}", verdict.reason()));

    // Production hygiene either way: the album was this trip's scaffolding, and the post —
    // if there is one — lives on TikTok, not in the gallery. A failed cleanup is reported
    // and does not change the verdict.
    match driver.cleanup_publish_media(serial, &album).await {
        Ok(cleaned) => say(&format!("cleanup  {cleaned}")),
        Err(error) => say(&format!("cleanup  KHÔNG sạch: {error:#}")),
    }

    anyhow::ensure!(
        matches!(
            verdict,
            riviu_core::tiktok_composer::ComposerVerdict::Posted
        ),
        "chuyến M7 không kết thúc ở Posted: {}",
        verdict.reason()
    );
    say("\nM7: bài đã lên. Bước đo tiếp theo là share_scout trên chính máy này.");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(args: &[&str]) -> Vec<String> {
        args.iter().map(|arg| arg.to_string()).collect()
    }

    /// **The one tool that can post refuses everything it does not understand.**
    #[test]
    fn unknown_flags_and_repeats_are_refused() {
        assert!(refuse_unknown_flags(&line(&["SN", "--source", "d", "--caption", "c"])).is_ok());
        assert!(refuse_unknown_flags(&line(&["SN", "--yess"])).is_err());
        assert!(required_value(
            &line(&["SN", "--caption", "a", "--caption", "b"]),
            "--caption"
        )
        .is_err());
        assert!(required_value(&line(&["SN", "--caption", "--yes"]), "--caption").is_err());
        assert!(required_value(&line(&["SN"]), "--source").is_err());
        assert!(switch(&line(&["SN", "--yes", "--yes"]), "--yes").is_err());
        assert_eq!(
            switch(&line(&["SN"]), "--yes"),
            Ok(false),
            "refusal is the default"
        );
    }
}
