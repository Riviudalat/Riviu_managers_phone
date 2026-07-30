use riviu_core::{DeviceControlError, DeviceWorkOwner};
use serde::Serialize;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CommandError {
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub udid: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub requested_owner: Option<DeviceWorkOwner>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_owner: Option<DeviceWorkOwner>,
}

impl CommandError {
    pub fn operation(error: impl std::fmt::Display) -> Self {
        Self {
            code: "OperationFailed".to_string(),
            message: error.to_string(),
            udid: None,
            requested_owner: None,
            current_owner: None,
        }
    }
}

impl From<DeviceControlError> for CommandError {
    fn from(error: DeviceControlError) -> Self {
        match error {
            DeviceControlError::Busy(busy) => Self {
                code: "DeviceBusy".to_string(),
                message: busy.to_string(),
                udid: Some(busy.udid),
                requested_owner: Some(busy.requested_owner),
                current_owner: Some(busy.current_owner),
            },
            other => Self {
                code: "DeviceControlFailed".to_string(),
                message: other.to_string(),
                udid: None,
                requested_owner: None,
                current_owner: None,
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
