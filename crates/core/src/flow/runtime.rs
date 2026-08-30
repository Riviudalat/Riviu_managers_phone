use std::collections::{BTreeSet, HashMap};
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use futures_util::future::join_all;
use parking_lot::Mutex;
use sha2::{Digest, Sha256};
use tokio::task::JoinHandle;
use uuid::Uuid;

use super::{
    FlowArtifactStore, FlowCancellation, FlowContextReleaseProof, FlowDeviceRunRecord,
    FlowDeviceRunState, FlowErrorRecord, FlowRevisionRecord, FlowRunRecord, FlowSelectionSnapshot,
};
use crate::db::Database;
use crate::events::{AppEvent, EventBus};
use crate::{
    ConnectionKind, DeviceControlPlane, DeviceRegistry, DeviceStatus, DeviceWorkOwner,
    FlowAttemptState, FlowNodeAttemptRecord, FlowTargetSelection, GenerationFrameSource,
    InteractionSessionKind, SideEffectClass,
};

/// How long shutdown waits for running flows to stop themselves before it stops waiting.
///
/// Long enough for a node inside `EVIDENCE_TIMEOUT` plus its release to finish on its own,
/// which is the outcome worth waiting for: a flow that ends itself releases the device lease,
/// and one that is abandoned does not. Bounded because the app must still close.
const SHUTDOWN_DEADLINE: Duration = Duration::from_secs(30);

#[derive(Clone)]
pub struct FlowRuntime {
    inner: Arc<FlowRuntimeInner>,
}

struct FlowRuntimeInner {
    database: Arc<Database>,
    events: EventBus,
    registry: DeviceRegistry,
    control: Arc<DeviceControlPlane>,
    frames: Arc<dyn GenerationFrameSource>,
    artifacts: FlowArtifactStore,
    cancellations: Mutex<HashMap<Uuid, FlowCancellation>>,
    tasks: Mutex<HashMap<Uuid, TrackedFlowTask>>,
    emitted_revisions: Mutex<HashMap<Uuid, u64>>,
    /// How many consecutive sweeps have failed to settle a run, by run id.
    ///
    /// **A refusal is said once, then counted.** The sweep runs once a minute, and a run
    /// whose ledger or plan will not validate is exactly the population it cannot fix — so
    /// without this it wrote the same warning every sixty seconds for the life of the
    /// process, and the operator learnt to scroll past the one line that mattered. The
    /// same "say it once" shape `IdleSweeper` and the Sheets outbox sweeper already use;
    /// the count is what turns the repeat into information ("still stuck after N passes")
    /// on the rare occasions it is worth re-saying. An entry disappears the moment the run
    /// settles or leaves the unsettled list, so a run that recovers starts fresh.
    stuck_runs: Mutex<HashMap<Uuid, u32>>,
    lifecycle: AtomicU8,
    recovery_active: AtomicBool,
    admission: tokio::sync::Mutex<()>,
    shutdown_lock: tokio::sync::Mutex<()>,
    #[cfg(test)]
    shutdown_deadline_ms: std::sync::atomic::AtomicU64,
}

struct TrackedFlowTask {
    run_ids: BTreeSet<Uuid>,
    handle: JoinHandle<anyhow::Result<()>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
enum FlowRuntimeLifecycle {
    Recovering = 0,
    Ready = 1,
    Stopping = 2,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum FlowRetryError {
    #[error("RetryNotAllowed: {reason}")]
    NotAllowed { reason: &'static str },
    #[error("RetryAlreadyRunning: Flow retry already has a live worker")]
    AlreadyRunning,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum FlowRuntimeError {
    #[error("Flow run {run_id} does not exist")]
    RunNotFound { run_id: Uuid },
    #[error("Flow attempt {attempt_id} does not exist")]
    AttemptNotFound { attempt_id: Uuid },
    #[error("Flow run {run_id} has no live cancellation owner")]
    CancellationOwnerMissing { run_id: Uuid },
}

/// One pass of the in-process orphaned-run sweep, counted — so the ticker can log
/// something an operator can act on, and so a test can assert a pass changed nothing.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct FlowSweepReport {
    /// Tracked tasks whose handle had finished without retiring itself (a panic
    /// unwound past the closure tail); removed from the map by this pass.
    pub dead_tasks_retired: usize,
    /// Runs settled to a terminal projection by this pass.
    pub runs_settled: usize,
    /// Runs this pass tried to settle and could not; logged, retried next tick.
    pub runs_unsettleable: usize,
    /// Device runs forced `Failed`/`WorkerLost` by this pass.
    pub devices_settled: usize,
    /// Orphaned attempts that had committed intent and dispatched nothing.
    pub attempts_failed_before_dispatch: usize,
    /// Orphaned attempts that may have touched the phone; never auto-retried.
    pub attempts_marked_uncertain: usize,
    /// Orphaned attempts walked out of `Interrupted`, the state's only legal exit.
    pub attempts_requeued_from_interrupted: usize,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum FlowSelectionError {
    #[error("selected devices are empty")]
    Empty,
    #[error("a selected device is unknown")]
    UnknownDevice,
    #[error("selected devices contain a duplicate")]
    Duplicate,
    #[error("no eligible device exists")]
    NoEligibleDevice,
}

enum RecoveryDeviceWork {
    Initial {
        device: Box<FlowDeviceRunRecord>,
        plan: Box<super::CompiledFlowPlanV2>,
        selection: FlowTargetSelection,
    },
    Resume {
        device_run_id: Uuid,
        udid: String,
        plan: Box<super::CompiledFlowPlanV2>,
        first_attempt_id: Uuid,
    },
    Reconcile(Box<crate::db::FlowAttemptExecutionContext>),
}

pub struct FlowRuntimeDeps {
    pub database: Arc<Database>,
    pub events: EventBus,
    pub registry: DeviceRegistry,
    pub control: Arc<DeviceControlPlane>,
    pub frames: Arc<dyn GenerationFrameSource>,
    pub artifacts: FlowArtifactStore,
}

impl FlowRuntime {
    pub fn new(deps: FlowRuntimeDeps) -> Self {
        Self {
            inner: Arc::new(FlowRuntimeInner {
                database: deps.database,
                events: deps.events,
                registry: deps.registry,
                control: deps.control,
                frames: deps.frames,
                artifacts: deps.artifacts,
                cancellations: Mutex::new(HashMap::new()),
                tasks: Mutex::new(HashMap::new()),
                emitted_revisions: Mutex::new(HashMap::new()),
                stuck_runs: Mutex::new(HashMap::new()),
                lifecycle: AtomicU8::new(FlowRuntimeLifecycle::Recovering as u8),
                recovery_active: AtomicBool::new(false),
                admission: tokio::sync::Mutex::new(()),
                shutdown_lock: tokio::sync::Mutex::new(()),
                #[cfg(test)]
                shutdown_deadline_ms: std::sync::atomic::AtomicU64::new(
                    u64::try_from(SHUTDOWN_DEADLINE.as_millis())
                        .expect("shutdown deadline fits u64"),
                ),
            }),
        }
    }

    pub async fn recover_startup(&self) -> anyhow::Result<()> {
        let admission = self.inner.admission.lock().await;
        self.require_lifecycle(FlowRuntimeLifecycle::Recovering)?;
        if self.inner.recovery_active.swap(true, Ordering::AcqRel) {
            anyhow::bail!("Flow startup recovery is already running");
        }
        let contexts = match self.inner.database.load_flow_recovery_contexts() {
            Ok(contexts) => contexts,
            Err(error) => {
                self.inner.recovery_active.store(false, Ordering::Release);
                return Err(error);
            }
        };
        let run_ids = contexts
            .iter()
            .map(|context| context.run.id)
            .collect::<BTreeSet<_>>();
        let runtime = self.clone();
        let (sender, receiver) = tokio::sync::oneshot::channel();
        let handle = tokio::spawn(async move {
            let result = runtime.recover_startup_contexts(contexts).await;
            runtime
                .inner
                .recovery_active
                .store(false, Ordering::Release);
            let caller_result = result
                .as_ref()
                .map(|_| ())
                .map_err(|error| format!("{error:#}"));
            let _ = sender.send(caller_result);
            result
        });
        let task_id = Uuid::new_v4();
        self.inner
            .tasks
            .lock()
            .insert(task_id, TrackedFlowTask { run_ids, handle });
        drop(admission);
        let caller_result = receiver
            .await
            .map_err(|_| anyhow::anyhow!("Flow startup recovery worker ended without a result"))?
            .map_err(anyhow::Error::msg);
        let completed = self.retire_tracked_task(task_id);
        if let Some(task) = completed {
            task.handle
                .await
                .context("join Flow startup recovery worker")??;
        }
        caller_result
    }

    /// Recover every interrupted run, and let a run that cannot be recovered fail alone.
    ///
    /// **One unrecoverable row used to stop the app from opening at all.** This loop was
    /// `self.recover_run_context(context).await?`, so the first context that failed aborted
    /// recovery, `recover_startup` returned `Err`, `AppState::bootstrap` failed, and the
    /// desktop came up on its startup-failure screen. Nothing in the UI could clear it —
    /// the row is in the database and the app that owns the database will not start — so
    /// the only remedy was editing SQLite by hand.
    ///
    /// A run that cannot be recovered is a failed run, which the schema can already
    /// express. Its devices are marked terminal with the reason, the rest of the fleet's
    /// runs recover as usual, and the app opens with the failure visible on the Flow page
    /// where an operator can read it.
    async fn recover_startup_contexts(
        &self,
        contexts: Vec<crate::db::FlowRecoveryRunContext>,
    ) -> anyhow::Result<()> {
        let mut unrecoverable = Vec::new();
        for context in contexts {
            let run_id = context.run.id;
            // Captured before the move: after a failure the context is gone, and these are
            // what say which device runs are still owed a terminal state.
            let pending = context
                .devices
                .iter()
                .filter(|device| !device.state.is_terminal())
                .map(|device| (device.id, device.udid.clone()))
                .collect::<Vec<_>>();
            let context_plan = context.plan.context_plan.clone();
            let Err(error) = self.recover_run_context(context).await else {
                continue;
            };
            let message = format!("{error:#}");
            tracing::error!(%run_id, error = %message, "Flow run could not be recovered");
            for (device_id, udid) in pending {
                // `Queued` belongs in this list because recovery itself puts devices there:
                // `reopen_device_for_resume` writes `queued`, and every resume validation
                // error returns before the Queued→Preflight transition. Without it, a failed
                // recovery left the device Queued, this catch was refused with StateConflict,
                // the projection stayed non-terminal — and the same run re-failed the same
                // way at every startup, forever.
                if let Err(mark) = self.inner.database.mark_device_terminal(
                    device_id,
                    &[
                        FlowDeviceRunState::Queued,
                        FlowDeviceRunState::Preflight,
                        FlowDeviceRunState::Running,
                    ],
                    FlowDeviceRunState::Failed,
                    Some(FlowErrorRecord {
                        code: "RecoveryFailed".to_string(),
                        message: format!("startup recovery could not restore this run: {message}"),
                        node_id: None,
                        field: None,
                        udid: Some(udid.clone()),
                        attempt_id: None,
                    }),
                    recovery_release_proof(&udid, &context_plan),
                ) {
                    // The database is the thing that is broken; saying so and carrying on is
                    // still better than refusing to open. The run stays where it is and the
                    // log names it.
                    tracing::error!(
                        %run_id,
                        %udid,
                        error = %format!("{mark:#}"),
                        "could not mark an unrecoverable Flow device run as failed"
                    );
                }
            }
            // The devices are terminal; the run has to say so too. Without this the run
            // aggregate stays `Queued` — a run that will never move, described as one that
            // has not started yet.
            if let Err(project) = self.inner.database.recompute_run_projection(run_id) {
                tracing::error!(
                    %run_id,
                    error = %format!("{project:#}"),
                    "could not settle the aggregate state of an unrecoverable Flow run"
                );
            }
            let _ = self.emit_run_updated(run_id);
            unrecoverable.push(run_id);
        }
        if !unrecoverable.is_empty() {
            tracing::warn!(
                count = unrecoverable.len(),
                "startup recovery failed {} Flow run(s) and continued",
                unrecoverable.len()
            );
        }
        self.inner
            .lifecycle
            .compare_exchange(
                FlowRuntimeLifecycle::Recovering as u8,
                FlowRuntimeLifecycle::Ready as u8,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .map_err(|_| anyhow::anyhow!("Flow runtime stopped during startup recovery"))?;
        Ok(())
    }

    fn lifecycle(&self) -> FlowRuntimeLifecycle {
        match self.inner.lifecycle.load(Ordering::Acquire) {
            value if value == FlowRuntimeLifecycle::Recovering as u8 => {
                FlowRuntimeLifecycle::Recovering
            }
            value if value == FlowRuntimeLifecycle::Ready as u8 => FlowRuntimeLifecycle::Ready,
            _ => FlowRuntimeLifecycle::Stopping,
        }
    }

    fn require_lifecycle(&self, expected: FlowRuntimeLifecycle) -> anyhow::Result<()> {
        let actual = self.lifecycle();
        if actual != expected {
            anyhow::bail!("Flow runtime is {actual:?}, expected {expected:?}");
        }
        Ok(())
    }

    async fn recover_run_context(
        &self,
        context: crate::db::FlowRecoveryRunContext,
    ) -> anyhow::Result<()> {
        let run_id = context.run.id;
        let cancellation = FlowCancellation::default();
        self.inner
            .cancellations
            .lock()
            .insert(run_id, cancellation.clone());
        if self.lifecycle() == FlowRuntimeLifecycle::Stopping {
            cancellation.cancel();
        }
        let result = self.recover_run_context_inner(context, cancellation).await;
        self.inner.cancellations.lock().remove(&run_id);
        result
    }

    async fn recover_run_context_inner(
        &self,
        context: crate::db::FlowRecoveryRunContext,
        cancellation: FlowCancellation,
    ) -> anyhow::Result<()> {
        let committed_artifact_attempts = context
            .artifacts
            .iter()
            .map(|artifact| artifact.attempt_id)
            .collect::<BTreeSet<_>>();
        let mut work = Vec::new();
        for device in context.devices {
            if device.state.is_terminal() {
                continue;
            }
            let mut attempts = context
                .attempts
                .iter()
                .filter(|attempt| attempt.device_run_id == device.id)
                .cloned()
                .collect::<Vec<_>>();
            attempts.sort_by_key(|attempt| {
                let node_index = context
                    .plan
                    .execution_order
                    .iter()
                    .position(|node_id| *node_id == attempt.node_id)
                    .unwrap_or(usize::MAX);
                (node_index, attempt.attempt_no, attempt.id)
            });

            if attempts.is_empty() {
                let device = if matches!(
                    device.state,
                    FlowDeviceRunState::Preflight | FlowDeviceRunState::Running
                ) {
                    self.inner
                        .database
                        .reopen_flow_device_for_recovery(device.id)?
                } else {
                    device
                };
                work.push(RecoveryDeviceWork::Initial {
                    device: Box::new(device),
                    plan: Box::new(context.plan.clone()),
                    selection: context.run.selection.requested.clone(),
                });
                continue;
            }

            if let Some(intent) = attempts
                .iter()
                .find(|attempt| attempt.state == FlowAttemptState::IntentCommitted)
            {
                let error = FlowErrorRecord {
                    code: "InterruptedBeforeDispatch".to_string(),
                    message: "desktop stopped after intent commit and before dispatch".to_string(),
                    node_id: Some(intent.node_id),
                    field: None,
                    udid: Some(device.udid.clone()),
                    attempt_id: Some(intent.id),
                };
                self.inner.database.transition_attempt(
                    intent.id,
                    FlowAttemptState::IntentCommitted,
                    FlowAttemptState::FailedBeforeDispatch,
                    crate::db::AttemptTransitionPatch {
                        error: Some(error.clone()),
                        ..Default::default()
                    },
                )?;
                self.inner.database.mark_device_terminal(
                    device.id,
                    &[FlowDeviceRunState::Preflight, FlowDeviceRunState::Running],
                    FlowDeviceRunState::Failed,
                    Some(error),
                    recovery_release_proof(&device.udid, &context.plan.context_plan),
                )?;
                continue;
            }

            if let Some(active) = attempts.iter().find(|attempt| {
                matches!(
                    attempt.state,
                    FlowAttemptState::EffectDispatched | FlowAttemptState::Verifying
                ) && attempt.side_effect_class != SideEffectClass::None
            }) {
                anyhow::ensure!(
                    !committed_artifact_attempts.contains(&active.id),
                    "active Flow attempt already owns a committed artifact"
                );
                let execution_context = self
                    .inner
                    .database
                    .get_flow_attempt_execution_context(active.id)?
                    .context("startup reconciliation attempt disappeared")?;
                work.push(RecoveryDeviceWork::Reconcile(Box::new(execution_context)));
                continue;
            }

            for attempt in &attempts {
                if matches!(
                    attempt.state,
                    FlowAttemptState::EffectDispatched | FlowAttemptState::Verifying
                ) {
                    self.inner.database.transition_attempt(
                        attempt.id,
                        attempt.state,
                        FlowAttemptState::Interrupted,
                        Default::default(),
                    )?;
                    self.inner.database.transition_attempt(
                        attempt.id,
                        FlowAttemptState::Interrupted,
                        FlowAttemptState::Queued,
                        Default::default(),
                    )?;
                } else if attempt.state == FlowAttemptState::Interrupted {
                    self.inner.database.transition_attempt(
                        attempt.id,
                        FlowAttemptState::Interrupted,
                        FlowAttemptState::Queued,
                        Default::default(),
                    )?;
                }
            }

            let refreshed = self
                .inner
                .database
                .get_flow_attempt_execution_context(attempts[0].id)?
                .context("normalized startup Flow attempt disappeared")?;
            if let Some(successor) =
                first_reclaimable_attempt(&refreshed.plan, &refreshed.device_attempts)
            {
                if matches!(
                    refreshed.device.state,
                    FlowDeviceRunState::Preflight | FlowDeviceRunState::Running
                ) {
                    self.inner
                        .database
                        .reopen_flow_device_for_recovery(refreshed.device.id)?;
                }
                work.push(RecoveryDeviceWork::Resume {
                    device_run_id: refreshed.device.id,
                    udid: refreshed.device.udid,
                    plan: Box::new(refreshed.plan),
                    first_attempt_id: successor.id,
                });
                continue;
            }

            // Path-aware, not blanket: `taken_path_succeeded` mirrors
            // `validate_device_success_projection`, so a branching device whose taken path
            // finished clean is recognised even though its untaken branch sits at `queued`.
            if taken_path_succeeded(&refreshed.plan, &refreshed.device_attempts) {
                self.inner.database.mark_device_terminal(
                    refreshed.device.id,
                    &[FlowDeviceRunState::Preflight, FlowDeviceRunState::Running],
                    FlowDeviceRunState::Succeeded,
                    None,
                    recovery_release_proof(&refreshed.device.udid, &refreshed.plan.context_plan),
                )?;
                continue;
            }

            let latest = latest_attempts(&refreshed.plan, &refreshed.device_attempts)?;

            if let Some(failed) = latest.iter().find(|attempt| {
                matches!(
                    attempt.state,
                    FlowAttemptState::FailedBeforeDispatch
                        | FlowAttemptState::FailedVerified
                        | FlowAttemptState::Uncertain
                        | FlowAttemptState::Cancelled
                )
            }) {
                let next = if failed.state == FlowAttemptState::Cancelled {
                    FlowDeviceRunState::Cancelled
                } else {
                    FlowDeviceRunState::Failed
                };
                let error = failed.error.clone().unwrap_or_else(|| FlowErrorRecord {
                    code: "RecoveredTerminalAttempt".to_string(),
                    message: "startup found a terminal attempt before queued successors"
                        .to_string(),
                    node_id: Some(failed.node_id),
                    field: None,
                    udid: Some(refreshed.device.udid.clone()),
                    attempt_id: Some(failed.id),
                });
                self.inner.database.mark_device_terminal(
                    refreshed.device.id,
                    &[FlowDeviceRunState::Preflight, FlowDeviceRunState::Running],
                    next,
                    Some(error),
                    recovery_release_proof(&refreshed.device.udid, &refreshed.plan.context_plan),
                )?;
                continue;
            }

            anyhow::bail!(
                "startup Flow device {} has no deterministic recovery target",
                refreshed.device.udid
            );
        }

        self.run_recovery_devices(context.run.id, work, cancellation)
            .await?;
        let recovered = self
            .inner
            .database
            .get_flow_run(context.run.id)?
            .context("recovered Flow run disappeared")?;
        anyhow::ensure!(
            recovered
                .device_runs
                .iter()
                .all(|device| device.state.is_terminal()),
            "startup recovery left nonterminal device work"
        );
        let projected = self
            .inner
            .database
            .recompute_run_projection(context.run.id)?;
        self.emit_run_updated(projected.id)?;
        Ok(())
    }

    pub async fn enqueue(
        &self,
        revision: FlowRevisionRecord,
        selection: FlowTargetSelection,
    ) -> anyhow::Result<FlowRunRecord> {
        let _admission = self.inner.admission.lock().await;
        self.require_lifecycle(FlowRuntimeLifecycle::Ready)?;
        let target_udids = resolve_targets(&self.inner.registry, &selection)?;
        let snapshot = FlowSelectionSnapshot {
            requested: selection.clone(),
            target_udids: target_udids.clone(),
        };
        let (run, devices) = self
            .inner
            .database
            .create_flow_run_with_devices(&revision, snapshot)?;
        self.emit_run_updated(run.id)?;

        let cancellation = FlowCancellation::default();
        self.inner
            .cancellations
            .lock()
            .insert(run.id, cancellation.clone());
        let runtime = self.clone();
        let run_id = run.id;
        let plan = revision.compiled_plan;
        let task_cancellation = cancellation.clone();
        let task_id = Uuid::new_v4();
        let (registered_sender, registered_receiver) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            registered_receiver
                .await
                .context("Flow run worker registration was abandoned")?;
            let result = runtime
                .run_flow(run_id, selection, devices, plan, task_cancellation)
                .await;
            runtime.inner.cancellations.lock().remove(&run_id);
            if let Err(error) = &result {
                tracing::error!(run_id = %run_id, error = %error, "Flow run worker failed");
            }
            let _ = runtime.retire_tracked_task(task_id);
            result
        });
        self.inner.tasks.lock().insert(
            task_id,
            TrackedFlowTask {
                run_ids: BTreeSet::from([run.id]),
                handle: task,
            },
        );
        let _ = registered_sender.send(());
        if self.lifecycle() != FlowRuntimeLifecycle::Ready {
            cancellation.cancel();
        }
        Ok(run)
    }

    pub fn cancel_run(&self, run_id: Uuid) -> anyhow::Result<()> {
        if let Some(cancellation) = self.inner.cancellations.lock().get(&run_id).cloned() {
            cancellation.cancel();
            return Ok(());
        }
        let run = self
            .inner
            .database
            .get_flow_run(run_id)?
            .ok_or(FlowRuntimeError::RunNotFound { run_id })?;
        if run.run.state.is_terminal() {
            return Ok(());
        }
        Err(FlowRuntimeError::CancellationOwnerMissing { run_id }.into())
    }

    pub async fn retry_attempt(&self, attempt_id: Uuid) -> anyhow::Result<FlowNodeAttemptRecord> {
        let admission = self.inner.admission.lock().await;
        self.require_lifecycle(FlowRuntimeLifecycle::Ready)?;
        let context = self
            .inner
            .database
            .get_flow_attempt_execution_context(attempt_id)?
            .ok_or(FlowRuntimeError::AttemptNotFound { attempt_id })?;
        if !context.run.state.is_terminal() || context.device.state != FlowDeviceRunState::Failed {
            return Err(FlowRetryError::NotAllowed {
                reason: "attempt device is not durably failed",
            }
            .into());
        }
        if !retry_can_enter_reconciliation(&context.attempt) {
            return Err(FlowRetryError::NotAllowed {
                reason: "attempt has no safe reconciliation path",
            }
            .into());
        }
        if self
            .inner
            .cancellations
            .lock()
            .contains_key(&context.run.id)
        {
            return Err(FlowRetryError::AlreadyRunning.into());
        }
        let run_id = context.run.id;
        let cancellation = FlowCancellation::default();
        self.inner
            .cancellations
            .lock()
            .insert(run_id, cancellation.clone());
        let task_cancellation = cancellation.clone();
        let runtime = self.clone();
        let (sender, receiver) = tokio::sync::oneshot::channel();
        let task_id = Uuid::new_v4();
        let (registered_sender, registered_receiver) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            registered_receiver
                .await
                .context("Flow retry worker registration was abandoned")?;
            let result = runtime
                .perform_retry_attempt(attempt_id, task_cancellation)
                .await;
            runtime.inner.cancellations.lock().remove(&run_id);
            let _ = sender.send(result);
            let _ = runtime.retire_tracked_task(task_id);
            Ok(())
        });
        self.inner.tasks.lock().insert(
            task_id,
            TrackedFlowTask {
                run_ids: BTreeSet::from([run_id]),
                handle: task,
            },
        );
        let _ = registered_sender.send(());
        if self.lifecycle() != FlowRuntimeLifecycle::Ready {
            cancellation.cancel();
        }
        drop(admission);
        receiver
            .await
            .map_err(|_| anyhow::anyhow!("Flow retry worker ended without a result"))?
    }

    async fn perform_retry_attempt(
        &self,
        attempt_id: Uuid,
        cancellation: FlowCancellation,
    ) -> anyhow::Result<FlowNodeAttemptRecord> {
        let mut context = self
            .inner
            .database
            .get_flow_attempt_execution_context(attempt_id)?
            .ok_or_else(|| anyhow::anyhow!("Flow attempt {attempt_id} does not exist"))?;
        if context.attempt.state == FlowAttemptState::FailedVerified {
            self.refresh_idempotent_retry_safety(&context, &cancellation)
                .await?;
            context = self
                .inner
                .database
                .get_flow_attempt_execution_context(attempt_id)?
                .context("freshly reconciled Flow attempt disappeared")?;
        }
        if !retry_is_allowed(&context.attempt, context.attempt.retry_allowed) {
            return Err(FlowRetryError::NotAllowed {
                reason: "fresh reconciliation did not prove non-delivery",
            }
            .into());
        }
        let node = context
            .plan
            .nodes
            .get(&context.attempt.node_id)
            .ok_or_else(|| anyhow::anyhow!("retry node is absent from its immutable plan"))?;
        let retry = self.inner.database.create_flow_attempt(
            context.device.id,
            node,
            context.attempt.side_effect_class,
            context
                .attempt
                .attempt_no
                .checked_add(1)
                .ok_or_else(|| anyhow::anyhow!("Flow retry attempt number overflow"))?,
        )?;
        self.emit_run_updated(context.run.id)?;

        let executor = super::FlowExecutor::new(super::FlowExecutorDeps {
            run_id: context.run.id,
            udid: context.device.udid.clone(),
            database: self.inner.database.clone(),
            control: self.inner.control.clone(),
            frames: self.inner.frames.clone(),
            artifacts: self.inner.artifacts.clone(),
            cancellation,
        });
        let result = executor
            .resume_device(context.device.id, context.plan, retry.id)
            .await;
        let projected = self
            .inner
            .database
            .recompute_run_projection(context.run.id)?;
        self.emit_run_updated(projected.id)?;
        if let Err(error) = result {
            anyhow::bail!("Flow retry failed [{}]: {error}", error.code());
        }
        let completed = self
            .inner
            .database
            .get_flow_attempt_execution_context(retry.id)?
            .ok_or_else(|| anyhow::anyhow!("completed Flow retry disappeared"))?;
        Ok(completed.attempt)
    }

    pub fn stop_all(&self) {
        self.inner
            .lifecycle
            .store(FlowRuntimeLifecycle::Stopping as u8, Ordering::Release);
        for cancellation in self.inner.cancellations.lock().values() {
            cancellation.cancel();
        }
    }

    pub async fn shutdown(&self) -> anyhow::Result<()> {
        let _shutdown_guard = self.inner.shutdown_lock.lock().await;
        let deadline = tokio::time::Instant::now() + self.shutdown_deadline();
        self.stop_all();
        let admission = self.inner.admission.lock();
        tokio::pin!(admission);
        let _admission = tokio::select! {
            guard = &mut admission => guard,
            _ = tokio::time::sleep_until(deadline) => {
                anyhow::bail!(
                    "ShutdownDeadlineExceeded: Flow admission exceeded the joined shutdown deadline"
                );
            }
        };
        let mut tasks = {
            let mut tasks = self.inner.tasks.lock();
            tasks.drain().collect::<Vec<_>>()
        };
        let mut first_error = None;
        let mut deadline_runs = BTreeSet::new();
        while let Some((_, mut task)) = tasks.pop() {
            tokio::select! {
                result = &mut task.handle => {
                    match result {
                        Ok(Ok(())) => {}
                        Ok(Err(error)) => {
                            first_error.get_or_insert_with(|| anyhow::anyhow!(
                                "Flow task {:?}: {error:#}",
                                task.run_ids
                            ));
                        }
                        Err(error) => {
                            first_error.get_or_insert_with(|| anyhow::anyhow!(
                                "Flow task {:?}: {error}",
                                task.run_ids
                            ));
                        }
                    }
                }
                _ = tokio::time::sleep_until(deadline) => {
                    deadline_runs.extend(task.run_ids.iter().copied());
                    task.handle.abort();
                    let _ = task.handle.await;
                    for (_, task) in &tasks {
                        deadline_runs.extend(task.run_ids.iter().copied());
                        task.handle.abort();
                    }
                    while let Some((_, task)) = tasks.pop() {
                        let _ = task.handle.await;
                    }
                    first_error.get_or_insert_with(|| anyhow::anyhow!(
                        "ShutdownDeadlineExceeded: Flow workers exceeded 30 seconds"
                    ));
                    break;
                }
            }
        }
        self.inner.cancellations.lock().clear();
        self.inner.emitted_revisions.lock().clear();
        for run_id in deadline_runs {
            if self
                .inner
                .database
                .get_flow_run(run_id)?
                .is_some_and(|detail| !detail.run.state.is_terminal())
            {
                self.inner.database.record_flow_runtime_error(
                    run_id,
                    FlowErrorRecord {
                        code: "ShutdownDeadlineExceeded".to_string(),
                        message: "Flow workers exceeded the joined shutdown deadline".to_string(),
                        node_id: None,
                        field: None,
                        udid: None,
                        attempt_id: None,
                    },
                )?;
                self.emit_run_updated(run_id)?;
            }
        }
        if let Some(error) = first_error {
            return Err(error);
        }
        Ok(())
    }

    fn shutdown_deadline(&self) -> Duration {
        #[cfg(test)]
        {
            Duration::from_millis(self.inner.shutdown_deadline_ms.load(Ordering::Acquire))
        }
        #[cfg(not(test))]
        {
            SHUTDOWN_DEADLINE
        }
    }

    #[cfg(test)]
    fn set_shutdown_deadline(&self, deadline: Duration) {
        self.inner.shutdown_deadline_ms.store(
            u64::try_from(deadline.as_millis()).expect("test shutdown deadline fits u64"),
            Ordering::Release,
        );
    }

    async fn run_flow(
        &self,
        run_id: Uuid,
        selection: FlowTargetSelection,
        devices: Vec<FlowDeviceRunRecord>,
        plan: super::CompiledFlowPlanV2,
        cancellation: FlowCancellation,
    ) -> anyhow::Result<()> {
        let children = devices.into_iter().map(|device| {
            let runtime = self.clone();
            let plan = plan.clone();
            let cancellation = cancellation.clone();
            let selection = selection.clone();
            async move {
                runtime
                    .run_device(run_id, selection, device, plan, cancellation)
                    .await
            }
        });

        let mut first_error = None;
        for result in join_all(children).await {
            match result {
                Ok(()) => {}
                Err(error) => {
                    first_error.get_or_insert(error);
                }
            }
        }
        let projected = self.inner.database.recompute_run_projection(run_id)?;
        self.emit_run_updated(projected.id)?;
        if let Some(error) = first_error {
            tracing::debug!(run_id = %run_id, error = %error, "Flow device completed with a persisted failure");
        }
        Ok(())
    }

    async fn run_recovery_devices(
        &self,
        run_id: Uuid,
        devices: Vec<RecoveryDeviceWork>,
        cancellation: FlowCancellation,
    ) -> anyhow::Result<()> {
        let children = devices.into_iter().map(|device| {
            let runtime = self.clone();
            let cancellation = cancellation.clone();
            async move {
                match device {
                    RecoveryDeviceWork::Initial {
                        device,
                        plan,
                        selection,
                    } => {
                        runtime
                            .run_device(run_id, selection, *device, *plan, cancellation)
                            .await
                    }
                    RecoveryDeviceWork::Resume {
                        device_run_id,
                        udid,
                        plan,
                        first_attempt_id,
                    } => {
                        let executor = super::FlowExecutor::new(super::FlowExecutorDeps {
                            run_id,
                            udid,
                            database: runtime.inner.database.clone(),
                            control: runtime.inner.control.clone(),
                            frames: runtime.inner.frames.clone(),
                            artifacts: runtime.inner.artifacts.clone(),
                            cancellation,
                        });
                        executor
                            .resume_device(device_run_id, *plan, first_attempt_id)
                            .await
                            .map_err(anyhow::Error::from)
                    }
                    RecoveryDeviceWork::Reconcile(context) => {
                        runtime
                            .reconcile_persisted_attempt(*context, cancellation)
                            .await
                    }
                }
            }
        });
        for result in join_all(children).await {
            match result {
                Ok(()) => {}
                Err(error) => {
                    tracing::debug!(run_id = %run_id, error = %error, "recovered Flow device failed");
                }
            }
        }
        Ok(())
    }

    async fn reconcile_persisted_attempt(
        &self,
        context: crate::db::FlowAttemptExecutionContext,
        cancellation: FlowCancellation,
    ) -> anyhow::Result<()> {
        let attempt = &context.attempt;
        let node = context
            .plan
            .nodes
            .get(&attempt.node_id)
            .context("reconciliation node is absent from immutable plan")?;
        let mut success_evidence = None;
        let mut artifact_recovered = false;
        let mut reconciliation_error = None;
        let mut retry_safe_evidence = None;
        let mut release_proof =
            recovery_release_proof(&context.device.udid, &context.plan.context_plan);

        match super::contracts(attempt.action_kind).3 {
            super::ReconciliationPolicy::ReadFrame => {
                reconciliation_error = Some((
                    "StaleGenerationEpoch",
                    "a numeric stream generation cannot be trusted across desktop restarts",
                ));
            }
            super::ReconciliationPolicy::ReadProcess => {
                let baseline: super::EvidenceBaseline = serde_json::from_value(
                    attempt
                        .evidence_baseline
                        .clone()
                        .context("process reconciliation has no persisted baseline")?,
                )?;
                let (bundle_id, pre_effect_pid) = match &baseline {
                    super::EvidenceBaseline::Process { bundle_id, pid } => {
                        (bundle_id.clone(), *pid)
                    }
                    _ => anyhow::bail!("process reconciliation baseline is not process-bound"),
                };
                let exclusive = self
                    .acquire_reconciliation_context(&context.device.udid, &cancellation)
                    .await?;
                let observed = self
                    .inner
                    .control
                    .inspect_app_process(&exclusive, &bundle_id)
                    .await;
                let proof = self.inner.control.close_exclusive_context(exclusive)?;
                merge_release_proof(&mut release_proof, context_release_proof(proof));
                match observed {
                    Ok(observed) if !observed.running && observed.pid.is_none() => {
                        let evidence = super::verify_process_absence(
                            node.postcondition
                                .as_ref()
                                .context("process reconciliation has no postcondition")?,
                            &baseline,
                            &crate::ProcessAbsenceProof {
                                bundle_id,
                                old_pid: pre_effect_pid,
                            },
                        )?;
                        success_evidence = Some(serde_json::to_value(evidence)?);
                    }
                    Ok(observed) if observed.pid == pre_effect_pid && pre_effect_pid.is_some() => {
                        let evidence = super::evaluate_process_state(
                            node.postcondition
                                .as_ref()
                                .context("process reconciliation has no postcondition")?,
                            &baseline,
                            &observed,
                        )?;
                        let retry_proof = serde_json::to_value(evidence)?;
                        let verification = serde_json::json!({
                            "kind": "processAbsent",
                            "matched": false,
                            "observedSha256": retry_proof["observedSha256"].clone(),
                            "measurement": {
                                "bundleId": bundle_id,
                                "running": true,
                                "oldPid": pre_effect_pid,
                            },
                        });
                        retry_safe_evidence = Some((verification, retry_proof));
                    }
                    Ok(_) => {
                        reconciliation_error = Some((
                            "RecoveryUncertain",
                            "process PID changed after the persisted terminate effect",
                        ));
                    }
                    Err(_) => {
                        reconciliation_error =
                            Some(("RecoveryUncertain", "process reconciliation read failed"));
                    }
                }
            }
            super::ReconciliationPolicy::ReadActiveApp => {
                let expected = match node.postcondition.as_ref() {
                    Some(super::EvidenceSpec::ActiveAppEquals { bundle_id }) => bundle_id,
                    _ => anyhow::bail!("active-app reconciliation has no exact postcondition"),
                };
                let exclusive = self
                    .acquire_reconciliation_context(&context.device.udid, &cancellation)
                    .await?;
                let observed = self.inner.control.read_active_app_bundle(&exclusive).await;
                let proof = self.inner.control.close_exclusive_context(exclusive)?;
                merge_release_proof(&mut release_proof, context_release_proof(proof));
                match observed {
                    Ok(observed) => {
                        let matched = observed == *expected;
                        let observed_sha256 = format!("{:x}", Sha256::digest(observed.as_bytes()));
                        let verification = serde_json::json!({
                            "kind": "activeAppEquals",
                            "matched": matched,
                            "observedSha256": observed_sha256,
                            "measurement": {"bundleId": observed},
                        });
                        if matched {
                            success_evidence = Some(verification);
                        } else {
                            retry_safe_evidence = Some((
                                verification,
                                serde_json::json!({
                                    "kind": "activeAppEquals",
                                    "matched": false,
                                    "observedSha256": observed_sha256,
                                    "measurement": {
                                        "expectedBundleId": expected,
                                        "observedBundleId": observed,
                                    },
                                }),
                            ));
                        }
                    }
                    Err(_) => {
                        reconciliation_error =
                            Some(("RecoveryUncertain", "active-app reconciliation read failed"));
                    }
                }
            }
            super::ReconciliationPolicy::ReadText => {
                let (locator, expected) = match node.postcondition.as_ref() {
                    Some(super::EvidenceSpec::TextReadBackEquals { locator, value }) => {
                        (locator.clone(), value.clone())
                    }
                    _ => anyhow::bail!("text reconciliation has no exact read-back contract"),
                };
                let target_bundle = context
                    .plan
                    .context_plan
                    .initial_bundle_id
                    .as_deref()
                    .context("text reconciliation has no pinned target app")?;
                let exclusive = self
                    .acquire_reconciliation_context(&context.device.udid, &cancellation)
                    .await?;
                let mut device_context = super::FlowDeviceContext::Exclusive(exclusive);
                let capacity = device_context
                    .reserve_capacity(&self.inner.control)
                    .await
                    .map_err(|failure| anyhow::anyhow!(failure.error.to_string()))?;
                let read_result = async {
                    let active = device_context
                        .active_app_bundle(&self.inner.control)
                        .await?;
                    anyhow::ensure!(
                        active == target_bundle,
                        "text reconciliation target app is not foreground"
                    );
                    device_context
                        .upgrade_existing_session(
                            &self.inner.control,
                            target_bundle,
                            InteractionSessionKind::FreshText,
                        )
                        .await?;
                    let session = device_context.session(&self.inner.control)?;
                    anyhow::ensure!(
                        session.supports_accessibility_readback(),
                        "text reconciliation session has no accessibility read-back"
                    );
                    let observed = session.read_text(&locator, Duration::from_secs(5)).await?;
                    Ok::<_, anyhow::Error>(observed == expected)
                }
                .await;
                let proof = device_context.close(&self.inner.control).await?;
                drop(capacity);
                merge_release_proof(&mut release_proof, context_release_proof(proof));
                reconciliation_error = Some(match read_result {
                    Ok(true) => (
                        "StaleGenerationEpoch",
                        "text matched, but its numeric frame generation cannot be trusted across desktop restarts",
                    ),
                    Ok(false) => (
                        "RecoveryUncertain",
                        "text read-back differs, but prior delivery remains ambiguous",
                    ),
                    Err(_) => (
                        "RecoveryUncertain",
                        "text read-back could not prove the persisted effect",
                    ),
                });
            }
            super::ReconciliationPolicy::ReadArtifact => {
                let (label, format) = match &node.config {
                    super::CompiledActionConfig::Screenshot { label, format } => {
                        (label.clone(), format.clone())
                    }
                    _ => anyhow::bail!("artifact reconciliation is not a Screenshot"),
                };
                match self.inner.artifacts.inspect_attempt_image(
                    context.run.id,
                    context.device.id,
                    attempt.id,
                    &format,
                )? {
                    super::AttemptArtifactInspection::Valid(artifact) => {
                        if attempt.state == FlowAttemptState::EffectDispatched {
                            self.inner.database.transition_attempt(
                                attempt.id,
                                FlowAttemptState::EffectDispatched,
                                FlowAttemptState::Verifying,
                                Default::default(),
                            )?;
                        }
                        self.inner.database.publish_artifact_and_succeed(
                            attempt.id,
                            &super::FlowArtifactRecord {
                                id: artifact.id,
                                attempt_id: attempt.id,
                                relative_path: artifact
                                    .relative_path
                                    .to_str()
                                    .context("recovered artifact path is not UTF-8")?
                                    .to_string(),
                                label,
                                kind: artifact.kind,
                                size: artifact.size,
                                sha256: artifact.sha256,
                                created_at: chrono::Utc::now(),
                            },
                        )?;
                        artifact_recovered = true;
                    }
                    super::AttemptArtifactInspection::Absent => {
                        reconciliation_error = Some((
                            "RecoveryUncertain",
                            "no attempt-qualified artifact survived the persisted effect",
                        ));
                    }
                    super::AttemptArtifactInspection::AmbiguousOrInvalid => {
                        reconciliation_error = Some((
                            "RecoveryUncertain",
                            "attempt artifact is ambiguous or failed integrity validation",
                        ));
                    }
                }
            }
            _ => {
                reconciliation_error = Some((
                    "RecoveryUncertain",
                    "startup reconciler has no conclusive typed proof",
                ));
            }
        }

        if artifact_recovered {
            return self
                .continue_after_reconciled_success(
                    attempt.id,
                    &context,
                    cancellation,
                    release_proof,
                )
                .await;
        }

        if let Some((verification, retry_proof)) = retry_safe_evidence {
            if attempt.state == FlowAttemptState::EffectDispatched {
                self.inner.database.transition_attempt(
                    attempt.id,
                    FlowAttemptState::EffectDispatched,
                    FlowAttemptState::Verifying,
                    Default::default(),
                )?;
            }
            let error = FlowErrorRecord {
                code: "ReconciledDesiredStateAbsent".to_string(),
                message: "typed reconciliation proved the desired state is absent".to_string(),
                node_id: Some(attempt.node_id),
                field: None,
                udid: Some(context.device.udid.clone()),
                attempt_id: Some(attempt.id),
            };
            self.inner.database.transition_attempt(
                attempt.id,
                FlowAttemptState::Verifying,
                FlowAttemptState::FailedVerified,
                crate::db::AttemptTransitionPatch {
                    evidence_result: Some(verification),
                    error: Some(error.clone()),
                    ..Default::default()
                },
            )?;
            self.inner.database.mark_device_terminal(
                context.device.id,
                &[FlowDeviceRunState::Preflight, FlowDeviceRunState::Running],
                FlowDeviceRunState::Failed,
                Some(error),
                release_proof,
            )?;
            self.inner
                .database
                .record_retry_safe_reconciliation(attempt.id, retry_proof)?;
            return Ok(());
        }

        if let Some(evidence) = success_evidence {
            if attempt.state == FlowAttemptState::EffectDispatched {
                self.inner.database.transition_attempt(
                    attempt.id,
                    FlowAttemptState::EffectDispatched,
                    FlowAttemptState::Verifying,
                    Default::default(),
                )?;
            }
            self.inner.database.transition_attempt(
                attempt.id,
                FlowAttemptState::Verifying,
                FlowAttemptState::Succeeded,
                crate::db::AttemptTransitionPatch {
                    evidence_result: Some(evidence),
                    ..Default::default()
                },
            )?;
            return self
                .continue_after_reconciled_success(
                    attempt.id,
                    &context,
                    cancellation,
                    release_proof,
                )
                .await;
        }

        let (code, message) = reconciliation_error.context("missing reconciliation outcome")?;
        let error = FlowErrorRecord {
            code: code.to_string(),
            message: message.to_string(),
            node_id: Some(attempt.node_id),
            field: None,
            udid: Some(context.device.udid.clone()),
            attempt_id: Some(attempt.id),
        };
        self.inner.database.transition_attempt(
            attempt.id,
            attempt.state,
            FlowAttemptState::Uncertain,
            crate::db::AttemptTransitionPatch {
                error: Some(error.clone()),
                ..Default::default()
            },
        )?;
        self.inner.database.mark_device_terminal(
            context.device.id,
            &[FlowDeviceRunState::Preflight, FlowDeviceRunState::Running],
            FlowDeviceRunState::Failed,
            Some(error),
            release_proof,
        )?;
        Ok(())
    }

    async fn continue_after_reconciled_success(
        &self,
        attempt_id: Uuid,
        context: &crate::db::FlowAttemptExecutionContext,
        cancellation: FlowCancellation,
        release_proof: FlowContextReleaseProof,
    ) -> anyhow::Result<()> {
        let refreshed = self
            .inner
            .database
            .get_flow_attempt_execution_context(attempt_id)?
            .context("reconciled Flow attempt disappeared")?;
        if let Some(successor) =
            first_reclaimable_attempt(&refreshed.plan, &refreshed.device_attempts)
        {
            self.inner
                .database
                .reopen_flow_device_for_recovery(refreshed.device.id)?;
            let executor = super::FlowExecutor::new(super::FlowExecutorDeps {
                run_id: refreshed.run.id,
                udid: refreshed.device.udid,
                database: self.inner.database.clone(),
                control: self.inner.control.clone(),
                frames: self.inner.frames.clone(),
                artifacts: self.inner.artifacts.clone(),
                cancellation,
            });
            executor
                .resume_device(refreshed.device.id, refreshed.plan, successor.id)
                .await
                .map_err(anyhow::Error::from)?;
        } else {
            self.inner.database.mark_device_terminal(
                context.device.id,
                &[FlowDeviceRunState::Preflight, FlowDeviceRunState::Running],
                FlowDeviceRunState::Succeeded,
                None,
                release_proof,
            )?;
        }
        Ok(())
    }

    async fn acquire_reconciliation_context(
        &self,
        udid: &str,
        cancellation: &FlowCancellation,
    ) -> anyhow::Result<crate::DeviceExclusiveContext> {
        let acquire = self
            .inner
            .control
            .acquire_exclusive(udid, DeviceWorkOwner::Script);
        let cancelled = cancellation.cancelled();
        tokio::pin!(acquire);
        tokio::pin!(cancelled);
        tokio::select! {
            biased;
            _ = &mut cancelled => anyhow::bail!("Flow reconciliation was cancelled during acquire"),
            result = &mut acquire => result.map_err(anyhow::Error::from),
        }
    }

    async fn refresh_idempotent_retry_safety(
        &self,
        context: &crate::db::FlowAttemptExecutionContext,
        cancellation: &FlowCancellation,
    ) -> anyhow::Result<()> {
        let attempt = &context.attempt;
        let node = context
            .plan
            .nodes
            .get(&attempt.node_id)
            .context("retry reconciliation node is absent from immutable plan")?;
        let exclusive = self
            .acquire_reconciliation_context(&context.device.udid, cancellation)
            .await?;
        let evidence = match super::contracts(attempt.action_kind).3 {
            super::ReconciliationPolicy::ReadProcess => {
                let baseline: super::EvidenceBaseline = serde_json::from_value(
                    attempt
                        .evidence_baseline
                        .clone()
                        .context("retry process reconciliation has no baseline")?,
                )?;
                let (bundle_id, pre_effect_pid) = match &baseline {
                    super::EvidenceBaseline::Process { bundle_id, pid } => {
                        (bundle_id.clone(), *pid)
                    }
                    _ => anyhow::bail!("retry process reconciliation baseline is invalid"),
                };
                let observed = self
                    .inner
                    .control
                    .inspect_app_process(&exclusive, &bundle_id)
                    .await;
                let _ = self.inner.control.close_exclusive_context(exclusive)?;
                let observed = observed?;
                if !observed.running && observed.pid.is_none() {
                    return Err(FlowRetryError::NotAllowed {
                        reason: "desired process state is already present",
                    }
                    .into());
                }
                if observed.pid != pre_effect_pid || pre_effect_pid.is_none() {
                    return Err(FlowRetryError::NotAllowed {
                        reason: "process identity changed after failure",
                    }
                    .into());
                }
                serde_json::to_value(super::evaluate_process_state(
                    node.postcondition
                        .as_ref()
                        .context("retry process reconciliation has no postcondition")?,
                    &baseline,
                    &observed,
                )?)?
            }
            super::ReconciliationPolicy::ReadActiveApp => {
                let expected = match node.postcondition.as_ref() {
                    Some(super::EvidenceSpec::ActiveAppEquals { bundle_id }) => bundle_id,
                    _ => anyhow::bail!("retry active-app reconciliation has no target"),
                };
                let observed = self.inner.control.read_active_app_bundle(&exclusive).await;
                let _ = self.inner.control.close_exclusive_context(exclusive)?;
                let observed = observed?;
                if observed == *expected {
                    return Err(FlowRetryError::NotAllowed {
                        reason: "desired active app is already present",
                    }
                    .into());
                }
                serde_json::json!({
                    "kind": "activeAppEquals",
                    "matched": false,
                    "observedSha256": format!("{:x}", Sha256::digest(observed.as_bytes())),
                    "measurement": {
                        "expectedBundleId": expected,
                        "observedBundleId": observed,
                    },
                })
            }
            _ => {
                return Err(FlowRetryError::NotAllowed {
                    reason: "action has no idempotent read reconciler",
                }
                .into());
            }
        };
        if cancellation.is_cancelled() {
            anyhow::bail!("Flow retry was cancelled after reconciliation read");
        }
        self.inner
            .database
            .record_retry_safe_reconciliation(attempt.id, evidence)?;
        Ok(())
    }

    async fn run_device(
        &self,
        run_id: Uuid,
        selection: FlowTargetSelection,
        device: FlowDeviceRunRecord,
        plan: super::CompiledFlowPlanV2,
        cancellation: FlowCancellation,
    ) -> anyhow::Result<()> {
        if !device_is_eligible(self.inner.registry.get(&device.udid).as_ref()) {
            let error = FlowErrorRecord {
                code: "DeviceIneligible".to_string(),
                message: "device is no longer eligible for this Flow run".to_string(),
                node_id: None,
                field: None,
                udid: Some(device.udid.clone()),
                attempt_id: None,
            };
            let next = if matches!(selection, FlowTargetSelection::AllEligible) {
                FlowDeviceRunState::Skipped
            } else {
                FlowDeviceRunState::Failed
            };
            self.inner.database.mark_device_terminal(
                device.id,
                &[FlowDeviceRunState::Queued],
                next,
                Some(error),
                empty_release_proof(&device.udid),
            )?;
            self.emit_run_updated(run_id)?;
            return Ok(());
        }

        let executor = super::FlowExecutor::new(super::FlowExecutorDeps {
            run_id,
            udid: device.udid.clone(),
            database: self.inner.database.clone(),
            control: self.inner.control.clone(),
            frames: self.inner.frames.clone(),
            artifacts: self.inner.artifacts.clone(),
            cancellation,
        });
        let result = executor.run_device(device.id, plan).await;
        self.emit_run_updated(run_id)?;
        if let Err(error) = result {
            self.settle_device_after_failure(
                run_id,
                device.id,
                &device.udid,
                error.code(),
                &error.to_string(),
            )?;
        }
        Ok(())
    }

    /// Make sure a device the executor gave up on is actually terminal.
    ///
    /// `run_device` returning an error usually means it already wrote the failure itself through
    /// `finish_failed`, and then `debug!` is the right level. But it can also mean the terminal
    /// write is exactly what failed — `finish_failed` could not close the context, or the success
    /// transition was refused — and in that case the device is left non-terminal with no task
    /// behind it. Nothing else would notice: the worker completes normally, the run stays
    /// `Running` forever, and a restart hands it to recovery, which cannot classify a device that
    /// was never terminalized and marks it `RecoveryFailed`.
    ///
    /// So ask the database what actually happened rather than assuming. Split out of the caller
    /// because a safety net nobody can pull on is not a net: this is the only way a test can put a
    /// device in the stuck state and check that it comes out of it.
    fn settle_device_after_failure(
        &self,
        run_id: Uuid,
        device_run_id: Uuid,
        udid: &str,
        code: &str,
        message: &str,
    ) -> anyhow::Result<()> {
        let settled = self
            .inner
            .database
            .get_flow_run(run_id)
            .ok()
            .flatten()
            .and_then(|detail| {
                detail
                    .device_runs
                    .into_iter()
                    .find(|record| record.id == device_run_id)
            })
            .map(|record| record.state.is_terminal())
            .unwrap_or(false);
        if settled {
            tracing::debug!(run_id = %run_id, udid = %udid, code = %code, "Flow device terminalized with failure");
            return Ok(());
        }
        tracing::warn!(
            run_id = %run_id,
            udid = %udid,
            code = %code,
            "Flow device did not terminalize; forcing it failed so the run cannot hang"
        );
        let record = FlowErrorRecord {
            code: code.to_string(),
            message: format!("device did not terminalize: {message}"),
            node_id: None,
            field: None,
            udid: Some(udid.to_string()),
            attempt_id: None,
        };
        match self.inner.database.mark_device_terminal(
            device_run_id,
            &[
                FlowDeviceRunState::Queued,
                FlowDeviceRunState::Preflight,
                FlowDeviceRunState::Running,
            ],
            FlowDeviceRunState::Failed,
            Some(record),
            empty_release_proof(udid),
        ) {
            Ok(_) => self.emit_run_updated(run_id),
            Err(forced) => {
                // Nothing left to try, so say so at a level someone reads. A silent return here is
                // what turned this into "the run just sits there".
                tracing::error!(
                    run_id = %run_id,
                    udid = %udid,
                    error = %forced,
                    "Flow device is stuck non-terminal and could not be forced failed"
                );
                Ok(())
            }
        }
    }

    /// Settle runs the database calls live that no live in-process worker owns.
    ///
    /// The third leg of the settlement tripod, and the only one on a clock:
    /// `settle_device_after_failure` covers a worker whose own error path still runs,
    /// startup recovery covers a process that died — and nothing covered a worker task
    /// that vanished mid-run while the process lived (a panic unwinding past
    /// `retire_tracked_task`, an abort). Such a run read `running` forever, its stale
    /// cancellation entry made `retry_attempt` answer `AlreadyRunning`, and only an app
    /// restart cleared it.
    ///
    /// DB-only, deliberately: a janitor must never drive phones. Attempts settle through
    /// [`Database::transition_attempt`] — the same CAS, validators and ledger event every
    /// worker write uses — because `recompute_run_projection` re-validates every persisted
    /// attempt and the ledger's contiguity, so a raw UPDATE here would have to re-implement
    /// exactly the machinery it bypassed.
    ///
    /// The caller owns the clock (a ticker in the desktop layer, with its own kill switch);
    /// this method is one pass, callable directly by a test with no timer at all.
    pub async fn sweep_orphaned_runs(&self) -> anyhow::Result<FlowSweepReport> {
        let mut report = FlowSweepReport::default();
        if self.lifecycle() != FlowRuntimeLifecycle::Ready {
            return Ok(report);
        }
        // Belt for the window inside `recover_startup`'s worker between the lifecycle CAS
        // to Ready and `recovery_active.store(false)`: recovery may still be writing.
        if self.inner.recovery_active.load(Ordering::Acquire) {
            return Ok(report);
        }
        // Under `admission` there is no "row in the database, entry not yet in the map"
        // window: every worker-creating path writes the DB and inserts its task while
        // holding this lock, and the worker body cannot run before the insert (it waits on
        // the registration oneshot). Hold time is milliseconds of SQLite writes at a
        // once-a-minute cadence, sharing `shutdown`'s deadline budget.
        let _admission = self.inner.admission.lock().await;
        if self.lifecycle() != FlowRuntimeLifecycle::Ready {
            return Ok(report);
        }
        let (live_runs, dead_tasks) = {
            let tasks = self.inner.tasks.lock();
            let mut live = BTreeSet::new();
            let mut dead = Vec::new();
            for (task_id, task) in tasks.iter() {
                if task.handle.is_finished() {
                    // Finished but still tracked can only mean the worker unwound without
                    // reaching `retire_tracked_task` — its runs are nobody's now.
                    dead.push(*task_id);
                } else {
                    live.extend(task.run_ids.iter().copied());
                }
            }
            (live, dead)
        };
        for task_id in dead_tasks {
            let _ = self.retire_tracked_task(task_id);
            report.dead_tasks_retired += 1;
        }
        let candidates = self
            .inner
            .database
            .list_unsettled_flow_run_ids()?
            .into_iter()
            .filter(|run_id| !live_runs.contains(run_id))
            .collect::<Vec<_>>();
        // A run that is no longer a candidate — settled, resumed by a worker, or deleted —
        // stops being stuck, so its counter goes. Pruning here rather than on the settle
        // path covers every way a run can leave the list, including the ones this sweep did
        // not cause.
        {
            let mut stuck = self.inner.stuck_runs.lock();
            stuck.retain(|run_id, _| candidates.contains(run_id));
        }
        for run_id in candidates {
            // `stop_all` can flip the lifecycle while this loop runs. Every settled run
            // left a legal persisted state, so bailing between runs is safe — the next
            // tick, or the next process, finishes the job.
            if self.lifecycle() != FlowRuntimeLifecycle::Ready {
                break;
            }
            match self.settle_orphaned_run(run_id, &mut report) {
                Ok(()) => {
                    self.inner.stuck_runs.lock().remove(&run_id);
                }
                Err(error) => {
                    report.runs_unsettleable += 1;
                    let passes = {
                        let mut stuck = self.inner.stuck_runs.lock();
                        let passes = stuck.entry(run_id).or_insert(0);
                        *passes += 1;
                        *passes
                    };
                    // Said on the first pass, and then only when the number of passes is
                    // itself news. A minute-by-minute repeat of a refusal this code cannot
                    // fix buries everything else in the log.
                    if passes == 1 || passes % 60 == 0 {
                        tracing::warn!(
                            %run_id,
                            passes,
                            error = %format!("{error:#}"),
                            "orphaned Flow run could not be settled; it needs a person, not \
                             another pass"
                        );
                    }
                }
            }
        }
        Ok(report)
    }

    /// Attempts → device → projection → emit, for one run no worker owns.
    ///
    /// Attempts first, and the order is load-bearing twice over: `mark_device_terminal`
    /// refuses a device that still owns an active attempt, and the transition validators
    /// want the device still `Running` for anything leaving the dispatched family. The
    /// settlement map is startup recovery's, applied cold:
    ///
    /// * `IntentCommitted → FailedBeforeDispatch` — nothing was dispatched; the name is
    ///   the guarantee, and retry stays open.
    /// * `EffectDispatched`/`Verifying → Uncertain` — the phone may have been touched;
    ///   never re-dispatched, automatically or manually.
    /// * `Interrupted → Queued` — the state's only legal exit. The device then fails and
    ///   the run goes terminal, so the queued attempt is permanently inert: the recovery
    ///   loader only reads live runs.
    fn settle_orphaned_run(
        &self,
        run_id: Uuid,
        report: &mut FlowSweepReport,
    ) -> anyhow::Result<()> {
        let Some(detail) = self.inner.database.get_flow_run(run_id)? else {
            // A benign lost race: the run vanished between the candidate list and here.
            return Ok(());
        };
        if detail.run.state.is_terminal() {
            // Same: someone settled it first. Not counted — this pass changed nothing.
            return Ok(());
        }
        for device in &detail.device_runs {
            if device.state.is_terminal() {
                continue;
            }
            let mut non_requeued: Vec<Uuid> = Vec::new();
            for attempt in detail
                .attempts
                .iter()
                .filter(|attempt| attempt.device_run_id == device.id)
            {
                let (next, counter) = match attempt.state {
                    FlowAttemptState::IntentCommitted => (
                        FlowAttemptState::FailedBeforeDispatch,
                        &mut report.attempts_failed_before_dispatch,
                    ),
                    FlowAttemptState::EffectDispatched | FlowAttemptState::Verifying => (
                        FlowAttemptState::Uncertain,
                        &mut report.attempts_marked_uncertain,
                    ),
                    FlowAttemptState::Interrupted => (
                        FlowAttemptState::Queued,
                        &mut report.attempts_requeued_from_interrupted,
                    ),
                    // Queued owns no active context and does not block the device settle;
                    // terminal states are already where they belong.
                    _ => continue,
                };
                let error = (next != FlowAttemptState::Queued).then(|| FlowErrorRecord {
                    code: "WorkerLost".to_string(),
                    message: "the in-process Flow worker vanished mid-run; the sweep \
                              settled this attempt without touching the phone"
                        .to_string(),
                    node_id: Some(attempt.node_id),
                    field: None,
                    udid: Some(device.udid.clone()),
                    attempt_id: Some(attempt.id),
                });
                self.inner.database.transition_attempt(
                    attempt.id,
                    attempt.state,
                    next,
                    crate::db::AttemptTransitionPatch {
                        error,
                        ..Default::default()
                    },
                )?;
                if next != FlowAttemptState::Queued {
                    non_requeued.push(attempt.id);
                }
                *counter += 1;
            }
            // The device error points at the one attempt this sweep settled when there is
            // exactly one — the same attribution rule recovery uses — and at nothing when
            // the story is wider than a single attempt.
            let attempt_id = match non_requeued.as_slice() {
                [only] => Some(*only),
                _ => None,
            };
            self.inner.database.mark_device_terminal(
                device.id,
                &[
                    FlowDeviceRunState::Queued,
                    FlowDeviceRunState::Preflight,
                    FlowDeviceRunState::Running,
                ],
                FlowDeviceRunState::Failed,
                Some(FlowErrorRecord {
                    code: "WorkerLost".to_string(),
                    message: "the in-process Flow worker vanished mid-run; the sweep \
                              settled this device without touching the phone"
                        .to_string(),
                    node_id: None,
                    field: None,
                    udid: Some(device.udid.clone()),
                    attempt_id,
                }),
                empty_release_proof(&device.udid),
            )?;
            report.devices_settled += 1;
        }
        // The run has to say so too — matching startup recovery, not
        // `settle_device_after_failure`, whose missing recompute is one of the ways a run
        // used to stay `Running` with every device terminal. Only reached for a candidate
        // that was really non-terminal, so a healthy ledger never grows a tick-shaped event.
        self.inner.database.recompute_run_projection(run_id)?;
        // A vanished worker skipped its closure tail, so its cancellation entry leaks —
        // and a stale entry makes `retry_attempt` answer `AlreadyRunning` forever.
        self.inner.cancellations.lock().remove(&run_id);
        let _ = self.emit_run_updated(run_id);
        // No task will ever retire this run's emitted-revision entry; drop it here or it
        // leaks one map slot per swept run for the life of the process.
        self.inner.emitted_revisions.lock().remove(&run_id);
        report.runs_settled += 1;
        Ok(())
    }

    fn emit_run_updated(&self, run_id: Uuid) -> anyhow::Result<()> {
        let Some(detail) = self.inner.database.get_flow_run(run_id)? else {
            return Ok(());
        };
        let revision = detail.run.event_revision;
        let mut emitted = self.inner.emitted_revisions.lock();
        let previous = emitted.entry(run_id).or_insert(0);
        if revision > *previous {
            *previous = revision;
            self.inner
                .events
                .emit(AppEvent::FlowRunUpdated { run_id, revision });
        }
        Ok(())
    }

    fn retire_tracked_task(&self, task_id: Uuid) -> Option<TrackedFlowTask> {
        let mut tasks = self.inner.tasks.lock();
        let retired = tasks.remove(&task_id);
        if let Some(task) = retired.as_ref() {
            let mut emitted = self.inner.emitted_revisions.lock();
            for run_id in &task.run_ids {
                if !tasks.values().any(|other| other.run_ids.contains(run_id)) {
                    emitted.remove(run_id);
                }
            }
        }
        retired
    }

    #[cfg(test)]
    async fn wait_terminal(&self, run_id: Uuid) -> anyhow::Result<super::FlowRunDetail> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(detail) = self.inner.database.get_flow_run(run_id)? {
                if detail.run.state.is_terminal() {
                    return Ok(detail);
                }
            }
            if tokio::time::Instant::now() >= deadline {
                anyhow::bail!("flow run did not become terminal before test deadline");
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    #[cfg(test)]
    async fn active_task_count(&self) -> usize {
        self.inner.tasks.lock().len()
    }

    #[cfg(test)]
    fn emitted_revision_count(&self) -> usize {
        self.inner.emitted_revisions.lock().len()
    }
}

fn first_reclaimable_attempt(
    plan: &super::CompiledFlowPlanV2,
    attempts: &[FlowNodeAttemptRecord],
) -> Option<FlowNodeAttemptRecord> {
    // Walk the taken path from Start, advancing through succeeded nodes and
    // following each IfVision node's recorded branch. The first node on that
    // path still Queued is the frontier to reclaim; a non-terminal or failed
    // node means there is nothing safely reclaimable ahead of it.
    let mut current = plan.entry_node();
    let mut guard = 0usize;
    while let Some(node_id) = current {
        guard += 1;
        if guard > plan.nodes.len() + 1 {
            return None;
        }
        let latest = attempts
            .iter()
            .filter(|attempt| attempt.node_id == node_id)
            .max_by_key(|attempt| attempt.attempt_no)?;
        match latest.state {
            FlowAttemptState::Queued => return Some(latest.clone()),
            FlowAttemptState::Succeeded => {
                current = plan.successor_on_path(node_id, latest.chosen_port.as_deref());
            }
            _ => return None,
        }
    }
    None
}

/// Whether every node **on the taken path** has a succeeded latest attempt.
///
/// The recovery-side twin of `validate_device_success_projection` in `db/flow_runs.rs`, built
/// from the same two authorities (`entry_node`, `successor_on_path`) so the two cannot answer
/// differently. The blanket "every node's latest attempt succeeded" question was retired from
/// two DB sites because a branching plan keeps its untaken branch at `queued` forever — and
/// recovery kept asking it here: a device that crashed after its taken path fully succeeded
/// matched neither the success arm nor the failed arm, hit "no deterministic recovery
/// target", and collateral-failed every pending sibling device.
///
/// A node on the path with no attempt, or a succeeded `IfVision` with no recorded branch,
/// ends the walk without claiming success — recovery must not guess a path.
fn taken_path_succeeded(
    plan: &super::CompiledFlowPlanV2,
    attempts: &[FlowNodeAttemptRecord],
) -> bool {
    let mut current = plan.entry_node();
    let mut guard = 0usize;
    while let Some(node_id) = current {
        guard += 1;
        if guard > plan.nodes.len() + 1 {
            return false;
        }
        let Some(latest) = attempts
            .iter()
            .filter(|attempt| attempt.node_id == node_id)
            .max_by_key(|attempt| attempt.attempt_no)
        else {
            return false;
        };
        if latest.state != FlowAttemptState::Succeeded {
            return false;
        }
        current = plan.successor_on_path(node_id, latest.chosen_port.as_deref());
    }
    true
}

fn latest_attempts(
    plan: &super::CompiledFlowPlanV2,
    attempts: &[FlowNodeAttemptRecord],
) -> anyhow::Result<Vec<FlowNodeAttemptRecord>> {
    plan.execution_order
        .iter()
        .map(|node_id| {
            attempts
                .iter()
                .filter(|attempt| attempt.node_id == *node_id)
                .max_by_key(|attempt| attempt.attempt_no)
                .cloned()
                .with_context(|| format!("Flow node {node_id} has no persisted attempt"))
        })
        .collect()
}

fn empty_release_proof(udid: &str) -> FlowContextReleaseProof {
    FlowContextReleaseProof {
        udid: udid.to_string(),
        owner: DeviceWorkOwner::Script,
        had_session: false,
        had_stream: false,
    }
}

fn recovery_release_proof(udid: &str, plan: &super::ContextPlan) -> FlowContextReleaseProof {
    FlowContextReleaseProof {
        udid: udid.to_string(),
        owner: DeviceWorkOwner::Script,
        had_session: plan.requires_ui_session,
        had_stream: plan.requires_stream,
    }
}

fn context_release_proof(proof: crate::ContextReleaseProof) -> FlowContextReleaseProof {
    FlowContextReleaseProof {
        udid: proof.udid,
        owner: proof.owner,
        had_session: proof.had_session,
        had_stream: proof.had_stream,
    }
}

fn merge_release_proof(target: &mut FlowContextReleaseProof, observed: FlowContextReleaseProof) {
    debug_assert_eq!(target.udid, observed.udid);
    target.had_session |= observed.had_session;
    target.had_stream |= observed.had_stream;
}

fn device_is_eligible(device: Option<&crate::DeviceInfo>) -> bool {
    device.is_some_and(|device| {
        matches!(
            device.connection,
            ConnectionKind::Usb | ConnectionKind::Mock
        ) && matches!(
            device.status,
            DeviceStatus::Connected | DeviceStatus::Ready | DeviceStatus::Busy
        )
    })
}

fn resolve_targets(
    registry: &DeviceRegistry,
    selection: &FlowTargetSelection,
) -> Result<Vec<String>, FlowSelectionError> {
    let devices = registry.list();
    let known = devices
        .iter()
        .map(|device| device.udid.clone())
        .collect::<BTreeSet<_>>();
    let mut targets = match selection {
        FlowTargetSelection::One { udid } => vec![udid.clone()],
        FlowTargetSelection::Selected { udids } if !udids.is_empty() => udids.clone(),
        FlowTargetSelection::Selected { .. } => return Err(FlowSelectionError::Empty),
        FlowTargetSelection::AllEligible => devices
            .into_iter()
            .filter(|device| {
                matches!(
                    device.connection,
                    ConnectionKind::Usb | ConnectionKind::Mock
                )
            })
            .filter(|device| {
                matches!(
                    device.status,
                    DeviceStatus::Connected | DeviceStatus::Ready | DeviceStatus::Busy
                )
            })
            .map(|device| device.udid)
            .collect(),
    };
    if !matches!(selection, FlowTargetSelection::AllEligible)
        && targets.iter().any(|udid| !known.contains(udid))
    {
        return Err(FlowSelectionError::UnknownDevice);
    }
    targets.sort();
    let before = targets.len();
    targets.dedup();
    if targets.len() != before {
        return Err(FlowSelectionError::Duplicate);
    }
    if targets.is_empty() {
        return Err(FlowSelectionError::NoEligibleDevice);
    }
    Ok(targets)
}

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RecoveryTarget {
    ReclaimIfPredecessorsSucceeded,
    Terminal(FlowAttemptState),
    Reconcile,
    AlreadyTerminal,
}

#[cfg(test)]
fn recovery_target(attempt: &FlowNodeAttemptRecord) -> RecoveryTarget {
    match attempt.state {
        FlowAttemptState::Queued | FlowAttemptState::Interrupted => {
            RecoveryTarget::ReclaimIfPredecessorsSucceeded
        }
        FlowAttemptState::IntentCommitted => {
            RecoveryTarget::Terminal(FlowAttemptState::FailedBeforeDispatch)
        }
        FlowAttemptState::EffectDispatched | FlowAttemptState::Verifying => {
            RecoveryTarget::Reconcile
        }
        FlowAttemptState::Succeeded
        | FlowAttemptState::FailedBeforeDispatch
        | FlowAttemptState::FailedVerified
        | FlowAttemptState::Uncertain
        | FlowAttemptState::Cancelled => RecoveryTarget::AlreadyTerminal,
    }
}

fn retry_is_allowed(attempt: &FlowNodeAttemptRecord, reconciler_proved_retry_safe: bool) -> bool {
    attempt.state == FlowAttemptState::FailedBeforeDispatch
        || (attempt.side_effect_class == SideEffectClass::IdempotentSet
            && attempt.state == FlowAttemptState::FailedVerified
            && reconciler_proved_retry_safe)
}

fn retry_can_enter_reconciliation(attempt: &FlowNodeAttemptRecord) -> bool {
    attempt.state == FlowAttemptState::FailedBeforeDispatch
        || (attempt.state == FlowAttemptState::FailedVerified
            && attempt.side_effect_class == SideEffectClass::IdempotentSet
            && matches!(
                super::contracts(attempt.action_kind).3,
                super::ReconciliationPolicy::ReadProcess
                    | super::ReconciliationPolicy::ReadActiveApp
            ))
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet, HashMap};
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use super::*;
    use crate::db::Database;
    use crate::{
        compiled_plan_sha256, ActionKind, ActiveTransport, AgentState, AgentStatus,
        AppProcessState, CompiledActionConfig, CompiledFlowNode, CompiledFlowPlanV2,
        CompiledTapTarget, ConnectionKind, ContextPlan, DeviceCapabilitySnapshot,
        DeviceControlPlane, DeviceDriver, DeviceInfo, DeviceStatus, DeviceWorkCoordinator,
        DeviceWorkOwner, EvidenceBaseline, EvidenceSpec, FlowAggregateState, FlowArtifactStore,
        FlowAttemptState, FlowCapabilitySnapshot, FlowDeviceRunState, FlowDocumentV2,
        FlowErrorRecord, FlowNodeAttemptRecord, FlowPreflightScope, FlowRevisionRecord,
        FlowSelectionSnapshot, FlowTargetSelection, Frame, FrameSource, FrameStream,
        GenerationFrame, GenerationFrameEvent, GenerationFrameSource, GenerationFrameStream,
        ImageCoordinateTarget, InstalledAgentIdentity, InstalledTargetIdentity,
        ProcessAbsenceProof, QualifiedElementLocator, QualifiedGeometry, ScreenOrientation,
        SideEffectClass, StreamBudgetManager, StreamHandoffProof, SwipeGesture, TapPoint,
        TileStreamState, UiSession, FLOW_SCHEMA_VERSION,
    };
    use async_trait::async_trait;
    use chrono::Utc;
    use image::RgbImage;
    use parking_lot::Mutex;
    use uuid::Uuid;

    #[test]
    fn selected_targets_are_exact_sorted_and_unique() {
        let registry = registry(&[
            device("iphone-b", ConnectionKind::Mock, DeviceStatus::Ready),
            device("iphone-a", ConnectionKind::Usb, DeviceStatus::Connected),
        ]);

        assert_eq!(
            resolve_targets(
                &registry,
                &FlowTargetSelection::Selected {
                    udids: vec!["iphone-b".into(), "iphone-a".into()],
                },
            )
            .expect("selected targets"),
            vec!["iphone-a".to_string(), "iphone-b".to_string()]
        );
        assert!(matches!(
            resolve_targets(&registry, &FlowTargetSelection::Selected { udids: vec![] },),
            Err(FlowSelectionError::Empty)
        ));
        assert!(matches!(
            resolve_targets(
                &registry,
                &FlowTargetSelection::Selected {
                    udids: vec!["iphone-a".into(), "iphone-a".into()],
                },
            ),
            Err(FlowSelectionError::Duplicate)
        ));
        assert!(matches!(
            resolve_targets(
                &registry,
                &FlowTargetSelection::One {
                    udid: "unknown".into(),
                },
            ),
            Err(FlowSelectionError::UnknownDevice)
        ));
    }

    #[test]
    fn all_eligible_freezes_only_connected_usb_or_mock_devices() {
        let registry = registry(&[
            device("ready-usb", ConnectionKind::Usb, DeviceStatus::Ready),
            device("busy-mock", ConnectionKind::Mock, DeviceStatus::Busy),
            device("wifi", ConnectionKind::Wifi, DeviceStatus::Ready),
            device(
                "disconnected",
                ConnectionKind::Usb,
                DeviceStatus::Disconnected,
            ),
        ]);

        assert_eq!(
            resolve_targets(&registry, &FlowTargetSelection::AllEligible)
                .expect("eligible targets"),
            vec!["busy-mock".to_string(), "ready-usb".to_string()]
        );
    }

    #[test]
    fn all_eligible_rejects_an_empty_frozen_snapshot() {
        let registry = registry(&[
            device("wifi", ConnectionKind::Wifi, DeviceStatus::Ready),
            device("offline", ConnectionKind::Usb, DeviceStatus::Disconnected),
        ]);

        assert!(matches!(
            resolve_targets(&registry, &FlowTargetSelection::AllEligible),
            Err(FlowSelectionError::NoEligibleDevice)
        ));
    }

    #[tokio::test]
    async fn missing_cancel_and_retry_targets_return_typed_runtime_errors() {
        let fixture = RuntimeFixture::new(&[], wait_plan(1)).await;
        let run_id = Uuid::new_v4();
        let run_error = fixture
            .runtime
            .cancel_run(run_id)
            .expect_err("missing run must be typed");
        assert_eq!(
            run_error.downcast_ref::<FlowRuntimeError>(),
            Some(&FlowRuntimeError::RunNotFound { run_id })
        );

        let attempt_id = Uuid::new_v4();
        let attempt_error = fixture
            .runtime
            .retry_attempt(attempt_id)
            .await
            .expect_err("missing attempt must be typed");
        assert_eq!(
            attempt_error.downcast_ref::<FlowRuntimeError>(),
            Some(&FlowRuntimeError::AttemptNotFound { attempt_id })
        );
        fixture.runtime.shutdown().await.expect("shutdown fixture");
    }

    #[test]
    fn recovery_classifies_every_attempt_state_without_redispatching_ambiguity() {
        let expected = [
            (
                FlowAttemptState::Queued,
                RecoveryTarget::ReclaimIfPredecessorsSucceeded,
            ),
            (
                FlowAttemptState::Interrupted,
                RecoveryTarget::ReclaimIfPredecessorsSucceeded,
            ),
            (
                FlowAttemptState::IntentCommitted,
                RecoveryTarget::Terminal(FlowAttemptState::FailedBeforeDispatch),
            ),
            (
                FlowAttemptState::EffectDispatched,
                RecoveryTarget::Reconcile,
            ),
            (FlowAttemptState::Verifying, RecoveryTarget::Reconcile),
            (FlowAttemptState::Succeeded, RecoveryTarget::AlreadyTerminal),
            (
                FlowAttemptState::FailedBeforeDispatch,
                RecoveryTarget::AlreadyTerminal,
            ),
            (
                FlowAttemptState::FailedVerified,
                RecoveryTarget::AlreadyTerminal,
            ),
            (FlowAttemptState::Uncertain, RecoveryTarget::AlreadyTerminal),
            (FlowAttemptState::Cancelled, RecoveryTarget::AlreadyTerminal),
        ];

        for (state, target) in expected {
            assert_eq!(
                recovery_target(&attempt(state, SideEffectClass::None)),
                target
            );
        }
    }

    #[test]
    fn retry_policy_never_redispatches_uncertain_ambiguous_ui_effects() {
        for kind in [ActionKind::Tap, ActionKind::Swipe, ActionKind::TypeText] {
            let mut record = attempt(FlowAttemptState::Uncertain, SideEffectClass::AmbiguousUi);
            record.action_kind = kind;
            assert!(!retry_is_allowed(&record, true), "{kind:?}");
        }

        assert!(retry_is_allowed(
            &attempt(
                FlowAttemptState::FailedBeforeDispatch,
                SideEffectClass::AmbiguousUi,
            ),
            false,
        ));
        assert!(retry_is_allowed(
            &attempt(
                FlowAttemptState::FailedVerified,
                SideEffectClass::IdempotentSet,
            ),
            true,
        ));
        assert!(!retry_is_allowed(
            &attempt(
                FlowAttemptState::FailedVerified,
                SideEffectClass::IdempotentSet,
            ),
            false,
        ));
    }

    #[tokio::test]
    async fn selected_devices_keep_independent_attempt_histories() {
        let fixture = RuntimeFixture::new(&["iphone-a", "iphone-b"], terminate_plan()).await;
        fixture.driver.fail_terminate_for("iphone-a");

        let run = fixture
            .runtime
            .enqueue(
                fixture.revision.clone(),
                FlowTargetSelection::Selected {
                    udids: vec!["iphone-a".into(), "iphone-b".into()],
                },
            )
            .await
            .expect("enqueue");
        let detail = fixture
            .runtime
            .wait_terminal(run.id)
            .await
            .expect("terminal run");

        assert_eq!(detail.run.state, FlowAggregateState::Partial);
        assert_eq!(detail.device_runs.len(), 2);
        assert_eq!(
            device_state(&detail, "iphone-a"),
            FlowDeviceRunState::Failed
        );
        assert_eq!(
            device_state(&detail, "iphone-b"),
            FlowDeviceRunState::Succeeded
        );
        assert_eq!(detail.attempts.len(), 2);
        assert!(detail
            .attempts
            .iter()
            .all(|attempt| attempt.device_run_id != Uuid::nil()));

        fixture.shutdown().await;
    }

    #[tokio::test(start_paused = true)]
    async fn run_update_events_are_post_commit_monotonic_and_end_at_the_persisted_revision() {
        let fixture = RuntimeFixture::new(&["iphone-a", "iphone-b"], single_wait_plan()).await;
        let mut events = fixture.events.subscribe();
        let run = fixture
            .runtime
            .enqueue(
                fixture.revision.clone(),
                FlowTargetSelection::Selected {
                    udids: vec!["iphone-b".into(), "iphone-a".into()],
                },
            )
            .await
            .expect("enqueue event run");
        let detail = fixture
            .runtime
            .wait_terminal(run.id)
            .await
            .expect("terminal event run");
        tokio::time::sleep(Duration::from_millis(20)).await;

        let mut revisions = Vec::new();
        while let Ok(event) = events.try_recv() {
            if let AppEvent::FlowRunUpdated { run_id, revision } = event {
                if run_id == run.id {
                    revisions.push(revision);
                }
            }
        }
        assert!(!revisions.is_empty());
        assert!(revisions.windows(2).all(|pair| pair[0] < pair[1]));
        assert_eq!(revisions.last().copied(), Some(detail.run.event_revision));
        fixture.shutdown().await;
    }

    // ------------------------------------------------------------- orphan sweep

    /// **A dispatched attempt whose worker vanished settles as what it is: uncertain.**
    ///
    /// The sweep is the third leg of the settlement tripod — the worker's own error path
    /// and startup recovery are the other two — and this is its worst case: the phone may
    /// have been tapped, so the attempt must land in the one state nothing retries.
    #[tokio::test]
    async fn sweep_settles_orphaned_dispatched_attempt_as_uncertain() {
        let fixture = RuntimeFixture::new(&["iphone-a"], single_wait_plan()).await;
        let (run_id, attempt_id) =
            fixture.seed_wait_recovery_run(FlowAttemptState::EffectDispatched);
        let mut events = fixture.events.subscribe();

        let report = fixture.runtime.sweep_orphaned_runs().await.expect("sweep");
        assert_eq!(report.runs_settled, 1);
        assert_eq!(report.devices_settled, 1);
        assert_eq!(report.attempts_marked_uncertain, 1);
        assert_eq!(report.attempts_failed_before_dispatch, 0);

        let detail = fixture
            .database
            .get_flow_run(run_id)
            .expect("load swept run")
            .expect("swept run");
        assert!(
            detail.run.state.is_terminal(),
            "the projection must be recomputed, not left running: {:?}",
            detail.run.state
        );
        let attempt = detail
            .attempts
            .iter()
            .find(|attempt| attempt.id == attempt_id)
            .expect("swept attempt");
        assert_eq!(attempt.state, FlowAttemptState::Uncertain);
        assert!(
            !attempt.retry_allowed,
            "a swept dispatched attempt must never be retryable"
        );
        let device = &detail.device_runs[0];
        assert_eq!(device.state, FlowDeviceRunState::Failed);
        let device_error = device.error.as_ref().expect("device error");
        assert_eq!(device_error.code, "WorkerLost");
        assert_eq!(
            device_error.attempt_id,
            Some(attempt_id),
            "exactly one settled attempt, so the device error names it"
        );

        let mut saw_update = false;
        while let Ok(event) = events.try_recv() {
            if matches!(event, AppEvent::FlowRunUpdated { run_id: id, .. } if id == run_id) {
                saw_update = true;
            }
        }
        assert!(saw_update, "the Flow page must hear about a swept run");
        assert_eq!(
            fixture.runtime.emitted_revision_count(),
            0,
            "no worker will ever retire a swept run's emitted-revision entry"
        );

        let second = fixture
            .runtime
            .sweep_orphaned_runs()
            .await
            .expect("second sweep");
        assert_eq!(
            second,
            FlowSweepReport::default(),
            "a second pass over a settled run changes nothing"
        );
        fixture.shutdown().await;
    }

    /// An intent nothing dispatched fails with the name that is the guarantee — and stays
    /// retryable, because nothing reached the phone.
    #[tokio::test]
    async fn sweep_turns_undispatched_intent_into_retryable_failure() {
        let fixture = RuntimeFixture::new(&["iphone-a"], single_wait_plan()).await;
        let (run_id, attempt_id) =
            fixture.seed_wait_recovery_run(FlowAttemptState::IntentCommitted);

        let report = fixture.runtime.sweep_orphaned_runs().await.expect("sweep");
        assert_eq!(report.runs_settled, 1);
        assert_eq!(report.attempts_failed_before_dispatch, 1);
        assert_eq!(report.attempts_marked_uncertain, 0);

        let detail = fixture
            .database
            .get_flow_run(run_id)
            .expect("load swept run")
            .expect("swept run");
        let attempt = detail
            .attempts
            .iter()
            .find(|attempt| attempt.id == attempt_id)
            .expect("swept attempt");
        assert_eq!(attempt.state, FlowAttemptState::FailedBeforeDispatch);
        assert!(
            attempt.retry_allowed,
            "nothing was dispatched, so the operator may run this again"
        );
        assert!(detail.run.state.is_terminal());
        fixture.shutdown().await;
    }

    /// `Interrupted` leaves by its only legal exit; the failed device and terminal run
    /// leave the requeued attempt permanently inert.
    #[tokio::test]
    async fn sweep_requeues_interrupted_attempt_and_fails_the_device() {
        let fixture = RuntimeFixture::new(&["iphone-a"], single_wait_plan()).await;
        let (run_id, attempt_id) = fixture.seed_wait_recovery_run(FlowAttemptState::Interrupted);

        let report = fixture.runtime.sweep_orphaned_runs().await.expect("sweep");
        assert_eq!(report.attempts_requeued_from_interrupted, 1);
        assert_eq!(report.runs_settled, 1);

        let detail = fixture
            .database
            .get_flow_run(run_id)
            .expect("load swept run")
            .expect("swept run");
        let attempt = detail
            .attempts
            .iter()
            .find(|attempt| attempt.id == attempt_id)
            .expect("swept attempt");
        assert_eq!(attempt.state, FlowAttemptState::Queued);
        let device = &detail.device_runs[0];
        assert_eq!(device.state, FlowDeviceRunState::Failed);
        assert_eq!(
            device.error.as_ref().expect("device error").attempt_id,
            None,
            "a requeued attempt is not the story the device error should point at"
        );
        assert!(detail.run.state.is_terminal());
        fixture.shutdown().await;
    }

    /// A run whose worker is alive is nobody's to sweep — and the same run settles the
    /// moment its worker dies without retiring itself.
    #[tokio::test]
    async fn sweep_leaves_a_live_worker_alone() {
        let fixture = RuntimeFixture::new(&["iphone-a"], single_wait_plan()).await;
        let (run_id, _) = fixture.seed_wait_recovery_run(FlowAttemptState::EffectDispatched);
        let (release, parked) = tokio::sync::oneshot::channel::<()>();
        let task_id = Uuid::new_v4();
        fixture.runtime.inner.tasks.lock().insert(
            task_id,
            TrackedFlowTask {
                run_ids: BTreeSet::from([run_id]),
                handle: tokio::spawn(async move {
                    let _ = parked.await;
                    Ok(())
                }),
            },
        );

        let report = fixture.runtime.sweep_orphaned_runs().await.expect("sweep");
        assert_eq!(
            report,
            FlowSweepReport::default(),
            "a tracked, unfinished worker owns its run"
        );
        let untouched = fixture
            .database
            .get_flow_run(run_id)
            .expect("load live run")
            .expect("live run");
        assert!(!untouched.run.state.is_terminal());

        release.send(()).expect("release the parked worker");
        for _ in 0..1_000 {
            let finished = fixture
                .runtime
                .inner
                .tasks
                .lock()
                .get(&task_id)
                .map(|task| task.handle.is_finished())
                .unwrap_or(true);
            if finished {
                break;
            }
            tokio::task::yield_now().await;
        }

        let second = fixture
            .runtime
            .sweep_orphaned_runs()
            .await
            .expect("second sweep");
        assert_eq!(second.dead_tasks_retired, 1);
        assert_eq!(second.runs_settled, 1);
        fixture.shutdown().await;
    }

    /// A finished-but-tracked handle is a worker that unwound past its own cleanup: the
    /// sweep retires it, settles its run, and clears the leaked cancellation entry that
    /// would otherwise make `retry_attempt` answer `AlreadyRunning` forever.
    #[tokio::test]
    async fn sweep_retires_a_dead_tracked_task_and_settles_its_run() {
        let fixture = RuntimeFixture::new(&["iphone-a"], single_wait_plan()).await;
        let (run_id, _) = fixture.seed_wait_recovery_run(FlowAttemptState::IntentCommitted);
        fixture
            .runtime
            .inner
            .cancellations
            .lock()
            .insert(run_id, FlowCancellation::default());
        let mut handle = tokio::spawn(async { Ok(()) });
        (&mut handle)
            .await
            .expect("join the already-finished worker")
            .expect("worker result");
        fixture.runtime.inner.tasks.lock().insert(
            Uuid::new_v4(),
            TrackedFlowTask {
                run_ids: BTreeSet::from([run_id]),
                handle,
            },
        );

        let report = fixture.runtime.sweep_orphaned_runs().await.expect("sweep");
        assert_eq!(report.dead_tasks_retired, 1);
        assert_eq!(report.runs_settled, 1);
        assert_eq!(fixture.runtime.active_task_count().await, 0);
        assert!(
            !fixture
                .runtime
                .inner
                .cancellations
                .lock()
                .contains_key(&run_id),
            "the leaked cancellation entry must go with the run"
        );
        fixture.shutdown().await;
    }

    /// Outside `Ready` the sweep is a silent no-op: recovery owns the runs before it, and
    /// a stopping runtime owns nothing any more.
    #[tokio::test]
    async fn sweep_is_a_noop_outside_ready() {
        let recovering = RuntimeFixture::new_recovering(&["iphone-a"], single_wait_plan());
        let (run_id, _) = recovering.seed_wait_recovery_run(FlowAttemptState::EffectDispatched);
        let report = recovering
            .runtime
            .sweep_orphaned_runs()
            .await
            .expect("sweep while recovering");
        assert_eq!(report, FlowSweepReport::default());
        assert!(
            !recovering
                .database
                .get_flow_run(run_id)
                .expect("load run")
                .expect("run")
                .run
                .state
                .is_terminal(),
            "a recovering runtime's runs are recovery's to settle"
        );

        let stopped = RuntimeFixture::new(&["iphone-a"], single_wait_plan()).await;
        let (stopped_run, _) = stopped.seed_wait_recovery_run(FlowAttemptState::EffectDispatched);
        stopped.runtime.shutdown().await.expect("shutdown");
        let report = stopped
            .runtime
            .sweep_orphaned_runs()
            .await
            .expect("sweep after shutdown");
        assert_eq!(report, FlowSweepReport::default());
        assert!(
            !stopped
                .database
                .get_flow_run(stopped_run)
                .expect("load run")
                .expect("run")
                .run
                .state
                .is_terminal(),
            "a stopping runtime must not keep writing"
        );
    }

    /// **A run the sweep cannot fix is said once and then counted, not shouted every pass.**
    ///
    /// The sweep runs once a minute, and the runs it cannot settle are exactly the ones it
    /// will never settle — a persisted shape that fails revalidation needs a person. Warning
    /// on every pass buries the rest of the log within a day, which is why `IdleSweeper` and
    /// the Sheets outbox sweeper both say a standing refusal once. The counter is the part
    /// worth asserting: it must climb per pass (so a re-say can carry "still stuck after N")
    /// and it must disappear the moment the run stops being a candidate, or a run that
    /// recovers would carry a stale count forever.
    ///
    /// The poison is honest rather than injected — see `seed_unsettleable_run`: a run whose
    /// frozen selection names two devices while only one device run exists, which is what a
    /// process that died mid-creation leaves behind and what `recompute_run_projection`
    /// refuses by design.
    #[tokio::test]
    async fn an_unsettleable_run_is_warned_once_and_counted_thereafter() {
        let fixture = RuntimeFixture::new(&["iphone-a", "iphone-b"], single_wait_plan()).await;
        let run_id = fixture.seed_unsettleable_run();

        for pass in 1..=3u32 {
            let report = fixture.runtime.sweep_orphaned_runs().await.expect("sweep");
            assert_eq!(
                report.runs_unsettleable, 1,
                "pass {pass} must report the run it could not settle"
            );
            assert_eq!(
                report.runs_settled, 0,
                "pass {pass} settled a run it cannot settle"
            );
            assert_eq!(
                fixture
                    .runtime
                    .inner
                    .stuck_runs
                    .lock()
                    .get(&run_id)
                    .copied(),
                Some(pass),
                "the stuck count must climb once per pass, so a re-say can name it"
            );
        }

        // And the moment the run stops being a candidate, the entry goes. A worker picking
        // it up is the reachable way that happens without a public state mutator — and it is
        // the case that matters, because a run that gets rescued must not carry a stale
        // count into the pass after it.
        let (release, parked) = tokio::sync::oneshot::channel::<()>();
        fixture.runtime.inner.tasks.lock().insert(
            Uuid::new_v4(),
            TrackedFlowTask {
                run_ids: BTreeSet::from([run_id]),
                handle: tokio::spawn(async move {
                    let _ = parked.await;
                    Ok(())
                }),
            },
        );
        let report = fixture.runtime.sweep_orphaned_runs().await.expect("sweep");
        assert_eq!(
            report,
            FlowSweepReport::default(),
            "a run with a live worker is not the sweep's to settle or to count"
        );
        assert!(
            fixture.runtime.inner.stuck_runs.lock().is_empty(),
            "a run that left the unsettled list must leave the stuck map with it"
        );
        release.send(()).expect("release the parked worker");
        fixture.shutdown().await;
    }

    /// An empty, healthy runtime sweeps to nothing — and emits nothing, because a healthy
    /// ledger must not grow a tick-shaped event.
    #[tokio::test]
    async fn sweep_reports_clean_on_an_empty_ready_runtime() {
        let fixture = RuntimeFixture::new(&["iphone-a"], single_wait_plan()).await;
        let mut events = fixture.events.subscribe();
        let report = fixture.runtime.sweep_orphaned_runs().await.expect("sweep");
        assert_eq!(report, FlowSweepReport::default());
        assert!(
            events.try_recv().is_err(),
            "a clean pass must not touch the ledger or the event bus"
        );
        fixture.shutdown().await;
    }

    #[tokio::test]
    async fn one_device_progresses_without_holding_a_lease_while_another_waits() {
        let fixture = RuntimeFixture::new(&["iphone-a", "iphone-b"], terminate_plan()).await;
        let held = fixture
            .control
            .acquire_exclusive("iphone-b", DeviceWorkOwner::Nurture)
            .await
            .expect("hold iphone-b");

        let run = fixture
            .runtime
            .enqueue(
                fixture.revision.clone(),
                FlowTargetSelection::Selected {
                    udids: vec!["iphone-a".into(), "iphone-b".into()],
                },
            )
            .await
            .expect("enqueue");

        fixture
            .wait_device_state(run.id, "iphone-a", FlowDeviceRunState::Succeeded)
            .await;
        assert_eq!(
            fixture.work.current_owner("iphone-b"),
            Some(DeviceWorkOwner::Nurture)
        );
        assert_eq!(fixture.work.current_owner("iphone-a"), None);

        fixture
            .control
            .close_exclusive_context(held)
            .expect("release iphone-b");
        assert_eq!(
            fixture
                .runtime
                .wait_terminal(run.id)
                .await
                .expect("terminal run")
                .run
                .state,
            FlowAggregateState::Succeeded
        );
        fixture.shutdown().await;
    }

    #[tokio::test]
    async fn cancellation_while_waiting_for_a_device_lease_does_not_wait_for_the_holder() {
        let fixture = RuntimeFixture::new(&["iphone-a"], terminate_plan()).await;
        let held = fixture
            .control
            .acquire_exclusive("iphone-a", DeviceWorkOwner::Nurture)
            .await
            .expect("hold iphone-a");
        let run = fixture
            .runtime
            .enqueue(
                fixture.revision.clone(),
                FlowTargetSelection::One {
                    udid: "iphone-a".into(),
                },
            )
            .await
            .expect("enqueue");
        fixture
            .wait_device_state(run.id, "iphone-a", FlowDeviceRunState::Preflight)
            .await;

        fixture.runtime.cancel_run(run.id).expect("cancel run");
        let detail = fixture
            .runtime
            .wait_terminal(run.id)
            .await
            .expect("cancelled terminal run");
        assert_eq!(detail.run.state, FlowAggregateState::Cancelled);
        assert_eq!(
            device_state(&detail, "iphone-a"),
            FlowDeviceRunState::Cancelled
        );
        assert_eq!(
            fixture.work.current_owner("iphone-a"),
            Some(DeviceWorkOwner::Nurture)
        );

        fixture
            .control
            .close_exclusive_context(held)
            .expect("release iphone-a");
        fixture.shutdown().await;
    }

    #[tokio::test]
    async fn cancellation_interrupts_wait_and_shutdown_joins_the_worker() {
        let fixture = RuntimeFixture::new(&["iphone-a"], wait_plan(10_000)).await;
        let run = fixture
            .runtime
            .enqueue(
                fixture.revision.clone(),
                FlowTargetSelection::One {
                    udid: "iphone-a".into(),
                },
            )
            .await
            .expect("enqueue");
        fixture
            .wait_attempt_state(run.id, FlowAttemptState::EffectDispatched)
            .await;

        let started = Instant::now();
        fixture.runtime.shutdown().await.expect("runtime shutdown");
        assert!(started.elapsed() < Duration::from_secs(2));
        let detail = fixture
            .database
            .get_flow_run(run.id)
            .expect("load run")
            .expect("run");
        assert_eq!(detail.run.state, FlowAggregateState::Cancelled);
        assert!(detail
            .attempts
            .iter()
            .any(|attempt| attempt.state == FlowAttemptState::Cancelled));
        assert_eq!(fixture.runtime.active_task_count().await, 0);

        fixture
            .control
            .shutdown_cleanup()
            .await
            .expect("control shutdown");
    }

    #[tokio::test]
    async fn completed_run_reaps_task_and_revision_bookkeeping_before_shutdown() {
        let fixture = RuntimeFixture::new(&["iphone-a"], wait_plan(1)).await;
        let run = fixture
            .runtime
            .enqueue(
                fixture.revision.clone(),
                FlowTargetSelection::One {
                    udid: "iphone-a".into(),
                },
            )
            .await
            .expect("enqueue short Flow");
        let detail = fixture
            .runtime
            .wait_terminal(run.id)
            .await
            .expect("short Flow terminal");
        assert_eq!(detail.run.state, FlowAggregateState::Succeeded);

        fixture.wait_runtime_bookkeeping_empty().await;
        fixture.shutdown().await;
    }

    #[tokio::test(start_paused = true)]
    async fn shutdown_tracks_and_joins_a_retry_worker_waiting_for_device_ownership() {
        let fixture = RuntimeFixture::new(&["iphone-a"], terminate_wait_plan()).await;
        let (_run_id, _terminate_id, wait_id, _end_id) = fixture.seed_wait_retry_run();
        let held = fixture
            .control
            .acquire_exclusive("iphone-a", DeviceWorkOwner::Nurture)
            .await
            .expect("hold retry device");
        let runtime = fixture.runtime.clone();
        let retry = tokio::spawn(async move { runtime.retry_attempt(wait_id).await });
        let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
        while fixture.runtime.active_task_count().await == 0 {
            assert!(
                tokio::time::Instant::now() < deadline,
                "retry was not tracked"
            );
            tokio::time::sleep(Duration::from_millis(5)).await;
        }

        fixture.runtime.shutdown().await.expect("runtime shutdown");
        let retry_error = retry
            .await
            .expect("join retry caller")
            .expect_err("shutdown cancels retry acquire");
        assert!(retry_error.to_string().contains("Cancelled"));
        assert_eq!(fixture.runtime.active_task_count().await, 0);
        assert_eq!(
            fixture.work.current_owner("iphone-a"),
            Some(DeviceWorkOwner::Nurture)
        );
        fixture
            .control
            .close_exclusive_context(held)
            .expect("release retry device");
        fixture
            .control
            .shutdown_cleanup()
            .await
            .expect("control shutdown");
    }

    #[tokio::test]
    async fn shutdown_deadline_aborts_owned_futures_joins_them_and_records_the_run_error() {
        let fixture = RuntimeFixture::new(&["iphone-a"], terminate_plan()).await;
        fixture.driver.block_terminate_for("iphone-a");
        fixture
            .runtime
            .set_shutdown_deadline(Duration::from_millis(50));
        let run = fixture
            .runtime
            .enqueue(
                fixture.revision.clone(),
                FlowTargetSelection::One {
                    udid: "iphone-a".into(),
                },
            )
            .await
            .expect("enqueue blocked effect");
        fixture.driver.wait_terminate_started().await;

        let error = fixture
            .runtime
            .shutdown()
            .await
            .expect_err("blocked effect exceeds shutdown deadline");
        assert!(error.to_string().contains("ShutdownDeadlineExceeded"));
        assert_eq!(fixture.runtime.active_task_count().await, 0);
        let detail = fixture
            .database
            .get_flow_run(run.id)
            .expect("load deadline run")
            .expect("deadline run");
        assert_eq!(
            detail.run.error.as_ref().map(|error| error.code.as_str()),
            Some("ShutdownDeadlineExceeded")
        );
        assert!(detail
            .attempts
            .iter()
            .any(|attempt| attempt.state == FlowAttemptState::EffectDispatched));
        assert_eq!(fixture.work.current_owner("iphone-a"), None);
        fixture
            .control
            .shutdown_cleanup()
            .await
            .expect("control shutdown");
    }

    #[tokio::test]
    async fn shutdown_deadline_includes_waiting_for_admission() {
        let fixture = RuntimeFixture::new(&["iphone-a"], wait_plan(1)).await;
        fixture
            .runtime
            .set_shutdown_deadline(Duration::from_millis(50));
        let admission = fixture.runtime.inner.admission.lock().await;
        let runtime = fixture.runtime.clone();
        let started = Instant::now();
        let shutdown = tokio::spawn(async move { runtime.shutdown().await });

        let error = tokio::time::timeout(Duration::from_millis(250), shutdown)
            .await
            .expect("shutdown must include admission in its fixed bound")
            .expect("join shutdown")
            .expect_err("held admission must exhaust the shutdown deadline");

        assert!(started.elapsed() < Duration::from_millis(250));
        assert!(error.to_string().contains("ShutdownDeadlineExceeded"));
        drop(admission);
        fixture
            .control
            .shutdown_cleanup()
            .await
            .expect("control shutdown");
    }

    #[tokio::test]
    async fn shutdown_owns_and_aborts_startup_recovery_at_the_global_deadline() {
        let fixture = RuntimeFixture::new_recovering(&["iphone-a"], terminate_plan());
        fixture.seed_terminate_recovery_run();
        fixture.driver.block_process_inspection();
        fixture
            .runtime
            .set_shutdown_deadline(Duration::from_millis(50));
        let runtime = fixture.runtime.clone();
        let mut recovery = tokio::spawn(async move { runtime.recover_startup().await });
        fixture.driver.wait_process_inspection_started().await;

        let shutdown_error = fixture
            .runtime
            .shutdown()
            .await
            .expect_err("blocked startup recovery must exhaust the shutdown deadline");
        assert!(shutdown_error
            .to_string()
            .contains("ShutdownDeadlineExceeded"));
        let recovery_result = tokio::time::timeout(Duration::from_millis(250), &mut recovery).await;
        if recovery_result.is_err() {
            fixture.driver.release_process_inspection();
            let _ = recovery.await;
            panic!("shutdown left startup recovery running after its global deadline");
        }
        assert!(recovery_result
            .expect("checked recovery completion")
            .expect("join recovery caller")
            .is_err());
        assert_eq!(fixture.runtime.active_task_count().await, 0);
        fixture
            .control
            .shutdown_cleanup()
            .await
            .expect("control shutdown");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn all_eligible_device_that_changes_before_attempts_is_persisted_as_skipped() {
        let fixture = RuntimeFixture::new(&["iphone-a", "iphone-b"], wait_plan(1)).await;
        let run = fixture
            .runtime
            .enqueue(fixture.revision.clone(), FlowTargetSelection::AllEligible)
            .await
            .expect("enqueue");
        fixture.registry.set_status(
            "iphone-a",
            DeviceStatus::Disconnected,
            Some("fixture disconnect".into()),
        );

        let detail = fixture
            .runtime
            .wait_terminal(run.id)
            .await
            .expect("terminal run");
        assert_eq!(
            device_state(&detail, "iphone-a"),
            FlowDeviceRunState::Skipped
        );
        assert!(detail.attempts.iter().all(|attempt| attempt.device_run_id
            != detail
                .device_runs
                .iter()
                .find(|device| device.udid == "iphone-a")
                .expect("iphone-a device run")
                .id));
        assert_eq!(
            device_state(&detail, "iphone-b"),
            FlowDeviceRunState::Succeeded
        );
        fixture.shutdown().await;
    }

    #[tokio::test(start_paused = true)]
    async fn in_flight_device_effect_is_not_aborted_by_cancellation() {
        let fixture = RuntimeFixture::new(&["iphone-a"], terminate_plan()).await;
        fixture.driver.block_terminate_for("iphone-a");
        let run = fixture
            .runtime
            .enqueue(
                fixture.revision.clone(),
                FlowTargetSelection::One {
                    udid: "iphone-a".into(),
                },
            )
            .await
            .expect("enqueue");
        fixture.driver.wait_terminate_started().await;

        fixture.runtime.cancel_run(run.id).expect("cancel run");
        tokio::time::sleep(Duration::from_millis(25)).await;
        assert!(!fixture
            .database
            .get_flow_run(run.id)
            .expect("load run")
            .expect("run")
            .run
            .state
            .is_terminal());

        fixture.driver.release_terminate();
        let detail = fixture
            .runtime
            .wait_terminal(run.id)
            .await
            .expect("effect completion");
        assert_eq!(detail.run.state, FlowAggregateState::Succeeded);
        assert_eq!(detail.attempts[0].state, FlowAttemptState::Succeeded);
        fixture.shutdown().await;
    }

    #[tokio::test]
    async fn failed_before_dispatch_retry_skips_succeeded_predecessor_and_continues_successor() {
        let fixture = RuntimeFixture::new(&["iphone-a"], terminate_wait_plan()).await;
        let (run_id, terminate_id, wait_id, end_id) = fixture.seed_wait_retry_run();

        let retried = fixture
            .runtime
            .retry_attempt(wait_id)
            .await
            .expect("retry failed-before-dispatch Wait");
        assert_eq!(retried.attempt_no, 2);
        assert_eq!(retried.state, FlowAttemptState::Succeeded);

        let detail = fixture
            .database
            .get_flow_run(run_id)
            .expect("load retried run")
            .expect("retried run");
        assert_eq!(detail.run.state, FlowAggregateState::Succeeded);
        assert_eq!(
            detail
                .attempts
                .iter()
                .find(|attempt| attempt.id == terminate_id)
                .expect("terminate predecessor")
                .state,
            FlowAttemptState::Succeeded
        );
        assert_eq!(
            detail
                .attempts
                .iter()
                .find(|attempt| attempt.id == end_id)
                .expect("end successor")
                .state,
            FlowAttemptState::Succeeded
        );
        assert_eq!(fixture.driver.terminate_calls.load(Ordering::SeqCst), 0);
        fixture.wait_runtime_bookkeeping_empty().await;
        fixture.shutdown().await;
    }

    #[tokio::test]
    async fn ui_resume_checks_the_active_app_before_non_foreground_session_attach() {
        let fixture = RuntimeFixture::new(&["iphone-a"], launch_wait_plan()).await;
        fixture.driver.set_active_bundle("com.example.other");
        let wait_id = fixture.seed_launch_wait_retry_run();

        let error = fixture
            .runtime
            .retry_attempt(wait_id)
            .await
            .expect_err("foreground drift must stop UI resume");
        assert!(
            error.to_string().contains("ActiveAppMismatch"),
            "unexpected retry error: {error:#}"
        );
        assert!(fixture.driver.active_app_reads.load(Ordering::SeqCst) > 0);
        assert_eq!(fixture.driver.session_start_calls.load(Ordering::SeqCst), 0);
        assert_eq!(fixture.driver.launch_calls.load(Ordering::SeqCst), 0);
        fixture.shutdown().await;
    }

    #[tokio::test]
    async fn uncertain_tap_swipe_and_type_text_are_rejected_by_runtime_retry() {
        for kind in [ActionKind::Tap, ActionKind::Swipe, ActionKind::TypeText] {
            let fixture = RuntimeFixture::new(&["iphone-a"], ambiguous_plan(kind)).await;
            let attempt_id = fixture.seed_uncertain_run();

            let error = fixture
                .runtime
                .retry_attempt(attempt_id)
                .await
                .expect_err("uncertain ambiguous UI effect must not retry");
            assert!(error.to_string().contains("RetryNotAllowed"), "{kind:?}");
            let persisted = fixture
                .database
                .get_flow_attempt_execution_context(attempt_id)
                .expect("load uncertain attempt")
                .expect("uncertain attempt");
            assert_eq!(persisted.attempt.state, FlowAttemptState::Uncertain);
            assert_eq!(persisted.attempt.attempt_no, 1);
            fixture.shutdown().await;
        }
    }

    #[tokio::test]
    async fn startup_reclaims_every_safe_nonterminal_wait_state() {
        for state in [
            FlowAttemptState::Queued,
            FlowAttemptState::Interrupted,
            FlowAttemptState::EffectDispatched,
            FlowAttemptState::Verifying,
        ] {
            let fixture = RuntimeFixture::new_recovering(&["iphone-a"], single_wait_plan());
            let (run_id, attempt_id) = fixture.seed_wait_recovery_run(state);

            fixture
                .runtime
                .recover_startup()
                .await
                .expect("recover startup");
            let detail = fixture
                .runtime
                .wait_terminal(run_id)
                .await
                .expect("recovered terminal run");
            assert_eq!(detail.run.state, FlowAggregateState::Succeeded, "{state:?}");
            assert_eq!(
                detail
                    .attempts
                    .iter()
                    .find(|attempt| attempt.id == attempt_id)
                    .expect("recovered Wait attempt")
                    .state,
                FlowAttemptState::Succeeded,
                "{state:?}"
            );
            fixture.shutdown().await;
        }
    }

    #[tokio::test]
    async fn startup_turns_intent_without_dispatch_into_retryable_failure() {
        let fixture = RuntimeFixture::new_recovering(&["iphone-a"], single_wait_plan());
        let (run_id, attempt_id) =
            fixture.seed_wait_recovery_run(FlowAttemptState::IntentCommitted);

        fixture
            .runtime
            .recover_startup()
            .await
            .expect("recover startup");
        let detail = fixture
            .runtime
            .wait_terminal(run_id)
            .await
            .expect("recovered failed run");
        assert_eq!(detail.run.state, FlowAggregateState::Failed);
        let attempt = detail
            .attempts
            .iter()
            .find(|attempt| attempt.id == attempt_id)
            .expect("recovered intent");
        assert_eq!(attempt.state, FlowAttemptState::FailedBeforeDispatch);
        assert!(attempt.retry_allowed);
        fixture.shutdown().await;
    }

    #[tokio::test]
    async fn startup_generation_advance_marks_tap_uncertain_without_redispatch() {
        let frames = Arc::new(GenerationAdvanceFrames::default());
        let fixture = RuntimeFixture::new_recovering_with_frames(
            &["iphone-a"],
            ambiguous_plan(ActionKind::Tap),
            frames.clone(),
        );
        let (run_id, attempt_id) = fixture.seed_ambiguous_recovery_run();

        fixture
            .runtime
            .recover_startup()
            .await
            .expect("recover startup");
        let detail = fixture
            .runtime
            .wait_terminal(run_id)
            .await
            .expect("reconciled terminal run");
        assert_eq!(detail.run.state, FlowAggregateState::Failed);
        assert_eq!(
            detail
                .attempts
                .iter()
                .find(|attempt| attempt.id == attempt_id)
                .expect("reconciled Tap")
                .state,
            FlowAttemptState::Uncertain
        );
        assert_eq!(frames.generation_reads.load(Ordering::SeqCst), 0);
        fixture.shutdown().await;
    }

    #[tokio::test]
    async fn startup_never_accepts_a_reused_numeric_frame_generation() {
        let frames = Arc::new(ReusedGenerationFrames::new());
        let fixture = RuntimeFixture::new_recovering_with_frames(
            &["iphone-a"],
            ambiguous_plan(ActionKind::Tap),
            frames.clone(),
        );
        let (run_id, attempt_id) = fixture.seed_ambiguous_recovery_run();

        fixture
            .runtime
            .recover_startup()
            .await
            .expect("recover startup");
        let detail = fixture
            .runtime
            .wait_terminal(run_id)
            .await
            .expect("reconciled terminal run");
        assert_eq!(
            detail
                .attempts
                .iter()
                .find(|attempt| attempt.id == attempt_id)
                .expect("reconciled Tap")
                .state,
            FlowAttemptState::Uncertain
        );
        assert_eq!(frames.generation_reads.load(Ordering::SeqCst), 0);
        fixture.shutdown().await;
    }

    #[tokio::test]
    async fn startup_type_text_reads_exact_text_but_rejects_the_old_generation_epoch() {
        let fixture = RuntimeFixture::new_recovering(&["iphone-a"], type_text_plan());
        fixture.driver.set_active_bundle(TARGET);
        fixture.driver.set_session_text("fixture");
        let (run_id, attempt_id) = fixture.seed_ambiguous_recovery_run();

        fixture
            .runtime
            .recover_startup()
            .await
            .expect("recover startup");
        let detail = fixture
            .runtime
            .wait_terminal(run_id)
            .await
            .expect("terminal text recovery");
        let attempt = detail
            .attempts
            .iter()
            .find(|attempt| attempt.id == attempt_id)
            .expect("Type Text attempt");
        assert_eq!(attempt.state, FlowAttemptState::Uncertain);
        assert_eq!(fixture.driver.read_text_calls.load(Ordering::SeqCst), 1);
        assert_eq!(fixture.driver.type_text_calls.load(Ordering::SeqCst), 0);
        let proof = detail.device_runs[0]
            .release_proof
            .as_ref()
            .expect("recovery release proof");
        assert!(proof.had_session);
        assert!(proof.had_stream);
        fixture.shutdown().await;
    }

    #[tokio::test]
    async fn startup_screenshot_adopts_only_an_exact_attempt_qualified_artifact() {
        let fixture = RuntimeFixture::new_recovering(&["iphone-a"], screenshot_plan());
        let (run_id, attempt_id) = fixture.seed_ambiguous_recovery_run();
        let context = fixture
            .database
            .get_flow_attempt_execution_context(attempt_id)
            .expect("load Screenshot context")
            .expect("Screenshot context");
        let prepared = fixture
            .artifacts
            .prepare_image(
                run_id,
                context.device.id,
                attempt_id,
                "capture",
                "png",
                &encoded_png(),
            )
            .expect("prepare crash-surviving artifact");
        fixture
            .artifacts
            .publish_file(&prepared)
            .expect("publish file before simulated crash");

        fixture
            .runtime
            .recover_startup()
            .await
            .expect("recover Screenshot artifact");
        let detail = fixture
            .database
            .get_flow_run(run_id)
            .expect("load recovered Screenshot")
            .expect("recovered Screenshot");
        assert_eq!(detail.run.state, FlowAggregateState::Succeeded);
        assert_eq!(detail.attempts[0].state, FlowAttemptState::Succeeded);
        assert_eq!(detail.artifacts.len(), 1);
        assert_eq!(detail.artifacts[0].id, prepared.id);
        assert_eq!(detail.artifacts[0].sha256, prepared.sha256);
        fixture.shutdown().await;
    }

    #[tokio::test]
    async fn startup_screenshot_without_an_exact_artifact_is_uncertain() {
        let fixture = RuntimeFixture::new_recovering(&["iphone-a"], screenshot_plan());
        let (run_id, attempt_id) = fixture.seed_ambiguous_recovery_run();

        fixture
            .runtime
            .recover_startup()
            .await
            .expect("recover absent Screenshot artifact");
        let detail = fixture
            .database
            .get_flow_run(run_id)
            .expect("load absent Screenshot")
            .expect("absent Screenshot");
        assert_eq!(detail.run.state, FlowAggregateState::Failed);
        assert_eq!(
            detail
                .attempts
                .iter()
                .find(|attempt| attempt.id == attempt_id)
                .expect("Screenshot attempt")
                .state,
            FlowAttemptState::Uncertain
        );
        assert!(detail.artifacts.is_empty());
        fixture.shutdown().await;
    }

    #[tokio::test]
    async fn one_unrecoverable_run_fails_alone_instead_of_stopping_the_app() {
        // This loop was `self.recover_run_context(context).await?`, so the first row it
        // could not recover aborted recovery, `recover_startup` returned Err,
        // `AppState::bootstrap` failed, and the desktop came up on its startup-failure
        // screen. Nothing in the UI could clear that: the row is in the database and the
        // app that owns the database will not start. Editing SQLite by hand was the remedy.
        let fixture = RuntimeFixture::new_recovering(&["iphone-a", "iphone-b"], single_wait_plan());
        let mut seeded = Vec::new();
        for udid in ["iphone-a", "iphone-b"] {
            let (run, devices) = fixture
                .database
                .create_flow_run_with_devices(
                    &fixture.revision,
                    FlowSelectionSnapshot {
                        requested: FlowTargetSelection::One { udid: udid.into() },
                        target_udids: vec![udid.into()],
                    },
                )
                .expect("seed run");
            fixture
                .database
                .transition_flow_device_run(
                    devices[0].id,
                    FlowDeviceRunState::Queued,
                    FlowDeviceRunState::Preflight,
                    None,
                )
                .expect("seed preflight device");
            seeded.push(run.id);
        }

        let mut contexts = fixture
            .database
            .load_flow_recovery_contexts()
            .expect("load recovery contexts");
        contexts.sort_by_key(|context| seeded.iter().position(|id| *id == context.run.id));
        let broken_run = contexts[0].run.id;
        let healthy_run = contexts[1].run.id;
        // The bad row: an attempt that points at a `flow_node_attempts` id which is not in
        // the database. Recovery loads it to normalise the device and finds nothing there.
        let device_run_id = contexts[0].devices[0].id;
        let node_id = *fixture
            .revision
            .compiled_plan
            .execution_order
            .first()
            .expect("a node");
        contexts[0].attempts.push(FlowNodeAttemptRecord {
            id: Uuid::new_v4(),
            device_run_id,
            node_id,
            action_kind: ActionKind::Wait,
            attempt_no: 1,
            side_effect_class: SideEffectClass::None,
            state: FlowAttemptState::Queued,
            canonical_input: None,
            evidence_baseline: None,
            evidence_result: None,
            chosen_port: None,
            retry_allowed: true,
            error: None,
            started_at: None,
            updated_at: chrono::Utc::now(),
            finished_at: None,
        });

        fixture
            .runtime
            .recover_startup_contexts(contexts)
            .await
            .expect("one unrecoverable run must not abort startup recovery");

        // The healthy run recovered, which is what the abort used to prevent.
        let healthy = fixture
            .database
            .get_flow_run(healthy_run)
            .expect("load healthy run")
            .expect("healthy run");
        assert_eq!(healthy.run.state, FlowAggregateState::Succeeded);

        // And the bad one is a failed run with a reason, rather than a run that stopped the
        // app from opening.
        let broken = fixture
            .database
            .get_flow_run(broken_run)
            .expect("load broken run")
            .expect("broken run");
        assert_eq!(broken.run.state, FlowAggregateState::Failed);
        let failure = broken.device_runs[0]
            .error
            .as_ref()
            .expect("a reason on the failed device");
        assert_eq!(failure.code, "RecoveryFailed");
        assert!(failure.message.contains("disappeared"), "{failure:?}");

        fixture.shutdown().await;
    }

    /// **A failed recovery must terminalize a device it left `Queued`, not skip it.**
    ///
    /// The catch in `recover_startup_contexts` named `[Preflight, Running]` as the states a
    /// pending device could be in — but recovery itself reopens devices to `queued`, and
    /// every resume validation error returns before the Queued→Preflight transition. So a
    /// deterministic recovery failure left the device `Queued`, `mark_device_terminal` was
    /// refused with StateConflict, the projection stayed non-terminal, and the same run
    /// re-failed the same way at every startup, forever.
    #[tokio::test]
    async fn a_failed_recovery_terminalizes_a_device_it_left_queued() {
        let fixture = RuntimeFixture::new_recovering(&["iphone-a"], single_wait_plan());
        let (run, devices) = fixture
            .database
            .create_flow_run_with_devices(
                &fixture.revision,
                FlowSelectionSnapshot {
                    requested: FlowTargetSelection::One {
                        udid: "iphone-a".into(),
                    },
                    target_udids: vec!["iphone-a".into()],
                },
            )
            .expect("seed run");
        // The device stays `Queued` on purpose: this is the state a failed resume leaves
        // behind, and the state the old catch could not settle.
        let mut contexts = fixture
            .database
            .load_flow_recovery_contexts()
            .expect("load recovery contexts");
        assert_eq!(contexts.len(), 1, "one queued run to recover");
        assert_eq!(contexts[0].devices[0].state, FlowDeviceRunState::Queued);
        let node_id = *fixture
            .revision
            .compiled_plan
            .execution_order
            .first()
            .expect("a node");
        // The same poisoned row the test above uses: recovery loads the attempt to
        // normalise the device and finds nothing behind it, so the run cannot recover.
        contexts[0].attempts.push(FlowNodeAttemptRecord {
            id: Uuid::new_v4(),
            device_run_id: devices[0].id,
            node_id,
            action_kind: ActionKind::Wait,
            attempt_no: 1,
            side_effect_class: SideEffectClass::None,
            state: FlowAttemptState::Queued,
            canonical_input: None,
            evidence_baseline: None,
            evidence_result: None,
            chosen_port: None,
            retry_allowed: true,
            error: None,
            started_at: None,
            updated_at: chrono::Utc::now(),
            finished_at: None,
        });

        fixture
            .runtime
            .recover_startup_contexts(contexts)
            .await
            .expect("an unrecoverable run must not abort startup recovery");

        let broken = fixture
            .database
            .get_flow_run(run.id)
            .expect("load run")
            .expect("run");
        assert_eq!(
            broken.device_runs[0].state,
            FlowDeviceRunState::Failed,
            "a queued device under a failed recovery must end terminal, not retry forever"
        );
        assert_eq!(
            broken.device_runs[0]
                .error
                .as_ref()
                .expect("a reason on the failed device")
                .code,
            "RecoveryFailed"
        );
        assert_eq!(
            broken.run.state,
            FlowAggregateState::Failed,
            "the projection must settle terminal so the next startup does not repeat this"
        );

        fixture.shutdown().await;
    }

    /// **Recovery recognises a branching device whose taken path finished clean.**
    ///
    /// The classifier used to ask the blanket question — every node's latest attempt
    /// Succeeded — which a branching plan can never answer yes: its untaken branch sits at
    /// `queued` forever. A device that crashed after its taken path fully succeeded matched
    /// neither the success arm nor the failed arm, recovery bailed with "no deterministic
    /// recovery target", and the whole run (sibling devices included) was collateral-failed.
    /// The same question was already made path-aware in two `flow_runs.rs` sites; this pins
    /// the third asker.
    #[tokio::test]
    async fn recovery_classifies_a_succeeded_branching_device_by_its_taken_path() {
        let vision_id = Uuid::new_v4();
        let taken_id = Uuid::new_v4();
        let untaken_id = Uuid::new_v4();
        let mut plan = compiled_plan(
            vec![
                CompiledFlowNode {
                    id: vision_id,
                    kind: ActionKind::IfVision,
                    config: CompiledActionConfig::IfVision {
                        template_png_base64: "Zml4dHVyZQ==".to_string(),
                        threshold: 0.9,
                        region: None,
                    },
                    postcondition: None,
                },
                CompiledFlowNode {
                    id: taken_id,
                    kind: ActionKind::Wait,
                    config: CompiledActionConfig::Wait { duration_ms: 1 },
                    postcondition: None,
                },
                CompiledFlowNode {
                    id: untaken_id,
                    kind: ActionKind::Wait,
                    config: CompiledActionConfig::Wait { duration_ms: 1 },
                    postcondition: None,
                },
            ],
            ContextPlan {
                requires_exclusive: false,
                requires_ui_session: false,
                requires_stream: false,
                requires_fresh_text_session: false,
                initial_bundle_id: None,
            },
            &[],
        );
        plan.successors = BTreeMap::from([
            (
                vision_id,
                BTreeMap::from([
                    ("matched".to_string(), taken_id),
                    ("notMatched".to_string(), untaken_id),
                ]),
            ),
            (taken_id, BTreeMap::new()),
            (untaken_id, BTreeMap::new()),
        ]);
        let fixture = RuntimeFixture::new_recovering(&["iphone-a"], plan);
        let run = fixture
            .database
            .create_flow_run(
                &fixture.revision,
                FlowSelectionSnapshot {
                    requested: FlowTargetSelection::One {
                        udid: "iphone-a".into(),
                    },
                    target_udids: vec!["iphone-a".into()],
                },
            )
            .expect("create recovery run");
        let device = fixture
            .database
            .create_flow_device_run(run.id, "iphone-a")
            .expect("create recovery device");
        fixture
            .database
            .transition_flow_device_run(
                device.id,
                FlowDeviceRunState::Queued,
                FlowDeviceRunState::Preflight,
                None,
            )
            .expect("recovery preflight");
        fixture
            .database
            .transition_flow_device_run(
                device.id,
                FlowDeviceRunState::Preflight,
                FlowDeviceRunState::Running,
                Some(FlowCapabilitySnapshot {
                    scope: FlowPreflightScope::TargetFree,
                    device: None,
                    agent_status: None,
                    capability_ids: BTreeSet::new(),
                }),
            )
            .expect("recovery running");
        let attempts = fixture
            .database
            .initialize_flow_device_attempts(device.id)
            .expect("initialize recovery attempts");
        // Seeded the way a crash leaves them: the vision node carries its branch and the
        // taken node succeeded, while the untaken branch keeps its initial `queued` attempt.
        for attempt in &attempts {
            if attempt.node_id == vision_id {
                fixture
                    .database
                    .seed_flow_attempt_state_for_test(
                        attempt.id,
                        FlowAttemptState::Succeeded,
                        Some("matched"),
                    )
                    .expect("seed vision attempt");
            } else if attempt.node_id == taken_id {
                fixture
                    .database
                    .seed_flow_attempt_state_for_test(attempt.id, FlowAttemptState::Succeeded, None)
                    .expect("seed taken attempt");
            }
        }

        fixture
            .runtime
            .recover_startup()
            .await
            .expect("recover startup");

        let detail = fixture
            .database
            .get_flow_run(run.id)
            .expect("load recovered run")
            .expect("recovered run");
        assert_eq!(
            detail.device_runs[0].state,
            FlowDeviceRunState::Succeeded,
            "a clean taken path is a success even though the untaken branch stays queued"
        );
        assert_eq!(detail.run.state, FlowAggregateState::Succeeded);

        fixture.shutdown().await;
    }

    #[tokio::test]
    async fn runtime_rejects_admission_until_startup_recovery_is_complete() {
        let fixture = RuntimeFixture::new_recovering(&["iphone-a"], single_wait_plan());
        let error = fixture
            .runtime
            .enqueue(
                fixture.revision.clone(),
                FlowTargetSelection::One {
                    udid: "iphone-a".into(),
                },
            )
            .await
            .expect_err("recovering runtime must reject admission");
        assert!(error.to_string().contains("Recovering"));

        fixture
            .runtime
            .recover_startup()
            .await
            .expect("complete startup recovery");
        let run = fixture
            .runtime
            .enqueue(
                fixture.revision.clone(),
                FlowTargetSelection::One {
                    udid: "iphone-a".into(),
                },
            )
            .await
            .expect("ready runtime accepts work");
        assert_eq!(
            fixture
                .runtime
                .wait_terminal(run.id)
                .await
                .expect("terminal admitted run")
                .run
                .state,
            FlowAggregateState::Succeeded
        );
        fixture.shutdown().await;
    }

    #[tokio::test]
    async fn startup_recovers_preflight_and_running_devices_with_zero_attempts() {
        let fixture = RuntimeFixture::new_recovering(&["iphone-a", "iphone-b"], single_wait_plan());
        let (run, devices) = fixture
            .database
            .create_flow_run_with_devices(
                &fixture.revision,
                FlowSelectionSnapshot {
                    requested: FlowTargetSelection::Selected {
                        udids: vec!["iphone-a".into(), "iphone-b".into()],
                    },
                    target_udids: vec!["iphone-a".into(), "iphone-b".into()],
                },
            )
            .expect("seed zero-attempt run");
        fixture
            .database
            .transition_flow_device_run(
                devices[0].id,
                FlowDeviceRunState::Queued,
                FlowDeviceRunState::Preflight,
                None,
            )
            .expect("seed preflight device");
        fixture
            .database
            .transition_flow_device_run(
                devices[1].id,
                FlowDeviceRunState::Queued,
                FlowDeviceRunState::Preflight,
                None,
            )
            .expect("seed running preflight");
        fixture
            .database
            .transition_flow_device_run(
                devices[1].id,
                FlowDeviceRunState::Preflight,
                FlowDeviceRunState::Running,
                Some(target_free_snapshot()),
            )
            .expect("seed running device");

        fixture
            .runtime
            .recover_startup()
            .await
            .expect("recover zero-attempt devices");
        let detail = fixture
            .database
            .get_flow_run(run.id)
            .expect("load recovered run")
            .expect("recovered run");
        assert_eq!(detail.run.state, FlowAggregateState::Succeeded);
        assert!(detail
            .device_runs
            .iter()
            .all(|device| device.state == FlowDeviceRunState::Succeeded));
        assert_eq!(detail.attempts.len(), 2);
        fixture.shutdown().await;
    }

    #[tokio::test]
    async fn startup_terminalizes_a_device_after_all_attempts_were_already_succeeded() {
        let fixture = RuntimeFixture::new_recovering(&["iphone-a"], single_wait_plan());
        let (run, devices) = fixture
            .database
            .create_flow_run_with_devices(
                &fixture.revision,
                FlowSelectionSnapshot {
                    requested: FlowTargetSelection::One {
                        udid: "iphone-a".into(),
                    },
                    target_udids: vec!["iphone-a".into()],
                },
            )
            .expect("seed succeeded-gap run");
        fixture
            .database
            .transition_flow_device_run(
                devices[0].id,
                FlowDeviceRunState::Queued,
                FlowDeviceRunState::Preflight,
                None,
            )
            .expect("succeeded-gap preflight");
        fixture
            .database
            .transition_flow_device_run(
                devices[0].id,
                FlowDeviceRunState::Preflight,
                FlowDeviceRunState::Running,
                Some(target_free_snapshot()),
            )
            .expect("succeeded-gap running");
        let attempt = fixture
            .database
            .initialize_flow_device_attempts(devices[0].id)
            .expect("initialize succeeded-gap attempt")
            .remove(0);
        let node = fixture
            .revision
            .compiled_plan
            .nodes
            .get(&attempt.node_id)
            .expect("succeeded-gap node");
        fixture
            .database
            .transition_attempt(
                attempt.id,
                FlowAttemptState::Queued,
                FlowAttemptState::IntentCommitted,
                crate::db::AttemptTransitionPatch {
                    canonical_input: Some(
                        serde_json::to_value(&node.config).expect("succeeded-gap input"),
                    ),
                    evidence_baseline: Some(
                        serde_json::to_value(EvidenceBaseline::None)
                            .expect("succeeded-gap baseline"),
                    ),
                    ..Default::default()
                },
            )
            .expect("succeeded-gap intent");
        fixture
            .database
            .transition_attempt(
                attempt.id,
                FlowAttemptState::IntentCommitted,
                FlowAttemptState::EffectDispatched,
                Default::default(),
            )
            .expect("succeeded-gap dispatch");
        fixture
            .database
            .transition_attempt(
                attempt.id,
                FlowAttemptState::EffectDispatched,
                FlowAttemptState::Verifying,
                Default::default(),
            )
            .expect("succeeded-gap verifying");
        fixture
            .database
            .transition_attempt(
                attempt.id,
                FlowAttemptState::Verifying,
                FlowAttemptState::Succeeded,
                Default::default(),
            )
            .expect("succeeded-gap success");

        fixture
            .runtime
            .recover_startup()
            .await
            .expect("recover succeeded-gap device");
        let detail = fixture
            .database
            .get_flow_run(run.id)
            .expect("load succeeded-gap run")
            .expect("succeeded-gap run");
        assert_eq!(detail.run.state, FlowAggregateState::Succeeded);
        assert_eq!(detail.device_runs[0].state, FlowDeviceRunState::Succeeded);
        fixture.shutdown().await;
    }

    #[tokio::test]
    async fn startup_terminalizes_a_failed_attempt_without_running_queued_successors() {
        let fixture = RuntimeFixture::new_recovering(&["iphone-a"], wait_plan(1));
        let (run, devices) = fixture
            .database
            .create_flow_run_with_devices(
                &fixture.revision,
                FlowSelectionSnapshot {
                    requested: FlowTargetSelection::One {
                        udid: "iphone-a".into(),
                    },
                    target_udids: vec!["iphone-a".into()],
                },
            )
            .expect("seed failed-gap run");
        fixture
            .database
            .transition_flow_device_run(
                devices[0].id,
                FlowDeviceRunState::Queued,
                FlowDeviceRunState::Preflight,
                None,
            )
            .expect("failed-gap preflight");
        fixture
            .database
            .transition_flow_device_run(
                devices[0].id,
                FlowDeviceRunState::Preflight,
                FlowDeviceRunState::Running,
                Some(target_free_snapshot()),
            )
            .expect("failed-gap running");
        let attempts = fixture
            .database
            .initialize_flow_device_attempts(devices[0].id)
            .expect("initialize failed-gap attempts");
        let first = attempts
            .iter()
            .find(|attempt| attempt.action_kind == ActionKind::Start)
            .expect("failed-gap first attempt");
        let node = fixture
            .revision
            .compiled_plan
            .nodes
            .get(&first.node_id)
            .expect("failed-gap first node");
        fixture
            .database
            .transition_attempt(
                first.id,
                FlowAttemptState::Queued,
                FlowAttemptState::IntentCommitted,
                crate::db::AttemptTransitionPatch {
                    canonical_input: Some(
                        serde_json::to_value(&node.config).expect("failed-gap input"),
                    ),
                    evidence_baseline: Some(
                        serde_json::to_value(EvidenceBaseline::None).expect("failed-gap baseline"),
                    ),
                    ..Default::default()
                },
            )
            .expect("failed-gap intent");
        let failure = FlowErrorRecord {
            code: "FixtureNonDelivery".into(),
            message: "fixture failed before dispatch".into(),
            node_id: Some(first.node_id),
            field: None,
            udid: Some("iphone-a".into()),
            attempt_id: Some(first.id),
        };
        fixture
            .database
            .transition_attempt(
                first.id,
                FlowAttemptState::IntentCommitted,
                FlowAttemptState::FailedBeforeDispatch,
                crate::db::AttemptTransitionPatch {
                    error: Some(failure),
                    ..Default::default()
                },
            )
            .expect("failed-gap terminal attempt");

        fixture
            .runtime
            .recover_startup()
            .await
            .expect("recover failed-gap device");
        let detail = fixture
            .database
            .get_flow_run(run.id)
            .expect("load failed-gap run")
            .expect("failed-gap run");
        assert_eq!(detail.run.state, FlowAggregateState::Failed);
        assert_eq!(detail.device_runs[0].state, FlowDeviceRunState::Failed);
        assert_eq!(
            detail
                .attempts
                .iter()
                .filter(|attempt| attempt.state == FlowAttemptState::Queued)
                .count(),
            2
        );
        fixture.shutdown().await;
    }

    #[tokio::test]
    async fn startup_terminate_reconciles_pid_without_calling_kill_again() {
        for (observed_pid, expected_state, expected_run) in [
            (
                None,
                FlowAttemptState::Succeeded,
                FlowAggregateState::Succeeded,
            ),
            (
                Some(10),
                FlowAttemptState::FailedVerified,
                FlowAggregateState::Failed,
            ),
            (
                Some(99),
                FlowAttemptState::Uncertain,
                FlowAggregateState::Failed,
            ),
        ] {
            let fixture = RuntimeFixture::new_recovering(&["iphone-a"], terminate_plan());
            let (run_id, attempt_id) = fixture.seed_terminate_recovery_run();
            fixture.driver.set_process("iphone-a", TARGET, observed_pid);

            fixture
                .runtime
                .recover_startup()
                .await
                .expect("recover startup");
            let detail = fixture
                .runtime
                .wait_terminal(run_id)
                .await
                .expect("reconciled Terminate run");
            let attempt = detail
                .attempts
                .iter()
                .find(|attempt| attempt.id == attempt_id)
                .expect("reconciled Terminate");
            assert_eq!(attempt.state, expected_state, "{observed_pid:?}");
            assert_eq!(detail.run.state, expected_run, "{observed_pid:?}");
            assert_eq!(fixture.driver.terminate_calls.load(Ordering::SeqCst), 0);
            assert!(
                fixture
                    .driver
                    .process_inspection_calls
                    .load(Ordering::SeqCst)
                    > 0
            );
            if observed_pid == Some(10) {
                assert!(attempt.retry_allowed);
                assert_eq!(
                    attempt
                        .evidence_result
                        .as_ref()
                        .and_then(|evidence| evidence.get("kind"))
                        .and_then(serde_json::Value::as_str),
                    Some("processAbsent")
                );
            }
            fixture.shutdown().await;
        }
    }

    #[tokio::test]
    async fn retry_reconciles_idempotent_state_again_instead_of_trusting_stale_permission() {
        let fixture = RuntimeFixture::new_recovering(&["iphone-a"], terminate_plan());
        let (run_id, attempt_id) = fixture.seed_terminate_recovery_run();
        fixture.driver.set_process("iphone-a", TARGET, Some(10));
        fixture
            .runtime
            .recover_startup()
            .await
            .expect("recover retryable Terminate");
        let recovered = fixture
            .database
            .get_flow_run(run_id)
            .expect("load retryable run")
            .expect("retryable run");
        assert!(recovered.attempts[0].retry_allowed);

        fixture.driver.set_process("iphone-a", TARGET, None);
        let error = fixture
            .runtime
            .retry_attempt(attempt_id)
            .await
            .expect_err("stale retry permission must be re-read");
        assert!(error
            .to_string()
            .contains("desired process state is already present"));
        let detail = fixture
            .database
            .get_flow_run(run_id)
            .expect("load rejected retry")
            .expect("rejected retry");
        assert_eq!(detail.attempts.len(), 1);
        assert_eq!(fixture.driver.terminate_calls.load(Ordering::SeqCst), 0);
        assert!(
            fixture
                .driver
                .process_inspection_calls
                .load(Ordering::SeqCst)
                >= 2
        );
        fixture.shutdown().await;
    }

    #[tokio::test]
    async fn retry_repairs_a_crash_between_failed_device_and_retry_proof_commit() {
        let fixture = RuntimeFixture::new_recovering(&["iphone-a"], terminate_plan());
        let (_run_id, attempt_id) = fixture.seed_failed_verified_terminate_without_retry_proof();
        fixture.driver.set_process("iphone-a", TARGET, Some(10));
        fixture
            .runtime
            .recover_startup()
            .await
            .expect("finish startup after the terminal crash gap");

        let retry = fixture
            .runtime
            .retry_attempt(attempt_id)
            .await
            .expect("fresh reconciliation repairs the missing retry proof");
        assert_eq!(retry.attempt_no, 2);
        assert_eq!(retry.state, FlowAttemptState::Succeeded);
        assert_eq!(fixture.driver.terminate_calls.load(Ordering::SeqCst), 1);
        assert!(
            fixture
                .driver
                .process_inspection_calls
                .load(Ordering::SeqCst)
                >= 2
        );
        fixture.shutdown().await;
    }

    #[tokio::test]
    async fn retry_refreshes_the_exact_process_non_delivery_proof_before_dispatch() {
        let fixture = RuntimeFixture::new_recovering(&["iphone-a"], terminate_plan());
        let (_run_id, attempt_id) = fixture.seed_terminate_recovery_run();
        fixture.driver.set_process("iphone-a", TARGET, Some(10));
        fixture
            .runtime
            .recover_startup()
            .await
            .expect("recover retryable Terminate");

        let retry = fixture
            .runtime
            .retry_attempt(attempt_id)
            .await
            .expect("fresh process proof permits retry");
        assert_eq!(retry.attempt_no, 2);
        assert_eq!(retry.state, FlowAttemptState::Succeeded);
        assert_eq!(fixture.driver.terminate_calls.load(Ordering::SeqCst), 1);
        assert!(
            fixture
                .driver
                .process_inspection_calls
                .load(Ordering::SeqCst)
                >= 3
        );
        fixture.shutdown().await;
    }

    #[tokio::test]
    async fn startup_launch_reads_active_app_without_foregrounding_again() {
        let fixture = RuntimeFixture::new_recovering(&["iphone-a"], launch_plan());
        fixture.driver.set_active_bundle(TARGET);
        let (run_id, attempt_id) = fixture.seed_launch_recovery_run();

        fixture
            .runtime
            .recover_startup()
            .await
            .expect("recover startup");
        let detail = fixture
            .runtime
            .wait_terminal(run_id)
            .await
            .expect("reconciled Launch run");
        assert_eq!(detail.run.state, FlowAggregateState::Succeeded);
        assert_eq!(
            detail
                .attempts
                .iter()
                .find(|attempt| attempt.id == attempt_id)
                .expect("reconciled Launch")
                .state,
            FlowAttemptState::Succeeded
        );
        assert_eq!(fixture.driver.launch_calls.load(Ordering::SeqCst), 0);
        assert!(fixture.driver.active_app_reads.load(Ordering::SeqCst) > 0);
        fixture.shutdown().await;
    }

    fn device_state(detail: &crate::FlowRunDetail, udid: &str) -> FlowDeviceRunState {
        detail
            .device_runs
            .iter()
            .find(|device| device.udid == udid)
            .expect("device run")
            .state
    }

    /// **A device the executor gave up on must not be left `Running`.**
    ///
    /// `run_device` can return an error *because the terminal write is what failed* — the context
    /// close was refused, or the success transition was. The device is then non-terminal with no
    /// task behind it, and the old code logged that at `debug!` and returned `Ok(())`: the worker
    /// finished normally, the run stayed `Running` forever, and a restart handed it to recovery,
    /// which cannot classify a device that was never terminalized and marks it `RecoveryFailed`.
    #[tokio::test]
    async fn a_device_the_executor_abandoned_is_forced_terminal() {
        let fixture = RuntimeFixture::new(&["iphone-a"], wait_plan(10)).await;
        let run = fixture
            .database
            .create_flow_run(
                &fixture.revision,
                FlowSelectionSnapshot {
                    requested: FlowTargetSelection::One {
                        udid: "iphone-a".into(),
                    },
                    target_udids: vec!["iphone-a".into()],
                },
            )
            .expect("create run");
        let device = fixture
            .database
            .create_flow_device_run(run.id, "iphone-a")
            .expect("create device run");
        fixture
            .database
            .transition_flow_device_run(
                device.id,
                FlowDeviceRunState::Queued,
                FlowDeviceRunState::Preflight,
                None,
            )
            .expect("preflight");
        fixture
            .database
            .transition_flow_device_run(
                device.id,
                FlowDeviceRunState::Preflight,
                FlowDeviceRunState::Running,
                Some(FlowCapabilitySnapshot {
                    scope: FlowPreflightScope::TargetQualified {
                        bundle_id: TARGET.into(),
                    },
                    device: Some(capability_snapshot(TARGET)),
                    agent_status: None,
                    capability_ids: BTreeSet::from(["app.terminate".to_string()]),
                }),
            )
            .expect("running");

        // Exactly the state the old code walked away from: Running, no task, and the executor has
        // already returned an error.
        fixture
            .runtime
            .settle_device_after_failure(
                run.id,
                device.id,
                "iphone-a",
                "DeviceControl",
                "context close refused",
            )
            .expect("settle a stuck device");

        let detail = fixture
            .database
            .get_flow_run(run.id)
            .expect("load run")
            .expect("run");
        assert_eq!(
            device_state(&detail, "iphone-a"),
            FlowDeviceRunState::Failed
        );
        let error = detail.device_runs[0]
            .error
            .as_ref()
            .expect("a forced failure records why");
        assert_eq!(error.code, "DeviceControl");
        assert!(
            error.message.contains("did not terminalize"),
            "the message has to say this was forced, not ordinary: {}",
            error.message
        );
        // And the run itself must not be left Running behind it.
        assert!(
            fixture
                .database
                .recompute_run_projection(run.id)
                .expect("project run")
                .state
                .is_terminal(),
            "a run whose only device was forced failed is finished"
        );
    }

    /// A device that *did* terminalize is left exactly as it is -- the net must not overwrite a
    /// real outcome with a synthetic one.
    #[tokio::test]
    async fn a_device_that_already_settled_is_not_touched_again() {
        let fixture = RuntimeFixture::new(&["iphone-a"], wait_plan(10)).await;
        let run = fixture
            .database
            .create_flow_run(
                &fixture.revision,
                FlowSelectionSnapshot {
                    requested: FlowTargetSelection::One {
                        udid: "iphone-a".into(),
                    },
                    target_udids: vec!["iphone-a".into()],
                },
            )
            .expect("create run");
        let device = fixture
            .database
            .create_flow_device_run(run.id, "iphone-a")
            .expect("create device run");
        let cancelled = FlowErrorRecord {
            code: "Cancelled".into(),
            message: "operator cancelled the run".into(),
            node_id: None,
            field: None,
            udid: Some("iphone-a".into()),
            attempt_id: None,
        };
        fixture
            .database
            .mark_device_terminal(
                device.id,
                &[FlowDeviceRunState::Queued],
                FlowDeviceRunState::Cancelled,
                Some(cancelled),
                super::empty_release_proof("iphone-a"),
            )
            .expect("cancel the device");

        fixture
            .runtime
            .settle_device_after_failure(
                run.id,
                device.id,
                "iphone-a",
                "Cancelled",
                "operator cancelled the run",
            )
            .expect("settling a settled device is a no-op");

        let detail = fixture
            .database
            .get_flow_run(run.id)
            .expect("load run")
            .expect("run");
        assert_eq!(
            device_state(&detail, "iphone-a"),
            FlowDeviceRunState::Cancelled,
            "a real outcome is not overwritten with a forced failure"
        );
        assert_eq!(
            detail.device_runs[0]
                .error
                .as_ref()
                .expect("cancellation reason")
                .code,
            "Cancelled"
        );
    }

    struct RuntimeFixture {
        runtime: FlowRuntime,
        revision: FlowRevisionRecord,
        database: Arc<Database>,
        registry: crate::DeviceRegistry,
        driver: Arc<RuntimeDriver>,
        control: Arc<DeviceControlPlane>,
        work: Arc<DeviceWorkCoordinator>,
        artifacts: FlowArtifactStore,
        events: EventBus,
    }

    impl RuntimeFixture {
        async fn new(udids: &[&str], plan: CompiledFlowPlanV2) -> Self {
            let fixture = Self::new_recovering(udids, plan);
            fixture
                .runtime
                .recover_startup()
                .await
                .expect("initialize ready runtime");
            fixture
        }

        fn new_recovering(udids: &[&str], plan: CompiledFlowPlanV2) -> Self {
            Self::new_recovering_with_frames(udids, plan, Arc::new(EmptyFrames))
        }

        fn new_recovering_with_frames(
            udids: &[&str],
            plan: CompiledFlowPlanV2,
            frames: Arc<dyn GenerationFrameSource>,
        ) -> Self {
            let events = crate::EventBus::new(128);
            let registry = crate::DeviceRegistry::new(events.clone());
            registry.upsert_many(
                udids
                    .iter()
                    .map(|udid| device(udid, ConnectionKind::Mock, DeviceStatus::Ready))
                    .collect(),
            );
            let database_path =
                std::env::temp_dir().join(format!("riviu-flow-runtime-{}.db", Uuid::new_v4()));
            let database = Arc::new(Database::open(database_path).expect("runtime db"));
            let mut document = FlowDocumentV2::empty("Runtime fixture");
            document.id = plan.flow_id;
            document.revision = plan.revision;
            let hash = compiled_plan_sha256(&plan).expect("plan hash");
            let revision = database
                .save_flow_revision(None, &document, &plan, &hash)
                .expect("save revision");

            let work = Arc::new(DeviceWorkCoordinator::new());
            let streams = Arc::new(StreamBudgetManager::new(2).expect("stream budget"));
            let driver = Arc::new(RuntimeDriver::new(udids));
            let control = Arc::new(DeviceControlPlane::new(
                driver.clone(),
                work.clone(),
                streams,
            ));
            let artifact_root = std::env::temp_dir()
                .join(format!("riviu-flow-runtime-artifacts-{}", Uuid::new_v4()));
            let artifacts = FlowArtifactStore::new(artifact_root).expect("artifact store");
            let runtime = FlowRuntime::new(FlowRuntimeDeps {
                database: database.clone(),
                events: events.clone(),
                registry: registry.clone(),
                control: control.clone(),
                frames,
                artifacts: artifacts.clone(),
            });
            Self {
                runtime,
                revision,
                database,
                registry,
                driver,
                control,
                work,
                artifacts,
                events,
            }
        }

        async fn wait_device_state(&self, run_id: Uuid, udid: &str, expected: FlowDeviceRunState) {
            let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
            loop {
                let detail = self
                    .database
                    .get_flow_run(run_id)
                    .expect("load run")
                    .expect("run");
                if device_state(&detail, udid) == expected {
                    return;
                }
                assert!(
                    tokio::time::Instant::now() < deadline,
                    "device {udid} did not reach {expected:?}"
                );
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        }

        async fn wait_attempt_state(&self, run_id: Uuid, expected: FlowAttemptState) {
            let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
            loop {
                let detail = self
                    .database
                    .get_flow_run(run_id)
                    .expect("load run")
                    .expect("run");
                if detail
                    .attempts
                    .iter()
                    .any(|attempt| attempt.state == expected)
                {
                    return;
                }
                assert!(
                    tokio::time::Instant::now() < deadline,
                    "attempt did not reach {expected:?}"
                );
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        }

        async fn wait_runtime_bookkeeping_empty(&self) {
            let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
            loop {
                let tasks = self.runtime.active_task_count().await;
                let revisions = self.runtime.emitted_revision_count();
                if tasks == 0 && revisions == 0 {
                    return;
                }
                assert!(
                    tokio::time::Instant::now() < deadline,
                    "Flow runtime bookkeeping did not drain: tasks={tasks}, revisions={revisions}"
                );
                tokio::task::yield_now().await;
            }
        }

        async fn shutdown(&self) {
            self.runtime.shutdown().await.expect("runtime shutdown");
            self.control
                .shutdown_cleanup()
                .await
                .expect("control shutdown");
        }

        fn seed_wait_retry_run(&self) -> (Uuid, Uuid, Uuid, Uuid) {
            let run = self
                .database
                .create_flow_run(
                    &self.revision,
                    FlowSelectionSnapshot {
                        requested: FlowTargetSelection::One {
                            udid: "iphone-a".into(),
                        },
                        target_udids: vec!["iphone-a".into()],
                    },
                )
                .expect("create retry run");
            let device = self
                .database
                .create_flow_device_run(run.id, "iphone-a")
                .expect("create retry device");
            self.database
                .transition_flow_device_run(
                    device.id,
                    FlowDeviceRunState::Queued,
                    FlowDeviceRunState::Preflight,
                    None,
                )
                .expect("retry preflight");
            self.database
                .transition_flow_device_run(
                    device.id,
                    FlowDeviceRunState::Preflight,
                    FlowDeviceRunState::Running,
                    Some(FlowCapabilitySnapshot {
                        scope: FlowPreflightScope::TargetQualified {
                            bundle_id: TARGET.into(),
                        },
                        device: Some(capability_snapshot(TARGET)),
                        agent_status: None,
                        capability_ids: BTreeSet::from(["app.terminate".to_string()]),
                    }),
                )
                .expect("retry running");
            let attempts = self
                .database
                .initialize_flow_device_attempts(device.id)
                .expect("initialize retry attempts");
            let terminate = attempts
                .iter()
                .find(|attempt| attempt.action_kind == ActionKind::TerminateApp)
                .expect("terminate attempt")
                .clone();
            let wait = attempts
                .iter()
                .find(|attempt| attempt.action_kind == ActionKind::Wait)
                .expect("wait attempt")
                .clone();
            let end = attempts
                .iter()
                .find(|attempt| attempt.action_kind == ActionKind::End)
                .expect("end attempt")
                .clone();

            let terminate_node = self
                .revision
                .compiled_plan
                .nodes
                .get(&terminate.node_id)
                .expect("terminate node");
            let baseline = EvidenceBaseline::Process {
                bundle_id: TARGET.into(),
                pid: Some(10),
            };
            self.database
                .transition_attempt(
                    terminate.id,
                    FlowAttemptState::Queued,
                    FlowAttemptState::IntentCommitted,
                    crate::db::AttemptTransitionPatch {
                        canonical_input: Some(
                            serde_json::to_value(&terminate_node.config).expect("terminate input"),
                        ),
                        evidence_baseline: Some(
                            serde_json::to_value(&baseline).expect("terminate baseline"),
                        ),
                        ..Default::default()
                    },
                )
                .expect("terminate intent");
            self.database
                .transition_attempt(
                    terminate.id,
                    FlowAttemptState::IntentCommitted,
                    FlowAttemptState::EffectDispatched,
                    Default::default(),
                )
                .expect("terminate dispatched");
            self.database
                .transition_attempt(
                    terminate.id,
                    FlowAttemptState::EffectDispatched,
                    FlowAttemptState::Verifying,
                    Default::default(),
                )
                .expect("terminate verifying");
            let evidence = crate::verify_process_absence(
                terminate_node
                    .postcondition
                    .as_ref()
                    .expect("terminate proof"),
                &baseline,
                &ProcessAbsenceProof {
                    bundle_id: TARGET.into(),
                    old_pid: Some(10),
                },
            )
            .expect("terminate evidence");
            self.database
                .transition_attempt(
                    terminate.id,
                    FlowAttemptState::Verifying,
                    FlowAttemptState::Succeeded,
                    crate::db::AttemptTransitionPatch {
                        evidence_result: Some(
                            serde_json::to_value(evidence).expect("terminate result"),
                        ),
                        ..Default::default()
                    },
                )
                .expect("terminate success");

            let wait_node = self
                .revision
                .compiled_plan
                .nodes
                .get(&wait.node_id)
                .expect("wait node");
            self.database
                .transition_attempt(
                    wait.id,
                    FlowAttemptState::Queued,
                    FlowAttemptState::IntentCommitted,
                    crate::db::AttemptTransitionPatch {
                        canonical_input: Some(
                            serde_json::to_value(&wait_node.config).expect("wait input"),
                        ),
                        evidence_baseline: Some(
                            serde_json::to_value(EvidenceBaseline::None).expect("wait baseline"),
                        ),
                        ..Default::default()
                    },
                )
                .expect("wait intent");
            let failure = FlowErrorRecord {
                code: "FixtureNonDelivery".into(),
                message: "fixture failed before dispatch".into(),
                node_id: Some(wait.node_id),
                field: None,
                udid: Some("iphone-a".into()),
                attempt_id: Some(wait.id),
            };
            self.database
                .transition_attempt(
                    wait.id,
                    FlowAttemptState::IntentCommitted,
                    FlowAttemptState::FailedBeforeDispatch,
                    crate::db::AttemptTransitionPatch {
                        error: Some(failure.clone()),
                        ..Default::default()
                    },
                )
                .expect("wait failed before dispatch");
            self.database
                .mark_device_terminal(
                    device.id,
                    &[FlowDeviceRunState::Running],
                    FlowDeviceRunState::Failed,
                    Some(failure),
                    super::empty_release_proof("iphone-a"),
                )
                .expect("failed retry device");
            self.database
                .recompute_run_projection(run.id)
                .expect("failed retry projection");
            (run.id, terminate.id, wait.id, end.id)
        }

        fn seed_launch_wait_retry_run(&self) -> Uuid {
            let run = self
                .database
                .create_flow_run(
                    &self.revision,
                    FlowSelectionSnapshot {
                        requested: FlowTargetSelection::One {
                            udid: "iphone-a".into(),
                        },
                        target_udids: vec!["iphone-a".into()],
                    },
                )
                .expect("create launch retry run");
            let device = self
                .database
                .create_flow_device_run(run.id, "iphone-a")
                .expect("create launch retry device");
            self.database
                .transition_flow_device_run(
                    device.id,
                    FlowDeviceRunState::Queued,
                    FlowDeviceRunState::Preflight,
                    None,
                )
                .expect("launch retry preflight");
            self.database
                .transition_flow_device_run(
                    device.id,
                    FlowDeviceRunState::Preflight,
                    FlowDeviceRunState::Running,
                    Some(FlowCapabilitySnapshot {
                        scope: FlowPreflightScope::TargetQualified {
                            bundle_id: TARGET.into(),
                        },
                        device: Some(capability_snapshot(TARGET)),
                        agent_status: None,
                        capability_ids: BTreeSet::from(["app.launch".to_string()]),
                    }),
                )
                .expect("launch retry running");
            let attempts = self
                .database
                .initialize_flow_device_attempts(device.id)
                .expect("initialize launch retry attempts");
            let launch = attempts
                .iter()
                .find(|attempt| attempt.action_kind == ActionKind::LaunchApp)
                .expect("launch predecessor");
            let wait = attempts
                .iter()
                .find(|attempt| attempt.action_kind == ActionKind::Wait)
                .expect("wait retry");
            let launch_node = self
                .revision
                .compiled_plan
                .nodes
                .get(&launch.node_id)
                .expect("launch node");
            self.database
                .transition_attempt(
                    launch.id,
                    FlowAttemptState::Queued,
                    FlowAttemptState::IntentCommitted,
                    crate::db::AttemptTransitionPatch {
                        canonical_input: Some(
                            serde_json::to_value(&launch_node.config).expect("launch input"),
                        ),
                        evidence_baseline: Some(
                            serde_json::to_value(EvidenceBaseline::None).expect("launch baseline"),
                        ),
                        ..Default::default()
                    },
                )
                .expect("launch intent");
            self.database
                .transition_attempt(
                    launch.id,
                    FlowAttemptState::IntentCommitted,
                    FlowAttemptState::EffectDispatched,
                    Default::default(),
                )
                .expect("launch dispatched");
            self.database
                .transition_attempt(
                    launch.id,
                    FlowAttemptState::EffectDispatched,
                    FlowAttemptState::Verifying,
                    Default::default(),
                )
                .expect("launch verifying");
            self.database
                .transition_attempt(
                    launch.id,
                    FlowAttemptState::Verifying,
                    FlowAttemptState::Succeeded,
                    crate::db::AttemptTransitionPatch {
                        evidence_result: Some(serde_json::json!({
                            "kind": "activeAppEquals",
                            "matched": true,
                            "observedSha256": format!("{:x}", Sha256::digest(TARGET.as_bytes())),
                            "measurement": {"bundleId": TARGET},
                        })),
                        ..Default::default()
                    },
                )
                .expect("launch success");
            let wait_node = self
                .revision
                .compiled_plan
                .nodes
                .get(&wait.node_id)
                .expect("wait node");
            self.database
                .transition_attempt(
                    wait.id,
                    FlowAttemptState::Queued,
                    FlowAttemptState::IntentCommitted,
                    crate::db::AttemptTransitionPatch {
                        canonical_input: Some(
                            serde_json::to_value(&wait_node.config).expect("wait input"),
                        ),
                        evidence_baseline: Some(
                            serde_json::to_value(EvidenceBaseline::None).expect("wait baseline"),
                        ),
                        ..Default::default()
                    },
                )
                .expect("wait intent");
            let failure = FlowErrorRecord {
                code: "FixtureNonDelivery".into(),
                message: "fixture failed before dispatch".into(),
                node_id: Some(wait.node_id),
                field: None,
                udid: Some("iphone-a".into()),
                attempt_id: Some(wait.id),
            };
            self.database
                .transition_attempt(
                    wait.id,
                    FlowAttemptState::IntentCommitted,
                    FlowAttemptState::FailedBeforeDispatch,
                    crate::db::AttemptTransitionPatch {
                        error: Some(failure.clone()),
                        ..Default::default()
                    },
                )
                .expect("wait failed before dispatch");
            self.database
                .mark_device_terminal(
                    device.id,
                    &[FlowDeviceRunState::Running],
                    FlowDeviceRunState::Failed,
                    Some(failure),
                    FlowContextReleaseProof {
                        udid: "iphone-a".into(),
                        owner: DeviceWorkOwner::Script,
                        had_session: true,
                        had_stream: false,
                    },
                )
                .expect("close launch retry device");
            self.database
                .recompute_run_projection(run.id)
                .expect("launch retry projection");
            wait.id
        }

        fn seed_uncertain_run(&self) -> Uuid {
            let run = self
                .database
                .create_flow_run(
                    &self.revision,
                    FlowSelectionSnapshot {
                        requested: FlowTargetSelection::One {
                            udid: "iphone-a".into(),
                        },
                        target_udids: vec!["iphone-a".into()],
                    },
                )
                .expect("create uncertain run");
            let device = self
                .database
                .create_flow_device_run(run.id, "iphone-a")
                .expect("create uncertain device");
            self.database
                .transition_flow_device_run(
                    device.id,
                    FlowDeviceRunState::Queued,
                    FlowDeviceRunState::Preflight,
                    None,
                )
                .expect("uncertain preflight");
            self.database
                .transition_flow_device_run(
                    device.id,
                    FlowDeviceRunState::Preflight,
                    FlowDeviceRunState::Running,
                    Some(FlowCapabilitySnapshot {
                        scope: FlowPreflightScope::TargetQualified {
                            bundle_id: TARGET.into(),
                        },
                        device: Some(capability_snapshot(TARGET)),
                        agent_status: None,
                        capability_ids: BTreeSet::new(),
                    }),
                )
                .expect("uncertain running");
            let attempt = self
                .database
                .initialize_flow_device_attempts(device.id)
                .expect("initialize uncertain attempt")
                .into_iter()
                .next()
                .expect("uncertain attempt");
            let node = self
                .revision
                .compiled_plan
                .nodes
                .get(&attempt.node_id)
                .expect("uncertain node");
            let baseline = EvidenceBaseline::Frame {
                generation: 1,
                jpeg_sha256: "c".repeat(64),
                image: RgbImage::new(4, 4),
            };
            self.database
                .transition_attempt(
                    attempt.id,
                    FlowAttemptState::Queued,
                    FlowAttemptState::IntentCommitted,
                    crate::db::AttemptTransitionPatch {
                        canonical_input: Some(
                            serde_json::to_value(&node.config).expect("ambiguous input"),
                        ),
                        evidence_baseline: Some(
                            serde_json::to_value(baseline).expect("ambiguous baseline"),
                        ),
                        ..Default::default()
                    },
                )
                .expect("ambiguous intent");
            self.database
                .transition_attempt(
                    attempt.id,
                    FlowAttemptState::IntentCommitted,
                    FlowAttemptState::EffectDispatched,
                    Default::default(),
                )
                .expect("ambiguous dispatched");
            let error = FlowErrorRecord {
                code: "FixtureAmbiguous".into(),
                message: "fixture effect outcome is ambiguous".into(),
                node_id: Some(attempt.node_id),
                field: None,
                udid: Some("iphone-a".into()),
                attempt_id: Some(attempt.id),
            };
            self.database
                .transition_attempt(
                    attempt.id,
                    FlowAttemptState::EffectDispatched,
                    FlowAttemptState::Uncertain,
                    crate::db::AttemptTransitionPatch {
                        error: Some(error.clone()),
                        ..Default::default()
                    },
                )
                .expect("ambiguous uncertain");
            self.database
                .mark_device_terminal(
                    device.id,
                    &[FlowDeviceRunState::Running],
                    FlowDeviceRunState::Failed,
                    Some(error),
                    super::empty_release_proof("iphone-a"),
                )
                .expect("failed uncertain device");
            self.database
                .recompute_run_projection(run.id)
                .expect("uncertain projection");
            attempt.id
        }

        fn seed_wait_recovery_run(&self, state: FlowAttemptState) -> (Uuid, Uuid) {
            let run = self
                .database
                .create_flow_run(
                    &self.revision,
                    FlowSelectionSnapshot {
                        requested: FlowTargetSelection::One {
                            udid: "iphone-a".into(),
                        },
                        target_udids: vec!["iphone-a".into()],
                    },
                )
                .expect("create recovery run");
            let device = self
                .database
                .create_flow_device_run(run.id, "iphone-a")
                .expect("create recovery device");
            self.database
                .transition_flow_device_run(
                    device.id,
                    FlowDeviceRunState::Queued,
                    FlowDeviceRunState::Preflight,
                    None,
                )
                .expect("recovery preflight");
            self.database
                .transition_flow_device_run(
                    device.id,
                    FlowDeviceRunState::Preflight,
                    FlowDeviceRunState::Running,
                    Some(FlowCapabilitySnapshot {
                        scope: FlowPreflightScope::TargetFree,
                        device: None,
                        agent_status: None,
                        capability_ids: BTreeSet::new(),
                    }),
                )
                .expect("recovery running");
            let attempt = self
                .database
                .initialize_flow_device_attempts(device.id)
                .expect("initialize recovery attempt")
                .into_iter()
                .next()
                .expect("recovery attempt");
            if state == FlowAttemptState::Interrupted {
                self.database
                    .transition_attempt(
                        attempt.id,
                        FlowAttemptState::Queued,
                        FlowAttemptState::Interrupted,
                        Default::default(),
                    )
                    .expect("interrupt queued Wait");
            } else if matches!(
                state,
                FlowAttemptState::IntentCommitted
                    | FlowAttemptState::EffectDispatched
                    | FlowAttemptState::Verifying
            ) {
                let node = self
                    .revision
                    .compiled_plan
                    .nodes
                    .get(&attempt.node_id)
                    .expect("recovery Wait node");
                self.database
                    .transition_attempt(
                        attempt.id,
                        FlowAttemptState::Queued,
                        FlowAttemptState::IntentCommitted,
                        crate::db::AttemptTransitionPatch {
                            canonical_input: Some(
                                serde_json::to_value(&node.config).expect("recovery input"),
                            ),
                            evidence_baseline: Some(
                                serde_json::to_value(EvidenceBaseline::None)
                                    .expect("recovery baseline"),
                            ),
                            ..Default::default()
                        },
                    )
                    .expect("recovery intent");
                if matches!(
                    state,
                    FlowAttemptState::EffectDispatched | FlowAttemptState::Verifying
                ) {
                    self.database
                        .transition_attempt(
                            attempt.id,
                            FlowAttemptState::IntentCommitted,
                            FlowAttemptState::EffectDispatched,
                            Default::default(),
                        )
                        .expect("recovery dispatched");
                }
                if state == FlowAttemptState::Verifying {
                    self.database
                        .transition_attempt(
                            attempt.id,
                            FlowAttemptState::EffectDispatched,
                            FlowAttemptState::Verifying,
                            Default::default(),
                        )
                        .expect("recovery verifying");
                }
            }
            (run.id, attempt.id)
        }

        /// A run whose persisted shape will not revalidate: the frozen selection names two
        /// devices and only one device run exists.
        ///
        /// Not a contrived corruption — it is what a run half-written by a process that died
        /// between `create_flow_run` and its second `create_flow_device_run` looks like, and
        /// `recompute_run_projection` refuses it by design ("Flow device projection does not
        /// match its frozen selection"). The sweep therefore settles every attempt and
        /// device it can and still cannot finish the run, which is the population the
        /// stuck-run counter exists for.
        fn seed_unsettleable_run(&self) -> Uuid {
            let run = self
                .database
                .create_flow_run(
                    &self.revision,
                    FlowSelectionSnapshot {
                        requested: FlowTargetSelection::Selected {
                            udids: vec!["iphone-a".into(), "iphone-b".into()],
                        },
                        target_udids: vec!["iphone-a".into(), "iphone-b".into()],
                    },
                )
                .expect("create half-written run");
            let device = self
                .database
                .create_flow_device_run(run.id, "iphone-a")
                .expect("create the one device that made it");
            self.database
                .transition_flow_device_run(
                    device.id,
                    FlowDeviceRunState::Queued,
                    FlowDeviceRunState::Preflight,
                    None,
                )
                .expect("preflight");
            self.database
                .transition_flow_device_run(
                    device.id,
                    FlowDeviceRunState::Preflight,
                    FlowDeviceRunState::Running,
                    Some(FlowCapabilitySnapshot {
                        scope: FlowPreflightScope::TargetFree,
                        device: None,
                        agent_status: None,
                        capability_ids: BTreeSet::new(),
                    }),
                )
                .expect("running");
            let attempt = self
                .database
                .initialize_flow_device_attempts(device.id)
                .expect("initialize attempt")
                .into_iter()
                .next()
                .expect("attempt");
            let node = self
                .revision
                .compiled_plan
                .nodes
                .get(&attempt.node_id)
                .expect("Wait node");
            self.database
                .transition_attempt(
                    attempt.id,
                    FlowAttemptState::Queued,
                    FlowAttemptState::IntentCommitted,
                    crate::db::AttemptTransitionPatch {
                        canonical_input: Some(serde_json::to_value(&node.config).expect("input")),
                        evidence_baseline: Some(
                            serde_json::to_value(EvidenceBaseline::None).expect("baseline"),
                        ),
                        ..Default::default()
                    },
                )
                .expect("intent");
            run.id
        }

        fn seed_ambiguous_recovery_run(&self) -> (Uuid, Uuid) {
            let run = self
                .database
                .create_flow_run(
                    &self.revision,
                    FlowSelectionSnapshot {
                        requested: FlowTargetSelection::One {
                            udid: "iphone-a".into(),
                        },
                        target_udids: vec!["iphone-a".into()],
                    },
                )
                .expect("create ambiguous recovery run");
            let device = self
                .database
                .create_flow_device_run(run.id, "iphone-a")
                .expect("create ambiguous recovery device");
            self.database
                .transition_flow_device_run(
                    device.id,
                    FlowDeviceRunState::Queued,
                    FlowDeviceRunState::Preflight,
                    None,
                )
                .expect("ambiguous recovery preflight");
            self.database
                .transition_flow_device_run(
                    device.id,
                    FlowDeviceRunState::Preflight,
                    FlowDeviceRunState::Running,
                    Some(FlowCapabilitySnapshot {
                        scope: FlowPreflightScope::TargetQualified {
                            bundle_id: TARGET.into(),
                        },
                        device: Some(capability_snapshot(TARGET)),
                        agent_status: None,
                        capability_ids: BTreeSet::new(),
                    }),
                )
                .expect("ambiguous recovery running");
            let attempt = self
                .database
                .initialize_flow_device_attempts(device.id)
                .expect("initialize ambiguous recovery attempt")
                .into_iter()
                .next()
                .expect("ambiguous recovery attempt");
            let node = self
                .revision
                .compiled_plan
                .nodes
                .get(&attempt.node_id)
                .expect("ambiguous recovery node");
            self.database
                .transition_attempt(
                    attempt.id,
                    FlowAttemptState::Queued,
                    FlowAttemptState::IntentCommitted,
                    crate::db::AttemptTransitionPatch {
                        canonical_input: Some(
                            serde_json::to_value(&node.config).expect("ambiguous recovery input"),
                        ),
                        evidence_baseline: Some(
                            serde_json::to_value(EvidenceBaseline::Frame {
                                generation: 1,
                                jpeg_sha256: "c".repeat(64),
                                image: RgbImage::new(4, 4),
                            })
                            .expect("ambiguous recovery baseline"),
                        ),
                        ..Default::default()
                    },
                )
                .expect("ambiguous recovery intent");
            self.database
                .transition_attempt(
                    attempt.id,
                    FlowAttemptState::IntentCommitted,
                    FlowAttemptState::EffectDispatched,
                    Default::default(),
                )
                .expect("ambiguous recovery dispatched");
            (run.id, attempt.id)
        }

        fn seed_terminate_recovery_run(&self) -> (Uuid, Uuid) {
            self.seed_effect_recovery_run(EvidenceBaseline::Process {
                bundle_id: TARGET.into(),
                pid: Some(10),
            })
        }

        fn seed_failed_verified_terminate_without_retry_proof(&self) -> (Uuid, Uuid) {
            let (run_id, attempt_id) = self.seed_terminate_recovery_run();
            let context = self
                .database
                .get_flow_attempt_execution_context(attempt_id)
                .expect("load crash-gap attempt")
                .expect("crash-gap attempt");
            let error = FlowErrorRecord {
                code: "ReconciledDesiredStateAbsent".into(),
                message: "fixture stopped before retry-safety proof commit".into(),
                node_id: Some(context.attempt.node_id),
                field: None,
                udid: Some(context.device.udid.clone()),
                attempt_id: Some(attempt_id),
            };
            self.database
                .transition_attempt(
                    attempt_id,
                    FlowAttemptState::EffectDispatched,
                    FlowAttemptState::Verifying,
                    Default::default(),
                )
                .expect("crash-gap verifying");
            self.database
                .transition_attempt(
                    attempt_id,
                    FlowAttemptState::Verifying,
                    FlowAttemptState::FailedVerified,
                    crate::db::AttemptTransitionPatch {
                        evidence_result: Some(serde_json::json!({
                            "kind": "processAbsent",
                            "matched": false,
                            "observedSha256": format!(
                                "{:x}",
                                Sha256::digest(
                                    serde_json::to_vec(&serde_json::json!({
                                        "bundleId": TARGET,
                                        "pid": 10,
                                        "running": true,
                                    }))
                                    .expect("serialize crash-gap observation")
                                )
                            ),
                            "measurement": {
                                "bundleId": TARGET,
                                "running": true,
                                "oldPid": 10,
                            },
                        })),
                        error: Some(error.clone()),
                        ..Default::default()
                    },
                )
                .expect("crash-gap failed verification");
            self.database
                .mark_device_terminal(
                    context.device.id,
                    &[FlowDeviceRunState::Running],
                    FlowDeviceRunState::Failed,
                    Some(error),
                    super::empty_release_proof(&context.device.udid),
                )
                .expect("crash-gap failed device");
            self.database
                .recompute_run_projection(run_id)
                .expect("crash-gap failed run");
            let persisted = self
                .database
                .get_flow_attempt_execution_context(attempt_id)
                .expect("reload crash-gap attempt")
                .expect("persisted crash-gap attempt");
            assert_eq!(persisted.attempt.state, FlowAttemptState::FailedVerified);
            assert!(!persisted.attempt.retry_allowed);
            (run_id, attempt_id)
        }

        fn seed_launch_recovery_run(&self) -> (Uuid, Uuid) {
            self.seed_effect_recovery_run(EvidenceBaseline::None)
        }

        fn seed_effect_recovery_run(&self, baseline: EvidenceBaseline) -> (Uuid, Uuid) {
            let run = self
                .database
                .create_flow_run(
                    &self.revision,
                    FlowSelectionSnapshot {
                        requested: FlowTargetSelection::One {
                            udid: "iphone-a".into(),
                        },
                        target_udids: vec!["iphone-a".into()],
                    },
                )
                .expect("create effect recovery run");
            let device = self
                .database
                .create_flow_device_run(run.id, "iphone-a")
                .expect("create effect recovery device");
            self.database
                .transition_flow_device_run(
                    device.id,
                    FlowDeviceRunState::Queued,
                    FlowDeviceRunState::Preflight,
                    None,
                )
                .expect("effect recovery preflight");
            self.database
                .transition_flow_device_run(
                    device.id,
                    FlowDeviceRunState::Preflight,
                    FlowDeviceRunState::Running,
                    Some(FlowCapabilitySnapshot {
                        scope: FlowPreflightScope::TargetQualified {
                            bundle_id: TARGET.into(),
                        },
                        device: Some(capability_snapshot(TARGET)),
                        agent_status: None,
                        capability_ids: BTreeSet::from(["app.terminate".to_string()]),
                    }),
                )
                .expect("effect recovery running");
            let attempt = self
                .database
                .initialize_flow_device_attempts(device.id)
                .expect("initialize effect recovery attempt")
                .into_iter()
                .next()
                .expect("effect recovery attempt");
            let node = self
                .revision
                .compiled_plan
                .nodes
                .get(&attempt.node_id)
                .expect("effect recovery node");
            self.database
                .transition_attempt(
                    attempt.id,
                    FlowAttemptState::Queued,
                    FlowAttemptState::IntentCommitted,
                    crate::db::AttemptTransitionPatch {
                        canonical_input: Some(
                            serde_json::to_value(&node.config).expect("effect recovery input"),
                        ),
                        evidence_baseline: Some(
                            serde_json::to_value(baseline).expect("effect recovery baseline"),
                        ),
                        ..Default::default()
                    },
                )
                .expect("effect recovery intent");
            self.database
                .transition_attempt(
                    attempt.id,
                    FlowAttemptState::IntentCommitted,
                    FlowAttemptState::EffectDispatched,
                    Default::default(),
                )
                .expect("effect recovery dispatched");
            (run.id, attempt.id)
        }
    }

    #[derive(Default)]
    struct RuntimeDriver {
        processes: Mutex<HashMap<(String, String), u64>>,
        fail_terminate: Mutex<BTreeSet<String>>,
        blocked_terminate: Mutex<Option<String>>,
        terminate_started: AtomicBool,
        terminate_started_notify: tokio::sync::Notify,
        terminate_release: tokio::sync::Notify,
        terminate_calls: AtomicUsize,
        block_process_read: AtomicBool,
        process_read_started: AtomicBool,
        process_read_started_notify: tokio::sync::Notify,
        process_read_release: tokio::sync::Notify,
        process_inspection_calls: AtomicUsize,
        active_bundle: Mutex<String>,
        active_app_reads: AtomicUsize,
        session_start_calls: AtomicUsize,
        session_text: Arc<Mutex<Option<String>>>,
        read_text_calls: Arc<AtomicUsize>,
        type_text_calls: Arc<AtomicUsize>,
        launch_calls: AtomicUsize,
    }

    impl RuntimeDriver {
        fn new(udids: &[&str]) -> Self {
            Self {
                processes: Mutex::new(
                    udids
                        .iter()
                        .enumerate()
                        .map(|(index, udid)| {
                            (
                                ((*udid).to_string(), TARGET.to_string()),
                                u64::try_from(index + 10).expect("fixture pid"),
                            )
                        })
                        .collect(),
                ),
                ..Default::default()
            }
        }

        fn fail_terminate_for(&self, udid: &str) {
            self.fail_terminate.lock().insert(udid.to_string());
        }

        fn block_terminate_for(&self, udid: &str) {
            *self.blocked_terminate.lock() = Some(udid.to_string());
        }

        async fn wait_terminate_started(&self) {
            loop {
                let notified = self.terminate_started_notify.notified();
                tokio::pin!(notified);
                notified.as_mut().enable();
                if self.terminate_started.load(Ordering::Acquire) {
                    return;
                }
                notified.await;
            }
        }

        fn release_terminate(&self) {
            self.terminate_release.notify_waiters();
        }

        fn block_process_inspection(&self) {
            self.block_process_read.store(true, Ordering::Release);
        }

        async fn wait_process_inspection_started(&self) {
            loop {
                let notified = self.process_read_started_notify.notified();
                tokio::pin!(notified);
                notified.as_mut().enable();
                if self.process_read_started.load(Ordering::Acquire) {
                    return;
                }
                notified.await;
            }
        }

        fn release_process_inspection(&self) {
            self.process_read_release.notify_waiters();
        }

        fn set_process(&self, udid: &str, bundle_id: &str, pid: Option<u64>) {
            let key = (udid.to_string(), bundle_id.to_string());
            let mut processes = self.processes.lock();
            if let Some(pid) = pid {
                processes.insert(key, pid);
            } else {
                processes.remove(&key);
            }
        }

        fn set_active_bundle(&self, bundle_id: &str) {
            *self.active_bundle.lock() = bundle_id.to_string();
        }

        fn set_session_text(&self, value: &str) {
            *self.session_text.lock() = Some(value.to_string());
        }
    }

    #[async_trait]
    impl DeviceDriver for RuntimeDriver {
        fn cached_agent_status(&self, udid: &str) -> AgentStatus {
            AgentStatus {
                udid: udid.to_string(),
                state: AgentState::Ready,
                artifact_id: "fixture-agent".to_string(),
                artifact_version: "1".to_string(),
                bundle_id: "com.fixture.agent".to_string(),
                protocol_version: 2,
                features: vec!["stream".into(), "tap".into(), "swipe".into(), "text".into()],
                installed_version: Some("1".to_string()),
                installed_build: Some("1".to_string()),
                auth_ready: true,
                mjpeg_ready: false,
                session_ready: false,
                message: None,
            }
        }

        async fn inspect_device_for_target(
            &self,
            _udid: &str,
            target_bundle_id: &str,
        ) -> anyhow::Result<DeviceCapabilitySnapshot> {
            Ok(capability_snapshot(target_bundle_id))
        }

        fn supports_verified_app_termination(&self, _udid: &str) -> bool {
            true
        }

        async fn inspect_app_process(
            &self,
            udid: &str,
            bundle_id: &str,
        ) -> anyhow::Result<AppProcessState> {
            self.process_inspection_calls.fetch_add(1, Ordering::SeqCst);
            if self.block_process_read.load(Ordering::Acquire) {
                self.process_read_started.store(true, Ordering::Release);
                self.process_read_started_notify.notify_waiters();
                self.process_read_release.notified().await;
            }
            let pid = self
                .processes
                .lock()
                .get(&(udid.to_string(), bundle_id.to_string()))
                .copied();
            Ok(AppProcessState {
                bundle_id: bundle_id.to_string(),
                pid,
                running: pid.is_some(),
            })
        }

        async fn list_devices(&self) -> anyhow::Result<Vec<DeviceInfo>> {
            Ok(Vec::new())
        }

        async fn refresh_device(&self, udid: &str) -> anyhow::Result<DeviceInfo> {
            Ok(device(udid, ConnectionKind::Mock, DeviceStatus::Ready))
        }

        async fn install_app(&self, _udid: &str, _path: &Path) -> anyhow::Result<()> {
            Ok(())
        }

        async fn uninstall_app(&self, _udid: &str, _bundle_id: &str) -> anyhow::Result<()> {
            Ok(())
        }

        async fn screenshot(&self, _udid: &str, dest: &Path) -> anyhow::Result<PathBuf> {
            Ok(dest.to_path_buf())
        }

        async fn syslog_tail(&self, _udid: &str, _lines: usize) -> anyhow::Result<String> {
            Ok(String::new())
        }

        async fn launch_app(&self, _udid: &str, _bundle_id: &str) -> anyhow::Result<()> {
            self.launch_calls.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }

        async fn read_active_app_bundle(&self, _udid: &str) -> anyhow::Result<String> {
            self.active_app_reads.fetch_add(1, Ordering::SeqCst);
            Ok(self.active_bundle.lock().clone())
        }

        async fn confirm_interaction_stream_stopped(
            &self,
            _udid: &str,
        ) -> anyhow::Result<StreamHandoffProof> {
            Ok(StreamHandoffProof { generation: 1 })
        }

        async fn start_interaction_session(
            &self,
            _udid: &str,
            _bundle_id: &str,
            _kind: crate::InteractionSessionKind,
        ) -> anyhow::Result<Box<dyn UiSession>> {
            self.session_start_calls.fetch_add(1, Ordering::SeqCst);
            if self.session_text.lock().is_none() {
                anyhow::bail!("fixture session attach should not be reached");
            }
            Ok(Box::new(RuntimeSession {
                text: self.session_text.clone(),
                read_text_calls: self.read_text_calls.clone(),
                type_text_calls: self.type_text_calls.clone(),
            }))
        }

        async fn terminate_app(
            &self,
            udid: &str,
            bundle_id: &str,
        ) -> anyhow::Result<ProcessAbsenceProof> {
            self.terminate_calls.fetch_add(1, Ordering::SeqCst);
            if self.blocked_terminate.lock().as_deref() == Some(udid) {
                let released = self.terminate_release.notified();
                tokio::pin!(released);
                released.as_mut().enable();
                self.terminate_started.store(true, Ordering::Release);
                self.terminate_started_notify.notify_waiters();
                released.await;
            }
            if self.fail_terminate.lock().contains(udid) {
                anyhow::bail!("fixture terminate failed for {udid}");
            }
            let old_pid = self
                .processes
                .lock()
                .remove(&(udid.to_string(), bundle_id.to_string()));
            Ok(ProcessAbsenceProof {
                bundle_id: bundle_id.to_string(),
                old_pid,
            })
        }

        async fn reboot(&self, _udid: &str) -> anyhow::Result<()> {
            Ok(())
        }

        async fn start_ui_session(&self, _udid: &str) -> anyhow::Result<Box<dyn UiSession>> {
            anyhow::bail!("runtime bridge fixture has no UI session")
        }

        async fn ensure_stream(&self, _udid: &str) -> anyhow::Result<String> {
            anyhow::bail!("runtime bridge fixture has no stream")
        }

        async fn prepare_device(&self, _udid: &str) -> anyhow::Result<()> {
            Ok(())
        }
    }

    struct RuntimeSession {
        text: Arc<Mutex<Option<String>>>,
        read_text_calls: Arc<AtomicUsize>,
        type_text_calls: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl UiSession for RuntimeSession {
        async fn tap(&self, _point: TapPoint) -> anyhow::Result<()> {
            Ok(())
        }

        async fn swipe(&self, _gesture: SwipeGesture) -> anyhow::Result<()> {
            Ok(())
        }

        async fn type_text(&self, _text: &str) -> anyhow::Result<()> {
            self.type_text_calls.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }

        fn supports_text_input(&self) -> bool {
            true
        }

        async fn home(&self) -> anyhow::Result<()> {
            Ok(())
        }

        async fn find_and_tap(&self, _accessibility_id: &str) -> anyhow::Result<()> {
            Ok(())
        }

        async fn assert_visible(&self, _accessibility_id: &str) -> anyhow::Result<()> {
            Ok(())
        }

        async fn read_text(
            &self,
            _locator: &QualifiedElementLocator,
            _request_timeout: Duration,
        ) -> anyhow::Result<String> {
            self.read_text_calls.fetch_add(1, Ordering::SeqCst);
            self.text.lock().clone().context("fixture text is absent")
        }

        fn supports_accessibility_readback(&self) -> bool {
            true
        }

        fn stream_url(&self) -> Option<String> {
            Some("http://fixture/stream".to_string())
        }
    }

    struct EmptyFrames;
    struct EmptyFrameStream;
    struct EmptyGenerationStream;

    #[async_trait]
    impl FrameStream for EmptyFrameStream {
        async fn next(&mut self) -> Option<Frame> {
            None
        }
    }

    impl FrameSource for EmptyFrames {
        fn subscribe(&self, _udid: &str) -> Box<dyn FrameStream> {
            Box::new(EmptyFrameStream)
        }

        fn latest(&self, _udid: &str) -> Option<Frame> {
            None
        }
    }

    #[async_trait]
    impl GenerationFrameStream for EmptyGenerationStream {
        async fn next(&mut self) -> GenerationFrameEvent {
            GenerationFrameEvent::Closed
        }
    }

    impl GenerationFrameSource for EmptyFrames {
        fn subscribe_generation(
            &self,
            _udid: &str,
            _generation: u64,
        ) -> Box<dyn GenerationFrameStream> {
            Box::new(EmptyGenerationStream)
        }

        fn latest_in_generation(&self, _udid: &str, _generation: u64) -> Option<GenerationFrame> {
            None
        }
    }

    #[derive(Default)]
    struct GenerationAdvanceFrames {
        generation_reads: AtomicUsize,
    }

    impl FrameSource for GenerationAdvanceFrames {
        fn subscribe(&self, _udid: &str) -> Box<dyn FrameStream> {
            Box::new(EmptyFrameStream)
        }

        fn latest(&self, _udid: &str) -> Option<Frame> {
            None
        }
    }

    struct AdvancedGenerationStream;

    #[async_trait]
    impl GenerationFrameStream for AdvancedGenerationStream {
        async fn next(&mut self) -> GenerationFrameEvent {
            GenerationFrameEvent::Advanced {
                expected: 1,
                actual: 2,
            }
        }
    }

    impl GenerationFrameSource for GenerationAdvanceFrames {
        fn subscribe_generation(
            &self,
            _udid: &str,
            _generation: u64,
        ) -> Box<dyn GenerationFrameStream> {
            self.generation_reads.fetch_add(1, Ordering::SeqCst);
            Box::new(AdvancedGenerationStream)
        }

        fn latest_in_generation(&self, _udid: &str, _generation: u64) -> Option<GenerationFrame> {
            self.generation_reads.fetch_add(1, Ordering::SeqCst);
            None
        }
    }

    struct ReusedGenerationFrames {
        generation_reads: AtomicUsize,
        before: Frame,
        after: Frame,
    }

    impl ReusedGenerationFrames {
        fn new() -> Self {
            Self {
                generation_reads: AtomicUsize::new(0),
                before: encoded_jpeg([0, 0, 0]),
                after: encoded_jpeg([255, 255, 255]),
            }
        }
    }

    impl FrameSource for ReusedGenerationFrames {
        fn subscribe(&self, _udid: &str) -> Box<dyn FrameStream> {
            Box::new(EmptyFrameStream)
        }

        fn latest(&self, _udid: &str) -> Option<Frame> {
            Some(self.before.clone())
        }
    }

    struct ReusedGenerationStream {
        frame: Option<Frame>,
    }

    #[async_trait]
    impl GenerationFrameStream for ReusedGenerationStream {
        async fn next(&mut self) -> GenerationFrameEvent {
            self.frame
                .take()
                .map_or(GenerationFrameEvent::Closed, |bytes| {
                    GenerationFrameEvent::Frame(GenerationFrame {
                        generation: 1,
                        sequence: 2,
                        bytes,
                    })
                })
        }
    }

    impl GenerationFrameSource for ReusedGenerationFrames {
        fn subscribe_generation(
            &self,
            _udid: &str,
            _generation: u64,
        ) -> Box<dyn GenerationFrameStream> {
            self.generation_reads.fetch_add(1, Ordering::SeqCst);
            Box::new(ReusedGenerationStream {
                frame: Some(self.after.clone()),
            })
        }

        fn latest_in_generation(&self, _udid: &str, generation: u64) -> Option<GenerationFrame> {
            self.generation_reads.fetch_add(1, Ordering::SeqCst);
            (generation == 1).then(|| GenerationFrame {
                generation,
                sequence: 1,
                bytes: self.before.clone(),
            })
        }
    }

    fn encoded_jpeg(rgb: [u8; 3]) -> Frame {
        let image = RgbImage::from_pixel(4, 4, image::Rgb(rgb));
        let mut bytes = Vec::new();
        image::codecs::jpeg::JpegEncoder::new(&mut bytes)
            .encode_image(&image)
            .expect("encode fixture JPEG");
        Arc::new(bytes)
    }

    const TARGET: &str = "com.apple.Preferences";

    fn terminate_plan() -> CompiledFlowPlanV2 {
        let node = CompiledFlowNode {
            id: Uuid::new_v4(),
            kind: ActionKind::TerminateApp,
            config: CompiledActionConfig::TerminateApp {
                bundle_id: TARGET.to_string(),
            },
            postcondition: Some(EvidenceSpec::ProcessAbsent {
                bundle_id: TARGET.to_string(),
            }),
        };
        compiled_plan(
            vec![node],
            ContextPlan {
                requires_exclusive: true,
                requires_ui_session: false,
                requires_stream: false,
                requires_fresh_text_session: false,
                initial_bundle_id: None,
            },
            &["app.terminate"],
        )
    }

    fn launch_plan() -> CompiledFlowPlanV2 {
        compiled_plan(
            vec![CompiledFlowNode {
                id: Uuid::new_v4(),
                kind: ActionKind::LaunchApp,
                config: CompiledActionConfig::LaunchApp {
                    bundle_id: TARGET.to_string(),
                },
                postcondition: Some(EvidenceSpec::ActiveAppEquals {
                    bundle_id: TARGET.to_string(),
                }),
            }],
            ContextPlan {
                requires_exclusive: true,
                requires_ui_session: true,
                requires_stream: false,
                requires_fresh_text_session: false,
                initial_bundle_id: Some(TARGET.to_string()),
            },
            &["app.launch"],
        )
    }

    fn launch_wait_plan() -> CompiledFlowPlanV2 {
        compiled_plan(
            vec![
                CompiledFlowNode {
                    id: Uuid::new_v4(),
                    kind: ActionKind::LaunchApp,
                    config: CompiledActionConfig::LaunchApp {
                        bundle_id: TARGET.to_string(),
                    },
                    postcondition: Some(EvidenceSpec::ActiveAppEquals {
                        bundle_id: TARGET.to_string(),
                    }),
                },
                CompiledFlowNode {
                    id: Uuid::new_v4(),
                    kind: ActionKind::Wait,
                    config: CompiledActionConfig::Wait { duration_ms: 1 },
                    postcondition: None,
                },
            ],
            ContextPlan {
                requires_exclusive: true,
                requires_ui_session: true,
                requires_stream: false,
                requires_fresh_text_session: false,
                initial_bundle_id: Some(TARGET.to_string()),
            },
            &["app.launch"],
        )
    }

    fn wait_plan(duration_ms: u64) -> CompiledFlowPlanV2 {
        let nodes = vec![
            CompiledFlowNode {
                id: Uuid::new_v4(),
                kind: ActionKind::Start,
                config: CompiledActionConfig::Empty,
                postcondition: None,
            },
            CompiledFlowNode {
                id: Uuid::new_v4(),
                kind: ActionKind::Wait,
                config: CompiledActionConfig::Wait { duration_ms },
                postcondition: None,
            },
            CompiledFlowNode {
                id: Uuid::new_v4(),
                kind: ActionKind::End,
                config: CompiledActionConfig::Empty,
                postcondition: None,
            },
        ];
        compiled_plan(
            nodes,
            ContextPlan {
                requires_exclusive: false,
                requires_ui_session: false,
                requires_stream: false,
                requires_fresh_text_session: false,
                initial_bundle_id: None,
            },
            &[],
        )
    }

    fn terminate_wait_plan() -> CompiledFlowPlanV2 {
        let nodes = vec![
            CompiledFlowNode {
                id: Uuid::new_v4(),
                kind: ActionKind::TerminateApp,
                config: CompiledActionConfig::TerminateApp {
                    bundle_id: TARGET.to_string(),
                },
                postcondition: Some(EvidenceSpec::ProcessAbsent {
                    bundle_id: TARGET.to_string(),
                }),
            },
            CompiledFlowNode {
                id: Uuid::new_v4(),
                kind: ActionKind::Wait,
                config: CompiledActionConfig::Wait { duration_ms: 1 },
                postcondition: None,
            },
            CompiledFlowNode {
                id: Uuid::new_v4(),
                kind: ActionKind::End,
                config: CompiledActionConfig::Empty,
                postcondition: None,
            },
        ];
        compiled_plan(
            nodes,
            ContextPlan {
                requires_exclusive: true,
                requires_ui_session: false,
                requires_stream: false,
                requires_fresh_text_session: false,
                initial_bundle_id: None,
            },
            &["app.terminate"],
        )
    }

    fn single_wait_plan() -> CompiledFlowPlanV2 {
        compiled_plan(
            vec![CompiledFlowNode {
                id: Uuid::new_v4(),
                kind: ActionKind::Wait,
                config: CompiledActionConfig::Wait { duration_ms: 1 },
                postcondition: None,
            }],
            ContextPlan {
                requires_exclusive: false,
                requires_ui_session: false,
                requires_stream: false,
                requires_fresh_text_session: false,
                initial_bundle_id: None,
            },
            &[],
        )
    }

    fn ambiguous_plan(kind: ActionKind) -> CompiledFlowPlanV2 {
        let coordinate = ImageCoordinateTarget {
            x: 1.0,
            y: 1.0,
            image_width: 4,
            image_height: 4,
            orientation: ScreenOrientation::Portrait,
            profile_id: "d".repeat(64),
        };
        let locator = QualifiedElementLocator {
            strategy: crate::ElementLocatorStrategy::AccessibilityId,
            value: "SearchField".to_string(),
        };
        let node = match kind {
            ActionKind::Tap => CompiledFlowNode {
                id: Uuid::new_v4(),
                kind,
                config: CompiledActionConfig::Tap {
                    target: CompiledTapTarget::Point {
                        target: coordinate.clone(),
                    },
                },
                postcondition: Some(EvidenceSpec::FrameRegionChanged {
                    x: 0,
                    y: 0,
                    width: 2,
                    height: 2,
                    minimum_distance: 1,
                }),
            },
            ActionKind::Swipe => CompiledFlowNode {
                id: Uuid::new_v4(),
                kind,
                config: CompiledActionConfig::Swipe {
                    from: coordinate.clone(),
                    to: ImageCoordinateTarget {
                        x: 2.0,
                        y: 2.0,
                        ..coordinate
                    },
                    duration_ms: 100,
                },
                postcondition: Some(EvidenceSpec::FrameDigestChanged {
                    minimum_distance: 1,
                }),
            },
            ActionKind::TypeText => CompiledFlowNode {
                id: Uuid::new_v4(),
                kind,
                config: CompiledActionConfig::TypeText {
                    text: "fixture".to_string(),
                    read_back_locator: locator.clone(),
                },
                postcondition: Some(EvidenceSpec::TextReadBackEquals {
                    locator,
                    value: "fixture".to_string(),
                }),
            },
            _ => unreachable!("ambiguous fixture action"),
        };
        compiled_plan(
            vec![node],
            ContextPlan {
                requires_exclusive: true,
                requires_ui_session: false,
                requires_stream: false,
                requires_fresh_text_session: false,
                initial_bundle_id: Some(TARGET.to_string()),
            },
            &[],
        )
    }

    fn type_text_plan() -> CompiledFlowPlanV2 {
        let locator = QualifiedElementLocator {
            strategy: crate::ElementLocatorStrategy::AccessibilityId,
            value: "SearchField".to_string(),
        };
        compiled_plan(
            vec![CompiledFlowNode {
                id: Uuid::new_v4(),
                kind: ActionKind::TypeText,
                config: CompiledActionConfig::TypeText {
                    text: "fixture".to_string(),
                    read_back_locator: locator.clone(),
                },
                postcondition: Some(EvidenceSpec::TextReadBackEquals {
                    locator,
                    value: "fixture".to_string(),
                }),
            }],
            ContextPlan {
                requires_exclusive: true,
                requires_ui_session: true,
                requires_stream: true,
                requires_fresh_text_session: true,
                initial_bundle_id: Some(TARGET.to_string()),
            },
            &["ui.text", "stream", "accessibility.readText"],
        )
    }

    fn screenshot_plan() -> CompiledFlowPlanV2 {
        compiled_plan(
            vec![CompiledFlowNode {
                id: Uuid::new_v4(),
                kind: ActionKind::Screenshot,
                config: CompiledActionConfig::Screenshot {
                    label: "capture".to_string(),
                    format: "png".to_string(),
                },
                postcondition: Some(EvidenceSpec::ArtifactDecodedAndHashed),
            }],
            ContextPlan {
                requires_exclusive: true,
                requires_ui_session: true,
                requires_stream: true,
                requires_fresh_text_session: false,
                initial_bundle_id: Some(TARGET.to_string()),
            },
            &["stream", "screenshot"],
        )
    }

    fn encoded_png() -> Vec<u8> {
        let image =
            image::DynamicImage::ImageRgb8(RgbImage::from_pixel(4, 4, image::Rgb([12, 34, 56])));
        let mut output = std::io::Cursor::new(Vec::new());
        image
            .write_to(&mut output, image::ImageFormat::Png)
            .expect("encode fixture PNG");
        output.into_inner()
    }

    fn compiled_plan(
        nodes: Vec<CompiledFlowNode>,
        context_plan: ContextPlan,
        capabilities: &[&str],
    ) -> CompiledFlowPlanV2 {
        let flow_id = Uuid::new_v4();
        let execution_order = nodes.iter().map(|node| node.id).collect::<Vec<_>>();
        let action_definition_versions = nodes
            .iter()
            .map(|node| (node.kind, 1))
            .collect::<BTreeMap<_, _>>();
        CompiledFlowPlanV2 {
            schema_version: FLOW_SCHEMA_VERSION,
            flow_id,
            revision: 1,
            nodes: nodes.into_iter().map(|node| (node.id, node)).collect(),
            execution_order,
            successors: Default::default(),
            context_plan,
            action_definition_versions,
            required_capabilities: capabilities
                .iter()
                .map(|capability| (*capability).to_string())
                .collect(),
        }
    }

    fn capability_snapshot(bundle_id: &str) -> DeviceCapabilitySnapshot {
        DeviceCapabilitySnapshot {
            installed_agent: InstalledAgentIdentity {
                bundle_id: "com.fixture.agent".to_string(),
                version: "1".to_string(),
                build: "1".to_string(),
                executable_name: "Fixture".to_string(),
                signer_identity_sha256: "a".repeat(64),
            },
            selected_artifact_sha256: "b".repeat(64),
            agent_version: "1".to_string(),
            protocol_version: 2,
            driver_adapter_version: "fixture".to_string(),
            transport: ActiveTransport::Mock,
            product_type: "fixture".to_string(),
            os_version: "fixture".to_string(),
            target_app: InstalledTargetIdentity {
                bundle_id: bundle_id.to_string(),
                version: "1".to_string(),
                build: "1".to_string(),
            },
            protected_auth_ready: true,
            geometry: Some(QualifiedGeometry {
                logical_width: 375.0,
                logical_height: 667.0,
                pixel_width: 375,
                pixel_height: 667,
                scale_x: 1.0,
                scale_y: 1.0,
                orientation: ScreenOrientation::Portrait,
            }),
        }
    }

    fn target_free_snapshot() -> FlowCapabilitySnapshot {
        FlowCapabilitySnapshot {
            scope: FlowPreflightScope::TargetFree,
            device: None,
            agent_status: None,
            capability_ids: BTreeSet::new(),
        }
    }

    fn registry(devices: &[DeviceInfo]) -> crate::DeviceRegistry {
        let registry = crate::DeviceRegistry::new(crate::EventBus::new(8));
        registry.upsert_many(devices.to_vec());
        registry
    }

    fn device(udid: &str, connection: ConnectionKind, status: DeviceStatus) -> DeviceInfo {
        DeviceInfo {
            udid: udid.to_string(),
            name: udid.to_string(),
            model: "fixture".to_string(),
            platform: crate::DevicePlatform::Ios,
            os_version: "fixture".to_string(),
            connection,
            status,
            battery: None,
            wda_ready: false,
            wda_expires_at: None,
            stream_url: None,
            tile_stream_state: TileStreamState::Parked,
            last_error: None,
        }
    }

    fn attempt(
        state: FlowAttemptState,
        side_effect_class: SideEffectClass,
    ) -> FlowNodeAttemptRecord {
        FlowNodeAttemptRecord {
            id: Uuid::new_v4(),
            device_run_id: Uuid::new_v4(),
            node_id: Uuid::new_v4(),
            action_kind: ActionKind::Wait,
            attempt_no: 1,
            side_effect_class,
            state,
            canonical_input: None,
            evidence_baseline: None,
            evidence_result: None,
            chosen_port: None,
            retry_allowed: false,
            error: None,
            started_at: None,
            updated_at: Utc::now(),
            finished_at: None,
        }
    }
}
