//! Streams and the capacity budget behind them, plus everything that can only be done
//! while one is running.
//!
//! The clipboard calls live here rather than with the app operations because they need a
//! live UI session, and getting one is the hard part.

use super::*;

impl DeviceControlPlane {
    pub fn reserve_background_stream(
        &self,
        udid: &str,
    ) -> Result<BackgroundStreamLease, DeviceControlError> {
        let _background_guard = self.background_gate.lock();
        self.ensure_running()?;
        let _capacity_guard = self.capacity_gate.try_lock().map_err(|_| {
            DeviceControlError::BackgroundStreamTransitionBusy {
                udid: udid.to_string(),
            }
        })?;
        let lease = self
            .work
            .with_idle_device(udid, || {
                let reserved = self.streams.reserve_background(udid);
                // Record the BackgroundStore entry under the same device
                // metadata lock that reserved the budget. Inserting it after the
                // lock was released let a concurrent park see the budget record
                // without this entry, orphaning the ticket and spuriously
                // failing shutdown with CleanupQuarantined.
                if let Ok(lease) = &reserved {
                    self.backgrounds.insert(lease.udid(), lease.token());
                }
                reserved
            })
            .map_err(
                |current_owner| DeviceControlError::BackgroundStreamBlocked {
                    udid: udid.to_string(),
                    current_owner,
                },
            )??;
        Ok(lease)
    }
    pub async fn start_background_stream(
        &self,
        lease: &BackgroundStreamLease,
    ) -> Result<String, DeviceControlError> {
        let response_rx = {
            let _background_guard = self.background_gate.lock();
            self.ensure_running()?;
            self.validate_background_lease(lease)?;
            let (response_tx, response_rx) = oneshot::channel();
            self.cleanup_tx
                .send(WorkerCommand::StartBackground {
                    ticket: BackgroundCleanupTicket::from_lease(lease),
                    response: response_tx,
                })
                .map_err(|_| DeviceControlError::CleanupWorkerClosed)?;
            response_rx
        };
        let started = response_rx
            .await
            .map_err(|_| DeviceControlError::CleanupWorkerClosed)??;
        Ok(started.into_url())
    }
    pub fn background_stream_is_current(&self, lease: &BackgroundStreamLease) -> bool {
        self.streams.reservation_udid(lease.token()).as_deref() == Some(lease.udid())
    }
    pub fn background_turn_due(
        &self,
        lease: &BackgroundStreamLease,
    ) -> Result<bool, DeviceControlError> {
        self.streams
            .background_turn_due(lease.token())
            .map_err(Into::into)
    }
    pub async fn stop_background_stream(
        &self,
        lease: &BackgroundStreamLease,
    ) -> Result<StreamStopProof, DeviceControlError> {
        self.validate_background_lease(lease)?;
        let (response_tx, response_rx) = oneshot::channel();
        self.cleanup_tx
            .send(WorkerCommand::StopBackground {
                ticket: BackgroundCleanupTicket::from_lease(lease),
                quarantine_on_error: false,
                response: Some(response_tx),
            })
            .map_err(|_| DeviceControlError::CleanupWorkerClosed)?;
        response_rx
            .await
            .map_err(|_| DeviceControlError::CleanupWorkerClosed)?
    }
    /// How many devices may hold a foreground stream at once.
    ///
    /// Exposed for callers that fan out across devices and need to bound themselves:
    /// exhausting this is not a queue, it is a **refusal** — `preview_foreground_victim`
    /// returns `CapacityExhausted` when the budget is full and there is no background
    /// producer to evict. A caller that starts more concurrent work than this does not
    /// run slower, it fails the excess.
    pub fn stream_capacity(&self) -> usize {
        self.streams.configured_limit()
    }
    pub async fn reserve_ui_capacity(
        &self,
        context: DeviceExclusiveContext,
    ) -> Result<(DeviceExclusiveContext, UiCapacityReservation), DeviceControlError> {
        self.try_reserve_ui_capacity(context)
            .await
            .map_err(|failure| failure.error)
    }
    pub fn streaming_session(
        &self,
        context: &UiWithStreamContext,
    ) -> Result<Arc<dyn UiSession>, DeviceControlError> {
        self.validate_stream(context)?;
        context
            .session
            .as_ref()
            .cloned()
            .ok_or(DeviceControlError::InvalidContext {
                reason: "stream context has been consumed",
            })
    }
    #[allow(clippy::too_many_arguments)]
    pub async fn set_streaming_clipboard(
        &self,
        context: &mut UiWithStreamContext,
        agent_bundle_id: &str,
        target_bundle_id: &str,
        final_session_kind: InteractionSessionKind,
        mode: ClipboardAccessMode,
        content_type: String,
        bytes: Vec<u8>,
    ) -> Result<(), DeviceControlError> {
        let output = self
            .guarded_clipboard(
                context,
                agent_bundle_id,
                target_bundle_id,
                final_session_kind,
                mode,
                GuardedClipboardOperation::Set {
                    content_type,
                    bytes,
                },
            )
            .await?;
        if output != GuardedClipboardOutput::Written {
            return Err(DeviceControlError::InvalidContext {
                reason: "guarded clipboard set returned a read result",
            });
        }
        Ok(())
    }
    pub async fn get_streaming_clipboard(
        &self,
        context: &mut UiWithStreamContext,
        agent_bundle_id: &str,
        target_bundle_id: &str,
        final_session_kind: InteractionSessionKind,
        mode: ClipboardAccessMode,
        maximum_decoded_bytes: usize,
    ) -> Result<(String, Vec<u8>), DeviceControlError> {
        match self
            .guarded_clipboard(
                context,
                agent_bundle_id,
                target_bundle_id,
                final_session_kind,
                mode,
                GuardedClipboardOperation::Get {
                    maximum_decoded_bytes,
                },
            )
            .await?
        {
            GuardedClipboardOutput::Value {
                content_type,
                bytes,
            } => Ok((content_type, bytes)),
            GuardedClipboardOutput::Written => Err(DeviceControlError::InvalidContext {
                reason: "guarded clipboard get returned a write result",
            }),
        }
    }
    async fn guarded_clipboard(
        &self,
        context: &mut UiWithStreamContext,
        agent_bundle_id: &str,
        target_bundle_id: &str,
        final_session_kind: InteractionSessionKind,
        mode: ClipboardAccessMode,
        operation: GuardedClipboardOperation,
    ) -> Result<GuardedClipboardOutput, DeviceControlError> {
        match mode {
            ClipboardAccessMode::TargetBackgroundSafe => {
                self.guarded_background_safe_clipboard(
                    context,
                    agent_bundle_id,
                    target_bundle_id,
                    final_session_kind,
                    operation,
                )
                .await
            }
            ClipboardAccessMode::AgentForegroundRequired => {
                self.submit_agent_foreground_clipboard(
                    context,
                    agent_bundle_id,
                    target_bundle_id,
                    final_session_kind,
                    operation,
                )
                .await
            }
        }
    }
    async fn guarded_background_safe_clipboard(
        &self,
        context: &mut UiWithStreamContext,
        agent_bundle_id: &str,
        target_bundle_id: &str,
        final_session_kind: InteractionSessionKind,
        operation: GuardedClipboardOperation,
    ) -> Result<GuardedClipboardOutput, DeviceControlError> {
        let lease = self.validate_stream(context)?;
        let udid = lease.udid().to_string();
        let progress = GuardedClipboardProgress::default();
        let mut pending = GuardedClipboardCleanup::new(context, progress.clone());
        let transition = self
            .driver
            .guarded_clipboard_transition(
                &udid,
                agent_bundle_id,
                target_bundle_id,
                final_session_kind,
                ClipboardAccessMode::TargetBackgroundSafe,
                operation,
                progress,
            )
            .await
            .map_err(|error| driver_error(&udid, "guardedClipboardTransition", error))?;

        if transition.target.bundle_id != target_bundle_id || transition.target.pid == 0 {
            return Err(DeviceControlError::InvalidContext {
                reason: "guarded clipboard target identity proof is invalid",
            });
        }
        if transition.stop.is_some()
            || transition.agent.is_some()
            || transition.final_session.is_some()
            || transition.stream.is_some()
        {
            return Err(DeviceControlError::InvalidContext {
                reason: "background-safe clipboard unexpectedly changed UI lifecycle",
            });
        }
        let output = transition.output;
        pending.committed = true;
        Ok(output)
    }
    async fn submit_agent_foreground_clipboard(
        &self,
        context: &mut UiWithStreamContext,
        agent_bundle_id: &str,
        target_bundle_id: &str,
        final_session_kind: InteractionSessionKind,
        operation: GuardedClipboardOperation,
    ) -> Result<GuardedClipboardOutput, DeviceControlError> {
        self.validate_stream(context)?;
        let plane_id = context.plane_id;
        let original_stream = context.start_proof.clone();
        let cleanup = context.cleanup.clone();
        let ticket = context
            .take_ticket()
            .expect("validated clipboard context has a cleanup ticket");
        let (response_tx, response_rx) = oneshot::channel();
        let command = WorkerCommand::GuardedClipboard {
            plane_id,
            ticket,
            cleanup: cleanup.clone(),
            original_stream: original_stream.clone(),
            agent_bundle_id: agent_bundle_id.to_string(),
            target_bundle_id: target_bundle_id.to_string(),
            final_session_kind,
            operation,
            response: response_tx,
        };
        if let Err(error) = self.cleanup_tx.send(command) {
            if let WorkerCommand::GuardedClipboard { ticket, .. } = error.0 {
                *context = ticket.into_context(plane_id, cleanup, original_stream, None);
            }
            return Err(DeviceControlError::CleanupWorkerClosed);
        }
        let response = response_rx
            .await
            .map_err(|_| DeviceControlError::CleanupWorkerClosed)?;
        let GuardedClipboardResponse {
            result,
            context: replacement,
        } = response;
        if let Some(replacement) = replacement {
            *context = replacement;
        }
        result
    }
    pub async fn foreground_streaming_app(
        &self,
        context: &UiWithStreamContext,
        bundle_id: &str,
    ) -> Result<ForegroundAppProof, DeviceControlError> {
        let lease = self.validate_stream(context)?;
        self.driver
            .launch_app(lease.udid(), bundle_id)
            .await
            .map_err(|error| driver_error(lease.udid(), "foregroundStreamingApp", error))?;
        Ok(ForegroundAppProof {
            udid: lease.udid().to_string(),
            bundle_id: bundle_id.to_string(),
        })
    }
    pub async fn terminate_streaming_app(
        &self,
        context: &UiWithStreamContext,
        bundle_id: &str,
    ) -> Result<ProcessAbsenceProof, DeviceControlError> {
        let lease = self.validate_stream(context)?;
        self.driver
            .terminate_app(lease.udid(), bundle_id)
            .await
            .map_err(|error| driver_error(lease.udid(), "terminateStreamingApp", error))
    }
    pub async fn inspect_streaming_app_process(
        &self,
        context: &UiWithStreamContext,
        bundle_id: &str,
    ) -> Result<AppProcessState, DeviceControlError> {
        let lease = self.validate_stream(context)?;
        self.driver
            .inspect_app_process(lease.udid(), bundle_id)
            .await
            .map_err(|error| driver_error(lease.udid(), "inspectStreamingAppProcess", error))
    }
    /// Recover a wedged interaction session: stop the stream, optionally recycle
    /// the transport, then start a fresh session + stream.
    ///
    /// The whole destructive sequence runs on the cleanup worker, not in the
    /// caller's future. Cancelling the caller only drops the `oneshot` response;
    /// the worker still owns the in-flight stops and either finishes the recovery
    /// or cleans up by the observed generation. Running the stops directly here
    /// let a dropped caller abort a half-done stop and leave the generation
    /// inconsistent, quarantining the lease and (at budget=1) wedging the fleet.
    pub async fn recover_streaming_session(
        &self,
        context: &mut UiWithStreamContext,
        bundle_id: &str,
        kind: InteractionSessionKind,
        recycle_transport: bool,
    ) -> Result<Arc<dyn UiSession>, DeviceControlError> {
        self.validate_stream(context)?;
        let plane_id = context.plane_id;
        let original_stream = context.start_proof.clone();
        let cleanup = context.cleanup.clone();
        let ticket = context
            .take_ticket()
            .expect("validated stream context has a cleanup ticket");
        let (response_tx, response_rx) = oneshot::channel();
        let command = WorkerCommand::RecoverStream {
            plane_id,
            ticket,
            cleanup: cleanup.clone(),
            original_stream: original_stream.clone(),
            bundle_id: bundle_id.to_string(),
            kind,
            recycle_transport,
            response: response_tx,
        };
        if let Err(error) = self.cleanup_tx.send(command) {
            if let WorkerCommand::RecoverStream { ticket, .. } = error.0 {
                *context = ticket.into_context(plane_id, cleanup, original_stream, None);
            }
            return Err(DeviceControlError::CleanupWorkerClosed);
        }
        let response = response_rx
            .await
            .map_err(|_| DeviceControlError::CleanupWorkerClosed)?;
        let RecoverStreamResponse {
            result,
            context: replacement,
        } = response;
        if let Some(replacement) = replacement {
            *context = replacement;
        }
        result
    }
    pub async fn start_reserved_stream(
        &self,
        context: UiSessionContext,
        capacity: UiCapacityReservation,
    ) -> Result<UiWithStreamContext, DeviceControlError> {
        self.start_reserved_stream_internal(context, capacity)
            .await
            .map_err(|failure| failure.error)
    }
    pub(super) async fn start_reserved_stream_internal(
        &self,
        mut context: UiSessionContext,
        mut capacity: UiCapacityReservation,
    ) -> Result<UiWithStreamContext, StreamContextUpgradeFailure> {
        let lease = match self.validate_session(&context) {
            Ok(lease) => lease,
            Err(error) => {
                context.ui_capacity_token = None;
                return Err(StreamContextUpgradeFailure {
                    context: Some(context),
                    failed_start: None,
                    error,
                });
            }
        };
        if capacity.plane_id != self.plane_id {
            context.ui_capacity_token = None;
            return Err(StreamContextUpgradeFailure {
                context: Some(context),
                failed_start: None,
                error: DeviceControlError::InvalidContext {
                    reason: "stream capacity belongs to another control plane",
                },
            });
        }
        let Some(reservation) = capacity.reservation.as_ref() else {
            context.ui_capacity_token = None;
            return Err(StreamContextUpgradeFailure {
                context: Some(context),
                failed_start: None,
                error: DeviceControlError::InvalidContext {
                    reason: "stream capacity has been consumed",
                },
            });
        };
        if let Err(error) = self.validate_reservation(lease, reservation) {
            context.ui_capacity_token = None;
            return Err(StreamContextUpgradeFailure {
                context: Some(context),
                failed_start: None,
                error,
            });
        }
        if context.ui_capacity_token != Some(reservation.token()) {
            context.ui_capacity_token = None;
            return Err(StreamContextUpgradeFailure {
                context: Some(context),
                failed_start: None,
                error: DeviceControlError::InvalidContext {
                    reason: "session and stream capacity reservations do not match",
                },
            });
        }
        let udid = lease.udid().to_string();
        let Some(handoff_generation) = context.stream_handoff_generation else {
            context.ui_capacity_token = None;
            return Err(StreamContextUpgradeFailure {
                context: Some(context),
                failed_start: None,
                error: DeviceControlError::InvalidContext {
                    reason: "session has no exact stream handoff generation",
                },
            });
        };
        if let Err(error) = self.streams.mark_running(reservation.token()) {
            context.ui_capacity_token = None;
            return Err(StreamContextUpgradeFailure {
                context: Some(context),
                failed_start: None,
                error: error.into(),
            });
        }
        let reservation = capacity
            .reservation
            .take()
            .expect("validated capacity has a reservation");

        let cleanup = CleanupSink {
            tx: self.cleanup_tx.clone(),
            quarantined: self.quarantined.clone(),
        };
        let mut pending = PendingStreamStart {
            ticket: Some(DeviceCleanupTicket {
                lease: context.lease.take().expect("validated context has a lease"),
                activity: context
                    .activity
                    .take()
                    .expect("validated context has an activity permit"),
                reservation,
                session: context
                    .session
                    .take()
                    .expect("validated context has a session"),
                expected_generation: Some(handoff_generation),
            }),
            cleanup: cleanup.clone(),
        };

        let proof = match self.driver.start_stream_after_session(&udid).await {
            Ok(proof) => proof,
            Err(error) => {
                if let Ok(handoff) = self.driver.confirm_interaction_stream_stopped(&udid).await {
                    if handoff.generation >= handoff_generation {
                        pending
                            .ticket
                            .as_mut()
                            .expect("failed stream start retains its cleanup ticket")
                            .expected_generation = Some(handoff.generation);
                    }
                }
                let error = driver_error(&udid, "startStreamAfterSession", error);
                return Err(StreamContextUpgradeFailure {
                    context: None,
                    failed_start: Some(FailedStreamStartContext {
                        plane_id: self.plane_id,
                        pending: Some(pending),
                    }),
                    error,
                });
            }
        };
        if proof.generation != handoff_generation {
            return Err(StreamContextUpgradeFailure {
                context: None,
                failed_start: Some(FailedStreamStartContext {
                    plane_id: self.plane_id,
                    pending: Some(pending),
                }),
                error: DeviceControlError::StopProofMismatch {
                    udid,
                    expected: handoff_generation,
                    actual: proof.generation,
                },
            });
        }
        if !proof.first_frame_observed {
            let error = DeviceControlError::FirstFrameMissing { udid: udid.clone() };
            return Err(StreamContextUpgradeFailure {
                context: None,
                failed_start: Some(FailedStreamStartContext {
                    plane_id: self.plane_id,
                    pending: Some(pending),
                }),
                error,
            });
        }
        let ticket = pending
            .ticket
            .take()
            .expect("pending stream ticket remains armed");

        Ok(UiWithStreamContext {
            plane_id: self.plane_id,
            lease: Some(ticket.lease),
            activity: Some(ticket.activity),
            reservation: Some(ticket.reservation),
            session: Some(ticket.session),
            start_proof: proof,
            cleanup,
        })
    }
    pub async fn close_ui_context(
        &self,
        mut context: UiWithStreamContext,
    ) -> Result<DeviceReleaseProof, DeviceControlError> {
        self.validate_stream(&context)?;
        let ticket = context
            .take_ticket()
            .expect("validated streaming context has a cleanup ticket");
        self.close_cleanup_ticket(ticket).await
    }
    pub(super) async fn close_cleanup_ticket(
        &self,
        ticket: DeviceCleanupTicket,
    ) -> Result<DeviceReleaseProof, DeviceControlError> {
        let (response_tx, response_rx) = oneshot::channel();
        if let Err(error) = self.cleanup_tx.send(WorkerCommand::Close {
            ticket,
            response: response_tx,
        }) {
            if let WorkerCommand::Close { ticket, .. } = error.0 {
                self.quarantined.push_cleanup(ticket);
            }
            return Err(DeviceControlError::CleanupWorkerClosed);
        }
        response_rx
            .await
            .map_err(|_| DeviceControlError::CleanupWorkerClosed)?
    }
    pub fn reserved_stream_capacity(&self) -> usize {
        self.streams.reserved_capacity()
    }
    /// Maximum number of background/foreground producers this control plane
    /// may keep alive at once.
    pub fn configured_stream_capacity(&self) -> usize {
        self.streams.configured_limit()
    }
    pub fn cleanup_quarantine_count(&self) -> usize {
        self.quarantined.count()
    }
    pub async fn shutdown_cleanup(&self) -> Result<(), DeviceControlError> {
        let _shutdown_guard = self.shutdown_gate.lock().await;
        if self.lifecycle.phase() == ControlPlanePhase::Stopped {
            let count = self.cleanup_quarantine_count();
            return if count == 0 {
                Ok(())
            } else {
                Err(DeviceControlError::CleanupQuarantined { count })
            };
        }
        let drain_rx = {
            let _background_guard = self.background_gate.lock();
            self.lifecycle.begin_shutdown();
            let (drain_tx, drain_rx) = oneshot::channel();
            self.cleanup_tx
                .send(WorkerCommand::DrainBackground { ack: drain_tx })
                .map_err(|_| DeviceControlError::CleanupWorkerClosed)?;
            drain_rx
        };
        drain_rx
            .await
            .map_err(|_| DeviceControlError::CleanupWorkerClosed)?;

        loop {
            let quarantined = self.quarantined.context_activity_count();
            if self.lifecycle.outstanding() <= quarantined {
                break;
            }
            let lifecycle_changed = self.lifecycle.changed.notified();
            let quarantine_changed = self.quarantined.changed.notified();
            tokio::pin!(lifecycle_changed);
            tokio::pin!(quarantine_changed);
            lifecycle_changed.as_mut().enable();
            quarantine_changed.as_mut().enable();
            if self.lifecycle.outstanding() <= self.quarantined.context_activity_count() {
                continue;
            }
            tokio::select! {
                _ = lifecycle_changed => {}
                _ = quarantine_changed => {}
            }
        }

        let (ack_tx, ack_rx) = oneshot::channel();
        let worker_closed = self
            .cleanup_tx
            .send(WorkerCommand::Shutdown { ack: ack_tx })
            .is_err();
        if !worker_closed {
            let _ = ack_rx.await;
        }
        let handle = self.cleanup_handle.lock().take();
        if let Some(handle) = handle {
            handle.await.map_err(|error| DeviceControlError::Driver {
                udid: "control-plane".to_string(),
                operation: "joinCleanupWorker",
                message: error.to_string(),
            })?;
        }
        let driver_shutdown = self
            .driver
            .shutdown_owned_processes()
            .await
            .map_err(|error| DeviceControlError::Driver {
                udid: "control-plane".to_string(),
                operation: "shutdownOwnedProcesses",
                message: error.to_string(),
            });
        self.lifecycle.mark_stopped();
        let count = self.cleanup_quarantine_count();
        if count > 0 {
            return Err(DeviceControlError::CleanupQuarantined { count });
        }
        if worker_closed {
            return Err(DeviceControlError::CleanupWorkerClosed);
        }
        driver_shutdown
    }
}
