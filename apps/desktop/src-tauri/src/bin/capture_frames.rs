//! Capture calibration frames from a plugged-in iPhone.
//!
//! Do not run this while the desktop app is open — same usbmux contention rule
//! as `live_nurture_test`.
//!
//! `RIVIU_FRAME_DUMP` only writes when the *classification* changes, so it can
//! never produce the case the swipe detector has to survive: two frames of the
//! same feed card while the video plays. Both classify as `Feed`, so the dump
//! keeps one. This tool captures on a timer instead, and prints the numbers each
//! detector actually keys on, so a threshold can be read off a real frame rather
//! than guessed.
//!
//! ```text
//! cargo run -p riviu-managers-phone --bin capture_frames -- \
//!   --label feed-same-card --delay 12 --count 2 --gap-ms 700
//! ```
//!
//! `--delay` gives you time to put the phone on the screen you want. Files land
//! in `crates/core/tests/fixtures/<label>-<n>.jpg` unless `--out` says otherwise.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use app_lib::agent_runtime::{resolve_desktop_agent_runtime_with_candidate, ResolvedAgentRuntime};
use riviu_core::db::Database;
use riviu_core::screen;
use riviu_core::{
    DeviceControlPlane, DeviceWorkCoordinator, DeviceWorkOwner, InteractionSessionKind,
    StreamBudgetManager,
};
use riviu_ios_driver::create_driver;
use riviu_signing::CredentialStore;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    riviu_ios_driver::install_process_tree_guard()?;

    let mut udid = String::new();
    let mut label = "capture".to_string();
    let mut count: u32 = 2;
    let mut gap_ms: u64 = 700;
    let mut delay: u64 = 10;
    let mut bundle = "com.ss.iphone.ugc.Ame".to_string();
    let mut out: Option<PathBuf> = None;
    let mut explore: u32 = 0;

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--udid" => udid = args.next().unwrap_or_default(),
            "--label" => label = args.next().unwrap_or(label),
            "--count" => count = args.next().and_then(|v| v.parse().ok()).unwrap_or(count),
            "--gap-ms" => gap_ms = args.next().and_then(|v| v.parse().ok()).unwrap_or(gap_ms),
            "--delay" => delay = args.next().and_then(|v| v.parse().ok()).unwrap_or(delay),
            "--bundle" => bundle = args.next().unwrap_or(bundle),
            "--out" => out = args.next().map(PathBuf::from),
            "--explore" => explore = args.next().and_then(|v| v.parse().ok()).unwrap_or(explore),
            other => eprintln!("bỏ qua tham số {other}"),
        }
    }

    let root = resolve_sidecar_root();
    let out_dir = out.unwrap_or_else(|| root.join("../crates/core/tests/fixtures"));
    std::fs::create_dir_all(&out_dir)?;

    let data = std::env::temp_dir().join("riviu-capture-frames");
    std::fs::create_dir_all(&data)?;
    let db = Database::open(data.join("riviu.db"))?;

    // Resolve exactly as the desktop app does — same keychain account, same
    // manifest — or the capture authenticates against an agent the device is
    // not running.
    let ResolvedAgentRuntime { driver_config, .. } = resolve_desktop_agent_runtime_with_candidate(
        root.clone(),
        data.clone(),
        &db,
        &CredentialStore::system()?,
        std::env::var("RIVIU_RTMMO_TOKEN").ok().as_deref(),
        false,
        true,
    )?;
    let driver = create_driver(driver_config).await?;
    let devices = driver.driver.list_devices().await?;
    if devices.is_empty() {
        anyhow::bail!("không có iPhone nào đang cắm");
    }
    if udid.is_empty() {
        udid = devices[0].udid.clone();
    }
    println!("thiết bị = {udid}");

    let control = Arc::new(DeviceControlPlane::new(
        driver.driver,
        Arc::new(DeviceWorkCoordinator::new()),
        Arc::new(StreamBudgetManager::default()),
    ));
    let frames = Arc::new(driver.streams.clone());

    // The desktop sampler normally runs this install/auth-only preflight before
    // any interaction job. Without it `start_interaction_session` refuses with
    // "install-only protected control relay is required".
    println!("tiền kiểm agent…");
    let preflight_context = control
        .try_acquire_exclusive(&udid, DeviceWorkOwner::Repair)
        .await?;
    let preflight = tokio::time::timeout(
        Duration::from_secs(45),
        control.preflight_agent(&preflight_context),
    )
    .await
    .map_err(|_| anyhow::anyhow!("tiền kiểm agent quá 45s"))??;
    if !preflight.auth_ready {
        anyhow::bail!(
            "Riviu Agent chưa sẵn sàng: {}",
            preflight
                .message
                .unwrap_or_else(|| "protected auth unavailable".to_string())
        );
    }
    println!("agent OK: state={:?}", preflight.state);
    // Release before the capture takes its own device owner, or the interaction
    // job queues forever behind this completed preflight.
    drop(preflight_context);

    // Same acquisition order the nurture engine uses, so the capture sees the
    // stream exactly as the engine would.
    let exclusive = control
        .acquire_exclusive(&udid, DeviceWorkOwner::Nurture)
        .await?;
    let (exclusive, capacity) = control.reserve_ui_capacity(exclusive).await?;
    let session = control
        .start_interaction_session(exclusive, &bundle, InteractionSessionKind::Ordinary)
        .await?;
    let context = control.start_reserved_stream(session, capacity).await?;

    println!("stream đã mở — đưa máy về màn hình cần chụp, còn {delay}s");
    for remaining in (1..=delay).rev() {
        if remaining % 5 == 0 || remaining <= 3 {
            println!("  {remaining}…");
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }

    if explore > 0 {
        let written = explore_feed(&control, &context, &frames, &udid, explore, &out_dir).await?;
        if let Err(error) = control.close_ui_context(context).await {
            eprintln!("dọn dẹp: {error}");
        }
        control.shutdown_cleanup().await?;
        println!("đã ghi {written} ảnh vào {}", out_dir.display());
        return Ok(());
    }

    let mut written = 0u32;
    for index in 1..=count {
        match frames.latest(&udid) {
            Some(frame) => {
                let path = out_dir.join(format!("{label}-{index}.jpg"));
                std::fs::write(&path, frame.as_slice())?;
                written += 1;
                let measured = match image::load_from_memory(&frame) {
                    Ok(decoded) => describe(&decoded.to_rgb8()),
                    Err(error) => format!("<giải mã lỗi: {error}>"),
                };
                println!("[{index}/{count}] {}\n    {measured}", path.display());
            }
            None => println!("[{index}/{count}] chưa có khung hình nào"),
        }
        if index < count {
            tokio::time::sleep(Duration::from_millis(gap_ms)).await;
        }
    }

    if let Err(error) = control.close_ui_context(context).await {
        eprintln!("dọn dẹp: {error}");
    }
    control.shutdown_cleanup().await?;
    println!("đã ghi {written} ảnh vào {}", out_dir.display());
    Ok(())
}

fn resolve_sidecar_root() -> PathBuf {
    std::env::var_os("RIVIU_SIDECAR_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../sidecars"))
}

/// Swipe the feed collecting the cases the fixture set is missing: a real LIVE
/// room, and the reddest unliked heart the video content can produce. Only
/// swipes and — for LIVE — one enter/exit tap; never likes, comments or follows.
async fn explore_feed(
    control: &Arc<DeviceControlPlane>,
    context: &riviu_core::device_control::UiWithStreamContext,
    frames: &Arc<riviu_ios_driver::StreamHub>,
    udid: &str,
    cards: u32,
    out_dir: &std::path::Path,
) -> anyhow::Result<u32> {
    use riviu_core::{SwipeGesture, TapPoint};

    let session = control.streaming_session(context)?;
    let size = session.window_size().await.unwrap_or((375.0, 667.0));
    let mut written = 0u32;
    let mut live_captured = false;
    // The reddest heart seen while the heart is still an outline. This is the
    // number that decides whether LIKE_FILLED_REDNESS = 90 has any headroom.
    let mut reddest_unliked = f64::MIN;

    for card in 1..=cards {
        tokio::time::sleep(Duration::from_millis(1_400)).await;
        let Some(frame) = frames.latest(udid) else {
            continue;
        };
        let Ok(decoded) = image::load_from_memory(&frame) else {
            continue;
        };
        let img = decoded.to_rgb8();
        let observation = screen::classify(&img, Some(size.0));
        let kind = screen::feed_card_kind(&img);
        let rail = screen::locate_action_rail(&img);
        let redness = rail.map(|found| screen::like_redness_at(&img, &found));
        println!(
            "[{card}/{cards}] kind={:?} card={kind:?} rail={} like_redness={}",
            observation.kind,
            rail.is_some(),
            redness.map_or_else(|| "n/a".to_string(), |value| format!("{value:.1}")),
        );

        // An outline heart reads well below the filled threshold; anything
        // approaching it on an unliked card is the false-positive margin.
        if let Some(redness) =
            redness.filter(|value| *value > reddest_unliked && *value < screen::LIKE_FILLED_REDNESS)
        {
            reddest_unliked = redness;
            let path = out_dir.join("feed-red-video-1.jpg");
            std::fs::write(&path, frame.as_slice())?;
            written += 1;
            println!(
                "    ↳ nền đỏ nhất tới giờ ({redness:.1}) → {}",
                path.display()
            );
        }

        if !live_captured && kind == screen::FeedCardKind::LivePreview {
            println!("    ↳ gặp thẻ LIVE — vào phòng để chụp");
            session
                .tap(TapPoint {
                    x: size.0 * 0.50,
                    y: size.1 * 0.46,
                })
                .await?;
            tokio::time::sleep(Duration::from_secs(6)).await;
            if let Some(inside) = frames.latest(udid) {
                let path = out_dir.join("live-room-1.jpg");
                std::fs::write(&path, inside.as_slice())?;
                written += 1;
                let note = image::load_from_memory(&inside)
                    .map(|d| describe(&d.to_rgb8()))
                    .unwrap_or_else(|e| format!("<giải mã lỗi: {e}>"));
                println!("    ↳ {}\n      {note}", path.display());
                live_captured = true;
            }
            // Leave the room the same way the engine does.
            session
                .tap(TapPoint {
                    x: size.0 * screen::LIVE_EXIT.0,
                    y: size.1 * screen::LIVE_EXIT.1,
                })
                .await?;
            tokio::time::sleep(Duration::from_secs(4)).await;
            continue;
        }

        session
            .swipe(SwipeGesture {
                from: TapPoint {
                    x: size.0 * 0.5,
                    y: size.1 * 0.75,
                },
                to: TapPoint {
                    x: size.0 * 0.5,
                    y: size.1 * 0.25,
                },
                duration_ms: 420,
            })
            .await?;
    }

    println!("\nlike_redness cao nhất trên tim CHƯA like: {reddest_unliked:.1}");
    println!(
        "ngưỡng LIKE_FILLED_REDNESS = {:.1}",
        screen::LIKE_FILLED_REDNESS
    );
    println!("LIVE room đã chụp: {live_captured}");
    Ok(written)
}

/// Print what each detector keys on, so a threshold can be read off a real
/// frame instead of guessed.
fn describe(img: &image::RgbImage) -> String {
    let observation = screen::classify(img, None);
    let rail = screen::locate_action_rail(img);
    let like = rail
        .map(|found| screen::like_redness_at(img, &found))
        .unwrap_or_else(|| screen::like_redness(img));
    format!(
        "{}x{} kind={:?} card={:?} rail={} like_redness={:.1} live_pill={:.1} compose_bar={}",
        img.width(),
        img.height(),
        observation.kind,
        screen::feed_card_kind(img),
        rail.is_some(),
        like,
        observation.evidence.live_pill,
        screen::compose_bar_visible(img).0,
    )
}
