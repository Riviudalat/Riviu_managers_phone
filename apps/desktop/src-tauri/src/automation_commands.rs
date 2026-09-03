use riviu_core::{
    AutomationDefinition, AutomationDefinitionRecord, AutomationKind, AutomationSchedule,
    AutomationScheduleV1, TargetRef,
};
use serde_json::Value;
use tauri::State;
use uuid::Uuid;

use crate::command_error::CommandError;
use crate::state::AppState;

#[tauri::command]
pub fn automation_list(
    state: State<'_, AppState>,
    include_archived: bool,
) -> Result<Vec<AutomationDefinition>, CommandError> {
    state
        .db
        .list_automation_definitions(include_archived)
        .map_err(CommandError::from_service)
}

#[tauri::command]
pub fn automation_get(
    state: State<'_, AppState>,
    definition_id: String,
    revision: u64,
) -> Result<Option<AutomationDefinitionRecord>, CommandError> {
    state
        .db
        .get_automation_definition_record(parse_uuid(&definition_id, "profile ID")?, revision)
        .map_err(CommandError::from_service)
}

#[tauri::command]
pub fn automation_create(
    state: State<'_, AppState>,
    name: String,
    kind: AutomationKind,
    target: TargetRef,
    config: Value,
) -> Result<AutomationDefinitionRecord, CommandError> {
    let _admission = state.ensure_accepting_work()?;
    state
        .db
        .create_automation_definition(&name, kind, &target, &config)
        .map_err(CommandError::from_service)
}

#[tauri::command]
pub fn automation_revise(
    state: State<'_, AppState>,
    definition_id: String,
    expected_revision: u64,
    target: TargetRef,
    config: Value,
) -> Result<AutomationDefinitionRecord, CommandError> {
    let _admission = state.ensure_accepting_work()?;
    state
        .db
        .revise_automation_definition(
            parse_uuid(&definition_id, "profile ID")?,
            expected_revision,
            &target,
            &config,
        )
        .map_err(CommandError::from_service)
}

#[tauri::command]
pub fn automation_archive(
    state: State<'_, AppState>,
    definition_id: String,
) -> Result<(), CommandError> {
    let _admission = state.ensure_accepting_work()?;
    state
        .db
        .archive_automation_definition(parse_uuid(&definition_id, "profile ID")?)
        .map_err(CommandError::from_service)
}

#[tauri::command]
pub fn automation_schedule_list(
    state: State<'_, AppState>,
) -> Result<Vec<AutomationSchedule>, CommandError> {
    state
        .db
        .list_automation_schedules()
        .map_err(CommandError::from_service)
}

#[tauri::command]
pub fn automation_schedule_create(
    state: State<'_, AppState>,
    name: String,
    definition_id: String,
    definition_revision: u64,
    enabled: bool,
    schedule: AutomationScheduleV1,
) -> Result<AutomationSchedule, CommandError> {
    let _admission = state.ensure_accepting_work()?;
    state
        .db
        .create_automation_schedule(
            &name,
            parse_uuid(&definition_id, "profile ID")?,
            definition_revision,
            enabled,
            &schedule,
        )
        .map_err(CommandError::from_service)
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub fn automation_schedule_update(
    state: State<'_, AppState>,
    schedule_id: String,
    expected_revision: u64,
    name: String,
    definition_id: String,
    definition_revision: u64,
    enabled: bool,
    schedule: AutomationScheduleV1,
) -> Result<AutomationSchedule, CommandError> {
    let _admission = state.ensure_accepting_work()?;
    state
        .db
        .update_automation_schedule(
            parse_uuid(&schedule_id, "schedule ID")?,
            expected_revision,
            &name,
            parse_uuid(&definition_id, "profile ID")?,
            definition_revision,
            enabled,
            &schedule,
        )
        .map_err(CommandError::from_service)
}

fn parse_uuid(value: &str, field: &str) -> Result<Uuid, CommandError> {
    Uuid::parse_str(value)
        .map_err(|_| CommandError::invalid_argument(format!("{field} is not a valid UUID")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_ids_have_a_stable_argument_error() {
        let error = parse_uuid("not-an-id", "profile ID").expect_err("invalid UUID");
        assert_eq!(error.code, "InvalidArgument");
    }
}
