use riviu_core::{
    AutomationConfigSecret, AutomationDefinitionArchived, AutomationDefinitionNotFound,
    AutomationRevisionConflict, AutomationScheduleConflict, AutomationScheduleNotFound,
    DeviceControlError, DeviceWorkOwner, FlowNotFound, FlowRetryError, FlowRuntimeError,
    FlowSelectionError, OrchestrationNotFound, OrchestrationRevisionConflict, RevisionConflict,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account_conflict: Option<Box<riviu_core::db::AccountAssignmentConflict>>,
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
            account_conflict: None,
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
            account_conflict: None,
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
            account_conflict: None,
        }
    }

    pub fn from_service(error: anyhow::Error) -> Self {
        if let Some(conflict) = error.downcast_ref::<riviu_core::db::AccountAssignmentConflict>() {
            let mut result = Self::code("AccountAssignmentConflict", conflict.to_string());
            result.udid = Some(conflict.udid.clone().into_boxed_str());
            result.account_conflict = Some(Box::new(conflict.clone()));
            return result;
        }
        if let Some(secret) = error.downcast_ref::<AutomationConfigSecret>() {
            return Self::code("SecretFieldRejected", secret.to_string());
        }
        if let Some(not_found) = error.downcast_ref::<AutomationDefinitionNotFound>() {
            return Self::code("AutomationDefinitionNotFound", not_found.to_string());
        }
        if let Some(archived) = error.downcast_ref::<AutomationDefinitionArchived>() {
            return Self::code("AutomationDefinitionArchived", archived.to_string());
        }
        if let Some(conflict) = error.downcast_ref::<AutomationRevisionConflict>() {
            return Self::code("AutomationRevisionConflict", conflict.to_string());
        }
        if let Some(not_found) = error.downcast_ref::<AutomationScheduleNotFound>() {
            return Self::code("AutomationScheduleNotFound", not_found.to_string());
        }
        if let Some(conflict) = error.downcast_ref::<AutomationScheduleConflict>() {
            return Self::code("AutomationScheduleConflict", conflict.to_string());
        }
        if let Some(not_found) = error.downcast_ref::<OrchestrationNotFound>() {
            return Self::code("OrchestrationNotFound", not_found.to_string());
        }
        if let Some(conflict) = error.downcast_ref::<OrchestrationRevisionConflict>() {
            return Self::code("OrchestrationRevisionConflict", conflict.to_string());
        }
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
            DeviceControlError::Driver {
                udid,
                operation,
                message,
            } if riviu_core::publish_recovery::classify(&message)
                == riviu_core::publish_recovery::FailureKind::Disconnected =>
            {
                let mut error=Self::code("DeviceDisconnected",format!("Máy {udid} mất kết nối ADB; kiểm tra USB và chờ đúng máy kết nối lại. Chi tiết: {operation}: {message}"));
                error.udid = Some(udid.into_boxed_str());
                error
            }
            DeviceControlError::Busy(busy) => Self {
                code: "DeviceBusy".to_string(),
                message: busy.to_string().into_boxed_str(),
                udid: Some(busy.udid.into_boxed_str()),
                requested_owner: Some(busy.requested_owner),
                current_owner: Some(busy.current_owner),
                node_id: None,
                field: None,
                attempt_id: None,
                account_conflict: None,
            },
            DeviceControlError::Driver {
                udid,
                operation,
                message,
            } => {
                let ambiguous = message.contains("more than one measured TikTok build")
                    || message.contains("foreground package must break the tie");
                let mut error = Self::code(
                    if ambiguous {
                        "DeviceAppSelectionRequired"
                    } else {
                        "DeviceControlFailed"
                    },
                    format!("{operation}: {message}"),
                );
                error.udid = Some(udid.into_boxed_str());
                error
            }
            other => Self {
                code: "DeviceControlFailed".to_string(),
                message: other.to_string().into_boxed_str(),
                udid: None,
                requested_owner: None,
                current_owner: None,
                node_id: None,
                field: None,
                attempt_id: None,
                account_conflict: None,
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
    fn account_collision_serializes_the_rejected_transaction_snapshot() {
        let path = std::env::temp_dir().join(format!("riviu-account-conflict-{}.db", uuid::Uuid::new_v4()));
        let db = riviu_core::db::Database::open(&path).unwrap();
        db.set_device_handle("target", "", "old.account").unwrap();
        for (udid, number, alias, handle) in [
            ("owner-a", Some(8), "Kệ A", " @Account "),
            ("owner-b", None, "", "ACCOUNT"),
        ] {
            let mut meta = db.get_device_meta(udid).unwrap();
            meta.number = number;
            meta.alias = alias.into();
            meta.handle = handle.into();
            db.upsert_device_meta(&meta).unwrap();
        }
        let collision = db.set_device_handle("target", "old.account", " @Account ").unwrap_err();
        // Later metadata changes must not alter the context of the rejected transaction.
        db.set_device_handle("owner-a", " @Account ", "").unwrap();
        let json = serde_json::to_value(CommandError::from_service(collision.context("save binding"))).unwrap();
        assert_eq!(json["code"], "AccountAssignmentConflict");
        assert_eq!(json["udid"], "target");
        assert_eq!(json["accountConflict"], serde_json::json!({
            "udid": "target", "attemptedHandle": "Account",
            "expectedHandle": "old.account", "currentHandle": "old.account",
            "conflictsTruncated": false,
            "conflictingDevices": [
                {"udid": "owner-a", "number": 8, "alias": "Kệ A", "handle": " @Account "},
                {"udid": "owner-b", "number": null, "alias": "", "handle": "ACCOUNT"}
            ]
        }));
        assert_eq!(db.get_device_meta("target").unwrap().handle, "old.account");
        let stale = db.set_device_handle("target", "stale", "Account").unwrap_err();
        let stale = serde_json::to_value(CommandError::from_service(stale)).unwrap();
        assert_eq!(stale["code"], "OperationFailed");
        assert!(stale.get("accountConflict").is_none());
        for index in 0..21 {
            let mut meta = db.get_device_meta(&format!("duplicate-{index:02}")).unwrap();
            meta.handle = "Account".into();
            db.upsert_device_meta(&meta).unwrap();
        }
        let collision = db.set_device_handle("target", "old.account", "Account").unwrap_err();
        let json = serde_json::to_value(CommandError::from_service(collision)).unwrap();
        assert_eq!(json["accountConflict"]["conflictingDevices"].as_array().unwrap().len(), 20);
        assert_eq!(json["accountConflict"]["conflictsTruncated"], true);
        assert_eq!(db.get_device_meta("target").unwrap().handle, "old.account");
        drop(db);
        let _ = std::fs::remove_file(path);
    }

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

    #[test]
    fn ambiguous_app_driver_error_preserves_device_and_requests_selection() {
        let error = CommandError::from(DeviceControlError::Driver {
            udid: "dual-phone".into(),
            operation: "resolveTikTokPackage",
            message: "more than one measured TikTok build is installed".into(),
        });
        assert_eq!(error.code, "DeviceAppSelectionRequired");
        assert_eq!(error.udid.as_deref(), Some("dual-phone"));
        assert!(error.message.contains("resolveTikTokPackage"));

        let ordinary = CommandError::from(DeviceControlError::Driver {
            udid: "other-phone".into(),
            operation: "readHierarchy",
            message: "unreadable".into(),
        });
        assert_eq!(ordinary.code, "DeviceControlFailed");
        assert_eq!(ordinary.udid.as_deref(), Some("other-phone"));
    }
}
