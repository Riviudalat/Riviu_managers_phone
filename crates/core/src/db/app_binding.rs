//! Operator-selected application package for a device and social application.
//!
//! A device can carry more than one regional build of the same social app. The
//! binding is deliberately persisted outside the driver: the driver proves what
//! is installed, while this row records the operator's choice with CAS semantics.

use super::*;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DeviceAppBinding {
    pub udid: String,
    pub app_key: String,
    pub package: String,
    pub revision: i64,
    pub updated_at: String,
}

#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq)]
#[error("DeviceAppBindingConflict: expected revision {expected}, actual revision {actual}")]
pub struct DeviceAppBindingConflict {
    pub expected: i64,
    pub actual: i64,
}

fn validate_key(value: &str, name: &str, max: usize) -> anyhow::Result<()> {
    anyhow::ensure!(
        !value.is_empty()
            && value.len() <= max
            && value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-')),
        "Invalid {name}"
    );
    Ok(())
}

fn validate_udid(udid: &str) -> anyhow::Result<()> {
    anyhow::ensure!(
        !udid.trim().is_empty() && udid.len() <= 256 && !udid.chars().any(char::is_control),
        "Invalid device id"
    );
    Ok(())
}

fn read_binding(
    connection: &Connection,
    udid: &str,
    app_key: &str,
) -> anyhow::Result<Option<DeviceAppBinding>> {
    Ok(connection
        .query_row(
            "SELECT udid,app_key,package,revision,updated_at
             FROM device_app_bindings WHERE udid=?1 AND app_key=?2",
            params![udid, app_key],
            |row| {
                Ok(DeviceAppBinding {
                    udid: row.get(0)?,
                    app_key: row.get(1)?,
                    package: row.get(2)?,
                    revision: row.get(3)?,
                    updated_at: row.get(4)?,
                })
            },
        )
        .optional()?)
}

impl Database {
    pub fn device_app_binding(
        &self,
        udid: &str,
        app_key: &str,
    ) -> anyhow::Result<Option<DeviceAppBinding>> {
        validate_udid(udid)?;
        validate_key(app_key, "app key", 64)?;
        read_binding(&self.conn()?, udid, app_key)
    }

    /// Compare-and-swap the package choice. Repeating an already-applied choice is
    /// idempotent even when the caller lost the first response; changing it still
    /// requires the current revision.
    pub fn select_device_app(
        &self,
        udid: &str,
        app_key: &str,
        package: &str,
        expected_revision: i64,
    ) -> anyhow::Result<DeviceAppBinding> {
        validate_udid(udid)?;
        validate_key(app_key, "app key", 64)?;
        validate_key(package, "application package", 255)?;
        anyhow::ensure!(expected_revision >= 0, "Invalid expected revision");
        let mut connection = self.conn()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let current = read_binding(&transaction, udid, app_key)?;
        if let Some(current) = current.as_ref() {
            if current.package == package {
                transaction.commit()?;
                return Ok(current.clone());
            }
        }
        let actual = current.as_ref().map_or(0, |binding| binding.revision);
        if actual != expected_revision {
            return Err(DeviceAppBindingConflict {
                expected: expected_revision,
                actual,
            }
            .into());
        }
        let revision = actual
            .checked_add(1)
            .context("device app revision overflow")?;
        let updated_at = Utc::now().to_rfc3339();
        transaction.execute(
            "INSERT INTO device_app_bindings(udid,app_key,package,revision,updated_at)
             VALUES(?1,?2,?3,?4,?5)
             ON CONFLICT(udid,app_key) DO UPDATE SET
               package=excluded.package,revision=excluded.revision,updated_at=excluded.updated_at",
            params![udid, app_key, package, revision, updated_at],
        )?;
        let selected = read_binding(&transaction, udid, app_key)?
            .context("selected device app binding missing")?;
        transaction.commit()?;
        Ok(selected)
    }

    /// Changing the selected app while an exact-package cleanup is pending can
    /// make the handoff close the new app and strand the old one. This guard is
    /// stricter than ordinary completion admission only for the selection path.
    pub fn device_app_selection_block_reason(&self, udid: &str) -> anyhow::Result<Option<String>> {
        if let Some(reason) = self.app_completion_block_reason(udid)? {
            return Ok(Some(reason));
        }
        let pending: bool = self.conn()?.query_row(
            "SELECT EXISTS(SELECT 1 FROM app_completion_queue WHERE udid=?1 AND state='pending')",
            [udid],
            |row| row.get(0),
        )?;
        Ok(pending.then(|| "Chờ đóng ứng dụng của tác vụ cũ trên thiết bị hoàn tất".into()))
    }

    pub fn pending_app_completions_for_device(
        &self,
        udid: &str,
    ) -> anyhow::Result<Vec<super::AppCompletionRecord>> {
        validate_udid(udid)?;
        let connection = self.conn()?;
        let mut statement = connection.prepare(
            "SELECT udid,bundle_id,revision,attempts FROM app_completion_queue
             WHERE udid=?1 AND state='pending' ORDER BY requested_at_ms,bundle_id",
        )?;
        let records = statement
            .query_map([udid], |row| {
                Ok(super::AppCompletionRecord {
                    udid: row.get(0)?,
                    bundle_id: row.get(1)?,
                    revision: row.get(2)?,
                    attempts: row.get::<_, i64>(3)?.clamp(0, u32::MAX as i64) as u32,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(records)
    }

    pub fn pending_app_completion_packages(&self, udid: &str) -> anyhow::Result<Vec<String>> {
        Ok(self
            .pending_app_completions_for_device(udid)?
            .into_iter()
            .map(|record| record.bundle_id)
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_binding_is_cas_protected_and_same_choice_is_idempotent() {
        let path = std::env::temp_dir().join(format!("app-binding-{}.db", Uuid::new_v4()));
        let database = Database::open(&path).unwrap();
        assert_eq!(
            database.device_app_binding("phone", "tiktok").unwrap(),
            None
        );
        let first = database
            .select_device_app("phone", "tiktok", "com.zhiliaoapp.musically", 0)
            .unwrap();
        assert_eq!(first.revision, 1);
        let replay = database
            .select_device_app("phone", "tiktok", "com.zhiliaoapp.musically", 0)
            .unwrap();
        assert_eq!(replay, first);
        let error = database
            .select_device_app("phone", "tiktok", "com.ss.android.ugc.trill", 0)
            .unwrap_err();
        assert!(error.downcast_ref::<DeviceAppBindingConflict>().is_some());
        let changed = database
            .select_device_app("phone", "tiktok", "com.ss.android.ugc.trill", 1)
            .unwrap();
        assert_eq!(changed.revision, 2);
        assert!(database
            .device_app_selection_block_reason("phone")
            .unwrap()
            .is_none());
        database
            .request_app_completion("phone", "com.ss.android.ugc.trill")
            .unwrap();
        assert!(database
            .device_app_selection_block_reason("phone")
            .unwrap()
            .is_some());
        assert_eq!(
            database.pending_app_completion_packages("phone").unwrap(),
            ["com.ss.android.ugc.trill"]
        );
        let pending = database
            .pending_app_completions_for_device("phone")
            .unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].revision, 1);
        assert!(database
            .finish_app_completion(
                &pending[0],
                r#"{"bundleId":"com.ss.android.ugc.trill","oldPid":12}"#,
            )
            .unwrap());
        assert!(database
            .device_app_selection_block_reason("phone")
            .unwrap()
            .is_none());
        drop(database);
        let reopened = Database::open(&path).unwrap();
        assert_eq!(
            reopened
                .device_app_binding("phone", "tiktok")
                .unwrap()
                .unwrap(),
            changed
        );
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn migration_47_adds_binding_table_without_changing_existing_rows() {
        let path =
            std::env::temp_dir().join(format!("app-binding-migration-{}.db", Uuid::new_v4()));
        {
            let database = Database::open(&path).unwrap();
            database
                .set_setting("fixture.before.binding", "kept")
                .unwrap();
        }
        let connection = Connection::open(&path).unwrap();
        connection
            .execute("DELETE FROM schema_migrations WHERE version=47", [])
            .unwrap();
        connection
            .execute("DROP TABLE device_app_bindings", [])
            .unwrap();
        drop(connection);
        let migrated = Database::open(&path).unwrap();
        let backup = path.with_extension("pre-device-app-v46.db");
        assert!(backup.is_file());
        let backup_connection = Connection::open(&backup).unwrap();
        assert_eq!(
            backup_connection
                .query_row(
                    "SELECT value FROM settings WHERE key='fixture.before.binding'",
                    [],
                    |row| row.get::<_, String>(0),
                )
                .unwrap(),
            "kept"
        );
        assert!(backup_connection
            .query_row(
                "SELECT 1 FROM sqlite_master WHERE type='table' AND name='device_app_bindings'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .optional()
            .unwrap()
            .is_none());
        assert_eq!(migrated.schema_version().unwrap(), 47);
        assert_eq!(
            migrated
                .get_setting("fixture.before.binding")
                .unwrap()
                .as_deref(),
            Some("kept")
        );
        assert_eq!(
            migrated.device_app_binding("phone", "tiktok").unwrap(),
            None
        );
        drop(backup_connection);
        drop(migrated);
        let _ = std::fs::remove_file(path);
        let _ = std::fs::remove_file(backup);
    }

    #[test]
    fn direct_upgrade_from_v43_backs_up_the_actual_source_schema_before_migrating() {
        let path = std::env::temp_dir().join(format!(
            "app-binding-direct-v43-migration-{}.db",
            Uuid::new_v4()
        ));
        let mut connection = Connection::open(&path).unwrap();
        super::super::migrations::initialize_through(&mut connection, 43).unwrap();
        connection
            .execute(
                "INSERT INTO settings(key,value) VALUES('fixture.direct.v43','kept')",
                [],
            )
            .unwrap();
        drop(connection);

        let migrated = Database::open(&path).unwrap();
        let backup = path.with_extension("pre-device-app-v43.db");
        assert!(backup.is_file());
        let backup_connection = Connection::open(&backup).unwrap();
        assert_eq!(
            backup_connection
                .query_row("SELECT MAX(version) FROM schema_migrations", [], |row| {
                    row.get::<_, i64>(0)
                })
                .unwrap(),
            43
        );
        assert_eq!(
            backup_connection
                .query_row(
                    "SELECT value FROM settings WHERE key='fixture.direct.v43'",
                    [],
                    |row| row.get::<_, String>(0),
                )
                .unwrap(),
            "kept"
        );
        assert!(backup_connection
            .query_row(
                "SELECT 1 FROM sqlite_master WHERE type='table' AND name='device_app_bindings'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .optional()
            .unwrap()
            .is_none());
        assert_eq!(migrated.schema_version().unwrap(), 47);
        assert_eq!(
            migrated
                .get_setting("fixture.direct.v43")
                .unwrap()
                .as_deref(),
            Some("kept")
        );
        drop(backup_connection);
        drop(migrated);
        let _ = std::fs::remove_file(path);
        let _ = std::fs::remove_file(backup);
    }

    #[test]
    fn exact_legacy_v1_is_backed_up_before_the_migration_ledger_is_bootstrapped() {
        let path = std::env::temp_dir().join(format!(
            "app-binding-direct-legacy-v1-migration-{}.db",
            Uuid::new_v4()
        ));
        let connection = Connection::open(&path).unwrap();
        super::super::migrations::initialize_exact_legacy_v1(&connection).unwrap();
        connection
            .execute(
                "INSERT INTO settings(key,value) VALUES('fixture.legacy.v1','kept')",
                [],
            )
            .unwrap();
        drop(connection);

        let migrated = Database::open(&path).unwrap();
        let backup = path.with_extension("pre-device-app-v1.db");
        assert!(backup.is_file());
        let backup_connection = Connection::open(&backup).unwrap();
        assert!(backup_connection
            .query_row(
                "SELECT 1 FROM sqlite_master WHERE type='table' AND name='schema_migrations'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .optional()
            .unwrap()
            .is_none());
        assert_eq!(
            backup_connection
                .query_row(
                    "SELECT value FROM settings WHERE key='fixture.legacy.v1'",
                    [],
                    |row| row.get::<_, String>(0),
                )
                .unwrap(),
            "kept"
        );
        assert_eq!(migrated.schema_version().unwrap(), 47);
        assert_eq!(
            migrated
                .get_setting("fixture.legacy.v1")
                .unwrap()
                .as_deref(),
            Some("kept")
        );
        drop(backup_connection);
        drop(migrated);
        let _ = std::fs::remove_file(path);
        let _ = std::fs::remove_file(backup);
    }

    #[test]
    fn stale_same_version_backup_is_replaced_before_direct_upgrade() {
        let path =
            std::env::temp_dir().join(format!("app-binding-stale-backup-{}.db", Uuid::new_v4()));
        let mut connection = Connection::open(&path).unwrap();
        super::super::migrations::initialize_through(&mut connection, 43).unwrap();
        let backup = path.with_extension("pre-device-app-v43.db");
        connection
            .execute("VACUUM INTO ?1", [backup.to_string_lossy().as_ref()])
            .unwrap();
        connection
            .execute(
                "INSERT INTO settings(key,value) VALUES('fixture.after.stale.backup','must survive')",
                [],
            )
            .unwrap();
        drop(connection);

        let migrated = Database::open(&path).unwrap();
        let backup_connection = Connection::open(&backup).unwrap();
        assert_eq!(
            backup_connection
                .query_row(
                    "SELECT value FROM settings WHERE key='fixture.after.stale.backup'",
                    [],
                    |row| row.get::<_, String>(0),
                )
                .unwrap(),
            "must survive"
        );
        assert_eq!(migrated.schema_version().unwrap(), 47);
        drop(backup_connection);
        drop(migrated);
        let _ = std::fs::remove_file(path);
        let _ = std::fs::remove_file(backup);
    }

    #[test]
    fn concurrent_database_openers_serialize_backup_and_direct_migration() {
        let path = std::env::temp_dir().join(format!(
            "app-binding-concurrent-direct-migration-{}.db",
            Uuid::new_v4()
        ));
        let mut connection = Connection::open(&path).unwrap();
        super::super::migrations::initialize_through(&mut connection, 43).unwrap();
        drop(connection);
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(3));
        let mut threads = Vec::new();
        for _ in 0..2 {
            let path = path.clone();
            let barrier = barrier.clone();
            threads.push(std::thread::spawn(move || {
                barrier.wait();
                Database::open(path).map(|database| database.schema_version().unwrap())
            }));
        }
        barrier.wait();
        for thread in threads {
            assert_eq!(thread.join().unwrap().unwrap(), 47);
        }
        let backup = path.with_extension("pre-device-app-v43.db");
        assert_eq!(
            Connection::open(&backup)
                .unwrap()
                .query_row("SELECT MAX(version) FROM schema_migrations", [], |row| {
                    row.get::<_, i64>(0)
                })
                .unwrap(),
            43
        );
        let _ = std::fs::remove_file(path);
        let _ = std::fs::remove_file(backup);
    }
}
