use std::collections::{BTreeMap, BTreeSet, VecDeque};

use riviu_core::{
    canonical_compiled_plan_json, compiled_plan_sha256, contracts, validate_artifact_label,
    ActionDefinition, ActionKind, CompiledActionConfig, CompiledFlowNode, CompiledFlowPlanV2,
    CompiledTapTarget, ContextPlan, EvidenceKind, EvidenceRequirement, EvidenceSpec,
    FlowDocumentV2, ImageCoordinateTarget, NodeId, QualifiedElementLocator, ResourceClass,
    FLOW_SCHEMA_VERSION,
};
use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Clone, serde::Serialize, PartialEq, Eq, thiserror::Error)]
#[serde(rename_all = "camelCase")]
#[error("{message}")]
pub struct FlowCompileError {
    pub code: String,
    pub message: String,
    pub node_id: Option<NodeId>,
    pub field: Option<String>,
}

impl FlowCompileError {
    pub fn document(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            node_id: None,
            field: None,
        }
    }

    pub fn node(
        code: impl Into<String>,
        message: impl Into<String>,
        node_id: NodeId,
        field: Option<impl Into<String>>,
    ) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            node_id: Some(node_id),
            field: field.map(Into::into),
        }
    }
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CompiledRevision {
    pub plan: CompiledFlowPlanV2,
    pub canonical_json: String,
    pub sha256: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EmptyConfig {}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct LaunchAppConfig {
    bundle_id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct TerminateAppConfig {
    bundle_id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct WaitConfig {
    duration_ms: u64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct TapConfig {
    point: Option<ImageCoordinateTarget>,
    accessibility_id: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SwipeConfig {
    from: ImageCoordinateTarget,
    to: ImageCoordinateTarget,
    duration_ms: u64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct TypeTextConfig {
    text: String,
    read_back_locator: QualifiedElementLocator,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ScreenshotConfig {
    label: String,
    format: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AssertVisibleConfig {
    accessibility_id: String,
}

pub fn compile_flow(
    document: &FlowDocumentV2,
    catalog: &[ActionDefinition],
) -> Result<CompiledRevision, Vec<FlowCompileError>> {
    let mut errors = Vec::new();
    if document.schema_version != FLOW_SCHEMA_VERSION {
        errors.push(FlowCompileError::document(
            "SchemaVersionUnsupported",
            format!(
                "flow schema {} is unsupported; expected {FLOW_SCHEMA_VERSION}",
                document.schema_version
            ),
        ));
    }
    if !document.viewport.x.is_finite()
        || !document.viewport.y.is_finite()
        || !document.viewport.zoom.is_finite()
    {
        errors.push(FlowCompileError::document(
            "NonFiniteCoordinate",
            "viewport coordinates must be finite",
        ));
    }

    let definitions: BTreeMap<_, _> = catalog
        .iter()
        .map(|definition| (definition.kind, definition))
        .collect();
    if definitions.len() != catalog.len() {
        errors.push(FlowCompileError::document(
            "DuplicateActionDefinition",
            "catalog contains duplicate action definitions",
        ));
    }

    let mut nodes = BTreeMap::new();
    for node in &document.nodes {
        if nodes.insert(node.id, node).is_some() {
            errors.push(FlowCompileError::node(
                "DuplicateNodeId",
                "node ID is duplicated",
                node.id,
                None::<String>,
            ));
        }
        if !node.position.x.is_finite() || !node.position.y.is_finite() {
            errors.push(FlowCompileError::node(
                "NonFiniteCoordinate",
                "canvas coordinates must be finite",
                node.id,
                Some("position"),
            ));
        }
        if !f0_feature_enabled(node.kind) {
            errors.push(FlowCompileError::node(
                "FeatureNotEnabled",
                format!("{:?} is not enabled in F0", node.kind),
                node.id,
                Some("kind"),
            ));
            continue;
        }
        match definitions.get(&node.kind) {
            None => errors.push(FlowCompileError::node(
                "FeatureNotEnabled",
                format!("{:?} is not enabled in this catalog", node.kind),
                node.id,
                Some("kind"),
            )),
            Some(definition) if definition.disabled_reason.is_some() => {
                errors.push(FlowCompileError::node(
                    "ActionDisabled",
                    definition.disabled_reason.clone().unwrap_or_default(),
                    node.id,
                    Some("kind"),
                ));
            }
            Some(_) => {}
        }
    }

    let start_count = nodes
        .values()
        .filter(|node| node.kind == ActionKind::Start)
        .count();
    if start_count != 1 {
        errors.push(FlowCompileError::document(
            "StartCount",
            format!("expected exactly one Start node, found {start_count}"),
        ));
    }
    let end_count = nodes
        .values()
        .filter(|node| node.kind == ActionKind::End)
        .count();
    if end_count != 1 {
        errors.push(FlowCompileError::document(
            "EndCount",
            format!("expected exactly one End node, found {end_count}"),
        ));
    }

    let mut edge_ids = BTreeSet::new();
    let mut incoming: BTreeMap<NodeId, Vec<NodeId>> =
        nodes.keys().map(|id| (*id, Vec::new())).collect();
    let mut outgoing: BTreeMap<NodeId, Vec<NodeId>> =
        nodes.keys().map(|id| (*id, Vec::new())).collect();
    for edge in &document.edges {
        if !edge_ids.insert(edge.id) {
            errors.push(FlowCompileError::document(
                "DuplicateEdgeId",
                format!("edge ID {} is duplicated", edge.id),
            ));
        }
        if edge.source_port != "flow" || edge.target_port != "flow" {
            errors.push(FlowCompileError::document(
                "InvalidPort",
                format!("edge {} must connect flow ports", edge.id),
            ));
        }
        if !nodes.contains_key(&edge.source_node_id) || !nodes.contains_key(&edge.target_node_id) {
            errors.push(FlowCompileError::document(
                "UnknownEdgeNode",
                format!("edge {} references an unknown node", edge.id),
            ));
            continue;
        }
        outgoing
            .get_mut(&edge.source_node_id)
            .expect("known source")
            .push(edge.target_node_id);
        incoming
            .get_mut(&edge.target_node_id)
            .expect("known target")
            .push(edge.source_node_id);
    }

    for (&node_id, node) in &nodes {
        let incoming_count = incoming.get(&node_id).map_or(0, Vec::len);
        let outgoing_count = outgoing.get(&node_id).map_or(0, Vec::len);
        let valid = match node.kind {
            ActionKind::Start => incoming_count == 0 && outgoing_count == 1,
            ActionKind::End => incoming_count == 1 && outgoing_count == 0,
            _ => incoming_count == 1 && outgoing_count == 1,
        };
        if !valid {
            errors.push(FlowCompileError::node(
                "InvalidDegree",
                format!(
                    "{:?} requires a linear flow degree; got {incoming_count} incoming and {outgoing_count} outgoing",
                    node.kind
                ),
                node_id,
                Some("edges"),
            ));
        }
    }

    if nodes.get(&document.entry_node_id).map(|node| node.kind) != Some(ActionKind::Start) {
        errors.push(FlowCompileError::document(
            "EntryNodeInvalid",
            "entryNodeId must identify the Start node",
        ));
    }

    if graph_has_cycle(&nodes, &incoming, &outgoing) {
        errors.push(FlowCompileError::document(
            "Cycle",
            "release-one flows must be acyclic",
        ));
    }
    let execution_order = linear_walk(document.entry_node_id, &outgoing, nodes.len());
    let visited: BTreeSet<_> = execution_order.iter().copied().collect();
    for node_id in nodes.keys().filter(|node_id| !visited.contains(node_id)) {
        errors.push(FlowCompileError::node(
            "DisconnectedNode",
            "node is not reachable from entryNodeId",
            *node_id,
            None::<String>,
        ));
    }

    let mut compiled_nodes = BTreeMap::new();
    for (&node_id, node) in &nodes {
        if !f0_feature_enabled(node.kind) {
            continue;
        }
        let Some(definition) = definitions.get(&node.kind) else {
            continue;
        };
        if definition.disabled_reason.is_some() {
            continue;
        }
        match compile_config(node.kind, &node.config) {
            Ok(config) => {
                validate_evidence(node, definition, &config, &mut errors);
                compiled_nodes.insert(
                    node_id,
                    CompiledFlowNode {
                        id: node_id,
                        kind: node.kind,
                        config,
                        postcondition: node.postcondition.clone(),
                    },
                );
            }
            Err(config_error) => errors.push(FlowCompileError::node(
                config_error.code,
                config_error.message,
                node_id,
                config_error.field,
            )),
        }
    }

    if !errors.is_empty() {
        sort_errors(&mut errors);
        return Err(errors);
    }

    let ordered_kinds = execution_order
        .iter()
        .map(|node_id| nodes[node_id].kind)
        .collect::<Vec<_>>();
    let mut context_plan = context_plan(ordered_kinds.iter().copied());
    let executable = execution_order
        .iter()
        .filter(|node_id| !matches!(nodes[node_id].kind, ActionKind::Start | ActionKind::End))
        .copied()
        .collect::<Vec<_>>();
    let launch_nodes = executable
        .iter()
        .filter(|node_id| nodes[node_id].kind == ActionKind::LaunchApp)
        .copied()
        .collect::<Vec<_>>();
    if context_plan.requires_ui_session
        && executable.first().map(|node_id| nodes[node_id].kind) != Some(ActionKind::LaunchApp)
    {
        errors.push(FlowCompileError::document(
            "UiSessionTargetRequired",
            "a UI-session plan must begin with Launch App",
        ));
    }
    if let Some(launch_id) = launch_nodes.first() {
        if let Some(CompiledFlowNode {
            config: CompiledActionConfig::LaunchApp { bundle_id },
            ..
        }) = compiled_nodes.get(launch_id)
        {
            context_plan.initial_bundle_id = Some(bundle_id.clone());
        }
    }
    if !errors.is_empty() {
        sort_errors(&mut errors);
        return Err(errors);
    }

    let mut action_definition_versions = BTreeMap::new();
    let mut required_capabilities = BTreeSet::new();
    for kind in ordered_kinds {
        let definition = definitions[&kind];
        action_definition_versions.insert(kind, definition.schema_version);
        required_capabilities.extend(definition.required_capabilities.iter().cloned());
    }
    let plan = CompiledFlowPlanV2 {
        schema_version: FLOW_SCHEMA_VERSION,
        flow_id: document.id,
        revision: document.revision,
        nodes: compiled_nodes,
        execution_order,
        context_plan,
        action_definition_versions,
        required_capabilities,
    };
    let canonical_json = canonical_compiled_plan_json(&plan).map_err(|error| {
        vec![FlowCompileError::document(
            "CanonicalSerialization",
            error.to_string(),
        )]
    })?;
    let sha256 = compiled_plan_sha256(&plan).map_err(|error| {
        vec![FlowCompileError::document(
            "CanonicalSerialization",
            error.to_string(),
        )]
    })?;
    Ok(CompiledRevision {
        plan,
        canonical_json,
        sha256,
    })
}

fn f0_feature_enabled(kind: ActionKind) -> bool {
    !matches!(
        kind,
        ActionKind::TerminateApp | ActionKind::RawHttp | ActionKind::RawWda | ActionKind::Shell
    )
}

#[derive(Debug)]
struct ConfigError {
    code: &'static str,
    message: String,
    field: Option<&'static str>,
}

impl ConfigError {
    fn invalid(message: impl Into<String>) -> Self {
        Self {
            code: "ConfigInvalid",
            message: message.into(),
            field: Some("config"),
        }
    }

    fn range(field: &'static str, message: impl Into<String>) -> Self {
        Self {
            code: "ConfigOutOfRange",
            message: message.into(),
            field: Some(field),
        }
    }
}

fn compile_config(kind: ActionKind, value: &Value) -> Result<CompiledActionConfig, ConfigError> {
    match kind {
        ActionKind::Start | ActionKind::End | ActionKind::Home => {
            decode::<EmptyConfig>(value)?;
            Ok(CompiledActionConfig::Empty)
        }
        ActionKind::LaunchApp => {
            let config = decode::<LaunchAppConfig>(value)?;
            validate_chars("bundleId", &config.bundle_id, 1, 255)?;
            Ok(CompiledActionConfig::LaunchApp {
                bundle_id: config.bundle_id,
            })
        }
        ActionKind::TerminateApp => {
            let config = decode::<TerminateAppConfig>(value)?;
            validate_chars("bundleId", &config.bundle_id, 1, 255)?;
            Ok(CompiledActionConfig::TerminateApp {
                bundle_id: config.bundle_id,
            })
        }
        ActionKind::Wait => {
            let config = decode::<WaitConfig>(value)?;
            validate_u64("durationMs", config.duration_ms, 1, 60_000)?;
            Ok(CompiledActionConfig::Wait {
                duration_ms: config.duration_ms,
            })
        }
        ActionKind::Tap => {
            let config = decode::<TapConfig>(value)?;
            let target = match (config.point, config.accessibility_id) {
                (Some(point), None) => {
                    validate_coordinate("point", &point)?;
                    CompiledTapTarget::Point { target: point }
                }
                (None, Some(value)) => {
                    validate_chars("accessibilityId", &value, 1, 512)?;
                    CompiledTapTarget::AccessibilityId { value }
                }
                _ => {
                    return Err(ConfigError::invalid(
                        "Tap requires exactly one of point or accessibilityId",
                    ));
                }
            };
            Ok(CompiledActionConfig::Tap { target })
        }
        ActionKind::Swipe => {
            let config = decode::<SwipeConfig>(value)?;
            validate_coordinate("from", &config.from)?;
            validate_coordinate("to", &config.to)?;
            validate_u64("durationMs", config.duration_ms, 1, 5_000)?;
            Ok(CompiledActionConfig::Swipe {
                from: config.from,
                to: config.to,
                duration_ms: config.duration_ms,
            })
        }
        ActionKind::TypeText => {
            reject_unsupported_selector(value)?;
            let config = decode::<TypeTextConfig>(value)?;
            validate_chars("text", &config.text, 1, 4_096)?;
            validate_chars(
                "readBackLocator.value",
                &config.read_back_locator.value,
                1,
                512,
            )?;
            Ok(CompiledActionConfig::TypeText {
                text: config.text,
                read_back_locator: config.read_back_locator,
            })
        }
        ActionKind::Screenshot => {
            let config = decode::<ScreenshotConfig>(value)?;
            if config.format != "jpeg" {
                return Err(ConfigError::range(
                    "format",
                    "release one supports only jpeg screenshots",
                ));
            }
            validate_artifact_label(&config.label, &config.format).map_err(|code| ConfigError {
                code: "InvalidArtifactLabel",
                message: code.into(),
                field: Some("label"),
            })?;
            Ok(CompiledActionConfig::Screenshot {
                label: config.label,
                format: config.format,
            })
        }
        ActionKind::AssertVisible => {
            let config = decode::<AssertVisibleConfig>(value)?;
            validate_chars("accessibilityId", &config.accessibility_id, 1, 512)?;
            Ok(CompiledActionConfig::AssertVisible {
                accessibility_id: config.accessibility_id,
            })
        }
        ActionKind::RawHttp | ActionKind::RawWda | ActionKind::Shell => {
            Err(ConfigError::invalid("raw actions are not enabled"))
        }
    }
}

fn decode<T: for<'de> Deserialize<'de>>(value: &Value) -> Result<T, ConfigError> {
    serde_json::from_value(value.clone()).map_err(|error| ConfigError::invalid(error.to_string()))
}

fn validate_chars(
    field: &'static str,
    value: &str,
    minimum: usize,
    maximum: usize,
) -> Result<(), ConfigError> {
    let count = value.chars().count();
    if !(minimum..=maximum).contains(&count) {
        return Err(ConfigError::range(
            field,
            format!("{field} must contain {minimum}..={maximum} characters"),
        ));
    }
    Ok(())
}

fn validate_u64(
    field: &'static str,
    value: u64,
    minimum: u64,
    maximum: u64,
) -> Result<(), ConfigError> {
    if !(minimum..=maximum).contains(&value) {
        return Err(ConfigError::range(
            field,
            format!("{field} must be in {minimum}..={maximum}"),
        ));
    }
    Ok(())
}

fn validate_coordinate(
    field: &'static str,
    coordinate: &ImageCoordinateTarget,
) -> Result<(), ConfigError> {
    if !coordinate.x.is_finite() || !coordinate.y.is_finite() {
        return Err(ConfigError::range(field, "coordinates must be finite"));
    }
    if coordinate.image_width == 0 || coordinate.image_height == 0 {
        return Err(ConfigError::range(
            field,
            "image dimensions must be positive",
        ));
    }
    if coordinate.profile_id.len() != 64
        || !coordinate.profile_id.is_ascii()
        || !coordinate
            .profile_id
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(ConfigError::range(
            field,
            "profileId must be exactly 64 lowercase hexadecimal characters",
        ));
    }
    Ok(())
}

fn reject_unsupported_selector(value: &Value) -> Result<(), ConfigError> {
    let Some(strategy) = value
        .get("readBackLocator")
        .and_then(|locator| locator.get("strategy"))
        .and_then(Value::as_str)
    else {
        return Ok(());
    };
    if !matches!(strategy, "accessibilityId" | "className") {
        return Err(ConfigError {
            code: "UnsupportedSelector",
            message: format!("selector strategy {strategy:?} is not supported"),
            field: Some("readBackLocator.strategy"),
        });
    }
    Ok(())
}

fn validate_evidence(
    node: &riviu_core::FlowNode,
    definition: &ActionDefinition,
    config: &CompiledActionConfig,
    errors: &mut Vec<FlowCompileError>,
) {
    if node.postcondition.is_none() && definition.evidence_requirement != EvidenceRequirement::None
    {
        errors.push(FlowCompileError::node(
            "EvidenceRequired",
            format!("{:?} requires a postcondition", node.kind),
            node.id,
            Some("postcondition"),
        ));
        return;
    }
    let Some(evidence) = node.postcondition.as_ref() else {
        return;
    };
    let kind = evidence_kind(evidence);
    if !definition.allowed_evidence.contains(&kind) {
        errors.push(FlowCompileError::node(
            "EvidenceNotEnabled",
            format!("{kind:?} is not enabled for {:?}", node.kind),
            node.id,
            Some("postcondition"),
        ));
        return;
    }

    let matches_config = match (node.kind, config, evidence) {
        (
            ActionKind::LaunchApp,
            CompiledActionConfig::LaunchApp {
                bundle_id: configured,
            },
            EvidenceSpec::ActiveAppEquals {
                bundle_id: expected,
            },
        ) => configured == expected,
        (
            ActionKind::TerminateApp,
            CompiledActionConfig::TerminateApp {
                bundle_id: configured,
            },
            EvidenceSpec::ProcessAbsent {
                bundle_id: expected,
            },
        ) => configured == expected,
        (ActionKind::Home, _, EvidenceSpec::ActiveAppEquals { bundle_id }) => {
            bundle_id == "com.apple.springboard"
        }
        (
            ActionKind::TypeText,
            CompiledActionConfig::TypeText {
                text,
                read_back_locator,
            },
            EvidenceSpec::TextReadBackEquals { locator, value },
        ) => text == value && read_back_locator == locator,
        (ActionKind::Screenshot, _, EvidenceSpec::ArtifactDecodedAndHashed)
        | (ActionKind::Tap, _, EvidenceSpec::FrameRegionChanged { .. })
        | (ActionKind::Swipe, _, EvidenceSpec::FrameDigestChanged { .. }) => true,
        _ => false,
    };
    if !matches_config {
        errors.push(FlowCompileError::node(
            "EvidenceMismatch",
            "postcondition does not match the compiled action config",
            node.id,
            Some("postcondition"),
        ));
    }
}

fn evidence_kind(evidence: &EvidenceSpec) -> EvidenceKind {
    match evidence {
        EvidenceSpec::ActiveAppEquals { .. } => EvidenceKind::ActiveAppEquals,
        EvidenceSpec::ProcessAbsent { .. } => EvidenceKind::ProcessAbsent,
        EvidenceSpec::FrameDigestChanged { .. } => EvidenceKind::FrameDigestChanged,
        EvidenceSpec::FrameRegionChanged { .. } => EvidenceKind::FrameRegionChanged,
        EvidenceSpec::QualifiedFramePredicate { .. } => EvidenceKind::QualifiedFramePredicate,
        EvidenceSpec::AccessibilityVisible { .. } => EvidenceKind::AccessibilityVisible,
        EvidenceSpec::TextReadBackEquals { .. } => EvidenceKind::TextReadBackEquals,
        EvidenceSpec::ArtifactDecodedAndHashed => EvidenceKind::ArtifactDecodedAndHashed,
    }
}

fn context_plan(nodes: impl Iterator<Item = ActionKind>) -> ContextPlan {
    let mut plan = ContextPlan {
        requires_exclusive: false,
        requires_ui_session: false,
        requires_stream: false,
        requires_fresh_text_session: false,
        initial_bundle_id: None,
    };
    for kind in nodes {
        let (resource, _, _, _, _) = contracts(kind);
        plan.requires_exclusive |= resource != ResourceClass::PureDesktop;
        plan.requires_ui_session |= matches!(
            resource,
            ResourceClass::UiSession | ResourceClass::UiWithStream
        );
        plan.requires_stream |= resource == ResourceClass::UiWithStream;
        plan.requires_fresh_text_session |= kind == ActionKind::TypeText;
    }
    plan
}

fn graph_has_cycle(
    nodes: &BTreeMap<NodeId, &riviu_core::FlowNode>,
    incoming: &BTreeMap<NodeId, Vec<NodeId>>,
    outgoing: &BTreeMap<NodeId, Vec<NodeId>>,
) -> bool {
    let mut indegrees: BTreeMap<_, _> = nodes
        .keys()
        .map(|node_id| (*node_id, incoming.get(node_id).map_or(0, Vec::len)))
        .collect();
    let mut queue: VecDeque<_> = indegrees
        .iter()
        .filter_map(|(&node_id, &degree)| (degree == 0).then_some(node_id))
        .collect();
    let mut processed = 0;
    while let Some(node_id) = queue.pop_front() {
        processed += 1;
        for target in outgoing.get(&node_id).into_iter().flatten() {
            let degree = indegrees.get_mut(target).expect("known target");
            *degree = degree.saturating_sub(1);
            if *degree == 0 {
                queue.push_back(*target);
            }
        }
    }
    processed != nodes.len()
}

fn linear_walk(
    entry_node_id: NodeId,
    outgoing: &BTreeMap<NodeId, Vec<NodeId>>,
    node_count: usize,
) -> Vec<NodeId> {
    let mut order = Vec::new();
    let mut current = Some(entry_node_id);
    while let Some(node_id) = current {
        if order.contains(&node_id) || order.len() >= node_count {
            break;
        }
        order.push(node_id);
        current = outgoing
            .get(&node_id)
            .and_then(|targets| (targets.len() == 1).then(|| targets[0]));
    }
    order
}

fn sort_errors(errors: &mut [FlowCompileError]) {
    errors.sort_by(|left, right| {
        left.node_id
            .cmp(&right.node_id)
            .then_with(|| left.code.cmp(&right.code))
            .then_with(|| left.field.cmp(&right.field))
            .then_with(|| left.message.cmp(&right.message))
    });
}

#[cfg(test)]
mod tests {
    use riviu_core::{
        release_one_catalog, ActionKind, CanvasPoint, CompiledActionConfig, EvidenceSpec,
        FlowDocumentV2, FlowEdge, FlowNode, FlowViewport, ImageCoordinateTarget,
        FLOW_SCHEMA_VERSION,
    };
    use serde_json::{json, Value};
    use uuid::Uuid;

    use super::*;

    fn linear_document(nodes: Vec<FlowNode>) -> FlowDocumentV2 {
        let entry_node_id = nodes[0].id;
        let edges = nodes
            .windows(2)
            .map(|pair| FlowEdge::flow(pair[0].id, pair[1].id))
            .collect();
        FlowDocumentV2 {
            schema_version: FLOW_SCHEMA_VERSION,
            id: Uuid::from_u128(1),
            name: "Fixture".into(),
            revision: 1,
            entry_node_id,
            nodes,
            edges,
            viewport: FlowViewport::default(),
        }
    }

    fn start() -> FlowNode {
        FlowNode::new(ActionKind::Start, json!({}))
    }

    fn end() -> FlowNode {
        FlowNode::new(ActionKind::End, json!({}))
    }

    fn launch(bundle_id: &str) -> FlowNode {
        let mut node = FlowNode::new(ActionKind::LaunchApp, json!({ "bundleId": bundle_id }));
        node.postcondition = Some(EvidenceSpec::ActiveAppEquals {
            bundle_id: bundle_id.into(),
        });
        node
    }

    fn tap_config() -> Value {
        json!({
            "point": {
                "x": 100.0,
                "y": 200.0,
                "imageWidth": 375,
                "imageHeight": 667,
                "orientation": "portrait",
                "profileId": "11".repeat(32)
            }
        })
    }

    fn tap_with_evidence() -> FlowNode {
        let mut node = FlowNode::new(ActionKind::Tap, tap_config());
        node.postcondition = Some(EvidenceSpec::FrameRegionChanged {
            x: 90,
            y: 190,
            width: 20,
            height: 20,
            minimum_distance: 8,
        });
        node
    }

    fn swipe_with_evidence(config: Value) -> FlowNode {
        let mut node = FlowNode::new(ActionKind::Swipe, config);
        node.postcondition = Some(EvidenceSpec::FrameDigestChanged {
            minimum_distance: 8,
        });
        node
    }

    fn compile(document: &FlowDocumentV2) -> Result<CompiledRevision, Vec<FlowCompileError>> {
        compile_flow(document, &release_one_catalog())
    }

    fn assert_error(document: &FlowDocumentV2, code: &str) {
        let errors = compile(document).expect_err(code);
        assert!(
            errors.iter().any(|error| error.code == code),
            "missing {code}: {errors:?}"
        );
    }

    #[test]
    fn valid_start_launch_end_compiles_to_typed_nodes() {
        let document = linear_document(vec![start(), launch("com.apple.Preferences"), end()]);
        let compiled = compile(&document).expect("valid graph");
        assert_eq!(compiled.plan.execution_order.len(), 3);
        assert_eq!(
            compiled.plan.context_plan.initial_bundle_id.as_deref(),
            Some("com.apple.Preferences")
        );
        let launch_id = compiled.plan.execution_order[1];
        assert!(matches!(
            compiled.plan.nodes[&launch_id].config,
            CompiledActionConfig::LaunchApp { .. }
        ));
        assert_eq!(
            compiled.canonical_json,
            riviu_core::canonical_compiled_plan_json(&compiled.plan).unwrap()
        );
        assert_eq!(
            compiled.sha256,
            riviu_core::compiled_plan_sha256(&compiled.plan).unwrap()
        );
    }

    #[test]
    fn topology_validation_rejects_duplicate_missing_cycle_disconnected_and_bad_ports() {
        let mut duplicate = linear_document(vec![start(), end()]);
        duplicate.nodes[1].id = duplicate.nodes[0].id;
        assert_error(&duplicate, "DuplicateNodeId");

        let missing_end = linear_document(vec![
            start(),
            FlowNode::new(
                ActionKind::Wait,
                json!({
                    "durationMs": 1
                }),
            ),
        ]);
        assert_error(&missing_end, "EndCount");

        let mut cycle = linear_document(vec![start(), launch("com.apple.Preferences"), end()]);
        cycle
            .edges
            .push(FlowEdge::flow(cycle.nodes[2].id, cycle.nodes[1].id));
        assert_error(&cycle, "Cycle");

        let mut disconnected = linear_document(vec![start(), end()]);
        disconnected
            .nodes
            .push(FlowNode::new(ActionKind::Wait, json!({ "durationMs": 1 })));
        assert_error(&disconnected, "DisconnectedNode");

        let mut invalid_port = linear_document(vec![start(), end()]);
        invalid_port.edges[0].source_port = "data".into();
        assert_error(&invalid_port, "InvalidPort");
    }

    #[test]
    fn non_finite_canvas_coordinates_are_rejected() {
        let mut document = linear_document(vec![start(), end()]);
        document.nodes[0].position = CanvasPoint {
            x: f64::NAN,
            y: 0.0,
        };
        assert_error(&document, "NonFiniteCoordinate");

        let mut viewport = linear_document(vec![start(), end()]);
        viewport.viewport.zoom = f64::INFINITY;
        assert_error(&viewport, "NonFiniteCoordinate");
    }

    #[test]
    fn tap_without_qualified_postcondition_is_rejected() {
        let document = linear_document(vec![
            start(),
            launch("com.apple.Preferences"),
            FlowNode::new(ActionKind::Tap, tap_config()),
            end(),
        ]);
        assert_error(&document, "EvidenceRequired");
    }

    #[test]
    fn tap_rejects_whole_frame_and_other_unqualified_evidence() {
        let mut tap = FlowNode::new(ActionKind::Tap, tap_config());
        tap.postcondition = Some(EvidenceSpec::FrameDigestChanged {
            minimum_distance: 8,
        });
        let document = linear_document(vec![start(), launch("com.apple.Preferences"), tap, end()]);
        assert_error(&document, "EvidenceNotEnabled");
    }

    #[test]
    fn layout_and_input_order_do_not_change_plan_hash() {
        let first = linear_document(vec![
            start(),
            launch("com.apple.Preferences"),
            tap_with_evidence(),
            end(),
        ]);
        let mut second = first.clone();
        second.nodes.reverse();
        second.edges.reverse();
        second.viewport = FlowViewport {
            x: 900.0,
            y: 400.0,
            zoom: 1.5,
        };
        for node in &mut second.nodes {
            node.position.x += 73.0;
        }
        let first = compile(&first).expect("first plan");
        let second = compile(&second).expect("second plan");
        assert_eq!(first.sha256, second.sha256);
        assert_eq!(first.canonical_json, second.canonical_json);
    }

    #[test]
    fn wait_and_swipe_duration_boundaries_match_the_catalog() {
        for (duration, accepted) in [(0, false), (1, true), (60_000, true), (60_001, false)] {
            let document = linear_document(vec![
                start(),
                FlowNode::new(ActionKind::Wait, json!({ "durationMs": duration })),
                end(),
            ]);
            assert_eq!(compile(&document).is_ok(), accepted, "Wait {duration}");
        }

        for (duration, accepted) in [(0, false), (1, true), (5_000, true), (5_001, false)] {
            let point = json!({
                "x": 100.0, "y": 200.0,
                "imageWidth": 375, "imageHeight": 667,
                "orientation": "portrait", "profileId": "11".repeat(32)
            });
            let swipe = swipe_with_evidence(json!({
                "from": point,
                "to": {
                    "x": 100.0, "y": 100.0,
                    "imageWidth": 375, "imageHeight": 667,
                    "orientation": "portrait", "profileId": "11".repeat(32)
                },
                "durationMs": duration
            }));
            let document =
                linear_document(vec![start(), launch("com.apple.Preferences"), swipe, end()]);
            assert_eq!(compile(&document).is_ok(), accepted, "Swipe {duration}");
        }
    }

    #[test]
    fn string_boundaries_match_the_catalog() {
        for (length, accepted) in [(0, false), (1, true), (255, true), (256, false)] {
            let bundle = "b".repeat(length);
            let document = linear_document(vec![start(), launch(&bundle), end()]);
            assert_eq!(compile(&document).is_ok(), accepted, "bundle {length}");
        }

        for (length, accepted) in [(0, false), (1, true), (512, true), (513, false)] {
            let id = "a".repeat(length);
            let document = linear_document(vec![
                start(),
                launch("com.apple.Preferences"),
                FlowNode::new(ActionKind::AssertVisible, json!({ "accessibilityId": id })),
                end(),
            ]);
            assert_eq!(
                compile(&document).is_ok(),
                accepted,
                "accessibility {length}"
            );
        }
    }

    #[test]
    fn type_text_bounds_and_exact_read_back_are_enforced() {
        for (length, accepted) in [(0, false), (1, true), (4_096, true), (4_097, false)] {
            let text = "x".repeat(length);
            let locator = riviu_core::QualifiedElementLocator {
                strategy: riviu_core::ElementLocatorStrategy::ClassName,
                value: "XCUIElementTypeSearchField".into(),
            };
            let mut node = FlowNode::new(
                ActionKind::TypeText,
                json!({
                    "text": text,
                    "readBackLocator": locator,
                }),
            );
            node.postcondition = Some(EvidenceSpec::TextReadBackEquals {
                locator,
                value: text,
            });
            let document =
                linear_document(vec![start(), launch("com.apple.Preferences"), node, end()]);
            assert_eq!(compile(&document).is_ok(), accepted, "text {length}");
        }

        let locator = riviu_core::QualifiedElementLocator {
            strategy: riviu_core::ElementLocatorStrategy::AccessibilityId,
            value: "Search".into(),
        };
        let mut mismatch = FlowNode::new(
            ActionKind::TypeText,
            json!({
                "text": "expected",
                "readBackLocator": locator,
            }),
        );
        mismatch.postcondition = Some(EvidenceSpec::TextReadBackEquals {
            locator,
            value: "different".into(),
        });
        let document = linear_document(vec![
            start(),
            launch("com.apple.Preferences"),
            mismatch,
            end(),
        ]);
        assert_error(&document, "EvidenceMismatch");
    }

    #[test]
    fn locator_and_screenshot_boundaries_match_the_catalog() {
        for (length, accepted) in [(0, false), (1, true), (512, true), (513, false)] {
            let value = "f".repeat(length);
            let locator = riviu_core::QualifiedElementLocator {
                strategy: riviu_core::ElementLocatorStrategy::AccessibilityId,
                value: value.clone(),
            };
            let mut node = FlowNode::new(
                ActionKind::TypeText,
                json!({
                    "text": "x",
                    "readBackLocator": locator,
                }),
            );
            node.postcondition = Some(EvidenceSpec::TextReadBackEquals {
                locator,
                value: "x".into(),
            });
            let document =
                linear_document(vec![start(), launch("com.apple.Preferences"), node, end()]);
            assert_eq!(compile(&document).is_ok(), accepted, "locator {length}");
        }

        for (length, accepted) in [(0, false), (1, true), (96, true), (97, false)] {
            let mut node = FlowNode::new(
                ActionKind::Screenshot,
                json!({
                    "label": "a".repeat(length),
                    "format": "jpeg",
                }),
            );
            node.postcondition = Some(EvidenceSpec::ArtifactDecodedAndHashed);
            let document =
                linear_document(vec![start(), launch("com.apple.Preferences"), node, end()]);
            assert_eq!(compile(&document).is_ok(), accepted, "label {length}");
        }

        let mut png = FlowNode::new(
            ActionKind::Screenshot,
            json!({
                "label": "capture.png",
                "format": "png",
            }),
        );
        png.postcondition = Some(EvidenceSpec::ArtifactDecodedAndHashed);
        assert_error(
            &linear_document(vec![start(), launch("com.apple.Preferences"), png, end()]),
            "ConfigOutOfRange",
        );
    }

    #[test]
    fn integer_fields_reject_fractional_json_numbers() {
        let document = linear_document(vec![
            start(),
            FlowNode::new(ActionKind::Wait, json!({ "durationMs": 1.5 })),
            end(),
        ]);
        assert_error(&document, "ConfigInvalid");
    }

    #[test]
    fn profile_id_and_coordinate_boundaries_are_enforced() {
        for (profile, accepted) in [
            ("1".repeat(63), false),
            ("1".repeat(64), true),
            ("1".repeat(65), false),
            ("A".repeat(64), false),
            ("z".repeat(64), false),
        ] {
            let mut config = tap_config();
            config["point"]["profileId"] = json!(profile);
            let mut tap = tap_with_evidence();
            tap.config = config;
            let document =
                linear_document(vec![start(), launch("com.apple.Preferences"), tap, end()]);
            assert_eq!(compile(&document).is_ok(), accepted, "profile");
        }

        for field in ["imageWidth", "imageHeight"] {
            let mut config = tap_config();
            config["point"][field] = json!(0);
            let mut tap = tap_with_evidence();
            tap.config = config;
            let document =
                linear_document(vec![start(), launch("com.apple.Preferences"), tap, end()]);
            assert_error(&document, "ConfigOutOfRange");
        }
    }

    #[test]
    fn every_release_config_rejects_unknown_fields_and_non_objects() {
        let cases = [
            (ActionKind::Start, json!({})),
            (ActionKind::End, json!({})),
            (
                ActionKind::LaunchApp,
                json!({ "bundleId": "com.apple.Preferences" }),
            ),
            (ActionKind::Wait, json!({ "durationMs": 1 })),
            (ActionKind::Tap, json!({ "accessibilityId": "Button" })),
            (
                ActionKind::Swipe,
                json!({
                    "from": { "x": 1.0, "y": 2.0, "imageWidth": 375, "imageHeight": 667,
                        "orientation": "portrait", "profileId": "11".repeat(32) },
                    "to": { "x": 2.0, "y": 1.0, "imageWidth": 375, "imageHeight": 667,
                        "orientation": "portrait", "profileId": "11".repeat(32) },
                    "durationMs": 1
                }),
            ),
            (
                ActionKind::TypeText,
                json!({
                    "text": "x",
                    "readBackLocator": { "strategy": "accessibilityId", "value": "Field" }
                }),
            ),
            (
                ActionKind::Screenshot,
                json!({ "label": "capture", "format": "jpeg" }),
            ),
            (ActionKind::Home, json!({})),
            (
                ActionKind::AssertVisible,
                json!({ "accessibilityId": "Button" }),
            ),
        ];

        for (kind, config) in cases {
            let mut with_extra = config.as_object().unwrap().clone();
            with_extra.insert("extra".into(), json!(true));
            let node = FlowNode::new(kind, Value::Object(with_extra));
            let document = document_for_node(node);
            assert_error(&document, "ConfigInvalid");

            let node = FlowNode::new(kind, json!("not-an-object"));
            let document = document_for_node(node);
            assert_error(&document, "ConfigInvalid");
        }
    }

    #[test]
    fn tap_requires_exactly_one_target_and_rejects_unsupported_selectors() {
        let mut both = tap_config();
        both["accessibilityId"] = json!("Button");
        let mut tap = tap_with_evidence();
        tap.config = both;
        assert_error(
            &linear_document(vec![start(), launch("com.apple.Preferences"), tap, end()]),
            "ConfigInvalid",
        );

        let mut neither = tap_with_evidence();
        neither.config = json!({});
        assert_error(
            &linear_document(vec![
                start(),
                launch("com.apple.Preferences"),
                neither,
                end(),
            ]),
            "ConfigInvalid",
        );

        for strategy in ["xpath", "predicate", "classChain"] {
            let node = FlowNode::new(
                ActionKind::TypeText,
                json!({
                    "text": "x",
                    "readBackLocator": { "strategy": strategy, "value": "query" }
                }),
            );
            let document =
                linear_document(vec![start(), launch("com.apple.Preferences"), node, end()]);
            assert_error(&document, "UnsupportedSelector");
        }
    }

    #[test]
    fn ui_session_requires_launch_first_and_allows_later_launch_nodes() {
        let mut home = FlowNode::new(ActionKind::Home, json!({}));
        home.postcondition = Some(EvidenceSpec::ActiveAppEquals {
            bundle_id: "com.apple.springboard".into(),
        });
        assert_error(
            &linear_document(vec![start(), home, end()]),
            "UiSessionTargetRequired",
        );

        let document = linear_document(vec![
            start(),
            FlowNode::new(ActionKind::Wait, json!({ "durationMs": 1 })),
            launch("com.apple.Preferences"),
            tap_with_evidence(),
            end(),
        ]);
        assert_error(&document, "UiSessionTargetRequired");

        let later_launch = linear_document(vec![
            start(),
            launch("com.apple.Preferences"),
            launch("com.apple.MobileSafari"),
            end(),
        ]);
        let compiled = compile(&later_launch).expect("later Launch runs after session upgrade");
        assert_eq!(
            compiled.plan.context_plan.initial_bundle_id.as_deref(),
            Some("com.apple.Preferences")
        );

        let wait_only = linear_document(vec![
            start(),
            FlowNode::new(ActionKind::Wait, json!({ "durationMs": 1 })),
            end(),
        ]);
        assert!(compile(&wait_only).is_ok());
    }

    #[test]
    fn raw_and_terminate_actions_are_not_enabled_in_f0() {
        for kind in [
            ActionKind::TerminateApp,
            ActionKind::RawHttp,
            ActionKind::RawWda,
            ActionKind::Shell,
        ] {
            let document = linear_document(vec![start(), FlowNode::new(kind, json!({})), end()]);
            assert_error(&document, "FeatureNotEnabled");
        }
    }

    #[test]
    fn f0_feature_gate_cannot_be_bypassed_by_an_injected_catalog_definition() {
        for kind in [ActionKind::TerminateApp, ActionKind::RawHttp] {
            let mut catalog = release_one_catalog();
            let mut injected = catalog[0].clone();
            injected.kind = kind;
            injected.disabled_reason = None;
            catalog.push(injected);

            let mut node = FlowNode::new(kind, json!({ "bundleId": "com.example.fixture" }));
            if kind == ActionKind::TerminateApp {
                node.postcondition = Some(EvidenceSpec::ProcessAbsent {
                    bundle_id: "com.example.fixture".into(),
                });
            }
            let document = linear_document(vec![start(), node, end()]);
            let errors = compile_flow(&document, &catalog).expect_err("F0 feature gate");
            assert!(
                errors.iter().any(|error| error.code == "FeatureNotEnabled"),
                "{kind:?}: {errors:?}"
            );
        }
    }

    fn document_for_node(mut node: FlowNode) -> FlowDocumentV2 {
        match node.kind {
            ActionKind::Start => {
                let mut document = linear_document(vec![node, end()]);
                document.nodes[0].postcondition = None;
                document
            }
            ActionKind::End => linear_document(vec![start(), node]),
            ActionKind::LaunchApp | ActionKind::Wait => linear_document(vec![start(), node, end()]),
            ActionKind::Home => {
                node.postcondition = Some(EvidenceSpec::ActiveAppEquals {
                    bundle_id: "com.apple.springboard".into(),
                });
                linear_document(vec![start(), launch("com.apple.Preferences"), node, end()])
            }
            ActionKind::Tap => {
                node.postcondition = Some(EvidenceSpec::FrameRegionChanged {
                    x: 0,
                    y: 0,
                    width: 1,
                    height: 1,
                    minimum_distance: 1,
                });
                linear_document(vec![start(), launch("com.apple.Preferences"), node, end()])
            }
            ActionKind::Swipe => {
                node.postcondition = Some(EvidenceSpec::FrameDigestChanged {
                    minimum_distance: 1,
                });
                linear_document(vec![start(), launch("com.apple.Preferences"), node, end()])
            }
            ActionKind::TypeText => {
                linear_document(vec![start(), launch("com.apple.Preferences"), node, end()])
            }
            ActionKind::Screenshot => {
                node.postcondition = Some(EvidenceSpec::ArtifactDecodedAndHashed);
                linear_document(vec![start(), launch("com.apple.Preferences"), node, end()])
            }
            ActionKind::AssertVisible => {
                linear_document(vec![start(), launch("com.apple.Preferences"), node, end()])
            }
            ActionKind::TerminateApp
            | ActionKind::RawHttp
            | ActionKind::RawWda
            | ActionKind::Shell => linear_document(vec![start(), node, end()]),
        }
    }

    #[allow(dead_code)]
    fn assert_typed_coordinate(target: &ImageCoordinateTarget) {
        assert!(target.x.is_finite());
    }
}
