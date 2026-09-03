//! Interaction campaigns: the plan, each assignment's state, and the evidence filed
//! against them.

use super::*;

impl Database {
    pub fn create_interaction_campaign(
        &self,
        request: &crate::interaction::ThreadCampaignRequest,
        plan: &crate::interaction::ThreadPlan,
    ) -> anyhow::Result<String> {
        self.create_interaction_campaign_with_id(&Uuid::new_v4().to_string(), request, plan)
            .map(|(id, _created)| id)
    }

    /// Create the exact child an orchestration attempt armed, or prove that an identical
    /// retry already created it. A matching request ID under any other campaign ID is a
    /// conflict: reconciliation addresses the persisted child ID, never a substitute.
    pub fn create_interaction_campaign_with_id(
        &self,
        campaign_id: &str,
        request: &crate::interaction::ThreadCampaignRequest,
        plan: &crate::interaction::ThreadPlan,
    ) -> anyhow::Result<(String, bool)> {
        request.validate().map_err(|error| anyhow::anyhow!(error))?;
        let expected_plan =
            crate::interaction::plan_threads(request).map_err(|error| anyhow::anyhow!(error))?;
        anyhow::ensure!(
            &expected_plan == plan,
            "interaction plan does not match its immutable request"
        );
        let mut conn = self.conn()?;
        let transaction = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let request_json = serde_json::to_string(request)?;
        let existing = transaction
            .query_row(
                "SELECT id,request_json FROM interaction_campaigns \
                 WHERE id=?1 OR request_id=?2 LIMIT 1",
                params![campaign_id, request.request_id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()?;
        if let Some((existing_id, existing_request)) = existing {
            anyhow::ensure!(
                existing_id == campaign_id && existing_request == request_json,
                "interaction child idempotency conflict"
            );
            transaction.commit()?;
            return Ok((existing_id, false));
        }
        let now = Utc::now().to_rfc3339();
        transaction.execute(
            "INSERT INTO interaction_campaigns
             (id,request_id,request_json,state,message_count,revision,created_at,updated_at)
             VALUES (?1,?2,?3,'queued',?4,0,?5,?5)",
            params![
                campaign_id,
                request.request_id,
                request_json,
                i64::from(request.message_count),
                now,
            ],
        )?;
        for (ordinal, udid) in request.actor_udids.iter().enumerate() {
            transaction.execute(
                "INSERT INTO interaction_campaign_actors
                 (campaign_id,actor_ordinal,udid) VALUES (?1,?2,?3)",
                params![campaign_id, ordinal as i64, udid],
            )?;
        }

        let mut target_ids = std::collections::HashMap::new();
        for (line_index, target) in request.targets.iter().enumerate() {
            let target_id = Uuid::new_v4().to_string();
            let kind = match target.kind {
                crate::interaction::TikTokPostKind::Video => "video",
                crate::interaction::TikTokPostKind::Photo => "photo",
            };
            transaction.execute(
                "INSERT INTO interaction_targets
                 (id,campaign_id,line_no,original_url,normalized_url,target_key,content_id,kind,created_at)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
                params![
                    target_id,
                    campaign_id,
                    (line_index + 1) as i64,
                    target.original_url,
                    target.normalized_url,
                    target.target_key,
                    target.content_id,
                    kind,
                    now,
                ],
            )?;
            target_ids.insert(target.target_key.clone(), target_id);
        }

        let mut assignment_ids = std::collections::HashMap::new();
        for assignment in &plan.assignments {
            let assignment_id = Uuid::new_v4().to_string();
            let target_id = target_ids
                .get(&assignment.target_key)
                .ok_or_else(|| anyhow::anyhow!("plan target is missing from request"))?;
            let parent_id = assignment.parent_ordinal.and_then(|ordinal| {
                assignment_ids
                    .get(&(assignment.target_key.clone(), ordinal))
                    .cloned()
            });
            transaction.execute(
                "INSERT INTO interaction_assignments
                 (id,campaign_id,target_id,message_ordinal,actor_udid,parent_assignment_id,created_at,updated_at)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?7)",
                params![
                    assignment_id,
                    campaign_id,
                    target_id,
                    i64::from(assignment.ordinal),
                    assignment.actor_udid,
                    parent_id,
                    now,
                ],
            )?;
            super::interaction_actions::insert_interaction_action_runs(
                &transaction,
                campaign_id,
                &assignment_id,
                &assignment.actor_udid,
                request.actions,
                &now,
            )?;
            assignment_ids.insert(
                (assignment.target_key.clone(), assignment.ordinal),
                assignment_id,
            );
        }
        // `interaction_dispatch` used to get a 'queued' row here. Migration 14 dropped the
        // table: it was shaped as a single-owner lease and nothing ever claimed it, so the
        // row proved nothing while looking like proof. If two app instances over one data
        // directory ever becomes possible, the guard belongs here — it never was one.
        transaction.commit()?;
        Ok((campaign_id.to_string(), true))
    }
    pub fn list_interaction_campaigns(
        &self,
        limit: usize,
    ) -> anyhow::Result<Vec<crate::interaction::InteractionCampaignSummary>> {
        let conn = self.conn()?;
        let mut statement = conn.prepare(
            "SELECT c.id,c.request_id,c.state,c.message_count,c.updated_at,c.error_code,
                    (SELECT COUNT(*) FROM interaction_targets t WHERE t.campaign_id=c.id),
                    (SELECT COUNT(*) FROM interaction_assignments a WHERE a.campaign_id=c.id AND a.state='succeeded'),
                    (SELECT COUNT(*) FROM interaction_assignments a WHERE a.campaign_id=c.id AND a.state IN ('failed','uncertain','skipped_parent')),
                    c.request_json,
                    (SELECT COUNT(*) FROM tiktok_action_runs r WHERE r.owner_kind='interaction' AND r.campaign_id=c.id),
                    (SELECT COUNT(*) FROM tiktok_action_runs r WHERE r.owner_kind='interaction' AND r.campaign_id=c.id AND (r.effect_intent IS NOT NULL OR r.state IN ('armed','confirmed','uncertain'))),
                    (SELECT COUNT(*) FROM tiktok_action_runs r WHERE r.owner_kind='interaction' AND r.campaign_id=c.id AND r.state='confirmed'),
                    (SELECT COUNT(*) FROM tiktok_action_runs r WHERE r.owner_kind='interaction' AND r.campaign_id=c.id AND r.state='no_op'),
                    (SELECT COUNT(*) FROM tiktok_action_runs r WHERE r.owner_kind='interaction' AND r.campaign_id=c.id AND r.state IN ('armed','uncertain'))
             FROM interaction_campaigns c ORDER BY c.updated_at DESC LIMIT ?1",
        )?;
        let rows = statement.query_map(params![limit as i64], interaction_summary_from_row)?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }
    pub fn get_interaction_campaign(
        &self,
        campaign_id: &str,
    ) -> anyhow::Result<Option<crate::interaction::InteractionCampaignDetail>> {
        let conn = self.conn()?;
        let summary = conn
            .query_row(
                "SELECT c.id,c.request_id,c.state,c.message_count,c.updated_at,c.error_code,
                        (SELECT COUNT(*) FROM interaction_targets t WHERE t.campaign_id=c.id),
                        (SELECT COUNT(*) FROM interaction_assignments a WHERE a.campaign_id=c.id AND a.state='succeeded'),
                        (SELECT COUNT(*) FROM interaction_assignments a WHERE a.campaign_id=c.id AND a.state IN ('failed','uncertain','skipped_parent')),
                        c.request_json,
                        (SELECT COUNT(*) FROM tiktok_action_runs r WHERE r.owner_kind='interaction' AND r.campaign_id=c.id),
                        (SELECT COUNT(*) FROM tiktok_action_runs r WHERE r.owner_kind='interaction' AND r.campaign_id=c.id AND (r.effect_intent IS NOT NULL OR r.state IN ('armed','confirmed','uncertain'))),
                        (SELECT COUNT(*) FROM tiktok_action_runs r WHERE r.owner_kind='interaction' AND r.campaign_id=c.id AND r.state='confirmed'),
                        (SELECT COUNT(*) FROM tiktok_action_runs r WHERE r.owner_kind='interaction' AND r.campaign_id=c.id AND r.state='no_op'),
                        (SELECT COUNT(*) FROM tiktok_action_runs r WHERE r.owner_kind='interaction' AND r.campaign_id=c.id AND r.state IN ('armed','uncertain'))
                 FROM interaction_campaigns c WHERE c.id=?1",
                params![campaign_id],
                interaction_summary_from_row,
            )
            .optional()?;
        let Some(mut summary) = summary else {
            return Ok(None);
        };
        let mut statement = conn.prepare(
            // `evidence_json` is here for the retry path: a succeeded root's posted comment
            // identity lives in it, and without reading it back a retry cannot tell a reply
            // what it is replying to.
            "SELECT a.id,t.target_key,a.message_ordinal,a.actor_udid,a.parent_assignment_id,
                    a.state,a.prepared_json,a.error_code,a.evidence_json
             FROM interaction_assignments a
             JOIN interaction_targets t ON t.id=a.target_id
             WHERE a.campaign_id=?1 ORDER BY t.line_no,a.message_ordinal",
        )?;
        let rows = statement.query_map(params![campaign_id], |row| {
            let state: String = row.get(5)?;
            let prepared_json: Option<String> = row.get(6)?;
            let prepared_text = prepared_json
                .as_deref()
                .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok())
                .and_then(|value| {
                    value
                        .get("text")
                        .and_then(|text| text.as_str())
                        .map(str::to_string)
                });
            Ok(crate::interaction::InteractionAssignmentRecord {
                id: row.get(0)?,
                target_key: row.get(1)?,
                ordinal: narrow(row.get::<_, i64>(2)?, "ordinal")?,
                actor_udid: row.get(3)?,
                parent_assignment_id: row.get(4)?,
                state: interaction_message_state(&state),
                prepared_text,
                error_code: row.get(7)?,
                evidence_json: row.get(8)?,
                // Filled from the blob just below, so every reader of a stored campaign gets
                // the same answer without parsing it again.
                like: None,
                mention: None,
                parent_was_folded: false,
                actions: Vec::new(),
            })
        })?;
        let mut assignments: Vec<crate::interaction::InteractionAssignmentRecord> = rows
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .map(|mut assignment| {
                assignment.like = assignment.like_note();
                assignment.mention = assignment.mention_note();
                assignment.parent_was_folded = assignment.parent_was_folded_from_evidence();
                assignment
            })
            .collect();
        for assignment in &mut assignments {
            assignment.actions = self
                .list_interaction_action_runs(&assignment.id)?
                .into_iter()
                .map(|run| crate::interaction::PublicActionResult {
                    kind: run.kind,
                    state: run.state,
                    revision: run.revision,
                    effect_intent: run.effect_intent,
                    evidence: run.evidence,
                    error: run.error,
                })
                .collect();
        }
        let action_results = assignments
            .iter()
            .flat_map(|assignment| assignment.actions.iter().cloned())
            .collect::<Vec<_>>();
        let action_aggregate = (!action_results.is_empty())
            .then(|| crate::interaction::aggregate_interaction_actions(&action_results));
        // The detail already has the exact rows in memory. Re-derive through the canonical
        // helper so the row-level projection and the summary cannot drift.
        summary.action_counters = crate::interaction::count_interaction_actions(&action_results);
        Ok(Some(crate::interaction::InteractionCampaignDetail {
            summary,
            assignments,
            action_aggregate,
        }))
    }
    pub fn get_interaction_campaign_request(
        &self,
        campaign_id: &str,
    ) -> anyhow::Result<
        Option<(
            crate::interaction::ThreadCampaignRequest,
            crate::interaction::ThreadPlan,
        )>,
    > {
        let conn = self.conn()?;
        let raw = conn
            .query_row(
                "SELECT request_json FROM interaction_campaigns WHERE id=?1",
                params![campaign_id],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        let Some(raw) = raw else {
            return Ok(None);
        };
        let request = serde_json::from_str::<crate::interaction::ThreadCampaignRequest>(&raw)?;
        let plan = crate::interaction::plan_threads(&request)
            .map_err(|error| anyhow::anyhow!("persisted interaction plan invalid: {error}"))?;
        Ok(Some((request, plan)))
    }
    pub fn update_interaction_campaign_state(
        &self,
        campaign_id: &str,
        state: crate::interaction::ThreadCampaignState,
        error_code: Option<&str>,
    ) -> anyhow::Result<()> {
        let conn = self.conn()?;
        conn.execute(
            "UPDATE interaction_campaigns SET state=?1,revision=revision+1,error_code=?2,updated_at=?3 WHERE id=?4",
            params![interaction_campaign_state_label(state), error_code, Utc::now().to_rfc3339(), campaign_id],
        )?;
        Ok(())
    }

    /// Settle a live campaign without overwriting an operator cancellation that committed first.
    pub fn settle_interaction_campaign_if_running(
        &self,
        campaign_id: &str,
        state: crate::interaction::ThreadCampaignState,
        error_code: Option<&str>,
    ) -> anyhow::Result<bool> {
        use crate::interaction::ThreadCampaignState;
        anyhow::ensure!(
            matches!(
                state,
                ThreadCampaignState::Succeeded
                    | ThreadCampaignState::Partial
                    | ThreadCampaignState::Failed
            ),
            "interaction campaign settlement requires a terminal non-cancelled state"
        );
        let conn = self.conn()?;
        let changed = conn.execute(
            "UPDATE interaction_campaigns
             SET state=?1,revision=revision+1,error_code=?2,updated_at=?3
             WHERE id=?4 AND state='running'",
            params![
                interaction_campaign_state_label(state),
                error_code,
                Utc::now().to_rfc3339(),
                campaign_id
            ],
        )?;
        Ok(changed > 0)
    }

    /// Cancel a campaign and classify every claimed assignment in the same transaction.
    ///
    /// A cancelled campaign is outside startup orphan recovery. Leaving even one assignment in
    /// `preparing` would therefore preserve ownership for a worker that may have died immediately
    /// after this commit, and the normal claim CAS would correctly refuse every later retry.
    /// Claims with no action effect become retryable failures; any action already armed or
    /// uncertain makes the legacy envelope uncertain too. An immediate transaction also closes
    /// the race with a live worker: a claim that commits first is classified here, while a claim
    /// attempted after this commit is rejected by
    /// `claim_interaction_assignment_for_send`'s campaign-state guard.
    pub fn cancel_interaction_campaign(&self, campaign_id: &str) -> anyhow::Result<usize> {
        let mut conn = self.conn()?;
        let transaction = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let now = Utc::now().to_rfc3339();
        let cancelled = transaction.execute(
            "UPDATE interaction_campaigns \
             SET state='cancelled',revision=revision+1,error_code=NULL,updated_at=?1 \
             WHERE id=?2 AND state IN ('queued','running')",
            params![now, campaign_id],
        )?;
        if cancelled == 0 {
            transaction.commit()?;
            return Ok(0);
        }
        let released = transaction.execute(
            "UPDATE interaction_assignments \
             SET state=CASE WHEN EXISTS ( \
                   SELECT 1 FROM tiktok_action_runs AS action \
                   WHERE action.assignment_id=interaction_assignments.id \
                     AND action.state IN ('armed','uncertain') \
                 ) THEN 'uncertain' ELSE 'failed' END, \
                 error_code=CASE WHEN EXISTS ( \
                   SELECT 1 FROM tiktok_action_runs AS action \
                   WHERE action.assignment_id=interaction_assignments.id \
                     AND action.state IN ('armed','uncertain') \
                 ) THEN ?1 ELSE ?2 END, \
                 effect_intent=NULL,revision=revision+1,updated_at=?3 \
             WHERE campaign_id=?4 AND state='preparing'",
            params![
                "interaction_cancelled_after_action_effect_intent",
                "interaction_cancelled_before_send: operator dừng trước khi tin bắt đầu gửi — có thể thử lại",
                now,
                campaign_id
            ],
        )?;
        transaction.execute(
            "UPDATE tiktok_action_runs
             SET state='failed_before_effect',effect_intent=NULL,
                 error_code=?1,revision=revision+1,updated_at=?2
             WHERE owner_kind='interaction' AND campaign_id=?3 AND state='preparing'",
            params![
                "interaction_cancelled_before_action_effect",
                now,
                campaign_id
            ],
        )?;
        transaction.commit()?;
        Ok(released)
    }
    /// Record a failure reason **without** overwriting a verdict that is already terminal.
    ///
    /// `execute_thread_campaign` reads the campaign's own totals, writes `Partial` when some
    /// messages really did post, and *then* returns `Err` for the first failure it saw. Both
    /// callers used to answer that `Err` with an unconditional `Failed` — which erased exactly
    /// the verdict the totals read was added to produce, on exactly the path it was added for.
    /// The scenario in its own comment ("a campaign the operator could see five posted comments
    /// under, filed as a total loss") therefore still happened, every time any cohort task
    /// returned an error rather than failing per-assignment.
    ///
    /// So the reason is always recorded and the state moves only from a non-terminal one. Done
    /// as one statement rather than read-then-write: two workers can finish at once, and a
    /// read-modify-write between them would reintroduce the same overwrite through the back
    /// door.
    pub fn fail_interaction_campaign_unless_settled(
        &self,
        campaign_id: &str,
        reason: &str,
    ) -> anyhow::Result<()> {
        let conn = self.conn()?;
        conn.execute(
            "UPDATE interaction_campaigns SET              state=CASE WHEN state IN ('queued','running') THEN 'failed' ELSE state END,             revision=revision+1,error_code=?1,updated_at=?2 WHERE id=?3",
            params![reason, Utc::now().to_rfc3339(), campaign_id],
        )?;
        Ok(())
    }

    /// Atomically claim one assignment for a send, moving it to `preparing` **only if no
    /// other worker already holds it**. Returns the new revision as an ownership token.
    ///
    /// Interaction retry starts a worker without any campaign-wide lock, and `run_cohort`
    /// used to stamp the row `preparing` unconditionally — so two rapid retries of the same
    /// manual assignment both proceeded and posted the same deterministic text twice, on a
    /// real account, with no delete path. The claim is a CAS: it moves the row out of an
    /// idle/retryable state (`queued`/`ready`/`failed`/`skipped_parent`) into `preparing`, and
    /// a row already `preparing`/`sending`/`succeeded`/`uncertain` — i.e. one another worker
    /// is on, or one already delivered — is refused. The revision matters even after this
    /// state CAS: a target failure can move A's `preparing` to `failed`, then B can claim it
    /// back to `preparing`; the state looks the same but only B's newer revision owns it.
    pub fn claim_interaction_assignment_for_send(
        &self,
        assignment_id: &str,
    ) -> anyhow::Result<Option<i64>> {
        let conn = self.conn()?;
        conn.query_row(
            "UPDATE interaction_assignments \
             SET state='preparing',revision=revision+1,updated_at=?1 \
             WHERE id=?2 AND state IN ('queued','ready','failed','skipped_parent') \
               AND EXISTS ( \
                   SELECT 1 FROM interaction_campaigns AS campaign \
                   WHERE campaign.id=interaction_assignments.campaign_id \
                     AND campaign.state!='cancelled' \
               ) \
             RETURNING revision",
            params![Utc::now().to_rfc3339(), assignment_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(Into::into)
    }

    /// Cross the public-effect boundary exactly once.
    ///
    /// Preparation holds the row in `preparing`; this CAS is the one-shot callback the driver
    /// invokes after typing and arming, immediately before its Send tap. It writes the intent
    /// in the same SQL statement so startup recovery can conservatively classify a process
    /// lost after this point. A concurrent worker sees zero changed rows and must not tap Send.
    pub fn begin_interaction_assignment_send(
        &self,
        assignment_id: &str,
        ownership_revision: i64,
        effect_intent: &str,
    ) -> anyhow::Result<bool> {
        let conn = self.conn()?;
        let changed = conn.execute(
            "UPDATE interaction_assignments \
             SET state='sending',error_code=NULL,effect_intent=?1,revision=revision+1,updated_at=?2 \
             WHERE id=?3 AND state='preparing' AND revision=?4",
            params![
                effect_intent,
                Utc::now().to_rfc3339(),
                assignment_id,
                ownership_revision
            ],
        )?;
        Ok(changed > 0)
    }

    /// Release this worker's pre-Send claims when the operator cancels the campaign.
    ///
    /// Every row is guarded by both `preparing` and the revision returned from preparation.
    /// That is what keeps a stale cancelling worker from lowering a replacement sibling's
    /// claim after an ABA transition, and keeps a delivery that already crossed into
    /// `sending`/`succeeded`/`uncertain` outside this cleanup. The batch is transactional so a
    /// database failure cannot leave half of one worker's remaining queue stranded.
    pub fn release_owned_interaction_preparations(
        &self,
        claims: &[(String, i64)],
        error_code: &str,
    ) -> anyhow::Result<usize> {
        let mut conn = self.conn()?;
        let transaction = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let now = Utc::now().to_rfc3339();
        let mut released = 0usize;
        for (assignment_id, ownership_revision) in claims {
            released += transaction.execute(
                "UPDATE interaction_assignments \
                 SET state='failed',error_code=?1,effect_intent=NULL,revision=revision+1,updated_at=?2 \
                 WHERE id=?3 AND state='preparing' AND revision=?4",
                params![error_code, now, assignment_id, ownership_revision],
            )?;
        }
        transaction.commit()?;
        Ok(released)
    }

    /// Stamp one assignment `failed` — **but never over a delivery that is settled or in
    /// flight** — and say whether it actually changed.
    ///
    /// The guard is in the SQL, not in a caller's snapshot, and that is the whole point. A
    /// target-wide failure (a shared evidence pass that could not photograph the post) used
    /// to consult a `protected` set captured once when the task started; a sibling that
    /// reached `succeeded` *after* that snapshot was not in it, so its public comment was
    /// stamped `failed` — retryable — and the next retry posted it a second time. `sending`,
    /// `succeeded` and `uncertain` are the three states `retryable_assignments` excludes
    /// because tapping Send is not idempotent, and the `CASE` here refuses to lower any of
    /// them regardless of what any snapshot believed. Returns `true` only when a row was
    /// really moved to `failed`.
    pub fn fail_interaction_assignment_unless_settled(
        &self,
        assignment_id: &str,
        error_code: &str,
    ) -> anyhow::Result<bool> {
        let conn = self.conn()?;
        let changed = conn.execute(
            "UPDATE interaction_assignments \
             SET state='failed',error_code=?1,revision=revision+1,updated_at=?2 \
             WHERE id=?3 AND state NOT IN ('sending','succeeded','uncertain')",
            params![error_code, Utc::now().to_rfc3339(), assignment_id],
        )?;
        Ok(changed > 0)
    }

    /// Target-wide evidence failure may only lower idle assignments.
    ///
    /// Unlike a worker-local failure, it does not own every row it visits. In Standalone mode
    /// a sibling task can already hold one in `preparing`; excluding active claims here keeps
    /// the target sweep from stealing that row while preserving normal startup recovery.
    pub fn fail_interaction_assignment_unless_active_or_settled(
        &self,
        assignment_id: &str,
        error_code: &str,
    ) -> anyhow::Result<bool> {
        let conn = self.conn()?;
        let changed = conn.execute(
            "UPDATE interaction_assignments \
             SET state='failed',error_code=?1,revision=revision+1,updated_at=?2 \
             WHERE id=?3 AND state NOT IN ('preparing','sending','succeeded','uncertain')",
            params![error_code, Utc::now().to_rfc3339(), assignment_id],
        )?;
        Ok(changed > 0)
    }

    /// Close out campaigns whose worker died with the process, and say so.
    ///
    /// An interaction worker is a `tokio::spawn` inside this process: it cannot outlive the
    /// app. Durable orchestration attempts and schedule occurrences restart their exact child,
    /// so this sweep excludes them. Every other campaign left `running` or `queued` at startup
    /// is not running — it is a campaign the app was killed in the middle of, and the Monitor
    /// tab draws it as "Đang chạy" for ever. That state also draws no Retry button (partial /
    /// failed / cancelled only), so the campaign becomes unreachable from the UI by the same
    /// route it became stuck.
    ///
    /// Two different writes, and the difference is the safety-critical part:
    ///
    /// * an assignment left `sending` crossed the write-ahead effect boundary. The process
    ///   died before it could report whether the later Send tap happened, so delivery cannot
    ///   be excluded; it becomes `uncertain`, permanently outside retry. Leaving it `sending`
    ///   would claim an in-flight worker that no longer exists.
    /// * an assignment left `preparing` or legacy `ready` never touched the device, so it is
    ///   released to `failed`, an explicitly retryable state. Leaving `preparing` intact would
    ///   preserve a claim whose worker died, while the claim CAS correctly refuses to steal it.
    ///
    /// The campaign itself becomes `cancelled`: it is the state that says "stopped before it
    /// finished" and it is retryable, so the operator's next action is one button.
    ///
    /// Returns how many campaigns were closed, for the caller to log.
    pub fn interrupt_orphaned_interaction_campaigns(&self) -> anyhow::Result<usize> {
        let mut conn = self.conn()?;
        let transaction = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let now = Utc::now().to_rfc3339();
        let stranded: Vec<String> = transaction
            .prepare(
                "SELECT c.id FROM interaction_campaigns c
                 WHERE c.state IN ('running','queued')
                   AND NOT EXISTS (
                     SELECT 1 FROM orchestration_attempts a
                     WHERE a.child_campaign_id=c.id
                       AND a.state IN ('dispatching','waiting_child')
                   )
                   AND NOT EXISTS (
                     SELECT 1 FROM automation_schedule_occurrences o
                     WHERE o.child_campaign_id=c.id
                       AND o.state IN ('dispatching','running')
                   )",
            )?
            .query_map([], |row| row.get(0))?
            .collect::<Result<_, _>>()?;
        for campaign_id in &stranded {
            transaction.execute(
                "UPDATE tiktok_action_runs
                 SET state='failed_before_effect',revision=revision+1,updated_at=?2,
                     error_code=COALESCE(error_code,'interaction_worker_lost_before_effect')
                 WHERE campaign_id=?1 AND state='preparing'",
                params![campaign_id, now],
            )?;
            transaction.execute(
                "UPDATE tiktok_action_runs
                 SET state='uncertain',revision=revision+1,updated_at=?2,
                     error_code=COALESCE(error_code,'interaction_worker_lost_after_effect_intent')
                 WHERE campaign_id=?1 AND state='armed'",
                params![campaign_id, now],
            )?;
            transaction.execute(
                "UPDATE interaction_assignments
                 SET state='failed',revision=revision+1,updated_at=?2,
                     error_code=COALESCE(error_code,'interaction_worker_lost_before_send: app đóng trước khi tin này bắt đầu gửi — có thể thử lại')
                 WHERE campaign_id=?1 AND state IN ('preparing','ready')",
                params![campaign_id, now],
            )?;
            transaction.execute(
                "UPDATE interaction_assignments
                 SET state='uncertain',revision=revision+1,updated_at=?2,
                     error_code=COALESCE(error_code,'interaction_worker_lost: app đóng khi tin này đang gửi — không xác nhận được là đã đăng hay chưa, nên không gửi lại')
                 WHERE campaign_id=?1 AND state='sending'",
                params![campaign_id, now],
            )?;
            transaction.execute(
                "UPDATE interaction_campaigns
                 SET state='cancelled',revision=revision+1,updated_at=?2,
                     error_code=COALESCE(error_code,'interaction_worker_lost: app đóng khi chiến dịch đang chạy — phần chưa gửi vẫn thử lại được')
                 WHERE id=?1",
                params![campaign_id, now],
            )?;
        }
        transaction.commit()?;
        Ok(stranded.len())
    }

    /// Recover the row-level effect ledger for a campaign whose durable orchestration or
    /// schedule owner will restart it. Manual or detached campaigns are deliberately rejected:
    /// those still belong to [`Self::interrupt_orphaned_interaction_campaigns`].
    pub fn recover_owned_interaction_campaign(&self, campaign_id: &str) -> anyhow::Result<bool> {
        let mut conn = self.conn()?;
        let transaction = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let owned: bool = transaction.query_row(
            "SELECT EXISTS(
               SELECT 1 FROM orchestration_attempts a
               WHERE a.child_campaign_id=?1 AND a.state IN ('dispatching','waiting_child')
               UNION ALL
               SELECT 1 FROM automation_schedule_occurrences o
               WHERE o.child_campaign_id=?1 AND o.state IN ('dispatching','running')
             )",
            params![campaign_id],
            |row| row.get(0),
        )?;
        anyhow::ensure!(
            owned,
            "interaction campaign has no active durable automation owner"
        );
        let running: bool = transaction.query_row(
            "SELECT EXISTS(
               SELECT 1 FROM interaction_campaigns WHERE id=?1 AND state IN ('running','queued')
             )",
            params![campaign_id],
            |row| row.get(0),
        )?;
        if !running {
            transaction.commit()?;
            return Ok(false);
        }
        let now = Utc::now().to_rfc3339();
        transaction.execute(
            "UPDATE tiktok_action_runs
             SET state='failed_before_effect',revision=revision+1,updated_at=?2,
                 error_code=COALESCE(error_code,'interaction_worker_lost_before_effect')
             WHERE campaign_id=?1 AND state='preparing'",
            params![campaign_id, now],
        )?;
        transaction.execute(
            "UPDATE tiktok_action_runs
             SET state='uncertain',revision=revision+1,updated_at=?2,
                 error_code=COALESCE(error_code,'interaction_worker_lost_after_effect_intent')
             WHERE campaign_id=?1 AND state='armed'",
            params![campaign_id, now],
        )?;
        transaction.execute(
            "UPDATE interaction_assignments
             SET state='failed',revision=revision+1,updated_at=?2,
                 error_code=COALESCE(error_code,'interaction_worker_lost_before_send')
             WHERE campaign_id=?1 AND state IN ('preparing','ready')",
            params![campaign_id, now],
        )?;
        transaction.execute(
            "UPDATE interaction_assignments
             SET state='uncertain',revision=revision+1,updated_at=?2,
                 error_code=COALESCE(error_code,'interaction_worker_lost_after_send_intent')
             WHERE campaign_id=?1 AND state='sending'",
            params![campaign_id, now],
        )?;
        transaction.execute(
            "UPDATE interaction_campaigns
             SET state='running',revision=revision+1,updated_at=?2,
                 error_code='automation_worker_resumed'
             WHERE id=?1 AND state IN ('running','queued')",
            params![campaign_id, now],
        )?;
        transaction.commit()?;
        Ok(true)
    }
    /// Persist the prepared text without releasing this worker's claim.
    ///
    /// `ready` remains claimable so databases written by an older build can recover, but the
    /// live runner must not publish that state between preparation and Send: a concurrent retry
    /// would claim the same assignment and both workers could post it. State alone is not
    /// ownership because it can go `preparing -> failed -> preparing` (ABA), so the claim's
    /// revision must match too. A winner gets the next revision for the effect CAS; the row
    /// stays `preparing` until that CAS.
    pub fn prepare_interaction_assignment(
        &self,
        assignment_id: &str,
        ownership_revision: i64,
        prepared: &crate::interaction::PreparedThreadMessage,
    ) -> anyhow::Result<Option<i64>> {
        let conn = self.conn()?;
        conn.query_row(
            "UPDATE interaction_assignments
             SET prepared_json=?1,revision=revision+1,updated_at=?2
             WHERE id=?3 AND state='preparing' AND revision=?4
             RETURNING revision",
            params![
                serde_json::to_string(prepared)?,
                Utc::now().to_rfc3339(),
                assignment_id,
                ownership_revision
            ],
            |row| row.get(0),
        )
        .optional()
        .map_err(Into::into)
    }
    /// Read back what the web lookup filed against each target of one campaign.
    ///
    /// Ordered by `line_no`, so the panel lists targets in the order the operator pasted them.
    ///
    /// **Every column named, and none read by position.** Inserting a column into the middle of
    /// a `SELECT *` shifts every one after it silently, and the two `TEXT` columns here
    /// (`normalized_url`, `context_json`) would swap without any type error to catch it.
    pub fn list_interaction_target_notes(
        &self,
        campaign_id: &str,
    ) -> anyhow::Result<Vec<crate::interaction::InteractionTargetNote>> {
        let conn = self.conn()?;
        let mut statement = conn.prepare(
            "SELECT target_key,line_no,normalized_url,kind,context_json
             FROM interaction_targets WHERE campaign_id=?1 ORDER BY line_no",
        )?;
        let rows = statement.query_map(params![campaign_id], |row| {
            let target_key: String = row.get(0)?;
            let line_no: i64 = row.get(1)?;
            let normalized_url: String = row.get(2)?;
            let kind: String = row.get(3)?;
            let context_json: Option<String> = row.get(4)?;
            Ok(crate::interaction::InteractionTargetNote::from_row(
                target_key,
                line_no.max(0) as u32,
                normalized_url,
                match kind.as_str() {
                    "photo" => crate::interaction::TikTokPostKind::Photo,
                    _ => crate::interaction::TikTokPostKind::Video,
                },
                context_json.as_deref(),
            ))
        })?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    /// File what the desktop learned about one target from outside the phones.
    ///
    /// **Written to `context_json`, a column that has been in the schema and unused since it
    /// was created** — so this needs no migration, and it lands in the one place an operator
    /// auditing a campaign would already be looking.
    ///
    /// Best-effort by contract, like everything on this path: a campaign that could not file
    /// its note still has to send its comments. The caller logs and carries on.
    pub fn record_interaction_target_context(
        &self,
        campaign_id: &str,
        target_key: &str,
        context_json: &str,
    ) -> anyhow::Result<()> {
        let conn = self.conn()?;
        conn.execute(
            "UPDATE interaction_targets SET context_json=?1 WHERE campaign_id=?2 AND target_key=?3",
            params![context_json, campaign_id, target_key],
        )?;
        Ok(())
    }

    pub fn update_interaction_assignment_state(
        &self,
        assignment_id: &str,
        state: crate::interaction::ThreadMessageState,
        error_code: Option<&str>,
        effect_intent: Option<&str>,
        evidence_json: Option<&str>,
    ) -> anyhow::Result<()> {
        let conn = self.conn()?;
        conn.execute(
            "UPDATE interaction_assignments SET state=?1,error_code=?2,effect_intent=COALESCE(?3,effect_intent),evidence_json=COALESCE(?4,evidence_json),revision=revision+1,updated_at=?5 WHERE id=?6",
            params![
                interaction_message_state_label(state),
                error_code,
                effect_intent,
                evidence_json,
                Utc::now().to_rfc3339(),
                assignment_id,
            ],
        )?;
        Ok(())
    }

    /// Settle only the assignment this worker still owns.
    ///
    /// Before a public effect, the preparation revision is the ownership token and a success
    /// additionally requires the campaign to remain running. After Comment crosses Send, the
    /// non-retryable `sending` state is itself exclusive and may still be confirmed after Cancel.
    pub fn settle_owned_interaction_assignment(
        &self,
        assignment_id: &str,
        ownership_revision: i64,
        state: crate::interaction::ThreadMessageState,
        error_code: Option<&str>,
        evidence_json: Option<&str>,
    ) -> anyhow::Result<bool> {
        use crate::interaction::ThreadMessageState;
        anyhow::ensure!(
            matches!(
                state,
                ThreadMessageState::Succeeded
                    | ThreadMessageState::Failed
                    | ThreadMessageState::Uncertain
                    | ThreadMessageState::SkippedParent
            ),
            "interaction assignment settlement requires a terminal state"
        );
        let conn = self.conn()?;
        let state = interaction_message_state_label(state);
        let changed = conn.execute(
            "UPDATE interaction_assignments
             SET state=?1,error_code=?2,evidence_json=COALESCE(?3,evidence_json),
                 revision=revision+1,updated_at=?4
             WHERE id=?5 AND (
               (state='sending' AND ?1 IN ('succeeded','uncertain')) OR
               (state='preparing' AND revision=?6 AND (
                 ?1 IN ('failed','uncertain') OR EXISTS (
                   SELECT 1 FROM interaction_campaigns AS campaign
                   WHERE campaign.id=interaction_assignments.campaign_id
                     AND campaign.state='running'
                 )
               ))
             )",
            params![
                state,
                error_code,
                evidence_json,
                Utc::now().to_rfc3339(),
                assignment_id,
                ownership_revision,
            ],
        )?;
        Ok(changed > 0)
    }
    /// Saved frames for a campaign, newest first. Rows without a
    /// `relative_path` predate evidence storage and have no file behind them.
    pub fn list_interaction_artifacts(
        &self,
        campaign_id: &str,
    ) -> anyhow::Result<Vec<crate::interaction::InteractionArtifactRecord>> {
        let conn = self.conn()?;
        let mut statement = conn.prepare(
            "SELECT id,assignment_id,kind,relative_path,sha256,created_at
             FROM interaction_artifacts WHERE campaign_id=?1 ORDER BY created_at DESC",
        )?;
        let rows = statement.query_map(params![campaign_id], |row| {
            Ok(crate::interaction::InteractionArtifactRecord {
                id: row.get(0)?,
                assignment_id: row.get(1)?,
                kind: row.get(2)?,
                relative_path: row.get(3)?,
                sha256: row.get(4)?,
                created_at: row.get(5)?,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }
    /// One saved frame by id, for reading its bytes back.
    pub fn get_interaction_artifact(
        &self,
        artifact_id: &str,
    ) -> anyhow::Result<Option<crate::interaction::InteractionArtifactRecord>> {
        let conn = self.conn()?;
        Ok(conn
            .query_row(
                "SELECT id,assignment_id,kind,relative_path,sha256,created_at
                 FROM interaction_artifacts WHERE id=?1",
                params![artifact_id],
                |row| {
                    Ok(crate::interaction::InteractionArtifactRecord {
                        id: row.get(0)?,
                        assignment_id: row.get(1)?,
                        kind: row.get(2)?,
                        relative_path: row.get(3)?,
                        sha256: row.get(4)?,
                        created_at: row.get(5)?,
                    })
                },
            )
            .optional()?)
    }
    #[allow(clippy::too_many_arguments)]
    pub fn add_interaction_artifact(
        &self,
        campaign_id: &str,
        target_key: &str,
        assignment_id: Option<&str>,
        kind: &str,
        metadata_json: &str,
        sha256: &str,
        relative_path: Option<&str>,
    ) -> anyhow::Result<String> {
        let conn = self.conn()?;
        let target_id: String = conn.query_row(
            "SELECT id FROM interaction_targets WHERE campaign_id=?1 AND target_key=?2",
            params![campaign_id, target_key],
            |row| row.get(0),
        )?;
        let id = Uuid::new_v4().to_string();
        conn.execute(
            "INSERT INTO interaction_artifacts
             (id,campaign_id,target_id,assignment_id,kind,metadata_json,relative_path,sha256,created_at)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
            params![
                id,
                campaign_id,
                target_id,
                assignment_id,
                kind,
                metadata_json,
                relative_path,
                sha256,
                Utc::now().to_rfc3339(),
            ],
        )?;
        Ok(id)
    }
}
