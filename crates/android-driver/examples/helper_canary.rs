//! Exact-scope helper canary. No TikTok input or public effects.
use anyhow::{ensure, Context};
use riviu_android_driver::{AdbProgram, AndroidDriver};
use riviu_core::{DeviceControlPlane, DeviceWorkCoordinator, DeviceWorkOwner, StreamBudgetManager};
use serde_json::json;
use std::{path::PathBuf, sync::Arc};
#[path = "common/mod.rs"]
mod common;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args: Vec<_> = std::env::args().collect();
    ensure!(
        args.len() == 6
            && matches!(
                args[1].as_str(),
                "--approved-helper-only"
                    | "--approved-helper-runtime"
                    | "--probe-helper-only"
                    | "--reconcile-helper-readonly"
                    | "--inspect-helper-owner"
                    | "--diagnose-helper-bootstrap"
                    | "--recover-helper-canary"
            ),
        "helper_canary --approved-helper-only|--probe-helper-only SERIAL APK SHA256 REPORT"
    );
    let serial = &args[2];
    let apk = PathBuf::from(&args[3]);
    let report = PathBuf::from(&args[5]);
    ensure!(
        report.is_absolute() && !report.exists(),
        "fresh absolute report required"
    );
    let bytes = std::fs::read(&apk)?;
    ensure!(
        riviu_core::frame_sha256(&bytes) == args[4],
        "candidate hash changed"
    );
    std::fs::create_dir_all(&report)?;
    // Stable device fence survives a crashed canary; never retry an uncertain install/claim.
    let fence_root = PathBuf::from(
        std::env::var_os("LOCALAPPDATA").context("stable helper fence root unavailable")?,
    )
    .join("riviu-helper-canary-fences");
    std::fs::create_dir_all(&fence_root)?;
    let fence = fence_root.join(format!(
        "{}.json",
        riviu_core::frame_sha256(serial.as_bytes())
    ));
    use std::io::Write;
    let reconcile = matches!(
        args[1].as_str(),
        "--reconcile-helper-readonly"
            | "--inspect-helper-owner"
            | "--diagnose-helper-bootstrap"
            | "--recover-helper-canary"
    );
    if reconcile {
        ensure!(
            fence.is_file(),
            "read-only reconciliation requires existing device fence"
        );
    } else {
        let mut fence_file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&fence)
            .context("device canary fence exists; reconciliation required")?;
        fence_file.write_all(&serde_json::to_vec(
            &json!({"serial":serial,"report":report,"publicEffectsAllowed":false}),
        )?)?;
        fence_file.sync_all()?;
    }
    // Refuse a concurrent desktop controller. Do not stop it or claim its device leases.
    #[cfg(windows)]
    {
        let output = std::process::Command::new("powershell.exe").args(["-NoProfile","-Command", "$ErrorActionPreference='Stop'; @(Get-Process | Where-Object {$_.ProcessName -eq 'riviu-managers-phone'}).Count"]).output()?;
        ensure!(
            output.status.success() && String::from_utf8(output.stdout)?.trim() == "0",
            "desktop controller present or inventory unknown"
        );
    }
    let mut config = common::repo_config();
    config.automatic_setup_allowed = false;
    if args[1] == "--approved-helper-runtime" {
        config.helper_state_dir = Some(report.join("runtime-state"));
        config.riviu_agent_apk = Some(apk.clone());
    }
    let driver = Arc::new(AndroidDriver::new(&config)?);
    let control = Arc::new(DeviceControlPlane::new(
        driver.clone(),
        Arc::new(DeviceWorkCoordinator::new()),
        Arc::new(StreamBudgetManager::new(1)?),
    ));
    ensure!(
        control
            .list_devices()
            .await?
            .iter()
            .any(|d| d.udid == *serial),
        "scoped phone absent"
    );
    let lease = control
        .try_acquire_exclusive_keeping_stream(serial, DeviceWorkOwner::Repair)
        .await?;
    let adb = AdbProgram::at(PathBuf::from(driver.adb_path()));
    if args[1] == "--recover-helper-canary" {
        ensure!(
            serial == "ce021712aaf9533405",
            "recovery authorization exact canary only"
        );
        let old: serde_json::Value = serde_json::from_slice(&std::fs::read(&fence)?)?;
        let old_path = PathBuf::from(
            old["report"]
                .as_str()
                .context("prior canary receipt missing")?,
        );
        let baseline: serde_json::Value =
            serde_json::from_slice(&std::fs::read(old_path.join("baseline.json"))?)?;
        ensure!(
            old["serial"].as_str() == Some(serial.as_str())
                && baseline["serial"].as_str() == Some(serial.as_str()),
            "recovery baseline belongs to different device"
        );
        ensure!(
            adb.shell(serial, "am get-current-user").await?.trim() == "0",
            "recovery Android user changed"
        );
        ensure!(
            !old_path.join("ime-checkpoint.json").exists(),
            "IME mutation checkpoint unresolved; helper recovery refused"
        );
        let current = adb
            .shell(serial, "settings get secure default_input_method")
            .await?;
        let enabled = adb
            .shell(serial, "settings get secure enabled_input_methods")
            .await?;
        riviu_android_driver::riviu_agent::validate_ime_id(current.trim())?;
        ensure!(
            current.trim() != riviu_android_driver::riviu_agent::IME_ID,
            "recovery baseline already helper IME"
        );
        ensure!(
            baseline["defaultIme"].as_str() == Some(current.trim())
                && baseline["enabledIme"].as_str() == Some(enabled.trim()),
            "IME baseline changed; recovery refused"
        );
        let before = adb.shell(serial, "pidof com.riviu.agent || true").await?;
        let stopped = control
            .device_shell(&lease, "am force-stop com.riviu.agent")
            .await?;
        ensure!(stopped.exit_code == 0, "scoped helper stop rejected");
        let after = adb
            .shell_output(
                serial,
                "pidof com.riviu.agent",
                std::time::Duration::from_secs(10),
            )
            .await?;
        ensure!(
            matches!(after.exit_code, 0 | 1)
                && after.stdout.trim().is_empty()
                && after.stderr.trim().is_empty(),
            "helper process absence not verified"
        );
        ensure!(
            adb.shell(serial, "settings get secure default_input_method")
                .await?
                .trim()
                == current.trim()
                && adb
                    .shell(serial, "settings get secure enabled_input_methods")
                    .await?
                    .trim()
                    == enabled.trim(),
            "IME baseline changed during helper recovery"
        );
        std::fs::write(
            report.join("recovery.json"),
            serde_json::to_vec_pretty(
                &json!({"serial":serial,"operatorApproved":true,"scope":"com.riviu.agent only","priorPid":before.trim(),"helperStoppedVerified":true,"imeUnchanged":true,"oldEvidence":old_path,"publicEffectsAllowed":false}),
            )?,
        )?;
        control.close_exclusive_context(lease)?;
        control.shutdown_cleanup().await?;
        std::fs::rename(&fence, report.join("prior-device-fence.json"))?;
        println!("exact helper recovery confirmed; old evidence preserved");
        return Ok(());
    }
    if args[1] == "--diagnose-helper-bootstrap" {
        let jar = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../sidecars/riviu-android-agent/build/probe/probe-bootstrap.jar");
        let value = riviu_android_driver::HelperClient::diagnose_bootstrap_readonly(
            &adb,
            serial,
            &jar,
            "3ee2b4b6feaaa540cd33561e654f421bf9b637696490d81b20a388f8337d98d4",
        )
        .await;
        std::fs::write(
            report.join("bootstrap-diagnosis.json"),
            serde_json::to_vec_pretty(
                &json!({"reply":value.as_ref().ok(),"error":value.as_ref().err().map(|e|format!("{e:#}")),"claimSent":false,"rekeySent":false,"ownedProbeJarCleanupAttempted":true,"fenceRetained":true}),
            )?,
        )?;
        control.close_exclusive_context(lease)?;
        control.shutdown_cleanup().await?;
        value?;
        return Ok(());
    }
    if args[1] == "--inspect-helper-owner" {
        let value =
            riviu_android_driver::HelperClient::inspect_bootstrap_owner_readonly(&adb, serial)
                .await;
        std::fs::write(
            report.join("owner-inspect.json"),
            serde_json::to_vec_pretty(
                &json!({"reply":value.as_ref().ok(),"error":value.as_ref().err().map(ToString::to_string),"claimSent":false,"rekeySent":false,"fenceRetained":true}),
            )?,
        )?;
        control.close_exclusive_context(lease)?;
        control.shutdown_cleanup().await?;
        value?;
        println!("owner observation received; no ownership mutation, fence retained");
        return Ok(());
    }
    if reconcile {
        let default = adb
            .shell(serial, "settings get secure default_input_method")
            .await?;
        let enabled = adb
            .shell(serial, "settings get secure enabled_input_methods")
            .await?;
        let package = adb.shell(serial, "dumpsys package com.riviu.agent").await?;
        let processes = adb.shell(serial, "pidof com.riviu.agent || true").await?;
        let log = adb
            .shell(
                serial,
                "logcat -d -v brief -s RiviuHelper:V AndroidRuntime:E -t 200",
            )
            .await?;
        let listeners = adb.shell(serial, "cat /proc/net/unix").await?;
        let services = adb
            .shell(serial, "dumpsys activity services com.riviu.agent")
            .await?;
        let service_lines: Vec<_> = services
            .lines()
            .filter(|line| {
                line.contains("ServiceRecord{")
                    || line.contains("startRequested=")
                    || line.contains("lastStartId=")
                    || line.contains("ProcessRecord{")
                    || line.contains("bootstrapSocket")
                    || line.contains("intentDelivered=")
            })
            .map(str::to_owned)
            .collect();
        let pid = processes.trim();
        let thread_states = if !pid.is_empty() && pid.chars().all(|c| c.is_ascii_digit()) {
            adb.shell(serial, &format!("for t in /proc/{pid}/task/*; do printf '%s ' \"$t\"; cat \"$t/comm\" \"$t/wchan\" 2>/dev/null; done")).await.unwrap_or_default()
        } else {
            String::new()
        };
        let helper_listeners: Vec<_> = listeners
            .lines()
            .filter(|line| line.contains("riviu-bootstrap-"))
            .map(str::to_owned)
            .collect();
        let paths = adb.shell(serial, "pm path com.riviu.agent").await?;
        let apk_path = paths
            .lines()
            .find_map(|line| line.strip_prefix("package:"))
            .context("helper path missing")?;
        riviu_android_driver::adb::validate_device_path(apk_path)?;
        // Invalid arguments terminate before connecting or consuming credentials. This tests
        // actual DEX/classloading only; no bootstrap claim or device setup is repeated.
        let classload = adb.shell_output(serial, &format!("CLASSPATH={} app_process /system/bin com.riviu.agent.BootstrapBridge invalid 0", riviu_android_driver::adb::quote_device_path(apk_path)), std::time::Duration::from_secs(15)).await?;
        let stdin_probe =
            riviu_android_driver::HelperClient::probe_stdin_transport(&adb, serial).await;
        let old_report: serde_json::Value = serde_json::from_slice(&std::fs::read(&fence)?)?;
        let old_path = PathBuf::from(
            old_report["report"]
                .as_str()
                .context("old canary report missing")?,
        );
        let old_intent: serde_json::Value =
            serde_json::from_slice(&std::fs::read(old_path.join("helper-claim-intent.json"))?)?;
        let bootstrap_read = if stdin_probe.is_ok() && !helper_listeners.is_empty() {
            riviu_android_driver::HelperClient::inspect_existing_bootstrap(
                &adb,
                serial,
                &old_intent,
            )
            .await
        } else {
            Err(anyhow::anyhow!(
                "transport or listener not proven; inspect not sent"
            ))
        };
        let classload_reason = if classload.stderr.contains("ClassNotFoundException")
            || classload.stdout.contains("ClassNotFoundException")
        {
            "classNotFound"
        } else if classload.stderr.contains("Permission denied")
            || classload.stdout.contains("Permission denied")
        {
            "permissionDenied"
        } else if classload.stderr.contains("bootstrap_failed")
            || classload.stdout.contains("bootstrap_failed")
        {
            "bridgeLoadedArgumentsRefused"
        } else {
            "unclassified"
        };
        std::fs::write(
            report.join("readonly.json"),
            serde_json::to_vec_pretty(
                &json!({"serial":serial,"defaultIme":default.trim(),"enabledIme":enabled.trim(),"helperVersion6":package.lines().any(|line|line.trim().starts_with("versionCode=6 ")),"helperPid":processes.trim(),"log":log,"bootstrapListeners":helper_listeners,"serviceLines":service_lines,"threadStates":thread_states,"stdinRoundTrip":stdin_probe.is_ok(),"stdinError":stdin_probe.err().map(|e|e.to_string()),"bootstrapRead":bootstrap_read.as_ref().ok(),"bootstrapReadError":bootstrap_read.as_ref().err().map(ToString::to_string),"bridgeProbe":{"exitCode":classload.exit_code,"reason":classload_reason,"stdoutBytes":classload.stdout.len(),"stderrBytes":classload.stderr.len()},"fencePreserved":true,"mutations":0}),
            )?,
        )?;
        control.close_exclusive_context(lease)?;
        control.shutdown_cleanup().await?;
        println!("read-only helper reconciliation captured; device fence preserved");
        return Ok(());
    }
    let baseline = async {
        ensure!(adb.shell(serial, "id -u").await?.trim() == "2000", "canary excludes root ADB UID; no root fallback");
        let user = adb.shell(serial, "am get-current-user").await?;
        ensure!(user.trim() == "0", "canary requires Android user0");
        let power = adb.shell(serial, "dumpsys power").await?;
        let window = adb.shell(serial, "dumpsys window").await?;
        ensure!(riviu_android_driver::adb::parse_display_awake(&power) == Some(true)
            && riviu_android_driver::adb::parse_keyguard_locked(&window) == Some(false), "phone awake/unlocked proof missing; no unlock");
        let before = adb.shell(serial, "settings get secure default_input_method").await?;
        riviu_android_driver::riviu_agent::validate_ime_id(before.trim())?;
        ensure!(before.trim() != riviu_android_driver::riviu_agent::IME_ID, "helper already default; reconcile old IME before install");
        let enabled = adb.shell(serial, "settings get secure enabled_input_methods").await?;
        std::fs::write(report.join("baseline.json"), serde_json::to_vec_pretty(&json!({"serial":serial,"defaultIme":before.trim(),"enabledIme":enabled.trim(),"publicEffectsAllowed":false}))?)?;
        Ok::<_,anyhow::Error>((before,enabled))
    }.await;
    let (before, enabled) = match baseline {
        Ok(value) => value,
        Err(error) => {
            control.close_exclusive_context(lease)?;
            control.shutdown_cleanup().await?;
            std::fs::remove_file(&fence)?;
            return Err(error);
        }
    };
    if args[1] == "--probe-helper-only" {
        control.close_exclusive_context(lease)?;
        control.shutdown_cleanup().await?;
        std::fs::remove_file(&fence)?;
        println!("helper probe eligible: {}", serial);
        return Ok(());
    }
    std::fs::write(
        report.join("intent.json"),
        serde_json::to_vec_pretty(
            &json!({"serial":serial,"candidateSha256":args[4],"stage":"beforeInstall","publicEffectsAllowed":false}),
        )?,
    )?;
    let work = async {
        driver.install_helper_canary(serial, &apk).await?;
        let package = adb.shell(serial, "dumpsys package com.riviu.agent").await?;
        let runtime = args[1] == "--approved-helper-runtime";
        let version = if runtime { "versionCode=7 " } else { "versionCode=6 " };
        ensure!(package.lines().any(|line|line.trim().starts_with(version)), "candidate version readback mismatched");
        let helper = if runtime {
            driver.prepare_helper_runtime(serial).await?
        } else {
            driver.prepare_helper_canary(serial, report.clone()).await?
        };
        // Read-only clipboard canary: no clipboard replacement or TikTok navigation.
        let read = helper.get_clipboard(4096).await;
        let verify = async {
            let after = adb.shell(serial, "settings get secure default_input_method").await?;
            let after_enabled = adb.shell(serial, "settings get secure enabled_input_methods").await?;
            ensure!(before.trim() == after.trim() && enabled.trim() == after_enabled.trim(), "IME baseline restoration mismatch");
            ensure!(!report.join("ime-checkpoint.json").exists(), "IME recovery checkpoint unresolved");
            Ok::<_,anyhow::Error>(())
        }.await;
        // Always attempt known-owned settlement. Driver cache retains a clone on error.
        let released = helper.shutdown().await;
        verify?;
        released.context("owned helper release failed")?;
        std::fs::write(report.join("receipt.json"), serde_json::to_vec_pretty(&json!({"serial":serial,"runtimeBootstrap":runtime,"candidateInstalled":true,"bootstrapVerified":true,"clipboardReadSucceeded":read.is_ok(),"clipboardBytes":read.as_ref().ok().map(|(_,bytes)|bytes.len()),"clipboardError":read.as_ref().err().map(ToString::to_string),"imeRestored":true,"ownedReleaseVerified":true,"publicEffectsAllowed":false}))?)?;
        read?;
        if runtime && std::env::var_os("RIVIU_HELPER_VERIFY_REATTACH").as_deref() == Some(std::ffi::OsStr::new("1")) {
            let second = driver.prepare_helper_runtime(serial).await.context("same-driver released helper reattach failed")?;
            let second_read = second.get_clipboard(4096).await;
            let second_release = second.shutdown().await;
            second_release.context("second owner release failed")?;
            second_read?;
            ensure!(adb.shell(serial, "settings get secure default_input_method").await?.trim() == before.trim(), "second owner IME restore mismatch");
            ensure!(adb.shell(serial, "settings get secure enabled_input_methods").await?.trim() == enabled.trim(), "second owner IME enablement restore mismatch");
            std::fs::write(report.join("reattach.json"), serde_json::to_vec_pretty(&json!({"serial":serial,"sameDriver":true,"preparedTwice":true,"bothOwnersReleased":true,"imeRestored":true,"publicEffectsAllowed":false}))?)?;
        }
        Ok::<_,anyhow::Error>(())
    }.await;
    match work {
        Ok(()) => {
            control.close_exclusive_context(lease)?;
            control.shutdown_cleanup().await?;
            std::fs::remove_file(&fence)?;
            println!("helper canary prepared/read/restored/released: {}", serial);
            Ok(())
        }
        Err(error) => {
            control.quarantine_exclusive_context(lease)?;
            std::fs::write(
                report.join("needs-attention.json"),
                serde_json::to_vec_pretty(
                    &json!({"serial":serial,"error":error.to_string(),"retryAllowed":false,"publicEffectsAllowed":false}),
                )?,
            )?;
            eprintln!(
                "helper canary blocked; exact scoped evidence retained at {}",
                report.display()
            );
            Err(error)
        }
    }
}
