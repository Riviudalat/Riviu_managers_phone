use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{ensure, Context};
use async_trait::async_trait;
use parking_lot::Mutex;
use riviu_core::{
    cancel_orchestration, compile_orchestration, copy_bundle_to_managed,
    execute_automation_schedule_occurrence, execute_orchestration, plan_threads, resolve_target,
    scan_publish_folder, AutomationChildOwner, AutomationDefinitionRecord, AutomationKind,
    AutomationProfileRef, AutomationScheduleExecution, AutomationScheduleOccurrenceState,
    ChildCampaignOutcome, CompiledOrchestrationV1, InteractionAutomationProfileConfigV1,
    InteractionCampaignDetail, InteractionRunAggregate, NurtureAutomationProfileConfigV1,
    OrchestrationAttemptState, OrchestrationCancelResult, OrchestrationChildCancelResult,
    OrchestrationChildDispatch, OrchestrationChildFailure, OrchestrationChildPort,
    OrchestrationChildRequest, OrchestrationChildStatus, OrchestrationDocumentV1,
    OrchestrationExecution, OrchestrationIssue, OrchestrationNurtureChildRecord,
    OrchestrationNurtureChildState, OrchestrationRevisionRecord, OrchestrationRunDetail,
    OrchestrationRunRecord, OrchestrationRunState, OrchestrationSummary, Outcome,
    PublishAutomationProfileConfigV1, PublishCampaignRequest, PublishCampaignState,
    PublishCleanupPolicy, PublishScanOptions, PublishVisibility, ResolvedTargetSnapshot, TargetRef,
    ThreadCampaignState, ThreadMessageState,
};
use tauri::{AppHandle, Manager, State};
use uuid::Uuid;

use crate::command_error::CommandError;
use crate::state::AppState;

#[tauri::command]
pub fn orchestration_list(
    state: State<'_, AppState>,
    include_archived: bool,
) -> Result<Vec<OrchestrationSummary>, CommandError> {
    state
        .db
        .list_orchestrations(include_archived)
        .map_err(CommandError::from_service)
}

#[tauri::command]
pub fn orchestration_get(
    state: State<'_, AppState>,
    id: String,
    revision: Option<u64>,
) -> Result<Option<OrchestrationRevisionRecord>, CommandError> {
    state
        .db
        .get_orchestration_revision(parse_uuid(&id, "orchestration ID")?, revision)
        .map_err(CommandError::from_service)
}

#[tauri::command]
pub fn orchestration_validate(
    state: State<'_, AppState>,
    document: OrchestrationDocumentV1,
) -> Result<CompiledOrchestrationV1, Vec<CommandError>> {
    let profiles = load_profiles(&state, &document).map_err(|error| vec![error])?;
    compile_orchestration(&document, &profiles).map_err(map_issues)
}

#[tauri::command]
pub fn orchestration_save_revision(
    state: State<'_, AppState>,
    mut document: OrchestrationDocumentV1,
    expected_revision: Option<u64>,
) -> Result<OrchestrationRevisionRecord, CommandError> {
    let _admission = state.ensure_accepting_work()?;
    document.revision = expected_revision
        .unwrap_or(0)
        .checked_add(1)
        .ok_or_else(|| CommandError::invalid_argument("orchestration revision overflow"))?;
    let profiles = load_profiles(&state, &document)?;
    let compiled = compile_orchestration(&document, &profiles)
        .map_err(|issues| first_issue(issues, "OrchestrationInvalid"))?;
    state
        .db
        .save_orchestration_revision(expected_revision, &compiled)
        .map_err(CommandError::from_service)
}

#[tauri::command]
pub fn orchestration_archive(state: State<'_, AppState>, id: String) -> Result<(), CommandError> {
    let _admission = state.ensure_accepting_work()?;
    state
        .db
        .archive_orchestration(parse_uuid(&id, "orchestration ID")?)
        .map_err(CommandError::from_service)
}

#[derive(Clone)]
pub(crate) struct OrchestrationChildRuntime {
    inner: Arc<OrchestrationChildRuntimeInner>,
}

struct OrchestrationChildRuntimeInner {
    active_runs: Mutex<HashSet<Uuid>>,
    run_operations: Mutex<HashMap<Uuid, Arc<tokio::sync::Mutex<()>>>>,
    run_cancellations: Mutex<HashMap<Uuid, Arc<tokio::sync::Notify>>>,
    active_interaction_children: Mutex<HashSet<Uuid>>,
    active_publish_children: Mutex<HashSet<Uuid>>,
    nurture_children: Mutex<HashMap<Uuid, NurtureChild>>,
    schedule_runner_started: AtomicBool,
    schedule_stop_requested: AtomicBool,
    schedule_stop_notify: tokio::sync::Notify,
    schedule_runner_changed: tokio::sync::Notify,
    active_schedule_occurrences: Mutex<HashSet<Uuid>>,
    schedule_occurrences_changed: tokio::sync::Notify,
}

#[derive(Clone)]
struct NurtureChild {
    idempotency_key: String,
    run_id: Uuid,
    requested_udids: Vec<String>,
    started_udids: Vec<String>,
}

impl OrchestrationChildRuntime {
    pub(crate) fn new() -> Self {
        Self {
            inner: Arc::new(OrchestrationChildRuntimeInner {
                active_runs: Mutex::new(HashSet::new()),
                run_operations: Mutex::new(HashMap::new()),
                run_cancellations: Mutex::new(HashMap::new()),
                active_interaction_children: Mutex::new(HashSet::new()),
                active_publish_children: Mutex::new(HashSet::new()),
                nurture_children: Mutex::new(HashMap::new()),
                schedule_runner_started: AtomicBool::new(false),
                schedule_stop_requested: AtomicBool::new(false),
                schedule_stop_notify: tokio::sync::Notify::new(),
                schedule_runner_changed: tokio::sync::Notify::new(),
                active_schedule_occurrences: Mutex::new(HashSet::new()),
                schedule_occurrences_changed: tokio::sync::Notify::new(),
            }),
        }
    }

    fn reserve_run(&self, run_id: Uuid) -> bool {
        self.inner.active_runs.lock().insert(run_id)
    }

    fn run_is_active(&self, run_id: Uuid) -> bool {
        self.inner.active_runs.lock().contains(&run_id)
    }

    fn active_run_ids(&self) -> Vec<Uuid> {
        self.inner.active_runs.lock().iter().copied().collect()
    }

    fn run_operation(&self, run_id: Uuid) -> Arc<tokio::sync::Mutex<()>> {
        self.inner
            .run_operations
            .lock()
            .entry(run_id)
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
            .clone()
    }

    fn run_cancellation(&self, run_id: Uuid) -> Arc<tokio::sync::Notify> {
        self.inner
            .run_cancellations
            .lock()
            .entry(run_id)
            .or_insert_with(|| Arc::new(tokio::sync::Notify::new()))
            .clone()
    }

    fn release_run(&self, run_id: Uuid) {
        self.inner.active_runs.lock().remove(&run_id);
        self.inner.run_operations.lock().remove(&run_id);
        self.inner.run_cancellations.lock().remove(&run_id);
    }

    fn reserve_publish_child(&self, child_id: Uuid) -> bool {
        self.inner.active_publish_children.lock().insert(child_id)
    }

    fn reserve_interaction_child(&self, child_id: Uuid) -> bool {
        self.inner
            .active_interaction_children
            .lock()
            .insert(child_id)
    }

    fn release_interaction_child(&self, child_id: Uuid) {
        self.inner
            .active_interaction_children
            .lock()
            .remove(&child_id);
    }

    fn release_publish_child(&self, child_id: Uuid) {
        self.inner.active_publish_children.lock().remove(&child_id);
    }

    fn nurture_child(&self, child_id: Uuid) -> Option<NurtureChild> {
        self.inner.nurture_children.lock().get(&child_id).cloned()
    }

    fn insert_nurture_child(&self, child_id: Uuid, child: NurtureChild) -> bool {
        let mut children = self.inner.nurture_children.lock();
        if children.contains_key(&child_id) {
            return false;
        }
        children.insert(child_id, child);
        true
    }

    fn start_schedule_runner(&self) -> bool {
        if self
            .inner
            .schedule_runner_started
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return false;
        }
        self.inner
            .schedule_stop_requested
            .store(false, Ordering::Release);
        true
    }

    fn finish_schedule_runner(&self) {
        self.inner
            .schedule_runner_started
            .store(false, Ordering::Release);
        self.inner.schedule_runner_changed.notify_waiters();
    }

    fn schedule_stop_requested(&self) -> bool {
        self.inner.schedule_stop_requested.load(Ordering::Acquire)
    }

    fn request_schedule_stop(&self) {
        self.inner
            .schedule_stop_requested
            .store(true, Ordering::Release);
        self.inner.schedule_stop_notify.notify_waiters();
    }

    fn reserve_schedule_occurrence(&self, occurrence_id: Uuid) -> bool {
        self.inner
            .active_schedule_occurrences
            .lock()
            .insert(occurrence_id)
    }

    fn release_schedule_occurrence(&self, occurrence_id: Uuid) {
        self.inner
            .active_schedule_occurrences
            .lock()
            .remove(&occurrence_id);
        self.inner.schedule_occurrences_changed.notify_waiters();
    }

    async fn wait_for_schedule_occurrences(&self) {
        loop {
            let changed = self.inner.schedule_occurrences_changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            if self.inner.active_schedule_occurrences.lock().is_empty() {
                return;
            }
            changed.await;
        }
    }

    async fn wait_for_schedule_runner(&self) {
        loop {
            let changed = self.inner.schedule_runner_changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            if !self.inner.schedule_runner_started.load(Ordering::Acquire) {
                return;
            }
            changed.await;
        }
    }
}

struct ProductionOrchestrationPort {
    app: AppHandle,
    db: Arc<riviu_core::db::Database>,
    registry: riviu_core::DeviceRegistry,
    control: Arc<riviu_core::DeviceControlPlane>,
    streams: riviu_ios_driver::StreamHub,
    events: riviu_core::EventBus,
    interaction_artifacts: riviu_core::FlowArtifactStore,
    nurture: crate::nurture_commands::NurtureRuntime,
    nurture_engine: riviu_core::NurtureEngine,
    runtime: OrchestrationChildRuntime,
    delay_cancellation: Arc<tokio::sync::Notify>,
    artifacts_dir: std::path::PathBuf,
}

impl ProductionOrchestrationPort {
    fn from_state(app: AppHandle, state: &AppState, run_id: Uuid) -> Self {
        Self {
            app,
            db: state.db.clone(),
            registry: state.registry.clone(),
            control: state.control.clone(),
            streams: state.streams.clone(),
            events: state.events.clone(),
            interaction_artifacts: state.interaction_artifacts.clone(),
            nurture: state.nurture.clone(),
            nurture_engine: state.nurture_engine.clone(),
            runtime: state.orchestration.clone(),
            delay_cancellation: state.orchestration.run_cancellation(run_id),
            artifacts_dir: state.artifacts_dir.clone(),
        }
    }

    fn from_schedule_state(app: AppHandle, state: &AppState) -> Self {
        Self {
            app,
            db: state.db.clone(),
            registry: state.registry.clone(),
            control: state.control.clone(),
            streams: state.streams.clone(),
            events: state.events.clone(),
            interaction_artifacts: state.interaction_artifacts.clone(),
            nurture: state.nurture.clone(),
            nurture_engine: state.nurture_engine.clone(),
            runtime: state.orchestration.clone(),
            delay_cancellation: Arc::new(tokio::sync::Notify::new()),
            artifacts_dir: state.artifacts_dir.clone(),
        }
    }

    fn profile(
        &self,
        request: &OrchestrationChildRequest,
    ) -> Result<AutomationDefinitionRecord, OrchestrationChildFailure> {
        let record = self
            .db
            .get_automation_definition_record(
                request.profile.definition_id,
                request.profile.revision,
            )
            .map_err(|error| OrchestrationChildFailure::before_effect(error.to_string()))?
            .ok_or_else(|| {
                OrchestrationChildFailure::before_effect("pinned automation profile is missing")
            })?;
        if record.definition.archived
            || record.definition.kind != request.kind
            || record.revision.revision != request.profile.revision
        {
            return Err(OrchestrationChildFailure::before_effect(
                "pinned automation profile is unavailable or has the wrong kind",
            ));
        }
        Ok(record)
    }

    fn target_udids(request: &OrchestrationChildRequest) -> Vec<String> {
        request
            .target
            .included
            .iter()
            .map(|device| device.udid.clone())
            .collect()
    }

    async fn dispatch_nurture(
        &self,
        request: &OrchestrationChildRequest,
        record: AutomationDefinitionRecord,
    ) -> Result<OrchestrationChildDispatch, OrchestrationChildFailure> {
        let config: NurtureAutomationProfileConfigV1 =
            serde_json::from_value(record.revision.config).map_err(|error| {
                OrchestrationChildFailure::before_effect(format!(
                    "invalid pinned nurture profile: {error}"
                ))
            })?;
        let mut settings = config.settings;
        let live = self
            .db
            .get_nurture_settings()
            .map_err(|error| OrchestrationChildFailure::before_effect(error.to_string()))?;
        settings.api_key = live.api_key;
        settings.has_api_key = !settings.api_key.trim().is_empty();
        settings.schedule_enabled = false;
        settings.schedule_udids.clear();
        settings.schedule_windows.clear();
        crate::nurture_commands::validate_nurture_settings(&settings)
            .map_err(OrchestrationChildFailure::before_effect)?;
        let requested_udids = Self::target_udids(request);
        let preflight = crate::nurture_commands::preflight_comment_job(
            &self.control,
            &requested_udids,
            &settings,
        )
        .await;
        if preflight.ready.is_empty() {
            return Err(OrchestrationChildFailure::before_effect(
                preflight.refusal(),
            ));
        }
        let duration = config
            .duration_minutes
            .map(|minutes| Duration::from_secs(u64::from(minutes) * 60));
        if matches!(
            request.owner,
            AutomationChildOwner::ScheduleOccurrence { .. }
        ) {
            return self
                .dispatch_scheduled_nurture(
                    request,
                    requested_udids,
                    preflight.ready,
                    settings,
                    duration,
                )
                .await;
        }
        let (durable, created) = self
            .db
            .create_orchestration_nurture_child(
                match request.owner {
                    AutomationChildOwner::OrchestrationAttempt { attempt_id } => attempt_id,
                    AutomationChildOwner::ScheduleOccurrence { .. } => unreachable!(),
                },
                request.child_campaign_id,
                &request.idempotency_key,
            )
            .map_err(|error| OrchestrationChildFailure::after_effect(error.to_string()))?;
        if !created {
            return self
                .reconcile_nurture(request, durable)
                .map(status_as_dispatch)
                .map_err(OrchestrationChildFailure::after_effect);
        }
        let started_udids = self
            .nurture
            .start_many(
                self.app.clone(),
                self.nurture_engine.clone(),
                preflight.ready,
                settings,
                duration,
            )
            .await;
        if started_udids.is_empty() {
            self.db
                .settle_orchestration_nurture_child(
                    request.child_campaign_id,
                    &request.idempotency_key,
                    ChildCampaignOutcome::Failed,
                )
                .map_err(|error| OrchestrationChildFailure::after_effect(error.to_string()))?;
            return Err(OrchestrationChildFailure::before_effect(
                "no nurture session could reserve its device",
            ));
        }
        let statuses = self.nurture.list_status();
        let run_id = statuses
            .iter()
            .find(|status| started_udids.contains(&status.udid))
            .and_then(|status| status.run_id)
            .ok_or_else(|| {
                let _ = self.db.settle_orchestration_nurture_child(
                    request.child_campaign_id,
                    &request.idempotency_key,
                    ChildCampaignOutcome::Uncertain,
                );
                OrchestrationChildFailure::after_effect(
                    "nurture child started without a runtime identity",
                )
            })?;
        let child = NurtureChild {
            idempotency_key: request.idempotency_key.clone(),
            run_id,
            requested_udids,
            started_udids,
        };
        self.db
            .start_orchestration_nurture_child(
                request.child_campaign_id,
                &request.idempotency_key,
                run_id,
                &child.started_udids,
            )
            .map_err(|error| {
                let _ = self.db.settle_orchestration_nurture_child(
                    request.child_campaign_id,
                    &request.idempotency_key,
                    ChildCampaignOutcome::Uncertain,
                );
                OrchestrationChildFailure::after_effect(error.to_string())
            })?;
        if !self
            .runtime
            .insert_nurture_child(request.child_campaign_id, child)
        {
            let _ = self.db.settle_orchestration_nurture_child(
                request.child_campaign_id,
                &request.idempotency_key,
                ChildCampaignOutcome::Uncertain,
            );
            return Err(OrchestrationChildFailure::after_effect(
                "nurture child was concurrently registered",
            ));
        }
        Ok(OrchestrationChildDispatch::Started)
    }

    async fn dispatch_interaction(
        &self,
        request: &OrchestrationChildRequest,
        record: AutomationDefinitionRecord,
    ) -> Result<OrchestrationChildDispatch, OrchestrationChildFailure> {
        if let Some(existing) = self
            .db
            .get_interaction_campaign(&request.child_campaign_id.to_string())
            .map_err(|error| OrchestrationChildFailure::after_effect(error.to_string()))?
        {
            if existing.summary.request_id != request.idempotency_key {
                return Err(OrchestrationChildFailure::after_effect(
                    "interaction child idempotency conflict",
                ));
            }
            return Ok(status_as_dispatch(interaction_status(&existing)));
        }
        let config: InteractionAutomationProfileConfigV1 =
            serde_json::from_value(record.revision.config).map_err(|error| {
                OrchestrationChildFailure::before_effect(format!(
                    "invalid pinned interaction profile: {error}"
                ))
            })?;
        let campaign = config
            .into_campaign_request(request.idempotency_key.clone(), Self::target_udids(request));
        crate::interaction_commands::require_parent_locator(
            &self.control,
            campaign.mode,
            campaign.actions,
            &campaign.actor_udids,
        )
        .map_err(|error| OrchestrationChildFailure::before_effect(error.message.to_string()))?;
        let settings = self
            .db
            .get_nurture_settings()
            .map_err(|error| OrchestrationChildFailure::before_effect(error.to_string()))?;
        if riviu_core::interaction_campaign::ai_key_missing(&campaign, &settings.api_key) {
            return Err(OrchestrationChildFailure::before_effect(
                "interaction profile requires an AI key that is not configured",
            ));
        }
        let plan = plan_threads(&campaign)
            .map_err(|error| OrchestrationChildFailure::before_effect(error.to_string()))?;
        let child_id = request.child_campaign_id.to_string();
        let (_id, created) = self
            .db
            .create_interaction_campaign_with_id(&child_id, &campaign, &plan)
            .map_err(|error| OrchestrationChildFailure::before_effect(error.to_string()))?;
        if !created {
            let detail = self
                .db
                .get_interaction_campaign(&child_id)
                .map_err(|error| OrchestrationChildFailure::after_effect(error.to_string()))?
                .ok_or_else(|| {
                    OrchestrationChildFailure::after_effect(
                        "idempotent interaction child disappeared",
                    )
                })?;
            return Ok(status_as_dispatch(interaction_status(&detail)));
        }
        self.db
            .update_interaction_campaign_state(&child_id, ThreadCampaignState::Running, None)
            .map_err(|error| OrchestrationChildFailure::before_effect(error.to_string()))?;
        self.events.emit(riviu_core::AppEvent::InteractionUpdated {
            campaign_id: child_id.clone(),
            revision: riviu_core::interaction_campaign::revision(),
        });
        spawn_interaction_child(self, request.child_campaign_id, campaign, plan);
        Ok(OrchestrationChildDispatch::Started)
    }

    async fn dispatch_publish(
        &self,
        request: &OrchestrationChildRequest,
        record: AutomationDefinitionRecord,
    ) -> Result<OrchestrationChildDispatch, OrchestrationChildFailure> {
        let child_id = request.child_campaign_id.to_string();
        if let Some(existing) = self
            .db
            .get_publish_campaign(&child_id)
            .map_err(|error| OrchestrationChildFailure::after_effect(error.to_string()))?
        {
            if existing.campaign.request_id != request.idempotency_key {
                return Err(OrchestrationChildFailure::after_effect(
                    "publish child idempotency conflict",
                ));
            }
            return Ok(status_as_dispatch(publish_status(existing.campaign.state)));
        }
        let config: PublishAutomationProfileConfigV1 =
            serde_json::from_value(record.revision.config).map_err(|error| {
                OrchestrationChildFailure::before_effect(format!(
                    "invalid pinned publish profile: {error}"
                ))
            })?;
        if !config.execution_confirmed {
            return Err(OrchestrationChildFailure::before_effect(
                "publish automation profile has no execution confirmation",
            ));
        }
        let manifest = scan_publish_folder(&config.source_root, PublishScanOptions::default())
            .map_err(|error| OrchestrationChildFailure::before_effect(error.to_string()))?;
        let selected = config
            .bundle_ids
            .iter()
            .map(|id| {
                manifest
                    .bundles
                    .iter()
                    .find(|bundle| bundle.id == *id)
                    .cloned()
                    .ok_or_else(|| format!("bundle not found in pinned publish source: {id}"))
            })
            .collect::<Result<Vec<_>, _>>()
            .map_err(OrchestrationChildFailure::before_effect)?;
        let staging_root = self
            .artifacts_dir
            .join("publish")
            .join("orchestration")
            .join(&child_id);
        let mut managed = Vec::with_capacity(selected.len());
        for mut bundle in selected {
            if let Some(caption) = config.caption_overrides.get(&bundle.id) {
                if caption.trim().is_empty() {
                    return Err(OrchestrationChildFailure::before_effect(
                        "publish caption override is empty",
                    ));
                }
                bundle.caption = caption.trim().to_string();
                bundle.caption_sha256 = riviu_core::frame_sha256(bundle.caption.as_bytes());
            }
            let source_id = bundle.id.clone();
            let destination = staging_root.join(&source_id);
            let mut copied = copy_bundle_to_managed(&bundle, &destination)
                .map_err(|error| OrchestrationChildFailure::before_effect(error.to_string()))?;
            copied.id = format!("{child_id}:{source_id}");
            managed.push(copied);
        }
        let campaign = PublishCampaignRequest {
            request_id: request.idempotency_key.clone(),
            source_root: config.source_root,
            bundle_ids: managed.iter().map(|bundle| bundle.id.clone()).collect(),
            udids: Self::target_udids(request),
            run_at: None,
            visibility: PublishVisibility::Public,
            cleanup_policy: PublishCleanupPolicy::DeleteImportedAssetsAfterVerified,
            sound_policy: config.sound_policy.clone(),
            execution_confirmed: true,
        };
        let (_record, created) = self
            .db
            .create_publish_campaign_with_id(&child_id, &campaign, &managed)
            .map_err(|error| {
                let _ = fs::remove_dir_all(&staging_root);
                OrchestrationChildFailure::before_effect(error.to_string())
            })?;
        if !created {
            let detail = self
                .db
                .get_publish_campaign(&child_id)
                .map_err(|error| OrchestrationChildFailure::after_effect(error.to_string()))?
                .context("idempotent publish child disappeared")
                .map_err(|error| OrchestrationChildFailure::after_effect(error.to_string()))?;
            return Ok(status_as_dispatch(publish_status(detail.campaign.state)));
        }
        spawn_publish_child(self, request.child_campaign_id);
        Ok(OrchestrationChildDispatch::Started)
    }

    fn nurture_status(&self, child: &NurtureChild) -> Result<OrchestrationChildStatus, String> {
        let statuses = self.nurture.list_status();
        let exact = child
            .started_udids
            .iter()
            .map(|udid| {
                statuses
                    .iter()
                    .find(|status| status.udid == *udid && status.run_id == Some(child.run_id))
            })
            .collect::<Option<Vec<_>>>()
            .ok_or_else(|| "nurture child status disappeared after dispatch".to_string())?;
        if exact.iter().any(|status| status.running) {
            return Ok(OrchestrationChildStatus::Running);
        }
        if exact
            .iter()
            .any(|status| !status.phase.is_terminal() || status.outcome.is_none())
        {
            return Ok(OrchestrationChildStatus::Running);
        }
        let outcomes = exact
            .iter()
            .filter_map(|status| status.outcome)
            .collect::<Vec<_>>();
        let outcome = if outcomes.iter().all(|outcome| *outcome == Outcome::Done)
            && child.started_udids.len() == child.requested_udids.len()
        {
            ChildCampaignOutcome::Done
        } else if outcomes
            .iter()
            .all(|outcome| matches!(outcome, Outcome::Failed | Outcome::Stopped))
        {
            ChildCampaignOutcome::Failed
        } else {
            ChildCampaignOutcome::Partial
        };
        Ok(OrchestrationChildStatus::Finished(outcome))
    }

    fn reconcile_nurture(
        &self,
        request: &OrchestrationChildRequest,
        durable: OrchestrationNurtureChildRecord,
    ) -> Result<OrchestrationChildStatus, String> {
        let AutomationChildOwner::OrchestrationAttempt { attempt_id } = request.owner else {
            return Err("nurture orchestration child has the wrong owner kind".into());
        };
        if durable.id != request.child_campaign_id
            || durable.attempt_id != attempt_id
            || durable.idempotency_key != request.idempotency_key
        {
            return Err("nurture child durable identity conflict".into());
        }
        if let Some(outcome) = durable.state.outcome() {
            return Ok(OrchestrationChildStatus::Finished(outcome));
        }
        let Some(child) = self.runtime.nurture_child(request.child_campaign_id) else {
            let terminal = self
                .db
                .settle_orchestration_nurture_child(
                    request.child_campaign_id,
                    &request.idempotency_key,
                    ChildCampaignOutcome::Uncertain,
                )
                .map_err(|error| error.to_string())?;
            return Ok(OrchestrationChildStatus::Finished(
                terminal
                    .state
                    .outcome()
                    .unwrap_or(ChildCampaignOutcome::Uncertain),
            ));
        };
        if durable.state != OrchestrationNurtureChildState::Running
            || durable.run_id != Some(child.run_id)
            || durable.requested_udids != child.requested_udids
            || durable.started_udids != child.started_udids
            || child.idempotency_key != request.idempotency_key
        {
            return Err("nurture child runtime identity conflicts with durable state".into());
        }
        let status = match self.nurture_status(&child) {
            Ok(status) => status,
            Err(_) => OrchestrationChildStatus::Finished(ChildCampaignOutcome::Uncertain),
        };
        if let OrchestrationChildStatus::Finished(outcome) = status {
            self.db
                .settle_orchestration_nurture_child(
                    request.child_campaign_id,
                    &request.idempotency_key,
                    outcome,
                )
                .map_err(|error| error.to_string())?;
        }
        Ok(status)
    }

    async fn dispatch_scheduled_nurture(
        &self,
        request: &OrchestrationChildRequest,
        requested_udids: Vec<String>,
        ready_udids: Vec<String>,
        settings: riviu_core::NurtureSettings,
        duration: Option<Duration>,
    ) -> Result<OrchestrationChildDispatch, OrchestrationChildFailure> {
        let AutomationChildOwner::ScheduleOccurrence { occurrence_id } = request.owner else {
            unreachable!();
        };
        let started_udids = self
            .nurture
            .start_many(
                self.app.clone(),
                self.nurture_engine.clone(),
                ready_udids,
                settings,
                duration,
            )
            .await;
        if started_udids.is_empty() {
            return Err(OrchestrationChildFailure::before_effect(
                "no nurture session could reserve its device",
            ));
        }
        let run_id = self
            .nurture
            .list_status()
            .iter()
            .find(|status| started_udids.contains(&status.udid))
            .and_then(|status| status.run_id)
            .ok_or_else(|| {
                OrchestrationChildFailure::after_effect(
                    "scheduled nurture child started without a runtime identity",
                )
            })?;
        let child = NurtureChild {
            idempotency_key: request.idempotency_key.clone(),
            run_id,
            requested_udids,
            started_udids,
        };
        self.db
            .record_scheduled_nurture_started(
                occurrence_id,
                request.child_campaign_id,
                &request.idempotency_key,
                run_id,
                &child.started_udids,
            )
            .map_err(|error| OrchestrationChildFailure::after_effect(error.to_string()))?;
        if !self
            .runtime
            .insert_nurture_child(request.child_campaign_id, child)
        {
            return Err(OrchestrationChildFailure::after_effect(
                "scheduled nurture child was concurrently registered",
            ));
        }
        Ok(OrchestrationChildDispatch::Started)
    }

    fn reconcile_scheduled_nurture(
        &self,
        request: &OrchestrationChildRequest,
    ) -> Result<OrchestrationChildStatus, String> {
        let AutomationChildOwner::ScheduleOccurrence { occurrence_id } = request.owner else {
            return Err("scheduled nurture child has the wrong owner kind".into());
        };
        let occurrence = self
            .db
            .get_automation_schedule_occurrence(occurrence_id)
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "scheduled nurture occurrence is missing".to_string())?;
        if occurrence.child_campaign_id != request.child_campaign_id
            || occurrence.idempotency_key != request.idempotency_key
            || occurrence.kind != AutomationKind::Nurture
        {
            return Err("scheduled nurture child identity conflict".into());
        }
        if let Some(outcome) = occurrence.state.outcome() {
            return Ok(OrchestrationChildStatus::Finished(outcome));
        }
        let Some(child) = self.runtime.nurture_child(request.child_campaign_id) else {
            return Ok(OrchestrationChildStatus::Finished(
                ChildCampaignOutcome::Uncertain,
            ));
        };
        if occurrence.state != riviu_core::AutomationScheduleOccurrenceState::Running
            || occurrence.nurture_run_id != Some(child.run_id)
            || occurrence.nurture_started_udids != child.started_udids
            || child.idempotency_key != request.idempotency_key
        {
            return Err("scheduled nurture runtime identity conflict".into());
        }
        self.nurture_status(&child)
    }

    fn cancel_scheduled_nurture(
        &self,
        request: &OrchestrationChildRequest,
    ) -> Result<OrchestrationChildCancelResult, String> {
        let status = self.reconcile_scheduled_nurture(request)?;
        if let OrchestrationChildStatus::Finished(outcome) = status {
            return Ok(OrchestrationChildCancelResult::Finished(outcome));
        }
        let child = self
            .runtime
            .nurture_child(request.child_campaign_id)
            .ok_or_else(|| "scheduled nurture runtime identity disappeared".to_string())?;
        for udid in &child.started_udids {
            self.nurture.stop(udid);
        }
        Ok(OrchestrationChildCancelResult::EffectMayHaveStarted)
    }
}

#[async_trait]
impl OrchestrationChildPort for ProductionOrchestrationPort {
    async fn dispatch_child(
        &mut self,
        request: OrchestrationChildRequest,
    ) -> Result<OrchestrationChildDispatch, OrchestrationChildFailure> {
        let profile = self.profile(&request)?;
        match request.kind {
            AutomationKind::Nurture => self.dispatch_nurture(&request, profile).await,
            AutomationKind::Interaction => self.dispatch_interaction(&request, profile).await,
            AutomationKind::Publish => self.dispatch_publish(&request, profile).await,
        }
    }

    async fn reconcile_child(
        &mut self,
        request: &OrchestrationChildRequest,
    ) -> Result<OrchestrationChildStatus, String> {
        match request.kind {
            AutomationKind::Nurture => {
                if matches!(
                    request.owner,
                    AutomationChildOwner::ScheduleOccurrence { .. }
                ) {
                    return self.reconcile_scheduled_nurture(request);
                }
                match self
                    .db
                    .get_orchestration_nurture_child(request.child_campaign_id)
                    .map_err(|error| error.to_string())?
                {
                    Some(durable) => self.reconcile_nurture(request, durable),
                    None => Ok(OrchestrationChildStatus::MissingBeforeEffect),
                }
            }
            AutomationKind::Interaction => {
                let detail = self
                    .db
                    .get_interaction_campaign(&request.child_campaign_id.to_string())
                    .map_err(|error| error.to_string())?;
                Ok(match detail {
                    Some(detail) if detail.summary.request_id == request.idempotency_key => {
                        let status = interaction_status(&detail);
                        if status == OrchestrationChildStatus::Running {
                            resume_interaction_child(self, request.child_campaign_id)?;
                        }
                        status
                    }
                    Some(_) => return Err("interaction child idempotency conflict".into()),
                    None => OrchestrationChildStatus::MissingBeforeEffect,
                })
            }
            AutomationKind::Publish => {
                let detail = self
                    .db
                    .get_publish_campaign(&request.child_campaign_id.to_string())
                    .map_err(|error| error.to_string())?;
                match detail {
                    Some(detail) if detail.campaign.request_id == request.idempotency_key => {
                        let status = publish_status(detail.campaign.state);
                        if status == OrchestrationChildStatus::Running {
                            spawn_publish_child(self, request.child_campaign_id);
                        }
                        Ok(status)
                    }
                    Some(_) => Err("publish child idempotency conflict".into()),
                    None => Ok(OrchestrationChildStatus::MissingBeforeEffect),
                }
            }
        }
    }

    async fn cancel_child(
        &mut self,
        request: &OrchestrationChildRequest,
    ) -> Result<OrchestrationChildCancelResult, String> {
        match request.kind {
            AutomationKind::Nurture => {
                let AutomationChildOwner::OrchestrationAttempt { attempt_id } = request.owner
                else {
                    return self.cancel_scheduled_nurture(request);
                };
                let Some(durable) = self
                    .db
                    .get_orchestration_nurture_child(request.child_campaign_id)
                    .map_err(|error| error.to_string())?
                else {
                    return Ok(OrchestrationChildCancelResult::CancelledBeforeEffect);
                };
                if durable.attempt_id != attempt_id
                    || durable.idempotency_key != request.idempotency_key
                {
                    return Err("nurture child durable identity conflict".into());
                }
                let status = self.reconcile_nurture(request, durable)?;
                if let OrchestrationChildStatus::Finished(outcome) = status {
                    return Ok(OrchestrationChildCancelResult::Finished(outcome));
                }
                let child = self
                    .runtime
                    .nurture_child(request.child_campaign_id)
                    .ok_or_else(|| "running nurture child lost its runtime identity".to_string())?;
                for udid in &child.started_udids {
                    if self
                        .nurture
                        .list_status()
                        .iter()
                        .any(|status| status.udid == *udid && status.run_id == Some(child.run_id))
                    {
                        self.nurture.stop(udid);
                    }
                }
                Ok(OrchestrationChildCancelResult::EffectMayHaveStarted)
            }
            AutomationKind::Interaction => {
                let child_id = request.child_campaign_id.to_string();
                let Some(detail) = self
                    .db
                    .get_interaction_campaign(&child_id)
                    .map_err(|error| error.to_string())?
                else {
                    return Ok(OrchestrationChildCancelResult::CancelledBeforeEffect);
                };
                if let OrchestrationChildStatus::Finished(outcome) = interaction_status(&detail) {
                    return Ok(OrchestrationChildCancelResult::Finished(outcome));
                }
                self.db
                    .cancel_interaction_campaign(&child_id)
                    .map_err(|error| error.to_string())?;
                let after = self
                    .db
                    .get_interaction_campaign(&child_id)
                    .map_err(|error| error.to_string())?
                    .context("cancelled interaction child disappeared")
                    .map_err(|error| error.to_string())?;
                if interaction_effect_may_have_started(&after) {
                    Ok(OrchestrationChildCancelResult::EffectMayHaveStarted)
                } else {
                    Ok(OrchestrationChildCancelResult::CancelledBeforeEffect)
                }
            }
            AutomationKind::Publish => {
                let child_id = request.child_campaign_id.to_string();
                let Some(detail) = self
                    .db
                    .get_publish_campaign(&child_id)
                    .map_err(|error| error.to_string())?
                else {
                    return Ok(OrchestrationChildCancelResult::CancelledBeforeEffect);
                };
                if let OrchestrationChildStatus::Finished(outcome) =
                    publish_status(detail.campaign.state)
                {
                    return Ok(OrchestrationChildCancelResult::Finished(outcome));
                }
                match self
                    .db
                    .cancel_publish_campaign(&child_id)
                    .map_err(|error| error.to_string())?
                {
                    Some(PublishCampaignState::Cancelled) => {
                        Ok(OrchestrationChildCancelResult::CancelledBeforeEffect)
                    }
                    Some(_) => Ok(OrchestrationChildCancelResult::EffectMayHaveStarted),
                    None => Ok(OrchestrationChildCancelResult::CancelledBeforeEffect),
                }
            }
        }
    }

    async fn wait_delay(&mut self, duration: Duration) -> Result<(), String> {
        tokio::select! {
            _ = tokio::time::sleep(duration) => Ok(()),
            _ = self.delay_cancellation.notified() => {
                Err("delay interrupted by cancellation".into())
            }
        }
    }
}

#[tauri::command]
pub async fn orchestration_run(
    app: AppHandle,
    state: State<'_, AppState>,
    document_id: String,
    revision: u64,
    target: TargetRef,
) -> Result<OrchestrationRunDetail, CommandError> {
    let _admission = state.ensure_accepting_work()?;
    let document_id = parse_uuid(&document_id, "orchestration ID")?;
    let revision_record = state
        .db
        .get_orchestration_revision(document_id, Some(revision))
        .map_err(CommandError::from_service)?
        .ok_or_else(|| CommandError::code("OrchestrationMissing", "revision does not exist"))?;
    let (snapshot, node_targets) = resolve_confirmed_targets(
        &state.db,
        &state.registry,
        &target,
        &revision_record.compiled,
    )
    .map_err(CommandError::from_service)?;
    let run = state
        .db
        .create_orchestration_run(document_id, revision, &snapshot, &node_targets)
        .map_err(CommandError::from_service)?;
    let detail = state
        .db
        .get_orchestration_run(run.id)
        .map_err(CommandError::from_service)?
        .ok_or_else(|| CommandError::code("OrchestrationRunMissing", "created run disappeared"))?;
    spawn_orchestration_worker(app, &state, run.id)?;
    Ok(detail)
}

#[tauri::command]
pub fn orchestration_list_runs(
    state: State<'_, AppState>,
    limit: Option<usize>,
) -> Result<Vec<OrchestrationRunRecord>, CommandError> {
    state
        .db
        .list_orchestration_runs(limit.unwrap_or(50).clamp(1, 200))
        .map_err(CommandError::from_service)
}

#[tauri::command]
pub fn orchestration_get_run(
    state: State<'_, AppState>,
    run_id: String,
) -> Result<Option<OrchestrationRunDetail>, CommandError> {
    state
        .db
        .get_orchestration_run(parse_uuid(&run_id, "orchestration run ID")?)
        .map_err(CommandError::from_service)
}

#[tauri::command]
pub fn orchestration_reconcile(
    app: AppHandle,
    state: State<'_, AppState>,
    run_id: String,
) -> Result<OrchestrationRunDetail, CommandError> {
    let _admission = state.ensure_accepting_work()?;
    let run_id = parse_uuid(&run_id, "orchestration run ID")?;
    let detail = state
        .db
        .get_orchestration_run(run_id)
        .map_err(CommandError::from_service)?
        .ok_or_else(|| CommandError::code("OrchestrationRunMissing", "run does not exist"))?;
    if !detail.run.state.is_terminal() && !state.orchestration.run_is_active(run_id) {
        spawn_orchestration_worker(app, &state, run_id)?;
    }
    Ok(detail)
}

#[tauri::command]
pub async fn orchestration_cancel_run(
    app: AppHandle,
    state: State<'_, AppState>,
    run_id: String,
) -> Result<OrchestrationRunDetail, CommandError> {
    let _admission = state.ensure_accepting_work()?;
    let run_id = parse_uuid(&run_id, "orchestration run ID")?;
    let initial = state
        .db
        .get_orchestration_run(run_id)
        .map_err(CommandError::from_service)?
        .ok_or_else(|| CommandError::code("OrchestrationRunMissing", "run does not exist"))?;
    if initial.run.state.is_terminal() {
        return Ok(initial);
    }
    let _ = state
        .db
        .request_orchestration_cancel(run_id)
        .map_err(CommandError::from_service)?;
    state.orchestration.run_cancellation(run_id).notify_one();
    let operation = state.orchestration.run_operation(run_id);
    let _operation = operation.lock().await;
    let mut port = ProductionOrchestrationPort::from_state(app, &state, run_id);
    let _result: OrchestrationCancelResult = cancel_orchestration(&state.db, run_id, &mut port)
        .await
        .map_err(CommandError::from_service)?;
    state
        .db
        .get_orchestration_run(run_id)
        .map_err(CommandError::from_service)?
        .ok_or_else(|| CommandError::code("OrchestrationRunMissing", "run disappeared"))
}

pub(crate) fn start_automation_schedule_runner(app: AppHandle, state: &AppState) {
    if !state.orchestration.start_schedule_runner() {
        return;
    }
    let runtime = state.orchestration.clone();
    tauri::async_runtime::spawn(async move {
        resume_automation_schedule_occurrences(app.clone());

        // Schedule intervals are measured in minutes. A five-second scan bounds ordinary
        // desktop wake-up lateness without putting a per-second read loop on SQLite.
        let mut ticker = tokio::time::interval(Duration::from_secs(5));
        loop {
            if runtime.schedule_stop_requested() {
                break;
            }
            tokio::select! {
                _ = ticker.tick() => {
                    tick_automation_schedules(app.clone());
                }
                _ = runtime.inner.schedule_stop_notify.notified() => {}
            }
        }
        runtime.finish_schedule_runner();
    });
}

pub(crate) async fn shutdown_automation_schedule_runner(state: &AppState) {
    state.orchestration.request_schedule_stop();
    state.orchestration.wait_for_schedule_runner().await;
    state.orchestration.wait_for_schedule_occurrences().await;
}

fn resume_automation_schedule_occurrences(app: AppHandle) {
    let state = app.state::<AppState>();
    let occurrences = match state.db.list_recoverable_automation_schedule_occurrences() {
        Ok(occurrences) => occurrences,
        Err(error) => {
            log::error!("could not list automation schedule occurrences for recovery: {error:#}");
            return;
        }
    };
    for occurrence in occurrences {
        if let Err(error) =
            spawn_automation_schedule_occurrence_worker(app.clone(), &state, occurrence.id)
        {
            log::error!(
                "could not recover automation schedule occurrence {}: {}",
                occurrence.id,
                error.message
            );
        }
    }
}

fn tick_automation_schedules(app: AppHandle) {
    let state = app.state::<AppState>();
    let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Nanos, true);
    let schedules = match state.db.list_due_automation_schedules(&now, 25) {
        Ok(schedules) => schedules,
        Err(error) => {
            log::error!("could not list due automation schedules: {error:#}");
            return;
        }
    };
    for schedule in schedules {
        let Some(scheduled_for) = schedule.next_due_at.clone() else {
            continue;
        };
        let record = match state
            .db
            .get_automation_definition_record(schedule.definition_id, schedule.definition_revision)
        {
            Ok(Some(record)) => record,
            Ok(None) => {
                log::error!(
                    "automation schedule {} lost pinned profile {} revision {}",
                    schedule.id,
                    schedule.definition_id,
                    schedule.definition_revision
                );
                continue;
            }
            Err(error) => {
                log::error!(
                    "could not load automation schedule {} pinned profile: {error:#}",
                    schedule.id
                );
                continue;
            }
        };
        let resolved = if record.definition.archived {
            Err(anyhow::anyhow!("pinned profile is archived"))
        } else {
            resolve_target_snapshot(&state.db, &state.registry, &record.revision.target_ref)
        };
        let (target, error_code) = match resolved {
            Ok(target) => (Some(target), None),
            Err(error) => {
                log::warn!(
                    "automation schedule {} could not resolve its due target: {error:#}",
                    schedule.id
                );
                let code = if record.definition.archived {
                    "profile_archived"
                } else {
                    "target_resolution_failed"
                };
                (None, Some(code))
            }
        };
        let occurrence = match state.db.claim_automation_schedule_occurrence(
            schedule.id,
            schedule.revision,
            &scheduled_for,
            &now,
            target.as_ref(),
            error_code,
        ) {
            Ok(occurrence) => occurrence,
            Err(error) => {
                log::error!(
                    "could not claim automation schedule {} due slot {}: {error:#}",
                    schedule.id,
                    scheduled_for
                );
                continue;
            }
        };
        let Some(occurrence) = occurrence else {
            continue;
        };
        if occurrence.state.is_terminal() {
            continue;
        }
        if let Err(error) =
            spawn_automation_schedule_occurrence_worker(app.clone(), &state, occurrence.id)
        {
            log::error!(
                "could not start automation schedule occurrence {}: {}",
                occurrence.id,
                error.message
            );
        }
    }
}

fn spawn_automation_schedule_occurrence_worker(
    app: AppHandle,
    state: &AppState,
    occurrence_id: Uuid,
) -> Result<bool, CommandError> {
    if state.orchestration.schedule_stop_requested() {
        return Ok(false);
    }
    if !state
        .orchestration
        .reserve_schedule_occurrence(occurrence_id)
    {
        return Ok(false);
    }
    let admission = match state.ensure_accepting_work() {
        Ok(admission) => admission,
        Err(error) => {
            state
                .orchestration
                .release_schedule_occurrence(occurrence_id);
            return Err(error);
        }
    };
    let mut port = ProductionOrchestrationPort::from_schedule_state(app, state);
    let db = state.db.clone();
    let runtime = state.orchestration.clone();
    tauri::async_runtime::spawn(async move {
        let _admission = admission;
        loop {
            if runtime.schedule_stop_requested() {
                break;
            }
            match execute_automation_schedule_occurrence(&db, occurrence_id, &mut port).await {
                Ok(AutomationScheduleExecution::Complete(_)) => break,
                Ok(AutomationScheduleExecution::Waiting(_)) => {
                    tokio::select! {
                        _ = tokio::time::sleep(Duration::from_millis(500)) => {}
                        _ = runtime.inner.schedule_stop_notify.notified() => {}
                    }
                }
                Err(error) => {
                    settle_schedule_worker_failure(&db, occurrence_id, &format!("{error:#}"));
                    log::error!(
                        "automation schedule occurrence {occurrence_id} failed to reconcile: {error:#}"
                    );
                    break;
                }
            }
        }
        runtime.release_schedule_occurrence(occurrence_id);
    });
    Ok(true)
}

fn settle_schedule_worker_failure(db: &riviu_core::db::Database, occurrence_id: Uuid, error: &str) {
    let Ok(Some(occurrence)) = db.get_automation_schedule_occurrence(occurrence_id) else {
        return;
    };
    if occurrence.state.is_terminal() {
        return;
    }
    let (outcome, code) = if occurrence.state == AutomationScheduleOccurrenceState::Queued {
        (ChildCampaignOutcome::Failed, "schedule_worker_failed")
    } else {
        (
            ChildCampaignOutcome::Uncertain,
            "schedule_reconcile_uncertain",
        )
    };
    if let Err(settle_error) =
        db.settle_automation_schedule_occurrence(occurrence_id, outcome, Some(code))
    {
        log::error!(
            "could not settle automation schedule occurrence {occurrence_id} after `{error}`: {settle_error:#}"
        );
    }
}

pub(crate) fn resume_orchestration_runs(app: AppHandle, state: &AppState) {
    let runs = match state.db.list_recoverable_orchestration_runs() {
        Ok(runs) => runs,
        Err(error) => {
            log::error!("could not list orchestration runs for recovery: {error:#}");
            return;
        }
    };
    for run in runs {
        if let Err(error) = spawn_orchestration_worker(app.clone(), state, run.id) {
            log::error!(
                "could not recover orchestration run {}: {}",
                run.id,
                error.message
            );
        }
    }
}

/// Stop every in-process orchestration before the app drains command admission and device
/// leases. The durable cancel marker is written before waiting for the per-run operation lock,
/// so a long Delay wakes immediately and a child dispatch already in progress cannot advance to
/// its next node behind shutdown's back.
pub(crate) async fn shutdown_orchestration_runs(app: AppHandle, state: &AppState) {
    for run_id in state.orchestration.active_run_ids() {
        let operation = state.orchestration.run_operation(run_id);
        let cancellation = state.orchestration.run_cancellation(run_id);
        match state.db.get_orchestration_run(run_id) {
            Ok(Some(detail)) if !detail.run.state.is_terminal() => {
                if let Err(error) = state.db.request_orchestration_cancel(run_id) {
                    log::error!(
                        "could not persist shutdown cancellation for orchestration {run_id}: {error:#}"
                    );
                }
                cancellation.notify_one();
            }
            Ok(_) => continue,
            Err(error) => {
                log::error!("could not inspect orchestration {run_id} during shutdown: {error:#}");
                cancellation.notify_one();
            }
        }

        let _operation = operation.lock().await;
        let mut port = ProductionOrchestrationPort::from_state(app.clone(), state, run_id);
        if let Err(error) = cancel_orchestration(&state.db, run_id, &mut port).await {
            log::error!("could not stop orchestration {run_id} during shutdown: {error:#}");
        }
    }
}

fn spawn_orchestration_worker(
    app: AppHandle,
    state: &AppState,
    run_id: Uuid,
) -> Result<bool, CommandError> {
    if !state.orchestration.reserve_run(run_id) {
        return Ok(false);
    }
    let admission = match state.ensure_accepting_work() {
        Ok(admission) => admission,
        Err(error) => {
            state.orchestration.release_run(run_id);
            return Err(error);
        }
    };
    let mut port = ProductionOrchestrationPort::from_state(app, state, run_id);
    let db = state.db.clone();
    let runtime = state.orchestration.clone();
    let operation = runtime.run_operation(run_id);
    tauri::async_runtime::spawn(async move {
        let _admission = admission;
        loop {
            let execution = {
                let _operation = operation.lock().await;
                execute_orchestration(&db, run_id, &mut port).await
            };
            match execution {
                Ok(OrchestrationExecution::Complete(_)) => break,
                Ok(OrchestrationExecution::Waiting { .. }) => {
                    tokio::time::sleep(Duration::from_millis(500)).await;
                }
                Err(error) => {
                    settle_worker_failure(&db, run_id, &error);
                    log::error!("orchestration run {run_id} failed to reconcile: {error:#}");
                    break;
                }
            }
        }
        runtime.release_run(run_id);
    });
    Ok(true)
}

fn settle_worker_failure(db: &riviu_core::db::Database, run_id: Uuid, error: &anyhow::Error) {
    let Ok(Some(mut detail)) = db.get_orchestration_run(run_id) else {
        return;
    };
    if detail.run.state == OrchestrationRunState::Queued {
        let Ok(Some(running)) = db.transition_orchestration_run(
            run_id,
            OrchestrationRunState::Queued,
            OrchestrationRunState::Running,
            None,
            None,
        ) else {
            return;
        };
        detail.run = running;
    }
    if detail.run.state != OrchestrationRunState::Running {
        return;
    }
    let ambiguous = detail.attempts.iter().any(|attempt| {
        matches!(
            attempt.state,
            OrchestrationAttemptState::Dispatching | OrchestrationAttemptState::WaitingChild
        )
    });
    let state = if ambiguous {
        OrchestrationRunState::Uncertain
    } else {
        OrchestrationRunState::Failed
    };
    let code = if ambiguous {
        "orchestration_reconcile_uncertain"
    } else {
        "orchestration_worker_failed"
    };
    let _ = db.transition_orchestration_run(
        run_id,
        OrchestrationRunState::Running,
        state,
        detail.run.current_node_id,
        Some(code),
    );
    log::warn!("orchestration run {run_id} settled after worker error `{error:#}`");
}

fn resolve_confirmed_targets(
    db: &riviu_core::db::Database,
    registry: &riviu_core::DeviceRegistry,
    target: &TargetRef,
    compiled: &CompiledOrchestrationV1,
) -> anyhow::Result<(
    ResolvedTargetSnapshot,
    BTreeMap<Uuid, ResolvedTargetSnapshot>,
)> {
    let resolve = |target: &TargetRef| resolve_target_snapshot(db, registry, target);
    let run_target = resolve(target)?;
    let mut resolved_refs: Vec<(TargetRef, ResolvedTargetSnapshot)> = Vec::new();
    let mut node_targets = BTreeMap::new();
    for node in &compiled.document.nodes {
        let Some(target_ref) = node.action.target_override() else {
            continue;
        };
        let snapshot = match resolved_refs
            .iter()
            .find(|(candidate, _)| candidate == target_ref)
        {
            Some((_, snapshot)) => snapshot.clone(),
            None => {
                let snapshot = resolve(target_ref)?;
                resolved_refs.push((target_ref.clone(), snapshot.clone()));
                snapshot
            }
        };
        node_targets.insert(node.id, snapshot);
    }
    Ok((run_target, node_targets))
}

fn resolve_target_snapshot(
    db: &riviu_core::db::Database,
    registry: &riviu_core::DeviceRegistry,
    target: &TargetRef,
) -> anyhow::Result<ResolvedTargetSnapshot> {
    let fleet_order = registry
        .list()
        .into_iter()
        .map(|device| device.udid)
        .collect::<Vec<_>>();
    let metadata = db.list_device_metas()?;
    let groups = db.list_groups()?;
    let snapshot = resolve_target(target, &fleet_order, &metadata, &groups)?;
    ensure!(
        !snapshot.included.is_empty(),
        "automation target has no connected eligible device"
    );
    Ok(snapshot)
}

fn status_as_dispatch(status: OrchestrationChildStatus) -> OrchestrationChildDispatch {
    match status {
        OrchestrationChildStatus::Running => OrchestrationChildDispatch::Started,
        OrchestrationChildStatus::Finished(outcome) => {
            OrchestrationChildDispatch::Finished(outcome)
        }
        OrchestrationChildStatus::MissingBeforeEffect => {
            OrchestrationChildDispatch::Finished(ChildCampaignOutcome::Failed)
        }
    }
}

fn interaction_status(detail: &InteractionCampaignDetail) -> OrchestrationChildStatus {
    if matches!(
        detail.summary.state,
        ThreadCampaignState::Queued | ThreadCampaignState::Running
    ) {
        return OrchestrationChildStatus::Running;
    }
    let action = detail.action_aggregate.map(|aggregate| match aggregate {
        InteractionRunAggregate::Done => ChildCampaignOutcome::Done,
        InteractionRunAggregate::Partial => ChildCampaignOutcome::Partial,
        InteractionRunAggregate::Failed => ChildCampaignOutcome::Failed,
        InteractionRunAggregate::Uncertain => ChildCampaignOutcome::Uncertain,
    });
    let messages_uncertain = detail
        .assignments
        .iter()
        .any(|assignment| assignment.state == ThreadMessageState::Uncertain);
    let outcome = if action == Some(ChildCampaignOutcome::Uncertain) || messages_uncertain {
        ChildCampaignOutcome::Uncertain
    } else {
        match detail.summary.state {
            ThreadCampaignState::Succeeded => action.unwrap_or(ChildCampaignOutcome::Done),
            ThreadCampaignState::Partial => ChildCampaignOutcome::Partial,
            ThreadCampaignState::Failed => {
                if detail.summary.succeeded_messages > 0 {
                    ChildCampaignOutcome::Partial
                } else {
                    ChildCampaignOutcome::Failed
                }
            }
            ThreadCampaignState::Cancelled => {
                if detail.summary.succeeded_messages > 0 {
                    ChildCampaignOutcome::Partial
                } else {
                    action.unwrap_or(ChildCampaignOutcome::Failed)
                }
            }
            ThreadCampaignState::Queued | ThreadCampaignState::Running => unreachable!(),
        }
    };
    OrchestrationChildStatus::Finished(outcome)
}

fn interaction_effect_may_have_started(detail: &InteractionCampaignDetail) -> bool {
    detail.assignments.iter().any(|assignment| {
        matches!(
            assignment.state,
            ThreadMessageState::Sending
                | ThreadMessageState::Succeeded
                | ThreadMessageState::Uncertain
        ) || assignment
            .actions
            .iter()
            .any(|action| action.state.effect_may_have_gone_out())
    })
}

fn publish_status(state: PublishCampaignState) -> OrchestrationChildStatus {
    match state {
        PublishCampaignState::Succeeded => {
            OrchestrationChildStatus::Finished(ChildCampaignOutcome::Done)
        }
        PublishCampaignState::FailedBeforeDispatch | PublishCampaignState::Missed => {
            OrchestrationChildStatus::Finished(ChildCampaignOutcome::Failed)
        }
        PublishCampaignState::Uncertain => {
            OrchestrationChildStatus::Finished(ChildCampaignOutcome::Uncertain)
        }
        PublishCampaignState::Cancelled => {
            OrchestrationChildStatus::Finished(ChildCampaignOutcome::Failed)
        }
        PublishCampaignState::Queued
        | PublishCampaignState::Scheduled
        | PublishCampaignState::Preparing
        | PublishCampaignState::Ready
        | PublishCampaignState::Transferring
        | PublishCampaignState::Imported
        | PublishCampaignState::Posting
        | PublishCampaignState::Verifying => OrchestrationChildStatus::Running,
    }
}

fn spawn_interaction_child(
    port: &ProductionOrchestrationPort,
    child_id: Uuid,
    request: riviu_core::ThreadCampaignRequest,
    plan: riviu_core::ThreadPlan,
) {
    if !port.runtime.reserve_interaction_child(child_id) {
        return;
    }
    spawn_reserved_interaction_child(port, child_id, request, plan);
}

fn spawn_reserved_interaction_child(
    port: &ProductionOrchestrationPort,
    child_id: Uuid,
    request: riviu_core::ThreadCampaignRequest,
    plan: riviu_core::ThreadPlan,
) {
    let campaign_id = child_id.to_string();
    let db = port.db.clone();
    let control = port.control.clone();
    let engine = port.nurture_engine.clone();
    let events = port.events.clone();
    let artifacts = port.interaction_artifacts.clone();
    let runtime = port.runtime.clone();
    let frames: Arc<dyn riviu_core::GenerationFrameSource> = Arc::new(port.streams.clone());
    tauri::async_runtime::spawn(async move {
        if let Err(error) = riviu_core::interaction_campaign::execute_thread_campaign(
            db.clone(),
            control,
            engine,
            events.clone(),
            campaign_id.clone(),
            request,
            plan,
            None,
            artifacts,
            frames,
        )
        .await
        {
            let detail = format!("{error:#}");
            let _ = db.fail_interaction_campaign_unless_settled(&campaign_id, &detail);
            events.emit(riviu_core::AppEvent::InteractionUpdated {
                campaign_id,
                revision: riviu_core::interaction_campaign::revision(),
            });
        }
        runtime.release_interaction_child(child_id);
    });
}

fn resume_interaction_child(
    port: &ProductionOrchestrationPort,
    child_id: Uuid,
) -> Result<(), String> {
    if !port.runtime.reserve_interaction_child(child_id) {
        return Ok(());
    }
    let recovered = match port
        .db
        .recover_owned_interaction_campaign(&child_id.to_string())
    {
        Ok(recovered) => recovered,
        Err(error) => {
            port.runtime.release_interaction_child(child_id);
            return Err(error.to_string());
        }
    };
    if !recovered {
        port.runtime.release_interaction_child(child_id);
        return Ok(());
    }
    let (request, plan) = port
        .db
        .get_interaction_campaign_request(&child_id.to_string())
        .map_err(|error| {
            port.runtime.release_interaction_child(child_id);
            error.to_string()
        })?
        .ok_or_else(|| {
            port.runtime.release_interaction_child(child_id);
            "running interaction child lost its persisted request".to_string()
        })?;
    spawn_reserved_interaction_child(port, child_id, request, plan);
    Ok(())
}

fn spawn_publish_child(port: &ProductionOrchestrationPort, child_id: Uuid) {
    if !port.runtime.reserve_publish_child(child_id) {
        return;
    }
    let campaign_id = child_id.to_string();
    let control = port.control.clone();
    let registry = port.registry.clone();
    let db = port.db.clone();
    let events = port.events.clone();
    let runtime = port.runtime.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(error) = crate::publish_commands::execute_scheduled_publish_campaign_inner(
            control,
            registry,
            db,
            events,
            campaign_id.clone(),
        )
        .await
        {
            log::error!("orchestration publish child {campaign_id} failed: {error:#}");
        }
        runtime.release_publish_child(child_id);
    });
}

fn load_profiles(
    state: &AppState,
    document: &OrchestrationDocumentV1,
) -> Result<Vec<AutomationDefinitionRecord>, CommandError> {
    let references = document
        .nodes
        .iter()
        .filter_map(|node| node.action.profile())
        .fold(
            BTreeMap::<(Uuid, u64), AutomationProfileRef>::new(),
            |mut refs, profile| {
                refs.entry((profile.definition_id, profile.revision))
                    .or_insert_with(|| profile.clone());
                refs
            },
        );
    references
        .into_values()
        .map(|profile| {
            state
                .db
                .get_automation_definition_record(profile.definition_id, profile.revision)
                .map_err(CommandError::from_service)?
                .ok_or_else(|| {
                    CommandError::code(
                        "ProfileRevisionMissing",
                        format!(
                            "profile {} revision {} does not exist",
                            profile.definition_id, profile.revision
                        ),
                    )
                })
        })
        .collect()
}

fn map_issues(issues: Vec<OrchestrationIssue>) -> Vec<CommandError> {
    issues.into_iter().map(issue_error).collect()
}

fn first_issue(issues: Vec<OrchestrationIssue>, fallback: &str) -> CommandError {
    issues
        .into_iter()
        .next()
        .map(issue_error)
        .unwrap_or_else(|| CommandError::code(fallback, "orchestration is invalid"))
}

fn issue_error(issue: OrchestrationIssue) -> CommandError {
    let mut error = CommandError::code(issue.code, issue.message);
    error.node_id = issue.node_id.map(|id| id.to_string().into_boxed_str());
    error
}

fn parse_uuid(value: &str, field: &str) -> Result<Uuid, CommandError> {
    Uuid::parse_str(value)
        .map_err(|_| CommandError::invalid_argument(format!("{field} is not a valid UUID")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn issues_keep_their_code_and_node_scope() {
        let node_id = Uuid::new_v4();
        let error = issue_error(OrchestrationIssue {
            code: "ProfileRevisionMissing".into(),
            node_id: Some(node_id),
            message: "missing".into(),
        });
        assert_eq!(error.code, "ProfileRevisionMissing");
        let expected = node_id.to_string();
        assert_eq!(error.node_id.as_deref(), Some(expected.as_str()));
    }
}
