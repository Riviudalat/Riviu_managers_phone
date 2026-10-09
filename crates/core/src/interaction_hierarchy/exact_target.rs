use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use anyhow::Context;
use tokio::time::Instant;

use super::{
    bare_author_label, confirm_target_from_share_link, read_author_label,
    read_target_identity_caption, TargetArrival, TargetCardContinuityLost, TargetLinkMismatch,
};
use crate::app_automation::dialogs::{step_before_deadline_with_tree, PopupBudget, PopupStep};
use crate::driver::{ElementQuery, UiSession};
use crate::tiktok_labels::{TikTokControl, TikTokControls};
use crate::ui_automation::runtime::ReadWaitResult;
use crate::ui_automation::tree::Tree;
use crate::ResolvedTikTokTarget;

// SM-G955F/Trill 38.3.2, 14/09/2026: the blank loading surface and a feed
// LIVE placeholder can outlast 14s. Keep one URL dispatch and wait up to 30s.
// On the measured detail route the post id in the dispatched link authorizes the
// post; elsewhere only a copied canonical URL does.
const CARD_WINDOW: Duration = Duration::from_secs(30);
const CARD_POLL: Duration = Duration::from_millis(350);
const DISPATCH_SETTLE: Duration = Duration::from_millis(900);
const UNAVAILABLE_TEXT: &str = "This video is unavailable";
const POPUP_BLOCKED_SAYS: &str =
    "một lớp phủ chặn bài đích — Riviu không bấm gì; cần người vận hành kiểm tra";

#[derive(Debug, thiserror::Error)]
enum CardObservationError {
    #[error("target_post_unavailable: TikTok reported 'This video is unavailable'")]
    PostUnavailable,
    #[error("foreground: package unreadable")]
    ForegroundUnreadable,
    #[error("foreground: expected target app, observed {0}")]
    WrongForeground(String),
    #[error("Comments: measured control missing or unreadable")]
    CommentsUnreadable,
    #[error("author: label missing or unreadable")]
    AuthorUnreadable,
    #[error("caption: measured identity missing, empty, ambiguous or unreadable")]
    CaptionUnreadable,
    #[error("hierarchy: snapshot unreadable")]
    HierarchyUnreadable,
    #[error("detail: post detail page not shown")]
    NotDetailPage,
}

// Measured on TikTok Global 45.7.3/en, SM-G955F ce0417143cef30280d (Riviu #46),
// 09/10/2026 (apply-apk-20261009/share-deeplink-probe, B3b/B3e):
// `snssdk1233://aweme/detail/<id>` opens DetailActivity on exactly that post. The
// page has a top-left `bj1` "Back" and no Home tab; the For You feed, where the
// https form lands instead (B2g), has the Home tab and no `bj1`.
const DETAIL_PACKAGE: &str = "com.zhiliaoapp.musically";
const DETAIL_VERSION: &str = "45.7.3";
const DETAIL_BACK_ID: &str = "com.zhiliaoapp.musically:id/bj1";
const DETAIL_BACK_LABEL: &str = "Back";
/// Longest numeric post id accepted into the detail link. Real ids are 19 digits.
const MAX_CONTENT_ID_DIGITS: usize = 32;

/// Open the target post and prove it is the one on screen. It never taps a public-action
/// control.
///
/// On the measured detail route (Global 45.7.3/en) the post is opened by its id from the
/// campaign link, and the opened detail page is the proof: **the share sheet is never
/// opened**. TikTok very likely counts `Copy link` as a public share (probe 09/10/2026: the
/// share counter went 0 -> 1 about a minute after one copy), so routine interaction must not
/// copy a link it already has. See [`open_detail_target`].
///
/// Other builds keep the legacy proof: the current post's own copied URL, or the pinned URL
/// opened once and then copied. Effects are refused on those builds by the fleet allowlist,
/// so only read paths (comment verification, inspection) can reach it. Author and caption
/// are continuity evidence there, never authority for a particular post.
pub async fn open_exact_target_by_hierarchy(
    session: &dyn UiSession,
    labels: TikTokControls,
    target_package: &str,
    expected: &ResolvedTikTokTarget,
    stop: &AtomicBool,
) -> anyhow::Result<TargetArrival> {
    open_target(session, labels, target_package, expected, stop, true)
        .await
        .map(|(arrival, _)| arrival)
}

/// [`open_exact_target_by_hierarchy`], also saying whether a popup was dismissed on the way.
/// A dismissal voids every earlier proof on this phone: the caller renews its account proof
/// and opens the target again before any public action.
pub async fn open_exact_target_reporting_popups(
    session: &dyn UiSession,
    labels: TikTokControls,
    target_package: &str,
    expected: &ResolvedTikTokTarget,
    stop: &AtomicBool,
) -> anyhow::Result<(TargetArrival, bool)> {
    open_target(session, labels, target_package, expected, stop, true).await
}

async fn open_target(
    session: &dyn UiSession,
    labels: TikTokControls,
    target_package: &str,
    expected: &ResolvedTikTokTarget,
    stop: &AtomicBool,
    inspect_current: bool,
) -> anyhow::Result<(TargetArrival, bool)> {
    ensure_not_cancelled(stop)?;
    anyhow::ensure!(
        labels.label(TikTokControl::Comments).is_some(),
        "target_exact_open: comments control unmeasured"
    );
    if detail_route_measured(labels, target_package) {
        return open_detail_target(session, labels, target_package, expected, stop).await;
    }

    if inspect_current
        && readable_card_in_foreground(session, labels, target_package, false)
            .await
            .is_ok()
    {
        ensure_not_cancelled(stop)?;
        if let Ok(arrival) = confirm_target_from_share_link(session, labels, expected).await {
            ensure_not_cancelled(stop)?;
            ensure_foreground(session, target_package).await?;
            return Ok((arrival, false));
        }
    }

    ensure_not_cancelled(stop)?;
    session
        .open_url_in_app(&expected.normalized_url, target_package)
        .await
        .context("target_exact_open: pinned URL dispatch failed")?;

    // Do not require a Home/feed baseline: cold launches and an already-open target can
    // both lack a changed author. The URL readback below is the only arrival authority.
    let mut deadline = Instant::now() + CARD_WINDOW;
    let mut redispatched = false;
    let mut continuity_retry_used = false;
    // One budget for this open: each measured popup gets at most one press here.
    let mut popup = PopupBudget::default();
    let mut dismissed = false;
    tokio::time::sleep(DISPATCH_SETTLE).await;
    loop {
        ensure_not_cancelled(stop)?;
        let observation = readable_card_in_foreground(session, labels, target_package, true).await;
        if matches!(observation, Err(CardObservationError::PostUnavailable)) {
            return Err(CardObservationError::PostUnavailable.into());
        }
        if matches!(observation, Err(CardObservationError::CommentsUnreadable))
            && session.supports_accessibility_readback()
        {
            // The shared popup step replaces the single contacts decline: same measured
            // detectors, one press per popup, and unknown modals stop here untouched.
            match step_before_deadline_with_tree(session, labels, &mut popup, deadline, stop, stop)
                .await?
            {
                ReadWaitResult::Ready((PopupStep::Dismissed, _)) => {
                    dismissed = true;
                    tokio::time::sleep(CARD_POLL).await;
                    continue;
                }
                ReadWaitResult::Ready((PopupStep::Blocked, _)) => {
                    anyhow::bail!(
                        "target_exact_open: {}",
                        popup.says().unwrap_or(POPUP_BLOCKED_SAYS)
                    );
                }
                ReadWaitResult::Cancelled => anyhow::bail!("target_exact_open: cancelled"),
                ReadWaitResult::Ready((PopupStep::Clear, _)) | ReadWaitResult::DeadlineExceeded => {
                }
            }
        }
        // A cold Trill launch can consume the VIEW intent and remain on a LIVE
        // feed tile. Reopen the SAME pinned link once only after that observable
        // placeholder persists; this still cannot authorize a public action.
        if !redispatched
            && matches!(observation, Err(CardObservationError::CommentsUnreadable))
            && deadline.saturating_duration_since(Instant::now()) <= Duration::from_secs(20)
            && live_feed_placeholder(session, target_package).await
        {
            ensure_not_cancelled(stop)?;
            session
                .reopen_url_in_app(&expected.normalized_url, target_package)
                .await?;
            redispatched = true;
            deadline = Instant::now() + CARD_WINDOW;
            tokio::time::sleep(DISPATCH_SETTLE).await;
            continue;
        }
        if observation.is_ok() {
            ensure_not_cancelled(stop)?;
            let proof = confirm_target_from_share_link(session, labels, expected).await;
            ensure_not_cancelled(stop)?;
            ensure_foreground(session, target_package).await?;
            match proof {
                Ok(arrival) => return Ok((arrival, dismissed)),
                Err(error)
                    if error.downcast_ref::<TargetLinkMismatch>().is_some()
                        && !redispatched
                        && Instant::now() >= deadline
                        && ((target_package == "com.zhiliaoapp.musically"
                            && matches!(labels.resource_version(), Some("46.0.41" | "45.7.3")))
                            || (target_package == "com.ss.android.ugc.trill"
                                && labels.resource_version() == Some("38.3.2"))) =>
                {
                    // Measured Global and Trill 38.3.2 can retain a normal feed
                    // card after VIEW. A copied different post ID proves this
                    // arrival missed the target only after the original loading window.
                    // A wrong card during loading must not restart an in-flight VIEW.
                    // Reopen the pinned URL once; public actions still need exact proof.
                    ensure_not_cancelled(stop)?;
                    session
                        .reopen_url_in_app(&expected.normalized_url, target_package)
                        .await?;
                    redispatched = true;
                    deadline = Instant::now() + CARD_WINDOW;
                    tokio::time::sleep(DISPATCH_SETTLE).await;
                }
                Err(error)
                    if error.downcast_ref::<TargetLinkMismatch>().is_some()
                        && Instant::now() < deadline => {}
                Err(error)
                    if error.downcast_ref::<TargetCardContinuityLost>().is_some()
                        && !continuity_retry_used
                        && Instant::now() < deadline =>
                {
                    // The discarded proof crossed no public-action boundary. Acquire a
                    // new before/copy/after pair once, within the same loading deadline.
                    // The canonical URL and every continuity predicate still must match.
                    continuity_retry_used = true;
                }
                Err(error) => {
                    return Err(error.context("target_exact_open: exact post proof failed"));
                }
            }
        }
        if Instant::now() >= deadline {
            let last_check = observation
                .err()
                .map(|error| error.to_string())
                .unwrap_or_else(|| "copied URL did not identify the target".into());
            anyhow::bail!(
                "target_exact_open: no readable post in the target app within {} seconds; last check: {last_check}", CARD_WINDOW.as_secs()
            );
        }
        tokio::time::sleep(CARD_POLL.min(deadline.saturating_duration_since(Instant::now()))).await;
    }
}

fn detail_route_measured(labels: TikTokControls, target_package: &str) -> bool {
    target_package == DETAIL_PACKAGE
        && labels.package() == DETAIL_PACKAGE
        && labels.resource_version() == Some(DETAIL_VERSION)
        && labels.language() == "en"
}

fn detail_link(content_id: &str) -> anyhow::Result<String> {
    anyhow::ensure!(
        !content_id.is_empty()
            && content_id.len() <= MAX_CONTENT_ID_DIGITS
            && content_id.bytes().all(|b| b.is_ascii_digit()),
        "target_detail_open: post id is not numeric"
    );
    Ok(format!("snssdk1233://aweme/detail/{content_id}"))
}

/// What one snapshot of a detail page says about the post on it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct DetailCard {
    author: String,
    caption: String,
}

/// Open the target by its post id and prove it without the share sheet.
///
/// The authority is the id in the dispatched link, and it holds only for a detail page that
/// **this dispatch created**:
///
/// 1. Before the dispatch TikTok must be foreground on a screen that is not a detail page.
///    Otherwise the app is restarted once (force-stop clears every stacked page) and the
///    check repeated; a detail page that survives that is refused, because a link that
///    failed silently would leave it up and it could be any post.
/// 2. After the dispatch, two snapshots of increasing generation must both show the detail
///    page (`bj1` Back, no Home tab, Comments) with the same author and one caption.
/// 3. "This video is unavailable", a feed card, another app or an unreadable card refuse
///    once the loading window ends. The link may be dispatched one more time when no detail
///    page has appeared; that is navigation, never a public action.
///
/// The shared popup step runs on every read. A dismissal voids what was read before it and
/// is reported in the returned flag, so the caller can also renew its account proof.
async fn open_detail_target(
    session: &dyn UiSession,
    labels: TikTokControls,
    target_package: &str,
    expected: &ResolvedTikTokTarget,
    stop: &AtomicBool,
) -> anyhow::Result<(TargetArrival, bool)> {
    let link = detail_link(&expected.content_id)?;
    if !detail_baseline_clear(session, labels, target_package).await {
        ensure_not_cancelled(stop)?;
        session
            .restart_app(target_package)
            .await
            .context("target_detail_open: app restart failed")?;
        tokio::time::sleep(DISPATCH_SETTLE).await;
        ensure_not_cancelled(stop)?;
        anyhow::ensure!(
            detail_baseline_clear(session, labels, target_package).await,
            "target_detail_open: a post detail page was still open before the link; refusing to take it as the target"
        );
    }
    ensure_not_cancelled(stop)?;
    session
        .open_url_in_app(&link, target_package)
        .await
        .context("target_detail_open: detail link dispatch failed")?;

    let mut deadline = Instant::now() + CARD_WINDOW;
    let mut redispatched = false;
    let mut previous: Option<(u64, DetailCard)> = None;
    // One budget for this open: each measured popup gets at most one press here.
    let mut popup = PopupBudget::default();
    let mut dismissed = false;
    tokio::time::sleep(DISPATCH_SETTLE).await;
    loop {
        ensure_not_cancelled(stop)?;
        // The popup step reads the snapshot the card is then observed on, so a proof is
        // only ever taken from a read that showed no popup.
        let observation = if session.supports_accessibility_readback() {
            match step_before_deadline_with_tree(session, labels, &mut popup, deadline, stop, stop)
                .await
            {
                Ok(ReadWaitResult::Ready((PopupStep::Clear, Some(tree)))) => {
                    match observe_detail(&tree, labels) {
                        Ok(card) => Ok((tree.generation, card)),
                        Err(error) => Err((error, Some(tree))),
                    }
                }
                Ok(ReadWaitResult::Ready((PopupStep::Dismissed, _))) => {
                    // Whatever was read before the popup went is no longer proof.
                    previous = None;
                    dismissed = true;
                    tokio::time::sleep(CARD_POLL).await;
                    continue;
                }
                Ok(ReadWaitResult::Ready((PopupStep::Blocked, _))) => {
                    anyhow::bail!(
                        "target_detail_open: {}",
                        popup.says().unwrap_or(POPUP_BLOCKED_SAYS)
                    );
                }
                Ok(ReadWaitResult::Cancelled) => {
                    anyhow::bail!("target_exact_open: cancelled");
                }
                Ok(ReadWaitResult::Ready((PopupStep::Clear, None)))
                | Ok(ReadWaitResult::DeadlineExceeded)
                | Err(_) => Err((CardObservationError::HierarchyUnreadable, None)),
            }
        } else {
            observe_detail_in_foreground(session, labels, target_package).await
        };
        match observation {
            Ok((generation, card)) => {
                if previous
                    .as_ref()
                    .is_some_and(|(seen, first)| generation > *seen && *first == card)
                {
                    ensure_not_cancelled(stop)?;
                    ensure_foreground(session, target_package).await?;
                    return Ok((
                        TargetArrival::Identified {
                            author_label: card.author,
                        },
                        dismissed,
                    ));
                }
                previous = Some((generation, card));
            }
            Err((CardObservationError::PostUnavailable, _)) => {
                return Err(CardObservationError::PostUnavailable.into());
            }
            Err((CardObservationError::NotDetailPage, _))
                if !redispatched
                    && deadline.saturating_duration_since(Instant::now())
                        <= Duration::from_secs(20) =>
            {
                // A cold launch can swallow the first VIEW. Dispatch the same id once more.
                ensure_not_cancelled(stop)?;
                session
                    .open_url_in_app(&link, target_package)
                    .await
                    .context("target_detail_open: detail link redispatch failed")?;
                redispatched = true;
                previous = None;
                deadline = Instant::now() + CARD_WINDOW;
                tokio::time::sleep(DISPATCH_SETTLE).await;
                continue;
            }
            Err(_) => previous = None,
        }
        if Instant::now() >= deadline {
            let last_check =
                match observe_detail_in_foreground(session, labels, target_package).await {
                    Ok(_) => "detail page did not hold still across two reads".to_string(),
                    Err((error, _)) => error.to_string(),
                };
            anyhow::bail!(
                "target_detail_open: post {} not proven on a detail page within {} seconds; last check: {last_check}",
                expected.content_id,
                CARD_WINDOW.as_secs()
            );
        }
        tokio::time::sleep(CARD_POLL.min(deadline.saturating_duration_since(Instant::now()))).await;
    }
}

/// True only when TikTok is foreground on a readable screen that is not a detail page.
async fn detail_baseline_clear(
    session: &dyn UiSession,
    labels: TikTokControls,
    target_package: &str,
) -> bool {
    if !matches!(session.active_app_bundle().await, Ok(package) if package == target_package) {
        return false;
    }
    let Ok(snapshot) = session.hierarchy_source_snapshot().await else {
        return false;
    };
    let Ok(tree) = Tree::parse(snapshot) else {
        return false;
    };
    is_detail_page(&tree, labels) == Some(false)
}

/// `Some(true)` for the measured detail page, `Some(false)` for a screen with the Home tab or
/// without the detail Back, `None` when the Home tab is unmeasured.
fn is_detail_page(tree: &Tree, labels: TikTokControls) -> Option<bool> {
    let package = labels.package();
    let home = labels.label(TikTokControl::HomeTab)?;
    let back = tree
        .matching(
            package,
            ElementQuery::Description {
                value: DETAIL_BACK_LABEL,
                exact: true,
            },
        )
        .into_iter()
        .any(|index| tree.nodes[index].attr("resource-id") == DETAIL_BACK_ID);
    Some(back && tree.matching(package, home.to_query()).is_empty())
}

type DetailObservation = Result<(u64, DetailCard), (CardObservationError, Option<Tree>)>;

async fn observe_detail_in_foreground(
    session: &dyn UiSession,
    labels: TikTokControls,
    target_package: &str,
) -> DetailObservation {
    match session.active_app_bundle().await {
        Ok(package) if package == target_package => {}
        Ok(package) => {
            return Err((
                CardObservationError::WrongForeground(package.chars().take(128).collect()),
                None,
            ));
        }
        Err(_) => return Err((CardObservationError::ForegroundUnreadable, None)),
    }
    let Ok(snapshot) = session.hierarchy_source_snapshot().await else {
        return Err((CardObservationError::HierarchyUnreadable, None));
    };
    let Ok(tree) = Tree::parse(snapshot) else {
        return Err((CardObservationError::HierarchyUnreadable, None));
    };
    match observe_detail(&tree, labels) {
        Ok(card) => Ok((tree.generation, card)),
        Err(error) => Err((error, Some(tree))),
    }
}

/// Every predicate is read from one snapshot, so the card cannot change between them.
fn observe_detail(tree: &Tree, labels: TikTokControls) -> Result<DetailCard, CardObservationError> {
    let package = labels.package();
    let unavailable = ElementQuery::Text {
        value: UNAVAILABLE_TEXT,
        exact: true,
    };
    if !tree.matching(package, unavailable).is_empty() {
        return Err(CardObservationError::PostUnavailable);
    }
    let comments = labels
        .label(TikTokControl::Comments)
        .ok_or(CardObservationError::CommentsUnreadable)?;
    if tree.matching(package, comments.to_query()).is_empty() {
        return Err(CardObservationError::CommentsUnreadable);
    }
    if is_detail_page(tree, labels) != Some(true) {
        return Err(CardObservationError::NotDetailPage);
    }
    let mut authors = Vec::new();
    for control in [TikTokControl::AuthorProfileLink, TikTokControl::Follow] {
        let Some(label) = labels.label(control) else {
            continue;
        };
        for index in tree.matching(package, label.to_query()) {
            if let Some(bare) = bare_author_label(tree.nodes[index].attr("content-desc"), label) {
                if !authors.contains(&bare) {
                    authors.push(bare);
                }
            }
        }
    }
    let [author] = authors.as_slice() else {
        return Err(CardObservationError::AuthorUnreadable);
    };
    let captions: Vec<String> = tree
        .matching(package, ElementQuery::ResourceIdSuffix(":id/desc"))
        .into_iter()
        .map(|index| tree.nodes[index].attr("text").trim().to_string())
        .filter(|text| !text.is_empty())
        .collect();
    let [caption] = captions.as_slice() else {
        return Err(CardObservationError::CaptionUnreadable);
    };
    Ok(DetailCard {
        author: author.clone(),
        caption: caption.clone(),
    })
}

/// Open the saved target after a clean app start. The initial feed is not a candidate.
pub async fn open_pinned_target_by_hierarchy(
    session: &dyn UiSession,
    labels: TikTokControls,
    target_package: &str,
    expected: &ResolvedTikTokTarget,
    stop: &AtomicBool,
) -> anyhow::Result<TargetArrival> {
    open_target(session, labels, target_package, expected, stop, false)
        .await
        .map(|(arrival, _)| arrival)
}

async fn live_feed_placeholder(session: &dyn UiSession, package: &str) -> bool {
    let Ok(snapshot) = session.hierarchy_source_snapshot().await else {
        return false;
    };
    let Ok(tree) = crate::ui_automation::tree::Tree::parse(snapshot) else {
        return false;
    };
    let has = |label: &str| {
        tree.nodes.iter().enumerate().any(|(i, n)| {
            n.visible(package)
                && tree.ancestors_visible(i)
                && (n.attr("text") == label || n.attr("content-desc") == label)
        })
    };
    has("Home") && has("Profile") && (has("Tap to watch LIVE") || has("LIVE now"))
}

fn ensure_not_cancelled(stop: &AtomicBool) -> anyhow::Result<()> {
    anyhow::ensure!(
        !stop.load(Ordering::Relaxed),
        "target_exact_open: cancelled"
    );
    Ok(())
}

async fn ensure_foreground(session: &dyn UiSession, target_package: &str) -> anyhow::Result<()> {
    let foreground = session
        .active_app_bundle()
        .await
        .context("target_exact_open: foreground unreadable after proof")?;
    anyhow::ensure!(
        foreground == target_package,
        "target_exact_open: target app left foreground during proof"
    );
    Ok(())
}

async fn readable_card_in_foreground(
    session: &dyn UiSession,
    labels: TikTokControls,
    target_package: &str,
    after_dispatch: bool,
) -> Result<(), CardObservationError> {
    match session.active_app_bundle().await {
        Ok(package) if package == target_package => {}
        Ok(package) => {
            return Err(CardObservationError::WrongForeground(
                package.chars().take(128).collect(),
            ));
        }
        Err(_) => return Err(CardObservationError::ForegroundUnreadable),
    }
    if after_dispatch
        && matches!(
            session
                .locate(ElementQuery::Text {
                    value: UNAVAILABLE_TEXT,
                    exact: true,
                })
                .await,
            Ok(Some(_))
        )
    {
        return Err(CardObservationError::PostUnavailable);
    }
    let Some(comments) = labels.label(TikTokControl::Comments) else {
        return Err(CardObservationError::CommentsUnreadable);
    };
    if !matches!(session.locate(comments.to_query()).await, Ok(Some(_))) {
        return Err(CardObservationError::CommentsUnreadable);
    }
    if read_author_label(session, labels)
        .await
        .is_none_or(|value| value.trim().is_empty())
    {
        return Err(CardObservationError::AuthorUnreadable);
    }
    if read_target_identity_caption(session)
        .await
        .is_none_or(|value| value.trim().is_empty())
    {
        return Err(CardObservationError::CaptionUnreadable);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicUsize;

    use parking_lot::Mutex;

    use super::*;
    use crate::driver::{ElementBox, ElementQuery};
    use crate::tiktok_labels::controls_for;
    use crate::{SwipeGesture, TapPoint};

    const PACKAGE: &str = "com.ss.android.ugc.trill";
    const TARGET_URL: &str = "https://www.tiktok.com/@fixture/photo/12345";
    const WRONG_URL: &str = "https://www.tiktok.com/@fixture/photo/54321";
    const CAPTION: &str = "A stable caption long enough to identify this one post without guessing";

    struct ExactSession {
        card_visible: AtomicBool,
        clipboard: Mutex<String>,
        sheet_open: AtomicBool,
        opens: Mutex<Vec<(String, String)>>,
        dispatched_at: Mutex<Option<Instant>>,
        tapped_at: Mutex<Vec<Instant>>,
        navigation_taps: AtomicUsize,
        foreground: bool,
        copied_url: &'static str,
        card_changes_after_copy: bool,
        transient_author_change: AtomicBool,
        copies: AtomicUsize,
        caption_nodes: Vec<&'static str>,
        caption_query_fails: bool,
        fallback_caption: Option<&'static str>,
        target_load_delay: Option<Duration>,
        copied_urls: Mutex<Vec<&'static str>>,
        clipboard_write_fails: bool,
        clipboard_writes: AtomicUsize,
        unavailable_text: Option<&'static str>,
        comments_missing: bool,
        first_intent_lost_to_live: bool,
        author_missing: bool,
    }

    impl ExactSession {
        fn new(card_visible: bool, copied_url: &'static str) -> Self {
            Self {
                card_visible: AtomicBool::new(card_visible),
                clipboard: Mutex::new(String::new()),
                sheet_open: AtomicBool::new(false),
                opens: Mutex::new(Vec::new()),
                dispatched_at: Mutex::new(None),
                tapped_at: Mutex::new(Vec::new()),
                navigation_taps: AtomicUsize::new(0),
                foreground: true,
                copied_url,
                card_changes_after_copy: false,
                transient_author_change: AtomicBool::new(false),
                copies: AtomicUsize::new(0),
                caption_nodes: vec![CAPTION],
                caption_query_fails: false,
                fallback_caption: None,
                target_load_delay: None,
                copied_urls: Mutex::new(Vec::new()),
                clipboard_write_fails: false,
                clipboard_writes: AtomicUsize::new(0),
                unavailable_text: None,
                comments_missing: false,
                first_intent_lost_to_live: false,
                author_missing: false,
            }
        }

        fn is_visible(&self) -> bool {
            self.card_visible.load(Ordering::Relaxed)
        }
    }

    fn labels() -> TikTokControls {
        controls_for(PACKAGE, "en", "38.3.2").unwrap()
    }

    fn node(description: &str, y: f64) -> ElementBox {
        ElementBox {
            x: 0.0,
            y,
            width: 100.0,
            height: 100.0,
            description: Some(description.into()),
            enabled: true,
            clickable: true,
        }
    }

    #[async_trait::async_trait]
    impl UiSession for ExactSession {
        async fn hierarchy_source_snapshot(
            &self,
        ) -> anyhow::Result<crate::HierarchySourceSnapshot> {
            anyhow::ensure!(self.first_intent_lost_to_live, "no measured source");
            Ok(crate::HierarchySourceSnapshot {
                generation: 1,
                xml: format!(
                    r#"<hierarchy><node package="{PACKAGE}" text="Home" bounds="[0,0][100,100]"/><node package="{PACKAGE}" text="Profile" bounds="[100,0][200,100]"/><node package="{PACKAGE}" text="Tap to watch LIVE" bounds="[0,100][200,200]"/></hierarchy>"#
                ),
            })
        }
        async fn active_app_bundle(&self) -> anyhow::Result<String> {
            Ok(if self.foreground {
                PACKAGE.into()
            } else {
                "com.android.launcher".into()
            })
        }

        async fn open_url_in_app(&self, url: &str, bundle: &str) -> anyhow::Result<()> {
            self.opens.lock().push((url.into(), bundle.into()));
            *self.dispatched_at.lock() = Some(Instant::now());
            self.card_visible.store(true, Ordering::Relaxed);
            Ok(())
        }

        async fn tap(&self, point: TapPoint) -> anyhow::Result<()> {
            if let Some(dispatched_at) = *self.dispatched_at.lock() {
                assert!(
                    dispatched_at.elapsed() >= DISPATCH_SETTLE,
                    "the URL intent has not had time to replace the previous card"
                );
            }
            self.tapped_at.lock().push(Instant::now());
            // The fixture authorizes only these two navigation rectangles. Any public
            // tap, Home tap or guessed coordinate fails the test rather than succeeding.
            if point.x == 50.0 && point.y == 550.0 && !self.sheet_open.load(Ordering::Relaxed) {
                self.sheet_open.store(true, Ordering::Relaxed);
            } else if point.x == 50.0
                && point.y == 1050.0
                && self.sheet_open.load(Ordering::Relaxed)
            {
                let copied_url = match (self.target_load_delay, *self.dispatched_at.lock()) {
                    (Some(delay), Some(dispatched_at)) if dispatched_at.elapsed() < delay => {
                        WRONG_URL
                    }
                    _ => self.copied_url,
                };
                *self.clipboard.lock() = copied_url.into();
                self.copied_urls.lock().push(copied_url);
                self.copies.fetch_add(1, Ordering::Relaxed);
                self.sheet_open.store(false, Ordering::Relaxed);
            } else {
                panic!("unexpected/public tap: {point:?}");
            }
            self.navigation_taps.fetch_add(1, Ordering::Relaxed);
            Ok(())
        }

        async fn swipe(&self, _: SwipeGesture) -> anyhow::Result<()> {
            panic!("exact target proof must not swipe")
        }

        async fn type_text(&self, _: &str) -> anyhow::Result<()> {
            panic!("exact target proof must not type")
        }

        async fn home(&self) -> anyhow::Result<()> {
            panic!("exact target proof has no Home prerequisite")
        }

        async fn back(&self) -> anyhow::Result<()> {
            panic!("copy auto-dismissed; Back would leave the post")
        }

        async fn find_and_tap(&self, _: &str) -> anyhow::Result<()> {
            panic!("exact target proof uses measured rectangles")
        }

        async fn assert_visible(&self, _: &str) -> anyhow::Result<()> {
            panic!("exact target proof uses measured rectangles")
        }

        fn stream_url(&self) -> Option<String> {
            None
        }

        async fn set_clipboard(&self, _: &str, bytes: &[u8]) -> anyhow::Result<()> {
            self.clipboard_writes.fetch_add(1, Ordering::Relaxed);
            anyhow::ensure!(!self.clipboard_write_fails, "clipboard transport failed");
            *self.clipboard.lock() = String::from_utf8(bytes.to_vec())?;
            Ok(())
        }

        async fn get_clipboard(&self, _: usize) -> anyhow::Result<(String, Vec<u8>)> {
            Ok((
                "plaintext".into(),
                self.clipboard.lock().as_bytes().to_vec(),
            ))
        }

        async fn locate(&self, query: ElementQuery<'_>) -> anyhow::Result<Option<ElementBox>> {
            if !self.is_visible() {
                return Ok(None);
            }
            if let ElementQuery::Text { value, exact: true } = query {
                return Ok(self
                    .unavailable_text
                    .filter(|text| *text == value)
                    .map(|text| node(text, 1600.0)));
            }
            let ElementQuery::Description { value, .. } = query else {
                return Ok(None);
            };
            if value == labels().label(TikTokControl::Share).unwrap().value() {
                return Ok(Some(node("Share", 500.0)));
            }
            if value == labels().label(TikTokControl::Comments).unwrap().value() {
                if self.comments_missing
                    || (self.first_intent_lost_to_live && self.opens.lock().len() < 2)
                {
                    return Ok(None);
                }
                return Ok(Some(node(value, 300.0)));
            }
            if value
                == labels()
                    .label(TikTokControl::AuthorProfileLink)
                    .unwrap()
                    .value()
                || value == labels().label(TikTokControl::Follow).unwrap().value()
            {
                if self.author_missing {
                    return Ok(None);
                }
                let changed_once = self.copies.load(Ordering::Relaxed) > 0
                    && self.transient_author_change.swap(false, Ordering::Relaxed);
                let author = if changed_once || (self.card_changes_after_copy
                    && self.copies.load(Ordering::Relaxed) % 2 == 1)
                {
                    "Different Author"
                } else {
                    "fixture"
                };
                let description = if value
                    == labels()
                        .label(TikTokControl::AuthorProfileLink)
                        .unwrap()
                        .value()
                {
                    format!("{author} profile")
                } else {
                    format!("Follow {author}")
                };
                return Ok(Some(node(&description, 100.0)));
            }
            Ok(None)
        }

        async fn locate_all(&self, query: ElementQuery<'_>) -> anyhow::Result<Vec<ElementBox>> {
            Ok(match query {
                ElementQuery::Description {
                    value: "Copy link",
                    exact: true,
                } if self.sheet_open.load(Ordering::Relaxed) => {
                    vec![node("Copy link", 1000.0)]
                }
                _ => vec![],
            })
        }

        async fn locate_all_described(
            &self,
            query: ElementQuery<'_>,
        ) -> anyhow::Result<Vec<ElementBox>> {
            Ok(match query {
                ElementQuery::ResourceIdSuffix(":id/desc") if self.is_visible() => {
                    anyhow::ensure!(!self.caption_query_fails, "measured caption read failed");
                    self.caption_nodes
                        .iter()
                        .map(|caption| node(caption, 1400.0))
                        .collect()
                }
                ElementQuery::ClassName("android.widget.TextView") if self.is_visible() => self
                    .fallback_caption
                    .map(|caption| vec![node(caption, 1400.0)])
                    .unwrap_or_default(),
                _ => vec![],
            })
        }
    }

    async fn run(session: &ExactSession, stop: &AtomicBool) -> anyhow::Result<TargetArrival> {
        let target = crate::parse_tiktok_links(TARGET_URL)
            .remove(0)
            .target
            .unwrap();
        open_exact_target_by_hierarchy(session, labels(), PACKAGE, &target, stop).await
    }

    #[tokio::test(start_paused = true)]
    async fn shared_cleanup_navigation_pins_package_and_refuses_wrong_canonical_post() {
        let target = crate::parse_tiktok_links(TARGET_URL)
            .remove(0)
            .target
            .unwrap();
        let session = ExactSession::new(false, TARGET_URL);
        let proof = crate::interaction_campaign::open_hierarchy_target_confirmed(
            &session,
            labels(),
            &target,
            PACKAGE,
        )
        .await
        .unwrap();
        assert!(matches!(
            proof,
            crate::interaction_campaign::TargetProof::Identified
        ));
        assert_eq!(
            *session.opens.lock(),
            vec![(TARGET_URL.into(), PACKAGE.into())]
        );

        let wrong = ExactSession::new(false, WRONG_URL);
        assert!(
            crate::interaction_campaign::open_hierarchy_target_confirmed(
                &wrong,
                labels(),
                &target,
                PACKAGE,
            )
            .await
            .is_err()
        );
    }

    #[tokio::test(start_paused = true)]
    async fn already_target_requires_exact_link_and_dispatches_no_url() {
        let session = ExactSession::new(true, TARGET_URL);
        assert!(matches!(
            run(&session, &AtomicBool::new(false)).await.unwrap(),
            TargetArrival::Identified { .. }
        ));
        assert!(session.opens.lock().is_empty());
        assert_eq!(session.navigation_taps.load(Ordering::Relaxed), 2);
    }

    #[tokio::test(start_paused = true)]
    async fn pinned_readback_dispatches_before_probing_the_initial_feed() {
        let session = ExactSession::new(true, TARGET_URL);
        let target = crate::parse_tiktok_links(TARGET_URL)
            .remove(0)
            .target
            .unwrap();
        let result = open_pinned_target_by_hierarchy(
            &session,
            labels(),
            PACKAGE,
            &target,
            &AtomicBool::new(false),
        )
        .await
        .unwrap();
        assert!(matches!(result, TargetArrival::Identified { .. }));
        assert_eq!(
            *session.opens.lock(),
            vec![(TARGET_URL.into(), PACKAGE.into())]
        );
        let dispatched = session.dispatched_at.lock().unwrap();
        assert!(session
            .tapped_at
            .lock()
            .iter()
            .all(|tap| *tap >= dispatched + DISPATCH_SETTLE));
        assert_eq!(session.copies.load(Ordering::Relaxed), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn pinned_readback_still_rejects_wrong_post_and_observes_cancellation() {
        let target = crate::parse_tiktok_links(TARGET_URL)
            .remove(0)
            .target
            .unwrap();
        let wrong = ExactSession::new(true, WRONG_URL);
        assert!(open_pinned_target_by_hierarchy(
            &wrong,
            labels(),
            PACKAGE,
            &target,
            &AtomicBool::new(false),
        )
        .await
        .is_err());
        let stopped = ExactSession::new(true, TARGET_URL);
        assert!(open_pinned_target_by_hierarchy(
            &stopped,
            labels(),
            PACKAGE,
            &target,
            &AtomicBool::new(true),
        )
        .await
        .is_err());
        assert!(stopped.opens.lock().is_empty());
        assert_eq!(stopped.navigation_taps.load(Ordering::Relaxed), 0);
    }

    #[tokio::test(start_paused = true)]
    async fn lost_cold_intent_reopens_only_the_same_link_from_measured_live_feed() {
        let mut session = ExactSession::new(false, TARGET_URL);
        session.first_intent_lost_to_live = true;
        assert!(run(&session, &AtomicBool::new(false)).await.is_ok());
        assert_eq!(
            *session.opens.lock(),
            vec![
                (TARGET_URL.into(), PACKAGE.into()),
                (TARGET_URL.into(), PACKAGE.into())
            ]
        );
        assert_eq!(session.navigation_taps.load(Ordering::Relaxed), 2);
    }

    #[tokio::test(start_paused = true)]
    async fn slow_target_after_cold_launch_waits_for_exact_link_without_redispatch() {
        let mut session = ExactSession::new(false, TARGET_URL);
        session.target_load_delay = Some(Duration::from_secs(20));
        let before = Instant::now();
        assert!(run(&session, &AtomicBool::new(false)).await.is_ok());
        assert!(before.elapsed() >= Duration::from_secs(20));
        assert!(before.elapsed() < CARD_WINDOW);
        assert_eq!(session.opens.lock().len(), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn cold_start_needs_no_baseline_and_opens_pinned_target_once() {
        let session = ExactSession::new(false, TARGET_URL);
        assert!(run(&session, &AtomicBool::new(false)).await.is_ok());
        assert_eq!(
            *session.opens.lock(),
            vec![(TARGET_URL.into(), PACKAGE.into())]
        );
        assert_eq!(session.navigation_taps.load(Ordering::Relaxed), 2);
    }

    #[tokio::test(start_paused = true)]
    async fn pinned_dispatch_settles_before_copied_link_probe() {
        let session = ExactSession::new(false, TARGET_URL);
        assert!(run(&session, &AtomicBool::new(false)).await.is_ok());
        let dispatched_at = session.dispatched_at.lock().unwrap();
        let first_tap = session.tapped_at.lock()[0];
        assert_eq!(first_tap.duration_since(dispatched_at), DISPATCH_SETTLE);
        assert_eq!(session.copies.load(Ordering::Relaxed), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn same_author_but_wrong_post_never_authorizes_public_actions() {
        let session = ExactSession::new(true, WRONG_URL);
        let error = run(&session, &AtomicBool::new(false)).await.unwrap_err();
        assert!(format!("{error:#}").contains("different post"));
        let mismatch = error.downcast_ref::<TargetLinkMismatch>().unwrap();
        assert_eq!(mismatch.expected_url, TARGET_URL);
        assert_eq!(mismatch.observed_url, WRONG_URL);
        // The measured fallback may reopen this exact pinned link once, after waiting.
        assert_eq!(*session.opens.lock(), vec![
            (TARGET_URL.into(), PACKAGE.into()),
            (TARGET_URL.into(), PACKAGE.into()),
        ]);
        assert!(session.navigation_taps.load(Ordering::Relaxed) >= 4);
    }

    #[tokio::test(start_paused = true)]
    async fn cancellation_before_dispatch_does_not_open_or_tap() {
        let session = ExactSession::new(true, TARGET_URL);
        let error = run(&session, &AtomicBool::new(true)).await.unwrap_err();
        assert!(error.to_string().contains("cancelled"));
        assert!(session.opens.lock().is_empty());
        assert_eq!(session.navigation_taps.load(Ordering::Relaxed), 0);
    }

    #[tokio::test(start_paused = true)]
    async fn no_foreground_times_out_without_public_tap_or_redispatch() {
        let mut session = ExactSession::new(false, TARGET_URL);
        session.foreground = false;
        let before = Instant::now();
        let error = run(&session, &AtomicBool::new(false)).await.unwrap_err();
        assert!(error.to_string().contains("no readable post"));
        assert_eq!(before.elapsed(), CARD_WINDOW);
        assert_eq!(session.opens.lock().len(), 1);
        assert_eq!(session.navigation_taps.load(Ordering::Relaxed), 0);
    }

    #[tokio::test(start_paused = true)]
    async fn changing_card_during_copy_remains_a_refusal() {
        // A discarded pre-effect proof may be reacquired once from fresh observations.
        let transient = ExactSession::new(false, TARGET_URL);
        transient.transient_author_change.store(true, Ordering::Relaxed);
        let started = Instant::now();
        assert!(matches!(
            run(&transient, &AtomicBool::new(false)).await
                .expect("transient continuity loss needs one fresh exact-link proof"),
            TargetArrival::Identified { .. }
        ));
        assert_eq!(transient.opens.lock().len(), 1, "do not redispatch VIEW");
        assert_eq!(transient.copies.load(Ordering::Relaxed), 2);
        assert_eq!(transient.navigation_taps.load(Ordering::Relaxed), 4);
        assert!(started.elapsed() < CARD_WINDOW, "do not reset the deadline");

        let mut session = ExactSession::new(true, TARGET_URL);
        session.card_changes_after_copy = true;
        let error = run(&session, &AtomicBool::new(false)).await.unwrap_err();
        assert!(format!("{error:#}").contains("card changed"));
        assert_eq!(session.opens.lock().len(), 1);
        assert_eq!(session.navigation_taps.load(Ordering::Relaxed), 6);
    }

    #[tokio::test(start_paused = true)]
    async fn short_measured_caption_still_requires_an_exact_copied_url() {
        let mut session = ExactSession::new(true, TARGET_URL);
        session.caption_nodes = vec!["Short"];
        assert!(run(&session, &AtomicBool::new(false)).await.is_ok());
        assert!(session.opens.lock().is_empty());
        assert_eq!(session.navigation_taps.load(Ordering::Relaxed), 2);
        session.copied_url = WRONG_URL;
        let error = run(&session, &AtomicBool::new(false)).await.unwrap_err();
        assert!(format!("{error:#}").contains("different post"));
    }

    #[tokio::test(start_paused = true)]
    async fn duplicate_measured_caption_nodes_never_open_share_or_use_fallback() {
        let mut session = ExactSession::new(true, TARGET_URL);
        session.caption_nodes = vec!["Short", "Short"];
        session.fallback_caption = Some(CAPTION);
        assert!(run(&session, &AtomicBool::new(false)).await.is_err());
        assert_eq!(session.navigation_taps.load(Ordering::Relaxed), 0);
        assert_eq!(session.opens.lock().len(), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn no_measured_caption_node_preserves_long_class_fallback() {
        let mut session = ExactSession::new(true, TARGET_URL);
        session.caption_nodes.clear();
        session.fallback_caption = Some(CAPTION);
        assert!(run(&session, &AtomicBool::new(false)).await.is_ok());
        assert!(session.opens.lock().is_empty());
        assert_eq!(session.navigation_taps.load(Ordering::Relaxed), 2);
    }

    #[tokio::test(start_paused = true)]
    async fn empty_or_failed_measured_caption_read_cannot_use_class_fallback() {
        for query_fails in [false, true] {
            let mut session = ExactSession::new(true, TARGET_URL);
            session.caption_nodes = vec!["   "];
            session.caption_query_fails = query_fails;
            session.fallback_caption = Some(CAPTION);
            assert!(run(&session, &AtomicBool::new(false)).await.is_err());
            assert_eq!(session.navigation_taps.load(Ordering::Relaxed), 0);
        }
    }

    #[tokio::test(start_paused = true)]
    async fn short_class_text_is_not_a_measured_caption() {
        let mut session = ExactSession::new(true, TARGET_URL);
        session.caption_nodes.clear();
        session.fallback_caption = Some("Short");
        assert!(run(&session, &AtomicBool::new(false)).await.is_err());
        assert_eq!(session.navigation_taps.load(Ordering::Relaxed), 0);
    }

    #[tokio::test(start_paused = true)]
    async fn slow_old_card_waits_for_exact_url_without_redispatching() {
        let mut session = ExactSession::new(false, TARGET_URL);
        session.target_load_delay = Some(Duration::from_millis(1600));
        let before = Instant::now();
        assert!(run(&session, &AtomicBool::new(false)).await.is_ok());
        assert_eq!(session.opens.lock().len(), 1);
        let copied = session.copied_urls.lock();
        assert_eq!(copied.first(), Some(&WRONG_URL));
        assert_eq!(copied.last(), Some(&TARGET_URL));
        assert!(before.elapsed() >= Duration::from_millis(1600));
        assert!(before.elapsed() < CARD_WINDOW);
    }

    #[tokio::test(start_paused = true)]
    async fn proof_transport_failure_is_not_retried_as_a_loading_card() {
        let mut session = ExactSession::new(false, TARGET_URL);
        session.clipboard_write_fails = true;
        let before = Instant::now();
        let error = run(&session, &AtomicBool::new(false)).await.unwrap_err();
        assert!(format!("{error:#}").contains("clipboard transport failed"));
        assert_eq!(session.opens.lock().len(), 1);
        assert_eq!(session.clipboard_writes.load(Ordering::Relaxed), 1);
        assert_eq!(session.navigation_taps.load(Ordering::Relaxed), 0);
        assert_eq!(before.elapsed(), DISPATCH_SETTLE);
    }

    #[tokio::test(start_paused = true)]
    async fn explicit_unavailable_notice_after_dispatch_stops_before_copying() {
        let mut session = ExactSession::new(false, TARGET_URL);
        session.unavailable_text = Some(UNAVAILABLE_TEXT);
        let before = Instant::now();
        let error = run(&session, &AtomicBool::new(false)).await.unwrap_err();
        assert!(matches!(
            error.downcast_ref::<CardObservationError>(),
            Some(CardObservationError::PostUnavailable)
        ));
        assert!(error.to_string().starts_with("target_post_unavailable:"));
        assert_eq!(session.opens.lock().len(), 1);
        assert_eq!(session.navigation_taps.load(Ordering::Relaxed), 0);
        assert_eq!(session.clipboard_writes.load(Ordering::Relaxed), 0);
        assert_eq!(before.elapsed(), DISPATCH_SETTLE);
    }

    #[tokio::test(start_paused = true)]
    async fn unavailable_notice_must_match_exactly_and_target_app_must_be_foreground() {
        let mut session = ExactSession::new(false, TARGET_URL);
        session.unavailable_text = Some("This video is unavailable sometimes");
        assert!(run(&session, &AtomicBool::new(false)).await.is_ok());

        let mut session = ExactSession::new(false, TARGET_URL);
        session.foreground = false;
        session.unavailable_text = Some(UNAVAILABLE_TEXT);
        let error = run(&session, &AtomicBool::new(false)).await.unwrap_err();
        assert!(error.to_string().contains("foreground:"));
        assert!(!error.to_string().contains("target_post_unavailable"));
        assert_eq!(session.navigation_taps.load(Ordering::Relaxed), 0);
    }

    #[tokio::test(start_paused = true)]
    async fn missing_card_predicates_are_reported_individually() {
        for predicate in ["Comments:", "author:", "caption:"] {
            let mut session = ExactSession::new(false, TARGET_URL);
            match predicate {
                "Comments:" => session.comments_missing = true,
                "author:" => session.author_missing = true,
                "caption:" => session.caption_nodes = vec![""],
                _ => unreachable!(),
            }
            let error = run(&session, &AtomicBool::new(false)).await.unwrap_err();
            assert!(error.to_string().contains(predicate), "{error:#}");
            assert_eq!(session.opens.lock().len(), 1);
            assert_eq!(session.navigation_taps.load(Ordering::Relaxed), 0);
        }
    }

    #[test]
    fn diagnostic_target_urls_remove_query_fragment_and_bound_length() {
        let diagnostic =
            super::super::target_diagnostic_url(&format!("{TARGET_URL}?token=private#private"));
        assert_eq!(diagnostic, TARGET_URL);
        assert_eq!(
            super::super::target_diagnostic_url("https://user:private@www.tiktok.com/@a/photo/1"),
            "<invalid target URL>"
        );
        let long_url = format!("https://www.tiktok.com/@{}/photo/1", "a".repeat(800));
        assert_eq!(
            super::super::target_diagnostic_url(&long_url)
                .chars()
                .count(),
            512
        );
    }
}

#[cfg(test)]
mod detail_route_tests {
    use std::sync::atomic::{AtomicU64, AtomicUsize};

    use parking_lot::Mutex;

    use super::*;
    use crate::driver::ElementBox;
    use crate::tiktok_labels::controls_for;
    use crate::{SwipeGesture, TapPoint};

    const GLOBAL: &str = "com.zhiliaoapp.musically";
    const LAUNCHER: &str = "com.sec.android.app.launcher";
    const TARGET_URL: &str = "https://www.tiktok.com/@fixture/photo/7694508176024112405";
    const DETAIL_LINK: &str = "snssdk1233://aweme/detail/7694508176024112405";

    /// Screens shaped like the 45.7.3/en probe dumps (B2g feed, B3e detail), names replaced.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Screen {
        Launcher,
        Feed,
        Detail(&'static str),
        /// A detail page whose author changes on every read.
        Flapping,
        Unavailable,
    }

    struct DetailPhone {
        screen: Mutex<Screen>,
        after_open: Screen,
        after_restart: Screen,
        events: Mutex<Vec<String>>,
        generation: AtomicU64,
        restarts: AtomicUsize,
    }

    impl DetailPhone {
        fn new(start: Screen, after_open: Screen) -> Self {
            Self {
                screen: Mutex::new(start),
                after_open,
                after_restart: Screen::Feed,
                events: Mutex::new(Vec::new()),
                generation: AtomicU64::new(0),
                restarts: AtomicUsize::new(0),
            }
        }

        fn opens(&self) -> Vec<String> {
            self.events
                .lock()
                .iter()
                .filter_map(|event| event.strip_prefix("open ").map(str::to_string))
                .collect()
        }

        fn xml(&self, generation: u64) -> String {
            let node = |id: &str, text: &str, desc: &str, bounds: &str| {
                format!(
                    r#"<node package="{GLOBAL}" resource-id="{GLOBAL}:id/{id}" class="android.view.View" text="{text}" content-desc="{desc}" displayed="true" enabled="true" clickable="true" bounds="{bounds}"/>"#
                )
            };
            let card = |author: &str, caption: &str| {
                [
                    node(
                        "user_avatar",
                        "",
                        &format!("{author} profile"),
                        "[942,1011][1059,1128]",
                    ),
                    node(
                        "i_m",
                        "",
                        &format!("Follow {author}"),
                        "[921,1082][1080,1171]",
                    ),
                    node(
                        "eb7",
                        "",
                        "Read or add comments. 3 comments",
                        "[912,1329][1080,1487]",
                    ),
                    node("fpv", "", "Share video. 0 shares", "[912,1645][1080,1803]"),
                    node("desc", caption, "", "[32,1711][870,1861]"),
                ]
                .concat()
            };
            let back = node("bj1", "", "Back", "[15,63][151,199]");
            let body = match *self.screen.lock() {
                Screen::Launcher => format!(
                    r#"<node package="{LAUNCHER}" text="Phone" bounds="[0,1900][200,2000]"/>"#
                ),
                Screen::Feed => [
                    node("nr_", "", "Home", "[0,1965][216,2094]"),
                    node("nrb", "", "Profile", "[864,1965][1080,2094]"),
                    card(
                        "Feed Author",
                        "Some other post that happened to be on the feed",
                    ),
                ]
                .concat(),
                Screen::Detail(author) => [
                    back,
                    card(author, "The target caption, as the detail page shows it"),
                    node("e7q", "Add comment...", "", "[64,1976][1036,2054]"),
                ]
                .concat(),
                Screen::Flapping => {
                    let author = if generation.is_multiple_of(2) {
                        "Even"
                    } else {
                        "Odd"
                    };
                    [back, card(author, "A caption")].concat()
                }
                Screen::Unavailable => [
                    back,
                    node("u", "This video is unavailable", "", "[100,900][980,960]"),
                ]
                .concat(),
            };
            format!("<hierarchy>{body}</hierarchy>")
        }

        fn found(&self, query: ElementQuery<'_>, described_by_text: bool) -> Vec<ElementBox> {
            let generation = self.generation.load(Ordering::Relaxed).max(1);
            let tree = Tree::parse(crate::HierarchySourceSnapshot {
                generation,
                xml: self.xml(generation),
            })
            .unwrap();
            let key = if described_by_text {
                "text"
            } else {
                "content-desc"
            };
            tree.matching(GLOBAL, query)
                .into_iter()
                .filter_map(|index| {
                    let node = &tree.nodes[index];
                    let mut found = node.rect()?;
                    found.description =
                        Some(node.attr(key).to_string()).filter(|value| !value.is_empty());
                    Some(found)
                })
                .collect()
        }
    }

    #[async_trait::async_trait]
    impl UiSession for DetailPhone {
        async fn active_app_bundle(&self) -> anyhow::Result<String> {
            Ok(match *self.screen.lock() {
                Screen::Launcher => LAUNCHER.into(),
                _ => GLOBAL.into(),
            })
        }

        async fn hierarchy_source_snapshot(
            &self,
        ) -> anyhow::Result<crate::HierarchySourceSnapshot> {
            let generation = self.generation.fetch_add(1, Ordering::Relaxed) + 1;
            Ok(crate::HierarchySourceSnapshot {
                generation,
                xml: self.xml(generation),
            })
        }

        async fn open_url_in_app(&self, url: &str, bundle: &str) -> anyhow::Result<()> {
            assert_eq!(bundle, GLOBAL);
            self.events.lock().push(format!("open {url}"));
            *self.screen.lock() = self.after_open;
            Ok(())
        }

        async fn reopen_url_in_app(&self, url: &str, _: &str) -> anyhow::Result<()> {
            panic!("the detail route dispatches without clearing the task: {url}")
        }

        async fn restart_app(&self, bundle: &str) -> anyhow::Result<()> {
            assert_eq!(bundle, GLOBAL);
            self.events.lock().push("restart".into());
            self.restarts.fetch_add(1, Ordering::Relaxed);
            *self.screen.lock() = self.after_restart;
            Ok(())
        }

        async fn tap(&self, point: TapPoint) -> anyhow::Result<()> {
            panic!("a known link needs no tap, so no Share or Copy link: {point:?}")
        }

        async fn set_clipboard(&self, _: &str, _: &[u8]) -> anyhow::Result<()> {
            panic!("a known link never touches the clipboard")
        }

        async fn get_clipboard(&self, _: usize) -> anyhow::Result<(String, Vec<u8>)> {
            panic!("a known link never reads the clipboard")
        }

        async fn locate(&self, query: ElementQuery<'_>) -> anyhow::Result<Option<ElementBox>> {
            Ok(self.found(query, false).into_iter().next())
        }

        async fn locate_all(&self, query: ElementQuery<'_>) -> anyhow::Result<Vec<ElementBox>> {
            Ok(self.found(query, false))
        }

        async fn locate_all_described(
            &self,
            query: ElementQuery<'_>,
        ) -> anyhow::Result<Vec<ElementBox>> {
            Ok(self.found(query, true))
        }

        fn supports_accessibility_readback(&self) -> bool {
            true
        }

        async fn swipe(&self, _: SwipeGesture) -> anyhow::Result<()> {
            panic!("exact target proof must not swipe")
        }

        async fn type_text(&self, _: &str) -> anyhow::Result<()> {
            panic!("exact target proof must not type")
        }

        async fn home(&self) -> anyhow::Result<()> {
            panic!("exact target proof has no Home prerequisite")
        }

        async fn back(&self) -> anyhow::Result<()> {
            panic!("exact target proof does not navigate back")
        }

        async fn find_and_tap(&self, _: &str) -> anyhow::Result<()> {
            panic!("exact target proof uses no generic tap")
        }

        async fn assert_visible(&self, _: &str) -> anyhow::Result<()> {
            panic!("exact target proof uses no generic assert")
        }

        fn stream_url(&self) -> Option<String> {
            None
        }
    }

    fn global() -> TikTokControls {
        controls_for(GLOBAL, "en", "45.7.3").unwrap()
    }

    fn target() -> ResolvedTikTokTarget {
        crate::parse_tiktok_links(TARGET_URL)
            .remove(0)
            .target
            .unwrap()
    }

    async fn exact(phone: &DetailPhone) -> anyhow::Result<TargetArrival> {
        open_exact_target_by_hierarchy(phone, global(), GLOBAL, &target(), &AtomicBool::new(false))
            .await
    }

    #[tokio::test(start_paused = true)]
    async fn known_link_opens_the_detail_route_and_never_touches_the_share_sheet() {
        // The campaign entry point: the watch and Like proofs both go through it.
        let phone = DetailPhone::new(Screen::Feed, Screen::Detail("Fixture"));
        let proof = crate::interaction_campaign::open_hierarchy_target_confirmed(
            &phone,
            global(),
            &target(),
            GLOBAL,
        )
        .await
        .unwrap();
        assert!(matches!(
            proof,
            crate::interaction_campaign::TargetProof::Identified
        ));
        assert_eq!(phone.opens(), vec![DETAIL_LINK.to_string()]);
        assert_eq!(phone.restarts.load(Ordering::Relaxed), 0);

        // Comment verification and inspection open the pinned target the same way.
        let phone = DetailPhone::new(Screen::Launcher, Screen::Detail("Fixture"));
        let arrival = open_pinned_target_by_hierarchy(
            &phone,
            global(),
            GLOBAL,
            &target(),
            &AtomicBool::new(false),
        )
        .await
        .unwrap();
        assert_eq!(
            arrival,
            TargetArrival::Identified {
                author_label: "Fixture".into()
            }
        );
        // TikTok was not foreground, so a stale page could be waiting in its task.
        assert_eq!(
            *phone.events.lock(),
            vec!["restart".to_string(), format!("open {DETAIL_LINK}")]
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_detail_page_left_from_an_earlier_proof_is_cleared_before_the_link() {
        // Watch then Like: the second proof starts on the page the first one opened.
        let phone = DetailPhone::new(Screen::Detail("Fixture"), Screen::Detail("Fixture"));
        assert!(matches!(
            exact(&phone).await.unwrap(),
            TargetArrival::Identified { .. }
        ));
        assert_eq!(
            *phone.events.lock(),
            vec!["restart".to_string(), format!("open {DETAIL_LINK}")]
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_detail_page_that_survives_the_restart_refuses_before_any_dispatch() {
        let mut phone = DetailPhone::new(Screen::Detail("Someone Else"), Screen::Detail("Fixture"));
        phone.after_restart = Screen::Detail("Someone Else");
        let error = exact(&phone).await.unwrap_err();
        assert!(
            error.to_string().contains("still open before the link"),
            "{error:#}"
        );
        assert!(phone.opens().is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn a_link_that_lands_on_the_feed_is_not_proof_of_the_post() {
        // The https form lands on For You (probe B2g) and a swallowed link leaves the feed
        // up. Either way the card on screen is not authorized by the id.
        let phone = DetailPhone::new(Screen::Feed, Screen::Feed);
        let error = exact(&phone).await.unwrap_err();
        assert!(
            error.to_string().contains("detail page not shown"),
            "{error:#}"
        );
        assert_eq!(
            phone.opens(),
            vec![DETAIL_LINK.to_string(), DETAIL_LINK.to_string()]
        );
    }

    #[tokio::test(start_paused = true)]
    async fn an_unstable_or_unavailable_detail_page_fails_closed() {
        let phone = DetailPhone::new(Screen::Feed, Screen::Flapping);
        let error = exact(&phone).await.unwrap_err();
        assert!(
            error.to_string().contains("did not hold still"),
            "{error:#}"
        );
        assert_eq!(phone.opens(), vec![DETAIL_LINK.to_string()]);

        let phone = DetailPhone::new(Screen::Feed, Screen::Unavailable);
        let error = exact(&phone).await.unwrap_err();
        assert!(matches!(
            error.downcast_ref::<CardObservationError>(),
            Some(CardObservationError::PostUnavailable)
        ));
        assert_eq!(phone.opens(), vec![DETAIL_LINK.to_string()]);
    }
}
