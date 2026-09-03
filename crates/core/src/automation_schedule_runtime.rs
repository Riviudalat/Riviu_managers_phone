//! Restart-safe execution of one claimed automation schedule occurrence.
//!
//! The scheduler persists the occurrence, child ID, idempotency key, pinned profile and target
//! before this executor runs. This module owns only the occurrence state machine: once the queued
//! CAS wins, every later invocation reconciles that exact child and never dispatches another.

use anyhow::Context;
use uuid::Uuid;

use crate::db::Database;
use crate::{
    AutomationChildOwner, AutomationScheduleOccurrence, AutomationScheduleOccurrenceState,
    ChildCampaignOutcome, OrchestrationChildDispatch, OrchestrationChildFailurePhase,
    OrchestrationChildPort, OrchestrationChildRequest, OrchestrationChildStatus,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AutomationScheduleExecution {
    Complete(AutomationScheduleOccurrence),
    Waiting(AutomationScheduleOccurrence),
}

pub async fn execute_automation_schedule_occurrence<P: OrchestrationChildPort>(
    database: &Database,
    occurrence_id: Uuid,
    port: &mut P,
) -> anyhow::Result<AutomationScheduleExecution> {
    let occurrence = database
        .get_automation_schedule_occurrence(occurrence_id)?
        .context("automation schedule occurrence does not exist")?;
    if occurrence.state.is_terminal() {
        return Ok(AutomationScheduleExecution::Complete(occurrence));
    }

    if occurrence.state == AutomationScheduleOccurrenceState::Queued {
        let Some(dispatching) =
            database.mark_automation_schedule_occurrence_dispatching(occurrence_id)?
        else {
            return classify_current(database, occurrence_id);
        };
        let request = child_request(&dispatching)?;
        return match port.dispatch_child(request).await {
            Ok(OrchestrationChildDispatch::Started) => {
                let Some(running) =
                    database.mark_automation_schedule_occurrence_running(occurrence_id)?
                else {
                    return classify_current(database, occurrence_id);
                };
                Ok(AutomationScheduleExecution::Waiting(running))
            }
            Ok(OrchestrationChildDispatch::Finished(outcome)) => {
                settle(database, occurrence_id, outcome, None)
            }
            Err(error) => {
                let (outcome, code) = match error.phase {
                    OrchestrationChildFailurePhase::BeforeEffect => {
                        (ChildCampaignOutcome::Failed, "child_dispatch_failed")
                    }
                    OrchestrationChildFailurePhase::AfterEffect => {
                        (ChildCampaignOutcome::Uncertain, "child_dispatch_uncertain")
                    }
                };
                settle(database, occurrence_id, outcome, Some(code))
            }
        };
    }

    let request = child_request(&occurrence)?;
    match port
        .reconcile_child(&request)
        .await
        .map_err(anyhow::Error::msg)?
    {
        OrchestrationChildStatus::Running => {
            if occurrence.state == AutomationScheduleOccurrenceState::Dispatching {
                let Some(running) =
                    database.mark_automation_schedule_occurrence_running(occurrence_id)?
                else {
                    return classify_current(database, occurrence_id);
                };
                Ok(AutomationScheduleExecution::Waiting(running))
            } else {
                Ok(AutomationScheduleExecution::Waiting(occurrence))
            }
        }
        OrchestrationChildStatus::Finished(outcome) => {
            settle(database, occurrence_id, outcome, None)
        }
        OrchestrationChildStatus::MissingBeforeEffect => settle(
            database,
            occurrence_id,
            ChildCampaignOutcome::Failed,
            Some("child_missing_before_effect"),
        ),
    }
}

fn child_request(
    occurrence: &AutomationScheduleOccurrence,
) -> anyhow::Result<OrchestrationChildRequest> {
    Ok(OrchestrationChildRequest {
        owner: AutomationChildOwner::ScheduleOccurrence {
            occurrence_id: occurrence.id,
        },
        idempotency_key: occurrence.idempotency_key.clone(),
        child_campaign_id: occurrence.child_campaign_id,
        kind: occurrence.kind,
        profile: occurrence.profile.clone(),
        target: occurrence
            .target
            .clone()
            .context("executable automation schedule occurrence has no target snapshot")?,
    })
}

fn settle(
    database: &Database,
    occurrence_id: Uuid,
    outcome: ChildCampaignOutcome,
    error_code: Option<&str>,
) -> anyhow::Result<AutomationScheduleExecution> {
    match database.settle_automation_schedule_occurrence(occurrence_id, outcome, error_code)? {
        Some(occurrence) => Ok(AutomationScheduleExecution::Complete(occurrence)),
        None => classify_current(database, occurrence_id),
    }
}

fn classify_current(
    database: &Database,
    occurrence_id: Uuid,
) -> anyhow::Result<AutomationScheduleExecution> {
    let occurrence = database
        .get_automation_schedule_occurrence(occurrence_id)?
        .context("automation schedule occurrence disappeared during transition")?;
    if occurrence.state.is_terminal() {
        Ok(AutomationScheduleExecution::Complete(occurrence))
    } else {
        Ok(AutomationScheduleExecution::Waiting(occurrence))
    }
}
