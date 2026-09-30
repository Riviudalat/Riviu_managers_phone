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

pub fn decline_facebook_permission(tree: &Tree, labels: TikTokControls) -> Option<ElementBox> {
    let package = labels.package();
    let prompt_id = match (package, labels.resource_version()) {
        ("com.ss.android.ugc.trill", Some("38.3.2")) => ":id/d2o",
        ("com.zhiliaoapp.musically", Some("45.7.3")) if labels.language() == "en" => ":id/esv",
        _ => return None,
    };
    let prompt = tree.matching(package, ElementQuery::ResourceIdSuffix(prompt_id));
    let [index] = prompt.as_slice() else {
        return None;
    };
    let text = tree.nodes[*index].attr("text");
    let question = "Give TikTok access to your Facebook friends list and email?";
    let measured_global = package == "com.zhiliaoapp.musically";
    if measured_global {
        // The measured Global dialog wraps this text in bidi formatting characters.
        let plain: String = text
            .chars()
            .filter(|ch| !matches!(ch, '\u{200e}' | '\u{200f}' | '\u{2066}'..='\u{2069}'))
            .collect();
        if plain != "Give TikTok access to your Facebook friends list and email? This will be used to improve your TikTok experience, including connecting you with people you may know, people you share connections with on Facebook, and personalizing your ads. Learn more in the Help Center"
            || tree.nodes[*index].attr("class") != "android.widget.TextView"
        {
            return None;
        }
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
        if tree.nodes[*dialog].attr("resource-id") != format!("{package}:id/visual_area")
            || !tree.inside(*index, *dialog)
        {
            return None;
        }
    } else if !text.contains(question) {
        return None;
    }
    let controls = tree.matching(
        package,
        ElementQuery::Text {
            value: "Don’t allow",
            exact: true,
        },
    );
    let [index] = controls.as_slice() else {
        return None;
    };
    if tree.nodes[*index].attr("class") != "android.widget.Button" {
        return None;
    }
    if measured_global {
        let dialogs = tree.matching(
            package,
            ElementQuery::Description {
                value: "Dialog",
                exact: true,
            },
        );
        if !matches!(dialogs.as_slice(), [dialog] if tree.inside(*index, *dialog)) {
            return None;
        }
    }
    tree.nodes[*index]
        .rect()
        .filter(|r| r.enabled && r.clickable)
}

/// Positive blockers shared by pre-Post account proof and post-link verification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccountBlocker {
    LoginRequired,
    SecurityPrompt,
    UnrecognizedDialog,
}

fn exact_node(tree: &Tree, package: &str, id: &str, text: &str) -> Option<usize> {
    let matches = tree.matching(package, ElementQuery::ResourceIdSuffix(id));
    let [index] = matches.as_slice() else {
        return None;
    };
    let node = &tree.nodes[*index];
    (node.attr("text") == text && node.rect().is_some()).then_some(*index)
}

fn security_sheet(tree: &Tree, labels: TikTokControls) -> Option<usize> {
    if (
        labels.package(),
        labels.resource_version(),
        labels.language(),
    ) != ("com.ss.android.ugc.trill", Some("38.3.2"), "en")
    {
        return None;
    }
    let package = labels.package();
    let sheet = exact_node(tree, package, ":id/dox", "")?;
    if tree.nodes[sheet].attr("content-desc") != "Bottom sheet" {
        return None;
    }
    let heading = exact_node(
        tree,
        package,
        ":id/qwh",
        "Let's do a quick security checkup",
    )?;
    let body = exact_node(
        tree,
        package,
        ":id/doq",
        "Complete a few personalized security tips to strengthen the safety of your account.",
    )?;
    let next = exact_node(tree, package, ":id/jmt", "Continue")?;
    (tree.nodes[heading].attr("class") == "android.widget.TextView"
        && tree.nodes[body].attr("class") == "android.widget.TextView"
        && tree.nodes[next].attr("class") == "android.widget.Button"
        && [heading, body, next]
            .iter()
            .all(|index| tree.inside(*index, sheet)))
    .then_some(sheet)
}

/// Saved machine 12, Trill 38.3.2/en: the unlabeled Close is a unique Button
/// under ke4 inside the otb/ke7 header of dox. Never infer image coordinates.
pub fn security_reminder_close(tree: &Tree, labels: TikTokControls) -> Option<ElementBox> {
    let sheet = security_sheet(tree, labels)?;
    let package = labels.package();
    let header = exact_node(tree, package, ":id/otb", "")?;
    let group = exact_node(tree, package, ":id/ke7", "")?;
    let close_parent = exact_node(tree, package, ":id/ke4", "")?;
    if !tree.inside(header, sheet)
        || !tree.inside(group, header)
        || !tree.inside(close_parent, group)
    {
        return None;
    }
    let candidates: Vec<_> = tree
        .matching(package, ElementQuery::ClassName("android.widget.Button"))
        .into_iter()
        .filter(|index| {
            let node = &tree.nodes[*index];
            node.parent == Some(close_parent)
                && node.attr("text").is_empty()
                && node.attr("content-desc").is_empty()
                && node.attr("resource-id").is_empty()
        })
        .collect();
    let [index] = candidates.as_slice() else {
        return None;
    };
    tree.nodes[*index]
        .rect()
        .filter(|rect| rect.enabled && rect.clickable)
}

pub fn account_blocker(tree: &Tree, labels: TikTokControls) -> Option<AccountBlocker> {
    let package = labels.package();
    // Exact heading + separate login-method Button: a caption mentioning login
    // (including the complete heading) is not a login screen.
    let heading = tree.matching(
        package,
        ElementQuery::Text {
            value: "Log in to TikTok",
            exact: true,
        },
    );
    let login_heading = matches!(heading.as_slice(), [index] if
        tree.nodes[*index].attr("class") == "android.widget.TextView"
        && tree.nodes[*index].attr("resource-id").ends_with(":id/title")
        && tree.nodes[*index].rect().is_some());
    let login_method = tree
        .matching(package, ElementQuery::ClassName("android.widget.Button"))
        .into_iter()
        .any(|index| {
            let node = &tree.nodes[index];
            matches!(
                node.attr("content-desc"),
                "Use phone / email / username" | "Continue with Facebook" | "Continue with Google"
            ) && node.rect().is_some_and(|r| r.enabled && r.clickable)
        });
    if login_heading && login_method {
        return Some(AccountBlocker::LoginRequired);
    }
    if security_sheet(tree, labels).is_some() {
        return Some(AccountBlocker::SecurityPrompt);
    }
    // Preserve the existing measured negative-popup handlers.
    if decline_contacts(tree, labels).is_some()
        || decline_facebook_permission(tree, labels).is_some()
        || tree
            .control(
                package,
                labels.label(crate::tiktok_labels::TikTokControl::DialogDismiss),
            )
            .is_some()
    {
        return None;
    }
    tree.nodes
        .iter()
        .enumerate()
        .any(|(i, node)| {
            node.visible(package)
                && tree.ancestors_visible(i)
                && node.rect().is_some()
                && matches!(node.attr("content-desc"), "Dialog" | "Bottom sheet")
        })
        .then_some(AccountBlocker::UnrecognizedDialog)
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
    fn global_facebook_prompt_allows_only_the_unique_negative_button() {
        let package = "com.zhiliaoapp.musically";
        let labels = crate::tiktok_labels::controls_for(package, "en", "45.7.3").unwrap();
        let prompt = "\u{200e}\u{200e}Give TikTok access to your Facebook friends list and email?\u{200e} \u{2068}This will be used to improve your TikTok experience, including connecting you with people you may know, people you share connections with on Facebook, and personalizing your ads. Learn more in the Help Center\u{2069}";
        let xml = format!(
            r#"<hierarchy><node package="{package}" content-desc="Dialog" resource-id="{package}:id/visual_area" bounds="[172,732][907,1424]" displayed="true"><node package="{package}" class="android.widget.TextView" resource-id="{package}:id/esv" text="{prompt}" bounds="[225,795][843,1245]" displayed="true"/><node package="{package}" class="android.widget.Button" text="OK" enabled="true" clickable="true" bounds="[540,1299][907,1424]" displayed="true"/><node package="{package}" class="android.widget.Button" text="Don’t allow" enabled="true" clickable="true" bounds="[172,1299][539,1424]" displayed="true"/></node></hierarchy>"#
        );
        let button = decline_facebook_permission(&tree(xml.clone()), labels).unwrap();
        assert_eq!(
            (button.x, button.y, button.width, button.height),
            (172.0, 1299.0, 367.0, 125.0)
        );
        for changed in [
            xml.replace("friends list and email", "payment details"),
            xml.replace("personalizing your ads", "sharing your contacts"),
            xml.replace("Don’t allow", "Allow"),
            xml.replace("text=\"Don’t allow\" enabled=\"true\"", "text=\"Don’t allow\" enabled=\"false\""),
            xml.replace("content-desc=\"Dialog\"", "content-desc=\"Other\""),
            xml.replace("</hierarchy>", &format!(r#"<node package="{package}" class="android.widget.Button" text="Don’t allow" enabled="true" clickable="true" bounds="[1,1][2,2]" displayed="true"/></hierarchy>"#)),
        ] {
            assert!(decline_facebook_permission(&tree(changed), labels).is_none());
        }
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
