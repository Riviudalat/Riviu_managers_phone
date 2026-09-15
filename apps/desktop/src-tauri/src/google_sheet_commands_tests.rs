use super::*;

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
