//! Log a TikTok Global 45.7.3/en phone out, and back in by username and password.
//!
//! This module drives screens; it owns no lease, writes no database row and never reads a
//! credential store. The desktop command that calls it holds one Repair lease for the whole
//! reinstall, records every stage, and hands the password in only for the one `type_text`
//! into the masked field.
//!
//! Provenance of every selector, because most of it is not an English device read yet:
//!
//! - **Login chooser and login form:** emulator captures with an English UI
//!   (`target/tiktok-45.7.3-research/emulator/05`, `07`), confirmed in structure by Riviu
//!   phone 31 with a Vietnamese UI (2026-10-09, `login31/s5`–`s8`). The chooser options are
//!   Compose `android.view.View`s whose child carries the label as content-desc. The form
//!   title is `q62` on the emulator and `ovm` on the phone; either is accepted.
//! - **Password step:** phone 31 only, Vietnamese UI (`s9`/`s10`): `bpr > esq` heading,
//!   a masked EditText without an id under `kky`, Continue `f79`. The English strings are
//!   the 45.7.3 string-table values for the same ids ("Enter password", "Password").
//! - **Own profile menu:** phone 31, Vietnamese UI (`s13`): a Button without an id described
//!   "Menu hồ sơ"; English "Profile menu" is the string-table value.
//! - **Logout path (menu sheet, Settings and privacy, Log out, confirmation):** strings only.
//!   No device has dumped it in any language. Every step therefore needs one exact,
//!   unique label and fails closed otherwise.
//! - **Failure screens (wrong password, too many attempts, code entry, missing account):**
//!   string-table values only. Captcha is server-rendered and has no string; it is
//!   recognised by a WebView or by its usual words and always stops.
//!
//! The rules every path keeps:
//!
//! - Each tap is chosen from a snapshot read immediately before it, in the foreground TikTok,
//!   and only when exactly one target matches. Nothing is tapped from a remembered point.
//! - The final "Continue" on the password step is tapped at most once per call. Whatever
//!   follows it — a code prompt, a captcha, an error, silence — is observed and reported,
//!   never answered and never submitted again.
//! - The password is passed to `type_text` and compared by length to the masked field. It is
//!   never formatted, logged or put into an error. Errors from the typing call are replaced
//!   with a fixed message, since a driver error could quote its request.

use std::sync::atomic::AtomicBool;
use std::time::Duration;

use serde::Serialize;
use tokio::time::Instant;

use crate::app_automation::dialogs::{self, PopupBudget, PopupStep};
use crate::tiktok_labels::{TikTokControl, TikTokControls};
use crate::ui_automation::runtime::ReadWaitResult;
use crate::ui_automation::tree::Tree;
use crate::{ElementBox, ElementQuery, SwipeGesture, TapPoint, UiSession};

pub const GLOBAL_PACKAGE: &str = "com.zhiliaoapp.musically";
pub const GLOBAL_VERSION: &str = "45.7.3";
/// `aapt2 dump badging` of the unmodified Global 45.7.3 APK, 2026-10-09
/// (`tools/tiktok-apk/verify_apk.py`).
pub const GLOBAL_VERSION_CODE: &str = "2024507030";
/// apksigner certificate SHA-256 of the same APK (`KNOWN_SIGNERS` in `verify_apk.py`).
pub const GLOBAL_SIGNER_SHA256: &str =
    "9041803e91bcb814b4b4399fb5c85a91640b755e5e8ba76813814bf4cf2ab5ba";

const TEXT_VIEW: &str = "android.widget.TextView";
const BUTTON: &str = "android.widget.Button";
const EDIT_TEXT: &str = "android.widget.EditText";
const COMPOSE_VIEW: &str = "android.view.View";

/// Polling interval between reads while waiting for a screen. Each read is a full
/// `/source`; 2.6 s was a cold `uiautomator dump` on phone 44, so faster polling only
/// queues reads behind each other.
const POLL: Duration = Duration::from_millis(700);
/// The settings list is long; "Log out" is at its end. Each swipe moves about half a screen.
const LOGOUT_SCROLL_BUDGET: usize = 10;

/// The only build and language this module drives.
pub fn supported(labels: TikTokControls) -> bool {
    (
        labels.package(),
        labels.resource_version(),
        labels.language(),
    ) == (GLOBAL_PACKAGE, Some(GLOBAL_VERSION), "en")
}

/// Why a login stopped. Every variant is final for the run: nothing here retries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum LoginStop {
    WrongPassword,
    TooManyAttempts,
    Verification,
    Captcha,
    AccountMissing,
    /// A screen this module does not recognise.
    Unrecognized,
}

impl LoginStop {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::WrongPassword => "wrongPassword",
            Self::TooManyAttempts => "tooManyAttempts",
            Self::Verification => "verification",
            Self::Captcha => "captcha",
            Self::AccountMissing => "accountMissing",
            Self::Unrecognized => "unrecognized",
        }
    }

    pub fn operator_reason(self) -> &'static str {
        match self {
            Self::WrongPassword => "TikTok báo sai tài khoản hoặc mật khẩu. Riviu không thử lại; kiểm tra rồi lưu lại mật khẩu đúng.",
            Self::TooManyAttempts => "TikTok báo đăng nhập quá nhiều lần. Riviu dừng, không thử lại; chờ rồi đăng nhập tay.",
            Self::Verification => "TikTok đòi mã xác minh (OTP/2 bước). Riviu dừng; người vận hành nhập mã trên máy.",
            Self::Captcha => "TikTok hiện captcha. Riviu dừng, không giải; người vận hành tự xử lý trên máy.",
            Self::AccountMissing => "TikTok báo tài khoản không tồn tại. Riviu dừng; kiểm tra lại username đã lưu.",
            Self::Unrecognized => "Gặp màn hình đăng nhập chưa nhận diện được. Riviu dừng, không bấm gì thêm; cần người vận hành xem máy.",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LogoutOutcome {
    /// Logged out, proven by the login screen afterwards.
    LoggedOut { previous_handle: Option<String> },
    /// The profile tab opened the login chooser: nobody was logged in.
    AlreadyLoggedOut,
    /// Log out was confirmed but no logged-out screen was read afterwards.
    Unverified {
        previous_handle: Option<String>,
        reason: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoginOutcome {
    /// The submit led to the logged-in app (feed, profile or the security sheet).
    LoggedIn,
    /// Stopped before the password was submitted: nothing was sent to TikTok's login.
    StoppedBeforeSubmit(String),
    /// Submitted once; TikTok answered with something Riviu must not answer.
    Rejected(LoginStop),
    /// Submitted once; nothing conclusive was read before the deadline.
    Uncertain(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FirstRunOutcome {
    /// The feed's Profile tab is visible with no popup over it.
    FeedReached,
    /// A recognised screen the operator must answer, or an unknown modal.
    Blocked(String),
}

// ---------------------------------------------------------------------------------------
// Classifiers. Pure functions over one snapshot; `None` means "not this screen, or not
// unambiguously".
// ---------------------------------------------------------------------------------------

fn id(package: &str, id: &str) -> String {
    format!("{package}:id/{id}")
}

/// The single visible node with exactly this resource id.
fn unique_id(tree: &Tree, package: &str, name: &str) -> Option<usize> {
    let full = id(package, name);
    let found: Vec<_> = tree
        .matching(package, ElementQuery::ResourceIdSuffix(&full))
        .into_iter()
        .filter(|index| tree.nodes[*index].attr("resource-id") == full)
        .collect();
    match found.as_slice() {
        [index] => Some(*index),
        _ => None,
    }
}

fn unique_text(tree: &Tree, package: &str, text: &str) -> Option<usize> {
    match tree
        .matching(
            package,
            ElementQuery::Text {
                value: text,
                exact: true,
            },
        )
        .as_slice()
    {
        [index] => Some(*index),
        _ => None,
    }
}

fn unique_desc(tree: &Tree, package: &str, desc: &str) -> Option<usize> {
    // `Description` matching is case-insensitive; the labels here are compared exactly.
    let found: Vec<_> = tree
        .matching(
            package,
            ElementQuery::Description {
                value: desc,
                exact: true,
            },
        )
        .into_iter()
        .filter(|index| tree.nodes[*index].attr("content-desc") == desc)
        .collect();
    match found.as_slice() {
        [index] => Some(*index),
        _ => None,
    }
}

fn tappable(tree: &Tree, index: usize) -> Option<ElementBox> {
    tree.nodes[index]
        .rect()
        .filter(|rect| rect.enabled && rect.clickable)
}

/// The node itself when it is clickable, else its nearest clickable ancestor within
/// `levels`. A text row's click target is usually its container.
fn click_target(tree: &Tree, index: usize, levels: usize) -> Option<ElementBox> {
    let mut current = Some(index);
    for _ in 0..=levels {
        let at = current?;
        if let Some(rect) = tappable(tree, at) {
            return Some(rect);
        }
        current = tree.nodes[at].parent;
    }
    None
}

fn has_text(tree: &Tree, package: &str, text: &str) -> bool {
    !tree
        .matching(
            package,
            ElementQuery::Text {
                value: text,
                exact: true,
            },
        )
        .is_empty()
}

/// Own profile (phone 31, `s13`): the `s0v` handle Button reads `@<handle>`, and the
/// top-right Button without an id is described "Profile menu" ("Menu hồ sơ" there).
pub fn own_profile_menu(tree: &Tree, package: &str) -> Option<ElementBox> {
    let handle = unique_id(tree, package, "s0v")?;
    if !tree.nodes[handle].attr("text").starts_with('@') {
        return None;
    }
    let menu = unique_desc(tree, package, "Profile menu")?;
    let node = &tree.nodes[menu];
    if node.attr("class") != BUTTON || !node.attr("resource-id").is_empty() {
        return None;
    }
    tappable(tree, menu)
}

/// Strings only: the profile menu sheet's "Settings and privacy" row.
pub fn settings_and_privacy_row(tree: &Tree, package: &str) -> Option<ElementBox> {
    // The sheet opens over the own profile; without it this is another screen.
    unique_id(tree, package, "s0v")?;
    click_target(tree, unique_text(tree, package, "Settings and privacy")?, 3)
}

/// Strings only: on the "Settings and privacy" page, the one "Log out" row.
pub fn log_out_row(tree: &Tree, package: &str) -> Option<ElementBox> {
    if !has_text(tree, package, "Settings and privacy")
        || has_text(tree, package, "Are you sure you want to log out?")
    {
        return None;
    }
    click_target(tree, unique_text(tree, package, "Log out")?, 3)
}

/// Strings only: the confirmation. One question, one "Log out" and one "Cancel" sharing a
/// parent. Returns the "Log out" button.
pub fn log_out_confirmation(tree: &Tree, package: &str) -> Option<ElementBox> {
    unique_text(tree, package, "Are you sure you want to log out?")?;
    let confirm = unique_text(tree, package, "Log out")?;
    let cancel = unique_text(tree, package, "Cancel")?;
    if tree.nodes[confirm].parent != tree.nodes[cancel].parent {
        return None;
    }
    click_target(tree, confirm, 1)
}

/// The Compose login chooser ("Log in to TikTok"): emulator 05 (en) and phone 31 `s5` (vi).
/// Returns the clickable row of "Use phone / email / username".
pub fn login_chooser(tree: &Tree, package: &str) -> Option<ElementBox> {
    let root = unique_id(tree, package, "v6m")?;
    let heading = unique_text(tree, package, "Log in to TikTok")?;
    let label = unique_desc(tree, package, "Use phone / email / username")?;
    let row = tree.nodes[label].parent?;
    if tree.nodes[heading].attr("class") != TEXT_VIEW
        || tree.nodes[label].attr("class") != COMPOSE_VIEW
        || tree.nodes[row].attr("class") != COMPOSE_VIEW
        || !tree.inside(heading, root)
        || !tree.inside(row, root)
    {
        return None;
    }
    tappable(tree, row)
}

/// What the login form shows right now.
#[derive(Debug, Clone)]
pub struct LoginForm {
    /// The "Email / Username" tab when it is not selected yet.
    pub email_tab: Option<ElementBox>,
    /// The identifier field and its current text, once the email tab is showing.
    pub field: Option<(ElementBox, String)>,
    pub next: ElementBox,
}

/// The username form (emulator 07, phone 31 `s6`–`s8`): tabs `q60` with `emr > text1`,
/// an EditText without an id under `gtm`, Continue `f79`.
pub fn login_form(tree: &Tree, package: &str) -> Option<LoginForm> {
    if unique_id(tree, package, "esq").is_some() {
        return None;
    }
    let titled = ["q62", "ovm"].into_iter().any(|name| {
        unique_id(tree, package, name)
            .is_some_and(|index| tree.nodes[index].attr("text") == "Log in")
    });
    let tabs = unique_id(tree, package, "q60")?;
    let tab = unique_desc(tree, package, "Email / Username")?;
    let next = unique_id(tree, package, "f79")?;
    if !titled
        || !tree.inside(tab, tabs)
        || tree.nodes[next].attr("class") != BUTTON
        || tree.nodes[next].attr("text") != "Continue"
    {
        return None;
    }
    let next = tree.nodes[next].rect()?;
    if tree.nodes[tab].attr("selected") != "true" {
        return Some(LoginForm {
            email_tab: Some(tappable(tree, tab)?),
            field: None,
            next,
        });
    }
    let page = unique_id(tree, package, "gtm")?;
    let fields: Vec<_> = tree
        .matching(package, ElementQuery::ClassName(EDIT_TEXT))
        .into_iter()
        .filter(|index| {
            tree.inside(*index, page) && tree.nodes[*index].attr("resource-id").is_empty()
        })
        .collect();
    let [field] = fields.as_slice() else {
        return None;
    };
    let text = tree.nodes[*field].attr("text");
    // An empty field reports its hint as text.
    let text = if text == "Email or username" {
        ""
    } else {
        text
    };
    Some(LoginForm {
        email_tab: None,
        field: Some((tree.nodes[*field].rect()?, text.to_owned())),
        next,
    })
}

#[derive(Debug, Clone)]
pub struct PasswordStep {
    pub field: ElementBox,
    /// Characters in the masked field; 0 while the hint shows.
    pub masked_chars: usize,
    pub next: ElementBox,
}

/// The password step (phone 31 `s9`/`s10`, Vietnamese UI): `bpr > esq` "Enter password",
/// one masked EditText without an id under `kky`, Continue `f79`.
///
/// `masked_chars` is all this returns about the field's content: never its text.
pub fn password_step(tree: &Tree, package: &str) -> Option<PasswordStep> {
    let heading = unique_id(tree, package, "esq")?;
    let header = unique_id(tree, package, "bpr")?;
    let group = unique_id(tree, package, "kky")?;
    let next = unique_id(tree, package, "f79")?;
    if tree.nodes[heading].attr("text") != "Enter password"
        || !tree.inside(heading, header)
        || tree.nodes[next].attr("class") != BUTTON
        || tree.nodes[next].attr("text") != "Continue"
    {
        return None;
    }
    let fields: Vec<_> = tree
        .matching(package, ElementQuery::ClassName(EDIT_TEXT))
        .into_iter()
        .filter(|index| {
            let node = &tree.nodes[*index];
            tree.inside(*index, group)
                && node.attr("resource-id").is_empty()
                && node.attr("password") == "true"
        })
        .collect();
    let [field] = fields.as_slice() else {
        return None;
    };
    let text = tree.nodes[*field].attr("text");
    let masked_chars = if text == "Password" {
        0
    } else {
        text.chars().count()
    };
    Some(PasswordStep {
        field: tree.nodes[*field].rect()?,
        masked_chars,
        next: tree.nodes[next].rect()?,
    })
}

/// Screens that end a login attempt. Strings from the 45.7.3 string table; captcha by
/// shape. Scans every visible node, because these arrive as toasts, dialogs or pages.
pub fn login_stop(tree: &Tree, package: &str) -> Option<LoginStop> {
    let mut found = None;
    for (index, node) in tree.nodes.iter().enumerate() {
        if node.visibility() == Some(false) || !tree.ancestors_visible(index) {
            continue;
        }
        if node.attr("class") == "android.webkit.WebView" && node.attr("package") == package {
            return Some(LoginStop::Captcha);
        }
        for value in [node.attr("text"), node.attr("content-desc")] {
            if value.is_empty() {
                continue;
            }
            let lower = value.to_lowercase();
            let stop = if lower.contains("captcha")
                || lower.contains("puzzle")
                || lower.contains("drag the slider")
            {
                Some(LoginStop::Captcha)
            } else if value.starts_with("Too many attempts") {
                Some(LoginStop::TooManyAttempts)
            } else if matches!(
                value,
                "Incorrect account or password" | "Incorrect password"
            ) {
                Some(LoginStop::WrongPassword)
            } else if value.starts_with("Enter 6-digit code")
                || value.starts_with("Enter your 6-digit code")
                || value.starts_with("Enter the 6-digit code")
                || value == "Enter code"
                || value == "2-step verification"
                || lower.starts_with("verify it")
            {
                Some(LoginStop::Verification)
            } else if value.starts_with("This account doesn't exist")
                || value == "Account doesn\u{2019}t exist"
            {
                Some(LoginStop::AccountMissing)
            } else {
                None
            };
            // The most restrictive reading wins; a captcha over an error is a captcha.
            if stop == Some(LoginStop::Captcha) {
                return stop;
            }
            found = found.or(stop);
        }
    }
    found
}

/// The logged-in app after a submit: the Profile tab is on screen and no login screen is.
fn logged_in_shell(tree: &Tree, labels: TikTokControls) -> bool {
    let package = labels.package();
    let profile = labels
        .label(TikTokControl::ProfileTab)
        .is_some_and(|label| !tree.matching(package, label.to_query()).is_empty());
    profile
        && login_chooser(tree, package).is_none()
        && login_form(tree, package).is_none()
        && password_step(tree, package).is_none()
}

fn profile_tab(tree: &Tree, labels: TikTokControls) -> Option<ElementBox> {
    let label = labels.label(TikTokControl::ProfileTab)?;
    match tree.matching(labels.package(), label.to_query()).as_slice() {
        [index] => click_target(tree, *index, 1),
        _ => None,
    }
}

// ---------------------------------------------------------------------------------------
// Session steps.
// ---------------------------------------------------------------------------------------

/// One fresh snapshot of the foreground TikTok. A read with another app in front is an
/// error, not a screen: nothing may be chosen from it.
async fn read(session: &dyn UiSession, labels: TikTokControls) -> anyhow::Result<Tree> {
    let foreground = session.active_app_bundle().await?;
    anyhow::ensure!(
        foreground == labels.package(),
        "ứng dụng đang mở là {foreground}, không phải TikTok"
    );
    let tree = Tree::parse(session.hierarchy_source_snapshot().await?)?;
    let after = session.active_app_bundle().await?;
    anyhow::ensure!(
        after == foreground,
        "ứng dụng phía trước đổi trong lúc đọc màn hình"
    );
    Ok(tree)
}

/// Read until `pick` recognises the screen or the deadline passes.
async fn wait_for<T>(
    session: &dyn UiSession,
    labels: TikTokControls,
    deadline: Instant,
    mut pick: impl FnMut(&Tree) -> Option<T>,
) -> anyhow::Result<Option<(T, Tree)>> {
    loop {
        let tree = read(session, labels).await?;
        if let Some(found) = pick(&tree) {
            return Ok(Some((found, tree)));
        }
        if Instant::now() + POLL >= deadline {
            return Ok(None);
        }
        tokio::time::sleep(POLL).await;
    }
}

/// Re-read and tap what `pick` finds on that fresh read. Refuses when the target moved
/// since the read that decided to tap it.
async fn tap_fresh(
    session: &dyn UiSession,
    labels: TikTokControls,
    decided: &ElementBox,
    pick: impl Fn(&Tree) -> Option<ElementBox>,
    what: &str,
) -> anyhow::Result<()> {
    let tree = read(session, labels).await?;
    let current = pick(&tree).ok_or_else(|| anyhow::anyhow!("{what} vừa biến mất; không bấm"))?;
    anyhow::ensure!(
        current.x == decided.x
            && current.y == decided.y
            && current.width == decided.width
            && current.height == decided.height,
        "{what} vừa đổi chỗ; không bấm"
    );
    session.tap(current.centre()).await
}

/// Log out through Profile → Profile menu → Settings and privacy → Log out → confirm.
///
/// Every tap before the confirmation is navigation. The confirmation is tapped once; its
/// result is read and reported, never repeated.
pub async fn log_out(
    session: &dyn UiSession,
    labels: TikTokControls,
    deadline: Instant,
) -> anyhow::Result<LogoutOutcome> {
    anyhow::ensure!(
        supported(labels),
        "chỉ hỗ trợ TikTok Global 45.7.3 tiếng Anh"
    );
    let package = labels.package();
    session.launch_app_foreground(package).await?;
    tokio::time::sleep(Duration::from_secs(2)).await;
    if let Err(error) = crate::tiktok_share::navigate_own_profile(session, &labels).await {
        let tree = read(session, labels).await?;
        if login_chooser(&tree, package).is_some() {
            return Ok(LogoutOutcome::AlreadyLoggedOut);
        }
        return Err(error.context("chưa mở được trang Hồ sơ"));
    }
    let header_deadline = (Instant::now() + Duration::from_secs(6)).min(deadline);
    let previous_handle = loop {
        let observed = crate::tiktok_account::restore_own_profile_header(session, labels).await?;
        if observed.is_some() || Instant::now() >= header_deadline {
            break observed;
        }
        tokio::time::sleep(POLL).await;
    };
    let step_deadline = |seconds| (Instant::now() + Duration::from_secs(seconds)).min(deadline);

    let Some((menu, _)) = wait_for(session, labels, step_deadline(6), |tree| {
        login_chooser(tree, package)
            .map(|_| None)
            .or_else(|| own_profile_menu(tree, package).map(Some))
    })
    .await?
    else {
        anyhow::bail!("không thấy nút Profile menu trên trang Hồ sơ");
    };
    let Some(menu) = menu else {
        return Ok(LogoutOutcome::AlreadyLoggedOut);
    };
    tap_fresh(
        session,
        labels,
        &menu,
        |tree| own_profile_menu(tree, package),
        "nút Profile menu",
    )
    .await?;

    let Some((row, _)) = wait_for(session, labels, step_deadline(8), |tree| {
        settings_and_privacy_row(tree, package)
    })
    .await?
    else {
        anyhow::bail!("menu hồ sơ không có đúng một mục Settings and privacy");
    };
    tap_fresh(
        session,
        labels,
        &row,
        |tree| settings_and_privacy_row(tree, package),
        "mục Settings and privacy",
    )
    .await?;

    let mut entry = None;
    for _ in 0..=LOGOUT_SCROLL_BUDGET {
        anyhow::ensure!(Instant::now() < deadline, "hết thời gian tìm mục Log out");
        if let Some((found, _)) = wait_for(session, labels, step_deadline(3), |tree| {
            log_out_row(tree, package)
        })
        .await?
        {
            entry = Some(found);
            break;
        }
        let (width, height) = session.window_size().await?;
        // Inside the list, clear of the toolbar and the gesture bar.
        session
            .swipe(SwipeGesture {
                from: TapPoint {
                    x: width / 2.0,
                    y: height * 0.75,
                },
                to: TapPoint {
                    x: width / 2.0,
                    y: height * 0.35,
                },
                duration_ms: 400,
            })
            .await?;
        tokio::time::sleep(POLL).await;
    }
    let entry = entry
        .ok_or_else(|| anyhow::anyhow!("không tìm thấy mục Log out trong Settings and privacy"))?;
    tap_fresh(
        session,
        labels,
        &entry,
        |tree| log_out_row(tree, package),
        "mục Log out",
    )
    .await?;

    let Some((confirm, _)) = wait_for(session, labels, step_deadline(6), |tree| {
        log_out_confirmation(tree, package)
    })
    .await?
    else {
        anyhow::bail!("không thấy hộp xác nhận Log out");
    };
    tap_fresh(
        session,
        labels,
        &confirm,
        |tree| log_out_confirmation(tree, package),
        "nút Log out xác nhận",
    )
    .await?;

    // The one effect of this function has been sent. From here on only reads.
    let proof_deadline = step_deadline(20);
    loop {
        let tree = match read(session, labels).await {
            Ok(tree) => tree,
            Err(error) => {
                return Ok(LogoutOutcome::Unverified {
                    previous_handle,
                    reason: format!("{error:#}"),
                })
            }
        };
        if login_chooser(&tree, package).is_some()
            || unique_id(&tree, package, "o5u").is_some()
            || has_text(&tree, package, "Log in to TikTok")
        {
            return Ok(LogoutOutcome::LoggedOut { previous_handle });
        }
        if Instant::now() + POLL >= proof_deadline {
            return Ok(LogoutOutcome::Unverified {
                previous_handle,
                reason: "đã bấm Log out nhưng chưa thấy màn hình đăng nhập".into(),
            });
        }
        tokio::time::sleep(POLL).await;
    }
}

/// Launch a freshly installed TikTok and pass its first-run screens through the shared
/// popup step: Skip on interests, one swipe on the guide, Don't allow on notifications.
/// Terms consent and anything unknown go to the operator.
pub async fn pass_first_run(
    session: &dyn UiSession,
    labels: TikTokControls,
    deadline: Instant,
) -> anyhow::Result<FirstRunOutcome> {
    anyhow::ensure!(
        supported(labels),
        "chỉ hỗ trợ TikTok Global 45.7.3 tiếng Anh"
    );
    session.launch_app_foreground(labels.package()).await?;
    tokio::time::sleep(Duration::from_secs(3)).await;
    let stop = AtomicBool::new(false);
    let effect_stop = AtomicBool::new(false);
    let mut budget = PopupBudget::default();
    // Interests, guide, notification prompt and a margin for re-reads after each.
    for _ in 0..24 {
        let step = dialogs::step_before_deadline(
            session,
            labels,
            &mut budget,
            deadline,
            &stop,
            &effect_stop,
        )
        .await?;
        match step {
            ReadWaitResult::Ready(PopupStep::Dismissed) => {
                tokio::time::sleep(POLL).await;
            }
            ReadWaitResult::Ready(PopupStep::Blocked) => {
                return Ok(FirstRunOutcome::Blocked(
                    budget
                        .says()
                        .unwrap_or("TikTok đang hiện một hộp thoại chưa nhận diện; Riviu không bấm, cần người vận hành xem máy")
                        .to_owned(),
                ));
            }
            ReadWaitResult::Ready(PopupStep::Clear) => {
                let tree = read(session, labels).await?;
                if profile_tab(&tree, labels).is_some() {
                    return Ok(FirstRunOutcome::FeedReached);
                }
                tokio::time::sleep(POLL).await;
            }
            ReadWaitResult::Cancelled | ReadWaitResult::DeadlineExceeded => break,
        }
    }
    Ok(FirstRunOutcome::Blocked(
        "hết thời gian chờ TikTok qua các màn hình lần đầu; cần người vận hành xem máy".into(),
    ))
}

/// Type into the focused field. The driver's error is dropped on purpose: it could quote
/// the request that carried `text`.
async fn type_into(
    session: &dyn UiSession,
    field: &ElementBox,
    text: &str,
    what: &'static str,
) -> anyhow::Result<()> {
    session.tap(field.centre()).await?;
    tokio::time::sleep(Duration::from_millis(400)).await;
    session
        .type_text(text)
        .await
        .map_err(|_| anyhow::anyhow!("không nhập được {what} vào ô (chi tiết lỗi được ẩn)"))
}

/// From the logged-out feed: Profile → "Use phone / email / username" → Email / Username
/// tab → username → Continue → password → Continue **once**.
///
/// `before_submit` runs after every check passed and immediately before the one submit
/// tap; it records the intent. If it fails, nothing is submitted.
pub async fn log_in(
    session: &dyn UiSession,
    labels: TikTokControls,
    username: &str,
    password: &str,
    deadline: Instant,
    before_submit: impl FnOnce() -> anyhow::Result<()>,
) -> anyhow::Result<LoginOutcome> {
    use LoginOutcome::StoppedBeforeSubmit as Before;
    anyhow::ensure!(
        supported(labels),
        "chỉ hỗ trợ TikTok Global 45.7.3 tiếng Anh"
    );
    let package = labels.package();
    let step_deadline = |seconds| (Instant::now() + Duration::from_secs(seconds)).min(deadline);

    // 1. Profile tab of the logged-out feed opens the chooser.
    let tree = read(session, labels).await?;
    if login_chooser(&tree, package).is_none() {
        let Some(tab) = profile_tab(&tree, labels) else {
            return Ok(Before(
                "không thấy tab Profile để mở màn hình đăng nhập".into(),
            ));
        };
        tap_fresh(
            session,
            labels,
            &tab,
            |tree| profile_tab(tree, labels),
            "tab Profile",
        )
        .await?;
    }
    let Some((choice, _)) = wait_for(session, labels, step_deadline(10), |tree| {
        login_chooser(tree, package)
    })
    .await?
    else {
        return Ok(Before(
            "không thấy màn hình \"Log in to TikTok\" với lựa chọn Use phone / email / username"
                .into(),
        ));
    };
    tap_fresh(
        session,
        labels,
        &choice,
        |tree| login_chooser(tree, package),
        "lựa chọn Use phone / email / username",
    )
    .await?;

    // 2. Email / Username tab.
    let Some((form, _)) = wait_for(session, labels, step_deadline(10), |tree| {
        login_form(tree, package)
    })
    .await?
    else {
        return Ok(Before(
            "không thấy biểu mẫu đăng nhập (tab Email / Username)".into(),
        ));
    };
    let form = if let Some(tab) = form.email_tab {
        tap_fresh(
            session,
            labels,
            &tab,
            |tree| login_form(tree, package).and_then(|form| form.email_tab),
            "tab Email / Username",
        )
        .await?;
        match wait_for(session, labels, step_deadline(6), |tree| {
            login_form(tree, package).filter(|form| form.field.is_some())
        })
        .await?
        {
            Some((form, _)) => form,
            None => return Ok(Before("tab Email / Username không mở".into())),
        }
    } else {
        form
    };
    let Some((field, current)) = form.field else {
        return Ok(Before("không thấy ô username".into()));
    };
    if !current.is_empty() {
        return Ok(Before(
            "ô username đã có sẵn chữ; Riviu không xóa nội dung lạ, cần người vận hành xem máy"
                .into(),
        ));
    }

    // 3. Username, read back exactly.
    type_into(session, &field, username, "username").await?;
    let Some((form, _)) = wait_for(session, labels, step_deadline(5), |tree| {
        login_form(tree, package).filter(|form| {
            form.field
                .as_ref()
                .is_some_and(|(_, text)| text == username)
                && form.next.enabled
        })
    })
    .await?
    else {
        return Ok(Before(
            "ô username không hiện đúng username đã nhập hoặc nút Continue chưa bật".into(),
        ));
    };
    let next = form.next;
    tap_fresh(
        session,
        labels,
        &next,
        |tree| {
            login_form(tree, package)
                .filter(|form| {
                    form.field
                        .as_ref()
                        .is_some_and(|(_, text)| text == username)
                })
                .map(|form| form.next)
                .filter(|next| next.enabled)
        },
        "nút Continue sau username",
    )
    .await?;

    // 4. Password step, or a stop that arrived before it.
    let Some((step, _)) = wait_for(session, labels, step_deadline(20), |tree| {
        login_stop(tree, package)
            .map(Err)
            .or_else(|| password_step(tree, package).map(Ok))
    })
    .await?
    else {
        return Ok(Before("không thấy bước nhập mật khẩu sau username".into()));
    };
    let step = match step {
        Ok(step) => step,
        Err(stop) => return Ok(Before(stop.operator_reason().into())),
    };
    if step.masked_chars != 0 {
        return Ok(Before(
            "ô mật khẩu đã có sẵn chữ; Riviu không xóa nội dung lạ, cần người vận hành xem máy"
                .into(),
        ));
    }
    let expected = password.chars().count();
    type_into(session, &step.field, password, "mật khẩu").await?;
    let Some((step, _)) = wait_for(session, labels, step_deadline(5), |tree| {
        password_step(tree, package)
            .filter(|step| step.masked_chars == expected && step.next.enabled)
    })
    .await?
    else {
        return Ok(Before(
            "ô mật khẩu không nhận đủ số ký tự đã nhập hoặc nút Continue chưa bật".into(),
        ));
    };

    // 5. The one submit. Intent first; a failed record means no submit.
    before_submit()?;
    tap_fresh(
        session,
        labels,
        &step.next,
        |tree| {
            password_step(tree, package)
                .filter(|step| step.masked_chars == expected && step.next.enabled)
                .map(|step| step.next)
        },
        "nút Continue sau mật khẩu",
    )
    .await
    .map_err(|error| error.context("bước gửi mật khẩu"))?;

    // 6. Observe only.
    let observe_deadline = step_deadline(45);
    let mut last_error = None;
    loop {
        match read(session, labels).await {
            Ok(tree) => {
                // The shell first: a feed caption may contain any word, and the stops
                // below are read only while a login screen still owns the display.
                if logged_in_shell(&tree, labels)
                    || dialogs::first_run_screen(&tree, labels)
                        .is_some_and(|(screen, _)| screen == dialogs::FirstRunScreen::SecurityCheck)
                {
                    return Ok(LoginOutcome::LoggedIn);
                }
                if let Some(stop) = login_stop(&tree, package) {
                    return Ok(LoginOutcome::Rejected(stop));
                }
            }
            Err(error) => last_error = Some(format!("{error:#}")),
        }
        if Instant::now() + POLL >= observe_deadline {
            return Ok(LoginOutcome::Uncertain(match last_error {
                Some(error) => format!("đã gửi đăng nhập một lần; chưa đọc được kết quả: {error}"),
                None => "đã gửi đăng nhập một lần; chưa thấy kết quả rõ ràng (không phải trang chủ, không phải thông báo lỗi)".into(),
            }));
        }
        tokio::time::sleep(POLL).await;
    }
}

/// After login: close the security checkup once (Close, never Continue) and any measured
/// optional popup, until the Profile tab is clear. Unknown modals go to the operator.
pub async fn close_post_login_sheets(
    session: &dyn UiSession,
    labels: TikTokControls,
    deadline: Instant,
) -> anyhow::Result<FirstRunOutcome> {
    let stop = AtomicBool::new(false);
    let effect_stop = AtomicBool::new(false);
    let mut budget = PopupBudget::default();
    for _ in 0..12 {
        match dialogs::step_before_deadline(
            session,
            labels,
            &mut budget,
            deadline,
            &stop,
            &effect_stop,
        )
        .await?
        {
            ReadWaitResult::Ready(PopupStep::Dismissed) => tokio::time::sleep(POLL).await,
            ReadWaitResult::Ready(PopupStep::Blocked) => {
                return Ok(FirstRunOutcome::Blocked(
                    budget
                        .says()
                        .unwrap_or("sau đăng nhập TikTok hiện một hộp thoại chưa nhận diện; Riviu không bấm, cần người vận hành xem máy")
                        .to_owned(),
                ))
            }
            ReadWaitResult::Ready(PopupStep::Clear) => {
                let tree = read(session, labels).await?;
                if profile_tab(&tree, labels).is_some() {
                    return Ok(FirstRunOutcome::FeedReached);
                }
                if let Some(stop) = login_stop(&tree, labels.package()) {
                    return Ok(FirstRunOutcome::Blocked(stop.operator_reason().into()));
                }
                tokio::time::sleep(POLL).await;
            }
            ReadWaitResult::Cancelled | ReadWaitResult::DeadlineExceeded => break,
        }
    }
    Ok(FirstRunOutcome::Blocked(
        "hết thời gian chờ trang chủ sau đăng nhập; cần người vận hành xem máy".into(),
    ))
}
