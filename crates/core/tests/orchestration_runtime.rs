use std::collections::{BTreeMap, VecDeque};
use std::path::{Path, PathBuf};
use std::time::Duration;

use async_trait::async_trait;
use riviu_core::db::Database;
use riviu_core::{
    cancel_orchestration, compile_orchestration, execute_orchestration, AutomationDefinitionRecord,
    AutomationKind, AutomationProfileRef, ChildCampaignOutcome, OrchestrationAttemptState,
    OrchestrationBranch, OrchestrationCancelResult, OrchestrationChildCancelResult,
    OrchestrationChildDispatch, OrchestrationChildFailure, OrchestrationChildPort,
    OrchestrationChildRequest, OrchestrationChildStatus, OrchestrationDocumentV1,
    OrchestrationEdge, OrchestrationExecution, OrchestrationNode, OrchestrationNodeAction,
    OrchestrationPoint, OrchestrationRunState, ResolvedTargetDevice, ResolvedTargetSnapshot,
    TargetRef, ORCHESTRATION_SCHEMA_VERSION,
};
use serde_json::json;
use uuid::Uuid;

struct Fixture {
    database: Database,
    path: PathBuf,
    compiled: riviu_core::CompiledOrchestrationV1,
    target: ResolvedTargetSnapshot,
}

impl Fixture {
    fn new(action: OrchestrationNodeAction, config: serde_json::Value) -> Self {
        let path = std::env::temp_dir().join(format!("riviu-orch-runtime-{}.db", Uuid::new_v4()));
        let database = Database::open(&path).expect("open orchestration runtime fixture");
        let kind = match action {
            OrchestrationNodeAction::RunNurture { .. } => AutomationKind::Nurture,
            OrchestrationNodeAction::RunInteraction { .. } => AutomationKind::Interaction,
            OrchestrationNodeAction::RunPublish { .. } => AutomationKind::Publish,
            _ => AutomationKind::Interaction,
        };
        let config = if config.get("schemaVersion").is_some() {
            config
        } else {
            match kind {
                AutomationKind::Nurture => json!({
                    "schemaVersion": 1,
                    "settings": {}
                }),
                AutomationKind::Interaction => json!({
                    "schemaVersion": 1,
                    "request": {
                        "targets": [],
                        "messageCount": 1,
                        "instruction": config.to_string(),
                        "maxWords": 12,
                        "actions": { "like": true, "comment": false, "save": false }
                    }
                }),
                AutomationKind::Publish => json!({
                    "schemaVersion": 1,
                    "sourceRoot": "C:/fixture",
                    "bundleIds": ["bundle-a"],
                    "soundPolicy": { "kind": "default" },
                    "executionConfirmed": true
                }),
            }
        };
        let profile = database
            .create_automation_definition("Profile", kind, &TargetRef::All, &config)
            .expect("create profile");
        let action = pin_action(action, &profile);
        let document = document(action);
        let compiled = compile_orchestration(&document, &[profile]).expect("compile orchestration");
        database
            .save_orchestration_revision(None, &compiled)
            .expect("save orchestration");
        Self {
            database,
            path,
            compiled,
            target: target(&["phone-1", "phone-2"]),
        }
    }

    fn create_run(&self) -> Uuid {
        self.create_run_with_node_targets(&BTreeMap::new())
    }

    fn create_run_with_node_targets(
        &self,
        node_targets: &BTreeMap<Uuid, ResolvedTargetSnapshot>,
    ) -> Uuid {
        self.database
            .create_orchestration_run(
                self.compiled.document.id,
                self.compiled.document.revision,
                &self.target,
                node_targets,
            )
            .expect("create run")
            .id
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        cleanup(&self.path);
    }
}

fn profile_ref() -> AutomationProfileRef {
    AutomationProfileRef {
        definition_id: Uuid::nil(),
        revision: 1,
    }
}

fn pin_action(
    action: OrchestrationNodeAction,
    profile: &AutomationDefinitionRecord,
) -> OrchestrationNodeAction {
    let pinned = AutomationProfileRef {
        definition_id: profile.definition.id,
        revision: profile.revision.revision,
    };
    match action {
        OrchestrationNodeAction::RunNurture {
            target_override, ..
        } => OrchestrationNodeAction::RunNurture {
            profile: pinned,
            target_override,
        },
        OrchestrationNodeAction::RunInteraction {
            target_override, ..
        } => OrchestrationNodeAction::RunInteraction {
            profile: pinned,
            target_override,
        },
        OrchestrationNodeAction::RunPublish {
            target_override, ..
        } => OrchestrationNodeAction::RunPublish {
            profile: pinned,
            target_override,
        },
        other => other,
    }
}

fn interaction_action() -> OrchestrationNodeAction {
    OrchestrationNodeAction::RunInteraction {
        profile: profile_ref(),
        target_override: None,
    }
}

fn document(action: OrchestrationNodeAction) -> OrchestrationDocumentV1 {
    let start = Uuid::from_u128(101);
    let work = Uuid::from_u128(102);
    let end = Uuid::from_u128(103);
    let is_campaign = matches!(
        &action,
        OrchestrationNodeAction::RunNurture { .. }
            | OrchestrationNodeAction::RunInteraction { .. }
            | OrchestrationNodeAction::RunPublish { .. }
    );
    let mut edges = vec![
        OrchestrationEdge {
            source_node_id: start,
            source_port: OrchestrationBranch::Done,
            target_node_id: work,
        },
        OrchestrationEdge {
            source_node_id: work,
            source_port: OrchestrationBranch::Done,
            target_node_id: end,
        },
    ];
    if is_campaign {
        edges.extend(
            [
                OrchestrationBranch::Partial,
                OrchestrationBranch::Failed,
                OrchestrationBranch::Uncertain,
            ]
            .into_iter()
            .map(|source_port| OrchestrationEdge {
                source_node_id: work,
                source_port,
                target_node_id: end,
            }),
        );
    }
    OrchestrationDocumentV1 {
        schema_version: ORCHESTRATION_SCHEMA_VERSION,
        id: Uuid::new_v4(),
        revision: 1,
        name: "Ca fleet".into(),
        entry_node_id: start,
        nodes: vec![
            OrchestrationNode {
                id: start,
                position: OrchestrationPoint { x: 0.0, y: 0.0 },
                action: OrchestrationNodeAction::Start,
            },
            OrchestrationNode {
                id: work,
                position: OrchestrationPoint { x: 200.0, y: 0.0 },
                action,
            },
            OrchestrationNode {
                id: end,
                position: OrchestrationPoint { x: 400.0, y: 0.0 },
                action: OrchestrationNodeAction::End,
            },
        ],
        edges,
    }
}

fn target(udids: &[&str]) -> ResolvedTargetSnapshot {
    ResolvedTargetSnapshot {
        target_ref: TargetRef::All,
        included: udids
            .iter()
            .enumerate()
            .map(|(index, udid)| ResolvedTargetDevice {
                udid: (*udid).into(),
                alias: format!("Máy {}", index + 1),
                number: Some((index + 1) as u32),
            })
            .collect(),
        excluded: Vec::new(),
        roster_sha256: "a".repeat(64),
    }
}

fn cleanup(path: &Path) {
    for suffix in ["", "-wal", "-shm"] {
        let candidate = PathBuf::from(format!("{}{suffix}", path.display()));
        if candidate.exists() {
            std::fs::remove_file(candidate).expect("remove fixture");
        }
    }
}

#[derive(Default)]
struct Port {
    dispatch_results: VecDeque<Result<OrchestrationChildDispatch, OrchestrationChildFailure>>,
    reconcile_results: VecDeque<Result<OrchestrationChildStatus, String>>,
    cancel_results: VecDeque<Result<OrchestrationChildCancelResult, String>>,
    requests: Vec<OrchestrationChildRequest>,
    reconciled: Vec<Uuid>,
    cancelled: Vec<Uuid>,
    delays: Vec<Duration>,
    cancel_during_delay: Option<(PathBuf, Uuid)>,
    settle_during_cancel: Option<(PathBuf, Uuid, Uuid, OrchestrationBranch)>,
}

#[async_trait]
impl OrchestrationChildPort for Port {
    async fn dispatch_child(
        &mut self,
        request: OrchestrationChildRequest,
    ) -> Result<OrchestrationChildDispatch, OrchestrationChildFailure> {
        self.requests.push(request);
        self.dispatch_results
            .pop_front()
            .expect("dispatch result fixture")
    }

    async fn reconcile_child(
        &mut self,
        request: &OrchestrationChildRequest,
    ) -> Result<OrchestrationChildStatus, String> {
        self.reconciled.push(request.child_campaign_id);
        self.reconcile_results
            .pop_front()
            .expect("reconcile result fixture")
    }

    async fn cancel_child(
        &mut self,
        request: &OrchestrationChildRequest,
    ) -> Result<OrchestrationChildCancelResult, String> {
        self.cancelled.push(request.child_campaign_id);
        if let Some((path, attempt_id, child_id, branch)) = self.settle_during_cancel.take() {
            Database::open(path)
                .and_then(|database| {
                    database.settle_orchestration_child(attempt_id, child_id, branch, None)
                })
                .map_err(|error| error.to_string())?
                .ok_or_else(|| "concurrent child settlement lost its CAS".to_string())?;
        }
        self.cancel_results
            .pop_front()
            .expect("cancel result fixture")
    }

    async fn wait_delay(&mut self, duration: Duration) -> Result<(), String> {
        self.delays.push(duration);
        if let Some((path, run_id)) = self.cancel_during_delay.take() {
            Database::open(path)
                .and_then(|database| database.request_orchestration_cancel(run_id))
                .map_err(|error| error.to_string())?
                .ok_or_else(|| "run was not running during delay cancellation".to_string())?;
            return Err("delay interrupted by cancellation".into());
        }
        Ok(())
    }
}

#[tokio::test]
async fn one_interaction_node_dispatches_one_campaign_for_the_whole_fleet() {
    let fixture = Fixture::new(interaction_action(), json!({"operatorNote": "not copied"}));
    let run_id = fixture.create_run();
    let mut port = Port {
        dispatch_results: VecDeque::from([Ok(OrchestrationChildDispatch::Finished(
            ChildCampaignOutcome::Done,
        ))]),
        ..Port::default()
    };

    let result = execute_orchestration(&fixture.database, run_id, &mut port)
        .await
        .expect("execute orchestration");

    assert!(matches!(
        result,
        OrchestrationExecution::Complete(ref run) if run.state == OrchestrationRunState::Done
    ));
    assert_eq!(port.requests.len(), 1);
    assert_eq!(
        port.requests[0]
            .target
            .included
            .iter()
            .map(|device| device.udid.as_str())
            .collect::<Vec<_>>(),
        ["phone-1", "phone-2"]
    );
    let detail = fixture
        .database
        .get_orchestration_run(run_id)
        .expect("read run")
        .expect("run exists");
    assert_eq!(detail.attempts.len(), 3, "Start, campaign and End run once");
}

#[tokio::test]
async fn restart_reconciles_the_persisted_child_without_dispatching_again() {
    let fixture = Fixture::new(interaction_action(), json!({}));
    let run_id = fixture.create_run();
    let mut first_process = Port {
        dispatch_results: VecDeque::from([Ok(OrchestrationChildDispatch::Started)]),
        ..Port::default()
    };
    let first = execute_orchestration(&fixture.database, run_id, &mut first_process)
        .await
        .expect("start child");
    let child_id = match first {
        OrchestrationExecution::Waiting {
            child_campaign_id, ..
        } => child_campaign_id,
        other => panic!("expected waiting child, got {other:?}"),
    };

    let mut restarted_process = Port {
        reconcile_results: VecDeque::from([Ok(OrchestrationChildStatus::Finished(
            ChildCampaignOutcome::Done,
        ))]),
        ..Port::default()
    };
    let result = execute_orchestration(&fixture.database, run_id, &mut restarted_process)
        .await
        .expect("resume child");

    assert!(matches!(result, OrchestrationExecution::Complete(_)));
    assert!(restarted_process.requests.is_empty());
    assert_eq!(restarted_process.reconciled, [child_id]);
}

#[tokio::test]
async fn dispatch_failure_preserves_the_before_or_after_effect_boundary() {
    for (failure, expected_run, expected_branch) in [
        (
            OrchestrationChildFailure::before_effect("queue unavailable"),
            OrchestrationRunState::Failed,
            OrchestrationBranch::Failed,
        ),
        (
            OrchestrationChildFailure::after_effect("dispatch acknowledgement lost"),
            OrchestrationRunState::Uncertain,
            OrchestrationBranch::Uncertain,
        ),
    ] {
        let fixture = Fixture::new(interaction_action(), json!({}));
        let run_id = fixture.create_run();
        let mut port = Port {
            dispatch_results: VecDeque::from([Err(failure)]),
            ..Port::default()
        };

        let result = execute_orchestration(&fixture.database, run_id, &mut port)
            .await
            .expect("settle dispatch failure");
        assert!(matches!(
            result,
            OrchestrationExecution::Complete(ref run) if run.state == expected_run
        ));
        let detail = fixture
            .database
            .get_orchestration_run(run_id)
            .expect("read run")
            .expect("run exists");
        let campaign = detail
            .attempts
            .iter()
            .find(|attempt| attempt.child_campaign_id.is_some())
            .expect("campaign attempt");
        assert_eq!(campaign.branch, Some(expected_branch));
    }
}

#[tokio::test]
async fn every_child_outcome_selects_its_matching_branch_and_run_state() {
    for (outcome, expected) in [
        (ChildCampaignOutcome::Done, OrchestrationRunState::Done),
        (
            ChildCampaignOutcome::Partial,
            OrchestrationRunState::Partial,
        ),
        (ChildCampaignOutcome::Failed, OrchestrationRunState::Failed),
        (
            ChildCampaignOutcome::Uncertain,
            OrchestrationRunState::Uncertain,
        ),
    ] {
        let fixture = Fixture::new(interaction_action(), json!({}));
        let run_id = fixture.create_run();
        let mut port = Port {
            dispatch_results: VecDeque::from([Ok(OrchestrationChildDispatch::Finished(outcome))]),
            ..Port::default()
        };

        let result = execute_orchestration(&fixture.database, run_id, &mut port)
            .await
            .expect("execute child outcome");

        assert!(matches!(
            result,
            OrchestrationExecution::Complete(ref run) if run.state == expected
        ));
    }
}

#[tokio::test]
async fn target_override_uses_the_roster_persisted_at_run_confirmation() {
    let override_ref = TargetRef::Explicit {
        udids: vec!["phone-2".into()],
    };
    let fixture = Fixture::new(
        OrchestrationNodeAction::RunInteraction {
            profile: profile_ref(),
            target_override: Some(override_ref.clone()),
        },
        json!({}),
    );
    let overridden = ResolvedTargetSnapshot {
        target_ref: override_ref.clone(),
        included: vec![ResolvedTargetDevice {
            udid: "phone-2".into(),
            alias: "Máy 2".into(),
            number: Some(2),
        }],
        excluded: Vec::new(),
        roster_sha256: "b".repeat(64),
    };
    let node_targets =
        std::collections::BTreeMap::from([(Uuid::from_u128(102), overridden.clone())]);
    let run_id = fixture.create_run_with_node_targets(&node_targets);
    let mut first = Port {
        dispatch_results: VecDeque::from([Ok(OrchestrationChildDispatch::Started)]),
        ..Port::default()
    };

    execute_orchestration(&fixture.database, run_id, &mut first)
        .await
        .expect("start override child");
    assert_eq!(first.requests[0].target, overridden);

    let mut restarted = Port {
        reconcile_results: VecDeque::from([Ok(OrchestrationChildStatus::Finished(
            ChildCampaignOutcome::Done,
        ))]),
        ..Port::default()
    };
    execute_orchestration(&fixture.database, run_id, &mut restarted)
        .await
        .expect("reconcile override child");

    assert!(restarted.requests.is_empty());
}

#[tokio::test]
async fn cancel_only_finishes_when_the_child_proves_no_effect() {
    let fixture = Fixture::new(interaction_action(), json!({}));
    let run_id = fixture.create_run();
    let mut port = Port {
        dispatch_results: VecDeque::from([Ok(OrchestrationChildDispatch::Started)]),
        ..Port::default()
    };
    execute_orchestration(&fixture.database, run_id, &mut port)
        .await
        .expect("start child");
    port.cancel_results
        .push_back(Ok(OrchestrationChildCancelResult::CancelledBeforeEffect));

    let cancelled = cancel_orchestration(&fixture.database, run_id, &mut port)
        .await
        .expect("cancel orchestration");

    assert!(matches!(
        cancelled,
        OrchestrationCancelResult::Cancelled(ref run)
            if run.state == OrchestrationRunState::Cancelled
    ));
    assert_eq!(port.cancelled.len(), 1);

    let ambiguous_fixture = Fixture::new(interaction_action(), json!({}));
    let ambiguous_run = ambiguous_fixture.create_run();
    let mut ambiguous_port = Port {
        dispatch_results: VecDeque::from([Ok(OrchestrationChildDispatch::Started)]),
        ..Port::default()
    };
    execute_orchestration(
        &ambiguous_fixture.database,
        ambiguous_run,
        &mut ambiguous_port,
    )
    .await
    .expect("start ambiguous child");
    ambiguous_port
        .cancel_results
        .push_back(Ok(OrchestrationChildCancelResult::EffectMayHaveStarted));

    let pending = cancel_orchestration(
        &ambiguous_fixture.database,
        ambiguous_run,
        &mut ambiguous_port,
    )
    .await
    .expect("request ambiguous cancel");
    assert!(matches!(
        pending,
        OrchestrationCancelResult::ReconcileRequired { .. }
    ));
    assert_eq!(
        ambiguous_fixture
            .database
            .get_orchestration_run(ambiguous_run)
            .expect("read ambiguous run")
            .expect("run exists")
            .run
            .state,
        OrchestrationRunState::Running
    );
}

#[tokio::test]
async fn cancel_does_not_overwrite_a_child_that_already_finished() {
    let fixture = Fixture::new(interaction_action(), json!({}));
    let run_id = fixture.create_run();
    let mut port = Port {
        dispatch_results: VecDeque::from([Ok(OrchestrationChildDispatch::Started)]),
        ..Port::default()
    };
    execute_orchestration(&fixture.database, run_id, &mut port)
        .await
        .expect("start child");
    port.cancel_results
        .push_back(Ok(OrchestrationChildCancelResult::Finished(
            ChildCampaignOutcome::Partial,
        )));

    let cancel = cancel_orchestration(&fixture.database, run_id, &mut port)
        .await
        .expect("reconcile completed child during cancel");

    assert!(matches!(
        cancel,
        OrchestrationCancelResult::Cancelled(ref run)
            if run.state == OrchestrationRunState::Cancelled
    ));
    let detail = fixture
        .database
        .get_orchestration_run(run_id)
        .expect("read cancelled run")
        .expect("run exists");
    let child = detail
        .attempts
        .iter()
        .find(|attempt| attempt.child_campaign_id.is_some())
        .expect("child attempt");
    assert_eq!(child.branch, Some(OrchestrationBranch::Partial));
}

#[tokio::test]
async fn adapter_cancel_proof_cannot_overwrite_a_concurrent_child_settlement() {
    let fixture = Fixture::new(interaction_action(), json!({}));
    let run_id = fixture.create_run();
    let mut port = Port {
        dispatch_results: VecDeque::from([Ok(OrchestrationChildDispatch::Started)]),
        ..Port::default()
    };
    execute_orchestration(&fixture.database, run_id, &mut port)
        .await
        .expect("start child");
    let detail = fixture
        .database
        .get_orchestration_run(run_id)
        .expect("read running child")
        .expect("run exists");
    let attempt = detail
        .attempts
        .iter()
        .find(|attempt| attempt.child_campaign_id.is_some())
        .expect("armed attempt");
    let attempt_id = attempt.snapshot.attempt_id;
    let child_id = attempt.child_campaign_id.expect("child ID");
    port.settle_during_cancel = Some((
        fixture.path.clone(),
        attempt_id,
        child_id,
        OrchestrationBranch::Done,
    ));
    port.cancel_results
        .push_back(Ok(OrchestrationChildCancelResult::CancelledBeforeEffect));

    let result = cancel_orchestration(&fixture.database, run_id, &mut port)
        .await
        .expect("cancel races with child completion");
    assert!(matches!(
        result,
        OrchestrationCancelResult::Cancelled(ref run)
            if run.state == OrchestrationRunState::Cancelled
    ));
    let after = fixture
        .database
        .get_orchestration_run(run_id)
        .expect("read cancelled run")
        .expect("run exists");
    let settled = after
        .attempts
        .iter()
        .find(|attempt| attempt.snapshot.attempt_id == attempt_id)
        .expect("settled attempt");
    assert_eq!(settled.state, OrchestrationAttemptState::Done);
    assert_eq!(settled.branch, Some(OrchestrationBranch::Done));
    assert_eq!(settled.child_campaign_id, Some(child_id));
}

#[tokio::test]
async fn ambiguous_cancel_intent_survives_restart_and_blocks_the_next_node() {
    let fixture = Fixture::new(interaction_action(), json!({}));
    let run_id = fixture.create_run();
    let mut port = Port {
        dispatch_results: VecDeque::from([Ok(OrchestrationChildDispatch::Started)]),
        cancel_results: VecDeque::from([Ok(OrchestrationChildCancelResult::EffectMayHaveStarted)]),
        ..Port::default()
    };
    execute_orchestration(&fixture.database, run_id, &mut port)
        .await
        .expect("start child");
    let pending = cancel_orchestration(&fixture.database, run_id, &mut port)
        .await
        .expect("persist ambiguous cancel");
    assert!(matches!(
        pending,
        OrchestrationCancelResult::ReconcileRequired { .. }
    ));

    let mut restarted = Port {
        reconcile_results: VecDeque::from([Ok(OrchestrationChildStatus::Finished(
            ChildCampaignOutcome::Done,
        ))]),
        ..Port::default()
    };
    let result = execute_orchestration(&fixture.database, run_id, &mut restarted)
        .await
        .expect("reconcile after restart");

    assert!(matches!(
        result,
        OrchestrationExecution::Complete(ref run)
            if run.state == OrchestrationRunState::Cancelled
    ));
    assert!(restarted.requests.is_empty());
    let detail = fixture
        .database
        .get_orchestration_run(run_id)
        .expect("read cancelled run")
        .expect("run exists");
    assert_eq!(
        detail.attempts.len(),
        2,
        "End must not run after cancellation"
    );
}

#[test]
fn cancel_cas_refuses_to_overwrite_a_run_that_advanced_to_another_node() {
    let fixture = Fixture::new(interaction_action(), json!({}));
    let run_id = fixture.create_run();
    let start = Uuid::from_u128(101);
    let work = Uuid::from_u128(102);
    fixture
        .database
        .transition_orchestration_run(
            run_id,
            OrchestrationRunState::Queued,
            OrchestrationRunState::Running,
            Some(start),
            None,
        )
        .expect("start run")
        .expect("start owner");
    fixture
        .database
        .advance_orchestration_run(run_id, start, work)
        .expect("advance run")
        .expect("advance owner");

    assert!(fixture
        .database
        .cancel_orchestration_run(run_id, OrchestrationRunState::Running, Some(start))
        .expect("stale cancellation")
        .is_none());
    assert_eq!(
        fixture
            .database
            .get_orchestration_run(run_id)
            .expect("read run")
            .expect("run exists")
            .run
            .current_node_id,
        Some(work)
    );
}

#[tokio::test]
async fn missing_child_after_restart_fails_without_a_second_dispatch() {
    let fixture = Fixture::new(interaction_action(), json!({}));
    let run_id = fixture.create_run();
    let mut first = Port {
        dispatch_results: VecDeque::from([Ok(OrchestrationChildDispatch::Started)]),
        ..Port::default()
    };
    execute_orchestration(&fixture.database, run_id, &mut first)
        .await
        .expect("start child");
    let mut restarted = Port {
        reconcile_results: VecDeque::from([Ok(OrchestrationChildStatus::MissingBeforeEffect)]),
        ..Port::default()
    };

    let result = execute_orchestration(&fixture.database, run_id, &mut restarted)
        .await
        .expect("reconcile missing child");

    assert!(matches!(
        result,
        OrchestrationExecution::Complete(ref run)
            if run.state == OrchestrationRunState::Failed
    ));
    assert!(restarted.requests.is_empty());
    assert_eq!(restarted.reconciled.len(), 1);
}

#[tokio::test]
async fn delay_node_waits_once_before_advancing() {
    let fixture = Fixture::new(
        OrchestrationNodeAction::Delay { duration_ms: 1_250 },
        json!({}),
    );
    let run_id = fixture.create_run();
    let mut port = Port::default();

    let result = execute_orchestration(&fixture.database, run_id, &mut port)
        .await
        .expect("execute delay");

    assert!(matches!(result, OrchestrationExecution::Complete(_)));
    assert_eq!(port.delays, [Duration::from_millis(1_250)]);
}

#[tokio::test]
async fn cancellation_that_interrupts_delay_cancels_instead_of_failing() {
    let fixture = Fixture::new(
        OrchestrationNodeAction::Delay {
            duration_ms: 86_400_000,
        },
        json!({}),
    );
    let run_id = fixture.create_run();
    let mut port = Port {
        cancel_during_delay: Some((fixture.path.clone(), run_id)),
        ..Port::default()
    };

    let result = execute_orchestration(&fixture.database, run_id, &mut port)
        .await
        .expect("cancel interrupted delay");

    assert!(matches!(
        result,
        OrchestrationExecution::Complete(ref run)
            if run.state == OrchestrationRunState::Cancelled
    ));
    let detail = fixture
        .database
        .get_orchestration_run(run_id)
        .expect("read cancelled delay run")
        .expect("run exists");
    let delay = detail
        .attempts
        .iter()
        .find(|attempt| attempt.snapshot.node_id == Uuid::from_u128(102))
        .expect("delay attempt");
    assert_eq!(
        delay.state,
        riviu_core::OrchestrationAttemptState::Cancelled
    );
    assert_eq!(delay.branch, None);
}

#[tokio::test]
async fn queued_cancel_intent_wins_the_start_transition() {
    let fixture = Fixture::new(
        OrchestrationNodeAction::Delay { duration_ms: 1_000 },
        json!({}),
    );
    let run_id = fixture.create_run();
    fixture
        .database
        .request_orchestration_cancel(run_id)
        .expect("persist queued cancellation")
        .expect("queued run accepted cancellation");

    assert!(fixture
        .database
        .transition_orchestration_run(
            run_id,
            OrchestrationRunState::Queued,
            OrchestrationRunState::Running,
            Some(Uuid::from_u128(101)),
            None,
        )
        .expect("attempt stale start")
        .is_none());

    let mut port = Port::default();
    let result = execute_orchestration(&fixture.database, run_id, &mut port)
        .await
        .expect("honor queued cancellation");
    assert!(matches!(
        result,
        OrchestrationExecution::Complete(ref run)
            if run.state == OrchestrationRunState::Cancelled
    ));
    assert!(port.delays.is_empty());
}

#[tokio::test]
async fn profile_config_never_enters_child_requests_or_run_snapshots() {
    let fixture = Fixture::new(
        interaction_action(),
        json!({"operatorNote": "SENSITIVE-MARKER", "apiMode": "credential-manager"}),
    );
    let run_id = fixture.create_run();
    let mut port = Port {
        dispatch_results: VecDeque::from([Ok(OrchestrationChildDispatch::Finished(
            ChildCampaignOutcome::Done,
        ))]),
        ..Port::default()
    };

    execute_orchestration(&fixture.database, run_id, &mut port)
        .await
        .expect("execute orchestration");

    let detail = fixture
        .database
        .get_orchestration_run(run_id)
        .expect("read run")
        .expect("run exists");
    let persisted = serde_json::to_string(&detail).expect("serialize run detail");
    assert!(!persisted.contains("SENSITIVE-MARKER"));
    assert!(!persisted.contains("credential-manager"));
    let request = serde_json::to_string(&port.requests).expect("serialize child requests");
    assert!(!request.contains("SENSITIVE-MARKER"));
    assert!(!request.contains("credential-manager"));
}

#[tokio::test]
async fn archived_pinned_profile_fails_before_child_dispatch() {
    let fixture = Fixture::new(interaction_action(), json!({}));
    let run_id = fixture.create_run();
    let profile_id = fixture
        .compiled
        .document
        .nodes
        .iter()
        .find_map(|node| match &node.action {
            OrchestrationNodeAction::RunNurture { profile, .. }
            | OrchestrationNodeAction::RunInteraction { profile, .. }
            | OrchestrationNodeAction::RunPublish { profile, .. } => Some(profile.definition_id),
            _ => None,
        })
        .expect("pinned profile");
    fixture
        .database
        .archive_automation_definition(profile_id)
        .expect("archive pinned profile after orchestration save");
    let mut port = Port::default();

    let result = execute_orchestration(&fixture.database, run_id, &mut port)
        .await
        .expect("fail closed before child dispatch");

    assert!(matches!(
        result,
        OrchestrationExecution::Complete(ref run)
            if run.state == OrchestrationRunState::Failed
    ));
    assert!(port.requests.is_empty());
    let detail = fixture
        .database
        .get_orchestration_run(run_id)
        .expect("read run")
        .expect("run exists");
    let campaign = detail
        .attempts
        .iter()
        .find(|attempt| attempt.snapshot.profile.is_some())
        .expect("campaign attempt");
    assert_eq!(campaign.branch, Some(OrchestrationBranch::Failed));
    assert_eq!(campaign.error_code.as_deref(), Some("profile_unavailable"));
    assert!(campaign.child_campaign_id.is_none());
}
