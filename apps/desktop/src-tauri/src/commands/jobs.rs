//! The script/job queue.

use super::*;

#[tauri::command]
pub async fn operation_trace_export(
    state: State<'_, AppState>,
    operation_id: String,
    udid: String,
) -> Result<riviu_core::ipc_contract::TraceArtifact, CommandError> {
    let _admission = state.ensure_accepting_work()?;
    let live = state.nurture.list_status();
    let scope_live = live.clone();
    let scope_operation = operation_id.clone();
    let scope_device = udid.clone();
    let sources = state
        .db
        .storage_read(move |db| {
            Ok((|| {
                let detail = read_operation_run_from(db, &scope_live, &scope_operation)?
                    .ok_or_else(|| CommandError::invalid_argument("operation no longer exists"))?;
                operation_trace_sources(db, &detail, &scope_device)
            })())
        })
        .await
        .map_err(err)??;
    if let Some(android) = &state.android {
        for source in &sources {
            android
                .flush_traces_for_run(source, &udid)
                .await
                .map_err(err)?;
        }
    }
    let artifacts_root = state.artifacts_dir.clone();
    state
        .db
        .storage_read(move |db| {
            Ok(export_operation_trace(
                db,
                &live,
                &artifacts_root,
                &operation_id,
                &udid,
                &sources,
            ))
        })
        .await
        .map_err(err)?
}

fn operation_trace_sources(
    db: &riviu_core::db::Database,
    detail: &riviu_core::OperationRunDetail,
    udid: &str,
) -> Result<Vec<String>, CommandError> {
    let mut sources = vec![detail.summary.source_id.clone()];
    if detail.summary.kind == riviu_core::OperationRunKind::Orchestration {
        let source = db
            .get_orchestration_run(uuid::Uuid::parse_str(&detail.summary.source_id).map_err(err)?)
            .map_err(err)?
            .ok_or_else(|| CommandError::invalid_argument("orchestration source missing"))?;
        for attempt in source
            .attempts
            .iter()
            .filter(|a| a.snapshot.target.included.iter().any(|d| d.udid == udid))
        {
            if let Some((_, id)) = orchestration_child_source(db, attempt)? {
                sources.push(id);
            }
        }
    }
    sources.sort();
    sources.dedup();
    Ok(sources)
}

fn export_operation_trace(
    db: &riviu_core::db::Database,
    live: &[NurtureSessionStatus],
    artifacts_root: &Path,
    operation_id: &str,
    udid: &str,
    flushed_sources: &[String],
) -> Result<riviu_core::ipc_contract::TraceArtifact, CommandError> {
    use riviu_core::ipc_contract::{OperationTrace, TraceArtifact};
    use sha2::Digest;
    let detail = read_operation_run_from(db, live, operation_id)?
        .ok_or_else(|| CommandError::invalid_argument("operation no longer exists"))?;
    let mut logs = read_operation_device_log(db, live, operation_id, udid)?;
    let root = artifacts_root.canonicalize().map_err(err)?;
    let sources = operation_trace_sources(db, &detail, udid)?;
    if sources != flushed_sources {
        return Err(CommandError::invalid_argument(
            "Trace scope changed during export; retry export",
        ));
    }
    let mut observations = Vec::new();
    for source in &sources {
        observations.extend(
            riviu_core::ui_automation::trace::read_run(&root.join("traces"), source, udid)
                .map_err(err)?,
        );
    }
    if detail.summary.kind == riviu_core::OperationRunKind::Orchestration {
        observations.sort_by(|a, b| {
            a.observed_at
                .cmp(&b.observed_at)
                .then(a.sequence.cmp(&b.sequence))
        });
        observations.dedup_by(|a, b| {
            a.run_id == b.run_id && a.session_id == b.session_id && a.sequence == b.sequence
        });
    }
    let mut artifacts = observations
        .iter()
        .flat_map(|step| {
            [&step.hierarchy, &step.image]
                .into_iter()
                .flatten()
                .cloned()
        })
        .collect::<Vec<_>>();
    let mut paths = detail
        .items
        .iter()
        .filter(|row| row.udid.as_deref().is_none_or(|id| id == udid))
        .filter_map(|row| row.evidence.as_ref().map(PathBuf::from))
        .collect::<Vec<_>>();
    if detail.summary.kind == riviu_core::OperationRunKind::Script {
        paths.extend(
            logs.entries
                .iter()
                .filter_map(|row| row.detail.as_deref())
                .filter_map(|raw| serde_json::from_str::<serde_json::Value>(raw).ok())
                .filter_map(|v| v["artifact"].as_str().map(PathBuf::from)),
        );
    }
    for path in paths.into_iter().filter(|p| p.is_file()) {
        {
            let path = path.canonicalize().map_err(err)?;
            if !path.starts_with(&root) {
                continue;
            }
            if detail.summary.kind == riviu_core::OperationRunKind::Script
                && !path.starts_with(root.join(&detail.summary.source_id).join(udid))
            {
                continue;
            }
            let bytes = std::fs::read(&path).map_err(err)?;
            artifacts.push(TraceArtifact {
                path: path.to_string_lossy().into_owned(),
                sha256: format!("{:x}", sha2::Sha256::digest(&bytes)),
                bytes: bytes.len() as u64,
            });
        }
    }
    // Assignment evidence is persisted inline, not a filesystem path. Export only
    // this device's diagnostic projection; never borrow unscoped/sibling rows.
    if detail.summary.kind == riviu_core::OperationRunKind::Publish {
        for row in detail.items.iter().filter(|row| {
            row.kind == riviu_core::OperationRunItemKind::Assignment
                && row.udid.as_deref() == Some(udid)
        }) {
            let Some(evidence) = row.evidence.as_deref().and_then(persisted_trace_evidence) else {
                continue;
            };
            let mut bytes = serde_json::to_vec(&serde_json::json!({
                "runId": operation_id,
                "sourceId": detail.summary.source_id,
                "deviceId": udid,
                "assignmentId": row.id,
                "evidence": evidence,
            }))
            .map_err(err)?;
            if bytes.len() > 256 * 1024 {
                bytes = serde_json::to_vec(&serde_json::json!({
                    "runId": operation_id,
                    "sourceId": detail.summary.source_id,
                    "deviceId": udid,
                    "assignmentId": row.id,
                    "evidence": trace_evidence_omitted("exportByteLimit"),
                }))
                .map_err(err)?;
            }
            let sha256 = format!("{:x}", sha2::Sha256::digest(&bytes));
            // Keep the bounded diagnostic body in the exported trace too: a
            // copied log must remain useful without local artifact paths.
            logs.entries.push(riviu_core::OperationDeviceLogEntry {
                id: format!("assignmentEvidence:{}:{sha256}", row.id),
                at: detail.summary.updated_at.clone(),
                action: "assignmentEvidence".into(),
                state: "persisted".into(),
                text: Some("Bằng chứng chẩn đoán của lượt đăng".into()),
                detail: Some(String::from_utf8(bytes.clone()).map_err(err)?),
            });
            let directory = root.join("trace-exports").join("assignment-evidence");
            std::fs::create_dir_all(&directory).map_err(err)?;
            let directory = directory.canonicalize().map_err(err)?;
            if !directory.starts_with(&root) {
                return Err(CommandError::invalid_argument(
                    "evidence directory escaped root",
                ));
            }
            let path = directory.join(format!("{sha256}.json"));
            match std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
            {
                Ok(mut file) => {
                    use std::io::Write;
                    file.write_all(&bytes).map_err(err)?;
                    file.sync_all().map_err(err)?;
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(err(error)),
            }
            let path = path.canonicalize().map_err(err)?;
            if !path.starts_with(&directory)
                || std::fs::metadata(&path).map_err(err)?.len() != bytes.len() as u64
                || std::fs::read(&path).map_err(err)? != bytes
            {
                return Err(CommandError::invalid_argument(
                    "evidence artifact identity mismatch",
                ));
            }
            artifacts.push(TraceArtifact {
                path: path.to_string_lossy().into_owned(),
                sha256,
                bytes: bytes.len() as u64,
            });
        }
    }
    artifacts.sort_by(|a, b| a.path.cmp(&b.path));
    artifacts.dedup_by(|a, b| a.path == b.path);
    let sessions = observations
        .iter()
        .map(|s| s.session_id.clone())
        .collect::<std::collections::BTreeSet<_>>();
    let session_id = if sessions.len() == 1 {
        sessions.into_iter().next()
    } else {
        None
    };
    let trace = OperationTrace {
        run_id: operation_id.into(),
        device_id: udid.into(),
        session_id,
        observed_at: chrono::Utc::now().to_rfc3339(),
        steps: logs.entries,
        artifacts,
        observations,
    };
    // Export bundles have their own namespace in the existing store. Flow's
    // attempt reconciliation must not quarantine an exported trace on restart.
    let export_root = root.join("trace-exports");
    let store = riviu_core::FlowArtifactStore::new(&export_root).map_err(err)?;
    let run =
        uuid::Uuid::parse_str(&detail.summary.source_id).unwrap_or_else(|_| uuid::Uuid::new_v4());
    let artifact = store
        .prepare_trace(
            run,
            uuid::Uuid::new_v4(),
            uuid::Uuid::new_v4(),
            &serde_json::to_value(trace).map_err(err)?,
        )
        .map_err(err)?;
    let relative = store.publish_file(&artifact).map_err(err)?;
    Ok(TraceArtifact {
        path: export_root.join(relative).to_string_lossy().into_owned(),
        sha256: artifact.sha256,
        bytes: artifact.size,
    })
}

// Bounded, allowlisted projection: raw intent, captions, credentials and helper
// envelopes are never copied. priorEvidenceJson may be an object or JSON string.
fn persisted_trace_evidence(raw: &str) -> Option<serde_json::Value> {
    const MAX_BYTES: usize = 256 * 1024;
    if raw.len() > MAX_BYTES {
        return Some(trace_evidence_omitted("rawByteLimit"));
    }
    fn project(value: &serde_json::Value, depth: usize) -> Option<serde_json::Value> {
        if depth == 0 {
            return Some(trace_evidence_omitted("depthLimit"));
        }
        let mut fields = serde_json::Map::new();
        for key in [
            "selectionDiagnostic",
            "stageDiagnostics",
            "message",
            "mediaStage",
            "nativePrepare",
            "nativeImport",
        ] {
            if let Some(value) = value.get(key).and_then(|v| bounded_trace_value(v, 8)) {
                fields.insert(key.to_owned(), value);
            }
        }
        // Android staging evidence can be the top-level persisted value.
        if value.get("manifestSha256").is_some() {
            if let Some(stage) = bounded_trace_value(value, 8) {
                fields.insert("mediaStage".into(), stage);
            }
        }
        if let Some(prior) = value.get("priorEvidenceJson") {
            let projected = if let Some(raw) = prior.as_str() {
                if raw.len() > MAX_BYTES {
                    Some(trace_evidence_omitted("rawByteLimit"))
                } else {
                    serde_json::from_str::<serde_json::Value>(raw)
                        .ok()
                        .and_then(|prior| project(&prior, depth - 1))
                }
            } else {
                project(prior, depth - 1)
            };
            if let Some(prior) = projected {
                fields.insert("priorEvidenceJson".into(), prior);
            }
        }
        (!fields.is_empty()).then_some(serde_json::Value::Object(fields))
    }
    let value = serde_json::from_str::<serde_json::Value>(raw).ok()?;
    project(&value, 8)
}

fn trace_evidence_omitted(reason: &str) -> serde_json::Value {
    serde_json::json!({"truncated": true, "omitted": true, "reasonCode": reason})
}

fn bounded_trace_value(value: &serde_json::Value, depth: usize) -> Option<serde_json::Value> {
    use serde_json::Value;
    if depth == 0 {
        return Some(trace_evidence_omitted("depthLimit"));
    }
    match value {
        Value::String(s) if s.len() > 8192 => Some(trace_evidence_omitted("stringByteLimit")),
        Value::String(_) => Some(value.clone()),
        Value::Array(values) if values.len() <= 128 => values
            .iter()
            .map(|v| bounded_trace_value(v, depth - 1))
            .collect::<Option<Vec<_>>>()
            .map(Value::Array),
        Value::Array(_) => Some(trace_evidence_omitted("arrayLimit")),
        Value::Object(values) => {
            let mut out = serde_json::Map::new();
            for (key, value) in values {
                if matches!(
                    key.as_str(),
                    "stage"
                        | "context"
                        | "requestId"
                        | "operationId"
                        | "queueWaitMs"
                        | "events"
                        | "droppedEvents"
                        | "phase"
                        | "operation"
                        | "selector"
                        | "sessionEpoch"
                        | "startedMs"
                        | "elapsedMs"
                        | "remainingBudgetMs"
                        | "remainingAfterMs"
                        | "outcome"
                        | "bounds"
                        | "plannerPoint"
                        | "enabled"
                        | "clickable"
                        | "lastCompletedSnapshot"
                        | "lastVerifiedSnapshot"
                        | "reasonCode"
                        | "expectedCount"
                        | "lastVerifiedCount"
                        | "observedOrdinals"
                        | "nextCount"
                        | "albumMatches"
                        | "foregroundPackage"
                        | "scrollCount"
                        | "viewport"
                        | "snapshotGeneration"
                        | "snapshotSha256"
                        | "selectorBounds"
                        | "nextBounds"
                        | "artifacts"
                        | "artifactWriteFailed"
                        | "x"
                        | "y"
                        | "width"
                        | "height"
                        | "role"
                        | "generation"
                        | "sha256"
                        | "xmlPath"
                        | "screenshotPath"
                        | "xml_path"
                        | "screenshot_path"
                        | "ok"
                        | "udid"
                        | "campaignId"
                        | "fileCount"
                        | "manifestSha256"
                        | "manifestBytes"
                        | "readback"
                        | "hiddenFromMediaStore"
                        | "reusedImport"
                        | "value"
                        | "state"
                        | "importId"
                        | "files"
                        | "error"
                        | "code"
                        | "message"
                        | "chain"
                ) {
                    // Only the producer's event stream has the larger 1024-item budget.
                    if key == "events" {
                        if let Some(events) = value.as_array() {
                            let projected = events
                                .iter()
                                .take(1024)
                                .map(|event| {
                                    bounded_trace_value(event, depth - 1).unwrap_or_else(|| {
                                        trace_evidence_omitted("unsupportedEvent")
                                    })
                                })
                                .collect::<Vec<_>>();
                            out.insert(key.clone(), Value::Array(projected));
                            if events.len() > 1024 {
                                out.insert("eventsTruncated".into(), Value::Bool(true));
                                out.insert(
                                    "omittedEvents".into(),
                                    Value::from(events.len() - 1024),
                                );
                            }
                            continue;
                        }
                    }
                    if let Some(value) = bounded_trace_value(value, depth - 1) {
                        out.insert(key.clone(), value);
                    }
                }
            }
            (!out.is_empty()).then_some(Value::Object(out))
        }
        _ => Some(value.clone()),
    }
}

use std::collections::BTreeMap;

use riviu_core::{
    nurture_source_id, project_flow_detail, project_flow_summary, project_interaction_detail,
    project_interaction_summary, project_job, project_nurture, project_orchestration_detail,
    project_orchestration_summary, project_publish_detail_with_target, project_publish_summary,
    query_operation_summaries, NurtureSessionStatus, OperationRunDetail, OperationRunPage,
    OperationRunQuery, OperationRunState, OperationRunSummary,
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
pub async fn list_jobs(state: State<'_, AppState>) -> Result<Vec<JobRecord>, CommandError> {
    state
        .db
        .storage_read(|db| db.list_jobs(100))
        .await
        .map_err(err)
}

/// Read every persisted/runtime work source through one normalized operations contract.
/// The source rows remain authoritative; this command creates no history of its own.
#[tauri::command]
pub async fn operation_list_runs(
    state: State<'_, AppState>,
    limit: Option<usize>,
) -> Result<Vec<OperationRunSummary>, CommandError> {
    let limit = limit.unwrap_or(100).clamp(1, 200);
    let live = state.nurture.list_status();
    state
        .db
        .storage_read(move |db| Ok(list_operation_runs(db, live, limit)))
        .await
        .map_err(err)?
}

fn list_operation_runs(
    db: &riviu_core::db::Database,
    live: Vec<NurtureSessionStatus>,
    limit: usize,
) -> Result<Vec<OperationRunSummary>, CommandError> {
    let mut runs = Vec::new();

    runs.extend(
        db.list_jobs(limit)
            .map_err(err)?
            .iter()
            .map(|job| project_job(job).summary),
    );
    for run in db.list_flow_runs(limit).map_err(err)? {
        let title = db
            .get_flow_revision(run.flow_id, Some(run.flow_revision))
            .map_err(err)?
            .map(|revision| revision.document.name)
            .unwrap_or_else(|| "Flow thiết bị".to_string());
        let summary = db
            .get_flow_run(run.id)
            .map_err(err)?
            .map(|detail| project_flow_detail(&detail, title.clone()).summary)
            .unwrap_or_else(|| project_flow_summary(&run, title));
        runs.push(summary);
    }
    for run in db.list_orchestration_runs(limit).map_err(err)? {
        let title = db
            .get_orchestration_revision(run.document_id, Some(run.document_revision))
            .map_err(err)?
            .map(|revision| revision.compiled.document.name)
            .unwrap_or_else(|| "Điều phối".to_string());
        let summary = db
            .get_orchestration_run(run.id)
            .map_err(err)?
            .map(|detail| project_orchestration_detail(&detail, title.clone()).summary)
            .unwrap_or_else(|| project_orchestration_summary(&run, title));
        runs.push(summary);
    }

    let nurture_runs =
        merge_nurture_history(db.list_nurture_runs(limit).map_err(err)?, live.clone());
    runs.extend(
        nurture_runs
            .iter()
            .map(|(source_id, sessions)| project_nurture(source_id, sessions).summary),
    );

    for campaign in db.list_interaction_campaigns(limit).map_err(err)? {
        let summary = db
            .get_interaction_campaign(&campaign.id)
            .map_err(err)?
            .map(|detail| project_interaction_detail(&detail).summary)
            .unwrap_or_else(|| project_interaction_summary(&campaign));
        runs.push(summary);
    }
    for campaign in db.list_publish_campaigns(limit).map_err(err)? {
        let Some(detail) = db.get_publish_campaign(&campaign.id).map_err(err)? else {
            continue;
        };
        let snapshot = db
            .get_publish_execution_snapshot(&campaign.id)
            .map_err(err)?;
        runs.push(project_publish_summary(&detail, snapshot.as_ref()));
    }

    for kind in [
        riviu_core::OperationRunKind::AppInstall,
        riviu_core::OperationRunKind::MaterialTransfer,
    ] {
        for id in db.operation_source_ids(None, Some(kind)).map_err(err)? {
            if let Some(detail) = read_operation_run_from(db, &live, &id)? {
                runs.push(detail.summary);
            }
        }
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
pub async fn operation_query_runs(
    state: State<'_, AppState>,
    query: OperationRunQuery,
) -> Result<OperationRunPage, CommandError> {
    if let Some(since) = &query.since {
        chrono::DateTime::parse_from_rfc3339(since)
            .map_err(|_| CommandError::invalid_argument("since must be RFC3339"))?;
    }
    let live = state.nurture.list_status();
    state
        .db
        .storage_read(move |db| Ok(query_operation_runs(db, live, &query)))
        .await
        .map_err(err)?
}

fn query_operation_runs(
    db: &riviu_core::db::Database,
    live: Vec<NurtureSessionStatus>,
    query: &OperationRunQuery,
) -> Result<OperationRunPage, CommandError> {
    let mut runs = BTreeMap::new();
    let mut publications = Vec::new();
    for id in db
        .operation_source_ids(query.since.as_deref(), query.kind)
        .map_err(err)?
    {
        if let Some(source_id) = id.strip_prefix("publish:") {
            publications.push(source_id.to_owned());
            continue;
        }
        if let Some(detail) = read_operation_run_from(db, &live, &id)? {
            runs.insert(id, detail.summary);
        }
    }
    for summary in db.publish_operation_summaries(&publications).map_err(err)? {
        runs.insert(summary.id.clone(), summary);
    }
    for (id, sessions) in merge_nurture_history(Vec::new(), live.clone()) {
        let summary = project_nurture(&id, &sessions).summary;
        let detail = read_operation_run_from(db, &live, &summary.id)?;
        runs.insert(
            summary.id.clone(),
            detail.map(|detail| detail.summary).unwrap_or(summary),
        );
    }
    Ok(query_operation_summaries(
        runs.into_values().collect(),
        query,
    ))
}

#[tauri::command]
pub async fn operation_get_run(
    state: State<'_, AppState>,
    operation_id: String,
) -> Result<Option<OperationRunDetail>, CommandError> {
    let live = state.nurture.list_status();
    state
        .db
        .storage_read(move |db| Ok(read_operation_run_from(db, &live, &operation_id)))
        .await
        .map_err(err)?
}

#[tauri::command]
pub async fn operation_device_log(
    state: State<'_, AppState>,
    operation_id: String,
    udid: String,
) -> Result<riviu_core::OperationDeviceLog, CommandError> {
    let live = state.nurture.list_status();
    state
        .db
        .storage_read(move |db| Ok(read_operation_device_log(db, &live, &operation_id, &udid)))
        .await
        .map_err(err)?
}

fn read_operation_device_log(
    db: &riviu_core::db::Database,
    live: &[NurtureSessionStatus],
    operation_id: &str,
    udid: &str,
) -> Result<riviu_core::OperationDeviceLog, CommandError> {
    let detail = read_operation_run_from(db, live, operation_id)?
        .ok_or_else(|| CommandError::invalid_argument("operation no longer exists"))?;
    let belongs = if detail.summary.kind == riviu_core::OperationRunKind::Script {
        let id = uuid::Uuid::parse_str(&detail.summary.source_id).map_err(err)?;
        db.get_job(id)
            .map_err(err)?
            .is_some_and(|job| job.udids.iter().any(|id| id == udid))
    } else {
        detail
            .items
            .iter()
            .any(|item| item.udid.as_deref() == Some(udid))
    };
    if !belongs {
        return Err(CommandError::invalid_argument(
            "device does not belong to this operation",
        ));
    }
    if detail.summary.kind == riviu_core::OperationRunKind::Orchestration {
        let source = db
            .get_orchestration_run(uuid::Uuid::parse_str(&detail.summary.source_id).map_err(err)?)
            .map_err(err)?
            .ok_or_else(|| CommandError::invalid_argument("orchestration source missing"))?;
        return orchestration_device_log(db, &source, udid);
    }
    db.operation_device_log(detail.summary.kind, &detail.summary.source_id, udid)
        .map_err(err)
}

fn orchestration_device_log(
    db: &riviu_core::db::Database,
    source: &riviu_core::OrchestrationRunDetail,
    udid: &str,
) -> Result<riviu_core::OperationDeviceLog, CommandError> {
    let mut entries = Vec::new();
    let mut truncated = false;
    for attempt in source
        .attempts
        .iter()
        .filter(|a| a.snapshot.target.included.iter().any(|d| d.udid == udid))
    {
        let mut child = match orchestration_child_source(db, attempt)? {
            Some((kind, id)) => db.operation_device_log(kind, &id, udid).map_err(err)?,
            None => riviu_core::OperationDeviceLog {
                entries: vec![],
                truncated: false,
            },
        };
        truncated |= child.truncated;
        for row in &mut child.entries {
            row.id = format!("{}:{}", attempt.snapshot.attempt_id, row.id);
        }
        entries.extend(child.entries);
        entries.push(riviu_core::OperationDeviceLogEntry {
                id:format!("{}:state",attempt.snapshot.attempt_id),at:Some(attempt.updated_at.clone()),
                action:"orchestration".into(),state:serde_json::to_value(attempt.state).map_err(err)?.as_str().unwrap_or("uncertain").into(),
                text:attempt.error_code.clone(),detail:Some(serde_json::json!({"attemptId":attempt.snapshot.attempt_id,"nodeId":attempt.snapshot.node_id,"childKind":attempt.child_kind,"childId":attempt.child_campaign_id}).to_string()),
            });
    }
    entries.sort_by(|a, b| a.at.cmp(&b.at).then(a.id.cmp(&b.id)));
    if entries.len() > 500 {
        truncated = true;
        entries.drain(..entries.len() - 500);
    }
    Ok(riviu_core::OperationDeviceLog { entries, truncated })
}

fn orchestration_child_source(
    db: &riviu_core::db::Database,
    attempt: &riviu_core::OrchestrationAttemptRecord,
) -> Result<Option<(riviu_core::OperationRunKind, String)>, CommandError> {
    let Some(id) = attempt.child_campaign_id else {
        return Ok(None);
    };
    Ok(match attempt.child_kind {
        Some(riviu_core::AutomationKind::Interaction) => {
            Some((riviu_core::OperationRunKind::Interaction, id.to_string()))
        }
        Some(riviu_core::AutomationKind::Publish) => {
            Some((riviu_core::OperationRunKind::Publish, id.to_string()))
        }
        Some(riviu_core::AutomationKind::Nurture) => db
            .get_orchestration_nurture_child(id)
            .map_err(err)?
            .and_then(|r| r.run_id)
            .map(|id| (riviu_core::OperationRunKind::Nurture, id.to_string())),
        None => None,
    })
}

fn project_orchestration_devices(
    source: &riviu_core::OrchestrationRunDetail,
    title: String,
) -> OperationRunDetail {
    let mut projected = project_orchestration_detail(source, title);
    let attempts = projected.items.clone();
    projected.items.clear();
    let devices = std::iter::once(&source.run.target)
        .chain(source.run.node_targets.values())
        .chain(source.attempts.iter().map(|a| &a.snapshot.target))
        .flat_map(|t| t.included.iter().map(|d| d.udid.clone()))
        .collect::<std::collections::BTreeSet<_>>();
    for udid in devices {
        let mut matched = false;
        for (attempt, row) in source.attempts.iter().zip(&attempts) {
            if attempt
                .snapshot
                .target
                .included
                .iter()
                .any(|d| d.udid == udid)
            {
                let mut row = row.clone();
                row.id = format!("{}:{udid}", row.id);
                row.udid = Some(udid.clone());
                projected.items.push(row);
                matched = true;
            }
        }
        if !matched {
            projected.items.push(riviu_core::OperationRunItem {
                id: format!("device:{udid}"),
                kind: riviu_core::OperationRunItemKind::Device,
                label: "Máy trong snapshot".into(),
                state: if projected.summary.state.is_terminal() {
                    OperationRunState::Skipped
                } else {
                    OperationRunState::Queued
                },
                udid: Some(udid),
                error_code: None,
                detail: None,
                evidence: None,
                retryable: false,
            });
        }
    }
    projected
}

pub(crate) fn read_operation_run(
    state: &AppState,
    operation_id: &str,
) -> Result<Option<OperationRunDetail>, CommandError> {
    read_operation_run_from(&state.db, &state.nurture.list_status(), operation_id)
}

fn read_operation_run_from(
    db: &riviu_core::db::Database,
    live: &[NurtureSessionStatus],
    operation_id: &str,
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
        "appInstall" | "materialTransfer" => Ok(db
            .get_library_batch(source_id)
            .map_err(err)?
            .filter(|detail| detail.summary.kind.as_key() == kind)),
        "script" => {
            let id = uuid::Uuid::parse_str(source_id).map_err(|_| {
                CommandError::invalid_argument("script operation ID must be a UUID")
            })?;
            let Some(job) = db.get_job(id).map_err(err)? else {
                return Ok(None);
            };
            let mut detail = project_job(&job);
            for udid in &job.udids {
                let log = db
                    .operation_device_log(riviu_core::OperationRunKind::Script, source_id, udid)
                    .map_err(err)?;
                let mut steps = std::collections::BTreeMap::new();
                for entry in &log.entries {
                    if let Some(index) = entry
                        .detail
                        .as_deref()
                        .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok())
                        .and_then(|value| value["stepIndex"].as_u64())
                    {
                        steps.insert(index, entry);
                    }
                }
                let state = if steps.values().any(|row| row.state == "uncertain") {
                    OperationRunState::Uncertain
                } else if steps.values().any(|row| row.state == "failed") {
                    OperationRunState::Failed
                } else if steps.values().any(|row| row.state == "running") {
                    if detail.summary.state.is_terminal() {
                        OperationRunState::Uncertain
                    } else {
                        OperationRunState::Running
                    }
                } else if !steps.is_empty()
                    && steps.len() == job.steps.len()
                    && steps.values().all(|row| row.state == "succeeded")
                {
                    OperationRunState::Succeeded
                } else if detail.summary.state == OperationRunState::Cancelled {
                    OperationRunState::Skipped
                } else if steps.is_empty() && !detail.summary.state.is_terminal() {
                    OperationRunState::Queued
                } else if !detail.summary.state.is_terminal() {
                    OperationRunState::Running
                } else {
                    OperationRunState::Uncertain
                };
                detail.items.push(riviu_core::OperationRunItem {
                    id: format!("device:{udid}"),
                    kind: riviu_core::OperationRunItemKind::Device,
                    label: "Máy trong snapshot".into(),
                    state,
                    udid: Some(udid.clone()),
                    error_code: None,
                    detail: None,
                    evidence: None,
                    retryable: false,
                });
            }
            Ok(Some(detail))
        }
        "flow" => {
            let id = uuid::Uuid::parse_str(source_id)
                .map_err(|_| CommandError::invalid_argument("Flow operation ID must be a UUID"))?;
            let Some(detail) = db.get_flow_run(id).map_err(err)? else {
                return Ok(None);
            };
            let title = db
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
            let Some(detail) = db.get_orchestration_run(id).map_err(err)? else {
                return Ok(None);
            };
            let title = db
                .get_orchestration_revision(
                    detail.run.document_id,
                    Some(detail.run.document_revision),
                )
                .map_err(err)?
                .map(|revision| revision.compiled.document.name)
                .unwrap_or_else(|| "Điều phối".to_string());
            Ok(Some(project_orchestration_devices(&detail, title)))
        }
        "nurture" => {
            let live = live
                .iter()
                .filter(|status| nurture_source_id(status) == source_id)
                .cloned()
                .collect::<Vec<_>>();
            let persisted = uuid::Uuid::parse_str(source_id)
                .ok()
                .map(|run_id| db.get_nurture_run(run_id))
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
        "interaction" => Ok(db
            .get_interaction_campaign(source_id)
            .map_err(err)?
            .as_ref()
            .map(project_interaction_detail)),
        "publish" => {
            let Some(detail) = db.get_publish_campaign(source_id).map_err(err)? else {
                return Ok(None);
            };
            let snapshot = db.get_publish_execution_snapshot(source_id).map_err(err)?;
            let request = db.publish_campaign_request(source_id).map_err(err)?;
            let mut projected = project_publish_detail_with_target(
                &detail,
                snapshot.as_ref(),
                request
                    .as_ref()
                    .and_then(|request| request.target_snapshot.as_ref()),
            );
            if db.publish_operation_stopped(source_id).map_err(err)? {
                projected.summary.state = OperationRunState::Cancelled;
            }
            Ok(Some(projected))
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


    #[test]
    fn persisted_diagnostics_projection_preserves_bounded_evidence_without_raw_envelopes() {
        use serde_json::json;
        let snapshot = json!({"generation": 7, "sha256": "snapshot", "sessionEpoch": "epoch",
            "albumMatches": true, "observedOrdinals": [1, 2], "nextCount": 2,
            "selectorBounds": [{"x": 1, "y": 2, "width": 3, "height": 4}],
            "nextBounds": [], "foregroundPackage": "com.instagram.barcelona", "album": "SECRET"});
        let event = json!({"phase": "picker", "operation": "read", "sessionEpoch": "epoch",
            "startedMs": 10, "elapsedMs": 20, "remainingBudgetMs": 30, "remainingAfterMs": 10,
            "outcome": "completed", "selector": "text", "bounds": [1,2,3,4],
            "plannerPoint": [2,3], "enabled": true, "clickable": false,
            "snapshotGeneration": 7, "snapshotSha256": "snapshot", "caption": "SECRET"});
        let raw = json!({"stageDiagnostics": {"context": {"requestId": "request",
            "operationId": "operation", "udid": "phone", "queueWaitMs": null, "account": "SECRET"},
            "events": [event], "droppedEvents": 2, "helper": {"message": "SECRET"}},
            "selectionDiagnostic": {"stage": "tapReadback", "snapshotGeneration": null,
                "lastCompletedSnapshot": snapshot, "lastVerifiedSnapshot": snapshot},
            "caption": "SECRET", "account": "SECRET", "album": "SECRET"});
        let projected = persisted_trace_evidence(&raw.to_string()).unwrap();
        let stage = &projected["stageDiagnostics"];
        assert_eq!(stage["context"]["requestId"], "request");
        assert_eq!(stage["context"]["operationId"], "operation");
        assert_eq!(stage["context"]["udid"], "phone");
        assert!(stage["context"]["queueWaitMs"].is_null());
        assert_eq!(stage["droppedEvents"], 2);
        let mut safe_event = event.clone();
        safe_event.as_object_mut().unwrap().remove("caption");
        assert_eq!(stage["events"][0], safe_event);
        let mut safe_snapshot = snapshot.clone();
        safe_snapshot.as_object_mut().unwrap().remove("album");
        for key in ["lastCompletedSnapshot", "lastVerifiedSnapshot"] {
            assert_eq!(projected["selectionDiagnostic"][key], safe_snapshot);
        }
        assert!(projected["selectionDiagnostic"]["snapshotGeneration"].is_null());
        assert!(!projected.to_string().contains("SECRET"));
        for prior in [raw.clone(), json!(raw.to_string())] {
            let wrapped =
                persisted_trace_evidence(&json!({"priorEvidenceJson": prior}).to_string()).unwrap();
            assert_eq!(wrapped["priorEvidenceJson"], projected);
        }
        // Compact events exercise the count cap independently of the raw byte cap.
        let mut many = json!({"stageDiagnostics": {"events": vec![json!({"elapsedMs": 1}); 1024]}});
        let full = persisted_trace_evidence(&many.to_string()).unwrap();
        assert_eq!(
            full["stageDiagnostics"]["events"].as_array().unwrap().len(),
            1024
        );
        many["stageDiagnostics"]["events"]
            .as_array_mut()
            .unwrap()
            .push(json!({"elapsedMs": 2}));
        let limited = persisted_trace_evidence(&many.to_string()).unwrap();
        assert_eq!(
            limited["stageDiagnostics"]["events"]
                .as_array()
                .unwrap()
                .len(),
            1024
        );
        assert_eq!(limited["stageDiagnostics"]["omittedEvents"], 1);
        assert_eq!(limited["stageDiagnostics"]["eventsTruncated"], true);
        let oversized = json!({"stageDiagnostics": {"events": vec![event; 1024]}}).to_string();
        assert!(oversized.len() > 256 * 1024);
        let omitted = persisted_trace_evidence(&oversized).expect("explicit omission");
        assert_eq!(omitted["reasonCode"], "rawByteLimit");
        assert_eq!(omitted["omitted"], true);
        assert_eq!(omitted["truncated"], true);
        let mut deep = raw;
        for _ in 0..8 {
            deep = json!({"priorEvidenceJson": deep});
        }
        let deep = persisted_trace_evidence(&deep.to_string()).unwrap();
        let mut leaf = &deep;
        for _ in 0..8 {
            leaf = &leaf["priorEvidenceJson"];
        }
        assert_eq!(leaf["reasonCode"], "depthLimit");
    }

    #[test]
    fn script_export_uses_its_device_artifact_even_after_another_device_overwrites_the_global_step()
    {
        let root = std::env::temp_dir().join(format!("script-export-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let db = riviu_core::db::Database::open(root.join("fixture.db")).unwrap();
        let id = uuid::Uuid::new_v4();
        let a = root.join(id.to_string()).join("a").join("step.png");
        let b = root.join(id.to_string()).join("b").join("step.png");
        for (p, bytes) in [(&a, b"a"), (&b, b"b")] {
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, bytes).unwrap();
        }
        let mut job = JobRecord {
            id,
            script_name: "fixture".into(),
            udids: vec!["a".into(), "b".into()],
            status: riviu_core::JobStatus::Succeeded,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            steps: vec![riviu_core::JobStepRecord {
                index: 0,
                action: "screenshot".into(),
                status: riviu_core::StepStatus::Succeeded,
                error: None,
                artifact_path: Some(a.to_string_lossy().into_owned()),
            }],
            error: None,
        };
        db.save_job_device_step(&job, "a", 0, "session-a", 10)
            .unwrap();
        job.steps[0].artifact_path = Some(b.to_string_lossy().into_owned());
        db.save_job_device_step(&job, "b", 0, "session-b", 10)
            .unwrap();
        let changed_scope =
            export_operation_trace(&db, &[], &root, &format!("script:{id}"), "a", &[]).unwrap_err();
        assert!(changed_scope.message.contains("Trace scope changed"));
        let exported = export_operation_trace(
            &db,
            &[],
            &root,
            &format!("script:{id}"),
            "a",
            &[id.to_string()],
        )
        .unwrap();
        let trace: riviu_core::ipc_contract::OperationTrace =
            serde_json::from_slice(&std::fs::read(exported.path).unwrap()).unwrap();
        assert_eq!(trace.artifacts.len(), 1);
        assert_eq!(
            PathBuf::from(&trace.artifacts[0].path),
            a.canonicalize().unwrap()
        );
        assert_eq!(trace.steps.len(), 1);
    }

    #[test]
    fn orchestration_timeline_uses_exact_child_and_device_scope_without_borrowing_sibling_events() {
        use serde_json::json;
        let path =
            std::env::temp_dir().join(format!("orchestration-trace-{}.db", uuid::Uuid::new_v4()));
        let db = riviu_core::db::Database::open(&path).unwrap();
        let run = uuid::Uuid::new_v4();
        let doc = uuid::Uuid::new_v4();
        let child = uuid::Uuid::new_v4();
        let attempt = uuid::Uuid::new_v4();
        let node = uuid::Uuid::new_v4();
        let target = |ids: &[&str]| json!({"targetRef":{"type":"all"},"included":ids.iter().map(|id|json!({"udid":id,"alias":"","number":null})).collect::<Vec<_>>(),"excluded":[],"rosterSha256":"a".repeat(64)});
        let source:riviu_core::OrchestrationRunDetail=serde_json::from_value(json!({
            "run":{"id":run,"documentId":doc,"documentRevision":1,"documentSha256":"a".repeat(64),"target":target(&["phone-a","phone-b"]),"nodeTargets":{},"state":"running","currentNodeId":node,"errorCode":null,"createdAt":"2026-09-19T00:00:00Z","updatedAt":"2026-09-19T00:00:01Z"},
            "attempts":[{"snapshot":{"documentId":doc,"documentRevision":1,"documentSha256":"a".repeat(64),"canonicalDocumentJson":"{}","nodeId":node,"attemptId":attempt,"idempotencyKey":"b".repeat(64),"profile":null,"target":target(&["phone-a"])},"runId":run,"attemptNo":1,"state":"waitingChild","childKind":"interaction","childCampaignId":child,"branch":null,"errorCode":null,"createdAt":"2026-09-19T00:00:00Z","updatedAt":"2026-09-19T00:00:01Z"}]
        })).unwrap();
        let conn = rusqlite::Connection::open(&path).unwrap();
        for (campaign, device, text) in [
            (child.to_string(), "phone-a", "owned"),
            (child.to_string(), "phone-b", "sibling"),
            (uuid::Uuid::new_v4().to_string(), "phone-a", "other run"),
        ] {
            conn.execute("INSERT INTO operation_device_events(source_kind,source_id,udid,action,state,recorded_at,text) VALUES('interaction',?1,?2,'follow','confirmed','2026-09-19T00:00:00Z',?3)",rusqlite::params![campaign,device,text]).unwrap();
        }
        let log = orchestration_device_log(&db, &source, "phone-a").unwrap();
        assert_eq!(log.entries.len(), 2);
        assert_eq!(log.entries[0].text.as_deref(), Some("owned"));
        assert!(log
            .entries
            .iter()
            .all(|e| e.id.starts_with(&attempt.to_string())));
        assert!(orchestration_device_log(&db, &source, "phone-b")
            .unwrap()
            .entries
            .is_empty());
        let projected = project_orchestration_devices(&source, "fixture".into());
        assert_eq!(projected.items.len(), 2);
        assert!(
            projected
                .items
                .iter()
                .any(|r| r.udid.as_deref() == Some("phone-a")
                    && r.state == OperationRunState::Running)
        );
        assert!(projected
            .items
            .iter()
            .any(|r| r.udid.as_deref() == Some("phone-b") && r.state == OperationRunState::Queued));
    }

    #[tokio::test]
    async fn admitted_reads_keep_uncertainty_and_reject_a_foreign_device_log() {
        let path =
            std::env::temp_dir().join(format!("riviu-admitted-reads-{}.db", uuid::Uuid::new_v4()));
        let db = Arc::new(riviu_core::db::Database::open(&path).unwrap());
        let id = uuid::Uuid::new_v4();
        let job = JobRecord {
            id,
            script_name: "interrupted script".into(),
            udids: vec!["phone-a".into()],
            status: riviu_core::JobStatus::Uncertain,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            steps: vec![riviu_core::JobStepRecord {
                index: 0,
                action: "tap".into(),
                status: riviu_core::StepStatus::Uncertain,
                error: Some("worker lost after intent".into()),
                artifact_path: None,
            }],
            error: None,
        };
        db.storage_write(move |db| db.save_job(&job)).await.unwrap();
        let (page, detail, refused) = db
            .storage_read(move |db| {
                let operation_id = format!("script:{id}");
                Ok((
                    query_operation_runs(db, Vec::new(), &OperationRunQuery::default()),
                    read_operation_run_from(db, &[], &operation_id),
                    read_operation_device_log(db, &[], &operation_id, "phone-b"),
                ))
            })
            .await
            .unwrap();
        let page = page.unwrap();
        let detail = detail.unwrap().unwrap();
        assert_eq!(page.total, 1);
        assert_eq!(page.runs[0], detail.summary);
        assert_eq!(detail.summary.state, OperationRunState::Uncertain);
        assert_eq!(detail.summary.retryable_count, 0);
        assert!(detail
            .items
            .iter()
            .any(|row| row.udid.as_deref() == Some("phone-a")
                && row.state == OperationRunState::Uncertain));
        assert_eq!(refused.unwrap_err().code, "InvalidArgument");
        drop(db);
        let _ = std::fs::remove_file(path);
    }

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
