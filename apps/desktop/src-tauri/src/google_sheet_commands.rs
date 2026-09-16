//! Google login, per-file selection and direct delivery share the existing outbox.
use crate::{command_error::CommandError, state::AppState};
use anyhow::Context;
use riviu_core::{
    db::{Database, GoogleSheetConnection, GOOGLE_MIGRATION_SETTING},
    google_oauth::{
        GoogleOAuthClient, GoogleOAuthClientConfig, GoogleOAuthTokens, GooglePickerFile,
        GoogleSessionCancel,
    },
    google_sheets::{DirectSheetsClient, DirectTargetCheck},
    publish_sheet::{SheetCheckResult, SheetDeliveryTarget},
};
use serde::Serialize;
use std::sync::OnceLock;
use tauri::State;
use tokio::sync::Mutex;

const PICKED_FILE: &str = "google.sheets.picked-file.v1";
static TOKEN_LOCK: Mutex<()> = Mutex::const_new(());
static CONNECTION_LOCK: Mutex<()> = Mutex::const_new(());
#[path = "google_sheet_app_config.rs"]
mod app_config;
#[cfg(test)]
#[path = "google_sheet_commands_tests.rs"]
mod tests;
#[derive(Default)]
struct SessionState {
    generation: u64,
    phase: &'static str,
    error: Option<String>,
    cancel: Option<GoogleSessionCancel>,
}
fn sessions() -> &'static Mutex<SessionState> {
    static STATE: OnceLock<Mutex<SessionState>> = OnceLock::new();
    STATE.get_or_init(|| {
        Mutex::new(SessionState {
            phase: "idle",
            ..Default::default()
        })
    })
}
fn err(e: impl std::fmt::Display) -> CommandError {
    CommandError::operation(e)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GoogleSheetsStatus {
    configured: bool,
    connected: bool,
    active: bool,
    email: Option<String>,
    account_id: Option<String>,
    client_id: String,
    picker_configured: bool,
    selected_file_id: Option<String>,
    selected_file_name: Option<String>,
    sheet_url: Option<String>,
    writer_id: Option<String>,
    phase: String,
    error: Option<String>,
}
async fn status(db: &Database) -> anyhow::Result<GoogleSheetsStatus> {
    let config = app_config::configured(db)?;
    let tokens = db.google_oauth_tokens()?;
    let connection = db.google_sheet_connection()?;
    let selected = picked(db)?;
    let session = sessions().lock().await;
    let connected = tokens.is_some();
    let active = db.sheet_uses_google_direct()?;
    Ok(GoogleSheetsStatus {
        configured: config.is_some(),
        connected,
        active,
        email: tokens.as_ref().map(|t| t.email.clone()),
        account_id: tokens.as_ref().map(|t| t.account_id.clone()),
        client_id: config
            .as_ref()
            .map(|c| c.client_id.clone())
            .unwrap_or_default(),
        picker_configured: config
            .as_ref()
            .is_some_and(|c| c.picker_api_key.is_some() && c.project_number.is_some()),
        selected_file_id: selected.as_ref().map(|s| s.id.clone()),
        selected_file_name: selected.as_ref().map(|s| s.name.clone()),
        sheet_url: if active {
            connection.as_ref().map(|c| {
                format!(
                    "https://docs.google.com/spreadsheets/d/{}/edit#gid={}",
                    c.target.spreadsheet_id, c.target.sheet_gid
                )
            })
        } else {
            None
        },
        writer_id: connection.map(|c| c.writer_id),
        phase: session.phase.into(),
        error: session.error.clone(),
    })
}
fn picked(db: &Database) -> anyhow::Result<Option<GooglePickerFile>> {
    db.get_setting(PICKED_FILE)?
        .filter(|s| !s.is_empty())
        .map(|s| serde_json::from_str(&s).context("Lựa chọn Sheet đã lưu không hợp lệ"))
        .transpose()
}

pub(crate) async fn access_tokens(db: &Database) -> anyhow::Result<GoogleOAuthTokens> {
    let _lock = TOKEN_LOCK.lock().await;
    let tokens = db
        .google_oauth_tokens()?
        .context("Đăng nhập Google để tiếp tục ghi Sheet")?;
    if !tokens.needs_refresh() {
        return Ok(tokens);
    }
    let client = GoogleOAuthClient::new(
        app_config::configured(db)?.context("Chưa cấu hình Google OAuth Desktop")?,
    )?;
    match client.refresh(&tokens).await {
        Ok(next) => {
            db.set_google_oauth_tokens(Some(&next))?;
            Ok(next)
        }
        Err(error) => {
            if error
                .downcast_ref::<riviu_core::google_oauth::GoogleOAuthError>()
                .is_some_and(|e| e.requires_reconnect())
            {
                sessions().lock().await.error=Some("Phiên Google đã hết hiệu lực; đăng nhập lại để tiếp tục. Hàng chờ vẫn được giữ.".into());
            }
            Err(error)
        }
    }
}
fn open_browser(url: &str) -> anyhow::Result<()> {
    let parsed = reqwest::Url::parse(url)?;
    anyhow::ensure!(
        (parsed.scheme() == "https" && parsed.host_str() == Some("accounts.google.com"))
            || (parsed.scheme() == "http" && parsed.host_str() == Some("127.0.0.1")),
        "Địa chỉ đăng nhập Google không hợp lệ"
    );
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        std::process::Command::new("rundll32.exe")
            .arg("url.dll,FileProtocolHandler")
            .arg(url)
            .creation_flags(0x08000000)
            .spawn()?;
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open").arg(url).spawn()?;
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        std::process::Command::new("xdg-open").arg(url).spawn()?;
    }
    Ok(())
}
#[tauri::command]
pub async fn google_sheets_status(
    state: State<'_, AppState>,
) -> Result<GoogleSheetsStatus, CommandError> {
    let _a = state.ensure_accepting_work()?;
    status(&state.db).await.map_err(err)
}
#[tauri::command]
pub async fn google_sheets_configure(
    state: State<'_, AppState>,
    client_id: String,
    client_secret: Option<String>,
    picker_api_key: Option<String>,
    project_number: Option<String>,
) -> Result<GoogleSheetsStatus, CommandError> {
    let _a = state.ensure_accepting_work()?;
    let _connection = CONNECTION_LOCK.lock().await;
    let _guard = TOKEN_LOCK.lock().await;
    let mut pending = sessions().lock().await;
    if pending.phase != "idle" {
        return Err(err("Hoàn tất hoặc hủy cửa sổ Google đang mở"));
    }
    let previous = app_config::configured(&state.db).map_err(err)?;
    let same = previous
        .as_ref()
        .is_some_and(|c| c.client_id == client_id.trim());
    let keep = |new: Option<String>, old: Option<String>| {
        new.map(|s| s.trim().to_owned())
            .filter(|s| !s.is_empty())
            .or(if same { old } else { None })
    };
    let config = GoogleOAuthClientConfig {
        client_id: client_id.trim().into(),
        client_secret: keep(
            client_secret,
            previous.as_ref().and_then(|c| c.client_secret.clone()),
        ),
        picker_api_key: keep(
            picker_api_key,
            previous.as_ref().and_then(|c| c.picker_api_key.clone()),
        ),
        project_number: keep(
            project_number,
            previous.as_ref().and_then(|c| c.project_number.clone()),
        ),
    };
    state.db.set_google_oauth_config(&config).map_err(err)?;
    if !same {
        state.db.set_google_oauth_tokens(None).map_err(err)?;
        state.db.set_setting(PICKED_FILE, "").map_err(err)?;
    }
    pending.error = None;
    drop(pending);
    drop(_guard);
    status(&state.db).await.map_err(err)
}
#[tauri::command]
pub async fn google_sheets_login(
    state: State<'_, AppState>,
) -> Result<GoogleSheetsStatus, CommandError> {
    let _a = state.ensure_accepting_work()?;
    let _connection = CONNECTION_LOCK.lock().await;
    let mut slot = sessions().lock().await;
    if slot.phase != "idle" {
        return Err(err("Đang có cửa sổ Google chờ hoàn tất"));
    }
    let config = app_config::configured(&state.db)
        .map_err(err)?
        .context("Mở Thiết lập Google để cấu hình ứng dụng trước khi đăng nhập")
        .map_err(err)?;
    // Pin the application's client alongside the local login. A later binary with
    // another bundled client must not refresh this account under that client.
    state.db.set_google_oauth_config(&config).map_err(err)?;
    let client = GoogleOAuthClient::new(config).map_err(err)?;
    let session = client.authorization_session().await.map_err(err)?;
    open_browser(session.authorization_url()).map_err(err)?;
    slot.generation += 1;
    let generation = slot.generation;
    slot.phase = "authorizing";
    slot.error = None;
    slot.cancel = Some(session.cancel_handle());
    let db = state.db.clone();
    drop(slot);
    tokio::spawn(async move {
        let result = session.wait().await;
        let _connection = CONNECTION_LOCK.lock().await;
        let _token = TOKEN_LOCK.lock().await;
        let mut slot = sessions().lock().await;
        if slot.generation != generation {
            return;
        }
        let result = result.and_then(|tokens| {
            if let Some(c) = db.google_sheet_connection()? {
                anyhow::ensure!(
                    c.account_id == tokens.account_id,
                    "Đăng nhập đúng tài khoản Google đã liên kết với bảng này"
                );
            }
            db.set_google_oauth_tokens(Some(&tokens))?;
            db.set_setting(
                "google.sheets.authorization-generation",
                &uuid::Uuid::new_v4().to_string(),
            )?;
            Ok(())
        });
        slot.phase = "idle";
        slot.cancel = None;
        slot.error = result.err().map(|e| e.to_string());
    });
    status(&state.db).await.map_err(err)
}
#[tauri::command]
pub async fn google_sheets_cancel(
    state: State<'_, AppState>,
) -> Result<GoogleSheetsStatus, CommandError> {
    let _a = state.ensure_accepting_work()?;
    let mut slot = sessions().lock().await;
    if let Some(cancel) = slot.cancel.take() {
        cancel.cancel()
    }
    slot.generation += 1;
    slot.phase = "idle";
    slot.error = None;
    drop(slot);
    status(&state.db).await.map_err(err)
}
#[tauri::command]
pub async fn google_sheets_disconnect(
    state: State<'_, AppState>,
) -> Result<GoogleSheetsStatus, CommandError> {
    let _a = state.ensure_accepting_work()?;
    let _connection = CONNECTION_LOCK.lock().await;
    let _token = TOKEN_LOCK.lock().await;
    let mut slot = sessions().lock().await;
    if let Some(c) = slot.cancel.take() {
        c.cancel()
    }
    slot.generation += 1;
    slot.phase = "idle";
    slot.error = None;
    state.db.set_google_oauth_tokens(None).map_err(err)?;
    drop(slot);
    drop(_token);
    status(&state.db).await.map_err(err)
}
#[tauri::command]
pub async fn google_sheets_pick_file(
    state: State<'_, AppState>,
) -> Result<GoogleSheetsStatus, CommandError> {
    let _a = state.ensure_accepting_work()?;
    let _connection = CONNECTION_LOCK.lock().await;
    let tokens = access_tokens(&state.db).await.map_err(err)?;
    let mut slot = sessions().lock().await;
    if slot.phase != "idle" {
        return Err(err("Đang có cửa sổ Google chờ hoàn tất"));
    }
    let client = GoogleOAuthClient::new(
        app_config::configured(&state.db)
            .map_err(err)?
            .context("Chưa cấu hình Google")
            .map_err(err)?,
    )
    .map_err(err)?;
    let session = client.picker_session(&tokens).await.map_err(err)?;
    open_browser(session.picker_url()).map_err(err)?;
    slot.generation += 1;
    let generation = slot.generation;
    slot.phase = "picking";
    slot.error = None;
    slot.cancel = Some(session.cancel_handle());
    let db = state.db.clone();
    drop(slot);
    tokio::spawn(async move {
        let result = session.wait().await;
        let _connection = CONNECTION_LOCK.lock().await;
        let mut slot = sessions().lock().await;
        if slot.generation != generation {
            return;
        }
        let result =
            result.and_then(|file| db.set_setting(PICKED_FILE, &serde_json::to_string(&file)?));
        slot.phase = "idle";
        slot.cancel = None;
        slot.error = result.err().map(|e| e.to_string());
    });
    status(&state.db).await.map_err(err)
}
#[tauri::command]
pub async fn google_sheets_list_tabs(
    state: State<'_, AppState>,
    spreadsheet_id: String,
) -> Result<Vec<riviu_core::google_sheets::DirectSheetTab>, CommandError> {
    let _a = state.ensure_accepting_work()?;
    let file = picked(&state.db)
        .map_err(err)?
        .context("Chọn Sheet qua Google trước")
        .map_err(err)?;
    if file.id != spreadsheet_id {
        return Err(err("Lựa chọn Sheet đã thay đổi"));
    }
    let tokens = access_tokens(&state.db).await.map_err(err)?;
    DirectSheetsClient::new(tokens.access_token)
        .map_err(err)?
        .list_tabs(&spreadsheet_id)
        .await
        .map_err(err)
}
fn checked_result(check: DirectTargetCheck, writer: &str) -> anyhow::Result<SheetCheckResult> {
    let mut result = riviu_core::publish_sheet::parse_sheet_url(&format!(
        "https://docs.google.com/spreadsheets/d/{}/edit#gid={}",
        check.spreadsheet_id, check.sheet_gid
    ))?;
    result.readable = true;
    result.connection_verified = check.writer_id.as_deref() == Some(writer);
    result.reporting_ready = check.reporting_ready && result.connection_verified;
    result.reporting_epoch = check.reporting_epoch;
    result.layout = check.layout;
    result.columns = check.columns;
    result.message = if result.reporting_ready {
        "Google Sheets đã sẵn sàng ghi trực tiếp"
    } else {
        "Bảng chưa được liên kết để ghi trên máy tính này"
    }
    .into();
    Ok(result)
}
fn checked_bound_result(
    check: DirectTargetCheck,
    connection: &GoogleSheetConnection,
) -> anyhow::Result<SheetCheckResult> {
    anyhow::ensure!(
        check.spreadsheet_id == connection.target.spreadsheet_id
            && check.sheet_gid == connection.target.sheet_gid
            && check.reporting_epoch == connection.target.reporting_epoch
            && check.layout.as_deref()
                == Some(if connection.target.internal_reporting {
                    "internal"
                } else {
                    "compact"
                }),
        "Đợt báo cáo hoặc bố cục Google Sheet đã thay đổi; kết nối lại đúng bảng"
    );
    checked_result(check, &connection.writer_id)
}
pub(crate) async fn check_current(db: &Database, url: &str) -> anyhow::Result<SheetCheckResult> {
    if !db.sheet_uses_google_direct()? {
        return riviu_core::publish_sheet::check_sheet(url, &db.publish_sheet_delivery_settings()?)
            .await;
    }
    let connection = db
        .google_sheet_connection()?
        .context("Chọn và kết nối Google Sheet")?;
    let parsed = riviu_core::publish_sheet::parse_sheet_url(url)?;
    anyhow::ensure!(
        parsed.spreadsheet_id == connection.target.spreadsheet_id
            && parsed.sheet_gid == connection.target.sheet_gid,
        "Chọn đúng bảng và tab đã liên kết Google"
    );
    let tokens = access_tokens(db).await?;
    anyhow::ensure!(
        tokens.account_id == connection.account_id,
        "Đăng nhập đúng tài khoản Google đã liên kết"
    );
    let check = DirectSheetsClient::new(tokens.access_token)?
        .check_target(&parsed.spreadsheet_id, parsed.sheet_gid)
        .await?;
    checked_bound_result(check, &connection)
}

#[tauri::command]
pub async fn google_sheets_connect(
    state: State<'_, AppState>,
    spreadsheet_id: String,
    sheet_id: u64,
    confirmed: bool,
) -> Result<SheetCheckResult, CommandError> {
    let _a = state.ensure_accepting_work()?;
    let _guard = CONNECTION_LOCK.lock().await;
    let result = connect(&state.db, &spreadsheet_id, sheet_id, confirmed).await;
    result.map_err(err)
}
async fn connect(
    db: &Database,
    book: &str,
    gid: u64,
    confirmed: bool,
) -> anyhow::Result<SheetCheckResult> {
    anyhow::ensure!(
        sessions().lock().await.phase == "idle",
        "Hoàn tất hoặc hủy đăng nhập/chọn bảng trước khi kết nối"
    );
    anyhow::ensure!(confirmed, "Xác nhận kết nối bảng để ghi kết quả");
    let file = picked(db)?.context("Chọn bảng bằng Google Picker")?;
    anyhow::ensure!(file.id == book, "Bảng được chọn đã thay đổi");
    let tokens = access_tokens(db).await?;
    let client = DirectSheetsClient::new(tokens.access_token.clone())?;
    let initial = client.check_target(book, gid).await?;
    let writer = db.google_writer_id()?;
    let pending = db
        .get_setting(GOOGLE_MIGRATION_SETTING)?
        .filter(|s| !s.is_empty())
        .map(|s| serde_json::from_str::<serde_json::Value>(&s))
        .transpose()?;
    let request_id = pending
        .as_ref()
        .and_then(|v| v["requestId"].as_str())
        .map(str::to_owned)
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let mut target = SheetDeliveryTarget {
        version: 2,
        spreadsheet_id: book.into(),
        sheet_gid: gid,
        reporting_epoch: initial.reporting_epoch.clone(),
        internal_reporting: initial.layout.as_deref() != Some("compact"),
    };
    let legacy = db.publish_sheet_delivery_settings()?;
    let saved = db
        .get_setting(riviu_core::publish_sheet::SHEET_URL_SETTING)?
        .and_then(|url| riviu_core::publish_sheet::parse_sheet_url(&url).ok());
    let migrating_legacy = !db.sheet_uses_google_direct()?
        && saved
            .as_ref()
            .is_some_and(|s| s.spreadsheet_id == book && s.sheet_gid == gid)
        && !legacy.webhook_url.is_empty()
        && !legacy.token.is_empty();
    if let Some(pending) = &pending {
        target = serde_json::from_value(pending["target"].clone())?;
        anyhow::ensure!(
            target.spreadsheet_id == book && target.sheet_gid == gid,
            "Tiếp tục chuyển đúng bảng đang chờ"
        );
    } else if migrating_legacy {
        let old =
            riviu_core::publish_sheet::check_sheet(&saved.as_ref().unwrap().sheet_url, &legacy)
                .await?;
        anyhow::ensure!(old.connection_verified, "{}", old.message);
        target.reporting_epoch = old.reporting_epoch;
    }
    begin_checked_migration(db, &initial, &target, &writer, &request_id)?;
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(125);
    while !db.publish_sheet_requests_drained()? {
        anyhow::ensure!(
            tokio::time::Instant::now() < deadline,
            "Còn yêu cầu Sheet đang chạy; tiếp tục kết nối để hoàn tất"
        );
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    }
    if migrating_legacy {
        riviu_core::publish_sheet::retire_apps_script_writer(
            &legacy,
            &target,
            &request_id,
            &writer,
        )
        .await?;
    }
    let ready = client.prepare_target(&target, &writer).await?;
    anyhow::ensure!(ready.reporting_ready, "Google Sheet chưa sẵn sàng");
    target.reporting_epoch = ready.reporting_epoch.clone();
    target.internal_reporting = ready.layout.as_deref() == Some("internal");
    db.finish_google_sheet_migration(
        &GoogleSheetConnection {
            target,
            account_id: tokens.account_id,
            writer_id: writer.clone(),
            spreadsheet_name: file.name,
        },
        &request_id,
    )?;
    sessions().lock().await.error = None;
    checked_result(ready, &writer)
}

fn begin_checked_migration(
    db: &Database,
    initial: &DirectTargetCheck,
    target: &SheetDeliveryTarget,
    writer: &str,
    request_id: &str,
) -> anyhow::Result<()> {
    // Refuse a known conflict before persisting the migration barrier. That
    // barrier pauses every Sheet delivery and must not strand an existing
    // connection merely because the operator selected another PC's tab.
    anyhow::ensure!(
        initial.spreadsheet_id == target.spreadsheet_id && initial.sheet_gid == target.sheet_gid,
        "Kết quả kiểm tra không khớp bảng và tab đã chọn"
    );
    if let Some(owner) = initial.writer_id.as_deref() {
        anyhow::ensure!(
            owner == writer,
            "Tab này đang được liên kết với máy tính khác. Chọn tab mới hoặc dùng máy đã liên kết để tiếp tục ghi; app chưa đổi kết nối hiện tại."
        );
        anyhow::ensure!(
            initial.reporting_ready
                && target.reporting_epoch.as_ref().is_none_or(|epoch| Some(epoch) == initial.reporting_epoch.as_ref())
                && initial.layout.as_deref() == Some(if target.internal_reporting { "internal" } else { "compact" }),
            "Tab đang dọn dữ liệu hoặc đợt báo cáo/bố cục đã đổi; hoàn tất trên máy đã liên kết rồi thử lại"
        );
    }
    db.begin_google_sheet_migration(target, request_id)
}
pub(crate) async fn deliver(
    db: &Database,
    claim: &riviu_core::db::SheetDeliveryClaim,
) -> anyhow::Result<()> {
    let connection = db
        .google_connection_for_target(&claim.target)?
        .context("Hàng chờ thuộc bảng hoặc đợt chưa liên kết Google; giữ nguyên để đối chiếu")?;
    let tokens = access_tokens(db).await?;
    anyhow::ensure!(
        tokens.account_id == connection.account_id,
        "Tài khoản Google khác đích đã chọn"
    );
    let mut payload = match &claim.payload {
        riviu_core::db::SheetDeliveryPayload::Canonical { row, metadata } => {
            let mut value = serde_json::json!({"rowKind":"canonical","assignmentId":row.assignment_id,"postUrl":row.post_url,"poster":row.poster,"partners":row.partners,"postedAt":row.posted_at,"deliveryRevision":row.revision});
            if let Some(meta) = metadata {
                value
                    .as_object_mut()
                    .unwrap()
                    .extend(serde_json::to_value(meta)?.as_object().unwrap().clone());
            }
            value
        }
        riviu_core::db::SheetDeliveryPayload::Report(row) => {
            let mut value = serde_json::to_value(row)?;
            value["deliveryRevision"] = serde_json::json!(row.metadata.row_revision);
            value
        }
    };
    payload["publicationId"] = payload["assignmentId"].clone();
    payload["deliveryVersion"] = serde_json::json!(2);
    payload["spreadsheetId"] = serde_json::json!(claim.target.spreadsheet_id);
    payload["sheetGid"] = serde_json::json!(claim.target.sheet_gid);
    payload["reportingEpoch"] =
        serde_json::json!(claim.target.reporting_epoch.as_deref().unwrap_or("legacy"));
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(95),
        DirectSheetsClient::new(tokens.access_token)?.deliver(
            &claim.target,
            &payload,
            &connection.writer_id,
        ),
    )
    .await
    .context("Hết thời gian ghi Google Sheet; kiểm tra dòng trước khi thử lại")?;
    let receipt = match result {
        Ok(receipt) => receipt,
        Err(error)
            if error.kind == riviu_core::google_sheets::DirectSheetsErrorKind::Unauthorized =>
        {
            let _guard = TOKEN_LOCK.lock().await;
            if let Some(mut tokens) = db.google_oauth_tokens()? {
                tokens.expires_at_ms = 0;
                db.set_google_oauth_tokens(Some(&tokens))?;
            }
            return Err(riviu_core::google_oauth::GoogleOAuthError {
                code: riviu_core::google_oauth::GoogleOAuthErrorCode::Transient,
                message: "Google yêu cầu làm mới phiên; giữ dòng chờ và kiểm tra lại trước khi ghi"
                    .into(),
            }
            .into());
        }
        Err(error) => return Err(error.into()),
    };
    anyhow::ensure!(
        receipt.publication_id == claim.assignment_id
            && receipt.reporting_epoch
                == claim.target.reporting_epoch.as_deref().unwrap_or("legacy")
            && receipt.post_url == payload["postUrl"].as_str().unwrap_or_default(),
        "Google Sheet trả bằng chứng khác bài đang ghi"
    );
    log::info!(
        "Google Sheets receipt publication={} row={} revision={} epoch={}",
        receipt.publication_id,
        receipt.row,
        receipt.revision,
        receipt.reporting_epoch
    );
    Ok(())
}
pub(crate) fn retryable(error: &anyhow::Error) -> bool {
    error
        .downcast_ref::<riviu_core::google_sheets::DirectSheetsError>()
        .is_some_and(|e| e.retryable())
        || error
            .downcast_ref::<riviu_core::google_oauth::GoogleOAuthError>()
            .is_some_and(|e| e.is_retryable())
        || error
            .downcast_ref::<tokio::time::error::Elapsed>()
            .is_some()
}

pub(crate) async fn reset_reporting(
    db: &Database,
    reset_id: &str,
) -> anyhow::Result<serde_json::Value> {
    let _guard = CONNECTION_LOCK.lock().await;
    uuid::Uuid::parse_str(reset_id)?;
    let connection = db
        .google_sheet_connection()?
        .context("Chưa kết nối Google Sheet")?;
    anyhow::ensure!(
        connection.target.sheet_gid == 0,
        "Dọn bảng chỉ dành cho tab gid=0"
    );
    db.begin_publish_sheet_reset(&connection.target, reset_id)?;
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(125);
    while !db.publish_sheet_requests_drained()? {
        anyhow::ensure!(
            tokio::time::Instant::now() < deadline,
            "Còn yêu cầu Google Sheet đang chạy; tiếp tục bằng cùng resetId"
        );
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    }
    let tokens = access_tokens(db).await?;
    anyhow::ensure!(
        tokens.account_id == connection.account_id,
        "Tài khoản Google khác bảng đã chọn"
    );
    let reset = DirectSheetsClient::new(tokens.access_token)?
        .reset_target(&connection.target, &connection.writer_id, reset_id)
        .await?;
    anyhow::ensure!(
        reset.complete && reset.reporting_epoch == reset_id,
        "Chưa xác nhận hoàn tất dọn Google Sheet"
    );
    db.finish_publish_sheet_reset(&connection.target, reset_id, &reset.backup_spreadsheet_id)?;
    Ok(
        serde_json::json!({"reportingEpoch":reset_id,"backupSpreadsheetId":reset.backup_spreadsheet_id,"sheetGid":0,"complete":true}),
    )
}
