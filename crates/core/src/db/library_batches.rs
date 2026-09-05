use super::*;
use crate::{
    OperationBatchSnapshot, OperationRunDetail, OperationRunItem, OperationRunItemKind,
    OperationRunKind, OperationRunState, OperationRunSummary, ResolvedTargetSnapshot,
};

impl Database {
    pub fn create_library_batch(
        &self,
        id: &str,
        kind: OperationRunKind,
        artifact_id: &str,
        title: &str,
        target: &ResolvedTargetSnapshot,
    ) -> anyhow::Result<()> {
        anyhow::ensure!(
            matches!(
                kind,
                OperationRunKind::AppInstall | OperationRunKind::MaterialTransfer
            ),
            "not a library batch kind"
        );
        anyhow::ensure!(!target.included.is_empty(), "empty library batch");
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let now = Utc::now().to_rfc3339();
        tx.execute("INSERT INTO library_batches(id,kind,artifact_id,title,target_json,created_at,updated_at)
            VALUES(?1,?2,?3,?4,?5,?6,?6)",
            params![id,kind.as_key(),artifact_id,title,serde_json::to_string(target)?,now])?;
        for (index, device) in target.included.iter().enumerate() {
            let label = match (device.number, device.alias.trim()) {
                (Some(number), "") => format!("Máy {number}"),
                (Some(number), alias) => format!("Máy {number} · {alias}"),
                (None, "") => format!("Máy {}", index + 1),
                (None, alias) => format!("Máy {} · {alias}", index + 1),
            };
            tx.execute("INSERT INTO library_batch_items(batch_id,udid,ordinal,label,state) VALUES(?1,?2,?3,?4,'queued')",
                params![id,device.udid,index as i64,label])?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Durable write-ahead boundary. Cancellation can only win while this row is queued.
    pub fn claim_library_batch_item(&self, id: &str, udid: &str) -> anyhow::Result<bool> {
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let changed = tx.execute("UPDATE library_batch_items SET state='running' WHERE batch_id=?1 AND udid=?2 AND state='queued'", params![id,udid])?;
        if changed == 1 {
            tx.execute(
                "UPDATE library_batches SET updated_at=?2 WHERE id=?1",
                params![id, Utc::now().to_rfc3339()],
            )?;
        }
        tx.commit()?;
        Ok(changed == 1)
    }

    pub fn settle_library_batch_item(
        &self,
        id: &str,
        udid: &str,
        state: OperationRunState,
        error_code: Option<&str>,
        detail: Option<&str>,
        evidence: Option<&str>,
    ) -> anyhow::Result<()> {
        anyhow::ensure!(
            matches!(
                state,
                OperationRunState::Succeeded
                    | OperationRunState::Failed
                    | OperationRunState::Uncertain
                    | OperationRunState::Cancelled
            ),
            "nonterminal batch settlement"
        );
        let state = serde_json::to_value(state)?
            .as_str()
            .context("batch state string")?
            .to_string();
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute(
            "UPDATE library_batch_items SET state=?3,error_code=?4,detail=?5,evidence=?6
            WHERE batch_id=?1 AND udid=?2 AND state IN ('queued','running')",
            params![id, udid, state, error_code, detail, evidence],
        )?;
        tx.execute(
            "UPDATE library_batches SET updated_at=?2 WHERE id=?1",
            params![id, Utc::now().to_rfc3339()],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn cancel_library_batch(&self, id: &str) -> anyhow::Result<usize> {
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let count = tx.execute("UPDATE library_batch_items SET state='cancelled',error_code='CancelledBeforeDispatch',detail='Đã dừng trước khi gửi lệnh tới máy'
            WHERE batch_id=?1 AND state='queued'", [id])?;
        tx.execute(
            "UPDATE library_batches SET updated_at=?2 WHERE id=?1",
            params![id, Utc::now().to_rfc3339()],
        )?;
        tx.commit()?;
        Ok(count)
    }

    /// Called once at app bootstrap, never from a read command or Database::open.
    pub fn recover_library_batches(&self) -> anyhow::Result<usize> {
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute(
            "UPDATE library_batches SET updated_at=?1 WHERE id IN
            (SELECT batch_id FROM library_batch_items WHERE state IN ('queued','running'))",
            [Utc::now().to_rfc3339()],
        )?;
        let count = tx.execute("UPDATE library_batch_items SET
            error_code=CASE state WHEN 'queued' THEN 'RestartBeforeDispatch' ELSE 'RestartAfterIntent' END,
            detail=CASE state WHEN 'queued' THEN 'Ứng dụng khởi động lại trước khi gửi lệnh; chưa chạy trên máy'
            ELSE 'Ứng dụng khởi động lại sau khi ghi ý định thực hiện; chưa đủ bằng chứng đọc lại, cần kiểm tra máy trước khi chạy tiếp' END,
            state=CASE state WHEN 'queued' THEN 'cancelled' ELSE 'uncertain' END
            WHERE state IN ('queued','running')", [])?;
        tx.commit()?;
        Ok(count)
    }

    pub fn get_library_batch(&self, id: &str) -> anyhow::Result<Option<OperationRunDetail>> {
        let conn = self.conn()?;
        let row = conn.query_row("SELECT kind,artifact_id,title,target_json,created_at,updated_at FROM library_batches WHERE id=?1", [id], |row| {
            Ok((row.get::<_,String>(0)?,row.get::<_,String>(1)?,row.get::<_,String>(2)?,row.get::<_,String>(3)?,row.get::<_,String>(4)?,row.get::<_,String>(5)?))
        }).optional()?;
        let Some((kind, artifact_id, title, target_json, created_at, updated_at)) = row else {
            return Ok(None);
        };
        let kind = serde_json::from_value::<OperationRunKind>(serde_json::Value::String(kind))?;
        let target = serde_json::from_str::<ResolvedTargetSnapshot>(&target_json)?;
        let mut stmt = conn.prepare("SELECT udid,label,state,error_code,detail,evidence FROM library_batch_items WHERE batch_id=?1 ORDER BY ordinal")?;
        let rows = stmt
            .query_map([id], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, Option<String>>(5)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let items = rows
            .into_iter()
            .map(|(udid, label, state, error_code, detail, evidence)| {
                let state =
                    serde_json::from_value::<OperationRunState>(serde_json::Value::String(state))?;
                Ok(OperationRunItem {
                    id: udid.clone(),
                    kind: OperationRunItemKind::Device,
                    label,
                    state,
                    udid: Some(udid),
                    error_code,
                    detail,
                    evidence,
                    retryable: state == OperationRunState::Failed
                        || state == OperationRunState::Cancelled,
                })
            })
            .collect::<anyhow::Result<Vec<_>>>()?;
        let total = items.len() as u32;
        let completed = items.iter().filter(|item| item.state.is_terminal()).count() as u32;
        let succeeded = items
            .iter()
            .filter(|item| item.state == OperationRunState::Succeeded)
            .count() as u32;
        let state = if items
            .iter()
            .any(|item| item.state == OperationRunState::Running)
        {
            OperationRunState::Running
        } else if completed < total {
            OperationRunState::Queued
        } else if items
            .iter()
            .any(|item| item.state == OperationRunState::Uncertain)
        {
            OperationRunState::Uncertain
        } else if succeeded == total {
            OperationRunState::Succeeded
        } else if succeeded > 0 {
            OperationRunState::Partial
        } else if items
            .iter()
            .all(|item| item.state == OperationRunState::Cancelled)
        {
            OperationRunState::Cancelled
        } else {
            OperationRunState::Failed
        };
        let summary = OperationRunSummary {
            id: format!("{}:{id}", kind.as_key()),
            source_id: id.into(),
            kind,
            title,
            state,
            target_count: target.included.len() as u32,
            total_items: total,
            completed_items: completed,
            issue_count: items
                .iter()
                .filter(|item| item.state.needs_attention())
                .count() as u32,
            retryable_count: items.iter().filter(|item| item.retryable).count() as u32,
            retry_scope: None,
            created_at: Some(created_at),
            updated_at: Some(updated_at),
        };
        Ok(Some(OperationRunDetail {
            summary,
            items,
            batch: Some(OperationBatchSnapshot {
                artifact_id,
                target,
            }),
        }))
    }

    /// Filter in SQL before hydrating heterogeneous sources. Refuse an oversized result,
    /// rather than displaying partial counts as complete. No source has its own hidden cap.
    pub fn operation_source_ids(
        &self,
        since: Option<&str>,
        kind: Option<OperationRunKind>,
    ) -> anyhow::Result<Vec<String>> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare("SELECT operation_id FROM (
            SELECT 'script:' || id operation_id,updated_at,'script' kind,json_extract(status,'$') IN ('queued','running') active FROM jobs UNION ALL
            SELECT 'flow:' || id,updated_at,'flow',state IN ('queued','running') FROM flow_runs UNION ALL
            SELECT 'orchestration:' || id,updated_at,'orchestration',state IN ('queued','running') FROM orchestration_runs UNION ALL
            SELECT 'nurture:' || id,updated_at,'nurture',0 FROM nurture_runs UNION ALL
            SELECT 'interaction:' || id,updated_at,'interaction',state IN ('queued','running') FROM interaction_campaigns UNION ALL
            SELECT 'publish:' || id,updated_at,'publish',state IN ('queued','scheduled','preparing','ready','transferring','imported','posting','verifying') FROM publish_campaigns UNION ALL
            SELECT kind || ':' || id,updated_at,kind,EXISTS(SELECT 1 FROM library_batch_items i WHERE i.batch_id=library_batches.id AND i.state IN ('queued','running')) FROM library_batches
        ) WHERE (?1 IS NULL OR active OR julianday(updated_at)>=julianday(?1)) AND (?2 IS NULL OR kind=?2)
        ORDER BY updated_at DESC,operation_id LIMIT 10001")?;
        let ids = stmt
            .query_map(params![since, kind.map(OperationRunKind::as_key)], |row| {
                row.get::<_, String>(0)
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        anyhow::ensure!(
            ids.len() <= 10000,
            "OperationQueryTooBroad: more than 10000 runs; choose a shorter time range"
        );
        Ok(ids)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TempRoot(std::path::PathBuf);
    impl Drop for TempRoot {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    fn fixture() -> (Database, TempRoot, ResolvedTargetSnapshot) {
        let root =
            TempRoot(std::env::temp_dir().join(format!("riviu-library-batch-{}", Uuid::new_v4())));
        let db = Database::open(root.0.join("batch.db")).unwrap();
        let target = crate::resolve_target(
            &crate::TargetRef::Explicit {
                udids: vec!["a".into(), "b".into()],
            },
            &["a".into(), "b".into()],
            &[],
            &[],
        )
        .unwrap();
        (db, root, target)
    }

    #[test]
    fn restart_preserves_terminal_and_separates_queued_from_intent() {
        let (db, _root, target) = fixture();
        db.create_library_batch(
            "one",
            OperationRunKind::AppInstall,
            "artifact",
            "Install",
            &target,
        )
        .unwrap();
        assert!(db.claim_library_batch_item("one", "a").unwrap());
        assert!(!db.claim_library_batch_item("one", "a").unwrap());
        assert_eq!(db.recover_library_batches().unwrap(), 2);
        let detail = db.get_library_batch("one").unwrap().unwrap();
        assert_eq!(detail.items[0].state, OperationRunState::Uncertain);
        assert!(!detail.items[0].retryable);
        assert_eq!(detail.items[1].state, OperationRunState::Cancelled);
        assert!(!db.claim_library_batch_item("one", "a").unwrap());
        assert_eq!(db.recover_library_batches().unwrap(), 0);
    }

    #[test]
    fn failed_write_ahead_never_allows_dispatch_or_loses_queued_state() {
        let (db, _root, target) = fixture();
        db.create_library_batch(
            "audit",
            OperationRunKind::AppInstall,
            "artifact",
            "Install",
            &target,
        )
        .unwrap();
        db.conn()
            .unwrap()
            .execute_batch(
                "CREATE TRIGGER reject_intent BEFORE UPDATE ON library_batch_items
            WHEN NEW.state='running' BEGIN SELECT RAISE(ABORT,'fixture audit unavailable'); END;",
            )
            .unwrap();
        let mut dispatches = 0;
        if db.claim_library_batch_item("audit", "a").unwrap_or(false) {
            dispatches += 1;
        }
        assert_eq!(dispatches, 0);
        assert_eq!(
            db.get_library_batch("audit").unwrap().unwrap().items[0].state,
            OperationRunState::Queued
        );
        db.settle_library_batch_item(
            "audit",
            "a",
            OperationRunState::Failed,
            Some("IntentPersistenceUnavailable"),
            Some("fixture audit unavailable"),
            None,
        )
        .unwrap();
        let failed = db.get_library_batch("audit").unwrap().unwrap();
        assert_eq!(failed.items[0].state, OperationRunState::Failed);
        assert!(failed.items[0].retryable);
        assert!(!db.claim_library_batch_item("audit", "a").unwrap());
        db.conn()
            .unwrap()
            .execute_batch("DROP TRIGGER reject_intent")
            .unwrap();
        assert!(db.claim_library_batch_item("audit", "b").unwrap());
        db.settle_library_batch_item(
            "audit",
            "b",
            OperationRunState::Succeeded,
            None,
            None,
            Some("readback"),
        )
        .unwrap();
        db.settle_library_batch_item(
            "audit",
            "b",
            OperationRunState::Failed,
            Some("LateFailure"),
            None,
            None,
        )
        .unwrap();
        db.recover_library_batches().unwrap();
        let settled = db.get_library_batch("audit").unwrap().unwrap();
        assert_eq!(settled.items[1].state, OperationRunState::Succeeded);
        assert_eq!(settled.items[1].evidence.as_deref(), Some("readback"));
    }

    #[test]
    fn cancellation_only_wins_before_dispatch_and_snapshot_survives() {
        let (db, _root, target) = fixture();
        db.create_library_batch(
            "one",
            OperationRunKind::MaterialTransfer,
            "artifact",
            "Transfer",
            &target,
        )
        .unwrap();
        assert!(db.claim_library_batch_item("one", "a").unwrap());
        assert_eq!(db.cancel_library_batch("one").unwrap(), 1);
        assert!(!db.claim_library_batch_item("one", "b").unwrap());
        db.settle_library_batch_item(
            "one",
            "a",
            OperationRunState::Succeeded,
            None,
            None,
            Some("hash-readback"),
        )
        .unwrap();
        let detail = db.get_library_batch("one").unwrap().unwrap();
        assert_eq!(detail.summary.state, OperationRunState::Partial);
        assert_eq!(detail.items[0].evidence.as_deref(), Some("hash-readback"));
        assert_eq!(detail.batch.unwrap().target, target);
        assert!(db
            .create_library_batch(
                "one",
                OperationRunKind::MaterialTransfer,
                "artifact",
                "Transfer",
                &target
            )
            .is_err());
        assert_eq!(
            db.operation_source_ids(None, None).unwrap(),
            vec!["materialTransfer:one"]
        );
    }

    #[test]
    fn query_counts_all_matches_before_pagination_and_preserves_old_active_runs() {
        let (db, _root, target) = fixture();
        for index in 0..205 {
            let id = format!("batch-{index:03}");
            db.create_library_batch(
                &id,
                OperationRunKind::AppInstall,
                "artifact",
                "Fixture",
                &target,
            )
            .unwrap();
            db.cancel_library_batch(&id).unwrap();
        }
        db.create_library_batch(
            "active",
            OperationRunKind::MaterialTransfer,
            "artifact",
            "Transfer",
            &target,
        )
        .unwrap();
        db.conn()
            .unwrap()
            .execute(
                "UPDATE library_batches SET updated_at='2020-01-01T00:00:00Z'",
                [],
            )
            .unwrap();
        assert_eq!(
            db.operation_source_ids(Some("2026-01-01T00:00:00Z"), None)
                .unwrap(),
            vec!["materialTransfer:active"]
        );
        let ids = db
            .operation_source_ids(None, Some(OperationRunKind::AppInstall))
            .unwrap();
        assert_eq!(ids.len(), 205);
        let runs = ids
            .iter()
            .map(|id| {
                db.get_library_batch(id.split_once(':').unwrap().1)
                    .unwrap()
                    .unwrap()
                    .summary
            })
            .collect();
        let page = crate::query_operation_summaries(
            runs,
            &crate::OperationRunQuery {
                offset: Some(200),
                limit: Some(50),
                ..Default::default()
            },
        );
        assert_eq!(page.total, 205);
        assert_eq!(page.runs.len(), 5);
        assert!(!page.has_more);
    }
}
