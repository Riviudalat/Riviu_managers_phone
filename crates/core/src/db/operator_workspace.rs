use super::Database;
use crate::operator_workspace::{OperatorRecord, OperatorRecordInput, OperatorRecordKind};
use anyhow::{ensure, Context};
use rusqlite::{params, OptionalExtension, TransactionBehavior};
use uuid::Uuid;

impl Database {
    pub fn operator_import(
        &self,
        inputs: Vec<OperatorRecordInput>,
    ) -> anyhow::Result<Vec<OperatorRecord>> {
        ensure!(
            !inputs.is_empty() && inputs.len() <= 1000,
            "Import 1-1000 records at a time"
        );
        let mut identities = std::collections::BTreeSet::new();
        for input in &inputs {
            input.validate()?;
            ensure!(
                input.expected_revision.is_none(),
                "Imports create new records"
            );
            ensure!(identities.insert(input.id), "Duplicate imported identity");
        }
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let now = chrono::Utc::now().to_rfc3339();
        let mut records = Vec::new();
        for input in inputs {
            let record = OperatorRecord {
                id: input.id,
                kind: input.kind,
                name: input.name.trim().to_string(),
                revision: 1,
                data: input.data,
                archived: false,
                created_at: now.clone(),
                updated_at: now.clone(),
            };
            let raw = serde_json::to_string(&record)?;
            tx.execute("INSERT INTO operator_records(id,kind,name,revision,archived,record_json,updated_at) VALUES(?1,?2,?3,1,0,?4,?5)",params![record.id.to_string(),record.kind.as_str(),record.name,raw,now])?;
            tx.execute("INSERT INTO operator_record_revisions(record_id,revision,record_json) VALUES(?1,1,?2)",params![record.id.to_string(),raw])?;
            records.push(record);
        }
        tx.commit()?;
        Ok(records)
    }
    pub fn operator_list(
        &self,
        kind: OperatorRecordKind,
        search: &str,
    ) -> anyhow::Result<Vec<OperatorRecord>> {
        ensure!(search.len() <= 256, "Search is too long");
        let conn = self.conn()?;
        let mut statement = conn.prepare("SELECT record_json FROM operator_records WHERE kind=?1 AND archived=0 AND (?2='' OR instr(lower(name),lower(?2))>0) ORDER BY updated_at DESC,id LIMIT 1000")?;
        let data = statement
            .query_map(params![kind.as_str(), search.trim()], |row| {
                row.get::<_, String>(0)
            })?
            .collect::<Result<Vec<_>, _>>()?;
        data.into_iter()
            .map(|value| serde_json::from_str(&value).map_err(Into::into))
            .collect()
    }

    pub fn operator_get(&self, id: Uuid) -> anyhow::Result<Option<OperatorRecord>> {
        self.conn()?
            .query_row(
                "SELECT record_json FROM operator_records WHERE id=?1",
                [id.to_string()],
                |row| row.get::<_, String>(0),
            )
            .optional()?
            .map(|raw| serde_json::from_str(&raw).map_err(Into::into))
            .transpose()
    }

    pub fn operator_save(&self, input: OperatorRecordInput) -> anyhow::Result<OperatorRecord> {
        input.validate()?;
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let previous = tx
            .query_row(
                "SELECT record_json FROM operator_records WHERE id=?1",
                [input.id.to_string()],
                |row| row.get::<_, String>(0),
            )
            .optional()?
            .map(|raw| serde_json::from_str::<OperatorRecord>(&raw))
            .transpose()?;
        ensure!(
            previous.as_ref().map(|record| record.revision) == input.expected_revision,
            "Record revision conflict; reload before saving"
        );
        ensure!(
            previous
                .as_ref()
                .is_none_or(|record| record.kind == input.kind && !record.archived),
            "Archived record or kind mismatch"
        );
        let now = chrono::Utc::now().to_rfc3339();
        let record = OperatorRecord {
            id: input.id,
            kind: input.kind,
            name: input.name.trim().to_string(),
            revision: input
                .expected_revision
                .unwrap_or(0)
                .checked_add(1)
                .context("Revision overflow")?,
            data: input.data,
            archived: false,
            created_at: previous
                .map(|record| record.created_at)
                .unwrap_or_else(|| now.clone()),
            updated_at: now,
        };
        let raw = serde_json::to_string(&record)?;
        tx.execute("INSERT INTO operator_records(id,kind,name,revision,archived,record_json,updated_at) VALUES(?1,?2,?3,?4,0,?5,?6) ON CONFLICT(id) DO UPDATE SET name=excluded.name,revision=excluded.revision,record_json=excluded.record_json,updated_at=excluded.updated_at",params![record.id.to_string(),record.kind.as_str(),record.name,record.revision,raw,record.updated_at])?;
        tx.execute("INSERT INTO operator_record_revisions(record_id,revision,record_json) VALUES(?1,?2,?3)",params![record.id.to_string(),record.revision,raw])?;
        tx.commit()?;
        Ok(record)
    }

    pub fn operator_archive(&self, id: Uuid, expected_revision: u64) -> anyhow::Result<()> {
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let raw: String = tx.query_row(
            "SELECT record_json FROM operator_records WHERE id=?1 AND archived=0",
            [id.to_string()],
            |row| row.get(0),
        )?;
        let mut record: OperatorRecord = serde_json::from_str(&raw)?;
        ensure!(
            record.revision == expected_revision,
            "Record revision conflict; reload before archiving"
        );
        record.revision = record
            .revision
            .checked_add(1)
            .context("Revision overflow")?;
        record.archived = true;
        record.updated_at = chrono::Utc::now().to_rfc3339();
        let raw = serde_json::to_string(&record)?;
        tx.execute("UPDATE operator_records SET archived=1,revision=?2,record_json=?3,updated_at=?4 WHERE id=?1", params![id.to_string(),record.revision,raw,record.updated_at])?;
        tx.execute("INSERT INTO operator_record_revisions(record_id,revision,record_json) VALUES(?1,?2,?3)",params![id.to_string(),record.revision,raw])?;
        tx.commit()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn records_keep_revisions_and_reject_stale_updates_and_secrets() {
        let path = std::env::temp_dir().join(format!("riviu-operator-{}.db", Uuid::new_v4()));
        let db = Database::open(&path).unwrap();
        let input = OperatorRecordInput {
            id: Uuid::new_v4(),
            kind: OperatorRecordKind::Account,
            name: "Account one".into(),
            expected_revision: None,
            data: serde_json::json!({"username":"example","platform":"tiktok","deviceIds":["phone-1"]}),
        };
        let first = db.operator_save(input.clone()).unwrap();
        assert_eq!(first.revision, 1);
        assert!(db.operator_save(input.clone()).is_err());
        let mut second = input.clone();
        second.expected_revision = Some(1);
        second.name = "Renamed account".into();
        assert_eq!(db.operator_save(second.clone()).unwrap().revision, 2);
        assert!(db.operator_archive(first.id, 1).is_err());
        db.operator_archive(first.id, 2).unwrap();
        assert!(db
            .operator_list(OperatorRecordKind::Account, "")
            .unwrap()
            .is_empty());
        assert!(db.operator_get(first.id).unwrap().unwrap().archived);
        second.id = Uuid::new_v4();
        second.expected_revision = None;
        second.data["password"] = "not-for-sqlite".into();
        assert!(db.operator_save(second).is_err());
        assert_eq!(
            db.conn()
                .unwrap()
                .query_row("SELECT count(*) FROM operator_record_revisions", [], |r| {
                    r.get::<_, i64>(0)
                })
                .unwrap(),
            3
        );
    }
}
