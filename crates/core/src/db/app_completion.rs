//! Explicit completion intent; historical jobs never enroll automatically.
use super::*;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AppCompletionRecord {
    pub udid: String,
    pub bundle_id: String,
    pub revision: i64,
    pub attempts: u32,
}

#[cfg(test)]
mod tests {
    use super::*;
    const PACKAGE: &str = "com.zhiliaoapp.musically";
    struct Fixture {
        db: Database,
        path: PathBuf,
    }
    impl Fixture {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!("completion-{}.db", Uuid::new_v4()));
            Self {
                db: Database::open(&path).unwrap(),
                path,
            }
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.path);
        }
    }
    fn proof() -> String {
        serde_json::json!({"bundleId":PACKAGE,"oldPid":12}).to_string()
    }
    fn job(f: &Fixture, device: &str, state: &str) {
        f.db.conn().unwrap().execute("INSERT INTO jobs(id,script_name,udids_json,status,created_at,updated_at,steps_json) VALUES(?1,'fixture',?2,?3,'now','now','[]')",params![Uuid::new_v4().to_string(),serde_json::to_string(&[device]).unwrap(),state]).unwrap();
    }
    fn publish(
        f: &Fixture,
        device: &str,
        state: &str,
        run_at: Option<&str>,
        evidence: Option<&str>,
    ) -> String {
        let conn = f.db.conn().unwrap();
        let campaign = Uuid::new_v4().to_string();
        let id = Uuid::new_v4().to_string();
        conn.execute("INSERT INTO publish_campaigns(id,request_id,source_root,request_json,state,run_at,created_at,updated_at) VALUES(?1,?2,'fixture','{}','posting',?3,'now','now')",params![campaign,Uuid::new_v4().to_string(),run_at]).unwrap();
        conn.execute("INSERT INTO publish_bundles(id,campaign_id,ordinal,name,source_path,caption,caption_sha256,manifest_json,created_at) VALUES(?1,?2,0,'fixture','fixture','caption',?3,'{}','now')",params![id,campaign,"a".repeat(64)]).unwrap();
        conn.execute("INSERT INTO publish_assignments(id,campaign_id,bundle_id,ordinal,udid,state,evidence_json,created_at,updated_at) VALUES(?1,?2,?1,0,?3,?4,?5,'now','now')",params![id,campaign,device,state,evidence]).unwrap();
        id
    }
    #[test]
    fn explicit_intents_survive_restart_retry_is_same_row_and_stale_results_lose() {
        let f = Fixture::new();
        assert_eq!(f.db.schema_version().unwrap(), 47);
        assert!(f.db.list_due_app_completions(32).unwrap().is_empty());
        let first = f.db.request_app_completion("a", PACKAGE).unwrap();
        assert_eq!(first, 1);
        let old = f.db.list_due_app_completions(32).unwrap().remove(0);
        assert!(f.db.defer_app_completion(&old, "device busy").unwrap());
        assert!(!f.db.defer_app_completion(&old, "duplicate retry").unwrap());
        let reopened = Database::open(&f.path).unwrap();
        assert!(reopened.list_due_app_completions(32).unwrap().is_empty());
        let s = reopened.get_app_completion("a", PACKAGE).unwrap().unwrap();
        assert_eq!(s.attempts, 1);
        assert_eq!(s.revision, 1);
        assert_eq!(s.state, "pending");
        assert_eq!(reopened.request_app_completion("a", PACKAGE).unwrap(), 2);
        assert!(!reopened.app_completion_is_current(&old).unwrap());
        assert!(!reopened.finish_app_completion(&old, &proof()).unwrap());
        let current = reopened.list_due_app_completions(32).unwrap().remove(0);
        assert!(reopened.finish_app_completion(&current, &proof()).unwrap());
        assert!(!reopened.finish_app_completion(&current, &proof()).unwrap());
        assert!(reopened.list_due_app_completions(32).unwrap().is_empty());
        assert_eq!(
            reopened
                .get_app_completion("a", PACKAGE)
                .unwrap()
                .unwrap()
                .proof_json
                .as_deref(),
            Some(proof().as_str())
        );
        assert_eq!(
            reopened
                .conn()
                .unwrap()
                .query_row("SELECT COUNT(*) FROM app_completion_queue", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            1
        );
    }
    #[test]
    fn unrelated_or_future_work_does_not_hold_completed_phone() {
        let f = Fixture::new();
        job(&f, "b", "\"running\"");
        job(&f, "a", "succeeded");
        job(&f, "a", "\"failed\"");
        publish(&f, "a", "scheduled", Some("2999-01-01T00:00:00"), None);
        publish(&f, "b", "verifying", None, None);
        assert!(f.db.app_completion_block_reason("a").unwrap().is_none());
        assert!(f.db.app_completion_block_reason("b").unwrap().is_some());
        publish(&f, "a", "ready", None, None);
        assert!(f.db.app_completion_block_reason("a").unwrap().is_some());
    }
    #[test]
    fn unresolved_link_debt_blocks_but_legacy_confirmed_link_does_not() {
        let f = Fixture::new();
        publish(
            &f,
            "a",
            "succeeded",
            None,
            Some(r#"{"post":{"postUrl":"https://www.tiktok.com/@fixture/photo/123"}}"#),
        );
        assert!(f.db.app_completion_block_reason("a").unwrap().is_none());
        publish(
            &f,
            "a",
            "uncertain",
            None,
            Some(
                r#"{"verificationStatus":{"state":"needsReview"},"post":{"verdict":"Posted","state":"posted"}}"#,
            ),
        );
        assert!(f.db.app_completion_block_reason("a").unwrap().is_some());
    }
    #[test]
    fn latest_nurture_state_and_own_completion_capacity_are_respected() {
        let f = Fixture::new();
        let conn = f.db.conn().unwrap();
        conn.execute("INSERT INTO nurture_runs(id,target_udids_json,target_count,created_at,updated_at) VALUES('r','[\"a\"]',1,'now','now')",[]).unwrap();
        for running in [true, false] {
            conn.execute("INSERT INTO nurture_run_status_events(run_id,udid,status_json,recorded_at) VALUES('r','a',?1,'now')",[serde_json::json!({"udid":"a","runId":"r","running":running}).to_string()]).unwrap();
            assert_eq!(
                f.db.app_completion_block_reason("a").unwrap().is_some(),
                running
            );
        }
        conn.execute("INSERT INTO publish_work_claims(token,udid,stage,owner,claimed_at_ms) VALUES('c','a','appCompletion','close',1)",[]).unwrap();
        assert!(f.db.app_completion_block_reason("a").unwrap().is_none());
        conn.execute(
            "UPDATE publish_work_claims SET stage='verify' WHERE token='c'",
            [],
        )
        .unwrap();
        assert!(f.db.app_completion_block_reason("a").unwrap().is_some());
    }
    #[test]
    fn queue_is_bounded_per_poll_and_wrong_proof_does_not_settle() {
        let f = Fixture::new();
        for n in 0..40 {
            f.db.request_app_completion(&format!("device{n}"), PACKAGE)
                .unwrap();
        }
        assert_eq!(f.db.list_due_app_completions(usize::MAX).unwrap().len(), 32);
        assert!(f.db.list_due_app_completions(0).unwrap().is_empty());
        let row = f.db.list_due_app_completions(1).unwrap().remove(0);
        assert!(f
            .db
            .finish_app_completion(&row, r#"{"bundleId":"other","oldPid":null}"#)
            .is_err());
        assert!(f.db.app_completion_is_current(&row).unwrap());
    }

    #[test]
    fn normal_busy_wait_stays_five_seconds_without_growing_error_backoff() {
        let f = Fixture::new();
        f.db.request_app_completion("a", PACKAGE).unwrap();
        let row = f.db.list_due_app_completions(1).unwrap().remove(0);
        for _ in 0..100 {
            assert!(f
                .db
                .wait_app_completion(&row, "another task owns phone")
                .unwrap());
        }
        let status = f.db.get_app_completion("a", PACKAGE).unwrap().unwrap();
        assert_eq!(status.attempts, 0);
        assert_eq!(status.revision, row.revision);
        let remaining: i64 =
            f.db.conn()
                .unwrap()
                .query_row(
                    "SELECT next_attempt_at_ms-?1 FROM app_completion_queue WHERE udid='a'",
                    [Utc::now().timestamp_millis()],
                    |r| r.get(0),
                )
                .unwrap();
        assert!((0..=5000).contains(&remaining));
        assert!(f.db.defer_app_completion(&row, "USB disconnected").unwrap());
        let status = f.db.get_app_completion("a", PACKAGE).unwrap().unwrap();
        assert_eq!(status.attempts, 1);
    }

    #[test]
    fn skipped_parent_phone_is_done_even_while_sibling_campaign_keeps_running() {
        let f = Fixture::new();
        let conn = f.db.conn().unwrap();
        conn.execute("INSERT INTO interaction_campaigns(id,request_id,request_json,state,message_count,created_at,updated_at) VALUES('campaign','request','{}','running',2,'now','now')",[]).unwrap();
        conn.execute("INSERT INTO interaction_targets(id,campaign_id,line_no,original_url,normalized_url,target_key,content_id,kind,created_at) VALUES('target','campaign',1,'https://www.tiktok.com/@fixture/video/123','https://www.tiktok.com/@fixture/video/123','video:123','123','video','now')",[]).unwrap();
        for (id, device, ordinal, state) in [
            ("a", "finished", 0, "skipped_parent"),
            ("b", "busy", 1, "queued"),
        ] {
            conn.execute("INSERT INTO interaction_assignments(id,campaign_id,target_id,message_ordinal,actor_udid,state,created_at,updated_at) VALUES(?1,'campaign','target',?2,?3,?4,'now','now')",params![id,ordinal,device,state]).unwrap();
        }
        assert!(f
            .db
            .app_completion_block_reason("finished")
            .unwrap()
            .is_none());
        assert!(f.db.app_completion_block_reason("busy").unwrap().is_some());
        conn.execute(
            "UPDATE interaction_assignments SET state='queued' WHERE id='a'",
            [],
        )
        .unwrap();
        assert!(f
            .db
            .app_completion_block_reason("finished")
            .unwrap()
            .is_some());
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppCompletionStatus {
    pub udid: String,
    pub bundle_id: String,
    pub revision: i64,
    pub attempts: u32,
    pub state: String,
    pub reason: Option<String>,
    pub proof_json: Option<String>,
    pub updated_at_ms: i64,
}

fn validate(udid: &str, bundle: &str) -> anyhow::Result<()> {
    anyhow::ensure!(
        !udid.trim().is_empty() && udid.len() <= 256 && !udid.chars().any(char::is_control),
        "Invalid completion device"
    );
    anyhow::ensure!(
        !bundle.is_empty()
            && bundle.len() <= 255
            && bundle
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-')),
        "Invalid completion package"
    );
    Ok(())
}

impl Database {
    pub fn request_app_completion(&self, udid: &str, bundle_id: &str) -> anyhow::Result<i64> {
        validate(udid, bundle_id)?;
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let now = Utc::now().timestamp_millis();
        let revision=tx.query_row("INSERT INTO app_completion_queue(udid,bundle_id,revision,state,attempts,next_attempt_at_ms,requested_at_ms,updated_at_ms)
            VALUES(?1,?2,1,'pending',0,?3,?3,?3)
            ON CONFLICT(udid,bundle_id) DO UPDATE SET revision=app_completion_queue.revision+1,state='pending',attempts=0,
            next_attempt_at_ms=excluded.next_attempt_at_ms,reason=NULL,proof_json=NULL,requested_at_ms=excluded.requested_at_ms,updated_at_ms=excluded.updated_at_ms
            RETURNING revision",params![udid,bundle_id,now],|r|r.get(0))?;
        tx.commit()?;
        Ok(revision)
    }
    pub fn list_due_app_completions(
        &self,
        limit: usize,
    ) -> anyhow::Result<Vec<AppCompletionRecord>> {
        let conn = self.conn()?;
        let mut statement=conn.prepare("SELECT udid,bundle_id,revision,attempts FROM app_completion_queue
            WHERE state='pending' AND next_attempt_at_ms<=?1 ORDER BY next_attempt_at_ms,requested_at_ms,udid,bundle_id LIMIT ?2")?;
        let rows = statement
            .query_map(
                params![Utc::now().timestamp_millis(), limit.min(32) as i64],
                |r| {
                    Ok(AppCompletionRecord {
                        udid: r.get(0)?,
                        bundle_id: r.get(1)?,
                        revision: r.get(2)?,
                        attempts: r.get::<_, i64>(3)?.clamp(0, u32::MAX as i64) as u32,
                    })
                },
            )?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }
    pub fn app_completion_is_current(&self, record: &AppCompletionRecord) -> anyhow::Result<bool> {
        Ok(self.conn()?.query_row("SELECT EXISTS(SELECT 1 FROM app_completion_queue WHERE udid=?1 AND bundle_id=?2 AND revision=?3 AND state='pending')",params![record.udid,record.bundle_id,record.revision],|r|r.get(0))?)
    }
    pub fn defer_app_completion(
        &self,
        record: &AppCompletionRecord,
        reason: &str,
    ) -> anyhow::Result<bool> {
        let now = Utc::now().timestamp_millis();
        let delay = 30_000_i64
            .saturating_mul(1_i64 << record.attempts.min(5))
            .min(900_000);
        Ok(self.conn()?.execute("UPDATE app_completion_queue SET attempts=attempts+1,next_attempt_at_ms=?4,reason=?5,updated_at_ms=?6
            WHERE udid=?1 AND bundle_id=?2 AND revision=?3 AND state='pending' AND attempts=?7",
            params![record.udid,record.bundle_id,record.revision,now.saturating_add(delay),reason.chars().take(2048).collect::<String>(),now,record.attempts])?>0)
    }

    /// Ordinary device work is polled promptly without consuming the retry
    /// budget. Exponential backoff belongs only to actual closure failures.
    pub fn wait_app_completion(
        &self,
        record: &AppCompletionRecord,
        reason: &str,
    ) -> anyhow::Result<bool> {
        let now = Utc::now().timestamp_millis();
        Ok(self.conn()?.execute(
            "UPDATE app_completion_queue SET next_attempt_at_ms=?4,reason=?5,updated_at_ms=?6
            WHERE udid=?1 AND bundle_id=?2 AND revision=?3 AND state='pending'",
            params![
                record.udid,
                record.bundle_id,
                record.revision,
                now.saturating_add(5000),
                reason.chars().take(2048).collect::<String>(),
                now
            ],
        )? > 0)
    }
    pub fn finish_app_completion(
        &self,
        record: &AppCompletionRecord,
        proof_json: &str,
    ) -> anyhow::Result<bool> {
        anyhow::ensure!(proof_json.len() <= 16_384, "Completion proof exceeds limit");
        let proof: crate::driver::ProcessAbsenceProof = serde_json::from_str(proof_json)?;
        anyhow::ensure!(
            proof.bundle_id == record.bundle_id,
            "Completion proof package mismatch"
        );
        Ok(self.conn()?.execute("UPDATE app_completion_queue SET state='completed',proof_json=?4,reason=NULL,updated_at_ms=?5
            WHERE udid=?1 AND bundle_id=?2 AND revision=?3 AND state='pending'",params![record.udid,record.bundle_id,record.revision,proof_json,Utc::now().timestamp_millis()])?>0)
    }
    pub fn get_app_completion(
        &self,
        udid: &str,
        bundle_id: &str,
    ) -> anyhow::Result<Option<AppCompletionStatus>> {
        Ok(self.conn()?.query_row("SELECT udid,bundle_id,revision,attempts,state,reason,proof_json,updated_at_ms FROM app_completion_queue WHERE udid=?1 AND bundle_id=?2",params![udid,bundle_id],|r|Ok(AppCompletionStatus{udid:r.get(0)?,bundle_id:r.get(1)?,revision:r.get(2)?,attempts:r.get::<_,i64>(3)?.clamp(0,u32::MAX as i64) as u32,state:r.get(4)?,reason:r.get(5)?,proof_json:r.get(6)?,updated_at_ms:r.get(7)?})).optional()?)
    }
    /// Conservative persisted guard. The exclusive control-plane lease protects
    /// in-memory jobs and callers must recheck this after acquiring that lease.
    pub fn app_completion_block_reason(&self, udid: &str) -> anyhow::Result<Option<String>> {
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Deferred)?;
        let now = Utc::now().timestamp_millis();
        let local = chrono::Local::now()
            .naive_local()
            .format("%Y-%m-%dT%H:%M:%S")
            .to_string();
        let checks=[
            ("publish capacity", "SELECT EXISTS(SELECT 1 FROM publish_work_claims WHERE udid=?1 AND stage<>'appCompletion')"),
            ("nurture", "SELECT EXISTS(SELECT 1 FROM nurture_run_status_events e WHERE e.udid=?1 AND COALESCE(json_extract(e.status_json,'$.running'),1)<>0 AND NOT EXISTS(SELECT 1 FROM nurture_run_status_events n WHERE n.run_id=e.run_id AND n.udid=e.udid AND n.sequence>e.sequence))"),
            ("interaction", "SELECT EXISTS(SELECT 1 FROM interaction_assignments a JOIN interaction_campaigns c ON c.id=a.campaign_id WHERE a.actor_udid=?1 AND (a.state IN ('uncertain','sending','posting','verifying') OR (c.state IN ('queued','running') AND a.state NOT IN ('succeeded','failed','failed_before_dispatch','failed_before_effect','cancelled','skipped','skipped_parent'))))"),
            ("comment verification", "SELECT EXISTS(SELECT 1 FROM interaction_comment_verification WHERE device_id=?1 AND state='pending')"),
            ("public action", "SELECT EXISTS(SELECT 1 FROM tiktok_action_runs WHERE device_udid=?1 AND state IN ('planned','preparing','armed','uncertain'))"),
            ("Flow", "SELECT EXISTS(SELECT 1 FROM flow_device_runs d WHERE d.udid=?1 AND (d.state IN ('queued','preflight','running') OR EXISTS(SELECT 1 FROM flow_node_attempts a WHERE a.device_run_id=d.id AND a.state IN ('intentCommitted','effectDispatched','verifying','uncertain','interrupted'))))"),
            ("job", "SELECT EXISTS(SELECT 1 FROM jobs j WHERE j.status NOT IN ('\"succeeded\"','\"failed\"','\"cancelled\"','succeeded','failed','cancelled','\"uncertain\"','uncertain') AND CASE WHEN json_valid(j.udids_json) THEN EXISTS(SELECT 1 FROM json_each(j.udids_json) u WHERE u.value=?1) ELSE 1 END)"),
            ("orchestration", "SELECT EXISTS(SELECT 1 FROM orchestration_runs r WHERE r.state IN ('queued','running','uncertain') AND (EXISTS(SELECT 1 FROM orchestration_attempts a JOIN json_each(a.snapshot_json,'$.target.included') t WHERE a.run_id=r.id AND a.state IN ('queued','dispatching','waiting_child','uncertain') AND json_extract(t.value,'$.udid')=?1) OR EXISTS(SELECT 1 FROM json_each(r.node_targets_json) n JOIN json_each(n.value,'$.included') t WHERE json_extract(t.value,'$.udid')=?1 AND NOT EXISTS(SELECT 1 FROM orchestration_attempts a WHERE a.run_id=r.id AND a.node_id=n.key AND a.state IN ('done','partial','failed','cancelled'))) OR (NOT EXISTS(SELECT 1 FROM json_each(r.node_targets_json)) AND EXISTS(SELECT 1 FROM json_each(r.target_json,'$.included') t WHERE json_extract(t.value,'$.udid')=?1))))"),
        ];
        for (name, sql) in checks {
            if tx
                .query_row(sql, [udid], |r| r.get::<_, bool>(0))
                .with_context(|| format!("Unable to verify pending {name} work"))?
            {
                return Ok(Some(format!("Chờ công việc {name} trên thiết bị hoàn tất")));
            }
        }
        // Share the same generation/intent-bound release proof as new Publish.
        // An old unresolved post remains in linkReview after verified Stop; it
        // must not permanently block selecting the next app on this device.
        if !Self::publish_device_guard_from_connection(&tx, udid)?
            .blocking
            .is_empty()
        {
            return Ok(Some(
                "Chờ tải bài hoặc xác minh liên kết trên thiết bị hoàn tất".into(),
            ));
        }
        let pending:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM publish_assignments a JOIN publish_campaigns c ON c.id=a.campaign_id WHERE a.udid=?1 AND (
            a.state IN ('preparing','transferring') OR (a.state IN ('queued','scheduled','ready','imported') AND c.state NOT IN ('cancelled','missed','succeeded') AND (c.run_at IS NULL OR datetime(c.run_at)<=datetime(?2))) OR EXISTS(SELECT 1 FROM publish_dispatch_jobs j WHERE j.assignment_id=a.id AND j.state IN ('queued','running') AND (j.started_at_ms IS NOT NULL OR j.deadline_ms IS NULL OR j.deadline_ms-30000<=?3))))",params![udid,local,now],|r|r.get(0))?;
        if pending {
            return Ok(Some(
                "Chờ bài đăng đã đến lượt trên thiết bị hoàn tất".into(),
            ));
        }
        tx.commit()?;
        Ok(None)
    }
}
