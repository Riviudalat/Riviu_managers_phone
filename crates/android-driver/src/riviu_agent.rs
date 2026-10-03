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
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::adb::{self, AdbProgram};
use crate::frames;

mod runtime;
pub use runtime::HelperRecoveryRequired;

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
pub const AGENT_VERSION: &str = "0.5.0";

/// What this build needs the installed helper to advertise on `/status`.
///
/// Features and not a version number, deliberately: the question is never "is it 0.3.0", it is
/// "can it answer the call I am about to make", and a phone can legitimately carry a newer
/// APK than this build knows about. A helper missing any of these is reinstalled once — see
/// [`HelperClient::upgrade_if_stale`].
const REQUIRED_FEATURES: &[&str] = &["clipboard", "pushMedia", "appLabels", "auth", "launcher"];

/// Serials this process has already tried to upgrade, so a stale APK on disk cannot turn
/// every helper call into another install attempt.
fn upgrade_attempts() -> &'static parking_lot::Mutex<std::collections::HashSet<String>> {
    static ATTEMPTED: std::sync::OnceLock<parking_lot::Mutex<std::collections::HashSet<String>>> =
        std::sync::OnceLock::new();
    ATTEMPTED.get_or_init(Default::default)
}

/// Header the helper's shared token travels in. Mirrors `HttpServer.TOKEN_HEADER`.
pub const TOKEN_HEADER: &str = "X-Riviu-Token";

fn helper_tokens() -> &'static parking_lot::Mutex<std::collections::HashMap<String, String>> {
    static TOKENS: std::sync::OnceLock<
        parking_lot::Mutex<std::collections::HashMap<String, String>>,
    > = std::sync::OnceLock::new();
    TOKENS.get_or_init(Default::default)
}

/// Legacy/debug credentials remain per-serial RAM state. Production owners use
/// an immutable client credential persisted under their exact identity in the OS vault.
fn helper_token(serial: &str) -> String {
    let mut tokens = helper_tokens().lock();
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

// Old non-persisted intents may reuse only an existing process credential.
// Never manufacture a replacement while resuming an unresolved owner.
fn existing_helper_token(serial: &str) -> Option<String> {
    helper_tokens().lock().get(serial).cloned()
}

/// One helper request, over `adb forward` to loopback on the phone.
///
/// Everything on this path is small and local — clipboard text, an app label, a wallpaper
/// path — so ten seconds is already far past working. It exists to bound the case the port
/// is held by something that accepted the connection and then said nothing, which
/// `adb forward` makes reachable.
const HTTP_TIMEOUT: Duration = Duration::from_secs(10);

fn ime_lock(serial: &str) -> std::sync::Arc<tokio::sync::Mutex<()>> {
    static LOCKS: std::sync::OnceLock<
        parking_lot::Mutex<
            std::collections::HashMap<String, std::sync::Arc<tokio::sync::Mutex<()>>>,
        >,
    > = std::sync::OnceLock::new();
    LOCKS
        .get_or_init(Default::default)
        .lock()
        .entry(serial.into())
        .or_default()
        .clone()
}

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
    token: std::sync::Arc<str>,
    adb: AdbProgram,
    serial: String,
    base: String,
    host_port: u16,
    lifecycle: std::sync::Arc<tokio::sync::Mutex<bool>>,
    pending_clipboard: std::sync::Arc<parking_lot::Mutex<Option<Value>>>,
    canary: Option<CanaryOwner>,
    production_runtime: bool,
    clipboard_qualified: std::sync::Arc<std::sync::atomic::AtomicBool>,
    clipboard_baseline: std::sync::Arc<parking_lot::Mutex<Option<ClipboardBaseline>>>,
}

/// None text is an absent native clip; Some(empty bytes) is a plaintext empty string.
#[derive(Clone)]
struct ClipboardBaseline {
    id: String,
    text: Option<Vec<u8>>,
}

impl ClipboardBaseline {
    fn capture(value: &Value) -> anyhow::Result<Self> {
        let text = clipboard_read_payload(value, 4096)?;
        let id = value["baselineId"].as_str().filter(|id| !id.is_empty())
            .context("clipboard snapshot identity missing; no SET")?.to_owned();
        Ok(Self { id, text })
    }

    fn restored(&self, value: &Value) -> bool {
        value["written"] == true && value["verified"] == true
            && value["available"] == self.text.is_some()
    }
}

/// A successful job must explicitly prove absence; unavailable or malformed reads fail closed.
fn clipboard_read_payload(value: &Value, limit: usize) -> anyhow::Result<Option<Vec<u8>>> {
    let kind = match value["baselineKind"].as_str() {
        Some("empty") => "empty",
        Some("plaintext") => "plaintext",
        Some("unsupported") => "unsupported",
        _ => "unknown",
    };
    if kind == "empty" && value["available"] == false && value["plainTextBaseline"] == false
        && value.get("text").is_none() {
        return Ok(None);
    }
    anyhow::ensure!((kind == "plaintext" || (kind == "unknown" && value.get("baselineKind").is_none())) && value["available"] == true
        && value["plainTextBaseline"] == true,
        "supported clipboard baseline unavailable; baselineKind={kind} available={} plainTextBaseline={} snapshotRetained={}; no SET",
        value["available"] == true, value["plainTextBaseline"] == true, value["baselineId"].as_str().is_some());
    let text = value["text"].as_str().context("clipboard plaintext missing; empty was not proved")?;
    anyhow::ensure!(text.len() <= limit, "helper clipboard exceeded read limit");
    Ok(Some(text.as_bytes().to_vec()))
}

#[derive(Clone)]
struct CanaryOwner {
    owner_id: String,
    instance: String,
    generation: String,
    socket: String,
    nonce: String,
    uid: u32,
    apk_path: String,
    report: PathBuf,
}

impl HelperClient {
    pub(crate) fn is_scoped_canary(&self) -> bool { self.canary.is_some() && !self.lifecycle.try_lock().map(|closed| *closed).unwrap_or(true) }
    /// A closed clone is replaceable only after all owned cleanup is proved complete.
    pub(crate) async fn is_released(&self) -> anyhow::Result<bool> {
        let closed = self.lifecycle.lock().await;
        if !*closed { return Ok(false); }
        anyhow::ensure!(!self.cleanup_is_pending(), "helper cleanup pending; retain released cache for reconciliation");
        if self.production_runtime {
            let owner = self.canary.as_ref().context("released runtime owner missing")?;
            anyhow::ensure!(!owner.report.join("runtime-owner.json").try_exists()?, "helper owner intent retained; no fresh claim");
        }
        if let Some(owner) = &self.canary {
            anyhow::ensure!(!owner.report.join("ime-checkpoint.json").try_exists()?, "helper cleanup checkpoint retained; no fresh claim");
        }
        Ok(true)
    }
    pub(crate) fn cleanup_is_pending(&self) -> bool {
        self.pending_clipboard.lock().is_some() || self.clipboard_baseline.lock().is_some()
    }

    /// Read only the retained protected owner under the same IME/lifecycle locks
    /// as clipboard settlement. No claim, release, IME switch, or restoration.
    pub(crate) async fn verify_owned_cleanup_ready(
        &self,
        serial: &str,
        session_epoch: &str,
    ) -> anyhow::Result<riviu_core::driver::OwnedSessionCleanupProof> {
        anyhow::ensure!(
            serial == self.serial && !session_epoch.is_empty(),
            "helper cleanup serial/session binding missing"
        );
        let _serial = ime_lock(&self.serial).lock_owned().await;
        let closed = self.lifecycle.lock().await;
        anyhow::ensure!(
            !*closed && self.production_runtime,
            "retained production helper unavailable"
        );
        anyhow::ensure!(
            !self.cleanup_is_pending(),
            "helper clipboard/baseline cleanup unresolved"
        );
        let owner = self
            .canary
            .as_ref()
            .context("retained helper owner missing")?;
        let checkpoint = owner.report.join("ime-checkpoint.json");
        if checkpoint.try_exists()? {
            let bytes = std::fs::read(checkpoint)?;
            anyhow::ensure!(
                bytes.len() <= 16 * 1024,
                "helper cleanup checkpoint exceeds bound"
            );
            let saved: Value = serde_json::from_slice(&bytes)?;
            anyhow::ensure!(
                saved["serial"].as_str() == Some(serial)
                    && saved["androidUser"] == 0
                    && saved["ownerId"].as_str() == Some(owner.owner_id.as_str())
                    && saved["serviceInstance"].as_str() == Some(owner.instance.as_str())
                    && saved["generation"].as_str() == Some(owner.generation.as_str())
                    && saved.get("pending").is_some_and(Value::is_null)
                    && saved.get("baseline").is_some_and(Value::is_null),
                "helper durable cleanup unresolved or identity changed"
            );
        }
        let nonce = uuid::Uuid::new_v4().simple().to_string();
        let value = self
            .post_json_open("/v1/session/status", json!({"nonce":nonce}))
            .await?;
        anyhow::ensure!(
            value["ok"] == true
                && value["nonce"].as_str() == Some(nonce.as_str())
                && value["serviceInstance"].as_str() == Some(owner.instance.as_str())
                && value["ownerId"].as_str() == Some(owner.owner_id.as_str())
                && value["ownerGeneration"].as_str() == Some(owner.generation.as_str())
                && value["ownership"] == "owned",
            "helper owner identity changed"
        );
        anyhow::ensure!(
            !self.cleanup_is_pending(),
            "helper cleanup changed during owner proof"
        );
        Ok(riviu_core::driver::OwnedSessionCleanupProof {
            udid: serial.to_owned(),
            session_epoch: session_epoch.to_owned(),
            helper_owner_id: owner.owner_id.clone(),
            helper_instance: owner.instance.clone(),
            helper_generation: owner.generation.clone(),
            input_sealed: true,
            clipboard_pending: false,
            baseline_pending: false,
        })
    }
    /// Read-only diagnostic transport using non-secret bytes; no helper service request.
    pub async fn probe_stdin_transport(adb: &AdbProgram, serial: &str) -> anyhow::Result<()> {
        let marker = b"riviu-bootstrap-stdin-check".to_vec();
        let reply = adb
            .shell_secret_input(serial, "cat", marker.clone(), Duration::from_secs(10))
            .await?;
        anyhow::ensure!(
            reply == marker,
            "ADB stdin framing transport does not round-trip"
        );
        Ok(())
    }

    /// Inspect existing bootstrap admission with an invalid action; this APK returns identity
    /// but performs no claim/release for it. Never starts/restarts a service or sends credentials.
    pub async fn inspect_existing_bootstrap(
        adb: &AdbProgram,
        serial: &str,
        intent: &Value,
    ) -> anyhow::Result<Value> {
        let dump = adb.shell(serial, "dumpsys package com.riviu.agent").await?;
        let uid = dump
            .lines()
            .find_map(|line| {
                line.trim()
                    .strip_prefix("userId=")
                    .and_then(|s| s.split_whitespace().next())
                    .and_then(|s| s.parse::<u32>().ok())
            })
            .context("helper UID unavailable")?;
        let paths = adb.shell(serial, "pm path com.riviu.agent").await?;
        let apk = paths
            .lines()
            .find_map(|line| line.trim().strip_prefix("package:"))
            .context("helper path unavailable")?;
        crate::adb::validate_device_path(apk)?;
        let socket = intent["socket"].as_str().context("scoped socket missing")?;
        let nonce = intent["nonce"].as_str().context("scoped nonce missing")?;
        let owner = intent["ownerId"].as_str().context("scoped owner missing")?;
        anyhow::ensure!(
            socket.starts_with("riviu-bootstrap-")
                && socket
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-'),
            "invalid diagnostic socket"
        );
        let reply = bootstrap_exchange(adb, serial, apk, socket, uid, json!({"action":"inspect_invalid_readonly","nonce":nonce,"ownerId":owner,"token":"readonly-marker-not-a-credential-0000"})).await?;
        anyhow::ensure!(
            reply["nonce"].as_str() == Some(nonce)
                && reply["ownerId"].as_str() == Some(owner)
                && reply["ok"] == false
                && reply["state"] == "invalid_action",
            "read-only bootstrap identity unavailable"
        );
        Ok(reply)
    }

    /// Opens a new nonce-bound observation socket, not a new owner/token/session.
    pub async fn inspect_bootstrap_owner_readonly(
        adb: &AdbProgram,
        serial: &str,
    ) -> anyhow::Result<Value> {
        let dump = adb.shell(serial, "dumpsys package com.riviu.agent").await?;
        let uid = dump
            .lines()
            .find_map(|line| {
                line.trim()
                    .strip_prefix("userId=")
                    .and_then(|s| s.split_whitespace().next())
                    .and_then(|s| s.parse::<u32>().ok())
            })
            .context("helper UID unavailable")?;
        let paths = adb.shell(serial, "pm path com.riviu.agent").await?;
        let apk = paths
            .lines()
            .find_map(|line| line.trim().strip_prefix("package:"))
            .context("helper path unavailable")?;
        crate::adb::validate_device_path(apk)?;
        let owner = uuid::Uuid::new_v4().simple().to_string();
        let nonce = uuid::Uuid::new_v4().simple().to_string();
        let socket = format!("riviu-bootstrap-{owner}");
        let started_at = std::time::Instant::now();
        let started = adb.shell_output(serial, &format!("am start-foreground-service -n {SERVICE} --es bootstrapSocket {socket} --es bootstrapNonce {nonce} --es bootstrapOwnerId {owner}"), Duration::from_secs(15)).await?;
        anyhow::ensure!(
            started.exit_code == 0
                && !started.stdout.contains("Error")
                && !started.stderr.contains("Error"),
            "owner observation listener rejected"
        );
        let start_ms = started_at.elapsed().as_millis();
        let reply = bootstrap_exchange(adb, serial, apk, &socket, uid, json!({"action":"inspect_invalid_readonly","nonce":nonce,"ownerId":owner,"token":"readonly-marker-not-a-credential-0000"})).await
            .with_context(||format!("read-only bootstrap failed after service-start {start_ms}ms"))?;
        anyhow::ensure!(
            reply["nonce"].as_str() == Some(nonce.as_str())
                && reply["ownerId"].as_str() == Some(owner.as_str())
                && reply["ok"] == false
                && reply["state"] == "invalid_action",
            "owner observation reply mismatch"
        );
        Ok(reply)
    }

    pub async fn diagnose_bootstrap_readonly(
        adb: &AdbProgram,
        serial: &str,
        jar: &Path,
        hash: &str,
    ) -> anyhow::Result<Value> {
        anyhow::ensure!(
            riviu_core::frame_sha256(&std::fs::read(jar)?) == hash,
            "probe jar changed"
        );
        let dump = adb.shell(serial, "dumpsys package com.riviu.agent").await?;
        let uid = dump
            .lines()
            .find_map(|line| {
                line.trim()
                    .strip_prefix("userId=")
                    .and_then(|s| s.split_whitespace().next())
                    .and_then(|s| s.parse::<u32>().ok())
            })
            .context("helper UID unavailable")?;
        let owner = uuid::Uuid::new_v4().simple().to_string();
        let nonce = uuid::Uuid::new_v4().simple().to_string();
        let socket = format!("riviu-bootstrap-{owner}");
        let remote = format!("/data/local/tmp/riviu-probe-{owner}.jar");
        let absent = adb
            .shell(serial, &format!("test ! -e {remote} && printf absent"))
            .await?;
        anyhow::ensure!(absent == "absent", "probe path already exists");
        adb.device(
            serial,
            &["push", jar.to_str().context("probe path invalid")?, &remote],
            Duration::from_secs(30),
        )
        .await?;
        let probe = async {
            let observed = adb.shell(serial, &format!("sha256sum {remote}")).await?;
            anyhow::ensure!(observed.split_whitespace().next() == Some(hash), "probe device hash mismatched");
            let marker = b"non-secret-be32-roundtrip";
            let mut payload = (marker.len() as u32).to_be_bytes().to_vec(); payload.extend(marker);
            let stdout = adb.shell_secret_input(serial, &format!("CLASSPATH={remote} app_process /system/bin com.riviu.agent.ProbeBootstrap --stdin-only"), payload, Duration::from_secs(12)).await?;
            anyhow::ensure!(stdout.len() >= 4, "probe stdin frame missing");
            let input: Value = serde_json::from_slice(&stdout[4..])?;
            anyhow::ensure!(input["sha256"].as_str() == Some(riviu_core::frame_sha256(marker).as_str()), "BE32 input mismatch");
            let started = adb.shell_output(serial, &format!("am start-foreground-service -n {SERVICE} --es bootstrapSocket {socket} --es bootstrapNonce {nonce} --es bootstrapOwnerId {owner}"), Duration::from_secs(15)).await?;
            anyhow::ensure!(started.exit_code == 0, "diagnostic observation listener rejected");
            let sockets = adb.shell(serial, "cat /proc/net/unix").await?;
            anyhow::ensure!(sockets.lines().any(|line|line.ends_with(&format!("@{socket}"))), "fresh bootstrap socket absent immediately after service start");
            let body = serde_json::to_vec(&json!({"action":"inspect_invalidaction","token":"xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx","nonce":nonce,"ownerId":owner}))?;
            let mut payload = (body.len() as u32).to_be_bytes().to_vec(); payload.extend(body);
            let reply = adb.shell_secret_input(serial, &format!("CLASSPATH={remote} app_process /system/bin com.riviu.agent.ProbeBootstrap {socket} {uid}"),payload,Duration::from_secs(12)).await?;
            anyhow::ensure!(reply.len() >= 4, "diagnostic owner frame missing");
            let value: Value = serde_json::from_slice(&reply[4..])?;
            anyhow::ensure!(value["ok"] == false && value["state"] == "invalid_action" && value["nonce"].as_str() == Some(nonce.as_str()), "diagnostic owner reply mismatch");
            Ok::<_,anyhow::Error>(value)
        }.await;
        let cleanup = adb
            .shell(
                serial,
                &format!("rm -- {remote}; test ! -e {remote} && printf removed"),
            )
            .await?;
        anyhow::ensure!(cleanup.trim() == "removed", "owned probe cleanup unproved");
        probe
    }

    /// Called only by the admitted helper-canary facade while its device lease is held.
    pub(crate) async fn prepare_canary(
        adb: AdbProgram,
        serial: &str,
        report: PathBuf,
    ) -> anyhow::Result<Self> {
        anyhow::ensure!(
            adb.shell(serial, "id -u").await?.trim() == "2000",
            "helper bootstrap canary requires ADB shell UID2000; no root fallback"
        );
        let package = adb.shell(serial, "dumpsys package com.riviu.agent").await?;
        let uid = package
            .lines()
            .find_map(|line| {
                line.trim()
                    .strip_prefix("userId=")
                    .and_then(|s| s.split_whitespace().next())
                    .and_then(|s| s.parse::<u32>().ok())
            })
            .context("helper package UID unreadable")?;
        anyhow::ensure!(uid >= 10000 && uid < 100000, "helper package UID invalid");
        let run_as_uid = adb
            .shell_output(
                serial,
                "run-as com.riviu.agent id -u",
                Duration::from_secs(10),
            )
            .await?;
        anyhow::ensure!(
            run_as_uid.exit_code == 0 && run_as_uid.stdout.trim().parse::<u32>().ok() == Some(uid),
            "debug helper UID carrier unavailable; no root/SELinux fallback"
        );
        let path = adb.shell(serial, "pm path com.riviu.agent").await?;
        let paths: Vec<_> = path
            .lines()
            .filter_map(|line| line.trim().strip_prefix("package:"))
            .collect();
        anyhow::ensure!(paths.len() == 1, "helper APK path ambiguous");
        let apk_path = paths[0].to_owned();
        crate::adb::validate_device_path(&apk_path)?;
        let owner_id = uuid::Uuid::new_v4().simple().to_string();
        let nonce = uuid::Uuid::new_v4().simple().to_string();
        let socket = format!("riviu-bootstrap-{owner_id}");
        let started = adb.shell_output(serial, &format!("am start-foreground-service -n {SERVICE} --es bootstrapSocket {socket} --es bootstrapNonce {nonce} --es bootstrapOwnerId {owner_id}"), Duration::from_secs(15)).await?;
        anyhow::ensure!(
            started.exit_code == 0
                && !started.stdout.contains("Error")
                && !started.stderr.contains("Error"),
            "helper bootstrap start rejected"
        );
        let token = helper_token(serial);
        use std::io::Write;
        let mut intent = std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(report.join("helper-claim-intent.json"))?;
        intent.write_all(&serde_json::to_vec(&json!({"serial":serial,"ownerId":owner_id,"nonce":nonce,"socket":socket,"state":"beforeClaim","tokenPersisted":false}))?)?;
        intent.sync_all()?;
        let body = json!({"action":"claim","nonce":nonce,"ownerId":owner_id,"token":token});
        let reply = match bootstrap_exchange(&adb, serial, &apk_path, &socket, uid, body).await {
            Ok(reply) => reply,
            Err(error) => {
                // Query protected identity with the SAME in-memory credential, never replay claim.
                let port = forward_helper(&adb, serial).await?;
                let probe = Self::at(adb.clone(), serial, port)?;
                let observed = probe
                    .post_json("/v1/session/status", json!({"nonce":nonce}))
                    .await;
                let removed = frames::remove_forward(&adb, serial, port).await;
                removed?;
                let identity = observed
                    .context("claim lost ACK and identity unavailable; reconciliation required")?;
                anyhow::ensure!(
                    identity["ok"] == true
                        && identity["nonce"].as_str() == Some(nonce.as_str())
                        && identity["ownerId"].as_str() == Some(owner_id.as_str()),
                    "claim lost ACK owner not proved"
                );
                let recovered = CanaryOwner {
                    owner_id: owner_id.clone(),
                    nonce: nonce.clone(),
                    socket: socket.clone(),
                    uid,
                    apk_path: apk_path.clone(),
                    report: report.clone(),
                    instance: identity["serviceInstance"]
                        .as_str()
                        .context("claim recovery instance missing")?
                        .into(),
                    generation: identity["ownerGeneration"]
                        .as_str()
                        .context("claim recovery generation missing")?
                        .into(),
                };
                release_canary_owner(&adb, serial, &recovered)
                    .await
                    .context("claim lost ACK owner cleanup unresolved")?;
                return Err(error);
            }
        };
        anyhow::ensure!(
            reply["ok"] == true
                && reply["nonce"].as_str() == Some(nonce.as_str())
                && reply["ownerId"].as_str() == Some(owner_id.as_str())
                && reply["state"] == "ready",
            "helper owner claim not proved"
        );
        let instance = reply["serviceInstance"]
            .as_str()
            .context("helper instance missing")?
            .to_owned();
        let generation = reply["ownerGeneration"]
            .as_str()
            .context("helper owner generation missing")?
            .to_owned();
        uuid::Uuid::parse_str(&instance)?;
        uuid::Uuid::parse_str(&generation)?;
        let owner = CanaryOwner {
            owner_id,
            instance,
            generation,
            socket,
            nonce,
            uid,
            apk_path,
            report,
        };
        let receipt = owner.report.join("helper-owner.json");
        let persisted = (|| -> anyhow::Result<()> {
            let mut file = std::fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&receipt)?;
            file.write_all(&serde_json::to_vec(&json!({"serial":serial,"ownerId":owner.owner_id,"serviceInstance":owner.instance,"generation":owner.generation,"state":"claimed","tokenPersisted":false}))?)?;
            file.sync_all()?;
            Ok(())
        })();
        if let Err(error) = persisted {
            release_canary_owner(&adb, serial, &owner)
                .await
                .context("claim receipt failure cleanup unresolved")?;
            return Err(error);
        }
        let host_port = match forward_helper(&adb, serial).await {
            Ok(port) => port,
            Err(error) => {
                release_canary_owner(&adb, serial, &owner)
                    .await
                    .context("claim cleanup release unresolved")?;
                return Err(error);
            }
        };
        let mut client = match Self::at(adb.clone(), serial, host_port) {
            Ok(client) => client,
            Err(error) => {
                release_canary_owner(&adb, serial, &owner).await?;
                frames::remove_forward(&adb, serial, host_port).await?;
                return Err(error);
            }
        };
        client.canary = Some(owner);
        if let Err(error) = client.require_canary_identity().await {
            client
                .shutdown()
                .await
                .context("claim attach cleanup unresolved")?;
            return Err(error);
        }
        Ok(client)
    }

    fn persist_clipboard_checkpoint(&self) -> anyhow::Result<()> {
        let owner = self
            .canary
            .as_ref()
            .context("clipboard checkpoint requires scoped owner")?;
        let path = owner.report.join("ime-checkpoint.json");
        let temp = owner
            .report
            .join(format!("ime-{}.tmp", uuid::Uuid::new_v4()));
        // Persist only cleanup metadata, never credentials or clipboard contents.
        let baseline = self.clipboard_baseline.lock().as_ref().map(|saved|
            json!({"baselineId":saved.id,"baselineKind":if saved.text.is_none() {"empty"} else {"plaintext"},
                "decodedBytes":saved.text.as_ref().map_or(0, Vec::len),"metadataPreserving":true}));
        let value = json!({"serial":self.serial,"androidUser":0,"ownerId":owner.owner_id,"serviceInstance":owner.instance,"generation":owner.generation,"pending":self.pending_clipboard.lock().clone(),"baseline":baseline});
        let bytes = serde_json::to_vec(&value)?;
        anyhow::ensure!(bytes.len() <= 16 * 1024, "helper cleanup checkpoint exceeds bound");
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        std::fs::rename(temp, path)?;
        Ok(())
    }

    async fn require_canary_identity(&self) -> anyhow::Result<()> {
        let owner = self.canary.as_ref().context("scoped helper owner absent")?;
        let nonce = uuid::Uuid::new_v4().simple().to_string();
        let value = self
            .post_json("/v1/session/status", json!({"nonce":nonce}))
            .await?;
        anyhow::ensure!(
            value["ok"] == true
                && value["nonce"].as_str() == Some(nonce.as_str())
                && value["serviceInstance"].as_str() == Some(owner.instance.as_str())
                && value["ownerId"].as_str() == Some(owner.owner_id.as_str())
                && value["ownerGeneration"].as_str() == Some(owner.generation.as_str())
                && value["ownership"] == "owned",
            "helper owner identity changed"
        );
        Ok(())
    }

    /// Install if needed, enable the IME, start the service, forward, prove `/status`.
    pub async fn ensure(adb: AdbProgram, serial: &str, apk: Option<&Path>) -> anyhow::Result<Self> {
        // Package preparation is independent of runtime bootstrap qualification.
        if !package_installed(&adb, serial).await? {
            let apk = apk.ok_or_else(|| {
                anyhow!(
                    "com.riviu.agent is not installed on {serial} and no helper APK is configured \
                     (RIVIU_ANDROID_AGENT_APK or the bundled riviu-agent.apk)"
                )
            })?;
            install_apk(&adb, serial, apk).await?;
            anyhow::ensure!(
                package_installed(&adb, serial).await?,
                "helper installation on {serial} was not verified; no runtime provisioning"
            );
        }
        // Keep service/IME effects blocked until secret-free bootstrap is qualified.
        start_service(&adb, serial).await?;
        enable_ime(&adb, serial).await?;
        let host_port = forward_helper(&adb, serial).await?;
        finish_helper_attach(
            async {
                let client = Self::at(adb.clone(), serial, host_port)?;
                let status = await_helper_ready(|| client.require_status()).await?;
                if let Some(apk) = apk {
                    client.upgrade_if_stale(&status, apk).await;
                }
                Ok(client)
            },
            || frames::remove_forward(&adb, serial, host_port),
        )
        .await
    }

    /// Attach only an already running helper: no install, IME enable or service start.
    pub async fn attach_existing(adb: AdbProgram, serial: &str) -> anyhow::Result<Self> {
        anyhow::ensure!(
            package_installed(&adb, serial).await?,
            "Diagnostic requires an existing helper package"
        );
        let host_port = forward_helper(&adb, serial).await?;
        finish_helper_attach(
            async {
                let client = Self::at(adb.clone(), serial, host_port)?;
                client.require_status().await?;
                // /status is intentionally unauthenticated. It cannot prove that this
                // process owns a token usable for clipboard or native media requests.
                client.describe_apps(&[PACKAGE.to_string()], false).await?;
                Ok(client)
            },
            || frames::remove_forward(&adb, serial, host_port),
        )
        .await
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
        Self::at_with_token(adb, serial, host_port, helper_token(serial).into())
    }

    fn at_with_token(adb: AdbProgram, serial: &str, host_port: u16, token: std::sync::Arc<str>) -> anyhow::Result<Self> {
        let http = reqwest::Client::builder()
            .timeout(HTTP_TIMEOUT)
            .build()
            .context("dựng HTTP client cho Riviu helper")?;
        Ok(Self {
            http,
            token,
            adb,
            serial: serial.to_string(),
            base: format!("http://127.0.0.1:{host_port}"),
            host_port,
            lifecycle: Default::default(),
            pending_clipboard: Default::default(),
            canary: None,
            production_runtime: false,
            clipboard_qualified: Default::default(),
            clipboard_baseline: Default::default(),
        })
    }

    /// Stop only the helper transport this client established.
    pub async fn shutdown(self) -> anyhow::Result<()> {
        // A cancelled caller may leave its owned clipboard task draining/restoring.
        let _serial = ime_lock(&self.serial).lock_owned().await;
        let mut closed = self.lifecycle.lock().await;
        anyhow::ensure!(
            self.pending_clipboard.lock().is_none() && self.clipboard_baseline.lock().is_none(),
            "helper clipboard cleanup unresolved; retain forward for reconciliation"
        );
        if *closed {
            return Ok(());
        }
        if let Some(owner) = &self.canary {
            if self.production_runtime {
                runtime::release(&self.adb, &self.serial, owner, self.token.as_ref(), Some(self.host_port)).await?;
            } else {
                release_canary_owner(&self.adb, &self.serial, owner).await?;
            }
        }
        let mut failures = Vec::new();
        if let Err(error) = runtime::remove_owned_forward(&self.adb, &self.serial, self.host_port).await {
            failures.push(format!("remove tcp:{} forward: {error}", self.host_port));
        }
        if failures.is_empty() && self.production_runtime {
            if let Some(owner) = &self.canary {
                runtime::finish_release(&self.serial, owner).await?;
            }
        }
        // Legacy token provisioning has no instance/generation ownership proof.
        // Never force-stop the package: it can terminate a replacement owner or IME.
        *closed = failures.is_empty();
        anyhow::ensure!(
            failures.is_empty(),
            "could not shut down Riviu helper transport on {}: {}",
            self.serial,
            failures.join("; ")
        );
        Ok(())
    }

    pub async fn is_alive(&self) -> bool {
        if self.canary.is_some() && self.require_canary_identity().await.is_err() {
            return false;
        }
        self.require_status().await.is_ok()
            && self
                .describe_apps(&[PACKAGE.to_string()], false)
                .await
                .is_ok()
    }

    /// Read only this connection; do not repair, provision or touch the IME.
    pub async fn health(&self) -> HelperHealth {
        let mut health = HelperHealth::unobserved();
        match self.require_status().await {
            Ok(status) => {
                health.service_reachable = Some(true);
                health.agent_version = Some(status.agent_version);
                health.protocol_version = Some(status.protocol_version);
                health.advertised_features = Some(status.features);
                health.reason = "authenticationUnobserved".into();
            }
            Err(_) => {
                health.service_reachable = Some(false);
                health.reason = "statusUnavailable".into();
                return health;
            }
        }
        // App-label lookup is an authenticated read, not a clipboard mutation.
        if self.canary.is_some() && self.require_canary_identity().await.is_err() {
            health.authenticated = Some(false);
            health.reason = "ownerStatusUnproved".into();
            return health;
        }
        match self.describe_apps(&[PACKAGE.to_string()], false).await {
            Ok(_) => {
                health.authenticated = Some(true);
                health.reason = "authenticatedReadVerified".into();
            }
            Err(_) => {
                health.authenticated = Some(false);
                health.reason = "authenticatedReadFailed".into();
            }
        }
        health
    }

    async fn require_status(&self) -> anyhow::Result<HelperStatus> {
        let closed = self.lifecycle.lock().await;
        anyhow::ensure!(!*closed, "helper connection is closed");
        let response = self
            .http
            .get(format!("{}/status", self.base))
            .header(TOKEN_HEADER, self.token.as_ref())
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

    async fn clipboard_job(&self, action: &str, text: Option<&str>) -> anyhow::Result<Value> {
        self.clipboard_job_inner(action, text)
            .await
            .map_err(|error| {
                if error.is::<ClipboardSettledFailure>() {
                    error
                } else {
                    anyhow!(ClipboardSettlementUnknown)
                }
            })
    }

    async fn clipboard_job_inner(&self, action: &str, text: Option<&str>) -> anyhow::Result<Value> {
        self.clipboard_job_compare_inner(action, text, None).await
    }

    async fn clipboard_job_compare_inner(&self, action: &str, text: Option<&str>, expected: Option<&str>) -> anyhow::Result<Value> {
        self.require_canary_identity().await.map_err(|_| anyhow!(ClipboardSettledFailure))?;
        let status = self
            .require_status()
            .await
            .map_err(|_| anyhow!(ClipboardSettledFailure))?;
        if !status
            .features
            .iter()
            .any(|feature| feature == "clipboardJobs")
        {
            return Err(anyhow!(ClipboardSettledFailure));
        }
        let nonce = uuid::Uuid::new_v4().simple().to_string();
        let identity = self
            .post_json("/v1/session/status", json!({"nonce": nonce}))
            .await
            .map_err(|_| anyhow!(ClipboardSettledFailure))?;
        if identity["ok"] != true || identity["nonce"].as_str() != Some(nonce.as_str()) {
            return Err(anyhow!(ClipboardSettledFailure));
        }
        let owner = self.canary.as_ref().ok_or_else(|| anyhow!(ClipboardSettledFailure))?;
        if identity["ownerId"].as_str() != Some(owner.owner_id.as_str())
            || identity["serviceInstance"].as_str() != Some(owner.instance.as_str())
            || identity["ownerGeneration"].as_str() != Some(owner.generation.as_str())
            || identity["ownership"] != "owned" {
            return Err(anyhow!(ClipboardSettledFailure));
        }
        let instance = identity["serviceInstance"]
            .as_str()
            .ok_or_else(|| anyhow!(ClipboardSettledFailure))?;
        let id = uuid::Uuid::new_v4().simple().to_string();
        let mut body = json!({"serviceInstance":instance,"requestId":id,"action":action});
        if let Some(text) = text {
            body[if action == "restoreSnapshot" { "baselineId" } else { "text" }] = json!(text);
        }
        if let Some(expected) = expected {
            body["expectedText"] = json!(expected);
        }
        if let Some(ticket) = self.pending_clipboard.lock().as_mut() {
            ticket["serviceInstance"] = json!(instance);
            ticket["operationId"] = json!(id);
            ticket["phase"] = json!("submissionPending");
        }
        self.persist_clipboard_checkpoint()
            .map_err(|_| anyhow!(ClipboardSettledFailure))?;
        // Send once. A lost submission ACK is reconciled by ID, never resubmitted.
        let submitted = self.post_json("/v1/clipboard/jobs/submit", body).await;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(12);
        let request = json!({"serviceInstance":instance,"requestId":id});
        let mut value = submitted.ok();
        loop {
            if let Some(reply) = value.take() {
                require_ok(&reply, "clipboard job")?;
                anyhow::ensure!(
                    reply["serviceInstance"].as_str() == Some(instance)
                        && reply["operationId"].as_str() == Some(id.as_str()),
                    "clipboard job identity mismatch"
                );
                match reply["state"].as_str() {
                    Some("succeeded") => return Ok(reply["result"].clone()),
                    Some("failed" | "cancelled") => return Err(anyhow!(ClipboardSettledFailure)),
                    Some("queued" | "running") => {}
                    _ => anyhow::bail!("clipboard job settlement is unknown"),
                }
            }
            if tokio::time::Instant::now() >= deadline {
                let cancel = self
                    .post_json("/v1/clipboard/jobs/cancelQueued", request.clone())
                    .await?;
                require_ok(&cancel, "clipboard cancel settlement")?;
                anyhow::ensure!(cancel["serviceInstance"].as_str() == Some(instance)
                    && cancel["operationId"].as_str() == Some(id.as_str())
                    && matches!(cancel["state"].as_str(), Some("cancelled" | "succeeded" | "failed")),
                    "clipboard job still running or identity changed; settlement required before IME cleanup");
                return Err(anyhow!(ClipboardSettledFailure));
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
            value = Some(
                self.post_json("/v1/clipboard/jobs/status", request.clone())
                    .await?,
            );
        }
    }

    pub async fn set_clipboard(&self, content_type: &str, bytes: &[u8]) -> anyhow::Result<()> {
        require_plaintext(content_type)?;
        let text = std::str::from_utf8(bytes)
            .context("clipboard text is not UTF-8")?
            .to_owned();
        let client = self.clone();
        tokio::spawn(async move {
            client
                .with_ime(|| async {
                    let value = client.clipboard_job("set", Some(&text)).await?;
                    anyhow::ensure!(
                        value["written"] == true,
                        "clipboard write result not verified"
                    );
                    Ok(())
                })
                .await
        })
        .await
        .context("clipboard operation task failed")?
    }

    pub async fn get_clipboard(
        &self,
        maximum_decoded_bytes: usize,
    ) -> anyhow::Result<(String, Vec<u8>)> {
        riviu_core::device_capabilities::validate_clipboard_read_limit(maximum_decoded_bytes)?;
        let client = self.clone();
        tokio::spawn(async move {
            client
                .with_ime(|| async {
                    let value = client.clipboard_job("get", None).await?;
                    let bytes = clipboard_read_payload(&value, maximum_decoded_bytes)?;
                    // Public read API represents proven absence as zero bytes, never writes it back.
                    Ok(("plaintext".to_string(), bytes.unwrap_or_default()))
                })
                .await
        })
        .await
        .context("clipboard operation task failed")?
    }

    /// One owned task keeps baseline contents in memory through settlement/restore.
    /// Conditional restoration runs on Android's main looper, not a stale host read.
    pub async fn capture_clipboard_baseline(&self) -> anyhow::Result<()> {
        let client = self.clone();
        tokio::spawn(async move {
            client.with_ime(|| async {
                let snapshot = client.clipboard_job("snapshot", None).await?;
                *client.clipboard_baseline.lock() = Some(ClipboardBaseline::capture(&snapshot)?);
                Ok(())
            }).await
        }).await.context("clipboard baseline task failed")?
    }

    pub async fn restore_clipboard_baseline(&self, expected: &[u8]) -> anyhow::Result<()> {
        let saved = self.clipboard_baseline.lock().clone().context("clipboard baseline absent")?;
        let expected = std::str::from_utf8(expected)?.to_owned();
        let client = self.clone();
        tokio::spawn(async move {
            client.with_ime(|| async {
                let restored = client.clipboard_job_compare_inner("restoreSnapshot", Some(&saved.id), Some(&expected)).await
                    .map_err(|_|anyhow!(ClipboardSettlementUnknown))?;
                if !saved.restored(&restored) { return Err(anyhow!(ClipboardSettlementUnknown)); }
                client.clipboard_baseline.lock().take();
                Ok(())
            }).await
        }).await.context("clipboard baseline restoration task failed")?
    }

    pub async fn qualify_clipboard_roundtrip(&self) -> anyhow::Result<()> {
        if self.production_runtime && self.clipboard_qualified.load(std::sync::atomic::Ordering::Acquire) {
            self.require_canary_identity().await?;
            return Ok(());
        }
        let client = self.clone();
        tokio::spawn(async move {
            client.with_ime_mode(true, || async {
                let status = client.require_status().await?;
                anyhow::ensure!(status.features.iter().any(|f|f == "clipboardSnapshotRestore"), "metadata-preserving clipboard restore unsupported");
                let baseline = client.clipboard_job("snapshot", None).await?;
                let saved = ClipboardBaseline::capture(&baseline)?;
                *client.clipboard_baseline.lock() = Some(saved.clone());
                client.persist_clipboard_checkpoint()?;
                let marker = format!("riviu-clipboard-sentinel-{}", uuid::Uuid::new_v4().simple());
                let write = client.clipboard_job("set", Some(&marker)).await;
                if write.as_ref().err().is_some_and(|e|e.is::<ClipboardSettlementUnknown>()) { return Err(anyhow!(ClipboardSettlementUnknown)); }
                // Even a failed readback after a settled write still attempts conditional restore.
                let observed = if write.is_ok() { client.clipboard_job("get", None).await } else { Err(anyhow!(ClipboardSettledFailure)) };
                let restoration = client.clipboard_job_compare_inner("restoreSnapshot", Some(&saved.id), Some(&marker)).await;
                let restored = restoration.map_err(|_|anyhow!(ClipboardSettlementUnknown))?;
                if !saved.restored(&restored) { return Err(anyhow!(ClipboardSettlementUnknown)); }
                client.clipboard_baseline.lock().take();
                client.persist_clipboard_checkpoint()?;
                write?;
                let observed = observed?;
                anyhow::ensure!(observed["text"].as_str() == Some(marker.as_str()), "clipboard marker readback unproved");
                Ok(())
            }).await?;
            client.clipboard_qualified.store(true, std::sync::atomic::Ordering::Release);
            Ok(())
        }).await.context("clipboard qualification task failed")?
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
        let closed = self.lifecycle.lock().await;
        anyhow::ensure!(!*closed, "helper connection is closed");
        self.post_json_open(path, body).await
    }

    /// Caller holds the lifecycle lock; avoid re-entering it for owner status.
    async fn post_json_open(&self, path: &str, body: Value) -> anyhow::Result<Value> {
        let response = self
            .http
            .post(format!("{}{path}", self.base))
            .header(TOKEN_HEADER, self.token.as_ref())
            .json(&body)
            .send()
            .await
            .with_context(|| format!("POST {}{path}", self.base))?;
        let status = response.status();
        let text = read_capped(response, path).await?;
        let value: Value = serde_json::from_str(&text)
            .with_context(|| format!("helper {path} response is not JSON"))?;
        if !status.is_success() {
            anyhow::bail!("helper {path} HTTP {status}");
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
        self.with_ime_mode(false, op).await
    }

    async fn with_ime_mode<F, Fut, T>(&self, qualification_probe: bool, op: F) -> anyhow::Result<T>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = anyhow::Result<T>>,
    {
        if self.canary.is_none() {
            require_clipboard_qualification()?;
        } else {
            self.require_canary_identity().await?;
            if self.production_runtime && !qualification_probe
                && !self.clipboard_qualified.load(std::sync::atomic::Ordering::Acquire) {
                require_clipboard_qualification()?;
            }
        }
        let _serial = ime_lock(&self.serial).lock_owned().await;
        if self.production_runtime {
            self.require_canary_identity().await?;
            anyhow::ensure!(self.adb.shell(&self.serial, "am get-current-user").await?.trim() == "0", "clipboard Android user changed; no IME mutation");
        }
        anyhow::ensure!(
            self.pending_clipboard.lock().is_none(),
            "helper has unresolved clipboard cleanup"
        );
        let status = self.require_status().await?;
        if self.production_runtime {
            anyhow::ensure!(status.features.iter().any(|f| f == "secureBootstrapBinderShell")
                && status.features.iter().any(|f| f == "clipboardSnapshotRestore"),
                "release clipboard qualification features unavailable; no IME switch");
        }
        anyhow::ensure!(
            status
                .features
                .iter()
                .any(|feature| feature == "clipboardJobs"),
            "helper clipboard settlement is unavailable; no IME switch"
        );
        self.describe_apps(&[PACKAGE.to_string()], false).await?;
        let previous = current_ime(&self.adb, &self.serial).await?;
        anyhow::ensure!(
            previous != IME_ID,
            "previous IME is already the helper; reconciliation required"
        );
        let enabled = self
            .adb
            .shell(&self.serial, "settings get secure enabled_input_methods")
            .await?;
        let helper_enabled = enabled
            .trim()
            .split(':')
            .any(|entry| entry.split(';').next() == Some(IME_ID));
        *self.pending_clipboard.lock() = Some(
            json!({"previousIme":previous,"helperWasEnabled":helper_enabled,"phase":"switching"}),
        );
        self.persist_clipboard_checkpoint()?;
        let outcome = async {
            self.require_canary_identity().await?;
            if !helper_enabled {
                enable_ime(&self.adb, &self.serial).await?;
            }
            set_ime(&self.adb, &self.serial, IME_ID).await?;
            anyhow::ensure!(
                current_ime(&self.adb, &self.serial).await? == IME_ID,
                "helper IME selection was not verified"
            );
            // Selection readback is required; this settle is not readiness proof.
            tokio::time::sleep(Duration::from_millis(250)).await;
            op().await
        }
        .await;
        if outcome
            .as_ref()
            .err()
            .is_some_and(|error| error.is::<ClipboardSettlementUnknown>())
        {
            anyhow::bail!(
                "clipboard settlement unknown; IME retained for explicit reconciliation, no replay"
            );
        }
        let restore = async {
            self.require_canary_identity().await?;
            let current = current_ime(&self.adb, &self.serial).await?;
            if current == IME_ID {
                set_ime(&self.adb, &self.serial, &previous).await?;
                anyhow::ensure!(
                    current_ime(&self.adb, &self.serial).await? == previous,
                    "previous IME restoration was not verified"
                );
            } else {
                anyhow::ensure!(
                    current == previous,
                    "IME changed outside this operation; preserved, reconciliation required"
                );
            }
            Ok(())
        }
        .await;
        let restore = if restore.is_ok() && !helper_enabled {
            async {
                self.require_canary_identity().await?;
                self.adb
                    .shell(&self.serial, &format!("ime disable {IME_ID}"))
                    .await?;
                let enabled = self
                    .adb
                    .shell(&self.serial, "settings get secure enabled_input_methods")
                    .await?;
                anyhow::ensure!(
                    !enabled
                        .trim()
                        .split(':')
                        .any(|entry| entry.split(';').next() == Some(IME_ID)),
                    "helper IME enablement restore unverified"
                );
                Ok(())
            }
            .await
        } else {
            restore
        };
        if restore.is_ok() {
            self.pending_clipboard.lock().take();
            if self.clipboard_baseline.lock().is_some() {
                self.persist_clipboard_checkpoint()?;
            } else if let Some(owner) = &self.canary {
                std::fs::remove_file(owner.report.join("ime-checkpoint.json"))?;
            }
        }
        combine_ime_guard(outcome, restore)
    }
}

async fn release_canary_owner(
    adb: &AdbProgram,
    serial: &str,
    owner: &CanaryOwner,
) -> anyhow::Result<()> {
    let output = adb.shell_output(serial, &format!("am start-foreground-service -n {SERVICE} --es bootstrapSocket {} --es bootstrapNonce {} --es bootstrapOwnerId {}", owner.socket, owner.nonce, owner.owner_id), Duration::from_secs(15)).await?;
    anyhow::ensure!(
        output.exit_code == 0
            && !output.stdout.contains("Error")
            && !output.stderr.contains("Error"),
        "bootstrap release listener start failed"
    );
    let reply = bootstrap_exchange(adb, serial, &owner.apk_path, &owner.socket, owner.uid,
        json!({"action":"release","nonce":owner.nonce,"ownerId":owner.owner_id,"token":helper_token(serial),"serviceInstance":owner.instance,"ownerGeneration":owner.generation})).await?;
    anyhow::ensure!(
        reply["ok"] == true
            && reply["state"] == "released"
            && reply["nonce"].as_str() == Some(owner.nonce.as_str())
            && reply["ownerId"].as_str() == Some(owner.owner_id.as_str())
            && reply["serviceInstance"].as_str() == Some(owner.instance.as_str())
            && reply["ownerGeneration"].as_str() == Some(owner.generation.as_str()),
        "helper owned release unresolved"
    );
    std::fs::write(
        owner.report.join("helper-released.json"),
        serde_json::to_vec(&reply)?,
    )?;
    Ok(())
}

async fn bootstrap_exchange(
    adb: &AdbProgram,
    serial: &str,
    apk: &str,
    socket: &str,
    uid: u32,
    body: Value,
) -> anyhow::Result<Value> {
    let bytes = serde_json::to_vec(&body)?;
    anyhow::ensure!(bytes.len() <= 4096, "bootstrap envelope too large");
    let mut payload = (bytes.len() as u32).to_be_bytes().to_vec();
    payload.extend(bytes);
    let bridge = format!(
        "CLASSPATH={} app_process /system/bin com.riviu.agent.BootstrapBridge {socket} {uid}",
        crate::adb::quote_device_path(apk)
    );
    let script = format!(
        "run-as com.riviu.agent sh -c '{}'",
        bridge.replace('\'', "'\\''")
    );
    let reply = adb
        .shell_secret_input(serial, &script, payload, Duration::from_secs(12))
        .await?;
    anyhow::ensure!(reply.len() >= 4, "bootstrap reply missing");
    let size = u32::from_be_bytes(reply[..4].try_into()?) as usize;
    anyhow::ensure!(
        size <= 4096 && reply.len() == size + 4,
        "bootstrap reply framing invalid"
    );
    serde_json::from_slice(&reply[4..]).context("bootstrap reply invalid")
}

fn require_clipboard_qualification() -> anyhow::Result<()> {
    Err(anyhow!(HelperClipboardNotQualified))
}

#[derive(Debug)]
pub struct HelperClipboardNotQualified;
impl std::fmt::Display for HelperClipboardNotQualified {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("HelperClipboardNotQualified: fresh owner-bound metadata restoration proof required; no IME mutation")
    }
}
impl std::error::Error for HelperClipboardNotQualified {}

#[derive(Debug)]
struct ClipboardSettledFailure;
impl std::fmt::Display for ClipboardSettledFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("clipboard job settled without success")
    }
}
impl std::error::Error for ClipboardSettledFailure {}

#[derive(Debug)]
struct ClipboardSettlementUnknown;
impl std::fmt::Display for ClipboardSettlementUnknown {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("clipboard job settlement is unknown")
    }
}
impl std::error::Error for ClipboardSettlementUnknown {}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HelperHealth {
    pub service_reachable: Option<bool>,
    pub authenticated: Option<bool>,
    pub agent_version: Option<String>,
    pub protocol_version: Option<u32>,
    pub advertised_features: Option<Vec<String>>,
    /// No capability is live-qualified by a health read alone.
    pub reason: String,
}
impl HelperHealth {
    pub fn unobserved() -> Self {
        Self {
            service_reachable: None,
            authenticated: None,
            agent_version: None,
            protocol_version: None,
            advertised_features: None,
            reason: "connectionUnobserved".into(),
        }
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
        .context("Không đọc được gói Riviu Helper; chưa xác định được đã cài hay chưa")?;
    Ok(listing.contains("package:"))
}

pub(crate) async fn install_apk(adb: &AdbProgram, serial: &str, apk: &Path) -> anyhow::Result<()> {
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
    anyhow::ensure!(
        output.lines().any(|line| line.trim() == "Success"),
        "Cài Riviu Helper chưa thành công: {}",
        output.trim()
    );
    Ok(())
}

fn miui_install_refused(serial: &str) -> String {
    format!(
        "Máy {serial} chặn cài Riviu Helper (INSTALL_FAILED_USER_RESTRICTED). \
         Bật Tuỳ chọn nhà phát triển → Cài đặt qua USB trên điện thoại, rồi kết nối lại. \
         MIUI/HyperOS cũng có thể yêu cầu Gỡ lỗi USB (Cài đặt bảo mật)."
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
async fn start_service(_adb: &AdbProgram, _serial: &str) -> anyhow::Result<()> {
    anyhow::bail!("HelperSecureProvisioningRequired: secret-free bootstrap is not runtime-qualified; token-bearing ADB arguments refused")
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
    let allocated = adb
        .device(
            serial,
            &["forward", "tcp:0", &remote],
            Duration::from_secs(30),
        )
        .await
        .with_context(|| format!("allocate helper forward on {serial}"))?;
    allocated
        .trim()
        .parse::<u16>()
        .ok()
        .filter(|port| *port != 0)
        .ok_or_else(|| anyhow!("ADB did not report the owned helper forward port"))
}

// Until attach succeeds the driver has no cached client to clean during shutdown.
async fn finish_helper_attach<T, Attach, Cleanup, CleanupFuture>(
    attach: Attach,
    cleanup: Cleanup,
) -> anyhow::Result<T>
where
    Attach: std::future::Future<Output = anyhow::Result<T>>,
    Cleanup: FnOnce() -> CleanupFuture,
    CleanupFuture: std::future::Future<Output = anyhow::Result<()>>,
{
    match attach.await {
        Ok(client) => Ok(client),
        Err(error) => match cleanup().await {
            Ok(()) => Err(error),
            Err(cleanup) => {
                Err(error.context(format!("helper attach forward cleanup failed: {cleanup:#}")))
            }
        },
    }
}

async fn await_helper_ready<T, F, Fut>(mut probe: F) -> anyhow::Result<T>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = anyhow::Result<T>>,
{
    let deadline = tokio::time::Instant::now() + Duration::from_secs(4);
    let mut last_error = None;
    loop {
        match tokio::time::timeout_at(deadline, probe()).await {
            Ok(Ok(value)) => return Ok(value),
            Ok(Err(error)) => last_error = Some(error),
            Err(_) => {
                return Err(last_error.unwrap_or_else(|| anyhow!("helper status timed out")))
                    .context("helper not ready within four seconds")
            }
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(last_error.unwrap()).context("helper not ready within four seconds");
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
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

    #[tokio::test]
    async fn reconnect_rejects_authenticated_helper_with_replaced_owner() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = tokio::spawn(async move {
            loop {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = [0; 4096];
                let size = socket.read(&mut request).await.unwrap();
                let request = String::from_utf8_lossy(&request[..size]);
                let body = if request.starts_with("GET /status ") {
                    json!({"ok":true,"agentVersion":"0.7.0","protocolVersion":1,"features":["auth"]})
                } else if request.starts_with("POST /v1/apps/describe ") {
                    json!({"ok":true,"apps":[]})
                } else {
                    json!({"ok":true,"ownership":"owned","ownerId":"replacement","serviceInstance":"replacement","ownerGeneration":"2"})
                }.to_string();
                socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
            }
        });
        let mut client = HelperClient::at(AdbProgram::at(PathBuf::from("never-run-adb")), "fixture-reconnect-owner", port).unwrap();
        client.canary = Some(CanaryOwner {
            owner_id: "original".into(), instance: "original".into(), generation: "1".into(),
            socket: "unused".into(), nonce: "unused".into(), uid: 10001,
            apk_path: "unused".into(), report: std::env::temp_dir(),
        });
        let alive = client.is_alive().await;
        server.abort();
        assert!(!alive, "authenticated app labels cannot prove the cached session owner survived reconnect");
    }

    #[test]
    fn durable_clipboard_checkpoint_excludes_credentials_and_clipboard_text() {
        let root = std::env::temp_dir().join(format!("helper-checkpoint-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        let mut client = HelperClient::at(AdbProgram::at(PathBuf::from("never-run-adb")), "fixture-checkpoint", 1).unwrap();
        client.canary = Some(CanaryOwner { owner_id: "owner".into(), instance: "instance".into(), generation: "1".into(), socket: "unused".into(), nonce: "nonce".into(), uid: 10001, apk_path: "unused".into(), report: root.clone() });
        *client.pending_clipboard.lock() = Some(json!({"previousIme":"fixture/.Ime","helperWasEnabled":false,"phase":"switching"}));
        *client.clipboard_baseline.lock() = Some(ClipboardBaseline { id: "baseline-id".into(), text: Some(b"PRIVATE_CLIPBOARD_TEXT".to_vec()) });
        client.persist_clipboard_checkpoint().unwrap();
        let text = std::fs::read_to_string(root.join("ime-checkpoint.json")).unwrap();
        let value: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(value["baseline"]["baselineId"], "baseline-id");
        assert_eq!(value["baseline"]["decodedBytes"], 22);
        assert_eq!(value["baseline"]["baselineKind"], "plaintext");
        assert_eq!(value["androidUser"], 0);
        assert!(!text.contains("PRIVATE_CLIPBOARD_TEXT") && !text.contains(&helper_token("fixture-checkpoint")));
        // A later phase atomically replaces the same persistent record on Windows too.
        client.pending_clipboard.lock().as_mut().unwrap()["phase"] = json!("submissionPending");
        client.persist_clipboard_checkpoint().unwrap();
        let value: Value = serde_json::from_slice(&std::fs::read(root.join("ime-checkpoint.json")).unwrap()).unwrap();
        assert_eq!(value["pending"]["phase"], "submissionPending");
        let empty = ClipboardBaseline::capture(&json!({"baselineId":"empty-id","baselineKind":"empty","available":false,"plainTextBaseline":false})).unwrap();
        let plain_empty = ClipboardBaseline::capture(&json!({"baselineId":"plain-empty-id","baselineKind":"plaintext","available":true,"plainTextBaseline":true,"text":""})).unwrap();
        assert!(empty.text.is_none());
        assert_eq!(plain_empty.text, Some(Vec::new()));
        let null_restored = json!({"written":true,"verified":true,"available":false});
        let text_restored = json!({"written":true,"verified":true,"available":true});
        assert!(empty.restored(&null_restored) && !empty.restored(&text_restored));
        assert!(plain_empty.restored(&text_restored) && !plain_empty.restored(&null_restored));
        assert!(!empty.restored(&json!({"written":true,"verified":false,"available":false})));
        assert!(clipboard_read_payload(&json!({"baselineKind":"empty","available":false,"plainTextBaseline":false,"text":""}), 4096).is_err());
        assert!(ClipboardBaseline::capture(&json!({"baselineId":"unknown","available":false,"plainTextBaseline":false})).is_err());
        assert!(ClipboardBaseline::capture(&json!({"baselineId":"rich","baselineKind":"unsupported","available":true,"plainTextBaseline":false,"text":"foreign"})).is_err());
        assert!(ClipboardBaseline::capture(&json!({"baselineKind":"empty","available":false,"plainTextBaseline":false})).is_err());
        *client.clipboard_baseline.lock() = Some(empty);
        client.persist_clipboard_checkpoint().unwrap();
        let value: Value = serde_json::from_slice(&std::fs::read(root.join("ime-checkpoint.json")).unwrap()).unwrap();
        assert_eq!(value["baseline"]["baselineKind"], "empty");
        assert_eq!(value["baseline"]["decodedBytes"], 0);
        assert!(value["baseline"].get("text").is_none());
        *client.clipboard_baseline.lock() = Some(plain_empty);
        client.persist_clipboard_checkpoint().unwrap();
        let value: Value = serde_json::from_slice(&std::fs::read(root.join("ime-checkpoint.json")).unwrap()).unwrap();
        assert_eq!(value["baseline"]["baselineKind"], "plaintext");
        assert_eq!(value["baseline"]["decodedBytes"], 0);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn ensure_installs_missing_helper_once_before_secure_provisioning_refusal() {
        let root = std::env::temp_dir().join(format!("helper-install-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        let calls = root.join("calls.txt");
        std::fs::write(&calls, "").unwrap();
        let apk = root.join("helper.apk");
        std::fs::write(&apk, b"fixture-apk-not-for-device").unwrap();
        #[cfg(windows)]
        let executable = {
            let path = root.join("fake-adb.cmd");
            std::fs::write(&path, concat!(
                "@echo off\r\n",
                "echo %*>>\"%~dp0calls.txt\"\r\n",
                "if \"%~3\"==\"install\" (\r\n",
                "  if exist \"%~dp0refused\" (\r\n",
                "    echo Failure [INSTALL_FAILED_USER_RESTRICTED]\r\n",
                "    exit /b 0\r\n",
                "  )\r\n",
                "  type nul > \"%~dp0installed\"\r\n",
                "  echo Success\r\n",
                "  exit /b 0\r\n",
                ")\r\n",
                "if \"%~3\"==\"shell\" (\r\n",
                "  if exist \"%~dp0installed\" echo package:/data/app/com.riviu.agent/base.apk\r\n",
                "  exit /b 0\r\n",
                ")\r\n",
                "exit /b 97\r\n",
            )).unwrap();
            path
        };
        #[cfg(not(windows))]
        let executable = {
            use std::os::unix::fs::PermissionsExt;
            let path = root.join("fake-adb");
            std::fs::write(&path, concat!(
                "#!/bin/sh\n",
                "cd -- \"$(dirname -- \"$0\")\" || exit 97\n",
                "printf '%s\\n' \"$*\" >> calls.txt\n",
                "case \"$3\" in\n",
                "install) if [ -f refused ]; then echo 'Failure [INSTALL_FAILED_USER_RESTRICTED]'; else touch installed; echo Success; fi ;;\n",
                "shell) if [ -f installed ]; then echo package:/data/app/com.riviu.agent/base.apk; fi ;;\n",
                "*) exit 97 ;;\n",
                "esac\n",
            )).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
            path
        };
        let adb = AdbProgram::at(executable);
        let error = HelperClient::ensure(
            adb.clone(),
            "fixture-provision",
            Some(&apk),
        )
        .await
        .err()
        .expect("must refuse");
        assert!(error
            .to_string()
            .contains("HelperSecureProvisioningRequired"));
        let first = std::fs::read_to_string(&calls).unwrap();
        let commands: Vec<_> = first.lines().collect();
        assert_eq!(commands.len(), 3, "must query, install once, then verify before refusing: {first}");
        assert!(commands[0].contains("pm path com.riviu.agent"));
        assert!(commands[1].contains("install -r -g"));
        assert!(commands[1].contains(apk.to_str().unwrap()));
        assert!(commands[2].contains("pm path com.riviu.agent"));
        // An already-present package is queried, never installed again.
        let second = HelperClient::ensure(adb.clone(), "fixture-provision", Some(&apk)).await.err().unwrap();
        assert!(second.to_string().contains("HelperSecureProvisioningRequired"));
        let all = std::fs::read_to_string(&calls).unwrap();
        assert_eq!(all.lines().count(), 4, "existing helper must only be queried: {all}");
        assert_eq!(all.matches("install -r -g").count(), 1);
        assert!(!all.contains("ime ") && !all.contains("am start") && !all.contains("forward"));
        assert!(!all.contains(&helper_token("fixture-provision")), "credential must never enter argv");

        // Missing bundle and platform install refusal must keep their specific diagnostics.
        std::fs::remove_file(root.join("installed")).unwrap();
        std::fs::write(&calls, "").unwrap();
        let missing = HelperClient::ensure(adb.clone(), "fixture-provision", None).await.err().unwrap();
        assert!(missing.to_string().contains("no helper APK is configured"));
        let missing_calls = std::fs::read_to_string(&calls).unwrap();
        assert_eq!(missing_calls.lines().count(), 1);
        assert!(missing_calls.contains("pm path com.riviu.agent"));

        std::fs::write(&calls, "").unwrap();
        std::fs::write(root.join("refused"), "").unwrap();
        let refused = HelperClient::ensure(adb, "fixture-provision", Some(&apk)).await.err().unwrap();
        assert!(refused.to_string().contains("INSTALL_FAILED_USER_RESTRICTED"));
        let refused_calls = std::fs::read_to_string(&calls).unwrap();
        assert_eq!(refused_calls.lines().count(), 2, "refused install must not retry or reach provisioning: {refused_calls}");
        assert_eq!(refused_calls.matches("install -r -g").count(), 1);
        assert!(!refused_calls.contains("ime ") && !refused_calls.contains("am start") && !refused_calls.contains("forward"));
        assert!(!refused_calls.contains(&helper_token("fixture-provision")));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn unqualified_clipboard_never_switches_ime() {
        let client = HelperClient::at(
            AdbProgram::at(PathBuf::from("never-run-adb")),
            "fixture-unqualified",
            1,
        )
        .unwrap();
        assert!(client
            .get_clipboard(1024)
            .await
            .unwrap_err()
            .to_string()
            .contains("HelperClipboardNotQualified"));
        assert!(client.pending_clipboard.lock().is_none());
    }

    #[tokio::test]
    async fn unresolved_clipboard_blocks_shutdown_and_preserves_reconciliation_state() {
        let client = HelperClient::at(
            AdbProgram::at(PathBuf::from("never-run-adb")),
            "fixture-pending",
            1,
        )
        .unwrap();
        *client.pending_clipboard.lock() = Some(
            json!({"previousIme":"fixture/.Ime","operationId":"fixture-operation","serviceInstance":"fixture-instance"}),
        );
        let clone = client.clone();
        assert!(clone
            .shutdown()
            .await
            .unwrap_err()
            .to_string()
            .contains("unresolved"));
        assert!(!*client.lifecycle.lock().await);
        assert_eq!(
            client.pending_clipboard.lock().as_ref().unwrap()["operationId"],
            "fixture-operation"
        );
    }

    #[tokio::test]
    async fn public_status_does_not_prove_authenticated_helper_health() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = tokio::spawn(async move {
            for index in 0..2 {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = [0; 4096];
                let size = socket.read(&mut request).await.unwrap();
                let request = String::from_utf8_lossy(&request[..size]);
                let (status, body) = if index == 0 {
                    assert!(request.starts_with("GET /status "));
                    (
                        "200 OK",
                        r#"{"ok":true,"agentVersion":"0.5.0","protocolVersion":1,"features":["clipboard"]}"#,
                    )
                } else {
                    assert!(request.starts_with("POST /v1/apps/describe "));
                    ("401 Unauthorized", r#"{"ok":false,"error":"unauthorized"}"#)
                };
                socket.write_all(format!("HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
            }
        });
        let client = HelperClient::at(
            AdbProgram::at(PathBuf::from("never-run-adb")),
            "fixture-health",
            port,
        )
        .unwrap();
        let health = client.health().await;
        assert_eq!(health.service_reachable, Some(true));
        assert_eq!(health.authenticated, Some(false));
        assert_eq!(health.reason, "authenticatedReadFailed");
        assert_eq!(health.advertised_features, Some(vec!["clipboard".into()]));
        server.await.unwrap();
    }

    #[tokio::test]
    async fn closed_helper_clone_never_dispatches() {
        let mut client = HelperClient::at(
            AdbProgram::at(PathBuf::from("never-run-adb")),
            "fixture-closed",
            1,
        )
        .unwrap();
        let root = std::env::temp_dir().join(format!("riviu-released-clone-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        client.canary = Some(CanaryOwner {
            owner_id: "released".into(), instance: "released".into(), generation: "1".into(),
            socket: "unused".into(), nonce: "unused".into(), uid: 10001,
            apk_path: "unused".into(), report: root.clone(),
        });
        client.production_runtime = true;
        assert!(!client.is_released().await.unwrap(), "a live owner must remain cached");
        *client.lifecycle.lock().await = true;
        let clone = client.clone();
        assert!(!clone.is_scoped_canary(), "closed ownership cannot admit a request");
        assert!(clone.is_released().await.unwrap(), "a clean released clone can be retired");
        *client.pending_clipboard.lock() = Some(json!({"operationId":"pending"}));
        assert!(clone.is_released().await.is_err());
        client.pending_clipboard.lock().take();
        *client.clipboard_baseline.lock() = Some(ClipboardBaseline { id: "pending".into(), text: None });
        assert!(clone.is_released().await.is_err());
        client.clipboard_baseline.lock().take();
        for filename in ["runtime-owner.json", "ime-checkpoint.json"] {
            let path = root.join(filename);
            std::fs::write(&path, b"{}").unwrap();
            assert!(clone.is_released().await.is_err(), "retained {filename} must fence a fresh claim");
            std::fs::remove_file(path).unwrap();
        }
        assert!(clone.is_released().await.unwrap());
        assert!(clone
            .require_status()
            .await
            .unwrap_err()
            .to_string()
            .contains("closed"));
        assert!(clone
            .describe_apps(&[PACKAGE.into()], false)
            .await
            .unwrap_err()
            .to_string()
            .contains("closed"));
        // Repeated shutdown does not spawn the deliberately nonexistent ADB.
        clone.shutdown().await.unwrap();
        std::fs::remove_dir(root).unwrap();
    }

    #[tokio::test]
    async fn authenticated_probe_refuses_http_denial_even_with_ok_payload() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = vec![0; 4096];
            let size = socket.read(&mut request).await.unwrap();
            let request = String::from_utf8_lossy(&request[..size]);
            assert!(request.starts_with("POST /v1/apps/describe "));
            assert!(request.to_ascii_lowercase().contains("x-riviu-token:"));
            let body = r#"{"ok":true,"apps":[]}"#;
            socket.write_all(format!(
                "HTTP/1.1 401 Unauthorized\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()
            ).as_bytes()).await.unwrap();
        });
        let client = HelperClient::at(
            AdbProgram::at(PathBuf::from("never-run-adb")),
            "fixture-auth",
            port,
        )
        .unwrap();
        let error = client
            .describe_apps(&[PACKAGE.into()], false)
            .await
            .unwrap_err();
        assert!(format!("{error:#}").contains("401"));
        server.await.unwrap();
    }

    #[tokio::test]
    async fn helper_startup_can_finish_after_the_service_start_command_returns() {
        let calls = std::sync::atomic::AtomicUsize::new(0);
        let value = await_helper_ready(|| async {
            if calls.fetch_add(1, std::sync::atomic::Ordering::Relaxed) == 0 {
                anyhow::bail!("connection refused");
            }
            Ok(42)
        })
        .await
        .unwrap();
        assert_eq!(value, 42);
        assert_eq!(calls.load(std::sync::atomic::Ordering::Relaxed), 2);
    }

    #[tokio::test]
    async fn helper_startup_deadline_keeps_the_last_status_error() {
        let started = std::time::Instant::now();
        let error = await_helper_ready(|| async { Err::<(), _>(anyhow!("status auth not ready")) })
            .await
            .unwrap_err();
        assert!(format!("{error:#}").contains("status auth not ready"));
        assert!(started.elapsed() < Duration::from_secs(6));
    }

    #[tokio::test]
    async fn failed_helper_attach_releases_its_forward_before_returning_error() {
        let cleaned = std::sync::atomic::AtomicBool::new(false);
        let result = finish_helper_attach(
            async { Err::<(), _>(anyhow!("status timeout")) },
            || async {
                cleaned.store(true, std::sync::atomic::Ordering::SeqCst);
                Ok(())
            },
        )
        .await;
        assert!(cleaned.load(std::sync::atomic::Ordering::SeqCst));
        assert_eq!(result.unwrap_err().to_string(), "status timeout");
    }

    #[tokio::test]
    async fn successful_helper_attach_retains_its_forward() {
        let result = finish_helper_attach(async { Ok(42) }, || async {
            panic!("must retain transport")
        })
        .await;
        assert_eq!(result.unwrap(), 42);
    }

    #[tokio::test]
    async fn failed_helper_attach_preserves_status_and_cleanup_errors() {
        let result = finish_helper_attach(
            async { Err::<(), _>(anyhow!("status timeout")) },
            || async { Err(anyhow!("adb offline")) },
        )
        .await;
        let message = format!("{:#}", result.unwrap_err());
        assert!(message.contains("status timeout") && message.contains("adb offline"));
    }

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
    fn a_miui_refusal_names_the_phone_and_the_operator_recovery() {
        let text = miui_install_refused("10969614");
        assert!(text.contains("10969614"), "{text}");
        assert!(text.contains("INSTALL_FAILED_USER_RESTRICTED"), "{text}");
        assert!(text.contains("Cài đặt qua USB"), "{text}");
        assert!(text.contains("kết nối lại"), "{text}");
    }
}
