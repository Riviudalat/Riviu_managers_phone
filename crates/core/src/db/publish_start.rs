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
        let mut rows: Vec<(String, i64)> = query.query_map([udid], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?;
        for hold in Self::publish_device_guard_from_connection(&conn, udid)?.blocking {
            if !rows.iter().any(|(id, _)| id == &hold.assignment_id) {
                let revision = conn.query_row("SELECT revision FROM publish_assignments WHERE id=?1",
                    [&hold.assignment_id], |r| r.get(0))?;
                rows.push((hold.assignment_id, revision));
            }
        }
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
    /// Explicitly retire an unacknowledged ID, not an inference that a null read failed.
    /// The IMMEDIATE transaction races acceptance/create: either their receipt wins, or
    /// this terminal tombstone wins and every later use of the ID is refused.
    pub fn cancel_unaccepted_publish_start(&self, request: &str) -> anyhow::Result<PublishStartReceipt> {
        Uuid::parse_str(request)?;
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(receipt) = start_on(&tx, request)? { return Ok(receipt); }
        let legacy: Option<(String, String)> = tx.query_row(
            "SELECT campaign_id,request_fingerprint FROM publish_create_requests WHERE request_id=?1",
            [request], |r| Ok((r.get(0)?,r.get(1)?))).optional()?;
        if let Some((campaign, fingerprint)) = legacy {
            // Adopt the authoritative create receipt without dispatching or cancelling it.
            tx.execute("INSERT INTO publish_start_requests(request_id,input_digest,request_fingerprint,preparation_id,campaign_id,state,stage,updated_at)
                VALUES(?1,?2,?3,?1,?4,'uncertain','existingCampaign',?5)",
                params![request,"0".repeat(64),fingerprint,campaign,Utc::now().to_rfc3339()])?;
        } else {
            let error = PublishStartError { code:"PublishStartCancelled".into(),
                message:"Đã hủy mã yêu cầu chưa được tiếp nhận; mã này không thể dùng lại. Kiểm tra lại để tạo lượt mới.".into() };
            // Sentinel fingerprint deliberately does not assert knowledge of a lost body.
            // accept/reject validate body fingerprints, legacy create requires preparing.
            tx.execute("INSERT INTO publish_start_requests(request_id,input_digest,request_fingerprint,preparation_id,state,stage,error_json,updated_at)
                VALUES(?1,?2,'cancelled-without-body',?1,'failed','cancelledBeforeAcceptance',?3,?4)",
                params![request,"0".repeat(64),serde_json::to_string(&error)?,Utc::now().to_rfc3339()])?;
        }
        let receipt = start_on(&tx, request)?.context("start cancellation receipt missing")?;
        tx.commit()?;
        Ok(receipt)
    }

    pub fn publish_start_status(
        &self,
        request: &str,
    ) -> anyhow::Result<Option<PublishStartReceipt>> {
        Uuid::parse_str(request)?;
        start_on(&self.conn()?, request)
    }

    /// Only the insert winner owns preparation. No phone, scan, handoff or network precedes it.
    /// The same transaction account-locks every receiving phone (review R3), before Start reads
    /// any account to compare with the approval, under the caller's already registered
    /// preparation `generation`, and records that generation as the Start's owner (review R4-F1).
    pub fn accept_publish_start(
        &self,
        request: &str,
        digest: &str,
        fingerprint: &str,
        preparation: &str,
        generation: &str,
        udids: &[String],
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
        if is_new {
            super::publish_pipeline::lock_preparing_accounts(&tx, generation, request, udids)?;
            super::publish_pipeline::record_start_owner(&tx, request, generation)?;
        }
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
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let changed = tx.execute(
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
        )? == 1;
        // Leaving preparation settles the Start: its owner record goes, and its preparation
        // locks, which no longer bind, are deleted; a campaign's own lock still binds and stays.
        if changed && !matches!(state, "accepted" | "preparing") {
            super::publish_pipeline::forget_start_owner(&tx, request)?;
            super::publish_pipeline::release_settled_account_locks(&tx, Some(request), None)?;
        }
        tx.commit()?;
        Ok(changed)
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
        let receipt = exclude_on(&tx, assignment, revision, request, handoff, None)?;
        tx.commit()?;
        Ok(receipt)
    }

    /// Step 1 of a scheduled job's prior-post release (`release_prior_posts`), one IMMEDIATE
    /// transaction. Re-checks that the job is the current running attempt of a pre-Post row whose
    /// approval carries `priorPosts`, and that every blocking post of another campaign on its phone
    /// is still Releasable at the revision read here. Then excludes each one exactly as a confirmed
    /// replacement does (debt kept, `handoff-pending` marker written) and records the receipt
    /// `publish.prior-post-release.{job}` in state `requested`. `Ok(None)`: nothing to release.
    /// Any mismatch commits nothing and returns the typed `prior_post_pending` failure.
    pub fn request_prior_post_release(
        &self,
        assignment: &str,
        run: &super::PublishPipelineRun,
        udid: &str,
        scope: &super::PriorPostScope<'_>,
    ) -> anyhow::Result<Result<Option<PriorPostRelease>, crate::publish_recovery::RecoveryFailure>>
    {
        use crate::publish_recovery::prior_post_pending;
        let mut conn = self.conn()?;
        // Most phones hold no earlier post: answer from a read, without the write lock.
        if super::publish_verification::prior_post_outlook_on(
            &conn,
            udid,
            Some(&run.campaign_id),
            scope,
        )?
        .is_empty()
        {
            return Ok(Ok(None));
        }
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        // The running attempt of this pre-Post row under this pipeline run, on this phone.
        let attempt: Option<String> = tx
            .query_row(
                "SELECT j.attempt_id FROM publish_dispatch_jobs j
                 JOIN publish_assignments a ON a.id=j.assignment_id
                 JOIN publish_pipeline_runs r ON r.campaign_id=j.campaign_id AND r.token=j.run_token
                 WHERE j.assignment_id=?1 AND j.campaign_id=?2 AND j.run_token=?3 AND j.udid=?4
                 AND j.state='running' AND a.effect_intent IS NULL
                 AND NOT EXISTS(SELECT 1 FROM publish_exclude_requests e WHERE e.assignment_id=a.id)",
                params![assignment, run.campaign_id, run.token, udid],
                |r| r.get(0),
            )
            .optional()?;
        let Some(attempt) = attempt else {
            return Ok(Err(prior_post_pending(
                "Lượt đăng đã đổi trước khi nhả bài cũ; chưa thao tác trên máy",
            )));
        };
        let holds = super::publish_verification::prior_post_outlook_on(
            &tx,
            udid,
            Some(&run.campaign_id),
            scope,
        )?;
        if holds.is_empty() {
            return Ok(Ok(None));
        }
        let raw: String = tx.query_row(
            "SELECT request_json FROM publish_campaigns WHERE id=?1",
            [&run.campaign_id],
            |r| r.get(0),
        )?;
        let request: crate::PublishCampaignRequest = serde_json::from_str(&raw)?;
        let approved = request
            .approval
            .as_ref()
            .is_some_and(|approval| approval.prior_posts.is_some());
        if let Some(hold) = holds
            .iter()
            .find(|hold| !approved || hold.class != super::PriorPostClass::Releasable)
        {
            return Ok(Err(prior_post_pending(format!(
                "Bài của lượt {} trên máy này chưa nhả được ({}); chưa thao tác trên máy",
                short_id(&hold.campaign_id),
                hold.reason
            ))));
        }
        let key = prior_post_release_key(assignment);
        let pending: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM settings WHERE key=?1 AND json_valid(value)
             AND json_extract(value,'$.state')='requested')",
            [&key],
            |r| r.get(0),
        )?;
        if pending {
            return Ok(Err(prior_post_pending(
                "Lần nhả bài cũ trước của lượt này chưa khép lại; chưa thao tác trên máy",
            )));
        }
        let request_id = Uuid::new_v4().to_string();
        let reason = format!(
            "Tạm dừng xác minh để máy đăng bài hẹn giờ của lượt {}; bài này đã lên TikTok và tự xác minh tiếp khi máy được trả, không đăng lại.",
            short_id(&run.campaign_id)
        );
        let mut rows = Vec::with_capacity(holds.len());
        for hold in &holds {
            let exclusion = Uuid::new_v4().to_string();
            exclude_on(
                &tx,
                &hold.assignment_id,
                hold.revision,
                &exclusion,
                true,
                Some(&reason),
            )?;
            rows.push(PriorPostReleaseRow {
                assignment_id: hold.assignment_id.clone(),
                campaign_id: hold.campaign_id.clone(),
                revision: hold.revision,
                intent_sha256: hold.intent_sha256.clone(),
                publication_stage: hold.publication_stage,
                package: hold.package.clone(),
                exclusion_request_id: exclusion,
            });
        }
        let release = PriorPostRelease {
            request_id,
            assignment_id: assignment.to_owned(),
            campaign_id: run.campaign_id.clone(),
            attempt_id: attempt,
            udid: udid.to_owned(),
            rows,
        };
        tx.execute(
            "INSERT INTO settings(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            params![key, release.receipt("requested", None).to_string()],
        )?;
        tx.commit()?;
        Ok(Ok(Some(release)))
    }

    /// Step 4 of the release, after the job closed TikTok under its own lease: for each excluded
    /// row the same proof as `record_publish_handoff_release` (fence re-checked, reservation and
    /// marker deleted, exclusion settled), the receipt `released`, and the events of both rows.
    /// All rows or none. Returns the campaigns to announce.
    pub fn record_prior_post_release(
        &self,
        release: &PriorPostRelease,
        fences: &[String],
        terminations: &serde_json::Value,
    ) -> anyhow::Result<Vec<String>> {
        anyhow::ensure!(
            fences.len() == release.rows.len(),
            "Thiếu dấu khóa của bài cũ; chưa ghi nhận nhả máy"
        );
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let key = prior_post_release_key(&release.assignment_id);
        let state: Option<String> = tx
            .query_row(
                "SELECT json_extract(value,'$.state') FROM settings WHERE key=?1 AND json_valid(value)
                 AND json_extract(value,'$.requestId')=?2",
                params![key, release.request_id],
                |r| r.get(0),
            )
            .optional()?;
        anyhow::ensure!(
            state.as_deref() == Some("requested"),
            "Biên nhận nhả bài cũ đã đổi; chưa ghi nhận nhả máy"
        );
        let now = Utc::now().to_rfc3339();
        for (row, fence) in release.rows.iter().zip(fences) {
            anyhow::ensure!(
                handoff_fence(&tx, &row.assignment_id, &row.exclusion_request_id)? == *fence,
                "Bài cũ thay đổi trong lúc đóng máy; chưa ghi nhận nhả máy"
            );
            let proof = serde_json::json!({"requestId":row.exclusion_request_id,"fence":fence});
            tx.execute("INSERT INTO settings(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
                params![format!("publish.handoff-release.{}", row.assignment_id),proof.to_string()])?;
            tx.execute(
                "DELETE FROM publish_account_reservations WHERE assignment_id=?1",
                [&row.assignment_id],
            )?;
            tx.execute(
                "DELETE FROM settings WHERE key=?1 AND value=?2",
                params![
                    format!("publish.handoff-pending.{}", row.assignment_id),
                    row.exclusion_request_id
                ],
            )?;
            settle_exclusion_after_release(&tx, &row.assignment_id)?;
            tx.execute("INSERT INTO operation_device_events(source_kind,source_id,udid,action,state,recorded_at,text,detail)
                VALUES('publish',?1,?2,'publishPriorPostRelease','released',?3,?4,?5)",
                params![row.campaign_id, release.udid, now,
                    format!("Tạm dừng xác minh để máy đăng bài hẹn giờ của lượt {}; bài này đã lên TikTok và tự xác minh tiếp khi máy được trả, không đăng lại.", short_id(&release.campaign_id)),
                    serde_json::json!({"requestId":release.request_id,"assignmentId":row.assignment_id,"exclusionRequestId":row.exclusion_request_id,"forAssignmentId":release.assignment_id}).to_string()])?;
        }
        let text = format!(
            "Bài cũ của lượt {} đã lên TikTok nhưng chưa có link: tạm dừng xác minh bài đó, đóng TikTok để đăng bài hẹn giờ; bài cũ tự xác minh tiếp khi đăng xong, không đăng lại.",
            release
                .rows
                .iter()
                .map(|row| short_id(&row.campaign_id))
                .collect::<std::collections::BTreeSet<_>>()
                .into_iter()
                .collect::<Vec<_>>()
                .join(", ")
        );
        tx.execute("INSERT INTO operation_device_events(source_kind,source_id,udid,action,state,recorded_at,text,detail)
            VALUES('publish',?1,?2,'publishPriorPostRelease','released',?3,?4,?5)",
            params![release.campaign_id, release.udid, now, text,
                serde_json::json!({"requestId":release.request_id,"assignmentId":release.assignment_id}).to_string()])?;
        tx.execute(
            "UPDATE publish_campaigns SET revision=revision+1,updated_at=?2 WHERE id=?1",
            params![release.campaign_id, now],
        )?;
        tx.execute(
            "INSERT INTO publish_events(campaign_id,revision,kind,payload_json,created_at)
            SELECT id,revision,'priorPostRelease',?2,?3 FROM publish_campaigns WHERE id=?1",
            params![
                release.campaign_id,
                serde_json::json!({"source":"prior_post_release","requestId":release.request_id,
                    "assignmentId":release.assignment_id,"udid":release.udid,
                    "rows":release.rows.iter().map(|row| &row.assignment_id).collect::<Vec<_>>()})
                .to_string(),
                now
            ],
        )?;
        tx.execute(
            "UPDATE settings SET value=?2 WHERE key=?1",
            params![
                key,
                release.receipt("released", Some(terminations)).to_string()
            ],
        )?;
        tx.commit()?;
        let mut campaigns = vec![release.campaign_id.clone()];
        for row in &release.rows {
            if !campaigns.contains(&row.campaign_id) {
                campaigns.push(row.campaign_id.clone());
            }
        }
        Ok(campaigns)
    }

    /// Step 5: the release of `job` failed after its request. Each excluded row with no current
    /// release proof gets a resume stamp tied to its exclusion, so verification carries on as
    /// before, and its marker goes; the receipt becomes `aborted`. Returns the campaigns changed,
    /// empty when the receipt was not `requested` (already settled).
    pub fn abort_prior_post_release(
        &self,
        job_assignment: &str,
        error: &str,
    ) -> anyhow::Result<Vec<String>> {
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let campaigns = abort_prior_post_release_on(&tx, job_assignment, error)?;
        tx.commit()?;
        Ok(campaigns)
    }

    /// Receipts still `requested` whose job no longer runs that attempt: their release ended
    /// without its abort. The dispatcher aborts each one.
    pub fn stale_prior_post_releases(&self) -> anyhow::Result<Vec<String>> {
        let conn = self.conn()?;
        let mut statement = conn.prepare(
            "SELECT json_extract(s.value,'$.assignmentId') FROM settings s
             WHERE s.key GLOB 'publish.prior-post-release.*' AND json_valid(s.value)
             AND json_extract(s.value,'$.state')='requested'
             AND NOT EXISTS(SELECT 1 FROM publish_dispatch_jobs j
                 WHERE j.assignment_id=json_extract(s.value,'$.assignmentId') AND j.state='running'
                 AND j.attempt_id=json_extract(s.value,'$.attemptId'))",
        )?;
        let rows = statement
            .query_map([], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }
}

/// One earlier post a scheduled job excluded to release, at the revision it read.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PriorPostReleaseRow {
    pub assignment_id: String,
    pub campaign_id: String,
    pub revision: i64,
    pub intent_sha256: Option<String>,
    pub publication_stage: u64,
    pub package: Option<String>,
    pub exclusion_request_id: String,
}

/// The receipt of one prior-post release by a scheduled job (`publish.prior-post-release.{job}`).
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PriorPostRelease {
    pub request_id: String,
    pub assignment_id: String,
    pub campaign_id: String,
    pub attempt_id: String,
    pub udid: String,
    pub rows: Vec<PriorPostReleaseRow>,
}

impl PriorPostRelease {
    fn receipt(&self, state: &str, terminations: Option<&serde_json::Value>) -> serde_json::Value {
        let mut receipt = serde_json::json!({
            "version": 1, "state": state, "requestId": self.request_id,
            "assignmentId": self.assignment_id, "campaignId": self.campaign_id,
            "attemptId": self.attempt_id, "udid": self.udid, "rows": self.rows,
            "updatedAt": Utc::now().to_rfc3339(),
        });
        if let Some(terminations) = terminations {
            receipt["terminations"] = terminations.clone();
        }
        receipt
    }
}

fn prior_post_release_key(job_assignment: &str) -> String {
    format!("publish.prior-post-release.{job_assignment}")
}

fn short_id(id: &str) -> &str {
    id.get(..8).unwrap_or(id)
}

/// campaign, phone, revision, Post intent and evidence of a row about to get a resume stamp.
type StampedRow = (String, String, i64, Option<String>, Option<String>);

/// The abort body, shared with startup recovery (`interrupt_orphaned_publish_campaigns`).
pub(super) fn abort_prior_post_release_on(
    conn: &Connection,
    job_assignment: &str,
    error: &str,
) -> anyhow::Result<Vec<String>> {
    use sha2::Digest;
    let key = prior_post_release_key(job_assignment);
    let raw: Option<String> = conn
        .query_row(
            "SELECT value FROM settings WHERE key=?1 AND json_valid(value)
             AND json_extract(value,'$.state')='requested'",
            [&key],
            |r| r.get(0),
        )
        .optional()?;
    let Some(raw) = raw else {
        return Ok(Vec::new());
    };
    let mut receipt: serde_json::Value = serde_json::from_str(&raw)?;
    let now = Utc::now().to_rfc3339();
    let job_campaign = receipt["campaignId"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    let mut campaigns = Vec::new();
    for row in receipt["rows"].as_array().cloned().unwrap_or_default() {
        let (Some(assignment), Some(exclusion)) = (
            row["assignmentId"].as_str(),
            row["exclusionRequestId"].as_str(),
        ) else {
            continue;
        };
        let released = conn
            .query_row(
                "SELECT value FROM settings WHERE key=?1",
                [format!("publish.handoff-release.{assignment}")],
                |r| r.get::<_, String>(0),
            )
            .optional()?
            .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok())
            .is_some_and(|proof| proof["requestId"] == exclusion);
        conn.execute(
            "DELETE FROM settings WHERE key=?1 AND value=?2",
            params![format!("publish.handoff-pending.{assignment}"), exclusion],
        )?;
        if released {
            continue;
        }
        // The stamp `handoff_resumed` reads: observation is authorized again for exactly this
        // exclusion. Verification status and budget stay as they were.
        let current: Option<StampedRow> = conn
            .query_row(
                "SELECT campaign_id,udid,revision,effect_intent,evidence_json FROM publish_assignments WHERE id=?1",
                [assignment],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
            )
            .optional()?;
        let Some((campaign, udid, revision, Some(intent), evidence)) = current else {
            continue;
        };
        let Some(mut evidence) = evidence
            .as_deref()
            .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok())
            .filter(serde_json::Value::is_object)
        else {
            continue;
        };
        let reason = format!(
            "Lượt hẹn giờ {} chưa nhả được máy; tiếp tục xác minh bài này như trước, không đăng lại.",
            short_id(&job_campaign)
        );
        evidence["verificationHandoffResume"] = serde_json::json!({
            "version":1,"assignmentId":assignment,"campaignId":campaign,"udid":udid,
            "exclusionRequestId":exclusion,
            "intentSha256":format!("{:x}",sha2::Sha256::digest(intent.as_bytes())),
            "resumedAt":now,"reason":reason,"statusBefore":evidence["verificationStatus"].clone(),
            "source":"prior_post_release_aborted",
        });
        conn.execute(
            "UPDATE publish_assignments SET evidence_json=?1,revision=revision+1,updated_at=?2
             WHERE id=?3 AND revision=?4",
            params![evidence.to_string(), now, assignment, revision],
        )?;
        conn.execute("INSERT INTO operation_device_events(source_kind,source_id,udid,action,state,recorded_at,text,detail)
            VALUES('publish',?1,?2,'publishPriorPostRelease','aborted',?3,?4,?5)",
            params![campaign, udid, now, reason,
                serde_json::json!({"requestId":receipt["requestId"],"assignmentId":assignment,"exclusionRequestId":exclusion,"error":error}).to_string()])?;
        if !campaigns.contains(&campaign) {
            campaigns.push(campaign);
        }
    }
    receipt["state"] = serde_json::json!("aborted");
    receipt["error"] = serde_json::json!(error);
    receipt["updatedAt"] = serde_json::json!(now);
    conn.execute(
        "UPDATE settings SET value=?2 WHERE key=?1",
        params![key, receipt.to_string()],
    )?;
    if !job_campaign.is_empty() && !campaigns.contains(&job_campaign) {
        campaigns.push(job_campaign);
    }
    Ok(campaigns)
}

/// The exclusion body shared by an operator's exclusion, a confirmed replacement (`handoff`) and a
/// scheduled job's prior-post release (`reason` names that job). Debt rows keep their state.
fn exclude_on(
    tx: &Connection,
    assignment: &str,
    revision: i64,
    request: &str,
    handoff: bool,
    reason: Option<&str>,
) -> anyhow::Result<PublishExcludeReceipt> {
    if let Some(receipt) = exclusion_on(tx, request)? {
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
    let reason =
        if debt {
            Some(reason.unwrap_or(
                "Bài có thể đã đăng; giữ bằng chứng và nợ xác minh/Sheet, không đăng lại",
            ))
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
    exclusion_on(tx, request)?.context("exclusion receipt missing")
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
