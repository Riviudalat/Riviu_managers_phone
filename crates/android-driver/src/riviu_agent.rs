//! HTTP client for the Riviu helper APK (`com.riviu.agent`).
//!
//! This is **not** the uiautomator2 session in [`crate::agent`]. That server
//! drives taps, the tree, and `ACTION_SET_TEXT`. This one exists for the things
//! neither that server nor adb can honestly do:
//!
//! * clipboard read on Android 10+ (the Appium route returns empty; advertising
//!   that as success is the lie AGENTS.md §9 forbids);
//! * MediaStore insert from an app UID, so `is_pending` starts clearable;
//! * wallpaper and mock location, both of which need an app context;
//! * **app names and icons** — `PackageManager.getApplicationLabel` and
//!   `getApplicationIcon`. adb returns the label as a resource id needing the
//!   device locale, and no farm phone here has `aapt` (AGENTS.md §9.55/§9.89).
//!
//! The list grows, so [`REQUIRED_FEATURES`] and `/status` carry it: a phone with
//! an older APK is reinstalled once rather than left silently short of a feature.
//!
//! The helper IME is enabled for one request and then the previous IME is
//! restored. Leaving it as the default keyboard is GenFarmer's mark, not ours.

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{anyhow, Context};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::adb::{self, AdbProgram};
use crate::frames;

/// Package installed on the phone.
pub const PACKAGE: &str = "com.riviu.agent";
/// IME id the driver `ime set`s for one clipboard call.
pub const IME_ID: &str = "com.riviu.agent/.RiviuIme";
/// Foreground service that binds the loopback HTTP server.
pub const SERVICE: &str = "com.riviu.agent/.AgentService";
/// Device-side listen port. Host reaches it through `adb forward tcp:0 tcp:17980`.
pub const DEVICE_PORT: u16 = 17980;
/// Protocol the APK and this client both speak. A newer APK with a different
/// number is refused rather than half-read.
pub const PROTOCOL_VERSION: u32 = 1;
pub const AGENT_VERSION: &str = "0.4.0";

/// What this build needs the installed helper to advertise on `/status`.
///
/// Features and not a version number, deliberately: the question is never "is it 0.3.0", it is
/// "can it answer the call I am about to make", and a phone can legitimately carry a newer
/// APK than this build knows about. A helper missing any of these is reinstalled once — see
/// [`HelperClient::upgrade_if_stale`].
const REQUIRED_FEATURES: &[&str] = &["clipboard", "pushMedia", "appLabels", "auth"];

/// Serials this process has already tried to upgrade, so a stale APK on disk cannot turn
/// every helper call into another install attempt.
fn upgrade_attempts() -> &'static parking_lot::Mutex<std::collections::HashSet<String>> {
    static ATTEMPTED: std::sync::OnceLock<parking_lot::Mutex<std::collections::HashSet<String>>> =
        std::sync::OnceLock::new();
    ATTEMPTED.get_or_init(Default::default)
}

/// Header the helper's shared token travels in. Mirrors `HttpServer.TOKEN_HEADER`.
pub const TOKEN_HEADER: &str = "X-Riviu-Token";

/// The token this process uses for one phone, minted on first use.
///
/// Per **serial**, not one for the fleet: a token is only as contained as the thing that holds
/// it, and a helper on one phone has no business being able to answer for another. Per
/// **process**, not persisted: it lives as long as the desktop does, so there is nothing on
/// either disk to steal, and a restart simply re-provisions.
fn helper_token(serial: &str) -> String {
    static TOKENS: std::sync::OnceLock<
        parking_lot::Mutex<std::collections::HashMap<String, String>>,
    > = std::sync::OnceLock::new();
    let tokens = TOKENS.get_or_init(Default::default);
    let mut tokens = tokens.lock();
    tokens
        .entry(serial.to_string())
        .or_insert_with(|| {
            format!(
                "{}{}",
                uuid::Uuid::new_v4().simple(),
                uuid::Uuid::new_v4().simple()
            )
        })
        .clone()
}

/// One helper request, over `adb forward` to loopback on the phone.
///
/// Everything on this path is small and local — clipboard text, an app label, a wallpaper
/// path — so ten seconds is already far past working. It exists to bound the case the port
/// is held by something that accepted the connection and then said nothing, which
/// `adb forward` makes reachable.
const HTTP_TIMEOUT: Duration = Duration::from_secs(10);

/// Installing the helper APK, which is a different order of work entirely.
///
/// `pm install` on the older phones in this fleet verifies and optimises the package, and
/// that is minutes, not seconds. Bounded anyway: an install that has not finished by now has
/// hung, and the caller needs to hear that rather than block the fleet.
const INSTALL_TIMEOUT: Duration = Duration::from_secs(300);
/// Hard cap on what the host will read back from one helper request.
///
/// A timeout alone does not bound a response: a helper — or anything that got to the port
/// first, since `adb forward` reaches whatever is listening — can stream as fast as USB allows
/// for the full ten seconds and the host buffers all of it. Twenty phones doing that at once is
/// an out-of-memory on the desktop from one call.
///
/// 8 MiB is chosen against the biggest legitimate response, not against a round number: the
/// helper's own icon budget is 3 MB of base64 PNG (`AppList.java`), and that budget is
/// *voluntary* — it only binds an honest server, which is exactly why the host needs its own.
/// `wda.rs` caps the iOS side the same way at 64 KiB; Android needs more only because app icons
/// travel on this channel.
const MAX_RESPONSE_BYTES: usize = 8 * 1024 * 1024;

/// One helper connection, cheap to clone (shared HTTP client + serial).
#[derive(Clone)]
pub struct HelperClient {
    http: reqwest::Client,
    adb: AdbProgram,
    serial: String,
    base: String,
    host_port: u16,
}

impl HelperClient {
    /// Install if needed, enable the IME, start the service, forward, prove `/status`.
    pub async fn ensure(adb: AdbProgram, serial: &str, apk: Option<&Path>) -> anyhow::Result<Self> {
        if !package_installed(&adb, serial).await? {
            let apk = apk.ok_or_else(|| {
                anyhow!(
                    "com.riviu.agent is not installed on {serial} and no helper APK is configured \
                     (RIVIU_ANDROID_AGENT_APK or the bundled riviu-agent.apk)"
                )
            })?;
            install_apk(&adb, serial, apk).await?;
        }
        enable_ime(&adb, serial).await?;
        start_service(&adb, serial).await?;
        let host_port = forward_helper(&adb, serial).await?;
        let client = Self::at(adb, serial, host_port)?;
        let status = client.require_status().await?;
        if let Some(apk) = apk {
            client.upgrade_if_stale(&status, apk).await;
        }
        Ok(client)
    }

    /// Replace a helper that predates a feature this build needs, once per phone per run.
    ///
    /// Twenty phones already carry the APK from before `appLabels` existed, and `pm path`
    /// says only *whether* something is installed — so without this the new feature would be
    /// silently dead on the whole fleet while `/status` answered happily. That is precisely
    /// the failure this project's rules call out: a fallback nobody knows is a fallback.
    ///
    /// Best effort by design. An upgrade that cannot happen (MIUI refusing an install, no
    /// bundled APK) must not cost the caller the clipboard call it actually asked for, so
    /// this logs and returns rather than failing `ensure`. Attempted at most once per serial
    /// per process, because if the APK on disk is also old the version never advances and a
    /// retry every call would reinstall forever.
    async fn upgrade_if_stale(&self, status: &HelperStatus, apk: &Path) {
        let missing: Vec<&str> = REQUIRED_FEATURES
            .iter()
            .copied()
            .filter(|feature| !status.features.iter().any(|have| have == feature))
            .collect();
        if missing.is_empty() {
            return;
        }
        {
            let mut attempted = upgrade_attempts().lock();
            if !attempted.insert(self.serial.clone()) {
                return;
            }
        }
        tracing::warn!(
            serial = %self.serial,
            installed = %status.agent_version,
            missing = %missing.join(", "),
            "Riviu helper thiếu tính năng — cài lại APK helper một lần"
        );
        if let Err(error) = install_apk(&self.adb, &self.serial, apk).await {
            tracing::warn!(serial = %self.serial, %error, "cài lại helper thất bại, dùng bản cũ");
            return;
        }
        // The reinstall kills the service; the host-side forward survives it because it is
        // keyed on the device port, not on the process.
        if let Err(error) = start_service(&self.adb, &self.serial).await {
            tracing::warn!(serial = %self.serial, %error, "helper mới chưa khởi động lại được");
            return;
        }
        match self.require_status().await {
            Ok(fresh) => tracing::info!(
                serial = %self.serial,
                version = %fresh.agent_version,
                features = %fresh.features.join(", "),
                "helper đã cài lại"
            ),
            Err(error) => {
                tracing::warn!(serial = %self.serial, %error, "helper mới không trả /status")
            }
        }
    }

    fn at(adb: AdbProgram, serial: &str, host_port: u16) -> anyhow::Result<Self> {
        let http = reqwest::Client::builder()
            .timeout(HTTP_TIMEOUT)
            .build()
            .context("dựng HTTP client cho Riviu helper")?;
        Ok(Self {
            http,
            adb,
            serial: serial.to_string(),
            base: format!("http://127.0.0.1:{host_port}"),
            host_port,
        })
    }

    /// Stop only the helper transport this client established.
    pub async fn shutdown(self) -> anyhow::Result<()> {
        let mut failures = Vec::new();
        if let Err(error) = frames::remove_forward(&self.adb, &self.serial, self.host_port).await {
            failures.push(format!("remove tcp:{} forward: {error}", self.host_port));
        }
        if let Err(error) = self
            .adb
            .shell(&self.serial, &format!("am force-stop {PACKAGE}"))
            .await
        {
            failures.push(format!("force-stop {PACKAGE}: {error}"));
        }
        anyhow::ensure!(
            failures.is_empty(),
            "could not shut down Riviu helper transport on {}: {}",
            self.serial,
            failures.join("; ")
        );
        Ok(())
    }

    pub async fn is_alive(&self) -> bool {
        self.require_status().await.is_ok()
    }

    async fn require_status(&self) -> anyhow::Result<HelperStatus> {
        let response = self
            .http
            .get(format!("{}/status", self.base))
            .header(TOKEN_HEADER, helper_token(&self.serial))
            .send()
            .await
            .with_context(|| format!("GET {}/status", self.base))?;
        let status = response.status();
        let body = read_capped(response, "/status").await?;
        if !status.is_success() {
            anyhow::bail!(
                "Riviu helper trên {} trả HTTP {status} cho /status: {body}",
                self.serial
            );
        }
        parse_status(&body)
    }

    pub async fn set_clipboard(&self, content_type: &str, bytes: &[u8]) -> anyhow::Result<()> {
        require_plaintext(content_type)?;
        let text = std::str::from_utf8(bytes).context("clipboard text is not UTF-8")?;
        self.with_ime(|| async {
            let value: Value = self
                .post_json("/v1/clipboard/set", json!({ "text": text }))
                .await?;
            require_ok(&value, "set clipboard")?;
            Ok(())
        })
        .await
    }

    pub async fn get_clipboard(
        &self,
        maximum_decoded_bytes: usize,
    ) -> anyhow::Result<(String, Vec<u8>)> {
        riviu_core::device_capabilities::validate_clipboard_read_limit(maximum_decoded_bytes)?;
        self.with_ime(|| async {
            let value: Value = self.post_json("/v1/clipboard/get", json!({})).await?;
            require_ok(&value, "get clipboard")?;
            let text = value
                .get("text")
                .and_then(Value::as_str)
                .ok_or_else(|| anyhow!("helper clipboard get had no text field: {value}"))?;
            let bytes = text.as_bytes();
            if bytes.len() > maximum_decoded_bytes {
                anyhow::bail!(
                    "helper clipboard is {} bytes, limit is {maximum_decoded_bytes}",
                    bytes.len()
                );
            }
            Ok(("plaintext".to_string(), bytes.to_vec()))
        })
        .await
    }

    pub async fn import_media(
        &self,
        relative_path: &str,
        display_name: &str,
    ) -> anyhow::Result<MediaImport> {
        let value = self
            .post_json(
                "/v1/media/import",
                json!({
                    "relativePath": relative_path,
                    "displayName": display_name,
                }),
            )
            .await?;
        require_ok(&value, "media import")?;
        parse_media_import(&value)
    }

    /// Set the device wallpaper from a file already on the device (feature A3). The caller
    /// pushes the PNG to `device_path` first (e.g. `/data/local/tmp/...`).
    pub async fn set_wallpaper(&self, device_path: &str) -> anyhow::Result<()> {
        let value = self
            .post_json("/v1/wallpaper/set", json!({ "path": device_path }))
            .await?;
        require_ok(&value, "set wallpaper")?;
        Ok(())
    }

    /// Inject a mock GPS location (feature B). Requires the helper to be the selected
    /// mock-location app — the caller grants that with `appops set <pkg> android:mock_location
    /// allow` before the first call.
    pub async fn set_mock_location(&self, lat: f64, lng: f64) -> anyhow::Result<()> {
        let value = self
            .post_json("/v1/location/set", json!({ "lat": lat, "lng": lng }))
            .await?;
        require_ok(&value, "set mock location")?;
        Ok(())
    }

    /// Remove the mock-location test providers, so the device returns to its real GPS.
    pub async fn stop_mock_location(&self) -> anyhow::Result<()> {
        let value = self.post_json("/v1/location/stop", json!({})).await?;
        require_ok(&value, "stop mock location")?;
        Ok(())
    }

    /// Ask the phone what a list of packages is *called* and what they look like.
    ///
    /// The one question adb cannot answer: a label is a resource id needing the device's own
    /// locale, and no farm phone here has `aapt` (AGENTS.md §9.55). On the device it is one
    /// `PackageManager` call per app, so the whole fleet's app names cost one HTTP request per
    /// phone.
    ///
    /// `packages` is the list adb already gave the caller, so the helper never decides which
    /// apps exist — only what they are named and what icon they carry. An empty list asks the
    /// helper for everything the launcher would show, which is a different (narrower)
    /// question and is only useful to a caller that has no list of its own.
    pub async fn describe_apps(
        &self,
        packages: &[String],
        with_icons: bool,
    ) -> anyhow::Result<Vec<HelperApp>> {
        let value = self
            .post_json(
                "/v1/apps/describe",
                json!({ "packages": packages, "icons": with_icons }),
            )
            .await
            .map_err(|error| {
                // A helper from before this endpoint answers `not_found`, and the useful
                // sentence names the fix rather than the HTTP status.
                if format!("{error:#}").contains("not_found") {
                    anyhow!(
                        "Riviu helper trên {} quá cũ (chưa có /v1/apps/describe) — cài lại APK \
                         helper để lấy tên và icon app",
                        self.serial
                    )
                } else {
                    error
                }
            })?;
        require_ok(&value, "describe apps")?;
        parse_described_apps(&value)
    }

    async fn post_json(&self, path: &str, body: Value) -> anyhow::Result<Value> {
        let response = self
            .http
            .post(format!("{}{path}", self.base))
            .header(TOKEN_HEADER, helper_token(&self.serial))
            .json(&body)
            .send()
            .await
            .with_context(|| format!("POST {}{path}", self.base))?;
        let status = response.status();
        let text = read_capped(response, path).await?;
        let value: Value = serde_json::from_str(&text)
            .with_context(|| format!("helper {path} không phải JSON: {text}"))?;
        if !status.is_success() && value.get("ok") != Some(&Value::Bool(true)) {
            anyhow::bail!("helper {path} HTTP {status}: {text}");
        }
        Ok(value)
    }

    /// Switch to the helper IME, run `op`, always restore the previous IME.
    ///
    /// Refuses to switch when the current IME cannot be read or is not a legal
    /// id — same shape as an arrival check that cannot read a baseline.
    async fn with_ime<F, Fut, T>(&self, op: F) -> anyhow::Result<T>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = anyhow::Result<T>>,
    {
        let previous = current_ime(&self.adb, &self.serial).await?;
        set_ime(&self.adb, &self.serial, IME_ID).await?;
        // The IME service has to become current before ClipboardManager will
        // answer. 250 ms is a settle, not a proof; `/status` already proved the
        // process is up. A phone that still returns empty after this is a live
        // measurement, not something to paper over with a longer sleep.
        tokio::time::sleep(Duration::from_millis(250)).await;
        let outcome = op().await;
        let restore = set_ime(&self.adb, &self.serial, &previous).await;
        combine_ime_guard(outcome, restore)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HelperStatus {
    pub agent_version: String,
    pub protocol_version: u32,
    pub features: Vec<String>,
}

/// One app as the phone itself describes it: the name a person sees, and its icon.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HelperApp {
    pub package: String,
    pub label: String,
    pub system: bool,
    /// Base64 PNG, at the size the helper rendered. `None` when the icon could not be drawn
    /// (measured: a handful of system packages have none) or when the caller asked for no
    /// icons — never a placeholder, so the UI can tell "no icon" from "a grey square".
    pub icon_png_base64: Option<String>,
}

/// Read the `/v1/apps/describe` reply.
///
/// A row with no `package` is dropped rather than defaulted: the package name is the key the
/// desktop joins this onto its own listing by, and a row that cannot be joined is not a row.
pub fn parse_described_apps(value: &Value) -> anyhow::Result<Vec<HelperApp>> {
    let apps = value
        .get("apps")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("helper apps/describe had no apps array: {value}"))?;
    let mut out = Vec::with_capacity(apps.len());
    for row in apps {
        let Some(package) = row.get("package").and_then(Value::as_str) else {
            continue;
        };
        if package.is_empty() {
            continue;
        }
        out.push(HelperApp {
            package: package.to_string(),
            label: row
                .get("label")
                .and_then(Value::as_str)
                .unwrap_or(package)
                .to_string(),
            system: row.get("system").and_then(Value::as_bool).unwrap_or(false),
            icon_png_base64: row
                .get("icon")
                .and_then(Value::as_str)
                .filter(|icon| !icon.is_empty())
                .map(str::to_string),
        });
    }
    Ok(out)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaImport {
    pub id: String,
    pub pending_model: String,
}

/// `config → env → bundled`. Putting the bundled path in the configured field
/// would outrank `RIVIU_ANDROID_AGENT_APK` — the minicap trap in AGENTS.md §9.27.
pub fn resolve_apk_path(
    configured: Option<PathBuf>,
    env: Option<String>,
    bundled: Option<PathBuf>,
) -> Option<PathBuf> {
    configured
        .filter(|path| !path.as_os_str().is_empty())
        .or_else(|| {
            env.map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
                .map(PathBuf::from)
        })
        .or(bundled)
}

/// Read a helper response into a `String`, refusing anything over [`MAX_RESPONSE_BYTES`].
///
/// Streamed rather than `response.text()` so the cap is enforced *while* reading: `text()`
/// buffers the whole body first, which is the allocation being defended against, so checking
/// its length afterwards would be checking after the damage. Same shape as `wda.rs`.
async fn read_capped(response: reqwest::Response, what: &str) -> anyhow::Result<String> {
    let mut response = response;
    let mut bytes: Vec<u8> = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .with_context(|| format!("đọc {what} từ helper"))?
    {
        if bytes.len().saturating_add(chunk.len()) > MAX_RESPONSE_BYTES {
            anyhow::bail!("helper trả lời {what} quá {MAX_RESPONSE_BYTES} byte — đã cắt kết nối");
        }
        bytes.extend_from_slice(&chunk);
    }
    String::from_utf8(bytes).with_context(|| format!("{what} từ helper không phải UTF-8"))
}

pub fn parse_status(body: &str) -> anyhow::Result<HelperStatus> {
    let value: StatusWire = serde_json::from_str(body)
        .with_context(|| format!("helper /status is not the v1 object: {body}"))?;
    if !value.ok {
        anyhow::bail!("helper /status ok=false: {body}");
    }
    if value.protocol_version != PROTOCOL_VERSION {
        anyhow::bail!(
            "helper protocolVersion is {}, this build speaks {PROTOCOL_VERSION}",
            value.protocol_version
        );
    }
    Ok(HelperStatus {
        agent_version: value.agent_version,
        protocol_version: value.protocol_version,
        features: value.features,
    })
}

pub fn parse_media_import(value: &Value) -> anyhow::Result<MediaImport> {
    let id = value
        .get("id")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
        .ok_or_else(|| anyhow!("media import had no id: {value}"))?;
    if !id.bytes().all(|byte| byte.is_ascii_digit()) {
        anyhow::bail!("media import id must be digits, got {id:?}");
    }
    let pending_model = value
        .get("pendingModel")
        .and_then(Value::as_str)
        .unwrap_or("absent")
        .to_string();
    Ok(MediaImport {
        id: id.to_string(),
        pending_model,
    })
}

/// An IME id reaches `adb shell`, so it is code. Reject anything a shell would
/// act on; do not quote and hope.
pub fn validate_ime_id(id: &str) -> anyhow::Result<&str> {
    let invalid = || anyhow!("not a valid Android IME id: {id:?}");
    if id.is_empty() || id.len() > 255 {
        return Err(invalid());
    }
    let (package, class) = id.split_once('/').ok_or_else(invalid)?;
    if class.contains('/') {
        return Err(invalid());
    }
    adb::validate_package_name(package)?;
    let class_name = class.strip_prefix('.').unwrap_or(class);
    if class_name.is_empty() {
        return Err(invalid());
    }
    for segment in class_name.split('.') {
        let mut chars = segment.chars();
        match chars.next() {
            Some(first) if first.is_ascii_alphabetic() || first == '_' => {}
            _ => return Err(invalid()),
        }
        if !chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_') {
            return Err(invalid());
        }
    }
    Ok(id)
}

pub fn parse_current_ime(stdout: &str) -> Option<&str> {
    let line = stdout
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())?;
    if line.eq_ignore_ascii_case("null") || line.eq_ignore_ascii_case("none") {
        return None;
    }
    validate_ime_id(line).ok()
}

/// Combine the clipboard (or other) result with the IME restore.
///
/// A successful op that leaves our IME as the default is a product defect —
/// that is GenFarmer's mark. The restore error wins in that case so the
/// operator sees it. A failed op still restores; the op error stays primary.
pub fn combine_ime_guard<T>(
    outcome: anyhow::Result<T>,
    restore: anyhow::Result<()>,
) -> anyhow::Result<T> {
    match (outcome, restore) {
        (Ok(value), Ok(())) => Ok(value),
        (Ok(_), Err(restore)) => Err(restore).context(
            "clipboard (or helper) succeeded but the previous keyboard was not restored — \
             the phone may still be on com.riviu.agent/.RiviuIme",
        ),
        (Err(error), Ok(())) => Err(error),
        (Err(error), Err(restore)) => Err(error).context(format!(
            "also failed to restore the previous keyboard: {restore:#}"
        )),
    }
}

pub fn clipboard_unavailable(serial: &str) -> String {
    format!(
        "Riviu helper is not running on {serial}, so clipboard is unsupported. \
         This is not advertised through uiautomator2 (that route returns empty on \
         Android 10+). Install com.riviu.agent or set RIVIU_ANDROID_AGENT_APK."
    )
}

fn require_plaintext(content_type: &str) -> anyhow::Result<()> {
    if content_type.is_empty() || content_type.eq_ignore_ascii_case("plaintext") {
        return Ok(());
    }
    anyhow::bail!("Riviu helper only stores plaintext, not {content_type:?}")
}

fn require_ok(value: &Value, what: &str) -> anyhow::Result<()> {
    if value.get("ok") == Some(&Value::Bool(true)) {
        return Ok(());
    }
    anyhow::bail!("helper {what} failed: {value}")
}

async fn package_installed(adb: &AdbProgram, serial: &str) -> anyhow::Result<bool> {
    let listing = adb
        .shell(serial, &format!("pm path {PACKAGE}"))
        .await
        .unwrap_or_default();
    Ok(listing.contains("package:"))
}

async fn install_apk(adb: &AdbProgram, serial: &str, apk: &Path) -> anyhow::Result<()> {
    let path = apk
        .to_str()
        .ok_or_else(|| anyhow!("the helper APK path is not UTF-8"))?;
    if !apk.is_file() {
        anyhow::bail!("helper APK is not a file: {}", apk.display());
    }
    let output = match adb
        .device(serial, &["install", "-r", "-g", path], INSTALL_TIMEOUT)
        .await
    {
        Ok(output) => output,
        Err(error) => {
            let text = format!("{error:#}");
            if text.contains("INSTALL_FAILED_USER_RESTRICTED") {
                anyhow::bail!("{}", miui_install_refused(serial));
            }
            return Err(error).context(format!(
                "install {} on {serial}. On MIUI/HyperOS a refusal is often \
                 INSTALL_FAILED_USER_RESTRICTED until Developer options → \
                 Cài đặt qua USB is on — that is policy, not a bad APK",
                apk.display()
            ));
        }
    };
    if output.contains("INSTALL_FAILED_USER_RESTRICTED") {
        anyhow::bail!("{}", miui_install_refused(serial));
    }
    Ok(())
}

fn miui_install_refused(serial: &str) -> String {
    format!(
        "MIUI/HyperOS refused to install com.riviu.agent on {serial} \
         (INSTALL_FAILED_USER_RESTRICTED). Turn on Developer options → \
         Cài đặt qua USB. Do not retry adb install / pm install / \
         install-create — all three fail the same way (AGENTS.md §9)."
    )
}

async fn enable_ime(adb: &AdbProgram, serial: &str) -> anyhow::Result<()> {
    adb.shell(serial, &format!("ime enable {IME_ID}"))
        .await
        .map(|_| ())
        .with_context(|| format!("ime enable {IME_ID} on {serial}"))
}

/// Start the helper service, handing it the token it must then demand on every request.
///
/// The token goes as an Intent extra rather than a file or a property: it reaches exactly one
/// process, leaves nothing behind on the device, and a helper started by anyone *else* — which
/// an exported service always allows — comes up with no token and therefore serves nothing.
async fn start_service(adb: &AdbProgram, serial: &str) -> anyhow::Result<()> {
    let token = helper_token(serial);
    // Token is hex from `Uuid::simple`, so it needs no quoting; asserted rather than assumed,
    // because this string is pasted into a device shell command.
    debug_assert!(token.chars().all(|c| c.is_ascii_hexdigit()));
    adb.shell(
        serial,
        &format!("am start-foreground-service -n {SERVICE} --es token {token}"),
    )
    .await
    .map(|_| ())
    .with_context(|| format!("start {SERVICE} on {serial}"))
}

async fn current_ime(adb: &AdbProgram, serial: &str) -> anyhow::Result<String> {
    let stdout = adb
        .shell(serial, "settings get secure default_input_method")
        .await
        .context("read default_input_method")?;
    parse_current_ime(&stdout)
        .map(str::to_string)
        .ok_or_else(|| {
            anyhow!(
                "cannot read the current IME on {serial} ({stdout:?}) — \
                 refusing to switch, because restore would have no target"
            )
        })
}

async fn set_ime(adb: &AdbProgram, serial: &str, ime: &str) -> anyhow::Result<()> {
    let ime = validate_ime_id(ime)?;
    adb.shell(serial, &format!("ime set {ime}"))
        .await
        .map(|_| ())
        .with_context(|| format!("ime set {ime} on {serial}"))
}

async fn forward_helper(adb: &AdbProgram, serial: &str) -> anyhow::Result<u16> {
    let remote = format!("tcp:{DEVICE_PORT}");
    prune_helper_forwards(adb, serial).await;
    adb.device(
        serial,
        &["forward", "tcp:0", &remote],
        Duration::from_secs(30),
    )
    .await
    .with_context(|| format!("forward tcp:0 to {remote} on {serial}"))?;
    let listing = adb
        .run(&["forward", "--list"], Duration::from_secs(30))
        .await
        .context("list adb forwards")?;
    frames::parse_forward_port(&listing, serial, &remote).ok_or_else(|| {
        anyhow!("adb reported no helper forward for {serial} -> {remote}; listing was {listing:?}")
    })
}

async fn prune_helper_forwards(adb: &AdbProgram, serial: &str) -> usize {
    let remote = format!("tcp:{DEVICE_PORT}");
    let listing = match adb
        .run(&["forward", "--list"], Duration::from_secs(30))
        .await
    {
        Ok(listing) => listing,
        Err(_) => return 0,
    };
    let stale = frames::parse_forward_ports(&listing, serial, &remote);
    let mut removed = 0;
    for port in stale {
        if frames::remove_forward(adb, serial, port).await.is_ok() {
            removed += 1;
        }
    }
    if removed > 0 {
        tracing::info!(serial, removed, "reclaimed stale Riviu helper forwards");
    }
    removed
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StatusWire {
    ok: bool,
    agent_version: String,
    protocol_version: u32,
    #[serde(default)]
    features: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_v1_is_accepted() {
        let status = parse_status(
            r#"{"ok":true,"agentVersion":"0.1.0","protocolVersion":1,"features":["clipboard","pushMedia"]}"#,
        )
        .expect("status");
        assert_eq!(status.protocol_version, 1);
        assert_eq!(status.agent_version, "0.1.0");
        assert_eq!(status.features, ["clipboard", "pushMedia"]);
    }

    /// The real reply, trimmed from `POST /v1/apps/describe` on 23021RAAEG (Android 15,
    /// helper 0.3.0, 21/08/2026) — labels the phone resolved off `PackageManager`, which is
    /// the whole reason this endpoint exists.
    #[test]
    fn described_apps_carry_the_names_the_phone_resolved() {
        let value: Value = serde_json::from_str(
            r#"{"ok":true,"apps":[
                {"package":"com.kakaopay.app","label":"kakaopay","system":false},
                {"package":"com.gojek.gopay","label":"GoPay","system":false,"icon":"iVBORw0KGgo="},
                {"package":"com.android.settings","label":"Cài đặt","system":true}
            ],"iconPx":48,"iconsTruncated":0}"#,
        )
        .expect("wire");
        let apps = parse_described_apps(&value).expect("parse");
        assert_eq!(apps.len(), 3);
        assert_eq!(apps[0].label, "kakaopay");
        assert_eq!(apps[0].icon_png_base64, None);
        assert_eq!(apps[1].icon_png_base64.as_deref(), Some("iVBORw0KGgo="));
        assert!(
            apps[2].system,
            "the system partition is flagged, not hidden"
        );
    }

    #[test]
    fn a_row_with_no_package_is_dropped_because_nothing_can_be_joined_onto_it() {
        // The package name is the key the desktop joins this onto its own adb listing by, so
        // a row without one is not a row — and defaulting it to "" would attach a stranger's
        // name to whichever app sorted first.
        let value: Value = serde_json::from_str(
            r#"{"ok":true,"apps":[{"label":"ghost"},{"package":"","label":"also ghost"},
                {"package":"com.real.app"}]}"#,
        )
        .expect("wire");
        let apps = parse_described_apps(&value).expect("parse");
        assert_eq!(apps.len(), 1);
        // No label of its own: falls back to the package name rather than to empty text.
        assert_eq!(apps[0].label, "com.real.app");
    }

    #[test]
    fn a_reply_without_an_apps_array_is_an_error_not_an_empty_fleet() {
        let value: Value = serde_json::from_str(r#"{"ok":true}"#).expect("wire");
        let error = parse_described_apps(&value).expect_err("no apps array");
        assert!(error.to_string().contains("apps"), "{error}");
    }

    #[test]
    fn a_newer_protocol_is_refused_rather_than_half_read() {
        let error =
            parse_status(r#"{"ok":true,"agentVersion":"9.0.0","protocolVersion":2,"features":[]}"#)
                .expect_err("v2");
        assert!(error.to_string().contains("protocolVersion"), "{error}");
    }

    #[test]
    fn status_ok_false_is_refused() {
        let error = parse_status(
            r#"{"ok":false,"agentVersion":"0.1.0","protocolVersion":1,"features":[]}"#,
        )
        .expect_err("ok");
        assert!(error.to_string().contains("ok=false"), "{error}");
    }

    #[test]
    fn media_import_requires_a_digit_id() {
        let ok = parse_media_import(&json!({"ok":true,"id":"1000011143","pendingModel":"cleared"}))
            .expect("id");
        assert_eq!(ok.id, "1000011143");
        assert_eq!(ok.pending_model, "cleared");
        assert!(parse_media_import(&json!({"ok":true,"id":"../x"})).is_err());
        assert!(parse_media_import(&json!({"ok":true,"id":""})).is_err());
    }

    #[test]
    fn ime_ids_from_the_fleet_parse_and_injections_do_not() {
        assert_eq!(
            validate_ime_id("com.android.inputmethod.latin/.LatinIME").unwrap(),
            "com.android.inputmethod.latin/.LatinIME"
        );
        assert!(validate_ime_id(
            "com.google.android.inputmethod.latin/com.android.inputmethod.latin.LatinIME"
        )
        .is_ok());
        assert!(validate_ime_id(IME_ID).is_ok());
        assert!(validate_ime_id("com.foo/.Bar; rm -rf /sdcard").is_err());
        assert!(validate_ime_id("com.foo/.Bar && reboot").is_err());
        assert!(validate_ime_id("latin").is_err());
        assert!(validate_ime_id("").is_err());
    }

    #[test]
    fn current_ime_ignores_null_and_rejects_junk() {
        assert_eq!(
            parse_current_ime("com.android.inputmethod.latin/.LatinIME\n"),
            Some("com.android.inputmethod.latin/.LatinIME")
        );
        assert_eq!(parse_current_ime("null\n"), None);
        assert_eq!(parse_current_ime("\n"), None);
        assert_eq!(parse_current_ime("com.foo/.Bar; reboot\n"), None);
    }

    #[test]
    fn a_successful_op_that_fails_to_restore_is_an_error() {
        let error = combine_ime_guard::<()>(Ok(()), Err(anyhow!("ime set failed"))).unwrap_err();
        let text = format!("{error:#}");
        assert!(text.contains("not restored"), "{text}");
        assert!(text.contains("ime set failed"), "{text}");
    }

    #[test]
    fn a_failed_op_keeps_its_error_when_restore_works() {
        let error = combine_ime_guard::<()>(Err(anyhow!("empty clip")), Ok(())).unwrap_err();
        assert_eq!(error.to_string(), "empty clip");
    }

    #[test]
    fn both_failures_keep_the_op_and_name_the_restore() {
        let error =
            combine_ime_guard::<()>(Err(anyhow!("empty clip")), Err(anyhow!("ime set failed")))
                .unwrap_err();
        let text = format!("{error:#}");
        assert!(text.contains("empty clip"), "{text}");
        assert!(text.contains("ime set failed"), "{text}");
    }

    #[test]
    fn bundled_apk_loses_to_env_and_env_loses_to_config() {
        let bundled = PathBuf::from("bundled.apk");
        let env = Some("  env.apk  ".to_string());
        let configured = Some(PathBuf::from("config.apk"));
        assert_eq!(
            resolve_apk_path(configured.clone(), env.clone(), Some(bundled.clone())),
            Some(PathBuf::from("config.apk"))
        );
        assert_eq!(
            resolve_apk_path(None, env, Some(bundled.clone())),
            Some(PathBuf::from("env.apk"))
        );
        assert_eq!(
            resolve_apk_path(None, Some(String::new()), Some(bundled.clone())),
            Some(bundled)
        );
        assert_eq!(resolve_apk_path(None, None, None), None);
    }

    #[test]
    fn clipboard_unavailable_names_the_phone_and_refuses_uiautomator2() {
        let text = clipboard_unavailable("10969614");
        assert!(text.contains("10969614"), "{text}");
        assert!(text.contains("uiautomator2"), "{text}");
        assert!(text.contains("RIVIU_ANDROID_AGENT_APK"), "{text}");
    }

    #[test]
    fn a_miui_refusal_names_the_phone_and_forbids_the_three_install_paths() {
        let text = miui_install_refused("10969614");
        assert!(text.contains("10969614"), "{text}");
        assert!(text.contains("INSTALL_FAILED_USER_RESTRICTED"), "{text}");
        assert!(text.contains("adb install"), "{text}");
        assert!(text.contains("pm install"), "{text}");
        assert!(text.contains("install-create"), "{text}");
    }
}
