use std::path::PathBuf;

use app_lib::deployment_check::{
    exit_code_for, parse_args, resolve_install_layout, CheckStatus, DeploymentProfile,
    DeploymentReport,
};

#[test]
fn internal_profile_accepts_unsigned_installer_as_warning() {
    let report = DeploymentReport::fixture_with_authenticode(CheckStatus::Warning);

    assert_eq!(
        report.overall_for(DeploymentProfile::Internal),
        CheckStatus::Warning
    );
    assert_eq!(exit_code_for(&report, DeploymentProfile::Internal), 0);
}

#[test]
fn production_profile_rejects_unsigned_installer() {
    let report = DeploymentReport::fixture_with_authenticode(CheckStatus::Warning);

    assert_eq!(
        report.overall_for(DeploymentProfile::Production),
        CheckStatus::Fail
    );
    assert_eq!(exit_code_for(&report, DeploymentProfile::Production), 2);
}

#[test]
fn production_profile_requires_installer_evidence_even_when_executables_are_signed() {
    let mut report = DeploymentReport::fixture_with_authenticode(CheckStatus::Pass);
    report.installer_path = None;
    report.installer_sha256 = None;

    assert_eq!(
        report.overall_for(DeploymentProfile::Production),
        CheckStatus::Fail
    );
    assert_eq!(exit_code_for(&report, DeploymentProfile::Production), 2);
}

#[test]
fn production_profile_accepts_signed_installer_evidence() {
    let report = DeploymentReport::fixture_with_authenticode(CheckStatus::Pass);

    assert_eq!(
        report.overall_for(DeploymentProfile::Production),
        CheckStatus::Pass
    );
    assert_eq!(exit_code_for(&report, DeploymentProfile::Production), 0);
}

#[test]
fn production_cli_requires_the_installer_path() {
    let error = parse_args([
        "riviu-deployment-check.exe",
        "--profile",
        "production",
        "--report",
        r"C:\Temp\riviu-deployment.json",
    ])
    .expect_err("production without installer evidence must fail");

    assert!(error.to_string().contains("--installer is required"));
}

#[test]
fn cli_requires_a_profile_and_report_path() {
    let parsed = parse_args([
        "riviu-deployment-check.exe",
        "--profile",
        "internal",
        "--report",
        "C:\\Temp\\riviu-deployment.json",
        "--device-check",
        "serial-1",
    ])
    .expect("valid command line");

    assert_eq!(parsed.profile, DeploymentProfile::Internal);
    assert_eq!(
        parsed.report,
        PathBuf::from(r"C:\Temp\riviu-deployment.json")
    );
    assert_eq!(parsed.device_serial.as_deref(), Some("serial-1"));
}

#[test]
fn installed_sidecar_layout_resolves_resources_beside_the_checker() {
    let executable =
        PathBuf::from(r"C:\Users\operator\AppData\Local\Riviu Manager\riviu-deployment-check.exe");
    let layout = resolve_install_layout(&executable).expect("installed layout");

    assert_eq!(
        layout.sidecars_root,
        PathBuf::from(r"C:\Users\operator\AppData\Local\Riviu Manager\sidecars")
    );
    assert_eq!(
        layout.adb,
        PathBuf::from(
            r"C:\Users\operator\AppData\Local\Riviu Manager\sidecars\android\win-x86_64\adb.exe"
        )
    );
}

#[test]
fn report_schema_uses_stable_contract_status_values() {
    let report = DeploymentReport::fixture_with_authenticode(CheckStatus::Pass);
    let value = serde_json::to_value(report).expect("serialize report");

    assert_eq!(value["schemaVersion"], 1);
    assert_eq!(value["host"]["name"], "Windows");
    assert!(value["host"]["build"].is_string());
    assert!(value["appSha256"].is_string());
    assert!(value["checkerSha256"].is_string());
    assert_eq!(value["checks"]["authenticode"]["status"], "pass");
    assert_eq!(value["checks"]["androidPackageTools"]["status"], "pass");
    assert_eq!(value["checks"]["deviceState"]["status"], "not_applicable");
    assert!(value["checks"]["databaseMigration"]["detail"].is_string());
}
