//! Decline only the measured contact-sync dialog. Never accept contact access.
use crate::{tiktok_labels::TikTokControls, ui_automation::tree::Tree, ElementBox, ElementQuery};

pub fn decline_contacts(tree: &Tree, labels: TikTokControls) -> Option<ElementBox> {
    let package = labels.package();
    if labels.language() != "en" {
        return None;
    }
    let prompt_text = match (package, labels.resource_version()) {
        ("com.ss.android.ugc.trill", Some("38.3.2")) => "TikTok is more fun with friends. By syncing your phone contacts, you can find and get discovered by people you know.",
        ("com.zhiliaoapp.musically", Some("45.4.3")) => "To connect with people you know on TikTok, allow access to your contacts in your device settings.",
        _ => return None,
    };
    let dialogs = tree.matching(
        package,
        ElementQuery::Description {
            value: "Dialog",
            exact: true,
        },
    );
    let [dialog] = dialogs.as_slice() else {
        return None;
    };
    let prompts = tree.matching(
        package,
        ElementQuery::Text {
            value: prompt_text,
            exact: true,
        },
    );
    let [prompt] = prompts.as_slice() else {
        return None;
    };
    if !tree.inside(*prompt, *dialog) {
        return None;
    }
    let controls = tree.matching(
        package,
        ElementQuery::Text {
            value: "Don’t allow",
            exact: true,
        },
    );
    let [control] = controls.as_slice() else {
        return None;
    };
    let node = &tree.nodes[*control];
    if !tree.inside(*control, *dialog) || node.attr("class") != "android.widget.Button" {
        return None;
    }
    node.rect().filter(|r| r.enabled && r.clickable)
}

#[cfg(test)]
mod tests {
    use super::*;
    const PACKAGE: &str = "com.ss.android.ugc.trill";
    const FIXTURE: &str =
        include_str!("../../fixtures/tiktok-publish/contacts-sync-trill-38.3.2-en.fixture");
    fn labels() -> TikTokControls {
        crate::tiktok_labels::controls_for(PACKAGE, "en", "38.3.2").unwrap()
    }
    fn tree(xml: String) -> Tree {
        Tree::parse(crate::HierarchySourceSnapshot { generation: 1, xml }).unwrap()
    }

    #[test]
    fn global_contact_settings_prompt_only_permits_the_negative_action() {
        let source =
            include_str!("../../fixtures/tiktok-publish/contacts-settings-global45.4.3.fixture");
        let labels =
            crate::tiktok_labels::controls_for("com.zhiliaoapp.musically", "en", "45.4.3").unwrap();
        let result = decline_contacts(&tree(source.into()), labels).unwrap();
        assert_eq!((result.x, result.y), (172.0, 1190.0));
        assert!(decline_contacts(
            &tree(source.replace("your contacts", "your location")),
            labels
        )
        .is_none());
        assert!(decline_contacts(
            &tree(source.replace("Don’t allow", "Open settings")),
            labels
        )
        .is_none());
    }

    #[test]
    fn regression_contacts_decline_requires_the_exact_dialog_and_unique_negative_button() {
        let button = decline_contacts(&tree(FIXTURE.into()), labels()).expect("measured decline");
        assert_eq!((button.x, button.y), (120.0, 1234.0));
        for changed in [
            FIXTURE.replace("syncing your phone contacts", "sharing your location"),
            FIXTURE.replace("Don’t allow", "Allow"),
            FIXTURE.replace("content-desc=\"Dialog\"", "content-desc=\"Other\""),
            FIXTURE.replace("text=\"Don’t allow\" enabled=\"true\"", "text=\"Don’t allow\" enabled=\"false\""),
            FIXTURE.replace("</hierarchy>", "<node package=\"com.ss.android.ugc.trill\" text=\"Don’t allow\" class=\"android.widget.Button\" enabled=\"true\" clickable=\"true\" bounds=\"[1,1][2,2]\"/></hierarchy>"),
        ] {
            assert!(decline_contacts(&tree(changed), labels()).is_none());
        }
        for (package, language, version) in [
            ("com.zhiliaoapp.musically", "en", "46.2.1"),
            (PACKAGE, "vi", "38.3.2"),
            (PACKAGE, "en", "46.3.3"),
        ] {
            let other = crate::tiktok_labels::controls_for(package, language, version).unwrap();
            assert!(decline_contacts(&tree(FIXTURE.into()), other).is_none());
        }
    }
}
