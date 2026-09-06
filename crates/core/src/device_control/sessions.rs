//! UI sessions: opening one on top of a lease, bringing the target app forward, and
//! closing both in the right order.

use super::*;

impl DeviceControlPlane {
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
