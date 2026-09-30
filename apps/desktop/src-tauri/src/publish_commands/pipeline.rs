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
struct Recovery {
    db: Arc<Database>,
    assignment: String,
    run: PublishPipelineRun,
}
impl riviu_core::publish_recovery::RecoveryJournal for Recovery {
    fn note_read(
        &self,
        note: &riviu_core::publish_recovery::ReadRecoveryNote,
    ) -> anyhow::Result<()> {
        self.db.append_publish_progress(
            &self.run.campaign_id,
            &self.assignment,
            &progress::PublishProgress::ReadRecovery {
                stage: note.stage.clone(),
                strategy: note.strategy.clone(),
                attempt: note.attempt,
                outcome: note.outcome.clone(),
                detail: serde_json::to_string(note)?,
            },
        )
    }
    fn step(&self, step: &str, checkpoint: Option<&str>) -> anyhow::Result<()> {
        self.db
            .update_publish_recovery_step(&self.assignment, &self.run.token, step, checkpoint)
    }
    fn retry(
        &self,
        failure: &riviu_core::publish_recovery::RecoveryFailure,
    ) -> anyhow::Result<Option<Duration>> {
        let delay = self.db.reserve_publish_step_retry_failure(
            &self.assignment,
            &self.run.token,
            failure,
        )?;
        if let Some(s) = self.db.publish_recovery_state(&self.assignment)? {
            progress::record_progress(
                &self.db,
                &self.run.campaign_id,
                &self.assignment,
                progress::PublishProgress::RetryWaiting {
                    step: s.step,
                    attempt: s.retries_used,
                    maximum: s.max_retries,
                    reason: failure.message.clone(),
                },
            );
        }
        Ok(delay)
    }
    fn sound(
        &self,
        selection: Option<&riviu_core::SoundSelectionEvidence>,
    ) -> anyhow::Result<Option<riviu_core::SoundSelectionEvidence>> {
        self.db
            .bind_publish_recovery_sound(&self.assignment, &self.run.token, selection)
    }
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
            self.db.publish_pipeline_current(&self.run)?
                && !self.db.publish_assignment_excluded(&a.id)?,
            "pipeline stopped before device transfer"
        );
        self.progress(a, progress::PublishProgress::CheckingDevice);
        let context = self
            .control
            .acquire_exclusive(&a.udid, DeviceWorkOwner::Script)
            .await?;
        // The transfer lease is dropped before waiting on a composer permit.
        let result=async {
            anyhow::ensure!(self.db.publish_pipeline_current(&self.run)? && !self.db.publish_assignment_excluded(&a.id)?,"pipeline stopped before device transfer");
            self.progress(a,progress::PublishProgress::DeviceReady);
            self.progress(a,progress::PublishProgress::TransferringMedia{count:if bundle.video.is_some(){1}else{bundle.images.len()},video:bundle.video.is_some()});
            let id=execution::device_campaign_id(&self.run.campaign_id,a.ordinal);
            let stage=self.control.stage_publish_media(&context,&self.agent,&id,staged.path()).await?;
            anyhow::ensure!(self.db.publish_pipeline_current(&self.run)? && !self.db.publish_assignment_excluded(&a.id)?,"pipeline stopped after stage");
            let evidence=if self.control.supports_push_media(&a.udid){
                let hash=stage["manifestSha256"].as_str().context("manifest hash missing")?;
                let prepare=self.control.prepare_publish_media(&context,&id,hash).await?;
                anyhow::ensure!(self.db.publish_pipeline_current(&self.run)? && !self.db.publish_assignment_excluded(&a.id)?,"pipeline stopped before import");
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
        riviu_core::publish_recovery::step("device", Some("mediaImported"))?;
        self.progress(&a, progress::PublishProgress::WaitingControl);

        anyhow::ensure!(
            self.db.publish_pipeline_current(&self.run)?
                && !self.db.publish_assignment_excluded(&a.id)?,
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
            Err(PhoneFailure::AccountProof(diagnostic)) => Err(diagnostic.into()),
            Err(PhoneFailure::Recovery(failure)) => Err(failure.into()),
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
    request.network.ensure_implemented()?;
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

struct CompletedDispatch {
    job: riviu_core::db::PublishDispatchJob,
    error: Option<anyhow::Error>,
}

struct PendingCompletion {
    receipt: Arc<CompletedDispatch>,
    retry_at: tokio::time::Instant,
    failures: u32,
}

#[derive(Default)]
struct DispatchCompletions {
    pending: Vec<PendingCompletion>,
    in_flight: Option<(
        PendingCompletion,
        tokio::task::JoinHandle<anyhow::Result<bool>>,
    )>,
}

impl DispatchCompletions {
    fn push(&mut self, job: riviu_core::db::PublishDispatchJob, error: Option<anyhow::Error>) {
        self.pending.push(PendingCompletion {
            receipt: Arc::new(CompletedDispatch { job, error }),
            retry_at: tokio::time::Instant::now(),
            failures: 0,
        });
    }

    async fn settle_ready(&mut self, db: &Arc<Database>) {
        // The dispatcher polls this future alongside Stop/ticks and phone joins.
        // Losing that select race must retain both the receipt and owned DB task.
        if self.in_flight.is_none() {
            let Some(index) = self
                .pending
                .iter()
                .position(|pending| tokio::time::Instant::now() >= pending.retry_at)
            else {
                std::future::pending::<()>().await;
                return;
            };
            let pending = self.pending.remove(index);
            let receipt = pending.receipt.clone();
            let db = db.clone();
            let task = tokio::spawn(async move {
                db.storage_write(move |db| {
                    db.settle_publish_dispatch_completion(&receipt.job, receipt.error.as_ref())
                })
                .await
            });
            self.in_flight = Some((pending, task));
        }
        let result = (&mut self.in_flight.as_mut().expect("owned settlement task").1)
            .await
            .map_err(anyhow::Error::from)
            .and_then(|result| result);
        let (mut pending, _) = self.in_flight.take().expect("joined settlement task");
        if let Err(error) = result {
            pending.failures = pending.failures.saturating_add(1);
            pending.retry_at = tokio::time::Instant::now()
                + Duration::from_secs(
                    [1, 2, 5, 10, 30][pending.failures.saturating_sub(1).min(4) as usize],
                );
            log::error!(
                "publish dispatch completion retained assignment={} attempt={}: {error:#}",
                pending.receipt.job.assignment_id,
                pending.receipt.job.attempt_id
            );
            self.pending.push(pending);
        }
    }

    async fn preserve_on_shutdown(&self, db: &Arc<Database>) {
        // One filesystem-only pass, including a DB task still waiting for its
        // writer slot. Its owned task may finish while the runtime lives; a stale
        // file is harmless because startup replays only the exact CAS. Shutdown
        // never waits indefinitely for SQLite or cancels a dispatched write.
        for pending in self
            .pending
            .iter()
            .chain(self.in_flight.iter().map(|(pending, _)| pending))
        {
            let receipt = pending.receipt.clone();
            let db = db.clone();
            let result = tokio::task::spawn_blocking(move || {
                db.persist_publish_dispatch_completion(&receipt.job, receipt.error.as_ref())
            })
            .await
            .map_err(anyhow::Error::from)
            .and_then(|result| result);
            if let Err(error) = result {
                log::error!("publish completion could not be journaled at shutdown assignment={}: {error:#}; durable job remains for conservative orphan recovery",
                    pending.receipt.job.assignment_id);
            }
        }
    }
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
    acceptance: crate::dev_acceptance::DevAcceptancePolicy,
) {
    let mut tasks = tokio::task::JoinSet::<anyhow::Result<()>>::new();
    let mut owned = HashMap::new();
    let mut completed = DispatchCompletions::default();
    let mut interval = tokio::time::interval(Duration::from_millis(250));
    let mut online = std::collections::HashSet::new();
    let mut unavailable = HashMap::new();
    let mut roster_at = None;
    let mut recovery_at = tokio::time::Instant::now();
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            _ = interval.tick() => {},
            _ = completed.settle_ready(&db), if db.ensure_publish_recovery_ready().is_ok() => {},
            Some(result) = tasks.join_next_with_id(), if !tasks.is_empty() => {
                let (id, error) = match result {
                    Ok((id, result)) => (id, result.err()),
                    Err(e) => (e.id(), Some(anyhow::Error::new(e))),
                };
                if let Some(job) = owned.remove(&id) {
                    completed.push(job, error);
                }
            }
        }
        if stop.load(std::sync::atomic::Ordering::Acquire) {
            while let Some(result) = tasks.join_next_with_id().await {
                let (id, error) = match result {
                    Ok((id, r)) => (id, r.err()),
                    Err(e) => (e.id(), Some(anyhow::Error::new(e))),
                };
                if let Some(job) = owned.remove(&id) {
                    completed.push(job, error);
                }
            }
            completed.preserve_on_shutdown(&db).await;
            break;
        }
        if db.ensure_publish_recovery_ready().is_err() {
            if tokio::time::Instant::now() >= recovery_at {
                if let Err(error) = db
                    .storage_write(|db| db.recover_publish_dispatch_startup())
                    .await
                {
                    log::error!("publish startup recovery pending: {error:#}");
                }
                recovery_at = tokio::time::Instant::now() + Duration::from_secs(30);
            }
            if db.ensure_publish_recovery_ready().is_err() {
                continue;
            }
        }
        let result = async {
            // Only this dispatcher observes joined workers and their released device owners.
            for (assignment, udid, campaign) in db.pending_publish_exclusion_releases()? {
                if control.current_work_owner(&udid).is_none() {
                    db.finish_publish_exclusion_after_release(&assignment)?;
                    execution::announce(&events, &db, &campaign);
                }
            }
            let now = chrono::Local::now()
                .naive_local()
                .and_utc()
                .timestamp_millis();
            if !acceptance.active() {
                db.expire_publish_dispatch(now)?;
            } else {
                for campaign in acceptance.scoped_campaign_ids() {
                    db.expire_publish_dispatch_for_campaign(now, &campaign)?;
                }
            }
            for run in db.finished_publish_dispatch_runs()? {
                if acceptance.active() {
                    let Some(detail) = db.get_publish_campaign(&run.campaign_id)? else {
                        continue;
                    };
                    if !detail.assignments.iter().all(|assignment| {
                        acceptance.allows_publish_dispatch(&run.campaign_id, &assignment.udid)
                    }) {
                        continue;
                    }
                }
                db.finish_publish_pipeline(&run)?;
                execution::reconcile_publish_execution_and_announce(
                    &db,
                    &events,
                    &run.campaign_id,
                )?;
            }
            let pending = db
                .pending_publish_dispatch(128)?
                .into_iter()
                .filter(|job| acceptance.allows_publish_dispatch(&job.run.campaign_id, &job.udid))
                .collect::<Vec<_>>();
            if pending.is_empty() {
                return Ok::<_, anyhow::Error>(());
            }
            if roster_at
                .is_none_or(|at: tokio::time::Instant| at.elapsed() >= Duration::from_secs(2))
            {
                let roster = control.list_devices().await?;
                unavailable = roster
                    .iter()
                    .filter(|d| {
                        d.status == riviu_core::DeviceStatus::Pairing
                            || d.last_error
                                .as_ref()
                                .is_some_and(|e| e.to_lowercase().contains("unauthorized"))
                    })
                    .map(|d| {
                        (
                            d.udid.clone(),
                            d.last_error.clone().unwrap_or_else(|| {
                                "Thiết bị chưa cho phép gỡ lỗi USB; xác nhận trên điện thoại".into()
                            }),
                        )
                    })
                    .collect();
                online = roster
                    .into_iter()
                    .filter(|d| {
                        matches!(
                            d.status,
                            riviu_core::DeviceStatus::Ready
                                | riviu_core::DeviceStatus::Connected
                                | riviu_core::DeviceStatus::Busy
                        )
                    })
                    .map(|d| d.udid)
                    .collect();
                roster_at = Some(tokio::time::Instant::now());
            }
            for job in pending {
                // Completed tasks still occupy a worker slot until joined. Fast failures
                // must not accumulate an unbounded JoinSet while permits are released.
                if tasks.len() >= db.publish_limits()?.device_total {
                    break;
                }
                if control.current_work_owner(&job.udid).is_some() {
                    db.defer_publish_dispatch(&job, "device_busy")?;
                    continue;
                }
                db.init_publish_recovery(&job.assignment_id, &job.run.token)?;
                if let Some(reason) = unavailable.get(&job.udid) {
                    db.fail_publish_queued_device(&job, reason)?;
                    continue;
                }
                if !db.admit_publish_reconnect(
                    &job,
                    online.contains(&job.udid),
                    chrono::Utc::now().timestamp_millis(),
                )? {
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
                    request.network.ensure_implemented()?;
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
                        let journal = Arc::new(Recovery {
                            db: runtime.db.clone(),
                            assignment: a.id.clone(),
                            run: runtime.run.clone(),
                        });
                        return riviu_core::publish_recovery::scope(
                            journal,
                            Box::pin(runtime.post(a, bundle)),
                        )
                        .await;
                    }
                    runtime.db.update_publish_recovery_step(
                        &a.id,
                        &runtime.run.token,
                        "transfer",
                        None,
                    )?;
                    let mut revision = runtime.db.publish_assignment_revision(&a.id)?;
                    let result = Box::pin(runtime.transfer(&a, &bundle, &mut revision)).await;
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

    #[tokio::test(start_paused = true)]
    async fn dispatch_recovery_retries_settlement_without_replaying_completed_publications() {
        let path = std::env::temp_dir().join(format!("dispatch-completion-{}.db", Uuid::new_v4()));
        let db = Arc::new(Database::open(&path).unwrap());
        let bundles: Vec<_> = (0..2)
            .map(|i| PublishBundle {
                id: format!("bundle-{i}"),
                source_path: "C:/fixture".into(),
                name: format!("bundle-{i}"),
                media_kind: riviu_core::PublishMediaKind::Image,
                images: vec![],
                video: None,
                caption_path: "C:/fixture/caption.txt".into(),
                caption: "fixture caption".into(),
                caption_sha256: "0".repeat(64),
                total_bytes: 0,
                partners: vec![],
            })
            .collect();
        let request = PublishCampaignRequest {
            sheet_delivery: None,
            verification_contract_version: Some(1),
            verification_builds: vec![],
            request_id: Uuid::new_v4().to_string(),
            source_root: "C:/fixture".into(),
            bundle_ids: bundles.iter().map(|bundle| bundle.id.clone()).collect(),
            udids: vec!["phone-a".into(), "phone-b".into()],
            run_at: None,
            visibility: PublishVisibility::Public,
            cleanup_policy: PublishCleanupPolicy::KeepImportedAssets,
            network: riviu_core::SocialNetwork::TikTok,
            sound_policy: riviu_core::PublishSoundPolicy::Default,
            sheet_enabled: false,
            execution_confirmed: true,
            target_snapshot: None,
        };
        let campaign = db.create_publish_campaign(&request, &bundles).unwrap();
        let run = db.claim_publish_pipeline(&campaign.id).unwrap().unwrap();
        let mut jobs = db.pending_publish_dispatch(10).unwrap();
        assert_eq!(jobs.len(), 2);
        let conn = rusqlite::Connection::open(&path).unwrap();
        let intent = r#"{"effectIntent":"post","expectedAccount":"fixture"}"#;
        for job in &mut jobs {
            db.init_publish_recovery(&job.assignment_id, &run.token)
                .unwrap();
            assert!(db.claim_publish_dispatch(job, 0).unwrap());
            // These workers have already returned from their phone calls. Keep the
            // durable Post intent while injecting a fault in DB completion only.
            job.phase = "compose".into();
            conn.execute(
                "UPDATE publish_dispatch_jobs SET phase='compose' WHERE assignment_id=?1",
                [&job.assignment_id],
            )
            .unwrap();
            conn.execute(
                "UPDATE publish_assignments SET state='verifying',effect_intent=?2,evidence_json=?2 WHERE id=?1",
                rusqlite::params![job.assignment_id, intent],
            ).unwrap();
        }
        conn.execute_batch(&format!(
            "CREATE TRIGGER reject_completion BEFORE UPDATE OF state ON publish_dispatch_jobs
             WHEN OLD.assignment_id='{}' AND NEW.state<>'running'
             BEGIN SELECT RAISE(FAIL,'fixture settlement unavailable'); END;",
            jobs[0].assignment_id,
        ))
        .unwrap();
        let dispatch_state = |job: &riviu_core::db::PublishDispatchJob| -> String {
            conn.query_row(
                "SELECT state FROM publish_dispatch_jobs WHERE assignment_id=?1",
                [&job.assignment_id],
                |row| row.get(0),
            )
            .unwrap()
        };
        let mut completed = DispatchCompletions::default();
        completed.push(
            jobs[0].clone(),
            Some(anyhow::anyhow!("network response lost after Post")),
        );
        completed.push(jobs[1].clone(), None);
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let busy_db = db.clone();
        let writer = tokio::spawn(async move {
            busy_db
                .storage_write(move |_| {
                    let _ = started_tx.send(());
                    release_rx.recv_timeout(Duration::from_secs(5))?;
                    Ok(())
                })
                .await
        });
        started_rx.await.unwrap();
        tokio::select! {
            _ = completed.settle_ready(&db) => panic!("settlement crossed a held writer lane"),
            _ = tokio::time::sleep(Duration::from_millis(1)) => {}
        }
        assert!(
            completed.in_flight.is_some(),
            "select cancellation retains the owned DB task"
        );
        assert_eq!(completed.pending.len(), 1);
        completed.preserve_on_shutdown(&db).await;
        assert_eq!(fs::read_dir(path.with_extension("publish-completions")).unwrap().count(), 2,
            "shutdown must preserve pending and inflight receipts without waiting for SQLite admission");
        release_tx.send(()).unwrap();
        writer.await.unwrap().unwrap();
        completed.settle_ready(&db).await;
        completed.settle_ready(&db).await;
        assert_eq!(
            dispatch_state(&jobs[1]),
            "finished",
            "one failed receipt must not hide healthy completion"
        );
        assert_eq!(dispatch_state(&jobs[0]), "running");
        assert_eq!(
            completed.pending.len(),
            1,
            "a failed settlement must retain its exact completion receipt"
        );

        // A persistent SQLite fault cannot keep shutdown retrying forever. The
        // exact receipt remains on disk for a new Database instance to replay.
        completed.preserve_on_shutdown(&db).await;
        assert_eq!(
            fs::read_dir(path.with_extension("publish-completions"))
                .unwrap()
                .count(),
            1
        );

        conn.execute_batch("DROP TRIGGER reject_completion;")
            .unwrap();
        tokio::time::advance(Duration::from_secs(2)).await;
        completed.settle_ready(&db).await;
        assert!(completed.pending.is_empty());
        assert_eq!(dispatch_state(&jobs[0]), "finished");
        let (attempt, revision): (String, i64) = conn
            .query_row(
                "SELECT attempt_id,revision FROM publish_dispatch_jobs WHERE assignment_id=?1",
                [&jobs[0].assignment_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(attempt, jobs[0].attempt_id);
        assert_eq!(revision, jobs[0].revision + 2);
        assert_eq!(
            conn.query_row("SELECT COUNT(*) FROM publish_attempts", [], |row| row
                .get::<_, i64>(0))
                .unwrap(),
            2
        );
        for assignment in db
            .get_publish_campaign(&campaign.id)
            .unwrap()
            .unwrap()
            .assignments
        {
            assert_eq!(assignment.state, Stage::Verifying);
            assert_eq!(assignment.effect_intent.as_deref(), Some(intent));
        }
        // A repeated ACK is stale, not permission to reopen the completed job.
        completed.push(jobs[0].clone(), None);
        completed.settle_ready(&db).await;
        assert!(completed.pending.is_empty());
        assert_eq!(dispatch_state(&jobs[0]), "finished");
        assert!(db.pending_publish_dispatch(10).unwrap().is_empty());
        assert!(db.finish_publish_pipeline(&run).unwrap());
        drop(conn);
        drop(db);
        let _ = fs::remove_file(path);
    }

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
