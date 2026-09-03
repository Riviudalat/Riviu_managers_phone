//! Restart-safe execution for fleet orchestration documents.
//!
//! This runtime dispatches one child campaign per campaign node. It records the child ID before
//! calling the adapter and reconstructs every later operation from the persisted attempt, so a
//! restart can only reconcile the same child, never enqueue a replacement.

use std::time::Duration;

use anyhow::{ensure, Context};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::db::Database;
use crate::{
    branch_for_child_outcome, AutomationKind, AutomationProfileRef, ChildCampaignOutcome,
    CompiledOrchestrationV1, OrchestrationAttemptRecord, OrchestrationAttemptState,
    OrchestrationBranch, OrchestrationNode, OrchestrationNodeAction, OrchestrationRunRecord,
    OrchestrationRunState, ResolvedTargetSnapshot,
};

const CANCEL_REQUESTED: &str = "cancel_requested";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OrchestrationChildRequest {
    pub owner: AutomationChildOwner,
    pub idempotency_key: String,
    pub child_campaign_id: Uuid,
    pub kind: AutomationKind,
    pub profile: AutomationProfileRef,
    pub target: ResolvedTargetSnapshot,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum AutomationChildOwner {
    OrchestrationAttempt { attempt_id: Uuid },
    ScheduleOccurrence { occurrence_id: Uuid },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OrchestrationChildDispatch {
    Started,
    Finished(ChildCampaignOutcome),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OrchestrationChildStatus {
    Running,
    Finished(ChildCampaignOutcome),
    MissingBeforeEffect,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OrchestrationChildCancelResult {
    CancelledBeforeEffect,
    EffectMayHaveStarted,
    Finished(ChildCampaignOutcome),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OrchestrationChildFailurePhase {
    BeforeEffect,
    AfterEffect,
}

#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
#[error("{message}")]
pub struct OrchestrationChildFailure {
    pub phase: OrchestrationChildFailurePhase,
    pub message: String,
}

impl OrchestrationChildFailure {
    pub fn before_effect(message: impl Into<String>) -> Self {
        Self {
            phase: OrchestrationChildFailurePhase::BeforeEffect,
            message: message.into(),
        }
    }

    pub fn after_effect(message: impl Into<String>) -> Self {
        Self {
            phase: OrchestrationChildFailurePhase::AfterEffect,
            message: message.into(),
        }
    }
}

/// Boundary to the existing Nurture, Interaction, and Publish campaign services.
///
/// `dispatch_child` must create exactly `child_campaign_id`, use `idempotency_key`, and receive
/// the whole target snapshot in one call. It must not fan this request out by calling the
/// campaign constructor once per device. `reconcile_child` and `cancel_child` address that exact
/// persisted ID and must never synthesize another campaign.
#[async_trait]
pub trait OrchestrationChildPort: Send {
    async fn dispatch_child(
        &mut self,
        request: OrchestrationChildRequest,
    ) -> Result<OrchestrationChildDispatch, OrchestrationChildFailure>;

    async fn reconcile_child(
        &mut self,
        request: &OrchestrationChildRequest,
    ) -> Result<OrchestrationChildStatus, String>;

    async fn cancel_child(
        &mut self,
        request: &OrchestrationChildRequest,
    ) -> Result<OrchestrationChildCancelResult, String>;

    async fn wait_delay(&mut self, duration: Duration) -> Result<(), String>;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum OrchestrationExecution {
    Complete(OrchestrationRunRecord),
    Waiting {
        run: OrchestrationRunRecord,
        attempt_id: Uuid,
        child_campaign_id: Uuid,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum OrchestrationCancelResult {
    Cancelled(OrchestrationRunRecord),
    AlreadyTerminal(OrchestrationRunRecord),
    ReconcileRequired {
        run: OrchestrationRunRecord,
        attempt_id: Uuid,
        child_campaign_id: Uuid,
    },
}

pub async fn execute_orchestration<P: OrchestrationChildPort>(
    database: &Database,
    run_id: Uuid,
    port: &mut P,
) -> anyhow::Result<OrchestrationExecution> {
    let mut detail = database
        .get_orchestration_run(run_id)?
        .context("orchestration run does not exist")?;
    if detail.run.state.is_terminal() {
        return Ok(OrchestrationExecution::Complete(detail.run));
    }
    let revision = database
        .get_orchestration_revision(detail.run.document_id, Some(detail.run.document_revision))?
        .context("orchestration run revision is missing")?;
    ensure!(
        detail.run.document_sha256 == revision.compiled.sha256,
        "orchestration run revision hash changed"
    );
    let compiled = revision.compiled;

    if detail.run.state == OrchestrationRunState::Queued {
        if detail.run.error_code.as_deref() == Some(CANCEL_REQUESTED) {
            let run = database
                .cancel_orchestration_run(run_id, OrchestrationRunState::Queued, None)?
                .context("orchestration changed while cancelling before start")?;
            return Ok(OrchestrationExecution::Complete(run));
        }
        let started = database.transition_orchestration_run(
            run_id,
            OrchestrationRunState::Queued,
            OrchestrationRunState::Running,
            Some(compiled.document.entry_node_id),
            None,
        )?;
        if started.is_none() {
            detail = database
                .get_orchestration_run(run_id)?
                .context("orchestration run disappeared while starting")?;
            if detail.run.state.is_terminal() {
                return Ok(OrchestrationExecution::Complete(detail.run));
            }
            if detail.run.state == OrchestrationRunState::Queued
                && detail.run.error_code.as_deref() == Some(CANCEL_REQUESTED)
            {
                let run = database
                    .cancel_orchestration_run(run_id, OrchestrationRunState::Queued, None)?
                    .context("orchestration changed while cancelling at startup")?;
                return Ok(OrchestrationExecution::Complete(run));
            }
        }
    }

    // The compiler caps a graph at 100 acyclic nodes. A larger traversal means concurrent
    // mutation or corrupt persisted data, not a graph that should keep dispatching work.
    for _ in 0..=compiled.document.nodes.len() {
        detail = database
            .get_orchestration_run(run_id)?
            .context("orchestration run disappeared")?;
        if detail.run.state.is_terminal() {
            return Ok(OrchestrationExecution::Complete(detail.run));
        }
        ensure!(
            detail.run.state == OrchestrationRunState::Running,
            "orchestration run is not executable"
        );
        let node_id = detail
            .run
            .current_node_id
            .context("running orchestration has no current node")?;
        let node = node(&compiled, node_id)?;
        let existing = detail
            .attempts
            .iter()
            .find(|attempt| attempt.snapshot.node_id == node_id)
            .cloned();
        if detail.run.error_code.as_deref() == Some(CANCEL_REQUESTED) && existing.is_none() {
            let run = database
                .cancel_orchestration_run(run_id, OrchestrationRunState::Running, Some(node_id))?
                .context("orchestration changed while completing cancellation")?;
            return Ok(OrchestrationExecution::Complete(run));
        }
        let attempt = match existing {
            Some(attempt) => attempt,
            None => {
                let target = if node.action.target_override().is_some() {
                    detail
                        .run
                        .node_targets
                        .get(&node_id)
                        .cloned()
                        .context("orchestration node has no confirmed target snapshot")?
                } else {
                    detail.run.target.clone()
                };
                let attempt_id = Uuid::new_v4();
                let snapshot =
                    crate::snapshot_orchestration_attempt(&compiled, node_id, attempt_id, target)?;
                database.create_orchestration_attempt(
                    run_id,
                    u32::try_from(detail.attempts.len() + 1)
                        .context("orchestration attempt count overflow")?,
                    &snapshot,
                )?
            }
        };

        if attempt.state.is_terminal() {
            if attempt.state == OrchestrationAttemptState::Cancelled {
                let run = database
                    .cancel_orchestration_run(
                        run_id,
                        OrchestrationRunState::Running,
                        Some(node_id),
                    )?
                    .or_else(|| {
                        database
                            .get_orchestration_run(run_id)
                            .ok()
                            .flatten()
                            .map(|d| d.run)
                    })
                    .context("cancelled orchestration run disappeared")?;
                return Ok(OrchestrationExecution::Complete(run));
            }
            let branch = attempt.branch.context("terminal attempt has no branch")?;
            if detail.run.error_code.as_deref() == Some(CANCEL_REQUESTED) {
                return finish_cancel_request(database, run_id, node_id, branch)
                    .map(OrchestrationExecution::Complete);
            }
            if matches!(node.action, OrchestrationNodeAction::End) {
                return finish_run(database, run_id, node_id).map(OrchestrationExecution::Complete);
            }
            route(database, run_id, &compiled, node_id, branch)?;
            continue;
        }

        match &node.action {
            OrchestrationNodeAction::Start => {
                if detail.run.error_code.as_deref() == Some(CANCEL_REQUESTED) {
                    database
                        .cancel_orchestration_attempt_before_effect(attempt.snapshot.attempt_id)?;
                    let run = database
                        .cancel_orchestration_run(
                            run_id,
                            OrchestrationRunState::Running,
                            Some(node_id),
                        )?
                        .context("orchestration changed while cancelling Start")?;
                    return Ok(OrchestrationExecution::Complete(run));
                }
                settle_internal(
                    database,
                    attempt.snapshot.attempt_id,
                    OrchestrationBranch::Done,
                    None,
                )?;
                route(
                    database,
                    run_id,
                    &compiled,
                    node_id,
                    OrchestrationBranch::Done,
                )?;
            }
            OrchestrationNodeAction::Delay { duration_ms } => {
                if detail.run.error_code.as_deref() == Some(CANCEL_REQUESTED) {
                    database
                        .cancel_orchestration_attempt_before_effect(attempt.snapshot.attempt_id)?;
                    let run = database
                        .cancel_orchestration_run(
                            run_id,
                            OrchestrationRunState::Running,
                            Some(node_id),
                        )?
                        .context("orchestration changed while cancelling Delay")?;
                    return Ok(OrchestrationExecution::Complete(run));
                }
                let delay = port.wait_delay(Duration::from_millis(*duration_ms)).await;
                let refreshed = database
                    .get_orchestration_run(run_id)?
                    .context("orchestration run disappeared after Delay")?;
                if refreshed.run.error_code.as_deref() == Some(CANCEL_REQUESTED) {
                    database
                        .cancel_orchestration_attempt_before_effect(attempt.snapshot.attempt_id)?;
                    let run = database
                        .cancel_orchestration_run(
                            run_id,
                            OrchestrationRunState::Running,
                            Some(node_id),
                        )?
                        .context("orchestration changed while interrupting Delay")?;
                    return Ok(OrchestrationExecution::Complete(run));
                }
                if let Err(_error) = delay {
                    settle_internal(
                        database,
                        attempt.snapshot.attempt_id,
                        OrchestrationBranch::Failed,
                        Some("delay_failed"),
                    )?;
                    return terminal_run(
                        database,
                        run_id,
                        node_id,
                        OrchestrationRunState::Failed,
                        Some("delay_failed"),
                    )
                    .map(OrchestrationExecution::Complete);
                }
                settle_internal(
                    database,
                    attempt.snapshot.attempt_id,
                    OrchestrationBranch::Done,
                    None,
                )?;
                route(
                    database,
                    run_id,
                    &compiled,
                    node_id,
                    OrchestrationBranch::Done,
                )?;
            }
            OrchestrationNodeAction::End => {
                if detail.run.error_code.as_deref() == Some(CANCEL_REQUESTED) {
                    database
                        .cancel_orchestration_attempt_before_effect(attempt.snapshot.attempt_id)?;
                    let run = database
                        .cancel_orchestration_run(
                            run_id,
                            OrchestrationRunState::Running,
                            Some(node_id),
                        )?
                        .context("orchestration changed while cancelling End")?;
                    return Ok(OrchestrationExecution::Complete(run));
                }
                settle_internal(
                    database,
                    attempt.snapshot.attempt_id,
                    OrchestrationBranch::Done,
                    None,
                )?;
                return finish_run(database, run_id, node_id).map(OrchestrationExecution::Complete);
            }
            OrchestrationNodeAction::RunNurture { .. }
            | OrchestrationNodeAction::RunInteraction { .. }
            | OrchestrationNodeAction::RunPublish { .. } => {
                if let Some(waiting) =
                    execute_campaign_node(database, &compiled, node, attempt, port).await?
                {
                    return Ok(waiting);
                }
            }
        }
    }
    anyhow::bail!("orchestration traversal exceeded its compiled node count")
}

pub async fn cancel_orchestration<P: OrchestrationChildPort>(
    database: &Database,
    run_id: Uuid,
    port: &mut P,
) -> anyhow::Result<OrchestrationCancelResult> {
    let mut detail = database
        .get_orchestration_run(run_id)?
        .context("orchestration run does not exist")?;
    if detail.run.state.is_terminal() {
        return Ok(OrchestrationCancelResult::AlreadyTerminal(detail.run));
    }
    if detail.run.state == OrchestrationRunState::Queued {
        let run = database
            .cancel_orchestration_run(run_id, OrchestrationRunState::Queued, None)?
            .context("orchestration changed while cancelling")?;
        return Ok(OrchestrationCancelResult::Cancelled(run));
    }

    database
        .request_orchestration_cancel(run_id)?
        .context("orchestration changed while recording cancellation")?;
    detail = database
        .get_orchestration_run(run_id)?
        .context("orchestration run disappeared after cancellation request")?;
    if detail.run.state.is_terminal() {
        return Ok(OrchestrationCancelResult::AlreadyTerminal(detail.run));
    }

    let active = detail
        .attempts
        .iter()
        .find(|attempt| {
            !attempt.state.is_terminal()
                && detail.run.current_node_id == Some(attempt.snapshot.node_id)
        })
        .cloned();
    let Some(mut active) = active else {
        let run = database
            .cancel_orchestration_run(
                run_id,
                OrchestrationRunState::Running,
                detail.run.current_node_id,
            )?
            .context("orchestration changed while cancelling")?;
        return Ok(OrchestrationCancelResult::Cancelled(run));
    };

    let child_campaign_id = if let Some(child_campaign_id) = active.child_campaign_id {
        child_campaign_id
    } else if database
        .cancel_orchestration_attempt_before_effect(active.snapshot.attempt_id)?
        .is_some()
    {
        let run = database
            .cancel_orchestration_run(
                run_id,
                OrchestrationRunState::Running,
                detail.run.current_node_id,
            )?
            .context("orchestration changed while cancelling")?;
        return Ok(OrchestrationCancelResult::Cancelled(run));
    } else {
        // Another executor may have armed the campaign after the cancellation path read the
        // queued attempt. From this point onward the durable child identity wins: reload it and
        // ask the adapter for proof instead of treating the stale no-child read as proof.
        active = database
            .get_orchestration_attempt(active.snapshot.attempt_id)?
            .context("orchestration attempt disappeared during cancellation")?;
        if active.state == OrchestrationAttemptState::Cancelled {
            let run = database
                .cancel_orchestration_run(
                    run_id,
                    OrchestrationRunState::Running,
                    detail.run.current_node_id,
                )?
                .context("orchestration changed while completing cancellation")?;
            return Ok(OrchestrationCancelResult::Cancelled(run));
        }
        if let Some(branch) = active.branch {
            let run = finish_cancel_request(database, run_id, active.snapshot.node_id, branch)?;
            return if run.state == OrchestrationRunState::Cancelled {
                Ok(OrchestrationCancelResult::Cancelled(run))
            } else {
                Ok(OrchestrationCancelResult::AlreadyTerminal(run))
            };
        }
        active
            .child_campaign_id
            .context("orchestration attempt changed without a child or terminal branch")?
    };
    let request = child_request(&active)?;
    match port.cancel_child(&request).await {
        Ok(OrchestrationChildCancelResult::CancelledBeforeEffect) => {
            if database
                .cancel_orchestration_child_before_effect(
                    active.snapshot.attempt_id,
                    child_campaign_id,
                )?
                .is_none()
            {
                let current = database
                    .get_orchestration_attempt(active.snapshot.attempt_id)?
                    .context("orchestration attempt disappeared after child cancellation")?;
                if let Some(branch) = current.branch {
                    let run =
                        finish_cancel_request(database, run_id, current.snapshot.node_id, branch)?;
                    return if run.state == OrchestrationRunState::Cancelled {
                        Ok(OrchestrationCancelResult::Cancelled(run))
                    } else {
                        Ok(OrchestrationCancelResult::AlreadyTerminal(run))
                    };
                }
                if current.state != OrchestrationAttemptState::Cancelled {
                    return Ok(OrchestrationCancelResult::ReconcileRequired {
                        run: detail.run,
                        attempt_id: current.snapshot.attempt_id,
                        child_campaign_id,
                    });
                }
            }
            let run = database
                .cancel_orchestration_run(
                    run_id,
                    OrchestrationRunState::Running,
                    detail.run.current_node_id,
                )?
                .context("orchestration changed while cancelling")?;
            Ok(OrchestrationCancelResult::Cancelled(run))
        }
        Ok(OrchestrationChildCancelResult::Finished(outcome)) => {
            let branch = branch_for_child_outcome(outcome);
            database.settle_orchestration_child(
                active.snapshot.attempt_id,
                child_campaign_id,
                branch,
                None,
            )?;
            let run = finish_cancel_request(database, run_id, active.snapshot.node_id, branch)?;
            if run.state == OrchestrationRunState::Cancelled {
                Ok(OrchestrationCancelResult::Cancelled(run))
            } else {
                Ok(OrchestrationCancelResult::AlreadyTerminal(run))
            }
        }
        Ok(OrchestrationChildCancelResult::EffectMayHaveStarted) | Err(_) => {
            Ok(OrchestrationCancelResult::ReconcileRequired {
                run: detail.run,
                attempt_id: active.snapshot.attempt_id,
                child_campaign_id,
            })
        }
    }
}

async fn execute_campaign_node<P: OrchestrationChildPort>(
    database: &Database,
    compiled: &CompiledOrchestrationV1,
    node: &OrchestrationNode,
    mut attempt: OrchestrationAttemptRecord,
    port: &mut P,
) -> anyhow::Result<Option<OrchestrationExecution>> {
    let kind = child_kind(&node.action).context("campaign node has no child kind")?;
    let profile = attempt
        .snapshot
        .profile
        .clone()
        .context("campaign attempt has no pinned profile")?;
    let cancel_requested = database
        .get_orchestration_run(attempt.run_id)?
        .context("orchestration run disappeared")?
        .run
        .error_code
        .as_deref()
        == Some(CANCEL_REQUESTED);
    if attempt.state == OrchestrationAttemptState::Queued && cancel_requested {
        if database
            .cancel_orchestration_attempt_before_effect(attempt.snapshot.attempt_id)?
            .is_some()
        {
            let run = database
                .cancel_orchestration_run(
                    attempt.run_id,
                    OrchestrationRunState::Running,
                    Some(node.id),
                )?
                .context("orchestration changed while cancelling campaign node")?;
            return Ok(Some(OrchestrationExecution::Complete(run)));
        }
        attempt = database
            .get_orchestration_attempt(attempt.snapshot.attempt_id)?
            .context("campaign attempt disappeared during cancellation")?;
        if attempt.state == OrchestrationAttemptState::Cancelled {
            let run = database
                .cancel_orchestration_run(
                    attempt.run_id,
                    OrchestrationRunState::Running,
                    Some(node.id),
                )?
                .context("orchestration changed while completing campaign cancellation")?;
            return Ok(Some(OrchestrationExecution::Complete(run)));
        }
        if let Some(branch) = attempt.branch {
            let run = finish_cancel_request(database, attempt.run_id, node.id, branch)?;
            return Ok(Some(OrchestrationExecution::Complete(run)));
        }
    }
    if attempt.state == OrchestrationAttemptState::Queued {
        let profile_record =
            database.get_automation_definition_record(profile.definition_id, profile.revision)?;
        let valid_profile = profile_record
            .as_ref()
            .is_some_and(|record| !record.definition.archived && record.definition.kind == kind);
        if !valid_profile {
            settle_internal(
                database,
                attempt.snapshot.attempt_id,
                OrchestrationBranch::Failed,
                Some("profile_unavailable"),
            )?;
            route(
                database,
                attempt.run_id,
                compiled,
                node.id,
                OrchestrationBranch::Failed,
            )?;
            return Ok(None);
        }
        let child_campaign_id = Uuid::new_v4();
        attempt = database
            .arm_orchestration_child(attempt.snapshot.attempt_id, kind, child_campaign_id)?
            .context("orchestration attempt changed before child arm")?;
        let request = child_request(&attempt)?;
        match port.dispatch_child(request).await {
            Ok(OrchestrationChildDispatch::Started) => {
                let waiting = database
                    .mark_orchestration_child_started(
                        attempt.snapshot.attempt_id,
                        child_campaign_id,
                    )?
                    .context("orchestration child changed before start acknowledgement")?;
                return Ok(Some(OrchestrationExecution::Waiting {
                    run: database
                        .get_orchestration_run(attempt.run_id)?
                        .context("orchestration run disappeared")?
                        .run,
                    attempt_id: waiting.snapshot.attempt_id,
                    child_campaign_id,
                }));
            }
            Ok(OrchestrationChildDispatch::Finished(outcome)) => {
                settle_child(database, &attempt, branch_for_child_outcome(outcome), None)?;
            }
            Err(failure) => {
                let (branch, code) = match failure.phase {
                    OrchestrationChildFailurePhase::BeforeEffect => {
                        (OrchestrationBranch::Failed, "child_failed_before_effect")
                    }
                    OrchestrationChildFailurePhase::AfterEffect => {
                        (OrchestrationBranch::Uncertain, "child_dispatch_uncertain")
                    }
                };
                settle_child(database, &attempt, branch, Some(code))?;
            }
        }
    } else if matches!(
        attempt.state,
        OrchestrationAttemptState::Dispatching | OrchestrationAttemptState::WaitingChild
    ) {
        let request = child_request(&attempt)?;
        match port
            .reconcile_child(&request)
            .await
            .map_err(anyhow::Error::msg)?
        {
            OrchestrationChildStatus::Running => {
                if attempt.state == OrchestrationAttemptState::Dispatching {
                    attempt = database
                        .mark_orchestration_child_started(
                            attempt.snapshot.attempt_id,
                            request.child_campaign_id,
                        )?
                        .context("orchestration child changed while reconciling")?;
                }
                return Ok(Some(OrchestrationExecution::Waiting {
                    run: database
                        .get_orchestration_run(attempt.run_id)?
                        .context("orchestration run disappeared")?
                        .run,
                    attempt_id: attempt.snapshot.attempt_id,
                    child_campaign_id: request.child_campaign_id,
                }));
            }
            OrchestrationChildStatus::Finished(outcome) => {
                settle_child(database, &attempt, branch_for_child_outcome(outcome), None)?;
            }
            OrchestrationChildStatus::MissingBeforeEffect => {
                settle_child(
                    database,
                    &attempt,
                    OrchestrationBranch::Failed,
                    Some("child_missing_before_effect"),
                )?;
            }
        }
    }

    let settled = database
        .get_orchestration_run(attempt.run_id)?
        .context("orchestration run disappeared")?
        .attempts
        .into_iter()
        .find(|candidate| candidate.snapshot.attempt_id == attempt.snapshot.attempt_id)
        .context("orchestration attempt disappeared")?;
    let branch = settled.branch.context("settled child has no branch")?;
    let current_run = database
        .get_orchestration_run(attempt.run_id)?
        .context("orchestration run disappeared")?
        .run;
    if current_run.error_code.as_deref() == Some(CANCEL_REQUESTED) {
        let run = finish_cancel_request(database, attempt.run_id, node.id, branch)?;
        return Ok(Some(OrchestrationExecution::Complete(run)));
    }
    route(database, attempt.run_id, compiled, node.id, branch)?;
    Ok(None)
}

fn settle_internal(
    database: &Database,
    attempt_id: Uuid,
    branch: OrchestrationBranch,
    error_code: Option<&str>,
) -> anyhow::Result<()> {
    database
        .settle_orchestration_node(attempt_id, branch, error_code)?
        .context("orchestration node changed before settlement")?;
    Ok(())
}

fn settle_child(
    database: &Database,
    attempt: &OrchestrationAttemptRecord,
    branch: OrchestrationBranch,
    error_code: Option<&str>,
) -> anyhow::Result<()> {
    database
        .settle_orchestration_child(
            attempt.snapshot.attempt_id,
            attempt
                .child_campaign_id
                .context("armed orchestration attempt has no child ID")?,
            branch,
            error_code,
        )?
        .context("orchestration child changed before settlement")?;
    Ok(())
}

fn child_request(
    attempt: &OrchestrationAttemptRecord,
) -> anyhow::Result<OrchestrationChildRequest> {
    Ok(OrchestrationChildRequest {
        owner: AutomationChildOwner::OrchestrationAttempt {
            attempt_id: attempt.snapshot.attempt_id,
        },
        idempotency_key: attempt.snapshot.idempotency_key.clone(),
        child_campaign_id: attempt
            .child_campaign_id
            .context("orchestration child ID is missing")?,
        kind: attempt
            .child_kind
            .context("orchestration child kind is missing")?,
        profile: attempt
            .snapshot
            .profile
            .clone()
            .context("orchestration child profile is missing")?,
        target: attempt.snapshot.target.clone(),
    })
}

fn child_kind(action: &OrchestrationNodeAction) -> Option<AutomationKind> {
    match action {
        OrchestrationNodeAction::RunNurture { .. } => Some(AutomationKind::Nurture),
        OrchestrationNodeAction::RunInteraction { .. } => Some(AutomationKind::Interaction),
        OrchestrationNodeAction::RunPublish { .. } => Some(AutomationKind::Publish),
        OrchestrationNodeAction::Start
        | OrchestrationNodeAction::Delay { .. }
        | OrchestrationNodeAction::End => None,
    }
}

fn node(compiled: &CompiledOrchestrationV1, node_id: Uuid) -> anyhow::Result<&OrchestrationNode> {
    compiled
        .document
        .nodes
        .iter()
        .find(|node| node.id == node_id)
        .with_context(|| format!("orchestration node {node_id} is missing"))
}

fn route(
    database: &Database,
    run_id: Uuid,
    compiled: &CompiledOrchestrationV1,
    source_node_id: Uuid,
    branch: OrchestrationBranch,
) -> anyhow::Result<()> {
    let target = compiled
        .document
        .edges
        .iter()
        .find(|edge| edge.source_node_id == source_node_id && edge.source_port == branch)
        .with_context(|| format!("orchestration node {source_node_id} has no {branch:?} branch"))?
        .target_node_id;
    database
        .advance_orchestration_run(run_id, source_node_id, target)?
        .context("orchestration run changed before graph advance")?;
    Ok(())
}

fn finish_run(
    database: &Database,
    run_id: Uuid,
    node_id: Uuid,
) -> anyhow::Result<OrchestrationRunRecord> {
    let detail = database
        .get_orchestration_run(run_id)?
        .context("orchestration run disappeared before completion")?;
    let final_state = aggregate_state(&detail.attempts);
    terminal_run(database, run_id, node_id, final_state, None)
}

fn aggregate_state(attempts: &[OrchestrationAttemptRecord]) -> OrchestrationRunState {
    if attempts
        .iter()
        .any(|attempt| attempt.branch == Some(OrchestrationBranch::Uncertain))
    {
        OrchestrationRunState::Uncertain
    } else if attempts
        .iter()
        .any(|attempt| attempt.branch == Some(OrchestrationBranch::Failed))
    {
        OrchestrationRunState::Failed
    } else if attempts
        .iter()
        .any(|attempt| attempt.branch == Some(OrchestrationBranch::Partial))
    {
        OrchestrationRunState::Partial
    } else {
        OrchestrationRunState::Done
    }
}

fn terminal_run(
    database: &Database,
    run_id: Uuid,
    node_id: Uuid,
    state: OrchestrationRunState,
    error_code: Option<&str>,
) -> anyhow::Result<OrchestrationRunRecord> {
    database
        .transition_orchestration_run(
            run_id,
            OrchestrationRunState::Running,
            state,
            Some(node_id),
            error_code,
        )?
        .context("orchestration run changed before terminal settlement")
}

fn finish_cancel_request(
    database: &Database,
    run_id: Uuid,
    node_id: Uuid,
    branch: OrchestrationBranch,
) -> anyhow::Result<OrchestrationRunRecord> {
    if branch == OrchestrationBranch::Uncertain {
        terminal_run(
            database,
            run_id,
            node_id,
            OrchestrationRunState::Uncertain,
            Some("cancel_requested_child_uncertain"),
        )
    } else {
        database
            .cancel_orchestration_run(run_id, OrchestrationRunState::Running, Some(node_id))?
            .context("orchestration changed while finishing cancellation")
    }
}
