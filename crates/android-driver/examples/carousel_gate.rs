//! Gate: prove a **photo post is photographed slide by slide**, on a real phone.
//!
//! ```text
//! cargo run -p riviu-android-driver --example carousel_gate -- <serial> <url> [out-dir]
//! ```
//!
//! **Read-only.** It opens the post, swipes sideways, and saves screenshots. Nothing is
//! liked, followed, typed or sent.
//!
//! This exists because the defect it covers is invisible to every unit test in the repo. The
//! interaction engine used to photograph a target by taking three stream frames 500 ms apart;
//! `make_contact_sheet` then collapsed them, correctly, into one picture, because a photo post
//! does not move. So every comment on a carousel was written from **slide one** — and on the
//! post that surfaced this (`@pht.th.h.slay/photo/7668948504827448583`) slide one is a person
//! lying by a lake while slide two is the whole content, a costed three-day itinerary. No
//! amount of extra sampling time would have found slide two: a carousel waits to be swiped.
//!
//! Three things have to be true on hardware, and none of them can be asserted in a fixture:
//!
//! 1. **The slide counter is readable through the driver's agent.** It was measured with
//!    `uiautomator dump`, which is not the path production takes — and on some phones in this
//!    fleet `uiautomator dump` is killed outright because another agent holds `UiAutomation`.
//!    `read_carousel_index` goes through `find_all` + `rect` + `attribute` instead, and that
//!    it finds the same three nodes is a claim about a real screen.
//! 2. **The counter is still there when it is read.** It leaves the tree about three seconds
//!    after the page turns — a dump taken later is byte-identical to the pre-swipe one — so
//!    the read has to win a race against a fade, with a hierarchy round-trip in between.
//! 3. **The walk stops at the last slide.** One swipe past it does not wrap: it leaves for
//!    the author's profile page, Follow button and all. That is the failure this gate is
//!    really watching for, because it is permanent and it happens on a stranger's account.
//!
//! It calls `photograph_photo_post` itself rather than re-implementing the sequence. A gate
//! with its own copy of the order of operations proves only that the copy works.

use std::sync::atomic::AtomicBool;
use std::time::Duration;

use riviu_android_driver::AndroidDriver;
use riviu_core::driver::{DeviceDriver, UiSession};
use riviu_core::interaction_hierarchy::{
    open_target_by_hierarchy, photograph_photo_post, read_carousel_index, SlideCamera,
    TargetArrival,
};
use riviu_core::tiktok_labels;

#[path = "common/mod.rs"]
mod common;

/// Screenshots, because the walk's own pictures are the thing under test.
///
/// Production reads the phone's live scrcpy stream; there is no stream here and standing one
/// up would put a second moving part between the gate and what it is checking. A screencap is
/// the same screen by a slower road, which is the right trade for a measurement.
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
        println!("usage: carousel_gate <serial> <url> [out-dir]");
        return Ok(());
    };
    let out_dir = args
        .get(2)
        .cloned()
        .unwrap_or_else(|| "target/carousel-gate".to_string());
    std::fs::create_dir_all(&out_dir)?;

    let driver = AndroidDriver::new(&common::repo_config())?;
    let package = driver.resolve_tiktok_package(serial).await?;
    let session = driver.open_session(serial).await?;
    let language = session.ui_language().await.unwrap_or_default();
    let app_version = session.app_version(&package).await.unwrap_or_default();
    let labels = tiktok_labels::controls_for(&package, &language, &app_version)
        .ok_or_else(|| anyhow::anyhow!("no measured labels for {package} + {language:?}"))?;
    println!(
        "serial   {serial}\npackage  {package}\nlanguage {language:?}\nversion  {app_version:?}"
    );

    // The handle out of the link, the same way the resolver does it, so arrival can report
    // `Identified` rather than settling for the structural proof.
    let handle = url
        .split("/@")
        .nth(1)
        .and_then(|rest| rest.split('/').next())
        .unwrap_or_default();
    let stop = AtomicBool::new(false);

    // Foreground TikTok first, which is what `DeviceControlPlane::start_interaction_session`
    // does before every assignment in the app. Skipping it is not a shortcut: the first run of
    // this gate refused at `target_open_no_baseline` because the phone was left on the author's
    // profile page by the previous run, where there is no author row to read *and* no For You
    // tab for the fallback to reach for.
    // Stopped first, then launched. Resuming instead is what the second run of this gate did,
    // and TikTok came back **on the post itself** — so the author read before the open matched
    // the author read after it, and arrival correctly refused with `target_open_screen_unchanged`
    // for a post that was on screen the whole time. The campaign path has a retry for that; a
    // measurement should not need one, so it starts from a feed.
    driver.terminate_app(serial, &package).await?;
    driver.launch_app(serial, &package).await?;

    // **Wait the splash out properly, and say how long it took.** Twelve seconds was not
    // enough: the third run of this gate refused with `target_open_screen_unchanged` and the
    // phone was still on `…aweme.splash.SplashActivity` afterwards, which is the trap §9.19
    // measured at 15-27 s for a cold start. `AndroidDriver::wait_out_splash` grants eight
    // seconds because it is used on twenty phones at once and must not fail any of them; one
    // phone being measured can afford to wait for the real thing.
    let splash_started = std::time::Instant::now();
    loop {
        // Both halves matter. Waiting only for "not the splash" broke on the fourth run: at
        // 0.1 s the foreground was still `com.sec.android.app.launcher…`, which is not a splash
        // and is not TikTok either, so the gate went straight on to arrive on a home screen.
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
    // The feed still has to render after the splash leaves, and the baseline is read off it.
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

    // What the counter says the moment the post is open, before anything is swiped. Measured
    // once as absent on slide one — worth printing, because a build that does show it there
    // would let the walk know the length up front instead of after the first flick.
    println!(
        "counter at slide 1: {:?}",
        read_carousel_index(&session).await
    );

    let gestures = tokio::sync::Mutex::new(());
    let camera = ScreencapCamera { session: &session };
    let started = std::time::Instant::now();
    let walk = photograph_photo_post(
        &session,
        &camera,
        &package,
        &gestures,
        Duration::from_millis(500),
    )
    .await;
    let frames = &walk.frames;
    println!(
        "photographed {} slide(s) in {:.1}s",
        frames.len(),
        started.elapsed().as_secs_f64()
    );
    // **The whole point of printing these.** On a two-slide post, "read 2 of 2 and stopped" and
    // "could not read the counter and stopped" both end with two pictures, so the frame count
    // alone proves nothing about whether the counter was legible on this build.
    for (index, counter) in walk.counters.iter().enumerate() {
        match counter {
            Some((current, total)) => {
                println!("  counter after swipe {}: {current} / {total}", index + 1)
            }
            None => println!(
                "  counter after swipe {}: UNREADABLE (walk stops here)",
                index + 1
            ),
        }
    }

    let mut distinct = Vec::new();
    for (index, frame) in frames.iter().enumerate() {
        let path = format!("{out_dir}/slide-{:02}.png", index + 1);
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
            if distinct.contains(&digest) {
                "   <- SAME PICTURE as an earlier slide"
            } else {
                ""
            }
        );
        distinct.push(digest);
    }

    // The counter is gone by now; what matters is that the post is still the thing on screen.
    // Leaving on a profile page is the failure mode this gate exists for, and it is silent.
    println!(
        "still on the post: {:?}",
        read_carousel_index(&session).await
    );
    println!(
        "\ndistinct pictures: {} of {} frame(s)",
        distinct
            .iter()
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        frames.len()
    );
    Ok(())
}
