# Riviu Flow V2 Foundation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build Flow V2's typed graph model, deterministic compiler, legacy import diagnostics, database migration runner, and immutable revision repository.

**Architecture:** `riviu-core` owns serializable authoring/compiled types and persistence. `riviu-script-engine` depends on core and performs pure parse, validation, import, and compilation; core never depends back on script-engine.

**Tech Stack:** Rust 2021, serde/serde_json, uuid, rusqlite, sha2, chrono, cargo test.

---

### Task 1: Add Versioned Flow Model And Action Catalog

**Files:**
- Create: `crates/core/src/flow/mod.rs`
- Create: `crates/core/src/flow/model.rs`
- Create: `crates/core/src/flow/catalog.rs`
- Modify: `crates/core/src/lib.rs`
- Modify: `crates/core/Cargo.toml`

- [ ] **Step 1: Write serialization and catalog tests**

Create `crates/core/src/flow/mod.rs` with the module declarations and these tests:

```rust
pub mod catalog;
pub mod model;

pub use catalog::*;
pub use model::*;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn document_uses_camel_case_and_exact_schema_two() {
        let document = FlowDocumentV2::empty("Fixture");
        let json = serde_json::to_value(document).expect("serialize flow");
        assert_eq!(json["schemaVersion"], 2);
        assert!(json.get("entryNodeId").is_some());
    }

    #[test]
    fn catalog_never_exposes_raw_transport_actions_or_terminate() {
        let catalog = release_one_catalog();
        assert!(catalog.iter().all(|entry| !matches!(
            entry.kind,
            ActionKind::RawHttp | ActionKind::RawWda | ActionKind::Shell
        )));
        assert!(catalog.iter().all(|entry| entry.kind != ActionKind::TerminateApp));
    }

    #[test]
    fn every_side_effect_declares_evidence_and_reconciliation() {
        for action in release_one_catalog() {
            if action.side_effect_class != SideEffectClass::None {
                assert_ne!(action.evidence_requirement, EvidenceRequirement::None);
                assert_ne!(action.reconciliation_policy, ReconciliationPolicy::None);
            }
        }
    }

    #[test]
    fn evidence_variant_fields_are_camel_case() {
        let value = serde_json::to_value(EvidenceSpec::ActiveAppEquals {
            bundle_id: "com.apple.Preferences".to_string(),
        }).expect("evidence JSON");
        assert_eq!(value["bundleId"], "com.apple.Preferences");
        assert!(value.get("bundle_id").is_none());
    }
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run:

```powershell
cargo test -p riviu-core flow::tests -- --nocapture
```

Expected: FAIL because `flow`, `FlowDocumentV2`, and the catalog types do not exist.

- [ ] **Step 3: Implement the model**

Add `sha2 = { workspace = true }` to `crates/core/Cargo.toml`. Create
`crates/core/src/flow/model.rs` with these public types and derives:

```rust
use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::Value;
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
        Self { x: 0.0, y: 0.0, zoom: 1.0 }
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
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum EvidenceSpec {
    ActiveAppEquals { bundle_id: String },
    ProcessAbsent { bundle_id: String },
    FrameDigestChanged { minimum_distance: u32 },
    FrameRegionChanged { x: u32, y: u32, width: u32, height: u32, minimum_distance: u32 },
    QualifiedFramePredicate { detector_id: String },
    AccessibilityVisible { accessibility_id: String },
    TextReadBackEquals { locator: QualifiedElementLocator, value: String },
    ArtifactDecodedAndHashed,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ElementLocatorStrategy { AccessibilityId, ClassName }

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct QualifiedElementLocator {
    pub strategy: ElementLocatorStrategy,
    pub value: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ResourceClass { PureDesktop, Bridge, UiSession, UiWithStream }

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum SideEffectClass { None, IdempotentSet, AmbiguousUi, ArtifactWrite }

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum EvidenceRequirement { None, ActiveApp, Process, Frame, TextOrQualifiedFrame, Artifact }

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
pub enum ReconciliationPolicy { None, ReadActiveApp, ReadProcess, ReadFrame, ReadText, ReadArtifact }

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum RetryPolicy { Never, BeforeDispatchOnly, IdempotentAfterRead }

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
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum CompiledActionConfig {
    Empty,
    LaunchApp { bundle_id: String },
    TerminateApp { bundle_id: String },
    Wait { duration_ms: u64 },
    Tap { target: CompiledTapTarget },
    Swipe {
        from: ImageCoordinateTarget,
        to: ImageCoordinateTarget,
        duration_ms: u64,
    },
    TypeText {
        text: String,
        read_back_locator: QualifiedElementLocator,
    },
    Screenshot { label: String, format: String },
    AssertVisible { accessibility_id: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "mode", rename_all = "camelCase", rename_all_fields = "camelCase")]
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
    use sha2::{Digest, Sha256};
    Ok(Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

fn canonicalize_json(value: serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Object(object) => {
            let sorted: std::collections::BTreeMap<_, _> = object
                .into_iter()
                .map(|(key, value)| (key, canonicalize_json(value)))
                .collect();
            serde_json::Value::Object(sorted.into_iter().collect())
        }
        serde_json::Value::Array(values) => serde_json::Value::Array(
            values.into_iter().map(canonicalize_json).collect()
        ),
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
    value.as_object_mut().expect("compiled plan is an object").remove("revision");
    serde_json::to_string(&canonicalize_json(value))
}

pub fn compiled_plan_sha256(plan: &CompiledFlowPlanV2) -> Result<String, serde_json::Error> {
    let canonical = canonical_execution_hash_material_json(plan)?;
    use sha2::{Digest, Sha256};
    Ok(Sha256::digest(canonical.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

pub fn validate_artifact_label(label: &str, format: &str) -> Result<(), &'static str> {
    if label.trim() != label || label.is_empty() || label.chars().count() > 96 {
        return Err("ArtifactLabelLength");
    }
    if label.chars().any(|ch| ch.is_control() || ch == '/' || ch == '\\')
        || label.contains("..")
    {
        return Err("ArtifactLabelCharacters");
    }
    let (stem, extension) = match label.rsplit_once('.') {
        Some((stem, extension)) => (stem, Some(extension.to_ascii_lowercase())),
        None => (label, None),
    };
    let reserved = ["CON", "PRN", "AUX", "NUL"];
    let upper = stem.to_ascii_uppercase();
    if stem.is_empty()
        || reserved.contains(&upper.as_str())
        || (upper.len() == 4
            && upper.is_ascii()
            && matches!(&upper[..3], "COM" | "LPT")
            && matches!(&upper[3..], "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9"))
    {
        return Err("ArtifactLabelReserved");
    }
    match (format, extension.as_deref()) {
        ("jpeg", None | Some("jpg") | Some("jpeg")) | ("png", None | Some("png")) => Ok(()),
        ("jpeg" | "png", Some(_)) => Err("ArtifactLabelExtension"),
        _ => Err("ArtifactFormat"),
    }
}
```

The stored canonical compiled JSON still includes `revision`; only the execution
hash material excludes it. `flow_id`, typed node order, action-definition versions,
context plan, and required capabilities remain in the hash. Add golden tests proving
that the same execution at revisions 1 and 2 has the same hash, while changed typed
config, action version, context, capability, or flow ID changes it.

Add a test with a complete `DeviceCapabilitySnapshot`: the same snapshot hashes
identically twice; changing target app build, iOS version, orientation, or one
pixel dimension changes the hash; absent/non-finite geometry returns an error.

- [ ] **Step 4: Implement the release-1 catalog**

Create `crates/core/src/flow/catalog.rs`. Define `ActionDefinition` with `kind`,
`schema_version`, `label`, backend-owned `disabled_reason`, `category`,
`config_schema`, ports, required capabilities, resource/effect/evidence/
reconciliation/retry fields, and timeout. Return definitions only for Start, End,
Launch App, Wait, Tap, Swipe, Type Text, Screenshot, Home, and Assert Visible in F0.
Every returned F0 action has `disabled_reason: None`. Use `serde_json::json!` schemas
with `additionalProperties: false`; omit Terminate App and all raw kinds from the F0
catalog. F1 adds Terminate only after its bounded DVT path and process-absence proof
pass their tests; future unavailable entries may be returned with a non-empty reason,
and the compiler rejects selecting any definition whose reason is set.

Use these exact public catalog types:

```rust
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::model::*;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ActionCategory { Control, App, Input, Timing, Evidence }

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PortDefinition {
    pub name: String,
    pub value_type: String,
    pub required: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ActionDefinition {
    pub kind: ActionKind,
    pub schema_version: u32,
    pub label: String,
    pub disabled_reason: Option<String>,
    pub category: ActionCategory,
    pub config_schema: Value,
    pub input_ports: Vec<PortDefinition>,
    pub output_ports: Vec<PortDefinition>,
    pub required_capabilities: Vec<String>,
    pub resource_class: ResourceClass,
    pub side_effect_class: SideEffectClass,
    pub evidence_requirement: EvidenceRequirement,
    pub allowed_evidence: Vec<EvidenceKind>,
    pub qualified_detector_ids: Vec<String>,
    pub reconciliation_policy: ReconciliationPolicy,
    pub default_timeout_ms: u32,
    pub retry_policy: RetryPolicy,
}
```

Use these exact config fields in both the catalog and compiler. The helper keeps
the coordinate schema byte-identical for Tap and Swipe:

```rust
fn coordinate_schema() -> Value {
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["x", "y", "imageWidth", "imageHeight", "orientation", "profileId"],
        "properties": {
            "x": { "type": "number" },
            "y": { "type": "number" },
            "imageWidth": { "type": "integer", "minimum": 1 },
            "imageHeight": { "type": "integer", "minimum": 1 },
            "orientation": {
                "type": "string",
                "enum": ["portrait", "portraitUpsideDown", "landscapeLeft", "landscapeRight"]
            },
            "profileId": { "type": "string", "pattern": "^[0-9a-f]{64}$" }
        }
    })
}

fn read_back_locator_schema() -> Value {
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["strategy", "value"],
        "properties": {
            "strategy": { "type": "string", "enum": ["accessibilityId", "className"] },
            "value": { "type": "string", "minLength": 1, "maxLength": 512 }
        }
    })
}

pub fn config_schema(kind: ActionKind) -> Value {
    match kind {
        ActionKind::Start | ActionKind::End | ActionKind::Home => serde_json::json!({
            "type": "object", "additionalProperties": false, "properties": {}
        }),
        ActionKind::LaunchApp => serde_json::json!({
            "type": "object", "additionalProperties": false,
            "required": ["bundleId"],
            "properties": {
                "bundleId": { "type": "string", "minLength": 1, "maxLength": 255 }
            }
        }),
        ActionKind::TerminateApp => serde_json::json!({
            "type": "object", "additionalProperties": false,
            "required": ["bundleId"],
            "properties": {
                "bundleId": { "type": "string", "minLength": 1, "maxLength": 255 }
            }
        }),
        ActionKind::Wait => serde_json::json!({
            "type": "object", "additionalProperties": false,
            "required": ["durationMs"],
            "properties": {
                "durationMs": { "type": "integer", "minimum": 1, "maximum": 60000 }
            }
        }),
        ActionKind::Tap => serde_json::json!({
            "type": "object", "additionalProperties": false,
            "properties": {
                "point": coordinate_schema(),
                "accessibilityId": { "type": "string", "minLength": 1, "maxLength": 512 }
            },
            "oneOf": [
                { "required": ["point"], "not": { "required": ["accessibilityId"] } },
                { "required": ["accessibilityId"], "not": { "required": ["point"] } }
            ]
        }),
        ActionKind::Swipe => serde_json::json!({
            "type": "object", "additionalProperties": false,
            "required": ["from", "to", "durationMs"],
            "properties": {
                "from": coordinate_schema(),
                "to": coordinate_schema(),
                "durationMs": { "type": "integer", "minimum": 1, "maximum": 5000 }
            }
        }),
        ActionKind::TypeText => serde_json::json!({
            "type": "object", "additionalProperties": false,
            "required": ["text", "readBackLocator"],
            "properties": {
                "text": { "type": "string", "minLength": 1, "maxLength": 4096 },
                "readBackLocator": read_back_locator_schema()
            }
        }),
        ActionKind::Screenshot => serde_json::json!({
            "type": "object", "additionalProperties": false,
            "required": ["label", "format"],
            "properties": {
                "label": { "type": "string", "minLength": 1, "maxLength": 96 },
                "format": { "type": "string", "enum": ["jpeg"] }
            }
        }),
        ActionKind::AssertVisible => serde_json::json!({
            "type": "object", "additionalProperties": false,
            "required": ["accessibilityId"],
            "properties": {
                "accessibilityId": { "type": "string", "minLength": 1, "maxLength": 512 }
            }
        }),
        ActionKind::RawHttp | ActionKind::RawWda | ActionKind::Shell =>
            Value::Null,
    }
}

pub fn required_capabilities(kind: ActionKind) -> Vec<String> {
    let ids: &[&str] = match kind {
        ActionKind::Start | ActionKind::End | ActionKind::Wait => &[],
        ActionKind::LaunchApp => &["app.launch"],
        ActionKind::TerminateApp => &["app.terminate"],
        ActionKind::Tap => &["ui.tap", "stream"],
        ActionKind::Swipe => &["ui.swipe", "stream"],
        ActionKind::TypeText => &["ui.text", "stream", "accessibility.readText"],
        ActionKind::Screenshot => &["stream"],
        ActionKind::Home => &["ui.home"],
        ActionKind::AssertVisible => &["accessibility.visible"],
        ActionKind::RawHttp | ActionKind::RawWda | ActionKind::Shell => &[],
    };
    ids.iter().map(|id| (*id).to_string()).collect()
}
```

Set every input/output port to the single control port named `flow` except Start
(output only) and End (input only). Set schema/action version to 1. Use default
timeouts Start/End 1,000 ms; Wait 60,000 ms; Launch/Home/Terminate 10,000 ms;
Tap/Swipe 5,000 ms; Type Text 10,000 ms; Screenshot 5,000 ms; Assert Visible
4,000 ms. The Terminate definition is constructed only by the F1-enabled catalog.

Set `allowed_evidence` exactly to: empty for Start/End/Wait/Assert Visible;
`ActiveAppEquals` for Launch/Home; `ProcessAbsent` for Terminate;
`FrameRegionChanged` for Tap;
`FrameDigestChanged` for Swipe;
`TextReadBackEquals` for Type Text; and `ArtifactDecodedAndHashed` for Screenshot.
Set every release-1 `qualified_detector_ids` list empty. Keep
`QualifiedFramePredicate` in the versioned model for later qualified catalogs, but
the release-1 compiler returns `EvidenceNotEnabled` for it and for any variant not
listed on that action.

The side-effect contracts must be exactly:

```rust
pub fn contracts(kind: ActionKind) -> (ResourceClass, SideEffectClass, EvidenceRequirement, ReconciliationPolicy, RetryPolicy) {
    match kind {
        ActionKind::Start | ActionKind::End =>
            (ResourceClass::PureDesktop, SideEffectClass::None, EvidenceRequirement::None, ReconciliationPolicy::None, RetryPolicy::BeforeDispatchOnly),
        ActionKind::Wait =>
            (ResourceClass::Bridge, SideEffectClass::None, EvidenceRequirement::None, ReconciliationPolicy::None, RetryPolicy::BeforeDispatchOnly),
        ActionKind::AssertVisible =>
            (ResourceClass::UiSession, SideEffectClass::None, EvidenceRequirement::None, ReconciliationPolicy::None, RetryPolicy::BeforeDispatchOnly),
        ActionKind::LaunchApp | ActionKind::Home =>
            (ResourceClass::UiSession, SideEffectClass::IdempotentSet, EvidenceRequirement::ActiveApp, ReconciliationPolicy::ReadActiveApp, RetryPolicy::IdempotentAfterRead),
        ActionKind::Tap | ActionKind::Swipe =>
            (ResourceClass::UiWithStream, SideEffectClass::AmbiguousUi, EvidenceRequirement::Frame, ReconciliationPolicy::ReadFrame, RetryPolicy::BeforeDispatchOnly),
        ActionKind::TypeText =>
            (ResourceClass::UiWithStream, SideEffectClass::AmbiguousUi, EvidenceRequirement::TextOrQualifiedFrame, ReconciliationPolicy::ReadText, RetryPolicy::BeforeDispatchOnly),
        ActionKind::Screenshot =>
            (ResourceClass::UiWithStream, SideEffectClass::ArtifactWrite, EvidenceRequirement::Artifact, ReconciliationPolicy::ReadArtifact, RetryPolicy::BeforeDispatchOnly),
        ActionKind::TerminateApp =>
            (ResourceClass::Bridge, SideEffectClass::IdempotentSet, EvidenceRequirement::Process, ReconciliationPolicy::ReadProcess, RetryPolicy::IdempotentAfterRead),
        ActionKind::RawHttp | ActionKind::RawWda | ActionKind::Shell =>
            (ResourceClass::Bridge, SideEffectClass::AmbiguousUi, EvidenceRequirement::None, ReconciliationPolicy::None, RetryPolicy::Never),
    }
}
```

- [ ] **Step 5: Export Flow types and run the tests**

Add `pub mod flow;` and `pub use flow::*;` to `crates/core/src/lib.rs`.

Run:

```powershell
cargo fmt --all
cargo test -p riviu-core flow::tests -- --nocapture
```

Expected: 4 passed, 0 failed.

- [ ] **Step 6: Commit Task 1**

```powershell
git add crates/core/Cargo.toml crates/core/src/flow crates/core/src/lib.rs
git commit -m "feat(flow): add versioned graph model and catalog"
```

### Task 2: Build The Deterministic Compiler

**Files:**
- Create: `crates/script-engine/src/flow.rs`
- Modify: `crates/script-engine/src/lib.rs`
- Modify: `crates/script-engine/Cargo.toml`

- [ ] **Step 1: Add failing compiler tests**

In `crates/script-engine/src/flow.rs`, add tests for: valid Start -> Launch -> End; duplicate node IDs; missing End; cycle; disconnected node; invalid port; non-finite canvas/config coordinates; Wait 60,001; Tap with point and selector; XPath/predicate; missing Tap evidence; generic Tap with whole-frame digest evidence; and deterministic hashes after node/edge/layout reordering.

Use this fixture builder as the common test input:

```rust
fn linear_document(nodes: Vec<FlowNode>) -> FlowDocumentV2 {
    let entry_node_id = nodes[0].id;
    let edges = nodes.windows(2).map(|pair| FlowEdge::flow(pair[0].id, pair[1].id)).collect();
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
```

The key red tests are:

```rust
#[test]
fn tap_without_qualified_postcondition_is_rejected() {
    let start = FlowNode::new(ActionKind::Start, serde_json::json!({}));
    let mut launch = FlowNode::new(ActionKind::LaunchApp, serde_json::json!({
        "bundleId": "com.apple.Preferences"
    }));
    launch.postcondition = Some(EvidenceSpec::ActiveAppEquals {
        bundle_id: "com.apple.Preferences".into(),
    });
    let tap = FlowNode::new(ActionKind::Tap, serde_json::json!({
        "point": {
            "x": 100.0, "y": 200.0,
            "imageWidth": 375, "imageHeight": 667,
            "orientation": "portrait", "profileId": ("11".repeat(32))
        }
    }));
    let end = FlowNode::new(ActionKind::End, serde_json::json!({}));
    let errors = compile_flow(
        &linear_document(vec![start, launch, tap, end]),
        &release_one_catalog(),
    )
        .expect_err("tap must require evidence");
    assert!(errors.iter().any(|error| error.code == "EvidenceRequired"));
}

#[test]
fn layout_and_input_order_do_not_change_plan_hash() {
    let start = FlowNode::new(ActionKind::Start, serde_json::json!({}));
    let mut launch = FlowNode::new(ActionKind::LaunchApp, serde_json::json!({
        "bundleId": "com.apple.Preferences"
    }));
    launch.postcondition = Some(EvidenceSpec::ActiveAppEquals {
        bundle_id: "com.apple.Preferences".into(),
    });
    let end = FlowNode::new(ActionKind::End, serde_json::json!({}));
    let first = linear_document(vec![start, launch, end]);
    let mut second = first.clone();
    second.nodes.reverse();
    second.edges.reverse();
    second.viewport = FlowViewport { x: 900.0, y: 400.0, zoom: 1.5 };
    for node in &mut second.nodes {
        node.position.x += 73.0;
    }
    let first = compile_flow(&first, &release_one_catalog()).expect("first plan");
    let second = compile_flow(&second, &release_one_catalog()).expect("second plan");
    assert_eq!(first.sha256, second.sha256);
}
```

- [ ] **Step 2: Run tests red**

```powershell
cargo test -p riviu-script-engine flow -- --nocapture
```

Expected: FAIL because `compile_flow` and `FlowCompileError` do not exist.

- [ ] **Step 3: Add compiler dependencies and error types**

Add `sha2 = { workspace = true }` to `crates/script-engine/Cargo.toml`.

Define these public results in `flow.rs`:

```rust
#[derive(Debug, Clone, serde::Serialize, PartialEq, Eq, thiserror::Error)]
#[serde(rename_all = "camelCase")]
#[error("{message}")]
pub struct FlowCompileError {
    pub code: String,
    pub message: String,
    pub node_id: Option<NodeId>,
    pub field: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CompiledRevision {
    pub plan: CompiledFlowPlanV2,
    pub canonical_json: String,
    pub sha256: String,
}
```

- [ ] **Step 4: Implement validation and linear ordering**

Implement `compile_flow(document: &FlowDocumentV2, catalog: &[ActionDefinition]) -> Result<CompiledRevision, Vec<FlowCompileError>>`. Validate in the design order, collect node-scoped errors, and return no plan when any error exists. Use `BTreeMap`/`BTreeSet` for identity and ordering. Require exactly one Start and End, one incoming/outgoing flow edge for executable nodes, Start with no incoming, End with no outgoing, and a walk from `entry_node_id` that visits every node exactly once.

Decode every config from a JSON object into a private `Deserialize` struct per
action, including an empty `#[serde(deny_unknown_fields)]` struct for Start/End/Home.
The structs below reject additional properties; explicit validators then mirror
every advertised schema bound exactly: bundle IDs contain 1..=255 Unicode scalar
values; accessibility IDs and locator values 1..=512; Type Text 1..=4096; Wait
duration 1..=60,000 ms; Swipe duration 1..=5,000 ms; image dimensions are positive;
coordinates are finite; orientation is one of the four enum variants; and profile
ID is exactly 64 lowercase ASCII hex characters. Validate screenshot label/format
by first requiring release-1 `format == "jpeg"`, then calling
`validate_artifact_label`. Tap must contain exactly one of point or
accessibility ID. A non-object config, a JSON number that cannot deserialize into
the exact integer type, an unknown field, or a non-finite programmatically-created
number is a typed node error. Reject raw kinds as `FeatureNotEnabled`. F0 also
rejects Terminate; F1 Task 4 changes that feature gate only after installing its
catalog definition and verifier.

After validation, convert every authoring `Value` into `CompiledActionConfig` and
store only that typed enum in `CompiledFlowNode`. Start/End/Home use `Empty`; a Tap
point becomes `CompiledTapTarget::Point`, and a selector becomes
`CompiledTapTarget::AccessibilityId`. Runtime never deserializes authoring JSON.

Use these names so runtime and frontend projections do not invent alternate fields:

```rust
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct LaunchAppConfig { bundle_id: String }

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct TerminateAppConfig { bundle_id: String }

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct WaitConfig { duration_ms: u64 }

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct TapConfig {
    point: Option<ImageCoordinateTarget>,
    accessibility_id: Option<String>,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SwipeConfig {
    from: ImageCoordinateTarget,
    to: ImageCoordinateTarget,
    duration_ms: u64,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct TypeTextConfig { text: String, read_back_locator: QualifiedElementLocator }

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ScreenshotConfig { label: String, format: String }

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AssertVisibleConfig { accessibility_id: String }
```

Add table-driven boundary tests for every length/duration at minimum, maximum,
minimum-minus-one, and maximum-plus-one; for 63/64/65-character profile IDs plus
uppercase/non-hex cases; for every config's unknown field; for non-object configs;
and for both/neither Tap target modes. These tests are the executable proof that the
manual backend validator and the published schema accept the same boundary values.

- [ ] **Step 5: Implement evidence and context planning**

Validate the postcondition against the catalog's exact `allowed_evidence` list. In
particular, reject whole-frame digest, accessibility lookup, and qualified predicates
for Tap; accept only `FrameRegionChanged`. Accept `FrameDigestChanged` for
Swipe and require `TextReadBackEquals` for Type Text in release 1.

For Launch App, require `ActiveAppEquals.bundle_id == config.bundleId`; once F1
enables Terminate, require `ProcessAbsent.bundle_id == config.bundleId`; for Home,
require `ActiveAppEquals.bundle_id == "com.apple.springboard"`; for Type Text with
`TextReadBackEquals`, require both its locator and value to equal the two Type Text
config fields; and for Screenshot require `ArtifactDecodedAndHashed`. Locator
strategy is exhaustively `AccessibilityId` or `ClassName`; reject XPath, predicate,
class chain, empty values, and unknown strategies.

Fold the action contracts into this exact monotonic plan:

```rust
fn context_plan(nodes: impl Iterator<Item = ActionKind>) -> ContextPlan {
    let mut plan = ContextPlan {
        requires_exclusive: false,
        requires_ui_session: false,
        requires_stream: false,
        requires_fresh_text_session: false,
        initial_bundle_id: None,
    };
    for kind in nodes {
        let (resource, _, _, _, _) = riviu_core::flow::catalog::contracts(kind);
        plan.requires_exclusive |= resource != ResourceClass::PureDesktop;
        plan.requires_ui_session |= matches!(resource, ResourceClass::UiSession | ResourceClass::UiWithStream);
        plan.requires_stream |= resource == ResourceClass::UiWithStream;
        plan.requires_fresh_text_session |= kind == ActionKind::TypeText;
    }
    plan
}
```

After folding resources, set `initial_bundle_id` from the first executable Launch
App. Every plan with `requires_ui_session=true` must have Launch App as its first
executable node; otherwise return `UiSessionTargetRequired`. This covers Home-only,
Assert Visible, Tap, Swipe, Type Text, and Screenshot plans rather than only stream
plans. The first Launch App is the only node consumed by the initial session upgrade
and must not be dispatched again by the generic loop. A pure desktop/bridge plan such
as Wait or Terminate does not need Launch; Terminate carries its own exact bundle ID.
Later Launch nodes may foreground another app only after the context is already
upgraded.

- [ ] **Step 6: Use shared canonical serialization and hashing**

Call F0 Task 1's `canonical_compiled_plan_json` and `compiled_plan_sha256`; do not
create a second canonicalizer in script-engine. The canonical stored JSON contains
the assigned revision. The hash helper canonicalizes the separate execution
material with only the top-level revision omitted. Layout and viewport are absent
from `CompiledFlowPlanV2`, object keys are recursively sorted, and serde rejects
non-finite numbers. Return those exact outputs in `CompiledRevision`.

```rust
let canonical_json = canonical_compiled_plan_json(&plan).map_err(|error|
    vec![FlowCompileError::document("CanonicalSerialization", error.to_string())]
)?;
let sha256 = compiled_plan_sha256(&plan).map_err(|error|
    vec![FlowCompileError::document("CanonicalSerialization", error.to_string())]
)?;
Ok(CompiledRevision { plan, canonical_json, sha256 })
```

- [ ] **Step 7: Export and run tests green**

Add `pub mod flow;` and `pub use flow::*;` to `crates/script-engine/src/lib.rs`.

```powershell
cargo fmt --all
cargo test -p riviu-script-engine flow -- --nocapture
```

Expected: all compiler tests pass; the two layout/order variants have identical SHA-256.

- [ ] **Step 8: Commit Task 2**

```powershell
git add crates/script-engine crates/core/src/flow
git commit -m "feat(flow): compile linear graphs into deterministic plans"
```

### Task 3: Add Legacy V1 Import Diagnostics

**Files:**
- Modify: `crates/script-engine/src/flow.rs`
- Test: `crates/script-engine/src/flow.rs`

- [ ] **Step 1: Write failing import tests**

Test a supported Launch -> Wait -> Screenshot -> Home script and assert a
straight-line graph with automatic evidence. Add separate tests asserting exact
diagnostics for Wait >60s, Terminate App, XPath, predicate, Tap point+selector, Tap
without postcondition, Swipe/Tap coordinates without a qualified geometry profile,
Type Text without read-back target, and non-finite coordinates.

Use this exact supported-case assertion:

```rust
#[test]
fn legacy_import_accepts_only_semantics_preserving_steps() {
    let script = AutomationScript {
        version: 1,
        name: "Fixture".into(),
        steps: vec![
            ScriptAction::LaunchApp { bundle_id: "com.apple.Preferences".into() },
            ScriptAction::Wait { milliseconds: 20 },
            ScriptAction::Screenshot { name: "settings".into() },
            ScriptAction::Home,
        ],
    };
    let imported = import_legacy_v1(&script);
    assert!(imported.diagnostics.is_empty());
    let document = imported.document.expect("supported document");
    assert_eq!(document.nodes.len(), 6);
    assert_eq!(document.edges.len(), 5);
}
```

- [ ] **Step 2: Run the import tests red**

```powershell
cargo test -p riviu-script-engine legacy_import -- --nocapture
```

Expected: FAIL because `import_legacy_v1` is absent.

- [ ] **Step 3: Implement result and diagnostic types**

```rust
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
```

- [ ] **Step 4: Implement conservative import**

Map Launch to `ActiveAppEquals`, Screenshot to `ArtifactDecodedAndHashed`, and Home
to `ActiveAppEquals { bundle_id: "com.apple.springboard" }`. Preserve Wait and
qualified Assert Visible. Emit `GeometryRequired` for legacy Tap/Swipe coordinates
and `EvidenceRequired` for Tap and Type Text rather than inventing proof. Return
`document: None` when any diagnostic exists; never mutate or save the source v1
JSON.

Use one exhaustive match; supported Assert Visible requires exactly one non-empty
accessibility ID:

```rust
let mut push_node = |kind, config, postcondition| {
    let mut node = FlowNode::new(kind, config);
    node.position = CanvasPoint { x: (nodes.len() as f64) * 220.0, y: 80.0 };
    node.postcondition = postcondition;
    nodes.push(node);
};

match step {
    ScriptAction::LaunchApp { bundle_id } => push_node(
        ActionKind::LaunchApp,
        serde_json::json!({ "bundleId": bundle_id }),
        Some(EvidenceSpec::ActiveAppEquals { bundle_id: bundle_id.clone() }),
    ),
    ScriptAction::Wait { milliseconds } if *milliseconds <= 60_000 => push_node(
        ActionKind::Wait,
        serde_json::json!({ "durationMs": milliseconds }),
        None,
    ),
    ScriptAction::Screenshot { name } if validate_artifact_label(name, "jpeg").is_ok() => push_node(
        ActionKind::Screenshot,
        serde_json::json!({ "label": name, "format": "jpeg" }),
        Some(EvidenceSpec::ArtifactDecodedAndHashed),
    ),
    ScriptAction::Home => push_node(
        ActionKind::Home,
        serde_json::json!({}),
        Some(EvidenceSpec::ActiveAppEquals {
            bundle_id: "com.apple.springboard".to_string(),
        }),
    ),
    ScriptAction::AssertVisible { selector }
        if selector.accessibility_id.as_deref().is_some_and(|value| !value.is_empty())
            && selector.xpath.is_none()
            && selector.predicate.is_none() => push_node(
                ActionKind::AssertVisible,
                serde_json::json!({
                    "accessibilityId": selector.accessibility_id.as_deref().unwrap_or_default()
                }),
                None,
            ),
    unsupported => diagnostics.push(diagnostic_for_legacy_step(index, unsupported)),
}
```

Use Task 1's `validate_artifact_label`, which is also reused by
`FlowArtifactStore`. `diagnostic_for_legacy_step` maps Wait overflow to
`WaitOutOfRange`, Terminate to
`FeatureNotEnabled`, XPath/predicate/conflicting selectors to `UnsupportedSelector`,
Tap/Swipe to `GeometryRequired`, Type Text to `EvidenceRequired`, and invalid
Screenshot labels to `ArtifactLabelInvalid`.

- [ ] **Step 5: Run tests and commit**

```powershell
cargo fmt --all
cargo test -p riviu-script-engine legacy_import -- --nocapture
git add crates/script-engine/src/flow.rs
git commit -m "feat(flow): import the semantics-preserving legacy subset"
```

Expected: import tests pass and unsupported shapes have stable node-scoped codes.

### Task 4: Introduce Transactional Schema Migrations

**Files:**
- Create: `crates/core/src/db/migrations.rs`
- Modify: `crates/core/src/db.rs`
- Test: `crates/core/src/db/migrations.rs`

- [ ] **Step 1: Write migration tests against a populated legacy database**

Create a fixture with the exact current tables and one row in `scripts`, `jobs`, `settings`, and `device_meta`. Test first open, second open, an unknown partial schema, and an injected migration failure. Assert legacy row bytes are unchanged and `schema_migrations` contains versions 1 and 2 only after success.

The populated-upgrade test must include this assertion sequence:

```rust
#[test]
fn populated_legacy_database_upgrades_once_without_rewriting_rows() {
    let path = temp_db_path("flow-migration");
    let expected_body =
        r#"{"version":1,"name":"fixture","steps":[{"action":"wait","milliseconds":1}]}"#;
    let mut connection = rusqlite::Connection::open(&path).expect("legacy db");
    apply_v1_schema(&mut connection).expect("legacy schema");
    connection.execute(
        "INSERT INTO scripts (id,name,body_json,updated_at) VALUES (?1,?2,?3,?4)",
        rusqlite::params!["script-1", "fixture", expected_body, "2026-07-30T00:00:00Z"],
    ).expect("legacy row");
    drop(connection);

    let database = Database::open(&path).expect("first migration");
    drop(database);
    let database = Database::open(&path).expect("idempotent reopen");
    let connection = database.conn().expect("inspect");
    let body: String = connection.query_row(
        "SELECT body_json FROM scripts WHERE id='script-1'", [], |row| row.get(0),
    ).expect("legacy body");
    let versions: Vec<i64> = connection.prepare(
        "SELECT version FROM schema_migrations ORDER BY version"
    ).expect("prepare").query_map([], |row| row.get(0)).expect("query")
        .collect::<Result<_, _>>().expect("versions");
    assert_eq!(body, expected_body);
    assert_eq!(versions, vec![1, 2]);
}
```

Add this ignored fixture writer in the same test module. Acceptance invokes it
only with an explicit path under the operating-system temp directory.

```rust
#[test]
#[ignore = "writes the explicit rollback fixture path"]
fn write_populated_legacy_fixture() {
    let path = std::path::PathBuf::from(
        std::env::var_os("RIVIU_LEGACY_FIXTURE_PATH")
            .expect("RIVIU_LEGACY_FIXTURE_PATH"),
    );
    assert!(!path.exists(), "fixture path already exists");
    let mut connection = rusqlite::Connection::open(&path).expect("fixture database");
    apply_v1_schema(&mut connection).expect("legacy schema");
    let transaction = connection.transaction().expect("fixture transaction");
    transaction.execute(
        "INSERT INTO scripts (id,name,body_json,updated_at) VALUES (?1,?2,?3,?4)",
        rusqlite::params![
            "script-1", "fixture",
            r#"{"version":1,"name":"fixture","steps":[{"action":"wait","milliseconds":1}]}"#,
            "2026-07-30T00:00:00Z"
        ],
    ).expect("script row");
    transaction.execute(
        "INSERT INTO jobs (
            id,script_name,udids_json,status,created_at,updated_at,steps_json,error
         ) VALUES (?1,?2,?3,?4,?5,?6,?7,NULL)",
        rusqlite::params![
            "job-1", "fixture", "[\"MOCK-IPHONE-01\"]", r#""succeeded""#,
            "2026-07-30T00:00:00Z", "2026-07-30T00:00:01Z", "[]"
        ],
    ).expect("job row");
    transaction.execute(
        "INSERT INTO settings (key,value) VALUES (?1,?2)",
        rusqlite::params!["fixture", "{\"enabled\":true}"],
    ).expect("settings row");
    transaction.execute(
        "INSERT INTO device_meta (udid,notes,tags_json) VALUES (?1,?2,?3)",
        rusqlite::params!["MOCK-IPHONE-01", "fixture", "[\"legacy\"]"],
    ).expect("device row");
    transaction.commit().expect("fixture commit");
}
```

Add a separate test opening a normal post-migration `Database` connection and
inserting a `flow_device_runs` row with a missing `run_id`; assert SQLite returns a
foreign-key violation. This proves enforcement is active outside the migration
connection.

- [ ] **Step 2: Run tests red**

```powershell
cargo test -p riviu-core db::migrations -- --nocapture
```

Expected: FAIL because the migration ledger and runner are absent.

- [ ] **Step 3: Add the migration runner**

In `db.rs`, declare `mod migrations;`, enable foreign keys on every connection, and
replace the batch in `Database::migrate` with:

```rust
fn conn(&self) -> anyhow::Result<rusqlite::Connection> {
    let connection = rusqlite::Connection::open(&self.path)?;
    connection.pragma_update(None, "foreign_keys", "ON")?;
    connection.busy_timeout(std::time::Duration::from_secs(5))?;
    Ok(connection)
}

fn migrate(&self) -> anyhow::Result<()> {
    let mut conn = self.conn()?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    migrations::run(&mut conn)
}
```

In `migrations.rs`, define an ordered slice of `(version, name, apply_fn)`. Open one transaction per migration, run SQL, insert the ledger row, and commit. On a database without the ledger, inspect `sqlite_master`: an empty DB receives baseline tables and ledger version 1; the exact known legacy table set receives only ledger version 1; every other non-empty shape returns `UnknownLegacySchema` before creating the ledger.

Create the ledger inside the same bootstrap transaction as baseline recognition:

```sql
CREATE TABLE schema_migrations (
  version INTEGER PRIMARY KEY CHECK (version >= 1),
  name TEXT NOT NULL UNIQUE,
  applied_at TEXT NOT NULL
);
```

For an empty database, create the ledger, apply migration 1, seed the guest, and
insert version 1 before commit. For an exact legacy database, create only the ledger
and insert version 1 before commit. Then run migration 2 in its own transaction.
On an existing ledger, require contiguous known versions and exact migration names;
fail closed on a gap, renamed entry, duplicate logical name, or version newer than
this binary before running any SQL.

- [ ] **Step 4: Add Flow schema migration 2**

Create all seven Flow tables from the design with `TEXT` UUIDs, `INTEGER` revisions/attempts, foreign keys, `CHECK` constraints for hashes/statuses, and indexes on document update, run update, device-run state, attempt state, artifacts by attempt, and events by aggregate/revision. The migration must use `CREATE TABLE`, not `IF NOT EXISTS`, because the ledger controls exactly-once execution.

Migration 2 executes this exact schema in one transaction:

```sql
CREATE TABLE flow_documents (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  latest_revision INTEGER NOT NULL CHECK (latest_revision >= 1),
  archived INTEGER NOT NULL DEFAULT 0 CHECK (archived IN (0, 1)),
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL
);

CREATE TABLE flow_revisions (
  flow_id TEXT NOT NULL,
  revision INTEGER NOT NULL CHECK (revision >= 1),
  authoring_json TEXT NOT NULL,
  compiled_json TEXT NOT NULL,
  plan_sha256 TEXT NOT NULL CHECK (
    length(plan_sha256) = 64 AND plan_sha256 NOT GLOB '*[^0-9a-f]*'
  ),
  created_at TEXT NOT NULL,
  PRIMARY KEY (flow_id, revision),
  FOREIGN KEY (flow_id) REFERENCES flow_documents(id) ON DELETE RESTRICT
);

CREATE TABLE flow_runs (
  id TEXT PRIMARY KEY,
  flow_id TEXT NOT NULL,
  flow_revision INTEGER NOT NULL,
  plan_sha256 TEXT NOT NULL CHECK (
    length(plan_sha256) = 64 AND plan_sha256 NOT GLOB '*[^0-9a-f]*'
  ),
  selection_json TEXT NOT NULL,
  state TEXT NOT NULL CHECK (
    state IN ('queued','running','succeeded','partial','failed','cancelled')
  ),
  event_revision INTEGER NOT NULL DEFAULT 0 CHECK (event_revision >= 0),
  error_json TEXT,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  FOREIGN KEY (flow_id, flow_revision)
    REFERENCES flow_revisions(flow_id, revision) ON DELETE RESTRICT
);

CREATE TABLE flow_device_runs (
  id TEXT PRIMARY KEY,
  run_id TEXT NOT NULL,
  udid TEXT NOT NULL,
  state TEXT NOT NULL CHECK (
    state IN ('queued','preflight','running','succeeded','failed','skipped','cancelled')
  ),
  capability_snapshot_json TEXT,
  release_proof_json TEXT,
  error_json TEXT,
  started_at TEXT,
  finished_at TEXT,
  UNIQUE (run_id, udid),
  FOREIGN KEY (run_id) REFERENCES flow_runs(id) ON DELETE RESTRICT
);

CREATE TABLE flow_node_attempts (
  id TEXT PRIMARY KEY,
  device_run_id TEXT NOT NULL,
  node_id TEXT NOT NULL,
  action_kind TEXT NOT NULL,
  attempt_no INTEGER NOT NULL CHECK (attempt_no >= 1),
  side_effect_class TEXT NOT NULL CHECK (
    side_effect_class IN ('none','idempotentSet','ambiguousUi','artifactWrite')
  ),
  state TEXT NOT NULL CHECK (
    state IN (
      'queued','intentCommitted','effectDispatched','verifying','succeeded',
      'failedBeforeDispatch','failedVerified','uncertain','cancelled','interrupted'
    )
  ),
  canonical_input_json TEXT,
  evidence_baseline_json TEXT,
  evidence_result_json TEXT,
  retry_safe INTEGER NOT NULL DEFAULT 0 CHECK (retry_safe IN (0, 1)),
  error_json TEXT,
  started_at TEXT,
  updated_at TEXT NOT NULL,
  finished_at TEXT,
  UNIQUE (device_run_id, node_id, attempt_no),
  FOREIGN KEY (device_run_id) REFERENCES flow_device_runs(id) ON DELETE RESTRICT
);

CREATE TABLE flow_artifacts (
  id TEXT PRIMARY KEY,
  attempt_id TEXT NOT NULL,
  relative_path TEXT NOT NULL UNIQUE,
  label TEXT NOT NULL,
  kind TEXT NOT NULL CHECK (kind IN ('jpeg','png')),
  size INTEGER NOT NULL CHECK (size > 0),
  sha256 TEXT NOT NULL CHECK (
    length(sha256) = 64 AND sha256 NOT GLOB '*[^0-9a-f]*'
  ),
  created_at TEXT NOT NULL,
  FOREIGN KEY (attempt_id) REFERENCES flow_node_attempts(id) ON DELETE RESTRICT
);

CREATE TABLE flow_events (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  run_id TEXT NOT NULL,
  revision INTEGER NOT NULL CHECK (revision >= 1),
  kind TEXT NOT NULL,
  payload_json TEXT NOT NULL,
  created_at TEXT NOT NULL,
  UNIQUE (run_id, revision),
  FOREIGN KEY (run_id) REFERENCES flow_runs(id) ON DELETE RESTRICT
);

CREATE INDEX idx_flow_documents_updated ON flow_documents(updated_at DESC);
CREATE INDEX idx_flow_runs_updated ON flow_runs(updated_at DESC);
CREATE INDEX idx_flow_device_runs_state ON flow_device_runs(run_id, state);
CREATE INDEX idx_flow_attempts_state ON flow_node_attempts(device_run_id, state);
CREATE INDEX idx_flow_artifacts_attempt ON flow_artifacts(attempt_id);
CREATE INDEX idx_flow_events_revision ON flow_events(run_id, revision);
```

Migration 1 is the current pre-Flow table batch byte-for-byte, including the guest
admin seed for a new empty database. Legacy recognition compares table names,
columns, primary keys, and unique constraints, not only the table-name set.

- [ ] **Step 5: Run migration tests and full DB tests**

```powershell
cargo fmt --all
cargo test -p riviu-core db::migrations -- --nocapture
cargo test -p riviu-core db -- --nocapture
```

Expected: populated upgrade, rollback failpoint, reopen, and unknown-schema tests pass.

- [ ] **Step 6: Commit Task 4**

```powershell
git add crates/core/src/db.rs crates/core/src/db/migrations.rs
git commit -m "feat(flow): add transactional database migrations"
```

### Task 5: Persist Immutable Flow Revisions

**Files:**
- Create: `crates/core/src/db/flows.rs`
- Modify: `crates/core/src/db.rs`
- Modify: `crates/core/src/flow/model.rs`
- Test: `crates/core/src/db/flows.rs`

- [ ] **Step 1: Write repository tests**

Test create revision 1, optimistic save revision 2, stale expected-revision conflict, list ordering, get exact revision, archive, hash mismatch rejection, and proof that editing canvas position changes authoring JSON but preserves a compiler-provided plan hash.

Use this exact optimistic-lock test:

```rust
fn flow_database_fixture() -> (Database, std::path::PathBuf) {
    let path = std::env::temp_dir().join(format!(
        "riviu-flow-repository-{}.db",
        uuid::Uuid::new_v4()
    ));
    let database = Database::open(&path).expect("flow database");
    (database, path)
}

#[test]
fn immutable_revision_save_rejects_a_stale_writer() {
    let (database, path) = flow_database_fixture();
    let document = FlowDocumentV2::empty("Fixture");
    let compiled = CompiledFlowPlanV2 {
        schema_version: FLOW_SCHEMA_VERSION,
        flow_id: document.id,
        revision: 0,
        nodes: Default::default(),
        execution_order: Vec::new(),
        context_plan: ContextPlan {
            requires_exclusive: false,
            requires_ui_session: false,
            requires_stream: false,
            requires_fresh_text_session: false,
            initial_bundle_id: None,
        },
        action_definition_versions: Default::default(),
        required_capabilities: Default::default(),
    };
    let mut document = document;
    document.revision = 1;
    let mut compiled = compiled;
    compiled.revision = 1;
    let hash = compiled_plan_sha256(&compiled).expect("plan hash");
    let first = database.save_flow_revision(None, &document, &compiled, &hash)
        .expect("revision one");
    assert_eq!(first.document.revision, 1);

    let mut second_document = first.document.clone();
    second_document.revision = 2;
    second_document.viewport.x = 240.0;
    second_document.nodes[0].position.y += 40.0;
    let mut second_plan = first.compiled_plan.clone();
    second_plan.revision = 2;
    let second_hash = compiled_plan_sha256(&second_plan).expect("second hash");
    assert_eq!(hash, second_hash, "revision/layout-only save changed execution hash");
    let second = database.save_flow_revision(Some(1), &second_document, &second_plan, &second_hash)
        .expect("revision two");
    assert_eq!(second.document.revision, 2);
    let error = database.save_flow_revision(Some(1), &second.document, &second.compiled_plan, &second.plan_hash)
        .expect_err("stale save must fail");
    assert!(error.to_string().contains("expected revision 1"));
    std::fs::remove_file(path).expect("remove fixture");
}
```

- [ ] **Step 2: Run tests red**

```powershell
cargo test -p riviu-core db::flows -- --nocapture
```

Expected: FAIL because Flow repository methods do not exist.

- [ ] **Step 3: Define persisted projections**

Add these projections to `model.rs`:

```rust
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RevisionConflict {
    pub expected: u64,
    pub actual: u64,
}
```

- [ ] **Step 4: Implement transactional repository methods**

Declare `mod flows;` in `db.rs`. In `flows.rs`, implement on `Database`:

```rust
pub fn save_flow_revision(
    &self,
    expected_revision: Option<u64>,
    document: &FlowDocumentV2,
    compiled: &CompiledFlowPlanV2,
    plan_hash: &str,
) -> anyhow::Result<FlowRevisionRecord>;
pub fn list_flows(&self, include_archived: bool) -> anyhow::Result<Vec<FlowSummary>>;
pub fn get_flow_revision(&self, id: FlowId, revision: Option<u64>) -> anyhow::Result<Option<FlowRevisionRecord>>;
pub fn archive_flow(&self, id: FlowId) -> anyhow::Result<()>;
```

Open one `IMMEDIATE` transaction, load `latest_revision` (zero for a new ID), and
compare `expected_revision`. Require both `document.revision` and
`compiled.revision` to equal `latest_revision + 1`; never mutate either after
compilation. Recompute `compiled_plan_sha256(compiled)` and require byte equality
with `plan_hash`; store `canonical_compiled_plan_json(compiled)` verbatim in
`compiled_json` before inserting the immutable revision and updating the document
projection. `expected_revision=None` is valid only when the flow ID does not exist.

- [ ] **Step 5: Run repository and workspace tests**

```powershell
cargo fmt --all
cargo test -p riviu-core db::flows -- --nocapture
cargo test -p riviu-script-engine
cargo test --workspace
```

Expected: all tests pass with legacy rows still readable.

- [ ] **Step 6: Commit Task 5**

```powershell
git add crates/core/src/db crates/core/src/flow/model.rs
git commit -m "feat(flow): persist immutable compiled revisions"
```

### Task 6: Close Foundation Gate F0

**Files:**
- Modify: `AGENTS.md`
- Modify: `docs/superpowers/plans/2026-07-30-riviu-flow-v2-foundation.md`

- [ ] **Step 1: Run the full gate**

```powershell
cargo fmt --all -- --check
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
git diff --check
```

Expected: every command exits 0.

- [ ] **Step 2: Record F0 and commit**

Mark all F0 checkboxes complete. Append the F0 commit range, test counts, disabled Terminate/TikTok/runtime nodes, next plan path, and rollback commit to `AGENTS.md`.

```powershell
git add AGENTS.md docs/superpowers/plans/2026-07-30-riviu-flow-v2-foundation.md
git commit -m "docs(flow): record foundation gate F0"
```
