//! Live, draft-only video understanding comparison. No public action sender is used.
use anyhow::Context;
use riviu_core::driver::DeviceDriver;
use riviu_core::interaction_hierarchy::{open_target_by_hierarchy, SlideCamera, TargetArrival};
use riviu_core::video_evidence::{collect_video_evidence, VideoSample};
use riviu_core::{DeviceControlPlane, DeviceWorkCoordinator, FrameSource, StreamBudgetManager};
use std::path::{Path, PathBuf};
use std::sync::{atomic::AtomicBool, Arc};
use std::time::Duration;

struct FreshCamera {
    frames: Arc<riviu_ios_driver::StreamHub>,
    serial: String,
}
#[async_trait::async_trait]
impl SlideCamera for FreshCamera {
    async fn capture(&self) -> Option<Vec<u8>> {
        let mut stream = FrameSource::subscribe(self.frames.as_ref(), &self.serial);
        tokio::time::timeout(Duration::from_secs(5), stream.next())
            .await
            .ok()??
            .as_ref()
            .clone()
            .into()
    }
}

fn save(path: &Path, data: &impl serde::Serialize) -> anyhow::Result<()> {
    std::fs::write(path, serde_json::to_vec_pretty(data)?)?;
    Ok(())
}

async fn capture(serial: &str, url: &str, out: &Path) -> anyhow::Result<()> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../sidecars/android");
    let noarch = root.join("noarch");
    let driver = Arc::new(riviu_android_driver::AndroidDriver::new(
        &riviu_android_driver::AndroidDriverConfig {
            bundled_adb_path: Some(root.join("win-x86_64/adb.exe")),
            bundled_minicap_apk: Some(noarch.join("minicap.apk")),
            bundled_scrcpy_server: Some(noarch.join("scrcpy-server")),
            bundled_riviu_agent_apk: Some(noarch.join("riviu-agent.apk")),
            bundled_agent_server_apk: Some(noarch.join("appium-uiautomator2-server.apk")),
            bundled_agent_test_apk: Some(noarch.join("appium-uiautomator2-server-test.apk")),
            ..Default::default()
        },
    )?);
    let frames = Arc::new(riviu_ios_driver::StreamHub::new());
    driver.set_frame_sink(Arc::new(frames.as_ref().clone()));
    let control = DeviceControlPlane::new(
        driver.clone(),
        Arc::new(DeviceWorkCoordinator::new()),
        Arc::new(StreamBudgetManager::new(1)?),
    );
    let package = driver.resolve_tiktok_package(serial).await?;
    let exclusive = control
        .try_acquire_exclusive(serial, riviu_core::DeviceWorkOwner::Nurture)
        .await?;
    let (exclusive, capacity) = control.reserve_ui_capacity(exclusive).await?;
    let ui = control
        .start_interaction_session(
            exclusive,
            &package,
            riviu_core::InteractionSessionKind::Ordinary,
        )
        .await?;
    let context = control.start_reserved_stream(ui, capacity).await?;
    let result: anyhow::Result<()> = async {
        let session = control.streaming_session(&context)?;
        let version = session
            .app_version(&package)
            .await
            .context("version unavailable")?;
        let language = session.ui_language().await.context("locale unavailable")?;
        let labels = riviu_core::tiktok_labels::controls_for(&package, &language, &version)
            .context("unmeasured TikTok build")?;
        let parsed = reqwest::Url::parse(url)?;
        anyhow::ensure!(
            parsed.scheme() == "https" && parsed.host_str() == Some("www.tiktok.com"),
            "canonical TikTok URL required"
        );
        let handle = parsed
            .path_segments()
            .and_then(|mut p| p.next())
            .and_then(|v| v.strip_prefix('@'))
            .context("canonical author required")?;
        let stop = AtomicBool::new(false);
        let arrival =
            open_target_by_hierarchy(session.as_ref(), labels, &package, url, handle, &stop)
                .await
                .map_err(|error| anyhow::anyhow!("arrival: {}", error.code()))?;
        anyhow::ensure!(
            matches!(arrival, TargetArrival::Identified { .. }),
            "target author not proven"
        );
        let identity =
            riviu_core::video_evidence::read_video_identity(session.as_ref(), &package).await?;
        println!(
            "arrival=identified; author={}; package={package}; version={version}",
            identity.author
        );
        let camera = FreshCamera {
            frames: frames.clone(),
            serial: serial.to_owned(),
        };
        let baseline = collect_video_evidence(
            session.as_ref(),
            &camera,
            &package,
            Duration::from_millis(1200),
            3,
            &stop,
        )
        .await?;
        let improved = collect_video_evidence(
            session.as_ref(),
            &camera,
            &package,
            Duration::from_secs(48),
            12,
            &stop,
        )
        .await?;
        anyhow::ensure!(
            baseline.identity.matches(&improved.identity),
            "card changed between comparison windows"
        );
        for (name, evidence) in [("baseline", baseline), ("improved", improved)] {
            let dir = out.join(name);
            std::fs::create_dir(&dir)?;
            let mut records = Vec::new();
            for (index, sample) in evidence.samples.iter().enumerate() {
                let file = format!("frame-{index:02}.jpg");
                std::fs::write(dir.join(&file), &sample.frame)?;
                records.push(serde_json::json!({"file":file,"observedMs":sample.observed_ms}));
            }
            save(&dir.join("samples.json"), &records)?;
            save(&dir.join("identity.json"), &evidence.identity)?;
            println!(
                "{name}: frames={} spanMs={}",
                evidence.samples.len(),
                evidence.span_ms()
            );
        }
        Ok(())
    }
    .await;
    let termination = control.terminate_streaming_app(&context, &package).await;
    let closed = control.close_ui_context(context).await;
    let cleanup = control.shutdown_cleanup().await;
    save(
        &out.join("cleanup.json"),
        &serde_json::json!({
            "proof":termination.as_ref().ok(),"terminationError":termination.as_ref().err().map(ToString::to_string),
            "closeError":closed.as_ref().err().map(ToString::to_string),"cleanupError":cleanup.as_ref().err().map(ToString::to_string)
        }),
    )?;
    result?;
    termination?;
    closed?;
    cleanup?;
    Ok(())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    anyhow::ensure!(
        args.len() == 4,
        "usage: video_understanding <serial|replay> <canonical-url> <output-dir> <capture|analyze>"
    );
    let out = PathBuf::from(&args[2]);
    std::fs::create_dir_all(&out)?;
    let web = riviu_core::tiktok_web::fetch_post_context(&args[1])
        .await
        .map_err(|e| anyhow::anyhow!("web: {e}"))?;
    // The presence of offered subtitles is the useful signal, not the music category.
    let transcript = match web.transcript_track() {
        Some(track) => riviu_core::tiktok_web::fetch_transcript_with_limit(track, 1200).await,
        None => None,
    };
    save(
        &out.join("context.json"),
        &serde_json::json!({"url":args[1],"caption":web.caption,"durationSeconds":web.duration_secs,"transcript":transcript,"subtitleLanguages":web.subtitle_langs()}),
    )?;
    if args[3] == "capture" {
        return capture(&args[0], &args[1], &out).await;
    }
    anyhow::ensure!(
        args[0] == "replay" && args[3] == "analyze",
        "analyze reads an existing capture directory"
    );
    let db_path = dirs::data_dir()
        .context("data directory")?
        .join("riviu-managers-phone/riviu.db");
    let wal = PathBuf::from(format!("{}-wal", db_path.display()));
    anyhow::ensure!(
        !wal.exists() || wal.metadata()?.len() == 0,
        "close app before snapshot"
    );
    let temp =
        std::env::temp_dir().join(format!("riviu-understanding-{}.db", uuid::Uuid::new_v4()));
    std::fs::copy(&db_path, &temp)?;
    let db = riviu_core::db::Database::open(&temp)?;
    let raw = db
        .get_setting("nurture.settings")?
        .context("no stored AI settings")?;
    let mut settings: riviu_core::NurtureSettings = serde_json::from_str(&raw)?;
    settings.api_key = riviu_signing::CredentialStore::system()?
        .app_secret(riviu_core::db::SECRET_AI_API_KEY)?
        .unwrap_or_default();
    anyhow::ensure!(!settings.api_key.trim().is_empty(), "no configured AI key");
    drop(db);
    std::fs::remove_file(temp)?;
    println!(
        "provider={} model={}; draft_only=true",
        riviu_core::openai_client::host_of(&settings.base_url),
        settings.model
    );
    let mut failed = false;
    for name in ["baseline", "improved"] {
        let dir = out.join(name);
        let records: Vec<serde_json::Value> =
            serde_json::from_slice(&std::fs::read(dir.join("samples.json"))?)?;
        let samples = records
            .iter()
            .map(|record| {
                Ok(VideoSample {
                    observed_ms: record["observedMs"].as_u64().context("sample time")?,
                    frame: std::fs::read(
                        dir.join(record["file"].as_str().context("sample file")?),
                    )?,
                })
            })
            .collect::<anyhow::Result<Vec<_>>>()?;
        let enhanced = name == "improved";
        let result = riviu_core::openai_client::video_understanding::understand_video(
            &settings,
            &samples,
            if enhanced {
                web.caption.as_deref()
            } else {
                None
            },
            if enhanced {
                transcript.as_deref()
            } else {
                None
            },
        )
        .await;
        match result {
            Ok(result) => {
                save(&out.join(format!("{name}-analysis.json")), &result)?;
                println!("{name}: {}", serde_json::to_string(&result)?);
            }
            Err(error) => {
                save(
                    &out.join(format!("{name}-error.json")),
                    &serde_json::json!({"error":format!("{error:#}"),"spend":riviu_core::openai_client::spend_of_failure(&error).map(|spend|
                        serde_json::json!({"promptTokens":spend.prompt_tokens,"completionTokens":spend.completion_tokens,"costUsd":spend.cost_usd}))}),
                )?;
                eprintln!("{name} analysis rejected: {error:#}");
                failed = true;
            }
        }
    }
    anyhow::ensure!(
        !failed,
        "one or more analyses were rejected; see saved results"
    );
    Ok(())
}
