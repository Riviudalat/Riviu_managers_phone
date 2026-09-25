//! Stable element identity shared by the inspector, agent bridge and recorded Flow.
use super::tree::Tree;
use serde::{Deserialize, Serialize};

const SEMANTIC_SELECTOR_SCHEMA_VERSION: u32 = 2;
const MAX_ANCESTOR_DEPTH: u8 = 4;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AncestorConstraint {
    pub max_depth: u8,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource_id_suffix: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub class_name: Option<String>,
}

impl AncestorConstraint {
    fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            (1..=MAX_ANCESTOR_DEPTH).contains(&self.max_depth),
            "selector_ancestor_depth_invalid"
        );
        anyhow::ensure!(
            self.resource_id
                .as_ref()
                .is_some_and(|value| !value.trim().is_empty())
                || self
                    .resource_id_suffix
                    .as_ref()
                    .is_some_and(|value| !value.trim().is_empty())
                || self
                    .class_name
                    .as_ref()
                    .is_some_and(|value| !value.trim().is_empty()),
            "selector_ancestor_identity_missing"
        );
        for value in [
            &self.resource_id,
            &self.resource_id_suffix,
            &self.class_name,
        ]
        .into_iter()
        .flatten()
        {
            anyhow::ensure!(
                !value.trim().is_empty() && value.len() <= 2048,
                "selector_ancestor_value_invalid"
            );
        }
        Ok(())
    }

    fn matches(&self, node: &super::tree::Node) -> bool {
        self.resource_id
            .as_ref()
            .is_none_or(|value| node.attr("resource-id") == value)
            && self
                .resource_id_suffix
                .as_ref()
                .is_none_or(|value| node.attr("resource-id").ends_with(value))
            && self
                .class_name
                .as_ref()
                .is_none_or(|value| node.attr("class") == value)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum ElementActionTarget {
    ClickableAncestor { ancestor: AncestorConstraint },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ElementSelector {
    pub package: String,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub resource_id: Option<String>,
    #[serde(default)]
    pub class_name: Option<String>,
    /// Missing means the legacy exact-match contract. Version 2 is opt-in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema_version: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_prefix: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description_prefix: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<AncestorConstraint>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action_target: Option<ElementActionTarget>,
}
impl ElementSelector {
    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            !self.package.trim().is_empty() && self.package.len() <= 256,
            "selector_package_invalid"
        );
        anyhow::ensure!(
            [
                &self.text,
                &self.description,
                &self.resource_id,
                &self.text_prefix,
                &self.description_prefix,
            ]
            .iter()
            .any(|v| v.as_ref().is_some_and(|s| !s.trim().is_empty())),
            "selector_identity_missing"
        );
        for value in [
            &self.text,
            &self.description,
            &self.resource_id,
            &self.class_name,
            &self.text_prefix,
            &self.description_prefix,
        ]
        .into_iter()
        .flatten()
        {
            anyhow::ensure!(
                !value.trim().is_empty() && value.len() <= 2048,
                "selector_value_invalid"
            );
        }
        let uses_v2 = self.text_prefix.is_some()
            || self.description_prefix.is_some()
            || self.scope.is_some()
            || self.action_target.is_some();
        anyhow::ensure!(
            (!uses_v2 && self.schema_version.is_none())
                || (uses_v2 && self.schema_version == Some(SEMANTIC_SELECTOR_SCHEMA_VERSION)),
            "selector_schema_version_invalid"
        );
        anyhow::ensure!(
            self.text.is_none() || self.text_prefix.is_none(),
            "selector_text_mode_conflict"
        );
        anyhow::ensure!(
            self.description.is_none() || self.description_prefix.is_none(),
            "selector_description_mode_conflict"
        );
        if let Some(scope) = &self.scope {
            scope.validate()?;
        }
        if let Some(ElementActionTarget::ClickableAncestor { ancestor }) = &self.action_target {
            ancestor.validate()?;
        }
        Ok(())
    }
    pub fn matches(&self, tree: &Tree) -> Vec<usize> {
        tree.nodes
            .iter()
            .enumerate()
            .filter(|(i, n)| {
                n.visible(&self.package)
                    && tree.ancestors_visible(*i)
                    && n.rect().is_some()
                    && self.text.as_ref().is_none_or(|v| n.attr("text") == v)
                    && self
                        .description
                        .as_ref()
                        .is_none_or(|v| n.attr("content-desc") == v)
                    && self
                        .resource_id
                        .as_ref()
                        .is_none_or(|v| n.attr("resource-id") == v)
                    && self
                        .class_name
                        .as_ref()
                        .is_none_or(|v| n.attr("class") == v)
                    && self
                        .text_prefix
                        .as_ref()
                        .is_none_or(|v| n.attr("text").starts_with(v))
                    && self
                        .description_prefix
                        .as_ref()
                        .is_none_or(|v| n.attr("content-desc").starts_with(v))
                    && self
                        .scope
                        .as_ref()
                        .is_none_or(|scope| matching_ancestor(tree, *i, scope).is_some())
            })
            .map(|(i, _)| i)
            .collect()
    }
}

fn matching_ancestor(tree: &Tree, index: usize, constraint: &AncestorConstraint) -> Option<usize> {
    let mut current = tree.nodes.get(index)?.parent;
    for _ in 0..constraint.max_depth {
        let index = current?;
        let node = tree.nodes.get(index)?;
        if constraint.matches(node) {
            return Some(index);
        }
        current = node.parent;
    }
    None
}

fn contains(outer: &crate::ElementBox, inner: &crate::ElementBox) -> bool {
    outer.x <= inner.x
        && outer.y <= inner.y
        && outer.x + outer.width >= inner.x + inner.width
        && outer.y + outer.height >= inner.y + inner.height
}

fn safe_action_ancestor(
    tree: &Tree,
    anchor_index: usize,
    constraint: &AncestorConstraint,
) -> Option<usize> {
    let anchor = tree.nodes.get(anchor_index)?.rect()?;
    let action_index = matching_ancestor(tree, anchor_index, constraint)?;
    let action = tree.nodes.get(action_index)?.rect()?;
    if !action.enabled || !action.clickable || !contains(&action, &anchor) {
        return None;
    }
    let anchor_area = anchor.width * anchor.height;
    let action_area = action.width * action.height;
    if anchor_area <= 0.0 || action_area / anchor_area > 64.0 {
        return None;
    }
    // A large container with another actionable descendant is not a button.
    if tree.nodes.iter().enumerate().any(|(index, node)| {
        index != action_index
            && index != anchor_index
            && tree.inside(index, action_index)
            && node.rect().is_some_and(|rect| rect.clickable)
    }) {
        return None;
    }
    Some(action_index)
}

pub fn resolve_unique_in_tree(
    tree: &Tree,
    selector: &ElementSelector,
) -> anyhow::Result<(usize, crate::ElementBox)> {
    selector.validate()?;
    let matches = selector.matches(tree);
    let [anchor_index] = matches.as_slice() else {
        anyhow::bail!("inspector_match_count: expected 1, found {}", matches.len());
    };
    let action_index = match &selector.action_target {
        Some(ElementActionTarget::ClickableAncestor { ancestor }) => {
            safe_action_ancestor(tree, *anchor_index, ancestor)
                .ok_or_else(|| anyhow::anyhow!("inspector_action_relation_not_satisfied"))?
        }
        None => *anchor_index,
    };
    let rect = tree.nodes[action_index]
        .rect()
        .ok_or_else(|| anyhow::anyhow!("inspector_bounds_missing"))?;
    anyhow::ensure!(rect.enabled, "inspector_element_disabled");
    Ok((action_index, rect))
}

fn dynamic_prefix(value: &str) -> Option<String> {
    let digit = value.char_indices().find(|(_, ch)| ch.is_ascii_digit())?.0;
    let prefix = value[..digit].trim_end();
    let suffix = &value[digit..];
    if prefix.len() < 8
        || !suffix.chars().next()?.is_ascii_digit()
        || !suffix.chars().any(|ch| ch.is_ascii_alphabetic())
    {
        return None;
    }
    Some(prefix.to_owned())
}

fn relation_for_node(tree: &Tree, anchor_index: usize) -> Option<ElementActionTarget> {
    let anchor = tree.nodes.get(anchor_index)?.rect()?;
    if anchor.clickable {
        return None;
    }
    let mut current = tree.nodes.get(anchor_index)?.parent;
    for depth in 1..=MAX_ANCESTOR_DEPTH {
        let index = current?;
        let node = tree.nodes.get(index)?;
        let resource_id =
            (!node.attr("resource-id").is_empty()).then(|| node.attr("resource-id").to_owned());
        let class_name = (!node.attr("class").is_empty()).then(|| node.attr("class").to_owned());
        let constraint = AncestorConstraint {
            max_depth: depth,
            resource_id,
            resource_id_suffix: None,
            class_name,
        };
        if constraint.validate().is_ok()
            && safe_action_ancestor(tree, anchor_index, &constraint) == Some(index)
        {
            return Some(ElementActionTarget::ClickableAncestor {
                ancestor: constraint,
            });
        }
        current = node.parent;
    }
    None
}

/// Generate a stable inspector selector without changing the legacy exact-match contract.
pub fn selector_for_node(tree: &Tree, index: usize, package: &str) -> Option<ElementSelector> {
    let node = tree.nodes.get(index)?;
    let non_empty = |value: &str| (!value.trim().is_empty()).then(|| value.to_owned());
    let description = non_empty(node.attr("content-desc"));
    let text = description
        .is_none()
        .then(|| non_empty(node.attr("text")))
        .flatten();
    let resource_id = (description.is_none() && text.is_none())
        .then(|| non_empty(node.attr("resource-id")))
        .flatten();
    let description_prefix = description.as_deref().and_then(dynamic_prefix);
    let text_prefix = text.as_deref().and_then(dynamic_prefix);
    let uses_prefix = description_prefix.is_some() || text_prefix.is_some();
    let action_target = relation_for_node(tree, index);
    let mut selector = ElementSelector {
        package: package.to_owned(),
        description: (!uses_prefix).then_some(description).flatten(),
        text: (!uses_prefix).then_some(text).flatten(),
        resource_id,
        class_name: non_empty(node.attr("class")),
        schema_version: (uses_prefix || action_target.is_some())
            .then_some(SEMANTIC_SELECTOR_SCHEMA_VERSION),
        text_prefix,
        description_prefix,
        scope: None,
        action_target,
    };
    if selector.validate().is_err() {
        return None;
    }
    if selector.matches(tree).len() != 1 {
        let mut current = node.parent;
        for depth in 1..=MAX_ANCESTOR_DEPTH {
            let ancestor_index = current?;
            let ancestor = tree.nodes.get(ancestor_index)?;
            let resource_id = non_empty(ancestor.attr("resource-id"));
            let class_name = non_empty(ancestor.attr("class"));
            let scope = AncestorConstraint {
                max_depth: depth,
                resource_id,
                resource_id_suffix: None,
                class_name,
            };
            if scope.validate().is_ok() {
                selector.schema_version = Some(SEMANTIC_SELECTOR_SCHEMA_VERSION);
                selector.scope = Some(scope);
                if selector.matches(tree) == vec![index] {
                    break;
                }
                selector.scope = None;
            }
            current = ancestor.parent;
        }
    }
    (selector.matches(tree) == vec![index]).then_some(selector)
}

pub async fn resolve_unique(
    session: &dyn crate::UiSession,
    selector: &ElementSelector,
) -> anyhow::Result<crate::ElementBox> {
    selector.validate()?;
    anyhow::ensure!(
        session.active_app_bundle().await? == selector.package,
        "inspector_wrong_app"
    );
    let tree = Tree::parse(session.hierarchy_source_snapshot().await?)?;
    let (_, rect) = resolve_unique_in_tree(&tree, selector)?;
    anyhow::ensure!(
        session.active_app_bundle().await? == selector.package,
        "inspector_app_changed"
    );
    Ok(rect)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn selector_binds_package_and_unique_semantics_without_coordinates() {
        let tree=Tree::parse(crate::HierarchySourceSnapshot{generation:1,xml:r#"<hierarchy><node package="app" text="Profile" content-desc="" enabled="true" bounds="[10,20][100,80]"/><node package="other" text="Profile" enabled="true" bounds="[100,20][200,80]"/></hierarchy>"#.into()}).unwrap();
        let mut selector = ElementSelector {
            package: "app".into(),
            text: Some("Profile".into()),
            description: None,
            resource_id: None,
            class_name: None,
            schema_version: None,
            text_prefix: None,
            description_prefix: None,
            scope: None,
            action_target: None,
        };
        assert_eq!(selector.matches(&tree), vec![1]);
        selector.text = None;
        assert!(selector.validate().is_err());
    }

    #[test]
    fn semantic_selector_removes_dynamic_count_but_keeps_meaning_and_uniqueness() {
        let tree = Tree::parse(crate::HierarchySourceSnapshot {
            generation: 1,
            xml: r#"<hierarchy>
              <node package="app" class="android.widget.Button" resource-id="app:id/fpv" content-desc="Like video. 38 likes" clickable="true" enabled="true" visible-to-user="true" bounds="[900,1000][1080,1150]"/>
              <node package="app" class="android.widget.Button" resource-id="app:id/fpv" content-desc="Share video. 1 shares" clickable="true" enabled="true" visible-to-user="true" bounds="[900,1300][1080,1450]"/>
            </hierarchy>"#.into(),
        })
        .unwrap();
        let selector = selector_for_node(&tree, 1, "app").unwrap();
        assert_eq!(selector.schema_version, Some(2));
        assert_eq!(selector.description_prefix.as_deref(), Some("Like video."));
        assert_eq!(selector.resource_id, None);
        assert_eq!(selector.matches(&tree), vec![1]);
    }

    #[test]
    fn labelled_favorites_control_resolves_only_its_measured_clickable_parent() {
        let tree = Tree::parse(crate::HierarchySourceSnapshot {
            generation: 1,
            xml: r#"<hierarchy>
              <node package="app" class="android.widget.FrameLayout" resource-id="app:id/hlx" clickable="true" enabled="true" visible-to-user="true" bounds="[912,1487][1080,1645]">
                <node package="app" class="android.widget.Button" resource-id="app:id/hly" content-desc="Add or remove this video from Favorites." clickable="false" enabled="true" visible-to-user="true" bounds="[912,1487][1080,1645]"/>
              </node>
            </hierarchy>"#.into(),
        })
        .unwrap();
        let selector = selector_for_node(&tree, 2, "app").unwrap();
        let (action_index, action) = resolve_unique_in_tree(&tree, &selector).unwrap();
        assert_eq!(action_index, 1);
        assert!(action.clickable);
        assert_eq!((action.x, action.y), (912.0, 1487.0));
    }

    #[test]
    fn unlabelled_pager_dot_never_climbs_to_a_large_clickable_feed_container() {
        let tree = Tree::parse(crate::HierarchySourceSnapshot {
            generation: 1,
            xml: r#"<hierarchy>
              <node package="app" class="android.widget.FrameLayout" resource-id="app:id/feed" clickable="true" enabled="true" visible-to-user="true" bounds="[0,0][1080,1965]">
                <node package="app" class="android.widget.LinearLayout" resource-id="app:id/dots" clickable="false" enabled="true" visible-to-user="true" bounds="[436,1376][644,1392]">
                  <node package="app" class="android.widget.ImageView" clickable="false" enabled="true" visible-to-user="true" bounds="[532,1376][548,1392]"/>
                </node>
              </node>
            </hierarchy>"#.into(),
        })
        .unwrap();
        assert!(selector_for_node(&tree, 3, "app").is_none());
    }
}
