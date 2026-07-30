pub mod artifact_store;
#[allow(dead_code)]
pub(crate) mod cancellation;
pub mod catalog;
#[allow(dead_code)]
pub(crate) mod device_context;
#[allow(dead_code)]
pub mod evidence;
#[allow(dead_code)]
pub(crate) mod executor;
pub mod model;

pub use artifact_store::*;
pub(crate) use cancellation::FlowCancellation;
pub use catalog::*;
pub(crate) use device_context::*;
pub use evidence::*;
#[allow(unused_imports)]
pub(crate) use executor::*;
pub use model::*;

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use serde_json::{json, Value};
    use uuid::Uuid;

    use super::*;
    use crate::{
        ActiveTransport, DeviceCapabilitySnapshot, InstalledAgentIdentity, InstalledTargetIdentity,
        QualifiedGeometry, ScreenOrientation,
    };

    const FLOW_ID: &str = "00000000-0000-0000-0000-000000000010";
    const NODE_ID: &str = "00000000-0000-0000-0000-000000000011";

    #[test]
    fn document_uses_camel_case_and_exact_schema_two() {
        let document = FlowDocumentV2::empty("Fixture");
        let json = serde_json::to_value(&document).expect("serialize flow");

        assert_eq!(json["schemaVersion"], 2);
        assert!(json.get("entryNodeId").is_some());
        assert!(json.get("schema_version").is_none());
        assert_eq!(document.revision, 0);
        assert_eq!(document.viewport, FlowViewport::default());
        assert_eq!(document.nodes.len(), 2);
        assert_eq!(document.edges.len(), 1);

        let start = &document.nodes[0];
        let end = &document.nodes[1];
        let edge = &document.edges[0];
        assert_eq!(start.kind, ActionKind::Start);
        assert_eq!(start.position, CanvasPoint { x: 0.0, y: 80.0 });
        assert_eq!(end.kind, ActionKind::End);
        assert_eq!(end.position, CanvasPoint { x: 320.0, y: 80.0 });
        assert_eq!(document.entry_node_id, start.id);
        assert_eq!(edge.source_node_id, start.id);
        assert_eq!(edge.target_node_id, end.id);
        assert_eq!(edge.source_port, "flow");
        assert_eq!(edge.target_port, "flow");
    }

    #[test]
    fn catalog_exposes_verified_terminate_but_never_raw_transport_actions() {
        let catalog = release_one_catalog();
        assert_eq!(
            catalog.iter().map(|entry| entry.kind).collect::<Vec<_>>(),
            vec![
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
            ]
        );
        assert!(catalog.iter().all(|entry| !matches!(
            entry.kind,
            ActionKind::RawHttp | ActionKind::RawWda | ActionKind::Shell
        )));
        let terminate = catalog
            .iter()
            .find(|entry| entry.kind == ActionKind::TerminateApp)
            .expect("verified Terminate definition");
        assert_eq!(terminate.required_capabilities, ["app.terminate"]);
        assert_eq!(terminate.resource_class, ResourceClass::Bridge);
        assert_eq!(terminate.side_effect_class, SideEffectClass::IdempotentSet);
        assert_eq!(terminate.evidence_requirement, EvidenceRequirement::Process);
        assert_eq!(terminate.allowed_evidence, [EvidenceKind::ProcessAbsent]);
        assert_eq!(
            terminate.reconciliation_policy,
            ReconciliationPolicy::ReadProcess
        );
        assert_eq!(terminate.retry_policy, RetryPolicy::IdempotentAfterRead);
        assert!(terminate.disabled_reason.is_none());
        assert!(catalog.iter().all(|entry| entry.disabled_reason.is_none()));
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
        })
        .expect("evidence JSON");
        assert_eq!(value["bundleId"], "com.apple.Preferences");
        assert!(value.get("bundle_id").is_none());
    }

    #[test]
    fn canonical_compiled_plan_json_matches_golden_and_keeps_revision() {
        let plan = compiled_plan_fixture(1);

        assert_eq!(
            canonical_compiled_plan_json(&plan).expect("canonical compiled plan"),
            concat!(
                "{\"actionDefinitionVersions\":{\"wait\":1},",
                "\"contextPlan\":{\"initialBundleId\":\"com.example.fixture\",",
                "\"requiresExclusive\":true,\"requiresFreshTextSession\":false,",
                "\"requiresStream\":true,\"requiresUiSession\":true},",
                "\"executionOrder\":[\"00000000-0000-0000-0000-000000000011\"],",
                "\"flowId\":\"00000000-0000-0000-0000-000000000010\",",
                "\"nodes\":{\"00000000-0000-0000-0000-000000000011\":{",
                "\"config\":{\"durationMs\":250,\"kind\":\"wait\"},",
                "\"id\":\"00000000-0000-0000-0000-000000000011\",",
                "\"kind\":\"wait\",\"postcondition\":null}},",
                "\"requiredCapabilities\":[\"stream\"],\"revision\":1,",
                "\"schemaVersion\":2}"
            )
        );
    }

    #[test]
    fn execution_hash_material_removes_only_top_level_revision() {
        let plan = compiled_plan_fixture(1);
        let stored: Value = serde_json::from_str(
            &canonical_compiled_plan_json(&plan).expect("stored canonical JSON"),
        )
        .expect("stored JSON value");
        let execution: Value = serde_json::from_str(
            &canonical_execution_hash_material_json(&plan).expect("execution hash material"),
        )
        .expect("execution JSON value");

        let mut expected = stored;
        let removed = expected
            .as_object_mut()
            .expect("compiled plan object")
            .remove("revision");
        assert_eq!(removed, Some(json!(1)));
        assert_eq!(execution, expected);
    }

    #[test]
    fn execution_hash_ignores_revision_but_covers_execution_identity() {
        let baseline = compiled_plan_fixture(1);
        let baseline_hash = compiled_plan_sha256(&baseline).expect("baseline hash");

        let revision_two = compiled_plan_fixture(2);
        assert_eq!(
            compiled_plan_sha256(&revision_two).expect("revision two hash"),
            baseline_hash
        );

        let mut typed_config = baseline.clone();
        typed_config.nodes.get_mut(&node_id()).unwrap().config =
            CompiledActionConfig::Wait { duration_ms: 251 };
        assert_hash_changes(&baseline_hash, &typed_config);

        let mut action_version = baseline.clone();
        action_version
            .action_definition_versions
            .insert(ActionKind::Wait, 2);
        assert_hash_changes(&baseline_hash, &action_version);

        let mut context = baseline.clone();
        context.context_plan.requires_fresh_text_session = true;
        assert_hash_changes(&baseline_hash, &context);

        let mut capability = baseline.clone();
        capability
            .required_capabilities
            .insert("ui.tap".to_string());
        assert_hash_changes(&baseline_hash, &capability);

        let mut flow = baseline.clone();
        flow.flow_id = Uuid::parse_str("00000000-0000-0000-0000-000000000099").unwrap();
        assert_hash_changes(&baseline_hash, &flow);
    }

    #[test]
    fn geometry_profile_is_stable_and_qualified_by_runtime_tuple() {
        let snapshot = capability_snapshot();
        let baseline = qualified_geometry_profile_id(&snapshot).expect("qualified geometry");
        assert_eq!(
            baseline,
            "689551a9dbaa2e8ca25165f5b76ecaf43aa1f354551f957d3a75657105b9072b"
        );
        assert_eq!(
            qualified_geometry_profile_id(&snapshot).expect("repeat geometry hash"),
            baseline
        );

        let mut target_build = snapshot.clone();
        target_build.target_app.build = "102".into();
        assert_geometry_hash_changes(&baseline, &target_build);

        let mut ios = snapshot.clone();
        ios.ios_version = "16.7.16".into();
        assert_geometry_hash_changes(&baseline, &ios);

        let mut orientation = snapshot.clone();
        orientation.geometry.as_mut().unwrap().orientation = ScreenOrientation::LandscapeLeft;
        assert_geometry_hash_changes(&baseline, &orientation);

        let mut pixels = snapshot;
        pixels.geometry.as_mut().unwrap().pixel_width += 1;
        assert_geometry_hash_changes(&baseline, &pixels);
    }

    #[test]
    fn geometry_profile_rejects_missing_or_non_finite_geometry() {
        let mut missing = capability_snapshot();
        missing.geometry = None;
        assert_eq!(
            qualified_geometry_profile_id(&missing),
            Err("geometry is absent")
        );

        let mut non_finite = capability_snapshot();
        non_finite.geometry.as_mut().unwrap().scale_x = f64::NAN;
        assert_eq!(
            qualified_geometry_profile_id(&non_finite),
            Err("geometry is invalid")
        );
    }

    #[test]
    fn catalog_definitions_match_release_one_contracts() {
        let catalog = release_one_catalog();
        for definition in &catalog {
            assert_eq!(definition.schema_version, 1);
            assert_eq!(definition.config_schema, config_schema(definition.kind));
            assert_eq!(
                definition.required_capabilities,
                required_capabilities(definition.kind)
            );
            let (resource, effect, evidence, reconciliation, retry) = contracts(definition.kind);
            assert_eq!(definition.resource_class, resource);
            assert_eq!(definition.side_effect_class, effect);
            assert_eq!(definition.evidence_requirement, evidence);
            assert_eq!(definition.reconciliation_policy, reconciliation);
            assert_eq!(definition.retry_policy, retry);

            let expected_inputs = usize::from(definition.kind != ActionKind::Start);
            let expected_outputs = usize::from(definition.kind != ActionKind::End);
            assert_eq!(definition.input_ports.len(), expected_inputs);
            assert_eq!(definition.output_ports.len(), expected_outputs);
            for port in definition
                .input_ports
                .iter()
                .chain(&definition.output_ports)
            {
                assert_eq!(port.name, "flow");
                assert_eq!(port.value_type, "flow");
                assert!(port.required);
            }
            assert!(definition.qualified_detector_ids.is_empty());
        }

        assert_eq!(config_schema(ActionKind::RawHttp), Value::Null);
        assert_eq!(config_schema(ActionKind::RawWda), Value::Null);
        assert_eq!(config_schema(ActionKind::Shell), Value::Null);
        assert_eq!(
            contracts(ActionKind::TerminateApp),
            (
                ResourceClass::Bridge,
                SideEffectClass::IdempotentSet,
                EvidenceRequirement::Process,
                ReconciliationPolicy::ReadProcess,
                RetryPolicy::IdempotentAfterRead,
            )
        );
        assert_eq!(
            contracts(ActionKind::RawHttp),
            (
                ResourceClass::Bridge,
                SideEffectClass::AmbiguousUi,
                EvidenceRequirement::None,
                ReconciliationPolicy::None,
                RetryPolicy::Never,
            )
        );
    }

    #[test]
    fn catalog_schemas_are_closed_and_exact() {
        for kind in [
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
        ] {
            let schema = config_schema(kind);
            assert_eq!(schema["additionalProperties"], false, "{kind:?}");
        }

        let tap = config_schema(ActionKind::Tap);
        assert_eq!(tap["oneOf"].as_array().unwrap().len(), 2);
        assert_eq!(tap["properties"]["point"]["additionalProperties"], false);
        assert_eq!(
            tap["properties"]["point"]["properties"]["profileId"]["pattern"],
            "^[0-9a-f]{64}$"
        );

        let type_text = config_schema(ActionKind::TypeText);
        assert_eq!(
            type_text["properties"]["readBackLocator"]["additionalProperties"],
            false
        );
        assert_eq!(
            type_text["properties"]["readBackLocator"]["properties"]["strategy"]["enum"],
            json!(["accessibilityId", "className"])
        );

        let screenshot = config_schema(ActionKind::Screenshot);
        assert_eq!(screenshot["properties"]["format"]["enum"], json!(["jpeg"]));
    }

    #[test]
    fn catalog_metadata_matches_the_release_one_action_table() {
        let expected = [
            (
                ActionKind::Start,
                "Start",
                ActionCategory::Control,
                1_000,
                vec![],
            ),
            (
                ActionKind::End,
                "End",
                ActionCategory::Control,
                1_000,
                vec![],
            ),
            (
                ActionKind::LaunchApp,
                "Launch App",
                ActionCategory::App,
                10_000,
                vec![EvidenceKind::ActiveAppEquals],
            ),
            (
                ActionKind::TerminateApp,
                "Terminate App",
                ActionCategory::App,
                10_000,
                vec![EvidenceKind::ProcessAbsent],
            ),
            (
                ActionKind::Wait,
                "Wait",
                ActionCategory::Timing,
                60_000,
                vec![],
            ),
            (
                ActionKind::Tap,
                "Tap",
                ActionCategory::Input,
                5_000,
                vec![EvidenceKind::FrameRegionChanged],
            ),
            (
                ActionKind::Swipe,
                "Swipe",
                ActionCategory::Input,
                5_000,
                vec![EvidenceKind::FrameDigestChanged],
            ),
            (
                ActionKind::TypeText,
                "Type Text",
                ActionCategory::Input,
                10_000,
                vec![EvidenceKind::TextReadBackEquals],
            ),
            (
                ActionKind::Screenshot,
                "Screenshot",
                ActionCategory::Evidence,
                5_000,
                vec![EvidenceKind::ArtifactDecodedAndHashed],
            ),
            (
                ActionKind::Home,
                "Home",
                ActionCategory::App,
                10_000,
                vec![EvidenceKind::ActiveAppEquals],
            ),
            (
                ActionKind::AssertVisible,
                "Assert Visible",
                ActionCategory::Evidence,
                4_000,
                vec![],
            ),
        ];
        let catalog = release_one_catalog();

        for (definition, (kind, label, category, timeout, evidence)) in catalog.iter().zip(expected)
        {
            assert_eq!(definition.kind, kind);
            assert_eq!(definition.label, label);
            assert_eq!(definition.category, category);
            assert_eq!(definition.default_timeout_ms, timeout);
            assert_eq!(definition.allowed_evidence, evidence);
        }
    }

    #[test]
    fn artifact_labels_are_bounded_portable_and_format_qualified() {
        for (label, format) in [
            ("capture", "jpeg"),
            ("capture.jpg", "jpeg"),
            ("capture.JPEG", "jpeg"),
            ("capture", "png"),
            ("capture.PNG", "png"),
        ] {
            assert_eq!(validate_artifact_label(label, format), Ok(()));
        }

        assert_eq!(
            validate_artifact_label("", "jpeg"),
            Err("ArtifactLabelLength")
        );
        assert_eq!(
            validate_artifact_label(" capture", "jpeg"),
            Err("ArtifactLabelLength")
        );
        assert_eq!(
            validate_artifact_label(&"a".repeat(97), "jpeg"),
            Err("ArtifactLabelLength")
        );
        for label in ["a/b", "a\\b", "a..b", "a\u{0007}b"] {
            assert_eq!(
                validate_artifact_label(label, "jpeg"),
                Err("ArtifactLabelCharacters")
            );
        }
        for label in [
            ".jpg",
            "CON",
            "CON.capture.jpg",
            "prn.jpeg",
            "AUX",
            "nul",
            "COM1",
            "com1.capture.jpeg",
            "lpt9.png",
        ] {
            assert_eq!(
                validate_artifact_label(label, "jpeg"),
                Err("ArtifactLabelReserved"),
                "{label}"
            );
        }
        assert_eq!(
            validate_artifact_label("capture.png", "jpeg"),
            Err("ArtifactLabelExtension")
        );
        assert_eq!(
            validate_artifact_label("capture.jpg", "png"),
            Err("ArtifactLabelExtension")
        );
        assert_eq!(
            validate_artifact_label("capture", "webp"),
            Err("ArtifactFormat")
        );
    }

    #[test]
    fn qualified_locator_and_coordinate_reject_unknown_fields() {
        let locator = json!({
            "strategy": "accessibilityId",
            "value": "SearchField",
            "extra": true,
        });
        assert!(serde_json::from_value::<QualifiedElementLocator>(locator).is_err());

        let coordinate = json!({
            "x": 10.0,
            "y": 20.0,
            "imageWidth": 750,
            "imageHeight": 1334,
            "orientation": "portrait",
            "profileId": "a".repeat(64),
            "extra": true,
        });
        assert!(serde_json::from_value::<ImageCoordinateTarget>(coordinate).is_err());
    }

    fn flow_id() -> Uuid {
        Uuid::parse_str(FLOW_ID).unwrap()
    }

    fn node_id() -> Uuid {
        Uuid::parse_str(NODE_ID).unwrap()
    }

    fn compiled_plan_fixture(revision: u64) -> CompiledFlowPlanV2 {
        let node = CompiledFlowNode {
            id: node_id(),
            kind: ActionKind::Wait,
            config: CompiledActionConfig::Wait { duration_ms: 250 },
            postcondition: None,
        };
        CompiledFlowPlanV2 {
            schema_version: FLOW_SCHEMA_VERSION,
            flow_id: flow_id(),
            revision,
            nodes: BTreeMap::from([(node.id, node)]),
            execution_order: vec![node_id()],
            context_plan: ContextPlan {
                requires_exclusive: true,
                requires_ui_session: true,
                requires_stream: true,
                requires_fresh_text_session: false,
                initial_bundle_id: Some("com.example.fixture".into()),
            },
            action_definition_versions: BTreeMap::from([(ActionKind::Wait, 1)]),
            required_capabilities: BTreeSet::from(["stream".to_string()]),
        }
    }

    fn capability_snapshot() -> DeviceCapabilitySnapshot {
        DeviceCapabilitySnapshot {
            installed_agent: InstalledAgentIdentity {
                bundle_id: "com.mrph.svc".into(),
                version: "1.0".into(),
                build: "1".into(),
                executable_name: "fixture-agent".into(),
                signer_identity_sha256: "22".repeat(32),
            },
            selected_artifact_sha256: "33".repeat(32),
            agent_version: "1.0".into(),
            protocol_version: 1,
            driver_adapter_version: "fixture-driver-1".into(),
            transport: ActiveTransport::Mock,
            product_type: "iPhone10,1".into(),
            ios_version: "16.7.15".into(),
            target_app: InstalledTargetIdentity {
                bundle_id: "com.apple.Preferences".into(),
                version: "1".into(),
                build: "1".into(),
            },
            protected_auth_ready: true,
            geometry: Some(QualifiedGeometry {
                logical_width: 375.0,
                logical_height: 667.0,
                pixel_width: 375,
                pixel_height: 667,
                scale_x: 1.0,
                scale_y: 1.0,
                orientation: ScreenOrientation::Portrait,
            }),
        }
    }

    fn assert_hash_changes(baseline: &str, plan: &CompiledFlowPlanV2) {
        assert_ne!(
            compiled_plan_sha256(plan).expect("mutated execution hash"),
            baseline
        );
    }

    fn assert_geometry_hash_changes(baseline: &str, snapshot: &DeviceCapabilitySnapshot) {
        assert_ne!(
            qualified_geometry_profile_id(snapshot).expect("mutated geometry hash"),
            baseline
        );
    }
}
