//! Gate: prove a **video is watched, not sampled where it stands**, on a real phone.
//!
//! ```text
//! cargo run -p riviu-android-driver --example video_gate -- <serial> <url> [out-dir]
//! ```
//!
//! **Read-only, and more strictly so than any other gate here.** `photograph_video_post` taps
//! nothing and swipes nothing — a video plays on its own — so this opens the post, watches it,
//! and saves screenshots. Nothing is liked, followed, typed or sent.
//!
//! # What it is watching for
//!
//! The interaction engine used to photograph a video with three stream frames 500 ms apart, so
//! it saw the first ~1.0 second of a post that can run a minute. `make_contact_sheet` then
//! collapsed them — correctly — because a video still on its cover does not move, and the sheet
//! reported "ĐÚNG MỘT khung … KHÔNG có chuyển động để mô tả". Every comment on such a post was
//! written from its title card.
//!
//! Three claims about hardware, none of which a fixture can make:
//!
//! 1. **The stream really does produce different pictures across the watch.** The unit tests
//!    drive a scripted camera; only a phone can say whether four samples 1.2–3 s apart on
//!    *this* fleet come back distinct, or whether the stream repeats and the walk correctly
//!    stops after two strikes.
//! 2. **The caption is readable on a video post page, repeatedly.** The walk re-reads it before
//!    every sample, because the comment rail alone cannot tell this post from the next one —
//!    TikTok advances when a video ends, and that screen has a rail too. If `:id/desc` is not
//!    there on a video page, the walk silently falls back to the weaker check and this gate is
//!    how that gets noticed.
//! 3. **The walk ends on the post it started on.** Printed at the end: if the caption changed,
//!    the guard fired, and the pictures kept are the ones taken before it did.
//!
//! It calls `photograph_video_post` itself rather than re-implementing the sequence. A gate with
//! its own copy of the order of operations proves only that the copy works.
//!
//! `duration` comes from `tiktok_web::fetch_post_context` — the same lookup the campaign does —
//! so a run also exercises that path. Pass `--secs N` to skip the network and force a length.

use std::sync::atomic::AtomicBool;
use std::time::Duration;

use riviu_android_driver::{AndroidDriver, AndroidDriverConfig};
use riviu_core::driver::{DeviceDriver, UiSession};
use riviu_core::interaction_hierarchy::{
    open_target_by_hierarchy, photograph_video_post, read_post_caption, SlideCamera, TargetArrival,
};
use riviu_core::tiktok_labels;

/// Screenshots, because the walk's own pictures are the thing under test.
///
/// Production reads the phone's live scrcpy stream; there is no stream here and standing one up
/// would put a second moving part between the gate and what it is checking. A screencap is the
/// same screen by a slower road, which is the right trade for a measurement — and it is the
/// same choice `carousel_gate` makes, for the same reason.
struct ScreencapCamera<'a> {
    session: &'a dyn UiSession,
}

#[async_trait::async_trait]
impl SlideCamera for ScreencapCamera<'_> {
    async fn capture(&self) -> Option<Vec<u8>> {
        self.session.screenshot_png().await.ok()
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (Some(serial), Some(url)) = (args.first(), args.get(1)) else {
        println!("usage: video_gate <serial> <url> [out-dir] [--secs N]");
        return Ok(());
    };
    let forced_secs: Option<u64> = args
        .iter()
        .position(|arg| arg == "--secs")
        .and_then(|at| args.get(at + 1))
        .and_then(|value| value.parse().ok());
    let out_dir = args
        .get(2)
        .filter(|arg| !arg.starts_with("--"))
        .cloned()
        .unwrap_or_else(|| "target/video-gate".to_string());
    std::fs::create_dir_all(&out_dir)?;

    // The length first, and off the network rather than off the phone: the walk needs it to
    // size its gaps, and a failure here is not a failure of the walk — it just means the gaps
    // fall back to spending the whole budget.
    let duration = match forced_secs {
        Some(secs) => {
            println!("thời lượng {secs}s (đặt tay bằng --secs)");
            Some(Duration::from_secs(secs))
        }
        None => match riviu_core::tiktok_web::fetch_post_context(url).await {
            Ok(context) => {
                println!(
                    "web      caption {} ký tự | thời lượng {:?}s | phụ đề {:?}",
                    context.caption.as_deref().map(|c| c.chars().count()).unwrap_or(0),
                    context.duration_secs,
                    context.subtitle_langs()
                );
                context.duration_secs.map(Duration::from_secs)
            }
            Err(error) => {
                println!("web      không tra được ({}): {error}", error.code());
                None
            }
        },
    };

    // **Point the driver at the repo's own adb, and do it through `bundled_adb_path`.**
    //
    // This host has no adb on `PATH` and no `ANDROID_HOME`, so a bare
    // `AndroidDriverConfig::default()` finds no adb at all — and the way that surfaces is the
    // trap: `list_devices` comes back empty, every `pm list packages` call fails silently, and
    // `resolve_tiktok_package` reports **`no TikTok build with measured labels is installed`**
    // on a phone that has one. Measured here on 26/08/2026: that message cost a whole wrong
    // conclusion ("no fleet phone attached") before `fleet_list` showed `0 device(s)`.
    //
    // `bundled_adb_path` and not `adb_path`, because that field is the documented lowest
    // priority: an operator's `RIVIU_ADB_PATH` still wins, which is the whole reason the two
    // fields are separate.
    let mut config = AndroidDriverConfig::default();
    if config.adb_path.is_none() && config.bundled_adb_path.is_none() {
        let bundled = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../sidecars/android/win-x86_64")
            .join(if cfg!(windows) { "adb.exe" } else { "adb" });
        if bundled.is_file() {
            config.bundled_adb_path = Some(bundled);
        }
    }
    let driver = AndroidDriver::new(&config)?;
    // Asked before anything else, because "no devices" and "no TikTok" are the same silence
    // otherwise. A gate that cannot tell the operator which one it hit is a gate that sends
    // them looking in the wrong place.
    let visible = driver.list_devices().await?;
    anyhow::ensure!(
        visible.iter().any(|device| device.udid == *serial),
        "adb does not see {serial} (thấy {} máy: {:?}) — kiểm cáp, hoặc đặt RIVIU_ADB_PATH",
        visible.len(),
        visible.iter().map(|device| &device.udid).collect::<Vec<_>>()
    );
    let package = driver.resolve_tiktok_package(serial).await?;
    let session = driver.open_session(serial).await?;
    let language = session.ui_language().await.unwrap_or_default();
    let app_version = session.app_version(&package).await.unwrap_or_default();
    let labels = tiktok_labels::controls_for(&package, &language, &app_version)
        .ok_or_else(|| anyhow::anyhow!("no measured labels for {package} + {language:?}"))?;
    println!(
        "serial   {serial}\npackage  {package}\nlanguage {language:?}\nversion  {app_version:?}"
    );

    let handle = url
        .split("/@")
        .nth(1)
        .and_then(|rest| rest.split('/').next())
        .unwrap_or_default();
    let stop = AtomicBool::new(false);

    // Stopped, then launched, then waited out — the same sequence and for the same three
    // reasons `carousel_gate` documents: a phone left on a profile page has no author row to
    // read, resuming can come back *on the post* and make arrival refuse correctly, and a cold
    // splash runs 15-27 s (§9.19).
    driver.terminate_app(serial, &package).await?;
    driver.launch_app(serial, &package).await?;
    let splash_started = std::time::Instant::now();
    loop {
        match driver.foreground_activity(serial).await {
            Some(activity) if activity.contains(&package) && !activity.contains("splash") => {
                println!(
                    "TikTok up after {:.1}s -> {activity}",
                    splash_started.elapsed().as_secs_f64()
                );
                break;
            }
            _ => {}
        }
        if splash_started.elapsed() >= Duration::from_secs(45) {
            println!("still on the splash after 45s - going ahead anyway");
            break;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    tokio::time::sleep(Duration::from_secs(4)).await;

    let arrival = open_target_by_hierarchy(&session, labels, &package, url, handle, &stop).await;
    match &arrival {
        Ok(TargetArrival::Identified { author_label }) => {
            println!("arrival  Identified ({author_label})")
        }
        Ok(TargetArrival::Structural) => println!("arrival  Structural"),
        Err(refusal) => {
            println!("arrival  REFUSED: {}", refusal.code());
            return Ok(());
        }
    }

    // **Printed before the walk, because the walk's second guard rests on it.** An unreadable
    // caption here is not a failure — the walk falls back to the rail check — but it is the
    // difference between "we know it is still this post" and "we know a post is up", and that
    // is exactly the distinction nobody would notice from a frame count.
    let baseline = read_post_caption(&session).await;
    match &baseline {
        Some(caption) => println!(
            "caption  đọc được, {} ký tự: {:?}",
            caption.chars().count(),
            caption.chars().take(48).collect::<String>()
        ),
        None => println!("caption  KHÔNG đọc được — vòng xem sẽ chỉ dựa vào thanh rail"),
    }

    let camera = ScreencapCamera { session: &session };
    let started = std::time::Instant::now();
    let watch = photograph_video_post(&session, &camera, &package, duration).await;
    println!(
        "\nxem {} khung trong {:.1}s thật, span khai báo {} giây",
        watch.frames.len(),
        started.elapsed().as_secs_f64(),
        watch.span_secs
    );

    // Byte digests, with the caveat stated: a PNG screencap of an unchanged screen still encodes
    // to the same bytes, so equal digests here really do mean an unchanged picture. That is *not*
    // true of the JPEG stream production uses, which is why the walk itself deduplicates on
    // decoded pixels via `picture_digest_of` rather than on bytes (§9.104).
    let mut seen: Vec<u64> = Vec::new();
    for (index, frame) in watch.frames.iter().enumerate() {
        let path = format!("{out_dir}/moment-{:02}.png", index + 1);
        std::fs::write(&path, frame)?;
        let digest = {
            use std::hash::{Hash, Hasher};
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            frame.hash(&mut hasher);
            hasher.finish()
        };
        println!(
            "  {path}  {} bytes  digest {digest:016x}{}",
            frame.len(),
            if seen.contains(&digest) {
                "   <- SAME PICTURE as an earlier moment"
            } else {
                ""
            }
        );
        seen.push(digest);
    }

    // Did the guard fire, or did the walk simply run out of samples? Both end with fewer than
    // four pictures and only this line tells them apart.
    let after = read_post_caption(&session).await;
    match (&baseline, &after) {
        (Some(before), Some(now)) if before == now => {
            println!("\ncòn đúng bài lúc kết thúc: CÓ")
        }
        (Some(_), Some(now)) => println!(
            "\ncòn đúng bài lúc kết thúc: KHÔNG — đã sang {:?} (chốt caption đã chặn đúng)",
            now.chars().take(48).collect::<String>()
        ),
        (Some(_), None) => println!("\ncaption biến mất lúc kết thúc — có thể đã rời trang bài"),
        (None, _) => println!("\nkhông có mốc caption để so"),
    }
    println!(
        "ảnh khác nhau: {} trên {} khung",
        seen.iter().collect::<std::collections::BTreeSet<_>>().len(),
        watch.frames.len()
    );
    Ok(())
}
