//! One read-only projection over every kind of work the desktop can launch.
//!
//! The underlying records stay authoritative. This module only normalizes their names,
//! terminal states and child rows so the operations page does not invent a second state
//! machine in TypeScript.

use serde::{Deserialize, Serialize};

use crate::{
    AutomationKind, FlowAggregateState, FlowAttemptState, FlowDeviceRunState, FlowRunDetail,
    FlowRunRecord, InteractionCampaignDetail, InteractionCampaignSummary, JobRecord, JobStatus,
    NurtureCleanupState, NurtureSessionStatus, OrchestrationAttemptState, OrchestrationBranch,
    OrchestrationRunDetail, OrchestrationRunRecord, OrchestrationRunState, Outcome,
    PublishCampaignDetail, PublishCampaignState, PublishExecutionSnapshot, PublishExecutionStatus,
    PublishRetryScope, ResolvedTargetDevice, ResolvedTargetSnapshot, StepStatus,
    ThreadCampaignState, ThreadMessageState,
};

#[derive(Debug, Clone, Copy, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum OperationRunKind {
    Script,
    Flow,
    Orchestration,
    Nurture,
    Interaction,
    Publish,
}

impl OperationRunKind {
    pub fn as_key(self) -> &'static str {
        match self {
            Self::Script => "script",
            Self::Flow => "flow",
            Self::Orchestration => "orchestration",
            Self::Nurture => "nurture",
            Self::Interaction => "interaction",
            Self::Publish => "publish",
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum OperationRunState {
    Queued,
    Running,
    Succeeded,
    Partial,
    Failed,
    Uncertain,
    Cancelled,
    Skipped,
}

impl OperationRunState {
    pub fn is_terminal(self) -> bool {
        !matches!(self, Self::Queued | Self::Running)
    }

    pub fn needs_attention(self) -> bool {
        matches!(self, Self::Partial | Self::Failed | Self::Uncertain)
    }
}

#[derive(Debug, Clone, Copy, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum OperationRunItemKind {
    Step,
    Device,
    Attempt,
    Assignment,
}

#[derive(Debug, Clone, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OperationRunSummary {
    /// Collision-free UI identity (`kind:sourceId`).
    pub id: String,
    pub source_id: String,
    pub kind: OperationRunKind,
    pub title: String,
    pub state: OperationRunState,
    pub target_count: u32,
    pub total_items: u32,
    pub completed_items: u32,
    pub issue_count: u32,
    pub retryable_count: u32,
    /// Publish exposes the exact restart boundary. Other sources use per-item `retryable`.
    pub retry_scope: Option<PublishRetryScope>,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OperationRunItem {
    pub id: String,
    pub kind: OperationRunItemKind,
    pub label: String,
    pub state: OperationRunState,
    pub udid: Option<String>,
    pub error_code: Option<String>,
    pub detail: Option<String>,
    pub evidence: Option<String>,
    pub retryable: bool,
}

#[derive(Debug, Clone, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OperationRunDetail {
    pub summary: OperationRunSummary,
    pub items: Vec<OperationRunItem>,
}

fn operation_id(kind: OperationRunKind, source_id: &str) -> String {
    format!("{}:{source_id}", kind.as_key())
}

fn terminal_progress(state: OperationRunState, total: u32) -> u32 {
    if state.is_terminal() {
        total
    } else {
        0
    }
}

pub fn project_job(job: &JobRecord) -> OperationRunDetail {
    let state = match job.status {
        JobStatus::Queued => OperationRunState::Queued,
        JobStatus::Running => OperationRunState::Running,
        JobStatus::Succeeded => OperationRunState::Succeeded,
        JobStatus::Failed => OperationRunState::Failed,
        JobStatus::Cancelled => OperationRunState::Cancelled,
    };
    let items = job
        .steps
        .iter()
        .map(|step| OperationRunItem {
            id: step.index.to_string(),
            kind: OperationRunItemKind::Step,
            label: format!("Bước {}", step.index + 1),
            state: match step.status {
                StepStatus::Pending => OperationRunState::Queued,
                StepStatus::Running => OperationRunState::Running,
                StepStatus::Succeeded => OperationRunState::Succeeded,
                StepStatus::Failed => OperationRunState::Failed,
                StepStatus::Skipped => OperationRunState::Skipped,
            },
            udid: None,
            error_code: step.error.clone(),
            detail: Some(step.action.clone()),
            evidence: step.artifact_path.clone(),
            retryable: false,
        })
        .collect::<Vec<_>>();
    let issue_count = (items
        .iter()
        .filter(|item| item.state.needs_attention())
        .count() as u32)
        .max(u32::from(state.needs_attention()));
    OperationRunDetail {
        summary: OperationRunSummary {
            id: operation_id(OperationRunKind::Script, &job.id.to_string()),
            source_id: job.id.to_string(),
            kind: OperationRunKind::Script,
            title: job.script_name.clone(),
            state,
            target_count: job.udids.len() as u32,
            total_items: items.len() as u32,
            completed_items: items.iter().filter(|item| item.state.is_terminal()).count() as u32,
            issue_count,
            retryable_count: 0,
            retry_scope: None,
            created_at: Some(job.created_at.to_rfc3339()),
            updated_at: Some(job.updated_at.to_rfc3339()),
        },
        items,
    }
}

pub fn project_flow_summary(run: &FlowRunRecord, title: String) -> OperationRunSummary {
    let state = match run.state {
        FlowAggregateState::Queued => OperationRunState::Queued,
        FlowAggregateState::Running => OperationRunState::Running,
        FlowAggregateState::Succeeded => OperationRunState::Succeeded,
        FlowAggregateState::Partial => OperationRunState::Partial,
        FlowAggregateState::Failed => OperationRunState::Failed,
        FlowAggregateState::Cancelled => OperationRunState::Cancelled,
    };
    let target_count = run.selection.target_udids.len() as u32;
    OperationRunSummary {
        id: operation_id(OperationRunKind::Flow, &run.id.to_string()),
        source_id: run.id.to_string(),
        kind: OperationRunKind::Flow,
        title,
        state,
        target_count,
        total_items: target_count,
        completed_items: terminal_progress(state, target_count),
        issue_count: u32::from(state.needs_attention()),
        retryable_count: 0,
        retry_scope: None,
        created_at: Some(run.created_at.to_rfc3339()),
        updated_at: Some(run.updated_at.to_rfc3339()),
    }
}

pub fn project_flow_detail(detail: &FlowRunDetail, title: String) -> OperationRunDetail {
    let mut summary = project_flow_summary(&detail.run, title);
    let attempt_by_device = detail.attempts.iter().fold(
        std::collections::HashMap::new(),
        |mut by_device, attempt| {
            by_device
                .entry(attempt.device_run_id)
                .or_insert_with(Vec::new)
                .push(attempt);
            by_device
        },
    );
    let items = detail
        .device_runs
        .iter()
        .map(|device| {
            let attempts = attempt_by_device
                .get(&device.id)
                .cloned()
                .unwrap_or_default();
            let uncertain = attempts
                .iter()
                .any(|attempt| attempt.state == FlowAttemptState::Uncertain);
            let retryable = !uncertain && attempts.iter().any(|attempt| attempt.retry_allowed);
            let evidence = attempts
                .iter()
                .rev()
                .find_map(|attempt| attempt.evidence_result.as_ref())
                .and_then(|value| serde_json::to_string(value).ok());
            OperationRunItem {
                id: device.id.to_string(),
                kind: OperationRunItemKind::Device,
                label: "Thiết bị".to_string(),
                state: if uncertain {
                    OperationRunState::Uncertain
                } else {
                    match device.state {
                        FlowDeviceRunState::Queued | FlowDeviceRunState::Preflight => {
                            OperationRunState::Queued
                        }
                        FlowDeviceRunState::Running => OperationRunState::Running,
                        FlowDeviceRunState::Succeeded => OperationRunState::Succeeded,
                        FlowDeviceRunState::Failed => OperationRunState::Failed,
                        FlowDeviceRunState::Skipped => OperationRunState::Skipped,
                        FlowDeviceRunState::Cancelled => OperationRunState::Cancelled,
                    }
                },
                udid: Some(device.udid.clone()),
                error_code: device.error.as_ref().map(|error| error.code.clone()),
                detail: device.error.as_ref().map(|error| error.message.clone()),
                evidence,
                retryable,
            }
        })
        .collect::<Vec<_>>();
    finish_detail(&mut summary, &items);
    OperationRunDetail { summary, items }
}

pub fn project_orchestration_summary(
    run: &OrchestrationRunRecord,
    title: String,
) -> OperationRunSummary {
    let state = match run.state {
        OrchestrationRunState::Queued => OperationRunState::Queued,
        OrchestrationRunState::Running => OperationRunState::Running,
        OrchestrationRunState::Done => OperationRunState::Succeeded,
        OrchestrationRunState::Partial => OperationRunState::Partial,
        OrchestrationRunState::Failed => OperationRunState::Failed,
        OrchestrationRunState::Uncertain => OperationRunState::Uncertain,
        OrchestrationRunState::Cancelled => OperationRunState::Cancelled,
    };
    let target_count = run.target.included.len() as u32;
    OperationRunSummary {
        id: operation_id(OperationRunKind::Orchestration, &run.id.to_string()),
        source_id: run.id.to_string(),
        kind: OperationRunKind::Orchestration,
        title,
        state,
        target_count,
        total_items: run.node_targets.len() as u32,
        completed_items: terminal_progress(state, run.node_targets.len() as u32),
        issue_count: u32::from(state.needs_attention()),
        retryable_count: 0,
        retry_scope: None,
        created_at: Some(run.created_at.clone()),
        updated_at: Some(run.updated_at.clone()),
    }
}

pub fn project_orchestration_detail(
    detail: &OrchestrationRunDetail,
    title: String,
) -> OperationRunDetail {
    let mut summary = project_orchestration_summary(&detail.run, title);
    let items = detail
        .attempts
        .iter()
        .map(|attempt| OperationRunItem {
            id: attempt.snapshot.attempt_id.to_string(),
            kind: OperationRunItemKind::Attempt,
            label: attempt
                .child_kind
                .map(orchestration_child_label)
                .unwrap_or_else(|| "Bước điều phối".to_string()),
            state: match attempt.state {
                OrchestrationAttemptState::Queued | OrchestrationAttemptState::Dispatching => {
                    OperationRunState::Queued
                }
                OrchestrationAttemptState::WaitingChild => OperationRunState::Running,
                OrchestrationAttemptState::Done => OperationRunState::Succeeded,
                OrchestrationAttemptState::Partial => OperationRunState::Partial,
                OrchestrationAttemptState::Failed => OperationRunState::Failed,
                OrchestrationAttemptState::Uncertain => OperationRunState::Uncertain,
                OrchestrationAttemptState::Cancelled => OperationRunState::Cancelled,
            },
            udid: None,
            error_code: attempt.error_code.clone(),
            detail: attempt.branch.map(orchestration_branch_label),
            evidence: attempt.child_campaign_id.map(|id| id.to_string()),
            retryable: false,
        })
        .collect::<Vec<_>>();
    finish_detail(&mut summary, &items);
    OperationRunDetail { summary, items }
}

fn orchestration_child_label(kind: AutomationKind) -> String {
    match kind {
        AutomationKind::Nurture => "Nuôi TikTok",
        AutomationKind::Interaction => "Tương tác",
        AutomationKind::Publish => "Đăng bài",
    }
    .to_string()
}

fn orchestration_branch_label(branch: OrchestrationBranch) -> String {
    match branch {
        OrchestrationBranch::Done => "Nhánh hoàn tất",
        OrchestrationBranch::Partial => "Nhánh một phần",
        OrchestrationBranch::Failed => "Nhánh thất bại",
        OrchestrationBranch::Uncertain => "Nhánh chưa chắc chắn",
    }
    .to_string()
}

pub fn nurture_source_id(status: &NurtureSessionStatus) -> String {
    status
        .run_id
        .map(|id| id.to_string())
        .unwrap_or_else(|| format!("legacy:{}", status.udid))
}

fn nurture_item_state(status: &NurtureSessionStatus) -> OperationRunState {
    if status.running {
        return OperationRunState::Running;
    }
    match status.outcome {
        Some(Outcome::Done) if status.cleanup_state == NurtureCleanupState::ProcessAbsent => {
            OperationRunState::Succeeded
        }
        Some(Outcome::Done | Outcome::Partial) => OperationRunState::Partial,
        Some(Outcome::Failed) => OperationRunState::Failed,
        Some(Outcome::Stopped)
            if status.cleanup_state == NurtureCleanupState::ProcessAbsent
                || (status.cleanup_state == NurtureCleanupState::Pending
                    && status.started_at.is_none()) =>
        {
            OperationRunState::Cancelled
        }
        Some(Outcome::Stopped) => OperationRunState::Partial,
        None => OperationRunState::Uncertain,
    }
}

pub fn project_nurture(source_id: &str, sessions: &[NurtureSessionStatus]) -> OperationRunDetail {
    let items = sessions
        .iter()
        .map(|session| OperationRunItem {
            id: session.udid.clone(),
            kind: OperationRunItemKind::Device,
            label: "Thiết bị".to_string(),
            state: nurture_item_state(session),
            udid: Some(session.udid.clone()),
            error_code: None,
            detail: match (
                session.last_message.is_empty(),
                session.cleanup_error.as_deref(),
            ) {
                (true, None) => None,
                (false, None) => Some(session.last_message.clone()),
                (true, Some(cleanup_error)) => Some(cleanup_error.to_string()),
                (false, Some(cleanup_error)) => {
                    Some(format!("{} · {cleanup_error}", session.last_message))
                }
            },
            evidence: session
                .cleanup_proof
                .as_ref()
                .and_then(|proof| serde_json::to_string(proof).ok()),
            retryable: false,
        })
        .collect::<Vec<_>>();
    let state = aggregate_item_state(&items);
    let target_count = sessions
        .iter()
        .map(|session| session.run_size)
        .max()
        .unwrap_or_default()
        .max(sessions.len() as u32);
    let started = sessions
        .iter()
        .filter_map(|session| session.started_at)
        .min()
        .map(|value| value.to_rfc3339());
    let updated = sessions
        .iter()
        .filter_map(|session| session.updated_at.or(session.started_at))
        .max()
        .map(|value| value.to_rfc3339());
    let mut summary = OperationRunSummary {
        id: operation_id(OperationRunKind::Nurture, source_id),
        source_id: source_id.to_string(),
        kind: OperationRunKind::Nurture,
        title: "Nuôi TikTok".to_string(),
        state,
        target_count,
        total_items: sessions.len() as u32,
        completed_items: 0,
        issue_count: 0,
        retryable_count: 0,
        retry_scope: None,
        created_at: started.clone(),
        updated_at: updated.or(started),
    };
    finish_detail(&mut summary, &items);
    OperationRunDetail { summary, items }
}

pub fn project_interaction_summary(summary: &InteractionCampaignSummary) -> OperationRunSummary {
    let state = match summary.state {
        ThreadCampaignState::Queued => OperationRunState::Queued,
        ThreadCampaignState::Running => OperationRunState::Running,
        ThreadCampaignState::Succeeded => OperationRunState::Succeeded,
        ThreadCampaignState::Partial => OperationRunState::Partial,
        ThreadCampaignState::Failed => OperationRunState::Failed,
        ThreadCampaignState::Cancelled => OperationRunState::Cancelled,
    };
    let title = summary
        .brief
        .as_ref()
        .and_then(|brief| brief.first_author.as_deref())
        .map(|author| format!("Tương tác · @{author}"))
        .unwrap_or_else(|| "Tương tác".to_string());
    OperationRunSummary {
        id: operation_id(OperationRunKind::Interaction, &summary.id),
        source_id: summary.id.clone(),
        kind: OperationRunKind::Interaction,
        title,
        state,
        target_count: summary.target_count,
        total_items: summary.message_count as u32,
        completed_items: summary.succeeded_messages + summary.failed_messages,
        issue_count: summary.failed_messages + summary.action_counters.uncertain,
        retryable_count: 0,
        retry_scope: None,
        created_at: None,
        updated_at: Some(summary.updated_at.clone()),
    }
}

pub fn project_interaction_detail(detail: &InteractionCampaignDetail) -> OperationRunDetail {
    let mut summary = project_interaction_summary(&detail.summary);
    let retryable = crate::interaction_campaign::retryable_assignments(&detail.assignments, None);
    let items = detail
        .assignments
        .iter()
        .map(|assignment| OperationRunItem {
            id: assignment.id.clone(),
            kind: OperationRunItemKind::Assignment,
            label: format!("Lượt tương tác {}", assignment.ordinal + 1),
            state: match assignment.state {
                ThreadMessageState::Queued
                | ThreadMessageState::Preparing
                | ThreadMessageState::Ready => OperationRunState::Queued,
                ThreadMessageState::Sending => OperationRunState::Running,
                ThreadMessageState::Succeeded => OperationRunState::Succeeded,
                ThreadMessageState::Failed => OperationRunState::Failed,
                ThreadMessageState::Uncertain => OperationRunState::Uncertain,
                ThreadMessageState::SkippedParent => OperationRunState::Skipped,
            },
            udid: Some(assignment.actor_udid.clone()),
            error_code: assignment.error_code.clone(),
            detail: assignment.prepared_text.clone(),
            evidence: (!assignment.actions.is_empty())
                .then(|| serde_json::to_string(&assignment.actions).ok())
                .flatten(),
            retryable: retryable.contains(&assignment.id),
        })
        .collect::<Vec<_>>();
    finish_detail(&mut summary, &items);
    OperationRunDetail { summary, items }
}

pub fn project_publish_summary(
    detail: &PublishCampaignDetail,
    snapshot: Option<&PublishExecutionSnapshot>,
) -> OperationRunSummary {
    let campaign = &detail.campaign;
    let typed_uncertain = campaign.state == PublishCampaignState::Uncertain
        || detail
            .assignments
            .iter()
            .any(|assignment| assignment.state == PublishCampaignState::Uncertain)
        || (!matches!(
            campaign.state,
            PublishCampaignState::Posting | PublishCampaignState::Verifying
        ) && detail.assignments.iter().any(|assignment| {
            matches!(
                assignment.state,
                PublishCampaignState::Posting | PublishCampaignState::Verifying
            )
        }));
    let (state, retry_scope) = if typed_uncertain {
        // Startup recovery can advance these rows after the last execution snapshot. A stale
        // FullPipeline value must never turn an ambiguous Post into another public Post.
        (OperationRunState::Uncertain, PublishRetryScope::None)
    } else {
        match campaign.state {
            PublishCampaignState::Queued | PublishCampaignState::Scheduled => {
                (OperationRunState::Queued, PublishRetryScope::None)
            }
            PublishCampaignState::Preparing
            | PublishCampaignState::Ready
            | PublishCampaignState::Transferring
            | PublishCampaignState::Imported
            | PublishCampaignState::Posting
            | PublishCampaignState::Verifying => {
                (OperationRunState::Running, PublishRetryScope::None)
            }
            PublishCampaignState::FailedBeforeDispatch => {
                let has_success = detail
                    .assignments
                    .iter()
                    .any(|assignment| assignment.state == PublishCampaignState::Succeeded);
                (
                    if has_success {
                        OperationRunState::Partial
                    } else {
                        OperationRunState::Failed
                    },
                    PublishRetryScope::FullPipeline,
                )
            }
            PublishCampaignState::Missed => (OperationRunState::Failed, PublishRetryScope::None),
            PublishCampaignState::Uncertain => unreachable!("handled above"),
            PublishCampaignState::Cancelled => {
                (OperationRunState::Cancelled, PublishRetryScope::None)
            }
            PublishCampaignState::Succeeded => match snapshot {
                Some(snapshot) => match snapshot.status {
                    PublishExecutionStatus::Complete => {
                        (OperationRunState::Succeeded, PublishRetryScope::None)
                    }
                    PublishExecutionStatus::Uncertain => {
                        (OperationRunState::Uncertain, PublishRetryScope::None)
                    }
                    PublishExecutionStatus::Partial => {
                        // A succeeded campaign proves the public Post already happened. A stale
                        // FullPipeline snapshot cannot reopen that effect boundary.
                        let scope = match snapshot.retry_scope {
                            PublishRetryScope::LinkAndSheet | PublishRetryScope::SheetOnly => {
                                snapshot.retry_scope
                            }
                            PublishRetryScope::FullPipeline | PublishRetryScope::None => {
                                PublishRetryScope::None
                            }
                        };
                        (OperationRunState::Partial, scope)
                    }
                },
                // A terminal campaign only proves that Post was confirmed. The execution
                // snapshot is the durable evidence for canonical-link capture and Sheet
                // settlement. Legacy or interrupted rows without it therefore still owe those
                // post-effect steps; presenting them as complete hides recoverable work.
                None => (OperationRunState::Partial, PublishRetryScope::LinkAndSheet),
            },
        }
    };
    let retryable_count = u32::from(retry_scope != PublishRetryScope::None);
    let total_items = detail.assignments.len() as u32;
    let completed_items = detail
        .assignments
        .iter()
        .filter(|assignment| publish_item_state(assignment.state.clone(), state).is_terminal())
        .count() as u32;
    let typed_issues = detail
        .assignments
        .iter()
        .filter(|assignment| publish_item_state(assignment.state.clone(), state).needs_attention())
        .count() as u32;
    OperationRunSummary {
        id: operation_id(OperationRunKind::Publish, &campaign.id),
        source_id: campaign.id.clone(),
        kind: OperationRunKind::Publish,
        title: "Đăng bài".to_string(),
        state,
        target_count: total_items,
        total_items,
        completed_items,
        issue_count: typed_issues.max(u32::from(state.needs_attention())),
        retryable_count,
        retry_scope: Some(retry_scope),
        created_at: Some(campaign.created_at.clone()),
        updated_at: Some(
            snapshot
                .filter(|snapshot| snapshot.updated_at.as_str() > campaign.updated_at.as_str())
                .map(|snapshot| snapshot.updated_at.clone())
                .unwrap_or_else(|| campaign.updated_at.clone()),
        ),
    }
}

fn publish_item_state(
    assignment_state: PublishCampaignState,
    aggregate_state: OperationRunState,
) -> OperationRunState {
    match assignment_state {
        PublishCampaignState::Queued | PublishCampaignState::Scheduled => OperationRunState::Queued,
        PublishCampaignState::Preparing
        | PublishCampaignState::Ready
        | PublishCampaignState::Transferring
        | PublishCampaignState::Imported => OperationRunState::Running,
        PublishCampaignState::Posting | PublishCampaignState::Verifying => {
            if aggregate_state == OperationRunState::Uncertain {
                OperationRunState::Uncertain
            } else {
                OperationRunState::Running
            }
        }
        PublishCampaignState::Succeeded => OperationRunState::Succeeded,
        PublishCampaignState::FailedBeforeDispatch | PublishCampaignState::Missed => {
            OperationRunState::Failed
        }
        PublishCampaignState::Uncertain => OperationRunState::Uncertain,
        PublishCampaignState::Cancelled => OperationRunState::Cancelled,
    }
}

pub fn project_publish_detail(
    detail: &PublishCampaignDetail,
    snapshot: Option<&PublishExecutionSnapshot>,
) -> OperationRunDetail {
    project_publish_detail_with_target(detail, snapshot, None)
}

pub fn project_publish_detail_with_target(
    detail: &PublishCampaignDetail,
    snapshot: Option<&PublishExecutionSnapshot>,
    target_snapshot: Option<&ResolvedTargetSnapshot>,
) -> OperationRunDetail {
    let mut summary = project_publish_summary(detail, snapshot);
    let aggregate_issue_count = summary.issue_count;
    let retry_scope = summary.retry_scope.unwrap_or(PublishRetryScope::None);
    let aggregate_state = summary.state;
    let items = detail
        .assignments
        .iter()
        .map(|assignment| OperationRunItem {
            id: assignment.id.clone(),
            kind: OperationRunItemKind::Assignment,
            label: target_snapshot
                .and_then(|snapshot| {
                    snapshot
                        .included
                        .iter()
                        .find(|device| device.udid == assignment.udid)
                })
                .map(reviewed_target_label)
                .unwrap_or_else(|| format!("Bài {}", assignment.ordinal + 1)),
            state: publish_item_state(assignment.state.clone(), aggregate_state),
            udid: Some(assignment.udid.clone()),
            error_code: assignment.error_code.clone(),
            detail: Some(assignment.bundle_id.clone()),
            evidence: assignment.evidence_json.clone(),
            retryable: retry_scope == PublishRetryScope::FullPipeline
                && matches!(
                    assignment.state,
                    PublishCampaignState::Queued
                        | PublishCampaignState::Scheduled
                        | PublishCampaignState::Ready
                        | PublishCampaignState::Imported
                        | PublishCampaignState::FailedBeforeDispatch
                ),
        })
        .collect::<Vec<_>>();
    finish_detail(&mut summary, &items);
    // Link/Sheet settlement lives on the aggregate snapshot, not an assignment row.
    // Keep that warning even when every public Post assignment itself succeeded.
    summary.issue_count = summary.issue_count.max(aggregate_issue_count);
    OperationRunDetail { summary, items }
}

fn reviewed_target_label(device: &ResolvedTargetDevice) -> String {
    match (device.number, device.alias.trim()) {
        (Some(number), "") => format!("Máy {number}"),
        (Some(number), alias) => format!("Máy {number} · {alias}"),
        (None, "") => "Máy trong snapshot".to_string(),
        (None, alias) => alias.to_string(),
    }
}

fn aggregate_item_state(items: &[OperationRunItem]) -> OperationRunState {
    if items
        .iter()
        .any(|item| item.state == OperationRunState::Running)
    {
        return OperationRunState::Running;
    }
    if items
        .iter()
        .any(|item| item.state == OperationRunState::Queued)
    {
        return OperationRunState::Queued;
    }
    if items
        .iter()
        .any(|item| item.state == OperationRunState::Uncertain)
    {
        return OperationRunState::Uncertain;
    }
    if items
        .iter()
        .any(|item| item.state == OperationRunState::Partial)
    {
        return OperationRunState::Partial;
    }
    let successes = items
        .iter()
        .filter(|item| item.state == OperationRunState::Succeeded)
        .count();
    let failures = items
        .iter()
        .filter(|item| item.state.needs_attention() || item.state == OperationRunState::Skipped)
        .count();
    let cancelled = items
        .iter()
        .filter(|item| item.state == OperationRunState::Cancelled)
        .count();
    if failures > 0 && (successes > 0 || cancelled > 0) {
        OperationRunState::Partial
    } else if failures > 0 {
        OperationRunState::Failed
    } else if !items.is_empty()
        && items
            .iter()
            .all(|item| item.state == OperationRunState::Cancelled)
    {
        OperationRunState::Cancelled
    } else {
        OperationRunState::Succeeded
    }
}

fn finish_detail(summary: &mut OperationRunSummary, items: &[OperationRunItem]) {
    summary.total_items = items.len() as u32;
    summary.completed_items = items.iter().filter(|item| item.state.is_terminal()).count() as u32;
    summary.issue_count = items
        .iter()
        .filter(|item| item.state.needs_attention())
        .count() as u32;
    let retryable_items = items.iter().filter(|item| item.retryable).count() as u32;
    if retryable_items > 0 || summary.retryable_count == 0 {
        summary.retryable_count = retryable_items;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        InteractionActionCounters, InteractionActionSet, InteractionCampaignBrief, ThreadMode,
        ThreadShape,
    };

    #[test]
    fn failed_script_keeps_step_error_in_real_detail() {
        let now = chrono::Utc::now();
        let detail = project_job(&JobRecord {
            id: uuid::Uuid::new_v4(),
            script_name: "Kiểm tra màn hình".into(),
            udids: vec!["phone-a".into()],
            status: JobStatus::Failed,
            created_at: now,
            updated_at: now,
            steps: vec![crate::JobStepRecord {
                index: 0,
                action: "screenshot".into(),
                status: StepStatus::Failed,
                error: Some("frame source unavailable".into()),
                artifact_path: None,
            }],
            error: Some("script failed".into()),
        });

        assert_eq!(detail.summary.state, OperationRunState::Failed);
        assert_eq!(detail.summary.issue_count, 1);
        assert_eq!(detail.items[0].detail.as_deref(), Some("screenshot"));
        assert_eq!(
            detail.items[0].error_code.as_deref(),
            Some("frame source unavailable")
        );
    }

    #[test]
    fn interaction_projection_keeps_partial_and_retryable_assignments_distinct() {
        let summary = InteractionCampaignSummary {
            id: "campaign-a".into(),
            request_id: "request-a".into(),
            state: ThreadCampaignState::Partial,
            message_count: 2,
            target_count: 1,
            succeeded_messages: 1,
            failed_messages: 1,
            error_code: Some("OneFailed".into()),
            updated_at: "2026-09-04T12:00:00Z".into(),
            brief: Some(InteractionCampaignBrief {
                first_author: Some("creator".into()),
                first_content_id: Some("post".into()),
                mode: ThreadMode::Standalone,
                shape: ThreadShape::Star,
                cohort_size: None,
                actor_count: 2,
                manual: true,
                like_target: false,
                actions: InteractionActionSet::default(),
            }),
            action_counters: InteractionActionCounters::default(),
        };
        let assignment = |id: &str, state| crate::InteractionAssignmentRecord {
            id: id.into(),
            target_key: "target".into(),
            ordinal: 0,
            actor_udid: "phone".into(),
            parent_assignment_id: None,
            state,
            prepared_text: None,
            error_code: None,
            evidence_json: None,
            like: None,
            mention: None,
            parent_was_folded: false,
            actions: Vec::new(),
        };
        let detail = project_interaction_detail(&InteractionCampaignDetail {
            summary,
            assignments: vec![
                assignment("sent", ThreadMessageState::Succeeded),
                assignment("retry", ThreadMessageState::Failed),
            ],
            action_aggregate: None,
        });

        assert_eq!(detail.summary.state, OperationRunState::Partial);
        assert_eq!(detail.summary.target_count, 1);
        assert_eq!(detail.summary.retryable_count, 1);
        assert!(!detail.items[0].retryable);
        assert!(detail.items[1].retryable);
    }

    #[test]
    fn nurture_cleanup_without_absence_proof_is_partial_not_success() {
        let mut status = NurtureSessionStatus::new("phone-a");
        status.finish(Outcome::Done);
        let detail = project_nurture("run-a", &[status]);
        assert_eq!(detail.summary.state, OperationRunState::Partial);
        assert_eq!(detail.items[0].state, OperationRunState::Partial);
    }

    #[test]
    fn stopped_nurture_with_failed_cleanup_requires_attention() {
        let mut status = NurtureSessionStatus::new("phone-a");
        status.started_at = Some(chrono::Utc::now());
        status.cleanup_state = NurtureCleanupState::Failed;
        status.cleanup_error = Some("không chứng minh được process đã dừng".into());
        status.finish(Outcome::Stopped);

        let detail = project_nurture("run-a", &[status]);

        assert_eq!(detail.summary.state, OperationRunState::Partial);
        assert_eq!(detail.summary.issue_count, 1);
        assert_eq!(detail.items[0].state, OperationRunState::Partial);
    }

    #[test]
    fn stopped_nurture_with_absence_proof_projects_as_cancelled() {
        let mut status = NurtureSessionStatus::new("phone-a");
        status.started_at = Some(chrono::Utc::now());
        status.cleanup_state = NurtureCleanupState::ProcessAbsent;
        status.finish(Outcome::Stopped);

        let detail = project_nurture("run-a", &[status]);

        assert_eq!(detail.summary.state, OperationRunState::Cancelled);
        assert_eq!(detail.summary.issue_count, 0);
        assert_eq!(detail.items[0].state, OperationRunState::Cancelled);
    }

    #[test]
    fn nurture_projection_uses_the_latest_status_update_not_the_oldest_start() {
        let at = |second| {
            chrono::DateTime::parse_from_rfc3339(&format!("2026-09-04T10:00:{second:02}Z"))
                .expect("fixed timestamp")
                .with_timezone(&chrono::Utc)
        };
        let first = NurtureSessionStatus {
            started_at: Some(at(1)),
            updated_at: Some(at(40)),
            ..NurtureSessionStatus::new("phone-a")
        };
        let second = NurtureSessionStatus {
            started_at: Some(at(2)),
            updated_at: Some(at(55)),
            ..NurtureSessionStatus::new("phone-b")
        };

        let detail = project_nurture("run-a", &[first, second]);

        assert_eq!(
            detail.summary.created_at.as_deref(),
            Some("2026-09-04T10:00:01+00:00")
        );
        assert_eq!(
            detail.summary.updated_at.as_deref(),
            Some("2026-09-04T10:00:55+00:00")
        );
    }

    #[test]
    fn publish_snapshot_is_authoritative_for_partial_and_retry_scope() {
        let campaign = crate::PublishCampaignRecord {
            id: "campaign-a".into(),
            request_id: "request-a".into(),
            source_root: "C:/media".into(),
            state: PublishCampaignState::Succeeded,
            run_at: None,
            visibility: crate::PublishVisibility::Public,
            cleanup_policy: crate::PublishCleanupPolicy::DeleteImportedAssetsAfterVerified,
            assignments: Vec::new(),
            created_at: "2026-09-04T10:00:00Z".into(),
            updated_at: "2026-09-04T10:01:00Z".into(),
            error_code: None,
        };
        let snapshot = PublishExecutionSnapshot {
            campaign_id: campaign.id.clone(),
            input_digest: "a".repeat(64),
            status: PublishExecutionStatus::Partial,
            retry_scope: PublishRetryScope::SheetOnly,
            report_json: serde_json::json!({}),
            updated_at: "2026-09-04T10:02:00Z".into(),
        };
        let detail = PublishCampaignDetail {
            campaign,
            bundles: Vec::new(),
            assignments: Vec::new(),
            events: Vec::new(),
        };

        let projected = project_publish_summary(&detail, Some(&snapshot));
        assert_eq!(projected.state, OperationRunState::Partial);
        assert_eq!(projected.retryable_count, 1);
        assert_eq!(projected.retry_scope, Some(PublishRetryScope::SheetOnly));
        assert_eq!(
            projected.updated_at.as_deref(),
            Some("2026-09-04T10:02:00Z")
        );
    }

    #[test]
    fn publish_detail_keeps_sheet_pending_as_an_aggregate_issue() {
        let campaign_id = "campaign-sheet-pending".to_string();
        let detail = PublishCampaignDetail {
            campaign: crate::PublishCampaignRecord {
                id: campaign_id.clone(),
                request_id: "request-sheet-pending".into(),
                source_root: "C:/media".into(),
                state: PublishCampaignState::Succeeded,
                run_at: None,
                visibility: crate::PublishVisibility::Public,
                cleanup_policy: crate::PublishCleanupPolicy::DeleteImportedAssetsAfterVerified,
                assignments: vec![crate::PublishAssignmentPlan {
                    bundle_id: "bundle-a".into(),
                    udid: "phone-a".into(),
                    ordinal: 0,
                }],
                created_at: "2026-09-04T10:00:00Z".into(),
                updated_at: "2026-09-04T10:01:00Z".into(),
                error_code: None,
            },
            bundles: Vec::new(),
            assignments: vec![crate::PublishAssignmentRecord {
                id: "assignment-a".into(),
                campaign_id: campaign_id.clone(),
                bundle_id: "bundle-a".into(),
                ordinal: 0,
                udid: "phone-a".into(),
                state: PublishCampaignState::Succeeded,
                effect_intent: Some("post-intent".into()),
                evidence_json: Some(
                    serde_json::json!({"canonicalUrl": "https://example.test/post"}).to_string(),
                ),
                error_code: None,
            }],
            events: Vec::new(),
        };
        let snapshot = PublishExecutionSnapshot {
            campaign_id,
            input_digest: "a".repeat(64),
            status: PublishExecutionStatus::Partial,
            retry_scope: PublishRetryScope::SheetOnly,
            report_json: serde_json::json!({"sheet": "pending"}),
            updated_at: "2026-09-04T10:02:00Z".into(),
        };

        let target_snapshot = crate::ResolvedTargetSnapshot {
            target_ref: crate::TargetRef::Explicit {
                udids: vec!["phone-a".into()],
            },
            included: vec![crate::ResolvedTargetDevice {
                udid: "phone-a".into(),
                alias: "Kệ trên".into(),
                number: Some(19),
            }],
            excluded: Vec::new(),
            roster_sha256: "a".repeat(64),
        };
        let projected =
            project_publish_detail_with_target(&detail, Some(&snapshot), Some(&target_snapshot));

        assert_eq!(projected.summary.state, OperationRunState::Partial);
        assert_eq!(projected.summary.issue_count, 1);
        assert_eq!(projected.items[0].state, OperationRunState::Succeeded);
        assert_eq!(projected.items[0].label, "Máy 19 · Kệ trên");
    }

    #[test]
    fn orchestration_labels_are_operator_facing_vietnamese() {
        assert_eq!(
            orchestration_child_label(AutomationKind::Nurture),
            "Nuôi TikTok"
        );
        assert_eq!(
            orchestration_child_label(AutomationKind::Interaction),
            "Tương tác"
        );
        assert_eq!(
            orchestration_child_label(AutomationKind::Publish),
            "Đăng bài"
        );
        assert_eq!(
            orchestration_branch_label(OrchestrationBranch::Uncertain),
            "Nhánh chưa chắc chắn"
        );
    }

    #[test]
    fn succeeded_publish_without_execution_snapshot_still_owes_link_and_sheet() {
        let detail = PublishCampaignDetail {
            campaign: crate::PublishCampaignRecord {
                id: "legacy-campaign".into(),
                request_id: "legacy-request".into(),
                source_root: "C:/media".into(),
                state: PublishCampaignState::Succeeded,
                run_at: None,
                visibility: crate::PublishVisibility::Public,
                cleanup_policy: crate::PublishCleanupPolicy::DeleteImportedAssetsAfterVerified,
                assignments: Vec::new(),
                created_at: "2026-09-04T10:00:00Z".into(),
                updated_at: "2026-09-04T10:01:00Z".into(),
                error_code: None,
            },
            bundles: Vec::new(),
            assignments: Vec::new(),
            events: Vec::new(),
        };

        let projected = project_publish_summary(&detail, None);

        assert_eq!(projected.state, OperationRunState::Partial);
        assert_eq!(projected.retry_scope, Some(PublishRetryScope::LinkAndSheet));
        assert_eq!(projected.retryable_count, 1);
        assert_eq!(projected.issue_count, 1);
    }

    #[test]
    fn orphaned_posting_overrides_stale_full_pipeline_snapshot() {
        let campaign_id = "campaign-orphan".to_string();
        let detail = PublishCampaignDetail {
            campaign: crate::PublishCampaignRecord {
                id: campaign_id.clone(),
                request_id: "request-orphan".into(),
                source_root: "C:/media".into(),
                // This is the typed state written by startup recovery for an orphaned Post.
                state: PublishCampaignState::Uncertain,
                run_at: None,
                visibility: crate::PublishVisibility::Public,
                cleanup_policy: crate::PublishCleanupPolicy::DeleteImportedAssetsAfterVerified,
                assignments: vec![crate::PublishAssignmentPlan {
                    bundle_id: "bundle-a".into(),
                    udid: "phone-a".into(),
                    ordinal: 0,
                }],
                created_at: "2026-09-04T10:00:00Z".into(),
                updated_at: "2026-09-04T10:03:00Z".into(),
                error_code: Some("publish_worker_lost".into()),
            },
            bundles: Vec::new(),
            assignments: vec![crate::PublishAssignmentRecord {
                id: "assignment-a".into(),
                campaign_id: campaign_id.clone(),
                bundle_id: "bundle-a".into(),
                ordinal: 0,
                udid: "phone-a".into(),
                state: PublishCampaignState::Uncertain,
                effect_intent: Some("post-intent".into()),
                evidence_json: None,
                error_code: Some("publish_worker_lost".into()),
            }],
            events: Vec::new(),
        };
        let stale_snapshot = PublishExecutionSnapshot {
            campaign_id,
            input_digest: "b".repeat(64),
            status: PublishExecutionStatus::Partial,
            retry_scope: PublishRetryScope::FullPipeline,
            report_json: serde_json::json!({"source": "before-crash"}),
            updated_at: "2026-09-04T10:01:00Z".into(),
        };

        let projected = project_publish_detail(&detail, Some(&stale_snapshot));
        assert_eq!(projected.summary.state, OperationRunState::Uncertain);
        assert_eq!(projected.summary.retry_scope, Some(PublishRetryScope::None));
        assert_eq!(projected.summary.retryable_count, 0);
        assert_eq!(projected.items[0].state, OperationRunState::Uncertain);
        assert!(!projected.items[0].retryable);
        assert_eq!(
            projected.summary.updated_at.as_deref(),
            Some("2026-09-04T10:03:00Z"),
            "typed recovery timestamp must outrank the stale snapshot"
        );
    }

    #[test]
    fn startup_recovery_projection_never_reopens_an_orphaned_post() {
        let path = std::env::temp_dir().join(format!(
            "riviu-operation-publish-recovery-{}.db",
            uuid::Uuid::new_v4()
        ));
        let db = crate::db::Database::open(&path).expect("open operation fixture");
        let bundle = crate::PublishBundle {
            id: "bundle-a".into(),
            source_path: "C:/fixture/bundle-a".into(),
            name: "bundle-a".into(),
            media_kind: crate::PublishMediaKind::Image,
            images: Vec::new(),
            video: None,
            caption_path: "C:/fixture/bundle-a/caption.txt".into(),
            caption: "caption".into(),
            caption_sha256: "0".repeat(64),
            total_bytes: 1,
            partners: Vec::new(),
        };
        let request = crate::PublishCampaignRequest {
            request_id: "request-orphan-db".into(),
            source_root: "C:/fixture".into(),
            bundle_ids: vec![bundle.id.clone()],
            udids: vec!["phone-a".into()],
            run_at: None,
            visibility: crate::PublishVisibility::Public,
            cleanup_policy: crate::PublishCleanupPolicy::DeleteImportedAssetsAfterVerified,
            sound_policy: crate::PublishSoundPolicy::Default,
            execution_confirmed: true,
            target_snapshot: None,
        };
        let campaign = db
            .create_publish_campaign(&request, &[bundle])
            .expect("create campaign");
        db.update_publish_campaign_state(&campaign.id, PublishCampaignState::Posting, None)
            .expect("campaign was posting when the process died");
        let assignment = db
            .get_publish_campaign(&campaign.id)
            .expect("read campaign")
            .expect("campaign exists")
            .assignments
            .into_iter()
            .next()
            .expect("assignment exists");
        db.update_publish_assignment_state(
            &assignment.id,
            PublishCampaignState::Posting,
            None,
            Some(r#"{"effectIntent":"post"}"#),
        )
        .expect("assignment was posting when the process died");
        db.save_publish_execution_snapshot(
            &campaign.id,
            &"c".repeat(64),
            PublishExecutionStatus::Partial,
            PublishRetryScope::FullPipeline,
            &serde_json::json!({"source": "before-crash"}),
        )
        .expect("save stale snapshot");

        assert_eq!(
            db.interrupt_orphaned_publish_campaigns().expect("recover"),
            1
        );
        let recovered = db
            .get_publish_campaign(&campaign.id)
            .expect("read recovered campaign")
            .expect("campaign remains");
        let stale_snapshot = db
            .get_publish_execution_snapshot(&campaign.id)
            .expect("read snapshot")
            .expect("snapshot remains until explicit publish reconcile");
        assert_eq!(stale_snapshot.retry_scope, PublishRetryScope::FullPipeline);

        let projected = project_publish_detail(&recovered, Some(&stale_snapshot));
        assert_eq!(projected.summary.state, OperationRunState::Uncertain);
        assert_eq!(projected.summary.retry_scope, Some(PublishRetryScope::None));
        assert_eq!(projected.summary.retryable_count, 0);
        assert!(projected.items.iter().all(|item| !item.retryable));

        drop(db);
        let _ = std::fs::remove_file(path);
    }
}
