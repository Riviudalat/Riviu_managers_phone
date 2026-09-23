//! Dependency scheduling using the existing assignment worker and device ownership.
use super::*;

#[allow(clippy::too_many_arguments)]
pub(super) async fn run(
    db: Arc<crate::db::Database>,
    control: Arc<DeviceControlPlane>,
    engine: crate::NurtureEngine,
    events: crate::EventBus,
    campaign: String,
    request: ThreadCampaignRequest,
    plan: ThreadPlan,
    only: Option<std::collections::HashSet<String>>,
    artifacts: crate::FlowArtifactStore,
    frames: Arc<dyn crate::GenerationFrameSource>,
) -> anyhow::Result<(usize, usize)> {
    let capacity = control.stream_capacity().clamp(1, 4);
    let gate = Arc::new(tokio::sync::Semaphore::new(capacity));
    let mut attempted = std::collections::HashSet::new();
    let mut busy = std::collections::HashSet::new();
    let mut action_targets = std::collections::HashSet::new();
    let mut tasks = tokio::task::JoinSet::new();
    let mut counts = (0, 0);
    loop {
        let stopped = campaign_is_cancelled(&db, &campaign)?;
        let detail = db
            .get_interaction_campaign(&campaign)?
            .context("Campaign missing")?;
        let mut added = false;
        let mut waiting_for_parent = false;
        if !stopped {
            for row in &detail.assignments {
                if tasks.len() >= capacity {
                    break;
                }
                let action_only = !request
                    .seeding
                    .as_ref()
                    .expect("seeding config")
                    .is_comment(&request, row.ordinal);
                if action_only && action_targets.contains(&row.target_key) {
                    continue;
                }
                if !request
                    .seeding
                    .as_ref()
                    .expect("seeding config")
                    .is_comment(&request, row.ordinal)
                    && detail.assignments.iter().any(|a| {
                        a.target_key == row.target_key
                            && a.ordinal < row.ordinal
                            && !request
                                .seeding
                                .as_ref()
                                .expect("seeding config")
                                .is_comment(&request, a.ordinal)
                            && matches!(
                                a.state,
                                ThreadMessageState::Queued
                                    | ThreadMessageState::Ready
                                    | ThreadMessageState::Preparing
                                    | ThreadMessageState::Sending
                            )
                    })
                {
                    continue;
                }
                if attempted.contains(&row.id)
                    || busy.contains(&row.actor_udid)
                    || only.as_ref().is_some_and(|s| !s.contains(&row.id))
                    || !matches!(
                        row.state,
                        ThreadMessageState::Queued
                            | ThreadMessageState::Ready
                            | ThreadMessageState::Failed
                            | ThreadMessageState::SkippedParent
                    )
                {
                    continue;
                }
                // The readback worker owns the same device while settling a sent
                // comment. Let it finish before dispatching this actor's next turn,
                // even when that turn is a standalone comment with no parent.
                if detail.assignments.iter().any(|a| {
                    a.actor_udid == row.actor_udid
                        && a.id != row.id
                        && a.comment_verification.as_ref().is_some_and(|v| {
                            v.state == crate::comment_verification::VerificationState::Pending
                                && v.deadline_ms
                                    .is_some_and(|d| d > chrono::Utc::now().timestamp_millis())
                        })
                }) {
                    waiting_for_parent = true;
                    continue;
                }
                // Keep action candidate priority deterministic, and serialize each actor's
                // own turns. Reply dependencies are checked against durable parent evidence.
                if detail.assignments.iter().any(|a| {
                    a.actor_udid == row.actor_udid
                        && a.ordinal < row.ordinal
                        && !attempted.contains(&a.id)
                        && matches!(
                            a.state,
                            ThreadMessageState::Queued | ThreadMessageState::Ready
                        )
                }) {
                    continue;
                }
                if let Some(parent) = row
                    .parent_assignment_id
                    .as_ref()
                    .and_then(|id| detail.assignments.iter().find(|a| &a.id == id))
                {
                    if only.as_ref().is_none_or(|scope| scope.contains(&parent.id))
                        && (!attempted.contains(&parent.id) || busy.contains(&parent.actor_udid))
                        && matches!(
                            parent.state,
                            ThreadMessageState::Failed
                                | ThreadMessageState::SkippedParent
                                | ThreadMessageState::Queued
                                | ThreadMessageState::Ready
                        )
                    {
                        continue;
                    }
                    if parent.comment_verification.as_ref().is_some_and(|v| {
                        v.state == crate::comment_verification::VerificationState::Pending
                            && v.deadline_ms
                                .is_some_and(|d| d > chrono::Utc::now().timestamp_millis())
                    }) {
                        waiting_for_parent = true;
                        continue;
                    }
                    if matches!(
                        parent.state,
                        ThreadMessageState::Queued
                            | ThreadMessageState::Ready
                            | ThreadMessageState::Preparing
                            | ThreadMessageState::Sending
                    ) {
                        continue;
                    }
                }
                let actor = row.actor_udid.clone();
                let id = row.id.clone();
                attempted.insert(id.clone());
                busy.insert(actor.clone());
                let action_target = action_only.then(|| row.target_key.clone());
                if let Some(key) = &action_target {
                    action_targets.insert(key.clone());
                }
                added = true;
                let (db, control, engine, events, campaign, request, plan, artifacts, frames, gate) = (
                    db.clone(),
                    control.clone(),
                    engine.clone(),
                    events.clone(),
                    campaign.clone(),
                    request.clone(),
                    plan.clone(),
                    artifacts.clone(),
                    frames.clone(),
                    gate.clone(),
                );
                tasks.spawn(async move {
                    (
                        actor,
                        action_target,
                        gated_cohort(
                            gate,
                            None,
                            db,
                            control,
                            engine,
                            events,
                            campaign,
                            request,
                            plan,
                            Some(std::collections::HashSet::from([id])),
                            artifacts,
                            frames,
                            Arc::new(HashMap::new()),
                        )
                        .await,
                    )
                });
            }
        }
        if tasks.is_empty() {
            if waiting_for_parent && !stopped {
                tokio::time::sleep(Duration::from_millis(500)).await;
                continue;
            }
            break;
        }
        if !added || tasks.len() >= capacity || stopped {
            if let Some(joined) = tasks.join_next().await {
                let (actor, action_target, result) = joined?;
                busy.remove(&actor);
                if let Some(key) = action_target {
                    action_targets.remove(&key);
                }
                let (a, b) = result?;
                counts.0 += a;
                counts.1 += b;
            }
        }
    }
    Ok(counts)
}
