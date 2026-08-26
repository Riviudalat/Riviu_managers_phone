//! Interaction campaigns: the plan, each assignment's state, and the evidence filed
//! against them.

use super::*;

impl Database {
    pub fn create_interaction_campaign(
        &self,
        request: &crate::interaction::ThreadCampaignRequest,
        plan: &crate::interaction::ThreadPlan,
    ) -> anyhow::Result<String> {
        request.validate().map_err(|error| anyhow::anyhow!(error))?;
        let mut conn = self.conn()?;
        let transaction = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let campaign_id = Uuid::new_v4().to_string();
        let now = Utc::now().to_rfc3339();
        transaction.execute(
            "INSERT INTO interaction_campaigns
             (id,request_id,request_json,state,message_count,revision,created_at,updated_at)
             VALUES (?1,?2,?3,'queued',?4,0,?5,?5)",
            params![
                campaign_id,
                request.request_id,
                serde_json::to_string(request)?,
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
        Ok(campaign_id)
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
                    c.request_json
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
                        c.request_json
                 FROM interaction_campaigns c WHERE c.id=?1",
                params![campaign_id],
                interaction_summary_from_row,
            )
            .optional()?;
        let Some(summary) = summary else {
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
            })
        })?;
        let assignments = rows
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .map(|mut assignment| {
                assignment.like = assignment.like_note();
                assignment.mention = assignment.mention_note();
                assignment
            })
            .collect();
        Ok(Some(crate::interaction::InteractionCampaignDetail {
            summary,
            assignments,
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

    /// Close out campaigns whose worker died with the process, and say so.
    ///
    /// An interaction worker is a `tokio::spawn` inside this process: it cannot outlive the
    /// app, and nothing restarts it. So a campaign left `running` or `queued` at startup is
    /// not running — it is a campaign the app was killed in the middle of, and the Monitor
    /// tab draws it as "Đang chạy" for ever. That state also draws no Retry button (partial /
    /// failed / cancelled only), so the campaign becomes unreachable from the UI by the same
    /// route it became stuck.
    ///
    /// Two different writes, and the difference is the safety-critical part:
    ///
    /// * an assignment left `sending` had its Send tap go out with no confirmation coming
    ///   back, which is exactly what `uncertain` means — and `uncertain` is permanently
    ///   excluded from retry, so the comment can never be posted twice. Leaving it `sending`
    ///   would claim an in-flight message that no longer exists.
    /// * an assignment left `preparing` or `ready` never touched the device, so it stays
    ///   retryable and is not rewritten at all.
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
            .prepare("SELECT id FROM interaction_campaigns WHERE state IN ('running','queued')")?
            .query_map([], |row| row.get(0))?
            .collect::<Result<_, _>>()?;
        for campaign_id in &stranded {
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
    pub fn prepare_interaction_assignment(
        &self,
        assignment_id: &str,
        prepared: &crate::interaction::PreparedThreadMessage,
    ) -> anyhow::Result<()> {
        let conn = self.conn()?;
        conn.execute(
            "UPDATE interaction_assignments
             SET prepared_json=?1,state='ready',revision=revision+1,updated_at=?2
             WHERE id=?3 AND effect_intent IS NULL",
            params![
                serde_json::to_string(prepared)?,
                Utc::now().to_rfc3339(),
                assignment_id
            ],
        )?;
        Ok(())
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
