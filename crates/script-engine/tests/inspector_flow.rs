use riviu_core::{
    ui_automation::inspector::ElementSelector, ActionKind, EvidenceSpec, FlowDocumentV2, FlowEdge,
    FlowNode,
};
#[test]
fn captured_selector_and_expected_element_compile_as_replayable_flow() {
    let selector = ElementSelector {
        package: "app.test".into(),
        description: Some("Profile".into()),
        text: None,
        resource_id: None,
        class_name: None,
    };
    let start = FlowNode::new(ActionKind::Start, serde_json::json!({}));
    let mut launch = FlowNode::new(
        ActionKind::LaunchApp,
        serde_json::json!({"bundleId":"app.test"}),
    );
    launch.postcondition = Some(EvidenceSpec::ActiveAppEquals {
        bundle_id: "app.test".into(),
    });
    let mut tap = FlowNode::new(ActionKind::Tap, serde_json::json!({"selector":selector}));
    tap.postcondition = Some(EvidenceSpec::ElementVisible {
        selector: ElementSelector {
            text: Some("Edit profile".into()),
            description: None,
            ..selector
        },
    });
    let end = FlowNode::new(ActionKind::End, serde_json::json!({}));
    let mut document:FlowDocumentV2=serde_json::from_value(serde_json::json!({"schemaVersion":2,"id":uuid::Uuid::new_v4(),"name":"Recorded Profile","revision":1,"entryNodeId":start.id,"nodes":[start,launch,tap,end],"edges":[],"viewport":{"x":0,"y":0,"zoom":1}})).unwrap();
    document.edges=document.nodes.windows(2).map(|p|serde_json::from_value::<FlowEdge>(serde_json::json!({"id":uuid::Uuid::new_v4(),"sourceNodeId":p[0].id,"sourcePort":"flow","targetNodeId":p[1].id,"targetPort":"flow"})).unwrap()).collect();
    assert!(
        riviu_script_engine::compile_flow(&document, &riviu_core::release_one_catalog()).is_ok()
    );
}
