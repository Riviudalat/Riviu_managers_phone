use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PackageToolsProvenance {
    Packaged,
    DebugOverride,
    Missing,
}

#[derive(Debug, Clone)]
pub struct AndroidPackageTools {
    pub java: Option<PathBuf>,
    pub bundletool: Option<PathBuf>,
    pub provenance: PackageToolsProvenance,
}

/// Resolve the two tools used only for `.apks` device-specific extraction. Overrides are
/// intentionally pairwise: using a debug JRE with a packaged JAR (or vice versa) creates a
/// mixed toolchain nobody can reproduce.
pub fn resolve_android_package_tools(sidecar_root: &Path) -> AndroidPackageTools {
    #[cfg(debug_assertions)]
    {
        let debug_java = std::env::var_os("RIVIU_JAVA_PATH").map(PathBuf::from);
        let debug_bundletool = std::env::var_os("RIVIU_BUNDLETOOL_PATH").map(PathBuf::from);
        if let (Some(java), Some(bundletool)) = (debug_java, debug_bundletool) {
            if java.is_file() && bundletool.is_file() {
                return AndroidPackageTools {
                    java: Some(java),
                    bundletool: Some(bundletool),
                    provenance: PackageToolsProvenance::DebugOverride,
                };
            }
        }
    }

    let root = sidecar_root.join("android-package-tools");
    let java = root
        .join("jre")
        .join("bin")
        .join(if cfg!(windows) { "java.exe" } else { "java" });
    let bundletool = root.join("bundletool.jar");
    let verified = java.is_file()
        && bundletool.is_file()
        && crate::deployment_check::check_android_package_tools(sidecar_root).status
            == crate::deployment_check::CheckStatus::Pass;
    if verified {
        AndroidPackageTools {
            java: Some(java),
            bundletool: Some(bundletool),
            provenance: PackageToolsProvenance::Packaged,
        }
    } else {
        AndroidPackageTools {
            java: None,
            bundletool: None,
            provenance: PackageToolsProvenance::Missing,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unattested_packaged_paths_are_rejected_as_a_pair() {
        let root =
            std::env::temp_dir().join(format!("riviu-package-tools-{}", uuid::Uuid::new_v4()));
        let tools = root.join("android-package-tools");
        std::fs::create_dir_all(tools.join("jre/bin")).expect("jre dir");
        std::fs::write(
            tools
                .join("jre/bin")
                .join(if cfg!(windows) { "java.exe" } else { "java" }),
            b"java",
        )
        .expect("java");
        std::fs::write(tools.join("bundletool.jar"), b"jar").expect("jar");
        let resolved = resolve_android_package_tools(&root);
        assert_eq!(resolved.provenance, PackageToolsProvenance::Missing);
        assert!(resolved.java.is_none() && resolved.bundletool.is_none());
        std::fs::remove_dir_all(root).expect("cleanup");
    }
}
