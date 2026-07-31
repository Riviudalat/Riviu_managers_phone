use std::collections::{HashMap, HashSet};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Context;
use async_trait::async_trait;
use parking_lot::{Mutex, RwLock};
use riviu_core::{
    validate_clipboard_read_limit, ActiveAppIdentity, ActiveTransport, AgentInstallProof,
    AgentSettings, AgentState, AgentStatus, AppProcessState, ClipboardAccessMode, ConnectionKind,
    DeviceCapabilitySnapshot, DeviceDriver, DeviceInfo, DeviceStatus, FrameSource,
    GuardedClipboardOperation, GuardedClipboardOutput, GuardedClipboardProgress,
    GuardedClipboardTransition, InstalledAgentIdentity, InstalledTargetIdentity,
    InteractionSessionKind, ProcessAbsenceProof, QualifiedElementLocator, StreamHandoffProof,
    StreamStartProof, StreamStopProof, SwipeGesture, TapPoint, TileStreamState, UiCapabilities,
    UiError, UiErrorKind, UiSession, MAX_INTERACTION_CLIPBOARD_BYTES, STREAM_FPS,
};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use tokio::io::AsyncReadExt;
use tokio::process::{Child, Command};

use crate::config::{DriverConfig, DriverTarget};
use crate::interaction_runtime::{
    repair_install_only_locked, InstallOnlyInspection, InstallOnlyRuntime,
    InteractionLifecycleRegistry,
};
use crate::process_tree::background_command;
use crate::stream::StreamHub;
use crate::supervisor::{DeviceOwned, OwnedChild, ProcessRegistry, Role, SlotMap};
use crate::telemetry;
use crate::wda::{WdaBackend, WdaClient, WdaProfile};
use crate::{decide_install, AgentArtifact, AgentInstallDecision, InstalledAppMetadata};

/// First local port used for the USB tunnel to device WDA (8100).
const WDA_LOCAL_PORT_BASE: u16 = 18100;
/// Distinct WDA-control tunnels, one per device. Room for a large phone farm.
const WDA_LOCAL_PORT_SPAN: u16 = 64;
const SIDECAR_COMMAND_TIMEOUT: Duration = Duration::from_secs(180);
const PMD_SIDECAR_PROTOCOL_VERSION: u64 = 2;
const VERIFIED_PROCESS_CONTROL_CONTRACT: &str = "verifiedProcessControl";
const INTERACTION_DRIVER_ADAPTER_VERSION: &str = "interaction-v1";
const INTERACTION_TARGET_BUNDLE_ID: &str = "com.ss.iphone.ugc.Ame";
const RTMMO_TOKEN_ENV: &str = "RIVIU_RTMMO_TOKEN";

#[derive(Debug, Clone)]
struct SidecarProgram {
    executable: PathBuf,
    prefix_args: Vec<OsString>,
}

impl SidecarProgram {
    async fn resolve(sidecar_root: &Path) -> anyhow::Result<Self> {
        let bundled = sidecar_root
            .join("pymobiledevice3")
            .join("runtime")
            .join(bundled_sidecar_filename());
        if bundled.is_file() {
            return Ok(Self {
                executable: bundled,
                prefix_args: Vec::new(),
            });
        }

        let script = sidecar_root.join("pymobiledevice3").join("riviu_pmd.py");
        if !script.is_file() {
            anyhow::bail!(
                "missing bundled sidecar {} and development script {}",
                bundled.display(),
                script.display()
            );
        }
        Ok(Self {
            executable: find_python().await?,
            prefix_args: vec![script.into_os_string()],
        })
    }

    fn command(&self) -> Command {
        let mut command = background_command(&self.executable);
        command.args(&self.prefix_args);
        command
    }

    fn is_bundled(&self) -> bool {
        self.prefix_args.is_empty()
    }
}

#[cfg(windows)]
fn bundled_sidecar_filename() -> &'static str {
    "riviu-pmd.exe"
}

#[cfg(not(windows))]
fn bundled_sidecar_filename() -> &'static str {
    "riviu-pmd"
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StreamReadiness {
    BestEffort,
    NonEmptyFrame,
    DecodedFrame,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InteractionInspectionTransport<'a> {
    LegacyUsbmux,
    Rsd { host: &'a str, port: u16 },
}

impl InteractionInspectionTransport<'_> {
    fn active_transport(self) -> ActiveTransport {
        match self {
            Self::LegacyUsbmux => ActiveTransport::LegacyUsbmuxTransport,
            Self::Rsd { .. } => ActiveTransport::RsdTransport,
        }
    }
}

fn interaction_inspection_args(
    udid: &str,
    target_bundle_id: &str,
    agent_bundle_id: &str,
    transport: InteractionInspectionTransport<'_>,
) -> Vec<String> {
    let mut args = vec![
        "inspect-device-capabilities".into(),
        "--udid".into(),
        udid.into(),
        "--target-bundle-id".into(),
        target_bundle_id.into(),
        "--agent-bundle-id".into(),
        agent_bundle_id.into(),
    ];
    if let InteractionInspectionTransport::Rsd { host, port } = transport {
        args.extend([
            "--rsd-host".into(),
            host.into(),
            "--rsd-port".into(),
            port.to_string(),
        ]);
    }
    args
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct InteractionInspection {
    ok: bool,
    udid: String,
    product_type: String,
    ios_version: String,
    transport: ActiveTransport,
    target_app: Option<InteractionInspectedApp>,
    agent_app: Option<InteractionInspectedApp>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct InteractionInspectedApp {
    bundle_id: String,
    version: String,
    build: String,
    #[serde(default)]
    executable_name: Option<String>,
    #[serde(default)]
    signer_identity: Option<String>,
}

fn require_inspection_value<'a>(field: &str, value: &'a str) -> anyhow::Result<&'a str> {
    if value.trim().is_empty() {
        anyhow::bail!("interaction inspection field {field} must not be blank");
    }
    Ok(value)
}

fn parse_interaction_inspection(
    value: serde_json::Value,
    expected_udid: &str,
    target_bundle_id: &str,
    artifact: &AgentArtifact,
    expected_transport: ActiveTransport,
) -> anyhow::Result<DeviceCapabilitySnapshot> {
    let response: InteractionInspection =
        serde_json::from_value(value).context("invalid interaction inspection response")?;
    if !response.ok {
        anyhow::bail!("interaction inspection sidecar returned ok=false");
    }
    if response.udid != expected_udid {
        anyhow::bail!("interaction inspection returned a different UDID");
    }
    if response.transport != expected_transport {
        anyhow::bail!("interaction inspection returned a different transport");
    }
    require_inspection_value("productType", &response.product_type)?;
    require_inspection_value("iosVersion", &response.ios_version)?;

    let target = response
        .target_app
        .context("target app is not installed on the inspected device")?;
    if target.bundle_id != target_bundle_id {
        anyhow::bail!("interaction inspection returned a different target app");
    }
    require_inspection_value("targetApp.version", &target.version)?;
    require_inspection_value("targetApp.build", &target.build)?;

    let agent = response
        .agent_app
        .context("selected Agent is not installed on the inspected device")?;
    if agent.bundle_id != artifact.manifest.bundle_id {
        anyhow::bail!("interaction inspection returned a different Agent bundle");
    }
    require_inspection_value("agentApp.version", &agent.version)?;
    require_inspection_value("agentApp.build", &agent.build)?;
    let executable_name = agent
        .executable_name
        .as_deref()
        .context("installed Agent executable name is unavailable")?;
    require_inspection_value("agentApp.executableName", executable_name)?;
    let signer_identity = agent
        .signer_identity
        .as_deref()
        .context("installed Agent signer identity is unavailable")?;
    require_inspection_value("agentApp.signerIdentity", signer_identity)?;
    let signer_identity_sha256 = format!("{:x}", Sha256::digest(signer_identity.as_bytes()));

    Ok(DeviceCapabilitySnapshot {
        installed_agent: InstalledAgentIdentity {
            bundle_id: agent.bundle_id,
            version: agent.version,
            build: agent.build,
            executable_name: executable_name.to_string(),
            signer_identity_sha256,
        },
        selected_artifact_sha256: artifact.manifest.sha256.to_ascii_lowercase(),
        agent_version: artifact.manifest.artifact_version.clone(),
        protocol_version: artifact.manifest.protocol_version,
        driver_adapter_version: INTERACTION_DRIVER_ADAPTER_VERSION.to_string(),
        transport: response.transport,
        product_type: response.product_type,
        ios_version: response.ios_version,
        target_app: InstalledTargetIdentity {
            bundle_id: target.bundle_id,
            version: target.version,
            build: target.build,
        },
        // Metadata inspection never proves that a protected runtime route is
        // currently live. Install-only repair supplies that separate proof.
        protected_auth_ready: false,
        geometry: None,
    })
}

fn proxy_args(
    profile: &WdaProfile,
    udid: &str,
    local_port: u16,
    force_restart: bool,
) -> Vec<String> {
    let mut args = vec![
        "wda-proxy".to_string(),
        "--udid".into(),
        udid.to_string(),
        "--local-port".into(),
        local_port.to_string(),
        "--backend".into(),
        match profile.backend {
            WdaBackend::Stock => "stock",
            WdaBackend::RtMmo => "rt-mmo",
        }
        .into(),
        "--device-port".into(),
        profile.device_port.to_string(),
        "--mjpeg-port".into(),
        profile.mjpeg_port.to_string(),
        "--bundle-id".into(),
        profile.bundle_id.clone(),
    ];
    if force_restart {
        args.push("--restart-wda".into());
    }
    args
}

fn proxy_process_fingerprint(udid: &str) -> String {
    format!("wda-proxy --udid {udid}")
}

fn text_bootstrap_args(profile: &WdaProfile, udid: &str, local_port: u16) -> Vec<String> {
    let mut args = proxy_args(profile, udid, local_port, true);
    args.push("--bootstrap-only".into());
    args
}

fn stream_args(profile: &WdaProfile, udid: &str) -> Vec<String> {
    vec![
        "stream".to_string(),
        "--udid".into(),
        udid.to_string(),
        "--fps".into(),
        STREAM_FPS.to_string(),
        "--quality".into(),
        "55".into(),
        "--mode".into(),
        match profile.backend {
            WdaBackend::RtMmo => "mjpeg",
            WdaBackend::Stock => "auto",
        }
        .into(),
        "--wda-bundle".into(),
        profile.bundle_id.clone(),
        "--wda-port".into(),
        profile.device_port.to_string(),
        "--mjpeg-port".into(),
        profile.mjpeg_port.to_string(),
    ]
}

fn proxy_ready_window(profile: &WdaProfile, force_restart: bool) -> Duration {
    match (profile.backend, force_restart) {
        // Two bounded app-launch attempts can each spend 25 s launching and
        // 35 s waiting for the device port, plus cleanup between attempts.
        (WdaBackend::RtMmo, _) => Duration::from_secs(180),
        (WdaBackend::Stock, true) => Duration::from_secs(110),
        (WdaBackend::Stock, false) => Duration::from_secs(55),
    }
}

fn session_attach_required(stream_running: bool) -> bool {
    !stream_running
}

fn runtime_for_target(
    target: &DriverTarget,
) -> anyhow::Result<(WdaProfile, Option<AgentArtifact>, AgentSettings)> {
    match target {
        DriverTarget::Real(config) => Ok((
            WdaProfile::unified(config),
            Some(config.artifact.clone()),
            config.settings.clone(),
        )),
        DriverTarget::LegacyStock => Ok((WdaProfile::stock(), None, AgentSettings::default())),
        DriverTarget::Mock => anyhow::bail!("mock target does not use the pymobiledevice3 driver"),
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SidecarPingResponse {
    ok: bool,
    pymobiledevice3: bool,
    sidecar_protocol_version: u64,
    contracts: Vec<String>,
}

fn verified_process_control_from_ping(stdout: &[u8], command_succeeded: bool) -> bool {
    if !command_succeeded {
        return false;
    }
    let Ok(response) = serde_json::from_slice::<SidecarPingResponse>(stdout) else {
        return false;
    };
    response.ok
        && response.pymobiledevice3
        && response.sidecar_protocol_version == PMD_SIDECAR_PROTOCOL_VERSION
        && response
            .contracts
            .iter()
            .any(|contract| contract == VERIFIED_PROCESS_CONTROL_CONTRACT)
}

#[derive(Clone)]
pub struct PmdIosDriver {
    sidecar: Option<SidecarProgram>,
    verified_app_termination: bool,
    streams: StreamHub,
    wda_host: String,
    profile: WdaProfile,
    slots: SlotMap,
    registry: ProcessRegistry,
    /// Local control port per UDID. Assignments are sticky, so a device keeps
    /// its port across reconnects and can never collide with another device.
    ports: Arc<Mutex<HashMap<String, u16>>>,
    /// Cached WDA session per UDID.
    sessions: Arc<Mutex<HashMap<String, WdaClient>>>,
    negotiated_ui: Arc<Mutex<HashMap<String, UiCapabilities>>>,
    interaction_lifecycle: InteractionLifecycleRegistry,
    agent_statuses: Arc<Mutex<HashMap<String, AgentStatus>>>,
    agent_settings: Arc<RwLock<AgentSettings>>,
    artifact: Option<AgentArtifact>,
}

impl PmdIosDriver {
    pub async fn probe(config: &DriverConfig) -> anyhow::Result<Self> {
        let sidecar = SidecarProgram::resolve(&config.sidecar_root).await?;
        let output = sidecar
            .command()
            .arg("ping")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await?;
        if !output.status.success() && output.status.code() != Some(2) {
            anyhow::bail!(
                "pmd ping failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        let verified_app_termination =
            verified_process_control_from_ping(&output.stdout, output.status.success());
        tracing::info!(
            distribution = if sidecar.is_bundled() {
                "bundled"
            } else {
                "development-python"
            },
            executable = %sidecar.executable.display(),
            "resolved pymobiledevice3 sidecar"
        );
        let (profile, artifact, settings) = runtime_for_target(&config.target)?;
        let driver = Self::build(
            Some(sidecar),
            verified_app_termination,
            profile,
            artifact,
            settings,
            config.state_dir.clone(),
        );
        // A previous crash can leave a relay holding the port and an XCTest
        // runner holding the device. Reclaim ours before touching any device.
        for what in driver.registry.reclaim_orphans().await {
            tracing::warn!("reclaimed orphaned child process: {what}");
            telemetry::record_event("-", "reclaim_orphan", 0, &what);
        }
        Ok(driver)
    }

    pub fn degraded(config: &DriverConfig) -> anyhow::Result<Self> {
        let (profile, artifact, settings) = runtime_for_target(&config.target)?;
        Ok(Self::build(
            None,
            false,
            profile,
            artifact,
            settings,
            config.state_dir.clone(),
        ))
    }

    fn build(
        sidecar: Option<SidecarProgram>,
        verified_app_termination: bool,
        profile: WdaProfile,
        artifact: Option<AgentArtifact>,
        agent_settings: AgentSettings,
        state_dir: PathBuf,
    ) -> Self {
        Self {
            sidecar,
            verified_app_termination,
            streams: StreamHub::new(),
            wda_host: "127.0.0.1".into(),
            profile,
            slots: SlotMap::default(),
            registry: ProcessRegistry::new(state_dir),
            ports: Arc::new(Mutex::new(HashMap::new())),
            sessions: Arc::new(Mutex::new(HashMap::new())),
            negotiated_ui: Arc::new(Mutex::new(HashMap::new())),
            interaction_lifecycle: InteractionLifecycleRegistry::default(),
            agent_statuses: Arc::new(Mutex::new(HashMap::new())),
            agent_settings: Arc::new(RwLock::new(agent_settings)),
            artifact,
        }
    }

    fn sidecar_command(&self) -> anyhow::Result<Command> {
        self.sidecar
            .as_ref()
            .map(SidecarProgram::command)
            .context("pymobiledevice3 sidecar is not configured")
    }

    pub fn stream_hub(&self) -> StreamHub {
        self.streams.clone()
    }

    async fn inspect_device_for_target_using(
        &self,
        udid: &str,
        target_bundle_id: &str,
        transport: InteractionInspectionTransport<'_>,
    ) -> anyhow::Result<DeviceCapabilitySnapshot> {
        self.artifact()?
            .verify_checksum()
            .context("selected Agent artifact failed integrity verification")?;
        self.inspect_device_for_target_verified_using(udid, target_bundle_id, transport)
            .await
    }

    async fn inspect_device_for_target_verified_using(
        &self,
        udid: &str,
        target_bundle_id: &str,
        transport: InteractionInspectionTransport<'_>,
    ) -> anyhow::Result<DeviceCapabilitySnapshot> {
        let artifact = self.artifact()?;
        let args = interaction_inspection_args(
            udid,
            target_bundle_id,
            &artifact.manifest.bundle_id,
            transport,
        );
        let borrowed_args: Vec<&str> = args.iter().map(String::as_str).collect();
        let value = self.run_json(&borrowed_args).await?;
        parse_interaction_inspection(
            value,
            udid,
            target_bundle_id,
            artifact,
            transport.active_transport(),
        )
    }

    /// Inspect metadata through an already-established RSD endpoint.
    pub async fn inspect_interaction_device_over_rsd(
        &self,
        udid: &str,
        host: &str,
        port: u16,
    ) -> anyhow::Result<DeviceCapabilitySnapshot> {
        if host.trim().is_empty() || port == 0 {
            anyhow::bail!("RSD inspection endpoint must include a host and non-zero port");
        }
        self.inspect_device_for_target_using(
            udid,
            INTERACTION_TARGET_BUNDLE_ID,
            InteractionInspectionTransport::Rsd { host, port },
        )
        .await
    }

    async fn run_json(&self, args: &[&str]) -> anyhow::Result<serde_json::Value> {
        if self.sidecar.is_none() {
            anyhow::bail!(
                "pymobiledevice3 sidecar not configured — install python3 + pymobiledevice3"
            );
        }
        let mut command = self.sidecar_command()?;
        command
            .args(args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let output = tokio::time::timeout(SIDECAR_COMMAND_TIMEOUT, command.output())
            .await
            .map_err(|_| {
                anyhow::anyhow!(
                    "pmd {} timed out after {}s",
                    args.first().unwrap_or(&""),
                    SIDECAR_COMMAND_TIMEOUT.as_secs()
                )
            })??;
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        if !output.status.success() {
            // Prefer the structured JSON error on stdout over urllib3 noise.
            let detail = serde_json::from_str::<serde_json::Value>(stdout.trim())
                .ok()
                .and_then(|v| v.get("error").and_then(|e| e.as_str()).map(String::from))
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| {
                    let s = stderr.trim();
                    if s.is_empty() {
                        stdout.trim().to_string()
                    } else {
                        s.lines()
                            .rev()
                            .find(|l| {
                                !l.contains("NotOpenSSLWarning") && !l.contains("warnings.warn")
                            })
                            .unwrap_or(s)
                            .to_string()
                    }
                });
            anyhow::bail!("pmd {}: {}", args.first().unwrap_or(&""), detail);
        }
        Ok(serde_json::from_str(stdout.trim()).unwrap_or(serde_json::json!({ "ok": true })))
    }

    /// The sticky control port for a device.
    fn port_for(&self, udid: &str) -> u16 {
        let mut ports = self.ports.lock();
        if let Some(p) = ports.get(udid) {
            return *p;
        }
        let taken: HashSet<u16> = ports.values().copied().collect();
        let port = (0..WDA_LOCAL_PORT_SPAN)
            .map(|i| WDA_LOCAL_PORT_BASE + i)
            .find(|p| !taken.contains(p))
            .unwrap_or(WDA_LOCAL_PORT_BASE);
        ports.insert(udid.to_string(), port);
        port
    }

    async fn wda_http_reachable(&self, port: u16) -> bool {
        WdaClient::new_with_profile(&self.wda_host, port, "-", self.profile.clone())
            .health_quick()
            .await
    }

    fn artifact(&self) -> anyhow::Result<&AgentArtifact> {
        self.artifact
            .as_ref()
            .context("unified agent artifact is unavailable for the selected driver target")
    }

    fn profile_for(&self, udid: &str) -> anyhow::Result<WdaProfile> {
        let capabilities = self
            .negotiated_ui
            .lock()
            .get(udid)
            .cloned()
            .unwrap_or_default();
        self.profile
            .clone()
            .with_interaction_capabilities(capabilities)
            .map_err(anyhow::Error::new)
    }

    fn status(
        &self,
        udid: &str,
        state: AgentState,
        installed: Option<&InstalledAppMetadata>,
        readiness: (bool, bool, bool),
        message: Option<String>,
    ) -> AgentStatus {
        let (artifact_id, artifact_version, bundle_id, protocol_version, features) = self
            .artifact
            .as_ref()
            .map(|artifact| {
                (
                    artifact.manifest.artifact_id.clone(),
                    artifact.manifest.artifact_version.clone(),
                    artifact.manifest.bundle_id.clone(),
                    artifact.manifest.protocol_version,
                    artifact.manifest.features.clone(),
                )
            })
            .unwrap_or_else(|| {
                (
                    "legacy-stock-wda".to_string(),
                    String::new(),
                    self.profile.bundle_id.clone(),
                    0,
                    self.profile.features.clone(),
                )
            });
        let (auth_ready, mjpeg_ready, session_ready) = readiness;
        AgentStatus {
            udid: udid.to_string(),
            state,
            artifact_id,
            artifact_version,
            bundle_id,
            protocol_version,
            features,
            installed_version: installed.and_then(|info| info.version.clone()),
            installed_build: installed.and_then(|info| info.build.clone()),
            auth_ready,
            mjpeg_ready,
            session_ready,
            message,
        }
    }

    fn publish_status(&self, status: AgentStatus) -> AgentStatus {
        self.agent_statuses
            .lock()
            .insert(status.udid.clone(), status.clone());
        status
    }

    fn publish_interaction_readiness(
        &self,
        udid: &str,
        state: AgentState,
        readiness: (bool, bool, bool),
        message: Option<String>,
    ) {
        let mut status = self.cached_agent_status(udid);
        if status.artifact_id.is_empty() {
            status = self.status(udid, state.clone(), None, readiness, message.clone());
        }
        let (auth_ready, mjpeg_ready, session_ready) = readiness;
        status.state = state;
        status.auth_ready = auth_ready;
        status.mjpeg_ready = mjpeg_ready;
        status.session_ready = session_ready;
        status.message = message;
        self.publish_status(status);
    }

    fn secret_free_error(&self, error: &anyhow::Error) -> String {
        let mut message = format!("{error:#}");
        if let Some(token) = self.profile.auth_token.as_ref() {
            message = message.replace(token.expose(), "[REDACTED]");
        }
        message
    }

    fn secret_free_result_detail<T>(&self, result: &anyhow::Result<T>) -> String {
        match result {
            Ok(_) => "ok".to_string(),
            Err(error) => self.secret_free_error(error),
        }
    }

    async fn inspect_agent_locked(
        &self,
        udid: &str,
    ) -> anyhow::Result<Option<InstalledAppMetadata>> {
        let artifact = self.artifact()?;
        let value = self
            .run_json(&[
                "is-installed",
                "--udid",
                udid,
                "--bundle-id",
                &artifact.manifest.bundle_id,
            ])
            .await?;
        if value.get("installed").and_then(|item| item.as_bool()) != Some(true) {
            return Ok(None);
        }
        Ok(Some(InstalledAppMetadata {
            bundle_id: value
                .get("bundleId")
                .and_then(|item| item.as_str())
                .unwrap_or(&artifact.manifest.bundle_id)
                .to_string(),
            version: value
                .get("version")
                .and_then(|item| item.as_str())
                .map(str::to_string),
            build: value
                .get("build")
                .and_then(|item| item.as_str())
                .map(str::to_string),
            application_type: value
                .get("applicationType")
                .and_then(|item| item.as_str())
                .map(str::to_string),
            path: value
                .get("path")
                .and_then(|item| item.as_str())
                .map(str::to_string),
            signer_identity: value
                .get("signerIdentity")
                .and_then(|item| item.as_str())
                .map(str::to_string),
        }))
    }

    async fn install_bundled_agent_locked(&self, udid: &str) -> anyhow::Result<()> {
        let artifact = self.artifact()?.clone();
        artifact.verify_checksum()?;
        let ipa = artifact.ipa_path.to_string_lossy().into_owned();
        self.run_json(&["install", "--udid", udid, "--ipa", &ipa])
            .await?;
        Ok(())
    }

    async fn prove_agent_ready_locked(
        &self,
        udid: &str,
        installed: Option<&InstalledAppMetadata>,
        owned: &mut DeviceOwned,
    ) -> anyhow::Result<AgentStatus> {
        let port = self
            .ensure_relay_locked(udid, owned)
            .await
            .context("protected agent launch/auth failed")?;
        if !self.wda_http_reachable(port).await {
            anyhow::bail!("protected agent auth probe failed");
        }
        self.publish_status(self.status(
            udid,
            AgentState::Starting,
            installed,
            (true, false, false),
            None,
        ));

        self.session_locked(udid, owned)
            .await
            .context("agent session readiness failed")?;
        self.publish_status(self.status(
            udid,
            AgentState::Starting,
            installed,
            (true, false, true),
            None,
        ));

        self.ensure_stream_locked(udid, owned)
            .await
            .context("agent MJPEG readiness failed")?;
        Ok(self.publish_status(self.status(
            udid,
            AgentState::Ready,
            installed,
            (true, true, true),
            None,
        )))
    }

    async fn preflight_agent_locked(
        &self,
        udid: &str,
        owned: &mut DeviceOwned,
    ) -> anyhow::Result<AgentStatus> {
        if self.profile.backend != WdaBackend::RtMmo {
            return Ok(self.publish_status(self.status(
                udid,
                AgentState::RepairRequired,
                None,
                (false, false, false),
                Some("Legacy stock WDA does not provide trusted text comments".to_string()),
            )));
        }

        if let Err(error) = self.artifact()?.verify_checksum() {
            let message = self.secret_free_error(&error);
            self.publish_status(self.status(
                udid,
                AgentState::Error,
                None,
                (false, false, false),
                Some(message),
            ));
            return Err(error);
        }
        let installed = match self.inspect_agent_locked(udid).await {
            Ok(installed) => installed,
            Err(error) => {
                let message = self.secret_free_error(&error);
                self.publish_status(self.status(
                    udid,
                    AgentState::Error,
                    None,
                    (false, false, false),
                    Some(message),
                ));
                return Err(error);
            }
        };
        let auto_repair = self.agent_settings.read().auto_repair;
        let decision = decide_install(&self.artifact()?.manifest, installed.as_ref(), auto_repair);
        match decision {
            AgentInstallDecision::InstallMissing | AgentInstallDecision::RepairVersionMismatch => {
                self.repair_agent_locked(udid, owned).await
            }
            AgentInstallDecision::ReportRepairRequired => Ok(self.publish_status(self.status(
                udid,
                if installed.is_some() {
                    AgentState::RepairRequired
                } else {
                    AgentState::Missing
                },
                installed.as_ref(),
                (false, false, false),
                Some("Riviu Agent requires repair".to_string()),
            ))),
            AgentInstallDecision::Reuse => {
                self.publish_status(self.status(
                    udid,
                    AgentState::Starting,
                    installed.as_ref(),
                    (false, false, false),
                    None,
                ));
                match self
                    .prove_agent_ready_locked(udid, installed.as_ref(), owned)
                    .await
                {
                    Ok(status) => Ok(status),
                    Err(error) => {
                        self.teardown_stream_locked(udid, owned).await;
                        self.sessions.lock().remove(udid);
                        self.teardown_proxy_locked(owned).await;
                        let message = self.secret_free_error(&error);
                        Ok(self.publish_status(self.status(
                            udid,
                            AgentState::Error,
                            installed.as_ref(),
                            (false, false, false),
                            Some(message),
                        )))
                    }
                }
            }
        }
    }

    async fn repair_agent_locked(
        &self,
        udid: &str,
        owned: &mut DeviceOwned,
    ) -> anyhow::Result<AgentStatus> {
        self.negotiated_ui.lock().remove(udid);
        self.publish_status(self.status(
            udid,
            AgentState::Starting,
            None,
            (false, false, false),
            Some("Repairing Riviu Agent".to_string()),
        ));

        let result: anyhow::Result<AgentStatus> = async {
            self.teardown_stream_locked(udid, owned).await;
            self.sessions.lock().remove(udid);
            self.teardown_proxy_locked(owned).await;

            let installed = self.inspect_agent_locked(udid).await?;
            if installed.is_some() {
                let bundle_id = self.artifact()?.manifest.bundle_id.clone();
                self.run_json(&["uninstall", "--udid", udid, "--bundle-id", &bundle_id])
                    .await?;
            }
            self.install_bundled_agent_locked(udid).await?;
            owned.force_restart = false;

            let provisional = self.inspect_agent_locked(udid).await?;
            if decide_install(&self.artifact()?.manifest, provisional.as_ref(), false)
                != AgentInstallDecision::Reuse
            {
                anyhow::bail!("installed Riviu Agent metadata does not match bundled artifact");
            }
            self.prove_agent_ready_locked(udid, provisional.as_ref(), owned)
                .await?;
            let installed = self.inspect_agent_locked(udid).await?;
            if decide_install(&self.artifact()?.manifest, installed.as_ref(), false)
                != AgentInstallDecision::Reuse
            {
                anyhow::bail!("installed Riviu Agent metadata does not match bundled artifact");
            }
            Ok(self.publish_status(self.status(
                udid,
                AgentState::Ready,
                installed.as_ref(),
                (true, true, true),
                None,
            )))
        }
        .await;

        match result {
            Ok(status) => Ok(status),
            Err(error) => {
                self.teardown_stream_locked(udid, owned).await;
                self.sessions.lock().remove(udid);
                self.teardown_proxy_locked(owned).await;
                let message = self.secret_free_error(&error);
                self.publish_status(self.status(
                    udid,
                    AgentState::Error,
                    None,
                    (false, false, false),
                    Some(message),
                ));
                Err(error)
            }
        }
    }

    /// Restart only the device-side RT-MMO agent. The existing USB relay is
    /// deliberately left alone; this refreshes the trusted text channel
    /// without classifying a healthy transport as wedged.
    async fn bootstrap_rt_text_agent_locked(
        &self,
        udid: &str,
        local_port: u16,
    ) -> anyhow::Result<()> {
        let args = text_bootstrap_args(&self.profile, udid, local_port);
        let mut command = self.sidecar_command()?;
        command
            .args(&args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        if let Some(token) = self.profile.auth_token.as_ref() {
            command.env(RTMMO_TOKEN_ENV, token.expose());
        }

        let started = Instant::now();
        let output =
            tokio::time::timeout(proxy_ready_window(&self.profile, true), command.output())
                .await
                .map_err(|_| anyhow::anyhow!("timeout chờ RT-MMO text bootstrap"))??;
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        let response = stdout
            .lines()
            .rev()
            .find_map(|line| serde_json::from_str::<serde_json::Value>(line.trim()).ok());
        let ok = output.status.success()
            && response
                .as_ref()
                .and_then(|value| value.get("ok"))
                .and_then(|value| value.as_bool())
                == Some(true);
        telemetry::record_event(
            udid,
            "text_agent_restart",
            started.elapsed().as_millis() as u32,
            if ok { "ok" } else { "failed" },
        );
        if !ok {
            let detail = response
                .as_ref()
                .and_then(|value| value.get("error"))
                .and_then(|value| value.as_str())
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| {
                    let stderr = stderr.trim();
                    if stderr.is_empty() {
                        stdout.trim()
                    } else {
                        stderr
                    }
                });
            anyhow::bail!("RT-MMO text bootstrap failed: {detail}");
        }
        Ok(())
    }

    /// Ensure the control relay is up. Caller holds the device lock.
    async fn ensure_relay_locked(
        &self,
        udid: &str,
        owned: &mut DeviceOwned,
    ) -> anyhow::Result<u16> {
        if self.sidecar.is_none() {
            anyhow::bail!("sidecar missing");
        }
        let port = self.port_for(udid);

        // Reuse a live relay. Transient /status blips right after an app launch
        // must NOT tear it down — that is what used to spawn a second relay.
        let proxy_alive = owned.proxy.as_mut().is_some_and(|c| !c.has_exited());
        if owned.wda_port == Some(port) && proxy_alive && !owned.force_restart {
            for _ in 0..4 {
                if self.wda_http_reachable(port).await {
                    return Ok(port);
                }
                tokio::time::sleep(Duration::from_millis(400)).await;
            }
        }

        // Something is already serving this port and we have no child of our
        // own: adopt it rather than start a competing relay.
        if owned.proxy.is_none() && !owned.force_restart && self.wda_http_reachable(port).await {
            owned.wda_port = Some(port);
            tracing::info!("adopting WDA already listening on {port} for {udid}");
            return Ok(port);
        }

        self.teardown_proxy_locked(owned).await;
        self.sessions.lock().remove(udid);

        let started = Instant::now();
        let force = owned.force_restart;
        let result = self.spawn_proxy_locked(udid, port, force, owned).await;
        telemetry::record_event(
            udid,
            if force {
                "relay_restart"
            } else {
                "relay_start"
            },
            started.elapsed().as_millis() as u32,
            &self.secret_free_result_detail(&result),
        );
        result?;

        for _ in 0..12 {
            if self.wda_http_reachable(port).await {
                owned.wda_port = Some(port);
                owned.force_restart = false;
                return Ok(port);
            }
            tokio::time::sleep(Duration::from_millis(300)).await;
        }
        self.teardown_proxy_locked(owned).await;
        anyhow::bail!(
            "Không kết nối được WDA qua USB (relay {port} mở nhưng /status im lặng). \
             Mở khoá iPhone, Trust developer, rồi bấm Start/Agent."
        )
    }

    async fn spawn_proxy_locked(
        &self,
        udid: &str,
        port: u16,
        force_restart: bool,
        owned: &mut DeviceOwned,
    ) -> anyhow::Result<()> {
        let args = proxy_args(&self.profile, udid, port, force_restart);
        // Never pipe stderr without a reader — the buffer fills and Python
        // deadlocks. Send it to a file when someone is debugging a relay that
        // dies right after coming up.
        let stderr = match std::env::var("RIVIU_PROXY_LOG") {
            Ok(path) if !path.is_empty() => std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)
                .map(Stdio::from)
                .unwrap_or_else(|_| Stdio::null()),
            _ => Stdio::null(),
        };
        let mut command = self.sidecar_command()?;
        command
            .args(&args)
            .stdout(Stdio::piped())
            .stderr(stderr)
            .kill_on_drop(true);
        if let Some(token) = self.profile.auth_token.as_ref() {
            command.env("RIVIU_RTMMO_TOKEN", token.expose());
        }
        let mut child: Child = command.spawn()?;

        let mut stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow::anyhow!("wda-proxy stdout missing"))?;

        // A restart kills the device-side runner first and waits for iOS to
        // reap it, so it needs a longer window than a plain start.
        let ready_window = proxy_ready_window(&self.profile, force_restart);
        let ready = tokio::time::timeout(ready_window, async {
            let mut buf = Vec::new();
            let mut byte = [0u8; 1];
            loop {
                stdout.read_exact(&mut byte).await?;
                if byte[0] == b'\n' {
                    break;
                }
                buf.push(byte[0]);
                if buf.len() > 8192 {
                    anyhow::bail!("wda-proxy ready line too long");
                }
            }
            let line = String::from_utf8_lossy(&buf);
            let v: serde_json::Value = serde_json::from_str(line.trim())
                .map_err(|e| anyhow::anyhow!("wda-proxy ready parse: {e} ({line})"))?;
            if v.get("ok").and_then(|x| x.as_bool()) != Some(true) {
                anyhow::bail!(
                    "{}",
                    v.get("error")
                        .and_then(|e| e.as_str())
                        .unwrap_or(line.trim())
                );
            }
            Ok::<(), anyhow::Error>(())
        })
        .await
        .unwrap_or_else(|_| Err(anyhow::anyhow!("timeout chờ wda-proxy")));

        if let Err(e) = ready {
            let _ = child.start_kill();
            let _ = child.wait().await;
            return Err(e);
        }

        owned.proxy = Some(OwnedChild::adopt(
            &self.registry,
            udid,
            Role::Proxy,
            child,
            &proxy_process_fingerprint(udid),
        ));
        Ok(())
    }

    async fn teardown_proxy_locked(&self, owned: &mut DeviceOwned) {
        if let Some(mut proxy) = owned.proxy.take() {
            proxy.shutdown().await;
        }
        owned.wda_port = None;
    }

    async fn stop_stream_child_locked(&self, owned: &mut DeviceOwned) -> bool {
        if let Some(mut stream) = owned.stream.take() {
            if !stream.shutdown_confirmed().await {
                owned.stream = Some(stream);
                return false;
            }
        }
        true
    }

    async fn teardown_stream_locked(&self, udid: &str, owned: &mut DeviceOwned) {
        self.stop_stream_child_locked(owned).await;
        self.streams.clear(udid);
        self.interaction_lifecycle.clear(udid);
    }

    /// Kill the relay *and* the device-side runner, then let the next call
    /// bring both back. Reserved for a confirmed wedge — never a slow probe.
    async fn recycle_locked(&self, udid: &str, owned: &mut DeviceOwned) {
        let started = Instant::now();
        let unexpected_stream = owned.stream.is_some();
        self.stop_stream_child_locked(owned).await;
        // DeviceControlPlane always obtains a stop proof before recycling. Do
        // not advance that generation again here; its second stop proof owns
        // the next transition. A direct caller with a live producer still
        // fails closed by invalidating that unexpected generation.
        if unexpected_stream {
            self.streams.clear(udid);
        }
        self.interaction_lifecycle.clear(udid);
        self.sessions.lock().remove(udid);
        let port = owned.wda_port;
        self.teardown_proxy_locked(owned).await;
        owned.force_restart = true;
        if let Some(port) = port {
            for _ in 0..24 {
                if !self.wda_http_reachable(port).await {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(250)).await;
            }
        }
        telemetry::record_event(
            udid,
            "hard_recycle",
            started.elapsed().as_millis() as u32,
            "relay + runner torn down",
        );
    }

    async fn spawn_stream_at_generation_locked(
        &self,
        udid: &str,
        owned: &mut DeviceOwned,
        generation: u64,
        readiness: StreamReadiness,
    ) -> anyhow::Result<bool> {
        if self.sidecar.is_none() {
            anyhow::bail!("sidecar missing");
        }
        if owned.stream.is_some() {
            anyhow::bail!("stream producer already exists for {udid}");
        }

        let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();

        let mut command = self.sidecar_command()?;
        command
            .args(stream_args(&self.profile, udid))
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        let mut child: Child = command.spawn()?;

        let mut stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow::anyhow!("stream stdout missing"))?;
        let streams = self.streams.clone();
        let udid_owned = udid.to_string();
        tokio::spawn(async move {
            let mut ready_tx = Some(ready_tx);
            let mut len_buf = [0u8; 4];
            loop {
                if stdout.read_exact(&mut len_buf).await.is_err() {
                    break;
                }
                let len = u32::from_be_bytes(len_buf) as usize;
                if len == 0 || len > 8_000_000 {
                    break;
                }
                let mut frame = vec![0u8; len];
                if stdout.read_exact(&mut frame).await.is_err() {
                    break;
                }
                let waiting_for_first_frame = ready_tx.is_some();
                let qualifies = !waiting_for_first_frame
                    || match readiness {
                        StreamReadiness::BestEffort | StreamReadiness::NonEmptyFrame => {
                            !frame.is_empty()
                        }
                        StreamReadiness::DecodedFrame => image::load_from_memory(&frame).is_ok(),
                    };
                if waiting_for_first_frame
                    && readiness == StreamReadiness::DecodedFrame
                    && !qualifies
                {
                    continue;
                }
                if streams.publish_if_current(&udid_owned, generation, frame) && qualifies {
                    if let Some(ready_tx) = ready_tx.take() {
                        let _ = ready_tx.send(());
                    }
                }
            }
            tracing::info!("stream ended for {udid_owned}");
        });

        owned.stream = Some(OwnedChild::adopt(
            &self.registry,
            udid,
            Role::Stream,
            child,
            &format!("stream --udid {udid}"),
        ));
        match readiness {
            StreamReadiness::BestEffort => {
                // Stock's lockdown fallback can be slow; startup remains best-effort.
                tokio::time::sleep(Duration::from_millis(200)).await;
                Ok(false)
            }
            StreamReadiness::NonEmptyFrame | StreamReadiness::DecodedFrame => {
                let observed = matches!(
                    tokio::time::timeout(Duration::from_secs(12), ready_rx).await,
                    Ok(Ok(()))
                );
                if !observed {
                    let expectation = if readiness == StreamReadiness::DecodedFrame {
                        "a decoded frame"
                    } else {
                        "a frame"
                    };
                    anyhow::bail!("MJPEG stream did not produce {expectation}");
                }
                Ok(true)
            }
        }
    }

    async fn ensure_stream_locked(
        &self,
        udid: &str,
        owned: &mut DeviceOwned,
    ) -> anyhow::Result<()> {
        self.interaction_lifecycle.clear(udid);
        if self.sidecar.is_none() {
            anyhow::bail!("sidecar missing");
        }
        if owned
            .stream
            .as_mut()
            .is_some_and(|child| !child.has_exited())
        {
            return Ok(());
        }
        self.stop_stream_child_locked(owned).await;
        let (_, generation) = self.streams.clear_and_advance(udid);
        let readiness = if self.profile.backend == WdaBackend::RtMmo {
            StreamReadiness::NonEmptyFrame
        } else {
            StreamReadiness::BestEffort
        };
        if let Err(error) = self
            .spawn_stream_at_generation_locked(udid, owned, generation, readiness)
            .await
        {
            if owned.stream.is_some() {
                self.teardown_stream_locked(udid, owned).await;
            }
            return Err(error);
        }
        Ok(())
    }

    async fn launch_app_locked(&self, udid: &str, bundle_id: &str) -> anyhow::Result<()> {
        let started = Instant::now();
        let result = tokio::time::timeout(
            Duration::from_secs(12),
            self.run_json(&["launch", "--udid", udid, "--bundle-id", bundle_id]),
        )
        .await
        .unwrap_or_else(|_| Err(anyhow::anyhow!("launch timeout ({bundle_id})")));
        telemetry::record_event(
            udid,
            "launch_app",
            started.elapsed().as_millis() as u32,
            if result.is_ok() { "ok" } else { "failed" },
        );
        result?;
        Ok(())
    }

    async fn require_existing_interaction_relay_locked(
        &self,
        _udid: &str,
        owned: &mut DeviceOwned,
    ) -> anyhow::Result<u16> {
        if owned.force_restart {
            anyhow::bail!(
                "install-only protected control relay is not reusable for interaction session"
            );
        }
        let port = owned.wda_port.context(
            "install-only protected control relay is required before interaction foreground",
        )?;
        if owned.proxy.as_mut().is_some_and(OwnedChild::has_exited) {
            anyhow::bail!(
                "install-only protected control relay exited before interaction foreground"
            );
        }
        if !self.wda_http_reachable(port).await {
            anyhow::bail!(
                "install-only protected control relay failed auth before interaction foreground"
            );
        }
        Ok(port)
    }

    /// Get a usable WDA session, creating relay/runner/session as needed.
    /// Caller holds the device lock.
    async fn session_locked(
        &self,
        udid: &str,
        owned: &mut DeviceOwned,
    ) -> anyhow::Result<WdaClient> {
        let port = self.ensure_relay_locked(udid, owned).await?;

        let cached = self.sessions.lock().get(udid).cloned();
        if let Some(client) = cached {
            if client.port() == port && client.session_alive().await {
                return Ok(client);
            }
            self.sessions.lock().remove(udid);
        }

        let mut last_err: Option<UiError> = None;
        for attempt in 0..2 {
            let mut client =
                WdaClient::new_with_profile(&self.wda_host, port, udid, self.profile_for(udid)?);
            match client.create_session().await {
                Ok(()) => {
                    // create_session succeeding is enough — window/size can
                    // false-negative under USB load right after session init.
                    self.sessions
                        .lock()
                        .insert(udid.to_string(), client.clone());
                    return Ok(client);
                }
                Err(e) => last_err = Some(e),
            }
            if attempt == 0 {
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
        }
        Err(anyhow::Error::new(last_err.unwrap_or_else(|| {
            UiError::new(UiErrorKind::Session, "session.create", "unavailable")
        })))
    }

    /// Interaction-only session setup. The explicit stop reservation must
    /// already exist, and this path never starts or restores a stream.
    async fn interaction_session_locked(
        &self,
        udid: &str,
        bundle_id: &str,
        kind: InteractionSessionKind,
        owned: &mut DeviceOwned,
    ) -> anyhow::Result<WdaClient> {
        if owned.stream.is_some() {
            anyhow::bail!("interaction session requires stop_owned_stream first");
        }
        if kind == InteractionSessionKind::FreshText && self.profile.backend != WdaBackend::RtMmo {
            self.interaction_lifecycle.clear(udid);
            anyhow::bail!("fresh-text interaction session requires the unified Agent profile");
        }
        let generation = self.streams.generation(udid);
        let reservation = self
            .interaction_lifecycle
            .begin_session(udid, generation, kind)?;
        self.sessions.lock().remove(udid);

        let transition: anyhow::Result<WdaClient> = async {
            let fresh_rt = kind == InteractionSessionKind::FreshText
                && self.profile.backend == WdaBackend::RtMmo;
            let existing_port = if fresh_rt {
                None
            } else {
                Some(
                    self.require_existing_interaction_relay_locked(udid, owned)
                        .await?,
                )
            };
            if fresh_rt {
                let local_port = self.port_for(udid);
                self.bootstrap_rt_text_agent_locked(udid, local_port)
                    .await?;
            }

            self.launch_app_locked(udid, bundle_id).await?;
            let port = match existing_port {
                Some(port) => port,
                None => self.ensure_relay_locked(udid, owned).await?,
            };
            let mut client =
                WdaClient::new_with_profile(&self.wda_host, port, udid, self.profile_for(udid)?);
            if fresh_rt {
                client.create_fresh_session().await?;
            } else {
                client.create_session().await?;
            }
            self.sessions
                .lock()
                .insert(udid.to_string(), client.clone());
            self.interaction_lifecycle.complete_session(&reservation)?;
            self.publish_interaction_readiness(
                udid,
                AgentState::Starting,
                (true, false, true),
                Some("Interaction session ready; stream pending".to_string()),
            );
            Ok(client)
        }
        .await;

        match transition {
            Ok(client) => Ok(client),
            Err(error) => {
                self.sessions.lock().remove(udid);
                self.interaction_lifecycle.clear(udid);
                self.stop_stream_child_locked(owned).await;
                self.teardown_proxy_locked(owned).await;
                owned.force_restart = false;
                let message = self.secret_free_error(&error);
                self.publish_status(self.status(
                    udid,
                    AgentState::Error,
                    None,
                    (false, false, false),
                    Some(message.clone()),
                ));
                anyhow::bail!("interaction session transition failed: {message}")
            }
        }
    }

    /// RT-MMO only: reset the agent's text channel, foreground the target app,
    /// then create a new session before MJPEG starts.
    async fn fresh_text_session_locked(
        &self,
        udid: &str,
        bundle_id: &str,
        owned: &mut DeviceOwned,
    ) -> anyhow::Result<WdaClient> {
        self.teardown_stream_locked(udid, owned).await;
        self.sessions.lock().remove(udid);

        let transition: anyhow::Result<WdaClient> = async {
            let local_port = self.port_for(udid);
            self.bootstrap_rt_text_agent_locked(udid, local_port)
                .await?;
            self.launch_app_locked(udid, bundle_id).await?;

            let port = self.ensure_relay_locked(udid, owned).await?;
            let mut client =
                WdaClient::new_with_profile(&self.wda_host, port, udid, self.profile_for(udid)?);
            client.create_fresh_session().await?;
            self.sessions
                .lock()
                .insert(udid.to_string(), client.clone());
            Ok(client)
        }
        .await;

        match transition {
            Ok(client) => Ok(client),
            Err(original) => {
                self.sessions.lock().remove(udid);
                self.teardown_stream_locked(udid, owned).await;
                self.teardown_proxy_locked(owned).await;
                owned.force_restart = false;
                let restore = async {
                    self.session_locked(udid, owned).await?;
                    self.ensure_stream_locked(udid, owned).await
                }
                .await;
                let message = self.secret_free_error(&original);
                self.publish_status(self.status(
                    udid,
                    AgentState::Error,
                    None,
                    (false, false, false),
                    Some(message),
                ));
                match restore {
                    Ok(()) => Err(original),
                    Err(cleanup) => Err(original.context(format!(
                        "ordinary UI channel restore also failed: {cleanup:#}"
                    ))),
                }
            }
        }
    }
}

struct PmdInstallOnlyRuntime<'a> {
    driver: &'a PmdIosDriver,
    artifact: &'a AgentArtifact,
    owned: &'a mut DeviceOwned,
    control_port: Option<u16>,
}

fn ensure_install_only_runtime_is_idle(
    has_owned_stream: bool,
    has_cached_session: bool,
) -> anyhow::Result<()> {
    if has_owned_stream || has_cached_session {
        anyhow::bail!(
            "install-only repair requires stop_owned_stream before inspecting or mutating the Agent"
        );
    }
    Ok(())
}

impl PmdInstallOnlyRuntime<'_> {
    async fn prepare_install_mutation(&mut self, udid: &str) -> anyhow::Result<()> {
        ensure_install_only_runtime_is_idle(
            self.owned.stream.is_some(),
            self.driver.sessions.lock().contains_key(udid),
        )?;
        self.driver.sessions.lock().remove(udid);
        self.driver.teardown_proxy_locked(self.owned).await;
        self.control_port = None;
        Ok(())
    }
}

#[async_trait]
impl InstallOnlyRuntime for PmdInstallOnlyRuntime<'_> {
    fn verify_artifact(&self) -> anyhow::Result<()> {
        self.artifact.verify_checksum()
    }

    fn artifact_sha256(&self) -> &str {
        &self.artifact.manifest.sha256
    }

    fn install_decision(&self, installed: Option<&InstalledAppMetadata>) -> AgentInstallDecision {
        decide_install(&self.artifact.manifest, installed, true)
    }

    async fn inspect(&mut self, udid: &str) -> anyhow::Result<Option<InstallOnlyInspection>> {
        let Some(metadata) = self.driver.inspect_agent_locked(udid).await? else {
            return Ok(None);
        };
        let identity = if decide_install(&self.artifact.manifest, Some(&metadata), false)
            == AgentInstallDecision::Reuse
        {
            Some(
                self.driver
                    .inspect_device_for_target_verified_using(
                        udid,
                        INTERACTION_TARGET_BUNDLE_ID,
                        InteractionInspectionTransport::LegacyUsbmux,
                    )
                    .await?
                    .installed_agent,
            )
        } else {
            None
        };
        Ok(Some(InstallOnlyInspection { metadata, identity }))
    }

    async fn uninstall(&mut self, udid: &str) -> anyhow::Result<()> {
        self.prepare_install_mutation(udid).await?;
        self.driver
            .run_json(&[
                "uninstall",
                "--udid",
                udid,
                "--bundle-id",
                &self.artifact.manifest.bundle_id,
            ])
            .await?;
        Ok(())
    }

    async fn install(&mut self, udid: &str) -> anyhow::Result<()> {
        self.prepare_install_mutation(udid).await?;
        self.driver.install_bundled_agent_locked(udid).await?;
        self.owned.force_restart = false;
        Ok(())
    }

    async fn launch_auth(&mut self, udid: &str) -> anyhow::Result<()> {
        self.control_port = Some(self.driver.ensure_relay_locked(udid, self.owned).await?);
        Ok(())
    }

    async fn protected_health(&mut self, _udid: &str) -> anyhow::Result<bool> {
        let port = self
            .control_port
            .context("install-only protected relay was not launched")?;
        Ok(self.driver.wda_http_reachable(port).await)
    }
}

async fn find_python() -> anyhow::Result<PathBuf> {
    for candidate in ["python3", "python"] {
        if let Ok(output) = background_command(candidate)
            .arg("--version")
            .output()
            .await
        {
            if output.status.success() {
                return Ok(PathBuf::from(candidate));
            }
        }
    }
    anyhow::bail!("python3 not found")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GuardedClipboardProof {
    pub stop: Option<StreamStopProof>,
    pub agent: Option<ActiveAppIdentity>,
    pub target: Option<ActiveAppIdentity>,
    pub stream: Option<StreamStartProof>,
}

#[derive(Debug)]
pub(crate) struct GuardedClipboardResult {
    pub output: GuardedClipboardOutput,
    pub proof: GuardedClipboardProof,
}

#[async_trait]
pub(crate) trait GuardedClipboardRuntime: Send {
    async fn stop_and_advance_stream(&mut self) -> anyhow::Result<StreamStopProof>;
    async fn foreground_agent(&mut self) -> anyhow::Result<()>;
    async fn probe_active_app(&mut self) -> anyhow::Result<ActiveAppIdentity>;
    async fn perform_clipboard(
        &mut self,
        operation: &GuardedClipboardOperation,
    ) -> anyhow::Result<GuardedClipboardOutput>;
    async fn foreground_target(&mut self, bundle_id: &str) -> anyhow::Result<()>;
    async fn create_final_session(&mut self, kind: InteractionSessionKind) -> anyhow::Result<()>;
    async fn start_new_stream(&mut self) -> anyhow::Result<StreamStartProof>;
}

pub(crate) async fn run_guarded_clipboard_transition(
    runtime: &mut (impl GuardedClipboardRuntime + ?Sized),
    progress: &GuardedClipboardProgress,
    mode: ClipboardAccessMode,
    agent_bundle_id: &str,
    target_bundle_id: &str,
    final_session_kind: InteractionSessionKind,
    operation: GuardedClipboardOperation,
) -> anyhow::Result<GuardedClipboardResult> {
    validate_guarded_clipboard_operation(&operation)?;
    if mode == ClipboardAccessMode::TargetBackgroundSafe {
        let target_before = runtime.probe_active_app().await?;
        require_active_identity(&target_before, target_bundle_id, "target")?;
        let output = runtime.perform_clipboard(&operation).await?;
        validate_guarded_clipboard_output(&operation, &output)?;
        let target_after = runtime.probe_active_app().await?;
        require_active_identity(&target_after, target_bundle_id, "target")?;
        if target_after.pid != target_before.pid {
            anyhow::bail!("background-safe clipboard changed the target PID");
        }
        return Ok(GuardedClipboardResult {
            output,
            proof: GuardedClipboardProof {
                stop: None,
                agent: None,
                target: Some(target_after),
                stream: None,
            },
        });
    }

    let stop = runtime.stop_and_advance_stream().await?;
    progress.record_stop(stop);
    if !stop.child_stopped || stop.new_generation <= stop.old_generation {
        anyhow::bail!("guarded clipboard requires a confirmed stream stop and new generation");
    }

    runtime.foreground_agent().await?;
    let agent_first = runtime.probe_active_app().await?;
    require_active_identity(&agent_first, agent_bundle_id, "Agent")?;
    let agent_stable = runtime.probe_active_app().await?;
    require_active_identity(&agent_stable, agent_bundle_id, "Agent")?;
    if agent_stable.pid != agent_first.pid {
        anyhow::bail!("guarded clipboard Agent PID changed before the clipboard operation");
    }

    let output = runtime.perform_clipboard(&operation).await?;
    validate_guarded_clipboard_output(&operation, &output)?;
    let agent_after = runtime.probe_active_app().await?;
    require_active_identity(&agent_after, agent_bundle_id, "Agent")?;
    if agent_after.pid != agent_first.pid {
        anyhow::bail!("guarded clipboard Agent PID changed during the clipboard operation");
    }

    runtime.foreground_target(target_bundle_id).await?;
    let target = runtime.probe_active_app().await?;
    require_active_identity(&target, target_bundle_id, "target")?;
    runtime.create_final_session(final_session_kind).await?;
    let target_after_session = runtime.probe_active_app().await?;
    require_active_identity(&target_after_session, target_bundle_id, "target")?;
    if target_after_session.pid != target.pid {
        anyhow::bail!("guarded clipboard target PID changed while creating the final session");
    }
    let stream = runtime.start_new_stream().await?;
    progress.record_stream(stream.clone());
    if stream.generation != stop.new_generation
        || !stream.first_frame_observed
        || stream.stream_url.trim().is_empty()
    {
        anyhow::bail!("guarded clipboard replacement stream generation is not proven fresh");
    }

    Ok(GuardedClipboardResult {
        output,
        proof: GuardedClipboardProof {
            stop: Some(stop),
            agent: Some(agent_first),
            target: Some(target_after_session),
            stream: Some(stream),
        },
    })
}

fn validate_guarded_clipboard_operation(
    operation: &GuardedClipboardOperation,
) -> anyhow::Result<()> {
    match operation {
        GuardedClipboardOperation::Set {
            content_type,
            bytes,
        } => {
            if content_type.trim().is_empty() || content_type.trim() != content_type {
                anyhow::bail!("guarded clipboard content type is blank or non-canonical");
            }
            if bytes.len() > MAX_INTERACTION_CLIPBOARD_BYTES {
                anyhow::bail!("guarded clipboard value exceeds 65536 bytes");
            }
        }
        GuardedClipboardOperation::Get {
            maximum_decoded_bytes,
        } => validate_clipboard_read_limit(*maximum_decoded_bytes)?,
    }
    Ok(())
}

fn validate_guarded_clipboard_output(
    operation: &GuardedClipboardOperation,
    output: &GuardedClipboardOutput,
) -> anyhow::Result<()> {
    match (operation, output) {
        (GuardedClipboardOperation::Set { .. }, GuardedClipboardOutput::Written) => Ok(()),
        (
            GuardedClipboardOperation::Get {
                maximum_decoded_bytes,
            },
            GuardedClipboardOutput::Value {
                content_type,
                bytes,
            },
        ) if !content_type.trim().is_empty()
            && content_type.trim() == content_type
            && bytes.len() <= *maximum_decoded_bytes =>
        {
            Ok(())
        }
        _ => anyhow::bail!("guarded clipboard output does not match the bounded operation"),
    }
}

fn require_active_identity(
    identity: &ActiveAppIdentity,
    expected_bundle_id: &str,
    role: &str,
) -> anyhow::Result<()> {
    if identity.bundle_id != expected_bundle_id || identity.pid == 0 {
        anyhow::bail!(
            "guarded clipboard {role} identity mismatch: expected {expected_bundle_id}, got {} pid {}",
            identity.bundle_id,
            identity.pid
        );
    }
    Ok(())
}

struct PmdGuardedClipboardRuntime<'a> {
    driver: &'a PmdIosDriver,
    udid: &'a str,
    client: WdaClient,
    mode: ClipboardAccessMode,
    target_bundle_id: &'a str,
    final_session: Option<Box<dyn UiSession>>,
}

#[async_trait]
impl GuardedClipboardRuntime for PmdGuardedClipboardRuntime<'_> {
    async fn stop_and_advance_stream(&mut self) -> anyhow::Result<StreamStopProof> {
        DeviceDriver::stop_owned_stream(self.driver, self.udid).await
    }

    async fn foreground_agent(&mut self) -> anyhow::Result<()> {
        DeviceDriver::launch_app(self.driver, self.udid, &self.driver.profile.bundle_id).await
    }

    async fn probe_active_app(&mut self) -> anyhow::Result<ActiveAppIdentity> {
        self.client
            .active_app_identity()
            .await
            .map_err(anyhow::Error::new)
    }

    async fn perform_clipboard(
        &mut self,
        operation: &GuardedClipboardOperation,
    ) -> anyhow::Result<GuardedClipboardOutput> {
        match operation {
            GuardedClipboardOperation::Set {
                content_type,
                bytes,
            } => {
                match self.mode {
                    ClipboardAccessMode::TargetBackgroundSafe => {
                        self.client.set_clipboard(content_type, bytes).await
                    }
                    ClipboardAccessMode::AgentForegroundRequired => {
                        self.client
                            .set_clipboard_agent_foregrounded(content_type, bytes)
                            .await
                    }
                }
                .map_err(anyhow::Error::new)?;
                Ok(GuardedClipboardOutput::Written)
            }
            GuardedClipboardOperation::Get {
                maximum_decoded_bytes,
            } => {
                let (content_type, bytes) = match self.mode {
                    ClipboardAccessMode::TargetBackgroundSafe => {
                        self.client.get_clipboard(*maximum_decoded_bytes).await
                    }
                    ClipboardAccessMode::AgentForegroundRequired => {
                        self.client
                            .get_clipboard_agent_foregrounded(*maximum_decoded_bytes)
                            .await
                    }
                }
                .map_err(anyhow::Error::new)?;
                Ok(GuardedClipboardOutput::Value {
                    content_type,
                    bytes,
                })
            }
        }
    }

    async fn foreground_target(&mut self, bundle_id: &str) -> anyhow::Result<()> {
        DeviceDriver::launch_app(self.driver, self.udid, bundle_id).await
    }

    async fn create_final_session(&mut self, kind: InteractionSessionKind) -> anyhow::Result<()> {
        self.final_session = Some(
            DeviceDriver::start_interaction_session(
                self.driver,
                self.udid,
                self.target_bundle_id,
                kind,
            )
            .await?,
        );
        Ok(())
    }

    async fn start_new_stream(&mut self) -> anyhow::Result<StreamStartProof> {
        DeviceDriver::start_stream_after_session(self.driver, self.udid).await
    }
}

struct PmdUiSession {
    client: WdaClient,
    mjpeg_url: String,
    supports_text_input: bool,
    supports_accessibility_readback: bool,
    target_bundle_id: String,
}

#[async_trait]
impl UiSession for PmdUiSession {
    async fn tap(&self, point: TapPoint) -> anyhow::Result<()> {
        self.client.tap(point).await.map_err(anyhow::Error::new)
    }

    async fn tap_native(&self, point: TapPoint) -> anyhow::Result<()> {
        self.client
            .tap_native(point)
            .await
            .map_err(anyhow::Error::new)
    }

    async fn swipe(&self, gesture: SwipeGesture) -> anyhow::Result<()> {
        self.client.swipe(gesture).await.map_err(anyhow::Error::new)
    }

    async fn tap_image(&self, x: f64, y: f64, image_w: f64, image_h: f64) -> anyhow::Result<()> {
        let point = self.client.to_points(x, y, image_w, image_h).await?;
        self.client.tap(point).await.map_err(anyhow::Error::new)
    }

    async fn swipe_image(
        &self,
        from: TapPoint,
        to: TapPoint,
        image_w: f64,
        image_h: f64,
        duration_ms: u64,
    ) -> anyhow::Result<()> {
        let from = self
            .client
            .to_points(from.x, from.y, image_w, image_h)
            .await?;
        let to = self.client.to_points(to.x, to.y, image_w, image_h).await?;
        self.client
            .swipe(SwipeGesture {
                from,
                to,
                duration_ms,
            })
            .await
            .map_err(anyhow::Error::new)
    }

    async fn type_text(&self, text: &str) -> anyhow::Result<()> {
        self.client
            .type_text(text)
            .await
            .map_err(anyhow::Error::new)
    }

    fn supports_text_input(&self) -> bool {
        self.supports_text_input
    }

    async fn read_text(
        &self,
        locator: &QualifiedElementLocator,
        request_timeout: Duration,
    ) -> anyhow::Result<String> {
        if !self.supports_accessibility_readback {
            anyhow::bail!("qualified accessibility read-back is unavailable");
        }
        self.client
            .read_text(locator, request_timeout)
            .await
            .map_err(anyhow::Error::new)
    }

    fn supports_accessibility_readback(&self) -> bool {
        self.supports_accessibility_readback
    }

    async fn home(&self) -> anyhow::Result<()> {
        self.client.home().await.map_err(anyhow::Error::new)
    }

    async fn find_and_tap(&self, accessibility_id: &str) -> anyhow::Result<()> {
        self.client
            .find_and_tap(accessibility_id)
            .await
            .map_err(anyhow::Error::new)
    }

    async fn assert_visible(&self, accessibility_id: &str) -> anyhow::Result<()> {
        self.client
            .assert_visible(accessibility_id)
            .await
            .map_err(anyhow::Error::new)
    }

    async fn dismiss_alert(&self) -> anyhow::Result<()> {
        self.client
            .dismiss_alert()
            .await
            .map_err(anyhow::Error::new)
    }

    async fn healthy(&self) -> bool {
        self.client.health_quick().await
    }

    async fn window_size(&self) -> anyhow::Result<(f64, f64)> {
        self.client.window_size().await.map_err(anyhow::Error::new)
    }

    async fn launch_app_foreground(&self, bundle_id: &str) -> anyhow::Result<()> {
        self.client
            .activate_app(bundle_id)
            .await
            .map_err(anyhow::Error::new)
    }

    async fn active_app_bundle(&self) -> anyhow::Result<String> {
        self.client
            .active_app_bundle()
            .await
            .map_err(anyhow::Error::new)
    }

    async fn open_url(&self, url: &str) -> anyhow::Result<()> {
        self.client.open_url(url).await.map_err(anyhow::Error::new)
    }

    async fn set_clipboard(&self, content_type: &str, bytes: &[u8]) -> anyhow::Result<()> {
        let before = self
            .client
            .active_app_identity()
            .await
            .map_err(anyhow::Error::new)?;
        require_active_identity(&before, &self.target_bundle_id, "target")?;
        self.client
            .set_clipboard(content_type, bytes)
            .await
            .map_err(anyhow::Error::new)?;
        let after = self
            .client
            .active_app_identity()
            .await
            .map_err(anyhow::Error::new)?;
        require_active_identity(&after, &self.target_bundle_id, "target")?;
        if after.pid != before.pid {
            anyhow::bail!("background-safe clipboard changed the target PID");
        }
        Ok(())
    }

    async fn get_clipboard(
        &self,
        maximum_decoded_bytes: usize,
    ) -> anyhow::Result<(String, Vec<u8>)> {
        let before = self
            .client
            .active_app_identity()
            .await
            .map_err(anyhow::Error::new)?;
        require_active_identity(&before, &self.target_bundle_id, "target")?;
        let value = self
            .client
            .get_clipboard(maximum_decoded_bytes)
            .await
            .map_err(anyhow::Error::new)?;
        let after = self
            .client
            .active_app_identity()
            .await
            .map_err(anyhow::Error::new)?;
        require_active_identity(&after, &self.target_bundle_id, "target")?;
        if after.pid != before.pid {
            anyhow::bail!("background-safe clipboard changed the target PID");
        }
        Ok(value)
    }

    async fn active_app_identity(&self) -> anyhow::Result<riviu_core::ActiveAppIdentity> {
        self.client
            .active_app_identity()
            .await
            .map_err(anyhow::Error::new)
    }

    async fn screenshot_png(&self) -> anyhow::Result<Vec<u8>> {
        self.client
            .screenshot_png()
            .await
            .map_err(anyhow::Error::new)
    }

    fn stream_url(&self) -> Option<String> {
        Some(self.mjpeg_url.clone())
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ProcessAbsencePayload {
    ok: bool,
    bundle_id: String,
    #[serde(deserialize_with = "deserialize_optional_positive_pid")]
    old_pid: Option<u64>,
    running: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AppProcessPayload {
    ok: bool,
    bundle_id: String,
    #[serde(deserialize_with = "deserialize_optional_positive_pid")]
    pid: Option<u64>,
    running: bool,
}

fn deserialize_optional_positive_pid<'de, D>(deserializer: D) -> Result<Option<u64>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let pid = Option::<u64>::deserialize(deserializer)?;
    if pid == Some(0) {
        return Err(serde::de::Error::custom("PID must be positive"));
    }
    Ok(pid)
}

fn require_exact_fields(value: &serde_json::Value, expected: &[&str]) -> anyhow::Result<()> {
    let object = value
        .as_object()
        .context("sidecar process response must be a JSON object")?;
    if object.len() != expected.len() || expected.iter().any(|field| !object.contains_key(*field)) {
        anyhow::bail!(
            "sidecar process response must contain exactly: {}",
            expected.join(", ")
        );
    }
    Ok(())
}

fn parse_process_absence_proof(
    value: serde_json::Value,
    expected_bundle_id: &str,
) -> anyhow::Result<ProcessAbsenceProof> {
    require_exact_fields(&value, &["ok", "bundleId", "oldPid", "running"])?;
    let payload: ProcessAbsencePayload =
        serde_json::from_value(value).context("decode verified terminate response")?;
    if !payload.ok {
        anyhow::bail!("sidecar did not confirm termination");
    }
    if payload.bundle_id != expected_bundle_id {
        anyhow::bail!("sidecar terminate response bundle does not match request");
    }
    if payload.running {
        anyhow::bail!("sidecar terminate response still reports a running process");
    }
    Ok(ProcessAbsenceProof {
        bundle_id: payload.bundle_id,
        old_pid: payload.old_pid,
    })
}

fn parse_app_process_state(
    value: serde_json::Value,
    expected_bundle_id: &str,
) -> anyhow::Result<AppProcessState> {
    require_exact_fields(&value, &["ok", "bundleId", "pid", "running"])?;
    let payload: AppProcessPayload =
        serde_json::from_value(value).context("decode app process response")?;
    if !payload.ok {
        anyhow::bail!("sidecar did not confirm process state");
    }
    if payload.bundle_id != expected_bundle_id {
        anyhow::bail!("sidecar process response bundle does not match request");
    }
    if payload.running != payload.pid.is_some() {
        anyhow::bail!("sidecar process response has inconsistent running and pid fields");
    }
    Ok(AppProcessState {
        bundle_id: payload.bundle_id,
        pid: payload.pid,
        running: payload.running,
    })
}

#[async_trait]
impl DeviceDriver for PmdIosDriver {
    fn agent_settings(&self) -> AgentSettings {
        self.agent_settings.read().clone()
    }

    fn set_agent_settings(&self, settings: AgentSettings) {
        *self.agent_settings.write() = settings;
    }

    fn cached_agent_status(&self, udid: &str) -> AgentStatus {
        self.agent_statuses
            .lock()
            .get(udid)
            .cloned()
            .unwrap_or_else(|| {
                self.status(udid, AgentState::Unknown, None, (false, false, false), None)
            })
    }

    async fn preflight_agent(&self, udid: &str) -> anyhow::Result<AgentStatus> {
        let slot = self.slots.get(udid);
        let mut owned = slot.owned.lock().await;
        self.preflight_agent_locked(udid, &mut owned).await
    }

    async fn repair_agent(&self, udid: &str) -> anyhow::Result<AgentStatus> {
        if self.profile.backend != WdaBackend::RtMmo {
            anyhow::bail!("legacy stock WDA cannot repair the unified Riviu Agent");
        }
        let slot = self.slots.get(udid);
        let mut owned = slot.owned.lock().await;
        self.repair_agent_locked(udid, &mut owned).await
    }

    async fn repair_agent_install_only(&self, udid: &str) -> anyhow::Result<AgentInstallProof> {
        if self.profile.backend != WdaBackend::RtMmo {
            anyhow::bail!("legacy stock WDA cannot repair the unified Riviu Agent");
        }
        let artifact = self.artifact()?;
        let slot = self.slots.get(udid);
        let mut owned = slot.owned.lock().await;
        ensure_install_only_runtime_is_idle(
            owned.stream.is_some(),
            self.sessions.lock().contains_key(udid),
        )?;
        self.negotiated_ui.lock().remove(udid);
        self.publish_status(self.status(
            udid,
            AgentState::Starting,
            None,
            (false, false, false),
            Some("Checking install-only Agent readiness".to_string()),
        ));
        let (result, protected_relay_touched) = {
            let mut runtime = PmdInstallOnlyRuntime {
                driver: self,
                artifact,
                owned: &mut owned,
                control_port: None,
            };
            let result = repair_install_only_locked(&mut runtime, udid).await;
            (result, runtime.control_port.is_some())
        };

        match result {
            Ok(proof) => {
                let installed = InstalledAppMetadata {
                    bundle_id: proof.installed.bundle_id.clone(),
                    version: Some(proof.installed.version.clone()),
                    build: Some(proof.installed.build.clone()),
                    application_type: None,
                    path: None,
                    signer_identity: None,
                };
                self.publish_status(self.status(
                    udid,
                    AgentState::Starting,
                    Some(&installed),
                    (true, false, false),
                    Some("Agent auth ready; interaction session and stream pending".to_string()),
                ));
                Ok(proof)
            }
            Err(error) => {
                if protected_relay_touched {
                    self.teardown_proxy_locked(&mut owned).await;
                }
                self.sessions.lock().remove(udid);
                let message = self.secret_free_error(&error);
                self.publish_status(self.status(
                    udid,
                    AgentState::Error,
                    None,
                    (false, false, false),
                    Some(message.clone()),
                ));
                anyhow::bail!(message)
            }
        }
    }

    async fn guarded_clipboard_transition(
        &self,
        udid: &str,
        agent_bundle_id: &str,
        target_bundle_id: &str,
        final_session_kind: InteractionSessionKind,
        mode: ClipboardAccessMode,
        operation: GuardedClipboardOperation,
        progress: GuardedClipboardProgress,
    ) -> anyhow::Result<GuardedClipboardTransition> {
        if agent_bundle_id != self.profile.bundle_id {
            anyhow::bail!("guarded clipboard Agent bundle does not match the active profile");
        }
        let client = self
            .sessions
            .lock()
            .get(udid)
            .cloned()
            .context("guarded clipboard requires the current interaction session")?;
        let mut runtime = PmdGuardedClipboardRuntime {
            driver: self,
            udid,
            client,
            mode,
            target_bundle_id,
            final_session: None,
        };
        let result = run_guarded_clipboard_transition(
            &mut runtime,
            &progress,
            mode,
            agent_bundle_id,
            target_bundle_id,
            final_session_kind,
            operation,
        )
        .await?;
        let target = result
            .proof
            .target
            .context("guarded clipboard target identity proof is missing")?;
        Ok(GuardedClipboardTransition {
            output: result.output,
            stop: result.proof.stop,
            agent: result.proof.agent,
            target,
            final_session: runtime.final_session,
            stream: result.proof.stream,
        })
    }

    async fn stop_owned_stream(&self, udid: &str) -> anyhow::Result<StreamStopProof> {
        let slot = self.slots.get(udid);
        let mut owned = slot.owned.lock().await;
        let child_stopped = self.stop_stream_child_locked(&mut owned).await;
        let (old_generation, new_generation) = self.streams.clear_and_advance(udid);
        self.sessions.lock().remove(udid);
        if child_stopped {
            self.interaction_lifecycle
                .record_stopped(udid, new_generation);
        } else {
            self.interaction_lifecycle.clear(udid);
        }
        let auth_ready = self.cached_agent_status(udid).auth_ready;
        self.publish_interaction_readiness(
            udid,
            AgentState::Starting,
            (auth_ready, false, false),
            Some("UI stream stopped; interaction session pending".to_string()),
        );
        Ok(StreamStopProof {
            old_generation,
            new_generation,
            child_stopped,
        })
    }

    async fn confirm_interaction_stream_stopped(
        &self,
        udid: &str,
    ) -> anyhow::Result<StreamHandoffProof> {
        let slot = self.slots.get(udid);
        let owned = slot.owned.lock().await;
        if owned.stream.is_some() {
            anyhow::bail!("interaction handoff still owns an MJPEG producer");
        }
        if self.sessions.lock().contains_key(udid) {
            anyhow::bail!("interaction handoff still has a cached session");
        }

        let generation = self.streams.generation(udid);
        self.interaction_lifecycle.record_stopped(udid, generation);
        Ok(StreamHandoffProof { generation })
    }

    async fn read_active_app_bundle(&self, udid: &str) -> anyhow::Result<String> {
        let client =
            self.sessions.lock().get(udid).cloned().ok_or_else(|| {
                anyhow::anyhow!("active-app reconciliation has no cached session")
            })?;
        client.active_app_bundle().await.map_err(anyhow::Error::new)
    }

    async fn inspect_interaction_device(
        &self,
        udid: &str,
    ) -> anyhow::Result<DeviceCapabilitySnapshot> {
        self.inspect_device_for_target(udid, INTERACTION_TARGET_BUNDLE_ID)
            .await
    }

    async fn inspect_device_for_target(
        &self,
        udid: &str,
        target_bundle_id: &str,
    ) -> anyhow::Result<DeviceCapabilitySnapshot> {
        self.inspect_device_for_target_using(
            udid,
            target_bundle_id,
            InteractionInspectionTransport::LegacyUsbmux,
        )
        .await
    }

    async fn set_negotiated_interaction_capabilities(
        &self,
        udid: &str,
        capabilities: UiCapabilities,
    ) -> anyhow::Result<()> {
        self.profile
            .clone()
            .with_interaction_capabilities(capabilities.clone())
            .map_err(anyhow::Error::new)?;
        let slot = self.slots.get(udid);
        let owned = slot.owned.lock().await;
        if owned.stream.is_some() || self.sessions.lock().contains_key(udid) {
            anyhow::bail!(
                "interaction capabilities can only change before session and stream startup"
            );
        }
        if capabilities == UiCapabilities::default() {
            self.negotiated_ui.lock().remove(udid);
        } else {
            self.negotiated_ui
                .lock()
                .insert(udid.to_string(), capabilities);
        }
        self.interaction_lifecycle.clear(udid);
        Ok(())
    }

    fn supports_text_comments(&self) -> bool {
        self.profile.backend == WdaBackend::RtMmo
    }

    fn supports_verified_app_termination(&self) -> bool {
        self.verified_app_termination
    }

    async fn inspect_app_process(
        &self,
        udid: &str,
        bundle_id: &str,
    ) -> anyhow::Result<AppProcessState> {
        let slot = self.slots.get(udid);
        let _owned = slot.owned.lock().await;
        let value = self
            .run_json(&["app-process", "--udid", udid, "--bundle-id", bundle_id])
            .await?;
        parse_app_process_state(value, bundle_id)
    }

    async fn list_devices(&self) -> anyhow::Result<Vec<DeviceInfo>> {
        if self.sidecar.is_none() {
            return Ok(Vec::new());
        }
        let value = match self.run_json(&["list"]).await {
            Ok(v) => v,
            Err(_) => return Ok(Vec::new()),
        };
        let devices = value
            .get("devices")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        let mut out = Vec::new();
        for d in devices {
            let udid = d
                .get("udid")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            if udid.is_empty() {
                continue;
            }
            let conn = match d
                .get("connection")
                .and_then(|v| v.as_str())
                .unwrap_or("usb")
            {
                "wifi" | "network" => ConnectionKind::Wifi,
                _ => ConnectionKind::Usb,
            };
            let streaming = FrameSource::latest(&self.streams, &udid).is_some();
            out.push(DeviceInfo {
                udid: udid.clone(),
                name: d
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("iPhone")
                    .to_string(),
                model: d
                    .get("model")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Unknown")
                    .to_string(),
                ios_version: d
                    .get("iosVersion")
                    .and_then(|v| v.as_str())
                    .unwrap_or("?")
                    .to_string(),
                connection: conn,
                status: if streaming {
                    DeviceStatus::Ready
                } else {
                    DeviceStatus::Connected
                },
                battery: d.get("battery").and_then(|v| v.as_u64()).map(|v| v as u8),
                wda_ready: streaming,
                wda_expires_at: None,
                stream_url: if streaming {
                    Some(format!("screenshot-stream://{udid}"))
                } else {
                    None
                },
                tile_stream_state: if streaming {
                    TileStreamState::Live
                } else {
                    TileStreamState::Parked
                },
                last_error: d
                    .get("pairingError")
                    .and_then(|v| v.as_str())
                    .map(String::from),
            });
        }
        Ok(out)
    }

    async fn refresh_device(&self, udid: &str) -> anyhow::Result<DeviceInfo> {
        self.list_devices()
            .await?
            .into_iter()
            .find(|d| d.udid == udid)
            .ok_or_else(|| anyhow::anyhow!("device not found"))
    }

    async fn install_app(&self, udid: &str, path: &Path) -> anyhow::Result<()> {
        self.run_json(&[
            "install",
            "--udid",
            udid,
            "--ipa",
            &path.display().to_string(),
        ])
        .await?;
        Ok(())
    }

    async fn uninstall_app(&self, udid: &str, bundle_id: &str) -> anyhow::Result<()> {
        self.run_json(&["uninstall", "--udid", udid, "--bundle-id", bundle_id])
            .await?;
        Ok(())
    }

    async fn screenshot(&self, udid: &str, dest: &Path) -> anyhow::Result<PathBuf> {
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }
        self.run_json(&[
            "screenshot",
            "--udid",
            udid,
            "--out",
            &dest.display().to_string(),
        ])
        .await?;
        Ok(dest.to_path_buf())
    }

    async fn syslog_tail(&self, udid: &str, lines: usize) -> anyhow::Result<String> {
        let value = self
            .run_json(&["syslog", "--udid", udid, "--lines", &lines.to_string()])
            .await?;
        Ok(value
            .get("log")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string())
    }

    /// Launch by bundle id via the Instruments channel.
    ///
    /// Held under the device lock because this competes with the WDA relay for
    /// the same usbmux connection — running the two concurrently is what wedged
    /// usbmux in live tests #5–#7. Prefer `UiSession::launch_app_foreground`
    /// (WDA activate) when a session already exists; this is the cold path.
    async fn launch_app(&self, udid: &str, bundle_id: &str) -> anyhow::Result<()> {
        let slot = self.slots.get(udid);
        let _owned = slot.owned.lock().await;
        self.launch_app_locked(udid, bundle_id).await
    }

    async fn terminate_app(
        &self,
        udid: &str,
        bundle_id: &str,
    ) -> anyhow::Result<ProcessAbsenceProof> {
        let slot = self.slots.get(udid);
        let _owned = slot.owned.lock().await;
        let value = self
            .run_json(&["terminate", "--udid", udid, "--bundle-id", bundle_id])
            .await?;
        parse_process_absence_proof(value, bundle_id)
    }

    async fn reboot(&self, udid: &str) -> anyhow::Result<()> {
        self.run_json(&["reboot", "--udid", udid]).await?;
        Ok(())
    }

    async fn start_interaction_session(
        &self,
        udid: &str,
        bundle_id: &str,
        kind: InteractionSessionKind,
    ) -> anyhow::Result<Box<dyn UiSession>> {
        let slot = self.slots.get(udid);
        let mut owned = slot.owned.lock().await;
        let client = self
            .interaction_session_locked(udid, bundle_id, kind, &mut owned)
            .await?;
        Ok(Box::new(PmdUiSession {
            client,
            mjpeg_url: WdaClient::mjpeg_url(&self.wda_host, self.profile.mjpeg_port),
            supports_text_input: kind == InteractionSessionKind::FreshText
                && self.profile.backend == WdaBackend::RtMmo,
            supports_accessibility_readback: kind == InteractionSessionKind::FreshText
                && self.profile.backend == WdaBackend::RtMmo,
            target_bundle_id: bundle_id.to_string(),
        }))
    }

    async fn foreground_target_app_and_start_interaction_session(
        &self,
        udid: &str,
        bundle_id: &str,
        kind: InteractionSessionKind,
    ) -> anyhow::Result<Box<dyn UiSession>> {
        // interaction_session_locked owns the required bootstrap -> foreground ->
        // fresh-session order, so invoking launch_app separately would double-launch.
        self.start_interaction_session(udid, bundle_id, kind).await
    }

    async fn start_ui_session(&self, udid: &str) -> anyhow::Result<Box<dyn UiSession>> {
        let slot = self.slots.get(udid);
        let mut owned = slot.owned.lock().await;
        if self.profile.backend == WdaBackend::RtMmo {
            if owned.stream.is_some() {
                anyhow::bail!(
                    "session-only startup requires the owned MJPEG producer to be stopped first"
                );
            }
            self.require_existing_interaction_relay_locked(udid, &mut owned)
                .await?;
        }
        let client = self.session_locked(udid, &mut owned).await?;
        if self.profile.backend == WdaBackend::RtMmo {
            self.publish_interaction_readiness(
                udid,
                AgentState::Starting,
                (true, false, true),
                Some("Control session ready; stream not started".to_string()),
            );
        }
        Ok(Box::new(PmdUiSession {
            client,
            mjpeg_url: WdaClient::mjpeg_url(&self.wda_host, self.profile.mjpeg_port),
            supports_text_input: false,
            supports_accessibility_readback: false,
            target_bundle_id: INTERACTION_TARGET_BUNDLE_ID.to_string(),
        }))
    }

    fn requires_fresh_text_session(&self) -> bool {
        self.profile.backend == WdaBackend::RtMmo
    }

    async fn start_fresh_text_session(
        &self,
        udid: &str,
        bundle_id: &str,
    ) -> anyhow::Result<Box<dyn UiSession>> {
        let slot = self.slots.get(udid);
        let mut owned = slot.owned.lock().await;
        let client = if self.profile.backend == WdaBackend::RtMmo {
            let status = self.preflight_agent_locked(udid, &mut owned).await?;
            if status.state != AgentState::Ready {
                anyhow::bail!(
                    "Riviu Agent is not ready: {}",
                    status
                        .message
                        .unwrap_or_else(|| "repair required".to_string())
                );
            }
            self.fresh_text_session_locked(udid, bundle_id, &mut owned)
                .await?
        } else {
            self.session_locked(udid, &mut owned).await?
        };
        Ok(Box::new(PmdUiSession {
            client,
            mjpeg_url: WdaClient::mjpeg_url(&self.wda_host, self.profile.mjpeg_port),
            supports_text_input: self.profile.backend == WdaBackend::RtMmo,
            supports_accessibility_readback: self.profile.backend == WdaBackend::RtMmo,
            target_bundle_id: bundle_id.to_string(),
        }))
    }

    async fn ui_session_cached(&self, udid: &str) -> bool {
        self.sessions.lock().contains_key(udid)
    }

    fn invalidate_ui_session(&self, udid: &str) {
        // Soft: drop the cached session id only. Force-killing the agent on
        // every reopen caused Instruments death spirals when health probes
        // false-negatived.
        self.sessions.lock().remove(udid);
        self.interaction_lifecycle.clear(udid);
    }

    async fn recycle_ui_transport(&self, udid: &str) {
        let slot = self.slots.get(udid);
        let mut owned = slot.owned.lock().await;
        self.recycle_locked(udid, &mut owned).await;
    }

    async fn start_stream_after_session(&self, udid: &str) -> anyhow::Result<StreamStartProof> {
        let slot = self.slots.get(udid);
        let mut owned = slot.owned.lock().await;
        if owned.stream.is_some() {
            anyhow::bail!("interaction stream requires the old producer to be stopped first");
        }
        if !self.sessions.lock().contains_key(udid) {
            self.interaction_lifecycle.clear(udid);
            anyhow::bail!("interaction stream requires a driver-held session reservation");
        }

        let generation = self.streams.generation(udid);
        let reservation = self
            .interaction_lifecycle
            .reserve_stream(udid, generation)?;
        let transition = async {
            let first_frame_observed = self
                .spawn_stream_at_generation_locked(
                    udid,
                    &mut owned,
                    reservation.generation(),
                    StreamReadiness::DecodedFrame,
                )
                .await?;
            self.interaction_lifecycle.complete_stream(&reservation)?;
            Ok::<StreamStartProof, anyhow::Error>(StreamStartProof {
                generation: reservation.generation(),
                first_frame_observed,
                stream_url: format!("auto-stream://{udid}"),
            })
        }
        .await;

        match transition {
            Ok(proof) => {
                self.publish_interaction_readiness(
                    udid,
                    AgentState::Ready,
                    (true, true, true),
                    None,
                );
                Ok(proof)
            }
            Err(error) => {
                self.teardown_stream_locked(udid, &mut owned).await;
                self.sessions.lock().remove(udid);
                self.teardown_proxy_locked(&mut owned).await;
                owned.force_restart = false;
                let message = self.secret_free_error(&error);
                self.publish_interaction_readiness(
                    udid,
                    AgentState::Error,
                    (false, false, false),
                    Some(message.clone()),
                );
                anyhow::bail!("interaction stream transition failed: {message}")
            }
        }
    }

    async fn ensure_stream(&self, udid: &str) -> anyhow::Result<String> {
        let slot = self.slots.get(udid);
        let mut owned = slot.owned.lock().await;
        if self.profile.backend == WdaBackend::RtMmo {
            let status = self.preflight_agent_locked(udid, &mut owned).await?;
            if status.state != AgentState::Ready {
                anyhow::bail!(
                    "Riviu Agent is not ready: {}",
                    status
                        .message
                        .unwrap_or_else(|| "repair required".to_string())
                );
            }
            return Ok(format!("auto-stream://{udid}"));
        }
        let stream_running = owned
            .stream
            .as_mut()
            .is_some_and(|child| !child.has_exited());
        if !session_attach_required(stream_running) {
            return Ok(format!("auto-stream://{udid}"));
        }
        // The session must attach before MJPEG starts. Both services live in
        // the device agent and starting the stream first can wedge its first
        // session-scoped command.
        self.session_locked(udid, &mut owned).await?;
        self.ensure_stream_locked(udid, &mut owned).await?;
        Ok(format!("auto-stream://{udid}"))
    }

    async fn prepare_device(&self, udid: &str) -> anyhow::Result<()> {
        self.repair_agent_install_only(udid).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AgentArtifact, AgentToken, UnifiedAgentConfig};
    use riviu_core::{
        ActiveAppIdentity, AgentSettings, ClipboardAccessMode, OpenUrlCapability,
        ProtectedRouteContract, RouteMethod, RouteScope,
    };
    use std::collections::VecDeque;

    struct LegacyTerminateSidecarFixture {
        root: PathBuf,
        args_path: PathBuf,
    }

    impl LegacyTerminateSidecarFixture {
        fn new(ping_payload: serde_json::Value, ping_exit_code: i32) -> Self {
            let nonce = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("fixture clock after epoch")
                .as_nanos();
            let root = std::env::temp_dir().join(format!(
                "riviu-legacy-terminate-sidecar-{}-{nonce}",
                std::process::id()
            ));
            let sidecar_dir = root.join("pymobiledevice3");
            std::fs::create_dir_all(&sidecar_dir).expect("create legacy sidecar fixture");
            let script = sidecar_dir.join("riviu_pmd.py");
            let args_path = sidecar_dir.join("riviu_pmd.args.json");
            let source = r#"import json
import pathlib
import sys

if sys.argv[1:] == ['ping']:
    print(json.dumps(json.loads('__PING_PAYLOAD__')), flush=True)
    raise SystemExit(__PING_EXIT__)

pathlib.Path(__file__).with_suffix('.args.json').write_text(
    json.dumps(sys.argv[1:]), encoding='utf-8'
)
print(json.dumps({'ok': True, 'note': 'terminate best-effort'}), flush=True)
"#
            .replace(
                "__PING_PAYLOAD__",
                &serde_json::to_string(&ping_payload).expect("serialize fixture ping"),
            )
            .replace("__PING_EXIT__", &ping_exit_code.to_string());
            std::fs::write(&script, source).expect("write legacy sidecar fixture");
            Self { root, args_path }
        }
    }

    impl Drop for LegacyTerminateSidecarFixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    #[tokio::test]
    async fn bundled_runtime_is_preferred_over_the_development_python_script() {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("fixture clock after epoch")
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "riviu-bundled-sidecar-{}-{nonce}",
            std::process::id()
        ));
        let sidecar_dir = root.join("pymobiledevice3");
        let runtime_dir = sidecar_dir.join("runtime");
        std::fs::create_dir_all(&runtime_dir).expect("create bundled runtime fixture");
        std::fs::write(
            sidecar_dir.join("riviu_pmd.py"),
            b"# development fallback\n",
        )
        .expect("write development fixture");
        let bundled = runtime_dir.join(bundled_sidecar_filename());
        std::fs::write(&bundled, b"fixture").expect("write bundled entrypoint fixture");

        let resolved = SidecarProgram::resolve(&root)
            .await
            .expect("resolve bundled sidecar");
        std::fs::remove_dir_all(&root).expect("remove bundled runtime fixture");

        assert!(resolved.is_bundled());
        assert_eq!(resolved.executable, bundled);
        assert!(resolved.prefix_args.is_empty());
    }

    struct FixtureClipboardRuntime {
        calls: Vec<&'static str>,
        identities: VecDeque<ActiveAppIdentity>,
        stop_proof: StreamStopProof,
        start_proof: StreamStartProof,
    }

    #[async_trait]
    impl GuardedClipboardRuntime for FixtureClipboardRuntime {
        async fn stop_and_advance_stream(&mut self) -> anyhow::Result<StreamStopProof> {
            self.calls.push("stop");
            Ok(self.stop_proof)
        }

        async fn foreground_agent(&mut self) -> anyhow::Result<()> {
            self.calls.push("foreground-agent");
            Ok(())
        }

        async fn probe_active_app(&mut self) -> anyhow::Result<ActiveAppIdentity> {
            self.calls.push("probe-active");
            self.identities
                .pop_front()
                .ok_or_else(|| anyhow::anyhow!("fixture identity exhausted"))
        }

        async fn perform_clipboard(
            &mut self,
            operation: &GuardedClipboardOperation,
        ) -> anyhow::Result<GuardedClipboardOutput> {
            self.calls.push("clipboard");
            Ok(match operation {
                GuardedClipboardOperation::Set { .. } => GuardedClipboardOutput::Written,
                GuardedClipboardOperation::Get { .. } => GuardedClipboardOutput::Value {
                    content_type: "plaintext".to_string(),
                    bytes: b"fixture".to_vec(),
                },
            })
        }

        async fn foreground_target(&mut self, _bundle_id: &str) -> anyhow::Result<()> {
            self.calls.push("foreground-target");
            Ok(())
        }

        async fn create_final_session(
            &mut self,
            _kind: InteractionSessionKind,
        ) -> anyhow::Result<()> {
            self.calls.push("final-session");
            Ok(())
        }

        async fn start_new_stream(&mut self) -> anyhow::Result<StreamStartProof> {
            self.calls.push("start-stream");
            Ok(self.start_proof.clone())
        }
    }

    fn fixture_clipboard_runtime() -> FixtureClipboardRuntime {
        FixtureClipboardRuntime {
            calls: Vec::new(),
            identities: VecDeque::from([
                ActiveAppIdentity {
                    bundle_id: "com.fixture.agent".to_string(),
                    pid: 701,
                },
                ActiveAppIdentity {
                    bundle_id: "com.fixture.agent".to_string(),
                    pid: 701,
                },
                ActiveAppIdentity {
                    bundle_id: "com.fixture.agent".to_string(),
                    pid: 701,
                },
                ActiveAppIdentity {
                    bundle_id: INTERACTION_TARGET_BUNDLE_ID.to_string(),
                    pid: 902,
                },
                ActiveAppIdentity {
                    bundle_id: INTERACTION_TARGET_BUNDLE_ID.to_string(),
                    pid: 902,
                },
            ]),
            stop_proof: StreamStopProof {
                old_generation: 9,
                new_generation: 10,
                child_stopped: true,
            },
            start_proof: StreamStartProof {
                generation: 10,
                first_frame_observed: true,
                stream_url: "fixture://fresh-generation-10".to_string(),
            },
        }
    }

    #[test]
    fn terminate_protocol_requires_exact_verified_process_absence() {
        let running = parse_process_absence_proof(
            serde_json::json!({
                "ok": true,
                "bundleId": "com.fixture.app",
                "oldPid": 42,
                "running": false,
            }),
            "com.fixture.app",
        )
        .expect("verified process absence");
        assert_eq!(running.bundle_id, "com.fixture.app");
        assert_eq!(running.old_pid, Some(42));

        let absent = parse_process_absence_proof(
            serde_json::json!({
                "ok": true,
                "bundleId": "com.fixture.app",
                "oldPid": null,
                "running": false,
            }),
            "com.fixture.app",
        )
        .expect("already absent process");
        assert_eq!(absent.old_pid, None);

        for invalid in [
            serde_json::json!({"ok": true, "note": "terminate best-effort"}),
            serde_json::json!({
                "ok": true,
                "bundleId": "com.fixture.app",
                "running": false,
            }),
            serde_json::json!({
                "ok": true,
                "bundleId": "com.fixture.app",
                "oldPid": 0,
                "running": false,
            }),
            serde_json::json!({
                "ok": true,
                "bundleId": "com.fixture.app",
                "oldPid": true,
                "running": false,
            }),
            serde_json::json!({
                "ok": true,
                "bundleId": "com.fixture.other",
                "oldPid": 42,
                "running": false,
            }),
            serde_json::json!({
                "ok": true,
                "bundleId": "com.fixture.app",
                "oldPid": 42,
                "running": true,
            }),
            serde_json::json!({
                "ok": true,
                "bundleId": "com.fixture.app",
                "oldPid": 42,
                "running": false,
                "extra": true,
            }),
        ] {
            assert!(
                parse_process_absence_proof(invalid, "com.fixture.app").is_err(),
                "invalid terminate payload must be rejected"
            );
        }
    }

    #[tokio::test]
    async fn terminate_driver_rejects_the_legacy_best_effort_sidecar_payload() {
        let fixture = LegacyTerminateSidecarFixture::new(
            serde_json::json!({ "ok": true, "pymobiledevice3": true }),
            0,
        );
        let driver = PmdIosDriver::probe(&DriverConfig {
            sidecar_root: fixture.root.clone(),
            state_dir: fixture.root.join("state"),
            target: DriverTarget::LegacyStock,
        })
        .await
        .expect("probe legacy sidecar");

        assert!(!driver.supports_verified_app_termination());

        let error = DeviceDriver::terminate_app(&driver, "fixture-udid", "com.fixture.app")
            .await
            .expect_err("legacy best-effort payload must not prove termination");

        assert!(
            error
                .to_string()
                .contains("sidecar process response must contain exactly"),
            "unexpected protocol error: {error:#}"
        );
        let args: Vec<String> = serde_json::from_slice(
            &std::fs::read(&fixture.args_path).expect("read fixture sidecar argv"),
        )
        .expect("decode fixture sidecar argv");
        assert_eq!(
            args,
            [
                "terminate",
                "--udid",
                "fixture-udid",
                "--bundle-id",
                "com.fixture.app",
            ]
        );
    }

    #[tokio::test]
    async fn verified_process_control_requires_a_versioned_ready_ping_handshake() {
        let cases = [
            (
                serde_json::json!({
                    "ok": true,
                    "pymobiledevice3": true,
                    "sidecarProtocolVersion": 2,
                    "contracts": ["verifiedProcessControl"],
                }),
                0,
                true,
            ),
            (
                serde_json::json!({ "ok": true, "pymobiledevice3": true }),
                0,
                false,
            ),
            (
                serde_json::json!({
                    "ok": true,
                    "pymobiledevice3": true,
                    "sidecarProtocolVersion": 1,
                    "contracts": ["verifiedProcessControl"],
                }),
                0,
                false,
            ),
            (
                serde_json::json!({
                    "ok": true,
                    "pymobiledevice3": true,
                    "sidecarProtocolVersion": 2,
                    "contracts": [],
                }),
                0,
                false,
            ),
            (
                serde_json::json!({
                    "ok": true,
                    "pymobiledevice3": false,
                    "sidecarProtocolVersion": 2,
                    "contracts": [],
                }),
                2,
                false,
            ),
        ];

        for (ping, exit_code, expected) in cases {
            let fixture = LegacyTerminateSidecarFixture::new(ping, exit_code);
            let driver = PmdIosDriver::probe(&DriverConfig {
                sidecar_root: fixture.root.clone(),
                state_dir: fixture.root.join("state"),
                target: DriverTarget::LegacyStock,
            })
            .await
            .expect("probe fixture sidecar");
            assert_eq!(driver.supports_verified_app_termination(), expected);
        }
    }

    #[test]
    fn app_process_protocol_requires_exact_consistent_state() {
        let running = parse_app_process_state(
            serde_json::json!({
                "ok": true,
                "bundleId": "com.fixture.app",
                "pid": 42,
                "running": true,
            }),
            "com.fixture.app",
        )
        .expect("running process state");
        assert_eq!(running.bundle_id, "com.fixture.app");
        assert_eq!(running.pid, Some(42));
        assert!(running.running);

        let absent = parse_app_process_state(
            serde_json::json!({
                "ok": true,
                "bundleId": "com.fixture.app",
                "pid": null,
                "running": false,
            }),
            "com.fixture.app",
        )
        .expect("absent process state");
        assert_eq!(absent.pid, None);
        assert!(!absent.running);

        for invalid in [
            serde_json::json!({
                "ok": true,
                "bundleId": "com.fixture.app",
                "running": false,
            }),
            serde_json::json!({
                "ok": true,
                "bundleId": "com.fixture.app",
                "pid": 0,
                "running": true,
            }),
            serde_json::json!({
                "ok": true,
                "bundleId": "com.fixture.app",
                "pid": null,
                "running": true,
            }),
            serde_json::json!({
                "ok": true,
                "bundleId": "com.fixture.app",
                "pid": 42,
                "running": false,
            }),
            serde_json::json!({
                "ok": false,
                "bundleId": "com.fixture.app",
                "pid": null,
                "running": false,
            }),
            serde_json::json!({
                "ok": true,
                "bundleId": "com.fixture.app",
                "pid": null,
                "running": false,
                "extra": true,
            }),
        ] {
            assert!(
                parse_app_process_state(invalid, "com.fixture.app").is_err(),
                "invalid process inspection payload must be rejected"
            );
        }
    }

    #[tokio::test]
    async fn interaction_http_contract_target_background_safe_never_foregrounds_agent() {
        let mut runtime = fixture_clipboard_runtime();
        runtime.identities = VecDeque::from([
            ActiveAppIdentity {
                bundle_id: INTERACTION_TARGET_BUNDLE_ID.to_string(),
                pid: 902,
            },
            ActiveAppIdentity {
                bundle_id: INTERACTION_TARGET_BUNDLE_ID.to_string(),
                pid: 902,
            },
        ]);

        let progress = GuardedClipboardProgress::default();
        let result = run_guarded_clipboard_transition(
            &mut runtime,
            &progress,
            ClipboardAccessMode::TargetBackgroundSafe,
            "com.fixture.agent",
            INTERACTION_TARGET_BUNDLE_ID,
            InteractionSessionKind::Ordinary,
            GuardedClipboardOperation::Get {
                maximum_decoded_bytes: 65_536,
            },
        )
        .await
        .expect("background-safe clipboard");

        assert_eq!(
            runtime.calls,
            vec!["probe-active", "clipboard", "probe-active"]
        );
        assert!(matches!(
            result.output,
            GuardedClipboardOutput::Value { ref bytes, .. } if bytes == b"fixture"
        ));
        assert!(result.proof.stop.is_none());
        assert!(result.proof.agent.is_none());
        assert_eq!(result.proof.target.as_ref().unwrap().pid, 902);
        assert!(result.proof.stream.is_none());
    }

    #[tokio::test]
    async fn interaction_http_contract_agent_foreground_transition_proves_full_lifecycle() {
        let mut runtime = fixture_clipboard_runtime();
        let progress = GuardedClipboardProgress::default();

        let result = run_guarded_clipboard_transition(
            &mut runtime,
            &progress,
            ClipboardAccessMode::AgentForegroundRequired,
            "com.fixture.agent",
            INTERACTION_TARGET_BUNDLE_ID,
            InteractionSessionKind::FreshText,
            GuardedClipboardOperation::Set {
                content_type: "plaintext".to_string(),
                bytes: b"fixture".to_vec(),
            },
        )
        .await
        .expect("guarded foreground clipboard");

        assert_eq!(
            runtime.calls,
            vec![
                "stop",
                "foreground-agent",
                "probe-active",
                "probe-active",
                "clipboard",
                "probe-active",
                "foreground-target",
                "probe-active",
                "final-session",
                "probe-active",
                "start-stream",
            ]
        );
        assert_eq!(result.proof.stop.as_ref().unwrap().new_generation, 10);
        assert!(matches!(result.output, GuardedClipboardOutput::Written));
        assert_eq!(result.proof.agent.as_ref().unwrap().pid, 701);
        assert_eq!(result.proof.target.as_ref().unwrap().pid, 902);
        assert_eq!(result.proof.stream.as_ref().unwrap().generation, 10);
    }

    #[tokio::test]
    async fn interaction_http_contract_agent_foreground_transition_rejects_pid_and_generation_drift(
    ) {
        let mut pid_drift = fixture_clipboard_runtime();
        pid_drift.identities[1].pid = 702;
        let progress = GuardedClipboardProgress::default();
        let error = run_guarded_clipboard_transition(
            &mut pid_drift,
            &progress,
            ClipboardAccessMode::AgentForegroundRequired,
            "com.fixture.agent",
            INTERACTION_TARGET_BUNDLE_ID,
            InteractionSessionKind::Ordinary,
            GuardedClipboardOperation::Get {
                maximum_decoded_bytes: 65_536,
            },
        )
        .await
        .expect_err("Agent PID drift must fail closed");
        assert!(error.to_string().contains("Agent PID"));

        let mut target_drift = fixture_clipboard_runtime();
        target_drift.identities[4].pid = 903;
        let progress = GuardedClipboardProgress::default();
        let error = run_guarded_clipboard_transition(
            &mut target_drift,
            &progress,
            ClipboardAccessMode::AgentForegroundRequired,
            "com.fixture.agent",
            INTERACTION_TARGET_BUNDLE_ID,
            InteractionSessionKind::Ordinary,
            GuardedClipboardOperation::Get {
                maximum_decoded_bytes: 65_536,
            },
        )
        .await
        .expect_err("target PID drift must fail closed");
        assert!(error.to_string().contains("target PID"));

        let mut generation_drift = fixture_clipboard_runtime();
        generation_drift.start_proof.generation = 11;
        let progress = GuardedClipboardProgress::default();
        let error = run_guarded_clipboard_transition(
            &mut generation_drift,
            &progress,
            ClipboardAccessMode::AgentForegroundRequired,
            "com.fixture.agent",
            INTERACTION_TARGET_BUNDLE_ID,
            InteractionSessionKind::Ordinary,
            GuardedClipboardOperation::Get {
                maximum_decoded_bytes: 65_536,
            },
        )
        .await
        .expect_err("stream generation drift must fail closed");
        assert!(error.to_string().contains("generation"));
    }

    fn driver_config(target: DriverTarget) -> DriverConfig {
        DriverConfig {
            sidecar_root: PathBuf::new(),
            state_dir: std::env::temp_dir().join("riviu-driver-config-test"),
            target,
        }
    }

    fn stock_driver() -> PmdIosDriver {
        PmdIosDriver::degraded(&driver_config(DriverTarget::LegacyStock))
            .expect("degraded stock driver")
    }

    #[tokio::test]
    async fn hard_recycle_preserves_the_generation_reserved_by_the_first_stop() {
        let driver = stock_driver();

        let first = DeviceDriver::stop_owned_stream(&driver, "fixture-udid")
            .await
            .expect("first stop proof");
        DeviceDriver::recycle_ui_transport(&driver, "fixture-udid").await;
        let second = DeviceDriver::stop_owned_stream(&driver, "fixture-udid")
            .await
            .expect("post-recycle stop proof");

        assert_eq!((first.old_generation, first.new_generation), (0, 1));
        assert_eq!((second.old_generation, second.new_generation), (1, 2));
    }

    #[tokio::test]
    async fn negotiated_ui_is_attached_to_the_next_device_profile() {
        let driver = stock_driver();
        let capabilities = UiCapabilities {
            open_url: Some(OpenUrlCapability {
                route: ProtectedRouteContract {
                    contract_id: "fixture-open-url-v1".to_string(),
                    method: RouteMethod::Post,
                    scope: RouteScope::Sessionless,
                    path: "/fixture/url".to_string(),
                    auth_header_name: "X-Fixture-Token".to_string(),
                    body_schema_id: "open-url-body-v1".to_string(),
                    request_timeout_ms: 10_000,
                },
                target_bundle_id: INTERACTION_TARGET_BUNDLE_ID.to_string(),
                live_report_sha256: "aa".repeat(32),
            }),
            ..UiCapabilities::default()
        };

        DeviceDriver::set_negotiated_interaction_capabilities(
            &driver,
            "fixture-udid",
            capabilities,
        )
        .await
        .expect("attach exact negotiated UI");

        let profile = driver.profile_for("fixture-udid").expect("device profile");
        assert!(profile
            .configured_interaction_capabilities()
            .open_url
            .is_some());
        assert!(driver
            .profile_for("other-udid")
            .expect("deny-all profile")
            .configured_interaction_capabilities()
            .open_url
            .is_none());
    }

    #[test]
    fn degraded_driver_keeps_unified_profile_instead_of_stock() {
        let artifact = AgentArtifact::load(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../sidecars/wda/agent-manifest.json"),
        )
        .expect("load bundled agent");
        let expected_ipa = artifact.ipa_path.clone();
        let config = driver_config(DriverTarget::Real(UnifiedAgentConfig {
            token: AgentToken::new("fixture-token").unwrap(),
            artifact,
            settings: AgentSettings::default(),
        }));

        let driver = PmdIosDriver::degraded(&config).expect("degraded unified driver");

        assert_eq!(driver.profile.backend, WdaBackend::RtMmo);
        assert_eq!(driver.profile.bundle_id, "com.mrph.svc");
        assert_eq!(driver.profile.device_port, 8906);
        assert_eq!(driver.profile.mjpeg_port, 9093);
        assert_eq!(driver.profile.agent_ipa.as_ref(), Some(&expected_ipa));
        assert!(driver
            .profile
            .features
            .iter()
            .any(|feature| feature == "text"));
    }

    #[test]
    fn lifecycle_only_fresh_unified_sessions_advertise_trusted_text() {
        let profile = WdaProfile::rt_mmo("fixture-token".to_string());
        let ordinary = PmdUiSession {
            client: WdaClient::new_with_profile("127.0.0.1", 18100, "fixture", profile.clone()),
            mjpeg_url: String::new(),
            supports_text_input: false,
            supports_accessibility_readback: false,
            target_bundle_id: INTERACTION_TARGET_BUNDLE_ID.to_string(),
        };
        let fresh = PmdUiSession {
            client: WdaClient::new_with_profile("127.0.0.1", 18100, "fixture", profile),
            mjpeg_url: String::new(),
            supports_text_input: true,
            supports_accessibility_readback: true,
            target_bundle_id: INTERACTION_TARGET_BUNDLE_ID.to_string(),
        };

        assert!(!ordinary.supports_text_input());
        assert!(!ordinary.supports_accessibility_readback());
        assert!(fresh.supports_text_input());
        assert!(fresh.supports_accessibility_readback());
    }

    #[test]
    fn a_running_stream_is_reused_without_another_session_probe() {
        assert!(!session_attach_required(true));
        assert!(session_attach_required(false));
    }

    #[test]
    fn rt_mmo_proxy_args_select_one_agent_and_all_required_ports() {
        let profile = WdaProfile::rt_mmo("test-token".to_string());
        let args = proxy_args(&profile, "udid-a", 18100, false);

        assert_eq!(
            args,
            vec![
                "wda-proxy",
                "--udid",
                "udid-a",
                "--local-port",
                "18100",
                "--backend",
                "rt-mmo",
                "--device-port",
                "8906",
                "--mjpeg-port",
                "9093",
                "--bundle-id",
                "com.mrph.svc",
            ]
        );
        assert!(!args.iter().any(|arg| arg == "8100"));
        assert!(
            !args.iter().any(|arg| arg == "test-token"),
            "the token must be injected through the child environment, not argv"
        );
    }

    #[test]
    fn rt_mmo_stream_args_use_the_same_control_and_mjpeg_profile() {
        let profile = WdaProfile::rt_mmo("test-token".to_string());
        let args = stream_args(&profile, "udid-a");

        assert!(args.windows(2).any(|v| v == ["--wda-port", "8906"]));
        assert!(args.windows(2).any(|v| v == ["--mjpeg-port", "9093"]));
        assert!(args.windows(2).any(|v| v == ["--mode", "mjpeg"]));
        assert!(args
            .windows(2)
            .any(|v| v == ["--wda-bundle", "com.mrph.svc"]));
    }

    #[test]
    fn rt_mmo_text_bootstrap_restarts_agent_without_putting_token_in_argv() {
        let profile = WdaProfile::rt_mmo("test-token".to_string());
        let args = text_bootstrap_args(&profile, "udid-a", 18100);

        assert!(args.iter().any(|arg| arg == "--restart-wda"));
        assert!(args.iter().any(|arg| arg == "--bootstrap-only"));
        assert!(
            !args.iter().any(|arg| arg == "test-token"),
            "the token must only be passed through the child environment"
        );
    }

    #[test]
    fn rt_mmo_parent_waits_for_both_bounded_launch_attempts() {
        let profile = WdaProfile::rt_mmo("test-token".to_string());

        assert!(proxy_ready_window(&profile, false) >= Duration::from_secs(170));
        assert!(proxy_ready_window(&profile, true) >= Duration::from_secs(170));
        assert_eq!(
            proxy_ready_window(&WdaProfile::stock(), false),
            Duration::from_secs(55)
        );
    }

    #[test]
    fn each_device_gets_its_own_sticky_control_port() {
        let driver = stock_driver();
        let a = driver.port_for("udid-a");
        let b = driver.port_for("udid-b");
        assert_ne!(a, b, "two devices must not share a relay port");
        assert_eq!(a, driver.port_for("udid-a"), "assignments are sticky");
        assert!((WDA_LOCAL_PORT_BASE..WDA_LOCAL_PORT_BASE + WDA_LOCAL_PORT_SPAN).contains(&a));
        assert!((WDA_LOCAL_PORT_BASE..WDA_LOCAL_PORT_BASE + WDA_LOCAL_PORT_SPAN).contains(&b));
    }

    #[test]
    fn interaction_inspection_args_are_read_only() {
        let args = interaction_inspection_args(
            "fixture-udid",
            "com.apple.Preferences",
            "com.mrph.svc",
            InteractionInspectionTransport::LegacyUsbmux,
        );

        assert_eq!(
            args,
            vec![
                "inspect-device-capabilities",
                "--udid",
                "fixture-udid",
                "--target-bundle-id",
                "com.apple.Preferences",
                "--agent-bundle-id",
                "com.mrph.svc",
            ]
        );
        for forbidden in [
            "install",
            "uninstall",
            "launch",
            "terminate",
            "wda-proxy",
            "start-wda",
            "stream",
            "session",
        ] {
            assert!(!args.iter().any(|arg| arg == forbidden));
        }
    }

    #[test]
    fn interaction_inspection_args_select_rsd_as_one_typed_transport() {
        let args = interaction_inspection_args(
            "fixture-udid",
            "com.ss.iphone.ugc.Ame",
            "com.mrph.svc",
            InteractionInspectionTransport::Rsd {
                host: "fd00::1",
                port: 58783,
            },
        );

        assert!(args
            .windows(2)
            .any(|pair| pair == ["--rsd-host", "fd00::1"]));
        assert!(args.windows(2).any(|pair| pair == ["--rsd-port", "58783"]));
    }

    #[test]
    fn interaction_inspection_parses_identity_without_claiming_runtime_proof() {
        let artifact = AgentArtifact::load(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../sidecars/wda/agent-manifest.json"),
        )
        .expect("load bundled agent");
        let value = serde_json::json!({
            "ok": true,
            "udid": "fixture-udid",
            "productType": "iPhone10,1",
            "iosVersion": "16.7.15",
            "transport": "legacyUsbmuxTransport",
            "targetApp": {
                "bundleId": "com.ss.iphone.ugc.Ame",
                "version": "35.0.0",
                "build": "350001"
            },
            "agentApp": {
                "bundleId": "com.mrph.svc",
                "version": "1.0",
                "build": "1",
                "executableName": "FixtureRunner",
                "signerIdentity": "iPhone Distribution: Fixture"
            }
        });

        let snapshot = parse_interaction_inspection(
            value,
            "fixture-udid",
            "com.ss.iphone.ugc.Ame",
            &artifact,
            ActiveTransport::LegacyUsbmuxTransport,
        )
        .expect("parse capability inspection");

        assert_eq!(snapshot.transport, ActiveTransport::LegacyUsbmuxTransport);
        assert_eq!(snapshot.product_type, "iPhone10,1");
        assert_eq!(snapshot.target_app.version, "35.0.0");
        assert_eq!(snapshot.installed_agent.executable_name, "FixtureRunner");
        assert_eq!(snapshot.installed_agent.signer_identity_sha256.len(), 64);
        assert!(!snapshot.protected_auth_ready);
        assert_eq!(snapshot.geometry, None);
    }

    #[test]
    fn interaction_inspection_rejects_a_different_provider_udid() {
        let artifact = AgentArtifact::load(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../sidecars/wda/agent-manifest.json"),
        )
        .expect("load bundled agent");
        let value = serde_json::json!({
            "ok": true,
            "udid": "different-udid",
            "productType": "iPhone10,1",
            "iosVersion": "16.7.15",
            "transport": "legacyUsbmuxTransport",
            "targetApp": null,
            "agentApp": null
        });

        let error = parse_interaction_inspection(
            value,
            "fixture-udid",
            "com.ss.iphone.ugc.Ame",
            &artifact,
            ActiveTransport::LegacyUsbmuxTransport,
        )
        .expect_err("provider identity drift must fail closed");

        assert!(error.to_string().contains("different UDID"));
    }

    #[test]
    fn interaction_inspection_rejects_missing_installed_identities() {
        let artifact = AgentArtifact::load(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../sidecars/wda/agent-manifest.json"),
        )
        .expect("load bundled agent");
        let base = serde_json::json!({
            "ok": true,
            "udid": "fixture-udid",
            "productType": "iPhone10,1",
            "iosVersion": "16.7.15",
            "transport": "legacyUsbmuxTransport",
            "targetApp": {
                "bundleId": "com.ss.iphone.ugc.Ame",
                "version": "35.0.0",
                "build": "350001"
            },
            "agentApp": {
                "bundleId": "com.mrph.svc",
                "version": "1.0",
                "build": "1",
                "executableName": "FixtureRunner",
                "signerIdentity": "iPhone Distribution: Fixture"
            }
        });

        for (field, expected) in [
            ("targetApp", "target app is not installed"),
            ("agentApp", "selected Agent is not installed"),
        ] {
            let mut value = base.clone();
            value[field] = serde_json::Value::Null;
            let error = parse_interaction_inspection(
                value,
                "fixture-udid",
                "com.ss.iphone.ugc.Ame",
                &artifact,
                ActiveTransport::LegacyUsbmuxTransport,
            )
            .expect_err("missing installed identity must fail closed");
            assert!(error.to_string().contains(expected));
        }
    }

    #[tokio::test]
    async fn interaction_inspection_rehashes_the_artifact_before_sidecar_io() {
        let mut artifact = AgentArtifact::load(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../sidecars/wda/agent-manifest.json"),
        )
        .expect("load bundled agent");
        artifact.manifest.sha256 = "00".repeat(32);
        let driver =
            PmdIosDriver::degraded(&driver_config(DriverTarget::Real(UnifiedAgentConfig {
                token: AgentToken::new("fixture-token").unwrap(),
                artifact,
                settings: AgentSettings::default(),
            })))
            .expect("degraded driver");

        let error = driver
            .inspect_interaction_device("fixture-udid")
            .await
            .expect_err("tampered artifact must stop inspection before sidecar IO");

        assert!(error.to_string().to_ascii_lowercase().contains("integrity"));
        assert!(!error.to_string().contains("sidecar not configured"));
    }

    #[tokio::test]
    async fn install_only_repair_rehashes_the_artifact_before_sidecar_io() {
        let mut artifact = AgentArtifact::load(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../sidecars/wda/agent-manifest.json"),
        )
        .expect("load bundled agent");
        artifact.manifest.sha256 = "00".repeat(32);
        let driver =
            PmdIosDriver::degraded(&driver_config(DriverTarget::Real(UnifiedAgentConfig {
                token: AgentToken::new("fixture-token").unwrap(),
                artifact,
                settings: AgentSettings::default(),
            })))
            .expect("degraded driver");

        let error = driver
            .repair_agent_install_only("fixture-udid")
            .await
            .expect_err("tampered artifact must stop before sidecar IO");

        assert!(error.to_string().to_ascii_lowercase().contains("integrity"));
        assert!(!error.to_string().contains("sidecar not configured"));
        let status = driver.cached_agent_status("fixture-udid");
        assert_eq!(status.state, AgentState::Error);
        assert!(!status.auth_ready);
        assert!(!status.session_ready);
        assert!(!status.mjpeg_ready);
    }

    #[tokio::test]
    async fn interaction_lifecycle_fails_closed_without_session_or_replacement_stream() {
        let driver = stock_driver();

        let error = driver
            .start_stream_after_session("fixture-udid")
            .await
            .expect_err("stream-before-session must fail before sidecar IO");
        assert!(error.to_string().contains("session reservation"));

        let proof = driver
            .stop_owned_stream("fixture-udid")
            .await
            .expect("explicit stop");
        assert!(proof.child_stopped);
        assert!(proof.new_generation > proof.old_generation);
        let error = match driver
            .start_interaction_session(
                "fixture-udid",
                INTERACTION_TARGET_BUNDLE_ID,
                InteractionSessionKind::Ordinary,
            )
            .await
        {
            Ok(_) => panic!("degraded interaction session unexpectedly succeeded"),
            Err(error) => error,
        };
        assert!(error
            .to_string()
            .contains("install-only protected control relay"));
        assert!(!driver.sessions.lock().contains_key("fixture-udid"));
        assert!(!driver
            .interaction_lifecycle
            .has_session_reservation("fixture-udid"));
        assert!(driver
            .slots
            .get("fixture-udid")
            .owned
            .lock()
            .await
            .stream
            .is_none());
    }

    #[tokio::test]
    async fn interaction_stream_handoff_records_the_current_generation_without_mutation() {
        let driver = stock_driver();
        let generation = driver.streams.generation("fixture-udid");

        driver
            .confirm_interaction_stream_stopped("fixture-udid")
            .await
            .expect("non-destructive stream handoff");

        assert_eq!(driver.streams.generation("fixture-udid"), generation);
        assert!(driver.sessions.lock().get("fixture-udid").is_none());
        assert!(driver
            .slots
            .get("fixture-udid")
            .owned
            .lock()
            .await
            .stream
            .is_none());
        assert!(driver
            .interaction_lifecycle
            .begin_session("fixture-udid", generation, InteractionSessionKind::Ordinary,)
            .is_ok());
    }

    #[tokio::test]
    async fn interaction_stream_handoff_rejects_a_cached_session() {
        let driver = stock_driver();
        driver.sessions.lock().insert(
            "fixture-udid".to_string(),
            WdaClient::new_with_profile("127.0.0.1", 18_100, "fixture-udid", WdaProfile::stock()),
        );

        let error = driver
            .confirm_interaction_stream_stopped("fixture-udid")
            .await
            .expect_err("cached sessions must be closed before handoff");

        assert!(error.to_string().contains("cached session"));
        assert!(driver
            .interaction_lifecycle
            .begin_session(
                "fixture-udid",
                driver.streams.generation("fixture-udid"),
                InteractionSessionKind::Ordinary,
            )
            .is_err());
    }

    #[tokio::test]
    async fn combined_interaction_fresh_text_rejects_stock_without_foreground_side_effects() {
        let driver = stock_driver();
        driver
            .stop_owned_stream("fixture-udid")
            .await
            .expect("explicit stop");

        let error = match driver
            .foreground_target_app_and_start_interaction_session(
                "fixture-udid",
                INTERACTION_TARGET_BUNDLE_ID,
                InteractionSessionKind::FreshText,
            )
            .await
        {
            Ok(_) => panic!("stock fresh-text session unexpectedly succeeded"),
            Err(error) => error,
        };

        assert!(error.to_string().contains("fresh-text"));
        assert!(!driver.sessions.lock().contains_key("fixture-udid"));
    }

    #[tokio::test]
    async fn interaction_stop_downgrades_cached_ready_status() {
        let driver = stock_driver();
        driver.publish_status(driver.status(
            "fixture-udid",
            AgentState::Ready,
            None,
            (true, true, true),
            None,
        ));

        driver
            .stop_owned_stream("fixture-udid")
            .await
            .expect("explicit stop");

        let status = driver.cached_agent_status("fixture-udid");
        assert_eq!(status.state, AgentState::Starting);
        assert!(status.auth_ready);
        assert!(!status.session_ready);
        assert!(!status.mjpeg_ready);
    }

    #[test]
    fn install_only_requires_the_owned_stream_and_cached_session_to_be_stopped() {
        assert!(ensure_install_only_runtime_is_idle(false, false).is_ok());
        for (has_stream, has_session) in [(true, false), (false, true), (true, true)] {
            let error = ensure_install_only_runtime_is_idle(has_stream, has_session)
                .expect_err("active UI runtime must be stopped through the control plane first");
            assert!(error.to_string().contains("stop_owned_stream"));
        }
    }

    #[test]
    fn install_only_launch_keeps_token_environment_only_and_fingerprint_scoped() {
        let token = "fixture-install-only-secret";
        let profile = WdaProfile::rt_mmo(token.to_string());
        let args = proxy_args(&profile, "fixture-udid", 18_100, false);

        assert_eq!(RTMMO_TOKEN_ENV, "RIVIU_RTMMO_TOKEN");
        assert!(!args.iter().any(|argument| argument.contains(token)));
        assert_eq!(
            proxy_process_fingerprint("fixture-udid"),
            "wda-proxy --udid fixture-udid"
        );
        assert!(!proxy_process_fingerprint("fixture-udid").contains(token));
    }

    #[test]
    fn relay_event_detail_is_redacted_before_the_telemetry_sink() {
        let token = "fixture-install-only-secret";
        let artifact = AgentArtifact::load(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../sidecars/wda/agent-manifest.json"),
        )
        .expect("load bundled agent");
        let driver =
            PmdIosDriver::degraded(&driver_config(DriverTarget::Real(UnifiedAgentConfig {
                token: AgentToken::new(token).unwrap(),
                artifact,
                settings: AgentSettings::default(),
            })))
            .expect("degraded driver");
        let result: anyhow::Result<()> = Err(anyhow::anyhow!("spawn leaked {token}"));

        let detail = driver.secret_free_result_detail(&result);

        assert!(!detail.contains(token));
        assert!(detail.contains("[REDACTED]"));
    }

    /// Two logical jobs on one device must serialise on the slot lock rather
    /// than both proceeding to spawn a relay.
    #[tokio::test]
    async fn a_second_job_for_the_same_device_queues() {
        let driver = stock_driver();
        let slot = driver.slots.get("udid-a");
        let held = slot.owned.lock().await;
        assert!(driver.slots.get("udid-a").owned.try_lock().is_err());
        assert!(driver.slots.get("udid-b").owned.try_lock().is_ok());
        drop(held);
        assert!(driver.slots.get("udid-a").owned.try_lock().is_ok());
    }
}
