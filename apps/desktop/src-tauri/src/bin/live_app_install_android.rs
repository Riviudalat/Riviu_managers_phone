use std::collections::HashSet;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use futures_util::{stream, StreamExt};
use riviu_core::{
    AppInstallResult, AppInstallStatus, DeviceAppInstallRequest, DeviceControlPlane, DeviceStatus,
    DeviceWorkCoordinator, DeviceWorkOwner, StreamBudgetManager,
};
use serde::Serialize;

const MAX_INSTALL_CONCURRENCY: usize = 2;
const SUMMARY_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LiveMode {
    Install,
    Uninstall,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct LiveArgs {
    mode: LiveMode,
    udids: Vec<String>,
    apk_paths: Vec<PathBuf>,
    package: String,
    version_name: Option<String>,
    version_code: Option<String>,
    allow_downgrade: bool,
}

fn take_value(
    arguments: &mut impl Iterator<Item = String>,
    option: &str,
) -> anyhow::Result<String> {
    let value = arguments
        .next()
        .ok_or_else(|| anyhow::anyhow!("{option} requires a value"))?;
    anyhow::ensure!(
        !value.trim().is_empty() && !value.starts_with("--"),
        "{option} requires a value"
    );
    Ok(value)
}

fn parse_args(arguments: impl IntoIterator<Item = String>) -> anyhow::Result<LiveArgs> {
    let mut arguments = arguments.into_iter();
    let mut uninstall = false;
    let mut saw_uninstall = false;
    let mut udids = Vec::new();
    let mut apk_paths = Vec::new();
    let mut package = None;
    let mut version_name = None;
    let mut version_code = None;
    let mut allow_downgrade = false;
    let mut saw_allow_downgrade = false;

    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--udid" => udids.push(take_value(&mut arguments, "--udid")?),
            "--apk" => {
                let path = PathBuf::from(take_value(&mut arguments, "--apk")?);
                anyhow::ensure!(
                    path.extension()
                        .and_then(|extension| extension.to_str())
                        .is_some_and(|extension| extension.eq_ignore_ascii_case("apk")),
                    "--apk only accepts an .apk file"
                );
                apk_paths.push(path);
            }
            "--package" => {
                anyhow::ensure!(package.is_none(), "--package may only be specified once");
                package = Some(take_value(&mut arguments, "--package")?);
            }
            "--version-name" => {
                anyhow::ensure!(
                    version_name.is_none(),
                    "--version-name may only be specified once"
                );
                version_name = Some(take_value(&mut arguments, "--version-name")?);
            }
            "--version-code" => {
                anyhow::ensure!(
                    version_code.is_none(),
                    "--version-code may only be specified once"
                );
                version_code = Some(take_value(&mut arguments, "--version-code")?);
            }
            "--allow-downgrade" => {
                anyhow::ensure!(
                    !saw_allow_downgrade,
                    "--allow-downgrade may only be specified once"
                );
                saw_allow_downgrade = true;
                allow_downgrade = true;
            }
            "--uninstall" => {
                anyhow::ensure!(!saw_uninstall, "--uninstall may only be specified once");
                saw_uninstall = true;
                uninstall = true;
            }
            value => anyhow::bail!("unknown argument {value}"),
        }
    }

    anyhow::ensure!(!udids.is_empty(), "at least one --udid is required");
    let mut unique_udids = HashSet::with_capacity(udids.len());
    anyhow::ensure!(
        udids.iter().all(|udid| unique_udids.insert(udid)),
        "each --udid must be unique"
    );
    let package = package.ok_or_else(|| anyhow::anyhow!("--package is required"))?;

    let mode = if uninstall {
        anyhow::ensure!(apk_paths.is_empty(), "--uninstall does not accept --apk");
        anyhow::ensure!(
            version_name.is_none(),
            "--uninstall does not accept --version-name"
        );
        anyhow::ensure!(
            version_code.is_none(),
            "--uninstall does not accept --version-code"
        );
        anyhow::ensure!(
            !allow_downgrade,
            "--uninstall does not accept --allow-downgrade"
        );
        LiveMode::Uninstall
    } else {
        anyhow::ensure!(!apk_paths.is_empty(), "at least one --apk is required");
        anyhow::ensure!(version_name.is_some(), "--version-name is required");
        LiveMode::Install
    };

    Ok(LiveArgs {
        mode,
        udids,
        apk_paths,
        package,
        version_name,
        version_code,
        allow_downgrade,
    })
}

fn exact_adb_config(adb_path: PathBuf) -> riviu_android_driver::AndroidDriverConfig {
    riviu_android_driver::AndroidDriverConfig {
        adb_path: Some(adb_path),
        bundled_adb_path: None,
        ..riviu_android_driver::AndroidDriverConfig::default()
    }
}

async fn run_bounded<T, U, I, F, Fut>(items: I, limit: usize, map: F) -> Vec<U>
where
    I: IntoIterator<Item = T>,
    F: FnMut(T) -> Fut,
    Fut: Future<Output = U>,
{
    stream::iter(items)
        .map(map)
        .buffer_unordered(limit)
        .collect()
        .await
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct LiveSummary {
    schema_version: u32,
    operation: &'static str,
    package: String,
    adb_path: String,
    max_concurrency: usize,
    overall: &'static str,
    results: Vec<AppInstallResult>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cleanup_error: Option<String>,
}

impl LiveSummary {
    fn new(
        mode: LiveMode,
        package: String,
        adb_path: String,
        results: Vec<AppInstallResult>,
    ) -> Self {
        let all_succeeded = !results.is_empty()
            && results
                .iter()
                .all(|result| result.status == AppInstallStatus::Succeeded);
        Self {
            schema_version: SUMMARY_SCHEMA_VERSION,
            operation: match mode {
                LiveMode::Install => "install",
                LiveMode::Uninstall => "uninstall",
            },
            package,
            adb_path,
            max_concurrency: MAX_INSTALL_CONCURRENCY,
            overall: if all_succeeded { "pass" } else { "failed" },
            results,
            cleanup_error: None,
        }
    }

    fn is_success(&self) -> bool {
        self.overall == "pass" && self.cleanup_error.is_none()
    }

    fn set_cleanup_error(&mut self, error: impl std::fmt::Display) {
        self.cleanup_error = Some(error.to_string());
        self.overall = "failed";
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct HarnessFailure {
    schema_version: u32,
    overall: &'static str,
    error: String,
}

fn before_effect(udid: String, detail: impl Into<String>) -> AppInstallResult {
    AppInstallResult {
        udid,
        status: AppInstallStatus::BeforeEffect,
        effect_started: false,
        observed_version_name: None,
        observed_version_code: None,
        detail: Some(detail.into()),
    }
}

fn uncertain(udid: String, detail: impl Into<String>) -> AppInstallResult {
    AppInstallResult {
        udid,
        status: AppInstallStatus::Uncertain,
        effect_started: true,
        observed_version_name: None,
        observed_version_code: None,
        detail: Some(detail.into()),
    }
}

fn resolve_sidecar_adb() -> anyhow::Result<PathBuf> {
    let root = std::env::var_os("RIVIU_SIDECAR_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../sidecars"));
    let path = root.join("android").join("win-x86_64").join("adb.exe");
    anyhow::ensure!(
        path.is_file(),
        "the packaged adb.exe is missing at {}",
        path.display()
    );
    std::fs::canonicalize(&path)
        .map_err(|error| anyhow::anyhow!("resolve packaged adb {}: {error}", path.display()))
}

fn validate_apk_paths(paths: &[PathBuf]) -> anyhow::Result<()> {
    for path in paths {
        anyhow::ensure!(
            path.is_file(),
            "the APK is missing or is not a file: {}",
            path.display()
        );
    }
    Ok(())
}

async fn execute(args: LiveArgs) -> anyhow::Result<LiveSummary> {
    if args.mode == LiveMode::Install {
        validate_apk_paths(&args.apk_paths)?;
    }

    let adb_path = resolve_sidecar_adb()?;
    let android = Arc::new(riviu_android_driver::AndroidDriver::new(
        &exact_adb_config(adb_path.clone()),
    )?);
    let selected_adb_path = android.adb_path();
    anyhow::ensure!(
        Path::new(&selected_adb_path) == adb_path,
        "the live harness refused an adb resolver fallback"
    );

    let control = Arc::new(DeviceControlPlane::new(
        android,
        Arc::new(DeviceWorkCoordinator::new()),
        Arc::new(StreamBudgetManager::new(MAX_INSTALL_CONCURRENCY)?),
    ));

    let roster = match control.list_devices().await {
        Ok(devices) => devices,
        Err(error) => {
            let results = args
                .udids
                .iter()
                .cloned()
                .map(|udid| before_effect(udid, format!("device roster unavailable: {error}")))
                .collect();
            let mut summary = LiveSummary::new(
                args.mode,
                args.package,
                adb_path.display().to_string(),
                results,
            );
            if let Err(error) = control.shutdown_cleanup().await {
                summary.set_cleanup_error(error);
            }
            return Ok(summary);
        }
    };
    let available = Arc::new(
        roster
            .into_iter()
            .filter(|device| {
                !matches!(
                    device.status,
                    DeviceStatus::Disconnected | DeviceStatus::Pairing | DeviceStatus::Error
                )
            })
            .map(|device| device.udid)
            .collect::<HashSet<_>>(),
    );

    let install_request = (args.mode == LiveMode::Install).then(|| {
        Arc::new(DeviceAppInstallRequest {
            apk_paths: args.apk_paths.clone(),
            application_id: args.package.clone(),
            version_name: args
                .version_name
                .clone()
                .expect("the parser requires an install version"),
            version_code: args.version_code.clone(),
            allow_downgrade: args.allow_downgrade,
            effect_gate: None,
        })
    });
    let package = Arc::new(args.package.clone());
    let mut indexed = run_bounded(
        args.udids.into_iter().enumerate(),
        MAX_INSTALL_CONCURRENCY,
        {
            let available = Arc::clone(&available);
            let control = Arc::clone(&control);
            move |(index, udid)| {
                let available = Arc::clone(&available);
                let control = Arc::clone(&control);
                let install_request = install_request.clone();
                let package = Arc::clone(&package);
                async move {
                    if !available.contains(&udid) {
                        return (
                            index,
                            before_effect(udid, "device is not connected and ready for adb"),
                        );
                    }
                    let context = match control
                        .try_acquire_exclusive(&udid, DeviceWorkOwner::Repair)
                        .await
                    {
                        Ok(context) => context,
                        Err(error) => {
                            return (
                                index,
                                before_effect(udid, format!("Repair lease unavailable: {error}")),
                            );
                        }
                    };

                    let result = match install_request {
                        Some(request) => match control
                            .install_app_set_checked(&context, request.as_ref())
                            .await
                        {
                            Ok(result) => result,
                            Err(error) => uncertain(
                                udid,
                                format!("checked install returned no effect verdict: {error}"),
                            ),
                        },
                        None => match control.uninstall_app(&context, package.as_str()).await {
                            Ok(()) => AppInstallResult {
                                udid,
                                status: AppInstallStatus::Succeeded,
                                effect_started: true,
                                observed_version_name: None,
                                observed_version_code: None,
                                detail: Some("explicit cleanup uninstall completed".to_string()),
                            },
                            Err(error) => uncertain(
                                udid,
                                format!("explicit cleanup uninstall was not confirmed: {error}"),
                            ),
                        },
                    };
                    (index, result)
                }
            }
        },
    )
    .await;
    indexed.sort_by_key(|(index, _)| *index);
    let results = indexed.into_iter().map(|(_, result)| result).collect();
    let mut summary = LiveSummary::new(
        args.mode,
        args.package,
        adb_path.display().to_string(),
        results,
    );
    if let Err(error) = control.shutdown_cleanup().await {
        summary.set_cleanup_error(error);
    }
    Ok(summary)
}

fn print_json(value: &impl Serialize) {
    match serde_json::to_string_pretty(value) {
        Ok(json) => println!("{json}"),
        Err(error) => eprintln!("JSON serialization failed: {error}"),
    }
}

#[tokio::main]
async fn main() {
    let args = match parse_args(std::env::args().skip(1)) {
        Ok(args) => args,
        Err(error) => {
            eprintln!(
                "usage: live_app_install_android --udid <serial> [--udid <serial> ...] --package <application-id> (--apk <file.apk> [--apk <split.apk> ...] --version-name <version> [--version-code <code>] [--allow-downgrade] | --uninstall)"
            );
            print_json(&HarnessFailure {
                schema_version: SUMMARY_SCHEMA_VERSION,
                overall: "failed",
                error: error.to_string(),
            });
            std::process::exit(2);
        }
    };

    match execute(args).await {
        Ok(summary) => {
            let exit_code = if summary.is_success() { 0 } else { 2 };
            print_json(&summary);
            std::process::exit(exit_code);
        }
        Err(error) => {
            print_json(&HarnessFailure {
                schema_version: SUMMARY_SCHEMA_VERSION,
                overall: "error",
                error: error.to_string(),
            });
            std::process::exit(3);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    use riviu_core::{AppInstallResult, AppInstallStatus};
    use serde_json::json;

    use super::*;

    fn strings(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_string()).collect()
    }

    #[test]
    fn parser_accepts_repeated_devices_and_explicit_install_identity() {
        let args = parse_args(strings(&[
            "--udid",
            "ONE-01",
            "--udid",
            "ONE-02",
            "--apk",
            "C:\\fixture path\\base.apk",
            "--apk",
            "C:\\fixture path\\split_config.arm64_v8a.apk",
            "--package",
            "com.riviu.fixture",
            "--version-name",
            "1.2.3",
            "--version-code",
            "123",
            "--allow-downgrade",
        ]))
        .expect("valid install arguments");

        assert_eq!(args.mode, LiveMode::Install);
        assert_eq!(args.udids, ["ONE-01", "ONE-02"]);
        assert_eq!(
            args.apk_paths,
            [
                PathBuf::from("C:\\fixture path\\base.apk"),
                PathBuf::from("C:\\fixture path\\split_config.arm64_v8a.apk"),
            ]
        );
        assert_eq!(args.package, "com.riviu.fixture");
        assert_eq!(args.version_name.as_deref(), Some("1.2.3"));
        assert_eq!(args.version_code.as_deref(), Some("123"));
        assert!(args.allow_downgrade);
    }

    #[test]
    fn parser_accepts_explicit_uninstall_cleanup_without_install_flags() {
        let args = parse_args(strings(&[
            "--uninstall",
            "--udid",
            "ONE-01",
            "--package",
            "com.riviu.fixture",
        ]))
        .expect("valid uninstall arguments");

        assert_eq!(args.mode, LiveMode::Uninstall);
        assert!(args.apk_paths.is_empty());
        assert!(args.version_name.is_none());
        assert!(args.version_code.is_none());
        assert!(!args.allow_downgrade);
    }

    #[test]
    fn parser_rejects_incomplete_or_ambiguous_invocations() {
        let cases = [
            strings(&["--package", "com.riviu.fixture", "--version-name", "1"]),
            strings(&[
                "--udid",
                "ONE-01",
                "--apk",
                "base.apk",
                "--package",
                "com.riviu.fixture",
            ]),
            strings(&[
                "--uninstall",
                "--udid",
                "ONE-01",
                "--apk",
                "base.apk",
                "--package",
                "com.riviu.fixture",
            ]),
            strings(&[
                "--udid",
                "ONE-01",
                "--udid",
                "ONE-01",
                "--apk",
                "base.apk",
                "--package",
                "com.riviu.fixture",
                "--version-name",
                "1",
            ]),
            strings(&[
                "--udid",
                "ONE-01",
                "--apk",
                "bundle.apks",
                "--package",
                "com.riviu.fixture",
                "--version-name",
                "1",
            ]),
        ];

        for arguments in cases {
            assert!(parse_args(arguments).is_err());
        }
    }

    #[test]
    fn driver_config_names_only_the_verified_sidecar_adb() {
        let path = PathBuf::from(r"C:\Riviu sidecars\android\win-x86_64\adb.exe");
        let config = exact_adb_config(path.clone());

        assert_eq!(config.adb_path, Some(path));
        assert!(config.bundled_adb_path.is_none());
    }

    #[tokio::test]
    async fn bounded_runner_never_dispatches_more_than_two_devices() {
        let active = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let mut results = run_bounded(vec![1usize, 2, 3, 4, 5], 2, {
            let active = Arc::clone(&active);
            let peak = Arc::clone(&peak);
            move |item| {
                let active = Arc::clone(&active);
                let peak = Arc::clone(&peak);
                async move {
                    let now = active.fetch_add(1, Ordering::SeqCst) + 1;
                    peak.fetch_max(now, Ordering::SeqCst);
                    tokio::time::sleep(Duration::from_millis(10)).await;
                    active.fetch_sub(1, Ordering::SeqCst);
                    item
                }
            }
        })
        .await;
        results.sort_unstable();

        assert_eq!(results, [1, 2, 3, 4, 5]);
        assert_eq!(peak.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn json_summary_preserves_effect_aware_install_results() {
        let results = vec![
            AppInstallResult {
                udid: "ONE-01".to_string(),
                status: AppInstallStatus::Succeeded,
                effect_started: true,
                observed_version_name: Some("1.2.3".to_string()),
                observed_version_code: Some("123".to_string()),
                detail: None,
            },
            AppInstallResult {
                udid: "ONE-02".to_string(),
                status: AppInstallStatus::Uncertain,
                effect_started: true,
                observed_version_name: Some("1.2.2".to_string()),
                observed_version_code: Some("122".to_string()),
                detail: Some("readback did not prove the requested version".to_string()),
            },
        ];
        let summary = LiveSummary::new(
            LiveMode::Install,
            "com.riviu.fixture".to_string(),
            r"C:\Riviu sidecars\android\win-x86_64\adb.exe".to_string(),
            results,
        );

        assert!(!summary.is_success());
        assert_eq!(
            serde_json::to_value(summary).expect("serialize live summary"),
            json!({
                "schemaVersion": 1,
                "operation": "install",
                "package": "com.riviu.fixture",
                "adbPath": r"C:\Riviu sidecars\android\win-x86_64\adb.exe",
                "maxConcurrency": 2,
                "overall": "failed",
                "results": [
                    {
                        "udid": "ONE-01",
                        "status": "succeeded",
                        "effectStarted": true,
                        "observedVersionName": "1.2.3",
                        "observedVersionCode": "123"
                    },
                    {
                        "udid": "ONE-02",
                        "status": "uncertain",
                        "effectStarted": true,
                        "observedVersionName": "1.2.2",
                        "observedVersionCode": "122",
                        "detail": "readback did not prove the requested version"
                    }
                ]
            })
        );
    }
}
