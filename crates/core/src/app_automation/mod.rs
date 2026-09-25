//! Application-specific interpretation. The UI runtime does not depend on these adapters.
pub mod dialogs;
pub mod tiktok_roles;

/// Uses the same catalog and plans as production preflight. A catalog match is
/// never a substitute for unique targets and postconditions in the live session.
pub fn action_capabilities(
    udid: &str,
    package: &str,
    version: &str,
    locale: &str,
    ready: bool,
) -> crate::ipc_contract::DeviceActionCapabilities {
    use crate::ipc_contract::{
        ActionCapability, CapabilityEvidenceState as S, DeviceActionCapabilities,
    };
    use crate::tiktok_labels::controls_for_runtime;
    use crate::tiktok_labels::TikTokControl;
    let controls = controls_for_runtime(package, locale, version);
    let supported = controls.is_some();
    let labelled = |needed: &[TikTokControl]| {
        controls.is_some_and(|c| needed.iter().all(|control| c.label(*control).is_some()))
    };
    let plans = [
        ("feed", supported, false),
        ("search", supported, false),
        (
            "photo",
            controls.as_ref().is_some_and(|c| {
                crate::tiktok_composer::ComposerPlan::missing_for_carousel(c).is_empty()
            }),
            controls.as_ref().is_some_and(|c| {
                !c.adaptive()
                    && crate::tiktok_composer::ComposerPlan::missing_for_carousel(c).is_empty()
            }),
        ),
        (
            "video",
            crate::tiktok_composer::VideoPickerPlan::resolve_runtime(package, locale, version)
                .is_some(),
            crate::tiktok_composer::VideoPickerPlan::resolve(package, locale, version).is_some(),
        ),
        (
            "sound",
            crate::tiktok_sound::SoundPickerPlan::resolve_runtime(package, locale, version)
                .is_some(),
            crate::tiktok_sound::SoundPickerPlan::resolve(package, locale, version).is_some(),
        ),
        (
            "like",
            labelled(&[TikTokControl::Like, TikTokControl::Liked]),
            false,
        ),
        (
            "save",
            labelled(&[TikTokControl::Bookmark])
                || crate::tiktok_save::hierarchy::global_supported(package, version, locale),
            false,
        ),
        (
            "follow",
            crate::tiktok_follow_target::supported(package, version, locale),
            false,
        ),
        (
            "share",
            crate::tiktok_friend_share::supported(package, version, locale),
            false,
        ),
        (
            "feedFollow",
            package == crate::tiktok_follow_cleanup::MEASURED_FOLLOW_PACKAGE
                && version == crate::tiktok_follow_cleanup::MEASURED_FOLLOW_VERSION
                && locale.split(['-', '_']).next()
                    == Some(crate::tiktok_follow_cleanup::MEASURED_FOLLOW_LOCALE),
            false,
        ),
        (
            "mentionReply",
            labelled(&[
                TikTokControl::Comments,
                TikTokControl::CommentSend,
                TikTokControl::CommentReply,
            ]),
            false,
        ),
    ];
    DeviceActionCapabilities {
        udid: udid.into(),
        package: package.into(),
        version: version.into(),
        locale: locale.into(),
        actions: plans
            .into_iter()
            .map(|(action, available, measured)| {
                let (state, reason) = if !ready {
                    (
                        S::DeviceNotReady,
                        "Thiết bị chưa sẵn sàng hoặc đang có owner",
                    )
                } else if !available {
                    (
                        S::Unsupported,
                        "Chưa có adapter cho package/build/ngôn ngữ này",
                    )
                } else if measured {
                    (
                        S::Measured,
                        "Đã đo adapter; vẫn phải kiểm đúng account, target và hậu điều kiện",
                    )
                } else {
                    (
                        S::RuntimeProofRequired,
                        "Cần chứng minh target duy nhất và trạng thái trước/sau trong phiên",
                    )
                };
                ActionCapability {
                    action: action.into(),
                    state,
                    reason: reason.into(),
                }
            })
            .collect(),
    }
}

/// Static capability permits entering the runtime proof path, never a public tap.
pub fn require_actions(
    report: &crate::ipc_contract::DeviceActionCapabilities,
    requested: &[&str],
) -> anyhow::Result<()> {
    use crate::ipc_contract::CapabilityEvidenceState as S;
    let refusals = requested
        .iter()
        .filter_map(
            |requested| match report.actions.iter().find(|row| row.action == *requested) {
                Some(row) if matches!(row.state, S::Measured | S::RuntimeProofRequired) => None,
                Some(row) => Some(format!("{} / {}: {}", report.udid, requested, row.reason)),
                None => Some(format!(
                    "{} / {}: hành động chưa được khai báo",
                    report.udid, requested
                )),
            },
        )
        .collect::<Vec<_>>();
    anyhow::ensure!(refusals.is_empty(), "{}", refusals.join("; "));
    Ok(())
}

pub fn nurture_actions(settings: &crate::NurtureSettings) -> Vec<&'static str> {
    let mut actions = vec![
        if settings.feed_source == crate::types::NurtureFeedSource::Search {
            "search"
        } else {
            "feed"
        },
    ];
    if settings.like_enabled && settings.like_prob > 0 {
        actions.push("like");
    }
    if settings.save_enabled && settings.save_prob > 0 {
        actions.push("save");
    }
    if settings.follow_enabled && settings.follow_prob > 0 {
        actions.push("feedFollow");
    }
    if settings.comment_enabled && settings.comment_prob > 0 {
        actions.push("mentionReply");
    }
    actions
}

pub fn interaction_actions(actions: crate::InteractionActionSet) -> Vec<&'static str> {
    let mut requested = vec!["feed"];
    if actions.like {
        requested.push("like");
    }
    if actions.save {
        requested.push("save");
    }
    if actions.share {
        requested.push("share");
    }
    if actions.follow {
        requested.push("follow");
    }
    if actions.comment {
        requested.push("mentionReply");
    }
    requested
}

#[cfg(test)]
mod capability_preflight_tests {
    use super::*;
    #[test]
    fn profile_follow_does_not_admit_an_unmeasured_feed_follow_adapter() {
        let report = action_capabilities("phone", "com.zhiliaoapp.musically", "45.7.3", "en", true);
        let settings = crate::NurtureSettings {
            like_enabled: false,
            save_enabled: false,
            follow_enabled: true,
            follow_prob: 100,
            comment_enabled: false,
            ..Default::default()
        };
        assert!(require_actions(&report, &["follow"]).is_ok());
        assert!(require_actions(&report, &nurture_actions(&settings)).is_err());
    }
    #[test]
    fn shared_action_preflight_refuses_unknown_action_locked_phone_and_unmeasured_follow() {
        let report = action_capabilities("phone", "com.ss.android.ugc.trill", "38.3.2", "en", true);
        assert!(require_actions(&report, &["feed", "follow"]).is_ok());
        assert!(require_actions(&report, &["madeUp"]).is_err());
        let locked =
            action_capabilities("phone", "com.ss.android.ugc.trill", "38.3.2", "en", false);
        assert!(require_actions(&locked, &["feed"]).is_err());
        let global = action_capabilities("phone", "com.zhiliaoapp.musically", "45.7.3", "en", true);
        assert!(require_actions(&global, &["follow"]).is_ok());
        let unknown =
            action_capabilities("phone", "com.zhiliaoapp.musically", "45.7.4", "en", true);
        assert!(require_actions(&unknown, &["follow"]).is_err());
        let settings = crate::NurtureSettings {
            like_enabled: false,
            save_enabled: false,
            follow_enabled: false,
            comment_enabled: false,
            ..Default::default()
        };
        assert_eq!(nurture_actions(&settings), vec!["feed"]);
    }
}
use crate::ui_automation::{profile::CompatibilityPack, tree::Tree, AppContext};
use serde::{Deserialize, Serialize};

pub trait AppAdapter: Send + Sync {
    fn id(&self) -> &'static str;
    fn accepts(&self, package: &str) -> bool;
    fn classify(&self, tree: &Tree, app: &AppContext) -> String;
    fn pack(&self) -> CompatibilityPack;
}
pub struct TikTokAdapter;
impl AppAdapter for TikTokAdapter {
    fn id(&self) -> &'static str {
        "tiktok"
    }
    fn accepts(&self, p: &str) -> bool {
        matches!(p, "com.zhiliaoapp.musically" | "com.ss.android.ugc.trill")
    }
    fn pack(&self) -> CompatibilityPack {
        CompatibilityPack::parse(include_bytes!("tiktok.json")).expect("bundled compatibility pack")
    }
    fn classify(&self, tree: &Tree, app: &AppContext) -> String {
        classify_by_labels(
            tree,
            app,
            &[
                ("login", &["Log in", "Đăng nhập"]),
                ("caption", &["Post", "Đăng"]),
                (
                    "sound",
                    &["Sounds", "Âm thanh", "Add sound", "Thêm âm thanh"],
                ),
                ("profile", &["Edit profile", "Edit", "Sửa hồ sơ"]),
                (
                    "comments",
                    &[
                        "Add comment...",
                        "Add comment…",
                        "Thêm bình luận…",
                        "Reply",
                        "Trả lời",
                    ],
                ),
                ("share", &["Copy link", "Sao chép liên kết"]),
                (
                    "picker",
                    &["Albums", "All photos", "Photos", "Ảnh", "Recents"],
                ),
                ("composer", &["Upload", "Tải lên"]),
                ("feed", &["For You", "Dành cho bạn", "Following"]),
            ],
        )
    }
}
pub struct SettingsAdapter;
impl SettingsAdapter {
    pub fn android_version(&self, tree: &Tree, app: &AppContext) -> Option<String> {
        if !self.accepts(&app.package) {
            return None;
        }
        let labels = tree
            .nodes
            .iter()
            .enumerate()
            .filter(|(i, n)| {
                n.visible(&app.package)
                    && tree.ancestors_visible(*i)
                    && matches!(n.attr("text"), "Android version" | "Phiên bản Android")
            })
            .collect::<Vec<_>>();
        let [(index, label)] = labels.as_slice() else {
            return None;
        };
        let parent = label.parent?;
        let values = tree
            .nodes
            .iter()
            .enumerate()
            .filter(|(i, n)| {
                i != index
                    && tree.inside(*i, parent)
                    && n.visible(&app.package)
                    && tree.ancestors_visible(*i)
            })
            .map(|(_, n)| n.attr("text").trim())
            .filter(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit() || b == b'.'))
            .collect::<Vec<_>>();
        let [value] = values.as_slice() else {
            return None;
        };
        Some(value.to_string())
    }
}
impl AppAdapter for SettingsAdapter {
    fn id(&self) -> &'static str {
        "android-settings"
    }
    fn accepts(&self, p: &str) -> bool {
        p == "com.android.settings"
    }
    fn pack(&self) -> CompatibilityPack {
        CompatibilityPack::parse(include_bytes!("settings.json")).expect("bundled settings pack")
    }
    fn classify(&self, tree: &Tree, app: &AppContext) -> String {
        classify_by_labels(
            tree,
            app,
            &[
                ("device-info", &["Android version", "Phiên bản Android"]),
                ("settings", &["Settings", "Cài đặt"]),
            ],
        )
    }
}
fn classify_by_labels(tree: &Tree, app: &AppContext, screens: &[(&str, &[&str])]) -> String {
    let nodes = crate::ui_automation::resolver::visible_nodes(tree, app);
    screens
        .iter()
        .find(|(_, labels)| {
            nodes.iter().any(|n| {
                labels.iter().any(|s| {
                    n.text.eq_ignore_ascii_case(s) || n.description.eq_ignore_ascii_case(s)
                })
            })
        })
        .map(|(screen, _)| screen.to_string())
        .unwrap_or_else(|| "unknown".into())
}
pub fn adapter(package: &str) -> Option<Box<dyn AppAdapter>> {
    if TikTokAdapter.accepts(package) {
        Some(Box::new(TikTokAdapter))
    } else if SettingsAdapter.accepts(package) {
        Some(Box::new(SettingsAdapter))
    } else {
        None
    }
}

pub fn observed_language(tree: &Tree, app: &AppContext) -> Option<String> {
    let nodes = crate::ui_automation::resolver::visible_nodes(tree, app);
    let en = [
        "Profile",
        "For You",
        "Edit",
        "Share",
        "Copy link",
        "Post",
        "Next",
        "Photos",
        "Sounds",
    ];
    let vi = [
        "Hồ sơ",
        "Dành cho bạn",
        "Sửa hồ sơ",
        "Chia sẻ",
        "Sao chép liên kết",
        "Đăng",
        "Tiếp",
        "Ảnh",
        "Âm thanh",
    ];
    let score = |labels: &[&str]| {
        nodes
            .iter()
            .filter(|n| {
                labels.iter().any(|l| {
                    n.text.eq_ignore_ascii_case(l) || n.description.eq_ignore_ascii_case(l)
                })
            })
            .count()
    };
    match (score(&en), score(&vi)) {
        (a, 0) if a >= 2 => Some("en".into()),
        (0, b) if b >= 2 => Some("vi".into()),
        _ => None,
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AppTarget {
    pub app_id: String,
    pub kind: String,
    pub key: String,
    pub url: Option<String>,
}
impl From<&crate::interaction::ResolvedTikTokTarget> for AppTarget {
    fn from(value: &crate::interaction::ResolvedTikTokTarget) -> Self {
        Self {
            app_id: "tiktok".into(),
            kind: "post".into(),
            key: value.target_key.clone(),
            url: Some(value.normalized_url.clone()),
        }
    }
}
