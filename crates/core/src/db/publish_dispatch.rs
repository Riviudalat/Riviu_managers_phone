//! Durable per-publication work. Only admitted stages become runtime tasks.
use super::*;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PublishLimits {
    pub transfer: usize,
    pub compose: usize,
    pub verify: usize,
    pub device_total: usize,
}

#[cfg(test)]
#[path = "publish_dispatch_tests.rs"]
mod tests;
impl Default for PublishLimits {
    fn default() -> Self {
        Self {
            transfer: 4,
            compose: 4,
            verify: 4,
            device_total: 8,
        }
    }
}
impl PublishLimits {
    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            [self.transfer, self.compose, self.verify, self.device_total]
                .iter()
                .all(|v| (1..=64).contains(v)),
            "Giới hạn đăng bài phải từ 1 đến 64"
        );
        Ok(())
    }
    fn stage(self, stage: &str) -> anyhow::Result<usize> {
        Ok(match stage {
            "transfer" => self.transfer,
            "compose" => self.compose,
            "verify" => self.verify,
            "cleanup" => 1,
            "appCompletion" => 4,
            _ => anyhow::bail!("unknown publish work stage"),
        })
    }
}

/// No timer revokes a live device call. Drop releases exactly this token; startup
/// recovery clears process claims only after the old process has ended.
pub struct PublishWorkPermit {
    connection: std::sync::Arc<parking_lot::Mutex<Connection>>,
    token: String,
}
impl Drop for PublishWorkPermit {
    fn drop(&mut self) {
        let result = (|| -> anyhow::Result<()> {
            let conn = self.connection.lock();
            conn.execute(
                "DELETE FROM publish_work_claims WHERE token=?1",
                [&self.token],
            )?;
            Ok(())
        })();
        if let Err(error) = result {
            tracing::error!("publish permit release: {error:#}");
        }
    }
}

#[derive(Debug, Clone)]
pub struct PublishDispatchJob {
    pub assignment_id: String,
    pub run: PublishPipelineRun,
    pub attempt_id: String,
    pub udid: String,
    pub phase: String,
    pub revision: i64,
}

impl Database {
    /// Explicit operator retry of one pre-Send failure. Siblings are untouched;
    /// an intent, running parent, cancelled campaign or missed schedule cannot
    /// be reopened. The publication survives and only this attempt is new.
    pub fn claim_publish_assignment_retry(
        &self,
        assignment_id: &str,
    ) -> anyhow::Result<Option<PublishPipelineRun>> {
        self.claim_publish_assignment_retry_inner(assignment_id, None)
    }
    pub fn claim_publish_assignment_retry_checked(
        &self,
        assignment_id: &str,
        revision: i64,
        request_id: &str,
    ) -> anyhow::Result<Option<PublishPipelineRun>> {
        Uuid::parse_str(request_id)?;
        self.claim_publish_assignment_retry_inner(assignment_id, Some((revision, request_id)))
    }
    fn claim_publish_assignment_retry_inner(
        &self,
        assignment_id: &str,
        request: Option<(i64, &str)>,
    ) -> anyhow::Result<Option<PublishPipelineRun>> {
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some((revision, request_id)) = request {
            let prior:Option<(String,i64,String)>=tx.query_row("SELECT assignment_id,expected_revision,run_token FROM publish_retry_requests WHERE request_id=?1",[request_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?;
            if let Some((id, expected, token)) = prior {
                anyhow::ensure!(
                    id == assignment_id && expected == revision,
                    "Retry request identity changed"
                );
                let campaign_id = tx.query_row(
                    "SELECT campaign_id FROM publish_assignments WHERE id=?1",
                    [assignment_id],
                    |r| r.get(0),
                )?;
                return Ok(Some(PublishPipelineRun { campaign_id, token }));
            }
            let current: i64 = tx.query_row(
                "SELECT revision FROM publish_assignments WHERE id=?1",
                [assignment_id],
                |r| r.get(0),
            )?;
            anyhow::ensure!(
                current == revision,
                "PublishRetryConflict: bài đã thay đổi; tải lại tiến độ"
            );
        }
        let eligible: Option<(String, String, String)> = tx.query_row(
            "SELECT a.campaign_id,a.publication_id,a.udid FROM publish_assignments a
             JOIN publish_campaigns c ON c.id=a.campaign_id
             WHERE a.id=?1 AND a.state='failed_before_dispatch' AND a.effect_intent IS NULL
             AND (c.state IN ('failed_before_dispatch','verifying','uncertain') OR (?2=1 AND c.state='posting'))
             AND (?2=1 OR NOT EXISTS(SELECT 1 FROM publish_pipeline_runs r WHERE r.campaign_id=c.id))
             AND NOT EXISTS(SELECT 1 FROM publish_dispatch_jobs j WHERE j.assignment_id=a.id AND j.state IN ('queued','running','paused'))",
            params![assignment_id,request.is_some()], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?)),
        ).optional()?;
        let Some((campaign_id, publication_id, udid)) = eligible else {
            return Ok(None);
        };
        let now = Utc::now();
        let agent_ready_at = tx
            .query_row(
                "SELECT reason,finished_at_ms FROM publish_dispatch_jobs WHERE assignment_id=?1 AND state='finished' ORDER BY finished_at_ms DESC LIMIT 1",
                [assignment_id],
                |row| Ok((row.get::<_, Option<String>>(0)?, row.get::<_, Option<i64>>(1)?)),
            )
            .optional()?
            .and_then(|(reason, finished_at)| {
                let delay = crate::publish_recovery::RecoveryFailure::legacy(reason?)
                    .minimum_retry_delay()?;
                Some(finished_at.unwrap_or_else(|| now.timestamp_millis()) + delay.as_millis() as i64)
            })
            .filter(|at| *at > now.timestamp_millis());
        let token: Option<String> = tx
            .query_row(
                "SELECT token FROM publish_pipeline_runs WHERE campaign_id=?1",
                [&campaign_id],
                |r| r.get(0),
            )
            .optional()?;
        let run = PublishPipelineRun {
            campaign_id,
            token: token.unwrap_or_else(|| Uuid::new_v4().to_string()),
        };
        let attempt_id = Uuid::new_v4().to_string();
        tx.execute("UPDATE publish_campaigns SET state='posting',error_code=NULL,revision=revision+1,updated_at=?2 WHERE id=?1",
            params![run.campaign_id,now.to_rfc3339()])?;
        tx.execute("UPDATE publish_assignments SET state='queued',error_code=NULL,revision=revision+1,updated_at=?2 WHERE id=?1 AND effect_intent IS NULL",params![assignment_id,now.to_rfc3339()])?;
        tx.execute(
            "INSERT OR IGNORE INTO publish_pipeline_runs(campaign_id,token,created_at) VALUES(?1,?2,?3)",
            params![run.campaign_id, run.token, now.to_rfc3339()],
        )?;
        tx.execute("INSERT INTO publish_attempts(attempt_id,publication_id,campaign_id,started_at_ms) VALUES(?1,?2,?3,?4)",
            params![attempt_id,publication_id,run.campaign_id,now.timestamp_millis()])?;
        tx.execute("INSERT INTO publish_dispatch_jobs(assignment_id,campaign_id,run_token,attempt_id,udid,phase,state,queued_at_ms)
            VALUES(?1,?2,?3,?4,?5,'transfer','queued',?6)
            ON CONFLICT(assignment_id) DO UPDATE SET run_token=excluded.run_token,attempt_id=excluded.attempt_id,
            phase='transfer',state='queued',queued_at_ms=excluded.queued_at_ms,deadline_ms=NULL,
            started_at_ms=NULL,finished_at_ms=NULL,owner=NULL,reason='operator_retry_before_send',revision=publish_dispatch_jobs.revision+1",
            params![assignment_id,run.campaign_id,run.token,attempt_id,udid,now.timestamp_millis()])?;
        tx.execute("INSERT INTO publish_events(campaign_id,revision,kind,payload_json,created_at)
            SELECT id,revision,'state',json_object('state',state,'source','retry_failed_assignment','assignmentId',?2),?3
            FROM publish_campaigns WHERE id=?1", params![run.campaign_id,assignment_id,now.to_rfc3339()])?;
        let mut recovery: crate::publish_recovery::PublishRecoveryState = tx
            .query_row(
                "SELECT payload FROM publish_recovery_state WHERE assignment_id=?1",
                [assignment_id],
                |r| r.get::<_, String>(0),
            )
            .optional()?
            .map(|s| serde_json::from_str(&s))
            .transpose()?
            .unwrap_or_default();
        recovery.manual = true;
        let current_account: String = tx.query_row(
            "SELECT COALESCE((SELECT handle FROM device_meta WHERE udid=?1),'')",
            [&udid],
            |r| r.get(0),
        )?;
        if recovery.expected_account.is_empty() {
            recovery.expected_account = current_account.clone();
        }
        anyhow::ensure!(
            recovery.expected_account == current_account,
            "Tài khoản đã đổi; không thử lại bài cũ"
        );
        if let Some((_, request_id)) = request {
            if recovery.step == "sound"
                && recovery
                    .sound
                    .as_ref()
                    .is_some_and(|sound| !sound.confirmed)
            {
                let json: String = tx.query_row(
                    "SELECT request_json FROM publish_campaigns WHERE id=?1",
                    [&run.campaign_id],
                    |row| row.get(0),
                )?;
                let frozen: crate::publish::PublishCampaignRequest = serde_json::from_str(&json)?;
                if matches!(
                    frozen.sound_policy,
                    crate::publish::PublishSoundPolicy::Default
                        | crate::publish::PublishSoundPolicy::TrendingAny { .. }
                ) {
                    let prior = recovery.sound.take();
                    tx.execute("INSERT INTO operation_device_events(source_kind,source_id,udid,action,state,recorded_at,text,detail)
                        VALUES('publish',?1,?2,'publishSoundRebind','retrying',?3,?4,?5)",
                        params![run.campaign_id, udid, now.to_rfc3339(),
                            "Chọn lại nhạc tự chọn chưa xác minh trong lượt thử lại trước Post",
                            serde_json::to_string(&serde_json::json!({
                            "assignmentId": assignment_id, "requestId": request_id,
                            "source": "confirmed_operator_retry_before_post", "priorSound": prior,
                        }))?])?;
                }
            }
        }
        // The operator grants a new pre-Post attempt, including bounded retries
        // inside a step. `manual` still forbids requeueing the whole job.
        recovery.max_retries = 3;
        recovery.retries_used = 0;
        recovery.counts.clear();
        recovery.state = if agent_ready_at.is_some() {
            "retryWaiting"
        } else {
            "running"
        }
        .into();
        recovery.next_retry_at = agent_ready_at.map(|at| at as f64);
        recovery.reconnect_deadline = None;
        recovery.reconnect_retry = false;
        tx.execute("INSERT INTO publish_recovery_state VALUES(?1,?2,?3) ON CONFLICT(assignment_id) DO UPDATE SET run_token=excluded.run_token,payload=excluded.payload",params![assignment_id,run.token,serde_json::to_string(&recovery)?])?;
        if let Some((revision, id)) = request {
            tx.execute(
                "INSERT INTO publish_retry_requests VALUES(?1,?2,?3,?4)",
                params![id, assignment_id, revision, run.token],
            )?;
        }
        tx.commit()?;
        Ok(Some(run))
    }

    pub fn rotate_publish_devices(&self, devices: &[String]) -> anyhow::Result<()> {
        let connection = self.dispatch_conn()?;
        let mut conn = connection.lock();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        for udid in devices.iter().collect::<std::collections::BTreeSet<_>>() {
            Self::publish_dispatch_turn(&tx, udid)?;
        }
        tx.commit()?;
        Ok(())
    }
    pub fn pause_publish_dispatch(&self) -> anyhow::Result<()> {
        self.conn()?.execute("UPDATE publish_dispatch_jobs SET state='paused',reason='app_closing',revision=revision+1 WHERE state='queued'",[])?;
        Ok(())
    }
    pub fn publish_limits(&self) -> anyhow::Result<PublishLimits> {
        let connection = self.dispatch_conn()?;
        let conn = connection.lock();
        Self::publish_limits_from_connection(&conn)
    }

    fn publish_limits_from_connection(conn: &Connection) -> anyhow::Result<PublishLimits> {
        let raw: Option<String> = conn
            .query_row(
                "SELECT value FROM settings WHERE key='publish.dispatch.limits'",
                [],
                |r| r.get(0),
            )
            .optional()?;
        let limits: PublishLimits = raw
            .map(|raw| serde_json::from_str(&raw))
            .transpose()?
            .unwrap_or_default();
        limits.validate()?;
        Ok(limits)
    }

    pub fn try_publish_work(
        &self,
        udid: &str,
        stage: &str,
        owner: &str,
    ) -> anyhow::Result<Option<PublishWorkPermit>> {
        let connection = self.dispatch_conn()?;
        let mut conn = connection.lock();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        // A concurrent configuration reduction must commit before or after this
        // admission, never between its capacity snapshot and the claim insert.
        let limits = Self::publish_limits_from_connection(&tx)?;
        let cap = limits.stage(stage)? as i64;
        let (total, same, busy): (i64,i64,i64) = tx.query_row(
            "SELECT COUNT(*),COALESCE(SUM(stage=?1),0),COALESCE(SUM(udid=?2),0) FROM publish_work_claims",
            params![stage,udid], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?)))?;
        if total >= limits.device_total as i64 || same >= cap || busy > 0 {
            return Ok(None);
        }
        let token = Uuid::new_v4().to_string();
        tx.execute(
            "INSERT INTO publish_work_claims VALUES(?1,?2,?3,?4,?5)",
            params![token, udid, stage, owner, Utc::now().timestamp_millis()],
        )?;
        tx.commit()?;
        Ok(Some(PublishWorkPermit {
            connection: connection.clone(),
            token,
        }))
    }

    /// Called inside the campaign claim transaction: double clicks cannot enqueue twice.
    pub(super) fn enqueue_publish_jobs(
        conn: &Connection,
        run: &PublishPipelineRun,
        now: i64,
    ) -> anyhow::Result<()> {
        let raw: Option<String> = conn.query_row(
            "SELECT run_at FROM publish_campaigns WHERE id=?1",
            [&run.campaign_id],
            |r| r.get(0),
        )?;
        let deadline = raw
            .map(|v| {
                chrono::NaiveDateTime::parse_from_str(&v, "%Y-%m-%dT%H:%M:%S")
                    .or_else(|_| chrono::NaiveDateTime::parse_from_str(&v, "%Y-%m-%dT%H:%M"))
                    .map(|v| v.and_utc().timestamp_millis() + 30_000)
            })
            .transpose()?;
        let rows: Vec<(String,String,String,String)> = conn.prepare(
            "SELECT id,publication_id,udid,state FROM publish_assignments WHERE campaign_id=?1
             AND effect_intent IS NULL AND state IN ('queued','scheduled','ready','imported','failed_before_dispatch')")?
            .query_map([&run.campaign_id], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?)))?
            .collect::<Result<_,_>>()?;
        for (id, publication, udid, state) in rows {
            let attempt = Uuid::new_v4().to_string();
            conn.execute("INSERT INTO publish_attempts(attempt_id,publication_id,campaign_id,started_at_ms) VALUES(?1,?2,?3,?4)",
                params![attempt,publication,run.campaign_id,now])?;
            conn.execute("INSERT INTO publish_dispatch_jobs(assignment_id,campaign_id,run_token,attempt_id,udid,phase,state,queued_at_ms,deadline_ms)
                VALUES(?1,?2,?3,?4,?5,?6,'queued',?7,?8)
                ON CONFLICT(assignment_id) DO UPDATE SET run_token=excluded.run_token,attempt_id=excluded.attempt_id,
                phase=excluded.phase,state='queued',queued_at_ms=excluded.queued_at_ms,deadline_ms=excluded.deadline_ms,
                started_at_ms=NULL,finished_at_ms=NULL,owner=NULL,reason=NULL,revision=publish_dispatch_jobs.revision+1",
                params![id,run.campaign_id,run.token,attempt,udid,if state=="imported" {"compose"} else {"transfer"},now,deadline])?;
        }
        Ok(())
    }

    pub fn pending_publish_dispatch(
        &self,
        limit: usize,
    ) -> anyhow::Result<Vec<PublishDispatchJob>> {
        let connection = self.dispatch_conn()?;
        let conn = connection.lock();
        let mut statement = conn.prepare("SELECT j.assignment_id,j.campaign_id,j.run_token,j.attempt_id,j.udid,j.phase,j.revision
          FROM publish_dispatch_jobs j JOIN publish_pipeline_runs r ON r.campaign_id=j.campaign_id AND r.token=j.run_token
          JOIN publish_campaigns c ON c.id=j.campaign_id
          LEFT JOIN publish_dispatch_turns t ON t.udid=j.udid
          WHERE j.state='queued' AND c.state='posting'
          AND NOT EXISTS(SELECT 1 FROM publish_recovery_state retry WHERE retry.assignment_id=j.assignment_id AND CAST(json_extract(retry.payload,'$.nextRetryAt') AS REAL)>CAST(strftime('%s','now') AS INTEGER)*1000)
          AND ((j.phase='transfer' AND (SELECT COUNT(*) FROM publish_work_claims WHERE stage='transfer')<CAST(COALESCE((SELECT json_extract(value,'$.transfer') FROM settings WHERE key='publish.dispatch.limits'),4) AS INTEGER))
            OR (j.phase='compose' AND (SELECT COUNT(*) FROM publish_work_claims WHERE stage='compose')<CAST(COALESCE((SELECT json_extract(value,'$.compose') FROM settings WHERE key='publish.dispatch.limits'),4) AS INTEGER)))
          AND NOT EXISTS(SELECT 1 FROM publish_work_claims w WHERE w.udid=j.udid)
          AND NOT EXISTS(SELECT 1 FROM publish_dispatch_jobs busy WHERE busy.udid=j.udid
            AND busy.assignment_id<>j.assignment_id AND busy.state IN ('queued','running') AND busy.started_at_ms IS NOT NULL)
          ORDER BY COALESCE(t.last_turn,0),j.queued_at_ms,j.assignment_id LIMIT ?1")?;
        let rows = statement
            .query_map([limit.clamp(1, 256) as i64], |r| {
                Ok(PublishDispatchJob {
                    assignment_id: r.get(0)?,
                    run: PublishPipelineRun {
                        campaign_id: r.get(1)?,
                        token: r.get(2)?,
                    },
                    attempt_id: r.get(3)?,
                    udid: r.get(4)?,
                    phase: r.get(5)?,
                    revision: r.get(6)?,
                })
            })?
            .collect::<Result<_, _>>()?;
        Ok(rows)
    }

    pub fn defer_publish_dispatch(
        &self,
        job: &PublishDispatchJob,
        reason: &str,
    ) -> anyhow::Result<()> {
        let connection = self.dispatch_conn()?;
        let conn = connection.lock();
        conn.execute("UPDATE publish_dispatch_jobs SET reason=?3 WHERE assignment_id=?1 AND revision=?2 AND state='queued'",params![job.assignment_id,job.revision,reason])?;
        Self::publish_dispatch_turn(&conn, &job.udid)?;
        Ok(())
    }
    fn publish_dispatch_turn(conn: &Connection, udid: &str) -> anyhow::Result<()> {
        conn.execute("INSERT INTO publish_dispatch_turns(udid,last_turn) VALUES(?1,(SELECT COALESCE(MAX(last_turn),0)+1 FROM publish_dispatch_turns))
          ON CONFLICT(udid) DO UPDATE SET last_turn=excluded.last_turn",[udid])?;
        Ok(())
    }

    pub fn claim_publish_dispatch(
        &self,
        job: &PublishDispatchJob,
        local_now_ms: i64,
    ) -> anyhow::Result<bool> {
        let connection = self.dispatch_conn()?;
        let mut conn = connection.lock();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let changed = tx.execute("UPDATE publish_dispatch_jobs SET state='running',owner=?3,reason=NULL,
            started_at_ms=COALESCE(started_at_ms,?4),revision=revision+1
            WHERE assignment_id=?1 AND revision=?2 AND state='queued'
            AND (started_at_ms IS NOT NULL OR deadline_ms IS NULL OR deadline_ms>=?5)
            AND NOT EXISTS(SELECT 1 FROM publish_dispatch_jobs busy
                WHERE busy.udid=publish_dispatch_jobs.udid AND busy.assignment_id<>publish_dispatch_jobs.assignment_id
                AND busy.state IN ('queued','running','paused') AND busy.started_at_ms IS NOT NULL)
            AND EXISTS(SELECT 1 FROM publish_pipeline_runs r JOIN publish_campaigns c ON c.id=r.campaign_id
                WHERE r.campaign_id=publish_dispatch_jobs.campaign_id AND r.token=publish_dispatch_jobs.run_token AND c.state='posting')",
            params![job.assignment_id,job.revision,job.attempt_id,Utc::now().timestamp_millis(),local_now_ms])?;
        if changed == 1 {
            Self::publish_dispatch_turn(&tx, &job.udid)?;
        }
        tx.commit()?;
        Ok(changed == 1)
    }

    pub fn finish_publish_dispatch(
        &self,
        job: &PublishDispatchJob,
        error: Option<&str>,
    ) -> anyhow::Result<bool> {
        if let Some(error) = error {
            if self.requeue_publish_recovery(job, error)? {
                return Ok(true);
            }
        }
        let connection = self.dispatch_conn()?;
        let mut conn = connection.lock();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let to_compose = error.is_none() && job.phase == "transfer";
        let changed = tx.execute("UPDATE publish_dispatch_jobs SET state=?4,phase=?5,owner=NULL,reason=?6,
            finished_at_ms=?7,revision=revision+1 WHERE assignment_id=?1 AND attempt_id=?2 AND revision=?3 AND state='running'",
            params![job.assignment_id,job.attempt_id,job.revision+1,if to_compose {"queued"} else {"finished"},
                if to_compose {"compose"} else {&job.phase},error,if to_compose {None} else {Some(Utc::now().timestamp_millis())}])?;
        if changed == 1 && !to_compose {
            tx.execute("UPDATE publish_recovery_state SET payload=json_set(payload,'$.state',CASE WHEN json_extract(payload,'$.state') IN ('exhausted','stopped') THEN json_extract(payload,'$.state') ELSE ?2 END,'$.nextRetryAt',NULL,'$.reconnectDeadline',NULL) WHERE assignment_id=?1",params![job.assignment_id,if error.is_some(){"failed"}else{"finished"}])?;
            if let Some(error) = error {
                tx.execute("UPDATE publish_assignments SET state='failed_before_dispatch',error_code=?2,revision=revision+1,updated_at=?3
                    WHERE id=?1 AND effect_intent IS NULL AND state IN ('queued','scheduled','ready','transferring','imported')",
                    params![job.assignment_id,error,Utc::now().to_rfc3339()])?;
            }
            tx.execute("UPDATE publish_attempts SET finished_at_ms=?2,result=?3,evidence_json=(SELECT evidence_json FROM publish_assignments WHERE id=?4) WHERE attempt_id=?1",
                params![job.attempt_id,Utc::now().timestamp_millis(),error.unwrap_or("submitted_or_settled"),job.assignment_id])?;
        }
        if changed == 1 && to_compose {
            tx.execute("UPDATE publish_recovery_state SET payload=json_set(payload,'$.checkpoint','mediaImported','$.state','running','$.nextRetryAt',NULL) WHERE assignment_id=?1",[&job.assignment_id])?;
        }
        tx.commit()?;
        Ok(changed == 1)
    }

    /// Expiry affects only jobs that never obtained a stage. An already submitted
    /// publication remains eligible for verification regardless of its schedule.
    pub fn expire_publish_dispatch(&self, local_now_ms: i64) -> anyhow::Result<()> {
        self.expire_publish_dispatch_scope(local_now_ms, None)
    }

    /// Same production deadline settlement for a frozen acceptance campaign;
    /// unrelated persisted schedules are left untouched.
    pub fn expire_publish_dispatch_for_campaign(
        &self,
        local_now_ms: i64,
        campaign: &str,
    ) -> anyhow::Result<()> {
        self.expire_publish_dispatch_scope(local_now_ms, Some(campaign))
    }

    fn expire_publish_dispatch_scope(
        &self,
        local_now_ms: i64,
        campaign: Option<&str>,
    ) -> anyhow::Result<()> {
        let connection = self.dispatch_conn()?;
        let mut conn = connection.lock();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute("UPDATE publish_dispatch_jobs SET state='missed',reason='schedule_capacity_deadline',revision=revision+1
            WHERE state='queued' AND started_at_ms IS NULL AND deadline_ms<?1 AND (?2 IS NULL OR campaign_id=?2)",params![local_now_ms,campaign])?;
        tx.execute("UPDATE publish_dispatch_jobs SET state='cancelled',reason='campaign_cancelled',revision=revision+1
            WHERE state='queued' AND (?1 IS NULL OR campaign_id=?1) AND EXISTS(SELECT 1 FROM publish_campaigns c WHERE c.id=campaign_id AND c.state='cancelled')",[campaign])?;
        tx.execute("UPDATE publish_assignments SET state='missed',error_code='schedule_capacity_deadline',revision=revision+1
            WHERE effect_intent IS NULL AND (?1 IS NULL OR campaign_id=?1) AND id IN(SELECT assignment_id FROM publish_dispatch_jobs WHERE state='missed') AND state<>'missed'",[campaign])?;
        tx.execute("UPDATE publish_attempts SET finished_at_ms=?1,result=(SELECT reason FROM publish_dispatch_jobs j WHERE j.attempt_id=publish_attempts.attempt_id)
            WHERE finished_at_ms IS NULL AND attempt_id IN(SELECT attempt_id FROM publish_dispatch_jobs WHERE state IN ('missed','cancelled') AND (?2 IS NULL OR campaign_id=?2))",params![Utc::now().timestamp_millis(),campaign])?;
        tx.commit()?;
        Ok(())
    }

    pub fn finished_publish_dispatch_runs(&self) -> anyhow::Result<Vec<PublishPipelineRun>> {
        let connection = self.dispatch_conn()?;
        let conn = connection.lock();
        let mut s = conn.prepare("SELECT r.campaign_id,r.token FROM publish_pipeline_runs r
            WHERE EXISTS(SELECT 1 FROM publish_dispatch_jobs j WHERE j.campaign_id=r.campaign_id AND j.run_token=r.token)
            AND NOT EXISTS(SELECT 1 FROM publish_dispatch_jobs j WHERE j.campaign_id=r.campaign_id AND j.run_token=r.token AND j.state IN ('queued','running','paused')) LIMIT 256")?;
        let rows = s
            .query_map([], |r| {
                Ok(PublishPipelineRun {
                    campaign_id: r.get(0)?,
                    token: r.get(1)?,
                })
            })?
            .collect::<Result<_, _>>()?;
        Ok(rows)
    }
}
