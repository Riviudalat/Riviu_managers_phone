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

use riviu_core::db::Database;
use riviu_core::{
    AgentSettings, DeviceControlPlane, DeviceWorkCoordinator, NurtureEngine, NurtureSettings,
    StreamBudgetManager,
};
use riviu_ios_driver::{
    create_driver, telemetry, AgentArtifact, AgentToken, DriverConfig, DriverTarget,
    UnifiedAgentConfig,
};
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

    let agent_settings = db.get_agent_settings()?;
    let bundle = create_driver(resolve_driver_config(&root, &data, agent_settings)?).await?;
    let control = Arc::new(DeviceControlPlane::new(
        bundle.driver,
        Arc::new(DeviceWorkCoordinator::new()),
        Arc::new(StreamBudgetManager::default()),
    ));
    let engine = NurtureEngine::new(
        db,
        control.clone(),
        Arc::new(bundle.streams.clone()),
        artifacts,
    );

    let mut settings = NurtureSettings::default();
    settings.num_videos = args.videos;
    settings.num_rounds = args.rounds;
    settings.like_prob = args.like_prob;
    settings.comment_prob = args.comment_prob;
    settings.follow_prob = args.follow_prob;
    settings.frenzy_prob = 0;
    settings.watch_min = args.watch_min;
    settings.watch_max = args.watch_max;
    settings.night_start = 0;
    settings.night_end = 0;
    settings.steady_mood = args.steady.clone();
    // Comments need a key; take it from the environment so it is never in the
    // repo or in a shell history line.
    if let Ok(key) = std::env::var("RIVIU_AI_API_KEY") {
        settings.api_key = key;
    }
    if args.comment_prob > 0 && settings.api_key.trim().is_empty() {
        eprintln!(
            "cảnh báo: --comment-prob {} nhưng RIVIU_AI_API_KEY trống — sẽ dùng pool dựng sẵn",
            args.comment_prob
        );
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
                eprintln!(
                    "[{:>6.1}s] videos={} likes={} comments={} follows={} | {}",
                    started.elapsed().as_secs_f64(),
                    st.videos_done,
                    st.likes,
                    st.comments,
                    st.follows,
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
            "likes": status.likes,
            "comments": status.comments,
            "follows": status.follows,
            "usd": status.session_usd,
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

fn resolve_driver_config(
    sidecar_root: &std::path::Path,
    state_dir: &std::path::Path,
    settings: AgentSettings,
) -> anyhow::Result<DriverConfig> {
    let target = if std::env::var("RIVIU_MOCK_DEVICES")
        .map(|value| value == "1" || value.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
    {
        DriverTarget::Mock
    } else if std::env::var("RIVIU_WDA_BACKEND")
        .map(|value| value.trim().eq_ignore_ascii_case("stock"))
        .unwrap_or(false)
    {
        DriverTarget::LegacyStock
    } else {
        let credentials = CredentialStore::system()?;
        let env_token = std::env::var("RIVIU_RTMMO_TOKEN").ok();
        let token = credentials.agent_token_or_create(env_token.as_deref())?;
        let mut artifact =
            AgentArtifact::load(sidecar_root.join("wda").join("agent-manifest.json"))?;
        if let Some(path) = std::env::var_os("RIVIU_RTMMO_IPA") {
            artifact.ipa_path = PathBuf::from(path);
        }
        artifact.verify_checksum()?;
        DriverTarget::Real(UnifiedAgentConfig {
            token: AgentToken::new(token)?,
            artifact,
            settings,
        })
    };

    Ok(DriverConfig {
        sidecar_root: sidecar_root.to_path_buf(),
        state_dir: state_dir.to_path_buf(),
        target,
    })
}
