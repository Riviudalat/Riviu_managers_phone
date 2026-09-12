//! Seed artifact: three automation profiles + one orchestration Nuôi → Tương tác → Đăng.
//!
//! Engines stay unchanged; this only builds durable profiles and a graph that calls them.

use serde_json::{json, Value};
use uuid::Uuid;

use crate::{
    AutomationKind, AutomationProfileRef, InteractionActionSet, NurtureSettings,
    OrchestrationBranch, OrchestrationDocumentV1, OrchestrationEdge, OrchestrationNode,
    OrchestrationNodeAction, OrchestrationPoint, PublishSoundPolicy, SocialNetwork,
    ORCHESTRATION_SCHEMA_VERSION,
};

pub const THREE_FEATURE_ORCHESTRATION_NAME: &str = "Riviu TikTok — Nuôi · Tương tác · Đăng bài";
pub const THREE_FEATURE_NURTURE_PROFILE_NAME: &str = "Mẫu Nuôi TikTok";
pub const THREE_FEATURE_INTERACTION_PROFILE_NAME: &str = "Mẫu Tương tác TikTok";
pub const THREE_FEATURE_PUBLISH_PROFILE_NAME: &str = "Mẫu Đăng bài TikTok";

/// Default nurture profile payload (`network: tiktok`).
pub fn three_feature_nurture_config() -> Value {
    let settings = NurtureSettings {
        network: SocialNetwork::TikTok,
        ..NurtureSettings::default()
    };
    let mut public_settings = serde_json::to_value(&settings).expect("nurture settings serialize");
    if let Some(object) = public_settings.as_object_mut() {
        object.remove("apiKey");
        object.remove("hasApiKey");
    }
    json!({
        "schemaVersion": 1,
        "settings": public_settings,
        "durationMinutes": 30,
    })
}

/// Default interaction profile payload (like+save; operator fills targets later).
pub fn three_feature_interaction_config() -> Value {
    json!({
        "schemaVersion": 1,
        "request": {
            "targets": [],
            "messageCount": 2,
            "instruction": "Viết ngắn, tự nhiên bằng tiếng Việt.",
            "maxWords": 12,
            "mode": "threaded",
            "shape": "chain",
            "actions": InteractionActionSet {
                like: true,
                comment: false,
                save: true,
            },
            "network": SocialNetwork::TikTok,
        }
    })
}

/// Default publish profile payload (empty source; operator stages media before run).
pub fn three_feature_publish_config() -> Value {
    json!({
        "schemaVersion": 1,
        "sourceRoot": "",
        "bundleIds": [],
        "captionOverrides": {},
        "soundPolicy": PublishSoundPolicy::Default,
        "executionConfirmed": false,
        "sheetEnabled": true,
        "deleteAfterPublish": true,
        "network": SocialNetwork::TikTok,
    })
}

pub fn three_feature_config_for(kind: AutomationKind) -> Value {
    match kind {
        AutomationKind::Nurture => three_feature_nurture_config(),
        AutomationKind::Interaction => three_feature_interaction_config(),
        AutomationKind::Publish => three_feature_publish_config(),
    }
}

pub fn three_feature_profile_name(kind: AutomationKind) -> &'static str {
    match kind {
        AutomationKind::Nurture => THREE_FEATURE_NURTURE_PROFILE_NAME,
        AutomationKind::Interaction => THREE_FEATURE_INTERACTION_PROFILE_NAME,
        AutomationKind::Publish => THREE_FEATURE_PUBLISH_PROFILE_NAME,
    }
}

/// Sequential Start → Nuôi → Tương tác → Đăng → End. Non-done campaign branches stop at End.
pub fn build_three_feature_document(
    document_id: Uuid,
    nurture: AutomationProfileRef,
    interaction: AutomationProfileRef,
    publish: AutomationProfileRef,
) -> OrchestrationDocumentV1 {
    let start = Uuid::new_v4();
    let nurture_node = Uuid::new_v4();
    let interaction_node = Uuid::new_v4();
    let publish_node = Uuid::new_v4();
    let end = Uuid::new_v4();

    let nodes = vec![
        OrchestrationNode {
            id: start,
            position: OrchestrationPoint { x: 40.0, y: 80.0 },
            action: OrchestrationNodeAction::Start,
        },
        OrchestrationNode {
            id: nurture_node,
            position: OrchestrationPoint { x: 280.0, y: 80.0 },
            action: OrchestrationNodeAction::RunNurture {
                profile: nurture,
                target_override: None,
            },
        },
        OrchestrationNode {
            id: interaction_node,
            position: OrchestrationPoint { x: 520.0, y: 80.0 },
            action: OrchestrationNodeAction::RunInteraction {
                profile: interaction,
                target_override: None,
            },
        },
        OrchestrationNode {
            id: publish_node,
            position: OrchestrationPoint { x: 760.0, y: 80.0 },
            action: OrchestrationNodeAction::RunPublish {
                profile: publish,
                target_override: None,
            },
        },
        OrchestrationNode {
            id: end,
            position: OrchestrationPoint { x: 1000.0, y: 80.0 },
            action: OrchestrationNodeAction::End,
        },
    ];

    let mut edges = vec![OrchestrationEdge {
        source_node_id: start,
        source_port: OrchestrationBranch::Done,
        target_node_id: nurture_node,
    }];
    edges.extend(campaign_success_edges(nurture_node, interaction_node, end));
    edges.extend(campaign_success_edges(interaction_node, publish_node, end));
    edges.extend(campaign_success_edges(publish_node, end, end));

    OrchestrationDocumentV1 {
        schema_version: ORCHESTRATION_SCHEMA_VERSION,
        id: document_id,
        revision: 0,
        name: THREE_FEATURE_ORCHESTRATION_NAME.into(),
        entry_node_id: start,
        nodes,
        edges,
    }
}

fn campaign_success_edges(
    source: Uuid,
    next_on_done: Uuid,
    end_on_other: Uuid,
) -> Vec<OrchestrationEdge> {
    [
        (OrchestrationBranch::Done, next_on_done),
        (OrchestrationBranch::Partial, end_on_other),
        (OrchestrationBranch::Failed, end_on_other),
        (OrchestrationBranch::Uncertain, end_on_other),
    ]
    .into_iter()
    .map(|(source_port, target_node_id)| OrchestrationEdge {
        source_node_id: source,
        source_port,
        target_node_id,
    })
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        compile_orchestration, validate_automation_profile_config, AutomationDefinition,
        AutomationDefinitionRecord, AutomationDefinitionRevision, TargetRef,
    };

    fn profile_record(
        kind: AutomationKind,
        definition_id: Uuid,
        revision: u64,
    ) -> AutomationDefinitionRecord {
        AutomationDefinitionRecord {
            definition: AutomationDefinition {
                id: definition_id,
                name: three_feature_profile_name(kind).into(),
                kind,
                latest_revision: revision,
                archived: false,
                created_at: "2026-09-11T00:00:00Z".into(),
                updated_at: "2026-09-11T00:00:00Z".into(),
            },
            revision: AutomationDefinitionRevision {
                definition_id,
                revision,
                target_ref: TargetRef::All,
                config: three_feature_config_for(kind),
                created_at: "2026-09-11T00:00:00Z".into(),
            },
        }
    }

    #[test]
    fn template_configs_validate_and_document_compiles() {
        for kind in [
            AutomationKind::Nurture,
            AutomationKind::Interaction,
            AutomationKind::Publish,
        ] {
            validate_automation_profile_config(kind, &three_feature_config_for(kind))
                .unwrap_or_else(|error| panic!("{kind:?}: {error}"));
        }

        let nurture_id = Uuid::from_u128(11);
        let interaction_id = Uuid::from_u128(12);
        let publish_id = Uuid::from_u128(13);
        let profiles = [
            profile_record(AutomationKind::Nurture, nurture_id, 1),
            profile_record(AutomationKind::Interaction, interaction_id, 1),
            profile_record(AutomationKind::Publish, publish_id, 1),
        ];
        let document = build_three_feature_document(
            Uuid::from_u128(20),
            AutomationProfileRef {
                definition_id: nurture_id,
                revision: 1,
            },
            AutomationProfileRef {
                definition_id: interaction_id,
                revision: 1,
            },
            AutomationProfileRef {
                definition_id: publish_id,
                revision: 1,
            },
        );
        let compiled = compile_orchestration(&document, &profiles).expect("compile template");
        assert_eq!(compiled.document.name, THREE_FEATURE_ORCHESTRATION_NAME);
        assert_eq!(compiled.execution_order.len(), 5);
        let kinds: Vec<_> = compiled
            .document
            .nodes
            .iter()
            .map(|node| match &node.action {
                OrchestrationNodeAction::Start => "start",
                OrchestrationNodeAction::RunNurture { .. } => "runNurture",
                OrchestrationNodeAction::RunInteraction { .. } => "runInteraction",
                OrchestrationNodeAction::RunPublish { .. } => "runPublish",
                OrchestrationNodeAction::End => "end",
                OrchestrationNodeAction::Delay { .. } => "delay",
            })
            .collect();
        assert_eq!(
            kinds,
            ["start", "runNurture", "runInteraction", "runPublish", "end"]
        );
    }
}
