use crate::command_error::CommandError;
use crate::interaction_ocr;
use crate::state::AppState;
use riviu_core::parse_tiktok_links;
use riviu_core::plan_threads;
use riviu_core::AppEvent;
use riviu_core::DeviceControlPlane;
use riviu_core::InteractionCampaignDetail;
use riviu_core::InteractionCampaignSummary;
use riviu_core::ThreadCampaignRequest;
use riviu_core::ThreadCampaignState;
use riviu_core::ThreadPreview;
use riviu_core::TikTokLinkLine;
use serde::Serialize;
use std::sync::Arc;
use std::time::Duration;
use tauri::State;

use riviu_core::interaction_campaign::{
    execute_thread_campaign, open_interaction_context, open_target_confirmed,
    retryable_assignments, revision, InteractionDevice,
};

/// One device's TikTok build plus the context opened against it.
///

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InteractionStartResult {
    pub campaign: InteractionCampaignSummary,
    pub queued: bool,
}

fn interaction_error(error: impl std::fmt::Display) -> CommandError {
    CommandError::code("InteractionFailed", error.to_string())
}

#[tauri::command]
pub fn interaction_parse_links(raw_text: String) -> Vec<TikTokLinkLine> {
    parse_tiktok_links(&raw_text)
}

#[tauri::command]
pub async fn interaction_resolve_links(
    raw_text: String,
) -> Result<Vec<TikTokLinkLine>, CommandError> {
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::limited(5))
        .timeout(Duration::from_secs(12))
        .user_agent("RiviuManagersPhone/interaction")
        .build()
        .map_err(CommandError::operation)?;
    let mut resolved = Vec::new();
    for line in parse_tiktok_links(&raw_text) {
        let Some(error) = line.error else {
            resolved.push(line);
            continue;
        };
        if error != riviu_core::LinkErrorCode::UnresolvedShortLink {
            resolved.push(line);
            continue;
        }
        let response = client
            .get(&line.original)
            .header("Range", "bytes=0-0")
            .send()
            .await
            .map_err(|e| {
                interaction_error(format!("resolve short link line {}: {e}", line.line_no))
            })?
            .error_for_status()
            .map_err(|e| {
                interaction_error(format!("short link line {} returned {e}", line.line_no))
            })?;
        let final_url = response.url().to_string();
        let final_line = parse_tiktok_links(&final_url).into_iter().next();
        resolved.push(match final_line {
            Some(mut final_line) => {
                final_line.line_no = line.line_no;
                final_line.original = line.original;
                final_line
            }
            None => TikTokLinkLine {
                line_no: line.line_no,
                original: line.original,
                target: None,
                error: Some(riviu_core::LinkErrorCode::UnresolvedShortLink),
            },
        });
    }
    Ok(resolved)
}

#[tauri::command]
pub fn interaction_preview_thread(
    state: State<'_, AppState>,
    request: ThreadCampaignRequest,
) -> Result<ThreadPreview, CommandError> {
    let lines = request
        .targets
        .iter()
        .enumerate()
        .map(|(index, target)| TikTokLinkLine {
            line_no: index + 1,
            original: target.original_url.clone(),
            target: Some(target.clone()),
            error: None,
        })
        .collect::<Vec<_>>();
    let plan = plan_threads(&request).map_err(interaction_error)?;
    Ok(ThreadPreview {
        valid_target_count: request.targets.len() as u32,
        // Both from the real planner and the real budget, so the desktop stops maintaining a
        // second copy of the cohort split in TypeScript and can warn about a capacity the
        // campaign would actually hit.
        cohort_count: riviu_core::partition_actors(&request.actor_udids, request.cohort_size).len()
            as u32,
        stream_capacity: state.control.stream_capacity() as u32,
        lines,
        plan: Some(plan),
    })
}

/// Refuse a Threaded campaign the actors cannot actually carry out.
///
/// Two separate reasons, and the OCR one used to be the only one — stated as a
/// property of the *desktop host*, which it is not. It is a property of the
/// **actors**: a device that reads the accessibility tree never calls
/// `interaction_ocr` at all, so the host's OCR language is irrelevant to it.
///
/// The second reason is new and has no equivalent in the pixel-only code, because
/// the situation could not arise before. A thread is a linear chain and each message
/// is sent from a *different* actor, so message N must find message N-1's comment on
/// screen. A hierarchy actor stores an author label read from a node's `text`; a pixel
/// actor then has to re-find that row by OCR and match the author label
/// (`locate_parent_comment` requires it). The bodies will agree — both sides compare
/// a string this project typed — but the author labels may not: a badge, a
/// truncation, a rendered-versus-attribute difference. Refusing costs nothing today
/// because nobody is running mixed campaigns, and it is far cheaper than a chain that
/// breaks halfway with no explanation. Standalone is unaffected: it has no parent to
/// find.
fn require_parent_locator(
    control: &DeviceControlPlane,
    mode: riviu_core::ThreadMode,
    actor_udids: &[String],
) -> Result<(), CommandError> {
    // Standalone comments never look for a parent, so nothing has to be read back off
    // the screen and neither reason applies.
    if mode == riviu_core::ThreadMode::Standalone {
        return Ok(());
    }
    let (hierarchy, pixel): (Vec<&String>, Vec<&String>) = actor_udids
        .iter()
        .partition(|udid| control.reports_element_bounds(udid));

    if !hierarchy.is_empty() && !pixel.is_empty() {
        return Err(CommandError::code(
            "MixedPlatformThread",
            format!(
                "chuỗi bình luận lồng nhau không chạy trộn hai loại máy: {} máy đọc hierarchy                  ({}) và {} máy nhận dạng ảnh ({}). Hai bên đọc nhãn tác giả theo hai cách nên                  mắt xích có thể đứt giữa chừng. Dùng toàn iPhone, toàn Android, hoặc chuyển                  sang chế độ Standalone.",
                hierarchy.len(),
                hierarchy
                    .iter()
                    .map(|udid| udid.as_str())
                    .collect::<Vec<_>>()
                    .join(", "),
                pixel.len(),
                pixel
                    .iter()
                    .map(|udid| udid.as_str())
                    .collect::<Vec<_>>()
                    .join(", "),
            ),
        ));
    }

    // All-hierarchy: the parent is found in the tree and OCR is never invoked.
    if pixel.is_empty() {
        return Ok(());
    }
    if interaction_ocr::reads_vietnamese() {
        return Ok(());
    }
    let found = interaction_ocr::recognizer_language().unwrap_or_else(|| "không có".into());
    Err(CommandError::code(
        "OcrLanguageUnavailable",
        format!(
            "chuỗi bình luận trên máy nhận dạng ảnh ({}) cần OCR đọc được tiếng Việt để tìm              lại bình luận cha; máy này đọc bằng '{found}'. Windows.Media.Ocr không phát hành              gói tiếng Việt (35 gói, không có vi-VN), nên không có gì để cài — chạy chiến dịch              trên máy Mac, nơi helper Vision đã ghim sẵn vi-VN.",
            pixel
                .iter()
                .map(|udid| udid.as_str())
                .collect::<Vec<_>>()
                .join(", "),
        ),
    ))
}

#[tauri::command]
pub async fn interaction_start_thread(
    state: State<'_, AppState>,
    request: ThreadCampaignRequest,
) -> Result<InteractionStartResult, CommandError> {
    let admission = state.ensure_accepting_work()?;
    require_parent_locator(&state.control, request.mode, &request.actor_udids)?;
    let plan = plan_threads(&request).map_err(interaction_error)?;
    // Asked before anything is persisted. The engine checks this too, but by then the row
    // exists and the operator's history fills with campaigns that were Running for a second
    // and then Failed on a missing key — an AI campaign with no key never started, and the
    // list should not claim it did. A manual campaign passes: it never calls the AI.
    if riviu_core::interaction_campaign::ai_key_missing(
        &request,
        &state
            .db
            .get_nurture_settings()
            .map_err(CommandError::operation)?
            .api_key,
    ) {
        return Err(CommandError::code(
            "AiKeyMissing",
            "chưa cấu hình AI API key — dùng bình luận thủ công hoặc nhập key trong Nuôi TT",
        ));
    }
    let campaign_id = state
        .db
        .create_interaction_campaign(&request, &plan)
        .map_err(CommandError::operation)?;
    state
        .db
        .update_interaction_campaign_state(&campaign_id, ThreadCampaignState::Running, None)
        .map_err(CommandError::operation)?;
    state.events.emit(AppEvent::InteractionUpdated {
        campaign_id: campaign_id.clone(),
        revision: revision(),
    });
    let campaign = state
        .db
        .get_interaction_campaign(&campaign_id)
        .map_err(CommandError::operation)?
        .ok_or_else(|| interaction_error("campaign disappeared after persistence"))?
        .summary;

    let db = state.db.clone();
    let control = state.control.clone();
    let engine = state.nurture_engine.clone();
    let events = state.events.clone();
    let artifacts = state.interaction_artifacts.clone();
    // The concrete hub, because only it can answer a generation-qualified read.
    let frames: Arc<dyn riviu_core::GenerationFrameSource> = Arc::new(state.streams.clone());
    tauri::async_runtime::spawn(async move {
        let _worker_admission = admission;
        if let Err(error) = execute_thread_campaign(
            db.clone(),
            control,
            engine,
            events.clone(),
            campaign_id.clone(),
            request,
            plan,
            None,
            artifacts,
            frames,
        )
        .await
        {
            // `{:#}` not `to_string()`: anyhow keeps the cause chain and `to_string`
            // returns only the outermost context, which is how a live AI failure came
            // to record `AI chuẩn bị assignment 0` and nothing about the HTTP status
            // behind it. Logged too, because this is the only place the reason exists.
            let detail = format!("{error:#}");
            log::error!("interaction campaign thất bại: {detail}");
            // Not an unconditional `Failed`: the engine may already have written `Partial`
            // off the campaign's own totals, and stomping that is how a run with real posted
            // comments under it got filed as a total loss. See
            // `Database::fail_interaction_campaign_unless_settled`.
            let _ = db.fail_interaction_campaign_unless_settled(&campaign_id, &detail);
            events.emit(AppEvent::InteractionUpdated {
                campaign_id,
                revision: revision(),
            });
        }
    });

    Ok(InteractionStartResult {
        campaign,
        queued: true,
    })
}

#[tauri::command]
pub fn interaction_list(
    state: State<'_, AppState>,
    limit: Option<usize>,
) -> Result<Vec<InteractionCampaignSummary>, CommandError> {
    state
        .db
        .list_interaction_campaigns(limit.unwrap_or(30).clamp(1, 100))
        .map_err(CommandError::operation)
}

#[tauri::command]
pub fn interaction_get(
    state: State<'_, AppState>,
    campaign_id: String,
) -> Result<Option<InteractionCampaignDetail>, CommandError> {
    state
        .db
        .get_interaction_campaign(&campaign_id)
        .map_err(CommandError::operation)
}

#[tauri::command]
pub fn interaction_cancel(
    state: State<'_, AppState>,
    campaign_id: String,
) -> Result<(), CommandError> {
    let _admission = state.ensure_accepting_work()?;
    state
        .db
        .update_interaction_campaign_state(&campaign_id, ThreadCampaignState::Cancelled, None)
        .map_err(CommandError::operation)?;
    state.events.emit(AppEvent::InteractionUpdated {
        campaign_id,
        revision: revision(),
    });
    Ok(())
}

#[tauri::command]
pub fn interaction_retry(
    state: State<'_, AppState>,
    campaign_id: String,
    assignment_ids: Option<Vec<String>>,
) -> Result<(), CommandError> {
    let requested: Option<std::collections::HashSet<String>> =
        assignment_ids.map(|ids| ids.into_iter().collect());
    let admission = state.ensure_accepting_work()?;
    let detail = state
        .db
        .get_interaction_campaign(&campaign_id)
        .map_err(CommandError::operation)?
        .ok_or_else(|| CommandError::code("InteractionNotFound", "campaign không tồn tại"))?;
    // Retry used to refuse the whole campaign the moment *one* message reached
    // Succeeded — which is precisely the state a broken chain produces (one
    // posted, the rest skipped), so the failure mode was also unrepairable.
    //
    // Scope it per assignment instead. Anything already sent, or whose delivery
    // is unproven, stays out: tapping Send is not idempotent and a second run
    // would post the comment twice.
    let retryable = retryable_assignments(&detail.assignments, requested.as_ref());
    if retryable.is_empty() {
        return Err(CommandError::code(
            "RetryNotAllowed",
            "không có tin nào gửi lại được: mọi tin đã đăng, đang gửi, hoặc ở trạng thái không chắc chắn",
        ));
    }
    // Every refusal below this point used to sit *after* a write of `Queued`, and a refused
    // retry therefore left the campaign parked there — a state the Monitor shows neither a
    // Cancel button for (running only) nor a Retry button for (partial/failed/cancelled
    // only), so the campaign became unreachable from the UI by asking to repair it. Read the
    // whole request and judge it first; the state moves only once, and only once nothing can
    // still say no.
    let (request, plan) = state
        .db
        .get_interaction_campaign_request(&campaign_id)
        .map_err(CommandError::operation)?
        .ok_or_else(|| {
            CommandError::code("InteractionNotFound", "campaign request không tồn tại")
        })?;
    // The mode is whatever the campaign was created with, so the reader
    // requirement has to be judged against that rather than a fresh choice.
    require_parent_locator(&state.control, request.mode, &request.actor_udids)?;
    // A manual campaign never calls the AI, so only an AI one needs the key — and asking here
    // rather than inside the worker means the refusal reaches the operator as a refusal
    // instead of as a campaign that flipped to Failed a second after they pressed retry.
    if riviu_core::interaction_campaign::ai_key_missing(
        &request,
        &state
            .db
            .get_nurture_settings()
            .map_err(CommandError::operation)?
            .api_key,
    ) {
        return Err(CommandError::code(
            "AiKeyMissing",
            "chưa cấu hình AI API key — dùng bình luận thủ công hoặc nhập key trong Nuôi TT",
        ));
    }
    state
        .db
        .update_interaction_campaign_state(&campaign_id, ThreadCampaignState::Running, None)
        .map_err(CommandError::operation)?;
    let db = state.db.clone();
    let control = state.control.clone();
    let engine = state.nurture_engine.clone();
    let events = state.events.clone();
    let artifacts = state.interaction_artifacts.clone();
    // The concrete hub, because only it can answer a generation-qualified read.
    let frames: Arc<dyn riviu_core::GenerationFrameSource> = Arc::new(state.streams.clone());
    let worker_id = campaign_id.clone();
    tauri::async_runtime::spawn(async move {
        let _worker_admission = admission;
        if let Err(error) = execute_thread_campaign(
            db.clone(),
            control,
            engine,
            events.clone(),
            worker_id.clone(),
            request,
            plan,
            Some(retryable),
            artifacts,
            frames,
        )
        .await
        {
            // `{:#}` not `to_string()`: anyhow keeps the cause chain and `to_string`
            // returns only the outermost context, which is how a live AI failure came
            // to record `AI chuẩn bị assignment 0` and nothing about the HTTP status
            // behind it. Logged too, because this is the only place the reason exists.
            let detail = format!("{error:#}");
            log::error!("interaction campaign thất bại: {detail}");
            // Same reason as the start path: keep a terminal verdict, record the reason.
            let _ = db.fail_interaction_campaign_unless_settled(&worker_id, &detail);
            // The start path has always emitted here; retry did not, so a retry that died in
            // the worker left the Monitor showing "Đang chạy" until something else happened
            // to refresh it. The state write above is invisible without this.
            events.emit(AppEvent::InteractionUpdated {
                campaign_id: worker_id,
                revision: revision(),
            });
        }
    });
    state.events.emit(AppEvent::InteractionUpdated {
        campaign_id,
        revision: revision(),
    });
    Ok(())
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InteractionArtifactPayload {
    pub id: String,
    pub kind: String,
    pub mime_type: String,
    pub base64: String,
}

/// List the frames saved for a campaign so the operator can see what the phone
/// actually showed. Rows written before evidence storage have no file.
#[tauri::command]
pub fn interaction_list_artifacts(
    state: State<'_, AppState>,
    campaign_id: String,
) -> Result<Vec<riviu_core::InteractionArtifactRecord>, CommandError> {
    state
        .db
        .list_interaction_artifacts(&campaign_id)
        .map_err(CommandError::operation)
}

#[tauri::command]
pub fn interaction_read_artifact(
    state: State<'_, AppState>,
    artifact_id: String,
) -> Result<InteractionArtifactPayload, CommandError> {
    let record = state
        .db
        .get_interaction_artifact(&artifact_id)
        .map_err(CommandError::operation)?
        .ok_or_else(|| CommandError::code("ArtifactNotFound", "không có artifact này"))?;
    let relative = record.relative_path.as_deref().ok_or_else(|| {
        CommandError::code(
            "ArtifactNotStored",
            "artifact này được ghi trước khi có lưu ảnh, không có file kèm theo",
        )
    })?;
    let bytes = state
        .interaction_artifacts
        .read_published(relative, &record.sha256)
        .map_err(|error| {
            log::error!("interaction artifact validation failed: {error:#}");
            CommandError::code("ArtifactUnreadable", "ảnh bằng chứng không đọc/kiểm được")
        })?;
    Ok(InteractionArtifactPayload {
        id: record.id,
        kind: record.kind,
        mime_type: "image/jpeg".into(),
        base64: base64::Engine::encode(&base64::engine::general_purpose::STANDARD, bytes),
    })
}

#[tauri::command]
pub async fn interaction_open_on_device(
    state: State<'_, AppState>,
    udid: String,
    url: String,
) -> Result<(), CommandError> {
    let _admission = state.ensure_accepting_work()?;
    let parsed = parse_tiktok_links(&url);
    let target = parsed
        .first()
        .and_then(|line| line.target.clone())
        .ok_or_else(|| CommandError::invalid_argument("link TikTok phải là video/photo URL"))?;
    let InteractionDevice {
        context,
        target_package,
    } = open_interaction_context(&state.control, &udid).await?;
    let session = state
        .control
        .streaming_session(&context)
        .map_err(CommandError::from)?;
    let result = async {
        let _proof = open_target_confirmed(
            &state.nurture_engine,
            &udid,
            session.as_ref(),
            &target,
            &target_package,
        )
        .await?;
        Ok::<(), anyhow::Error>(())
    }
    .await;
    let cleanup = state.control.close_ui_context(context).await;
    result.map_err(CommandError::operation)?;
    cleanup.map(|_| ()).map_err(CommandError::from)
}
