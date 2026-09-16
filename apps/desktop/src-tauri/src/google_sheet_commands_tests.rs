use super::*;

#[test]
fn foreign_or_busy_sheet_does_not_pause_delivery_and_another_tab_can_be_selected() {
    let path = std::env::temp_dir().join(format!("google-connect-{}.db", uuid::Uuid::new_v4()));
    let db = Database::open(&path).unwrap();
    let local_writer = db.google_writer_id().unwrap();
    let target = connection().target;
    let request = uuid::Uuid::new_v4().to_string();
    let mut initial = checked();
    assert!(begin_checked_migration(&db, &initial, &target, &local_writer, &request).is_err());
    assert!(db.get_setting(GOOGLE_MIGRATION_SETTING).unwrap().is_none());
    initial.writer_id = Some(local_writer.clone());
    initial.reporting_ready = false;
    assert!(begin_checked_migration(&db, &initial, &target, &local_writer, &request).is_err());
    assert!(db.get_setting(GOOGLE_MIGRATION_SETTING).unwrap().is_none());
    initial.writer_id = None;
    initial.sheet_gid = 7;
    initial.reporting_epoch = None;
    initial.layout = None;
    let mut other_target = target;
    other_target.sheet_gid = 7;
    other_target.reporting_epoch = None;
    begin_checked_migration(&db, &initial, &other_target, &local_writer, &request).unwrap();
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
    assert!(begin_checked_migration(&db, &initial, &target, &local_writer, &request).is_err());
    initial.reporting_epoch = target.reporting_epoch.clone();
    initial.layout = Some("compact".into());
    assert!(begin_checked_migration(&db, &initial, &target, &local_writer, &request).is_err());
    initial.layout = Some("internal".into());
    begin_checked_migration(&db, &initial, &target, &local_writer, &request).unwrap();
    begin_checked_migration(&db, &initial, &target, &local_writer, &request).unwrap();
    drop(db);
    std::fs::remove_file(path).unwrap();
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
fn direct_readiness_binds_exact_book_tab_and_never_reports_a_foreign_or_busy_writer_ready() {
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
    assert!(!result.connection_verified && !result.reporting_ready);
    let mut read = checked();
    read.reporting_ready = false;
    let result = checked_bound_result(read, &connection).unwrap();
    assert!(result.connection_verified && !result.reporting_ready);
}
