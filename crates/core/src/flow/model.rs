use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use uuid::Uuid;

pub const FLOW_SCHEMA_VERSION: u32 = 2;

pub type FlowId = Uuid;
pub type NodeId = Uuid;
pub type EdgeId = Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FlowDocumentV2 {
    pub schema_version: u32,
    pub id: FlowId,
    pub name: String,
    pub revision: u64,
    pub entry_node_id: NodeId,
    pub nodes: Vec<FlowNode>,
    pub edges: Vec<FlowEdge>,
    pub viewport: FlowViewport,
}

impl FlowDocumentV2 {
    pub fn empty(name: impl Into<String>) -> Self {
        let mut start = FlowNode::new(ActionKind::Start, Value::Object(Default::default()));
        start.position = CanvasPoint { x: 0.0, y: 80.0 };
        let mut end = FlowNode::new(ActionKind::End, Value::Object(Default::default()));
        end.position = CanvasPoint { x: 320.0, y: 80.0 };
        let entry_node_id = start.id;
        let edge = FlowEdge::flow(start.id, end.id);

        Self {
            schema_version: FLOW_SCHEMA_VERSION,
            id: Uuid::new_v4(),
            name: name.into(),
            revision: 0,
            entry_node_id,
            nodes: vec![start, end],
            edges: vec![edge],
            viewport: FlowViewport::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FlowNode {
    pub id: NodeId,
    pub kind: ActionKind,
    pub position: CanvasPoint,
    pub config: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub postcondition: Option<EvidenceSpec>,
}

impl FlowNode {
    pub fn new(kind: ActionKind, config: Value) -> Self {
        Self {
            id: Uuid::new_v4(),
            kind,
            position: CanvasPoint::default(),
            config,
            postcondition: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FlowEdge {
    pub id: EdgeId,
    pub source_node_id: NodeId,
    pub source_port: String,
    pub target_node_id: NodeId,
    pub target_port: String,
}

impl FlowEdge {
    pub fn flow(source_node_id: NodeId, target_node_id: NodeId) -> Self {
        Self {
            id: Uuid::new_v4(),
            source_node_id,
            source_port: "flow".into(),
            target_node_id,
            target_port: "flow".into(),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CanvasPoint {
    pub x: f64,
    pub y: f64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FlowViewport {
    pub x: f64,
    pub y: f64,
    pub zoom: f64,
}

impl Default for FlowViewport {
    fn default() -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            zoom: 1.0,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "camelCase")]
pub enum ActionKind {
    Start,
    End,
    LaunchApp,
    TerminateApp,
    Wait,
    Tap,
    Swipe,
    TypeText,
    Screenshot,
    Home,
    AssertVisible,
    RawHttp,
    RawWda,
    Shell,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum EvidenceSpec {
    ActiveAppEquals {
        bundle_id: String,
    },
    ProcessAbsent {
        bundle_id: String,
    },
    FrameDigestChanged {
        minimum_distance: u32,
    },
    FrameRegionChanged {
        x: u32,
        y: u32,
        width: u32,
        height: u32,
        minimum_distance: u32,
    },
    QualifiedFramePredicate {
        detector_id: String,
    },
    AccessibilityVisible {
        accessibility_id: String,
    },
    TextReadBackEquals {
        locator: QualifiedElementLocator,
        value: String,
    },
    ArtifactDecodedAndHashed,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ElementLocatorStrategy {
    AccessibilityId,
    ClassName,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct QualifiedElementLocator {
    pub strategy: ElementLocatorStrategy,
    pub value: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ResourceClass {
    PureDesktop,
    Bridge,
    UiSession,
    UiWithStream,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum SideEffectClass {
    None,
    IdempotentSet,
    AmbiguousUi,
    ArtifactWrite,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum EvidenceRequirement {
    None,
    ActiveApp,
    Process,
    Frame,
    TextOrQualifiedFrame,
    Artifact,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum EvidenceKind {
    ActiveAppEquals,
    ProcessAbsent,
    FrameDigestChanged,
    FrameRegionChanged,
    QualifiedFramePredicate,
    AccessibilityVisible,
    TextReadBackEquals,
    ArtifactDecodedAndHashed,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ReconciliationPolicy {
    None,
    ReadActiveApp,
    ReadProcess,
    ReadFrame,
    ReadText,
    ReadArtifact,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum RetryPolicy {
    Never,
    BeforeDispatchOnly,
    IdempotentAfterRead,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ContextPlan {
    pub requires_exclusive: bool,
    pub requires_ui_session: bool,
    pub requires_stream: bool,
    pub requires_fresh_text_session: bool,
    pub initial_bundle_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CompiledFlowPlanV2 {
    pub schema_version: u32,
    pub flow_id: FlowId,
    pub revision: u64,
    pub nodes: BTreeMap<NodeId, CompiledFlowNode>,
    pub execution_order: Vec<NodeId>,
    pub context_plan: ContextPlan,
    pub action_definition_versions: BTreeMap<ActionKind, u32>,
    pub required_capabilities: BTreeSet<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FlowSummary {
    pub id: FlowId,
    pub name: String,
    pub latest_revision: u64,
    pub archived: bool,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FlowRevisionRecord {
    pub document: FlowDocumentV2,
    pub compiled_plan: CompiledFlowPlanV2,
    pub plan_hash: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, thiserror::Error)]
#[serde(rename_all = "camelCase")]
#[error("expected revision {expected}, actual revision {actual}")]
pub struct RevisionConflict {
    pub expected: u64,
    pub actual: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CompiledFlowNode {
    pub id: NodeId,
    pub kind: ActionKind,
    pub config: CompiledActionConfig,
    pub postcondition: Option<EvidenceSpec>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ImageCoordinateTarget {
    pub x: f64,
    pub y: f64,
    pub image_width: u32,
    pub image_height: u32,
    pub orientation: crate::ScreenOrientation,
    pub profile_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum CompiledActionConfig {
    Empty,
    LaunchApp {
        bundle_id: String,
    },
    TerminateApp {
        bundle_id: String,
    },
    Wait {
        duration_ms: u64,
    },
    Tap {
        target: CompiledTapTarget,
    },
    Swipe {
        from: ImageCoordinateTarget,
        to: ImageCoordinateTarget,
        duration_ms: u64,
    },
    TypeText {
        text: String,
        read_back_locator: QualifiedElementLocator,
    },
    Screenshot {
        label: String,
        format: String,
    },
    AssertVisible {
        accessibility_id: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(
    tag = "mode",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum CompiledTapTarget {
    Point { target: ImageCoordinateTarget },
    AccessibilityId { value: String },
}

pub fn qualified_geometry_profile_id(
    snapshot: &crate::DeviceCapabilitySnapshot,
) -> Result<String, &'static str> {
    let geometry = snapshot.geometry.as_ref().ok_or("geometry is absent")?;
    let dimensions_are_valid = geometry.pixel_width > 0
        && geometry.pixel_height > 0
        && geometry.logical_width.is_finite()
        && geometry.logical_height.is_finite()
        && geometry.scale_x.is_finite()
        && geometry.scale_y.is_finite()
        && geometry.logical_width > 0.0
        && geometry.logical_height > 0.0
        && geometry.scale_x > 0.0
        && geometry.scale_y > 0.0;
    if !dimensions_are_valid {
        return Err("geometry is invalid");
    }

    let material = serde_json::json!({
        "selectedArtifactSha256": &snapshot.selected_artifact_sha256,
        "installedAgent": &snapshot.installed_agent,
        "agentVersion": &snapshot.agent_version,
        "protocolVersion": snapshot.protocol_version,
        "driverAdapterVersion": &snapshot.driver_adapter_version,
        "transport": snapshot.transport,
        "productType": &snapshot.product_type,
        "iosVersion": &snapshot.ios_version,
        "targetApp": &snapshot.target_app,
        "geometry": geometry,
    });
    let bytes = serde_json::to_vec(&canonicalize_json(material))
        .map_err(|_| "geometry serialization failed")?;
    Ok(hex_sha256(&bytes))
}

fn canonicalize_json(value: Value) -> Value {
    match value {
        Value::Object(object) => {
            let sorted: BTreeMap<_, _> = object
                .into_iter()
                .map(|(key, value)| (key, canonicalize_json(value)))
                .collect();
            Value::Object(sorted.into_iter().collect())
        }
        Value::Array(values) => Value::Array(values.into_iter().map(canonicalize_json).collect()),
        scalar => scalar,
    }
}

pub fn canonical_compiled_plan_json(
    plan: &CompiledFlowPlanV2,
) -> Result<String, serde_json::Error> {
    let value = serde_json::to_value(plan)?;
    serde_json::to_string(&canonicalize_json(value))
}

pub fn canonical_execution_hash_material_json(
    plan: &CompiledFlowPlanV2,
) -> Result<String, serde_json::Error> {
    let mut value = serde_json::to_value(plan)?;
    value
        .as_object_mut()
        .expect("compiled plan is an object")
        .remove("revision");
    serde_json::to_string(&canonicalize_json(value))
}

pub fn compiled_plan_sha256(plan: &CompiledFlowPlanV2) -> Result<String, serde_json::Error> {
    let canonical = canonical_execution_hash_material_json(plan)?;
    Ok(hex_sha256(canonical.as_bytes()))
}

pub fn validate_artifact_label(label: &str, format: &str) -> Result<(), &'static str> {
    if label.trim() != label || label.is_empty() || label.chars().count() > 96 {
        return Err("ArtifactLabelLength");
    }
    if label
        .chars()
        .any(|character| character.is_control() || character == '/' || character == '\\')
        || label.contains("..")
    {
        return Err("ArtifactLabelCharacters");
    }

    let extension = label
        .rsplit_once('.')
        .map(|(_, extension)| extension.to_ascii_lowercase());
    let windows_stem = label
        .split_once('.')
        .map_or(label, |(windows_stem, _)| windows_stem);
    let upper = windows_stem.to_ascii_uppercase();
    let reserved = ["CON", "PRN", "AUX", "NUL"];
    if windows_stem.is_empty()
        || reserved.contains(&upper.as_str())
        || (upper.len() == 4
            && upper.is_ascii()
            && matches!(&upper[..3], "COM" | "LPT")
            && matches!(
                &upper[3..],
                "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9"
            ))
    {
        return Err("ArtifactLabelReserved");
    }

    match (format, extension.as_deref()) {
        ("jpeg", None | Some("jpg") | Some("jpeg")) | ("png", None | Some("png")) => Ok(()),
        ("jpeg" | "png", Some(_)) => Err("ArtifactLabelExtension"),
        _ => Err("ArtifactFormat"),
    }
}

fn hex_sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
