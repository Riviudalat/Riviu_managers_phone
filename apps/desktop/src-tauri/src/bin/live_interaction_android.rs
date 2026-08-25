//! Run one real Interaction campaign on real Android phones, the way the app runs it.
//!
//! ```text
//! RIVIU_ADB_PATH=… cargo run -p riviu-managers-phone --bin live_interaction_android -- \
//!   --url <post> --devices <serial,serial,…> --i-will-post [--instruction "…"]
//! ```
//!
//! **It posts public comments** under whichever accounts those phones are logged into, one per
//! device, and there is no undo. `--i-will-post` is required for exactly that reason: every
//! other harness in this crate is read-only or opt-in per feature, and a campaign runner that
//! started on a bare command line would eventually be started by accident.
//!
//! **Do not run this while the desktop app is open.** Two processes reaching for the same
//! phones is the contention this project spent a week removing.
//!
//! It exists because the batched-draft path cannot be judged any other way. Its cost and its
//! variety were measured through `carousel_comment`, which calls the same functions but sends
//! nothing; whether twenty — or five — phones actually *arrive, type and confirm* on one link
//! is a claim about hardware, and the only honest test of it is the campaign itself. Driving
//! the app by hand is not an option: a blind click once posted a real comment under the wrong
//! post.
//!
//! Settings come from the desktop app's database, **copied** into a scratch directory, so the
//! AI key never appears on a command line and a run here can never rewrite what the operator
//! configured. The campaign rows are written to the copy too, which is why the run prints its
//! own summary rather than telling the operator to look in the app.

use std::io::Write;
use std::sync::Arc;

use app_lib::interaction_ocr::DesktopFrameTextSource;
use riviu_core::db::{Database, SecretStore};
use riviu_core::driver::DeviceDriver;
use riviu_core::interaction::{
    parse_tiktok_links, plan_threads, ThreadCampaignRequest, ThreadMode, ThreadShape,
};
use riviu_core::interaction_campaign::execute_thread_campaign;
use riviu_core::{
    DeviceControlPlane, DeviceWorkCoordinator, EventBus, FlowArtifactStore, NurtureEngine,
    StreamBudgetManager, ThreadCampaignState,
};
use riviu_ios_driver::StreamHub;
use uuid::Uuid;

/// The same seam `AppState::bootstrap` uses, so the key comes from where the app put it.
///
/// **Copying the database is not enough and that is measured.** The AI key stopped living in
/// the settings blob when it moved to the OS credential store, so the first run of this harness
/// read `khoá API TRỐNG` off a faithful copy of the operator's own database and refused before
/// opening a phone. `live_nurture_android` still copies without this and inherits the same hole.
struct KeyringSecrets {
    credentials: riviu_signing::CredentialStore,
}

impl SecretStore for KeyringSecrets {
    fn get_secret(&self, name: &str) -> anyhow::Result<Option<String>> {
        self.credentials.app_secret(name)
    }
    fn set_secret(&self, name: &str, value: &str) -> anyhow::Result<()> {
        self.credentials.set_app_secret(name, value)
    }
}

fn say(line: &str) {
    println!("{line}");
    let _ = std::io::stdout().flush();
}

fn arg(name: &str) -> Option<String> {
    let args: Vec<String> = std::env::args().collect();
    args.iter()
        .position(|value| value == name)
        .and_then(|at| args.get(at + 1).cloned())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let Some(url) = arg("--url") else {
        say("usage: live_interaction_android --url <post> --devices <a,b,c> --i-will-post");
        return Ok(());
    };
    let devices: Vec<String> = arg("--devices")
        .unwrap_or_default()
        .split(',')
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .collect();
    if devices.is_empty() {
        say("--devices trống: harness này không tự chọn máy để đăng bình luận thật");
        return Ok(());
    }
    if !std::env::args().any(|value| value == "--i-will-post") {
        say(&format!(
            "sẽ đăng {} bình luận công khai lên {url} — thêm --i-will-post nếu đúng ý",
            devices.len()
        ));
        return Ok(());
    }
    let instruction = arg("--instruction").unwrap_or_else(|| "tự nhiên".to_string());

    let android = Arc::new(riviu_android_driver::AndroidDriver::new(
        &riviu_android_driver::AndroidDriverConfig::default(),
    )?);
    let streams = Arc::new(StreamHub::new());
    android.set_frame_sink(Arc::new(streams.as_ref().clone()));

    let attached = android.list_devices().await?;
    let missing: Vec<&String> = devices
        .iter()
        .filter(|serial| !attached.iter().any(|device| &device.udid == *serial))
        .collect();
    if !missing.is_empty() {
        say(&format!("không thấy máy: {missing:?}"));
        return Ok(());
    }

    let budget = match StreamBudgetManager::new(devices.len()) {
        Ok(budget) => budget,
        Err(error) => {
            say(&format!("stream budget từ chối: {error}"));
            return Ok(());
        }
    };

    let scratch = std::env::temp_dir().join(format!("riviu-live-interaction-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&scratch)?;
    let app_db = dirs::data_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("riviu-managers-phone")
        .join("riviu.db");
    if app_db.is_file() {
        std::fs::copy(&app_db, scratch.join("riviu.db"))?;
    } else {
        say("không thấy CSDL của app — không có khoá AI, campaign sẽ từ chối");
    }
    let db = Arc::new(
        Database::open(scratch.join("riviu.db"))?.with_secrets(Arc::new(KeyringSecrets {
            credentials: riviu_signing::CredentialStore::system()?,
        })),
    );
    let control = Arc::new(DeviceControlPlane::new(
        android.clone(),
        Arc::new(DeviceWorkCoordinator::new()),
        Arc::new(budget),
    ));
    let engine = NurtureEngine::new(
        db.clone(),
        control.clone(),
        Arc::new(streams.as_ref().clone()),
        scratch.join("artifacts"),
    )
    .with_frame_text_source(Arc::new(DesktopFrameTextSource));
    let events = EventBus::new(256);
    let artifacts = FlowArtifactStore::new(scratch.join("artifacts"))?;

    let settings = db.get_nurture_settings()?;
    say(&format!(
        "model {} @ {}, tối đa {} từ, khoá API {}",
        settings.model,
        settings.base_url,
        settings.max_comment_words,
        if settings.api_key.trim().is_empty() {
            "TRỐNG"
        } else {
            "có"
        }
    ));

    // Resolved through the same parser the app's link box uses, so a URL this harness
    // accepts is a URL the app would accept — and a refusal here is the refusal an operator
    // would have seen instead of a campaign that opened phones and then gave up.
    let lines = parse_tiktok_links(&url);
    let targets: Vec<_> = lines
        .iter()
        .filter_map(|line| line.target.clone())
        .collect();
    if targets.is_empty() {
        say(&format!(
            "link không dùng được: {:?}",
            lines.first().and_then(|line| line.error)
        ));
        return Ok(());
    }
    say(&format!(
        "link: {} ({:?}) của @{}",
        targets[0].normalized_url, targets[0].kind, targets[0].author
    ));

    // `Standalone`, because that is the shape the batched draft covers and the shape an
    // operator uses to put one comment per phone under one post.
    let request = ThreadCampaignRequest {
        request_id: Uuid::new_v4().to_string(),
        targets,
        actor_udids: devices.clone(),
        message_count: devices.len() as u8,
        instruction: instruction.clone(),
        max_words: 12,
        mode: ThreadMode::Standalone,
        shape: ThreadShape::default(),
        cohort_size: None,
        manual_comments: Vec::new(),
        like_target: false,
        mentions: Vec::new(),
        mention_parent: false,
    };
    if riviu_core::interaction_campaign::ai_key_missing(&request, &settings.api_key) {
        say("chưa có khoá AI — dừng trước khi mở máy nào");
        return Ok(());
    }
    let plan = plan_threads(&request).map_err(|error| anyhow::anyhow!("{error}"))?;
    say(&format!("kế hoạch: {} assignment", plan.assignments.len()));

    let campaign_id = db.create_interaction_campaign(&request, &plan)?;
    db.update_interaction_campaign_state(&campaign_id, ThreadCampaignState::Running, None)?;

    let frames: Arc<dyn riviu_core::GenerationFrameSource> = Arc::new(streams.as_ref().clone());
    let started = std::time::Instant::now();
    let outcome = execute_thread_campaign(
        db.clone(),
        control.clone(),
        engine.clone(),
        events.clone(),
        campaign_id.clone(),
        request,
        plan,
        None,
        artifacts,
        frames,
    )
    .await;
    match &outcome {
        Ok(()) => say(&format!(
            "\ncampaign xong sau {:.1}s",
            started.elapsed().as_secs_f64()
        )),
        Err(error) => say(&format!(
            "\ncampaign lỗi sau {:.1}s: {error:#}",
            started.elapsed().as_secs_f64()
        )),
    }

    // Read the rows back rather than trusting the return value: what matters is what each
    // assignment ended as, and the campaign's own error is only the first thing that broke.
    if let Some(detail) = db.get_interaction_campaign(&campaign_id)? {
        say(&format!("trạng thái: {:?}", detail.summary.state));
        for assignment in &detail.assignments {
            say(&format!(
                "  ordinal {:>2} {:<20} {:?}  {}{}",
                assignment.ordinal,
                assignment.actor_udid,
                assignment.state,
                assignment.prepared_text.as_deref().unwrap_or(""),
                assignment
                    .error_code
                    .as_deref()
                    .map(|code| format!("   [{code}]"))
                    .unwrap_or_default()
            ));
        }
    }
    say(&format!("\nCSDL của lượt chạy: {}", scratch.display()));
    Ok(())
}
