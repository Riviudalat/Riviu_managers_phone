//! Idempotent cleanup of one imported media set after canonical publication proof.
use super::*;

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
            .is_none_or(|request| request.verification_contract_version != Some(1))
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
