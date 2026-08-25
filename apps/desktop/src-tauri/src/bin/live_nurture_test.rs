//! Headless live nurture smoke test against a real USB iPhone.
//!
//! Do not run this while the desktop app is open: two processes competing for
//! the same usbmux channel is exactly the contention this project spent a week
//! removing.
//!
//! ```text
//! cargo build -p riviu-managers-phone --bin live_nurture_test --release
//! ./target/release/live_nurture_test \
//!   --udid a99f4bd9f877b2a0e3682ee24fd1c68f75ba6982 \
//!   --minutes 5 --videos 10 --like-prob 60 --comment-prob 0
//! ```
//!
//! Exit codes: `0` success, `1` bad usage/setup, `2` the run did not meet its
//! bar (no videos, ended `partial`/`failed`, or needed more than one recovery).

use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::{Duration, Instant};

use app_lib::agent_runtime::{resolve_desktop_agent_runtime_with_candidate, ResolvedAgentRuntime};
use app_lib::interaction_ocr::DesktopFrameTextSource;
use riviu_core::db::Database;
use riviu_core::{
    DeviceControlPlane, DeviceWorkCoordinator, DeviceWorkOwner, InteractionSessionKind,
    NurtureEngine, NurtureSettings, StreamBudgetManager,
};
use riviu_ios_driver::{create_driver, telemetry};
use riviu_signing::CredentialStore;

struct Args {
    udid: String,
    minutes: u64,
    videos: u32,
    rounds: u32,
    like_prob: u32,
    comment_prob: u32,
    follow_prob: u32,
    watch_min: f64,
    watch_max: f64,
    jsonl: Option<PathBuf>,
    steady: String,
    open_url: Option<String>,
}

impl Default for Args {
    fn default() -> Self {
        Self {
            udid: String::new(),
            minutes: 5,
            videos: 10,
            rounds: 1,
            like_prob: 50,
            comment_prob: 0,
            follow_prob: 5,
            watch_min: 4.0,
            watch_max: 8.0,
            jsonl: None,
            steady: String::new(),
            open_url: None,
        }
    }
}

fn parse_args() -> Result<Args, String> {
    let mut a = Args::default();
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let mut i = 0;
    while i < argv.len() {
        let value = argv
            .get(i + 1)
            .cloned()
            .ok_or_else(|| format!("{} cần một giá trị", argv[i]))?;
        let num = |s: &str, name: &str| -> Result<f64, String> {
            s.parse::<f64>()
                .map_err(|_| format!("{name} không phải số: {s}"))
        };
        match argv[i].as_str() {
            "--udid" => a.udid = value,
            "--minutes" => a.minutes = num(&value, "--minutes")? as u64,
            "--videos" => a.videos = num(&value, "--videos")? as u32,
            "--rounds" => a.rounds = num(&value, "--rounds")? as u32,
            "--like-prob" => a.like_prob = num(&value, "--like-prob")? as u32,
            "--comment-prob" => a.comment_prob = num(&value, "--comment-prob")? as u32,
            "--follow-prob" => a.follow_prob = num(&value, "--follow-prob")? as u32,
            "--watch-min" => a.watch_min = num(&value, "--watch-min")?,
            "--watch-max" => a.watch_max = num(&value, "--watch-max")?,
            "--jsonl" => a.jsonl = Some(PathBuf::from(value)),
            "--steady" => a.steady = value,
            "--open-url" => a.open_url = Some(value),
            other => return Err(format!("tham số lạ: {other}")),
        }
        i += 2;
    }
    if a.udid.is_empty() {
        return Err("--udid bắt buộc".into());
    }
    if a.watch_max < a.watch_min {
        return Err("--watch-max phải ≥ --watch-min".into());
    }
    Ok(a)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    riviu_ios_driver::install_process_tree_guard()?;
    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("lỗi tham số: {e}");
            std::process::exit(1);
        }
    };

    let root = resolve_sidecar_root();
    let data = std::env::temp_dir().join("riviu-live-nurture-test");
    std::fs::create_dir_all(&data)?;
    let artifacts = data.join("artifacts");
    std::fs::create_dir_all(&artifacts)?;
    let db = Arc::new(Database::open(data.join("riviu.db"))?);

    eprintln!("driver: loading settings");
    eprintln!("driver: creating sidecar driver");
    // Resolve exactly as the desktop app does — same keychain account, same
    // manifest. The harness used to run its own token lookup, which reads the
    // *legacy* account and never creates one, so a machine whose desktop app
    // holds a candidate token failed outright with "set RIVIU_RTMMO_TOKEN" —
    // and a machine that had both would have authenticated this run against an
    // agent the device is not running.
    let ResolvedAgentRuntime { driver_config, .. } = resolve_desktop_agent_runtime_with_candidate(
        root.clone(),
        data.clone(),
        &db,
        &CredentialStore::system()?,
        std::env::var("RIVIU_AGENT_TOKEN")
            .or_else(|_| std::env::var("RIVIU_RTMMO_TOKEN"))
            .ok()
            .as_deref(),
        false,
        true,
    )?;
    let bundle = create_driver(driver_config).await?;
    eprintln!("driver: created");
    let control = Arc::new(DeviceControlPlane::new(
        bundle.driver,
        Arc::new(DeviceWorkCoordinator::new()),
        Arc::new(StreamBudgetManager::default()),
    ));

    // The desktop sampler normally performs this install/auth-only preflight
    // before a nurture job is submitted.  The headless harness has no sampler,
    // so establish the protected relay explicitly while the repair context
    // owns the device; the nurture transition then reuses that relay and still
    // creates the WDA session before starting MJPEG.
    eprintln!("preflight: acquiring repair context");
    let preflight_context = control
        .try_acquire_exclusive(&args.udid, DeviceWorkOwner::Repair)
        .await?;
    eprintln!("preflight: checking protected agent");
    let preflight = tokio::time::timeout(
        Duration::from_secs(45),
        control.preflight_agent(&preflight_context),
    )
    .await
    .map_err(|_| anyhow::anyhow!("Agent preflight timed out after 45s"))??;
    if !preflight.auth_ready {
        anyhow::bail!(
            "Riviu Agent preflight chưa sẵn sàng: {}",
            preflight
                .message
                .unwrap_or_else(|| "protected auth unavailable".to_string())
        );
    }
    eprintln!(
        "Agent preflight OK: state={:?} auth={} session={} stream={}",
        preflight.state, preflight.auth_ready, preflight.session_ready, preflight.mjpeg_ready
    );
    // Release the short-lived repair lease before Nurture acquires its own
    // device owner. Keeping this context alive would queue the interaction
    // job forever behind the completed preflight.
    drop(preflight_context);
    let engine = NurtureEngine::new(
        db,
        control.clone(),
        Arc::new(bundle.streams.clone()),
        artifacts,
    )
    .with_frame_text_source(Arc::new(DesktopFrameTextSource));

    let mut settings = NurtureSettings {
        num_videos: args.videos,
        num_rounds: args.rounds,
        like_prob: args.like_prob,
        comment_prob: args.comment_prob,
        follow_prob: args.follow_prob,
        frenzy_prob: 0,
        watch_min: args.watch_min,
        watch_max: args.watch_max,
        night_start: 0,
        night_end: 0,
        steady_mood: args.steady.clone(),
        ..Default::default()
    };
    // Comments need a key; take it from the environment so it is never in the
    // repo or in a shell history line.
    if let Ok(key) = std::env::var("RIVIU_AI_API_KEY") {
        settings.api_key = key;
    }
    if args.comment_prob > 0 && settings.api_key.trim().is_empty() {
        eprintln!(
            "cảnh báo: --comment-prob {} nhưng RIVIU_AI_API_KEY trống — comment sẽ được bỏ qua",
            args.comment_prob
        );
    }

    if let Some(url) = args.open_url.as_deref() {
        open_url_on_device(&control, &args.udid, &settings.bundle_id, url).await?;
        eprintln!("opened TikTok URL fixture");
    }

    let stop = Arc::new(AtomicBool::new(false));
    // Ctrl-C must stop the run cleanly so the summary and the JSONL still land.
    {
        let stop = stop.clone();
        tokio::spawn(async move {
            let _ = tokio::signal::ctrl_c().await;
            eprintln!("\n-- nhận Ctrl-C, đang dừng --");
            stop.store(true, std::sync::atomic::Ordering::Relaxed);
        });
    }

    eprintln!(
        "LIVE NURTURE udid={} videos={} rounds={} like={}% comment={}% follow={}% watch={}–{}s max={}m",
        &args.udid[..8.min(args.udid.len())],
        args.videos,
        args.rounds,
        args.like_prob,
        args.comment_prob,
        args.follow_prob,
        args.watch_min,
        args.watch_max,
        args.minutes
    );

    let started = Instant::now();
    let status = match engine
        .run_session(
            &args.udid,
            settings,
            stop,
            Some(Duration::from_secs(args.minutes * 60)),
            |st| {
                // Attempted/confirmed pairs, not bare successes. A run that
                // reports 10 videos out of 40 swipes is telling you something a
                // bare "videos=10" cannot.
                eprintln!(
                    "[{:>6.1}s] videos={}/{} likes={}/{} comments={}/{} follows={}/{} | {}",
                    started.elapsed().as_secs_f64(),
                    st.videos_done,
                    st.swipe_attempts,
                    st.likes,
                    st.like_attempts,
                    st.comments,
                    st.comment_attempts,
                    st.follows,
                    st.follow_attempts,
                    st.last_message
                );
            },
        )
        .await
    {
        Ok(status) => status,
        Err(error) => {
            control.shutdown_cleanup().await?;
            return Err(error);
        }
    };
    let elapsed = started.elapsed();

    eprintln!("\n──────── WDA latency ────────");
    for line in telemetry::summary_lines() {
        eprintln!("  {line}");
    }
    let (slow_endpoint, slow_ms) = telemetry::slowest_request();
    eprintln!("  chậm nhất: {slow_endpoint} {slow_ms}ms");
    let failures = telemetry::failure_counts();
    if failures.is_empty() {
        eprintln!("  không có request lỗi");
    } else {
        let mut parts: Vec<String> = failures
            .iter()
            .map(|(k, v)| format!("{}={v}", k.as_str()))
            .collect();
        parts.sort();
        eprintln!("  lỗi: {}", parts.join(" "));
    }
    let events = telemetry::events();
    if !events.is_empty() {
        eprintln!("──────── recovery / launch ────────");
        for (kind, detail, ms) in &events {
            eprintln!("  {kind:<16} {ms:>6}ms  {detail}");
        }
    }

    eprintln!(
        "\nLIVE NURTURE kết thúc sau {:.1}s — {}",
        elapsed.as_secs_f64(),
        status.last_message
    );

    if let Some(path) = &args.jsonl {
        if let Err(error) = write_jsonl(path, &args, &status, elapsed, slow_ms) {
            control.shutdown_cleanup().await?;
            return Err(error);
        }
        eprintln!("JSONL: {}", path.display());
    }

    // A run has to have achieved something to count as a pass.
    let recoveries = events
        .iter()
        .filter(|(k, _, _)| k == "hard_recycle" || k == "relay_restart")
        .count();
    let exit_code = if status.videos_done == 0 {
        eprintln!("KHÔNG ĐẠT: 0 video");
        2
    } else if status.last_message.starts_with("failed")
        || status.last_message.starts_with("partial")
    {
        eprintln!("KHÔNG ĐẠT: phiên kết thúc không trọn vẹn");
        2
    } else if recoveries > 1 {
        eprintln!("KHÔNG ĐẠT: {recoveries} lần recovery nặng");
        2
    } else {
        0
    };
    control.shutdown_cleanup().await?;
    if exit_code != 0 {
        std::process::exit(exit_code);
    }
    Ok(())
}

async fn open_url_on_device(
    control: &DeviceControlPlane,
    udid: &str,
    bundle_id: &str,
    url: &str,
) -> anyhow::Result<()> {
    if !url.starts_with("https://") || url.trim() != url {
        anyhow::bail!("--open-url chỉ nhận HTTPS URL canonical");
    }
    let exclusive = control
        .try_acquire_exclusive(udid, DeviceWorkOwner::Interaction)
        .await?;
    let (exclusive, capacity) = control.reserve_ui_capacity(exclusive).await?;
    let session = control
        .start_interaction_session(exclusive, bundle_id, InteractionSessionKind::Ordinary)
        .await?;
    let stream = control.start_reserved_stream(session, capacity).await?;
    let ui = control.streaming_session(&stream)?;
    let result = async {
        ui.open_url(url).await?;
        tokio::time::sleep(Duration::from_millis(1200)).await;
        Ok::<(), anyhow::Error>(())
    }
    .await;
    let cleanup = control.close_ui_context(stream).await;
    result?;
    cleanup?;
    Ok(())
}

fn write_jsonl(
    path: &PathBuf,
    args: &Args,
    status: &riviu_core::NurtureSessionStatus,
    elapsed: Duration,
    slowest_ms: u32,
) -> anyhow::Result<()> {
    use std::io::Write;
    let mut f = std::fs::File::create(path)?;
    writeln!(
        f,
        "{}",
        serde_json::json!({
            "kind": "run",
            "udid": args.udid,
            "elapsed_s": elapsed.as_secs_f64(),
            "videos": status.videos_done,
            "swipe_attempts": status.swipe_attempts,
            "likes": status.likes,
            "like_attempts": status.like_attempts,
            "comments": status.comments,
            "comment_attempts": status.comment_attempts,
            "follows": status.follows,
            "follow_attempts": status.follow_attempts,
            "prompt_tokens": status.session_prompt_tokens,
            "completion_tokens": status.session_completion_tokens,
            "summary": status.last_message,
            "slowest_request_ms": slowest_ms,
        })
    )?;
    for l in telemetry::summary_lines() {
        writeln!(
            f,
            "{}",
            serde_json::json!({ "kind": "endpoint", "line": l })
        )?;
    }
    for (k, detail, ms) in telemetry::events() {
        writeln!(
            f,
            "{}",
            serde_json::json!({ "kind": "event", "event": k, "ms": ms, "detail": detail })
        )?;
    }
    Ok(())
}

fn resolve_sidecar_root() -> PathBuf {
    if let Ok(p) = std::env::var("RIVIU_SIDECAR_ROOT") {
        return PathBuf::from(p);
    }
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest
        .join("../../..")
        .join("sidecars")
        .canonicalize()
        .unwrap_or_else(|_| manifest.join("../../../sidecars"))
}
