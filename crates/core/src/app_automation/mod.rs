//! Application-specific interpretation. The UI runtime does not depend on these adapters.
pub mod tiktok_roles;
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
