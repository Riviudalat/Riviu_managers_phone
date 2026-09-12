//! Per-turn scheduling inside the existing campaign runtime.
use super::*;

#[allow(clippy::too_many_arguments)]
pub(super) async fn run(
    db: Arc<crate::db::Database>,
    control: Arc<DeviceControlPlane>,
    engine: crate::NurtureEngine,
    events: crate::EventBus,
    campaign_id: String,
    request: ThreadCampaignRequest,
    plan: ThreadPlan,
    only: Option<std::collections::HashSet<String>>,
    artifacts: crate::FlowArtifactStore,
    frames: Arc<dyn crate::GenerationFrameSource>,
    mut session: crate::db::ConversationSession,
    token: String,
) -> anyhow::Result<(usize, usize)> {
    let script = request
        .scripted_conversation
        .as_ref()
        .context("Kịch bản thiếu")?;
    struct Owner {
        db: Arc<crate::db::Database>,
        id: String,
        token: String,
    }
    impl Drop for Owner {
        fn drop(&mut self) {
            let _ = self.db.release_conversation_session(&self.id, &self.token);
        }
    }
    let _owner = Owner {
        db: db.clone(),
        id: campaign_id.clone(),
        token: token.clone(),
    };
    let mut attempted = std::collections::HashSet::new();
    let mut ok = 0;
    let mut bad = 0;
    loop {
        if campaign_is_cancelled(&db, &campaign_id)? {
            break;
        }
        let detail = db
            .get_interaction_campaign(&campaign_id)?
            .context("Phiên mất dữ liệu")?;
        let now = chrono::Utc::now().timestamp_millis();
        let pending: Vec<_> = detail
            .assignments
            .iter()
            .filter(|a| {
                only.as_ref().is_none_or(|s| s.contains(&a.id))
                    && !attempted.contains(&a.id)
                    && matches!(
                        a.state,
                        ThreadMessageState::Queued
                            | ThreadMessageState::Ready
                            | ThreadMessageState::Failed
                            | ThreadMessageState::SkippedParent
                    )
            })
            .collect();
        if pending.is_empty() {
            break;
        }
        if now >= session.ends_at_ms {
            for a in pending {
                db.update_interaction_assignment_state(
                    &a.id,
                    ThreadMessageState::Failed,
                    Some("Chưa thực hiện — hết thời lượng phiên"),
                    None,
                    None,
                )?;
            }
            notify(&events, &campaign_id);
            break;
        }
        if now < session.next_at_ms {
            tokio::time::sleep(Duration::from_millis(
                (session.next_at_ms - now).min(500) as u64
            ))
            .await;
            continue;
        }
        let mut picked = None;
        for offset in 0..request.targets.len() {
            let index = (session.cursor + offset) % request.targets.len();
            let target = &request.targets[index];
            let Some(a) = pending
                .iter()
                .filter(|a| a.target_key == target.target_key)
                .min_by_key(|a| a.ordinal)
            else {
                continue;
            };
            let step = script
                .step(&a.target_key, a.ordinal)
                .context("Câu không tồn tại")?;
            if let Some(parent) = &a.parent_assignment_id {
                let parent = detail
                    .assignments
                    .iter()
                    .find(|p| p.id == *parent)
                    .context("Parent không tồn tại")?;
                if parent.posted_identity().is_none() {
                    if matches!(
                        parent.state,
                        ThreadMessageState::Failed
                            | ThreadMessageState::Uncertain
                            | ThreadMessageState::SkippedParent
                            | ThreadMessageState::Succeeded
                    ) || attempted.contains(&parent.id)
                    {
                        db.update_interaction_assignment_state(
                            &a.id,
                            ThreadMessageState::SkippedParent,
                            Some("Bình luận cha chưa được xác nhận"),
                            None,
                            None,
                        )?;
                        attempted.insert(a.id.clone());
                        bad += 1;
                        notify(&events, &campaign_id);
                    }
                    continue;
                }
            }
            let role = script.role(&step.speaker_id).context("Vai chưa có máy")?;
            if control.current_work_owner(&role.udid).is_some() {
                continue;
            }
            picked = Some((index, (*a).clone()));
            break;
        }
        let Some((index, assignment)) = picked else {
            tokio::time::sleep(Duration::from_millis(500)).await;
            continue;
        };
        attempted.insert(assignment.id.clone());
        let start = chrono::Utc::now().timestamp_millis();
        let result = run_cohort(
            Some(std::collections::HashSet::from([assignment
                .target_key
                .clone()])),
            db.clone(),
            control.clone(),
            engine.clone(),
            events.clone(),
            campaign_id.clone(),
            request.clone(),
            plan.clone(),
            Some(std::collections::HashSet::from([assignment.id.clone()])),
            artifacts.clone(),
            frames.clone(),
            Arc::new(HashMap::new()),
        )
        .await;
        match result {
            Ok((a, b)) => {
                ok += a;
                bad += b;
            }
            Err(error) => {
                bad += 1;
                tracing::warn!("Hội thoại {}: {error:#}", assignment.id);
            }
        }
        let finish = chrono::Utc::now().timestamp_millis();
        let remaining = pending.len().saturating_sub(1) as i64;
        let tuple = control
            .tiktok_build(&assignment.actor_udid)
            .await
            .map(|(p, v, l)| format!("{p}/{v}/{l}/{}", assignment.parent_assignment_id.is_some()))
            .unwrap_or_default();
        let estimate = db.conversation_turn_estimate(&tuple)?;
        let slack = (session.ends_at_ms
            - finish
            - (session.ends_at_ms - session.started_at_ms) / 10
            - remaining * estimate)
            .max(0);
        let factor = 800 + (script.seed.wrapping_add(attempted.len() as u64 * 7919) % 401) as i64;
        session.next_at_ms = finish
            + if remaining > 0 {
                (slack / remaining * factor / 1000).min(slack)
            } else {
                0
            };
        session.cursor = (index + 1) % request.targets.len();
        db.finish_conversation_turn(
            &campaign_id,
            &token,
            &assignment.id,
            session.cursor,
            session.next_at_ms,
            start,
            finish,
            &tuple,
        )?;
        notify(&events, &campaign_id);
    }
    Ok((ok, bad))
}
