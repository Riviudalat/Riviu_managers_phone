//! Measure Riviu's own `/source` read on a screen that is already open. Read-only.
//!
//! The 45.7.3 sound picker stalled publish on #23/#28 (2026-10-08): every `/source` on the
//! long-lived UiAutomator2 session waited out the server's ~10.5 s root timeout, while a fresh
//! `uiautomator dump` on the same screen read it in ~2.6 s
//! (`target/sound-picker-probe-20261009/REPORT.md`, phone 44). This tool times the read
//! Riviu actually makes, so the fix can be chosen from measurements:
//!
//! - end-to-end [`UiSession::hierarchy_source_snapshot`], the call every sound step makes;
//! - each HTTP attempt inside it (one when pinned; a stale-root replacement adds a second);
//! - response bytes and node count;
//! - whether the tree holds the sound sheet (`:id/fl4` "Bottom sheet"), the editor
//!   (`:id/tv_top_text` or `:id/p31`), both or neither; plus the row/tab/playing markers;
//! - the error text, the focused window and the session epoch after each read.
//!
//! It never taps, swipes, types or presses a key, never runs `uiautomator dump`, and never
//! installs anything. Reads are strictly sequential: the next request starts only after the
//! previous one answered, so a timeout is never followed by an overlapping request.
//!
//! # Running it
//!
//! **Close the Riviu controller first.** The UiAutomator2 server holds one session; opening
//! this tool's session replaces the controller's. Open the editor or the sound picker on the
//! phone by hand, then:
//!
//! ```text
//! cargo run -p riviu-managers-phone --features diagnostics --bin source_probe -- \
//!   --serial ce031713d903001b01 --out target/source-probe/44-picker-playing.jsonl \
//!   --label picker-playing [--reads 8] [--interval-ms 1000] [--unpinned] \
//!   [--setting enableMultiWindows=true]
//! ```
//!
//! - `--label` names the screen state in every line (`editor`, `picker-opened`,
//!   `picker-playing`, ...). The tool does not change the screen, so the operator does.
//! - Reads are pinned by default, as the sound step pins them: an unreadable root stays
//!   unreadable instead of replacing the session. `--unpinned` lets the driver replace the
//!   session once on the stale-root message, as unpinned production reads do.
//! - `--setting name=value` applies ONE UiAutomator2 setting from a short allowlist, reads it
//!   back, and restores the previous value on exit, including after Ctrl+C (the read in flight
//!   finishes first). The value is JSON (`true`, `0`, `30`). If the restore fails the tool
//!   exits with code 3: do not reuse that session. Reopening the controller starts a new
//!   session with default settings (`POST /session`), which discards the change.
//!
//! The adb binary is the packaged one (`sidecars/android/win-x86_64/adb.exe`, or under
//! `RIVIU_SIDECAR_ROOT`), so the client matches the running adb server and never restarts it.
//! The UiAutomator2 runner must already be installed; a stopped runner is started, never
//! installed or replaced.
//!
//! # Output
//!
//! One JSON object per line, appended to `--out`:
//!
//! - `"kind":"start"`: serial, label, pinned, server `/status`, every server setting, runner,
//!   helper and TikTok versions, focused window.
//! - `"kind":"setting"`: name, value before, value requested, value read back.
//! - `"kind":"read"`: `endToEndMs`, `attempts` (`operation`, `elapsedMs`, `outcome`,
//!   `sessionEpoch`), `bytes`, `nodeCount`, `contains` (`soundSheet`/`editor`/`both`/`neither`),
//!   markers, `error`, `focus`, `sessionAgeMs`, `sessionEpoch`.
//! - `"kind":"restore"` and a final `"kind":"summary"` with p50/max of successful reads.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use riviu_core::driver::UiSession;
use riviu_core::tiktok_composer::{with_stage_diagnostics, StageDiagnosticContext};
use serde_json::{json, Value};

const TIKTOK_GLOBAL: &str = "com.zhiliaoapp.musically";
const RUNNER_PACKAGE: &str = "io.appium.uiautomator2.server";
const HELPER_PACKAGE: &str = "com.riviu.agent";
const MAX_READS: u32 = 200;
/// Read-shaping settings only. Ports, MJPEG, power and invisible-element settings change more
/// than how the tree is read, so they are refused.
const ALLOWED_SETTINGS: &[&str] = &[
    "enableTopmostWindowFromActivePackage",
    "enableMultiWindows",
    "snapshotMaxDepth",
    "waitForIdleTimeout",
    "deferAccessibilityCacheReset",
];
const ADB_DEADLINE: Duration = Duration::from_secs(15);

#[derive(Debug)]
struct Args {
    serial: String,
    out: PathBuf,
    label: String,
    reads: u32,
    interval: Duration,
    pinned: bool,
    setting: Option<(String, Value)>,
}

fn parse_args(mut arguments: impl Iterator<Item = String>) -> anyhow::Result<Args> {
    let mut serial = None;
    let mut out = None;
    let mut label = "unlabeled".to_string();
    let mut reads = 8;
    let mut interval = Duration::from_millis(1_000);
    let mut pinned = true;
    let mut setting = None;
    while let Some(argument) = arguments.next() {
        let mut value = |name: &str| {
            arguments
                .next()
                .ok_or_else(|| anyhow::anyhow!("{name} requires a value"))
        };
        match argument.as_str() {
            "--serial" => serial = Some(value("--serial")?),
            "--out" => out = Some(PathBuf::from(value("--out")?)),
            "--label" => label = value("--label")?,
            "--reads" => reads = value("--reads")?.parse()?,
            "--interval-ms" => interval = Duration::from_millis(value("--interval-ms")?.parse()?),
            "--unpinned" => pinned = false,
            "--setting" => {
                anyhow::ensure!(setting.is_none(), "--setting may be given once");
                let raw = value("--setting")?;
                let (name, json_value) = raw
                    .split_once('=')
                    .ok_or_else(|| anyhow::anyhow!("--setting expects name=value"))?;
                anyhow::ensure!(
                    ALLOWED_SETTINGS.contains(&name),
                    "--setting {name} is not allowed; use one of {ALLOWED_SETTINGS:?}"
                );
                setting = Some((name.to_string(), serde_json::from_str(json_value)?));
            }
            other => anyhow::bail!("unknown argument {other}"),
        }
    }
    anyhow::ensure!(
        (1..=MAX_READS).contains(&reads),
        "--reads must be 1..={MAX_READS}"
    );
    Ok(Args {
        serial: serial.ok_or_else(|| anyhow::anyhow!("--serial is required"))?,
        out: out.ok_or_else(|| anyhow::anyhow!("--out is required"))?,
        label,
        reads,
        interval,
        pinned,
        setting,
    })
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

fn unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis().min(u64::MAX as u128) as u64)
        .unwrap_or_default()
}

fn ms(duration: Duration) -> u64 {
    duration.as_millis().min(u64::MAX as u128) as u64
}

struct Output(std::fs::File);

impl Output {
    fn open(path: &Path) -> anyhow::Result<Self> {
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            std::fs::create_dir_all(parent)?;
        }
        Ok(Self(
            std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)?,
        ))
    }

    fn line(&mut self, mut record: Value) -> anyhow::Result<()> {
        if let Some(object) = record.as_object_mut() {
            object.insert("ts".into(), json!(unix_ms()));
        }
        println!("{record}");
        writeln!(self.0, "{record}")?;
        self.0.flush()?;
        Ok(())
    }
}

/// One read-only adb shell command, bounded. A failure is recorded, never retried.
async fn adb_shell(adb: &Path, serial: &str, command: &str) -> Result<String, String> {
    let output = tokio::time::timeout(
        ADB_DEADLINE,
        tokio::process::Command::new(adb)
            .args(["-s", serial, "shell", command])
            .kill_on_drop(true)
            .output(),
    )
    .await
    .map_err(|_| format!("adb shell timed out after {} ms", ms(ADB_DEADLINE)))?
    .map_err(|error| error.to_string())?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

async fn focus(adb: &Path, serial: &str) -> Value {
    let started = Instant::now();
    match adb_shell(adb, serial, "dumpsys window windows").await {
        Ok(dump) => {
            let lines: Vec<&str> = dump
                .lines()
                .map(str::trim)
                .filter(|line| line.starts_with("mCurrentFocus") || line.starts_with("mFocusedApp"))
                .collect();
            json!({ "lines": lines, "ms": ms(started.elapsed()) })
        }
        Err(error) => json!({ "error": error, "ms": ms(started.elapsed()) }),
    }
}

async fn version_name(adb: &Path, serial: &str, package: &str) -> Value {
    match adb_shell(adb, serial, &format!("dumpsys package {package}")).await {
        Ok(dump) => dump
            .lines()
            .map(str::trim)
            .find_map(|line| line.strip_prefix("versionName="))
            .map_or(Value::Null, |version| json!(version)),
        Err(error) => json!({ "error": error }),
    }
}

/// Ids measured on Global 45.7.3/en, phone 44 (SM-G955F), 2026-10-09
/// (`target/sound-picker-probe-20261009/REPORT.md`).
fn tree_markers(xml: &str) -> Value {
    let snapshot = riviu_core::HierarchySourceSnapshot {
        generation: 1,
        xml: xml.to_string(),
    };
    let tree = match riviu_core::ui_automation::tree::Tree::parse(snapshot) {
        Ok(tree) => tree,
        Err(error) => return json!({ "parseError": format!("{error:#}") }),
    };
    let has_id = |suffix: &str| {
        tree.nodes
            .iter()
            .filter(|node| node.attr("resource-id").ends_with(suffix))
            .count()
    };
    let sheet = tree.nodes.iter().any(|node| {
        node.attr("resource-id").ends_with(":id/fl4") && node.attr("content-desc") == "Bottom sheet"
    });
    let editor = has_id(":id/tv_top_text") + has_id(":id/p31") > 0;
    let mut packages: Vec<&str> = tree
        .nodes
        .iter()
        .map(|node| node.attr("package"))
        .filter(|package| !package.is_empty())
        .collect();
    packages.sort_unstable();
    packages.dedup();
    json!({
        "nodeCount": tree.nodes.len(),
        "contains": match (sheet, editor) {
            (true, true) => "both",
            (true, false) => "soundSheet",
            (false, true) => "editor",
            (false, false) => "neither",
        },
        "soundRows": has_id(":id/vertical_item_music_new_rl"),
        "rowContainers": has_id(":id/opr"),
        "tabs": has_id(":id/wrv"),
        "playing": has_id(":id/nms") > 0,
        "packages": packages,
    })
}

fn percentile(sorted: &[u64], fraction: f64) -> Option<u64> {
    if sorted.is_empty() {
        return None;
    }
    let rank = ((sorted.len() as f64) * fraction).ceil() as usize;
    sorted
        .get(rank.saturating_sub(1).min(sorted.len() - 1))
        .copied()
}

async fn restore(
    session: &dyn UiSession,
    agent: &riviu_android_driver::AgentClient,
    output: &mut Output,
    name: &str,
    before: &Value,
) -> anyhow::Result<bool> {
    // An absent setting cannot be restored by writing null; the server would reject it.
    let applied = if before.is_null() {
        Err(anyhow::anyhow!(
            "setting {name} had no previous value to restore"
        ))
    } else {
        agent.update_settings(json!({ name: before })).await
    };
    let readback = agent.settings().await;
    let restored = applied.is_ok()
        && readback
            .as_ref()
            .is_ok_and(|settings| settings.get(name) == Some(before));
    output.line(json!({
        "kind": "restore",
        "name": name,
        "restoredTo": before,
        "readback": readback.as_ref().ok().and_then(|settings| settings.get(name)).cloned(),
        "ok": restored,
        "error": applied.err().or(readback.err()).map(|error| format!("{error:#}")),
        "sessionEpoch": session.gui_session_epoch(),
    }))?;
    Ok(restored)
}

async fn run(args: Args, stop: Arc<AtomicBool>) -> anyhow::Result<bool> {
    let mut output = Output::open(&args.out)?;
    let adb = resolve_sidecar_adb()?;
    let config = riviu_android_driver::AndroidDriverConfig {
        adb_path: Some(adb.clone()),
        bundled_adb_path: None,
        // Strict diagnostics: no install, no helper, no restart of a blind runner. An
        // installed, stopped runner may be started for this serial only.
        automatic_setup_allowed: false,
        diagnostic_runner_devices: vec![args.serial.clone()],
        ..riviu_android_driver::AndroidDriverConfig::default()
    };
    let android = riviu_android_driver::AndroidDriver::new(&config)?;
    anyhow::ensure!(
        Path::new(&android.adb_path()) == adb,
        "the probe refused an adb resolver fallback"
    );
    let session = android.open_session(&args.serial).await?;
    let opened = Instant::now();
    let agent = session.agent().clone();

    let settings_before = agent.settings().await;
    output.line(json!({
        "kind": "start",
        "serial": args.serial,
        "label": args.label,
        "pinned": args.pinned,
        "reads": args.reads,
        "intervalMs": ms(args.interval),
        "sessionEpoch": session.gui_session_epoch(),
        "foreground": session.active_app_bundle().await.map_err(|error| format!("{error:#}")),
        "serverStatus": agent.server_status().await.map_err(|error| format!("{error:#}")),
        "settings": settings_before.as_ref().map_err(|error| format!("{error:#}")),
        "runnerVersion": version_name(&adb, &args.serial, RUNNER_PACKAGE).await,
        "helperVersion": version_name(&adb, &args.serial, HELPER_PACKAGE).await,
        "tiktokVersion": version_name(&adb, &args.serial, TIKTOK_GLOBAL).await,
        "focus": focus(&adb, &args.serial).await,
    }))?;

    let mut applied: Option<(String, Value)> = None;
    if let Some((name, requested)) = &args.setting {
        let before = settings_before
            .as_ref()
            .map_err(|error| anyhow::anyhow!("cannot read settings before change: {error:#}"))?
            .get(name)
            .cloned()
            .unwrap_or(Value::Null);
        anyhow::ensure!(
            !before.is_null(),
            "server reports no setting named {name}; nothing changed"
        );
        let change = agent
            .update_settings(json!({ name.as_str(): requested }))
            .await;
        // Restore whenever the request may have reached the server, even if it errored.
        applied = Some((name.clone(), before.clone()));
        let readback = agent.settings().await;
        let value = readback
            .as_ref()
            .ok()
            .and_then(|settings| settings.get(name))
            .cloned();
        output.line(json!({
            "kind": "setting",
            "name": name,
            "before": before,
            "requested": requested,
            "readback": value,
            "error": change.as_ref().err().or(readback.as_ref().err()).map(|error| format!("{error:#}")),
        }))?;
        if change.is_err() || value.as_ref() != Some(requested) {
            stop.store(true, Ordering::SeqCst);
            eprintln!("setting {name} was not applied as requested; no reads, restoring");
        }
    }

    let pin = args.pinned.then(|| session.pin_read_session());
    let mut successes = Vec::new();
    let mut failures = 0u32;
    let mut epoch = session.gui_session_epoch();
    // Any error inside the loop still reaches the restore below.
    let measured: anyhow::Result<()> = async {
        for index in 0..args.reads {
            if stop.load(Ordering::SeqCst) {
                break;
            }
            let context = StageDiagnosticContext {
                request_id: format!("source-probe-{}", unix_ms()),
                operation_id: format!("read-{index}"),
                udid: args.serial.clone(),
                queue_wait_ms: None,
            };
            let age = opened.elapsed();
            let started = Instant::now();
            let (result, diagnostics) =
                with_stage_diagnostics(context, None, session.hierarchy_source_snapshot()).await;
            let end_to_end = started.elapsed();
            let attempts: Vec<Value> = diagnostics
                .events
                .iter()
                .map(|event| {
                    json!({
                        "operation": event.operation,
                        "elapsedMs": event.elapsed_ms,
                        "startedMs": event.started_ms,
                        "outcome": event.outcome,
                        "sessionEpoch": event.session_epoch,
                    })
                })
                .collect();
            let epoch_after = session.gui_session_epoch();
            let mut record = json!({
                "kind": "read",
                "label": args.label,
                "index": index,
                "pinned": args.pinned,
                "sessionAgeMs": ms(age),
                "endToEndMs": ms(end_to_end),
                "attempts": attempts,
                "sessionEpoch": epoch_after,
                "sessionReplaced": epoch_after != epoch,
            });
            match result {
                Ok(snapshot) => {
                    successes.push(ms(end_to_end));
                    record["ok"] = json!(true);
                    record["bytes"] = json!(snapshot.xml.len());
                    if let (Some(fields), Value::Object(markers)) =
                        (record.as_object_mut(), tree_markers(&snapshot.xml))
                    {
                        fields.extend(markers);
                    }
                }
                Err(error) => {
                    failures += 1;
                    record["ok"] = json!(false);
                    record["error"] = json!(format!("{error:#}"));
                    record["failureKind"] = json!(format!(
                        "{:?}",
                        riviu_core::driver::classify_read_failure(&error)
                    ));
                }
            }
            if epoch_after != epoch {
                // A replacement session starts from server defaults; show what this one holds.
                if let Some((name, _)) = &applied {
                    record["settingAfterReplacement"] = agent
                        .settings()
                        .await
                        .map(|settings| settings.get(name).cloned().unwrap_or(Value::Null))
                        .unwrap_or_else(|error| json!({ "error": format!("{error:#}") }));
                }
                epoch = epoch_after;
            }
            record["focus"] = focus(&adb, &args.serial).await;
            output.line(record)?;
            if index + 1 < args.reads && !stop.load(Ordering::SeqCst) {
                tokio::time::sleep(args.interval).await;
            }
        }
        Ok(())
    }
    .await;
    drop(pin);

    let mut restored = true;
    if let Some((name, before)) = &applied {
        restored = restore(&session, &agent, &mut output, name, before).await?;
    }
    measured?;
    successes.sort_unstable();
    output.line(json!({
        "kind": "summary",
        "label": args.label,
        "ok": successes.len(),
        "failed": failures,
        "p50Ms": percentile(&successes, 0.5),
        "maxMs": successes.last(),
        "restored": restored,
        "stoppedEarly": stop.load(Ordering::SeqCst),
    }))?;
    Ok(restored)
}

#[tokio::main]
async fn main() {
    let args = match parse_args(std::env::args().skip(1)) {
        Ok(args) => args,
        Err(error) => {
            eprintln!(
                "usage: source_probe --serial <udid> --out <file.jsonl> [--label <state>] \
                 [--reads <n>] [--interval-ms <ms>] [--unpinned] [--setting <name>=<json>]\n{error:#}"
            );
            std::process::exit(2);
        }
    };
    // Ctrl+C stops after the read in flight, so the restore never overlaps a request.
    let stop = Arc::new(AtomicBool::new(false));
    let signal = stop.clone();
    tokio::spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            eprintln!("stopping after the current read; settings will be restored");
            signal.store(true, Ordering::SeqCst);
        }
    });
    match run(args, stop).await {
        Ok(true) => {}
        Ok(false) => {
            eprintln!(
                "RESTORE FAILED: this UiAutomator2 session keeps the changed setting. Do not reuse \
                 it; reopening the Riviu controller starts a new session with default settings."
            );
            std::process::exit(3);
        }
        Err(error) => {
            eprintln!("source_probe failed: {error:#}");
            std::process::exit(1);
        }
    }
}
