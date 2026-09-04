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
    enter_picker_for_measuring, reach_edit_step, reach_picker, reach_video_edit_step,
    CarouselRequest, Composer, ComposerPlan, ComposerVerdict, Screen, VideoPickerPlan,
    VideoRequest, REQUIRED_TO_PUBLISH,
};
use riviu_core::tiktok_labels::{controls_for, TikTokControl, TikTokControls};

#[path = "common/mod.rs"]
mod common;

fn say(line: &str) {
    use std::io::Write;
    println!("{line}");
    let _ = std::io::stdout().flush();
}

async fn force_stop_and_confirm(
    driver: &AndroidDriver,
    serial: &str,
    package: &str,
) -> anyhow::Result<()> {
    let first = driver.terminate_app(serial, package).await?;
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    let mut after = driver.inspect_app_process(serial, package).await?;
    if after.running {
        // Observed on the video-editor scout: the first force-stop proved PID 13794 absent,
        // then the still-open automation session caused TikTok to appear as PID 17793. A
        // second force-stop plus delayed readback closes that lifecycle race.
        driver.terminate_app(serial, package).await?;
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        after = driver.inspect_app_process(serial, package).await?;
    }
    anyhow::ensure!(
        !after.running,
        "{package} tự chạy lại sau force-stop; PID hiện tại {:?}",
        after.pid
    );
    say(&format!(
        "\nđã force-stop TikTok và kiểm tra PID vắng (PID cũ {:?}) — không Post",
        first.old_pid
    ));
    Ok(())
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
/// Every flag this tool defines. A `--word` outside this list is refused.
const KNOWN_FLAGS: &[&str] = &[
    "--album",
    "--images",
    "--video",
    "--visit-caption-step",
    "--dump-sound-picker",
    "--sound-entry-id",
    "--peek-multi-select",
    "--dump-picker",
    "--dump-exit-menu",
];

/// Refuse a flag this tool does not define, instead of running a different stage.
///
/// A misspelled `--visit-caption-step` reads as "not asked for", so the run stops at the edit
/// step and exits 0 — reporting a measurement the operator asked for and did not get. The
/// same silent-downgrade shape `--images --album x` had. Values never start with `--` (every
/// parser here refuses a `--`-shaped value), so anything that does is a flag position.
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

fn refuse_usage(complaint: &str) -> anyhow::Error {
    say(complaint);
    anyhow::anyhow!("dòng lệnh bị từ chối — xem thông báo ở trên")
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(serial) = args.first().filter(|arg| !arg.starts_with("--")) else {
        return Err(refuse_usage(
            "usage: composer_scout <serial> --album \"<tên album>\" [--images 3 | --video] [--visit-caption-step]",
        ));
    };
    if let Err(complaint) = refuse_unknown_flags(&args) {
        return Err(refuse_usage(&complaint));
    }
    // Parsed here, demanded later: the album is what the walk selects, and `--dump-picker`
    // stops before any album exists to select — see below.
    let album = match flag_value(&args, "--album") {
        Flag::Value(album) => Some(album),
        Flag::Absent => None,
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
    let video = match switch(&args, "--video") {
        Ok(on) => on,
        Err(complaint) => return Err(refuse_usage(&complaint)),
    };
    if video && !matches!(flag_value(&args, "--images"), Flag::Absent) {
        return Err(refuse_usage(
            "--video luôn chọn đúng một MP4; không đi cùng --images",
        ));
    }
    let images = if video {
        1
    } else {
        match how_many_images(&args) {
            Ok(images) => images,
            Err(complaint) => return Err(refuse_usage(&complaint)),
        }
    };
    let visit_caption_step = match switch(&args, "--visit-caption-step") {
        Ok(on) => on,
        Err(complaint) => return Err(refuse_usage(&complaint)),
    };
    let dump_sound_picker = match switch(&args, "--dump-sound-picker") {
        Ok(on) => on,
        Err(complaint) => return Err(refuse_usage(&complaint)),
    };
    let sound_entry_id = match flag_value(&args, "--sound-entry-id") {
        Flag::Value(value) => Some(value),
        Flag::Absent => None,
        Flag::Unusable => {
            return Err(refuse_usage(
                "--sound-entry-id cần suffix resource-id đọc từ dump bước chỉnh sửa",
            ));
        }
        Flag::Repeated => {
            return Err(refuse_usage(
                "--sound-entry-id xuất hiện nhiều lần — không đoán lần nào là thật",
            ));
        }
    };
    if dump_sound_picker != sound_entry_id.is_some() {
        return Err(refuse_usage(
            "--dump-sound-picker và --sound-entry-id phải đi cùng nhau",
        ));
    }
    let peek_multi_select = match switch(&args, "--peek-multi-select") {
        Ok(on) => on,
        Err(complaint) => return Err(refuse_usage(&complaint)),
    };
    let dump_picker = match switch(&args, "--dump-picker") {
        Ok(on) => on,
        Err(complaint) => return Err(refuse_usage(&complaint)),
    };
    let dump_exit_menu = match switch(&args, "--dump-exit-menu") {
        Ok(on) => on,
        Err(complaint) => return Err(refuse_usage(&complaint)),
    };
    if [
        peek_multi_select,
        visit_caption_step,
        dump_sound_picker,
        dump_picker,
        dump_exit_menu,
    ]
    .iter()
    .filter(|on| **on)
    .count()
        > 1
    {
        return Err(refuse_usage(
            "--peek-multi-select / --visit-caption-step / --dump-picker / --dump-exit-menu \
             là những chuyến khác nhau — chạy từng cái một",
        ));
    }
    if video && (peek_multi_select || dump_picker || visit_caption_step || dump_sound_picker) {
        return Err(refuse_usage(
            "--video chỉ đo tới editor (có thể đi cùng --dump-exit-menu); không mở caption, sound hay chuyến picker khác",
        ));
    }

    let driver = AndroidDriver::new(&common::repo_config())?;
    let package = driver.resolve_tiktok_package(serial).await?;
    let session = driver.open_session(serial).await?;
    let language = session.ui_language().await.unwrap_or_default();
    let version = session.app_version(&package).await.unwrap_or_default();
    say(&format!(
        "serial   {serial}\npackage  {package}\nlanguage {language}\nversion  {version}\nalbum    {album:?}\nmedia    {}\ncount    {images}\n",
        if video { "video" } else { "images" }
    ));

    let Some(labels) = controls_for(&package, &language, &version) else {
        say("no measured label set for this (package, language) — run label_scout first");
        // A refusal exits like one. `refuse_usage` closed this shape for the argument arms;
        // these three are the same shape one step later — a run that reached the phone,
        // measured nothing, and told a multi-serial loop it had succeeded.
        anyhow::bail!("bộ nhãn chưa đo cho ({package}, {language})");
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

    let (width, height) = riviu_core::screen::measured_screen_size(&session).await?;
    let Some(screen) = Screen::new(width, height) else {
        say(&format!("screen {width}x{height} is not a screen"));
        anyhow::bail!("kích thước màn hình đọc ra không dùng được: {width}x{height}");
    };
    say(&format!("screen   {width}x{height}\n"));

    // **The picker-harvest trip, for a build whose picker has never been read.** The normal
    // walk cannot start here — `ComposerPlan::resolve` refuses while the picker labels are
    // missing, and standing on the picker is the only way to measure them. This stage needs
    // just the road TO it (Create, the shutter as proof the camera is up, and the measured
    // gallery-entry id), dumps what it finds, and backs out. The dump is the harvest: the
    // album pill's id, the tab strip, `Select multiple` and `Next` all live on this screen.
    if dump_picker {
        let plan = match ComposerPlan::resolve_for_picker_measuring(&labels) {
            Ok(plan) => plan,
            Err(refusal) => {
                say(&format!("\nkhông đi được tới picker để đo: {refusal}"));
                anyhow::bail!("thiếu nhãn cho chuyến đo picker: {refusal}");
            }
        };
        driver.launch_app(serial, &package).await?;
        tokio::time::sleep(std::time::Duration::from_secs(4)).await;
        let stop = AtomicBool::new(false);
        let mut composer = Composer::new(&session, plan, |element: &riviu_core::ElementBox| {
            element.centre()
        });
        let arrived = enter_picker_for_measuring(&mut composer, screen, &stop).await;
        let mut trip_failure: Option<String> = None;
        if matches!(&arrived, Ok(ComposerVerdict::Stopped)) {
            tokio::time::sleep(std::time::Duration::from_millis(2000)).await;
            say("\n--- picker (chuyến đo nhãn) ---");
            match session.agent().source().await {
                Ok(source) => {
                    let _ = std::fs::create_dir_all("target");
                    let _ = std::fs::write("target/picker-measuring.xml", &source);
                    dump_elements(&source);
                    say("\n(XML đầy đủ: target/picker-measuring.xml)");
                }
                Err(error) => trip_failure = Some(format!("không dump được picker: {error:#}")),
            }
        } else {
            trip_failure = Some(match &arrived {
                Ok(refusal) => format!("không tới được picker: {}", refusal.reason()),
                Err(error) => format!("lỗi giữa đường: {error:#}"),
            });
        }
        let out = composer.leave().await;
        let stopped = force_stop_and_confirm(&driver, serial, &package).await;
        say(&format!(
            "\nlùi về feed: {}",
            if out {
                "xong"
            } else {
                "CHƯA — kiểm tra máy"
            }
        ));
        arrived?;
        anyhow::ensure!(
            out,
            "không lùi được về feed sau chuyến đo picker — kiểm tra máy"
        );
        stopped?;
        if let Some(reason) = trip_failure {
            anyhow::bail!("chuyến đo picker không trọn: {reason}");
        }
        return Ok(());
    }

    let Some(album) = album else {
        return Err(refuse_usage(
            "--album is required: the name of an album this phone's picker already shows",
        ));
    };

    let plan = match ComposerPlan::resolve(&labels) {
        Ok(plan) => plan,
        Err(refusal) => {
            say(&format!("\nkhông đi được tới bước chỉnh sửa: {refusal}"));
            anyhow::bail!("thiếu nhãn để tới bước chỉnh sửa: {refusal}");
        }
    };
    let video_plan = if video {
        Some(
            VideoPickerPlan::resolve(&package, &language, &version).ok_or_else(|| {
                anyhow::anyhow!(
                    "video picker chưa đo cho ({package}, {language}, {version}); không mở composer"
                )
            })?,
        )
    } else {
        None
    };

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

    // **The toggle-state trip: two dumps around one production tap, nothing else.** The
    // 31/08 twelve-run alternation proved `Select multiple` is a remembered two-way switch;
    // writing a read-before-tap needs to know how the picker LOOKS in each state, and this
    // is the instrument that goes and looks. It walks to the picker with the production
    // walk (`reach_picker` — the same code `drive` runs), dumps, makes the one tap the
    // publish path makes, dumps again, and backs out. Because the state is remembered,
    // running this twice back-to-back photographs both directions: OFF→ON, then ON→OFF.
    if peek_multi_select {
        let arrived = reach_picker(&mut composer, &request, &stop).await;
        let mut trip_failure: Option<String> = None;
        if matches!(&arrived, Ok(ComposerVerdict::Stopped)) {
            let _ = std::fs::create_dir_all("target");
            say("\n--- picker TRƯỚC cú bấm 'Chọn nhiều' ---");
            match session.agent().source().await {
                Ok(source) => {
                    let _ = std::fs::write("target/picker-before.xml", &source);
                    dump_elements(&source);
                    say("\n(XML đầy đủ: target/picker-before.xml)");
                }
                Err(error) => trip_failure = Some(format!("không dump được màn TRƯỚC: {error:#}")),
            }
            if trip_failure.is_none() {
                match composer.tap_multi_select_once(&stop).await {
                    Ok(true) => {
                        tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
                        say("\n--- picker SAU cú bấm ---");
                        match session.agent().source().await {
                            Ok(source) => {
                                let _ = std::fs::write("target/picker-after.xml", &source);
                                dump_elements(&source);
                                say("\n(XML đầy đủ: target/picker-after.xml)");
                            }
                            Err(error) => {
                                trip_failure = Some(format!("không dump được màn SAU: {error:#}"))
                            }
                        }
                    }
                    Ok(false) => {
                        trip_failure = Some("không thấy nút 'Chọn nhiều' trên picker".to_string())
                    }
                    Err(error) => trip_failure = Some(format!("cú bấm lỗi: {error:#}")),
                }
            }
        } else {
            trip_failure = Some(match &arrived {
                Ok(refusal) => format!("không tới được picker: {}", refusal.reason()),
                Err(error) => format!("lỗi giữa đường: {error:#}"),
            });
        }
        let out = composer.leave().await;
        let stopped = force_stop_and_confirm(&driver, serial, &package).await;
        say(&format!(
            "\nlùi về feed: {}",
            if out {
                "xong"
            } else {
                "CHƯA — kiểm tra máy"
            }
        ));
        arrived?;
        anyhow::ensure!(out, "không lùi được về feed sau chuyến peek — kiểm tra máy");
        stopped?;
        if let Some(reason) = trip_failure {
            anyhow::bail!("chuyến peek không trọn: {reason}");
        }
        return Ok(());
    }

    let verdict = if let Some(video_plan) = video_plan {
        let video_request = VideoRequest {
            album: &album,
            caption: "",
            screen,
        };
        reach_video_edit_step(&mut composer, video_plan, &video_request, &stop).await
    } else {
        reach_edit_step(&mut composer, &request, &stop).await
    };
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
    //
    // **A requested stage that fails is a failed run.** Its error is printed and then
    // carried past the walk-out below rather than thrown here, because the phone is one tap
    // from Post and the Back walk has to happen whatever the peek did — but it must not
    // vanish: the operator asked for this stage, and exiting 0 without it tells a script the
    // measurement happened.
    let mut peek_failure: Option<String> = None;
    if dump_sound_picker {
        let mut trip_failure: Option<String> = None;
        if matches!(&verdict, Ok(ComposerVerdict::Stopped)) {
            let suffix = sound_entry_id
                .as_deref()
                .expect("paired flag was validated");
            match session
                .locate(riviu_core::ElementQuery::ResourceIdSuffix(suffix))
                .await
            {
                Ok(Some(entry)) if entry.enabled && entry.clickable => {
                    if let Err(error) = session.tap(entry.centre()).await {
                        trip_failure = Some(format!("không mở được sound picker: {error:#}"));
                    } else {
                        tokio::time::sleep(std::time::Duration::from_secs(3)).await;
                        say("\n--- sound picker (tool chỉ mở và đọc, không chọn nhạc) ---");
                        match session.agent().source().await {
                            Ok(source) => {
                                let path = std::path::Path::new("target").join("sound-picker.xml");
                                let _ = std::fs::create_dir_all("target");
                                let _ = std::fs::write(&path, &source);
                                dump_elements(&source);
                                say(&format!("\n(XML đầy đủ: {})", path.display()));
                            }
                            Err(error) => {
                                trip_failure =
                                    Some(format!("không dump được sound picker: {error:#}"));
                            }
                        }
                    }
                }
                Ok(Some(_)) => {
                    trip_failure = Some(format!(
                        "node sound {suffix:?} có mặt nhưng chưa enabled/clickable"
                    ));
                }
                Ok(None) => {
                    trip_failure = Some(format!("không thấy node sound {suffix:?}"));
                }
                Err(error) => {
                    trip_failure = Some(format!("không đọc được node sound {suffix:?}: {error:#}"));
                }
            }
        } else {
            trip_failure = Some("chưa đứng ở bước chỉnh sửa nên không mở sound picker".to_string());
        }
        match force_stop_and_confirm(&driver, serial, &package).await {
            Ok(()) => {}
            Err(error) => {
                say(&format!("\nforce-stop KHÔNG xác nhận được: {error:#}"));
                trip_failure
                    .get_or_insert_with(|| format!("force-stop không xác nhận được: {error:#}"));
            }
        }
        verdict?;
        if let Some(reason) = trip_failure {
            anyhow::bail!("chuyến đo sound picker không trọn: {reason}");
        }
        return Ok(());
    }
    if visit_caption_step {
        if matches!(&verdict, Ok(ComposerVerdict::Stopped)) {
            // `stop` is never set in this tool, so `Stopped` here can only be the measuring
            // terminus: standing on the edit step, before its Next.
            if let Err(error) = peek_at_the_caption_step(&session, labels).await {
                say(&format!("\n--visit-caption-step lỗi: {error:#}"));
                peek_failure = Some(format!("{error:#}"));
            }
        } else {
            say("\n--visit-caption-step: chưa đứng ở bước chỉnh sửa nên không đi tiếp");
            peek_failure = Some(format!(
                "không đứng ở bước chỉnh sửa (verdict: {})",
                match &verdict {
                    Ok(reached) => reached.reason().to_string(),
                    Err(error) => format!("{error:#}"),
                }
            ));
        }
    }

    // **The exit-menu trip: one Back on the edit step, dump what appears, then a verified
    // kill.** Measured need: on `musically` Back there opens a Discard / Save draft / Send
    // to friends sheet instead of leaving, so every walk on that build ended stranded. The
    // walk-back arm that will tap Discard needs that sheet's nodes measured first — and
    // this trip cannot use `leave()` to get out (that is the thing that does not work), so
    // it exits through `terminate_app`, which proves the kill instead of assuming it.
    if dump_exit_menu {
        let mut trip_failure: Option<String> = None;
        if matches!(&verdict, Ok(ComposerVerdict::Stopped)) {
            let _ = session.back().await;
            tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
            say("\n--- màn hình sau MỘT cú Back trên bước chỉnh sửa ---");
            match session.agent().source().await {
                Ok(source) => {
                    let _ = std::fs::write("target/exit-menu.xml", &source);
                    dump_elements(&source);
                    say("\n(XML đầy đủ: target/exit-menu.xml)");
                }
                Err(error) => trip_failure = Some(format!("không dump được menu thoát: {error:#}")),
            }
        } else {
            trip_failure = Some("chưa đứng ở bước chỉnh sửa nên không có menu để đo".to_string());
        }
        match force_stop_and_confirm(&driver, serial, &package).await {
            Ok(()) => {}
            Err(error) => {
                say(&format!("\nforce-stop KHÔNG xác nhận được: {error:#}"));
                trip_failure
                    .get_or_insert_with(|| format!("force-stop không xác nhận được: {error:#}"));
            }
        }
        verdict?;
        if let Some(reason) = trip_failure {
            anyhow::bail!("chuyến đo menu thoát không trọn: {reason}");
        }
        return Ok(());
    }

    // The caller owns the walk-back — see `reach_edit_step`.
    let out = composer.leave().await;
    let stopped = force_stop_and_confirm(&driver, serial, &package).await;
    say(&format!(
        "\nlùi về feed: {}",
        if out {
            "xong"
        } else {
            "CHƯA — kiểm tra máy"
        }
    ));
    verdict?;
    stopped?;
    // **Not backing out is a failure of this run**, even when the walk itself went fine. It
    // used to print the warning and exit 0, so a script saw success while the phone sat on the
    // edit step with a carousel selected — and the next thing to run started from there.
    anyhow::ensure!(
        out,
        "không lùi được về feed: máy còn đang ở trong composer với ảnh đã chọn. Kiểm tra tay          trước khi chạy tiếp bất cứ thứ gì trên máy này."
    );
    if let Some(reason) = peek_failure {
        anyhow::bail!("--visit-caption-step không đo được: {reason}");
    }
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

    /// **A misspelled flag refuses; it does not quietly run a shorter trip.**
    ///
    /// `--visit-caption-stpe` read as "not asked for", so the run stopped at the edit step
    /// and exited 0 — the caption screen the operator went out to measure never opened, and
    /// nothing said so.
    #[test]
    fn a_misspelled_flag_is_refused_rather_than_silently_skipping_its_stage() {
        assert_eq!(
            refuse_unknown_flags(&line(&["SN", "--album", "Camera", "--images", "3"])),
            Ok(())
        );
        assert_eq!(
            refuse_unknown_flags(&line(&["SN", "--album", "Camera", "--visit-caption-step"])),
            Ok(())
        );
        for typo in ["--visit-caption-stpe", "--albums", "--image", "--capture"] {
            let refused = refuse_unknown_flags(&line(&["SN", "--album", "Camera", typo]));
            assert!(
                refused.as_ref().is_err_and(|why| why.contains(typo)),
                "{typo} must be refused by name, not ignored: {refused:?}"
            );
        }
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
