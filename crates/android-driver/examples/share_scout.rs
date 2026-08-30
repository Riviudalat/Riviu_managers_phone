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

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(serial) = args.first().filter(|arg| !arg.starts_with("--")) else {
        say("usage: share_scout <serial> [--open <x>,<y>] [--capture]");
        anyhow::bail!("thiếu serial");
    };
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

    // Stage 1 — the profile grid. ProfileTab is measured on every set this fleet runs.
    let Some(profile) = labels.label(TikTokControl::ProfileTab) else {
        say("ProfileTab chưa đo trên bộ nhãn này — không có đường vào profile");
        anyhow::bail!("ProfileTab chưa đo");
    };
    let Some(tab) = session.locate(profile.to_query()).await? else {
        say("không thấy tab Profile trên màn hình hiện tại");
        anyhow::bail!("không thấy tab Profile");
    };
    session.tap(tab.centre()).await?;
    tokio::time::sleep(std::time::Duration::from_secs(3)).await;
    say("\n--- màn profile (tìm ô bài MỚI NHẤT trên lưới, ghi bounds cho --open) ---");
    let grid = session.agent().source().await?;
    let _ = std::fs::create_dir_all("target");
    let _ = std::fs::write("target/share-scout-profile.xml", &grid);
    dump_elements(&grid);
    say("\n(XML đầy đủ: target/share-scout-profile.xml)");

    let mut capture_outcome: Option<String> = None;
    if let Some(point) = open {
        // Stage 2 — the post page.
        session.tap(point).await?;
        tokio::time::sleep(std::time::Duration::from_secs(3)).await;
        say("\n--- trang bài (xác nhận share control + so với nhãn Share đo trên feed) ---");
        let post_page = session.agent().source().await?;
        let _ = std::fs::write("target/share-scout-post.xml", &post_page);
        dump_elements(&post_page);
        say("\n(XML đầy đủ: target/share-scout-post.xml)");

        if capture {
            // Stage 3 — the real capture, exactly the code the publish path will call.
            say("\n--- capture_post_link (production code, không phải bản chép) ---");
            let outcome = capture_post_link(&session, &labels).await;
            match outcome.link() {
                Some(link) => say(&format!("LINK: {link}")),
                None => say(&format!("không lấy được link: {}", outcome.reason())),
            }
            capture_outcome = Some(outcome.reason());
            say("\n--- màn hình sau capture ---");
            let after = session.agent().source().await?;
            let _ = std::fs::write("target/share-scout-after.xml", &after);
            dump_elements(&after);
            say("\n(XML đầy đủ: target/share-scout-after.xml)");
        }
    }

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
    Ok(())
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
}
