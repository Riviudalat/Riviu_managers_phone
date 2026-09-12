use super::{model::*, profile::CompatibilityPack, resolver, tree::Tree};
use crate::app_automation::{AppAdapter, SettingsAdapter};

fn app() -> AppContext {
    AppContext {
        package: "com.android.settings".into(),
        version: "any".into(),
        system_locale: "en-US".into(),
        observed_language: None,
        width: 400,
        height: 800,
    }
}
fn tree(children: &str) -> Tree {
    Tree::parse(crate::HierarchySourceSnapshot{generation:1,xml:format!("<hierarchy><node package=\"com.android.settings\" bounds=\"[0,0][400,800]\" enabled=\"true\">{children}</node></hierarchy>")}).unwrap()
}
fn node(id: &str, text: &str) -> String {
    format!("<node package=\"com.android.settings\" resource-id=\"{id}\" text=\"{text}\" bounds=\"[10,20][200,70]\" enabled=\"true\" clickable=\"true\"/>")
}

#[test]
fn settings_adapter_survives_id_change_but_rejects_competing_targets() {
    let pack = SettingsAdapter.pack();
    let spec = pack.target("aboutDevice").unwrap();
    for id in ["old", "new", "another-layout"] {
        let t = tree(&node(id, "About phone"));
        assert_eq!(resolver::resolve(&t, &app(), spec).len(), 1);
    }
    let t = tree(&(node("a", "About phone") + &node("b", "About device")));
    assert_eq!(resolver::resolve(&t, &app(), spec).len(), 2);
}
#[test]
fn hidden_ancestor_and_disabled_node_never_authorize_navigation() {
    let pack = SettingsAdapter.pack();
    let spec = pack.target("aboutDevice").unwrap();
    for content in [
        format!(
            "<node displayed=\"false\">{}</node>",
            node("a", "About phone")
        ),
        node("a", "About phone").replace("enabled=\"true\"", "enabled=\"false\""),
    ] {
        assert!(resolver::resolve(&tree(&content), &app(), spec).is_empty());
    }
}
#[test]
fn malformed_or_oversized_tree_is_rejected() {
    for xml in ["<!DOCTYPE a><hierarchy/>", "<hierarchy><node>"] {
        assert!(Tree::parse(crate::HierarchySourceSnapshot {
            generation: 1,
            xml: xml.into()
        })
        .is_err());
    }
}
#[test]
fn compatibility_pack_rejects_unknown_code_and_duplicate_targets() {
    let mut pack = serde_json::to_value(SettingsAdapter.pack()).unwrap();
    pack["shell"] = serde_json::json!("command");
    assert!(CompatibilityPack::parse(&serde_json::to_vec(&pack).unwrap()).is_err());
    pack.as_object_mut().unwrap().remove("shell");
    let target = pack["targets"][0].clone();
    pack["targets"].as_array_mut().unwrap().push(target);
    assert!(CompatibilityPack::parse(&serde_json::to_vec(&pack).unwrap()).is_err());
}

#[test]
fn bundled_profiles_replay_positive_negative_and_ambiguous_observations() {
    use crate::app_automation::TikTokAdapter;
    assert_eq!(SettingsAdapter.pack().verify_fixtures().unwrap(), 30);
    assert_eq!(TikTokAdapter.pack().verify_fixtures().unwrap(), 42);
}

#[test]
fn settings_read_uses_its_own_adapter_and_sibling_relationship() {
    let t=tree("<node package=\"com.android.settings\"><node package=\"com.android.settings\" text=\"Android version\"/><node package=\"com.android.settings\" text=\"15\"/></node><node package=\"com.android.settings\" text=\"2026\"/>");
    assert_eq!(
        SettingsAdapter.android_version(&t, &app()).as_deref(),
        Some("15")
    );
}

#[test]
fn unknown_version_uses_semantic_roles_without_claiming_measured_ids() {
    use crate::tiktok_labels::{controls_for, controls_for_runtime, LabelMatch, TikTokControl};
    let package = "com.zhiliaoapp.musically";
    assert!(controls_for(package, "en-US", "99.1")
        .unwrap()
        .resource_version()
        .is_none());
    let runtime = controls_for_runtime(package, "en-US", "99.1").unwrap();
    assert!(runtime.adaptive());
    assert_eq!(
        runtime.label(TikTokControl::CommentSend),
        Some(LabelMatch::Semantic("commentSend"))
    );
    assert!(crate::tiktok_composer::ComposerPlan::resolve(&runtime)
        .unwrap()
        .can_publish_carousel());
    assert!(
        crate::tiktok_share::PublishVerificationPlan::for_runtime(package, "en-US", "99.1").is_ok()
    );
    assert!(
        crate::tiktok_share::PublishVerificationPlan::for_build(package, "en-US", "99.1").is_err()
    );
}

#[test]
fn semantic_caption_requires_a_post_screen_and_rejects_ambiguous_inputs() {
    let package = "com.zhiliaoapp.musically";
    let node = |class: &str, text: &str| {
        format!("<node package=\"{package}\" class=\"{class}\" text=\"{text}\" bounds=\"[10,10][100,50]\" enabled=\"true\" clickable=\"true\"/>")
    };
    let parse = |xml: String| {
        Tree::parse(crate::HierarchySourceSnapshot {
            generation: 1,
            xml: format!("<hierarchy>{xml}</hierarchy>"),
        })
        .unwrap()
    };
    let one = node("android.widget.EditText", "Some text");
    assert!(
        crate::app_automation::tiktok_roles::indices(&parse(one.clone()), package, "caption")
            .is_empty()
    );
    let screen = node("android.widget.Button", "Post") + &one;
    assert_eq!(
        crate::app_automation::tiktok_roles::indices(&parse(screen.clone()), package, "caption")
            .len(),
        1
    );
    assert!(crate::app_automation::tiktok_roles::indices(
        &parse(screen + &one),
        package,
        "caption"
    )
    .is_empty());
}
#[test]
fn stale_provider_response_cannot_be_used_after_session_change() {
    let request = GuiRequest {
        scope: None,
        protocol_version: 1,
        request_id: "a".into(),
        observation_id: "b".into(),
        session_epoch: "c".into(),
        generation: 1,
        app: app(),
        target: "aboutDevice".into(),
        expected_screen: "settings".into(),
        remaining_ms: 1000,
        nodes: vec![],
        screenshot: String::new(),
        screenshot_sha256: String::new(),
    };
    let mut response = GuiResponse {
        protocol_version: 1,
        request_id: "a".into(),
        observation_id: "b".into(),
        session_epoch: "c".into(),
        generation: 1,
        status: ResolutionStatus::Unresolved,
        candidates: vec![],
        reason: "missing".into(),
        model: None,
        elapsed_ms: 0,
        prompt_tokens: None,
        completion_tokens: None,
        cost_usd: None,
    };
    assert!(response.validate_binding(&request).is_ok());
    response.session_epoch = "new".into();
    assert!(response.validate_binding(&request).is_err());
}

#[test]
fn post_inputs_passed_does_not_hide_link_or_pending_blockers() {
    let mut row = crate::PublishPreflightAssignmentReport {
        checks: Vec::new(),
        ordinal: 0,
        bundle_id: "bundle".into(),
        udid: "device".into(),
        package_name: Some("com.zhiliaoapp.musically".into()),
        version: Some("46.0.41".into()),
        locale: Some("en-US".into()),
        media: crate::PublishPreflightCheck::Pass,
        composer: crate::PublishPreflightCheck::Pass,
        sound_picker: crate::PublishPreflightCheck::Pass,
        storage: crate::PublishPreflightCheck::Pass,
        required_bytes: 1,
        available_bytes: Some(2),
        issues: Vec::new(),
    };
    row.issues.push(crate::PublishExecutionIssue {
        code: "post_verification_pending".into(),
        assignment_id: None,
        udid: Some("device".into()),
        bundle_id: None,
        message: "pending".into(),
    });
    let checks = super::checks::publish_checks(&row);
    assert_eq!(
        checks.iter().find(|c| c.id == "pending").unwrap().status,
        CheckStatus::Blocked
    );
    assert_eq!(
        checks.iter().find(|c| c.id == "account").unwrap().status,
        CheckStatus::Unknown
    );
}
