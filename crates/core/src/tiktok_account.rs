//! Read-only account proof for a measured own-profile screen. Never changes the login.
use crate::tiktok_labels::TikTokControls;
use crate::{ElementBox, ElementQuery, UiSession};

use crate::ui_automation::tree::Tree;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AccountState {
    Proved,
    LoginRequired,
    SecurityPrompt,
    UnrecognizedDialog,
    Unreadable,
    Mismatch,
    TransportReadFailed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountDiagnostic {
    pub state: AccountState,
    pub expected_account: Option<String>,
    pub observed_account: Option<String>,
    pub package: String,
    pub generation: Option<u64>,
    pub message: String,
    #[serde(skip)]
    transport_failure: Option<crate::publish_recovery::RecoveryFailure>,
}
impl std::fmt::Display for AccountDiagnostic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}
impl std::error::Error for AccountDiagnostic {}
impl AccountDiagnostic {
    pub fn new(
        state: AccountState,
        labels: TikTokControls,
        generation: Option<u64>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            state,
            expected_account: None,
            observed_account: None,
            package: labels.package().into(),
            generation,
            message: message.into(),
            transport_failure: None,
        }
    }
    pub fn failure(&self) -> Option<crate::publish_recovery::RecoveryFailure> {
        use crate::publish_recovery::{FailureKind, RecoveryFailure};
        let code = match self.state {
            AccountState::Proved => return None,
            AccountState::LoginRequired => "account_login_required",
            AccountState::SecurityPrompt => "account_security_prompt",
            AccountState::Mismatch => "account_mismatch",
            AccountState::TransportReadFailed => "account_read_failed",
            AccountState::UnrecognizedDialog | AccountState::Unreadable => "account_unreadable",
        };
        let kind = self
            .transport_failure
            .as_ref()
            .map(|failure| failure.kind)
            .filter(|_| self.state == AccountState::TransportReadFailed)
            .unwrap_or(FailureKind::Terminal);
        Some(RecoveryFailure::new(code, kind, self.message.clone()))
    }
    pub fn read_failed(labels: TikTokControls, error: &anyhow::Error) -> Self {
        let failure = crate::publish_recovery::describe(error);
        let mut diagnostic = Self::new(
            AccountState::TransportReadFailed,
            labels,
            None,
            format!("{error:#}"),
        );
        diagnostic.transport_failure = Some(failure);
        diagnostic
    }
    pub fn record(&self) {
        let _ = ACCOUNT_DIAGNOSTIC.try_with(|slot| *slot.borrow_mut() = Some(self.clone()));
    }
    pub fn into_error(self) -> anyhow::Error {
        self.record();
        self.into()
    }
}

tokio::task_local! { static ACCOUNT_DIAGNOSTIC: std::cell::RefCell<Option<AccountDiagnostic>>; }
pub fn current_account_diagnostic() -> Option<AccountDiagnostic> {
    ACCOUNT_DIAGNOSTIC
        .try_with(|slot| slot.borrow().clone())
        .ok()
        .flatten()
}

/// Keep structured evidence through legacy composer outcomes without changing
/// their effect-boundary contract. Each assignment has its own async task scope.
pub async fn with_account_diagnostic<T>(
    future: impl std::future::Future<Output = T>,
) -> (T, Option<AccountDiagnostic>) {
    ACCOUNT_DIAGNOSTIC
        .scope(std::cell::RefCell::new(None), async {
            let result = future.await;
            (
                result,
                ACCOUNT_DIAGNOSTIC.with(|slot| slot.borrow().clone()),
            )
        })
        .await
}

pub fn blocker_diagnostic(tree: &Tree, labels: TikTokControls) -> Option<AccountDiagnostic> {
    use crate::app_automation::dialogs::{account_blocker, AccountBlocker};
    let (state, message) = match account_blocker(tree, labels)? {
        AccountBlocker::LoginRequired => (
            AccountState::LoginRequired,
            "TikTok yêu cầu đăng nhập; chưa xác minh tài khoản, không Đăng.",
        ),
        AccountBlocker::SecurityPrompt => (
            AccountState::SecurityPrompt,
            "TikTok đang hiện nhắc bảo mật; chưa xác minh tài khoản, không Đăng.",
        ),
        AccountBlocker::UnrecognizedDialog => (
            AccountState::UnrecognizedDialog,
            "Hộp thoại chưa được nhận diện; không tự đóng hoặc Đăng.",
        ),
    };
    Some(AccountDiagnostic::new(
        state,
        labels,
        Some(tree.generation),
        message,
    ))
}

pub async fn account_snapshot(
    session: &dyn UiSession,
    labels: TikTokControls,
) -> anyhow::Result<Tree> {
    let snapshot = session
        .hierarchy_source_snapshot()
        .await
        .map_err(|error| AccountDiagnostic::read_failed(labels, &error).into_error())?;
    let generation = snapshot.generation;
    Tree::parse(snapshot).map_err(|error| {
        AccountDiagnostic::new(
            AccountState::Unreadable,
            labels,
            Some(generation),
            format!("Cây giao diện tài khoản không đọc được: {error:#}"),
        )
        .into_error()
    })
}

pub async fn refuse_account_blocker(
    session: &dyn UiSession,
    labels: TikTokControls,
) -> anyhow::Result<()> {
    let tree = account_snapshot(session, labels).await?;
    if let Some(diagnostic) = blocker_diagnostic(&tree, labels) {
        return Err(diagnostic.into_error());
    }
    Ok(())
}

fn single_username(elements: &[ElementBox]) -> Option<String> {
    let [element] = elements else {
        return None;
    };
    let text = element.description.as_deref()?.trim();
    let handle = text.strip_prefix('@')?;
    (!handle.is_empty()
        && handle.len() <= 24
        && !handle.ends_with('.')
        && handle
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'.')
        && element.width > 0.0
        && element.height > 0.0)
        .then(|| handle.to_owned())
}

pub fn account_read_supported(labels: TikTokControls) -> bool {
    if labels.adaptive() {
        return true;
    }
    if labels.package() == "com.zhiliaoapp.musically" && labels.language() == "en" {
        return global_username_id(labels.resource_version()).is_some();
    }
    matches!(
        (
            labels.package(),
            labels.resource_version(),
            labels.language(),
        ),
        ("com.ss.android.ugc.trill", Some("38.3.2"), "en")
            | ("com.zhiliaoapp.musically", Some("45.7.3"), "en")
    )
}

fn global_username_id(version: Option<&str>) -> Option<&'static str> {
    match version? {
        "45.4.3" => Some("rq1"),
        "45.7.3" => Some("s0v"),
        "46.0.41" => Some("s7b"),
        "46.1.3" => Some("s8i"),
        "46.2.1" => Some("scn"),
        // AGENTS.md §9.210: ce0517155ab38c390d own Profile snapshot, 10/09/2026.
        "46.2.42" => Some("sao"),
        "46.4.3" => Some("sj8"),
        _ => None,
    }
}

// Measured 08/09/2026, Global 45.7.3/en, SM-G955F Android 9: Edit and Profile menu
// are Buttons without ids; only the username has :id/s0v. All three must belong to
// one source read. Separate locator requests could combine two different screens.
fn global_profile_from_snapshot_with_id(
    xml: &str,
    username_id: &str,
) -> anyhow::Result<Option<String>> {
    let tree = crate::ui_automation::tree::Tree::parse(crate::HierarchySourceSnapshot {
        generation: 1,
        xml: xml.into(),
    })?;
    let mut edits = 0;
    let mut menus = 0;
    let mut bio_edits = 0;
    let mut selected_profiles = 0;
    let mut usernames = Vec::new();
    for (index, node) in tree.nodes.iter().enumerate() {
        if !node.visible("com.zhiliaoapp.musically")
            || !tree.ancestors_visible(index)
            || node.attr("enabled") != "true"
            || node.attr("clickable") != "true"
        {
            continue;
        }
        if username_id == "s0v" {
            let bio = node.attr("class") == "android.widget.Button"
                && node.attr("resource-id") == "com.zhiliaoapp.musically:id/rv2"
                && node.attr("text") == "Add bio"
                && node.attr("content-desc") == "Add bio";
            let selected_profile = node.attr("class") == "android.widget.FrameLayout"
                && node.attr("resource-id") == "com.zhiliaoapp.musically:id/nrb"
                && node.attr("content-desc") == "Profile"
                && node.attr("selected") == "true";
            if bio || selected_profile {
                if node.rect().is_none() {
                    return Ok(None);
                }
                bio_edits += usize::from(bio);
                selected_profiles += usize::from(selected_profile);
            }
        }
        if node.attr("class") != "android.widget.Button" {
            continue;
        }
        let description = node.attr("content-desc");
        let edit = node.attr("text") == "Edit";
        let menu = description == "Profile menu";
        let username =
            node.attr("resource-id") == format!("com.zhiliaoapp.musically:id/{username_id}");
        if edit || menu || username {
            let Some(rect) = node.rect() else {
                return Ok(None);
            };
            edits += usize::from(edit);
            menus += usize::from(menu);
            if username {
                usernames.push(rect);
            }
        }
    }
    Ok(((edits == 1 && menus == 1)
        || (username_id == "s0v" && bio_edits == 1 && selected_profiles == 1))
        .then(|| single_username(&usernames))
        .flatten())
}

fn profile_from_tree(tree: &Tree, labels: TikTokControls) -> Option<String> {
    let mut own = 0;
    let mut menu = false;
    let mut usernames = Vec::new();
    for (index, node) in tree.nodes.iter().enumerate() {
        if !node.visible(labels.package())
            || !tree.ancestors_visible(index)
            || node.attr("enabled") != "true"
            || node.rect().is_none()
        {
            continue;
        }
        if labels.adaptive() {
            own += usize::from(matches!(
                node.attr("text"),
                "Edit" | "Edit profile" | "Sửa hồ sơ"
            ));
            menu |= matches!(node.attr("content-desc"), "Profile menu" | "Menu hồ sơ");
            if node.attr("text").starts_with('@') && node.attr("class") == "android.widget.Button" {
                if let Some(rect) = node.rect() {
                    usernames.push(rect);
                }
            }
        } else {
            // Trill own marker and mjf username must be in THIS snapshot.
            own += usize::from(
                node.attr("resource-id") == "com.ss.android.ugc.trill:id/dby"
                    && node.attr("class") == "android.widget.TextView"
                    && node.attr("text") == "Edit profile",
            );
            if node.attr("resource-id") == "com.ss.android.ugc.trill:id/mjf"
                && node.attr("class") == "android.widget.Button"
            {
                if let Some(rect) = node.rect() {
                    usernames.push(rect);
                }
            }
        }
    }
    if own == 1 && (!labels.adaptive() || menu) {
        single_username(&usernames)
    } else {
        None
    }
}

pub async fn observe_own_account(
    session: &dyn UiSession,
    labels: TikTokControls,
) -> anyhow::Result<Option<String>> {
    if !account_read_supported(labels) {
        return Ok(None);
    }
    let epoch = session.gui_session_epoch();
    let active = session
        .active_app_bundle()
        .await
        .map_err(|e| AccountDiagnostic::read_failed(labels, &e).into_error())?;
    if active != labels.package() {
        return Ok(None);
    }
    let mut previous: Option<(u64, String)> = None;
    for _ in 0..2 {
        let snapshot = session
            .hierarchy_source_snapshot()
            .await
            .map_err(|e| AccountDiagnostic::read_failed(labels, &e).into_error())?;
        let tree = Tree::parse(snapshot.clone()).map_err(|e| {
            AccountDiagnostic::new(
                AccountState::Unreadable,
                labels,
                Some(snapshot.generation),
                e.to_string(),
            )
            .into_error()
        })?;
        if let Some(diagnostic) = blocker_diagnostic(&tree, labels) {
            return Err(diagnostic.into_error());
        }
        anyhow::ensure!(
            session.gui_session_epoch() == epoch,
            "account_session_changed"
        );
        let handle = if labels.package() == "com.zhiliaoapp.musically" && !labels.adaptive() {
            let id = global_username_id(labels.resource_version())
                .ok_or_else(|| anyhow::anyhow!("unmeasured account build"))?;
            global_profile_from_snapshot_with_id(&snapshot.xml, id)?
        } else {
            profile_from_tree(&tree, labels)
        };
        let Some(handle) = handle else {
            return Ok(None);
        };
        if let Some((generation, before)) = &previous {
            if tree.generation <= *generation || handle != *before {
                return Ok(None);
            }
        }
        previous = Some((tree.generation, handle));
    }
    if session
        .active_app_bundle()
        .await
        .map_err(|e| AccountDiagnostic::read_failed(labels, &e).into_error())?
        != labels.package()
    {
        return Ok(None);
    }
    anyhow::ensure!(
        session.gui_session_epoch() == epoch,
        "account_session_changed"
    );
    let result = previous.map(|(generation, handle)| {
        let mut diagnostic = AccountDiagnostic::new(
            AccountState::Proved,
            labels,
            Some(generation),
            "Đã xác minh tài khoản từ hai snapshot mới.",
        );
        diagnostic.observed_account = Some(handle.clone());
        diagnostic.record();
        handle
    });
    Ok(result)
}

/// Read the own header, recovering a collapsed profile only inside its observed
/// scroll container. The S8 Trill 38.3.2 snapshots from 14/09 show Edit profile
/// still visible while :id/mjf/@handle is above the viewport after grid browsing.
pub async fn restore_own_profile_header(
    session: &dyn UiSession,
    labels: TikTokControls,
) -> anyhow::Result<Option<String>> {
    for attempt in 0..=3 {
        refuse_account_blocker(session, labels).await?;
        if let Some(dismiss) = labels.label(crate::tiktok_labels::TikTokControl::DialogDismiss) {
            let tree = crate::ui_automation::tree::Tree::parse(
                session.hierarchy_source_snapshot().await?,
            )?;
            let controls = tree.matching(labels.package(), dismiss.to_query());
            if let [index] = controls.as_slice() {
                let button = tree.nodes[*index]
                    .rect()
                    .filter(|r| r.enabled && r.clickable)
                    .ok_or_else(|| anyhow::anyhow!("profile dialog dismiss not actionable"))?;
                session.tap(button.centre()).await?;
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            }
        }
        if let Some(account) = observe_own_account(session, labels).await? {
            return Ok(Some(account));
        }
        if attempt == 3 {
            break;
        }
        let tree =
            crate::ui_automation::tree::Tree::parse(session.hierarchy_source_snapshot().await?)?;
        let own = tree.nodes.iter().enumerate().any(|(i, n)| {
            n.visible(labels.package())
                && tree.ancestors_visible(i)
                && matches!(n.attr("text"), "Edit profile" | "Edit" | "Sửa hồ sơ")
        });
        let profile_grid = labels
            .post_tile_id()
            .is_some_and(|label| !tree.matching(labels.package(), label.to_query()).is_empty());
        // The header can be entirely outside the viewport. Scrolling an observed
        // profile grid back up is navigation, not proof of its account ownership.
        if !own && !profile_grid {
            return Ok(None);
        }
        if attempt == 0 {
            if let Some(account) = selected_profile_account(session, labels, &tree).await? {
                return Ok(Some(account));
            }
        }
        if attempt == 0 {
            if let Some(home) = labels.label(crate::tiktok_labels::TikTokControl::HomeTab) {
                if let Some(tab) = session.locate(home.to_query()).await? {
                    session.tap(tab.centre()).await?;
                    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                }
            }
            if let Some(label) = labels.label(crate::tiktok_labels::TikTokControl::ProfileTab) {
                if let Some(tab) = session.locate(label.to_query()).await? {
                    session.tap(tab.centre()).await?;
                    tokio::time::sleep(std::time::Duration::from_millis(350)).await;
                    continue;
                }
            }
        }
        let area = tree
            .nodes
            .iter()
            .enumerate()
            .filter(|(i, n)| {
                n.visible(labels.package())
                    && tree.ancestors_visible(*i)
                    && n.attr("scrollable") == "true"
            })
            .filter_map(|(_, n)| n.rect())
            .max_by(|a, b| (a.width * a.height).total_cmp(&(b.width * b.height)));
        let Some(area) = area else {
            return Ok(None);
        };
        session
            .swipe(crate::SwipeGesture {
                from: crate::TapPoint {
                    x: area.x + area.width * 0.5,
                    y: area.y + area.height * 0.55,
                },
                to: crate::TapPoint {
                    x: area.x + area.width * 0.5,
                    y: area.y + area.height * 0.93,
                },
                duration_ms: 650,
            })
            .await?;
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    }
    Ok(None)
}

fn selected_account(tree: &crate::ui_automation::tree::Tree, package: &str) -> Option<String> {
    let heading = tree.matching(
        package,
        ElementQuery::Text {
            value: "Switch account",
            exact: true,
        },
    );
    if heading.len() != 1 {
        return None;
    }
    let rows: Vec<_> = tree
        .matching(package, ElementQuery::ResourceIdSuffix(":id/hms"))
        .into_iter()
        .filter(|i| tree.nodes[*i].attr("selected") == "true")
        .collect();
    let [index] = rows.as_slice() else {
        return None;
    };
    let row = &tree.nodes[*index];
    if !row.rect().is_some_and(|r| r.enabled && r.clickable) {
        return None;
    }
    let username = row.attr("content-desc");
    let labels: Vec<_> = tree
        .matching(package, ElementQuery::ResourceIdSuffix(":id/iss"))
        .into_iter()
        .filter(|i| tree.inside(*i, *index) && tree.nodes[*i].attr("selected") == "true")
        .collect();
    let [text] = labels.as_slice() else {
        return None;
    };
    if tree.nodes[*text].attr("text") != username {
        return None;
    }
    let mut rect = row.rect()?;
    rect.description = Some(format!("@{username}"));
    single_username(&[rect])
}

async fn selected_profile_account(
    session: &dyn UiSession,
    labels: TikTokControls,
    profile: &crate::ui_automation::tree::Tree,
) -> anyhow::Result<Option<String>> {
    // Trill 38.3.2, machine 7, 14/09/2026: collapsed header cannot be expanded
    // by grid scrolling; Switch account exposes the current row as selected=true.
    // Only open/close this sheet. Never click an account row or Add account.
    if labels.package() != "com.ss.android.ugc.trill"
        || labels.resource_version() != Some("38.3.2")
        || !session.supports_accessibility_readback()
    {
        return Ok(None);
    }
    let profile_tab = labels.label(crate::tiktok_labels::TikTokControl::ProfileTab);
    if profile.control(labels.package(), profile_tab).is_none() {
        return Ok(None);
    }
    let query = ElementQuery::ResourceIdSuffix(":id/kdu");
    let titles = profile.matching(labels.package(), query);
    let [index] = titles.as_slice() else {
        return Ok(None);
    };
    if !profile.nodes[*index]
        .rect()
        .is_some_and(|r| r.enabled && r.clickable)
    {
        return Ok(None);
    }
    session.activate_element(query).await?;
    let observed = async {
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        let first =
            crate::ui_automation::tree::Tree::parse(session.hierarchy_source_snapshot().await?)?;
        let second =
            crate::ui_automation::tree::Tree::parse(session.hierarchy_source_snapshot().await?)?;
        anyhow::ensure!(
            second.generation > first.generation,
            "account selection snapshot stale"
        );
        anyhow::ensure!(
            session.active_app_bundle().await? == labels.package(),
            "account sheet app changed"
        );
        let handle = selected_account(&first, labels.package());
        Ok::<_, anyhow::Error>(
            handle.filter(|h| Some(h) == selected_account(&second, labels.package()).as_ref()),
        )
    }
    .await;
    let close = session
        .activate_element(ElementQuery::Description {
            value: "Close",
            exact: true,
        })
        .await;
    close?;
    let restored =
        crate::ui_automation::tree::Tree::parse(session.hierarchy_source_snapshot().await?)?;
    anyhow::ensure!(
        restored.control(labels.package(), profile_tab).is_some(),
        "account sheet did not return to profile"
    );
    observed
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn collapsed_profile_account_requires_selected_row_and_matching_label() {
        let xml = include_str!("../fixtures/tiktok-publish/trill-38.3.2-selected-account.xml");
        let parse = |xml: String| {
            crate::ui_automation::tree::Tree::parse(crate::HierarchySourceSnapshot {
                generation: 1,
                xml,
            })
            .unwrap()
        };
        let package = "com.ss.android.ugc.trill";
        let good = parse(xml.into());
        assert_eq!(
            selected_account(&good, package).as_deref(),
            Some("fixture.account")
        );
        assert!(selected_account(
            &parse(xml.replace("selected=\"true\"", "selected=\"false\"")),
            package
        )
        .is_none());
        assert!(selected_account(
            &parse(xml.replace("text=\"fixture.account\"", "text=\"someone.else\"")),
            package
        )
        .is_none());
        assert!(selected_account(&good, "another.package").is_none());
        let mut duplicate = good.clone();
        let row = duplicate
            .nodes
            .iter()
            .find(|n| n.attr("resource-id").ends_with(":id/hms") && n.attr("selected") == "true")
            .unwrap()
            .clone();
        duplicate.nodes.push(row);
        assert!(selected_account(&duplicate, package).is_none());
    }
    #[test]
    fn new_fleet_accounts_require_their_measured_username_id_and_own_profile_controls() {
        for (version, xml) in [
            (
                "46.2.42",
                include_str!("../fixtures/tiktok-publish/musically-46.2.42-en/profile.xml"),
            ),
            (
                "46.0.41",
                include_str!("../fixtures/tiktok-publish/musically-46.0.41-en/profile.xml"),
            ),
            (
                "46.2.1",
                include_str!("../fixtures/tiktok-publish/musically-46.2.1-en/profile.xml"),
            ),
            (
                "45.4.3",
                include_str!("../fixtures/tiktok-publish/musically-45.4.3-en/profile.xml"),
            ),
            (
                "46.1.3",
                include_str!("../fixtures/tiktok-publish/musically-46.1.3-en/profile.xml"),
            ),
            (
                "46.4.3",
                include_str!("../fixtures/tiktok-publish/musically-46.4.3-en/profile.xml"),
            ),
        ] {
            let id = global_username_id(Some(version)).unwrap();
            assert_eq!(
                global_profile_from_snapshot_with_id(xml, id)
                    .unwrap()
                    .as_deref(),
                Some("fixture.account")
            );
            assert!(global_profile_from_snapshot_with_id(xml, "s0v")
                .unwrap()
                .is_none());
            assert!(global_profile_from_snapshot_with_id(
                &xml.replace("Profile menu", "Other menu"),
                id
            )
            .unwrap()
            .is_none());
        }
        assert!(global_username_id(Some("46.4.4")).is_none());
    }
    fn global_profile_from_snapshot(xml: &str) -> anyhow::Result<Option<String>> {
        global_profile_from_snapshot_with_id(xml, "s0v")
    }
    use crate::tiktok_labels::controls_for;
    use std::collections::VecDeque;
    use std::sync::Mutex;

    const PROFILE: &str = include_str!("../../../fixtures/tiktok/account-musically-45.7.3-en.xml");

    struct ProfileSession {
        snapshots: Mutex<VecDeque<String>>,
        packages: Mutex<VecDeque<String>>,
    }

    impl ProfileSession {
        fn global(before: &str, after: &str) -> Self {
            Self {
                snapshots: Mutex::new(VecDeque::from([before.to_owned(), after.to_owned()])),
                packages: Mutex::new(VecDeque::from([
                    "com.zhiliaoapp.musically".to_owned(),
                    "com.zhiliaoapp.musically".to_owned(),
                ])),
            }
        }
    }

    #[async_trait::async_trait]
    impl UiSession for ProfileSession {
        async fn tap(&self, _: crate::TapPoint) -> anyhow::Result<()> {
            panic!("account observation must not tap")
        }
        async fn swipe(&self, _: crate::SwipeGesture) -> anyhow::Result<()> {
            panic!("account observation must not swipe")
        }
        async fn type_text(&self, _: &str) -> anyhow::Result<()> {
            panic!("account observation must not type")
        }
        async fn home(&self) -> anyhow::Result<()> {
            panic!("account observation must not navigate")
        }
        async fn find_and_tap(&self, _: &str) -> anyhow::Result<()> {
            panic!("account observation must not tap")
        }
        async fn assert_visible(&self, _: &str) -> anyhow::Result<()> {
            panic!("account observation requires complete proof")
        }
        fn stream_url(&self) -> Option<String> {
            None
        }
        async fn active_app_bundle(&self) -> anyhow::Result<String> {
            Ok(self
                .packages
                .lock()
                .unwrap()
                .pop_front()
                .expect("foreground proof"))
        }
        async fn hierarchy_source_snapshot(
            &self,
        ) -> anyhow::Result<crate::HierarchySourceSnapshot> {
            let mut snapshots = self.snapshots.lock().unwrap();
            Ok(crate::HierarchySourceSnapshot {
                generation: 3 - snapshots.len() as u64,
                xml: snapshots.pop_front().expect("two source reads"),
            })
        }
        async fn locate_all_described(
            &self,
            query: ElementQuery<'_>,
        ) -> anyhow::Result<Vec<ElementBox>> {
            // Legacy Trill uses its existing exact Edit profile/mjf locator route.
            Ok(match query {
                ElementQuery::Text {
                    value: "Edit profile",
                    exact: true,
                } => vec![node("Edit profile")],
                ElementQuery::ResourceIdSuffix(":id/mjf") => vec![node("@legacy.account")],
                _ => panic!("unmeasured account locator"),
            })
        }
    }

    #[test]
    fn measured_global_profile_requires_its_three_unique_controls() {
        assert_eq!(
            global_profile_from_snapshot(PROFILE).unwrap().as_deref(),
            Some("fixture.account")
        );
        for changed in [
            PROFILE.replace("text=\"Edit\"", "text=\"Edit profile\""),
            PROFILE.replace("Profile menu", "Profile menu help"),
            PROFILE.replace(":id/s0v", ":id/other"),
            PROFILE.replace("text=\"Edit\" clickable=\"true\"", "text=\"Edit\" clickable=\"false\""),
            PROFILE.replace("[423,510][656,549]", "[423,510][423,549]"),
            PROFILE.replace("resource-id=\"com.zhiliaoapp.musically:id/s0v\" clickable=\"true\" enabled=\"true\"", "resource-id=\"com.zhiliaoapp.musically:id/s0v\" clickable=\"true\" enabled=\"false\""),
            PROFILE.replace("com.zhiliaoapp.musically", "com.other.app"),
            PROFILE.replace("android.widget.Button", "android.widget.TextView"),
            PROFILE.replace("text=\"@bio.mention\"", "text=\"Edit\""),
            PROFILE.replace("text=\"@bio.mention\"", "text=\"@other.account\" resource-id=\"com.zhiliaoapp.musically:id/s0v\""),
            PROFILE.replace("text=\"@bio.mention\"", "text=\"\" content-desc=\"Profile menu\""),
            PROFILE.replace("displayed=\"true\"", "displayed=\"false\""),
        ] {
            assert_eq!(global_profile_from_snapshot(&changed).unwrap(), None);
        }
        assert!(global_profile_from_snapshot(&PROFILE.replace("</hierarchy>", "")).is_err());
    }

    #[test]
    fn measured_global_draft_overlay_keeps_own_profile_proof_without_edit_menu() {
        const OVERLAY: &str =
            include_str!("../../../fixtures/tiktok/account-musically-45.7.3-en-draft-overlay.xml");
        assert_eq!(
            global_profile_from_snapshot(OVERLAY).unwrap().as_deref(),
            Some("fixture.account")
        );
        for changed in [
            OVERLAY.replace("selected=\"true\"", "selected=\"false\""),
            OVERLAY.replace("text=\"Add bio\"", "text=\"Other\""),
            OVERLAY.replace(":id/rv2", ":id/other"),
        ] {
            assert_eq!(global_profile_from_snapshot(&changed).unwrap(), None);
        }
    }

    #[tokio::test]
    async fn measured_global_account_is_repeated_without_effects() {
        let session = ProfileSession::global(PROFILE, PROFILE);
        let labels = controls_for("com.zhiliaoapp.musically", "en", "45.7.3").unwrap();
        assert_eq!(
            observe_own_account(&session, labels)
                .await
                .unwrap()
                .as_deref(),
            Some("fixture.account")
        );
        assert!(session.snapshots.lock().unwrap().is_empty());
        assert!(session.packages.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn changed_account_or_own_profile_proof_never_returns_a_handle() {
        let labels = controls_for("com.zhiliaoapp.musically", "en", "45.7.3").unwrap();
        for after in [
            PROFILE.replace("@fixture.account", "@changed.account"),
            PROFILE.replace("Profile menu", "Share profile"),
        ] {
            assert_eq!(
                observe_own_account(&ProfileSession::global(PROFILE, &after), labels)
                    .await
                    .unwrap(),
                None
            );
        }
        let session = ProfileSession::global(PROFILE, PROFILE);
        *session.packages.lock().unwrap() = VecDeque::from([
            "com.zhiliaoapp.musically".to_owned(),
            "com.other.app".to_owned(),
        ]);
        assert_eq!(observe_own_account(&session, labels).await.unwrap(), None);
    }

    #[tokio::test]
    async fn unmeasured_global_build_language_and_wrong_foreground_do_not_read_sources() {
        assert!(controls_for("com.zhiliaoapp.musically", "vi", "45.7.3").is_none());
        for (language, version) in [("en", "46.0.42"), ("en", "45.7.4")] {
            let session = ProfileSession::global(PROFILE, PROFILE);
            let labels = controls_for("com.zhiliaoapp.musically", language, version).unwrap();
            assert_eq!(observe_own_account(&session, labels).await.unwrap(), None);
            assert_eq!(session.snapshots.lock().unwrap().len(), 2);
            assert_eq!(session.packages.lock().unwrap().len(), 2);
        }
        let session = ProfileSession::global(PROFILE, PROFILE);
        session.packages.lock().unwrap()[0] = "com.other.app".to_owned();
        assert_eq!(
            observe_own_account(
                &session,
                controls_for("com.zhiliaoapp.musically", "en", "45.7.3").unwrap()
            )
            .await
            .unwrap(),
            None
        );
        assert_eq!(session.snapshots.lock().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn legacy_trill_keeps_its_measured_profile_locators() {
        let snapshot = PROFILE
            .replace("com.zhiliaoapp.musically", "com.ss.android.ugc.trill")
            .replace(":id/s0v", ":id/mjf");
        let own = "<node package=\"com.ss.android.ugc.trill\" resource-id=\"com.ss.android.ugc.trill:id/dby\" class=\"android.widget.TextView\" text=\"Edit profile\" displayed=\"true\" enabled=\"true\" bounds=\"[1,1][20,20]\"/>";
        let snapshot = snapshot.replace("</hierarchy>", &format!("{own}</hierarchy>"));
        let session = ProfileSession::global(&snapshot, &snapshot);
        *session.packages.lock().unwrap() = VecDeque::from([
            "com.ss.android.ugc.trill".to_owned(),
            "com.ss.android.ugc.trill".to_owned(),
        ]);
        let account = observe_own_account(
            &session,
            controls_for("com.ss.android.ugc.trill", "en", "38.3.2").unwrap(),
        )
        .await
        .unwrap();
        assert_eq!(account.as_deref(), Some("fixture.account"));
        assert!(session.snapshots.lock().unwrap().is_empty());
    }
    fn node(text: &str) -> ElementBox {
        ElementBox {
            x: 386.0,
            y: 552.0,
            width: 308.0,
            height: 50.0,
            description: Some(text.into()),
            enabled: true,
            clickable: true,
        }
    }
    #[test]
    fn rejects_display_names_urls_and_ambiguous_accounts() {
        assert_eq!(
            single_username(&[node("@valid.account")]).as_deref(),
            Some("valid.account")
        );
        for text in [
            "Display Name",
            "https://tiktok.com/@valid",
            "@",
            "@with space",
            "@@wrong",
            "@trailing.",
        ] {
            assert!(single_username(&[node(text)]).is_none());
        }
        assert!(single_username(&[node("@a"), node("@b")]).is_none());
    }
    #[test]
    fn unmeasured_build_and_language_never_claim_a_profile_locator() {
        use crate::tiktok_labels::controls_for;
        assert!(account_read_supported(
            controls_for("com.ss.android.ugc.trill", "en", "38.3.2").unwrap()
        ));
        assert!(!account_read_supported(
            controls_for("com.ss.android.ugc.trill", "vi", "38.3.2").unwrap()
        ));
        assert!(!account_read_supported(
            controls_for("com.ss.android.ugc.trill", "en", "46.3.3").unwrap()
        ));
    }
}
