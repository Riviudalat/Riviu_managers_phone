use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct StopDeviceResult {
    pub udid: String,
    pub closed: bool,
    pub message: String,
}

#[derive(Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct OperationStopResult {
    pub operation_id: String,
    pub state: String,
    pub devices: Vec<StopDeviceResult>,
    #[serde(default)]
    pub stop_marker: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct DeviceAppChoices {
    pub udid: String,
    pub app_key: String,
    pub installed_packages: Vec<String>,
    pub selected_package: Option<String>,
    pub suggested_package: Option<String>,
    pub revision: i64,
    pub selection_valid: bool,
    pub reason: Option<String>,
}

#[derive(Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct PublishSheetReadback {
    pub assignment_id: String,
    pub url: String,
    pub range: String,
    pub revision: i64,
    pub epoch: String,
    pub checked_at: String,
    pub receipt: crate::google_sheets::DeliveryReceipt,
}

#[derive(Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct PublishSheetFailedDiagnostic {
    pub assignment_id: String,
    pub expected_revision: i64,
    pub checked_at: String,
    pub slice: crate::google_sheets::SheetDiagnosticSlice,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum CapabilityEvidenceState {
    Measured,
    RuntimeProofRequired,
    Unsupported,
    DeviceNotReady,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ActionCapability {
    pub action: String,
    pub state: CapabilityEvidenceState,
    pub reason: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct DeviceActionCapabilities {
    pub udid: String,
    pub package: String,
    pub version: String,
    pub locale: String,
    pub actions: Vec<ActionCapability>,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct TraceArtifact {
    pub path: String,
    pub sha256: String,
    pub bytes: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct OperationTrace {
    pub run_id: String,
    pub device_id: String,
    pub session_id: Option<String>,
    pub observed_at: String,
    pub steps: Vec<crate::operation::OperationDeviceLogEntry>,
    pub artifacts: Vec<TraceArtifact>,
    #[serde(default)]
    pub observations: Vec<crate::ui_automation::trace::DeviceTraceStep>,
}
