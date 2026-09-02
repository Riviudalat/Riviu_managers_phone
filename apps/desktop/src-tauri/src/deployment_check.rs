//! Installed-build diagnostics for a clean Windows host.
//!
//! This module deliberately avoids Tauri state. The companion console binary must be able to
//! prove the package before the GUI starts, and it must return a stable JSON report and exit
//! status to an unattended acceptance harness.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, Context};
use riviu_android_driver::adb::{parse_devices, AdbDeviceState};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::android_tools::{AndroidTools, SidecarOrigin};

pub const REPORT_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckStatus {
    Pass,
    Warning,
    Fail,
    NotApplicable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DeploymentProfile {
    Internal,
    Production,
}

impl DeploymentProfile {
    fn parse(value: &str) -> anyhow::Result<Self> {
        match value {
            "internal" => Ok(Self::Internal),
            "production" => Ok(Self::Production),
            _ => Err(anyhow!(
                "--profile must be `internal` or `production`, got {value:?}"
            )),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckResult {
    pub status: CheckStatus,
    pub detail: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

impl CheckResult {
    fn new(status: CheckStatus, detail: impl Into<String>) -> Self {
        Self {
            status,
            detail: detail.into(),
            data: None,
        }
    }

    fn with_data(mut self, data: Value) -> Self {
        self.data = Some(data);
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeploymentChecks {
    pub operating_system: CheckResult,
    pub architecture: CheckResult,
    pub authenticode: CheckResult,
    pub web_view2: CheckResult,
    pub resource_hashes: CheckResult,
    pub database_migration: CheckResult,
    pub credential_manager: CheckResult,
    pub environment_overrides: CheckResult,
    pub adb_version: CheckResult,
    pub android_package_tools: CheckResult,
    pub device_state: CheckResult,
}

impl DeploymentChecks {
    fn values(&self) -> [&CheckResult; 11] {
        [
            &self.operating_system,
            &self.architecture,
            &self.authenticode,
            &self.web_view2,
            &self.resource_hashes,
            &self.database_migration,
            &self.credential_manager,
            &self.environment_overrides,
            &self.adb_version,
            &self.android_package_tools,
            &self.device_state,
        ]
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeploymentReport {
    pub schema_version: u32,
    pub generated_at_unix_ms: u128,
    pub profile: DeploymentProfile,
    pub host: HostInfo,
    pub app_version: String,
    pub install_dir: PathBuf,
    pub app_sha256: String,
    pub checker_sha256: String,
    pub installer_path: Option<PathBuf>,
    pub installer_sha256: Option<String>,
    pub adb_origin: Option<String>,
    pub checks: DeploymentChecks,
    pub overall: CheckStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostInfo {
    pub name: String,
    pub version: String,
    pub build: String,
    pub architecture: String,
}

impl DeploymentReport {
    pub fn overall_for(&self, profile: DeploymentProfile) -> CheckStatus {
        if profile == DeploymentProfile::Production
            && (self.checks.authenticode.status != CheckStatus::Pass
                || self.installer_path.is_none()
                || self.installer_sha256.is_none())
        {
            return CheckStatus::Fail;
        }
        if self
            .checks
            .values()
            .iter()
            .any(|check| check.status == CheckStatus::Fail)
        {
            return CheckStatus::Fail;
        }
        if self
            .checks
            .values()
            .iter()
            .any(|check| check.status == CheckStatus::Warning)
        {
            return CheckStatus::Warning;
        }
        CheckStatus::Pass
    }

    #[doc(hidden)]
    pub fn fixture_with_authenticode(status: CheckStatus) -> Self {
        let pass = || CheckResult::new(CheckStatus::Pass, "fixture pass");
        Self {
            schema_version: REPORT_SCHEMA_VERSION,
            generated_at_unix_ms: 0,
            profile: DeploymentProfile::Internal,
            host: HostInfo {
                name: "Windows".into(),
                version: "10.0.22631.0".into(),
                build: "22631".into(),
                architecture: "x86_64".into(),
            },
            app_version: env!("CARGO_PKG_VERSION").to_string(),
            install_dir: PathBuf::from(r"C:\fixture"),
            app_sha256: "0".repeat(64),
            checker_sha256: "0".repeat(64),
            installer_path: Some(PathBuf::from(r"C:\fixture\Riviu.msi")),
            installer_sha256: Some("0".repeat(64)),
            adb_origin: Some("Bundled".into()),
            checks: DeploymentChecks {
                operating_system: pass(),
                architecture: pass(),
                authenticode: CheckResult::new(status, "fixture signature"),
                web_view2: pass(),
                resource_hashes: pass(),
                database_migration: pass(),
                credential_manager: pass(),
                environment_overrides: CheckResult::new(CheckStatus::NotApplicable, "fixture"),
                adb_version: pass(),
                android_package_tools: pass(),
                device_state: CheckResult::new(CheckStatus::NotApplicable, "fixture"),
            },
            overall: CheckStatus::Pass,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeploymentArgs {
    pub profile: DeploymentProfile,
    pub report: PathBuf,
    pub device_check: bool,
    pub device_serial: Option<String>,
    pub installer: Option<PathBuf>,
}

pub fn parse_args<I, T>(args: I) -> anyhow::Result<DeploymentArgs>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString>,
{
    let mut args = args.into_iter().map(Into::into);
    let _program = args.next();
    let mut profile = None;
    let mut report = None;
    let mut installer = None;
    let mut device_check = false;
    let mut device_serial = None;
    let pending: Vec<OsString> = args.collect();
    let mut index = 0;
    while index < pending.len() {
        let flag = pending[index]
            .to_str()
            .ok_or_else(|| anyhow!("command-line arguments must be valid Unicode"))?;
        match flag {
            "--profile" | "--report" | "--installer" => {
                let value = pending
                    .get(index + 1)
                    .and_then(|value| value.to_str())
                    .filter(|value| !value.starts_with("--"))
                    .ok_or_else(|| anyhow!("{flag} requires a value"))?;
                match flag {
                    "--profile" => profile = Some(DeploymentProfile::parse(value)?),
                    "--report" => report = Some(PathBuf::from(value)),
                    "--installer" => installer = Some(PathBuf::from(value)),
                    _ => unreachable!(),
                }
                index += 2;
            }
            "--device-check" => {
                device_check = true;
                if let Some(value) = pending.get(index + 1).and_then(|value| value.to_str()) {
                    if !value.starts_with("--") {
                        device_serial = Some(value.to_string());
                        index += 1;
                    }
                }
                index += 1;
            }
            "--help" | "-h" => {
                return Err(anyhow!(usage()));
            }
            _ => return Err(anyhow!("unknown argument {flag:?}\n{}", usage())),
        }
    }
    let profile = profile.ok_or_else(|| anyhow!("--profile is required"))?;
    if profile == DeploymentProfile::Production && installer.is_none() {
        return Err(anyhow!(
            "--installer is required for the production profile"
        ));
    }
    Ok(DeploymentArgs {
        profile,
        report: report.ok_or_else(|| anyhow!("--report is required"))?,
        device_check,
        device_serial,
        installer,
    })
}

pub fn usage() -> &'static str {
    "Usage: riviu-deployment-check.exe --profile internal|production --report <path> [--installer <path>] [--device-check [serial]]"
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeploymentSmokeArgs {
    pub report: PathBuf,
    pub data_dir: PathBuf,
}

pub fn write_startup_smoke_report(
    report: &Path,
    data_dir: &Path,
    database_version: i64,
) -> anyhow::Result<()> {
    if let Some(parent) = report.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let temporary = report.with_extension("json.tmp");
    std::fs::write(
        &temporary,
        serde_json::to_vec_pretty(&json!({
            "schemaVersion": 1,
            "status": "ready",
            "mode": "mock",
            "tauriReady": true,
            "frontendReady": true,
            "databaseVersion": database_version,
            "dataDir": data_dir,
        }))?,
    )
    .with_context(|| format!("write startup smoke report {}", temporary.display()))?;
    std::fs::rename(&temporary, report)
        .with_context(|| format!("publish startup smoke report {}", report.display()))?;
    Ok(())
}

pub fn read_startup_smoke_report(report: &Path) -> anyhow::Result<Value> {
    let bytes = std::fs::read(report)
        .with_context(|| format!("read startup smoke report {}", report.display()))?;
    serde_json::from_slice(&bytes)
        .with_context(|| format!("parse startup smoke report {}", report.display()))
}

pub fn parse_deployment_smoke_args<I, T>(args: I) -> anyhow::Result<Option<DeploymentSmokeArgs>>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString>,
{
    let values: Vec<OsString> = args.into_iter().map(Into::into).collect();
    let Some(smoke_index) = values
        .iter()
        .position(|value| value == "--deployment-smoke")
    else {
        return Ok(None);
    };
    let report = values
        .get(smoke_index + 1)
        .filter(|value| value.to_str().is_some_and(|value| !value.starts_with("--")))
        .map(PathBuf::from)
        .ok_or_else(|| anyhow!("--deployment-smoke requires a report path"))?;
    let data_index = values
        .iter()
        .position(|value| value == "--data-dir")
        .ok_or_else(|| anyhow!("--deployment-smoke requires --data-dir"))?;
    let data_dir = values
        .get(data_index + 1)
        .filter(|value| value.to_str().is_some_and(|value| !value.starts_with("--")))
        .map(PathBuf::from)
        .ok_or_else(|| anyhow!("--data-dir requires a path"))?;
    if !report.is_absolute() || !data_dir.is_absolute() {
        return Err(anyhow!(
            "deployment smoke report and data directory must be absolute paths"
        ));
    }
    Ok(Some(DeploymentSmokeArgs { report, data_dir }))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallLayout {
    pub install_dir: PathBuf,
    pub sidecars_root: PathBuf,
    pub adb: PathBuf,
    pub main_executable: PathBuf,
}

pub fn resolve_install_layout(checker_executable: &Path) -> anyhow::Result<InstallLayout> {
    let install_dir = checker_executable
        .parent()
        .context("deployment checker has no parent install directory")?
        .to_path_buf();
    let sidecars_root = install_dir.join("sidecars");
    Ok(InstallLayout {
        adb: sidecars_root
            .join("android")
            .join("win-x86_64")
            .join("adb.exe"),
        sidecars_root,
        main_executable: install_dir.join("riviu-managers-phone.exe"),
        install_dir,
    })
}

pub fn exit_code_for(report: &DeploymentReport, profile: DeploymentProfile) -> i32 {
    if report.overall_for(profile) == CheckStatus::Fail {
        2
    } else {
        0
    }
}

pub fn run(args: &DeploymentArgs) -> anyhow::Result<DeploymentReport> {
    let checker = std::env::current_exe().context("resolve deployment checker path")?;
    let layout = resolve_install_layout(&checker)?;
    let checker_sha256 = sha256_file(&checker).context("hash deployment checker")?;
    let (app_sha256, app_hash_error) = match sha256_file(&layout.main_executable) {
        Ok(digest) => (digest, None),
        Err(error) => (String::new(), Some(format!("{error:#}"))),
    };
    let installer_sha256 = args
        .installer
        .as_deref()
        .and_then(|installer| sha256_file(installer).ok());

    let host = collect_host_info();
    let operating_system = check_operating_system(&host);
    let architecture = check_architecture();
    let mut authenticode =
        check_authenticode(&checker, &layout.main_executable, args.installer.as_deref());
    if let Some(error) = app_hash_error {
        authenticode = CheckResult::new(
            CheckStatus::Fail,
            format!(
                "installed app is missing or unreadable at {}: {error:#}",
                layout.main_executable.display()
            ),
        );
    }
    if let Some(installer) = args.installer.as_deref() {
        if installer_sha256.is_none() {
            authenticode = CheckResult::new(
                CheckStatus::Fail,
                format!(
                    "installer could not be read and hashed: {}",
                    installer.display()
                ),
            );
        }
    }
    let web_view2 = check_webview2();
    let (resource_hashes, adb_verified) = check_resources(&layout);
    let database_migration = check_database_migration();
    let credential_manager = check_credential_manager();
    let environment_overrides = check_environment_overrides(&layout.adb);
    let adb_version = check_adb_version(&layout.adb, adb_verified);
    let android_package_tools = check_android_package_tools(&layout.sidecars_root);
    let device_state = if args.device_check {
        check_devices(&layout.adb, args.device_serial.as_deref(), adb_verified)
    } else {
        CheckResult::new(
            CheckStatus::NotApplicable,
            "device check was not requested; first-host ADB enrollment remains a separate step",
        )
    };

    let mut report = DeploymentReport {
        schema_version: REPORT_SCHEMA_VERSION,
        generated_at_unix_ms: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis(),
        profile: args.profile,
        host,
        app_version: env!("CARGO_PKG_VERSION").to_string(),
        install_dir: layout.install_dir,
        app_sha256,
        checker_sha256,
        installer_path: args.installer.clone(),
        installer_sha256,
        adb_origin: adb_verified.then(|| "Bundled".to_string()),
        checks: DeploymentChecks {
            operating_system,
            architecture,
            authenticode,
            web_view2,
            resource_hashes,
            database_migration,
            credential_manager,
            environment_overrides,
            adb_version,
            android_package_tools,
            device_state,
        },
        overall: CheckStatus::Pass,
    };
    report.overall = report.overall_for(args.profile);
    Ok(report)
}

pub fn write_report(path: &Path, report: &DeploymentReport) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create report directory {}", parent.display()))?;
    }
    let bytes = serde_json::to_vec_pretty(report).context("serialize deployment report")?;
    std::fs::write(path, bytes)
        .with_context(|| format!("write deployment report {}", path.display()))
}

fn check_operating_system(host: &HostInfo) -> CheckResult {
    if !cfg!(windows) {
        return CheckResult::new(
            CheckStatus::Fail,
            "this certification profile supports Windows 10/11 only",
        );
    }
    let status = windows_version_status(&host.version, &host.build);
    CheckResult::new(
        status,
        if status == CheckStatus::Pass {
            format!("{} build {} is supported", host.name, host.build)
        } else {
            format!(
                "Windows version/build could not prove Windows 10 or newer: version={:?}, build={:?}",
                host.version, host.build
            )
        },
    )
}

fn windows_version_status(version: &str, build: &str) -> CheckStatus {
    let mut components = version.split('.');
    let parsed = (
        components
            .next()
            .and_then(|value| value.parse::<u32>().ok()),
        components
            .next()
            .and_then(|value| value.parse::<u32>().ok()),
        components
            .next()
            .and_then(|value| value.parse::<u32>().ok()),
        build.parse::<u32>().ok(),
    );
    match parsed {
        (Some(10), Some(0), Some(version_build), Some(build))
            if version_build == build && build >= 10_240 =>
        {
            CheckStatus::Pass
        }
        _ => CheckStatus::Fail,
    }
}

fn collect_host_info() -> HostInfo {
    let raw = if cfg!(windows) {
        Command::new("cmd.exe")
            .args(["/D", "/C", "ver"])
            .output()
            .ok()
            .filter(|output| output.status.success())
            .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
            .unwrap_or_default()
    } else {
        String::new()
    };
    let version = extract_windows_version(&raw).unwrap_or_default();
    let build = version.split('.').nth(2).unwrap_or_default().to_string();
    let name = build
        .parse::<u32>()
        .ok()
        .map(|build| {
            if build >= 22_000 {
                "Windows 11"
            } else {
                "Windows 10"
            }
        })
        .unwrap_or(if cfg!(windows) {
            "Windows"
        } else {
            std::env::consts::OS
        })
        .to_string();
    HostInfo {
        name,
        version,
        build,
        architecture: std::env::consts::ARCH.to_string(),
    }
}

fn extract_windows_version(raw: &str) -> Option<String> {
    raw.split(|character: char| !character.is_ascii_digit() && character != '.')
        .map(|candidate| candidate.trim_matches('.'))
        .filter(|candidate| candidate.matches('.').count() >= 2)
        .find(|candidate| {
            let components: Vec<_> = candidate.split('.').collect();
            components.len() >= 3
                && components
                    .iter()
                    .all(|component| !component.is_empty() && component.parse::<u32>().is_ok())
        })
        .map(str::to_string)
}

fn check_architecture() -> CheckResult {
    let architecture = std::env::consts::ARCH;
    let status = if architecture == "x86_64" {
        CheckStatus::Pass
    } else {
        CheckStatus::Fail
    };
    CheckResult::new(status, format!("process architecture: {architecture}"))
}

fn classify_authenticode_statuses(
    statuses: &BTreeMap<String, String>,
) -> (CheckStatus, &'static str) {
    if statuses.is_empty() {
        return (CheckStatus::Fail, "no Authenticode targets were checked");
    }
    if statuses.values().all(|value| value == "Valid") {
        return (
            CheckStatus::Pass,
            "installer and installed executables have valid Authenticode signatures",
        );
    }
    if statuses
        .values()
        .all(|value| matches!(value.as_str(), "Valid" | "NotSigned"))
    {
        return (
            CheckStatus::Warning,
            "one or more files are unsigned; internal profile requires the documented SmartScreen confirmation",
        );
    }
    (
        CheckStatus::Fail,
        "one or more Authenticode targets are missing, damaged, untrusted, or unverifiable",
    )
}

fn check_authenticode(checker: &Path, app: &Path, installer: Option<&Path>) -> CheckResult {
    if !cfg!(windows) {
        return CheckResult::new(CheckStatus::NotApplicable, "Authenticode is Windows-only");
    }
    let mut targets = vec![checker.to_path_buf(), app.to_path_buf()];
    if let Some(installer) = installer {
        targets.push(installer.to_path_buf());
    }
    let mut statuses = BTreeMap::new();
    for target in targets {
        if !target.is_file() {
            statuses.insert(target.display().to_string(), "Missing".to_string());
            continue;
        }
        let escaped = target.display().to_string().replace('\'', "''");
        let script = format!(
            "(Get-AuthenticodeSignature -LiteralPath '{}').Status.ToString()",
            escaped
        );
        let value = Command::new("powershell.exe")
            .args([
                "-NoLogo",
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                &script,
            ])
            .output()
            .ok()
            .filter(|output| output.status.success())
            .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "Unknown".to_string());
        statuses.insert(target.display().to_string(), value);
    }
    let (status, detail) = classify_authenticode_statuses(&statuses);
    CheckResult::new(status, detail).with_data(json!(statuses))
}

fn check_webview2() -> CheckResult {
    if !cfg!(windows) {
        return CheckResult::new(CheckStatus::NotApplicable, "WebView2 is Windows-only");
    }
    // Microsoft's documented Evergreen WebView2 Runtime product id. Edge browser channel
    // registrations use different ids and are not proof that the runtime is available.
    let client = r"{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}";
    let keys = [
        format!(r"HKLM\SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\{client}"),
        format!(r"HKCU\Software\Microsoft\EdgeUpdate\Clients\{client}"),
    ];
    for key in keys {
        if let Ok(output) = Command::new("reg.exe")
            .args(["query", &key, "/v", "pv"])
            .output()
        {
            let stdout = String::from_utf8_lossy(&output.stdout);
            if output.status.success() {
                if let Some(version) = webview_version_from_reg_output(&stdout) {
                    return CheckResult::new(
                        CheckStatus::Pass,
                        format!("WebView2 Evergreen Runtime {version}"),
                    );
                }
            }
        }
    }
    CheckResult::new(
        CheckStatus::Fail,
        "WebView2 Evergreen Runtime was not found after installer bootstrap",
    )
}

fn webview_version_from_reg_output(output: &str) -> Option<String> {
    let version = output
        .lines()
        .find(|line| line.split_whitespace().next() == Some("pv"))?
        .split_whitespace()
        .last()?
        .trim();
    (!version.is_empty() && version != "0.0.0.0").then(|| version.to_owned())
}

fn check_resources(layout: &InstallLayout) -> (CheckResult, bool) {
    let tools = AndroidTools::load_from(&layout.sidecars_root, SidecarOrigin::Packaged);
    let manifest = layout
        .sidecars_root
        .join("android")
        .join("android-tools-manifest.json");
    let manifest_sha256 = sha256_file(&manifest).ok();
    let declared_resource_hashes = std::fs::read(&manifest)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        .and_then(|document| document.get("files").cloned());
    let notice = layout.install_dir.join("NOTICE");
    let notice_sha256 = sha256_file(&notice).ok();
    let runtime_root = layout.sidecars_root.join("pymobiledevice3").join("runtime");
    let runtime = verify_runtime_manifest(&runtime_root);
    let ytdlp_root = layout.sidecars_root.join("yt-dlp");
    let ytdlp = verify_ytdlp_manifest(&ytdlp_root, true);
    let adb_verified = tools.adb_path.as_deref() == Some(layout.adb.as_path());
    let complete = tools.problems.is_empty()
        && adb_verified
        && tools.minicap_apk.is_some()
        && tools.scrcpy_server.is_some()
        && tools.riviu_agent_apk.is_some()
        && tools.agent_server_apk.is_some()
        && tools.agent_test_apk.is_some()
        && notice_sha256.is_some()
        && runtime.is_ok()
        && ytdlp.is_ok();
    let mut resource_problems = tools.problems.clone();
    if notice_sha256.is_none() {
        resource_problems.push(format!(
            "NOTICE is missing or unreadable at {}",
            notice.display()
        ));
    }
    if let Err(error) = &runtime {
        resource_problems.push(format!("runtime sidecar: {error:#}"));
    }
    if let Err(error) = &ytdlp {
        resource_problems.push(format!("yt-dlp: {error:#}"));
    }
    let detail = if complete {
        "Android, runtime, yt-dlp and NOTICE hashes match the installed resource manifests"
            .to_string()
    } else {
        format!(
            "installed resources are incomplete: {}",
            if resource_problems.is_empty() {
                "NOTICE or a required Android role is missing".to_string()
            } else {
                resource_problems.join("; ")
            }
        )
    };
    (
        CheckResult::new(
            if complete {
                CheckStatus::Pass
            } else {
                CheckStatus::Fail
            },
            detail,
        )
        .with_data(json!({
            "manifest": manifest,
            "manifestSha256": manifest_sha256,
            "declaredFiles": declared_resource_hashes,
            "notice": notice,
            "noticeSha256": notice_sha256,
            "runtime": runtime.as_ref().ok(),
            "runtimeError": runtime.as_ref().err().map(|error| format!("{error:#}")),
            "ytDlp": ytdlp.as_ref().ok(),
            "ytDlpError": ytdlp.as_ref().err().map(|error| format!("{error:#}")),
            "adb": tools.adb_path,
            "problems": tools.problems,
        })),
        adb_verified,
    )
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct RuntimeTreeAttestation {
    file_count: usize,
    payload_bytes: u64,
    tree_sha256: String,
}

fn runtime_tree_attestation(root: &Path) -> anyhow::Result<RuntimeTreeAttestation> {
    fn visit(
        root: &Path,
        current: &Path,
        entries: &mut Vec<(String, u32, u64, String)>,
    ) -> anyhow::Result<()> {
        for entry in fs::read_dir(current)
            .with_context(|| format!("read runtime directory {}", current.display()))?
        {
            let path = entry?.path();
            let metadata = fs::symlink_metadata(&path)?;
            if metadata.is_dir() {
                visit(root, &path, entries)?;
                continue;
            }
            if metadata.file_type().is_symlink() {
                return Err(anyhow!(
                    "runtime contains an unsupported symbolic link: {}",
                    path.display()
                ));
            }
            if !metadata.is_file()
                || path
                    .file_name()
                    .is_some_and(|name| name == "runtime-manifest.json")
            {
                continue;
            }
            let relative = path
                .strip_prefix(root)?
                .to_string_lossy()
                .replace('\\', "/");
            #[cfg(windows)]
            let mode = if metadata.permissions().readonly() {
                0o444
            } else if matches!(
                path.extension()
                    .and_then(|value| value.to_str())
                    .map(str::to_ascii_lowercase)
                    .as_deref(),
                Some("exe" | "com" | "bat" | "cmd")
            ) {
                0o777
            } else {
                0o666
            };
            #[cfg(unix)]
            let mode = {
                use std::os::unix::fs::PermissionsExt;
                metadata.permissions().mode() & 0o777
            };
            entries.push((relative, mode, metadata.len(), sha256_file(&path)?));
        }
        Ok(())
    }

    let mut entries = Vec::new();
    visit(root, root, &mut entries)?;
    entries.sort_by(|left, right| left.0.cmp(&right.0));
    let mut tree = Sha256::new();
    let mut payload_bytes = 0;
    for (relative, mode, size, digest) in &entries {
        payload_bytes += size;
        tree.update(relative.as_bytes());
        tree.update(b"\0file\0");
        tree.update(format!("{mode:o}").as_bytes());
        tree.update(b"\0");
        tree.update(size.to_string().as_bytes());
        tree.update(b"\0");
        tree.update(digest.as_bytes());
        tree.update(b"\n");
    }
    Ok(RuntimeTreeAttestation {
        file_count: entries.len(),
        payload_bytes,
        tree_sha256: format!("{:x}", tree.finalize()),
    })
}

fn safe_manifest_path(root: &Path, relative: &str) -> anyhow::Result<PathBuf> {
    let relative = Path::new(relative);
    if relative.is_absolute()
        || relative.components().any(|component| {
            matches!(
                component,
                std::path::Component::ParentDir
                    | std::path::Component::RootDir
                    | std::path::Component::Prefix(_)
            )
        })
    {
        return Err(anyhow!("manifest path escapes its resource root"));
    }
    Ok(root.join(relative))
}

fn verify_runtime_manifest(root: &Path) -> anyhow::Result<Value> {
    let manifest_path = root.join("runtime-manifest.json");
    let manifest: Value = serde_json::from_slice(
        &fs::read(&manifest_path)
            .with_context(|| format!("read runtime manifest {}", manifest_path.display()))?,
    )?;
    if manifest.get("schemaVersion").and_then(Value::as_u64) != Some(1) {
        return Err(anyhow!("runtime manifest schema is not 1"));
    }
    let entrypoint_relative = manifest
        .get("entrypoint")
        .and_then(Value::as_str)
        .context("runtime manifest entrypoint is missing")?;
    let entrypoint = safe_manifest_path(root, entrypoint_relative)?;
    let actual_entrypoint = sha256_file(&entrypoint)?;
    if manifest.get("entrypointSha256").and_then(Value::as_str) != Some(actual_entrypoint.as_str())
    {
        return Err(anyhow!(
            "runtime entrypoint SHA-256 does not match its manifest"
        ));
    }
    let measured = runtime_tree_attestation(root)?;
    if manifest.get("fileCount").and_then(Value::as_u64) != Some(measured.file_count as u64)
        || manifest.get("payloadBytes").and_then(Value::as_u64) != Some(measured.payload_bytes)
        || manifest.get("treeSha256").and_then(Value::as_str) != Some(measured.tree_sha256.as_str())
    {
        return Err(anyhow!(
            "runtime tree attestation does not match its manifest"
        ));
    }
    Ok(json!({
        "manifest": manifest_path,
        "entrypoint": entrypoint,
        "treeSha256": measured.tree_sha256,
        "fileCount": measured.file_count,
        "payloadBytes": measured.payload_bytes,
    }))
}

fn verify_ytdlp_manifest(root: &Path, execute: bool) -> anyhow::Result<Value> {
    let manifest_path = root.join("yt-dlp-manifest.json");
    let manifest: Value = serde_json::from_slice(
        &fs::read(&manifest_path)
            .with_context(|| format!("read yt-dlp manifest {}", manifest_path.display()))?,
    )?;
    if manifest.get("schemaVersion").and_then(Value::as_u64) != Some(1) {
        return Err(anyhow!("yt-dlp manifest schema is not 1"));
    }
    let relative = manifest
        .get("path")
        .and_then(Value::as_str)
        .context("yt-dlp manifest path is missing")?;
    let binary = safe_manifest_path(root, relative)?;
    let metadata = fs::metadata(&binary)
        .with_context(|| format!("read yt-dlp metadata {}", binary.display()))?;
    if manifest.get("bytes").and_then(Value::as_u64) != Some(metadata.len())
        || manifest.get("sha256").and_then(Value::as_str) != Some(sha256_file(&binary)?.as_str())
    {
        return Err(anyhow!("yt-dlp bytes or SHA-256 do not match its manifest"));
    }
    let expected_version = manifest
        .get("version")
        .and_then(Value::as_str)
        .context("yt-dlp manifest version is missing")?;
    if execute {
        let output = Command::new(&binary).arg("--version").output()?;
        let actual_version = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !output.status.success() || actual_version != expected_version {
            return Err(anyhow!(
                "yt-dlp version command does not match manifest: expected {expected_version:?}, got {actual_version:?}"
            ));
        }
    }
    Ok(json!({
        "manifest": manifest_path,
        "path": binary,
        "sha256": manifest.get("sha256"),
        "version": expected_version,
    }))
}

struct TempDatabase {
    root: PathBuf,
    armed: bool,
}

impl TempDatabase {
    fn new() -> anyhow::Result<Self> {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "riviu-deployment-check-db-{}-{nonce}",
            std::process::id()
        ));
        std::fs::create_dir_all(&root).context("create temporary database directory")?;
        Ok(Self { root, armed: true })
    }

    fn finish(&mut self) -> anyhow::Result<()> {
        fs::remove_dir_all(&self.root).with_context(|| {
            format!(
                "remove temporary database directory {}",
                self.root.display()
            )
        })?;
        self.armed = false;
        Ok(())
    }
}

impl Drop for TempDatabase {
    fn drop(&mut self) {
        if self.armed {
            let _ = fs::remove_dir_all(&self.root);
        }
    }
}

fn combine_with_cleanup<T>(
    operation: anyhow::Result<T>,
    cleanup: anyhow::Result<()>,
) -> anyhow::Result<T> {
    match (operation, cleanup) {
        (Ok(value), Ok(())) => Ok(value),
        (Err(error), Ok(())) => Err(error),
        (Ok(_), Err(cleanup)) => Err(cleanup.context("cleanup failed")),
        (Err(error), Err(cleanup)) => Err(anyhow!("{error:#}; cleanup failed: {cleanup:#}")),
    }
}

fn check_database_migration() -> CheckResult {
    let result = (|| -> anyhow::Result<i64> {
        let mut temporary = TempDatabase::new()?;
        let operation = (|| -> anyhow::Result<i64> {
            let database = riviu_core::db::Database::open(temporary.root.join("riviu.db"))?;
            let version = database.schema_version()?;
            if version != riviu_core::db::Database::latest_schema_version() {
                return Err(anyhow!(
                    "schema reached {version}, expected {}",
                    riviu_core::db::Database::latest_schema_version()
                ));
            }
            Ok(version)
        })();
        combine_with_cleanup(operation, temporary.finish())
    })();
    match result {
        Ok(version) => CheckResult::new(
            CheckStatus::Pass,
            format!("fresh production database migrated to schema {version}"),
        )
        .with_data(json!({ "schemaVersion": version })),
        Err(error) => CheckResult::new(
            CheckStatus::Fail,
            format!("fresh database migration failed: {error:#}"),
        ),
    }
}

fn check_credential_manager() -> CheckResult {
    let name = format!(
        "deployment-check-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    );
    let value = format!("fixture-{}", std::process::id());
    let result = riviu_signing::CredentialStore::system()
        .and_then(|store| credential_round_trip(store, name, value));
    match result {
        Ok(()) => CheckResult::new(
            CheckStatus::Pass,
            "Windows Credential Manager set/get/delete round-trip passed",
        ),
        Err(error) => CheckResult::new(
            CheckStatus::Fail,
            format!("credential round-trip failed: {error:#}"),
        ),
    }
}

fn credential_round_trip(
    store: riviu_signing::CredentialStore,
    name: String,
    value: String,
) -> anyhow::Result<()> {
    store.set_app_secret(&name, &value)?;
    let mut cleanup = CredentialCleanup::new(store.clone(), name.clone());
    let operation = (|| -> anyhow::Result<()> {
        let read = store.app_secret(&name)?;
        if read.as_deref() != Some(value.as_str()) {
            return Err(anyhow!("credential read-back did not match"));
        }
        Ok(())
    })();
    combine_with_cleanup(operation, cleanup.finish())
}

struct CredentialCleanup {
    store: riviu_signing::CredentialStore,
    name: String,
    armed: bool,
}

impl CredentialCleanup {
    fn new(store: riviu_signing::CredentialStore, name: String) -> Self {
        Self {
            store,
            name,
            armed: true,
        }
    }

    fn finish(&mut self) -> anyhow::Result<()> {
        self.store.set_app_secret(&self.name, "")?;
        if self.store.app_secret(&self.name)?.is_some() {
            return Err(anyhow!("temporary credential remained after cleanup"));
        }
        self.armed = false;
        Ok(())
    }
}

impl Drop for CredentialCleanup {
    fn drop(&mut self) {
        if self.armed {
            let _ = self.store.set_app_secret(&self.name, "");
        }
    }
}

fn check_environment_overrides(packaged_adb: &Path) -> CheckResult {
    let mut overrides = BTreeMap::new();
    for key in [
        "RIVIU_ADB_PATH",
        "RIVIU_JAVA_PATH",
        "RIVIU_BUNDLETOOL_PATH",
        "ANDROID_SDK_ROOT",
        "ANDROID_HOME",
        "JAVA_HOME",
    ] {
        if let Ok(value) = std::env::var(key) {
            if !value.trim().is_empty() {
                overrides.insert(key.to_string(), value);
            }
        }
    }
    let adb_name = if cfg!(windows) { "adb.exe" } else { "adb" };
    let path_adb: Vec<PathBuf> =
        std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
            .map(|root| root.join(adb_name))
            .filter(|path| path.is_file() && path != packaged_adb)
            .collect();
    let status = if overrides.is_empty() && path_adb.is_empty() {
        CheckStatus::Pass
    } else {
        CheckStatus::Warning
    };
    CheckResult::new(
        status,
        if status == CheckStatus::Pass {
            "no Android environment override or competing adb was found"
        } else {
            "external Android configuration exists; checker still invoked the bundled adb explicitly"
        },
    )
    .with_data(json!({ "environment": overrides, "pathAdb": path_adb }))
}

fn check_adb_version(adb: &Path, resource_verified: bool) -> CheckResult {
    if !resource_verified {
        return CheckResult::new(
            CheckStatus::Fail,
            "bundled adb did not pass resource verification",
        );
    }
    match Command::new(adb).arg("version").output() {
        Ok(output)
            if output.status.success()
                && String::from_utf8_lossy(&output.stdout)
                    .contains("Android Debug Bridge version") =>
        {
            CheckResult::new(
                CheckStatus::Pass,
                String::from_utf8_lossy(&output.stdout).trim().to_string(),
            )
            .with_data(json!({ "path": adb, "origin": "Bundled" }))
        }
        Ok(output) => CheckResult::new(
            CheckStatus::Fail,
            format!(
                "bundled adb version command failed: status={}; stdout={:?}; stderr={:?}",
                output.status,
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            ),
        ),
        Err(error) => CheckResult::new(
            CheckStatus::Fail,
            format!("could not start bundled adb {}: {error}", adb.display()),
        ),
    }
}

pub(crate) fn check_android_package_tools(sidecars_root: &Path) -> CheckResult {
    const EXPECTED_BUNDLETOOL: &str = "1.18.3";
    const EXPECTED_JRE: &str = "21.0.12.1+1";
    const EXPECTED_BUNDLETOOL_BYTES: u64 = 32_520_401;
    const EXPECTED_JRE_SOURCE_BYTES: u64 = 48_999_141;
    const EXPECTED_BUNDLETOOL_SHA256: &str =
        "a099cfa1543f55593bc2ed16a70a7c67fe54b1747bb7301f37fdfd6d91028e29";
    const EXPECTED_JRE_SOURCE_SHA256: &str =
        "d35f31e712f0fcf6ac5a093edc90204fbff22f720ba3950bd09d331d5e621636";
    const EXPECTED_BUNDLETOOL_SOURCE: &str =
        "https://github.com/google/bundletool/releases/download/1.18.3/bundletool-all-1.18.3.jar";
    const EXPECTED_JRE_SOURCE: &str = "https://github.com/adoptium/temurin21-binaries/releases/download/jdk-21.0.12.1%2B1/OpenJDK21U-jre_x64_windows_hotspot_21.0.12.1_1.zip";
    const EXPECTED_TREE_SHA256: &str =
        "f24951701beb69fe74ef073196c249d6df153749722f82260d79fc6687a7d57f";

    let root = sidecars_root.join("android-package-tools");
    let manifest_path = root.join("android-package-tools-manifest.json");
    let result = (|| -> anyhow::Result<Value> {
        let bytes = fs::read(&manifest_path)
            .with_context(|| format!("read {}", manifest_path.display()))?;
        let manifest: Value = serde_json::from_slice(&bytes)
            .with_context(|| format!("parse {}", manifest_path.display()))?;
        if manifest.get("schemaVersion").and_then(Value::as_u64) != Some(1) {
            return Err(anyhow!("unsupported Android package-tools manifest schema"));
        }
        let bundletool = manifest
            .get("bundletool")
            .and_then(Value::as_object)
            .context("manifest lacks Bundletool provenance")?;
        let jre = manifest
            .get("jre")
            .and_then(Value::as_object)
            .context("manifest lacks JRE provenance")?;
        if bundletool.get("path").and_then(Value::as_str) != Some("bundletool.jar")
            || bundletool.get("version").and_then(Value::as_str) != Some(EXPECTED_BUNDLETOOL)
            || bundletool.get("sourceBytes").and_then(Value::as_u64)
                != Some(EXPECTED_BUNDLETOOL_BYTES)
            || bundletool.get("source").and_then(Value::as_str) != Some(EXPECTED_BUNDLETOOL_SOURCE)
            || bundletool.get("sourceSha256").and_then(Value::as_str)
                != Some(EXPECTED_BUNDLETOOL_SHA256)
            || jre.get("javaPath").and_then(Value::as_str) != Some("jre/bin/java.exe")
            || jre.get("version").and_then(Value::as_str) != Some(EXPECTED_JRE)
            || jre.get("sourceBytes").and_then(Value::as_u64) != Some(EXPECTED_JRE_SOURCE_BYTES)
            || jre.get("source").and_then(Value::as_str) != Some(EXPECTED_JRE_SOURCE)
            || jre.get("sourceSha256").and_then(Value::as_str) != Some(EXPECTED_JRE_SOURCE_SHA256)
        {
            return Err(anyhow!("Android package-tools provenance pin mismatch"));
        }
        let entries = manifest
            .get("files")
            .and_then(Value::as_array)
            .filter(|entries| !entries.is_empty())
            .context("manifest has no package-tool files")?;
        let mut declared = std::collections::BTreeSet::new();
        let mut manifest_files = Vec::new();
        for entry in entries {
            let relative = entry
                .get("path")
                .and_then(Value::as_str)
                .context("package-tool file has no path")?;
            let relative_path = Path::new(relative);
            if relative_path.is_absolute()
                || relative_path.components().any(|component| {
                    matches!(
                        component,
                        std::path::Component::ParentDir
                            | std::path::Component::RootDir
                            | std::path::Component::Prefix(_)
                    )
                })
                || !declared.insert(relative.replace('\\', "/").to_ascii_lowercase())
            {
                return Err(anyhow!(
                    "unsafe or duplicate package-tool path: {relative:?}"
                ));
            }
            let path = root.join(relative_path);
            let expected_size = entry
                .get("bytes")
                .and_then(Value::as_u64)
                .context("package-tool file has no byte count")?;
            let expected_sha = entry
                .get("sha256")
                .and_then(Value::as_str)
                .context("package-tool file has no SHA-256")?;
            if fs::metadata(&path)?.len() != expected_size || sha256_file(&path)? != expected_sha {
                return Err(anyhow!(
                    "package-tool bytes differ from manifest: {}",
                    path.display()
                ));
            }
            manifest_files.push((
                relative.replace('\\', "/"),
                expected_size,
                expected_sha.to_string(),
            ));
        }
        let mut on_disk = std::collections::BTreeSet::new();
        collect_regular_relative_files(&root, &root, &mut on_disk)?;
        on_disk.remove("android-package-tools-manifest.json");
        if on_disk != declared {
            return Err(anyhow!(
                "package-tools manifest does not cover the complete tree"
            ));
        }

        manifest_files.sort_by(|left, right| left.0.cmp(&right.0));
        let payload_bytes: u64 = manifest_files.iter().map(|entry| entry.1).sum();
        let mut tree = Sha256::new();
        for (relative, size, digest) in &manifest_files {
            tree.update(relative.as_bytes());
            tree.update(b"\0");
            tree.update(size.to_string().as_bytes());
            tree.update(b"\0");
            tree.update(digest.as_bytes());
            tree.update(b"\n");
        }
        let tree_digest = format!("{:x}", tree.finalize());
        if manifest.get("fileCount").and_then(Value::as_u64) != Some(manifest_files.len() as u64)
            || manifest.get("payloadBytes").and_then(Value::as_u64) != Some(payload_bytes)
            || manifest.get("treeSha256").and_then(Value::as_str) != Some(tree_digest.as_str())
            || tree_digest != EXPECTED_TREE_SHA256
        {
            return Err(anyhow!(
                "Android package-tools tree attestation mismatch: {tree_digest}"
            ));
        }

        let java = root.join("jre").join("bin").join("java.exe");
        let jar = root.join("bundletool.jar");
        if fs::metadata(&jar)?.len() != EXPECTED_BUNDLETOOL_BYTES
            || sha256_file(&jar)? != EXPECTED_BUNDLETOOL_SHA256
        {
            return Err(anyhow!(
                "packaged Bundletool does not match the pinned release bytes"
            ));
        }
        let java_output = Command::new(&java).arg("-version").output()?;
        let java_version = format!(
            "{}{}",
            String::from_utf8_lossy(&java_output.stdout),
            String::from_utf8_lossy(&java_output.stderr)
        );
        if !java_output.status.success() || !java_version.contains("21.0.12.1+1") {
            return Err(anyhow!("packaged java -version mismatch: {java_version:?}"));
        }
        let bundle_output = Command::new(&java)
            .args(["-jar"])
            .arg(&jar)
            .arg("version")
            .output()?;
        let bundle_version = String::from_utf8_lossy(&bundle_output.stdout)
            .trim()
            .to_string();
        if !bundle_output.status.success() || bundle_version != EXPECTED_BUNDLETOOL {
            return Err(anyhow!(
                "packaged Bundletool version mismatch: {bundle_version:?}"
            ));
        }
        Ok(json!({
            "manifest": manifest_path,
            "manifestSha256": sha256_file(&manifest_path)?,
            "fileCount": declared.len(),
            "payloadBytes": payload_bytes,
            "treeSha256": tree_digest,
            "java": java,
            "bundletool": jar,
            "javaVersion": EXPECTED_JRE,
            "bundletoolVersion": EXPECTED_BUNDLETOOL,
        }))
    })();

    match result {
        Ok(data) => CheckResult::new(
            CheckStatus::Pass,
            "bundled Temurin JRE and Bundletool match their complete pinned manifest",
        )
        .with_data(data),
        Err(error) => CheckResult::new(
            CheckStatus::Fail,
            format!("Android package tools failed verification: {error:#}"),
        ),
    }
}

fn collect_regular_relative_files(
    root: &Path,
    current: &Path,
    files: &mut std::collections::BTreeSet<String>,
) -> anyhow::Result<()> {
    for entry in fs::read_dir(current)? {
        let path = entry?.path();
        let metadata = fs::symlink_metadata(&path)?;
        if metadata.file_type().is_symlink() {
            return Err(anyhow!(
                "package-tools tree contains a symlink: {}",
                path.display()
            ));
        }
        if metadata.is_dir() {
            collect_regular_relative_files(root, &path, files)?;
        } else if metadata.is_file() {
            files.insert(
                path.strip_prefix(root)?
                    .to_string_lossy()
                    .replace('\\', "/")
                    .to_ascii_lowercase(),
            );
        }
    }
    Ok(())
}

fn check_devices(adb: &Path, serial: Option<&str>, resource_verified: bool) -> CheckResult {
    if !resource_verified {
        return CheckResult::new(
            CheckStatus::Fail,
            "device check needs a verified bundled adb",
        );
    }
    let read = || -> anyhow::Result<Vec<riviu_android_driver::adb::AdbDeviceLine>> {
        let output = Command::new(adb)
            .args(["devices", "-l"])
            .output()
            .context("run bundled adb devices -l")?;
        if !output.status.success() {
            return Err(anyhow!(
                "adb devices failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }
        Ok(parse_devices(&String::from_utf8_lossy(&output.stdout)))
    };
    let result = (|| -> anyhow::Result<Vec<riviu_android_driver::adb::AdbDeviceLine>> {
        let first = read()?;
        std::thread::sleep(Duration::from_millis(300));
        let second = read()?;
        if first != second {
            return Err(anyhow!("two consecutive adb readings did not match"));
        }
        Ok(second)
    })();
    let devices = match result {
        Ok(devices) => devices,
        Err(error) => {
            return CheckResult::new(CheckStatus::Fail, format!("device read failed: {error:#}"));
        }
    };
    let selected: Vec<_> = devices
        .iter()
        .filter(|device| serial.is_none_or(|serial| device.serial == serial))
        .collect();
    let healthy = !selected.is_empty()
        && selected
            .iter()
            .all(|device| device.state == AdbDeviceState::Device);
    CheckResult::new(
        if healthy {
            CheckStatus::Pass
        } else {
            CheckStatus::Fail
        },
        if healthy {
            "requested Android device enrollment is authorized and stable".to_string()
        } else {
            "no matching authorized device; install the OEM driver and approve Allow USB debugging once on the phone".to_string()
        },
    )
    .with_data(json!({
        "requestedSerial": serial,
        "devices": devices.iter().map(|device| json!({
            "serial": device.serial,
            "state": format!("{:?}", device.state).to_lowercase(),
            "model": device.model,
        })).collect::<Vec<_>>()
    }))
}

fn sha256_file(path: &Path) -> anyhow::Result<String> {
    let file = std::fs::File::open(path).with_context(|| format!("open {}", path.display()))?;
    let mut reader = std::io::BufReader::new(file);
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = reader
            .read(&mut buffer)
            .with_context(|| format!("read {}", path.display()))?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    Ok(format!("{:x}", digest.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[test]
    fn missing_required_arguments_are_rejected() {
        let error =
            parse_args(["checker", "--profile", "internal"]).expect_err("missing report must fail");
        assert!(error.to_string().contains("--report is required"));
    }

    #[test]
    fn device_check_accepts_an_optional_serial() {
        let args = parse_args([
            "checker",
            "--profile",
            "internal",
            "--report",
            "report.json",
            "--device-check",
        ])
        .expect("device check without serial");
        assert!(args.device_check);
        assert_eq!(args.device_serial, None);
    }

    #[test]
    fn ordinary_gui_arguments_do_not_enter_deployment_smoke() {
        assert_eq!(
            parse_deployment_smoke_args(["riviu-managers-phone.exe"]).expect("ordinary args"),
            None
        );
    }

    #[test]
    fn deployment_smoke_requires_absolute_scratch_paths() {
        let error = parse_deployment_smoke_args([
            "app.exe",
            "--deployment-smoke",
            "report.json",
            "--data-dir",
            "data",
        ])
        .expect_err("relative smoke paths must fail");
        assert!(error.to_string().contains("absolute"));
    }

    #[test]
    fn webview_registry_detection_requires_a_nonzero_runtime_version() {
        let installed = "    pv    REG_SZ    136.0.3240.92\r\n";
        assert_eq!(
            webview_version_from_reg_output(installed).as_deref(),
            Some("136.0.3240.92")
        );
        assert_eq!(
            webview_version_from_reg_output("    pv    REG_SZ    0.0.0.0\r\n"),
            None
        );
        assert_eq!(webview_version_from_reg_output(""), None);
    }

    #[test]
    fn windows_10_and_11_builds_are_supported() {
        assert_eq!(
            windows_version_status("10.0.19045.0", "19045"),
            CheckStatus::Pass
        );
        assert_eq!(
            windows_version_status("10.0.22631.0", "22631"),
            CheckStatus::Pass
        );
    }

    #[test]
    fn missing_malformed_and_pre_windows_10_builds_fail_closed() {
        assert_eq!(windows_version_status("", ""), CheckStatus::Fail);
        assert_eq!(
            windows_version_status("not-a-version", "19045"),
            CheckStatus::Fail
        );
        assert_eq!(
            windows_version_status("6.3.9600", "9600"),
            CheckStatus::Fail
        );
        assert_eq!(
            windows_version_status("10.0.9600", "9600"),
            CheckStatus::Fail
        );
    }

    #[test]
    fn localized_windows_version_output_is_parsed_by_numeric_shape() {
        assert_eq!(
            extract_windows_version("Microsoft Windows [Phiên bản 10.0.19045.4046]"),
            Some("10.0.19045.4046".to_string())
        );
        assert_eq!(extract_windows_version("không có số phiên bản"), None);
    }

    #[test]
    fn internal_authenticode_warns_only_for_unsigned_files() {
        let statuses = BTreeMap::from([
            ("app.exe".to_string(), "Valid".to_string()),
            ("installer.msi".to_string(), "NotSigned".to_string()),
        ]);
        assert_eq!(
            classify_authenticode_statuses(&statuses).0,
            CheckStatus::Warning
        );
        for invalid in [
            "HashMismatch",
            "NotTrusted",
            "UnknownError",
            "Unknown",
            "Missing",
        ] {
            let statuses = BTreeMap::from([
                ("app.exe".to_string(), "Valid".to_string()),
                ("installer.msi".to_string(), invalid.to_string()),
            ]);
            assert_eq!(
                classify_authenticode_statuses(&statuses).0,
                CheckStatus::Fail,
                "{invalid} cannot be downgraded to an unsigned warning"
            );
        }
    }

    #[test]
    fn runtime_manifest_detects_a_changed_installed_payload() {
        let root = std::env::temp_dir().join(format!(
            "riviu-runtime-resource-test-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&root).expect("create runtime fixture");
        let entrypoint = root.join("riviu-pmd.exe");
        std::fs::write(&entrypoint, b"runtime-v1").expect("write runtime fixture");
        let attestation = runtime_tree_attestation(&root).expect("attest runtime fixture");
        #[cfg(windows)]
        assert_eq!(
            attestation.tree_sha256,
            "1470f9292efdad1caab9561be55ec69c3277a3e7aed2edaff8cf6743c2db46e3"
        );
        std::fs::write(
            root.join("runtime-manifest.json"),
            serde_json::to_vec(&json!({
                "schemaVersion": 1,
                "entrypoint": "riviu-pmd.exe",
                "entrypointSha256": sha256_file(&entrypoint).expect("hash entrypoint"),
                "fileCount": attestation.file_count,
                "payloadBytes": attestation.payload_bytes,
                "treeSha256": attestation.tree_sha256,
            }))
            .expect("encode runtime manifest"),
        )
        .expect("write runtime manifest");
        verify_runtime_manifest(&root).expect("valid runtime manifest");

        std::fs::write(&entrypoint, b"runtime-v2").expect("corrupt runtime fixture");
        let error = verify_runtime_manifest(&root).expect_err("changed runtime must fail");
        assert!(error.to_string().contains("entrypoint SHA-256"));
        std::fs::remove_dir_all(root).expect("remove runtime fixture");
    }

    #[test]
    fn ytdlp_manifest_detects_a_changed_installed_payload() {
        let root = std::env::temp_dir().join(format!(
            "riviu-ytdlp-resource-test-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&root).expect("create yt-dlp fixture");
        let binary = root.join("yt-dlp.exe");
        std::fs::write(&binary, b"yt-dlp-v1").expect("write yt-dlp fixture");
        std::fs::write(
            root.join("yt-dlp-manifest.json"),
            serde_json::to_vec(&json!({
                "schemaVersion": 1,
                "path": "yt-dlp.exe",
                "bytes": 9,
                "sha256": sha256_file(&binary).expect("hash yt-dlp"),
                "version": "fixture",
            }))
            .expect("encode yt-dlp manifest"),
        )
        .expect("write yt-dlp manifest");
        verify_ytdlp_manifest(&root, false).expect("valid yt-dlp manifest");

        std::fs::write(&binary, b"yt-dlp-v2").expect("corrupt yt-dlp fixture");
        let error = verify_ytdlp_manifest(&root, false).expect_err("changed yt-dlp must fail");
        assert!(error.to_string().contains("SHA-256"));
        std::fs::remove_dir_all(root).expect("remove yt-dlp fixture");
    }

    #[test]
    fn package_tools_manifest_detects_an_extra_installed_payload() {
        let source = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("..")
            .join("target")
            .join("android-package-tools");
        if !source.is_dir() {
            return;
        }
        let sidecars = std::env::temp_dir().join(format!(
            "riviu-package-tools-resource-test-{}",
            uuid::Uuid::new_v4()
        ));
        copy_directory(&source, &sidecars.join("android-package-tools"))
            .expect("copy package-tools fixture");
        assert_eq!(
            check_android_package_tools(&sidecars).status,
            CheckStatus::Pass
        );
        std::fs::write(
            sidecars
                .join("android-package-tools")
                .join("unmanifested.dll"),
            b"extra",
        )
        .expect("write unmanifested package tool");
        assert_eq!(
            check_android_package_tools(&sidecars).status,
            CheckStatus::Fail
        );
        std::fs::remove_file(
            sidecars
                .join("android-package-tools")
                .join("unmanifested.dll"),
        )
        .expect("remove unmanifested package tool");
        let manifest_path = sidecars
            .join("android-package-tools")
            .join("android-package-tools-manifest.json");
        let mut manifest: Value = serde_json::from_slice(
            &std::fs::read(&manifest_path).expect("read package-tools manifest"),
        )
        .expect("parse package-tools manifest");
        manifest["jre"]["sourceBytes"] = json!(1);
        std::fs::write(
            &manifest_path,
            serde_json::to_vec_pretty(&manifest).expect("encode changed package-tools manifest"),
        )
        .expect("write changed package-tools manifest");
        assert_eq!(
            check_android_package_tools(&sidecars).status,
            CheckStatus::Fail
        );
        std::fs::remove_dir_all(sidecars).expect("remove package-tools fixture");
    }

    fn copy_directory(source: &Path, destination: &Path) -> anyhow::Result<()> {
        std::fs::create_dir_all(destination)?;
        for entry in std::fs::read_dir(source)? {
            let path = entry?.path();
            let target = destination.join(path.file_name().context("fixture file name")?);
            if path.is_dir() {
                copy_directory(&path, &target)?;
            } else {
                std::fs::copy(&path, &target)?;
            }
        }
        Ok(())
    }

    #[test]
    fn primary_and_cleanup_failures_are_both_reported() {
        let error = combine_with_cleanup::<()>(
            Err(anyhow!("migration failed")),
            Err(anyhow!("remove failed")),
        )
        .expect_err("two failures must remain observable");
        let message = format!("{error:#}");
        assert!(message.contains("migration failed"));
        assert!(message.contains("cleanup failed"));
        assert!(message.contains("remove failed"));
    }

    #[test]
    fn credential_delete_failure_is_a_failed_round_trip() {
        #[derive(Default)]
        struct DeleteFails {
            value: Mutex<Option<String>>,
        }
        impl riviu_signing::CredentialBackend for DeleteFails {
            fn get(&self, _account: &str) -> anyhow::Result<Option<String>> {
                Ok(self.value.lock().expect("fixture value").clone())
            }

            fn set(&self, _account: &str, value: &str) -> anyhow::Result<()> {
                *self.value.lock().expect("fixture value") = Some(value.to_string());
                Ok(())
            }

            fn delete(&self, _account: &str) -> anyhow::Result<()> {
                Err(anyhow!("fixture credential delete failed"))
            }
        }

        let store = riviu_signing::CredentialStore::new(Arc::new(DeleteFails::default()));
        let error = credential_round_trip(store, "fixture".into(), "secret".into())
            .expect_err("cleanup failure must fail the check");
        let message = format!("{error:#}");
        assert!(message.contains("cleanup failed"));
        assert!(message.contains("fixture credential delete failed"));
    }
}
