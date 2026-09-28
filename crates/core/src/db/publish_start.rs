//! Durable acceptance before preparation; a replay is an observation, never a dispatch.
use super::*;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PublishStartError {
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PublishStartReceipt {
    pub request_id: String,
    pub operation_id: String,
    pub input_digest: String,
    pub preparation_id: String,
    pub reserved_campaign_id: String,
    pub campaign_id: Option<String>,
    pub state: String,
    pub stage: String,
    pub error: Option<PublishStartError>,
    pub revision: i64,
    pub updated_at: String,
}

fn start_on(conn: &Connection, id: &str) -> anyhow::Result<Option<PublishStartReceipt>> {
    let row = conn.query_row(
        "SELECT request_id,input_digest,preparation_id,campaign_id,state,stage,error_json,revision,updated_at
         FROM publish_start_requests WHERE request_id=?1", [id], |r| {
            Ok((PublishStartReceipt {
                request_id: r.get(0)?, operation_id: format!("publish-start:{id}"),
                input_digest: r.get(1)?, preparation_id: r.get(2)?, campaign_id: r.get(3)?,
                reserved_campaign_id: r.get::<_, Option<String>>(3)?.unwrap_or_else(|| id.to_owned()),
                state: r.get(4)?, stage: r.get(5)?, error: None, revision: r.get(7)?, updated_at: r.get(8)?,
            }, r.get::<_, Option<String>>(6)?))
        }).optional()?;
    row.map(|(mut receipt, error)| {
        receipt.error = error.as_deref().map(serde_json::from_str).transpose()?;
        Ok(receipt)
    })
    .transpose()
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PublishExcludeReceipt {
    pub assignment_id: String,
    pub state: String,
    pub revision: i64,
    pub reason: Option<String>,
}

fn exclusion_on(conn: &Connection, request: &str) -> anyhow::Result<Option<PublishExcludeReceipt>> {
    Ok(conn
        .query_row(
            "SELECT e.assignment_id,e.state,a.revision,e.reason
        FROM publish_exclude_requests e JOIN publish_assignments a ON a.id=e.assignment_id
        WHERE e.request_id=?1",
            [request],
            |r| {
                Ok(PublishExcludeReceipt {
                    assignment_id: r.get(0)?,
                    state: r.get(1)?,
                    revision: r.get(2)?,
                    reason: r.get(3)?,
                })
            },
        )
        .optional()?)
}

impl Database {
    /// Exact device scope for replacement; scheduled work is not today's owner.
    pub fn publish_handoff_assignments(&self, udid: &str) -> anyhow::Result<Vec<(String, i64)>> {
        let conn = self.conn()?;
        let mut query = conn.prepare("SELECT a.id,a.revision FROM publish_assignments a
            JOIN publish_campaigns c ON c.id=a.campaign_id WHERE a.udid=?1 AND c.state<>'scheduled'
            AND (a.state IN ('queued','preparing','ready','transferring','imported','posting','verifying','uncertain')
                OR EXISTS(SELECT 1 FROM publish_account_reservations r WHERE r.assignment_id=a.id)
                OR EXISTS(SELECT 1 FROM settings WHERE key='publish.handoff-pending.' || a.id))
            ORDER BY a.id")?;
        let rows = query.query_map([udid], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    pub fn publish_handoff_worker_running(&self, assignment: &str) -> anyhow::Result<bool> {
        Ok(self.conn()?.query_row("SELECT EXISTS(SELECT 1 FROM publish_dispatch_jobs WHERE assignment_id=?1 AND state='running') OR EXISTS(SELECT 1 FROM publish_work_claims WHERE owner=?1)",
            [assignment], |r| r.get(0))?)
    }

    /// Snapshot after the old worker drained, checked again after physical closure.
    pub fn publish_handoff_fence(&self, assignment: &str, request: &str) -> anyhow::Result<String> {
        handoff_fence(&self.conn()?, assignment, request)
    }

    /// Caller holds the handoff lock and has closed and dropped this device's exclusive
    /// context. Only this assignment's reservation is released; its Post evidence remains.
    pub fn record_publish_handoff_release(&self, assignment: &str, request: &str, expected: &str) -> anyhow::Result<()> {
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        anyhow::ensure!(handoff_fence(&tx, assignment, request)? == expected,
            "Bài cũ thay đổi trong lúc đóng máy; chưa ghi nhận nhả máy");
        let proof = serde_json::json!({"requestId":request,"fence":expected});
        tx.execute("INSERT INTO settings(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            params![format!("publish.handoff-release.{assignment}"),proof.to_string()])?;
        tx.execute("DELETE FROM publish_account_reservations WHERE assignment_id=?1", [assignment])?;
        tx.execute("DELETE FROM settings WHERE key=?1 AND value=?2",
            params![format!("publish.handoff-pending.{assignment}"),request])?;
        settle_exclusion_after_release(&tx, assignment)?;
        tx.commit()?;
        Ok(())
    }

    pub fn pending_publish_exclusion_releases(
        &self,
    ) -> anyhow::Result<Vec<(String, String, String)>> {
        let conn = self.conn()?;
        let mut statement = conn.prepare("SELECT DISTINCT a.id,a.udid,a.campaign_id FROM publish_assignments a
            JOIN publish_exclude_requests e ON e.assignment_id=a.id WHERE e.state='stopping'
            AND NOT EXISTS(SELECT 1 FROM publish_dispatch_jobs j WHERE j.assignment_id=a.id AND j.state='running')")?;
        let rows = statement
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    /// Caller is the existing dispatcher, after join and a fresh owner-release observation.
    pub fn finish_publish_exclusion_after_release(&self, assignment: &str) -> anyhow::Result<()> {
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let running: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM publish_dispatch_jobs WHERE assignment_id=?1 AND state='running')",[assignment], |r| r.get(0))?;
        anyhow::ensure!(!running, "Assignment still owns a worker");
        settle_exclusion_after_release(&tx, assignment)?;
        tx.commit()?;
        Ok(())
    }
    pub fn publish_start_status(
        &self,
        request: &str,
    ) -> anyhow::Result<Option<PublishStartReceipt>> {
        Uuid::parse_str(request)?;
        start_on(&self.conn()?, request)
    }

    /// Only the insert winner owns preparation. No phone, scan, handoff or network precedes it.
    pub fn accept_publish_start(
        &self,
        request: &str,
        digest: &str,
        fingerprint: &str,
        preparation: &str,
    ) -> anyhow::Result<(PublishStartReceipt, bool)> {
        Uuid::parse_str(request)?;
        anyhow::ensure!(
            digest.len() == 64 && digest.bytes().all(|b| b.is_ascii_hexdigit()),
            "Invalid input digest"
        );
        anyhow::ensure!(fingerprint.len() == 64, "Invalid request fingerprint");
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(receipt) = start_on(&tx, request)? {
            let prior: String = tx.query_row(
                "SELECT request_fingerprint FROM publish_start_requests WHERE request_id=?1",
                [request],
                |r| r.get(0),
            )?;
            anyhow::ensure!(
                prior == fingerprint && receipt.input_digest == digest,
                "requestId đã dùng cho nội dung khác; tạo lượt mới để thay đổi nội dung"
            );
            return Ok((receipt, false));
        }
        // A legacy create receipt with this ID is already authoritative. Adopting it must not Post.
        let campaign: Option<String> = tx
            .query_row(
                "SELECT campaign_id FROM publish_create_requests WHERE request_id=?1",
                [request],
                |r| r.get(0),
            )
            .optional()?;
        if campaign.is_some() {
            let prior: String = tx.query_row(
                "SELECT request_fingerprint FROM publish_create_requests WHERE request_id=?1",
                [request],
                |r| r.get(0),
            )?;
            anyhow::ensure!(prior == fingerprint, "requestId đã dùng cho nội dung khác");
        }
        let is_new = campaign.is_none();
        tx.execute("INSERT INTO publish_start_requests(request_id,input_digest,request_fingerprint,preparation_id,campaign_id,state,stage,updated_at)
            VALUES(?1,?2,?3,?4,?5,?6,?7,?8)", params![request,digest,fingerprint,preparation,campaign,
                if is_new {"accepted"} else {"uncertain"}, if is_new {"accepted"} else {"existingCampaign"},Utc::now().to_rfc3339()])?;
        let receipt = start_on(&tx, request)?.context("start receipt missing")?;
        tx.commit()?;
        Ok((receipt, is_new))
    }

    /// Record a pre-acceptance refusal for exactly this request ID, so the frontend can
    /// retire its pending marker from a durable fact instead of guessing from an error code.
    /// An existing receipt is returned unchanged only when it binds the exact same input
    /// (digest and fingerprint); a refusal never overwrites a request that may own a campaign,
    /// and never silently answers for a different body reusing the ID.
    pub fn reject_publish_start(
        &self,
        request: &str,
        digest: &str,
        fingerprint: &str,
        error: &PublishStartError,
    ) -> anyhow::Result<PublishStartReceipt> {
        Uuid::parse_str(request)?;
        anyhow::ensure!(
            digest.len() == 64 && digest.bytes().all(|b| b.is_ascii_hexdigit()),
            "Invalid input digest"
        );
        anyhow::ensure!(fingerprint.len() == 64, "Invalid request fingerprint");
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(receipt) = start_on(&tx, request)? {
            let prior: String = tx.query_row(
                "SELECT request_fingerprint FROM publish_start_requests WHERE request_id=?1",
                [request],
                |r| r.get(0),
            )?;
            anyhow::ensure!(
                prior == fingerprint && receipt.input_digest == digest,
                "requestId đã dùng cho nội dung khác; tạo lượt mới để thay đổi nội dung"
            );
            return Ok(receipt);
        }
        let legacy: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM publish_create_requests WHERE request_id=?1)",
            [request],
            |r| r.get(0),
        )?;
        anyhow::ensure!(!legacy, "requestId đã có lượt tạo cũ; kiểm tra lượt đó trước");
        tx.execute("INSERT INTO publish_start_requests(request_id,input_digest,request_fingerprint,preparation_id,campaign_id,state,stage,error_json,updated_at)
            VALUES(?1,?2,?3,?4,NULL,'failed','rejected',?5,?6)",
            params![request, digest, fingerprint, request, serde_json::to_string(error)?, Utc::now().to_rfc3339()])?;
        let receipt = start_on(&tx, request)?.context("start receipt missing")?;
        tx.commit()?;
        Ok(receipt)
    }

    /// Expected state fences late preparation completion after shutdown/recovery.
    pub fn advance_publish_start(
        &self,
        request: &str,
        expected: &str,
        state: &str,
        stage: &str,
        error: Option<&PublishStartError>,
    ) -> anyhow::Result<bool> {
        let error = error.map(serde_json::to_string).transpose()?;
        Ok(self.conn()?.execute(
            "UPDATE publish_start_requests SET state=?3,stage=?4,error_json=?5,
            revision=revision+1,updated_at=?6 WHERE request_id=?1 AND state=?2",
            params![
                request,
                expected,
                state,
                stage,
                error,
                Utc::now().to_rfc3339()
            ],
        )? == 1)
    }

    pub fn publish_assignment_excluded(&self, assignment: &str) -> anyhow::Result<bool> {
        Ok(self.conn()?.query_row(
            "SELECT EXISTS(SELECT 1 FROM publish_exclude_requests WHERE assignment_id=?1)",
            [assignment],
            |r| r.get(0),
        )?)
    }

    /// Cancel only this child under CAS. Public-effect evidence and Sheet debt are never removed.
    pub fn request_publish_exclusion(
        &self,
        assignment: &str,
        revision: i64,
        request: &str,
    ) -> anyhow::Result<PublishExcludeReceipt> {
        self.request_publish_exclusion_inner(assignment, revision, request, false)
    }

    /// Confirmation of a replacement keeps the account reserved until physical close.
    pub fn request_publish_handoff_exclusion(
        &self,
        assignment: &str,
        revision: i64,
        request: &str,
    ) -> anyhow::Result<PublishExcludeReceipt> {
        self.request_publish_exclusion_inner(assignment, revision, request, true)
    }

    fn request_publish_exclusion_inner(
        &self,
        assignment: &str,
        revision: i64,
        request: &str,
        handoff: bool,
    ) -> anyhow::Result<PublishExcludeReceipt> {
        Uuid::parse_str(request)?;
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(receipt) = exclusion_on(&tx, request)? {
            let prior: i64 = tx.query_row(
                "SELECT expected_revision FROM publish_exclude_requests WHERE request_id=?1",
                [request],
                |r| r.get(0),
            )?;
            anyhow::ensure!(
                receipt.assignment_id == assignment && prior == revision,
                "Exclude request identity changed"
            );
            return Ok(receipt);
        }
        let (actual, intent, state): (i64, Option<String>, String) = tx.query_row(
            "SELECT revision,effect_intent,state FROM publish_assignments WHERE id=?1",
            [assignment],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )?;
        anyhow::ensure!(actual == revision, "Bài đã thay đổi; tải lại tiến độ");
        let debt = intent.is_some()
            || matches!(
                state.as_str(),
                "posting" | "verifying" | "uncertain" | "succeeded"
            );
        let next = if debt { "needsReview" } else { "stopping" };
        let reason = if debt {
            Some("Bài có thể đã đăng; giữ bằng chứng và nợ xác minh/Sheet, không đăng lại")
        } else {
            None
        };
        tx.execute("INSERT INTO publish_exclude_requests(request_id,assignment_id,expected_revision,state,reason,updated_at)
            VALUES(?1,?2,?3,?4,?5,?6)", params![request,assignment,revision,next,reason,Utc::now().to_rfc3339()])?;
        if handoff {
            tx.execute("INSERT INTO settings(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
                params![format!("publish.handoff-pending.{assignment}"),request])?;
        }
        // A cancelled child fences transfer/composer CAS; Post-intent rows keep their exact state.
        tx.execute(
            "UPDATE publish_assignments SET state='cancelled',
            revision=revision+1,updated_at=?4 WHERE id=?1 AND revision=?2 AND NOT ?3",
            params![assignment, revision, debt, Utc::now().to_rfc3339()],
        )?;
        tx.execute("UPDATE publish_dispatch_jobs SET state='cancelled',reason='assignment_excluded',revision=revision+1,
            finished_at_ms=?2 WHERE assignment_id=?1 AND state IN ('queued','paused')",params![assignment,Utc::now().timestamp_millis()])?;
        tx.execute("UPDATE publish_recovery_state SET payload=json_set(payload,'$.state','stopped','$.nextRetryAt',NULL,'$.reconnectDeadline',NULL) WHERE assignment_id=?1",[assignment])?;
        let receipt = exclusion_on(&tx, request)?.context("exclusion receipt missing")?;
        tx.commit()?;
        Ok(receipt)
    }
}

fn handoff_fence(conn: &Connection, assignment: &str, request: &str) -> anyhow::Result<String> {
    use sha2::Digest;
    let row: (String, String, i64, Option<String>, Option<String>) = conn.query_row(
        "SELECT udid,state,revision,effect_intent,evidence_json FROM publish_assignments WHERE id=?1",
        [assignment], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?)))?;
    let excluded: bool = conn.query_row("SELECT EXISTS(SELECT 1 FROM publish_exclude_requests WHERE assignment_id=?1 AND request_id=?2 AND rowid=(SELECT MAX(rowid) FROM publish_exclude_requests WHERE assignment_id=?1))",
        params![assignment,request], |r| r.get(0))?;
    let running: bool = conn.query_row("SELECT EXISTS(SELECT 1 FROM publish_dispatch_jobs WHERE assignment_id=?1 AND state='running') OR EXISTS(SELECT 1 FROM publish_work_claims WHERE owner=?1)",
        [assignment], |r| r.get(0))?;
    anyhow::ensure!(excluded && !running, "Lượt cũ chưa nhả worker của máy");
    let excluded_at: String = conn.query_row("SELECT updated_at FROM publish_exclude_requests WHERE request_id=?1",
        [request], |r| r.get(0))?;
    let resumed_at = row.4.as_deref().and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok())
        .and_then(|e| e["verificationResume"]["requestedAt"].as_str().map(str::to_owned));
    if let Some(resumed) = resumed_at {
        anyhow::ensure!(chrono::DateTime::parse_from_rfc3339(&resumed)? <= chrono::DateTime::parse_from_rfc3339(&excluded_at)?,
            "Bài cũ đã được tiếp tục xác minh; kiểm tra lại trước khi nhả máy");
    }
    Ok(format!("{:x}", sha2::Sha256::digest(serde_json::to_vec(&(assignment,request,row))?)))
}

pub(super) fn handoff_release_current(conn: &Connection, assignment: &str) -> anyhow::Result<bool> {
    let raw: Option<String> = conn.query_row("SELECT value FROM settings WHERE key=?1",
        [format!("publish.handoff-release.{assignment}")], |r| r.get(0)).optional()?;
    let Some(raw) = raw else { return Ok(false) };
    let proof: serde_json::Value = serde_json::from_str(&raw)?;
    let Some(request) = proof["requestId"].as_str() else { return Ok(false) };
    // A newly running worker, changed evidence, revision or explicit resume invalidates proof.
    Ok(handoff_fence(conn, assignment, request).is_ok_and(|fence| proof["fence"] == fence))
}

/// Called only after the dispatcher joined the exact worker (its work permit and UI lease dropped).
pub(super) fn settle_exclusion_after_release(
    conn: &Connection,
    assignment: &str,
) -> anyhow::Result<()> {
    conn.execute("UPDATE publish_exclude_requests SET state=CASE
        WHEN EXISTS(SELECT 1 FROM publish_assignments WHERE id=?1 AND effect_intent IS NOT NULL) THEN 'needsReview'
        ELSE 'excluded' END,revision=revision+1,updated_at=?2 WHERE assignment_id=?1 AND state='stopping'",
        params![assignment,Utc::now().to_rfc3339()])?;
    conn.execute("DELETE FROM publish_account_reservations WHERE assignment_id=?1
        AND NOT EXISTS(SELECT 1 FROM settings WHERE key='publish.handoff-pending.' || ?1)
        AND EXISTS(SELECT 1 FROM publish_exclude_requests WHERE assignment_id=?1 AND state='excluded')
        AND EXISTS(SELECT 1 FROM publish_assignments WHERE id=?1 AND effect_intent IS NULL)",[assignment])?;
    Ok(())
}
