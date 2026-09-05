//! Local automation API (Giai đoạn B, xiaowei "openapi" — cổng loopback 22222).
//!
//! A loopback-only HTTP/1.1 server exposing a small, whitelisted set of fleet gestures to
//! scripts running on the same machine — the parity feature for xiaowei's local API. It is
//! **off by default**: it binds nothing until the operator turns it on in Settings, it never
//! binds anything but `127.0.0.1`, and it demands a bearer token on every request, so it is
//! not reachable off-box and not usable without the token the operator was shown.
//!
//! Everything that decides *what* a request means — parsing, routing, auth, response
//! rendering — is a pure function with unit tests at the bottom of this file. The async serve
//! loop is a thin I/O shell around them, and it dispatches through the very same
//! [`crate::commands::with_manual_session`] path the Tauri commands use, so the API can reach
//! a device by no route a command could not (same lease, same admission, same cleanup).

use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::{AppHandle, Manager, State};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::Semaphore;
use tokio::time::timeout;

use riviu_core::db::Database;
use riviu_core::types::{HardwareKey, SwipeGesture, TapPoint};
use riviu_core::DeviceWorkOwner;

use crate::command_error::CommandError;
use crate::state::AppState;
use riviu_signing::CredentialStore;

/// DB key the config is persisted under (KV settings table).
const CONFIG_KEY: &str = "local_api.config.v1";
/// The loopback port xiaowei uses; kept as the default so operators' notes still apply.
const DEFAULT_PORT: u16 = 22222;
/// Hard cap on one request (head + body). Generous for a JSON gesture, small enough that a
/// stray client cannot make the server buffer without bound.
const MAX_REQUEST_BYTES: usize = 64 * 1024;
/// How long a client gets to finish sending its request.
///
/// The size cap alone does not bound a connection: a client that sends **one byte every few
/// seconds** never trips it and holds the task forever, and because auth happens *after* both
/// read loops, this costs an attacker **no token**. Ten seconds is far more than a loopback
/// client needs (the whole point of this API is that it is on the same machine) and far less
/// than the "forever" it was.
const REQUEST_READ_TIMEOUT: Duration = Duration::from_secs(10);
/// After device work settles, an unread response gets the same bounded socket lifetime.
const RESPONSE_WRITE_TIMEOUT: Duration = Duration::from_secs(10);
/// How many connections may be in flight at once.
///
/// Paired with the timeout: the timeout bounds one slow client, this bounds how many of them
/// there can be. 64 is generous for a scripting API driving 20 phones from the same box, and
/// small enough that the accept loop cannot be turned into unbounded task growth. Excess
/// connections wait for a slot rather than being refused, so an honest burst still completes.
const MAX_CONCURRENT_CONNECTIONS: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalApiConfig {
    pub enabled: bool,
    pub port: u16,
    /// Bearer token required on every request. Empty until first generated.
    pub token: String,
}

#[derive(Default)]
pub(crate) struct LocalApiRuntime {
    pub startup: Option<LocalApiConfig>,
    pub running: Option<bool>,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalApiStatus {
    configured_enabled: bool,
    configured_port: u16,
    running: Option<bool>,
    active_port: Option<u16>,
    restart_required: bool,
    last_error: Option<String>,
}

impl LocalApiRuntime {
    fn status(&self, configured: &LocalApiConfig) -> LocalApiStatus {
        LocalApiStatus {
            configured_enabled: configured.enabled,
            configured_port: configured.port,
            running: self.running,
            active_port: self
                .startup
                .as_ref()
                .filter(|_| self.running == Some(true))
                .map(|config| config.port),
            restart_required: self.startup.as_ref().map_or(
                self.running == Some(false) && configured.enabled,
                |startup| startup != configured,
            ),
            last_error: self.last_error.clone(),
        }
    }
}

#[tauri::command]
pub async fn local_api_status(state: State<'_, AppState>) -> Result<LocalApiStatus, CommandError> {
    let configured = load_config(&state.db, &state.secrets).map_err(CommandError::operation)?;
    Ok(state.local_api_runtime.read().status(&configured))
}

impl Default for LocalApiConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            port: DEFAULT_PORT,
            token: String::new(),
        }
    }
}

/// Name the bearer token lives under in the OS credential store.
pub const SECRET_LOCAL_API_TOKEN: &str = "local-api-token";

/// Read the stored config. Missing settings use the disabled default; storage failures
/// remain errors so a credential migration can be retried without losing its source.
///
/// The **token comes from the credential store**, not from the SQLite row: it is a bearer
/// credential for a server that can drive the whole fleet, and the database is a plain
/// unencrypted file. A token still sitting in an old row is migrated on read and removed from
/// the row, so a database written before this change stops carrying it.
pub fn load_config(db: &Database, secrets: &CredentialStore) -> anyhow::Result<LocalApiConfig> {
    let mut config: LocalApiConfig = match db.get_setting(CONFIG_KEY)? {
        Some(raw) => serde_json::from_str(&raw)?,
        None => LocalApiConfig::default(),
    };
    if !config.token.is_empty() {
        // Keep the legacy row until the credential store confirms the write.
        let moved = std::mem::take(&mut config.token);
        secrets.set_app_secret(SECRET_LOCAL_API_TOKEN, &moved)?;
        anyhow::ensure!(
            secrets.app_secret(SECRET_LOCAL_API_TOKEN)?.as_deref() == Some(moved.as_str()),
            "local API credential migration was not confirmed"
        );
        db.set_setting(CONFIG_KEY, &serde_json::to_string(&config)?)?;
        config.token = moved;
        return Ok(config);
    }
    if let Some(token) = secrets.app_secret(SECRET_LOCAL_API_TOKEN)? {
        config.token = token;
    }
    Ok(config)
}

/// Persist the config: everything but the token as JSON, the token in the credential store.
pub fn save_config(
    db: &Database,
    secrets: &CredentialStore,
    config: &LocalApiConfig,
) -> anyhow::Result<()> {
    secrets.set_app_secret(SECRET_LOCAL_API_TOKEN, &config.token)?;
    let mut without = config.clone();
    without.token.clear();
    db.set_setting(CONFIG_KEY, &serde_json::to_string(&without)?)
}

/// A fresh random bearer token: two v4 UUIDs, hyphens stripped (~256 bits of entropy).
pub fn generate_token() -> String {
    format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    )
}

/// The stored Local-API config, for the Settings panel.
#[tauri::command]
pub async fn local_api_get_config(
    state: State<'_, AppState>,
) -> Result<LocalApiConfig, CommandError> {
    load_config(&state.db, &state.secrets).map_err(CommandError::operation)
}

/// Persist the Local-API config. A change takes effect on the next app launch — the server is
/// bound once at startup, which keeps the socket single-owner and the lifecycle simple. The
/// UI says as much. Enabling without a token mints one, so an open (tokenless) server can
/// never be started by accident.
#[tauri::command]
pub async fn local_api_set_config(
    state: State<'_, AppState>,
    config: LocalApiConfig,
) -> Result<LocalApiConfig, CommandError> {
    let _admission = state.ensure_accepting_work()?;
    let mut next = config;
    if next.port == 0 {
        next.port = DEFAULT_PORT;
    }
    if next.enabled && next.token.trim().is_empty() {
        next.token = generate_token();
    }
    save_config(&state.db, &state.secrets, &next)
        .map_err(|error| CommandError::operation(error.to_string()))?;
    Ok(next)
}

// ---------------------------------------------------------------------------
// Pure request handling (unit-tested below)
// ---------------------------------------------------------------------------

/// The request line + the headers this server cares about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Head {
    pub method: String,
    pub path: String,
    pub bearer: Option<String>,
    pub content_length: usize,
}

/// Parse the request line and headers from the text before the blank line.
///
/// Returns `None` on a malformed start line (anything that is not `METHOD /path VERSION`).
/// Header names match case-insensitively; the query string is dropped from the path so
/// routing sees `/v1/tap`, not `/v1/tap?x=1`.
pub fn parse_head(text: &str) -> Option<Head> {
    let mut lines = text.split("\r\n");
    let start = lines.next()?;
    let mut parts = start.split(' ');
    let method = parts.next()?.to_string();
    let raw_path = parts.next()?;
    parts.next()?; // require a version token, so a random line is not accepted as a request
    if method.is_empty() || !raw_path.starts_with('/') {
        return None;
    }
    let path = raw_path.split('?').next().unwrap_or(raw_path).to_string();

    let mut bearer = None;
    let mut content_length = 0usize;
    for line in lines {
        if line.is_empty() {
            break;
        }
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        let value = value.trim();
        match name.trim().to_ascii_lowercase().as_str() {
            "authorization" => {
                // Accept "Bearer <t>" in any case for the scheme word.
                bearer = value
                    .split_once(' ')
                    .filter(|(scheme, _)| scheme.eq_ignore_ascii_case("bearer"))
                    .map(|(_, t)| t.trim().to_string());
            }
            "content-length" => content_length = value.parse().unwrap_or(0),
            _ => {}
        }
    }
    Some(Head {
        method,
        path,
        bearer,
        content_length,
    })
}

/// An error mapped to an HTTP status.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiError {
    pub status: u16,
    pub message: String,
}

impl ApiError {
    fn new(status: u16, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
        }
    }
}

/// One fleet gesture the API can perform on a device.
#[derive(Debug, Clone, PartialEq)]
pub enum Gesture {
    Tap {
        x: f64,
        y: f64,
    },
    Swipe {
        from: (f64, f64),
        to: (f64, f64),
        duration_ms: u64,
    },
    Key(HardwareKey),
    Home,
    Text(String),
    Lock(bool),
}

/// A routed request: either a read (device list) or an action on one device.
#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    ListDevices,
    Act { udid: String, gesture: Gesture },
}

/// Map method + path + JSON body to a [`Command`]. Pure; the executor performs it.
pub fn route(method: &str, path: &str, body: &Value) -> Result<Command, ApiError> {
    let udid = || {
        body.get("udid")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .ok_or_else(|| ApiError::new(400, "thiếu \"udid\""))
    };
    let num = |key: &str| {
        body.get(key)
            .and_then(Value::as_f64)
            .ok_or_else(|| ApiError::new(400, format!("thiếu số \"{key}\"")))
    };

    match (method, path) {
        ("GET", "/v1/devices") => Ok(Command::ListDevices),
        ("POST", "/v1/tap") => Ok(Command::Act {
            udid: udid()?,
            gesture: Gesture::Tap {
                x: num("x")?,
                y: num("y")?,
            },
        }),
        ("POST", "/v1/swipe") => Ok(Command::Act {
            udid: udid()?,
            gesture: Gesture::Swipe {
                from: (num("x1")?, num("y1")?),
                to: (num("x2")?, num("y2")?),
                duration_ms: body
                    .get("durationMs")
                    .and_then(Value::as_u64)
                    .unwrap_or(300),
            },
        }),
        ("POST", "/v1/key") => {
            let key: HardwareKey =
                serde_json::from_value(body.get("key").cloned().unwrap_or(Value::Null))
                    .map_err(|_| ApiError::new(400, "\"key\" không hợp lệ"))?;
            Ok(Command::Act {
                udid: udid()?,
                gesture: Gesture::Key(key),
            })
        }
        ("POST", "/v1/home") => Ok(Command::Act {
            udid: udid()?,
            gesture: Gesture::Home,
        }),
        ("POST", "/v1/text") => {
            let text = body
                .get("text")
                .and_then(Value::as_str)
                .ok_or_else(|| ApiError::new(400, "thiếu \"text\""))?;
            Ok(Command::Act {
                udid: udid()?,
                gesture: Gesture::Text(text.to_string()),
            })
        }
        ("POST", "/v1/lock") => Ok(Command::Act {
            udid: udid()?,
            gesture: Gesture::Lock(body.get("locked").and_then(Value::as_bool).unwrap_or(true)),
        }),
        ("GET", _) | ("POST", _) => Err(ApiError::new(404, "không có route")),
        _ => Err(ApiError::new(405, "method không hỗ trợ")),
    }
}

/// Bearer check in constant time. An empty configured token denies everything — the server
/// should never run without one, and this is the belt to that suspenders.
pub fn authorized(head: &Head, token: &str) -> bool {
    if token.is_empty() {
        return false;
    }
    match &head.bearer {
        Some(got) => bytes_eq_ct(got.as_bytes(), token.as_bytes()),
        None => false,
    }
}

/// Length-then-content compare that does not short-circuit on the first differing byte, so a
/// caller cannot time its way to the token byte by byte.
fn bytes_eq_ct(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b) {
        diff |= x ^ y;
    }
    diff == 0
}

/// Render an HTTP/1.1 response with a JSON body and `Connection: close`.
pub fn render_response(status: u16, body: &Value) -> Vec<u8> {
    let reason = match status {
        200 => "OK",
        400 => "Bad Request",
        401 => "Unauthorized",
        404 => "Not Found",
        405 => "Method Not Allowed",
        409 => "Conflict",
        500 => "Internal Server Error",
        503 => "Service Unavailable",
        _ => "OK",
    };
    let payload = serde_json::to_vec(body).unwrap_or_else(|_| b"{}".to_vec());
    let mut out = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        payload.len()
    )
    .into_bytes();
    out.extend_from_slice(&payload);
    out
}

/// First index of `needle` in `haystack`, or `None`.
fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack.windows(needle.len()).position(|w| w == needle)
}

// ---------------------------------------------------------------------------
// I/O shell (loopback server) — thin, delegates every decision to the above
// ---------------------------------------------------------------------------

/// Bind `127.0.0.1:port` and serve until the process exits. Loopback only — binding anything
/// routable is the one thing this must never do, and it is why the address is a literal.
pub async fn serve(app: AppHandle, port: u16, token: String) -> anyhow::Result<()> {
    let listener = TcpListener::bind(("127.0.0.1", port)).await?;
    app.state::<AppState>().local_api_runtime.write().running = Some(true);
    log::info!("local API listening on 127.0.0.1:{port}");
    let slots = Arc::new(Semaphore::new(MAX_CONCURRENT_CONNECTIONS));
    // The semaphore below bounds how many connections are *handled* at once; it does nothing
    // about an `accept` that fails before there is a connection to handle. See
    // `crate::accept_loop`.
    let mut failures = crate::accept_loop::AcceptFailures::default();
    loop {
        let (stream, _peer) = match listener.accept().await {
            Ok(pair) => {
                failures.succeeded();
                pair
            }
            Err(error) => {
                let retry = failures.failed();
                if retry.report {
                    log::warn!(
                        "local API accept failed ({} in a row): {error}",
                        failures.consecutive()
                    );
                }
                if !retry.delay.is_zero() {
                    tokio::time::sleep(retry.delay).await;
                }
                continue;
            }
        };
        // Taken before the spawn, so a flood waits in the accept loop instead of turning into
        // an unbounded pile of tasks. The semaphore lives as long as the server, so `acquire`
        // can only fail if it were closed — which it never is.
        let Ok(slot) = Arc::clone(&slots).acquire_owned().await else {
            log::warn!("local API connection semaphore closed; refusing new connections");
            return Ok(());
        };
        let app = app.clone();
        let token = token.clone();
        tokio::spawn(async move {
            let _slot = slot;
            if let Err(error) = handle_conn(stream, &token, |command| execute(&app, command)).await
            {
                log::debug!("local API connection ended: {error}");
            }
        });
    }
}

async fn handle_conn<S, F, Fut>(mut stream: S, token: &str, execute: F) -> anyhow::Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin,
    F: FnOnce(Command) -> Fut,
    Fut: std::future::Future<Output = Result<Value, ApiError>>,
{
    let request = match timeout(REQUEST_READ_TIMEOUT, read_command(&mut stream, token)).await {
        Ok(request) => request?,
        Err(_) => {
            log::debug!("local API connection timed out before a full request");
            return Ok(());
        }
    };
    let Some(command) = request else {
        return Ok(());
    };
    // AGENTS.md §2.5: once admitted, device work must finish on its own request deadline.
    // A slow client or disconnected response socket must not cancel an in-flight gesture.
    let result = match command {
        Ok(command) => execute(command).await,
        Err(error) => Err(error),
    };
    let response = async {
        match result {
            Ok(payload) => {
                write_response(&mut stream, 200, &json!({"ok": true, "result": payload})).await
            }
            Err(error) => {
                write_response(
                    &mut stream,
                    error.status,
                    &json!({"ok": false, "error": error.message}),
                )
                .await
            }
        }
    };
    timeout(RESPONSE_WRITE_TIMEOUT, response).await?
}

async fn read_command<S: AsyncRead + Unpin>(
    stream: &mut S,
    token: &str,
) -> anyhow::Result<Option<Result<Command, ApiError>>> {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];

    // 1) Read up to and including the header terminator.
    let header_end = loop {
        if let Some(pos) = find_subslice(&buf, b"\r\n\r\n") {
            break pos + 4;
        }
        if buf.len() > MAX_REQUEST_BYTES {
            return Ok(Some(Err(ApiError::new(400, "request quá lớn"))));
        }
        let n = stream.read(&mut chunk).await?;
        if n == 0 {
            return Ok(None); // client vanished before sending a full request
        }
        buf.extend_from_slice(&chunk[..n]);
    };

    let Some(head) = parse_head(&String::from_utf8_lossy(&buf[..header_end])) else {
        return Ok(Some(Err(ApiError::new(400, "request lỗi định dạng"))));
    };

    // 2) Read the declared body.
    let mut body = buf[header_end..].to_vec();
    while body.len() < head.content_length {
        if body.len() > MAX_REQUEST_BYTES {
            return Ok(Some(Err(ApiError::new(400, "body quá lớn"))));
        }
        let n = stream.read(&mut chunk).await?;
        if n == 0 {
            break;
        }
        body.extend_from_slice(&chunk[..n]);
    }

    // 3) Auth before anything is parsed or done.
    if !authorized(&head, token) {
        return Ok(Some(Err(ApiError::new(401, "unauthorized"))));
    }

    // 4) Body JSON (empty body is an empty object, for GETs and no-arg POSTs).
    let value: Value = if body.is_empty() {
        json!({})
    } else {
        match serde_json::from_slice(&body) {
            Ok(v) => v,
            Err(_) => {
                return Ok(Some(Err(ApiError::new(400, "JSON không hợp lệ"))));
            }
        }
    };

    Ok(Some(route(&head.method, &head.path, &value)))
}

async fn write_response<S: AsyncWrite + Unpin>(
    stream: &mut S,
    status: u16,
    body: &Value,
) -> anyhow::Result<()> {
    stream.write_all(&render_response(status, body)).await?;
    stream.flush().await?;
    Ok(())
}

async fn execute(app: &AppHandle, command: Command) -> Result<Value, ApiError> {
    let state = app.state::<AppState>();
    // Honour app admission so the API stops taking work the moment shutdown begins. Holding
    // the guard for the request duration mirrors what every mutating command does.
    let _admission = state
        .ensure_accepting_work()
        .map_err(|_| ApiError::new(503, "app đang tắt hoặc chưa sẵn sàng"))?;

    match command {
        Command::ListDevices => {
            Ok(serde_json::to_value(state.registry.list()).unwrap_or(Value::Null))
        }
        Command::Act { udid, gesture } => run_gesture(&state, &udid, gesture)
            .await
            .map(|()| json!({})),
    }
}

async fn run_gesture(state: &AppState, udid: &str, gesture: Gesture) -> Result<(), ApiError> {
    crate::commands::with_manual_session(
        state,
        udid,
        DeviceWorkOwner::ManualControl,
        move |session| async move {
            match gesture {
                Gesture::Tap { x, y } => session.tap(TapPoint { x, y }).await,
                Gesture::Swipe {
                    from,
                    to,
                    duration_ms,
                } => {
                    session
                        .swipe(SwipeGesture {
                            from: TapPoint {
                                x: from.0,
                                y: from.1,
                            },
                            to: TapPoint { x: to.0, y: to.1 },
                            duration_ms,
                        })
                        .await
                }
                Gesture::Key(key) => session.press_hardware_key(key).await,
                Gesture::Home => session.home().await,
                Gesture::Text(text) => session.type_text(&text).await,
                Gesture::Lock(locked) => session.set_locked(locked).await,
            }
        },
    )
    .await
    .map_err(|error| {
        // A busy device is a 409 the caller can retry; everything else is a 500.
        let status = if error.code == "DeviceBusy" { 409 } else { 500 };
        ApiError::new(status, error.message.to_string())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};

    #[derive(Default)]
    struct TestCredentials {
        value: parking_lot::Mutex<Option<String>>,
        fail_write: AtomicBool,
        fail_read: AtomicBool,
        ignore_write: AtomicBool,
    }

    impl riviu_signing::CredentialBackend for TestCredentials {
        fn get(&self, _: &str) -> anyhow::Result<Option<String>> {
            anyhow::ensure!(
                !self.fail_read.load(Ordering::Relaxed),
                "fixture read error"
            );
            Ok(self.value.lock().clone())
        }

        fn set(&self, _: &str, value: &str) -> anyhow::Result<()> {
            anyhow::ensure!(
                !self.fail_write.load(Ordering::Relaxed),
                "fixture write error"
            );
            if !self.ignore_write.load(Ordering::Relaxed) {
                *self.value.lock() = Some(value.to_string());
            }
            Ok(())
        }

        fn delete(&self, _: &str) -> anyhow::Result<()> {
            *self.value.lock() = None;
            Ok(())
        }
    }

    struct ConfigFixture {
        db: Database,
        root: std::path::PathBuf,
        backend: Arc<TestCredentials>,
        secrets: CredentialStore,
    }

    impl ConfigFixture {
        fn legacy() -> Self {
            let root =
                std::env::temp_dir().join(format!("riviu-local-api-{}", uuid::Uuid::new_v4()));
            let db = Database::open(root.join("settings.db")).unwrap();
            db.set_setting(
                CONFIG_KEY,
                r#"{"enabled":true,"port":22222,"token":"fixture-token"}"#,
            )
            .unwrap();
            let backend = Arc::new(TestCredentials::default());
            let secrets = CredentialStore::new(backend.clone());
            Self {
                db,
                root,
                backend,
                secrets,
            }
        }

        fn stored(&self) -> LocalApiConfig {
            serde_json::from_str(&self.db.get_setting(CONFIG_KEY).unwrap().unwrap()).unwrap()
        }
    }

    impl Drop for ConfigFixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    fn legacy_token_survives_failed_keyring_write_and_migrates_on_retry() {
        let fixture = ConfigFixture::legacy();
        fixture.backend.fail_write.store(true, Ordering::Relaxed);
        assert!(load_config(&fixture.db, &fixture.secrets).is_err());
        assert_eq!(fixture.stored().token, "fixture-token");
        assert_eq!(*fixture.backend.value.lock(), None);
        fixture.backend.fail_write.store(false, Ordering::Relaxed);
        let migrated = load_config(&fixture.db, &fixture.secrets).unwrap();
        assert_eq!(migrated.token, "fixture-token");
        assert!(fixture.stored().token.is_empty());
        assert_eq!(
            load_config(&fixture.db, &fixture.secrets).unwrap(),
            migrated
        );
    }

    #[test]
    fn legacy_token_is_retained_until_the_keyring_read_confirms_it() {
        let fixture = ConfigFixture::legacy();
        fixture.backend.ignore_write.store(true, Ordering::Relaxed);
        assert!(load_config(&fixture.db, &fixture.secrets).is_err());
        assert_eq!(fixture.stored().token, "fixture-token");
        fixture.backend.ignore_write.store(false, Ordering::Relaxed);
        fixture.backend.fail_read.store(true, Ordering::Relaxed);
        assert!(load_config(&fixture.db, &fixture.secrets).is_err());
        assert_eq!(fixture.stored().token, "fixture-token");
        fixture.backend.fail_read.store(false, Ordering::Relaxed);
        assert_eq!(
            load_config(&fixture.db, &fixture.secrets).unwrap().token,
            "fixture-token"
        );
        assert!(fixture.stored().token.is_empty());
    }

    #[test]
    fn config_read_errors_are_not_reported_as_successful_disabled_settings() {
        let fixture = ConfigFixture::legacy();
        fixture.db.set_setting(CONFIG_KEY, "not JSON").unwrap();
        assert!(load_config(&fixture.db, &fixture.secrets).is_err());
        fixture
            .db
            .set_setting(CONFIG_KEY, r#"{"enabled":true,"port":22222,"token":""}"#)
            .unwrap();
        fixture.backend.fail_read.store(true, Ordering::Relaxed);
        assert!(load_config(&fixture.db, &fixture.secrets).is_err());
    }

    const VALID_REQUEST: &[u8] =
        b"GET /v1/devices HTTP/1.1\r\nAuthorization: Bearer fixture-token\r\n\r\n";

    #[tokio::test(start_paused = true)]
    async fn slow_request_read_times_out_without_dispatch() {
        let (mut client, server) = tokio::io::duplex(1024);
        client
            .write_all(b"GET /v1/devices HTTP/1.1\r\n")
            .await
            .unwrap();
        let called = Arc::new(AtomicBool::new(false));
        let dispatched = called.clone();
        let task = tokio::spawn(handle_conn(server, "fixture-token", move |_| async move {
            dispatched.store(true, Ordering::Relaxed);
            Ok(json!({}))
        }));
        tokio::task::yield_now().await;
        tokio::time::advance(REQUEST_READ_TIMEOUT + Duration::from_secs(1)).await;
        task.await.unwrap().unwrap();
        assert!(!called.load(Ordering::Relaxed));
    }

    #[tokio::test(start_paused = true)]
    async fn admitted_operation_outlives_read_deadline_and_finishes_before_response() {
        let (mut client, server) = tokio::io::duplex(1024);
        client.write_all(VALID_REQUEST).await.unwrap();
        let settled = Arc::new(AtomicBool::new(false));
        let completed = settled.clone();
        let task = tokio::spawn(handle_conn(server, "fixture-token", move |_| async move {
            tokio::time::sleep(REQUEST_READ_TIMEOUT * 2).await;
            completed.store(true, Ordering::Relaxed);
            Ok(json!({"finished": true}))
        }));
        tokio::task::yield_now().await;
        tokio::time::advance(REQUEST_READ_TIMEOUT + Duration::from_secs(1)).await;
        assert!(!task.is_finished());
        assert!(!settled.load(Ordering::Relaxed));
        tokio::time::advance(REQUEST_READ_TIMEOUT).await;
        task.await.unwrap().unwrap();
        let mut response = String::new();
        client.read_to_string(&mut response).await.unwrap();
        assert!(settled.load(Ordering::Relaxed));
        assert!(response.starts_with("HTTP/1.1 200 OK"));
        assert!(response.contains(r#""finished":true"#));
    }

    #[tokio::test(start_paused = true)]
    async fn disconnected_client_does_not_cancel_admitted_operation() {
        let (mut client, server) = tokio::io::duplex(1024);
        client.write_all(VALID_REQUEST).await.unwrap();
        let settled = Arc::new(AtomicBool::new(false));
        let completed = settled.clone();
        let task = tokio::spawn(handle_conn(server, "fixture-token", move |_| async move {
            tokio::time::sleep(REQUEST_READ_TIMEOUT * 2).await;
            completed.store(true, Ordering::Relaxed);
            Ok(json!({}))
        }));
        tokio::task::yield_now().await;
        drop(client);
        tokio::time::advance(REQUEST_READ_TIMEOUT * 2).await;
        assert!(task.await.unwrap().is_err());
        assert!(settled.load(Ordering::Relaxed));
    }

    #[tokio::test(start_paused = true)]
    async fn unread_response_times_out_only_after_operation_settles() {
        let (mut client, server) = tokio::io::duplex(128);
        client.write_all(VALID_REQUEST).await.unwrap();
        let task = tokio::spawn(handle_conn(server, "fixture-token", |_| async {
            Ok(json!({"body": "x".repeat(1024)}))
        }));
        tokio::task::yield_now().await;
        tokio::time::advance(RESPONSE_WRITE_TIMEOUT + Duration::from_secs(1)).await;
        assert!(task.await.unwrap().is_err());
    }

    fn head(raw: &str) -> Head {
        parse_head(raw).expect("well-formed request")
    }

    #[test]
    fn parses_request_line_headers_and_strips_query() {
        let h = head("POST /v1/tap?debug=1 HTTP/1.1\r\nHost: x\r\nAuthorization: Bearer abc123\r\nContent-Length: 7\r\n\r\n");
        assert_eq!(h.method, "POST");
        assert_eq!(h.path, "/v1/tap");
        assert_eq!(h.bearer.as_deref(), Some("abc123"));
        assert_eq!(h.content_length, 7);
    }

    #[test]
    fn bearer_scheme_word_is_case_insensitive_but_token_is_not() {
        assert_eq!(
            head("GET /v1/devices HTTP/1.1\r\nauthorization: bearer T\r\n\r\n")
                .bearer
                .as_deref(),
            Some("T")
        );
    }

    #[test]
    fn rejects_a_line_that_is_not_a_request() {
        assert!(parse_head("this is not http\r\n\r\n").is_none());
        assert!(parse_head("GET only-two-tokens\r\n\r\n").is_none());
        assert!(parse_head("GET relative HTTP/1.1\r\n\r\n").is_none());
    }

    #[test]
    fn routes_the_gesture_endpoints() {
        assert_eq!(
            route("GET", "/v1/devices", &json!({})),
            Ok(Command::ListDevices)
        );
        assert_eq!(
            route(
                "POST",
                "/v1/tap",
                &json!({"udid": "A", "x": 10.0, "y": 20.0})
            ),
            Ok(Command::Act {
                udid: "A".into(),
                gesture: Gesture::Tap { x: 10.0, y: 20.0 }
            })
        );
        assert_eq!(
            route("POST", "/v1/lock", &json!({"udid": "A", "locked": false})),
            Ok(Command::Act {
                udid: "A".into(),
                gesture: Gesture::Lock(false)
            })
        );
        // durationMs defaults when omitted.
        assert_eq!(
            route(
                "POST",
                "/v1/swipe",
                &json!({"udid":"A","x1":0.0,"y1":0.0,"x2":5.0,"y2":9.0})
            ),
            Ok(Command::Act {
                udid: "A".into(),
                gesture: Gesture::Swipe {
                    from: (0.0, 0.0),
                    to: (5.0, 9.0),
                    duration_ms: 300
                }
            })
        );
    }

    #[test]
    fn key_endpoint_deserialises_the_camelcase_hardware_key() {
        assert_eq!(
            route("POST", "/v1/key", &json!({"udid":"A","key":"volumeUp"})),
            Ok(Command::Act {
                udid: "A".into(),
                gesture: Gesture::Key(HardwareKey::VolumeUp)
            })
        );
        assert_eq!(
            route("POST", "/v1/key", &json!({"udid":"A","key":"nope"}))
                .unwrap_err()
                .status,
            400
        );
    }

    #[test]
    fn missing_fields_are_400_and_unknown_paths_are_404() {
        assert_eq!(
            route("POST", "/v1/tap", &json!({"x":1.0,"y":2.0}))
                .unwrap_err()
                .status,
            400
        );
        assert_eq!(
            route("POST", "/v1/tap", &json!({"udid":"A","x":1.0}))
                .unwrap_err()
                .status,
            400
        );
        assert_eq!(
            route("POST", "/v1/nope", &json!({})).unwrap_err().status,
            404
        );
        assert_eq!(
            route("DELETE", "/v1/tap", &json!({})).unwrap_err().status,
            405
        );
    }

    #[test]
    fn authorization_requires_the_exact_token_and_a_nonempty_config() {
        let with = |b: Option<&str>| Head {
            method: "GET".into(),
            path: "/v1/devices".into(),
            bearer: b.map(str::to_string),
            content_length: 0,
        };
        assert!(authorized(&with(Some("secret")), "secret"));
        assert!(!authorized(&with(Some("secret")), "other"));
        assert!(!authorized(&with(None), "secret"));
        // A server with no token configured denies everything, even a blank bearer.
        assert!(!authorized(&with(Some("")), ""));
    }

    #[test]
    fn response_carries_status_reason_and_json_body() {
        let bytes = render_response(401, &json!({"ok": false, "error": "unauthorized"}));
        let text = String::from_utf8(bytes).unwrap();
        assert!(text.starts_with("HTTP/1.1 401 Unauthorized\r\n"));
        assert!(text.contains("Content-Type: application/json\r\n"));
        assert!(text.contains("Connection: close\r\n"));
        assert!(text.ends_with("{\"error\":\"unauthorized\",\"ok\":false}"));
    }

    #[test]
    fn generated_tokens_are_long_and_distinct() {
        let a = generate_token();
        let b = generate_token();
        assert_eq!(a.len(), 64);
        assert_ne!(a, b);
    }

    #[test]
    fn default_config_is_off_on_the_xiaowei_port() {
        let c = LocalApiConfig::default();
        assert!(!c.enabled);
        assert_eq!(c.port, 22222);
        assert!(c.token.is_empty());
    }

    #[test]
    fn status_keeps_running_server_distinct_from_disabled_configuration() {
        let startup = LocalApiConfig {
            enabled: true,
            port: 22222,
            token: "fixture-secret".into(),
        };
        let runtime = LocalApiRuntime {
            startup: Some(startup.clone()),
            running: Some(true),
            last_error: None,
        };
        let configured = LocalApiConfig {
            enabled: false,
            ..startup.clone()
        };
        let status = runtime.status(&configured);
        assert_eq!(status.running, Some(true));
        assert_eq!(status.active_port, Some(22222));
        assert!(status.restart_required);
        assert!(!status.configured_enabled);
        assert!(!serde_json::to_string(&status)
            .unwrap()
            .contains("fixture-secret"));
        assert!(!runtime.status(&startup).restart_required);
    }

    #[test]
    fn status_distinguishes_unstarted_and_failed_listener() {
        let configured = LocalApiConfig::default();
        assert_eq!(LocalApiRuntime::default().status(&configured).running, None);
        let runtime = LocalApiRuntime {
            startup: Some(configured.clone()),
            running: Some(false),
            last_error: Some("port occupied".into()),
        };
        let status = runtime.status(&configured);
        assert_eq!(status.active_port, None);
        assert_eq!(status.last_error.as_deref(), Some("port occupied"));
    }

    #[test]
    fn repaired_configuration_requires_restart_after_startup_read_failure() {
        let runtime = LocalApiRuntime {
            startup: None,
            running: Some(false),
            last_error: Some("credential read failed".into()),
        };
        let config = LocalApiConfig {
            enabled: true,
            token: "fixture-token".into(),
            ..LocalApiConfig::default()
        };
        let status = runtime.status(&config);
        assert!(status.restart_required);
        assert_eq!(status.active_port, None);
        assert_eq!(status.running, Some(false));
    }

    #[test]
    fn finds_the_header_terminator() {
        assert_eq!(find_subslice(b"abc\r\n\r\nbody", b"\r\n\r\n"), Some(3));
        assert_eq!(find_subslice(b"no terminator", b"\r\n\r\n"), None);
    }
}
