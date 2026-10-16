//! Acceptance facade over the existing create/execute pipeline, not another controller.
use super::*;
use futures_util::FutureExt;
use riviu_core::db::{PublishStartError, PublishStartReceipt};
use tauri::Manager;

#[tauri::command]
pub async fn publish_start_status(
    state: State<'_, AppState>,
    request_id: String,
) -> Result<Option<PublishStartReceipt>, CommandError> {
    let id = request_id.clone();
    let (receipt, orphaned) = state
        .db
        .storage_read(move |db| {
            Ok((
                db.publish_start_status(&id)?,
                db.publish_start_orphaned(&id)?,
            ))
        })
        .await
        .map_err(preflight::err)?;
    if !orphaned {
        return Ok(receipt);
    }
    // Review R4-F1: the preparation that owned this Start ended without settling it. Settle it
    // as a restart would (uncertain, never posted again) and release its account locks.
    state
        .db
        .storage_write(move |db| {
            db.reconcile_orphaned_publish_starts()?;
            db.publish_start_status(&request_id)
        })
        .await
        .map_err(preflight::err)
}

/// Finite budget for writing a Start's terminal state (review R4-F1): about 45 seconds of
/// retries through storage. If every attempt fails, the owner's drop leaves the receipt to
/// the status read, the next account write and startup, which settle it as `uncertain`.
const START_SETTLEMENT_ATTEMPTS: u32 = 8;

/// How far ahead of its hour a scheduled Start must be accepted (drop audit schedule_restart#3).
/// Its preparation reads every phone again and copies the media before the campaign exists; a
/// campaign created after its hour would be missed at once, so creation refuses it instead.
/// Measured on box 21-50 (30 SM-G955F, debug acceptance controller): 39-47 s from the Start's
/// approval check to its scheduled campaign on 16/10/2026 (d75f7cb8, fc42362a, 8324f685,
/// 3c039b04), 68 s on 06/10/2026, and up to 166 s from the click. Three minutes covers that with
/// margin; the check at creation stays the guarantee when a slower fleet needs more.
const SCHEDULED_START_MIN_LEAD_SECS: i64 = 180;

/// Claims of a Start's new campaign that failed on storage (drop audit schedule_restart#0).
/// Not measured: SQLite waits up to 5 s for the write lock per attempt, so three attempts 1 s
/// and 2 s apart give a held lock about 18 s to clear. Past that the rows stay queued, unsent.
const START_CLAIM_ATTEMPTS: u32 = 3;

/// The refusal of a scheduled Start whose hour is closer than its preparation needs. A past or
/// malformed time keeps the preflight's own refusal.
fn scheduled_start_too_soon(run_at: Option<&str>, phones: usize) -> Option<PublishStartError> {
    let at = execution::parse_run_at(run_at?).ok()?;
    let lead = at
        .signed_duration_since(Local::now().naive_local())
        .num_seconds();
    (lead < SCHEDULED_START_MIN_LEAD_SECS).then(|| PublishStartError {
        code: "PublishScheduleTooSoon".into(),
        message: format!(
            "Giờ hẹn {} chỉ còn {lead} giây: chuẩn bị {phones} máy cần ít nhất {} phút trước giờ hẹn. Chưa nhận lượt này và không đăng gì; chọn giờ muộn hơn hoặc Đăng ngay.",
            at.format("%H:%M %d/%m"),
            SCHEDULED_START_MIN_LEAD_SECS / 60
        ),
    })
}

/// Queue a Start's new immediate campaign under the admission the Start already holds (drop
/// audit schedule_restart#0). The admission-gated `publish_execute` refused here once the app
/// began closing, after the campaign existed, and the Start settled `uncertain` over rows nothing
/// could claim (06/10/2026, c3a4d8c8: 18 rows queued until cancelled). Queuing writes only SQLite:
/// during a close the dispatcher admits no job and the shutdown fence refuses every Post, and the
/// next launch resumes the queued rows like any other unsent work.
async fn enqueue_created_campaign(
    state: &AppState,
    campaign: &str,
    confirmed: bool,
) -> Result<(), CommandError> {
    let mut delay = Duration::from_secs(1);
    for attempt in 1..=START_CLAIM_ATTEMPTS {
        let result = execution::execute_publish_campaign_inner(
            state.control.clone(),
            state.registry.clone(),
            state.db.clone(),
            state.events.clone(),
            state.active_agent_bundle_id.clone(),
            Arc::new(state.streams.clone()),
            campaign.to_owned(),
            confirmed,
        )
        .await
        .map_err(preflight::err)?;
        // A claim that failed on storage comes back as an issue, not an error, and leaves the
        // campaign without jobs; a restart would then settle its Start `uncertain`.
        let failed = result
            .issues
            .iter()
            .find(|issue| issue.code == "publish_phone_failed");
        let Some(failed) = failed else {
            return Ok(());
        };
        if state
            .db
            .has_active_publish_pipeline(campaign)
            .map_err(preflight::err)?
        {
            return Ok(());
        }
        let short = campaign.get(..8).unwrap_or(campaign);
        if attempt == START_CLAIM_ATTEMPTS {
            log::warn!(
                "Đăng bài: chưa xếp được lượt {short} vào hàng chờ sau {attempt} lần ({}); các dòng vẫn chờ, chưa máy nào đăng",
                failed.message
            );
            return Ok(());
        }
        log::warn!(
            "Đăng bài: xếp lượt {short} vào hàng chờ lần {attempt} chưa được ({}); thử lại sau {} giây",
            failed.message,
            delay.as_secs()
        );
        tokio::time::sleep(delay).await;
        delay *= 2;
    }
    Ok(())
}

#[tauri::command]
pub async fn publish_cancel_unaccepted_start(
    state: State<'_, AppState>,
    request_id: String,
) -> Result<PublishStartReceipt, CommandError> {
    let _admission = state.ensure_accepting_work()?;
    state.db.storage_write(move |db| db.cancel_unaccepted_publish_start(&request_id))
        .await.map_err(preflight::err)
}

#[tauri::command]
#[allow(clippy::too_many_arguments)] // Tauri exposes each wire field as a named command argument.
pub async fn publish_start(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    mut request: riviu_core::PublishPreflightRequest,
    request_id: String,
    approved_input_digest: String,
    confirmed: bool,
    preparation_id: Option<String>,
    approved_release: Option<Vec<String>>,
) -> Result<PublishStartReceipt, CommandError> {
    let admission = state.ensure_accepting_work()?;
    state
        .db
        .ensure_publish_recovery_ready()
        .map_err(preflight::err)?;
    if !confirmed {
        return Err(CommandError::invalid_argument(
            "Cần xác nhận bắt đầu đúng lượt đăng",
        ));
    }
    Uuid::parse_str(&request_id).map_err(preflight::err)?;
    request.source_root = request.source_root.trim().to_owned();
    request.run_at = request.run_at.map(|at| at.trim().to_owned());
    // Computed before the acceptance gate so a refusal is bound to this exact body too.
    let fingerprint = execution::frame_sha256(
        &serde_json::to_vec(&serde_json::json!({"request":request,"confirmed":confirmed}))
            .map_err(preflight::err)?,
    );
    let scope_denied = state.dev_acceptance.active()
        && request.udids.iter().any(|udid| {
            !state
                .dev_acceptance
                .allows_publish_dispatch(&request_id, udid)
        });
    let refusal = if scope_denied {
        Some(PublishStartError {
            code: "AcceptanceScopeDenied".into(),
            message: "Lượt bắt đầu hoặc máy chưa được cấp quyền trong phạm vi nghiệm thu".into(),
        })
    } else {
        scheduled_start_too_soon(request.run_at.as_deref(), request.udids.len())
    };
    if let Some(error) = refusal {
        // Durable, request-bound refusal: the frontend retires its pending marker only from
        // a failed receipt without campaign, never from this error code alone.
        let (id, digest, bound, recorded) = (
            request_id.clone(),
            approved_input_digest.clone(),
            fingerprint.clone(),
            error.clone(),
        );
        // A failed write surfaces as a plain storage error without attemptId: the frontend
        // must keep the pending marker, because nothing durable says this ID was refused.
        let receipt = state
            .db
            .storage_write(move |db| db.reject_publish_start(&id, &digest, &bound, &recorded))
            .await
            .map_err(preflight::err)?;
        if receipt.state != "failed" || receipt.campaign_id.is_some() {
            // An earlier acceptance of this exact ID wins; report it, never refuse over it.
            return Ok(receipt);
        }
        return Err(CommandError {
            attempt_id: Some(request_id.into_boxed_str()),
            ..CommandError::code(error.code, error.message)
        });
    }
    let preparation_id = preparation_id.unwrap_or_else(|| Uuid::new_v4().to_string());
    Uuid::parse_str(&preparation_id).map_err(preflight::err)?;
    let (id, digest, preparation, udids) = (
        request_id.clone(),
        approved_input_digest.clone(),
        preparation_id,
        request.udids.clone(),
    );
    // Acceptance account-locks the receiving phones in the same transaction (review R3), under
    // an owner registered first (review R4-F1): if this future is dropped while the write runs,
    // the owner drops with it and the committed lock no longer binds.
    let owner = riviu_core::db::PublishPreparationOwner::register();
    let generation = owner.generation().to_owned();
    let (receipt, inserted) = state
        .db
        .storage_write(move |db| {
            db.accept_publish_start(
                &id,
                &digest,
                &fingerprint,
                &preparation,
                &generation,
                &udids,
            )
        })
        .await
        .map_err(preflight::err)?;
    if !inserted {
        return Ok(receipt);
    }
    // The owned task survives a lost IPC reply. App admission remains held until preparation
    // ends; all device work is still admitted and owned by the existing dispatcher. The owner
    // lives until the Start's terminal state is written or its retry budget is spent.
    let progress_preparation = receipt.preparation_id.clone();
    tauri::async_runtime::spawn(async move {
        let _admission = admission;
        let _owner = owner;
        let state = app.state::<AppState>();
        let context = preparation::PreparationContext::new(
            state.events.clone(),
            request_id.clone(),
            progress_preparation,
            true,
        );
        let result = std::panic::AssertUnwindSafe(preparation::CONTEXT.scope(context, async {
            let id = request_id.clone();
            let claimed = state
                .db
                .storage_write(move |db| {
                    db.advance_publish_start(&id, "accepted", "preparing", "preparingDevices", None)
                })
                .await
                .map_err(preflight::err)?;
            if !claimed {
                return Ok::<(), CommandError>(());
            }
            let campaign = execution::publish_create_campaign(
                app.clone(),
                app.state(),
                request.source_root,
                request.bundle_ids,
                request.udids,
                request.run_at,
                Some(request.caption_overrides.into_iter().collect()),
                Some(request.sound_policy),
                Some(request.sheet_enabled),
                Some(request.delete_after_publish),
                request.target_ref,
                Some(confirmed),
                approved_input_digest,
                Some(request_id.clone()),
                approved_release,
            )
            .await?;
            if campaign.run_at.is_none() {
                enqueue_created_campaign(&state, &campaign.id, confirmed).await?;
            }
            let id = request_id.clone();
            state
                .db
                .storage_write(move |db| {
                    db.advance_publish_start(&id, "preparing", "queued", "queued", None)
                })
                .await
                .map_err(preflight::err)?;
            Ok(())
        }))
        .catch_unwind()
        .await;
        let failure = match result {
            Ok(Ok(())) => None,
            Ok(Err(error)) => Some(PublishStartError {
                code: error.code,
                message: error.message.into(),
            }),
            Err(_) => Some(PublishStartError {
                code: "PublishStartInterrupted".into(),
                message: "Chuẩn bị bị gián đoạn; kiểm tra lượt cũ trước khi bắt đầu lượt mới"
                    .into(),
            }),
        };
        if let Some(error) = failure {
            // Review R4-F1: a failed terminal write is retried through storage, not abandoned.
            let mut delay = Duration::from_millis(500);
            for attempt in 1..=START_SETTLEMENT_ATTEMPTS {
                let (request_id, error) = (request_id.clone(), error.clone());
                let settled = state
                    .db
                    .storage_write(move |db| {
                        let receipt = db
                            .publish_start_status(&request_id)?
                            .context("start receipt missing")?;
                        let interrupted = error.code == "PublishStartInterrupted";
                        // Drop audit schedule_restart#4: a campaign already waiting for its
                        // hour ended this preparation, and only the Start's own last write
                        // failed. `uncertain` kept the scheduler from claiming it at its hour.
                        if !interrupted && db.publish_start_campaign_waits(&request_id)? {
                            return db.advance_publish_start(
                                &request_id,
                                &receipt.state,
                                "queued",
                                "queued",
                                None,
                            );
                        }
                        let next = if receipt.campaign_id.is_some() || interrupted {
                            "uncertain"
                        } else {
                            "failed"
                        };
                        db.advance_publish_start(
                            &request_id,
                            &receipt.state,
                            next,
                            "failed",
                            Some(&error),
                        )
                    })
                    .await;
                let Err(error) = settled else {
                    break;
                };
                if attempt == START_SETTLEMENT_ATTEMPTS {
                    log::error!(
                        "publish start settlement gave up after {attempt} attempts: {error:#}"
                    );
                    break;
                }
                log::warn!("publish start settlement attempt {attempt}: {error:#}");
                tokio::time::sleep(delay).await;
                delay = (delay * 2).min(Duration::from_secs(15));
            }
        }
    });
    Ok(receipt)
}

#[tauri::command]
pub async fn publish_exclude_assignment(
    state: State<'_, AppState>,
    assignment_id: String,
    expected_revision: i64,
    request_id: String,
) -> Result<riviu_core::db::PublishExcludeReceipt, CommandError> {
    let _admission = state.ensure_accepting_work()?;
    if state.dev_acceptance.active() {
        verification::ensure_assignment_allowed(&state.db, &assignment_id, |campaign, udid| {
            state.dev_acceptance.allows_publish_dispatch(campaign, udid)
        })?;
    }
    let receipt = state
        .db
        .storage_write(move |db| {
            db.request_publish_exclusion(&assignment_id, expected_revision, &request_id)
        })
        .await
        .map_err(preflight::err)?;
    if let Some(campaign) = state
        .db
        .publication_campaign_id(&receipt.assignment_id)
        .map_err(preflight::err)?
    {
        execution::announce(&state.events, &state.db, &campaign);
    }
    Ok(receipt)
}
