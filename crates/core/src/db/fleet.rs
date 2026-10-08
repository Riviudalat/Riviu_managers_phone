//! What the operator has recorded about the fleet: each phone's alias and number, and the
//! groups they are organised into.

use super::*;

/// Saved mapping captured by the transaction rejecting a new account assignment.
#[derive(Debug, Clone, serde::Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ConflictingAccountDevice {
    pub udid: String,
    pub number: Option<u32>,
    pub alias: String,
    pub handle: String,
}

/// Collision context is a snapshot of saved mappings, not live account identity.
#[derive(Debug, Clone, serde::Serialize, thiserror::Error, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
#[error(
    "TikTok username is already assigned to another device: @{attempted_handle} (target {udid})"
)]
pub struct AccountAssignmentConflict {
    pub udid: String,
    pub attempted_handle: String,
    pub expected_handle: String,
    pub current_handle: String,
    pub conflicting_devices: Vec<ConflictingAccountDevice>,
    pub conflicts_truncated: bool,
}

/// A backend-observed identity and its exact pre-observation stored value.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountMappingObservation {
    pub udid: String,
    pub expected_handle: String,
    pub observed_handle: String,
}

/// Portable operator metadata only; no credentials or runtime/device state.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceMetadataTransfer {
    pub namespace: String,
    pub version: u32,
    pub high_water: u32,
    pub devices: Vec<crate::DeviceMeta>,
    pub groups: Vec<crate::DeviceGroup>,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceMetadataImportPreview {
    pub conflicts: Vec<String>,
    pub applied: bool,
}

impl Database {
    pub fn export_device_metadata(&self) -> anyhow::Result<DeviceMetadataTransfer> {
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        let devices = read_device_metas(&tx)?;
        let mut groups = {
            let mut stmt = tx.prepare("SELECT id,name,color,created_at FROM groups ORDER BY id")?;
            let rows = stmt
                .query_map([], |r| {
                    Ok(crate::DeviceGroup {
                        id: r.get(0)?,
                        name: r.get(1)?,
                        color: r.get(2)?,
                        created_at: r.get(3)?,
                        udids: Vec::new(),
                    })
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            rows
        };
        for group in &mut groups {
            let mut stmt =
                tx.prepare("SELECT udid FROM group_members WHERE group_id=?1 ORDER BY udid")?;
            group.udids = stmt
                .query_map([&group.id], |r| r.get(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
        }
        let high_water = tx.query_row(
            "SELECT high_water FROM device_number_sequence WHERE id=1",
            [],
            |r| r.get(0),
        )?;
        tx.commit()?;
        Ok(DeviceMetadataTransfer {
            namespace: "riviu.device-meta".into(),
            version: 1,
            high_water,
            devices,
            groups,
        })
    }

    /// Preview and apply share validation. Apply rechecks under the same write lock;
    /// different existing mappings must be resolved explicitly, never overwritten.
    pub fn import_device_metadata(
        &self,
        input: &DeviceMetadataTransfer,
        apply: bool,
    ) -> anyhow::Result<DeviceMetadataImportPreview> {
        anyhow::ensure!(
            input.namespace == "riviu.device-meta" && input.version == 1,
            "unsupported device metadata namespace/version"
        );
        anyhow::ensure!(
            input.devices.len() <= 10000
                && input.groups.len() <= 10000
                && input.groups.iter().all(|group| group.udids.len() <= 10000),
            "device metadata import too large"
        );
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut conflicts = Vec::new();
        let mut serials = std::collections::HashSet::new();
        let mut numbers = std::collections::HashSet::new();
        for meta in &input.devices {
            if meta.udid.trim().is_empty() || !serials.insert(&meta.udid) {
                conflicts.push(format!("duplicate or empty serial: {}", meta.udid));
            }
            if let Some(number) = meta.number {
                if number == 0 || number > input.high_water || !numbers.insert(number) {
                    conflicts.push(format!("invalid or duplicate device number: {number}"));
                }
                let owner: Option<String> = tx.query_row("SELECT udid FROM device_meta WHERE number=?1 AND udid<>?2 ORDER BY udid LIMIT 1",
                    params![number,meta.udid],|r|r.get(0)).optional()?;
                if let Some(owner) = owner {
                    conflicts.push(format!("device number {number} belongs to {owner}"));
                }
            }
            let old = tx
                .query_row(
                    &format!(
                        "SELECT {} FROM device_meta WHERE udid=?1",
                        Self::DEVICE_META_COLUMNS
                    ),
                    [&meta.udid],
                    Self::device_meta_from_row,
                )
                .optional()?;
            if let Some(old) = old {
                if serde_json::to_value(old)? != serde_json::to_value(meta)? {
                    conflicts.push(format!("saved metadata differs for {}", meta.udid));
                }
            }
            // Legacy group_id is independent of authoritative group_members.
            // write_group/delete_group do not synchronize it; preserve it verbatim.
        }
        let mut group_ids = std::collections::HashSet::new();
        let mut members = std::collections::HashSet::new();
        for group in &input.groups {
            if group.id.trim().is_empty() || !group_ids.insert(&group.id) {
                conflicts.push(format!("duplicate or empty group: {}", group.id));
            }
            let old: Option<(String, String, String)> = tx
                .query_row(
                    "SELECT name,color,created_at FROM groups WHERE id=?1",
                    [&group.id],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )
                .optional()?;
            if let Some(old) = old {
                let mut stmt =
                    tx.prepare("SELECT udid FROM group_members WHERE group_id=?1 ORDER BY udid")?;
                let old_members = stmt
                    .query_map([&group.id], |r| r.get::<_, String>(0))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                let mut expected = group.udids.clone();
                expected.sort();
                if old
                    != (
                        group.name.clone(),
                        group.color.clone(),
                        group.created_at.clone(),
                    )
                    || old_members != expected
                {
                    conflicts.push(format!("saved group differs: {}", group.id));
                }
            }
            for serial in &group.udids {
                if serial.trim().is_empty() || !members.insert(serial) {
                    conflicts.push(format!("duplicate or empty group member: {serial}"));
                }
                let owner: Option<String> = tx
                    .query_row(
                        "SELECT group_id FROM group_members WHERE udid=?1 AND group_id<>?2 LIMIT 1",
                        params![serial, group.id],
                        |r| r.get(0),
                    )
                    .optional()?;
                if let Some(owner) = owner {
                    conflicts.push(format!("{serial} belongs to group {owner}"));
                }
            }
        }
        if apply {
            anyhow::ensure!(
                conflicts.is_empty(),
                "device metadata import conflicts: {}",
                conflicts.join("; ")
            );
            for meta in &input.devices {
                write_device_meta(&tx, meta)?;
            }
            for group in &input.groups {
                write_group(&tx, group)?;
            }
            tx.execute(
                "UPDATE device_number_sequence SET high_water=MAX(high_water,?1) WHERE id=1",
                [input.high_water],
            )?;
            tx.commit()?;
        }
        Ok(DeviceMetadataImportPreview {
            conflicts,
            applied: apply,
        })
    }

    /// Allocate only missing numbers under one SQLite write lock. The sequence includes
    /// offline rows and survives explicit clearing, lowering or deleting a saved number.
    /// Return the complete saved roster so consumers retain offline mappings.
    pub fn ensure_device_numbers(
        &self,
        udids: &[String],
    ) -> anyhow::Result<Vec<crate::DeviceMeta>> {
        anyhow::ensure!(
            udids.iter().all(|id| !id.trim().is_empty()),
            "device identifier missing"
        );
        let serials: std::collections::BTreeSet<_> = udids.iter().collect();
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut highest: u32 = tx.query_row(
            "SELECT high_water FROM device_number_sequence WHERE id=1",
            [],
            |row| row.get(0),
        )?;
        for udid in serials {
            let current: Option<Option<u32>> = tx
                .query_row(
                    "SELECT number FROM device_meta WHERE udid=?1",
                    [udid],
                    |row| row.get(0),
                )
                .optional()?;
            if current.flatten().is_some() {
                continue;
            }
            highest = highest
                .checked_add(1)
                .context("device number allocation exhausted")?;
            tx.execute(
                "INSERT INTO device_meta(udid,number) VALUES(?1,?2)
                 ON CONFLICT(udid) DO UPDATE SET number=excluded.number",
                params![udid, highest],
            )?;
        }
        let rows = read_device_metas(&tx)?;
        tx.commit()?;
        Ok(rows)
    }

    /// Validate the complete final mapping before writing any slot. Callers own
    /// observation provenance/freshness; this boundary owns exact CAS and collisions.
    pub fn reconcile_account_mappings(
        &self,
        observations: &[AccountMappingObservation],
        apply: bool,
    ) -> anyhow::Result<()> {
        anyhow::ensure!(!observations.is_empty(), "no verified account observations");
        let normalize = |value: &str| value.trim().trim_start_matches('@').to_ascii_lowercase();
        let mut serials = std::collections::HashSet::new();
        let mut handles = std::collections::HashSet::new();
        for item in observations {
            anyhow::ensure!(
                !item.udid.trim().is_empty() && serials.insert(&item.udid),
                "duplicate or empty device identifier"
            );
            let handle = &item.observed_handle;
            anyhow::ensure!(
                !handle.is_empty()
                    && handle.len() <= 24
                    && !handle.ends_with('.')
                    && handle
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'.'),
                "invalid observed TikTok username"
            );
            anyhow::ensure!(
                handles.insert(normalize(handle)),
                "multiple phones observed the same TikTok account; mappings unchanged"
            );
        }
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        for item in observations {
            let current: Option<String> = tx
                .query_row(
                    "SELECT handle FROM device_meta WHERE udid=?1",
                    [&item.udid],
                    |row| row.get(0),
                )
                .optional()?;
            anyhow::ensure!(
                current.as_deref().unwrap_or_default() == item.expected_handle,
                "device account mapping changed for {}; read again before reconciling",
                item.udid
            );
            let mut stmt = tx.prepare("SELECT udid, number, alias, handle FROM device_meta WHERE udid<>?1 AND lower(ltrim(trim(handle),'@'))=lower(?2) ORDER BY udid")?;
            let conflicts = stmt
                .query_map(params![item.udid, item.observed_handle], |row| {
                    Ok(ConflictingAccountDevice {
                        udid: row.get(0)?,
                        number: row.get(1)?,
                        alias: row.get(2)?,
                        handle: row.get(3)?,
                    })
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            let blocked: Vec<_> = conflicts
                .into_iter()
                .filter(|slot| {
                    !observations.iter().any(|proof| {
                        proof.udid == slot.udid
                            && proof.expected_handle == slot.handle
                            && normalize(&proof.observed_handle) != normalize(&slot.handle)
                    })
                })
                .collect();
            if !blocked.is_empty() {
                return Err(AccountAssignmentConflict {
                    udid: item.udid.clone(),
                    attempted_handle: item.observed_handle.clone(),
                    expected_handle: item.expected_handle.clone(),
                    current_handle: current.unwrap_or_default(),
                    conflicting_devices: blocked,
                    conflicts_truncated: false,
                }
                .into());
            }
        }
        if apply {
            for item in observations {
                // CAS and collision checks above still cover unchanged observations.
                // Keep them in the command receipt without issuing redundant writes.
                if item.expected_handle == item.observed_handle {
                    continue;
                }
                tx.execute("INSERT INTO device_meta(udid,handle) VALUES(?1,?2) ON CONFLICT(udid) DO UPDATE SET handle=excluded.handle",
                    params![item.udid, item.observed_handle])?;
            }
            tx.commit()?;
        }
        Ok(())
    }

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
    /// asking per device is twenty IPC round trips for a table that fits in a page.
    /// Discovery persists rows before numeric labels are used; offline rows remain here.
    pub fn list_device_metas(&self) -> anyhow::Result<Vec<crate::types::DeviceMeta>> {
        let conn = self.conn()?;
        read_device_metas(&conn)
    }

    pub fn upsert_device_meta(&self, meta: &crate::types::DeviceMeta) -> anyhow::Result<()> {
        let conn = self.conn()?;
        write_device_meta(&conn, meta)
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
        // Readback of an existing account is not a new assignment. Keep legacy
        // spelling and duplicates untouched, but only after the exact CAS check.
        if let Some(stored) = current.as_ref() {
            if !handle.is_empty()
                && stored
                    .trim()
                    .trim_start_matches('@')
                    .eq_ignore_ascii_case(handle)
            {
                return Ok(stored.clone());
            }
        }
        if !handle.is_empty() {
            // Capture conflicting saved mappings under the same write lock as
            // the collision check; a later roster read could describe another state.
            let mut stmt = tx.prepare(
                "SELECT udid, number, alias, handle FROM device_meta
                 WHERE udid<>?1 AND lower(ltrim(trim(handle),'@'))=lower(?2)
                 ORDER BY udid LIMIT 21",
            )?;
            let mut conflicting_devices = stmt
                .query_map(params![udid, handle], |row| {
                    Ok(ConflictingAccountDevice {
                        udid: row.get(0)?,
                        number: row.get(1)?,
                        alias: row.get(2)?,
                        handle: row.get(3)?,
                    })
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            let conflicts_truncated = conflicting_devices.len() > 20;
            conflicting_devices.truncate(20);
            if !conflicting_devices.is_empty() {
                return Err(AccountAssignmentConflict {
                    udid: udid.to_string(),
                    attempted_handle: handle.to_string(),
                    expected_handle: expected.to_string(),
                    current_handle: current.unwrap_or_default(),
                    conflicting_devices,
                    conflicts_truncated,
                }
                .into());
            }
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

fn write_device_meta(conn: &Connection, meta: &crate::DeviceMeta) -> anyhow::Result<()> {
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

// All roster consumers retain offline rows and fail closed on unreadable metadata.
fn read_device_metas(conn: &Connection) -> anyhow::Result<Vec<crate::DeviceMeta>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {} FROM device_meta ORDER BY udid",
        Database::DEVICE_META_COLUMNS
    ))?;
    let rows = stmt.query_map([], Database::device_meta_from_row)?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .context("read device_meta: unreadable row")
}
