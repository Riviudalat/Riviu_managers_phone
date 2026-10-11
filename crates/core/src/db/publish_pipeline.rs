//! A campaign owns one execution token; every worker write also checks the child revision.
use super::publish_sheet::queue_sheet_row;
use super::*;

#[derive(Debug, Clone)]
pub struct PublishPipelineRun {
    pub campaign_id: String,
    pub token: String,
}

impl Database {
    /// The approval gate in front of every fresh claim (PLAN §3): the schedule timer and the
    /// manual, scheduled and orchestration execution paths all call it before
    /// [`Self::claim_publish_pipeline`], and dispatcher admission repeats the same per-phone
    /// check atomically. It reads the frozen approval and the current account of every phone
    /// that has not crossed the effect boundary, and refuses the whole receiver set when any
    /// one is stale: a smaller, silently re-chosen set is never dispatched. Submitted and
    /// uncertain rows are not fresh work and are not looked at here.
    pub fn publish_approval_refusal(&self, campaign_id: &str) -> anyhow::Result<Option<String>> {
        let conn = self.conn()?;
        let Some(raw) = conn
            .query_row(
                "SELECT request_json FROM publish_campaigns WHERE id=?1",
                [campaign_id],
                |row| row.get::<_, String>(0),
            )
            .optional()?
        else {
            return Ok(None);
        };
        let request: crate::PublishCampaignRequest = serde_json::from_str(&raw)?;
        let mut statement = conn.prepare(
            "SELECT a.udid,COALESCE((SELECT handle FROM device_meta WHERE udid=a.udid),'')
             FROM publish_assignments a WHERE a.campaign_id=?1 AND a.effect_intent IS NULL
             AND a.state IN ('queued','scheduled','ready','imported','failed_before_dispatch')
             ORDER BY a.ordinal",
        )?;
        let fresh = statement
            .query_map([campaign_id], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        if fresh.is_empty() {
            return Ok(None);
        }
        if request.approval.is_none() {
            return Ok(crate::publish::publish_approval_refusal(&request, "", ""));
        }
        let refusals: Vec<String> = fresh
            .iter()
            .filter_map(|(udid, handle)| {
                crate::publish::publish_approval_refusal(&request, udid, handle)
            })
            .collect();
        Ok((!refusals.is_empty()).then(|| refusals.join("; ")))
    }

    /// Settle a refusal from [`Self::publish_approval_refusal`] before any device work: the
    /// fresh rows and the campaign become `failed_before_dispatch` with the reason kept in
    /// `error_code`. A later retry is checked against the same frozen approval and refused
    /// again; only a newly reviewed campaign can carry changed inputs. Rows past the effect
    /// boundary are not touched.
    pub fn refuse_publish_for_approval(
        &self,
        campaign_id: &str,
        reason: &str,
    ) -> anyhow::Result<()> {
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let now = Utc::now().to_rfc3339();
        let code = format!("publish_approval_changed: {reason}");
        tx.execute(
            "UPDATE publish_assignments SET state='failed_before_dispatch',error_code=?2,revision=revision+1,updated_at=?3
             WHERE campaign_id=?1 AND effect_intent IS NULL
             AND state IN ('queued','scheduled','ready','imported','failed_before_dispatch')",
            params![campaign_id, code, now],
        )?;
        tx.execute(
            "UPDATE publish_campaigns SET state='failed_before_dispatch',error_code=?2,revision=revision+1,updated_at=?3
             WHERE id=?1 AND state IN ('queued','scheduled','ready','imported','failed_before_dispatch')",
            params![campaign_id, code, now],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Effect admission (PLAN §3, review R2-F2). Called with the device lease held and after
    /// every capacity wait, immediately before the first transfer, app-session or composer
    /// effect. One short transaction compares the frozen approval with the current account of
    /// every phone in this campaign that has not crossed the effect boundary; nothing is held
    /// across device I/O. A passing admission stays valid for the running step because
    /// account edits on a phone with a running publish step are refused
    /// (`ensure_account_edit_allowed`). On any change the whole remaining pre-dispatch set is
    /// settled review-required and the caller gets a terminal refusal, so the old approval is
    /// never retried. A row already past the effect boundary only reconciles and is admitted.
    pub fn admit_publish_effect(
        &self,
        assignment_id: &str,
    ) -> anyhow::Result<Result<(), crate::publish_recovery::RecoveryFailure>> {
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (campaign_id, raw, intent): (String, String, Option<String>) = tx.query_row(
            "SELECT a.campaign_id,c.request_json,a.effect_intent FROM publish_assignments a
             JOIN publish_campaigns c ON c.id=a.campaign_id WHERE a.id=?1",
            [assignment_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )?;
        if intent.is_some() {
            return Ok(Ok(()));
        }
        let request: crate::PublishCampaignRequest = serde_json::from_str(&raw)?;
        let remaining = {
            let mut statement = tx.prepare(
                "SELECT a.udid,COALESCE((SELECT handle FROM device_meta WHERE udid=a.udid),'')
                 FROM publish_assignments a WHERE a.campaign_id=?1 AND a.effect_intent IS NULL
                 AND a.state NOT IN ('cancelled','missed','succeeded','uncertain','verifying')
                 ORDER BY a.ordinal",
            )?;
            let rows = statement
                .query_map([&campaign_id], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })?
                .collect::<Result<Vec<_>, _>>()?;
            rows
        };
        let refusals: Vec<String> = if request.approval.is_none() {
            crate::publish::publish_approval_refusal(&request, "", "")
                .into_iter()
                .collect()
        } else {
            remaining
                .iter()
                .filter_map(|(udid, handle)| {
                    crate::publish::publish_approval_refusal(&request, udid, handle)
                })
                .collect()
        };
        if refusals.is_empty() {
            tx.commit()?;
            return Ok(Ok(()));
        }
        let message = format!("publish_approval_changed: {}", refusals.join("; "));
        let now = Utc::now();
        // Siblings waiting for their own step stop here too: queued jobs finish, recovery
        // records fail, rows become failed_before_dispatch. This assignment is settled by the
        // caller's terminal failure; rows past the effect boundary are untouched.
        tx.execute(
            "UPDATE publish_dispatch_jobs SET state='finished',reason='publish_approval_changed',
             finished_at_ms=?2,revision=revision+1 WHERE state='queued' AND assignment_id<>?3 AND assignment_id IN
             (SELECT id FROM publish_assignments WHERE campaign_id=?1 AND effect_intent IS NULL)",
            params![campaign_id, now.timestamp_millis(), assignment_id],
        )?;
        tx.execute(
            "UPDATE publish_recovery_state SET payload=json_set(payload,'$.state','failed','$.lastError',?2,
             '$.nextRetryAt',NULL,'$.reconnectDeadline',NULL) WHERE assignment_id<>?3 AND assignment_id IN
             (SELECT id FROM publish_assignments WHERE campaign_id=?1 AND effect_intent IS NULL
              AND state IN ('queued','scheduled','ready','imported','failed_before_dispatch'))",
            params![campaign_id, message, assignment_id],
        )?;
        tx.execute(
            "UPDATE publish_assignments SET state='failed_before_dispatch',error_code=?2,revision=revision+1,updated_at=?3
             WHERE campaign_id=?1 AND id<>?4 AND effect_intent IS NULL
             AND state IN ('queued','scheduled','ready','imported','failed_before_dispatch')",
            params![campaign_id, message, now.to_rfc3339(), assignment_id],
        )?;
        tx.commit()?;
        Ok(Err(crate::publish_recovery::RecoveryFailure::new(
            "publish_approval_changed",
            crate::publish_recovery::FailureKind::Terminal,
            message,
        )))
    }

    pub fn claim_publish_pipeline(
        &self,
        campaign_id: &str,
    ) -> anyhow::Result<Option<PublishPipelineRun>> {
        self.ensure_publish_recovery_ready()?;
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let request: Option<String> = tx
            .query_row(
                "SELECT request_json FROM publish_campaigns WHERE id=?1",
                [campaign_id],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(request) = request {
            serde_json::from_str::<crate::PublishCampaignRequest>(&request)?
                .network
                .ensure_implemented()?;
        }
        let now = Utc::now().to_rfc3339();
        let token = Uuid::new_v4().to_string();
        let admitted_at = chrono::Local::now().naive_local();
        let changed=tx.execute("UPDATE publish_campaigns SET state='posting',error_code=NULL,revision=revision+1,updated_at=?2
            WHERE id=?1 AND state IN ('queued','scheduled','ready','imported','failed_before_dispatch','verifying')
            AND NOT EXISTS(SELECT 1 FROM publish_start_requests s WHERE s.campaign_id=?1 AND s.state='uncertain')
            AND (run_at IS NULL OR datetime(run_at)<=datetime(?3))
            AND (run_at IS NULL OR datetime(?3)<=datetime(run_at,'+30 seconds')
                OR EXISTS(SELECT 1 FROM publish_dispatch_jobs j WHERE j.campaign_id=?1 AND j.started_at_ms IS NOT NULL))
            AND NOT EXISTS(SELECT 1 FROM publish_pipeline_runs r WHERE r.campaign_id=?1)
            AND NOT EXISTS(SELECT 1 FROM publish_assignments a WHERE a.campaign_id=?1 AND a.state IN ('posting','uncertain'))
            AND EXISTS(SELECT 1 FROM publish_assignments a WHERE a.campaign_id=?1 AND a.effect_intent IS NULL AND a.state IN ('queued','scheduled','ready','imported','failed_before_dispatch'))",
            params![campaign_id,now,admitted_at.format("%Y-%m-%dT%H:%M:%S").to_string()])?;
        if changed == 0 {
            return Ok(None);
        }
        tx.execute(
            "INSERT INTO publish_pipeline_runs(campaign_id,token,created_at) VALUES(?1,?2,?3)",
            params![campaign_id, token, now],
        )?;
        Self::enqueue_publish_jobs(
            &tx,
            &PublishPipelineRun {
                campaign_id: campaign_id.into(),
                token: token.clone(),
            },
            Utc::now().timestamp_millis(),
            admitted_at,
        )?;
        tx.execute("UPDATE publish_start_requests SET state='queued',stage='queued',revision=revision+1,updated_at=?2
            WHERE campaign_id=?1 AND state='preparing'",params![campaign_id,now])?;
        pipeline_event(&tx, campaign_id, &now)?;
        tx.commit()?;
        Ok(Some(PublishPipelineRun {
            campaign_id: campaign_id.into(),
            token,
        }))
    }

    pub fn has_active_publish_pipeline(&self, campaign_id: &str) -> anyhow::Result<bool> {
        self.conn()?
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM publish_pipeline_runs WHERE campaign_id=?1)",
                [campaign_id],
                |r| r.get(0),
            )
            .map_err(Into::into)
    }

    pub fn publish_pipeline_current(&self, run: &PublishPipelineRun) -> anyhow::Result<bool> {
        self.conn()?.query_row("SELECT EXISTS(SELECT 1 FROM publish_pipeline_runs r JOIN publish_campaigns c ON c.id=r.campaign_id WHERE r.campaign_id=?1 AND r.token=?2 AND c.state='posting')",params![run.campaign_id,run.token],|r|r.get(0)).map_err(Into::into)
    }

    pub fn publish_assignment_revision(&self, assignment: &str) -> anyhow::Result<i64> {
        self.conn()?
            .query_row(
                "SELECT revision FROM publish_assignments WHERE id=?1",
                [assignment],
                |r| r.get(0),
            )
            .map_err(Into::into)
    }

    /// The expected previous state and null intent prevent a transfer from reopening a Post.
    #[allow(clippy::too_many_arguments)]
    pub fn advance_publish_pipeline_assignment(
        &self,
        run: &PublishPipelineRun,
        assignment: &str,
        expected_revision: i64,
        expected_state: crate::PublishCampaignState,
        next_state: crate::PublishCampaignState,
        error: Option<&str>,
        evidence: Option<&str>,
    ) -> anyhow::Result<bool> {
        anyhow::ensure!(
            matches!(
                next_state,
                crate::PublishCampaignState::Transferring
                    | crate::PublishCampaignState::Imported
                    | crate::PublishCampaignState::FailedBeforeDispatch
            ),
            "invalid pre-Post pipeline transition"
        );
        use crate::PublishCampaignState as S;
        anyhow::ensure!(
            matches!(
                (&expected_state, &next_state),
                (
                    S::Queued | S::Scheduled | S::Ready | S::FailedBeforeDispatch,
                    S::Transferring | S::FailedBeforeDispatch
                ) | (S::Transferring, S::Imported | S::FailedBeforeDispatch)
                    | (S::Imported, S::Imported | S::FailedBeforeDispatch)
            ),
            "invalid pipeline state edge"
        );
        let changed=self.conn()?.execute("UPDATE publish_assignments SET state=?1,error_code=?2,evidence_json=COALESCE(?3,evidence_json),revision=revision+1,updated_at=?4
            WHERE id=?5 AND campaign_id=?6 AND revision=?7 AND state=?8 AND effect_intent IS NULL
            AND EXISTS(SELECT 1 FROM publish_pipeline_runs r JOIN publish_campaigns c ON c.id=r.campaign_id WHERE r.campaign_id=?6 AND r.token=?9 AND c.state='posting')",
            params![next_state.as_str(),error,evidence,Utc::now().to_rfc3339(),assignment,run.campaign_id,expected_revision,expected_state.as_str(),run.token])?;
        Ok(changed == 1)
    }

    pub fn claim_pipeline_post(
        &self,
        run: &PublishPipelineRun,
        assignment: &str,
        revision: i64,
        intent: &str,
    ) -> anyhow::Result<bool> {
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        // Read under the write lock: see `publish_shutdown` for why this is the exit barrier.
        if self.publish_shutdown_started() {
            return Ok(false);
        }
        let changed=tx.execute("UPDATE publish_assignments SET state='posting',error_code=NULL,effect_intent=?1,evidence_json=?1,revision=revision+1,updated_at=?2
            WHERE id=?3 AND campaign_id=?4 AND revision=?5 AND state='imported' AND effect_intent IS NULL
            AND EXISTS(SELECT 1 FROM publish_pipeline_runs r JOIN publish_campaigns c ON c.id=r.campaign_id WHERE r.campaign_id=?4 AND r.token=?6 AND c.state='posting')",
            params![intent,Utc::now().to_rfc3339(),assignment,run.campaign_id,revision,run.token])?;
        if changed == 1 {
            super::publish_submission::validate_submission_claim(&tx, assignment, intent)?;
        }
        tx.commit()?;
        Ok(changed == 1)
    }

    /// Cancellation blocks a new Post, but cannot discard the result of an already dispatched one.
    #[allow(clippy::too_many_arguments)]
    pub fn settle_pipeline_post(
        &self,
        run: &PublishPipelineRun,
        assignment: &str,
        revision: i64,
        state: crate::PublishCampaignState,
        error: Option<&str>,
        evidence: &str,
        link: Option<&str>,
        poster: &str,
        partners: &[String],
    ) -> anyhow::Result<bool> {
        use crate::PublishCampaignState as S;
        anyhow::ensure!(
            matches!(
                state,
                S::Succeeded | S::Verifying | S::Uncertain | S::FailedBeforeDispatch
            ),
            "invalid post outcome state"
        );
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let changed=tx.execute("UPDATE publish_assignments SET state=?1,error_code=?2,evidence_json=?3,revision=revision+1,updated_at=?4 WHERE id=?5 AND campaign_id=?6 AND revision=?7 AND state IN ('imported','posting')
            AND ((state='imported' AND ?1='failed_before_dispatch' AND effect_intent IS NULL) OR (state='posting' AND effect_intent IS NOT NULL AND ?1 IN ('succeeded','verifying','uncertain')))
            AND EXISTS(SELECT 1 FROM publish_pipeline_runs r WHERE r.campaign_id=?6 AND r.token=?8)",params![state.as_str(),error,evidence,Utc::now().to_rfc3339(),assignment,run.campaign_id,revision,run.token])?;
        if changed == 0 {
            return Ok(false);
        }
        if let Some(url) = link {
            anyhow::ensure!(
                state == crate::PublishCampaignState::Succeeded,
                "only verified publication queues Sheet"
            );
            queue_sheet_row(&tx, assignment, &run.campaign_id, url, poster, partners)?;
        }
        tx.commit()?;
        Ok(true)
    }

    /// Only the coordinator removes its token, after joining every worker (including panics).
    pub fn finish_publish_pipeline(&self, run: &PublishPipelineRun) -> anyhow::Result<bool> {
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let changed = tx.execute(
            "DELETE FROM publish_pipeline_runs WHERE campaign_id=?1 AND token=?2",
            params![run.campaign_id, run.token],
        )?;
        if changed == 0 {
            return Ok(false);
        }
        let now = Utc::now().to_rfc3339();
        tx.execute("UPDATE publish_assignments SET state=CASE WHEN effect_intent IS NULL THEN 'failed_before_dispatch' ELSE 'uncertain' END,error_code='publish_worker_lost',revision=revision+1,updated_at=?2 WHERE campaign_id=?1 AND state IN ('transferring','posting')",params![run.campaign_id,now])?;
        let rows: Vec<String> = tx
            .prepare("SELECT state FROM publish_assignments WHERE campaign_id=?1")?
            .query_map([&run.campaign_id], |r| r.get(0))?
            .collect::<Result<_, _>>()?;
        let state = if rows.iter().any(|s| s == "uncertain") {
            "uncertain"
        } else if rows.iter().any(|s| s == "verifying") {
            "verifying"
        } else if !rows.is_empty() && rows.iter().all(|s| s == "missed") {
            "missed"
        } else if !rows.is_empty() && rows.iter().all(|s| s == "succeeded") {
            "succeeded"
        } else {
            "failed_before_dispatch"
        };
        let error = match state {
            "succeeded" => None,
            "verifying" => Some("post_verification_pending"),
            "uncertain" => Some("post_or_cleanup_failed"),
            "missed" => Some("schedule_capacity_deadline"),
            _ => Some("publish_assignment_failed"),
        };
        if tx.execute("UPDATE publish_campaigns SET state=?1,error_code=?2,revision=revision+1,updated_at=?3 WHERE id=?4 AND state='posting'",params![state,error,now,run.campaign_id])?==1{pipeline_event(&tx,&run.campaign_id,&now)?;}
        tx.commit()?;
        Ok(true)
    }
}
fn pipeline_event(tx: &rusqlite::Transaction<'_>, campaign: &str, now: &str) -> anyhow::Result<()> {
    tx.execute("INSERT INTO publish_events(campaign_id,revision,kind,payload_json,created_at) SELECT id,revision,'state',json_object('state',state,'errorCode',error_code,'source','per_device_pipeline'),?2 FROM publish_campaigns WHERE id=?1",params![campaign,now])?;
    Ok(())
}

#[cfg(test)]
#[path = "publish_pipeline_tests.rs"]
pub(super) mod tests;
