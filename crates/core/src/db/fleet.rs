//! What the operator has recorded about the fleet: each phone's alias and number, and the
//! groups they are organised into.

use super::*;

impl Database {
    /// The columns of one `device_meta` row, in the order both readers below bind them.
    /// One constant so a column added to the table cannot be added to one reader only —
    /// which is how `handle` came to be selected by the single-row read and not by anything
    /// else for a while.
    const DEVICE_META_COLUMNS: &'static str =
        "udid, notes, tags_json, group_id, proxy_id, handle, alias, number";

    fn device_meta_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<crate::types::DeviceMeta> {
        let tags_json: String = row.get(2)?;
        Ok(crate::types::DeviceMeta {
            udid: row.get(0)?,
            notes: row.get(1)?,
            tags: serde_json::from_str(&tags_json).unwrap_or_default(),
            group_id: row.get(3)?,
            proxy_id: row.get(4)?,
            handle: row.get(5)?,
            alias: row.get(6)?,
            number: row.get(7)?,
        })
    }
    pub fn get_device_meta(&self, udid: &str) -> anyhow::Result<crate::types::DeviceMeta> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(&format!(
            "SELECT {} FROM device_meta WHERE udid = ?1",
            Self::DEVICE_META_COLUMNS
        ))?;
        let mut rows = stmt.query(params![udid])?;
        if let Some(row) = rows.next()? {
            Ok(Self::device_meta_from_row(row)?)
        } else {
            Ok(crate::types::DeviceMeta {
                udid: udid.to_string(),
                notes: String::new(),
                tags: vec![],
                group_id: None,
                proxy_id: None,
                handle: String::new(),
                alias: String::new(),
                number: None,
            })
        }
    }
    /// Every phone this app has a record for, in one read.
    ///
    /// The grid needs the alias and the number of *twenty* phones to draw one frame, and
    /// asking per device is twenty IPC round trips for a table that fits in a page. Rows
    /// exist only for phones somebody has edited, so a fleet with no records answers empty
    /// and every tile falls back to what the phone reports.
    pub fn list_device_metas(&self) -> anyhow::Result<Vec<crate::types::DeviceMeta>> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(&format!(
            "SELECT {} FROM device_meta",
            Self::DEVICE_META_COLUMNS
        ))?;
        let rows = stmt.query_map([], Self::device_meta_from_row)?;
        Ok(rows.filter_map(|row| row.ok()).collect())
    }
    pub fn upsert_device_meta(&self, meta: &crate::types::DeviceMeta) -> anyhow::Result<()> {
        let conn = self.conn()?;
        conn.execute(
            r#"INSERT INTO device_meta (udid, notes, tags_json, group_id, proxy_id, handle, alias, number)
               VALUES (?1,?2,?3,?4,?5,?6,?7,?8)
               ON CONFLICT(udid) DO UPDATE SET
                 notes=excluded.notes, tags_json=excluded.tags_json,
                 group_id=excluded.group_id, proxy_id=excluded.proxy_id,
                 handle=excluded.handle, alias=excluded.alias,
                 number=excluded.number"#,
            params![
                meta.udid,
                meta.notes,
                serde_json::to_string(&meta.tags)?,
                meta.group_id,
                meta.proxy_id,
                meta.handle,
                meta.alias,
                meta.number
            ],
        )?;
        Ok(())
    }
    pub fn list_groups(&self) -> anyhow::Result<Vec<crate::types::DeviceGroup>> {
        let conn = self.conn()?;
        let mut stmt =
            conn.prepare("SELECT id, name, color, created_at FROM groups ORDER BY name")?;
        let groups: Vec<(String, String, String, String)> = stmt
            .query_map([], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
            })?
            .collect::<rusqlite::Result<_>>()?;
        let mut out = Vec::new();
        for (id, name, color, created_at) in groups {
            let mut mstmt = conn.prepare("SELECT udid FROM group_members WHERE group_id = ?1")?;
            let udids: Vec<String> = mstmt
                .query_map(params![id], |row| row.get(0))?
                .collect::<rusqlite::Result<_>>()?;
            out.push(crate::types::DeviceGroup {
                id,
                name,
                color,
                udids,
                created_at,
            });
        }
        Ok(out)
    }
    /// Replace a group and its membership, **atomically**.
    ///
    /// The membership rewrite is a delete-everything-then-rebuild, and it used to run in
    /// autocommit: the `DELETE` was durable the instant it returned, so anything that went
    /// wrong in the insert loop left the group **empty and saved that way**. Adding one phone
    /// to a group could erase it.
    ///
    /// The permanent erase needs an error mid-loop and is rare. The everyday version is not:
    /// any `list_groups` landing in the window between the delete and the last insert reads a
    /// group with no members, and the tab strip renders it as an empty tab. One transaction
    /// closes both.
    ///
    /// `Immediate` is load-bearing rather than decoration — a deferred transaction that
    /// upgrades to a write can be refused `SQLITE_BUSY` **without** the busy handler running.
    /// Same idiom as `create_publish_campaign` and `create_interaction_campaign` below.
    pub fn upsert_group(&self, group: &crate::types::DeviceGroup) -> anyhow::Result<()> {
        let mut conn = self.conn()?;
        let transaction = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        write_group(&transaction, group)?;
        transaction.commit()?;
        Ok(())
    }
    pub fn delete_group(&self, id: &str) -> anyhow::Result<()> {
        let mut conn = self.conn()?;
        let transaction = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        erase_group(&transaction, id)?;
        transaction.commit()?;
        Ok(())
    }
}
