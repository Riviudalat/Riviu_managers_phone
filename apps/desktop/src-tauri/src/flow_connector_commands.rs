use riviu_core::flow::connectors::{FileDataFormat, FileWriteConfig, FlowConnectorExecutor};
use serde::Serialize;
use tauri::State;

use crate::{command_error::CommandError, state::AppState};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FlowConnectorInfo {
    root: String,
    credential_names: Vec<String>,
    sheet_configured: bool,
}

#[tauri::command]
pub fn flow_connector_info(state: State<'_, AppState>) -> Result<FlowConnectorInfo, CommandError> {
    let settings = state
        .db
        .publish_sheet_delivery_settings()
        .map_err(CommandError::operation)?;
    Ok(FlowConnectorInfo {
        root: state.db.flow_connector_root().display().to_string(),
        credential_names: state
            .db
            .list_flow_connector_secrets()
            .map_err(CommandError::operation)?,
        sheet_configured: riviu_core::publish_sheet::is_acceptable_webhook(&settings.webhook_url)
            && !settings.token.is_empty(),
    })
}

#[tauri::command]
pub fn flow_connector_save_secret(
    state: State<'_, AppState>,
    name: String,
    value: String,
) -> Result<(), CommandError> {
    let _admission = state.ensure_accepting_work()?;
    state
        .db
        .set_flow_connector_secret(&name, &value)
        .map_err(CommandError::operation)
}

/// Explicit operator upload into the connector folder. It has no device effect.
#[tauri::command]
pub async fn flow_connector_import_file(
    state: State<'_, AppState>,
    path: String,
    content: String,
) -> Result<(), CommandError> {
    let _admission = state.ensure_accepting_work()?;
    let executor = FlowConnectorExecutor::new(state.db.flow_connector_root(), state.db.clone())
        .map_err(CommandError::operation)?;
    executor
        .file_write(&FileWriteConfig {
            name: "uploaded".into(),
            path,
            value: content,
            format: FileDataFormat::Text,
        })
        .await
        .map_err(CommandError::operation)?;
    Ok(())
}
