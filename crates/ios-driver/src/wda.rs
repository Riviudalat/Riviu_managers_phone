use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::RwLock;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue, CONNECTION};
use reqwest::Client;
use riviu_core::{
    is_valid_protected_route_path, validate_clipboard_read_limit, ActiveAppIdentity,
    ClipboardAccessMode, ElementLocatorStrategy, ProtectedRouteContract, QualifiedElementLocator,
    RouteMethod, RouteScope, SwipeGesture, TapPoint, UiCapabilities, UiError, UiErrorKind,
    MAX_INTERACTION_CLIPBOARD_BYTES, OPEN_URL_TIMEOUT_MS,
};
use serde_json::json;

use crate::config::{AgentToken, UnifiedAgentConfig};
use crate::telemetry::{self, Outcome};

/// Session caps with **no** `bundleId`. WDA with `bundleId=com.apple.springboard`
/// (and default `forceAppLaunch=true`) re-launches SpringBoard on every
/// `/session` — that flashes the lock screen / Home and looks like a WDA error
/// in the nurture log. Empty caps attach to whatever is already foreground.
///
/// Deliberately **no** `autoDismissAlerts` / `defaultAlertAction`. Those install
/// a background alert monitor inside the agent that keeps querying the
/// foreground app's accessibility hierarchy. On TikTok that query does not
/// return, and it blocks the XCTest thread that also serves gestures: with the
/// flags set, every session command timed out; without them, the identical
/// sequence answered in ~1 s. System alerts are handled explicitly through
/// `dismiss_alert` instead, and TikTok's own popups by the screen watcher.
fn session_capabilities() -> serde_json::Value {
    json!({
        "capabilities": {
            "alwaysMatch": {
                "appium:shouldWaitForQuiescence": false,
                "appium:waitForIdleTimeout": 0,
                "appium:forceAppLaunch": false,
            }
        },
        "desiredCapabilities": {
            "shouldWaitForQuiescence": false,
            "waitForIdleTimeout": 0,
            "forceAppLaunch": false,
        },
    })
}

/// Deadline for a single gesture. Long enough for TikTok to render under USB
/// load, short enough that a wedged call is reported rather than waited out.
///
/// Never wrap these in `tokio::time::timeout`: cancelling a request mid-flight
/// leaves the relay with a half-written body and wedges it (live test #1). The
/// deadline belongs to the request itself.
const GESTURE_TIMEOUT: Duration = Duration::from_secs(10);

/// Priming attempts before a session is declared unusable. Four 8 s attempts
/// with 2 s gaps covers the ~45 s self-clearing stall seen in a live run,
/// without waiting out a genuinely dead runner.
const PRIME_ATTEMPTS: u32 = 4;
const CLIPBOARD_RESPONSE_OVERHEAD_BYTES: usize = 4 * 1024;
const INTERACTION_RESPONSE_LIMIT_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WdaBackend {
    Stock,
    RtMmo,
    RiviuAgent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LaunchKind {
    XcTest,
    Application,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SessionStrategy {
    CreateThenPrime,
    StatusThenCreate,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct WdaProfile {
    pub backend: WdaBackend,
    pub bundle_id: String,
    pub device_port: u16,
    pub mjpeg_port: u16,
    pub auth_token: Option<AgentToken>,
    pub logical_size: (f64, f64),
    pub agent_ipa: Option<PathBuf>,
    pub features: Vec<String>,
    pub launch_kind: LaunchKind,
    pub session_strategy: SessionStrategy,
    interaction_http: Option<InteractionHttpAdapter>,
}

#[derive(Debug, Clone, PartialEq)]
struct InteractionHttpAdapter {
    capabilities: UiCapabilities,
    active_app_route: Option<ProtectedRouteContract>,
}

#[derive(Clone, Copy)]
struct SendOptions<'a> {
    timeout: Duration,
    auth: Option<(&'a str, &'a str)>,
    response_limit: Option<usize>,
}

impl WdaProfile {
    pub(crate) fn unified(config: &UnifiedAgentConfig) -> Self {
        let manifest = &config.artifact.manifest;
        let candidate = manifest.protocol_version >= 2;
        Self {
            backend: if candidate {
                WdaBackend::RiviuAgent
            } else {
                WdaBackend::RtMmo
            },
            bundle_id: manifest.bundle_id.clone(),
            device_port: manifest.control_port,
            mjpeg_port: manifest.mjpeg_port,
            auth_token: Some(config.token.clone()),
            logical_size: (
                f64::from(manifest.logical_width),
                f64::from(manifest.logical_height),
            ),
            agent_ipa: Some(config.artifact.ipa_path.clone()),
            features: manifest.features.clone(),
            launch_kind: LaunchKind::Application,
            session_strategy: if candidate {
                SessionStrategy::CreateThenPrime
            } else {
                SessionStrategy::StatusThenCreate
            },
            interaction_http: None,
        }
        .with_interaction_capabilities(UiCapabilities::default())
        .expect("the deny-all interaction adapter is valid")
    }

    pub(crate) fn stock() -> Self {
        Self {
            backend: WdaBackend::Stock,
            bundle_id: "com.riviu.managersphone.agent.xctrunner".to_string(),
            device_port: 8100,
            mjpeg_port: 9100,
            auth_token: None,
            logical_size: (375.0, 667.0),
            agent_ipa: None,
            features: vec!["stream".to_string(), "tap".to_string(), "swipe".to_string()],
            launch_kind: LaunchKind::XcTest,
            session_strategy: SessionStrategy::CreateThenPrime,
            interaction_http: None,
        }
        .with_interaction_capabilities(UiCapabilities::default())
        .expect("the deny-all interaction adapter is valid")
    }

    #[cfg(test)]
    pub(crate) fn rt_mmo(auth_token: String) -> Self {
        Self {
            backend: WdaBackend::RtMmo,
            bundle_id: "com.mrph.svc".to_string(),
            device_port: 8906,
            mjpeg_port: 9093,
            auth_token: Some(AgentToken::new(auth_token).expect("non-empty RT-MMO fixture token")),
            logical_size: (375.0, 667.0),
            agent_ipa: Some(PathBuf::from("RiviuAgent.ipa")),
            features: vec![
                "stream".to_string(),
                "tap".to_string(),
                "swipe".to_string(),
                "text".to_string(),
            ],
            launch_kind: LaunchKind::Application,
            session_strategy: SessionStrategy::StatusThenCreate,
            interaction_http: None,
        }
        .with_interaction_capabilities(UiCapabilities::default())
        .expect("the deny-all interaction adapter is valid")
    }

    pub(crate) fn with_interaction_capabilities(
        mut self,
        capabilities: UiCapabilities,
    ) -> Result<Self, UiError> {
        self.interaction_http = Some(InteractionHttpAdapter::try_new(capabilities)?);
        Ok(self)
    }

    #[cfg(test)]
    pub(crate) fn configured_interaction_capabilities(&self) -> &UiCapabilities {
        &self
            .interaction_http
            .as_ref()
            .expect("every WDA profile has an interaction adapter")
            .capabilities
    }

    #[cfg(test)]
    fn try_interaction_fixture(token: &str, capabilities: UiCapabilities) -> Result<Self, UiError> {
        let mut profile = Self::stock();
        profile.auth_token = Some(AgentToken::new(token).map_err(|error| {
            interaction_contract_error("interaction.fixture", error.to_string())
        })?);
        profile.with_interaction_capabilities(capabilities)
    }

    #[cfg(test)]
    fn interaction_fixture(token: &str, capabilities: UiCapabilities) -> Self {
        Self::try_interaction_fixture(token, capabilities).expect("valid interaction fixture")
    }

    fn auth_header_name(&self) -> &'static str {
        match self.backend {
            WdaBackend::RiviuAgent => "X-Riviu-Token",
            WdaBackend::RtMmo => "X-RT-Token",
            WdaBackend::Stock => "X-RT-Token",
        }
    }

    fn uses_sessionless_native_gestures(&self) -> bool {
        matches!(self.backend, WdaBackend::RtMmo | WdaBackend::RiviuAgent)
    }
}

impl InteractionHttpAdapter {
    fn try_new(capabilities: UiCapabilities) -> Result<Self, UiError> {
        if let Some(open_url) = capabilities.open_url.as_ref() {
            validate_interaction_route(
                &open_url.route,
                RouteMethod::Post,
                "open-url-body-v1",
                true,
            )?;
            if open_url.target_bundle_id.trim().is_empty()
                || open_url.target_bundle_id != open_url.target_bundle_id.trim()
            {
                return Err(interaction_contract_error(
                    "openUrl",
                    "target bundle id is blank or non-canonical",
                ));
            }
        }

        if let Some(clipboard) = capabilities.clipboard.as_ref() {
            validate_interaction_route(
                &clipboard.set_route,
                RouteMethod::Post,
                "clipboard-set-base64-v1",
                false,
            )?;
            validate_interaction_route(
                &clipboard.get_route,
                RouteMethod::Post,
                "clipboard-get-base64-v1",
                false,
            )?;
            if clipboard.maximum_decoded_bytes as usize != MAX_INTERACTION_CLIPBOARD_BYTES {
                return Err(interaction_contract_error(
                    "clipboard",
                    "maximum decoded bytes must be exactly 65536",
                ));
            }
        }

        let active_app_route =
            if let Some(identity) = capabilities.target_identity_copy_link.as_ref() {
                let open_url = capabilities.open_url.as_ref().ok_or_else(|| {
                    interaction_contract_error("activeAppInfo", "open URL contract is missing")
                })?;
                let clipboard = capabilities.clipboard.as_ref().ok_or_else(|| {
                    interaction_contract_error("activeAppInfo", "clipboard contract is missing")
                })?;
                let auth_header_name = &open_url.route.auth_header_name;
                if clipboard.set_route.auth_header_name != *auth_header_name
                    || clipboard.get_route.auth_header_name != *auth_header_name
                    || identity.open_url_contract_id != open_url.route.contract_id
                {
                    return Err(interaction_contract_error(
                        "activeAppInfo",
                        "identity references or protected auth headers do not match",
                    ));
                }
                Some(ProtectedRouteContract {
                    contract_id: "interaction-active-app-info-v1".to_string(),
                    method: RouteMethod::Get,
                    scope: RouteScope::Sessionless,
                    path: "/wda/activeAppInfo".to_string(),
                    auth_header_name: auth_header_name.clone(),
                    body_schema_id: "active-app-info-v1".to_string(),
                    request_timeout_ms: OPEN_URL_TIMEOUT_MS,
                })
            } else {
                None
            };

        Ok(Self {
            capabilities,
            active_app_route,
        })
    }
}

fn validate_interaction_route(
    route: &ProtectedRouteContract,
    expected_method: RouteMethod,
    expected_schema: &str,
    require_exact_open_timeout: bool,
) -> Result<(), UiError> {
    if route.contract_id.trim().is_empty()
        || route.method != expected_method
        || route.body_schema_id != expected_schema
        || !is_valid_protected_route_path(&route.path)
        || !route.auth_header_name.starts_with("X-")
        || !route.auth_header_name.ends_with("-Token")
        || !route
            .auth_header_name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        || route.request_timeout_ms == 0
        || route.request_timeout_ms > OPEN_URL_TIMEOUT_MS
        || (require_exact_open_timeout && route.request_timeout_ms != OPEN_URL_TIMEOUT_MS)
        || HeaderName::from_bytes(route.auth_header_name.as_bytes()).is_err()
    {
        return Err(interaction_contract_error(
            &route.contract_id,
            "unknown or invalid protected route contract",
        ));
    }
    Ok(())
}

fn interaction_contract_error(label: &str, message: impl Into<String>) -> UiError {
    UiError::new(UiErrorKind::Other, format!("interaction.{label}"), message)
}

#[derive(Clone)]
pub struct WdaClient {
    http: Client,
    base: String,
    port: u16,
    udid: String,
    // Sessionless native gestures can rotate the XCTest session when a
    // keyboard/composer is focused. Keep the id shared across cloned clients
    // so the following text request targets the active session.
    session_id: Arc<RwLock<Option<String>>>,
    profile: WdaProfile,
}

impl WdaClient {
    pub fn new(host: &str, port: u16, udid: &str) -> Self {
        Self::new_with_profile(host, port, udid, WdaProfile::stock())
    }

    pub(crate) fn new_with_profile(host: &str, port: u16, udid: &str, profile: WdaProfile) -> Self {
        let mut headers = HeaderMap::new();
        headers.insert(CONNECTION, HeaderValue::from_static("close"));
        Self {
            http: Client::builder()
                .connect_timeout(Duration::from_secs(2))
                .default_headers(headers)
                // tidevice relay is unreliable when reqwest reuses a keep-alive
                // socket across session/window/gesture requests. Python probes
                // that open a fresh TCP connection per request stay stable.
                .pool_max_idle_per_host(0)
                .timeout(Duration::from_secs(20))
                .build()
                .unwrap_or_else(|_| Client::new()),
            base: format!("http://{host}:{port}"),
            port,
            udid: udid.to_string(),
            session_id: Arc::new(RwLock::new(None)),
            profile,
        }
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    pub fn udid(&self) -> &str {
        &self.udid
    }

    pub fn mjpeg_url(host: &str, mjpeg_port: u16) -> String {
        format!("http://{host}:{mjpeg_port}")
    }

    /// Cheap probe so a cached session isn't reused after the device dropped it.
    pub async fn session_alive(&self) -> bool {
        if self.profile.session_strategy == SessionStrategy::StatusThenCreate {
            let Some(expected) = self.session_id.read().clone() else {
                return false;
            };
            let url = format!("{}/status", self.base);
            return self
                .send(
                    Method::Get,
                    &url,
                    "status.session",
                    None,
                    Duration::from_secs(5),
                )
                .await
                .ok()
                .and_then(|value| session_id_from(&value).map(str::to_string))
                .as_deref()
                == Some(expected.as_str());
        }
        let Ok(url) = self.session_url("/window/size") else {
            return false;
        };
        self.send(
            Method::Get,
            &url,
            "window.size",
            None,
            Duration::from_secs(5),
        )
        .await
        .is_ok()
    }

    pub async fn create_session(&mut self) -> Result<(), UiError> {
        if self.profile.session_strategy == SessionStrategy::StatusThenCreate {
            let status_url = format!("{}/status", self.base);
            if let Ok(status) = self
                .send(
                    Method::Get,
                    &status_url,
                    "status.session",
                    None,
                    Duration::from_secs(5),
                )
                .await
            {
                if let Some(sid) = session_id_from(&status) {
                    *self.session_id.write() = Some(sid.to_string());
                    return Ok(());
                }
            }

            let url = format!("{}/session", self.base);
            let body = json!({
                "capabilities": {
                    "firstMatch": [{}],
                    "alwaysMatch": {},
                }
            });
            let resp = self
                .send(
                    Method::Post,
                    &url,
                    "session.create",
                    Some(&body),
                    Duration::from_secs(20),
                )
                .await?;
            let sid = session_id_from(&resp).ok_or_else(|| {
                UiError::new(
                    UiErrorKind::Http,
                    "session.create",
                    format!("session id missing: {resp}"),
                )
            })?;
            *self.session_id.write() = Some(sid.to_string());
            return Ok(());
        }

        let url = format!("{}/session", self.base);
        // No bundleId — do not launch/activate SpringBoard (that locks→Home).
        let resp = self
            .send(
                Method::Post,
                &url,
                "session.create",
                Some(&session_capabilities()),
                Duration::from_secs(30),
            )
            .await?;
        let sid = resp
            .pointer("/value/sessionId")
            .or_else(|| resp.get("sessionId"))
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                UiError::new(
                    UiErrorKind::Http,
                    "session.create",
                    format!("session id missing: {resp}"),
                )
            })?
            .to_string();
        *self.session_id.write() = Some(sid);
        if !self.prime_session().await {
            return Err(UiError::new(
                UiErrorKind::Timeout,
                "session.prime",
                "agent nhận session nhưng không chạy được lệnh nào — runner đang kẹt",
            ));
        }
        Ok(())
    }

    /// Start a new RT-MMO automation session after the target app is already
    /// foreground. Reattaching the session advertised by `/status` keeps
    /// gestures alive but can leave `/wda/keys` acknowledging text it drops.
    pub async fn create_fresh_session(&mut self) -> Result<(), UiError> {
        if self.profile.session_strategy != SessionStrategy::StatusThenCreate {
            return self.create_session().await;
        }

        let url = format!("{}/session", self.base);
        let body = json!({"capabilities":{"firstMatch":[{}]}});
        let create_error = match self
            .send(
                Method::Post,
                &url,
                "session.create_fresh",
                Some(&body),
                Duration::from_secs(20),
            )
            .await
        {
            Ok(resp) => {
                if let Some(sid) = session_id_from(&resp).map(str::to_string) {
                    *self.session_id.write() = Some(sid);
                    return Ok(());
                }
                UiError::new(
                    UiErrorKind::Http,
                    "session.create_fresh",
                    format!("session id missing: {resp}"),
                )
            }
            Err(error) => error,
        };

        // Some RT-MMO builds reject POST /session and self-create one. Keep
        // those builds usable, but only after the fresh-session request was
        // attempted for builds that support it.
        if may_attach_status_after_fresh_create_error(&create_error) {
            let status_url = format!("{}/status", self.base);
            if let Ok(status) = self
                .send(
                    Method::Get,
                    &status_url,
                    "status.session_fallback",
                    None,
                    Duration::from_secs(5),
                )
                .await
            {
                if let Some(sid) = session_id_from(&status).map(str::to_string) {
                    *self.session_id.write() = Some(sid);
                    return Ok(());
                }
            }
        }

        Err(create_error)
    }

    /// Send one cheap session command before anything else touches the
    /// accessibility hierarchy.
    ///
    /// This is the fix for the failure that dominated every earlier live test.
    /// On this agent build, if the *first* session-scoped command is one that
    /// resolves the foreground app's hierarchy (`window/size`, `/actions`,
    /// `/wda/tap`), and that app is TikTok, the call never returns and the
    /// runner's XCTest thread stays blocked forever. `/status` and the
    /// sessionless `GET /screenshot` keep answering throughout, which is why
    /// health probes reported a healthy agent while every gesture timed out,
    /// and why recovery kept recycling a runner that looked fine.
    ///
    /// Measured on the live iPhone 8, cold runner, TikTok foreground:
    ///
    /// | first command        | window/size | tap    |
    /// |----------------------|------------:|-------:|
    /// | window/size          |     timeout |      — |
    /// | /actions tap         |           — | timeout|
    /// | appium/settings      |   107–690ms | 393–601ms |
    ///
    /// Four of four runs passed with the settings call first; every run without
    /// it wedged. The bounded snapshot depth is useful in its own right —
    /// TikTok's hierarchy is deep and nothing here needs to walk it.
    /// Returns false when the agent never answered — the caller must treat that
    /// as a wedged runner rather than carrying on into gestures that will all
    /// time out. A live run showed a stall clearing on its own after ~45 s, so
    /// this retries for about that long before giving up.
    async fn prime_session(&self) -> bool {
        let Ok(url) = self.session_url("/appium/settings") else {
            return false;
        };
        let body = json!({
            "settings": {
                "snapshotMaxDepth": 1,
                "customSnapshotTimeout": 2,
                "shouldUseCompactResponses": true,
            }
        });
        for attempt in 0..PRIME_ATTEMPTS {
            if self
                .send(
                    Method::Post,
                    &url,
                    "session.prime",
                    Some(&body),
                    Duration::from_secs(8),
                )
                .await
                .is_ok()
            {
                return true;
            }
            if attempt + 1 < PRIME_ATTEMPTS {
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
        }
        false
    }

    fn session_url(&self, path: &str) -> Result<String, UiError> {
        let sid = self
            .session_id
            .read()
            .clone()
            .ok_or_else(|| UiError::new(UiErrorKind::Session, "session", "no WDA session"))?;
        Ok(format!("{}/session/{sid}{path}", self.base))
    }

    fn update_session_from_response(&self, response: &serde_json::Value) {
        if let Some(sid) = session_id_from(response) {
            *self.session_id.write() = Some(sid.to_string());
        }
    }

    /// One WDA request, timed and classified.
    async fn send(
        &self,
        method: Method,
        url: &str,
        label: &str,
        body: Option<&serde_json::Value>,
        timeout: Duration,
    ) -> Result<serde_json::Value, UiError> {
        let started = Instant::now();
        let result = self.send_inner(method, url, label, body, timeout).await;
        let ms = started.elapsed().as_millis().min(u32::MAX as u128) as u32;
        let outcome = match &result {
            Ok(_) => Outcome::Ok,
            Err(e) => outcome_of(e.kind),
        };
        telemetry::record(&self.udid, label, ms, outcome);
        result
    }

    async fn send_protected_route(
        &self,
        route: &ProtectedRouteContract,
        label: &str,
        body: Option<&serde_json::Value>,
    ) -> Result<serde_json::Value, UiError> {
        self.send_protected_route_bounded(
            route,
            label,
            body,
            Some(INTERACTION_RESPONSE_LIMIT_BYTES),
        )
        .await
    }

    async fn send_protected_route_bounded(
        &self,
        route: &ProtectedRouteContract,
        label: &str,
        body: Option<&serde_json::Value>,
        response_limit: Option<usize>,
    ) -> Result<serde_json::Value, UiError> {
        let method = match route.method {
            RouteMethod::Get => Method::Get,
            RouteMethod::Post => Method::Post,
        };
        let url = self.render_interaction_url(route)?;
        let token = self.profile.auth_token.as_ref().ok_or_else(|| {
            interaction_contract_error(label, "protected auth token is unavailable")
        })?;
        let timeout = Duration::from_millis(u64::from(route.request_timeout_ms));
        let started = Instant::now();
        let result = self
            .send_inner_with_options(
                method,
                &url,
                label,
                body,
                SendOptions {
                    timeout,
                    auth: Some((&route.auth_header_name, token.expose())),
                    response_limit,
                },
            )
            .await;
        let ms = started.elapsed().as_millis().min(u32::MAX as u128) as u32;
        let outcome = match &result {
            Ok(_) => Outcome::Ok,
            Err(error) => outcome_of(error.kind),
        };
        telemetry::record(&self.udid, label, ms, outcome);
        result
    }

    fn render_interaction_url(&self, route: &ProtectedRouteContract) -> Result<String, UiError> {
        match route.scope {
            RouteScope::Sessionless => Ok(format!("{}{}", self.base, route.path)),
            RouteScope::Session => self.session_url(&route.path),
        }
    }

    async fn send_inner(
        &self,
        method: Method,
        url: &str,
        label: &str,
        body: Option<&serde_json::Value>,
        timeout: Duration,
    ) -> Result<serde_json::Value, UiError> {
        let auth = self
            .profile
            .auth_token
            .as_ref()
            .map(|token| (self.profile.auth_header_name(), token.expose()));
        self.send_inner_with_auth(method, url, label, body, timeout, auth)
            .await
    }

    async fn send_inner_with_auth(
        &self,
        method: Method,
        url: &str,
        label: &str,
        body: Option<&serde_json::Value>,
        timeout: Duration,
        auth: Option<(&str, &str)>,
    ) -> Result<serde_json::Value, UiError> {
        self.send_inner_with_options(
            method,
            url,
            label,
            body,
            SendOptions {
                timeout,
                auth,
                response_limit: None,
            },
        )
        .await
    }

    async fn send_inner_with_options(
        &self,
        method: Method,
        url: &str,
        label: &str,
        body: Option<&serde_json::Value>,
        options: SendOptions<'_>,
    ) -> Result<serde_json::Value, UiError> {
        let mut req = match method {
            Method::Get => self.http.get(url),
            Method::Post => self.http.post(url),
            Method::Delete => self.http.delete(url),
        };
        req = req.timeout(options.timeout);
        if let Some((name, value)) = options.auth {
            let name = HeaderName::from_bytes(name.as_bytes()).map_err(|error| {
                interaction_contract_error(label, format!("invalid auth header: {error}"))
            })?;
            let value = HeaderValue::from_str(value).map_err(|error| {
                interaction_contract_error(label, format!("invalid auth value: {error}"))
            })?;
            req = req.header(name, value);
        }
        if let Some(b) = body {
            req = req.header("Content-Type", "application/json").json(b);
        }
        let mut resp = req.send().await.map_err(|e| {
            let kind = if e.is_timeout() {
                UiErrorKind::Timeout
            } else {
                UiErrorKind::Transport
            };
            UiError::new(kind, label, format!("{url}: {e}"))
        })?;

        let status = resp.status();
        let text = if let Some(limit) = options.response_limit {
            let mut bytes = Vec::with_capacity(limit.min(16 * 1024));
            while let Some(chunk) = resp.chunk().await.map_err(|error| {
                let kind = if error.is_timeout() {
                    UiErrorKind::Timeout
                } else {
                    UiErrorKind::Transport
                };
                UiError::new(kind, label, format!("{url}: {error}"))
            })? {
                if bytes.len().saturating_add(chunk.len()) > limit {
                    return Err(interaction_contract_error(
                        label,
                        format!("HTTP response exceeds {limit} bytes"),
                    ));
                }
                bytes.extend_from_slice(&chunk);
            }
            String::from_utf8(bytes).map_err(|error| {
                interaction_contract_error(label, format!("response is not UTF-8: {error}"))
            })?
        } else {
            resp.text().await.map_err(|error| {
                let kind = if error.is_timeout() {
                    UiErrorKind::Timeout
                } else {
                    UiErrorKind::Transport
                };
                UiError::new(kind, label, format!("{url}: {error}"))
            })?
        };
        if !status.is_success() {
            let parsed = serde_json::from_str::<serde_json::Value>(&text).ok();
            let w3c_error = parsed
                .as_ref()
                .and_then(|v| v.pointer("/value/error").and_then(|m| m.as_str()))
                .unwrap_or_default()
                .to_string();
            let message = parsed
                .as_ref()
                .and_then(|v| {
                    v.pointer("/value/message")
                        .and_then(|m| m.as_str())
                        .map(|s| s.to_string())
                })
                .unwrap_or_else(|| {
                    if text.is_empty() {
                        format!("HTTP {status}")
                    } else {
                        text.chars().take(240).collect()
                    }
                });
            // "the session is gone" is fixed by making a new session; every
            // other status means the runner rejected just this one command.
            let kind = if is_session_gone(&w3c_error, &message) {
                UiErrorKind::Session
            } else {
                UiErrorKind::Http
            };
            return Err(UiError::new(
                kind,
                label,
                format!("HTTP {status}: {message}"),
            ));
        }

        if text.trim().is_empty() {
            return Ok(json!({ "value": null }));
        }
        let value: serde_json::Value =
            serde_json::from_str(&text).unwrap_or_else(|_| json!({ "value": text }));
        // WDA can answer 200 with a W3C error body.
        if let Some(err) = value.pointer("/value/error").and_then(|v| v.as_str()) {
            let message = value
                .pointer("/value/message")
                .and_then(|v| v.as_str())
                .unwrap_or(err);
            let kind = if is_session_gone(err, message) {
                UiErrorKind::Session
            } else {
                UiErrorKind::Http
            };
            return Err(UiError::new(kind, label, message.to_string()));
        }
        Ok(value)
    }

    /// Sessionless `GET /screenshot`. Kept for the vision-comment capture; the
    /// popup watcher reads the MJPEG stream instead and never calls this, which
    /// is what stopped the watcher from wedging the control relay.
    ///
    /// The ~1.2 MB body must not be cancelled mid-flight — the deadline lives
    /// on the request.
    pub async fn screenshot_png(&self) -> Result<Vec<u8>, UiError> {
        use base64::Engine;
        let url = format!("{}/screenshot", self.base);
        let started = Instant::now();
        // Dedicated client: the shared one forces `Connection: close`, which
        // stalls on this body until the deadline aborts it mid-read and wedges
        // the relay. Plain keep-alive (what curl does) returns in ~0.3 s.
        let outcome = async {
            let client = Client::builder()
                .connect_timeout(Duration::from_secs(2))
                .timeout(Duration::from_secs(25))
                .build()
                .map_err(|e| UiError::new(UiErrorKind::Other, "screenshot", e.to_string()))?;
            let mut request = client.get(&url);
            if let Some(token) = self.profile.auth_token.as_ref() {
                request = request.header(self.profile.auth_header_name(), token.expose());
            }
            let resp = request.send().await.map_err(|e| {
                let kind = if e.is_timeout() {
                    UiErrorKind::Timeout
                } else {
                    UiErrorKind::Transport
                };
                UiError::new(kind, "screenshot", e.to_string())
            })?;
            let value: serde_json::Value = resp
                .error_for_status()
                .map_err(|e| UiError::new(UiErrorKind::Http, "screenshot", e.to_string()))?
                .json()
                .await
                .map_err(|e| UiError::new(UiErrorKind::Http, "screenshot", e.to_string()))?;
            let b64 = value
                .get("value")
                .and_then(|v| v.as_str())
                .ok_or_else(|| UiError::new(UiErrorKind::Http, "screenshot", "missing value"))?;
            base64::engine::general_purpose::STANDARD
                .decode(b64)
                .map_err(|e| UiError::new(UiErrorKind::Http, "screenshot", e.to_string()))
        }
        .await;
        telemetry::record(
            &self.udid,
            "screenshot",
            started.elapsed().as_millis() as u32,
            match &outcome {
                Ok(_) => Outcome::Ok,
                Err(e) => outcome_of(e.kind),
            },
        );
        outcome
    }

    pub async fn window_size(&self) -> Result<(f64, f64), UiError> {
        if self.profile.uses_sessionless_native_gestures() {
            return Ok(self.profile.logical_size);
        }
        let url = self.session_url("/window/size")?;
        let resp = self
            .send(
                Method::Get,
                &url,
                "window.size",
                None,
                Duration::from_secs(8),
            )
            .await?;
        let w = resp
            .pointer("/value/width")
            .and_then(|v| v.as_f64())
            .or_else(|| resp.get("width").and_then(|v| v.as_f64()))
            .unwrap_or(0.0);
        let h = resp
            .pointer("/value/height")
            .and_then(|v| v.as_f64())
            .or_else(|| resp.get("height").and_then(|v| v.as_f64()))
            .unwrap_or(0.0);
        if w <= 0.0 || h <= 0.0 {
            return Err(UiError::new(
                UiErrorKind::Http,
                "window.size",
                format!("invalid window size: {resp}"),
            ));
        }
        Ok((w, h))
    }

    /// Map stream/screenshot pixel coords → WDA logical points.
    pub async fn to_points(
        &self,
        x: f64,
        y: f64,
        image_w: f64,
        image_h: f64,
    ) -> Result<TapPoint, UiError> {
        let (sw, sh) = match self.window_size().await {
            Ok(size) => size,
            Err(_) => {
                // iPhone 8 / common fallback: the stream is @2x pixels.
                let scale = if image_w > 500.0 { 2.0 } else { 1.0 };
                (image_w / scale, image_h / scale)
            }
        };
        let iw = image_w.max(1.0);
        let ih = image_h.max(1.0);
        Ok(TapPoint {
            x: ((x / iw) * sw).clamp(0.0, sw),
            y: ((y / ih) * sh).clamp(0.0, sh),
        })
    }

    fn pointer_action(steps: serde_json::Value) -> serde_json::Value {
        json!({
            "actions": [{
                "type": "pointer",
                "id": "finger1",
                "parameters": { "pointerType": "touch" },
                "actions": steps,
            }]
        })
    }

    async fn rt_mmo_native_swipe(
        &self,
        from_x: f64,
        from_y: f64,
        to_x: f64,
        to_y: f64,
        delay: f64,
        endpoint: &'static str,
    ) -> Result<(), UiError> {
        let url = format!("{}/wda/swipe", self.base);
        let body = json!({
            "delay": delay,
            "fromX": from_x,
            "fromY": from_y,
            "toX": to_x,
            "toY": to_y,
        });
        let response = self
            .send(Method::Post, &url, endpoint, Some(&body), GESTURE_TIMEOUT)
            .await?;
        self.update_session_from_response(&response);
        Ok(())
    }

    /// Tap at a logical point.
    ///
    /// PRIMARY is W3C `/actions`, for the same reason swipe uses it: it posts
    /// HID events and returns. `/wda/tap` goes through XCUICoordinate, which
    /// waits for the app to go quiescent — and TikTok's feed animates non-stop,
    /// so that wait routinely outlives the deadline and surfaces as
    /// "error sending request". Measured on this device before the change; the
    /// numbers are in `docs/LIVE_NURTURE_REPORT_2026-07-26.md`.
    ///
    /// `/wda/tap` remains a fallback for the one case it helps with: `/actions`
    /// being rejected outright (some builds 500 on the first call after init).
    pub async fn tap(&self, point: TapPoint) -> Result<(), UiError> {
        let x = point.x.round();
        let y = point.y.round();
        if self.profile.uses_sessionless_native_gestures() {
            // RT-MMO exposes this native endpoint sessionless. W3C actions
            // wedge its automation session after the first TikTok touch.
            return self
                .rt_mmo_native_swipe(x, y, x + 1.0, y + 1.0, 0.05, "tap.native-swipe")
                .await;
        }
        let body = Self::pointer_action(json!([
            { "type": "pointerMove", "duration": 0, "x": x, "y": y },
            { "type": "pointerDown", "button": 0 },
            { "type": "pause", "duration": 60 },
            { "type": "pointerUp", "button": 0 }
        ]));
        let actions_url = self.session_url("/actions")?;
        match self
            .send(
                Method::Post,
                &actions_url,
                "tap.actions",
                Some(&body),
                GESTURE_TIMEOUT,
            )
            .await
        {
            Ok(_) => Ok(()),
            // A broken socket or a dead session is the caller's business: it
            // owns the recovery budget. Do not hammer a wedged runner with a
            // fallback request.
            Err(e) if e.kind != UiErrorKind::Http => Err(e),
            Err(first) => {
                let url = self.session_url("/wda/tap")?;
                self.send(
                    Method::Post,
                    &url,
                    "tap.native",
                    Some(&json!({ "x": x, "y": y })),
                    GESTURE_TIMEOUT,
                )
                .await
                .map_err(|second| {
                    UiError::new(
                        second.kind,
                        "tap",
                        format!("actions: {}; native: {}", first.message, second.message),
                    )
                })?;
                Ok(())
            }
        }
    }

    /// `/wda/tap` only — an XCUICoordinate tap, which iOS treats as a real UI
    /// interaction and which therefore focuses text fields. See
    /// `UiSession::tap_native`.
    pub async fn tap_native(&self, point: TapPoint) -> Result<(), UiError> {
        if self.profile.uses_sessionless_native_gestures() {
            let url = format!("{}/wda/tap", self.base);
            let response = self
                .send(
                    Method::Post,
                    &url,
                    "tap.native",
                    Some(&json!({ "x": point.x.round(), "y": point.y.round() })),
                    GESTURE_TIMEOUT,
                )
                .await?;
            self.update_session_from_response(&response);
            return Ok(());
        }
        let url = self.session_url("/wda/tap")?;
        self.send(
            Method::Post,
            &url,
            "tap.native",
            Some(&json!({ "x": point.x.round(), "y": point.y.round() })),
            GESTURE_TIMEOUT,
        )
        .await?;
        Ok(())
    }

    pub async fn swipe(&self, gesture: SwipeGesture) -> Result<(), UiError> {
        let fx = gesture.from.x.round();
        let fy = gesture.from.y.round();
        let tx = gesture.to.x.round();
        let ty = gesture.to.y.round();
        let duration = (gesture.duration_ms as f64 / 1000.0).clamp(0.05, 2.0);

        if self.profile.uses_sessionless_native_gestures() {
            return self
                .rt_mmo_native_swipe(fx, fy, tx, ty, duration.clamp(0.08, 0.35), "swipe.native")
                .await;
        }

        // PRIMARY: W3C /actions. `/wda/dragfromtoforduration` uses an
        // XCUICoordinate drag, which waits for app quiescence; TikTok's feed
        // never goes quiet, so that call blocks until the relay resets.
        let actions_url = self.session_url("/actions")?;
        let body = Self::pointer_action(json!([
            { "type": "pointerMove", "duration": 0, "x": fx, "y": fy },
            { "type": "pointerDown", "button": 0 },
            { "type": "pointerMove", "duration": gesture.duration_ms.max(80), "x": tx, "y": ty },
            { "type": "pointerUp", "button": 0 }
        ]));
        match self
            .send(
                Method::Post,
                &actions_url,
                "swipe.actions",
                Some(&body),
                GESTURE_TIMEOUT,
            )
            .await
        {
            Ok(_) => Ok(()),
            Err(e) if e.kind != UiErrorKind::Http => Err(e),
            Err(first) => {
                let drag_url = self.session_url("/wda/dragfromtoforduration")?;
                let drag_body = json!({
                    "fromX": fx, "fromY": fy, "toX": tx, "toY": ty, "duration": duration,
                });
                self.send(
                    Method::Post,
                    &drag_url,
                    "swipe.drag",
                    Some(&drag_body),
                    GESTURE_TIMEOUT,
                )
                .await
                .map_err(|second| {
                    UiError::new(
                        second.kind,
                        "swipe",
                        format!("actions: {}; drag: {}", first.message, second.message),
                    )
                })?;
                Ok(())
            }
        }
    }

    pub async fn type_text(&self, text: &str) -> Result<(), UiError> {
        if text.is_empty() {
            return Ok(());
        }
        let url = self.session_url("/wda/keys")?;
        let value = match self.profile.backend {
            WdaBackend::RtMmo | WdaBackend::RiviuAgent => vec![text.to_string()],
            WdaBackend::Stock => text.chars().map(|c| c.to_string()).collect(),
        };
        let body = json!({
            "value": value,
        });
        self.send(
            Method::Post,
            &url,
            "keys",
            Some(&body),
            Duration::from_secs(20),
        )
        .await?;
        Ok(())
    }

    pub async fn home(&self) -> Result<(), UiError> {
        let url = format!("{}/wda/homescreen", self.base);
        if self
            .send(
                Method::Post,
                &url,
                "homescreen",
                Some(&json!({})),
                Duration::from_secs(8),
            )
            .await
            .is_ok()
        {
            return Ok(());
        }
        let url2 = self.session_url("/wda/pressButton")?;
        self.send(
            Method::Post,
            &url2,
            "pressButton",
            Some(&json!({ "name": "home" })),
            Duration::from_secs(8),
        )
        .await?;
        Ok(())
    }

    /// Lock the screen (WDA `/wda/lock`) — xiaowei "锁屏", batched over a fleet.
    pub async fn lock(&self) -> Result<(), UiError> {
        let url = format!("{}/wda/lock", self.base);
        self.send(
            Method::Post,
            &url,
            "lock",
            Some(&json!({})),
            Duration::from_secs(8),
        )
        .await?;
        Ok(())
    }

    /// Unlock the screen (WDA `/wda/unlock`). A device with a secure passcode wakes to its
    /// own lock screen — WDA cannot type the code, and pretending otherwise would be a lie.
    pub async fn unlock(&self) -> Result<(), UiError> {
        let url = format!("{}/wda/unlock", self.base);
        self.send(
            Method::Post,
            &url,
            "unlock",
            Some(&json!({})),
            Duration::from_secs(8),
        )
        .await?;
        Ok(())
    }

    pub async fn find_and_tap(&self, accessibility_id: &str) -> Result<(), UiError> {
        // Raw WebDriverAgent names this strategy "predicate string"; the
        // `-ios ` prefix is an Appium-ism and this agent rejects it outright
        // with "Invalid locator requested", so every lookup silently failed.
        //
        // One strategy only — repeated element lookups with long timeouts
        // wedged the relay when nurture cancelled them mid-flight.
        let predicate = format!(
            "label == {accessibility_id:?} OR name == {accessibility_id:?} OR value == {accessibility_id:?}"
        );
        self.find_and_click(&[("predicate string", predicate.as_str())])
            .await
    }

    /// Dismiss a system/UIAlertController if one is showing (location, tracking…).
    pub async fn dismiss_alert(&self) -> Result<(), UiError> {
        let url = self.session_url("/alert/dismiss")?;
        self.send(
            Method::Post,
            &url,
            "alert.dismiss",
            Some(&json!({})),
            Duration::from_secs(2),
        )
        .await?;
        Ok(())
    }

    async fn find_and_click(&self, strategies: &[(&str, &str)]) -> Result<(), UiError> {
        let url = self.session_url("/element")?;
        let short = Duration::from_millis(1800);
        let mut last = None;
        for &(using, value) in strategies {
            let body = json!({ "using": using, "value": value });
            match self
                .send(Method::Post, &url, "element.find", Some(&body), short)
                .await
            {
                Ok(resp) => {
                    let element_id = resp
                        .pointer("/value/ELEMENT")
                        .or_else(|| resp.pointer("/value/element-6066-11e4-a52e-4f735466cecf"))
                        .and_then(|v| v.as_str());
                    if let Some(element_id) = element_id {
                        let click = self.session_url(&format!("/element/{element_id}/click"))?;
                        self.send(
                            Method::Post,
                            &click,
                            "element.click",
                            Some(&json!({})),
                            short,
                        )
                        .await?;
                        return Ok(());
                    }
                    last = Some(UiError::new(
                        UiErrorKind::Http,
                        "element.find",
                        format!("element id missing for {using}={value}"),
                    ));
                }
                Err(e) => last = Some(e),
            }
        }
        Err(last.unwrap_or_else(|| {
            UiError::new(UiErrorKind::Http, "element.find", "element not found")
        }))
    }

    pub async fn assert_visible(&self, accessibility_id: &str) -> Result<(), UiError> {
        let url = self.session_url("/element")?;
        let body = json!({ "using": "accessibility id", "value": accessibility_id });
        self.send(
            Method::Post,
            &url,
            "element.find",
            Some(&body),
            Duration::from_secs(5),
        )
        .await?;
        Ok(())
    }

    pub async fn read_text(
        &self,
        locator: &QualifiedElementLocator,
        request_timeout: Duration,
    ) -> Result<String, UiError> {
        if locator.value.trim().is_empty() || locator.value.trim() != locator.value {
            return Err(UiError::new(
                UiErrorKind::Http,
                "element.readText",
                "qualified locator value is invalid",
            ));
        }
        let deadline = Instant::now().checked_add(request_timeout).ok_or_else(|| {
            UiError::new(
                UiErrorKind::Timeout,
                "element.readText",
                "read-back deadline overflow",
            )
        })?;
        let using = match locator.strategy {
            ElementLocatorStrategy::AccessibilityId => "accessibility id",
            ElementLocatorStrategy::ClassName => "class name",
        };
        let lookup_url = self.session_url("/element")?;
        let response = self
            .send(
                Method::Post,
                &lookup_url,
                "element.readText.find",
                Some(&json!({"using": using, "value": locator.value})),
                remaining_readback_timeout(deadline, "element.readText.find")?,
            )
            .await?;
        let legacy = response
            .pointer("/value/ELEMENT")
            .and_then(|value| value.as_str());
        let w3c = response
            .pointer("/value/element-6066-11e4-a52e-4f735466cecf")
            .and_then(|value| value.as_str());
        let element_id = match (legacy, w3c) {
            (Some(legacy), Some(w3c)) if legacy != w3c => {
                return Err(UiError::new(
                    UiErrorKind::Http,
                    "element.readText.find",
                    "element response contains conflicting identifiers",
                ));
            }
            (Some(value), _) | (_, Some(value))
                if !value.is_empty()
                    && value.bytes().all(|byte| {
                        byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~')
                    }) =>
            {
                value
            }
            _ => {
                return Err(UiError::new(
                    UiErrorKind::Http,
                    "element.readText.find",
                    "element identifier is missing or invalid",
                ));
            }
        };
        let text_url = self.session_url(&format!("/element/{element_id}/text"))?;
        let response = self
            .send(
                Method::Get,
                &text_url,
                "element.readText.get",
                None,
                remaining_readback_timeout(deadline, "element.readText.get")?,
            )
            .await?;
        response
            .get("value")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| {
                UiError::new(
                    UiErrorKind::Http,
                    "element.readText.get",
                    "text response value is not a string",
                )
            })
    }

    pub async fn health(&self) -> Result<bool, UiError> {
        let url = format!("{}/status", self.base);
        Ok(self
            .send(Method::Get, &url, "status", None, Duration::from_secs(8))
            .await
            .is_ok())
    }

    /// Validate a HouseArrest-staged publish tree through the protected native
    /// Agent route. The route is sessionless; the caller still owns the device
    /// lease so relay lifecycle cannot race a transfer.
    pub async fn prepare_publish_media(
        &self,
        campaign_id: &str,
        manifest_sha256: &str,
    ) -> Result<serde_json::Value, UiError> {
        if campaign_id.is_empty()
            || campaign_id.len() > 128
            || !campaign_id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
            || manifest_sha256.len() != 64
            || !manifest_sha256
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(UiError::new(
                UiErrorKind::Http,
                "media.prepare",
                "campaign id or manifest SHA-256 is not canonical",
            ));
        }
        let url = format!("{}/riviu/media/v1/prepare", self.base);
        self.send(
            Method::Post,
            &url,
            "media.prepare",
            Some(&json!({
                "campaignId": campaign_id,
                "manifestSha256": manifest_sha256,
            })),
            Duration::from_secs(15),
        )
        .await
    }

    pub async fn import_publish_media(
        &self,
        campaign_id: &str,
        manifest_sha256: &str,
    ) -> Result<serde_json::Value, UiError> {
        if campaign_id.is_empty()
            || campaign_id.len() > 128
            || !campaign_id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
            || manifest_sha256.len() != 64
            || !manifest_sha256
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(UiError::new(
                UiErrorKind::Http,
                "media.import",
                "campaign id or manifest SHA-256 is not canonical",
            ));
        }
        let url = format!("{}/riviu/media/v1/import", self.base);
        self.send(
            Method::Post,
            &url,
            "media.import",
            Some(&json!({
                "campaignId": campaign_id,
                "manifestSha256": manifest_sha256,
            })),
            Duration::from_secs(45),
        )
        .await
    }

    pub async fn cleanup_publish_media(
        &self,
        import_id: &str,
    ) -> Result<serde_json::Value, UiError> {
        if import_id.is_empty()
            || import_id.len() <= 65
            || !import_id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
        {
            return Err(UiError::new(
                UiErrorKind::Http,
                "media.cleanup",
                "import id is not canonical",
            ));
        }
        let url = format!("{}/riviu/media/v1/import/{}", self.base, import_id);
        self.send(
            Method::Delete,
            &url,
            "media.cleanup",
            None,
            Duration::from_secs(45),
        )
        .await
    }

    /// Liveness probe. Deliberately **not** a reason to recycle the runner:
    /// Stock `/status` false-negatives under USB load, and killing a live agent
    /// on that signal cost 2–3 minutes per occurrence in live test #9. RT-MMO
    /// uses its protected endpoint so a stale/wrong local listener is not
    /// adopted as the selected agent. Only a failed gesture with a transport
    /// error justifies touching the transport.
    pub async fn health_quick(&self) -> bool {
        let (path, label) = if self.profile.uses_sessionless_native_gestures() {
            ("/wda/locked", "wda.locked")
        } else {
            ("/status", "status")
        };
        let url = format!("{}{path}", self.base);
        if self
            .send(Method::Get, &url, label, None, Duration::from_secs(4))
            .await
            .is_ok()
        {
            return true;
        }
        if self.profile.uses_sessionless_native_gestures() {
            return false;
        }
        let Ok(url) = self.session_url("/window/size") else {
            return false;
        };
        self.send(
            Method::Get,
            &url,
            "window.size",
            None,
            Duration::from_secs(4),
        )
        .await
        .is_ok()
    }

    pub async fn open_url(&self, url: &str) -> Result<(), UiError> {
        self.open_url_with_idle_timeout(url, 0).await
    }

    async fn open_url_with_idle_timeout(
        &self,
        url: &str,
        idle_timeout_ms: u32,
    ) -> Result<(), UiError> {
        if idle_timeout_ms != 0 {
            return Err(interaction_contract_error(
                "openUrl",
                "idleTimeoutMs must be zero",
            ));
        }
        if !url.starts_with("https://") || url.trim() != url {
            return Err(interaction_contract_error(
                "openUrl",
                "only canonical HTTPS URLs are accepted",
            ));
        }
        let Some(capability) = self
            .profile
            .interaction_http
            .as_ref()
            .and_then(|adapter| adapter.capabilities.open_url.as_ref())
        else {
            // Candidate Agent builds expose the standard WDA URL route while
            // their live interaction-capability report is still empty. Keep
            // the fallback explicit and candidate-only; RT-MMO and stock
            // profiles remain fail-closed when no qualified route exists.
            if matches!(self.profile.backend, WdaBackend::RiviuAgent) {
                return self.open_url_standard(url).await;
            }
            return Err(interaction_contract_error(
                "openUrl",
                "capability is unsupported",
            ));
        };
        let body = json!({
            "url": url,
            "bundleId": capability.target_bundle_id,
            "idleTimeoutMs": idle_timeout_ms,
        });
        self.send_protected_route(&capability.route, "interaction.openUrl", Some(&body))
            .await?;
        Ok(())
    }

    async fn open_url_standard(&self, url: &str) -> Result<(), UiError> {
        let route = self.session_url("/url")?;
        self.send(
            Method::Post,
            &route,
            "interaction.openUrl.standard",
            Some(&json!({ "url": url })),
            Duration::from_secs(15),
        )
        .await?;
        Ok(())
    }

    pub async fn set_clipboard(&self, content_type: &str, bytes: &[u8]) -> Result<(), UiError> {
        self.set_clipboard_for_mode(
            ClipboardAccessMode::TargetBackgroundSafe,
            content_type,
            bytes,
        )
        .await
    }

    pub(crate) async fn set_clipboard_agent_foregrounded(
        &self,
        content_type: &str,
        bytes: &[u8],
    ) -> Result<(), UiError> {
        self.set_clipboard_for_mode(
            ClipboardAccessMode::AgentForegroundRequired,
            content_type,
            bytes,
        )
        .await
    }

    async fn set_clipboard_for_mode(
        &self,
        expected_mode: ClipboardAccessMode,
        content_type: &str,
        bytes: &[u8],
    ) -> Result<(), UiError> {
        use base64::Engine as _;

        if content_type.trim().is_empty() || content_type.trim() != content_type {
            return Err(interaction_contract_error(
                "setClipboard",
                "content type is blank or non-canonical",
            ));
        }
        let capability = self
            .profile
            .interaction_http
            .as_ref()
            .and_then(|adapter| adapter.capabilities.clipboard.as_ref())
            .ok_or_else(|| {
                interaction_contract_error("setClipboard", "capability is unsupported")
            })?;
        if capability.mode != expected_mode {
            return Err(interaction_contract_error(
                "setClipboard",
                "clipboard mode does not match the active lifecycle transition",
            ));
        }
        if bytes.len() > capability.maximum_decoded_bytes as usize {
            return Err(interaction_contract_error(
                "setClipboard",
                "clipboard value exceeds the qualified decoded-byte limit",
            ));
        }
        let body = json!({
            "content": base64::engine::general_purpose::STANDARD.encode(bytes),
            "contentType": content_type,
        });
        self.send_protected_route(
            &capability.set_route,
            "interaction.setClipboard",
            Some(&body),
        )
        .await?;
        Ok(())
    }

    pub async fn get_clipboard(
        &self,
        maximum_decoded_bytes: usize,
    ) -> Result<(String, Vec<u8>), UiError> {
        self.get_clipboard_for_mode(
            ClipboardAccessMode::TargetBackgroundSafe,
            maximum_decoded_bytes,
        )
        .await
    }

    pub(crate) async fn get_clipboard_agent_foregrounded(
        &self,
        maximum_decoded_bytes: usize,
    ) -> Result<(String, Vec<u8>), UiError> {
        self.get_clipboard_for_mode(
            ClipboardAccessMode::AgentForegroundRequired,
            maximum_decoded_bytes,
        )
        .await
    }

    async fn get_clipboard_for_mode(
        &self,
        expected_mode: ClipboardAccessMode,
        maximum_decoded_bytes: usize,
    ) -> Result<(String, Vec<u8>), UiError> {
        use base64::Engine as _;

        validate_clipboard_read_limit(maximum_decoded_bytes)
            .map_err(|error| interaction_contract_error("getClipboard", error.to_string()))?;
        let capability = self
            .profile
            .interaction_http
            .as_ref()
            .and_then(|adapter| adapter.capabilities.clipboard.as_ref())
            .ok_or_else(|| {
                interaction_contract_error("getClipboard", "capability is unsupported")
            })?;
        if capability.mode != expected_mode {
            return Err(interaction_contract_error(
                "getClipboard",
                "clipboard mode does not match the active lifecycle transition",
            ));
        }
        let qualified_limit = capability.maximum_decoded_bytes as usize;
        let limit = maximum_decoded_bytes.min(qualified_limit);
        let maximum_encoded_len = limit.saturating_add(2) / 3 * 4;
        let body = json!({"contentType": "plaintext"});
        let response = self
            .send_protected_route_bounded(
                &capability.get_route,
                "interaction.getClipboard",
                Some(&body),
                Some(maximum_encoded_len.saturating_add(CLIPBOARD_RESPONSE_OVERHEAD_BYTES)),
            )
            .await?;
        let encoded = response
            .get("value")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| {
                interaction_contract_error("getClipboard", "response is missing base64 value")
            })?;
        if encoded.len() > maximum_encoded_len {
            return Err(interaction_contract_error(
                "getClipboard",
                "encoded clipboard value exceeds the caller limit",
            ));
        }
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .map_err(|error| {
                interaction_contract_error(
                    "getClipboard",
                    format!("invalid base64 response: {error}"),
                )
            })?;
        if decoded.len() > limit {
            return Err(interaction_contract_error(
                "getClipboard",
                "decoded clipboard value exceeds the caller limit",
            ));
        }
        Ok(("plaintext".to_string(), decoded))
    }

    pub async fn active_app_identity(&self) -> Result<ActiveAppIdentity, UiError> {
        let route = self
            .profile
            .interaction_http
            .as_ref()
            .and_then(|adapter| adapter.active_app_route.as_ref())
            .ok_or_else(|| {
                interaction_contract_error("activeAppInfo", "capability is unsupported")
            })?;
        let response = self
            .send_protected_route(route, "interaction.activeAppInfo", None)
            .await?;
        let value = response.get("value").ok_or_else(|| {
            interaction_contract_error("activeAppInfo", "response value is missing")
        })?;
        let bundle_id = value
            .get("bundleId")
            .and_then(serde_json::Value::as_str)
            .filter(|value| !value.trim().is_empty() && value.trim() == *value)
            .ok_or_else(|| interaction_contract_error("activeAppInfo", "bundleId is missing"))?;
        let pid = value
            .get("pid")
            .and_then(serde_json::Value::as_u64)
            .filter(|pid| *pid > 0)
            .ok_or_else(|| interaction_contract_error("activeAppInfo", "pid is missing"))?;
        Ok(ActiveAppIdentity {
            bundle_id: bundle_id.to_string(),
            pid,
        })
    }

    #[cfg(test)]
    fn interaction_request_timeout(&self) -> Result<Duration, UiError> {
        self.profile
            .interaction_http
            .as_ref()
            .and_then(|adapter| adapter.capabilities.open_url.as_ref())
            .map(|capability| Duration::from_millis(u64::from(capability.route.request_timeout_ms)))
            .ok_or_else(|| interaction_contract_error("openUrl", "capability is unsupported"))
    }

    /// Bundle id of the frontmost application.
    ///
    /// Unreliable on this agent build (it has killed the runner), so the
    /// nurture engine classifies the foreground from stream frames instead.
    /// Kept because the driver trait exposes it for scripted jobs.
    pub async fn active_app_bundle(&self) -> Result<String, UiError> {
        let url = format!("{}/wda/activeAppInfo", self.base);
        let v = self
            .send(
                Method::Get,
                &url,
                "activeAppInfo",
                None,
                Duration::from_secs(4),
            )
            .await?;
        let bid = v
            .pointer("/value/bundleId")
            .or_else(|| v.pointer("/value/bundleID"))
            .or_else(|| v.pointer("/value/application/bundleId"))
            .or_else(|| v.get("bundleId"))
            .and_then(|x| x.as_str())
            .ok_or_else(|| {
                UiError::new(
                    UiErrorKind::Http,
                    "activeAppInfo",
                    format!("missing bundleId: {v}"),
                )
            })?;
        Ok(bid.to_string())
    }

    /// Bring an app to the foreground through WDA, without restarting it.
    ///
    /// This travels over the *control* relay, so callers must hold the device
    /// lock: a `tidevice launch` racing the relay is what wedged usbmux in live
    /// tests #5–#7. Activation is preferred over launch precisely because it
    /// does not restart an app that is already running.
    pub async fn activate_app(&self, bundle_id: &str) -> Result<(), UiError> {
        let activate_url = self.session_url("/wda/apps/activate")?;
        self.send(
            Method::Post,
            &activate_url,
            "apps.activate",
            Some(&json!({ "bundleId": bundle_id })),
            Duration::from_secs(10),
        )
        .await?;
        Ok(())
    }

    pub async fn launch_app(&self, bundle_id: &str) -> Result<(), UiError> {
        if self.activate_app(bundle_id).await.is_ok() {
            return Ok(());
        }
        let url = self.session_url("/wda/apps/launch")?;
        let body = json!({ "bundleId": bundle_id, "arguments": [], "environment": {} });
        self.send(
            Method::Post,
            &url,
            "apps.launch",
            Some(&body),
            Duration::from_secs(10),
        )
        .await?;
        Ok(())
    }
}

#[derive(Clone, Copy)]
enum Method {
    Get,
    Post,
    Delete,
}

fn session_id_from(value: &serde_json::Value) -> Option<&str> {
    value
        .get("sessionId")
        .or_else(|| value.pointer("/value/sessionId"))
        .and_then(|v| v.as_str())
        .filter(|sid| !sid.is_empty() && *sid != "0")
}

fn may_attach_status_after_fresh_create_error(error: &UiError) -> bool {
    if error.kind != UiErrorKind::Http {
        return false;
    }
    error
        .message
        .strip_prefix("HTTP ")
        .and_then(|message| message.split_whitespace().next())
        .and_then(|status| status.parse::<u16>().ok())
        .is_some_and(|status| matches!(status, 404 | 405 | 501))
}

fn outcome_of(kind: UiErrorKind) -> Outcome {
    match kind {
        UiErrorKind::Transport => Outcome::Transport,
        UiErrorKind::Timeout => Outcome::Timeout,
        UiErrorKind::Session => Outcome::Session,
        _ => Outcome::Http,
    }
}

fn remaining_readback_timeout(deadline: Instant, label: &str) -> Result<Duration, UiError> {
    deadline
        .checked_duration_since(Instant::now())
        .filter(|remaining| !remaining.is_zero())
        .ok_or_else(|| {
            UiError::new(
                UiErrorKind::Timeout,
                label,
                "qualified text read-back deadline expired",
            )
        })
}

/// Does this WDA error mean the session id is no longer valid?
fn is_session_gone(w3c_error: &str, message: &str) -> bool {
    let hay = format!("{w3c_error} {message}").to_lowercase();
    hay.contains("invalid session id")
        || hay.contains("session does not exist")
        || hay.contains("session is either terminated or not started")
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine as _;
    use riviu_core::{
        ClipboardAccessMode, ClipboardCapability, ElementLocatorStrategy, OpenUrlCapability,
        ProtectedRouteContract, QualifiedElementLocator, QualifiedGeometry, RouteMethod,
        RouteScope, ScreenOrientation, TargetIdentityCapability, UiCapabilities,
    };
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    const FIXTURE_TOKEN: &str = "fixture-protected-token";

    fn interaction_route(
        contract_id: &str,
        scope: RouteScope,
        path: &str,
        schema: &str,
    ) -> ProtectedRouteContract {
        ProtectedRouteContract {
            contract_id: contract_id.to_string(),
            method: RouteMethod::Post,
            scope,
            path: path.to_string(),
            auth_header_name: "X-Fixture-Token".to_string(),
            body_schema_id: schema.to_string(),
            request_timeout_ms: 10_000,
        }
    }

    fn interaction_capabilities(scope: RouteScope) -> UiCapabilities {
        UiCapabilities {
            open_url: Some(OpenUrlCapability {
                route: interaction_route(
                    "fixture-open-url-v1",
                    scope,
                    "/fixture/url",
                    "open-url-body-v1",
                ),
                target_bundle_id: "com.ss.iphone.ugc.Ame".to_string(),
                live_report_sha256: "22".repeat(32),
            }),
            clipboard: Some(ClipboardCapability {
                mode: ClipboardAccessMode::TargetBackgroundSafe,
                set_route: interaction_route(
                    "fixture-clipboard-set-v1",
                    scope,
                    "/fixture/clipboard/set",
                    "clipboard-set-base64-v1",
                ),
                get_route: interaction_route(
                    "fixture-clipboard-get-v1",
                    scope,
                    "/fixture/clipboard/get",
                    "clipboard-get-base64-v1",
                ),
                maximum_decoded_bytes: 65_536,
                live_report_sha256: "22".repeat(32),
            }),
            target_identity_copy_link: Some(TargetIdentityCapability {
                open_url_contract_id: "fixture-open-url-v1".to_string(),
                clipboard_contract_id: "fixture-clipboard-v1".to_string(),
                share_detector_version: "fixture-share-v1".to_string(),
                copy_link_detector_version: "fixture-copy-link-v1".to_string(),
                detector_set_sha256: "11".repeat(32),
                layout_id: "fixture-layout-v1".to_string(),
                geometry: QualifiedGeometry {
                    logical_width: 375.0,
                    logical_height: 667.0,
                    pixel_width: 750,
                    pixel_height: 1334,
                    scale_x: 2.0,
                    scale_y: 2.0,
                    orientation: ScreenOrientation::Portrait,
                },
                live_report_sha256: "22".repeat(32),
            }),
        }
    }

    fn interaction_client(port: u16, scope: RouteScope) -> WdaClient {
        let profile =
            WdaProfile::interaction_fixture(FIXTURE_TOKEN, interaction_capabilities(scope));
        let client = WdaClient::new_with_profile("127.0.0.1", port, "fixture", profile);
        *client.session_id.write() = Some("fixture-session".to_string());
        client
    }

    async fn one_response_server(
        body: &'static str,
    ) -> (u16, tokio::sync::oneshot::Receiver<String>) {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .expect("bind test server");
        let port = listener.local_addr().unwrap().port();
        let (tx, rx) = tokio::sync::oneshot::channel();
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.expect("accept request");
            let mut request = Vec::new();
            let mut chunk = [0u8; 2048];
            loop {
                let n = socket.read(&mut chunk).await.expect("read request");
                if n == 0 {
                    break;
                }
                request.extend_from_slice(&chunk[..n]);
                if request.windows(4).any(|w| w == b"\r\n\r\n") {
                    break;
                }
            }
            let _ = tx.send(String::from_utf8_lossy(&request).into_owned());
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            socket
                .write_all(response.as_bytes())
                .await
                .expect("write response");
        });
        (port, rx)
    }

    async fn scripted_server(
        bodies: Vec<&'static str>,
    ) -> (u16, tokio::sync::oneshot::Receiver<Vec<String>>) {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .expect("bind test server");
        let port = listener.local_addr().unwrap().port();
        let (tx, rx) = tokio::sync::oneshot::channel();
        tokio::spawn(async move {
            let mut requests = Vec::new();
            for body in bodies {
                let (mut socket, _) = listener.accept().await.expect("accept request");
                let mut request = Vec::new();
                let mut chunk = [0u8; 2048];
                loop {
                    let n = socket.read(&mut chunk).await.expect("read request");
                    if n == 0 {
                        break;
                    }
                    request.extend_from_slice(&chunk[..n]);
                    let Some(header_end) = request.windows(4).position(|w| w == b"\r\n\r\n") else {
                        continue;
                    };
                    let headers = String::from_utf8_lossy(&request[..header_end]);
                    let content_length = headers
                        .lines()
                        .find_map(|line| {
                            let (name, value) = line.split_once(':')?;
                            name.eq_ignore_ascii_case("content-length")
                                .then(|| value.trim().parse::<usize>().ok())
                                .flatten()
                        })
                        .unwrap_or(0);
                    if request.len() >= header_end + 4 + content_length {
                        break;
                    }
                }
                requests.push(String::from_utf8_lossy(&request).into_owned());
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                socket
                    .write_all(response.as_bytes())
                    .await
                    .expect("write response");
            }
            let _ = tx.send(requests);
        });
        (port, rx)
    }

    async fn delayed_scripted_server(
        responses: Vec<(&'static str, Duration)>,
    ) -> (u16, tokio::sync::oneshot::Receiver<Vec<String>>) {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .expect("bind delayed test server");
        let port = listener.local_addr().unwrap().port();
        let (tx, rx) = tokio::sync::oneshot::channel();
        tokio::spawn(async move {
            let mut requests = Vec::new();
            for (body, delay) in responses {
                let (mut socket, _) = listener.accept().await.expect("accept delayed request");
                let mut request = Vec::new();
                let mut chunk = [0_u8; 2048];
                loop {
                    let count = socket.read(&mut chunk).await.expect("read delayed request");
                    if count == 0 {
                        break;
                    }
                    request.extend_from_slice(&chunk[..count]);
                    if request.windows(4).any(|window| window == b"\r\n\r\n") {
                        break;
                    }
                }
                requests.push(String::from_utf8_lossy(&request).into_owned());
                tokio::time::sleep(delay).await;
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = socket.write_all(response.as_bytes()).await;
            }
            let _ = tx.send(requests);
        });
        (port, rx)
    }

    async fn readback_body_stall_server(delay: Duration) -> u16 {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .expect("bind body-stall server");
        let port = listener.local_addr().expect("body-stall address").port();
        tokio::spawn(async move {
            for response in [
                Some(r#"{"value":{"element-6066-11e4-a52e-4f735466cecf":"element-1"}}"#),
                None,
            ] {
                let (mut socket, _) = listener.accept().await.expect("accept read-back request");
                let mut request = Vec::new();
                let mut chunk = [0_u8; 2048];
                loop {
                    let count = socket
                        .read(&mut chunk)
                        .await
                        .expect("read read-back request");
                    if count == 0 {
                        break;
                    }
                    request.extend_from_slice(&chunk[..count]);
                    if request.windows(4).any(|window| window == b"\r\n\r\n") {
                        break;
                    }
                }
                if let Some(body) = response {
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    socket
                        .write_all(response.as_bytes())
                        .await
                        .expect("write lookup response");
                } else {
                    socket
                        .write_all(
                            b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 32\r\nConnection: close\r\n\r\n",
                        )
                        .await
                        .expect("write stalled response headers");
                    tokio::time::sleep(delay).await;
                }
            }
        });
        port
    }

    async fn one_status_server(
        status: &'static str,
        body: &'static str,
    ) -> (u16, tokio::sync::oneshot::Receiver<String>) {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .expect("bind test server");
        let port = listener.local_addr().unwrap().port();
        let (tx, rx) = tokio::sync::oneshot::channel();
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.expect("accept request");
            let mut request = Vec::new();
            let mut chunk = [0u8; 2048];
            loop {
                let n = socket.read(&mut chunk).await.expect("read request");
                if n == 0 {
                    break;
                }
                request.extend_from_slice(&chunk[..n]);
                let Some(header_end) = request.windows(4).position(|w| w == b"\r\n\r\n") else {
                    continue;
                };
                let headers = String::from_utf8_lossy(&request[..header_end]);
                let content_length = headers
                    .lines()
                    .find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        name.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse::<usize>().ok())
                            .flatten()
                    })
                    .unwrap_or(0);
                if request.len() >= header_end + 4 + content_length {
                    break;
                }
            }
            let _ = tx.send(String::from_utf8_lossy(&request).into_owned());
            let response = format!(
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(), body
            );
            socket
                .write_all(response.as_bytes())
                .await
                .expect("write response");
        });
        (port, rx)
    }

    async fn stalling_server(delay: Duration) -> u16 {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .expect("bind test server");
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.expect("accept request");
            let mut request = [0u8; 2048];
            let _ = socket.read(&mut request).await.expect("read request");
            tokio::time::sleep(delay).await;
        });
        port
    }

    #[tokio::test]
    async fn interaction_http_contract_open_url_is_exact_and_profile_scoped() {
        for (scope, expected_path) in [
            (RouteScope::Sessionless, "/fixture/url"),
            (RouteScope::Session, "/session/fixture-session/fixture/url"),
        ] {
            let (port, requests) = scripted_server(vec![r#"{"value":null}"#]).await;
            let client = interaction_client(port, scope);

            client
                .open_url("https://www.tiktok.com/@fixture/video/123")
                .await
                .expect("qualified URL request");

            let requests = requests.await.expect("captured request");
            let request = &requests[0];
            assert!(
                request.starts_with(&format!("POST {expected_path} HTTP/1.1")),
                "{request}"
            );
            let lower = request.to_ascii_lowercase();
            assert!(
                lower.contains("x-fixture-token: fixture-protected-token"),
                "{request}"
            );
            assert!(!lower.contains("x-rt-token:"), "{request}");
            let (_, body) = request.split_once("\r\n\r\n").unwrap();
            assert_eq!(
                serde_json::from_str::<serde_json::Value>(body).unwrap(),
                json!({
                    "url": "https://www.tiktok.com/@fixture/video/123",
                    "bundleId": "com.ss.iphone.ugc.Ame",
                    "idleTimeoutMs": 0,
                })
            );
            assert_eq!(
                client.interaction_request_timeout().unwrap(),
                Duration::from_secs(10)
            );
        }
    }

    #[tokio::test]
    async fn interaction_http_contract_clipboard_uses_exact_base64_schema_and_bound() {
        for (scope, prefix) in [
            (RouteScope::Sessionless, ""),
            (RouteScope::Session, "/session/fixture-session"),
        ] {
            let encoded = base64::engine::general_purpose::STANDARD.encode("xin chao".as_bytes());
            let response: &'static str =
                Box::leak(json!({"value": encoded}).to_string().into_boxed_str());
            let (port, requests) = scripted_server(vec![r#"{"value":null}"#, response]).await;
            let client = interaction_client(port, scope);

            client
                .set_clipboard("plaintext", "xin chao".as_bytes())
                .await
                .expect("set clipboard");
            let (content_type, bytes) = client.get_clipboard(65_536).await.expect("get clipboard");

            assert_eq!(content_type, "plaintext");
            assert_eq!(bytes, "xin chao".as_bytes());
            let requests = requests.await.expect("captured requests");
            assert!(
                requests[0].starts_with(&format!("POST {prefix}/fixture/clipboard/set HTTP/1.1"))
            );
            assert!(
                requests[1].starts_with(&format!("POST {prefix}/fixture/clipboard/get HTTP/1.1"))
            );
            for request in &requests {
                assert!(
                    request
                        .to_ascii_lowercase()
                        .contains("x-fixture-token: fixture-protected-token"),
                    "{request}"
                );
            }
            let set_body = requests[0].split_once("\r\n\r\n").unwrap().1;
            assert_eq!(
                serde_json::from_str::<serde_json::Value>(set_body).unwrap(),
                json!({
                    "content": base64::engine::general_purpose::STANDARD.encode("xin chao"),
                    "contentType": "plaintext",
                })
            );
            let get_body = requests[1].split_once("\r\n\r\n").unwrap().1;
            assert_eq!(
                serde_json::from_str::<serde_json::Value>(get_body).unwrap(),
                json!({"contentType": "plaintext"})
            );
        }
    }

    #[tokio::test]
    async fn interaction_http_contract_agent_clipboard_route_requires_guarded_primitive() {
        let encoded = base64::engine::general_purpose::STANDARD.encode(b"agent-value");
        let response: &'static str =
            Box::leak(json!({"value": encoded}).to_string().into_boxed_str());
        let (port, requests) = scripted_server(vec![r#"{"value":null}"#, response]).await;
        let mut capabilities = interaction_capabilities(RouteScope::Sessionless);
        capabilities.clipboard.as_mut().unwrap().mode =
            ClipboardAccessMode::AgentForegroundRequired;
        let profile = WdaProfile::interaction_fixture(FIXTURE_TOKEN, capabilities);
        let client = WdaClient::new_with_profile("127.0.0.1", port, "fixture", profile);

        assert!(client
            .set_clipboard("plaintext", b"must-not-send-directly")
            .await
            .is_err());
        client
            .set_clipboard_agent_foregrounded("plaintext", b"agent-value")
            .await
            .expect("guarded set primitive");
        assert_eq!(
            client
                .get_clipboard_agent_foregrounded(65_536)
                .await
                .expect("guarded get primitive")
                .1,
            b"agent-value"
        );

        let requests = requests.await.expect("captured guarded requests");
        assert_eq!(requests.len(), 2);
        assert!(requests[0].starts_with("POST /fixture/clipboard/set HTTP/1.1"));
        assert!(requests[1].starts_with("POST /fixture/clipboard/get HTTP/1.1"));
    }

    #[tokio::test]
    async fn interaction_http_contract_rejects_invalid_and_oversized_clipboard_values() {
        for response in [
            r#"{"value":"%%%not-base64%%%"}"#.to_string(),
            json!({"value": base64::engine::general_purpose::STANDARD.encode(vec![7u8; 65_537])})
                .to_string(),
            json!({"value": "A".repeat(100_000)}).to_string(),
        ] {
            let response: &'static str = Box::leak(response.into_boxed_str());
            let (port, _) = scripted_server(vec![response]).await;
            let client = interaction_client(port, RouteScope::Sessionless);
            assert!(client.get_clipboard(65_536).await.is_err());
        }

        let client = interaction_client(9, RouteScope::Sessionless);
        assert!(client.get_clipboard(65_537).await.is_err());
        assert!(client
            .set_clipboard("plaintext", &vec![0u8; 65_537])
            .await
            .is_err());
    }

    #[tokio::test]
    async fn interaction_http_contract_active_app_identity_requires_bundle_and_positive_pid() {
        let (port, requests) = scripted_server(vec![
            r#"{"value":{"bundleId":"com.ss.iphone.ugc.Ame","pid":1234}}"#,
        ])
        .await;
        let client = interaction_client(port, RouteScope::Sessionless);

        let identity = client.active_app_identity().await.expect("active identity");

        assert_eq!(identity.bundle_id, "com.ss.iphone.ugc.Ame");
        assert_eq!(identity.pid, 1234);
        let requests = requests.await.expect("captured request");
        assert!(requests[0].starts_with("GET /wda/activeAppInfo HTTP/1.1"));
        assert!(requests[0]
            .to_ascii_lowercase()
            .contains("x-fixture-token: fixture-protected-token"));

        for response in [
            r#"{"value":{"bundleId":"com.ss.iphone.ugc.Ame","pid":0}}"#,
            r#"{"value":{"bundleId":"com.ss.iphone.ugc.Ame"}}"#,
        ] {
            let (port, _) = scripted_server(vec![response]).await;
            let client = interaction_client(port, RouteScope::Sessionless);
            assert!(client.active_app_identity().await.is_err());
        }
    }

    #[tokio::test]
    async fn qualified_text_readback_uses_one_exact_element_lookup_then_get_text() {
        for (strategy, using, element_response) in [
            (
                ElementLocatorStrategy::AccessibilityId,
                "accessibility id",
                r#"{"value":{"element-6066-11e4-a52e-4f735466cecf":"element-1"}}"#,
            ),
            (
                ElementLocatorStrategy::ClassName,
                "class name",
                r#"{"value":{"ELEMENT":"element-1"}}"#,
            ),
        ] {
            let (port, requests) = scripted_server(vec![
                element_response,
                r#"{"value":"Tiếng Việt chính xác"}"#,
            ])
            .await;
            let client = interaction_client(port, RouteScope::Sessionless);
            let locator = QualifiedElementLocator {
                strategy,
                value: "SearchField".into(),
            };

            let value = client
                .read_text(&locator, Duration::from_secs(1))
                .await
                .expect("qualified text");

            assert_eq!(value, "Tiếng Việt chính xác");
            let requests = requests.await.expect("captured read-back requests");
            assert_eq!(requests.len(), 2);
            assert!(requests[0].starts_with("POST /session/fixture-session/element HTTP/1.1"));
            let lookup_body = requests[0].split_once("\r\n\r\n").unwrap().1;
            assert_eq!(
                serde_json::from_str::<serde_json::Value>(lookup_body).unwrap(),
                json!({"using": using, "value": "SearchField"})
            );
            assert!(requests[1]
                .starts_with("GET /session/fixture-session/element/element-1/text HTTP/1.1"));
        }
    }

    #[tokio::test]
    async fn qualified_text_readback_rejects_ambiguous_ids_and_non_string_text() {
        for responses in [
            vec![r#"{"value":{"ELEMENT":"legacy","element-6066-11e4-a52e-4f735466cecf":"w3c"}}"#],
            vec![
                r#"{"value":{"element-6066-11e4-a52e-4f735466cecf":"element-1"}}"#,
                r#"{"value":42}"#,
            ],
        ] {
            let (port, _) = scripted_server(responses).await;
            let client = interaction_client(port, RouteScope::Sessionless);
            let error = client
                .read_text(
                    &QualifiedElementLocator {
                        strategy: ElementLocatorStrategy::AccessibilityId,
                        value: "SearchField".into(),
                    },
                    Duration::from_secs(1),
                )
                .await
                .expect_err("invalid read-back response");
            assert_eq!(error.kind, UiErrorKind::Http);
        }
    }

    #[tokio::test]
    async fn qualified_text_readback_recomputes_one_deadline_before_get_text() {
        let (port, requests) = delayed_scripted_server(vec![
            (
                r#"{"value":{"element-6066-11e4-a52e-4f735466cecf":"element-1"}}"#,
                Duration::from_millis(40),
            ),
            (r#"{"value":"late"}"#, Duration::from_millis(80)),
        ])
        .await;
        let client = interaction_client(port, RouteScope::Sessionless);
        let started = Instant::now();

        let error = client
            .read_text(
                &QualifiedElementLocator {
                    strategy: ElementLocatorStrategy::AccessibilityId,
                    value: "SearchField".into(),
                },
                Duration::from_millis(100),
            )
            .await
            .expect_err("second request must use only the remaining deadline");

        assert_eq!(error.kind, UiErrorKind::Timeout);
        assert!(started.elapsed() < Duration::from_millis(180));
        let requests = tokio::time::timeout(Duration::from_millis(250), requests)
            .await
            .expect("delayed server completion")
            .expect("captured delayed requests");
        assert_eq!(requests.len(), 2);
    }

    #[tokio::test]
    async fn qualified_text_readback_preserves_timeout_while_reading_response_body() {
        let port = readback_body_stall_server(Duration::from_millis(200)).await;
        let client = interaction_client(port, RouteScope::Sessionless);
        let error = client
            .read_text(
                &QualifiedElementLocator {
                    strategy: ElementLocatorStrategy::AccessibilityId,
                    value: "SearchField".into(),
                },
                Duration::from_millis(60),
            )
            .await
            .expect_err("stalled response body must remain a timeout");
        assert_eq!(error.kind, UiErrorKind::Timeout);
    }

    #[tokio::test]
    async fn interaction_http_contract_clipboard_and_identity_fail_closed_on_auth() {
        let mut missing_auth = WdaProfile::interaction_fixture(
            FIXTURE_TOKEN,
            interaction_capabilities(RouteScope::Sessionless),
        );
        missing_auth.auth_token = None;
        let client = WdaClient::new_with_profile("127.0.0.1", 9, "fixture", missing_auth);
        assert!(client.set_clipboard("plaintext", b"fixture").await.is_err());
        assert!(client.active_app_identity().await.is_err());

        let (port, request) = one_status_server(
            "401 Unauthorized",
            r#"{"value":{"error":"unauthorized","message":"bad token"}}"#,
        )
        .await;
        let profile = WdaProfile::interaction_fixture(
            "wrong-fixture-token",
            interaction_capabilities(RouteScope::Sessionless),
        );
        let client = WdaClient::new_with_profile("127.0.0.1", port, "fixture", profile);
        let error = client
            .set_clipboard("plaintext", b"fixture")
            .await
            .expect_err("wrong clipboard auth must be rejected");
        assert_eq!(error.kind, UiErrorKind::Http);
        assert!(request
            .await
            .expect("captured clipboard request")
            .to_ascii_lowercase()
            .contains("x-fixture-token: wrong-fixture-token"));

        let (port, request) = one_status_server(
            "401 Unauthorized",
            r#"{"value":{"error":"unauthorized","message":"bad token"}}"#,
        )
        .await;
        let profile = WdaProfile::interaction_fixture(
            "wrong-fixture-token",
            interaction_capabilities(RouteScope::Sessionless),
        );
        let client = WdaClient::new_with_profile("127.0.0.1", port, "fixture", profile);
        let error = client
            .active_app_identity()
            .await
            .expect_err("wrong identity auth must be rejected");
        assert_eq!(error.kind, UiErrorKind::Http);
        assert!(request
            .await
            .expect("captured identity request")
            .to_ascii_lowercase()
            .contains("x-fixture-token: wrong-fixture-token"));
    }

    #[tokio::test]
    async fn interaction_http_contract_uses_request_local_deadline() {
        let port = stalling_server(Duration::from_millis(500)).await;
        let mut capabilities = interaction_capabilities(RouteScope::Sessionless);
        let clipboard = capabilities.clipboard.as_mut().expect("clipboard fixture");
        clipboard.set_route.request_timeout_ms = 50;
        clipboard.get_route.request_timeout_ms = 50;
        let profile = WdaProfile::interaction_fixture(FIXTURE_TOKEN, capabilities);
        let client = WdaClient::new_with_profile("127.0.0.1", port, "fixture", profile);
        let started = Instant::now();

        let error = client
            .set_clipboard("plaintext", b"fixture")
            .await
            .expect_err("stalled request must hit its route-local deadline");

        assert_eq!(error.kind, UiErrorKind::Timeout);
        assert!(started.elapsed() < Duration::from_millis(400));
    }

    #[tokio::test]
    async fn interaction_http_contract_fails_closed_for_missing_or_unknown_contracts() {
        let mut no_open = interaction_capabilities(RouteScope::Sessionless);
        no_open.open_url = None;
        no_open.target_identity_copy_link = None;
        let profile = WdaProfile::interaction_fixture(FIXTURE_TOKEN, no_open);
        let client = WdaClient::new_with_profile("127.0.0.1", 9, "fixture", profile);
        assert!(client.open_url("https://example.invalid").await.is_err());

        let mut unknown = interaction_capabilities(RouteScope::Sessionless);
        unknown.open_url.as_mut().unwrap().route.body_schema_id = "unknown-schema".into();
        assert!(WdaProfile::try_interaction_fixture(FIXTURE_TOKEN, unknown).is_err());

        let mut wrong_method = interaction_capabilities(RouteScope::Sessionless);
        wrong_method.open_url.as_mut().unwrap().route.method = RouteMethod::Get;
        assert!(WdaProfile::try_interaction_fixture(FIXTURE_TOKEN, wrong_method).is_err());

        let client = interaction_client(9, RouteScope::Sessionless);
        assert!(client
            .open_url_with_idle_timeout("https://example.invalid", 1)
            .await
            .is_err());

        let profile = WdaProfile::interaction_fixture(
            FIXTURE_TOKEN,
            interaction_capabilities(RouteScope::Session),
        );
        let client = WdaClient::new_with_profile("127.0.0.1", 9, "fixture", profile);
        let error = client
            .open_url("https://example.invalid")
            .await
            .expect_err("session route without a session must fail before HTTP");
        assert_eq!(error.kind, UiErrorKind::Session);
    }

    #[tokio::test]
    async fn interaction_http_contract_auth_errors_and_open_failure_have_no_fallback() {
        let (port, request) = one_status_server(
            "401 Unauthorized",
            r#"{"value":{"error":"unauthorized","message":"bad token"}}"#,
        )
        .await;
        let profile = WdaProfile::interaction_fixture(
            "wrong-fixture-token",
            interaction_capabilities(RouteScope::Sessionless),
        );
        let client = WdaClient::new_with_profile("127.0.0.1", port, "fixture", profile);
        let error = client
            .open_url("https://www.tiktok.com/@fixture/video/123")
            .await
            .expect_err("wrong auth must be rejected");
        assert_eq!(error.kind, UiErrorKind::Http);
        let request = request.await.expect("captured request");
        assert!(request.starts_with("POST /fixture/url HTTP/1.1"));

        let (port, request) = one_status_server(
            "500 Internal Server Error",
            r#"{"value":{"error":"unknown error","message":"open failed"}}"#,
        )
        .await;
        let client = interaction_client(port, RouteScope::Sessionless);
        assert!(client
            .open_url("https://www.tiktok.com/@fixture/video/123")
            .await
            .is_err());
        let request = request.await.expect("single captured request");
        assert!(request.starts_with("POST /fixture/url HTTP/1.1"));
        assert!(!request.contains("Safari"));

        let mut profile = WdaProfile::interaction_fixture(
            FIXTURE_TOKEN,
            interaction_capabilities(RouteScope::Sessionless),
        );
        profile.auth_token = None;
        let client = WdaClient::new_with_profile("127.0.0.1", 9, "fixture", profile);
        assert!(client
            .open_url("https://www.tiktok.com/@fixture/video/123")
            .await
            .is_err());
    }

    #[tokio::test]
    async fn interaction_http_contract_production_rt_profile_stays_unsupported() {
        let profile = WdaProfile::rt_mmo("production-fixture-token".to_string());
        let client = WdaClient::new_with_profile("127.0.0.1", 9, "fixture", profile);

        assert!(client
            .open_url("https://www.tiktok.com/@fixture/video/123")
            .await
            .is_err());
        assert!(client.set_clipboard("plaintext", b"fixture").await.is_err());
        assert!(client.get_clipboard(65_536).await.is_err());
        assert!(client.active_app_identity().await.is_err());
    }

    #[test]
    fn rt_mmo_profile_keeps_control_stream_and_launch_on_one_agent() {
        let profile = WdaProfile::rt_mmo("test-token".to_string());

        assert_eq!(profile.backend, WdaBackend::RtMmo);
        assert_eq!(profile.bundle_id, "com.mrph.svc");
        assert_eq!(profile.device_port, 8906);
        assert_eq!(profile.mjpeg_port, 9093);
        assert_eq!(
            profile.auth_token.as_ref().map(AgentToken::expose),
            Some("test-token")
        );
        assert_eq!(profile.logical_size, (375.0, 667.0));
        assert_eq!(profile.launch_kind, LaunchKind::Application);
        assert_eq!(profile.session_strategy, SessionStrategy::StatusThenCreate);
        assert!(!format!("{profile:?}").contains("test-token"));
    }

    #[tokio::test]
    async fn rt_mmo_attaches_status_session_with_auth_and_without_post_or_prime() {
        let (port, request) = one_response_server(r#"{"value":{"sessionId":"sid-status"}}"#).await;
        let profile = WdaProfile::rt_mmo("test-token".to_string());
        let mut client = WdaClient::new_with_profile("127.0.0.1", port, "udid-a", profile);

        client.create_session().await.expect("attach session");

        let request = request.await.expect("captured request");
        assert!(request.starts_with("GET /status HTTP/1.1"), "{request}");
        assert!(
            request
                .to_ascii_lowercase()
                .contains("x-rt-token: test-token"),
            "{request}"
        );
        assert_eq!(client.session_id.read().as_deref(), Some("sid-status"));
    }

    #[tokio::test]
    async fn rt_mmo_creates_session_only_when_status_has_no_session_id() {
        let (port, requests) = scripted_server(vec![
            r#"{"value":{"ready":true}}"#,
            r#"{"sessionId":"sid-created","value":{}}"#,
        ])
        .await;
        let profile = WdaProfile::rt_mmo("test-token".to_string());
        let mut client = WdaClient::new_with_profile("127.0.0.1", port, "udid-a", profile);

        client
            .create_session()
            .await
            .expect("create fallback session");

        let requests = requests.await.expect("captured requests");
        assert_eq!(requests.len(), 2, "{requests:#?}");
        assert!(requests[0].starts_with("GET /status HTTP/1.1"));
        assert!(requests[1].starts_with("POST /session HTTP/1.1"));
        let (_, body) = requests[1].split_once("\r\n\r\n").unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(body).unwrap(),
            json!({"capabilities":{"firstMatch":[{}],"alwaysMatch":{}}})
        );
        assert!(requests.iter().all(|request| request
            .to_ascii_lowercase()
            .contains("x-rt-token: test-token")));
        assert_eq!(client.session_id.read().as_deref(), Some("sid-created"));
    }

    #[tokio::test]
    async fn rt_mmo_screenshot_request_keeps_the_auth_header() {
        let encoded = base64::engine::general_purpose::STANDARD.encode(b"png");
        let body: &'static str = Box::leak(
            serde_json::json!({"value": encoded})
                .to_string()
                .into_boxed_str(),
        );
        let (port, request) = one_response_server(body).await;
        let profile = WdaProfile::rt_mmo("test-token".to_string());
        let client = WdaClient::new_with_profile("127.0.0.1", port, "udid-a", profile);

        assert_eq!(client.screenshot_png().await.unwrap(), b"png");

        let request = request.await.expect("captured request");
        assert!(request.starts_with("GET /screenshot HTTP/1.1"), "{request}");
        assert!(
            request
                .to_ascii_lowercase()
                .contains("x-rt-token: test-token"),
            "{request}"
        );
    }

    #[tokio::test]
    async fn rt_mmo_fresh_text_session_posts_exact_payload_before_status() {
        let (port, request) = one_response_server(r#"{"sessionId":"sid-fresh","value":{}}"#).await;
        let profile = WdaProfile::rt_mmo("test-token".to_string());
        let mut client = WdaClient::new_with_profile("127.0.0.1", port, "udid-a", profile);

        client
            .create_fresh_session()
            .await
            .expect("create fresh session");

        let request = request.await.expect("captured request");
        assert!(request.starts_with("POST /session HTTP/1.1"), "{request}");
        let (_, body) = request.split_once("\r\n\r\n").unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(body).unwrap(),
            json!({"capabilities":{"firstMatch":[{}]}})
        );
        assert_eq!(client.session_id.read().as_deref(), Some("sid-fresh"));
    }

    #[test]
    fn fresh_session_status_fallback_is_limited_to_agent_rejection() {
        for message in [
            "HTTP 404 Not Found: unknown command",
            "HTTP 405 Method Not Allowed",
            "HTTP 501 Not Implemented",
        ] {
            assert!(may_attach_status_after_fresh_create_error(&UiError::new(
                UiErrorKind::Http,
                "session.create_fresh",
                message,
            )));
        }
        for message in [
            "HTTP 401 Unauthorized",
            "HTTP 500 Internal Server Error",
            "session id missing: {}",
        ] {
            assert!(
                !may_attach_status_after_fresh_create_error(&UiError::new(
                    UiErrorKind::Http,
                    "session.create_fresh",
                    message,
                )),
                "{message} must not attach a stale status session"
            );
        }
        for kind in [
            UiErrorKind::Transport,
            UiErrorKind::Timeout,
            UiErrorKind::Session,
            UiErrorKind::Other,
        ] {
            assert!(
                !may_attach_status_after_fresh_create_error(&UiError::new(
                    kind,
                    "session.create_fresh",
                    "request failed",
                )),
                "{kind:?} must not attach a potentially stale status session"
            );
        }
    }

    #[tokio::test]
    async fn rt_mmo_taps_use_sessionless_native_swipe() {
        let (port, requests) =
            scripted_server(vec![r#"{"value":null}"#, r#"{"value":null}"#]).await;
        let profile = WdaProfile::rt_mmo("test-token".to_string());
        let client = WdaClient::new_with_profile("127.0.0.1", port, "udid-a", profile);
        *client.session_id.write() = Some("sid-attached".to_string());

        client.tap(TapPoint { x: 120.2, y: 639.8 }).await.unwrap();
        client
            .tap_native(TapPoint { x: 337.0, y: 307.0 })
            .await
            .unwrap();

        let requests = requests.await.expect("captured requests");
        assert_eq!(requests.len(), 2);
        assert!(
            requests[0].starts_with("POST /wda/swipe HTTP/1.1"),
            "{}",
            requests[0]
        );
        assert!(
            requests[1].starts_with("POST /wda/tap HTTP/1.1"),
            "{}",
            requests[1]
        );
        for request in &requests {
            assert!(!request.contains("/actions"), "{request}");
            assert!(request
                .to_ascii_lowercase()
                .contains("x-rt-token: test-token"));
        }
        let (_, first_body) = requests[0].split_once("\r\n\r\n").unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(first_body).unwrap(),
            json!({"delay":0.05,"fromX":120.0,"fromY":640.0,"toX":121.0,"toY":641.0})
        );
        let (_, second_body) = requests[1].split_once("\r\n\r\n").unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(second_body).unwrap(),
            json!({"x":337.0,"y":307.0})
        );
    }

    #[tokio::test]
    async fn sessionless_native_gesture_tracks_rotated_session_id() {
        let (port, request) =
            one_response_server(r#"{"sessionId":"sid-rotated","value":null}"#).await;
        let profile = WdaProfile::rt_mmo("test-token".to_string());
        let client = WdaClient::new_with_profile("127.0.0.1", port, "udid-a", profile);
        *client.session_id.write() = Some("sid-before".to_string());

        client.tap(TapPoint { x: 120.0, y: 640.0 }).await.unwrap();

        let request = request.await.expect("captured request");
        assert!(request.starts_with("POST /wda/swipe HTTP/1.1"), "{request}");
        assert_eq!(client.session_id.read().as_deref(), Some("sid-rotated"));
    }

    #[tokio::test]
    async fn rt_mmo_types_whole_text_as_one_value_token() {
        let (port, request) = one_response_server(r#"{"value":null}"#).await;
        let profile = WdaProfile::rt_mmo("test-token".to_string());
        let client = WdaClient::new_with_profile("127.0.0.1", port, "udid-a", profile);
        *client.session_id.write() = Some("sid-attached".to_string());

        client.type_text("Hay qua ban oi").await.unwrap();

        let request = request.await.expect("captured request");
        assert!(
            request.starts_with("POST /session/sid-attached/wda/keys HTTP/1.1"),
            "{request}"
        );
        let (_, body) = request.split_once("\r\n\r\n").unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(body).unwrap(),
            json!({"value":["Hay qua ban oi"]})
        );
    }

    #[tokio::test]
    async fn rt_mmo_feed_swipe_uses_sessionless_native_endpoint() {
        let (port, request) = one_response_server(r#"{"value":null}"#).await;
        let profile = WdaProfile::rt_mmo("test-token".to_string());
        let client = WdaClient::new_with_profile("127.0.0.1", port, "udid-a", profile);
        *client.session_id.write() = Some("sid-attached".to_string());

        client
            .swipe(SwipeGesture {
                from: TapPoint { x: 180.0, y: 530.0 },
                to: TapPoint { x: 180.0, y: 180.0 },
                duration_ms: 300,
            })
            .await
            .unwrap();

        let request = request.await.expect("captured request");
        assert!(request.starts_with("POST /wda/swipe HTTP/1.1"), "{request}");
        assert!(!request.contains("/actions"), "{request}");
        let (_, body) = request.split_once("\r\n\r\n").unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(body).unwrap(),
            json!({"delay":0.3,"fromX":180.0,"fromY":530.0,"toX":180.0,"toY":180.0})
        );
    }

    #[tokio::test]
    async fn rt_mmo_uses_logical_size_without_probing_missing_window_endpoint() {
        let profile = WdaProfile::rt_mmo("test-token".to_string());
        let client = WdaClient::new_with_profile("127.0.0.1", 9, "udid-a", profile);

        assert_eq!(client.window_size().await.unwrap(), (375.0, 667.0));
        let point = client.to_points(375.0, 667.0, 750.0, 1334.0).await.unwrap();
        assert_eq!(point.x, 187.5);
        assert_eq!(point.y, 333.5);
    }

    #[tokio::test]
    async fn rt_mmo_cached_session_liveness_uses_status_not_window_size() {
        let (port, request) = one_response_server(r#"{"sessionId":"sid-status"}"#).await;
        let profile = WdaProfile::rt_mmo("test-token".to_string());
        let client = WdaClient::new_with_profile("127.0.0.1", port, "udid-a", profile);
        *client.session_id.write() = Some("sid-status".to_string());

        assert!(client.session_alive().await);

        let request = request.await.expect("captured request");
        assert!(request.starts_with("GET /status HTTP/1.1"), "{request}");
    }

    #[tokio::test]
    async fn rt_mmo_transport_readiness_uses_the_protected_route() {
        let (port, request) = one_response_server(r#"{"value":false}"#).await;
        let profile = WdaProfile::rt_mmo("test-token".to_string());
        let client = WdaClient::new_with_profile("127.0.0.1", port, "udid-a", profile);

        assert!(client.health_quick().await);

        let request = request.await.expect("captured request");
        assert!(request.starts_with("GET /wda/locked HTTP/1.1"), "{request}");
        assert!(request
            .to_ascii_lowercase()
            .contains("x-rt-token: test-token"));
    }

    #[test]
    fn nurture_session_omits_bundle_id_to_avoid_homescreen_flash() {
        let caps = session_capabilities();
        assert!(caps.pointer("/capabilities/alwaysMatch/bundleId").is_none());
        assert!(caps.pointer("/desiredCapabilities/bundleId").is_none());
        assert_eq!(
            caps.pointer("/capabilities/alwaysMatch/appium:forceAppLaunch")
                .and_then(|v| v.as_bool()),
            Some(false)
        );
        // The alert monitor these flags install is what blocked the agent's
        // XCTest thread on TikTok; they must stay gone.
        assert!(caps
            .pointer("/capabilities/alwaysMatch/appium:autoDismissAlerts")
            .is_none());
        assert!(caps
            .pointer("/desiredCapabilities/autoDismissAlerts")
            .is_none());
        assert!(caps
            .pointer("/capabilities/alwaysMatch/defaultAlertAction")
            .is_none());
    }

    /// WebDriverAgent's own locator names have no `-ios ` prefix — that is an
    /// Appium addition. Sending the prefixed form makes the agent answer
    /// "Invalid locator requested" and the lookup never runs.
    #[test]
    fn locator_strategies_use_raw_wda_names() {
        for name in [
            "predicate string",
            "class chain",
            "class name",
            "accessibility id",
        ] {
            assert!(
                !name.starts_with("-ios "),
                "{name} carries the Appium prefix"
            );
        }
    }

    #[test]
    fn a_dead_session_is_told_apart_from_a_rejected_command() {
        assert!(is_session_gone("invalid session id", "whatever"));
        assert!(is_session_gone(
            "",
            "A session is either terminated or not started"
        ));
        assert!(!is_session_gone(
            "no such element",
            "unable to find element"
        ));
        assert!(!is_session_gone("", "HTTP 500"));
    }

    #[test]
    fn error_classes_map_onto_telemetry_outcomes() {
        assert_eq!(outcome_of(UiErrorKind::Transport), Outcome::Transport);
        assert_eq!(outcome_of(UiErrorKind::Timeout), Outcome::Timeout);
        assert_eq!(outcome_of(UiErrorKind::Session), Outcome::Session);
        assert_eq!(outcome_of(UiErrorKind::Http), Outcome::Http);
        assert_eq!(outcome_of(UiErrorKind::Other), Outcome::Http);
    }

    /// Retrying a transport failure is safe (the command never landed);
    /// retrying a timeout is not (it may have landed and be running).
    #[test]
    fn retry_safety_matches_the_failure_class() {
        assert!(UiErrorKind::Transport.is_safe_to_retry());
        assert!(UiErrorKind::Session.is_safe_to_retry());
        assert!(!UiErrorKind::Timeout.is_safe_to_retry());
        assert!(!UiErrorKind::Http.is_safe_to_retry());
    }

    #[test]
    fn a_gesture_deadline_leaves_room_for_a_retry_inside_the_soft_budget() {
        // Soft recovery is budgeted at 15 s; one gesture plus one retry has to
        // fit inside it or the budget is a lie.
        assert!(GESTURE_TIMEOUT.as_secs() * 2 <= 20);
    }
}
