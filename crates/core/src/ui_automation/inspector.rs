//! Stable element identity shared by the inspector, agent bridge and recorded Flow.
use super::tree::Tree;
use serde::{Deserialize, Serialize};

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
}
impl ElementSelector {
    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            !self.package.trim().is_empty() && self.package.len() <= 256,
            "selector_package_invalid"
        );
        anyhow::ensure!(
            [&self.text, &self.description, &self.resource_id]
                .iter()
                .any(|v| v.as_ref().is_some_and(|s| !s.trim().is_empty())),
            "selector_identity_missing"
        );
        for value in [
            &self.text,
            &self.description,
            &self.resource_id,
            &self.class_name,
        ]
        .into_iter()
        .flatten()
        {
            anyhow::ensure!(
                !value.trim().is_empty() && value.len() <= 2048,
                "selector_value_invalid"
            );
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
            })
            .map(|(i, _)| i)
            .collect()
    }
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
    let matches = selector.matches(&tree);
    let [index] = matches.as_slice() else {
        anyhow::bail!("inspector_match_count: expected 1, found {}", matches.len());
    };
    let rect = tree.nodes[*index]
        .rect()
        .ok_or_else(|| anyhow::anyhow!("inspector_bounds_missing"))?;
    anyhow::ensure!(rect.enabled, "inspector_element_disabled");
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
        };
        assert_eq!(selector.matches(&tree), vec![1]);
        selector.text = None;
        assert!(selector.validate().is_err());
    }
}
