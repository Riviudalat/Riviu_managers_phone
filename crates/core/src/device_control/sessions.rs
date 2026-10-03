//! UI sessions: opening one on top of a lease, bringing the target app forward, and
//! closing both in the right order.

use super::*;

impl DeviceControlPlane {
    /// Persist the final-close obligation before releasing ownership. Never assert process
    /// absence for a deferred close, and always release the session on a persistence failure.
    pub async fn complete_app_session(
        &self,
        context: UiWithStreamContext,
        bundle_id: &str,
    ) -> Result<AppCompletionDisposition, DeviceControlError> {
        self.validate_stream(&context)?;
        let requested = self.request_app_completion(context.udid(), bundle_id);
        if matches!(requested, Ok(false)) {
            return self
                .finish_app_session(context, bundle_id)
                .await
                .map(AppCompletionDisposition::ProcessAbsent);
        }
        let closed = self.close_ui_context(context).await;
        requested?;
        closed?;
        Ok(AppCompletionDisposition::Deferred)
    }

    /// New automation attempt only. Reserve capacity before any stop; keep ownership on errors.
    /// This does not reset a campaign journal or authorize retry after a public effect.
    pub async fn start_clean_app_session(
        &self,
        exclusive: DeviceExclusiveContext,
        capacity: UiCapacityReservation,
        bundle_id: &str,
        kind: InteractionSessionKind,
    ) -> Result<UiWithStreamContext, DeviceControlError> {
        let token = self.validate_interaction_capacity(&exclusive)?;
        if capacity.plane_id != self.plane_id
            || capacity.reservation.as_ref().map(|r| r.token()) != Some(token)
        {
            return Err(DeviceControlError::InvalidContext {
                reason: "clean start capacity does not match its device lease",
            });
        }
        let udid = exclusive.udid().to_owned();
        self.ensure_clean_start_allowed(&udid)?;
        let proof = self.terminate_app(&exclusive, bundle_id).await?;
        if proof.bundle_id != bundle_id {
            return Err(DeviceControlError::InvalidContext {
                reason: "termination proof names another app",
            });
        }
        tracing::info!(udid, bundle_id, old_pid=?proof.old_pid, "automation clean start: target process absent");
        let session = match self
            .try_start_interaction_session(exclusive, bundle_id, kind)
            .await
        {
            Ok(session) => session,
            Err(failure) => {
                let cleanup = self.terminate_app(&failure.context, bundle_id).await;
                let closed = self.close_exclusive_context(failure.context);
                return Err(lifecycle_start_error(
                    &udid,
                    failure.error,
                    cleanup.err(),
                    closed.err(),
                ));
            }
        };
        match self.try_start_reserved_stream(session, capacity).await {
            Ok(context) => Ok(context),
            Err(failure) => {
                let (cleanup, closed) = if let Some(context) = failure.context {
                    let cleanup = self.terminate_session_app(&context, bundle_id).await;
                    (cleanup.err(), self.close_session_context(context).err())
                } else if let Some(context) = failure.failed_start {
                    // The pending stream owns this device until close_failed_stream_start completes.
                    let cleanup = self
                        .driver
                        .terminate_app(&udid, bundle_id)
                        .await
                        .map_err(|error| driver_error(&udid, "terminateFailedStart", error));
                    (
                        cleanup.err(),
                        self.close_failed_stream_start(context).await.err(),
                    )
                } else {
                    (None, None)
                };
                Err(lifecycle_start_error(&udid, failure.error, cleanup, closed))
            }
        }
    }

    /// Capture public-effect evidence before calling; close even when termination fails.
    pub async fn finish_app_session(
        &self,
        context: UiWithStreamContext,
        bundle_id: &str,
    ) -> Result<ProcessAbsenceProof, DeviceControlError> {
        let udid = context.udid().to_owned();
        let stopped = self.terminate_streaming_app(&context, bundle_id).await;
        let closed = self.close_ui_context(context).await;
        match (stopped, closed) {
            (Ok(proof), Ok(_)) if proof.bundle_id == bundle_id => Ok(proof),
            (Ok(_), Ok(_)) => Err(DeviceControlError::InvalidContext {
                reason: "termination proof names another app",
            }),
            (Err(error), Ok(_)) | (Ok(_), Err(error)) => Err(error),
            (Err(stop), Err(close)) => Err(lifecycle_start_error(&udid, stop, None, Some(close))),
        }
    }
    pub async fn foreground_target_app(
        &self,
        context: &DeviceExclusiveContext,
        bundle_id: &str,
    ) -> Result<ForegroundAppProof, DeviceControlError> {
        let lease = self.validate_exclusive(context)?;
        self.driver
            .launch_app(lease.udid(), bundle_id)
            .await
            .map_err(|error| driver_error(lease.udid(), "foregroundTargetApp", error))?;
        self.validate_exclusive(context)?;
        Ok(ForegroundAppProof {
            udid: lease.udid().to_string(),
            bundle_id: bundle_id.to_string(),
        })
    }
    pub async fn start_interaction_session(
        &self,
        context: DeviceExclusiveContext,
        bundle_id: &str,
        kind: InteractionSessionKind,
    ) -> Result<UiSessionContext, DeviceControlError> {
        self.try_start_interaction_session(context, bundle_id, kind)
            .await
            .map_err(|failure| failure.error)
    }
    /// Diagnostic preparation retains ownership when session setup fails after helper setup.
    pub async fn start_interaction_session_or_quarantine(
        &self, context: DeviceExclusiveContext, bundle_id: &str, kind: InteractionSessionKind,
    ) -> Result<UiSessionContext, DeviceControlError> {
        match self.try_start_interaction_session(context, bundle_id, kind).await {
            Ok(session) => Ok(session),
            Err(failure) => { self.quarantined.push_context(failure.context); Err(failure.error) }
        }
    }

    pub async fn start_owned_ui_session(
        &self,
        mut context: DeviceExclusiveContext,
    ) -> Result<UiSessionContext, DeviceControlError> {
        let lease = self.validate_exclusive(&context)?;
        let udid = lease.udid().to_string();
        let session = self
            .driver
            .open_control_session(&udid)
            .await
            .map_err(|error| driver_error(&udid, "openControlSession", error))?;
        self.validate_exclusive(&context)?;
        Ok(UiSessionContext {
            plane_id: self.plane_id,
            lease: context.lease.take(),
            activity: context.activity.take(),
            session: Some(Arc::from(session)),
            ui_capacity_token: None,
            stream_handoff_generation: None,
        })
    }
    /// Exclusive + control session for an operator gesture, without touching
    /// the live preview. Close with [`Self::close_manual_session`] so the
    /// cached iOS session stays in place for the background stream.
    pub async fn open_manual_session(
        &self,
        udid: &str,
        owner: DeviceWorkOwner,
    ) -> Result<UiSessionContext, DeviceControlError> {
        let exclusive = self
            .try_acquire_exclusive_keeping_stream(udid, owner)
            .await?;
        self.start_owned_ui_session(exclusive).await
    }
    pub fn close_manual_session(
        &self,
        mut context: UiSessionContext,
    ) -> Result<ContextReleaseProof, DeviceControlError> {
        let lease = self.validate_session(&context)?;
        let proof = ContextReleaseProof {
            udid: lease.udid().to_string(),
            owner: lease.owner(),
            had_session: true,
            had_stream: false,
        };
        // Do not invalidate: the background stream on iOS still needs the
        // cached WDA session. Dropping the Arc releases the exclusive lease.
        context.session.take();
        context.activity.take();
        context.lease.take();
        Ok(proof)
    }
    pub async fn foreground_target_app_and_start_interaction_session(
        &self,
        context: DeviceExclusiveContext,
        bundle_id: &str,
        kind: InteractionSessionKind,
    ) -> Result<(UiSessionContext, ForegroundAppProof), DeviceControlError> {
        self.try_foreground_target_app_and_start_interaction_session(context, bundle_id, kind)
            .await
            .map_err(|failure| failure.error)
    }
    pub fn session(
        &self,
        context: &UiSessionContext,
    ) -> Result<Arc<dyn UiSession>, DeviceControlError> {
        self.validate_session(context)?;
        context
            .session
            .as_ref()
            .cloned()
            .ok_or(DeviceControlError::InvalidContext {
                reason: "session context has been consumed",
            })
    }
    /// Retain a failed helper-canary lease until explicit reconciliation.
    pub fn quarantine_exclusive_context(&self, context: DeviceExclusiveContext) -> Result<(), DeviceControlError> {
        let same_plane = context.plane_id == self.plane_id;
        if !same_plane { return Err(DeviceControlError::InvalidContext { reason: "quarantine context belongs to another plane" }); }
        self.quarantined.push_context(context);
        Ok(())
    }

    pub fn close_exclusive_context(
        &self,
        mut context: DeviceExclusiveContext,
    ) -> Result<ContextReleaseProof, DeviceControlError> {
        let lease = self.validate_exclusive(&context)?;
        let proof = ContextReleaseProof {
            udid: lease.udid().to_string(),
            owner: lease.owner(),
            had_session: false,
            had_stream: false,
        };
        context.activity.take();
        context.lease.take();
        Ok(proof)
    }
    pub fn close_session_context(
        &self,
        mut context: UiSessionContext,
    ) -> Result<ContextReleaseProof, DeviceControlError> {
        let lease = self.validate_session(&context)?;
        let udid = lease.udid().to_string();
        let proof = ContextReleaseProof {
            udid: udid.clone(),
            owner: lease.owner(),
            had_session: true,
            had_stream: false,
        };
        self.driver.invalidate_ui_session(&udid);
        context.session.take();
        context.activity.take();
        context.lease.take();
        Ok(proof)
    }
    pub async fn foreground_session_app(
        &self,
        context: &UiSessionContext,
        bundle_id: &str,
    ) -> Result<ForegroundAppProof, DeviceControlError> {
        let lease = self.validate_session(context)?;
        self.driver
            .launch_app(lease.udid(), bundle_id)
            .await
            .map_err(|error| driver_error(lease.udid(), "foregroundSessionApp", error))?;
        Ok(ForegroundAppProof {
            udid: lease.udid().to_string(),
            bundle_id: bundle_id.to_string(),
        })
    }
    pub async fn terminate_session_app(
        &self,
        context: &UiSessionContext,
        bundle_id: &str,
    ) -> Result<ProcessAbsenceProof, DeviceControlError> {
        let lease = self.validate_session(context)?;
        self.driver
            .terminate_app(lease.udid(), bundle_id)
            .await
            .map_err(|error| driver_error(lease.udid(), "terminateSessionApp", error))
    }
    pub async fn inspect_session_app_process(
        &self,
        context: &UiSessionContext,
        bundle_id: &str,
    ) -> Result<AppProcessState, DeviceControlError> {
        let lease = self.validate_session(context)?;
        self.driver
            .inspect_app_process(lease.udid(), bundle_id)
            .await
            .map_err(|error| driver_error(lease.udid(), "inspectSessionAppProcess", error))
    }
    pub async fn session_screenshot(
        &self,
        context: &UiSessionContext,
        destination: &Path,
    ) -> Result<PathBuf, DeviceControlError> {
        let lease = self.validate_session(context)?;
        self.driver
            .screenshot(lease.udid(), destination)
            .await
            .map_err(|error| driver_error(lease.udid(), "sessionScreenshot", error))
    }
}

/// Root-supplied ledger evidence is checked by the installed read-only guard on
/// prepare and execute. It is never relabelled as historical session provenance.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InteractionQuarantineRecoveryRequest {
    pub maintenance_id: Uuid,
    pub plane_id: Uuid,
    pub udid: String,
    pub owner: DeviceWorkOwner,
    pub lease_token: Uuid,
    pub stream_generation: u64,
    pub session_epoch: String,
    pub campaign_id: String,
    pub assignment_id: String,
    pub operation_id: String,
    pub revision: i64,
    pub intent_sha256: String,
    pub acknowledge_old_draft_unproved: bool,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InteractionRecoveryStreamStopProof {
    pub old_generation: u64,
    pub new_generation: u64,
    pub child_stopped: bool,
}
impl From<StreamStopProof> for InteractionRecoveryStreamStopProof {
    fn from(proof: StreamStopProof) -> Self {
        Self {
            old_generation: proof.old_generation,
            new_generation: proof.new_generation,
            child_stopped: proof.child_stopped,
        }
    }
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InteractionQuarantineSnapshot {
    pub plane_id: Uuid,
    pub udid: String,
    pub owner: DeviceWorkOwner,
    pub lease_token: Uuid,
    pub stream_generation: Option<u64>,
    pub session_epoch: String,
    pub scope: Option<crate::ui_automation::GuiScope>,
    pub recorded_package: Option<String>,
    pub process_absence: Option<ProcessAbsenceProof>,
    pub stream_stop: Option<InteractionRecoveryStreamStopProof>,
    pub historical_semantic_binding_unproved: bool,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InteractionQuarantineRecoveryPlan {
    pub plan_id: Uuid,
    pub binding: InteractionQuarantineRecoveryRequest,
    pub package: String,
    pub observed_pid: u64,
    pub recorded_package: Option<String>,
    pub historical_semantic_binding_unproved: bool,
    pub cleanup_ready: crate::driver::OwnedSessionCleanupProof,
    pub expires_at_ms: i64,
    pub old_draft_restoration_unproved: bool,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InteractionQuarantineRecoveryReceipt {
    pub plan: InteractionQuarantineRecoveryPlan,
    pub process_absence: ProcessAbsenceProof,
    pub stream_stop: InteractionRecoveryStreamStopProof,
    pub cleanup_ready: crate::driver::OwnedSessionCleanupProof,
    pub released_at_ms: i64,
    pub old_draft_restoration_unproved: bool,
    pub action_assignment_ledger_unchanged: bool,
}

impl DeviceControlPlane {
    pub fn set_interaction_recovery_ledger_guard(
        &self,
        guard: Arc<InteractionRecoveryLedgerGuard>,
    ) {
        *self.recovery_ledger_guard.lock() = Some(guard);
    }

    pub fn interaction_quarantine_snapshot(
        &self,
        udid: &str,
    ) -> Vec<InteractionQuarantineSnapshot> {
        self.quarantined
            .cleanup
            .lock()
            .iter()
            .filter(|ticket| {
                ticket.lease.udid() == udid && ticket.lease.owner() == DeviceWorkOwner::Interaction
            })
            .map(|ticket| InteractionQuarantineSnapshot {
                plane_id: self.plane_id,
                udid: udid.to_owned(),
                owner: ticket.lease.owner(),
                lease_token: ticket.lease.token(),
                stream_generation: ticket.expected_generation,
                session_epoch: ticket.session.gui_session_epoch(),
                scope: ticket.session.gui_scope(),
                recorded_package: ticket.recorded_package.clone(),
                process_absence: ticket.recovery_process.clone(),
                stream_stop: ticket.recovery_stop.map(Into::into),
                historical_semantic_binding_unproved: true,
            })
            .collect()
    }

    /// Explicit maintenance preparation seals new input, but dispatches no phone
    /// writes. The legacy draft and semantic provenance remain unknown.
    pub async fn prepare_interaction_quarantine_recovery(
        &self,
        request: InteractionQuarantineRecoveryRequest,
    ) -> Result<InteractionQuarantineRecoveryPlan, DeviceControlError> {
        let (response, receiver) = oneshot::channel();
        {
            // Admission and enqueue are atomic with shutdown's final quarantine
            // check. The worker owns activity through cancellation/drain.
            let _shutdown_guard = self.shutdown_gate.lock().await;
            let activity = self.lifecycle.register_retained_maintenance()?;
            self.cleanup_tx
                .send(WorkerCommand::PrepareInteractionRecovery {
                    plane_id: self.plane_id,
                    activity,
                    request,
                    response,
                })
                .map_err(|_| DeviceControlError::CleanupWorkerClosed)?;
        }
        receiver
            .await
            .map_err(|_| DeviceControlError::CleanupWorkerClosed)?
    }

    /// Confirmation can name only an immutable prepared plan, not a package or
    /// replacement binding. Failure/cancellation leaves the retained ticket owned.
    pub async fn execute_interaction_quarantine_recovery(
        &self,
        plan_id: Uuid,
        confirmed: bool,
    ) -> Result<InteractionQuarantineRecoveryReceipt, DeviceControlError> {
        let (response, receiver) = oneshot::channel();
        {
            let _shutdown_guard = self.shutdown_gate.lock().await;
            let activity = self.lifecycle.register_retained_maintenance()?;
            if !confirmed {
                return Err(recovery_error("explicit operator confirmation required"));
            }
            if let Some(receipt) = self.interaction_quarantine_recovery_receipt(plan_id) {
                return Ok(receipt);
            }
            self.cleanup_tx
                .send(WorkerCommand::ExecuteInteractionRecovery {
                    plane_id: self.plane_id,
                    activity,
                    plan_id,
                    response,
                })
                .map_err(|_| DeviceControlError::CleanupWorkerClosed)?;
        }
        receiver
            .await
            .map_err(|_| DeviceControlError::CleanupWorkerClosed)?
    }

    pub fn interaction_quarantine_recovery_receipt(
        &self,
        plan_id: Uuid,
    ) -> Option<InteractionQuarantineRecoveryReceipt> {
        self.quarantined
            .recovery_receipts
            .lock()
            .get(&plan_id)
            .cloned()
    }
}

pub(super) fn recovery_error(reason: &str) -> DeviceControlError {
    DeviceControlError::QuarantineRecovery {
        reason: reason.to_owned(),
    }
}

/// The worker exclusively borrows a quarantined ticket; unwinding or any early
/// exit restores it. A session Arc count is deliberately not used as drain proof.
struct RetainedRecoveryTicket {
    ticket: Option<DeviceCleanupTicket>,
    store: Arc<QuarantineStore>,
}
impl Drop for RetainedRecoveryTicket {
    fn drop(&mut self) {
        if let Some(ticket) = self.ticket.take() {
            self.store.push_cleanup(ticket);
        }
    }
}
fn take_recovery_ticket(
    store: &Arc<QuarantineStore>,
    binding: &InteractionQuarantineRecoveryRequest,
) -> Result<RetainedRecoveryTicket, DeviceControlError> {
    let mut tickets = store.cleanup.lock();
    let index = tickets
        .iter()
        .position(|t| t.lease.udid() == binding.udid && t.lease.token() == binding.lease_token)
        .ok_or_else(|| recovery_error("exact retained ticket missing or already in recovery"))?;
    Ok(RetainedRecoveryTicket {
        ticket: Some(tickets.remove(index)),
        store: store.clone(),
    })
}

fn validate_recovery_ticket(
    ticket: &DeviceCleanupTicket,
    work: &DeviceWorkCoordinator,
    streams: &StreamBudgetManager,
    plane_id: Uuid,
    binding: &InteractionQuarantineRecoveryRequest,
) -> Result<(), DeviceControlError> {
    if binding.maintenance_id.is_nil()
        || plane_id != binding.plane_id
        || binding.owner != DeviceWorkOwner::Interaction
        || ticket.lease.udid() != binding.udid
        || ticket.lease.owner() != binding.owner
        || ticket.lease.token() != binding.lease_token
        || work.validate_token(&binding.udid, binding.lease_token).ok() != Some(binding.owner)
        || ticket.reservation.udid() != binding.udid
        || ticket.reservation.owner() != binding.owner
        || streams
            .reservation_udid(ticket.reservation.token())
            .as_deref()
            != Some(binding.udid.as_str())
        || ticket.expected_generation != Some(binding.stream_generation)
        || ticket.session.gui_session_epoch() != binding.session_epoch
        || binding.session_epoch.is_empty()
        || binding.operation_id.is_empty()
        || binding.revision < 0
        || !binding.acknowledge_old_draft_unproved
        || binding.intent_sha256.len() != 64
        || !binding.intent_sha256.bytes().all(|c| c.is_ascii_hexdigit())
    {
        return Err(recovery_error(
            "retained lease/session/generation or ledger evidence binding invalid",
        ));
    }
    let scope = ticket
        .session
        .gui_scope()
        .ok_or_else(|| recovery_error("retained GuiScope missing"))?;
    if scope.device_id != binding.udid
        || scope.run_id != binding.campaign_id
        || scope.assignment_id.as_deref() != Some(binding.assignment_id.as_str())
        || binding.campaign_id.is_empty()
        || binding.assignment_id.is_empty()
    {
        return Err(recovery_error(
            "retained campaign/assignment scope mismatch",
        ));
    }
    Ok(())
}

async fn check_ledger(
    guard: &Option<Arc<InteractionRecoveryLedgerGuard>>,
    binding: &InteractionQuarantineRecoveryRequest,
) -> Result<(), DeviceControlError> {
    let guard = guard.as_ref().ok_or_else(|| {
        recovery_error("persistence/active-campaign evidence guard not installed")
    })?;
    guard(binding.clone())
        .await
        .map_err(|reason| DeviceControlError::QuarantineRecovery { reason })
}

async fn cleanup_proof(
    ticket: &DeviceCleanupTicket,
    binding: &InteractionQuarantineRecoveryRequest,
) -> Result<crate::driver::OwnedSessionCleanupProof, DeviceControlError> {
    let proof = ticket
        .session
        .verify_owned_cleanup_ready(&binding.udid, &binding.session_epoch)
        .await
        .map_err(|error| driver_error(&binding.udid, "verifyOwnedCleanupReady", error))?;
    if proof.udid != binding.udid
        || proof.session_epoch != binding.session_epoch
        || !proof.input_sealed
        || proof.clipboard_pending
        || proof.baseline_pending
        || proof.helper_owner_id.is_empty()
        || proof.helper_instance.is_empty()
        || proof.helper_generation.is_empty()
    {
        return Err(recovery_error("owned session cleanup proof invalid"));
    }
    Ok(proof)
}

fn recovery_not_cancelled<T>(response: &oneshot::Sender<T>) -> Result<(), DeviceControlError> {
    if response.is_closed() {
        Err(recovery_error(
            "operator recovery cancelled; ticket retained",
        ))
    } else {
        Ok(())
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn prepare_recovery(
    driver: &Arc<dyn DeviceDriver>,
    work: &Arc<DeviceWorkCoordinator>,
    streams: &Arc<StreamBudgetManager>,
    store: &Arc<QuarantineStore>,
    plane_id: Uuid,
    binding: InteractionQuarantineRecoveryRequest,
    ledger_guard: Option<Arc<InteractionRecoveryLedgerGuard>>,
    response: oneshot::Sender<Result<InteractionQuarantineRecoveryPlan, DeviceControlError>>,
) {
    let outcome = async {
        recovery_not_cancelled(&response)?;
        check_ledger(&ledger_guard, &binding).await?;
        let retained = take_recovery_ticket(store, &binding)?;
        let ticket = retained.ticket.as_ref().expect("retained recovery ticket");
        validate_recovery_ticket(ticket, work, streams, plane_id, &binding)?;
        let cleanup_ready = cleanup_proof(ticket, &binding).await?;
        if !driver.supports_verified_app_termination(&binding.udid) {
            return Err(recovery_error("verified ProcessControl unsupported"));
        }
        let package = if let Some(proof) = &ticket.recovery_process {
            proof.bundle_id.clone()
        } else {
            driver
                .read_active_app_bundle(&binding.udid)
                .await
                .map_err(|error| driver_error(&binding.udid, "observeRecoveryPackage", error))?
        };
        if !matches!(
            package.as_str(),
            "com.ss.android.ugc.trill" | "com.zhiliaoapp.musically"
        ) {
            return Err(recovery_error(
                "current foreground is not a supported TikTok package",
            ));
        }
        if ticket
            .recorded_package
            .as_ref()
            .is_some_and(|recorded| recorded != &package)
        {
            return Err(recovery_error(
                "observed package differs from recorded quarantine package",
            ));
        }
        let process = driver
            .inspect_app_process(&binding.udid, &package)
            .await
            .map_err(|error| driver_error(&binding.udid, "inspectRecoveryProcess", error))?;
        let observed_pid = if let Some(proof) = &ticket.recovery_process {
            if process.bundle_id != package || process.running || process.pid.is_some() {
                return Err(recovery_error(
                    "previously stopped package was replaced; new maintenance required",
                ));
            }
            proof
                .old_pid
                .filter(|pid| *pid > 0)
                .ok_or_else(|| recovery_error("prior exact PID missing"))?
        } else {
            if process.bundle_id != package || !process.running {
                return Err(recovery_error("current TikTok process identity not proved"));
            }
            process
                .pid
                .filter(|pid| *pid > 0)
                .ok_or_else(|| recovery_error("current TikTok PID missing"))?
        };
        validate_recovery_ticket(ticket, work, streams, plane_id, &binding)?;
        recovery_not_cancelled(&response)?;
        let plan = InteractionQuarantineRecoveryPlan {
            plan_id: Uuid::new_v4(),
            binding,
            package,
            observed_pid,
            cleanup_ready,
            recorded_package: ticket.recorded_package.clone(),
            historical_semantic_binding_unproved: true,
            expires_at_ms: chrono::Utc::now().timestamp_millis() + 120_000,
            old_draft_restoration_unproved: true,
        };
        let mut plans = store.recovery_plans.lock();
        plans.retain(|_, existing| {
            existing.expires_at_ms > chrono::Utc::now().timestamp_millis()
                && existing.binding.lease_token != plan.binding.lease_token
        });
        plans.insert(plan.plan_id, plan.clone());
        Ok(plan)
    }
    .await;
    let _ = response.send(outcome);
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn execute_recovery(
    driver: &Arc<dyn DeviceDriver>,
    work: &Arc<DeviceWorkCoordinator>,
    streams: &Arc<StreamBudgetManager>,
    store: &Arc<QuarantineStore>,
    plane_id: Uuid,
    plan_id: Uuid,
    ledger_guard: Option<Arc<InteractionRecoveryLedgerGuard>>,
    response: oneshot::Sender<Result<InteractionQuarantineRecoveryReceipt, DeviceControlError>>,
) {
    let outcome = async {
        if let Some(receipt) = store.recovery_receipts.lock().get(&plan_id).cloned() {
            return Ok(receipt);
        }
        let plan = store
            .recovery_plans
            .lock()
            .get(&plan_id)
            .cloned()
            .ok_or_else(|| recovery_error("unknown prepared recovery plan"))?;
        if plan.expires_at_ms <= chrono::Utc::now().timestamp_millis() {
            return Err(recovery_error("recovery confirmation expired"));
        }
        recovery_not_cancelled(&response)?;
        check_ledger(&ledger_guard, &plan.binding).await?;
        let mut retained = take_recovery_ticket(store, &plan.binding)?;
        let ticket = retained.ticket.as_mut().expect("retained recovery ticket");
        validate_recovery_ticket(ticket, work, streams, plane_id, &plan.binding)?;
        if cleanup_proof(ticket, &plan.binding).await? != plan.cleanup_ready {
            return Err(recovery_error("prepared helper cleanup identity changed"));
        }
        recovery_not_cancelled(&response)?;
        if !driver.supports_verified_app_termination(&plan.binding.udid) {
            return Err(recovery_error("verified ProcessControl unsupported"));
        }
        let process = driver
            .inspect_app_process(&plan.binding.udid, &plan.package)
            .await
            .map_err(|error| driver_error(&plan.binding.udid, "recheckRecoveryProcess", error))?;
        if ticket.recovery_process.is_none() {
            let package = driver
                .read_active_app_bundle(&plan.binding.udid)
                .await
                .map_err(|error| {
                    driver_error(&plan.binding.udid, "recheckRecoveryPackage", error)
                })?;
            if package != plan.package
                || process.bundle_id != plan.package
                || !process.running
                || process.pid != Some(plan.observed_pid)
            {
                return Err(recovery_error("prepared foreground package/PID changed"));
            }
            validate_recovery_ticket(ticket, work, streams, plane_id, &plan.binding)?;
            recovery_not_cancelled(&response)?;
            let proof = driver
                .terminate_app(&plan.binding.udid, &plan.package)
                .await
                .map_err(|error| {
                    driver_error(&plan.binding.udid, "terminateQuarantinedTikTok", error)
                })?;
            if proof.bundle_id != plan.package || proof.old_pid != Some(plan.observed_pid) {
                return Err(recovery_error(
                    "termination did not prove the prepared package/PID",
                ));
            }
            ticket.recovery_process = Some(proof);
        } else {
            let prior = ticket
                .recovery_process
                .as_ref()
                .expect("retained process proof");
            if prior.bundle_id != plan.package
                || prior.old_pid != Some(plan.observed_pid)
                || process.bundle_id != plan.package
                || process.running
                || process.pid.is_some()
            {
                return Err(recovery_error(
                    "terminated TikTok restarted or proof changed; retained ticket not releasable",
                ));
            }
        }
        recovery_not_cancelled(&response)?;
        let absent = driver
            .inspect_app_process(&plan.binding.udid, &plan.package)
            .await
            .map_err(|error| {
                driver_error(&plan.binding.udid, "verifyRecoveryProcessAbsent", error)
            })?;
        if absent.bundle_id != plan.package || absent.running || absent.pid.is_some() {
            return Err(recovery_error("TikTok process absence unknown"));
        }
        let ready = cleanup_proof(ticket, &plan.binding).await?;
        if ready != plan.cleanup_ready {
            return Err(recovery_error("helper cleanup changed after termination"));
        }
        validate_recovery_ticket(ticket, work, streams, plane_id, &plan.binding)?;
        recovery_not_cancelled(&response)?;
        let stop = streams.begin_stop(ticket.reservation.token())?;
        let proof = if let Some(proof) = ticket.recovery_stop {
            proof
        } else {
            let proof = driver
                .stop_owned_stream(&plan.binding.udid)
                .await
                .map_err(|error| {
                    driver_error(&plan.binding.udid, "stopQuarantineOwnedStream", error)
                })?;
            validate_stop_generation(&plan.binding.udid, plan.binding.stream_generation, proof)?;
            ticket.recovery_stop = Some(proof);
            proof
        };
        validate_stop_generation(&plan.binding.udid, plan.binding.stream_generation, proof)?;
        recovery_not_cancelled(&response)?;
        let stopped = driver
            .confirm_interaction_stream_stopped(&plan.binding.udid)
            .await
            .map_err(|error| {
                driver_error(&plan.binding.udid, "confirmQuarantineStreamAbsent", error)
            })?;
        if stopped.generation != proof.new_generation {
            return Err(recovery_error(
                "stopped stream generation changed before release",
            ));
        }
        check_ledger(&ledger_guard, &plan.binding).await?;
        let ready = cleanup_proof(ticket, &plan.binding).await?;
        if ready != plan.cleanup_ready {
            return Err(recovery_error("helper cleanup changed before release"));
        }
        let absent = driver
            .inspect_app_process(&plan.binding.udid, &plan.package)
            .await
            .map_err(|error| {
                driver_error(&plan.binding.udid, "verifyFinalRecoveryAbsence", error)
            })?;
        if absent.bundle_id != plan.package || absent.running || absent.pid.is_some() {
            return Err(recovery_error("TikTok process changed before release"));
        }
        validate_recovery_ticket(ticket, work, streams, plane_id, &plan.binding)?;
        recovery_not_cancelled(&response)?;
        streams.complete_stop(stop, proof)?;
        driver.invalidate_ui_session(&plan.binding.udid);
        let receipt = InteractionQuarantineRecoveryReceipt {
            process_absence: ticket
                .recovery_process
                .clone()
                .expect("verified process absence"),
            stream_stop: proof.into(),
            cleanup_ready: ready,
            plan,
            released_at_ms: chrono::Utc::now().timestamp_millis(),
            old_draft_restoration_unproved: true,
            action_assignment_ledger_unchanged: true,
        };
        drop(retained.ticket.take());
        store
            .recovery_receipts
            .lock()
            .insert(plan_id, receipt.clone());
        store.recovery_plans.lock().remove(&plan_id);
        Ok(receipt)
    }
    .await;
    let _ = response.send(outcome);
}

fn lifecycle_start_error(
    udid: &str,
    primary: DeviceControlError,
    cleanup: Option<DeviceControlError>,
    close: Option<DeviceControlError>,
) -> DeviceControlError {
    DeviceControlError::Driver {
        udid: udid.into(),
        operation: "cleanAppSession",
        message: format!(
            "{primary}{}{}",
            cleanup
                .map(|e| format!("; cleanup: {e}"))
                .unwrap_or_default(),
            close.map(|e| format!("; close: {e}")).unwrap_or_default()
        ),
    }
}
