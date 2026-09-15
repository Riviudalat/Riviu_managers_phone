//! Durable, read-only recovery of a submitted post. Never re-enters the composer.
use super::*;

#[tauri::command]
pub async fn publish_check_links(
    state: State<'_, AppState>,
    campaign_id: String,
    udid: Option<String>,
) -> Result<serde_json::Value, CommandError> {
    let _admission = state.ensure_accepting_work()?;
    let rows = state
        .db
        .publish_verifications_for_campaign(&campaign_id, 1000)
        .map_err(preflight::err)?;
    let mut outcomes = Vec::new();
    for row in rows
        .into_iter()
        .filter(|row| udid.as_ref().is_none_or(|id| id == &row.udid))
    {
        let result =
            verify_pending_assignment_inner(&state.control, &state.db, &state.events, &row, true)
                .await;
        outcomes.push(serde_json::json!({"assignmentId":row.assignment_id,"udid":row.udid,"verified":matches!(result,Ok(true)),"error":result.err().map(|e|e.to_string())}));
    }
    Ok(serde_json::json!({"campaignId":campaign_id,"outcomes":outcomes}))
}

#[derive(Debug)]
pub(super) struct VerificationObservation {
    pub code: &'static str,
    pub reason: String,
    pub diagnostic: Option<serde_json::Value>,
}
impl std::fmt::Display for VerificationObservation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.reason)
    }
}
impl std::error::Error for VerificationObservation {}

/// Only this observer's publication is hydrated. Other devices' media and the
/// campaign event history do not belong in each concurrent verification task.
pub(super) fn verification_input(
    db: &Database,
    candidate: &riviu_core::db::PendingPublishVerification,
) -> anyhow::Result<
    Option<(
        riviu_core::PublishAssignmentRecord,
        riviu_core::PublishBundle,
    )>,
> {
    let detail = db
        .get_publish_assignment_detail(&candidate.campaign_id, &candidate.assignment_id)?
        .context("campaign missing")?;
    let assignment = detail
        .assignments
        .into_iter()
        .find(|a| a.id == candidate.assignment_id)
        .context("assignment missing")?;
    if assignment.effect_intent != candidate.effect_intent
        || assignment.evidence_json != candidate.evidence_json
    {
        return Ok(None);
    }
    let bundle = detail
        .bundles
        .into_iter()
        .find(|b| b.id == candidate.bundle_id)
        .context("bundle missing")?;
    Ok(Some((assignment, bundle)))
}

pub(crate) async fn verify_pending_assignment(
    control: &DeviceControlPlane,
    db: &Database,
    events: &riviu_core::events::EventBus,
    candidate: &riviu_core::db::PendingPublishVerification,
) -> anyhow::Result<bool> {
    if db.publish_operation_stopped(&candidate.campaign_id)? {
        return Ok(false);
    }
    verify_pending_assignment_inner(control, db, events, candidate, false).await
}

async fn verify_pending_assignment_inner(
    control: &DeviceControlPlane,
    db: &Database,
    events: &riviu_core::events::EventBus,
    candidate: &riviu_core::db::PendingPublishVerification,
    manual: bool,
) -> anyhow::Result<bool> {
    if db.publish_operation_stopped(&candidate.campaign_id)? {
        return Ok(false);
    }
    if control.current_work_owner(&candidate.udid).is_some() {
        return Ok(false);
    }
    let Some(permit) = db.try_publish_work(&candidate.udid, "verify", &candidate.assignment_id)?
    else {
        return Ok(false);
    };
    let Some((assignment, bundle)) = verification_input(db, candidate)? else {
        return Ok(false);
    };
    let capture =
        execution::capture_confirmed_assignment_link(db, control, &assignment, &bundle).await;
    drop(permit);
    match capture {
        Ok(captured) => {
            let link = captured.url;
            let mut evidence = execution::evidence_with_post_url(
                candidate
                    .evidence_json
                    .as_deref()
                    .and_then(|v| serde_json::from_str(v).ok()),
                &link,
            );
            evidence["verificationDiagnostic"] = captured.diagnostic;
            let expanded =
                evidence["verificationDiagnostic"]["stage"] == "expandedPhotoPublicProof";
            let video_public = evidence["verificationDiagnostic"]["stage"] == "videoPublicProof";
            let post = if evidence.get("post").is_some() {
                &mut evidence["post"]
            } else {
                &mut evidence
            };
            post["publicationVerified"] = serde_json::json!(true);
            post["state"] = serde_json::json!("posted");
            post["verificationMethod"] = if video_public {
                serde_json::json!("videoCanonicalPublicMetadata")
            } else if expanded {
                serde_json::json!("expandedPhotoCaptionCanonicalPublicMetadata")
            } else {
                serde_json::json!("ownProfileCaptionAndCanonicalLink")
            };
            let changed = db.record_verified_publish_with_sheet_row(
                candidate,
                &evidence.to_string(),
                &link,
                execution::poster_identity(),
                &bundle.partners,
            )?;
            if changed {
                progress::record_progress(
                    db,
                    &candidate.campaign_id,
                    &candidate.assignment_id,
                    riviu_core::tiktok_composer::PublishProgress::PostConfirmed,
                );
                execution::announce(events, db, &candidate.campaign_id);
            }
            Ok(changed)
        }
        Err(error) => {
            if matches!(
                error.downcast_ref::<riviu_core::DeviceControlError>(),
                Some(riviu_core::DeviceControlError::Busy(_))
            ) {
                return Ok(false);
            }
            // The stale observer can never overwrite a newer link or cancelled assignment.
            let reason: String = error.to_string().chars().take(512).collect();
            let code = error
                .downcast_ref::<VerificationObservation>()
                .map_or("readFailed", |o| o.code);
            let record = if manual {
                Database::record_manual_publish_verification_diagnostic
            } else {
                Database::record_publish_verification_diagnostic
            };
            if record(
                db,
                candidate,
                &reason,
                code,
                error
                    .downcast_ref::<VerificationObservation>()
                    .and_then(|observation| observation.diagnostic.as_ref()),
            )? {
                progress::record_progress(
                    db,
                    &candidate.campaign_id,
                    &candidate.assignment_id,
                    progress::PublishProgress::LinkPending { reason },
                );
                execution::announce(events, db, &candidate.campaign_id);
            }
            Ok(false)
        }
    }
}
