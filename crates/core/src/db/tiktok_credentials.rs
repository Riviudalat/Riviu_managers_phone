//! The non-secret TikTok login index (migration 50).
//!
//! Only the handle, the kind of identifier, its masked form and timestamps live here. The
//! secret itself is in the OS credential store and this module has no way to receive it:
//! every write takes a [`TikTokCredentialIndexEntry`], which has no field for one.

use rusqlite::{params, OptionalExtension};

use super::Database;
use crate::tiktok_credentials::{TikTokCredentialIndexEntry, TikTokIdentifierKind};

fn entry_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<TikTokCredentialIndexEntry> {
    let kind: String = row.get(1)?;
    Ok(TikTokCredentialIndexEntry {
        handle: row.get(0)?,
        identifier_kind: TikTokIdentifierKind::parse(&kind).ok_or_else(|| {
            rusqlite::Error::FromSqlConversionFailure(
                1,
                rusqlite::types::Type::Text,
                format!("unknown identifier kind {kind}").into(),
            )
        })?,
        masked_identifier: row.get(2)?,
        updated_at: row.get(3)?,
        cooldown_until: row.get(4)?,
        last_status: row.get(5)?,
    })
}

const COLUMNS: &str =
    "handle,identifier_kind,masked_identifier,updated_at,cooldown_until,last_status";

impl Database {
    pub fn list_tiktok_credential_index(&self) -> anyhow::Result<Vec<TikTokCredentialIndexEntry>> {
        let conn = self.conn()?;
        let mut statement = conn.prepare(&format!(
            "SELECT {COLUMNS} FROM tiktok_credential_index ORDER BY handle"
        ))?;
        let rows = statement.query_map([], entry_from_row)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn get_tiktok_credential_index(
        &self,
        handle: &str,
    ) -> anyhow::Result<Option<TikTokCredentialIndexEntry>> {
        let conn = self.conn()?;
        Ok(conn
            .query_row(
                &format!("SELECT {COLUMNS} FROM tiktok_credential_index WHERE handle=?1"),
                [handle],
                entry_from_row,
            )
            .optional()?)
    }

    /// Record that a credential was written. Called only **after** the OS credential store
    /// accepted the entry, so an index row never describes a write that did not happen.
    ///
    /// Replacing a credential keeps `cooldown_until` and `last_status`: they describe the
    /// account's recent login history, which a new secret does not erase.
    pub fn upsert_tiktok_credential_index(
        &self,
        entry: &TikTokCredentialIndexEntry,
    ) -> anyhow::Result<TikTokCredentialIndexEntry> {
        let conn = self.conn()?;
        conn.execute(
            "INSERT INTO tiktok_credential_index(handle,identifier_kind,masked_identifier,updated_at)
             VALUES(?1,?2,?3,?4)
             ON CONFLICT(handle) DO UPDATE SET identifier_kind=excluded.identifier_kind,
                masked_identifier=excluded.masked_identifier, updated_at=excluded.updated_at",
            params![
                entry.handle,
                entry.identifier_kind.as_str(),
                entry.masked_identifier,
                entry.updated_at
            ],
        )?;
        drop(conn);
        self.get_tiktok_credential_index(&entry.handle)?
            .ok_or_else(|| anyhow::anyhow!("TikTok credential index row vanished after write"))
    }

    /// Record the latest login outcome for `handle`. `None` clears it: saving a credential
    /// again is the operator's acknowledgement of the previous outcome. Returns false when no
    /// index row exists.
    pub fn set_tiktok_credential_status(
        &self,
        handle: &str,
        status: Option<&str>,
    ) -> anyhow::Result<bool> {
        let conn = self.conn()?;
        Ok(conn.execute(
            "UPDATE tiktok_credential_index SET last_status=?2 WHERE handle=?1",
            params![handle, status],
        )? > 0)
    }

    /// Forget a credential. Called after the OS credential store entry was deleted.
    pub fn delete_tiktok_credential_index(&self, handle: &str) -> anyhow::Result<bool> {
        let conn = self.conn()?;
        Ok(conn.execute(
            "DELETE FROM tiktok_credential_index WHERE handle=?1",
            [handle],
        )? > 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (Database, std::path::PathBuf) {
        let path = std::env::temp_dir().join(format!(
            "riviu-tiktok-credentials-{}.db",
            uuid::Uuid::new_v4()
        ));
        (Database::open(&path).expect("open fixture database"), path)
    }

    fn entry(handle: &str, masked: &str, at: &str) -> TikTokCredentialIndexEntry {
        TikTokCredentialIndexEntry {
            handle: handle.into(),
            identifier_kind: TikTokIdentifierKind::Email,
            masked_identifier: masked.into(),
            updated_at: at.into(),
            cooldown_until: None,
            last_status: None,
        }
    }

    #[test]
    fn replacing_an_entry_keeps_login_history_and_delete_forgets_it() {
        let (db, path) = fixture();
        db.upsert_tiktok_credential_index(&entry("riviu.demo", "th***@gmail.com", "t1"))
            .expect("first write");
        db.conn()
            .unwrap()
            .execute(
                "UPDATE tiktok_credential_index SET cooldown_until='later', last_status='blocked'",
                [],
            )
            .unwrap();

        let replaced = db
            .upsert_tiktok_credential_index(&entry("riviu.demo", "ng***@gmail.com", "t2"))
            .expect("replace");
        assert_eq!(replaced.masked_identifier, "ng***@gmail.com");
        assert_eq!(replaced.updated_at, "t2");
        assert_eq!(replaced.cooldown_until.as_deref(), Some("later"));
        assert_eq!(replaced.last_status.as_deref(), Some("blocked"));
        assert_eq!(db.list_tiktok_credential_index().unwrap().len(), 1);

        assert!(db.delete_tiktok_credential_index("riviu.demo").unwrap());
        assert!(!db.delete_tiktok_credential_index("riviu.demo").unwrap());
        assert!(db.list_tiktok_credential_index().unwrap().is_empty());
        drop(db);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn the_schema_refuses_a_handle_that_is_not_canonical() {
        let (db, path) = fixture();
        for handle in ["Riviu", "@riviu", "a", "user.", "has space"] {
            assert!(
                db.upsert_tiktok_credential_index(&entry(handle, "x***", "t"))
                    .is_err(),
                "{handle}"
            );
        }
        drop(db);
        let _ = std::fs::remove_file(path);
    }
}
