//! Release-only Binder carrier and durable, nonsecret cleanup admission.
use super::*;
use std::io::{Read, Write};

const CERT_SHA256: &str = "0ecb2f06620b2d0f2fcc2a71cede204819a20ddcc1210573bc8ab3f4b48ff813";
const MIN_VERSION: u64 = 7;
const CHECKPOINT_LIMIT: u64 = 16 * 1024;

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

pub(super) async fn release(adb: &AdbProgram, serial: &str, owner: &CanaryOwner) -> anyhow::Result<()> {
    let reply = exchange(adb, serial, owner, json!({"action":"release","nonce":owner.nonce,"ownerId":owner.owner_id,"token":helper_token(serial),"serviceInstance":owner.instance,"ownerGeneration":owner.generation})).await?;
    anyhow::ensure!(reply["ok"] == true && reply["state"] == "released" && reply["nonce"].as_str() == Some(owner.nonce.as_str()) && reply["ownerId"].as_str() == Some(owner.owner_id.as_str()) && reply["serviceInstance"].as_str() == Some(owner.instance.as_str()) && reply["ownerGeneration"].as_str() == Some(owner.generation.as_str()), "exact helper owner release unresolved");
    match std::fs::remove_file(owner.report.join("runtime-owner.json")) {
        Ok(()) => {},
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {},
        Err(error) => return Err(error.into()),
    }
    Ok(())
}

impl HelperClient {
    /// Called under the driver's inventory writer and the caller's control lease.
    pub(crate) async fn ensure_runtime(adb: AdbProgram, serial: &str, apk: Option<&Path>, state_dir: Option<&Path>) -> anyhow::Result<Self> {
        let state = state_path(state_dir.context("helper durable state directory missing")?, serial)?;
        // Recovery precedes any installation, service restart or owner claim.
        if let Some(checkpoint) = pending_checkpoint(&state, serial).map_err(|_| anyhow!(HelperRecoveryRequired))? {
            Self::recover_runtime_checkpoint(adb.clone(), serial, &state, checkpoint).await.map_err(|_| anyhow!(HelperRecoveryRequired))?;
        }
        // A prior failed claim/attach is never replaced by a new claim. Its credential
        // may have died with the host process; explicit owner reconciliation is required.
        if state.join("runtime-owner.json").try_exists()? {
            return Self::resume_runtime_owner(adb, serial, apk, &state).await;
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
        let mut intent = std::fs::OpenOptions::new().write(true).create_new(true).open(owner.report.join("runtime-owner.json"))?;
        intent.write_all(&serde_json::to_vec(&json!({"serial":serial,"androidUser":0,"ownerId":owner.owner_id,"nonce":owner.nonce,"apkSha256":hash,"state":"claimPending","tokenPersisted":false}))?)?;
        intent.sync_all()?;
        // Tokenless foreground startup keeps the owned service alive after the bridge unbinds.
        let started = adb.shell_output(serial, &format!("am start-foreground-service -n {SERVICE}"), Duration::from_secs(15)).await?;
        anyhow::ensure!(started.exit_code == 0 && !started.stdout.contains("Error") && !started.stderr.contains("Error"), "release helper foreground start refused");
        let body = json!({"action":"claim","nonce":owner.nonce,"ownerId":owner.owner_id,"token":helper_token(serial),"serviceInstance":"-","ownerGeneration":"-"});
        // Claim is sent exactly once. Any lost ACK uses this same credential and owner.
        let claimed = exchange(&adb, serial, &owner, body).await;
        // If a proved claim precedes a transport failure, release only that exact owner.
        if let Ok(identity) = &claimed {
            if identity["ok"] == true && identity["nonce"].as_str() == Some(owner.nonce.as_str()) && identity["ownerId"].as_str() == Some(owner.owner_id.as_str()) && identity["state"] == "ready" {
                owner.instance = identity["serviceInstance"].as_str().context("helper claim instance missing")?.into();
                owner.generation = identity["ownerGeneration"].as_str().context("helper claim generation missing")?.into();
            }
        }
        let port = match forward_helper(&adb, serial).await {
            Ok(port) => port,
            Err(error) => {
                if owner.instance != "-" { release(&adb, serial, &owner).await?; }
                return Err(error);
            }
        };
        let mut client = match Self::at(adb.clone(), serial, port) {
            Ok(client) => client,
            Err(error) => {
                if owner.instance != "-" { release(&adb, serial, &owner).await?; }
                frames::remove_forward(&adb, serial, port).await?;
                return Err(error);
            }
        };
        client.production_runtime = true;
        let attach = async {
            let identity = match claimed {
                Ok(value) => value,
                Err(bootstrap_error) => {
                    match client.post_json("/v1/session/status", json!({"nonce":owner.nonce})).await {
                        Ok(value) => value,
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
            anyhow::ensure!(identity["ok"] == true && identity["nonce"].as_str() == Some(owner.nonce.as_str()) && identity["ownerId"].as_str() == Some(owner.owner_id.as_str()) && (identity["state"] == "ready" || identity["ownership"] == "owned"), "helper claim owner not proved");
            owner.instance = identity["serviceInstance"].as_str().filter(|s| !s.is_empty() && *s != "-").context("helper instance missing")?.into();
            owner.generation = identity["ownerGeneration"].as_str().filter(|s| !s.is_empty() && *s != "-").context("helper generation missing")?.into();
            client.canary = Some(owner.clone());
            await_helper_ready(|| async { client.require_canary_identity().await?; let status = client.require_status().await?; anyhow::ensure!(status.features.iter().any(|f| f == "secureBootstrapBinderShell") && REQUIRED_FEATURES.iter().all(|required| status.features.iter().any(|f| f == required)), "release helper features missing"); Ok(()) }).await?;
            client.qualify_clipboard_roundtrip().await?;
            Ok::<(), anyhow::Error>(())
        }.await;
        if let Err(error) = attach {
            if client.cleanup_is_pending() {
                return Err(anyhow!(HelperRecoveryRequired)).context("helper qualification cleanup pending; owner and forward retained");
            }
            if client.canary.is_some() { release(&adb, serial, &owner).await.context("failed attach exact-owner release unresolved")?; }
            frames::remove_forward(&adb, serial, port).await?;
            return Err(error);
        }
        Ok(client)
    }

    async fn resume_runtime_owner(adb: AdbProgram, serial: &str, apk: Option<&Path>, state: &Path) -> anyhow::Result<Self> {
        let mut bytes = Vec::new();
        std::fs::File::open(state.join("runtime-owner.json"))?.take(CHECKPOINT_LIMIT + 1).read_to_end(&mut bytes)?;
        anyhow::ensure!(bytes.len() as u64 <= CHECKPOINT_LIMIT, HelperRecoveryRequired);
        let record: Value = serde_json::from_slice(&bytes)?;
        anyhow::ensure!(record["serial"].as_str() == Some(serial) && record["androidUser"] == 0, HelperRecoveryRequired);
        anyhow::ensure!(checked_shell(&adb, serial, "id -u").await?.trim() == "2000" && checked_shell(&adb, serial, "am get-current-user").await?.trim() == "0", HelperRecoveryRequired);
        let hash = riviu_core::frame_sha256(&std::fs::read(apk.context("bundled helper unavailable for resume")?)?);
        let (version, uid, apk_path) = inventory(&adb, serial).await?;
        anyhow::ensure!(version >= MIN_VERSION && record["apkSha256"].as_str() == Some(hash.as_str()) && matches_bundle(&adb, serial, &apk_path, &hash).await?, HelperRecoveryRequired);
        let owner_id = record["ownerId"].as_str().context("pending claim owner missing")?.to_owned();
        let nonce = uuid::Uuid::new_v4().simple().to_string();
        let port = forward_helper(&adb, serial).await?;
        let mut client = Self::at(adb.clone(), serial, port)?;
        client.production_runtime = true;
        let proof = async {
            // The same in-memory credential only. No claim, restart, install or action replay.
            let value = client.post_json("/v1/session/status", json!({"nonce":nonce})).await?;
            anyhow::ensure!(value["ok"] == true && value["nonce"].as_str() == Some(nonce.as_str()) && value["ownerId"].as_str() == Some(owner_id.as_str()) && value["ownership"] == "owned", HelperRecoveryRequired);
            client.canary = Some(CanaryOwner { owner_id, nonce, instance: value["serviceInstance"].as_str().context("owner instance missing")?.into(), generation: value["ownerGeneration"].as_str().context("owner generation missing")?.into(), socket: String::new(), uid, apk_path, report: state.into() });
            client.require_canary_identity().await?;
            client.qualify_clipboard_roundtrip().await?;
            Ok::<(), anyhow::Error>(())
        }.await;
        if proof.is_err() {
            if !client.cleanup_is_pending() { frames::remove_forward(&adb, serial, port).await?; }
            return Err(anyhow!(HelperRecoveryRequired));
        }
        Ok(client)
    }

    /// Rebind a lost host forward without claiming, reinstalling or replaying any operation.
    pub(crate) async fn reconnect_runtime(&self) -> anyhow::Result<Option<Self>> {
        if !self.production_runtime { return Ok(None); }
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
    }

    async fn recover_runtime_checkpoint(adb: AdbProgram, serial: &str, state: &Path, checkpoint: Value) -> anyhow::Result<()> {
        let _serial = ime_lock(serial).lock_owned().await;
        anyhow::ensure!(checked_shell(&adb, serial, "id -u").await?.trim() == "2000", "pending cleanup shell identity changed");
        anyhow::ensure!(checked_shell(&adb, serial, "am get-current-user").await?.trim() == "0", "pending cleanup Android user changed");
        let (_, uid, apk_path) = inventory(&adb, serial).await?;
        let field = |name: &str| checkpoint[name].as_str().map(str::to_owned).context("pending cleanup owner metadata missing");
        let owner = CanaryOwner { owner_id: field("ownerId")?, instance: field("serviceInstance")?, generation: field("generation")?, nonce: uuid::Uuid::new_v4().simple().to_string(), socket: String::new(), uid, apk_path, report: state.into() };
        let port = forward_helper(&adb, serial).await?;
        let mut client = Self::at(adb.clone(), serial, port)?;
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
            release(&adb, serial, &owner).await?;
            std::fs::remove_file(state.join("ime-checkpoint.json"))?;
            Ok::<(), anyhow::Error>(())
        }.await;
        let cleanup = frames::remove_forward(&adb, serial, port).await;
        result?;
        cleanup
    }
}
