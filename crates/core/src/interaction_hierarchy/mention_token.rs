//! Bind a measured picker's canonical handle to the token rendered by TikTok.
use crate::{ui_automation::tree::Tree, ElementBox, ElementQuery};

pub(super) fn rendered_after_pick(
    tree: &Tree,
    package: &str,
    version: &str,
    locale: &str,
    handle: &str,
    selected: &ElementBox,
    before: &str,
) -> Option<String> {
    if package != "com.zhiliaoapp.musically" || locale.split(['-', '_']).next() != Some("en") {
        return None;
    }
    let (handle_id, row_id, list_id, name_id) = match version {
        "45.4.3" => (":id/ns7", "nta", "nsf", ":id/nsm"),
        "45.7.3" => (":id/o28", "o3b", "o2g", ":id/o2n"),
        "46.0.41" => (":id/o72", "o87", "o7b", ":id/o7i"),
        "46.1.3" => (":id/o8v", "o_0", "o95", ":id/o9b"),
        "46.4.3" => (":id/ohb", "oig", "ohl", ":id/ohs"),
        _ => return None,
    };
    let matches = tree
        .matching(package, ElementQuery::ResourceIdSuffix(handle_id))
        .into_iter()
        .filter(|i| {
            tree.nodes[*i]
                .attr("text")
                .trim()
                .trim_start_matches('@')
                .eq_ignore_ascii_case(handle)
        })
        .collect::<Vec<_>>();
    let [index] = matches.as_slice() else {
        return None;
    };
    let node = &tree.nodes[*index];
    let rect = node.rect()?;
    if node.attr("class") != "android.widget.TextView"
        || (rect.x, rect.y, rect.width, rect.height)
            != (selected.x, selected.y, selected.width, selected.height)
    {
        return None;
    }
    let parent = node.parent?;
    let group = &tree.nodes[parent];
    if group.attr("resource-id") != format!("{package}:id/{row_id}")
        || group.attr("class") != "android.view.ViewGroup"
        || !group.rect().is_some_and(|r| r.enabled && r.clickable)
    {
        return None;
    }
    let list = &tree.nodes[group.parent?];
    if list.attr("resource-id") != format!("{package}:id/{list_id}")
        || list.attr("class") != "androidx.recyclerview.widget.RecyclerView"
    {
        return None;
    }
    let names = tree
        .matching(package, ElementQuery::ResourceIdSuffix(name_id))
        .into_iter()
        .filter(|i| tree.nodes[*i].parent == Some(parent))
        .collect::<Vec<_>>();
    let [name] = names.as_slice() else {
        return None;
    };
    if tree.nodes[*name].attr("class") != "android.widget.TextView" {
        return None;
    }
    let display = tree.nodes[*name].attr("text").trim();
    if display.is_empty()
        || display.chars().count() > 64
        || display.contains('@')
        || display.chars().any(char::is_control)
    {
        return None;
    }
    let query = format!("@{handle}");
    let prefix = before.trim_end().strip_suffix(&query)?;
    Some(format!("{prefix}@{display}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    const XML: &str = include_str!("../../fixtures/tiktok-mention/musically-45.7.3-picker.fixture");
    #[test]
    fn current_global_picker_tokens_bind_same_row_handle_to_display_name() {
        for (version, handle, display, source) in [
            (
                "45.4.3",
                "bn.i.trn",
                "Fixture Target",
                include_str!("../../fixtures/tiktok-mention/musically-45.4.3-picker.fixture"),
            ),
            (
                "46.4.3",
                "fixture.target",
                "Fixture Target",
                include_str!("../../fixtures/tiktok-mention/musically-46.4.3-picker.fixture"),
            ),
            (
                "46.0.41",
                "dongchoinoinoi_dalat",
                "Dong chơi nơi nơi",
                include_str!("../../fixtures/tiktok-mention/musically-46.0.41-picker.fixture"),
            ),
            (
                "46.1.3",
                "user9957537538059",
                "Tuyết",
                include_str!("../../fixtures/tiktok-mention/musically-46.1.3-picker.fixture"),
            ),
        ] {
            let tree = Tree::parse(crate::HierarchySourceSnapshot {
                generation: 1,
                xml: source.into(),
            })
            .unwrap();
            let selected = tree
                .nodes
                .iter()
                .find(|n| n.attr("text") == handle)
                .unwrap()
                .rect()
                .unwrap();
            let before = format!("Fixture comment @{handle}");
            assert_eq!(
                rendered_after_pick(
                    &tree,
                    "com.zhiliaoapp.musically",
                    version,
                    "en",
                    handle,
                    &selected,
                    &before
                ),
                Some(format!("Fixture comment @{display}"))
            );
            assert!(rendered_after_pick(
                &tree,
                "com.zhiliaoapp.musically",
                version,
                "en",
                "similar.handle",
                &selected,
                &before
            )
            .is_none());
            assert!(rendered_after_pick(
                &tree,
                "com.zhiliaoapp.musically",
                "unknown",
                "en",
                handle,
                &selected,
                &before
            )
            .is_none());
        }
    }
    #[test]
    fn nickname_token_binds_exact_picker_handle_and_unchanged_body() {
        let tree = Tree::parse(crate::HierarchySourceSnapshot {
            generation: 1,
            xml: XML.into(),
        })
        .unwrap();
        let selected = tree
            .nodes
            .iter()
            .find(|n| n.attr("text") == "ghin.lt.sng.sng")
            .unwrap()
            .rect()
            .unwrap();
        let before = tree
            .nodes
            .iter()
            .find(|n| n.attr("focused") == "true")
            .unwrap()
            .attr("text");
        let actual = rendered_after_pick(
            &tree,
            "com.zhiliaoapp.musically",
            "45.7.3",
            "en",
            "ghin.lt.sng.sng",
            &selected,
            before,
        )
        .unwrap();
        assert_eq!(
            actual,
            before.replace("@ghin.lt.sng.sng", "@Ghiền Đà Lạt Sương Sương")
        );
        assert!(rendered_after_pick(
            &tree,
            "com.zhiliaoapp.musically",
            "45.7.4",
            "en",
            "ghin.lt.sng.sng",
            &selected,
            before
        )
        .is_none());
        for raw in [
            XML.replace(":id/o3b", ":id/comment"),
            XML.replace("text=\"ghin.lt.sng.sng\"", "text=\"ghin.lt.sng.sngx\""),
            XML.replace("displayed=\"true\"", "displayed=\"false\""),
        ] {
            let changed = Tree::parse(crate::HierarchySourceSnapshot {
                generation: 2,
                xml: raw,
            })
            .unwrap();
            assert!(rendered_after_pick(
                &changed,
                "com.zhiliaoapp.musically",
                "45.7.3",
                "en",
                "ghin.lt.sng.sng",
                &selected,
                before
            )
            .is_none());
        }
    }
}
