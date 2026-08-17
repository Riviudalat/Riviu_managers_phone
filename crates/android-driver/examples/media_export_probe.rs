//! Run the Export path against a real phone, from Rust.
//!
//! It exists because the only other way to reach [`riviu_android_driver::publish::pull_media`]
//! is through a GUI folder dialog, and a capability that can only be exercised by hand is a
//! capability nobody exercises.
//!
//! It also has to be Rust rather than a shell one-liner, and that is the interesting part.
//! **AGENTS.md 9.12: Git Bash (MSYS2) rewrites the path arguments of `adb push`/`adb pull`**,
//! so `adb pull /sdcard/DCIM/Camera/x.png /c/Users/.../x.png` from a bash prompt silently
//! writes nothing while exiting zero. A probe written in bash would therefore "prove" this
//! path broken on a machine where it works. From Rust the path never passes through a shell.
//!
//! ```text
//! cargo run -p riviu-android-driver --example media_export_probe -- <serial> [dest-dir]
//! cargo run -p riviu-android-driver --example media_export_probe -- <serial> --import <file>
//! ```
//!
//! Export is read-only on the phone: it copies files off and changes nothing. `--import`
//! **writes**, running the same stage → prepare → import the desktop's Import row does, and
//! leaves the file in the gallery — which is the point, but it is a change to the device.

use std::path::PathBuf;

use riviu_android_driver::AndroidDriverConfig;
use riviu_core::driver::DeviceDriver;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let serial = std::env::args()
        .nth(1)
        .ok_or_else(|| anyhow::anyhow!("usage: media_export_probe <serial> [dest-dir]"))?;
    let dest = std::env::args()
        .nth(2)
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join(format!("riviu-export-{serial}"));

    let driver = riviu_android_driver::detect_driver(&AndroidDriverConfig::default())
        .await
        .map_err(|reason| anyhow::anyhow!("no usable adb on this host: {reason}"))?;

    if std::env::args().nth(2).as_deref() == Some("--import") {
        let file = std::env::args()
            .nth(3)
            .ok_or_else(|| anyhow::anyhow!("usage: … <serial> --import <file>"))?;
        return import_one(&driver, &serial, PathBuf::from(file)).await;
    }
    if std::env::args().nth(2).as_deref() == Some("--cleanup") {
        // Undo an `--import`. Deletes MediaStore rows by `_id`, never by a `_data LIKE
        // '%riviu%'` pattern — the measured phone already had an unrelated
        // `riviufarm-shot.png` from another tool, and a loose pattern would take it too.
        let import_id = std::env::args()
            .nth(3)
            .ok_or_else(|| anyhow::anyhow!("usage: … <serial> --cleanup <importId>"))?;
        println!(
            "{}",
            driver.cleanup_publish_media(&serial, &import_id).await?
        );
        return Ok(());
    }

    println!("exporting {serial} into {}", dest.display());
    let started = std::time::Instant::now();
    let pulled = driver.pull_media(&serial, &dest).await?;
    let elapsed = started.elapsed();

    let bytes: u64 = pulled
        .fetched
        .iter()
        .filter_map(|path| std::fs::metadata(path).ok())
        .map(|meta| meta.len())
        .sum();
    println!(
        "{} of {} files, {:.1} MB, in {:.1}s",
        pulled.fetched.len(),
        pulled.found,
        bytes as f64 / (1024.0 * 1024.0),
        elapsed.as_secs_f64()
    );
    if pulled.missed() > 0 {
        println!(
            "  {} file(s) found on the phone did not arrive",
            pulled.missed()
        );
    }
    for path in pulled.fetched.iter().take(5) {
        let size = std::fs::metadata(path).map(|meta| meta.len()).unwrap_or(0);
        println!("  {} ({size} bytes)", path.display());
    }
    if pulled.fetched.len() > 5 {
        println!("  … and {} more", pulled.fetched.len() - 5);
    }
    Ok(())
}

/// The same three steps `import_media` runs, in the same order and reading the same key.
///
/// Staged as a one-file campaign, which is the shape the measured pipeline takes. The
/// manifest hash is read back out of staging's own evidence rather than recomputed here:
/// prepare and import both key on it, and the two sides agreeing about what landed is the
/// whole point of having a manifest at all.
async fn import_one(
    driver: &std::sync::Arc<riviu_android_driver::AndroidDriver>,
    serial: &str,
    file: PathBuf,
) -> anyhow::Result<()> {
    anyhow::ensure!(file.is_file(), "no such file: {}", file.display());
    let name = file
        .file_name()
        .ok_or_else(|| anyhow::anyhow!("source has no file name"))?
        .to_owned();

    let campaign_id = format!("probe-{}", std::process::id());
    let staged = std::env::temp_dir()
        .join("riviu-import-probe")
        .join(&campaign_id);
    std::fs::create_dir_all(&staged)?;
    std::fs::copy(&file, staged.join(&name))?;

    let evidence = driver
        .stage_publish_media(serial, "com.riviu.agent", &campaign_id, &staged)
        .await?;
    println!("staged: {evidence}");
    let manifest = evidence
        .get("manifestSha256")
        .and_then(|value| value.as_str())
        .ok_or_else(|| anyhow::anyhow!("staging did not report a manifest hash"))?;

    let prepared = driver
        .prepare_publish_media(serial, &campaign_id, manifest)
        .await?;
    println!("prepared: {prepared}");
    let imported = driver
        .import_publish_media(serial, &campaign_id, manifest)
        .await?;
    println!("imported: {imported}");

    let _ = std::fs::remove_dir_all(&staged);
    println!(
        "\n{} is now in the gallery on {serial}. To undo, cleanup_publish_media with the \
         importId above.",
        name.to_string_lossy()
    );
    Ok(())
}
