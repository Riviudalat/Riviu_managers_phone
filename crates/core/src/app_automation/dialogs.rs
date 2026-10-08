//! Measured negative popup actions shared by caller-owned automation stages.
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

/// Machine 17, Trill 38.3.2/en on Android 9, 2026-10-06: optional location.
/// The OS package owns this modal; its resource IDs deliberately use the AOSP prefix.
/// Match the complete question and both buttons inside one measured container. Camera,
/// microphone and media permissions remain with the driver's required-permission path.
pub fn decline_optional_location(tree: &Tree, labels: TikTokControls) -> Option<ElementBox> {
    if (labels.package(), labels.resource_version(), labels.language())
        != ("com.ss.android.ugc.trill", Some("38.3.2"), "en")
    {
        return None;
    }
    let package = "com.google.android.packageinstaller";
    let exact = |id: &str, text: &str, class: &str| {
        let matches = tree.matching(package, ElementQuery::ResourceIdSuffix(id));
        let [index] = matches.as_slice() else { return None; };
        let node = &tree.nodes[*index];
        (node.attr("resource-id") == id && node.attr("text") == text
            && node.attr("class") == class && node.visibility() == Some(true))
            .then_some(*index)
    };
    let container = exact(
        "com.android.packageinstaller:id/dialog_container", "", "android.widget.LinearLayout",
    )?;
    let prompt = exact(
        "com.android.packageinstaller:id/permission_message",
        "Allow TikTok to access this device's location?", "android.widget.TextView",
    )?;
    let deny = exact(
        "com.android.packageinstaller:id/permission_deny_button", "Deny", "android.widget.Button",
    )?;
    let allow = exact(
        "com.android.packageinstaller:id/permission_allow_button", "Allow", "android.widget.Button",
    )?;
    if ![prompt, deny, allow].into_iter().all(|index| tree.inside(index, container))
        || tree.nodes[deny].parent != tree.nodes[allow].parent
        || !tree.nodes[allow].rect().is_some_and(|rect| rect.enabled && rect.clickable)
    {
        return None;
    }
    tree.nodes[deny].rect().filter(|rect| rect.enabled && rect.clickable)
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

use crate::ui_automation::runtime::{read_before_deadline, ReadWaitResult};
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::time::Instant;

/// One caller-owned stage budget. An attempted decline is spent even if its ACK fails.
#[derive(Debug, Clone, Default)]
pub struct PopupBudget {
    epoch: Option<String>,
    spent: u8,
    last_action_generation: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PopupStep {
    Clear,
    Dismissed,
    Blocked,
}

/// Negative actions shared across stages; provenance stays with each detector.
pub fn optional_decline(tree: &Tree, labels: TikTokControls) -> Option<(u8, ElementBox)> {
    decline_optional_location(tree, labels)
        .map(|button| (1, button))
        .or_else(|| decline_contacts(tree, labels).map(|button| (2, button)))
        .or_else(|| decline_facebook_permission(tree, labels).map(|button| (4, button)))
        .or_else(|| decline_precise_location(tree, labels).map(|button| (8, button)))
}

/// Screenshot-derived contract, NOT a measured XML/resource profile. A future
/// observation must prove the full modal before this rule can authorize decline.
fn decline_precise_location(tree: &Tree, labels: TikTokControls) -> Option<ElementBox> {
    let package = labels.package();
    if labels.language() != "en"
        || !matches!(
            (package, labels.resource_version()),
            ("com.ss.android.ugc.trill", Some("38.3.2"))
                | ("com.zhiliaoapp.musically", Some("45.7.3"))
        )
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
    let container = tree.nodes[*dialog].rect()?;
    if tree.nodes[*dialog].visibility() != Some(true) {
        return None;
    }
    let unique = |text: &str, class: &str| {
        let found = tree.matching(
            package,
            ElementQuery::Text {
                value: text,
                exact: true,
            },
        );
        let [index] = found.as_slice() else {
            return None;
        };
        let node = &tree.nodes[*index];
        let rect = node.rect()?;
        (tree.inside(*index, *dialog)
            && node.visibility() == Some(true)
            && node.attr("class") == class
            && rect.x >= container.x
            && rect.y >= container.y
            && rect.x + rect.width <= container.x + container.width
            && rect.y + rect.height <= container.y + container.height)
            .then_some((*index, rect))
    };
    let (heading, _) = unique("Turn on precise location", "android.widget.TextView")?;
    let negatives: Vec<_> = ["Don't allow", "Don\u{2019}t allow"]
        .into_iter()
        .flat_map(|text| {
            tree.matching(
                package,
                ElementQuery::Text {
                    value: text,
                    exact: true,
                },
            )
        })
        .collect();
    let [negative] = negatives.as_slice() else {
        return None;
    };
    let (deny, button) = unique(tree.nodes[*negative].attr("text"), "android.widget.Button")?;
    let (allow, affirmative) = unique("OK", "android.widget.Button")?;
    if button.x < affirmative.x + affirmative.width
        && affirmative.x < button.x + button.width
        && button.y < affirmative.y + affirmative.height
        && affirmative.y < button.y + button.height
    {
        return None;
    }
    if !button.enabled || !button.clickable || !affirmative.enabled || !affirmative.clickable {
        return None;
    }
    // A challenge, destructive decision, public-effect confirmation, or additional
    // actionable choice is not this optional popup, even with the same title.
    for (index, node) in tree.nodes.iter().enumerate() {
        if index != *dialog && !tree.inside(index, *dialog) {
            continue;
        }
        if node.visibility() == Some(false) || !tree.ancestors_visible(index) {
            continue;
        }
        if index != deny && index != allow && node.rect().is_some_and(|r| r.clickable) {
            return None;
        }
        let text = format!("{} {}", node.attr("text"), node.attr("content-desc")).to_lowercase();
        if [
            "captcha", "verify", "security", "log in", "delete", "discard", "terms", "publish",
            "post", "account",
        ]
        .iter()
        .any(|word| text.contains(word))
        {
            return None;
        }
        if !node.attr("text").is_empty()
            && index != heading
            && index != deny
            && index != allow
            && node.attr("class") == "android.widget.Button"
        {
            return None;
        }
    }
    Some(button)
}

fn modal_present(tree: &Tree) -> bool {
    tree.nodes.iter().enumerate().any(|(index, node)| {
        node.visibility() != Some(false)
            && tree.ancestors_visible(index)
            && node.rect().is_some()
            && (node.attr("content-desc") == "Dialog"
                || node.attr("resource-id") == "com.android.packageinstaller:id/dialog_container")
    })
}

/// Run before accepting an underlying navigation target. The caller must observe again
/// after Dismissed; it is not destination proof. Unknown modals never authorize Back.
/// No gesture is wrapped in a cancellable timeout and no public effect is dispatched.
pub async fn step_before_deadline(
    session: &dyn crate::UiSession,
    labels: TikTokControls,
    budget: &mut PopupBudget,
    deadline: Instant,
    stop: &AtomicBool,
    effect_stop: &AtomicBool,
) -> anyhow::Result<ReadWaitResult<PopupStep>> {
    Ok(match step_before_deadline_with_tree(
        session, labels, budget, deadline, stop, effect_stop,
    ).await? {
        ReadWaitResult::Ready((step, _)) => ReadWaitResult::Ready(step),
        ReadWaitResult::Cancelled => ReadWaitResult::Cancelled,
        ReadWaitResult::DeadlineExceeded => ReadWaitResult::DeadlineExceeded,
    })
}

/// Clear carries the same tree screened by the existing popup/foreground gates.
/// Dismissed and Blocked never expose an underlying action target.
pub(crate) async fn step_before_deadline_with_tree(
    session: &dyn crate::UiSession,
    labels: TikTokControls,
    budget: &mut PopupBudget,
    deadline: Instant,
    stop: &AtomicBool,
    effect_stop: &AtomicBool,
) -> anyhow::Result<ReadWaitResult<(PopupStep, Option<Tree>)>> {
    let epoch = session.gui_session_epoch();
    if let Some(bound) = &budget.epoch {
        anyhow::ensure!(bound == &epoch, "popup session changed");
    } else {
        budget.epoch = Some(epoch.clone());
    }
    macro_rules! read {
        ($future:expr) => {
            match read_before_deadline($future, deadline, stop).await? {
                ReadWaitResult::Ready(value) => {
                    anyhow::ensure!(
                        session.gui_session_epoch() == epoch,
                        "popup session changed"
                    );
                    value
                }
                ReadWaitResult::Cancelled => return Ok(ReadWaitResult::Cancelled),
                ReadWaitResult::DeadlineExceeded => return Ok(ReadWaitResult::DeadlineExceeded),
            }
        };
    }
    let foreground = read!(session.active_app_bundle());
    let tree = Tree::parse(read!(session.hierarchy_source_snapshot()))?;
    if budget
        .last_action_generation
        .is_some_and(|generation| tree.generation <= generation)
    {
        return Ok(ReadWaitResult::Ready((PopupStep::Blocked, None)));
    }
    let candidate = optional_decline(&tree, labels);
    // The measured OS location modal owns foreground itself. Every other case
    // requires the exact selected TikTok package, never a substring or recents.
    anyhow::ensure!(
        foreground == labels.package()
            || (foreground == "com.google.android.packageinstaller"
                && candidate.as_ref().is_some_and(|(kind, _)| *kind == 1)),
        "popup foreground does not match target package"
    );
    let fresh_foreground = read!(session.active_app_bundle());
    anyhow::ensure!(
        fresh_foreground == foreground,
        "popup foreground changed during observation"
    );
    let Some((kind, button)) = candidate else {
        let blocked = modal_present(&tree)
            || matches!(
                account_blocker(&tree, labels),
                Some(AccountBlocker::LoginRequired | AccountBlocker::SecurityPrompt)
            );
        return Ok(ReadWaitResult::Ready(if blocked {
            (PopupStep::Blocked, None)
        } else {
            (PopupStep::Clear, Some(tree))
        }));
    };
    if effect_stop.load(Ordering::Relaxed) || budget.spent & kind != 0 {
        return Ok(ReadWaitResult::Ready((PopupStep::Blocked, None)));
    }
    let fresh_foreground = read!(session.active_app_bundle());
    anyhow::ensure!(
        fresh_foreground == foreground,
        "popup foreground changed before decline"
    );
    if stop.load(Ordering::Relaxed) {
        return Ok(ReadWaitResult::Cancelled);
    }
    if Instant::now() >= deadline {
        return Ok(ReadWaitResult::DeadlineExceeded);
    }
    if effect_stop.load(Ordering::Relaxed) {
        return Ok(ReadWaitResult::Ready((PopupStep::Blocked, None)));
    }
    budget.spent |= kind;
    budget.last_action_generation = Some(tree.generation);
    session.tap(button.centre()).await?;
    anyhow::ensure!(
        session.gui_session_epoch() == epoch,
        "popup session changed after decline"
    );
    Ok(ReadWaitResult::Ready((PopupStep::Dismissed, None)))
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
    fn screenshot_precise_location_semantics_require_one_safe_modal() {
        // Synthetic contract only: title/buttons come from the screenshot; XML,
        // hierarchy, classes and bounds below are NOT a live measurement.
        let xml = r#"<hierarchy><node package="com.ss.android.ugc.trill" content-desc="Dialog" displayed="true" bounds="[100,200][900,1200]">
          <node package="com.ss.android.ugc.trill" class="android.widget.TextView" text="Turn on precise location" displayed="true" bounds="[150,250][850,350]"/>
          <node package="com.ss.android.ugc.trill" class="android.widget.Button" text="Don't allow" displayed="true" enabled="true" clickable="true" bounds="[150,950][450,1050]"/>
          <node package="com.ss.android.ugc.trill" class="android.widget.Button" text="OK" displayed="true" enabled="true" clickable="true" bounds="[550,950][850,1050]"/>
        </node></hierarchy>"#;
        for (package, version) in [(PACKAGE, "38.3.2"), ("com.zhiliaoapp.musically", "45.7.3")] {
            let labels = crate::tiktok_labels::controls_for(package, "en", version).unwrap();
            let xml = xml.replace(PACKAGE, package);
            let (_, button) = optional_decline(&tree(xml.clone()), labels)
                .expect("strict screenshot-derived negative action");
            assert_eq!((button.x, button.y), (150.0, 950.0));
            assert!(optional_decline(
                &tree(xml.replace("Don't allow", "Don\u{2019}t allow")),
                labels
            )
            .is_some());
            for changed in [
                xml.replace("Turn on precise location", "Confirm publication"),
                xml.replace("Don't allow", "Allow"),
                xml.replace("text=\"OK\"", "text=\"Post\""),
                xml.replace("enabled=\"true\"", "enabled=\"false\""),
                xml.replace("content-desc=\"Dialog\"", "content-desc=\"Background\""),
                xml.replace("[150,950][450,1050]", "[10,950][450,1050]"),
                xml.replace("</hierarchy>", &format!(r#"<node package="{package}" class="android.widget.Button" text="Don't allow" displayed="true" enabled="true" clickable="true" bounds="[150,950][450,1050]"/></hierarchy>"#)),
                xml.replace("text=\"Turn on precise location\"", "text=\"Turn on precise location\" content-desc=\"Verify your account\""),
            ] { assert!(optional_decline(&tree(changed), labels).is_none()); }
            assert!(crate::tiktok_labels::controls_for(package, "vi", version)
                .is_none_or(|other| optional_decline(&tree(xml), other).is_none()));
        }
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
