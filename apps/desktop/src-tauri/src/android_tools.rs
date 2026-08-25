//! The Android tools shipped inside the installer, and proof they arrived intact.
//!
//! Deliberately a mirror of `AgentArtifact::load` + `verify_checksum`
//! (`crates/ios-driver/src/agent.rs`): same manifest-beside-the-payload shape, same
//! refusal of path traversal, same streaming digest. Two loaders for two bundled
//! artefacts is already one more than ideal; two loaders that disagree about what a
//! safe relative path is would be worse.
//!
//! Two rules run through everything here.
//!
//! **Nothing in this module is fatal.** A corrupt bundled `adb.exe` must not stop an
//! operator whose own adb is fine — the bundled copy is the *last* candidate
//! (`AdbProgram::candidates`), so losing it costs nothing on a machine that has its
//! own. So a mismatch yields `None` for that tool plus a problem string naming the
//! file and both digests, never a panic and never a startup failure.
//!
//! **Verification is over the bytes, not the size.** `frames::ensure_apk` decides
//! whether to re-push minicap by comparing byte counts on the device, so a corrupted
//! APK of the identical size would be trusted forever. The manifest therefore pins
//! `bytes` *and* `sha256` and this checks both — the size first, because it makes the
//! common failure (a truncated or Git-mangled file) report a number an operator can
//! act on instead of two hex strings.

use std::io::Read;
use std::path::{Component, Path, PathBuf};

use serde::Deserialize;
use sha2::{Digest, Sha256};

/// The manifest that ships beside the binaries.
///
/// Only the fields this loader acts on are modelled. The provenance fields
/// (`adbRevision`, `minicapSource`, …) are for humans reading the file and for
/// `NOTICE`; parsing them here would invite drift between what is checked and what
/// is claimed.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AndroidToolsManifest {
    manifest_version: u32,
    files: Vec<ManifestFile>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ManifestFile {
    path: String,
    bytes: u64,
    sha256: String,
    /// Which tool this file *is*, when it is one the app resolves by name.
    ///
    /// Absent for the files that only have to exist next to a tool — the two DLLs
    /// Windows loads from `adb.exe`'s own directory, and Google's NOTICE. Those are
    /// still verified; they just are not looked up.
    #[serde(default)]
    role: Option<String>,
}

/// What the installer actually provided, after checking.
#[derive(Debug, Default, Clone)]
pub struct AndroidTools {
    /// The bundled `adb`, or `None` when absent, unverifiable, or not this platform.
    pub adb_path: Option<PathBuf>,
    /// The bundled `minicap.apk`, same conditions.
    pub minicap_apk: Option<PathBuf>,
    /// The bundled scrcpy 3.3.4 server JAR, used only for the H.264 view path.
    pub scrcpy_server: Option<PathBuf>,
    /// The bundled Riviu helper APK, when a build has pinned one.
    pub riviu_agent_apk: Option<PathBuf>,
    /// `appium-uiautomator2-server`, the instrumentation the control loop talks to.
    ///
    /// Bundled since 16/08/2026. Before that the app told the operator to "install both
    /// appium-uiautomator2-server APKs" and shipped neither, so on a freshly plugged box
    /// every device streamed video and refused every tap -- measured 0/20 on a Galaxy S8
    /// farm. Video worked because `scrcpy-server` IS bundled and pushed; nothing pushed
    /// these.
    pub agent_server_apk: Option<PathBuf>,
    /// The `androidTest` half. Both halves are required: the runner lives in this one and
    /// `am instrument` names it, so installing only the server leaves the same refusal.
    pub agent_test_apk: Option<PathBuf>,
    /// Everything that went wrong, in the operator's language.
    ///
    /// A list rather than a single `Option` because a bad checkout tends to damage
    /// more than one file, and reporting only the first sends someone to fix a
    /// symptom.
    pub problems: Vec<String>,
}

/// The manifest version this build understands.
///
/// A newer bundle is refused rather than half-read: the failure mode of guessing is
/// verifying files against fields that have moved.
const SUPPORTED_MANIFEST_VERSION: u32 = 1;

const MANIFEST_NAME: &str = "android-tools-manifest.json";

impl AndroidTools {
    /// Load and verify the tools under `<sidecar_root>/android`.
    ///
    /// Never returns an error. Absent is a normal state: a source checkout that has
    /// not fetched the binaries, or any non-Windows host for `adb`.
    pub fn load(sidecar_root: &Path) -> Self {
        let dir = sidecar_root.join("android");
        let manifest_path = dir.join(MANIFEST_NAME);
        let mut tools = Self::default();

        let bytes = match std::fs::read(&manifest_path) {
            Ok(bytes) => bytes,
            Err(error) => {
                // Not a problem worth surfacing on its own: a source checkout without
                // the bundled tools is a normal developer state, and the operator-facing
                // consequence (no adb, no stream) is reported by the code that needs
                // them. Recording it would put a red line in front of every developer.
                log::debug!(
                    "no bundled Android tools at {}: {error}",
                    manifest_path.display()
                );
                return tools;
            }
        };

        let manifest: AndroidToolsManifest = match serde_json::from_slice(&bytes) {
            Ok(manifest) => manifest,
            Err(error) => {
                tools.problems.push(format!(
                    "{MANIFEST_NAME} không đọc được ({error}) — bỏ qua adb, minicap, scrcpy và helper đóng gói"
                ));
                return tools;
            }
        };
        if manifest.manifest_version != SUPPORTED_MANIFEST_VERSION {
            tools.problems.push(format!(
                "{MANIFEST_NAME} là phiên bản {} nhưng bản dựng này chỉ đọc được {SUPPORTED_MANIFEST_VERSION}",
                manifest.manifest_version
            ));
            return tools;
        }

        for file in &manifest.files {
            let path = match safe_join(&dir, &file.path) {
                Ok(path) => path,
                Err(reason) => {
                    tools
                        .problems
                        .push(format!("{MANIFEST_NAME}: {} — {reason}", file.path));
                    continue;
                }
            };
            if let Err(reason) = verify(&path, file) {
                tools.problems.push(reason);
                continue;
            }
            match file.role.as_deref() {
                Some("adbExe") => {
                    // Only Windows. The bundle carries a `win-x86_64` adb, and offering
                    // it to macOS would hand `AdbProgram` a path that cannot execute —
                    // a candidate that always fails, which is worse than no candidate
                    // because it lands in every refusal message.
                    #[cfg(windows)]
                    {
                        tools.adb_path = Some(path);
                    }
                    #[cfg(not(windows))]
                    {
                        let _ = path;
                    }
                }
                Some("minicapApk") => tools.minicap_apk = Some(path),
                Some("scrcpyServer") => tools.scrcpy_server = Some(path),
                Some("riviuAgentApk") => tools.riviu_agent_apk = Some(path),
                Some("agentServerApk") => tools.agent_server_apk = Some(path),
                Some("agentTestApk") => tools.agent_test_apk = Some(path),
                // A role this build does not know is not an error. The bytes were still
                // verified above; the manifest is simply describing something newer.
                Some(_) | None => {}
            }
        }

        tools
    }
}

/// Join a manifest-declared relative path onto the bundle directory, or refuse.
///
/// The manifest travels inside the installer, so this is not a hostile-input
/// boundary in the usual sense — but it is exactly the shape where "the file we
/// verified" and "the file we then use" can be made to differ, and the iOS loader
/// already refuses the same three component kinds. Same rule, same place.
fn safe_join(dir: &Path, relative: &str) -> Result<PathBuf, String> {
    let candidate = Path::new(relative);
    if relative.trim().is_empty() {
        return Err("đường dẫn rỗng".to_string());
    }
    if candidate.is_absolute() {
        return Err("phải là đường dẫn tương đối".to_string());
    }
    if candidate.components().any(|component| {
        matches!(
            component,
            Component::ParentDir | Component::RootDir | Component::Prefix(_)
        )
    }) {
        return Err("đường dẫn thoát ra ngoài thư mục sidecar".to_string());
    }
    Ok(dir.join(candidate))
}

/// Check one file against its manifest entry: size, then digest.
fn verify(path: &Path, expected: &ManifestFile) -> Result<(), String> {
    let metadata = std::fs::metadata(path)
        .map_err(|error| format!("{} không đọc được: {error}", path.display()))?;
    if !metadata.is_file() {
        return Err(format!("{} không phải một file", path.display()));
    }
    if metadata.len() != expected.bytes {
        return Err(format!(
            "{} sai kích thước: {} byte, manifest ghi {} byte",
            path.display(),
            metadata.len(),
            expected.bytes
        ));
    }

    let actual = digest(path).map_err(|error| format!("{}: {error}", path.display()))?;
    if !actual.eq_ignore_ascii_case(expected.sha256.trim()) {
        return Err(format!(
            "{} sai SHA-256: đọc được {actual}, manifest ghi {}",
            path.display(),
            expected.sha256
        ));
    }
    Ok(())
}

/// Stream the file rather than reading it whole: `adb.exe` is 8 MB and the bundled
/// NOTICE is another 1,1 MB, and this runs on the startup path.
fn digest(path: &Path) -> std::io::Result<String> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a bundle directory whose bytes match the manifest it writes.
    fn good_bundle(dir: &Path) -> PathBuf {
        let android = dir.join("android");
        std::fs::create_dir_all(android.join("noarch")).expect("noarch");
        std::fs::create_dir_all(android.join("win-x86_64")).expect("win dir");
        std::fs::write(android.join("noarch/minicap.apk"), b"apk bytes").expect("apk");
        std::fs::write(android.join("noarch/scrcpy-server"), b"jar bytes").expect("scrcpy");
        std::fs::write(android.join("win-x86_64/adb.exe"), b"adb bytes").expect("adb");

        let manifest = serde_json::json!({
            "manifestVersion": 1,
            "files": [
                {
                    "path": "noarch/minicap.apk",
                    "bytes": 9,
                    "sha256": sha_of(b"apk bytes"),
                    "role": "minicapApk"
                },
                {
                    "path": "noarch/scrcpy-server",
                    "bytes": 9,
                    "sha256": sha_of(b"jar bytes"),
                    "role": "scrcpyServer"
                },
                {
                    "path": "win-x86_64/adb.exe",
                    "bytes": 9,
                    "sha256": sha_of(b"adb bytes"),
                    "role": "adbExe"
                }
            ]
        });
        std::fs::write(
            android.join(MANIFEST_NAME),
            serde_json::to_vec_pretty(&manifest).expect("manifest json"),
        )
        .expect("manifest");
        dir.to_path_buf()
    }

    fn sha_of(bytes: &[u8]) -> String {
        let mut hasher = Sha256::new();
        hasher.update(bytes);
        format!("{:x}", hasher.finalize())
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("riviu-android-tools-{name}"));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).expect("scratch dir");
        dir
    }

    #[test]
    fn a_matching_bundle_resolves_both_tools_and_reports_nothing() {
        let root = scratch("good");
        let tools = AndroidTools::load(&good_bundle(&root));
        assert!(
            tools.problems.is_empty(),
            "expected no problems, got {:?}",
            tools.problems
        );
        assert!(tools.minicap_apk.is_some());
        assert!(tools.scrcpy_server.is_some());
        // adb is Windows-only by design; asserting it per platform keeps this honest
        // rather than passing for the wrong reason on a Mac.
        #[cfg(windows)]
        assert!(tools.adb_path.is_some());
        #[cfg(not(windows))]
        assert!(tools.adb_path.is_none());
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn no_bundle_at_all_is_silent_because_a_source_checkout_is_normal() {
        let root = scratch("absent");
        let tools = AndroidTools::load(&root);
        assert!(tools.problems.is_empty());
        assert!(tools.adb_path.is_none());
        assert!(tools.minicap_apk.is_none());
        assert!(tools.scrcpy_server.is_none());
        assert!(tools.riviu_agent_apk.is_none());
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn a_wrong_digest_drops_that_tool_and_names_both_digests() {
        // The failure this exists for: a file of the *right length* whose contents are
        // wrong. Byte counts alone would pass it, and `ensure_apk` compares byte counts.
        let root = scratch("baddigest");
        let bundle = good_bundle(&root);
        std::fs::write(bundle.join("android/noarch/minicap.apk"), b"apk bytez").expect("rewrite");

        let tools = AndroidTools::load(&bundle);
        assert!(
            tools.minicap_apk.is_none(),
            "a corrupt APK must not be used"
        );
        assert_eq!(tools.problems.len(), 1);
        let problem = &tools.problems[0];
        assert!(problem.contains("SHA-256"), "{problem}");
        assert!(problem.contains(&sha_of(b"apk bytez")), "{problem}");
        assert!(problem.contains(&sha_of(b"apk bytes")), "{problem}");
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn a_wrong_size_reports_the_two_numbers_rather_than_two_hashes() {
        // Truncation and line-ending mangling are the likely real causes, and both are
        // far easier to recognise from "9 byte vs 12 byte" than from hex.
        let root = scratch("badsize");
        let bundle = good_bundle(&root);
        std::fs::write(bundle.join("android/noarch/minicap.apk"), b"much longer").expect("rewrite");

        let tools = AndroidTools::load(&bundle);
        assert!(tools.minicap_apk.is_none());
        let problem = &tools.problems[0];
        assert!(problem.contains("kích thước"), "{problem}");
        assert!(!problem.contains("SHA-256"), "{problem}");
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn one_broken_file_does_not_take_the_other_tool_with_it() {
        // Independence matters: minicap breaking must not cost the operator adb, and a
        // single early `return` would do exactly that.
        let root = scratch("partial");
        let bundle = good_bundle(&root);
        std::fs::write(bundle.join("android/noarch/minicap.apk"), b"corrupted").expect("rewrite");

        let tools = AndroidTools::load(&bundle);
        assert!(tools.minicap_apk.is_none());
        assert_eq!(tools.problems.len(), 1);
        #[cfg(windows)]
        assert!(
            tools.adb_path.is_some(),
            "adb was verified and must still be offered"
        );
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn a_manifest_path_that_climbs_out_is_refused_before_anything_is_read() {
        let root = scratch("traversal");
        let android = root.join("android");
        std::fs::create_dir_all(&android).expect("dir");
        std::fs::write(
            android.join(MANIFEST_NAME),
            serde_json::to_vec(&serde_json::json!({
                "manifestVersion": 1,
                "files": [{
                    "path": "../../escaped.apk",
                    "bytes": 1,
                    "sha256": sha_of(b"x"),
                    "role": "minicapApk"
                }]
            }))
            .expect("json"),
        )
        .expect("manifest");

        let tools = AndroidTools::load(&root);
        assert!(tools.minicap_apk.is_none());
        assert!(
            tools.problems[0].contains("thoát ra ngoài"),
            "{:?}",
            tools.problems
        );
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn a_newer_manifest_version_is_refused_rather_than_half_read() {
        let root = scratch("version");
        let android = root.join("android");
        std::fs::create_dir_all(&android).expect("dir");
        std::fs::write(
            android.join(MANIFEST_NAME),
            br#"{"manifestVersion": 2, "files": []}"#,
        )
        .expect("manifest");

        let tools = AndroidTools::load(&root);
        assert!(
            tools.adb_path.is_none()
                && tools.minicap_apk.is_none()
                && tools.scrcpy_server.is_none()
                && tools.riviu_agent_apk.is_none()
        );
        assert!(
            tools.problems[0].contains("phiên bản 2"),
            "{:?}",
            tools.problems
        );
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn the_shipped_manifest_matches_the_bytes_committed_to_this_repo() {
        // The one test that would fail on a real mistake: it verifies the actual
        // `sidecars/android/` tree against the actual manifest, so replacing a binary
        // without regenerating the pin cannot reach a release. Skipped rather than
        // failed when the tree is absent, because a shallow checkout is legitimate.
        let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../..")
            .join("sidecars");
        if !repo.join("android").join(MANIFEST_NAME).is_file() {
            eprintln!("sidecars/android not present, skipping");
            return;
        }
        let tools = AndroidTools::load(&repo);
        assert!(
            tools.problems.is_empty(),
            "the committed bundle disagrees with its manifest: {:?}",
            tools.problems
        );
        assert!(tools.minicap_apk.is_some(), "minicap.apk did not resolve");
        assert!(
            tools.scrcpy_server.is_some(),
            "scrcpy-server did not resolve"
        );
        #[cfg(windows)]
        assert!(tools.adb_path.is_some(), "adb.exe did not resolve");
    }

    #[test]
    fn an_unknown_role_is_verified_but_does_not_fail_the_bundle() {
        // A newer installer can describe a tool this build does not resolve.
        // The bytes still have to match; the role itself must not be fatal.
        let root = scratch("unknown-role");
        let bundle = good_bundle(&root);
        let android = bundle.join("android");
        std::fs::write(android.join("noarch/extra.bin"), b"extra").expect("extra");
        let manifest = serde_json::json!({
            "manifestVersion": 1,
            "files": [
                {
                    "path": "noarch/minicap.apk",
                    "bytes": 9,
                    "sha256": sha_of(b"apk bytes"),
                    "role": "minicapApk"
                },
                {
                    "path": "noarch/scrcpy-server",
                    "bytes": 9,
                    "sha256": sha_of(b"jar bytes"),
                    "role": "scrcpyServer"
                },
                {
                    "path": "win-x86_64/adb.exe",
                    "bytes": 9,
                    "sha256": sha_of(b"adb bytes"),
                    "role": "adbExe"
                },
                {
                    "path": "noarch/extra.bin",
                    "bytes": 5,
                    "sha256": sha_of(b"extra"),
                    "role": "futureTool"
                }
            ]
        });
        std::fs::write(
            android.join(MANIFEST_NAME),
            serde_json::to_vec_pretty(&manifest).expect("manifest json"),
        )
        .expect("manifest");

        let tools = AndroidTools::load(&bundle);
        assert!(tools.problems.is_empty(), "{:?}", tools.problems);
        assert!(tools.minicap_apk.is_some());
        assert!(tools.scrcpy_server.is_some());
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn a_riviu_agent_role_resolves_when_the_bytes_match() {
        let root = scratch("helper-role");
        let bundle = good_bundle(&root);
        let android = bundle.join("android");
        std::fs::write(android.join("noarch/riviu-agent.apk"), b"helperapk").expect("apk");
        let manifest = serde_json::json!({
            "manifestVersion": 1,
            "files": [
                {
                    "path": "noarch/minicap.apk",
                    "bytes": 9,
                    "sha256": sha_of(b"apk bytes"),
                    "role": "minicapApk"
                },
                {
                    "path": "noarch/scrcpy-server",
                    "bytes": 9,
                    "sha256": sha_of(b"jar bytes"),
                    "role": "scrcpyServer"
                },
                {
                    "path": "win-x86_64/adb.exe",
                    "bytes": 9,
                    "sha256": sha_of(b"adb bytes"),
                    "role": "adbExe"
                },
                {
                    "path": "noarch/riviu-agent.apk",
                    "bytes": 9,
                    "sha256": sha_of(b"helperapk"),
                    "role": "riviuAgentApk"
                }
            ]
        });
        std::fs::write(
            android.join(MANIFEST_NAME),
            serde_json::to_vec_pretty(&manifest).expect("manifest json"),
        )
        .expect("manifest");

        let tools = AndroidTools::load(&bundle);
        assert!(tools.problems.is_empty(), "{:?}", tools.problems);
        assert_eq!(
            tools.riviu_agent_apk.as_deref(),
            Some(android.join("noarch/riviu-agent.apk").as_path())
        );
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn a_corrupt_scrcpy_server_does_not_take_minicap_with_it() {
        let root = scratch("scrcpy-partial");
        let bundle = good_bundle(&root);
        std::fs::write(bundle.join("android/noarch/scrcpy-server"), b"corrupted").expect("rewrite");

        let tools = AndroidTools::load(&bundle);
        assert!(tools.scrcpy_server.is_none());
        assert!(tools.minicap_apk.is_some());
        assert_eq!(tools.problems.len(), 1);
        std::fs::remove_dir_all(&root).ok();
    }
}
