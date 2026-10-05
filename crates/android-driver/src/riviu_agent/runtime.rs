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
        anyhow::ensure!(!self.cleanup_is_pending(), "helper clipboard/baseline cleanup unresolved");
        let owner = self.canary.as_ref().context("cached maintenance owner missing")?;
        let state = state_path(root.context("helper state directory missing")?, &self.serial)?;
        anyhow::ensure!(owner.report == state, "cached helper state binding changed");
        if let Some(bytes) = journal_bytes(&state.join("ime-checkpoint.json"))? {
            let saved: Value = serde_json::from_slice(&bytes)?;
            anyhow::ensure!(saved["serial"] == self.serial && saved["androidUser"] == 0
                && saved["ownerId"] == owner.owner_id && saved["serviceInstance"] == owner.instance
                && saved["generation"] == owner.generation
                && saved.get("pending").is_some_and(Value::is_null)
                && saved.get("baseline").is_some_and(Value::is_null),
                "helper durable clipboard cleanup unresolved");
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
        anyhow::ensure!(checked_shell(adb, serial, "id -u").await?.trim() == "2000"
            && checked_shell(adb, serial, "am get-current-user").await?.trim() == "0", HelperRecoveryRequired);
        let (version, uid, installed_path) = inventory(adb, serial).await?;
        let bundled_hash = riviu_core::frame_sha256(&std::fs::read(apk.context("bundled helper missing")?)?);
        let installed_hash = checked_shell(adb, serial, &format!("sha256sum {}", crate::adb::quote_device_path(&installed_path))).await?
            .split_whitespace().next().context("installed helper hash missing")?.to_owned();
        // Only the recorded trusted package or the current trusted bundle may be superseded.
        anyhow::ensure!(installed_hash.len() == 64 && installed_hash.bytes().all(|b| b.is_ascii_hexdigit())
            && (record["apkSha256"].as_str() == Some(installed_hash.as_str()) || installed_hash == bundled_hash),
            "maintenance helper APK provenance changed");
        let boot_ime = current_ime(adb, serial).await?;
        validate_ime_id(&boot_ime)?;
        anyhow::ensure!(boot_ime != IME_ID, "maintenance cannot guess or restore a lost boot IME");
        let boot_id = checked_shell(adb, serial, "cat /proc/sys/kernel/random/boot_id").await?.trim().to_owned();
        anyhow::ensure!(!boot_id.is_empty() && boot_id.len() <= 128, "maintenance boot identity missing");
        let process = maintenance_process(adb, serial).await?;
        let mut journals = serde_json::Map::new();
        for name in ["runtime-owner.json".to_owned(), "ime-checkpoint.json".to_owned(), format!("release-{owner_id}.json")] {
            let bytes = journal_bytes(&state.join(&name))?;
            journals.insert(name, bytes.as_ref().map(|b| json!(riviu_core::frame_sha256(b))).unwrap_or(Value::Null));
        }
        Ok(json!({"schema":1,"maintenanceId":maintenance_id,"serial":serial,"androidUser":0,
            "bundledApkSha256":bundled_hash,"installedApkSha256":installed_hash,"installedApkPath":installed_path,
            "packageUid":uid,"packageVersion":version,"certificateSha256":CERT_SHA256,
            "oldOwnerId":owner_id,"oldServiceInstance":record["serviceInstance"],"oldOwnerGeneration":record["ownerGeneration"],
            "oldClaimNonce":record["nonce"],"journalHashes":journals,"bootIme":boot_ime,"bootId":boot_id,
            "observedProcess":process,"effectIntent":effect_intent,"disposition":"operatorAuthorizedSupersession",
            "exactReleaseProved":false,"clipboardRestorationProved":false}))
    }

    /// Explicit maintenance only; never called by automatic install/admission/resume.
    pub(crate) async fn execute_maintenance(adb: &AdbProgram, serial: &str, apk: Option<&Path>, root: Option<&Path>, plan: Value, operator_authorized: bool, observation_only: bool) -> anyhow::Result<Value> {
        Self::execute_maintenance_with_cache(adb, serial, apk, root, plan, operator_authorized, observation_only, None).await
    }

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
        // An active intent never grants replay authority, even if the earlier dispatch is unknown.
        let stop_dispatched = !reconciling && !observation_only && !plan["observedProcess"].as_str().context("process binding missing")?.is_empty();
        if stop_dispatched {
            checked_shell(adb, serial, "am force-stop --user 0 com.riviu.agent").await?;
        }
        let mut settled = Self::observe_maintenance(adb, serial, apk, root, id, plan["effectIntent"].clone()).await?;
        anyhow::ensure!(settled["observedProcess"] == "", "superseded helper process still present");
        settled["observedProcess"] = plan["observedProcess"].clone();
        anyhow::ensure!(settled == plan, "maintenance binding changed; settlement unresolved");
        let stop_disposition = if reconciling { "alreadyAbsentOnReconciliationPriorDispatchUnknown" }
            else if observation_only { "alreadyAbsentObservationOnlyPriorDispatchUnknown" }
            else if stop_dispatched { "dispatched" } else { "alreadyAbsent" };
        let receipt = json!({"state":"superseded","plan":plan,"exactReleaseProved":false,
            "clipboardRestorationProved":false,"oldObligations":"archivedUnresolved","packageProcessAbsent":true,
            "stopDispatchedThisAttempt":stop_dispatched,"stopDisposition":stop_disposition});
        durable_json(&receipt_path, &receipt)?;
        finish_maintenance(&state, &archive, &plan)?;
        Ok(receipt)
    }

    /// A durable release (including an unresolved intent) fences authenticated reconnect.
    pub(crate) async fn settle_released_runtime(&self) -> anyhow::Result<bool> {
        let result = async {
            if !self.production_runtime { return Ok(false); }
            let owner = self.canary.as_ref().context("runtime owner missing")?;
            if !release_path(owner).try_exists()? { return Ok(false); }
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
            anyhow::ensure!(checked_shell(&adb, serial, "id -u").await?.trim() == "2000"
                && checked_shell(&adb, serial, "am get-current-user").await?.trim() == "0",
                "helper cold readiness device user unproved");
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
        result.map_err(|error| anyhow!(HelperRecoveryRequired)
            .context(format!("helper cold readiness unproved; no claim ({error:#})")))
    }

    /// Called under the driver's inventory writer and the caller's control lease.
    pub(crate) async fn ensure_runtime(adb: AdbProgram, serial: &str, apk: Option<&Path>, state_dir: Option<&Path>) -> anyhow::Result<Self> {
        let state = state_path(state_dir.context("helper durable state directory missing")?, serial)?;
        anyhow::ensure!(!state.join("maintenance-active.json").try_exists().context(HelperRecoveryRequired)?, HelperRecoveryRequired);
        Self::settle_runtime_release(&adb, serial, &state).await?;
        // Recovery precedes any installation, service restart or owner claim.
        if let Some(checkpoint) = pending_checkpoint(&state, serial).map_err(|_| anyhow!(HelperRecoveryRequired))? {
            Self::recover_runtime_checkpoint(adb.clone(), serial, &state, checkpoint).await.map_err(|error| {
                anyhow!(HelperRecoveryRequired).context(format!("helper pending cleanup unresolved ({error:#})"))
            })?;
        }
        // A prior failed claim/attach is never replaced by a new claim. Its credential
        // may have died with the host process; explicit owner reconciliation is required.
        if state.join("runtime-owner.json").try_exists().context(HelperRecoveryRequired)? {
            return Self::resume_runtime_owner(adb, serial, apk, &state).await.map_err(|error| {
                anyhow!(HelperRecoveryRequired).context(format!("helper owner resume unresolved ({error:#})"))
            });
        }
        let apk = apk.context("compatible bundled helper APK missing")?;
        let hash = riviu_core::frame_sha256(&std::fs::read(apk)?);
        anyhow::ensure!(checked_shell(&adb, serial, "id -u").await?.trim() == "2000", "helper runtime requires shell UID2000; no root fallback");
        anyhow::ensure!(checked_shell(&adb, serial, "am get-current-user").await?.trim() == "0", "helper runtime requires checked Android user0");
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
            intent.write_all(&serde_json::to_vec(&json!({"serial":serial,"androidUser":0,"ownerId":owner.owner_id,"nonce":owner.nonce,"apkSha256":hash,"state":"claimPending","tokenPersisted":true,"credentialStore":OWNER_CREDENTIAL_STORE}))?)?;
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
        anyhow::ensure!(checked_shell(&adb, serial, "id -u").await?.trim() == "2000" && checked_shell(&adb, serial, "am get-current-user").await?.trim() == "0", HelperRecoveryRequired);
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
            client.qualify_clipboard_roundtrip().await?;
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
        anyhow::ensure!(checked_shell(&adb, serial, "id -u").await?.trim() == "2000", "pending cleanup shell identity changed");
        anyhow::ensure!(checked_shell(&adb, serial, "am get-current-user").await?.trim() == "0", "pending cleanup Android user changed");
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
            client.require_canary_identity().await?;
            let pending = &checkpoint["pending"];
            let previous = pending["previousIme"].as_str().context("pending cleanup previous IME missing")?;
            anyhow::ensure!(validate_ime_id(previous).is_ok() && previous != IME_ID, "pending cleanup previous IME invalid");
            let was_enabled = pending["helperWasEnabled"].as_bool().context("pending cleanup prior enablement missing")?;
            // Clipboard contents are never persisted. A snapshot obligation cannot be guessed.
            anyhow::ensure!(checkpoint["baseline"].is_null(), "clipboard baseline restoration still pending");
            if let Some(id) = pending["operationId"].as_str() {
                let status = client.post_json("/v1/clipboard/jobs/status", json!({"serviceInstance":owner.instance,"requestId":id})).await?;
                anyhow::ensure!(status["ok"] == true && status["serviceInstance"].as_str() == Some(owner.instance.as_str()) && status["operationId"].as_str() == Some(id) && matches!(status["state"].as_str(), Some("succeeded" | "failed" | "cancelled")), "pending clipboard job not terminal; no replay");
            } else { anyhow::ensure!(pending["phase"] == "switching", "pending cleanup job identity unavailable"); }
            client.require_canary_identity().await?;
            let current = current_ime(&adb, serial).await?;
            if current == IME_ID { set_ime(&adb, serial, previous).await?; }
            else { anyhow::ensure!(current == previous, "pending IME changed outside owner; preserved"); }
            anyhow::ensure!(current_ime(&adb, serial).await? == previous, "pending IME restore unverified");
            if !was_enabled {
                client.require_canary_identity().await?;
                adb.shell(serial, &format!("ime disable {IME_ID}")).await?;
                let enabled = adb.shell(serial, "settings get secure enabled_input_methods").await?;
                anyhow::ensure!(!enabled.trim().split(':').any(|entry| entry.split(';').next() == Some(IME_ID)), "pending IME enablement restore unverified");
            }
            std::fs::remove_file(state.join("ime-checkpoint.json"))?;
            release(&adb, serial, &owner, token.as_ref(), Some(port)).await?;
            Ok::<(), anyhow::Error>(())
        }.await;
        let cleanup = frames::remove_forward(&adb, serial, port).await;
        result?;
        cleanup?;
        finish_release(serial, &owner).await
    }
}
