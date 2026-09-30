//! Completed workers retry database settlement, never device work. The sidecar
//! survives SQLite write failures and is replayed before startup orphan recovery.
use super::*;
use crate::publish_recovery::RecoveryFailure;
use std::io::{Read, Write};

const MAX_RECEIPT_BYTES: u64 = 16 * 1024;
const MAX_STARTUP_ENTRIES: usize = 64;

#[derive(Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct CompletionReceipt {
    version: u8,
    assignment_id: String,
    campaign_id: String,
    run_token: String,
    attempt_id: String,
    udid: String,
    phase: String,
    revision: i64,
    failure: Option<RecoveryFailure>,
}

impl CompletionReceipt {
    fn new(job: &PublishDispatchJob, error: Option<&anyhow::Error>) -> Self {
        let failure = error.map(|error| {
            let mut failure = crate::publish_recovery::describe(error);
            failure.message = failure.message.chars().take(2048).collect();
            failure.code = failure.code.chars().take(128).collect();
            failure
        });
        Self {
            version: 1,
            assignment_id: job.assignment_id.clone(),
            campaign_id: job.run.campaign_id.clone(),
            run_token: job.run.token.clone(),
            attempt_id: job.attempt_id.clone(),
            udid: job.udid.clone(),
            phase: job.phase.clone(),
            revision: job.revision,
            failure,
        }
    }

    fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.version == 1 && self.revision >= 0,
            "invalid dispatch completion version/revision"
        );
        anyhow::ensure!(
            [
                &self.assignment_id,
                &self.campaign_id,
                &self.run_token,
                &self.attempt_id,
                &self.udid
            ]
            .iter()
            .all(|value| !value.is_empty() && value.len() <= 512),
            "invalid dispatch completion identity"
        );
        anyhow::ensure!(
            matches!(self.phase.as_str(), "transfer" | "compose"),
            "invalid dispatch completion phase"
        );
        Ok(())
    }

    fn file_name(&self) -> String {
        use sha2::Digest;
        let identity = serde_json::json!([
            self.assignment_id,
            self.campaign_id,
            self.run_token,
            self.attempt_id,
            self.revision
        ]);
        format!(
            "{:x}.json",
            sha2::Sha256::digest(identity.to_string().as_bytes())
        )
    }

    fn job(&self) -> PublishDispatchJob {
        PublishDispatchJob {
            assignment_id: self.assignment_id.clone(),
            run: PublishPipelineRun {
                campaign_id: self.campaign_id.clone(),
                token: self.run_token.clone(),
            },
            attempt_id: self.attempt_id.clone(),
            udid: self.udid.clone(),
            phase: self.phase.clone(),
            revision: self.revision,
        }
    }
}

fn read_receipt(path: &Path) -> anyhow::Result<CompletionReceipt> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)?
        .take(MAX_RECEIPT_BYTES + 1)
        .read_to_end(&mut bytes)?;
    anyhow::ensure!(
        bytes.len() as u64 <= MAX_RECEIPT_BYTES,
        "dispatch completion exceeds size limit"
    );
    let receipt: CompletionReceipt = serde_json::from_slice(&bytes)?;
    receipt.validate()?;
    anyhow::ensure!(
        path.file_name().and_then(|name| name.to_str()) == Some(receipt.file_name().as_str()),
        "dispatch completion filename does not match identity"
    );
    Ok(receipt)
}

impl Database {
    fn settle_completion_receipt(&self, receipt: &CompletionReceipt) -> anyhow::Result<bool> {
        let job = receipt.job();
        let current: bool = self.conn()?.query_row(
            "SELECT EXISTS(SELECT 1 FROM publish_dispatch_jobs
             WHERE assignment_id=?1 AND campaign_id=?2 AND run_token=?3 AND attempt_id=?4
             AND udid=?5 AND phase=?6 AND revision=?7 AND state='running')",
            params![
                job.assignment_id,
                job.run.campaign_id,
                job.run.token,
                job.attempt_id,
                job.udid,
                job.phase,
                job.revision.saturating_add(1)
            ],
            |row| row.get(0),
        )?;
        // The existing settlement repeats the attempt/revision CAS inside its
        // transaction, so cancellation or another acknowledgement still wins.
        let changed = if current {
            self.finish_publish_dispatch_inner(
                &job,
                receipt
                    .failure
                    .as_ref()
                    .map(|failure| failure.message.as_str()),
                receipt.failure.as_ref(),
            )?
        } else {
            false
        };
        // The worker was joined before its completion was recorded. If the
        // permit's Drop hit the same SQLite fault, retry only this ended stage's
        // claim; a compose permit sharing a transfer attempt remains untouched.
        self.conn()?.execute(
            "DELETE FROM publish_work_claims WHERE owner=?1 AND udid=?2 AND stage=?3",
            params![job.attempt_id, job.udid, job.phase],
        )?;
        Ok(changed)
    }

    pub fn ensure_publish_recovery_ready(&self) -> anyhow::Result<()> {
        if let Some(error) = self.publish_recovery_error.read().as_ref() {
            anyhow::bail!("PublishRecoveryPending: Chưa đối soát được kết quả đăng trước khi mở ứng dụng ({error}). Chưa nhận bài mới; kiểm tra quyền ghi/dung lượng ổ đĩa. Hệ thống tự kiểm tra lại sau 30 giây, không đăng lại bài cũ.");
        }
        Ok(())
    }

    /// Bootstrap is retried by the existing dispatcher before it admits device
    /// work. A failed replay cannot be followed by a sweep that destroys its CAS.
    pub fn recover_publish_dispatch_startup(&self) -> anyhow::Result<usize> {
        *self.publish_recovery_error.write() = Some("đang phục hồi kết quả".into());
        let result = self
            .recover_publish_dispatch_completions()
            .and_then(|_| self.interrupt_orphaned_publish_campaigns());
        *self.publish_recovery_error.write() = result
            .as_ref()
            .err()
            .map(|error| format!("{error:#}").chars().take(512).collect());
        result
    }

    fn publish_completion_directory(&self) -> PathBuf {
        self.path.with_extension("publish-completions")
    }

    /// Immutable completion evidence is synced independently of SQLite before
    /// attempting its state transition. Repeating the same receipt is harmless.
    pub fn persist_publish_dispatch_completion(
        &self,
        job: &PublishDispatchJob,
        error: Option<&anyhow::Error>,
    ) -> anyhow::Result<()> {
        let receipt = CompletionReceipt::new(job, error);
        receipt.validate()?;
        let directory = self.publish_completion_directory();
        std::fs::create_dir_all(&directory)?;
        let path = directory.join(receipt.file_name());
        if path.exists() {
            anyhow::ensure!(
                read_receipt(&path)? == receipt,
                "dispatch completion identity changed"
            );
            return Ok(());
        }
        let bytes = serde_json::to_vec(&receipt)?;
        anyhow::ensure!(
            bytes.len() as u64 <= MAX_RECEIPT_BYTES,
            "dispatch completion exceeds size limit"
        );
        let temporary = directory.join(format!("{}.tmp", Uuid::new_v4()));
        let result = (|| -> anyhow::Result<()> {
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temporary)?;
            file.write_all(&bytes)?;
            file.sync_all()?;
            drop(file);
            if let Err(error) = std::fs::rename(&temporary, &path) {
                if !path.exists() || read_receipt(&path)? != receipt {
                    return Err(error.into());
                }
                std::fs::remove_file(&temporary)?;
            }
            #[cfg(unix)]
            std::fs::File::open(&directory)?.sync_all()?;
            Ok(())
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(&temporary);
        }
        result
    }

    /// A journal file is removed only after the database acknowledges the exact
    /// attempt/revision (including an already-settled stale acknowledgement).
    pub fn settle_publish_dispatch_completion(
        &self,
        job: &PublishDispatchJob,
        error: Option<&anyhow::Error>,
    ) -> anyhow::Result<bool> {
        let receipt = CompletionReceipt::new(job, error);
        if let Err(journal_error) = self.persist_publish_dispatch_completion(job, error) {
            if journal_error.downcast_ref::<std::io::Error>().is_none() {
                return Err(journal_error);
            }
            // A read-only journal directory must not block a healthy SQLite
            // settlement. Keep the caller's receipt when both stores fail.
            tracing::error!("publish completion journal unavailable: {journal_error:#}; attempting database settlement");
            return self.settle_completion_receipt(&receipt).map_err(|error| {
                error.context(format!(
                    "completion journal also unavailable: {journal_error:#}"
                ))
            });
        }
        let changed = self.settle_completion_receipt(&receipt)?;
        std::fs::remove_file(
            self.publish_completion_directory()
                .join(receipt.file_name()),
        )?;
        Ok(changed)
    }

    /// Run before the orphan sweep changes run tokens or revisions. Every entry
    /// is database-only; a malformed entry stays on disk and cannot launch work.
    pub fn recover_publish_dispatch_completions(&self) -> anyhow::Result<usize> {
        let directory = self.publish_completion_directory();
        if !directory.exists() {
            return Ok(0);
        }
        let mut recovered = 0;
        for entry in std::fs::read_dir(directory)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|value| value.to_str()) != Some("json") {
                continue;
            }
            anyhow::ensure!(
                recovered < MAX_STARTUP_ENTRIES,
                "publish completion recovery has more entries; continue next pass"
            );
            let result = (|| -> anyhow::Result<()> {
                let receipt = match read_receipt(&path) {
                    Ok(receipt) => receipt,
                    Err(error) if error.downcast_ref::<std::io::Error>().is_some() => {
                        return Err(error)
                    }
                    Err(error) => {
                        // Keep invalid evidence for diagnosis. It conveys no job
                        // ownership, so only the conservative orphan sweep may act.
                        let invalid = path.with_extension(format!("{}.invalid", Uuid::new_v4()));
                        std::fs::rename(&path, &invalid)?;
                        tracing::error!(
                            "invalid publish completion retained at {}: {error:#}",
                            invalid.display()
                        );
                        return Ok(());
                    }
                };
                self.settle_completion_receipt(&receipt)?;
                std::fs::remove_file(path)?;
                Ok(())
            })();
            match result {
                Ok(()) => recovered += 1,
                Err(error) => {
                    tracing::error!(
                        "publish completion recovery retained {}: {error:#}",
                        entry.file_name().to_string_lossy()
                    );
                    return Err(
                        error.context("publish completion recovery incomplete; receipts retained")
                    );
                }
            }
        }
        Ok(recovered)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dispatch_recovery_restart_keeps_failed_receipt_and_settles_same_post_once() {
        let (db, path, campaign, _) = super::super::super::publish_pipeline::tests::fixture();
        let run = db.claim_publish_pipeline(&campaign).unwrap().unwrap();
        let mut job = db.pending_publish_dispatch(10).unwrap().remove(0);
        db.init_publish_recovery(&job.assignment_id, &run.token)
            .unwrap();
        assert!(db.claim_publish_dispatch(&job, 0).unwrap());
        job.phase = "compose".into();
        let conn = Connection::open(&path).unwrap();
        let intent = r#"{"effectIntent":"post","expectedAccount":"fixture"}"#;
        conn.execute(
            "UPDATE publish_dispatch_jobs SET phase='compose' WHERE assignment_id=?1",
            [&job.assignment_id],
        )
        .unwrap();
        conn.execute("UPDATE publish_assignments SET state='verifying',effect_intent=?2,evidence_json=?2 WHERE id=?1",
            params![job.assignment_id, intent]).unwrap();
        conn.execute_batch("CREATE TRIGGER completion_fault BEFORE UPDATE OF state ON publish_dispatch_jobs BEGIN SELECT RAISE(FAIL,'fixture DB unavailable'); END;").unwrap();
        // Preserve the leaked claim from an ended worker whose permit Drop also
        // encountered the DB fault; this is never a live device lease.
        conn.execute(
            "INSERT INTO publish_work_claims VALUES('ended-claim',?1,'compose',?2,0)",
            params![job.udid, job.attempt_id],
        )
        .unwrap();
        let error: anyhow::Error = RecoveryFailure::new(
            "fixture_after_post",
            crate::publish_recovery::FailureKind::Retryable,
            "response lost",
        )
        .into();
        assert!(db
            .settle_publish_dispatch_completion(&job, Some(&error))
            .is_err());
        let directory = db.publish_completion_directory();
        let receipt_path = directory.join(CompletionReceipt::new(&job, Some(&error)).file_name());
        let saved = std::fs::read(&receipt_path).unwrap();
        drop(db);

        let db = Database::open(&path).unwrap();
        assert!(db.recover_publish_dispatch_startup().is_err());
        assert!(db
            .ensure_publish_recovery_ready()
            .unwrap_err()
            .to_string()
            .contains("PublishRecoveryPending"));
        assert!(db.claim_publish_pipeline(&campaign).is_err());
        assert_eq!(std::fs::read(&receipt_path).unwrap(), saved);
        assert_eq!(
            conn.query_row(
                "SELECT COUNT(*) FROM publish_work_claims WHERE token='ended-claim'",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
            1
        );
        assert!(
            db.publish_pipeline_current(&run).unwrap(),
            "failed replay must not run the orphan sweep"
        );
        conn.execute_batch("DROP TRIGGER completion_fault;")
            .unwrap();
        assert_eq!(db.recover_publish_dispatch_completions().unwrap(), 1);
        assert_eq!(db.recover_publish_dispatch_completions().unwrap(), 0);
        assert_eq!(
            conn.query_row(
                "SELECT COUNT(*) FROM publish_work_claims WHERE token='ended-claim'",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
            0
        );
        let row: (String, String, i64, String) = conn.query_row(
            "SELECT state,attempt_id,revision,reason FROM publish_dispatch_jobs WHERE assignment_id=?1",
            [&job.assignment_id], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        ).unwrap();
        assert_eq!(
            row,
            (
                "finished".into(),
                job.attempt_id.clone(),
                job.revision + 2,
                "response lost".into()
            )
        );
        let before = db
            .get_publish_assignment_detail(&campaign, &job.assignment_id)
            .unwrap()
            .unwrap();
        assert_eq!(before.assignments[0].effect_intent.as_deref(), Some(intent));
        assert_eq!(
            before.assignments[0].state,
            crate::PublishCampaignState::Verifying
        );
        assert_eq!(
            conn.query_row(
                "SELECT COUNT(*) FROM publish_attempts WHERE publication_id=?1",
                [&before.assignments[0].publication_id],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
            1
        );
        db.recover_publish_dispatch_startup().unwrap();
        db.ensure_publish_recovery_ready().unwrap();
        assert!(!receipt_path.exists());
        drop(conn);
        drop(db);
        let _ = std::fs::remove_dir(&directory);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn dispatch_recovery_corrupt_receipt_is_retained_without_authorizing_work() {
        let path = std::env::temp_dir().join(format!("completion-invalid-{}.db", Uuid::new_v4()));
        let db = Database::open(&path).unwrap();
        let directory = db.publish_completion_directory();
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(directory.join("bad.json"), b"{not a receipt").unwrap();
        db.recover_publish_dispatch_startup().unwrap();
        let files: Vec<_> = std::fs::read_dir(&directory)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect();
        assert_eq!(files.len(), 1);
        assert_eq!(
            files[0].extension().and_then(|value| value.to_str()),
            Some("invalid")
        );
        assert_eq!(std::fs::read(&files[0]).unwrap(), b"{not a receipt");
        assert!(db.pending_publish_dispatch(10).unwrap().is_empty());
        std::fs::remove_file(&files[0]).unwrap();
        std::fs::remove_dir(directory).unwrap();
        drop(db);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn dispatch_recovery_journal_failure_still_allows_healthy_database_settlement() {
        let (db, path, campaign, _) = super::super::super::publish_pipeline::tests::fixture();
        db.claim_publish_pipeline(&campaign).unwrap().unwrap();
        let job = db.pending_publish_dispatch(10).unwrap().remove(0);
        assert!(db.claim_publish_dispatch(&job, 0).unwrap());
        let directory = db.publish_completion_directory();
        // A file at the journal directory forces a real filesystem failure,
        // while the scratch SQLite database remains writable.
        std::fs::write(&directory, b"fixture journal unavailable").unwrap();
        assert!(db.settle_publish_dispatch_completion(&job, None).unwrap());
        let conn = Connection::open(&path).unwrap();
        let (state, phase): (String, String) = conn
            .query_row(
                "SELECT state,phase FROM publish_dispatch_jobs WHERE assignment_id=?1",
                [&job.assignment_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!((state.as_str(), phase.as_str()), ("queued", "compose"));
        // Transfer and compose reuse an attempt identity. A repeated transfer
        // completion must not delete the newly admitted compose stage's claim.
        let compose = db
            .try_publish_work(&job.udid, "compose", &job.attempt_id)
            .unwrap()
            .unwrap();
        assert!(!db.settle_publish_dispatch_completion(&job, None).unwrap());
        assert_eq!(
            conn.query_row(
                "SELECT COUNT(*) FROM publish_work_claims WHERE owner=?1 AND stage='compose'",
                [&job.attempt_id],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
            1
        );
        drop(compose);
        drop(conn);
        drop(db);
        std::fs::remove_file(directory).unwrap();
        let _ = std::fs::remove_file(path);
    }
}
