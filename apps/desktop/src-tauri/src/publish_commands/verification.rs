//! Durable, read-only recovery of a submitted post. Never re-enters the composer.
use super::*;

#[derive(Debug)]
pub(super) struct VerificationObservation {
    pub code: &'static str,
    pub reason: String,
}
impl std::fmt::Display for VerificationObservation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.reason)
    }
}
impl std::error::Error for VerificationObservation {}

pub(crate) async fn verify_pending_assignment(
    control: &DeviceControlPlane,
    db: &Database,
    events: &riviu_core::events::EventBus,
    candidate: &riviu_core::db::PendingPublishVerification,
) -> anyhow::Result<bool> {
    if control.current_work_owner(&candidate.udid).is_some() {
        return Ok(false);
    }
    let detail = db
        .get_publish_campaign(&candidate.campaign_id)?
        .context("campaign missing")?;
    let assignment = detail
        .assignments
        .iter()
        .find(|a| a.id == candidate.assignment_id)
        .context("assignment missing")?;
    if assignment.effect_intent != candidate.effect_intent
        || assignment.evidence_json != candidate.evidence_json
    {
        return Ok(false);
    }
    let bundle = detail
        .bundles
        .iter()
        .find(|b| b.id == candidate.bundle_id)
        .context("bundle missing")?;
    let capture = execution::capture_confirmed_assignment_link(control, assignment, bundle).await;
    match capture {
        Ok(link) => {
            let mut evidence = execution::evidence_with_post_url(
                candidate
                    .evidence_json
                    .as_deref()
                    .and_then(|v| serde_json::from_str(v).ok()),
                &link,
            );
            let post = if evidence.get("post").is_some() {
                &mut evidence["post"]
            } else {
                &mut evidence
            };
            post["publicationVerified"] = serde_json::json!(true);
            post["state"] = serde_json::json!("posted");
            post["verificationMethod"] = serde_json::json!("ownProfileCaptionAndCanonicalLink");
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
                if let Err(error) =
                    sheet::deliver_assignment_sheet_row(db, events, &candidate.assignment_id).await
                {
                    log::warn!(
                        "verified post {} owes Sheet delivery: {error}",
                        candidate.assignment_id
                    );
                }
                // Link settlement creates a separate durable media obligation. The
                // cleanup worker reacquires the same device lease and rechecks revision.
                if let Some(cleanup) = db.pending_publish_cleanup(&candidate.assignment_id)? {
                    if let Err(error) = super::verified_cleanup::cleanup_verified_assignment(
                        control, db, events, &cleanup,
                    )
                    .await
                    {
                        log::warn!(
                            "verified post {} still owes media cleanup: {error}",
                            candidate.assignment_id
                        );
                    }
                }
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
            if db.record_publish_verification_observation(candidate, &reason, code)? {
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
