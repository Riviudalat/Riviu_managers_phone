//! Structural roles for layouts whose resource IDs changed. Engines verify every transition.
use crate::{ui_automation::tree::Tree, ElementBox};

pub fn indices(tree: &Tree, package: &str, role: &str) -> Vec<usize> {
    let visible = |i: usize| {
        tree.nodes[i].visible(package)
            && tree.ancestors_visible(i)
            && tree.nodes[i].rect().is_some()
    };
    let text = |i: usize, labels: &[&str]| {
        labels.iter().any(|l| {
            [
                tree.nodes[i].attr("text"),
                tree.nodes[i].attr("content-desc"),
            ]
            .iter()
            .any(|s| s.trim().eq_ignore_ascii_case(l))
        })
    };
    let has = |labels: &[&str]| {
        tree.nodes
            .iter()
            .enumerate()
            .any(|(i, _)| visible(i) && text(i, labels))
    };
    let editor = has(&["Add sound", "Thêm âm thanh", "Next", "Tiếp"]);
    let post = has(&["Post", "Đăng"]);
    let picker = has(&["Photos", "Ảnh", "Videos", "Select multiple", "Chọn nhiều"]);
    let profile = has(&["Edit", "Edit profile", "Sửa hồ sơ"]);
    let inputs: Vec<_> = tree
        .nodes
        .iter()
        .enumerate()
        .filter(|(i, n)| visible(*i) && n.attr("class") == "android.widget.EditText")
        .map(|(i, _)| i)
        .collect();
    let mut found = Vec::new();
    for (i, node) in tree.nodes.iter().enumerate() {
        if !visible(i) {
            continue;
        }
        let value = node.attr("text").trim();
        let id = node.attr("resource-id");
        let image_sibling = || {
            node.parent.is_some_and(|p| {
                tree.nodes.iter().enumerate().any(|(j, n)| {
                    j != i && tree.inside(j, p) && n.attr("class") == "android.widget.ImageView"
                })
            })
        };
        let matched = match role {
            "profile" => text(i, &["Profile", "Hồ sơ"]),
            "create" => text(
                i,
                &[
                    "Create",
                    "Create a video",
                    "Create video",
                    "Tạo",
                    "Tạo video",
                ],
            ),
            "gallery" => {
                !picker
                    && !post
                    && (text(i, &["Upload", "Tải lên"]) || id.ends_with(":id/upload_hot_area"))
            }
            "shutter" => {
                !picker
                    && !post
                    && (text(i, &["Record video", "Take photo", "Quay video", "Chụp ảnh"])
                        || id.ends_with(":id/upload_hot_area"))
            }
            "photos" => picker && text(i, &["Photos", "Ảnh"]),
            "multiSelect" => picker && text(i, &["Select multiple", "Chọn nhiều"]),
            "pickerNext" => {
                picker
                    && (value.starts_with("Next (")
                        || value.starts_with("Tiếp (")
                        || text(i, &["Next", "Tiếp"]))
            }
            "editorNext" => editor && !picker && !post && text(i, &["Next", "Tiếp"]),
            "post" => post && text(i, &["Post", "Đăng"]) && node.attr("class").contains("Button"),
            "caption" => {
                post && node.attr("class") == "android.widget.EditText"
                    && (inputs.len() == 1
                        || value.contains("description")
                        || value.contains("caption")
                        || value.contains("mô tả"))
            }
            "postTile" => profile && id.ends_with(":id/cover") && node.attr("clickable") == "true",
            "commentSend" => {
                !post
                    && !picker
                    && !inputs.is_empty()
                    && text(
                        i,
                        &[
                            "Post comment",
                            "Send comment",
                            "Gửi bình luận",
                            "Send",
                            "Gửi",
                        ],
                    )
            }
            "album" => {
                picker
                    && (id.ends_with(":id/tv_title")
                        || text(i, &["Albums", "All", "Recents", "Gần đây"]))
                    && image_sibling()
            }
            "selector" => {
                picker
                    && node.attr("class") == "android.widget.Button"
                    && node.attr("clickable") == "true"
                    && (value.is_empty() || value.parse::<u32>().is_ok())
                    && image_sibling()
            }
            _ => false,
        };
        if matched {
            found.push(i);
        }
    }
    found
}
pub fn locate(tree: &Tree, package: &str, role: &str) -> Vec<ElementBox> {
    indices(tree, package, role)
        .into_iter()
        .filter_map(|i| tree.nodes[i].rect())
        .collect()
}
