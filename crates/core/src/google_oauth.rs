//! Google desktop OAuth with PKCE, bounded loopback callbacks and a Sheets Picker.
//! Tokens belong in the caller's credential store, never logs or SQLite settings.
use anyhow::{Context, Result};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use rand::{rngs::OsRng, RngCore};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fmt,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};
use tokio::{net::TcpListener, sync::Notify};

mod loopback;
mod picker;
pub use picker::{GooglePickerFile, GooglePickerSession};
#[cfg(test)]
mod tests;

const AUTHORIZATION_ENDPOINT: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const TOKEN_ENDPOINT: &str = "https://oauth2.googleapis.com/token";
const USERINFO_ENDPOINT: &str = "https://openidconnect.googleapis.com/v1/userinfo";
pub const GOOGLE_SHEETS_SCOPES: &str = "openid email https://www.googleapis.com/auth/spreadsheets";
const SHEETS_SCOPE: &str = "https://www.googleapis.com/auth/spreadsheets";
const LEGACY_FILE_SCOPE: &str = "https://www.googleapis.com/auth/drive.file";
const SESSION_TIMEOUT: Duration = Duration::from_secs(300);
const BODY_LIMIT: usize = 64 * 1024;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum GoogleOAuthErrorCode {
    Configuration,
    Cancelled,
    Denied,
    Timeout,
    ReconnectRequired,
    Transient,
    InvalidResponse,
}
#[derive(Debug, Clone, thiserror::Error)]
#[error("{message}")]
pub struct GoogleOAuthError {
    pub code: GoogleOAuthErrorCode,
    pub message: String,
}
impl GoogleOAuthError {
    pub fn requires_reconnect(&self) -> bool {
        self.code == GoogleOAuthErrorCode::ReconnectRequired
    }
    pub fn is_retryable(&self) -> bool {
        matches!(
            self.code,
            GoogleOAuthErrorCode::Transient | GoogleOAuthErrorCode::Timeout
        )
    }
}
fn oauth_error(code: GoogleOAuthErrorCode, message: impl Into<String>) -> anyhow::Error {
    GoogleOAuthError {
        code,
        message: message.into(),
    }
    .into()
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GoogleOAuthClientConfig {
    pub client_id: String,
    #[serde(default)]
    pub client_secret: Option<String>,
    #[serde(default)]
    pub picker_api_key: Option<String>,
    #[serde(default)]
    pub project_number: Option<String>,
}
impl fmt::Debug for GoogleOAuthClientConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GoogleOAuthClientConfig")
            .field("client_id", &self.client_id)
            .field(
                "client_secret",
                &self.client_secret.as_ref().map(|_| "[redacted]"),
            )
            .field(
                "picker_api_key",
                &self.picker_api_key.as_ref().map(|_| "[redacted]"),
            )
            .field("project_number", &self.project_number)
            .finish()
    }
}
impl GoogleOAuthClientConfig {
    pub fn validate(&self) -> Result<()> {
        let prefix = self
            .client_id
            .strip_suffix(".apps.googleusercontent.com")
            .context("Google OAuth client ID phải có đuôi .apps.googleusercontent.com")?;
        anyhow::ensure!(
            !prefix.is_empty()
                && self.client_id.len() <= 256
                && prefix
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_')),
            "Google OAuth client ID không hợp lệ"
        );
        for (value, name) in [
            (&self.client_secret, "client secret"),
            (&self.picker_api_key, "Picker API key"),
        ] {
            if let Some(value) = value {
                anyhow::ensure!(
                    !value.is_empty()
                        && value.len() <= 4096
                        && !value.chars().any(char::is_control),
                    "Google {name} không hợp lệ"
                );
            }
        }
        if let Some(number) = &self.project_number {
            anyhow::ensure!(
                !number.is_empty()
                    && number.len() <= 32
                    && number.bytes().all(|c| c.is_ascii_digit()),
                "Google project number phải là số"
            );
        }
        Ok(())
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GoogleOAuthTokens {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_at_ms: i64,
    pub scope: String,
    pub account_id: String,
    pub email: String,
}
impl fmt::Debug for GoogleOAuthTokens {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GoogleOAuthTokens")
            .field("access_token", &"[redacted]")
            .field("refresh_token", &"[redacted]")
            .field("expires_at_ms", &self.expires_at_ms)
            .field("scope", &self.scope)
            .field("account_id", &"[redacted]")
            .field("email", &"[redacted]")
            .finish()
    }
}
impl GoogleOAuthTokens {
    /// Scope is not file permission: callers must still prove edit access at the target.
    pub fn has_sheets_scope(&self) -> bool {
        self.scope
            .split_whitespace()
            .any(|scope| scope == SHEETS_SCOPE)
    }
    pub fn needs_refresh(&self) -> bool {
        self.expires_at_ms <= chrono::Utc::now().timestamp_millis().saturating_add(60_000)
    }
}

#[derive(Default)]
struct CancelState {
    cancelled: AtomicBool,
    changed: Notify,
}
#[derive(Clone, Default)]
pub struct GoogleSessionCancel(Arc<CancelState>);
impl GoogleSessionCancel {
    pub fn cancel(&self) {
        self.0.cancelled.store(true, Ordering::Release);
        self.0.changed.notify_waiters();
    }
    pub fn is_cancelled(&self) -> bool {
        self.0.cancelled.load(Ordering::Acquire)
    }
    async fn cancelled(&self) {
        loop {
            let notified = self.0.changed.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if self.is_cancelled() {
                return;
            }
            notified.await;
        }
    }
}
impl fmt::Debug for GoogleSessionCancel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GoogleSessionCancel")
            .field("cancelled", &self.is_cancelled())
            .finish()
    }
}

#[derive(Clone, Debug)]
pub struct GoogleOAuthClient {
    config: GoogleOAuthClientConfig,
    http: reqwest::Client,
}
impl GoogleOAuthClient {
    pub fn new(config: GoogleOAuthClientConfig) -> Result<Self> {
        config
            .validate()
            .map_err(|error| oauth_error(GoogleOAuthErrorCode::Configuration, error.to_string()))?;
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(20))
            .connect_timeout(Duration::from_secs(7))
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .build()
            .context("Khởi tạo kết nối Google thất bại")?;
        Ok(Self { config, http })
    }
    pub fn config(&self) -> &GoogleOAuthClientConfig {
        &self.config
    }
    pub async fn authorization_session(&self) -> Result<GoogleAuthorizationSession> {
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .context("Mở cổng đăng nhập Google cục bộ thất bại")?;
        let origin = format!("http://{}", listener.local_addr()?);
        let redirect_uri = format!("{origin}/oauth/callback");
        let state = random_secret();
        let verifier = random_secret();
        let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
        let mut authorization_url = url::Url::parse(AUTHORIZATION_ENDPOINT)?;
        authorization_url.query_pairs_mut().extend_pairs([
            ("client_id", self.config.client_id.as_str()),
            ("redirect_uri", redirect_uri.as_str()),
            ("response_type", "code"),
            ("scope", GOOGLE_SHEETS_SCOPES),
            ("state", state.as_str()),
            ("code_challenge", challenge.as_str()),
            ("code_challenge_method", "S256"),
            ("access_type", "offline"),
            ("prompt", "consent select_account"),
        ]);
        Ok(GoogleAuthorizationSession {
            client: self.clone(),
            listener,
            authorization_url: authorization_url.to_string(),
            redirect_uri,
            state,
            verifier,
            cancel: GoogleSessionCancel::default(),
            deadline: tokio::time::Instant::now() + SESSION_TIMEOUT,
        })
    }
    async fn exchange(
        &self,
        code: &str,
        verifier: &str,
        redirect_uri: &str,
    ) -> Result<GoogleOAuthTokens> {
        let mut form = vec![
            ("client_id", self.config.client_id.as_str()),
            ("code", code),
            ("code_verifier", verifier),
            ("redirect_uri", redirect_uri),
            ("grant_type", "authorization_code"),
        ];
        if let Some(secret) = &self.config.client_secret {
            form.push(("client_secret", secret));
        }
        let response = self
            .http
            .post(TOKEN_ENDPOINT)
            .form(&form)
            .send()
            .await
            .map_err(|_| {
                oauth_error(
                    GoogleOAuthErrorCode::Transient,
                    "Đổi mã đăng nhập Google thất bại",
                )
            })?;
        let token: TokenResponse = decode_response(response, "đổi mã đăng nhập").await?;
        let mut tokens = token.into_tokens(None)?;
        self.populate_identity(&mut tokens, None).await?;
        Ok(tokens)
    }
    pub async fn refresh(&self, previous: &GoogleOAuthTokens) -> Result<GoogleOAuthTokens> {
        if previous.refresh_token.is_empty() {
            return Err(oauth_error(
                GoogleOAuthErrorCode::ReconnectRequired,
                "Cần đăng nhập Google lại để nhận refresh token",
            ));
        }
        let mut form = vec![
            ("client_id", self.config.client_id.as_str()),
            ("refresh_token", previous.refresh_token.as_str()),
            ("grant_type", "refresh_token"),
        ];
        if let Some(secret) = &self.config.client_secret {
            form.push(("client_secret", secret));
        }
        let response = self
            .http
            .post(TOKEN_ENDPOINT)
            .form(&form)
            .send()
            .await
            .map_err(|_| {
                oauth_error(
                    GoogleOAuthErrorCode::Transient,
                    "Làm mới đăng nhập Google thất bại",
                )
            })?;
        let token: TokenResponse = decode_response(response, "làm mới đăng nhập").await?;
        let mut tokens = token.into_tokens(Some(previous))?;
        self.populate_identity(&mut tokens, Some(&previous.account_id))
            .await?;
        Ok(tokens)
    }
    async fn populate_identity(
        &self,
        tokens: &mut GoogleOAuthTokens,
        expected: Option<&str>,
    ) -> Result<()> {
        let response = self
            .http
            .get(USERINFO_ENDPOINT)
            .bearer_auth(&tokens.access_token)
            .send()
            .await
            .map_err(|_| {
                oauth_error(
                    GoogleOAuthErrorCode::Transient,
                    "Đọc tài khoản Google thất bại",
                )
            })?;
        let identity: GoogleIdentity = decode_response(response, "đọc tài khoản").await?;
        identity.validate(expected)?;
        tokens.account_id = identity.sub;
        tokens.email = identity.email;
        Ok(())
    }
    pub async fn picker_session(&self, tokens: &GoogleOAuthTokens) -> Result<GooglePickerSession> {
        picker::create_session(&self.config, tokens).await
    }
}

pub struct GoogleAuthorizationSession {
    client: GoogleOAuthClient,
    listener: TcpListener,
    authorization_url: String,
    redirect_uri: String,
    state: String,
    verifier: String,
    cancel: GoogleSessionCancel,
    deadline: tokio::time::Instant,
}
impl fmt::Debug for GoogleAuthorizationSession {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GoogleAuthorizationSession")
            .field("callback", &self.redirect_uri)
            .finish_non_exhaustive()
    }
}
impl GoogleAuthorizationSession {
    pub fn authorization_url(&self) -> &str {
        &self.authorization_url
    }
    pub fn cancel_handle(&self) -> GoogleSessionCancel {
        self.cancel.clone()
    }
    pub async fn wait(self) -> Result<GoogleOAuthTokens> {
        let cancel = self.cancel.clone();
        tokio::select! {
            biased;
            _=cancel.cancelled()=>Err(oauth_error(GoogleOAuthErrorCode::Cancelled,"Đã hủy đăng nhập Google")),
            result=tokio::time::timeout_at(self.deadline,self.finish())=>result.map_err(|_|oauth_error(GoogleOAuthErrorCode::Timeout,"Hết thời gian đăng nhập Google"))?,
        }
    }
    async fn finish(self) -> Result<GoogleOAuthTokens> {
        let host = self.listener.local_addr()?.to_string();
        // Own the listener until one valid callback; repeated/foreign requests
        // receive a fixed response without consuming the OAuth state.
        let code = loopback::receive_authorization(&self.listener, &host, &self.state).await?;
        drop(self.listener);
        self.client
            .exchange(&code, &self.verifier, &self.redirect_uri)
            .await
    }
}

fn random_secret() -> String {
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    token_type: String,
    expires_in: i64,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    scope: Option<String>,
}
impl TokenResponse {
    fn into_tokens(self, prior: Option<&GoogleOAuthTokens>) -> Result<GoogleOAuthTokens> {
        anyhow::ensure!(
            self.token_type.eq_ignore_ascii_case("Bearer")
                && !self.access_token.is_empty()
                && self.access_token.len() <= 8192
                && (1..=86400).contains(&self.expires_in),
            "Google trả token không hợp lệ"
        );
        let refresh = self
            .refresh_token
            .or_else(|| prior.map(|p| p.refresh_token.clone()))
            .ok_or_else(|| {
                oauth_error(
                    GoogleOAuthErrorCode::ReconnectRequired,
                    "Google chưa cấp refresh token; đăng nhập và đồng ý lại",
                )
            })?;
        anyhow::ensure!(
            !refresh.is_empty() && refresh.len() <= 8192,
            "Google trả refresh token không hợp lệ"
        );
        let scope = self
            .scope
            .or_else(|| prior.map(|p| p.scope.clone()))
            .context("Google chưa trả các quyền đã cấp")?;
        anyhow::ensure!(
            (scope.split_whitespace().any(|s| s == SHEETS_SCOPE)
                || (prior.is_some() && scope.split_whitespace().any(|s| s == LEGACY_FILE_SCOPE)))
                && scope.split_whitespace().any(|s| s == "openid")
                && scope.split_whitespace().any(|s| matches!(
                    s,
                    "email" | "https://www.googleapis.com/auth/userinfo.email"
                )),
            "Google chưa cấp quyền Google Sheets; đăng nhập lại và chấp thuận quyền truy cập bảng tính"
        );
        Ok(GoogleOAuthTokens {
            access_token: self.access_token,
            refresh_token: refresh,
            expires_at_ms: chrono::Utc::now()
                .timestamp_millis()
                .saturating_add(self.expires_in * 1000),
            scope,
            account_id: String::new(),
            email: String::new(),
        })
    }
}
#[derive(Deserialize)]
struct GoogleIdentity {
    sub: String,
    email: String,
    #[serde(default)]
    email_verified: Option<bool>,
}
impl GoogleIdentity {
    fn validate(&self, expected: Option<&str>) -> Result<()> {
        anyhow::ensure!(
            !self.sub.trim().is_empty()
                && self.sub.len() <= 255
                && !self.email.trim().is_empty()
                && self.email.len() <= 320
                && self.email.contains('@')
                && self.email_verified != Some(false),
            "Google chưa xác nhận tài khoản email"
        );
        if expected.is_some_and(|id| id != self.sub) {
            return Err(oauth_error(
                GoogleOAuthErrorCode::ReconnectRequired,
                "Tài khoản Google thay đổi khi làm mới token",
            ));
        }
        Ok(())
    }
}
async fn decode_response<T: serde::de::DeserializeOwned>(
    mut response: reqwest::Response,
    stage: &str,
) -> Result<T> {
    let status = response.status();
    anyhow::ensure!(
        response
            .content_length()
            .is_none_or(|n| n <= BODY_LIMIT as u64),
        "Phản hồi Google quá lớn"
    );
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| {
        oauth_error(
            GoogleOAuthErrorCode::Transient,
            "Đọc phản hồi Google thất bại",
        )
    })? {
        anyhow::ensure!(
            bytes.len() + chunk.len() <= BODY_LIMIT,
            "Phản hồi Google quá lớn"
        );
        bytes.extend_from_slice(&chunk);
    }
    if !status.is_success() {
        let code = serde_json::from_slice::<serde_json::Value>(&bytes)
            .ok()
            .and_then(|v| v["error"].as_str().map(str::to_owned));
        let label = match code.as_deref() {
            Some("invalid_grant") => "phiên đã hết hạn, cần đăng nhập lại",
            Some("invalid_client") => "cấu hình OAuth chưa hợp lệ",
            Some("access_denied") => "quyền truy cập bị từ chối",
            _ => "dịch vụ chưa chấp nhận yêu cầu",
        };
        let kind = match code.as_deref() {
            Some("invalid_grant") => GoogleOAuthErrorCode::ReconnectRequired,
            Some("invalid_client") => GoogleOAuthErrorCode::Configuration,
            Some("access_denied") => GoogleOAuthErrorCode::Denied,
            _ if status.is_server_error() || status == reqwest::StatusCode::TOO_MANY_REQUESTS => {
                GoogleOAuthErrorCode::Transient
            }
            _ if status == reqwest::StatusCode::UNAUTHORIZED => {
                GoogleOAuthErrorCode::ReconnectRequired
            }
            _ => GoogleOAuthErrorCode::InvalidResponse,
        };
        return Err(oauth_error(
            kind,
            format!("Google {stage}: {label} (HTTP {})", status.as_u16()),
        ));
    }
    serde_json::from_slice(&bytes).map_err(|_| {
        oauth_error(
            GoogleOAuthErrorCode::InvalidResponse,
            format!("Google trả dữ liệu {stage} không hợp lệ"),
        )
    })
}
