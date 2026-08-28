use std::collections::{BTreeMap, BTreeSet, VecDeque};

use riviu_core::{
    canonical_compiled_plan_json, compiled_plan_sha256, contracts, decode_vision_template,
    release_one_catalog, validate_artifact_label, validate_vision_region, ActionDefinition,
    ActionKind, AutomationScript, CanvasPoint, CompiledActionConfig, CompiledFlowNode,
    CompiledFlowPlanV2, CompiledTapTarget, ContextPlan, EvidenceKind, EvidenceRequirement,
    EvidenceSpec, FlowDocumentV2, FlowEdge, FlowNode, FlowViewport, ImageCoordinateTarget, NodeId,
    QualifiedElementLocator, ResourceClass, ScriptAction, VisionRegion, FLOW_SCHEMA_VERSION,
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

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct TapVisionConfig {
    template_png_base64: String,
    threshold: f64,
    #[serde(default)]
    region: Option<VisionRegion>,
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
        if !release_one_feature_enabled(node.kind) {
            errors.push(FlowCompileError::node(
                "FeatureNotEnabled",
                format!("{:?} is not enabled in release one", node.kind),
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
    // Adjacency keyed by the source node's output port. Each port routes to
    // exactly one target: branch nodes expose several ports, linear nodes only
    // `flow`. This is the graph the executor walks at runtime.
    let mut successors: BTreeMap<NodeId, BTreeMap<String, NodeId>> = BTreeMap::new();
    for edge in &document.edges {
        if !edge_ids.insert(edge.id) {
            errors.push(FlowCompileError::document(
                "DuplicateEdgeId",
                format!("edge ID {} is duplicated", edge.id),
            ));
        }
        if edge.target_port != "flow" {
            errors.push(FlowCompileError::document(
                "InvalidPort",
                format!("edge {} must enter the flow port", edge.id),
            ));
        }
        if !nodes.contains_key(&edge.source_node_id) || !nodes.contains_key(&edge.target_node_id) {
            errors.push(FlowCompileError::document(
                "UnknownEdgeNode",
                format!("edge {} references an unknown node", edge.id),
            ));
            continue;
        }
        // The source port must be one the source node's kind actually exposes.
        let source_kind = nodes[&edge.source_node_id].kind;
        let port_is_declared =
            definitions
                .get(&source_kind)
                .map_or(edge.source_port == "flow", |definition| {
                    definition
                        .output_ports
                        .iter()
                        .any(|port| port.name == edge.source_port)
                });
        if !port_is_declared {
            errors.push(FlowCompileError::document(
                "InvalidPort",
                format!(
                    "edge {} leaves port {:?}, which {source_kind:?} does not expose",
                    edge.id, edge.source_port
                ),
            ));
            continue;
        }
        if successors
            .entry(edge.source_node_id)
            .or_default()
            .insert(edge.source_port.clone(), edge.target_node_id)
            .is_some()
        {
            errors.push(FlowCompileError::document(
                "InvalidPort",
                format!(
                    "port {:?} of a node fans out to more than one target",
                    edge.source_port
                ),
            ));
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
        let node_successors = successors.get(&node_id);
        let valid = match node.kind {
            ActionKind::Start => incoming_count == 0 && outgoing_count == 1,
            // End may be the join of several branches: one or more in, none out.
            ActionKind::End => incoming_count >= 1 && outgoing_count == 0,
            // Branch predicate: one in, both typed ports wired to distinct edges.
            ActionKind::IfVision => {
                incoming_count == 1
                    && outgoing_count == 2
                    && node_successors.is_some_and(|ports| {
                        ports.contains_key("matched") && ports.contains_key("notMatched")
                    })
            }
            _ => incoming_count == 1 && outgoing_count == 1,
        };
        if !valid {
            errors.push(FlowCompileError::node(
                "InvalidDegree",
                format!(
                    "{:?} has an invalid flow degree; got {incoming_count} incoming and {outgoing_count} outgoing",
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
    let execution_order = topological_order(&nodes, &incoming, &outgoing);
    let reachable = reachable_from(document.entry_node_id, &outgoing);
    for node_id in nodes.keys().filter(|node_id| !reachable.contains(node_id)) {
        errors.push(FlowCompileError::node(
            "DisconnectedNode",
            "node is not reachable from entryNodeId",
            *node_id,
            None::<String>,
        ));
    }

    let mut compiled_nodes = BTreeMap::new();
    for (&node_id, node) in &nodes {
        if !release_one_feature_enabled(node.kind) {
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
        successors,
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

fn release_one_feature_enabled(kind: ActionKind) -> bool {
    !matches!(
        kind,
        ActionKind::RawHttp | ActionKind::RawWda | ActionKind::Shell
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
            validate_bundle_id("bundleId", &config.bundle_id)?;
            Ok(CompiledActionConfig::LaunchApp {
                bundle_id: config.bundle_id,
            })
        }
        ActionKind::TerminateApp => {
            let config = decode::<TerminateAppConfig>(value)?;
            validate_bundle_id("bundleId", &config.bundle_id)?;
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
        ActionKind::TapVision => {
            let config = decode::<TapVisionConfig>(value)?;
            if !config.threshold.is_finite() || !(0.0..=1.0).contains(&config.threshold) {
                return Err(ConfigError::range(
                    "threshold",
                    "threshold must be in 0.0..=1.0",
                ));
            }
            decode_vision_template(&config.template_png_base64)
                .map_err(|message| ConfigError::range("templatePngBase64", message))?;
            if let Some(region) = &config.region {
                validate_vision_region(region)
                    .map_err(|message| ConfigError::range("region", message))?;
            }
            Ok(CompiledActionConfig::TapVision {
                template_png_base64: config.template_png_base64,
                threshold: config.threshold,
                region: config.region,
            })
        }
        ActionKind::IfVision => {
            let config = decode::<TapVisionConfig>(value)?;
            if !config.threshold.is_finite() || !(0.0..=1.0).contains(&config.threshold) {
                return Err(ConfigError::range(
                    "threshold",
                    "threshold must be in 0.0..=1.0",
                ));
            }
            decode_vision_template(&config.template_png_base64)
                .map_err(|message| ConfigError::range("templatePngBase64", message))?;
            if let Some(region) = &config.region {
                validate_vision_region(region)
                    .map_err(|message| ConfigError::range("region", message))?;
            }
            Ok(CompiledActionConfig::IfVision {
                template_png_base64: config.template_png_base64,
                threshold: config.threshold,
                region: config.region,
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

/// A bundle id the runtime will also accept.
///
/// `validate_chars("bundleId", .., 1, 255)` counted characters, so a single space passed
/// compilation and saved fine — and then `evidence.rs::validate_bundle` rejected it at run time
/// (`bundle_id.trim().is_empty() || bundle_id.trim() != bundle_id`), after the process baseline had
/// already been read. A `TerminateApp` node configured with `" "` compiled, saved, and could never
/// dispatch; the operator had a flow that failed on every device with nothing pointing at the
/// field. Refuse it where the field is, and say which field.
fn validate_bundle_id(field: &'static str, value: &str) -> Result<(), ConfigError> {
    validate_chars(field, value, 1, 255)?;
    if value.trim().is_empty() || value.trim() != value {
        return Err(ConfigError::range(
            field,
            format!("{field} must not be blank or padded with spaces"),
        ));
    }
    Ok(())
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

    // Refuse evidence the runtime is *guaranteed* to refuse, here, where the field is named.
    //
    // Compilation checked the kind/action pairing and nothing about the numbers, so
    // `frameRegionChanged` with a zero width or height compiled and saved — and then
    // `evidence.rs::validate_region` rejected it while capturing the baseline, which happens
    // *before* the intent is committed. Every run of that flow failed before the tap ever went
    // out, with the node attempt left queued and no issue pointing at the postcondition. Same for a
    // blank or padded bundle id inside `activeAppEquals`/`processAbsent`, which
    // `evidence.rs::validate_bundle` refuses.
    if let Some(unsatisfiable) = unsatisfiable_evidence(evidence) {
        errors.push(FlowCompileError::node(
            "EvidenceUnsatisfiable",
            unsatisfiable,
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
        | (ActionKind::TapVision, _, EvidenceSpec::FrameRegionChanged { .. })
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

/// The reason this postcondition can never be satisfied, if there is one.
///
/// Mirrors the runtime's own refusals in `riviu_core::flow::evidence`; anything this returns would
/// otherwise surface as a run that fails identically on every device.
fn unsatisfiable_evidence(evidence: &EvidenceSpec) -> Option<String> {
    match evidence {
        EvidenceSpec::FrameRegionChanged { width, height, .. } => {
            if *width == 0 || *height == 0 {
                Some(format!(
                    "frame region {width}x{height} has no area, so no change can ever be measured"
                ))
            } else {
                None
            }
        }
        EvidenceSpec::ActiveAppEquals { bundle_id } | EvidenceSpec::ProcessAbsent { bundle_id } => {
            if bundle_id.trim().is_empty() || bundle_id.trim() != bundle_id {
                Some("bundle id must not be blank or padded with spaces".to_string())
            } else {
                None
            }
        }
        _ => None,
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
        plan.requires_fresh_text_session |=
            matches!(kind, ActionKind::TypeText | ActionKind::AssertVisible);
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

/// A deterministic topological order over every node (Kahn's algorithm, ties
/// broken by NodeId). `Start` is the unique in-degree-0 node so it leads; the
/// result is independent of document node/edge ordering, which keeps the plan
/// hash stable. Assumes the graph is acyclic — a cycle is reported separately
/// via `graph_has_cycle`, and the (partial) order returned here is never used
/// because compilation aborts on that error first.
fn topological_order(
    nodes: &BTreeMap<NodeId, &riviu_core::FlowNode>,
    incoming: &BTreeMap<NodeId, Vec<NodeId>>,
    outgoing: &BTreeMap<NodeId, Vec<NodeId>>,
) -> Vec<NodeId> {
    let mut indegree: BTreeMap<NodeId, usize> = nodes
        .keys()
        .map(|id| (*id, incoming.get(id).map_or(0, Vec::len)))
        .collect();
    let mut ready: BTreeSet<NodeId> = indegree
        .iter()
        .filter_map(|(&id, &degree)| (degree == 0).then_some(id))
        .collect();
    let mut order = Vec::with_capacity(nodes.len());
    while let Some(&node_id) = ready.iter().next() {
        ready.remove(&node_id);
        order.push(node_id);
        for &target in outgoing.get(&node_id).into_iter().flatten() {
            let degree = indegree.get_mut(&target).expect("known target");
            *degree = degree.saturating_sub(1);
            if *degree == 0 {
                ready.insert(target);
            }
        }
    }
    order
}

/// Every node reachable from `entry` by walking output edges — used to flag
/// nodes stranded off the graph (`DisconnectedNode`).
fn reachable_from(entry: NodeId, outgoing: &BTreeMap<NodeId, Vec<NodeId>>) -> BTreeSet<NodeId> {
    let mut seen = BTreeSet::new();
    let mut stack = vec![entry];
    while let Some(node_id) = stack.pop() {
        if !seen.insert(node_id) {
            continue;
        }
        for &target in outgoing.get(&node_id).into_iter().flatten() {
            if !seen.contains(&target) {
                stack.push(target);
            }
        }
    }
    seen
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

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LegacyImportDiagnostic {
    pub step_index: usize,
    pub code: String,
    pub message: String,
    pub field: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LegacyImportResult {
    pub document: Option<FlowDocumentV2>,
    pub diagnostics: Vec<LegacyImportDiagnostic>,
}

pub fn import_legacy_v1(script: &AutomationScript) -> LegacyImportResult {
    if script.version != 1 {
        return LegacyImportResult {
            document: None,
            diagnostics: vec![LegacyImportDiagnostic {
                step_index: 0,
                code: "UnsupportedVersion".into(),
                message: format!("legacy script version {} is unsupported", script.version),
                field: Some("version".into()),
            }],
        };
    }

    let mut diagnostics = Vec::new();
    let mut nodes = Vec::with_capacity(script.steps.len() + 2);
    let mut start = FlowNode::new(ActionKind::Start, serde_json::json!({}));
    start.position = CanvasPoint { x: 0.0, y: 80.0 };
    let entry_node_id = start.id;
    nodes.push(start);

    {
        let mut push_node = |kind, config, postcondition| {
            let mut node = FlowNode::new(kind, config);
            node.position = CanvasPoint {
                x: (nodes.len() as f64) * 220.0,
                y: 80.0,
            };
            node.postcondition = postcondition;
            nodes.push(node);
        };

        for (index, step) in script.steps.iter().enumerate() {
            match step {
                ScriptAction::LaunchApp { bundle_id } => push_node(
                    ActionKind::LaunchApp,
                    serde_json::json!({ "bundleId": bundle_id }),
                    Some(EvidenceSpec::ActiveAppEquals {
                        bundle_id: bundle_id.clone(),
                    }),
                ),
                ScriptAction::TerminateApp { bundle_id } => push_node(
                    ActionKind::TerminateApp,
                    serde_json::json!({ "bundleId": bundle_id }),
                    Some(EvidenceSpec::ProcessAbsent {
                        bundle_id: bundle_id.clone(),
                    }),
                ),
                ScriptAction::Wait { milliseconds } if (1..=60_000).contains(milliseconds) => {
                    push_node(
                        ActionKind::Wait,
                        serde_json::json!({ "durationMs": milliseconds }),
                        None,
                    );
                }
                ScriptAction::Screenshot { name }
                    if validate_artifact_label(name, "jpeg").is_ok() =>
                {
                    push_node(
                        ActionKind::Screenshot,
                        serde_json::json!({ "label": name, "format": "jpeg" }),
                        Some(EvidenceSpec::ArtifactDecodedAndHashed),
                    );
                }
                ScriptAction::Home => push_node(
                    ActionKind::Home,
                    serde_json::json!({}),
                    Some(EvidenceSpec::ActiveAppEquals {
                        bundle_id: "com.apple.springboard".into(),
                    }),
                ),
                ScriptAction::AssertVisible { selector }
                    if selector
                        .accessibility_id
                        .as_deref()
                        .is_some_and(|value| !value.is_empty())
                        && selector.xpath.is_none()
                        && selector.predicate.is_none() =>
                {
                    push_node(
                        ActionKind::AssertVisible,
                        serde_json::json!({
                            "accessibilityId": selector.accessibility_id.as_deref().unwrap_or_default()
                        }),
                        None,
                    );
                }
                unsupported => diagnostics.push(diagnostic_for_legacy_step(index, unsupported)),
            }
        }
    }

    if !diagnostics.is_empty() {
        return LegacyImportResult {
            document: None,
            diagnostics,
        };
    }

    let mut end = FlowNode::new(ActionKind::End, serde_json::json!({}));
    end.position = CanvasPoint {
        x: (nodes.len() as f64) * 220.0,
        y: 80.0,
    };
    nodes.push(end);
    let edges = nodes
        .windows(2)
        .map(|pair| FlowEdge::flow(pair[0].id, pair[1].id))
        .collect();
    let document = FlowDocumentV2 {
        schema_version: FLOW_SCHEMA_VERSION,
        id: uuid::Uuid::new_v4(),
        name: script.name.clone(),
        revision: 0,
        entry_node_id,
        nodes,
        edges,
        viewport: FlowViewport::default(),
    };
    if let Err(compile_errors) = compile_flow(&document, &release_one_catalog()) {
        let step_by_node: BTreeMap<_, _> = document
            .nodes
            .iter()
            .skip(1)
            .take(script.steps.len())
            .enumerate()
            .map(|(index, node)| (node.id, index))
            .collect();
        diagnostics.extend(compile_errors.into_iter().map(|error| {
            LegacyImportDiagnostic {
                step_index: error
                    .node_id
                    .and_then(|node_id| step_by_node.get(&node_id).copied())
                    .unwrap_or(0),
                code: error.code,
                message: error.message,
                field: error.field,
            }
        }));
        sort_legacy_diagnostics(&mut diagnostics);
        return LegacyImportResult {
            document: None,
            diagnostics,
        };
    }
    LegacyImportResult {
        document: Some(document),
        diagnostics,
    }
}

fn diagnostic_for_legacy_step(index: usize, step: &ScriptAction) -> LegacyImportDiagnostic {
    let (code, message, field) = match step {
        ScriptAction::Wait { .. } => (
            "WaitOutOfRange",
            "legacy Wait must be between 1 and 60000 milliseconds",
            "milliseconds",
        ),
        ScriptAction::TerminateApp { .. } => {
            unreachable!("TerminateApp is handled before legacy diagnostics")
        }
        ScriptAction::Tap {
            selector: Some(_),
            point: Some(_),
        } => (
            "UnsupportedSelector",
            "legacy Tap cannot preserve both selector and point semantics",
            "selector",
        ),
        ScriptAction::Tap {
            selector: Some(selector),
            point: None,
        } if selector.xpath.is_some()
            || selector.predicate.is_some()
            || selector
                .accessibility_id
                .as_deref()
                .is_none_or(str::is_empty) =>
        {
            (
                "UnsupportedSelector",
                "legacy Tap selector is not supported",
                "selector",
            )
        }
        ScriptAction::Tap {
            selector: Some(_),
            point: None,
        } => (
            "EvidenceRequired",
            "legacy Tap has no qualified postcondition",
            "postcondition",
        ),
        ScriptAction::Tap {
            selector: None,
            point: Some(_),
        } => (
            "GeometryRequired",
            "legacy Tap coordinates have no qualified geometry profile",
            "point",
        ),
        ScriptAction::Tap {
            selector: None,
            point: None,
        } => (
            "EvidenceRequired",
            "legacy Tap has neither a target nor qualified evidence",
            "postcondition",
        ),
        ScriptAction::Swipe { .. } => (
            "GeometryRequired",
            "legacy Swipe coordinates have no qualified geometry profile",
            "gesture",
        ),
        ScriptAction::TypeText { .. } => (
            "EvidenceRequired",
            "legacy Type Text has no read-back target",
            "readBackLocator",
        ),
        ScriptAction::Screenshot { .. } => (
            "ArtifactLabelInvalid",
            "legacy Screenshot label is not portable",
            "name",
        ),
        ScriptAction::AssertVisible { .. } => (
            "UnsupportedSelector",
            "Assert Visible requires exactly one non-empty accessibility ID",
            "selector",
        ),
        ScriptAction::LaunchApp { .. } | ScriptAction::Home => (
            "LegacyShapeUnsupported",
            "legacy action shape cannot be imported",
            "action",
        ),
    };
    LegacyImportDiagnostic {
        step_index: index,
        code: code.into(),
        message: message.into(),
        field: Some(field.into()),
    }
}

fn sort_legacy_diagnostics(diagnostics: &mut [LegacyImportDiagnostic]) {
    diagnostics.sort_by(|left, right| {
        left.step_index
            .cmp(&right.step_index)
            .then_with(|| left.code.cmp(&right.code))
            .then_with(|| left.field.cmp(&right.field))
            .then_with(|| left.message.cmp(&right.message))
    });
}

#[cfg(test)]
mod tests {
    use riviu_core::{
        release_one_catalog, ActionKind, AutomationScript, CanvasPoint, CompiledActionConfig,
        ElementSelector, EvidenceSpec, FlowDocumentV2, FlowEdge, FlowNode, FlowViewport,
        ImageCoordinateTarget, ScriptAction, SwipeGesture, TapPoint, FLOW_SCHEMA_VERSION,
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

    fn branch_edge(source: NodeId, port: &str, target: NodeId) -> FlowEdge {
        FlowEdge {
            id: Uuid::new_v4(),
            source_node_id: source,
            source_port: port.into(),
            target_node_id: target,
            target_port: "flow".into(),
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

    fn terminate(bundle_id: &str) -> FlowNode {
        let mut node = FlowNode::new(ActionKind::TerminateApp, json!({ "bundleId": bundle_id }));
        node.postcondition = Some(EvidenceSpec::ProcessAbsent {
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

    /// A valid 2x2 RGB PNG, base64-encoded (correct CRCs; decodes in the `image`
    /// crate, which validates chunk CRCs strictly).
    const PNG_1X1: &str =
        "iVBORw0KGgoAAAANSUhEUgAAAAIAAAACCAIAAAD91JpzAAAAEElEQVR4nGP8zwACTGCSAQANHQEDgslx/wAAAABJRU5ErkJggg==";

    fn tap_vision_node(template: &str, threshold: f64) -> FlowNode {
        let mut node = FlowNode::new(
            ActionKind::TapVision,
            json!({ "templatePngBase64": template, "threshold": threshold }),
        );
        node.postcondition = Some(EvidenceSpec::FrameRegionChanged {
            x: 0,
            y: 0,
            width: 1,
            height: 1,
            minimum_distance: 1,
        });
        node
    }

    #[test]
    fn tap_vision_compiles_a_valid_template_and_rejects_bad_config() {
        let ok = linear_document(vec![
            start(),
            launch("com.apple.Preferences"),
            tap_vision_node(PNG_1X1, 0.85),
            end(),
        ]);
        assert!(compile(&ok).is_ok(), "valid tap vision must compile");

        let bad_threshold = linear_document(vec![
            start(),
            launch("com.apple.Preferences"),
            tap_vision_node(PNG_1X1, 1.5),
            end(),
        ]);
        assert_error(&bad_threshold, "ConfigOutOfRange");

        // Valid base64 ("not-a-png") that is not a PNG must be rejected.
        let bad_template = linear_document(vec![
            start(),
            launch("com.apple.Preferences"),
            tap_vision_node("bm90LWEtcG5n", 0.85),
            end(),
        ]);
        assert_error(&bad_template, "ConfigOutOfRange");
    }

    #[test]
    fn if_vision_compiles_to_a_branching_plan_with_typed_successors() {
        let start_node = start();
        let launch_node = launch("com.apple.Preferences");
        let if_node = FlowNode::new(
            ActionKind::IfVision,
            json!({ "templatePngBase64": PNG_1X1, "threshold": 0.8 }),
        );
        let matched_tail = FlowNode::new(ActionKind::Wait, json!({ "durationMs": 1 }));
        let end_node = end();
        // start → launch → ifVision; matched → wait → end; notMatched → end.
        let document = FlowDocumentV2 {
            schema_version: FLOW_SCHEMA_VERSION,
            id: Uuid::from_u128(7),
            name: "Branch".into(),
            revision: 1,
            entry_node_id: start_node.id,
            edges: vec![
                FlowEdge::flow(start_node.id, launch_node.id),
                FlowEdge::flow(launch_node.id, if_node.id),
                branch_edge(if_node.id, "matched", matched_tail.id),
                branch_edge(if_node.id, "notMatched", end_node.id),
                FlowEdge::flow(matched_tail.id, end_node.id),
            ],
            nodes: vec![
                start_node,
                launch_node,
                if_node.clone(),
                matched_tail.clone(),
                end_node.clone(),
            ],
            viewport: FlowViewport::default(),
        };
        let plan = compile(&document).expect("branching flow compiles").plan;

        let ports = plan.successors.get(&if_node.id).expect("branch successors");
        assert_eq!(ports.get("matched"), Some(&matched_tail.id));
        assert_eq!(ports.get("notMatched"), Some(&end_node.id));
        // The matched tail rejoins the shared End node.
        assert_eq!(
            plan.successors
                .get(&matched_tail.id)
                .and_then(|ports| ports.get("flow")),
            Some(&end_node.id)
        );
        // successor_on_path honours the runtime-chosen branch.
        assert_eq!(
            plan.successor_on_path(if_node.id, Some("matched")),
            Some(matched_tail.id)
        );
        assert_eq!(
            plan.successor_on_path(if_node.id, Some("notMatched")),
            Some(end_node.id)
        );
        // Every node appears exactly once in the topological order.
        assert_eq!(plan.execution_order.len(), plan.nodes.len());
    }

    #[test]
    fn if_vision_requires_both_branch_ports() {
        let start_node = start();
        let launch_node = launch("com.apple.Preferences");
        let if_node = FlowNode::new(
            ActionKind::IfVision,
            json!({ "templatePngBase64": PNG_1X1, "threshold": 0.8 }),
        );
        let end_node = end();
        // Only the matched port is wired — notMatched is missing.
        let document = FlowDocumentV2 {
            schema_version: FLOW_SCHEMA_VERSION,
            id: Uuid::from_u128(8),
            name: "Half branch".into(),
            revision: 1,
            entry_node_id: start_node.id,
            edges: vec![
                FlowEdge::flow(start_node.id, launch_node.id),
                FlowEdge::flow(launch_node.id, if_node.id),
                branch_edge(if_node.id, "matched", end_node.id),
            ],
            nodes: vec![start_node, launch_node, if_node, end_node],
            viewport: FlowViewport::default(),
        };
        assert_error(&document, "InvalidDegree");
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

        // A cycle between two Wait nodes. Each node's single `flow` port is used
        // exactly once (no fan-out), so the graph reaches cycle detection rather
        // than tripping the port checks first.
        let start_node = start();
        let first = FlowNode::new(ActionKind::Wait, json!({ "durationMs": 1 }));
        let second = FlowNode::new(ActionKind::Wait, json!({ "durationMs": 1 }));
        let end_node = end();
        let cycle = FlowDocumentV2 {
            schema_version: FLOW_SCHEMA_VERSION,
            id: Uuid::from_u128(1),
            name: "Fixture".into(),
            revision: 1,
            entry_node_id: start_node.id,
            edges: vec![
                FlowEdge::flow(start_node.id, first.id),
                FlowEdge::flow(first.id, second.id),
                FlowEdge::flow(second.id, first.id),
            ],
            nodes: vec![start_node, first, second, end_node],
            viewport: FlowViewport::default(),
        };
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
    fn assert_visible_requests_a_fresh_readback_session_without_a_stream() {
        let assertion = FlowNode::new(
            ActionKind::AssertVisible,
            json!({"accessibilityId": "SearchField"}),
        );
        let document = linear_document(vec![
            start(),
            launch("com.apple.Preferences"),
            assertion,
            end(),
        ]);

        let compiled = compile(&document).expect("Assert Visible flow");
        assert!(compiled.plan.context_plan.requires_ui_session);
        assert!(compiled.plan.context_plan.requires_fresh_text_session);
        assert!(!compiled.plan.context_plan.requires_stream);
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
            (
                ActionKind::TapVision,
                json!({ "templatePngBase64": "aGVsbG8=", "threshold": 0.85 }),
            ),
            (
                ActionKind::IfVision,
                json!({ "templatePngBase64": "aGVsbG8=", "threshold": 0.85 }),
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
        let compiled = compile(&wait_only).expect("target-free Wait flow");
        assert!(!compiled.plan.context_plan.requires_exclusive);
        assert!(!compiled.plan.context_plan.requires_ui_session);
        assert!(!compiled.plan.context_plan.requires_stream);
        assert_eq!(compiled.plan.context_plan.initial_bundle_id, None);
    }

    #[test]
    fn verified_terminate_is_a_bridge_only_compiled_action() {
        let compiled = compile(&linear_document(vec![
            start(),
            terminate("com.example.fixture"),
            end(),
        ]))
        .expect("verified Terminate compiles without a UI launch");
        assert_eq!(compiled.plan.context_plan.initial_bundle_id, None);
        assert!(compiled.plan.context_plan.requires_exclusive);
        assert!(!compiled.plan.context_plan.requires_ui_session);
        assert!(!compiled.plan.context_plan.requires_stream);
        assert!(matches!(
            compiled.plan.nodes[&compiled.plan.execution_order[1]].config,
            CompiledActionConfig::TerminateApp { ref bundle_id }
                if bundle_id == "com.example.fixture"
        ));
        assert!(compiled
            .plan
            .required_capabilities
            .contains("app.terminate"));
    }

    #[test]
    fn raw_actions_are_not_enabled_in_release_one() {
        for kind in [ActionKind::RawHttp, ActionKind::RawWda, ActionKind::Shell] {
            let document = linear_document(vec![start(), FlowNode::new(kind, json!({})), end()]);
            assert_error(&document, "FeatureNotEnabled");
        }
    }

    #[test]
    fn a_postcondition_the_runtime_can_never_satisfy_fails_compilation() {
        // `validate_evidence` checked the kind/action pairing and nothing about the numbers, so a
        // zero-area region compiled and saved — and then `evidence.rs::validate_region` refused it
        // while capturing the baseline, which happens *before* the intent is committed. Every run
        // of that flow failed before the tap ever went out, with the attempt left queued and no
        // issue pointing at the postcondition.
        let catalog = release_one_catalog();
        for (label, width, height) in [
            ("zero width", 0u32, 10u32),
            ("zero height", 10, 0),
            ("no area at all", 0, 0),
        ] {
            let mut node = FlowNode::new(
                ActionKind::Tap,
                json!({ "point": { "x": 10, "y": 20, "imageWidth": 100, "imageHeight": 200,
                                   "orientation": "portrait", "profileId": "a".repeat(64) } }),
            );
            node.postcondition = Some(EvidenceSpec::FrameRegionChanged {
                x: 0,
                y: 0,
                width,
                height,
                minimum_distance: 1,
            });
            let document = linear_document(vec![start(), node, end()]);
            let errors =
                compile_flow(&document, &catalog).expect_err(&format!("{label} must not compile"));
            assert!(
                errors
                    .iter()
                    .any(|error| error.code == "EvidenceUnsatisfiable"
                        && error.field.as_deref() == Some("postcondition")),
                "{label}: {errors:?}"
            );
        }

        // The same region with area compiles, so the gate refuses the impossible and nothing else.
        let mut ok = FlowNode::new(
            ActionKind::Tap,
            json!({ "point": { "x": 10, "y": 20, "imageWidth": 100, "imageHeight": 200,
                               "orientation": "portrait", "profileId": "a".repeat(64) } }),
        );
        ok.postcondition = Some(EvidenceSpec::FrameRegionChanged {
            x: 0,
            y: 0,
            width: 10,
            height: 10,
            minimum_distance: 1,
        });
        // A Tap needs a UI session, and a UI-session plan has to open with Launch App.
        let mut launch = FlowNode::new(
            ActionKind::LaunchApp,
            json!({ "bundleId": "com.example.fixture" }),
        );
        launch.postcondition = Some(EvidenceSpec::ActiveAppEquals {
            bundle_id: "com.example.fixture".into(),
        });
        compile_flow(&linear_document(vec![start(), launch, ok, end()]), &catalog)
            .expect("a region with area is satisfiable");
    }

    #[test]
    fn a_bundle_id_the_runtime_will_reject_fails_compilation() {
        // `validate_chars("bundleId", .., 1, 255)` counted characters, so a single space passed and
        // `evidence.rs::validate_bundle` refused it at run time — after the process baseline had
        // been read. A Terminate App node configured with `" "` compiled, saved, and could never
        // dispatch on any device.
        let catalog = release_one_catalog();
        for blank in [" ", "  ", " com.example.padded", "com.example.padded "] {
            let mut node = FlowNode::new(ActionKind::TerminateApp, json!({ "bundleId": blank }));
            node.postcondition = Some(EvidenceSpec::ProcessAbsent {
                bundle_id: blank.to_string(),
            });
            let document = linear_document(vec![start(), node, end()]);
            let errors = compile_flow(&document, &catalog)
                .expect_err(&format!("bundle {blank:?} must not compile"));
            // Named against the field the operator typed in, not just "something was wrong".
            // Accepting the postcondition's own refusal here as well made this case vacuous: the
            // evidence gate covered for the config gate, so the config gate went untested.
            assert!(
                errors
                    .iter()
                    .any(|error| error.field.as_deref() == Some("bundleId")),
                "bundle {blank:?}: {errors:?}"
            );
        }

        let mut ok = FlowNode::new(
            ActionKind::TerminateApp,
            json!({ "bundleId": "com.example.fixture" }),
        );
        ok.postcondition = Some(EvidenceSpec::ProcessAbsent {
            bundle_id: "com.example.fixture".into(),
        });
        compile_flow(&linear_document(vec![start(), ok, end()]), &catalog)
            .expect("a trimmed bundle id compiles");
    }

    #[test]
    fn release_feature_gate_cannot_be_bypassed_by_an_injected_raw_definition() {
        let kind = ActionKind::RawHttp;
        let mut catalog = release_one_catalog();
        let mut injected = catalog[0].clone();
        injected.kind = kind;
        injected.disabled_reason = None;
        catalog.push(injected);

        let node = FlowNode::new(kind, json!({ "bundleId": "com.example.fixture" }));
        let document = linear_document(vec![start(), node, end()]);
        let errors = compile_flow(&document, &catalog).expect_err("release feature gate");
        assert!(
            errors.iter().any(|error| error.code == "FeatureNotEnabled"),
            "{kind:?}: {errors:?}"
        );
    }

    #[test]
    fn legacy_import_accepts_only_semantics_preserving_steps() {
        let script = AutomationScript {
            version: 1,
            name: "Fixture".into(),
            steps: vec![
                ScriptAction::LaunchApp {
                    bundle_id: "com.apple.Preferences".into(),
                },
                ScriptAction::TerminateApp {
                    bundle_id: "com.example.background".into(),
                },
                ScriptAction::Wait { milliseconds: 20 },
                ScriptAction::Screenshot {
                    name: "settings".into(),
                },
                ScriptAction::Home,
            ],
        };
        let original = serde_json::to_value(&script).expect("source JSON");
        let imported = import_legacy_v1(&script);
        assert!(imported.diagnostics.is_empty());
        let document = imported.document.expect("supported document");
        assert_eq!(document.nodes.len(), 7);
        assert_eq!(document.edges.len(), 6);
        assert_eq!(document.entry_node_id, document.nodes[0].id);
        assert_eq!(
            document
                .nodes
                .iter()
                .map(|node| node.kind)
                .collect::<Vec<_>>(),
            vec![
                ActionKind::Start,
                ActionKind::LaunchApp,
                ActionKind::TerminateApp,
                ActionKind::Wait,
                ActionKind::Screenshot,
                ActionKind::Home,
                ActionKind::End,
            ]
        );
        assert_eq!(
            document
                .nodes
                .iter()
                .map(|node| node.config.clone())
                .collect::<Vec<_>>(),
            vec![
                json!({}),
                json!({ "bundleId": "com.apple.Preferences" }),
                json!({ "bundleId": "com.example.background" }),
                json!({ "durationMs": 20 }),
                json!({ "label": "settings", "format": "jpeg" }),
                json!({}),
                json!({}),
            ]
        );
        assert_eq!(
            document
                .nodes
                .iter()
                .map(|node| node.postcondition.clone())
                .collect::<Vec<_>>(),
            vec![
                None,
                Some(EvidenceSpec::ActiveAppEquals {
                    bundle_id: "com.apple.Preferences".into(),
                }),
                Some(EvidenceSpec::ProcessAbsent {
                    bundle_id: "com.example.background".into(),
                }),
                None,
                Some(EvidenceSpec::ArtifactDecodedAndHashed),
                Some(EvidenceSpec::ActiveAppEquals {
                    bundle_id: "com.apple.springboard".into(),
                }),
                None,
            ]
        );
        for (edge, pair) in document.edges.iter().zip(document.nodes.windows(2)) {
            assert_eq!(edge.source_node_id, pair[0].id);
            assert_eq!(edge.source_port, "flow");
            assert_eq!(edge.target_node_id, pair[1].id);
            assert_eq!(edge.target_port, "flow");
        }
        assert_eq!(
            serde_json::to_value(&script).expect("unchanged source JSON"),
            original
        );
    }

    #[test]
    fn legacy_import_reports_stable_diagnostics_and_returns_no_partial_document() {
        let cases = vec![
            (
                ScriptAction::Wait { milliseconds: 0 },
                "WaitOutOfRange",
                Some("milliseconds"),
            ),
            (
                ScriptAction::Wait {
                    milliseconds: 60_001,
                },
                "WaitOutOfRange",
                Some("milliseconds"),
            ),
            (
                ScriptAction::AssertVisible {
                    selector: ElementSelector {
                        accessibility_id: None,
                        xpath: Some("//XCUIElementTypeButton".into()),
                        predicate: None,
                    },
                },
                "UnsupportedSelector",
                Some("selector"),
            ),
            (
                ScriptAction::AssertVisible {
                    selector: ElementSelector {
                        accessibility_id: None,
                        xpath: None,
                        predicate: Some("label == 'OK'".into()),
                    },
                },
                "UnsupportedSelector",
                Some("selector"),
            ),
            (
                ScriptAction::Tap {
                    selector: Some(ElementSelector {
                        accessibility_id: Some("Button".into()),
                        xpath: None,
                        predicate: None,
                    }),
                    point: Some(TapPoint { x: 1.0, y: 2.0 }),
                },
                "UnsupportedSelector",
                Some("selector"),
            ),
            (
                ScriptAction::Tap {
                    selector: Some(ElementSelector {
                        accessibility_id: Some("Button".into()),
                        xpath: None,
                        predicate: None,
                    }),
                    point: None,
                },
                "EvidenceRequired",
                Some("postcondition"),
            ),
            (
                ScriptAction::Tap {
                    selector: None,
                    point: Some(TapPoint { x: 1.0, y: 2.0 }),
                },
                "GeometryRequired",
                Some("point"),
            ),
            (
                ScriptAction::Swipe {
                    gesture: SwipeGesture {
                        from: TapPoint { x: 1.0, y: 2.0 },
                        to: TapPoint { x: 2.0, y: 1.0 },
                        duration_ms: 200,
                    },
                },
                "GeometryRequired",
                Some("gesture"),
            ),
            (
                ScriptAction::TypeText { value: "x".into() },
                "EvidenceRequired",
                Some("readBackLocator"),
            ),
            (
                ScriptAction::Screenshot {
                    name: "../bad".into(),
                },
                "ArtifactLabelInvalid",
                Some("name"),
            ),
        ];

        for (index, (step, code, field)) in cases.into_iter().enumerate() {
            let script = AutomationScript {
                version: 1,
                name: "Fixture".into(),
                steps: vec![step],
            };
            let imported = import_legacy_v1(&script);
            assert!(imported.document.is_none(), "{code}");
            assert_eq!(imported.diagnostics.len(), 1, "{code}");
            let diagnostic = &imported.diagnostics[0];
            assert_eq!(diagnostic.step_index, 0, "case {index}");
            assert_eq!(diagnostic.code, code, "case {index}");
            assert_eq!(diagnostic.field.as_deref(), field, "case {index}");
        }
    }

    #[test]
    fn legacy_import_reports_non_finite_coordinates_without_serializing_them() {
        for point in [
            TapPoint {
                x: f64::NAN,
                y: 1.0,
            },
            TapPoint {
                x: 1.0,
                y: f64::INFINITY,
            },
        ] {
            let script = AutomationScript {
                version: 1,
                name: "Fixture".into(),
                steps: vec![ScriptAction::Tap {
                    selector: None,
                    point: Some(point),
                }],
            };
            let imported = import_legacy_v1(&script);
            assert!(imported.document.is_none());
            assert_eq!(imported.diagnostics[0].code, "GeometryRequired");
        }
    }

    #[test]
    fn legacy_import_never_returns_a_document_that_the_release_compiler_rejects() {
        let invalid_scripts = [
            AutomationScript {
                version: 1,
                name: "Missing launch".into(),
                steps: vec![ScriptAction::Screenshot {
                    name: "capture".into(),
                }],
            },
            AutomationScript {
                version: 1,
                name: "Empty bundle".into(),
                steps: vec![ScriptAction::LaunchApp {
                    bundle_id: String::new(),
                }],
            },
            AutomationScript {
                version: 1,
                name: "Long selector".into(),
                steps: vec![
                    ScriptAction::LaunchApp {
                        bundle_id: "com.apple.Preferences".into(),
                    },
                    ScriptAction::AssertVisible {
                        selector: ElementSelector {
                            accessibility_id: Some("a".repeat(513)),
                            xpath: None,
                            predicate: None,
                        },
                    },
                ],
            },
        ];
        for script in invalid_scripts {
            let imported = import_legacy_v1(&script);
            assert!(imported.document.is_none(), "{}", script.name);
            assert!(!imported.diagnostics.is_empty(), "{}", script.name);
        }

        let valid = AutomationScript {
            version: 1,
            name: "Valid assert".into(),
            steps: vec![
                ScriptAction::LaunchApp {
                    bundle_id: "com.apple.Preferences".into(),
                },
                ScriptAction::AssertVisible {
                    selector: ElementSelector {
                        accessibility_id: Some("Search".into()),
                        xpath: None,
                        predicate: None,
                    },
                },
            ],
        };
        let imported = import_legacy_v1(&valid);
        let document = imported.document.expect("valid imported document");
        assert!(compile(&document).is_ok());
    }

    #[test]
    fn legacy_import_orders_compiler_diagnostics_by_source_step() {
        let script = AutomationScript {
            version: 1,
            name: "Two invalid supported shapes".into(),
            steps: vec![
                ScriptAction::LaunchApp {
                    bundle_id: String::new(),
                },
                ScriptAction::AssertVisible {
                    selector: ElementSelector {
                        accessibility_id: Some("a".repeat(513)),
                        xpath: None,
                        predicate: None,
                    },
                },
            ],
        };

        for _ in 0..64 {
            let imported = import_legacy_v1(&script);
            assert!(imported.document.is_none());
            assert_eq!(
                imported
                    .diagnostics
                    .iter()
                    .map(|diagnostic| diagnostic.step_index)
                    .collect::<Vec<_>>(),
                vec![0, 1]
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
            ActionKind::TapVision => {
                node.postcondition = Some(EvidenceSpec::FrameRegionChanged {
                    x: 0,
                    y: 0,
                    width: 1,
                    height: 1,
                    minimum_distance: 1,
                });
                linear_document(vec![start(), launch("com.apple.Preferences"), node, end()])
            }
            ActionKind::IfVision => {
                // A branch node: both ports rejoin at a single End node.
                let start_node = start();
                let launch_node = launch("com.apple.Preferences");
                let end_node = end();
                let entry_node_id = start_node.id;
                let edges = vec![
                    FlowEdge::flow(start_node.id, launch_node.id),
                    FlowEdge::flow(launch_node.id, node.id),
                    branch_edge(node.id, "matched", end_node.id),
                    branch_edge(node.id, "notMatched", end_node.id),
                ];
                FlowDocumentV2 {
                    schema_version: FLOW_SCHEMA_VERSION,
                    id: Uuid::from_u128(1),
                    name: "Fixture".into(),
                    revision: 1,
                    entry_node_id,
                    nodes: vec![start_node, launch_node, node, end_node],
                    edges,
                    viewport: FlowViewport::default(),
                }
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
