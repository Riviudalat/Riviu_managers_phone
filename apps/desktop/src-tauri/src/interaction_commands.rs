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

/// What one phone read off a post, and what that means for the numbers asked for.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InteractionPostReading {
    /// What the post says about itself right now. A `null` field is one this build or this
    /// screen could not state — never a zero standing in for "unknown".
    pub now: riviu_core::interaction_threshold::PostNow,
    /// What it would take to reach the targets, or why it cannot be reached.
    pub plan: riviu_core::interaction_threshold::ThresholdPlan,
    /// Whether the view count was asked for. It is the expensive half; see `read_post_now`.
    pub views_read: bool,
}

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
    require_parent_locator(
        &state.control,
        request.mode,
        request.actions,
        &request.actor_udids,
    )?;
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

/// Refuse an Interaction campaign the actors cannot actually carry out.
///
/// Save is desired-state, so it may only run on an actor that reports element bounds: that is
/// the preflight signal for the hierarchy adapter which reads `checked`/`selected`. A pixel actor
/// cannot distinguish Saved from Unsaved yet. Refusing it here is a zero-effect failure rather
/// than letting a worker discover the missing proof after the campaign was persisted/dispatched.
///
/// Threaded comments have two separate reasons, and the OCR one used to be the only one — stated as a
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
pub(crate) fn require_parent_locator(
    control: &DeviceControlPlane,
    mode: riviu_core::ThreadMode,
    actions: riviu_core::InteractionActionSet,
    actor_udids: &[String],
) -> Result<(), CommandError> {
    if actions.save {
        let unsupported = actor_udids
            .iter()
            .filter(|udid| !control.reports_element_bounds(udid))
            .map(String::as_str)
            .collect::<Vec<_>>();
        if !unsupported.is_empty() {
            return Err(CommandError::code(
                "SaveUnsupportedDevice",
                format!(
                    "Lưu bài cần đọc trạng thái Saved/Unsaved từ hierarchy; các máy chưa hỗ trợ: {}. Bỏ chọn Lưu hoặc chỉ chọn máy Android có hierarchy.",
                    unsupported.join(", ")
                ),
            ));
        }
    }

    if !actions.comment {
        return Ok(());
    }
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
    require_parent_locator(
        &state.control,
        request.mode,
        request.actions,
        &request.actor_udids,
    )?;
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

/// Read one post's numbers from one phone, and say what the targets would take.
///
/// **Manual on purpose.** Likes and comments are two label reads on a page already open, but a
/// view count is a navigation — TikTok states a play count only on the author's profile grid,
/// and the grid says nothing about which post a tile is, so each candidate is opened and its
/// caption compared. Timed 24/08/2026: about four and a half minutes per reading for a post near
/// the top of the author's grid, and longer when it sits deeper — on top of a cold start. A panel
/// that did it on its own, on a link the operator had only just pasted, would take a phone away
/// for minutes without being asked; so `read_views` is a flag the operator sets and this command
/// only ever runs when a button is pressed.
///
/// Holds admission and takes the same exclusive lease a campaign does, because it drives a real
/// phone. `unliked` is the actor count: without like history the honest assumption is that none
/// of them has liked the post yet, which is the *largest* a like ceiling can be — so a like
/// target this refuses is one no fleet state could have reached.
#[tauri::command]
pub async fn interaction_measure_post(
    state: State<'_, AppState>,
    udid: String,
    target: riviu_core::ResolvedTikTokTarget,
    targets: riviu_core::interaction_threshold::PostTargets,
    actor_count: u32,
    read_views: bool,
) -> Result<InteractionPostReading, CommandError> {
    let _admission = state.ensure_accepting_work()?;
    let device = open_interaction_context(&state.control, &udid)
        .await
        .map_err(CommandError::operation)?;
    let session = state
        .control
        .streaming_session(&device.context)
        .map_err(CommandError::operation)?;
    let session = session.as_ref();
    if !session.supports_element_bounds() {
        return Err(interaction_error(
            "máy này đọc màn hình bằng ảnh, không đọc được số của bài — chọn một máy Android",
        ));
    }
    let language = session.ui_language().await.unwrap_or_default();
    let app_version = session
        .app_version(&device.target_package)
        .await
        .unwrap_or_default();
    let labels =
        riviu_core::tiktok_labels::controls_for(&device.target_package, &language, &app_version)
            .ok_or_else(|| {
                interaction_error(format!(
                    "chưa đo nhãn cho {} + {language:?}",
                    device.target_package
                ))
            })?;
    let screen = session.window_size().await.map_err(interaction_error)?;
    let stop = std::sync::atomic::AtomicBool::new(false);
    // **Pressing Đo bài twice must not report the post as gone.** The arrival check decides it
    // got there by watching the author label *change*, so a phone already sitting on this post
    // gives it nothing to observe and it refuses `target_open_screen_unchanged` — a message that
    // reads "bài đã bị xoá/riêng tư/chặn vùng". The second press is the ordinary case: the first
    // one left the phone right there.
    //
    // The answer is a real second arrival, not a relaxed first one. The gate is what keeps a
    // campaign from commenting under the wrong post, so it is never loosened; instead the phone
    // is stepped off the post — Back from a deep-linked post leaves the feed, or leaves the app,
    // and arriving is visible from either — and asked again. Once only, and any other refusal is
    // returned as it came.
    //
    // Back rather than a force-stop: `device_shell` requires a lease context and stopping the
    // app underneath a live session and stream would break both, to save an arrival that costs
    // one screen change.
    let mut arrival = riviu_core::interaction_hierarchy::open_target_by_hierarchy(
        session,
        labels,
        &device.target_package,
        &target.normalized_url,
        &target.author,
        &stop,
    )
    .await;
    if arrival
        .as_ref()
        .err()
        .is_some_and(|refusal| refusal.code() == "target_open_screen_unchanged")
    {
        session.back().await.map_err(interaction_error)?;
        tokio::time::sleep(Duration::from_millis(2_500)).await;
        arrival = riviu_core::interaction_hierarchy::open_target_by_hierarchy(
            session,
            labels,
            &device.target_package,
            &target.normalized_url,
            &target.author,
            &stop,
        )
        .await;
    }
    arrival.map_err(|refusal| CommandError::code("InteractionFailed", refusal.message()))?;
    let now = riviu_core::interaction_hierarchy::read_post_now(
        session, labels, screen, read_views, &stop,
    )
    .await;
    let plan =
        riviu_core::interaction_threshold::plan_thresholds(targets, now, actor_count, actor_count);
    Ok(InteractionPostReading {
        now,
        plan,
        views_read: read_views,
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
        .cancel_interaction_campaign(&campaign_id)
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
    require_parent_locator(
        &state.control,
        request.mode,
        request.actions,
        &request.actor_udids,
    )?;
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
/// What the web lookup learned about every target of one campaign.
///
/// Read-only, DB-only. Registered, allowlisted **and called from `api.ts`** — the third of
/// those is the one AGENTS.md 9.103 §4 is about, and a command that skips it is a column
/// nobody can audit.
#[tauri::command]
pub fn interaction_list_target_notes(
    state: State<'_, AppState>,
    campaign_id: String,
) -> Result<Vec<riviu_core::InteractionTargetNote>, CommandError> {
    state
        .db
        .list_interaction_target_notes(&campaign_id)
        .map_err(CommandError::operation)
}

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

#[cfg(test)]
mod tests {
    use super::*;
    use riviu_core::{
        DeviceDriver, DeviceWorkCoordinator, InteractionActionSet, StreamBudgetManager, ThreadMode,
    };
    use riviu_ios_driver::MockIosDriver;

    fn pixel_control() -> DeviceControlPlane {
        let driver: Arc<dyn DeviceDriver> = Arc::new(MockIosDriver::new());
        DeviceControlPlane::new(
            driver,
            Arc::new(DeviceWorkCoordinator::new()),
            Arc::new(StreamBudgetManager::default()),
        )
    }

    #[tokio::test]
    async fn save_preflight_rejects_every_pixel_actor_with_a_typed_zero_effect_error() {
        let control = pixel_control();
        let actors = vec!["iphone-02".to_string(), "iphone-07".to_string()];
        let error = require_parent_locator(
            &control,
            ThreadMode::Standalone,
            InteractionActionSet {
                like: true,
                comment: false,
                save: true,
            },
            &actors,
        )
        .expect_err("pixel Save must fail before a campaign can be dispatched");

        assert_eq!(error.code, "SaveUnsupportedDevice");
        assert!(error.message.contains("iphone-02"));
        assert!(error.message.contains("iphone-07"));
        assert!(error.message.contains("Saved/Unsaved"));
    }

    #[tokio::test]
    async fn save_gate_is_inert_when_save_is_not_requested() {
        let control = pixel_control();
        require_parent_locator(
            &control,
            ThreadMode::Standalone,
            InteractionActionSet {
                like: true,
                comment: false,
                save: false,
            },
            &["iphone-02".to_string()],
        )
        .expect("a Like-only campaign does not require hierarchy Save state");
    }

    #[test]
    fn preview_start_and_retry_all_gate_save_before_their_first_effectful_step() {
        let source = include_str!("interaction_commands.rs");
        for (command, effect_marker) in [
            ("interaction_preview_thread", "plan_threads(&request)"),
            (
                "interaction_start_thread",
                "create_interaction_campaign(&request, &plan)",
            ),
            (
                "interaction_retry",
                "update_interaction_campaign_state(&campaign_id",
            ),
        ] {
            let start = source
                .find(&format!("fn {command}("))
                .expect("command remains registered");
            let body = &source[start..];
            let gate = body
                .find("require_parent_locator(")
                .expect("command must call capability gate");
            let effect = body
                .find(effect_marker)
                .unwrap_or_else(|| panic!("{command} must retain {effect_marker}"));
            assert!(
                gate < effect,
                "{command} must reject unsupported Save before {effect_marker}"
            );
        }
    }
}
