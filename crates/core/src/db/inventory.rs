//! The lists the farm pages edit: media, the app library and the schedule.
//!
//! All the same shape — list, upsert, delete against one table — which is why they sit
//! together rather than in four files of thirty lines.

use super::*;
use sha2::{Digest, Sha256};
use std::io::Read;

impl Database {
    pub fn list_materials(&self) -> anyhow::Result<Vec<crate::types::MaterialItem>> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT id, name, path, kind, size, created_at FROM materials ORDER BY created_at DESC",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(crate::types::MaterialItem {
                id: row.get(0)?,
                name: row.get(1)?,
                path: row.get(2)?,
                kind: row.get(3)?,
                size: row.get::<_, i64>(4)? as u64,
                created_at: row.get(5)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }
    pub fn add_material(&self, item: &crate::types::MaterialItem) -> anyhow::Result<()> {
        let conn = self.conn()?;
        conn.execute(
            "INSERT INTO materials (id, name, path, kind, size, created_at) VALUES (?1,?2,?3,?4,?5,?6)",
            params![
                item.id,
                item.name,
                item.path,
                item.kind,
                item.size as i64,
                item.created_at
            ],
        )?;
        Ok(())
    }
    pub fn delete_material(&self, id: &str) -> anyhow::Result<()> {
        let conn = self.conn()?;
        conn.execute("DELETE FROM materials WHERE id = ?1", params![id])?;
        Ok(())
    }
    pub fn list_apps_library(&self) -> anyhow::Result<Vec<crate::types::AppLibraryItem>> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT id, name, path, bundle_id, version, platform, package_format, artifact_kind, application_id, version_name, version_code, sha256, size_bytes, signer_sha256, icon_png_base64, metadata_status, metadata_error, created_at FROM apps_library ORDER BY name",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(crate::types::AppLibraryItem {
                id: row.get(0)?,
                name: row.get(1)?,
                path: row.get(2)?,
                bundle_id: row.get(3)?,
                version: row.get(4)?,
                platform: match row.get::<_, String>(5)?.as_str() {
                    "ios" => crate::types::AppLibraryPlatform::Ios,
                    "android" => crate::types::AppLibraryPlatform::Android,
                    unexpected => {
                        return Err(rusqlite::Error::FromSqlConversionFailure(
                            5,
                            rusqlite::types::Type::Text,
                            format!("unknown app platform {unexpected:?}").into(),
                        ))
                    }
                },
                package_format: match row.get::<_, String>(6)?.as_str() {
                    "ipa" => crate::types::AppPackageFormat::Ipa,
                    "apk" => crate::types::AppPackageFormat::Apk,
                    "xapk" => crate::types::AppPackageFormat::Xapk,
                    "apkm" => crate::types::AppPackageFormat::Apkm,
                    "apks" => crate::types::AppPackageFormat::Apks,
                    unexpected => {
                        return Err(rusqlite::Error::FromSqlConversionFailure(
                            6,
                            rusqlite::types::Type::Text,
                            format!("unknown app package format {unexpected:?}").into(),
                        ))
                    }
                },
                artifact_kind: match row.get::<_, String>(7)?.as_str() {
                    "ipa" => crate::types::AppPackageFormat::Ipa,
                    "apk" => crate::types::AppPackageFormat::Apk,
                    "xapk" => crate::types::AppPackageFormat::Xapk,
                    "apkm" => crate::types::AppPackageFormat::Apkm,
                    "apks" => crate::types::AppPackageFormat::Apks,
                    unexpected => {
                        return Err(rusqlite::Error::FromSqlConversionFailure(
                            7,
                            rusqlite::types::Type::Text,
                            format!("unknown app artifact kind {unexpected:?}").into(),
                        ))
                    }
                },
                application_id: row.get(8)?,
                version_name: row.get(9)?,
                version_code: row.get(10)?,
                sha256: row.get(11)?,
                size_bytes: row.get::<_, i64>(12)? as u64,
                signer_sha256: row.get(13)?,
                icon_png_base64: row.get(14)?,
                metadata_status: row.get(15)?,
                metadata_error: row.get(16)?,
                created_at: row.get(17)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }
    pub fn add_app_library(&self, item: &crate::types::AppLibraryItem) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.add_app_library_if_new(item)?,
            "app-library artifact with SHA-256 {} already exists",
            item.sha256
        );
        Ok(())
    }

    /// Atomically claim one content hash and insert its library row.
    ///
    /// The immediate transaction serializes the lookup with the insert even for formats such
    /// as legacy IPA which are intentionally outside the verified-row partial unique index.
    pub fn add_app_library_if_new(
        &self,
        item: &crate::types::AppLibraryItem,
    ) -> anyhow::Result<bool> {
        let mut conn = self.conn()?;
        let transaction = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        if !item.sha256.is_empty()
            && transaction.query_row(
                "SELECT EXISTS(SELECT 1 FROM apps_library WHERE sha256=?1)",
                [&item.sha256],
                |row| row.get::<_, bool>(0),
            )?
        {
            transaction.commit()?;
            return Ok(false);
        }
        transaction.execute(
            "INSERT INTO apps_library (id, name, path, bundle_id, version, platform, package_format, artifact_kind, application_id, version_name, version_code, sha256, size_bytes, signer_sha256, icon_png_base64, metadata_status, metadata_error, created_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18)",
            params![
                item.id,
                item.name,
                item.path,
                item.bundle_id,
                item.version,
                match item.platform { crate::types::AppLibraryPlatform::Ios => "ios", crate::types::AppLibraryPlatform::Android => "android" },
                match item.package_format { crate::types::AppPackageFormat::Ipa => "ipa", crate::types::AppPackageFormat::Apk => "apk", crate::types::AppPackageFormat::Xapk => "xapk", crate::types::AppPackageFormat::Apkm => "apkm", crate::types::AppPackageFormat::Apks => "apks" },
                match item.artifact_kind { crate::types::AppPackageFormat::Ipa => "ipa", crate::types::AppPackageFormat::Apk => "apk", crate::types::AppPackageFormat::Xapk => "xapk", crate::types::AppPackageFormat::Apkm => "apkm", crate::types::AppPackageFormat::Apks => "apks" },
                item.application_id, item.version_name, item.version_code, item.sha256, item.size_bytes as i64, item.signer_sha256,
                item.icon_png_base64, item.metadata_status, item.metadata_error, item.created_at
            ],
        )?;
        transaction.commit()?;
        Ok(true)
    }
    pub fn find_app_library_by_sha256(
        &self,
        sha256: &str,
    ) -> anyhow::Result<Option<crate::types::AppLibraryItem>> {
        Ok(self
            .list_apps_library()?
            .into_iter()
            .find(|item| item.sha256 == sha256 && !sha256.is_empty()))
    }

    /// Fill content identity for rows created before migration 19, one file at a time.
    ///
    /// Historical duplicates deliberately remain separate rows. A unique index here would turn
    /// a metadata migration into a destructive library cleanup and make the surviving display
    /// name depend on iteration order.
    pub fn backfill_app_library_hashes(&self) -> anyhow::Result<(usize, Vec<String>)> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT id, path FROM apps_library WHERE sha256 = '' ORDER BY created_at, id",
        )?;
        let pending = stmt
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        drop(stmt);

        let mut updated = 0_usize;
        let mut failures = Vec::new();
        for (id, path) in pending {
            match hash_library_file(Path::new(&path)) {
                Ok((sha256, size_bytes)) => {
                    let size_bytes = i64::try_from(size_bytes)
                        .context("app-library artifact is too large for SQLite")?;
                    conn.execute(
                        "UPDATE apps_library
                         SET sha256=?2, size_bytes=?3, metadata_status='legacy', metadata_error=NULL
                         WHERE id=?1 AND sha256=''",
                        params![id, sha256, size_bytes],
                    )?;
                    updated += 1;
                }
                Err(error) => {
                    let detail = format!("{path}: {error:#}");
                    conn.execute(
                        "UPDATE apps_library
                         SET metadata_status='error', metadata_error=?2
                         WHERE id=?1 AND sha256=''",
                        params![id, detail],
                    )?;
                    failures.push(detail);
                }
            }
        }
        Ok((updated, failures))
    }
    pub fn delete_app_library(&self, id: &str) -> anyhow::Result<()> {
        let conn = self.conn()?;
        conn.execute("DELETE FROM apps_library WHERE id = ?1", params![id])?;
        Ok(())
    }
    pub fn list_schedules(&self) -> anyhow::Result<Vec<crate::types::ScheduleItem>> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT id, name, script_name, udids_json, every_minutes, enabled, last_run_at, next_run_at, last_error FROM schedules ORDER BY name",
        )?;
        let rows = stmt.query_map([], |row| {
            let udids_json: String = row.get(3)?;
            Ok(crate::types::ScheduleItem {
                id: row.get(0)?,
                name: row.get(1)?,
                script_name: row.get(2)?,
                // **An unreadable target list is not an empty one.** `unwrap_or_default()`
                // turned malformed `udids_json` — an older build, a hand-edited recovery, disk
                // corruption — into `vec![]`, leaving the schedule enabled and otherwise valid
                // while targeting no devices. Nothing downstream could tell that apart from a
                // schedule the operator had deliberately emptied.
                //
                // Found by an independent review on 27/08/2026.
                udids: serde_json::from_str(&udids_json).map_err(|error| {
                    rusqlite::Error::FromSqlConversionFailure(
                        3,
                        rusqlite::types::Type::Text,
                        Box::new(error),
                    )
                })?,
                every_minutes: narrow(row.get::<_, i64>(4)?, "every_minutes")?,
                enabled: row.get::<_, i64>(5)? != 0,
                last_run_at: row.get(6)?,
                next_run_at: row.get(7)?,
                last_error: row.get(8)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }
    pub fn upsert_schedule(&self, s: &crate::types::ScheduleItem) -> anyhow::Result<()> {
        let conn = self.conn()?;
        conn.execute(
            r#"INSERT INTO schedules (id, name, script_name, udids_json, every_minutes, enabled, last_run_at, next_run_at, last_error)
               VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)
               ON CONFLICT(id) DO UPDATE SET
                 name=excluded.name, script_name=excluded.script_name, udids_json=excluded.udids_json,
                 every_minutes=excluded.every_minutes, enabled=excluded.enabled,
                 last_run_at=excluded.last_run_at, next_run_at=excluded.next_run_at,
                 last_error=excluded.last_error"#,
            params![
                s.id,
                s.name,
                s.script_name,
                serde_json::to_string(&s.udids)?,
                s.every_minutes as i64,
                if s.enabled { 1 } else { 0 },
                s.last_run_at,
                s.next_run_at,
                s.last_error
            ],
        )?;
        Ok(())
    }
    pub fn delete_schedule(&self, id: &str) -> anyhow::Result<()> {
        let conn = self.conn()?;
        conn.execute("DELETE FROM schedules WHERE id = ?1", params![id])?;
        Ok(())
    }
}

fn hash_library_file(path: &Path) -> anyhow::Result<(String, u64)> {
    let mut file = std::fs::File::open(path)
        .with_context(|| format!("open legacy app-library file {}", path.display()))?;
    let mut digest = Sha256::new();
    let mut size = 0_u64;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .with_context(|| format!("read legacy app-library file {}", path.display()))?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
        size = size
            .checked_add(read as u64)
            .context("app-library artifact size overflow")?;
    }
    Ok((format!("{:x}", digest.finalize()), size))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app_fixture(id: &str, sha256: &str, version: &str) -> crate::types::AppLibraryItem {
        crate::types::AppLibraryItem {
            id: id.into(),
            name: id.into(),
            path: format!("C:/fixtures/{id}.apk"),
            bundle_id: "com.example.app".into(),
            version: version.into(),
            platform: crate::types::AppLibraryPlatform::Android,
            package_format: crate::types::AppPackageFormat::Apk,
            artifact_kind: crate::types::AppPackageFormat::Apk,
            application_id: "com.example.app".into(),
            version_name: version.into(),
            version_code: Some(version.into()),
            sha256: sha256.into(),
            size_bytes: 123,
            signer_sha256: "fixture-signer".into(),
            icon_png_base64: None,
            metadata_status: "verified".into(),
            metadata_error: None,
            created_at: "2026-09-02T00:00:00Z".into(),
        }
    }

    #[test]
    fn app_library_claim_is_atomic_and_keeps_distinct_versions() {
        let root =
            std::env::temp_dir().join(format!("riviu-app-library-atomic-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).expect("create fixture root");
        let db_path = root.join("riviu.db");
        Database::open(&db_path).expect("migrate fixture database");
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
        let handles = ["writer-a", "writer-b"].map(|id| {
            let db_path = db_path.clone();
            let barrier = std::sync::Arc::clone(&barrier);
            std::thread::spawn(move || {
                let database = Database::open(db_path).expect("open writer database");
                barrier.wait();
                database
                    .add_app_library_if_new(&app_fixture(id, "same-content", "1"))
                    .expect("claim app content")
            })
        });
        let inserted = handles
            .into_iter()
            .map(|handle| handle.join().expect("writer thread"))
            .collect::<Vec<_>>();
        assert_eq!(inserted.iter().filter(|inserted| **inserted).count(), 1);

        let database = Database::open(&db_path).expect("reopen fixture database");
        assert!(database
            .add_app_library_if_new(&app_fixture("version-two", "other-content", "2"))
            .expect("insert second version"));
        let rows = database.list_apps_library().expect("list library rows");
        assert_eq!(rows.len(), 2);
        assert_eq!(
            rows.iter()
                .map(|item| item.version_name.as_str())
                .collect::<std::collections::BTreeSet<_>>(),
            std::collections::BTreeSet::from(["1", "2"])
        );
        std::fs::remove_dir_all(root).expect("remove fixture root");
    }

    #[test]
    fn legacy_hash_backfill_is_sequential_and_keeps_duplicate_rows() {
        let root = std::env::temp_dir().join(format!(
            "riviu-app-library-backfill-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&root).expect("create fixture root");
        let first = root.join("first.ipa");
        let duplicate = root.join("duplicate.ipa");
        std::fs::write(&first, b"same historical bytes").expect("write first fixture");
        std::fs::write(&duplicate, b"same historical bytes").expect("write duplicate fixture");
        let database = Database::open(root.join("riviu.db")).expect("open fixture database");
        let connection = database.conn().expect("fixture connection");
        for (id, name, path) in [
            ("legacy-a", "First", &first),
            ("legacy-b", "Duplicate", &duplicate),
        ] {
            connection
                .execute(
                    "INSERT INTO apps_library (id,name,path,bundle_id,version,created_at)
                     VALUES (?1,?2,?3,'com.example.legacy','1','2026-01-01T00:00:00Z')",
                    params![id, name, path.display().to_string()],
                )
                .expect("insert legacy row");
        }
        drop(connection);

        let (updated, failures) = database
            .backfill_app_library_hashes()
            .expect("backfill legacy rows");
        assert_eq!(updated, 2);
        assert!(failures.is_empty());
        let rows = database.list_apps_library().expect("read back library");
        assert_eq!(rows.len(), 2, "duplicate history must not be collapsed");
        assert_eq!(rows[0].sha256, rows[1].sha256);
        assert_eq!(rows[0].size_bytes, 21);
        assert_eq!(rows[1].size_bytes, 21);

        std::fs::remove_dir_all(root).expect("remove fixture root");
    }
}
