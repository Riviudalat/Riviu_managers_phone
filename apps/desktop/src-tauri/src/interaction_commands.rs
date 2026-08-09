use std::collections::HashMap;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use riviu_core::AppEvent;
use riviu_core::{
    discover_comment_identity, locate_parent_comment, parse_tiktok_links, plan_threads,
    CommentLocatorIdentity, CommentOcrObservation, DeviceControlPlane, DeviceWorkOwner,
    InteractionCampaignDetail, InteractionCampaignSummary, InteractionSessionKind,
    PreparedThreadMessage, TapPoint, ThreadCampaignRequest, ThreadCampaignState,
    ThreadMessageState, ThreadPlan, ThreadPreview, TikTokLinkLine,
};
use serde::Serialize;
use tauri::State;

use crate::command_error::CommandError;
use crate::interaction_ocr;
use crate::state::AppState;

const TIKTOK_BUNDLE_ID: &str = "com.ss.iphone.ugc.Ame";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InteractionStartResult {
    pub campaign: InteractionCampaignSummary,
    pub queued: bool,
}

fn interaction_error(error: impl std::fmt::Display) -> CommandError {
    CommandError::code("InteractionFailed", error.to_string())
}

fn revision() -> u64 {
    chrono::Utc::now().timestamp_millis().max(0) as u64
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
        lines,
        plan: Some(plan),
    })
}

/// The thread campaign grounds every comment in frames of the post, so it needs
/// a provider that accepts images.
///
/// Nurture branches on this and falls back to an OCR caption
/// (`provider_supports_vision` in `openai_client`); this path calls
/// `prepare_grounded_comment` unconditionally and propagates the failure with
/// `?`, which aborts the whole campaign. With the default `api.deepseek.com` —
/// a text-only API — that happens before any device is touched, and the
/// operator sees "AI chuẩn bị assignment 0" with no hint that the provider is
/// the problem. Say it up front instead.
fn require_vision_provider(settings: &riviu_core::NurtureSettings) -> Result<(), CommandError> {
    if riviu_core::openai_client::provider_supports_vision(settings) {
        return Ok(());
    }
    Err(CommandError::code(
        "VisionProviderRequired",
        format!(
            "chuỗi bình luận cần model đọc được ảnh để bám nội dung bài;              '{}' chỉ nhận text. Đổi sang provider có vision (OpenAI, Gemini, Claude…)              trong cấu hình AI.",
            riviu_core::openai_client::host_of(&settings.base_url)
        ),
    ))
}

/// A thread needs to read its own comment back off the screen to reply to it,
/// and the campaign writes Vietnamese. Without a reader that can, the run posts
/// the first message of every thread and skips the rest — so refuse up front,
/// naming both ways out, instead of discovering it one message in.
fn require_vietnamese_reader() -> Result<(), CommandError> {
    if interaction_ocr::reads_vietnamese() {
        return Ok(());
    }
    let found = interaction_ocr::recognizer_language().unwrap_or_else(|| "không có".into());
    Err(CommandError::code(
        "OcrLanguageUnavailable",
        format!(
            "chuỗi bình luận cần OCR đọc được tiếng Việt để tìm lại bình luận cha;              máy này đọc bằng '{found}'. Windows.Media.Ocr không phát hành gói tiếng Việt              (35 gói, không có vi-VN), nên không có gì để cài — chạy chiến dịch trên máy Mac,              nơi helper Vision đã ghim sẵn vi-VN."
        ),
    ))
}

#[tauri::command]
pub async fn interaction_start_thread(
    state: State<'_, AppState>,
    request: ThreadCampaignRequest,
) -> Result<InteractionStartResult, CommandError> {
    let admission = state.ensure_accepting_work()?;
    require_vietnamese_reader()?;
    require_vision_provider(
        &state
            .db
            .get_nurture_settings()
            .map_err(CommandError::operation)?,
    )?;
    let plan = plan_threads(&request).map_err(interaction_error)?;
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
        )
        .await
        {
            let _ = db.update_interaction_campaign_state(
                &campaign_id,
                ThreadCampaignState::Failed,
                Some(&error.to_string()),
            );
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

/// Which assignments a retry may re-send.
///
/// Excluding `Succeeded` is the whole point: tapping Send is not idempotent, so
/// a retry that re-ran a delivered message would post the comment twice.
/// `Uncertain` is excluded for the same reason with less information — delivery
/// is exactly what is unproven there. `Sending` is still in flight.
///
/// `requested` narrows further when the caller names specific assignments;
/// `None` means "everything still retryable".
fn retryable_assignments(
    assignments: &[riviu_core::InteractionAssignmentRecord],
    requested: Option<&std::collections::HashSet<String>>,
) -> std::collections::HashSet<String> {
    assignments
        .iter()
        .filter(|assignment| {
            !matches!(
                assignment.state,
                ThreadMessageState::Sending
                    | ThreadMessageState::Succeeded
                    | ThreadMessageState::Uncertain
            )
        })
        .filter(|assignment| requested.map_or(true, |ids| ids.contains(&assignment.id)))
        .map(|assignment| assignment.id.clone())
        .collect()
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
    require_vietnamese_reader()?;
    require_vision_provider(
        &state
            .db
            .get_nurture_settings()
            .map_err(CommandError::operation)?,
    )?;
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
    state
        .db
        .update_interaction_campaign_state(&campaign_id, ThreadCampaignState::Queued, None)
        .map_err(CommandError::operation)?;
    let (request, plan) = state
        .db
        .get_interaction_campaign_request(&campaign_id)
        .map_err(CommandError::operation)?
        .ok_or_else(|| {
            CommandError::code("InteractionNotFound", "campaign request không tồn tại")
        })?;
    state
        .db
        .update_interaction_campaign_state(&campaign_id, ThreadCampaignState::Running, None)
        .map_err(CommandError::operation)?;
    let db = state.db.clone();
    let control = state.control.clone();
    let engine = state.nurture_engine.clone();
    let events = state.events.clone();
    let artifacts = state.interaction_artifacts.clone();
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
        )
        .await
        {
            let _ = db.update_interaction_campaign_state(
                &worker_id,
                ThreadCampaignState::Failed,
                Some(&error.to_string()),
            );
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
    let context = open_interaction_context(&state.control, &udid).await?;
    let session = state
        .control
        .streaming_session(&context)
        .map_err(CommandError::from)?;
    let result = async {
        let _proof =
            open_target_confirmed(&state.nurture_engine, &udid, session.as_ref(), &target).await?;
        Ok::<(), anyhow::Error>(())
    }
    .await;
    let cleanup = state.control.close_ui_context(context).await;
    result.map_err(CommandError::operation)?;
    cleanup.map(|_| ()).map_err(CommandError::from)
}

async fn open_interaction_context(
    control: &DeviceControlPlane,
    udid: &str,
) -> Result<riviu_core::UiWithStreamContext, CommandError> {
    let exclusive = control
        .try_acquire_exclusive(udid, DeviceWorkOwner::Interaction)
        .await
        .map_err(CommandError::from)?;
    let (exclusive, capacity) = control
        .reserve_ui_capacity(exclusive)
        .await
        .map_err(CommandError::from)?;
    let kind = if control.requires_fresh_text_session() {
        InteractionSessionKind::FreshText
    } else {
        InteractionSessionKind::Ordinary
    };
    let session = control
        .start_interaction_session(exclusive, TIKTOK_BUNDLE_ID, kind)
        .await
        .map_err(CommandError::from)?;
    control
        .start_reserved_stream(session, capacity)
        .await
        .map_err(CommandError::from)
}

/// Save the screen as it stands and return its stored path.
///
/// The campaign used to persist frame hashes without keeping a single frame, so
/// nothing it recorded could be checked afterwards — and `Uncertain`, the state
/// that most needs looking at, wrote no artifact at all. Publishing is
/// best-effort: a campaign must not fail because evidence could not be filed.
fn publish_evidence_frame(
    artifacts: &riviu_core::FlowArtifactStore,
    engine: &riviu_core::NurtureEngine,
    campaign_id: &str,
    assignment_id: &str,
    udid: &str,
) -> Option<(String, String)> {
    let frame = engine.frames.latest(udid)?;
    let campaign = uuid::Uuid::parse_str(campaign_id).ok()?;
    let assignment = uuid::Uuid::parse_str(assignment_id).ok()?;
    let prepared = artifacts
        .prepare_image(
            campaign,
            assignment,
            uuid::Uuid::new_v4(),
            "comment-drawer.jpeg",
            "jpeg",
            &frame,
        )
        .map_err(|error| log::warn!("interaction: không chuẩn bị được ảnh bằng chứng: {error}"))
        .ok()?;
    match artifacts.publish_file(&prepared) {
        Ok(relative) => Some((relative.to_string_lossy().into_owned(), prepared.sha256)),
        Err(error) => {
            log::warn!("interaction: không lưu được ảnh bằng chứng: {error}");
            let _ = artifacts.rollback_file(&prepared);
            None
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn execute_thread_campaign(
    db: Arc<riviu_core::db::Database>,
    control: Arc<DeviceControlPlane>,
    engine: riviu_core::NurtureEngine,
    events: riviu_core::EventBus,
    campaign_id: String,
    request: ThreadCampaignRequest,
    plan: ThreadPlan,
    only_assignments: Option<std::collections::HashSet<String>>,
    artifacts: riviu_core::FlowArtifactStore,
) -> anyhow::Result<()> {
    let settings = db.get_nurture_settings().context("đọc cấu hình AI")?;
    if settings.api_key.trim().is_empty() {
        anyhow::bail!("AI API key chưa được cấu hình cho Interaction");
    }
    let detail = db
        .get_interaction_campaign(&campaign_id)?
        .context("campaign không tồn tại")?;
    let assignment_ids: HashMap<(String, u8), String> = detail
        .assignments
        .into_iter()
        .map(|assignment| {
            (
                (assignment.target_key.clone(), assignment.ordinal),
                assignment.id,
            )
        })
        .collect();
    // Held false for the life of the campaign, on purpose. It reaches the send
    // functions, where it would abort their waits mid-flight — including the
    // window between tapping Send and confirming the field cleared, which would
    // manufacture `Uncertain` and block retry for a comment that did post.
    // Stopping is handled between assignments instead, where nothing is in
    // flight.
    let stop = AtomicBool::new(false);
    let mut succeeded = 0usize;
    let mut failed = 0usize;

    for target in &request.targets {
        if campaign_is_cancelled(&db, &campaign_id)? {
            return Ok(());
        }

        // Open the target and collect the same-post evidence before preparing
        // any message. All texts for this target are persisted before send.
        let target_root_actor = plan
            .assignments
            .iter()
            .find(|assignment| {
                assignment.target_key == target.target_key && assignment.ordinal == 0
            })
            .map(|assignment| assignment.actor_udid.as_str())
            .context("target root actor missing")?;
        let context = open_interaction_context(&control, target_root_actor)
            .await
            .map_err(|error| anyhow::anyhow!("{}: {}", error.code, error.message))?;
        let session = control.streaming_session(&context)?;
        // Evidence for the AI has to come from the target, not from whatever
        // survived the two seconds this used to sleep.
        let _proof =
            open_target_confirmed(&engine, target_root_actor, session.as_ref(), target).await?;
        let mut frames = Vec::with_capacity(3);
        for _ in 0..3 {
            if let Some(frame) = engine.frames.latest(target_root_actor) {
                frames.push((*frame).clone());
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
        control.close_ui_context(context).await?;
        if frames.is_empty() {
            anyhow::bail!("target {} không có frame stream", target.target_key);
        }

        let mut prepared_messages = Vec::with_capacity(request.message_count as usize);
        let mut previous = None::<String>;
        for assignment in plan
            .assignments
            .iter()
            .filter(|assignment| assignment.target_key == target.target_key)
        {
            let id = assignment_ids
                .get(&(assignment.target_key.clone(), assignment.ordinal))
                .context("assignment id missing")?;
            db.update_interaction_assignment_state(
                id,
                ThreadMessageState::Preparing,
                None,
                None,
                None,
            )?;
            let mut scoped = settings.clone();
            scoped.max_comment_words = u32::from(request.max_words);
            let direction = if let Some(parent) = &previous {
                format!(
                    "{}; trả lời tự nhiên câu trước {:?}, không nhắc lại nguyên văn",
                    request.instruction, parent
                )
            } else {
                request.instruction.clone()
            };
            let grounded = riviu_core::openai_client::prepare_grounded_comment(
                &scoped,
                &frames,
                Some(&direction),
            )
            .await
            .with_context(|| format!("AI chuẩn bị assignment {}", assignment.ordinal))?;
            let prepared = PreparedThreadMessage::new(assignment, grounded.text);
            previous = Some(prepared.text.clone());
            db.prepare_interaction_assignment(id, &prepared)?;
            prepared_messages.push((id.clone(), prepared));
        }

        // A root comment is sent with full frame evidence. Each subsequent
        // reply first resolves the exact parent text+author on two OCR frames.
        let mut identities = HashMap::<u8, CommentLocatorIdentity>::new();
        let mut chain_broken_at: Option<u8> = None;
        for (id, prepared) in prepared_messages {
            // A retry runs the same plan but must not re-send anything already
            // posted; the caller decides which assignments are in scope.
            if only_assignments
                .as_ref()
                .is_some_and(|only| !only.contains(&id))
            {
                continue;
            }
            // Cancellation was only ever checked between *targets*, so pressing
            // Dừng left every remaining message of the current target to post —
            // up to six more public comments. Check before each one instead.
            //
            // Deliberately not mid-send: the Send tap is a side effect that has
            // already gone out, and aborting between it and its confirmation
            // would manufacture `Uncertain`, which blocks retry. One in-flight
            // message finishing is the correct cost of stopping.
            if campaign_is_cancelled(&db, &campaign_id)? {
                return Ok(());
            }
            let parent_identity = prepared
                .parent_ordinal
                .and_then(|ordinal| identities.get(&ordinal).cloned());
            if let Some(parent_ordinal) = prepared
                .parent_ordinal
                .filter(|_| parent_identity.is_none())
            {
                // The chain is linear and an identity is only ever learned by
                // sending, so once it breaks nothing later in this target can
                // recover — every remaining message lands here. Naming the
                // ordinal that broke it is the difference between "5 messages
                // skipped" and knowing which one to look at.
                if chain_broken_at.is_none() {
                    chain_broken_at = Some(parent_ordinal);
                }
                let broke_at = chain_broken_at.unwrap_or(parent_ordinal);
                db.update_interaction_assignment_state(
                    &id,
                    ThreadMessageState::SkippedParent,
                    Some(&format!(
                        "parent_identity_not_confirmed_at_ordinal_{broke_at}"
                    )),
                    None,
                    None,
                )?;
                failed += 1;
                continue;
            }
            let context = match open_interaction_context(&control, &prepared.actor_udid).await {
                Ok(context) => context,
                Err(error) => {
                    db.update_interaction_assignment_state(
                        &id,
                        ThreadMessageState::Failed,
                        Some(&format!("{}: {}", error.code, error.message)),
                        None,
                        None,
                    )?;
                    failed += 1;
                    continue;
                }
            };
            let session = control.streaming_session(&context)?;
            let gestures = tokio::sync::Mutex::new(());
            let mut effect_intent = false;
            let result = async {
                // Nothing is typed until the target is on screen: an
                // unconfirmed open posts the campaign's comment to whichever
                // video the phone happens to be showing.
                let proof =
                    open_target_confirmed(&engine, &prepared.actor_udid, session.as_ref(), target)
                        .await?;
                if proof == TargetProof::Structural {
                    // Worth saying out loud: the post is open but unidentified,
                    // so nothing here rules out the link having resolved to a
                    // different video. On Windows this is every send, because
                    // the OCR that reads the handle back is macOS-only.
                    log::warn!(
                        "interaction {}: đăng vào bài của @{} chưa đối chiếu được tên tác giả \
                         (OCR không khả dụng trên nền tảng này)",
                        target.target_key,
                        target.author
                    );
                }
                if let Some(parent) = parent_identity.as_ref() {
                    let (observations, screen_size) = open_comment_for_ocr(
                        &engine,
                        &prepared.actor_udid,
                        session.as_ref(),
                        &gestures,
                    )
                    .await?;
                    // Hunt down the list first; the stability check then runs
                    // on the frame that actually shows the parent.
                    let observations = scroll_to_parent(
                        &engine,
                        &prepared.actor_udid,
                        session.as_ref(),
                        &gestures,
                        parent,
                        screen_size,
                        observations,
                    )
                    .await?;
                    let parent_match =
                        stable_parent_match(&engine, &prepared.actor_udid, parent, observations)
                            .await?;
                    let reply_point = TapPoint {
                        x: screen_size.0 * parent_match.reply_x,
                        y: screen_size.1 * parent_match.reply_y,
                    };
                    db.update_interaction_assignment_state(
                        &id,
                        ThreadMessageState::Sending,
                        None,
                        Some("reply_comment"),
                        None,
                    )?;
                    effect_intent = true;
                    let evidence = engine
                        .send_prepared_thread_reply(
                            &prepared.actor_udid,
                            session.as_ref(),
                            &gestures,
                            reply_point,
                            &prepared,
                            &stop,
                        )
                        .await?;
                    let identity = discover_after_send(
                        &engine,
                        &prepared.actor_udid,
                        session.as_ref(),
                        &gestures,
                        &prepared.text,
                    )
                    .await
                    .ok();
                    Ok::<serde_json::Value, anyhow::Error>(serde_json::json!({
                        "send": evidence,
                        "parent": parent,
                        "postedIdentity": identity,
                    }))
                } else {
                    db.update_interaction_assignment_state(
                        &id,
                        ThreadMessageState::Sending,
                        None,
                        Some("post_comment"),
                        None,
                    )?;
                    effect_intent = true;
                    let evidence = engine
                        .send_prepared_thread_comment(
                            &prepared.actor_udid,
                            session.as_ref(),
                            &gestures,
                            &prepared,
                            &stop,
                        )
                        .await?;
                    let identity = discover_after_send(
                        &engine,
                        &prepared.actor_udid,
                        session.as_ref(),
                        &gestures,
                        &prepared.text,
                    )
                    .await
                    .ok();
                    Ok::<serde_json::Value, anyhow::Error>(serde_json::json!({
                        "send": evidence,
                        "postedIdentity": identity,
                    }))
                }
            }
            .await;
            let cleanup = control.close_ui_context(context).await;
            match result {
                Ok(evidence_json) => {
                    let evidence_text = evidence_json.to_string();
                    db.update_interaction_assignment_state(
                        &id,
                        ThreadMessageState::Succeeded,
                        None,
                        None,
                        Some(&evidence_text),
                    )?;
                    let artifact_kind = if prepared.parent_ordinal.is_some() {
                        "comment-reply-evidence"
                    } else {
                        "comment-root-evidence"
                    };
                    // The drawer is still open at this point — nothing closes
                    // it after the identity pass — so this frame shows the
                    // comment that was just posted, in the list, which is the
                    // only thing that settles a dispute later.
                    let saved = publish_evidence_frame(
                        &artifacts,
                        &engine,
                        &campaign_id,
                        &id,
                        &prepared.actor_udid,
                    );
                    let artifact_sha = saved.as_ref().map(|(_, sha)| sha.as_str()).unwrap_or(
                        evidence_json
                            .get("postedIdentity")
                            .and_then(|identity| identity.get("frameSha256"))
                            .and_then(|value| value.as_str())
                            .unwrap_or_default(),
                    );
                    let _ = db.add_interaction_artifact(
                        &campaign_id,
                        &target.target_key,
                        Some(&id),
                        artifact_kind,
                        &evidence_text,
                        artifact_sha,
                        saved.as_ref().map(|(path, _)| path.as_str()),
                    )?;
                    if let Some(identity) = evidence_json.get("postedIdentity").and_then(|value| {
                        serde_json::from_value::<CommentLocatorIdentity>(value.clone()).ok()
                    }) {
                        identities.insert(prepared.ordinal, identity);
                    }
                    succeeded += 1;
                }
                Err(error) => {
                    let state = if effect_intent {
                        ThreadMessageState::Uncertain
                    } else {
                        ThreadMessageState::Failed
                    };
                    db.update_interaction_assignment_state(
                        &id,
                        state,
                        Some(&error.to_string()),
                        None,
                        None,
                    )?;
                    // Especially here. `Uncertain` means the Send tap went out
                    // and its confirmation did not arrive, so whether the
                    // comment posted can only be settled by looking — and this
                    // path used to write no artifact at all.
                    if let Some((path, sha)) = publish_evidence_frame(
                        &artifacts,
                        &engine,
                        &campaign_id,
                        &id,
                        &prepared.actor_udid,
                    ) {
                        let _ = db.add_interaction_artifact(
                            &campaign_id,
                            &target.target_key,
                            Some(&id),
                            "comment-failure-evidence",
                            &serde_json::json!({ "error": error.to_string() }).to_string(),
                            &sha,
                            Some(&path),
                        );
                    }
                    failed += 1;
                }
            }
            if let Err(error) = cleanup {
                log::warn!("interaction cleanup {}: {}", prepared.actor_udid, error);
            }
            events.emit(AppEvent::InteractionUpdated {
                campaign_id: campaign_id.clone(),
                revision: revision(),
            });
        }
    }

    let cancelled = matches!(
        db.get_interaction_campaign(&campaign_id)?
            .map(|detail| detail.summary.state),
        Some(ThreadCampaignState::Cancelled)
    );
    if !cancelled {
        let final_state = if failed == 0 {
            ThreadCampaignState::Succeeded
        } else if succeeded == 0 {
            ThreadCampaignState::Failed
        } else {
            ThreadCampaignState::Partial
        };
        db.update_interaction_campaign_state(&campaign_id, final_state, None)?;
    }
    events.emit(AppEvent::InteractionUpdated {
        campaign_id,
        revision: revision(),
    });
    Ok(())
}

/// Longest the device gets to land on the target before the open is called a
/// failure, and how often the frame is read while waiting. The check costs one
/// OCR pass per poll, so this is deliberately slower than a frame loop — it runs
/// once per target, not per frame.
const OPEN_TARGET_TIMEOUT: Duration = Duration::from_secs(14);
const OPEN_TARGET_POLL: Duration = Duration::from_millis(900);
/// Characters of the author handle that must be legible. TikTok truncates long
/// handles with an ellipsis, so a prefix is all that can be required; six is
/// enough that another account on screen is unlikely to share it.
const OPEN_TARGET_HANDLE_PREFIX: usize = 6;
/// How long to keep looking for the handle once the post has visibly settled,
/// before accepting the structural proof and moving on. The post is already up
/// by then; this is only the identity check catching up.
const OPEN_TARGET_HANDLE_GRACE: Duration = Duration::from_secs(4);

/// Open a target link and prove the device landed on *that* post.
///
/// `open_url` returning Ok means the request was accepted, not that the video is
/// on screen. This module already draws that distinction for typing — "the
/// request returning OK is not evidence of insertion" — but not for opening: two
/// of the three call sites slept two seconds and carried on, and the third only
/// asked whether TikTok was frontmost, which does not say *which post* is up,
/// and skipped even that whenever the query itself failed.
///
/// It matters most on the send path. A campaign that opens nothing then posts
/// its comment to whatever the phone happens to be showing — someone else's
/// video, under the operator's account. On the evidence path it means the AI
/// writes its comment about a post that is not the target.
///
/// The link carries the author handle and the opened post displays it, so that
/// is the proof: wait for a frame whose OCR contains it.
/// How far the open could be proved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TargetProof {
    /// The author handle from the link was read back off the screen. This is
    /// the only level that identifies *which* post is open.
    Identified,
    /// TikTok came forward, the screen changed, and it settled on a post — but
    /// the handle could not be read, so the post is unidentified. OCR here is
    /// macOS-only (`interaction_ocr::recognize` bails everywhere else), so this
    /// is the best available proof on Windows.
    Structural,
}

async fn open_target_confirmed(
    engine: &riviu_core::NurtureEngine,
    udid: &str,
    session: &dyn riviu_core::UiSession,
    target: &riviu_core::ResolvedTikTokTarget,
) -> anyhow::Result<TargetProof> {
    // The screen as it was before the request, so "nothing happened" is
    // distinguishable from "the post loaded".
    let before = engine
        .frames
        .latest(udid)
        .map(|frame| riviu_core::frame_sha256(&frame))
        .unwrap_or_default();

    session
        .open_url(&target.normalized_url)
        .await
        .with_context(|| format!("mở link {}", target.normalized_url))?;

    let wanted = riviu_core::normalize_locator_text(&target.author);
    let needle: String = wanted
        .chars()
        .take(OPEN_TARGET_HANDLE_PREFIX.min(wanted.chars().count()))
        .collect();

    let deadline = tokio::time::Instant::now() + OPEN_TARGET_TIMEOUT;
    let mut ocr_available = true;
    let mut settled_since: Option<tokio::time::Instant> = None;
    while tokio::time::Instant::now() < deadline {
        tokio::time::sleep(OPEN_TARGET_POLL).await;
        // A wrong frontmost app is decisive; a query that cannot answer it is
        // not, and never stands in for the frame proof either way.
        if let Ok(bundle) = session.active_app_bundle().await {
            if bundle != TIKTOK_BUNDLE_ID {
                continue;
            }
        }
        let Some(frame) = engine.frames.latest(udid) else {
            continue;
        };
        if riviu_core::frame_sha256(&frame) == before {
            // Byte-identical to the pre-request screen: the open did nothing.
            continue;
        }
        let Ok(decoded) = image::load_from_memory(&frame) else {
            continue;
        };
        if riviu_core::screen::locate_action_rail(&decoded.to_rgb8()).is_none() {
            // Still loading, or on an interstitial — not a post yet.
            continue;
        }
        let settled_at = *settled_since.get_or_insert(tokio::time::Instant::now());

        if ocr_available && !needle.is_empty() {
            match interaction_ocr::recognize(&frame).await {
                Ok(observations) => {
                    if observations.iter().any(|observation| {
                        riviu_core::normalize_locator_text(&observation.text).contains(&needle)
                    }) {
                        return Ok(TargetProof::Identified);
                    }
                }
                // Not a reason to fail the open — it is how a build with no OCR
                // reports that fact. Fall back to the structural proof rather
                // than blocking the feature on a platform capability.
                Err(_) => ocr_available = false,
            }
        }
        // A missing handle downgrades the proof; it never fails the open.
        //
        // The handle comes from the URL, and what the post displays is the
        // account's *nickname*, which often is not the same string — captures
        // from this device show both cases ("nguyenvantoan8584" on one card,
        // "Lúc này lúc kia" on another). Until that has been checked against a
        // link-opened page on a real device, treating a missing handle as proof
        // of the wrong post would break the feature for every account whose
        // nickname differs from its handle, to guard against a failure that has
        // never been observed. The failure that *has* been observed is the open
        // doing nothing at all, and the checks above catch that.
        if !ocr_available || settled_at.elapsed() >= OPEN_TARGET_HANDLE_GRACE {
            return Ok(TargetProof::Structural);
        }
    }

    if settled_since.is_some() {
        return Ok(TargetProof::Structural);
    }
    anyhow::bail!(
        "mở link {} nhưng máy không chuyển sang bài viết nào",
        target.normalized_url
    )
}

async fn open_comment_for_ocr(
    engine: &riviu_core::NurtureEngine,
    udid: &str,
    session: &dyn riviu_core::UiSession,
    gestures: &tokio::sync::Mutex<()>,
) -> anyhow::Result<(Vec<CommentOcrObservation>, (f64, f64))> {
    let frame = engine
        .frames
        .latest(udid)
        .context("không có frame trước khi mở comment")?;
    let image = image::load_from_memory(&frame)?.to_rgb8();
    // Locate per frame and fail closed rather than tapping the layout-2
    // fallback blind (an already-followed card hides the red badge; a
    // layout-1 card would then bookmark instead of opening comments).
    let rail = riviu_core::screen::locate_action_rail(&image)
        .context("không định vị được action rail để mở comment")?;
    let screen_size = session.window_size().await.unwrap_or((375.0, 667.0));
    {
        let _guard = gestures.lock().await;
        session
            .tap(TapPoint {
                x: screen_size.0 * rail.x,
                y: screen_size.1 * rail.comment_y,
            })
            .await?;
    }
    for _ in 0..30 {
        if let Some(next) = engine.frames.latest(udid) {
            if let Ok(decoded) = image::load_from_memory(&next) {
                let state = riviu_core::screen::comment_drawer_state(&decoded.to_rgb8()).0;
                if !matches!(
                    state,
                    riviu_core::screen::CommentDrawer::Closed
                        | riviu_core::screen::CommentDrawer::Unknown
                ) {
                    tokio::time::sleep(Duration::from_millis(900)).await;
                    let stable = engine.frames.latest(udid).context("drawer frame missing")?;
                    let observations = interaction_ocr::recognize(&stable).await?;
                    return Ok((observations, screen_size));
                }
            }
        }
        tokio::time::sleep(Duration::from_millis(180)).await;
    }
    anyhow::bail!("comment drawer OCR không mở được")
}

async fn discover_after_send(
    engine: &riviu_core::NurtureEngine,
    udid: &str,
    session: &dyn riviu_core::UiSession,
    gestures: &tokio::sync::Mutex<()>,
    exact_text: &str,
) -> anyhow::Result<CommentLocatorIdentity> {
    let (first, _) = open_comment_for_ocr(engine, udid, session, gestures).await?;
    let first_frame = engine
        .frames
        .latest(udid)
        .context("identity frame missing")?;
    let first_sha = riviu_core::frame_sha256(&first_frame);
    let first_identity = discover_comment_identity(
        &first,
        exact_text,
        &first_sha,
        interaction_ocr::locator_version(),
    )
    .context("comment author/text chưa xuất hiện trong OCR")?;
    tokio::time::sleep(Duration::from_millis(700)).await;
    let second_frame = engine
        .frames
        .latest(udid)
        .context("identity second frame missing")?;
    let second = interaction_ocr::recognize(&second_frame).await?;
    let second_identity = discover_comment_identity(
        &second,
        exact_text,
        &riviu_core::frame_sha256(&second_frame),
        interaction_ocr::locator_version(),
    )
    .context("comment identity không ổn định")?;
    if riviu_core::normalize_locator_text(&first_identity.author_label)
        != riviu_core::normalize_locator_text(&second_identity.author_label)
        || riviu_core::normalize_locator_text(&first_identity.text)
            != riviu_core::normalize_locator_text(&second_identity.text)
    {
        anyhow::bail!("comment identity thay đổi giữa hai frame")
    }
    dismiss_comment_drawer(session, gestures).await;
    Ok(first_identity)
}

/// How far to hunt for the parent before giving up, and the gesture used.
///
/// The drawer only ever showed its first screenful: `open_comment_for_ocr`
/// opens it, reads one frame, and returns. Every reply is sent from a *different
/// device* that re-opens the link fresh, so TikTok re-ranks the list each time
/// and the campaign's own comment is under no obligation to still be near the
/// top. When it is not, the parent is simply unfindable and the rest of the
/// thread dies — with no attempt to look further down.
const PARENT_SCROLL_ATTEMPTS: u32 = 4;
const PARENT_SCROLL_FROM_Y: f64 = 0.62;
const PARENT_SCROLL_TO_Y: f64 = 0.38;
const PARENT_SCROLL_SETTLE: Duration = Duration::from_millis(900);

/// Scroll the comment list until the parent is on screen.
///
/// Each scroll has to prove it happened, and here the proof is cheap in a way it
/// never was for the feed: the drawer is static, so a frame that changed is a
/// list that moved. (In the feed the opposite held — a playing video changed
/// every frame, which is what made the old swipe check worthless.) A swipe that
/// changes nothing means the list is at its end, and a frame that stops
/// classifying as an open drawer means the gesture closed it instead of
/// scrolling it; both stop the hunt rather than swiping on blindly.
async fn scroll_to_parent(
    engine: &riviu_core::NurtureEngine,
    udid: &str,
    session: &dyn riviu_core::UiSession,
    gestures: &tokio::sync::Mutex<()>,
    identity: &CommentLocatorIdentity,
    screen_size: (f64, f64),
    first: Vec<CommentOcrObservation>,
) -> anyhow::Result<Vec<CommentOcrObservation>> {
    if locate_parent_comment(&first, identity).is_some() {
        return Ok(first);
    }
    for attempt in 1..=PARENT_SCROLL_ATTEMPTS {
        let before = engine
            .frames
            .latest(udid)
            .context("no frame before scrolling the comment list")?;
        {
            let _guard = gestures.lock().await;
            session
                .swipe(riviu_core::SwipeGesture {
                    from: TapPoint {
                        x: screen_size.0 * 0.5,
                        y: screen_size.1 * PARENT_SCROLL_FROM_Y,
                    },
                    to: TapPoint {
                        x: screen_size.0 * 0.5,
                        y: screen_size.1 * PARENT_SCROLL_TO_Y,
                    },
                    duration_ms: 320,
                })
                .await
                .context("scroll the comment list")?;
        }
        tokio::time::sleep(PARENT_SCROLL_SETTLE).await;

        let after = engine
            .frames
            .latest(udid)
            .context("no frame after scrolling the comment list")?;
        if riviu_core::frame_sha256(&after) == riviu_core::frame_sha256(&before) {
            anyhow::bail!(
                "cuộn {attempt} lần nhưng danh sách bình luận không nhúc nhích —                  nhiều khả năng đã hết danh sách"
            );
        }
        let decoded = image::load_from_memory(&after)
            .context("decode comment drawer frame")?
            .to_rgb8();
        if matches!(
            riviu_core::screen::comment_drawer_state(&decoded).0,
            riviu_core::screen::CommentDrawer::Closed | riviu_core::screen::CommentDrawer::Unknown
        ) {
            anyhow::bail!("cú vuốt đã đóng khay bình luận thay vì cuộn nó");
        }

        let observations = interaction_ocr::recognize(&after).await?;
        if locate_parent_comment(&observations, identity).is_some() {
            return Ok(observations);
        }
    }
    anyhow::bail!("không thấy bình luận cha sau {PARENT_SCROLL_ATTEMPTS} lần cuộn danh sách")
}

async fn stable_parent_match(
    engine: &riviu_core::NurtureEngine,
    udid: &str,
    identity: &CommentLocatorIdentity,
    first: Vec<CommentOcrObservation>,
) -> anyhow::Result<riviu_core::CommentParentMatch> {
    let first_match =
        locate_parent_comment(&first, identity).context("parent/reply control không khớp")?;
    tokio::time::sleep(Duration::from_millis(700)).await;
    let second_frame = engine
        .frames
        .latest(udid)
        .context("parent second frame missing")?;
    let second = interaction_ocr::recognize(&second_frame).await?;
    let second_match = locate_parent_comment(&second, identity).context("parent không ổn định")?;
    if (first_match.reply_x - second_match.reply_x).abs() > 0.04
        || (first_match.reply_y - second_match.reply_y).abs() > 0.04
    {
        anyhow::bail!("reply control thay đổi giữa hai frame")
    }
    Ok(first_match)
}

async fn dismiss_comment_drawer(
    session: &dyn riviu_core::UiSession,
    gestures: &tokio::sync::Mutex<()>,
) {
    let screen_size = session.window_size().await.unwrap_or((375.0, 667.0));
    let _guard = gestures.lock().await;
    let _ = session
        .tap(TapPoint {
            x: screen_size.0 * riviu_core::screen::DRAWER_DISMISS.0,
            y: screen_size.1 * riviu_core::screen::DRAWER_DISMISS.1,
        })
        .await;
}

/// `interaction_cancel` only flips the campaign row, so the running worker has
/// to read it back to notice. Nothing else signals it.
fn campaign_is_cancelled(db: &riviu_core::db::Database, campaign_id: &str) -> anyhow::Result<bool> {
    Ok(matches!(
        db.get_interaction_campaign(campaign_id)?
            .map(|detail| detail.summary.state),
        Some(ThreadCampaignState::Cancelled)
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use riviu_core::InteractionAssignmentRecord;

    fn assignment(id: &str, ordinal: u8, state: ThreadMessageState) -> InteractionAssignmentRecord {
        InteractionAssignmentRecord {
            id: id.into(),
            target_key: "content:1".into(),
            ordinal,
            actor_udid: format!("actor-{ordinal}"),
            parent_assignment_id: None,
            state,
            prepared_text: Some("nội dung".into()),
            error_code: None,
        }
    }

    /// The state a broken chain actually leaves behind: one message posted, the
    /// rest skipped. Retry used to refuse the whole campaign on sight of that
    /// single `Succeeded`, so the most common failure was also the one that
    /// could never be repaired.
    #[test]
    fn a_posted_message_is_never_re_sent_but_the_skipped_ones_are_retryable() {
        let assignments = vec![
            assignment("a0", 0, ThreadMessageState::Succeeded),
            assignment("a1", 1, ThreadMessageState::SkippedParent),
            assignment("a2", 2, ThreadMessageState::SkippedParent),
        ];

        let retryable = retryable_assignments(&assignments, None);

        assert!(
            !retryable.contains("a0"),
            "tapping Send is not idempotent — a delivered comment must never be re-sent"
        );
        assert_eq!(retryable.len(), 2);
        assert!(retryable.contains("a1") && retryable.contains("a2"));
    }

    /// Delivery is exactly what `Uncertain` does not establish, and `Sending`
    /// is still in flight. Both stay out for the same reason as `Succeeded`.
    #[test]
    fn unproven_and_in_flight_deliveries_are_excluded() {
        let assignments = vec![
            assignment("u", 0, ThreadMessageState::Uncertain),
            assignment("s", 1, ThreadMessageState::Sending),
            assignment("f", 2, ThreadMessageState::Failed),
        ];

        let retryable = retryable_assignments(&assignments, None);

        assert_eq!(retryable.len(), 1);
        assert!(retryable.contains("f"));
    }

    /// Naming assignments narrows the set; it can never widen it past the
    /// safety filter above.
    #[test]
    fn naming_an_already_posted_assignment_does_not_authorise_re_sending_it() {
        let assignments = vec![
            assignment("a0", 0, ThreadMessageState::Succeeded),
            assignment("a1", 1, ThreadMessageState::SkippedParent),
        ];
        let requested: std::collections::HashSet<String> =
            ["a0".to_string(), "a1".to_string()].into_iter().collect();

        let retryable = retryable_assignments(&assignments, Some(&requested));

        assert_eq!(retryable.len(), 1);
        assert!(retryable.contains("a1"));
    }
}
