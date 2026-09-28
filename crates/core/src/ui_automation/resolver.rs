use super::{model::*, profile::TargetSpec, tree::Tree};

/// An unresolved predicate is not a mismatch: absence needs exhaustive known results.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SemanticMatch {
    Match,
    NoMatch,
    Unknown,
}

/// Compiles the app-independent locator in the shared resolver, separate from action targets.
pub struct CompiledSemanticLocator<'a> {
    locator: &'a super::SemanticLocator,
}

pub fn compile_locator(locator: &super::SemanticLocator) -> CompiledSemanticLocator<'_> {
    CompiledSemanticLocator { locator }
}

impl CompiledSemanticLocator<'_> {
    pub fn matches(&self, node: &super::SemanticNode) -> SemanticMatch {
        let mut unknown = false;
        for (expected, actual) in [
            (&self.locator.role, &node.role),
            (&self.locator.name, &node.name),
            (&self.locator.text, &node.text),
            (&self.locator.id, &node.id),
        ] {
            let Some(expected) = expected else {
                continue;
            };
            let Some(actual) = actual else {
                unknown = true;
                continue;
            };
            let matches = if self.locator.exact {
                actual == expected
            } else {
                actual.contains(expected)
            };
            if !matches {
                return SemanticMatch::NoMatch;
            }
        }
        if unknown {
            SemanticMatch::Unknown
        } else {
            SemanticMatch::Match
        }
    }
}

/// Only native classes with unambiguous semantics receive inferred roles.
fn semantic_role(node: &super::tree::Node) -> Option<String> {
    if let Some(role) = node.attribute("role") {
        return Some(role.to_owned());
    }
    let role = match node.attr("class") {
        "android.widget.Button" | "android.widget.ImageButton" => "button",
        "android.widget.EditText" => "textbox",
        "android.widget.CheckBox" => "checkbox",
        "android.widget.RadioButton" => "radio",
        "android.widget.Switch" => "switch",
        "android.widget.ImageView" => "image",
        "android.widget.TextView" => "text",
        "android.widget.SeekBar" => "slider",
        _ => return None,
    };
    Some(role.to_owned())
}

/// Lossless semantic projection: no bounds, visibility, enabled or clickability filter.
pub fn semantic_nodes(tree: &Tree) -> Vec<super::SemanticNode> {
    tree.nodes
        .iter()
        .enumerate()
        .filter_map(|(node_id, node)| {
            // XML hierarchy wrappers carry rotation but are not accessibility elements.
            if ![
                "class",
                "role",
                "text",
                "content-desc",
                "resource-id",
                "package",
            ]
            .iter()
            .any(|key| node.attribute(key).is_some())
            {
                return None;
            }
            let value = |key| node.attribute(key).map(str::to_owned);
            let boolean = |key| match node.attribute(key) {
                Some("true") => Some(true),
                Some("false") => Some(false),
                _ => None,
            };
            let mut package = value("package");
            let mut parent = node.parent;
            let mut visible = node.visibility();
            while let Some(index) = parent {
                let ancestor = &tree.nodes[index];
                if package.is_none() {
                    package = ancestor.attribute("package").map(str::to_owned);
                }
                if ancestor.visibility() == Some(false) {
                    visible = Some(false);
                }
                parent = ancestor.parent;
            }
        let mut semantic = super::SemanticNode {
                node_id,
                parent: node.parent,
                id: value("resource-id"),
                class_name: value("class"),
                package,
                role: semantic_role(node),
                name: value("content-desc"),
            text: value("text"),
            value: value("value"),
            password: boolean("password"),
            showing_hint: boolean("showing-hint").or_else(|| boolean("showing-hint-text")),
            checkable: boolean("checkable"),
            scrollable: boolean("scrollable"),
            long_clickable: boolean("long-clickable"),
            focusable: boolean("focusable"),
                enabled: boolean("enabled"),
                clickable: boolean("clickable"),
                visible,
                focused: boolean("focused"),
                selected: boolean("selected"),
                checked: boolean("checked"),
                bounds: node.rect().as_ref().map(Rect::from),
                raw_attributes: Some(
                    node.attributes()
                        .iter()
                        .map(|(key, value)| (key.clone(), value.clone()))
                        .collect(),
                ),
            })
        };
        semantic.redact_sensitive();
        Some(semantic)
        .collect()
}

#[derive(Debug, Clone)]
pub struct SemanticResolution {
    pub matches: Vec<super::SemanticNode>,
    pub unknown_match_count: usize,
}

/// Resolve predicates and a unique inclusive subtree before applying the output field mask.
/// A missing/ambiguous/unknown scope root is an error, never an empty successful observation.
pub fn resolve_observation(
    tree: &Tree,
    request: &super::ObservationRequest,
) -> anyhow::Result<SemanticResolution> {
    request.validate()?;
    let nodes = semantic_nodes(tree);
    let scope = request.scope.as_ref();
    let package_match = |node: &super::SemanticNode| match scope.and_then(|s| s.package.as_ref()) {
        None => SemanticMatch::Match,
        Some(package) => match node.package.as_ref() {
            Some(actual) if actual == package => SemanticMatch::Match,
            Some(_) => SemanticMatch::NoMatch,
            None => SemanticMatch::Unknown,
        },
    };
    let root = if let Some(root) = scope.and_then(|s| s.root.as_ref()) {
        let compiled = compile_locator(root);
        let mut roots = Vec::new();
        let mut unknown = false;
        for node in &nodes {
            let package = package_match(node);
            let matched = compiled.matches(node);
            if package == SemanticMatch::NoMatch || matched == SemanticMatch::NoMatch {
                continue;
            }
            if package == SemanticMatch::Unknown || matched == SemanticMatch::Unknown {
                unknown = true;
            } else {
                roots.push(node.node_id);
            }
        }
        anyhow::ensure!(!unknown && roots.len() == 1, "observation_scope_not_unique");
        roots.first().copied()
    } else {
        None
    };
    let compiled = compile_locator(&request.query);
    let mut result = SemanticResolution {
        matches: Vec::new(),
        unknown_match_count: 0,
    };
    for mut node in nodes {
        if root.is_some_and(|root| !tree.inside(node.node_id, root)) {
            continue;
        }
        let package = package_match(&node);
        let matched = compiled.matches(&node);
        if package == SemanticMatch::NoMatch || matched == SemanticMatch::NoMatch {
            continue;
        }
        if package == SemanticMatch::Unknown || matched == SemanticMatch::Unknown {
            result.unknown_match_count += 1;
        } else {
            node.project(&request.fields);
            result.matches.push(node);
        }
    }
    Ok(result)
}

fn normalized(s: &str) -> String {
    s.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

pub fn visible_nodes(tree: &Tree, app: &AppContext) -> Vec<GuiNode> {
    tree.nodes
        .iter()
        .enumerate()
        .filter_map(|(id, node)| {
            if !node.visible(&app.package) || !tree.ancestors_visible(id) {
                return None;
            }
            let b = node.rect()?;
            let bounds = Rect::from(&b);
            if !bounds.valid(app.width, app.height) {
                return None;
            }
            let boolean = |name| match node.attr(name) {
                "true" => Some(true),
                "false" => Some(false),
                _ => None,
            };
            Some(GuiNode {
                id,
                parent: node.parent,
                text: node.attr("text").into(),
                description: node.attr("content-desc").into(),
                resource_id: node.attr("resource-id").into(),
                class_name: node.attr("class").into(),
                bounds,
                enabled: boolean("enabled"),
                clickable: boolean("clickable"),
                visible: node.visibility(),
            })
        })
        .collect()
}

/// Find the smallest actionable ancestor. A container holding competing actions is not a button.
pub fn actionable_node<'a>(tree: &Tree, nodes: &'a [GuiNode], id: usize) -> Option<&'a GuiNode> {
    let mut current = Some(id);
    for _ in 0..4 {
        let index = current?;
        let node = nodes.iter().find(|n| n.id == index)?;
        if node.enabled != Some(true) {
            return None;
        }
        if node.clickable == Some(true) {
            let actions = nodes
                .iter()
                .filter(|n| n.id != index && tree.inside(n.id, index) && n.clickable == Some(true))
                .count();
            return (actions == 0).then_some(node);
        }
        current = node.parent;
    }
    None
}

pub fn resolve(tree: &Tree, app: &AppContext, spec: &TargetSpec) -> Vec<TargetCandidate> {
    let nodes = visible_nodes(tree, app);
    let matches = |node: &GuiNode, labels: &[String]| {
        labels.iter().any(|s| {
            normalized(&node.text) == normalized(s)
                || normalized(&node.description) == normalized(s)
        })
    };
    if spec
        .anchors
        .iter()
        .any(|a| !nodes.iter().any(|n| matches(n, std::slice::from_ref(a))))
    {
        return Vec::new();
    }
    if nodes.iter().any(|n| matches(n, &spec.excluded_labels)) {
        return Vec::new();
    }
    let mut candidates = Vec::new();
    for node in &nodes {
        let semantic = matches(node, &spec.labels);
        let resource = spec
            .resource_suffixes
            .iter()
            .any(|id| node.resource_id.ends_with(id));
        if !semantic && !resource {
            continue;
        }
        let Some(button) = actionable_node(tree, &nodes, node.id) else {
            continue;
        };
        if candidates
            .iter()
            .any(|c: &TargetCandidate| c.node_id == Some(button.id))
        {
            continue;
        }
        candidates.push(TargetCandidate {
            node_id: Some(button.id),
            bounds: button.bounds.clone(),
            method: if semantic { "semantic" } else { "resource" }.into(),
            evidence: vec![format!("target:{}", spec.id), format!("anchor:{}", node.id)],
        });
    }
    candidates
}
