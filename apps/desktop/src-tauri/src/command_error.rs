use riviu_core::{
    DeviceControlError, DeviceWorkOwner, FlowNotFound, FlowRetryError, FlowRuntimeError,
    FlowSelectionError, RevisionConflict,
};
use riviu_script_engine::FlowCompileError;
use serde::Serialize;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CommandError {
    pub code: String,
    pub message: Box<str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub udid: Option<Box<str>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub requested_owner: Option<DeviceWorkOwner>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_owner: Option<DeviceWorkOwner>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub node_id: Option<Box<str>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub field: Option<Box<str>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attempt_id: Option<Box<str>>,
}

impl CommandError {
    pub fn operation(error: impl std::fmt::Display) -> Self {
        Self {
            code: "OperationFailed".to_string(),
            message: error.to_string().into_boxed_str(),
            udid: None,
            requested_owner: None,
            current_owner: None,
            node_id: None,
            field: None,
            attempt_id: None,
        }
    }

    pub fn code(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into().into_boxed_str(),
            udid: None,
            requested_owner: None,
            current_owner: None,
            node_id: None,
            field: None,
            attempt_id: None,
        }
    }

    pub fn application_shutting_down() -> Self {
        Self::code(
            "ApplicationShuttingDown",
            "the application is shutting down",
        )
    }

    pub fn invalid_argument(message: impl Into<String>) -> Self {
        Self::code("InvalidArgument", message)
    }

    pub fn from_compile(error: FlowCompileError) -> Self {
        Self {
            code: error.code,
            message: error.message.into_boxed_str(),
            udid: None,
            requested_owner: None,
            current_owner: None,
            node_id: error.node_id.map(|id| id.to_string().into_boxed_str()),
            field: error.field.map(String::into_boxed_str),
            attempt_id: None,
        }
    }

    pub fn from_service(error: anyhow::Error) -> Self {
        if let Some(conflict) = error.downcast_ref::<RevisionConflict>() {
            return Self::code("RevisionConflict", conflict.to_string());
        }
        if let Some(not_found) = error.downcast_ref::<FlowNotFound>() {
            return Self::code("FlowNotFound", not_found.to_string());
        }
        if let Some(selection) = error.downcast_ref::<FlowSelectionError>() {
            let code = match selection {
                FlowSelectionError::Empty => "EmptySelection",
                FlowSelectionError::UnknownDevice => "UnknownDevice",
                FlowSelectionError::Duplicate => "DuplicateDevice",
                FlowSelectionError::NoEligibleDevice => "NoEligibleDevice",
            };
            return Self::code(code, selection.to_string());
        }
        if let Some(retry) = error.downcast_ref::<FlowRetryError>() {
            let code = match retry {
                FlowRetryError::NotAllowed { .. } => "RetryNotAllowed",
                FlowRetryError::AlreadyRunning => "RetryAlreadyRunning",
            };
            return Self::code(code, retry.to_string());
        }
        if let Some(runtime) = error.downcast_ref::<FlowRuntimeError>() {
            let code = match runtime {
                FlowRuntimeError::RunNotFound { .. } => "FlowRunNotFound",
                FlowRuntimeError::AttemptNotFound { .. } => "FlowAttemptNotFound",
                FlowRuntimeError::CancellationOwnerMissing { .. } => "FlowCancellationOwnerMissing",
            };
            return Self::code(code, runtime.to_string());
        }
        Self::operation(error)
    }
}

impl From<DeviceControlError> for CommandError {
    fn from(error: DeviceControlError) -> Self {
        match error {
            DeviceControlError::Busy(busy) => Self {
                code: "DeviceBusy".to_string(),
                message: busy.to_string().into_boxed_str(),
                udid: Some(busy.udid.into_boxed_str()),
                requested_owner: Some(busy.requested_owner),
                current_owner: Some(busy.current_owner),
                node_id: None,
                field: None,
                attempt_id: None,
            },
            other => Self {
                code: "DeviceControlFailed".to_string(),
                message: other.to_string().into_boxed_str(),
                udid: None,
                requested_owner: None,
                current_owner: None,
                node_id: None,
                field: None,
                attempt_id: None,
            },
        }
    }
}

impl From<String> for CommandError {
    fn from(message: String) -> Self {
        Self::operation(message)
    }
}

impl From<&str> for CommandError {
    fn from(message: &str) -> Self {
        Self::operation(message)
    }
}

impl From<CommandError> for String {
    fn from(error: CommandError) -> Self {
        format!("{}: {}", error.code, error.message)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_device_owner_busy_has_a_stable_serialized_code() {
        let error = CommandError::from(DeviceControlError::Busy(riviu_core::DeviceBusy {
            udid: "fixture".to_string(),
            requested_owner: DeviceWorkOwner::ManualControl,
            current_owner: DeviceWorkOwner::Script,
        }));
        let json = serde_json::to_value(error).expect("serialize command error");

        assert_eq!(json["code"], "DeviceBusy");
        assert_eq!(json["udid"], "fixture");
        assert_eq!(json["requestedOwner"], "manualControl");
        assert_eq!(json["currentOwner"], "script");
    }
}
