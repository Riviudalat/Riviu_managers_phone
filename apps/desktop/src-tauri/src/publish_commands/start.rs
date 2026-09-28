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
    state
        .db
        .storage_read(move |db| db.publish_start_status(&request_id))
        .await
        .map_err(preflight::err)
}

#[tauri::command]
pub async fn publish_start(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    mut request: riviu_core::PublishPreflightRequest,
    request_id: String,
    approved_input_digest: String,
    confirmed: bool,
    preparation_id: Option<String>,
) -> Result<PublishStartReceipt, CommandError> {
    let admission = state.ensure_accepting_work()?;
    if !confirmed {
        return Err(CommandError::invalid_argument(
            "Cần xác nhận bắt đầu đúng lượt đăng",
        ));
    }
    Uuid::parse_str(&request_id).map_err(preflight::err)?;
    request.source_root = request.source_root.trim().to_owned();
    request.run_at = request.run_at.map(|at| at.trim().to_owned());
    if state.dev_acceptance.active()
        && request.udids.iter().any(|udid| {
            !state
                .dev_acceptance
                .allows_publish_dispatch(&request_id, udid)
        })
    {
        return Err(CommandError::code(
            "AcceptanceScopeDenied",
            "Lượt bắt đầu hoặc máy chưa được cấp quyền trong phạm vi nghiệm thu",
        ));
    }
    let fingerprint = execution::frame_sha256(
        &serde_json::to_vec(&serde_json::json!({"request":request,"confirmed":confirmed}))
            .map_err(preflight::err)?,
    );
    let preparation_id = preparation_id.unwrap_or_else(|| Uuid::new_v4().to_string());
    Uuid::parse_str(&preparation_id).map_err(preflight::err)?;
    let (id, digest, preparation) = (
        request_id.clone(),
        approved_input_digest.clone(),
        preparation_id,
    );
    let (receipt, inserted) = state
        .db
        .storage_write(move |db| db.accept_publish_start(&id, &digest, &fingerprint, &preparation))
        .await
        .map_err(preflight::err)?;
    if !inserted {
        return Ok(receipt);
    }
    // The owned task survives a lost IPC reply. App admission remains held until preparation
    // ends; all device work is still admitted and owned by the existing dispatcher.
    let progress_preparation = receipt.preparation_id.clone();
    tauri::async_runtime::spawn(async move {
        let _admission = admission;
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
            )
            .await?;
            if campaign.run_at.is_none() {
                execution::publish_execute(app.state(), campaign.id, confirmed).await?;
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
            let settled = state
                .db
                .storage_write(move |db| {
                    let receipt = db
                        .publish_start_status(&request_id)?
                        .context("start receipt missing")?;
                    let next = if receipt.campaign_id.is_some()
                        || error.code == "PublishStartInterrupted"
                    {
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
            if let Err(error) = settled {
                log::error!("publish start settlement: {error:#}");
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
