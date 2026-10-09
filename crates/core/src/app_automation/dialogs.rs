//! Measured negative popup actions shared by caller-owned automation stages.
use crate::{tiktok_labels::TikTokControls, ui_automation::tree::Tree, ElementBox, ElementQuery};

mod catalog;
use catalog::CATALOG;

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

/// Riviu #32 and #34 (SM-G955F, Android 9), Global 45.7.3/en, 2026-10-09: the OS contacts
/// question raised over TikTok's "Find contacts" banner while a target post opened
/// (acceptance-20261010 interaction-32-deeplink-feed.png). Screenshot only, no XML: the
/// question, Deny and Allow are those of the measured Android 9 location dialog above, so the
/// same OS ids, classes and container are required. Only Deny is ever pressed.
///
/// Box 31-50, 2026-10-10: the question came back on every TikTok start. From the second
/// request on, Android 9 adds AOSP PackageInstaller's "Don't ask again" checkbox
/// (`grant_permissions.xml`, `do_not_ask_checkbox`, string `never_ask_again`) and disables
/// Allow while it is ticked. Not yet in a measured dump, so the checkbox must be the one
/// visible, enabled, clickable, checkable node of that id inside the container. Unticked,
/// it is ticked first ([`OS_CONTACTS_DO_NOT_ASK_KIND`]); a fresh read must then show it
/// ticked before Deny ([`OS_CONTACTS_KIND`]) denies for good. Without it, Deny alone.
fn decline_os_contacts(tree: &Tree, labels: TikTokControls) -> Option<(u64, ElementBox)> {
    os_contacts_answer(tree, labels).map(|(kind, button, _)| (kind, button))
}

/// [`decline_os_contacts`] plus whether its Deny is permanent ("Don't ask again" ticked).
fn os_contacts_answer(tree: &Tree, labels: TikTokControls) -> Option<(u64, ElementBox, bool)> {
    if (
        labels.package(),
        labels.resource_version(),
        labels.language(),
    ) != ("com.zhiliaoapp.musically", Some("45.7.3"), "en")
    {
        return None;
    }
    let package = OS_PERMISSION_PACKAGE;
    let exact = |id: &str, text: &str, class: &str| {
        let matches = tree.matching(package, ElementQuery::ResourceIdSuffix(id));
        let [index] = matches.as_slice() else {
            return None;
        };
        let node = &tree.nodes[*index];
        (node.attr("resource-id") == id
            && node.attr("text") == text
            && node.attr("class") == class
            && node.visibility() == Some(true))
        .then_some(*index)
    };
    let container = exact(
        "com.android.packageinstaller:id/dialog_container",
        "",
        "android.widget.LinearLayout",
    )?;
    let prompt = exact(
        "com.android.packageinstaller:id/permission_message",
        "Allow TikTok to access your contacts?",
        "android.widget.TextView",
    )?;
    let deny = exact(
        "com.android.packageinstaller:id/permission_deny_button",
        "Deny",
        "android.widget.Button",
    )?;
    let allow = exact(
        "com.android.packageinstaller:id/permission_allow_button",
        "Allow",
        "android.widget.Button",
    )?;
    let do_not_ask = os_do_not_ask_checkbox(tree, container)?;
    let ticked = matches!(do_not_ask, Some((_, true)));
    if ![prompt, deny, allow]
        .into_iter()
        .all(|index| tree.inside(index, container))
        || tree.nodes[deny].parent != tree.nodes[allow].parent
        // Allow is disabled only while "Don't ask again" is ticked.
        || (!ticked
            && !tree.nodes[allow]
                .rect()
                .is_some_and(|rect| rect.enabled && rect.clickable))
    {
        return None;
    }
    if let Some((checkbox, false)) = do_not_ask {
        return tree.nodes[checkbox]
            .rect()
            .map(|rect| (OS_CONTACTS_DO_NOT_ASK_KIND, rect, false));
    }
    tree.nodes[deny]
        .rect()
        .filter(|rect| rect.enabled && rect.clickable)
        .map(|rect| (OS_CONTACTS_KIND, rect, ticked))
}

/// `Some(None)`: no "Don't ask again" on screen. `Some(Some((index, checked)))`: the one
/// checkbox, proven inside `container` and tappable. `None`: a duplicate, foreign or
/// unreadable checkbox, which authorizes nothing.
fn os_do_not_ask_checkbox(tree: &Tree, container: usize) -> Option<Option<(usize, bool)>> {
    let found: Vec<usize> = tree
        .matching(
            OS_PERMISSION_PACKAGE,
            ElementQuery::ResourceIdSuffix(":id/do_not_ask_checkbox"),
        )
        .into_iter()
        .filter(|index| tree.nodes[*index].visibility() != Some(false))
        .collect();
    let index = match found.as_slice() {
        [] => return Some(None),
        [index] => *index,
        _ => return None,
    };
    let node = &tree.nodes[index];
    let checked = match node.attr("checked") {
        "true" => true,
        "false" => false,
        _ => return None,
    };
    (matches!(
        node.attr("resource-id"),
        "com.android.packageinstaller:id/do_not_ask_checkbox"
            | "com.google.android.packageinstaller:id/do_not_ask_checkbox"
    ) && matches!(
        node.attr("text"),
        "Don't ask again" | "Don\u{2019}t ask again"
    ) && node.attr("class") == "android.widget.CheckBox"
        && node.attr("checkable") == "true"
        && node.visibility() == Some(true)
        && tree.ancestors_visible(index)
        && tree.inside(index, container)
        && node
            .rect()
            .is_some_and(|rect| rect.enabled && rect.clickable))
    .then_some(Some((index, checked)))
}

/// Android 9 runtime permission questions belong to this package; their ids keep the AOSP
/// `com.android.packageinstaller` prefix.
const OS_PERMISSION_PACKAGE: &str = "com.google.android.packageinstaller";

/// How long a permission-package foreground whose tree names no measured OS question is
/// re-read before it counts as a foreign window.
///
/// Box 31-50, 10/10/2026, nurture on ce031713aadf361905 (SM-G955F, Android 9, Global
/// 45.7.3/en; run box3150d, 21:34:38-39): one second after Deny on the first-time contacts
/// question (no "Don't ask again" yet) the foreground was still
/// `com.google.android.packageinstaller` but the tree matched no OS question: the dialog
/// was fading out, or the second one with the checkbox was still animating in. Five other
/// phones passed the same Deny. Such a read is a transition, not a verdict: within this
/// window the step re-reads foreground and tree, and acts only on a measured question or
/// on TikTok itself. An unrecognised permission tree is never tapped.
const OS_PERMISSION_TRANSITION_WINDOW: std::time::Duration = std::time::Duration::from_secs(3);
/// Gap between two reads inside [`OS_PERMISSION_TRANSITION_WINDOW`]; an Android 9 dialog
/// fade is shorter than this, so the second read already sees the settled screen.
const OS_PERMISSION_TRANSITION_POLL: std::time::Duration = std::time::Duration::from_millis(450);

/// Whether `candidate` is one of the measured OS permission questions, the only trees that
/// may own a `com.google.android.packageinstaller` foreground.
fn os_permission_candidate(candidate: Option<&(u64, ElementBox)>) -> bool {
    candidate.is_some_and(|(kind, _)| {
        matches!(
            *kind,
            OS_LOCATION_KIND | OS_CONTACTS_KIND | OS_CONTACTS_DO_NOT_ASK_KIND
        )
    })
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
    if first_run_profile(labels) {
        return global_security_check(tree, labels.package()).map(|(sheet, _)| sheet);
    }
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

const GLOBAL: &str = "com.zhiliaoapp.musically";
const TEXT_VIEW: &str = "android.widget.TextView";
const BUTTON: &str = "android.widget.Button";
/// The emulator's notification prompt is owned by this package; its ids keep the AOSP prefix.
const NOTIFICATION_PROMPT_PACKAGE: &str = "com.google.android.permissioncontroller";

/// First-run screens of Global 45.7.3. Interests, swipe-up guide and security sheet are
/// real English captures from Riviu phones 32/33 during reinstalls on 2026-10-09; phone 31
/// gave the same screens in Vietnamese the same day. The guest consent dialog and
/// notification prompt come from the emulator (English UI) in
/// target/tiktok-45.7.3-research REPORT.md §7; see crates/core/fixtures/tiktok-first-run/.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FirstRunScreen {
    Interests,
    SwipeUpGuide,
    SecurityCheck,
    NotificationPermission,
    TermsConsent,
    /// TikTok wants a code sent to the account's email. Only the operator can read it.
    EmailVerification,
}

/// The only action a recognised first-run screen permits. Never an affirmative.
#[derive(Debug, Clone)]
pub enum FirstRunPlan {
    Tap(ElementBox),
    Swipe(crate::SwipeGesture),
    /// Recognised, but only the operator may answer it.
    Operator,
}

impl FirstRunScreen {
    /// One attempt per caller budget; bits 1..8, 1 << 61 and 1 << 62 belong to
    /// [`optional_decline`] and bits from 256 up to [`CATALOG`].
    fn budget_bit(self) -> u64 {
        match self {
            Self::Interests => 16,
            Self::SwipeUpGuide => 32,
            Self::SecurityCheck => 64,
            Self::NotificationPermission => 128,
            Self::TermsConsent | Self::EmailVerification => 0,
        }
    }

    /// The operator-facing line once the bounded action has been sent.
    pub fn says(self) -> &'static str {
        match self {
            Self::Interests => "bỏ qua trang chọn chủ đề của TikTok (Skip, không chọn chủ đề nào); chờ đọc lại màn hình",
            Self::SwipeUpGuide => "vuốt lên một lần để tắt lớp hướng dẫn \"Vuốt lên để xem thêm\"; chờ đọc lại màn hình",
            Self::SecurityCheck => "đóng bảng kiểm tra bảo mật nhanh (bấm Đóng, không bấm Tiếp tục); chờ đọc lại màn hình",
            Self::NotificationPermission => "từ chối quyền gửi thông báo của TikTok; chờ đọc lại màn hình",
            Self::TermsConsent => "TikTok đang hỏi đồng ý Điều khoản dịch vụ — Riviu không tự đồng ý; cần người vận hành tự xem và bấm trên máy",
            Self::EmailVerification => EMAIL_CODE_SAYS,
        }
    }

    /// Why the screen is left to the operator: consent always, the rest once spent.
    pub fn operator_reason(self) -> &'static str {
        match self {
            Self::Interests => "trang chọn chủ đề vẫn còn sau một lần bấm Skip — không bấm lại, cần người vận hành kiểm tra",
            Self::SwipeUpGuide => "lớp hướng dẫn vuốt lên vẫn còn sau một lần vuốt — không vuốt lại, cần người vận hành kiểm tra",
            Self::SecurityCheck => "bảng kiểm tra bảo mật vẫn còn sau một lần bấm Đóng — không bấm Tiếp tục, cần người vận hành xử lý",
            Self::NotificationPermission => "hộp xin quyền thông báo vẫn còn sau một lần từ chối — không bấm lại, cần người vận hành kiểm tra",
            Self::TermsConsent | Self::EmailVerification => self.says(),
        }
    }
}

fn first_run_profile(labels: TikTokControls) -> bool {
    (
        labels.package(),
        labels.resource_version(),
        labels.language(),
    ) == (GLOBAL, Some("45.7.3"), "en")
}

/// The single node carrying exactly `package:id/<id>`, with the measured class and text.
fn measured_node(
    tree: &Tree,
    package: &str,
    id_package: &str,
    id: &str,
    class: Option<&str>,
    text: Option<&str>,
) -> Option<usize> {
    let full = format!("{id_package}:id/{id}");
    let found: Vec<_> = tree
        .matching(package, ElementQuery::ResourceIdSuffix(&full))
        .into_iter()
        .filter(|index| tree.nodes[*index].attr("resource-id") == full)
        .collect();
    let [index] = found.as_slice() else {
        return None;
    };
    let node = &tree.nodes[*index];
    (class.is_none_or(|class| node.attr("class") == class)
        && text.is_none_or(|text| node.attr("text") == text))
    .then_some(*index)
}

fn tappable(tree: &Tree, index: usize) -> Option<ElementBox> {
    tree.nodes[index]
        .rect()
        .filter(|rect| rect.enabled && rect.clickable)
}

/// The two interest pickers of an A/B test, both under tk8 with the same c8c bar: cqt
/// (Skip) and cnt (Next, whose count suffix is set from code). Skip selects no topic.
/// Variant A (Riviu phone 32, en): jg3 "Choose what you like" over jfw. Variant B (Riviu
/// phone 33, en): jg3 "Choose your interests" over ubo, with krq tiles labelled by krz.
fn interests_skip(tree: &Tree, package: &str) -> Option<ElementBox> {
    let root = measured_node(
        tree,
        package,
        package,
        "tk8",
        Some("android.view.ViewGroup"),
        None,
    )?;
    // (title, subtitle id, subtitle, whether krq tiles carry the topics)
    let variants = [
        (
            "Choose what you like",
            "jfw",
            "Your feed will be personalized based on what you like.",
            false,
        ),
        (
            "Choose your interests",
            "ubo",
            "Get better video recommendations",
            true,
        ),
    ];
    let (title, subtitle) =
        variants
            .into_iter()
            .find_map(|(heading, subtitle_id, subtitle, tiles)| {
                let title = measured_node(
                    tree,
                    package,
                    package,
                    "jg3",
                    Some(TEXT_VIEW),
                    Some(heading),
                )?;
                let subtitle = measured_node(
                    tree,
                    package,
                    package,
                    subtitle_id,
                    Some(TEXT_VIEW),
                    Some(subtitle),
                )?;
                (!tiles || interest_tile(tree, package, root)).then_some((title, subtitle))
            })?;
    let bar = measured_node(
        tree,
        package,
        package,
        "c8c",
        Some("android.view.ViewGroup"),
        None,
    )?;
    let skip = measured_node(tree, package, package, "cqt", Some(BUTTON), Some("Skip"))?;
    let next = measured_node(tree, package, package, "cnt", Some(BUTTON), None)?;
    (tree.nodes[title].parent == tree.nodes[subtitle].parent
        && [title, bar]
            .into_iter()
            .all(|index| tree.inside(index, root))
        && [skip, next]
            .into_iter()
            .all(|index| tree.nodes[index].parent == Some(bar)))
    .then(|| tappable(tree, skip))
    .flatten()
}

/// Variant B: a clickable krq ViewGroup tile whose krz TextView names a topic, in tk8.
fn interest_tile(tree: &Tree, package: &str, root: usize) -> bool {
    let tile_id = format!("{package}:id/krq");
    let label_id = format!("{package}:id/krz");
    tree.matching(package, ElementQuery::ResourceIdSuffix(&label_id))
        .into_iter()
        .any(|label| {
            let node = &tree.nodes[label];
            node.attr("resource-id") == label_id
                && node.attr("class") == TEXT_VIEW
                && !node.attr("text").is_empty()
                && node.parent.is_some_and(|tile| {
                    let tile_node = &tree.nodes[tile];
                    tile_node.attr("resource-id") == tile_id
                        && tile_node.attr("class") == "android.view.ViewGroup"
                        && tile_node.rect().is_some_and(|r| r.clickable)
                        && tree.inside(tile, root)
                })
        })
}

/// Phone 31: SwipeUpGuideStrengthenLayout (wmf > wmc) covers the feed page hpk and
/// swallows taps until one vertical swipe. The swipe stays inside that page and uses the
/// 0.72 -> 0.28 targets of the nurture feed swipe; hpk was [0,0][1080,2094] there.
fn swipe_up_guide(tree: &Tree, package: &str) -> Option<crate::SwipeGesture> {
    let guide = measured_node(
        tree,
        package,
        package,
        "tv_strengthen_swipe_up_guide",
        Some(TEXT_VIEW),
        Some("Swipe up for more"),
    )?;
    let overlay = measured_node(tree, package, package, "wmf", None, None)?;
    let content = measured_node(tree, package, package, "wmc", None, None)?;
    if tree.nodes[content].parent != Some(overlay) || !tree.inside(guide, content) {
        return None;
    }
    let page_id = format!("{package}:id/hpk");
    let mut page = tree.nodes[overlay].parent;
    while let Some(index) = page {
        if tree.nodes[index].attr("resource-id") == page_id {
            break;
        }
        page = tree.nodes[index].parent;
    }
    let page = tree.nodes[page?].rect()?;
    let x = page.x + page.width / 2.0;
    Some(crate::SwipeGesture {
        from: crate::TapPoint {
            x,
            y: page.y + page.height * 0.72,
        },
        to: crate::TapPoint {
            x,
            y: page.y + page.height * 0.28,
        },
        duration_ms: 300,
    })
}

/// After login: bottom sheet fl4 with xw7/fkw and Continue nri; the header Close is the
/// only Button described "Close", with no text and no id, under ovy (Riviu phone 33, en,
/// 2026-10-09; phone 31's vi capture reads "Đóng"). The sheet copy is server text absent
/// from the string table; the English strings are phone 33's.
fn global_security_check(tree: &Tree, package: &str) -> Option<(usize, ElementBox)> {
    let sheet = measured_node(
        tree,
        package,
        package,
        "fl4",
        Some("android.widget.FrameLayout"),
        None,
    )?;
    if tree.nodes[sheet].attr("content-desc") != "Bottom sheet" {
        return None;
    }
    let parts = [
        measured_node(tree, package, package, "xw7", Some(TEXT_VIEW), Some("Let's do a quick security checkup"))?,
        measured_node(
            tree,
            package,
            package,
            "fkw",
            Some(TEXT_VIEW),
            Some("Complete a few personalized security tips to strengthen the safety of your account."),
        )?,
        measured_node(tree, package, package, "nri", Some(BUTTON), Some("Continue"))?,
    ];
    let header = measured_node(
        tree,
        package,
        package,
        "ovy",
        Some("android.widget.LinearLayout"),
        None,
    )?;
    let closes = tree.matching(
        package,
        ElementQuery::Description {
            value: "Close",
            exact: true,
        },
    );
    let [close] = closes.as_slice() else {
        return None;
    };
    let node = &tree.nodes[*close];
    if !parts
        .into_iter()
        .chain([header])
        .all(|index| tree.inside(index, sheet))
        || node.parent != Some(header)
        || node.attr("class") != BUTTON
        || !node.attr("text").is_empty()
        || !node.attr("resource-id").is_empty()
    {
        return None;
    }
    Some((sheet, tappable(tree, *close)?))
}

/// Emulator, Android 17: the OS notification prompt's negative button.
fn decline_notifications(tree: &Tree) -> Option<ElementBox> {
    let package = NOTIFICATION_PROMPT_PACKAGE;
    let aosp = "com.android.permissioncontroller";
    let dialog = measured_node(
        tree,
        package,
        aosp,
        "grant_dialog",
        Some("android.widget.LinearLayout"),
        None,
    )?;
    let prompt = measured_node(
        tree,
        package,
        aosp,
        "permission_message",
        Some(TEXT_VIEW),
        Some("Allow TikTok to send you notifications?"),
    )?;
    let allow = measured_node(
        tree,
        package,
        aosp,
        "permission_allow_button",
        Some(BUTTON),
        Some("Allow"),
    )?;
    let deny = measured_node(
        tree,
        package,
        aosp,
        "permission_deny_button",
        Some(BUTTON),
        Some("Don’t allow"),
    )?;
    ([prompt, allow, deny]
        .into_iter()
        .all(|index| tree.inside(index, dialog))
        && tree.nodes[allow].parent == tree.nodes[deny].parent
        && tappable(tree, allow).is_some())
    .then(|| tappable(tree, deny))
    .flatten()
}

/// Terms consent offers only an affirmative. Recognise it, never answer it: the dialog
/// (emulator guest session; Riviu phones 37/38/42 after login, 2026-10-09) or the welcome
/// page shown before login.
fn terms_consent(tree: &Tree, package: &str) -> bool {
    terms_dialog(tree, package) || welcome_terms(tree, package)
}

/// Riviu phones, English UI, before login, 2026-10-09: read as text only, no XML capture.
/// ele "Welcome to TikTok" and elc "Videos to make your day" over elg (terms text, set from
/// code) and el8 "Agree and continue"; the 45.7.3 layout puts ele/elc under elf and
/// elg/el8 under el_, all in tk8. Recognition only, so el8's class is not required.
fn welcome_terms(tree: &Tree, package: &str) -> bool {
    let node = |id: &str, text: Option<&str>| measured_node(tree, package, package, id, None, text);
    let (Some(root), Some(heading), Some(tagline), Some(terms), Some(agree)) = (
        node("tk8", None),
        node("ele", Some("Welcome to TikTok")),
        node("elc", Some("Videos to make your day")),
        node("elg", None),
        node("el8", Some("Agree and continue")),
    ) else {
        return false;
    };
    tree.nodes[terms].attr("text").contains("Terms of Service")
        && tree.nodes[heading].parent == tree.nodes[tagline].parent
        && tree.nodes[terms].parent == tree.nodes[agree].parent
        && [heading, terms]
            .into_iter()
            .all(|index| tree.inside(index, root))
}

fn terms_dialog(tree: &Tree, package: &str) -> bool {
    let dialogs = tree.matching(
        package,
        ElementQuery::Description {
            value: "Dialog",
            exact: true,
        },
    );
    let [dialog] = dialogs.as_slice() else {
        return false;
    };
    let notice = measured_node(
        tree,
        package,
        package,
        "esv",
        Some(TEXT_VIEW),
        Some("By tapping “Agree and continue”, you agree to our Terms of Service and acknowledge that you have read our Privacy Policy to learn how we collect, use, and share your data."),
    );
    let agree = tree.matching(
        package,
        ElementQuery::Text {
            value: "Agree and continue",
            exact: true,
        },
    );
    tree.nodes[*dialog].attr("resource-id") == format!("{package}:id/visual_area")
        && notice.is_some_and(|index| tree.inside(index, *dialog))
        && matches!(agree.as_slice(), [index] if tree.inside(*index, *dialog)
            && tree.nodes[*index].attr("class") == BUTTON)
}

/// Operator line for TikTok's email verification pages, shared with the login stage.
pub const EMAIL_CODE_SAYS: &str = "TikTok đòi xác minh danh tính bằng mã gửi qua email — Riviu dừng, không bấm gửi mã và không nhập mã; cần người vận hành lấy mã trong email và nhập trên máy";

/// TikTok's email verification, a WebView page in the TikTok package. The method chooser
/// (Riviu phones 40/42/45/49, en, 2026-10-09, XML): "Verify it’s really you" and its body over an
/// "Email <masked address>" Button. The code page (phones 37/38/40, read as text only):
/// "Verify identity", the "code sent to" line and the `code-input` field. The masked
/// address is never compared. Recognition only: nothing here is ever tapped.
pub fn email_verification(tree: &Tree, package: &str) -> bool {
    let visible = |index: usize| {
        let node = &tree.nodes[index];
        node.visible(package) && tree.ancestors_visible(index) && node.rect().is_some()
    };
    let in_web_view = |index: usize| {
        let mut current = tree.nodes[index].parent;
        while let Some(at) = current {
            if tree.nodes[at].attr("class") == "android.webkit.WebView" {
                return true;
            }
            current = tree.nodes[at].parent;
        }
        false
    };
    let with_text = |text: &str| -> Vec<usize> {
        (0..tree.nodes.len())
            .filter(|index| visible(*index) && tree.nodes[*index].attr("text") == text)
            .collect()
    };
    // TikTok's own wording, typos included.
    let body = "Choose a method to verify your identify and make sure its really you logging in.";
    let chooser = matches!(with_text("Verify it\u{2019}s really you").as_slice(), [heading]
        if tree.nodes[*heading].attr("class") == TEXT_VIEW && in_web_view(*heading))
        && matches!(with_text(body).as_slice(), [index] if in_web_view(*index))
        && (0..tree.nodes.len()).any(|index| {
            let node = &tree.nodes[index];
            visible(index)
                && node.attr("class") == BUTTON
                && node.attr("text").starts_with("Email ")
                && in_web_view(index)
        });
    let code_page = !with_text("Verify identity").is_empty()
        && (0..tree.nodes.len()).any(|index| {
            visible(index)
                && tree.nodes[index]
                    .attr("text")
                    .starts_with("Verify your identity by entering the code sent to ")
        })
        && (0..tree.nodes.len()).any(|index| {
            let id = tree.nodes[index].attr("resource-id");
            visible(index) && (id == "code-input" || id.ends_with(":id/code-input"))
        });
    chooser || code_page
}

/// A modal above the picker or feed owns the screen; it is not this screen's to clear.
fn foreign_modal(tree: &Tree, package: &str) -> bool {
    ["Dialog", "Bottom sheet"].into_iter().any(|value| {
        !tree
            .matching(package, ElementQuery::Description { value, exact: true })
            .is_empty()
    })
}

/// Exactly one recognised first-run screen in this snapshot, or `None`. Unknown,
/// ambiguous or unmeasured screens stay with the existing fail-closed handling.
pub fn first_run_screen(
    tree: &Tree,
    labels: TikTokControls,
) -> Option<(FirstRunScreen, FirstRunPlan)> {
    if !first_run_profile(labels) {
        return None;
    }
    let package = labels.package();
    let mut found = Vec::new();
    if let Some(skip) = interests_skip(tree, package) {
        found.push((FirstRunScreen::Interests, FirstRunPlan::Tap(skip)));
    }
    if let Some(swipe) = swipe_up_guide(tree, package) {
        found.push((FirstRunScreen::SwipeUpGuide, FirstRunPlan::Swipe(swipe)));
    }
    if let Some((_, close)) = global_security_check(tree, package) {
        found.push((FirstRunScreen::SecurityCheck, FirstRunPlan::Tap(close)));
    }
    if let Some(deny) = decline_notifications(tree) {
        found.push((
            FirstRunScreen::NotificationPermission,
            FirstRunPlan::Tap(deny),
        ));
    }
    if terms_consent(tree, package) {
        found.push((FirstRunScreen::TermsConsent, FirstRunPlan::Operator));
    }
    if email_verification(tree, package) {
        found.push((FirstRunScreen::EmailVerification, FirstRunPlan::Operator));
    }
    let [(screen, plan)] = <[_; 1]>::try_from(found).ok()?;
    if matches!(
        screen,
        FirstRunScreen::Interests | FirstRunScreen::SwipeUpGuide
    ) && foreign_modal(tree, package)
    {
        return None;
    }
    Some((screen, plan))
}

/// Global 45.7.3 login chooser, Compose: the heading TextView has no resource-id and each
/// option is a clickable `android.view.View` whose child View carries the label as its
/// content-desc. Emulator 05-login-method-chooser (en) and Riviu phone 31 s5 (vi), both
/// under `tk8 > v6m`; target/apk-leverage-audit-20261009/login_notes.md.
fn compose_login_chooser(tree: &Tree, labels: TikTokControls) -> bool {
    if !first_run_profile(labels) {
        return false;
    }
    let package = labels.package();
    let Some(page) = measured_node(tree, package, package, "v6m", None, None) else {
        return false;
    };
    let headings = tree.matching(
        package,
        ElementQuery::Text {
            value: "Log in to TikTok",
            exact: true,
        },
    );
    let heading = matches!(headings.as_slice(), [index]
        if tree.nodes[*index].attr("class") == TEXT_VIEW
        && tree.nodes[*index].attr("resource-id").is_empty()
        && tree.inside(*index, page));
    heading
        && tree.nodes.iter().enumerate().any(|(index, node)| {
            node.visible(package)
                && tree.ancestors_visible(index)
                && node.attr("class") == "android.view.View"
                && matches!(
                    node.attr("content-desc"),
                    "Use phone / email / username"
                        | "Continue with Facebook"
                        | "Continue with Google"
                )
                && tree.inside(index, page)
                && node.parent.is_some_and(|option| {
                    let option = &tree.nodes[option];
                    option.attr("class") == "android.view.View"
                        && option.rect().is_some_and(|r| r.enabled && r.clickable)
                })
        })
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
    if (login_heading && login_method) || compose_login_chooser(tree, labels) {
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

const LOGIN_REQUIRED_SAYS: &str = "TikTok đang ở màn hình đăng nhập — Riviu không tự đăng nhập và không bấm Back; cần người vận hành đăng nhập lại trên máy";
const SECURITY_PROMPT_SAYS: &str = "TikTok đang hỏi kiểm tra bảo mật — Riviu không bấm Tiếp tục và không bấm Back; cần người vận hành xử lý trên máy";
const UNKNOWN_MODAL_SAYS: &str = "hộp thoại lạ đang che màn hình — Riviu không bấm gì, kể cả Back; cần người vận hành xem và đóng tay. Đã lưu XML (đã che chữ nhập) và ảnh màn hình vào thư mục artifacts/popups để rà soát";
/// Review-capture bit for an unknown modal; catalog bits stay below it.
const UNKNOWN_BIT: u64 = 1 << 63;

/// Whether a catalog entry may press its control. The promotion switch of each entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PopupMode {
    /// A real 45.7.3/en dump proved the whole popup: its one bounded action is allowed.
    Act,
    /// Known only from the APK: recognised, logged and captured, never pressed.
    Shadow,
}

/// What the popup asks of the account.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PopupClass {
    /// Not now, Later, Skip, Close, Got it on a pure tooltip, Don't allow on an optional
    /// OS permission: one press commits nothing.
    Dismissable,
    /// Agree, allow, continue, buy, follow, sync contacts; terms, age, captcha/OTP,
    /// verification, expired login; discarding user work or cancelling an upload.
    Operator,
}

/// One node of a catalog entry. Every given field must match exactly; `id` is the short
/// id under the TikTok package (`<package>:id/<id>`), a full id when it contains `:`
/// (`android:id/button3`), and `Some("")` requires no id.
#[derive(Debug, Clone, Copy)]
struct PopupPart {
    id: Option<&'static str>,
    class: Option<&'static str>,
    text: Option<&'static str>,
    desc: Option<&'static str>,
}

/// One in-app popup of Global 45.7.3/en. The container, every anchor and the control must
/// each be the only match in the snapshot, with the anchors and control inside the
/// container. `docs`: target/popup-catalog-45.7.3/CATALOG.md, by `key`.
#[derive(Debug, Clone, Copy)]
pub struct CatalogPopup {
    pub key: &'static str,
    pub mode: PopupMode,
    pub class: PopupClass,
    container: PopupPart,
    anchors: &'static [PopupPart],
    /// The one non-committal control. `None` for [`PopupClass::Operator`].
    dismiss: Option<PopupPart>,
    /// Operator line once the dismissal was sent.
    acted: &'static str,
    /// Operator line when Riviu leaves it: Operator entries, or a dismissal already spent.
    spent: &'static str,
    /// Logged in shadow mode: what was recognised and what would be pressed.
    shadow: &'static str,
}

enum CatalogMatch {
    None,
    Act {
        bit: u64,
        entry: &'static CatalogPopup,
        button: ElementBox,
    },
    Operator {
        entry: &'static CatalogPopup,
    },
    Shadow {
        bit: u64,
        entry: &'static CatalogPopup,
    },
    /// More than one entry that may act matched one snapshot.
    Ambiguous,
}

fn part_matches(tree: &Tree, package: &str, part: PopupPart) -> Vec<usize> {
    let id = part.id.map(|id| {
        if id.is_empty() || id.contains(':') {
            id.to_owned()
        } else {
            format!("{package}:id/{id}")
        }
    });
    tree.nodes
        .iter()
        .enumerate()
        .filter(|(index, node)| {
            node.visible(package)
                && tree.ancestors_visible(*index)
                && node.rect().is_some()
                && id
                    .as_deref()
                    .is_none_or(|id| node.attr("resource-id") == id)
                && part.class.is_none_or(|class| node.attr("class") == class)
                && part.text.is_none_or(|text| node.attr("text") == text)
                && part
                    .desc
                    .is_none_or(|desc| node.attr("content-desc") == desc)
        })
        .map(|(index, _)| index)
        .collect()
}

/// `Some(control)` when the entry's whole structure is present; the control is `None` for
/// an Operator entry and `Some(None)` never authorizes a tap.
fn catalog_entry(tree: &Tree, package: &str, entry: &CatalogPopup) -> Option<Option<ElementBox>> {
    let [container] = part_matches(tree, package, entry.container)[..] else {
        return None;
    };
    for anchor in entry.anchors {
        let [index] = part_matches(tree, package, *anchor)[..] else {
            return None;
        };
        if !tree.inside(index, container) {
            return None;
        }
    }
    let Some(dismiss) = entry.dismiss else {
        return Some(None);
    };
    let [control] = part_matches(tree, package, dismiss)[..] else {
        return None;
    };
    if !tree.inside(control, container) {
        return None;
    }
    Some(tappable(tree, control))
}

fn catalog_popup(tree: &Tree, labels: TikTokControls) -> CatalogMatch {
    if !first_run_profile(labels) {
        return CatalogMatch::None;
    }
    let package = labels.package();
    let mut acting = Vec::new();
    let mut shadow = None;
    for (index, entry) in CATALOG.iter().enumerate() {
        let Some(control) = catalog_entry(tree, package, entry) else {
            continue;
        };
        let bit = 1u64 << (8 + index);
        match (entry.mode, entry.class, control) {
            (PopupMode::Shadow, _, _) => {
                if shadow.is_none() {
                    shadow = Some(CatalogMatch::Shadow { bit, entry });
                }
            }
            (PopupMode::Act, PopupClass::Operator, _) => {
                acting.push(CatalogMatch::Operator { entry });
            }
            (PopupMode::Act, PopupClass::Dismissable, Some(button)) => {
                acting.push(CatalogMatch::Act { bit, entry, button });
            }
            // The structure is there but its control is not enabled and clickable: the
            // operator sees it rather than a guessed tap.
            (PopupMode::Act, PopupClass::Dismissable, None) => {
                acting.push(CatalogMatch::Operator { entry });
            }
        }
    }
    match acting.len() {
        0 => shadow.unwrap_or(CatalogMatch::None),
        1 => acting.pop().unwrap_or(CatalogMatch::Ambiguous),
        _ => CatalogMatch::Ambiguous,
    }
}

const _: () = assert!(
    CATALOG.len() <= 53,
    "catalog bits must stay below the OS contacts bits and UNKNOWN_BIT"
);

use crate::ui_automation::runtime::{read_before_deadline, ReadWaitResult};
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::time::Instant;

/// One caller-owned stage budget. An attempted decline is spent even if its ACK fails.
#[derive(Debug, Clone, Default)]
pub struct PopupBudget {
    epoch: Option<String>,
    spent: u64,
    last_action_generation: Option<u64>,
    says: Option<&'static str>,
    /// Popup kinds already captured for review under this budget, by the same bits.
    captured: u64,
}

impl PopupBudget {
    /// The operator line for the last step: the first-run action sent, or why a
    /// recognised screen was left to the operator. `None` for unknown blockers.
    pub fn says(&self) -> Option<&'static str> {
        self.says
    }
}

enum PopupGesture {
    Tap(ElementBox),
    Swipe(crate::SwipeGesture),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PopupStep {
    Clear,
    Dismissed,
    Blocked,
}

/// Negative actions shared across stages; provenance stays with each detector.
pub fn optional_decline(tree: &Tree, labels: TikTokControls) -> Option<(u64, ElementBox)> {
    decline_optional_location(tree, labels)
        .map(|button| (OS_LOCATION_KIND, button))
        .or_else(|| decline_contacts(tree, labels).map(|button| (2, button)))
        .or_else(|| decline_facebook_permission(tree, labels).map(|button| (4, button)))
        .or_else(|| decline_precise_location(tree, labels).map(|button| (8, button)))
        .or_else(|| decline_os_contacts(tree, labels))
}

/// [`optional_decline`] kinds raised by the OS permission package rather than TikTok. The
/// contacts kinds sit at the top of the budget, clear of the first-run bits (16..128) and
/// of the catalog bits below them.
const OS_LOCATION_KIND: u64 = 1;
const OS_CONTACTS_KIND: u64 = 1 << 62;
const OS_CONTACTS_DO_NOT_ASK_KIND: u64 = 1 << 61;

/// Operator lines for the contacts question: (once sent, once spent).
fn os_contacts_lines(kind: u64) -> Option<(&'static str, &'static str)> {
    match kind {
        OS_CONTACTS_DO_NOT_ASK_KIND => Some((
            "tích \"Don't ask again\" trên hộp thoại hệ thống hỏi quyền danh bạ (không bấm Allow); chờ đọc lại màn hình rồi bấm Deny",
            "hộp thoại hệ thống hỏi quyền danh bạ vẫn chưa nhận dấu tích \"Don't ask again\" — Riviu không bấm Allow; cần người vận hành kiểm tra trên máy",
        )),
        OS_CONTACTS_KIND => Some((
            "từ chối quyền danh bạ trên hộp thoại hệ thống (Deny, không bấm Allow); chờ đọc lại màn hình",
            "hộp thoại hệ thống hỏi quyền danh bạ vẫn còn sau khi đã bấm Deny — Riviu không bấm Allow; cần người vận hành kiểm tra trên máy",
        )),
        _ => None,
    }
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
    budget.says = None;
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
    // A permission-package foreground without a measured OS question is re-read for a
    // bounded window (see [`OS_PERMISSION_TRANSITION_WINDOW`]) under the same deadline,
    // Stop and epoch checks. The loop leaves with a settled read: the measured question
    // (then answered as usual), TikTok itself, or still unrecognised, which fails below
    // exactly as a foreign window does.
    let transition_started = Instant::now();
    let transition_until = (transition_started + OS_PERMISSION_TRANSITION_WINDOW).min(deadline);
    let mut transition_reads: u32 = 0;
    let (foreground, tree, candidate) = loop {
        let foreground = read!(session.active_app_bundle());
        let tree = Tree::parse(read!(session.hierarchy_source_snapshot()))?;
        if budget
            .last_action_generation
            .is_some_and(|generation| tree.generation <= generation)
        {
            return Ok(ReadWaitResult::Ready((PopupStep::Blocked, None)));
        }
        let candidate = optional_decline(&tree, labels);
        let settled = foreground != OS_PERMISSION_PACKAGE
            || os_permission_candidate(candidate.as_ref())
            || Instant::now() >= transition_until;
        if settled {
            if transition_reads > 0 {
                tracing::info!(
                    package = labels.package(),
                    foreground = %foreground,
                    recognised = os_permission_candidate(candidate.as_ref()),
                    reads = transition_reads,
                    elapsed_ms = transition_started.elapsed().as_millis() as u64,
                    generation = tree.generation,
                    "đọc lại màn hình sau khi hộp thoại quyền hệ thống chuyển cảnh"
                );
            }
            break (foreground, tree, candidate);
        }
        if transition_reads == 0 {
            tracing::info!(
                package = labels.package(),
                foreground = %foreground,
                generation = tree.generation,
                window_ms = OS_PERMISSION_TRANSITION_WINDOW.as_millis() as u64,
                "hộp thoại quyền hệ thống đang chuyển cảnh (chưa nhận ra nút nào) — không bấm gì; đọc lại màn hình"
            );
        }
        transition_reads = transition_reads.saturating_add(1);
        let pause = OS_PERMISSION_TRANSITION_POLL
            .min(transition_until.saturating_duration_since(Instant::now()));
        read!(async {
            tokio::time::sleep(pause).await;
            anyhow::Ok(())
        });
    };
    let first_run = candidate
        .is_none()
        .then(|| first_run_screen(&tree, labels))
        .flatten();
    let catalog = if candidate.is_none() && first_run.is_none() {
        catalog_popup(&tree, labels)
    } else {
        CatalogMatch::None
    };
    // The measured OS location and notification modals own foreground themselves. Every
    // other case requires the exact selected TikTok package, never a substring or recents.
    anyhow::ensure!(
        foreground == labels.package()
            || (foreground == OS_PERMISSION_PACKAGE && os_permission_candidate(candidate.as_ref()))
            || (foreground == NOTIFICATION_PROMPT_PACKAGE
                && matches!(first_run, Some((FirstRunScreen::NotificationPermission, _)))),
        "popup foreground does not match target package"
    );
    let fresh_foreground = read!(session.active_app_bundle());
    anyhow::ensure!(
        fresh_foreground == foreground,
        "popup foreground changed during observation"
    );
    // (bit, gesture, operator line once sent, operator line once spent)
    let (kind, gesture, lines) = match (candidate, first_run, catalog) {
        (Some((kind, button)), _, _) => (kind, PopupGesture::Tap(button), os_contacts_lines(kind)),
        (None, Some((screen, plan)), _) => {
            let gesture = match plan {
                FirstRunPlan::Tap(button) => PopupGesture::Tap(button),
                FirstRunPlan::Swipe(swipe) => PopupGesture::Swipe(swipe),
                FirstRunPlan::Operator => {
                    budget.says = Some(screen.operator_reason());
                    return Ok(ReadWaitResult::Ready((PopupStep::Blocked, None)));
                }
            };
            (
                screen.budget_bit(),
                gesture,
                Some((screen.says(), screen.operator_reason())),
            )
        }
        (None, None, CatalogMatch::Act { bit, entry, button }) => (
            bit,
            PopupGesture::Tap(button),
            Some((entry.acted, entry.spent)),
        ),
        (None, None, CatalogMatch::Operator { entry }) => {
            budget.says = Some(entry.spent);
            return Ok(ReadWaitResult::Ready((PopupStep::Blocked, None)));
        }
        (None, None, catalog) => {
            // A shadow entry is only explained: what was recognised and what would be
            // pressed once promoted. The screen keeps the verdict it had without it.
            let shadow = match catalog {
                CatalogMatch::Shadow { bit, entry } => {
                    // Once per budget: a popup that stays up is one review item, not a log
                    // line per poll.
                    if budget.captured & bit == 0 {
                        budget.captured |= bit;
                        tracing::warn!(
                            popup = entry.key,
                            package = labels.package(),
                            "{}",
                            entry.shadow
                        );
                        super::popup_evidence::capture(
                            session,
                            &tree,
                            labels,
                            entry.key,
                            entry.shadow,
                            deadline,
                            stop,
                        )
                        .await;
                    }
                    Some(entry)
                }
                _ => None,
            };
            let blocker = account_blocker(&tree, labels);
            let says = match blocker {
                Some(AccountBlocker::LoginRequired) => Some(LOGIN_REQUIRED_SAYS),
                Some(AccountBlocker::SecurityPrompt) => Some(SECURITY_PROMPT_SAYS),
                _ if modal_present(&tree) || matches!(catalog, CatalogMatch::Ambiguous) => {
                    Some(shadow.map_or(UNKNOWN_MODAL_SAYS, |entry| entry.shadow))
                }
                _ => None,
            };
            let Some(says) = says else {
                return Ok(ReadWaitResult::Ready((PopupStep::Clear, Some(tree))));
            };
            budget.says = Some(says);
            if shadow.is_none() && says == UNKNOWN_MODAL_SAYS && budget.captured & UNKNOWN_BIT == 0
            {
                budget.captured |= UNKNOWN_BIT;
                super::popup_evidence::capture(
                    session,
                    &tree,
                    labels,
                    "unknown-modal",
                    UNKNOWN_MODAL_SAYS,
                    deadline,
                    stop,
                )
                .await;
            }
            return Ok(ReadWaitResult::Ready((PopupStep::Blocked, None)));
        }
    };
    if effect_stop.load(Ordering::Relaxed) || budget.spent & kind != 0 {
        budget.says = lines
            .filter(|_| budget.spent & kind != 0)
            .map(|(_, spent)| spent);
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
    budget.says = lines.map(|(sent, _)| sent);
    if kind == OS_CONTACTS_KIND {
        // The answer is recorded before it is sent: Deny with "Don't ask again" ticked is
        // permanent; without the checkbox Android may ask again on the next start.
        let permanent_deny =
            os_contacts_answer(&tree, labels).is_some_and(|(_, _, permanent)| permanent);
        tracing::info!(
            popup = "os_contacts",
            package = labels.package(),
            permanentDeny = permanent_deny,
            generation = tree.generation,
            "bấm Deny trên hộp thoại hệ thống hỏi quyền danh bạ"
        );
    }
    match gesture {
        PopupGesture::Tap(button) => session.tap(button.centre()).await?,
        PopupGesture::Swipe(swipe) => session.swipe(swipe).await?,
    }
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
    fn global_in_app_precise_location_dialog_acts_only_on_not_now() {
        let source = include_str!(
            "../../fixtures/tiktok-publish/musically-45.7.3-en/real-20261009-precise-location-dialog.xml"
        );
        let labels =
            crate::tiktok_labels::controls_for("com.zhiliaoapp.musically", "en", "45.7.3").unwrap();
        assert!(optional_decline(&tree(source.into()), labels).is_none());
        let CatalogMatch::Act { entry, button, .. } = catalog_popup(&tree(source.into()), labels)
        else {
            panic!("the real precise-location capture must act");
        };
        assert_eq!(entry.key, "precise_location_dialog");
        assert_eq!(
            (button.x, button.y, button.width, button.height),
            (172.0, 1273.0, 367.0, 125.0)
        );
        for changed in [
            source.replace("text=\"Not now\"", "text=\"Later\""),
            source.replace("text=\"Allow\"", "text=\"OK\""),
            source.replace("device's approximate location", "device's contacts"),
            source.replace("id/zp_", "id/other"),
        ] {
            assert!(!matches!(
                catalog_popup(&tree(changed), labels),
                CatalogMatch::Act { .. }
            ));
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

    mod first_run {
        use super::*;
        const GLOBAL: &str = "com.zhiliaoapp.musically";
        const EN_INTERESTS: &str =
            include_str!("../../fixtures/tiktok-first-run/musically-45.7.3-en/interests.fixture");
        const EN_INTERESTS_B: &str = include_str!(
            "../../fixtures/tiktok-first-run/musically-45.7.3-en/interests-variant-b.fixture"
        );
        const EN_GUIDE: &str = include_str!(
            "../../fixtures/tiktok-first-run/musically-45.7.3-en/swipe-up-guide.fixture"
        );
        const EN_SECURITY: &str = include_str!(
            "../../fixtures/tiktok-first-run/musically-45.7.3-en/security-check.fixture"
        );
        const EN_NOTIFICATION: &str = include_str!(
            "../../fixtures/tiktok-first-run/musically-45.7.3-en/notification-permission.fixture"
        );
        const EN_CONSENT: &str = include_str!(
            "../../fixtures/tiktok-first-run/musically-45.7.3-en/terms-consent.fixture"
        );
        const VI_INTERESTS: &str =
            include_str!("../../fixtures/tiktok-first-run/musically-45.7.3-vi/interests.fixture");
        const VI_GUIDE: &str = include_str!(
            "../../fixtures/tiktok-first-run/musically-45.7.3-vi/swipe-up-guide.fixture"
        );
        const VI_SECURITY: &str = include_str!(
            "../../fixtures/tiktok-first-run/musically-45.7.3-vi/security-check.fixture"
        );

        fn global() -> TikTokControls {
            crate::tiktok_labels::controls_for(GLOBAL, "en", "45.7.3").unwrap()
        }
        fn screen(xml: &str) -> Option<(FirstRunScreen, FirstRunPlan)> {
            first_run_screen(&tree(xml.into()), global())
        }
        fn tap(xml: &str) -> Option<(FirstRunScreen, (f64, f64, f64, f64))> {
            match screen(xml)? {
                (screen, FirstRunPlan::Tap(b)) => Some((screen, (b.x, b.y, b.width, b.height))),
                _ => None,
            }
        }

        #[test]
        fn global_first_run_screens_permit_only_their_measured_negative_action() {
            for interests in [EN_INTERESTS, EN_INTERESTS_B] {
                assert_eq!(
                    tap(interests),
                    Some((FirstRunScreen::Interests, (53.0, 1936.0, 476.0, 137.0)))
                );
            }
            assert_eq!(
                tap(EN_SECURITY),
                Some((FirstRunScreen::SecurityCheck, (954.0, 1212.0, 105.0, 116.0)))
            );
            assert_eq!(
                tap(EN_NOTIFICATION),
                Some((
                    FirstRunScreen::NotificationPermission,
                    (133.0, 1421.0, 814.0, 126.0)
                ))
            );
            let Some((FirstRunScreen::SwipeUpGuide, FirstRunPlan::Swipe(swipe))) = screen(EN_GUIDE)
            else {
                panic!("swipe-up guide must plan exactly one swipe");
            };
            assert_eq!(
                (swipe.from.x, swipe.from.y, swipe.to.x, swipe.to.y),
                (540.0, 2094.0 * 0.72, 540.0, 2094.0 * 0.28)
            );
            assert!(matches!(
                screen(EN_CONSENT),
                Some((FirstRunScreen::TermsConsent, FirstRunPlan::Operator))
            ));
            let sheet = tree(EN_SECURITY.into());
            assert_eq!(
                account_blocker(&sheet, global()),
                Some(AccountBlocker::SecurityPrompt)
            );

            let duplicate = |xml: &str, node: &str| {
                xml.replacen("</hierarchy>", &format!("{node}</hierarchy>"), 1)
            };
            for changed in [
                EN_INTERESTS.replace("text=\"Skip\"", "text=\"Select all\""),
                EN_INTERESTS.replace("Choose what you like", "Choose who to follow"),
                EN_INTERESTS_B.replace("Get better video recommendations", "Follow these creators"),
                EN_INTERESTS_B.replace("id/krq", "id/other_tile"),
                EN_INTERESTS.replace("id/c8c", "id/other_bar"),
                EN_INTERESTS.replace(
                    r#"enabled="true" focusable="true" focused="false" scrollable="false" long-clickable="false" password="false" selected="false" bounds="[53,1936][529,2073]""#,
                    r#"enabled="false" focusable="true" focused="false" scrollable="false" long-clickable="false" password="false" selected="false" bounds="[53,1936][529,2073]""#,
                ),
                duplicate(EN_INTERESTS, r#"<node package="com.zhiliaoapp.musically" resource-id="com.zhiliaoapp.musically:id/cqt" class="android.widget.Button" text="Skip" clickable="true" enabled="true" bounds="[1,1][2,2]"/>"#),
                duplicate(EN_INTERESTS, r#"<node package="com.zhiliaoapp.musically" content-desc="Dialog" bounds="[100,100][900,900]"/>"#),
                EN_GUIDE.replace("Swipe up for more", "Tap to continue"),
                EN_GUIDE.replace("id/wmc", "id/other"),
                EN_GUIDE.replace("id/hpk", "id/other_page"),
                duplicate(EN_GUIDE, r#"<node package="com.zhiliaoapp.musically" resource-id="com.zhiliaoapp.musically:id/tv_strengthen_swipe_up_guide" class="android.widget.TextView" text="Swipe up for more" bounds="[1,1][2,2]"/>"#),
                EN_SECURITY.replace("content-desc=\"Close\"", "content-desc=\"Back\""),
                EN_SECURITY.replace("quick security checkup", "quick password reset"),
                EN_SECURITY.replace("text=\"Continue\"", "text=\"Verify\""),
                duplicate(EN_SECURITY, r#"<node package="com.zhiliaoapp.musically" class="android.widget.Button" content-desc="Close" clickable="true" enabled="true" bounds="[1,1][2,2]"/>"#),
                EN_NOTIFICATION.replace("send you notifications", "access your location"),
                EN_NOTIFICATION.replace("text=\"Don’t allow\"", "text=\"Allow\""),
                EN_NOTIFICATION.replace("com.google.android.permissioncontroller", "com.example.foreign"),
                EN_CONSENT.replace("Terms of Service", "Community Guidelines"),
                // The real Vietnamese captures never match the English profile.
                VI_INTERESTS.into(),
                VI_GUIDE.into(),
                VI_SECURITY.into(),
            ] {
                assert!(screen(&changed).is_none());
            }
            for (package, language, version) in [
                (GLOBAL, "en", "46.2.1"),
                (GLOBAL, "en", "45.4.3"),
                ("com.ss.android.ugc.trill", "en", "38.3.2"),
            ] {
                let other = crate::tiktok_labels::controls_for(package, language, version).unwrap();
                for xml in [
                    EN_INTERESTS,
                    EN_INTERESTS_B,
                    EN_GUIDE,
                    EN_SECURITY,
                    EN_NOTIFICATION,
                    EN_CONSENT,
                ] {
                    assert!(first_run_screen(&tree(xml.into()), other).is_none());
                }
            }
        }
    }
}
