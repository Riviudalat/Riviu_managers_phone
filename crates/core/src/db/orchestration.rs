use std::collections::{BTreeMap, HashSet};

use anyhow::{ensure, Context};
use chrono::{SecondsFormat, Utc};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use super::Database;
use crate::{
    canonical_orchestration_document_json, orchestration_revision_sha256, AutomationKind,
    ChildCampaignOutcome, CompiledOrchestrationV1, OrchestrationAttemptRecord,
    OrchestrationAttemptSnapshot, OrchestrationAttemptState, OrchestrationBranch,
    OrchestrationNodeAction, OrchestrationNotFound, OrchestrationNurtureChildRecord,
    OrchestrationNurtureChildState, OrchestrationRevisionConflict, OrchestrationRevisionRecord,
    OrchestrationRunDetail, OrchestrationRunRecord, OrchestrationRunState, OrchestrationSummary,
    ResolvedTargetSnapshot,
};

impl Database {
    pub fn save_orchestration_revision(
        &self,
        expected_revision: Option<u64>,
        compiled: &CompiledOrchestrationV1,
    ) -> anyhow::Result<OrchestrationRevisionRecord> {
        validate_compiled(compiled)?;
        let mut connection = self.conn()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let id = compiled.document.id;
        let current: Option<i64> = transaction
            .query_row(
                "SELECT latest_revision FROM orchestration_documents WHERE id=?1",
                [id.to_string()],
                |row| row.get(0),
            )
            .optional()?;
        let actual = current
            .map(|value| sql_to_revision(value, "orchestration_documents.latest_revision"))
            .transpose()?
            .unwrap_or(0);
        match (current.is_some(), expected_revision) {
            (false, None) => {}
            (false, Some(expected)) => {
                return Err(OrchestrationRevisionConflict {
                    expected,
                    actual: 0,
                }
                .into());
            }
            (true, Some(expected)) if expected == actual => {}
            (true, Some(expected)) => {
                return Err(OrchestrationRevisionConflict { expected, actual }.into());
            }
            (true, None) => {
                return Err(OrchestrationRevisionConflict {
                    expected: 0,
                    actual,
                }
                .into());
            }
        }
        let next = actual
            .checked_add(1)
            .context("orchestration revision overflow")?;
        ensure!(
            compiled.document.revision == next,
            "orchestration document revision {} must equal next revision {next}",
            compiled.document.revision
        );
        let now = timestamp();
        if current.is_none() {
            transaction.execute(
                "INSERT INTO orchestration_documents(
                    id,name,latest_revision,archived,created_at,updated_at
                 ) VALUES(?1,?2,?3,0,?4,?4)",
                params![
                    id.to_string(),
                    compiled.document.name,
                    revision_to_sql(next)?,
                    now
                ],
            )?;
        } else {
            let changed = transaction.execute(
                "UPDATE orchestration_documents
                 SET name=?2,latest_revision=?3,updated_at=?4
                 WHERE id=?1 AND latest_revision=?5 AND archived=0",
                params![
                    id.to_string(),
                    compiled.document.name,
                    revision_to_sql(next)?,
                    now,
                    revision_to_sql(actual)?,
                ],
            )?;
            ensure!(changed == 1, "orchestration changed or is archived");
        }
        transaction.execute(
            "INSERT INTO orchestration_revisions(
                document_id,revision,compiled_json,canonical_json,document_sha256,created_at
             ) VALUES(?1,?2,?3,?4,?5,?6)",
            params![
                id.to_string(),
                revision_to_sql(next)?,
                serde_json::to_string(compiled).context("serialize orchestration revision")?,
                compiled.canonical_json,
                compiled.sha256,
                now,
            ],
        )?;
        transaction.commit()?;
        self.get_orchestration_revision(id, Some(next))?
            .context("saved orchestration revision disappeared")
    }

    pub fn list_orchestrations(
        &self,
        include_archived: bool,
    ) -> anyhow::Result<Vec<OrchestrationSummary>> {
        let connection = self.conn()?;
        let mut statement = connection.prepare(
            "SELECT id,name,latest_revision,archived,updated_at
             FROM orchestration_documents
             WHERE (?1=1 OR archived=0)
             ORDER BY updated_at DESC,id ASC",
        )?;
        let rows = statement.query_map([i64::from(include_archived)], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, bool>(3)?,
                row.get::<_, String>(4)?,
            ))
        })?;
        rows.map(|row| {
            let (id, name, latest_revision, archived, updated_at) = row?;
            Ok(OrchestrationSummary {
                id: Uuid::parse_str(&id).context("parse orchestration ID")?,
                name,
                latest_revision: sql_to_revision(
                    latest_revision,
                    "orchestration_documents.latest_revision",
                )?,
                archived,
                updated_at,
            })
        })
        .collect()
    }

    pub fn get_orchestration_revision(
        &self,
        id: Uuid,
        revision: Option<u64>,
    ) -> anyhow::Result<Option<OrchestrationRevisionRecord>> {
        let revision = revision.map(revision_to_sql).transpose()?;
        let connection = self.conn()?;
        let row = connection
            .query_row(
                "SELECT r.revision,r.compiled_json,r.canonical_json,r.document_sha256,r.created_at
                 FROM orchestration_revisions r
                 JOIN orchestration_documents d ON d.id=r.document_id
                 WHERE r.document_id=?1 AND r.revision=COALESCE(?2,d.latest_revision)",
                params![id.to_string(), revision],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                    ))
                },
            )
            .optional()?;
        row.map(
            |(revision, compiled_json, canonical_json, sha256, created_at)| {
                let compiled: CompiledOrchestrationV1 = serde_json::from_str(&compiled_json)
                    .context("parse persisted orchestration revision")?;
                ensure!(
                    compiled.document.id == id,
                    "orchestration revision ID mismatch"
                );
                ensure!(
                    compiled.document.revision
                        == sql_to_revision(revision, "orchestration_revisions.revision")?,
                    "orchestration revision number mismatch"
                );
                ensure!(
                    compiled.canonical_json == canonical_json && compiled.sha256 == sha256,
                    "orchestration revision metadata mismatch"
                );
                validate_compiled(&compiled)?;
                Ok(OrchestrationRevisionRecord {
                    compiled,
                    created_at,
                })
            },
        )
        .transpose()
    }

    pub fn archive_orchestration(&self, id: Uuid) -> anyhow::Result<()> {
        let changed = self.conn()?.execute(
            "UPDATE orchestration_documents SET archived=1,updated_at=?2 WHERE id=?1",
            params![id.to_string(), timestamp()],
        )?;
        if changed == 0 {
            return Err(OrchestrationNotFound { document_id: id }.into());
        }
        Ok(())
    }

    pub fn create_orchestration_run(
        &self,
        document_id: Uuid,
        document_revision: u64,
        target: &ResolvedTargetSnapshot,
        node_targets: &BTreeMap<Uuid, ResolvedTargetSnapshot>,
    ) -> anyhow::Result<OrchestrationRunRecord> {
        validate_target(target)?;
        let revision = self
            .get_orchestration_revision(document_id, Some(document_revision))?
            .ok_or(OrchestrationNotFound { document_id })?;
        validate_node_targets(&revision.compiled, node_targets)?;
        for node in &revision.compiled.document.nodes {
            let (profile, expected_kind) = match &node.action {
                OrchestrationNodeAction::RunNurture { profile, .. } => {
                    (profile, AutomationKind::Nurture)
                }
                OrchestrationNodeAction::RunInteraction { profile, .. } => {
                    (profile, AutomationKind::Interaction)
                }
                OrchestrationNodeAction::RunPublish { profile, .. } => {
                    (profile, AutomationKind::Publish)
                }
                OrchestrationNodeAction::Start
                | OrchestrationNodeAction::Delay { .. }
                | OrchestrationNodeAction::End => continue,
            };
            let record = self
                .get_automation_definition_record(profile.definition_id, profile.revision)
                .with_context(|| {
                    format!(
                        "ProfileRevisionMissing: profile {} revision {}",
                        profile.definition_id, profile.revision
                    )
                })?
                .with_context(|| {
                    format!(
                        "ProfileRevisionMissing: profile {} revision {}",
                        profile.definition_id, profile.revision
                    )
                })?;
            ensure!(
                !record.definition.archived,
                "ProfileArchived: profile {}",
                profile.definition_id
            );
            ensure!(
                record.definition.kind == expected_kind,
                "ProfileKindMismatch: profile {}",
                profile.definition_id
            );
        }
        let id = Uuid::new_v4();
        let now = timestamp();
        self.conn()?.execute(
            "INSERT INTO orchestration_runs(
                id,document_id,document_revision,document_sha256,target_json,node_targets_json,
                state,current_node_id,error_code,created_at,updated_at
             ) VALUES(?1,?2,?3,?4,?5,?6,'queued',NULL,NULL,?7,?7)",
            params![
                id.to_string(),
                document_id.to_string(),
                revision_to_sql(document_revision)?,
                revision.compiled.sha256,
                serde_json::to_string(target).context("serialize orchestration target")?,
                serde_json::to_string(node_targets)
                    .context("serialize orchestration node targets")?,
                now,
            ],
        )?;
        query_run(&self.conn()?, id)?.context("created orchestration run disappeared")
    }

    pub fn create_orchestration_attempt(
        &self,
        run_id: Uuid,
        attempt_no: u32,
        snapshot: &OrchestrationAttemptSnapshot,
    ) -> anyhow::Result<OrchestrationAttemptRecord> {
        ensure!(
            attempt_no >= 1,
            "orchestration attempt number must be at least one"
        );
        let run = query_run(&self.conn()?, run_id)?.context("orchestration run does not exist")?;
        ensure!(
            !run.state.is_terminal(),
            "terminal orchestration run cannot create an attempt"
        );
        ensure!(
            snapshot.document_id == run.document_id
                && snapshot.document_revision == run.document_revision
                && snapshot.document_sha256 == run.document_sha256,
            "orchestration attempt snapshot does not match its run"
        );
        let revision = self
            .get_orchestration_revision(run.document_id, Some(run.document_revision))?
            .context("orchestration run revision is missing")?;
        ensure!(
            snapshot.canonical_document_json == revision.compiled.canonical_json,
            "orchestration attempt canonical document mismatch"
        );
        let node = revision
            .compiled
            .document
            .nodes
            .iter()
            .find(|node| node.id == snapshot.node_id)
            .context("orchestration attempt node does not exist")?;
        match node.action.target_override() {
            Some(_) => ensure!(
                run.node_targets.get(&node.id) == Some(&snapshot.target),
                "orchestration attempt does not use its confirmed node target snapshot"
            ),
            None => ensure!(
                snapshot.target == run.target,
                "orchestration attempt target differs from its run"
            ),
        }
        validate_target(&snapshot.target)?;
        ensure!(
            snapshot.profile == node.action.profile().cloned(),
            "orchestration attempt profile mismatch"
        );
        validate_snapshot_key(snapshot)?;
        let now = timestamp();
        self.conn()?.execute(
            "INSERT INTO orchestration_attempts(
                id,run_id,node_id,attempt_no,idempotency_key,snapshot_json,state,
                child_kind,child_campaign_id,branch,error_code,created_at,updated_at
             ) VALUES(?1,?2,?3,?4,?5,?6,'queued',NULL,NULL,NULL,NULL,?7,?7)",
            params![
                snapshot.attempt_id.to_string(),
                run_id.to_string(),
                snapshot.node_id.to_string(),
                i64::from(attempt_no),
                snapshot.idempotency_key,
                serde_json::to_string(snapshot).context("serialize orchestration attempt")?,
                now,
            ],
        )?;
        query_attempt(&self.conn()?, snapshot.attempt_id)?
            .context("created orchestration attempt disappeared")
    }

    /// Persist the orchestration-owned identity before Nurture can cross an effect boundary.
    /// A replay with the exact attempt, child ID and key returns the existing row; any aliasing
    /// of one of those unique identities fails closed.
    pub fn create_orchestration_nurture_child(
        &self,
        attempt_id: Uuid,
        child_id: Uuid,
        idempotency_key: &str,
    ) -> anyhow::Result<(OrchestrationNurtureChildRecord, bool)> {
        let mut connection = self.conn()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let attempt = query_attempt(&transaction, attempt_id)?
            .context("orchestration nurture attempt does not exist")?;
        ensure!(
            attempt.child_kind == Some(AutomationKind::Nurture)
                && attempt.child_campaign_id == Some(child_id)
                && matches!(
                    attempt.state,
                    OrchestrationAttemptState::Dispatching
                        | OrchestrationAttemptState::WaitingChild
                ),
            "orchestration nurture child does not match its armed attempt"
        );
        ensure!(
            attempt.snapshot.idempotency_key == idempotency_key,
            "orchestration nurture child idempotency key mismatch"
        );
        let requested_udids = attempt
            .snapshot
            .target
            .included
            .iter()
            .map(|device| device.udid.clone())
            .collect::<Vec<_>>();
        validate_udids(&requested_udids, "requested")?;

        let existing_id = transaction
            .query_row(
                "SELECT id FROM orchestration_nurture_children
                 WHERE id=?1 OR attempt_id=?2 OR idempotency_key=?3 LIMIT 1",
                params![
                    child_id.to_string(),
                    attempt_id.to_string(),
                    idempotency_key
                ],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        if let Some(existing_id) = existing_id {
            let existing_id = Uuid::parse_str(&existing_id)
                .context("parse existing orchestration nurture child ID")?;
            let existing = query_nurture_child(&transaction, existing_id)?
                .context("orchestration nurture child disappeared")?;
            ensure!(
                existing.id == child_id
                    && existing.attempt_id == attempt_id
                    && existing.idempotency_key == idempotency_key
                    && existing.requested_udids == requested_udids,
                "orchestration nurture child identity conflict"
            );
            transaction.commit()?;
            return Ok((existing, false));
        }

        let now = timestamp();
        transaction.execute(
            "INSERT INTO orchestration_nurture_children(
                id,attempt_id,idempotency_key,run_id,requested_udids_json,
                started_udids_json,state,created_at,updated_at
             ) VALUES(?1,?2,?3,NULL,?4,'[]','dispatching',?5,?5)",
            params![
                child_id.to_string(),
                attempt_id.to_string(),
                idempotency_key,
                serde_json::to_string(&requested_udids)
                    .context("serialize nurture requested devices")?,
                now,
            ],
        )?;
        let child = query_nurture_child(&transaction, child_id)?
            .context("created orchestration nurture child disappeared")?;
        transaction.commit()?;
        Ok((child, true))
    }

    pub fn start_orchestration_nurture_child(
        &self,
        child_id: Uuid,
        idempotency_key: &str,
        run_id: Uuid,
        started_udids: &[String],
    ) -> anyhow::Result<OrchestrationNurtureChildRecord> {
        validate_udids(started_udids, "started")?;
        let connection = self.conn()?;
        let current = query_nurture_child(&connection, child_id)?
            .context("orchestration nurture child does not exist")?;
        ensure!(
            current.idempotency_key == idempotency_key,
            "orchestration nurture child idempotency conflict"
        );
        let requested = current
            .requested_udids
            .iter()
            .map(String::as_str)
            .collect::<HashSet<_>>();
        ensure!(
            started_udids
                .iter()
                .all(|udid| requested.contains(udid.as_str())),
            "orchestration nurture child started outside its confirmed target"
        );
        if current.state != OrchestrationNurtureChildState::Dispatching {
            ensure!(
                current.run_id == Some(run_id) && current.started_udids == started_udids,
                "orchestration nurture child start identity conflict"
            );
            return Ok(current);
        }
        let changed = connection.execute(
            "UPDATE orchestration_nurture_children
             SET run_id=?3,started_udids_json=?4,state='running',updated_at=?5
             WHERE id=?1 AND idempotency_key=?2 AND state='dispatching'",
            params![
                child_id.to_string(),
                idempotency_key,
                run_id.to_string(),
                serde_json::to_string(started_udids)
                    .context("serialize nurture started devices")?,
                timestamp(),
            ],
        )?;
        ensure!(changed == 1, "orchestration nurture child start raced");
        query_nurture_child(&connection, child_id)?
            .context("started orchestration nurture child disappeared")
    }

    pub fn settle_orchestration_nurture_child(
        &self,
        child_id: Uuid,
        idempotency_key: &str,
        outcome: ChildCampaignOutcome,
    ) -> anyhow::Result<OrchestrationNurtureChildRecord> {
        let next = nurture_state_for_outcome(outcome);
        let connection = self.conn()?;
        let changed = connection.execute(
            "UPDATE orchestration_nurture_children
             SET state=?3,updated_at=?4
             WHERE id=?1 AND idempotency_key=?2 AND state IN ('dispatching','running')",
            params![
                child_id.to_string(),
                idempotency_key,
                nurture_state_label(next),
                timestamp(),
            ],
        )?;
        let child = query_nurture_child(&connection, child_id)?
            .context("orchestration nurture child does not exist")?;
        ensure!(
            child.idempotency_key == idempotency_key,
            "orchestration nurture child idempotency conflict"
        );
        ensure!(
            changed == 1 || child.state == next,
            "orchestration nurture child already has a different terminal outcome"
        );
        Ok(child)
    }

    pub fn get_orchestration_nurture_child(
        &self,
        child_id: Uuid,
    ) -> anyhow::Result<Option<OrchestrationNurtureChildRecord>> {
        query_nurture_child(&self.conn()?, child_id)
    }

    pub fn arm_orchestration_child(
        &self,
        attempt_id: Uuid,
        child_kind: AutomationKind,
        child_campaign_id: Uuid,
    ) -> anyhow::Result<Option<OrchestrationAttemptRecord>> {
        let current = query_attempt(&self.conn()?, attempt_id)?
            .context("orchestration attempt does not exist")?;
        ensure!(
            expected_child_kind(&current)? == Some(child_kind),
            "orchestration child kind does not match the pinned node"
        );
        let connection = self.conn()?;
        let changed = connection.execute(
            "UPDATE orchestration_attempts
             SET state='dispatching',child_kind=?2,child_campaign_id=?3,updated_at=?4
             WHERE id=?1 AND state='queued' AND child_campaign_id IS NULL",
            params![
                attempt_id.to_string(),
                child_kind.as_str(),
                child_campaign_id.to_string(),
                timestamp(),
            ],
        )?;
        if changed == 0 {
            return Ok(None);
        }
        query_attempt(&connection, attempt_id)
    }

    pub fn mark_orchestration_child_started(
        &self,
        attempt_id: Uuid,
        child_campaign_id: Uuid,
    ) -> anyhow::Result<Option<OrchestrationAttemptRecord>> {
        let connection = self.conn()?;
        let changed = connection.execute(
            "UPDATE orchestration_attempts SET state='waiting_child',updated_at=?3
             WHERE id=?1 AND state='dispatching' AND child_campaign_id=?2",
            params![
                attempt_id.to_string(),
                child_campaign_id.to_string(),
                timestamp()
            ],
        )?;
        if changed == 0 {
            return Ok(None);
        }
        query_attempt(&connection, attempt_id)
    }

    pub fn settle_orchestration_child(
        &self,
        attempt_id: Uuid,
        child_campaign_id: Uuid,
        branch: OrchestrationBranch,
        error_code: Option<&str>,
    ) -> anyhow::Result<Option<OrchestrationAttemptRecord>> {
        let state = attempt_state_for_branch(branch);
        let connection = self.conn()?;
        let changed = connection.execute(
            "UPDATE orchestration_attempts
             SET state=?3,branch=?4,error_code=?5,updated_at=?6
             WHERE id=?1 AND child_campaign_id=?2 AND state IN ('dispatching','waiting_child')",
            params![
                attempt_id.to_string(),
                child_campaign_id.to_string(),
                attempt_state_label(state),
                branch_label(branch),
                error_code,
                timestamp(),
            ],
        )?;
        if changed == 0 {
            return Ok(None);
        }
        query_attempt(&connection, attempt_id)
    }

    /// Settle a Start, Delay, End, or a campaign node rejected before a child was armed.
    pub fn settle_orchestration_node(
        &self,
        attempt_id: Uuid,
        branch: OrchestrationBranch,
        error_code: Option<&str>,
    ) -> anyhow::Result<Option<OrchestrationAttemptRecord>> {
        let state = attempt_state_for_branch(branch);
        let connection = self.conn()?;
        let changed = connection.execute(
            "UPDATE orchestration_attempts
             SET state=?2,branch=?3,error_code=?4,updated_at=?5
             WHERE id=?1 AND state='queued' AND child_campaign_id IS NULL",
            params![
                attempt_id.to_string(),
                attempt_state_label(state),
                branch_label(branch),
                error_code,
                timestamp(),
            ],
        )?;
        if changed == 0 {
            return Ok(None);
        }
        query_attempt(&connection, attempt_id)
    }

    /// Cancel only an attempt whose effect boundary is known not to have been crossed.
    ///
    /// A child ID is deliberately retained when one was armed: it is the durable proof that
    /// a later restart must reconcile that exact child rather than create a replacement.
    pub fn cancel_orchestration_attempt_before_effect(
        &self,
        attempt_id: Uuid,
    ) -> anyhow::Result<Option<OrchestrationAttemptRecord>> {
        let connection = self.conn()?;
        let changed = connection.execute(
            "UPDATE orchestration_attempts
             SET state='cancelled',updated_at=?2
             WHERE id=?1 AND state='queued' AND child_campaign_id IS NULL AND branch IS NULL",
            params![attempt_id.to_string(), timestamp()],
        )?;
        if changed == 0 {
            return Ok(None);
        }
        query_attempt(&connection, attempt_id)
    }

    /// Apply an adapter's positive proof that an already-armed child never crossed its effect
    /// boundary. The child identity is part of the CAS so a stale cancellation cannot erase a
    /// different or concurrently completed child.
    pub fn cancel_orchestration_child_before_effect(
        &self,
        attempt_id: Uuid,
        child_campaign_id: Uuid,
    ) -> anyhow::Result<Option<OrchestrationAttemptRecord>> {
        let connection = self.conn()?;
        let changed = connection.execute(
            "UPDATE orchestration_attempts
             SET state='cancelled',updated_at=?3
             WHERE id=?1 AND child_campaign_id=?2
               AND state IN ('dispatching','waiting_child') AND branch IS NULL",
            params![
                attempt_id.to_string(),
                child_campaign_id.to_string(),
                timestamp()
            ],
        )?;
        if changed == 0 {
            return Ok(None);
        }
        query_attempt(&connection, attempt_id)
    }

    pub fn get_orchestration_attempt(
        &self,
        attempt_id: Uuid,
    ) -> anyhow::Result<Option<OrchestrationAttemptRecord>> {
        query_attempt(&self.conn()?, attempt_id)
    }

    pub fn list_recoverable_orchestration_attempts(
        &self,
    ) -> anyhow::Result<Vec<OrchestrationAttemptRecord>> {
        let connection = self.conn()?;
        let mut statement = connection.prepare(
            "SELECT id FROM orchestration_attempts
             WHERE state IN ('dispatching','waiting_child') ORDER BY updated_at,id",
        )?;
        let ids = statement
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        ids.into_iter()
            .map(|id| {
                let id = Uuid::parse_str(&id).context("parse orchestration attempt ID")?;
                query_attempt(&connection, id)?.context("recoverable attempt disappeared")
            })
            .collect()
    }

    pub fn get_orchestration_run(
        &self,
        run_id: Uuid,
    ) -> anyhow::Result<Option<OrchestrationRunDetail>> {
        let connection = self.conn()?;
        let Some(run) = query_run(&connection, run_id)? else {
            return Ok(None);
        };
        let mut statement = connection.prepare(
            "SELECT id FROM orchestration_attempts WHERE run_id=?1 ORDER BY created_at,id",
        )?;
        let ids = statement
            .query_map([run_id.to_string()], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        let attempts = ids
            .into_iter()
            .map(|id| {
                let id = Uuid::parse_str(&id).context("parse orchestration attempt ID")?;
                query_attempt(&connection, id)?.context("orchestration attempt disappeared")
            })
            .collect::<anyhow::Result<Vec<_>>>()?;
        Ok(Some(OrchestrationRunDetail { run, attempts }))
    }

    pub fn list_orchestration_runs(
        &self,
        limit: usize,
    ) -> anyhow::Result<Vec<OrchestrationRunRecord>> {
        ensure!(
            (1..=200).contains(&limit),
            "orchestration run limit must be 1..=200"
        );
        let connection = self.conn()?;
        let mut statement = connection.prepare(
            "SELECT id FROM orchestration_runs ORDER BY updated_at DESC,id DESC LIMIT ?1",
        )?;
        let ids = statement
            .query_map([i64::try_from(limit)?], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        ids.into_iter()
            .map(|id| {
                let id = Uuid::parse_str(&id).context("parse orchestration run ID")?;
                query_run(&connection, id)?.context("orchestration run disappeared")
            })
            .collect()
    }

    /// Return every nonterminal run for process-start recovery.
    ///
    /// This intentionally has no UI page limit: leaving an older queued or running row out of
    /// the bootstrap scan would strand it permanently after a restart.
    pub fn list_recoverable_orchestration_runs(
        &self,
    ) -> anyhow::Result<Vec<OrchestrationRunRecord>> {
        let connection = self.conn()?;
        let mut statement = connection.prepare(
            "SELECT id FROM orchestration_runs
             WHERE state IN ('queued','running') ORDER BY updated_at,id",
        )?;
        let ids = statement
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        ids.into_iter()
            .map(|id| {
                let id = Uuid::parse_str(&id).context("parse recoverable orchestration run ID")?;
                query_run(&connection, id)?.context("recoverable orchestration run disappeared")
            })
            .collect()
    }

    pub fn transition_orchestration_run(
        &self,
        run_id: Uuid,
        expected: OrchestrationRunState,
        next: OrchestrationRunState,
        current_node_id: Option<Uuid>,
        error_code: Option<&str>,
    ) -> anyhow::Result<Option<OrchestrationRunRecord>> {
        let legal = matches!(
            (expected, next),
            (
                OrchestrationRunState::Queued,
                OrchestrationRunState::Running
            ) | (
                OrchestrationRunState::Running,
                OrchestrationRunState::Done
                    | OrchestrationRunState::Partial
                    | OrchestrationRunState::Failed
                    | OrchestrationRunState::Uncertain
                    | OrchestrationRunState::Cancelled
            )
        );
        ensure!(legal, "invalid orchestration run transition");
        let current =
            query_run(&self.conn()?, run_id)?.context("orchestration run does not exist")?;
        if let Some(node_id) = current_node_id {
            let revision = self
                .get_orchestration_revision(current.document_id, Some(current.document_revision))?
                .context("orchestration run revision is missing")?;
            ensure!(
                revision
                    .compiled
                    .document
                    .nodes
                    .iter()
                    .any(|node| node.id == node_id),
                "orchestration current node is outside the pinned document"
            );
        }
        let connection = self.conn()?;
        let changed = connection.execute(
            "UPDATE orchestration_runs
             SET state=?3,current_node_id=?4,error_code=?5,updated_at=?6
             WHERE id=?1 AND state=?2
               AND NOT (state='queued' AND ?3='running'
                        AND COALESCE(error_code,'')='cancel_requested')",
            params![
                run_id.to_string(),
                run_state_label(expected),
                run_state_label(next),
                current_node_id.map(|id| id.to_string()),
                error_code,
                timestamp(),
            ],
        )?;
        if changed == 0 {
            return Ok(None);
        }
        query_run(&connection, run_id)
    }

    /// Advance the fleet graph without changing its running state.
    pub fn advance_orchestration_run(
        &self,
        run_id: Uuid,
        expected_node_id: Uuid,
        next_node_id: Uuid,
    ) -> anyhow::Result<Option<OrchestrationRunRecord>> {
        let current =
            query_run(&self.conn()?, run_id)?.context("orchestration run does not exist")?;
        ensure!(
            current.state == OrchestrationRunState::Running,
            "only a running orchestration can advance"
        );
        let revision = self
            .get_orchestration_revision(current.document_id, Some(current.document_revision))?
            .context("orchestration run revision is missing")?;
        ensure!(
            revision
                .compiled
                .document
                .nodes
                .iter()
                .any(|node| node.id == next_node_id),
            "orchestration next node is outside the pinned document"
        );
        let connection = self.conn()?;
        let changed = connection.execute(
            "UPDATE orchestration_runs
             SET current_node_id=?3,
                 error_code=CASE WHEN error_code='cancel_requested' THEN error_code ELSE NULL END,
                 updated_at=?4
             WHERE id=?1 AND state='running' AND current_node_id=?2",
            params![
                run_id.to_string(),
                expected_node_id.to_string(),
                next_node_id.to_string(),
                timestamp(),
            ],
        )?;
        if changed == 0 {
            return Ok(None);
        }
        query_run(&connection, run_id)
    }

    /// Persist operator intent before waiting for the worker's per-run operation lock. A queued
    /// marker prevents startup from crossing into `running`; a running marker lets recovery
    /// reconcile an ambiguous child without advancing to another campaign node.
    pub fn request_orchestration_cancel(
        &self,
        run_id: Uuid,
    ) -> anyhow::Result<Option<OrchestrationRunRecord>> {
        let connection = self.conn()?;
        let changed = connection.execute(
            "UPDATE orchestration_runs
             SET error_code='cancel_requested',updated_at=?2
             WHERE id=?1 AND state IN ('queued','running')",
            params![run_id.to_string(), timestamp()],
        )?;
        if changed == 0 {
            return Ok(None);
        }
        query_run(&connection, run_id)
    }

    /// Cancel a queued or running graph with a CAS. Callers must first prove that no active
    /// child can still cross an effect boundary.
    pub fn cancel_orchestration_run(
        &self,
        run_id: Uuid,
        expected: OrchestrationRunState,
        expected_node_id: Option<Uuid>,
    ) -> anyhow::Result<Option<OrchestrationRunRecord>> {
        ensure!(
            matches!(
                expected,
                OrchestrationRunState::Queued | OrchestrationRunState::Running
            ),
            "only a queued or running orchestration can be cancelled"
        );
        let connection = self.conn()?;
        let changed = connection.execute(
            "UPDATE orchestration_runs
             SET state='cancelled',error_code=NULL,updated_at=?4
             WHERE id=?1 AND state=?2
               AND ((?3 IS NULL AND current_node_id IS NULL) OR current_node_id=?3)",
            params![
                run_id.to_string(),
                run_state_label(expected),
                expected_node_id.map(|id| id.to_string()),
                timestamp(),
            ],
        )?;
        if changed == 0 {
            return Ok(None);
        }
        query_run(&connection, run_id)
    }
}

fn validate_compiled(compiled: &CompiledOrchestrationV1) -> anyhow::Result<()> {
    let canonical = canonical_orchestration_document_json(&compiled.document)?;
    ensure!(
        canonical == compiled.canonical_json,
        "orchestration canonical document mismatch"
    );
    let sha256 = orchestration_revision_sha256(&canonical, &compiled.profiles)?;
    ensure!(sha256 == compiled.sha256, "orchestration SHA-256 mismatch");
    for node in &compiled.document.nodes {
        ensure!(
            compiled.profiles.get(&node.id) == node.action.profile(),
            "orchestration profile index mismatch"
        );
    }
    Ok(())
}

fn validate_target(target: &ResolvedTargetSnapshot) -> anyhow::Result<()> {
    ensure!(
        target.roster_sha256.len() == 64
            && target
                .roster_sha256
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit()),
        "orchestration target roster hash is invalid"
    );
    ensure!(
        !target.included.is_empty(),
        "orchestration target has no eligible device"
    );
    let mut udids = std::collections::HashSet::new();
    ensure!(
        target
            .included
            .iter()
            .all(|device| udids.insert(&device.udid)),
        "orchestration target contains duplicate devices"
    );
    Ok(())
}

fn validate_node_targets(
    compiled: &CompiledOrchestrationV1,
    node_targets: &BTreeMap<Uuid, ResolvedTargetSnapshot>,
) -> anyhow::Result<()> {
    let expected = compiled
        .document
        .nodes
        .iter()
        .filter_map(|node| {
            node.action
                .target_override()
                .map(|target| (node.id, target))
        })
        .collect::<BTreeMap<_, _>>();
    ensure!(
        node_targets.len() == expected.len(),
        "orchestration run must confirm exactly one target snapshot per override node"
    );
    for (node_id, target_ref) in expected {
        let snapshot = node_targets
            .get(&node_id)
            .with_context(|| format!("orchestration node {node_id} has no confirmed target"))?;
        ensure!(
            snapshot.target_ref == *target_ref,
            "orchestration node {node_id} confirmed the wrong target reference"
        );
        validate_target(snapshot)?;
    }
    Ok(())
}

fn validate_snapshot_key(snapshot: &OrchestrationAttemptSnapshot) -> anyhow::Result<()> {
    let material = format!(
        "riviu-orchestration-attempt-v1:{}:{}:{}:{}",
        snapshot.document_id, snapshot.document_revision, snapshot.node_id, snapshot.attempt_id
    );
    ensure!(
        snapshot.idempotency_key == format!("{:x}", Sha256::digest(material.as_bytes())),
        "orchestration attempt idempotency key mismatch"
    );
    Ok(())
}

fn expected_child_kind(
    record: &OrchestrationAttemptRecord,
) -> anyhow::Result<Option<AutomationKind>> {
    let document: crate::OrchestrationDocumentV1 =
        serde_json::from_str(&record.snapshot.canonical_document_json)
            .context("parse orchestration attempt document")?;
    let node = document
        .nodes
        .iter()
        .find(|node| node.id == record.snapshot.node_id)
        .context("orchestration attempt node is missing")?;
    Ok(match &node.action {
        OrchestrationNodeAction::RunNurture { .. } => Some(AutomationKind::Nurture),
        OrchestrationNodeAction::RunInteraction { .. } => Some(AutomationKind::Interaction),
        OrchestrationNodeAction::RunPublish { .. } => Some(AutomationKind::Publish),
        OrchestrationNodeAction::Start
        | OrchestrationNodeAction::Delay { .. }
        | OrchestrationNodeAction::End => None,
    })
}

fn query_run(connection: &Connection, id: Uuid) -> anyhow::Result<Option<OrchestrationRunRecord>> {
    connection
        .query_row(
            "SELECT id,document_id,document_revision,document_sha256,target_json,node_targets_json,
                    state,current_node_id,error_code,created_at,updated_at
             FROM orchestration_runs WHERE id=?1",
            [id.to_string()],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, Option<String>>(7)?,
                    row.get::<_, Option<String>>(8)?,
                    row.get::<_, String>(9)?,
                    row.get::<_, String>(10)?,
                ))
            },
        )
        .optional()?
        .map(
            |(
                id,
                document_id,
                revision,
                sha256,
                target_json,
                node_targets_json,
                state,
                node,
                error,
                created,
                updated,
            )| {
                let target: ResolvedTargetSnapshot =
                    serde_json::from_str(&target_json).context("parse orchestration run target")?;
                validate_target(&target)?;
                let node_targets: BTreeMap<Uuid, ResolvedTargetSnapshot> =
                    serde_json::from_str(&node_targets_json)
                        .context("parse orchestration run node targets")?;
                for node_target in node_targets.values() {
                    validate_target(node_target)?;
                }
                Ok(OrchestrationRunRecord {
                    id: Uuid::parse_str(&id).context("parse orchestration run ID")?,
                    document_id: Uuid::parse_str(&document_id)
                        .context("parse orchestration document ID")?,
                    document_revision: sql_to_revision(
                        revision,
                        "orchestration_runs.document_revision",
                    )?,
                    document_sha256: sha256,
                    target,
                    node_targets,
                    state: run_state_from_label(&state)?,
                    current_node_id: node
                        .map(|value| Uuid::parse_str(&value).context("parse current node ID"))
                        .transpose()?,
                    error_code: error,
                    created_at: created,
                    updated_at: updated,
                })
            },
        )
        .transpose()
}

fn query_attempt(
    connection: &Connection,
    id: Uuid,
) -> anyhow::Result<Option<OrchestrationAttemptRecord>> {
    connection
        .query_row(
            "SELECT id,run_id,node_id,attempt_no,idempotency_key,snapshot_json,state,
                    child_kind,child_campaign_id,branch,error_code,created_at,updated_at
             FROM orchestration_attempts WHERE id=?1",
            [id.to_string()],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, Option<String>>(7)?,
                    row.get::<_, Option<String>>(8)?,
                    row.get::<_, Option<String>>(9)?,
                    row.get::<_, Option<String>>(10)?,
                    row.get::<_, String>(11)?,
                    row.get::<_, String>(12)?,
                ))
            },
        )
        .optional()?
        .map(
            |(
                row_id,
                run_id,
                node_id,
                attempt_no,
                key,
                snapshot_json,
                state,
                kind,
                child_id,
                branch,
                error,
                created,
                updated,
            )| {
                let snapshot: OrchestrationAttemptSnapshot =
                    serde_json::from_str(&snapshot_json)
                        .context("parse orchestration attempt snapshot")?;
                ensure!(
                    snapshot.attempt_id.to_string() == row_id,
                    "attempt ID mismatch"
                );
                ensure!(
                    snapshot.node_id.to_string() == node_id,
                    "attempt node mismatch"
                );
                ensure!(snapshot.idempotency_key == key, "attempt key mismatch");
                validate_snapshot_key(&snapshot)?;
                Ok(OrchestrationAttemptRecord {
                    snapshot,
                    run_id: Uuid::parse_str(&run_id).context("parse attempt run ID")?,
                    attempt_no: u32::try_from(attempt_no)
                        .context("attempt number does not fit u32")?,
                    state: attempt_state_from_label(&state)?,
                    child_kind: kind
                        .map(|value| AutomationKind::from_str(&value))
                        .transpose()?,
                    child_campaign_id: child_id
                        .map(|value| Uuid::parse_str(&value).context("parse child campaign ID"))
                        .transpose()?,
                    branch: branch.map(|value| branch_from_label(&value)).transpose()?,
                    error_code: error,
                    created_at: created,
                    updated_at: updated,
                })
            },
        )
        .transpose()
}

fn query_nurture_child(
    connection: &Connection,
    id: Uuid,
) -> anyhow::Result<Option<OrchestrationNurtureChildRecord>> {
    connection
        .query_row(
            "SELECT id,attempt_id,idempotency_key,run_id,requested_udids_json,
                    started_udids_json,state,created_at,updated_at
             FROM orchestration_nurture_children WHERE id=?1",
            [id.to_string()],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, String>(7)?,
                    row.get::<_, String>(8)?,
                ))
            },
        )
        .optional()?
        .map(
            |(
                id,
                attempt_id,
                key,
                run_id,
                requested_json,
                started_json,
                state,
                created_at,
                updated_at,
            )| {
                let requested_udids: Vec<String> = serde_json::from_str(&requested_json)
                    .context("parse orchestration nurture requested devices")?;
                let started_udids: Vec<String> = serde_json::from_str(&started_json)
                    .context("parse orchestration nurture started devices")?;
                validate_udids(&requested_udids, "requested")?;
                if !started_udids.is_empty() {
                    validate_udids(&started_udids, "started")?;
                }
                let requested = requested_udids
                    .iter()
                    .map(String::as_str)
                    .collect::<HashSet<_>>();
                ensure!(
                    started_udids
                        .iter()
                        .all(|udid| requested.contains(udid.as_str())),
                    "persisted orchestration nurture child started outside its target"
                );
                Ok(OrchestrationNurtureChildRecord {
                    id: Uuid::parse_str(&id).context("parse orchestration nurture child ID")?,
                    attempt_id: Uuid::parse_str(&attempt_id)
                        .context("parse orchestration nurture attempt ID")?,
                    idempotency_key: key,
                    run_id: run_id
                        .map(|value| {
                            Uuid::parse_str(&value)
                                .context("parse orchestration nurture runtime ID")
                        })
                        .transpose()?,
                    requested_udids,
                    started_udids,
                    state: nurture_state_from_label(&state)?,
                    created_at,
                    updated_at,
                })
            },
        )
        .transpose()
}

fn run_state_from_label(value: &str) -> anyhow::Result<OrchestrationRunState> {
    Ok(match value {
        "queued" => OrchestrationRunState::Queued,
        "running" => OrchestrationRunState::Running,
        "done" => OrchestrationRunState::Done,
        "partial" => OrchestrationRunState::Partial,
        "failed" => OrchestrationRunState::Failed,
        "uncertain" => OrchestrationRunState::Uncertain,
        "cancelled" => OrchestrationRunState::Cancelled,
        other => anyhow::bail!("unknown orchestration run state `{other}`"),
    })
}

fn nurture_state_from_label(value: &str) -> anyhow::Result<OrchestrationNurtureChildState> {
    Ok(match value {
        "dispatching" => OrchestrationNurtureChildState::Dispatching,
        "running" => OrchestrationNurtureChildState::Running,
        "done" => OrchestrationNurtureChildState::Done,
        "partial" => OrchestrationNurtureChildState::Partial,
        "failed" => OrchestrationNurtureChildState::Failed,
        "uncertain" => OrchestrationNurtureChildState::Uncertain,
        other => anyhow::bail!("unknown orchestration nurture child state `{other}`"),
    })
}

fn nurture_state_label(value: OrchestrationNurtureChildState) -> &'static str {
    match value {
        OrchestrationNurtureChildState::Dispatching => "dispatching",
        OrchestrationNurtureChildState::Running => "running",
        OrchestrationNurtureChildState::Done => "done",
        OrchestrationNurtureChildState::Partial => "partial",
        OrchestrationNurtureChildState::Failed => "failed",
        OrchestrationNurtureChildState::Uncertain => "uncertain",
    }
}

fn nurture_state_for_outcome(outcome: ChildCampaignOutcome) -> OrchestrationNurtureChildState {
    match outcome {
        ChildCampaignOutcome::Done => OrchestrationNurtureChildState::Done,
        ChildCampaignOutcome::Partial => OrchestrationNurtureChildState::Partial,
        ChildCampaignOutcome::Failed => OrchestrationNurtureChildState::Failed,
        ChildCampaignOutcome::Uncertain => OrchestrationNurtureChildState::Uncertain,
    }
}

fn validate_udids(udids: &[String], label: &str) -> anyhow::Result<()> {
    ensure!(
        !udids.is_empty(),
        "orchestration nurture {label} devices must not be empty"
    );
    let mut unique = HashSet::new();
    ensure!(
        udids
            .iter()
            .all(|udid| !udid.trim().is_empty() && unique.insert(udid.as_str())),
        "orchestration nurture {label} devices are empty or duplicated"
    );
    Ok(())
}

fn run_state_label(value: OrchestrationRunState) -> &'static str {
    match value {
        OrchestrationRunState::Queued => "queued",
        OrchestrationRunState::Running => "running",
        OrchestrationRunState::Done => "done",
        OrchestrationRunState::Partial => "partial",
        OrchestrationRunState::Failed => "failed",
        OrchestrationRunState::Uncertain => "uncertain",
        OrchestrationRunState::Cancelled => "cancelled",
    }
}

fn attempt_state_label(value: OrchestrationAttemptState) -> &'static str {
    match value {
        OrchestrationAttemptState::Queued => "queued",
        OrchestrationAttemptState::Dispatching => "dispatching",
        OrchestrationAttemptState::WaitingChild => "waiting_child",
        OrchestrationAttemptState::Done => "done",
        OrchestrationAttemptState::Partial => "partial",
        OrchestrationAttemptState::Failed => "failed",
        OrchestrationAttemptState::Uncertain => "uncertain",
        OrchestrationAttemptState::Cancelled => "cancelled",
    }
}

fn attempt_state_from_label(value: &str) -> anyhow::Result<OrchestrationAttemptState> {
    Ok(match value {
        "queued" => OrchestrationAttemptState::Queued,
        "dispatching" => OrchestrationAttemptState::Dispatching,
        "waiting_child" => OrchestrationAttemptState::WaitingChild,
        "done" => OrchestrationAttemptState::Done,
        "partial" => OrchestrationAttemptState::Partial,
        "failed" => OrchestrationAttemptState::Failed,
        "uncertain" => OrchestrationAttemptState::Uncertain,
        "cancelled" => OrchestrationAttemptState::Cancelled,
        other => anyhow::bail!("unknown orchestration attempt state `{other}`"),
    })
}

fn attempt_state_for_branch(branch: OrchestrationBranch) -> OrchestrationAttemptState {
    match branch {
        OrchestrationBranch::Done => OrchestrationAttemptState::Done,
        OrchestrationBranch::Partial => OrchestrationAttemptState::Partial,
        OrchestrationBranch::Failed => OrchestrationAttemptState::Failed,
        OrchestrationBranch::Uncertain => OrchestrationAttemptState::Uncertain,
    }
}

fn branch_label(value: OrchestrationBranch) -> &'static str {
    match value {
        OrchestrationBranch::Done => "done",
        OrchestrationBranch::Partial => "partial",
        OrchestrationBranch::Failed => "failed",
        OrchestrationBranch::Uncertain => "uncertain",
    }
}

fn branch_from_label(value: &str) -> anyhow::Result<OrchestrationBranch> {
    Ok(match value {
        "done" => OrchestrationBranch::Done,
        "partial" => OrchestrationBranch::Partial,
        "failed" => OrchestrationBranch::Failed,
        "uncertain" => OrchestrationBranch::Uncertain,
        other => anyhow::bail!("unknown orchestration branch `{other}`"),
    })
}

fn revision_to_sql(value: u64) -> anyhow::Result<i64> {
    i64::try_from(value).context("orchestration revision does not fit SQLite INTEGER")
}

fn sql_to_revision(value: i64, field: &'static str) -> anyhow::Result<u64> {
    u64::try_from(value).with_context(|| format!("{field} must not be negative"))
}

fn timestamp() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Nanos, true)
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use uuid::Uuid;

    use super::Database;
    use crate::{
        compile_orchestration, snapshot_orchestration_attempt, AutomationDefinitionRecord,
        AutomationKind, AutomationProfileRef, CompiledOrchestrationV1, OrchestrationAttemptState,
        OrchestrationBranch, OrchestrationDocumentV1, OrchestrationEdge, OrchestrationNode,
        OrchestrationNodeAction, OrchestrationNurtureChildState, OrchestrationPoint,
        ResolvedTargetDevice, ResolvedTargetSnapshot, TargetRef, ORCHESTRATION_SCHEMA_VERSION,
    };

    fn fixture() -> (Database, std::path::PathBuf) {
        let path = std::env::temp_dir().join(format!("riviu-orchestration-{}.db", Uuid::new_v4()));
        (Database::open(&path).expect("orchestration fixture"), path)
    }

    fn cleanup(path: &std::path::Path) {
        for suffix in ["", "-wal", "-shm"] {
            let candidate = std::path::PathBuf::from(format!("{}{suffix}", path.display()));
            if candidate.exists() {
                std::fs::remove_file(candidate).expect("remove orchestration fixture");
            }
        }
    }

    fn profile(database: &Database) -> AutomationDefinitionRecord {
        database
            .create_automation_definition(
                "Tương tác buổi sáng",
                AutomationKind::Interaction,
                &TargetRef::All,
                &json!({
                    "schemaVersion": 1,
                    "request": {
                        "targets": [],
                        "messageCount": 1,
                        "instruction": "fixture",
                        "maxWords": 12,
                        "actions": { "comment": false, "like": true, "save": true }
                    }
                }),
            )
            .expect("create profile")
    }

    fn document(profile: &AutomationDefinitionRecord, revision: u64) -> OrchestrationDocumentV1 {
        let start = Uuid::from_u128(1);
        let run = Uuid::from_u128(2);
        let end = Uuid::from_u128(3);
        OrchestrationDocumentV1 {
            schema_version: ORCHESTRATION_SCHEMA_VERSION,
            id: Uuid::from_u128(20),
            revision,
            name: "Ca sáng".into(),
            entry_node_id: start,
            nodes: vec![
                OrchestrationNode {
                    id: start,
                    position: OrchestrationPoint { x: 0.0, y: 0.0 },
                    action: OrchestrationNodeAction::Start,
                },
                OrchestrationNode {
                    id: run,
                    position: OrchestrationPoint { x: 200.0, y: 0.0 },
                    action: OrchestrationNodeAction::RunInteraction {
                        profile: AutomationProfileRef {
                            definition_id: profile.definition.id,
                            revision: profile.revision.revision,
                        },
                        target_override: None,
                    },
                },
                OrchestrationNode {
                    id: end,
                    position: OrchestrationPoint { x: 400.0, y: 0.0 },
                    action: OrchestrationNodeAction::End,
                },
            ],
            edges: vec![
                OrchestrationEdge {
                    source_node_id: start,
                    source_port: OrchestrationBranch::Done,
                    target_node_id: run,
                },
                OrchestrationEdge {
                    source_node_id: run,
                    source_port: OrchestrationBranch::Done,
                    target_node_id: end,
                },
                OrchestrationEdge {
                    source_node_id: run,
                    source_port: OrchestrationBranch::Partial,
                    target_node_id: end,
                },
                OrchestrationEdge {
                    source_node_id: run,
                    source_port: OrchestrationBranch::Failed,
                    target_node_id: end,
                },
                OrchestrationEdge {
                    source_node_id: run,
                    source_port: OrchestrationBranch::Uncertain,
                    target_node_id: end,
                },
            ],
        }
    }

    fn compiled(profile: &AutomationDefinitionRecord, revision: u64) -> CompiledOrchestrationV1 {
        compile_orchestration(&document(profile, revision), std::slice::from_ref(profile))
            .expect("compile fixture")
    }

    fn target() -> ResolvedTargetSnapshot {
        ResolvedTargetSnapshot {
            target_ref: TargetRef::All,
            included: vec![ResolvedTargetDevice {
                udid: "phone-1".into(),
                alias: "Máy 1".into(),
                number: Some(1),
            }],
            excluded: Vec::new(),
            roster_sha256: "a".repeat(64),
        }
    }

    #[test]
    fn revisions_are_immutable_and_runs_keep_the_exact_pinned_revision() {
        let (database, path) = fixture();
        let profile = profile(&database);
        let first = database
            .save_orchestration_revision(None, &compiled(&profile, 1))
            .expect("save first revision");
        let second = database
            .save_orchestration_revision(Some(1), &compiled(&profile, 2))
            .expect("save second revision");
        assert_eq!(first.compiled.document.revision, 1);
        assert_eq!(second.compiled.document.revision, 2);

        let run = database
            .create_orchestration_run(
                first.compiled.document.id,
                1,
                &target(),
                &std::collections::BTreeMap::new(),
            )
            .expect("create pinned run");
        assert_eq!(run.document_revision, 1);
        assert_eq!(run.document_sha256, first.compiled.sha256);
        assert_ne!(run.document_sha256, second.compiled.sha256);
        assert_eq!(
            database
                .get_orchestration_revision(first.compiled.document.id, Some(1))
                .expect("read first")
                .expect("first exists"),
            first
        );
        cleanup(&path);
    }

    #[test]
    fn recovery_run_scan_is_unbounded_and_excludes_terminal_runs() {
        let (database, path) = fixture();
        let profile = profile(&database);
        let compiled = compiled(&profile, 1);
        database
            .save_orchestration_revision(None, &compiled)
            .expect("save revision");

        let runs = (0..201)
            .map(|_| {
                database
                    .create_orchestration_run(
                        compiled.document.id,
                        1,
                        &target(),
                        &std::collections::BTreeMap::new(),
                    )
                    .expect("create recoverable run")
            })
            .collect::<Vec<_>>();
        assert_eq!(
            database
                .list_recoverable_orchestration_runs()
                .expect("list all recoverable runs")
                .len(),
            201,
            "bootstrap recovery must not inherit the monitor API's 200-row page cap"
        );

        database
            .transition_orchestration_run(
                runs[0].id,
                crate::OrchestrationRunState::Queued,
                crate::OrchestrationRunState::Running,
                Some(compiled.document.entry_node_id),
                None,
            )
            .expect("start run")
            .expect("start transition owner");
        database
            .transition_orchestration_run(
                runs[0].id,
                crate::OrchestrationRunState::Running,
                crate::OrchestrationRunState::Done,
                None,
                None,
            )
            .expect("finish run")
            .expect("finish transition owner");
        let recoverable = database
            .list_recoverable_orchestration_runs()
            .expect("list nonterminal runs");
        assert_eq!(recoverable.len(), 200);
        assert!(!recoverable.iter().any(|run| run.id == runs[0].id));
        cleanup(&path);
    }

    #[test]
    fn run_enqueue_rechecks_that_every_pinned_profile_is_available() {
        let (database, path) = fixture();
        let profile = profile(&database);
        let compiled = compiled(&profile, 1);
        database
            .save_orchestration_revision(None, &compiled)
            .expect("save revision while profile is active");
        database
            .archive_automation_definition(profile.definition.id)
            .expect("archive profile after orchestration save");

        let error = database
            .create_orchestration_run(
                compiled.document.id,
                1,
                &target(),
                &std::collections::BTreeMap::new(),
            )
            .expect_err("archived pinned profile must fail before enqueue");
        assert!(error.to_string().contains("ProfileArchived"), "{error:#}");
        assert!(database
            .list_orchestration_runs(1)
            .expect("list runs")
            .is_empty());
        cleanup(&path);
    }

    #[test]
    fn graph_advance_preserves_a_concurrent_cancel_marker() {
        let (database, path) = fixture();
        let profile = profile(&database);
        let compiled = compiled(&profile, 1);
        database
            .save_orchestration_revision(None, &compiled)
            .expect("save revision");
        let run = database
            .create_orchestration_run(
                compiled.document.id,
                1,
                &target(),
                &std::collections::BTreeMap::new(),
            )
            .expect("create run");
        let start = compiled.document.entry_node_id;
        let campaign = Uuid::from_u128(2);
        database
            .transition_orchestration_run(
                run.id,
                crate::OrchestrationRunState::Queued,
                crate::OrchestrationRunState::Running,
                Some(start),
                None,
            )
            .expect("start run")
            .expect("start owner");
        database
            .request_orchestration_cancel(run.id)
            .expect("request cancel")
            .expect("running run accepted cancellation");

        let advanced = database
            .advance_orchestration_run(run.id, start, campaign)
            .expect("advance graph")
            .expect("advance owner");

        assert_eq!(advanced.current_node_id, Some(campaign));
        assert_eq!(advanced.error_code.as_deref(), Some("cancel_requested"));
        cleanup(&path);
    }

    #[test]
    fn one_attempt_can_arm_only_one_child_and_restart_reconciles_it() {
        let (database, path) = fixture();
        let profile = profile(&database);
        let compiled = compiled(&profile, 1);
        database
            .save_orchestration_revision(None, &compiled)
            .expect("save revision");
        let run = database
            .create_orchestration_run(
                compiled.document.id,
                1,
                &target(),
                &std::collections::BTreeMap::new(),
            )
            .expect("create run");
        let attempt_id = Uuid::from_u128(30);
        let snapshot =
            snapshot_orchestration_attempt(&compiled, Uuid::from_u128(2), attempt_id, target())
                .expect("snapshot attempt");
        let attempt = database
            .create_orchestration_attempt(run.id, 1, &snapshot)
            .expect("create attempt");
        assert_eq!(attempt.state, OrchestrationAttemptState::Queued);

        let child_id = Uuid::from_u128(31);
        let armed = database
            .arm_orchestration_child(attempt_id, AutomationKind::Interaction, child_id)
            .expect("arm child")
            .expect("first owner");
        assert_eq!(armed.state, OrchestrationAttemptState::Dispatching);
        assert_eq!(armed.child_campaign_id, Some(child_id));
        assert!(database
            .arm_orchestration_child(attempt_id, AutomationKind::Interaction, Uuid::from_u128(32),)
            .expect("repeat arm")
            .is_none());

        let recoverable = database
            .list_recoverable_orchestration_attempts()
            .expect("list recovery");
        assert_eq!(recoverable.len(), 1);
        assert_eq!(recoverable[0].child_campaign_id, Some(child_id));
        assert_eq!(recoverable[0].state, OrchestrationAttemptState::Dispatching);

        let settled = database
            .settle_orchestration_child(
                attempt_id,
                child_id,
                OrchestrationBranch::Uncertain,
                Some("child_uncertain"),
            )
            .expect("settle uncertain")
            .expect("owner");
        assert_eq!(settled.state, OrchestrationAttemptState::Uncertain);
        assert!(database
            .list_recoverable_orchestration_attempts()
            .expect("terminal recovery scan")
            .is_empty());
        cleanup(&path);
    }

    #[test]
    fn stale_unarmed_cancel_cannot_overwrite_a_concurrently_armed_child() {
        let (database, path) = fixture();
        let profile = profile(&database);
        let compiled = compiled(&profile, 1);
        database
            .save_orchestration_revision(None, &compiled)
            .expect("save revision");
        let run = database
            .create_orchestration_run(
                compiled.document.id,
                1,
                &target(),
                &std::collections::BTreeMap::new(),
            )
            .expect("create run");
        let attempt_id = Uuid::from_u128(33);
        let snapshot =
            snapshot_orchestration_attempt(&compiled, Uuid::from_u128(2), attempt_id, target())
                .expect("snapshot attempt");
        database
            .create_orchestration_attempt(run.id, 1, &snapshot)
            .expect("create queued attempt");

        // The cancel path read the queued row before this second connection won the arm CAS.
        let executor = Database::open(&path).expect("open concurrent executor");
        let child_id = Uuid::from_u128(34);
        executor
            .arm_orchestration_child(attempt_id, AutomationKind::Interaction, child_id)
            .expect("arm from concurrent executor")
            .expect("executor owns arm CAS");

        assert!(database
            .cancel_orchestration_attempt_before_effect(attempt_id)
            .expect("stale unarmed cancel CAS")
            .is_none());
        let armed = database
            .get_orchestration_run(run.id)
            .expect("read run")
            .expect("run exists")
            .attempts
            .into_iter()
            .find(|attempt| attempt.snapshot.attempt_id == attempt_id)
            .expect("attempt exists");
        assert_eq!(armed.state, OrchestrationAttemptState::Dispatching);
        assert_eq!(armed.child_campaign_id, Some(child_id));

        let cancelled = database
            .cancel_orchestration_child_before_effect(attempt_id, child_id)
            .expect("adapter-proven child cancellation")
            .expect("exact armed child owns cancellation CAS");
        assert_eq!(cancelled.state, OrchestrationAttemptState::Cancelled);
        assert_eq!(cancelled.child_campaign_id, Some(child_id));
        drop(executor);
        cleanup(&path);
    }

    #[test]
    fn nurture_child_identity_and_terminal_status_survive_database_reopen() {
        let (database, path) = fixture();
        let profile = database
            .create_automation_definition(
                "Nuoi TikTok",
                AutomationKind::Nurture,
                &TargetRef::All,
                &json!({"schemaVersion": 1, "settings": {}}),
            )
            .expect("create nurture profile");
        let mut document = document(&profile, 1);
        document.nodes[1].action = OrchestrationNodeAction::RunNurture {
            profile: AutomationProfileRef {
                definition_id: profile.definition.id,
                revision: profile.revision.revision,
            },
            target_override: None,
        };
        let compiled = compile_orchestration(&document, std::slice::from_ref(&profile))
            .expect("compile nurture orchestration");
        database
            .save_orchestration_revision(None, &compiled)
            .expect("save nurture orchestration");
        let run = database
            .create_orchestration_run(
                compiled.document.id,
                1,
                &target(),
                &std::collections::BTreeMap::new(),
            )
            .expect("create run");
        let attempt_id = Uuid::from_u128(40);
        let snapshot =
            snapshot_orchestration_attempt(&compiled, Uuid::from_u128(2), attempt_id, target())
                .expect("snapshot nurture attempt");
        database
            .create_orchestration_attempt(run.id, 1, &snapshot)
            .expect("create nurture attempt");
        let child_id = Uuid::from_u128(41);
        database
            .arm_orchestration_child(attempt_id, AutomationKind::Nurture, child_id)
            .expect("arm nurture child")
            .expect("arm owner");

        let requested = vec!["phone-1".to_string()];
        let (created, inserted) = database
            .create_orchestration_nurture_child(attempt_id, child_id, &snapshot.idempotency_key)
            .expect("persist nurture child before dispatch");
        assert!(inserted);
        assert_eq!(created.state, OrchestrationNurtureChildState::Dispatching);
        assert_eq!(created.requested_udids, requested);

        let run_id = Uuid::from_u128(42);
        let running = database
            .start_orchestration_nurture_child(
                child_id,
                &snapshot.idempotency_key,
                run_id,
                &["phone-1".to_string()],
            )
            .expect("persist live nurture identity");
        assert_eq!(running.run_id, Some(run_id));
        assert_eq!(running.state, OrchestrationNurtureChildState::Running);
        let terminal = database
            .settle_orchestration_nurture_child(
                child_id,
                &snapshot.idempotency_key,
                crate::ChildCampaignOutcome::Partial,
            )
            .expect("persist terminal nurture status");
        assert_eq!(terminal.state, OrchestrationNurtureChildState::Partial);

        drop(database);
        let reopened = Database::open(&path).expect("reopen database");
        assert_eq!(
            reopened
                .get_orchestration_nurture_child(child_id)
                .expect("read durable nurture child"),
            Some(terminal)
        );
        drop(reopened);
        cleanup(&path);
    }
}
