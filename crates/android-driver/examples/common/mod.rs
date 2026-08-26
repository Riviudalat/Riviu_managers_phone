//! Shared wiring for the examples and gates in this crate.
//!
//! **Why this file exists, and why it is here rather than in the library.**
//!
//! Every example in this crate used to build its driver from
//! `AndroidDriverConfig::default()`, which fills none of the `bundled_*` fields. On a host with
//! no `adb` on `PATH` and no `ANDROID_HOME` — this repo's own dev box, among others — that
//! config finds no adb at all, and the way it fails is the trap: `list_devices()` comes back
//! empty, every `pm list packages` call fails silently, and `resolve_tiktok_package` reports
//!
//! ```text
//! no TikTok build with measured labels is installed; expected one of: …
//! ```
//!
//! on a phone that has one. Measured 26/08/2026: that message cost a whole wrong conclusion
//! ("no fleet phone attached", written into AGENTS.md before `fleet_list` showed `0 device(s)`).
//!
//! It lives in `examples/` and **not** in `src/` on purpose. The only way to find the repo's
//! sidecars from code is `CARGO_MANIFEST_DIR`, which is a **compile-time** path: in a packaged
//! build it points at the build agent's checkout. A helper in the library would compile into the
//! shipped app carrying that path — which is precisely the defect that shipped the yt-dlp lookup
//! inert (AGENTS.md §9.115). Keeping it in `examples/` means it cannot reach production at all.
//!
//! Cargo does not build `examples/common/mod.rs` as an example of its own — an example is
//! `examples/*.rs` or `examples/*/main.rs` — so each example pulls it in with
//! `#[path = "common/mod.rs"] mod common;`.

// This file is compiled **into each example separately**, so a helper only some of them need
// is dead code in the rest. The alternative is a warning in every example that does not use
// every helper, which is noise that trains people to ignore warnings.
#![allow(dead_code)]

use std::path::{Path, PathBuf};

use riviu_android_driver::{AndroidDriver, AndroidDriverConfig};
use riviu_core::driver::DeviceDriver;

/// This repo's `sidecars/` tree, as seen from `crates/android-driver`.
fn sidecars() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../sidecars")
}

/// Only a path that is actually there, so a partial checkout degrades to `default()` rather
/// than to a config full of paths that fail on first use.
fn present(path: PathBuf) -> Option<PathBuf> {
    path.is_file().then_some(path)
}

/// A driver config wired to this repo's own bundled tools.
///
/// Fills the **`bundled_*`** fields, never the plain ones. That is the documented precedence
/// (`AndroidDriverConfig::bundled_adb_path`): an operator's `RIVIU_ADB_PATH`, `ANDROID_SDK_ROOT`
/// or `PATH` still wins. A path the operator cannot outrank is not a safety net, it is a hijack.
pub fn repo_config() -> AndroidDriverConfig {
    let android = sidecars().join("android");
    let noarch = android.join("noarch");
    let exe = if cfg!(windows) { "adb.exe" } else { "adb" };
    AndroidDriverConfig {
        bundled_adb_path: present(android.join("win-x86_64").join(exe)),
        bundled_minicap_apk: present(noarch.join("minicap.apk")),
        bundled_scrcpy_server: present(noarch.join("scrcpy-server")),
        bundled_riviu_agent_apk: present(noarch.join("riviu-agent.apk")),
        bundled_agent_server_apk: present(noarch.join("appium-uiautomator2-server.apk")),
        bundled_agent_test_apk: present(noarch.join("appium-uiautomator2-server-test.apk")),
        ..Default::default()
    }
}

/// Which `adb` this config actually resolves to, and where that came from.
///
/// **The line that would have saved a wrong conclusion.** `list_devices()` answering
/// `0 device(s)` means "adb sees no phone" and "there is no adb at all" equally well, and on
/// 26/08/2026 the second one was read as the first — then as "this phone has no TikTok", which
/// is what `resolve_tiktok_package` reports when every `pm list packages` call fails silently.
/// Printing the resolved path costs one line and removes the ambiguity permanently.
pub fn describe_adb(config: &AndroidDriverConfig) -> String {
    let resolved = riviu_android_driver::AdbProgram::resolve(
        config.adb_path.as_deref(),
        config.bundled_adb_path.as_deref(),
    );
    let Ok(program) = resolved else {
        return "adb      KHÔNG giải được".to_string();
    };
    let path = program.path().to_path_buf();
    // The origin, taken from the same candidate list `resolve` walked rather than guessed at,
    // so "we read your RIVIU_ADB_PATH" is a fact and not a hope.
    let origin = riviu_android_driver::AdbProgram::candidates(
        config.adb_path.as_deref(),
        config.bundled_adb_path.as_deref(),
    )
    .into_iter()
    .find(|candidate| candidate.path == path)
    .map(|candidate| format!("{:?}", candidate.origin))
    .unwrap_or_else(|| "Path".to_string());
    let exists = if path.is_file() {
        ""
    } else {
        "  <- KHÔNG PHẢI FILE: mọi lệnh adb sẽ thất bại im lặng"
    };
    format!("adb      {} [{origin}]{exists}", path.display())
}

/// Refuse early, and say which of the two silences this is.
///
/// "adb cannot see the phone" and "the phone has no TikTok" are the same empty string to
/// everything downstream, and a gate that cannot tell the operator which one it hit sends them
/// looking in the wrong place. Ask before anything else asks.
pub async fn require_device(driver: &AndroidDriver, serial: &str) -> anyhow::Result<()> {
    let visible = driver.list_devices().await?;
    anyhow::ensure!(
        visible.iter().any(|device| device.udid == serial),
        "adb does not see {serial} (thấy {} máy: {:?}) — kiểm cáp, hoặc đặt RIVIU_ADB_PATH",
        visible.len(),
        visible
            .iter()
            .map(|device| device.udid.as_str())
            .collect::<Vec<_>>()
    );
    Ok(())
}
