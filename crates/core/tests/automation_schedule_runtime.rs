use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Duration as ChronoDuration};
use riviu_core::db::Database;
use riviu_core::{
    execute_automation_schedule_occurrence, AutomationChildOwner, AutomationKind,
    AutomationScheduleExecution, AutomationScheduleKind, AutomationScheduleOccurrence,
    AutomationScheduleOccurrenceState, AutomationScheduleV1, ChildCampaignOutcome,
    OrchestrationChildCancelResult, OrchestrationChildDispatch, OrchestrationChildFailure,
    OrchestrationChildPort, OrchestrationChildRequest, OrchestrationChildStatus,
    ResolvedTargetDevice, ResolvedTargetSnapshot, TargetRef,
};
use serde_json::json;
use uuid::Uuid;

#[derive(Clone)]
struct ChildService {
    inner: Arc<Mutex<ChildServiceState>>,
}

struct ChildServiceState {
    dispatch_count: usize,
    dispatched: Option<OrchestrationChildRequest>,
    reconciled: Vec<OrchestrationChildRequest>,
    dispatch_result: Result<OrchestrationChildDispatch, OrchestrationChildFailure>,
    reconcile_result: Result<OrchestrationChildStatus, String>,
}

impl ChildService {
    fn started() -> Self {
        Self {
            inner: Arc::new(Mutex::new(ChildServiceState {
                dispatch_count: 0,
                dispatched: None,
                reconciled: Vec::new(),
                dispatch_result: Ok(OrchestrationChildDispatch::Started),
                reconcile_result: Ok(OrchestrationChildStatus::Running),
            })),
        }
    }

    fn port(&self) -> FakePort {
        FakePort {
            service: self.clone(),
        }
    }

    fn set_reconcile(&self, status: OrchestrationChildStatus) {
        self.inner.lock().unwrap().reconcile_result = Ok(status);
    }

    fn set_dispatch_failure(&self, failure: OrchestrationChildFailure) {
        self.inner.lock().unwrap().dispatch_result = Err(failure);
    }
}

struct FakePort {
    service: ChildService,
}

#[async_trait]
impl OrchestrationChildPort for FakePort {
    async fn dispatch_child(
        &mut self,
        request: OrchestrationChildRequest,
    ) -> Result<OrchestrationChildDispatch, OrchestrationChildFailure> {
        let mut state = self.service.inner.lock().unwrap();
        state.dispatch_count += 1;
        state.dispatched = Some(request);
        state.dispatch_result.clone()
    }

    async fn reconcile_child(
        &mut self,
        request: &OrchestrationChildRequest,
    ) -> Result<OrchestrationChildStatus, String> {
        let mut state = self.service.inner.lock().unwrap();
        state.reconciled.push(request.clone());
        state.reconcile_result.clone()
    }

    async fn cancel_child(
        &mut self,
        _request: &OrchestrationChildRequest,
    ) -> Result<OrchestrationChildCancelResult, String> {
        unreachable!("the occurrence executor does not cancel children")
    }

    async fn wait_delay(&mut self, _duration: Duration) -> Result<(), String> {
        unreachable!("the occurrence executor has no delay nodes")
    }
}

struct Fixture {
    database: Database,
    path: std::path::PathBuf,
    occurrence: AutomationScheduleOccurrence,
}

impl Fixture {
    fn new(label: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "riviu-automation-schedule-runtime-{label}-{}.db",
            Uuid::new_v4()
        ));
        let database = Database::open(&path).expect("open database");
        let definition = database
            .create_automation_definition(
                "Nurture",
                AutomationKind::Nurture,
                &TargetRef::All,
                &json!({
                    "schemaVersion": 1,
                    "settings": { "likeProb": 0 }
                }),
            )
            .expect("create profile");
        let schedule = database
            .create_automation_schedule(
                "Every 15 minutes",
                definition.definition.id,
                definition.revision.revision,
                true,
                &AutomationScheduleV1 {
                    schema_version: 1,
                    kind: AutomationScheduleKind::Interval,
                    every_minutes: 15,
                },
            )
            .expect("create schedule");
        let due = schedule.next_due_at.expect("enabled schedule is armed");
        let now = (DateTime::parse_from_rfc3339(&due).expect("due timestamp")
            + ChronoDuration::seconds(1))
        .to_rfc3339();
        let target = ResolvedTargetSnapshot {
            target_ref: TargetRef::All,
            included: vec![ResolvedTargetDevice {
                udid: "phone-1".into(),
                alias: "Phone 1".into(),
                number: Some(1),
            }],
            excluded: Vec::new(),
            roster_sha256: "a".repeat(64),
        };
        let occurrence = database
            .claim_automation_schedule_occurrence(
                schedule.id,
                schedule.revision,
                &due,
                &now,
                Some(&target),
                None,
            )
            .expect("claim due slot")
            .expect("win due slot");
        Self {
            database,
            path,
            occurrence,
        }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        for suffix in ["", "-wal", "-shm"] {
            std::fs::remove_file(format!("{}{suffix}", self.path.display())).ok();
        }
    }
}

fn completed(execution: AutomationScheduleExecution) -> AutomationScheduleOccurrence {
    match execution {
        AutomationScheduleExecution::Complete(occurrence) => occurrence,
        AutomationScheduleExecution::Waiting(occurrence) => {
            panic!("expected complete occurrence, got {:?}", occurrence.state)
        }
    }
}

#[tokio::test]
async fn restart_reconciles_the_same_child_and_never_dispatches_a_duplicate() {
    let fixture = Fixture::new("restart");
    let service = ChildService::started();

    let first = execute_automation_schedule_occurrence(
        &fixture.database,
        fixture.occurrence.id,
        &mut service.port(),
    )
    .await
    .expect("dispatch occurrence");
    let AutomationScheduleExecution::Waiting(running) = first else {
        panic!("started child must be waiting");
    };
    assert_eq!(running.state, AutomationScheduleOccurrenceState::Running);

    service.set_reconcile(OrchestrationChildStatus::Finished(
        ChildCampaignOutcome::Done,
    ));
    let after_restart = execute_automation_schedule_occurrence(
        &fixture.database,
        fixture.occurrence.id,
        &mut service.port(),
    )
    .await
    .expect("reconcile after restart");
    let done = completed(after_restart);
    assert_eq!(done.state, AutomationScheduleOccurrenceState::Done);

    let state = service.inner.lock().unwrap();
    assert_eq!(state.dispatch_count, 1);
    assert_eq!(state.reconciled.len(), 1);
    let dispatched = state.dispatched.as_ref().expect("dispatch request");
    let reconciled = &state.reconciled[0];
    assert_eq!(
        dispatched.owner,
        AutomationChildOwner::ScheduleOccurrence {
            occurrence_id: fixture.occurrence.id
        }
    );
    assert_eq!(reconciled.owner, dispatched.owner);
    assert_eq!(reconciled.child_campaign_id, dispatched.child_campaign_id);
    assert_eq!(reconciled.idempotency_key, dispatched.idempotency_key);
    assert_eq!(
        dispatched.child_campaign_id,
        fixture.occurrence.child_campaign_id
    );
    assert_eq!(
        dispatched.idempotency_key,
        fixture.occurrence.idempotency_key
    );
}

#[tokio::test]
async fn dispatching_and_running_recovery_only_reconcile() {
    for initial in [
        AutomationScheduleOccurrenceState::Dispatching,
        AutomationScheduleOccurrenceState::Running,
    ] {
        let fixture = Fixture::new(&format!("recover-{initial:?}"));
        fixture
            .database
            .mark_automation_schedule_occurrence_dispatching(fixture.occurrence.id)
            .expect("mark dispatching")
            .expect("queued CAS");
        if initial == AutomationScheduleOccurrenceState::Running {
            fixture
                .database
                .mark_automation_schedule_occurrence_running(fixture.occurrence.id)
                .expect("mark running")
                .expect("dispatching CAS");
        }
        let service = ChildService::started();
        service.set_reconcile(OrchestrationChildStatus::Finished(
            ChildCampaignOutcome::Partial,
        ));

        let result = execute_automation_schedule_occurrence(
            &fixture.database,
            fixture.occurrence.id,
            &mut service.port(),
        )
        .await
        .expect("recover occurrence");
        assert_eq!(
            completed(result).state,
            AutomationScheduleOccurrenceState::Partial
        );
        let state = service.inner.lock().unwrap();
        assert_eq!(state.dispatch_count, 0, "{initial:?} redispatched");
        assert_eq!(state.reconciled.len(), 1);
    }
}

#[tokio::test]
async fn dispatch_failure_phase_selects_failed_or_uncertain() {
    for (failure, expected) in [
        (
            OrchestrationChildFailure::before_effect("preflight refused"),
            AutomationScheduleOccurrenceState::Failed,
        ),
        (
            OrchestrationChildFailure::after_effect("acknowledgement lost"),
            AutomationScheduleOccurrenceState::Uncertain,
        ),
    ] {
        let fixture = Fixture::new(&format!("failure-{expected:?}"));
        let service = ChildService::started();
        service.set_dispatch_failure(failure);

        let result = execute_automation_schedule_occurrence(
            &fixture.database,
            fixture.occurrence.id,
            &mut service.port(),
        )
        .await
        .expect("settle dispatch failure");
        assert_eq!(completed(result).state, expected);
        assert_eq!(service.inner.lock().unwrap().dispatch_count, 1);
    }
}

#[tokio::test]
async fn every_child_outcome_maps_to_the_matching_terminal_occurrence() {
    for (outcome, expected) in [
        (
            ChildCampaignOutcome::Done,
            AutomationScheduleOccurrenceState::Done,
        ),
        (
            ChildCampaignOutcome::Partial,
            AutomationScheduleOccurrenceState::Partial,
        ),
        (
            ChildCampaignOutcome::Failed,
            AutomationScheduleOccurrenceState::Failed,
        ),
        (
            ChildCampaignOutcome::Uncertain,
            AutomationScheduleOccurrenceState::Uncertain,
        ),
    ] {
        let fixture = Fixture::new(&format!("outcome-{expected:?}"));
        let service = ChildService::started();
        service.inner.lock().unwrap().dispatch_result =
            Ok(OrchestrationChildDispatch::Finished(outcome));

        let result = execute_automation_schedule_occurrence(
            &fixture.database,
            fixture.occurrence.id,
            &mut service.port(),
        )
        .await
        .expect("settle immediate child outcome");
        assert_eq!(completed(result).state, expected);
    }
}
