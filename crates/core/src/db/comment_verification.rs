use super::*;
use crate::comment_verification::{
    CommentVerification, VerificationContext, VerificationJob, VerificationState,
};

#[cfg(test)]
#[path = "comment_verification_tests.rs"]
mod tests;

fn view(row: &rusqlite::Row<'_>) -> rusqlite::Result<CommentVerification> {
    let state: String = row.get(0)?;
    Ok(CommentVerification {
        state: match state.as_str() {
            "verified" => VerificationState::Verified,
            "pending" => VerificationState::Pending,
            _ => VerificationState::NeedsReview,
        },
        attempts: row.get(1)?,
        next_check_at_ms: row.get(2)?,
        deadline_ms: row.get(3)?,
        reason: row.get(4)?,
        evidence: row.get(5)?,
    })
}

impl Database {
    pub fn defer_parent_recheck(&self, assignment: &str, now: i64) -> anyhow::Result<bool> {
        let conn = self.conn()?;
        let n=conn.execute("INSERT INTO interaction_parent_rechecks(assignment_id,attempts,first_at_ms,next_at_ms) VALUES(?1,1,?2,?2+15000)
         ON CONFLICT(assignment_id) DO UPDATE SET attempts=attempts+1,next_at_ms=first_at_ms+45000 WHERE attempts<2",params![assignment,now])?;
        Ok(n == 1)
    }
    pub fn parent_recheck_due(&self, assignment: &str, now: i64) -> anyhow::Result<bool> {
        let next: Option<i64> = self
            .conn()?
            .query_row(
                "SELECT next_at_ms FROM interaction_parent_rechecks WHERE assignment_id=?1",
                [assignment],
                |r| r.get(0),
            )
            .optional()?;
        Ok(next.is_none_or(|next| now >= next))
    }
    /// Save immutable observation inputs before the Send transaction. It is dormant until armed.
    pub fn prepare_comment_verification(
        &self,
        assignment: &str,
        context: &VerificationContext,
    ) -> anyhow::Result<()> {
        let conn = self.conn()?;
        conn.execute("INSERT INTO interaction_comment_verification(assignment_id,campaign_id,device_id,context_json,state)
            SELECT id,campaign_id,actor_udid,?2,'pending' FROM interaction_assignments WHERE id=?1 AND state='preparing'
            ON CONFLICT(assignment_id) DO UPDATE SET context_json=excluded.context_json
            WHERE interaction_comment_verification.sent_at_ms IS NULL",params![assignment,serde_json::to_string(context)?])?;
        Ok(())
    }
    pub fn comment_verification(
        &self,
        assignment: &str,
    ) -> anyhow::Result<Option<CommentVerification>> {
        Ok(self.conn()?.query_row("SELECT state,attempts,next_at_ms,deadline_ms,reason,evidence_json FROM interaction_comment_verification WHERE assignment_id=?1 AND sent_at_ms IS NOT NULL",[assignment],view).optional()?)
    }
    pub fn comment_verification_pending(&self, assignment: &str) -> anyhow::Result<bool> {
        Ok(self
            .comment_verification(assignment)?
            .is_some_and(|v| v.state == VerificationState::Pending))
    }
    pub fn recover_comment_verifications(&self) -> anyhow::Result<Vec<String>> {
        let now = Utc::now().timestamp_millis();
        let conn = self.conn()?;
        // A lease survives process death only until its bounded deadline. Never reset attempts.
        conn.execute("UPDATE interaction_comment_verification SET owner=NULL,lease_until_ms=NULL,revision=revision+1 WHERE owner IS NOT NULL AND lease_until_ms<=?1",[now])?;
        let mut stmt=conn.prepare("UPDATE interaction_comment_verification SET state='needsReview',reason=COALESCE(reason,'verification_budget_exhausted'),revision=revision+1 WHERE state='pending' AND sent_at_ms IS NOT NULL AND owner IS NULL AND (deadline_ms<=?1 OR attempts>=3) RETURNING campaign_id")?;
        let changed = stmt
            .query_map([now], |r| r.get(0))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(changed)
    }
    pub fn due_comment_verifications(&self, now: i64) -> anyhow::Result<Vec<(String, String)>> {
        let conn = self.conn()?;
        let mut stmt=conn.prepare("SELECT v.assignment_id,v.device_id FROM interaction_comment_verification v JOIN interaction_assignments a ON a.id=v.assignment_id
           WHERE v.state='pending' AND v.sent_at_ms IS NOT NULL AND v.next_at_ms<=?1 AND v.deadline_ms>?1 AND v.attempts<3 AND v.owner IS NULL
           AND a.state='uncertain' ORDER BY v.next_at_ms LIMIT 20")?;
        let rows = stmt
            .query_map([now], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }
    pub fn claim_comment_verification(
        &self,
        assignment: &str,
        now: i64,
    ) -> anyhow::Result<Option<VerificationJob>> {
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let owner = Uuid::new_v4().to_string();
        let raw=tx.query_row("UPDATE interaction_comment_verification SET owner=?2,lease_until_ms=?3+90000,attempts=attempts+1,revision=revision+1,
             assignment_revision=(SELECT revision FROM interaction_assignments WHERE id=?1),
             action_revision=(SELECT revision FROM tiktok_action_runs WHERE assignment_id=?1 AND action_kind='comment')
           WHERE assignment_id=?1 AND state='pending' AND next_at_ms<=?3 AND deadline_ms>?3 AND attempts<3 AND owner IS NULL
           AND EXISTS(SELECT 1 FROM interaction_assignments WHERE id=?1 AND state='uncertain')
           AND NOT EXISTS(SELECT 1 FROM interaction_comment_verification other WHERE other.device_id=interaction_comment_verification.device_id AND other.owner IS NOT NULL)
           RETURNING campaign_id,context_json,revision,deadline_ms,attempts",params![assignment,owner,now],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,i64>(2)?,r.get::<_,i64>(3)?,r.get::<_,u32>(4)?))).optional()?;
        tx.commit()?;
        raw.map(|(campaign_id, raw, revision, deadline_ms, attempts)| {
            Ok(VerificationJob {
                assignment_id: assignment.into(),
                campaign_id,
                context: serde_json::from_str(&raw)?,
                owner,
                revision,
                deadline_ms,
                attempts,
            })
        })
        .transpose()
    }
    /// Atomic CAS of evidence, comment action and assignment. A stale observer cannot settle.
    pub fn settle_comment_verification(
        &self,
        job: &VerificationJob,
        identity: Option<&crate::CommentLocatorIdentity>,
        evidence: &str,
        reason: Option<&str>,
        now: i64,
    ) -> anyhow::Result<bool> {
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let owned:Option<(i64,i64,i64)>=tx.query_row("SELECT assignment_revision,action_revision,sent_at_ms FROM interaction_comment_verification WHERE assignment_id=?1 AND owner=?2 AND revision=?3 AND state='pending' AND lease_until_ms>=?4",params![job.assignment_id,job.owner,job.revision,now],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?;
        let Some((assignment_revision, action_revision, sent_at)) = owned else {
            return Ok(false);
        };
        let timestamp = Utc::now().to_rfc3339();
        anyhow::ensure!(
            identity.is_none_or(|i| i.text == job.context.text
                && !i.author_label.is_empty()
                && !i.frame_sha256.is_empty()),
            "invalid comment verification identity"
        );
        let identity = identity.filter(|_| now <= job.deadline_ms);
        let state = if identity.is_some() {
            "verified"
        } else if job.attempts >= 3 || now >= job.deadline_ms {
            "needsReview"
        } else {
            "pending"
        };
        if let Some(identity) = identity {
            let raw: Option<String> = tx.query_row(
                "SELECT evidence_json FROM interaction_assignments WHERE id=?1",
                [&job.assignment_id],
                |r| r.get(0),
            )?;
            let mut data = raw
                .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
                .unwrap_or_else(|| serde_json::json!({}));
            data["postedIdentity"] = serde_json::to_value(identity)?;
            data["commentVerification"] = serde_json::from_str(evidence)?;
            data["commentVerificationPending"] = false.into();
            let action=tx.execute("UPDATE tiktok_action_runs SET state='confirmed',evidence_json=?1,error_code=NULL,revision=revision+1,updated_at=?2 WHERE assignment_id=?3 AND action_kind='comment' AND state='uncertain' AND revision=?4",params![serde_json::json!({"verdict":"verified","postedIdentity":identity,"verification":serde_json::from_str::<serde_json::Value>(evidence)?}).to_string(),timestamp,job.assignment_id,action_revision])?;
            let assignment=tx.execute("UPDATE interaction_assignments SET state='succeeded',evidence_json=?1,error_code=NULL,revision=revision+1,updated_at=?2 WHERE id=?3 AND state='uncertain' AND revision=?4",params![data.to_string(),timestamp,job.assignment_id,assignment_revision])?;
            anyhow::ensure!(
                action == 1 && assignment == 1,
                "verification settlement lost ownership"
            );
        }
        let next = sent_at + if job.attempts == 1 { 20000 } else { 60000 };
        tx.execute("UPDATE interaction_comment_verification SET state=?1,next_at_ms=?2,reason=?3,evidence_json=?4,owner=NULL,lease_until_ms=NULL,revision=revision+1 WHERE assignment_id=?5",params![state,next,reason,evidence,job.assignment_id])?;
        // Running coordinators own their campaign state. Terminal projections can be refreshed.
        tx.execute("UPDATE interaction_campaigns SET state=CASE WHEN NOT EXISTS(SELECT 1 FROM interaction_assignments WHERE campaign_id=?1 AND state<>'succeeded') THEN 'succeeded'
             WHEN EXISTS(SELECT 1 FROM interaction_assignments WHERE campaign_id=?1 AND state='succeeded') THEN 'partial' ELSE 'failed' END,revision=revision+1,updated_at=?2
             WHERE id=?1 AND state IN ('succeeded','partial','failed')",params![job.campaign_id,timestamp])?;
        tx.execute("UPDATE interaction_campaigns SET error_code=NULL WHERE id=?1 AND state IN ('succeeded','partial','failed')",[&job.campaign_id])?;
        tx.commit()?;
        Ok(true)
    }
    pub fn request_comment_verification(
        &self,
        campaign: &str,
        assignment: &str,
    ) -> anyhow::Result<CommentVerification> {
        let detail = self
            .get_interaction_campaign(campaign)?
            .context("Chiến dịch không tồn tại")?;
        let row = detail
            .assignments
            .iter()
            .find(|a| a.id == assignment)
            .context("Bình luận không thuộc chiến dịch")?;
        anyhow::ensure!(
            matches!(
                row.state,
                crate::ThreadMessageState::Uncertain | crate::ThreadMessageState::Succeeded
            ),
            "Chỉ đọc lại lượt đã gửi"
        );
        let now = Utc::now().timestamp_millis();
        let conn = self.conn()?;
        let existing = self.comment_verification(assignment)?;
        if existing.as_ref().is_some_and(|v| {
            matches!(
                v.state,
                VerificationState::Pending | VerificationState::Verified
            )
        }) {
            return Ok(existing.unwrap());
        }
        if existing.is_none() {
            anyhow::ensure!(
                row.actions
                    .iter()
                    .any(|a| a.kind == crate::InteractionActionKind::Comment
                        && a.effect_intent
                            .as_deref()
                            .is_some_and(|e| matches!(e, "post_comment" | "reply_comment"))),
                "Không có bằng chứng lần gửi để đọc lại"
            );
            let (request, _) = self
                .get_interaction_campaign_request(campaign)?
                .context("Thiếu kịch bản")?;
            let target = request
                .targets
                .iter()
                .find(|t| t.target_key == row.target_key)
                .context("Thiếu bài")?
                .clone();
            let account = request
                .scripted_conversation
                .as_ref()
                .and_then(|s| {
                    s.role_bindings
                        .iter()
                        .find(|r| r.udid == row.actor_udid)
                        .map(|r| r.username.clone())
                })
                .unwrap_or(self.get_device_meta(&row.actor_udid)?.handle);
            anyhow::ensure!(
                !account.trim().is_empty(),
                "Cần username để kiểm tra người đăng"
            );
            let parent = row
                .parent_assignment_id
                .as_ref()
                .and_then(|id| detail.assignments.iter().find(|a| &a.id == id))
                .and_then(|a| a.posted_identity());
            anyhow::ensure!(
                row.parent_assignment_id.is_none() || parent.is_some(),
                "Thiếu bằng chứng bình luận cha; cần đọc lại câu cha trước"
            );
            let mut root_row = row;
            while let Some(parent_id) = &root_row.parent_assignment_id {
                root_row = detail
                    .assignments
                    .iter()
                    .find(|a| &a.id == parent_id)
                    .context("Thiếu câu gốc")?;
            }
            let root = if row.parent_assignment_id.is_some() {
                root_row.posted_identity()
            } else {
                None
            };
            let mentions = request
                .scripted_conversation
                .as_ref()
                .map(|s| s.mentions(&row.target_key, row.ordinal))
                .unwrap_or_default();
            let text = if let Some(identity) = row.posted_identity() {
                identity.text
            } else {
                let mut text = row.prepared_text.clone().context("Thiếu nội dung đã gửi")?;
                for handle in &mentions {
                    text.push_str(" @");
                    text.push_str(handle.trim_start_matches('@'));
                }
                text
            };
            let context = VerificationContext {
                target,
                device_id: row.actor_udid.clone(),
                account,
                text,
                mentions,
                parent: parent.clone(),
                root,
            };
            conn.execute("INSERT OR IGNORE INTO interaction_comment_verification(assignment_id,campaign_id,device_id,context_json,state,reason) VALUES(?1,?2,?3,?4,'needsReview','legacy_missing_evidence')",params![assignment,campaign,row.actor_udid,serde_json::to_string(&context)?])?;
        }
        // Explicit readback opens a fresh bounded observation budget, never a Send budget.
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute("UPDATE interaction_comment_verification SET state='pending',attempts=0,sent_at_ms=?2,next_at_ms=?2,deadline_ms=?2+120000,reason=NULL,revision=revision+1 WHERE assignment_id=?1 AND owner IS NULL AND state='needsReview'",params![assignment,now])?;
        tx.execute("UPDATE interaction_assignments SET state='uncertain',revision=revision+1 WHERE id=?1 AND state='succeeded'",[assignment])?;
        tx.execute("UPDATE tiktok_action_runs SET state='uncertain',revision=revision+1 WHERE assignment_id=?1 AND action_kind='comment' AND state='confirmed'",[assignment])?;
        tx.execute("UPDATE interaction_campaigns SET state='partial',revision=revision+1 WHERE id=?1 AND state='succeeded'",[campaign])?;
        tx.commit()?;
        self.comment_verification(assignment)?
            .context("Verification request missing")
    }
}
