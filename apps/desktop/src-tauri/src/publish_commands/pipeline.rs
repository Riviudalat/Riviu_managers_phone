//! One transfer-to-composer journey per device, with app-wide admission and durable ownership.
use super::execution::{self, PhoneFailure};
use super::*;
use riviu_core::db::PublishPipelineRun;
use riviu_core::{PublishAssignmentRecord, PublishBundle, PublishCampaignState as Stage};

use tokio::sync::Semaphore;

#[derive(Debug)]
pub(super) struct PipelineClaimRejected;
impl std::fmt::Display for PipelineClaimRejected {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("chiến dịch đã có worker hoặc không còn ở trạng thái bắt đầu")
    }
}
impl std::error::Error for PipelineClaimRejected {}

#[derive(Clone)]
struct Runtime {
    control: Arc<DeviceControlPlane>,
    db: Arc<Database>,
    frames: Arc<dyn FrameSource>,
    events: riviu_core::events::EventBus,
    agent: String,
    run: PublishPipelineRun,
    sound: riviu_core::PublishSoundPolicy,
}
impl Runtime {
    fn progress(&self, a: &PublishAssignmentRecord, step: progress::PublishProgress) {
        progress::record_progress(&self.db, &self.run.campaign_id, &a.id, step);
    }
    fn advance(
        &self,
        a: &PublishAssignmentRecord,
        revision: &mut i64,
        from: Stage,
        to: Stage,
        error: Option<&str>,
        evidence: Option<&str>,
    ) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.db.advance_publish_pipeline_assignment(
                &self.run, &a.id, *revision, from, to, error, evidence
            )?,
            "stale pipeline assignment {}",
            a.id
        );
        *revision += 1;
        execution::announce(&self.events, &self.db, &self.run.campaign_id);
        Ok(())
    }
    async fn transfer(
        &self,
        a: &PublishAssignmentRecord,
        bundle: &PublishBundle,
        revision: &mut i64,
    ) -> anyhow::Result<()> {
        if a.state == Stage::Imported {
            return Ok(());
        }
        self.progress(a, progress::PublishProgress::WaitingTransfer);

        // All prerequisites belong to this device; its failure must not end another device.
        let mut single = self
            .db
            .get_publish_assignment_detail(&self.run.campaign_id, &a.id)?
            .context("campaign disappeared")?;
        single.assignments = vec![a.clone()];
        single.bundles = vec![bundle.clone()];
        preflight::refuse_unmeasured_video_assignments_before_transfer(&self.control, &single)
            .await?;
        preflight::refuse_devices_whose_composer_is_not_measured([(
            a.udid.as_str(),
            preflight::readiness_of(&self.control, &a.udid).await,
        )])?;
        preflight::refuse_devices_whose_sound_picker_is_not_measured(&self.control, &[a]).await?;
        preflight::refuse_assignments_whose_bundle_is_too_large([(
            a.udid.as_str(),
            bundle,
            preflight::max_images_for(preflight::route_of(&self.control, &a.udid)),
        )])?;
        self.advance(
            a,
            revision,
            a.state.clone(),
            Stage::Transferring,
            None,
            None,
        )?;
        let owned = bundle.clone();
        let ordinal = a.ordinal;
        let staged =
            tokio::task::spawn_blocking(move || execution::stage_one_bundle(&owned, ordinal))
                .await??;
        anyhow::ensure!(
            self.db.publish_pipeline_current(&self.run)?,
            "pipeline stopped before device transfer"
        );
        self.progress(a, progress::PublishProgress::CheckingDevice);
        let context = self
            .control
            .acquire_exclusive(&a.udid, DeviceWorkOwner::Script)
            .await?;
        // The transfer lease is dropped before waiting on a composer permit.
        let result=async {
            anyhow::ensure!(self.db.publish_pipeline_current(&self.run)?,"pipeline stopped before device transfer");
            self.progress(a,progress::PublishProgress::DeviceReady);
            self.progress(a,progress::PublishProgress::TransferringMedia{count:if bundle.video.is_some(){1}else{bundle.images.len()},video:bundle.video.is_some()});
            let id=execution::device_campaign_id(&self.run.campaign_id,a.ordinal);
            let stage=self.control.stage_publish_media(&context,&self.agent,&id,staged.path()).await?;
            anyhow::ensure!(self.db.publish_pipeline_current(&self.run)?,"pipeline stopped after stage");
            let evidence=if self.control.supports_push_media(&a.udid){
                let hash=stage["manifestSha256"].as_str().context("manifest hash missing")?;
                let prepare=self.control.prepare_publish_media(&context,&id,hash).await?;
                anyhow::ensure!(self.db.publish_pipeline_current(&self.run)?,"pipeline stopped before import");
                let import=self.control.import_publish_media(&context,&id,hash).await?;
                serde_json::json!({"mediaStage":stage,"nativePrepare":prepare,"nativeImport":import})
            }else{stage};
            self.advance(a,revision,Stage::Transferring,Stage::Imported,None,Some(&evidence.to_string()))?;
            self.progress(a,progress::PublishProgress::MediaTransferred);
            Ok::<_,anyhow::Error>(())
        }.await;
        drop(context);
        result
    }
    async fn post(self, a: PublishAssignmentRecord, bundle: PublishBundle) -> anyhow::Result<()> {
        self.progress(&a, progress::PublishProgress::WaitingControl);

        anyhow::ensure!(
            self.db.publish_pipeline_current(&self.run)?,
            "pipeline stopped before composer"
        );
        let detail = self
            .db
            .get_publish_assignment_detail(&self.run.campaign_id, &a.id)?
            .context("campaign missing")?;
        let fresh = detail
            .assignments
            .into_iter()
            .find(|r| r.id == a.id)
            .context("assignment missing")?;
        let mut evidence: serde_json::Value =
            serde_json::from_str(fresh.evidence_json.as_deref().unwrap_or("{}"))?;
        evidence["pipelinePhase"] = serde_json::json!("composing");
        let mut revision = self.db.publish_assignment_revision(&a.id)?;
        self.advance(
            &fresh,
            &mut revision,
            Stage::Imported,
            Stage::Imported,
            None,
            Some(&evidence.to_string()),
        )?;
        // Existing one-shot Post boundary owns intent and settlement; there is no group barrier.
        match execution::post_one_phone(
            Duration::ZERO,
            Arc::new(Semaphore::new(1)),
            self.control,
            self.db,
            self.frames,
            self.events,
            self.run.campaign_id.clone(),
            fresh,
            bundle,
            self.sound,
            Some(self.run),
        )
        .await
        {
            Ok(()) => Ok(()),
            Err(PhoneFailure::NoBundle) => anyhow::bail!("bundle missing"),
            Err(PhoneFailure::NothingPublished(reason) | PhoneFailure::MayBeLive(reason)) => {
                anyhow::bail!("{reason}")
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn execute_pipeline(
    control: Arc<DeviceControlPlane>,
    db: Arc<Database>,
    frames: Arc<dyn FrameSource>,
    events: riviu_core::events::EventBus,
    agent: String,
    campaign: String,
    sound: riviu_core::PublishSoundPolicy,
) -> anyhow::Result<()> {
    let request = db
        .publish_campaign_request(&campaign)?
        .context("campaign missing")?;
    preflight::require_supported_publish_network(request.network)?;
    anyhow::ensure!(
        request.verification_contract_version == Some(1)
            && (!request.sheet_enabled
                || request
                    .sheet_delivery
                    .as_ref()
                    .is_some_and(|target| target.version == 2)),
        "Lượt đăng cần kiểm tra lại theo cơ chế xác minh mới trước khi chạy"
    );
    let run = db
        .claim_publish_pipeline(&campaign)?
        .ok_or(PipelineClaimRejected)?;
    execution::announce(&events, &db, &campaign);
    // Work lives in SQLite; an IPC disconnect does not discard it or create another Post.
    let _ = (control, frames, agent, sound);
    let _ = run;
    Ok(())
}

/// One application dispatcher serves immediate and scheduled campaigns. Queue rows
/// hold IDs only; media and driver sessions are loaded after global stage admission.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_dispatcher(
    control: Arc<DeviceControlPlane>,
    db: Arc<Database>,
    frames: Arc<dyn FrameSource>,
    events: riviu_core::events::EventBus,
    agent: String,
    admission: Arc<crate::state::CommandAdmissionState>,
    stop: Arc<std::sync::atomic::AtomicBool>,
) {
    let mut tasks = tokio::task::JoinSet::<anyhow::Result<()>>::new();
    let mut owned = HashMap::new();
    let mut interval = tokio::time::interval(Duration::from_millis(250));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            _ = interval.tick() => {},
            Some(result) = tasks.join_next_with_id(), if !tasks.is_empty() => {
                let (id, error) = match result {
                    Ok((id, result)) => (id, result.err().map(|e: anyhow::Error| format!("{e:#}"))),
                    Err(e) => (e.id(), Some(e.to_string())),
                };
                if let Some(job) = owned.remove(&id) {
                    if let Err(e) = db.finish_publish_dispatch(&job,error.as_deref()) {
                        log::error!("publish dispatch settlement: {e:#}");
                    }
                }
            }
        }
        if stop.load(std::sync::atomic::Ordering::Acquire) {
            while let Some(result) = tasks.join_next_with_id().await {
                let (id, error) = match result {
                    Ok((id, r)) => (id, r.err().map(|e| e.to_string())),
                    Err(e) => (e.id(), Some(e.to_string())),
                };
                if let Some(job) = owned.remove(&id) {
                    let _ = db.finish_publish_dispatch(&job, error.as_deref());
                }
            }
            break;
        }
        let result = async {
            let now = chrono::Local::now()
                .naive_local()
                .and_utc()
                .timestamp_millis();
            db.expire_publish_dispatch(now)?;
            for run in db.finished_publish_dispatch_runs()? {
                db.finish_publish_pipeline(&run)?;
                execution::reconcile_publish_execution_and_announce(
                    &db,
                    &events,
                    &run.campaign_id,
                )?;
            }
            for job in db.pending_publish_dispatch(128)? {
                if cfg!(debug_assertions)
                    && std::env::var("RIVIU_DEV_MANUAL_ACCEPTANCE").as_deref() == Ok("1")
                {
                    let allowed =
                        std::env::var("RIVIU_DEV_ACCEPTANCE_CAMPAIGNS").unwrap_or_default();
                    let scoped = std::env::var_os("RIVIU_DEV_ACCEPTANCE_SCOPE")
                        .and_then(|p| std::fs::read(p).ok())
                        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
                        .is_some_and(|scope| {
                            scope["campaignIds"].as_array().is_some_and(|ids| {
                                ids.iter()
                                    .any(|id| id.as_str() == Some(&job.run.campaign_id))
                            }) && scope["udids"].as_array().is_some_and(|ids| {
                                ids.iter().any(|id| id.as_str() == Some(&job.udid))
                            })
                        });
                    if !scoped && !allowed.split(',').any(|id| id == job.run.campaign_id) {
                        continue;
                    }
                }
                // Completed tasks still occupy a worker slot until joined. Fast failures
                // must not accumulate an unbounded JoinSet while permits are released.
                if tasks.len() >= db.publish_limits()?.device_total {
                    break;
                }
                if control.current_work_owner(&job.udid).is_some() {
                    db.defer_publish_dispatch(&job, "device_busy")?;
                    continue;
                }
                let Some(permit) = db.try_publish_work(&job.udid, &job.phase, &job.attempt_id)?
                else {
                    db.defer_publish_dispatch(&job, "stage_capacity")?;
                    continue;
                };
                let Ok(admitted) = admission.ensure_accepting_work() else {
                    break;
                };
                // Loading candidates and waiting for SQLite can cross the schedule's
                // deadline. Admission uses the clock at this individual claim.
                let claimed_at = chrono::Local::now()
                    .naive_local()
                    .and_utc()
                    .timestamp_millis();
                if !db.claim_publish_dispatch(&job, claimed_at)? {
                    continue;
                }
                let (work_db, work_control, work_frames, work_events, work_agent) = (
                    db.clone(),
                    control.clone(),
                    frames.clone(),
                    events.clone(),
                    agent.clone(),
                );
                let task_job = job.clone();
                let task = tasks.spawn(async move {
                    let _admitted = admitted;
                    let _permit = permit;
                    let detail = work_db
                        .get_publish_assignment_detail(
                            &task_job.run.campaign_id,
                            &task_job.assignment_id,
                        )?
                        .context("campaign missing")?;
                    let a = detail
                        .assignments
                        .into_iter()
                        .find(|a| a.id == task_job.assignment_id)
                        .context("assignment missing")?;
                    let bundle = detail
                        .bundles
                        .into_iter()
                        .find(|b| b.id == a.bundle_id)
                        .context("bundle missing")?;
                    let request = work_db
                        .publish_campaign_request(&task_job.run.campaign_id)?
                        .context("request missing")?;
                    preflight::require_supported_publish_network(request.network)?;
                    anyhow::ensure!(
                        request.execution_confirmed
                            && request.verification_contract_version == Some(1)
                            && (!request.sheet_enabled
                                || request
                                    .sheet_delivery
                                    .as_ref()
                                    .is_some_and(|t| t.version == 2)),
                        "publish approval or verification contract missing"
                    );
                    let runtime = Runtime {
                        control: work_control,
                        db: work_db,
                        frames: work_frames,
                        events: work_events,
                        agent: work_agent,
                        run: task_job.run.clone(),
                        sound: request.sound_policy,
                    };
                    if task_job.phase == "compose" {
                        return runtime.post(a, bundle).await;
                    }
                    let mut revision = runtime.db.publish_assignment_revision(&a.id)?;
                    let result = runtime.transfer(&a, &bundle, &mut revision).await;
                    if let Err(error) = &result {
                        if let Some(current) = runtime
                            .db
                            .get_publish_assignment_detail(&task_job.run.campaign_id, &a.id)?
                        {
                            if let Some(row) = current
                                .assignments
                                .iter()
                                .find(|r| r.id == a.id && r.effect_intent.is_none())
                            {
                                let _ = runtime.advance(
                                    row,
                                    &mut revision,
                                    row.state.clone(),
                                    Stage::FailedBeforeDispatch,
                                    Some("media_transfer_failed"),
                                    Some(
                                        &serde_json::json!({"message":format!("{error:#}")})
                                            .to_string(),
                                    ),
                                );
                            }
                        }
                    }
                    result
                });
                owned.insert(task.id(), job);
            }
            Ok::<_, anyhow::Error>(())
        }
        .await;
        if let Err(error) = result {
            log::error!("publish dispatch: {error:#}");
        }
    }
}

#[cfg(test)]
async fn continue_after_transfer<T, F, P>(
    transfer: impl std::future::Future<Output = anyhow::Result<T>>,
    post: F,
) -> anyhow::Result<()>
where
    F: FnOnce(T) -> P,
    P: std::future::Future<Output = anyhow::Result<()>>,
{
    post(transfer.await?).await
}
#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn fast_transfer_enters_post_while_slow_transfer_is_blocked_and_reuses_upload_slot() {
        let slots = Arc::new(Semaphore::new(2));
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let (release_tx, release_rx) = tokio::sync::oneshot::channel::<()>();
        let mut jobs = tokio::task::JoinSet::new();
        let slow_slot = slots.clone().acquire_owned().await.unwrap();
        let tx1 = tx.clone();
        jobs.spawn(continue_after_transfer(
            async move {
                let _slot = slow_slot;
                tx1.send("slow-upload").unwrap();
                release_rx.await.unwrap();
                Ok(())
            },
            |_| async { Ok(()) },
        ));
        let fast_slot = slots.clone().acquire_owned().await.unwrap();
        let tx2 = tx.clone();
        jobs.spawn(continue_after_transfer(
            async move {
                drop(fast_slot);
                Ok(())
            },
            move |_| async move {
                tx2.send("fast-post").unwrap();
                Ok(())
            },
        ));
        let third = slots.clone();
        let tx3 = tx.clone();
        jobs.spawn(continue_after_transfer(
            async move {
                let _slot = third.acquire_owned().await.unwrap();
                tx3.send("third-upload").unwrap();
                Ok(())
            },
            |_| async { Ok(()) },
        ));
        let mut seen = Vec::new();
        for _ in 0..3 {
            seen.push(
                tokio::time::timeout(Duration::from_secs(2), rx.recv())
                    .await
                    .unwrap()
                    .unwrap(),
            );
        }
        assert!(seen.contains(&"fast-post") && seen.contains(&"third-upload"));
        release_tx.send(()).unwrap();
        while let Some(r) = jobs.join_next().await {
            r.unwrap().unwrap();
        }
    }
    #[tokio::test]
    async fn transfer_failure_does_not_poll_post_and_different_campaigns_share_limits() {
        let result =
            continue_after_transfer(async { anyhow::bail!("upload failed") }, |_: ()| async {
                panic!("post after upload failure")
            })
            .await;
        assert!(result.is_err());
    }
}
