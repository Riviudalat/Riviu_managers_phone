use super::*;
use crate::{db::Database, AppEvent, DeviceControlPlane, EventBus};
use anyhow::Context;
use sha2::{Digest, Sha256};
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};
use tokio::sync::Mutex;

pub struct VerificationWorker {
    stop: Arc<AtomicBool>,
    task: Mutex<Option<tokio::task::JoinHandle<()>>>,
}
impl Drop for VerificationWorker {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}
impl VerificationWorker {
    pub fn start(
        db: Arc<Database>,
        control: Arc<DeviceControlPlane>,
        events: EventBus,
        artifacts: PathBuf,
    ) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let task = tokio::spawn(async move {
            let mut tasks = tokio::task::JoinSet::new();
            while !flag.load(Ordering::Relaxed) {
                while tasks.try_join_next().is_some() {}
                if let Ok(changed) = db.recover_comment_verifications() {
                    for campaign_id in changed {
                        events.emit(AppEvent::InteractionUpdated {
                            campaign_id,
                            revision: chrono::Utc::now().timestamp_millis() as u64,
                        });
                    }
                }
                match db.due_comment_verifications(chrono::Utc::now().timestamp_millis()) {
                    Ok(due) => {
                        for (id, device) in due {
                            if tasks.len() >= 2 {
                                break;
                            }
                            if control.current_work_owner(&device).is_some() {
                                continue;
                            }
                            match db.claim_comment_verification(
                                &id,
                                chrono::Utc::now().timestamp_millis(),
                            ) {
                                Ok(Some(job)) => {
                                    tasks.spawn(run_job(
                                        db.clone(),
                                        control.clone(),
                                        events.clone(),
                                        artifacts.clone(),
                                        job,
                                        flag.clone(),
                                    ));
                                }
                                Ok(None) => {}
                                Err(e) => tracing::warn!("comment verification claim: {e:#}"),
                            }
                        }
                    }
                    Err(e) => tracing::warn!("comment verification queue: {e:#}"),
                }
                tokio::time::sleep(Duration::from_millis(250)).await;
            }
            // Workers observe stop and always finish their device context before exit.
            while tasks.join_next().await.is_some() {}
        });
        Self {
            stop,
            task: Mutex::new(Some(task)),
        }
    }
    pub async fn shutdown(&self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(task) = self.task.lock().await.take() {
            let _ = task.await;
        }
    }
}

async fn observe(
    control: &DeviceControlPlane,
    job: &VerificationJob,
    stop: &AtomicBool,
    directory: &std::path::Path,
) -> anyhow::Result<(crate::CommentLocatorIdentity, String, Vec<u8>)> {
    // Reuse a ready app under a fresh owned session. Repeated clean starts on
    // slower phones can spend the whole readback window loading the feed.
    // Identity is still proved by the pinned post and the comment author's
    // canonical profile below, never by the screen inherited from this session.
    let device =
        crate::interaction_campaign::open_interaction_context(control, &job.context.device_id)
            .await?;
    let package = device.target_package;
    let context = device.context;
    let remaining =
        (job.deadline_ms - chrono::Utc::now().timestamp_millis()).clamp(1, 115000) as u64;
    let observation_end = tokio::time::Instant::now() + Duration::from_millis(remaining);
    let result = async {
        let session = control.streaming_session(&context)?;
        anyhow::ensure!(
            session.supports_accessibility_readback(),
            "comment_verification_android_required"
        );
        session.set_gui_scope(crate::ui_automation::GuiScope {
            run_id: job.campaign_id.clone(),
            assignment_id: Some(job.assignment_id.clone()),
            device_id: job.context.device_id.clone(),
            deadline_ms: Some(job.deadline_ms),
        });
        let (_, version, language) = control.tiktok_build(&job.context.device_id).await?;
        let labels = crate::tiktok_labels::controls_for_runtime(&package, &language, &version)
            .context("comment_verification_unsupported")?;
        // This path only reads. Verify the actual comment author's profile below;
        // navigating through our own profile first wastes the readback budget and
        // can replace the pinned post with a feed LIVE card on return.
        let account = &job.context.account;
        anyhow::ensure!(
            !account.trim().is_empty(),
            "comment_verification_account_missing"
        );
        // The inherited screen may be the feed. Dispatch the saved URL before
        // attempting Share/Copy: proving the unrelated feed can consume the
        // entire readback budget. The resolver still owns the single dispatch
        // and requires the target's exact canonical URL before reading comments.
        crate::interaction_hierarchy::open_pinned_target_by_hierarchy(
            session.as_ref(),
            labels,
            &package,
            &job.context.target,
            stop,
        )
        .await?;
        search::open_drawer(session.as_ref(), labels).await?;
        let mut found = search::find(
            session.as_ref(),
            labels,
            &job.context.text,
            None,
            job.context.root.as_ref(),
            stop,
        )
        .await?;
        if let Some(parent) = &job.context.parent {
            found = search::reveal_parent(session.as_ref(), labels, found, parent, stop).await?;
        }
        search::verify_author(session.as_ref(), labels, &found, account).await?;
        // Profile navigation invalidates the old tree; prove the row again on return.
        found = search::find(
            session.as_ref(),
            labels,
            &job.context.text,
            Some(&found.identity.author_label),
            job.context.root.as_ref(),
            stop,
        )
        .await?;
        if let Some(parent) = &job.context.parent {
            found = search::reveal_parent(session.as_ref(), labels, found, parent, stop).await?;
        }
        let png = session.screenshot_png().await?;
        found.identity.frame_sha256 = format!("{:x}", Sha256::digest(&png));
        // Optional enrichment after the comment and author are already proved.
        // A missing link never changes that verdict or reopens the Send boundary.
        let link_window = observation_end
            .saturating_duration_since(tokio::time::Instant::now())
            .saturating_sub(Duration::from_secs(3))
            .min(Duration::from_secs(45));
        if crate::tiktok_comment_link::supported(labels) && link_window >= Duration::from_secs(15) {
            match tokio::time::timeout(
                link_window,
                crate::tiktok_comment_link::capture(
                    session.as_ref(),
                    labels,
                    &found.identity,
                    &job.context.target.content_id,
                    stop,
                ),
            )
            .await
            {
                Ok(Ok(link)) => found.identity.comment_link = Some(link),
                _ => tracing::debug!(
                    "comment link unavailable; retaining measured text/author identity"
                ),
            }
        }
        Ok::<_, anyhow::Error>((found.identity, found.snapshot, png))
    };
    let result = tokio::select! {
        r=tokio::time::timeout_at(observation_end,result)=>r.context("comment_verification_timeout").and_then(|r|r),
        _=async {while !stop.load(Ordering::Relaxed){tokio::time::sleep(Duration::from_millis(50)).await}}=>Err(anyhow::anyhow!("comment_verification_stopped"))
    };
    if result.is_err() {
        if let Ok(session) = control.streaming_session(&context) {
            let _ = std::fs::create_dir_all(directory);
            if let Ok(Ok(snapshot)) =
                tokio::time::timeout(Duration::from_secs(2), session.hierarchy_source_snapshot())
                    .await
            {
                let _ = std::fs::write(directory.join("failure.xml"), snapshot.xml);
            }
            if let Ok(Ok(png)) =
                tokio::time::timeout(Duration::from_secs(2), session.screenshot_png()).await
            {
                let _ = std::fs::write(directory.join("failure.png"), png);
            }
        }
    }
    let cleanup = control.finish_app_session(context, &package).await;
    if let Err(e) = cleanup {
        tracing::warn!("comment verification cleanup: {e:#}");
    }
    result
}

async fn run_job(
    db: Arc<Database>,
    control: Arc<DeviceControlPlane>,
    events: EventBus,
    artifacts: PathBuf,
    job: VerificationJob,
    stop: Arc<AtomicBool>,
) {
    let directory = artifacts
        .join("comment-verification")
        .join(&job.assignment_id)
        .join(&job.owner);
    let observed = observe(&control, &job, &stop, &directory).await;
    let mut identity = None;
    let mut reason = None;
    let mut proof = serde_json::json!({"attempt":job.attempts,"checkedAt":chrono::Utc::now().to_rfc3339(),"directory":directory});
    match observed {
        Ok((id, xml, png)) => {
            proof["snapshotSha256"] = format!("{:x}", Sha256::digest(xml.as_bytes())).into();
            proof["screenshotSha256"] = format!("{:x}", Sha256::digest(&png)).into();
            let directory = artifacts
                .join("comment-verification")
                .join(&job.assignment_id)
                .join(&job.owner);
            let written = (|| -> anyhow::Result<()> {
                std::fs::create_dir_all(&directory)?;
                std::fs::write(directory.join("snapshot.xml"), xml)?;
                std::fs::write(directory.join("screen.png"), png)?;
                Ok(())
            })();
            match written {
                Ok(()) => {
                    proof["directory"] = directory.to_string_lossy().into_owned().into();
                    proof["identity"] = serde_json::to_value(&id).unwrap();
                    identity = Some(id)
                }
                Err(e) => reason = Some(format!("comment_evidence_write_failed: {e:#}")),
            }
        }
        Err(e) => reason = Some(format!("{e:#}")),
    }
    proof["reason"] = serde_json::to_value(&reason).unwrap();
    if let Err(e) = db.settle_comment_verification(
        &job,
        identity.as_ref(),
        &proof.to_string(),
        reason.as_deref(),
        chrono::Utc::now().timestamp_millis(),
    ) {
        tracing::warn!("comment verification settlement: {e:#}");
    }
    events.emit(AppEvent::InteractionUpdated {
        campaign_id: job.campaign_id,
        revision: chrono::Utc::now().timestamp_millis() as u64,
    });
}
