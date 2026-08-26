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

/// Where the sidecar tree came from, which is what decides whether a missing file is a fault.
///
/// **The same absence means opposite things in the two cases, and the loader used to treat them
/// alike.** A source checkout that has not fetched the binaries is an ordinary developer state:
/// putting a red banner in front of it would train everyone to ignore the banner. An *installed*
/// build whose bundle has no manifest is broken -- the installer shipped without the files it is
/// supposed to carry -- and that is exactly the state that produces "the app sees my phone but
/// nothing works", with nothing anywhere saying why.
///
/// The signal already existed and was thrown away: `resolve_sidecar_root_from` returns the
/// packaged path only when it can see a real sidecar inside the resource directory, and falls
/// back to the repo tree otherwise. It knew which world it was in and did not pass it on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SidecarOrigin {
    /// `RIVIU_SIDECAR_ROOT` named it. Someone pointed us here deliberately, so what is missing
    /// is worth saying.
    Configured,
    /// Inside the installed bundle. Anything absent here is a broken installation.
    Packaged,
    /// The repository's own `sidecars/`, running from source. Absent binaries are normal.
    RepoCheckout,
}

impl SidecarOrigin {
    /// Whether this tree is one that is *supposed* to be complete.
    pub fn expects_a_complete_bundle(self) -> bool {
        matches!(self, Self::Configured | Self::Packaged)
    }
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
    /// Never returns an error, and **what counts as an error depends on `origin`**: a checkout
    /// that has not fetched the binaries is an ordinary state, an installed bundle missing them
    /// is a fault. See [`SidecarOrigin`].
    pub fn load_from(sidecar_root: &Path, origin: SidecarOrigin) -> Self {
        let dir = sidecar_root.join("android");
        let manifest_path = dir.join(MANIFEST_NAME);
        let mut tools = Self::default();

        let bytes = match std::fs::read(&manifest_path) {
            Ok(bytes) => bytes,
            Err(error) => {
                if origin.expects_a_complete_bundle() {
                    // **An installed build with no manifest is the loudest case, and it used to
                    // be the quietest.** `problems` stayed empty, so the banner that exists to
                    // report exactly this had nothing to show, and the operator got a phone
                    // that listed and would not drive -- with no line anywhere naming a cause.
                    tools.problems.push(format!(
                        "bản cài thiếu {MANIFEST_NAME} ở {} ({error}) — không có adb, minicap, \
                         scrcpy hay helper nào được đóng gói, nên nhận được máy mà không điều \
                         khiển được. Cài lại bản dựng.",
                        manifest_path.display()
                    ));
                } else {
                    // A source checkout without the bundled tools is a normal developer state,
                    // and the operator-facing consequence (no adb, no stream) is reported by the
                    // code that needs them. Recording it would put a red line in front of every
                    // developer, every run.
                    log::debug!(
                        "no bundled Android tools at {}: {error}",
                        manifest_path.display()
                    );
                }
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

        // **And then say what did not arrive.** Every field above is an `Option`, and a role
        // whose entry is missing from the manifest -- or whose file failed verification -- simply
        // left its field `None`. The app then behaved as though that capability did not exist:
        // no tap, or no stream, or no clipboard, silently and per-capability. A bundle that is
        // supposed to be complete has to be told it is not.
        if origin.expects_a_complete_bundle() {
            for (role, what) in REQUIRED_ROLES {
                if tools.resolved(role) {
                    continue;
                }
                tools.problems.push(format!(
                    "bản cài thiếu `{role}` — mất {what}. Cài lại bản dựng."
                ));
            }
            #[cfg(windows)]
            if tools.adb_path.is_none() {
                tools.problems.push(
                    "bản cài thiếu `adbExe` — không nhận được máy nào qua USB. Cài lại bản dựng."
                        .to_string(),
                );
            }
        }

        tools
    }

    /// Whether one manifest role ended up with a usable file.
    ///
    /// Keyed by the manifest's own role name so [`REQUIRED_ROLES`] reads as one table rather
    /// than as a list that has to be kept in step with a match arm somewhere else.
    fn resolved(&self, role: &str) -> bool {
        match role {
            "adbExe" => self.adb_path.is_some(),
            "minicapApk" => self.minicap_apk.is_some(),
            "scrcpyServer" => self.scrcpy_server.is_some(),
            "riviuAgentApk" => self.riviu_agent_apk.is_some(),
            "agentServerApk" => self.agent_server_apk.is_some(),
            "agentTestApk" => self.agent_test_apk.is_some(),
            // An unknown role cannot be missing: this build does not use it.
            _ => true,
        }
    }
}

/// Every role a complete bundle carries, with the field it fills and why it matters.
///
/// `adbExe` is deliberately absent: it is Windows-only by construction (`win-x86_64/adb.exe`),
/// and the loader already skips it elsewhere. Everything here is `noarch/`, so on any platform a
/// complete bundle has all five.
const REQUIRED_ROLES: &[(&str, &str)] = &[
    ("minicapApk", "ảnh tile (minicap)"),
    ("scrcpyServer", "phát hình H.264 (scrcpy-server)"),
    (
        "riviuAgentApk",
        "clipboard, ảnh, hình nền, GPS (helper Riviu)",
    ),
    (
        "agentServerApk",
        "điều khiển: chạm, gõ chữ, đọc cây (uiautomator2 server)",
    ),
    (
        "agentTestApk",
        "nửa androidTest của uiautomator2 — thiếu nó là mọi cú chạm bị từ chối",
    ),
];

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

    /// `load_from` with the origin these tests are all about: a source checkout, where an
    /// absent binary is a normal state rather than a broken install.
    fn load_checkout(root: &Path) -> AndroidTools {
        AndroidTools::load_from(root, SidecarOrigin::RepoCheckout)
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

    /// **An installed build with no bundle at all was the loudest case and the quietest code.**
    ///
    /// The same absence in a checkout is normal, so the loader treated both alike: `problems`
    /// stayed empty, and the banner written to report exactly this had nothing to show. What the
    /// operator got instead was a phone that listed and would not drive, with no line anywhere
    /// naming a cause -- which is the report this whole pass came from.
    #[test]
    fn an_installed_build_with_no_bundle_says_so() {
        let root = scratch("packaged-absent");
        let tools = AndroidTools::load_from(&root, SidecarOrigin::Packaged);

        assert!(
            !tools.problems.is_empty(),
            "a packaged build with no manifest has to report it"
        );
        let said = tools.problems.join(" ");
        assert!(said.contains(MANIFEST_NAME), "{said}");
        assert!(
            said.contains("Cài lại"),
            "and it has to say what to do about it: {said}"
        );
        std::fs::remove_dir_all(&root).ok();
    }

    /// The same tree, read as a checkout, stays silent. Both halves of the distinction matter:
    /// a banner in front of every developer, every run, is a banner nobody reads.
    #[test]
    fn the_same_absence_in_a_checkout_stays_silent() {
        let root = scratch("checkout-absent");
        let tools = AndroidTools::load_from(&root, SidecarOrigin::RepoCheckout);
        assert!(tools.problems.is_empty(), "{:?}", tools.problems);
        std::fs::remove_dir_all(&root).ok();
    }

    /// **A role that never resolved has to be named, per role.**
    ///
    /// Every tool is an `Option`, and a role missing from the manifest -- or one whose file
    /// failed verification -- simply left its field `None`. The app then behaved as though that
    /// capability did not exist: no tap, or no stream, or no clipboard, silently and one
    /// capability at a time. `good_bundle` pins only three of the six roles, so this fixture is
    /// exactly the shape of a bundle that shipped short.
    #[test]
    fn an_installed_build_missing_a_role_names_that_role_and_what_it_costs() {
        let root = scratch("packaged-short");
        let tools = AndroidTools::load_from(&good_bundle(&root), SidecarOrigin::Packaged);

        let said = tools.problems.join(" | ");
        // The three `good_bundle` does carry must not be complained about.
        assert!(!said.contains("minicapApk"), "{said}");
        assert!(!said.contains("scrcpyServer"), "{said}");
        // The three it does not carry must each be named.
        for role in ["riviuAgentApk", "agentServerApk", "agentTestApk"] {
            assert!(
                said.contains(role),
                "{role} was missing and unreported: {said}"
            );
        }
        // And the report says what the operator loses, not just a role name.
        assert!(
            said.contains("chạm") || said.contains("điều khiển"),
            "a role name alone does not tell an operator anything: {said}"
        );
        std::fs::remove_dir_all(&root).ok();
    }

    /// A complete bundle read as packaged reports nothing.
    ///
    /// The guard against the opposite failure: a completeness check that always complains is a
    /// check nobody can act on. `REQUIRED_ROLES` is five `noarch/` entries, so a fixture that
    /// pins all five is clean on every platform.
    #[test]
    fn a_complete_installed_bundle_reports_nothing() {
        let root = scratch("packaged-complete");
        let tools = AndroidTools::load_from(&complete_bundle(&root), SidecarOrigin::Packaged);
        assert!(
            tools.problems.is_empty(),
            "a complete bundle must be silent, got {:?}",
            tools.problems
        );
        assert!(tools.agent_server_apk.is_some());
        assert!(tools.agent_test_apk.is_some());
        assert!(tools.riviu_agent_apk.is_some());
        std::fs::remove_dir_all(&root).ok();
    }

    /// Every role this build requires, as a fixture. Written from `REQUIRED_ROLES` so the two
    /// cannot drift apart without a compile error somewhere.
    fn complete_bundle(dir: &Path) -> PathBuf {
        let android = dir.join("android");
        std::fs::create_dir_all(android.join("noarch")).expect("noarch");
        std::fs::create_dir_all(android.join("win-x86_64")).expect("win dir");

        let mut files = Vec::new();
        for (role, name) in [
            ("minicapApk", "noarch/minicap.apk"),
            ("scrcpyServer", "noarch/scrcpy-server"),
            ("riviuAgentApk", "noarch/riviu-agent.apk"),
            ("agentServerApk", "noarch/appium-uiautomator2-server.apk"),
            ("agentTestApk", "noarch/appium-uiautomator2-server-test.apk"),
            ("adbExe", "win-x86_64/adb.exe"),
        ] {
            // Distinct bytes per file, so a digest mix-up between two of them would fail rather
            // than pass by coincidence.
            let bytes = role.as_bytes().to_vec();
            std::fs::write(android.join(name), &bytes).expect("fixture file");
            files.push(serde_json::json!({
                "path": name,
                "bytes": bytes.len(),
                "sha256": sha_of(&bytes),
                "role": role
            }));
        }

        std::fs::write(
            android.join(MANIFEST_NAME),
            serde_json::to_vec_pretty(&serde_json::json!({
                "manifestVersion": 1,
                "files": files
            }))
            .expect("manifest json"),
        )
        .expect("manifest");
        dir.to_path_buf()
    }

    /// **The required-role table has to cover what the app actually reads.**
    ///
    /// A completeness check is only as complete as its own list: a seventh tool added to
    /// `AndroidTools` and left out of `REQUIRED_ROLES` would go missing exactly as silently as
    /// before. Counted here against the struct's own fields.
    #[test]
    fn every_bundled_tool_is_either_required_or_windows_only() {
        // adb is the one deliberate exclusion, because it is `win-x86_64/` by construction.
        let expected = [
            "minicapApk",
            "scrcpyServer",
            "riviuAgentApk",
            "agentServerApk",
            "agentTestApk",
        ];
        let listed: Vec<&str> = REQUIRED_ROLES.iter().map(|(role, _)| *role).collect();
        assert_eq!(listed, expected);

        let source = include_str!("android_tools.rs");
        let fields = source
            .split("pub struct AndroidTools {")
            .nth(1)
            .expect("the struct is here")
            .split("pub problems")
            .next()
            .expect("problems ends the tool list")
            .matches("pub ")
            .count();
        assert_eq!(
            fields,
            expected.len() + 1,
            "AndroidTools holds {fields} tools but REQUIRED_ROLES lists {} plus adb; a tool \
             left out of that table goes missing as silently as before",
            expected.len()
        );
    }

    #[test]
    fn a_matching_bundle_resolves_both_tools_and_reports_nothing() {
        let root = scratch("good");
        let tools = load_checkout(&good_bundle(&root));
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
        let tools = load_checkout(&root);
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

        let tools = load_checkout(&bundle);
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

        let tools = load_checkout(&bundle);
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

        let tools = load_checkout(&bundle);
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

        let tools = load_checkout(&root);
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

        let tools = load_checkout(&root);
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
        let tools = load_checkout(&repo);
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

        let tools = load_checkout(&bundle);
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

        let tools = load_checkout(&bundle);
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

        let tools = load_checkout(&bundle);
        assert!(tools.scrcpy_server.is_none());
        assert!(tools.minicap_apk.is_some());
        assert_eq!(tools.problems.len(), 1);
        std::fs::remove_dir_all(&root).ok();
    }
}
