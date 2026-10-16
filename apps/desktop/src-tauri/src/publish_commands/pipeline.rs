//! One transfer-to-composer journey per device, with app-wide admission and durable ownership.
use super::execution::{self, PhoneFailure};
use super::prior_post;
use super::*;
use riviu_core::db::{PublishPipelineRun, PUBLISH_PHONE_WORK_CAP};
use riviu_core::read_latency::{
    fleet_tree_read_q90, ADB_INCIDENT_HOLD_MAX, ADB_INCIDENT_SAME_WITHIN,
    COMPOSE_CONCURRENCY_FLOOR, COMPOSE_CONCURRENCY_START, COMPOSE_START_SPACING, FLEET_LOADED_Q90,
};
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
    udid: String,
    ramp: Arc<parking_lot::Mutex<ComposeRamp>>,
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
        self.db.update_publish_recovery_step(
            &self.assignment,
            &self.run.token,
            step,
            checkpoint,
        )?;
        if step == "prePost" {
            self.ramp.lock().reached_pre_post(&self.udid);
        }
        Ok(())
    }
    fn retry(
        &self,
        failure: &riviu_core::publish_recovery::RecoveryFailure,
    ) -> anyhow::Result<Option<Duration>> {
        if failure.code == OBSERVATION_DEADLINE {
            self.ramp.lock().observation_deadline(&self.udid);
        }
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
        // The transfer lease is dropped before the same phone enters its composer.
        let result=async {
            anyhow::ensure!(self.db.publish_pipeline_current(&self.run)? && !self.db.publish_assignment_excluded(&a.id)?,"pipeline stopped before device transfer");
            // Effect admission (PLAN §3, review R2-F2): with the lease held, after every wait and
            // before load sampling, transfer or import, the approval must still match every
            // remaining phone. A refusal is terminal and settles the rest review-required.
            if let Err(refusal) = self.db.admit_publish_effect(&a.id)? {
                return Err(refusal.into());
            }
            // Load admission before any byte reaches the phone; a refusal settles below as
            // `device_overloaded` before dispatch.
            self.control.ensure_load_admissible(&context,None).await?;
            // An earlier post of another campaign that TikTok already holds is released by this
            // job, under this lease, before any byte reaches the phone (`release_prior_posts`).
            prior_post::release_prior_posts(&self.control,&self.db,Some(&self.events),&context,&a.id,&self.run).await?;
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
    if let Some(reason) = db.publish_approval_refusal(&campaign)? {
        anyhow::bail!(reason);
    }
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

/// A roster scan may admit or refuse a queued phone only this long after the scan began.
/// The dispatcher reads the background roster scan (`state.rs`, every ~3 s) rather than
/// running its own: on 14/10/2026 (r110, 28 Samsung phones, one adb 37.0.1 server) its own
/// scan reached it 145-180 s after spawn while the background scan of the same fleet
/// finished every ~3.8 s, so every snapshot was stale on arrival and all 28 jobs sat
/// `roster_pending` for 20 min. 300 s is a margin, not a measured scan time. A stale-looking
/// phone is still gated by its worker's lease and exact-serial reads, so the snapshot only
/// decides whether to try.
const ROSTER_MAX_AGE: Duration = Duration::from_secs(300);

/// What the latest usable roster says about one phone. `Unknown` is not `Offline`: it
/// starts no reconnect window and fails nothing.
#[derive(Debug, PartialEq, Eq)]
enum RosterPresence {
    Unknown,
    Unavailable(String),
    Online,
    Offline,
}

struct RosterSnapshot {
    scanned_at: tokio::time::Instant,
    online: std::collections::HashSet<String>,
    unavailable: HashMap<String, String>,
}

impl RosterSnapshot {
    fn new(scanned_at: tokio::time::Instant, roster: Vec<riviu_core::DeviceInfo>) -> Self {
        let unavailable = roster
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
        let online = roster
            .into_iter()
            .filter(|d| {
                matches!(
                    d.status,
                    riviu_core::DeviceStatus::Ready
                        | riviu_core::DeviceStatus::Connected
                        | riviu_core::DeviceStatus::Busy
                )
                    // A latched helper-setup failure is still on adb, not a lost
                    // connection: this phone's own transfer prepares its helper
                    // again under its lease and fails only itself if it cannot.
                    || (d.status == riviu_core::DeviceStatus::Error
                        && d.last_error.as_deref().is_some_and(|error| {
                            error.starts_with(riviu_android_driver::driver::HELPER_SETUP_FAILURE)
                        }))
            })
            .map(|d| d.udid)
            .collect();
        Self {
            scanned_at,
            online,
            unavailable,
        }
    }
}

/// The dispatcher's view of the background roster scan. It never scans: a scan waiting on
/// the inventory lock, the adb queue or a cold probe cannot hold completion drain or the
/// dispatch of phones the last scan already proved, and the fleet is not scanned twice. The
/// snapshot only admits a job to a claim; the worker's lease and its own reads of that exact
/// serial still decide whether any effect happens.
#[derive(Default)]
struct RosterRefresh {
    snapshot: Option<RosterSnapshot>,
    reported_stale: bool,
}

impl RosterRefresh {
    /// Take the newest background scan, if it is newer than the one already held.
    fn refresh(&mut self, registry: &riviu_core::DeviceRegistry) {
        if let Some(scan) = registry.latest_scan() {
            if self
                .snapshot
                .as_ref()
                .is_none_or(|held| scan.started_at > held.scanned_at)
            {
                self.snapshot = Some(RosterSnapshot::new(scan.started_at, scan.devices));
            }
        }
        let age = self.snapshot.as_ref().map(|held| held.scanned_at.elapsed());
        if age.is_some_and(|age| age <= ROSTER_MAX_AGE) {
            self.reported_stale = false;
        } else if !self.reported_stale {
            self.reported_stale = true;
            log::warn!(
                "Đăng bài: danh sách máy {}; tạm hoãn giao việc cho máy chưa rõ trạng thái, vẫn thu kết quả máy đang chạy",
                match age {
                    Some(age) => format!("đã {} giây chưa được quét lại", age.as_secs()),
                    None => "chưa được quét lần nào".to_string(),
                }
            );
        }
    }

    fn presence(&self, udid: &str) -> RosterPresence {
        let Some(snapshot) = self
            .snapshot
            .as_ref()
            .filter(|snapshot| snapshot.scanned_at.elapsed() <= ROSTER_MAX_AGE)
        else {
            return RosterPresence::Unknown;
        };
        if let Some(reason) = snapshot.unavailable.get(udid) {
            RosterPresence::Unavailable(reason.clone())
        } else if snapshot.online.contains(udid) {
            RosterPresence::Online
        } else {
            RosterPresence::Offline
        }
    }
}

/// The typed code of a read that outlived its observation window (`publish_recovery::describe`).
const OBSERVATION_DEADLINE: &str = "publish_observation_deadline";

/// Longest a queued row waits for a box that dropped all its phones at once before the usual
/// 2-minute reconnect window applies again. Not measured as an optimum: the r125 box stayed dark
/// 18 min (15/10/2026) until someone replugged it; an hour covers an operator who notices late,
/// and past it the row fails as before and waits for **Thử lại máy này**.
const BOX_OUTAGE_HOLD_MAX: Duration = Duration::from_secs(60 * 60);

/// Whether a queued phone's box was already dark when this process started (drop audit
/// schedule_restart#1). `mass_disconnect_since` only knows phones that left the roster; after a
/// restart or a first launch with the box unpowered its phones were never listed, so each queued
/// row ended `device_reconnect_timeout` two minutes in. Such a row is held like a box outage, for
/// at most [`BOX_OUTAGE_HOLD_MAX`] from start, while no phone of its campaign has been listed
/// since start, and, for a campaign smaller than a box (`MASS_DISCONNECT_MIN_PHONES`), while no
/// phone at all has been. A phone gone alone while its box is up keeps the 2-minute window.
fn box_dark_since_start(registry: &riviu_core::DeviceRegistry, phones: &[String]) -> bool {
    registry.started_at().elapsed() < BOX_OUTAGE_HOLD_MAX
        && (phones.len() >= riviu_core::registry::MASS_DISCONNECT_MIN_PHONES
            || !registry.listed_any_since_start())
        && !phones.iter().any(|udid| registry.listed_since_start(udid))
}

/// The phones of a job's campaign, read once per campaign, with the job's own among them.
fn campaign_phones<'a>(
    db: &Database,
    cache: &'a mut HashMap<String, Vec<String>>,
    job: &riviu_core::db::PublishDispatchJob,
) -> &'a [String] {
    cache.entry(job.run.campaign_id.clone()).or_insert_with(|| {
        let mut phones = db
            .publish_campaign_request(&job.run.campaign_id)
            .ok()
            .flatten()
            .map(|request| request.udids)
            .unwrap_or_default();
        if !phones.contains(&job.udid) {
            phones.push(job.udid.clone());
        }
        phones
    })
}

/// How often startup recovery is retried at first (drop audit schedule_restart#5). The schedule
/// clock admits nothing while it is pending, and the old 30 s retry was a schedule's whole
/// admission window. Not measured: the StorageBusy bursts logged right after bootstrap lasted up
/// to 22 s (15/10/2026 23:15:02-24), so every 2 s for 30 s outlasts the longest; after that the
/// 30 s cadence resumes, since whatever still fails is not a burst.
const STARTUP_RECOVERY_FAST_RETRY: Duration = Duration::from_secs(2);
const STARTUP_RECOVERY_FAST_ATTEMPTS: u32 = 15;

/// How many composes may run at once, and when the next may start. Only a new compose claim
/// is paced: no running permit is revoked, and transfer, verification and cleanup keep sharing
/// `PUBLISH_PHONE_WORK_CAP` as before. Constants and their reasons: `riviu_core::read_latency`.
struct ComposeRamp {
    limit: usize,
    last_start: Option<tokio::time::Instant>,
    /// When each phone's current compose started, so a deadline can be dated against `last_cut`.
    started_at: HashMap<String, tokio::time::Instant>,
    last_cut: Option<tokio::time::Instant>,
}

impl ComposeRamp {
    fn new() -> Self {
        Self {
            limit: COMPOSE_CONCURRENCY_START,
            last_start: None,
            started_at: HashMap::new(),
            last_cut: None,
        }
    }

    fn may_start(&self, composing: usize, now: tokio::time::Instant) -> bool {
        composing < self.limit
            && self.last_start.is_none_or(|started| {
                now.saturating_duration_since(started) >= COMPOSE_START_SPACING
            })
    }

    fn started(&mut self, udid: &str, now: tokio::time::Instant) {
        self.last_start = Some(now);
        self.started_at.insert(udid.to_string(), now);
    }

    fn set(&mut self, limit: usize, why: &str) {
        if limit != self.limit {
            log::info!(
                "Đăng bài: số máy soạn bài cùng lúc {} → {limit} ({why})",
                self.limit
            );
            self.limit = limit;
        }
    }

    /// One step up per attempt that reaches the pre-Post check, and only on a fleet measured
    /// fast; an unknown fleet q90 never raises the limit.
    fn reached_pre_post(&mut self, udid: &str) {
        let cap = PUBLISH_PHONE_WORK_CAP as usize;
        if let Some(q90) = fleet_tree_read_q90() {
            if q90 < FLEET_LOADED_Q90 && self.limit < cap {
                self.set(
                    self.limit + 1,
                    &format!(
                        "{udid} tới bước kiểm tra trước Đăng; q90 đọc màn hình của cả dàn {} ms < {} ms",
                        q90.as_millis(),
                        FLEET_LOADED_Q90.as_millis()
                    ),
                );
            }
        }
    }

    /// One cut per load episode. A compose admitted before the last cut ran under the higher
    /// limit that cut already answered, so its deadline is the same signal: r130 halved 8 → 4 → 2
    /// within one second on two such phones, and one phone's step retry and attempt end counted
    /// twice.
    fn observation_deadline(&mut self, udid: &str) {
        if let (Some(cut), Some(started)) = (self.last_cut, self.started_at.get(udid)) {
            if *started < cut {
                return;
            }
        }
        self.last_cut = Some(tokio::time::Instant::now());
        self.set(
            (self.limit / 2).max(COMPOSE_CONCURRENCY_FLOOR),
            &format!("{udid} hết giờ chờ đọc màn hình ({OBSERVATION_DEADLINE})"),
        );
    }

    fn reset_when_idle(&mut self) {
        self.started_at.clear();
        self.last_cut = None;
        self.set(
            COMPOSE_CONCURRENCY_START,
            "không còn bài nào đang chạy hay chờ; vòng sau bắt đầu lại từ mức đầu",
        );
    }
}

/// While the adb server is out or suspected hung, the dispatcher claims no new publish job, so
/// no transfer or compose starts and no new Post intent follows from one. Running jobs are left
/// alone; the intent and verification rules settle them. One incident holds at most
/// `ADB_INCIDENT_HOLD_MAX`, and a signal back within `ADB_INCIDENT_SAME_WITHIN` continues it.
#[derive(Default)]
struct AdbIncidentHold {
    /// When the current incident began; its hold budget runs from here.
    began: Option<tokio::time::Instant>,
    last_seen: Option<tokio::time::Instant>,
    /// The current hold's start, also in UTC ms for the reconnect windows it pauses.
    holding: Option<(tokio::time::Instant, i64)>,
}

impl AdbIncidentHold {
    /// Whether new claims are held this tick. Without queued or running publish work there is
    /// nothing to hold and nothing to say about Publish.
    fn observe(&mut self, signal: bool, in_flight: bool, db: &Database) -> bool {
        let now = tokio::time::Instant::now();
        if signal {
            if self
                .last_seen
                .is_none_or(|seen| now.saturating_duration_since(seen) > ADB_INCIDENT_SAME_WITHIN)
            {
                self.began = Some(now);
            }
            self.last_seen = Some(now);
        }
        let within_budget = self
            .began
            .is_some_and(|began| now.saturating_duration_since(began) < ADB_INCIDENT_HOLD_MAX);
        let hold = signal && in_flight && within_budget;
        match (self.holding, hold) {
            (None, true) => {
                self.holding = Some((now, chrono::Utc::now().timestamp_millis()));
                note_activity(
                    db,
                    "publish.dispatch.held",
                    &format!(
                        "Đăng bài: ADB server đang mất hoặc nghi bị treo; tạm dừng giao việc đăng mới \
                         (không bắt đầu chuyển media, soạn bài hay lệnh Đăng mới). Máy đang chạy dở vẫn \
                         tiếp tục; tạm dừng tối đa {} phút",
                        ADB_INCIDENT_HOLD_MAX.as_secs() / 60
                    ),
                );
            }
            (Some((since, since_ms)), false) => {
                self.holding = None;
                if let Err(error) = db.pause_publish_reconnect_windows(
                    since_ms,
                    chrono::Utc::now().timestamp_millis(),
                ) {
                    log::error!("publish dispatch: reconnect windows not paused: {error:#}");
                }
                let why = if !signal {
                    "ADB đã trả lời lại".to_string()
                } else if !within_budget {
                    format!(
                        "đã tạm dừng đủ {} phút mà ADB vẫn chưa ổn; giao việc lại như bình thường",
                        ADB_INCIDENT_HOLD_MAX.as_secs() / 60
                    )
                } else {
                    "không còn bài đăng nào đang chạy hay chờ".to_string()
                };
                note_activity(
                    db,
                    "publish.dispatch.resumed",
                    &format!(
                        "Đăng bài: tiếp tục giao việc đăng sau {} giây tạm dừng ({why})",
                        now.saturating_duration_since(since).as_secs()
                    ),
                );
            }
            _ => {}
        }
        hold
    }
}

fn note_activity(db: &Database, action: &str, message: &str) {
    if let Err(error) = db.log_op(action, message) {
        log::error!("could not persist {action}: {error:#}");
    }
    log::warn!("{message}");
}

/// One application dispatcher serves immediate and scheduled campaigns. Queue rows
/// hold IDs only; media and driver sessions are loaded after per-device admission.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_dispatcher(
    control: Arc<DeviceControlPlane>,
    registry: riviu_core::DeviceRegistry,
    db: Arc<Database>,
    frames: Arc<dyn FrameSource>,
    events: riviu_core::events::EventBus,
    agent: String,
    admission: Arc<crate::state::CommandAdmissionState>,
    stop: Arc<std::sync::atomic::AtomicBool>,
    acceptance: crate::dev_acceptance::DevAcceptancePolicy,
    android: Option<Arc<riviu_android_driver::AndroidDriver>>,
) {
    let mut tasks = tokio::task::JoinSet::<anyhow::Result<()>>::new();
    let mut owned: HashMap<tokio::task::Id, riviu_core::db::PublishDispatchJob> = HashMap::new();
    let mut idle_deferrals = HashMap::new();
    // What each row waiting for an earlier post on its phone waits for, logged once per change.
    let mut prior_post_waits: HashMap<String, String> = HashMap::new();
    // For `box_dark_since_start`: phones per campaign, and campaigns already logged as held.
    let mut phones_by_campaign: HashMap<String, Vec<String>> = HashMap::new();
    let mut dark_since_start = std::collections::HashSet::new();
    prior_post::install_acceptance(&acceptance);
    let mut completed = DispatchCompletions::default();
    let mut interval = tokio::time::interval(Duration::from_millis(250));
    let mut roster = RosterRefresh::default();
    let mut recovery_at = tokio::time::Instant::now();
    let mut recovery_attempts = 0u32;
    let ramp = Arc::new(parking_lot::Mutex::new(ComposeRamp::new()));
    let mut incident = AdbIncidentHold::default();
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
                    if error.as_ref().is_some_and(|error| {
                        riviu_core::publish_recovery::describe(error).code == OBSERVATION_DEADLINE
                    }) {
                        ramp.lock().observation_deadline(&job.udid);
                    }
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
                recovery_attempts += 1;
                let retry = if recovery_attempts < STARTUP_RECOVERY_FAST_ATTEMPTS {
                    STARTUP_RECOVERY_FAST_RETRY
                } else {
                    Duration::from_secs(30)
                };
                if let Err(error) = db
                    .storage_write(|db| db.recover_publish_dispatch_startup())
                    .await
                {
                    log::error!(
                        "Đăng bài: chưa đối soát xong kết quả đăng lúc khởi động (lần {recovery_attempts}, thử lại sau {} giây; lịch đến giờ chờ bước này): {error:#}",
                        retry.as_secs()
                    );
                }
                recovery_at = tokio::time::Instant::now() + retry;
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
            // A prior-post release whose job ended without its abort: abort it now, so the old
            // posts carry on verifying as before.
            for job in db.stale_prior_post_releases()? {
                for campaign in db.abort_prior_post_release(
                    &job,
                    "Lượt hẹn giờ đã kết thúc khi đang nhả bài cũ",
                )? {
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
            // Admission is capacity-filtered; foreground priority must also cover
            // queued journeys waiting for a phase slot or recovery backoff.
            // Owned workers retain priority even after cancellation until release.
            let needed: std::collections::HashSet<_> = db
                .foreground_publish_dispatch_devices(now)?
                .into_iter()
                .filter(|(campaign, udid)| acceptance.allows_publish_dispatch(campaign, udid))
                .map(|(_, udid)| udid)
                .chain(
                    owned
                        .values()
                        .map(|job: &riviu_core::db::PublishDispatchJob| job.udid.clone()),
                )
                .collect();
            idle_deferrals.retain(|udid, _| needed.contains(udid));
            let in_flight = !needed.is_empty();
            for udid in needed {
                idle_deferrals
                    .entry(udid.clone())
                    .or_insert_with(|| control.defer_idle_work(&[udid], DeviceWorkOwner::Script));
            }
            if !in_flight {
                ramp.lock().reset_when_idle();
            }
            let adb_incident = android.as_ref().is_some_and(|android| {
                android.adb_server_outage() || android.adb_server_hang_suspected()
            });
            if incident.observe(adb_incident, in_flight, &db) || pending.is_empty() {
                return Ok::<_, anyhow::Error>(());
            }
            roster.refresh(&registry);
            let fleet_q90 = fleet_tree_read_q90();
            let mut composing = owned.values().filter(|job| job.phase == "compose").count();
            for job in pending {
                // Durable claims and the device lease serialize one phone. A slow
                // sibling must not consume a global publish worker slot.
                if control.current_work_owner(&job.udid).is_some() {
                    db.defer_publish_dispatch(&job, "device_busy")?;
                    continue;
                }
                db.init_publish_recovery(&job.assignment_id, &job.run.token)?;
                let online = match roster.presence(&job.udid) {
                    RosterPresence::Unknown => {
                        db.defer_publish_dispatch(&job, "roster_pending")?;
                        continue;
                    }
                    RosterPresence::Unavailable(reason) => {
                        db.fail_publish_queued_device(&job, &reason)?;
                        continue;
                    }
                    RosterPresence::Online => true,
                    // Lost with its whole box: wait for the box, not the 2-minute window
                    // meant for one unplugged phone, for at most BOX_OUTAGE_HOLD_MAX.
                    RosterPresence::Offline
                        if registry
                            .mass_disconnect_since(&job.udid)
                            .is_some_and(|since| since.elapsed() < BOX_OUTAGE_HOLD_MAX) =>
                    {
                        db.hold_publish_dispatch_for_box_outage(&job)?;
                        continue;
                    }
                    RosterPresence::Offline => {
                        // Or dark already when this process started: no phone left, so no
                        // mass disconnect was recorded (`box_dark_since_start`).
                        if registry.started_at().elapsed() < BOX_OUTAGE_HOLD_MAX
                            && !registry.listed_since_start(&job.udid)
                            && box_dark_since_start(
                                &registry,
                                campaign_phones(&db, &mut phones_by_campaign, &job),
                            )
                        {
                            if dark_since_start.insert(job.run.campaign_id.clone()) {
                                log::warn!(
                                    "Đăng bài: chưa thấy máy nào của lượt {} từ khi mở ứng dụng (hộp/hub mất nguồn USB?); giữ hàng chờ tối đa {} phút kể từ lúc mở thay vì 2 phút mất kết nối, cắm lại là máy tự chạy tiếp, chưa đăng gì",
                                    job.run.campaign_id.get(..8).unwrap_or(&job.run.campaign_id),
                                    BOX_OUTAGE_HOLD_MAX.as_secs() / 60
                                );
                            }
                            db.hold_publish_dispatch_for_box_outage(&job)?;
                            continue;
                        }
                        false
                    }
                };
                if !db.admit_publish_reconnect(
                    &job,
                    online,
                    chrono::Utc::now().timestamp_millis(),
                )? {
                    continue;
                }
                // A whole-attempt retry whose clock has passed waits for a fleet measured fast.
                // Unknown counts as fast only while nothing composes: then it is the least load.
                let fleet_ready = fleet_q90.map_or(composing == 0, |q90| q90 < FLEET_LOADED_Q90);
                if db.publish_retry_waits_for_fleet(
                    &job.assignment_id,
                    chrono::Utc::now().timestamp_millis(),
                    fleet_ready,
                )? {
                    db.hold_publish_dispatch(&job, "fleet_load")?;
                    continue;
                }
                let compose = job.phase == "compose";
                if compose
                    && !ramp
                        .lock()
                        .may_start(composing, tokio::time::Instant::now())
                {
                    db.hold_publish_dispatch(&job, "compose_pacing")?;
                    continue;
                }
                // An earlier post of another campaign still owing its link on this phone: proceed
                // (the job releases a post TikTok holds), wait sending nothing, or stop before Post.
                let round = job.run.campaign_id.get(..8).unwrap_or(&job.run.campaign_id);
                match prior_post::with_scope(|scope| {
                    db.publish_prior_post_gate(&job, chrono::Utc::now().timestamp_millis(), scope)
                })? {
                    riviu_core::db::PriorPostGate::Proceed => {
                        if prior_post_waits.remove(&job.assignment_id).is_some() {
                            log::info!(
                                "Lịch đăng {round} trên {}: bài trước trên máy không còn giữ lượt này; tiếp tục",
                                job.udid
                            );
                        }
                    }
                    riviu_core::db::PriorPostGate::Held { key, text } => {
                        if prior_post_waits.insert(job.assignment_id.clone(), key.clone())
                            != Some(key)
                        {
                            log::warn!("Lịch đăng {round} trên {}: {text}", job.udid);
                            execution::announce(&events, &db, &job.run.campaign_id);
                        }
                        continue;
                    }
                    riviu_core::db::PriorPostGate::Ended { code, text } => {
                        prior_post_waits.remove(&job.assignment_id);
                        log::warn!(
                            "Lịch đăng {round} trên {}: dừng trước Đăng ({code}); không đóng TikTok, không đăng lại bài cũ. {text}",
                            job.udid
                        );
                        execution::announce(&events, &db, &job.run.campaign_id);
                        continue;
                    }
                }
                let Some(permit) = db.try_publish_work(&job.udid, &job.phase, &job.attempt_id)?
                else {
                    db.hold_publish_dispatch(&job, "stage_capacity")?;
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
                if compose {
                    let mut ramp = ramp.lock();
                    ramp.started(&job.udid, tokio::time::Instant::now());
                    composing += 1;
                    log::info!(
                        "Đăng bài: bắt đầu soạn bài trên {} ({composing}/{} máy đang soạn)",
                        job.udid,
                        ramp.limit
                    );
                }
                let (work_db, work_control, work_frames, work_events, work_agent, work_ramp) = (
                    db.clone(),
                    control.clone(),
                    frames.clone(),
                    events.clone(),
                    agent.clone(),
                    ramp.clone(),
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
                            udid: task_job.udid.clone(),
                            ramp: work_ramp,
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
                                // A load-admission refusal keeps its own code and sentence: the
                                // phone was overloaded, no media was sent.
                                let overloaded =
                                    riviu_core::device_control::load::mentions_device_overloaded(
                                        &format!("{error:#}"),
                                    );
                                // Waiting for an earlier post on this phone is not a transfer
                                // failure: nothing was sent, and the row is requeued unspent.
                                let waiting_prior_post = riviu_core::publish_recovery::describe(error).code
                                    == riviu_core::publish_recovery::PRIOR_POST_PENDING;
                                let (code, what) = if waiting_prior_post {
                                    (
                                        riviu_core::publish_recovery::PRIOR_POST_PENDING,
                                        "Chờ bài trước trên máy này, chưa chuyển ảnh/video",
                                    )
                                } else if overloaded {
                                    (
                                        riviu_core::device_control::load::DEVICE_OVERLOADED_CODE,
                                        "máy quá tải, chưa chuyển ảnh/video",
                                    )
                                } else {
                                    ("media_transfer_failed", "chuyển ảnh/video thất bại")
                                };
                                if runtime.advance(
                                    row,
                                    &mut revision,
                                    row.state.clone(),
                                    Stage::FailedBeforeDispatch,
                                    Some(code),
                                    Some(
                                        &serde_json::json!({"message":format!("{error:#}")})
                                            .to_string(),
                                    ),
                                ).is_ok() {
                                    runtime.progress(row, progress::PublishProgress::FailedBeforePost {
                                        reason: format!("{}: {what}: {error:#}", row.udid),
                                    });
                                }
                            }
                        }
                    }
                    // Nothing on the transfer path reaches a composer, so an unrecognised
                    // failure spends the bounded retry budget instead of ending the row.
                    result.map_err(|error| {
                        anyhow::Error::new(
                            riviu_core::publish_recovery::media_transfer_failure(&error),
                        )
                    })
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
            approval: None,
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
        // Paused Tokio time does not auto-advance while the blocking writer is
        // active. Poll once and drop the borrowed future to model a losing
        // select branch without waiting for that writer's wall-clock timeout.
        let mut settling = Box::pin(completed.settle_ready(&db));
        assert!(futures_util::poll!(settling.as_mut()).is_pending());
        drop(settling);
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

    fn roster_row(udid: &str, status: riviu_core::DeviceStatus) -> riviu_core::DeviceInfo {
        riviu_core::DeviceInfo {
            udid: udid.to_string(),
            name: udid.to_string(),
            model: "fixture".to_string(),
            platform: riviu_core::DevicePlatform::Android,
            os_version: "fixture".to_string(),
            connection: riviu_core::ConnectionKind::Usb,
            status,
            battery: None,
            wda_ready: false,
            wda_expires_at: None,
            stream_url: None,
            tile_stream_state: riviu_core::TileStreamState::default(),
            last_error: None,
            wifi_ip: None,
        }
    }

    #[tokio::test(start_paused = true)]
    async fn roster_snapshot_is_unknown_when_missing_or_stale_never_offline() {
        let mut roster = RosterRefresh::default();
        assert_eq!(roster.presence("phone-a"), RosterPresence::Unknown);
        let mut pairing = roster_row("phone-p", riviu_core::DeviceStatus::Pairing);
        pairing.last_error = Some("unauthorized".into());
        roster.snapshot = Some(RosterSnapshot::new(
            tokio::time::Instant::now(),
            vec![
                roster_row("phone-a", riviu_core::DeviceStatus::Connected),
                pairing,
            ],
        ));
        assert_eq!(roster.presence("phone-a"), RosterPresence::Online);
        assert_eq!(roster.presence("phone-gone"), RosterPresence::Offline);
        assert_eq!(
            roster.presence("phone-p"),
            RosterPresence::Unavailable("unauthorized".into())
        );
        tokio::time::advance(ROSTER_MAX_AGE + Duration::from_millis(1)).await;
        // An old scan neither admits a phone nor starts a reconnect window or a refusal.
        for udid in ["phone-a", "phone-gone", "phone-p"] {
            assert_eq!(roster.presence(udid), RosterPresence::Unknown, "{udid}");
        }
    }

    /// Counts fleet scans, which the dispatcher must never run itself; every scan it could
    /// start would hang.
    struct GatedPhonesDriver {
        scans: std::sync::atomic::AtomicUsize,
        gates: HashMap<&'static str, Arc<Semaphore>>,
    }

    #[async_trait::async_trait]
    impl riviu_core::driver::DeviceDriver for GatedPhonesDriver {
        async fn list_devices(&self) -> anyhow::Result<Vec<riviu_core::DeviceInfo>> {
            self.scans.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            std::future::pending().await
        }
        fn reports_element_bounds(&self, _udid: &str) -> bool {
            true
        }
        /// The worker's first read of its own serial; it waits for the test, then refuses.
        async fn tiktok_build(&self, udid: &str) -> anyhow::Result<(String, String, String)> {
            if let Some(gate) = self.gates.get(udid) {
                let _open = gate.acquire().await?;
            }
            anyhow::bail!("fixture: {udid} không đọc được bản TikTok")
        }
        async fn refresh_device(&self, _udid: &str) -> anyhow::Result<riviu_core::DeviceInfo> {
            anyhow::bail!("fixture")
        }
        async fn install_app(&self, _udid: &str, _path: &Path) -> anyhow::Result<()> {
            anyhow::bail!("fixture")
        }
        async fn uninstall_app(&self, _udid: &str, _bundle_id: &str) -> anyhow::Result<()> {
            anyhow::bail!("fixture")
        }
        async fn screenshot(&self, _udid: &str, _dest: &Path) -> anyhow::Result<PathBuf> {
            anyhow::bail!("fixture")
        }
        async fn syslog_tail(&self, _udid: &str, _lines: usize) -> anyhow::Result<String> {
            anyhow::bail!("fixture")
        }
        async fn launch_app(&self, _udid: &str, _bundle_id: &str) -> anyhow::Result<()> {
            anyhow::bail!("fixture")
        }
        async fn terminate_app(
            &self,
            _udid: &str,
            _bundle_id: &str,
        ) -> anyhow::Result<riviu_core::driver::ProcessAbsenceProof> {
            anyhow::bail!("fixture")
        }
        async fn reboot(&self, _udid: &str) -> anyhow::Result<()> {
            anyhow::bail!("fixture")
        }
        async fn start_ui_session(
            &self,
            _udid: &str,
        ) -> anyhow::Result<Box<dyn riviu_core::UiSession>> {
            anyhow::bail!("fixture")
        }
        async fn ensure_stream(&self, _udid: &str) -> anyhow::Result<String> {
            anyhow::bail!("fixture")
        }
        async fn prepare_device(&self, _udid: &str) -> anyhow::Result<()> {
            anyhow::bail!("fixture")
        }
    }

    async fn wait_until(what: &str, done: impl Fn() -> bool) {
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while !done() {
            assert!(std::time::Instant::now() < deadline, "timed out: {what}");
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    #[tokio::test]
    async fn dispatcher_uses_background_roster_and_never_scans_itself() {
        let path = std::env::temp_dir().join(format!("dispatch-roster-{}.db", Uuid::new_v4()));
        let db = Arc::new(Database::open(&path).unwrap());
        let udids = ["phone-a", "phone-b", "phone-c"];
        let bundles: Vec<_> = (0..udids.len())
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
            udids: udids.map(String::from).into(),
            run_at: None,
            visibility: PublishVisibility::Public,
            cleanup_policy: PublishCleanupPolicy::KeepImportedAssets,
            network: riviu_core::SocialNetwork::TikTok,
            sound_policy: riviu_core::PublishSoundPolicy::Default,
            sheet_enabled: false,
            execution_confirmed: true,
            target_snapshot: None,
            // The fixture phones carry no account metadata, so each was approved without one.
            approval: Some(riviu_core::PublishApproval {
                version: 1,
                input_digest: "0".repeat(64),
                accounts: udids.map(|udid| (udid.to_owned(), String::new())).into(),
                prior_posts: None,
            }),
        };
        let campaign = db.create_publish_campaign(&request, &bundles).unwrap();
        db.claim_publish_pipeline(&campaign.id).unwrap().unwrap();
        let gates: HashMap<_, _> = ["phone-a", "phone-b"]
            .map(|udid| (udid, Arc::new(Semaphore::new(0))))
            .into();
        let driver = Arc::new(GatedPhonesDriver {
            scans: Default::default(),
            gates: gates.clone(),
        });
        let control = Arc::new(DeviceControlPlane::new(
            driver.clone(),
            Arc::new(riviu_core::DeviceWorkCoordinator::new()),
            Arc::new(riviu_core::StreamBudgetManager::new(2).unwrap()),
        ));
        // The background scan listed `phone-a` and `phone-b`; `phone-c` is absent, so its job
        // stays queued. No later scan arrives.
        let registry = riviu_core::DeviceRegistry::new(riviu_core::events::EventBus::new(64));
        registry.record_scan(riviu_core::FleetScan {
            started_at: tokio::time::Instant::now(),
            devices: ["phone-a", "phone-b"]
                .map(|udid| roster_row(udid, riviu_core::DeviceStatus::Connected))
                .into(),
        });
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let mut dispatcher = tokio::spawn(run_dispatcher(
            control,
            registry,
            db.clone(),
            Arc::new(riviu_core::NullFrameSource),
            riviu_core::events::EventBus::new(64),
            "fixture.agent".into(),
            Arc::new(crate::state::CommandAdmissionState::new(true)),
            stop.clone(),
            crate::dev_acceptance::DevAcceptancePolicy::from_parts(false, None, None),
            None,
        ));
        let conn = rusqlite::Connection::open(&path).unwrap();
        let job = |udid: &str| -> (String, String, String) {
            conn.query_row(
                "SELECT assignment_id,state,attempt_id FROM publish_dispatch_jobs WHERE udid=?1",
                [udid],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap()
        };
        wait_until("both listed phones claimed", || {
            job("phone-a").1 == "running" && job("phone-b").1 == "running"
        })
        .await;
        let (assignment_a, _, attempt_a) = job("phone-a");
        let (assignment_b, _, attempt_b) = job("phone-b");

        // Phone A's worker finishes while no new roster arrives; its claim settles anyway.
        gates["phone-a"].add_permits(1);
        wait_until("phone-a completion settled without a new roster", || {
            let (_, state, attempt) = job("phone-a");
            !(state == "running" && attempt == attempt_a)
        })
        .await;
        let (state, intent): (String, Option<String>) = conn
            .query_row(
                "SELECT state,effect_intent FROM publish_assignments WHERE id=?1",
                [&assignment_a],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(state, "failed_before_dispatch");
        assert_eq!(
            intent, None,
            "the listed phone's own read refused it before any effect"
        );
        let (state, code): (String, Option<String>) = conn
            .query_row(
                "SELECT state,error_code FROM publish_assignments WHERE udid='phone-c'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_ne!(state, "failed_before_dispatch", "{code:?}");
        assert_eq!(job("phone-c").1, "queued");

        // Stopping waits for phone B's own worker and keeps its claim.
        stop.store(true, std::sync::atomic::Ordering::Release);
        tokio::time::sleep(Duration::from_millis(600)).await;
        assert!(
            !dispatcher.is_finished(),
            "shutdown waits for the running worker"
        );
        assert_eq!(job("phone-b").1, "running");
        gates["phone-b"].add_permits(1);
        tokio::time::timeout(Duration::from_secs(10), &mut dispatcher)
            .await
            .expect("shutdown ends once phone B's worker does")
            .unwrap();
        assert_eq!(
            driver.scans.load(std::sync::atomic::Ordering::SeqCst),
            0,
            "the dispatcher reads the background scan and never scans the fleet itself"
        );
        assert_eq!(
            job("phone-b"),
            (assignment_b.clone(), "running".into(), attempt_b),
            "the durable claim is kept for startup replay"
        );
        let journal = fs::read_dir(path.with_extension("publish-completions"))
            .unwrap()
            .map(|entry| fs::read_to_string(entry.unwrap().path()).unwrap())
            .collect::<Vec<_>>();
        assert!(
            journal
                .iter()
                .any(|receipt| receipt.contains(&assignment_b)),
            "phone-b's completion is journaled for replay"
        );
        drop(conn);
        drop(db);
        let _ = fs::remove_file(path);
    }
}
