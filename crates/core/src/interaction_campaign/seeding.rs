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
    let scheduler_db = db.clone();
    let scheduler_campaign = campaign.clone();
    let scheduler_request = request.clone();
    schedule(
        scheduler_db,
        scheduler_campaign,
        scheduler_request,
        only,
        capacity,
        move |id| {
            gated_cohort(
                gate.clone(),
                None,
                db.clone(),
                control.clone(),
                engine.clone(),
                events.clone(),
                campaign.clone(),
                request.clone(),
                plan.clone(),
                Some(std::collections::HashSet::from([id])),
                artifacts.clone(),
                frames.clone(),
                Arc::new(HashMap::new()),
            )
        },
    )
    .await
}

/// The seeding dispatcher: which assignment may start next, one task per assignment.
///
/// `cohort` runs exactly one assignment; it is a parameter so the dispatch and join rules
/// below can be exercised without a phone.
async fn schedule<F, Fut>(
    db: Arc<crate::db::Database>,
    campaign: String,
    request: ThreadCampaignRequest,
    only: Option<std::collections::HashSet<String>>,
    capacity: usize,
    cohort: F,
) -> anyhow::Result<(usize, usize)>
where
    F: Fn(String) -> Fut,
    Fut: std::future::Future<Output = anyhow::Result<(usize, usize)>> + Send + 'static,
{
    let seeding = request
        .seeding
        .as_ref()
        .context("Campaign không có cấu hình seeding")?;
    let mut attempted = std::collections::HashSet::new();
    let mut busy = std::collections::HashSet::new();
    let mut action_targets = std::collections::HashSet::new();
    let mut tasks = tokio::task::JoinSet::new();
    let mut counts = (0, 0);
    let mut first_error: Option<anyhow::Error> = None;
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
                let action_only = !seeding.is_comment(&request, row.ordinal);
                if action_only && action_targets.contains(&row.target_key) {
                    continue;
                }
                if !seeding.is_comment(&request, row.ordinal)
                    && detail.assignments.iter().any(|a| {
                        a.target_key == row.target_key
                            && a.ordinal < row.ordinal
                            && !seeding.is_comment(&request, a.ordinal)
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
                let task = cohort(id);
                tasks.spawn(async move { (actor, action_target, task.await) });
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
                // Recorded, never returned early: leaving here drops `tasks`, and dropping a
                // `JoinSet` aborts every sibling at whatever await it is parked on — between a
                // Like tap and its receipt included. Same contract as `join_campaign`.
                match result {
                    Ok((a, b)) => {
                        counts.0 += a;
                        counts.1 += b;
                    }
                    Err(error) => {
                        tracing::warn!("interaction seeding cohort failed: {error:#}");
                        first_error.get_or_insert(error);
                    }
                }
            }
        }
    }
    match first_error {
        Some(error) => Err(error),
        None => Ok(counts),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interaction::{
        plan_threads, InteractionActionSet, ResolvedTikTokTarget, ThreadMode, ThreadShape,
        TikTokPostKind,
    };
    use crate::seeding::{SecondsRange, SeedingConfig};

    fn target(id: &str) -> ResolvedTikTokTarget {
        ResolvedTikTokTarget {
            original_url: format!("https://www.tiktok.com/@creator/video/{id}"),
            normalized_url: format!("https://www.tiktok.com/@creator/video/{id}"),
            target_key: format!("content:{id}"),
            content_id: id.into(),
            author: "creator".into(),
            kind: TikTokPostKind::Video,
        }
    }

    fn like_only(seed: u64) -> ThreadCampaignRequest {
        ThreadCampaignRequest {
            seeding: Some(SeedingConfig {
                standalone_count: 0,
                like_count: 1,
                save_count: 0,
                share_count: 0,
                seed,
                preferred_actors: Vec::new(),
                expected_accounts: Default::default(),
                watch_seconds: SecondsRange { min: 0, max: 0 },
                comment_gap_seconds: SecondsRange { min: 0, max: 0 },
                comments: Default::default(),
            }),
            scripted_conversation: None,
            request_id: format!("seeding-sibling-{seed}"),
            targets: vec![target("1"), target("2")],
            actor_udids: vec!["phone-a".into(), "phone-b".into()],
            message_count: 0,
            instruction: String::new(),
            max_words: 0,
            manual_comments: Vec::new(),
            actions: InteractionActionSet {
                share: false,
                follow: false,
                like: true,
                comment: false,
                save: false,
            },
            mode: ThreadMode::Standalone,
            shape: ThreadShape::Star,
            cohort_size: None,
            mentions: Vec::new(),
            mention_parent: false,
            like_parent: false,
            post_dwell_seconds: None,
        }
    }

    /// One phone's cohort failing must not abort another phone's cohort mid-action.
    ///
    /// Dropping a `JoinSet` aborts every task still in it, so an early `?` on one joined
    /// result cancels each sibling at whatever await it was parked on — between a Like tap
    /// and its receipt included. `join_campaign` already keeps siblings standing.
    #[tokio::test]
    async fn one_failed_cohort_leaves_its_sibling_running_to_completion() {
        // A seed whose two targets open on different phones, so both start together.
        let (request, plan) = (0..64)
            .find_map(|seed| {
                let request = like_only(seed);
                let plan = plan_threads(&request).expect("plan");
                let first = |key: &str| {
                    plan.assignments
                        .iter()
                        .find(|a| a.target_key == key && a.ordinal == 0)
                        .map(|a| a.actor_udid.clone())
                };
                (first("content:1") != first("content:2")).then_some((request, plan))
            })
            .expect("a seed that splits the first phones");
        let path =
            std::env::temp_dir().join(format!("riviu-seeding-sibling-{}.db", uuid::Uuid::new_v4()));
        let db = Arc::new(crate::db::Database::open(&path).expect("open fixture database"));
        let campaign = db
            .create_interaction_campaign(&request, &plan)
            .expect("create campaign");
        db.update_interaction_campaign_state(&campaign, ThreadCampaignState::Running, None)
            .expect("mark running");
        let actor_of: HashMap<String, String> = db
            .get_interaction_campaign(&campaign)
            .expect("read")
            .expect("campaign")
            .assignments
            .into_iter()
            .map(|a| (a.id, a.actor_udid))
            .collect();

        let finished = Arc::new(AtomicBool::new(false));
        let witness = finished.clone();
        let result = schedule(db.clone(), campaign, request, None, 2, move |id| {
            let failing = actor_of.get(&id).map(String::as_str) == Some("phone-a");
            let witness = witness.clone();
            async move {
                if failing {
                    anyhow::bail!("phone-a stream lost");
                }
                tokio::time::sleep(Duration::from_millis(300)).await;
                witness.store(true, Ordering::SeqCst);
                Ok((1, 0))
            }
        })
        .await;

        assert!(
            finished.load(Ordering::SeqCst),
            "phone-b's cohort was aborted because phone-a's failed"
        );
        let error = result.expect_err("the failure is still reported to join_campaign");
        assert!(
            format!("{error:#}").contains("phone-a stream lost"),
            "{error:#}"
        );
        drop(db);
        let _ = std::fs::remove_file(path);
    }
}
