//! Live nurture smoke test against a plugged-in iPhone.
//!
//!   cargo run -p riviu-ios-driver --example nurture_live -- \
//!     --udid <UDID> [--videos 3] [--watch-min 3] [--watch-max 6]
//!
//! Env:
//!   RIVIU_NURTURE_API_KEY / RIVIU_NURTURE_BASE_URL / RIVIU_NURTURE_MODEL — optional comment AI

use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Instant;

use riviu_core::db::Database;
use riviu_core::{
    AgentSettings, DeviceControlPlane, DeviceWorkCoordinator, NurtureEngine, NurtureSettings,
    StreamBudgetManager,
};
use riviu_ios_driver::{
    create_driver, AgentArtifact, AgentToken, DriverConfig, DriverTarget, UnifiedAgentConfig,
};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    riviu_ios_driver::install_process_tree_guard()?;
    let mut udid = String::new();
    let mut videos: u32 = 3;
    let mut watch_min = 3.0_f64;
    let mut watch_max = 6.0_f64;
    let mut like_prob: u32 = 70;
    let mut comment_prob: u32 = 0;
    let mut follow_prob: u32 = 10;
    let mut frenzy_prob: u32 = 15;
    let mut max_secs: u64 = 120;
    let mut bundle = "com.ss.iphone.ugc.Ame".to_string();

    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--udid" => udid = args.next().unwrap_or_default(),
            "--videos" => videos = args.next().and_then(|s| s.parse().ok()).unwrap_or(videos),
            "--watch-min" => {
                watch_min = args
                    .next()
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(watch_min)
            }
            "--watch-max" => {
                watch_max = args
                    .next()
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(watch_max)
            }
            "--like" => {
                like_prob = args
                    .next()
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(like_prob)
            }
            "--comment" => {
                comment_prob = args
                    .next()
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(comment_prob)
            }
            "--follow" => {
                follow_prob = args
                    .next()
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(follow_prob)
            }
            "--frenzy" => {
                frenzy_prob = args
                    .next()
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(frenzy_prob)
            }
            "--max-secs" => max_secs = args.next().and_then(|s| s.parse().ok()).unwrap_or(max_secs),
            "--bundle" => bundle = args.next().unwrap_or(bundle),
            other => eprintln!("ignore arg {other}"),
        }
    }

    let sidecar_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../sidecars")
        .canonicalize()?;
    let state_dir = std::env::temp_dir().join("riviu-ios-driver-live");
    let bundle_drv = create_driver(resolve_driver_config(&sidecar_root, &state_dir)?).await?;
    let devices = bundle_drv.driver.list_devices().await?;
    if devices.is_empty() {
        anyhow::bail!("no iPhone connected");
    }
    if udid.is_empty() {
        udid = devices[0].udid.clone();
    }
    let device = devices
        .iter()
        .find(|d| d.udid == udid)
        .ok_or_else(|| anyhow::anyhow!("udid {udid} not found"))?;
    println!(
        "device={} name={} ios={}",
        device.udid, device.name, device.os_version
    );

    let tmp = std::env::temp_dir().join(format!("riviu-nurture-live-{}", std::process::id()));
    std::fs::create_dir_all(tmp.join("artifacts"))?;
    let db = Arc::new(Database::open(tmp.join("live.db"))?);

    let mut settings = NurtureSettings {
        bundle_id: bundle,
        num_videos: videos,
        num_rounds: 4,
        watch_min,
        watch_max,
        like_prob,
        comment_prob,
        follow_prob,
        frenzy_prob,
        fatigue: false,
        time_of_day: false,
        pause_swipe: false,
        night_start: 0,
        night_end: 0,
        recover_delay_min: 0,
        recover_delay_max: 0,
        ..Default::default()
    };
    if let Ok(k) = std::env::var("RIVIU_NURTURE_API_KEY") {
        if !k.trim().is_empty() {
            settings.api_key = k;
            if comment_prob == 0 {
                settings.comment_prob = 30;
                settings.like_prob = 50;
            }
        }
    }
    if let Ok(u) = std::env::var("RIVIU_NURTURE_BASE_URL") {
        if !u.trim().is_empty() {
            settings.base_url = u;
        }
    }
    if let Ok(m) = std::env::var("RIVIU_NURTURE_MODEL") {
        if !m.trim().is_empty() {
            settings.model = m;
        }
    }

    println!(
        "settings videos={} watch={}-{}s like={} comment={} follow={} frenzy={} bundle={} ai={}",
        settings.num_videos,
        settings.watch_min,
        settings.watch_max,
        settings.like_prob,
        settings.comment_prob,
        settings.follow_prob,
        settings.frenzy_prob,
        settings.bundle_id,
        !settings.api_key.is_empty()
    );

    // The engine starts the stream itself: it reads the screen from MJPEG
    // frames rather than from WDA screenshots, and the per-device supervisor
    // makes sure only one stream and one relay exist for this UDID.
    let control = Arc::new(DeviceControlPlane::new(
        bundle_drv.driver,
        Arc::new(DeviceWorkCoordinator::new()),
        Arc::new(StreamBudgetManager::default()),
    ));
    let engine = NurtureEngine::new(
        db,
        control.clone(),
        Arc::new(bundle_drv.streams.clone()),
        tmp.join("artifacts"),
    );
    let stop = Arc::new(AtomicBool::new(false));

    let started = Instant::now();
    let status_result = engine
        .run_session(
            &udid,
            settings,
            stop,
            Some(std::time::Duration::from_secs(max_secs)),
            |st| {
                use std::io::Write;
                println!(
                    "[{:>5.1}s] running={} videos={} likes={} comments={} follows={} | {}",
                    started.elapsed().as_secs_f64(),
                    st.running,
                    st.videos_done,
                    st.likes,
                    st.comments,
                    st.follows,
                    st.last_message
                );
                let _ = std::io::stdout().flush();
            },
        )
        .await;
    control.shutdown_cleanup().await?;
    let status = status_result?;

    println!("---- RESULT ----");
    println!(
        "videos={} likes={} comments={} follows={} usd={:.4} msg={} elapsed={:.1}s",
        status.videos_done,
        status.likes,
        status.comments,
        status.follows,
        status.session_usd,
        status.last_message,
        started.elapsed().as_secs_f64()
    );
    if status.videos_done == 0 && status.last_message.starts_with("ensure failed") {
        anyhow::bail!("nurture live FAIL: {}", status.last_message);
    }
    if status.videos_done == 0 && status.last_message.contains("ui session") {
        anyhow::bail!("nurture live FAIL: {}", status.last_message);
    }
    Ok(())
}

fn resolve_driver_config(
    sidecar_root: &std::path::Path,
    state_dir: &std::path::Path,
) -> anyhow::Result<DriverConfig> {
    let target = if std::env::var("RIVIU_WDA_BACKEND")
        .map(|value| value.trim().eq_ignore_ascii_case("stock"))
        .unwrap_or(false)
    {
        DriverTarget::LegacyStock
    } else {
        let token = std::env::var("RIVIU_RTMMO_TOKEN")?;
        let mut artifact =
            AgentArtifact::load(sidecar_root.join("wda").join("agent-manifest.json"))?;
        if let Some(path) = std::env::var_os("RIVIU_RTMMO_IPA") {
            artifact.ipa_path = PathBuf::from(path);
        }
        artifact.verify_checksum()?;
        DriverTarget::Real(UnifiedAgentConfig {
            token: AgentToken::new(token)?,
            artifact,
            settings: AgentSettings::default(),
        })
    };

    Ok(DriverConfig {
        sidecar_root: sidecar_root.to_path_buf(),
        state_dir: state_dir.to_path_buf(),
        target,
    })
}
