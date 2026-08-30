//! Walk from the feed to a post's own page and read its link — so M7's three unknowns can
//! be **measured** instead of guessed.
//!
//! ```text
//! cargo run -p riviu-android-driver --example share_scout -- <serial> [--open <x>,<y>] [--capture]
//! ```
//!
//! # Why this exists
//!
//! `capture_post_link` is finished and tested, and nothing calls it, because three things
//! were never read off a phone (tiktok_share.rs names them): the route from a just-published
//! carousel back to its own post page, whether the post page's share affordance matches the
//! `Share` label measured on the feed, and the share sheet's copy row. This tool goes and
//! looks, one staged hop per flag, dumping every screen it stands on.
//!
//! # Staged on purpose
//!
//! With no flags it only opens the Profile tab and dumps the grid — the operator reads the
//! newest tile's rectangle out of the dump. `--open x,y` taps exactly that rectangle and
//! dumps the post page. `--capture` (requires `--open`) then runs the real
//! `capture_post_link` — the same code the publish path will call — and prints what came
//! back, link or refusal. Each stage is one new tap beyond the previous run, so a wrong
//! guess is caught by a dump rather than compounded.
//!
//! # It cannot publish or delete anything
//!
//! Every tap here lands on navigation: a tab, a grid tile, Share, a copy row. The share
//! sheet's rows are read before anything is tapped, and `capture_post_link` refuses on
//! ambiguity rather than choosing. Back walks out at the end whatever happened, and a walk
//! that cannot get back to the feed exits non-zero and says so.

use riviu_android_driver::AndroidDriver;
use riviu_core::driver::{DeviceDriver, UiSession};
use riviu_core::tiktok_labels::{controls_for, TikTokControl};
use riviu_core::tiktok_share::capture_post_link;
use riviu_core::TapPoint;

#[path = "common/mod.rs"]
mod common;

fn say(line: &str) {
    use std::io::Write;
    println!("{line}");
    let _ = std::io::stdout().flush();
}

/// Every labelled or clickable node with its bounds — the harvest format shared with
/// `composer_scout`, copied rather than imported because examples do not link each other.
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
        if desc.is_empty() && text.is_empty() && id.is_empty() && read("clickable") != "true" {
            continue;
        }
        say(&format!(
            "  {class:<44} desc={desc:?} text={text:?} id={id:?} bounds={} clickable={}",
            read("bounds"),
            read("clickable")
        ));
    }
}

/// `--open 540,760` → a tap point, or the complaint the operator needs.
///
/// The same three-way discipline the other scouts settled on: absent is a valid stage,
/// present-but-unreadable is a refusal, and repeated is a refusal — this number is where a
/// finger lands on a real phone, and a guessed one taps a stranger's post.
fn open_point(args: &[String]) -> Result<Option<TapPoint>, String> {
    let mut occurrences = args.iter().enumerate().filter(|(_, arg)| *arg == "--open");
    let Some((at, _)) = occurrences.next() else {
        return Ok(None);
    };
    if occurrences.next().is_some() {
        return Err("--open xuất hiện nhiều lần — không đoán lần nào là thật".to_string());
    }
    let Some(raw) = args.get(at + 1).filter(|value| !value.starts_with("--")) else {
        return Err("--open cần \"x,y\" đi ngay sau nó (lấy từ dump lượt trước)".to_string());
    };
    let Some((x, y)) = raw.split_once(',') else {
        return Err(format!("--open {raw:?} không đọc được; cần dạng \"x,y\""));
    };
    match (x.trim().parse::<f64>(), y.trim().parse::<f64>()) {
        (Ok(x), Ok(y)) if x >= 0.0 && y >= 0.0 => Ok(Some(TapPoint { x, y })),
        _ => Err(format!(
            "--open {raw:?} không đọc được; cần hai số không âm"
        )),
    }
}

/// A boolean switch: off, on, or passed twice — which is refused like every other repeat.
fn switch(args: &[String], flag: &str) -> Result<bool, String> {
    match args.iter().filter(|arg| *arg == flag).count() {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(format!(
            "{flag} xuất hiện nhiều lần — không đoán lần nào là thật"
        )),
    }
}

/// Every flag this tool defines. A `--word` outside this list is refused.
const KNOWN_FLAGS: &[&str] = &["--open", "--capture"];

/// Refuse a flag this tool does not define, instead of running a different stage.
///
/// **A typo silently downgraded the run.** `--captuer` is not `--capture`, so the switch read
/// `false`, the staging guard had nothing to guard, and the tool opened a post page, skipped
/// the capture the operator asked for, walked out and exited 0 — reporting a measurement that
/// never happened. The same shape `--images --album x` had before it was made to refuse: an
/// unreadable instruction must not become a different, quieter one.
///
/// Values are not flags: every parser here already refuses a `--`-shaped value, so anything
/// starting with `--` is a flag position and must be one this tool knows.
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
        "không hiểu cờ {unknown:?}; tool này chỉ có {KNOWN_FLAGS:?} — gõ sai một cờ là chạy \
         một nấc khác mà vẫn báo thành công"
    ))
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(serial) = args.first().filter(|arg| !arg.starts_with("--")) else {
        say("usage: share_scout <serial> [--open <x>,<y>] [--capture]");
        anyhow::bail!("thiếu serial");
    };
    if let Err(complaint) = refuse_unknown_flags(&args) {
        say(&complaint);
        anyhow::bail!("{complaint}");
    }
    let open = match open_point(&args) {
        Ok(open) => open,
        Err(complaint) => {
            say(&complaint);
            anyhow::bail!("{complaint}");
        }
    };
    let capture = match switch(&args, "--capture") {
        Ok(on) => on,
        Err(complaint) => {
            say(&complaint);
            anyhow::bail!("{complaint}");
        }
    };
    if capture && open.is_none() {
        let complaint = "--capture cần --open đi kèm: link chỉ có nghĩa khi đứng trên trang bài";
        say(complaint);
        anyhow::bail!("{complaint}");
    }

    let driver = AndroidDriver::new(&common::repo_config())?;
    let package = driver.resolve_tiktok_package(serial).await?;
    let session = driver.open_session(serial).await?;
    let language = session.ui_language().await.unwrap_or_default();
    let version = session.app_version(&package).await.unwrap_or_default();
    say(&format!(
        "serial   {serial}\npackage  {package}\nlanguage {language}\nversion  {version}\n"
    ));
    let Some(labels) = controls_for(&package, &language, &version) else {
        say("no measured label set for this (package, language) — run label_scout first");
        anyhow::bail!("bộ nhãn chưa đo");
    };
    say(&format!("labels   {}", labels.provenance()));

    driver.launch_app(serial, &package).await?;
    tokio::time::sleep(std::time::Duration::from_secs(4)).await;

    // **Every `?` past this line used to skip the walk-out.** The stages tap into a post
    // page and read the hierarchy three times; a read that failed after the tap landed
    // exited straight out with the phone standing on the post page — the exact stale screen
    // the walk-out exists to prevent, and the final `ensure!` never ran to say so. Owning
    // the fallible region the way `composer_scout` owns its walk-back is the fix: run the
    // stages, walk out **whatever** they did, then report.
    let staged = run_the_stages(&session, labels, open, capture).await;
    let capture_outcome = match &staged {
        Ok(reason) => reason.clone(),
        Err(error) => {
            say(&format!("\nlỗi giữa đường: {error:#}"));
            None
        }
    };

    // Walk out. A measuring phone left inside a post page or a share sheet is the stale
    // screen every next command trips over, so not getting back is a failure of THIS run.
    let feed = labels
        .label(TikTokControl::FeedTab)
        .or_else(|| labels.label(TikTokControl::HomeTab));
    let mut out = false;
    for _ in 0..4 {
        if let Some(anchor) = feed {
            if session
                .locate(anchor.to_query())
                .await
                .ok()
                .flatten()
                .is_some()
            {
                out = true;
                break;
            }
        }
        if session.back().await.is_err() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(900)).await;
    }
    say(&format!(
        "\nlùi về feed: {}",
        if out {
            "xong"
        } else {
            "CHƯA — kiểm tra máy"
        }
    ));
    if let Some(reason) = capture_outcome {
        say(&format!("\nkết luận capture: {reason}"));
    }
    anyhow::ensure!(
        out,
        "không lùi được về feed: máy còn đứng trong profile/trang bài/share sheet. Kiểm tra \
         tay trước khi chạy tiếp bất cứ thứ gì trên máy này."
    );
    // The stages' own failure, reported after the phone is safe — never instead of it.
    staged?;
    Ok(())
}

/// The stages that touch the phone, so the walk-out can own every exit from them.
///
/// Returns the capture's reason when stage three ran. Split out for the reason
/// `post_into_drawer` and `read_the_gallery_candidates` are split out in their files: a `?`
/// in here must not be able to jump over the Back walk that puts the phone back on the feed.
async fn run_the_stages(
    session: &riviu_android_driver::AndroidUiSession,
    labels: riviu_core::tiktok_labels::TikTokControls,
    open: Option<TapPoint>,
    capture: bool,
) -> anyhow::Result<Option<String>> {
    // Stage 1 — the profile grid. ProfileTab is measured on every set this fleet runs.
    let Some(profile) = labels.label(TikTokControl::ProfileTab) else {
        anyhow::bail!("ProfileTab chưa đo trên bộ nhãn này — không có đường vào profile");
    };
    let Some(tab) = session.locate(profile.to_query()).await? else {
        anyhow::bail!("không thấy tab Profile trên màn hình hiện tại");
    };
    session.tap(tab.centre()).await?;
    tokio::time::sleep(std::time::Duration::from_secs(3)).await;
    say("\n--- màn profile (tìm ô bài MỚI NHẤT trên lưới, ghi bounds cho --open) ---");
    let grid = session.agent().source().await?;
    let _ = std::fs::create_dir_all("target");
    let _ = std::fs::write("target/share-scout-profile.xml", &grid);
    dump_elements(&grid);
    say("\n(XML đầy đủ: target/share-scout-profile.xml)");

    let Some(point) = open else {
        return Ok(None);
    };

    // Stage 2 — the post page.
    session.tap(point).await?;
    tokio::time::sleep(std::time::Duration::from_secs(3)).await;
    say("\n--- trang bài (xác nhận share control + so với nhãn Share đo trên feed) ---");
    let post_page = session.agent().source().await?;
    let _ = std::fs::write("target/share-scout-post.xml", &post_page);
    dump_elements(&post_page);
    say("\n(XML đầy đủ: target/share-scout-post.xml)");

    if !capture {
        return Ok(None);
    }

    // Stage 3 — the real capture, exactly the code the publish path will call.
    say("\n--- capture_post_link (production code, không phải bản chép) ---");
    let outcome = capture_post_link(session, &labels).await;
    match outcome.link() {
        Some(link) => say(&format!("LINK: {link}")),
        None => say(&format!("không lấy được link: {}", outcome.reason())),
    }
    let reason = outcome.reason();
    say("\n--- màn hình sau capture ---");
    let after = session.agent().source().await?;
    let _ = std::fs::write("target/share-scout-after.xml", &after);
    dump_elements(&after);
    say("\n(XML đầy đủ: target/share-scout-after.xml)");
    Ok(Some(reason))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(args: &[&str]) -> Vec<String> {
        args.iter().map(|arg| arg.to_string()).collect()
    }

    /// **A tap point is never guessed.** Absent is a valid stage; unreadable and repeated
    /// both refuse — this number is where a finger lands on a phone holding a real account.
    fn point_of(args: &[&str]) -> (f64, f64) {
        // `TapPoint` carries no `PartialEq` (it is a wire type, not a value type), so the
        // assertions read the fields instead of comparing wholes.
        let point = open_point(&line(args))
            .expect("readable")
            .expect("a point was asked for");
        (point.x, point.y)
    }

    #[test]
    fn an_unreadable_open_point_is_refused_not_defaulted() {
        assert!(
            open_point(&line(&["SN"])).expect("readable").is_none(),
            "stage one has no tap"
        );
        assert_eq!(point_of(&["SN", "--open", "540,760"]), (540.0, 760.0));
        assert_eq!(
            point_of(&["SN", "--open", " 540 , 760 "]),
            (540.0, 760.0),
            "the operator pastes from a dump; whitespace is not a refusal"
        );
        for bad in ["540", "540;760", "abc,760", "540,-1", ""] {
            assert!(
                open_point(&line(&["SN", "--open", bad])).is_err(),
                "--open {bad:?} must refuse, not tap a guess"
            );
        }
        assert!(
            open_point(&line(&["SN", "--open", "--capture"])).is_err(),
            "a flag is not a coordinate"
        );
        assert!(
            open_point(&line(&["SN", "--open", "1,2", "--open", "3,4"])).is_err(),
            "two --open are two instructions; neither is chosen"
        );
    }

    /// The capture stage cannot exist without a post page under it.
    #[test]
    fn capture_without_open_is_refused_in_parsing_terms() {
        // The refusal itself lives in `main` (it needs both answers); what the parser
        // guarantees is that the two flags read independently and repeats refuse.
        assert_eq!(switch(&line(&["SN", "--capture"]), "--capture"), Ok(true));
        assert!(switch(&line(&["SN", "--capture", "--capture"]), "--capture").is_err());
    }

    /// **A misspelled stage flag refuses; it does not quietly become a different stage.**
    ///
    /// `--captuer` read as "capture not asked for", so the run opened a post page, skipped
    /// the capture, walked out and exited 0 — a measurement reported that never happened.
    /// The staging guard only guards flags spelled the way this tool spells them.
    #[test]
    fn a_misspelled_flag_is_refused_rather_than_silently_skipping_its_stage() {
        assert_eq!(refuse_unknown_flags(&line(&["SN"])), Ok(()));
        assert_eq!(
            refuse_unknown_flags(&line(&["SN", "--open", "540,760", "--capture"])),
            Ok(())
        );
        for typo in ["--captuer", "--Capture", "--opne", "--visit-caption-step"] {
            let refused = refuse_unknown_flags(&line(&["SN", "--open", "1,2", typo]));
            assert!(
                refused.as_ref().is_err_and(|why| why.contains(typo)),
                "{typo} must be refused by name, not ignored: {refused:?}"
            );
        }
        // A flag's *value* is not a flag, and must not be mistaken for one.
        assert_eq!(
            refuse_unknown_flags(&line(&["SN", "--open", "540,760"])),
            Ok(())
        );
    }
}
