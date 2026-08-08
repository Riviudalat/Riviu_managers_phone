use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::model::*;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ActionCategory {
    Control,
    App,
    Input,
    Timing,
    Evidence,
}

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
            "type": "object",
            "additionalProperties": false,
            "properties": {}
        }),
        ActionKind::LaunchApp | ActionKind::TerminateApp => serde_json::json!({
            "type": "object",
            "additionalProperties": false,
            "required": ["bundleId"],
            "properties": {
                "bundleId": { "type": "string", "minLength": 1, "maxLength": 255 }
            }
        }),
        ActionKind::Wait => serde_json::json!({
            "type": "object",
            "additionalProperties": false,
            "required": ["durationMs"],
            "properties": {
                "durationMs": { "type": "integer", "minimum": 1, "maximum": 60000 }
            }
        }),
        ActionKind::Tap => serde_json::json!({
            "type": "object",
            "additionalProperties": false,
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
            "type": "object",
            "additionalProperties": false,
            "required": ["from", "to", "durationMs"],
            "properties": {
                "from": coordinate_schema(),
                "to": coordinate_schema(),
                "durationMs": { "type": "integer", "minimum": 1, "maximum": 5000 }
            }
        }),
        ActionKind::TypeText => serde_json::json!({
            "type": "object",
            "additionalProperties": false,
            "required": ["text", "readBackLocator"],
            "properties": {
                "text": { "type": "string", "minLength": 1, "maxLength": 4096 },
                "readBackLocator": read_back_locator_schema()
            }
        }),
        ActionKind::Screenshot => serde_json::json!({
            "type": "object",
            "additionalProperties": false,
            "required": ["label", "format"],
            "properties": {
                "label": { "type": "string", "minLength": 1, "maxLength": 96 },
                "format": { "type": "string", "enum": ["jpeg"] }
            }
        }),
        ActionKind::AssertVisible => serde_json::json!({
            "type": "object",
            "additionalProperties": false,
            "required": ["accessibilityId"],
            "properties": {
                "accessibilityId": { "type": "string", "minLength": 1, "maxLength": 512 }
            }
        }),
        ActionKind::TapVision => serde_json::json!({
            "type": "object",
            "additionalProperties": false,
            "required": ["templatePngBase64", "threshold"],
            "properties": {
                "templatePngBase64": { "type": "string", "minLength": 1 },
                "threshold": { "type": "number", "minimum": 0.0, "maximum": 1.0 },
                "region": {
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["x0", "y0", "x1", "y1"],
                    "properties": {
                        "x0": { "type": "number", "minimum": 0.0, "maximum": 1.0 },
                        "y0": { "type": "number", "minimum": 0.0, "maximum": 1.0 },
                        "x1": { "type": "number", "minimum": 0.0, "maximum": 1.0 },
                        "y1": { "type": "number", "minimum": 0.0, "maximum": 1.0 }
                    }
                }
            }
        }),
        ActionKind::RawHttp | ActionKind::RawWda | ActionKind::Shell => Value::Null,
    }
}

pub fn required_capabilities(kind: ActionKind) -> Vec<String> {
    let capabilities: &[&str] = match kind {
        ActionKind::Start | ActionKind::End | ActionKind::Wait => &[],
        ActionKind::LaunchApp => &["app.launch"],
        ActionKind::TerminateApp => &["app.terminate"],
        ActionKind::Tap | ActionKind::TapVision => &["ui.tap", "stream"],
        ActionKind::Swipe => &["ui.swipe", "stream"],
        ActionKind::TypeText => &["ui.text", "stream", "accessibility.readText"],
        ActionKind::Screenshot => &["stream"],
        ActionKind::Home => &["ui.home"],
        ActionKind::AssertVisible => &["accessibility.visible"],
        ActionKind::RawHttp | ActionKind::RawWda | ActionKind::Shell => &[],
    };
    capabilities
        .iter()
        .map(|capability| (*capability).to_string())
        .collect()
}

pub fn contracts(
    kind: ActionKind,
) -> (
    ResourceClass,
    SideEffectClass,
    EvidenceRequirement,
    ReconciliationPolicy,
    RetryPolicy,
) {
    match kind {
        ActionKind::Start | ActionKind::End => (
            ResourceClass::PureDesktop,
            SideEffectClass::None,
            EvidenceRequirement::None,
            ReconciliationPolicy::None,
            RetryPolicy::BeforeDispatchOnly,
        ),
        ActionKind::Wait => (
            ResourceClass::PureDesktop,
            SideEffectClass::None,
            EvidenceRequirement::None,
            ReconciliationPolicy::None,
            RetryPolicy::BeforeDispatchOnly,
        ),
        ActionKind::AssertVisible => (
            ResourceClass::UiSession,
            SideEffectClass::None,
            EvidenceRequirement::None,
            ReconciliationPolicy::None,
            RetryPolicy::BeforeDispatchOnly,
        ),
        ActionKind::LaunchApp | ActionKind::Home => (
            ResourceClass::UiSession,
            SideEffectClass::IdempotentSet,
            EvidenceRequirement::ActiveApp,
            ReconciliationPolicy::ReadActiveApp,
            RetryPolicy::IdempotentAfterRead,
        ),
        ActionKind::Tap | ActionKind::Swipe | ActionKind::TapVision => (
            ResourceClass::UiWithStream,
            SideEffectClass::AmbiguousUi,
            EvidenceRequirement::Frame,
            ReconciliationPolicy::ReadFrame,
            RetryPolicy::BeforeDispatchOnly,
        ),
        ActionKind::TypeText => (
            ResourceClass::UiWithStream,
            SideEffectClass::AmbiguousUi,
            EvidenceRequirement::TextOrQualifiedFrame,
            ReconciliationPolicy::ReadText,
            RetryPolicy::BeforeDispatchOnly,
        ),
        ActionKind::Screenshot => (
            ResourceClass::UiWithStream,
            SideEffectClass::ArtifactWrite,
            EvidenceRequirement::Artifact,
            ReconciliationPolicy::ReadArtifact,
            RetryPolicy::BeforeDispatchOnly,
        ),
        ActionKind::TerminateApp => (
            ResourceClass::Bridge,
            SideEffectClass::IdempotentSet,
            EvidenceRequirement::Process,
            ReconciliationPolicy::ReadProcess,
            RetryPolicy::IdempotentAfterRead,
        ),
        ActionKind::RawHttp | ActionKind::RawWda | ActionKind::Shell => (
            ResourceClass::Bridge,
            SideEffectClass::AmbiguousUi,
            EvidenceRequirement::None,
            ReconciliationPolicy::None,
            RetryPolicy::Never,
        ),
    }
}

pub fn release_one_catalog() -> Vec<ActionDefinition> {
    [
        ActionKind::Start,
        ActionKind::End,
        ActionKind::LaunchApp,
        ActionKind::TerminateApp,
        ActionKind::Wait,
        ActionKind::Tap,
        ActionKind::Swipe,
        ActionKind::TypeText,
        ActionKind::Screenshot,
        ActionKind::Home,
        ActionKind::AssertVisible,
        ActionKind::TapVision,
    ]
    .into_iter()
    .map(action_definition)
    .collect()
}

fn action_definition(kind: ActionKind) -> ActionDefinition {
    let (
        resource_class,
        side_effect_class,
        evidence_requirement,
        reconciliation_policy,
        retry_policy,
    ) = contracts(kind);
    ActionDefinition {
        kind,
        schema_version: 1,
        label: label(kind).into(),
        disabled_reason: None,
        category: category(kind),
        config_schema: config_schema(kind),
        input_ports: if kind == ActionKind::Start {
            Vec::new()
        } else {
            vec![flow_port()]
        },
        output_ports: if kind == ActionKind::End {
            Vec::new()
        } else {
            vec![flow_port()]
        },
        required_capabilities: required_capabilities(kind),
        resource_class,
        side_effect_class,
        evidence_requirement,
        allowed_evidence: allowed_evidence(kind),
        qualified_detector_ids: Vec::new(),
        reconciliation_policy,
        default_timeout_ms: default_timeout_ms(kind),
        retry_policy,
    }
}

fn flow_port() -> PortDefinition {
    PortDefinition {
        name: "flow".into(),
        value_type: "flow".into(),
        required: true,
    }
}

fn label(kind: ActionKind) -> &'static str {
    match kind {
        ActionKind::Start => "Start",
        ActionKind::End => "End",
        ActionKind::LaunchApp => "Launch App",
        ActionKind::TerminateApp => "Terminate App",
        ActionKind::Wait => "Wait",
        ActionKind::Tap => "Tap",
        ActionKind::Swipe => "Swipe",
        ActionKind::TypeText => "Type Text",
        ActionKind::Screenshot => "Screenshot",
        ActionKind::Home => "Home",
        ActionKind::AssertVisible => "Assert Visible",
        ActionKind::TapVision => "Tap Vision",
        ActionKind::RawHttp => "Raw HTTP",
        ActionKind::RawWda => "Raw WDA",
        ActionKind::Shell => "Shell",
    }
}

fn category(kind: ActionKind) -> ActionCategory {
    match kind {
        ActionKind::Start
        | ActionKind::End
        | ActionKind::RawHttp
        | ActionKind::RawWda
        | ActionKind::Shell => ActionCategory::Control,
        ActionKind::LaunchApp | ActionKind::TerminateApp | ActionKind::Home => ActionCategory::App,
        ActionKind::Tap | ActionKind::Swipe | ActionKind::TypeText | ActionKind::TapVision => {
            ActionCategory::Input
        }
        ActionKind::Wait => ActionCategory::Timing,
        ActionKind::Screenshot | ActionKind::AssertVisible => ActionCategory::Evidence,
    }
}

fn allowed_evidence(kind: ActionKind) -> Vec<EvidenceKind> {
    match kind {
        ActionKind::LaunchApp | ActionKind::Home => vec![EvidenceKind::ActiveAppEquals],
        ActionKind::TerminateApp => vec![EvidenceKind::ProcessAbsent],
        ActionKind::Tap | ActionKind::TapVision => vec![EvidenceKind::FrameRegionChanged],
        ActionKind::Swipe => vec![EvidenceKind::FrameDigestChanged],
        ActionKind::TypeText => vec![EvidenceKind::TextReadBackEquals],
        ActionKind::Screenshot => vec![EvidenceKind::ArtifactDecodedAndHashed],
        ActionKind::Start
        | ActionKind::End
        | ActionKind::Wait
        | ActionKind::AssertVisible
        | ActionKind::RawHttp
        | ActionKind::RawWda
        | ActionKind::Shell => Vec::new(),
    }
}

fn default_timeout_ms(kind: ActionKind) -> u32 {
    match kind {
        ActionKind::Start | ActionKind::End => 1_000,
        ActionKind::Wait => 60_000,
        ActionKind::LaunchApp | ActionKind::TerminateApp | ActionKind::Home => 10_000,
        ActionKind::Tap | ActionKind::Swipe | ActionKind::Screenshot | ActionKind::TapVision => {
            5_000
        }
        ActionKind::TypeText => 10_000,
        ActionKind::AssertVisible => 4_000,
        ActionKind::RawHttp | ActionKind::RawWda | ActionKind::Shell => 10_000,
    }
}
