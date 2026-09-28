use super::*;
use riviu_core::google_sheets::SheetWritePermission;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SheetVerificationBinding {
    client_id: String,
    account_id: String,
    spreadsheet_id: String,
    sheet_gid: u64,
    writer_id: Option<String>,
    reporting_epoch: Option<String>,
    authorization_generation: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GoogleSheetVerification {
    result: SheetCheckResult,
    binding: SheetVerificationBinding,
    verified_at: i64,
    expires_at: i64,
    ready_read: bool,
    write_permission: SheetWritePermission,
}

pub(super) async fn verify(db: &Database, url: &str) -> anyhow::Result<GoogleSheetVerification> {
    // The local lock freezes connect/configure/cancel while this observation is
    // in flight. It never acquires or renews a remote writer lease.
    let _connection = CONNECTION_LOCK.lock().await;
    let before = status(db).await?;
    anyhow::ensure!(
        before.connected && before.phase == "idle",
        "Đăng nhập Google và hoàn tất cửa sổ xác thực trước khi kiểm tra"
    );
    let staged = sessions()
        .lock()
        .await
        .staged
        .as_ref()
        .map(|s| s.tokens.clone());
    let tokens = if let Some(tokens) = staged {
        if tokens.needs_refresh() {
            GoogleOAuthClient::new(
                app_config::configured(db)?.context("Chưa cấu hình Google OAuth Desktop")?,
            )?
            .refresh(&tokens)
            .await?
        } else {
            tokens
        }
    } else {
        access_tokens(db).await?
    };
    let parsed = riviu_core::publish_sheet::parse_sheet_url(url)?;
    ensure_target_scope(db, &tokens, &parsed.spreadsheet_id, parsed.sheet_gid)?;
    let (check, write_permission) = DirectSheetsClient::new(tokens.access_token.clone())?
        .check_target_readonly(&parsed.spreadsheet_id, parsed.sheet_gid, &tokens.scope)
        .await?;
    let connection = db.google_sheet_connection()?;
    let bound = connection.as_ref().filter(|connection| {
        before.active
            && connection.account_id == tokens.account_id
            && connection.target.spreadsheet_id == parsed.spreadsheet_id
            && connection.target.sheet_gid == parsed.sheet_gid
    });
    let ready_read = matches!(check.layout.as_deref(), Some("internal" | "compact"));
    let reporting_ready =
        check.reporting_ready && check.writer_schema_version == Some(2) && ready_read;
    let epoch = check.reporting_epoch.clone();
    let mut result = if let Some(connection) = bound {
        checked_bound_result(check, connection)?
    } else {
        checked_result(check, "")?
    };
    // Reporting metadata and access capability are independent observations.
    result.reporting_ready = reporting_ready;
    result.connection_verified = bound.is_some() && result.connection_verified;
    result.message = match (
        ready_read,
        reporting_ready,
        bound.is_some(),
        write_permission,
    ) {
        (_, _, _, SheetWritePermission::Denied) => {
            "Tài khoản Google không có quyền chỉnh sửa bảng này."
        }
        (false, _, _, _) => {
            "Đã đọc bảng nhưng header chưa đúng; dùng Kiểm tra kết nối để thiết lập rõ ràng."
        }
        (_, false, _, _) => {
            "Đã đọc bảng; đợt báo cáo chưa sẵn sàng. Dùng Kiểm tra kết nối để kiểm tra thiết lập."
        }
        (_, _, false, _) => {
            "Đã đọc bảng và header; bấm Kiểm tra kết nối để liên kết bảng với tài khoản này."
        }
        (_, _, _, SheetWritePermission::Unknown) => {
            "Đã xác minh quyền đọc và header; quyền ghi sẽ được kiểm tra trước khi đăng."
        }
        (_, _, _, SheetWritePermission::Verified) => {
            "Đã xác minh quyền truy cập và header; khóa ghi vẫn được kiểm tra trước khi đăng."
        }
    }
    .into();
    let after = status(db).await?;
    anyhow::ensure!(
        before.client_id == after.client_id
            && before.account_id == after.account_id
            && before.authorization_generation == after.authorization_generation
            && before.active == after.active
            && before.writer_id == after.writer_id
            && before.reporting_epoch == after.reporting_epoch
            && after.phase == "idle",
        "Kết nối Google đã thay đổi trong lúc xác minh; kiểm tra lại"
    );
    let verified_at = chrono::Utc::now().timestamp_millis();
    Ok(GoogleSheetVerification {
        result,
        binding: SheetVerificationBinding {
            client_id: after.client_id,
            account_id: tokens.account_id,
            spreadsheet_id: parsed.spreadsheet_id,
            sheet_gid: parsed.sheet_gid,
            writer_id: bound.map(|c| c.writer_id.clone()),
            reporting_epoch: epoch,
            authorization_generation: after.authorization_generation,
        },
        verified_at,
        expires_at: verified_at.saturating_add(300_000),
        ready_read,
        write_permission,
    })
}
