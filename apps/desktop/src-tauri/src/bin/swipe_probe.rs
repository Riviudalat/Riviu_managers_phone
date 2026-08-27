//! Ground truth for one question: when the nurture engine sends its swipe, does
//! the feed actually move?
//!
//! Do not run this while the desktop app or `live_nurture_test` is open — same
//! usbmux contention rule.
//!
//! A live run reported `videos=1/7`: every swipe after the first said the action
//! rail never left the screen. That has two possible causes and they call for
//! opposite fixes — the detector is missing the transition, or the gesture is
//! not moving the feed and the *old* detector was hiding it, because a playing
//! video changes the frame either way.
//!
//! So this sends the exact gesture `do_swipe` sends, samples the stream as fast
//! as it arrives, and prints what each frame says. It writes the first and last
//! frame of each attempt so the screen can also be read by eye.
//!
//! ```text
//! cargo run -p riviu-managers-phone --bin swipe_probe --release -- --swipes 6
//! ```

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use app_lib::agent_runtime::{resolve_desktop_agent_runtime_with_candidate, ResolvedAgentRuntime};
use riviu_core::db::Database;
use riviu_core::screen;
use riviu_core::{
    DeviceControlPlane, DeviceWorkCoordinator, DeviceWorkOwner, InteractionSessionKind,
    StreamBudgetManager, SwipeGesture, TapPoint,
};
use riviu_ios_driver::create_driver;
use riviu_signing::CredentialStore;

/// Same 512-byte sampling `nurture::frame_digest` uses, so "the frame changed"
/// here means what it means there.
fn frame_digest(frame: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325 ^ frame.len() as u64;
    let step = (frame.len() / 512).max(1);
    let mut i = 0;
    while i < frame.len() {
        h ^= frame[i] as u64;
        h = h.wrapping_mul(0x100_0000_01b3);
        i += step;
    }
    h
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    riviu_ios_driver::install_process_tree_guard()?;

    let mut udid = String::new();
    let mut swipes: u32 = 6;
    let mut delay: u64 = 6;
    let mut window_ms: u64 = 3_000;
    let mut from_y: f64 = 0.75;
    let mut to_y: f64 = 0.25;
    let mut duration_ms: u64 = 220;
    let bundle = "com.ss.iphone.ugc.Ame".to_string();

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--udid" => udid = args.next().unwrap_or_default(),
            "--swipes" => swipes = args.next().and_then(|v| v.parse().ok()).unwrap_or(swipes),
            "--delay" => delay = args.next().and_then(|v| v.parse().ok()).unwrap_or(delay),
            "--window-ms" => {
                window_ms = args
                    .next()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(window_ms)
            }
            "--from-y" => from_y = args.next().and_then(|v| v.parse().ok()).unwrap_or(from_y),
            "--to-y" => to_y = args.next().and_then(|v| v.parse().ok()).unwrap_or(to_y),
            "--duration-ms" => {
                duration_ms = args
                    .next()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(duration_ms)
            }
            other => eprintln!("bỏ qua tham số {other}"),
        }
    }

    let root = std::env::var_os("RIVIU_SIDECAR_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../sidecars"));
    let out_dir = std::env::var_os("RIVIU_PROBE_OUT")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("riviu-swipe-probe"));
    std::fs::create_dir_all(&out_dir)?;

    let data = std::env::temp_dir().join("riviu-swipe-probe-state");
    std::fs::create_dir_all(&data)?;
    let db = Database::open(data.join("riviu.db"))?;

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
        anyhow::bail!("Riviu Agent chưa sẵn sàng");
    }
    drop(preflight_context);

    let exclusive = control
        .acquire_exclusive(&udid, DeviceWorkOwner::Nurture)
        .await?;
    let (exclusive, capacity) = control.reserve_ui_capacity(exclusive).await?;
    let session = control
        .start_interaction_session(exclusive, &bundle, InteractionSessionKind::Ordinary)
        .await?;
    let context = control.start_reserved_stream(session, capacity).await?;
    let ui = control.streaming_session(&context)?;
    // This probe swipes the real feed. Refuse rather than swipe from a guessed origin.
    let size = riviu_core::screen::measured_screen_size(ui.as_ref()).await?;
    println!("màn hình = {:.0}x{:.0}pt", size.0, size.1);

    println!("đưa máy về FYP, còn {delay}s");
    for remaining in (1..=delay).rev() {
        if remaining % 3 == 0 || remaining <= 2 {
            println!("  {remaining}…");
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }

    let mut advanced = 0u32;
    for attempt in 1..=swipes {
        let before = frames.latest(&udid);
        let (rail_before, kind_before) = match before.as_ref().and_then(|f| decode(f)) {
            Some(img) => (
                screen::rail_icons_present(&img),
                format!("{:?}", screen::feed_card_kind(&img)),
            ),
            None => (false, "<no frame>".into()),
        };
        if let Some(bytes) = before.as_ref() {
            std::fs::write(
                out_dir.join(format!("s{attempt}-before.jpg")),
                bytes.as_slice(),
            )?;
        }
        println!("\n── vuốt {attempt}/{swipes} ── rail_before={rail_before} card={kind_before}");

        // The whole point: sample CONCURRENTLY with the gesture, and record when
        // the gesture call returned. The engine only starts looking after that
        // moment, so if the rail-absent window lands before it, the engine is
        // structurally unable to see the transition it keys on.
        let start = Instant::now();
        let gesture = ui.swipe(SwipeGesture {
            from: TapPoint {
                x: size.0 * 0.5,
                y: size.1 * from_y,
            },
            to: TapPoint {
                x: size.0 * 0.5,
                y: size.1 * to_y,
            },
            duration_ms,
        });

        let sample = async {
            let deadline = start + Duration::from_millis(window_ms);
            let mut seen: Option<u64> = None;
            let mut samples = 0u32;
            let mut rail_gone_at: Option<u128> = None;
            let mut settled_at: Option<u128> = None;
            let mut last: Option<Arc<Vec<u8>>> = None;
            let mut trace: Vec<String> = Vec::new();
            while Instant::now() < deadline {
                if let Some(frame) = frames.latest(&udid) {
                    let digest = frame_digest(&frame);
                    if seen != Some(digest) {
                        seen = Some(digest);
                        samples += 1;
                        if let Some(img) = decode(&frame) {
                            let ms = start.elapsed().as_millis();
                            let rail = screen::rail_icons_present(&img);
                            let card = screen::feed_card_kind(&img);
                            let settled = card == screen::FeedCardKind::Video;
                            if !rail && rail_gone_at.is_none() {
                                rail_gone_at = Some(ms);
                            }
                            if rail_gone_at.is_some() && settled && settled_at.is_none() {
                                settled_at = Some(ms);
                            }
                            trace.push(format!("{ms:>4}ms rail={} {card:?}", u8::from(rail)));
                            last = Some(frame.clone());
                        }
                    }
                }
                tokio::time::sleep(Duration::from_millis(30)).await;
            }
            (samples, rail_gone_at, settled_at, last, trace)
        };

        let gesture_timed = async {
            let result = gesture.await;
            (result, start.elapsed().as_millis())
        };
        let ((sent, gesture_returned_ms), (samples, rail_gone_at, settled_at, last, trace)) =
            tokio::join!(gesture_timed, sample);
        sent?;
        if let Some(bytes) = last.as_ref() {
            std::fs::write(
                out_dir.join(format!("s{attempt}-after.jpg")),
                bytes.as_slice(),
            )?;
        }

        // Did the *content* change at all? Compares the two saved frames whole,
        // which is what the old detector effectively did.
        let content_changed = match (before.as_ref(), last.as_ref()) {
            (Some(a), Some(b)) => frame_digest(a) != frame_digest(b),
            _ => false,
        };
        let verdict = match (rail_gone_at, settled_at) {
            (Some(_), Some(_)) => {
                advanced += 1;
                "ADVANCED"
            }
            (Some(_), None) => "MOVED (rail left, nothing settled)",
            (None, _) => "BLOCKED (rail never left)",
        };
        println!("   {samples} khung mới trong {window_ms}ms");
        for line in trace.iter().take(40) {
            println!("     {line}");
        }
        println!(
            "   gesture trả về sau {gesture_returned_ms}ms | rail_gone={:?} settled={:?} \
             whole_frame_changed={content_changed} => {verdict}",
            rail_gone_at, settled_at
        );
        // The engine starts watching only once the gesture call returns. If the
        // rail-absent window closed before that, no amount of watching after it
        // can see the transition — the swipe reads Blocked no matter what.
        match rail_gone_at {
            Some(ms) if ms < gesture_returned_ms => println!(
                "   >>> ENGINE MÙ: rail mất ở {ms}ms, engine chỉ bắt đầu nhìn từ \
                 {gesture_returned_ms}ms"
            ),
            Some(ms) => println!("   engine kịp thấy (rail mất ở {ms}ms, sau mốc bắt đầu nhìn)"),
            None => println!("   rail không hề mất — feed có thể thật sự không chuyển"),
        }
        tokio::time::sleep(Duration::from_millis(1_200)).await;
    }

    println!("\n════ {advanced}/{swipes} xác nhận advance ════");
    println!("ảnh: {}", out_dir.display());
    if let Err(error) = control.close_ui_context(context).await {
        eprintln!("dọn dẹp: {error}");
    }
    control.shutdown_cleanup().await?;
    Ok(())
}

fn decode(frame: &[u8]) -> Option<image::RgbImage> {
    image::load_from_memory(frame).ok().map(|i| i.to_rgb8())
}
