//! Durable, independently settled public actions for Interaction assignments.

use super::*;

fn action_kind_label(kind: crate::interaction::InteractionActionKind) -> &'static str {
    kind.as_str()
}

fn owner_kind_from_label(
    label: &str,
) -> rusqlite::Result<crate::interaction::TikTokActionOwnerKind> {
    match label {
        "interaction" => Ok(crate::interaction::TikTokActionOwnerKind::Interaction),
        "nurture" => Ok(crate::interaction::TikTokActionOwnerKind::Nurture),
        _ => Err(rusqlite::Error::InvalidQuery),
    }
}

fn action_kind_from_label(
    label: &str,
) -> rusqlite::Result<crate::interaction::InteractionActionKind> {
    match label {
        "like" => Ok(crate::interaction::InteractionActionKind::Like),
        "save" => Ok(crate::interaction::InteractionActionKind::Save),
        "comment" => Ok(crate::interaction::InteractionActionKind::Comment),
        "follow" => Ok(crate::interaction::InteractionActionKind::Follow),
        _ => Err(rusqlite::Error::InvalidQuery),
    }
}

fn action_state_label(state: crate::interaction::InteractionActionState) -> &'static str {
    use crate::interaction::InteractionActionState;
    match state {
        InteractionActionState::Planned => "planned",
        InteractionActionState::Preparing => "preparing",
        InteractionActionState::Armed => "armed",
        InteractionActionState::Confirmed => "confirmed",
        InteractionActionState::NoOp => "no_op",
        InteractionActionState::FailedBeforeEffect => "failed_before_effect",
        InteractionActionState::Uncertain => "uncertain",
    }
}

fn action_state_from_label(
    label: &str,
) -> rusqlite::Result<crate::interaction::InteractionActionState> {
    use crate::interaction::InteractionActionState;
    match label {
        "planned" => Ok(InteractionActionState::Planned),
        "preparing" => Ok(InteractionActionState::Preparing),
        "armed" => Ok(InteractionActionState::Armed),
        "confirmed" => Ok(InteractionActionState::Confirmed),
        "no_op" => Ok(InteractionActionState::NoOp),
        "failed_before_effect" => Ok(InteractionActionState::FailedBeforeEffect),
        "uncertain" => Ok(InteractionActionState::Uncertain),
        _ => Err(rusqlite::Error::InvalidQuery),
    }
}

pub(super) fn insert_interaction_action_runs(
    transaction: &rusqlite::Transaction<'_>,
    campaign_id: &str,
    assignment_id: &str,
    device_udid: &str,
    actions: crate::interaction::InteractionActionSet,
    now: &str,
) -> anyhow::Result<()> {
    for kind in actions.ordered() {
        transaction.execute(
            "INSERT INTO tiktok_action_runs
             (id,owner_kind,owner_id,device_udid,campaign_id,assignment_id,action_kind,
              state,revision,created_at,updated_at)
             VALUES (?1,'interaction',?3,?4,?2,?3,?5,'planned',0,?6,?6)",
            params![
                Uuid::new_v4().to_string(),
                campaign_id,
                assignment_id,
                device_udid,
                action_kind_label(kind),
                now,
            ],
        )?;
    }
    Ok(())
}

impl Database {
    /// Atomically arm both the legacy comment assignment and its independent action row.
    pub fn begin_interaction_comment_action_effect(
        &self,
        assignment_id: &str,
        assignment_revision: i64,
        action_revision: i64,
        effect_intent: &str,
    ) -> anyhow::Result<Option<i64>> {
        let mut conn = self.conn()?;
        let transaction = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let now = Utc::now().to_rfc3339();
        let assignment_changed = transaction.execute(
            "UPDATE interaction_assignments
             SET state='sending',error_code=NULL,effect_intent=?1,revision=revision+1,updated_at=?2
             WHERE id=?3 AND state='preparing' AND revision=?4
               AND EXISTS (
                   SELECT 1 FROM interaction_campaigns AS campaign
                   WHERE campaign.id=interaction_assignments.campaign_id
                     AND campaign.state='running'
               )",
            params![effect_intent, now, assignment_id, assignment_revision],
        )?;
        if assignment_changed == 0 {
            transaction.rollback()?;
            return Ok(None);
        }
        let action_revision = transaction
            .query_row(
                "UPDATE tiktok_action_runs
                 SET state='armed',effect_intent=?1,revision=revision+1,updated_at=?2
                 WHERE assignment_id=?3 AND action_kind='comment'
                   AND state='preparing' AND revision=?4 RETURNING revision",
                params![effect_intent, now, assignment_id, action_revision],
                |row| row.get(0),
            )
            .optional()?;
        let Some(action_revision) = action_revision else {
            transaction.rollback()?;
            return Ok(None);
        };
        transaction.commit()?;
        Ok(Some(action_revision))
    }

    pub fn list_interaction_action_runs(
        &self,
        assignment_id: &str,
    ) -> anyhow::Result<Vec<crate::interaction::InteractionActionRunRecord>> {
        let conn = self.conn()?;
        let mut statement = conn.prepare(
            "SELECT id,owner_kind,owner_id,device_udid,card_identity_json,campaign_id,
                    assignment_id,action_kind,state,revision,effect_intent,evidence_json,
                    error_code,updated_at
             FROM tiktok_action_runs WHERE assignment_id=?1
             ORDER BY CASE action_kind WHEN 'like' THEN 0 WHEN 'save' THEN 1 ELSE 2 END",
        )?;
        let rows = statement.query_map(params![assignment_id], |row| {
            let owner_kind: String = row.get(1)?;
            let kind: String = row.get(7)?;
            let state: String = row.get(8)?;
            Ok(crate::interaction::InteractionActionRunRecord {
                id: row.get(0)?,
                owner_kind: owner_kind_from_label(&owner_kind)?,
                owner_id: row.get(2)?,
                device_udid: row.get(3)?,
                card_identity: row.get(4)?,
                campaign_id: row.get(5)?,
                assignment_id: row.get(6)?,
                kind: action_kind_from_label(&kind)?,
                state: action_state_from_label(&state)?,
                revision: row.get(9)?,
                effect_intent: row.get(10)?,
                evidence: row.get(11)?,
                error: row.get(12)?,
                updated_at: row.get(13)?,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// Create or reopen the durable ledger row for one Nurture card action.
    ///
    /// The caller supplies a stable per-card-attempt `owner_id`. Repeating the call after a
    /// restart returns the same row; database/validation errors are propagated before any UI
    /// effect can be authorized.
    pub fn ensure_tiktok_action_run(
        &self,
        owner: &crate::interaction::TikTokActionOwner,
        kind: crate::interaction::InteractionActionKind,
    ) -> anyhow::Result<crate::interaction::InteractionActionRunRecord> {
        use crate::interaction::TikTokActionOwnerKind;
        if owner.owner_id.trim().is_empty() || owner.device_udid.trim().is_empty() {
            anyhow::bail!("TikTok action owner id and device must be non-empty");
        }
        if owner.kind != TikTokActionOwnerKind::Nurture {
            anyhow::bail!("generic action-run insertion is reserved for nurture owners");
        }
        if let Some(raw) = owner.card_identity.as_deref() {
            serde_json::from_str::<serde_json::Value>(raw)
                .map_err(|error| anyhow::anyhow!("invalid card identity JSON: {error}"))?;
        }
        let conn = self.conn()?;
        let now = Utc::now().to_rfc3339();
        conn.execute(
            "INSERT OR IGNORE INTO tiktok_action_runs
             (id,owner_kind,owner_id,device_udid,card_identity_json,action_kind,state,revision,
              created_at,updated_at)
             VALUES (?1,?2,?3,?4,?5,?6,'planned',0,?7,?7)",
            params![
                Uuid::new_v4().to_string(),
                owner.kind.as_str(),
                owner.owner_id,
                owner.device_udid,
                owner.card_identity,
                action_kind_label(kind),
                now,
            ],
        )?;
        self.get_tiktok_action_run(owner, kind)?
            .ok_or_else(|| anyhow::anyhow!("TikTok action row disappeared after insertion"))
    }

    pub fn get_tiktok_action_run(
        &self,
        owner: &crate::interaction::TikTokActionOwner,
        kind: crate::interaction::InteractionActionKind,
    ) -> anyhow::Result<Option<crate::interaction::InteractionActionRunRecord>> {
        let conn = self.conn()?;
        conn.query_row(
            "SELECT id,owner_kind,owner_id,device_udid,card_identity_json,campaign_id,
                    assignment_id,action_kind,state,revision,effect_intent,evidence_json,
                    error_code,updated_at
             FROM tiktok_action_runs
             WHERE owner_kind=?1 AND owner_id=?2 AND device_udid=?3 AND action_kind=?4",
            params![
                owner.kind.as_str(),
                owner.owner_id,
                owner.device_udid,
                action_kind_label(kind),
            ],
            |row| {
                let owner_kind: String = row.get(1)?;
                let kind: String = row.get(7)?;
                let state: String = row.get(8)?;
                Ok(crate::interaction::InteractionActionRunRecord {
                    id: row.get(0)?,
                    owner_kind: owner_kind_from_label(&owner_kind)?,
                    owner_id: row.get(2)?,
                    device_udid: row.get(3)?,
                    card_identity: row.get(4)?,
                    campaign_id: row.get(5)?,
                    assignment_id: row.get(6)?,
                    kind: action_kind_from_label(&kind)?,
                    state: action_state_from_label(&state)?,
                    revision: row.get(9)?,
                    effect_intent: row.get(10)?,
                    evidence: row.get(11)?,
                    error: row.get(12)?,
                    updated_at: row.get(13)?,
                })
            },
        )
        .optional()
        .map_err(Into::into)
    }

    pub fn claim_tiktok_action(
        &self,
        owner: &crate::interaction::TikTokActionOwner,
        kind: crate::interaction::InteractionActionKind,
    ) -> anyhow::Result<Option<i64>> {
        let conn = self.conn()?;
        conn.query_row(
            "UPDATE tiktok_action_runs
             SET state='preparing',effect_intent=NULL,evidence_json=NULL,error_code=NULL,
                 revision=revision+1,updated_at=?1
             WHERE owner_kind=?2 AND owner_id=?3 AND device_udid=?4 AND action_kind=?5
               AND state IN ('planned','failed_before_effect')
             RETURNING revision",
            params![
                Utc::now().to_rfc3339(),
                owner.kind.as_str(),
                owner.owner_id,
                owner.device_udid,
                action_kind_label(kind),
            ],
            |row| row.get(0),
        )
        .optional()
        .map_err(Into::into)
    }

    pub fn arm_tiktok_action(
        &self,
        owner: &crate::interaction::TikTokActionOwner,
        kind: crate::interaction::InteractionActionKind,
        ownership_revision: i64,
        effect_intent: &str,
    ) -> anyhow::Result<Option<i64>> {
        if effect_intent.trim().is_empty() {
            anyhow::bail!("TikTok action effect intent is empty");
        }
        let conn = self.conn()?;
        conn.query_row(
            "UPDATE tiktok_action_runs
             SET state='armed',effect_intent=?1,revision=revision+1,updated_at=?2
             WHERE owner_kind=?3 AND owner_id=?4 AND device_udid=?5 AND action_kind=?6
               AND state='preparing' AND revision=?7 RETURNING revision",
            params![
                effect_intent,
                Utc::now().to_rfc3339(),
                owner.kind.as_str(),
                owner.owner_id,
                owner.device_udid,
                action_kind_label(kind),
                ownership_revision,
            ],
            |row| row.get(0),
        )
        .optional()
        .map_err(Into::into)
    }

    pub fn settle_tiktok_action(
        &self,
        owner: &crate::interaction::TikTokActionOwner,
        kind: crate::interaction::InteractionActionKind,
        ownership_revision: i64,
        state: crate::interaction::InteractionActionState,
        evidence_json: Option<&str>,
        error_code: Option<&str>,
    ) -> anyhow::Result<bool> {
        use crate::interaction::InteractionActionState;
        if owner.kind == crate::interaction::TikTokActionOwnerKind::Nurture
            && kind == crate::interaction::InteractionActionKind::Follow
            && state == InteractionActionState::Confirmed
        {
            anyhow::bail!("confirmed Nurture Follow must use atomic source/readback settlement");
        }
        let expected = match state {
            InteractionActionState::NoOp | InteractionActionState::FailedBeforeEffect => {
                "preparing"
            }
            InteractionActionState::Confirmed | InteractionActionState::Uncertain => "armed",
            _ => anyhow::bail!("TikTok action cannot settle as {state:?}"),
        };
        if let Some(raw) = evidence_json {
            serde_json::from_str::<serde_json::Value>(raw)
                .map_err(|error| anyhow::anyhow!("invalid TikTok action evidence: {error}"))?;
        }
        let conn = self.conn()?;
        let changed = conn.execute(
            "UPDATE tiktok_action_runs SET state=?1,evidence_json=?2,error_code=?3,
                    revision=revision+1,updated_at=?4
             WHERE owner_kind=?5 AND owner_id=?6 AND device_udid=?7 AND action_kind=?8
               AND state=?9 AND revision=?10",
            params![
                action_state_label(state),
                evidence_json,
                error_code,
                Utc::now().to_rfc3339(),
                owner.kind.as_str(),
                owner.owner_id,
                owner.device_udid,
                action_kind_label(kind),
                expected,
                ownership_revision,
            ],
        )?;
        Ok(changed > 0)
    }

    /// Ensure old assignments gain the action rows described by their normalized request.
    /// Existing rows are immutable with respect to kind and are left untouched.
    pub fn ensure_interaction_action_runs(
        &self,
        campaign_id: &str,
        assignment_id: &str,
        actions: crate::interaction::InteractionActionSet,
    ) -> anyhow::Result<()> {
        let mut conn = self.conn()?;
        let transaction = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let now = Utc::now().to_rfc3339();
        for kind in actions.ordered() {
            transaction.execute(
                "INSERT OR IGNORE INTO tiktok_action_runs
                 (id,owner_kind,owner_id,device_udid,campaign_id,assignment_id,action_kind,
                  state,revision,created_at,updated_at)
                 SELECT ?1,'interaction',?3,a.actor_udid,?2,?3,?4,'planned',0,?5,?5
                 FROM interaction_assignments a WHERE a.id=?3",
                params![
                    Uuid::new_v4().to_string(),
                    campaign_id,
                    assignment_id,
                    action_kind_label(kind),
                    now,
                ],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }

    pub fn claim_interaction_action(
        &self,
        assignment_id: &str,
        kind: crate::interaction::InteractionActionKind,
    ) -> anyhow::Result<Option<i64>> {
        let conn = self.conn()?;
        conn.query_row(
            "UPDATE tiktok_action_runs
             SET state='preparing',effect_intent=NULL,evidence_json=NULL,error_code=NULL,
                 revision=revision+1,updated_at=?1
             WHERE assignment_id=?2 AND action_kind=?3
               AND state IN ('planned','failed_before_effect')
               AND EXISTS (
                   SELECT 1
                   FROM interaction_assignments AS assignment
                   JOIN interaction_campaigns AS campaign ON campaign.id=assignment.campaign_id
                   WHERE assignment.id=tiktok_action_runs.assignment_id
                     AND assignment.state='preparing'
                     AND campaign.state='running'
               )
             RETURNING revision",
            params![
                Utc::now().to_rfc3339(),
                assignment_id,
                action_kind_label(kind)
            ],
            |row| row.get(0),
        )
        .optional()
        .map_err(Into::into)
    }

    /// Cross a public-action effect boundary exactly once.
    pub fn arm_interaction_action(
        &self,
        assignment_id: &str,
        kind: crate::interaction::InteractionActionKind,
        ownership_revision: i64,
        effect_intent: &str,
    ) -> anyhow::Result<Option<i64>> {
        if effect_intent.trim().is_empty() {
            anyhow::bail!("interaction action effect intent is empty");
        }
        let conn = self.conn()?;
        conn.query_row(
            "UPDATE tiktok_action_runs
             SET state='armed',effect_intent=?1,revision=revision+1,updated_at=?2
             WHERE assignment_id=?3 AND action_kind=?4 AND state='preparing' AND revision=?5
               AND EXISTS (
                   SELECT 1
                   FROM interaction_assignments AS assignment
                   JOIN interaction_campaigns AS campaign ON campaign.id=assignment.campaign_id
                   WHERE assignment.id=tiktok_action_runs.assignment_id
                     AND assignment.state='preparing'
                     AND campaign.state='running'
               )
             RETURNING revision",
            params![
                effect_intent,
                Utc::now().to_rfc3339(),
                assignment_id,
                action_kind_label(kind),
                ownership_revision,
            ],
            |row| row.get(0),
        )
        .optional()
        .map_err(Into::into)
    }

    pub fn settle_interaction_action(
        &self,
        assignment_id: &str,
        kind: crate::interaction::InteractionActionKind,
        ownership_revision: i64,
        state: crate::interaction::InteractionActionState,
        evidence_json: Option<&str>,
        error_code: Option<&str>,
    ) -> anyhow::Result<bool> {
        use crate::interaction::InteractionActionState;
        let expected = match state {
            InteractionActionState::NoOp | InteractionActionState::FailedBeforeEffect => {
                "preparing"
            }
            InteractionActionState::Confirmed | InteractionActionState::Uncertain => "armed",
            _ => anyhow::bail!("interaction action cannot settle as {state:?}"),
        };
        if let Some(raw) = evidence_json {
            serde_json::from_str::<serde_json::Value>(raw)
                .map_err(|error| anyhow::anyhow!("invalid interaction action evidence: {error}"))?;
        }
        let conn = self.conn()?;
        let changed = conn.execute(
            "UPDATE tiktok_action_runs
             SET state=?1,evidence_json=?2,error_code=?3,revision=revision+1,updated_at=?4
             WHERE assignment_id=?5 AND action_kind=?6 AND state=?7 AND revision=?8",
            params![
                action_state_label(state),
                evidence_json,
                error_code,
                Utc::now().to_rfc3339(),
                assignment_id,
                action_kind_label(kind),
                expected,
                ownership_revision,
            ],
        )?;
        Ok(changed > 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interaction::{
        plan_threads, InteractionActionKind, InteractionActionSet, InteractionActionState,
        ResolvedTikTokTarget, ThreadCampaignRequest, ThreadMode, ThreadShape, TikTokPostKind,
    };

    fn fixture() -> (Database, std::path::PathBuf) {
        let path = std::env::temp_dir().join(format!(
            "riviu-interaction-actions-test-{}.db",
            Uuid::new_v4()
        ));
        (Database::open(&path).expect("open fixture database"), path)
    }

    fn request(actions: InteractionActionSet) -> ThreadCampaignRequest {
        ThreadCampaignRequest {
            request_id: "action-ledger-1".into(),
            targets: vec![ResolvedTikTokTarget {
                original_url: "https://www.tiktok.com/@creator/video/123".into(),
                normalized_url: "https://www.tiktok.com/@creator/video/123".into(),
                target_key: "content:123".into(),
                content_id: "123".into(),
                author: "creator".into(),
                kind: TikTokPostKind::Video,
            }],
            actor_udids: vec!["actor-a".into(), "actor-b".into()],
            message_count: 2,
            instruction: "tu nhien".into(),
            max_words: 12,
            mode: ThreadMode::Standalone,
            shape: ThreadShape::Star,
            cohort_size: None,
            manual_comments: vec!["mot cau".into(), "hai cau".into()],
            actions,
            mentions: Vec::new(),
            mention_parent: false,
        }
    }

    fn start_assignment(db: &Database, campaign_id: &str, assignment_id: &str) -> i64 {
        db.update_interaction_campaign_state(
            campaign_id,
            crate::interaction::ThreadCampaignState::Running,
            None,
        )
        .expect("mark campaign running");
        db.claim_interaction_assignment_for_send(assignment_id)
            .expect("claim assignment")
            .expect("assignment owner")
    }

    #[test]
    fn campaign_creation_persists_only_the_requested_actions_per_assignment() {
        let (db, path) = fixture();
        let request = request(InteractionActionSet {
            like: true,
            comment: false,
            save: true,
        });
        let plan = plan_threads(&request).expect("plan");
        let campaign = db
            .create_interaction_campaign(&request, &plan)
            .expect("create");
        let detail = db
            .get_interaction_campaign(&campaign)
            .expect("read")
            .expect("campaign");

        assert_eq!(detail.assignments.len(), 2);
        assert_eq!(detail.summary.action_counters.planned, 4);
        assert_eq!(detail.summary.action_counters.attempted, 0);
        assert_eq!(
            db.list_interaction_campaigns(1)
                .expect("list")
                .remove(0)
                .action_counters,
            detail.summary.action_counters
        );
        for assignment in detail.assignments {
            assert_eq!(assignment.actions.len(), 2);
            assert_eq!(assignment.actions[0].kind, InteractionActionKind::Like);
            assert_eq!(assignment.actions[1].kind, InteractionActionKind::Save);
            let runs = db
                .list_interaction_action_runs(&assignment.id)
                .expect("read action rows");
            assert_eq!(runs.len(), 2);
            assert_eq!(runs[0].kind, InteractionActionKind::Like);
            assert_eq!(runs[1].kind, InteractionActionKind::Save);
            assert!(runs
                .iter()
                .all(|run| run.state == InteractionActionState::Planned));
        }
        std::fs::remove_file(path).expect("remove fixture");
    }

    #[test]
    fn every_non_empty_action_set_round_trips_through_campaign_detail_in_execution_order() {
        for mask in 1_u8..=7 {
            let (db, path) = fixture();
            let actions = InteractionActionSet {
                like: mask & 0b001 != 0,
                comment: mask & 0b010 != 0,
                save: mask & 0b100 != 0,
            };
            let mut request = request(actions);
            request.request_id = format!("action-mask-{mask}");
            if !actions.comment {
                request.message_count = 0;
                request.max_words = 0;
                request.manual_comments.clear();
                request.actor_udids.truncate(1);
            }
            let plan = plan_threads(&request).expect("plan action mask");
            let campaign = db
                .create_interaction_campaign(&request, &plan)
                .expect("persist action mask");
            let detail = db
                .get_interaction_campaign(&campaign)
                .expect("read action mask")
                .expect("campaign exists");
            let expected: Vec<_> = actions.ordered().collect();

            assert_eq!(detail.summary.brief.expect("brief").actions, actions);
            assert_eq!(
                detail.assignments[0]
                    .actions
                    .iter()
                    .map(|action| action.kind)
                    .collect::<Vec<_>>(),
                expected,
                "mask {mask:03b}"
            );
            assert_eq!(
                detail.summary.action_counters.planned,
                expected.len() as u32 * detail.assignments.len() as u32,
                "mask {mask:03b}"
            );
            std::fs::remove_file(path).expect("remove fixture");
        }
    }

    #[test]
    fn typed_cleanup_uncertainty_is_durable_and_not_retryable() {
        let (db, path) = fixture();
        let request = request(InteractionActionSet {
            like: false,
            comment: true,
            save: false,
        });
        let plan = plan_threads(&request).expect("plan");
        let campaign = db
            .create_interaction_campaign(&request, &plan)
            .expect("create");
        let assignment = db
            .get_interaction_campaign(&campaign)
            .expect("read")
            .expect("campaign")
            .assignments[0]
            .id
            .clone();
        start_assignment(&db, &campaign, &assignment);
        let claim = db
            .claim_interaction_action(&assignment, InteractionActionKind::Comment)
            .expect("claim")
            .expect("winner");
        let armed = db
            .arm_interaction_action(
                &assignment,
                InteractionActionKind::Comment,
                claim,
                "typed_comment_cleanup_unverified",
            )
            .expect("arm")
            .expect("owner");
        assert!(db
            .settle_interaction_action(
                &assignment,
                InteractionActionKind::Comment,
                armed,
                InteractionActionState::Uncertain,
                Some(r#"{"phase":"typedCleanupUnverified"}"#),
                Some("typed composer cleanup was not verified"),
            )
            .expect("settle"));

        let detail = db
            .get_interaction_campaign(&campaign)
            .expect("read")
            .expect("campaign");
        assert_eq!(
            detail.assignments[0].actions[0].state,
            InteractionActionState::Uncertain
        );
        assert_eq!(detail.summary.action_counters.attempted, 1);
        assert_eq!(detail.summary.action_counters.uncertain, 1);
        assert!(db
            .claim_interaction_action(&assignment, InteractionActionKind::Comment)
            .expect("retry claim")
            .is_none());
        std::fs::remove_file(path).expect("remove fixture");
    }

    #[test]
    fn action_claim_arm_and_settle_are_revision_guarded_and_one_shot() {
        let (db, path) = fixture();
        let request = request(InteractionActionSet {
            like: false,
            comment: false,
            save: true,
        });
        let plan = plan_threads(&request).expect("plan");
        let campaign = db
            .create_interaction_campaign(&request, &plan)
            .expect("create");
        let assignment = db
            .get_interaction_campaign(&campaign)
            .expect("read")
            .expect("campaign")
            .assignments[0]
            .id
            .clone();
        start_assignment(&db, &campaign, &assignment);

        let claim = db
            .claim_interaction_action(&assignment, InteractionActionKind::Save)
            .expect("claim")
            .expect("winner");
        assert!(db
            .claim_interaction_action(&assignment, InteractionActionKind::Save)
            .expect("second claim")
            .is_none());
        assert!(db
            .arm_interaction_action(
                &assignment,
                InteractionActionKind::Save,
                claim + 50,
                "bookmark_saved"
            )
            .expect("stale arm")
            .is_none());
        let armed = db
            .arm_interaction_action(
                &assignment,
                InteractionActionKind::Save,
                claim,
                "bookmark_saved",
            )
            .expect("arm")
            .expect("owner");
        assert!(db
            .arm_interaction_action(
                &assignment,
                InteractionActionKind::Save,
                claim,
                "bookmark_saved",
            )
            .expect("repeat arm")
            .is_none());
        assert!(db
            .settle_interaction_action(
                &assignment,
                InteractionActionKind::Save,
                armed,
                InteractionActionState::Confirmed,
                Some(r#"{"verdict":"saved"}"#),
                None,
            )
            .expect("settle"));
        assert!(!db
            .settle_interaction_action(
                &assignment,
                InteractionActionKind::Save,
                armed,
                InteractionActionState::FailedBeforeEffect,
                None,
                Some("late failure"),
            )
            .expect("stale settle"));

        let run = db
            .list_interaction_action_runs(&assignment)
            .expect("read")
            .pop()
            .expect("row");
        assert_eq!(run.state, InteractionActionState::Confirmed);
        assert_eq!(run.effect_intent.as_deref(), Some("bookmark_saved"));
        std::fs::remove_file(path).expect("remove fixture");
    }

    #[test]
    fn action_claim_and_arm_require_a_running_campaign_and_preparing_assignment() {
        let (db, path) = fixture();
        let request = request(InteractionActionSet {
            like: false,
            comment: false,
            save: true,
        });
        let plan = plan_threads(&request).expect("plan");
        let campaign = db
            .create_interaction_campaign(&request, &plan)
            .expect("create");
        let assignment = db
            .get_interaction_campaign(&campaign)
            .expect("read")
            .expect("campaign")
            .assignments[0]
            .id
            .clone();

        assert!(db
            .claim_interaction_action(&assignment, InteractionActionKind::Save)
            .expect("queued campaign claim")
            .is_none());
        db.update_interaction_campaign_state(
            &campaign,
            crate::interaction::ThreadCampaignState::Running,
            None,
        )
        .expect("mark campaign running");
        assert!(db
            .claim_interaction_action(&assignment, InteractionActionKind::Save)
            .expect("queued assignment claim")
            .is_none());

        db.claim_interaction_assignment_for_send(&assignment)
            .expect("claim assignment")
            .expect("assignment owner");
        let action_revision = db
            .claim_interaction_action(&assignment, InteractionActionKind::Save)
            .expect("claim action")
            .expect("action owner");
        db.update_interaction_campaign_state(
            &campaign,
            crate::interaction::ThreadCampaignState::Cancelled,
            None,
        )
        .expect("cancel campaign without releasing fixture claim");
        assert!(db
            .arm_interaction_action(
                &assignment,
                InteractionActionKind::Save,
                action_revision,
                "bookmark_saved",
            )
            .expect("cancelled arm")
            .is_none());

        std::fs::remove_file(path).expect("remove fixture");
    }

    #[test]
    fn cancel_between_like_and_save_releases_the_save_claim_before_any_tap() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let (db, path) = fixture();
        let request = request(InteractionActionSet {
            like: true,
            comment: false,
            save: true,
        });
        let plan = plan_threads(&request).expect("plan");
        let campaign = db
            .create_interaction_campaign(&request, &plan)
            .expect("create");
        let assignment = db
            .get_interaction_campaign(&campaign)
            .expect("read")
            .expect("campaign")
            .assignments[0]
            .id
            .clone();
        start_assignment(&db, &campaign, &assignment);

        let like_claim = db
            .claim_interaction_action(&assignment, InteractionActionKind::Like)
            .expect("claim Like")
            .expect("Like owner");
        let like_armed = db
            .arm_interaction_action(
                &assignment,
                InteractionActionKind::Like,
                like_claim,
                "like_desired_state",
            )
            .expect("arm Like")
            .expect("Like gate owner");
        assert!(db
            .settle_interaction_action(
                &assignment,
                InteractionActionKind::Like,
                like_armed,
                InteractionActionState::Confirmed,
                Some(r#"{"verdict":"liked"}"#),
                None,
            )
            .expect("settle Like"));

        // Interleave Cancel after Save has claimed its row but before the one-shot gate.
        let save_claim = db
            .claim_interaction_action(&assignment, InteractionActionKind::Save)
            .expect("claim Save")
            .expect("Save owner");
        db.cancel_interaction_campaign(&campaign)
            .expect("cancel campaign");

        let taps = AtomicUsize::new(0);
        let mut gate = crate::interaction_target::ActionEffectGate::new(|| {
            Ok(db
                .arm_interaction_action(
                    &assignment,
                    InteractionActionKind::Save,
                    save_claim,
                    "bookmark_saved",
                )?
                .is_some())
        });
        if gate.cross().is_ok() {
            taps.fetch_add(1, Ordering::Relaxed);
        }
        assert_eq!(taps.load(Ordering::Relaxed), 0);
        assert!(!gate.crossed());

        let detail = db
            .get_interaction_campaign(&campaign)
            .expect("read cancelled campaign")
            .expect("campaign");
        let like = detail.assignments[0]
            .actions
            .iter()
            .find(|action| action.kind == InteractionActionKind::Like)
            .expect("Like result");
        let save = detail.assignments[0]
            .actions
            .iter()
            .find(|action| action.kind == InteractionActionKind::Save)
            .expect("Save result");
        assert_eq!(like.state, InteractionActionState::Confirmed);
        assert_eq!(save.state, InteractionActionState::FailedBeforeEffect);
        assert!(db
            .claim_interaction_action(&assignment, InteractionActionKind::Save)
            .expect("cancelled campaign blocks a fresh Save claim")
            .is_none());

        std::fs::remove_file(path).expect("remove fixture");
    }

    #[test]
    fn campaign_cancel_and_terminal_settlement_are_mutually_exclusive() {
        let (db, path) = fixture();

        let terminal_first = request(InteractionActionSet {
            like: true,
            comment: false,
            save: false,
        });
        let plan = plan_threads(&terminal_first).expect("terminal-first plan");
        let terminal_campaign = db
            .create_interaction_campaign(&terminal_first, &plan)
            .expect("create terminal-first campaign");
        db.update_interaction_campaign_state(
            &terminal_campaign,
            crate::interaction::ThreadCampaignState::Running,
            None,
        )
        .expect("start terminal-first campaign");
        assert!(db
            .settle_interaction_campaign_if_running(
                &terminal_campaign,
                crate::interaction::ThreadCampaignState::Succeeded,
                None,
            )
            .expect("terminal settlement wins"));
        assert_eq!(
            db.cancel_interaction_campaign(&terminal_campaign)
                .expect("stale cancel loses"),
            0
        );
        assert_eq!(
            db.get_interaction_campaign(&terminal_campaign)
                .expect("read terminal-first campaign")
                .expect("campaign")
                .summary
                .state,
            crate::interaction::ThreadCampaignState::Succeeded
        );

        let mut cancelled_first = terminal_first;
        cancelled_first.request_id = "action-ledger-cancel-first".into();
        let plan = plan_threads(&cancelled_first).expect("cancel-first plan");
        let cancelled_campaign = db
            .create_interaction_campaign(&cancelled_first, &plan)
            .expect("create cancel-first campaign");
        db.update_interaction_campaign_state(
            &cancelled_campaign,
            crate::interaction::ThreadCampaignState::Running,
            None,
        )
        .expect("start cancel-first campaign");
        db.cancel_interaction_campaign(&cancelled_campaign)
            .expect("cancel wins");
        assert!(!db
            .settle_interaction_campaign_if_running(
                &cancelled_campaign,
                crate::interaction::ThreadCampaignState::Partial,
                Some("stale join"),
            )
            .expect("stale terminal settlement loses"));
        assert_eq!(
            db.get_interaction_campaign(&cancelled_campaign)
                .expect("read cancel-first campaign")
                .expect("campaign")
                .summary
                .state,
            crate::interaction::ThreadCampaignState::Cancelled
        );

        std::fs::remove_file(path).expect("remove fixture");
    }

    #[test]
    fn cancel_keeps_an_armed_action_and_its_assignment_non_retryable() {
        let (db, path) = fixture();
        let request = request(InteractionActionSet {
            like: false,
            comment: false,
            save: true,
        });
        let plan = plan_threads(&request).expect("plan");
        let campaign = db
            .create_interaction_campaign(&request, &plan)
            .expect("create campaign");
        let assignment = db
            .get_interaction_campaign(&campaign)
            .expect("read campaign")
            .expect("campaign")
            .assignments[0]
            .id
            .clone();
        let assignment_revision = start_assignment(&db, &campaign, &assignment);
        let claim = db
            .claim_interaction_action(&assignment, InteractionActionKind::Save)
            .expect("claim Save")
            .expect("Save owner");
        db.arm_interaction_action(
            &assignment,
            InteractionActionKind::Save,
            claim,
            "bookmark_saved",
        )
        .expect("arm Save")
        .expect("Save gate owner");

        db.cancel_interaction_campaign(&campaign)
            .expect("cancel after effect intent");
        assert!(!db
            .settle_owned_interaction_assignment(
                &assignment,
                assignment_revision,
                crate::interaction::ThreadMessageState::Failed,
                Some("stale pre-effect result"),
                None,
            )
            .expect("stale worker cannot lower cancellation classification"));
        let detail = db
            .get_interaction_campaign(&campaign)
            .expect("read cancelled campaign")
            .expect("campaign");
        assert_eq!(
            detail.assignments[0].state,
            crate::interaction::ThreadMessageState::Uncertain
        );
        assert_eq!(
            detail.assignments[0].actions[0].state,
            InteractionActionState::Armed
        );
        assert!(db
            .claim_interaction_assignment_for_send(&assignment)
            .expect("uncertain assignment is not retryable")
            .is_none());

        std::fs::remove_file(path).expect("remove fixture");
    }

    #[test]
    fn comment_effect_boundary_arms_assignment_and_action_in_one_transaction() {
        let (db, path) = fixture();
        let request = request(InteractionActionSet {
            like: false,
            comment: true,
            save: false,
        });
        let plan = plan_threads(&request).expect("plan");
        let campaign = db
            .create_interaction_campaign(&request, &plan)
            .expect("create");
        db.update_interaction_campaign_state(
            &campaign,
            crate::interaction::ThreadCampaignState::Running,
            None,
        )
        .expect("mark campaign running");
        let assignment = db
            .get_interaction_campaign(&campaign)
            .expect("read")
            .expect("campaign")
            .assignments
            .into_iter()
            .next()
            .expect("assignment");
        let assignment_claim = db
            .claim_interaction_assignment_for_send(&assignment.id)
            .expect("claim assignment")
            .expect("assignment winner");
        let prepared = crate::interaction::PreparedThreadMessage::new(&plan.assignments[0], "ok");
        let assignment_revision = db
            .prepare_interaction_assignment(&assignment.id, assignment_claim, &prepared)
            .expect("prepare")
            .expect("preparation winner");
        let action_revision = db
            .claim_interaction_action(&assignment.id, InteractionActionKind::Comment)
            .expect("claim action")
            .expect("action winner");

        assert!(db
            .begin_interaction_comment_action_effect(
                &assignment.id,
                assignment_revision,
                action_revision + 1,
                "post_comment",
            )
            .expect("stale atomic boundary")
            .is_none());
        let still_preparing = db
            .get_interaction_campaign(&campaign)
            .expect("read after loser")
            .expect("campaign")
            .assignments
            .into_iter()
            .next()
            .expect("assignment");
        assert_eq!(
            still_preparing.state,
            crate::interaction::ThreadMessageState::Preparing,
            "a losing action CAS must roll the assignment transition back"
        );

        let armed_revision = db
            .begin_interaction_comment_action_effect(
                &assignment.id,
                assignment_revision,
                action_revision,
                "post_comment",
            )
            .expect("atomic boundary")
            .expect("winner");
        assert!(armed_revision > action_revision);
        let detail = db
            .get_interaction_campaign(&campaign)
            .expect("read armed")
            .expect("campaign");
        assert_eq!(
            detail.assignments[0].state,
            crate::interaction::ThreadMessageState::Sending
        );
        assert_eq!(
            detail.assignments[0].actions[0].state,
            InteractionActionState::Armed
        );
        assert_eq!(
            detail.assignments[0].actions[0].effect_intent.as_deref(),
            Some("post_comment")
        );
        std::fs::remove_file(path).expect("remove fixture");
    }

    #[test]
    fn each_action_settles_without_erasing_its_siblings() {
        let (db, path) = fixture();
        let request = request(InteractionActionSet {
            like: true,
            comment: true,
            save: true,
        });
        let plan = plan_threads(&request).expect("plan");
        let campaign = db
            .create_interaction_campaign(&request, &plan)
            .expect("create");
        let assignment = db
            .get_interaction_campaign(&campaign)
            .expect("read")
            .expect("campaign")
            .assignments[0]
            .id
            .clone();
        start_assignment(&db, &campaign, &assignment);

        for (kind, state) in [
            (
                InteractionActionKind::Like,
                InteractionActionState::Confirmed,
            ),
            (
                InteractionActionKind::Save,
                InteractionActionState::FailedBeforeEffect,
            ),
            (
                InteractionActionKind::Comment,
                InteractionActionState::Confirmed,
            ),
        ] {
            let claim = db
                .claim_interaction_action(&assignment, kind)
                .expect("claim")
                .expect("winner");
            let revision = if state.effect_may_have_gone_out() {
                db.arm_interaction_action(&assignment, kind, claim, kind.as_str())
                    .expect("arm")
                    .expect("owner")
            } else {
                claim
            };
            assert!(db
                .settle_interaction_action(&assignment, kind, revision, state, None, None)
                .expect("settle"));
        }

        let runs = db.list_interaction_action_runs(&assignment).expect("read");
        assert_eq!(runs[0].state, InteractionActionState::Confirmed);
        assert_eq!(runs[1].state, InteractionActionState::FailedBeforeEffect);
        assert_eq!(runs[2].state, InteractionActionState::Confirmed);
        std::fs::remove_file(path).expect("remove fixture");
    }

    #[test]
    fn restart_releases_pre_effect_claims_but_makes_armed_actions_uncertain() {
        let (db, path) = fixture();
        let request = request(InteractionActionSet {
            like: true,
            comment: false,
            save: true,
        });
        let plan = plan_threads(&request).expect("plan");
        let campaign = db
            .create_interaction_campaign(&request, &plan)
            .expect("create");
        db.update_interaction_campaign_state(
            &campaign,
            crate::interaction::ThreadCampaignState::Running,
            None,
        )
        .expect("running");
        let assignment = &db
            .get_interaction_campaign(&campaign)
            .expect("read")
            .expect("campaign")
            .assignments[0]
            .id;
        db.claim_interaction_assignment_for_send(assignment)
            .expect("claim assignment")
            .expect("assignment owner");
        let like = db
            .claim_interaction_action(assignment, InteractionActionKind::Like)
            .expect("like claim")
            .expect("winner");
        let save = db
            .claim_interaction_action(assignment, InteractionActionKind::Save)
            .expect("save claim")
            .expect("winner");
        db.arm_interaction_action(
            assignment,
            InteractionActionKind::Save,
            save,
            "bookmark_saved",
        )
        .expect("arm")
        .expect("owner");
        assert!(like > 0);

        assert_eq!(
            db.interrupt_orphaned_interaction_campaigns()
                .expect("recover"),
            1
        );
        let runs = db.list_interaction_action_runs(assignment).expect("read");
        assert_eq!(runs[0].state, InteractionActionState::FailedBeforeEffect);
        assert_eq!(runs[1].state, InteractionActionState::Uncertain);
        std::fs::remove_file(path).expect("remove fixture");
    }
}
