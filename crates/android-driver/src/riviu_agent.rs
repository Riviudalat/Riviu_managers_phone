//! HTTP client for the Riviu helper APK (`com.riviu.agent`).
//!
//! This is **not** the uiautomator2 session in [`crate::agent`]. That server
//! drives taps, the tree, and `ACTION_SET_TEXT`. This one exists for the two
//! things uiautomator2 cannot honestly do:
//!
//! * clipboard read on Android 10+ (the Appium route returns empty; advertising
//!   that as success is the lie AGENTS.md §9 forbids);
//! * MediaStore insert from an app UID, so `is_pending` starts clearable.
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
pub const AGENT_VERSION: &str = "0.1.0";

const HTTP_TIMEOUT: Duration = Duration::from_secs(10);
const INSTALL_TIMEOUT: Duration = Duration::from_secs(300);

/// One helper connection, cheap to clone (shared HTTP client + serial).
#[derive(Clone)]
pub struct HelperClient {
    http: reqwest::Client,
    adb: AdbProgram,
    serial: String,
    base: String,
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
        client.require_status().await?;
        Ok(client)
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
        })
    }

    pub async fn is_alive(&self) -> bool {
        self.require_status().await.is_ok()
    }

    async fn require_status(&self) -> anyhow::Result<HelperStatus> {
        let response = self
            .http
            .get(format!("{}/status", self.base))
            .send()
            .await
            .with_context(|| format!("GET {}/status", self.base))?;
        let status = response.status();
        let body = response
            .text()
            .await
            .with_context(|| format!("đọc /status từ helper trên {}", self.serial))?;
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

    pub async fn delete_media(&self, id: &str) -> anyhow::Result<()> {
        let value = self
            .post_json("/v1/media/delete", json!({ "id": id }))
            .await?;
        require_ok(&value, "media delete")?;
        Ok(())
    }

    async fn post_json(&self, path: &str, body: Value) -> anyhow::Result<Value> {
        let response = self
            .http
            .post(format!("{}{path}", self.base))
            .json(&body)
            .send()
            .await
            .with_context(|| format!("POST {}{path}", self.base))?;
        let status = response.status();
        let text = response
            .text()
            .await
            .with_context(|| format!("đọc {path}"))?;
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

async fn start_service(adb: &AdbProgram, serial: &str) -> anyhow::Result<()> {
    adb.shell(serial, &format!("am start-foreground-service -n {SERVICE}"))
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
