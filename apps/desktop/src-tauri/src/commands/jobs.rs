//! The script/job queue.

use super::*;

use std::collections::BTreeMap;

use riviu_core::{
    nurture_source_id, project_flow_detail, project_flow_summary, project_interaction_detail,
    project_interaction_summary, project_job, project_nurture, project_orchestration_detail,
    project_orchestration_summary, project_publish_detail, project_publish_summary,
    NurtureSessionStatus, OperationRunDetail, OperationRunState, OperationRunSummary,
};

fn merge_nurture_history(
    persisted: Vec<riviu_core::db::NurtureRunHistory>,
    live: Vec<NurtureSessionStatus>,
) -> BTreeMap<String, Vec<NurtureSessionStatus>> {
    let mut runs = persisted
        .into_iter()
        .map(|history| (history.run_id.to_string(), history.statuses))
        .collect::<BTreeMap<_, _>>();
    for status in live {
        let sessions = runs.entry(nurture_source_id(&status)).or_default();
        match sessions
            .iter_mut()
            .find(|persisted| persisted.udid == status.udid)
        {
            Some(persisted) => *persisted = status,
            None => sessions.push(status),
        }
    }
    runs
}

#[tauri::command]
pub fn list_jobs(state: State<'_, AppState>) -> Result<Vec<JobRecord>, CommandError> {
    state.jobs.list_jobs(100).map_err(err)
}

/// Read every persisted/runtime work source through one normalized operations contract.
/// The source rows remain authoritative; this command creates no history of its own.
#[tauri::command]
pub fn operation_list_runs(
    state: State<'_, AppState>,
    limit: Option<usize>,
) -> Result<Vec<OperationRunSummary>, CommandError> {
    let limit = limit.unwrap_or(100).clamp(1, 200);
    let mut runs = Vec::new();

    runs.extend(
        state
            .jobs
            .list_jobs(limit)
            .map_err(err)?
            .iter()
            .map(|job| project_job(job).summary),
    );
    for run in state.db.list_flow_runs(limit).map_err(err)? {
        let title = state
            .db
            .get_flow_revision(run.flow_id, Some(run.flow_revision))
            .map_err(err)?
            .map(|revision| revision.document.name)
            .unwrap_or_else(|| "Flow thiết bị".to_string());
        let summary = state
            .db
            .get_flow_run(run.id)
            .map_err(err)?
            .map(|detail| project_flow_detail(&detail, title.clone()).summary)
            .unwrap_or_else(|| project_flow_summary(&run, title));
        runs.push(summary);
    }
    for run in state.db.list_orchestration_runs(limit).map_err(err)? {
        let title = state
            .db
            .get_orchestration_revision(run.document_id, Some(run.document_revision))
            .map_err(err)?
            .map(|revision| revision.compiled.document.name)
            .unwrap_or_else(|| "Điều phối".to_string());
        let summary = state
            .db
            .get_orchestration_run(run.id)
            .map_err(err)?
            .map(|detail| project_orchestration_detail(&detail, title.clone()).summary)
            .unwrap_or_else(|| project_orchestration_summary(&run, title));
        runs.push(summary);
    }

    let nurture_runs = merge_nurture_history(
        state.db.list_nurture_runs(limit).map_err(err)?,
        state.nurture.list_status(),
    );
    runs.extend(
        nurture_runs
            .iter()
            .map(|(source_id, sessions)| project_nurture(source_id, sessions).summary),
    );

    for campaign in state.db.list_interaction_campaigns(limit).map_err(err)? {
        let summary = state
            .db
            .get_interaction_campaign(&campaign.id)
            .map_err(err)?
            .map(|detail| project_interaction_detail(&detail).summary)
            .unwrap_or_else(|| project_interaction_summary(&campaign));
        runs.push(summary);
    }
    for campaign in state.db.list_publish_campaigns(limit).map_err(err)? {
        let Some(detail) = state.db.get_publish_campaign(&campaign.id).map_err(err)? else {
            continue;
        };
        let snapshot = state
            .db
            .get_publish_execution_snapshot(&campaign.id)
            .map_err(err)?;
        runs.push(project_publish_summary(&detail, snapshot.as_ref()));
    }

    runs.sort_by(|left, right| {
        let left_active = matches!(
            left.state,
            OperationRunState::Queued | OperationRunState::Running
        );
        let right_active = matches!(
            right.state,
            OperationRunState::Queued | OperationRunState::Running
        );
        right_active
            .cmp(&left_active)
            .then_with(|| {
                right
                    .updated_at
                    .as_deref()
                    .unwrap_or_default()
                    .cmp(left.updated_at.as_deref().unwrap_or_default())
            })
            .then_with(|| left.id.cmp(&right.id))
    });
    runs.truncate(limit);
    Ok(runs)
}

#[tauri::command]
pub fn operation_get_run(
    state: State<'_, AppState>,
    operation_id: String,
) -> Result<Option<OperationRunDetail>, CommandError> {
    let (kind, source_id) = operation_id.split_once(':').ok_or_else(|| {
        CommandError::invalid_argument("operationId must contain a source prefix")
    })?;
    if source_id.is_empty() {
        return Err(CommandError::invalid_argument(
            "operationId source ID is required",
        ));
    }
    match kind {
        "script" => {
            let id = uuid::Uuid::parse_str(source_id).map_err(|_| {
                CommandError::invalid_argument("script operation ID must be a UUID")
            })?;
            Ok(state.db.get_job(id).map_err(err)?.as_ref().map(project_job))
        }
        "flow" => {
            let id = uuid::Uuid::parse_str(source_id)
                .map_err(|_| CommandError::invalid_argument("Flow operation ID must be a UUID"))?;
            let Some(detail) = state.db.get_flow_run(id).map_err(err)? else {
                return Ok(None);
            };
            let title = state
                .db
                .get_flow_revision(detail.run.flow_id, Some(detail.run.flow_revision))
                .map_err(err)?
                .map(|revision| revision.document.name)
                .unwrap_or_else(|| "Flow thiết bị".to_string());
            Ok(Some(project_flow_detail(&detail, title)))
        }
        "orchestration" => {
            let id = uuid::Uuid::parse_str(source_id).map_err(|_| {
                CommandError::invalid_argument("orchestration operation ID must be a UUID")
            })?;
            let Some(detail) = state.db.get_orchestration_run(id).map_err(err)? else {
                return Ok(None);
            };
            let title = state
                .db
                .get_orchestration_revision(
                    detail.run.document_id,
                    Some(detail.run.document_revision),
                )
                .map_err(err)?
                .map(|revision| revision.compiled.document.name)
                .unwrap_or_else(|| "Điều phối".to_string());
            Ok(Some(project_orchestration_detail(&detail, title)))
        }
        "nurture" => {
            let live = state
                .nurture
                .list_status()
                .into_iter()
                .filter(|status| nurture_source_id(status) == source_id)
                .collect::<Vec<_>>();
            let persisted = uuid::Uuid::parse_str(source_id)
                .ok()
                .map(|run_id| state.db.get_nurture_run(run_id))
                .transpose()
                .map_err(err)?
                .flatten()
                .into_iter()
                .collect::<Vec<_>>();
            let sessions = merge_nurture_history(persisted, live)
                .remove(source_id)
                .unwrap_or_default();
            Ok((!sessions.is_empty()).then(|| project_nurture(source_id, &sessions)))
        }
        "interaction" => Ok(state
            .db
            .get_interaction_campaign(source_id)
            .map_err(err)?
            .as_ref()
            .map(project_interaction_detail)),
        "publish" => {
            let Some(detail) = state.db.get_publish_campaign(source_id).map_err(err)? else {
                return Ok(None);
            };
            let snapshot = state
                .db
                .get_publish_execution_snapshot(source_id)
                .map_err(err)?;
            Ok(Some(project_publish_detail(&detail, snapshot.as_ref())))
        }
        _ => Err(CommandError::invalid_argument(
            "operationId source prefix is unknown",
        )),
    }
}

#[tauri::command]
pub async fn run_script(
    state: State<'_, AppState>,
    script_json: String,
    udids: Vec<String>,
) -> Result<JobRecord, CommandError> {
    let _admission = state.ensure_accepting_work()?;
    let script: AutomationScript = parse_script(&script_json).map_err(err)?;
    state.jobs.enqueue(script, udids).await.map_err(err)
}

#[tauri::command]
pub fn cancel_job(state: State<'_, AppState>, job_id: String) -> Result<(), CommandError> {
    let _admission = state.ensure_accepting_work()?;
    let id = uuid::Uuid::parse_str(&job_id).map_err(err)?;
    state.jobs.cancel(id);
    Ok(())
}

#[tauri::command]
pub fn list_scripts(state: State<'_, AppState>) -> Result<Vec<(String, String)>, CommandError> {
    state.db.list_scripts().map_err(err)
}

#[tauri::command]
pub fn save_script(
    state: State<'_, AppState>,
    name: String,
    body_json: String,
) -> Result<(), CommandError> {
    let _admission = state.ensure_accepting_work()?;
    parse_script(&body_json).map_err(err)?;
    state.db.save_script(&name, &body_json).map_err(err)
}

#[tauri::command]
pub fn example_script() -> String {
    example_script_json().to_string()
}

#[cfg(test)]
mod nurture_history_tests {
    use super::*;
    use chrono::{TimeZone, Utc};

    fn status(
        run_id: uuid::Uuid,
        udid: &str,
        videos_done: u32,
        second: u32,
    ) -> NurtureSessionStatus {
        NurtureSessionStatus {
            run_id: Some(run_id),
            run_size: 2,
            running: true,
            videos_done,
            updated_at: Some(
                Utc.with_ymd_and_hms(2026, 9, 5, 10, 0, second)
                    .single()
                    .expect("fixed timestamp"),
            ),
            ..NurtureSessionStatus::new(udid)
        }
    }

    #[test]
    fn live_nurture_status_overlays_only_its_matching_persisted_device() {
        let run_id = uuid::Uuid::new_v4();
        let persisted_a = status(run_id, "phone-a", 1, 1);
        let persisted_b = status(run_id, "phone-b", 2, 2);
        let live_a = status(run_id, "phone-a", 9, 9);
        let histories = vec![riviu_core::db::NurtureRunHistory {
            run_id,
            target_udids: vec!["phone-a".into(), "phone-b".into()],
            statuses: vec![persisted_a, persisted_b.clone()],
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }];

        let merged = merge_nurture_history(histories, vec![live_a.clone()]);
        let sessions = merged.get(&run_id.to_string()).expect("merged run");
        assert_eq!(sessions.len(), 2);
        assert_eq!(
            sessions
                .iter()
                .find(|status| status.udid == "phone-a")
                .expect("live phone")
                .videos_done,
            live_a.videos_done
        );
        assert_eq!(
            sessions
                .iter()
                .find(|status| status.udid == "phone-b")
                .expect("persisted phone")
                .videos_done,
            persisted_b.videos_done
        );
        assert_eq!(
            project_nurture(&run_id.to_string(), sessions).summary.state,
            OperationRunState::Running
        );
    }
}
