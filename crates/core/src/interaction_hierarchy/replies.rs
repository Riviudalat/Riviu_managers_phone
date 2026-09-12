//! Expand a reply group through its clickable ancestor, scoped to the exact root row.
use super::*;
use crate::ui_automation::tree::Tree;

pub(crate) fn expand_target(
    tree: &Tree,
    package: &str,
    root: &CommentLocatorIdentity,
) -> anyhow::Result<Option<ElementBox>> {
    let bodies = tree.matching(
        package,
        ElementQuery::Text {
            value: &root.text,
            exact: true,
        },
    );
    anyhow::ensure!(
        bodies.len() <= 1,
        "Nhiều bình luận trùng nội dung gốc; chưa mở replies"
    );
    let Some(&body_id) = bodies.first() else {
        return Ok(None);
    };
    let body = tree.nodes[body_id].rect().context("Root bounds mất")?;
    // The same measured author/body relationship used by locate_parent_in_elements.
    let authors: Vec<_> = tree
        .matching(package, ElementQuery::ClassName(COMMENT_AUTHOR_CLASS))
        .into_iter()
        .filter_map(|i| tree.nodes[i].rect())
        .filter(|a| {
            let text = a.description.as_deref().unwrap_or_default();
            !text.is_empty()
                && !is_reply_label(text)
                && !is_count_label(text)
                && bottom(a) <= body.y + ABOVE_SLACK
                && a.y >= body.y - AUTHOR_REACH
                && (a.x - body.x).abs() <= AUTHOR_LEFT_SLACK
        })
        .collect();
    let Some(author) = authors.iter().min_by(|a, b| {
        (body.y - bottom(a))
            .abs()
            .total_cmp(&(body.y - bottom(b)).abs())
    }) else {
        return Ok(None);
    };
    if author.description.as_deref() != Some(root.author_label.as_str()) {
        return Ok(None);
    }
    // On SEA38.3.2 the expander is a sibling row, not a descendant of the root
    // body. The next comment body bounds this root's group (including a repeated author).
    let body_resource = tree.nodes[body_id].attr("resource-id");
    anyhow::ensure!(
        !body_resource.is_empty(),
        "Root thiếu định danh vùng nội dung"
    );
    let next_body_y = tree
        .nodes
        .iter()
        .enumerate()
        .filter(|(i, n)| {
            *i != body_id
                && n.visible(package)
                && tree.ancestors_visible(*i)
                && n.attr("resource-id") == body_resource
        })
        .filter_map(|(_, n)| n.rect())
        .filter(|r| r.y > body.y)
        .map(|r| r.y)
        .min_by(f64::total_cmp)
        .unwrap_or(f64::INFINITY);
    let labels: Vec<_> = tree
        .nodes
        .iter()
        .enumerate()
        .filter(|(i, n)| {
            n.visible(package) && tree.ancestors_visible(*i) && {
                let text = n.attr("text").to_lowercase();
                (text.starts_with("view ") && text.contains("repl"))
                    || (text.starts_with("xem ") && text.contains("trả lời"))
            }
        })
        .filter(|(_, n)| {
            n.rect().is_some_and(|r| {
                r.enabled && r.y >= bottom(&body) && r.y < next_body_y && r.x >= body.x
            })
        })
        .map(|(i, _)| i)
        .collect();
    anyhow::ensure!(
        labels.len() <= 1,
        "Nhiều nút mở replies thuộc vùng bình luận gốc"
    );
    let Some(&label) = labels.first() else {
        return Ok(None);
    };
    let mut current = Some(label);
    for _ in 0..4 {
        let Some(index) = current else {
            break;
        };
        let node = &tree.nodes[index];
        if !node.visible(package)
            || !tree.ancestors_visible(index)
            || node.attr("enabled") != "true"
        {
            break;
        }
        if node.attr("clickable") == "true" {
            let rect = node.rect().context("Reply expander bounds mất")?;
            anyhow::ensure!(
                rect.y >= bottom(&body) && bottom(&rect) <= next_body_y,
                "Vùng bấm replies vượt sang bình luận khác"
            );
            return Ok(Some(rect));
        }
        current = node.parent;
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    const PKG: &str = "com.ss.android.ugc.trill";
    fn fixture(other: &str) -> Tree {
        let xml = format!(
            r#"<hierarchy><node package="{PKG}">
        <node package="{PKG}" class="android.widget.Button" text="Same author" bounds="[155,1200][600,1250]"/>
        <node package="{PKG}" class="android.widget.TextView" resource-id="{PKG}:id/body" text="root" bounds="[155,1262][1048,1373]"/>
        <node package="{PKG}" class="android.widget.LinearLayout" clickable="true" enabled="true" bounds="[226,1453][457,1532]">
          <node package="{PKG}" class="android.widget.Button" text="View 1 reply" clickable="false" enabled="true" bounds="[226,1458][420,1500]"/>
        </node>{other}
        <node package="{PKG}" class="android.widget.Button" text="Same author" bounds="[155,1580][600,1630]"/>
        <node package="{PKG}" class="android.widget.TextView" resource-id="{PKG}:id/body" text="another root" bounds="[155,1640][1048,1750]"/>
        <node package="{PKG}" class="android.widget.Button" text="View 2 replies" clickable="true" enabled="true" bounds="[226,1810][457,1860]"/>
        </node></hierarchy>"#
        );
        Tree::parse(crate::HierarchySourceSnapshot { generation: 1, xml }).unwrap()
    }
    fn root() -> CommentLocatorIdentity {
        CommentLocatorIdentity {
            author_label: "Same author".into(),
            text: "root".into(),
            locator_version: "test".into(),
            frame_sha256: "test".into(),
        }
    }
    #[test]
    fn nonclickable_expander_uses_its_clickable_parent_with_repeated_author() {
        let target = expand_target(&fixture(""), PKG, &root()).unwrap().unwrap();
        assert_eq!(
            (target.x, target.y, target.width, target.height),
            (226.0, 1453.0, 231.0, 79.0)
        );
    }
    #[test]
    fn duplicate_root_or_wrong_author_never_selects_an_expander() {
        let duplicate =
            format!(r#"<node package="{PKG}" text="root" bounds="[155,1400][1048,1440]"/>"#);
        assert!(expand_target(&fixture(&duplicate), PKG, &root()).is_err());
        let mut wrong = root();
        wrong.author_label = "Wrong author".into();
        assert!(expand_target(&fixture(""), PKG, &wrong).unwrap().is_none());
    }
    #[test]
    fn competing_expander_in_same_row_refuses() {
        let other = format!(
            r#"<node package="{PKG}" text="View 3 replies" enabled="true" bounds="[226,1540][457,1580]"/>"#
        );
        assert!(expand_target(&fixture(&other), PKG, &root()).is_err());
    }
}
