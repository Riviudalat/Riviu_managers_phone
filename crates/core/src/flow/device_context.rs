use std::sync::Arc;

use crate::device_control::FailedStreamStartContext;

use crate::{
    AppProcessState, ContextReleaseProof, DeviceControlError, DeviceControlPlane,
    DeviceExclusiveContext, DeviceWorkOwner, InteractionSessionKind, ProcessAbsenceProof,
    UiCapacityReservation, UiSession, UiSessionContext, UiWithStreamContext,
};

pub(crate) enum FlowDeviceContext {
    NoDeviceResources { udid: String },
    Exclusive(DeviceExclusiveContext),
    Session(UiSessionContext),
    Streaming(UiWithStreamContext),
    Closed,
}

pub(crate) struct FlowDeviceUpgradeFailure {
    pub(crate) error: DeviceControlError,
    pub(crate) failed_stream: Option<FailedStreamStartContext>,
}

impl FlowDeviceUpgradeFailure {
    pub(crate) fn reconciliation_session(
        &self,
        control: &DeviceControlPlane,
    ) -> Result<Option<Arc<dyn UiSession>>, DeviceControlError> {
        self.failed_stream
            .as_ref()
            .map(|context| control.failed_stream_session(context))
            .transpose()
    }

    pub(crate) async fn release_failed_stream(
        &mut self,
        control: &DeviceControlPlane,
    ) -> Result<Option<ContextReleaseProof>, DeviceControlError> {
        match self.failed_stream.take() {
            Some(context) => control.close_failed_stream_start(context).await.map(Some),
            None => Ok(None),
        }
    }
}

impl FlowDeviceContext {
    pub(crate) fn level(&self) -> u8 {
        match self {
            Self::NoDeviceResources { .. } => 0,
            Self::Exclusive(_) => 1,
            Self::Session(_) => 2,
            Self::Streaming(_) => 3,
            Self::Closed => 4,
        }
    }

    pub(crate) fn no_device_resources(udid: impl Into<String>) -> Self {
        Self::NoDeviceResources { udid: udid.into() }
    }

    pub(crate) fn exclusive(&self) -> Result<&DeviceExclusiveContext, DeviceControlError> {
        match self {
            Self::Exclusive(context) => Ok(context),
            _ => Err(DeviceControlError::InvalidContext {
                reason: "Flow context is no longer exclusive",
            }),
        }
    }

    pub(crate) fn session(
        &self,
        control: &DeviceControlPlane,
    ) -> Result<Arc<dyn UiSession>, DeviceControlError> {
        match self {
            Self::Session(context) => control.session(context),
            Self::Streaming(context) => control.streaming_session(context),
            _ => Err(DeviceControlError::InvalidContext {
                reason: "Flow context has no UI session",
            }),
        }
    }

    pub(crate) fn generation(&self) -> u64 {
        match self {
            Self::Streaming(context) => context.stream_proof().generation,
            _ => 0,
        }
    }

    pub(crate) async fn upgrade_session(
        &mut self,
        control: &DeviceControlPlane,
        bundle_id: &str,
        kind: InteractionSessionKind,
    ) -> Result<(), DeviceControlError> {
        let Self::Exclusive(context) = std::mem::replace(self, Self::Closed) else {
            return Err(DeviceControlError::InvalidContext {
                reason: "Flow session upgrade requires an exclusive context",
            });
        };
        let session = match control
            .try_foreground_target_app_and_start_interaction_session(context, bundle_id, kind)
            .await
        {
            Ok((session, _proof)) => session,
            Err(failure) => {
                *self = Self::Exclusive(failure.context);
                return Err(failure.error);
            }
        };
        *self = Self::Session(session);
        Ok(())
    }

    pub(crate) async fn reserve_capacity(
        &mut self,
        control: &DeviceControlPlane,
    ) -> Result<UiCapacityReservation, FlowDeviceUpgradeFailure> {
        let Self::Exclusive(context) = std::mem::replace(self, Self::Closed) else {
            return Err(FlowDeviceUpgradeFailure {
                error: DeviceControlError::InvalidContext {
                    reason: "Flow capacity reservation requires an exclusive context",
                },
                failed_stream: None,
            });
        };
        match control.try_reserve_ui_capacity(context).await {
            Ok((context, reservation)) => {
                *self = Self::Exclusive(context);
                Ok(reservation)
            }
            Err(failure) => {
                if let Some(context) = failure.context {
                    *self = Self::Exclusive(context);
                }
                Err(FlowDeviceUpgradeFailure {
                    error: failure.error,
                    failed_stream: None,
                })
            }
        }
    }

    pub(crate) async fn upgrade_stream(
        &mut self,
        control: &DeviceControlPlane,
        reservation: crate::UiCapacityReservation,
    ) -> Result<(), FlowDeviceUpgradeFailure> {
        let Self::Session(context) = std::mem::replace(self, Self::Closed) else {
            return Err(FlowDeviceUpgradeFailure {
                error: DeviceControlError::InvalidContext {
                    reason: "Flow stream upgrade requires a session context",
                },
                failed_stream: None,
            });
        };
        let streaming = match control
            .try_start_reserved_stream(context, reservation)
            .await
        {
            Ok(streaming) => streaming,
            Err(failure) => {
                if let Some(context) = failure.context {
                    *self = Self::Session(context);
                }
                return Err(FlowDeviceUpgradeFailure {
                    error: failure.error,
                    failed_stream: failure.failed_start,
                });
            }
        };
        *self = Self::Streaming(streaming);
        Ok(())
    }

    pub(crate) async fn active_app_bundle(
        &self,
        control: &DeviceControlPlane,
    ) -> Result<String, DeviceControlError> {
        match self {
            Self::Exclusive(context) => control.read_active_app_bundle(context).await,
            Self::Session(context) => {
                let udid = context.udid().to_string();
                control
                    .session(context)?
                    .active_app_bundle()
                    .await
                    .map_err(|error| DeviceControlError::Driver {
                        udid,
                        operation: "readActiveAppBundle",
                        message: error.to_string(),
                    })
            }
            Self::Streaming(context) => {
                let udid = context.udid().to_string();
                control
                    .streaming_session(context)?
                    .active_app_bundle()
                    .await
                    .map_err(|error| DeviceControlError::Driver {
                        udid,
                        operation: "readActiveAppBundle",
                        message: error.to_string(),
                    })
            }
            Self::NoDeviceResources { .. } | Self::Closed => {
                Err(DeviceControlError::InvalidContext {
                    reason: "Flow context has no active-app channel",
                })
            }
        }
    }

    pub(crate) async fn foreground_app(
        &self,
        control: &DeviceControlPlane,
        bundle_id: &str,
    ) -> Result<(), DeviceControlError> {
        match self {
            Self::Exclusive(context) => {
                control.foreground_target_app(context, bundle_id).await?;
            }
            Self::Session(context) => {
                control.foreground_session_app(context, bundle_id).await?;
            }
            Self::Streaming(context) => {
                control.foreground_streaming_app(context, bundle_id).await?;
            }
            Self::NoDeviceResources { .. } | Self::Closed => {
                return Err(DeviceControlError::InvalidContext {
                    reason: "Flow context is closed",
                });
            }
        }
        Ok(())
    }

    pub(crate) async fn terminate_app(
        &self,
        control: &DeviceControlPlane,
        bundle_id: &str,
    ) -> Result<ProcessAbsenceProof, DeviceControlError> {
        match self {
            Self::Exclusive(context) => control.terminate_app(context, bundle_id).await,
            Self::Session(context) => control.terminate_session_app(context, bundle_id).await,
            Self::Streaming(context) => control.terminate_streaming_app(context, bundle_id).await,
            Self::NoDeviceResources { .. } | Self::Closed => {
                Err(DeviceControlError::InvalidContext {
                    reason: "Flow context has no device-control channel",
                })
            }
        }
    }

    pub(crate) async fn inspect_process(
        &self,
        control: &DeviceControlPlane,
        bundle_id: &str,
    ) -> Result<AppProcessState, DeviceControlError> {
        match self {
            Self::Exclusive(context) => control.inspect_app_process(context, bundle_id).await,
            Self::Session(context) => {
                control
                    .inspect_session_app_process(context, bundle_id)
                    .await
            }
            Self::Streaming(context) => {
                control
                    .inspect_streaming_app_process(context, bundle_id)
                    .await
            }
            Self::NoDeviceResources { .. } | Self::Closed => {
                Err(DeviceControlError::InvalidContext {
                    reason: "Flow context has no device-control channel",
                })
            }
        }
    }

    pub(crate) async fn close(
        &mut self,
        control: &DeviceControlPlane,
    ) -> Result<ContextReleaseProof, DeviceControlError> {
        match std::mem::replace(self, Self::Closed) {
            Self::NoDeviceResources { udid } => Ok(ContextReleaseProof {
                udid,
                owner: DeviceWorkOwner::Script,
                had_session: false,
                had_stream: false,
            }),
            Self::Exclusive(context) => control.close_exclusive_context(context),
            Self::Session(context) => control.close_session_context(context),
            Self::Streaming(context) => control.close_ui_context(context).await.map(Into::into),
            Self::Closed => Err(DeviceControlError::InvalidContext {
                reason: "Flow context was already closed",
            }),
        }
    }
}
