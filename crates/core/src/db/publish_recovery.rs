use super::*;
use crate::publish_recovery::{FailureKind, PublishRecoveryState, RecoveryFailure};
use std::time::Duration;

fn read(conn: &Connection, id: &str) -> anyhow::Result<Option<(String, PublishRecoveryState)>> {
    let raw: Option<(String, String)> = conn
        .query_row(
            "SELECT run_token,payload FROM publish_recovery_state WHERE assignment_id=?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    raw.map(|(token, raw)| Ok((token, serde_json::from_str(&raw)?)))
        .transpose()
}
fn write(conn: &Connection, id: &str, token: &str, s: &PublishRecoveryState) -> anyhow::Result<()> {
    let previous = read(conn, id)?.map(|(_, s)| s);
    conn.execute("INSERT INTO publish_recovery_state VALUES(?1,?2,?3) ON CONFLICT(assignment_id) DO UPDATE SET run_token=excluded.run_token,payload=excluded.payload",params![id,token,serde_json::to_string(s)?])?;
    if previous
        .as_ref()
        .is_none_or(|p| p.state != s.state || p.retries_used != s.retries_used || p.step != s.step)
    {
        let text = match s.state.as_str() {
            "waitingDevice" => "Mất kết nối; chờ đúng máy quay lại tối đa 2 phút".into(),
            "retryWaiting" => format!("Thử lại {}/{} · {}", s.retries_used, s.max_retries, s.step),
            "exhausted" => {
                "Đã hết lượt tự thử; kiểm tra lỗi và bấm Thử lại để chạy thêm một lần".into()
            }
            _ => format!("Tiếp tục bước {}", s.step),
        };
        conn.execute("INSERT INTO operation_device_events(source_kind,source_id,udid,action,state,recorded_at,text,detail) SELECT 'publish',campaign_id,udid,'publishRecovery',?2,?3,?4,?5 FROM publish_assignments WHERE id=?1",params![id,s.state,Utc::now().to_rfc3339(),text,s.last_error])?;
    }
    Ok(())
}
fn current(conn: &Connection, id: &str, token: &str) -> anyhow::Result<bool> {
    Ok(conn.query_row("SELECT EXISTS(SELECT 1 FROM publish_assignments a JOIN publish_pipeline_runs r ON r.campaign_id=a.campaign_id JOIN publish_campaigns c ON c.id=a.campaign_id WHERE a.id=?1 AND r.token=?2 AND c.state='posting' AND a.effect_intent IS NULL AND a.state NOT IN ('cancelled','missed','uncertain','succeeded','verifying'))",params![id,token],|r|r.get(0))?)
}
fn record_failure(s: &mut PublishRecoveryState, failure: &RecoveryFailure) {
    s.last_error = Some(failure.message.clone());
    s.last_error_code = Some(failure.code.clone());
    s.last_error_kind = Some(failure.kind.as_str().into());
}

fn spend(s: &mut PublishRecoveryState, failure: &RecoveryFailure, now: i64) -> Option<Duration> {
    record_failure(s, failure);
    let used = *s.counts.get(&s.step).unwrap_or(&0);
    if used >= s.max_retries {
        s.state = "exhausted".into();
        s.next_retry_at = None;
        return None;
    }
    s.retries_used = used + 1;
    s.counts.insert(s.step.clone(), used + 1);
    let delay = Duration::from_secs([2, 5, 10][used.min(2) as usize])
        .max(failure.minimum_retry_delay().unwrap_or_default());
    s.state = "retryWaiting".into();
    s.next_retry_at = Some((now + delay.as_millis() as i64) as f64);
    Some(delay)
}
impl Database {
    pub fn verify_publish_recovery_account(&self, id: &str, observed: &str) -> anyhow::Result<()> {
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let Some((token, mut s)) = read(&tx, id)? else {
            return Ok(());
        };
        anyhow::ensure!(current(&tx, id, &token)?, "Lượt đăng đã dừng");
        let norm = |s: &str| s.trim().trim_start_matches('@').to_lowercase();
        let assigned:String=tx.query_row("SELECT COALESCE((SELECT handle FROM device_meta WHERE udid=a.udid),'') FROM publish_assignments a WHERE id=?1",[id],|r|r.get(0))?;
        anyhow::ensure!(
            assigned == s.expected_account,
            "Tài khoản đã đổi; không tiếp tục bài cũ"
        );
        let expected = s.observed_account.as_deref().unwrap_or(&s.expected_account);
        anyhow::ensure!(
            expected.is_empty() || norm(expected) == norm(observed),
            "Tài khoản đã đổi trên điện thoại; không tiếp tục bài cũ"
        );
        anyhow::ensure!(!observed.trim().is_empty(), "Chưa đọc được tài khoản");
        s.observed_account = Some(observed.into());
        write(&tx, id, &token, &s)?;
        tx.commit()?;
        Ok(())
    }
    pub fn fail_publish_queued_device(
        &self,
        job: &PublishDispatchJob,
        reason: &str,
    ) -> anyhow::Result<()> {
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        if !current(&tx, &job.assignment_id, &job.run.token)? {
            return Ok(());
        }
        let changed=tx.execute("UPDATE publish_dispatch_jobs SET state='finished',reason=?3,finished_at_ms=?4,revision=revision+1 WHERE assignment_id=?1 AND revision=?2 AND state='queued'",params![job.assignment_id,job.revision,reason,Utc::now().timestamp_millis()])?;
        if changed == 1 {
            tx.execute("UPDATE publish_assignments SET state='failed_before_dispatch',error_code=?2,revision=revision+1 WHERE id=?1 AND effect_intent IS NULL",params![job.assignment_id,reason])?;
            let (_, mut s) = read(&tx, &job.assignment_id)?.context("recovery state missing")?;
            s.state = "failed".into();
            s.last_error = Some(reason.into());
            s.next_retry_at = None;
            s.reconnect_deadline = None;
            write(&tx, &job.assignment_id, &job.run.token, &s)?;
        }
        tx.commit()?;
        Ok(())
    }
    pub fn acknowledged_publish_retry(
        &self,
        id: &str,
        revision: i64,
        request: &str,
    ) -> anyhow::Result<bool> {
        let conn = self.conn()?;
        let found:Option<(String,i64)>=conn.query_row("SELECT assignment_id,expected_revision FROM publish_retry_requests WHERE request_id=?1",[request],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
        if let Some((old, rev)) = found {
            anyhow::ensure!(
                old == id && rev == revision,
                "Retry request identity changed"
            );
            return Ok(true);
        };
        Ok(false)
    }
    pub fn publish_recovery_state(&self, id: &str) -> anyhow::Result<Option<PublishRecoveryState>> {
        Ok(read(&self.conn()?, id)?.map(|(_, s)| s))
    }
    pub fn init_publish_recovery(&self, id: &str, token: &str) -> anyhow::Result<()> {
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        anyhow::ensure!(
            current(&tx, id, token)?,
            "Lượt đăng đã dừng hoặc đã có Post intent"
        );
        match read(&tx, id)? {
            Some((old, _)) if old == token => {}
            Some((_, mut s)) => {
                s.state = "running".into();
                s.next_retry_at = None;
                write(&tx, id, token, &s)?;
            }
            None => {
                let expected:String=tx.query_row("SELECT COALESCE((SELECT handle FROM device_meta WHERE udid=a.udid),'') FROM publish_assignments a WHERE id=?1",[id],|r|r.get(0))?;
                write(
                    &tx,
                    id,
                    token,
                    &PublishRecoveryState {
                        expected_account: expected,
                        ..Default::default()
                    },
                )?;
            }
        };
        tx.commit()?;
        Ok(())
    }
    pub fn update_publish_recovery_step(
        &self,
        id: &str,
        token: &str,
        step: &str,
        checkpoint: Option<&str>,
    ) -> anyhow::Result<()> {
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        anyhow::ensure!(
            current(&tx, id, token)?,
            "Lượt đăng đã dừng hoặc đã có Post intent"
        );
        let (_, mut s) = read(&tx, id)?.context("recovery state missing")?;
        if !step.is_empty() {
            s.step = step.into();
        }
        s.retries_used = *s.counts.get(&s.step).unwrap_or(&0);
        s.state = "running".into();
        s.next_retry_at = None;
        if let Some(c) = checkpoint {
            s.checkpoint = c.into()
        };
        write(&tx, id, token, &s)?;
        tx.commit()?;
        Ok(())
    }
    pub fn reserve_publish_step_retry(
        &self,
        id: &str,
        token: &str,
        error: &str,
    ) -> anyhow::Result<Option<Duration>> {
        self.reserve_publish_step_retry_failure(id, token, &RecoveryFailure::legacy(error))
    }

    pub fn reserve_publish_step_retry_failure(
        &self,
        id: &str,
        token: &str,
        failure: &RecoveryFailure,
    ) -> anyhow::Result<Option<Duration>> {
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        anyhow::ensure!(
            current(&tx, id, token)?,
            "Lượt đăng đã dừng hoặc đã có Post intent"
        );
        let (_, mut s) = read(&tx, id)?.context("recovery state missing")?;
        anyhow::ensure!(
            failure.kind == FailureKind::Retryable,
            "Only an explicitly retryable failure may spend the automatic retry budget"
        );
        let delay = spend(&mut s, failure, Utc::now().timestamp_millis());
        write(&tx, id, token, &s)?;
        tx.commit()?;
        Ok(delay)
    }
    pub fn bind_publish_recovery_sound(
        &self,
        id: &str,
        token: &str,
        selection: Option<&crate::SoundSelectionEvidence>,
    ) -> anyhow::Result<Option<crate::SoundSelectionEvidence>> {
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        anyhow::ensure!(
            current(&tx, id, token)?,
            "Lượt đăng đã dừng hoặc đã có Post intent"
        );
        let (_, mut s) = read(&tx, id)?.context("recovery state missing")?;
        if s.sound.is_none() {
            s.sound = selection.cloned();
            write(&tx, id, token, &s)?;
        };
        tx.commit()?;
        Ok(s.sound)
    }
    /// Called only after the previous worker has returned and released its permit.
    pub fn requeue_publish_recovery(
        &self,
        job: &PublishDispatchJob,
        error: &str,
    ) -> anyhow::Result<bool> {
        let failure = RecoveryFailure::legacy(error);
        let kind = failure.kind;
        if kind == FailureKind::Terminal {
            return Ok(false);
        }
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        if !current(&tx, &job.assignment_id, &job.run.token)? {
            return Ok(false);
        }
        let valid:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM publish_dispatch_jobs WHERE assignment_id=?1 AND attempt_id=?2 AND revision=?3 AND state='running')",params![job.assignment_id,job.attempt_id,job.revision+1],|r|r.get(0))?;
        if !valid {
            return Ok(false);
        }
        let mut s = read(&tx, &job.assignment_id)?
            .map(|(_, s)| s)
            .unwrap_or_else(|| PublishRecoveryState {
                step: job.phase.clone(),
                ..Default::default()
            });
        if s.manual {
            return Ok(false);
        }
        let now = Utc::now().timestamp_millis();
        record_failure(&mut s, &failure);
        if kind == FailureKind::Disconnected {
            if *s.counts.get(&s.step).unwrap_or(&0) >= s.max_retries {
                s.state = "exhausted".into();
                s.next_retry_at = None;
                write(&tx, &job.assignment_id, &job.run.token, &s)?;
                tx.commit()?;
                return Ok(false);
            }
            s.reconnect_retry = true;
            s.reconnect_deadline.get_or_insert((now + 120_000) as f64);
            s.state = "waitingDevice".into();
            s.next_retry_at = Some((now + 2000) as f64);
        } else if spend(&mut s, &failure, now).is_none() {
            write(&tx, &job.assignment_id, &job.run.token, &s)?;
            tx.commit()?;
            return Ok(false);
        }
        write(&tx, &job.assignment_id, &job.run.token, &s)?;
        // Composer cleanup may have invalidated imported assets. Re-enter transfer
        // for revalidation using the existing import identity, never a new publication.
        let attempt = Uuid::new_v4().to_string();
        tx.execute(
            "UPDATE publish_attempts SET finished_at_ms=?2,result=?3 WHERE attempt_id=?1",
            params![job.attempt_id, now, error],
        )?;
        tx.execute("INSERT INTO publish_attempts(attempt_id,publication_id,campaign_id,started_at_ms) SELECT ?2,publication_id,campaign_id,?3 FROM publish_assignments WHERE id=?1",params![job.assignment_id,attempt,now])?;
        tx.execute("UPDATE publish_dispatch_jobs SET state='queued',phase='transfer',attempt_id=?2,owner=NULL,reason='recovery_pending',revision=revision+1 WHERE assignment_id=?1",params![job.assignment_id,attempt])?;
        tx.execute("UPDATE publish_assignments SET state='queued',error_code=NULL,revision=revision+1,updated_at=?2 WHERE id=?1 AND effect_intent IS NULL",params![job.assignment_id,Utc::now().to_rfc3339()])?;
        tx.commit()?;
        Ok(true)
    }
    /// A queue observation spends no attempt while the exact device is absent.
    pub fn admit_publish_reconnect(
        &self,
        job: &PublishDispatchJob,
        online: bool,
        now: i64,
    ) -> anyhow::Result<bool> {
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        if !current(&tx, &job.assignment_id, &job.run.token)? {
            return Ok(false);
        }
        let Some((token, mut s)) = read(&tx, &job.assignment_id)? else {
            return Ok(online);
        };
        if token != job.run.token {
            return Ok(false);
        }
        let account:String=tx.query_row("SELECT COALESCE((SELECT handle FROM device_meta WHERE udid=a.udid),'') FROM publish_assignments a WHERE id=?1",[&job.assignment_id],|r|r.get(0))?;
        if account != s.expected_account {
            s.state = "failed".into();
            s.last_error = Some("Tài khoản đã đổi; chưa tiếp tục bài cũ".into());
            s.next_retry_at = None;
            tx.execute("UPDATE publish_dispatch_jobs SET state='finished',reason='retry_account_changed',revision=revision+1 WHERE assignment_id=?1 AND state='queued'",[&job.assignment_id])?;
            tx.execute("UPDATE publish_assignments SET state='failed_before_dispatch',error_code='retry_account_changed',revision=revision+1 WHERE id=?1 AND effect_intent IS NULL",[&job.assignment_id])?;
            write(&tx, &job.assignment_id, &token, &s)?;
            tx.commit()?;
            return Ok(false);
        }
        if s.next_retry_at.is_some_and(|t| t > now as f64) {
            return Ok(false);
        }
        if !online || s.state == "waitingDevice" {
            let deadline = *s.reconnect_deadline.get_or_insert((now + 120_000) as f64);
            // This due job has begun its bounded device-readiness stage. The
            // schedule's initial capacity window must not truncate reconnect.
            tx.execute("UPDATE publish_dispatch_jobs SET started_at_ms=COALESCE(started_at_ms,?2) WHERE assignment_id=?1 AND state='queued'",params![job.assignment_id,now])?;
            if now as f64 >= deadline {
                s.state = "exhausted".into();
                s.last_error =
                    Some("Mất kết nối quá 2 phút; kết nối lại đúng máy rồi bấm Thử lại".into());
                s.next_retry_at = None;
                tx.execute("UPDATE publish_dispatch_jobs SET state='finished',reason='device_reconnect_timeout',finished_at_ms=?2,revision=revision+1 WHERE assignment_id=?1 AND state='queued'",params![job.assignment_id,now])?;
                tx.execute("UPDATE publish_assignments SET state='failed_before_dispatch',error_code='device_reconnect_timeout',revision=revision+1 WHERE id=?1 AND effect_intent IS NULL",[&job.assignment_id])?;
            } else if online {
                if s.reconnect_retry {
                    let step = s.step.clone();
                    let n = *s.counts.get(&step).unwrap_or(&0) + 1;
                    s.counts.insert(step, n);
                    s.retries_used = n;
                    s.reconnect_retry = false;
                }
                s.state = "running".into();
                s.next_retry_at = None;
                s.reconnect_deadline = None;
            } else {
                s.state = "waitingDevice".into();
                s.next_retry_at = Some((now + 2000) as f64);
            }
            write(&tx, &job.assignment_id, &token, &s)?;
            tx.commit()?;
            return Ok(online && (now as f64) < deadline);
        };
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::super::publish_pipeline::tests::fixture;
    use super::*;
    struct TestJournal {
        db: std::sync::Arc<Database>,
        id: String,
        token: String,
    }
    impl crate::publish_recovery::RecoveryJournal for TestJournal {
        fn step(&self, step: &str, c: Option<&str>) -> anyhow::Result<()> {
            self.db
                .update_publish_recovery_step(&self.id, &self.token, step, c)
        }
        fn retry(
            &self,
            failure: &crate::publish_recovery::RecoveryFailure,
        ) -> anyhow::Result<Option<Duration>> {
            self.db
                .reserve_publish_step_retry_failure(&self.id, &self.token, failure)
        }
        fn sound(
            &self,
            s: Option<&crate::SoundSelectionEvidence>,
        ) -> anyhow::Result<Option<crate::SoundSelectionEvidence>> {
            self.db
                .bind_publish_recovery_sound(&self.id, &self.token, s)
        }
    }
    #[tokio::test(start_paused = true)]
    async fn actual_retry_loop_stops_after_four_attempts_and_cancel_prevents_the_next_one() {
        let (db, _, campaign, a) = fixture();
        let run = db.claim_publish_pipeline(&campaign).unwrap().unwrap();
        let db = std::sync::Arc::new(db);
        let id = a[0].id.clone();
        db.init_publish_recovery(&id, &run.token).unwrap();
        let journal = std::sync::Arc::new(TestJournal {
            db: db.clone(),
            id: id.clone(),
            token: run.token.clone(),
        });
        let stop = std::sync::atomic::AtomicBool::new(false);
        let attempts = crate::publish_recovery::scope(journal.clone(), async {
            crate::publish_recovery::step("sound", Some("mediaSelected")).unwrap();
            let mut attempts = 0;
            loop {
                attempts += 1;
                if !crate::publish_recovery::retry(&anyhow::anyhow!("network unavailable"), &stop)
                    .await
                    .unwrap()
                {
                    break;
                }
            }
            attempts
        })
        .await;
        assert_eq!(attempts, 4);
        stop.store(true, std::sync::atomic::Ordering::Release);
        crate::publish_recovery::scope(journal, async {
            crate::publish_recovery::step("caption", None).unwrap();
            assert!(
                !crate::publish_recovery::retry(&anyhow::anyhow!("timeout"), &stop)
                    .await
                    .unwrap()
            );
        })
        .await;
        assert!(!db
            .publish_recovery_state(&id)
            .unwrap()
            .unwrap()
            .counts
            .contains_key("caption"));
    }

    #[tokio::test]
    async fn stored_sound_reads_the_durable_binding_without_replacing_it() {
        let (db, _, campaign, assignments) = fixture();
        let run = db.claim_publish_pipeline(&campaign).unwrap().unwrap();
        let db = std::sync::Arc::new(db);
        let id = assignments[0].id.clone();
        db.init_publish_recovery(&id, &run.token).unwrap();
        let journal = std::sync::Arc::new(TestJournal {
            db: db.clone(),
            id: id.clone(),
            token: run.token.clone(),
        });
        let expected = crate::SoundSelectionEvidence {
            section: crate::publish::SoundSectionKind::Trending,
            title: "Vibin".into(),
            artist: "Wxoda".into(),
            index: 0,
            candidates_digest: "fixture-digest".into(),
            confirmed: false,
        };
        db.bind_publish_recovery_sound(&id, &run.token, Some(&expected))
            .unwrap();

        let observed = crate::publish_recovery::scope(journal, async {
            crate::publish_recovery::stored_sound().unwrap()
        })
        .await;

        assert_eq!(observed, Some(expected.clone()));
        assert_eq!(
            db.publish_recovery_state(&id).unwrap().unwrap().sound,
            Some(expected)
        );
    }
    #[test]
    fn restart_and_stop_keep_retry_counters_and_never_replay_interrupted_work() {
        let (db, _, campaign, a) = fixture();
        let run = db.claim_publish_pipeline(&campaign).unwrap().unwrap();
        let id = &a[0].id;
        db.init_publish_recovery(id, &run.token).unwrap();
        db.update_publish_recovery_step(id, &run.token, "sound", Some("mediaSelected"))
            .unwrap();
        db.reserve_publish_step_retry(id, &run.token, "network")
            .unwrap();
        db.interrupt_orphaned_publish_campaigns().unwrap();
        assert!(db
            .pending_publish_dispatch(20)
            .unwrap()
            .iter()
            .all(|j| j.assignment_id != *id));
        let s = db.publish_recovery_state(id).unwrap().unwrap();
        assert_eq!(s.state, "interrupted");
        assert_eq!(s.counts["sound"], 1);
        assert!(db
            .reserve_publish_step_retry(id, &run.token, "network")
            .is_err());
    }
    #[test]
    fn changed_account_and_stop_refuse_before_another_device_action() {
        let (db, _, campaign, _) = fixture();
        let run = db.claim_publish_pipeline(&campaign).unwrap().unwrap();
        let job = db.pending_publish_dispatch(10).unwrap().remove(0);
        db.init_publish_recovery(&job.assignment_id, &run.token)
            .unwrap();
        db.conn()
            .unwrap()
            .execute(
                "INSERT INTO device_meta(udid,handle) VALUES(?1,'different.account')",
                [&job.udid],
            )
            .unwrap();
        assert!(!db.admit_publish_reconnect(&job, true, 1000).unwrap());
        assert_eq!(
            db.publish_recovery_state(&job.assignment_id)
                .unwrap()
                .unwrap()
                .state,
            "failed"
        );
        db.begin_publish_operation_stop(&campaign).unwrap();
        assert!(db
            .reserve_publish_step_retry(&job.assignment_id, &run.token, "timeout")
            .is_err());
    }
    #[test]
    fn retries_rotate_attempt_identity_and_old_worker_cannot_settle_new_attempt() {
        let (db, _, campaign, _) = fixture();
        let run = db.claim_publish_pipeline(&campaign).unwrap().unwrap();
        let job = db.pending_publish_dispatch(10).unwrap().remove(0);
        db.init_publish_recovery(&job.assignment_id, &run.token)
            .unwrap();
        db.claim_publish_dispatch(&job, 0).unwrap();
        assert!(db
            .finish_publish_dispatch(&job, Some("connection reset"))
            .unwrap());
        assert!(!db
            .finish_publish_dispatch(&job, Some("connection reset"))
            .unwrap());
        let conn = db.conn().unwrap();
        let (attempt, state): (String, String) = conn
            .query_row(
                "SELECT attempt_id,state FROM publish_dispatch_jobs WHERE assignment_id=?1",
                [&job.assignment_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_ne!(attempt, job.attempt_id);
        assert_eq!(state, "queued");
        drop(conn);
        assert_eq!(
            db.publish_recovery_state(&job.assignment_id)
                .unwrap()
                .unwrap()
                .retries_used,
            1
        );
    }
    #[test]
    fn blind_session_requeues_only_before_post_and_after_cooldown() {
        let (db, _, campaign, _) = fixture();
        let run = db.claim_publish_pipeline(&campaign).unwrap().unwrap();
        let transfer = db.pending_publish_dispatch(10).unwrap().remove(0);
        db.init_publish_recovery(&transfer.assignment_id, &run.token)
            .unwrap();
        assert!(db.claim_publish_dispatch(&transfer, 0).unwrap());
        assert!(db.finish_publish_dispatch(&transfer, None).unwrap());
        let compose = db
            .pending_publish_dispatch(10)
            .unwrap()
            .into_iter()
            .find(|job| job.assignment_id == transfer.assignment_id)
            .unwrap();
        db.update_publish_recovery_step(
            &compose.assignment_id,
            &run.token,
            "device",
            Some("mediaImported"),
        )
        .unwrap();
        assert!(db.claim_publish_dispatch(&compose, 0).unwrap());
        let error = "the agent on 9889db374744474635 is listening but cannot read the accessibility tree, and its instrumentation was already restarted 52s ago without fixing it. Not restarting again for another 12s.";
        let before = Utc::now().timestamp_millis();
        assert!(db.finish_publish_dispatch(&compose, Some(error)).unwrap());
        let recovery = db
            .publish_recovery_state(&compose.assignment_id)
            .unwrap()
            .unwrap();
        assert_eq!(recovery.state, "retryWaiting");
        assert_eq!(recovery.retries_used, 1);
        assert!(recovery.next_retry_at.unwrap() >= (before + 13_000) as f64);
        let assignment = db
            .get_publish_assignment_detail(&campaign, &compose.assignment_id)
            .unwrap()
            .unwrap()
            .assignments
            .remove(0);
        assert!(assignment.effect_intent.is_none());
        assert!(!db
            .pending_publish_dispatch(10)
            .unwrap()
            .iter()
            .any(|job| job.assignment_id == compose.assignment_id));
    }
    #[test]
    fn manual_retry_queues_one_attempt_after_agent_cooldown() {
        let (db, _, campaign, _) = fixture();
        let run = db.claim_publish_pipeline(&campaign).unwrap().unwrap();
        let transfer = db.pending_publish_dispatch(10).unwrap().remove(0);
        db.init_publish_recovery(&transfer.assignment_id, &run.token)
            .unwrap();
        assert!(db.claim_publish_dispatch(&transfer, 0).unwrap());
        assert!(db.finish_publish_dispatch(&transfer, None).unwrap());
        let compose = db
            .pending_publish_dispatch(10)
            .unwrap()
            .into_iter()
            .find(|job| job.assignment_id == transfer.assignment_id)
            .unwrap();
        assert!(db.claim_publish_dispatch(&compose, 0).unwrap());
        let mut recovery = db
            .publish_recovery_state(&compose.assignment_id)
            .unwrap()
            .unwrap();
        recovery.manual = true;
        recovery.max_retries = 0;
        db.conn()
            .unwrap()
            .execute(
                "UPDATE publish_recovery_state SET payload=?2 WHERE assignment_id=?1",
                params![
                    compose.assignment_id,
                    serde_json::to_string(&recovery).unwrap()
                ],
            )
            .unwrap();
        let error = "openControlSession: the agent on 9889db374744474635 did not answer /status within 10 seconds";
        assert!(db.finish_publish_dispatch(&compose, Some(error)).unwrap());
        let revision = db
            .publish_assignment_revision(&compose.assignment_id)
            .unwrap();
        let request = Uuid::new_v4().to_string();
        let before = Utc::now().timestamp_millis();
        assert!(db
            .claim_publish_assignment_retry_checked(&compose.assignment_id, revision, &request)
            .unwrap()
            .is_some());
        assert!(db
            .acknowledged_publish_retry(&compose.assignment_id, revision, &request)
            .unwrap());
        let queued = db
            .publish_recovery_state(&compose.assignment_id)
            .unwrap()
            .unwrap();
        assert_eq!(queued.state, "retryWaiting");
        assert!(queued.manual);
        assert_eq!(queued.max_retries, 0);
        assert!(queued.next_retry_at.unwrap() >= (before + 64_000) as f64);
        assert!(!db
            .pending_publish_dispatch(10)
            .unwrap()
            .iter()
            .any(|job| job.assignment_id == compose.assignment_id));
        assert!(db
            .get_publish_assignment_detail(&campaign, &compose.assignment_id)
            .unwrap()
            .unwrap()
            .assignments[0]
            .effect_intent
            .is_none());
    }
    #[test]
    fn retry_budget_is_three_and_survives_reopening_database() {
        let (db, path, campaign, a) = fixture();
        let run = db.claim_publish_pipeline(&campaign).unwrap().unwrap();
        let id = &a[0].id;
        db.init_publish_recovery(id, &run.token).unwrap();
        db.update_publish_recovery_step(id, &run.token, "sound", Some("mediaSelected"))
            .unwrap();
        assert_eq!(
            db.reserve_publish_step_retry(id, &run.token, "network")
                .unwrap(),
            Some(Duration::from_secs(2))
        );
        let reopened = Database::open(&path).unwrap();
        assert_eq!(
            reopened
                .reserve_publish_step_retry(id, &run.token, "network")
                .unwrap(),
            Some(Duration::from_secs(5))
        );
        assert_eq!(
            reopened
                .reserve_publish_step_retry(id, &run.token, "network")
                .unwrap(),
            Some(Duration::from_secs(10))
        );
        assert!(reopened
            .reserve_publish_step_retry(id, &run.token, "network")
            .unwrap()
            .is_none());
        db.conn()
            .unwrap()
            .execute(
                "UPDATE publish_assignments SET effect_intent='post' WHERE id=?1",
                [id],
            )
            .unwrap();
        assert!(db
            .reserve_publish_step_retry(id, &run.token, "network")
            .is_err());
    }
    #[test]
    fn absent_device_waits_without_spending_and_times_out_at_two_minutes() {
        let (db, _, campaign, _) = fixture();
        let run = db.claim_publish_pipeline(&campaign).unwrap().unwrap();
        let job = db.pending_publish_dispatch(10).unwrap().remove(0);
        db.init_publish_recovery(&job.assignment_id, &run.token)
            .unwrap();
        assert!(!db.admit_publish_reconnect(&job, false, 1000).unwrap());
        assert!(!db.admit_publish_reconnect(&job, false, 119000).unwrap());
        assert_eq!(
            db.publish_recovery_state(&job.assignment_id)
                .unwrap()
                .unwrap()
                .retries_used,
            0
        );
        assert!(!db.admit_publish_reconnect(&job, true, 121000).unwrap());
        assert_eq!(
            db.get_publish_campaign(&campaign)
                .unwrap()
                .unwrap()
                .assignments
                .iter()
                .find(|a| a.id == job.assignment_id)
                .unwrap()
                .state,
            crate::PublishCampaignState::FailedBeforeDispatch
        );
    }
    #[test]
    fn manual_retry_can_use_active_parent_without_restarting_its_other_devices() {
        let (db, _, campaign, _) = fixture();
        let run = db.claim_publish_pipeline(&campaign).unwrap().unwrap();
        let jobs = db.pending_publish_dispatch(10).unwrap();
        let failed = &jobs[0];
        let sibling = &jobs[1];
        db.claim_publish_dispatch(failed, 0).unwrap();
        db.finish_publish_dispatch(failed, Some("terminal failure"))
            .unwrap();
        let rev = db
            .publish_assignment_revision(&failed.assignment_id)
            .unwrap();
        let request = Uuid::new_v4().to_string();
        let resumed = db
            .claim_publish_assignment_retry_checked(&failed.assignment_id, rev, &request)
            .unwrap()
            .unwrap();
        assert_eq!(resumed.token, run.token);
        let after = db.pending_publish_dispatch(10).unwrap();
        assert_eq!(
            after
                .iter()
                .find(|j| j.assignment_id == sibling.assignment_id)
                .unwrap()
                .attempt_id,
            sibling.attempt_id
        );
        assert!(db
            .claim_publish_assignment_retry_checked(
                &failed.assignment_id,
                rev,
                &Uuid::new_v4().to_string()
            )
            .is_err());
    }
    #[test]
    fn manual_retry_ack_is_idempotent_and_has_no_automatic_retries() {
        let (db, _, campaign, a) = fixture();
        let run = db.claim_publish_pipeline(&campaign).unwrap().unwrap();
        let job = db.pending_publish_dispatch(10).unwrap().remove(0);
        db.claim_publish_dispatch(&job, 0).unwrap();
        db.finish_publish_dispatch(&job, Some("terminal refusal"))
            .unwrap();
        db.finish_publish_pipeline(&run).unwrap();
        db.conn()
            .unwrap()
            .execute(
                "UPDATE publish_campaigns SET state='failed_before_dispatch' WHERE id=?1",
                [&campaign],
            )
            .unwrap();
        let id = &job.assignment_id;
        let revision = db.publish_assignment_revision(id).unwrap();
        let request = Uuid::new_v4().to_string();
        let first = db
            .claim_publish_assignment_retry_checked(id, revision, &request)
            .unwrap()
            .unwrap();
        let same = db
            .claim_publish_assignment_retry_checked(id, revision, &request)
            .unwrap()
            .unwrap();
        assert_eq!(first.token, same.token);
        assert!(db
            .acknowledged_publish_retry(id, revision, &request)
            .unwrap());
        assert!(db
            .reserve_publish_step_retry(id, &first.token, "network")
            .unwrap()
            .is_none());
        let other = a.iter().find(|a| a.id != *id).unwrap();
        assert!(db
            .acknowledged_publish_retry(&other.id, revision, &request)
            .is_err());
    }
}
