use crate::{command_error::CommandError, state::AppState};
use riviu_core::{app_workflow::*, AutomationKind};
use tauri::{AppHandle, State};
use uuid::Uuid;

#[tauri::command]
pub fn app_workflow_list(
    state: State<'_, AppState>,
) -> Result<Vec<AppWorkflowSummary>, CommandError> {
    let _admission = state.ensure_accepting_work()?;
    state
        .db
        .list_app_workflows()
        .map_err(CommandError::from_service)
}
#[tauri::command]
pub fn app_workflow_template(
    state: State<'_, AppState>,
    kind: AutomationKind,
) -> Result<AppWorkflowV1, CommandError> {
    let _admission = state.ensure_accepting_work()?;
    Ok(new_app_workflow(kind))
}
#[tauri::command]
pub fn app_workflow_catalog(
    state: State<'_, AppState>,
    kind: AutomationKind,
) -> Result<Vec<AppStepDefinition>, CommandError> {
    let _admission = state.ensure_accepting_work()?;
    Ok(app_step_catalog(kind))
}
#[tauri::command]
pub fn app_workflow_get(
    state: State<'_, AppState>,
    id: Uuid,
    revision: Option<u64>,
) -> Result<Option<AppWorkflowV1>, CommandError> {
    let _admission = state.ensure_accepting_work()?;
    state
        .db
        .get_app_workflow(id, revision)
        .map_err(CommandError::from_service)
}
#[tauri::command]
pub fn app_workflow_validate(
    state: State<'_, AppState>,
    document: AppWorkflowV1,
) -> Result<(), CommandError> {
    let _admission = state.ensure_accepting_work()?;
    compile_app_profile(&document)
        .map(|_| ())
        .map_err(CommandError::from_service)
}
#[tauri::command]
pub fn app_workflow_save(
    state: State<'_, AppState>,
    document: AppWorkflowV1,
    expected_revision: Option<u64>,
) -> Result<AppWorkflowV1, CommandError> {
    let _admission = state.ensure_accepting_work()?;
    state
        .db
        .save_app_workflow(document, expected_revision)
        .map_err(CommandError::from_service)
}
#[tauri::command]
pub fn app_workflow_archive(
    state: State<'_, AppState>,
    id: Uuid,
    revision: u64,
) -> Result<(), CommandError> {
    let _admission = state.ensure_accepting_work()?;
    state
        .db
        .archive_app_workflow(id, revision)
        .map_err(CommandError::from_service)
}

#[tauri::command]
pub fn app_workflow_schedule(
    state: State<'_, AppState>,
    id: Uuid,
    revision: u64,
    target: riviu_core::TargetRef,
    name: String,
    every_minutes: u32,
) -> Result<riviu_core::AutomationSchedule, CommandError> {
    let _admission = state.ensure_accepting_work()?;
    let doc = state
        .db
        .get_app_workflow(id, Some(revision))
        .map_err(CommandError::from_service)?
        .ok_or_else(|| CommandError::invalid_argument("App revision does not exist"))?;
    let mut config = compile_app_profile(&doc).map_err(CommandError::from_service)?;
    if doc.kind == AutomationKind::Publish {
        config["executionConfirmed"] = serde_json::json!(true);
    }
    if doc
        .nodes
        .iter()
        .any(|node| matches!(node.action.as_str(), "wait" | "log"))
    {
        return Err(CommandError::invalid_argument(
            "Scheduled native profiles do not support surrounding wait/log steps yet",
        ));
    }
    let schedule = serde_json::from_value(
        serde_json::json!({"schemaVersion":1,"kind":"interval","everyMinutes":every_minutes}),
    )
    .map_err(|error| CommandError::from_service(error.into()))?;
    state
        .db
        .create_automation_schedule_from_settings(&name, doc.kind, &target, &config, &schedule)
        .map_err(CommandError::from_service)
}

#[tauri::command]
pub async fn app_workflow_run(
    app: AppHandle,
    state: State<'_, AppState>,
    id: Uuid,
    revision: u64,
    target: riviu_core::TargetRef,
) -> Result<riviu_core::OrchestrationRunDetail, CommandError> {
    let _admission = state.ensure_accepting_work()?;
    let document = state
        .db
        .get_app_workflow(id, Some(revision))
        .map_err(CommandError::from_service)?
        .ok_or_else(|| CommandError::invalid_argument("App revision does not exist"))?;
    let mut config = compile_app_profile(&document).map_err(CommandError::from_service)?;
    if document.kind == AutomationKind::Publish {
        config["executionConfirmed"] = serde_json::json!(true);
    }
    let profile = state
        .db
        .create_automation_definition(
            &format!("{} · bản {}", document.name, revision),
            document.kind,
            &target,
            &config,
        )
        .map_err(CommandError::from_service)?;
    let orchestration = compile_app_orchestration(
        &document,
        riviu_core::AutomationProfileRef {
            definition_id: profile.definition.id,
            revision: profile.revision.revision,
        },
    )
    .map_err(CommandError::from_service)?;
    let saved = crate::orchestration_commands::orchestration_save_revision(
        state.clone(),
        orchestration,
        None,
    )?;
    crate::orchestration_commands::orchestration_run(
        app,
        state,
        saved.compiled.document.id.to_string(),
        saved.compiled.document.revision,
        target,
    )
    .await
}
