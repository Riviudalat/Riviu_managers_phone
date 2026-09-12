//! Reading back the link to a post we just published.
//!
//! # A post that went out and whose link could not be read is **not** a failed post
//!
//! The single rule this module exists to hold. These are two different outcomes and they
//! have to be recorded differently, because one of them is retryable and the other is a
//! carousel already sitting on a real account with no delete path on Android. So **nothing
//! here returns `Err`**: [`capture_post_link`] hands back a [`LinkCapture`], every unhappy
//! variant of which means *the post is fine and the link is missing*. Transport failures are
//! included in that — an `Err` would be indistinguishable from any other error a caller
//! might retry, and retrying a publish is what nothing here may cause.
//!
//! The link's own destination is [`crate::db::publish_sheet`], which is likewise built so a
//! missing row never reopens a published post.
//!
//! # The clipboard is shared state, and a sentinel is what makes it evidence
//!
//! `Copy link` writes to the device clipboard, so reading it back proves nothing by itself:
//! if the tap missed, the clipboard still holds **whatever was there before** — most likely
//! the previous post's link in the same run — and a naive read records that stale URL
//! against this post. The sheet would then be wrong in the one way nobody catches, because
//! every row in it looks like a valid TikTok link.
//!
//! Comparing against what was there before is not enough either, and the reason is worth
//! stating: reading the old value can *fail*, and a version of this treated an unreadable
//! baseline as an empty one — after which any stale value counted as a change. That is the
//! unsafe direction on the one path that must fail closed.
//!
//! So the baseline is **written, not observed**. A unique sentinel goes into the clipboard
//! first; the copy row is tapped; the clipboard must then hold something that is *not* the
//! sentinel. A value that is still the sentinel proves the tap did not land, and a failure
//! to write the sentinel at all is a refusal rather than a guess.
//!
//! **What that still does not prove**, stated plainly so nobody reads more into it: the
//! sentinel shows the clipboard changed *after this function wrote it*, not that TikTok
//! wrote it, and not that the link belongs to the post the caller has in mind. The caller
//! must already have the intended post on screen — and the route from a just-published
//! carousel back to its own post page is **not measured on any build**, which is why nothing
//! in the publish path calls this yet.

use std::time::Duration;

use crate::driver::{ElementBox, ElementQuery, UiSession};
use crate::tiktok_labels::{TikTokControl, TikTokControls};

pub(crate) mod hierarchy;
mod verification;
pub use verification::{
    capture_submission_link, probe_clipboard_restore, PublishVerificationPlan, VerificationCapture,
    VerificationDiagnostic, VerificationReason,
};

/// How long the share sheet may take to come up.
pub const SHEET_WINDOW: Duration = Duration::from_millis(6_000);
/// How long the clipboard may take to change after `Copy link` is tapped.
pub const CLIPBOARD_WINDOW: Duration = Duration::from_millis(4_000);
pub const POLL: Duration = Duration::from_millis(300);
/// Clipboard reads are capped; a TikTok link is far under this.
const CLIPBOARD_LIMIT: usize = 4_096;

/// The strings the copy row carries, lower-cased, across the builds seen so far.
///
/// **Preferred as whole labels, accepted as substrings only when exactly one row matches.**
/// The distinction is a real defect this list used to carry: a share sheet can hold
/// `Copy link` *and* `Copy link to profile`, both of which contain `copy link`, and taking
/// the first in hierarchy order chooses between them by luck. When a substring matches more
/// than one row this refuses instead.
///
/// Adding a language here is not a measurement, but it is not free either — see above. A
/// needle that is a prefix of some other row's label makes that row a candidate.
pub const COPY_ROW_NEEDLES: [&str; 3] = ["copy link", "sao chép liên kết", "sao chép link"];

/// The clipboard kinds a copied link may arrive as.
///
/// Checked because the kind is part of the evidence and was previously thrown away: `Copy
/// link` produces plain text, so a payload arriving as something else is an unexplained
/// transition rather than a link. The Android helper normalises to `plaintext` today, which
/// makes this latent on that backend and not on the trait.
fn is_text_kind(kind: &str) -> bool {
    let kind = kind.to_ascii_lowercase();
    kind.contains("text") || kind.is_empty()
}

/// What reading the link achieved. **No variant here means the post failed.**
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkCapture {
    /// The clipboard changed away from a sentinel this function wrote, and holds something
    /// shaped like a post link.
    Captured(String),
    /// This build has no measured Share control, so nothing was tapped.
    ShareUnmeasured,
    /// Share is measured and was not on screen.
    NoShareControl,
    /// The clipboard could not be primed, so no baseline exists to compare against.
    ///
    /// Refuses rather than falling back to reading whatever is there: without a written
    /// baseline, a copy tap that misses is indistinguishable from one that lands.
    ClipboardUnwritable(String),
    /// Share was tapped and the sheet never showed a copy row.
    NoCopyRow,
    /// More than one row could be the copy row, and none matched a needle exactly.
    ///
    /// A share sheet holding both `Copy link` and `Copy link to profile` is the measured
    /// shape of this. Choosing by hierarchy order would put a profile URL in the sheet.
    AmbiguousCopyRow,
    /// The copy row was tapped and the clipboard still holds the sentinel.
    ///
    /// Which is the proof the tap did not land — a much stronger statement than the old
    /// "the value did not change", and it cannot be confused with a stale link.
    CopyDidNotLand,
    /// The clipboard changed into something that is not a link to a post.
    NotAPostLink(String),
    /// A tap or a hierarchy read failed. The post is unaffected.
    ///
    /// Carried as a variant rather than an `Err` on purpose: see the module docs.
    ReadFailed(String),
}

impl LinkCapture {
    /// The link, if there is one.
    pub fn link(&self) -> Option<&str> {
        match self {
            Self::Captured(link) => Some(link),
            _ => None,
        }
    }

    pub fn reason(&self) -> String {
        match self {
            Self::Captured(link) => format!("đã lấy link: {link}"),
            Self::ShareUnmeasured => "chưa đo nút Chia sẻ trên bản build này".into(),
            Self::NoShareControl => "không thấy nút Chia sẻ trên màn hình".into(),
            Self::ClipboardUnwritable(reason) => {
                format!("không ghi được clipboard nên chưa lấy link: {reason}")
            }
            Self::NoCopyRow => "bảng chia sẻ không có dòng sao chép liên kết".into(),
            Self::AmbiguousCopyRow => {
                "bảng chia sẻ có nhiều hơn một dòng giống 'sao chép liên kết' — không đoán".into()
            }
            Self::CopyDidNotLand => {
                "TikTok chưa trả liên kết sau khi bấm sao chép; sẽ kiểm tra lại sau".into()
            }
            Self::NotAPostLink(value) => {
                format!("clipboard đổi nhưng không phải link bài: {value:.80}")
            }
            Self::ReadFailed(message) => format!("không đọc được link ({message}) — bài vẫn ổn"),
        }
    }
}

/// Whether a clipboard value is a link to a **post**, as opposed to anywhere else on TikTok.
///
/// # The host alone is not enough, which is what this used to check
///
/// TikTok's share sheet can copy a profile, a sound page, a search, or the homepage, and all
/// of them carry the same host. A predicate that stopped at the host called every one of them
/// a post link — so a mis-tapped share row put `https://www.tiktok.com/@account` into the
/// operator's sheet, where it reads as a perfectly ordinary row.
///
/// So the path is inspected too, and only two shapes are accepted:
///
/// * a canonical post: a `/video/<id>` or `/photo/<id>` segment on a `tiktok.com` host;
/// * a **short link**: `vt.` or `vm.tiktok.com` with a non-empty path, which is what the
///   share sheet produces on some builds. The destination cannot be checked without
///   following it, and that is left to whoever needs it.
///
/// Parsed with a real URL parser rather than by hand. The hand-rolled version accepted
/// `https://.tiktok.com/` and `https://foo..tiktok.com/`, and rejected
/// `https://www.tiktok.com:443/@a/video/1` and an upper-case scheme — because what it called
/// the host was the whole authority, userinfo and port included.
pub fn looks_like_a_post_link(value: &str) -> bool {
    let Ok(parsed) = url::Url::parse(value.trim()) else {
        return false;
    };
    if !matches!(parsed.scheme(), "http" | "https") {
        return false;
    }
    // A copied post link never carries credentials, and allowing them would let
    // `https://www.tiktok.com@evil.com/` through on a less careful host check.
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return false;
    }
    let Some(host) = parsed.host_str() else {
        return false;
    };
    // A trailing dot is the same host in DNS; the parser keeps it, so normalise it here.
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    if host.split('.').any(str::is_empty) {
        return false;
    }
    let on_tiktok = host == "tiktok.com" || host.ends_with(".tiktok.com");
    if !on_tiktok {
        return false;
    }
    let segments: Vec<&str> = parsed
        .path_segments()
        .map(|parts| parts.filter(|part| !part.is_empty()).collect())
        .unwrap_or_default();
    if matches!(host.as_str(), "vt.tiktok.com" | "vm.tiktok.com") {
        // The share sheet's short form. Opaque by design — the post id is on the far side of
        // a redirect — so the most that can be checked is that it addresses something.
        return !segments.is_empty();
    }
    // `/@handle/video/<id>` and `/@handle/photo/<id>`, without pinning the handle's position:
    // builds differ on what precedes it, and the pair that identifies a post does not.
    segments
        .windows(2)
        .any(|pair| matches!(pair[0], "video" | "photo") && !pair[1].is_empty())
}

/// Resolve only TikTok HTTPS redirects; retain the canonical post path, not tracking data.
pub async fn resolve_canonical_post_link(value: &str) -> anyhow::Result<String> {
    let mut url = url::Url::parse(value.trim())?;
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(8))
        .build()?;
    for _ in 0..5 {
        anyhow::ensure!(
            url.scheme() == "https"
                && url.username().is_empty()
                && url.password().is_none()
                && matches!(
                    url.host_str(),
                    Some(
                        "www.tiktok.com"
                            | "tiktok.com"
                            | "m.tiktok.com"
                            | "vt.tiktok.com"
                            | "vm.tiktok.com"
                    )
                )
                && url.port_or_known_default() == Some(443),
            "untrusted TikTok link redirect"
        );
        if let Some(link) = canonical_post_path(&url) {
            return Ok(link);
        }
        anyhow::ensure!(
            matches!(url.host_str(), Some("vt.tiktok.com" | "vm.tiktok.com")),
            "TikTok redirect did not identify a post"
        );
        let response = client.get(url.clone()).send().await?;
        anyhow::ensure!(
            response.status().is_redirection(),
            "TikTok short link did not redirect"
        );
        let location = response
            .headers()
            .get(reqwest::header::LOCATION)
            .ok_or_else(|| anyhow::anyhow!("TikTok redirect location missing"))?
            .to_str()?;
        url = url.join(location)?;
    }
    anyhow::bail!("TikTok redirect limit exceeded")
}

fn canonical_post_path(url: &url::Url) -> Option<String> {
    if !matches!(
        url.host_str(),
        Some("www.tiktok.com" | "tiktok.com" | "m.tiktok.com")
    ) {
        return None;
    }
    crate::interaction::parse_tiktok_links(url.as_str())
        .into_iter()
        .next()?
        .target
        .map(|target| target.normalized_url)
}

/// How long the profile grid may take to render after its tab is tapped.
pub const PROFILE_WINDOW: Duration = Duration::from_millis(30_000);
/// How long a tapped tile may take to become a post page carrying its caption.
pub const POST_PAGE_WINDOW: Duration = Duration::from_millis(6_000);
/// How many tiles the route will open looking for the caption it was given.
///
/// Three, because the failure it must survive is a pinned post plus a badge this build does
/// not carry an id for. Beyond that the answer is "the post is not here", and opening more
/// of an operator's own grid to keep guessing is not a better answer.
const TILES_TO_TRY: usize = 3;
/// How much of the caption is demanded on the page.
///
/// A prefix, not the whole string: TikTok truncates a long caption in the grid page's own
/// `TextView` and appends its own "more", so demanding the full text would refuse exactly
/// the posts whose captions are worth checking. Short enough to survive truncation, long
/// enough that two different bundles do not collide.
const CAPTION_PROOF_CHARS: usize = 24;

/// The result of routing to **our own** just-published post and reading its link.
///
/// Separate from [`LinkCapture`] because the route has failures the sheet does not, and
/// because the important one is new: standing on the wrong post. Every variant is a
/// statement about the *link*, never about the post — by the time this runs the carousel is
/// already out, and nothing here can change that.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OwnPostLink {
    /// A post carrying the expected caption was opened, and its link read.
    Captured(String),
    /// This build has no measured profile tab, so the route was never started.
    ProfileTabUnmeasured,
    /// The profile tab is measured and was not on screen.
    ProfileTabMissing,
    /// This build has no measured post tile, so the grid cannot be read.
    TilesUnmeasured,
    /// The profile opened and showed no tiles at all.
    NoTiles,
    /// The caller passed nothing to identify the post by.
    ///
    /// Refuses rather than trusting position: "the newest post is the first tile" is false
    /// on any account with a pinned post, and this one has one.
    CaptionUnusable,
    /// Tiles were opened and none carried the expected caption.
    CaptionNotFound,
    /// A measured profile shows drafts, but none of the opened posts proved this submission.
    DraftsPresent(usize),
    /// Own profile could not be proved or the copied link names a different account.
    AccountUnverified,
    SubmissionUnverified,
    /// The route reached our post; the sheet capture then said this.
    Sheet(LinkCapture),
    /// A tap or a read failed on the way. The post is unaffected.
    ReadFailed(String),
}

impl OwnPostLink {
    pub fn reason_code(&self) -> &'static str {
        match self {
            Self::Captured(_) => "verified",
            Self::ReadFailed(_) => "readFailed",
            Self::AccountUnverified => "accountUnverified",
            Self::SubmissionUnverified => "submissionUnverified",
            Self::Sheet(_) => "linkUnavailable",
            Self::ProfileTabUnmeasured | Self::TilesUnmeasured => "unmeasuredLayout",
            Self::ProfileTabMissing => "profileUnavailable",
            Self::CaptionUnusable => "captionUnusable",
            Self::DraftsPresent(_) => "draftsObserved",
            Self::NoTiles | Self::CaptionNotFound => "postNotVisible",
        }
    }
    /// The link, if there is one.
    pub fn link(&self) -> Option<&str> {
        match self {
            Self::Captured(link) => Some(link),
            Self::Sheet(capture) => capture.link(),
            _ => None,
        }
    }

    pub fn reason(&self) -> String {
        match self {
            Self::Captured(link) => format!("đã lấy link bài của mình: {link}"),
            Self::ProfileTabUnmeasured => "chưa đo tab Hồ sơ trên bản build này".into(),
            Self::ProfileTabMissing => "không thấy tab Hồ sơ trên màn hình".into(),
            Self::TilesUnmeasured => "chưa đo ô bài trên lưới hồ sơ của bản build này".into(),
            Self::NoTiles => "lưới hồ sơ không có ô bài nào".into(),
            Self::CaptionUnusable => {
                "không có caption để nhận ra bài của lượt này — KHÔNG lấy link theo vị trí, \
                 vì ô đầu tiên có thể là bài ghim"
                    .into()
            }
            Self::CaptionNotFound => {
                "mở các ô đầu lưới mà không ô nào mang caption của lượt này — có thể bài \
                 chưa hiện xong, hoặc caption bị sửa"
                    .into()
            }
            Self::DraftsPresent(count) => format!(
                "Hồ sơ có {count} bản nháp; chưa tìm thấy bài khớp lượt đã gửi. Bản nháp có thể thuộc lượt khác; cần kiểm tra trên máy"
            ),
            Self::AccountUnverified => {
                "chưa xác nhận tài khoản của bài; chờ kiểm tra lại liên kết".into()
            }
            Self::SubmissionUnverified => {
                "bài trên màn hình chưa khớp nội dung và thời điểm gửi của lượt này".into()
            }
            Self::Sheet(capture) => capture.reason(),
            Self::ReadFailed(message) => {
                format!("Lỗi đọc màn hình khi xác minh bài: {message}")
            }
        }
    }
}

/// The prefix of a caption that a post page must render for the post to count as ours.
///
/// Cut on a **character** boundary, not a byte one: every caption this fleet writes is
/// Vietnamese, and slicing `"Đà Lạt"` by bytes panics.
fn caption_proof(caption: &str) -> Option<String> {
    let trimmed = caption.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(trimmed.chars().take(CAPTION_PROOF_CHARS).collect())
}

/// Whether a tile's rectangle contains a badge's — i.e. the badge belongs to that tile.
fn contains(tile: &ElementBox, badge: &ElementBox) -> bool {
    badge.x >= tile.x
        && badge.y >= tile.y
        && badge.x + badge.width <= tile.x + tile.width
        && badge.y + badge.height <= tile.y + tile.height
}

/// Route from wherever the phone is to **our own** post, prove it is ours, and read its link.
///
/// # Why the caption is the proof
///
/// Measured 31/08/2026 (§9.136), on the fleet's own account: the first tile in the profile
/// grid is a **pinned** post from months ago. "Newest is first" is simply false, and an
/// account's own pinned post is still an account's own post — so ownership cannot separate
/// them either. What separates them is what the operator just wrote: the caption of *this*
/// run. It is checked on the page before the share sheet is opened, and a page that does not
/// carry it is backed out of rather than copied from.
///
/// # It never returns `Err`, and it never changes the post
///
/// Same contract as [`capture_post_link`], for the same reason: the carousel is already
/// published by the time this runs, and a failure to read its link must not read as a
/// failure to publish. Every tap it makes is navigation — a tab, a tile, Share, a copy row —
/// and it walks back out of every tile it opens.
pub async fn capture_own_post_link(
    session: &dyn UiSession,
    labels: &TikTokControls,
    caption: &str,
) -> OwnPostLink {
    capture_own_post_link_internal(session, labels, caption, None).await
}

#[derive(Debug, Clone)]
pub struct SubmissionIdentity {
    pub account: String,
    pub submitted_at: String,
}

/// Read the authenticated own account before composing, then return to the measured Home tab.
pub async fn observe_publish_account(
    session: &dyn UiSession,
    labels: &TikTokControls,
) -> anyhow::Result<String> {
    let profile = labels
        .label(TikTokControl::ProfileTab)
        .ok_or_else(|| anyhow::anyhow!("profile tab unmeasured"))?;
    let tab = session
        .locate(profile.to_query())
        .await?
        .ok_or_else(|| anyhow::anyhow!("profile tab absent"))?;
    session.tap(tab.centre()).await?;
    let deadline = tokio::time::Instant::now() + PROFILE_WINDOW;
    let account = loop {
        if let Some(account) = crate::tiktok_account::observe_own_account(session, *labels).await? {
            break account;
        }
        anyhow::ensure!(
            tokio::time::Instant::now() < deadline,
            "own account was not proven before Post"
        );
        tokio::time::sleep(POLL).await;
    };
    let home = labels
        .label(TikTokControl::HomeTab)
        .ok_or_else(|| anyhow::anyhow!("Home tab unmeasured"))?;
    let tab = session
        .locate(home.to_query())
        .await?
        .ok_or_else(|| anyhow::anyhow!("Home tab absent"))?;
    session.tap(tab.centre()).await?;
    Ok(account)
}

pub async fn capture_own_post_link_for_submission(
    session: &dyn UiSession,
    labels: &TikTokControls,
    caption: &str,
    identity: &SubmissionIdentity,
) -> OwnPostLink {
    if identity.account.trim().is_empty()
        || chrono::DateTime::parse_from_rfc3339(&identity.submitted_at).is_err()
    {
        return OwnPostLink::SubmissionUnverified;
    }
    capture_own_post_link_internal(session, labels, caption, Some(identity)).await
}

async fn capture_own_post_link_internal(
    session: &dyn UiSession,
    labels: &TikTokControls,
    caption: &str,
    identity: Option<&SubmissionIdentity>,
) -> OwnPostLink {
    let Some(proof) = caption_proof(caption) else {
        return OwnPostLink::CaptionUnusable;
    };
    let Some(profile) = labels.label(TikTokControl::ProfileTab) else {
        return OwnPostLink::ProfileTabUnmeasured;
    };
    let Some(tile_id) = labels.post_tile_id() else {
        return OwnPostLink::TilesUnmeasured;
    };

    let tab = match session.locate(profile.to_query()).await {
        Ok(Some(tab)) => tab,
        Ok(None) => return OwnPostLink::ProfileTabMissing,
        Err(error) => return OwnPostLink::ReadFailed(error.to_string()),
    };
    if let Err(error) = session.tap(tab.centre()).await {
        return OwnPostLink::ReadFailed(error.to_string());
    }

    let tiles = match await_tiles(session, tile_id.to_query()).await {
        Ok(tiles) if tiles.is_empty() => return OwnPostLink::NoTiles,
        Ok(tiles) => tiles,
        Err(error) => return OwnPostLink::ReadFailed(error.to_string()),
    };
    // A profile navigation is not an identity proof. Read the owned account twice before
    // considering tiles; a link for a different handle must never settle this assignment.
    let account = match crate::tiktok_account::observe_own_account(session, *labels).await {
        Ok(Some(account)) => account,
        Ok(None) => return OwnPostLink::AccountUnverified,
        Err(error) => return OwnPostLink::ReadFailed(error.to_string()),
    };
    if identity.is_some_and(|identity| {
        !identity
            .account
            .trim_start_matches('@')
            .eq_ignore_ascii_case(&account)
    }) {
        return OwnPostLink::AccountUnverified;
    }
    // Cheap skip, when this build's badge is measured: a pinned tile is a page load that
    // can be known not to be ours before it is spent. Identity is still the caption.
    let pinned = match labels.pinned_badge_id() {
        Some(badge) => session
            .locate_all(badge.to_query())
            .await
            .unwrap_or_default(),
        None => Vec::new(),
    };
    // A Drafts cover opens the composer, not a published post. Read its measured
    // badge before spending the tile budget; a failed read must not tap that cover.
    let drafts = match labels.draft_badge_id() {
        Some(badge) => match session.locate_all_described(badge.to_query()).await {
            Ok(badges) => badges,
            Err(error) => return OwnPostLink::ReadFailed(error.to_string()),
        },
        None => Vec::new(),
    };
    let observed_drafts = drafts
        .iter()
        .filter_map(|badge| {
            badge
                .description
                .as_deref()?
                .trim()
                .strip_prefix("Drafts:")?
                .trim()
                .parse::<usize>()
                .ok()
        })
        .fold(0usize, usize::saturating_add);
    let candidates: Vec<ElementBox> = tiles
        .into_iter()
        .filter(|tile| !pinned.iter().any(|badge| contains(tile, badge)))
        .filter(|tile| !drafts.iter().any(|badge| contains(tile, badge)))
        .take(TILES_TO_TRY)
        .collect();
    if candidates.is_empty() {
        return if observed_drafts > 0 {
            OwnPostLink::DraftsPresent(observed_drafts)
        } else {
            OwnPostLink::CaptionNotFound
        };
    }

    let mut submission_unverified = false;
    for tile in candidates {
        if let Err(error) = session.tap(tile.centre()).await {
            return OwnPostLink::ReadFailed(error.to_string());
        }
        match await_caption(session, &proof).await {
            Ok(true) => {
                if let Some(identity) = identity {
                    match submission_visible(session, labels, caption, identity).await {
                        Ok(true) => {}
                        Ok(false) => {
                            submission_unverified = true;
                            leave_post_page(session, labels).await;
                            continue;
                        }
                        Err(error) => {
                            leave_post_page(session, labels).await;
                            return OwnPostLink::ReadFailed(error.to_string());
                        }
                    }
                }
                let capture = capture_post_link(session, labels).await;
                // Out of the post page whatever the sheet said, so the next thing to run
                // does not start inside somebody's post.
                leave_post_page(session, labels).await;
                return match capture {
                    LinkCapture::Captured(link) => match resolve_canonical_post_link(&link).await {
                        Ok(canonical) if canonical_account_matches(&canonical, &account) => {
                            OwnPostLink::Captured(canonical)
                        }
                        Ok(_) => OwnPostLink::AccountUnverified,
                        Err(error) => OwnPostLink::ReadFailed(error.to_string()),
                    },
                    other => OwnPostLink::Sheet(other),
                };
            }
            Ok(false) => leave_post_page(session, labels).await,
            Err(error) => {
                leave_post_page(session, labels).await;
                return OwnPostLink::ReadFailed(error);
            }
        }
    }
    if submission_unverified {
        OwnPostLink::SubmissionUnverified
    } else if observed_drafts > 0 {
        OwnPostLink::DraftsPresent(observed_drafts)
    } else {
        OwnPostLink::CaptionNotFound
    }
}

fn canonical_account_matches(link: &str, account: &str) -> bool {
    let Ok(url) = url::Url::parse(link) else {
        return false;
    };
    url.path_segments()
        .and_then(|mut segments| segments.next())
        .and_then(|segment| segment.strip_prefix('@'))
        .is_some_and(|handle| handle.eq_ignore_ascii_case(account.trim_start_matches('@')))
}

/// Wait for the profile grid to have tiles, and hand them back in hierarchy order.
async fn await_tiles(
    session: &dyn UiSession,
    query: ElementQuery<'_>,
) -> anyhow::Result<Vec<ElementBox>> {
    let deadline = tokio::time::Instant::now() + PROFILE_WINDOW;
    loop {
        let tiles = session.locate_all(query).await?;
        if !tiles.is_empty() {
            return Ok(tiles);
        }
        if tokio::time::Instant::now() >= deadline {
            return Ok(Vec::new());
        }
        tokio::time::sleep(POLL).await;
    }
}

/// Whether the page now on screen renders the caption this run wrote.
async fn await_caption(session: &dyn UiSession, proof: &str) -> Result<bool, String> {
    let query = ElementQuery::Text {
        value: proof,
        exact: false,
    };
    let deadline = tokio::time::Instant::now() + POST_PAGE_WINDOW;
    loop {
        match session.locate(query).await {
            Ok(Some(_)) => return Ok(true),
            Ok(None) => {}
            Err(error) => return Err(error.to_string()),
        }
        if tokio::time::Instant::now() >= deadline {
            return Ok(false);
        }
        tokio::time::sleep(POLL).await;
    }
}

/// Back out of a post page until the profile grid is under us again.
///
/// Proof is the grid's own tiles: the tab bar is on both screens, so "the tab bar is back"
/// says nothing. Bounded, and a failure to get back is left to the caller's own walk-back —
/// this function's job is not to be the last line of defence.
async fn leave_post_page(session: &dyn UiSession, labels: &TikTokControls) {
    let Some(tile) = labels.post_tile_id() else {
        let _ = session.back().await;
        return;
    };
    for _ in 0..3 {
        if session
            .locate_all(tile.to_query())
            .await
            .map(|tiles| !tiles.is_empty())
            .unwrap_or(false)
        {
            return;
        }
        let _ = session.back().await;
        tokio::time::sleep(POLL).await;
    }
}

/// Tap Share, tap the copy row, and read the link back off the clipboard.
///
/// Puts the share sheet away **only when it opened one**, and never returns `Err`.
///
/// **Standing on the intended post is the caller's job**, and on the publish path that
/// caller is [`capture_own_post_link`] — never `post_through_the_composer` directly. After
/// Post, TikTok is on the feed, where Share belongs to a stranger's video and the link it
/// copies is a real post link that nothing downstream can tell from ours.
pub async fn capture_post_link(session: &dyn UiSession, labels: &TikTokControls) -> LinkCapture {
    let Some(share) = labels.label(TikTokControl::Share) else {
        // Nothing was opened, so there is nothing to close.
        return LinkCapture::ShareUnmeasured;
    };
    let mut opened = false;
    let outcome = read_through_sheet(session, share.to_query(), &mut opened).await;
    if opened {
        // Only when a sheet is actually up. An unconditional Back used to fire even on
        // `NoShareControl`, which navigates away from the very post the caller was on.
        close_sheet(session).await;
    }
    outcome
}

/// A value nothing else would ever put in a clipboard, unique per capture.
///
/// Not random for its own sake: it has to be distinguishable from *the previous capture's*
/// sentinel too, or a run whose second copy tap misses would compare against a stale
/// sentinel and read the first post's link as the second's.
fn sentinel() -> String {
    format!("riviu-clipboard-sentinel-{}", uuid::Uuid::new_v4())
}

async fn read_through_sheet(
    session: &dyn UiSession,
    share: ElementQuery<'_>,
    opened: &mut bool,
) -> LinkCapture {
    let control = match session.locate(share).await {
        Ok(Some(control)) => control,
        Ok(None) => match crate::ui_automation::runtime::resolve_navigation(
            session,
            "share",
            Duration::from_secs(30),
        )
        .await
        {
            Ok(Some(control)) => control,
            Ok(None) => return LinkCapture::NoShareControl,
            Err(error) => return LinkCapture::ReadFailed(error.to_string()),
        },
        Err(error) => return LinkCapture::ReadFailed(error.to_string()),
    };

    // **Written, not observed.** See the module docs: an unreadable prior value used to
    // become an empty baseline, after which any stale link counted as a change.
    let mark = sentinel();
    if let Err(error) = session.set_clipboard("plaintext", mark.as_bytes()).await {
        return LinkCapture::ClipboardUnwritable(format!("{error:#}"));
    }

    if let Err(error) = session.tap(control.centre()).await {
        // The sheet may or may not be up; assume it is, so the caller closes it.
        *opened = true;
        return LinkCapture::ReadFailed(error.to_string());
    }
    *opened = true;

    let row = match await_copy_row(session).await {
        CopyRow::Found(point) => point,
        CopyRow::Missing => return LinkCapture::NoCopyRow,
        CopyRow::Ambiguous => return LinkCapture::AmbiguousCopyRow,
        CopyRow::Failed(message) => return LinkCapture::ReadFailed(message),
    };
    if let Err(error) = session.tap(row).await {
        return LinkCapture::ReadFailed(error.to_string());
    }

    let deadline = tokio::time::Instant::now() + CLIPBOARD_WINDOW;
    loop {
        if let Ok((kind, bytes)) = session.get_clipboard(CLIPBOARD_LIMIT).await {
            let now = String::from_utf8_lossy(&bytes).trim().to_string();
            if now != mark && !now.is_empty() {
                return if !is_text_kind(&kind) {
                    LinkCapture::NotAPostLink(format!("{kind}: {now}"))
                } else if looks_like_a_post_link(&now) {
                    LinkCapture::Captured(now)
                } else {
                    LinkCapture::NotAPostLink(now)
                };
            }
        }
        if tokio::time::Instant::now() >= deadline {
            return LinkCapture::CopyDidNotLand;
        }
        tokio::time::sleep(POLL).await;
    }
}

enum CopyRow {
    Found(crate::types::TapPoint),
    Missing,
    Ambiguous,
    Failed(String),
}

/// Find the copy row in the open share sheet, waiting for the sheet to arrive.
///
/// Reads `text` for every row rather than asking for one label, because the row's string is
/// not in the catalogue.
///
/// **An exact label wins over a substring, and an ambiguous substring refuses.** A sheet
/// holding `Copy link` and `Copy link to profile` matches the needle twice; taking the first
/// in hierarchy order picks between a post link and a profile link by luck, and the profile
/// link then looks entirely ordinary in the operator's sheet.
async fn await_copy_row(session: &dyn UiSession) -> CopyRow {
    let deadline = tokio::time::Instant::now() + SHEET_WINDOW;
    loop {
        // The caption under the icon can lie outside TikTok's clickable hit area.
        // Prefer its exactly labelled parent (Trill 38.3.2 live 2026-09-06).
        let mut controls = Vec::new();
        for label in ["Copy link", "Sao chép liên kết", "Sao chép link"] {
            if let Ok(found) = session
                .locate_all(ElementQuery::Description {
                    value: label,
                    exact: true,
                })
                .await
            {
                controls.extend(found);
            }
        }
        match controls.as_slice() {
            [control] => return CopyRow::Found(control.centre()),
            [_, _, ..] => return CopyRow::Ambiguous,
            [] => {}
        }
        let rows = match session
            .locate_all_described(ElementQuery::ClassName("android.widget.TextView"))
            .await
        {
            Ok(rows) => rows,
            Err(error) => return CopyRow::Failed(error.to_string()),
        };
        let label = |row: &ElementBox| {
            row.description
                .as_deref()
                .map(|value| value.trim().to_lowercase())
        };
        let exact: Vec<&ElementBox> = rows
            .iter()
            .filter(|row| {
                label(row).is_some_and(|value| COPY_ROW_NEEDLES.contains(&value.as_str()))
            })
            .collect();
        if let [row] = exact.as_slice() {
            return CopyRow::Found(row.centre());
        }
        if exact.len() > 1 {
            return CopyRow::Ambiguous;
        }
        let loose: Vec<&ElementBox> = rows
            .iter()
            .filter(|row| {
                label(row).is_some_and(|value| {
                    COPY_ROW_NEEDLES.iter().any(|needle| value.contains(needle))
                })
            })
            .collect();
        match loose.as_slice() {
            [row] => return CopyRow::Found(row.centre()),
            [_, _, ..] => return CopyRow::Ambiguous,
            [] => {}
        }
        if tokio::time::Instant::now() >= deadline {
            return CopyRow::Missing;
        }
        tokio::time::sleep(POLL).await;
    }
}

/// Press Back until the copy row is gone, and say nothing if it will not go.
///
/// A single unverified Back used to be the whole of this, under a doc claiming the sheet was
/// put away on every exit. Leaving it up costs the next step its screen — which is how a
/// probe run once measured the share sheet's nodes and reported them as the comment
/// drawer's.
///
/// **What this cannot cover:** dropping the future — an aborted task — skips it entirely,
/// because there is no async destructor to hang it on.
async fn close_sheet(session: &dyn UiSession) {
    for _ in 0..3 {
        if !matches!(await_copy_row_once(session).await, CopyRow::Found(_)) {
            return;
        }
        let _ = session.back().await;
        tokio::time::sleep(POLL).await;
        if matches!(await_copy_row_once(session).await, CopyRow::Missing) {
            return;
        }
    }
}

/// One look for the copy row, with no waiting. Split out so the close loop does not spend a
/// full sheet window per press.
async fn await_copy_row_once(session: &dyn UiSession) -> CopyRow {
    let rows = match session
        .locate_all_described(ElementQuery::ClassName("android.widget.TextView"))
        .await
    {
        Ok(rows) => rows,
        Err(error) => return CopyRow::Failed(error.to_string()),
    };
    let present = rows.iter().any(|row| {
        row.description.as_deref().is_some_and(|value| {
            let value = value.trim().to_lowercase();
            COPY_ROW_NEEDLES.iter().any(|needle| value.contains(needle))
        })
    });
    if present {
        CopyRow::Found(crate::types::TapPoint { x: 0.0, y: 0.0 })
    } else {
        CopyRow::Missing
    }
}

fn visible_caption_matches(visible: &str, expected: &str) -> bool {
    let normalize = |value: &str| {
        value
            .trim_matches(['\u{200e}', '\u{200f}'])
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
    };
    let expected = normalize(expected);
    let visible = normalize(visible);
    if visible == expected {
        return true;
    }
    // Only TikTok's explicit truncation markers authorize a prefix comparison.
    // EN "…more" and VI "xem thêm" both count; bare "more" in the caption does not.
    let prefix = [
        "…more",
        "...more",
        "… more",
        "... more",
        "…xem thêm",
        "...xem thêm",
        "… xem thêm",
        "... xem thêm",
        "xem thêm",
        "…",
        "...",
    ]
    .iter()
    .find_map(|suffix| visible.strip_suffix(suffix))
    .map(str::trim_end);
    let Some(prefix) = prefix else {
        return false;
    };
    prefix.chars().count() >= expected.chars().count().min(64) && expected.starts_with(prefix)
}

/// Fold Vietnamese letters to ASCII so "phút trước" and "phut truoc" share one parser.
fn fold_vi_ascii(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        let mapped = match ch {
            'à' | 'á' | 'ạ' | 'ả' | 'ã' | 'â' | 'ầ' | 'ấ' | 'ậ' | 'ẩ' | 'ẫ' | 'ă' | 'ằ' | 'ắ'
            | 'ặ' | 'ẳ' | 'ẵ' => 'a',
            'è' | 'é' | 'ẹ' | 'ẻ' | 'ẽ' | 'ê' | 'ề' | 'ế' | 'ệ' | 'ể' | 'ễ' => {
                'e'
            }
            'ì' | 'í' | 'ị' | 'ỉ' | 'ĩ' => 'i',
            'ò' | 'ó' | 'ọ' | 'ỏ' | 'õ' | 'ô' | 'ồ' | 'ố' | 'ộ' | 'ổ' | 'ỗ' | 'ơ' | 'ờ' | 'ớ'
            | 'ợ' | 'ở' | 'ỡ' => 'o',
            'ù' | 'ú' | 'ụ' | 'ủ' | 'ũ' | 'ư' | 'ừ' | 'ứ' | 'ự' | 'ử' | 'ữ' => {
                'u'
            }
            'ỳ' | 'ý' | 'ỵ' | 'ỷ' | 'ỹ' => 'y',
            'đ' => 'd',
            other => other,
        };
        out.push(mapped);
    }
    out
}

fn relative_post_age(text: &str) -> Option<(i64, i64)> {
    let trimmed = text
        .trim()
        .trim_start_matches(['\u{200e}', '\u{200f}'])
        .trim()
        .trim_start_matches('·')
        .trim();
    let ascii = trimmed.to_ascii_lowercase();
    if matches!(ascii.as_str(), "just now" | "now") {
        return Some((0, 60));
    }
    if let Some(age) = ascii.strip_suffix(" ago") {
        let split = age.find(|c: char| !c.is_ascii_digit()).unwrap_or(age.len());
        let amount = age[..split].parse::<i64>().ok()?;
        let unit = match age[split..].trim() {
            "s" | "sec" | "seconds" => 1,
            "m" | "min" | "minutes" => 60,
            "h" | "hr" | "hours" => 3600,
            "d" | "day" | "days" => 86400,
            _ => return None,
        };
        return Some((amount.checked_mul(unit)?, unit));
    }

    let folded = fold_vi_ascii(&trimmed.to_lowercase());
    if matches!(
        folded.as_str(),
        "vua xong" | "vua moi" | "bay gio" | "luc nay"
    ) {
        return Some((0, 60));
    }
    for (suffix, unit) in [
        (" giay truoc", 1_i64),
        (" phut truoc", 60),
        (" gio truoc", 3600),
        (" ngay truoc", 86400),
    ] {
        if let Some(age) = folded.strip_suffix(suffix) {
            let amount = age.trim().parse::<i64>().ok()?;
            return Some((amount.checked_mul(unit)?, unit));
        }
    }
    None
}

fn relative_post_time_matches(
    text: &str,
    submitted_at: &str,
    now: chrono::DateTime<chrono::Utc>,
) -> bool {
    let Ok(submitted) = chrono::DateTime::parse_from_rfc3339(submitted_at) else {
        return false;
    };
    let Some((age, precision)) = relative_post_age(text) else {
        return false;
    };
    let Some(oldest_age) = age
        .checked_add(precision)
        .and_then(chrono::Duration::try_seconds)
    else {
        return false;
    };
    let Some(oldest_creation) = now.checked_sub_signed(oldest_age) else {
        return false;
    };
    // The whole rounded interval must remain after dispatch; waiting never relaxes this proof.
    submitted <= now && oldest_creation >= submitted
}

/// A matching newborn post can have only a few valid seconds per minute. Fixed retries
/// miss that window forever. Wait on that same page, then read BOTH caption and time anew.
fn submission_time_recheck_delay(
    text: &str,
    submitted_at: &str,
    now: chrono::DateTime<chrono::Utc>,
) -> Option<Duration> {
    let submitted = chrono::DateTime::parse_from_rfc3339(submitted_at).ok()?;
    let (age, precision) = relative_post_age(text)?;
    if precision > 60 || submitted > now {
        return None;
    }
    let youngest = now.checked_sub_signed(chrono::Duration::try_seconds(age)?)?;
    if youngest < submitted {
        return None;
    }
    let boundary = submitted
        .checked_add_signed(chrono::Duration::try_seconds(age.checked_add(precision)?)?)?;
    let wait = boundary.signed_duration_since(now).to_std().ok()?;
    (wait <= Duration::from_secs(60)).then_some(wait + Duration::from_millis(150))
}

async fn submission_visible(
    session: &dyn UiSession,
    labels: &TikTokControls,
    caption: &str,
    identity: &SubmissionIdentity,
) -> anyhow::Result<bool> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(65);
    loop {
        let caption_id = match (labels.package(), labels.resource_version()) {
            // AGENTS.md §9.210: 9889db374744474635, own post snapshot 10/09/2026.
            ("com.ss.android.ugc.trill", Some("38.3.2")) => ":id/dmk",
            _ => ":id/desc",
        };
        let captions = session
            .locate_all_described(ElementQuery::ResourceIdSuffix(caption_id))
            .await?;
        let [visible] = captions.as_slice() else {
            return Ok(false);
        };
        if !visible
            .description
            .as_deref()
            .is_some_and(|text| visible_caption_matches(text, caption))
        {
            return Ok(false);
        }
        // Measured 09/09/2026: Global 45.7.3/en uses zwj in the matching post
        // snapshots links/{2,7,8,9}/post-0.xml under target/publish-approved-20260909.
        // The matching 46.0.41, 46.2.1 and 46.4.3 captures retain tv_post_time.
        let time_id = match (
            labels.package(),
            labels.language(),
            labels.resource_version(),
        ) {
            // AGENTS.md §9.209: ce0417141d4decde0c and ce04171435f104080c,
            // fresh carousel and older own posts, 10/09/2026. No timing tolerance change.
            ("com.ss.android.ugc.trill", "en", Some("38.3.2")) => ":id/qrp",
            ("com.zhiliaoapp.musically", "en", Some("45.4.3")) => ":id/zj1",
            ("com.zhiliaoapp.musically", "en", Some("45.7.3")) => ":id/zwj",
            _ => ":id/tv_post_time",
        };
        let times = session
            .locate_all_described(ElementQuery::ResourceIdSuffix(time_id))
            .await?;
        let [time] = times.as_slice() else {
            return Ok(false);
        };
        let Some(text) = time.description.as_deref() else {
            return Ok(false);
        };
        let now = chrono::Utc::now();
        if relative_post_time_matches(text, &identity.submitted_at, now) {
            return Ok(true);
        }
        let Some(delay) = submission_time_recheck_delay(text, &identity.submitted_at, now) else {
            return Ok(false);
        };
        if tokio::time::Instant::now() + delay >= deadline {
            return Ok(false);
        }
        tokio::time::sleep(delay).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tiktok_labels::{controls_for, nothing_measured};
    use crate::types::TapPoint;
    use parking_lot::Mutex;

    /// A phone whose clipboard changes **because something tapped the copy row**.
    ///
    /// The causal model is the whole point of this fake. An earlier one popped a queued
    /// clipboard value on every *read*, so the clipboard appeared to change whether or not
    /// the tap landed — which made a mutation that tapped a point far off screen pass every
    /// test. Here `tap` writes `copies` into the clipboard only when the point falls inside
    /// the copy row's rectangle, exactly as the device does.
    #[derive(Default)]
    struct FakeSession {
        share: Option<ElementBox>,
        rows: Vec<ElementBox>,
        /// What tapping the copy row puts on the clipboard.
        copies: String,
        /// The kind that comes back with it.
        kind: String,
        clipboard: Mutex<Option<(String, String)>>,
        taps: Mutex<Vec<TapPoint>>,
        backs: Mutex<usize>,
        /// The row that a landing tap must hit, if it is not `rows[0]`.
        copy_row: Option<ElementBox>,
        copy_controls: Vec<ElementBox>,
        set_clipboard_fails: bool,
        share_tap_fails: bool,
        /// Once the sheet is dismissed the rows go away, like the real one.
        dismissed: Mutex<bool>,
        auto_dismiss_copy: bool,
        post_nodes: Option<Vec<(String, ElementBox)>>,
    }

    fn labelled(label: &str, y: f64) -> ElementBox {
        ElementBox {
            x: 0.0,
            y,
            width: 400.0,
            height: 100.0,
            description: Some(label.into()),
            enabled: true,
            clickable: true,
        }
    }

    impl FakeSession {
        fn sheet(rows: Vec<ElementBox>, copies: &str) -> Self {
            Self {
                share: Some(labelled("Share", 1200.0)),
                rows,
                copies: copies.to_string(),
                kind: "plaintext".into(),
                ..Default::default()
            }
        }

        fn copying_row(mut self, row: ElementBox) -> Self {
            self.copy_row = Some(row);
            self
        }

        fn primed_with(self, value: &str) -> Self {
            *self.clipboard.lock() = Some(("plaintext".into(), value.to_string()));
            self
        }

        fn hits_copy_row(&self, point: &TapPoint) -> bool {
            let row = self.copy_row.clone().or_else(|| self.rows.first().cloned());
            row.is_some_and(|row| {
                point.x >= row.x
                    && point.x <= row.x + row.width
                    && point.y >= row.y
                    && point.y <= row.y + row.height
            })
        }
    }

    #[async_trait::async_trait]
    impl UiSession for FakeSession {
        async fn locate_all(&self, query: ElementQuery<'_>) -> anyhow::Result<Vec<ElementBox>> {
            if matches!(
                query,
                ElementQuery::Description {
                    value: "Copy link",
                    exact: true
                }
            ) && !*self.dismissed.lock()
            {
                Ok(self.copy_controls.clone())
            } else {
                Ok(Vec::new())
            }
        }
        async fn tap(&self, point: TapPoint) -> anyhow::Result<()> {
            if self.share_tap_fails {
                anyhow::bail!("the agent went away mid-gesture");
            }
            let hit = self.hits_copy_row(&point);
            self.taps.lock().push(point);
            if hit && !self.copies.is_empty() {
                *self.clipboard.lock() = Some((self.kind.clone(), self.copies.clone()));
                if self.auto_dismiss_copy {
                    *self.dismissed.lock() = true;
                }
            }
            Ok(())
        }
        async fn swipe(&self, _gesture: crate::types::SwipeGesture) -> anyhow::Result<()> {
            Ok(())
        }
        async fn type_text(&self, _text: &str) -> anyhow::Result<()> {
            Ok(())
        }
        async fn home(&self) -> anyhow::Result<()> {
            Ok(())
        }
        async fn back(&self) -> anyhow::Result<()> {
            *self.backs.lock() += 1;
            *self.dismissed.lock() = true;
            Ok(())
        }
        async fn find_and_tap(&self, _accessibility_id: &str) -> anyhow::Result<()> {
            Ok(())
        }
        async fn assert_visible(&self, _accessibility_id: &str) -> anyhow::Result<()> {
            Ok(())
        }
        fn stream_url(&self) -> Option<String> {
            None
        }
        fn supports_element_bounds(&self) -> bool {
            true
        }
        async fn set_clipboard(&self, kind: &str, bytes: &[u8]) -> anyhow::Result<()> {
            if self.set_clipboard_fails {
                anyhow::bail!("no clipboard helper on this device");
            }
            *self.clipboard.lock() =
                Some((kind.to_string(), String::from_utf8_lossy(bytes).to_string()));
            Ok(())
        }
        async fn get_clipboard(&self, _limit: usize) -> anyhow::Result<(String, Vec<u8>)> {
            let held = self.clipboard.lock().clone();
            let (kind, value) = held.ok_or_else(|| anyhow::anyhow!("clipboard unreadable"))?;
            Ok((kind, value.into_bytes()))
        }
        async fn locate(&self, _query: ElementQuery<'_>) -> anyhow::Result<Option<ElementBox>> {
            Ok(self.share.clone())
        }
        async fn locate_all_described(
            &self,
            query: ElementQuery<'_>,
        ) -> anyhow::Result<Vec<ElementBox>> {
            if let Some(nodes) = &self.post_nodes {
                return Ok(nodes
                    .iter()
                    .filter(|(id, _)| matches!(query, ElementQuery::ResourceIdSuffix(suffix) if id.ends_with(suffix)))
                    .map(|(_, node)| node.clone())
                    .collect());
            }
            if *self.dismissed.lock() {
                return Ok(Vec::new());
            }
            Ok(self.rows.clone())
        }
    }

    fn english() -> TikTokControls {
        controls_for("com.ss.android.ugc.trill", "en", "").expect("a measured set")
    }

    struct DraftProfileSession {
        sheet: FakeSession,
        page: Mutex<&'static str>,
        draft_tile: ElementBox,
        post_tile: ElementBox,
        draft_badge: ElementBox,
        draft_taps: Mutex<usize>,
        post_taps: Mutex<usize>,
        badge_read_fails: bool,
        post_matches: bool,
    }

    const DRAFT_TEST_CAPTION: &str =
        "Fixture caption identifies the newly submitted carousel exactly";
    const DRAFT_TEST_URL: &str = "https://www.tiktok.com/@fixture.account/photo/123456789";

    impl DraftProfileSession {
        fn measured() -> Self {
            let fixture: serde_json::Value = serde_json::from_str(include_str!(
                "../../../docs/fixtures/tiktok-share/global-46.2.1-draft-profile.json"
            ))
            .unwrap();
            let rectangle = |name: &str, text: Option<String>| {
                let values = fixture[name].as_array().unwrap();
                ElementBox {
                    x: values[0].as_f64().unwrap(),
                    y: values[1].as_f64().unwrap(),
                    width: values[2].as_f64().unwrap(),
                    height: values[3].as_f64().unwrap(),
                    description: text,
                    enabled: true,
                    clickable: true,
                }
            };
            Self {
                sheet: FakeSession::sheet(vec![labelled("Copy link", 1800.0)], DRAFT_TEST_URL),
                page: Mutex::new("feed"),
                draft_tile: rectangle("draftTile", None),
                post_tile: rectangle("postTile", None),
                draft_badge: rectangle(
                    "draftBadge",
                    Some(fixture["draftText"].as_str().unwrap().into()),
                ),
                draft_taps: Mutex::new(0),
                post_taps: Mutex::new(0),
                badge_read_fails: false,
                post_matches: true,
            }
        }
    }

    #[async_trait::async_trait]
    impl UiSession for DraftProfileSession {
        async fn tap(&self, point: TapPoint) -> anyhow::Result<()> {
            let page = *self.page.lock();
            match page {
                "feed" => *self.page.lock() = "profile",
                "profile" if point.x < self.post_tile.x => {
                    *self.draft_taps.lock() += 1;
                    *self.page.lock() = "draft";
                }
                "profile" => {
                    *self.post_taps.lock() += 1;
                    *self.page.lock() = "post";
                }
                "post" => {
                    *self.page.lock() = "sheet";
                    self.sheet.tap(point).await?;
                }
                "sheet" => self.sheet.tap(point).await?,
                _ => {}
            }
            Ok(())
        }
        async fn swipe(&self, _gesture: crate::types::SwipeGesture) -> anyhow::Result<()> {
            anyhow::bail!("unexpected swipe")
        }
        async fn type_text(&self, _text: &str) -> anyhow::Result<()> {
            anyhow::bail!("unexpected text effect")
        }
        async fn home(&self) -> anyhow::Result<()> {
            anyhow::bail!("unexpected Home")
        }
        async fn back(&self) -> anyhow::Result<()> {
            let page = *self.page.lock();
            *self.page.lock() = match page {
                "sheet" => "post",
                "post" => "profile",
                // The measured defect: a draft exit loses the profile grid.
                _ => "feed",
            };
            if page == "sheet" {
                self.sheet.back().await?;
            }
            Ok(())
        }
        async fn find_and_tap(&self, _id: &str) -> anyhow::Result<()> {
            anyhow::bail!("unexpected unmeasured tap")
        }
        async fn assert_visible(&self, _id: &str) -> anyhow::Result<()> {
            Ok(())
        }
        fn stream_url(&self) -> Option<String> {
            None
        }
        async fn active_app_bundle(&self) -> anyhow::Result<String> {
            Ok("com.zhiliaoapp.musically".into())
        }
        async fn hierarchy_source_snapshot(
            &self,
        ) -> anyhow::Result<crate::HierarchySourceSnapshot> {
            anyhow::ensure!(
                *self.page.lock() == "profile",
                "account read outside profile"
            );
            Ok(crate::HierarchySourceSnapshot {
                generation: 1,
                xml: include_str!("../fixtures/tiktok-publish/musically-46.2.1-en/profile.xml")
                    .into(),
            })
        }
        async fn locate(&self, query: ElementQuery<'_>) -> anyhow::Result<Option<ElementBox>> {
            let page = *self.page.lock();
            match page {
                "feed" => Ok(Some(labelled("Profile", 2100.0))),
                "post" => match query {
                    ElementQuery::Text { value, .. }
                        if self.post_matches && DRAFT_TEST_CAPTION.starts_with(value) =>
                    {
                        Ok(Some(labelled(DRAFT_TEST_CAPTION, 1500.0)))
                    }
                    ElementQuery::Text { .. } => Ok(None),
                    _ => Ok(self.sheet.share.clone()),
                },
                _ => Ok(None),
            }
        }
        async fn locate_all(&self, query: ElementQuery<'_>) -> anyhow::Result<Vec<ElementBox>> {
            let page = *self.page.lock();
            if page == "profile" && matches!(query, ElementQuery::ResourceIdSuffix(":id/cover")) {
                return Ok(vec![self.draft_tile.clone(), self.post_tile.clone()]);
            }
            if page == "sheet" {
                return self.sheet.locate_all(query).await;
            }
            Ok(Vec::new())
        }
        async fn locate_all_described(
            &self,
            query: ElementQuery<'_>,
        ) -> anyhow::Result<Vec<ElementBox>> {
            let page = *self.page.lock();
            match (page, query) {
                ("profile", ElementQuery::ResourceIdSuffix(":id/zq_")) => {
                    anyhow::ensure!(!self.badge_read_fails, "draft badge read failed");
                    Ok(vec![self.draft_badge.clone()])
                }
                ("post", ElementQuery::ResourceIdSuffix(":id/desc")) => {
                    Ok(vec![labelled(DRAFT_TEST_CAPTION, 1500.0)])
                }
                ("post", ElementQuery::ResourceIdSuffix(":id/tv_post_time")) => {
                    Ok(vec![labelled("· 6s ago", 1400.0)])
                }
                ("sheet", _) => self.sheet.locate_all_described(query).await,
                _ => Ok(Vec::new()),
            }
        }
        async fn set_clipboard(&self, kind: &str, value: &[u8]) -> anyhow::Result<()> {
            self.sheet.set_clipboard(kind, value).await
        }
        async fn get_clipboard(&self, limit: usize) -> anyhow::Result<(String, Vec<u8>)> {
            self.sheet.get_clipboard(limit).await
        }
    }

    #[tokio::test(start_paused = true)]
    async fn own_profile_drafts_never_open_and_next_post_keeps_submission_proof() {
        let session = DraftProfileSession::measured();
        let labels = controls_for("com.zhiliaoapp.musically", "en", "46.2.1").unwrap();
        let identity = SubmissionIdentity {
            account: "fixture.account".into(),
            submitted_at: (chrono::Utc::now() - chrono::Duration::seconds(30)).to_rfc3339(),
        };
        let result =
            capture_own_post_link_for_submission(&session, &labels, DRAFT_TEST_CAPTION, &identity)
                .await;
        assert_eq!(
            *session.draft_taps.lock(),
            0,
            "the Drafts cover must never be opened"
        );
        assert_eq!(result, OwnPostLink::Captured(DRAFT_TEST_URL.into()));
        assert_eq!(*session.post_taps.lock(), 1);
        assert_eq!(*session.page.lock(), "profile");
    }

    #[tokio::test(start_paused = true)]
    async fn own_profile_drafts_read_failure_stops_before_any_cover_tap() {
        let mut session = DraftProfileSession::measured();
        session.badge_read_fails = true;
        let labels = controls_for("com.zhiliaoapp.musically", "en", "46.2.1").unwrap();
        let result = capture_own_post_link(&session, &labels, DRAFT_TEST_CAPTION).await;
        assert!(matches!(result, OwnPostLink::ReadFailed(_)));
        assert_eq!(*session.draft_taps.lock(), 0);
        assert_eq!(*session.post_taps.lock(), 0);
    }

    #[tokio::test(start_paused = true)]
    async fn own_profile_drafts_report_observation_only_after_no_matching_post() {
        let mut session = DraftProfileSession::measured();
        session.post_matches = false;
        let labels = controls_for("com.zhiliaoapp.musically", "en", "46.2.1").unwrap();
        let result = capture_own_post_link(&session, &labels, DRAFT_TEST_CAPTION).await;
        assert_eq!(result, OwnPostLink::DraftsPresent(1));
        assert!(result.reason().contains("có thể thuộc lượt khác"));
        assert_eq!(*session.draft_taps.lock(), 0);
        assert_eq!(*session.post_taps.lock(), 1);
        assert_eq!(*session.page.lock(), "profile");
    }

    #[test]
    fn draft_badge_mapping_stays_on_the_measured_build_and_language() {
        for (package, language, version, expected) in [
            ("com.zhiliaoapp.musically", "en", "46.2.1", true),
            ("com.zhiliaoapp.musically", "en", "46.4.3", false),
            ("com.zhiliaoapp.musically", "en", "45.7.3", false),
            ("com.zhiliaoapp.musically", "vi", "46.2.1", false),
            ("com.ss.android.ugc.trill", "en", "38.3.2", false),
        ] {
            let mapped =
                controls_for(package, language, version).and_then(|labels| labels.draft_badge_id());
            assert_eq!(mapped.is_some(), expected);
        }
    }

    #[tokio::test(start_paused = true)]
    async fn a_copy_that_auto_dismisses_never_backs_out_of_the_target_post() {
        let session = FakeSession {
            auto_dismiss_copy: true,
            ..FakeSession::sheet(
                vec![labelled("Copy link", 1800.0)],
                "https://www.tiktok.com/@a/photo/2",
            )
        };
        assert_eq!(
            capture_post_link(&session, &english()).await.link(),
            Some("https://www.tiktok.com/@a/photo/2")
        );
        assert_eq!(*session.backs.lock(), 0);
    }

    #[tokio::test]
    async fn canonical_links_strip_tracking_and_reject_foreign_hosts_before_network() {
        assert_eq!(
            resolve_canonical_post_link(
                "https://www.tiktok.com/@fixture/photo/123456789?_r=1#tracking"
            )
            .await
            .unwrap(),
            "https://www.tiktok.com/@fixture/photo/123456789"
        );
        for url in [
            "http://www.tiktok.com/@fixture/photo/123",
            "https://example.com/@fixture/photo/123",
            "https://www.tiktok.com:444/@fixture/photo/123",
            "https://user@www.tiktok.com/@fixture/photo/123",
        ] {
            assert!(resolve_canonical_post_link(url).await.is_err());
        }
    }

    #[tokio::test(start_paused = true)]
    async fn copy_uses_the_accessible_button_not_its_outside_text_caption() {
        let button = labelled("Copy link", 1626.0);
        let mut session = FakeSession::sheet(
            vec![labelled("Copy link", 1800.0)],
            "https://www.tiktok.com/@a/photo/2",
        )
        .copying_row(button.clone());
        session.copy_controls = vec![button];
        assert_eq!(
            capture_post_link(&session, &english()).await.link(),
            Some("https://www.tiktok.com/@a/photo/2")
        );
        assert_eq!(session.taps.lock().len(), 2);
    }

    #[tokio::test(start_paused = true)]
    async fn ambiguous_accessible_copy_buttons_never_fall_back_to_text() {
        let mut session = FakeSession::sheet(
            vec![labelled("Copy link", 1800.0)],
            "https://www.tiktok.com/@a/photo/2",
        );
        session.copy_controls = vec![labelled("Copy link", 1626.0), labelled("Copy link", 1750.0)];
        assert_eq!(
            capture_post_link(&session, &english()).await,
            LinkCapture::AmbiguousCopyRow
        );
        assert_eq!(session.taps.lock().len(), 1);
    }

    // -------------------------------------------------------------- the happy path

    /// The copy row is tapped, the clipboard changes away from the sentinel, and that is the
    /// link.
    #[tokio::test(start_paused = true)]
    async fn tapping_the_copy_row_is_what_produces_the_link() {
        let session = FakeSession::sheet(
            vec![labelled("Copy link", 1800.0)],
            "https://www.tiktok.com/@a/photo/2",
        );
        let capture = capture_post_link(&session, &english()).await;
        assert_eq!(
            capture,
            LinkCapture::Captured("https://www.tiktok.com/@a/photo/2".into())
        );
        assert!(
            *session.backs.lock() > 0,
            "the share sheet must be put away"
        );
    }

    /// **A tap that misses the copy row produces no link at all.**
    ///
    /// The assertion the old fake could not make. Its clipboard advanced on every *read*, so
    /// a version of this module that tapped a point far off screen still passed every test.
    /// Here the clipboard only changes when something lands inside the row, which is what the
    /// device does — so a missed tap leaves the sentinel and the capture refuses.
    #[tokio::test(start_paused = true)]
    async fn a_tap_that_lands_nowhere_leaves_the_sentinel_and_captures_nothing() {
        let session = FakeSession::sheet(
            vec![labelled("Copy link", 1800.0)],
            "https://www.tiktok.com/@a/photo/2",
        )
        // The row the tap must hit is somewhere else entirely, so the real one is a miss.
        .copying_row(labelled("Copy link", 9_000.0));
        let capture = capture_post_link(&session, &english()).await;
        assert_eq!(capture, LinkCapture::CopyDidNotLand);
        assert_eq!(capture.link(), None);
    }

    /// **A stale link is never reported as this post's, even when the baseline read fails.**
    ///
    /// The defect this module is shaped around, in its worst form: the clipboard holds the
    /// previous post's perfectly valid link, and the code cannot read it beforehand. A
    /// version that treated an unreadable baseline as empty accepted the stale value. Here
    /// the baseline is *written*, so there is nothing to fail to read — and if the write
    /// itself fails, that is a refusal.
    #[tokio::test(start_paused = true)]
    async fn the_previous_posts_link_cannot_become_this_posts_link() {
        let stale = "https://www.tiktok.com/@a/photo/1";
        // The copy tap misses; the clipboard already holds a valid link from the last run.
        let session =
            FakeSession::sheet(vec![labelled("Copy link", 1800.0)], "").primed_with(stale);
        let capture = capture_post_link(&session, &english()).await;
        assert_eq!(capture, LinkCapture::CopyDidNotLand);
        assert_eq!(
            capture.link(),
            None,
            "a stale link leaked out as this post's"
        );
    }

    /// And an unwritable clipboard refuses rather than reading whatever is there.
    #[tokio::test(start_paused = true)]
    async fn a_clipboard_that_cannot_be_primed_refuses_instead_of_guessing() {
        let session = FakeSession {
            set_clipboard_fails: true,
            ..FakeSession::sheet(
                vec![labelled("Copy link", 1800.0)],
                "https://www.tiktok.com/@a/photo/2",
            )
        }
        .primed_with("https://www.tiktok.com/@a/photo/1");
        assert_eq!(
            capture_post_link(&session, &english()).await,
            LinkCapture::ClipboardUnwritable("no clipboard helper on this device".into())
        );
        assert!(
            session.taps.lock().is_empty(),
            "nothing should be tapped once the baseline cannot be established"
        );
    }

    // ------------------------------------------------------------- the copy row

    /// **`Copy link` and `Copy link to profile` both match, so neither is chosen by luck.**
    ///
    /// Substring matching plus hierarchy order used to decide this, and the wrong answer is
    /// a profile URL that reads as a perfectly ordinary row in the operator's sheet.
    #[tokio::test(start_paused = true)]
    async fn a_sheet_holding_two_copy_rows_refuses_rather_than_taking_the_first() {
        let session = FakeSession::sheet(
            vec![
                labelled("Copy link to profile", 1500.0),
                labelled("Copy link to sound", 1650.0),
            ],
            "https://www.tiktok.com/@a",
        );
        assert_eq!(
            capture_post_link(&session, &english()).await,
            LinkCapture::AmbiguousCopyRow
        );
    }

    /// An exact label wins over a substring, so the real row is still reachable.
    #[tokio::test(start_paused = true)]
    async fn the_exact_copy_row_wins_over_a_longer_one_that_contains_it() {
        let real = labelled("Copy link", 1800.0);
        let session = FakeSession::sheet(
            vec![labelled("Copy link to profile", 1500.0), real.clone()],
            "https://www.tiktok.com/@a/video/7",
        )
        .copying_row(real.clone());
        assert_eq!(
            capture_post_link(&session, &english()).await.link(),
            Some("https://www.tiktok.com/@a/video/7")
        );
        let taps = session.taps.lock();
        assert_eq!(taps.len(), 2, "Share, then the row");
        assert_eq!(
            (taps[1].x, taps[1].y),
            (real.centre().x, real.centre().y),
            "tapped the profile row instead of the post row"
        );
    }

    /// The row is matched case-insensitively, and in Vietnamese too.
    #[tokio::test(start_paused = true)]
    async fn the_copy_row_is_found_whatever_its_capitalisation_or_language() {
        for label in ["COPY LINK", "Copy Link", "Sao chép liên kết"] {
            let row = labelled(label, 1800.0);
            let session = FakeSession::sheet(
                vec![labelled("Report", 1700.0), row.clone()],
                "https://vt.tiktok.com/ZS1/",
            )
            .copying_row(row.clone());
            assert_eq!(
                capture_post_link(&session, &english()).await.link(),
                Some("https://vt.tiktok.com/ZS1/"),
                "{label} was not recognised"
            );
        }
    }

    // ------------------------------------------------------------ never an error

    /// **A transport failure is a `LinkCapture`, never an `Err`.**
    ///
    /// The post is already live at this point. An `Err` is indistinguishable from any other
    /// error a workflow might retry, and retrying publishes a duplicate that cannot be taken
    /// down.
    #[tokio::test(start_paused = true)]
    async fn a_dead_link_while_reading_is_reported_as_a_missing_link_not_a_failure() {
        let session = FakeSession {
            share_tap_fails: true,
            ..FakeSession::sheet(
                vec![labelled("Copy link", 1800.0)],
                "https://www.tiktok.com/@a/photo/2",
            )
        };
        let capture = capture_post_link(&session, &english()).await;
        assert!(matches!(capture, LinkCapture::ReadFailed(_)), "{capture:?}");
        assert_eq!(capture.link(), None);
        assert!(!capture.reason().is_empty());
    }

    /// **Back is pressed only when a sheet was actually opened.**
    ///
    /// An unconditional Back navigated away from the very post the caller was standing on,
    /// on a path where nothing had been opened at all.
    #[tokio::test(start_paused = true)]
    async fn nothing_is_dismissed_when_no_sheet_was_opened() {
        let session = FakeSession {
            share: None,
            ..FakeSession::sheet(vec![], "")
        };
        assert_eq!(
            capture_post_link(&session, &english()).await,
            LinkCapture::NoShareControl
        );
        assert_eq!(
            *session.backs.lock(),
            0,
            "pressed Back on a screen it never opened"
        );

        let unmeasured = FakeSession::sheet(vec![], "");
        assert_eq!(
            capture_post_link(&unmeasured, &nothing_measured()).await,
            LinkCapture::ShareUnmeasured
        );
        assert_eq!(*unmeasured.backs.lock(), 0);
        assert!(unmeasured.taps.lock().is_empty());
    }

    // --------------------------------------------------------------- the predicate

    /// **A TikTok host is not a TikTok post**, and the difference is what goes in the sheet.
    #[test]
    fn only_a_post_path_counts_as_a_post_link() {
        for good in [
            "https://www.tiktok.com/@a/photo/7668947001618320660",
            "https://www.tiktok.com/@a/video/7668947001618320660",
            "https://tiktok.com/@a/photo/1?is_from_webapp=1",
            "https://www.tiktok.com:443/@a/video/1",
            "HTTPS://www.tiktok.com/@a/video/1",
            "https://www.tiktok.com./@a/video/1",
            // The short form the share sheet produces on some builds.
            "https://vt.tiktok.com/ZS1abc/",
            "https://vm.tiktok.com/ZS1abc/",
        ] {
            assert!(looks_like_a_post_link(good), "{good} should be a post link");
        }
        for bad in [
            "",
            "đi Đà Lạt thật đã",
            "www.tiktok.com/@a/photo/1",
            // Same host, not a post — every one of these used to pass.
            "https://www.tiktok.com/@some_account",
            "https://www.tiktok.com/music/original-sound-123",
            "https://www.tiktok.com/search?q=foo",
            "https://tiktok.com/",
            // Malformed authorities that a hand-rolled suffix check accepted.
            "https://.tiktok.com/@a/video/1",
            "https://foo..tiktok.com/@a/video/1",
            // Lookalike hosts.
            "https://tiktok.com.example.net/@a/video/1",
            "https://nottiktok.com/@a/video/1",
            // Credentials never belong in a copied link, and this is the classic disguise.
            "https://evil.com@www.tiktok.com/@a/video/1",
            "https://tiktok.com@evil.com/@a/video/1",
            // A short link with nothing after the host addresses no post.
            "https://vt.tiktok.com/",
            // Not a web scheme.
            "javascript:alert(1)",
        ] {
            assert!(!looks_like_a_post_link(bad), "{bad} should be refused");
        }
    }

    /// A payload that arrives as something other than text is not a link.
    #[tokio::test(start_paused = true)]
    async fn a_clipboard_payload_that_is_not_text_is_refused() {
        let row = labelled("Copy link", 1800.0);
        let session = FakeSession {
            kind: "image/png".into(),
            ..FakeSession::sheet(vec![row.clone()], "https://www.tiktok.com/@a/video/1")
        }
        .copying_row(row);
        let capture = capture_post_link(&session, &english()).await;
        assert!(
            matches!(capture, LinkCapture::NotAPostLink(_)),
            "{capture:?}"
        );
        assert_eq!(capture.link(), None);
    }

    /// Something that changed into a non-post is named as that rather than stored.
    #[tokio::test(start_paused = true)]
    async fn a_clipboard_that_changed_into_a_profile_is_not_a_post_link() {
        let row = labelled("Copy link", 1800.0);
        let session =
            FakeSession::sheet(vec![row.clone()], "https://www.tiktok.com/@a").copying_row(row);
        let capture = capture_post_link(&session, &english()).await;
        assert!(
            matches!(capture, LinkCapture::NotAPostLink(_)),
            "{capture:?}"
        );
        assert_eq!(capture.link(), None);
    }

    /// **Two captures in a row must not share a sentinel.**
    ///
    /// If they did, a second copy that missed would compare against the first run's mark, and
    /// the first post's link would be read as the second post's — the very failure the
    /// sentinel exists to prevent, reintroduced one run later.
    #[test]
    fn every_capture_writes_a_sentinel_nothing_else_would_hold() {
        let first = sentinel();
        let second = sentinel();
        assert_ne!(first, second);
        assert!(first.starts_with("riviu-clipboard-sentinel-"));
        assert!(
            !looks_like_a_post_link(&first),
            "a sentinel must never be mistaken for a link"
        );
    }

    #[test]
    fn canonical_link_must_name_the_observed_profile() {
        assert!(canonical_account_matches(
            "https://www.tiktok.com/@fixture.account/photo/123",
            "fixture.account"
        ));
        assert!(canonical_account_matches(
            "https://www.tiktok.com/@fixture.account/photo/123",
            "@fixture.account"
        ));
        assert!(!canonical_account_matches(
            "https://www.tiktok.com/@other/photo/123",
            "fixture.account"
        ));
        assert!(!canonical_account_matches(
            "https://vt.tiktok.com/abc/",
            "fixture.account"
        ));
    }

    fn measured_post_time_fixture() -> FakeSession {
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../../docs/fixtures/tiktok-share/global-45.7.3-own-post-time.json"
        ))
        .unwrap();
        FakeSession {
            post_nodes: Some(
                fixture["nodes"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|node| {
                        (
                            node["resourceId"].as_str().unwrap().to_owned(),
                            labelled(node["text"].as_str().unwrap(), 100.0),
                        )
                    })
                    .collect(),
            ),
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn submission_time_locator_reads_measured_build_aliases() {
        let identity = SubmissionIdentity {
            account: "fixture.account".into(),
            submitted_at: (chrono::Utc::now() - chrono::Duration::minutes(3)).to_rfc3339(),
        };
        for (version, time_id) in [
            ("45.4.3", "zj1"),
            ("45.7.3", "zwj"),
            ("46.0.41", "tv_post_time"),
            ("46.2.1", "tv_post_time"),
            ("46.4.3", "tv_post_time"),
        ] {
            let mut session = measured_post_time_fixture();
            session.post_nodes.as_mut().unwrap()[1].0 =
                format!("com.zhiliaoapp.musically:id/{time_id}");
            let labels = controls_for("com.zhiliaoapp.musically", "en", version).unwrap();
            assert!(
                submission_visible(
                    &session,
                    &labels,
                    "Fixture caption for a submitted carousel",
                    &identity,
                )
                .await
                .unwrap(),
                "measured post time missing for {version}"
            );
        }
    }

    #[tokio::test]
    async fn trill_submission_uses_measured_caption_time_and_direction_mark() {
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../../docs/fixtures/tiktok-share/trill-38.3.2-own-post-time.json"
        ))
        .unwrap();
        let mut session = measured_post_time_fixture();
        session.post_nodes = Some(
            fixture["nodes"]
                .as_array()
                .unwrap()
                .iter()
                .map(|n| {
                    (
                        n["resourceId"].as_str().unwrap().into(),
                        labelled(n["text"].as_str().unwrap(), 100.0),
                    )
                })
                .collect(),
        );
        let labels = controls_for("com.ss.android.ugc.trill", "en-US", "38.3.2").unwrap();
        let identity = SubmissionIdentity {
            account: "fixture.account".into(),
            submitted_at: (chrono::Utc::now() - chrono::Duration::hours(18)).to_rfc3339(),
        };
        assert!(submission_visible(
            &session,
            &labels,
            "Fixture caption for a submitted carousel",
            &identity
        )
        .await
        .unwrap());
        session.post_nodes.as_mut().unwrap()[1].1.description = Some("\u{200e} · 2d ago".into());
        assert!(!submission_visible(
            &session,
            &labels,
            "Fixture caption for a submitted carousel",
            &identity
        )
        .await
        .unwrap());
    }

    #[tokio::test]
    async fn submission_time_locator_requires_a_single_time_node() {
        let labels = controls_for("com.zhiliaoapp.musically", "en", "45.7.3").unwrap();
        let identity = SubmissionIdentity {
            account: "fixture.account".into(),
            submitted_at: (chrono::Utc::now() - chrono::Duration::minutes(3)).to_rfc3339(),
        };
        let mut session = measured_post_time_fixture();
        let nodes = session.post_nodes.as_mut().unwrap();
        nodes.push(nodes[1].clone());
        assert!(!submission_visible(
            &session,
            &labels,
            "Fixture caption for a submitted carousel",
            &identity,
        )
        .await
        .unwrap());
        session.post_nodes.as_mut().unwrap().truncate(1);
        assert!(!submission_visible(
            &session,
            &labels,
            "Fixture caption for a submitted carousel",
            &identity,
        )
        .await
        .unwrap());
    }

    #[test]
    fn rounded_time_recheck_targets_the_live_window_without_accepting_old_posts() {
        let at = |value: &str| {
            chrono::DateTime::parse_from_rfc3339(value)
                .unwrap()
                .with_timezone(&chrono::Utc)
        };
        let submitted = "2026-09-09T21:30:56.473Z";
        let now = at("2026-09-09T21:47:47.573Z");
        assert!(!relative_post_time_matches("· 16m ago", submitted, now));
        let delay = submission_time_recheck_delay("· 16m ago", submitted, now).unwrap();
        assert_eq!(delay, Duration::from_millis(9050));
        let rechecked = now + chrono::Duration::from_std(delay).unwrap();
        assert!(relative_post_time_matches(
            "· 16m ago",
            submitted,
            rechecked
        ));
        // A label that advanced during the wait is rechecked and remains unproven.
        assert!(!relative_post_time_matches(
            "· 17m ago",
            submitted,
            rechecked
        ));
        assert!(submission_time_recheck_delay("· 18m ago", submitted, now).is_none());
        assert!(submission_time_recheck_delay("· 1h ago", submitted, now).is_none());
        assert!(submission_time_recheck_delay("yesterday", submitted, now).is_none());
    }

    #[test]
    fn submission_proof_rejects_old_same_prefix_and_retains_visible_caption_identity() {
        let caption = "A measured caption with a long exact opening and unique destination for this submission";
        assert!(visible_caption_matches(caption, caption));
        assert!(visible_caption_matches(
            &format!("{}…more", &caption[..70]),
            caption
        ));
        assert!(!visible_caption_matches(
            "A measured caption with another destination",
            caption
        ));
        assert!(!visible_caption_matches("A measured caption", caption));
        let complete_old = "A".repeat(64);
        assert!(!visible_caption_matches(
            &complete_old,
            &format!("{complete_old} destination NEW")
        ));
        assert!(visible_caption_matches("Visit more", "Visit more"));
        assert!(!visible_caption_matches(
            "Visit more",
            "Visit destination NEW"
        ));
        assert!(!visible_caption_matches(
            &format!("{complete_old}more"),
            &format!("{complete_old} destination NEW")
        ));
        let submitted = "2026-09-09T00:00:00Z";
        let now = chrono::DateTime::parse_from_rfc3339("2026-09-09T00:06:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        assert!(relative_post_time_matches("· 5m ago", submitted, now));
        assert!(!relative_post_time_matches("· 2h ago", submitted, now));
        assert!(!relative_post_time_matches("2026-08-01", submitted, now));
        assert!(!relative_post_time_matches("yesterday", submitted, now));
        assert!(relative_post_time_matches("· 5 phút trước", submitted, now));
        assert!(relative_post_time_matches("5 phut truoc", submitted, now));
        assert!(relative_post_age("vừa xong").is_some());
        assert!(relative_post_age("Vừa xong").is_some());
        assert_eq!(
            relative_post_age("3 giờ trước").map(|v| v.0),
            Some(3 * 3600)
        );
        let vi_caption =
            "Một caption đo được với phần mở đầu dài và đích đến riêng để xác minh bài đăng TikTok tiếng Việt";
        assert!(visible_caption_matches(
            &format!(
                "{}…xem thêm",
                &vi_caption.chars().take(70).collect::<String>()
            ),
            vi_caption
        ));
        assert!(visible_caption_matches(
            &format!(
                "{} xem thêm",
                &vi_caption.chars().take(70).collect::<String>()
            ),
            vi_caption
        ));
        assert!(!visible_caption_matches(
            "Một caption đo được với phần mở đầu khác",
            vi_caption
        ));
    }

    #[test]
    fn relative_time_requires_the_entire_creation_interval_after_submission() {
        let at = |value: &str| {
            chrono::DateTime::parse_from_rfc3339(value)
                .unwrap()
                .with_timezone(&chrono::Utc)
        };
        let submitted = "2026-09-09T10:00:00Z";
        // An existing 09:10 post appears as 1h old at 11:05, but that rounded
        // interval spans the new submission and cannot identify its publication.
        assert!(!relative_post_time_matches(
            "· 1h ago",
            submitted,
            at("2026-09-09T11:05:00Z")
        ));
        assert!(!relative_post_time_matches(
            "· 1m ago",
            submitted,
            at("2026-09-09T10:00:10Z")
        ));
        assert!(!relative_post_time_matches(
            "just now",
            submitted,
            at("2026-09-09T10:00:10Z")
        ));
        // A new upload can settle on a later read as its full time bucket moves
        // after the recorded dispatch; ambiguity is temporary, not publication failure.
        assert!(relative_post_time_matches(
            "· 1m ago",
            submitted,
            at("2026-09-09T10:02:02Z")
        ));
        assert!(relative_post_time_matches(
            "just now",
            submitted,
            at("2026-09-09T10:01:01Z")
        ));
        assert!(!relative_post_time_matches(
            "now",
            submitted,
            at("2026-09-09T09:59:59Z")
        ));
        assert!(!relative_post_time_matches(
            "1m ago",
            "2026-09-09T10:00:00.900Z",
            at("2026-09-09T10:02:00.100Z")
        ));
        assert!(!relative_post_time_matches(
            "9223372036854775807h ago",
            submitted,
            at("2026-09-09T10:02:02Z")
        ));
    }
}
