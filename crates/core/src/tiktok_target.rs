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

/// The package that owns the runtime permission question on the measured Android 9 phones.
pub const RUNTIME_PERMISSION_PACKAGE: &str = "com.google.android.packageinstaller";

/// Whether `focused` (`package/activity`, as `mCurrentFocus` names it) is Android's runtime
/// permission question standing over a launch of the measured TikTok `target`.
///
/// Interaction acceptance on box 31-50, Global 45.7.3, 09/10/2026: ce0417141550322202 and
/// ce051715e9f964e10d launched TikTok and then read
/// `com.google.android.packageinstaller` for the whole eight-second confirmation, so the
/// launch was refused as `activity launch ACK timed out` before any popup step could Deny.
/// The launch had worked; the question lives in TikTok's own task. Only that exact package
/// and `GrantPermissionsActivity` count; any other foreign window still blocks. This is
/// launch evidence only — nothing here answers the question, and Allow is never pressed.
pub fn runtime_permission_over_measured_tiktok(target: &str, focused: &str) -> bool {
    let Some((package, activity)) = focused.trim().split_once('/') else {
        return false;
    };
    is_measured_android_tiktok(target)
        && package == RUNTIME_PERMISSION_PACKAGE
        && activity.rsplit('.').next() == Some("GrantPermissionsActivity")
}

/// What a foreground read means for a loop bound to one TikTok build.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TikTokForeground {
    /// The bound build itself.
    TikTok,
    /// The measured runtime permission question over the bound build. TikTok is still
    /// there; the shared popup step owns the question and may only Deny it.
    PermissionWindow,
    /// Anything else, including a foreground that could not be named.
    Other,
}

/// How long a permission-package foreground whose focused window does not prove
/// `GrantPermissionsActivity` is re-read before it counts as [`TikTokForeground::Other`].
///
/// Box 31-50, 10/10/2026 (nurture on ce031713aadf361905, run box3150d, 21:34:38-39): right
/// after our own Deny the package read still named the permission package while the dialog
/// faded out or the second question (with "Don't ask again") animated in. The focused
/// window in that second is not evidence either way, so the read is repeated under the
/// caller's deadline until the question is proven or TikTok itself is back. Nothing here
/// answers the question, and Allow is never pressed.
pub const PERMISSION_TRANSITION_WINDOW: std::time::Duration = std::time::Duration::from_secs(3);
/// Gap between two reads inside [`PERMISSION_TRANSITION_WINDOW`].
const PERMISSION_TRANSITION_POLL: std::time::Duration = std::time::Duration::from_millis(450);

/// Classify `active` (an [`crate::UiSession::active_app_bundle`] read) against `target`.
///
/// Box 31-50, Global 45.7.3, 10/10/2026: the contacts question kept coming back over every
/// TikTok start, and loops that compared the foreground package with the TikTok package
/// bailed as "left TikTok" before any popup step could Deny it. Only the permission
/// package named with `GrantPermissionsActivity` counts; an unreadable activity, a missed
/// deadline or any other window stays [`TikTokForeground::Other`].
///
/// A permission package whose focused window does not prove the question is re-read for
/// [`PERMISSION_TRANSITION_WINDOW`] at most (and never past `deadline`): it becomes
/// [`TikTokForeground::TikTok`] when the package read returns to `target`,
/// [`TikTokForeground::PermissionWindow`] once the activity is proven, and
/// [`TikTokForeground::Other`] when the window runs out or any other package shows up.
pub async fn foreground_is_tiktok_or_permission_window(
    session: &dyn crate::UiSession,
    target: &str,
    active: &str,
    deadline: tokio::time::Instant,
) -> TikTokForeground {
    if active == target {
        return TikTokForeground::TikTok;
    }
    if active != RUNTIME_PERMISSION_PACKAGE {
        return TikTokForeground::Other;
    }
    let started = tokio::time::Instant::now();
    let transition_until = (started + PERMISSION_TRANSITION_WINDOW).min(deadline);
    let mut reads: u32 = 0;
    loop {
        match tokio::time::timeout_at(deadline, session.active_app_component()).await {
            Ok(Ok(focused)) if runtime_permission_over_measured_tiktok(target, &focused) => {
                if reads > 0 {
                    tracing::info!(
                        package = target,
                        reads,
                        elapsed_ms = started.elapsed().as_millis() as u64,
                        "hộp thoại quyền hệ thống đã hiện rõ sau khi chuyển cảnh; bước popup chung sẽ trả lời"
                    );
                }
                return TikTokForeground::PermissionWindow;
            }
            Ok(_) => {}
            Err(_) => return TikTokForeground::Other,
        }
        if tokio::time::Instant::now() >= transition_until {
            return TikTokForeground::Other;
        }
        if reads == 0 {
            tracing::info!(
                package = target,
                window_ms = PERMISSION_TRANSITION_WINDOW.as_millis() as u64,
                "foreground là gói quyền hệ thống nhưng cửa sổ chưa rõ (đang chuyển cảnh) — không bấm gì; đọc lại foreground"
            );
        }
        reads = reads.saturating_add(1);
        tokio::time::sleep(
            PERMISSION_TRANSITION_POLL
                .min(transition_until.saturating_duration_since(tokio::time::Instant::now())),
        )
        .await;
        match tokio::time::timeout_at(deadline, session.active_app_bundle()).await {
            Ok(Ok(package)) if package == target => {
                tracing::info!(
                    package = target,
                    reads,
                    elapsed_ms = started.elapsed().as_millis() as u64,
                    "TikTok đã trở lại foreground sau khi hộp thoại quyền hệ thống chuyển cảnh"
                );
                return TikTokForeground::TikTok;
            }
            Ok(Ok(package)) if package == RUNTIME_PERMISSION_PACKAGE => {}
            _ => return TikTokForeground::Other,
        }
    }
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
