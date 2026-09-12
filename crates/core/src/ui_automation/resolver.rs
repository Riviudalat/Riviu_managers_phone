use super::{model::*, profile::TargetSpec, tree::Tree};

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
