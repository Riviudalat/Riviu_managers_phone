//! Which TikTok builds this fleet may run **effects** on.
//!
//! Owner decision, 09/10/2026: the whole fleet runs exactly one build — TikTok Global
//! `com.zhiliaoapp.musically` `45.7.3` with an English UI. The label catalogue
//! ([`crate::tiktok_labels`]) still knows other builds, and
//! [`crate::tiktok_labels::controls_for_runtime`] still sends an unmeasured version of a
//! known (package, language) pair down an adaptive route. That is right for *reads* — a phone
//! that auto-updated must still be able to verify a post it already submitted — and wrong for
//! *effects*: an unmeasured build could reach Post, Like, Comment or Follow.
//!
//! So effects ask this allowlist first, at two moments:
//!
//! * at planning, through [`crate::app_automation::action_capabilities`] (the capability
//!   chips, interaction/nurture admission, publish preflight), which reports
//!   [`crate::ipc_contract::CapabilityEvidenceState::BuildNotAllowed`];
//! * again when the effect session opens ([`require_fleet_build_in_session`]), because TikTok
//!   can update itself between the plan and the first tap.
//!
//! Read-only work — link verification of submitted posts, account reads, diagnostics — never
//! asks this module.
//!
//! The allowlist is a code constant on purpose: there is no settings override, so changing
//! the fleet build is a reviewed code change and a release, not a toggle.

use std::fmt;

/// One build the fleet may run effects on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FleetBuild {
    pub package: &'static str,
    /// The `versionName` exactly as `dumpsys package` reports it.
    pub version: &'static str,
    /// The UI language: the part of the device locale before `-`/`_`, lowercase.
    pub language: &'static str,
    /// How the operator names this build, in Vietnamese.
    pub label: &'static str,
}

/// The fleet allowlist. Exactly one build, by owner decision (09/10/2026).
pub const TIKTOK_FLEET_BUILDS: &[FleetBuild] = &[FleetBuild {
    package: "com.zhiliaoapp.musically",
    version: "45.7.3",
    language: "en",
    label: "TikTok Global 45.7.3 tiếng Anh",
}];

/// An allowlist of builds. Production uses [`TikTokFleetPolicy::FLEET`]; a unit test that
/// exercises adapter behaviour on another measured build passes its own allowlist through
/// [`TikTokFleetPolicy::allowing`] instead of deleting that coverage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TikTokFleetPolicy {
    builds: &'static [FleetBuild],
}

impl TikTokFleetPolicy {
    pub const FLEET: Self = Self {
        builds: TIKTOK_FLEET_BUILDS,
    };

    pub const fn allowing(builds: &'static [FleetBuild]) -> Self {
        Self { builds }
    }

    /// True only for an exact package, an exact (trimmed) `versionName` and a matching UI
    /// language. An unread version or locale is never admitted.
    pub fn admits(&self, package: &str, version: &str, locale: &str) -> bool {
        let version = version.trim();
        let language = language_of(locale);
        self.builds.iter().any(|build| {
            build.package == package.trim()
                && build.version == version
                && language.as_deref() == Some(build.language)
        })
    }

    pub fn check(&self, package: &str, version: &str, locale: &str) -> Result<(), BuildNotAllowed> {
        if self.admits(package, version, locale) {
            Ok(())
        } else {
            Err(BuildNotAllowed {
                package: package.trim().to_owned(),
                version: version.trim().to_owned(),
                locale: locale.trim().to_owned(),
                policy: *self,
            })
        }
    }
}

fn language_of(locale: &str) -> Option<String> {
    let language = locale.trim().split(['-', '_']).next()?.trim();
    (!language.is_empty()).then(|| language.to_ascii_lowercase())
}

/// A refusal before any device effect: the phone's TikTok is not on the fleet allowlist.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildNotAllowed {
    pub package: String,
    pub version: String,
    pub locale: String,
    policy: TikTokFleetPolicy,
}

impl fmt::Display for BuildNotAllowed {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let or_unread = |value: &str, unread: &'static str| {
            if value.is_empty() {
                unread.to_owned()
            } else {
                value.to_owned()
            }
        };
        let allowed = self
            .policy
            .builds
            .iter()
            .map(|build| build.label)
            .collect::<Vec<_>>()
            .join(" hoặc ");
        let downgrade = self
            .policy
            .builds
            .iter()
            .map(|build| build.version)
            .collect::<Vec<_>>()
            .join(" hoặc ");
        write!(
            formatter,
            "TikTok trên máy đang là {} {} ({}); đội máy chỉ chạy {}. Chưa thực hiện thao tác \
             nào. Hạ về {} trong Trung tâm ứng dụng rồi kiểm tra lại.",
            or_unread(&self.package, "chưa rõ gói"),
            or_unread(&self.version, "chưa đọc được phiên bản"),
            or_unread(&self.locale, "chưa đọc được ngôn ngữ"),
            allowed,
            downgrade,
        )
    }
}

impl std::error::Error for BuildNotAllowed {}

/// The planning-time check against the fleet allowlist.
pub fn require_fleet_build(
    package: &str,
    version: &str,
    locale: &str,
) -> Result<(), BuildNotAllowed> {
    TikTokFleetPolicy::FLEET.check(package, version, locale)
}

/// The session-open check: reads the build off the session that is about to act, so an app
/// that updated itself after planning is refused before the first tap.
///
/// An unreadable language or version is refused, never assumed.
pub async fn require_fleet_build_in_session(
    session: &dyn crate::UiSession,
    package: &str,
) -> Result<(), BuildNotAllowed> {
    let language = session.ui_language().await.unwrap_or_default();
    let version = session.app_version(package).await.unwrap_or_default();
    require_fleet_build(package, &version, &language)
}

#[cfg(test)]
pub(crate) mod fixtures {
    /// A session that reports a build and refuses every input, so a test proves the
    /// check ran before any effect.
    pub(crate) struct BuildSession {
        pub package: &'static str,
        pub language: Option<&'static str>,
        pub version: Option<&'static str>,
    }

    #[async_trait::async_trait]
    impl crate::UiSession for BuildSession {
        async fn tap(&self, _: crate::TapPoint) -> anyhow::Result<()> {
            panic!("build check dispatched a tap")
        }
        async fn swipe(&self, _: crate::SwipeGesture) -> anyhow::Result<()> {
            panic!("build check dispatched a swipe")
        }
        async fn type_text(&self, _: &str) -> anyhow::Result<()> {
            panic!("build check typed text")
        }
        async fn home(&self) -> anyhow::Result<()> {
            panic!("build check pressed Home")
        }
        async fn find_and_tap(&self, _: &str) -> anyhow::Result<()> {
            panic!("build check dispatched a tap")
        }
        async fn assert_visible(&self, _: &str) -> anyhow::Result<()> {
            Ok(())
        }
        fn supports_element_bounds(&self) -> bool {
            true
        }
        async fn active_app_bundle(&self) -> anyhow::Result<String> {
            Ok(self.package.into())
        }
        async fn ui_language(&self) -> Option<String> {
            self.language.map(str::to_owned)
        }
        async fn app_version(&self, _: &str) -> Option<String> {
            self.version.map(str::to_owned)
        }
        fn stream_url(&self) -> Option<String> {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GLOBAL: &str = "com.zhiliaoapp.musically";
    const TRILL: &str = "com.ss.android.ugc.trill";

    /// Builds this farm has run or measured, none of which the owner allows for effects.
    const REFUSED: &[(&str, &str, &str)] = &[
        (GLOBAL, "46.0.41", "en"),
        (GLOBAL, "46.4.3", "en"),
        (GLOBAL, "47.1.4", "en"),
        (TRILL, "38.3.2", "en"),
        (GLOBAL, "45.7.3", "vi"),
    ];

    #[test]
    fn only_global_45_7_3_in_english_is_allowed() {
        assert!(require_fleet_build(GLOBAL, "45.7.3", "en").is_ok());
        assert!(require_fleet_build(GLOBAL, "45.7.3", "en-US").is_ok());
        assert!(require_fleet_build(GLOBAL, " 45.7.3 ", "en_GB").is_ok());
        for (package, version, locale) in REFUSED.iter().copied().chain([
            (GLOBAL, "", "en"),
            (GLOBAL, "45.7.3", ""),
            (GLOBAL, "45.7.30", "en"),
        ]) {
            let refusal = require_fleet_build(package, version, locale)
                .expect_err("a build outside the allowlist must be refused");
            assert_eq!(refusal.package, package);
        }
    }

    #[test]
    fn refusal_names_the_phone_build_and_the_fleet_build_in_vietnamese() {
        let refusal = require_fleet_build(GLOBAL, "46.4.3", "en-US").unwrap_err();
        assert_eq!(
            refusal.to_string(),
            "TikTok trên máy đang là com.zhiliaoapp.musically 46.4.3 (en-US); đội máy chỉ chạy \
             TikTok Global 45.7.3 tiếng Anh. Chưa thực hiện thao tác nào. Hạ về 45.7.3 trong \
             Trung tâm ứng dụng rồi kiểm tra lại."
        );
        let unread = require_fleet_build(GLOBAL, "", "").unwrap_err().to_string();
        assert!(unread.contains("chưa đọc được phiên bản"), "{unread}");
        assert!(unread.contains("chưa đọc được ngôn ngữ"), "{unread}");
    }

    #[test]
    fn an_explicit_policy_admits_another_measured_build_for_adapter_tests() {
        const TRILL_38: &[FleetBuild] = &[FleetBuild {
            package: TRILL,
            version: "38.3.2",
            language: "en",
            label: "Trill 38.3.2 tiếng Anh",
        }];
        let policy = TikTokFleetPolicy::allowing(TRILL_38);
        assert!(policy.admits(TRILL, "38.3.2", "en"));
        assert!(!policy.admits(GLOBAL, "45.7.3", "en"));
    }

    #[tokio::test]
    async fn the_session_check_reads_the_build_and_refuses_before_any_input() {
        let allowed = fixtures::BuildSession {
            package: GLOBAL,
            language: Some("en"),
            version: Some("45.7.3"),
        };
        assert!(require_fleet_build_in_session(&allowed, GLOBAL)
            .await
            .is_ok());
        for (package, version, locale) in REFUSED.iter().copied() {
            let session = fixtures::BuildSession {
                package,
                language: Some(locale),
                version: Some(version),
            };
            let refusal = require_fleet_build_in_session(&session, package)
                .await
                .expect_err("session check must refuse");
            assert!(refusal.to_string().contains("Chưa thực hiện thao tác nào"));
        }
        let unread = fixtures::BuildSession {
            package: GLOBAL,
            language: None,
            version: None,
        };
        assert!(require_fleet_build_in_session(&unread, GLOBAL)
            .await
            .is_err());
    }

    /// The allowlist gates effects only. Read paths keep resolving labels and the link
    /// verification plan on a phone that auto-updated, so in-flight posts stay verifiable.
    #[test]
    fn verification_reads_still_resolve_on_builds_the_allowlist_refuses() {
        for (package, version, locale) in [(GLOBAL, "46.4.3", "en"), (GLOBAL, "47.1.4", "en")] {
            assert!(require_fleet_build(package, version, locale).is_err());
            assert!(
                crate::tiktok_labels::controls_for_runtime(package, locale, version).is_some(),
                "{package} {version}: read labels must still resolve"
            );
            assert!(
                crate::tiktok_share::PublishVerificationPlan::for_runtime(package, locale, version)
                    .is_ok(),
                "{package} {version}: link verification must not be refused"
            );
        }
    }
}
