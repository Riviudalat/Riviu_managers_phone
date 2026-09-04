//! Durable write-ahead journal for reversing campaign-owned public effects.

use super::*;
use crate::tiktok_public_cleanup::{
    PublicCleanupKind, PublicCleanupRecovery, PublicCleanupRunRecord, PublicCleanupRunState,
    PublicCleanupSourceAction,
};

fn kind_label(kind: PublicCleanupKind) -> anyhow::Result<&'static str> {
    match kind {
        PublicCleanupKind::Like => Ok("like"),
        PublicCleanupKind::Save => Ok("save"),
        _ => anyhow::bail!("only measured Like/Save cleanup can create a journal row"),
    }
}

fn kind_from_label(label: &str) -> rusqlite::Result<PublicCleanupKind> {
    match label {
        "like" => Ok(PublicCleanupKind::Like),
        "save" => Ok(PublicCleanupKind::Save),
        _ => Err(rusqlite::Error::InvalidQuery),
    }
}

fn state_label(state: PublicCleanupRunState) -> &'static str {
    match state {
        PublicCleanupRunState::Planned => "planned",
        PublicCleanupRunState::Preparing => "preparing",
        PublicCleanupRunState::Armed => "armed",
        PublicCleanupRunState::Cleared => "cleared",
        PublicCleanupRunState::AlreadyClear => "already_clear",
        PublicCleanupRunState::FailedBeforeEffect => "failed_before_effect",
        PublicCleanupRunState::Uncertain => "uncertain",
    }
}

fn state_from_label(label: &str) -> rusqlite::Result<PublicCleanupRunState> {
    match label {
        "planned" => Ok(PublicCleanupRunState::Planned),
        "preparing" => Ok(PublicCleanupRunState::Preparing),
        "armed" => Ok(PublicCleanupRunState::Armed),
        "cleared" => Ok(PublicCleanupRunState::Cleared),
        "already_clear" => Ok(PublicCleanupRunState::AlreadyClear),
        "failed_before_effect" => Ok(PublicCleanupRunState::FailedBeforeEffect),
        "uncertain" => Ok(PublicCleanupRunState::Uncertain),
        _ => Err(rusqlite::Error::InvalidQuery),
    }
}

fn cleanup_record(row: &rusqlite::Row<'_>) -> rusqlite::Result<PublicCleanupRunRecord> {
    let kind: String = row.get(6)?;
    let state: String = row.get(8)?;
    Ok(PublicCleanupRunRecord {
        id: row.get(0)?,
        request_id: row.get(1)?,
        source_action_run_id: row.get(2)?,
        campaign_id: row.get(3)?,
        assignment_id: row.get(4)?,
        device_udid: row.get(5)?,
        kind: kind_from_label(&kind)?,
        target_json: row.get(7)?,
        state: state_from_label(&state)?,
        revision: row.get(9)?,
        effect_intent: row.get(10)?,
        evidence: row.get(11)?,
        error: row.get(12)?,
        updated_at: row.get(13)?,
    })
}

const CLEANUP_COLUMNS: &str =
    "id,request_id,source_action_run_id,campaign_id,assignment_id,device_udid,action_kind,\
     target_json,state,revision,effect_intent,evidence_json,error_code,updated_at";

fn read_cleanup_by_source(
    connection: &Connection,
    source_action_run_id: &str,
) -> anyhow::Result<Option<PublicCleanupRunRecord>> {
    connection
        .query_row(
            &format!(
                "SELECT {CLEANUP_COLUMNS} FROM public_cleanup_runs WHERE source_action_run_id=?1"
            ),
            [source_action_run_id],
            cleanup_record,
        )
        .optional()
        .map_err(Into::into)
}

impl Database {
    /// Read the immutable action/target join without manufacturing a cleanup capability.
    pub fn interaction_public_cleanup_source(
        &self,
        campaign_id: &str,
        assignment_id: &str,
        kind: PublicCleanupKind,
    ) -> anyhow::Result<Option<PublicCleanupSourceAction>> {
        let label = kind_label(kind)?;
        let connection = self.conn()?;
        connection
            .query_row(
                "SELECT action.id,action.device_udid,target.target_key,action.state
                 FROM tiktok_action_runs AS action
                 JOIN interaction_assignments AS assignment
                   ON assignment.id=action.assignment_id
                 JOIN interaction_targets AS target ON target.id=assignment.target_id
                 WHERE action.owner_kind='interaction' AND action.campaign_id=?1
                   AND action.assignment_id=?2 AND action.action_kind=?3",
                params![campaign_id, assignment_id, label],
                |row| {
                    let state: String = row.get(3)?;
                    Ok(PublicCleanupSourceAction {
                        action_run_id: row.get(0)?,
                        campaign_id: campaign_id.to_owned(),
                        assignment_id: assignment_id.to_owned(),
                        device_udid: row.get(1)?,
                        target_key: row.get(2)?,
                        kind,
                        source_confirmed: state == "confirmed",
                    })
                },
            )
            .optional()
            .map_err(Into::into)
    }

    /// Create the one journal row owned by a confirmed source action, or return it unchanged.
    pub fn ensure_public_cleanup_run(
        &self,
        request_id: &str,
        source: &PublicCleanupSourceAction,
        target: &crate::ResolvedTikTokTarget,
    ) -> anyhow::Result<PublicCleanupRunRecord> {
        anyhow::ensure!(!request_id.trim().is_empty(), "cleanup request id is empty");
        anyhow::ensure!(
            source.source_confirmed,
            "source public action is not confirmed"
        );
        anyhow::ensure!(
            source.target_key == target.target_key,
            "cleanup source target does not match the canonical target"
        );
        let label = kind_label(source.kind)?;
        let target_json = serde_json::to_string(target)?;
        let mut connection = self.conn()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;

        if let Some(existing) = read_cleanup_by_source(&transaction, &source.action_run_id)? {
            anyhow::ensure!(
                existing.campaign_id == source.campaign_id
                    && existing.assignment_id == source.assignment_id
                    && existing.device_udid == source.device_udid
                    && existing.kind == source.kind
                    && existing.target_json == target_json,
                "existing cleanup journal does not match its immutable source"
            );
            transaction.commit()?;
            return Ok(existing);
        }

        let now = Utc::now().to_rfc3339();
        let id = Uuid::new_v4().to_string();
        let inserted = transaction.execute(
            "INSERT INTO public_cleanup_runs
             (id,request_id,source_action_run_id,campaign_id,assignment_id,device_udid,
              action_kind,target_json,state,revision,created_at,updated_at)
             SELECT ?1,?2,action.id,action.campaign_id,action.assignment_id,action.device_udid,
                    action.action_kind,?3,'planned',0,?4,?4
             FROM tiktok_action_runs AS action
             JOIN interaction_assignments AS assignment ON assignment.id=action.assignment_id
             JOIN interaction_targets AS target ON target.id=assignment.target_id
             WHERE action.id=?5 AND action.owner_kind='interaction' AND action.state='confirmed'
               AND action.campaign_id=?6 AND action.assignment_id=?7
               AND action.device_udid=?8 AND action.action_kind=?9 AND target.target_key=?10",
            params![
                id,
                request_id,
                target_json,
                now,
                source.action_run_id,
                source.campaign_id,
                source.assignment_id,
                source.device_udid,
                label,
                source.target_key,
            ],
        )?;
        anyhow::ensure!(
            inserted == 1,
            "confirmed source action changed before cleanup journal creation"
        );
        let record = read_cleanup_by_source(&transaction, &source.action_run_id)?
            .ok_or_else(|| anyhow::anyhow!("cleanup journal disappeared after insertion"))?;
        transaction.commit()?;
        Ok(record)
    }

    pub fn get_public_cleanup_run(
        &self,
        run_id: &str,
    ) -> anyhow::Result<Option<PublicCleanupRunRecord>> {
        let connection = self.conn()?;
        connection
            .query_row(
                &format!("SELECT {CLEANUP_COLUMNS} FROM public_cleanup_runs WHERE id=?1"),
                [run_id],
                cleanup_record,
            )
            .optional()
            .map_err(Into::into)
    }

    /// Claim only work that has provably not crossed its effect boundary.
    pub fn claim_public_cleanup(&self, run_id: &str) -> anyhow::Result<Option<i64>> {
        let connection = self.conn()?;
        connection
            .query_row(
                "UPDATE public_cleanup_runs
                 SET state='preparing',effect_intent=NULL,evidence_json=NULL,error_code=NULL,
                     revision=revision+1,updated_at=?1
                 WHERE id=?2 AND state IN ('planned','failed_before_effect') RETURNING revision",
                params![Utc::now().to_rfc3339(), run_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(Into::into)
    }

    /// Persist the one-shot boundary immediately before the unlike/unsave tap.
    pub fn arm_public_cleanup(
        &self,
        run_id: &str,
        ownership_revision: i64,
        effect_intent: &str,
    ) -> anyhow::Result<Option<i64>> {
        anyhow::ensure!(
            !effect_intent.trim().is_empty(),
            "cleanup effect intent is empty"
        );
        let connection = self.conn()?;
        connection
            .query_row(
                "UPDATE public_cleanup_runs
                 SET state='armed',effect_intent=?1,revision=revision+1,updated_at=?2
                 WHERE id=?3 AND state='preparing' AND revision=?4 RETURNING revision",
                params![
                    effect_intent,
                    Utc::now().to_rfc3339(),
                    run_id,
                    ownership_revision
                ],
                |row| row.get(0),
            )
            .optional()
            .map_err(Into::into)
    }

    pub fn settle_public_cleanup(
        &self,
        run_id: &str,
        ownership_revision: i64,
        state: PublicCleanupRunState,
        evidence_json: Option<&str>,
        error_code: Option<&str>,
    ) -> anyhow::Result<bool> {
        let expected = match state {
            PublicCleanupRunState::AlreadyClear | PublicCleanupRunState::FailedBeforeEffect => {
                "preparing"
            }
            PublicCleanupRunState::Cleared | PublicCleanupRunState::Uncertain => "armed",
            _ => anyhow::bail!("cleanup cannot settle as {state:?}"),
        };
        if let Some(raw) = evidence_json {
            serde_json::from_str::<serde_json::Value>(raw)
                .map_err(|error| anyhow::anyhow!("invalid cleanup evidence JSON: {error}"))?;
        }
        let connection = self.conn()?;
        let changed = connection.execute(
            "UPDATE public_cleanup_runs
             SET state=?1,evidence_json=?2,error_code=?3,revision=revision+1,updated_at=?4
             WHERE id=?5 AND state=?6 AND revision=?7",
            params![
                state_label(state),
                evidence_json,
                error_code,
                Utc::now().to_rfc3339(),
                run_id,
                expected,
                ownership_revision,
            ],
        )?;
        Ok(changed == 1)
    }

    /// Reconcile process loss without guessing whether an armed tap landed.
    pub fn recover_orphaned_public_cleanups(&self) -> anyhow::Result<PublicCleanupRecovery> {
        let mut connection = self.conn()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let now = Utc::now().to_rfc3339();
        let retryable = transaction.execute(
            "UPDATE public_cleanup_runs
             SET state='failed_before_effect',error_code='cleanup_worker_lost_before_effect',
                 revision=revision+1,updated_at=?1 WHERE state='preparing'",
            [&now],
        )?;
        let uncertain = transaction.execute(
            "UPDATE public_cleanup_runs
             SET state='uncertain',error_code='cleanup_worker_lost_after_effect_intent',
                 revision=revision+1,updated_at=?1 WHERE state='armed'",
            [&now],
        )?;
        transaction.commit()?;
        Ok(PublicCleanupRecovery {
            retryable: u32::try_from(retryable).unwrap_or(u32::MAX),
            uncertain: u32::try_from(uncertain).unwrap_or(u32::MAX),
        })
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
        let path =
            std::env::temp_dir().join(format!("riviu-public-cleanup-test-{}.db", Uuid::new_v4()));
        (Database::open(&path).expect("open fixture"), path)
    }

    fn target(content_id: &str) -> ResolvedTikTokTarget {
        ResolvedTikTokTarget {
            original_url: format!("https://www.tiktok.com/@creator/video/{content_id}"),
            normalized_url: format!("https://www.tiktok.com/@creator/video/{content_id}"),
            target_key: format!("content:{content_id}"),
            content_id: content_id.to_owned(),
            author: "creator".into(),
            kind: TikTokPostKind::Video,
        }
    }

    fn confirmed_source(
        db: &Database,
        content_id: &str,
        kind: InteractionActionKind,
    ) -> (PublicCleanupSourceAction, ResolvedTikTokTarget) {
        let target = target(content_id);
        let actions = InteractionActionSet {
            like: kind == InteractionActionKind::Like,
            save: kind == InteractionActionKind::Save,
            comment: false,
        };
        let request = ThreadCampaignRequest {
            request_id: format!("request-{content_id}"),
            targets: vec![target.clone()],
            actor_udids: vec![format!("phone-{content_id}")],
            message_count: 0,
            instruction: String::new(),
            max_words: 0,
            mode: ThreadMode::Standalone,
            shape: ThreadShape::Chain,
            cohort_size: None,
            manual_comments: Vec::new(),
            actions,
            mentions: Vec::new(),
            mention_parent: false,
        };
        let plan = plan_threads(&request).expect("plan");
        let campaign_id = db
            .create_interaction_campaign(&request, &plan)
            .expect("campaign");
        db.update_interaction_campaign_state(
            &campaign_id,
            crate::ThreadCampaignState::Running,
            None,
        )
        .expect("start campaign");
        let assignment = db
            .get_interaction_campaign(&campaign_id)
            .expect("detail")
            .expect("campaign exists")
            .assignments
            .remove(0);
        db.claim_interaction_assignment_for_send(&assignment.id)
            .expect("claim assignment")
            .expect("assignment owner");
        let action_revision = db
            .claim_interaction_action(&assignment.id, kind)
            .expect("claim action")
            .expect("action owner");
        let armed = db
            .arm_interaction_action(&assignment.id, kind, action_revision, "fixture_effect")
            .expect("arm action")
            .expect("armed owner");
        assert!(db
            .settle_interaction_action(
                &assignment.id,
                kind,
                armed,
                InteractionActionState::Confirmed,
                Some(r#"{"fixture":true}"#),
                None,
            )
            .expect("settle action"));
        let cleanup_kind = match kind {
            InteractionActionKind::Like => PublicCleanupKind::Like,
            InteractionActionKind::Save => PublicCleanupKind::Save,
            _ => unreachable!("fixture only builds reversible toggles"),
        };
        let source = db
            .interaction_public_cleanup_source(&campaign_id, &assignment.id, cleanup_kind)
            .expect("source query")
            .expect("source exists");
        (source, target)
    }

    #[test]
    fn cleanup_journal_is_idempotent_revision_guarded_and_terminal() {
        let (db, path) = fixture();
        let (source, target) = confirmed_source(&db, "101", InteractionActionKind::Like);
        let created = db
            .ensure_public_cleanup_run("cleanup-request-101", &source, &target)
            .expect("journal");
        let repeated = db
            .ensure_public_cleanup_run(
                "a-new-request-id-is-still-the-same-cleanup",
                &source,
                &target,
            )
            .expect("idempotent journal");
        assert_eq!(created.id, repeated.id);

        let claim = db
            .claim_public_cleanup(&created.id)
            .expect("claim")
            .expect("owner");
        assert!(db
            .claim_public_cleanup(&created.id)
            .expect("repeat claim")
            .is_none());
        assert!(db
            .arm_public_cleanup(&created.id, claim + 1, "unlike")
            .expect("stale arm")
            .is_none());
        let armed = db
            .arm_public_cleanup(&created.id, claim, "unlike")
            .expect("arm")
            .expect("armed");
        assert!(db
            .settle_public_cleanup(
                &created.id,
                armed,
                PublicCleanupRunState::Cleared,
                Some(r#"{"verdict":"cleared","effectBoundaryCrossed":true}"#),
                None,
            )
            .expect("settle"));
        let final_record = db
            .get_public_cleanup_run(&created.id)
            .expect("read")
            .expect("exists");
        assert_eq!(final_record.state, PublicCleanupRunState::Cleared);
        assert!(!final_record.state.retry_is_safe());
        assert!(db
            .claim_public_cleanup(&created.id)
            .expect("terminal claim")
            .is_none());

        drop(db);
        std::fs::remove_file(path).expect("remove fixture");
    }

    #[test]
    fn restart_releases_pre_effect_claim_and_quarantines_armed_cleanup() {
        let (db, path) = fixture();
        let (like_source, like_target) = confirmed_source(&db, "201", InteractionActionKind::Like);
        let (save_source, save_target) = confirmed_source(&db, "202", InteractionActionKind::Save);
        let retryable = db
            .ensure_public_cleanup_run("cleanup-request-201", &like_source, &like_target)
            .expect("like cleanup");
        let uncertain = db
            .ensure_public_cleanup_run("cleanup-request-202", &save_source, &save_target)
            .expect("save cleanup");
        db.claim_public_cleanup(&retryable.id)
            .expect("claim retryable")
            .expect("retryable owner");
        let claim = db
            .claim_public_cleanup(&uncertain.id)
            .expect("claim uncertain")
            .expect("uncertain owner");
        db.arm_public_cleanup(&uncertain.id, claim, "unsave")
            .expect("arm uncertain")
            .expect("armed");

        assert_eq!(
            db.recover_orphaned_public_cleanups().expect("recover"),
            PublicCleanupRecovery {
                retryable: 1,
                uncertain: 1,
            }
        );
        let retried = db.claim_public_cleanup(&retryable.id).expect("retry claim");
        assert!(
            retried.is_some(),
            "pre-effect process loss must be retryable"
        );
        assert!(db
            .claim_public_cleanup(&uncertain.id)
            .expect("uncertain claim")
            .is_none());
        assert_eq!(
            db.get_public_cleanup_run(&uncertain.id)
                .expect("read uncertain")
                .expect("exists")
                .state,
            PublicCleanupRunState::Uncertain
        );

        drop(db);
        std::fs::remove_file(path).expect("remove fixture");
    }
}
