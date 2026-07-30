use anyhow::Context;
use chrono::{SecondsFormat, Utc};
use rusqlite::{params, OptionalExtension, TransactionBehavior};
use uuid::Uuid;

use super::Database;
use crate::{
    canonical_compiled_plan_json, compiled_plan_sha256, CompiledFlowPlanV2, FlowDocumentV2, FlowId,
    FlowRevisionRecord, FlowSummary, RevisionConflict, FLOW_SCHEMA_VERSION,
};

impl Database {
    pub fn save_flow_revision(
        &self,
        expected_revision: Option<u64>,
        document: &FlowDocumentV2,
        compiled: &CompiledFlowPlanV2,
        plan_hash: &str,
    ) -> anyhow::Result<FlowRevisionRecord> {
        let mut connection = self.conn()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let flow_id = document.id.to_string();
        let current: Option<i64> = transaction
            .query_row(
                "SELECT latest_revision FROM flow_documents WHERE id=?1",
                [&flow_id],
                |row| row.get(0),
            )
            .optional()?;
        let actual = current
            .map(|revision| sql_to_revision(revision, "flow_documents.latest_revision"))
            .transpose()?
            .unwrap_or(0);

        match (current.is_some(), expected_revision) {
            (false, None) => {}
            (false, Some(_)) => {
                anyhow::bail!(
                    "expectedRevision must be absent when creating a new flow (actual revision 0)"
                );
            }
            (true, Some(expected)) if expected == actual => {}
            (true, Some(expected)) => {
                return Err(RevisionConflict { expected, actual }.into());
            }
            (true, None) => {
                return Err(RevisionConflict {
                    expected: 0,
                    actual,
                }
                .into());
            }
        }

        let next_revision = actual.checked_add(1).context("Flow revision overflow")?;
        if document.revision != next_revision || compiled.revision != next_revision {
            anyhow::bail!(
                "document revision {} and compiled revision {} must both equal next revision {next_revision}",
                document.revision,
                compiled.revision
            );
        }
        validate_revision_identity(document, compiled)?;
        let computed_hash = compiled_plan_sha256(compiled).context("hash compiled Flow plan")?;
        if computed_hash.as_bytes() != plan_hash.as_bytes() {
            anyhow::bail!("plan hash mismatch: supplied {plan_hash}, computed {computed_hash}");
        }
        let authoring_json =
            serde_json::to_string(document).context("serialize Flow authoring document")?;
        let compiled_json =
            canonical_compiled_plan_json(compiled).context("serialize canonical Flow plan")?;
        let next_revision_sql = revision_to_sql(next_revision, "next Flow revision")?;
        let now = timestamp();

        if current.is_none() {
            transaction.execute(
                "INSERT INTO flow_documents(
                    id,name,latest_revision,archived,created_at,updated_at
                 ) VALUES(?1,?2,?3,0,?4,?4)",
                params![flow_id, document.name, next_revision_sql, now],
            )?;
        } else {
            let changed = transaction.execute(
                "UPDATE flow_documents
                 SET name=?2,latest_revision=?3,updated_at=?4
                 WHERE id=?1 AND latest_revision=?5",
                params![
                    flow_id,
                    document.name,
                    next_revision_sql,
                    now,
                    revision_to_sql(actual, "current Flow revision")?
                ],
            )?;
            if changed != 1 {
                anyhow::bail!("Flow revision changed during the save transaction");
            }
        }
        transaction.execute(
            "INSERT INTO flow_revisions(
                flow_id,revision,authoring_json,compiled_json,plan_sha256,created_at
             ) VALUES(?1,?2,?3,?4,?5,?6)",
            params![
                flow_id,
                next_revision_sql,
                authoring_json,
                compiled_json,
                plan_hash,
                now
            ],
        )?;
        transaction.commit()?;

        Ok(FlowRevisionRecord {
            document: document.clone(),
            compiled_plan: compiled.clone(),
            plan_hash: plan_hash.to_owned(),
            created_at: now,
        })
    }

    pub fn list_flows(&self, include_archived: bool) -> anyhow::Result<Vec<FlowSummary>> {
        let connection = self.conn()?;
        let mut statement = connection.prepare(
            "SELECT id,name,latest_revision,archived,updated_at
             FROM flow_documents
             WHERE (?1=1 OR archived=0)
             ORDER BY updated_at DESC,id ASC",
        )?;
        let rows = statement.query_map([i64::from(include_archived)], |row| {
            Ok(FlowSummaryRow {
                id: row.get(0)?,
                name: row.get(1)?,
                latest_revision: row.get(2)?,
                archived: row.get(3)?,
                updated_at: row.get(4)?,
            })
        })?;
        let mut summaries = Vec::new();
        for row in rows {
            summaries.push(row?.into_summary()?);
        }
        Ok(summaries)
    }

    pub fn get_flow_revision(
        &self,
        id: FlowId,
        revision: Option<u64>,
    ) -> anyhow::Result<Option<FlowRevisionRecord>> {
        let revision = revision
            .map(|value| revision_to_sql(value, "requested Flow revision"))
            .transpose()?;
        let connection = self.conn()?;
        let row = connection
            .query_row(
                "SELECT r.revision,r.authoring_json,r.compiled_json,r.plan_sha256,r.created_at
                 FROM flow_revisions r
                 JOIN flow_documents d ON d.id=r.flow_id
                 WHERE r.flow_id=?1 AND r.revision=COALESCE(?2,d.latest_revision)",
                params![id.to_string(), revision],
                |row| {
                    Ok(FlowRevisionRow {
                        revision: row.get(0)?,
                        authoring_json: row.get(1)?,
                        compiled_json: row.get(2)?,
                        plan_hash: row.get(3)?,
                        created_at: row.get(4)?,
                    })
                },
            )
            .optional()?;
        row.map(|row| row.into_record(id)).transpose()
    }

    pub fn archive_flow(&self, id: FlowId) -> anyhow::Result<()> {
        let connection = self.conn()?;
        let changed = connection.execute(
            "UPDATE flow_documents SET archived=1,updated_at=?2 WHERE id=?1",
            params![id.to_string(), timestamp()],
        )?;
        if changed != 1 {
            anyhow::bail!("flow {id} not found");
        }
        Ok(())
    }
}

fn validate_revision_identity(
    document: &FlowDocumentV2,
    compiled: &CompiledFlowPlanV2,
) -> anyhow::Result<()> {
    if document.schema_version != FLOW_SCHEMA_VERSION
        || compiled.schema_version != FLOW_SCHEMA_VERSION
    {
        anyhow::bail!(
            "Flow schema version mismatch: document={}, compiled={}, expected={FLOW_SCHEMA_VERSION}",
            document.schema_version,
            compiled.schema_version
        );
    }
    if document.id != compiled.flow_id {
        anyhow::bail!(
            "flow ID mismatch: document={}, compiled={}",
            document.id,
            compiled.flow_id
        );
    }
    Ok(())
}

fn timestamp() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Nanos, true)
}

fn revision_to_sql(revision: u64, field: &str) -> anyhow::Result<i64> {
    i64::try_from(revision).with_context(|| format!("{field} exceeds SQLite INTEGER range"))
}

fn sql_to_revision(revision: i64, field: &str) -> anyhow::Result<u64> {
    u64::try_from(revision).with_context(|| format!("{field} is negative"))
}

struct FlowSummaryRow {
    id: String,
    name: String,
    latest_revision: i64,
    archived: i64,
    updated_at: String,
}

impl FlowSummaryRow {
    fn into_summary(self) -> anyhow::Result<FlowSummary> {
        if !matches!(self.archived, 0 | 1) {
            anyhow::bail!("flow {} has invalid archived flag", self.id);
        }
        Ok(FlowSummary {
            id: Uuid::parse_str(&self.id).context("parse persisted Flow ID")?,
            name: self.name,
            latest_revision: sql_to_revision(
                self.latest_revision,
                "flow_documents.latest_revision",
            )?,
            archived: self.archived == 1,
            updated_at: self.updated_at,
        })
    }
}

struct FlowRevisionRow {
    revision: i64,
    authoring_json: String,
    compiled_json: String,
    plan_hash: String,
    created_at: String,
}

impl FlowRevisionRow {
    fn into_record(self, requested_id: FlowId) -> anyhow::Result<FlowRevisionRecord> {
        let revision = sql_to_revision(self.revision, "flow_revisions.revision")?;
        let document: FlowDocumentV2 =
            serde_json::from_str(&self.authoring_json).context("parse persisted Flow document")?;
        let compiled_plan: CompiledFlowPlanV2 = serde_json::from_str(&self.compiled_json)
            .context("parse persisted compiled Flow plan")?;
        validate_revision_identity(&document, &compiled_plan)?;
        if document.id != requested_id
            || compiled_plan.flow_id != requested_id
            || document.revision != revision
            || compiled_plan.revision != revision
        {
            anyhow::bail!("persisted Flow revision identity mismatch");
        }
        let canonical = canonical_compiled_plan_json(&compiled_plan)
            .context("canonicalize persisted compiled Flow plan")?;
        if canonical.as_bytes() != self.compiled_json.as_bytes() {
            anyhow::bail!("persisted compiled Flow plan is not canonical");
        }
        let computed_hash =
            compiled_plan_sha256(&compiled_plan).context("hash persisted compiled Flow plan")?;
        if computed_hash.as_bytes() != self.plan_hash.as_bytes() {
            anyhow::bail!("persisted Flow plan hash mismatch");
        }
        Ok(FlowRevisionRecord {
            document,
            compiled_plan,
            plan_hash: self.plan_hash,
            created_at: self.created_at,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Barrier};

    use uuid::Uuid;

    use super::super::Database;
    use crate::{
        canonical_compiled_plan_json, compiled_plan_sha256, CompiledFlowPlanV2, ContextPlan,
        FlowDocumentV2, FLOW_SCHEMA_VERSION,
    };

    fn flow_database_fixture() -> (Database, PathBuf) {
        let path =
            std::env::temp_dir().join(format!("riviu-flow-repository-{}.db", uuid::Uuid::new_v4()));
        let database = Database::open(&path).expect("flow database");
        (database, path)
    }

    fn cleanup(path: &Path) {
        for suffix in ["", "-wal", "-shm"] {
            let candidate = PathBuf::from(format!("{}{suffix}", path.display()));
            if candidate.exists() {
                std::fs::remove_file(candidate).expect("remove flow fixture");
            }
        }
    }

    fn compiled_for(document: &FlowDocumentV2) -> CompiledFlowPlanV2 {
        CompiledFlowPlanV2 {
            schema_version: FLOW_SCHEMA_VERSION,
            flow_id: document.id,
            revision: document.revision,
            nodes: Default::default(),
            execution_order: Vec::new(),
            context_plan: ContextPlan {
                requires_exclusive: false,
                requires_ui_session: false,
                requires_stream: false,
                requires_fresh_text_session: false,
                initial_bundle_id: None,
            },
            action_definition_versions: Default::default(),
            required_capabilities: Default::default(),
        }
    }

    fn revision_one(name: &str) -> (FlowDocumentV2, CompiledFlowPlanV2, String) {
        let mut document = FlowDocumentV2::empty(name);
        document.revision = 1;
        let compiled = compiled_for(&document);
        let hash = compiled_plan_sha256(&compiled).expect("plan hash");
        (document, compiled, hash)
    }

    #[test]
    fn immutable_revision_save_rejects_a_stale_writer() {
        let (database, path) = flow_database_fixture();
        let (document, compiled, hash) = revision_one("Fixture");
        let first = database
            .save_flow_revision(None, &document, &compiled, &hash)
            .expect("revision one");
        assert_eq!(first.document.revision, 1);

        let mut second_document = first.document.clone();
        second_document.revision = 2;
        second_document.viewport.x = 240.0;
        second_document.nodes[0].position.y += 40.0;
        let mut second_plan = first.compiled_plan.clone();
        second_plan.revision = 2;
        let second_hash = compiled_plan_sha256(&second_plan).expect("second hash");
        assert_eq!(
            hash, second_hash,
            "revision/layout-only save changed execution hash"
        );
        let second = database
            .save_flow_revision(Some(1), &second_document, &second_plan, &second_hash)
            .expect("revision two");
        assert_eq!(second.document.revision, 2);
        let error = database
            .save_flow_revision(
                Some(1),
                &second.document,
                &second.compiled_plan,
                &second.plan_hash,
            )
            .expect_err("stale save must fail");
        assert!(error.to_string().contains("expected revision 1"));
        let error = database
            .save_flow_revision(
                Some(1),
                &second.document,
                &second.compiled_plan,
                "invalid-stale-hash",
            )
            .expect_err("stale conflict precedes payload validation");
        assert!(error.downcast_ref::<crate::RevisionConflict>().is_some());

        drop(database);
        cleanup(&path);
    }

    #[test]
    fn repository_lists_gets_and_archives_immutable_revisions() {
        let (database, path) = flow_database_fixture();
        let (alpha_document, alpha_plan, alpha_hash) = revision_one("Alpha");
        let alpha = database
            .save_flow_revision(None, &alpha_document, &alpha_plan, &alpha_hash)
            .expect("save Alpha");
        let (beta_document, beta_plan, beta_hash) = revision_one("Beta");
        let beta = database
            .save_flow_revision(None, &beta_document, &beta_plan, &beta_hash)
            .expect("save Beta");

        let connection = database.conn().expect("set deterministic timestamps");
        connection
            .execute(
                "UPDATE flow_documents SET updated_at='2026-07-30T00:00:01.000000000Z'
                 WHERE id=?1",
                [alpha.document.id.to_string()],
            )
            .expect("timestamp Alpha");
        connection
            .execute(
                "UPDATE flow_documents SET updated_at='2026-07-30T00:00:02.000000000Z'
                 WHERE id=?1",
                [beta.document.id.to_string()],
            )
            .expect("timestamp Beta");
        drop(connection);

        let active = database.list_flows(false).expect("list active flows");
        assert_eq!(
            active.iter().map(|flow| flow.id).collect::<Vec<_>>(),
            vec![beta.document.id, alpha.document.id]
        );
        assert!(active.iter().all(|flow| !flow.archived));

        let exact_alpha = database
            .get_flow_revision(alpha.document.id, Some(1))
            .expect("get exact Alpha")
            .expect("Alpha revision one");
        assert_eq!(exact_alpha, alpha);
        assert_eq!(
            database
                .get_flow_revision(alpha.document.id, None)
                .expect("get latest Alpha"),
            Some(alpha.clone())
        );
        assert!(database
            .get_flow_revision(Uuid::new_v4(), None)
            .expect("missing flow lookup")
            .is_none());

        database
            .archive_flow(beta.document.id)
            .expect("archive Beta");
        assert_eq!(
            database
                .list_flows(false)
                .expect("list non-archived")
                .iter()
                .map(|flow| flow.id)
                .collect::<Vec<_>>(),
            vec![alpha.document.id]
        );
        let including_archived = database.list_flows(true).expect("list all flows");
        assert_eq!(including_archived.len(), 2);
        assert_eq!(including_archived[0].id, beta.document.id);
        assert!(including_archived[0].archived);

        drop(database);
        cleanup(&path);
    }

    #[test]
    fn layout_only_revision_changes_authoring_json_but_not_execution_hash() {
        let (database, path) = flow_database_fixture();
        let (first_document, first_plan, first_hash) = revision_one("Layout");
        let first = database
            .save_flow_revision(None, &first_document, &first_plan, &first_hash)
            .expect("save first layout");

        let mut second_document = first.document.clone();
        second_document.revision = 2;
        second_document.viewport.x = 120.0;
        second_document.viewport.zoom = 1.25;
        second_document.nodes[0].position.x += 80.0;
        let mut second_plan = first.compiled_plan.clone();
        second_plan.revision = 2;
        let second_hash = compiled_plan_sha256(&second_plan).expect("layout plan hash");
        assert_eq!(first_hash, second_hash);
        let second = database
            .save_flow_revision(Some(1), &second_document, &second_plan, &second_hash)
            .expect("save second layout");

        let connection = database.conn().expect("inspect stored JSON");
        let first_authoring: String = connection
            .query_row(
                "SELECT authoring_json FROM flow_revisions WHERE flow_id=?1 AND revision=1",
                [first.document.id.to_string()],
                |row| row.get(0),
            )
            .expect("first authoring JSON");
        let second_authoring: String = connection
            .query_row(
                "SELECT authoring_json FROM flow_revisions WHERE flow_id=?1 AND revision=2",
                [first.document.id.to_string()],
                |row| row.get(0),
            )
            .expect("second authoring JSON");
        let stored_compiled: String = connection
            .query_row(
                "SELECT compiled_json FROM flow_revisions WHERE flow_id=?1 AND revision=2",
                [first.document.id.to_string()],
                |row| row.get(0),
            )
            .expect("stored compiled JSON");
        assert_ne!(first_authoring, second_authoring);
        assert_eq!(
            stored_compiled,
            canonical_compiled_plan_json(&second.compiled_plan).expect("canonical plan JSON")
        );
        drop(connection);

        let original = database
            .get_flow_revision(first.document.id, Some(1))
            .expect("read original")
            .expect("original revision");
        assert_eq!(original.document.viewport.x, 0.0);
        assert_eq!(second.document.viewport.x, 120.0);
        assert_eq!(original.plan_hash, second.plan_hash);

        drop(database);
        cleanup(&path);
    }

    #[test]
    fn repository_rejects_hash_identity_revision_and_create_contract_mismatches() {
        let (database, path) = flow_database_fixture();
        let (document, compiled, hash) = revision_one("Validation");

        let error = database
            .save_flow_revision(None, &document, &compiled, &format!("x{hash}"))
            .expect_err("hash mismatch");
        assert!(error.to_string().contains("plan hash mismatch"));
        assert!(database
            .list_flows(true)
            .expect("list after hash error")
            .is_empty());

        let mut wrong_identity = compiled.clone();
        wrong_identity.flow_id = Uuid::new_v4();
        let wrong_identity_hash =
            compiled_plan_sha256(&wrong_identity).expect("wrong identity hash");
        let error = database
            .save_flow_revision(None, &document, &wrong_identity, &wrong_identity_hash)
            .expect_err("flow identity mismatch");
        assert!(error.to_string().contains("flow ID mismatch"));

        let mut wrong_revision = compiled.clone();
        wrong_revision.revision = 2;
        let wrong_revision_hash =
            compiled_plan_sha256(&wrong_revision).expect("wrong revision hash");
        let error = database
            .save_flow_revision(None, &document, &wrong_revision, &wrong_revision_hash)
            .expect_err("compiled revision mismatch");
        assert!(error.to_string().contains("revision 1"));

        let error = database
            .save_flow_revision(Some(0), &document, &compiled, &hash)
            .expect_err("new flow requires absent expected revision");
        assert!(error
            .to_string()
            .contains("expectedRevision must be absent"));

        let saved = database
            .save_flow_revision(None, &document, &compiled, &hash)
            .expect("valid revision after rejected writes");
        assert_eq!(saved.document.revision, 1);
        assert_eq!(
            database
                .get_flow_revision(saved.document.id, Some(1))
                .expect("read immutable revision"),
            Some(saved)
        );

        drop(database);
        cleanup(&path);
    }

    #[test]
    fn concurrent_revision_writers_commit_once_and_return_a_typed_conflict() {
        let (database, path) = flow_database_fixture();
        let (document, compiled, hash) = revision_one("Concurrent");
        let first = database
            .save_flow_revision(None, &document, &compiled, &hash)
            .expect("save initial revision");
        drop(database);

        let barrier = Arc::new(Barrier::new(2));
        let handles = ["Writer A", "Writer B"].map(|name| {
            let database = Database::open(&path).expect("concurrent database handle");
            let mut document = first.document.clone();
            document.revision = 2;
            document.name = name.into();
            let mut compiled = first.compiled_plan.clone();
            compiled.revision = 2;
            let hash = compiled_plan_sha256(&compiled).expect("concurrent plan hash");
            let barrier = Arc::clone(&barrier);
            std::thread::spawn(move || {
                barrier.wait();
                database.save_flow_revision(Some(1), &document, &compiled, &hash)
            })
        });
        let results = handles.map(|handle| handle.join().expect("writer thread"));
        assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
        let conflict = results
            .iter()
            .find_map(|result| result.as_ref().err())
            .expect("one writer conflict");
        let conflict = conflict
            .downcast_ref::<crate::RevisionConflict>()
            .expect("typed revision conflict");
        assert_eq!((conflict.expected, conflict.actual), (1, 2));

        let database = Database::open(&path).expect("inspect concurrent save");
        let latest = database
            .get_flow_revision(first.document.id, None)
            .expect("latest concurrent revision")
            .expect("saved concurrent revision");
        assert_eq!(latest.document.revision, 2);
        drop(database);
        cleanup(&path);
    }

    #[test]
    fn get_rejects_a_persisted_schema_version_mismatch() {
        let (database, path) = flow_database_fixture();
        let (document, compiled, hash) = revision_one("Corrupt fixture");
        let saved = database
            .save_flow_revision(None, &document, &compiled, &hash)
            .expect("save valid record");
        let mut authoring = serde_json::to_value(&saved.document).expect("authoring value");
        authoring["schemaVersion"] = serde_json::json!(FLOW_SCHEMA_VERSION + 1);
        let connection = database.conn().expect("corrupt persisted schema version");
        connection
            .execute(
                "UPDATE flow_revisions SET authoring_json=?3 WHERE flow_id=?1 AND revision=?2",
                rusqlite::params![
                    saved.document.id.to_string(),
                    1,
                    serde_json::to_string(&authoring).expect("corrupt authoring JSON")
                ],
            )
            .expect("write corrupt authoring JSON");
        drop(connection);

        let error = database
            .get_flow_revision(saved.document.id, Some(1))
            .expect_err("schema mismatch must fail");
        assert!(error.to_string().contains("schema version mismatch"));

        drop(database);
        cleanup(&path);
    }
}
