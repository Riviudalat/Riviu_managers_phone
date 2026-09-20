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
    staged: Option<StagedAuthorization>,
}
struct StagedAuthorization {
    tokens: GoogleOAuthTokens,
}
#[derive(PartialEq, Eq)]
struct LoginSnapshot {
    connection: Option<GoogleSheetConnection>,
    saved_url: Option<String>,
}
impl LoginSnapshot {
    fn capture(db: &Database) -> anyhow::Result<Self> {
        Ok(Self {
            connection: db.google_sheet_connection()?,
            saved_url: db
                .get_setting(riviu_core::publish_sheet::SHEET_URL_SETTING)?
                .filter(|url| !url.trim().is_empty()),
        })
    }
}
impl LoginSnapshot {
    fn target(&self) -> anyhow::Result<Option<(String, u64)>> {
        if let Some(connection) = &self.connection {
            return Ok(Some((
                connection.target.spreadsheet_id.clone(),
                connection.target.sheet_gid,
            )));
        }
        self.saved_url
            .as_deref()
            .map(riviu_core::publish_sheet::parse_sheet_url)
            .transpose()
            .map(|parsed| parsed.map(|p| (p.spreadsheet_id, p.sheet_gid)))
    }
}
fn complete_authorization(
    db: &Database,
    slot: &mut SessionState,
    generation: u64,
    snapshot: &LoginSnapshot,
    tokens: GoogleOAuthTokens,
    check: Option<DirectTargetCheck>,
) -> anyhow::Result<()> {
    anyhow::ensure!(
        slot.generation == generation && *snapshot == LoginSnapshot::capture(db)?,
        "Kết nối đã thay đổi hoặc đăng nhập đã hủy; phiên trước vẫn được giữ"
    );
    anyhow::ensure!(
        tokens.has_sheets_scope(),
        "Đăng nhập lại và cấp quyền Google Sheets để kết nối bằng link"
    );
    if let Some((book, gid)) = snapshot.target()? {
        let check = check.context("Chưa kiểm tra quyền truy cập bảng đã lưu")?;
        anyhow::ensure!(
            check.spreadsheet_id == book && check.sheet_gid == gid,
            "Kết quả Google không khớp bảng đã lưu"
        );
        if let Some(connection) = &snapshot.connection {
            checked_bound_result(check, connection)?;
        }
        // Reading does not prove editing. Keep the previous credential until
        // explicit connect obtains a fresh shared lock with these exact tokens.
        slot.staged = Some(StagedAuthorization { tokens });
    } else {
        db.set_google_oauth_tokens(Some(&tokens))?;
    }
    Ok(())
}
fn ensure_target_scope(
    db: &Database,
    tokens: &GoogleOAuthTokens,
    book: &str,
    gid: u64,
) -> anyhow::Result<()> {
    anyhow::ensure!(tokens.has_sheets_scope() || db.google_sheet_connection()?.is_some_and(|c|
        c.target.spreadsheet_id == book && c.target.sheet_gid == gid && c.account_id == tokens.account_id),
        "Phiên Google cũ chỉ có quyền theo file; đăng nhập lại và cấp quyền Google Sheets để dùng link này");
    Ok(())
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
#[derive(Debug)]
struct SharedSheetUpgradeRequired;
impl std::fmt::Display for SharedSheetUpgradeRequired {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Cần nâng cấp tab để nhiều máy cùng ghi. Dừng và chờ hoàn tất ghi trên tất cả bản Riviu cũ trước khi xác nhận; dữ liệu và đợt báo cáo được giữ nguyên.")
    }
}
impl std::error::Error for SharedSheetUpgradeRequired {}
pub(crate) fn connection_error(error: anyhow::Error) -> CommandError {
    if error.is::<SharedSheetUpgradeRequired>()
        || error
            .downcast_ref::<riviu_core::google_sheets::DirectSheetsError>()
            .is_some_and(|e| {
                e.kind == riviu_core::google_sheets::DirectSheetsErrorKind::SharedUpgradeRequired
            })
    {
        return CommandError::code(
            "SharedSheetUpgradeRequired",
            SharedSheetUpgradeRequired.to_string(),
        );
    }
    if let Some(sheet) = error.downcast_ref::<riviu_core::google_sheets::DirectSheetsError>() {
        use riviu_core::google_sheets::DirectSheetsErrorKind;
        let message = match sheet.kind {
            DirectSheetsErrorKind::Busy => Some("Google Sheet đang có lượt ghi hoặc lượt trước chưa được xác minh. Giữ nguyên hàng chờ và thử lại; không tự xóa khóa của máy khác."),
            DirectSheetsErrorKind::Forbidden => Some("Tài khoản Google chưa được phép chỉnh sửa bảng này. Kiểm tra quyền chia sẻ và quyền Google Sheets; phiên trước vẫn được giữ."),
            DirectSheetsErrorKind::Unauthorized => Some("Phiên Google không còn hiệu lực. Đăng nhập lại để tiếp tục; hàng chờ vẫn được giữ."),
            DirectSheetsErrorKind::NotFound => Some("Không tìm thấy bảng/tab hoặc tài khoản chưa có quyền truy cập. Kiểm tra đúng link và quyền chia sẻ."),
            _ => None,
        };
        if let Some(message) = message {
            return CommandError::operation(message);
        }
    }
    err(error)
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
    has_sheets_scope: bool,
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
    let tokens = session
        .staged
        .as_ref()
        .map(|stage| &stage.tokens)
        .or(tokens.as_ref());
    let connected = tokens.is_some();
    let active = session.staged.is_none() && db.sheet_uses_google_direct()?;
    Ok(GoogleSheetsStatus {
        configured: config.is_some(),
        connected,
        active,
        email: tokens.map(|t| t.email.clone()),
        account_id: tokens.map(|t| t.account_id.clone()),
        client_id: config
            .as_ref()
            .map(|c| c.client_id.clone())
            .unwrap_or_default(),
        picker_configured: config
            .as_ref()
            .is_some_and(|c| c.picker_api_key.is_some() && c.project_number.is_some()),
        selected_file_id: selected.as_ref().map(|s| s.id.clone()),
        selected_file_name: selected.as_ref().map(|s| s.name.clone()),
        has_sheets_scope: tokens.is_some_and(GoogleOAuthTokens::has_sheets_scope),
        sheet_url: db
            .get_setting(riviu_core::publish_sheet::SHEET_URL_SETTING)?
            .filter(|url| !url.trim().is_empty())
            .or_else(|| {
                connection.as_ref().map(|c| {
                    format!(
                        "https://docs.google.com/spreadsheets/d/{}/edit#gid={}",
                        c.target.spreadsheet_id, c.target.sheet_gid
                    )
                })
            }),
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
    pending.generation += 1;
    pending.staged = None;
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
    let snapshot = LoginSnapshot::capture(&state.db).map_err(err)?;
    let client = GoogleOAuthClient::new(config).map_err(err)?;
    let session = client.authorization_session().await.map_err(err)?;
    open_browser(session.authorization_url()).map_err(err)?;
    slot.generation += 1;
    let generation = slot.generation;
    slot.phase = "authorizing";
    slot.staged = None;
    slot.error = None;
    slot.cancel = Some(session.cancel_handle());
    let db = state.db.clone();
    drop(slot);
    tokio::spawn(async move {
        let result: anyhow::Result<_> = async {
            let tokens = session.wait().await?;
            anyhow::ensure!(
                sessions().lock().await.generation == generation,
                "Đăng nhập Google đã hủy"
            );
            let check = if let Some((book, gid)) = snapshot.target()? {
                Some(
                    DirectSheetsClient::new(tokens.access_token.clone())?
                        .check_target(&book, gid)
                        .await?,
                )
            } else {
                None
            };
            Ok((tokens, check))
        }
        .await;
        let _connection = CONNECTION_LOCK.lock().await;
        let _token = TOKEN_LOCK.lock().await;
        let mut slot = sessions().lock().await;
        if slot.generation != generation {
            return;
        }
        let result = result.and_then(|(tokens, check)| {
            complete_authorization(&db, &mut slot, generation, &snapshot, tokens, check)
        });
        slot.phase = "idle";
        slot.cancel = None;
        slot.error = result.err().map(|e| connection_error(e).message.into());
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
    slot.staged = None;
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
    slot.staged = None;
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
        slot.error = result.err().map(|e| connection_error(e).message.into());
    });
    status(&state.db).await.map_err(err)
}
#[tauri::command]
pub async fn google_sheets_list_tabs(
    state: State<'_, AppState>,
    spreadsheet_id: String,
) -> Result<Vec<riviu_core::google_sheets::DirectSheetTab>, CommandError> {
    let _a = state.ensure_accepting_work()?;
    let tokens = access_tokens(&state.db).await.map_err(err)?;
    ensure_target_scope(&state.db, &tokens, &spreadsheet_id, 0).map_err(err)?;
    DirectSheetsClient::new(tokens.access_token)
        .map_err(err)?
        .list_tabs(&spreadsheet_id)
        .await
        .map_err(err)
}
fn checked_result(check: DirectTargetCheck, _writer: &str) -> anyhow::Result<SheetCheckResult> {
    let mut result = riviu_core::publish_sheet::parse_sheet_url(&format!(
        "https://docs.google.com/spreadsheets/d/{}/edit#gid={}",
        check.spreadsheet_id, check.sheet_gid
    ))?;
    result.readable = true;
    result.connection_verified = check.writer_schema_version == Some(2) && check.writable;
    result.reporting_ready = check.reporting_ready && result.connection_verified;
    result.reporting_epoch = check.reporting_epoch;
    result.layout = check.layout;
    result.columns = check.columns;
    result.message = if result.reporting_ready {
        "Google Sheets đã sẵn sàng ghi trực tiếp"
    } else if check.writer_schema_version == Some(1) {
        "Cần xác nhận tất cả bản Riviu cũ đã dừng ghi để nâng cấp tab dùng chung"
    } else {
        "Chưa xác minh quyền chỉnh sửa và khóa ghi Google Sheet; kết nối lại để kiểm tra"
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
    let _connection = CONNECTION_LOCK.lock().await;
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
    anyhow::ensure!(
        sessions().lock().await.staged.is_none(),
        "Tài khoản mới đang chờ kiểm tra quyền ghi; kết nối lại bảng"
    );
    let client = DirectSheetsClient::new(tokens.access_token)?;
    let initial = client
        .check_target(&parsed.spreadsheet_id, parsed.sheet_gid)
        .await?;
    checked_bound_result(initial.clone(), &connection)?;
    if initial.writer_schema_version == Some(1) {
        return Err(SharedSheetUpgradeRequired.into());
    }
    if initial.writer_schema_version != Some(2) {
        return checked_bound_result(initial, &connection);
    }
    let check = client
        .prepare_shared_target(&connection.target, &connection.writer_id, false, db)
        .await?;
    checked_bound_result(check, &connection)
}

#[tauri::command]
pub async fn google_sheets_connect(
    state: State<'_, AppState>,
    spreadsheet_id: String,
    sheet_id: u64,
    confirmed: bool,
    legacy_writers_stopped: Option<bool>,
) -> Result<SheetCheckResult, CommandError> {
    let _a = state.ensure_accepting_work()?;
    let _guard = CONNECTION_LOCK.lock().await;
    let result = connect(
        &state.db,
        &spreadsheet_id,
        sheet_id,
        confirmed,
        legacy_writers_stopped.unwrap_or(false),
    )
    .await;
    result.map_err(connection_error)
}
fn needs_legacy_retirement(configured: bool, remote_schema: Option<u32>) -> bool {
    // Schema 1/2 đã thuộc đường direct; PC mới không được retire lại Apps Script
    // bằng request/writer khác. Xác nhận dừng writer cũ vẫn do guard nâng cấp giữ.
    configured && !matches!(remote_schema, Some(1 | 2))
}
async fn connect(
    db: &Database,
    book: &str,
    gid: u64,
    confirmed: bool,
    legacy_writers_stopped: bool,
) -> anyhow::Result<SheetCheckResult> {
    let (generation, staged) = {
        let mut session = sessions().lock().await;
        anyhow::ensure!(
            session.phase == "idle",
            "Hoàn tất hoặc hủy đăng nhập/chọn bảng trước khi kết nối"
        );
        session.error = None;
        (
            session.generation,
            session.staged.as_ref().map(|stage| stage.tokens.clone()),
        )
    };
    anyhow::ensure!(confirmed, "Xác nhận kết nối bảng để ghi kết quả");
    let staged_login = staged.is_some();
    let tokens = if let Some(tokens) = staged {
        tokens
    } else {
        access_tokens(db).await?
    };
    ensure_target_scope(db, &tokens, book, gid)?;
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
    if let Some(current) = db.google_sheet_connection()? {
        if current.target.spreadsheet_id == book && current.target.sheet_gid == gid {
            target = current.target;
        }
    }
    let legacy = db.publish_sheet_delivery_settings()?;
    let saved = db
        .get_setting(riviu_core::publish_sheet::SHEET_URL_SETTING)?
        .and_then(|url| riviu_core::publish_sheet::parse_sheet_url(&url).ok());
    let legacy_configured = !db.sheet_uses_google_direct()?
        && saved
            .as_ref()
            .is_some_and(|s| s.spreadsheet_id == book && s.sheet_gid == gid)
        && !legacy.webhook_url.is_empty()
        && !legacy.token.is_empty();
    let migrating_legacy =
        needs_legacy_retirement(legacy_configured, initial.writer_schema_version);
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
    if migrating_legacy && !legacy_writers_stopped {
        return Err(SharedSheetUpgradeRequired.into());
    }
    validate_migration_target(&initial, &target, legacy_writers_stopped)?;
    if migrating_legacy {
        begin_checked_migration(
            db,
            &initial,
            &target,
            &writer,
            &request_id,
            legacy_writers_stopped,
        )?;
        drain_sheet_requests(db).await?;
        riviu_core::publish_sheet::retire_apps_script_writer(
            &legacy,
            &target,
            &request_id,
            &writer,
        )
        .await?;
    }
    let ready = client
        .prepare_shared_target(&target, &writer, legacy_writers_stopped, db)
        .await?;
    // Shared preparation has its own durable lock. Do not pause unrelated
    // outboxes until this target's real edit permission has been proved.
    if !migrating_legacy {
        anyhow::ensure!(
            sessions().lock().await.generation == generation,
            "Kết nối đã hủy; phiên trước vẫn được giữ"
        );
        begin_checked_migration(
            db,
            &ready,
            &target,
            &writer,
            &request_id,
            legacy_writers_stopped,
        )?;
        if let Err(error) = drain_sheet_requests(db).await {
            if pending.is_none() {
                db.abort_google_sheet_migration(&request_id)?;
            }
            return Err(error);
        }
    }
    let result = checked_result(ready.clone(), &writer)?;
    anyhow::ensure!(
        result.reporting_ready,
        "Google Sheet chưa xác minh quyền ghi"
    );
    target.reporting_epoch = ready.reporting_epoch.clone();
    target.internal_reporting = ready.layout.as_deref() == Some("internal");
    let connection = GoogleSheetConnection {
        target,
        account_id: tokens.account_id.clone(),
        writer_id: writer.clone(),
        spreadsheet_name: ready.spreadsheet_name,
    };
    let _token = TOKEN_LOCK.lock().await;
    let mut session = sessions().lock().await;
    finish_checked_connection(
        db,
        &mut session,
        generation,
        &connection,
        &request_id,
        staged_login.then_some(&tokens),
        pending.is_none() && !migrating_legacy,
    )?;
    Ok(result)
}
fn finish_checked_connection(
    db: &Database,
    session: &mut SessionState,
    generation: u64,
    connection: &GoogleSheetConnection,
    request_id: &str,
    tokens: Option<&GoogleOAuthTokens>,
    can_abandon: bool,
) -> anyhow::Result<()> {
    let result = (|| {
        anyhow::ensure!(
            session.generation == generation,
            "Kết nối đã hủy hoặc tài khoản đã thay đổi; phiên trước vẫn được giữ"
        );
        if let Some(tokens) = tokens {
            db.finish_google_sheet_authorization(connection, request_id, tokens)?;
        } else {
            db.finish_google_sheet_migration(connection, request_id)?;
        }
        Ok(())
    })();
    if result.is_err() && can_abandon {
        db.abort_google_sheet_migration(request_id)?;
    }
    result?;
    if tokens.is_some() {
        session.staged = None;
    }
    session.error = None;
    Ok(())
}

fn begin_checked_migration(
    db: &Database,
    initial: &DirectTargetCheck,
    target: &SheetDeliveryTarget,
    _writer: &str,
    request_id: &str,
    legacy_writers_stopped: bool,
) -> anyhow::Result<()> {
    validate_migration_target(initial, target, legacy_writers_stopped)?;
    db.begin_google_sheet_migration(target, request_id)
}

async fn drain_sheet_requests(db: &Database) -> anyhow::Result<()> {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(125);
    while !db.publish_sheet_requests_drained()? {
        anyhow::ensure!(
            tokio::time::Instant::now() < deadline,
            "Còn yêu cầu Sheet đang chạy; tiếp tục kết nối để hoàn tất"
        );
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    }
    Ok(())
}
fn validate_migration_target(
    initial: &DirectTargetCheck,
    target: &SheetDeliveryTarget,
    legacy_writers_stopped: bool,
) -> anyhow::Result<()> {
    anyhow::ensure!(
        initial.spreadsheet_id == target.spreadsheet_id && initial.sheet_gid == target.sheet_gid,
        "Kết quả kiểm tra không khớp bảng và tab đã chọn"
    );
    if initial.writer_schema_version == Some(1) && !legacy_writers_stopped {
        return Err(SharedSheetUpgradeRequired.into());
    }
    if initial.writer_id.is_some() {
        anyhow::ensure!(
            matches!(initial.writer_schema_version, Some(1 | 2)),
            "Phiên bản kết nối Google Sheet chưa được hỗ trợ"
        );
        anyhow::ensure!(
            initial.reporting_ready
                && target.reporting_epoch.as_ref().is_none_or(|epoch| Some(epoch) == initial.reporting_epoch.as_ref())
                && initial.layout.as_deref() == Some(if target.internal_reporting { "internal" } else { "compact" }),
            "Tab đang dọn dữ liệu hoặc đợt báo cáo/bố cục đã đổi; hoàn tất trên máy đã liên kết rồi thử lại"
        );
    }
    Ok(())
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
        DirectSheetsClient::new(tokens.access_token)?.deliver_shared(
            &claim.target,
            &payload,
            &connection.writer_id,
            db,
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
    db.save_sheet_receipt(&claim.target, &receipt)?;
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

pub use riviu_core::ipc_contract::PublishSheetReadback;

#[tauri::command]
pub async fn publish_sheet_readback(
    state: State<'_, AppState>,
    assignment_id: String,
    expected_revision: i64,
) -> Result<PublishSheetReadback, CommandError> {
    let _admission = state.ensure_accepting_work()?;
    let (target, expected) = state
        .db
        .sheet_readback_input(&assignment_id, expected_revision)
        .map_err(err)?;
    let connection = state
        .db
        .google_connection_for_target(&target)
        .map_err(err)?
        .context("No OAuth connection for publication target")
        .map_err(err)?;
    let tokens = access_tokens(&state.db).await.map_err(err)?;
    if tokens.account_id != connection.account_id {
        return Err(err("Google account changed"));
    }
    let client = DirectSheetsClient::new(tokens.access_token).map_err(err)?;
    let receipt = tokio::time::timeout(
        std::time::Duration::from_secs(90),
        client.readback_receipt(&target, &expected),
    )
    .await
    .map_err(err)?
    .map_err(err)?;
    state
        .db
        .sheet_readback_input(&assignment_id, expected_revision)
        .map_err(err)?;
    Ok(PublishSheetReadback {
        assignment_id,
        url: receipt.post_url.clone(),
        range: format!("gid={}:D{}", target.sheet_gid, receipt.row),
        revision: receipt.revision,
        epoch: receipt.reporting_epoch.clone(),
        checked_at: chrono::Utc::now().to_rfc3339(),
        receipt,
    })
}

fn begin_checked_reset(
    db: &Database,
    connection: &GoogleSheetConnection,
    check: &DirectTargetCheck,
    reset_id: &str,
) -> anyhow::Result<()> {
    anyhow::ensure!(
        check.spreadsheet_id == connection.target.spreadsheet_id
            && check.sheet_gid == connection.target.sheet_gid,
        "Kết quả kiểm tra không khớp bảng cần dọn"
    );
    anyhow::ensure!(check.writer_schema_version != Some(2),
        "Tab đang dùng chế độ nhiều máy. Bản 0.2.37 tạm khóa dọn toàn bảng để bảo vệ dữ liệu và hàng chờ trên các máy khác.");
    db.begin_publish_sheet_reset(&connection.target, reset_id)
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
    let tokens = access_tokens(db).await?;
    anyhow::ensure!(
        tokens.account_id == connection.account_id,
        "Tài khoản Google khác bảng đã chọn"
    );
    let client = DirectSheetsClient::new(tokens.access_token)?;
    let check = client
        .check_target(
            &connection.target.spreadsheet_id,
            connection.target.sheet_gid,
        )
        .await?;
    begin_checked_reset(db, &connection, &check, reset_id)?;
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(125);
    while !db.publish_sheet_requests_drained()? {
        anyhow::ensure!(
            tokio::time::Instant::now() < deadline,
            "Còn yêu cầu Google Sheet đang chạy; tiếp tục bằng cùng resetId"
        );
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    }
    let reset = client
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
