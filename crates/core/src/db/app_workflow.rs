use super::Database;
use crate::app_workflow::{compile_app_profile, AppWorkflowSummary, AppWorkflowV1};
use anyhow::{ensure, Context};
use rusqlite::{params, OptionalExtension, TransactionBehavior};
use uuid::Uuid;

impl Database {
    pub fn list_app_workflows(&self) -> anyhow::Result<Vec<AppWorkflowSummary>> {
        let conn = self.conn()?;
        let mut statement=conn.prepare("SELECT document_json,updated_at FROM app_workflow_documents WHERE archived=0 ORDER BY updated_at DESC,id")?;
        let records = statement
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        records
            .into_iter()
            .map(|(raw, updated_at)| {
                let doc: AppWorkflowV1 = serde_json::from_str(&raw)?;
                Ok(AppWorkflowSummary {
                    id: doc.id,
                    name: doc.name,
                    kind: doc.kind,
                    latest_revision: doc.revision,
                    updated_at,
                })
            })
            .collect()
    }
    pub fn get_app_workflow(
        &self,
        id: Uuid,
        revision: Option<u64>,
    ) -> anyhow::Result<Option<AppWorkflowV1>> {
        let conn = self.conn()?;
        let raw: Option<String> = if let Some(revision) = revision {
            conn.query_row("SELECT document_json FROM app_workflow_revisions WHERE document_id=?1 AND revision=?2",params![id.to_string(),revision],|row|row.get(0)).optional()?
        } else {
            conn.query_row(
                "SELECT document_json FROM app_workflow_documents WHERE id=?1",
                [id.to_string()],
                |row| row.get(0),
            )
            .optional()?
        };
        raw.map(|raw| serde_json::from_str(&raw).map_err(Into::into))
            .transpose()
    }
    pub fn save_app_workflow(
        &self,
        mut doc: AppWorkflowV1,
        expected: Option<u64>,
    ) -> anyhow::Result<AppWorkflowV1> {
        compile_app_profile(&doc)?;
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let current: Option<(u64, bool)> = tx
            .query_row(
                "SELECT revision,archived FROM app_workflow_documents WHERE id=?1",
                [doc.id.to_string()],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        ensure!(
            current.map(|r| r.0) == expected,
            "App revision conflict; reload before saving"
        );
        ensure!(!current.is_some_and(|r| r.1), "App is archived");
        doc.revision = expected
            .unwrap_or(0)
            .checked_add(1)
            .context("Revision overflow")?;
        let raw = serde_json::to_string(&doc)?;
        let now = chrono::Utc::now().to_rfc3339();
        tx.execute("INSERT INTO app_workflow_documents(id,revision,archived,document_json,updated_at) VALUES(?1,?2,0,?3,?4) ON CONFLICT(id) DO UPDATE SET revision=excluded.revision,document_json=excluded.document_json,updated_at=excluded.updated_at",params![doc.id.to_string(),doc.revision,raw,now])?;
        tx.execute("INSERT INTO app_workflow_revisions(document_id,revision,document_json) VALUES(?1,?2,?3)",params![doc.id.to_string(),doc.revision,raw])?;
        tx.commit()?;
        Ok(doc)
    }
    pub fn archive_app_workflow(&self, id: Uuid, revision: u64) -> anyhow::Result<()> {
        ensure!(self.conn()?.execute("UPDATE app_workflow_documents SET archived=1 WHERE id=?1 AND revision=?2 AND archived=0",params![id.to_string(),revision])?==1,"App revision conflict");
        Ok(())
    }
}
