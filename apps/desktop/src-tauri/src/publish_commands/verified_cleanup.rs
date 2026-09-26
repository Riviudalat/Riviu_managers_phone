//! Idempotent cleanup of one imported media set after canonical publication proof.
use super::*;

#[tauri::command]
pub async fn publish_cleanup_verified_assignment(
    state: State<'_, AppState>,
    assignment_id: String,
    expected_revision: i64,
) -> Result<serde_json::Value, CommandError> {
    let _admission = state.ensure_accepting_work()?;
    manual_cleanup_verified_assignment(
        &state.dev_acceptance,
        &state.control,
        &state.db,
        &state.events,
        &assignment_id,
        expected_revision,
    )
    .await
}

async fn manual_cleanup_verified_assignment(
    policy: &crate::dev_acceptance::DevAcceptancePolicy,
    control: &DeviceControlPlane,
    db: &Database,
    events: &riviu_core::events::EventBus,
    assignment_id: &str,
    expected_revision: i64,
) -> Result<serde_json::Value, CommandError> {
    use crate::dev_acceptance::AcceptanceCapability;

    if !cfg!(debug_assertions) || !policy.active() {
        return Err(verification::acceptance_scope_denied());
    }
    verification::ensure_assignment_allowed(db, assignment_id, |campaign, udid| {
        policy.allows(AcceptanceCapability::PublishCleanup, campaign, udid)
    })?;
    let campaign_id = db
        .publication_campaign_id(assignment_id)
        .map_err(preflight::err)?
        .ok_or_else(|| CommandError::invalid_argument("Publication assignment not found"))?;
    let request = db
        .publish_campaign_request(&campaign_id)
        .map_err(preflight::err)?
        .ok_or_else(|| CommandError::invalid_argument("Publication campaign not found"))?;
    request
        .network
        .ensure_implemented()
        .map_err(preflight::err)?;
    if request.verification_contract_version != Some(1)
        || request.cleanup_policy != PublishCleanupPolicy::DeleteImportedAssetsAfterVerified
    {
        return Err(CommandError::code(
            "PublishCleanupNotEligible",
            "This campaign has no verified media-cleanup obligation",
        ));
    }
    let current_revision = db
        .publish_assignment_revision(assignment_id)
        .map_err(preflight::err)?;
    if current_revision != expected_revision {
        return Err(CommandError::code(
            "PublishRevisionConflict",
            "Publication changed; reload its progress before cleanup",
        ));
    }
    let candidate = db
        .pending_publish_cleanup(assignment_id)
        .map_err(preflight::err)?;
    if let Some(candidate) = candidate.as_ref() {
        if candidate.revision != expected_revision
            || candidate.campaign_id != campaign_id
            || !policy.allows(
                AcceptanceCapability::PublishCleanup,
                &candidate.campaign_id,
                &candidate.udid,
            )
        {
            return Err(verification::acceptance_scope_denied());
        }
        cleanup_verified_assignment_guarded(control, db, events, candidate, || {
            policy.allows(
                AcceptanceCapability::PublishCleanup,
                &candidate.campaign_id,
                &candidate.udid,
            )
        })
        .await
        .map_err(preflight::err)?;
    }
    let result =
        cleanup_redacted_readback(db, &campaign_id, assignment_id).map_err(preflight::err)?;
    if candidate.is_none() && result["state"] != "cleaned" {
        return Err(CommandError::code(
            "PublishCleanupNotEligible",
            "Publication has no pending verified media-cleanup obligation",
        ));
    }
    Ok(result)
}

fn cleanup_redacted_readback(
    db: &Database,
    campaign_id: &str,
    assignment_id: &str,
) -> anyhow::Result<serde_json::Value> {
    let detail = db
        .get_publish_assignment_detail(campaign_id, assignment_id)?
        .context("Publication assignment not found")?;
    let assignment = detail
        .assignments
        .into_iter()
        .find(|assignment| assignment.id == assignment_id)
        .context("Publication assignment not found")?;
    anyhow::ensure!(
        assignment.state == riviu_core::PublishCampaignState::Succeeded,
        "Publication is not verified"
    );
    let evidence: serde_json::Value =
        serde_json::from_str(assignment.evidence_json.as_deref().unwrap_or("{}"))?;
    let post = evidence.get("post").unwrap_or(&evidence);
    let url = post
        .get("postUrl")
        .and_then(serde_json::Value::as_str)
        .context("Verified canonical URL is missing")?;
    let targets = riviu_core::interaction::parse_tiktok_links(url);
    anyhow::ensure!(
        post.get("publicationVerified")
            .and_then(serde_json::Value::as_bool)
            == Some(true)
            && targets.len() == 1
            && targets[0]
                .target
                .as_ref()
                .is_some_and(|target| target.normalized_url == url),
        "Publication proof is not canonical"
    );
    let cleanup = evidence.get("cleanup");
    let cleaned = cleanup
        .and_then(|value| {
            let native = value.get("value").unwrap_or(value);
            let expected_import = value
                .get("importId")
                .and_then(serde_json::Value::as_str)
                .or_else(|| post.get("importId").and_then(serde_json::Value::as_str));
            (native.get("state").and_then(serde_json::Value::as_str) == Some("cleaned")
                && native.get("importId").and_then(serde_json::Value::as_str) == expected_import
                && expected_import.is_some())
            .then_some(())
        })
        .is_some();
    Ok(serde_json::json!({
        "assignmentId": assignment_id,
        "campaignId": campaign_id,
        "udid": assignment.udid,
        "revision": db.publish_assignment_revision(assignment_id)?,
        "state": if cleaned { "cleaned" } else { "pending" },
        "proof": {
            "publicationVerified": true,
            "postUrl": url,
            "cleanupState": if cleaned { "cleaned" } else { "notCleaned" },
            "checkedAt": cleanup.and_then(|value| value.get("checkedAt")).and_then(serde_json::Value::as_str),
        }
    }))
}

pub(crate) async fn cleanup_verified_assignments(
    control: &DeviceControlPlane,
    db: &Database,
    events: &riviu_core::events::EventBus,
    limit: usize,
) -> anyhow::Result<usize> {
    let candidates = db.pending_publish_cleanups(limit)?;
    let mut completed = 0;
    for candidate in candidates {
        if db
            .publish_campaign_request(&candidate.campaign_id)?
            .is_none_or(|request| {
                !request.network.is_implemented()
                    || request.verification_contract_version != Some(1)
            })
        {
            continue;
        }
        if cleanup_verified_assignment(control, db, events, &candidate).await? {
            completed += 1;
        }
    }
    Ok(completed)
}

pub(super) async fn cleanup_verified_assignment(
    control: &DeviceControlPlane,
    db: &Database,
    events: &riviu_core::events::EventBus,
    candidate: &riviu_core::db::PendingPublishCleanup,
) -> anyhow::Result<bool> {
    cleanup_verified_assignment_guarded(control, db, events, candidate, || true).await
}

async fn cleanup_verified_assignment_guarded(
    control: &DeviceControlPlane,
    db: &Database,
    events: &riviu_core::events::EventBus,
    candidate: &riviu_core::db::PendingPublishCleanup,
    allowed: impl FnOnce() -> bool + Send,
) -> anyhow::Result<bool> {
    db.publish_campaign_request(&candidate.campaign_id)?
        .context("publish campaign request not found")?
        .network
        .ensure_implemented()?;
    let Some(_permit) =
        db.try_publish_work(&candidate.udid, "cleanup", &candidate.assignment_id)?
    else {
        return Ok(false);
    };
    let context = match control
        .try_acquire_exclusive_keeping_stream(&candidate.udid, DeviceWorkOwner::Script)
        .await
    {
        Ok(context) => context,
        Err(error) => {
            log::debug!(
                "deferred media cleanup waits for {}: {error}",
                candidate.assignment_id
            );
            return Ok(false);
        }
    };
    let outcome: anyhow::Result<bool> = async {
        // The lease makes the pending check and cleanup one device operation. A new
        // publishing worker cannot begin between this check and native deletion.
        if db.has_pending_publish_for_device(&candidate.udid)? {
            return Ok(false);
        }
        if !allowed() {
            return Ok(false);
        }
        let Some(claimed) = db.claim_publish_cleanup(candidate)? else {
            return Ok(false);
        };
        let cleanup = control
            .cleanup_publish_media(&context, &claimed.import_id)
            .await;
        let proof = cleanup_result(
            &claimed.import_id,
            cleanup.map_err(|error| error.to_string()),
        );
        let cleaned = proof.get("state").and_then(|value| value.as_str()) == Some("cleaned");
        let changed = db.finish_publish_cleanup(&claimed, &proof)?;
        if changed {
            execution::announce(events, db, &claimed.campaign_id);
        }
        Ok(changed && cleaned)
    }
    .await;
    // Preserve the warm app and preview. This worker only fulfills the media
    // obligation; it never opens a composer, starts TikTok or terminates an app.
    let released = control.close_exclusive_context(context);
    match (outcome, released) {
        (Err(error), _) => Err(error),
        (Ok(_), Err(error)) => Err(error.into()),
        (Ok(cleaned), Ok(_)) => Ok(cleaned),
    }
}

fn cleanup_result(import: &str, result: Result<serde_json::Value, String>) -> serde_json::Value {
    match result {
        Ok(value) => {
            let native = value.get("value").unwrap_or(&value);
            let cleaned = native.get("state").and_then(|v| v.as_str()) == Some("cleaned");
            let same_import = native.get("importId").and_then(|v| v.as_str()) == Some(import);
            if cleaned && same_import {
                return serde_json::json!({"state":"cleaned","importId":import,"nativeProof":value,
                    "appCleanup":{"state":"leftRunning","reason":"deferred_media_cleanup_preserves_app"}});
            }
            serde_json::json!({"state":"not_cleaned","importId":import,"message":"native cleanup did not prove the expected import was cleaned","nativeProof":value,
                "appCleanup":{"state":"leftRunning"}})
        }
        Err(error) => {
            serde_json::json!({"state":"not_cleaned","importId":import,"message":error.chars().take(512).collect::<String>(),
            "appCleanup":{"state":"leftRunning"}})
        }
    }
}

#[cfg(test)]
#[path = "verified_cleanup_tests.rs"]
mod tests;
