//! A campaign owns one execution token; every worker write also checks the child revision.
use super::publish_sheet::queue_sheet_row;
use super::*;

#[derive(Debug, Clone)]
pub struct PublishPipelineRun {
    pub campaign_id: String,
    pub token: String,
}

impl Database {
    pub fn claim_publish_pipeline(
        &self,
        campaign_id: &str,
    ) -> anyhow::Result<Option<PublishPipelineRun>> {
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let now = Utc::now().to_rfc3339();
        let token = Uuid::new_v4().to_string();
        let changed=tx.execute("UPDATE publish_campaigns SET state='posting',error_code=NULL,revision=revision+1,updated_at=?2
            WHERE id=?1 AND state IN ('queued','scheduled','ready','imported','failed_before_dispatch','verifying')
            AND (run_at IS NULL OR datetime(run_at)<=datetime(?3))
            AND NOT EXISTS(SELECT 1 FROM publish_pipeline_runs r WHERE r.campaign_id=?1)
            AND NOT EXISTS(SELECT 1 FROM publish_assignments a WHERE a.campaign_id=?1 AND a.state IN ('posting','uncertain'))
            AND EXISTS(SELECT 1 FROM publish_assignments a WHERE a.campaign_id=?1 AND a.effect_intent IS NULL AND a.state IN ('queued','scheduled','ready','imported','failed_before_dispatch'))",
            params![campaign_id,now,chrono::Local::now().naive_local().format("%Y-%m-%dT%H:%M:%S").to_string()])?;
        if changed == 0 {
            return Ok(None);
        }
        tx.execute(
            "INSERT INTO publish_pipeline_runs(campaign_id,token,created_at) VALUES(?1,?2,?3)",
            params![campaign_id, token, now],
        )?;
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
        } else if !rows.is_empty() && rows.iter().all(|s| s == "succeeded") {
            "succeeded"
        } else {
            "failed_before_dispatch"
        };
        let error = match state {
            "succeeded" => None,
            "verifying" => Some("post_verification_pending"),
            "uncertain" => Some("post_or_cleanup_failed"),
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
mod tests;
