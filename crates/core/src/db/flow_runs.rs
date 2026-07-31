use std::collections::BTreeSet;

use anyhow::{bail, ensure, Context};
use base64::Engine as _;
use chrono::{DateTime, SecondsFormat, Utc};
use rusqlite::{params, Connection, OptionalExtension, Transaction, TransactionBehavior};
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::{json, Value};
use uuid::Uuid;

use super::Database;
use crate::flow::artifact_store::validate_artifact_record_relative_path;
use crate::{
    canonical_compiled_plan_json, compiled_plan_sha256, contracts, validate_artifact_label,
    CompiledActionConfig, CompiledFlowNode, CompiledFlowPlanV2, DeviceWorkOwner, EvidenceKind,
    EvidenceRequirement, EvidenceSpec, FlowAggregateState, FlowArtifactRecord, FlowAttemptState,
    FlowCapabilitySnapshot, FlowContextReleaseProof, FlowDeviceRunRecord, FlowDeviceRunState,
    FlowErrorRecord, FlowNodeAttemptRecord, FlowRevisionRecord, FlowRunDetail, FlowRunRecord,
    FlowSelectionSnapshot, FlowTargetSelection, RetryPolicy, SideEffectClass, FLOW_SCHEMA_VERSION,
};

#[derive(Debug, Clone, Default)]
pub struct AttemptTransitionPatch {
    pub canonical_input: Option<Value>,
    pub evidence_baseline: Option<Value>,
    pub evidence_result: Option<Value>,
    pub error: Option<FlowErrorRecord>,
}

#[derive(Debug, Clone, thiserror::Error)]
#[error(
    "StateConflict: entity {entity_id}, expected {expected}, actual {actual}, requested {requested}"
)]
pub struct FlowStateConflict {
    pub entity_id: Uuid,
    pub expected: String,
    pub actual: String,
    pub requested: String,
}

#[derive(Debug, Clone)]
pub(crate) struct FlowAttemptExecutionContext {
    pub(crate) run: FlowRunRecord,
    pub(crate) device: FlowDeviceRunRecord,
    pub(crate) attempt: FlowNodeAttemptRecord,
    pub(crate) plan: CompiledFlowPlanV2,
    pub(crate) device_attempts: Vec<FlowNodeAttemptRecord>,
}

#[derive(Debug, Clone)]
pub(crate) struct FlowRecoveryRunContext {
    pub(crate) run: FlowRunRecord,
    pub(crate) plan: CompiledFlowPlanV2,
    pub(crate) devices: Vec<FlowDeviceRunRecord>,
    pub(crate) attempts: Vec<FlowNodeAttemptRecord>,
    pub(crate) artifacts: Vec<FlowArtifactRecord>,
}

fn insert_flow_run(
    transaction: &Transaction<'_>,
    revision: &FlowRevisionRecord,
    selection: &FlowSelectionSnapshot,
) -> anyhow::Result<FlowRunRecord> {
    let flow_id = revision.document.id;
    let flow_revision = revision.document.revision;
    let stored: Option<(String, String)> = transaction
        .query_row(
            "SELECT plan_sha256,compiled_json FROM flow_revisions
             WHERE flow_id=?1 AND revision=?2",
            params![
                flow_id.to_string(),
                u64_to_sql(flow_revision, "Flow run revision")?
            ],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    let (stored_hash, stored_compiled_json) =
        stored.context("Flow run revision is not persisted")?;
    ensure!(
        stored_hash == revision.plan_hash,
        "Flow run revision is not the exact persisted immutable revision"
    );
    let stored_plan: CompiledFlowPlanV2 =
        serde_json::from_str(&stored_compiled_json).context("parse persisted Flow run plan")?;
    validate_persisted_plan(
        &stored_plan,
        &stored_compiled_json,
        &flow_id.to_string(),
        u64_to_sql(flow_revision, "Flow run revision")?,
        &stored_hash,
        &stored_hash,
    )?;
    ensure!(
        stored_plan == revision.compiled_plan,
        "Flow run compiled plan differs from its persisted revision"
    );

    let id = Uuid::new_v4();
    let now = now_text();
    let selection_json = serde_json::to_string(selection).context("serialize Flow selection")?;
    transaction.execute(
        "INSERT INTO flow_runs(
            id,flow_id,flow_revision,plan_sha256,selection_json,state,event_revision,
            error_json,created_at,updated_at
         ) VALUES(?1,?2,?3,?4,?5,'queued',0,NULL,?6,?6)",
        params![
            id.to_string(),
            flow_id.to_string(),
            u64_to_sql(flow_revision, "Flow run revision")?,
            revision.plan_hash,
            selection_json,
            now
        ],
    )?;
    append_flow_event(
        transaction,
        id,
        "runCreated",
        &json!({
            "flowId": flow_id,
            "flowRevision": flow_revision,
            "planSha256": revision.plan_hash,
        }),
        &now,
    )?;
    query_run_record(transaction, id)?.context("created Flow run disappeared")
}

fn insert_flow_device_run(
    transaction: &Transaction<'_>,
    run: &FlowRunRecord,
    udid: &str,
) -> anyhow::Result<FlowDeviceRunRecord> {
    ensure!(
        run.selection
            .target_udids
            .iter()
            .any(|target| target == udid),
        "device is outside the frozen Flow selection"
    );
    ensure!(
        !run.state.is_terminal(),
        "StateConflict: cannot add a device to a terminal Flow run"
    );
    let id = Uuid::new_v4();
    transaction.execute(
        "INSERT INTO flow_device_runs(
            id,run_id,udid,state,capability_snapshot_json,release_proof_json,error_json,
            started_at,finished_at
         ) VALUES(?1,?2,?3,'queued',NULL,NULL,NULL,NULL,NULL)",
        params![id.to_string(), run.id.to_string(), udid],
    )?;
    let now = now_text();
    append_flow_event(
        transaction,
        run.id,
        "deviceRunCreated",
        &json!({"deviceRunId": id, "udid": udid}),
        &now,
    )?;
    query_device_run_record(transaction, id)?.context("created Flow device run disappeared")
}

fn query_flow_run_detail(
    connection: &Connection,
    run_id: Uuid,
) -> anyhow::Result<Option<FlowRunDetail>> {
    let Some(run) = query_run_record(connection, run_id)? else {
        return Ok(None);
    };
    validate_run_plan(connection, &run)?;
    validate_event_ledger(connection, &run)?;

    let mut device_statement = connection.prepare(
        "SELECT id,run_id,udid,state,capability_snapshot_json,release_proof_json,
                error_json,started_at,finished_at
         FROM flow_device_runs WHERE run_id=?1 ORDER BY udid ASC,id ASC",
    )?;
    let device_rows = device_statement.query_map([run_id.to_string()], device_run_row)?;
    let mut device_runs = Vec::new();
    for row in device_rows {
        device_runs.push(row?.into_record()?);
    }
    drop(device_statement);

    let mut attempt_statement = connection.prepare(
        "SELECT a.id,a.device_run_id,a.node_id,a.action_kind,a.attempt_no,
                a.side_effect_class,a.state,a.canonical_input_json,
                a.evidence_baseline_json,a.evidence_result_json,a.retry_safe,a.error_json,
                a.started_at,a.updated_at,a.finished_at
         FROM flow_node_attempts a
         JOIN flow_device_runs d ON d.id=a.device_run_id
         WHERE d.run_id=?1
         ORDER BY a.device_run_id ASC,a.attempt_no ASC,a.node_id ASC,a.id ASC",
    )?;
    let attempt_rows = attempt_statement.query_map([run_id.to_string()], attempt_row)?;
    let mut attempts = Vec::new();
    for row in attempt_rows {
        let attempt = row?.into_record()?;
        validate_persisted_attempt(connection, &attempt)?;
        attempts.push(attempt);
    }
    drop(attempt_statement);
    validate_device_attempt_projection(&run, &device_runs, &attempts)?;
    for device in &device_runs {
        if device.state == FlowDeviceRunState::Succeeded {
            validate_device_success_projection(connection, device)?;
        }
    }

    let mut artifact_statement = connection.prepare(
        "SELECT a.id,a.attempt_id,a.relative_path,a.label,a.kind,a.size,a.sha256,a.created_at
         FROM flow_artifacts a
         JOIN flow_node_attempts n ON n.id=a.attempt_id
         JOIN flow_device_runs d ON d.id=n.device_run_id
         WHERE d.run_id=?1 ORDER BY a.created_at ASC,a.id ASC",
    )?;
    let artifact_rows = artifact_statement.query_map([run_id.to_string()], artifact_row)?;
    let mut artifacts = Vec::new();
    for row in artifact_rows {
        artifacts.push(row?.into_record()?);
    }
    drop(artifact_statement);
    validate_artifact_projection(run.id, &device_runs, &attempts, &artifacts)?;
    if run.state.is_terminal() {
        let projection = device_runs
            .iter()
            .map(|device| (device.state, device.error.clone()))
            .collect::<Vec<_>>();
        let (expected_state, expected_error) = aggregate_projection(&projection);
        ensure!(
            run.state == expected_state && run.error == expected_error,
            "persisted terminal Flow aggregate does not match its device projection"
        );
    }

    Ok(Some(FlowRunDetail {
        run,
        device_runs,
        attempts,
        artifacts,
    }))
}

impl Database {
    pub fn create_flow_run(
        &self,
        revision: &FlowRevisionRecord,
        selection: FlowSelectionSnapshot,
    ) -> anyhow::Result<FlowRunRecord> {
        validate_selection(&selection)?;
        validate_revision_for_run(revision)?;
        let mut connection = self.conn()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let record = insert_flow_run(&transaction, revision, &selection)?;
        transaction.commit()?;
        Ok(record)
    }

    pub fn create_flow_run_with_devices(
        &self,
        revision: &FlowRevisionRecord,
        selection: FlowSelectionSnapshot,
    ) -> anyhow::Result<(FlowRunRecord, Vec<FlowDeviceRunRecord>)> {
        validate_selection(&selection)?;
        validate_revision_for_run(revision)?;
        let mut connection = self.conn()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let initial_run = insert_flow_run(&transaction, revision, &selection)?;
        let mut devices = Vec::with_capacity(selection.target_udids.len());
        for udid in &selection.target_udids {
            devices.push(insert_flow_device_run(&transaction, &initial_run, udid)?);
        }
        let run = query_run_record(&transaction, initial_run.id)?
            .context("created aggregate Flow run disappeared")?;
        transaction.commit()?;
        Ok((run, devices))
    }

    pub fn create_flow_device_run(
        &self,
        run_id: Uuid,
        udid: &str,
    ) -> anyhow::Result<FlowDeviceRunRecord> {
        validate_udid(udid)?;
        let mut connection = self.conn()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let run = query_run_record(&transaction, run_id)?
            .with_context(|| format!("Flow run {run_id} does not exist"))?;
        let record = insert_flow_device_run(&transaction, &run, udid)?;
        transaction.commit()?;
        Ok(record)
    }

    pub fn transition_flow_device_run(
        &self,
        device_run_id: Uuid,
        expected: FlowDeviceRunState,
        next: FlowDeviceRunState,
        capability_snapshot: Option<FlowCapabilitySnapshot>,
    ) -> anyhow::Result<FlowDeviceRunRecord> {
        if let Some(snapshot) = capability_snapshot.as_ref() {
            snapshot.validate().map_err(anyhow::Error::msg)?;
        }
        let legal = matches!(
            (expected, next, capability_snapshot.is_some()),
            (
                FlowDeviceRunState::Queued,
                FlowDeviceRunState::Preflight,
                false
            ) | (
                FlowDeviceRunState::Preflight,
                FlowDeviceRunState::Running,
                true
            )
        );
        ensure!(legal, "StateConflict: invalid Flow device transition");
        let mut connection = self.conn()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let current = query_device_run_record(&transaction, device_run_id)?
            .context("Flow device run does not exist")?;
        let run = query_run_record(&transaction, current.run_id)?
            .context("Flow device parent run does not exist")?;
        ensure!(
            !run.state.is_terminal(),
            "StateConflict: terminal Flow run cannot start a device"
        );
        ensure!(
            current.state == expected,
            "StateConflict: device run expected {:?}, actual {:?}",
            expected,
            current.state
        );
        if let Some(agent_status) = capability_snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.agent_status.as_ref())
        {
            ensure!(
                agent_status.udid == current.udid,
                "Flow AgentStatus UDID does not match its device run"
            );
        }
        let snapshot_json = encode_optional_json(capability_snapshot.as_ref())?;
        let now = now_text();
        let changed = transaction.execute(
            "UPDATE flow_device_runs SET
                state=?2,
                capability_snapshot_json=?3,
                started_at=COALESCE(started_at,?4)
             WHERE id=?1 AND state=?5",
            params![
                device_run_id.to_string(),
                enum_name(next)?,
                snapshot_json,
                now,
                enum_name(expected)?,
            ],
        )?;
        ensure!(changed == 1, "StateConflict: Flow device run changed");
        append_flow_event(
            &transaction,
            current.run_id,
            "deviceRunTransitioned",
            &json!({
                "deviceRunId": device_run_id,
                "expected": expected,
                "next": next,
                "capabilitySnapshot": capability_snapshot,
            }),
            &now,
        )?;
        let record = query_device_run_record(&transaction, device_run_id)?
            .context("transitioned Flow device run disappeared")?;
        transaction.commit()?;
        Ok(record)
    }

    pub fn create_flow_attempt(
        &self,
        device_run_id: Uuid,
        node: &CompiledFlowNode,
        side_effect_class: SideEffectClass,
        attempt_no: u32,
    ) -> anyhow::Result<FlowNodeAttemptRecord> {
        ensure!(attempt_no >= 1, "Flow attempt number must be at least one");
        ensure!(
            contracts(node.kind).1 == side_effect_class,
            "Flow attempt side-effect class does not match the action contract"
        );
        let mut connection = self.conn()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let row = transaction.query_row(
            "SELECT d.run_id,r.compiled_json,d.state,f.state,
                    f.flow_id,f.flow_revision,f.plan_sha256,r.plan_sha256
             FROM flow_device_runs d
             JOIN flow_runs f ON f.id=d.run_id
             JOIN flow_revisions r ON r.flow_id=f.flow_id AND r.revision=f.flow_revision
             WHERE d.id=?1",
            [device_run_id.to_string()],
            |row| {
                Ok(AttemptCreationRow {
                    run_id: row.get(0)?,
                    compiled_plan_json: row.get(1)?,
                    device_state: row.get(2)?,
                    run_state: row.get(3)?,
                    flow_id: row.get(4)?,
                    flow_revision: row.get(5)?,
                    plan_sha256: row.get(6)?,
                    revision_plan_sha256: row.get(7)?,
                })
            },
        )?;
        let run_id = parse_uuid(&row.run_id, "Flow attempt run ID")?;
        let device_state: FlowDeviceRunState =
            parse_enum_name(&row.device_state, "Flow device state")?;
        let run_state: FlowAggregateState =
            parse_enum_name(&row.run_state, "Flow aggregate state")?;
        let current_device = query_device_run_record(&transaction, device_run_id)?
            .context("Flow attempt device disappeared")?;
        ensure!(
            current_device.state == device_state,
            "persisted Flow device state changed during attempt creation"
        );
        let plan: CompiledFlowPlanV2 = serde_json::from_str(&row.compiled_plan_json)
            .context("parse persisted Flow run plan")?;
        validate_persisted_plan(
            &plan,
            &row.compiled_plan_json,
            &row.flow_id,
            row.flow_revision,
            &row.plan_sha256,
            &row.revision_plan_sha256,
        )?;
        ensure!(
            plan.nodes.get(&node.id) == Some(node),
            "Flow attempt node is not the exact persisted compiled node"
        );

        let previous: Option<(String, i64)> = transaction
            .query_row(
                "SELECT id,attempt_no FROM flow_node_attempts
                 WHERE device_run_id=?1 AND node_id=?2
                 ORDER BY attempt_no DESC LIMIT 1",
                params![device_run_id.to_string(), node.id.to_string()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        let reopening = if let Some((previous_id, previous_no)) = previous {
            let expected_attempt_no = previous_no
                .checked_add(1)
                .context("persisted Flow attempt number overflow")?;
            ensure!(
                i64::from(attempt_no) == expected_attempt_no,
                "Flow retry attempt number is not contiguous"
            );
            let previous_id = parse_uuid(&previous_id, "previous Flow attempt ID")?;
            let previous = query_attempt_record(&transaction, previous_id)?
                .context("previous Flow attempt disappeared")?;
            ensure!(
                previous.state.is_terminal() && previous.retry_allowed,
                "StateConflict: previous Flow attempt is not retryable"
            );
            ensure!(
                device_state == FlowDeviceRunState::Failed,
                "StateConflict: retry requires a failed device projection"
            );
            ensure!(
                current_device.release_proof.is_some(),
                "StateConflict: retry requires persisted device release proof"
            );
            ensure!(
                current_device
                    .error
                    .as_ref()
                    .and_then(|error| error.attempt_id)
                    == Some(previous.id),
                "StateConflict: failed device is not attributed to the retryable attempt"
            );
            true
        } else {
            ensure!(attempt_no == 1, "first Flow attempt number must be one");
            ensure!(
                device_state == FlowDeviceRunState::Running
                    && current_device.capability_snapshot.is_some()
                    && !run_state.is_terminal(),
                "StateConflict: an initial attempt requires a qualified running device"
            );
            false
        };

        if reopening {
            let changed = transaction.execute(
                "UPDATE flow_device_runs SET
                    state='queued',capability_snapshot_json=NULL,release_proof_json=NULL,
                    error_json=NULL,started_at=NULL,finished_at=NULL
                 WHERE id=?1 AND state=?2",
                params![device_run_id.to_string(), enum_name(device_state)?],
            )?;
            ensure!(
                changed == 1,
                "StateConflict: retry device projection changed"
            );
            let nonqueued_devices: i64 = transaction.query_row(
                "SELECT COUNT(*) FROM flow_device_runs WHERE run_id=?1 AND state<>'queued'",
                [run_id.to_string()],
                |row| row.get(0),
            )?;
            let reopened_state = if nonqueued_devices == 0 {
                FlowAggregateState::Queued
            } else {
                FlowAggregateState::Running
            };
            let changed = transaction.execute(
                "UPDATE flow_runs SET state=?2,error_json=NULL WHERE id=?1",
                params![run_id.to_string(), enum_name(reopened_state)?],
            )?;
            ensure!(changed == 1, "StateConflict: retry run projection changed");
        }

        let id = Uuid::new_v4();
        let now = now_text();
        transaction.execute(
            "INSERT INTO flow_node_attempts(
                id,device_run_id,node_id,action_kind,attempt_no,side_effect_class,state,
                canonical_input_json,evidence_baseline_json,evidence_result_json,retry_safe,
                error_json,started_at,updated_at,finished_at
             ) VALUES(?1,?2,?3,?4,?5,?6,'queued',NULL,NULL,NULL,0,NULL,NULL,?7,NULL)",
            params![
                id.to_string(),
                device_run_id.to_string(),
                node.id.to_string(),
                enum_name(node.kind)?,
                i64::from(attempt_no),
                enum_name(side_effect_class)?,
                now
            ],
        )?;
        append_flow_event(
            &transaction,
            run_id,
            "attemptCreated",
            &json!({
                "attemptId": id,
                "deviceRunId": device_run_id,
                "nodeId": node.id,
                "attemptNo": attempt_no,
                "reopened": reopening,
            }),
            &now,
        )?;
        let record =
            query_attempt_record(&transaction, id)?.context("created Flow attempt disappeared")?;
        transaction.commit()?;
        Ok(record)
    }

    pub(crate) fn initialize_flow_device_attempts(
        &self,
        device_run_id: Uuid,
    ) -> anyhow::Result<Vec<FlowNodeAttemptRecord>> {
        let mut connection = self.conn()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let device = query_device_run_record(&transaction, device_run_id)?
            .context("Flow device run does not exist")?;
        ensure!(
            device.state == FlowDeviceRunState::Running && device.capability_snapshot.is_some(),
            "StateConflict: Flow attempts require a qualified running device"
        );
        let run = query_run_record(&transaction, device.run_id)?
            .context("Flow device parent run does not exist")?;
        ensure!(
            !run.state.is_terminal(),
            "StateConflict: terminal Flow run cannot initialize attempts"
        );
        let existing: i64 = transaction.query_row(
            "SELECT COUNT(*) FROM flow_node_attempts WHERE device_run_id=?1",
            [device_run_id.to_string()],
            |row| row.get(0),
        )?;
        ensure!(
            existing == 0,
            "StateConflict: Flow device attempts are already initialized"
        );
        let plan = load_validated_run_plan(&transaction, &run)?;
        let now = now_text();
        let mut attempt_ids = Vec::with_capacity(plan.execution_order.len());
        for node_id in &plan.execution_order {
            let node = plan
                .nodes
                .get(node_id)
                .context("persisted Flow execution order references a missing node")?;
            let id = Uuid::new_v4();
            transaction.execute(
                "INSERT INTO flow_node_attempts(
                    id,device_run_id,node_id,action_kind,attempt_no,side_effect_class,state,
                    canonical_input_json,evidence_baseline_json,evidence_result_json,retry_safe,
                    error_json,started_at,updated_at,finished_at
                 ) VALUES(?1,?2,?3,?4,1,?5,'queued',NULL,NULL,NULL,0,NULL,NULL,?6,NULL)",
                params![
                    id.to_string(),
                    device_run_id.to_string(),
                    node.id.to_string(),
                    enum_name(node.kind)?,
                    enum_name(contracts(node.kind).1)?,
                    now,
                ],
            )?;
            attempt_ids.push(id);
        }
        append_flow_event(
            &transaction,
            run.id,
            "deviceAttemptsInitialized",
            &json!({
                "deviceRunId": device_run_id,
                "attemptIds": attempt_ids,
            }),
            &now,
        )?;
        let mut records = Vec::with_capacity(attempt_ids.len());
        for attempt_id in attempt_ids {
            records.push(
                query_attempt_record(&transaction, attempt_id)?
                    .context("initialized Flow attempt disappeared")?,
            );
        }
        transaction.commit()?;
        Ok(records)
    }

    pub(crate) fn reopen_flow_device_for_recovery(
        &self,
        device_run_id: Uuid,
    ) -> anyhow::Result<FlowDeviceRunRecord> {
        let mut connection = self.conn()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let device = query_device_run_record(&transaction, device_run_id)?
            .context("Flow recovery device does not exist")?;
        ensure!(
            matches!(
                device.state,
                FlowDeviceRunState::Preflight | FlowDeviceRunState::Running
            ),
            "StateConflict: Flow recovery requires a nonterminal started device"
        );
        let unsafe_attempts: i64 = transaction.query_row(
            "SELECT COUNT(*) FROM flow_node_attempts
             WHERE device_run_id=?1 AND state IN (
                'intentCommitted','effectDispatched','verifying','interrupted','uncertain'
             )",
            [device_run_id.to_string()],
            |row| row.get(0),
        )?;
        ensure!(
            unsafe_attempts == 0,
            "StateConflict: Flow recovery device still has unsafe attempts"
        );
        let run = query_run_record(&transaction, device.run_id)?
            .context("Flow recovery parent run disappeared")?;
        ensure!(
            !run.state.is_terminal(),
            "StateConflict: terminal Flow run cannot be reclaimed"
        );
        let changed = transaction.execute(
            "UPDATE flow_device_runs SET
                state='queued',capability_snapshot_json=NULL,release_proof_json=NULL,
                error_json=NULL,started_at=NULL,finished_at=NULL
             WHERE id=?1 AND state=?2",
            params![device_run_id.to_string(), enum_name(device.state)?],
        )?;
        ensure!(changed == 1, "StateConflict: Flow recovery device changed");
        let active_other_devices: i64 = transaction.query_row(
            "SELECT COUNT(*) FROM flow_device_runs
             WHERE run_id=?1 AND id<>?2 AND state<>'queued'",
            params![device.run_id.to_string(), device_run_id.to_string()],
            |row| row.get(0),
        )?;
        let run_state = if active_other_devices == 0 {
            FlowAggregateState::Queued
        } else {
            FlowAggregateState::Running
        };
        transaction.execute(
            "UPDATE flow_runs SET state=?2,error_json=NULL WHERE id=?1",
            params![device.run_id.to_string(), enum_name(run_state)?],
        )?;
        let now = now_text();
        append_flow_event(
            &transaction,
            device.run_id,
            "deviceRunReclaimed",
            &json!({"deviceRunId": device_run_id}),
            &now,
        )?;
        let record = query_device_run_record(&transaction, device_run_id)?
            .context("reclaimed Flow device disappeared")?;
        transaction.commit()?;
        Ok(record)
    }

    pub fn list_flow_runs(&self, limit: usize) -> anyhow::Result<Vec<FlowRunRecord>> {
        ensure!(
            (1..=200).contains(&limit),
            "Flow run list limit must be 1..=200"
        );
        let mut connection = self.conn()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Deferred)?;
        let mut statement = transaction.prepare(
            "SELECT id,flow_id,flow_revision,plan_sha256,selection_json,state,event_revision,
                    error_json,created_at,updated_at
             FROM flow_runs ORDER BY updated_at DESC,id ASC LIMIT ?1",
        )?;
        let rows = statement.query_map([usize_to_sql(limit, "Flow run list limit")?], run_row)?;
        let mut records = Vec::new();
        for row in rows {
            let record = row?.into_record()?;
            validate_run_plan(&transaction, &record)?;
            validate_event_ledger(&transaction, &record)?;
            records.push(record);
        }
        drop(statement);
        transaction.commit()?;
        Ok(records)
    }

    pub(crate) fn record_flow_runtime_error(
        &self,
        run_id: Uuid,
        error: FlowErrorRecord,
    ) -> anyhow::Result<FlowRunRecord> {
        validate_error(&error)?;
        let mut connection = self.conn()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let run = query_run_record(&transaction, run_id)?.context("Flow run does not exist")?;
        ensure!(
            !run.state.is_terminal(),
            "StateConflict: terminal Flow run cannot receive a runtime error"
        );
        let now = now_text();
        let changed = transaction.execute(
            "UPDATE flow_runs SET error_json=?2,updated_at=?3
             WHERE id=?1 AND state IN ('queued','running')",
            params![run_id.to_string(), serde_json::to_string(&error)?, now,],
        )?;
        ensure!(
            changed == 1,
            "StateConflict: Flow runtime projection changed"
        );
        append_flow_event(
            &transaction,
            run_id,
            "runtimeError",
            &json!({"error": error}),
            &now,
        )?;
        let record = query_run_record(&transaction, run_id)?
            .context("Flow runtime error projection disappeared")?;
        transaction.commit()?;
        Ok(record)
    }

    pub fn get_flow_run(&self, run_id: Uuid) -> anyhow::Result<Option<FlowRunDetail>> {
        let mut connection = self.conn()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Deferred)?;
        let detail = query_flow_run_detail(&transaction, run_id)?;
        transaction.commit()?;
        Ok(detail)
    }

    pub fn transition_attempt(
        &self,
        attempt_id: Uuid,
        expected: FlowAttemptState,
        next: FlowAttemptState,
        patch: AttemptTransitionPatch,
    ) -> anyhow::Result<FlowNodeAttemptRecord> {
        validate_attempt_error_identity(attempt_id, patch.error.as_ref())?;
        let mut connection = self.conn()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let identity = query_attempt_identity(&transaction, attempt_id)?
            .context("Flow attempt does not exist")?;
        if identity.state != expected {
            return Err(state_conflict(attempt_id, expected, identity.state, next).into());
        }
        if !legal_transition(expected, next) {
            return Err(state_conflict(attempt_id, expected, identity.state, next).into());
        }
        if expected == FlowAttemptState::Queued && next == FlowAttemptState::IntentCommitted {
            validate_attempt_claim(&transaction, attempt_id, &identity)?;
        }
        validate_attempt_error_context(&identity, patch.error.as_ref())?;
        validate_transition_proof(&identity, expected, next, &patch)?;

        let canonical_input = encode_optional_json(patch.canonical_input.as_ref())?;
        let evidence_baseline = encode_optional_json(patch.evidence_baseline.as_ref())?;
        let evidence_result = encode_optional_json(patch.evidence_result.as_ref())?;
        let error_json = encode_optional_json(patch.error.as_ref())?;
        let now = now_text();
        let expected_name = enum_name(expected)?;
        let next_name = enum_name(next)?;
        let changed = transaction.execute(
            "UPDATE flow_node_attempts SET
                state=?2,
                canonical_input_json=COALESCE(?3,canonical_input_json),
                evidence_baseline_json=COALESCE(?4,evidence_baseline_json),
                evidence_result_json=COALESCE(?5,evidence_result_json),
                error_json=CASE WHEN ?6=1 THEN NULL ELSE COALESCE(?7,error_json) END,
                started_at=CASE
                    WHEN started_at IS NULL AND ?2='intentCommitted' THEN ?8
                    ELSE started_at END,
                updated_at=?8,
                finished_at=CASE WHEN ?9=1 THEN ?8 ELSE finished_at END
             WHERE id=?1 AND state=?10",
            params![
                attempt_id.to_string(),
                next_name,
                canonical_input,
                evidence_baseline,
                evidence_result,
                i64::from(next == FlowAttemptState::Succeeded),
                error_json,
                now,
                i64::from(next.is_terminal()),
                expected_name,
            ],
        )?;
        if changed != 1 {
            let actual = query_attempt_identity(&transaction, attempt_id)?
                .context("Flow attempt disappeared")?
                .state;
            return Err(state_conflict(attempt_id, expected, actual, next).into());
        }
        append_flow_event(
            &transaction,
            identity.run_id,
            "attemptTransitioned",
            &json!({
                "attemptId": attempt_id,
                "expected": expected,
                "next": next,
            }),
            &now,
        )?;
        let record = query_attempt_record(&transaction, attempt_id)?
            .context("transitioned Flow attempt disappeared")?;
        transaction.commit()?;
        Ok(record)
    }

    pub fn record_nonterminal_attempt_error(
        &self,
        attempt_id: Uuid,
        expected: FlowAttemptState,
        error: FlowErrorRecord,
    ) -> anyhow::Result<FlowNodeAttemptRecord> {
        ensure!(
            matches!(
                expected,
                FlowAttemptState::IntentCommitted
                    | FlowAttemptState::EffectDispatched
                    | FlowAttemptState::Verifying
            ),
            "nonterminal error state is not recordable"
        );
        validate_attempt_error_identity(attempt_id, Some(&error))?;
        let mut connection = self.conn()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let identity = query_attempt_identity(&transaction, attempt_id)?
            .context("Flow attempt does not exist")?;
        if identity.state != expected {
            return Err(state_conflict(attempt_id, expected, identity.state, expected).into());
        }
        ensure_effect_device_ready(&identity)?;
        validate_attempt_error_context(&identity, Some(&error))?;
        let now = now_text();
        let changed = transaction.execute(
            "UPDATE flow_node_attempts SET error_json=?2,updated_at=?3
             WHERE id=?1 AND state=?4",
            params![
                attempt_id.to_string(),
                serde_json::to_string(&error)?,
                now,
                enum_name(expected)?,
            ],
        )?;
        if changed != 1 {
            bail!("StateConflict: Flow attempt changed while recording its error");
        }
        append_flow_event(
            &transaction,
            identity.run_id,
            "attemptErrorRecorded",
            &json!({"attemptId": attempt_id, "state": expected, "code": error.code}),
            &now,
        )?;
        let record = query_attempt_record(&transaction, attempt_id)?
            .context("updated Flow attempt disappeared")?;
        transaction.commit()?;
        Ok(record)
    }

    pub fn publish_artifact_and_succeed(
        &self,
        attempt_id: Uuid,
        artifact: &FlowArtifactRecord,
    ) -> anyhow::Result<FlowNodeAttemptRecord> {
        ensure!(
            artifact.attempt_id == attempt_id,
            "artifact attempt ID mismatch"
        );
        ensure!(artifact.size > 0, "artifact size must be positive");
        ensure!(
            is_lower_sha256(&artifact.sha256),
            "artifact SHA-256 is invalid"
        );
        validate_artifact_label(&artifact.label, &artifact.kind).map_err(anyhow::Error::msg)?;
        let mut connection = self.conn()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let identity = query_attempt_identity(&transaction, attempt_id)?
            .context("Flow attempt does not exist")?;
        ensure!(
            identity.state == FlowAttemptState::Verifying
                && identity.side_effect_class == SideEffectClass::ArtifactWrite,
            "StateConflict: artifact publication requires a verifying artifact-write attempt"
        );
        ensure_effect_device_ready(&identity)?;
        let CompiledActionConfig::Screenshot { label, format } = &identity.node.config else {
            bail!("persisted artifact attempt is not a compiled Screenshot");
        };
        ensure!(
            identity.node.postcondition == Some(EvidenceSpec::ArtifactDecodedAndHashed)
                && artifact.label == *label
                && artifact.kind == *format,
            "artifact publication does not match the compiled Screenshot"
        );
        validate_artifact_record_relative_path(
            &artifact.relative_path,
            identity.run_id,
            identity.device_run_id,
            attempt_id,
            artifact.id,
            &artifact.kind,
        )?;
        transaction.execute(
            "INSERT INTO flow_artifacts(
                id,attempt_id,relative_path,label,kind,size,sha256,created_at
             ) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
            params![
                artifact.id.to_string(),
                artifact.attempt_id.to_string(),
                artifact.relative_path,
                artifact.label,
                artifact.kind,
                u64_to_sql(artifact.size, "artifact size")?,
                artifact.sha256,
                datetime_text(artifact.created_at),
            ],
        )?;
        let now = now_text();
        let evidence = serde_json::to_string(&json!({
            "kind": "artifactDecodedAndHashed",
            "matched": true,
            "observedSha256": artifact.sha256,
            "measurement": {
                "artifactId": artifact.id,
                "size": artifact.size,
                "label": artifact.label,
                "format": artifact.kind,
            },
        }))?;
        let changed = transaction.execute(
            "UPDATE flow_node_attempts SET
                state='succeeded',evidence_result_json=?2,error_json=NULL,
                updated_at=?3,finished_at=?3
             WHERE id=?1 AND state='verifying' AND side_effect_class='artifactWrite'",
            params![attempt_id.to_string(), evidence, now],
        )?;
        ensure!(
            changed == 1,
            "StateConflict: artifact attempt changed during publication"
        );
        append_flow_event(
            &transaction,
            identity.run_id,
            "artifactPublished",
            &json!({"attemptId": attempt_id, "artifactId": artifact.id}),
            &now,
        )?;
        let record = query_attempt_record(&transaction, attempt_id)?
            .context("completed artifact attempt disappeared")?;
        transaction.commit()?;
        Ok(record)
    }

    pub fn record_retry_safe_reconciliation(
        &self,
        attempt_id: Uuid,
        evidence_result: Value,
    ) -> anyhow::Result<FlowNodeAttemptRecord> {
        let mut connection = self.conn()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let identity = query_attempt_identity(&transaction, attempt_id)?
            .context("Flow attempt does not exist")?;
        ensure!(
            identity.state == FlowAttemptState::FailedVerified
                && identity.side_effect_class == SideEffectClass::IdempotentSet,
            "StateConflict: retry safety requires failedVerified idempotentSet"
        );
        let latest_attempt_id: String = transaction.query_row(
            "SELECT id FROM flow_node_attempts
             WHERE device_run_id=?1 AND node_id=?2
             ORDER BY attempt_no DESC LIMIT 1",
            params![
                identity.device_run_id.to_string(),
                identity.node.id.to_string()
            ],
            |row| row.get(0),
        )?;
        ensure!(
            latest_attempt_id == attempt_id.to_string(),
            "StateConflict: retry safety requires the latest Flow attempt"
        );
        let device = query_device_run_record(&transaction, identity.device_run_id)?
            .context("retry-safe Flow device disappeared")?;
        ensure!(
            device.state == FlowDeviceRunState::Failed
                && device.error.as_ref().and_then(|error| error.attempt_id) == Some(attempt_id),
            "StateConflict: retry safety requires the attributed failed device"
        );
        validate_retry_safety_evidence(&identity, &evidence_result)?;
        let now = now_text();
        let changed = transaction.execute(
            "UPDATE flow_node_attempts SET evidence_result_json=?2,retry_safe=1,updated_at=?3
             WHERE id=?1 AND state='failedVerified' AND side_effect_class='idempotentSet'
            ",
            params![
                attempt_id.to_string(),
                serde_json::to_string(&evidence_result)?,
                now
            ],
        )?;
        ensure!(changed == 1, "StateConflict: retry-safe attempt changed");
        append_flow_event(
            &transaction,
            identity.run_id,
            "retrySafetyProved",
            &json!({"attemptId": attempt_id, "evidence": evidence_result}),
            &now,
        )?;
        let record = query_attempt_record(&transaction, attempt_id)?
            .context("retry-safe Flow attempt disappeared")?;
        transaction.commit()?;
        Ok(record)
    }

    pub fn mark_device_terminal(
        &self,
        device_run_id: Uuid,
        expected: &[FlowDeviceRunState],
        next: FlowDeviceRunState,
        error: Option<FlowErrorRecord>,
        release_proof: FlowContextReleaseProof,
    ) -> anyhow::Result<FlowDeviceRunRecord> {
        ensure!(
            !expected.is_empty(),
            "expected device states cannot be empty"
        );
        ensure!(next.is_terminal(), "next device state must be terminal");
        ensure!(
            release_proof.owner == DeviceWorkOwner::Script,
            "Flow release proof must belong to the Script owner"
        );
        ensure!(
            !release_proof.had_stream || release_proof.had_session,
            "a released stream requires a released session"
        );
        let mut connection = self.conn()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let current = query_device_run_record(&transaction, device_run_id)?
            .context("Flow device run does not exist")?;
        ensure!(
            release_proof.udid == current.udid,
            "Flow release proof UDID mismatch"
        );
        ensure!(
            !current.state.is_terminal() && expected.contains(&current.state),
            "StateConflict: device run expected {:?}, actual {:?}",
            expected,
            current.state
        );
        let active_attempts: i64 = transaction.query_row(
            "SELECT COUNT(*) FROM flow_node_attempts
             WHERE device_run_id=?1 AND state IN (
                'intentCommitted','effectDispatched','verifying','interrupted'
             )",
            [device_run_id.to_string()],
            |row| row.get(0),
        )?;
        ensure!(
            active_attempts == 0,
            "StateConflict: device run still owns active attempts"
        );
        if next == FlowDeviceRunState::Succeeded {
            ensure!(
                current.state == FlowDeviceRunState::Running
                    && current.capability_snapshot.is_some(),
                "StateConflict: device success requires a qualified running device"
            );
            let queued_attempts: i64 = transaction.query_row(
                "SELECT COUNT(*) FROM flow_node_attempts
                 WHERE device_run_id=?1 AND state='queued'",
                [device_run_id.to_string()],
                |row| row.get(0),
            )?;
            ensure!(
                queued_attempts == 0,
                "StateConflict: a successful device still has queued successors"
            );
            validate_device_success_projection(&transaction, &current)?;
        }
        if let Some(error) = &error {
            validate_error(error)?;
            if let Some(udid) = &error.udid {
                ensure!(udid == &current.udid, "Flow device error UDID mismatch");
            }
            if let Some(attempt_id) = error.attempt_id {
                let attempt = query_attempt_identity(&transaction, attempt_id)?
                    .context("Flow device error references a missing attempt")?;
                ensure!(
                    attempt.device_run_id == device_run_id
                        && attempt.state.is_terminal()
                        && !matches!(
                            attempt.state,
                            FlowAttemptState::Succeeded | FlowAttemptState::Cancelled
                        )
                        && error
                            .node_id
                            .is_none_or(|node_id| node_id == attempt.node.id),
                    "Flow device error attempt attribution mismatch"
                );
                let latest_attempt_id: String = transaction.query_row(
                    "SELECT id FROM flow_node_attempts
                     WHERE device_run_id=?1 AND node_id=?2
                     ORDER BY attempt_no DESC LIMIT 1",
                    params![device_run_id.to_string(), attempt.node.id.to_string()],
                    |row| row.get(0),
                )?;
                ensure!(
                    latest_attempt_id == attempt_id.to_string(),
                    "Flow device error must reference the latest attempt for its node"
                );
            }
        }
        if next == FlowDeviceRunState::Skipped {
            ensure!(
                matches!(
                    current.state,
                    FlowDeviceRunState::Queued | FlowDeviceRunState::Preflight
                ),
                "StateConflict: a running Flow device cannot be hidden as skipped"
            );
            let run = query_run_record(&transaction, current.run_id)?
                .context("Flow device parent run disappeared")?;
            ensure!(
                matches!(run.selection.requested, FlowTargetSelection::AllEligible),
                "StateConflict: only AllEligible selection may skip a device"
            );
            let attempts: i64 = transaction.query_row(
                "SELECT COUNT(*) FROM flow_node_attempts WHERE device_run_id=?1",
                [device_run_id.to_string()],
                |row| row.get(0),
            )?;
            ensure!(
                attempts == 0,
                "StateConflict: a Flow device with attempts cannot become skipped"
            );
        }
        ensure!(
            next != FlowDeviceRunState::Succeeded || error.is_none(),
            "a successful Flow device cannot carry an error"
        );
        let now = now_text();
        let changed = transaction.execute(
            "UPDATE flow_device_runs SET
                state=?2,release_proof_json=?3,error_json=?4,
                started_at=COALESCE(started_at,?5),finished_at=?5
             WHERE id=?1 AND state=?6",
            params![
                device_run_id.to_string(),
                enum_name(next)?,
                serde_json::to_string(&release_proof)?,
                encode_optional_json(error.as_ref())?,
                now,
                enum_name(current.state)?,
            ],
        )?;
        ensure!(changed == 1, "StateConflict: Flow device run changed");
        append_flow_event(
            &transaction,
            current.run_id,
            "deviceRunTerminal",
            &json!({
                "deviceRunId": device_run_id,
                "state": next,
                "udid": current.udid,
                "releaseProof": release_proof,
                "error": error,
            }),
            &now,
        )?;
        let record = query_device_run_record(&transaction, device_run_id)?
            .context("terminal Flow device run disappeared")?;
        transaction.commit()?;
        Ok(record)
    }

    pub fn recompute_run_projection(&self, run_id: Uuid) -> anyhow::Result<FlowRunRecord> {
        let mut connection = self.conn()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let run = query_run_record(&transaction, run_id)?
            .with_context(|| format!("Flow run {run_id} does not exist"))?;
        validate_run_plan(&transaction, &run)?;
        validate_event_ledger(&transaction, &run)?;
        let mut statement = transaction.prepare(
            "SELECT id,run_id,udid,state,capability_snapshot_json,release_proof_json,
                    error_json,started_at,finished_at FROM flow_device_runs
             WHERE run_id=?1 ORDER BY udid ASC,id ASC",
        )?;
        let rows = statement.query_map([run_id.to_string()], device_run_row)?;
        let mut device_records = Vec::new();
        let mut observed_udids = BTreeSet::new();
        for row in rows {
            let record = row?.into_record()?;
            ensure!(
                observed_udids.insert(record.udid.clone()),
                "duplicate Flow device projection"
            );
            device_records.push(record);
        }
        drop(statement);
        let selected_udids = run
            .selection
            .target_udids
            .iter()
            .cloned()
            .collect::<BTreeSet<_>>();
        ensure!(
            selected_udids.len() == run.selection.target_udids.len()
                && observed_udids == selected_udids,
            "Flow device projection does not match its frozen selection"
        );

        let mut attempt_statement = transaction.prepare(
            "SELECT a.id,a.device_run_id,a.node_id,a.action_kind,a.attempt_no,
                    a.side_effect_class,a.state,a.canonical_input_json,
                    a.evidence_baseline_json,a.evidence_result_json,a.retry_safe,a.error_json,
                    a.started_at,a.updated_at,a.finished_at
             FROM flow_node_attempts a
             JOIN flow_device_runs d ON d.id=a.device_run_id
             WHERE d.run_id=?1
             ORDER BY a.device_run_id ASC,a.attempt_no ASC,a.node_id ASC,a.id ASC",
        )?;
        let attempt_rows = attempt_statement.query_map([run_id.to_string()], attempt_row)?;
        let mut attempts = Vec::new();
        for row in attempt_rows {
            let attempt = row?.into_record()?;
            validate_persisted_attempt(&transaction, &attempt)?;
            attempts.push(attempt);
        }
        drop(attempt_statement);
        validate_device_attempt_projection(&run, &device_records, &attempts)?;
        for device in &device_records {
            if device.state == FlowDeviceRunState::Succeeded {
                validate_device_success_projection(&transaction, device)?;
            }
        }

        let mut artifact_statement = transaction.prepare(
            "SELECT a.id,a.attempt_id,a.relative_path,a.label,a.kind,a.size,a.sha256,a.created_at
             FROM flow_artifacts a
             JOIN flow_node_attempts n ON n.id=a.attempt_id
             JOIN flow_device_runs d ON d.id=n.device_run_id
             WHERE d.run_id=?1 ORDER BY a.created_at ASC,a.id ASC",
        )?;
        let artifact_rows = artifact_statement.query_map([run_id.to_string()], artifact_row)?;
        let mut artifacts = Vec::new();
        for row in artifact_rows {
            artifacts.push(row?.into_record()?);
        }
        drop(artifact_statement);
        validate_artifact_projection(run_id, &device_records, &attempts, &artifacts)?;

        let devices = device_records
            .iter()
            .map(|device| (device.state, device.error.clone()))
            .collect::<Vec<_>>();

        let (state, error) = aggregate_projection(&devices);
        let now = now_text();
        let changed = transaction.execute(
            "UPDATE flow_runs SET state=?2,error_json=?3,updated_at=?4 WHERE id=?1",
            params![
                run_id.to_string(),
                enum_name(state)?,
                encode_optional_json(error.as_ref())?,
                now
            ],
        )?;
        ensure!(changed == 1, "Flow run disappeared during projection");
        append_flow_event(
            &transaction,
            run_id,
            "runProjectionRecomputed",
            &json!({"state": state}),
            &now,
        )?;
        let record =
            query_run_record(&transaction, run_id)?.context("projected Flow run disappeared")?;
        transaction.commit()?;
        Ok(record)
    }

    pub fn load_nonterminal_attempts(&self) -> anyhow::Result<Vec<FlowNodeAttemptRecord>> {
        let connection = self.conn()?;
        let mut statement = connection.prepare(
            "SELECT id,device_run_id,node_id,action_kind,attempt_no,side_effect_class,state,
                    canonical_input_json,evidence_baseline_json,evidence_result_json,retry_safe,
                    error_json,started_at,updated_at,finished_at
             FROM flow_node_attempts
             WHERE state NOT IN (
                'succeeded','failedBeforeDispatch','failedVerified','uncertain','cancelled'
             )
             ORDER BY device_run_id ASC,attempt_no ASC,node_id ASC,id ASC",
        )?;
        let rows = statement.query_map([], attempt_row)?;
        let mut records = Vec::new();
        for row in rows {
            let record = row?.into_record()?;
            ensure!(
                !record.state.is_terminal(),
                "terminal Flow attempt leaked from query"
            );
            validate_persisted_attempt(&connection, &record)?;
            records.push(record);
        }
        Ok(records)
    }

    pub(crate) fn load_flow_recovery_contexts(
        &self,
    ) -> anyhow::Result<Vec<FlowRecoveryRunContext>> {
        let mut connection = self.conn()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Deferred)?;
        let mut statement = transaction.prepare(
            "SELECT id FROM flow_runs
             WHERE state IN ('queued','running')
             ORDER BY created_at ASC,id ASC",
        )?;
        let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
        let mut run_ids = Vec::new();
        for row in rows {
            run_ids.push(parse_uuid(&row?, "Flow recovery run ID")?);
        }
        drop(statement);

        let mut contexts = Vec::with_capacity(run_ids.len());
        for run_id in run_ids {
            let mut detail = query_flow_run_detail(&transaction, run_id)?
                .context("nonterminal Flow recovery run disappeared")?;
            ensure!(
                !detail.run.state.is_terminal(),
                "terminal Flow run leaked into recovery query"
            );
            let persisted_udids = detail
                .device_runs
                .iter()
                .map(|device| device.udid.as_str())
                .collect::<Vec<_>>();
            let selected_udids = detail
                .run
                .selection
                .target_udids
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>();
            ensure!(
                persisted_udids == selected_udids,
                "Flow recovery device projection does not match its frozen selection"
            );
            let plan = load_validated_run_plan(&transaction, &detail.run)?;
            detail.attempts.sort_by_key(|attempt| {
                let device_index = detail
                    .device_runs
                    .iter()
                    .position(|device| device.id == attempt.device_run_id)
                    .unwrap_or(usize::MAX);
                let node_index = plan
                    .execution_order
                    .iter()
                    .position(|node_id| *node_id == attempt.node_id)
                    .unwrap_or(usize::MAX);
                (device_index, node_index, attempt.attempt_no, attempt.id)
            });
            contexts.push(FlowRecoveryRunContext {
                run: detail.run,
                plan,
                devices: detail.device_runs,
                attempts: detail.attempts,
                artifacts: detail.artifacts,
            });
        }
        transaction.commit()?;
        Ok(contexts)
    }

    pub(crate) fn get_flow_attempt_execution_context(
        &self,
        attempt_id: Uuid,
    ) -> anyhow::Result<Option<FlowAttemptExecutionContext>> {
        let connection = self.conn()?;
        let Some(attempt) = query_attempt_record(&connection, attempt_id)? else {
            return Ok(None);
        };
        let identity = query_attempt_identity(&connection, attempt_id)?
            .context("Flow attempt identity disappeared")?;
        let run = query_run_record(&connection, identity.run_id)?
            .context("Flow attempt parent run disappeared")?;
        validate_run_plan(&connection, &run)?;
        validate_event_ledger(&connection, &run)?;
        let device = query_device_run_record(&connection, identity.device_run_id)?
            .context("Flow attempt device run disappeared")?;
        let plan = load_validated_run_plan(&connection, &run)?;
        let mut statement = connection.prepare(
            "SELECT id,device_run_id,node_id,action_kind,attempt_no,side_effect_class,state,
                    canonical_input_json,evidence_baseline_json,evidence_result_json,retry_safe,
                    error_json,started_at,updated_at,finished_at
             FROM flow_node_attempts WHERE device_run_id=?1
             ORDER BY attempt_no ASC,node_id ASC,id ASC",
        )?;
        let rows = statement.query_map([device.id.to_string()], attempt_row)?;
        let mut device_attempts = Vec::new();
        for row in rows {
            let record = row?.into_record()?;
            validate_persisted_attempt(&connection, &record)?;
            device_attempts.push(record);
        }
        drop(statement);
        Ok(Some(FlowAttemptExecutionContext {
            run,
            device,
            attempt,
            plan,
            device_attempts,
        }))
    }
}

fn validate_artifact_projection(
    run_id: Uuid,
    device_runs: &[FlowDeviceRunRecord],
    attempts: &[FlowNodeAttemptRecord],
    artifacts: &[FlowArtifactRecord],
) -> anyhow::Result<()> {
    for attempt in attempts {
        let artifact_count = artifacts
            .iter()
            .filter(|artifact| artifact.attempt_id == attempt.id)
            .count();
        if attempt.side_effect_class == SideEffectClass::ArtifactWrite
            && attempt.state == FlowAttemptState::Succeeded
        {
            ensure!(
                artifact_count == 1,
                "persisted succeeded artifact attempt must own exactly one artifact"
            );
        } else {
            ensure!(
                artifact_count == 0,
                "persisted artifact belongs to an ineligible attempt"
            );
        }
    }
    ensure!(
        artifacts.iter().all(|artifact| attempts
            .iter()
            .any(|attempt| attempt.id == artifact.attempt_id)),
        "persisted artifact has no attempt projection"
    );
    for artifact in artifacts {
        let attempt = attempts
            .iter()
            .find(|attempt| attempt.id == artifact.attempt_id)
            .context("persisted artifact has no attempt projection")?;
        ensure!(
            device_runs
                .iter()
                .any(|device| device.id == attempt.device_run_id && device.run_id == run_id),
            "persisted artifact attempt has no device projection"
        );
        validate_artifact_record_relative_path(
            &artifact.relative_path,
            run_id,
            attempt.device_run_id,
            attempt.id,
            artifact.id,
            &artifact.kind,
        )?;
        let evidence = attempt
            .evidence_result
            .as_ref()
            .and_then(Value::as_object)
            .context("persisted artifact success has no evidence object")?;
        let measurement = evidence
            .get("measurement")
            .and_then(Value::as_object)
            .context("persisted artifact success has no measurement")?;
        let artifact_id = artifact.id.to_string();
        ensure!(
            evidence.get("observedSha256").and_then(Value::as_str)
                == Some(artifact.sha256.as_str())
                && measurement.len() == 4
                && measurement.get("artifactId").and_then(Value::as_str)
                    == Some(artifact_id.as_str())
                && measurement.get("size").and_then(Value::as_u64) == Some(artifact.size)
                && measurement.get("label").and_then(Value::as_str)
                    == Some(artifact.label.as_str())
                && measurement.get("format").and_then(Value::as_str)
                    == Some(artifact.kind.as_str()),
            "persisted artifact evidence does not match its artifact row"
        );
    }
    Ok(())
}

fn validate_device_attempt_projection(
    run: &FlowRunRecord,
    device_runs: &[FlowDeviceRunRecord],
    attempts: &[FlowNodeAttemptRecord],
) -> anyhow::Result<()> {
    ensure!(
        attempts.iter().all(|attempt| device_runs
            .iter()
            .any(|device| device.id == attempt.device_run_id)),
        "persisted Flow attempt has no device projection"
    );
    for device in device_runs {
        let device_attempts = attempts
            .iter()
            .filter(|attempt| attempt.device_run_id == device.id)
            .collect::<Vec<_>>();
        if device.state.is_terminal() {
            ensure!(
                device_attempts
                    .iter()
                    .all(|attempt| !state_owns_active_context(attempt.state)),
                "persisted terminal Flow device still owns an active attempt"
            );
        }
        if device.state == FlowDeviceRunState::Succeeded {
            ensure!(
                device_attempts
                    .iter()
                    .all(|attempt| attempt.state.is_terminal()),
                "persisted successful Flow device has unfinished attempts"
            );
        }
        if device.state == FlowDeviceRunState::Skipped {
            ensure!(
                matches!(run.selection.requested, FlowTargetSelection::AllEligible)
                    && device_attempts.is_empty()
                    && device.capability_snapshot.is_none(),
                "persisted skipped Flow device violates its selection or attempt boundary"
            );
        }
        let Some(error_attempt_id) = device.error.as_ref().and_then(|error| error.attempt_id)
        else {
            continue;
        };
        let attempt = attempts
            .iter()
            .find(|attempt| attempt.id == error_attempt_id)
            .context("persisted Flow device error references a missing attempt")?;
        ensure!(
            attempt.device_run_id == device.id
                && attempt.state.is_terminal()
                && !matches!(
                    attempt.state,
                    FlowAttemptState::Succeeded | FlowAttemptState::Cancelled
                )
                && device
                    .error
                    .as_ref()
                    .and_then(|error| error.node_id)
                    .is_none_or(|node_id| node_id == attempt.node_id),
            "persisted Flow device error attempt attribution mismatch"
        );
        let latest_attempt = device_attempts
            .iter()
            .filter(|candidate| candidate.node_id == attempt.node_id)
            .max_by_key(|candidate| candidate.attempt_no)
            .context("persisted Flow device error node has no attempts")?;
        ensure!(
            latest_attempt.id == error_attempt_id,
            "persisted Flow device error does not reference the latest attempt for its node"
        );
    }
    Ok(())
}

fn legal_transition(from: FlowAttemptState, to: FlowAttemptState) -> bool {
    use FlowAttemptState::*;
    matches!(
        (from, to),
        (Queued, IntentCommitted)
            | (Queued, Cancelled)
            | (IntentCommitted, EffectDispatched)
            | (IntentCommitted, FailedBeforeDispatch)
            | (IntentCommitted, Cancelled)
            | (EffectDispatched, Verifying)
            | (EffectDispatched, FailedBeforeDispatch)
            | (EffectDispatched, Uncertain)
            | (EffectDispatched, Interrupted)
            | (EffectDispatched, Cancelled)
            | (Verifying, Succeeded)
            | (Verifying, FailedVerified)
            | (Verifying, Uncertain)
            | (Verifying, Interrupted)
            | (Verifying, Cancelled)
            | (Queued, Interrupted)
            | (Interrupted, Queued)
    )
}

fn state_requires_committed_intent(state: FlowAttemptState) -> bool {
    matches!(
        state,
        FlowAttemptState::IntentCommitted
            | FlowAttemptState::EffectDispatched
            | FlowAttemptState::Verifying
            | FlowAttemptState::Succeeded
            | FlowAttemptState::FailedBeforeDispatch
            | FlowAttemptState::FailedVerified
            | FlowAttemptState::Uncertain
    )
}

fn state_owns_active_context(state: FlowAttemptState) -> bool {
    matches!(
        state,
        FlowAttemptState::IntentCommitted
            | FlowAttemptState::EffectDispatched
            | FlowAttemptState::Verifying
            | FlowAttemptState::Interrupted
    )
}

fn validate_transition_proof(
    identity: &AttemptIdentity,
    from: FlowAttemptState,
    to: FlowAttemptState,
    patch: &AttemptTransitionPatch,
) -> anyhow::Result<()> {
    if to == FlowAttemptState::IntentCommitted
        || matches!(
            from,
            FlowAttemptState::IntentCommitted
                | FlowAttemptState::EffectDispatched
                | FlowAttemptState::Verifying
        )
    {
        ensure_effect_device_ready(identity)?;
    }
    if from == FlowAttemptState::Queued && to == FlowAttemptState::IntentCommitted {
        let canonical_input = patch
            .canonical_input
            .as_ref()
            .context("StateConflict: intent requires canonical input")?;
        let evidence_baseline = patch
            .evidence_baseline
            .as_ref()
            .context("StateConflict: intent requires an evidence baseline")?;
        validate_evidence_baseline(identity, evidence_baseline)?;
        if let Some(existing) = &identity.canonical_input {
            ensure!(
                existing == canonical_input,
                "StateConflict: committed canonical input is immutable"
            );
        }
        if identity.canonical_input.is_some() {
            ensure!(
                identity.evidence_baseline == patch.evidence_baseline,
                "StateConflict: committed evidence baseline is immutable"
            );
        }
    } else {
        ensure!(
            patch.canonical_input.is_none() && patch.evidence_baseline.is_none(),
            "StateConflict: intent fields may only be committed at IntentCommitted"
        );
    }
    if patch.evidence_result.is_some() {
        ensure!(
            matches!(
                (from, to),
                (
                    FlowAttemptState::EffectDispatched,
                    FlowAttemptState::FailedBeforeDispatch | FlowAttemptState::Uncertain
                ) | (
                    FlowAttemptState::Verifying,
                    FlowAttemptState::Succeeded
                        | FlowAttemptState::FailedVerified
                        | FlowAttemptState::Uncertain
                )
            ),
            "StateConflict: evidence result cannot be written before dispatch"
        );
    }
    if matches!(
        from,
        FlowAttemptState::EffectDispatched | FlowAttemptState::Verifying
    ) && matches!(
        to,
        FlowAttemptState::Interrupted | FlowAttemptState::Cancelled
    ) {
        ensure!(
            identity.side_effect_class == SideEffectClass::None,
            "StateConflict: a dispatched side effect must become Uncertain"
        );
    }
    if from == FlowAttemptState::EffectDispatched && to == FlowAttemptState::FailedBeforeDispatch {
        ensure!(
            patch
                .evidence_result
                .as_ref()
                .is_some_and(is_transport_non_delivery_proof),
            "StateConflict: failedBeforeDispatch requires typed transport non-delivery proof"
        );
    }
    if to == FlowAttemptState::Succeeded {
        ensure!(
            patch.error.is_none(),
            "StateConflict: success cannot carry an error"
        );
        ensure!(
            identity.side_effect_class != SideEffectClass::ArtifactWrite,
            "StateConflict: artifact success requires atomic artifact publication"
        );
        if let Some(postcondition) = &identity.node.postcondition {
            let evidence = patch
                .evidence_result
                .as_ref()
                .context("StateConflict: success requires typed evidence")?;
            validate_success_evidence(identity, postcondition, evidence)?;
        } else {
            ensure!(
                contracts(identity.action_kind).2 == EvidenceRequirement::None,
                "StateConflict: compiled action is missing its required postcondition"
            );
            ensure!(
                patch.evidence_result.is_none(),
                "StateConflict: an evidence-free action cannot carry success evidence"
            );
        }
    }
    if to == FlowAttemptState::FailedVerified {
        ensure!(
            patch
                .error
                .as_ref()
                .and_then(|error| error.attempt_id)
                .is_some(),
            "StateConflict: failedVerified requires a typed error bound to the attempt"
        );
        if let Some(evidence) = &patch.evidence_result {
            validate_failed_verification_evidence(identity, evidence)?;
        }
    }
    Ok(())
}

fn ensure_effect_device_ready(identity: &AttemptIdentity) -> anyhow::Result<()> {
    ensure!(
        identity.device_state == FlowDeviceRunState::Running
            && identity.capability_snapshot.is_some(),
        "StateConflict: effect boundary requires a qualified running device"
    );
    Ok(())
}

fn validate_evidence_baseline(identity: &AttemptIdentity, value: &Value) -> anyhow::Result<()> {
    let object = value
        .as_object()
        .context("evidence baseline must be a typed object")?;
    let kind = object
        .get("kind")
        .and_then(Value::as_str)
        .context("evidence baseline kind is missing")?;
    let expected = match contracts(identity.action_kind).2 {
        EvidenceRequirement::None | EvidenceRequirement::ActiveApp => "none",
        EvidenceRequirement::Process => "process",
        EvidenceRequirement::Frame
        | EvidenceRequirement::TextOrQualifiedFrame
        | EvidenceRequirement::Artifact => "frame",
    };
    ensure!(
        kind == expected,
        "evidence baseline kind does not match the action"
    );
    match expected {
        "none" => ensure!(object.len() == 1, "none baseline must use the exact schema"),
        "process" => validate_process_baseline(&identity.node, object)?,
        "frame" => validate_frame_baseline(object)?,
        _ => unreachable!("matched baseline kind"),
    }
    Ok(())
}

fn validate_frame_baseline(object: &serde_json::Map<String, Value>) -> anyhow::Result<()> {
    ensure!(
        object.len() == 6
            && object.get("generation").and_then(Value::as_u64).is_some()
            && object
                .get("jpegSha256")
                .and_then(Value::as_str)
                .is_some_and(is_lower_sha256),
        "frame baseline is incomplete"
    );
    let width = object
        .get("imageWidth")
        .and_then(Value::as_u64)
        .filter(|width| *width > 0 && *width <= u64::from(u32::MAX))
        .context("frame baseline width is invalid")?;
    let height = object
        .get("imageHeight")
        .and_then(Value::as_u64)
        .filter(|height| *height > 0 && *height <= u64::from(u32::MAX))
        .context("frame baseline height is invalid")?;
    let encoded = object
        .get("rgbBase64")
        .and_then(Value::as_str)
        .context("frame baseline RGB bytes are missing")?;
    let rgb = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .context("decode frame baseline RGB bytes")?;
    let expected_len = width
        .checked_mul(height)
        .and_then(|pixels| pixels.checked_mul(3))
        .and_then(|bytes| usize::try_from(bytes).ok())
        .context("frame baseline dimensions overflow")?;
    ensure!(
        rgb.len() == expected_len,
        "frame baseline RGB length mismatch"
    );
    Ok(())
}

fn validate_process_baseline(
    node: &CompiledFlowNode,
    object: &serde_json::Map<String, Value>,
) -> anyhow::Result<()> {
    let CompiledActionConfig::TerminateApp { bundle_id } = &node.config else {
        bail!("process baseline requires a compiled Terminate App node");
    };
    ensure!(
        object.len() == 3
            && object.get("bundleId").and_then(Value::as_str) == Some(bundle_id.as_str())
            && object.contains_key("pid")
            && (object.get("pid").is_some_and(Value::is_null)
                || object
                    .get("pid")
                    .and_then(Value::as_u64)
                    .is_some_and(|pid| pid > 0)),
        "process baseline does not match the compiled action"
    );
    Ok(())
}

fn validate_success_evidence(
    identity: &AttemptIdentity,
    postcondition: &EvidenceSpec,
    value: &Value,
) -> anyhow::Result<()> {
    validate_evidence_envelope(value)?;
    let object = value.as_object().expect("validated evidence object");
    let expected_kind = evidence_kind(postcondition);
    let expected_kind = enum_name(expected_kind)?;
    ensure!(
        object.get("kind").and_then(Value::as_str) == Some(expected_kind.as_str()),
        "success evidence kind does not match the compiled postcondition"
    );
    ensure!(
        object.get("matched") == Some(&Value::Bool(true)),
        "success evidence must be matched"
    );
    let measurement = object
        .get("measurement")
        .and_then(Value::as_object)
        .expect("validated evidence measurement");
    let observed_sha256 = object
        .get("observedSha256")
        .and_then(Value::as_str)
        .expect("validated observed SHA-256");
    validate_success_measurement(identity, postcondition, measurement, observed_sha256)?;
    Ok(())
}

fn validate_failed_verification_evidence(
    identity: &AttemptIdentity,
    value: &Value,
) -> anyhow::Result<()> {
    validate_evidence_envelope(value)?;
    let object = value.as_object().expect("validated evidence object");
    let postcondition = identity
        .node
        .postcondition
        .as_ref()
        .context("failed verification evidence requires a compiled postcondition")?;
    ensure!(
        object.get("kind").and_then(Value::as_str)
            == Some(enum_name(evidence_kind(postcondition))?.as_str()),
        "failed verification evidence kind does not match the compiled postcondition"
    );
    ensure!(
        object.get("matched") == Some(&Value::Bool(false)),
        "failed verification evidence must record a mismatch"
    );
    let measurement = object
        .get("measurement")
        .and_then(Value::as_object)
        .expect("validated evidence measurement");
    let observed_sha256 = object
        .get("observedSha256")
        .and_then(Value::as_str)
        .expect("validated observed SHA-256");
    validate_failed_measurement(identity, postcondition, measurement, observed_sha256)?;
    Ok(())
}

fn validate_failed_measurement(
    identity: &AttemptIdentity,
    postcondition: &EvidenceSpec,
    measurement: &serde_json::Map<String, Value>,
    observed_sha256: &str,
) -> anyhow::Result<()> {
    let frame_binding = match contracts(identity.action_kind).2 {
        EvidenceRequirement::Frame | EvidenceRequirement::TextOrQualifiedFrame => {
            let baseline = identity
                .evidence_baseline
                .as_ref()
                .and_then(Value::as_object)
                .context("failed frame evidence requires its committed baseline")?;
            validate_frame_baseline(baseline)?;
            let generation = baseline
                .get("generation")
                .and_then(Value::as_u64)
                .expect("validated frame generation");
            let sha256 = baseline
                .get("jpegSha256")
                .and_then(Value::as_str)
                .expect("validated frame SHA-256");
            ensure!(
                measurement.get("generation").and_then(Value::as_u64) == Some(generation)
                    && measurement.get("baselineSha256").and_then(Value::as_str) == Some(sha256),
                "failed frame evidence does not match its committed generation and baseline"
            );
            Some(sha256)
        }
        _ => None,
    };
    match postcondition {
        EvidenceSpec::ActiveAppEquals { bundle_id } => ensure!(
            measurement.len() == 1
                && measurement
                    .get("bundleId")
                    .and_then(Value::as_str)
                    .is_some_and(|observed| observed != bundle_id),
            "failed active-app evidence actually matches its compiled bundle"
        ),
        EvidenceSpec::ProcessAbsent { bundle_id } => {
            let baseline = identity
                .evidence_baseline
                .as_ref()
                .and_then(Value::as_object)
                .context("failed process evidence requires its committed baseline")?;
            validate_process_baseline(&identity.node, baseline)?;
            ensure!(
                measurement.len() == 3
                    && measurement.get("bundleId").and_then(Value::as_str)
                        == Some(bundle_id.as_str())
                    && measurement.get("running") == Some(&Value::Bool(true))
                    && measurement.get("oldPid") == baseline.get("pid"),
                "failed process evidence does not prove the process is still present"
            );
        }
        EvidenceSpec::FrameDigestChanged { minimum_distance } => {
            let distance = measurement
                .get("distance")
                .and_then(Value::as_u64)
                .context("failed frame-digest evidence has no numeric distance")?;
            ensure!(
                measurement.len() == 3
                    && (distance < u64::from(*minimum_distance)
                        || frame_binding.is_some_and(|baseline| observed_sha256 == baseline)),
                "failed frame-digest evidence actually meets its compiled threshold"
            );
        }
        EvidenceSpec::FrameRegionChanged {
            x,
            y,
            width,
            height,
            minimum_distance,
        } => {
            let identity_matches = measurement.len() == 7
                && measurement.get("x").and_then(Value::as_u64) == Some(u64::from(*x))
                && measurement.get("y").and_then(Value::as_u64) == Some(u64::from(*y))
                && measurement.get("width").and_then(Value::as_u64) == Some(u64::from(*width))
                && measurement.get("height").and_then(Value::as_u64) == Some(u64::from(*height));
            let distance = measurement
                .get("distance")
                .and_then(Value::as_u64)
                .context("failed frame-region evidence has no numeric distance")?;
            ensure!(
                identity_matches
                    && (distance < u64::from(*minimum_distance)
                        || frame_binding.is_some_and(|baseline| observed_sha256 == baseline)),
                "failed frame-region evidence actually meets its compiled threshold"
            );
        }
        EvidenceSpec::QualifiedFramePredicate { detector_id } => ensure!(
            measurement.len() == 3
                && frame_binding.is_some()
                && measurement.get("detectorId").and_then(Value::as_str)
                    == Some(detector_id.as_str()),
            "failed frame-predicate evidence does not match its compiled detector"
        ),
        EvidenceSpec::AccessibilityVisible { accessibility_id } => ensure!(
            measurement.len() == if frame_binding.is_some() { 4 } else { 2 }
                && measurement.get("accessibilityId").and_then(Value::as_str)
                    == Some(accessibility_id.as_str())
                && measurement.get("visible") == Some(&Value::Bool(false)),
            "failed accessibility evidence actually reports the target visible"
        ),
        EvidenceSpec::TextReadBackEquals { locator, value } => {
            let expected_locator = serde_json::to_value(locator)?;
            ensure!(
                measurement.len() == 4
                    && frame_binding.is_some()
                    && measurement.get("locator") == Some(&expected_locator)
                    && measurement
                        .get("value")
                        .and_then(Value::as_str)
                        .is_some_and(|observed| observed != value),
                "failed text evidence actually matches the compiled value"
            );
        }
        EvidenceSpec::ArtifactDecodedAndHashed => {
            bail!("artifact verification failure must use its typed error")
        }
    }
    Ok(())
}

fn validate_success_measurement(
    identity: &AttemptIdentity,
    postcondition: &EvidenceSpec,
    measurement: &serde_json::Map<String, Value>,
    observed_sha256: &str,
) -> anyhow::Result<()> {
    let frame_binding = match contracts(identity.action_kind).2 {
        EvidenceRequirement::Frame | EvidenceRequirement::TextOrQualifiedFrame => {
            let baseline = identity
                .evidence_baseline
                .as_ref()
                .and_then(Value::as_object)
                .context("frame evidence requires its committed baseline")?;
            validate_frame_baseline(baseline)?;
            let generation = baseline
                .get("generation")
                .and_then(Value::as_u64)
                .expect("validated frame generation");
            let sha256 = baseline
                .get("jpegSha256")
                .and_then(Value::as_str)
                .expect("validated frame SHA-256");
            ensure!(
                measurement.get("generation").and_then(Value::as_u64) == Some(generation)
                    && measurement.get("baselineSha256").and_then(Value::as_str) == Some(sha256),
                "frame evidence does not match its committed generation and baseline"
            );
            Some(sha256)
        }
        _ => None,
    };
    match postcondition {
        EvidenceSpec::ActiveAppEquals { bundle_id } => ensure!(
            measurement.len() == 1
                && measurement.get("bundleId").and_then(Value::as_str) == Some(bundle_id.as_str()),
            "active-app evidence does not match its compiled bundle"
        ),
        EvidenceSpec::ProcessAbsent { bundle_id } => {
            let baseline = identity
                .evidence_baseline
                .as_ref()
                .and_then(Value::as_object)
                .context("process evidence requires its committed baseline")?;
            validate_process_baseline(&identity.node, baseline)?;
            ensure!(
                measurement.len() == 3
                    && measurement.get("bundleId").and_then(Value::as_str)
                        == Some(bundle_id.as_str())
                    && measurement.get("running") == Some(&Value::Bool(false))
                    && measurement.get("oldPid") == baseline.get("pid"),
                "process evidence does not match the committed process baseline"
            );
        }
        EvidenceSpec::FrameDigestChanged { minimum_distance } => ensure!(
            measurement.len() == 3
                && measurement.get("distance").and_then(Value::as_u64)
                    >= Some(u64::from(*minimum_distance))
                && frame_binding.is_some_and(|baseline| observed_sha256 != baseline),
            "frame-digest evidence is below its compiled threshold"
        ),
        EvidenceSpec::FrameRegionChanged {
            x,
            y,
            width,
            height,
            minimum_distance,
        } => ensure!(
            measurement.len() == 7
                && measurement.get("x").and_then(Value::as_u64) == Some(u64::from(*x))
                && measurement.get("y").and_then(Value::as_u64) == Some(u64::from(*y))
                && measurement.get("width").and_then(Value::as_u64) == Some(u64::from(*width))
                && measurement.get("height").and_then(Value::as_u64) == Some(u64::from(*height))
                && measurement.get("distance").and_then(Value::as_u64)
                    >= Some(u64::from(*minimum_distance))
                && frame_binding.is_some_and(|baseline| observed_sha256 != baseline),
            "frame-region evidence does not match its compiled region"
        ),
        EvidenceSpec::QualifiedFramePredicate { detector_id } => ensure!(
            measurement.len() == 3
                && frame_binding.is_some()
                && measurement.get("detectorId").and_then(Value::as_str)
                    == Some(detector_id.as_str()),
            "frame-predicate evidence does not match its compiled detector"
        ),
        EvidenceSpec::AccessibilityVisible { accessibility_id } => ensure!(
            measurement.len() == if frame_binding.is_some() { 4 } else { 2 }
                && measurement.get("accessibilityId").and_then(Value::as_str)
                    == Some(accessibility_id.as_str())
                && measurement.get("visible") == Some(&Value::Bool(true)),
            "accessibility evidence does not match its compiled locator"
        ),
        EvidenceSpec::TextReadBackEquals { locator, value } => {
            let expected_locator = serde_json::to_value(locator)?;
            ensure!(
                measurement.len() == 4
                    && frame_binding.is_some()
                    && measurement.get("locator") == Some(&expected_locator)
                    && measurement.get("value").and_then(Value::as_str) == Some(value.as_str()),
                "text evidence does not match its compiled locator and value"
            );
        }
        EvidenceSpec::ArtifactDecodedAndHashed => {
            bail!("artifact evidence must be published through the atomic artifact transaction")
        }
    }
    Ok(())
}

fn evidence_kind(spec: &EvidenceSpec) -> EvidenceKind {
    match spec {
        EvidenceSpec::ActiveAppEquals { .. } => EvidenceKind::ActiveAppEquals,
        EvidenceSpec::ProcessAbsent { .. } => EvidenceKind::ProcessAbsent,
        EvidenceSpec::FrameDigestChanged { .. } => EvidenceKind::FrameDigestChanged,
        EvidenceSpec::FrameRegionChanged { .. } => EvidenceKind::FrameRegionChanged,
        EvidenceSpec::QualifiedFramePredicate { .. } => EvidenceKind::QualifiedFramePredicate,
        EvidenceSpec::AccessibilityVisible { .. } => EvidenceKind::AccessibilityVisible,
        EvidenceSpec::TextReadBackEquals { .. } => EvidenceKind::TextReadBackEquals,
        EvidenceSpec::ArtifactDecodedAndHashed => EvidenceKind::ArtifactDecodedAndHashed,
    }
}

fn validate_evidence_envelope(value: &Value) -> anyhow::Result<()> {
    let object = value
        .as_object()
        .context("evidence must be a typed object")?;
    ensure!(
        object.len() == 4,
        "evidence must use the exact result schema"
    );
    let kind = object
        .get("kind")
        .and_then(Value::as_str)
        .context("evidence kind is missing")?;
    parse_enum_name::<EvidenceKind>(kind, "evidence kind")?;
    ensure!(
        object.get("matched").is_some_and(Value::is_boolean),
        "evidence matched flag is missing"
    );
    ensure!(
        object
            .get("observedSha256")
            .and_then(Value::as_str)
            .is_some_and(is_lower_sha256),
        "evidence observed SHA-256 is invalid"
    );
    ensure!(
        object
            .get("measurement")
            .and_then(Value::as_object)
            .is_some_and(|measurement| !measurement.is_empty()),
        "evidence measurement is missing"
    );
    Ok(())
}

fn validate_retry_safety_contract(
    action_kind: crate::ActionKind,
    value: &Value,
) -> anyhow::Result<()> {
    let (_, _, requirement, _, retry_policy) = contracts(action_kind);
    ensure!(
        retry_policy == RetryPolicy::IdempotentAfterRead,
        "action contract does not permit reconciled retry"
    );
    validate_evidence_envelope(value)?;
    let object = value.as_object().expect("validated evidence object");
    let expected_kind = match requirement {
        EvidenceRequirement::ActiveApp => EvidenceKind::ActiveAppEquals,
        EvidenceRequirement::Process => EvidenceKind::ProcessAbsent,
        _ => bail!("action contract has no retry-safe read-back evidence"),
    };
    ensure!(
        object.get("kind").and_then(Value::as_str) == Some(enum_name(expected_kind)?.as_str()),
        "retry-safe evidence kind does not match the action contract"
    );
    ensure!(
        object.get("matched") == Some(&Value::Bool(false)),
        "retry-safe reconciliation must prove the desired effect was not applied"
    );
    Ok(())
}

fn validate_retry_safety_evidence(identity: &AttemptIdentity, value: &Value) -> anyhow::Result<()> {
    validate_retry_safety_contract(identity.action_kind, value)?;
    let object = value.as_object().expect("validated evidence object");
    let measurement = object
        .get("measurement")
        .and_then(Value::as_object)
        .expect("validated evidence measurement");
    match contracts(identity.action_kind).2 {
        EvidenceRequirement::Process => {
            let baseline = identity
                .evidence_baseline
                .as_ref()
                .and_then(Value::as_object)
                .context("retry-safe process proof requires its committed baseline")?;
            validate_process_baseline(&identity.node, baseline)?;
            let bundle_id = baseline
                .get("bundleId")
                .and_then(Value::as_str)
                .expect("validated process bundle");
            let baseline_pid = baseline
                .get("pid")
                .and_then(Value::as_u64)
                .context("an already-absent process has no retryable effect")?;
            ensure!(
                measurement.len() == 3
                    && measurement.get("bundleId").and_then(Value::as_str) == Some(bundle_id)
                    && measurement.get("pid").and_then(Value::as_u64) == Some(baseline_pid)
                    && measurement.get("preEffectPid").and_then(Value::as_u64)
                        == Some(baseline_pid),
                "retry-safe process proof does not match its committed baseline"
            );
        }
        EvidenceRequirement::ActiveApp => {
            let Some(EvidenceSpec::ActiveAppEquals { bundle_id }) = &identity.node.postcondition
            else {
                bail!("retry-safe active-app proof requires a compiled postcondition");
            };
            let observed = measurement
                .get("observedBundleId")
                .and_then(Value::as_str)
                .context("retry-safe active-app proof has no observed bundle")?;
            ensure!(
                measurement.len() == 2
                    && measurement.get("expectedBundleId").and_then(Value::as_str)
                        == Some(bundle_id.as_str())
                    && observed != bundle_id,
                "retry-safe active-app proof does not match its compiled target"
            );
        }
        _ => bail!("action contract has no retry-safe evidence binding"),
    }
    Ok(())
}

fn is_transport_non_delivery_proof(value: &Value) -> bool {
    let Some(object) = value.as_object() else {
        return false;
    };
    object.len() == 2
        && object.get("kind") == Some(&Value::String("transportNonDelivery".to_string()))
        && object.get("requestReachedDevice") == Some(&Value::Bool(false))
}

fn aggregate_projection(
    devices: &[(FlowDeviceRunState, Option<FlowErrorRecord>)],
) -> (FlowAggregateState, Option<FlowErrorRecord>) {
    if devices.is_empty() {
        return no_eligible_projection();
    }
    if devices.iter().any(|(state, _)| !state.is_terminal()) {
        let state = if devices
            .iter()
            .all(|(state, _)| *state == FlowDeviceRunState::Queued)
        {
            FlowAggregateState::Queued
        } else {
            FlowAggregateState::Running
        };
        return (state, None);
    }

    let non_skipped = devices
        .iter()
        .filter(|(state, _)| *state != FlowDeviceRunState::Skipped)
        .collect::<Vec<_>>();
    if non_skipped.is_empty() {
        return no_eligible_projection();
    }
    let succeeded = non_skipped
        .iter()
        .filter(|(state, _)| state.is_success())
        .count();
    let failed_or_cancelled = non_skipped.iter().any(|(state, _)| {
        matches!(
            state,
            FlowDeviceRunState::Failed | FlowDeviceRunState::Cancelled
        )
    });
    if succeeded > 0 && succeeded == non_skipped.len() {
        return (FlowAggregateState::Succeeded, None);
    }
    let first_error = non_skipped
        .iter()
        .find_map(|(_, error)| error.as_ref().cloned());
    if succeeded > 0 && failed_or_cancelled {
        return (FlowAggregateState::Partial, first_error);
    }
    if non_skipped
        .iter()
        .all(|(state, _)| *state == FlowDeviceRunState::Cancelled)
    {
        return (FlowAggregateState::Cancelled, first_error);
    }
    (FlowAggregateState::Failed, first_error)
}

fn no_eligible_projection() -> (FlowAggregateState, Option<FlowErrorRecord>) {
    (
        FlowAggregateState::Failed,
        Some(FlowErrorRecord {
            code: "NoEligibleDevice".to_string(),
            message: "the Flow run has no eligible device".to_string(),
            node_id: None,
            field: None,
            udid: None,
            attempt_id: None,
        }),
    )
}

fn append_flow_event(
    transaction: &Transaction<'_>,
    run_id: Uuid,
    kind: &str,
    payload: &Value,
    now: &str,
) -> anyhow::Result<u64> {
    ensure!(!kind.trim().is_empty(), "Flow event kind cannot be empty");
    let current_revision: i64 = transaction.query_row(
        "SELECT event_revision FROM flow_runs WHERE id=?1",
        [run_id.to_string()],
        |row| row.get(0),
    )?;
    validate_event_sequence(transaction, run_id, current_revision)?;
    current_revision
        .checked_add(1)
        .context("Flow event revision overflow")?;
    let changed = transaction.execute(
        "UPDATE flow_runs SET event_revision=event_revision+1,updated_at=?2 WHERE id=?1",
        params![run_id.to_string(), now],
    )?;
    ensure!(changed == 1, "Flow event parent run does not exist");
    let revision_sql: i64 = transaction.query_row(
        "SELECT event_revision FROM flow_runs WHERE id=?1",
        [run_id.to_string()],
        |row| row.get(0),
    )?;
    let revision = sql_to_u64(revision_sql, "Flow event revision")?;
    transaction.execute(
        "INSERT INTO flow_events(run_id,revision,kind,payload_json,created_at)
         VALUES(?1,?2,?3,?4,?5)",
        params![
            run_id.to_string(),
            revision_sql,
            kind,
            serde_json::to_string(payload)?,
            now,
        ],
    )?;
    Ok(revision)
}

fn validate_revision_for_run(revision: &FlowRevisionRecord) -> anyhow::Result<()> {
    ensure!(
        revision.document.schema_version == FLOW_SCHEMA_VERSION
            && revision.compiled_plan.schema_version == FLOW_SCHEMA_VERSION,
        "Flow run schema version mismatch"
    );
    ensure!(
        revision.document.id == revision.compiled_plan.flow_id
            && revision.document.revision == revision.compiled_plan.revision,
        "Flow run revision identity mismatch"
    );
    ensure!(
        is_lower_sha256(&revision.plan_hash),
        "Flow run plan hash is invalid"
    );
    ensure!(
        compiled_plan_sha256(&revision.compiled_plan)? == revision.plan_hash,
        "Flow run plan hash mismatch"
    );
    Ok(())
}

fn validate_persisted_plan(
    plan: &CompiledFlowPlanV2,
    compiled_json: &str,
    flow_id: &str,
    revision: i64,
    plan_sha256: &str,
    revision_plan_sha256: &str,
) -> anyhow::Result<()> {
    ensure!(
        plan.schema_version == FLOW_SCHEMA_VERSION,
        "persisted Flow plan schema version is invalid"
    );
    ensure!(
        plan.flow_id == parse_uuid(flow_id, "Flow plan flow ID")?
            && plan.revision == sql_to_u64(revision, "Flow plan revision")?,
        "persisted Flow plan identity mismatch"
    );
    ensure!(
        plan_sha256 == revision_plan_sha256
            && is_lower_sha256(plan_sha256)
            && compiled_plan_sha256(plan)? == plan_sha256,
        "persisted Flow plan hash mismatch"
    );
    ensure!(
        canonical_compiled_plan_json(plan)?.as_bytes() == compiled_json.as_bytes(),
        "persisted Flow plan JSON is not canonical"
    );
    Ok(())
}

fn validate_selection(selection: &FlowSelectionSnapshot) -> anyhow::Result<()> {
    let validate_values = |values: &[String], field: &str| -> anyhow::Result<()> {
        let mut unique = BTreeSet::new();
        for value in values {
            validate_udid(value).with_context(|| format!("validate {field}"))?;
            ensure!(unique.insert(value), "{field} contains duplicate UDIDs");
        }
        Ok(())
    };
    validate_values(&selection.target_udids, "targetUdids")?;
    ensure!(
        selection
            .target_udids
            .windows(2)
            .all(|pair| pair[0] < pair[1]),
        "targetUdids must be in canonical lexical order"
    );
    match &selection.requested {
        FlowTargetSelection::One { udid } => {
            validate_udid(udid)?;
            ensure!(
                selection.target_udids.as_slice() == [udid.as_str()],
                "One selection snapshot must contain the exact requested UDID"
            );
        }
        FlowTargetSelection::Selected { udids } => {
            ensure!(!udids.is_empty(), "Selected requires at least one UDID");
            validate_values(udids, "selected.udids")?;
            let requested = udids.iter().collect::<BTreeSet<_>>();
            let targets = selection.target_udids.iter().collect::<BTreeSet<_>>();
            ensure!(
                requested == targets,
                "Selected snapshot must resolve the exact requested UDIDs"
            );
        }
        FlowTargetSelection::AllEligible => {}
    }
    Ok(())
}

fn validate_udid(udid: &str) -> anyhow::Result<()> {
    ensure!(
        !udid.is_empty()
            && udid.trim() == udid
            && udid.len() <= 256
            && !udid.chars().any(char::is_control),
        "invalid device UDID"
    );
    Ok(())
}

fn validate_attempt_error_identity(
    attempt_id: Uuid,
    error: Option<&FlowErrorRecord>,
) -> anyhow::Result<()> {
    if let Some(error) = error {
        validate_error(error)?;
        if let Some(recorded_attempt_id) = error.attempt_id {
            ensure!(
                recorded_attempt_id == attempt_id,
                "Flow error attempt ID mismatch"
            );
        }
    }
    Ok(())
}

fn validate_error(error: &FlowErrorRecord) -> anyhow::Result<()> {
    ensure!(
        !error.code.trim().is_empty() && error.code.len() <= 128,
        "Flow error code is invalid"
    );
    ensure!(
        !error.message.trim().is_empty() && error.message.len() <= 4096,
        "Flow error message is invalid"
    );
    if let Some(field) = &error.field {
        ensure!(
            !field.trim().is_empty() && field.len() <= 256,
            "Flow error field is invalid"
        );
    }
    if let Some(udid) = &error.udid {
        validate_udid(udid)?;
    }
    Ok(())
}

fn validate_attempt_error_context(
    identity: &AttemptIdentity,
    error: Option<&FlowErrorRecord>,
) -> anyhow::Result<()> {
    if let Some(error) = error {
        if let Some(node_id) = error.node_id {
            ensure!(node_id == identity.node.id, "Flow error node ID mismatch");
        }
        if let Some(udid) = &error.udid {
            ensure!(udid == &identity.udid, "Flow error UDID mismatch");
        }
    }
    Ok(())
}

fn state_conflict(
    entity_id: Uuid,
    expected: FlowAttemptState,
    actual: FlowAttemptState,
    requested: FlowAttemptState,
) -> FlowStateConflict {
    FlowStateConflict {
        entity_id,
        expected: enum_name(expected).unwrap_or_else(|_| format!("{expected:?}")),
        actual: enum_name(actual).unwrap_or_else(|_| format!("{actual:?}")),
        requested: enum_name(requested).unwrap_or_else(|_| format!("{requested:?}")),
    }
}

struct AttemptIdentity {
    device_run_id: Uuid,
    run_id: Uuid,
    udid: String,
    device_state: FlowDeviceRunState,
    capability_snapshot: Option<FlowCapabilitySnapshot>,
    state: FlowAttemptState,
    attempt_no: u32,
    action_kind: crate::ActionKind,
    node: CompiledFlowNode,
    plan: CompiledFlowPlanV2,
    side_effect_class: SideEffectClass,
    canonical_input: Option<Value>,
    evidence_baseline: Option<Value>,
}

struct AttemptCreationRow {
    run_id: String,
    compiled_plan_json: String,
    device_state: String,
    run_state: String,
    flow_id: String,
    flow_revision: i64,
    plan_sha256: String,
    revision_plan_sha256: String,
}

struct AttemptIdentityRow {
    device_run_id: String,
    run_id: String,
    device_state: String,
    capability_snapshot_json: Option<String>,
    state: String,
    attempt_no: i64,
    action_kind: String,
    side_effect_class: String,
    canonical_input_json: Option<String>,
    evidence_baseline_json: Option<String>,
    node_id: String,
    compiled_plan_json: String,
    flow_id: String,
    flow_revision: i64,
    plan_sha256: String,
    revision_plan_sha256: String,
    udid: String,
}

fn query_attempt_identity(
    connection: &Connection,
    attempt_id: Uuid,
) -> anyhow::Result<Option<AttemptIdentity>> {
    let row: Option<AttemptIdentityRow> = connection
        .query_row(
            "SELECT a.device_run_id,d.run_id,d.state,d.capability_snapshot_json,
                    a.state,a.attempt_no,a.action_kind,a.side_effect_class,
                    a.canonical_input_json,a.evidence_baseline_json,a.node_id,r.compiled_json,
                    f.flow_id,f.flow_revision,f.plan_sha256,r.plan_sha256,d.udid
             FROM flow_node_attempts a
             JOIN flow_device_runs d ON d.id=a.device_run_id
             JOIN flow_runs f ON f.id=d.run_id
             JOIN flow_revisions r ON r.flow_id=f.flow_id AND r.revision=f.flow_revision
             WHERE a.id=?1",
            [attempt_id.to_string()],
            |row| {
                Ok(AttemptIdentityRow {
                    device_run_id: row.get(0)?,
                    run_id: row.get(1)?,
                    device_state: row.get(2)?,
                    capability_snapshot_json: row.get(3)?,
                    state: row.get(4)?,
                    attempt_no: row.get(5)?,
                    action_kind: row.get(6)?,
                    side_effect_class: row.get(7)?,
                    canonical_input_json: row.get(8)?,
                    evidence_baseline_json: row.get(9)?,
                    node_id: row.get(10)?,
                    compiled_plan_json: row.get(11)?,
                    flow_id: row.get(12)?,
                    flow_revision: row.get(13)?,
                    plan_sha256: row.get(14)?,
                    revision_plan_sha256: row.get(15)?,
                    udid: row.get(16)?,
                })
            },
        )
        .optional()?;
    row.map(|row| {
        let action_kind = parse_enum_name(&row.action_kind, "Flow action kind")?;
        let side_effect_class = parse_enum_name(&row.side_effect_class, "Flow side-effect class")?;
        let node_id = parse_uuid(&row.node_id, "Flow attempt node ID")?;
        let plan: CompiledFlowPlanV2 = serde_json::from_str(&row.compiled_plan_json)
            .context("parse persisted Flow attempt plan")?;
        validate_persisted_plan(
            &plan,
            &row.compiled_plan_json,
            &row.flow_id,
            row.flow_revision,
            &row.plan_sha256,
            &row.revision_plan_sha256,
        )?;
        let node = plan
            .nodes
            .get(&node_id)
            .cloned()
            .context("persisted Flow attempt node is absent from its plan")?;
        ensure!(
            node.kind == action_kind && contracts(action_kind).1 == side_effect_class,
            "persisted Flow attempt contract does not match its compiled node"
        );
        let state = parse_enum_name(&row.state, "Flow attempt state")?;
        let canonical_input = decode_optional_json(row.canonical_input_json, "canonical input")?;
        let evidence_baseline =
            decode_optional_json(row.evidence_baseline_json, "evidence baseline")?;
        ensure!(
            canonical_input.is_some() == evidence_baseline.is_some(),
            "persisted Flow intent input and baseline are incomplete"
        );
        ensure!(
            !state_requires_committed_intent(state) || canonical_input.is_some(),
            "persisted Flow attempt crossed intent without its durable boundary"
        );
        let identity = AttemptIdentity {
            device_run_id: parse_uuid(&row.device_run_id, "Flow attempt device-run ID")?,
            run_id: parse_uuid(&row.run_id, "Flow attempt run ID")?,
            udid: {
                validate_udid(&row.udid)?;
                row.udid
            },
            device_state: parse_enum_name(&row.device_state, "Flow device state")?,
            capability_snapshot: decode_optional_json(
                row.capability_snapshot_json,
                "Flow capability snapshot",
            )?,
            state,
            attempt_no: u32::try_from(row.attempt_no).context("Flow attempt number is invalid")?,
            action_kind,
            node,
            plan,
            side_effect_class,
            canonical_input,
            evidence_baseline,
        };
        if let Some(baseline) = &identity.evidence_baseline {
            validate_evidence_baseline(&identity, baseline)?;
        }
        Ok(identity)
    })
    .transpose()
}

fn validate_attempt_claim(
    transaction: &Transaction<'_>,
    attempt_id: Uuid,
    identity: &AttemptIdentity,
) -> anyhow::Result<()> {
    let latest_id: String = transaction.query_row(
        "SELECT id FROM flow_node_attempts
         WHERE device_run_id=?1 AND node_id=?2
         ORDER BY attempt_no DESC LIMIT 1",
        params![
            identity.device_run_id.to_string(),
            identity.node.id.to_string()
        ],
        |row| row.get(0),
    )?;
    ensure!(
        latest_id == attempt_id.to_string(),
        "StateConflict: only the latest Flow attempt may claim intent"
    );
    let node_index = identity
        .plan
        .execution_order
        .iter()
        .position(|node_id| *node_id == identity.node.id)
        .context("Flow attempt node is absent from execution order")?;
    for predecessor_id in &identity.plan.execution_order[..node_index] {
        let predecessor_state: String = transaction.query_row(
            "SELECT state FROM flow_node_attempts
             WHERE device_run_id=?1 AND node_id=?2
             ORDER BY attempt_no DESC LIMIT 1",
            params![
                identity.device_run_id.to_string(),
                predecessor_id.to_string()
            ],
            |row| row.get(0),
        )?;
        ensure!(
            parse_enum_name::<FlowAttemptState>(&predecessor_state, "predecessor state")?
                == FlowAttemptState::Succeeded,
            "StateConflict: Flow attempt predecessor is not succeeded"
        );
    }
    let uncertain: i64 = transaction.query_row(
        "SELECT COUNT(*) FROM flow_node_attempts
         WHERE device_run_id=?1 AND state='uncertain'",
        [identity.device_run_id.to_string()],
        |row| row.get(0),
    )?;
    ensure!(
        uncertain == 0,
        "StateConflict: uncertain Flow attempt blocks new intent"
    );
    ensure!(
        identity.attempt_no >= 1,
        "StateConflict: Flow attempt number is invalid"
    );
    Ok(())
}

fn query_run_record(connection: &Connection, id: Uuid) -> anyhow::Result<Option<FlowRunRecord>> {
    connection
        .query_row(
            "SELECT id,flow_id,flow_revision,plan_sha256,selection_json,state,event_revision,
                    error_json,created_at,updated_at
             FROM flow_runs WHERE id=?1",
            [id.to_string()],
            run_row,
        )
        .optional()?
        .map(FlowRunRow::into_record)
        .transpose()
}

fn validate_run_plan(connection: &Connection, run: &FlowRunRecord) -> anyhow::Result<()> {
    load_validated_run_plan(connection, run).map(|_| ())
}

fn load_validated_run_plan(
    connection: &Connection,
    run: &FlowRunRecord,
) -> anyhow::Result<CompiledFlowPlanV2> {
    let (compiled_json, revision_plan_sha256): (String, String) = connection.query_row(
        "SELECT compiled_json,plan_sha256 FROM flow_revisions
         WHERE flow_id=?1 AND revision=?2",
        params![
            run.flow_id.to_string(),
            u64_to_sql(run.flow_revision, "Flow run revision")?
        ],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let plan: CompiledFlowPlanV2 =
        serde_json::from_str(&compiled_json).context("parse persisted Flow run plan")?;
    validate_persisted_plan(
        &plan,
        &compiled_json,
        &run.flow_id.to_string(),
        u64_to_sql(run.flow_revision, "Flow run revision")?,
        &run.plan_sha256,
        &revision_plan_sha256,
    )?;
    Ok(plan)
}

fn validate_device_success_projection(
    connection: &Connection,
    device: &FlowDeviceRunRecord,
) -> anyhow::Result<()> {
    let run = query_run_record(connection, device.run_id)?
        .context("Flow device parent run does not exist")?;
    let plan = load_validated_run_plan(connection, &run)?;
    for node_id in &plan.execution_order {
        let latest_state: Option<String> = connection
            .query_row(
                "SELECT state FROM flow_node_attempts
                 WHERE device_run_id=?1 AND node_id=?2
                 ORDER BY attempt_no DESC LIMIT 1",
                params![device.id.to_string(), node_id.to_string()],
                |row| row.get(0),
            )
            .optional()?;
        ensure!(
            latest_state
                .as_deref()
                .map(|state| parse_enum_name::<FlowAttemptState>(state, "Flow attempt state"))
                .transpose()?
                == Some(FlowAttemptState::Succeeded),
            "StateConflict: device success requires every compiled node to succeed"
        );
    }
    Ok(())
}

fn validate_event_ledger(connection: &Connection, run: &FlowRunRecord) -> anyhow::Result<()> {
    let expected = u64_to_sql(run.event_revision, "Flow event revision")?;
    validate_event_sequence(connection, run.id, expected)?;
    let mut statement = connection.prepare(
        "SELECT kind,payload_json,created_at FROM flow_events
         WHERE run_id=?1 ORDER BY revision ASC",
    )?;
    let rows = statement.query_map([run.id.to_string()], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
        ))
    })?;
    for row in rows {
        let (kind, payload_json, created_at) = row?;
        ensure!(
            !kind.trim().is_empty(),
            "persisted Flow event kind is empty"
        );
        serde_json::from_str::<Value>(&payload_json)
            .context("parse persisted Flow event payload")?;
        parse_datetime(&created_at, "Flow event createdAt")?;
    }
    Ok(())
}

fn validate_event_sequence(
    connection: &Connection,
    run_id: Uuid,
    expected: i64,
) -> anyhow::Result<()> {
    ensure!(expected >= 0, "persisted Flow event revision is negative");
    let (count, minimum, maximum): (i64, Option<i64>, Option<i64>) = connection.query_row(
        "SELECT COUNT(*),MIN(revision),MAX(revision) FROM flow_events WHERE run_id=?1",
        [run_id.to_string()],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    ensure!(
        count == expected
            && ((expected == 0 && minimum.is_none() && maximum.is_none())
                || (minimum == Some(1) && maximum == Some(expected))),
        "persisted Flow event ledger is not contiguous"
    );
    Ok(())
}

fn query_device_run_record(
    connection: &Connection,
    id: Uuid,
) -> anyhow::Result<Option<FlowDeviceRunRecord>> {
    connection
        .query_row(
            "SELECT id,run_id,udid,state,capability_snapshot_json,release_proof_json,
                    error_json,started_at,finished_at
             FROM flow_device_runs WHERE id=?1",
            [id.to_string()],
            device_run_row,
        )
        .optional()?
        .map(FlowDeviceRunRow::into_record)
        .transpose()
}

fn query_attempt_record(
    connection: &Connection,
    id: Uuid,
) -> anyhow::Result<Option<FlowNodeAttemptRecord>> {
    let record = connection
        .query_row(
            "SELECT id,device_run_id,node_id,action_kind,attempt_no,side_effect_class,state,
                    canonical_input_json,evidence_baseline_json,evidence_result_json,retry_safe,
                    error_json,started_at,updated_at,finished_at
             FROM flow_node_attempts WHERE id=?1",
            [id.to_string()],
            attempt_row,
        )
        .optional()?
        .map(FlowAttemptRow::into_record)
        .transpose()?;
    if let Some(record) = &record {
        validate_persisted_attempt(connection, record)?;
    }
    Ok(record)
}

fn validate_persisted_attempt(
    connection: &Connection,
    attempt: &FlowNodeAttemptRecord,
) -> anyhow::Result<()> {
    let identity = query_attempt_identity(connection, attempt.id)?
        .context("persisted Flow attempt disappeared")?;
    ensure!(
        !identity.device_state.is_terminal() || !state_owns_active_context(attempt.state),
        "persisted active Flow attempt belongs to a terminal device"
    );
    ensure!(
        identity.node.id == attempt.node_id
            && identity.action_kind == attempt.action_kind
            && identity.side_effect_class == attempt.side_effect_class,
        "persisted Flow attempt does not match its pinned compiled node"
    );
    validate_attempt_error_context(&identity, attempt.error.as_ref())?;
    if attempt.state == FlowAttemptState::Succeeded {
        ensure!(
            attempt.error.is_none(),
            "persisted successful Flow attempt carries an error"
        );
        if identity.side_effect_class == SideEffectClass::ArtifactWrite {
            let evidence = attempt
                .evidence_result
                .as_ref()
                .context("persisted artifact success has no evidence")?;
            validate_evidence_envelope(evidence)?;
            let object = evidence.as_object().expect("validated evidence object");
            ensure!(
                object.get("kind").and_then(Value::as_str) == Some("artifactDecodedAndHashed")
                    && object.get("matched") == Some(&Value::Bool(true)),
                "persisted artifact success evidence is invalid"
            );
        } else if let Some(postcondition) = &identity.node.postcondition {
            validate_success_evidence(
                &identity,
                postcondition,
                attempt
                    .evidence_result
                    .as_ref()
                    .context("persisted Flow success has no evidence")?,
            )?;
        } else {
            ensure!(
                contracts(identity.action_kind).2 == EvidenceRequirement::None
                    && attempt.evidence_result.is_none(),
                "persisted evidence-free Flow success is invalid"
            );
        }
    }
    if attempt.state == FlowAttemptState::FailedVerified {
        ensure!(
            attempt.error.as_ref().and_then(|error| error.attempt_id) == Some(attempt.id),
            "persisted failedVerified attempt has no typed bound verifier error"
        );
        if attempt.retry_allowed {
            validate_retry_safety_evidence(
                &identity,
                attempt
                    .evidence_result
                    .as_ref()
                    .context("retry-safe Flow attempt has no evidence")?,
            )?;
        } else if let Some(evidence) = &attempt.evidence_result {
            validate_failed_verification_evidence(&identity, evidence)?;
        }
    }
    Ok(())
}

struct FlowRunRow {
    id: String,
    flow_id: String,
    flow_revision: i64,
    plan_sha256: String,
    selection_json: String,
    state: String,
    event_revision: i64,
    error_json: Option<String>,
    created_at: String,
    updated_at: String,
}

fn run_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<FlowRunRow> {
    Ok(FlowRunRow {
        id: row.get(0)?,
        flow_id: row.get(1)?,
        flow_revision: row.get(2)?,
        plan_sha256: row.get(3)?,
        selection_json: row.get(4)?,
        state: row.get(5)?,
        event_revision: row.get(6)?,
        error_json: row.get(7)?,
        created_at: row.get(8)?,
        updated_at: row.get(9)?,
    })
}

impl FlowRunRow {
    fn into_record(self) -> anyhow::Result<FlowRunRecord> {
        ensure!(
            is_lower_sha256(&self.plan_sha256),
            "persisted Flow run hash is invalid"
        );
        let selection: FlowSelectionSnapshot =
            serde_json::from_str(&self.selection_json).context("parse persisted Flow selection")?;
        validate_selection(&selection)?;
        let error = decode_optional_json::<FlowErrorRecord>(self.error_json, "Flow run error")?;
        if let Some(error) = &error {
            validate_error(error)?;
        }
        Ok(FlowRunRecord {
            id: parse_uuid(&self.id, "Flow run ID")?,
            flow_id: parse_uuid(&self.flow_id, "Flow run flow ID")?,
            flow_revision: sql_to_u64(self.flow_revision, "Flow run revision")?,
            plan_sha256: self.plan_sha256,
            selection,
            state: parse_enum_name(&self.state, "Flow aggregate state")?,
            event_revision: sql_to_u64(self.event_revision, "Flow event revision")?,
            error,
            created_at: parse_datetime(&self.created_at, "Flow run createdAt")?,
            updated_at: parse_datetime(&self.updated_at, "Flow run updatedAt")?,
        })
    }
}

struct FlowDeviceRunRow {
    id: String,
    run_id: String,
    udid: String,
    state: String,
    capability_snapshot_json: Option<String>,
    release_proof_json: Option<String>,
    error_json: Option<String>,
    started_at: Option<String>,
    finished_at: Option<String>,
}

fn device_run_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<FlowDeviceRunRow> {
    Ok(FlowDeviceRunRow {
        id: row.get(0)?,
        run_id: row.get(1)?,
        udid: row.get(2)?,
        state: row.get(3)?,
        capability_snapshot_json: row.get(4)?,
        release_proof_json: row.get(5)?,
        error_json: row.get(6)?,
        started_at: row.get(7)?,
        finished_at: row.get(8)?,
    })
}

impl FlowDeviceRunRow {
    fn into_record(self) -> anyhow::Result<FlowDeviceRunRecord> {
        validate_udid(&self.udid)?;
        let state: FlowDeviceRunState = parse_enum_name(&self.state, "Flow device-run state")?;
        let capability_snapshot = decode_optional_json::<FlowCapabilitySnapshot>(
            self.capability_snapshot_json,
            "Flow capability snapshot",
        )?;
        if let Some(snapshot) = capability_snapshot.as_ref() {
            snapshot.validate().map_err(anyhow::Error::msg)?;
            if let Some(agent_status) = snapshot.agent_status.as_ref() {
                ensure!(
                    agent_status.udid == self.udid,
                    "persisted Flow AgentStatus UDID mismatch"
                );
            }
        }
        ensure!(
            match state {
                FlowDeviceRunState::Queued | FlowDeviceRunState::Preflight => {
                    capability_snapshot.is_none()
                }
                FlowDeviceRunState::Running => capability_snapshot.is_some(),
                FlowDeviceRunState::Succeeded => capability_snapshot.is_some(),
                FlowDeviceRunState::Failed
                | FlowDeviceRunState::Skipped
                | FlowDeviceRunState::Cancelled => true,
            },
            "persisted Flow capability snapshot does not match device state"
        );
        let release_proof: Option<FlowContextReleaseProof> =
            decode_optional_json(self.release_proof_json, "Flow release proof")?;
        if let Some(proof) = &release_proof {
            ensure!(
                proof.udid == self.udid,
                "persisted Flow release proof UDID mismatch"
            );
            ensure!(
                proof.owner == DeviceWorkOwner::Script,
                "persisted Flow release proof owner is invalid"
            );
            ensure!(
                !proof.had_stream || proof.had_session,
                "persisted Flow release proof has a stream without a session"
            );
        }
        ensure!(
            state.is_terminal() == release_proof.is_some(),
            "persisted Flow release proof does not match device terminality"
        );
        let error =
            decode_optional_json::<FlowErrorRecord>(self.error_json, "Flow device-run error")?;
        if let Some(error) = &error {
            validate_error(error)?;
            if let Some(udid) = &error.udid {
                ensure!(
                    udid == &self.udid,
                    "persisted Flow device error UDID mismatch"
                );
            }
        }
        let started_at = parse_optional_datetime(self.started_at, "Flow device-run startedAt")?;
        let finished_at = parse_optional_datetime(self.finished_at, "Flow device-run finishedAt")?;
        ensure!(
            match state {
                FlowDeviceRunState::Queued => started_at.is_none() && finished_at.is_none(),
                FlowDeviceRunState::Preflight | FlowDeviceRunState::Running => {
                    started_at.is_some() && finished_at.is_none()
                }
                FlowDeviceRunState::Succeeded
                | FlowDeviceRunState::Failed
                | FlowDeviceRunState::Skipped
                | FlowDeviceRunState::Cancelled => {
                    started_at.is_some() && finished_at.is_some()
                }
            },
            "persisted Flow device timestamps do not match its state"
        );
        ensure!(
            state != FlowDeviceRunState::Succeeded || error.is_none(),
            "persisted successful Flow device carries an error"
        );
        Ok(FlowDeviceRunRecord {
            id: parse_uuid(&self.id, "Flow device-run ID")?,
            run_id: parse_uuid(&self.run_id, "Flow device-run parent ID")?,
            udid: self.udid,
            state,
            capability_snapshot,
            release_proof,
            error,
            started_at,
            finished_at,
        })
    }
}

struct FlowAttemptRow {
    id: String,
    device_run_id: String,
    node_id: String,
    action_kind: String,
    attempt_no: i64,
    side_effect_class: String,
    state: String,
    canonical_input_json: Option<String>,
    evidence_baseline_json: Option<String>,
    evidence_result_json: Option<String>,
    retry_safe: i64,
    error_json: Option<String>,
    started_at: Option<String>,
    updated_at: String,
    finished_at: Option<String>,
}

fn attempt_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<FlowAttemptRow> {
    Ok(FlowAttemptRow {
        id: row.get(0)?,
        device_run_id: row.get(1)?,
        node_id: row.get(2)?,
        action_kind: row.get(3)?,
        attempt_no: row.get(4)?,
        side_effect_class: row.get(5)?,
        state: row.get(6)?,
        canonical_input_json: row.get(7)?,
        evidence_baseline_json: row.get(8)?,
        evidence_result_json: row.get(9)?,
        retry_safe: row.get(10)?,
        error_json: row.get(11)?,
        started_at: row.get(12)?,
        updated_at: row.get(13)?,
        finished_at: row.get(14)?,
    })
}

impl FlowAttemptRow {
    fn into_record(self) -> anyhow::Result<FlowNodeAttemptRecord> {
        ensure!(
            matches!(self.retry_safe, 0 | 1),
            "invalid Flow retry-safe flag"
        );
        ensure!(self.attempt_no >= 1, "invalid Flow attempt number");
        let id = parse_uuid(&self.id, "Flow attempt ID")?;
        let state = parse_enum_name(&self.state, "Flow attempt state")?;
        let action_kind = parse_enum_name(&self.action_kind, "Flow action kind")?;
        let side_effect_class = parse_enum_name(&self.side_effect_class, "Flow side-effect class")?;
        let canonical_input = decode_optional_json(self.canonical_input_json, "canonical input")?;
        let evidence_baseline =
            decode_optional_json(self.evidence_baseline_json, "evidence baseline")?;
        let started_at = parse_optional_datetime(self.started_at, "Flow attempt startedAt")?;
        let finished_at = parse_optional_datetime(self.finished_at, "Flow attempt finishedAt")?;
        ensure!(
            canonical_input.is_some() == evidence_baseline.is_some(),
            "persisted Flow intent input and baseline are incomplete"
        );
        ensure!(
            !state_requires_committed_intent(state)
                || (canonical_input.is_some() && started_at.is_some()),
            "persisted Flow attempt crossed intent without its durable boundary"
        );
        ensure!(
            canonical_input.is_none() || started_at.is_some(),
            "persisted Flow committed intent has no start timestamp"
        );
        ensure!(
            state.is_terminal() == finished_at.is_some(),
            "persisted Flow attempt finish timestamp does not match terminality"
        );
        ensure!(
            contracts(action_kind).1 == side_effect_class,
            "persisted Flow side-effect class does not match its action"
        );
        ensure!(
            self.retry_safe == 0
                || (state == FlowAttemptState::FailedVerified
                    && side_effect_class == SideEffectClass::IdempotentSet),
            "persisted retry-safe proof is invalid"
        );
        let evidence_result = decode_optional_json(self.evidence_result_json, "evidence result")?;
        if self.retry_safe == 1 {
            validate_retry_safety_contract(
                action_kind,
                evidence_result
                    .as_ref()
                    .context("persisted retry-safe attempt has no evidence")?,
            )?;
        }
        let error = decode_optional_json::<FlowErrorRecord>(self.error_json, "Flow attempt error")?;
        validate_attempt_error_identity(id, error.as_ref())?;
        let retry_allowed = state == FlowAttemptState::FailedBeforeDispatch
            || (state == FlowAttemptState::FailedVerified
                && side_effect_class == SideEffectClass::IdempotentSet
                && self.retry_safe == 1);
        Ok(FlowNodeAttemptRecord {
            id,
            device_run_id: parse_uuid(&self.device_run_id, "Flow attempt device-run ID")?,
            node_id: parse_uuid(&self.node_id, "Flow attempt node ID")?,
            action_kind,
            attempt_no: u32::try_from(self.attempt_no).context("invalid Flow attempt number")?,
            side_effect_class,
            state,
            canonical_input,
            evidence_baseline,
            evidence_result,
            retry_allowed,
            error,
            started_at,
            updated_at: parse_datetime(&self.updated_at, "Flow attempt updatedAt")?,
            finished_at,
        })
    }
}

struct FlowArtifactRow {
    id: String,
    attempt_id: String,
    relative_path: String,
    label: String,
    kind: String,
    size: i64,
    sha256: String,
    created_at: String,
}

fn artifact_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<FlowArtifactRow> {
    Ok(FlowArtifactRow {
        id: row.get(0)?,
        attempt_id: row.get(1)?,
        relative_path: row.get(2)?,
        label: row.get(3)?,
        kind: row.get(4)?,
        size: row.get(5)?,
        sha256: row.get(6)?,
        created_at: row.get(7)?,
    })
}

impl FlowArtifactRow {
    fn into_record(self) -> anyhow::Result<FlowArtifactRecord> {
        ensure!(
            is_lower_sha256(&self.sha256),
            "persisted artifact hash is invalid"
        );
        ensure!(self.size > 0, "persisted artifact size is invalid");
        validate_artifact_label(&self.label, &self.kind).map_err(anyhow::Error::msg)?;
        Ok(FlowArtifactRecord {
            id: parse_uuid(&self.id, "Flow artifact ID")?,
            attempt_id: parse_uuid(&self.attempt_id, "Flow artifact attempt ID")?,
            relative_path: self.relative_path,
            label: self.label,
            kind: self.kind,
            size: sql_to_u64(self.size, "Flow artifact size")?,
            sha256: self.sha256,
            created_at: parse_datetime(&self.created_at, "Flow artifact createdAt")?,
        })
    }
}

fn enum_name<T: Serialize>(value: T) -> anyhow::Result<String> {
    serde_json::to_value(value)?
        .as_str()
        .map(str::to_owned)
        .context("persisted enum did not serialize as a string")
}

fn parse_enum_name<T: DeserializeOwned>(value: &str, field: &str) -> anyhow::Result<T> {
    serde_json::from_value(Value::String(value.to_string()))
        .with_context(|| format!("parse persisted {field}"))
}

fn encode_optional_json<T: Serialize>(value: Option<&T>) -> anyhow::Result<Option<String>> {
    value
        .map(serde_json::to_string)
        .transpose()
        .map_err(Into::into)
}

fn decode_optional_json<T: DeserializeOwned>(
    value: Option<String>,
    field: &str,
) -> anyhow::Result<Option<T>> {
    value
        .map(|json| serde_json::from_str(&json).with_context(|| format!("parse persisted {field}")))
        .transpose()
}

fn parse_uuid(value: &str, field: &str) -> anyhow::Result<Uuid> {
    Uuid::parse_str(value).with_context(|| format!("parse persisted {field}"))
}

fn parse_datetime(value: &str, field: &str) -> anyhow::Result<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .map(|value| value.with_timezone(&Utc))
        .with_context(|| format!("parse persisted {field}"))
}

fn parse_optional_datetime(
    value: Option<String>,
    field: &str,
) -> anyhow::Result<Option<DateTime<Utc>>> {
    value.map(|value| parse_datetime(&value, field)).transpose()
}

fn now_text() -> String {
    datetime_text(Utc::now())
}

fn datetime_text(value: DateTime<Utc>) -> String {
    value.to_rfc3339_opts(SecondsFormat::Nanos, true)
}

fn u64_to_sql(value: u64, field: &str) -> anyhow::Result<i64> {
    i64::try_from(value).with_context(|| format!("{field} exceeds SQLite INTEGER range"))
}

fn usize_to_sql(value: usize, field: &str) -> anyhow::Result<i64> {
    i64::try_from(value).with_context(|| format!("{field} exceeds SQLite INTEGER range"))
}

fn sql_to_u64(value: i64, field: &str) -> anyhow::Result<u64> {
    u64::try_from(value).with_context(|| format!("{field} is negative"))
}

fn is_lower_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};
    use std::path::{Path, PathBuf};

    use chrono::Utc;
    use rusqlite::params;
    use serde_json::json;
    use uuid::Uuid;

    use super::super::Database;
    use super::*;
    use crate::{
        compiled_plan_sha256, ActionKind, ActiveTransport, CompiledActionConfig, CompiledFlowNode,
        CompiledFlowPlanV2, ContextPlan, DeviceCapabilitySnapshot, DeviceWorkOwner,
        EvidenceBaseline, FlowAggregateState, FlowArtifactRecord, FlowAttemptState,
        FlowCapabilitySnapshot, FlowContextReleaseProof, FlowDeviceRunState, FlowDocumentV2,
        FlowErrorRecord, FlowPreflightScope, FlowSelectionSnapshot, FlowTargetSelection,
        InstalledAgentIdentity, InstalledTargetIdentity, QualifiedGeometry, ScreenOrientation,
        SideEffectClass, FLOW_SCHEMA_VERSION,
    };

    fn database_fixture() -> (Database, PathBuf) {
        let path = std::env::temp_dir().join(format!("riviu-flow-runs-{}.db", Uuid::new_v4()));
        let database = Database::open(&path).expect("runtime database");
        (database, path)
    }

    fn cleanup(path: &Path) {
        for suffix in ["", "-wal", "-shm"] {
            let candidate = PathBuf::from(format!("{}{suffix}", path.display()));
            if candidate.exists() {
                std::fs::remove_file(candidate).expect("remove runtime database");
            }
        }
    }

    fn save_revision(
        database: &Database,
        kind: ActionKind,
        config: CompiledActionConfig,
    ) -> (crate::FlowRevisionRecord, CompiledFlowNode) {
        let mut document = FlowDocumentV2::empty("Runtime fixture");
        document.revision = 1;
        let postcondition = match &config {
            CompiledActionConfig::LaunchApp { bundle_id } => Some(EvidenceSpec::ActiveAppEquals {
                bundle_id: bundle_id.clone(),
            }),
            CompiledActionConfig::TerminateApp { bundle_id } => Some(EvidenceSpec::ProcessAbsent {
                bundle_id: bundle_id.clone(),
            }),
            CompiledActionConfig::Screenshot { .. } => Some(EvidenceSpec::ArtifactDecodedAndHashed),
            CompiledActionConfig::Tap { .. } | CompiledActionConfig::Empty
                if kind == ActionKind::Tap =>
            {
                Some(EvidenceSpec::FrameDigestChanged {
                    minimum_distance: 1,
                })
            }
            _ => None,
        };
        let node = CompiledFlowNode {
            id: Uuid::new_v4(),
            kind,
            config,
            postcondition,
        };
        let plan = CompiledFlowPlanV2 {
            schema_version: FLOW_SCHEMA_VERSION,
            flow_id: document.id,
            revision: 1,
            nodes: BTreeMap::from([(node.id, node.clone())]),
            execution_order: vec![node.id],
            context_plan: ContextPlan {
                requires_exclusive: false,
                requires_ui_session: false,
                requires_stream: false,
                requires_fresh_text_session: false,
                initial_bundle_id: None,
            },
            action_definition_versions: BTreeMap::from([(kind, 1)]),
            required_capabilities: BTreeSet::new(),
        };
        let hash = compiled_plan_sha256(&plan).expect("plan hash");
        let revision = database
            .save_flow_revision(None, &document, &plan, &hash)
            .expect("save runtime revision");
        (revision, node)
    }

    fn save_two_wait_revision(
        database: &Database,
    ) -> (crate::FlowRevisionRecord, [CompiledFlowNode; 2]) {
        let mut document = FlowDocumentV2::empty("Recovery aggregate fixture");
        document.revision = 1;
        let first = CompiledFlowNode {
            id: Uuid::new_v4(),
            kind: ActionKind::Wait,
            config: CompiledActionConfig::Wait { duration_ms: 10 },
            postcondition: None,
        };
        let second = CompiledFlowNode {
            id: Uuid::new_v4(),
            kind: ActionKind::Wait,
            config: CompiledActionConfig::Wait { duration_ms: 20 },
            postcondition: None,
        };
        let plan = CompiledFlowPlanV2 {
            schema_version: FLOW_SCHEMA_VERSION,
            flow_id: document.id,
            revision: document.revision,
            nodes: BTreeMap::from([(first.id, first.clone()), (second.id, second.clone())]),
            execution_order: vec![first.id, second.id],
            context_plan: ContextPlan {
                requires_exclusive: false,
                requires_ui_session: false,
                requires_stream: false,
                requires_fresh_text_session: false,
                initial_bundle_id: None,
            },
            action_definition_versions: BTreeMap::from([(ActionKind::Wait, 1)]),
            required_capabilities: BTreeSet::new(),
        };
        let hash = compiled_plan_sha256(&plan).expect("plan hash");
        let revision = database
            .save_flow_revision(None, &document, &plan, &hash)
            .expect("save recovery revision");
        (revision, [first, second])
    }

    fn selection(udids: &[&str]) -> FlowSelectionSnapshot {
        let values = udids
            .iter()
            .map(|udid| (*udid).to_string())
            .collect::<Vec<_>>();
        let mut target_udids = values.clone();
        target_udids.sort();
        FlowSelectionSnapshot {
            requested: if values.len() == 1 {
                FlowTargetSelection::One {
                    udid: values[0].clone(),
                }
            } else {
                FlowTargetSelection::Selected {
                    udids: values.clone(),
                }
            },
            target_udids,
        }
    }

    fn release_proof(udid: &str) -> FlowContextReleaseProof {
        FlowContextReleaseProof {
            udid: udid.to_string(),
            owner: DeviceWorkOwner::Script,
            had_session: false,
            had_stream: false,
        }
    }

    fn ready_device(database: &Database, device_run_id: Uuid) -> FlowDeviceRunRecord {
        database
            .transition_flow_device_run(
                device_run_id,
                FlowDeviceRunState::Queued,
                FlowDeviceRunState::Preflight,
                None,
            )
            .expect("enter device preflight");
        database
            .transition_flow_device_run(
                device_run_id,
                FlowDeviceRunState::Preflight,
                FlowDeviceRunState::Running,
                Some(capability_snapshot()),
            )
            .expect("qualify running device")
    }

    fn capability_snapshot() -> FlowCapabilitySnapshot {
        let device = DeviceCapabilitySnapshot {
            installed_agent: InstalledAgentIdentity {
                bundle_id: "com.mrph.svc".into(),
                version: "1.0".into(),
                build: "1".into(),
                executable_name: "fixture-agent".into(),
                signer_identity_sha256: "22".repeat(32),
            },
            selected_artifact_sha256: "33".repeat(32),
            agent_version: "1.0".into(),
            protocol_version: 1,
            driver_adapter_version: "fixture-driver-1".into(),
            transport: ActiveTransport::Mock,
            product_type: "iPhone10,1".into(),
            ios_version: "16.7.15".into(),
            target_app: InstalledTargetIdentity {
                bundle_id: "com.apple.Preferences".into(),
                version: "1".into(),
                build: "1".into(),
            },
            protected_auth_ready: true,
            geometry: Some(QualifiedGeometry {
                logical_width: 375.0,
                logical_height: 667.0,
                pixel_width: 375,
                pixel_height: 667,
                scale_x: 1.0,
                scale_y: 1.0,
                orientation: ScreenOrientation::Portrait,
            }),
        };
        FlowCapabilitySnapshot {
            scope: FlowPreflightScope::TargetQualified {
                bundle_id: device.target_app.bundle_id.clone(),
            },
            device: Some(device),
            agent_status: None,
            capability_ids: BTreeSet::from(["app.launch".to_string()]),
        }
    }

    fn error(code: &str, attempt_id: Option<Uuid>) -> FlowErrorRecord {
        FlowErrorRecord {
            code: code.to_string(),
            message: format!("{code} fixture"),
            node_id: None,
            field: None,
            udid: None,
            attempt_id,
        }
    }

    fn attempt_fixture(
        kind: ActionKind,
        config: CompiledActionConfig,
        side_effect: SideEffectClass,
    ) -> (Database, PathBuf, Uuid, Uuid, CompiledFlowNode) {
        let (database, path) = database_fixture();
        let (revision, node) = save_revision(&database, kind, config);
        let run = database
            .create_flow_run(&revision, selection(&["fixture-udid"]))
            .expect("create run");
        let device = database
            .create_flow_device_run(run.id, "fixture-udid")
            .expect("create device run");
        ready_device(&database, device.id);
        let attempt = database
            .create_flow_attempt(device.id, &node, side_effect, 1)
            .expect("create attempt");
        (database, path, run.id, attempt.id, node)
    }

    fn set_attempt_state(database: &Database, attempt_id: Uuid, state: FlowAttemptState) {
        let state = serde_json::to_value(state)
            .expect("state JSON")
            .as_str()
            .expect("state string")
            .to_string();
        let connection = database.conn().expect("connection");
        let identity = query_attempt_identity(&connection, attempt_id)
            .expect("attempt identity query")
            .expect("attempt identity");
        let baseline = match contracts(identity.action_kind).2 {
            EvidenceRequirement::None | EvidenceRequirement::ActiveApp => json!({"kind":"none"}),
            EvidenceRequirement::Process => {
                let CompiledActionConfig::TerminateApp { bundle_id } = identity.node.config else {
                    panic!("process fixture requires Terminate App");
                };
                json!({"kind":"process","bundleId":bundle_id,"pid":42})
            }
            EvidenceRequirement::Frame
            | EvidenceRequirement::TextOrQualifiedFrame
            | EvidenceRequirement::Artifact => json!({
                "kind":"frame",
                "generation":1,
                "jpegSha256":"a".repeat(64),
                "imageWidth":1,
                "imageHeight":1,
                "rgbBase64":"AAAA"
            }),
        };
        let now = now_text();
        let finished_at = parse_enum_name::<FlowAttemptState>(&state, "fixture state")
            .expect("fixture state enum")
            .is_terminal()
            .then_some(now.clone());
        let error_json = (state == "failedVerified").then(|| {
            serde_json::to_string(&error("EvidenceMismatch", Some(attempt_id)))
                .expect("failed verification error")
        });
        connection
            .execute(
                "UPDATE flow_node_attempts SET
                    state=?2,canonical_input_json=?3,evidence_baseline_json=?4,
                    started_at=?5,finished_at=?6,error_json=?7
                 WHERE id=?1",
                params![
                    attempt_id.to_string(),
                    state,
                    serde_json::to_string(&json!({"fixture":true})).expect("canonical input"),
                    serde_json::to_string(&baseline).expect("baseline"),
                    now,
                    finished_at,
                    error_json,
                ],
            )
            .expect("seed attempt state");
    }

    fn fail_device_for_attempt(database: &Database, attempt_id: Uuid) {
        let context = database
            .get_flow_attempt_execution_context(attempt_id)
            .expect("load failed attempt context")
            .expect("failed attempt context");
        let error = context
            .attempt
            .error
            .clone()
            .unwrap_or_else(|| error("FixtureFailure", Some(attempt_id)));
        database
            .mark_device_terminal(
                context.device.id,
                &[FlowDeviceRunState::Running],
                FlowDeviceRunState::Failed,
                Some(error),
                release_proof(&context.device.udid),
            )
            .expect("terminalize failed attempt device");
        database
            .recompute_run_projection(context.run.id)
            .expect("project failed attempt run");
    }

    #[test]
    fn run_creation_freezes_selection_and_appends_monotonic_events() {
        let (database, path) = database_fixture();
        let (revision, _) = save_revision(
            &database,
            ActionKind::Wait,
            CompiledActionConfig::Wait { duration_ms: 10 },
        );
        let snapshot = selection(&["device-b", "device-a"]);

        let run = database
            .create_flow_run(&revision, snapshot.clone())
            .expect("create run");
        let first = database
            .create_flow_device_run(run.id, "device-b")
            .expect("first device");
        let second = database
            .create_flow_device_run(run.id, "device-a")
            .expect("second device");
        assert!(database
            .create_flow_device_run(run.id, "device-outside-snapshot")
            .is_err());

        assert_eq!(run.selection, snapshot);
        assert_eq!(run.state, FlowAggregateState::Queued);
        assert_eq!(run.event_revision, 1);
        assert_eq!(first.state, FlowDeviceRunState::Queued);
        assert_eq!(second.state, FlowDeviceRunState::Queued);
        let stored = database
            .get_flow_run(run.id)
            .expect("get run")
            .expect("stored run");
        assert_eq!(stored.run.event_revision, 3);
        assert_eq!(
            stored
                .device_runs
                .iter()
                .map(|device| device.udid.as_str())
                .collect::<Vec<_>>(),
            vec!["device-a", "device-b"]
        );

        let connection = database.conn().expect("connection");
        let revisions = connection
            .prepare("SELECT revision FROM flow_events WHERE run_id=?1 ORDER BY revision")
            .expect("prepare events")
            .query_map([run.id.to_string()], |row| row.get::<_, i64>(0))
            .expect("query events")
            .collect::<rusqlite::Result<Vec<_>>>()
            .expect("event revisions");
        assert_eq!(revisions, vec![1, 2, 3]);
        drop(connection);
        database
            .conn()
            .expect("corruption connection")
            .execute(
                "DELETE FROM flow_events WHERE run_id=?1 AND revision=2",
                [run.id.to_string()],
            )
            .expect("remove middle event");
        assert!(database.get_flow_run(run.id).is_err());
        assert!(database.list_flow_runs(10).is_err());
        cleanup(&path);
    }

    #[test]
    fn aggregate_run_creation_persists_the_exact_sorted_device_projection() {
        let (database, path) = database_fixture();
        let (revision, _) = save_revision(
            &database,
            ActionKind::Wait,
            CompiledActionConfig::Wait { duration_ms: 10 },
        );
        let snapshot = selection(&["device-c", "device-a", "device-b"]);

        let (run, devices) = database
            .create_flow_run_with_devices(&revision, snapshot.clone())
            .expect("create aggregate run");

        assert_eq!(run.selection, snapshot);
        assert_eq!(run.event_revision, 4);
        assert_eq!(
            devices
                .iter()
                .map(|device| device.udid.as_str())
                .collect::<Vec<_>>(),
            vec!["device-a", "device-b", "device-c"]
        );
        assert!(devices
            .iter()
            .all(|device| device.run_id == run.id && device.state == FlowDeviceRunState::Queued));

        let stored = database
            .get_flow_run(run.id)
            .expect("load aggregate run")
            .expect("aggregate run exists");
        assert_eq!(stored.run.event_revision, 4);
        assert_eq!(stored.device_runs, devices);
        cleanup(&path);
    }

    #[test]
    fn aggregate_run_creation_rolls_back_the_run_when_any_device_insert_fails() {
        let (database, path) = database_fixture();
        let (revision, _) = save_revision(
            &database,
            ActionKind::Wait,
            CompiledActionConfig::Wait { duration_ms: 10 },
        );
        database
            .conn()
            .expect("trigger connection")
            .execute_batch(
                "CREATE TRIGGER reject_device_b
                 BEFORE INSERT ON flow_device_runs
                 WHEN NEW.udid='device-b'
                 BEGIN
                   SELECT RAISE(ABORT, 'fixture device insert failed');
                 END;",
            )
            .expect("install failure trigger");

        let result =
            database.create_flow_run_with_devices(&revision, selection(&["device-a", "device-b"]));
        assert!(result.is_err());

        let connection = database.conn().expect("verification connection");
        let run_count: i64 = connection
            .query_row("SELECT COUNT(*) FROM flow_runs", [], |row| row.get(0))
            .expect("count runs");
        let device_count: i64 = connection
            .query_row("SELECT COUNT(*) FROM flow_device_runs", [], |row| {
                row.get(0)
            })
            .expect("count devices");
        assert_eq!((run_count, device_count), (0, 0));
        drop(connection);
        cleanup(&path);
    }

    #[test]
    fn recovery_aggregate_includes_every_device_crash_boundary() {
        let (database, path) = database_fixture();
        let (revision, nodes) = save_two_wait_revision(&database);
        let (run, devices) = database
            .create_flow_run_with_devices(
                &revision,
                selection(&[
                    "preflight-zero",
                    "succeeded-not-terminal",
                    "failed-with-successor",
                ]),
            )
            .expect("create recovery aggregate");
        let by_udid = devices
            .iter()
            .map(|device| (device.udid.as_str(), device.id))
            .collect::<std::collections::HashMap<_, _>>();

        database
            .transition_flow_device_run(
                by_udid["preflight-zero"],
                FlowDeviceRunState::Queued,
                FlowDeviceRunState::Preflight,
                None,
            )
            .expect("seed preflight with no attempts");

        ready_device(&database, by_udid["succeeded-not-terminal"]);
        let succeeded_attempts = database
            .initialize_flow_device_attempts(by_udid["succeeded-not-terminal"])
            .expect("initialize succeeded attempts");
        for attempt in &succeeded_attempts {
            set_attempt_state(&database, attempt.id, FlowAttemptState::Succeeded);
        }

        ready_device(&database, by_udid["failed-with-successor"]);
        let failed_attempts = database
            .initialize_flow_device_attempts(by_udid["failed-with-successor"])
            .expect("initialize failed attempts");
        let first = failed_attempts
            .iter()
            .find(|attempt| attempt.node_id == nodes[0].id)
            .expect("first attempt");
        set_attempt_state(&database, first.id, FlowAttemptState::FailedVerified);

        let contexts = database
            .load_flow_recovery_contexts()
            .expect("load recovery contexts");
        assert_eq!(contexts.len(), 1);
        let context = &contexts[0];
        assert_eq!(context.run.id, run.id);
        assert_eq!(context.plan, revision.compiled_plan);
        assert_eq!(context.devices.len(), 3);
        assert_eq!(context.attempts.len(), 4);
        assert!(context.artifacts.is_empty());

        let attempts_for = |udid: &str| {
            let device_id = by_udid[udid];
            context
                .attempts
                .iter()
                .filter(|attempt| attempt.device_run_id == device_id)
                .map(|attempt| attempt.state)
                .collect::<Vec<_>>()
        };
        assert!(attempts_for("preflight-zero").is_empty());
        assert_eq!(
            attempts_for("succeeded-not-terminal"),
            vec![FlowAttemptState::Succeeded, FlowAttemptState::Succeeded]
        );
        assert_eq!(
            attempts_for("failed-with-successor"),
            vec![FlowAttemptState::FailedVerified, FlowAttemptState::Queued]
        );
        cleanup(&path);
    }

    #[test]
    fn run_reads_fail_closed_when_the_pinned_plan_drifts() {
        let (database, path) = database_fixture();
        let (revision, _) = save_revision(
            &database,
            ActionKind::Wait,
            CompiledActionConfig::Wait { duration_ms: 10 },
        );
        let run = database
            .create_flow_run(&revision, selection(&["fixture-udid"]))
            .expect("create run");
        database
            .conn()
            .expect("corruption connection")
            .execute(
                "UPDATE flow_runs SET plan_sha256=?2 WHERE id=?1",
                params![run.id.to_string(), "44".repeat(32)],
            )
            .expect("seed run hash drift");

        assert!(database.get_flow_run(run.id).is_err());
        assert!(database.list_flow_runs(10).is_err());
        cleanup(&path);
    }

    #[test]
    fn device_preflight_persists_capability_snapshot_before_running() {
        let (database, path) = database_fixture();
        let (revision, node) = save_revision(
            &database,
            ActionKind::Wait,
            CompiledActionConfig::Wait { duration_ms: 10 },
        );
        let run = database
            .create_flow_run(&revision, selection(&["fixture-udid"]))
            .expect("create run");
        let device = database
            .create_flow_device_run(run.id, "fixture-udid")
            .expect("create device run");
        let snapshot = capability_snapshot();

        assert!(database
            .create_flow_attempt(device.id, &node, SideEffectClass::None, 1)
            .is_err());
        assert!(database
            .transition_flow_device_run(
                device.id,
                FlowDeviceRunState::Queued,
                FlowDeviceRunState::Running,
                Some(snapshot.clone()),
            )
            .is_err());
        let preflight = database
            .transition_flow_device_run(
                device.id,
                FlowDeviceRunState::Queued,
                FlowDeviceRunState::Preflight,
                None,
            )
            .expect("enter preflight");
        assert_eq!(preflight.state, FlowDeviceRunState::Preflight);
        assert!(preflight.capability_snapshot.is_none());
        assert!(preflight.started_at.is_some());

        let running = database
            .transition_flow_device_run(
                device.id,
                FlowDeviceRunState::Preflight,
                FlowDeviceRunState::Running,
                Some(snapshot.clone()),
            )
            .expect("enter running");
        assert_eq!(running.state, FlowDeviceRunState::Running);
        assert_eq!(running.capability_snapshot, Some(snapshot.clone()));
        database
            .create_flow_attempt(device.id, &node, SideEffectClass::None, 1)
            .expect("qualified device can create an attempt");
        assert!(database
            .transition_flow_device_run(
                device.id,
                FlowDeviceRunState::Preflight,
                FlowDeviceRunState::Running,
                Some(snapshot),
            )
            .is_err());

        let stored = database
            .get_flow_run(run.id)
            .expect("get run")
            .expect("stored run");
        assert_eq!(stored.device_runs[0], running);
        assert_eq!(stored.run.event_revision, 5);
        database
            .conn()
            .expect("corruption connection")
            .execute(
                "UPDATE flow_device_runs SET finished_at=?2 WHERE id=?1",
                params![device.id.to_string(), now_text()],
            )
            .expect("seed nonterminal finish timestamp");
        assert!(database.get_flow_run(run.id).is_err());
        cleanup(&path);
    }

    #[test]
    fn device_terminal_error_must_reference_the_latest_attempt_for_its_node() {
        let (database, path) = database_fixture();
        let (revision, node) = save_revision(
            &database,
            ActionKind::Wait,
            CompiledActionConfig::Wait { duration_ms: 10 },
        );
        let run = database
            .create_flow_run(&revision, selection(&["device-a", "device-b"]))
            .expect("create run");
        let device_a = database
            .create_flow_device_run(run.id, "device-a")
            .expect("device a");
        let device_b = database
            .create_flow_device_run(run.id, "device-b")
            .expect("device b");
        ready_device(&database, device_a.id);
        ready_device(&database, device_b.id);
        let attempt_a = database
            .create_flow_attempt(device_a.id, &node, SideEffectClass::None, 1)
            .expect("attempt a");
        let attempt_b = database
            .create_flow_attempt(device_b.id, &node, SideEffectClass::None, 1)
            .expect("attempt b");
        set_attempt_state(
            &database,
            attempt_a.id,
            FlowAttemptState::FailedBeforeDispatch,
        );
        set_attempt_state(
            &database,
            attempt_b.id,
            FlowAttemptState::FailedBeforeDispatch,
        );
        let latest_attempt_id = Uuid::new_v4();
        database
            .conn()
            .expect("connection")
            .execute(
                "INSERT INTO flow_node_attempts(
                    id,device_run_id,node_id,action_kind,attempt_no,side_effect_class,state,
                    canonical_input_json,evidence_baseline_json,evidence_result_json,retry_safe,
                    error_json,started_at,updated_at,finished_at
                 ) VALUES(?1,?2,?3,'wait',2,'none','queued',NULL,NULL,NULL,0,NULL,NULL,?4,NULL)",
                params![
                    latest_attempt_id.to_string(),
                    device_a.id.to_string(),
                    node.id.to_string(),
                    now_text(),
                ],
            )
            .expect("seed retry attempt");
        set_attempt_state(
            &database,
            latest_attempt_id,
            FlowAttemptState::FailedBeforeDispatch,
        );

        assert!(database
            .mark_device_terminal(
                device_a.id,
                &[FlowDeviceRunState::Running],
                FlowDeviceRunState::Failed,
                Some(error("WrongAttempt", Some(attempt_b.id))),
                release_proof("device-a"),
            )
            .is_err());
        assert!(database
            .mark_device_terminal(
                device_a.id,
                &[FlowDeviceRunState::Running],
                FlowDeviceRunState::Failed,
                Some(error("MissingAttempt", Some(Uuid::new_v4()))),
                release_proof("device-a"),
            )
            .is_err());
        assert!(database
            .mark_device_terminal(
                device_a.id,
                &[FlowDeviceRunState::Running],
                FlowDeviceRunState::Failed,
                Some(error("ActionFailed", Some(attempt_a.id))),
                release_proof("device-a"),
            )
            .is_err());
        database
            .mark_device_terminal(
                device_a.id,
                &[FlowDeviceRunState::Running],
                FlowDeviceRunState::Failed,
                Some(error("ActionFailed", Some(latest_attempt_id))),
                release_proof("device-a"),
            )
            .expect("terminal device with latest failed attempt");
        let detail = database
            .get_flow_run(run.id)
            .expect("run detail")
            .expect("run");
        assert_eq!(
            detail
                .attempts
                .iter()
                .filter(|attempt| {
                    attempt.device_run_id == device_a.id
                        && attempt.state == FlowAttemptState::Queued
                })
                .count(),
            0
        );
        database
            .conn()
            .expect("corruption connection")
            .execute(
                "UPDATE flow_device_runs SET error_json=?2 WHERE id=?1",
                params![
                    device_a.id.to_string(),
                    serde_json::to_string(&error("StaleAttempt", Some(attempt_a.id)))
                        .expect("stale device error"),
                ],
            )
            .expect("point device error at old attempt");
        assert!(database.get_flow_run(run.id).is_err());
        assert!(database.recompute_run_projection(run.id).is_err());
        cleanup(&path);
    }

    #[test]
    fn terminal_device_readback_rejects_an_active_attempt() {
        let (database, path, run_id, attempt_id, _) = attempt_fixture(
            ActionKind::Wait,
            CompiledActionConfig::Wait { duration_ms: 10 },
            SideEffectClass::None,
        );
        set_attempt_state(&database, attempt_id, FlowAttemptState::IntentCommitted);
        let device_run_id: String = database
            .conn()
            .expect("connection")
            .query_row(
                "SELECT device_run_id FROM flow_node_attempts WHERE id=?1",
                [attempt_id.to_string()],
                |row| row.get(0),
            )
            .expect("attempt device");
        let now = now_text();
        database
            .conn()
            .expect("corruption connection")
            .execute(
                "UPDATE flow_device_runs SET state='failed',release_proof_json=?2,
                    finished_at=?3 WHERE id=?1",
                params![
                    device_run_id,
                    serde_json::to_string(&release_proof("fixture-udid")).expect("release proof"),
                    now,
                ],
            )
            .expect("seed terminal device with active attempt");
        assert!(database.get_flow_run(run_id).is_err());
        assert!(database.recompute_run_projection(run_id).is_err());
        assert!(database.load_nonterminal_attempts().is_err());
        cleanup(&path);
    }

    #[test]
    fn legal_attempt_graph_is_exhaustive_and_intent_precedes_dispatch() {
        use FlowAttemptState::*;
        let states = [
            Queued,
            IntentCommitted,
            EffectDispatched,
            Verifying,
            Succeeded,
            FailedBeforeDispatch,
            FailedVerified,
            Uncertain,
            Cancelled,
            Interrupted,
        ];
        let allowed = BTreeSet::from([
            (Queued, IntentCommitted),
            (Queued, Cancelled),
            (IntentCommitted, EffectDispatched),
            (IntentCommitted, FailedBeforeDispatch),
            (IntentCommitted, Cancelled),
            (EffectDispatched, Verifying),
            (EffectDispatched, FailedBeforeDispatch),
            (EffectDispatched, Uncertain),
            (EffectDispatched, Interrupted),
            (EffectDispatched, Cancelled),
            (Verifying, Succeeded),
            (Verifying, FailedVerified),
            (Verifying, Uncertain),
            (Verifying, Interrupted),
            (Verifying, Cancelled),
            (Queued, Interrupted),
            (Interrupted, Queued),
        ]);
        for from in states {
            for to in states {
                assert_eq!(
                    legal_transition(from, to),
                    allowed.contains(&(from, to)),
                    "{from:?} -> {to:?}"
                );
            }
        }

        let (database, path, _, attempt_id, _) = attempt_fixture(
            ActionKind::Wait,
            CompiledActionConfig::Wait { duration_ms: 10 },
            SideEffectClass::None,
        );
        let error = database
            .transition_attempt(
                attempt_id,
                Queued,
                EffectDispatched,
                AttemptTransitionPatch::default(),
            )
            .expect_err("skipped intent must fail");
        assert!(error.to_string().contains("StateConflict"));
        assert!(database
            .transition_attempt(
                attempt_id,
                Queued,
                IntentCommitted,
                AttemptTransitionPatch::default(),
            )
            .is_err());
        let intent = database
            .transition_attempt(
                attempt_id,
                Queued,
                IntentCommitted,
                AttemptTransitionPatch {
                    canonical_input: Some(json!({"durationMs": 10})),
                    evidence_baseline: Some(json!({"kind": "none"})),
                    ..Default::default()
                },
            )
            .expect("commit intent");
        assert_eq!(intent.canonical_input, Some(json!({"durationMs": 10})));
        assert!(intent.started_at.is_some());
        database
            .transition_attempt(
                attempt_id,
                IntentCommitted,
                EffectDispatched,
                AttemptTransitionPatch::default(),
            )
            .expect("record dispatch boundary");
        database
            .transition_attempt(
                attempt_id,
                EffectDispatched,
                Interrupted,
                AttemptTransitionPatch::default(),
            )
            .expect("interrupt read-only attempt");
        database
            .transition_attempt(
                attempt_id,
                Interrupted,
                Queued,
                AttemptTransitionPatch::default(),
            )
            .expect("requeue read-only attempt");
        assert!(database
            .transition_attempt(
                attempt_id,
                Queued,
                IntentCommitted,
                AttemptTransitionPatch {
                    canonical_input: Some(json!({"durationMs": 11})),
                    evidence_baseline: Some(json!({"kind": "none"})),
                    ..Default::default()
                },
            )
            .is_err());
        database
            .transition_attempt(
                attempt_id,
                Queued,
                IntentCommitted,
                AttemptTransitionPatch {
                    canonical_input: Some(json!({"durationMs": 10})),
                    evidence_baseline: Some(json!({"kind": "none"})),
                    ..Default::default()
                },
            )
            .expect("recommit exact immutable intent");
        cleanup(&path);
    }

    #[test]
    fn post_dispatch_side_effects_fail_closed_without_delivery_or_cancel_proof() {
        let (database, path, _, attempt_id, _) = attempt_fixture(
            ActionKind::Tap,
            CompiledActionConfig::Empty,
            SideEffectClass::AmbiguousUi,
        );
        set_attempt_state(&database, attempt_id, FlowAttemptState::EffectDispatched);

        assert!(database
            .transition_attempt(
                attempt_id,
                FlowAttemptState::EffectDispatched,
                FlowAttemptState::Cancelled,
                AttemptTransitionPatch::default(),
            )
            .is_err());
        assert!(database
            .transition_attempt(
                attempt_id,
                FlowAttemptState::EffectDispatched,
                FlowAttemptState::FailedBeforeDispatch,
                AttemptTransitionPatch::default(),
            )
            .is_err());
        assert!(database
            .transition_attempt(
                attempt_id,
                FlowAttemptState::EffectDispatched,
                FlowAttemptState::Verifying,
                AttemptTransitionPatch {
                    evidence_result: Some(json!({
                        "kind": "frameDigestChanged",
                        "matched": true,
                        "observedSha256": "e".repeat(64),
                        "measurement": {"distance": 1}
                    })),
                    ..Default::default()
                },
            )
            .is_err());
        database
            .transition_attempt(
                attempt_id,
                FlowAttemptState::EffectDispatched,
                FlowAttemptState::FailedBeforeDispatch,
                AttemptTransitionPatch {
                    evidence_result: Some(json!({
                        "kind": "transportNonDelivery",
                        "requestReachedDevice": false
                    })),
                    ..Default::default()
                },
            )
            .expect("typed non-delivery proof");

        let (corrupt, corrupt_path, _, corrupt_attempt, _) = attempt_fixture(
            ActionKind::Tap,
            CompiledActionConfig::Empty,
            SideEffectClass::AmbiguousUi,
        );
        corrupt
            .conn()
            .expect("corruption connection")
            .execute(
                "UPDATE flow_node_attempts SET state='effectDispatched',side_effect_class='none' WHERE id=?1",
                [corrupt_attempt.to_string()],
            )
            .expect("seed valid-CHECK contract corruption");
        assert!(corrupt
            .transition_attempt(
                corrupt_attempt,
                FlowAttemptState::EffectDispatched,
                FlowAttemptState::Cancelled,
                AttemptTransitionPatch::default(),
            )
            .is_err());
        corrupt
            .conn()
            .expect("corruption connection")
            .execute(
                "UPDATE flow_node_attempts
                 SET action_kind='launchApp',side_effect_class='idempotentSet'
                 WHERE id=?1",
                [corrupt_attempt.to_string()],
            )
            .expect("seed internally consistent action corruption");
        assert!(corrupt.load_nonterminal_attempts().is_err());
        cleanup(&path);
        cleanup(&corrupt_path);
    }

    #[test]
    fn failed_verified_requires_a_durable_typed_verifier_error() {
        let (database, path, run_id, attempt_id, _) = attempt_fixture(
            ActionKind::Tap,
            CompiledActionConfig::Empty,
            SideEffectClass::AmbiguousUi,
        );
        set_attempt_state(&database, attempt_id, FlowAttemptState::Verifying);

        assert!(database
            .transition_attempt(
                attempt_id,
                FlowAttemptState::Verifying,
                FlowAttemptState::FailedVerified,
                AttemptTransitionPatch::default(),
            )
            .is_err());
        database
            .transition_attempt(
                attempt_id,
                FlowAttemptState::Verifying,
                FlowAttemptState::FailedVerified,
                AttemptTransitionPatch {
                    error: Some(error("EvidenceMismatch", Some(attempt_id))),
                    ..Default::default()
                },
            )
            .expect("typed verifier error");
        database
            .conn()
            .expect("corruption connection")
            .execute(
                "UPDATE flow_node_attempts SET error_json=NULL WHERE id=?1",
                [attempt_id.to_string()],
            )
            .expect("remove durable verifier diagnostic");
        assert!(database.get_flow_run(run_id).is_err());
        cleanup(&path);
    }

    #[test]
    fn failed_verified_rejects_a_measurement_that_actually_matches() {
        let (active, active_path, _, active_attempt, _) = attempt_fixture(
            ActionKind::LaunchApp,
            CompiledActionConfig::LaunchApp {
                bundle_id: "com.example.fixture".to_string(),
            },
            SideEffectClass::IdempotentSet,
        );
        set_attempt_state(&active, active_attempt, FlowAttemptState::Verifying);
        assert!(active
            .transition_attempt(
                active_attempt,
                FlowAttemptState::Verifying,
                FlowAttemptState::FailedVerified,
                AttemptTransitionPatch {
                    evidence_result: Some(json!({
                        "kind": "activeAppEquals",
                        "matched": false,
                        "observedSha256": "b".repeat(64),
                        "measurement": {"bundleId": "com.example.fixture"}
                    })),
                    error: Some(error("EvidenceMismatch", Some(active_attempt))),
                    ..Default::default()
                },
            )
            .is_err());

        let (frame, frame_path, _, frame_attempt, _) = attempt_fixture(
            ActionKind::Tap,
            CompiledActionConfig::Empty,
            SideEffectClass::AmbiguousUi,
        );
        set_attempt_state(&frame, frame_attempt, FlowAttemptState::Verifying);
        assert!(frame
            .transition_attempt(
                frame_attempt,
                FlowAttemptState::Verifying,
                FlowAttemptState::FailedVerified,
                AttemptTransitionPatch {
                    evidence_result: Some(json!({
                        "kind": "frameDigestChanged",
                        "matched": false,
                        "observedSha256": "b".repeat(64),
                        "measurement": {
                            "generation": 1,
                            "baselineSha256": "a".repeat(64),
                            "distance": 1
                        }
                    })),
                    error: Some(error("EvidenceMismatch", Some(frame_attempt))),
                    ..Default::default()
                },
            )
            .is_err());
        frame
            .transition_attempt(
                frame_attempt,
                FlowAttemptState::Verifying,
                FlowAttemptState::FailedVerified,
                AttemptTransitionPatch {
                    evidence_result: Some(json!({
                        "kind": "frameDigestChanged",
                        "matched": false,
                        "observedSha256": "a".repeat(64),
                        "measurement": {
                            "generation": 1,
                            "baselineSha256": "a".repeat(64),
                            "distance": 0
                        }
                    })),
                    error: Some(error("EvidenceMismatch", Some(frame_attempt))),
                    ..Default::default()
                },
            )
            .expect("bound frame mismatch evidence");
        cleanup(&active_path);
        cleanup(&frame_path);
    }

    #[test]
    fn nonterminal_errors_are_durable_but_never_enable_retry() {
        let (database, path, _, attempt_id, _) = attempt_fixture(
            ActionKind::Wait,
            CompiledActionConfig::Wait { duration_ms: 10 },
            SideEffectClass::None,
        );
        set_attempt_state(&database, attempt_id, FlowAttemptState::Verifying);
        let recorded = database
            .record_nonterminal_attempt_error(
                attempt_id,
                FlowAttemptState::Verifying,
                error("VerifierUnavailable", Some(attempt_id)),
            )
            .expect("record infrastructure error");
        assert_eq!(recorded.state, FlowAttemptState::Verifying);
        assert!(!recorded.retry_allowed);
        assert_eq!(recorded.error.unwrap().code, "VerifierUnavailable");
        assert!(database
            .record_nonterminal_attempt_error(
                attempt_id,
                FlowAttemptState::IntentCommitted,
                error("StaleWriter", Some(attempt_id)),
            )
            .is_err());
        cleanup(&path);
    }

    #[test]
    fn artifact_row_and_attempt_success_commit_atomically() {
        let (database, path, run_id, attempt_id, _) = attempt_fixture(
            ActionKind::Screenshot,
            CompiledActionConfig::Screenshot {
                label: "screen.png".to_string(),
                format: "png".to_string(),
            },
            SideEffectClass::ArtifactWrite,
        );
        set_attempt_state(&database, attempt_id, FlowAttemptState::Verifying);
        let device_run_id = database
            .conn()
            .expect("connection")
            .query_row(
                "SELECT device_run_id FROM flow_node_attempts WHERE id=?1",
                [attempt_id.to_string()],
                |row| row.get::<_, String>(0),
            )
            .map(|value| Uuid::parse_str(&value).expect("device-run ID"))
            .expect("attempt device run");
        let artifact_id = Uuid::new_v4();
        let artifact = FlowArtifactRecord {
            id: artifact_id,
            attempt_id,
            relative_path: format!(
                "{}/{}/{}/{}.png",
                run_id, device_run_id, attempt_id, artifact_id
            ),
            label: "screen.png".to_string(),
            kind: "png".to_string(),
            size: 128,
            sha256: "a".repeat(64),
            created_at: Utc::now(),
        };
        let mut escaping = artifact.clone();
        escaping.relative_path = "../outside.png".to_string();
        assert!(database
            .publish_artifact_and_succeed(attempt_id, &escaping)
            .is_err());
        let mut wrong_owner = artifact.clone();
        wrong_owner.relative_path = format!(
            "{}/{}/{}/{}.png",
            Uuid::new_v4(),
            device_run_id,
            attempt_id,
            artifact_id
        );
        assert!(database
            .publish_artifact_and_succeed(attempt_id, &wrong_owner)
            .is_err());
        assert!(database
            .transition_attempt(
                attempt_id,
                FlowAttemptState::Verifying,
                FlowAttemptState::Succeeded,
                AttemptTransitionPatch {
                    evidence_result: Some(json!({
                        "kind": "artifactDecodedAndHashed",
                        "matched": true,
                        "observedSha256": artifact.sha256,
                        "measurement": {"artifactId": artifact.id}
                    })),
                    ..Default::default()
                },
            )
            .is_err());
        let completed = database
            .publish_artifact_and_succeed(attempt_id, &artifact)
            .expect("publish artifact row");
        assert_eq!(completed.state, FlowAttemptState::Succeeded);
        assert!(!completed.retry_allowed);
        let detail = database.get_flow_run(run_id).expect("detail").expect("run");
        assert_eq!(detail.artifacts, vec![artifact]);
        assert!(database
            .publish_artifact_and_succeed(attempt_id, &detail.artifacts[0])
            .is_err());
        assert_eq!(
            database
                .conn()
                .expect("connection")
                .query_row("SELECT COUNT(*) FROM flow_artifacts", [], |row| row
                    .get::<_, i64>(0))
                .expect("artifact count"),
            1
        );
        cleanup(&path);
    }

    #[test]
    fn success_requires_fresh_matched_evidence_for_the_compiled_postcondition() {
        let (database, path, run_id, attempt_id, _) = attempt_fixture(
            ActionKind::LaunchApp,
            CompiledActionConfig::LaunchApp {
                bundle_id: "com.example.fixture".to_string(),
            },
            SideEffectClass::IdempotentSet,
        );
        set_attempt_state(&database, attempt_id, FlowAttemptState::Verifying);
        let evidence = |kind: &str, matched: bool, bundle_id: &str| {
            json!({
                "kind": kind,
                "matched": matched,
                "observedSha256": "d".repeat(64),
                "measurement": {"bundleId": bundle_id}
            })
        };
        for invalid in [
            evidence("processAbsent", true, "com.example.fixture"),
            evidence("activeAppEquals", false, "com.example.fixture"),
            evidence("activeAppEquals", true, "com.example.other"),
        ] {
            assert!(database
                .transition_attempt(
                    attempt_id,
                    FlowAttemptState::Verifying,
                    FlowAttemptState::Succeeded,
                    AttemptTransitionPatch {
                        evidence_result: Some(invalid),
                        ..Default::default()
                    },
                )
                .is_err());
        }
        let succeeded = database
            .transition_attempt(
                attempt_id,
                FlowAttemptState::Verifying,
                FlowAttemptState::Succeeded,
                AttemptTransitionPatch {
                    evidence_result: Some(evidence("activeAppEquals", true, "com.example.fixture")),
                    ..Default::default()
                },
            )
            .expect("matching fresh success proof");
        assert_eq!(succeeded.state, FlowAttemptState::Succeeded);
        database
            .conn()
            .expect("corruption connection")
            .execute(
                "UPDATE flow_node_attempts SET evidence_result_json=NULL WHERE id=?1",
                [attempt_id.to_string()],
            )
            .expect("remove success evidence");
        assert!(database.get_flow_run(run_id).is_err());
        cleanup(&path);
    }

    #[test]
    fn success_proof_is_bound_to_the_committed_process_or_frame_baseline() {
        let (process_db, process_path, _, process_attempt, _) = attempt_fixture(
            ActionKind::TerminateApp,
            CompiledActionConfig::TerminateApp {
                bundle_id: "com.example.fixture".to_string(),
            },
            SideEffectClass::IdempotentSet,
        );
        set_attempt_state(&process_db, process_attempt, FlowAttemptState::Verifying);
        let process_evidence = |old_pid| {
            json!({
                "kind":"processAbsent",
                "matched":true,
                "observedSha256":"b".repeat(64),
                "measurement":{
                    "bundleId":"com.example.fixture",
                    "oldPid":old_pid,
                    "running":false
                }
            })
        };
        assert!(process_db
            .transition_attempt(
                process_attempt,
                FlowAttemptState::Verifying,
                FlowAttemptState::Succeeded,
                AttemptTransitionPatch {
                    evidence_result: Some(process_evidence(7)),
                    ..Default::default()
                },
            )
            .is_err());
        process_db
            .transition_attempt(
                process_attempt,
                FlowAttemptState::Verifying,
                FlowAttemptState::Succeeded,
                AttemptTransitionPatch {
                    evidence_result: Some(process_evidence(42)),
                    ..Default::default()
                },
            )
            .expect("process proof matches committed PID");

        let (frame_db, frame_path, _, frame_attempt, _) = attempt_fixture(
            ActionKind::Tap,
            CompiledActionConfig::Empty,
            SideEffectClass::AmbiguousUi,
        );
        set_attempt_state(&frame_db, frame_attempt, FlowAttemptState::Verifying);
        let frame_evidence = |generation| {
            json!({
                "kind":"frameDigestChanged",
                "matched":true,
                "observedSha256":"b".repeat(64),
                "measurement":{
                    "generation":generation,
                    "baselineSha256":"a".repeat(64),
                    "distance":1
                }
            })
        };
        assert!(frame_db
            .transition_attempt(
                frame_attempt,
                FlowAttemptState::Verifying,
                FlowAttemptState::Succeeded,
                AttemptTransitionPatch {
                    evidence_result: Some(frame_evidence(2)),
                    ..Default::default()
                },
            )
            .is_err());
        frame_db
            .transition_attempt(
                frame_attempt,
                FlowAttemptState::Verifying,
                FlowAttemptState::Succeeded,
                AttemptTransitionPatch {
                    evidence_result: Some(frame_evidence(1)),
                    ..Default::default()
                },
            )
            .expect("frame proof matches committed generation");
        cleanup(&process_path);
        cleanup(&frame_path);
    }

    #[test]
    fn typed_process_evidence_round_trips_through_durable_transition_contracts() {
        let spec = EvidenceSpec::ProcessAbsent {
            bundle_id: "com.example.fixture".to_string(),
        };
        let baseline = EvidenceBaseline::Process {
            bundle_id: "com.example.fixture".to_string(),
            pid: Some(42),
        };

        let (success_db, success_path, _, success_attempt, _) = attempt_fixture(
            ActionKind::TerminateApp,
            CompiledActionConfig::TerminateApp {
                bundle_id: "com.example.fixture".to_string(),
            },
            SideEffectClass::IdempotentSet,
        );
        set_attempt_state(&success_db, success_attempt, FlowAttemptState::Verifying);
        let success = crate::verify_process_absence(
            &spec,
            &baseline,
            &crate::ProcessAbsenceProof {
                bundle_id: "com.example.fixture".to_string(),
                old_pid: Some(42),
            },
        )
        .expect("typed process success");
        success_db
            .transition_attempt(
                success_attempt,
                FlowAttemptState::Verifying,
                FlowAttemptState::Succeeded,
                AttemptTransitionPatch {
                    evidence_result: Some(
                        serde_json::to_value(success).expect("serialize process success"),
                    ),
                    ..Default::default()
                },
            )
            .expect("persist typed process success");

        let (retry_db, retry_path, _, retry_attempt, _) = attempt_fixture(
            ActionKind::TerminateApp,
            CompiledActionConfig::TerminateApp {
                bundle_id: "com.example.fixture".to_string(),
            },
            SideEffectClass::IdempotentSet,
        );
        set_attempt_state(&retry_db, retry_attempt, FlowAttemptState::FailedVerified);
        fail_device_for_attempt(&retry_db, retry_attempt);
        let non_delivery = crate::evaluate_process_state(
            &spec,
            &baseline,
            &crate::AppProcessState {
                bundle_id: "com.example.fixture".to_string(),
                pid: Some(42),
                running: true,
            },
        )
        .expect("typed process non-delivery");
        let retryable = retry_db
            .record_retry_safe_reconciliation(
                retry_attempt,
                serde_json::to_value(non_delivery).expect("serialize process non-delivery"),
            )
            .expect("persist typed retry-safe process evidence");
        assert!(retryable.retry_allowed);

        cleanup(&success_path);
        cleanup(&retry_path);
    }

    #[test]
    fn retry_safety_requires_failed_verified_idempotent_set_proof() {
        let (database, path, run_id, attempt_id, node) = attempt_fixture(
            ActionKind::TerminateApp,
            CompiledActionConfig::TerminateApp {
                bundle_id: "com.example.fixture".to_string(),
            },
            SideEffectClass::IdempotentSet,
        );
        database
            .conn()
            .expect("connection")
            .execute(
                "UPDATE flow_node_attempts
                 SET state='failedVerified',canonical_input_json=?2,
                     evidence_baseline_json=?3,error_json=?4,started_at=?5,finished_at=?5
                 WHERE id=?1",
                params![
                    attempt_id.to_string(),
                    serde_json::to_string(&json!({"bundleId":"com.example.fixture"}))
                        .expect("canonical terminate input"),
                    serde_json::to_string(&json!({
                        "kind": "process",
                        "bundleId": "com.example.fixture",
                        "pid": 42
                    }))
                    .expect("process baseline"),
                    serde_json::to_string(&error("ProcessStillPresent", Some(attempt_id),))
                        .expect("failed verification error"),
                    now_text(),
                ],
            )
            .expect("seed failed process verification");
        assert!(database
            .record_retry_safe_reconciliation(attempt_id, json!({}))
            .is_err());
        assert!(database
            .record_retry_safe_reconciliation(
                attempt_id,
                json!({
                    "kind": "processAbsent",
                    "matched": false,
                    "observedSha256": "b".repeat(64),
                    "measurement": {"bundleId": "com.example.fixture", "pid": 7, "preEffectPid": 7}
                }),
            )
            .is_err());
        database
            .mark_device_terminal(
                database
                    .get_flow_attempt_execution_context(attempt_id)
                    .expect("load retry context")
                    .expect("retry context")
                    .device
                    .id,
                &[FlowDeviceRunState::Running],
                FlowDeviceRunState::Failed,
                Some(error("ProcessStillPresent", Some(attempt_id))),
                release_proof("fixture-udid"),
            )
            .expect("close failed retryable device");
        assert_eq!(
            database
                .recompute_run_projection(run_id)
                .expect("terminal failed projection")
                .state,
            FlowAggregateState::Failed
        );
        assert!(database
            .create_flow_attempt(
                database
                    .get_flow_attempt_execution_context(attempt_id)
                    .expect("load retry context")
                    .expect("retry context")
                    .device
                    .id,
                &node,
                SideEffectClass::IdempotentSet,
                2,
            )
            .is_err());
        let retryable = database
            .record_retry_safe_reconciliation(
                attempt_id,
                json!({
                    "kind": "processAbsent",
                    "matched": false,
                    "observedSha256": "b".repeat(64),
                    "measurement": {
                        "bundleId": "com.example.fixture",
                        "pid": 42,
                        "preEffectPid": 42
                    }
                }),
            )
            .expect("retry-safe proof");
        assert!(retryable.retry_allowed);
        let refreshed = database
            .record_retry_safe_reconciliation(
                attempt_id,
                json!({
                    "kind": "processAbsent",
                    "matched": false,
                    "observedSha256": "c".repeat(64),
                    "measurement": {
                        "bundleId": "com.example.fixture",
                        "pid": 42,
                        "preEffectPid": 42
                    }
                }),
            )
            .expect("refresh retry-safe proof");
        assert!(refreshed.retry_allowed);
        let retried = database
            .create_flow_attempt(
                retryable.device_run_id,
                &node,
                SideEffectClass::IdempotentSet,
                2,
            )
            .expect("atomically reopen retryable projection");
        assert_eq!(retried.state, FlowAttemptState::Queued);
        let reopened = database
            .get_flow_run(run_id)
            .expect("reopened detail")
            .expect("reopened run");
        assert_eq!(reopened.run.state, FlowAggregateState::Queued);
        assert_eq!(reopened.device_runs[0].state, FlowDeviceRunState::Queued);
        assert!(reopened.device_runs[0].release_proof.is_none());
        assert_eq!(reopened.attempts.len(), 2);
        let terminal_event: String = database
            .conn()
            .expect("connection")
            .query_row(
                "SELECT payload_json FROM flow_events
                 WHERE run_id=?1 AND kind='deviceRunTerminal'",
                [run_id.to_string()],
                |row| row.get(0),
            )
            .expect("terminal proof event");
        assert_eq!(
            serde_json::from_str::<Value>(&terminal_event).expect("terminal event JSON")
                ["releaseProof"]["udid"],
            "fixture-udid"
        );

        let (other, other_path, _, other_attempt, _) = attempt_fixture(
            ActionKind::Tap,
            CompiledActionConfig::Empty,
            SideEffectClass::AmbiguousUi,
        );
        set_attempt_state(&other, other_attempt, FlowAttemptState::FailedVerified);
        assert!(other
            .record_retry_safe_reconciliation(other_attempt, json!({"matched":true}))
            .is_err());
        cleanup(&path);
        cleanup(&other_path);
    }

    #[test]
    fn device_projection_rules_cover_partial_success_and_zero_eligible() {
        let (database, path) = database_fixture();
        let (revision, node) = save_revision(
            &database,
            ActionKind::Wait,
            CompiledActionConfig::Wait { duration_ms: 10 },
        );
        let run = database
            .create_flow_run(&revision, selection(&["device-a", "device-b"]))
            .expect("create partial run");
        let a = database
            .create_flow_device_run(run.id, "device-a")
            .expect("device a");
        assert!(database
            .mark_device_terminal(
                a.id,
                &[FlowDeviceRunState::Queued],
                FlowDeviceRunState::Succeeded,
                None,
                release_proof("device-a"),
            )
            .is_err());
        ready_device(&database, a.id);
        let a_attempt = database
            .create_flow_attempt(a.id, &node, SideEffectClass::None, 1)
            .expect("device a attempt");
        set_attempt_state(&database, a_attempt.id, FlowAttemptState::Succeeded);
        database
            .mark_device_terminal(
                a.id,
                &[FlowDeviceRunState::Running],
                FlowDeviceRunState::Succeeded,
                None,
                release_proof("device-a"),
            )
            .expect("succeed a");
        assert!(database.recompute_run_projection(run.id).is_err());
        let b = database
            .create_flow_device_run(run.id, "device-b")
            .expect("device b after incomplete projection rollback");
        database
            .mark_device_terminal(
                b.id,
                &[FlowDeviceRunState::Queued],
                FlowDeviceRunState::Failed,
                Some(error("DeviceFailed", None)),
                release_proof("device-b"),
            )
            .expect("fail b");
        assert_eq!(
            database
                .recompute_run_projection(run.id)
                .expect("partial projection")
                .state,
            FlowAggregateState::Partial
        );

        let skipped_run = database
            .create_flow_run(
                &revision,
                FlowSelectionSnapshot {
                    requested: FlowTargetSelection::AllEligible,
                    target_udids: vec!["device-c".to_string()],
                },
            )
            .expect("create skipped run");
        let skipped = database
            .create_flow_device_run(skipped_run.id, "device-c")
            .expect("skipped device");
        database
            .mark_device_terminal(
                skipped.id,
                &[FlowDeviceRunState::Queued],
                FlowDeviceRunState::Skipped,
                None,
                release_proof("device-c"),
            )
            .expect("skip device");
        let projected = database
            .recompute_run_projection(skipped_run.id)
            .expect("zero eligible projection");
        assert_eq!(projected.state, FlowAggregateState::Failed);
        assert_eq!(projected.error.unwrap().code, "NoEligibleDevice");
        cleanup(&path);
    }

    #[test]
    fn skip_is_limited_to_unattempted_all_eligible_preflight() {
        let (database, path) = database_fixture();
        let (revision, node) = save_revision(
            &database,
            ActionKind::Wait,
            CompiledActionConfig::Wait { duration_ms: 10 },
        );

        let selected_run = database
            .create_flow_run(&revision, selection(&["selected-device"]))
            .expect("selected run");
        let selected = database
            .create_flow_device_run(selected_run.id, "selected-device")
            .expect("selected device");
        assert!(database
            .mark_device_terminal(
                selected.id,
                &[FlowDeviceRunState::Queued],
                FlowDeviceRunState::Skipped,
                None,
                release_proof("selected-device"),
            )
            .is_err());
        let now = now_text();
        database
            .conn()
            .expect("corruption connection")
            .execute(
                "UPDATE flow_device_runs SET state='skipped',release_proof_json=?2,
                    started_at=?3,finished_at=?3 WHERE id=?1",
                params![
                    selected.id.to_string(),
                    serde_json::to_string(&release_proof("selected-device"))
                        .expect("release proof"),
                    now,
                ],
            )
            .expect("seed selected device as skipped");
        assert!(database.get_flow_run(selected_run.id).is_err());
        assert!(database.recompute_run_projection(selected_run.id).is_err());

        let all_run = database
            .create_flow_run(
                &revision,
                FlowSelectionSnapshot {
                    requested: FlowTargetSelection::AllEligible,
                    target_udids: vec!["all-device".to_string()],
                },
            )
            .expect("all-eligible run");
        let all = database
            .create_flow_device_run(all_run.id, "all-device")
            .expect("all-eligible device");
        ready_device(&database, all.id);
        assert!(database
            .mark_device_terminal(
                all.id,
                &[FlowDeviceRunState::Running],
                FlowDeviceRunState::Skipped,
                None,
                release_proof("all-device"),
            )
            .is_err());

        database
            .conn()
            .expect("corruption connection")
            .execute(
                "UPDATE flow_device_runs SET state='queued',capability_snapshot_json=NULL,
                    started_at=NULL WHERE id=?1",
                [all.id.to_string()],
            )
            .expect("return fixture device to queued");
        database
            .conn()
            .expect("connection")
            .execute(
                "INSERT INTO flow_node_attempts(
                    id,device_run_id,node_id,action_kind,attempt_no,side_effect_class,state,
                    canonical_input_json,evidence_baseline_json,evidence_result_json,retry_safe,
                    error_json,started_at,updated_at,finished_at
                 ) VALUES(?1,?2,?3,'wait',1,'none','queued',NULL,NULL,NULL,0,NULL,NULL,?4,NULL)",
                params![
                    Uuid::new_v4().to_string(),
                    all.id.to_string(),
                    node.id.to_string(),
                    now_text(),
                ],
            )
            .expect("seed queued attempt");
        assert!(database
            .mark_device_terminal(
                all.id,
                &[FlowDeviceRunState::Queued],
                FlowDeviceRunState::Skipped,
                None,
                release_proof("all-device"),
            )
            .is_err());

        let eligible_run = database
            .create_flow_run(
                &revision,
                FlowSelectionSnapshot {
                    requested: FlowTargetSelection::AllEligible,
                    target_udids: vec!["preflight-device".to_string()],
                },
            )
            .expect("eligible run");
        let eligible = database
            .create_flow_device_run(eligible_run.id, "preflight-device")
            .expect("eligible device");
        database
            .transition_flow_device_run(
                eligible.id,
                FlowDeviceRunState::Queued,
                FlowDeviceRunState::Preflight,
                None,
            )
            .expect("enter preflight");
        database
            .mark_device_terminal(
                eligible.id,
                &[FlowDeviceRunState::Preflight],
                FlowDeviceRunState::Skipped,
                None,
                release_proof("preflight-device"),
            )
            .expect("skip unqualified all-eligible device");
        cleanup(&path);
    }

    #[test]
    fn startup_load_returns_every_and_only_nonterminal_attempt() {
        use FlowAttemptState::*;
        let nonterminal = [
            Queued,
            IntentCommitted,
            EffectDispatched,
            Verifying,
            Interrupted,
        ];
        let terminal = [
            Succeeded,
            FailedBeforeDispatch,
            FailedVerified,
            Uncertain,
            Cancelled,
        ];
        let (database, path) = database_fixture();
        let (revision, node) = save_revision(
            &database,
            ActionKind::Wait,
            CompiledActionConfig::Wait { duration_ms: 10 },
        );
        let udids = (0..nonterminal.len() + terminal.len())
            .map(|index| format!("fixture-udid-{index}"))
            .collect::<Vec<_>>();
        let run = database
            .create_flow_run(
                &revision,
                FlowSelectionSnapshot {
                    requested: FlowTargetSelection::Selected {
                        udids: udids.clone(),
                    },
                    target_udids: udids.clone(),
                },
            )
            .expect("run");
        for (index, state) in nonterminal.into_iter().chain(terminal).enumerate() {
            let device = database
                .create_flow_device_run(run.id, &udids[index])
                .expect("device");
            ready_device(&database, device.id);
            let attempt = database
                .create_flow_attempt(device.id, &node, SideEffectClass::None, 1)
                .expect("attempt");
            set_attempt_state(&database, attempt.id, state);
        }

        let loaded = database
            .load_nonterminal_attempts()
            .expect("load nonterminal");
        assert_eq!(
            loaded
                .iter()
                .map(|attempt| attempt.state)
                .collect::<BTreeSet<_>>(),
            nonterminal.into_iter().collect()
        );
        assert!(database.list_flow_runs(0).is_err());
        assert!(database.list_flow_runs(201).is_err());
        cleanup(&path);
    }
}
