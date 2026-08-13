//! Measure how an image gets into an Android device's media library.
//!
//! There is **no prior art for this in the repo** — a repo-wide search for
//! `MediaStore`, `MEDIA_SCANNER_SCAN_FILE` or `content://media` finds nothing but a
//! shell-injection example. So the Android publish pipeline cannot be designed yet,
//! only measured, and this is the measurement.
//!
//! The one `am broadcast` data point the project already has is a **negative** one
//! (`docs/ANDROID_PROBE_REPORT_2026-08-09.md`): `am broadcast -a ADB_INPUT_TEXT`
//! returned `result=0` while the input field did not change. That is exactly the trap
//! waiting here — **`result=0` proves nothing**. Every claim below is checked by
//! querying MediaStore, never by a broadcast's exit code.
//!
//! ```text
//! RIVIU_ADB_PATH=…/adb.exe \
//!   cargo run -p riviu-android-driver --example media_probe -- <serial>
//! ```
//!
//! **What it does to the phone**, so nobody is surprised: pushes up to three small
//! PNGs named `riviu-media-probe-*.png` into candidate public directories, asks
//! MediaStore whether it can see them, then deletes them and verifies the deletion.
//! The image content is a screenshot of the phone's own screen, so nothing new is
//! introduced. Pass `--keep` to skip the cleanup when a failure needs inspecting.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use riviu_android_driver::AdbProgram;

/// Where a pushed image might become visible to other apps.
///
/// Ordered by how plausible each is for a photo an app should be able to pick: the
/// camera folder first, then the general Pictures tree. `Movies` is included because
/// the same question will be asked for video, and answering it now costs one push.
const CANDIDATES: [&str; 3] = ["/sdcard/DCIM/Camera", "/sdcard/Pictures", "/sdcard/Movies"];

/// A name nothing else writes, so a query can find exactly our rows and a cleanup
/// cannot touch the operator's own photos.
const NAME_PREFIX: &str = "riviu-media-probe";

const IMAGES_URI: &str = "content://media/external/images/media";

struct Probe {
    adb: AdbProgram,
    serial: String,
}

impl Probe {
    async fn shell(&self, script: &str) -> anyhow::Result<String> {
        self.adb.shell(&self.serial, script).await
    }

    /// Every MediaStore row whose path contains our prefix.
    ///
    /// This is the **evidence**. A push that "succeeded" and a broadcast that returned
    /// `result=0` both prove nothing about whether another app can see the file.
    async fn rows(&self) -> Vec<String> {
        let query = format!(
            "content query --uri {IMAGES_URI} --projection _id:_data:date_added \
             --where \"_data LIKE '%{NAME_PREFIX}%'\""
        );
        match self.shell(&query).await {
            Ok(stdout) => stdout
                .lines()
                .map(str::trim)
                .filter(|line| line.contains(NAME_PREFIX))
                .map(str::to_string)
                .collect(),
            Err(error) => {
                println!("    ! content query failed: {error}");
                Vec::new()
            }
        }
    }

    /// `_id` values out of `content query` output.
    ///
    /// The rows read `Row: 0 _id=123, _data=/sdcard/…, date_added=…`.
    fn ids(rows: &[String]) -> Vec<String> {
        rows.iter()
            .filter_map(|row| {
                let after = row.split("_id=").nth(1)?;
                let id: String = after.chars().take_while(char::is_ascii_digit).collect();
                (!id.is_empty()).then_some(id)
            })
            .collect()
    }

    async fn push(&self, local: &std::path::Path, remote: &str) -> anyhow::Result<()> {
        self.adb
            .device(
                &self.serial,
                &["push", &local.display().to_string(), remote],
                Duration::from_secs(120),
            )
            .await
            .map(|_| ())
    }

    /// Ask the media scanner to look at one file, and say what the broadcast claimed.
    ///
    /// The return value is **only** the broadcast's own words. Whether it worked is
    /// decided by [`Self::rows`] afterwards.
    async fn scan(&self, remote: &str) -> String {
        let script = format!(
            "am broadcast -a android.intent.action.MEDIA_SCANNER_SCAN_FILE -d file://{remote}"
        );
        match self.shell(&script).await {
            Ok(stdout) => stdout.trim().replace('\n', " "),
            Err(error) => format!("<failed: {error}>"),
        }
    }

    async fn delete_rows(&self) -> anyhow::Result<String> {
        let script =
            format!("content delete --uri {IMAGES_URI} --where \"_data LIKE '%{NAME_PREFIX}%'\"");
        self.shell(&script).await
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let keep = args.iter().any(|arg| arg == "--keep");
    let adb = AdbProgram::resolve(None, None)?;
    let serial = match args.iter().find(|arg| !arg.starts_with("--")) {
        Some(serial) => serial.clone(),
        None => {
            let listing = adb.run(&["devices"], Duration::from_secs(20)).await?;
            listing
                .lines()
                .skip(1)
                .find_map(|line| {
                    let mut parts = line.split_whitespace();
                    let serial = parts.next()?;
                    (parts.next() == Some("device")).then(|| serial.to_string())
                })
                .ok_or_else(|| anyhow::anyhow!("no Android device attached"))?
        }
    };
    let probe = Probe {
        adb: adb.clone(),
        serial: serial.clone(),
    };
    println!("device {serial}");

    // A real, valid image with no encoder dependency: the phone's own screen.
    println!("\n== source image ==");
    let png = adb
        .device_bytes(
            &serial,
            &["exec-out", "screencap", "-p"],
            Duration::from_secs(60),
        )
        .await?;
    anyhow::ensure!(
        png.starts_with(&[0x89, b'P', b'N', b'G']),
        "screencap did not return a PNG ({} bytes)",
        png.len()
    );
    let local: PathBuf = std::env::temp_dir().join(format!("{NAME_PREFIX}-source.png"));
    tokio::fs::write(&local, &png).await?;
    println!("  {} bytes -> {}", png.len(), local.display());

    println!("\n== MediaStore before ==");
    let before = probe.rows().await;
    println!("  {} row(s) matching {NAME_PREFIX}", before.len());
    if !before.is_empty() {
        println!("  ! leftovers from an earlier run; cleaning first");
        let _ = probe.delete_rows().await;
    }

    // M2/M3: which directory works, and does a push alone make the file visible?
    println!("\n== push, then ask MediaStore (not the push's exit code) ==");
    let mut visible_without_scan: Vec<&str> = Vec::new();
    let mut visible_after_scan: Vec<&str> = Vec::new();
    let mut pushed: Vec<String> = Vec::new();
    for (index, directory) in CANDIDATES.iter().enumerate() {
        let remote = format!("{directory}/{NAME_PREFIX}-{index}.png");
        print!("  {remote} ... ");
        if let Err(error) = probe.push(&local, &remote).await {
            println!("push failed: {error}");
            continue;
        }
        pushed.push(remote.clone());
        // A file on disk is not a file in MediaStore. Give the automatic scanner a
        // moment before concluding it will not act.
        tokio::time::sleep(Duration::from_millis(1_500)).await;
        let seen = probe
            .rows()
            .await
            .iter()
            .any(|row| row.contains(&format!("{NAME_PREFIX}-{index}.png")));
        if seen {
            println!("visible WITHOUT a scan");
            visible_without_scan.push(directory);
            continue;
        }
        let claim = probe.scan(&remote).await;
        tokio::time::sleep(Duration::from_millis(1_500)).await;
        let seen_now = probe
            .rows()
            .await
            .iter()
            .any(|row| row.contains(&format!("{NAME_PREFIX}-{index}.png")));
        if seen_now {
            println!("visible after MEDIA_SCANNER_SCAN_FILE (broadcast said: {claim})");
            visible_after_scan.push(directory);
        } else {
            println!("NOT visible even after the scan (broadcast said: {claim})");
        }
    }

    println!("\n== what MediaStore actually holds now ==");
    let rows = probe.rows().await;
    for row in &rows {
        println!("  {row}");
    }
    let ids = Probe::ids(&rows);
    println!("  {} row(s), {} with a readable _id", rows.len(), ids.len());

    // M6: does the order survive? A carousel is ordered, so this decides whether the
    // import can rely on MediaStore ordering at all.
    println!("\n== ordering ==");
    if rows.len() >= 2 {
        let order: Vec<&str> = rows
            .iter()
            .filter_map(|row| {
                row.split("_data=")
                    .nth(1)?
                    .split(',')
                    .next()?
                    .rsplit('/')
                    .next()
            })
            .collect();
        println!("  query order: {}", order.join(" -> "));
        println!(
            "  pushed order: {}",
            pushed
                .iter()
                .filter_map(|remote| remote.rsplit('/').next())
                .collect::<Vec<_>>()
                .join(" -> ")
        );
        println!("  (if these differ, the import must set its own order, not trust MediaStore)");
    } else {
        println!("  fewer than two rows; ordering not measurable in this run");
    }

    // M5: is the cleanup idempotent? The publish contract calls cleanup twice in its
    // retry path and asserts `state == "cleaned"` both times.
    if keep {
        println!("\n== cleanup skipped (--keep) ==");
        println!("  {} file(s) left on the device:", pushed.len());
        for remote in &pushed {
            println!("    {remote}");
        }
        return Ok(());
    }

    println!("\n== cleanup, twice ==");
    let started = Instant::now();
    let first = probe.delete_rows().await.unwrap_or_default();
    println!("  content delete #1: {}", first.trim());
    for remote in &pushed {
        let _ = probe.shell(&format!("rm -f {remote}")).await;
    }
    let after_first = probe.rows().await;
    println!("  rows after #1: {}", after_first.len());
    let second = probe.delete_rows().await.unwrap_or_default();
    println!("  content delete #2: {}", second.trim());
    let after_second = probe.rows().await;
    println!(
        "  rows after #2: {} ({} ms total)",
        after_second.len(),
        started.elapsed().as_millis()
    );
    // The local source is deliberately NOT deleted here: the contract gate below reads
    // it. Deleting it at this point is what made the gate fail with a local os error 2,
    // which reads like a device problem and is not one.

    // Stage + import and stop, so the picker can be inspected while the files are
    // still there. Prints the importId; `--cleanup <id>` removes it afterwards.
    if args.iter().any(|arg| arg == "--leave-imported") {
        println!("\n== stage + import, leaving the files in place ==");
        match leave_imported(&adb, &serial, &local).await {
            Ok(id) => println!("  importId={id}\n  run again with --cleanup {id} to remove it"),
            Err(error) => println!("  FAILED: {error:#}"),
        }
        return Ok(());
    }
    if let Some(id) = args
        .iter()
        .position(|arg| arg == "--cleanup")
        .and_then(|at| args.get(at + 1))
    {
        println!("\n== cleanup {id} ==");
        match riviu_android_driver::publish::cleanup(&adb, &serial, id).await {
            Ok(value) => println!("  {value}"),
            Err(error) => println!("  FAILED: {error:#}"),
        }
        return Ok(());
    }

    // The four contract steps, through the shipped code rather than through shell
    // commands. This is the part that proves `crate::publish` works, not just that
    // MediaStore behaves.
    if args.iter().any(|arg| arg == "--contract") {
        println!("\n== the publish contract, through crate::publish ==");
        match exercise_contract(&adb, &serial, &local).await {
            Ok(()) => println!("  contract gate passed"),
            Err(error) => println!("  contract gate FAILED: {error:#}"),
        }
    } else {
        println!(
            "\n(skipping the contract gate; pass --contract to run stage/prepare/import/cleanup)"
        );
    }

    println!("\n== summary ==");
    println!(
        "  visible without a scan: {}",
        if visible_without_scan.is_empty() {
            "none".to_string()
        } else {
            visible_without_scan.join(", ")
        }
    );
    println!(
        "  visible only after a scan: {}",
        if visible_after_scan.is_empty() {
            "none".to_string()
        } else {
            visible_after_scan.join(", ")
        }
    );
    println!("  cleanup left {} row(s)", after_second.len());
    if visible_without_scan.is_empty() && visible_after_scan.is_empty() {
        println!(
            "  => a plain `adb push` does NOT get an image into this phone's media library. \
             The publish import needs another mechanism (a MediaStore insert, or an on-device \
             helper), and that is a design decision, not a detail."
        );
    }
    println!(
        "\n  MEASURED (11/08/2026, Redmi Note 12): MediaStore visibility is necessary and \
         NOT sufficient. A row an `adb push` created carries is_pending=1, and TikTok's \
         picker lists no pending row — not from DCIM/Camera, not after a cold start, not \
         after hand-setting datetaken. `content update --bind is_pending:i:0` alone put \
         the image in the picker's FIRST cell, and MediaProvider then filled _size, \
         width, height and date_modified by itself. `crate::publish::import` does exactly \
         that one update and reads the flag back."
    );
    tokio::fs::remove_file(&local).await.ok();
    Ok(())
}

/// Drive `crate::publish`'s four steps end to end and check each one's own claim.
///
/// Builds a two-file campaign in a temp directory so the ordering and the file count
/// are both observable, then asserts what each step is contractually required to
/// return — including that cleanup is idempotent, which the publish runner relies on
/// in its retry path.
async fn exercise_contract(
    adb: &AdbProgram,
    serial: &str,
    source_image: &std::path::Path,
) -> anyhow::Result<()> {
    // A campaign id in the shape the app really uses: `<request-id>:<bundle-id>`, with
    // the colon that has to survive becoming a directory name.
    let campaign_id = "riviu-probe-req:bundle-1";
    let root = std::env::temp_dir().join("riviu-publish-contract");
    let _ = tokio::fs::remove_dir_all(&root).await;
    tokio::fs::create_dir_all(&root).await?;
    let bytes = tokio::fs::read(source_image).await?;
    for name in ["01.png", "02.png"] {
        tokio::fs::write(root.join(name), &bytes).await?;
    }

    let staged = riviu_android_driver::publish::stage(adb, serial, campaign_id, &root).await?;
    let manifest_sha = staged
        .get("manifestSha256")
        .and_then(|value| value.as_str())
        .ok_or_else(|| anyhow::anyhow!("stage returned no manifestSha256"))?
        .to_string();
    println!(
        "  stage:   {} file(s), manifest {}…, readback {}",
        staged
            .get("fileCount")
            .and_then(|v| v.as_u64())
            .unwrap_or(0),
        &manifest_sha[..12],
        staged
            .get("readback")
            .and_then(|v| v.as_str())
            .unwrap_or("?")
    );

    let prepared =
        riviu_android_driver::publish::prepare(adb, serial, campaign_id, &manifest_sha).await?;
    let import_id = prepared
        .get("importId")
        .and_then(|value| value.as_str())
        .ok_or_else(|| anyhow::anyhow!("prepare returned no importId"))?
        .to_string();
    anyhow::ensure!(
        prepared.get("state").and_then(|v| v.as_str()) == Some("ready"),
        "prepare did not report ready: {prepared}"
    );
    println!("  prepare: state=ready importId={import_id}");

    let imported =
        riviu_android_driver::publish::import(adb, serial, campaign_id, &manifest_sha).await?;
    anyhow::ensure!(
        imported.get("state").and_then(|v| v.as_str()) == Some("imported"),
        "import did not report imported: {imported}"
    );
    let files = imported.get("files").and_then(|v| v.as_u64()).unwrap_or(0);
    // Print which of the two measured device behaviours this phone took. The gate passes
    // either way; the point is that the difference is visible instead of inferred.
    println!(
        "  import:  state=imported files={files} scan={} pending={} album={}",
        imported
            .get("scanBroadcast")
            .and_then(|v| v.as_bool())
            .map(|needed| if needed { "broadcast" } else { "not-needed" })
            .unwrap_or("?"),
        imported
            .get("pendingModel")
            .and_then(|v| v.as_str())
            .unwrap_or("?"),
        imported
            .get("albumId")
            .and_then(|v| v.as_str())
            .unwrap_or("?")
    );
    anyhow::ensure!(
        files == 2,
        "expected both files visible, MediaStore lists {files}"
    );

    // Cleanup twice: the publish runner calls it again on its retry path and asserts
    // `cleaned` both times.
    let first = riviu_android_driver::publish::cleanup(adb, serial, &import_id).await?;
    let second = riviu_android_driver::publish::cleanup(adb, serial, &import_id).await?;
    for (label, value) in [("first", &first), ("second", &second)] {
        anyhow::ensure!(
            value.get("state").and_then(|v| v.as_str()) == Some("cleaned"),
            "{label} cleanup did not report cleaned: {value}"
        );
    }
    println!(
        "  cleanup: cleaned twice (removed {} then {})",
        first.get("files").and_then(|v| v.as_u64()).unwrap_or(0),
        second.get("files").and_then(|v| v.as_u64()).unwrap_or(0)
    );

    tokio::fs::remove_dir_all(&root).await.ok();
    Ok(())
}

/// Stage and import one recognisable image, then stop.
///
/// The point is to leave MediaStore holding a row so the *picker* can be looked at —
/// MediaStore visibility is necessary and not sufficient, and only TikTok's own grid
/// answers the sufficient half.
async fn leave_imported(
    adb: &AdbProgram,
    serial: &str,
    source_image: &std::path::Path,
) -> anyhow::Result<String> {
    let campaign_id = "picker-check:one";
    let root = std::env::temp_dir().join("riviu-picker-check");
    let _ = tokio::fs::remove_dir_all(&root).await;
    tokio::fs::create_dir_all(&root).await?;
    tokio::fs::copy(source_image, root.join("01.png")).await?;

    let staged = riviu_android_driver::publish::stage(adb, serial, campaign_id, &root).await?;
    let manifest_sha = staged
        .get("manifestSha256")
        .and_then(|value| value.as_str())
        .ok_or_else(|| anyhow::anyhow!("stage returned no manifestSha256"))?
        .to_string();
    let prepared =
        riviu_android_driver::publish::prepare(adb, serial, campaign_id, &manifest_sha).await?;
    println!("  prepare: {prepared}");
    let imported =
        riviu_android_driver::publish::import(adb, serial, campaign_id, &manifest_sha).await?;
    println!("  import:  {imported}");
    tokio::fs::remove_dir_all(&root).await.ok();
    Ok(imported
        .get("importId")
        .and_then(|value| value.as_str())
        .unwrap_or_default()
        .to_string())
}
