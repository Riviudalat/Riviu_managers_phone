//! Credential-backed Google provider selection; publication identity never changes.
use super::*;
use crate::google_oauth::{GoogleOAuthClientConfig, GoogleOAuthTokens};
use crate::publish_sheet::SheetDeliveryTarget;
use serde::{Deserialize, Serialize};
pub const GOOGLE_CONNECTION_SETTING: &str = "google.sheets.connection.v1";
pub const GOOGLE_MIGRATION_SETTING: &str = "google.sheets.migration.v1";
pub const SHEET_PROVIDER_SETTING: &str = "publish.sheet.provider";
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GoogleSheetConnection {
    pub target: SheetDeliveryTarget,
    pub account_id: String,
    pub writer_id: String,
    pub spreadsheet_name: String,
}
impl Database {
    pub fn google_oauth_config(&self) -> anyhow::Result<Option<GoogleOAuthClientConfig>> {
        let Some(store) = &self.secrets else {
            return Ok(None);
        };
        store
            .get_secret("google-oauth-client-v1")?
            .filter(|s| !s.is_empty())
            .map(|s| serde_json::from_str(&s).context("Cấu hình Google đã lưu không hợp lệ"))
            .transpose()
    }
    pub fn set_google_oauth_config(&self, config: &GoogleOAuthClientConfig) -> anyhow::Result<()> {
        config.validate()?;
        let store = self
            .secrets
            .as_ref()
            .context("Kho thông tin xác thực chưa sẵn sàng")?;
        let value = serde_json::to_string(config)?;
        store.set_secret("google-oauth-client-v1", &value)?;
        anyhow::ensure!(
            store.get_secret("google-oauth-client-v1")?.as_deref() == Some(&value),
            "Chưa đọc lại được cấu hình Google"
        );
        Ok(())
    }
    pub fn google_oauth_tokens(&self) -> anyhow::Result<Option<GoogleOAuthTokens>> {
        let Some(store) = &self.secrets else {
            return Ok(None);
        };
        store
            .get_secret("google-oauth-tokens-v1")?
            .filter(|s| !s.is_empty())
            .map(|s| serde_json::from_str(&s).context("Phiên đăng nhập Google đã lưu không hợp lệ"))
            .transpose()
    }
    pub fn set_google_oauth_tokens(
        &self,
        tokens: Option<&GoogleOAuthTokens>,
    ) -> anyhow::Result<()> {
        let store = self
            .secrets
            .as_ref()
            .context("Kho thông tin xác thực chưa sẵn sàng")?;
        let previous = self.google_oauth_tokens()?;
        let value = tokens
            .map(serde_json::to_string)
            .transpose()?
            .unwrap_or_default();
        store.set_secret("google-oauth-tokens-v1", &value)?;
        anyhow::ensure!(
            store
                .get_secret("google-oauth-tokens-v1")?
                .unwrap_or_default()
                == value,
            "Chưa đọc lại được phiên đăng nhập Google"
        );
        if tokens.is_some_and(|tokens| {
            previous.as_ref().is_none_or(|prior| {
                prior.refresh_token != tokens.refresh_token || prior.account_id != tokens.account_id
            })
        }) {
            self.set_setting(
                "google.sheets.authorization-generation",
                &Uuid::new_v4().to_string(),
            )?;
        }
        Ok(())
    }
    pub fn google_sheet_connection(&self) -> anyhow::Result<Option<GoogleSheetConnection>> {
        self.get_setting(GOOGLE_CONNECTION_SETTING)?
            .filter(|s| !s.is_empty())
            .map(|s| serde_json::from_str(&s).context("Đích Google Sheets đã lưu không hợp lệ"))
            .transpose()
    }
    pub fn google_connection_for_target(
        &self,
        target: &SheetDeliveryTarget,
    ) -> anyhow::Result<Option<GoogleSheetConnection>> {
        if let Some(current) = self.google_sheet_connection()? {
            if current.target == *target {
                return Ok(Some(current));
            }
        }
        let map = self
            .get_setting("google.sheets.connections.v1")?
            .map(|s| serde_json::from_str::<Vec<GoogleSheetConnection>>(&s))
            .transpose()?
            .unwrap_or_default();
        Ok(map.into_iter().find(|c| c.target == *target))
    }
    pub fn sheet_uses_google_direct(&self) -> anyhow::Result<bool> {
        Ok(self.get_setting(SHEET_PROVIDER_SETTING)?.as_deref() == Some("googleDirect"))
    }
    pub fn google_writer_id(&self) -> anyhow::Result<String> {
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let id: Option<String> = tx
            .query_row(
                "SELECT value FROM settings WHERE key='google.sheets.writer-id'",
                [],
                |r| r.get(0),
            )
            .optional()?;
        let id = id.unwrap_or_else(|| Uuid::new_v4().to_string());
        Uuid::parse_str(&id)?;
        tx.execute(
            "INSERT OR IGNORE INTO settings(key,value) VALUES('google.sheets.writer-id',?1)",
            [&id],
        )?;
        tx.commit()?;
        Ok(id)
    }
    pub fn begin_google_sheet_migration(
        &self,
        target: &SheetDeliveryTarget,
        request_id: &str,
    ) -> anyhow::Result<()> {
        target.validate()?;
        Uuid::parse_str(request_id)?;
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let resetting: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM publish_reporting_epochs WHERE paused=1)",
            [],
            |r| r.get(0),
        )?;
        anyhow::ensure!(
            !resetting,
            "Hoàn tất đợt dọn bảng trước khi chuyển kết nối Google"
        );
        let existing: Option<String> = tx
            .query_row(
                "SELECT value FROM settings WHERE key=?1",
                [GOOGLE_MIGRATION_SETTING],
                |r| r.get(0),
            )
            .optional()?;
        let value = serde_json::json!({"requestId":request_id,"target":target});
        if let Some(existing) = existing.filter(|s| !s.is_empty()) {
            anyhow::ensure!(
                serde_json::from_str::<serde_json::Value>(&existing)? == value,
                "Đang chuyển kết nối một bảng khác; tiếp tục lần chuyển đang chờ"
            );
        }
        tx.execute("INSERT INTO settings(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",params![GOOGLE_MIGRATION_SETTING,value.to_string()])?;
        tx.commit()?;
        Ok(())
    }
    pub fn finish_google_sheet_authorization(
        &self,
        connection: &GoogleSheetConnection,
        request_id: &str,
        tokens: &GoogleOAuthTokens,
    ) -> anyhow::Result<()> {
        anyhow::ensure!(
            connection.account_id == tokens.account_id,
            "Tài khoản không khớp đích Google đã kiểm tra"
        );
        let previous = self.google_oauth_tokens()?;
        let generation = self.get_setting("google.sheets.authorization-generation")?;
        let result = (|| {
            self.set_google_oauth_tokens(Some(tokens))?;
            self.set_setting(
                "google.sheets.authorization-generation",
                &Uuid::new_v4().to_string(),
            )?;
            self.finish_google_sheet_migration(connection, request_id)
        })();
        if let Err(error) = result {
            // SecretStore and SQLite cannot share a transaction. Compensate any
            // failed local commit; never strand the old connection on new secrets.
            let store = self
                .secrets
                .as_ref()
                .context("Kho thông tin xác thực chưa sẵn sàng")?;
            let raw = previous
                .as_ref()
                .map(serde_json::to_string)
                .transpose()?
                .unwrap_or_default();
            store
                .set_secret("google-oauth-tokens-v1", &raw)
                .context("Không khôi phục được phiên Google trước đó; cần đăng nhập lại")?;
            anyhow::ensure!(
                store
                    .get_secret("google-oauth-tokens-v1")?
                    .unwrap_or_default()
                    == raw,
                "Không đọc lại được phiên Google đã khôi phục"
            );
            if let Some(generation) = generation {
                self.set_setting("google.sheets.authorization-generation", &generation)?;
            } else {
                self.conn()?.execute(
                    "DELETE FROM settings WHERE key='google.sheets.authorization-generation'",
                    [],
                )?;
            }
            return Err(error);
        }
        Ok(())
    }

    /// Only abandon a pre-effect rejection; uncertain remote work keeps its barrier.
    pub fn abort_google_sheet_migration(&self, request_id: &str) -> anyhow::Result<()> {
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let raw: String = tx.query_row(
            "SELECT value FROM settings WHERE key=?1",
            [GOOGLE_MIGRATION_SETTING],
            |r| r.get(0),
        )?;
        let pending: serde_json::Value = serde_json::from_str(&raw)?;
        anyhow::ensure!(
            pending["requestId"].as_str() == Some(request_id),
            "Lần chuyển kết nối đã thay đổi"
        );
        tx.execute(
            "UPDATE settings SET value='' WHERE key=?1",
            [GOOGLE_MIGRATION_SETTING],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn finish_google_sheet_migration(
        &self,
        connection: &GoogleSheetConnection,
        request_id: &str,
    ) -> anyhow::Result<()> {
        connection.target.validate()?;
        Uuid::parse_str(&connection.writer_id)?;
        anyhow::ensure!(
            !connection.account_id.is_empty(),
            "Thiếu tài khoản Google cho đích ghi"
        );
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let resetting: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM publish_reporting_epochs WHERE paused=1)",
            [],
            |r| r.get(0),
        )?;
        anyhow::ensure!(
            !resetting,
            "Đợt dọn bảng đang chờ; chưa chuyển kết nối Google"
        );
        let active:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM publish_sheet_sync_state WHERE claim_token IS NOT NULL AND claim_until_ms>?1)",[Utc::now().timestamp_millis()],|r|r.get(0))?;
        anyhow::ensure!(
            !active,
            "Còn yêu cầu Sheet đang thực hiện; chờ kết thúc để chuyển kết nối"
        );
        let raw: String = tx.query_row(
            "SELECT value FROM settings WHERE key=?1",
            [GOOGLE_MIGRATION_SETTING],
            |r| r.get(0),
        )?;
        let pending: serde_json::Value = serde_json::from_str(&raw)?;
        let pending_target: SheetDeliveryTarget =
            serde_json::from_value(pending["target"].clone())?;
        anyhow::ensure!(
            pending_target.internal_reporting == connection.target.internal_reporting
                && pending_target
                    .reporting_epoch
                    .as_ref()
                    .is_none_or(|epoch| Some(epoch) == connection.target.reporting_epoch.as_ref()),
            "Đợt báo cáo hoặc bố cục thay đổi khi chuyển kết nối"
        );
        anyhow::ensure!(
            pending["requestId"] == request_id
                && pending["target"]["spreadsheetId"] == connection.target.spreadsheet_id
                && pending["target"]["sheetGid"] == connection.target.sheet_gid,
            "Lần chuyển kết nối đã thay đổi"
        );
        let oldmap: Option<String> = tx
            .query_row(
                "SELECT value FROM settings WHERE key='google.sheets.connections.v1'",
                [],
                |r| r.get(0),
            )
            .optional()?;
        let mut map = oldmap
            .map(|s| serde_json::from_str::<Vec<GoogleSheetConnection>>(&s))
            .transpose()?
            .unwrap_or_default();
        // A new account only replaces this exact verified destination. Older
        // epochs and unrelated outboxes retain their original account binding.
        map.retain(|c| c.target != connection.target);
        map.push(connection.clone());
        let fields = [
            (
                GOOGLE_CONNECTION_SETTING,
                serde_json::to_string(connection)?,
            ),
            ("google.sheets.connections.v1", serde_json::to_string(&map)?),
            (SHEET_PROVIDER_SETTING, "googleDirect".to_owned()),
            (
                crate::publish_sheet::SHEET_URL_SETTING,
                format!(
                    "https://docs.google.com/spreadsheets/d/{}/edit#gid={}",
                    connection.target.spreadsheet_id, connection.target.sheet_gid
                ),
            ),
            (GOOGLE_MIGRATION_SETTING, String::new()),
        ];
        for (key, value) in fields {
            tx.execute("INSERT INTO settings(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",params![key,value])?;
        }
        tx.commit()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Arc;
    #[derive(Default)]
    struct Memory(parking_lot::Mutex<HashMap<String, String>>);
    impl SecretStore for Memory {
        fn get_secret(&self, key: &str) -> anyhow::Result<Option<String>> {
            Ok(self.0.lock().get(key).cloned())
        }
        fn set_secret(&self, key: &str, value: &str) -> anyhow::Result<()> {
            self.0.lock().insert(key.into(), value.into());
            Ok(())
        }
    }
    fn fixture_connection(db: &Database, book: &str, epoch: &str) -> GoogleSheetConnection {
        GoogleSheetConnection {
            target: SheetDeliveryTarget {
                version: 2,
                spreadsheet_id: book.into(),
                sheet_gid: 0,
                reporting_epoch: Some(epoch.into()),
                internal_reporting: true,
            },
            account_id: "fixture-account".into(),
            writer_id: db.google_writer_id().unwrap(),
            spreadsheet_name: book.into(),
        }
    }
    #[test]
    fn ordinary_access_refresh_preserves_generation_but_new_authorization_reopens_paused_delivery()
    {
        let path = std::env::temp_dir().join(format!("google-generation-{}.db", Uuid::new_v4()));
        let store = Arc::new(Memory::default());
        let db = Database::open(&path).unwrap().with_secrets(store.clone());
        let mut tokens = GoogleOAuthTokens {
            access_token: "access-original-secret".into(),
            refresh_token: "refresh-original-secret".into(),
            expires_at_ms: 10000,
            scope: crate::google_oauth::GOOGLE_SHEETS_SCOPES.into(),
            account_id: "account".into(),
            email: "fixture@example.com".into(),
        };
        db.set_google_oauth_tokens(Some(&tokens)).unwrap();
        let initial = db
            .get_setting("google.sheets.authorization-generation")
            .unwrap()
            .unwrap();
        tokens.access_token = "access-refreshed-secret".into();
        tokens.expires_at_ms = 999999;
        db.set_google_oauth_tokens(Some(&tokens)).unwrap();
        assert_eq!(
            db.get_setting("google.sheets.authorization-generation")
                .unwrap()
                .unwrap(),
            initial
        );
        db.conn().unwrap().execute("INSERT INTO publish_sheet_sync_state(assignment_id,connection_fingerprint,report_next_attempt_at_ms,report_last_error) VALUES('preserved-debt',?1,NULL,'invalid_grant')",[&initial]).unwrap();
        db.configure_bound_sheet_delivery(&initial, 100).unwrap();
        assert!(db.conn().unwrap().query_row("SELECT report_next_attempt_at_ms FROM publish_sheet_sync_state WHERE assignment_id='preserved-debt'",[],|r|r.get::<_,Option<i64>>(0)).unwrap().is_none());
        db.set_google_oauth_tokens(None).unwrap();
        db.set_google_oauth_tokens(Some(&tokens)).unwrap();
        let next = db
            .get_setting("google.sheets.authorization-generation")
            .unwrap()
            .unwrap();
        assert_ne!(initial, next);
        db.configure_bound_sheet_delivery(&next, 200).unwrap();
        assert_eq!(db.conn().unwrap().query_row("SELECT report_next_attempt_at_ms FROM publish_sheet_sync_state WHERE assignment_id='preserved-debt'",[],|r|r.get::<_,i64>(0)).unwrap(),200);
        drop(db);
        let db = Database::open(&path).unwrap().with_secrets(store);
        assert_eq!(
            db.get_setting("google.sheets.authorization-generation")
                .unwrap()
                .unwrap(),
            next
        );
        let bytes = std::fs::read(&path).unwrap();
        for secret in [
            "access-original-secret",
            "access-refreshed-secret",
            "refresh-original-secret",
        ] {
            assert!(!bytes
                .windows(secret.len())
                .any(|window| window == secret.as_bytes()));
        }
    }
    #[test]
    fn migration_cannot_start_or_finish_during_a_pending_reset_and_keeps_checkpoint() {
        let path =
            std::env::temp_dir().join(format!("google-reset-migration-{}.db", Uuid::new_v4()));
        let db = Database::open(&path).unwrap();
        let connection = fixture_connection(&db, "book", "legacy");
        let id = Uuid::new_v4().to_string();
        let reset = Uuid::new_v4().to_string();
        db.begin_publish_sheet_reset(&connection.target, &reset)
            .unwrap();
        assert!(db
            .begin_google_sheet_migration(&connection.target, &id)
            .is_err());
        assert!(db.get_setting(GOOGLE_MIGRATION_SETTING).unwrap().is_none());
        db.conn()
            .unwrap()
            .execute("DELETE FROM publish_reporting_epochs", [])
            .unwrap();
        db.begin_google_sheet_migration(&connection.target, &id)
            .unwrap();
        let checkpoint = db.get_setting(GOOGLE_MIGRATION_SETTING).unwrap().unwrap();
        db.begin_publish_sheet_reset(&connection.target, &reset)
            .unwrap();
        assert!(db.finish_google_sheet_migration(&connection, &id).is_err());
        assert_eq!(
            db.get_setting(GOOGLE_MIGRATION_SETTING).unwrap().unwrap(),
            checkpoint
        );
        assert!(!db.sheet_uses_google_direct().unwrap());
        assert!(db
            .claim_bound_sheet_delivery(
                crate::db::SheetDeliveryKind::Report,
                None,
                Utc::now().timestamp_millis()
            )
            .unwrap()
            .is_none());
    }
    #[test]
    fn migration_rejects_epoch_layout_drift_and_an_old_request_without_discarding_checkpoint() {
        let path =
            std::env::temp_dir().join(format!("google-migration-binding-{}.db", Uuid::new_v4()));
        let db = Database::open(&path).unwrap();
        let connection = fixture_connection(&db, "book", "legacy");
        let id = Uuid::new_v4().to_string();
        db.begin_google_sheet_migration(&connection.target, &id)
            .unwrap();
        let checkpoint = db.get_setting(GOOGLE_MIGRATION_SETTING).unwrap().unwrap();
        for changed in [
            GoogleSheetConnection {
                target: SheetDeliveryTarget {
                    reporting_epoch: Some("different-epoch".into()),
                    ..connection.target.clone()
                },
                ..connection.clone()
            },
            GoogleSheetConnection {
                target: SheetDeliveryTarget {
                    internal_reporting: false,
                    ..connection.target.clone()
                },
                ..connection.clone()
            },
        ] {
            assert!(db.finish_google_sheet_migration(&changed, &id).is_err());
            assert_eq!(
                db.get_setting(GOOGLE_MIGRATION_SETTING).unwrap().unwrap(),
                checkpoint
            );
        }
        assert!(db
            .finish_google_sheet_migration(&connection, &Uuid::new_v4().to_string())
            .is_err());
        drop(db);
        let db = Database::open(path).unwrap();
        db.begin_google_sheet_migration(&connection.target, &id)
            .unwrap();
        db.finish_google_sheet_migration(&connection, &id).unwrap();
        assert_eq!(db.google_sheet_connection().unwrap(), Some(connection));
    }
    #[test]
    fn multiple_target_mappings_preserve_original_epoch_and_never_fall_back_on_logout() {
        let path = std::env::temp_dir().join(format!("google-target-map-{}.db", Uuid::new_v4()));
        let db = Database::open(&path)
            .unwrap()
            .with_secrets(Arc::new(Memory::default()));
        let first = fixture_connection(&db, "first-book", "epoch-first");
        let second = fixture_connection(&db, "second-book", "epoch-second");
        for connection in [&first, &second] {
            let id = Uuid::new_v4().to_string();
            db.begin_google_sheet_migration(&connection.target, &id)
                .unwrap();
            db.finish_google_sheet_migration(connection, &id).unwrap();
        }
        assert_eq!(db.google_sheet_connection().unwrap(), Some(second.clone()));
        assert_eq!(
            db.google_connection_for_target(&first.target).unwrap(),
            Some(first.clone())
        );
        let wrong = SheetDeliveryTarget {
            reporting_epoch: Some("wrong-epoch".into()),
            ..first.target.clone()
        };
        assert!(db.google_connection_for_target(&wrong).unwrap().is_none());
        db.set_google_oauth_tokens(None).unwrap();
        assert!(db.sheet_uses_google_direct().unwrap());
        drop(db);
        let db = Database::open(path).unwrap();
        assert_eq!(
            db.google_connection_for_target(&first.target).unwrap(),
            Some(first)
        );
        assert_eq!(
            db.google_connection_for_target(&second.target).unwrap(),
            Some(second)
        );
    }
    #[test]
    fn reconnecting_same_tab_keeps_old_epoch_binding_for_its_unrelated_outbox() {
        let path = std::env::temp_dir().join(format!("google-epoch-map-{}.db", Uuid::new_v4()));
        let db = Database::open(path).unwrap();
        let first = fixture_connection(&db, "same-book", "original-epoch");
        let mut next = fixture_connection(&db, "same-book", "new-epoch");
        next.account_id = "other-account".into();
        for connection in [&first, &next] {
            let request = Uuid::new_v4().to_string();
            db.begin_google_sheet_migration(&connection.target, &request)
                .unwrap();
            db.finish_google_sheet_migration(connection, &request)
                .unwrap();
        }
        assert_eq!(
            db.google_connection_for_target(&first.target).unwrap(),
            Some(first)
        );
        assert_eq!(db.google_sheet_connection().unwrap(), Some(next));
    }

    #[test]
    fn failed_authorization_commit_restores_previous_credentials_and_generation() {
        let path = std::env::temp_dir().join(format!("google-auth-rollback-{}.db", Uuid::new_v4()));
        let db = Database::open(path)
            .unwrap()
            .with_secrets(Arc::new(Memory::default()));
        let prior = GoogleOAuthTokens {
            access_token: "old-access".into(),
            refresh_token: "old-refresh".into(),
            expires_at_ms: 123,
            scope: crate::google_oauth::GOOGLE_SHEETS_SCOPES.into(),
            account_id: "old-account".into(),
            email: "old@example.test".into(),
        };
        db.set_google_oauth_tokens(Some(&prior)).unwrap();
        let generation = db
            .get_setting("google.sheets.authorization-generation")
            .unwrap();
        let next = GoogleOAuthTokens {
            account_id: "new-account".into(),
            refresh_token: "new-refresh".into(),
            ..prior.clone()
        };
        let mut connection = fixture_connection(&db, "book", "epoch");
        connection.account_id = next.account_id.clone();
        // No matching pending migration: the DB commit must fail after credential staging.
        assert!(db
            .finish_google_sheet_authorization(&connection, &Uuid::new_v4().to_string(), &next)
            .is_err());
        let stored = db.google_oauth_tokens().unwrap().unwrap();
        assert_eq!(stored.account_id, "old-account");
        assert_eq!(stored.refresh_token, "old-refresh");
        assert_eq!(
            db.get_setting("google.sheets.authorization-generation")
                .unwrap(),
            generation
        );
        assert!(db.google_sheet_connection().unwrap().is_none());
    }

    #[test]
    fn verified_account_rebind_changes_only_exact_destination_not_old_outbox() {
        let path = std::env::temp_dir().join(format!("google-rebind-{}.db", Uuid::new_v4()));
        let db = Database::open(path)
            .unwrap()
            .with_secrets(Arc::new(Memory::default()));
        let first = fixture_connection(&db, "first-book", "original-epoch");
        let second = fixture_connection(&db, "second-book", "other-epoch");
        for connection in [&first, &second] {
            let request = Uuid::new_v4().to_string();
            db.begin_google_sheet_migration(&connection.target, &request)
                .unwrap();
            db.finish_google_sheet_migration(connection, &request)
                .unwrap();
        }
        db.conn().unwrap().execute("INSERT INTO publish_sheet_sync_state(assignment_id,connection_fingerprint,report_next_attempt_at_ms,report_last_error) VALUES('unchanged-debt','original-fingerprint',42,'unchanged-error')", []).unwrap();
        let tokens = GoogleOAuthTokens {
            access_token: "new-access".into(),
            refresh_token: "new-refresh".into(),
            expires_at_ms: 1000,
            scope: crate::google_oauth::GOOGLE_SHEETS_SCOPES.into(),
            account_id: "new-account".into(),
            email: "new@example.test".into(),
        };
        let rebound = GoogleSheetConnection {
            account_id: tokens.account_id.clone(),
            ..second.clone()
        };
        let request = Uuid::new_v4().to_string();
        db.begin_google_sheet_migration(&second.target, &request)
            .unwrap();
        db.finish_google_sheet_authorization(&rebound, &request, &tokens)
            .unwrap();
        assert_eq!(
            db.google_connection_for_target(&first.target).unwrap(),
            Some(first)
        );
        assert_eq!(
            db.google_connection_for_target(&second.target).unwrap(),
            Some(rebound)
        );
        let debt: (String, i64, String) = db.conn().unwrap().query_row("SELECT connection_fingerprint,report_next_attempt_at_ms,report_last_error FROM publish_sheet_sync_state WHERE assignment_id='unchanged-debt'", [], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).unwrap();
        assert_eq!(
            debt,
            ("original-fingerprint".into(), 42, "unchanged-error".into())
        );
    }

    #[test]
    fn abort_migration_removes_only_the_exact_pending_request() {
        let path =
            std::env::temp_dir().join(format!("google-migration-abort-{}.db", Uuid::new_v4()));
        let db = Database::open(path).unwrap();
        let connection = fixture_connection(&db, "book", "epoch");
        let request = Uuid::new_v4().to_string();
        db.begin_google_sheet_migration(&connection.target, &request)
            .unwrap();
        assert!(db
            .abort_google_sheet_migration(&Uuid::new_v4().to_string())
            .is_err());
        assert!(!db
            .get_setting(GOOGLE_MIGRATION_SETTING)
            .unwrap()
            .unwrap()
            .is_empty());
        db.abort_google_sheet_migration(&request).unwrap();
        assert!(db
            .get_setting(GOOGLE_MIGRATION_SETTING)
            .unwrap()
            .unwrap()
            .is_empty());
    }

    #[test]
    fn google_secrets_never_enter_sqlite_and_connection_migration_preserves_outbox_identity() {
        let path = std::env::temp_dir().join(format!("google-connection-{}.db", Uuid::new_v4()));
        let store = Arc::new(Memory::default());
        let db = Database::open(&path).unwrap().with_secrets(store.clone());
        let config = GoogleOAuthClientConfig {
            client_id: "123-fixture.apps.googleusercontent.com".into(),
            client_secret: Some("private-client-secret".into()),
            picker_api_key: Some("AIzaFixtureKey".into()),
            project_number: Some("123".into()),
        };
        db.set_google_oauth_config(&config).unwrap();
        let tokens = GoogleOAuthTokens {
            access_token: "private-access".into(),
            refresh_token: "private-refresh".into(),
            expires_at_ms: 9_000_000_000_000,
            scope: crate::google_oauth::GOOGLE_SHEETS_SCOPES.into(),
            account_id: "google-sub".into(),
            email: "fixture@example.test".into(),
        };
        db.set_google_oauth_tokens(Some(&tokens)).unwrap();
        assert_eq!(db.google_oauth_config().unwrap().unwrap(), config);
        assert_eq!(
            db.google_oauth_tokens().unwrap().unwrap().account_id,
            "google-sub"
        );
        let target = SheetDeliveryTarget {
            version: 2,
            spreadsheet_id: "fixture-book".into(),
            sheet_gid: 0,
            reporting_epoch: Some("legacy".into()),
            internal_reporting: true,
        };
        let request = Uuid::new_v4().to_string();
        db.begin_google_sheet_migration(&target, &request).unwrap();
        assert!(db
            .claim_bound_sheet_delivery(
                crate::db::SheetDeliveryKind::Report,
                None,
                Utc::now().timestamp_millis()
            )
            .unwrap()
            .is_none());
        assert!(db
            .begin_google_sheet_migration(&target, &Uuid::new_v4().to_string())
            .is_err());
        let connection = GoogleSheetConnection {
            target,
            account_id: "google-sub".into(),
            writer_id: db.google_writer_id().unwrap(),
            spreadsheet_name: "Fixture".into(),
        };
        db.finish_google_sheet_migration(&connection, &request)
            .unwrap();
        assert!(db.sheet_uses_google_direct().unwrap());
        assert!(db
            .get_setting(GOOGLE_MIGRATION_SETTING)
            .unwrap()
            .unwrap()
            .is_empty());
        db.set_google_oauth_tokens(None).unwrap();
        assert!(
            db.sheet_uses_google_direct().unwrap(),
            "logout must not reactivate legacy writer"
        );
        assert!(db.google_oauth_tokens().unwrap().is_none());
        drop(db);
        let db = Database::open(&path).unwrap().with_secrets(store);
        assert_eq!(db.google_sheet_connection().unwrap(), Some(connection));
        let bytes = std::fs::read(&path).unwrap();
        for secret in ["private-client-secret", "private-access", "private-refresh"] {
            assert!(!bytes.windows(secret.len()).any(|w| w == secret.as_bytes()));
        }
        drop(db);
        let _ = std::fs::remove_file(path);
    }
    #[test]
    fn credential_store_absence_does_not_fall_back_to_plaintext_settings() {
        let path = std::env::temp_dir().join(format!("google-no-secrets-{}.db", Uuid::new_v4()));
        let db = Database::open(&path).unwrap();
        assert!(db.google_oauth_tokens().unwrap().is_none());
        assert!(db.set_google_oauth_tokens(None).is_err());
        drop(db);
        let _ = std::fs::remove_file(path);
    }
}
