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
    reach_edit_step, CarouselRequest, Composer, ComposerPlan, ComposerVerdict, Screen,
    REQUIRED_TO_PUBLISH,
};
use riviu_core::tiktok_labels::{controls_for, TikTokControl, TikTokControls};

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

/// What a command line says about one flag.
#[derive(Debug, Clone, PartialEq)]
enum Flag {
    /// The flag is not there at all — the caller did not ask for this.
    Absent,
    /// The flag is there and its value cannot be used: nothing follows it, or what follows is
    /// another flag. **Not the same as absent**, and the difference is the whole point of this
    /// enum: `--images --album x` collapsing into "absent" is how the silent default came back.
    Unusable,
    /// The flag is there more than once. Two occurrences are two instructions, and picking
    /// either silently is the same guess-shaped hole `Unusable` closes: first-wins let
    /// `--images 7 --images --album x` tap seven cells while the malformed second `--images`
    /// — the last thing the operator typed — was never read at all.
    Repeated,
    Value(String),
}

fn flag_value(args: &[String], flag: &str) -> Flag {
    let mut occurrences = args.iter().enumerate().filter(|(_, arg)| *arg == flag);
    let Some((at, _)) = occurrences.next() else {
        return Flag::Absent;
    };
    if occurrences.next().is_some() {
        return Flag::Repeated;
    }
    match args.get(at + 1) {
        Some(value) if !value.starts_with("--") => Flag::Value(value.clone()),
        _ => Flag::Unusable,
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

/// How many cells the picker should tap, or what to tell the operator instead.
///
/// # Absent is three; anything else unreadable is a refusal
///
/// This number decides how many photos get selected on a real phone, and the operator does not
/// see it go wrong — the run just selects a different number and carries on. `--images abc`,
/// `--images -1`, `--images 0` and `--images --album x` all used to become **three**.
///
/// The first repair swapped the parse for one that skipped a flag-shaped value, which turned
/// `--images --album x` back into "absent" and defaulted it to three again — the same silent
/// three by a different road. So the two states are kept apart: nothing asked for (three), and
/// something asked for that cannot be read (refuse).
fn how_many_images(args: &[String]) -> Result<usize, String> {
    match flag_value(args, "--images") {
        Flag::Absent => Ok(3),
        Flag::Unusable => Err("--images cần một số từ 1 đến 12 đi ngay sau nó".to_string()),
        Flag::Repeated => {
            Err("--images xuất hiện nhiều lần — không đoán lần nào là thật".to_string())
        }
        Flag::Value(raw) => match raw.parse::<usize>() {
            Ok(count) if (1..=12).contains(&count) => Ok(count),
            _ => Err(format!(
                "--images {raw:?} không đọc được; cần một số từ 1 đến 12"
            )),
        },
    }
}

/// Print the refusal for the operator, then leave with a non-zero exit for the script.
///
/// Every arm here used to `return Ok(())` — a refused command line exited 0, so a
/// multi-serial loop sailed past a typo and reported a measuring run that never ran.
/// The same shape §9.129 fixed for a stranded phone, one layer earlier.
fn refuse_usage(complaint: &str) -> anyhow::Error {
    say(complaint);
    anyhow::anyhow!("dòng lệnh bị từ chối — xem thông báo ở trên")
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(serial) = args.first().filter(|arg| !arg.starts_with("--")) else {
        return Err(refuse_usage(
            "usage: composer_scout <serial> --album \"<tên album>\" [--images 3] [--visit-caption-step]",
        ));
    };
    let album = match flag_value(&args, "--album") {
        Flag::Value(album) => album,
        Flag::Absent => {
            return Err(refuse_usage(
                "--album is required: the name of an album this phone's picker already shows",
            ));
        }
        Flag::Unusable => {
            return Err(refuse_usage(
                "--album needs a name after it, not another flag",
            ));
        }
        Flag::Repeated => {
            return Err(refuse_usage(
                "--album appears more than once; pass it once — neither occurrence is guessed at",
            ));
        }
    };
    let images = match how_many_images(&args) {
        Ok(images) => images,
        Err(complaint) => return Err(refuse_usage(&complaint)),
    };
    let visit_caption_step = match switch(&args, "--visit-caption-step") {
        Ok(on) => on,
        Err(complaint) => return Err(refuse_usage(&complaint)),
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

    // **One screen further, only when asked, and only ever to look.** The caption step is
    // one tap from Post, which is exactly why `reach_edit_step` refuses to own it — so this
    // block taps the one measured `ComposerNext` and nothing else, dumps what appears, and
    // hands straight back to the walk-out below. It types nothing; the dump is the harvest,
    // and `ComposerCaption`/`PostButton` get measured from the file it writes.
    if visit_caption_step {
        if matches!(&verdict, Ok(ComposerVerdict::Stopped)) {
            // `stop` is never set in this tool, so `Stopped` here can only be the measuring
            // terminus: standing on the edit step, before its Next.
            if let Err(error) = peek_at_the_caption_step(&session, labels).await {
                say(&format!("\n--visit-caption-step lỗi: {error:#}"));
            }
        } else {
            say("\n--visit-caption-step: chưa đứng ở bước chỉnh sửa nên không đi tiếp");
        }
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

/// Tap the edit step's measured Next, dump the caption screen, and touch nothing else.
///
/// Errors are the caller's to print, not to propagate out of `main` — the walk-back below
/// this must run whatever happened here, or the phone is left one tap from Post.
async fn peek_at_the_caption_step(
    session: &riviu_android_driver::AndroidUiSession,
    labels: TikTokControls,
) -> anyhow::Result<()> {
    let Some(next) = labels.label(TikTokControl::ComposerNext) else {
        anyhow::bail!(
            "build này chưa đo ComposerNext — đo nó từ dump bước chỉnh sửa ở trên trước, \
             rồi chạy lại với cờ này"
        );
    };
    let Some(element) = session.locate(next.to_query()).await? else {
        anyhow::bail!("không thấy nút Next của bước chỉnh sửa trên màn hình");
    };
    session.tap(element.centre()).await?;
    tokio::time::sleep(std::time::Duration::from_secs(4)).await;
    say("\n--- màn hình caption (một tap trước Đăng — tool KHÔNG bấm gì ở đây) ---");
    let source = session.agent().source().await?;
    let path = std::path::Path::new("target").join("composer-caption.xml");
    let _ = std::fs::create_dir_all("target");
    let _ = std::fs::write(&path, &source);
    dump_elements(&source);
    say(&format!("\n(XML đầy đủ: {})", path.display()));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(args: &[&str]) -> Vec<String> {
        args.iter().map(|arg| arg.to_string()).collect()
    }

    /// The switch has three states and the third is a refusal, like every other repeat.
    #[test]
    fn a_repeated_switch_is_refused_not_first_wins() {
        assert_eq!(switch(&line(&["SN"]), "--visit-caption-step"), Ok(false));
        assert_eq!(
            switch(
                &line(&["SN", "--visit-caption-step"]),
                "--visit-caption-step"
            ),
            Ok(true)
        );
        assert!(switch(
            &line(&["SN", "--visit-caption-step", "--visit-caption-step"]),
            "--visit-caption-step"
        )
        .is_err());
    }

    /// **Nothing asked for is three; asked-for-and-unreadable is a refusal.**
    ///
    /// The number decides how many cells get tapped on a phone holding a real account, and a
    /// wrong one is invisible: the run selects four photos instead of eleven and prints nothing
    /// about it.
    #[test]
    fn an_unreadable_image_count_is_refused_rather_than_defaulted() {
        assert_eq!(how_many_images(&line(&["SN"])), Ok(3), "not asked for");
        assert_eq!(how_many_images(&line(&["SN", "--images", "7"])), Ok(7));
        for bad in ["abc", "-1", "0", "13", "3.5", ""] {
            assert!(
                how_many_images(&line(&["SN", "--images", bad])).is_err(),
                "--images {bad:?} must refuse, not silently mean three"
            );
        }
    }

    /// **A flag whose value is the next flag is unreadable, not absent.**
    ///
    /// This is the one the first repair got wrong. Skipping a flag-shaped value made
    /// `--images --album x` indistinguishable from not passing `--images` at all, so it
    /// defaulted to three — the exact behaviour the repair was written to remove.
    #[test]
    fn a_flag_swallowing_the_next_flag_is_not_the_same_as_an_absent_flag() {
        let swallowed = line(&["SN", "--images", "--album", "riviu-import-1"]);
        assert_eq!(flag_value(&swallowed, "--images"), Flag::Unusable);
        assert!(
            how_many_images(&swallowed).is_err(),
            "--images --album must refuse; defaulting to three is how this bug returns"
        );
        // And the album it swallowed is still read correctly, so the message the operator
        // gets is about the flag that is actually wrong.
        assert_eq!(
            flag_value(&swallowed, "--album"),
            Flag::Value("riviu-import-1".into())
        );
    }

    /// A trailing flag has nothing after it at all.
    #[test]
    fn a_flag_at_the_end_of_the_line_is_unusable() {
        assert_eq!(
            flag_value(&line(&["SN", "--album"]), "--album"),
            Flag::Unusable
        );
        assert_eq!(flag_value(&line(&["SN"]), "--album"), Flag::Absent);
    }

    /// **The same flag twice is two instructions, and neither is guessed at.**
    ///
    /// `flag_value` used to take the first occurrence, so `--images 7 --images --album x`
    /// selected seven cells while the malformed second `--images` — the last thing the
    /// operator typed — was silently ignored.
    #[test]
    fn a_repeated_flag_is_refused_not_first_wins() {
        let twice = line(&[
            "SN",
            "--images",
            "7",
            "--images",
            "--album",
            "riviu-import-1",
        ]);
        assert_eq!(flag_value(&twice, "--images"), Flag::Repeated);
        assert!(
            how_many_images(&twice).is_err(),
            "--images twice must refuse; taking the first is a silent guess"
        );
        assert_eq!(
            flag_value(&line(&["SN", "--album", "a", "--album", "b"]), "--album"),
            Flag::Repeated
        );
    }

    /// An album name is taken whole, including one that looks like a sentence.
    #[test]
    fn an_album_name_is_taken_as_written() {
        assert_eq!(
            flag_value(&line(&["SN", "--album", "Ảnh chụp màn hình"]), "--album"),
            Flag::Value("Ảnh chụp màn hình".into())
        );
    }
}
