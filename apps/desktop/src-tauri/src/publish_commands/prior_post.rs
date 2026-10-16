//! A scheduled job's release of earlier posts on its phone that TikTok already holds
//! (`docs/development/contracts.md`, "Lịch hẹn giờ trên máy còn bài chờ link").
//!
//! Only the dispatched job does this, inside its own Script lease and work permit: no other
//! owner can start on the phone meanwhile, and no lease is dropped or taken from anyone.

use super::*;
use crate::dev_acceptance::{AcceptanceCapability, DevAcceptancePolicy};
use riviu_core::db::{PriorPostScope, PublishPipelineRun};

static ACCEPTANCE: std::sync::OnceLock<DevAcceptancePolicy> = std::sync::OnceLock::new();

/// The controller's acceptance policy, installed once by the publish dispatcher.
pub(super) fn install_acceptance(policy: &DevAcceptancePolicy) {
    let _ = ACCEPTANCE.set(policy.clone());
}

/// Which earlier posts the automatic verifier still observes, as the gate and the release
/// classify them: under dev acceptance only its pinned verification scope, otherwise every post.
/// Parked rows get the automatic 30-minute re-check in both (under acceptance only in scope).
pub(super) fn with_scope<T>(work: impl FnOnce(&PriorPostScope<'_>) -> T) -> T {
    match ACCEPTANCE.get().filter(|policy| policy.active()) {
        Some(policy) => {
            let observable = |campaign: &str, udid: &str| {
                policy.allows(AcceptanceCapability::PublishVerification, campaign, udid)
            };
            work(&PriorPostScope {
                observable: &observable,
                automatic_recheck: true,
            })
        }
        None => work(&PriorPostScope::production()),
    }
}

/// Release every earlier post of another campaign on this phone that TikTok already holds, the
/// way "Thay thế đã xác nhận" does, before this job stages media or starts TikTok:
///
/// 1. one IMMEDIATE transaction re-checks the job and each post at the revision it reads and
///    excludes them (`request_prior_post_release`);
/// 2. the fences of the excluded rows;
/// 3. TikTok is closed under this job's lease;
/// 4. the release proof (`record_prior_post_release`), so the rows read `link_review`, and
///    they resume verification once this job's pipeline is done;
/// 5. any error after 1 aborts with a resume stamp: the old rows carry on verifying as before.
///
/// `Ok(())` when nothing on the phone holds the job back. Otherwise the typed Retryable
/// `prior_post_pending` failure: the job is requeued without spending its budget.
pub(super) async fn release_prior_posts(
    control: &DeviceControlPlane,
    db: &Database,
    events: Option<&riviu_core::events::EventBus>,
    context: &riviu_core::DeviceExclusiveContext,
    assignment: &str,
    run: &PublishPipelineRun,
) -> anyhow::Result<()> {
    let udid = context.udid().to_owned();
    let announce = |campaigns: Vec<String>| {
        if let Some(events) = events {
            for campaign in campaigns {
                execution::announce(events, db, &campaign);
            }
        }
    };
    let request = with_scope(|scope| db.request_prior_post_release(assignment, run, &udid, scope))?;
    let release = match request {
        Ok(None) => return Ok(()),
        Ok(Some(release)) => release,
        Err(failure) => return Err(failure.into()),
    };
    let round = run.campaign_id.get(..8).unwrap_or(&run.campaign_id);
    log::info!(
        "Lịch đăng {round} trên {udid}: tạm dừng xác minh {} bài đã lên TikTok, đóng TikTok để đăng; bài cũ tự xác minh tiếp sau khi đăng xong, không đăng lại",
        release.rows.len()
    );
    let result = async {
        let mut fences = Vec::with_capacity(release.rows.len());
        for row in &release.rows {
            fences.push(db.publish_handoff_fence(&row.assignment_id, &row.exclusion_request_id)?);
        }
        let mut packages: Vec<String> = release
            .rows
            .iter()
            .filter_map(|row| row.package.clone())
            .collect();
        if packages.len() < release.rows.len() {
            packages.push(control.resolve_tiktok_package(&udid).await?);
        }
        packages.sort();
        packages.dedup();
        let mut terminations = serde_json::Map::new();
        for package in packages {
            let proof = control.terminate_app(context, &package).await?;
            terminations.insert(package, serde_json::to_value(proof)?);
        }
        db.record_prior_post_release(&release, &fences, &serde_json::Value::Object(terminations))
    }
    .await;
    match result {
        Ok(campaigns) => {
            announce(campaigns);
            Ok(())
        }
        Err(error) => {
            let message = format!("{error:#}");
            log::warn!(
                "Lịch đăng {round} trên {udid}: chưa nhả được bài cũ ({message}); bài cũ tiếp tục được xác minh như trước, chưa đăng"
            );
            match db.abort_prior_post_release(assignment, &message) {
                Ok(campaigns) => announce(campaigns),
                // The receipt stays `requested`; the dispatcher sweeps it once this job ends.
                Err(abort) => log::error!(
                    "publish prior-post release abort pending assignment={assignment}: {abort:#}"
                ),
            }
            Err(riviu_core::publish_recovery::prior_post_pending(format!(
                "Chưa nhả được bài cũ trên máy ({message}); bài cũ tiếp tục được xác minh như trước, app thử lại sau 15 giây; chưa đăng"
            ))
            .into())
        }
    }
}
