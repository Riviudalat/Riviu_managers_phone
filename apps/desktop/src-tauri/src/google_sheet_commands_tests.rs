use super::*;

static SESSION_TEST_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[test]
fn failed_sheet_diagnosis_uses_the_shared_oauth_refresh_path() {
    let source = include_str!("google_sheet_commands.rs");
    let command = source
        .split("pub async fn publish_sheet_diagnose_failed(")
        .nth(1)
        .expect("failed Sheet diagnosis command")
        .split("pub async fn publish_sheet_readback(")
        .next()
        .expect("readback command boundary");
    assert!(command.contains("access_tokens(&state.db).await"));
    assert!(!command.contains("tokens.needs_refresh()"));
}

#[tokio::test]
async fn status_keeps_saved_sheet_url_before_direct_provider_is_active() {
    let _session = SESSION_TEST_LOCK.lock().await;
    let path = std::env::temp_dir().join(format!("google-saved-url-{}.db", uuid::Uuid::new_v4()));
    let db = Database::open(&path).unwrap();
    let url = "https://docs.google.com/spreadsheets/d/saved-book/edit#gid=7";
    db.set_setting(riviu_core::publish_sheet::SHEET_URL_SETTING, url)
        .unwrap();
    let result = status(&db).await.unwrap();
    assert!(!result.active);
    assert_eq!(result.sheet_url.as_deref(), Some(url));
}

#[test]
fn shared_foreign_writer_can_join_preserving_epoch_without_upgrade_confirmation() {
    let path = std::env::temp_dir().join(format!("google-shared-join-{}.db", uuid::Uuid::new_v4()));
    let db = Database::open(path).unwrap();
    let target = connection().target;
    begin_checked_migration(
        &db,
        &checked(),
        &target,
        &db.google_writer_id().unwrap(),
        &uuid::Uuid::new_v4().to_string(),
        false,
    )
    .unwrap();
    let pending: serde_json::Value =
        serde_json::from_str(&db.get_setting(GOOGLE_MIGRATION_SETTING).unwrap().unwrap()).unwrap();
    assert_eq!(pending["target"]["reportingEpoch"], "fixture-epoch");
}

#[test]
fn legacy_upgrade_uses_typed_error_and_ordinary_confirmation_is_not_drain_proof() {
    let path = std::env::temp_dir().join(format!("google-upgrade-{}.db", uuid::Uuid::new_v4()));
    let db = Database::open(path).unwrap();
    let mut initial = checked();
    initial.writer_schema_version = Some(1);
    let error = begin_checked_migration(
        &db,
        &initial,
        &connection().target,
        &db.google_writer_id().unwrap(),
        &uuid::Uuid::new_v4().to_string(),
        false,
    )
    .unwrap_err();
    assert_eq!(connection_error(error).code, "SharedSheetUpgradeRequired");
    assert!(db.get_setting(GOOGLE_MIGRATION_SETTING).unwrap().is_none());
}

#[test]
fn legacy_or_busy_sheet_does_not_pause_delivery_and_another_tab_can_be_selected() {
    let path = std::env::temp_dir().join(format!("google-connect-{}.db", uuid::Uuid::new_v4()));
    let db = Database::open(&path).unwrap();
    let local_writer = db.google_writer_id().unwrap();
    let target = connection().target;
    let request = uuid::Uuid::new_v4().to_string();
    let mut initial = checked();
    initial.writer_schema_version = Some(1);
    assert!(
        begin_checked_migration(&db, &initial, &target, &local_writer, &request, false).is_err()
    );
    assert!(db.get_setting(GOOGLE_MIGRATION_SETTING).unwrap().is_none());
    initial.writer_id = Some(local_writer.clone());
    initial.reporting_ready = false;
    assert!(
        begin_checked_migration(&db, &initial, &target, &local_writer, &request, true).is_err()
    );
    assert!(db.get_setting(GOOGLE_MIGRATION_SETTING).unwrap().is_none());
    initial.writer_id = None;
    initial.writer_schema_version = None;
    initial.sheet_gid = 7;
    initial.reporting_epoch = None;
    initial.layout = None;
    let mut other_target = target;
    other_target.sheet_gid = 7;
    other_target.reporting_epoch = None;
    begin_checked_migration(&db, &initial, &other_target, &local_writer, &request, false).unwrap();
    let pending: serde_json::Value =
        serde_json::from_str(&db.get_setting(GOOGLE_MIGRATION_SETTING).unwrap().unwrap()).unwrap();
    assert_eq!(pending["target"]["sheetGid"], 7);
    drop(db);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn same_writer_can_resume_only_the_frozen_epoch_and_layout() {
    let path = std::env::temp_dir().join(format!("google-resume-{}.db", uuid::Uuid::new_v4()));
    let db = Database::open(&path).unwrap();
    let local_writer = db.google_writer_id().unwrap();
    let target = connection().target;
    let request = uuid::Uuid::new_v4().to_string();
    let mut initial = checked();
    initial.writer_id = Some(local_writer.clone());
    initial.reporting_epoch = Some("changed".into());
    assert!(
        begin_checked_migration(&db, &initial, &target, &local_writer, &request, true).is_err()
    );
    initial.reporting_epoch = target.reporting_epoch.clone();
    initial.layout = Some("compact".into());
    assert!(
        begin_checked_migration(&db, &initial, &target, &local_writer, &request, true).is_err()
    );
    initial.layout = Some("internal".into());
    begin_checked_migration(&db, &initial, &target, &local_writer, &request, true).unwrap();
    begin_checked_migration(&db, &initial, &target, &local_writer, &request, true).unwrap();
    drop(db);
    std::fs::remove_file(path).unwrap();
}

#[tokio::test]
async fn staged_account_is_visible_but_never_active_or_exposes_tokens() {
    let _session = SESSION_TEST_LOCK.lock().await;
    let db = login_db();
    db.set_google_oauth_tokens(Some(&oauth_tokens("old")))
        .unwrap();
    db.set_setting(riviu_core::db::SHEET_PROVIDER_SETTING, "googleDirect")
        .unwrap();
    sessions().lock().await.staged = Some(StagedAuthorization {
        tokens: oauth_tokens("new"),
    });
    let result = status(&db).await;
    sessions().lock().await.staged = None;
    let result = result.unwrap();
    assert!(result.connected && result.has_sheets_scope);
    assert!(!result.active);
    assert_eq!(result.account_id.as_deref(), Some("new"));
    let wire = serde_json::to_string(&result).unwrap();
    assert!(!wire.contains("access-new") && !wire.contains("refresh-new"));
    assert_eq!(db.google_oauth_tokens().unwrap().unwrap().account_id, "old");
}

#[tokio::test]
async fn connect_without_picker_reaches_login_guard_not_file_selection() {
    let _session = SESSION_TEST_LOCK.lock().await;
    let db = login_db();
    let error = connect(&db, "fixture-book", 0, true, false)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("Đăng nhập Google"));
    assert!(db.get_setting(PICKED_FILE).unwrap().is_none());
}

#[test]
fn direct_writer_upgrade_and_join_do_not_retire_stale_legacy_webhook() {
    for schema in [1, 2] {
        assert!(
            !needs_legacy_retirement(true, Some(schema)),
            "Schema direct {schema} không được yêu cầu PC mới retire Apps Script lần nữa"
        );
    }
}

#[test]
fn unclaimed_sheet_still_requires_authenticated_legacy_retirement() {
    assert!(needs_legacy_retirement(true, None));
    assert!(!needs_legacy_retirement(false, None));
}

#[test]
fn unknown_direct_schema_does_not_bypass_retirement_or_migration_validation() {
    for schema in [0, 3] {
        assert!(needs_legacy_retirement(true, Some(schema)));
        let mut initial = checked();
        initial.writer_schema_version = Some(schema);
        assert!(validate_migration_target(&initial, &connection().target, true).is_err());
    }
}

#[test]
fn direct_upgrade_resumes_existing_checkpoint_with_its_original_target_and_request() {
    let db = login_db();
    let mut bound = connection();
    bound.writer_id = db.google_writer_id().unwrap();
    let request = uuid::Uuid::new_v4().to_string();
    db.begin_google_sheet_migration(&bound.target, &request)
        .unwrap();
    let checkpoint = db.get_setting(GOOGLE_MIGRATION_SETTING).unwrap();
    let mut initial = checked();
    initial.writer_schema_version = Some(1);
    assert!(!needs_legacy_retirement(
        true,
        initial.writer_schema_version
    ));
    let error = begin_checked_migration(
        &db,
        &initial,
        &bound.target,
        &bound.writer_id,
        &request,
        false,
    )
    .unwrap_err();
    assert_eq!(connection_error(error).code, "SharedSheetUpgradeRequired");
    assert_eq!(
        db.get_setting(GOOGLE_MIGRATION_SETTING).unwrap(),
        checkpoint
    );
    begin_checked_migration(
        &db,
        &initial,
        &bound.target,
        &bound.writer_id,
        &request,
        true,
    )
    .unwrap();
    assert_eq!(
        db.get_setting(GOOGLE_MIGRATION_SETTING).unwrap(),
        checkpoint
    );
    finish_checked_connection(
        &db,
        &mut SessionState::default(),
        0,
        &bound,
        &request,
        None,
        false,
    )
    .unwrap();
    assert!(db.sheet_uses_google_direct().unwrap());
    assert_eq!(db.google_sheet_connection().unwrap(), Some(bound));
    assert_eq!(
        db.get_setting(GOOGLE_MIGRATION_SETTING).unwrap().as_deref(),
        Some("")
    );
}

#[test]
fn ordinary_errors_never_request_a_legacy_upgrade() {
    let error = connection_error(anyhow::anyhow!("Không có quyền sửa bảng"));
    assert_eq!(error.code, "OperationFailed");
    assert!(error.message.contains("quyền"));
}

#[test]
fn cancelled_connect_releases_only_its_fresh_barrier_and_keeps_previous_credentials() {
    let db = login_db();
    db.set_google_oauth_tokens(Some(&oauth_tokens("old")))
        .unwrap();
    let mut bound = connection();
    bound.writer_id = db.google_writer_id().unwrap();
    let request = uuid::Uuid::new_v4().to_string();
    db.begin_google_sheet_migration(&bound.target, &request)
        .unwrap();
    let mut session = SessionState {
        generation: 2,
        ..Default::default()
    };
    assert!(finish_checked_connection(&db, &mut session, 1, &bound, &request, None, true).is_err());
    assert_eq!(db.google_oauth_tokens().unwrap().unwrap().account_id, "old");
    assert!(db.google_sheet_connection().unwrap().is_none());
    assert!(db
        .get_setting(GOOGLE_MIGRATION_SETTING)
        .unwrap()
        .unwrap()
        .is_empty());
}

#[test]
fn cancelled_connect_preserves_preexisting_or_retirement_checkpoint() {
    let db = login_db();
    let bound = connection();
    let request = uuid::Uuid::new_v4().to_string();
    db.begin_google_sheet_migration(&bound.target, &request)
        .unwrap();
    let mut session = SessionState {
        generation: 2,
        ..Default::default()
    };
    assert!(
        finish_checked_connection(&db, &mut session, 1, &bound, &request, None, false).is_err()
    );
    assert!(!db
        .get_setting(GOOGLE_MIGRATION_SETTING)
        .unwrap()
        .unwrap()
        .is_empty());
}

#[test]
fn shared_reset_is_rejected_before_pausing_local_queue() {
    let db = login_db();
    let target = connection();
    let reset = uuid::Uuid::new_v4().to_string();
    assert!(begin_checked_reset(&db, &target, &checked(), &reset).is_err());
    assert!(db.publish_sheet_requests_drained().unwrap());
    // A reset pause would prohibit this migration even though no request is active.
    db.begin_google_sheet_migration(&target.target, &uuid::Uuid::new_v4().to_string())
        .unwrap();
}

#[derive(Default)]
struct MemorySecrets(std::sync::Mutex<std::collections::HashMap<String, String>>);
impl riviu_core::db::SecretStore for MemorySecrets {
    fn get_secret(&self, key: &str) -> anyhow::Result<Option<String>> {
        Ok(self.0.lock().unwrap().get(key).cloned())
    }
    fn set_secret(&self, key: &str, value: &str) -> anyhow::Result<()> {
        self.0.lock().unwrap().insert(key.into(), value.into());
        Ok(())
    }
}
fn login_db() -> Database {
    Database::open(std::env::temp_dir().join(format!("google-login-{}.db", uuid::Uuid::new_v4())))
        .unwrap()
        .with_secrets(std::sync::Arc::new(MemorySecrets::default()))
}
fn oauth_tokens(account: &str) -> GoogleOAuthTokens {
    GoogleOAuthTokens {
        access_token: format!("access-{account}"),
        refresh_token: format!("refresh-{account}"),
        expires_at_ms: i64::MAX,
        scope: riviu_core::google_oauth::GOOGLE_SHEETS_SCOPES.into(),
        account_id: account.into(),
        email: format!("{account}@example.test"),
    }
}

#[test]
fn new_account_is_staged_for_exact_target_without_replacing_previous_credentials() {
    let db = login_db();
    db.set_google_oauth_tokens(Some(&oauth_tokens("old")))
        .unwrap();
    db.set_setting(
        riviu_core::publish_sheet::SHEET_URL_SETTING,
        "https://docs.google.com/spreadsheets/d/fixture-book/edit#gid=0",
    )
    .unwrap();
    let snapshot = LoginSnapshot::capture(&db).unwrap();
    let mut slot = SessionState {
        generation: 7,
        phase: "authorizing",
        ..Default::default()
    };
    complete_authorization(
        &db,
        &mut slot,
        7,
        &snapshot,
        oauth_tokens("new"),
        Some(checked()),
    )
    .unwrap();
    assert_eq!(db.google_oauth_tokens().unwrap().unwrap().account_id, "old");
    assert_eq!(slot.staged.as_ref().unwrap().tokens.account_id, "new");
}

#[test]
fn login_different_account_to_legacy_foreign_owner_stages_without_hidden_upgrade() {
    let db = login_db();
    let old = oauth_tokens("old");
    db.set_google_oauth_tokens(Some(&old)).unwrap();
    let mut bound = connection();
    bound.account_id = old.account_id;
    bound.writer_id = db.google_writer_id().unwrap();
    let request = uuid::Uuid::new_v4().to_string();
    db.begin_google_sheet_migration(&bound.target, &request)
        .unwrap();
    db.finish_google_sheet_migration(&bound, &request).unwrap();
    let snapshot = LoginSnapshot::capture(&db).unwrap();
    let mut read = checked();
    read.writer_schema_version = Some(1);
    read.writer_id = Some("another-installation".into());
    read.writable = false;
    let mut session = SessionState {
        generation: 4,
        ..Default::default()
    };
    complete_authorization(
        &db,
        &mut session,
        4,
        &snapshot,
        oauth_tokens("new"),
        Some(read),
    )
    .unwrap();
    assert_eq!(session.staged.as_ref().unwrap().tokens.account_id, "new");
    assert_eq!(db.google_sheet_connection().unwrap(), Some(bound));
    assert_eq!(db.google_oauth_tokens().unwrap().unwrap().account_id, "old");
    assert!(db
        .get_setting(GOOGLE_MIGRATION_SETTING)
        .unwrap()
        .unwrap()
        .is_empty());
}

#[test]
fn stale_or_wrong_target_callback_never_replaces_credentials_or_stages_account() {
    let db = login_db();
    db.set_google_oauth_tokens(Some(&oauth_tokens("old")))
        .unwrap();
    db.set_setting(
        riviu_core::publish_sheet::SHEET_URL_SETTING,
        "https://docs.google.com/spreadsheets/d/fixture-book/edit#gid=0",
    )
    .unwrap();
    let snapshot = LoginSnapshot::capture(&db).unwrap();
    let mut slot = SessionState {
        generation: 8,
        ..Default::default()
    };
    assert!(complete_authorization(
        &db,
        &mut slot,
        7,
        &snapshot,
        oauth_tokens("new"),
        Some(checked())
    )
    .is_err());
    let mut wrong = checked();
    wrong.sheet_gid = 9;
    assert!(complete_authorization(
        &db,
        &mut slot,
        8,
        &snapshot,
        oauth_tokens("new"),
        Some(wrong)
    )
    .is_err());
    db.set_setting(
        riviu_core::publish_sheet::SHEET_URL_SETTING,
        "https://docs.google.com/spreadsheets/d/other-book/edit#gid=0",
    )
    .unwrap();
    assert!(complete_authorization(
        &db,
        &mut slot,
        8,
        &snapshot,
        oauth_tokens("new"),
        Some(checked())
    )
    .is_err());
    assert_eq!(db.google_oauth_tokens().unwrap().unwrap().account_id, "old");
    assert!(slot.staged.is_none());
}

#[test]
fn legacy_grant_cannot_connect_a_new_url_without_picker_but_keeps_existing_target() {
    let db = login_db();
    let mut tokens = oauth_tokens("account");
    tokens.scope = "openid email https://www.googleapis.com/auth/drive.file".into();
    assert!(ensure_target_scope(&db, &tokens, "fixture-book", 0).is_err());
    let mut bound = connection();
    bound.account_id = "account".into();
    bound.writer_id = db.google_writer_id().unwrap();
    let request = uuid::Uuid::new_v4().to_string();
    db.begin_google_sheet_migration(&bound.target, &request)
        .unwrap();
    db.finish_google_sheet_migration(&bound, &request).unwrap();
    ensure_target_scope(&db, &tokens, "fixture-book", 0).unwrap();
    assert!(ensure_target_scope(&db, &tokens, "fixture-book", 1).is_err());
}

fn connection() -> GoogleSheetConnection {
    GoogleSheetConnection {
        target: SheetDeliveryTarget {
            version: 2,
            spreadsheet_id: "fixture-book".into(),
            sheet_gid: 0,
            internal_reporting: true,
            reporting_epoch: Some("fixture-epoch".into()),
        },
        account_id: "fixture-account".into(),
        writer_id: "fixture-writer".into(),
        spreadsheet_name: "Fixture".into(),
    }
}
fn checked() -> DirectTargetCheck {
    DirectTargetCheck {
        spreadsheet_id: "fixture-book".into(),
        sheet_gid: 0,
        title: "Fixture".into(),
        time_zone: "Asia/Ho_Chi_Minh".into(),
        columns: vec!["STT".into()],
        layout: Some("internal".into()),
        reporting_epoch: Some("fixture-epoch".into()),
        reporting_ready: true,
        writer_id: Some("fixture-writer".into()),
        writer_schema_version: Some(2),
        spreadsheet_name: "Actual workbook".into(),
        writable: true,
    }
}

#[test]
fn direct_readiness_binds_epoch_and_layout_even_when_writer_still_matches() {
    let connection = connection();
    let valid = checked_bound_result(checked(), &connection).unwrap();
    assert!(valid.connection_verified && valid.reporting_ready);
    for (epoch, layout) in [
        (Some("changed-epoch"), Some("internal")),
        (None, Some("internal")),
        (Some("fixture-epoch"), Some("compact")),
        (Some("fixture-epoch"), None),
    ] {
        let mut read = checked();
        read.reporting_epoch = epoch.map(str::to_owned);
        read.layout = layout.map(str::to_owned);
        assert!(checked_bound_result(read, &connection).is_err());
    }
}

#[test]
fn direct_readiness_binds_exact_target_and_requires_shared_write_proof_not_installation_owner() {
    let connection = connection();
    let mut read = checked();
    read.spreadsheet_id = "other-book".into();
    assert!(checked_bound_result(read, &connection).is_err());
    let mut read = checked();
    read.sheet_gid = 9;
    assert!(checked_bound_result(read, &connection).is_err());
    let mut read = checked();
    read.writer_id = Some("other-writer".into());
    let result = checked_bound_result(read, &connection).unwrap();
    assert!(result.connection_verified && result.reporting_ready);
    let mut read = checked();
    read.writable = false;
    let result = checked_bound_result(read, &connection).unwrap();
    assert!(!result.connection_verified && !result.reporting_ready);
    let mut read = checked();
    read.writer_schema_version = Some(1);
    let result = checked_bound_result(read, &connection).unwrap();
    assert!(!result.reporting_ready);
    let mut read = checked();
    read.reporting_ready = false;
    let result = checked_bound_result(read, &connection).unwrap();
    assert!(result.connection_verified && !result.reporting_ready);
}
