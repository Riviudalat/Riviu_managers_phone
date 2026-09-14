use crate::{command_error::CommandError, state::AppState};
use riviu_core::operator_workspace::{OperatorRecord, OperatorRecordInput, OperatorRecordKind};
use tauri::State;
use uuid::Uuid;

#[tauri::command]
pub fn operator_list(
    state: State<'_, AppState>,
    kind: OperatorRecordKind,
    search: String,
) -> Result<Vec<OperatorRecord>, CommandError> {
    let _admission = state.ensure_accepting_work()?;
    state
        .db
        .operator_list(kind, &search)
        .map_err(CommandError::from_service)
}
#[tauri::command]
pub fn operator_import(
    state: State<'_, AppState>,
    inputs: Vec<OperatorRecordInput>,
) -> Result<Vec<OperatorRecord>, CommandError> {
    let _admission = state.ensure_accepting_work()?;
    state
        .db
        .operator_import(inputs)
        .map_err(CommandError::from_service)
}
#[tauri::command]
pub fn operator_save(
    state: State<'_, AppState>,
    input: OperatorRecordInput,
) -> Result<OperatorRecord, CommandError> {
    let _admission = state.ensure_accepting_work()?;
    state
        .db
        .operator_save(input)
        .map_err(CommandError::from_service)
}
#[tauri::command]
pub fn operator_archive(
    state: State<'_, AppState>,
    id: Uuid,
    expected_revision: u64,
) -> Result<(), CommandError> {
    let _admission = state.ensure_accepting_work()?;
    state
        .db
        .operator_archive(id, expected_revision)
        .map_err(CommandError::from_service)
}

#[tauri::command]
pub async fn operator_network_probe(
    state: State<'_, AppState>,
    id: Uuid,
) -> Result<serde_json::Value, CommandError> {
    let _admission = state.ensure_accepting_work()?;
    let record = state
        .db
        .operator_get(id)
        .map_err(CommandError::from_service)?
        .ok_or_else(|| CommandError::invalid_argument("Network profile missing"))?;
    if record.kind != OperatorRecordKind::Network || record.archived {
        return Err(CommandError::invalid_argument(
            "Network profile is not active",
        ));
    }
    let host = record.data["host"]
        .as_str()
        .ok_or_else(|| CommandError::invalid_argument("Host missing"))?
        .to_string();
    let port = record.data["port"]
        .as_u64()
        .filter(|port| (1..=65535).contains(port))
        .ok_or_else(|| CommandError::invalid_argument("Port invalid"))? as u16;
    let started = std::time::Instant::now();
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(8),
        tokio::net::TcpStream::connect((host.as_str(), port)),
    )
    .await;
    match result {
        Ok(Ok(stream)) => {
            drop(stream);
            Ok(
                serde_json::json!({"reachable":true,"elapsedMs":started.elapsed().as_millis(),"host":host,"port":port,"protocolValidated":false}),
            )
        }
        Ok(Err(error)) => Err(CommandError::from_service(anyhow::anyhow!(
            "TCP connection failed: {error}"
        ))),
        Err(_) => Err(CommandError::from_service(anyhow::anyhow!(
            "TCP connection timed out after 8 seconds"
        ))),
    }
}

#[tauri::command]
pub async fn operator_network_apply(
    state: State<'_, AppState>,
    id: Uuid,
    clear: bool,
) -> Result<Vec<serde_json::Value>, CommandError> {
    let _admission = state.ensure_accepting_work()?;
    let record = state
        .db
        .operator_get(id)
        .map_err(CommandError::from_service)?
        .ok_or_else(|| CommandError::invalid_argument("Network profile missing"))?;
    if record.kind != OperatorRecordKind::Network || record.archived {
        return Err(CommandError::invalid_argument(
            "Network profile is not active",
        ));
    }
    if !clear
        && (record.data["protocol"] != "http"
            || !record.data["credentialRef"]
                .as_str()
                .unwrap_or_default()
                .is_empty())
    {
        return Err(CommandError::invalid_argument("Android system proxy requires HTTP without authentication; use a provider adapter for other protocols"));
    }
    let endpoint = format!(
        "{}:{}",
        record.data["host"].as_str().unwrap_or_default(),
        record.data["port"].as_u64().unwrap_or_default()
    );
    let targets = record.data["deviceIds"]
        .as_array()
        .ok_or_else(|| CommandError::invalid_argument("Select devices in the network profile"))?;
    if targets.is_empty() || targets.len() > 200 {
        return Err(CommandError::invalid_argument("Select 1-200 devices"));
    }
    let mut results = Vec::new();
    for target in targets {
        let udid = target
            .as_str()
            .ok_or_else(|| CommandError::invalid_argument("Invalid device ID"))?;
        let applied = async {
            let lease = state
                .device_lease(
                    udid,
                    riviu_core::DeviceWorkOwner::ManualControl,
                    crate::state::LeaseStream::Keep,
                )
                .await?;
            state
                .control
                .set_http_proxy(&lease, if clear { None } else { Some(endpoint.as_str()) })
                .await
                .map_err(CommandError::from)
        }
        .await;
        results.push(match applied {
            Ok(observed) => serde_json::json!({"udid":udid,"confirmed":true,"observed":observed}),
            Err(error) => serde_json::json!({"udid":udid,"confirmed":false,"error":error}),
        });
    }
    Ok(results)
}
