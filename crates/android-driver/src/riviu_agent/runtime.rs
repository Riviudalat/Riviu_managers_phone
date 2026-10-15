//! Release-only Binder carrier and durable, nonsecret cleanup admission.
use super::*;
use std::io::{Read, Write};

const CERT_SHA256: &str = "0ecb2f06620b2d0f2fcc2a71cede204819a20ddcc1210573bc8ab3f4b48ff813";
const MIN_VERSION: u64 = 7;
const CHECKPOINT_LIMIT: u64 = 16 * 1024;

#[derive(Debug)]
pub(crate) struct HelperMaintenanceBusyBeforeDispatch;
impl std::fmt::Display for HelperMaintenanceBusyBeforeDispatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("HelperMaintenanceBusyBeforeDispatch")
    }
}
impl std::error::Error for HelperMaintenanceBusyBeforeDispatch {}

#[derive(Debug)]
struct HelperMaintenanceRetainedOwnerMissing;
impl std::fmt::Display for HelperMaintenanceRetainedOwnerMissing {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("HelperMaintenanceRetainedOwnerMissing")
    }
}
impl std::error::Error for HelperMaintenanceRetainedOwnerMissing {}

#[derive(Debug)]
pub struct HelperRecoveryRequired;
impl std::fmt::Display for HelperRecoveryRequired {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("HelperRecoveryRequired: prior owner cleanup unresolved; no claim or action replay")
    }
}

/// adbd's shell identity is not the one the helper runtime is bound to.
///
/// Owner records, IME journals and maintenance plans are only valid for shell UID 2000 on
/// Android user 0, so this refuses before any claim, maintenance dispatch or action. A box
/// that starts adbd as root (`adb root` at boot) lands here as `Root`; the app repairs it
/// with one automatic `adb unroot` per phone boot (`AndroidDriver::repair_root_adb_shell`),
/// never a root fallback. Where the caller already fenced a retained
/// owner as recovery-required, the refusal keeps that fence as context.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HelperShellIdentityRefused {
    /// `id -u` answered 0.
    Root,
    /// `id -u` answered neither 0 nor 2000.
    UnexpectedUid,
    /// `am get-current-user` answered something other than 0.
    AndroidUser,
}

impl HelperShellIdentityRefused {
    const ALL: [Self; 3] = [Self::Root, Self::UnexpectedUid, Self::AndroidUser];

    /// Stable token that opens this refusal's Display text. It never carries device output.
    pub fn marker(self) -> &'static str {
        match self {
            Self::Root => "HelperShellRootRefused",
            Self::UnexpectedUid => "HelperShellUidRefused",
            Self::AndroidUser => "HelperAndroidUserRefused",
        }
    }

    /// Fixed public category shared by the roster latch and the maintenance commands.
    pub fn category(self) -> &'static str {
        match self {
            Self::Root => "shell_root",
            Self::UnexpectedUid => "shell_uid",
            Self::AndroidUser => "android_user",
        }
    }

    /// Recognize a refusal inside a stringified error chain (the control-plane boundary
    /// erases the anyhow type). Only this type writes the `<marker>:` token.
    pub fn from_chain_text(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|refusal| text.contains(&format!("{}:", refusal.marker())))
    }

    /// Operator-facing Vietnamese instruction, naming only steps an operator can take from
    /// the app or the phone (never a hand-typed adb command). It carries no device output.
    pub fn operator_message(self) -> &'static str {
        match self {
            Self::Root => "adb trên máy đang chạy quyền root (uid 0), thường do GenFarmer hoặc ROM bật; helper không chạy với root. Riviu tự tắt quyền root của adb một lần cho mỗi lần khởi động máy; nếu vẫn thấy lỗi này: tắt GenFarmer trên PC này rồi khởi động lại máy.",
            Self::UnexpectedUid => "adb shell của máy không chạy bằng tài khoản shell (uid 2000); helper từ chối. Tắt rồi bật lại Gỡ lỗi USB trên máy, rồi chọn Khôi phục helper lại.",
            Self::AndroidUser => "Máy đang ở người dùng Android khác người dùng chính (user 0) — helper chỉ chạy cho người dùng chính. Chuyển về người dùng chính trên máy, rồi Khôi phục helper lại.",
        }
    }
}

impl std::fmt::Display for HelperShellIdentityRefused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Root => "HelperShellRootRefused: adb shell runs as root (uid 0); helper requires shell uid 2000; no root fallback",
            Self::UnexpectedUid => "HelperShellUidRefused: adb shell uid is not 2000; no fallback",
            Self::AndroidUser => "HelperAndroidUserRefused: current Android user is not 0; no claim or action replay",
        })
    }
}

impl std::error::Error for HelperShellIdentityRefused {}

/// Prove adbd runs as shell UID 2000 on Android user 0. Transport failures stay transport
/// failures; only an answered mismatch becomes the typed refusal.
async fn require_shell_identity(adb: &AdbProgram, serial: &str) -> anyhow::Result<()> {
    match checked_shell(adb, serial, "id -u").await?.trim() {
        "2000" => {}
        "0" => return Err(anyhow!(HelperShellIdentityRefused::Root)),
        _ => return Err(anyhow!(HelperShellIdentityRefused::UnexpectedUid)),
    }
    anyhow::ensure!(checked_shell(adb, serial, "am get-current-user").await?.trim() == "0",
        HelperShellIdentityRefused::AndroidUser);
    Ok(())
}

/// Keep a typed identity refusal visible beneath the recovery fence the caller already had.
fn fence_identity_refusal(error: anyhow::Error) -> Result<anyhow::Error, anyhow::Error> {
    if error.is::<HelperShellIdentityRefused>() {
        if error.is::<HelperRecoveryRequired>() { Ok(error) } else { Ok(error.context(HelperRecoveryRequired)) }
    } else {
        Err(error)
    }
}

/// Same proof, for callers whose mismatch already meant recovery-required.
async fn require_shell_identity_for_recovery(adb: &AdbProgram, serial: &str) -> anyhow::Result<()> {
    require_shell_identity(adb, serial).await
        .map_err(|error| fence_identity_refusal(error).unwrap_or_else(|error| error))
}

impl std::error::Error for HelperRecoveryRequired {}

fn state_path(root: &Path, serial: &str) -> anyhow::Result<PathBuf> {
    anyhow::ensure!(root.is_absolute(), "helper state path must be absolute");
    Ok(root.join("helper-runtime").join(riviu_core::frame_sha256(serial.as_bytes())))
}

fn pending_checkpoint(path: &Path, serial: &str) -> anyhow::Result<Option<Value>> {
    let file = match std::fs::File::open(path.join("ime-checkpoint.json")) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let mut bytes = Vec::new();
    file.take(CHECKPOINT_LIMIT + 1).read_to_end(&mut bytes)?;
    anyhow::ensure!(bytes.len() as u64 <= CHECKPOINT_LIMIT, "helper cleanup checkpoint exceeds bound");
    let value: Value = serde_json::from_slice(&bytes)?;
    anyhow::ensure!(value["serial"].as_str() == Some(serial) && value["androidUser"] == 0, "helper cleanup checkpoint identity mismatch");
    // Only absence proves cleanup finished. A malformed or empty record is ambiguous.
    anyhow::ensure!(value["pending"].is_object() || value["baseline"].is_object(), "helper cleanup checkpoint malformed");
    Ok(Some(value))
}

// Response data is never copied into diagnostics. Only fixed protocol codes and
// binding booleans survive a failed claim, so a later resume can explain its fence.
fn claim_state(value: &Value) -> &'static str {
    match value.as_str() {
        Some("ready") => "ready",
        Some("owner_conflict") => "owner_conflict",
        Some("reconciliation_required") => "reconciliation_required",
        Some("bind_failed") => "bind_failed",
        Some("session_binding_refused") => "session_binding_refused",
        Some("bootstrap_failed") => "bootstrap_failed",
        Some("service_destroyed") => "service_destroyed",
        Some("invalid_action") => "invalid_action",
        _ => "unrecognized",
    }
}

fn claim_diagnostic(identity: &Value, owner: &CanaryOwner, reply: &'static str) -> Value {
    json!({
        "reply": reply,
        "state": claim_state(&identity["state"]),
        "ok": identity["ok"] == true,
        "nonceMatched": identity["nonce"].as_str() == Some(owner.nonce.as_str()),
        "ownerMatched": identity["ownerId"].as_str() == Some(owner.owner_id.as_str()),
        "ready": identity["state"] == "ready",
        "owned": identity["ownership"] == "owned",
        "instancePresent": identity["serviceInstance"].as_str().is_some_and(|v| !v.is_empty() && v != "-"),
        "generationPresent": identity["ownerGeneration"].as_str().is_some_and(|v| !v.is_empty() && v != "-"),
    })
}

fn claim_diagnostic_text(diagnostic: &Value) -> String {
    let reply = match diagnostic["reply"].as_str() {
        Some("binder") => "binder",
        Some("protectedStatus") => "protectedStatus",
        _ => "unobserved",
    };
    format!("reply={reply} state={} ok={} nonceMatched={} ownerMatched={} ready={} owned={} instancePresent={} generationPresent={}",
        claim_state(&diagnostic["state"]), diagnostic["ok"] == true,
        diagnostic["nonceMatched"] == true, diagnostic["ownerMatched"] == true,
        diagnostic["ready"] == true, diagnostic["owned"] == true,
        diagnostic["instancePresent"] == true, diagnostic["generationPresent"] == true)
}

fn persist_claim_diagnostic(owner: &CanaryOwner, serial: &str, diagnostic: Value) -> anyhow::Result<()> {
    let path = owner.report.join("runtime-owner.json");
    let mut bytes = Vec::new();
    std::fs::File::open(&path)?.take(CHECKPOINT_LIMIT + 1).read_to_end(&mut bytes)?;
    anyhow::ensure!(bytes.len() as u64 <= CHECKPOINT_LIMIT, "helper owner record exceeds bound");
    let mut record: Value = serde_json::from_slice(&bytes)?;
    anyhow::ensure!(record["serial"].as_str() == Some(serial) && record["androidUser"] == 0
        && record["ownerId"].as_str() == Some(owner.owner_id.as_str())
        && record["nonce"].as_str() == Some(owner.nonce.as_str()) && record["state"] == "claimPending",
        "helper owner diagnostic identity mismatch");
    record["claimDiagnostic"] = diagnostic;
    if owner.instance != "-" && owner.generation != "-" {
        record["serviceInstance"] = json!(owner.instance);
        record["ownerGeneration"] = json!(owner.generation);
    }
    let bytes = serde_json::to_vec(&record)?;
    anyhow::ensure!(bytes.len() as u64 <= CHECKPOINT_LIMIT, "helper owner diagnostic exceeds bound");
    let temporary = owner.report.join(format!("runtime-owner-{}.tmp", uuid::Uuid::new_v4().simple()));
    let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(&temporary)?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    drop(file);
    std::fs::rename(temporary, path)?;
    Ok(())
}

const OWNER_CREDENTIAL_STORE: &str = "os-vault-v1";

fn owner_credential_name(serial: &str, owner_id: &str) -> anyhow::Result<String> {
    anyhow::ensure!((16..=128).contains(&owner_id.len())
        && owner_id.bytes().all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b)),
        "helper credential owner identity invalid");
    Ok(format!("android-helper-owner-v1:{}:{owner_id}", riviu_core::frame_sha256(serial.as_bytes())))
}

fn owner_record(state: &Path, serial: &str) -> anyhow::Result<Value> {
    let mut bytes = Vec::new();
    std::fs::File::open(state.join("runtime-owner.json"))?
        .take(CHECKPOINT_LIMIT + 1).read_to_end(&mut bytes)?;
    anyhow::ensure!(bytes.len() as u64 <= CHECKPOINT_LIMIT, HelperRecoveryRequired);
    let value: Value = serde_json::from_slice(&bytes)?;
    anyhow::ensure!(value["serial"].as_str() == Some(serial) && value["androidUser"] == 0
        && value["state"] == "claimPending", HelperRecoveryRequired);
    Ok(value)
}

async fn create_owner_credential(serial: &str, owner_id: &str) -> anyhow::Result<std::sync::Arc<str>> {
    let name = owner_credential_name(serial, owner_id)?;
    tokio::task::spawn_blocking(move || {
        let store = riviu_signing::CredentialStore::system().map_err(|_| anyhow!("helper OS credential store unavailable"))?;
        anyhow::ensure!(store.app_secret(&name).map_err(|_| anyhow!("helper OS credential lookup failed"))?.is_none(),
            "helper owner credential account already exists; no replacement");
        let token = format!("{}{}", uuid::Uuid::new_v4().simple(), uuid::Uuid::new_v4().simple());
        store.set_app_secret(&name, &token).map_err(|_| anyhow!("helper OS credential save failed"))?;
        anyhow::ensure!(store.app_secret(&name).map_err(|_| anyhow!("helper OS credential readback failed"))?.as_deref() == Some(token.as_str()),
            "helper OS credential readback unproved; no claim");
        Ok::<std::sync::Arc<str>, anyhow::Error>(token.into())
    }).await.map_err(|_| anyhow!("helper OS credential worker failed"))?
}

async fn load_owner_credential(serial: &str, record: &Value) -> anyhow::Result<std::sync::Arc<str>> {
    let owner_id = record["ownerId"].as_str().context("helper credential owner missing")?;
    let name = owner_credential_name(serial, owner_id)?;
    match record["tokenPersisted"].as_bool() {
        Some(true) => {
            anyhow::ensure!(record["credentialStore"].as_str() == Some(OWNER_CREDENTIAL_STORE), HelperRecoveryRequired);
            tokio::task::spawn_blocking(move || {
                let store = riviu_signing::CredentialStore::system().map_err(|_| anyhow!("helper OS credential store unavailable"))?;
                let token = store.app_secret(&name).map_err(|_| anyhow!("helper OS credential read failed"))?
                    .ok_or_else(|| anyhow!("helper owner credential missing; no replacement or claim"))?;
                anyhow::ensure!((32..=256).contains(&token.len())
                    && token.bytes().all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b)),
                    "helper owner credential invalid; no replacement or claim");
                Ok::<std::sync::Arc<str>, anyhow::Error>(token.into())
            }).await.map_err(|_| anyhow!("helper OS credential worker failed"))?
        },
        Some(false) => existing_helper_token(serial).map(std::sync::Arc::from)
            .ok_or_else(|| anyhow!("legacy helper owner credential lost with process; no replacement or claim")),
        None => Err(anyhow!("helper owner credential record malformed; no replacement or claim")),
    }
}

async fn retire_owner_credential(serial: &str, owner: &CanaryOwner) -> anyhow::Result<()> {
    // Keep the owner journal as a cold-retry anchor until vault retirement succeeds.
    let receipt = bounded_json(&release_path(owner))?;
    anyhow::ensure!(receipt["state"] == "released" && receipt["serial"] == serial
        && receipt["androidUser"] == 0 && receipt["ownerId"] == owner.owner_id
        && receipt["serviceInstance"] == owner.instance && receipt["ownerGeneration"] == owner.generation,
        HelperRecoveryRequired);
    anyhow::ensure!(!owner.report.join("ime-checkpoint.json").try_exists()?, "helper credential retirement waits for cleanup records");
    if owner.report.join("runtime-owner.json").try_exists()? {
        anyhow::ensure!(owner_record(&owner.report, serial)? == receipt["ownerRecord"], HelperRecoveryRequired);
    }
    let name = owner_credential_name(serial, &owner.owner_id)?;
    tokio::task::spawn_blocking(move || {
        let store = riviu_signing::CredentialStore::system().map_err(|_| anyhow!("released helper OS credential store unavailable"))?;
        store.set_app_secret(&name, "").map_err(|_| anyhow!("helper owner released; OS credential retirement failed"))?;
        anyhow::ensure!(store.app_secret(&name).map_err(|_| anyhow!("released helper OS credential readback failed"))?.is_none(),
            "helper owner released; OS credential retirement unproved");
        Ok::<(), anyhow::Error>(())
    }).await.map_err(|_| anyhow!("released helper OS credential worker failed"))?
}

async fn checked_shell(adb: &AdbProgram, serial: &str, command: &str) -> anyhow::Result<String> {
    let output = adb.shell_output(serial, command, Duration::from_secs(15)).await?;
    anyhow::ensure!(output.exit_code == 0 && output.stderr.trim().is_empty(), "helper runtime inventory unavailable");
    Ok(output.stdout)
}

async fn inventory(adb: &AdbProgram, serial: &str) -> anyhow::Result<(u64, u32, String)> {
    let dump = checked_shell(adb, serial, "dumpsys package com.riviu.agent").await?;
    let uid = dump.lines().find_map(|line| line.trim().strip_prefix("userId=")?.split_whitespace().next()?.parse::<u32>().ok()).context("helper package UID unreadable")?;
    let version = dump.lines().find_map(|line| line.trim().strip_prefix("versionCode=")?.split_whitespace().next()?.parse::<u64>().ok()).context("helper package version unreadable")?;
    anyhow::ensure!((10000..100000).contains(&uid), "helper package is not a user-0 application");
    let paths = checked_shell(adb, serial, "pm path com.riviu.agent").await?;
    let paths: Vec<_> = paths.lines().filter_map(|line| line.trim().strip_prefix("package:")).collect();
    anyhow::ensure!(paths.len() == 1, "helper APK path ambiguous");
    crate::adb::validate_device_path(paths[0])?;
    Ok((version, uid, paths[0].into()))
}

async fn matches_bundle(adb: &AdbProgram, serial: &str, path: &str, hash: &str) -> anyhow::Result<bool> {
    let output = checked_shell(adb, serial, &format!("sha256sum {}", crate::adb::quote_device_path(path))).await?;
    let observed = output.split_whitespace().next().context("installed helper digest missing")?;
    anyhow::ensure!(observed.len() == 64 && observed.bytes().all(|b| b.is_ascii_hexdigit()), "installed helper digest invalid");
    Ok(observed.eq_ignore_ascii_case(hash))
}

pub(super) async fn exchange(adb: &AdbProgram, serial: &str, owner: &CanaryOwner, body: Value) -> anyhow::Result<Value> {
    // All argv values are nonsecret validated metadata. Credential travels only in stdin.
    for value in [&owner.nonce, &owner.owner_id, &owner.instance, &owner.generation] {
        anyhow::ensure!(!value.is_empty() && value.len() <= 128 && value.bytes().all(|b| b.is_ascii_alphanumeric() || b"_.-".contains(&b)), "Binder identity metadata invalid");
    }
    anyhow::ensure!((10000..100000).contains(&owner.uid), "Binder package UID invalid");
    crate::adb::validate_device_path(&owner.apk_path)?;
    let bytes = serde_json::to_vec(&body)?;
    anyhow::ensure!(bytes.len() <= 4096, "bootstrap envelope too large");
    let mut payload = (bytes.len() as u32).to_be_bytes().to_vec();
    payload.extend(bytes);
    let command = format!("CLASSPATH={} app_process /system/bin com.riviu.agent.BootstrapBridge binder-shell {} {CERT_SHA256} {} {} {} {}", crate::adb::quote_device_path(&owner.apk_path), owner.uid, owner.nonce, owner.owner_id, owner.instance, owner.generation);
    let reply = adb.shell_secret_input(serial, &command, payload, Duration::from_secs(12)).await?;
    anyhow::ensure!(reply.len() >= 4, "Binder bootstrap reply missing");
    let size = u32::from_be_bytes(reply[..4].try_into()?) as usize;
    anyhow::ensure!(size <= 4096 && reply.len() == size + 4, "Binder bootstrap reply framing invalid");
    serde_json::from_slice(&reply[4..]).context("Binder bootstrap reply invalid")
}

// A write-ahead intent prevents replay even when the release ACK or receipt write is lost.
fn release_path(owner: &CanaryOwner) -> PathBuf {
    owner.report.join(format!("release-{}.json", owner.owner_id))
}

fn durable_json(path: &Path, value: &Value) -> anyhow::Result<()> {
    let temporary = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4().simple()));
    let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(&temporary)?;
    let bytes = serde_json::to_vec(value)?;
    anyhow::ensure!(bytes.len() as u64 <= CHECKPOINT_LIMIT, "helper durable receipt exceeds bound");
    file.write_all(&bytes)?;
    file.sync_all()?;
    drop(file);
    std::fs::rename(temporary, path)?;
    Ok(())
}

fn bounded_json(path: &Path) -> anyhow::Result<Value> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)?.take(CHECKPOINT_LIMIT + 1).read_to_end(&mut bytes)?;
    anyhow::ensure!(bytes.len() as u64 <= CHECKPOINT_LIMIT, HelperRecoveryRequired);
    Ok(serde_json::from_slice(&bytes)?)
}

/// Nonsecret record that one exact helper owner proved clipboard snapshot/restore through
/// its IME on one phone boot. Bound to the boot, the bundled APK and the owner's service
/// epoch, so a reboot, a new helper build, a service restart or a new claim never matches.
fn clipboard_proof_path(state: &Path) -> PathBuf {
    state.join("clipboard-proof.json")
}

fn clipboard_proof(serial: &str, boot_id: &str, apk_sha256: &str, owner: &CanaryOwner) -> Value {
    json!({"serial":serial,"androidUser":0,"bootId":boot_id,"apkSha256":apk_sha256,
        "ownerId":owner.owner_id,"serviceInstance":owner.instance,"ownerGeneration":owner.generation})
}

async fn boot_identity(adb: &AdbProgram, serial: &str) -> anyhow::Result<String> {
    let boot_id = checked_shell(adb, serial, "cat /proc/sys/kernel/random/boot_id").await?.trim().to_owned();
    anyhow::ensure!(!boot_id.is_empty() && boot_id.len() <= 128, "helper boot identity missing");
    Ok(boot_id)
}

/// The boot this exact owner epoch was claimed or proved on. Newer owner records carry
/// `bootId`; older ones are bound through the clipboard proof naming the same epoch.
fn owner_boot(state: &Path, serial: &str, record: &Value) -> Option<String> {
    if let Some(boot) = record["bootId"].as_str() {
        return Some(boot.to_owned());
    }
    let epoch = |field: &str| record[field].as_str().filter(|v| !v.is_empty() && *v != "-");
    let (Some(owner), Some(instance), Some(generation)) =
        (epoch("ownerId"), epoch("serviceInstance"), epoch("ownerGeneration")) else { return None };
    let proof = bounded_json(&clipboard_proof_path(state)).ok()?;
    if proof["serial"].as_str() != Some(serial) || proof["androidUser"] != 0
        || proof["ownerId"].as_str() != Some(owner) || proof["serviceInstance"].as_str() != Some(instance)
        || proof["ownerGeneration"].as_str() != Some(generation) {
        return None;
    }
    proof["bootId"].as_str().map(str::to_owned)
}

/// Proof that the helper process holding this owner's claim is gone. The helper keeps the
/// owner, its token and its epoch only in memory (`OwnerSession.java`), so a new phone boot,
/// an absent helper process or a helper pid other than the recorded one ends that claim for
/// good. `None` is no proof: an unanswered read or a same-boot, same-process helper keeps
/// the existing fence.
async fn owner_restart_proof(adb: &AdbProgram, serial: &str, state: &Path, record: &Value) -> Option<Value> {
    let boot_id = boot_identity(adb, serial).await.ok()?;
    if let Some(recorded) = owner_boot(state, serial, record) {
        if recorded != boot_id {
            return Some(json!({"kind":"reboot","recordedBootId":recorded,"bootId":boot_id}));
        }
    }
    let process = maintenance_process(adb, serial).await.ok()?;
    if process.is_empty() {
        return Some(json!({"kind":"processAbsent","bootId":boot_id}));
    }
    let recorded = record["helperPid"].as_str()?;
    (!process.split_whitespace().any(|pid| pid == recorded))
        .then(|| json!({"kind":"processRestarted","recordedPid":recorded,"pid":process,"bootId":boot_id}))
}

/// Settle a retained owner whose helper process is proven gone (`owner_restart_proof`),
/// without RELEASE, claim or action replay. Its owner record and release journal move to
/// `restarted/<ownerId>/` beside the proof, so the caller can claim afresh. A pending
/// clipboard/IME cleanup, an active maintenance or no proof answers `false` and keeps the
/// existing recovery path.
async fn settle_restarted_owner(adb: &AdbProgram, serial: &str, state: &Path) -> anyhow::Result<bool> {
    if !state.join("runtime-owner.json").try_exists()?
        || state.join("maintenance-active.json").try_exists()?
        || state.join("ime-checkpoint.json").try_exists()? {
        return Ok(false);
    }
    let Ok(record) = owner_record(state, serial) else { return Ok(false) };
    let Some(owner_id) = record["ownerId"].as_str() else { return Ok(false) };
    let Ok(credential) = owner_credential_name(serial, owner_id) else { return Ok(false) };
    let Some(proof) = owner_restart_proof(adb, serial, state, &record).await else { return Ok(false) };
    let archive = state.join("restarted").join(owner_id);
    std::fs::create_dir_all(&archive)?;
    let release_name = format!("release-{owner_id}.json");
    let mut journals = serde_json::Map::new();
    let mut release_port = None;
    for name in [release_name.as_str(), "runtime-owner.json"] {
        let Some(bytes) = journal_bytes(&state.join(name))? else {
            journals.insert(name.into(), Value::Null);
            continue;
        };
        if name == release_name {
            release_port = serde_json::from_slice::<Value>(&bytes).ok()
                .and_then(|receipt| receipt["hostPort"].as_u64())
                .and_then(|port| u16::try_from(port).ok()).filter(|port| *port > 0);
        }
        let saved = archive.join(name);
        match journal_bytes(&saved)? {
            Some(existing) => anyhow::ensure!(existing == bytes, HelperRecoveryRequired),
            None => {
                let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(&saved)?;
                file.write_all(&bytes)?;
                file.sync_all()?;
            }
        }
        journals.insert(name.into(), json!(riviu_core::frame_sha256(&bytes)));
    }
    durable_json(&archive.join("receipt.json"), &json!({"state":"settledByRestart","serial":serial,
        "androidUser":0,"ownerId":owner_id,"proof":proof,"journalHashes":journals,
        "exactReleaseProved":false,"actionReplay":false}))?;
    // The owner record goes last: a crash before it leaves the same settlement to finish.
    for name in [release_name.as_str(), "runtime-owner.json"] {
        if state.join(name).try_exists()? { std::fs::remove_file(state.join(name))?; }
    }
    // Housekeeping only: neither the old forward nor the dead owner's credential can act.
    if let Some(port) = release_port {
        if let Err(error) = remove_owned_forward(adb, serial, port).await {
            tracing::warn!(serial, port, error = %format!("{error:#}"), "chưa gỡ được cổng chuyển tiếp của phiên helper cũ; không ảnh hưởng phiên mới");
        }
    }
    if record["tokenPersisted"] == true {
        let retired = tokio::task::spawn_blocking(move || {
            let store = riviu_signing::CredentialStore::system().map_err(|_| anyhow!("helper OS credential store unavailable"))?;
            store.set_app_secret(&credential, "").map_err(|_| anyhow!("helper OS credential retirement failed"))
        }).await;
        if !matches!(retired, Ok(Ok(()))) {
            tracing::warn!(serial, "chưa thu hồi được khóa của phiên helper cũ trong kho khóa hệ điều hành; phiên cũ đã mất cùng tiến trình");
        }
    }
    let kind = proof["kind"].as_str().unwrap_or("unknown");
    tracing::info!(serial, proof = kind, "tự đối soát helper: tiến trình giữ phiên cũ đã mất (máy hoặc helper khởi động lại); bỏ phiên cũ và nhận phiên mới, không lặp thao tác nào");
    Ok(true)
}

/// Bind the durable owner to the helper process that just proved it, so a later process
/// restart is provable from the pid alone. Best effort: without it only a reboot or an
/// absent process proves the claim gone.
async fn record_owner_process(adb: &AdbProgram, serial: &str, owner: &CanaryOwner) {
    let recorded = async {
        let pids = maintenance_process(adb, serial).await?;
        let mut pids = pids.split_whitespace();
        let (Some(pid), None) = (pids.next(), pids.next()) else {
            anyhow::bail!("helper process not single");
        };
        let mut record = owner_record(&owner.report, serial)?;
        anyhow::ensure!(record["ownerId"] == owner.owner_id && record["serviceInstance"] == owner.instance
            && record["ownerGeneration"] == owner.generation, "helper owner record changed");
        if record["helperPid"].as_str() == Some(pid) { return Ok(()); }
        record["helperPid"] = json!(pid);
        durable_json(&owner.report.join("runtime-owner.json"), &record)
    }.await;
    if let Err(error) = recorded {
        tracing::warn!(serial, error = %format!("{error:#}"), "helper owner process not recorded; only a reboot or an absent process proves a restart");
    }
}

pub(super) async fn release(adb: &AdbProgram, serial: &str, owner: &CanaryOwner, token: &str, host_port: Option<u16>) -> anyhow::Result<()> {
    let path = release_path(owner);
    if path.try_exists()? {
        let receipt = bounded_json(&path)?;
        anyhow::ensure!(receipt["serial"] == serial && receipt["androidUser"] == 0
            && receipt["ownerId"] == owner.owner_id && receipt["serviceInstance"] == owner.instance
            && receipt["ownerGeneration"] == owner.generation && receipt["state"] == "released",
            "helper release intent unresolved or identity changed; no repeat RELEASE");
        return Ok(());
    }
    let record = owner_record(&owner.report, serial)?;
    anyhow::ensure!(record["ownerId"].as_str() == Some(owner.owner_id.as_str()), HelperRecoveryRequired);
    for (field, expected) in [("serviceInstance", &owner.instance), ("ownerGeneration", &owner.generation)] {
        if let Some(recorded) = record[field].as_str().filter(|v| !v.is_empty() && *v != "-") {
            anyhow::ensure!(recorded == expected.as_str(), "release epoch differs from durable owner");
        }
    }
    let mut intent = json!({"serial":serial,"androidUser":0,"ownerId":owner.owner_id,
        "serviceInstance":owner.instance,"ownerGeneration":owner.generation,"nonce":owner.nonce,
        "hostPort":host_port,"state":"releasePending","ownerRecord":record});
    let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(&path)?;
    file.write_all(&serde_json::to_vec(&intent)?)?;
    file.sync_all()?;
    drop(file);
    let reply = exchange(adb, serial, owner, json!({"action":"release","nonce":owner.nonce,"ownerId":owner.owner_id,"token":token,"serviceInstance":owner.instance,"ownerGeneration":owner.generation})).await?;
    anyhow::ensure!(reply["ok"] == true && reply["state"] == "released" && reply["nonce"].as_str() == Some(owner.nonce.as_str()) && reply["ownerId"].as_str() == Some(owner.owner_id.as_str()) && reply["serviceInstance"].as_str() == Some(owner.instance.as_str()) && reply["ownerGeneration"].as_str() == Some(owner.generation.as_str()), "exact helper owner release unresolved");
    intent["state"] = json!("released");
    durable_json(&path, &intent)?;
    // Keep the exact durable settlement after journal retirement and forward retries.
    Ok(())
}

pub(super) async fn remove_owned_forward(adb: &AdbProgram, serial: &str, port: u16) -> anyhow::Result<()> {
    let list = adb.device(serial, &["forward", "--list"], Duration::from_secs(10)).await?;
    let local = format!("tcp:{port}");
    let remote = format!("tcp:{DEVICE_PORT}");
    let mut present = false;
    for line in list.lines().filter(|line| !line.trim().is_empty()) {
        let fields: Vec<_> = line.split_whitespace().collect();
        anyhow::ensure!(fields.len() == 3, "forward inventory malformed; preserve release settlement");
        if fields[1] == local {
            anyhow::ensure!(fields[0] == serial && fields[2] == remote,
                "released helper forward was reassigned; preserve replacement");
            present = true;
        }
    }
    if present { frames::remove_forward(adb, serial, port).await?; }
    Ok(())
}

pub(super) async fn finish_release(serial: &str, owner: &CanaryOwner) -> anyhow::Result<()> {
    let receipt = bounded_json(&release_path(owner))?;
    anyhow::ensure!(receipt["state"] == "released" && receipt["serial"] == serial
        && receipt["ownerId"] == owner.owner_id && receipt["serviceInstance"] == owner.instance
        && receipt["ownerGeneration"] == owner.generation, HelperRecoveryRequired);
    anyhow::ensure!(!owner.report.join("ime-checkpoint.json").try_exists()?, HelperRecoveryRequired);
    let path = owner.report.join("runtime-owner.json");
    if path.try_exists()? {
        let current = owner_record(&owner.report, serial)?;
        anyhow::ensure!(current == receipt["ownerRecord"], HelperRecoveryRequired);
    }
    if receipt["ownerRecord"]["tokenPersisted"] == true {
        retire_owner_credential(serial, owner).await?;
    }
    if path.try_exists()? { std::fs::remove_file(path)?; }
    Ok(())
}

fn journal_bytes(path: &Path) -> anyhow::Result<Option<Vec<u8>>> {
    let file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let mut bytes = Vec::new();
    file.take(CHECKPOINT_LIMIT + 1).read_to_end(&mut bytes)?;
    anyhow::ensure!(bytes.len() as u64 <= CHECKPOINT_LIMIT, HelperRecoveryRequired);
    Ok(Some(bytes))
}

async fn maintenance_process(adb: &AdbProgram, serial: &str) -> anyhow::Result<String> {
    let output = adb.shell_output(serial, "pidof com.riviu.agent", Duration::from_secs(10)).await?;
    anyhow::ensure!(output.stderr.trim().is_empty()
        && ((output.exit_code == 0 && !output.stdout.trim().is_empty()) || (output.exit_code == 1 && output.stdout.trim().is_empty())),
        "helper process observation unavailable");
    let pids = output.stdout.trim();
    anyhow::ensure!(pids.len() <= 128 && pids.bytes().all(|b| b.is_ascii_digit() || b.is_ascii_whitespace()),
        "helper process observation invalid");
    Ok(pids.to_owned())
}

/// The keyboard the retained owner recorded in its durable checkpoint before it selected the
/// helper IME, bound to that owner. `None` when no valid record exists (older data).
fn recorded_ime_restore(state: &Path, serial: &str, owner_id: &str) -> Option<Value> {
    let checkpoint = pending_checkpoint(state, serial).ok()??;
    if checkpoint["ownerId"].as_str() != Some(owner_id) { return None; }
    let pending = &checkpoint["pending"];
    let previous = pending["previousIme"].as_str()
        .filter(|ime| validate_ime_id(ime).is_ok() && *ime != IME_ID)?;
    let was_enabled = pending["helperWasEnabled"].as_bool()?;
    Some(json!({"previousIme":previous,"helperWasEnabled":was_enabled}))
}

/// Keyboard handed back when the lost clipboard job's checkpoint names none that Android still
/// lists as enabled (older data, or a helper IME left selected with no job pending). Not a
/// guess: every helper checkpoint on the fleet PC (13–16/10/2026, SM-G955F/N, Android 9)
/// recorded either this Samsung Keyboard or GenFarmer's AdbKeyboard, and AdbKeyboard is never
/// assumed because it exists only where GenFarmer installed it. Chosen only when enabled.
const FLEET_DEFAULT_KEYBOARD: &str = "com.sec.android.inputmethod/.SamsungKeypad";

/// `effectIntent.automatic` of the supersession `ensure_runtime` runs itself (no app ledger).
const AUTO_SUPERSEDE: &str = "retainedOwnerRefused";

fn ime_enabled(enabled: &str, ime: &str) -> bool {
    enabled
        .trim()
        .split(':')
        .any(|entry| entry.split(';').next() == Some(ime))
}

/// `FLEET_DEFAULT_KEYBOARD` when Android lists it as enabled; otherwise nothing is chosen.
async fn fallback_keyboard(adb: &AdbProgram, serial: &str) -> anyhow::Result<Option<String>> {
    let enabled = adb
        .shell(serial, "settings get secure enabled_input_methods")
        .await?;
    Ok(ime_enabled(&enabled, FLEET_DEFAULT_KEYBOARD).then(|| FLEET_DEFAULT_KEYBOARD.to_owned()))
}

/// How a clipboard job whose answer was lost was proved unable to act any more.
struct ClipboardDebtProof {
    /// Fixed log label.
    label: &'static str,
    /// The owner that took the job still answers to this host's credential.
    owner_alive: bool,
}

/// Prove a lost clipboard job settled, or that it can no longer run. `None` keeps the fence.
///
/// The helper server that took a job keeps it until it settles and never evicts a named one
/// (`MainThreadJobs`). So while that owner still answers, its status of the ID is the proof,
/// and an ID it does not know never became a job. A different owner, a refused credential or
/// a proven helper restart means that server is gone: a new claim needs the old owner
/// released after its work settled (`AgentService`), or a new process.
async fn prove_clipboard_settled(
    client: &HelperClient,
    owner: &CanaryOwner,
    pending: &Value,
    record: Option<&Value>,
) -> Option<ClipboardDebtProof> {
    let alive = |label| {
        Some(ClipboardDebtProof {
            label,
            owner_alive: true,
        })
    };
    let gone = |label| {
        Some(ClipboardDebtProof {
            label,
            owner_alive: false,
        })
    };
    let nonce = uuid::Uuid::new_v4().simple().to_string();
    match client
        .post_json_answer("/v1/session/status", json!({"nonce":nonce}))
        .await
    {
        Ok((status, value)) if status.is_success() => {
            if value["ok"] != true || value["nonce"].as_str() != Some(nonce.as_str()) {
                return None;
            }
            if value["ownerId"].as_str() != Some(owner.owner_id.as_str())
                || value["serviceInstance"].as_str() != Some(owner.instance.as_str())
                || value["ownerGeneration"].as_str() != Some(owner.generation.as_str())
                || value["ownership"] != "owned"
            {
                return gone("ownerReplaced");
            }
        }
        Ok((status, _)) if status == reqwest::StatusCode::UNAUTHORIZED => {
            return gone("credentialRefused")
        }
        Ok(_) => return None,
        Err(_) => {
            let proof =
                owner_restart_proof(&client.adb, &client.serial, &owner.report, record?).await?;
            return gone(match proof["kind"].as_str() {
                Some("reboot") => "reboot",
                Some("processAbsent") => "processAbsent",
                _ => "processRestarted",
            });
        }
    }
    let Some(id) = pending["operationId"].as_str() else {
        // Recorded before submission: the keyboard may have switched, but no job exists.
        return if pending["phase"] == "switching" {
            alive("notSubmitted")
        } else {
            None
        };
    };
    if pending["serviceInstance"].as_str() != Some(owner.instance.as_str()) {
        return None;
    }
    let request = json!({"serviceInstance":owner.instance,"requestId":id});
    let deadline = tokio::time::Instant::now() + CLIPBOARD_SETTLEMENT;
    let mut path = "/v1/clipboard/jobs/status";
    loop {
        match client.post_clipboard_job(path, request.clone()).await {
            Ok(JobReply::Answer(reply)) => {
                if reply["ok"] != true
                    || reply["operationId"].as_str() != Some(id)
                    || reply["serviceInstance"].as_str() != Some(owner.instance.as_str())
                {
                    return None;
                }
                match reply["state"].as_str() {
                    Some("succeeded" | "failed" | "cancelled") => return alive("settled"),
                    // Only a job not yet dispatched can be cancelled; the call starts nothing.
                    Some("queued") => path = "/v1/clipboard/jobs/cancelQueued",
                    Some("running") => {}
                    _ => return None,
                }
            }
            Ok(JobReply::Refused("operation_unknown")) => return alive("notAdmitted"),
            Ok(JobReply::Refused("unauthorized")) => return gone("credentialRefused"),
            Ok(JobReply::Refused(_)) | Err(_) => {}
        }
        if tokio::time::Instant::now() >= deadline {
            return None;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

/// Hand back the keyboard a settled clipboard job left selected and return the one now
/// selected: the recorded one while Android still lists it, else `fallback_keyboard`. A
/// keyboard chosen by someone after ours is kept. Every change is read back.
async fn hand_back_keyboard(
    adb: &AdbProgram,
    serial: &str,
    pending: &Value,
) -> anyhow::Result<String> {
    let current = current_ime(adb, serial).await?;
    let keyboard = if current == IME_ID {
        let enabled = adb
            .shell(serial, "settings get secure enabled_input_methods")
            .await?;
        let recorded = pending["previousIme"].as_str().filter(|ime| {
            validate_ime_id(ime).is_ok() && *ime != IME_ID && ime_enabled(&enabled, ime)
        });
        let target = match recorded {
            Some(ime) => ime.to_owned(),
            None => fallback_keyboard(adb, serial)
                .await?
                .context("clipboard settlement keyboard unknown; not guessed")?,
        };
        set_ime(adb, serial, &target).await?;
        anyhow::ensure!(
            current_ime(adb, serial).await? == target,
            "clipboard settlement keyboard restore unverified"
        );
        target
    } else {
        current
    };
    if pending["helperWasEnabled"] == false {
        adb.shell(serial, &format!("ime disable {IME_ID}")).await?;
        let enabled = adb
            .shell(serial, "settings get secure enabled_input_methods")
            .await?;
        anyhow::ensure!(
            !ime_enabled(&enabled, IME_ID),
            "clipboard settlement helper IME disable unverified"
        );
    }
    Ok(keyboard)
}

/// One operator line per settled lost job; fixed labels and the keyboard id only.
fn log_clipboard_settlement(
    serial: &str,
    proof: &ClipboardDebtProof,
    pending: &Value,
    baseline_dropped: bool,
    keyboard: &str,
) {
    let action = pending["action"]
        .as_str()
        .filter(|action| matches!(*action, "get" | "snapshot" | "set" | "restoreSnapshot"))
        .unwrap_or("unknown");
    let outcome = if matches!(action, "get" | "snapshot") && !baseline_dropped {
        "không làm đổi clipboard"
    } else {
        "nội dung clipboard coi như không xác định"
    };
    tracing::info!(serial, proof = proof.label, action, keyboard,
        "tự đối soát clipboard helper: thao tác cũ đã khép ({outcome}); đã trả bàn phím, không lặp thao tác");
}

/// Once the recorded keyboard is back, a fresh observation reads it as the boot IME and no
/// longer derives the restore; the plan's restore is the only permitted carry-over.
fn carry_ime_restore(observed: &mut Value, plan: &Value) {
    if let (Some(restore), Some(map)) = (plan.get("imeRestore"), observed.as_object_mut()) {
        if !map.contains_key("imeRestore") && map.get("bootIme") == plan.get("bootIme") {
            map.insert("imeRestore".into(), restore.clone());
        }
    }
}

/// Select the recorded keyboard again and, when the helper IME was not enabled before the old
/// owner switched, disable it. Idempotent for a reconciliation that already restored it.
async fn restore_recorded_ime(adb: &AdbProgram, serial: &str, restore: &Value) -> anyhow::Result<()> {
    let previous = restore["previousIme"].as_str().context("maintenance IME restore target missing")?;
    anyhow::ensure!(validate_ime_id(previous).is_ok() && previous != IME_ID, "maintenance IME restore target invalid");
    let was_enabled = restore["helperWasEnabled"].as_bool().context("maintenance IME prior enablement missing")?;
    let current = current_ime(adb, serial).await?;
    if current == IME_ID { set_ime(adb, serial, previous).await?; }
    else { anyhow::ensure!(current == previous, "maintenance IME changed outside the old owner; preserved"); }
    anyhow::ensure!(current_ime(adb, serial).await? == previous, "maintenance IME restore unverified");
    if !was_enabled {
        adb.shell(serial, &format!("ime disable {IME_ID}")).await?;
        let enabled = adb.shell(serial, "settings get secure enabled_input_methods").await?;
        anyhow::ensure!(!enabled.trim().split(':').any(|entry| entry.split(';').next() == Some(IME_ID)),
            "maintenance IME enablement restore unverified");
    }
    Ok(())
}

fn finish_maintenance(state: &Path, archive: &Path, plan: &Value) -> anyhow::Result<()> {
    let owner_id = plan["oldOwnerId"].as_str().context("maintenance owner missing")?;
    owner_credential_name(plan["serial"].as_str().context("maintenance serial missing")?, owner_id)?;
    let expected = ["runtime-owner.json".to_owned(), "ime-checkpoint.json".to_owned(), format!("release-{owner_id}.json")];
    let journals = plan["journalHashes"].as_object().context("maintenance journal hashes missing")?;
    anyhow::ensure!(journals.len() == expected.len() && expected.iter().all(|name| journals.contains_key(name)), HelperRecoveryRequired);
    let active = state.join("maintenance-active.json");
    if active.try_exists()? {
        anyhow::ensure!(bounded_json(&active)?["plan"] == *plan, HelperRecoveryRequired);
    }
    // Prove every surviving byte before retiring any; archived effects remain unresolved forever.
    for name in &expected {
        let hash = &journals[name];
        if !hash.is_null() {
            let saved = journal_bytes(&archive.join(name))?.context("maintenance archive missing")?;
            anyhow::ensure!(hash.as_str() == Some(riviu_core::frame_sha256(&saved).as_str()), HelperRecoveryRequired);
        }
        if let Some(bytes) = journal_bytes(&state.join(name))? {
            anyhow::ensure!(hash.as_str() == Some(riviu_core::frame_sha256(&bytes).as_str()), HelperRecoveryRequired);
        }
    }
    for name in &expected {
        if !journals[name].is_null() && state.join(name).try_exists()? { std::fs::remove_file(state.join(name))?; }
    }
    if active.try_exists()? { std::fs::remove_file(active)?; }
    Ok(())
}

/// Holds the same locks as clipboard cleanup and HTTP dispatch until the driver
/// finishes maintenance. Inventory/Repair ownership is retained by the caller.
pub(crate) struct CachedMaintenanceGuard {
    client: HelperClient,
    _ime: tokio::sync::OwnedMutexGuard<()>,
    closed: tokio::sync::OwnedMutexGuard<bool>,
}

impl CachedMaintenanceGuard {
    pub(crate) fn released(&self) -> anyhow::Result<bool> {
        self.client.is_released_locked(*self.closed)
    }

    pub(crate) fn bind_plan(&self, plan: &Value) -> anyhow::Result<()> {
        let owner = self.client.canary.as_ref().context("cached maintenance owner missing")?;
        anyhow::ensure!(plan["serial"] == self.client.serial && plan["androidUser"] == 0
            && plan["oldOwnerId"] == owner.owner_id
            && plan["oldServiceInstance"] == owner.instance
            && plan["oldOwnerGeneration"] == owner.generation,
            "cached helper maintenance identity changed");
        if let Some(previous) = self.client.maintenance_plan.lock().as_ref() {
            anyhow::ensure!(previous == plan, "cached helper maintenance plan changed; reconcile same intent");
        }
        Ok(())
    }

    pub(crate) fn seal(&self, plan: &Value) -> anyhow::Result<()> {
        self.bind_plan(plan)?;
        // No await between the final binding and the shared permanent fence.
        // Dropping/cancelling execute must never re-enable a stale clone.
        *self.client.maintenance_plan.lock() = Some(plan.clone());
        self.client.clipboard_qualified.store(false, std::sync::atomic::Ordering::Release);
        Ok(())
    }

    pub(crate) async fn retire_forward(&self) -> anyhow::Result<()> {
        remove_owned_forward(&self.client.adb, &self.client.serial, self.client.host_port).await
    }
}

impl HelperClient {
    pub(crate) async fn lock_cached_maintenance(&self, root: Option<&Path>) -> anyhow::Result<CachedMaintenanceGuard> {
        self.lock_cached_maintenance_for_execute(root, false).await
    }

    pub(crate) async fn lock_cached_maintenance_for_execute(&self, root: Option<&Path>, retryable_busy: bool) -> anyhow::Result<CachedMaintenanceGuard> {
        let ime = ime_lock(&self.serial).try_lock_owned()
            .map_err(|_| if retryable_busy { anyhow!(HelperMaintenanceBusyBeforeDispatch) }
                else { anyhow!("helper clipboard work still draining") })?;
        let closed = self.lifecycle.clone().try_lock_owned()
            .map_err(|_| if retryable_busy { anyhow!(HelperMaintenanceBusyBeforeDispatch) }
                else { anyhow!("helper request still draining") })?;
        let guard = CachedMaintenanceGuard { client: self.clone(), _ime: ime, closed };
        if guard.released()? { return Ok(guard); }
        anyhow::ensure!(self.production_runtime
            && (self.maintenance_plan.lock().is_some()
                || self.retained_failure.load(std::sync::atomic::Ordering::Acquire)),
            "live helper owner must settle normally");
        let owner = self.canary.as_ref().context("cached maintenance owner missing")?;
        let state = state_path(root.context("helper state directory missing")?, &self.serial)?;
        anyhow::ensure!(owner.report == state, "cached helper state binding changed");
        let mut durable_settled = true;
        if let Some(bytes) = journal_bytes(&state.join("ime-checkpoint.json"))? {
            let saved: Value = serde_json::from_slice(&bytes)?;
            anyhow::ensure!(saved["serial"] == self.serial && saved["androidUser"] == 0
                && saved["ownerId"] == owner.owner_id && saved["serviceInstance"] == owner.instance
                && saved["generation"] == owner.generation,
                "helper durable clipboard cleanup unresolved");
            durable_settled = saved.get("pending").is_some_and(Value::is_null)
                && saved.get("baseline").is_some_and(Value::is_null);
        }
        if self.cleanup_is_pending() || !durable_settled {
            // A clipboard job or snapshot lives only in the helper process that took it. When
            // that process is proven gone the obligation can never settle; maintenance archives
            // it unresolved and replays nothing. Without proof the fence stays.
            let proved = match owner_record(&state, &self.serial) {
                Ok(record) => owner_restart_proof(&self.adb, &self.serial, &state, &record).await,
                Err(_) => None,
            };
            anyhow::ensure!(proved.is_some(), if self.cleanup_is_pending() { "helper clipboard/baseline cleanup unresolved" }
                else { "helper durable clipboard cleanup unresolved" });
        }
        // The original record must match this client, not merely the requested serial.
        // After a settled receipt retires it, only the already sealed same-plan retry
        // may finish removal of this client's exact forward.
        if state.join("runtime-owner.json").try_exists()? {
            let record = owner_record(&state, &self.serial)?;
            anyhow::ensure!(record["ownerId"] == owner.owner_id
                && record["serviceInstance"] == owner.instance
                && record["ownerGeneration"] == owner.generation,
                "cached helper durable owner changed");
        } else {
            anyhow::ensure!(self.maintenance_plan.lock().is_some(), "cached helper durable owner missing");
        }
        Ok(guard)
    }

    /// Name an adbd identity refusal hidden beneath an unclassified recovery fence.
    ///
    /// Acquisition settles releases, loads the owner credential or reconnects a cached owner
    /// before it reaches the identity proof, so a root adbd surfaced only as
    /// `recovery_required` while maintenance named `shell_root` on the same phone. This is
    /// read-only (`id -u`, `am get-current-user`). The fence and the original chain stay; an
    /// unanswered probe keeps the original error.
    pub(crate) async fn name_identity_refusal(
        adb: &AdbProgram,
        serial: &str,
        error: anyhow::Error,
    ) -> anyhow::Error {
        if !error.is::<HelperRecoveryRequired>() || error.is::<HelperShellIdentityRefused>() {
            return error;
        }
        match require_shell_identity(adb, serial)
            .await
            .map_err(|probe| probe.downcast::<HelperShellIdentityRefused>())
        {
            Err(Ok(refusal)) => anyhow!(refusal)
                .context(HelperRecoveryRequired)
                .context(format!("{error:#}")),
            Ok(()) | Err(Err(_)) => error,
        }
    }

    /// Observation only. The caller supplies the existing effect identity, never a retry ID.
    pub(crate) async fn prepare_maintenance(adb: &AdbProgram, serial: &str, apk: Option<&Path>, root: Option<&Path>, maintenance_id: &str, effect_intent: Value) -> anyhow::Result<Value> {
        let state = state_path(root.context("helper state directory missing")?, serial)?;
        anyhow::ensure!(!state.join("maintenance-active.json").try_exists().context(HelperRecoveryRequired)?, HelperRecoveryRequired);
        Self::observe_maintenance(adb, serial, apk, root, maintenance_id, effect_intent).await
    }

    // Inventory only: callers retain the active-intent fence and decide whether dispatch is allowed.
    async fn observe_maintenance(adb: &AdbProgram, serial: &str, apk: Option<&Path>, root: Option<&Path>, maintenance_id: &str, effect_intent: Value) -> anyhow::Result<Value> {
        anyhow::ensure!((16..=128).contains(&maintenance_id.len())
            && maintenance_id.bytes().all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b)),
            "invalid helper maintenanceId");
        anyhow::ensure!(effect_intent.as_object().is_some_and(|v| !v.is_empty())
            && serde_json::to_vec(&effect_intent)?.len() <= 4096, "existing effect identity required");
        let state = state_path(root.context("helper state directory missing")?, serial)?;
        let record = owner_record(&state, serial).map_err(|error| {
            if error.downcast_ref::<std::io::Error>()
                .is_some_and(|cause| cause.kind() == std::io::ErrorKind::NotFound)
            {
                anyhow!(HelperMaintenanceRetainedOwnerMissing)
            } else {
                error
            }
        })?;
        let owner_id = record["ownerId"].as_str().context("old helper owner missing")?;
        owner_credential_name(serial, owner_id)?;
        require_shell_identity_for_recovery(adb, serial).await?;
        let (version, uid, installed_path) = inventory(adb, serial).await?;
        let bundled_hash = riviu_core::frame_sha256(&std::fs::read(apk.context("bundled helper missing")?)?);
        let installed_hash = checked_shell(adb, serial, &format!("sha256sum {}", crate::adb::quote_device_path(&installed_path))).await?
            .split_whitespace().next().context("installed helper hash missing")?.to_owned();
        // Only the recorded trusted package or the current trusted bundle may be superseded.
        anyhow::ensure!(installed_hash.len() == 64 && installed_hash.bytes().all(|b| b.is_ascii_hexdigit())
            && (record["apkSha256"].as_str() == Some(installed_hash.as_str()) || installed_hash == bundled_hash),
            "maintenance helper APK provenance changed");
        let current = current_ime(adb, serial).await?;
        validate_ime_id(&current)?;
        // A helper IME left selected by the old owner is handed back only to the keyboard that
        // owner wrote down before switching (`with_ime_mode`), never to a guess.
        let (boot_ime, ime_restore) = if current == IME_ID {
            let restore = recorded_ime_restore(&state, serial, owner_id)
                .ok_or_else(|| anyhow!("maintenance cannot guess or restore a lost boot IME"))?;
            (restore["previousIme"].as_str().context("recorded boot IME missing")?.to_owned(), Some(restore))
        } else {
            (current, None)
        };
        let boot_id = checked_shell(adb, serial, "cat /proc/sys/kernel/random/boot_id").await?.trim().to_owned();
        anyhow::ensure!(!boot_id.is_empty() && boot_id.len() <= 128, "maintenance boot identity missing");
        let process = maintenance_process(adb, serial).await?;
        let mut journals = serde_json::Map::new();
        for name in ["runtime-owner.json".to_owned(), "ime-checkpoint.json".to_owned(), format!("release-{owner_id}.json")] {
            let bytes = journal_bytes(&state.join(&name))?;
            journals.insert(name, bytes.as_ref().map(|b| json!(riviu_core::frame_sha256(b))).unwrap_or(Value::Null));
        }
        let mut plan = json!({"schema":1,"maintenanceId":maintenance_id,"serial":serial,"androidUser":0,
            "bundledApkSha256":bundled_hash,"installedApkSha256":installed_hash,"installedApkPath":installed_path,
            "packageUid":uid,"packageVersion":version,"certificateSha256":CERT_SHA256,
            "oldOwnerId":owner_id,"oldServiceInstance":record["serviceInstance"],"oldOwnerGeneration":record["ownerGeneration"],
            "oldClaimNonce":record["nonce"],"journalHashes":journals,"bootIme":boot_ime,"bootId":boot_id,
            "observedProcess":process,"effectIntent":effect_intent,"disposition":"operatorAuthorizedSupersession",
            "exactReleaseProved":false,"clipboardRestorationProved":false});
        // Absent unless needed, so a plan prepared by an older build still binds.
        if let Some(restore) = ime_restore { plan["imeRestore"] = restore; }
        Ok(plan)
    }

    /// Exact-plan maintenance only, authorized by operator confirmation or automatic setup policy.
    pub(crate) async fn execute_maintenance(adb: &AdbProgram, serial: &str, apk: Option<&Path>, root: Option<&Path>, plan: Value, operator_authorized: bool, observation_only: bool) -> anyhow::Result<Value> {
        Self::execute_maintenance_with_cache(adb, serial, apk, root, plan, operator_authorized, observation_only, None).await
    }

    // The arguments are execute_maintenance's exact plan tuple plus the cached guard; a bag struct would touch every maintenance caller.
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn execute_maintenance_with_cache(adb: &AdbProgram, serial: &str, apk: Option<&Path>, root: Option<&Path>, plan: Value, operator_authorized: bool, observation_only: bool, cached: Option<&CachedMaintenanceGuard>) -> anyhow::Result<Value> {
        anyhow::ensure!(operator_authorized, "operator authorization required for helper supersession");
        let state = state_path(root.context("helper state directory missing")?, serial)?;
        let id = plan["maintenanceId"].as_str().context("maintenanceId missing")?;
        anyhow::ensure!((16..=128).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b)), "invalid maintenanceId");
        anyhow::ensure!(plan["serial"] == serial && plan["androidUser"] == 0, HelperRecoveryRequired);
        let archive = state.join("maintenance").join(id);
        let active = state.join("maintenance-active.json");
        let receipt_path = archive.join("receipt.json");
        if receipt_path.try_exists()? {
            let receipt = bounded_json(&receipt_path)?;
            anyhow::ensure!(receipt["plan"] == plan && receipt["state"] == "superseded", HelperRecoveryRequired);
            if let Some(cached) = cached { cached.seal(&plan)?; }
            finish_maintenance(&state, &archive, &plan)?;
            return Ok(receipt);
        }
        let reconciling = active.try_exists()?;
        if reconciling {
            let intent = bounded_json(&active)?;
            anyhow::ensure!(intent["state"] == "supersessionPending" && intent["plan"] == plan,
                "maintenance active plan changed; no replay or retirement");
        }
        let mut observed = Self::observe_maintenance(adb, serial, apk, root, id, plan["effectIntent"].clone()).await?;
        carry_ime_restore(&mut observed, &plan);
        if reconciling || observation_only {
            anyhow::ensure!(observed["observedProcess"] == "", "maintenance effect unresolved; process present; no replay");
            // Process disappearance is the only permitted change; retain the original plan verbatim.
            observed["observedProcess"] = plan["observedProcess"].clone();
        }
        anyhow::ensure!(observed == plan, "helper maintenance binding changed; no replay or retirement");
        if let Some(cached) = cached { cached.seal(&plan)?; }
        if !reconciling { std::fs::create_dir_all(&archive)?; }
        let journals = plan["journalHashes"].as_object().context("maintenance journal binding missing")?;
        for (name, hash) in journals {
            if hash.is_null() {
                anyhow::ensure!(journal_bytes(&archive.join(name))?.is_none(), HelperRecoveryRequired);
                continue;
            }
            let bytes = journal_bytes(&state.join(name))?.context("maintenance journal disappeared")?;
            anyhow::ensure!(hash.as_str() == Some(riviu_core::frame_sha256(&bytes).as_str()), HelperRecoveryRequired);
            let saved = archive.join(name);
            if saved.try_exists()? {
                anyhow::ensure!(journal_bytes(&saved)?.as_ref() == Some(&bytes), HelperRecoveryRequired);
            } else {
                anyhow::ensure!(!reconciling, "maintenance archive missing; reconciliation fenced");
                let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(&saved)?;
                file.write_all(&bytes)?;
                file.sync_all()?;
            }
        }
        if !reconciling {
            let intent = json!({"state":"supersessionPending","plan":plan});
            let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(&active)?;
            file.write_all(&serde_json::to_vec(&intent)?)?;
            file.sync_all()?;
            drop(file);
        }
        // The boot keyboard goes back before the stop: Android may rebind a selected IME and
        // restart the helper process the stop must prove gone. The manual fix on 13/10/2026
        // (SM-G955N/F) used the same order and settled with the process absent.
        if let Some(restore) = plan.get("imeRestore") {
            restore_recorded_ime(adb, serial, restore).await?;
        }
        // An active intent never grants replay authority, even if the earlier dispatch is unknown.
        let stop_dispatched = !reconciling && !observation_only && !plan["observedProcess"].as_str().context("process binding missing")?.is_empty();
        if stop_dispatched {
            checked_shell(adb, serial, "am force-stop --user 0 com.riviu.agent").await?;
        }
        let mut settled = Self::observe_maintenance(adb, serial, apk, root, id, plan["effectIntent"].clone()).await?;
        carry_ime_restore(&mut settled, &plan);
        anyhow::ensure!(settled["observedProcess"] == "", "superseded helper process still present");
        settled["observedProcess"] = plan["observedProcess"].clone();
        anyhow::ensure!(settled == plan, "maintenance binding changed; settlement unresolved");
        let stop_disposition = if reconciling { "alreadyAbsentOnReconciliationPriorDispatchUnknown" }
            else if observation_only { "alreadyAbsentObservationOnlyPriorDispatchUnknown" }
            else if stop_dispatched { "dispatched" } else { "alreadyAbsent" };
        let receipt = json!({"state":"superseded","plan":plan,"exactReleaseProved":false,
            "clipboardRestorationProved":false,"oldObligations":"archivedUnresolved","packageProcessAbsent":true,
            "bootImeRestored":plan.get("imeRestore").is_some(),
            "stopDispatchedThisAttempt":stop_dispatched,"stopDisposition":stop_disposition});
        durable_json(&receipt_path, &receipt)?;
        finish_maintenance(&state, &archive, &plan)?;
        Ok(receipt)
    }

    /// Retire this cached owner when its helper process is proven gone, so the caller drops
    /// it and claims afresh instead of fencing the phone until an operator steps in. Busy
    /// locks, pending clipboard work, a maintenance fence, a changed record or no proof all
    /// answer `false`, leaving the existing reconnect and recovery path untouched.
    pub(crate) async fn retire_if_restarted(&self) -> anyhow::Result<bool> {
        if !self.production_runtime || self.require_unfenced().is_err() { return Ok(false); }
        let Ok(_serial) = ime_lock(&self.serial).try_lock_owned() else { return Ok(false) };
        let Ok(mut closed) = self.lifecycle.clone().try_lock_owned() else { return Ok(false) };
        if *closed || self.cleanup_is_pending() { return Ok(false); }
        let Some(owner) = self.canary.as_ref() else { return Ok(false) };
        let Ok(record) = owner_record(&owner.report, &self.serial) else { return Ok(false) };
        if record["ownerId"] != owner.owner_id || record["serviceInstance"] != owner.instance
            || record["ownerGeneration"] != owner.generation {
            return Ok(false);
        }
        if !settle_restarted_owner(&self.adb, &self.serial, &owner.report).await? { return Ok(false); }
        // Every clone now reads as released: the owner record is gone and no cleanup is pending.
        *closed = true;
        if let Err(error) = remove_owned_forward(&self.adb, &self.serial, self.host_port).await {
            tracing::warn!(serial = %self.serial, error = %format!("{error:#}"), "chưa gỡ được cổng chuyển tiếp của phiên helper cũ; không ảnh hưởng phiên mới");
        }
        Ok(true)
    }

    /// Settle this owner's clipboard job whose answer was lost (`prove_clipboard_settled`) and
    /// hand back the keyboard it kept. The caller holds this phone's IME lock. Best effort:
    /// without proof the debt and its fence stay exactly as they were.
    pub(super) async fn settle_clipboard_debt_locked(&self) {
        if let Err(error) = self.try_settle_clipboard_debt().await {
            tracing::warn!(serial = %self.serial, error = %format!("{error:#}"), "chưa tự đối soát được thao tác clipboard cũ của helper; giữ bàn phím và bản ghi, không lặp thao tác");
        }
    }

    async fn try_settle_clipboard_debt(&self) -> anyhow::Result<()> {
        if !self.production_runtime || self.require_unfenced().is_err() {
            return Ok(());
        }
        let Some(pending) = self.pending_clipboard.lock().clone() else {
            return Ok(());
        };
        let owner = self
            .canary
            .as_ref()
            .context("clipboard settlement owner missing")?;
        let record = owner_record(&owner.report, &self.serial).ok();
        let Some(proof) = prove_clipboard_settled(self, owner, &pending, record.as_ref()).await
        else {
            return Ok(());
        };
        let keyboard = hand_back_keyboard(&self.adb, &self.serial, &pending).await?;
        // A runtime owner takes a snapshot only inside its clipboard qualification, which ended
        // with this lost job. Nobody restores it any more: the clipboard is simply unknown.
        // The durable record goes first, so a failed removal leaves the debt still fenced.
        let checkpoint = owner.report.join("ime-checkpoint.json");
        if checkpoint.try_exists()? {
            std::fs::remove_file(checkpoint)?;
        }
        self.pending_clipboard.lock().take();
        let baseline_dropped = self.clipboard_baseline.lock().take().is_some();
        log_clipboard_settlement(&self.serial, &proof, &pending, baseline_dropped, &keyboard);
        Ok(())
    }

    /// Session open reaches a cached owner first in `settle_released_runtime`.
    async fn settle_clipboard_debt(&self) {
        if !self.production_runtime || self.pending_clipboard.lock().is_none() {
            return;
        }
        // A clipboard task still running holds this lock and settles its own job.
        let Ok(_serial) = ime_lock(&self.serial).try_lock_owned() else {
            return;
        };
        self.settle_clipboard_debt_locked().await;
    }

    /// The helper IME selected with no job of this owner pending (left by an older build or
    /// owner): select `fallback_keyboard` and read it back. The caller holds the IME lock.
    pub(super) async fn hand_back_stray_helper_keyboard(&self) -> anyhow::Result<String> {
        let keyboard = fallback_keyboard(&self.adb, &self.serial).await?
            .context("previous IME is already the helper and no keyboard is recorded; reconciliation required")?;
        set_ime(&self.adb, &self.serial, &keyboard).await?;
        anyhow::ensure!(
            current_ime(&self.adb, &self.serial).await? == keyboard,
            "helper IME hand-back unverified"
        );
        tracing::info!(serial = %self.serial, keyboard = %keyboard, "bàn phím mặc định đang là bàn phím Riviu Helper mà không có thao tác nào chờ; đã trả về bàn phím mặc định của đội máy");
        Ok(keyboard)
    }

    /// Release an owner whose helper ledger is spent or nearly so (`CLIPBOARD_LEDGER_RENEWAL`)
    /// between sessions, with no clipboard work pending, so the caller claims afresh with an
    /// empty one. The exact release and its journal are the normal ones.
    async fn renew_spent_owner(&self) -> anyhow::Result<bool> {
        let jobs = self
            .clipboard_jobs
            .load(std::sync::atomic::Ordering::Acquire);
        if !(self
            .owner_renewal_due
            .load(std::sync::atomic::Ordering::Acquire)
            || jobs >= CLIPBOARD_LEDGER_RENEWAL)
            || self.cleanup_is_pending()
            || self.require_unfenced().is_err()
        {
            return Ok(false);
        }
        tracing::info!(serial = %self.serial, jobs, "phiên helper đã dùng gần hết sổ thao tác clipboard; trả đúng phiên này và nhận phiên mới, không lặp thao tác nào");
        self.clone().shutdown().await?;
        Ok(true)
    }

    /// A durable release (including an unresolved intent) fences authenticated reconnect.
    /// Session open reaches a cached owner here first, so a clipboard job whose answer was lost
    /// is settled before the caller reads the cleanup fence, and a spent owner is renewed.
    pub(crate) async fn settle_released_runtime(&self) -> anyhow::Result<bool> {
        let result = async {
            if !self.production_runtime { return Ok(false); }
            let owner = self.canary.as_ref().context("runtime owner missing")?;
            self.settle_clipboard_debt().await;
            if !release_path(owner).try_exists()? {
                return self.renew_spent_owner().await;
            }
            // A release journal against a process proven gone settles by that restart; a
            // repeated RELEASE could never reach the claim that died with the process.
            if self.retire_if_restarted().await? { return Ok(true); }
            self.clone().shutdown().await?;
            self.is_released().await
        }.await;
        result.context(HelperRecoveryRequired).context("cached helper release settlement unresolved")
    }

    async fn settle_runtime_release(adb: &AdbProgram, serial: &str, state: &Path) -> anyhow::Result<()> {
        let result = async {
            if !state.join("runtime-owner.json").try_exists()? { return Ok(()); }
            let record = owner_record(state, serial)?;
            let owner_id = record["ownerId"].as_str().context("helper owner missing")?;
            owner_credential_name(serial, owner_id)?;
            let path = state.join(format!("release-{owner_id}.json"));
            if !path.try_exists()? { return Ok(()); }
            let receipt = bounded_json(&path)?;
            anyhow::ensure!(receipt["state"] == "released" && receipt["ownerRecord"] == record
                && receipt["serial"] == serial && receipt["androidUser"] == 0,
                "helper release pending; retain exact intent, no replay");
            let owner = CanaryOwner { owner_id: owner_id.into(),
                instance: receipt["serviceInstance"].as_str().context("release instance missing")?.into(),
                generation: receipt["ownerGeneration"].as_str().context("release generation missing")?.into(),
                nonce: receipt["nonce"].as_str().context("release nonce missing")?.into(),
                socket: String::new(), uid: 0, apk_path: String::new(), report: state.into() };
            if let Some(port) = receipt["hostPort"].as_u64() {
                let port = u16::try_from(port)?;
                anyhow::ensure!(port > 0, HelperRecoveryRequired);
                remove_owned_forward(adb, serial, port).await?;
            }
            finish_release(serial, &owner).await
        }.await;
        result.context(HelperRecoveryRequired).context("durable helper release settlement unresolved")
    }

    pub(crate) async fn fence_package_preparation(adb: &AdbProgram, serial: &str, root: Option<&Path>) -> anyhow::Result<()> {
        let state = state_path(root.context("helper durable state directory missing")?, serial)?;
        anyhow::ensure!(!state.join("maintenance-active.json").try_exists().context(HelperRecoveryRequired)?, HelperRecoveryRequired);
        Self::settle_runtime_release(adb, serial, &state).await?;
        anyhow::ensure!(!state.join("runtime-owner.json").try_exists()?
            && pending_checkpoint(&state, serial)?.is_none(), HelperRecoveryRequired);
        Ok(())
    }

    /// Cached qualification only, never a new live-device proof.
    pub(crate) fn cached_runtime_ready(&self) -> bool {
        self.production_runtime
            && self.is_scoped_canary()
            && self.clipboard_qualified.load(std::sync::atomic::Ordering::Acquire)
            && !self.cleanup_is_pending()
            && self.canary.as_ref().is_some_and(|owner| {
                self.validate_runtime_owner_record(&owner.report).is_ok()
                    && pending_checkpoint(&owner.report, &self.serial).is_ok_and(|pending| pending.is_none())
                    && owner_record(&owner.report, &self.serial).is_ok_and(|record| {
                        record["serviceInstance"].as_str() == Some(owner.instance.as_str())
                            && record["ownerGeneration"].as_str() == Some(owner.generation.as_str())
                    })
            })
    }

    /// Read-only admission proof for the cached owner and its exact durable record.
    /// A warm resume rotates the request nonce; live health checks the service epoch.
    pub(crate) fn validate_runtime_owner_record(&self, state: &Path) -> anyhow::Result<()> {
        anyhow::ensure!(self.production_runtime, HelperRecoveryRequired);
        let owner = self.canary.as_ref().ok_or_else(|| anyhow!(HelperRecoveryRequired))?;
        anyhow::ensure!(owner.report == state, HelperRecoveryRequired);
        anyhow::ensure!(!release_path(owner).try_exists()?
            && !state.join("maintenance-active.json").try_exists().context(HelperRecoveryRequired)?, HelperRecoveryRequired);
        let mut bytes = Vec::new();
        std::fs::File::open(state.join("runtime-owner.json"))?
            .take(CHECKPOINT_LIMIT + 1).read_to_end(&mut bytes)?;
        anyhow::ensure!(bytes.len() as u64 <= CHECKPOINT_LIMIT, HelperRecoveryRequired);
        let record: Value = serde_json::from_slice(&bytes)?;
        anyhow::ensure!(record["serial"].as_str() == Some(self.serial.as_str()) && record["androidUser"] == 0
            && record["ownerId"].as_str() == Some(owner.owner_id.as_str()) && record["state"] == "claimPending",
            HelperRecoveryRequired);
        Ok(())
    }

    /// Authenticate a previously admitted owner after desktop restart without
    /// claiming, qualifying clipboard, switching IME, or adding a cached client.
    pub(crate) async fn verify_runtime_owner_readiness(adb: AdbProgram, serial: &str, apk: Option<&Path>, state: &Path) -> anyhow::Result<()> {
        let result = async {
            anyhow::ensure!(pending_checkpoint(state, serial)?.is_none(), "helper cleanup record pending");
            let record = owner_record(state, serial)?;
            anyhow::ensure!(record["tokenPersisted"] == true
                && record["credentialStore"].as_str() == Some(OWNER_CREDENTIAL_STORE),
                "helper cold readiness requires exact persisted owner credential");
            let metadata = |field: &str| -> anyhow::Result<String> {
                let value = record[field].as_str().context("helper owner epoch field missing")?;
                anyhow::ensure!((16..=128).contains(&value.len())
                    && value.bytes().all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b)),
                    "helper owner epoch field invalid");
                Ok(value.into())
            };
            let owner_id = metadata("ownerId")?;
            anyhow::ensure!(!state.join(format!("release-{owner_id}.json")).try_exists()?
                && !state.join("maintenance-active.json").try_exists().context(HelperRecoveryRequired)?, HelperRecoveryRequired);
            let instance = metadata("serviceInstance")?;
            let generation = metadata("ownerGeneration")?;
            require_shell_identity(&adb, serial).await?;
            let hash = riviu_core::frame_sha256(&std::fs::read(apk.context("bundled helper missing for cold readiness")?)?);
            let (version, uid, apk_path) = inventory(&adb, serial).await?;
            anyhow::ensure!(version >= MIN_VERSION && record["apkSha256"].as_str() == Some(hash.as_str())
                && matches_bundle(&adb, serial, &apk_path, &hash).await?, "helper cold readiness APK binding unproved");
            let token = load_owner_credential(serial, &record).await?;
            let owner = CanaryOwner { owner_id, instance, generation,
                nonce: uuid::Uuid::new_v4().simple().to_string(), socket: String::new(), uid, apk_path, report: state.into() };
            let port = forward_helper(&adb, serial).await?;
            let observed = async {
                let mut client = Self::at_with_token(adb.clone(), serial, port, token)?;
                client.production_runtime = true;
                client.canary = Some(owner.clone());
                client.require_canary_identity().await
            }.await;
            // Read-only qualification never releases the live helper owner.
            let cleanup = frames::remove_forward(&adb, serial, port).await;
            match (observed, cleanup) {
                (Ok(()), Ok(())) => {},
                (Err(error), Ok(())) => return Err(error),
                (Ok(()), Err(error)) => return Err(error).context("cold readiness owned forward cleanup unproved"),
                (Err(error), Err(cleanup)) => return Err(error).context(format!("cold readiness owned forward cleanup unproved ({cleanup:#})")),
            }
            anyhow::ensure!(pending_checkpoint(state, serial)?.is_none(), "helper cleanup changed during cold readiness");
            let current = owner_record(state, serial)?;
            anyhow::ensure!(current["ownerId"].as_str() == Some(owner.owner_id.as_str())
                && current["serviceInstance"].as_str() == Some(owner.instance.as_str())
                && current["ownerGeneration"].as_str() == Some(owner.generation.as_str())
                && current["apkSha256"] == record["apkSha256"]
                && current["tokenPersisted"] == true && current["credentialStore"] == record["credentialStore"],
                "helper owner record changed during cold readiness");
            Ok::<(), anyhow::Error>(())
        }.await;
        result.map_err(|error| fence_identity_refusal(error).unwrap_or_else(|error| anyhow!(HelperRecoveryRequired)
            .context(format!("helper cold readiness unproved; no claim ({error:#})"))))
    }

    /// Record the clipboard proof this owner just made. Best effort: without the record the
    /// next desktop start only proves the clipboard again.
    async fn record_clipboard_proof(&self, apk_sha256: &str) {
        let Some(owner) = self.canary.as_ref() else { return };
        let recorded = async {
            let boot_id = boot_identity(&self.adb, &self.serial).await?;
            durable_json(&clipboard_proof_path(&owner.report), &clipboard_proof(&self.serial, &boot_id, apk_sha256, owner))
        }.await;
        if let Err(error) = recorded {
            tracing::warn!(serial = %self.serial, error = %format!("{error:#}"), "helper clipboard proof not recorded; next start proves it again");
        }
    }

    /// Adopt the clipboard proof that this same owner and service epoch made on this boot
    /// with this APK, instead of switching the IME again on every desktop start. Caption
    /// entry still proves IME selection and clipboard settlement on each use.
    async fn reuse_clipboard_proof(&self, apk_sha256: &str) -> bool {
        let Some(owner) = self.canary.as_ref() else { return false };
        let Ok(recorded) = bounded_json(&clipboard_proof_path(&owner.report)) else { return false };
        let Ok(boot_id) = boot_identity(&self.adb, &self.serial).await else { return false };
        if recorded != clipboard_proof(&self.serial, &boot_id, apk_sha256, owner) {
            return false;
        }
        self.clipboard_qualified.store(true, std::sync::atomic::Ordering::Release);
        tracing::info!(serial = %self.serial, "helper clipboard proof reused for this boot; no IME switch");
        true
    }

    /// Called under the driver's inventory writer and the caller's control lease.
    pub(crate) async fn ensure_runtime(adb: AdbProgram, serial: &str, apk: Option<&Path>, state_dir: Option<&Path>) -> anyhow::Result<Self> {
        let error = match Self::ensure_runtime_attempt(adb.clone(), serial, apk, state_dir).await {
            Ok(client) => return Ok(client),
            Err(error) => error,
        };
        match Self::supersede_refused_owner(&adb, serial, apk, state_dir, &error).await {
            Ok(true) => Self::ensure_runtime_attempt(adb, serial, apk, state_dir).await,
            Ok(false) => Err(error),
            Err(supersede) => {
                tracing::warn!(serial, error = %format!("{supersede:#}"), "tự thay phiên helper chưa xong; giữ bản ghi để đối soát, không lặp thao tác");
                Err(error)
            }
        }
    }

    /// Owner decision 2026-10-12 (no buttons): a retained owner this host can neither resume
    /// nor settle -- its claim met `owner_conflict`, or the helper refuses its credential after
    /// a desktop or USB-box restart -- is superseded by the existing maintenance here, under
    /// the caller's device lease, so session open and media staging claim afresh instead of
    /// failing (r127/r129, 15–16/10/2026, phones 35 and 29). At most once per phone boot
    /// (durable marker written first); never with a clipboard job unsettled, another
    /// maintenance active, an adbd identity refusal or the phone off adb. Its own interrupted
    /// plan is only reconciled, never stopped again. No claim or action is replayed.
    async fn supersede_refused_owner(
        adb: &AdbProgram,
        serial: &str,
        apk: Option<&Path>,
        state_dir: Option<&Path>,
        error: &anyhow::Error,
    ) -> anyhow::Result<bool> {
        if !error.is::<HelperRecoveryRequired>()
            || error.is::<HelperShellIdentityRefused>()
            || crate::adb::is_transport_lost(error)
        {
            return Ok(false);
        }
        let (Some(root), Some(apk)) = (state_dir, apk) else {
            return Ok(false);
        };
        let state = state_path(root, serial)?;
        let active = state.join("maintenance-active.json");
        if active.try_exists()? {
            let intent = bounded_json(&active)?;
            if intent["plan"]["effectIntent"]["automatic"] != AUTO_SUPERSEDE {
                return Ok(false);
            }
            Self::execute_maintenance(
                adb,
                serial,
                Some(apk),
                Some(root),
                intent["plan"].clone(),
                true,
                true,
            )
            .await?;
            tracing::info!(serial, "tự đối soát helper: đã khép lần tự thay phiên dở trước đó (chỉ quan sát, không dừng helper lần nữa)");
            return Ok(true);
        }
        if !state.join("runtime-owner.json").try_exists()?
            || state.join("ime-checkpoint.json").try_exists()?
        {
            return Ok(false);
        }
        let boot_id = boot_identity(adb, serial).await?;
        let marker = state.join("auto-supersede.json");
        // An unreadable marker cannot prove a new boot, so it counts as spent.
        if marker.try_exists()?
            && bounded_json(&marker).map_or(true, |spent| {
                spent["bootId"].as_str() == Some(boot_id.as_str())
            })
        {
            return Ok(false);
        }
        let maintenance_id = uuid::Uuid::new_v4().simple().to_string();
        // Written before any effect: a crash after it spends this boot, never repeats a stop.
        durable_json(
            &marker,
            &json!({"serial":serial,"androidUser":0,"bootId":boot_id,"maintenanceId":maintenance_id}),
        )?;
        tracing::info!(serial, "tự đối soát helper: không nhận lại được phiên helper cũ (helper đang có chủ phiên khác hoặc từ chối khóa); tự thay phiên một lần cho lần khởi động này của máy, không lặp thao tác nào");
        // A helper IME left selected with no checkpoint cannot be planned around; hand back
        // the fleet keyboard first, exactly as the next clipboard use would.
        if current_ime(adb, serial).await? == IME_ID {
            let keyboard = fallback_keyboard(adb, serial)
                .await?
                .context("maintenance cannot guess or restore a lost boot IME")?;
            set_ime(adb, serial, &keyboard).await?;
            anyhow::ensure!(
                current_ime(adb, serial).await? == keyboard,
                "maintenance IME restore unverified"
            );
        }
        let plan = Self::prepare_maintenance(
            adb,
            serial,
            Some(apk),
            Some(root),
            &maintenance_id,
            json!({"automatic":AUTO_SUPERSEDE,"bootId":boot_id}),
        )
        .await?;
        Self::execute_maintenance(adb, serial, Some(apk), Some(root), plan, true, false).await?;
        tracing::info!(serial, "tự đối soát helper: đã thay phiên helper cũ (tiến trình helper đã dừng, bản ghi cũ lưu trữ ở trạng thái chưa đối soát); nhận phiên mới");
        Ok(true)
    }

    async fn ensure_runtime_attempt(
        adb: AdbProgram,
        serial: &str,
        apk: Option<&Path>,
        state_dir: Option<&Path>,
    ) -> anyhow::Result<Self> {
        let state = state_path(
            state_dir.context("helper durable state directory missing")?,
            serial,
        )?;
        anyhow::ensure!(
            !state
                .join("maintenance-active.json")
                .try_exists()
                .context(HelperRecoveryRequired)?,
            HelperRecoveryRequired
        );
        // A reboot or helper restart already ended the retained claim; its unresolved
        // release or resume can never settle against a process that no longer exists.
        settle_restarted_owner(&adb, serial, &state).await.context(HelperRecoveryRequired)?;
        Self::settle_runtime_release(&adb, serial, &state).await?;
        // Recovery precedes any installation, service restart or owner claim.
        if let Some(checkpoint) = pending_checkpoint(&state, serial).map_err(|_| anyhow!(HelperRecoveryRequired))? {
            Self::recover_runtime_checkpoint(adb.clone(), serial, &state, checkpoint).await.map_err(|error| {
                fence_identity_refusal(error).unwrap_or_else(|error| anyhow!(HelperRecoveryRequired)
                    .context(format!("helper pending cleanup unresolved ({error:#})")))
            })?;
            // An owner the recovery proved gone is settled by its restart, not resumed.
            settle_restarted_owner(&adb, serial, &state)
                .await
                .context(HelperRecoveryRequired)?;
        }
        // A prior failed claim/attach is never replaced by a new claim. Its credential
        // may have died with the host process; explicit owner reconciliation is required.
        if state.join("runtime-owner.json").try_exists().context(HelperRecoveryRequired)? {
            return Self::resume_runtime_owner(adb, serial, apk, &state).await.map_err(|error| {
                fence_identity_refusal(error).unwrap_or_else(|error| anyhow!(HelperRecoveryRequired)
                    .context(format!("helper owner resume unresolved ({error:#})")))
            });
        }
        let apk = apk.context("compatible bundled helper APK missing")?;
        let hash = riviu_core::frame_sha256(&std::fs::read(apk)?);
        // Refused before any install or claim: no owner journal exists yet.
        require_shell_identity(&adb, serial).await?;
        let installed = package_installed(&adb, serial).await?;
        let old = if installed { Some(inventory(&adb, serial).await?) } else { None };
        let compatible = if let Some((version, _, path)) = &old { *version >= MIN_VERSION && matches_bundle(&adb, serial, path, &hash).await? } else { false };
        if !compatible {
            install_apk(&adb, serial, apk).await?;
            anyhow::ensure!(package_installed(&adb, serial).await?, "helper install readback missing");
        }
        let (version, uid, apk_path) = inventory(&adb, serial).await?;
        anyhow::ensure!(version >= MIN_VERSION && matches_bundle(&adb, serial, &apk_path, &hash).await?, "bundled release helper compatibility unverified");
        std::fs::create_dir_all(&state)?;
        // Binds the owner to this boot, so a later reboot proves its claim gone.
        let boot_id = boot_identity(&adb, serial).await?;
        let mut owner = CanaryOwner {
            owner_id: uuid::Uuid::new_v4().simple().to_string(), nonce: uuid::Uuid::new_v4().simple().to_string(),
            instance: "-".into(), generation: "-".into(), socket: String::new(), uid, apk_path, report: state,
        };
        // Read back durable credentials before recording or dispatching the claim.
        let token = create_owner_credential(serial, &owner.owner_id).await.map_err(|error| {
            anyhow!(HelperRecoveryRequired).context(format!("helper owner credential preparation unproved; no claim ({error:#})"))
        })?;
        let attempt = async {
            let mut intent = std::fs::OpenOptions::new().write(true).create_new(true).open(owner.report.join("runtime-owner.json"))?;
            intent.write_all(&serde_json::to_vec(&json!({"serial":serial,"androidUser":0,"ownerId":owner.owner_id,"nonce":owner.nonce,"apkSha256":hash,"bootId":boot_id,"state":"claimPending","tokenPersisted":true,"credentialStore":OWNER_CREDENTIAL_STORE}))?)?;
            intent.sync_all()?;
            drop(intent);
            // Tokenless foreground startup keeps the owned service alive after the bridge unbinds.
            let started = adb.shell_output(serial, &format!("am start-foreground-service -n {SERVICE}"), Duration::from_secs(15)).await?;
            anyhow::ensure!(started.exit_code == 0 && !started.stdout.contains("Error") && !started.stderr.contains("Error"), "release helper foreground start refused");
            let body = json!({"action":"claim","nonce":owner.nonce,"ownerId":owner.owner_id,"token":token.as_ref(),"serviceInstance":"-","ownerGeneration":"-"});
            // Claim is sent exactly once. Any lost ACK uses this same credential and owner.
            let claimed = exchange(&adb, serial, &owner, body).await;
            // If a proved claim precedes a transport failure, release only that exact owner.
            if let Ok(identity) = &claimed {
                if identity["ok"] == true && identity["nonce"].as_str() == Some(owner.nonce.as_str()) && identity["ownerId"].as_str() == Some(owner.owner_id.as_str()) && identity["state"] == "ready" {
                    owner.instance = identity["serviceInstance"].as_str().filter(|v| !v.is_empty() && *v != "-").context("helper claim instance missing")?.into();
                    owner.generation = identity["ownerGeneration"].as_str().filter(|v| !v.is_empty() && *v != "-").context("helper claim generation missing")?.into();
                }
                // Diagnostics must not orphan a successfully proved service owner.
                if let Err(error) = persist_claim_diagnostic(&owner, serial, claim_diagnostic(identity, &owner, "binder")) {
                    if owner.instance != "-" && owner.generation != "-" {
                        release(&adb, serial, &owner, token.as_ref(), None).await.context("claim diagnostic failed; exact-owner release unresolved")?;
                    }
                    return Err(error);
                }
            }
            let port = match forward_helper(&adb, serial).await {
                Ok(port) => port,
                Err(error) => {
                    if owner.instance != "-" { release(&adb, serial, &owner, token.as_ref(), None).await?; }
                    return Err(error);
                }
            };
            let mut client = match Self::at_with_token(adb.clone(), serial, port, token.clone()) {
                Ok(client) => client,
                Err(error) => {
                    if owner.instance != "-" { release(&adb, serial, &owner, token.as_ref(), Some(port)).await?; }
                    frames::remove_forward(&adb, serial, port).await?;
                    return Err(error);
                }
            };
            client.production_runtime = true;
            let attach = async {
                let (identity, reply) = match claimed {
                    Ok(value) => (value, "binder"),
                    Err(bootstrap_error) => {
                        match client.post_json("/v1/session/status", json!({"nonce":owner.nonce})).await {
                            Ok(value) => (value, "protectedStatus"),
                            Err(status_error) => {
                                // Transport and JSON parser errors contain no payload/token bytes.
                                // Preserve the first failure instead of hiding it behind reconciliation.
                                return Err(bootstrap_error).context(format!(
                                    "claim ACK lost; protected owner status unavailable ({status_error:#}); no reclaim"
                                ));
                            }
                        }
                    },
                };
                let diagnostic = claim_diagnostic(&identity, &owner, reply);
                let description = claim_diagnostic_text(&diagnostic);
                let owner_proved = identity["ok"] == true && identity["nonce"].as_str() == Some(owner.nonce.as_str()) && identity["ownerId"].as_str() == Some(owner.owner_id.as_str()) && (identity["state"] == "ready" || identity["ownership"] == "owned");
                if !owner_proved {
                    if reply == "protectedStatus" { persist_claim_diagnostic(&owner, serial, diagnostic)?; }
                    anyhow::bail!("helper claim owner not proved: {description}");
                }
                owner.instance = identity["serviceInstance"].as_str().filter(|s| !s.is_empty() && *s != "-").context("helper instance missing")?.into();
                owner.generation = identity["ownerGeneration"].as_str().filter(|s| !s.is_empty() && *s != "-").context("helper generation missing")?.into();
                client.canary = Some(owner.clone());
                // Existing attach cleanup now owns this proved identity if diagnostic I/O fails.
                if reply == "protectedStatus" { persist_claim_diagnostic(&owner, serial, diagnostic)?; }
                await_helper_ready(|| async { client.require_canary_identity().await?; let status = client.require_status().await?; anyhow::ensure!(status.features.iter().any(|f| f == "secureBootstrapBinderShell") && REQUIRED_FEATURES.iter().all(|required| status.features.iter().any(|f| f == required)), "release helper features missing"); Ok(()) }).await?;
                client.qualify_clipboard_roundtrip().await?;
                client.record_clipboard_proof(&hash).await;
                record_owner_process(&adb, serial, &owner).await;
                Ok::<(), anyhow::Error>(())
            }.await;
            if let Err(error) = attach {
                if client.cleanup_is_pending() {
                    return Err(anyhow!(HelperRecoveryRequired)).context("helper qualification cleanup pending; owner and forward retained");
                }
                if client.canary.is_some() { release(&adb, serial, &owner, token.as_ref(), Some(port)).await.context("failed attach exact-owner release unresolved")?; }
                frames::remove_forward(&adb, serial, port).await?;
                return Err(error);
            }
            Ok::<Self, anyhow::Error>(client)
        }.await;
        match attempt {
            Ok(client) => Ok(client),
            Err(error) => {
                // An initial rejected claim is as unresolved as the next resume. Never
                // expose a UI-only session while its helper owner intent still fences it.
                if owner.report.join("runtime-owner.json").try_exists().unwrap_or(true) {
                    return Err(anyhow!(HelperRecoveryRequired))
                        .context(format!("helper claim pending; owner intent retained ({error:#})"));
                }
                Err(error)
            },
        }
    }

    async fn resume_runtime_owner(adb: AdbProgram, serial: &str, apk: Option<&Path>, state: &Path) -> anyhow::Result<Self> {
        let record = owner_record(state, serial)?;
        let token = load_owner_credential(serial, &record).await?;
        require_shell_identity_for_recovery(&adb, serial).await?;
        let hash = riviu_core::frame_sha256(&std::fs::read(apk.context("bundled helper unavailable for resume")?)?);
        let (version, uid, apk_path) = inventory(&adb, serial).await?;
        anyhow::ensure!(version >= MIN_VERSION && record["apkSha256"].as_str() == Some(hash.as_str()) && matches_bundle(&adb, serial, &apk_path, &hash).await?, HelperRecoveryRequired);
        let owner_id = record["ownerId"].as_str().context("pending claim owner missing")?.to_owned();
        let nonce = uuid::Uuid::new_v4().simple().to_string();
        let port = forward_helper(&adb, serial).await?;
        let mut client = Self::at_with_token(adb.clone(), serial, port, token.clone())?;
        client.production_runtime = true;
        let proof = async {
            // The exact recorded owner's credential only. No claim, restart, install or action replay.
            let value = client.post_json("/v1/session/status", json!({"nonce":nonce})).await?;
            anyhow::ensure!(value["ok"] == true && value["nonce"].as_str() == Some(nonce.as_str()) && value["ownerId"].as_str() == Some(owner_id.as_str()) && value["ownership"] == "owned", HelperRecoveryRequired);
            for field in ["serviceInstance", "ownerGeneration"] {
                let observed = value[field].as_str().filter(|v| !v.is_empty() && *v != "-")
                    .context("resumed helper epoch missing")?;
                if let Some(expected) = record[field].as_str().filter(|v| !v.is_empty() && *v != "-") {
                    anyhow::ensure!(observed == expected, "persisted helper epoch changed; no adoption");
                } else {
                    anyhow::ensure!(record[field].is_null() || record[field] == "-", HelperRecoveryRequired);
                }
            }
            client.canary = Some(CanaryOwner { owner_id, nonce, instance: value["serviceInstance"].as_str().context("owner instance missing")?.into(), generation: value["ownerGeneration"].as_str().context("owner generation missing")?.into(), socket: String::new(), uid, apk_path, report: state.into() });
            let mut persisted = record.clone();
            persisted["serviceInstance"] = value["serviceInstance"].clone();
            persisted["ownerGeneration"] = value["ownerGeneration"].clone();
            anyhow::ensure!(owner_record(state, serial)? == record, HelperRecoveryRequired);
            durable_json(&state.join("runtime-owner.json"), &persisted)?;
            client.require_canary_identity().await?;
            if let Some(owner) = client.canary.as_ref() { record_owner_process(&adb, serial, owner).await; }
            if !client.reuse_clipboard_proof(&hash).await {
                client.qualify_clipboard_roundtrip().await?;
                client.record_clipboard_proof(&hash).await;
            }
            Ok::<(), anyhow::Error>(())
        }.await;
        if let Err(error) = proof {
            let diagnostic = claim_diagnostic_text(&record["claimDiagnostic"]);
            if !client.cleanup_is_pending() {
                frames::remove_forward(&adb, serial, port).await
                    .with_context(|| format!("helper owner resume cleanup unresolved; initial {diagnostic}"))?;
            }
            return Err(anyhow!(HelperRecoveryRequired))
                .context(format!("helper owner resume unproved; initial {diagnostic} ({error:#})"));
        }
        Ok(client)
    }

    /// Rebind a lost host forward without claiming, reinstalling or replaying any operation.
    pub(crate) async fn reconnect_runtime(&self) -> anyhow::Result<Option<Self>> {
        let result = async {
            if !self.production_runtime { return Ok(None); }
            self.require_unfenced()?;
            let owner = self.canary.clone().context("runtime owner missing")?;
            let port = forward_helper(&self.adb, &self.serial).await?;
            let mut client = self.clone();
            client.host_port = port;
            client.base = format!("http://127.0.0.1:{port}");
            let proof = client.require_canary_identity().await;
            if let Err(error) = proof {
                frames::remove_forward(&self.adb, &self.serial, port).await?;
                return Err(error).context("runtime reconnect owner unproved; no reclaim");
            }
            // Shared state retains qualifications and pending cleanup for this exact owner.
            anyhow::ensure!(client.canary.as_ref().is_some_and(|current| current.instance == owner.instance && current.generation == owner.generation), "reconnect owner changed");
            if self.host_port != port { frames::remove_forward(&self.adb, &self.serial, self.host_port).await?; }
            Ok(Some(client))
        }.await;
        result.context(HelperRecoveryRequired).context("retained helper reconnect unresolved; no reclaim")
    }

    async fn recover_runtime_checkpoint(adb: AdbProgram, serial: &str, state: &Path, checkpoint: Value) -> anyhow::Result<()> {
        let _serial = ime_lock(serial).lock_owned().await;
        require_shell_identity(&adb, serial).await?;
        let (_, uid, apk_path) = inventory(&adb, serial).await?;
        let field = |name: &str| checkpoint[name].as_str().map(str::to_owned).context("pending cleanup owner metadata missing");
        let owner = CanaryOwner { owner_id: field("ownerId")?, instance: field("serviceInstance")?, generation: field("generation")?, nonce: uuid::Uuid::new_v4().simple().to_string(), socket: String::new(), uid, apk_path, report: state.into() };
        let record = owner_record(state, serial)?;
        anyhow::ensure!(record["ownerId"].as_str() == Some(owner.owner_id.as_str()), HelperRecoveryRequired);
        let token = load_owner_credential(serial, &record).await?;
        let port = forward_helper(&adb, serial).await?;
        let mut client = Self::at_with_token(adb.clone(), serial, port, token.clone())?;
        client.canary = Some(owner.clone());
        client.production_runtime = true;
        let result = async {
            let pending = &checkpoint["pending"];
            // Settled, never replayed: `prove_clipboard_settled` decides, the job is not resent.
            let proof = prove_clipboard_settled(&client, &owner, pending, Some(&record))
                .await
                .context("pending clipboard job not provably settled; no replay")?;
            // Clipboard contents are never persisted, and a runtime owner's snapshot belongs to
            // the qualification this lost job ended: it is dropped, the clipboard is unknown.
            if proof.owner_alive {
                client.require_canary_identity().await?;
            }
            let keyboard = hand_back_keyboard(&adb, serial, pending).await?;
            std::fs::remove_file(state.join("ime-checkpoint.json"))?;
            log_clipboard_settlement(
                serial,
                &proof,
                pending,
                !checkpoint["baseline"].is_null(),
                &keyboard,
            );
            // An owner that still answers is released exactly; a gone one is settled by its
            // restart (`settle_restarted_owner`) or by supersession, never released twice.
            if proof.owner_alive {
                release(&adb, serial, &owner, token.as_ref(), Some(port)).await?;
            }
            Ok::<bool, anyhow::Error>(proof.owner_alive)
        }.await;
        let cleanup = frames::remove_forward(&adb, serial, port).await;
        let released = result?;
        cleanup?;
        if released {
            finish_release(serial, &owner).await?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn maintenance_missing_retained_owner_is_explicit_and_remains_failed() {
        let root = std::env::temp_dir().join(format!("helper-maintenance-missing-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        let error = HelperClient::prepare_maintenance(
            &AdbProgram::at(root.join("never-run-adb")), "fixture", None, Some(&root),
            "fixture_maintenance_1234", json!({"operation":"fixture"}),
        ).await.expect_err("missing retained owner must not become success");
        std::fs::remove_dir_all(&root).unwrap();
        assert_eq!(error.to_string(), "HelperMaintenanceRetainedOwnerMissing");
    }

    /// Fixture adb that answers only the two identity probes; anything else exits 97.
    fn identity_adb(root: &Path, uid: &str, user: &str) -> AdbProgram {
        #[cfg(windows)]
        let path = {
            let path = root.join("identity-adb.cmd");
            std::fs::write(&path, format!(concat!(
                "@echo off\r\n",
                "if \"%~4\"==\"id -u\" (\r\n  echo {uid}\r\n  exit /b 0\r\n)\r\n",
                "if \"%~4\"==\"am get-current-user\" (\r\n  echo {user}\r\n  exit /b 0\r\n)\r\n",
                "exit /b 97\r\n"), uid = uid, user = user)).unwrap();
            path
        };
        #[cfg(not(windows))]
        let path = {
            use std::os::unix::fs::PermissionsExt;
            let path = root.join("identity-adb");
            std::fs::write(&path, format!(concat!(
                "#!/bin/sh\n",
                "case \"$4\" in\n",
                "'id -u') echo {uid} ;;\n",
                "'am get-current-user') echo {user} ;;\n",
                "*) exit 97 ;;\n",
                "esac\n"), uid = uid, user = user)).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
            path
        };
        AdbProgram::at(path)
    }

    #[tokio::test]
    async fn shell_identity_refusals_are_typed_and_keep_the_recovery_fence() {
        let serial = "fixture-identity";
        for (uid, user, expected) in [
            ("0", "0", HelperShellIdentityRefused::Root),
            ("1000", "0", HelperShellIdentityRefused::UnexpectedUid),
            ("2000", "10", HelperShellIdentityRefused::AndroidUser),
        ] {
            let root = std::env::temp_dir().join(format!("helper-identity-{}", uuid::Uuid::new_v4()));
            let state = state_path(&root, serial).unwrap();
            std::fs::create_dir_all(&state).unwrap();
            let adb = identity_adb(&root, uid, user);
            // Fresh claim: refused before install or claim, not a retained-owner fence.
            let apk = root.join("helper.apk");
            std::fs::write(&apk, b"fixture-apk-not-for-device").unwrap();
            let fresh = HelperClient::ensure_runtime(adb.clone(), serial, Some(&apk), Some(&root)).await
                .err().expect("identity mismatch must refuse a fresh claim");
            assert!(format!("{fresh:#}").contains(&format!("{}:", expected.marker())), "{uid}/{user}: {fresh:#}");
            assert_eq!(fresh.downcast_ref::<HelperShellIdentityRefused>(), Some(&expected));
            assert!(!fresh.is::<HelperRecoveryRequired>());
            assert!(!state.join("runtime-owner.json").exists(), "no owner intent before identity proof");
            // Retained owner: maintenance preparation names the identity and keeps the fence.
            std::fs::write(state.join("runtime-owner.json"), serde_json::to_vec(&json!({
                "serial":serial,"androidUser":0,"state":"claimPending","ownerId":"fixture_owner_0123456789",
                "serviceInstance":"fixture_instance_012345","ownerGeneration":"fixture_generation_012345"})).unwrap()).unwrap();
            let error = HelperClient::prepare_maintenance(&adb, serial, Some(&apk), Some(&root),
                "fixture_maintenance_1234", json!({"operation":"fixture"})).await
                .expect_err("identity mismatch must not become a maintenance plan");
            let text = format!("{error:#}");
            std::fs::remove_dir_all(&root).unwrap();
            assert_eq!(HelperShellIdentityRefused::from_chain_text(&text), Some(expected), "{uid}/{user}: {text}");
            assert!(error.is::<HelperRecoveryRequired>(), "retained owner stays fenced: {text}");
        }
        // The operator cannot type adb: the root text names the in-app step instead.
        for refusal in HelperShellIdentityRefused::ALL {
            assert!(
                !refusal.operator_message().contains("adb -s"),
                "{refusal:?}"
            );
        }
        assert!(HelperShellIdentityRefused::Root
            .operator_message()
            .contains("tự tắt quyền root của adb"));
    }


    #[tokio::test]
    async fn cold_retained_release_blocks_optional_helper_fallback() {
        let root = std::env::temp_dir().join(format!("helper-release-admission-{}", uuid::Uuid::new_v4()));
        let serial = "fixture-release-admission";
        let state = state_path(&root, serial).unwrap();
        std::fs::create_dir_all(&state).unwrap();
        let owner_id = "fixture_owner_0123456789";
        let record = json!({"serial":serial,"androidUser":0,"ownerId":owner_id,
            "state":"claimPending","serviceInstance":"fixture_instance_012345",
            "ownerGeneration":"fixture_generation_012345"});
        let owner_bytes = serde_json::to_vec(&record).unwrap();
        let owner_path = state.join("runtime-owner.json");
        std::fs::write(&owner_path, &owner_bytes).unwrap();
        let release_path = state.join(format!("release-{owner_id}.json"));
        let mut failures = Vec::new();
        for (name, release_state, stale_owner) in [
            ("pending release", "releasePending", false),
            ("stale exact-owner receipt", "released", true),
        ] {
            let mut receipt = json!({"serial":serial,"androidUser":0,"ownerId":owner_id,
                "state":release_state,"ownerRecord":record,
                "serviceInstance":"fixture_instance_012345",
                "ownerGeneration":"fixture_generation_012345","nonce":"fixture_nonce_012345"});
            if stale_owner {
                receipt["ownerRecord"]["ownerGeneration"] = json!("replaced_generation_012345");
            }
            let release_bytes = serde_json::to_vec(&receipt).unwrap();
            std::fs::write(&release_path, &release_bytes).unwrap();
            // Missing APK and unusable ADB cannot convert a retained lifecycle into
            // optional helper absence or dispatch a new claim/release.
            let error = HelperClient::ensure_runtime(
                AdbProgram::at(root.join("never-run-adb")), serial, None, Some(&root),
            ).await.err().expect("retained release must reject runtime admission");
            assert_eq!(std::fs::read(&owner_path).unwrap(), owner_bytes);
            assert_eq!(std::fs::read(&release_path).unwrap(), release_bytes);
            if !error.is::<HelperRecoveryRequired>() {
                failures.push(format!("{name}: {error:#}"));
            }
        }
        std::fs::remove_dir_all(&root).unwrap();
        assert!(failures.is_empty(), "open_session must fence retained lifecycle errors: {failures:?}");
    }
    const FIXTURE_KEYBOARD: &str = "com.genfarmer.uiautomator/.AdbKeyboard";

    /// Fixture adb for one Android 9 phone with the helper installed. The default and enabled
    /// IMEs live in `ime.txt`/`enabled.txt`, the helper pid in `pid.txt` (absent: no process);
    /// any other command is logged to `unexpected.txt` and exits 97.
    fn maintenance_adb(root: &Path, apk_sha256: &str) -> AdbProgram {
        #[cfg(windows)]
        let path = {
            let path = root.join("maintenance-adb.cmd");
            std::fs::write(&path, format!(concat!(
                "@echo off\r\n",
                "set \"c=%~4\"\r\n",
                "set \"d=%~dp0\"\r\n",
                "if \"%c%\"==\"id -u\" (echo 2000& exit /b 0)\r\n",
                "if \"%c%\"==\"am get-current-user\" (echo 0& exit /b 0)\r\n",
                "if \"%c%\"==\"dumpsys package com.riviu.agent\" (echo userId=10123& echo versionCode=9& exit /b 0)\r\n",
                "if \"%c%\"==\"pm path com.riviu.agent\" (echo package:/data/app/com.riviu.agent-1/base.apk& exit /b 0)\r\n",
                "if \"%c%\"==\"sha256sum '/data/app/com.riviu.agent-1/base.apk'\" (echo {hash}  base.apk& exit /b 0)\r\n",
                "if \"%c%\"==\"cat /proc/sys/kernel/random/boot_id\" (echo fixture-boot& exit /b 0)\r\n",
                "if \"%c%\"==\"pidof com.riviu.agent\" if exist \"%d%pid.txt\" (type \"%d%pid.txt\"& exit /b 0) else (exit /b 1)\r\n",
                "if \"%c%\"==\"settings get secure default_input_method\" (type \"%d%ime.txt\"& exit /b 0)\r\n",
                "if \"%c%\"==\"settings get secure enabled_input_methods\" (type \"%d%enabled.txt\"& exit /b 0)\r\n",
                "if \"%c:~0,8%\"==\"ime set \" (>\"%d%ime.txt\" echo %c:~8%& exit /b 0)\r\n",
                "if \"%c%\"==\"ime disable com.riviu.agent/.RiviuIme\" (>\"%d%enabled.txt\" echo {keyboard}& exit /b 0)\r\n",
                ">>\"%d%unexpected.txt\" echo %c%\r\n",
                "exit /b 97\r\n"), hash = apk_sha256, keyboard = FIXTURE_KEYBOARD)).unwrap();
            path
        };
        #[cfg(not(windows))]
        let path = {
            use std::os::unix::fs::PermissionsExt;
            let path = root.join("maintenance-adb");
            std::fs::write(&path, format!(concat!(
                "#!/bin/sh\n",
                "d=$(dirname \"$0\")\n",
                "case \"$4\" in\n",
                "'id -u') echo 2000 ;;\n",
                "'am get-current-user') echo 0 ;;\n",
                "'dumpsys package com.riviu.agent') echo userId=10123; echo versionCode=9 ;;\n",
                "'pm path com.riviu.agent') echo package:/data/app/com.riviu.agent-1/base.apk ;;\n",
                "\"sha256sum '/data/app/com.riviu.agent-1/base.apk'\") echo '{hash}  base.apk' ;;\n",
                "'cat /proc/sys/kernel/random/boot_id') echo fixture-boot ;;\n",
                "'pidof com.riviu.agent') [ -f \"$d/pid.txt\" ] || exit 1; cat \"$d/pid.txt\" ;;\n",
                "'settings get secure default_input_method') cat \"$d/ime.txt\" ;;\n",
                "'settings get secure enabled_input_methods') cat \"$d/enabled.txt\" ;;\n",
                "'ime set '*) echo \"${{4#ime set }}\" > \"$d/ime.txt\" ;;\n",
                "'ime disable com.riviu.agent/.RiviuIme') echo '{keyboard}' > \"$d/enabled.txt\" ;;\n",
                "*) echo \"$4\" >> \"$d/unexpected.txt\"; exit 97 ;;\n",
                "esac\n"), hash = apk_sha256, keyboard = FIXTURE_KEYBOARD)).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
            path
        };
        AdbProgram::at(path)
    }

    /// 13/10/2026, phones 32 (SM-G955N) and 38 (SM-G955F): a clipboard job the restarted helper
    /// no longer knew left the Riviu IME selected. Maintenance refused (`ime_unproved`) although
    /// the checkpoint recorded the keyboard from before the switch; it must hand that one back.
    #[tokio::test]
    async fn maintenance_hands_back_the_recorded_keyboard_left_behind_the_helper_ime() {
        let serial = "fixture-recorded-ime";
        let root = std::env::temp_dir().join(format!("helper-recorded-ime-{}", uuid::Uuid::new_v4()));
        let state = state_path(&root, serial).unwrap();
        std::fs::create_dir_all(&state).unwrap();
        let apk = root.join("helper.apk");
        std::fs::write(&apk, b"fixture-apk-not-for-device").unwrap();
        let adb = maintenance_adb(&root, &riviu_core::frame_sha256(b"fixture-apk-not-for-device"));
        std::fs::write(root.join("ime.txt"), format!("{IME_ID}\n")).unwrap();
        std::fs::write(root.join("enabled.txt"), format!("{FIXTURE_KEYBOARD}:{IME_ID}\n")).unwrap();
        let owner_id = "fixture_owner_0123456789";
        std::fs::write(state.join("runtime-owner.json"), serde_json::to_vec(&json!({
            "serial":serial,"androidUser":0,"state":"claimPending","ownerId":owner_id,
            "apkSha256":riviu_core::frame_sha256(b"fixture-apk-not-for-device"),
            "serviceInstance":"fixture_instance_012345","ownerGeneration":"fixture_generation_012345"})).unwrap()).unwrap();
        let checkpoint = |pending: Value| serde_json::to_vec(&json!({"serial":serial,"androidUser":0,
            "ownerId":owner_id,"serviceInstance":"fixture_instance_012345","generation":"fixture_generation_012345",
            "pending":pending,"baseline":null})).unwrap();
        let prepare = || HelperClient::prepare_maintenance(&adb, serial, Some(&apk), Some(&root),
            "fixture_maintenance_1234", json!({"operation":"fixture"}));
        // Older data without the record: still refused, nothing guessed or changed.
        std::fs::write(state.join("ime-checkpoint.json"), checkpoint(json!({"phase":"submissionPending"}))).unwrap();
        let refused = prepare().await.expect_err("no recorded keyboard must stay unproved");
        assert_eq!(format!("{refused:#}"), "maintenance cannot guess or restore a lost boot IME");
        std::fs::write(state.join("ime-checkpoint.json"), checkpoint(json!({"previousIme":FIXTURE_KEYBOARD,
            "helperWasEnabled":false,"phase":"submissionPending","serviceInstance":"fixture_instance_012345",
            "operationId":"fixture-operation"}))).unwrap();
        let plan = prepare().await.expect("the recorded keyboard proves the boot IME");
        assert_eq!(plan["bootIme"], FIXTURE_KEYBOARD);
        assert_eq!(plan["imeRestore"], json!({"previousIme":FIXTURE_KEYBOARD,"helperWasEnabled":false}));
        let receipt = HelperClient::execute_maintenance(&adb, serial, Some(&apk), Some(&root), plan, true, false).await
            .expect("recorded keyboard restore and supersession");
        let ime = std::fs::read_to_string(root.join("ime.txt")).unwrap();
        let enabled = std::fs::read_to_string(root.join("enabled.txt")).unwrap();
        let unexpected = root.join("unexpected.txt").exists();
        let archived = state.join("maintenance/fixture_maintenance_1234/ime-checkpoint.json").exists();
        let retained = state.join("ime-checkpoint.json").exists();
        std::fs::remove_dir_all(&root).unwrap();
        assert_eq!(receipt["state"], "superseded");
        assert_eq!(receipt["bootImeRestored"], true);
        assert_eq!(receipt["oldObligations"], "archivedUnresolved");
        assert_eq!(ime.trim(), FIXTURE_KEYBOARD);
        assert!(!enabled.contains(IME_ID), "{enabled}");
        assert!(archived && !retained, "the old clipboard obligation is archived, not settled");
        assert!(!unexpected, "no command outside the fixture contract");
    }

    /// The cached owner of a clipboard job keeps maintenance fenced until its helper process
    /// is proven gone; then the job can never settle and maintenance may archive it.
    #[tokio::test]
    async fn cached_cleanup_debt_opens_maintenance_only_when_the_helper_process_is_gone() {
        let serial = "fixture-cached-debt";
        let root = std::env::temp_dir().join(format!("helper-cached-debt-{}", uuid::Uuid::new_v4()));
        let state = state_path(&root, serial).unwrap();
        std::fs::create_dir_all(&state).unwrap();
        let adb = maintenance_adb(&root, &"0".repeat(64));
        std::fs::write(root.join("pid.txt"), "4242\n").unwrap();
        let owner = CanaryOwner { owner_id: "fixture_owner_0123456789".into(), instance: "fixture_instance_012345".into(),
            generation: "fixture_generation_012345".into(), socket: String::new(), nonce: "fixture_nonce".into(),
            uid: 10123, apk_path: String::new(), report: state.clone() };
        let mut client = HelperClient::at(adb, serial, 1).unwrap();
        client.canary = Some(owner);
        client.production_runtime = true;
        client.retained_failure.store(true, std::sync::atomic::Ordering::Release);
        *client.pending_clipboard.lock() = Some(json!({"previousIme":FIXTURE_KEYBOARD,"helperWasEnabled":true,"phase":"submissionPending"}));
        let mut outcomes = Vec::new();
        for helper_pid in ["4242", "4141"] {
            std::fs::write(state.join("runtime-owner.json"), serde_json::to_vec(&json!({
                "serial":serial,"androidUser":0,"state":"claimPending","ownerId":"fixture_owner_0123456789",
                "serviceInstance":"fixture_instance_012345","ownerGeneration":"fixture_generation_012345",
                "bootId":"fixture-boot","helperPid":helper_pid})).unwrap()).unwrap();
            outcomes.push(client.lock_cached_maintenance(Some(&root)).await.map(drop).map_err(|error| format!("{error:#}")));
        }
        std::fs::remove_dir_all(&root).unwrap();
        assert_eq!(outcomes[0], Err("helper clipboard/baseline cleanup unresolved".into()), "same helper process keeps the fence");
        assert_eq!(outcomes[1], Ok(()), "a restarted helper process can never settle the old job");
    }

    #[test]
    fn pending_cleanup_admission_rejects_foreign_and_oversized_records() {
        let root = std::env::temp_dir().join(format!("helper-admission-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        assert!(pending_checkpoint(&root, "fixture").unwrap().is_none());
        let path = root.join("ime-checkpoint.json");
        std::fs::write(&path, serde_json::to_vec(&json!({"serial":"foreign","androidUser":0,"pending":{}})).unwrap()).unwrap();
        assert!(pending_checkpoint(&root, "fixture").is_err());
        std::fs::write(&path, vec![b' '; CHECKPOINT_LIMIT as usize + 1]).unwrap();
        assert!(pending_checkpoint(&root, "fixture").is_err());
        std::fs::write(&path, serde_json::to_vec(&json!({"serial":"fixture","androidUser":0,"pending":{"phase":"switching"}})).unwrap()).unwrap();
        assert!(pending_checkpoint(&root, "fixture").unwrap().is_some());
        assert!(state_path(Path::new("relative"), "fixture").is_err());
        assert!(state_path(&root, "../../serial").unwrap().starts_with(&root));
        std::fs::remove_dir_all(root).unwrap();
    }
}
