//! Bounded compile-time composition. Source snapshots remain unchanged and execution sees a DAG.
use std::collections::{BTreeMap, BTreeSet};

use riviu_core::{
    ActionKind, CompositionFrame, CompositionSource, FlowDocumentV2, FlowEdge, FlowNode, NodeId,
};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::flow::FlowCompileError;

pub const MAX_COMPOSITION_DEPTH: usize = 8;
pub const MAX_EXPANDED_NODES: usize = 2_000;
pub const MAX_COMPOSITION_REPEAT: u32 = 50;

#[derive(Debug)]
pub struct ExpandedComposition {
    pub document: FlowDocumentV2,
    pub source_paths: BTreeMap<NodeId, CompositionSource>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SubflowConfig {
    document: FlowDocumentV2,
    #[serde(default)]
    inputs: BTreeMap<String, String>,
    #[serde(default)]
    outputs: BTreeMap<String, String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RepeatConfig {
    document: FlowDocumentV2,
    count: u32,
    #[serde(default)]
    inputs: BTreeMap<String, String>,
    #[serde(default)]
    outputs: BTreeMap<String, String>,
}

fn error(code: &str, message: impl Into<String>, node_id: NodeId) -> Vec<FlowCompileError> {
    vec![FlowCompileError::node(
        code,
        message,
        node_id,
        Some("config.document"),
    )]
}

fn scoped_uuid(scope: &str, category: &str, value: impl std::fmt::Display) -> Uuid {
    let digest =
        Sha256::digest(format!("riviu-flow-composition/v1/{scope}/{category}/{value}").as_bytes());
    let mut bytes = [0_u8; 16];
    bytes.copy_from_slice(&digest[..16]);
    bytes[6] = (bytes[6] & 0x0f) | 0x80;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    Uuid::from_bytes(bytes)
}

fn variable(scope: &str, name: &str) -> String {
    if scope.is_empty() {
        name.to_owned()
    } else {
        format!("v_{}", scoped_uuid(scope, "variable", name).simple())
    }
}

fn valid_variable(name: &str) -> bool {
    let mut chars = name.bytes();
    !name.is_empty()
        && name.len() <= 64
        && chars
            .next()
            .is_some_and(|b| b.is_ascii_alphabetic() || b == b'_')
        && chars.all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

fn scope_interpolation(value: &mut serde_json::Value, scope: &str) {
    match value {
        serde_json::Value::String(text) => {
            let mut output = String::with_capacity(text.len());
            let mut rest = text.as_str();
            while let Some(begin) = rest.find("${") {
                output.push_str(&rest[..begin]);
                let tail = &rest[begin + 2..];
                let Some(end) = tail.find('}') else {
                    output.push_str(&rest[begin..]);
                    rest = "";
                    break;
                };
                let name = &tail[..end];
                output.push_str("${");
                if valid_variable(name) {
                    output.push_str(&variable(scope, name));
                } else {
                    output.push_str(name);
                }
                output.push('}');
                rest = &tail[end + 1..];
            }
            output.push_str(rest);
            *text = output;
        }
        serde_json::Value::Array(values) => {
            for item in values {
                scope_interpolation(item, scope);
            }
        }
        serde_json::Value::Object(values) => {
            for item in values.values_mut() {
                scope_interpolation(item, scope);
            }
        }
        _ => {}
    }
}

#[derive(Default)]
struct Builder {
    nodes: Vec<FlowNode>,
    edges: Vec<FlowEdge>,
    used_ids: BTreeSet<Uuid>,
    source_paths: BTreeMap<NodeId, CompositionSource>,
}

impl Builder {
    fn push_node(
        &mut self,
        node: FlowNode,
        source: CompositionSource,
    ) -> Result<(), Vec<FlowCompileError>> {
        if self.nodes.len() >= MAX_EXPANDED_NODES {
            return Err(error(
                "CompositionNodeLimit",
                "Expanded Flow exceeds 2000 nodes",
                source.source_node_id,
            ));
        }
        if !self.used_ids.insert(node.id) {
            return Err(error(
                "CompositionIdCollision",
                "Expanded node IDs collide",
                source.source_node_id,
            ));
        }
        if !source.path.is_empty() {
            self.source_paths.insert(node.id, source);
        }
        self.nodes.push(node);
        Ok(())
    }

    fn edge(
        &mut self,
        scope: &str,
        identity: impl std::fmt::Display,
        from: NodeId,
        source_port: &str,
        to: NodeId,
        target_port: &str,
    ) {
        self.edges.push(FlowEdge {
            id: scoped_uuid(scope, "edge", identity),
            source_node_id: from,
            source_port: source_port.to_owned(),
            target_node_id: to,
            target_port: target_port.to_owned(),
        });
    }

    fn expand(
        &mut self,
        doc: &FlowDocumentV2,
        scope: &str,
        path: &[CompositionFrame],
    ) -> Result<(NodeId, NodeId), Vec<FlowCompileError>> {
        if path.len() > MAX_COMPOSITION_DEPTH {
            return Err(error(
                "CompositionDepthLimit",
                "Flow nesting exceeds 8 levels",
                doc.entry_node_id,
            ));
        }
        let nested = !path.is_empty();
        let ids: BTreeSet<_> = doc.nodes.iter().map(|node| node.id).collect();
        if ids.len() != doc.nodes.len() {
            return Err(error(
                "DuplicateNodeId",
                "Composition snapshot has duplicate node IDs",
                doc.entry_node_id,
            ));
        }
        if doc.schema_version != 2 {
            return Err(error(
                "SchemaVersionUnsupported",
                "Composition snapshot must use Flow V2",
                doc.entry_node_id,
            ));
        }
        if !doc.viewport.x.is_finite()
            || !doc.viewport.y.is_finite()
            || !doc.viewport.zoom.is_finite()
            || doc
                .nodes
                .iter()
                .any(|n| !n.position.x.is_finite() || !n.position.y.is_finite())
        {
            return Err(error(
                "NonFiniteCoordinate",
                "Composition snapshot contains non-finite layout",
                doc.entry_node_id,
            ));
        }
        let starts: Vec<_> = doc
            .nodes
            .iter()
            .filter(|node| node.kind == ActionKind::Start)
            .collect();
        let ends: Vec<_> = doc
            .nodes
            .iter()
            .filter(|node| node.kind == ActionKind::End)
            .collect();
        if starts.len() != 1 || ends.len() != 1 || starts[0].id != doc.entry_node_id {
            return Err(error(
                "CompositionBoundaryInvalid",
                "Snapshot requires one matching Start and one End",
                doc.entry_node_id,
            ));
        }
        if starts.iter().chain(ends.iter()).any(|n| {
            n.postcondition.is_some() || !n.config.as_object().is_some_and(|cfg| cfg.is_empty())
        }) {
            return Err(error(
                "CompositionBoundaryInvalid",
                "Start and End require empty config and no postcondition",
                doc.entry_node_id,
            ));
        }
        if doc
            .edges
            .iter()
            .any(|e| e.target_node_id == starts[0].id || e.source_node_id == ends[0].id)
            || doc
                .edges
                .iter()
                .filter(|e| e.source_node_id == starts[0].id)
                .count()
                != 1
            || !doc.edges.iter().any(|e| e.target_node_id == ends[0].id)
        {
            return Err(error(
                "CompositionBoundaryInvalid",
                "Start must have no input and one output; End must have inputs and no output",
                doc.entry_node_id,
            ));
        }
        let mut boundaries = BTreeMap::new();
        for source in &doc.nodes {
            let source_meta = CompositionSource {
                source_node_id: source.id,
                path: path.to_vec(),
            };
            if matches!(source.kind, ActionKind::Subflow | ActionKind::Repeat) {
                if doc
                    .edges
                    .iter()
                    .filter(|e| e.source_node_id == source.id)
                    .count()
                    != 1
                    || doc
                        .edges
                        .iter()
                        .filter(|e| e.target_node_id == source.id)
                        .count()
                        != 1
                    || doc.edges.iter().any(|e| {
                        (e.source_node_id == source.id && e.source_port != "flow")
                            || (e.target_node_id == source.id && e.target_port != "flow")
                    })
                {
                    return Err(error(
                        "CompositionBoundaryInvalid",
                        "Composition node requires one input and one output through flow ports",
                        source.id,
                    ));
                }
                if source.postcondition.is_some() {
                    return Err(error(
                        "CompositionEvidenceUnsupported",
                        "Composition has no direct side effect; configure evidence on body actions",
                        source.id,
                    ));
                }
                let (body, count, inputs, outputs) = if source.kind == ActionKind::Subflow {
                    let cfg: SubflowConfig = serde_json::from_value(source.config.clone())
                        .map_err(|e| error("CompositionConfigInvalid", e.to_string(), source.id))?;
                    (cfg.document, 1, cfg.inputs, cfg.outputs)
                } else {
                    let cfg: RepeatConfig = serde_json::from_value(source.config.clone())
                        .map_err(|e| error("CompositionConfigInvalid", e.to_string(), source.id))?;
                    if cfg.count == 0 || cfg.count > MAX_COMPOSITION_REPEAT {
                        return Err(error(
                            "CompositionRepeatLimit",
                            "Repeat count must be from 1 to 50",
                            source.id,
                        ));
                    }
                    (cfg.document, cfg.count, cfg.inputs, cfg.outputs)
                };
                for (left, right) in inputs.iter().chain(outputs.iter()) {
                    if !valid_variable(left) || !valid_variable(right) {
                        return Err(error(
                            "CompositionBindingInvalid",
                            "Input/output bindings require valid variable names",
                            source.id,
                        ));
                    }
                }
                let mut first = None;
                let mut last = None;
                for iteration in 0..count {
                    let invocation = format!("{scope}/{}/{iteration}", source.id);
                    let mut child_path = path.to_vec();
                    child_path.push(CompositionFrame {
                        node_id: source.id,
                        flow_id: body.id,
                        revision: body.revision,
                        iteration: (source.kind == ActionKind::Repeat).then_some(iteration + 1),
                    });
                    let (entry, exit) = self.expand(&body, &invocation, &child_path)?;
                    // Start is an internal Join with one successor. Insert bindings before its body.
                    let child_start_edges: Vec<_> = self
                        .edges
                        .iter()
                        .enumerate()
                        .filter(|(_, e)| e.source_node_id == entry)
                        .map(|(i, _)| i)
                        .collect();
                    let mut input_tail = entry;
                    for (index, (child, parent)) in inputs.iter().enumerate() {
                        let id = scoped_uuid(&invocation, "input", index);
                        self.push_node(FlowNode { id, kind: ActionKind::CopyVariable, position: source.position, config: serde_json::json!({"name":variable(&invocation,child),"source":variable(scope,parent)}), postcondition: None }, CompositionSource { source_node_id: source.id, path: child_path.clone() })?;
                        self.edge(
                            &invocation,
                            format!("input/{index}"),
                            input_tail,
                            "flow",
                            id,
                            "flow",
                        );
                        input_tail = id;
                    }
                    if input_tail != entry {
                        for index in child_start_edges {
                            self.edges[index].source_node_id = input_tail;
                        }
                    }
                    let mut output_tail = exit;
                    for (index, (parent, child)) in outputs.iter().enumerate() {
                        let id = scoped_uuid(&invocation, "output", index);
                        self.push_node(FlowNode { id, kind: ActionKind::CopyVariable, position: source.position, config: serde_json::json!({"name":variable(scope,parent),"source":variable(&invocation,child)}), postcondition: None }, CompositionSource { source_node_id: source.id, path: child_path.clone() })?;
                        self.edge(
                            &invocation,
                            format!("output/{index}"),
                            output_tail,
                            "flow",
                            id,
                            "flow",
                        );
                        output_tail = id;
                    }
                    if first.is_none() {
                        first = Some(entry);
                    }
                    if let Some(previous) = last {
                        self.edge(
                            scope,
                            format!("repeat/{}/{iteration}", source.id),
                            previous,
                            "flow",
                            entry,
                            "flow",
                        );
                    }
                    last = Some(output_tail);
                }
                boundaries.insert(
                    source.id,
                    (
                        first.expect("bounded count is positive"),
                        last.expect("bounded count is positive"),
                    ),
                );
            } else {
                let mut copied = source.clone();
                if nested {
                    copied.id = scoped_uuid(scope, "node", source.id);
                    if matches!(source.kind, ActionKind::Start | ActionKind::End) {
                        copied.kind = ActionKind::Join;
                    }
                    for field in match copied.kind {
                        ActionKind::SetVariable
                        | ActionKind::ReadText
                        | ActionKind::IfValue
                        | ActionKind::OcrReadText
                        | ActionKind::FileRead
                        | ActionKind::FileWrite
                        | ActionKind::HttpRequest
                        | ActionKind::SheetRead
                        | ActionKind::SheetWrite => &["name"][..],
                        ActionKind::CopyVariable | ActionKind::Transform => &["name", "source"][..],
                        ActionKind::Log => &["variable"][..],
                        _ => &[][..],
                    } {
                        if let Some(name) = copied
                            .config
                            .get(*field)
                            .and_then(|v| v.as_str())
                            .filter(|v| !v.is_empty())
                        {
                            if !valid_variable(name) {
                                return Err(error(
                                    "CompositionBindingInvalid",
                                    "Body variable name is invalid before scoping",
                                    source.id,
                                ));
                            }
                            copied.config[*field] = variable(scope, name).into();
                        }
                    }
                    for field in match copied.kind {
                        ActionKind::FileRead => &["path"][..],
                        ActionKind::FileWrite => &["path", "value"][..],
                        ActionKind::HttpRequest => &["url", "body"][..],
                        ActionKind::SheetRead => &["spreadsheetUrl", "tab", "range"][..],
                        ActionKind::SheetWrite => &["spreadsheetUrl", "tab", "range", "values"][..],
                        _ => &[][..],
                    } {
                        if let Some(value) = copied.config.get_mut(*field) {
                            scope_interpolation(value, scope);
                        }
                    }
                    if let Some(riviu_core::EvidenceSpec::ConnectorResult { name }) =
                        &mut copied.postcondition
                    {
                        *name = variable(scope, name);
                    }
                }
                boundaries.insert(source.id, (copied.id, copied.id));
                self.push_node(copied, source_meta)?;
            }
        }
        let mut edge_ids = BTreeSet::new();
        for edge in &doc.edges {
            if !edge_ids.insert(edge.id) {
                return Err(error(
                    "DuplicateEdgeId",
                    "Snapshot has duplicate edge IDs",
                    doc.entry_node_id,
                ));
            }
            let from = boundaries.get(&edge.source_node_id).ok_or_else(|| {
                error(
                    "MissingEdgeSource",
                    "Snapshot edge source is missing",
                    edge.source_node_id,
                )
            })?;
            let to = boundaries.get(&edge.target_node_id).ok_or_else(|| {
                error(
                    "MissingEdgeTarget",
                    "Snapshot edge target is missing",
                    edge.target_node_id,
                )
            })?;
            // Preserve original ports, including matched/notMatched edges and branch joins.
            if nested {
                self.edge(
                    scope,
                    edge.id,
                    from.1,
                    &edge.source_port,
                    to.0,
                    &edge.target_port,
                );
            } else {
                self.edges.push(FlowEdge {
                    source_node_id: from.1,
                    target_node_id: to.0,
                    ..edge.clone()
                });
            }
        }
        Ok((boundaries[&doc.entry_node_id].0, boundaries[&ends[0].id].1))
    }
}

pub fn expand_composition(
    document: &FlowDocumentV2,
) -> Result<ExpandedComposition, Vec<FlowCompileError>> {
    // Preserve legacy documents exactly, including historical canonical plan hashes.
    if !document
        .nodes
        .iter()
        .any(|node| matches!(node.kind, ActionKind::Subflow | ActionKind::Repeat))
    {
        return Ok(ExpandedComposition {
            document: document.clone(),
            source_paths: BTreeMap::new(),
        });
    }
    let mut builder = Builder::default();
    let (entry, _) = builder.expand(document, "", &[])?;
    let document = FlowDocumentV2 {
        entry_node_id: entry,
        nodes: builder.nodes,
        edges: builder.edges,
        ..document.clone()
    };
    Ok(ExpandedComposition {
        document,
        source_paths: builder.source_paths,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use riviu_core::{CanvasPoint, EvidenceSpec};
    use serde_json::json;

    fn linear(kinds: Vec<(ActionKind, serde_json::Value)>) -> FlowDocumentV2 {
        let mut doc = FlowDocumentV2::empty("body");
        let end = doc.nodes.pop().unwrap();
        for (kind, config) in kinds {
            doc.nodes.push(FlowNode::new(kind, config));
        }
        doc.nodes.push(end);
        doc.edges = doc
            .nodes
            .windows(2)
            .map(|pair| FlowEdge::flow(pair[0].id, pair[1].id))
            .collect();
        doc
    }
    fn wrapped(body: &FlowDocumentV2, count: u32) -> FlowDocumentV2 {
        linear(vec![(
            ActionKind::Repeat,
            json!({"document":body,"count":count}),
        )])
    }
    fn code(result: Result<ExpandedComposition, Vec<FlowCompileError>>) -> String {
        result.unwrap_err()[0].code.clone()
    }

    #[test]
    fn plain_flow_is_returned_byte_equivalent_with_no_metadata() {
        let doc = linear(vec![(ActionKind::Wait, json!({"durationMs":100}))]);
        let expanded = expand_composition(&doc).unwrap();
        assert_eq!(expanded.document, doc);
        assert!(expanded.source_paths.is_empty());
    }

    #[test]
    fn expansion_is_deterministic_scoped_and_preserves_evidence() {
        let mut body = linear(vec![
            (
                ActionKind::SetVariable,
                json!({"name":"counter","value":"1"}),
            ),
            (
                ActionKind::Log,
                json!({"message":"value","variable":"counter"}),
            ),
            (ActionKind::LaunchApp, json!({"bundleId":"com.example.app"})),
        ]);
        body.revision = 7;
        body.nodes[3].postcondition = Some(EvidenceSpec::ActiveAppEquals {
            bundle_id: "com.example.app".into(),
        });
        let doc = wrapped(&body, 2);
        let a = expand_composition(&doc).unwrap();
        let b = expand_composition(&doc).unwrap();
        assert_eq!(a.document, b.document);
        assert_eq!(a.source_paths, b.source_paths);
        assert_eq!(a.document.nodes.len(), 12);
        assert_eq!(
            a.document
                .nodes
                .iter()
                .map(|n| n.id)
                .collect::<BTreeSet<_>>()
                .len(),
            12
        );
        assert_eq!(
            a.document
                .nodes
                .iter()
                .filter(|n| n.kind == ActionKind::Start)
                .count(),
            1
        );
        assert_eq!(
            a.document
                .nodes
                .iter()
                .filter(|n| n.kind == ActionKind::End)
                .count(),
            1
        );
        let assignments: Vec<_> = a
            .document
            .nodes
            .iter()
            .filter(|n| n.kind == ActionKind::SetVariable)
            .collect();
        assert_ne!(assignments[0].config["name"], assignments[1].config["name"]);
        for node in a
            .document
            .nodes
            .iter()
            .filter(|n| n.kind == ActionKind::LaunchApp)
        {
            assert_eq!(node.postcondition, body.nodes[3].postcondition);
        }
        assert!(a
            .source_paths
            .values()
            .any(|p| p.path[0].iteration == Some(2) && p.path[0].revision == 7));
        assert_eq!(body.nodes[1].config["name"], "counter");
    }

    #[test]
    fn injects_input_before_body_and_output_after_child_end_each_iteration() {
        let body = linear(vec![(
            ActionKind::CopyVariable,
            json!({"name":"out","source":"in"}),
        )]);
        let doc = linear(vec![(
            ActionKind::Repeat,
            json!({"document":body,"count":2,"inputs":{"in":"parent"},"outputs":{"parent":"out"}}),
        )]);
        let expanded = expand_composition(&doc).unwrap();
        let copies: Vec<_> = expanded
            .document
            .nodes
            .iter()
            .filter(|n| n.kind == ActionKind::CopyVariable)
            .collect();
        assert_eq!(copies.len(), 6);
        let inputs: Vec<_> = copies
            .iter()
            .filter(|n| n.config["source"] == "parent")
            .collect();
        let outputs: Vec<_> = copies
            .iter()
            .filter(|n| n.config["name"] == "parent")
            .collect();
        assert_eq!(inputs.len(), 2);
        assert_eq!(outputs.len(), 2);
        for input in inputs {
            let outgoing = expanded
                .document
                .edges
                .iter()
                .find(|e| e.source_node_id == input.id)
                .unwrap();
            let body_node = expanded
                .document
                .nodes
                .iter()
                .find(|n| n.id == outgoing.target_node_id)
                .unwrap();
            assert_eq!(body_node.config["source"], input.config["name"]);
        }
        assert!(expanded
            .document
            .edges
            .iter()
            .any(|e| e.source_node_id == outputs[0].id));
    }

    #[test]
    fn nested_branches_preserve_ports_and_merge_into_join() {
        let mut body = linear(vec![
            (
                ActionKind::IfValue,
                json!({"name":"x","operator":"equals","value":"y"}),
            ),
            (ActionKind::Log, json!({"message":"yes"})),
            (ActionKind::Log, json!({"message":"no"})),
        ]);
        let [s, p, a, b, e] = body
            .nodes
            .iter()
            .map(|n| n.id)
            .collect::<Vec<_>>()
            .try_into()
            .unwrap();
        body.edges = vec![
            FlowEdge::flow(s, p),
            FlowEdge {
                source_port: "matched".into(),
                ..FlowEdge::flow(p, a)
            },
            FlowEdge {
                source_port: "notMatched".into(),
                ..FlowEdge::flow(p, b)
            },
            FlowEdge::flow(a, e),
            FlowEdge::flow(b, e),
        ];
        let nested = linear(vec![(ActionKind::Subflow, json!({"document":body}))]);
        let expanded = expand_composition(&wrapped(&nested, 2)).unwrap();
        assert_eq!(
            expanded
                .document
                .edges
                .iter()
                .filter(|edge| edge.source_port == "matched")
                .count(),
            2
        );
        assert_eq!(
            expanded
                .document
                .edges
                .iter()
                .filter(|edge| edge.source_port == "notMatched")
                .count(),
            2
        );
        assert!(expanded
            .document
            .nodes
            .iter()
            .filter(|node| node.kind == ActionKind::Join)
            .any(|node| expanded
                .document
                .edges
                .iter()
                .filter(|edge| edge.target_node_id == node.id)
                .count()
                == 2));
        assert!(expanded.source_paths.values().any(|m| m.path.len() == 2));
    }

    #[test]
    fn rejects_unbounded_counts_unknown_fields_bad_bindings_and_hidden_boundary_errors() {
        let body = FlowDocumentV2::empty("empty");
        assert_eq!(
            code(expand_composition(&wrapped(&body, 0))),
            "CompositionRepeatLimit"
        );
        assert_eq!(
            code(expand_composition(&wrapped(&body, 51))),
            "CompositionRepeatLimit"
        );
        let unknown = linear(vec![(
            ActionKind::Subflow,
            json!({"document":body,"unknownEffect":true}),
        )]);
        assert_eq!(
            code(expand_composition(&unknown)),
            "CompositionConfigInvalid"
        );
        let binding = linear(vec![(
            ActionKind::Subflow,
            json!({"document":body,"inputs":{"bad-name":"good"}}),
        )]);
        assert_eq!(
            code(expand_composition(&binding)),
            "CompositionBindingInvalid"
        );
        let mut invalid = body.clone();
        invalid
            .edges
            .push(FlowEdge::flow(invalid.nodes[1].id, invalid.nodes[0].id));
        assert_eq!(
            code(expand_composition(&wrapped(&invalid, 1))),
            "CompositionBoundaryInvalid"
        );
        let mut layout = body;
        layout.nodes[0].position = CanvasPoint {
            x: f64::NAN,
            y: 0.0,
        };
        // JSON serialization normalizes NaN to null; direct shape is rejected while parsing config.
        assert_eq!(
            code(expand_composition(&wrapped(&layout, 1))),
            "CompositionConfigInvalid"
        );
    }

    #[test]
    fn enforces_depth_and_expanded_node_budgets_before_unbounded_allocation() {
        let mut body = FlowDocumentV2::empty("empty");
        for _ in 0..8 {
            body = wrapped(&body, 1);
        }
        assert!(expand_composition(&body).is_ok());
        assert_eq!(
            code(expand_composition(&wrapped(&body, 1))),
            "CompositionDepthLimit"
        );
        let body = wrapped(&FlowDocumentV2::empty("empty"), 50);
        assert_eq!(
            code(expand_composition(&wrapped(&body, 50))),
            "CompositionNodeLimit"
        );
    }

    #[test]
    fn repeated_empty_body_is_still_a_finite_chain() {
        let expanded = expand_composition(&wrapped(&FlowDocumentV2::empty("empty"), 3)).unwrap();
        assert_eq!(expanded.document.nodes.len(), 8);
        assert_eq!(expanded.document.edges.len(), 7);
    }

    #[test]
    fn compiled_composition_routes_branches_and_keeps_source_iteration_metadata() {
        let body = linear(vec![
            (
                ActionKind::SetVariable,
                json!({"name":"result","value":"ready"}),
            ),
            (
                ActionKind::Log,
                json!({"message":"status","variable":"result"}),
            ),
        ]);
        let doc = linear(vec![
            (
                ActionKind::Repeat,
                json!({"document":body,"count":2,"outputs":{"outer":"result"}}),
            ),
            (
                ActionKind::Log,
                json!({"message":"done","variable":"outer"}),
            ),
        ]);
        let compiled = crate::flow::compile_flow(&doc, &riviu_core::release_one_catalog()).unwrap();
        assert_eq!(
            compiled
                .plan
                .nodes
                .values()
                .filter(|node| node.kind == ActionKind::SetVariable)
                .count(),
            2
        );
        assert!(!compiled.plan.source_paths.is_empty());
        assert!(compiled
            .plan
            .nodes
            .values()
            .all(|node| !matches!(node.kind, ActionKind::Subflow | ActionKind::Repeat)));
        assert!(compiled.canonical_json.contains("sourcePaths"));
    }

    #[test]
    fn connector_interpolation_is_scoped_but_literal_assignments_stay_literal() {
        let body = linear(vec![
            (
                ActionKind::SetVariable,
                json!({"name":"x","value":"${literal}"}),
            ),
            (
                ActionKind::HttpRequest,
                json!({"name":"response","url":"https://example.test/${x}","method":"POST","body":{"field":"${x}","bad":"${not.valid}"},"timeoutMs":1000}),
            ),
        ]);
        let expanded = expand_composition(&wrapped(&body, 2)).unwrap();
        let assigned: Vec<_> = expanded
            .document
            .nodes
            .iter()
            .filter(|n| n.kind == ActionKind::SetVariable)
            .collect();
        let requests: Vec<_> = expanded
            .document
            .nodes
            .iter()
            .filter(|n| n.kind == ActionKind::HttpRequest)
            .collect();
        for (assignment, request) in assigned.iter().zip(requests) {
            assert_eq!(assignment.config["value"], "${literal}");
            let name = assignment.config["name"].as_str().unwrap();
            assert_eq!(
                request.config["url"],
                format!("https://example.test/${{{name}}}")
            );
            assert_eq!(request.config["body"]["field"], format!("${{{name}}}"));
            assert_eq!(request.config["body"]["bad"], "${not.valid}");
        }
    }

    #[test]
    fn compile_rejects_missing_input_and_output_writers_and_hashes_snapshot_revision() {
        let body = linear(vec![(
            ActionKind::Log,
            json!({"message":"child","variable":"in"}),
        )]);
        let missing_input = linear(vec![(
            ActionKind::Subflow,
            json!({"document":body,"inputs":{"in":"missing"}}),
        )]);
        assert!(
            crate::flow::compile_flow(&missing_input, &riviu_core::release_one_catalog())
                .unwrap_err()
                .iter()
                .any(|e| e.code == "VariableUndefined")
        );
        let missing_output = linear(vec![(
            ActionKind::Subflow,
            json!({"document":FlowDocumentV2::empty("empty"),"outputs":{"parent":"missing"}}),
        )]);
        assert!(
            crate::flow::compile_flow(&missing_output, &riviu_core::release_one_catalog())
                .unwrap_err()
                .iter()
                .any(|e| e.code == "VariableUndefined")
        );
        let mut body = linear(vec![(ActionKind::Wait, json!({"durationMs":1}))]);
        body.revision = 1;
        let mut root = wrapped(&body, 1);
        let first = crate::flow::compile_flow(&root, &riviu_core::release_one_catalog()).unwrap();
        root.nodes[1].config["document"]["revision"] = 2.into();
        let second = crate::flow::compile_flow(&root, &riviu_core::release_one_catalog()).unwrap();
        assert_ne!(first.sha256, second.sha256);
        assert_eq!(first.plan.nodes, second.plan.nodes);
    }
}
