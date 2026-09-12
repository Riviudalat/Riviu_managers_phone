//! One transfer-to-composer journey per device, with app-wide admission and durable ownership.
use super::execution::{self, PhoneFailure};
use super::*;
use riviu_core::db::PublishPipelineRun;
use riviu_core::{PublishAssignmentRecord, PublishBundle, PublishCampaignState as Stage};
use std::sync::{Mutex, OnceLock};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

#[derive(Debug)]
pub(super) struct PipelineClaimRejected;
impl std::fmt::Display for PipelineClaimRejected {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("chiến dịch đã có worker hoặc không còn ở trạng thái bắt đầu")
    }
}
impl std::error::Error for PipelineClaimRejected {}

const DEFAULT_TRANSFER_DEVICES: usize = 4;
fn transfer_slots() -> Arc<Semaphore> {
    static SLOTS: OnceLock<Arc<Semaphore>> = OnceLock::new();
    SLOTS
        .get_or_init(|| {
            let limit = if cfg!(debug_assertions) {
                std::env::var("RIVIU_BENCH_PIPELINE_TRANSFERS")
                    .ok()
                    .and_then(|v| v.parse::<usize>().ok())
                    .filter(|v| (1..=10).contains(v))
                    .unwrap_or(DEFAULT_TRANSFER_DEVICES)
            } else {
                DEFAULT_TRANSFER_DEVICES
            };
            Arc::new(Semaphore::new(limit))
        })
        .clone()
}
fn post_slots() -> Arc<Semaphore> {
    static SLOTS: OnceLock<Arc<Semaphore>> = OnceLock::new();
    SLOTS.get_or_init(|| Arc::new(Semaphore::new(10))).clone()
}
fn device_slot(udid: &str) -> Arc<Semaphore> {
    static SLOTS: OnceLock<Mutex<HashMap<String, std::sync::Weak<Semaphore>>>> = OnceLock::new();
    let mut slots = SLOTS
        .get_or_init(Mutex::default)
        .lock()
        .expect("device slots");
    slots.retain(|_, v| v.strong_count() > 0);
    if let Some(slot) = slots.get(udid).and_then(std::sync::Weak::upgrade) {
        return slot;
    }
    let slot = Arc::new(Semaphore::new(1));
    slots.insert(udid.into(), Arc::downgrade(&slot));
    slot
}
async fn admitted(
    db: &Database,
    run: &PublishPipelineRun,
    slots: Arc<Semaphore>,
) -> anyhow::Result<OwnedSemaphorePermit> {
    let acquire = slots.acquire_owned();
    tokio::pin!(acquire);
    loop {
        anyhow::ensure!(
            db.publish_pipeline_current(run)?,
            "publish pipeline cancelled or replaced"
        );
        tokio::select! {
            permit=&mut acquire=>return permit.context("publish queue closed"),
            _=tokio::time::sleep(Duration::from_millis(100))=>{}
        }
    }
}

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
        let _slot = admitted(&self.db, &self.run, transfer_slots()).await?;
        // All prerequisites belong to this device; its failure must not end another device.
        let mut single = self
            .db
            .get_publish_campaign(&self.run.campaign_id)?
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
    async fn one(self, a: PublishAssignmentRecord, bundle: PublishBundle) -> anyhow::Result<()> {
        let _device = admitted(&self.db, &self.run, device_slot(&a.udid)).await?;
        let ready = async {
            let mut revision = self.db.publish_assignment_revision(&a.id)?;
            if let Err(error) = self.transfer(&a, &bundle, &mut revision).await {
                let current = self
                    .db
                    .get_publish_campaign(&self.run.campaign_id)?
                    .context("campaign missing")?;
                if let Some(row) = current.assignments.iter().find(|r| r.id == a.id) {
                    if row.effect_intent.is_none() {
                        let _ = self.advance(
                            row,
                            &mut revision,
                            row.state.clone(),
                            Stage::FailedBeforeDispatch,
                            Some("media_transfer_failed"),
                            Some(&serde_json::json!({"message":format!("{error:#}")}).to_string()),
                        );
                    }
                }
                self.progress(
                    &a,
                    progress::PublishProgress::FailedBeforePost {
                        reason: format!("{error:#}"),
                    },
                );
                return Err(error);
            }
            Ok((self, a, bundle))
        };
        continue_after_transfer(ready, |(runtime, a, bundle)| runtime.post(a, bundle)).await
    }
    async fn post(self, a: PublishAssignmentRecord, bundle: PublishBundle) -> anyhow::Result<()> {
        self.progress(&a, progress::PublishProgress::WaitingControl);
        let _post = admitted(&self.db, &self.run, post_slots()).await?;
        anyhow::ensure!(
            self.db.publish_pipeline_current(&self.run)?,
            "pipeline stopped before composer"
        );
        let detail = self
            .db
            .get_publish_campaign(&self.run.campaign_id)?
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
    // A dropped IPC caller must not abort workers holding device requests or public effects.
    tokio::spawn(async move {
        let result = async {
            let detail = db
                .get_publish_campaign(&campaign)?
                .context("campaign missing")?;
            let runtime = Runtime {
                control,
                db: db.clone(),
                frames,
                events: events.clone(),
                agent,
                run: run.clone(),
                sound,
            };
            let mut errors = Vec::new();
            let mut jobs = tokio::task::JoinSet::new();
            let barrier_benchmark = cfg!(debug_assertions)
                && std::env::var("RIVIU_BENCH_PIPELINE_MODE").as_deref() == Ok("barrier")
                && db.get_setting("publish.rehearsal")?.as_deref() == Some("stopBeforePost");
            let mut staged = Vec::new();
            for a in detail.assignments {
                if a.effect_intent.is_some()
                    || !matches!(
                        a.state,
                        Stage::Queued
                            | Stage::Scheduled
                            | Stage::Ready
                            | Stage::Imported
                            | Stage::FailedBeforeDispatch
                    )
                {
                    continue;
                }
                let Some(b) = detail.bundles.iter().find(|b| b.id == a.bundle_id).cloned() else {
                    errors.push(format!("bundle missing for {}", a.id));
                    continue;
                };
                if barrier_benchmark {
                    let mut revision = db.publish_assignment_revision(&a.id)?;
                    match runtime.transfer(&a, &b, &mut revision).await {
                        Ok(()) => staged.push((a, b)),
                        Err(error) => errors.push(format!("{error:#}")),
                    }
                } else {
                    jobs.spawn(runtime.clone().one(a, b));
                }
            }
            for (a, b) in staged {
                jobs.spawn(runtime.clone().post(a, b));
            }
            while let Some(result) = jobs.join_next().await {
                match result {
                    Ok(Ok(())) => {}
                    Ok(Err(error)) => errors.push(format!("{error:#}")),
                    Err(error) => errors.push(error.to_string()),
                }
            }
            anyhow::ensure!(errors.is_empty(), "{}", errors.join("; "));
            Ok::<_, anyhow::Error>(())
        }
        .await;
        db.finish_publish_pipeline(&run)?;
        execution::announce(&events, &db, &campaign);
        result
    })
    .await?
}

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
        assert!(Arc::ptr_eq(&transfer_slots(), &transfer_slots()));
        assert!(Arc::ptr_eq(&post_slots(), &post_slots()));
        let same = device_slot("same");
        assert!(Arc::ptr_eq(&same, &device_slot("same")));
        let held = same.clone().acquire_owned().await.unwrap();
        assert!(device_slot("same").try_acquire_owned().is_err());
        assert!(device_slot("other").try_acquire_owned().is_ok());
        drop(held);
    }
}
