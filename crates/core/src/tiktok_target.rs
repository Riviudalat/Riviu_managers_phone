//! Which TikTok build a given device can actually be driven against.
//!
//! There is no single answer, which is the whole point of this module. The iOS
//! bundle is one fixed id; Android has **two** regional builds with different
//! `content-desc` strings, and a device carries one or the other. A fleet-wide
//! constant is therefore a lie about half a mixed fleet — the same argument
//! AGENTS.md §9 already made for giving `supports_text_comments` a udid.
//!
//! Before this existed the id was a module constant in three desktop files, all
//! spelling the *iOS* bundle. It was passed to `start_interaction_session` and then
//! compared against `active_app_bundle()`, where on Android it could never match, so
//! the wait loop could only ever exit by timing out.

/// TikTok on iOS. One build, one id — a fact rather than a guess, which is why the
/// [`crate::driver::DeviceDriver`] default can return it without asking the device.
pub const IOS_TIKTOK_BUNDLE: &str = "com.ss.iphone.ugc.Ame";

/// Every Android TikTok build this project has measured labels for.
///
/// Derived from [`crate::tiktok_labels::TIKTOK_LABEL_SETS`] rather than written out
/// again: a build whose labels cannot be read is not a build that can be driven, so
/// the two lists must not be able to disagree.
pub fn measured_android_packages() -> impl Iterator<Item = &'static str> {
    crate::tiktok_labels::TIKTOK_LABEL_SETS
        .iter()
        .map(|set| set.package)
}

/// Whether `package` is an Android TikTok build with measured labels.
///
/// The iOS bundle is deliberately **not** one: it is in no label set, and no Android
/// phone can launch it.
pub fn is_measured_android_tiktok(package: &str) -> bool {
    measured_android_packages().any(|candidate| candidate == package)
}

/// The measured packages as an operator-facing list, for refusal messages.
pub fn measured_android_packages_list() -> String {
    let mut seen: Vec<&str> = Vec::new();
    for package in measured_android_packages() {
        if !seen.contains(&package) {
            seen.push(package);
        }
    }
    seen.join(", ")
}

/// Pick the one installed build, from what `pm list packages` reported.
///
/// `installed` is the raw stdout of one or more `pm list packages <candidate>` calls.
/// Returns `Err` with an operator-facing reason rather than guessing:
///
/// * nothing matched — the phone has no TikTok this project can drive;
/// * more than one matched — genuinely ambiguous, and the caller must break the tie
///   with the foreground package rather than picking the first.
pub fn resolve_installed_android_tiktok(installed: &str) -> Result<String, TargetResolution> {
    let mut found: Vec<&'static str> = Vec::new();
    for package in measured_android_packages() {
        // `pm list packages com.foo` matches by substring, so `com.foo.bar` also
        // comes back. Compare the whole line's payload, not just "contains".
        let present = installed.lines().any(|line| {
            line.trim()
                .strip_prefix("package:")
                .map(|value| value.trim() == package)
                .unwrap_or(false)
        });
        if present && !found.contains(&package) {
            found.push(package);
        }
    }
    match found.as_slice() {
        [] => Err(TargetResolution::NoneInstalled),
        [only] => Ok((*only).to_string()),
        many => Err(TargetResolution::Ambiguous(
            many.iter().map(|value| value.to_string()).collect(),
        )),
    }
}

/// Why a device's TikTok build could not be resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TargetResolution {
    NoneInstalled,
    Ambiguous(Vec<String>),
}

impl std::fmt::Display for TargetResolution {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoneInstalled => write!(
                formatter,
                "no TikTok build with measured labels is installed; expected one of: {}",
                measured_android_packages_list()
            ),
            Self::Ambiguous(found) => write!(
                formatter,
                "more than one measured TikTok build is installed ({}); the foreground package \
                 must break the tie",
                found.join(", ")
            ),
        }
    }
}

impl std::error::Error for TargetResolution {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_measured_android_builds_count() {
        assert!(is_measured_android_tiktok("com.ss.android.ugc.trill"));
        assert!(is_measured_android_tiktok("com.zhiliaoapp.musically"));
        // The iOS bundle is the default in `NurtureSettings` and no Android phone can
        // launch it, so it must never pass as a target here.
        assert!(!is_measured_android_tiktok(IOS_TIKTOK_BUNDLE));
        assert!(!is_measured_android_tiktok(""));
    }

    #[test]
    fn one_installed_build_resolves() {
        // The exact stdout shape measured on a Redmi Note 12, 11/08/2026.
        let stdout = "package:com.ss.android.ugc.trill\n";
        assert_eq!(
            resolve_installed_android_tiktok(stdout).expect("resolved"),
            "com.ss.android.ugc.trill"
        );
    }

    #[test]
    fn a_prefix_match_is_not_the_package() {
        // `pm list packages com.zhiliaoapp.musically` also returns
        // `com.zhiliaoapp.musically.go` on a phone that has the Lite build. Treating
        // that as the measured build would drive an app whose labels were never read.
        let stdout = "package:com.zhiliaoapp.musically.go\n";
        assert_eq!(
            resolve_installed_android_tiktok(stdout),
            Err(TargetResolution::NoneInstalled)
        );
    }

    #[test]
    fn no_tiktok_refuses_and_names_the_catalog() {
        let error = resolve_installed_android_tiktok("").expect_err("nothing installed");
        assert_eq!(error, TargetResolution::NoneInstalled);
        let message = error.to_string();
        assert!(message.contains("com.ss.android.ugc.trill"), "{message}");
        assert!(message.contains("com.zhiliaoapp.musically"), "{message}");
    }

    #[test]
    fn two_builds_are_ambiguous_rather_than_first_wins() {
        // Both regional builds can be side by side. Picking one silently would drive
        // whichever the catalog happened to list first, which is not a decision
        // anybody made.
        let stdout = "package:com.zhiliaoapp.musically\npackage:com.ss.android.ugc.trill\n";
        let error = resolve_installed_android_tiktok(stdout).expect_err("ambiguous");
        match error {
            TargetResolution::Ambiguous(found) => assert_eq!(found.len(), 2),
            other => panic!("expected ambiguity, got {other:?}"),
        }
        assert!(resolve_installed_android_tiktok(stdout)
            .unwrap_err()
            .to_string()
            .contains("foreground"));
    }

    #[test]
    fn the_catalog_and_the_allowlist_cannot_drift() {
        // The list is derived, not duplicated. If a label set is added, it becomes
        // drivable automatically — and if one is removed, it stops being drivable.
        for set in crate::tiktok_labels::TIKTOK_LABEL_SETS {
            assert!(is_measured_android_tiktok(set.package), "{}", set.package);
        }
    }
}
