//! What the operator has recorded about the fleet: each phone's alias and number, and the
//! groups they are organised into.

use super::*;

impl Database {
    /// The columns of one `device_meta` row, in the order both readers below bind them.
    /// One constant so a column added to the table cannot be added to one reader only —
    /// which is how `handle` came to be selected by the single-row read and not by anything
    /// else for a while.
    const DEVICE_META_COLUMNS: &'static str =
        "udid, notes, tags_json, group_id, handle, alias, number";

    fn device_meta_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<crate::types::DeviceMeta> {
        let tags_json: String = row.get(2)?;
        Ok(crate::types::DeviceMeta {
            udid: row.get(0)?,
            notes: row.get(1)?,
            tags: serde_json::from_str(&tags_json).unwrap_or_default(),
            group_id: row.get(3)?,
            handle: row.get(4)?,
            alias: row.get(5)?,
            number: row.get(6)?,
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
        // **`filter_map(|row| row.ok())` made corruption look like absence.** A phone whose
        // `tags_json` is malformed, or whose `number` is outside the target type, produced an
        // error from `device_meta_from_row`; discarding it returned `Ok` with every *other*
        // phone, and the UI then treated that phone as simply having no stored metadata and
        // fell back to device-reported values. Its notes, tags, group, handle and alias were
        // silently ignored while the row sat in the database intact.
        //
        // That also defeats `narrow`, which exists in this file specifically to fail closed on
        // an out-of-range integer rather than truncate it.
        //
        // Found by an independent review on 27/08/2026.
        let rows = stmt.query_map([], Self::device_meta_from_row)?;
        rows.collect::<Result<Vec<_>, _>>()
            .context("đọc device_meta: có dòng không đọc được")
    }
    pub fn upsert_device_meta(&self, meta: &crate::types::DeviceMeta) -> anyhow::Result<()> {
        let conn = self.conn()?;
        conn.execute(
            r#"INSERT INTO device_meta (udid, notes, tags_json, group_id, handle, alias, number)
               VALUES (?1,?2,?3,?4,?5,?6,?7)
               ON CONFLICT(udid) DO UPDATE SET
                 notes=excluded.notes, tags_json=excluded.tags_json,
                 group_id=excluded.group_id,
                 handle=excluded.handle, alias=excluded.alias,
                 number=excluded.number"#,
            params![
                meta.udid,
                meta.notes,
                serde_json::to_string(&meta.tags)?,
                meta.group_id,
                meta.handle,
                meta.alias,
                meta.number
            ],
        )?;
        Ok(())
    }

    pub fn patch_device_meta(
        &self,
        udid: &str,
        change: &crate::DeviceMetaChange,
    ) -> anyhow::Result<crate::DeviceMeta> {
        anyhow::ensure!(!udid.trim().is_empty(), "device identifier missing");
        {
            let conn = self.conn()?;
            match change {
                crate::DeviceMetaChange::Alias(value) => {
                    conn.execute("INSERT INTO device_meta(udid,alias) VALUES(?1,?2) ON CONFLICT(udid) DO UPDATE SET alias=excluded.alias",params![udid,value])?;
                }
                crate::DeviceMetaChange::Number(value) => {
                    anyhow::ensure!(
                        value.is_none_or(|number| number > 0),
                        "device number must be positive"
                    );
                    conn.execute("INSERT INTO device_meta(udid,number) VALUES(?1,?2) ON CONFLICT(udid) DO UPDATE SET number=excluded.number",params![udid,value])?;
                }
            }
        }
        self.get_device_meta(udid)
    }

    /// Update only the account mapping, rejecting stale editors without overwriting aliases/groups.
    pub fn set_device_handle(
        &self,
        udid: &str,
        expected: &str,
        handle: &str,
    ) -> anyhow::Result<String> {
        let handle = handle.trim().trim_start_matches('@');
        anyhow::ensure!(
            handle.is_empty()
                || (handle.len() <= 24
                    && !handle.ends_with('.')
                    && handle
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'.')),
            "invalid TikTok username"
        );
        anyhow::ensure!(!udid.trim().is_empty(), "device identifier missing");
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let current: Option<String> = tx
            .query_row(
                "SELECT handle FROM device_meta WHERE udid=?1",
                [udid],
                |row| row.get(0),
            )
            .optional()?;
        anyhow::ensure!(
            current.as_deref().unwrap_or_default() == expected,
            "device account mapping changed; reload before saving"
        );
        if !handle.is_empty() {
            let duplicate:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM device_meta WHERE udid<>?1 AND lower(ltrim(trim(handle),'@'))=lower(?2))",params![udid,handle],|row|row.get(0))?;
            anyhow::ensure!(
                !duplicate,
                "TikTok username is already assigned to another device"
            );
        }
        tx.execute("INSERT INTO device_meta(udid,handle) VALUES(?1,?2) ON CONFLICT(udid) DO UPDATE SET handle=excluded.handle",params![udid,handle])?;
        tx.commit()?;
        Ok(handle.to_string())
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
            // **`ORDER BY` on purpose, even though the rows come back sorted anyway.**
            //
            // Without it this reads through `sqlite_autoindex_group_members_1` — a covering
            // index on `(group_id, udid)` — so the order is udid-ascending by accident of the
            // query plan, and nothing in the schema promises it stays that way. That order is
            // not decoration: `InteractionActorPicker` loads a group straight into the actor
            // list, and the actor list is what decides who replies to whom in a `Chain`. An
            // order nobody wrote down is an order that changes under an index change.
            //
            // udid is the stable tie-break; the *meaningful* order is the operator's device
            // number, which lives in `device_meta` and is applied by the frontend where the
            // numbers are already in hand.
            let mut mstmt =
                conn.prepare("SELECT udid FROM group_members WHERE group_id = ?1 ORDER BY udid")?;
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
