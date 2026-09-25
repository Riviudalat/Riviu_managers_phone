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
    let capabilities = state
        .db
        .publish_recovery_capabilities(&campaign_id)
        .map_err(preflight::err)?;
    let mut outcomes = Vec::new();
    let mut statuses = Vec::new();
    for capability in capabilities {
        let detail = state
            .db
            .get_publish_assignment_detail(&campaign_id, &capability.assignment_id)
            .map_err(preflight::err)?;
        let Some(assignment) = detail.and_then(|d| d.assignments.into_iter().next()) else {
            continue;
        };
        if udid.as_ref().is_some_and(|id| id != &assignment.udid) {
            continue;
        }
        let row = eligible_check_candidate(&capability, &rows);
        let (status, error) = if let Some(row) = row {
            match verify_pending_assignment_inner(
                &state.control,
                &state.db,
                &state.events,
                row,
                true,
            )
            .await
            {
                Ok(outcome) => outcome,
                Err(error) => (CheckStatus::Ineligible, Some(error.to_string())),
            }
        } else {
            unavailable_check_outcome(capability.check_link.reason)
        };
        statuses.push(status);
        outcomes.push(serde_json::json!({"assignmentId":assignment.id,"udid":assignment.udid,"verified":status==CheckStatus::Verified,"error":error,"status":status}));
    }
    let status = check_summary(&statuses);
    Ok(
        serde_json::json!({"campaignId":campaign_id,"state":status,"reason":if statuses.is_empty(){Some("noCandidate")}else{None},"outcomes":outcomes}),
    )
}

pub(super) fn eligible_check_candidate<'a>(
    capability: &riviu_core::db::PublishRecoveryCapabilities,
    candidates: &'a [riviu_core::db::PendingPublishVerification],
) -> Option<&'a riviu_core::db::PendingPublishVerification> {
    capability
        .check_link
        .allowed
        .then(|| {
            candidates
                .iter()
                .find(|row| row.assignment_id == capability.assignment_id)
        })
        .flatten()
}

#[tauri::command]
pub fn publish_recovery_capabilities(
    state: State<'_, AppState>,
    campaign_id: String,
) -> Result<Vec<riviu_core::db::PublishRecoveryCapabilities>, CommandError> {
    let _admission = state.ensure_accepting_work()?;
    state
        .db
        .publish_recovery_capabilities(&campaign_id)
        .map_err(preflight::err)
}

#[tauri::command]
pub fn publish_resume_verification(
    state: State<'_, AppState>,
    assignment_id: String,
    confirmed: bool,
    expected_revision: i64,
) -> Result<riviu_core::db::PublishResumeVerificationResult, CommandError> {
    let _admission = state.ensure_accepting_work()?;
    resume_verification_and_announce(
        &state.db,
        &state.events,
        &assignment_id,
        confirmed,
        expected_revision,
    )
    .map_err(preflight::err)
}

pub(super) fn resume_verification_and_announce(
    db: &Database,
    events: &riviu_core::events::EventBus,
    assignment_id: &str,
    confirmed: bool,
    expected_revision: i64,
) -> anyhow::Result<riviu_core::db::PublishResumeVerificationResult> {
    let campaign = db.publication_campaign_id(assignment_id)?;
    let result = db.resume_publish_verification(assignment_id, confirmed, expected_revision)?;
    if result.state == riviu_core::db::PublishResumeVerificationState::Accepted {
        if let Some(campaign) = campaign {
            execution::announce(events, db, &campaign);
        }
    }
    Ok(result)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) enum CheckStatus {
    Verified,
    Pending,
    Busy,
    Stopped,
    Stale,
    NoCandidate,
    Ineligible,
}

pub(super) fn unavailable_check_outcome(reason: Option<String>) -> (CheckStatus, Option<String>) {
    let status = match reason.as_deref() {
        Some("alreadyVerified") => CheckStatus::Verified,
        Some("operatorStopped") => CheckStatus::Stopped,
        Some("activePipeline") => CheckStatus::Busy,
        Some("noCandidate") | None => CheckStatus::NoCandidate,
        _ => CheckStatus::Ineligible,
    };
    (
        status,
        if status == CheckStatus::Verified {
            None
        } else {
            reason
        },
    )
}

pub(super) fn check_summary(statuses: &[CheckStatus]) -> CheckStatus {
    if statuses.is_empty() {
        return CheckStatus::NoCandidate;
    }
    for status in [
        CheckStatus::Stale,
        CheckStatus::Stopped,
        CheckStatus::Busy,
        CheckStatus::Pending,
        CheckStatus::Ineligible,
        CheckStatus::NoCandidate,
    ] {
        if statuses.contains(&status) {
            return status;
        }
    }
    CheckStatus::Verified
}

pub(super) fn committed_observation_outcome(
    db: &Database,
    candidate: &riviu_core::db::PendingPublishVerification,
) -> anyhow::Result<(CheckStatus, Option<String>)> {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    enum State {
        Pending,
        NeedsReview,
        Verified,
    }
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Status {
        state: State,
        reason: Option<String>,
        next_check_at: Option<String>,
    }
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Evidence {
        verification_status: Status,
    }
    let detail = db
        .get_publish_assignment_detail(&candidate.campaign_id, &candidate.assignment_id)?
        .context("assignment missing after observation")?;
    let assignment = detail
        .assignments
        .into_iter()
        .next()
        .context("assignment missing after observation")?;
    let evidence: Evidence = serde_json::from_str(
        assignment
            .evidence_json
            .as_deref()
            .context("observation evidence missing")?,
    )?;
    let status = evidence.verification_status;
    Ok(match status.state {
        State::Verified => (CheckStatus::Verified, None),
        State::Pending
            if status
                .next_check_at
                .as_deref()
                .is_some_and(|s| chrono::DateTime::parse_from_rfc3339(s).is_ok()) =>
        {
            (CheckStatus::Pending, None)
        }
        _ => (
            CheckStatus::Ineligible,
            status
                .reason
                .or_else(|| Some("verificationNeedsReview".into())),
        ),
    })
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
    if db.publish_assignment_revision(&candidate.assignment_id)? != candidate.revision
        || assignment.effect_intent != candidate.effect_intent
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
    Ok(
        verify_pending_assignment_inner(control, db, events, candidate, false)
            .await?
            .0
            == CheckStatus::Verified,
    )
}

async fn verify_pending_assignment_inner(
    control: &DeviceControlPlane,
    db: &Database,
    events: &riviu_core::events::EventBus,
    candidate: &riviu_core::db::PendingPublishVerification,
    manual: bool,
) -> anyhow::Result<(CheckStatus, Option<String>)> {
    if !db.publish_verification_is_current(candidate)? {
        return Ok((CheckStatus::Stale, None));
    }
    if control.current_work_owner(&candidate.udid).is_some() {
        return Ok((CheckStatus::Busy, None));
    }
    let Some(permit) = db.try_publish_work(&candidate.udid, "verify", &candidate.assignment_id)?
    else {
        return Ok((CheckStatus::Busy, None));
    };
    let Some((assignment, bundle)) = verification_input(db, candidate)? else {
        return Ok((CheckStatus::Stale, None));
    };
    if !db.publish_verification_is_current(candidate)? {
        return Ok((CheckStatus::Stale, None));
    }
    let capture = execution::capture_confirmed_assignment_link(
        db,
        control,
        &assignment,
        &bundle,
        Some(candidate),
    )
    .await;
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
            Ok((
                if changed {
                    CheckStatus::Verified
                } else {
                    CheckStatus::Stale
                },
                None,
            ))
        }
        Err(error) => {
            if matches!(
                error.downcast_ref::<riviu_core::DeviceControlError>(),
                Some(riviu_core::DeviceControlError::Busy(_))
            ) {
                return Ok((CheckStatus::Busy, None));
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
                let outcome = committed_observation_outcome(db, candidate)?;
                if outcome.0 == CheckStatus::Pending {
                    progress::record_progress(
                        db,
                        &candidate.campaign_id,
                        &candidate.assignment_id,
                        progress::PublishProgress::LinkPending { reason },
                    );
                }
                execution::announce(events, db, &candidate.campaign_id);
                return Ok(outcome);
            }
            Ok((CheckStatus::Stale, None))
        }
    }
}
