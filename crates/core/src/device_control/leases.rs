//! Who holds a device, and every check that says a caller still does.
//!
//! The acquire calls and the `validate_*` family are one subject: each validate is the
//! question an acquire already answered, asked again at the moment of use, because a lease
//! can be lost between the two.

use super::*;

impl DeviceControlPlane {
    pub async fn try_acquire_exclusive(
        &self,
        udid: &str,
        owner: DeviceWorkOwner,
    ) -> Result<DeviceExclusiveContext, DeviceControlError> {
        self.ensure_running()?;
        let lease = self.work.try_acquire(udid, owner)?;
        let activity = self.lifecycle.register()?;
        self.submit_park(DeviceExclusiveContext {
            plane_id: self.plane_id,
            lease: Some(lease),
            activity: Some(activity),
            ui_capacity_token: None,
        })
        .await
    }
    /// Take exclusive without parking the live preview.
    ///
    /// Manual tap/swipe/type/home from the desktop overlay must ride the
    /// background MJPEG/minicap producer. `try_acquire_exclusive` parks that
    /// producer, which is why clicking a live tile used to black the screen
    /// and then wait 40s for TikTok to foreground.
    pub async fn try_acquire_exclusive_keeping_stream(
        &self,
        udid: &str,
        owner: DeviceWorkOwner,
    ) -> Result<DeviceExclusiveContext, DeviceControlError> {
        self.ensure_running()?;
        let lease = self.work.try_acquire(udid, owner)?;
        let activity = self.lifecycle.register()?;
        Ok(DeviceExclusiveContext {
            plane_id: self.plane_id,
            lease: Some(lease),
            activity: Some(activity),
            ui_capacity_token: None,
        })
    }
    pub async fn acquire_exclusive(
        &self,
        udid: &str,
        owner: DeviceWorkOwner,
    ) -> Result<DeviceExclusiveContext, DeviceControlError> {
        self.ensure_running()?;
        let lease = self.work.acquire(udid, owner).await.map_err(|error| {
            DeviceControlError::InvalidContext {
                reason: match error {
                    crate::DeviceWorkAcquireError::WaitNotAllowed { .. } => {
                        "the requested owner is not allowed to wait"
                    }
                },
            }
        })?;
        let activity = self.lifecycle.register()?;
        self.submit_park(DeviceExclusiveContext {
            plane_id: self.plane_id,
            lease: Some(lease),
            activity: Some(activity),
            ui_capacity_token: None,
        })
        .await
    }
    pub async fn try_acquire_interaction(
        &self,
        udid: &str,
    ) -> Result<InteractionAcquireResult, DeviceControlError> {
        match self
            .try_acquire_exclusive(udid, DeviceWorkOwner::Interaction)
            .await
        {
            Ok(context) => Ok(InteractionAcquireResult::Acquired(context)),
            Err(DeviceControlError::Busy(busy)) => {
                Ok(InteractionAcquireResult::SkippedUnavailable(busy))
            }
            Err(error) => Err(error),
        }
    }
    pub(super) fn validate_exclusive<'a>(
        &self,
        context: &'a DeviceExclusiveContext,
    ) -> Result<&'a DeviceWorkLease, DeviceControlError> {
        self.validate_lease(context.plane_id, context.lease.as_ref())
    }
    /// The one check every device action actually needs, whichever context is holding the
    /// lease.
    ///
    /// Nothing new is checked here: both arms end in [`Self::validate_lease`], so the
    /// plane-id, the consumed-context guard, the work token and the owner match all still
    /// run exactly as before.
    pub(super) fn validate_leased<'a>(
        &self,
        lease: DeviceLeaseRef<'a>,
    ) -> Result<&'a DeviceWorkLease, DeviceControlError> {
        match lease {
            DeviceLeaseRef::Exclusive(context) => self.validate_exclusive(context),
            DeviceLeaseRef::Session(context) => self.validate_session(context),
        }
    }
    pub(super) fn validate_interaction_capacity(
        &self,
        context: &DeviceExclusiveContext,
    ) -> Result<Uuid, DeviceControlError> {
        let lease = self.validate_exclusive(context)?;
        let token = context
            .ui_capacity_token
            .ok_or(DeviceControlError::InvalidContext {
                reason: "interaction session requires reserved stream capacity",
            })?;
        if self.streams.reservation_udid(token).as_deref() != Some(lease.udid()) {
            return Err(DeviceControlError::InvalidContext {
                reason: "interaction stream capacity reservation is no longer current",
            });
        }
        Ok(token)
    }
    pub(super) fn validate_session<'a>(
        &self,
        context: &'a UiSessionContext,
    ) -> Result<&'a DeviceWorkLease, DeviceControlError> {
        if context.session.is_none() {
            return Err(DeviceControlError::InvalidContext {
                reason: "session context has been consumed",
            });
        }
        self.validate_lease(context.plane_id, context.lease.as_ref())
    }
    pub(super) fn validate_stream<'a>(
        &self,
        context: &'a UiWithStreamContext,
    ) -> Result<&'a DeviceWorkLease, DeviceControlError> {
        let lease = self.validate_lease(context.plane_id, context.lease.as_ref())?;
        let reservation =
            context
                .reservation
                .as_ref()
                .ok_or(DeviceControlError::InvalidContext {
                    reason: "stream context has been consumed",
                })?;
        self.validate_reservation(lease, reservation)?;
        Ok(lease)
    }
    fn validate_lease<'a>(
        &self,
        plane_id: Uuid,
        lease: Option<&'a DeviceWorkLease>,
    ) -> Result<&'a DeviceWorkLease, DeviceControlError> {
        self.ensure_running()?;
        if plane_id != self.plane_id {
            return Err(DeviceControlError::InvalidContext {
                reason: "context belongs to another control plane",
            });
        }
        let lease = lease.ok_or(DeviceControlError::InvalidContext {
            reason: "device context has been consumed",
        })?;
        let owner = self
            .work
            .validate_token(lease.udid(), lease.token())
            .map_err(|_| DeviceControlError::InvalidContext {
                reason: "device work token is no longer current",
            })?;
        if owner != lease.owner() {
            return Err(DeviceControlError::InvalidContext {
                reason: "device work owner does not match its token",
            });
        }
        Ok(lease)
    }
    pub(super) fn validate_reservation(
        &self,
        lease: &DeviceWorkLease,
        reservation: &ForegroundStreamReservation,
    ) -> Result<(), DeviceControlError> {
        if reservation.udid() != lease.udid() || reservation.owner() != lease.owner() {
            return Err(DeviceControlError::InvalidContext {
                reason: "stream reservation does not match the device context",
            });
        }
        if self
            .streams
            .reservation_udid(reservation.token())
            .as_deref()
            != Some(lease.udid())
        {
            return Err(DeviceControlError::InvalidContext {
                reason: "stream reservation token is no longer current",
            });
        }
        Ok(())
    }
    pub(super) fn validate_background_lease(
        &self,
        lease: &BackgroundStreamLease,
    ) -> Result<(), DeviceControlError> {
        if !self.backgrounds.contains(lease.udid(), lease.token())
            || self.streams.reservation_udid(lease.token()).as_deref() != Some(lease.udid())
        {
            return Err(DeviceControlError::InvalidContext {
                reason: "background stream reservation is no longer current",
            });
        }
        Ok(())
    }
    pub(super) fn ensure_running(&self) -> Result<(), DeviceControlError> {
        self.lifecycle.ensure_running()
    }
    async fn submit_park(
        &self,
        context: DeviceExclusiveContext,
    ) -> Result<DeviceExclusiveContext, DeviceControlError> {
        self.ensure_running()?;
        if self.streams.reservation_token(context.udid()).is_none() {
            return Ok(context);
        }
        let (response_tx, response_rx) = oneshot::channel();
        let command = WorkerCommand::Park {
            context,
            response: response_tx,
        };
        if let Err(error) = self.cleanup_tx.send(command) {
            if let WorkerCommand::Park { context, .. } = error.0 {
                drop(context);
            }
            return Err(DeviceControlError::CleanupWorkerClosed);
        }
        response_rx
            .await
            .map_err(|_| DeviceControlError::CleanupWorkerClosed)?
    }
    pub(super) async fn submit_reserve(
        &self,
        context: DeviceExclusiveContext,
    ) -> Result<ReservedUiCapacity, CapacityContextUpgradeFailure> {
        if let Err(error) = self.ensure_running() {
            return Err(CapacityContextUpgradeFailure {
                context: Some(context),
                error,
            });
        }
        let (response_tx, response_rx) = oneshot::channel();
        let command = WorkerCommand::Reserve {
            context,
            response: response_tx,
        };
        if let Err(error) = self.cleanup_tx.send(command) {
            if let WorkerCommand::Reserve { context, .. } = error.0 {
                return Err(CapacityContextUpgradeFailure {
                    context: Some(context),
                    error: DeviceControlError::CleanupWorkerClosed,
                });
            }
            unreachable!("send failure retains the Reserve command");
        }
        response_rx
            .await
            .map_err(|_| CapacityContextUpgradeFailure {
                context: None,
                error: DeviceControlError::CleanupWorkerClosed,
            })?
    }
}
