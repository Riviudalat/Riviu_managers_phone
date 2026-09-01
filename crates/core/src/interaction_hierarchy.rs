//! Locating a specific comment in a hierarchy instead of in pixels.
//!
//! The iOS Interaction path finds the comment it must reply to by OCR-ing the
//! screen (`crate::interaction::locate_parent_comment`). It has to: that transport
//! cannot read the accessibility tree. Android can, so the same *policy* runs over
//! rectangles the device reported rather than over text a reader guessed at.
//!
//! **The policy is ported, not rewritten.** Every rule below exists because the
//! pixel path was bitten by its absence, and the adversarial tests are ported with
//! it. What changes is only the coordinate space — pixels instead of screen
//! fractions — and one thing that gets *better*: the comment body is matched
//! against a string this project typed itself through `ACTION_SET_TEXT`, so there is
//! no transcription loss and the accent-folding apparatus
//! (`normalize_locator_text`, `LATIN_FOLD`) is unnecessary on this path.
//!
//! Measured shape of one comment row on `com.ss.android.ugc.trill` (Redmi Note 12,
//! app 46.3.3, 11/08/2026), which is what the geometry rules encode:
//!
//! ```text
//!   author  android.widget.Button    clickable  text=…   x=174   above the body
//!   body    android.widget.TextView  not click  text=…   x=174
//!   reply   android.widget.Button    clickable  text="Trả lời"  x=315..419, below the body
//!   "Xem N câu trả lời"  Button, not clickable, below the reply
//! ```
//!
//! Row pitch was ~300 px, and the band below one body reaches the next row's reply
//! button — which is exactly why "nearest below", not "first found", is load-bearing.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

// `tokio`'s clock rather than `std`'s, so `tokio::time::pause()` moves the arrival
// deadline as well as the sleeps. Identical behaviour in production — the two only
// diverge under a paused runtime — and without it a test of the 14-second timeout has to
// actually wait 14 seconds, or spin forever because `sleep` was virtual and the deadline
// was not.
use tokio::time::Instant;

use crate::driver::{ElementBox, ElementQuery, UiSession};
use crate::interaction::CommentLocatorIdentity;
use crate::tiktok_labels::{LabelMatch, TikTokControl, TikTokControls};

/// Where to tap to reply to one specific comment, and whose comment it is.
#[derive(Debug, Clone, PartialEq)]
pub struct ElementReplyTarget {
    /// The identity actually observed on screen, so the caller stores what it saw
    /// rather than what it was looking for.
    pub identity: CommentLocatorIdentity,
    /// The reply control, for the touch planner to jitter inside.
    pub reply: ElementBox,
}

/// Vertical slack when deciding whether a label sits "above" a body, in pixels.
///
/// Small on purpose. The author sits ~50 px above its body on the measured build, so
/// this only absorbs a row that has settled a pixel or two mid-scroll.
const ABOVE_SLACK: f64 = 8.0;

/// How far above a body an author label may start and still belong to it.
///
/// Measured gap is ~50 px (author y=1327, body y=1377). 140 px allows for a taller
/// author row — a badge, a second line — while staying well inside the ~300 px row
/// pitch, so it cannot reach the previous row's body.
const AUTHOR_REACH: f64 = 140.0;

/// How far below a body its own reply control may sit.
///
/// Measured gap is ~70 px (body y=1077, reply y=1149). 200 px covers a two-line
/// body while staying under the row pitch — past that lies the *next* comment's
/// reply button, and tapping it posts the reply under a stranger's comment.
const REPLY_REACH: f64 = 200.0;

fn bottom(element: &ElementBox) -> f64 {
    element.y + element.height
}

/// Whether a label is a reply control rather than an author name.
///
/// Every catalogued reply label, checked against all of them rather than against one
/// language's: the author sweep is by widget class, so on a fleet running two UI languages
/// the list can contain either string. Comparing case-insensitively because these are
/// rendered strings, not identifiers.
/// How far an author label's left edge may sit from its comment body's.
///
/// **Measured 25/08/2026 on `com.ss.android.ugc.trill`, en-US, with the drawer open**: every
/// author button and every comment body starts at `x=178`. Nothing in a comment row is
/// indented from that column, so the honest tolerance is a hair, not a horizon.
///
/// It used to be `body.width` — 866 px on that dump — which is not a "comparable left edge"
/// at all, it reaches the whole screen. What lives out there is the *video's* right rail,
/// still in the same tree while the drawer is open, and its counters are bare-number buttons
/// (`28` at `x=966`, `3` at `x=978`, `14` at `x=969` — the post's 28 likes, 3 comments and 14
/// shares). One of them was read as a comment's author on a live run and the reply refused
/// itself with `wanted: "28"`, which is the good outcome of a bad read.
const AUTHOR_LEFT_SLACK: f64 = 24.0;

/// Whether a label is a bare count rather than somebody's name.
///
/// `AUTHOR_LEFT_SLACK` already puts the rail's counters out of reach; this is the second lock
/// on the same door. Geometry is a measurement of one build, and "the author of this comment
/// is `28`" is a wrong answer no distance threshold should be trusted alone to prevent — the
/// same argument this file already makes for the reply control.
///
/// A nickname that is only digits would be refused by this. That costs a refusal, where the
/// alternative costs a reply under a stranger's comment, and only one of those can be undone.
fn is_count_label(label: &str) -> bool {
    let trimmed = label.trim().trim_end_matches(['K', 'M', 'k', 'm']);
    !trimmed.is_empty()
        && trimmed
            .chars()
            .all(|c| c.is_ascii_digit() || c == '.' || c == ',')
}

fn is_reply_label(label: &str) -> bool {
    let label = label.trim();
    crate::tiktok_labels::TIKTOK_LABEL_SETS.iter().any(|set| {
        set.reply_label()
            .is_some_and(|reply| reply.eq_ignore_ascii_case(label))
    })
}

/// The comment row matching `identity`, or `None` to refuse.
///
/// `bodies` are the candidates for the comment text, `replies` every reply control
/// on screen, and `authors` every label that could name an author. All three come
/// from `UiSession::locate_all`, so they are in tree order — which is **not** screen
/// order, and that is the whole reason the rules below measure distance instead of
/// taking the first match.
pub fn locate_parent_in_elements(
    bodies: &[ElementBox],
    replies: &[ElementBox],
    authors: &[ElementBox],
    identity: &CommentLocatorIdentity,
) -> Option<ElementReplyTarget> {
    // The body must appear exactly once. Two rows reading the same thing — a
    // repeated campaign message, someone quoting it back — give no way to tell which
    // is ours, and picking either would anchor the reply to a guess. Ported from the
    // pixel path, where the same rule has its own regression test.
    let wanted = identity.text.trim();
    if wanted.is_empty() {
        return None;
    }
    let mut matching = bodies
        .iter()
        .filter(|body| body.description.as_deref().unwrap_or_default().trim() == wanted);
    let body = matching.next()?;
    if matching.next().is_some() {
        return None;
    }

    // The author is the nearest label *above* the body with a comparable left edge.
    // Both sit at the same x on the measured build; a label indented differently
    // belongs to something else.
    //
    // Reply controls are excluded by name, not left to the geometry. On this backend the
    // author candidates arrive as a `ClassName("android.widget.Button")` sweep, and the
    // measured reply control is *also* a Button carrying `Trả lời` in `text` — so the
    // previous row's reply button is in the list and sits above this body. Today
    // `AUTHOR_REACH` happens to exclude it (measured gap ~228 px against a 140 px reach),
    // but that is an accident of the row pitch, and "the author of this comment is
    // `Trả lời`" is a wrong answer no distance threshold should be trusted to prevent.
    // The pixel path excludes the reply label the same way.
    let author = authors
        .iter()
        .filter(|candidate| {
            let label = candidate.description.as_deref().unwrap_or_default().trim();
            !label.is_empty()
                && !is_reply_label(label)
                && !is_count_label(label)
                && bottom(candidate) <= body.y + ABOVE_SLACK
                && candidate.y >= body.y - AUTHOR_REACH
                && (candidate.x - body.x).abs() <= AUTHOR_LEFT_SLACK
        })
        .min_by(|a, b| {
            (body.y - bottom(a))
                .abs()
                .total_cmp(&(body.y - bottom(b)).abs())
        })?;

    // Every comment carries its own reply control, and the band below this body is
    // wide enough to reach the next row's. Take the **closest below**, not the first
    // the tree happened to list, and require it to the right of the body's left edge
    // — the measured reply button is indented (x=315 against a body at x=174).
    let body_bottom = bottom(body);
    let reply = replies
        .iter()
        .filter(|candidate| {
            candidate.y >= body.y
                && candidate.y <= body_bottom + REPLY_REACH
                && candidate.x >= body.x
        })
        .min_by(|a, b| {
            (a.y - body_bottom)
                .abs()
                .total_cmp(&(b.y - body_bottom).abs())
        })?;

    Some(ElementReplyTarget {
        identity: CommentLocatorIdentity {
            author_label: author
                .description
                .as_deref()
                .unwrap_or_default()
                .trim()
                .to_string(),
            text: body
                .description
                .as_deref()
                .unwrap_or_default()
                .trim()
                .to_string(),
            locator_version: identity.locator_version.clone(),
            frame_sha256: identity.frame_sha256.clone(),
        },
        reply: reply.clone(),
    })
}

/// Read back the identity of a comment this project just posted.
///
/// Same uniqueness rule as [`locate_parent_in_elements`]: if the text is on screen
/// twice, the identity is ambiguous and storing either would send the next reply in
/// the chain to the wrong row.
pub fn discover_identity_in_elements(
    bodies: &[ElementBox],
    authors: &[ElementBox],
    exact_text: &str,
    frame_sha256: &str,
    locator_version: &str,
) -> Option<CommentLocatorIdentity> {
    let probe = CommentLocatorIdentity {
        author_label: String::new(),
        text: exact_text.to_string(),
        locator_version: locator_version.to_string(),
        frame_sha256: frame_sha256.to_string(),
    };
    // Reuse the row-matching rules rather than re-deriving "which author goes with
    // which body"; only the reply control is irrelevant here, so it is not required.
    let wanted = exact_text.trim();
    if wanted.is_empty() {
        return None;
    }
    let mut matching = bodies
        .iter()
        .filter(|body| body.description.as_deref().unwrap_or_default().trim() == wanted);
    let body = matching.next()?;
    if matching.next().is_some() {
        return None;
    }
    let author = authors
        .iter()
        .filter(|candidate| {
            let label = candidate.description.as_deref().unwrap_or_default().trim();
            !label.is_empty()
                && !is_reply_label(label)
                && !is_count_label(label)
                && bottom(candidate) <= body.y + ABOVE_SLACK
                && candidate.y >= body.y - AUTHOR_REACH
                && (candidate.x - body.x).abs() <= AUTHOR_LEFT_SLACK
        })
        .min_by(|a, b| {
            (body.y - bottom(a))
                .abs()
                .total_cmp(&(body.y - bottom(b)).abs())
        })?;
    Some(CommentLocatorIdentity {
        author_label: author
            .description
            .as_deref()
            .unwrap_or_default()
            .trim()
            .to_string(),
        text: body
            .description
            .as_deref()
            .unwrap_or_default()
            .trim()
            .to_string(),
        ..probe
    })
}

/// How far the open could be proved, in the order the evidence is strong.
///
/// Mirrors the pixel path's `TargetProof` on purpose — the campaign records the same
/// two levels either way, so an operator comparing an iPhone's run to an Android's is
/// reading the same word for the same claim.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TargetArrival {
    /// A node on the post carried the author handle from the link. The only level that
    /// identifies *which* post is open.
    Identified { author_label: String },
    /// TikTok is foreground on the right package and a post page is up, but nothing on
    /// screen carried the handle. Same honesty as the pixel path's `Structural`.
    Structural,
}

/// Why an arrival could not be proved. Every variant means **nothing was typed**.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArrivalRefusal {
    /// The foreground app is not the resolved TikTok package. Measured as a real hazard:
    /// a bare `VIEW` intent for a tiktok.com URL resolves to the system app chooser
    /// because Chrome claims the domain too (see `UiSession::open_url_in_app`).
    WrongApp { found: String },
    /// TikTok is foreground but no action rail appeared within the window.
    NoPostPage,
    /// The rail is there, but the post on screen is **the same one that was there before
    /// the link was opened** — so nothing suggests the intent did anything.
    ///
    /// This is the measured failure mode of an unavailable post: TikTok accepts the
    /// intent, fails to resolve it server-side, and **silently leaves the feed where it
    /// was**. That screen has a comment control on it, so without this check the campaign
    /// would post to whatever video happened to be playing.
    ScreenNeverChanged { author_label: String },
    /// A label set with no `Comments` measured — the predicate this check rests on
    /// cannot be evaluated, so it refuses rather than assuming arrival.
    NoLabelForPostPage,
    /// The author label of whatever was on screen **before** the link was opened could
    /// not be read, so there is no baseline to compare against.
    ///
    /// This variant exists because its absence was a hole in the guard, not a
    /// hypothetical. `before` used to be `read_author_label(..).unwrap_or_default()`, and
    /// an empty `before` makes the arrival condition `author != before` true for *any*
    /// author — collapsing the check to "TikTok is foreground and a post page is up",
    /// which the feed itself satisfies. `ScreenNeverChanged` then became unreachable and
    /// the campaign would comment on whichever post happened to be on screen and record
    /// it as a success.
    ///
    /// And an empty `before` is not rare: [`read_author_label`] returns `None` both when
    /// the node is genuinely absent **and** when the query errors, so one agent hiccup
    /// disarmed the guard. Refused **before** the intent is dispatched, so a device that
    /// cannot be baselined costs no side effect at all — the same discipline as
    /// [`Self::NoLabelForPostPage`].
    NoBaseline,
    /// The campaign was cancelled while waiting.
    Cancelled,
}

impl ArrivalRefusal {
    /// A stable code for the assignment row, so a failure is greppable.
    pub fn code(&self) -> &'static str {
        match self {
            Self::WrongApp { .. } => "target_open_wrong_app",
            Self::NoPostPage => "target_open_no_post_page",
            Self::ScreenNeverChanged { .. } => "target_open_screen_unchanged",
            Self::NoLabelForPostPage => "target_open_no_measured_label",
            Self::NoBaseline => "target_open_no_baseline",
            Self::Cancelled => "target_open_cancelled",
        }
    }

    pub fn message(&self) -> String {
        match self {
            Self::WrongApp { found } => {
                format!("liên kết mở ra {found} chứ không phải TikTok; không gõ gì cả")
            }
            Self::NoPostPage => {
                "TikTok đã lên nhưng không thấy thanh hành động của bài nào".to_string()
            }
            Self::ScreenNeverChanged { author_label } => format!(
                "mở link xong màn hình vẫn là bài cũ ({author_label}) — thường là bài đã bị \
                 xoá/riêng tư/chặn vùng: TikTok nhận intent, resolve thất bại rồi im lặng \
                 để nguyên feed. Không gõ gì cả, vì gõ ở đây là bình luận vào video đang phát."
            ),
            Self::NoLabelForPostPage => {
                "build/ngôn ngữ này chưa đo nhãn nút bình luận, nên không có cách kiểm \
                 đã tới trang bài — từ chối thay vì đoán"
                    .to_string()
            }
            Self::NoBaseline => {
                "không đọc được nhãn tác giả của bài đang trên màn trước khi mở link, nên \
                 không có gì để so — từ chối thay vì đoán. Chưa gửi intent nào, máy không \
                 bị tác động gì."
                    .to_string()
            }
            Self::Cancelled => "campaign bị dừng trong lúc chờ mở bài".to_string(),
        }
    }
}

/// Whether an author label plausibly belongs to a handle.
///
/// Folded and stripped of punctuation on both sides, then either may contain the other.
/// **It works for some accounts and not others, and that is measured** — which is exactly
/// why a failure here downgrades the proof instead of failing the open:
///
/// | handle | nickname on screen | run tried | matches |
/// |---|---|---|---|
/// | `mongquynh.dalat` | `Mộng Quỳnh` | `mongquynh` ⊂ `mongquynhdalat` | **yes** |
/// | `huongthao.dalat` | `Hương Thảo` | `huongthao` ⊂ `huongthaodalat` | **yes** |
/// | `n.sp.i.hoang` | `Ăn Sập Đi Hoang` | no run inside `nspihoang` | no |
/// | `nguyenvantoan8584` | `Lúc này lúc kia` | unrelated | no |
///
/// The third row is the instructive one: the handle is a *consonant skeleton* of the
/// nickname (`Ăn Sập Đi` → `n.sp.i`), which no amount of normalising recovers. Two matches
/// in four — good enough to strengthen a proof when it hits, never enough to require.
pub fn author_matches_handle(author_label: &str, handle: &str) -> bool {
    let squash = |value: &str| -> String {
        value
            .chars()
            .filter(|character| character.is_alphanumeric())
            .collect()
    };
    let handle = squash(&crate::interaction::normalize_locator_text(
        handle.trim_start_matches('@'),
    ));
    if handle.len() < MIN_HANDLE_RUN {
        return false;
    }
    // Compared as **contiguous word runs**, not as one string, because a handle is usually
    // built from *part* of the nickname — it matches on a run, never on the whole. The label
    // reaching here is already the bare nickname (`bare_author_label` strips whichever
    // catalogued needle wrapped it), so this no longer has to see past a `Follow ` prefix, but
    // it still has to see past the words the handle left out.
    //
    // **It does not catch a handle that drops letters, and the example this comment used to
    // claim as a match is one of those.** Measured 24/08/2026: `Đà Lạt Gói Mang Về` squashes to
    // runs of `da lat goi mang ve` while `.lt.gi.mang.v` squashes to `ltgimangv` — the handle
    // abbreviates each word rather than taking a prefix of the whole, so no run of one contains
    // the other and this returns `false`. Arrival at that account's posts therefore succeeds as
    // `TargetArrival::Structural`, not `Identified`. That is a weaker guarantee and it is the
    // honest one; loosening the comparison to catch abbreviations would make it match handles
    // that belong to other people, which is the failure this whole check exists to prevent.
    let words: Vec<String> = crate::interaction::normalize_locator_text(author_label)
        .split_whitespace()
        .map(squash)
        .filter(|word| !word.is_empty())
        .collect();
    for start in 0..words.len() {
        let mut run = String::new();
        for word in &words[start..] {
            run.push_str(word);
            // A short run matches far too much — `an` is inside plenty of handles — so a
            // match has to be substantial enough to mean something.
            if run.len() >= MIN_HANDLE_RUN && (handle.contains(&run) || run.contains(&handle)) {
                return true;
            }
        }
    }
    false
}

/// Shortest run of author-name characters that may be called a handle match.
///
/// Six, because shorter runs match by accident: `Ăn Sập Đi Hoang` contributes `an`, `sap`,
/// `di`, and any handle with those letters in it would "match".
const MIN_HANDLE_RUN: usize = 6;

/// Longest the device gets to land on the target, and how often the tree is read.
///
/// Deliberately not copied from the pixel path's 14 s / 900 ms: that budget is paced by
/// an OCR pass per poll. A `locate` on this fleet measured 212–410 ms on the feed, so
/// polling can be tighter — but the *total* stays generous, because what is being waited
/// for is a network fetch and a page transition, not a local computation.
const ARRIVAL_WINDOW: Duration = Duration::from_secs(14);
const ARRIVAL_POLL: Duration = Duration::from_millis(600);
/// How long to keep looking for the handle after the post page is up.
///
/// The post is already open by then; this is only the identity check catching up, so
/// running out of it downgrades to [`TargetArrival::Structural`] rather than failing.
const HANDLE_GRACE: Duration = Duration::from_secs(3);

/// Open a target link and report how far the arrival could be proved — by hierarchy.
///
/// **Measured on a Redmi Note 12, 11/08/2026, and the result changed this function.** A
/// deep-linked post is not a separate page: TikTok renders it as the *current card of the
/// For-You pager*, with the top tab row (`Đề xuất` still selected) and the bottom tab bar
/// both still on screen. So there is **no structural difference** between "the target
/// post" and "whatever video was playing". An earlier version of this function required
/// the feed tab to be *absent*; that would have refused every real arrival.
///
/// What is left, in the order it is weighed:
///
/// 1. **`active_app_bundle() == target_package`.** Catches a link resolving into a
///    browser, which is a live hazard — see [`UiSession::open_url_in_app`].
/// 2. **An action rail is on screen** (`Comments`), so *some* post is up.
/// 3. **The post is not the one that was already there.** Read the author label *before*
///    opening the link and require it to change. This is the Android equivalent of the
///    pixel path's frame-SHA comparison, read from the tree instead of from pixels, and it
///    is the check that catches the measured failure of an unavailable post: TikTok
///    accepts the intent, fails to resolve it server-side, and **leaves the feed exactly
///    where it was**. That screen has a comment control on it, so without this the
///    campaign would post to a stranger's video.
/// 4. **The author handle**, when the nickname happens to reveal it — see
///    [`author_matches_handle`]. One account in three, so it upgrades the proof and never
///    gates it.
///
/// [`TargetArrival::Structural`] therefore means: TikTok is foreground, a post is up, and
/// it is **not** the post that was there before — but nothing read off the screen names
/// it. That is the same level of proof the pixel path settles for on Windows, and it is
/// stated plainly rather than dressed up.
///
/// **It taps nothing.** An arrival check that taps could dismiss a sheet, open a profile
/// or like a post, and none of that is recoverable from a log line.
pub async fn open_target_by_hierarchy(
    session: &dyn UiSession,
    labels: TikTokControls,
    target_package: &str,
    url: &str,
    author_handle: &str,
    stop: &AtomicBool,
) -> Result<TargetArrival, ArrivalRefusal> {
    // Without a measured `Comments` label there is no post-page predicate at all.
    // Checked before the open, so a build nobody has measured costs no side effect.
    let Some(comments) = labels.label(TikTokControl::Comments) else {
        return Err(ArrivalRefusal::NoLabelForPostPage);
    };
    // **The post on screen before the link is opened, and a deliberate move away from it.**
    //
    // Arrival is proved by the author label *changing*, which is the only thing separating a
    // link that resolved from an intent TikTok swallowed. Two ways that reading goes wrong,
    // both measured on 19/08/2026 and both fixed by the same tap:
    //
    // * The phone is **already on the target post** — nothing changes, and the open is
    //   refused as `target_open_screen_unchanged`, the message for a deleted or blocked post,
    //   about a post that is on screen and perfectly fine (ce051715ac247a3f01, same link run
    //   twice). Retrying an assignment does exactly this.
    // * The phone is **not on a post at all** — no author label anywhere, so there is no
    //   baseline and the open is refused as `target_open_no_baseline` before it is even tried
    //   (ce0417145199e0490c, left on a search results page by an earlier run).
    //
    // So: read what is there, tap Home, and wait for an author label that is *different* from
    // whatever was read first. Home is the same gesture `await_feed` uses to get back to the
    // feed, and it neither posts nor follows anything. A phone that was lost lands on a post
    // and gets a baseline; a phone already on the target leaves it and gets a different one.
    //
    // An unreadable baseline is still a **refusal** rather than an empty string — see
    // `ArrivalRefusal::NoBaseline` for what `unwrap_or_default()` cost: the comparison went
    // vacuous and the guard stopped guarding.
    let initial = read_author_label(session, labels).await;
    // The retry the tests pin, and it comes first: `read_author_label` folds a query
    // error into the same `None` as an absent node, so one agent hiccup is otherwise
    // indistinguishable from an empty screen — and Home cannot help with a transient.
    let initial = match initial {
        Some(label) => Some(label),
        None => {
            tokio::time::sleep(ARRIVAL_POLL).await;
            if stop.load(Ordering::Relaxed) {
                return Err(ArrivalRefusal::Cancelled);
            }
            read_author_label(session, labels).await
        }
    };
    let mut settled = initial.clone();
    if let Some(home) = labels.label(TikTokControl::HomeTab) {
        if let Ok(Some(element)) = session.locate(home.to_query()).await {
            let _ = session.tap(element.centre()).await;
            let deadline = Instant::now() + BASELINE_SETTLE;
            loop {
                tokio::time::sleep(ARRIVAL_POLL).await;
                if stop.load(Ordering::Relaxed) {
                    return Err(ArrivalRefusal::Cancelled);
                }
                match read_author_label(session, labels).await {
                    Some(fresh) if Some(&fresh) != initial.as_ref() => {
                        settled = Some(fresh);
                        break;
                    }
                    fresh => {
                        if Instant::now() >= deadline {
                            // Nothing better arrived. Whatever was readable stands, and if
                            // nothing ever was, the refusal below is the honest answer.
                            settled = settled.or(fresh);
                            break;
                        }
                    }
                }
            }
        }
    }
    // **Home lands on whichever feed tab was last used, and one of them has no author at all.**
    //
    // Measured 25/08/2026, screenshots of the phones that failed: four of twenty were parked
    // on the **Friends** tab, whose cards carry a story rail and no author row — so there was
    // nothing to read, the Home tap kept them there, and every run refused those same four
    // with `no_baseline`. `FeedTab` is For You, which always names an author.
    //
    // Only when there is still nothing: a phone that already gave a baseline is left where it
    // is, because moving it would be a navigation nobody asked for.
    if settled.is_none() {
        if let Some(feed) = labels.label(TikTokControl::FeedTab) {
            if let Ok(Some(element)) = session.locate(feed.to_query()).await {
                let _ = session.tap(element.centre()).await;
                let deadline = Instant::now() + BASELINE_SETTLE;
                loop {
                    tokio::time::sleep(ARRIVAL_POLL).await;
                    if stop.load(Ordering::Relaxed) {
                        return Err(ArrivalRefusal::Cancelled);
                    }
                    if let Some(fresh) = read_author_label(session, labels).await {
                        settled = Some(fresh);
                        break;
                    }
                    if Instant::now() >= deadline {
                        break;
                    }
                }
            }
        }
    }
    let before = settled.ok_or(ArrivalRefusal::NoBaseline)?;

    // Pinned to the target app, not a bare `VIEW` intent: measured, a bare one resolves
    // to the system app chooser because TikTok and Chrome both claim `www.tiktok.com`, so
    // the link reaches a dialog instead of the post (`UiSession::open_url_in_app`).
    //
    // Returning Ok means the request was accepted, not that the post is up. A failure to
    // even dispatch is reported as "no post page" rather than swallowed, because the two
    // are indistinguishable from here and both mean: do not type.
    if session.open_url_in_app(url, target_package).await.is_err() {
        return Err(ArrivalRefusal::NoPostPage);
    }

    let deadline = Instant::now() + ARRIVAL_WINDOW;
    let mut post_page_since: Option<Instant> = None;
    let mut last_foreground = String::new();
    let mut last_author = String::new();
    loop {
        if stop.load(Ordering::Relaxed) {
            return Err(ArrivalRefusal::Cancelled);
        }

        if let Ok(foreground) = session.active_app_bundle().await {
            last_foreground = foreground;
        }
        // Only judged once something is actually foreground: mid-transition the query
        // can answer with the launcher, and that is not evidence of a wrong app.
        if !last_foreground.is_empty() && last_foreground == target_package {
            let on_post = session
                .locate(comments.to_query())
                .await
                .ok()
                .flatten()
                .is_some();
            let author = read_author_label(session, labels).await.unwrap_or_default();
            // A rail plus a *different* post. Both halves are necessary: the rail alone is
            // satisfied by the feed, and an unchanged author is what an unavailable post
            // looks like.
            if on_post && !author.is_empty() && author != before {
                let since = *post_page_since.get_or_insert_with(Instant::now);
                if author_matches_handle(&author, author_handle) {
                    return Ok(TargetArrival::Identified {
                        author_label: author,
                    });
                }
                // The nickname does not reveal the handle, which is the common case. Give
                // it a moment in case the label was still rendering, then settle for the
                // structural proof rather than failing an open that plainly worked.
                if since.elapsed() >= HANDLE_GRACE {
                    return Ok(TargetArrival::Structural);
                }
            } else {
                // **The arm is cleared, not latched forever.** `post_page_since` used to be
                // set once and never reset, so a single transient read — an adjacent feed
                // card's author caught mid-transition, a rail that flickered on — armed the
                // timeout's `Structural` fall-through below permanently, even if every later
                // poll lost the rail or read the baseline author again. That is the measured
                // signature of a *dead* link (the old post never left), and it must not pass
                // as an arrival. So a poll that no longer sees a different post on a rail
                // disarms it; `Structural` then requires the differing author to be the last
                // thing actually seen, not the first thing glimpsed.
                post_page_since = None;
            }
            last_author = author;
        }

        if Instant::now() >= deadline {
            // A wrong app is a more useful answer than "no post page", and it is only
            // claimed when something other than TikTok was actually seen foreground.
            if !last_foreground.is_empty() && last_foreground != target_package {
                return Err(ArrivalRefusal::WrongApp {
                    found: last_foreground,
                });
            }
            if post_page_since.is_some() {
                // A post that changed but was never identified — still an arrival.
                return Ok(TargetArrival::Structural);
            }
            // Distinguish "no rail at all" from "the same post as before": the second is
            // the measured signature of a link that could not be resolved, and saying so
            // sends the operator to check the link rather than the phone.
            return Err(if last_author.is_empty() || last_author != before {
                ArrivalRefusal::NoPostPage
            } else {
                ArrivalRefusal::ScreenNeverChanged {
                    author_label: last_author,
                }
            });
        }
        tokio::time::sleep(ARRIVAL_POLL).await;
    }
}

/// The class of node that carries a comment author's name.
///
/// Measured on `com.ss.android.ugc.trill` (AGENTS.md §9.5): the author is an
/// `android.widget.Button` carrying the nickname in `text`, with an **empty**
/// `content-desc`. The body is an `android.widget.TextView`, but it is never queried by
/// class — a body is always looked up by the exact string this code typed, which is both
/// cheaper and unambiguous.
const COMMENT_AUTHOR_CLASS: &str = "android.widget.Button";

/// What one hierarchy send achieved, and what it observed while doing it.
#[derive(Debug, Clone, PartialEq)]
pub struct HierarchySendOutcome {
    pub verdict: crate::tiktok_drawer::CommentVerdict,
    /// Frame at the moment Send was armed, and after it disarmed. Same two fields the
    /// pixel path fills, from the same source — so a stored evidence row means the same
    /// thing whichever backend wrote it.
    pub armed_frame_sha256: String,
    pub cleared_frame_sha256: String,
    /// The posted comment, read back out of the still-open list. `None` when it could
    /// not be found unambiguously, which breaks the chain rather than guessing.
    pub identity: Option<CommentLocatorIdentity>,
    /// What happened to the `@` tags, when any were asked for.
    ///
    /// A tag that could not be picked out of TikTok's list is still *typed*, so the comment
    /// goes out either way — but "tagged" and "wrote the characters of a tag" are different
    /// outcomes and the operator has to be able to tell them apart. See
    /// [`append_mentions_by_picker`].
    pub mention_note: Option<String>,
    /// The parent was only reachable after expanding `View folded comments`.
    ///
    /// A reply under a folded comment is posted, confirmed, and **seen by nobody** —
    /// TikTok folded the parent away from everyone but the account that wrote it. The send
    /// succeeded and the thread is invisible, and those two facts have to travel together
    /// or the operator reads a success and believes something untrue.
    pub parent_was_folded: bool,
}

impl HierarchySendOutcome {
    /// The outcome for a transport failure that happened **before the Send tap** — opening
    /// the drawer, scrolling to the parent, tapping Reply, typing, or waiting for the arm.
    ///
    /// A verdict rather than a propagated error so the known, observed interruption travels
    /// through the same finish path as the other pre-Send outcomes. The outer error type is
    /// still phase-aware for unexpected transport failures. Nothing was posted this early,
    /// so either route stays retryable; the tap itself and anything after it travel as
    /// `AfterEffect`.
    fn interrupted_before_send() -> Self {
        Self {
            verdict: crate::tiktok_drawer::CommentVerdict::SendFlowInterrupted,
            armed_frame_sha256: String::new(),
            cleared_frame_sha256: String::new(),
            identity: None,
            mention_note: None,
            parent_was_folded: false,
        }
    }
}

/// A hierarchy send transport failure, with the Send tap as an explicit effect boundary.
///
/// There is deliberately no blanket `From<anyhow::Error>` implementation: every propagated
/// transport error must name which side of the tap it came from. That makes a future pre-Send
/// `?` fail to compile instead of silently becoming an unretryable after-effect failure at the
/// campaign call site.
#[derive(Debug, thiserror::Error)]
pub enum HierarchySendFailure {
    /// Nothing was sent, so the assignment remains retryable.
    #[error("hierarchy send stopped before the Send tap: {0}")]
    BeforeEffect(anyhow::Error),
    /// The Send tap was attempted, so retrying could duplicate a public comment.
    #[error("hierarchy send failed at or after the Send tap: {0}")]
    AfterEffect(anyhow::Error),
    #[error("hierarchy send lost assignment ownership before the Send tap: {0}")]
    OwnershipLost(anyhow::Error),
}

impl HierarchySendFailure {
    pub fn before(error: impl Into<anyhow::Error>) -> Self {
        Self::BeforeEffect(error.into())
    }

    pub fn after(error: impl Into<anyhow::Error>) -> Self {
        Self::AfterEffect(error.into())
    }

    pub fn ownership_lost(&self) -> bool {
        matches!(self, Self::OwnershipLost(_))
    }

    fn from_gate(failure: crate::interaction_target::SendFailure) -> Self {
        match failure {
            crate::interaction_target::SendFailure::OwnershipLost(error) => {
                Self::OwnershipLost(error)
            }
            crate::interaction_target::SendFailure::BeforeEffect(error) => {
                Self::BeforeEffect(error)
            }
            crate::interaction_target::SendFailure::AfterEffect(error) => Self::AfterEffect(error),
        }
    }
}

/// The `locator_version` stamped on identities this module produces.
///
/// Distinct from the OCR path's on purpose: a stored identity has to say which reader
/// created it, because the two read the author label from different places and only one
/// of them can find it again.
pub const HIERARCHY_LOCATOR_VERSION: &str = "android-hierarchy-v1";

/// Clear an armed composer left by a process that died before crossing its effect gate.
///
/// The caller must reopen and re-prove the target after this returns `Ok`: cleanup ends on
/// the feed, so every pre-cleanup field, row and Reply rectangle is invalid by construction.
pub(crate) async fn clear_stale_hierarchy_comment_ui(
    session: &dyn UiSession,
    labels: TikTokControls,
    stop: &AtomicBool,
) -> Result<(), HierarchySendFailure> {
    use crate::tiktok_drawer::CommentDrawer;

    let drawer = CommentDrawer::new(session, labels, |element: &ElementBox| element.centre());
    let Some(send_query) = drawer.send_query() else {
        return Ok(());
    };
    let stale = session
        .locate(send_query)
        .await
        .map_err(HierarchySendFailure::before)?
        .is_some_and(|send| send.enabled);
    if !stale {
        return Ok(());
    }

    let feed_verified = drawer.leave(stop).await;
    let drawer_closed = session
        .locate(send_query)
        .await
        .map(|send| send.is_none())
        .unwrap_or(false);
    if feed_verified && drawer_closed {
        Ok(())
    } else {
        Err(HierarchySendFailure::after(anyhow::anyhow!(
            "crash-stale hierarchy comment composer cleanup was not verified"
        )))
    }
}

/// Post a root comment by hierarchy and read it back, leaving the drawer open.
///
/// **The drawer is deliberately left open.** The Interaction path depends on that: the
/// evidence frame it publishes has to show the comment *in the list*, and the identity
/// read-back below needs the list on screen. `crate::tiktok_drawer::post_comment` closes
/// it, which is right for nurture and wrong here — that difference is the whole reason
/// the drawer exposes its steps separately.
///
/// `frame_sha` is called at two moments rather than being passed a frame source, so this
/// module stays free of stream plumbing and is testable with a counter.
pub async fn send_root_by_hierarchy<F>(
    session: &dyn UiSession,
    labels: TikTokControls,
    screen: (f64, f64),
    text: &str,
    mentions: &[String],
    stop: &AtomicBool,
    frame_sha: F,
) -> Result<HierarchySendOutcome, HierarchySendFailure>
where
    F: FnMut() -> String,
{
    let mut effect_gate = crate::interaction_target::EffectGate::allow();
    send_root_by_hierarchy_with_gate(
        session,
        labels,
        screen,
        text,
        mentions,
        stop,
        frame_sha,
        &mut effect_gate,
    )
    .await
}

// This effect-boundary entry point intentionally mirrors the public sender's independent
// inputs and adds only the one-shot gate; grouping them would hide which values are re-proved
// immediately before the public tap.
#[allow(
    clippy::too_many_arguments,
    reason = "effect boundary keeps independently proved send inputs explicit"
)]
pub(crate) async fn send_root_by_hierarchy_with_gate<F>(
    session: &dyn UiSession,
    labels: TikTokControls,
    screen: (f64, f64),
    text: &str,
    mentions: &[String],
    stop: &AtomicBool,
    mut frame_sha: F,
    effect_gate: &mut crate::interaction_target::EffectGate<'_>,
) -> Result<HierarchySendOutcome, HierarchySendFailure>
where
    F: FnMut() -> String,
{
    use crate::tiktok_drawer::{CommentDrawer, CommentVerdict};

    let outcome = |verdict, armed: String, cleared: String, identity| HierarchySendOutcome {
        verdict,
        mention_note: None,
        parent_was_folded: false,
        armed_frame_sha256: armed,
        cleared_frame_sha256: cleared,
        identity,
    };

    // The same planner nurture uses, so taps on this device keep one jitter history
    // rather than two overlapping ones.
    let mut planner = crate::nurture::touch::TouchPointPlanner::new(screen);
    let plan = move |element: &ElementBox| planner.next(element.centre(), element.jitter_radius());
    let mut drawer = CommentDrawer::new(session, labels, plan);

    if drawer.send_query().is_none() {
        // Refused before the drawer is even opened: there is nothing to aim at, and
        // opening it would leave the phone inside a drawer with a draft it cannot send.
        return Ok(outcome(
            CommentVerdict::SendUnmeasured,
            String::new(),
            String::new(),
            None,
        ));
    }
    // A transport error here is BEFORE the Send tap — the drawer is only opening — so it is
    // a retryable verdict, not the `after`-tap error the campaign would settle `Uncertain`.
    let field = match drawer.open(stop).await {
        Ok(Some(field)) => field,
        Ok(None) => {
            return Ok(outcome(
                CommentVerdict::NoDrawer,
                String::new(),
                String::new(),
                None,
            ))
        }
        Err(_) => {
            return Ok(outcome(
                CommentVerdict::SendFlowInterrupted,
                String::new(),
                String::new(),
                None,
            ))
        }
    };
    // **A refusal here is settled as a verdict, never as a transport error.** Both answers
    // mean nothing was typed and nothing was tapped; routing either through `?` would send
    // it down the interaction path's after-effect channel, which retires the assignment
    // `Uncertain` — unretryable — for a message that never reached the field. A transport
    // error is the same story: still before the Send tap, still nothing posted.
    let typed = match drawer.focus_and_type(&field, text, stop).await {
        Ok(typed) => typed,
        Err(_) => {
            drawer.leave(stop).await;
            return Ok(outcome(
                CommentVerdict::SendFlowInterrupted,
                String::new(),
                String::new(),
                None,
            ));
        }
    };
    if typed != crate::tiktok_drawer::TypedInto::Typed {
        // Nothing to read back on a refusal, and a drawer left open with a lit draft makes
        // the next assignment on this phone refuse for the same reason. Interaction does
        // not close the drawer on its success path (it reads the posted comment out of the
        // still-open list); a refusal is the opposite case.
        drawer.leave(stop).await;
        return Ok(outcome(
            match typed {
                crate::tiktok_drawer::TypedInto::SendPreArmed => CommentVerdict::SendPreArmed,
                _ => CommentVerdict::NoSendControl,
            },
            String::new(),
            String::new(),
            None,
        ));
    }
    // After the body and before the send: the body is written with `set_text`, which replaces
    // the whole field, so a token added first would not survive it. See
    // `append_mentions_by_picker` for why the tags land at the end rather than the front.
    let mut posted: Option<String> = None;
    let mention_note = if mentions.is_empty() {
        None
    } else {
        let mention_outcome = match append_mentions_by_picker(session, screen, mentions, stop).await
        {
            Ok(outcome) => outcome,
            Err(_) => {
                drawer.leave(stop).await;
                return Ok(HierarchySendOutcome::interrupted_before_send());
            }
        };
        // Whatever is in the box now is what Send will publish, tags and spacing included.
        // Read rather than reconstructed: the token's exact spelling and trailing space are
        // TikTok's to decide, and guessing them is how the read-back missed the first time.
        posted = match composer_text(session).await {
            Ok(value) => value.filter(|value| value.contains(text)),
            Err(_) => {
                drawer.leave(stop).await;
                return Ok(HierarchySendOutcome::interrupted_before_send());
            }
        };
        mention_outcome.note()
    };
    // Still before the Send tap: a transport error waiting for the arm posted nothing.
    let send = match drawer.await_armed(stop).await {
        Ok(Some(send)) => send,
        Ok(None) => {
            return Ok(outcome(
                CommentVerdict::NotArmed,
                String::new(),
                String::new(),
                None,
            ))
        }
        Err(_) => {
            drawer.leave(stop).await;
            return Ok(outcome(
                CommentVerdict::SendFlowInterrupted,
                String::new(),
                String::new(),
                None,
            ));
        }
    };
    let armed = frame_sha();
    // Keep the successful gate-to-Send path adjacent: no await or other device operation may
    // enter between the durable CAS and the public tap.
    let confirmed = match effect_gate.cross() {
        Ok(()) => drawer
            .tap_send_and_confirm_disarm(&send, stop)
            .await
            .map_err(HierarchySendFailure::after)?,
        Err(failure) => {
            let cleaned = drawer.leave(stop).await;
            return Err(if cleaned || failure.ownership_lost() {
                HierarchySendFailure::from_gate(failure)
            } else {
                HierarchySendFailure::after(anyhow::anyhow!(
                    "effect gate failed after typing and comment UI cleanup was not verified: {}",
                    failure.into_error()
                ))
            });
        }
    };
    let cleared = frame_sha();
    if !confirmed {
        // `NotConfirmed` and never retried: the tap went out, so a retry is how a post
        // ends up with two identical comments on it. The frames are still returned —
        // they are the only way a person can settle what happened. The tag note rides along
        // for the same reason: whatever went out, it went out with these tags on it.
        let mut refused = outcome(CommentVerdict::NotConfirmed, armed, cleared, None);
        refused.mention_note = mention_note;
        return Ok(refused);
    }

    // Read the comment back out of the list that is still on screen. A failure here
    // does not un-send anything, so it downgrades the identity rather than the verdict.
    //
    // **Read back what was actually posted, which is not always `text`.** Appending tags puts
    // `@handle` after the body, so a comment sent with a tag is longer than the string this
    // function was given — and the read-back matches exactly, by design. Measured 24/08/2026
    // on the first real tagged send: the comment went out fine and came back unfindable,
    // which silently costs the identity every reply in the thread needs. `posted` is the
    // composer's own contents, captured before Send.
    let identity = read_back_identity(session, posted.as_deref().unwrap_or(text), &cleared).await;
    let mut sent = outcome(CommentVerdict::Sent, armed, cleared, identity);
    sent.mention_note = mention_note;
    Ok(sent)
}

/// How long to wait for TikTok's mention list to arrive after the handle is typed.
///
/// It is a network fetch, and a list that has not come back yet looks exactly like one that
/// has nothing in it — which is the wrong conclusion, because it would silently downgrade a
/// mention to plain text.
const MENTION_PICKER_WAIT: Duration = Duration::from_millis(4_000);
const MENTION_PICKER_POLL: Duration = Duration::from_millis(400);

/// What became of each handle the operator asked to tag.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MentionOutcome {
    /// Picked out of TikTok's own list **and confirmed against the composer afterwards**,
    /// so the comment carries a real mention.
    pub linked: Vec<String>,
    /// Typed into the comment, but no suggestion row ever arrived — so it posts as grey text
    /// and notifies nobody. The characters *are* in the comment.
    pub literal: Vec<String>,
    /// Never reached the field at all because the handle was refused before the keystrokes.
    /// **Not in the comment in any form** — which is why it cannot share a bucket with
    /// `literal`, whose whole meaning is "it is in there, just as plain text". A transport
    /// failure is not a handle outcome and aborts the send instead.
    pub untyped: Vec<String>,
    /// A row was tapped and the composer could not be read back afterwards, so whether the
    /// token landed is unknown — and the tap may have left the drawer entirely.
    pub unverified: Vec<String>,
}

impl MentionOutcome {
    /// One line for the operator, or `None` when nothing was asked for.
    pub fn note(&self) -> Option<String> {
        let mut parts = Vec::new();
        if !self.linked.is_empty() {
            parts.push(format!("tag thật: @{}", self.linked.join(" @")));
        }
        if !self.literal.is_empty() {
            parts.push(format!(
                "chỉ là chữ (TikTok không gợi ý ra): @{}",
                self.literal.join(" @")
            ));
        }
        if !self.untyped.is_empty() {
            parts.push(format!(
                "không gõ được nên không có trong bình luận: @{}",
                self.untyped.join(" @")
            ));
        }
        if !self.unverified.is_empty() {
            parts.push(format!(
                "đã bấm nhưng không đọc lại được ô soạn: @{}",
                self.unverified.join(" @")
            ));
        }
        (!parts.is_empty()).then(|| parts.join(" · "))
    }
}

/// Append real `@mentions` to whatever is already in the composer.
///
/// **Why this is not just more text.** Typing an `@handle` into a comment produces literal
/// characters: TikTok renders them grey, links nothing, and notifies nobody. A real mention
/// only exists if it is *chosen from the app's own suggestion list*, which inserts a token.
/// Measured 24/08/2026 on `ce051715ac247a3f01` against
/// `.../@.lt.gi.mang.v/photo/7668947001618320660`:
///
/// * writing `@lt.gi` with `set_text` put the characters in the field and opened nothing —
///   accessibility text does not reach the app's input watchers;
/// * injecting the same characters as **key events** opened the list and filtered it to four
///   real accounts (`lt.gi`, `.lt.gi.mang.v`, `lt.g94`, `lt.gr37`);
/// * tapping the matching row turned the field into `…@lt.gi `, a token.
///
/// **The body has to be in the field first, and that ordering is forced.** `set_text` is the
/// only path that carries Vietnamese (`input text` is killed by diacritics), and it replaces
/// the whole editable — so anything it writes after a token would take the token with it. The
/// mentions therefore land at the end of the comment rather than the front. A mention notifies
/// the account wherever it sits.
///
/// **It never guesses.** A row is tapped only when its text equals the wanted handle exactly;
/// the measured list is full of near-misses (`lt.g94` for `lt.gi`) and tapping one would tag a
/// stranger from the operator's account. A handle with no exact row is left as the literal
/// text already typed — today's behaviour — and reported as such.
pub async fn append_mentions_by_picker(
    session: &dyn UiSession,
    screen: (f64, f64),
    handles: &[String],
    stop: &AtomicBool,
) -> anyhow::Result<MentionOutcome> {
    let mut outcome = MentionOutcome::default();
    let mut planner = crate::nurture::touch::TouchPointPlanner::new(screen);
    for handle in handles {
        let handle = handle.trim().trim_start_matches('@');
        if handle.is_empty() {
            continue;
        }
        if stop.load(Ordering::Relaxed) {
            // Nothing was typed, so the comment does not carry these characters in any form.
            // Reported as `untyped` and not `literal`, and the old code got this wrong twice
            // over: it claimed the handle was in the comment as text, and it pushed the
            // untrimmed original, so a leading `@` came back out as `@@name`.
            outcome.untyped.push(handle.to_string());
            continue;
        }
        // Checked **here**, not only in the backend. `type_keys` reaches a real device shell,
        // and the rule about what may go into it belongs to whoever composes the string —
        // leaving it to the Android session alone would mean every other backend, and every
        // test double, silently gets a laxer one. The session keeps its own check as well.
        if !is_typeable_handle(handle) {
            outcome.untyped.push(handle.to_string());
            continue;
        }
        // **Everything that already claims this handle, before a key is pressed.** The comment
        // list is open behind the composer and its rows are `TextView`s carrying author
        // handles, so tagging somebody who has already commented under the post — its own
        // author, most often — puts an exact match on screen that has nothing to do with the
        // picker. The picker's container has never been measured, so the discrimination cannot
        // come from *where* a row sits; it comes from the fact that a suggestion row was not
        // there before typing.
        let before = mention_rows(session, handle).await?;
        // A leading space, or the tag runs into the last word of the comment — measured
        // 24/08/2026: the first real run posted `…đi được ngay@ghin.lt.sng.sng`. TikTok adds
        // its own trailing space when it inserts the token, so consecutive tags separate
        // themselves and only the first needs this.
        session.type_keys(&format!(" @{handle}")).await?;
        match await_mention_row(session, handle, &before, stop).await? {
            Some(row) => {
                let point = planner.next(row.centre(), row.jitter_radius());
                session.tap(point).await?;
                tokio::time::sleep(MENTION_PICKER_POLL).await;
                // **Ask the field, not the list.** The old check read "the row is gone" as
                // proof the pick landed — but a tap that misses the picker and opens somebody's
                // profile also makes the row go away, and takes the drawer and the unsent draft
                // with it. Those two outcomes were indistinguishable, and the wrong one was
                // recorded as a real mention. The composer can only answer while the drawer is
                // still on screen, so it is the one witness that separates them.
                match composer_text(session).await? {
                    None => {
                        // The field is gone, so that tap did not land in a suggestion list —
                        // and nothing after this can be typed into a drawer that is not there.
                        outcome.unverified.push(handle.to_string());
                        return Ok(outcome);
                    }
                    Some(field) if !field.to_lowercase().contains(&handle.to_lowercase()) => {
                        // Still a drawer, but the handle is no longer in it: the tap changed
                        // the field into something this function did not ask for.
                        //
                        // Compared case-insensitively, the same way the row match is. TikTok's
                        // token carries the account's own spelling, so an operator who typed
                        // `LT.GI` for `lt.gi` would otherwise have a real mention reported as
                        // unverified — and the pass abandoned with it.
                        outcome.unverified.push(handle.to_string());
                        return Ok(outcome);
                    }
                    Some(_) => {
                        // Drawer alive, handle still in the field. A fresh row that still
                        // offers the handle means the tap did nothing at all.
                        let offered =
                            new_mention_row(mention_rows(session, handle).await?, &before);
                        if offered.is_some() {
                            outcome.literal.push(handle.to_string());
                        } else {
                            outcome.linked.push(handle.to_string());
                        }
                    }
                }
            }
            None => outcome.literal.push(handle.to_string()),
        }
    }
    Ok(outcome)
}

/// What a post says about itself, read off the action rail.
///
/// The numbers a threshold can be measured against — and the set is short on purpose,
/// because it is exactly what the post page states. **Views are not here.** Measured
/// 24/08/2026 on `.../@.lt.gi.mang.v/photo/7668947001618320660`: the rail carries
/// `Like video. 22 likes`, `Read or add comments. 21 comments` and `Share video. 8 shares`,
/// and nothing anywhere on the page states a play count. The only place TikTok shows one is
/// the author's profile grid, under each thumbnail, where nothing identifies *which* post a
/// number belongs to — so a view target could be worked towards but never checked, and a
/// threshold nobody can verify is a promise, not a feature.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PostCounters {
    /// `None` when this build has no measured counted-like control, or the rail was
    /// unreadable — never `0`, which would read as "nobody liked it".
    pub likes: Option<u32>,
    pub comments: Option<u32>,
    /// Whether either number came from an abbreviated string like `1.2K`.
    ///
    /// TikTok stops showing exact totals in the thousands, so above that a threshold can only
    /// be met to the nearest hundred. Saying so beats reporting `1200` as if it were counted.
    pub approximate: bool,
}

/// Read the post's own totals. Taps nothing.
pub async fn read_post_counters(session: &dyn UiSession, labels: TikTokControls) -> PostCounters {
    let mut counters = PostCounters::default();
    for (control, slot) in [
        (TikTokControl::LikeCount, 0usize),
        (TikTokControl::Comments, 1usize),
    ] {
        let Some(label) = labels.label(control) else {
            continue;
        };
        let Some(found) = session.locate(label.to_query()).await.ok().flatten() else {
            continue;
        };
        let Some(text) = found.description.as_deref() else {
            continue;
        };
        if let Some((value, approximate)) = parse_count(text) {
            if slot == 0 {
                counters.likes = Some(value);
            } else {
                counters.comments = Some(value);
            }
            counters.approximate |= approximate;
        }
    }
    counters
}

/// The first number in a rail label, and whether it was abbreviated.
///
/// The labels read `Like video. 22 likes` and `Read or add comments. 21 comments`, so the
/// count is the first run of digits. Two shapes have to survive: thousands separators
/// (`1,160`) and TikTok's own abbreviation (`1.2K`, `3M`), which is *rounded* — the caller is
/// told so rather than being handed a number that looks exact.
fn parse_count(text: &str) -> Option<(u32, bool)> {
    let bytes: Vec<char> = text.chars().collect();
    let start = bytes.iter().position(|c| c.is_ascii_digit())?;
    let mut end = start;
    while end < bytes.len()
        && (bytes[end].is_ascii_digit() || bytes[end] == ',' || bytes[end] == '.')
    {
        end += 1;
    }
    // A trailing separator belongs to the sentence, not to the number: `22 likes.`
    while end > start && !bytes[end - 1].is_ascii_digit() {
        end -= 1;
    }
    let raw: String = bytes[start..end].iter().collect();
    let suffix = bytes.get(end).copied().unwrap_or(' ');
    let multiplier = match suffix.to_ascii_uppercase() {
        'K' => 1_000f64,
        'M' => 1_000_000f64,
        'B' => 1_000_000_000f64,
        _ => 1f64,
    };
    let abbreviated = multiplier > 1f64;
    // With a multiplier the dot is a decimal point; without one it is a thousands separator,
    // which is how `1.2K` and `1,160` can both be right.
    let cleaned = if abbreviated {
        raw.replace(',', "")
    } else {
        raw.replace([',', '.'], "")
    };
    let value: f64 = cleaned.parse().ok()?;
    // **Refused rather than saturated.** `value * multiplier` on a long digit run is `inf`,
    // and `inf as u32` has saturated to `u32::MAX` since Rust 1.45 — no panic, no overflow,
    // just a number. A threshold measured against `u32::MAX` reads as already satisfied, so
    // the quiet answer is the dangerous one: the farm stops working towards a target it never
    // reached. Anything that does not fit a play count is not a play count.
    let scaled = (value * multiplier).round();
    if !scaled.is_finite() || scaled < 0.0 || scaled > f64::from(u32::MAX) {
        return None;
    }
    Some((scaled as u32, abbreviated))
}

/// Everything a threshold can be measured against, read from one phone already on the post.
///
/// Arrival is the caller's job — this only reads. The order is not arbitrary: the rail is on the
/// post page and costs nothing, while the view count is a **navigation** (post → profile → tile
/// → post, see [`read_view_count`]) and leaves the phone somewhere else. Reading it first would
/// take the rail off a screen that is no longer the post.
///
/// `read_views` exists because the two numbers cost three orders of magnitude apart. Likes and
/// comments are two label reads on a page already open; a view count walks the profile grid
/// opening tiles until a caption matches. Timed 24/08/2026: about four and a half minutes per
/// reading for a post near the top of the author's grid, and longer when it sits deeper. A caller
/// that only needs the cheap two must be able to say so rather than pay for both.
pub async fn read_post_now(
    session: &dyn UiSession,
    labels: TikTokControls,
    screen: (f64, f64),
    read_views: bool,
    stop: &AtomicBool,
) -> crate::interaction_threshold::PostNow {
    let rail = read_post_counters(session, labels).await;
    let views = if read_views {
        read_view_count(session, labels, screen, stop).await
    } else {
        None
    };
    crate::interaction_threshold::PostNow {
        views,
        likes: rail.likes,
        comments: rail.comments,
    }
}

/// A post as it appears on the author's profile grid.
///
/// The grid is the **only** place TikTok states a play count — the post page does not, which
/// is why a view threshold has to come here at all (measured 24/08/2026: the rail carries
/// likes, comments and shares and nothing else).
#[derive(Debug, Clone)]
pub struct ProfileTile {
    /// A point inside the thumbnail, for opening it.
    pub tap: crate::types::TapPoint,
    pub views: u32,
    /// The count was abbreviated (`1.2K`), so it is rounded.
    pub approximate: bool,
}

/// Every play count currently on the profile grid, with somewhere to tap for each.
///
/// Paired **within one dump**, never across two. That is not fussiness: the same grid read
/// after a different scroll puts the tiles at different y, and pairing a count from one read
/// with a tile from another is how a measurement ends up describing the wrong post — it
/// happened twice while this was being worked out.
///
/// The tap point is derived from the count rather than from the tile node, because the
/// thumbnail is a bare `ImageView` with no id worth keying on: the count sits near the
/// bottom-left of its own tile, so a point up and to the right of it is inside that tile and
/// no other.
pub async fn read_profile_tiles(session: &dyn UiSession, screen: (f64, f64)) -> Vec<ProfileTile> {
    let Ok(nodes) = session
        .locate_all_described(ElementQuery::ClassName("android.widget.TextView"))
        .await
    else {
        return Vec::new();
    };
    // The header's own three numbers (following / followers / likes) live in the top strip.
    // Excluded by position: after one scroll they are gone anyway, and a threshold reading
    // the follower count as a play count would be badly wrong rather than slightly wrong.
    let header = screen.1 * 0.25;
    nodes
        .into_iter()
        .filter(|node| node.y > header)
        .filter_map(|node| {
            let text = node.description.as_deref()?;
            // **A play count node carries the number and nothing else.** Measured 24/08/2026:
            // the grid overlays read `431`, `1.2K` — no words, no spaces. "Starts with a digit"
            // was the only test, so a caption like `2026 was the year…` became a tile claiming
            // 2026 views, with a tap point derived from a caption's position.
            if !text.chars().all(|c| {
                c.is_ascii_digit() || matches!(c, '.' | ',' | 'K' | 'M' | 'B' | 'k' | 'm' | 'b')
            }) {
                return None;
            }
            if !text.starts_with(|c: char| c.is_ascii_digit()) {
                return None;
            }
            let (views, approximate) = parse_count(text)?;
            let tap = crate::types::TapPoint {
                x: node.x + TILE_TAP_RIGHT,
                y: node.y - TILE_TAP_UP,
            };
            // **The filter above guards where the count sits; this guards where the tap lands,
            // and they are `TILE_TAP_UP` apart.** A count just below the header line derives a
            // tap 200 px *inside* the strip that filter exists to exclude — and on a profile
            // that strip carries **Follow** and **Message**. A function documented as a read
            // would then follow a stranger from a real logged-in account, and a follow does not
            // undo itself. Discarded rather than clamped: a tile whose tap falls up there is
            // only part-scrolled into view, and it comes back whole on the next scroll.
            //
            // The screen bounds are checked for the same reason in the other direction: nothing
            // guarantees `window_size` reported anything sane, and a negative or off-screen tap
            // lands wherever the platform decides.
            if tap.y < header || tap.y >= screen.1 || tap.x < 0.0 || tap.x >= screen.0 {
                return None;
            }
            Some(ProfileTile {
                tap,
                views,
                approximate,
            })
        })
        .collect()
}

/// The caption of whatever post is on screen, used to tell one post from another.
///
/// Deliberately the **caption** and not the counters. The counters are what a threshold is
/// moving, so identifying a post by them would mean the post stops being recognisable as
/// soon as the farming works. A caption is long, unique and does not change.
///
/// **Found by `resource-id`, because the class name is not stable.** Measured 24/08/2026, the
/// same post open on both builds this farm runs:
///
/// ```text
/// com.ss.android.ugc.trill      class=com.bytedance.tux.input.TuxTextLayoutView   id=…:id/desc
/// com.zhiliaoapp.musically      class=X.1BOr                                      id=…:id/desc
/// ```
///
/// `X.1BOr` is an obfuscated name that changes with the build, so a class-keyed lookup read the
/// caption on sixteen phones and returned nothing on four — and "nothing" is indistinguishable
/// from "this is not a post page", which is how a whole pass got recorded as "the deep link did
/// not navigate" when the post had been open the entire time. The id suffix is the same on both
/// and carries no package, so one query serves every build.
///
/// The two classes stay as a fallback: they are what was measured first, they still work, and a
/// build that renames `desc` would otherwise take the caption with it. `TextView` is last on
/// purpose — with the comment drawer open it returns a *comment*, which is worse than nothing
/// because it looks like an answer.
///
/// The `TextView` sweep stays as a fallback for a build that renders it the ordinary way, and
/// the longest string wins in both: a caption is longer than any label or button on the page.
pub async fn read_post_caption(session: &dyn UiSession) -> Option<String> {
    const CAPTION_MIN_CHARS: usize = 40;
    /// The id suffix first, then the classes that were measured before it.
    const CAPTION_ID_SUFFIX: &str = ":id/desc";
    const CAPTION_CLASSES: [&str; 2] = [
        "com.bytedance.tux.input.TuxTextLayoutView",
        "android.widget.TextView",
    ];
    let by_id = std::iter::once(ElementQuery::ResourceIdSuffix(CAPTION_ID_SUFFIX));
    let by_class = CAPTION_CLASSES.into_iter().map(ElementQuery::ClassName);
    for query in by_id.chain(by_class) {
        let Ok(nodes) = session.locate_all_described(query).await else {
            continue;
        };
        let longest = nodes
            .into_iter()
            .filter_map(|node| node.description)
            .map(|text| text.trim().to_string())
            .filter(|text| text.chars().count() >= CAPTION_MIN_CHARS)
            .max_by_key(|text| text.chars().count());
        if longest.is_some() {
            return longest;
        }
    }
    // **Say so, loudly.** A silent `None` here is indistinguishable from "this is not a post
    // page", and that ambiguity is what made a whole pass unmeasurable on three phones: they had
    // TikTok in the foreground with the post very possibly open, and all this could report was
    // nothing. Naming the classes it tried turns the next investigation into one grep instead of
    // a hierarchy dump and a guess — run `examples/label_scout` on the phone at its post page and
    // add whatever class the caption really uses.
    tracing::warn!(
        "no caption node with >= {CAPTION_MIN_CHARS} characters under an id ending \n         {CAPTION_ID_SUFFIX:?} or in any of {CAPTION_CLASSES:?} — either this is not a post \n         page, or this TikTok build names the caption something never measured"
    );
    None
}

/// How many times the grid is re-read after `back()` before concluding it is not there.
///
/// Three at `PROFILE_SETTLE` each, so 7.5 s. A grid that has not come back by then is a screen
/// that is not the grid; a grid that comes back on the second read is just a slow frame, and
/// treating that as a failure aborted whole readings.
const PROFILE_BACK_ATTEMPTS: u32 = 3;

/// How many grid rows to scroll through looking for the post.
///
/// Bounded so a profile with hundreds of posts cannot turn one reading into an unbounded
/// walk. A post further down than this reports `None` — "not found", never a guess.
pub const PROFILE_SCROLL_ATTEMPTS: u32 = 6;

/// The target post's play count, read off the author's profile grid.
///
/// **This is a navigation, not a read**, and that is the honest cost of a view threshold:
/// the number is only on the grid, the grid says nothing about which post a tile is, so each
/// candidate is opened and its caption compared before its count is believed.
///
/// **Where it leaves the phone.** On success, on the matched post's own page, reached through
/// the profile — so two frames deep, not back where it started. On every `None`, wherever the
/// refusal happened: the grid, usually. It does not restore the screen it was given, and the
/// doc used to claim it did on every path, which was true on none of them.
///
/// `None` means the post was not found within the scroll budget, the caption could not be
/// read, or the walk lost the grid — never a number from a tile that was not checked.
pub async fn read_view_count(
    session: &dyn UiSession,
    labels: TikTokControls,
    screen: (f64, f64),
    stop: &AtomicBool,
) -> Option<u32> {
    let wanted = read_post_caption(session).await?;
    let profile = labels.label(TikTokControl::AuthorProfileLink)?;
    let link = session.locate(profile.to_query()).await.ok().flatten()?;
    session.tap(link.centre()).await.ok()?;
    tokio::time::sleep(PROFILE_SETTLE).await;

    // Keyed on the **pair**, because a row of the grid shares one `y`. `ElementBox.y` is the
    // top edge and `tap.y` is a fixed offset from it, so every count in a row derives exactly
    // the same y — and a y-only guard skipped every tile but the leftmost. On a three-column
    // grid that silently examined one post in three while scrolling past the rest, then
    // reported the indistinguishable "not found".
    let mut checked: Vec<(f64, f64)> = Vec::new();
    for _ in 0..PROFILE_SCROLL_ATTEMPTS {
        if stop.load(Ordering::Relaxed) {
            return None;
        }
        for tile in read_profile_tiles(session, screen).await {
            // Tiles already opened keep their position only within one view; `checked` is
            // cleared after every scroll for that reason. The guard is against re-opening the
            // same tile in the same view.
            if checked
                .iter()
                .any(|(x, y)| (x - tile.tap.x).abs() < 20.0 && (y - tile.tap.y).abs() < 20.0)
            {
                continue;
            }
            checked.push((tile.tap.x, tile.tap.y));
            if session.tap(tile.tap.clone()).await.is_err() {
                continue;
            }
            tokio::time::sleep(PROFILE_SETTLE).await;
            let caption = read_post_caption(session).await;
            let found = caption.as_deref() == Some(wanted.as_str());
            if found {
                tracing::info!(
                    "view count {} read from the tile whose caption matched {:?}",
                    tile.views,
                    wanted.chars().take(50).collect::<String>()
                );
                return Some(tile.views);
            }
            // **Back to the grid — and then check that is where it landed.** Back from a post
            // opened through a tile does not reliably return to the grid; it can leave the phone
            // on the feed. The next act of this loop is to tap a coordinate computed from the
            // *grid*, and on the feed that point is the video surface, where a tap opens the
            // author or the comment drawer. An empty grid read is the refusal: scrolling cannot
            // recover a screen that is not the grid.
            session.back().await.ok()?;
            // **Read until it answers, not once.** The first version checked the grid a single
            // `PROFILE_SETTLE` after `back()` and refused on an empty read — and an empty read
            // there usually means "the grid has not re-rendered yet", exactly the mistake the
            // parent-scroll loop in this file made and had to be fixed for. Measured 24/08/2026:
            // it aborted the whole walk on the first tile often enough to turn a working reading
            // into `views=None`. Refusing only after the budget is spent keeps the check that a
            // blind tap never lands on the feed, without inventing a failure out of a slow frame.
            let mut back_on_grid = false;
            for _ in 0..PROFILE_BACK_ATTEMPTS {
                tokio::time::sleep(PROFILE_SETTLE).await;
                if !read_profile_tiles(session, screen).await.is_empty() {
                    back_on_grid = true;
                    break;
                }
            }
            if !back_on_grid {
                tracing::warn!("back from a tile did not return to the profile grid; refusing");
                return None;
            }
        }
        checked.clear();
        scroll_profile_grid(session, screen).await.ok()?;
    }
    None
}

/// One grid row's worth of scroll, inside the grid rather than over the header.
async fn scroll_profile_grid(session: &dyn UiSession, screen: (f64, f64)) -> anyhow::Result<()> {
    session
        .swipe(crate::types::SwipeGesture {
            from: crate::types::TapPoint {
                x: screen.0 / 2.0,
                y: screen.1 * 0.75,
            },
            to: crate::types::TapPoint {
                x: screen.0 / 2.0,
                y: screen.1 * 0.30,
            },
            duration_ms: 300,
        })
        .await?;
    tokio::time::sleep(PROFILE_SETTLE).await;
    Ok(())
}

/// How long to let the profile settle after a tap or a scroll.
///
/// Waiting for the **grid or the post page to finish drawing**: both are network-backed, and
/// a read taken too early sees an empty grid — which the caller cannot tell apart from a
/// profile with no posts. Matched to `PARENT_SCROLL_SETTLE`, measured on the same fleet.
const PROFILE_SETTLE: Duration = Duration::from_millis(2_500);

/// How far from a play count the middle of its thumbnail is.
///
/// Measured on the 1080-wide grid: tiles are 358x477 with the count at the bottom-left, so
/// this lands well inside the same tile and nowhere near its neighbours.
const TILE_TAP_RIGHT: f64 = 100.0;
const TILE_TAP_UP: f64 = 200.0;

/// What the comment box holds right now.
///
/// `locate_all_described` reads the rendered `text` into `description`, which for an
/// `EditText` is its contents — the same read the drawer uses to tell a placeholder from a
/// draft. `None` when there is no field or it is empty.
async fn composer_text(session: &dyn UiSession) -> anyhow::Result<Option<String>> {
    Ok(session
        .locate_all_described(ElementQuery::ClassName(crate::tiktok_drawer::EDIT_TEXT))
        .await?
        .into_iter()
        .find_map(|field| field.description)
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty()))
}

/// Whether a handle can be sent as key events at all.
///
/// TikTok handles are `[A-Za-z0-9._-]`; anything else is either not a handle or would need
/// shell escaping that deliberately does not exist (see [`UiSession::type_keys`]). A handle
/// that fails this is left as the literal text the operator typed, which is what the feature
/// did for every handle before real mentions existed.
fn is_typeable_handle(handle: &str) -> bool {
    !handle.is_empty()
        && handle
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

/// Wait for a suggestion row that *is* this handle and was not on screen before it was typed.
async fn await_mention_row(
    session: &dyn UiSession,
    handle: &str,
    before: &[ElementBox],
    stop: &AtomicBool,
) -> anyhow::Result<Option<ElementBox>> {
    let deadline = Instant::now() + MENTION_PICKER_WAIT;
    loop {
        if stop.load(Ordering::Relaxed) {
            return Ok(None);
        }
        // **Sleep before the first read, not after it.** The list is a network fetch, so at
        // t=0 it definitionally has not arrived — and the only thing a read taken then can
        // match is something that was already on screen, which is exactly the comment row
        // this must never tap.
        tokio::time::sleep(MENTION_PICKER_POLL).await;
        if let Some(row) = new_mention_row(mention_rows(session, handle).await?, before) {
            return Ok(Some(row));
        }
        if Instant::now() >= deadline {
            return Ok(None);
        }
    }
}

/// Every row on screen whose text is exactly this handle.
///
/// Deliberately unscoped. The picker's own container has never been measured, and inventing a
/// selector for it would be the kind of guess this file refuses everywhere else — so the
/// sweep stays wide and [`new_mention_row`] supplies the discrimination instead.
async fn mention_rows(session: &dyn UiSession, handle: &str) -> anyhow::Result<Vec<ElementBox>> {
    let wanted = handle.trim_start_matches('@').to_lowercase();
    Ok(session
        .locate_all_described(ElementQuery::ClassName("android.widget.TextView"))
        .await?
        .into_iter()
        .filter(|row| {
            row.description
                .as_deref()
                .map(|text| text.trim().trim_start_matches('@').to_lowercase() == wanted)
                .unwrap_or(false)
        })
        .collect())
}

/// The one row claiming this handle that was **not** there before the handle was typed.
///
/// `before` is the same sweep taken a moment earlier, so a comment author who happens to be
/// the tag target cancels out. Without it the sweep matched that author's comment row while
/// the picker was still in flight, tapped it, opened their profile, and destroyed the drawer
/// with the unsent draft in it — and then recorded the handle as a real mention because the
/// row was no longer on screen.
///
/// Two *new* rows cannot happen on TikTok, but if the read ever returns two this refuses
/// rather than picking one: the cost of being wrong is a mention of somebody else's account
/// from a real login.
fn new_mention_row(now: Vec<ElementBox>, before: &[ElementBox]) -> Option<ElementBox> {
    let mut fresh = now
        .into_iter()
        .filter(|row| !before.iter().any(|seen| same_box(seen, row)));
    let first = fresh.next()?;
    fresh.next().is_none().then_some(first)
}

/// How far a node may move between two reads and still be the same node, in pixels.
///
/// Not a tuned threshold so much as a tolerance for redraw: the comparison exists to tell
/// "this row was already here" from "this row is new", and those differ by a whole row height
/// (≈120 px on the 1080-wide phones on this farm), not by a few pixels. Four is small enough
/// that no two distinct rows can collapse into one and large enough to absorb a sub-pixel
/// layout pass. Comparing all four edges rather than the centre is deliberate — two rows in a
/// list share a centre column, so x alone identifies nothing.
const BOX_SLACK: f64 = 4.0;

/// Whether two reads describe the same node on screen.
fn same_box(left: &ElementBox, right: &ElementBox) -> bool {
    (left.x - right.x).abs() < BOX_SLACK
        && (left.y - right.y).abs() < BOX_SLACK
        && (left.width - right.width).abs() < BOX_SLACK
        && (left.height - right.height).abs() < BOX_SLACK
}

/// How many times the comment list is scrolled looking for the parent.
///
/// Shared with the pixel path, which imports it: every reply is sent from a *different*
/// device that re-opens the link fresh, so TikTok re-ranks the list and the campaign's own
/// comment is under no obligation to still be near the top. The budget is a property of the
/// comment list, not of whichever path is scrolling it, so the two must not drift again —
/// they did once, and only the hierarchy half got the measured fix below.
///
/// **Ten, and the old four was measured short rather than argued short.** On 19/08/2026, on
/// a post carrying about twenty-five comments, the third actor of a star spent all four
/// scrolls, found no folded section to open, and refused — while the second actor, looking a
/// minute earlier, had found the same parent and replied. Nothing was wrong except that the
/// list was longer than the budget. One scroll covers roughly two or three rows, so four
/// reaches about ten comments deep: fine for the post this number was chosen on, and short
/// for any post a campaign has been working.
///
/// The cost is paid only by replies that fail, and only in full by the ones that were going
/// to fail anyway — a parent found on the second scroll still costs two. Bounded because a
/// list that will not yield the parent has to end in a refusal rather than in scrolling
/// forever, and because a swipe that closes the drawer is caught on every turn.
pub const PARENT_SCROLL_ATTEMPTS: u32 = 10;

/// How long the feed gets to render an author label after the phone is sent Home.
///
/// Bounded, and a miss is usually not a failure: the baseline read before the tap is kept.
/// It *is* a failure when there was nothing to keep — `ArrivalRefusal::NoBaseline`.
///
/// **Stays at four, and the experiment that says so is worth recording.** Five phones on a
/// twenty-phone run refused with `no_baseline`, so this was widened to twelve on the theory
/// that a cold-started feed needed longer. It did not help — the next run refused five again.
/// The cause was upstream: those phones were still on TikTok's splash screen, which the
/// foreground proof accepted because a splash carries the app's own package. Waiting longer
/// for a label on a screen that has none buys nothing, and a number raised on a theory the
/// measurement refuted is worse than the number it replaced.
const BASELINE_SETTLE: Duration = Duration::from_secs(4);

/// Tap `View folded comments`, if this build has it measured and it is on screen.
///
/// **Only ever called at the end of the list**, which is the one place it can be right: the
/// control sits below the last open comment, and reaching it means the parent was not in the
/// open list. Tapping it reveals; it posts nothing, follows nobody and subscribes to nothing,
/// so it is safe in the way that matters for a recovery path.
///
/// Returns whether anything was expanded, so the caller can spend one more scroll budget on
/// the newly-revealed rows and, more importantly, can tell the operator that the thread it is
/// about to build lives in the folded section.
async fn expand_folded_comments(
    session: &dyn UiSession,
    labels: TikTokControls,
) -> anyhow::Result<bool> {
    let Some(label) = labels.label(TikTokControl::FoldedComments) else {
        return Ok(false);
    };
    let Some(control) = session.locate(label.to_query()).await? else {
        return Ok(false);
    };
    session.tap(control.centre()).await?;
    tokio::time::sleep(PARENT_SCROLL_SETTLE).await;
    Ok(true)
}
const PARENT_SCROLL_DURATION_MS: u64 = 320;
/// Time given to the comment list to stop moving before it is read again.
///
/// TikTok's list keeps gliding after the finger leaves, and a hierarchy dump taken mid-glide
/// reports rows at coordinates they have already left. Paired with `SCROLL_PROGRESS_PX`
/// below: this decides *when* the list is read, that decides whether it moved.
const PARENT_SCROLL_SETTLE: Duration = Duration::from_millis(900);
/// How far the list must actually move for a swipe to count as having scrolled.
///
/// Larger than the touch planner's jitter so a re-read that landed a few pixels
/// differently is not mistaken for movement.
const SCROLL_PROGRESS_PX: f64 = 24.0;

/// Why a reply could not be sent. Every variant means **nothing was typed**.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReplyRefusal {
    /// This build's Reply control has never been measured.
    ReplyUnmeasured,
    /// The drawer never opened.
    NoDrawer,
    /// The parent comment was not on screen after the scroll budget ran out.
    ParentNotFound {
        scrolls: u32,
        unfolded: bool,
        /// Whether the comment list ever showed a single row. `false` means the drawer was
        /// open and empty the whole time, which is a different problem from a parent that
        /// is simply not in the list.
        saw_rows: bool,
    },
    /// A swipe closed the drawer instead of scrolling the list.
    DrawerClosedByScroll,
    /// The Reply button was tapped and no composer appeared.
    NoComposer,
    /// The composer appeared but its placeholder does not name the parent's author, so
    /// the tap landed on somebody else's Reply button.
    ///
    /// The strongest check on this path, and it is free: the placeholder was measured as
    /// `Trả lời <author>`, which names who is being replied to. Checked *before* typing,
    /// so a wrong row costs nothing.
    WrongParentComposer { placeholder: String, wanted: String },
}

impl ReplyRefusal {
    pub fn code(&self) -> &'static str {
        match self {
            Self::ReplyUnmeasured => "reply_control_unmeasured",
            Self::NoDrawer => "reply_no_drawer",
            Self::ParentNotFound { .. } => "reply_parent_not_found",
            Self::DrawerClosedByScroll => "reply_drawer_closed_by_scroll",
            Self::NoComposer => "reply_no_composer",
            Self::WrongParentComposer { .. } => "reply_wrong_parent",
        }
    }

    pub fn message(&self) -> String {
        match self {
            Self::ReplyUnmeasured => {
                "build/ngôn ngữ này chưa đo nút Trả lời, nên không có gì để bấm".to_string()
            }
            Self::NoDrawer => "drawer bình luận không mở ra ô nhập".to_string(),
            Self::ParentNotFound {
                scrolls,
                unfolded,
                saw_rows,
            } => {
                // A drawer that never rendered a row is not a list that ran out. Saying so
                // sends the operator to the phone's network rather than to the post.
                if !*saw_rows {
                    return format!(
                        "khay bình luận mở ra nhưng không hiện dòng nào sau {scrolls} lần cuộn —                          nhiều khả năng máy chưa tải được bình luận (mạng), chứ không phải bài                          thiếu bình luận cha; không gõ gì cả"
                    );
                }
                let folded = if *unfolded {
                    " (đã mở phần bị gấp và tìm cả trong đó)"
                } else {
                    " (không có phần bị gấp nào để mở)"
                };
                format!(
                    "không tìm thấy bình luận cha sau {scrolls} lần cuộn{folded}; không gõ gì cả"
                )
            }
            Self::DrawerClosedByScroll => "cú swipe đóng drawer thay vì cuộn danh sách".to_string(),
            Self::NoComposer => "đã bấm Trả lời nhưng không thấy ô nhập reply".to_string(),
            Self::WrongParentComposer {
                placeholder,
                wanted,
            } => format!(
                "ô reply ghi {placeholder:?}, không chứa tên tác giả cha {wanted:?} — đã bấm \
                 vào nút Trả lời của bình luận khác, từ chối trước khi gõ"
            ),
        }
    }
}

/// Reply to a specific comment by hierarchy, leaving the drawer open.
///
/// The order of evidence matters and it is all gathered **before anything is typed**:
///
/// 1. The parent's body appears **exactly once** in the list — the ported uniqueness
///    rule ([`locate_parent_in_elements`]).
/// 2. Its Reply control is the **nearest below and to the right** of that body, not the
///    first one the tree lists.
/// 3. After tapping it, the composer's placeholder **names the parent's author**. This
///    is a second, independent proof that the right button was hit, measured as
///    `Trả lời <author>`; it is compared with `contains`, not as a prefix, precisely so
///    the check does not depend on the translated `Trả lời ` part and no new catalogue
///    entry is needed.
///
/// Only then does it type. A failure at any step is a refusal with nothing posted.
pub async fn send_reply_by_hierarchy<F>(
    session: &dyn UiSession,
    labels: TikTokControls,
    screen: (f64, f64),
    parent: &CommentLocatorIdentity,
    text: &str,
    stop: &AtomicBool,
    frame_sha: F,
) -> Result<Result<HierarchySendOutcome, ReplyRefusal>, HierarchySendFailure>
where
    F: FnMut() -> String,
{
    let mut effect_gate = crate::interaction_target::EffectGate::allow();
    send_reply_by_hierarchy_with_gate(
        session,
        labels,
        screen,
        parent,
        text,
        stop,
        frame_sha,
        &mut effect_gate,
    )
    .await
}

// As with the root sender, these arguments are separate proof inputs at the effect boundary;
// retaining that shape makes the parent identity and one-shot gate impossible to conflate.
#[allow(
    clippy::too_many_arguments,
    reason = "effect boundary keeps independently proved reply inputs explicit"
)]
pub(crate) async fn send_reply_by_hierarchy_with_gate<F>(
    session: &dyn UiSession,
    labels: TikTokControls,
    screen: (f64, f64),
    parent: &CommentLocatorIdentity,
    text: &str,
    stop: &AtomicBool,
    mut frame_sha: F,
    effect_gate: &mut crate::interaction_target::EffectGate<'_>,
) -> Result<Result<HierarchySendOutcome, ReplyRefusal>, HierarchySendFailure>
where
    F: FnMut() -> String,
{
    use crate::tiktok_drawer::{CommentDrawer, CommentVerdict, EDIT_TEXT};

    let Some(reply_label) = labels.label(TikTokControl::CommentReply) else {
        return Ok(Err(ReplyRefusal::ReplyUnmeasured));
    };
    let mut planner = crate::nurture::touch::TouchPointPlanner::new(screen);
    let plan = move |element: &ElementBox| planner.next(element.centre(), element.jitter_radius());
    let mut drawer = CommentDrawer::new(session, labels, plan);
    if drawer.send_query().is_none() {
        return Ok(Ok(HierarchySendOutcome {
            verdict: CommentVerdict::SendUnmeasured,
            // Replies never re-tag: only the opening comment carries the mentions.
            mention_note: None,
            parent_was_folded: false,
            armed_frame_sha256: String::new(),
            cleared_frame_sha256: String::new(),
            identity: None,
        }));
    }
    // Opening the drawer also gives the field, whose top edge bounds the list — the
    // measured field jumps from y≈2127 to y≈1175 when the keyboard comes up, so a fixed
    // screen fraction can land inside the composer instead of the list.
    // Before the Send tap: a transport error opening the drawer posted nothing, so it is
    // the retryable interrupted verdict, not the `after`-tap error.
    let field = match drawer.open(stop).await {
        Ok(Some(field)) => field,
        Ok(None) => return Ok(Err(ReplyRefusal::NoDrawer)),
        Err(_) => return Ok(Ok(HierarchySendOutcome::interrupted_before_send())),
    };

    let mut scrolls = 0u32;
    let mut unfolded = false;
    // Whether the list was ever legible at all — see the `unreadable` branch below.
    let mut saw_rows = false;
    let target = loop {
        match find_parent(session, reply_label, parent).await {
            Ok(Some(found)) => break found,
            Ok(None) => {}
            Err(_) => return Ok(Ok(HierarchySendOutcome::interrupted_before_send())),
        }
        if scrolls >= PARENT_SCROLL_ATTEMPTS || stop.load(Ordering::Relaxed) {
            return Ok(Err(ReplyRefusal::ParentNotFound {
                scrolls,
                unfolded,
                saw_rows,
            }));
        }
        // Anchors before the swipe, so "the list did not move" is observable rather than
        // assumed. Reply controls are the cheapest anchor: geometry only, no text.
        let before = match anchor_positions(session, reply_label).await {
            Ok(before) => before,
            Err(_) => return Ok(Ok(HierarchySendOutcome::interrupted_before_send())),
        };
        let rows_before = match visible_rows(session).await {
            Ok(rows) => rows,
            Err(_) => return Ok(Ok(HierarchySendOutcome::interrupted_before_send())),
        };
        // Still before the Send tap; a transport error scrolling is retryable, not ambiguous.
        if scroll_comment_list(session, screen, &field).await.is_err() {
            return Ok(Ok(HierarchySendOutcome::interrupted_before_send()));
        }
        tokio::time::sleep(PARENT_SCROLL_SETTLE).await;
        scrolls += 1;
        // A swipe that closed the drawer is a different failure from one that hit the
        // end of the list, and the pixel path checks for exactly this.
        match session.locate(ElementQuery::ClassName(EDIT_TEXT)).await {
            Ok(Some(_)) => {}
            Ok(None) => return Ok(Err(ReplyRefusal::DrawerClosedByScroll)),
            Err(_) => return Ok(Ok(HierarchySendOutcome::interrupted_before_send())),
        }
        let after = match anchor_positions(session, reply_label).await {
            Ok(after) => after,
            Err(_) => return Ok(Ok(HierarchySendOutcome::interrupted_before_send())),
        };
        let rows_after = match visible_rows(session).await {
            Ok(rows) => rows,
            Err(_) => return Ok(Ok(HierarchySendOutcome::interrupted_before_send())),
        };
        // **An empty read is not a stationary list.** Both anchors answer with an empty
        // vector when they cannot see anything, and empty compares equal to empty: zero
        // reply controls before and zero after satisfy `!moved`, zero rows before and zero
        // rows after satisfy the text check, and the loop concludes it reached the end of a
        // list it never managed to read one row of.
        //
        // Measured 24/08/2026 on `.../@.lt.gi.mang.v/photo/7668947001618320660`, a post with
        // 22 comments: three replies refused `reply_parent_not_found` after **one** scroll,
        // with a budget of ten. The drawer opens before TikTok has rendered the comments, so
        // on a slow phone the first look is always empty — and that first look was being
        // read as proof there was nothing to find.
        //
        // Spending a scroll instead is the conservative reading: the budget still bounds the
        // loop, and the refusal at the end now says which of the two happened.
        if after.is_empty() && rows_after.is_empty() {
            continue;
        }
        saw_rows = true;
        if !moved(&before, &after) {
            // The cheap anchor says stopped. It is wrong often enough to matter — evenly
            // spaced rows alias — so the expensive one gets the final word, and only here,
            // where the alternative is refusing a reply whose parent is further down.
            if rows_after != rows_before {
                continue;
            }
            // The end of the *open* list, which is not the end of the comments. TikTok
            // folds these accounts' comments away progressively — measured on the same
            // post within the same hour, two replies found their parent in the open list
            // and the next two did not — so the parent is often one tap below the last
            // row rather than absent. This is the only place that tap can be right: every
            // scroll has been spent and the control sits under the final comment.
            //
            // The revealed rows get their own budget, once. `unfolded` latches, so a list
            // that keeps refusing to move still ends here.
            if !unfolded {
                match expand_folded_comments(session, labels).await {
                    Ok(true) => {
                        unfolded = true;
                        scrolls = 0;
                        continue;
                    }
                    Ok(false) => {}
                    Err(_) => {
                        return Ok(Ok(HierarchySendOutcome::interrupted_before_send()));
                    }
                }
            }
            return Ok(Err(ReplyRefusal::ParentNotFound {
                scrolls,
                unfolded,
                saw_rows,
            }));
        }
    };

    // The field already carries a **non-empty** placeholder before anything is tapped —
    // measured as `Thêm bình luận...`. So "wait for an EditText with some text in it" is
    // satisfied by the state we are already in, and would return instantly with the root
    // drawer's own hint. Read that hint first and wait for it to *change*.
    let before = match read_placeholder(session).await {
        Ok(before) => before.unwrap_or_default(),
        Err(_) => return Ok(Ok(HierarchySendOutcome::interrupted_before_send())),
    };

    // Tap this comment's own Reply control.
    let point = {
        let mut planner = crate::nurture::touch::TouchPointPlanner::new(screen);
        planner.next(target.reply.centre(), target.reply.jitter_radius())
    };
    // Tapping Reply to open the composer, still before anything is typed or sent.
    if session.tap(point).await.is_err() {
        return Ok(Ok(HierarchySendOutcome::interrupted_before_send()));
    }

    // The composer replaces the field rather than stacking on it (measured: still
    // exactly one `EditText`), and its *text* is the placeholder — which `locate` cannot
    // see, because `content-desc` is empty there.
    let placeholder = match await_composer(session, &before, stop).await {
        Ok(Some(placeholder)) => placeholder,
        Ok(None) => return Ok(Err(ReplyRefusal::NoComposer)),
        Err(_) => return Ok(Ok(HierarchySendOutcome::interrupted_before_send())),
    };
    // An empty stored author label is not a pass. It means the identity this reply is
    // aimed at carries no author, so the one independent check on this path cannot be
    // evaluated — and skipping it would leave the tap resting on geometry alone.
    let wanted = parent.author_label.trim();
    if wanted.is_empty() || !placeholder.contains(wanted) {
        return Ok(Err(ReplyRefusal::WrongParentComposer {
            placeholder,
            wanted: wanted.to_string(),
        }));
    }

    // Only now is anything typed.
    let field = match session.locate(ElementQuery::ClassName(EDIT_TEXT)).await {
        Ok(Some(field)) => field,
        Ok(None) => return Ok(Err(ReplyRefusal::NoComposer)),
        Err(_) => return Ok(Ok(HierarchySendOutcome::interrupted_before_send())),
    };
    // Same rule as the opening comment's path: a refusal is a verdict, not a transport
    // error, and a refused drawer is closed because there is nothing to read back out of it.
    // A transport error here is still before the Send tap, so it is retryable too.
    let typed = match drawer.focus_and_type(&field, text, stop).await {
        Ok(typed) => typed,
        Err(_) => {
            drawer.leave(stop).await;
            return Ok(Ok(HierarchySendOutcome::interrupted_before_send()));
        }
    };
    if typed != crate::tiktok_drawer::TypedInto::Typed {
        drawer.leave(stop).await;
        return Ok(Ok(HierarchySendOutcome {
            verdict: match typed {
                crate::tiktok_drawer::TypedInto::SendPreArmed => CommentVerdict::SendPreArmed,
                _ => CommentVerdict::NoSendControl,
            },
            // Replies never re-tag: only the opening comment carries the mentions.
            mention_note: None,
            parent_was_folded: false,
            armed_frame_sha256: String::new(),
            cleared_frame_sha256: String::new(),
            identity: None,
        }));
    }
    let send = match drawer.await_armed(stop).await {
        Ok(Some(send)) => send,
        Ok(None) => {
            return Ok(Ok(HierarchySendOutcome {
                verdict: CommentVerdict::NotArmed,
                // Replies never re-tag: only the opening comment carries the mentions.
                mention_note: None,
                parent_was_folded: unfolded,
                armed_frame_sha256: String::new(),
                cleared_frame_sha256: String::new(),
                identity: None,
            }));
        }
        // Still before the Send tap: retryable, not the ambiguous `after` error.
        Err(_) => return Ok(Ok(HierarchySendOutcome::interrupted_before_send())),
    };
    let armed = frame_sha();
    let confirmed = match effect_gate.cross() {
        Ok(()) => drawer
            .tap_send_and_confirm_disarm(&send, stop)
            .await
            .map_err(HierarchySendFailure::after)?,
        Err(failure) => {
            let cleaned = drawer.leave(stop).await;
            return Err(if cleaned || failure.ownership_lost() {
                HierarchySendFailure::from_gate(failure)
            } else {
                HierarchySendFailure::after(anyhow::anyhow!(
                    "effect gate failed after typing and reply UI cleanup was not verified: {}",
                    failure.into_error()
                ))
            });
        }
    };
    let cleared = frame_sha();
    if !confirmed {
        return Ok(Ok(HierarchySendOutcome {
            verdict: CommentVerdict::NotConfirmed,
            // Replies never re-tag: only the opening comment carries the mentions.
            mention_note: None,
            parent_was_folded: unfolded,
            armed_frame_sha256: armed,
            cleared_frame_sha256: cleared,
            identity: None,
        }));
    }
    // **Do not press Back unconditionally.** AGENTS.md §9.7 measured "Back from the
    // composer returns to the comment list with the drawer still open" — but that was
    // measured *before* Send. Measured after Send, on an SM-N950F on 11/08/2026, sending a
    // reply already collapses the composer, so the extra Back leaves the **drawer**: the
    // reply could not be read back, and the still-open drawer that
    // `publish_evidence_frame` depends on was gone.
    //
    // So ask first. The placeholder is the state: in the composer it names the parent, on
    // the list it is the generic hint. Only a composer gets a Back.
    if let Ok(Some(placeholder)) = read_placeholder(session).await {
        let wanted = parent.author_label.trim();
        if !wanted.is_empty() && placeholder.contains(wanted) {
            let _ = session.back().await;
            sleep_poll().await;
        }
    }
    let identity = read_back_identity(session, text, &cleared).await;
    Ok(Ok(HierarchySendOutcome {
        verdict: CommentVerdict::Sent,
        mention_note: None,
        parent_was_folded: unfolded,
        armed_frame_sha256: armed,
        cleared_frame_sha256: cleared,
        identity,
    }))
}

/// One pass over the visible list looking for `parent`.
async fn find_parent(
    session: &dyn UiSession,
    reply_label: crate::tiktok_labels::LabelMatch,
    parent: &CommentLocatorIdentity,
) -> anyhow::Result<Option<ElementReplyTarget>> {
    let wanted = parent.text.trim();
    if wanted.is_empty() {
        return Ok(None);
    }
    let bodies: Vec<ElementBox> = session
        .locate_all(ElementQuery::Text {
            value: wanted,
            exact: true,
        })
        .await?
        .into_iter()
        .map(|mut body| {
            // Known exactly: it is what the query matched on.
            body.description = Some(wanted.to_string());
            body
        })
        .collect();
    if bodies.is_empty() {
        return Ok(None);
    }
    let replies: Vec<ElementBox> = session
        .locate_all(reply_label.to_query())
        .await?
        .into_iter()
        .map(|mut reply| {
            reply.description = Some(reply_label.value().to_string());
            reply
        })
        .collect();
    // The expensive read, and only reached once a candidate body is on screen.
    let authors = session
        .locate_all_described(ElementQuery::ClassName(COMMENT_AUTHOR_CLASS))
        .await?;
    Ok(locate_parent_in_elements(
        &bodies, &replies, &authors, parent,
    ))
}

/// Y positions of the reply controls, as a cheap "did the list move" fingerprint.
async fn anchor_positions(
    session: &dyn UiSession,
    reply_label: crate::tiktok_labels::LabelMatch,
) -> anyhow::Result<Vec<f64>> {
    session
        .locate_all(reply_label.to_query())
        .await
        .map(|found| found.into_iter().map(|element| element.y).collect())
}

/// What the list is showing, as text.
///
/// The expensive anchor, and the reason there are two. [`anchor_positions`] is geometry only
/// and is right nearly always; but it reads the *y of the reply controls*, and comment rows
/// are close to evenly spaced, so a scroll that happens to advance about a whole number of
/// rows leaves those controls on the same pixels. The list moved and the anchor cannot tell.
///
/// Measured 19/08/2026: a reply refused with `reply_parent_not_found` after **two** scrolls on
/// a list of eighteen comments, having concluded it had reached the end. The text of the rows
/// cannot alias that way — different comments say different things.
async fn visible_rows(session: &dyn UiSession) -> anyhow::Result<Vec<String>> {
    Ok(session
        .locate_all_described(ElementQuery::ClassName("android.widget.TextView"))
        .await?
        .into_iter()
        .filter_map(|element| element.description)
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty())
        .collect())
}

/// Whether the list actually shifted. An unchanged set means the end of the list.
fn moved(before: &[f64], after: &[f64]) -> bool {
    if before.len() != after.len() {
        return true;
    }
    before
        .iter()
        .zip(after)
        .any(|(a, b)| (a - b).abs() > SCROLL_PROGRESS_PX)
}

/// Swipe the comment list, staying inside the drawer.
///
/// Bounded above by a margin below the drawer's top and below by the input field, rather
/// than by screen fractions: the field's measured position moves 950 px depending on
/// whether the keyboard is up, so a fixed fraction can land in the composer.
async fn scroll_comment_list(
    session: &dyn UiSession,
    screen: (f64, f64),
    field: &ElementBox,
) -> anyhow::Result<()> {
    let list_top = screen.1 * 0.35;
    let list_bottom = field.y.max(list_top + 200.0) - 40.0;
    let span = list_bottom - list_top;
    session
        .swipe(crate::types::SwipeGesture {
            from: crate::types::TapPoint {
                x: screen.0 * 0.5,
                y: list_top + span * 0.72,
            },
            to: crate::types::TapPoint {
                x: screen.0 * 0.5,
                y: list_top + span * 0.28,
            },
            duration_ms: PARENT_SCROLL_DURATION_MS,
        })
        .await
}

/// One drawer poll interval, for the few places that need to let the screen settle.
async fn sleep_poll() {
    tokio::time::sleep(crate::tiktok_drawer::DRAWER_POLL).await;
}

/// The comment field's placeholder text, if it has one.
///
/// Read with `locate_all_described` because the string lives in `text`, and `locate`
/// reports `content-desc` — measured empty on this field, so `locate` would answer
/// "no placeholder" for a field that plainly has one.
async fn read_placeholder(session: &dyn UiSession) -> anyhow::Result<Option<String>> {
    let fields = session
        .locate_all_described(ElementQuery::ClassName(crate::tiktok_drawer::EDIT_TEXT))
        .await?;
    Ok(fields
        .iter()
        .find_map(|field| field.description.as_deref())
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty()))
}

/// Wait for the reply composer's placeholder, which names who is being replied to.
///
/// **Waits for a placeholder that differs from `before`,** not merely for a non-empty
/// one. The measured field already holds `Thêm bình luận...` before the Reply button is
/// tapped, so "any EditText with text in it" is true of the state this is called from —
/// a wait on that predicate returns immediately with the root drawer's own hint, and the
/// author check then runs against the wrong string.
async fn await_composer(
    session: &dyn UiSession,
    before: &str,
    stop: &AtomicBool,
) -> anyhow::Result<Option<String>> {
    let deadline = Instant::now() + crate::tiktok_drawer::DRAWER_WINDOW;
    loop {
        if stop.load(Ordering::Relaxed) {
            return Ok(None);
        }
        if let Some(text) = read_placeholder(session).await? {
            if text != before {
                return Ok(Some(text));
            }
        }
        if Instant::now() >= deadline {
            return Ok(None);
        }
        tokio::time::sleep(crate::tiktok_drawer::DRAWER_POLL).await;
    }
}

/// Find a comment this project just posted, in the open drawer.
///
/// Queries by the **exact string this code typed**, which is the advantage this path has
/// over OCR: no transcription loss, so an exact match is sound and a miss is real.
async fn read_back_identity(
    session: &dyn UiSession,
    exact_text: &str,
    frame_sha256: &str,
) -> Option<CommentLocatorIdentity> {
    // Retried, because the comment does not appear in the list the instant Send disarms —
    // TikTok inserts the row after its own round trip. One query would report "not found"
    // for a comment that is on its way, and a missing identity breaks the whole rest of
    // the chain. The pixel path pays for the same thing with a 700 ms settle between its
    // two OCR passes.
    for attempt in 0..READ_BACK_ATTEMPTS {
        if attempt > 0 {
            tokio::time::sleep(READ_BACK_SETTLE).await;
        }
        if let Some(identity) = read_back_once(session, exact_text, frame_sha256).await {
            return Some(identity);
        }
    }
    None
}

/// How long to keep looking for a just-posted comment in the list.
///
/// Four attempts over ~2.1 s. Generous enough for a network insert, and bounded because
/// the alternative to finding it is a broken chain, not a retry.
const READ_BACK_ATTEMPTS: u32 = 4;
const READ_BACK_SETTLE: Duration = Duration::from_millis(700);

async fn read_back_once(
    session: &dyn UiSession,
    exact_text: &str,
    frame_sha256: &str,
) -> Option<CommentLocatorIdentity> {
    let bodies = session
        .locate_all(ElementQuery::Text {
            value: exact_text,
            exact: true,
        })
        .await
        .ok()?;
    // `locate_all` returns geometry only, so the text has to be put back — and it is
    // known exactly, because it is what the query matched on.
    let bodies: Vec<ElementBox> = bodies
        .into_iter()
        .map(|mut body| {
            body.description = Some(exact_text.to_string());
            body
        })
        .collect();
    // The authors are the expensive read: their strings differ per row and cannot be
    // inferred from the query. Once per send, never in a loop.
    let authors = session
        .locate_all_described(ElementQuery::ClassName(COMMENT_AUTHOR_CLASS))
        .await
        .ok()?;
    discover_identity_in_elements(
        &bodies,
        &authors,
        exact_text,
        frame_sha256,
        HIERARCHY_LOCATOR_VERSION,
    )
}

/// The bare nickname inside an author-bearing label, whichever label it came from.
///
/// **This is the safety-critical half of `read_author_label` and the reason the two sources
/// can be mixed at all.** The baseline is read before the link is opened and the arrival is
/// read after; if one of them came back `Follow Ánh` and the other `Ánh profile`, the check
/// would call an unchanged screen *changed* — and "the screen changed" is the entire proof
/// that the phone reached the post it was sent to. Folding both to `Ánh` is what keeps that
/// comparison honest.
///
/// The needle is stripped from wherever the catalogue puts it, because the builds disagree:
/// English is `<tên> profile`, Vietnamese is `Hồ sơ <tên>`, and `Follow ` is a prefix in
/// both. Returns `None` for a label with no name left in it — a prefix on its own is not an
/// identity, and an empty string here would make every post look like every other post.
fn bare_author_label(observed: &str, label: LabelMatch) -> Option<String> {
    let observed = observed.trim();
    let needle = label.value().trim();
    // An exact label carries no name by construction, so there is nothing to strip and
    // nothing to learn — the value *is* the label.
    let bare = if needle.is_empty() || label.is_exact() {
        observed.to_string()
    } else if let Some(rest) = strip_ignoring_case(observed, needle, true) {
        rest
    } else if let Some(rest) = strip_ignoring_case(observed, needle, false) {
        rest
    } else {
        observed.to_string()
    };
    let bare = bare.trim();
    (!bare.is_empty()).then(|| bare.to_string())
}

/// Strip `needle` from the front (`prefix`) or the back of `haystack`, case-insensitively —
/// which is how description matching behaves on the device, so the stripper has to agree
/// with the matcher that found the node.
fn strip_ignoring_case(haystack: &str, needle: &str, prefix: bool) -> Option<String> {
    let lower_needle = needle.to_lowercase();
    // Stripping nothing leaves everything, and the old shape answered that way too.
    if lower_needle.is_empty() {
        return Some(haystack.trim().to_string());
    }
    // **The match and the cut have to be measured on the same string.** This used to test
    // `lower_haystack.starts_with(&lower_needle)` and then slice `haystack` by `needle.len()`
    // — one comparison on the lowercased text, one index counted in bytes of the original.
    // Those agree only while every character in the matched region keeps its byte length
    // through `to_lowercase`, an invariant nothing stated and nothing checked. All three
    // needles in the catalogue today happen to satisfy it; the first that does not is a panic,
    // not a wrong answer, and `read_author_label` sits on the arrival hot path. U+212A KELVIN
    // SIGN is three bytes and lowercases to a one-byte `k`, so a needle containing `k` against
    // a display name using it indexes into the middle of a character.
    //
    // Walking real char boundaries costs a `to_lowercase` per boundary, on strings the length
    // of a button label. That is the right price for not being able to crash a live campaign.
    if prefix {
        haystack
            .char_indices()
            .map(|(at, ch)| at + ch.len_utf8())
            .find(|&end| haystack[..end].to_lowercase() == lower_needle)
            .map(|end| haystack[end..].trim().to_string())
    } else {
        haystack
            .char_indices()
            .map(|(at, _)| at)
            .find(|&start| haystack[start..].to_lowercase() == lower_needle)
            .map(|start| haystack[..start].trim().to_string())
    }
}

/// The author label of whatever post is on screen.
///
/// Two sources, in this order, because they fail on opposite screens. The **author's profile
/// link** on the action rail is there whether or not this account follows the creator; the
/// **Follow button** is not — it disappears the moment you follow, and on the account's own
/// post it never existed. Reading only the button is what made a followed post indistinguishable
/// from any other, so a perfectly good open was refused as `target_open_no_baseline`
/// (measured on the live fleet 24/08/2026, and again as `arrival = Structural` on the posts
/// that did get through: the comment landed, and nothing on screen could confirm it was the
/// right post).
///
/// Both are folded to the bare nickname by [`bare_author_label`] — see there for why that
/// matters more than the extra source does.
///
/// Unreadable still means `None`, and `None` still means the caller refuses. Widening where
/// the name can be read from must never widen what counts as having read one.
/// Public so a measurement can ask "whose post is this?" without navigating.
///
/// `open_target_by_hierarchy` answers the same question but *arrives* to do it, which is wrong for
/// a phone that is already there — and it is the only other way to get at this. See
/// `examples/threshold_gate`, which needs the author to tell one post from another across phones
/// after a caption turned out not to be able to.
pub async fn read_author_label(session: &dyn UiSession, labels: TikTokControls) -> Option<String> {
    for control in [TikTokControl::AuthorProfileLink, TikTokControl::Follow] {
        let Some(label) = labels.label(control) else {
            continue;
        };
        let Some(found) = session.locate(label.to_query()).await.ok().flatten() else {
            continue;
        };
        let observed = found.description.as_deref().unwrap_or_default();
        if let Some(bare) = bare_author_label(observed, label) {
            return Some(bare);
        }
    }
    None
}

/// What one walk through a photo post saw.
///
/// The counters are carried out rather than only logged because a two-slide post makes the two
/// endings indistinguishable from the outside: "read 2 of 2, so this is the last slide" and
/// "could not read the counter, so stop rather than risk the profile page" both hand back two
/// pictures. The first run of `examples/carousel_gate` on the second TikTok build in this fleet
/// looked like a pass for exactly that reason, and was not evidence of anything.
pub struct PhotoWalk {
    /// One picture per slide reached, in slide order.
    pub frames: Vec<Vec<u8>>,
    /// What the slide counter said after each swipe, in order.
    ///
    /// `None` means it could not be read, and when it is the last entry it is also why the walk
    /// stopped.
    pub counters: Vec<Option<(u32, u32)>>,
}

/// Where the carousel walk gets its pictures.
///
/// Production hands it the phone's live stream; the on-device gate hands it a screenshot. The
/// walk itself has to be the same code in both, because what it is really being asked to prove
/// is the *order of operations* — swipe, read the counter before it fades, settle, check the
/// post is still there, only then keep the picture — and a gate that re-implements that order
/// proves its own copy works.
#[async_trait::async_trait]
pub trait SlideCamera: Send + Sync {
    /// The picture currently on screen, or `None` if none can be had right now.
    async fn capture(&self) -> Option<Vec<u8>>;
}

/// How long to let a video play before the first sample.
///
/// A post that has just opened is showing a loading frame, a fade-in, or the first frame of a
/// title card. Measured on this fleet's own arrival path: the pictures the old sampler kept
/// were three copies of whatever was on screen in the first second, which on a video is
/// usually its cover.
const VIDEO_FIRST_SAMPLE: Duration = Duration::from_millis(1_200);

/// The longest a video may be watched for evidence.
///
/// **Bounded because a phone is held for all of it.** The sampling happens once per link — the
/// pre-pass photographs for every phone that will comment — so ten seconds is amortised across
/// the whole fan-out rather than paid per phone. It is still ten seconds of a device lease, and
/// a fifty-second video is not worth fifty.
const VIDEO_WATCH_BUDGET: Duration = Duration::from_secs(10);

/// The narrowest and widest gap between samples.
///
/// The floor keeps a very short video from being sampled faster than the stream produces
/// distinct frames; the ceiling keeps a long one from spending the whole budget on two
/// pictures.
const VIDEO_GAP_RANGE: (Duration, Duration) =
    (Duration::from_millis(1_200), Duration::from_secs(3));

/// How many identical samples in a row mean the video is not playing.
///
/// Two. One repeat is a stream that has not pushed a new frame yet; two is a paused video, a
/// still card, or a dead stream — and in all three cases waiting longer buys nothing. Measured
/// as a real state on this fleet: `ce051715cb22c30403` failed four assignments in a row with a
/// stuck stream, which from here is indistinguishable from a paused video and should end the
/// same way.
const VIDEO_STILL_STRIKES: usize = 2;

/// What a video walk saw, and how much of the video it actually spans.
pub struct VideoWatch {
    /// Samples in time order, oldest first.
    pub frames: Vec<Vec<u8>>,
    /// Seconds of video between the first kept sample and the last, rounded.
    ///
    /// **The honest half of this type.** Four pictures of a fifty-second video is four pictures
    /// of part of it, and a contact sheet that does not say so invites the model to narrate a
    /// video it saw a fraction of — the same dishonesty a short carousel sheet had.
    ///
    /// **Measured off a clock, not derived from the sample schedule** — and that distinction was
    /// paid for. The first version computed `gap * (kept - 1)` and reported **9 seconds** for a
    /// watch whose frames were the phở scene at ~6 s and `1/2 Circle Coffee` at ~28.5 s of the
    /// same video (`video_gate`, 26/08/2026). The schedule is not the truth because *taking* a
    /// picture is not free: a `screencap` of this fleet's 1080x2400 screen is a 2-4 MB PNG over
    /// USB and costs seconds of its own, and the video keeps playing through every one of them.
    /// Production's scrcpy stream is far cheaper, so there the two numbers nearly agree — but a
    /// number that is only right on the fast path is not a number to hand a model.
    pub span_secs: u64,
}

/// Photograph a video by watching it, not by sampling the instant it opens.
///
/// **What this replaces, and why it was wrong.** The old evidence pass took three stream frames
/// 500 ms apart, so it saw the first ~1.0 second of a post that can run a minute — and because
/// `make_contact_sheet` collapses identical pictures, a video still on its cover produced *one*
/// frame and the sheet honestly reported "ĐÚNG MỘT khung… KHÔNG có chuyển động để mô tả". Every
/// comment on such a post was written from its title card.
///
/// **It taps nothing and swipes nothing.** A video plays on its own, so unlike
/// [`photograph_photo_post`] there is no gesture here at all — which also means none of the
/// carousel walk's hazards (a swipe past the last slide leaving for the author's profile) exist
/// on this path. The only thing that can go wrong is keeping a picture of the wrong post.
///
/// **Two guards, and the second is the one that matters.** The comment rail still being on
/// screen proves *a* post page is up; it does not prove it is still **this** post, because
/// TikTok advances to the next video when one ends. So the caption is read once at the start
/// and compared before every sample is kept. When the caption cannot be read at all there is no
/// baseline, and the walk falls back to the rail check alone — the same level of proof the old
/// sampler had, rather than refusing to photograph anything.
///
/// `duration` sizes the gaps: a twelve-second video is covered end to end, a fifty-second one
/// gets its first ten seconds and [`VideoWatch::span_secs`] says so.
pub async fn photograph_video_post(
    session: &dyn UiSession,
    camera: &dyn SlideCamera,
    target_package: &str,
    duration: Option<Duration>,
) -> VideoWatch {
    let wanted = crate::openai_client::SHEET_MAX_FRAMES;
    let gap = video_sample_gap(duration, wanted);

    // Read before the first sample so the baseline is the post as it was on arrival.
    let baseline = read_post_caption(session).await;
    if baseline.is_none() {
        tracing::warn!(
            "interaction: khong doc duoc caption de lam moc — chi kiem bang thanh rail, \
             giong duong cu"
        );
    }
    let rail = post_page_rail(session, target_package).await;

    let mut frames: Vec<Vec<u8>> = Vec::with_capacity(wanted);
    let mut digests: Vec<u64> = Vec::with_capacity(wanted);
    let mut still = 0usize;
    // Stamped when a picture is *kept*, so the span covers exactly the frames on the sheet.
    let mut first_kept: Option<tokio::time::Instant> = None;
    let mut last_kept: Option<tokio::time::Instant> = None;
    tokio::time::sleep(VIDEO_FIRST_SAMPLE).await;

    for index in 0..wanted {
        if index > 0 {
            tokio::time::sleep(gap).await;
        }
        // Both checks before the picture is kept, in the cheap-first order: a rail lookup is one
        // query, a caption is a whole described sweep.
        if let Some(rail) = &rail {
            let on_post = session
                .locate(rail.to_query())
                .await
                .ok()
                .flatten()
                .is_some();
            if !on_post {
                tracing::warn!(
                    "interaction: roi khoi trang bai sau {} khung — dung, khong giu them",
                    frames.len()
                );
                break;
            }
        }
        if let Some(baseline) = &baseline {
            match read_post_caption(session).await {
                Some(now) if &now == baseline => {}
                other => {
                    // Measured hazard, not a hypothetical: a video that ends advances to the
                    // next one, and that screen has a comment rail on it too.
                    tracing::warn!(
                        "interaction: bai da doi sau {} khung (caption {:?}) — dung",
                        frames.len(),
                        other
                            .as_deref()
                            .map(|text| text.chars().take(24).collect::<String>())
                    );
                    break;
                }
            }
        }

        let Some(frame) = camera.capture().await else {
            continue;
        };
        // Deduplicated here as well as in the sheet, because here it can also *stop* the walk:
        // the sheet only learns a video was frozen after paying for the whole watch.
        // `picture_digest_of` and not a hash of the bytes: a JPEG stream re-encodes an
        // unchanged screen into different bytes every time, and the status bar's own animated
        // icons are excluded by it. Hashing the bytes is why the first version of the carousel
        // dedupe never fired on a real phone (9.104).
        if let Some(digest) = crate::nurture::picture_digest_of(&frame) {
            if digests.last() == Some(&digest) {
                still += 1;
                if still >= VIDEO_STILL_STRIKES {
                    tracing::info!(
                        "interaction: {} khung lien tiep giong nhau — video dung hoac stream \
                         ket, dung o {} khung",
                        still + 1,
                        frames.len()
                    );
                    break;
                }
                continue;
            }
            still = 0;
            digests.push(digest);
        }
        let now = tokio::time::Instant::now();
        first_kept = first_kept.or(Some(now));
        last_kept = Some(now);
        frames.push(frame);
    }

    // Clock, not schedule — see `VideoWatch::span_secs`. Stamping on *keep* also means an early
    // stop reports the span it really has rather than the one it planned for.
    let span_secs = match (first_kept, last_kept) {
        (Some(first), Some(last)) => {
            let span = last.saturating_duration_since(first);
            // Rounded to the nearest second: the prompt says "khoảng N giây", and a video cannot
            // usefully be described to a tenth.
            (span.as_millis() as u64 + 500) / 1_000
        }
        _ => 0,
    };
    VideoWatch { frames, span_secs }
}

/// How far apart to sample, given how long the video is.
///
/// Solves for covering the whole video when that fits the budget, and for spreading the budget
/// evenly when it does not. Clamped at both ends by [`VIDEO_GAP_RANGE`].
///
/// The arithmetic is on the gaps between samples, so `wanted` pictures need `wanted - 1` of
/// them — an off-by-one here would sample past the budget on every video.
fn video_sample_gap(duration: Option<Duration>, wanted: usize) -> Duration {
    let gaps = wanted.saturating_sub(1).max(1) as u32;
    let reach = match duration {
        // Cover the video itself when it is short enough to fit, minus the run-up already spent.
        Some(length) if length <= VIDEO_WATCH_BUDGET + VIDEO_FIRST_SAMPLE => {
            length.saturating_sub(VIDEO_FIRST_SAMPLE)
        }
        // Long, or unknown: spend the budget and admit the span.
        _ => VIDEO_WATCH_BUDGET,
    };
    (reach / gaps).clamp(VIDEO_GAP_RANGE.0, VIDEO_GAP_RANGE.1)
}

/// The control that says "a post page is up", or `None` when this build has no measured label.
///
/// Split out because both walks need it and the failure is the same in both: with no label set
/// there is no way to ask the question, and a walk that cannot ask it must not keep pictures on
/// the strength of a guess.
async fn post_page_rail(
    session: &dyn UiSession,
    target_package: &str,
) -> Option<crate::tiktok_labels::LabelMatch> {
    let language = session.ui_language().await.unwrap_or_default();
    let app_version = session
        .app_version(target_package)
        .await
        .unwrap_or_default();
    let labels = crate::tiktok_labels::controls_for(target_package, &language, &app_version)?;
    labels.label(crate::tiktok_labels::TikTokControl::Comments)
}

/// How many slides of a photo carousel are worth photographing.
///
/// The bound is about the **contact sheet**, not the phone. `openai_client` gives the sheet a
/// pixel budget per distinct picture, so every extra slide is more image the model is billed
/// for on every message the post gets. Four is where that stops being free; the post that
/// started this had two.
///
/// When the counter says the post is longer than this, the walk stops here and says so — a
/// short read that announces itself, rather than a sheet that quietly implies it saw all of it.
const CAROUSEL_SLIDE_CAP: usize = 4;

/// How long the page-turning drag takes.
///
/// Inside the range `human_behavior::photo_swipe_duration_ms` picks from for the nurture loop,
/// because this gesture goes out from a real account on a real post and there is no reason for
/// it to look different from the same account swiping a carousel any other day.
const CAROUSEL_SWIPE_MS: u64 = 420;

/// Photograph a photo post slide by slide.
///
/// **The defect this exists for**, measured 25/08/2026 on
/// `.../@pht.th.h.slay/photo/7668948504827448583`: the evidence pass took three stream frames
/// 500 ms apart and `make_contact_sheet` collapsed them, because a photo post does not move.
/// Every comment on that post was therefore written from **slide 1 only** — a person lying by
/// a lake — while the post is a two-slide carousel whose second slide carries the content, a
/// costed three-day itinerary. A carousel does not advance on its own; it waits to be swiped,
/// so no amount of extra sampling time would ever have reached the second slide.
///
/// **Why it stops the moment the counter is unreadable.** One swipe past the last slide does
/// not stay put and does not wrap: measured on the same post, it leaves for the author's
/// profile page, which has a Follow button on it. A follow from an operator's account is
/// permanent and this function is meant to be read-only, so an unread counter is treated as
/// *stop*, never as *probably one more*.
///
/// Frames are only kept while the comment rail is still on screen — the same structural proof
/// `open_target_by_hierarchy` uses for arrival. A picture taken after the post was left is not
/// evidence about the post, and it is worse than no picture because it looks like one.
pub async fn photograph_photo_post(
    session: &dyn UiSession,
    camera: &dyn SlideCamera,
    target_package: &str,
    gestures: &tokio::sync::Mutex<()>,
    settle: Duration,
) -> PhotoWalk {
    use rand::Rng;

    let mut frames = Vec::with_capacity(CAROUSEL_SLIDE_CAP);
    let mut counters = Vec::with_capacity(CAROUSEL_SLIDE_CAP);
    // The digest of the last frame actually kept. A no-op swipe leaves the counter — and the
    // picture — where they were, and without this the same slide is kept twice and the model
    // sees a repeated slide as evidence. The video walk already dedupes this way (see below);
    // the photo walk did not, which is the gap. `picture_digest_of` and not a byte hash, for
    // the reason that walk documents: a JPEG re-encodes an unchanged screen into new bytes.
    let mut last_kept_digest: Option<u64> = None;
    if let Some(frame) = camera.capture().await {
        last_kept_digest = crate::nurture::picture_digest_of(&frame);
        frames.push(frame);
    }

    // Without a measured label set there is no way to ask "is this still the post?", and
    // without that question there is no safe swipe. One slide is a thinner sheet than four;
    // a follow on a stranger's account is not recoverable at all.
    let language = session.ui_language().await.unwrap_or_default();
    let app_version = session
        .app_version(target_package)
        .await
        .unwrap_or_default();
    let Some(labels) = crate::tiktok_labels::controls_for(target_package, &language, &app_version)
    else {
        return PhotoWalk { frames, counters };
    };
    let Some(comments) = labels.label(crate::tiktok_labels::TikTokControl::Comments) else {
        return PhotoWalk { frames, counters };
    };
    // This fallback was **correct by coincidence**: 1080x2220 is exactly what today's S8/S8+
    // fleet reports, so a failed read produced the right number and nothing ever went wrong.
    // That is worse than a wrong constant, not better -- it is a landmine that only arms when
    // the fleet gains a phone of another size, and it arms silently, on the swipe path.
    // Same rule as everywhere else: a size the phone did not report is not a size.
    let Ok((width, height)) = crate::screen::measured_screen_size(session).await else {
        tracing::warn!("bỏ qua đi bài ảnh: không đọc được kích thước màn hình");
        return PhotoWalk { frames, counters };
    };
    let mut reported_total = None;

    // Bounded by **swipes**, not by frames collected. A stream that stops delivering leaves
    // `frames` empty, and a frame-counted loop would then keep flicking until the counter
    // happened to say stop -- up to `CAROUSEL_MAX_SLIDES` gestures on a real post. Three is
    // three whatever the stream does.
    for _ in 1..CAROUSEL_SLIDE_CAP {
        let gesture = {
            let mut rng = rand::thread_rng();
            let y = height * rng.gen_range(0.38..0.62);
            crate::types::SwipeGesture {
                from: crate::types::TapPoint {
                    x: width * rng.gen_range(0.74..0.84),
                    y,
                },
                to: crate::types::TapPoint {
                    x: width * rng.gen_range(0.16..0.27),
                    y: y + rng.gen_range(-4.0..4.0),
                },
                duration_ms: CAROUSEL_SWIPE_MS,
            }
        };
        {
            let _guard = gestures.lock().await;
            if let Err(error) = session.swipe(gesture).await {
                tracing::warn!("interaction: khong vuot duoc carousel: {error}");
                break;
            }
        }

        // Read the counter *before* settling. It is in the tree for about three seconds after
        // the page turns and then leaves it entirely — measured, a dump three seconds later is
        // byte-identical to the pre-swipe one — and the hierarchy round-trip is itself most of
        // the settle this would otherwise sleep through.
        let index = read_carousel_index(session).await;
        counters.push(index);
        tokio::time::sleep(settle).await;

        let still_on_post = session
            .locate(comments.to_query())
            .await
            .ok()
            .flatten()
            .is_some();
        if !still_on_post {
            tracing::warn!(
                "interaction: vuot ngang da roi bai sau {} anh - lui lai, khong giu khung nay",
                frames.len()
            );
            let _ = session.back().await;
            break;
        }
        if let Some(frame) = camera.capture().await {
            // Keep it only if the picture actually changed. A swipe the pager ignored leaves
            // the same slide on screen; keeping it would show the model a duplicate slide.
            let digest = crate::nurture::picture_digest_of(&frame);
            if digest.is_none() || digest != last_kept_digest {
                last_kept_digest = digest;
                frames.push(frame);
            }
        }

        let Some((current, total)) = index else {
            tracing::warn!(
                "interaction: khong doc duoc chi so slide - dung o {} anh thay vi vuot tiep",
                frames.len()
            );
            break;
        };
        reported_total = Some(total);
        if current >= total {
            break;
        }
    }

    if let Some(total) = reported_total {
        if total as usize > frames.len() {
            tracing::warn!(
                "interaction: bai co {total} anh, bang chung chi gom {} anh dau",
                frames.len()
            );
        }
    }
    PhotoWalk { frames, counters }
}

/// The most slides this will believe a post has.
///
/// Not a measured TikTok limit — a sanity bound. It is here so that a misread pair (a caption
/// that happens to render `1 / 2000`, a digit node that drifted onto the counter row) cannot
/// become a traversal that swipes two thousand times on a real account.
const CAROUSEL_MAX_SLIDES: u32 = 64;

/// How far out of line two nodes of the counter row may sit and still count as adjacent.
///
/// Measured **0 px** on the fleet build: the digit ends exactly where the separator begins
/// (`976` and `976`, then `1006` and `1006`). The tolerance is for rounding at another
/// density, not for joining up nodes that are visibly apart.
const CAROUSEL_ROW_SLACK: f64 = 8.0;

/// The class the counter's three nodes carry.
///
/// Only the *fallback* path needs it — see [`read_carousel_index`], which anchors on the
/// separator's text first precisely because a class name is the kind of thing that differs
/// between the two TikTok builds on this fleet.
const CAROUSEL_COUNTER_CLASS: &str = "android.widget.TextView";

/// Which slide of a photo carousel is on screen, as `(current, total)`.
///
/// **Measured 25/08/2026** on `com.ss.android.ugc.trill`, 1080x2220, opening
/// `.../@pht.th.h.slay/photo/7668948504827448583` — a two-slide post. The counter is three
/// sibling `android.widget.TextView`s laid out in one row at the top right:
///
/// ```text
///   text="2"    bounds=[955,195][976,234]
///   text=" / "  bounds=[976,195][1006,234]
///   text="2"    bounds=[1006,195][1027,234]
/// ```
///
/// Their parent is a `LinearLayout` whose resource-id ends `:id/llz`. **That id is deliberately
/// not the anchor.** It is minified, and a minified id is exactly what made the caption node
/// unreadable on the other build in this fleet — the story written up on
/// [`ElementQuery::ResourceIdSuffix`]. What this reads instead is the separator's own text and
/// the geometry of the row, neither of which is translated or obfuscated.
///
/// **The counter is transient, and that shapes the whole caller.** A dump taken immediately
/// after the swipe carries all three nodes; one taken three seconds later is byte-identical to
/// the pre-swipe dump — the counter has left the tree entirely. So it can only be read straight
/// after the gesture that turned the page.
///
/// `None` means **"could not read"**, never "one slide". A traversal has to treat it as *stop
/// here*, because the alternative was measured too: one swipe past the last slide leaves the
/// post and opens the author's profile page, Follow button and all, and a follow from a real
/// account does not expire.
pub async fn read_carousel_index(session: &dyn UiSession) -> Option<(u32, u32)> {
    let separators = session
        .locate_all_described(ElementQuery::Text {
            value: "/",
            exact: false,
        })
        .await
        .ok()?;
    if separators.is_empty() {
        return None;
    }
    // A build that renders the whole counter as one node answers here, without the row.
    for found in &separators {
        if let Some(pair) = parse_carousel_pair(found.description.as_deref().unwrap_or_default()) {
            return Some(pair);
        }
    }
    let texts = session
        .locate_all_described(ElementQuery::ClassName(CAROUSEL_COUNTER_CLASS))
        .await
        .ok()?;
    carousel_index_from(&separators, &texts)
}

/// Rebuild `current / total` from the separate nodes TikTok lays out around the separator.
///
/// Nearest-on-each-side rather than first-found, for the same reason the comment locator picks
/// the nearest row below a body: a screen carries plenty of numbers, and the only thing that
/// makes these two *the* pair is that they sit flush against the separator on its own row.
fn carousel_index_from(separators: &[ElementBox], texts: &[ElementBox]) -> Option<(u32, u32)> {
    for separator in separators {
        let middle = separator.y + separator.height / 2.0;
        let left_edge = separator.x;
        let right_edge = separator.x + separator.width;
        let mut current: Option<(f64, u32)> = None;
        let mut total: Option<(f64, u32)> = None;
        for candidate in texts {
            if middle < candidate.y || middle > candidate.y + candidate.height {
                continue;
            }
            let Some(value) = candidate
                .description
                .as_deref()
                .and_then(parse_slide_number)
            else {
                continue;
            };
            let candidate_right = candidate.x + candidate.width;
            if candidate_right <= left_edge + CAROUSEL_ROW_SLACK {
                let gap = (left_edge - candidate_right).abs();
                if current.is_none_or(|(best, _)| gap < best) {
                    current = Some((gap, value));
                }
            } else if candidate.x + CAROUSEL_ROW_SLACK >= right_edge {
                let gap = (candidate.x - right_edge).abs();
                if total.is_none_or(|(best, _)| gap < best) {
                    total = Some((gap, value));
                }
            }
        }
        let (Some((_, current)), Some((_, total))) = (current, total) else {
            continue;
        };
        if let Some(pair) = sane_pair(current, total) {
            return Some(pair);
        }
    }
    None
}

/// `"2 / 2"` in one node, for a build that does not split it.
fn parse_carousel_pair(text: &str) -> Option<(u32, u32)> {
    let (left, right) = text.trim().split_once('/')?;
    sane_pair(parse_slide_number(left)?, parse_slide_number(right)?)
}

/// One node of the counter: a short run of ASCII digits and nothing else.
///
/// Length-capped so that a stray node carrying a long number — a view count, a timestamp —
/// cannot present itself as a slide index just because it sits on the right row.
fn parse_slide_number(text: &str) -> Option<u32> {
    let trimmed = text.trim();
    if trimmed.is_empty() || trimmed.len() > 3 || !trimmed.bytes().all(|byte| byte.is_ascii_digit())
    {
        return None;
    }
    trimmed.parse().ok()
}

/// The one place the pair is judged, so both readers judge it the same way.
fn sane_pair(current: u32, total: u32) -> Option<(u32, u32)> {
    (current >= 1 && current <= total && total <= CAROUSEL_MAX_SLIDES).then_some((current, total))
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::driver::ElementQuery;
    use crate::tiktok_labels::controls_for;
    use crate::types::{SwipeGesture, TapPoint};
    use image::codecs::png::{CompressionType, FilterType, PngEncoder};
    use image::{ColorType, ImageEncoder, Rgb, RgbImage};
    use parking_lot::Mutex;

    const TIKTOK: &str = "com.ss.android.ugc.trill";

    /// The catalogued Follow label, not a hard-coded copy of it.
    ///
    /// It was a hard-coded `"Follow"` until the trailing space was added to stop the label
    /// matching the `Đã follow` tab; the fixture then silently stopped finding any author
    /// and four tests failed with `NoPostPage`. Reading it from the catalogue means the
    /// fixture keys on whatever the product keys on.
    fn follow_key() -> &'static str {
        vietnamese()
            .label(TikTokControl::Follow)
            .expect("the Vietnamese set measures Follow")
            .value()
    }

    /// The catalogued author-profile-link needle, for the same reason `follow_key` exists:
    /// the fixture must key on whatever the product keys on.
    fn profile_link_key() -> &'static str {
        vietnamese()
            .label(TikTokControl::AuthorProfileLink)
            .expect("the Vietnamese set measures the author profile link")
            .value()
    }

    fn vietnamese() -> TikTokControls {
        controls_for(TIKTOK, "vi", "46.3.3").expect("measured set")
    }

    /// A session for the arrival check: a foreground answer, a set of present labels,
    /// and a record of everything that was done to the device.
    ///
    /// The `taps`/`typed` records exist to assert the property that matters most about
    /// this function — **it changes nothing**. An arrival check that taps can dismiss a
    /// sheet or open a profile, and neither is recoverable from a log line.
    #[derive(Default)]
    struct ArrivalSession {
        foreground: Mutex<Vec<String>>,
        /// Whether `landing_on` should write the profile link rather than the Follow button.
        after_open_via_profile: Mutex<bool>,
        /// Label values that answer `locate`, and what they answer with.
        present: Mutex<Vec<(String, ElementBox)>>,
        opened: Mutex<Vec<String>>,
        taps: Mutex<Vec<TapPoint>>,
        typed: Mutex<Vec<String>>,
        open_url_fails: bool,
        /// Installed as the `Follow` label once the link has been opened.
        after_open: Mutex<Option<String>>,
        /// One-shot `Follow` answers after the link opens, then the fixture falls back to
        /// `present`. This models an adjacent card caught during the transition without
        /// turning that transient into the screen's settled author.
        after_open_follow_reads: Mutex<Vec<String>>,
        /// How many `Follow` reads to answer `None` for before the label appears.
        ///
        /// Models the transient the baseline retry exists for: `read_author_label` folds a
        /// query error into the same `None` as an absent node, so a fixture that can only
        /// say "always there" or "never there" cannot express one hiccup.
        follow_hidden_for: Mutex<u32>,
        follow_reads: Mutex<u32>,
    }

    impl ArrivalSession {
        fn new(foreground: &str, present: &[(&str, &str)]) -> Self {
            Self {
                foreground: Mutex::new(vec![foreground.to_string()]),
                present: Mutex::new(
                    present
                        .iter()
                        .map(|(key, label)| (key.to_string(), node(0.0, 0.0, 100.0, 100.0, label)))
                        .collect(),
                ),
                ..Default::default()
            }
        }

        /// The author label the screen shows **after** the link is opened.
        ///
        /// Modelling this is not fixture decoration: the arrival check's only real signal
        /// is that the post on screen *changed*, because a deep-linked post is rendered as
        /// the current card of the feed pager and is otherwise indistinguishable from it.
        /// A fixture that answers one author for ever can only express "the link did
        /// nothing".
        fn landing_on(self, author_label: &str) -> Self {
            *self.after_open.lock() = Some(author_label.to_string());
            self
        }

        /// Show one author read immediately after the open, then return to the settled card.
        fn transiently_landing_on(self, author_label: &str) -> Self {
            self.after_open_follow_reads
                .lock()
                .push(author_label.to_string());
            self
        }

        /// Land on the new post with only the **profile link** carrying the author.
        ///
        /// This is the followed-post shape: measured 24/08/2026 on the live fleet, following
        /// a creator removes `Follow <tên>` from the rail and leaves `<tên> profile` exactly
        /// as it was. A fixture that can only express the Follow button cannot express the
        /// screen this whole second source exists for.
        fn landing_on_profile_link(self, author_label: &str) -> Self {
            *self.after_open.lock() = Some(author_label.to_string());
            *self.after_open_via_profile.lock() = true;
            self
        }

        /// The `Follow` label answers `None` until the `attempt`-th read.
        fn follow_appears_on_attempt(self, attempt: u32) -> Self {
            *self.follow_hidden_for.lock() = attempt.saturating_sub(1);
            self
        }
    }

    #[async_trait::async_trait]
    impl UiSession for ArrivalSession {
        async fn tap(&self, point: TapPoint) -> anyhow::Result<()> {
            self.taps.lock().push(point);
            Ok(())
        }
        async fn swipe(&self, _gesture: SwipeGesture) -> anyhow::Result<()> {
            Ok(())
        }
        async fn type_text(&self, text: &str) -> anyhow::Result<()> {
            self.typed.lock().push(text.to_string());
            Ok(())
        }
        async fn home(&self) -> anyhow::Result<()> {
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
        async fn active_app_bundle(&self) -> anyhow::Result<String> {
            let mut queue = self.foreground.lock();
            if queue.len() > 1 {
                Ok(queue.remove(0))
            } else {
                queue
                    .first()
                    .cloned()
                    .ok_or_else(|| anyhow::anyhow!("none"))
            }
        }
        async fn open_url(&self, url: &str) -> anyhow::Result<()> {
            // Records the *unpinned* form, so a test can tell the two apart. The arrival
            // check must never use this one: a bare intent lands in the app chooser.
            self.opened.lock().push(format!("unpinned:{url}"));
            if self.open_url_fails {
                anyhow::bail!("intent refused");
            }
            Ok(())
        }
        async fn open_url_in_app(&self, url: &str, bundle_id: &str) -> anyhow::Result<()> {
            self.opened.lock().push(format!("{bundle_id}:{url}"));
            if self.open_url_fails {
                anyhow::bail!("intent refused");
            }
            // The screen changes, if the fixture said it should.
            if let Some(author) = self.after_open.lock().take() {
                let mut present = self.present.lock();
                let node = node(0.0, 0.0, 100.0, 100.0, &author);
                let key = if *self.after_open_via_profile.lock() {
                    profile_link_key()
                } else {
                    follow_key()
                };
                match present.iter_mut().find(|(slot, _)| slot == key) {
                    Some(slot) => slot.1 = node,
                    None => present.push((key.to_string(), node)),
                }
            }
            Ok(())
        }
        async fn locate(&self, query: ElementQuery<'_>) -> anyhow::Result<Option<ElementBox>> {
            let wanted = match query {
                ElementQuery::Description { value, .. } => value,
                ElementQuery::ClassName(value) => value,
                ElementQuery::Text { value, .. } => value,
                // Keyed the same way as everything else here: this fixture answers by the query's
                // value, whatever the strategy, and the arrival path does not use ids.
                ElementQuery::ResourceIdSuffix(value) => value,
            };
            if wanted == follow_key() {
                let mut reads = self.follow_reads.lock();
                *reads += 1;
                if *reads <= *self.follow_hidden_for.lock() {
                    return Ok(None);
                }
                if !self.opened.lock().is_empty() {
                    let mut scripted = self.after_open_follow_reads.lock();
                    if !scripted.is_empty() {
                        let author = scripted.remove(0);
                        return Ok(Some(node(0.0, 0.0, 100.0, 100.0, &author)));
                    }
                }
            }
            let present = self.present.lock();
            Ok(present
                .iter()
                .find(|(key, _)| key == wanted)
                .map(|(_, element)| element.clone()))
        }
    }

    /// Everything the arrival check needs to see a post page, and nothing more.
    fn post_page(handle_label: Option<&str>) -> Vec<(&'static str, &'static str)> {
        let mut present = vec![("bình luận", "Đọc hoặc viết bình luận. 3 bình luận")];
        if let Some(label) = handle_label {
            // Leaked as a `'static` so the fixture stays a plain slice; test-only.
            present.push((follow_key(), Box::leak(label.to_string().into_boxed_str())));
        }
        present
    }

    /// The two author sources have to fold to the same string, or mixing them lies.
    ///
    /// The baseline is read before the link opens and the arrival after. If a phone read
    /// `Follow Ánh` on one side and `Ánh profile` on the other, the check would call an
    /// unchanged screen *changed* — and "the screen changed" is the whole proof that the
    /// phone reached the post it was sent to. Getting this wrong types into a stranger's post.
    #[test]
    fn the_two_author_sources_fold_to_one_identity() {
        let vi = vietnamese();
        let en = controls_for(TIKTOK, "en", "38.3.2").expect("the English set is measured");
        let follow_vi = vi.label(TikTokControl::Follow).expect("Follow is measured");
        let profile_vi = vi
            .label(TikTokControl::AuthorProfileLink)
            .expect("the profile link is measured");
        let profile_en = en
            .label(TikTokControl::AuthorProfileLink)
            .expect("the profile link is measured");

        // Vietnamese puts the name last in both labels; English puts it first in the profile
        // link. All three are the same author.
        assert_eq!(
            bare_author_label("Follow Mộng Quỳnh", follow_vi).as_deref(),
            Some("Mộng Quỳnh")
        );
        assert_eq!(
            bare_author_label("Hồ sơ Mộng Quỳnh", profile_vi).as_deref(),
            Some("Mộng Quỳnh")
        );
        assert_eq!(
            bare_author_label("Đà Lạt Gói Mang Về profile", profile_en).as_deref(),
            Some("Đà Lạt Gói Mang Về")
        );
    }

    /// A label with the name stripped out of it is not an identity.
    ///
    /// Returning the empty string here would make every post look like every other post,
    /// which is precisely the comparison the arrival check is.
    #[test]
    fn a_prefix_only_label_is_not_an_identity() {
        let vi = vietnamese();
        let follow = vi.label(TikTokControl::Follow).expect("measured");
        let profile = vi
            .label(TikTokControl::AuthorProfileLink)
            .expect("measured");
        assert_eq!(bare_author_label("Follow ", follow), None);
        assert_eq!(bare_author_label("Hồ sơ", profile), None);
        assert_eq!(bare_author_label("   ", follow), None);
    }

    /// A post by a creator this account already follows still has an author to read.
    ///
    /// This is the gap the second source exists for, and it was measured on the live fleet
    /// on 24/08/2026: with only the Follow button to read, a followed post produced no
    /// baseline and no arrival, so the engine refused a perfectly good open as
    /// `target_open_no_baseline` — or got through with the proof downgraded to Structural,
    /// meaning the comment landed somewhere nobody could confirm was the right post.
    #[tokio::test(start_paused = true)]
    async fn a_followed_post_still_yields_a_baseline_and_arrives() {
        // Before: a post page whose author is readable only from the profile link. After:
        // a different author, again only from the profile link. No Follow button anywhere,
        // which is exactly what following the creator does to this screen.
        let session = ArrivalSession::new(
            TIKTOK,
            &[
                ("bình luận", "Đọc hoặc viết bình luận. 3 bình luận"),
                (profile_link_key(), "Hồ sơ Bích Vân"),
            ],
        )
        .landing_on_profile_link("Hồ sơ Mộng Quỳnh");

        let arrival = open_target_by_hierarchy(
            &session,
            vietnamese(),
            TIKTOK,
            "https://www.tiktok.com/@x/photo/1",
            "someone",
            &AtomicBool::new(false),
        )
        .await
        .expect("a followed post is still an arrival");

        // Structural rather than Identified only because this nickname does not fold onto
        // the handle — the point here is that it is an arrival at all. Before the second
        // source it was not one: no Follow button meant no author, and no author meant
        // `target_open_no_baseline` before the link was even opened.
        assert_eq!(arrival, TargetArrival::Structural);
        assert!(session.typed.lock().is_empty(), "arrival must not type");
    }

    #[test]
    fn the_handle_match_is_measured_against_real_accounts_and_mostly_fails() {
        // All three rows are real, read off a device on 11/08/2026. The point of the test
        // is the *ratio*: one in three. That is why a match upgrades the arrival proof and
        // never gates it — a predicate that fails on two accounts out of three is not a
        // predicate.
        assert!(
            author_matches_handle("Follow Mộng Quỳnh", "mongquynh.dalat"),
            "the nickname folds onto the handle here"
        );
        assert!(
            !author_matches_handle("Follow Ăn Sập Đi Hoang", "n.sp.i.hoang"),
            "the handle is a consonant skeleton of the nickname; no folding recovers it"
        );
        assert!(
            author_matches_handle("Follow Hương Thảo", "huongthao.dalat"),
            "same shape as Mộng Quỳnh"
        );
        assert!(
            !author_matches_handle("Follow Lúc này lúc kia", "nguyenvantoan8584"),
            "nickname and handle are simply unrelated"
        );
        // And the guards: a short accidental overlap is not a match.
        assert!(!author_matches_handle("Follow An", "anhtuan.dalat"));
        assert!(!author_matches_handle("Follow Somebody", "abc"));
        assert!(!author_matches_handle("", "mongquynh.dalat"));
        // A leading `@`, which a link carries and the label does not.
        assert!(author_matches_handle(
            "Follow Mộng Quỳnh",
            "@mongquynh.dalat"
        ));
    }

    #[tokio::test(start_paused = true)]
    async fn a_link_that_lands_in_a_browser_is_refused_and_nothing_is_typed() {
        // The Android-specific failure the pixel path has no check for: `am start …
        // VIEW` does not filter by package, so a TikTok link can open Chrome. The pixel
        // path would see the screen change and call that `Structural`.
        // The baseline is part of the fixture now, and has to be: an arrival check with
        // nothing readable before the open refuses at `NoBaseline` and never dispatches
        // the intent, so it could not reach a browser to complain about. A real phone is
        // sitting on a feed card here, which is what this models.
        let session = ArrivalSession::new("com.android.chrome", &post_page(Some("Follow Trước")));
        let refusal = open_target_by_hierarchy(
            &session,
            vietnamese(),
            TIKTOK,
            "https://www.tiktok.com/@someone/video/1",
            "someone",
            &AtomicBool::new(false),
        )
        .await
        .expect_err("a browser is not the target");
        assert_eq!(
            refusal,
            ArrivalRefusal::WrongApp {
                found: "com.android.chrome".into()
            }
        );
        assert_eq!(refusal.code(), "target_open_wrong_app");
        assert!(session.typed.lock().is_empty());
        assert!(session.taps.lock().is_empty(), "arrival must not tap");
    }

    #[tokio::test(start_paused = true)]
    async fn a_post_that_never_changes_is_refused_as_an_unresolved_link() {
        // The measured signature of a post that cannot be resolved — deleted, private, or
        // region-blocked. TikTok accepts the intent, fails server-side, and **leaves the
        // feed exactly where it was**. That screen has a comment control on it, so
        // `Comments` alone would call it an arrival and the campaign would comment on
        // whatever video was playing.
        //
        // Measured 11/08/2026 with a real dead link: the author stayed `Follow Bích Vân`
        // across four attempts while the comment count drifted with the feed.
        let session = ArrivalSession::new(TIKTOK, &post_page(Some("Follow Bích Vân")));
        let refusal = open_target_by_hierarchy(
            &session,
            vietnamese(),
            TIKTOK,
            "https://www.tiktok.com/@someone/photo/1",
            "someone",
            &AtomicBool::new(false),
        )
        .await
        .expect_err("an unchanged post is not an arrival");
        assert_eq!(
            refusal,
            ArrivalRefusal::ScreenNeverChanged {
                // Bare, because both author sources now fold to the nickname — see
                // `bare_author_label`. A baseline read off `Follow ` and an arrival read
                // off the profile link have to be the same string or the comparison lies.
                author_label: "Bích Vân".into()
            }
        );
        assert_eq!(refusal.code(), "target_open_screen_unchanged");
        assert!(session.typed.lock().is_empty());
        assert!(session.taps.lock().is_empty(), "arrival must not tap");
    }

    #[tokio::test(start_paused = true)]
    async fn a_transient_different_author_that_reverts_does_not_latch_arrival() {
        // The dead link settles back on Bich Van, but one poll catches an adjacent card while
        // TikTok is transitioning. That glimpse is shorter than HANDLE_GRACE and therefore
        // proves no arrival; every later poll sees the original post again.
        let session = ArrivalSession::new(TIKTOK, &post_page(Some("Follow Bich Van")))
            .transiently_landing_on("Follow Chớp Nhoáng");

        let refusal = open_target_by_hierarchy(
            &session,
            vietnamese(),
            TIKTOK,
            "https://www.tiktok.com/@someone/photo/1",
            "someone",
            &AtomicBool::new(false),
        )
        .await
        .expect_err("a reverted transient author is not a structural arrival");

        assert_eq!(
            refusal,
            ArrivalRefusal::ScreenNeverChanged {
                author_label: "Bich Van".into()
            }
        );
        assert!(session.typed.lock().is_empty());
        assert!(session.taps.lock().is_empty(), "arrival must not tap");
    }

    #[tokio::test(start_paused = true)]
    async fn the_feed_tab_staying_visible_does_not_prevent_an_arrival() {
        // Regression test for a predicate this function used to have and that measurement
        // disproved. A deep-linked post is rendered as the **current card of the For-You
        // pager**: the top tab row with `Đề xuất` selected and the bottom tab bar are both
        // still on screen. Requiring the feed tab to be absent refused every real arrival.
        let mut present = post_page(Some("Follow Bích Vân"));
        present.push(("Đề xuất", "Đề xuất"));
        let session = ArrivalSession::new(TIKTOK, &present).landing_on("Follow Mộng Quỳnh");
        let arrival = open_target_by_hierarchy(
            &session,
            vietnamese(),
            TIKTOK,
            "https://www.tiktok.com/@mongquynh.dalat/photo/7668954054680136967",
            "mongquynh.dalat",
            &AtomicBool::new(false),
        )
        .await
        .expect("the feed tab being visible must not refuse a real arrival");
        // And this account's nickname *does* fold onto its handle, so it identifies.
        assert_eq!(
            arrival,
            TargetArrival::Identified {
                author_label: "Mộng Quỳnh".into()
            }
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_post_page_without_a_readable_handle_is_structural() {
        // The expected case on this build, not the exceptional one: the labels that
        // carry an author string carry the *nickname*, which is routinely not the
        // handle. Downgrading rather than failing is the same call the pixel path makes
        // on Windows for every send.
        let session = ArrivalSession::new(TIKTOK, &post_page(Some("Follow Somebody Else")))
            .landing_on("Follow Ăn Sập Đi Hoang");
        let arrival = open_target_by_hierarchy(
            &session,
            vietnamese(),
            TIKTOK,
            "https://www.tiktok.com/@someone/video/1",
            "someone",
            &AtomicBool::new(false),
        )
        .await
        .expect("the post page is up");
        assert_eq!(arrival, TargetArrival::Structural);
        // Opened once, and **pinned to the target package**. Measured: a bare `VIEW`
        // intent for a tiktok.com URL resolves to the system app chooser, because Chrome
        // claims the domain too — so the link would reach a dialog, not the post.
        let opened = session.opened.lock().clone();
        assert_eq!(opened.len(), 1, "the link is opened once");
        assert_eq!(
            opened[0],
            format!("{TIKTOK}:https://www.tiktok.com/@someone/video/1"),
            "the arrival check must pin the intent to the target app"
        );
        assert!(session.taps.lock().is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn a_handle_in_the_follow_label_identifies_the_post() {
        let session = ArrivalSession::new(TIKTOK, &post_page(Some("Follow Somebody Else")))
            .landing_on("Follow nguyenvantoan8584");
        let arrival = open_target_by_hierarchy(
            &session,
            vietnamese(),
            TIKTOK,
            "https://www.tiktok.com/@nguyenvantoan8584/video/1",
            // With the `@` the link carries, which the label does not — stripping it is
            // the difference between identifying the post and never matching at all.
            "@nguyenvantoan8584",
            &AtomicBool::new(false),
        )
        .await
        .expect("identified");
        assert_eq!(
            arrival,
            TargetArrival::Identified {
                author_label: "nguyenvantoan8584".into()
            }
        );
        assert!(session.taps.lock().is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn a_nickname_that_is_not_the_handle_does_not_identify_the_post() {
        // Measured from this project's own capture: nickname `Lúc này lúc kia`, handle
        // `nguyenvantoan8584`. Matching loosely here would claim the wrong post is
        // confirmed, which is worse than admitting it is unidentified.
        let session = ArrivalSession::new(TIKTOK, &post_page(Some("Follow Somebody Else")))
            .landing_on("Follow Lúc này lúc kia");
        let arrival = open_target_by_hierarchy(
            &session,
            vietnamese(),
            TIKTOK,
            "https://www.tiktok.com/@nguyenvantoan8584/video/1",
            "nguyenvantoan8584",
            &AtomicBool::new(false),
        )
        .await
        .expect("the post page is still up");
        assert_eq!(arrival, TargetArrival::Structural);
    }

    #[tokio::test(start_paused = true)]
    async fn an_arrival_check_that_cannot_read_the_baseline_refuses_before_opening_the_link() {
        // The hole this closes was not hypothetical. `before` was
        // `read_author_label(..).unwrap_or_default()`, so an unreadable baseline became the
        // empty string, `author != before` was then true for *any* author, and the whole
        // check collapsed to "TikTok is foreground and a post page is up" — which the feed
        // satisfies. `ScreenNeverChanged` became unreachable and the campaign would have
        // commented on whatever post was on screen and called it a success.
        //
        // The strongest assertion here is `opened` being empty: the refusal happens before
        // the intent is dispatched, so a device that cannot be baselined costs no side
        // effect whatsoever.
        let session = ArrivalSession::new(TIKTOK, &post_page(None));
        let refusal = open_target_by_hierarchy(
            &session,
            vietnamese(),
            TIKTOK,
            "https://www.tiktok.com/@someone/video/1",
            "someone",
            &AtomicBool::new(false),
        )
        .await
        .expect_err("an unreadable baseline must refuse");
        assert_eq!(refusal, ArrivalRefusal::NoBaseline);
        assert_eq!(refusal.code(), "target_open_no_baseline");
        assert!(
            session.opened.lock().is_empty(),
            "no intent may be dispatched when there is no baseline to compare against"
        );
        assert!(session.taps.lock().is_empty());
        assert!(session.typed.lock().is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn an_unreadable_baseline_is_retried_once_before_it_is_refused() {
        // `read_author_label` folds a query *error* into the same `None` as a genuinely
        // absent node, so one agent hiccup used to be indistinguishable from an empty
        // screen. A single retry is the difference between losing an assignment to a
        // transient and refusing a screen that really has nothing on it.
        // The label *is* there; the first read simply does not answer. So the retry is the
        // difference between a normal arrival and losing the assignment to a transient.
        let session = ArrivalSession::new(TIKTOK, &post_page(Some("Follow Trước")))
            .follow_appears_on_attempt(2)
            .landing_on("Follow Sau");
        let arrival = open_target_by_hierarchy(
            &session,
            vietnamese(),
            TIKTOK,
            "https://www.tiktok.com/@someone/video/1",
            "someone",
            &AtomicBool::new(false),
        )
        .await
        .expect("a baseline that answers on the second read must not be refused as absent");
        assert_eq!(arrival, TargetArrival::Structural);
        assert!(
            !session.opened.lock().is_empty(),
            "the baseline was recovered, so the link should have been opened"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn tiktok_foreground_with_no_post_page_times_out_rather_than_proceeding() {
        // The right app came up and nothing else did. Waiting out the window and then
        // refusing is the only honest answer — the alternative is typing a public
        // comment into whatever is on screen.
        // The author label changes, so the intent plainly did *something* — but no action
        // rail ever appears, so there is no post page to comment on. Both halves are the
        // fixture's job: a readable baseline (or this refuses earlier, at `NoBaseline`)
        // and no `Comments` node at any point.
        let session =
            ArrivalSession::new(TIKTOK, &[(follow_key(), "Follow Trước")]).landing_on("Follow Sau");
        let refusal = open_target_by_hierarchy(
            &session,
            vietnamese(),
            TIKTOK,
            "https://www.tiktok.com/@someone/video/1",
            "someone",
            &AtomicBool::new(false),
        )
        .await
        .expect_err("no post page ever appears");
        assert_eq!(refusal, ArrivalRefusal::NoPostPage);
        assert_eq!(refusal.code(), "target_open_no_post_page");
        assert!(session.taps.lock().is_empty());
        assert!(session.typed.lock().is_empty());
        // It is *not* reported as a wrong app: TikTok really was foreground, and
        // blaming the app would send the next reader looking in the wrong place.
        assert!(!matches!(refusal, ArrivalRefusal::WrongApp { .. }));
    }

    #[test]
    fn every_catalogued_build_can_evaluate_the_post_page_predicate() {
        // `NoLabelForPostPage` is the refusal for a build whose `Comments` label was
        // never measured. No catalogued build is in that state, and this test says so
        // out loud — so if somebody adds one, they find out here rather than at the
        // moment a campaign refuses on a phone.
        for language in ["vi", "en"] {
            for package in ["com.ss.android.ugc.trill", "com.zhiliaoapp.musically"] {
                let Some(controls) = controls_for(package, language, "") else {
                    continue;
                };
                assert!(
                    controls.label(TikTokControl::Comments).is_some(),
                    "{package}/{language} cannot prove arrival at a post page"
                );
            }
        }
    }

    #[tokio::test(start_paused = true)]
    async fn a_stop_flag_ends_the_wait_without_touching_the_device() {
        let session = ArrivalSession::new(TIKTOK, &[]);
        let refusal = open_target_by_hierarchy(
            &session,
            vietnamese(),
            TIKTOK,
            "https://www.tiktok.com/@someone/video/1",
            "someone",
            &AtomicBool::new(true),
        )
        .await
        .expect_err("cancelled");
        assert_eq!(refusal, ArrivalRefusal::Cancelled);
        assert!(session.taps.lock().is_empty());
        assert!(session.typed.lock().is_empty());
    }

    /// A drawer that answers queries from a scripted screen, and records every effect.
    ///
    /// The fixture that matters most in this module: the reply flow is where a public
    /// comment lands under a stranger's post, or lands twice. Everything it does to the
    /// device is recorded so a test can assert **nothing was typed** on every refusal
    /// path.
    #[derive(Default)]
    struct DrawerSession {
        /// `locate` answers, by query value, with how many more times each may answer.
        ///
        /// A budget rather than a plain map because one case can only be expressed that
        /// way: the input field is present when the drawer opens and **gone** after a
        /// swipe closed it. A map that always answers cannot say that, and a queue that
        /// empties silently would make every other element look absent too.
        singles: Mutex<Vec<(String, ElementBox, Option<usize>)>>,
        /// `locate` answers that change once: `first` for the initial read, `rest` ever
        /// after. Consulted before `singles`.
        single_thens: Mutex<Vec<(String, Option<ElementBox>, ElementBox)>>,
        /// Successive `locate` answers for one key, with the last answer repeating.
        ///
        /// A confirmed send needs all three states: dark before typing, armed afterwards,
        /// then dark again after the tap. `single_thens` can express only two of them.
        single_queues: Mutex<Vec<(String, Vec<Option<ElementBox>>)>>,
        /// `locate_all` / `locate_all_described` answers, by query value.
        multiples: Mutex<Vec<(String, Vec<ElementBox>)>>,
        /// Replacement answers installed after the first read of a key.
        after_first: Mutex<Vec<(String, Vec<ElementBox>)>>,
        /// Successive answers for a key, one per read, last one repeating.
        queues: Mutex<Vec<(String, Vec<Vec<ElementBox>>)>>,
        /// Keys whose `locate` stops answering after a budget of ordinary reads.
        ///
        /// `0` fails from the first read. The element tables above can say "present",
        /// "absent" and "changed"; none of them can say "the agent stopped answering
        /// mid-flow", and which side of the Send tap that happens on is exactly what the
        /// transport-error tests below have to control.
        failing_after: Mutex<Vec<(String, usize)>>,
        /// Keys whose list read stops answering after a budget of ordinary reads.
        ///
        /// Separate from `failing_after` because `UiSession::locate` and
        /// `UiSession::locate_all` are different transport calls. The reply parent hunt uses
        /// the latter, and a fake that can only fail single-element reads cannot prove that a
        /// broken list request is not mistaken for an empty comment list.
        all_failing_after: Mutex<Vec<(String, usize)>>,
        /// Number of successful taps left before the transport starts failing.
        tap_failing_after: Mutex<Option<usize>>,
        /// Number of successful key-event writes left before the transport starts failing.
        keys_failing_after: Mutex<Option<usize>>,
        /// Every tap request, including ones rejected by the transport.
        tap_attempts: Mutex<Vec<TapPoint>>,
        taps: Mutex<Vec<TapPoint>>,
        typed: Mutex<Vec<String>>,
        /// What went in as **real key events**, which is a different channel from `typed`.
        keyed: Mutex<Vec<String>>,
        swipes: Mutex<usize>,
        backs: Mutex<usize>,
    }

    impl DrawerSession {
        fn with_single(self, key: &str, element: ElementBox) -> Self {
            self.singles.lock().push((key.to_string(), element, None));
            self
        }
        /// `locate(key)` answers a transport error from the very first read.
        fn with_failing(self, key: &str) -> Self {
            self.failing_after.lock().push((key.to_string(), 0));
            self
        }
        /// `locate(key)` answers normally `times` times, then errors for ever.
        fn with_failing_after(self, key: &str, times: usize) -> Self {
            self.failing_after.lock().push((key.to_string(), times));
            self
        }
        /// `locate_all(key)` answers a transport error from the first read.
        fn with_all_failing(self, key: &str) -> Self {
            self.all_failing_after.lock().push((key.to_string(), 0));
            self
        }
        /// `locate_all(key)` answers normally `times` times, then errors for ever.
        fn with_all_failing_after(self, key: &str, times: usize) -> Self {
            self.all_failing_after.lock().push((key.to_string(), times));
            self
        }
        /// `tap` answers normally `times` times, then errors for ever.
        fn with_tap_failing_after(self, times: usize) -> Self {
            *self.tap_failing_after.lock() = Some(times);
            self
        }
        /// `type_keys` answers normally `times` times, then errors for ever.
        fn with_keys_failing_after(self, times: usize) -> Self {
            *self.keys_failing_after.lock() = Some(times);
            self
        }
        /// Answers `times` times, then reports the element absent.
        fn with_single_for(self, key: &str, element: ElementBox, times: usize) -> Self {
            self.singles
                .lock()
                .push((key.to_string(), element, Some(times)));
            self
        }
        fn with_many(self, key: &str, elements: Vec<ElementBox>) -> Self {
            self.multiples.lock().push((key.to_string(), elements));
            self
        }
        /// Answers each screen in `answers` in turn, then repeats the last for ever.
        ///
        /// A mention needs **three** distinct screens to be described at all: what already
        /// claimed the handle before a key was pressed, what the picker offered, and what is
        /// left after the pick. Two-state `with_many_then` cannot say that — and a fixture
        /// that cannot say it also cannot tell a token that landed from a tap that did
        /// nothing, which is exactly the confusion the code under test exists to resolve.
        fn with_many_queue(self, key: &str, answers: Vec<Vec<ElementBox>>) -> Self {
            self.queues.lock().push((key.to_string(), answers));
            self
        }
        /// Answers `first` once, then `rest` for ever — the single-element mirror of
        /// [`Self::with_many_then`]. The drawer's pre-typing disarm check made this state
        /// mandatory to express: a real Send is dark over an empty field and lit once it
        /// holds text, and a fixture born lit is now (correctly) refused before typing.
        fn with_single_then(self, key: &str, first: ElementBox, rest: ElementBox) -> Self {
            self.single_thens
                .lock()
                .push((key.to_string(), Some(first), rest));
            self
        }
        /// Answers each state in order, then repeats the last state.
        fn with_single_queue(self, key: &str, answers: Vec<Option<ElementBox>>) -> Self {
            self.single_queues.lock().push((key.to_string(), answers));
            self
        }
        /// Answers `first` once, then `rest` for ever after.
        ///
        /// The composer check needs this: the reply placeholder is only meaningful
        /// *relative to* the one that was there before the Reply button was tapped, and a
        /// fixture that answers the same string both times cannot tell the two apart —
        /// which is the very confusion the code under test has to avoid.
        fn with_many_then(self, key: &str, first: Vec<ElementBox>, rest: Vec<ElementBox>) -> Self {
            self.multiples.lock().push((key.to_string(), first));
            self.after_first.lock().push((key.to_string(), rest));
            self
        }
    }

    #[async_trait::async_trait]
    impl UiSession for DrawerSession {
        async fn tap(&self, point: TapPoint) -> anyhow::Result<()> {
            self.tap_attempts.lock().push(point.clone());
            if let Some(remaining) = self.tap_failing_after.lock().as_mut() {
                if *remaining == 0 {
                    anyhow::bail!("transport error: tap stopped answering");
                }
                *remaining -= 1;
            }
            self.taps.lock().push(point);
            Ok(())
        }
        async fn swipe(&self, _gesture: SwipeGesture) -> anyhow::Result<()> {
            *self.swipes.lock() += 1;
            Ok(())
        }
        async fn type_text(&self, text: &str) -> anyhow::Result<()> {
            self.typed.lock().push(text.to_string());
            Ok(())
        }
        async fn type_keys(&self, text: &str) -> anyhow::Result<()> {
            if let Some(remaining) = self.keys_failing_after.lock().as_mut() {
                if *remaining == 0 {
                    anyhow::bail!("transport error: key events stopped answering");
                }
                *remaining -= 1;
            }
            self.keyed.lock().push(text.to_string());
            Ok(())
        }
        async fn home(&self) -> anyhow::Result<()> {
            Ok(())
        }
        async fn back(&self) -> anyhow::Result<()> {
            *self.backs.lock() += 1;
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
        async fn window_size(&self) -> anyhow::Result<(f64, f64)> {
            Ok((1080.0, 2400.0))
        }
        async fn locate(&self, query: ElementQuery<'_>) -> anyhow::Result<Option<ElementBox>> {
            let wanted = query_value(query);
            {
                // Checked before every table: a key can answer normally for its budget and
                // then stop answering at all, which is the difference between "absent" and
                // "the agent died" every transport-error test turns on.
                let mut failing = self.failing_after.lock();
                if let Some((_, remaining)) = failing.iter_mut().find(|(key, _)| key == wanted) {
                    if *remaining == 0 {
                        anyhow::bail!("transport error: agent stopped answering");
                    }
                    *remaining -= 1;
                }
            }
            {
                let mut queues = self.single_queues.lock();
                if let Some((_, answers)) = queues.iter_mut().find(|(key, _)| key == wanted) {
                    let answer = answers.first().cloned().unwrap_or(None);
                    if answers.len() > 1 {
                        answers.remove(0);
                    }
                    return Ok(answer);
                }
            }
            {
                let mut thens = self.single_thens.lock();
                if let Some((_, first, rest)) = thens.iter_mut().find(|(key, _, _)| key == wanted) {
                    return Ok(Some(first.take().unwrap_or_else(|| rest.clone())));
                }
            }
            let mut singles = self.singles.lock();
            let Some((_, element, remaining)) =
                singles.iter_mut().find(|(key, _, _)| key == wanted)
            else {
                return Ok(None);
            };
            match remaining {
                Some(0) => Ok(None),
                Some(left) => {
                    *left -= 1;
                    Ok(Some(element.clone()))
                }
                None => Ok(Some(element.clone())),
            }
        }
        async fn locate_all(&self, query: ElementQuery<'_>) -> anyhow::Result<Vec<ElementBox>> {
            let wanted = query_value(query);
            {
                let mut failing = self.all_failing_after.lock();
                if let Some((_, remaining)) = failing.iter_mut().find(|(key, _)| key == wanted) {
                    if *remaining == 0 {
                        anyhow::bail!("transport error: hierarchy list stopped answering");
                    }
                    *remaining -= 1;
                }
            }
            {
                let mut queues = self.queues.lock();
                if let Some((_, answers)) = queues.iter_mut().find(|(key, _)| key == wanted) {
                    let answer = answers.first().cloned().unwrap_or_default();
                    if answers.len() > 1 {
                        answers.remove(0);
                    }
                    return Ok(answer);
                }
            }
            let answer = self
                .multiples
                .lock()
                .iter()
                .find(|(key, _)| key == wanted)
                .map(|(_, elements)| elements.clone())
                .unwrap_or_default();
            // Swap in the follow-up answer, if this key has one, so the next read sees
            // the screen as it is *after* the tap.
            let mut after = self.after_first.lock();
            if let Some(index) = after.iter().position(|(key, _)| key == wanted) {
                let (key, rest) = after.remove(index);
                let mut multiples = self.multiples.lock();
                if let Some(slot) = multiples.iter_mut().find(|(existing, _)| *existing == key) {
                    slot.1 = rest;
                }
            }
            Ok(answer)
        }
    }

    fn query_value<'a>(query: ElementQuery<'a>) -> &'a str {
        match query {
            ElementQuery::Description { value, .. } => value,
            ElementQuery::ClassName(value) => value,
            ElementQuery::Text { value, .. } => value,
            // So a test can register an id suffix as a key like any other — which is how the
            // caption lookup is exercised now that it asks for `:id/desc` first.
            ElementQuery::ResourceIdSuffix(value) => value,
        }
    }

    const SEND_ID: &str = "@2131823284";
    const EDIT: &str = "android.widget.EditText";

    fn send_button(enabled: bool) -> ElementBox {
        ElementBox {
            x: 904.0,
            y: 1379.0,
            width: 143.0,
            height: 88.0,
            description: Some(SEND_ID.into()),
            enabled,
            clickable: true,
        }
    }

    fn root_mention_session() -> DrawerSession {
        DrawerSession::default()
            .with_single("bình luận", node(880.0, 900.0, 120.0, 120.0, "bình luận"))
            .with_single(EDIT, node(199.0, 1175.0, 700.0, 100.0, ""))
            .with_single_then(SEND_ID, send_button(false), send_button(true))
    }

    fn crash_stale_root_retry_session() -> DrawerSession {
        DrawerSession::default()
            .with_single("bình luận", node(880.0, 900.0, 120.0, 120.0, "bình luận"))
            .with_single(EDIT, node(199.0, 1175.0, 700.0, 100.0, ""))
            .with_single_queue(
                SEND_ID,
                vec![
                    Some(send_button(true)),
                    None,
                    Some(send_button(false)),
                    Some(send_button(true)),
                    Some(send_button(false)),
                ],
            )
            .with_single_queue(
                "Đề xuất",
                vec![None, Some(node(0.0, 0.0, 100.0, 50.0, "Đề xuất"))],
            )
    }

    #[tokio::test(start_paused = true)]
    async fn a_crash_stale_hierarchy_root_draft_is_discarded_before_the_fresh_retry() {
        let session = crash_stale_root_retry_session();
        let stop = AtomicBool::new(false);

        clear_stale_hierarchy_comment_ui(&session, vietnamese(), &stop)
            .await
            .expect("verified cleanup lets the caller reopen and re-prove the target");
        let outcome = send_root_by_hierarchy(
            &session,
            vietnamese(),
            (1080.0, 2400.0),
            "fresh root",
            &[],
            &stop,
            String::new,
        )
        .await
        .expect("fresh hierarchy root retry");

        assert!(outcome.verdict.is_sent());
        assert_eq!(session.typed.lock().as_slice(), &["fresh root"]);
        assert_eq!(
            *session.backs.lock(),
            1,
            "stale composer was discarded once"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn an_unverified_crash_stale_hierarchy_root_cleanup_is_after_effect() {
        let session = DrawerSession::default().with_single(SEND_ID, send_button(true));

        let failure =
            clear_stale_hierarchy_comment_ui(&session, vietnamese(), &AtomicBool::new(false))
                .await
                .expect_err("an armed draft that cannot be cleared is ambiguous");

        assert!(matches!(failure, HierarchySendFailure::AfterEffect(_)));
        assert!(session.typed.lock().is_empty());
        assert!(
            session.taps.lock().is_empty(),
            "no Send or fresh control was tapped"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_visible_feed_tab_does_not_verify_an_open_disarmed_hierarchy_drawer() {
        let session = DrawerSession::default()
            .with_single_queue(
                SEND_ID,
                vec![Some(send_button(true)), Some(send_button(false))],
            )
            .with_single("Đề xuất", node(0.0, 0.0, 100.0, 50.0, "Đề xuất"));

        let failure =
            clear_stale_hierarchy_comment_ui(&session, vietnamese(), &AtomicBool::new(false))
                .await
                .expect_err("the drawer must disappear, not merely expose the feed tab behind it");

        assert!(matches!(failure, HierarchySendFailure::AfterEffect(_)));
        assert!(session.typed.lock().is_empty());
        assert!(session.taps.lock().is_empty());
    }

    fn crash_stale_reply_retry_session() -> (DrawerSession, CommentLocatorIdentity) {
        let (bodies, replies, authors) = measured_rows();
        let parent = CommentLocatorIdentity {
            author_label: "Tồi nhưng tử tế".into(),
            text: "Vc cái phao câu".into(),
            locator_version: HIERARCHY_LOCATOR_VERSION.into(),
            frame_sha256: "sha".into(),
        };
        let session = DrawerSession::default()
            .with_single("bình luận", node(880.0, 900.0, 120.0, 120.0, "bình luận"))
            .with_single(EDIT, node(199.0, 1175.0, 700.0, 100.0, ""))
            .with_single_queue(
                SEND_ID,
                vec![
                    Some(send_button(true)),
                    None,
                    Some(send_button(false)),
                    Some(send_button(true)),
                    Some(send_button(false)),
                ],
            )
            .with_single_queue(
                "Đề xuất",
                vec![None, Some(node(0.0, 0.0, 100.0, 50.0, "Đề xuất"))],
            )
            .with_many("Vc cái phao câu", vec![bodies[1].clone()])
            .with_many("Trả lời", replies)
            .with_many_then(
                EDIT,
                vec![node(199.0, 2127.0, 700.0, 100.0, "Thêm bình luận...")],
                vec![node(199.0, 1175.0, 700.0, 100.0, "Trả lời Tồi nhưng tử tế")],
            )
            .with_many("android.widget.Button", authors);
        (session, parent)
    }

    #[tokio::test(start_paused = true)]
    async fn a_crash_stale_hierarchy_reply_draft_is_discarded_before_the_fresh_retry() {
        let (session, parent) = crash_stale_reply_retry_session();
        let stop = AtomicBool::new(false);

        clear_stale_hierarchy_comment_ui(&session, vietnamese(), &stop)
            .await
            .expect("verified cleanup lets the caller reopen and re-prove the parent");
        let outcome = send_reply_by_hierarchy(
            &session,
            vietnamese(),
            (1080.0, 2400.0),
            &parent,
            "fresh reply",
            &stop,
            String::new,
        )
        .await
        .expect("fresh hierarchy reply retry")
        .expect("parent is re-proved");

        assert!(outcome.verdict.is_sent());
        assert_eq!(session.typed.lock().as_slice(), &["fresh reply"]);
        assert_eq!(
            *session.backs.lock(),
            2,
            "one Back discards the stale composer and one collapses the freshly sent reply"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn an_unverified_crash_stale_hierarchy_reply_cleanup_is_after_effect() {
        let session = DrawerSession::default().with_single(SEND_ID, send_button(true));

        let failure =
            clear_stale_hierarchy_comment_ui(&session, vietnamese(), &AtomicBool::new(false))
                .await
                .expect_err("the reply path must not enter a dirty composer");

        assert!(matches!(failure, HierarchySendFailure::AfterEffect(_)));
        assert!(session.typed.lock().is_empty());
        assert!(
            session.taps.lock().is_empty(),
            "no Send or Reply control was tapped"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_denied_effect_gate_aborts_immediately_before_root_send() {
        let session = root_mention_session().with_single_queue(
            "Đề xuất",
            vec![None, Some(node(0.0, 0.0, 100.0, 50.0, "Đề xuất"))],
        );
        let calls = std::sync::atomic::AtomicUsize::new(0);
        let mut gate = crate::interaction_target::EffectGate::new(|| {
            calls.fetch_add(1, Ordering::Relaxed);
            Ok(false)
        });

        let failure = send_root_by_hierarchy_with_gate(
            &session,
            vietnamese(),
            (1080.0, 2400.0),
            "hello",
            &[],
            &AtomicBool::new(false),
            String::new,
            &mut gate,
        )
        .await
        .expect_err("lost ownership must abort the send");

        assert!(failure.ownership_lost());
        assert_eq!(calls.load(Ordering::Relaxed), 1);
        assert_eq!(session.taps.lock().len(), 2, "opener + field, never Send");
        assert_eq!(session.typed.lock().as_slice(), &["hello"]);
        assert_eq!(*session.backs.lock(), 1, "the typed composer is closed");
    }

    #[tokio::test(start_paused = true)]
    async fn an_unverified_hierarchy_gate_cleanup_is_after_effect() {
        let session = root_mention_session();
        let mut gate = crate::interaction_target::EffectGate::new(|| {
            Err(anyhow::anyhow!("effect gate database failure"))
        });

        let failure = send_root_by_hierarchy_with_gate(
            &session,
            vietnamese(),
            (1080.0, 2400.0),
            "hello",
            &[],
            &AtomicBool::new(false),
            String::new,
            &mut gate,
        )
        .await
        .expect_err("unverified cleanup cannot stay retryable");

        assert!(matches!(failure, HierarchySendFailure::AfterEffect(_)));
        assert_eq!(session.taps.lock().len(), 2, "opener + field, never Send");
        assert_eq!(*session.backs.lock(), 3, "all cleanup attempts were used");
    }

    #[tokio::test(start_paused = true)]
    async fn a_denied_effect_gate_cleans_a_typed_reply_without_sending() {
        let (bodies, replies, authors) = measured_rows();
        let parent = CommentLocatorIdentity {
            author_label: "Tồi nhưng tử tế".into(),
            text: "Vc cái phao câu".into(),
            locator_version: HIERARCHY_LOCATOR_VERSION.into(),
            frame_sha256: "sha".into(),
        };
        let session = DrawerSession::default()
            .with_single("bình luận", node(880.0, 900.0, 120.0, 120.0, "bình luận"))
            .with_single(EDIT, node(199.0, 1175.0, 700.0, 100.0, ""))
            .with_single_then(SEND_ID, send_button(false), send_button(true))
            .with_many("Vc cái phao câu", vec![bodies[1].clone()])
            .with_many("Trả lời", replies)
            .with_many_then(
                EDIT,
                vec![node(199.0, 2127.0, 700.0, 100.0, "Thêm bình luận...")],
                vec![node(199.0, 1175.0, 700.0, 100.0, "Trả lời Tồi nhưng tử tế")],
            )
            .with_many("android.widget.Button", authors)
            .with_single_queue(
                "Đề xuất",
                vec![None, Some(node(0.0, 0.0, 100.0, 50.0, "Đề xuất"))],
            );
        let mut gate = crate::interaction_target::EffectGate::new(|| Ok(false));

        let failure = send_reply_by_hierarchy_with_gate(
            &session,
            vietnamese(),
            (1080.0, 2400.0),
            &parent,
            "reply text",
            &AtomicBool::new(false),
            String::new,
            &mut gate,
        )
        .await
        .expect_err("lost ownership aborts before reply Send");

        assert!(failure.ownership_lost());
        assert_eq!(
            session.taps.lock().len(),
            3,
            "drawer + Reply + field, never Send"
        );
        assert_eq!(session.typed.lock().as_slice(), &["reply text"]);
        assert_eq!(*session.backs.lock(), 1, "the reply composer is closed");
    }

    #[test]
    fn an_effect_gate_is_one_shot_even_when_called_twice() {
        let calls = std::sync::atomic::AtomicUsize::new(0);
        let mut gate = crate::interaction_target::EffectGate::new(|| {
            calls.fetch_add(1, Ordering::Relaxed);
            Ok(true)
        });

        assert!(gate.cross().is_ok());
        assert!(gate.cross().is_err());
        assert_eq!(calls.load(Ordering::Relaxed), 1);
    }

    async fn send_root_with_one_mention(
        session: &DrawerSession,
    ) -> Result<HierarchySendOutcome, HierarchySendFailure> {
        send_root_by_hierarchy(
            session,
            vietnamese(),
            (1080.0, 2400.0),
            "hello",
            &["lt.gi".to_string()],
            &AtomicBool::new(false),
            String::new,
        )
        .await
    }

    #[tokio::test(start_paused = true)]
    async fn a_reply_never_types_when_the_composer_names_the_wrong_author() {
        // The strongest check on this path, and it is checked *before* typing. The
        // measured placeholder is `Trả lời <author>`; if the tap landed on another
        // comment's Reply button, the placeholder names that other author. Typing anyway
        // posts the campaign's reply under a stranger's comment, publicly, under the
        // operator's account.
        let (bodies, replies, authors) = measured_rows();
        let parent = CommentLocatorIdentity {
            author_label: "Tồi nhưng tử tế".into(),
            text: "Vc cái phao câu".into(),
            locator_version: HIERARCHY_LOCATOR_VERSION.into(),
            frame_sha256: "sha".into(),
        };
        let session = DrawerSession::default()
            .with_single("bình luận", node(880.0, 900.0, 120.0, 120.0, "bình luận"))
            .with_single(EDIT, node(199.0, 1175.0, 700.0, 100.0, ""))
            .with_single(SEND_ID, send_button(false))
            .with_many("Vc cái phao câu", vec![bodies[1].clone()])
            .with_many("Trả lời", replies.clone())
            // Before the tap the field carries the root drawer's own hint; after it, the
            // composer's — naming `Ghét tháng 9.`, who is *not* the parent's author.
            .with_many_then(
                EDIT,
                vec![node(199.0, 2127.0, 700.0, 100.0, "Thêm bình luận...")],
                vec![node(199.0, 1175.0, 700.0, 100.0, "Trả lời Ghét tháng 9.")],
            )
            .with_many("android.widget.Button", authors.clone());

        let refusal = send_reply_by_hierarchy(
            &session,
            vietnamese(),
            (1080.0, 2400.0),
            &parent,
            "reply text",
            &AtomicBool::new(false),
            || "frame".to_string(),
        )
        .await
        .expect("no transport error")
        .expect_err("the composer names somebody else");
        assert!(matches!(refusal, ReplyRefusal::WrongParentComposer { .. }));
        assert_eq!(refusal.code(), "reply_wrong_parent");
        assert!(
            session.typed.lock().is_empty(),
            "nothing may be typed once the parent is in doubt"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_root_drawer_placeholder_is_not_mistaken_for_the_reply_composer() {
        // Regression test for a real bug this code had. The wait used to be "any EditText
        // whose text is non-empty" — but the measured field already reads
        // `Thêm bình luận...` before the Reply button is tapped, so that condition was
        // *already true* and the wait returned instantly with the root drawer's own hint.
        // The author check then ran against the wrong string, and its verdict said nothing
        // about which Reply button had been hit.
        //
        // Here the placeholder never changes, which is what a tap that opened no composer
        // looks like. The right answer is `NoComposer` and nothing typed.
        let (bodies, replies, authors) = measured_rows();
        let parent = CommentLocatorIdentity {
            author_label: "Tồi nhưng tử tế".into(),
            text: "Vc cái phao câu".into(),
            locator_version: HIERARCHY_LOCATOR_VERSION.into(),
            frame_sha256: "sha".into(),
        };
        let session = DrawerSession::default()
            .with_single("bình luận", node(880.0, 900.0, 120.0, 120.0, "bình luận"))
            .with_single(EDIT, node(199.0, 2127.0, 700.0, 100.0, ""))
            .with_single(SEND_ID, send_button(false))
            .with_many("Vc cái phao câu", vec![bodies[1].clone()])
            .with_many("Trả lời", replies)
            // Same non-empty hint every read: the composer never appeared.
            .with_many(
                EDIT,
                vec![node(199.0, 2127.0, 700.0, 100.0, "Thêm bình luận...")],
            )
            .with_many("android.widget.Button", authors);

        let refusal = send_reply_by_hierarchy(
            &session,
            vietnamese(),
            (1080.0, 2400.0),
            &parent,
            "reply text",
            &AtomicBool::new(false),
            || "frame".to_string(),
        )
        .await
        .expect("no transport error")
        .expect_err("no composer opened");
        assert_eq!(refusal, ReplyRefusal::NoComposer);
        assert!(
            session.typed.lock().is_empty(),
            "an unchanged placeholder proves nothing about which Reply button was hit"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_reply_whose_parent_is_not_on_screen_refuses_without_typing() {
        // The body is simply absent, and no amount of scrolling finds it because the
        // anchors never move — the end of the list.
        let (_, replies, authors) = measured_rows();
        let parent = CommentLocatorIdentity {
            author_label: "Tồi nhưng tử tế".into(),
            text: "a comment nobody posted".into(),
            locator_version: HIERARCHY_LOCATOR_VERSION.into(),
            frame_sha256: "sha".into(),
        };
        let session = DrawerSession::default()
            .with_single("bình luận", node(880.0, 900.0, 120.0, 120.0, "bình luận"))
            .with_single(EDIT, node(199.0, 1175.0, 700.0, 100.0, ""))
            .with_single(SEND_ID, send_button(false))
            .with_many("Trả lời", replies)
            .with_many("android.widget.Button", authors);

        let refusal = send_reply_by_hierarchy(
            &session,
            vietnamese(),
            (1080.0, 2400.0),
            &parent,
            "reply text",
            &AtomicBool::new(false),
            || "frame".to_string(),
        )
        .await
        .expect("no transport error")
        .expect_err("the parent is not there");
        assert!(matches!(refusal, ReplyRefusal::ParentNotFound { .. }));
        assert!(session.typed.lock().is_empty());
        // It stopped at the first unmoved list rather than burning the whole budget.
        assert_eq!(*session.swipes.lock(), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn a_swipe_that_closes_the_drawer_is_reported_as_that_and_not_as_a_missing_parent() {
        // Distinguishing these two matters: "the drawer closed" is a gesture bug worth
        // fixing, "the parent is not here" is ordinary. The pixel path checks for exactly
        // this, and the check is that the input field is gone afterwards.
        let (_, replies, authors) = measured_rows();
        let parent = CommentLocatorIdentity {
            author_label: "Tồi nhưng tử tế".into(),
            text: "not on this screen".into(),
            locator_version: HIERARCHY_LOCATOR_VERSION.into(),
            frame_sha256: "sha".into(),
        };
        // The field answers exactly once — for `open` — and is gone by the time the
        // post-swipe check looks for it. That is precisely "the swipe closed the
        // drawer".
        let session = DrawerSession::default()
            .with_single("bình luận", node(880.0, 900.0, 120.0, 120.0, "bình luận"))
            .with_single(SEND_ID, send_button(false))
            .with_single_for(EDIT, node(199.0, 1175.0, 700.0, 100.0, ""), 1)
            .with_many("Trả lời", replies)
            .with_many("android.widget.Button", authors);

        let refusal = send_reply_by_hierarchy(
            &session,
            vietnamese(),
            (1080.0, 2400.0),
            &parent,
            "reply text",
            &AtomicBool::new(false),
            || "frame".to_string(),
        )
        .await
        .expect("no transport error")
        .expect_err("the drawer closed");
        assert_eq!(refusal, ReplyRefusal::DrawerClosedByScroll);
        assert_eq!(refusal.code(), "reply_drawer_closed_by_scroll");
        assert!(session.typed.lock().is_empty());
        // It swiped once and then noticed, rather than swiping four more times into a
        // screen that is no longer the comment list.
        assert_eq!(*session.swipes.lock(), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn an_unmeasured_reply_control_refuses_before_the_drawer_is_opened() {
        // Opening the drawer first would leave the phone inside it with nothing to aim at.
        //
        // Against a set with **nothing** measured rather than a real catalogue entry that
        // happened to lack the control. It used to name `musically/en`, and when that
        // build's Reply button was measured on 18/08/2026 this test quietly stopped testing
        // the thing it is named for: it passed, on a set that could reply perfectly well.
        let english = crate::tiktok_labels::nothing_measured();
        let session = DrawerSession::default();
        let parent = CommentLocatorIdentity {
            author_label: "someone".into(),
            text: "parent".into(),
            locator_version: HIERARCHY_LOCATOR_VERSION.into(),
            frame_sha256: "sha".into(),
        };
        let refusal = send_reply_by_hierarchy(
            &session,
            english,
            (1080.0, 2400.0),
            &parent,
            "reply",
            &AtomicBool::new(false),
            String::new,
        )
        .await
        .expect("no transport error")
        .expect_err("unmeasured");
        assert_eq!(refusal, ReplyRefusal::ReplyUnmeasured);
        assert!(session.taps.lock().is_empty(), "nothing may be tapped");
    }

    #[tokio::test(start_paused = true)]
    async fn a_root_send_that_cannot_confirm_the_disarm_is_not_reported_as_sent() {
        // `NotConfirmed` is the verdict that must never be retried: the Send tap went
        // out, so a retry is how a post ends up with two identical comments. The frames
        // are still returned, because they are the only way a person settles it.
        let session = DrawerSession::default()
            .with_single("bình luận", node(880.0, 900.0, 120.0, 120.0, "bình luận"))
            .with_single(EDIT, node(199.0, 1175.0, 700.0, 100.0, ""))
            // Dark before typing — the way a real empty field reads, and what the drawer's
            // pre-type check now demands — then armed forever: tapped, never disarmed,
            // never vanished.
            .with_single_then(SEND_ID, send_button(false), send_button(true));

        let mut shas = vec!["armed".to_string(), "cleared".to_string()].into_iter();
        let outcome = send_root_by_hierarchy(
            &session,
            vietnamese(),
            (1080.0, 2400.0),
            "hello",
            &[],
            &AtomicBool::new(false),
            || shas.next().unwrap_or_default(),
        )
        .await
        .expect("no transport error");
        assert_eq!(
            outcome.verdict,
            crate::tiktok_drawer::CommentVerdict::NotConfirmed
        );
        assert!(!outcome.verdict.is_sent());
        assert_eq!(outcome.armed_frame_sha256, "armed");
        assert_eq!(outcome.cleared_frame_sha256, "cleared");
        assert_eq!(
            outcome.identity, None,
            "an unconfirmed send must not claim an identity"
        );
    }

    /// **A transport error before the Send tap is a retryable verdict, never an `Err`.**
    ///
    /// The public error boundary is phase-aware, and known pre-Send interruptions use this
    /// named verdict so they share the ordinary hierarchy finish path. An agent that stops
    /// answering while the drawer is still opening provably posted nothing, so this result
    /// must remain retryable.
    #[tokio::test(start_paused = true)]
    async fn a_transport_error_before_the_send_tap_is_a_retryable_verdict() {
        let session = DrawerSession::default().with_failing("bình luận");
        let outcome = send_root_by_hierarchy(
            &session,
            vietnamese(),
            (1080.0, 2400.0),
            "hello",
            &[],
            &AtomicBool::new(false),
            String::new,
        )
        .await
        .expect("a pre-tap transport error must come back as a verdict, not an Err");
        assert_eq!(
            outcome.verdict,
            crate::tiktok_drawer::CommentVerdict::SendFlowInterrupted
        );
        assert!(!outcome.verdict.is_sent());
        assert!(session.typed.lock().is_empty(), "nothing was ever typed");
        assert!(session.taps.lock().is_empty(), "nothing was ever tapped");
    }

    #[tokio::test(start_paused = true)]
    async fn root_mention_transport_type_keys_failure_stops_before_send() {
        let session = root_mention_session().with_keys_failing_after(0);

        let outcome = send_root_with_one_mention(&session)
            .await
            .expect("a pre-Send transport failure is a retryable outcome");

        assert_eq!(
            outcome.verdict,
            crate::tiktok_drawer::CommentVerdict::SendFlowInterrupted
        );
        assert_eq!(
            session.tap_attempts.lock().len(),
            2,
            "only the drawer opener and composer focus may be attempted"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn root_mention_transport_initial_list_failure_stops_before_send() {
        let session = root_mention_session().with_all_failing("android.widget.TextView");

        let outcome = send_root_with_one_mention(&session)
            .await
            .expect("a pre-Send transport failure is a retryable outcome");

        assert_eq!(
            outcome.verdict,
            crate::tiktok_drawer::CommentVerdict::SendFlowInterrupted
        );
        assert_eq!(session.tap_attempts.lock().len(), 2, "Send was untouched");
        assert!(
            session.keyed.lock().is_empty(),
            "the picker was not typed into"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn root_mention_transport_waiting_list_failure_stops_before_send() {
        let session = root_mention_session().with_all_failing_after("android.widget.TextView", 1);

        let outcome = send_root_with_one_mention(&session)
            .await
            .expect("a pre-Send transport failure is a retryable outcome");

        assert_eq!(
            outcome.verdict,
            crate::tiktok_drawer::CommentVerdict::SendFlowInterrupted
        );
        assert_eq!(session.tap_attempts.lock().len(), 2, "Send was untouched");
        assert_eq!(session.keyed.lock().as_slice(), [" @lt.gi"]);
    }

    #[tokio::test(start_paused = true)]
    async fn root_mention_transport_suggestion_tap_failure_stops_before_send() {
        let session = root_mention_session()
            .with_many_queue(
                "android.widget.TextView",
                vec![Vec::new(), vec![node(179.0, 291.0, 223.0, 50.0, "lt.gi")]],
            )
            .with_tap_failing_after(2);

        let outcome = send_root_with_one_mention(&session)
            .await
            .expect("a pre-Send transport failure is a retryable outcome");

        assert_eq!(
            outcome.verdict,
            crate::tiktok_drawer::CommentVerdict::SendFlowInterrupted
        );
        assert_eq!(
            session.tap_attempts.lock().len(),
            3,
            "the failed suggestion tap must be the final tap attempt"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn root_mention_transport_composer_read_failure_stops_before_send() {
        let session = root_mention_session()
            .with_many_queue(
                "android.widget.TextView",
                vec![Vec::new(), vec![node(179.0, 291.0, 223.0, 50.0, "lt.gi")]],
            )
            .with_all_failing(EDIT);

        let outcome = send_root_with_one_mention(&session)
            .await
            .expect("a pre-Send transport failure is a retryable outcome");

        assert_eq!(
            outcome.verdict,
            crate::tiktok_drawer::CommentVerdict::SendFlowInterrupted
        );
        assert_eq!(
            session.tap_attempts.lock().len(),
            3,
            "the suggestion tap may occur, but Send must not"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn root_mention_transport_post_pick_list_failure_stops_before_send() {
        let session = root_mention_session()
            .with_many_queue(
                "android.widget.TextView",
                vec![Vec::new(), vec![node(179.0, 291.0, 223.0, 50.0, "lt.gi")]],
            )
            .with_all_failing_after("android.widget.TextView", 2)
            .with_many(EDIT, vec![node(64.0, 1379.0, 800.0, 88.0, "hello @lt.gi ")]);

        let outcome = send_root_with_one_mention(&session)
            .await
            .expect("a pre-Send transport failure is a retryable outcome");

        assert_eq!(
            outcome.verdict,
            crate::tiktok_drawer::CommentVerdict::SendFlowInterrupted
        );
        assert_eq!(
            session.tap_attempts.lock().len(),
            3,
            "the suggestion tap may occur, but Send must not"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn root_mention_transport_final_composer_read_failure_stops_before_send() {
        let session = root_mention_session()
            .with_many_queue(
                "android.widget.TextView",
                vec![
                    Vec::new(),
                    vec![node(179.0, 291.0, 223.0, 50.0, "lt.gi")],
                    Vec::new(),
                ],
            )
            .with_many(EDIT, vec![node(64.0, 1379.0, 800.0, 88.0, "hello @lt.gi ")])
            .with_all_failing_after(EDIT, 1);

        let outcome = send_root_with_one_mention(&session)
            .await
            .expect("a pre-Send transport failure is a retryable outcome");

        assert_eq!(
            outcome.verdict,
            crate::tiktok_drawer::CommentVerdict::SendFlowInterrupted
        );
        assert_eq!(
            session.tap_attempts.lock().len(),
            3,
            "the root's final composer read failed before Send"
        );
    }

    /// **A transport error after the Send tap stays an `Err` — the one ambiguous case.**
    ///
    /// The tap went out and the disarm read died, so whether the comment posted is genuinely
    /// unknown. That direction must keep travelling the `SendFailure::after` channel: settling
    /// it retryable is how a post ends up with two identical comments from one account.
    #[tokio::test(start_paused = true)]
    async fn a_transport_error_after_the_send_tap_stays_the_ambiguous_error() {
        let session = DrawerSession::default()
            .with_single("bình luận", node(880.0, 900.0, 120.0, 120.0, "bình luận"))
            .with_single(EDIT, node(199.0, 1175.0, 700.0, 100.0, ""))
            // Dark for the pre-type check, armed for the arm wait — and then the agent
            // dies exactly on the disarm read, the first read after the tap.
            .with_single_then(SEND_ID, send_button(false), send_button(true))
            .with_failing_after(SEND_ID, 2);

        let result = send_root_by_hierarchy(
            &session,
            vietnamese(),
            (1080.0, 2400.0),
            "hello",
            &[],
            &AtomicBool::new(false),
            String::new,
        )
        .await;
        assert!(
            result.is_err(),
            "an error at or after the tap must stay an Err for the after-effect channel"
        );
        assert!(
            !session.typed.lock().is_empty(),
            "the text was typed before the line went dead — this is the post-type case"
        );
    }

    /// A failed hierarchy list read is not an empty comment list.
    ///
    /// The parent lookup used to collapse `locate_all` errors to `None`, then spend the scroll
    /// budget and report `reply_parent_not_found`. That happened before Send and was retryable,
    /// but it lied about why: the agent had stopped answering, so no observation established
    /// that the parent was absent. Preserve the transport classification all the way out.
    #[tokio::test(start_paused = true)]
    async fn a_reply_list_transport_error_before_send_is_an_interrupted_verdict() {
        let parent = CommentLocatorIdentity {
            author_label: "Tồi nhưng tử tế".into(),
            text: "Vc cái phao câu".into(),
            locator_version: HIERARCHY_LOCATOR_VERSION.into(),
            frame_sha256: "sha".into(),
        };
        let session = DrawerSession::default()
            .with_single("bình luận", node(880.0, 900.0, 120.0, 120.0, "bình luận"))
            .with_single(EDIT, node(199.0, 1175.0, 700.0, 100.0, ""))
            .with_single(SEND_ID, send_button(false))
            .with_all_failing(&parent.text);

        let outcome = send_reply_by_hierarchy(
            &session,
            vietnamese(),
            (1080.0, 2400.0),
            &parent,
            "reply text",
            &AtomicBool::new(false),
            String::new,
        )
        .await
        .expect("the transport phase must be represented by the outcome")
        .expect("a pre-Send transport failure is a verdict, not a parent refusal");

        assert_eq!(
            outcome.verdict,
            crate::tiktok_drawer::CommentVerdict::SendFlowInterrupted
        );
        assert!(session.typed.lock().is_empty(), "nothing was typed");
        assert_eq!(
            session.taps.lock().len(),
            1,
            "only the drawer opener may have been tapped; Send was untouched"
        );
    }

    /// The same reply path must switch classification exactly at the Send tap.
    #[tokio::test(start_paused = true)]
    async fn a_reply_transport_error_after_send_stays_the_ambiguous_error() {
        let (bodies, replies, authors) = measured_rows();
        let parent = CommentLocatorIdentity {
            author_label: "Tồi nhưng tử tế".into(),
            text: "Vc cái phao câu".into(),
            locator_version: HIERARCHY_LOCATOR_VERSION.into(),
            frame_sha256: "sha".into(),
        };
        let session = DrawerSession::default()
            .with_single("bình luận", node(880.0, 900.0, 120.0, 120.0, "bình luận"))
            .with_single(EDIT, node(199.0, 1175.0, 700.0, 100.0, ""))
            .with_single_then(SEND_ID, send_button(false), send_button(true))
            .with_failing_after(SEND_ID, 2)
            .with_many(&parent.text, vec![bodies[1].clone()])
            .with_many("Trả lời", replies)
            .with_many_then(
                EDIT,
                vec![node(199.0, 2127.0, 700.0, 100.0, "Thêm bình luận...")],
                vec![node(199.0, 1175.0, 700.0, 100.0, "Trả lời Tồi nhưng tử tế")],
            )
            .with_many("android.widget.Button", authors);

        let result = send_reply_by_hierarchy(
            &session,
            vietnamese(),
            (1080.0, 2400.0),
            &parent,
            "reply text",
            &AtomicBool::new(false),
            String::new,
        )
        .await;

        assert!(
            result.is_err(),
            "a transport error after Send must remain the ambiguous outer error"
        );
        assert_eq!(session.typed.lock().as_slice(), ["reply text"]);
        assert_eq!(
            session.taps.lock().len(),
            4,
            "drawer, Reply, composer and Send must all have been tapped"
        );
    }

    /// Expanding the folded section is not merely a search detail: a confirmed reply under
    /// that parent is invisible to everyone except the posting account, so the outcome must
    /// retain the fact all the way to the campaign evidence.
    #[tokio::test(start_paused = true)]
    async fn a_reply_found_after_expanding_folded_comments_marks_the_sent_outcome() {
        let (bodies, replies, authors) = measured_rows();
        let parent = CommentLocatorIdentity {
            author_label: "Tồi nhưng tử tế".into(),
            text: "Vc cái phao câu".into(),
            locator_version: HIERARCHY_LOCATOR_VERSION.into(),
            frame_sha256: "sha".into(),
        };
        let controls = controls_for(TIKTOK, "en", "38.3.2").expect("measured English set");
        let send_key = controls
            .label(TikTokControl::CommentSend)
            .expect("English build measures Send")
            .value();
        let session = DrawerSession::default()
            .with_single("comments", node(880.0, 900.0, 120.0, 120.0, "comments"))
            .with_single(EDIT, node(199.0, 1175.0, 700.0, 100.0, ""))
            .with_single(
                "View folded comments",
                node(180.0, 1870.0, 620.0, 72.0, "View folded comments"),
            )
            .with_single_queue(
                send_key,
                vec![
                    Some(send_button(false)),
                    Some(send_button(true)),
                    Some(send_button(false)),
                ],
            )
            // The first parent lookup sees only the open list. After its stationary scroll
            // triggers the folded control, the same lookup sees the parent's real row.
            .with_many_then(&parent.text, Vec::new(), vec![bodies[1].clone()])
            .with_many("Reply", replies)
            .with_many("android.widget.Button", authors)
            .with_many(
                "android.widget.TextView",
                vec![node(140.0, 300.0, 600.0, 60.0, "an open-list row")],
            )
            .with_many_then(
                EDIT,
                vec![node(199.0, 2127.0, 700.0, 100.0, "Add comment...")],
                vec![node(
                    199.0,
                    1175.0,
                    700.0,
                    100.0,
                    "Reply to Tồi nhưng tử tế",
                )],
            );

        let outcome = send_reply_by_hierarchy(
            &session,
            controls,
            (1080.0, 2400.0),
            &parent,
            "reply text",
            &AtomicBool::new(false),
            String::new,
        )
        .await
        .expect("no transport error")
        .expect("the revealed parent is found");

        assert_eq!(outcome.verdict, crate::tiktok_drawer::CommentVerdict::Sent);
        assert!(
            outcome.parent_was_folded,
            "the successful reply must retain that its parent was folded"
        );
        assert_eq!(session.typed.lock().as_slice(), ["reply text"]);
        assert_eq!(*session.swipes.lock(), 1, "the open list was tested once");
    }

    #[tokio::test(start_paused = true)]
    async fn a_root_send_refuses_before_opening_the_drawer_when_send_is_unmeasured() {
        let english = controls_for("com.zhiliaoapp.musically", "en", "").expect("set");
        let session = DrawerSession::default();
        let outcome = send_root_by_hierarchy(
            &session,
            english,
            (1080.0, 2400.0),
            "hello",
            &[],
            &AtomicBool::new(false),
            String::new,
        )
        .await
        .expect("no transport error");
        assert_eq!(
            outcome.verdict,
            crate::tiktok_drawer::CommentVerdict::SendUnmeasured
        );
        assert!(session.taps.lock().is_empty());
        assert!(session.typed.lock().is_empty());
    }

    /// A node at a measured position. `label` goes in `description` because that is
    /// where `locate_all`'s caller puts whichever attribute it matched on.
    /// **The video's right rail is in the same tree as the comment rows.**
    ///
    /// Measured 25/08/2026, `com.ss.android.ugc.trill` en-US, drawer open over the post:
    /// authors and bodies both start at `x=178`, while the rail's counters sit at `x≈966-978`
    /// as bare-number buttons — `28` likes, `3` comments, `14` shares.
    ///
    /// The break needs the row's own author to be out of the list — scrolled past the drawer's
    /// clip, which is ordinary — because an author that *is* there sits 6 px above its body and
    /// wins on distance. With it gone, the old filter's left-edge tolerance of `body.width`
    /// (866 px there) reached all the way to the rail and took a counter instead. It did, on a
    /// live run: the reply refused itself with `ô reply ghi "Replying to Phùng Kiên", không
    /// chứa tên tác giả cha "28"`. The refusal was right; the read that produced `"28"` was not.
    ///
    /// Refusing to find a parent is the correct answer here. A reply that cannot name who it
    /// answers must not be typed.
    #[test]
    fn a_counter_from_the_videos_rail_is_not_a_comments_author() {
        let body = node(
            178.0,
            1200.0,
            866.0,
            126.0,
            "Giới trẻ giờ mê chill giữa thiên nhiên",
        );
        // `28` at x=966: within the old `body.width` tolerance, far outside a left edge.
        let rail_count = node(966.0, 1122.0, 47.0, 42.0, "28");
        let reply = node(333.0, 1338.0, 104.0, 48.0, "Reply");

        assert!(
            locate_parent_in_elements(
                &[body],
                &[reply],
                &[rail_count],
                &CommentLocatorIdentity {
                    author_label: "Xuân".into(),
                    text: "Giới trẻ giờ mê chill giữa thiên nhiên".into(),
                    locator_version: "v1".into(),
                    frame_sha256: "sha".into(),
                },
            )
            .is_none(),
            "with no author in reach, the answer is 'not found', never the rail's like count"
        );
    }

    /// **And a rail control that is not a number is caught by the left edge alone.**
    ///
    /// `is_count_label` cannot see this one: `Add or remove this video from Favorites` is a
    /// real button from the same dump, at `x=899`, with a perfectly name-like label. Only the
    /// left edge separates it from the comment column at `x=178`. This is the case that says
    /// the tightened tolerance is load-bearing rather than a second opinion.
    #[test]
    fn a_named_rail_control_is_not_a_comments_author_either() {
        let body = node(178.0, 1200.0, 866.0, 126.0, "câu vừa gửi");
        let rail_button = node(
            899.0,
            1122.0,
            181.0,
            42.0,
            "Add or remove this video from Favorites.",
        );

        assert!(
            discover_identity_in_elements(&[body], &[rail_button], "câu vừa gửi", "sha", "v1")
                .is_none(),
            "a control 721 px to the right of the comment column is not that comment's author"
        );
    }

    /// The row's real author still wins when it is there — the tightening must not cost that.
    #[test]
    fn the_real_author_is_still_found_next_to_its_body() {
        let author = node(178.0, 1146.0, 95.0, 48.0, "Xuân");
        let body = node(
            178.0,
            1200.0,
            866.0,
            126.0,
            "Giới trẻ giờ mê chill giữa thiên nhiên",
        );
        let rail_count = node(966.0, 1122.0, 47.0, 42.0, "28");
        let reply = node(333.0, 1338.0, 104.0, 48.0, "Reply");

        let found = locate_parent_in_elements(
            &[body],
            &[reply],
            &[rail_count, author],
            &CommentLocatorIdentity {
                author_label: "Xuân".into(),
                text: "Giới trẻ giờ mê chill giữa thiên nhiên".into(),
                locator_version: "v1".into(),
                frame_sha256: "sha".into(),
            },
        )
        .expect("the row is there to be found");
        assert_eq!(found.identity.author_label, "Xuân");
    }

    /// The same rail, on the path that *captures* an identity after sending.
    ///
    /// This is where the bad read originally happened: an identity stored with `"28"` as its
    /// author makes every later reply to it refuse, which is what the live run did.
    #[test]
    fn discovering_an_identity_also_ignores_the_rail() {
        let body = node(178.0, 1200.0, 866.0, 126.0, "câu vừa gửi");
        let rail_count = node(966.0, 1122.0, 47.0, 42.0, "28");

        assert!(
            discover_identity_in_elements(&[body], &[rail_count], "câu vừa gửi", "sha", "v1")
                .is_none(),
            "an identity whose author is the rail's counter poisons every reply that follows"
        );
    }

    fn node(x: f64, y: f64, width: f64, height: f64, label: &str) -> ElementBox {
        ElementBox {
            x,
            y,
            width,
            height,
            description: Some(label.to_string()),
            enabled: true,
            clickable: true,
        }
    }

    /// The measured Redmi layout: three rows at the real coordinates.
    ///
    /// Bodies at x=174, reply buttons at x=315..419, row pitch 300 px — read off
    /// `target/drawer-opened.xml` on 11/08/2026.
    fn measured_rows() -> (Vec<ElementBox>, Vec<ElementBox>, Vec<ElementBox>) {
        let bodies = vec![
            node(174.0, 1077.0, 700.0, 44.0, "first comment"),
            node(174.0, 1377.0, 700.0, 44.0, "Vc cái phao câu"),
            node(174.0, 1677.0, 700.0, 44.0, "third comment"),
        ];
        let replies = vec![
            node(315.0, 1149.0, 104.0, 44.0, "Trả lời"),
            node(315.0, 1449.0, 104.0, 44.0, "Trả lời"),
            node(315.0, 1749.0, 104.0, 44.0, "Trả lời"),
        ];
        let authors = vec![
            node(174.0, 1027.0, 300.0, 40.0, "author one"),
            node(174.0, 1327.0, 300.0, 40.0, "Tồi nhưng tử tế"),
            node(174.0, 1627.0, 300.0, 40.0, "author three"),
        ];
        (bodies, replies, authors)
    }

    fn identity(text: &str) -> CommentLocatorIdentity {
        CommentLocatorIdentity {
            author_label: String::new(),
            text: text.to_string(),
            locator_version: "android-hierarchy-v1".into(),
            frame_sha256: "f".repeat(64),
        }
    }

    #[test]
    fn the_middle_row_resolves_to_its_own_author_and_its_own_reply_button() {
        let (bodies, replies, authors) = measured_rows();
        let found =
            locate_parent_in_elements(&bodies, &replies, &authors, &identity("Vc cái phao câu"))
                .expect("the measured middle row");
        assert_eq!(found.identity.author_label, "Tồi nhưng tử tế");
        assert_eq!(found.reply.y, 1449.0);
        // The version and frame stamp travel through untouched — they say which
        // reader produced the evidence.
        assert_eq!(found.identity.locator_version, "android-hierarchy-v1");
    }

    #[test]
    fn the_nearest_reply_below_wins_even_when_the_tree_lists_another_first() {
        // Tree order is not screen order. Listing the third row's control first is
        // the exact shape that made the pixel path post a reply under a stranger's
        // comment, and it has its own regression test there.
        let (bodies, mut replies, authors) = measured_rows();
        replies.reverse();
        let found =
            locate_parent_in_elements(&bodies, &replies, &authors, &identity("first comment"))
                .expect("row one");
        assert_eq!(found.reply.y, 1149.0, "took a reply from another row");
    }

    #[test]
    fn a_reply_button_belonging_to_the_next_row_is_out_of_reach() {
        // Only the third row's control is present, 600 px below the first body.
        let (bodies, _, authors) = measured_rows();
        let replies = vec![node(315.0, 1749.0, 104.0, 44.0, "Trả lời")];
        assert!(
            locate_parent_in_elements(&bodies, &replies, &authors, &identity("first comment"))
                .is_none(),
            "a distant row's reply control must not be adopted"
        );
    }

    #[test]
    fn a_duplicated_body_is_refused_rather_than_guessed() {
        let (mut bodies, replies, authors) = measured_rows();
        bodies.push(node(174.0, 1977.0, 700.0, 44.0, "first comment"));
        assert!(
            locate_parent_in_elements(&bodies, &replies, &authors, &identity("first comment"))
                .is_none(),
            "two rows reading the same thing give no way to choose"
        );
    }

    #[test]
    fn a_body_is_never_its_own_author() {
        // The author search must look strictly above. Without the bound the body
        // itself satisfies "a non-empty label near the body".
        let bodies = vec![node(174.0, 1377.0, 700.0, 44.0, "solo")];
        let replies = vec![node(315.0, 1449.0, 104.0, 44.0, "Trả lời")];
        assert!(
            locate_parent_in_elements(&bodies, &replies, &bodies, &identity("solo")).is_none(),
            "with no label above it, the row has no author"
        );
    }

    #[test]
    fn a_body_that_merely_contains_the_wanted_text_does_not_match() {
        // The match is exact on a string we typed ourselves. A row quoting our
        // comment inside a longer one is somebody else's row.
        let bodies = vec![node(174.0, 1077.0, 700.0, 44.0, "hay qua that day")];
        let replies = vec![node(315.0, 1149.0, 104.0, 44.0, "Trả lời")];
        let authors = vec![node(174.0, 1027.0, 300.0, 40.0, "someone")];
        assert!(
            locate_parent_in_elements(&bodies, &replies, &authors, &identity("hay qua")).is_none()
        );
    }

    #[test]
    fn an_indented_label_is_not_taken_as_the_author() {
        // A nested reply's author sits further right. Accepting it would name the
        // wrong person in stored evidence, and the next device in the chain would
        // then fail to match the row.
        let bodies = vec![node(174.0, 1377.0, 700.0, 44.0, "row")];
        let replies = vec![node(315.0, 1449.0, 104.0, 44.0, "Trả lời")];
        let authors = vec![node(1400.0, 1327.0, 300.0, 40.0, "far right")];
        assert!(locate_parent_in_elements(&bodies, &replies, &authors, &identity("row")).is_none());
    }

    #[test]
    fn whitespace_around_the_stored_text_does_not_break_the_match() {
        let (bodies, replies, authors) = measured_rows();
        let padded = identity("  Vc cái phao câu  ");
        let found = locate_parent_in_elements(&bodies, &replies, &authors, &padded)
            .expect("trimmed on both sides");
        assert_eq!(found.identity.text, "Vc cái phao câu");
    }

    #[test]
    fn an_empty_wanted_text_refuses_instead_of_matching_a_blank_row() {
        let (bodies, replies, authors) = measured_rows();
        assert!(locate_parent_in_elements(&bodies, &replies, &authors, &identity("   ")).is_none());
    }

    #[test]
    fn reading_back_a_posted_comment_needs_no_reply_control() {
        let (bodies, _, authors) = measured_rows();
        let found = discover_identity_in_elements(
            &bodies,
            &authors,
            "Vc cái phao câu",
            &"a".repeat(64),
            "android-hierarchy-v1",
        )
        .expect("identity");
        assert_eq!(found.author_label, "Tồi nhưng tử tế");
        assert_eq!(found.locator_version, "android-hierarchy-v1");
        assert_eq!(found.frame_sha256, "a".repeat(64));
    }

    #[test]
    fn reading_back_a_duplicated_comment_is_refused() {
        let (mut bodies, _, authors) = measured_rows();
        bodies.push(node(174.0, 1977.0, 700.0, 44.0, "Vc cái phao câu"));
        assert!(discover_identity_in_elements(
            &bodies,
            &authors,
            "Vc cái phao câu",
            &"a".repeat(64),
            "android-hierarchy-v1",
        )
        .is_none());
    }

    #[tokio::test(start_paused = true)]
    async fn a_list_that_scrolled_is_not_read_as_the_end_just_because_the_rows_line_up() {
        // The cheap anchor is the y of the reply controls, and comment rows are close to
        // evenly spaced — so a scroll that advances about a whole number of rows leaves those
        // controls on the same pixels and the list looks stopped when it plainly is not.
        //
        // Measured 19/08/2026: a reply refused with `reply_parent_not_found` after **two**
        // scrolls on a list of eighteen comments, having decided it had reached the end. The
        // text of the rows cannot alias that way, so it gets the final word — and only here,
        // where the alternative is refusing a reply whose parent is further down.
        let (_, replies, authors) = measured_rows();
        let parent = CommentLocatorIdentity {
            author_label: "Tồi nhưng tử tế".into(),
            text: "a comment nobody posted".into(),
            locator_version: HIERARCHY_LOCATOR_VERSION.into(),
            frame_sha256: "sha".into(),
        };
        let session = DrawerSession::default()
            .with_single("bình luận", node(880.0, 900.0, 120.0, 120.0, "bình luận"))
            .with_single(EDIT, node(199.0, 1175.0, 700.0, 100.0, ""))
            .with_single(SEND_ID, send_button(false))
            // The reply controls never move: this is the aliasing, made exact.
            .with_many("Trả lời", replies)
            .with_many("android.widget.Button", authors)
            // The rows do change, once — which is what the list actually did.
            .with_many_then(
                "android.widget.TextView",
                vec![node(140.0, 300.0, 600.0, 60.0, "hàng trước khi cuộn")],
                vec![node(140.0, 300.0, 600.0, 60.0, "hàng sau khi cuộn")],
            );

        let refusal = send_reply_by_hierarchy(
            &session,
            vietnamese(),
            (1080.0, 2400.0),
            &parent,
            "reply text",
            &AtomicBool::new(false),
            String::new,
        )
        .await
        .expect("no device error")
        .expect_err("the parent is not there, so this must refuse");

        let ReplyRefusal::ParentNotFound { scrolls, .. } = refusal else {
            panic!("expected ParentNotFound, got {refusal:?}");
        };
        assert!(
            scrolls > 1,
            "one scroll then 'the end of the list' is the bug: the rows changed, so the list \
             moved, and the search had to keep going — got {scrolls}"
        );
        assert!(
            *session.typed.lock() == Vec::<String>::new(),
            "still nothing typed: a parent that was never found is never replied to"
        );
    }

    /// The counts a threshold is measured against, in the shapes TikTok actually writes.
    #[test]
    fn a_rail_label_yields_the_number_it_states() {
        // The two measured labels, verbatim.
        assert_eq!(parse_count("Like video. 22 likes"), Some((22, false)));
        assert_eq!(
            parse_count("Read or add comments. 21 comments"),
            Some((21, false))
        );
        // A thousands separator is not a decimal point…
        assert_eq!(parse_count("1,160 likes"), Some((1_160, false)));
        // …but inside an abbreviation it is, and the result is rounded — which the caller has
        // to know, because a threshold cannot be met exactly against a rounded total.
        assert_eq!(parse_count("1.2K likes"), Some((1_200, true)));
        assert_eq!(parse_count("3M likes"), Some((3_000_000, true)));
        // Nothing to read is `None`, never `0`: "no number here" and "nobody liked it" are
        // different answers and a threshold would act on them differently.
        assert_eq!(parse_count("Like"), None);
        assert_eq!(parse_count(""), None);
    }

    /// A build with no measured counted-like control reports nothing rather than guessing.
    #[tokio::test(start_paused = true)]
    async fn an_unmeasured_build_reports_no_like_count() {
        // The Vietnamese set has `like_count: None` — see the catalogue entry for why.
        let session = ArrivalSession::new(TIKTOK, &[]);
        let counters = read_post_counters(&session, vietnamese()).await;
        assert_eq!(counters.likes, None);
    }

    /// A near-miss in the suggestion list must never be tapped.
    ///
    /// Measured 24/08/2026: typing `@lt.gi` returned `lt.gi`, `.lt.gi.mang.v`, `lt.g94` and
    /// `lt.gr37`. Three of those are different people. Tapping the wrong one mentions a
    /// stranger from a real logged-in account, and nothing in the posted comment would show
    /// that the operator had not asked for it — so the rule is exact match or nothing.
    #[tokio::test(start_paused = true)]
    async fn a_mention_is_only_picked_when_the_row_is_exactly_that_handle() {
        let rows = vec![
            node(179.0, 291.0, 70.0, 50.0, "lt.g94"),
            node(179.0, 438.0, 223.0, 50.0, ".lt.gi.mang.v"),
            node(179.0, 585.0, 118.0, 50.0, "lt.gr37"),
        ];
        let session = DrawerSession::default().with_many("android.widget.TextView", rows);

        let outcome = append_mentions_by_picker(
            &session,
            (1080.0, 2400.0),
            &["lt.gi".to_string()],
            &AtomicBool::new(false),
        )
        .await
        .expect("no transport error");

        // Typed as real keys — that is the only channel the picker reacts to — and with a
        // leading space, or the tag runs into the last word of the comment.
        assert_eq!(*session.keyed.lock(), vec![" @lt.gi".to_string()]);
        // …and then nothing was tapped, because no row *is* `lt.gi`.
        assert!(
            session.taps.lock().is_empty(),
            "a row that merely starts like the handle is a different account"
        );
        assert_eq!(outcome.linked, Vec::<String>::new());
        assert_eq!(outcome.literal, vec!["lt.gi".to_string()]);
        assert!(outcome
            .note()
            .is_some_and(|note| note.contains("chỉ là chữ")));
    }

    /// Two rows claiming one handle is refused rather than resolved by picking the first.
    ///
    /// Both rows have to arrive **after** typing for this to be about ambiguity at all: a row
    /// already on screen beforehand is a comment, not a suggestion, and is refused for a
    /// different reason — see
    /// `a_comment_row_bearing_the_handle_is_never_mistaken_for_the_picker`.
    #[tokio::test(start_paused = true)]
    async fn an_ambiguous_suggestion_list_tags_nobody() {
        let session = DrawerSession::default().with_many_queue(
            "android.widget.TextView",
            vec![
                Vec::new(),
                vec![
                    node(179.0, 291.0, 70.0, 50.0, "lt.gi"),
                    node(179.0, 438.0, 70.0, 50.0, "lt.gi"),
                ],
            ],
        );
        let outcome = append_mentions_by_picker(
            &session,
            (1080.0, 2400.0),
            &["lt.gi".to_string()],
            &AtomicBool::new(false),
        )
        .await
        .expect("no transport error");
        assert!(session.taps.lock().is_empty());
        assert_eq!(outcome.literal, vec!["lt.gi".to_string()]);
    }

    /// A handle that cannot be typed safely is not typed at all — and is reported that way.
    ///
    /// `type_keys` reaches a real device shell, so its character whitelist is a security
    /// boundary — see `UiSession::type_keys`. A refusal there must not tap into an unfiltered
    /// list, and it must not claim the handle is in the comment: nothing was typed, so the
    /// characters are not there in any form. Reporting it as `literal` said the opposite.
    #[tokio::test(start_paused = true)]
    async fn a_handle_the_key_channel_refuses_is_never_typed() {
        let session = DrawerSession::default().with_many(
            "android.widget.TextView",
            vec![node(179.0, 291.0, 70.0, 50.0, "ai đó")],
        );
        let outcome = append_mentions_by_picker(
            &session,
            (1080.0, 2400.0),
            &["ai đó".to_string()],
            &AtomicBool::new(false),
        )
        .await
        .expect("no transport error");
        assert!(session.taps.lock().is_empty());
        assert!(session.keyed.lock().is_empty(), "nothing reached the shell");
        assert_eq!(outcome.untyped, vec!["ai đó".to_string()]);
        assert!(
            outcome.literal.is_empty(),
            "it is not in the comment as text either"
        );
        assert!(outcome
            .note()
            .is_some_and(|note| note.contains("không gõ được")));
    }

    /// A comment row is not a suggestion row, and the difference is *when it appeared*.
    ///
    /// Reachable straight from the product path. While `append_mentions_by_picker` runs the
    /// drawer is open and the comment **list** is on screen behind the composer; every row in
    /// it is a `TextView` whose text is an author handle. Ask to tag somebody who has already
    /// commented under the post — its own author, most often — and an exact match is on screen
    /// before a key is pressed. The old sweep read the whole screen the instant after typing,
    /// when the picker (a network fetch) definitionally had not arrived, found that one match
    /// and tapped it: profile opened, drawer gone, draft lost. It then asked whether the row
    /// was still there, found it was not — because the screen had been replaced — and recorded
    /// a **real mention**.
    #[tokio::test(start_paused = true)]
    async fn a_comment_row_bearing_the_handle_is_never_mistaken_for_the_picker() {
        // One screen throughout: a comment by the tag target, and no picker, ever.
        let session = DrawerSession::default().with_many(
            "android.widget.TextView",
            vec![node(48.0, 1002.0, 223.0, 50.0, ".lt.gi.mang.v")],
        );

        let outcome = append_mentions_by_picker(
            &session,
            (1080.0, 2400.0),
            &[".lt.gi.mang.v".to_string()],
            &AtomicBool::new(false),
        )
        .await
        .expect("no transport error");

        assert!(
            session.taps.lock().is_empty(),
            "the only row claiming the handle was there before a key was pressed"
        );
        assert_eq!(outcome.linked, Vec::<String>::new());
        assert_eq!(outcome.literal, vec![".lt.gi.mang.v".to_string()]);
    }

    /// A tap that loses the composer is not a mention, and it stops the pass.
    ///
    /// "The row went away" was the old proof of success, and a tap that navigates off the
    /// drawer satisfies it exactly as well as a token does. The composer is the witness that
    /// separates the two: it can only answer while the drawer is still on screen.
    #[tokio::test(start_paused = true)]
    async fn a_tap_that_loses_the_composer_is_not_a_mention() {
        let session = DrawerSession::default().with_many_queue(
            "android.widget.TextView",
            vec![
                Vec::new(),
                vec![node(179.0, 291.0, 223.0, 50.0, ".lt.gi.mang.v")],
            ],
        );
        // No `EditText` answer at all — which is what a profile page looks like from here.

        let outcome = append_mentions_by_picker(
            &session,
            (1080.0, 2400.0),
            &[".lt.gi.mang.v".to_string(), "lt.gi".to_string()],
            &AtomicBool::new(false),
        )
        .await
        .expect("no transport error");

        assert_eq!(session.taps.lock().len(), 1);
        assert_eq!(outcome.linked, Vec::<String>::new());
        assert_eq!(outcome.unverified, vec![".lt.gi.mang.v".to_string()]);
        assert_eq!(
            *session.keyed.lock(),
            vec![" @.lt.gi.mang.v".to_string()],
            "a second handle cannot be typed into a drawer that is gone"
        );
        assert!(outcome
            .note()
            .is_some_and(|note| note.contains("không đọc lại được ô soạn")));
    }

    /// The one shape that really is a mention: a fresh row, tapped, list closes, drawer stays.
    #[tokio::test(start_paused = true)]
    async fn a_fresh_row_that_closes_over_the_handle_is_a_real_mention() {
        let session = DrawerSession::default()
            // Three screens: before typing, what the picker offered, what is left after.
            .with_many_queue(
                "android.widget.TextView",
                vec![
                    Vec::new(),
                    vec![node(179.0, 291.0, 223.0, 50.0, ".lt.gi.mang.v")],
                    Vec::new(),
                ],
            )
            .with_many(
                EDIT,
                vec![node(64.0, 1379.0, 800.0, 88.0, "xin chào @.lt.gi.mang.v ")],
            );

        let outcome = append_mentions_by_picker(
            &session,
            (1080.0, 2400.0),
            &[".lt.gi.mang.v".to_string()],
            &AtomicBool::new(false),
        )
        .await
        .expect("no transport error");

        assert_eq!(session.taps.lock().len(), 1);
        assert_eq!(outcome.linked, vec![".lt.gi.mang.v".to_string()]);
        assert!(
            outcome.literal.is_empty()
                && outcome.untyped.is_empty()
                && outcome.unverified.is_empty(),
            "exactly one bucket may claim a handle"
        );
    }

    /// A handle typed in a different case than the account's own is still a real mention.
    ///
    /// The row match has always been case-insensitive, so the pick succeeds — but the read-back
    /// compared the composer against the operator's spelling, so `LT.GI` for `lt.gi` would have
    /// been filed `unverified` and the whole pass abandoned on a mention that actually landed.
    #[tokio::test(start_paused = true)]
    async fn a_handle_typed_in_another_case_is_still_linked() {
        let session = DrawerSession::default()
            .with_many_queue(
                "android.widget.TextView",
                vec![
                    Vec::new(),
                    vec![node(179.0, 291.0, 223.0, 50.0, ".lt.gi.mang.v")],
                    Vec::new(),
                ],
            )
            // The token carries the account's own spelling, not what was typed.
            .with_many(
                EDIT,
                vec![node(64.0, 1379.0, 800.0, 88.0, "xin chào @.lt.gi.mang.v ")],
            );

        let outcome = append_mentions_by_picker(
            &session,
            (1080.0, 2400.0),
            &[".LT.GI.MANG.V".to_string()],
            &AtomicBool::new(false),
        )
        .await
        .expect("no transport error");

        assert_eq!(outcome.linked, vec![".LT.GI.MANG.V".to_string()]);
        assert!(outcome.unverified.is_empty(), "the mention did land");
    }

    /// A three-column grid gets nine tiles opened, not three.
    ///
    /// `ElementBox.y` is the **top edge** and `tap.y` is a fixed offset from it, so every count
    /// in a grid row derives exactly the same y. The de-dup guard compared y alone, which made
    /// tiles two and three of every row look like tiles already visited: on a three-column grid
    /// it examined one post in three, scrolled past the rest, and reported the indistinguishable
    /// "not found". There was no test on this function at all, which is how it survived.
    #[tokio::test(start_paused = true)]
    async fn every_tile_in_a_grid_row_is_opened_not_just_the_leftmost() {
        const TARGET: &str = "bài của tôi hôm nay, chú thích đủ dài để được coi là caption";
        const OTHER: &str = "một bài khác hoàn toàn, cũng đủ dài để được coi là một caption";
        // Three rows of three, laid out the way the grid lays them out: one y per row.
        let grid: Vec<ElementBox> = [900.0, 1400.0, 1900.0]
            .into_iter()
            .flat_map(|y| {
                [12.0, 370.0, 728.0]
                    .into_iter()
                    .map(move |x| node(x, y, 60.0, 40.0, "431"))
            })
            .collect();
        let session = DrawerSession::default()
            .with_many("android.widget.TextView", grid)
            // The post being measured first, then a different post for every tile opened, so
            // nothing ever matches and the walk has to visit them all.
            .with_many_queue(
                "com.bytedance.tux.input.TuxTextLayoutView",
                vec![
                    vec![node(48.0, 700.0, 900.0, 60.0, TARGET)],
                    vec![node(48.0, 700.0, 900.0, 60.0, OTHER)],
                ],
            )
            .with_single("Hồ sơ ", node(940.0, 1500.0, 90.0, 90.0, "Hồ sơ ai đó"));

        let views = read_view_count(
            &session,
            vietnamese(),
            (1080.0, 2400.0),
            &AtomicBool::new(false),
        )
        .await;

        assert_eq!(views, None, "no tile carried the wanted caption");
        let mut distinct: Vec<(i64, i64)> = session
            .taps
            .lock()
            .iter()
            .map(|point| (point.x as i64, point.y as i64))
            .collect();
        distinct.sort_unstable();
        distinct.dedup();
        // Nine tiles plus the one tap on the profile link. A y-only guard reached four.
        assert_eq!(
            distinct.len(),
            10,
            "expected every tile of every row to be opened, got {distinct:?}"
        );
    }

    /// The derived tap point never lands in the profile header, whatever the count's own y.
    ///
    /// The count filter and the tap offset are `TILE_TAP_UP` apart, so guarding where the count
    /// sits is not guarding where the tap lands. On a 1080x2400 phone the header line is 600 and
    /// a count at 610 derived a tap at 410 — the header/action strip, which carries **Follow**
    /// and **Message**. A read that can follow a stranger from a real logged-in account is not a
    /// read, and a follow does not undo itself.
    #[tokio::test(start_paused = true)]
    async fn a_tile_tap_never_lands_in_the_profile_header() {
        let screen = (1080.0, 2400.0);
        let header = screen.1 * 0.25;
        // Every y from just inside the count filter down to the bottom of the screen.
        let nodes: Vec<ElementBox> = (0..120)
            .map(|step| {
                node(
                    12.0,
                    header + 1.0 + f64::from(step) * 15.0,
                    60.0,
                    40.0,
                    "431",
                )
            })
            .collect();
        let session = DrawerSession::default().with_many("android.widget.TextView", nodes);

        let tiles = read_profile_tiles(&session, screen).await;
        assert!(!tiles.is_empty(), "the fixture has to yield some tiles");
        for tile in tiles {
            assert!(
                tile.tap.y >= header,
                "tap at y={} is inside the header strip (< {header})",
                tile.tap.y
            );
            assert!(
                tile.tap.y < screen.1,
                "tap at y={} is off-screen",
                tile.tap.y
            );
            assert!(
                tile.tap.x >= 0.0 && tile.tap.x < screen.0,
                "tap at x={} is off-screen",
                tile.tap.x
            );
        }
    }

    /// Stripping a label never indexes into the middle of a character.
    ///
    /// The old shape matched on `haystack.to_lowercase()` and then cut `haystack` by
    /// `needle.len()`. Those two agree only while every character in the matched region keeps
    /// its byte length through `to_lowercase` — U+212A KELVIN SIGN is three bytes and lowercases
    /// to a one-byte `k`, so byte 1 is inside the first character and the slice **panics**. No
    /// needle in the catalogue contains a `k` today, which is the whole reason to pin it: adding
    /// one must not be able to crash a live campaign from the arrival hot path.
    #[test]
    fn stripping_a_label_never_indexes_into_the_middle_of_a_character() {
        assert_eq!(
            strip_ignoring_case("\u{212A}xyz", "k", true).as_deref(),
            Some("xyz")
        );
        assert_eq!(
            strip_ignoring_case("ai đó \u{212A}", "k", false).as_deref(),
            Some("ai đó")
        );
        // The same character inside a longer needle, where the old code gave a plausible but
        // wrong answer instead of panicking — and a wrong author identity means commenting
        // under the wrong post.
        assert_eq!(
            strip_ignoring_case("\u{212A}elvin Trần", "kelvin", true).as_deref(),
            Some("Trần")
        );
    }

    /// The needles that are actually catalogued keep working, in both directions.
    #[test]
    fn stripping_a_label_still_folds_the_two_measured_prefixes() {
        assert_eq!(
            strip_ignoring_case("Follow ai đó", "Follow ", true).as_deref(),
            Some("ai đó")
        );
        assert_eq!(
            strip_ignoring_case("Hồ sơ ai đó", "Hồ sơ ", true).as_deref(),
            Some("ai đó")
        );
        assert_eq!(
            strip_ignoring_case("someone profile", " profile", false).as_deref(),
            Some("someone")
        );
        // A prefix-only label is not an identity.
        assert_eq!(
            strip_ignoring_case("Follow ", "Follow ", true).as_deref(),
            Some("")
        );
        assert_eq!(strip_ignoring_case("ai đó", "Follow ", true), None);
    }

    /// A number too big to be a play count is refused, not saturated.
    #[test]
    fn a_number_too_big_to_be_a_count_is_refused_rather_than_saturated() {
        // `value * multiplier` on a long digit run is `inf`, and `inf as u32` has saturated to
        // `u32::MAX` since Rust 1.45 — no panic, no overflow, just a number. Every threshold
        // measured against `u32::MAX` then reads as already satisfied, so the farm quietly
        // stops working towards a target it never reached.
        assert_eq!(parse_count(&"9".repeat(400)), None);
        assert_eq!(parse_count("999999999999B"), None);
        // The counts that really appear on the rail and the grid still parse.
        assert_eq!(parse_count("Like video. 1,160 likes"), Some((1_160, false)));
        assert_eq!(parse_count("1.2K"), Some((1_200, true)));
        assert_eq!(parse_count("431"), Some((431, false)));
    }

    /// A caption that happens to begin with digits is not a play count.
    ///
    /// Measured 24/08/2026: a grid overlay carries the number and nothing else — `431`, `1.2K`.
    /// "Starts with a digit" was the only test, so `2026 was the year…` became a tile claiming
    /// 2026 views with a tap point derived from a caption's position on screen.
    #[tokio::test(start_paused = true)]
    async fn a_caption_that_begins_with_digits_is_not_a_play_count() {
        let session = DrawerSession::default().with_many(
            "android.widget.TextView",
            vec![
                node(
                    48.0,
                    900.0,
                    900.0,
                    60.0,
                    "2026 was the year everything changed",
                ),
                node(12.0, 1400.0, 60.0, 40.0, "431"),
            ],
        );
        let tiles = read_profile_tiles(&session, (1080.0, 2400.0)).await;
        assert_eq!(tiles.len(), 1, "only the bare number is a count");
        assert_eq!(tiles[0].views, 431);
    }

    /// The caption is found by `resource-id`, not by class name.
    ///
    /// Measured 24/08/2026 with the same post open on both builds this farm runs: the caption node
    /// is `com.bytedance.tux.input.TuxTextLayoutView` on `com.ss.android.ugc.trill` and **`X.1BOr`**
    /// on `com.zhiliaoapp.musically` — obfuscated, and it changes with the build — while both carry
    /// a `resource-id` ending `:id/desc`.
    ///
    /// The class-keyed lookup therefore read the caption on sixteen phones and returned nothing on
    /// four. `None` is indistinguishable from "this is not a post page", so a whole pass was
    /// recorded as "the deep link did not navigate" while the post had been open the entire time.
    /// This fixture offers **only** the id, the way an obfuscated build does.
    #[tokio::test(start_paused = true)]
    async fn the_caption_is_found_by_resource_id_not_by_class() {
        const CAPTION: &str =
            "A compact list to go to Da Lat without struggling to choose a place.";
        let session = DrawerSession::default()
            .with_many(":id/desc", vec![node(32.0, 1688.0, 838.0, 200.0, CAPTION)]);

        assert_eq!(read_post_caption(&session).await.as_deref(), Some(CAPTION));
    }

    /// And the classes measured first still work, for a build that renames `desc`.
    #[tokio::test(start_paused = true)]
    async fn the_measured_caption_classes_stay_as_a_fallback() {
        const CAPTION: &str =
            "A compact list to go to Da Lat without struggling to choose a place.";
        let session = DrawerSession::default().with_many(
            "com.bytedance.tux.input.TuxTextLayoutView",
            vec![node(32.0, 1688.0, 838.0, 200.0, CAPTION)],
        );

        assert_eq!(read_post_caption(&session).await.as_deref(), Some(CAPTION));
    }

    #[tokio::test(start_paused = true)]
    async fn a_drawer_that_has_not_rendered_yet_is_not_the_end_of_the_list() {
        // The sibling of the aliasing bug above, and a worse one: both anchors answer with an
        // **empty** vector when they cannot see anything, and empty compares equal to empty.
        // Zero reply controls before and zero after satisfy `!moved`; zero rows before and
        // zero after satisfy the text check; so the loop concluded it had reached the end of a
        // list it had not read one row of.
        //
        // Measured 24/08/2026 on `.../@.lt.gi.mang.v/photo/7668947001618320660`, a post
        // carrying 22 comments: three replies refused after **one** scroll out of a budget of
        // ten, because the drawer opens before TikTok has rendered anything into it.
        let parent = CommentLocatorIdentity {
            author_label: "Tồi nhưng tử tế".into(),
            text: "a comment nobody posted".into(),
            locator_version: HIERARCHY_LOCATOR_VERSION.into(),
            frame_sha256: "sha".into(),
        };
        // A drawer that opens and stays empty: the field is there, the list is not.
        let session = DrawerSession::default()
            .with_single("bình luận", node(880.0, 900.0, 120.0, 120.0, "bình luận"))
            .with_single(EDIT, node(199.0, 1175.0, 700.0, 100.0, ""))
            .with_single(SEND_ID, send_button(false));

        let refusal = send_reply_by_hierarchy(
            &session,
            vietnamese(),
            (1080.0, 2400.0),
            &parent,
            "reply text",
            &AtomicBool::new(false),
            String::new,
        )
        .await
        .expect("no device error")
        .expect_err("there is no parent to find, so this must still refuse");

        let ReplyRefusal::ParentNotFound {
            scrolls, saw_rows, ..
        } = refusal
        else {
            panic!("expected ParentNotFound, got {refusal:?}");
        };
        assert_eq!(
            scrolls, PARENT_SCROLL_ATTEMPTS,
            "an unreadable list must cost the whole budget, not one scroll — otherwise              'I could not see it' is being reported as 'it was not there'"
        );
        assert!(
            !saw_rows,
            "the refusal has to remember that nothing was ever legible, or its message              sends the operator to the post instead of to the phone's network"
        );
        assert!(
            refusal.message().contains("không hiện dòng nào"),
            "got {:?}",
            refusal.message()
        );
        assert!(
            session.typed.lock().is_empty(),
            "still nothing typed: a parent that was never found is never replied to"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn the_phone_is_sent_home_before_the_link_is_opened() {
        // Arrival is proved by the author label *changing*, and that test is vacuous in two
        // states a fleet is regularly left in — both measured 19/08/2026, both cured by the
        // same tap:
        //
        // * already on the target post, so nothing changes and the open is refused as
        //   `target_open_screen_unchanged`, the message for a deleted post, about a post that
        //   is on screen and fine (ce051715ac247a3f01, same link run twice — which is exactly
        //   what retrying an assignment does);
        // * not on a post at all, so there is no author to read and the open is refused as
        //   `target_open_no_baseline` before it is even attempted (ce0417145199e0490c, left on
        //   a search results page by an earlier run).
        let mut present = post_page(Some("Follow Trước"));
        present.push(("Trang chủ", "Trang chủ"));
        let session = ArrivalSession::new(TIKTOK, &present).landing_on("Follow Sau");

        let arrival = open_target_by_hierarchy(
            &session,
            vietnamese(),
            TIKTOK,
            "https://www.tiktok.com/@someone/video/1",
            "someone",
            &AtomicBool::new(false),
        )
        .await
        .expect("the open still works");

        assert_eq!(arrival, TargetArrival::Structural);
        assert!(
            !session.taps.lock().is_empty(),
            "Home has to be tapped, or a phone already on the target post can never prove it \
             arrived at the target post"
        );
    }

    /// The counter row exactly as the fleet build laid it out.
    ///
    /// Read off a dump taken immediately after one horizontal swipe on
    /// `.../@pht.th.h.slay/photo/7668948504827448583`, 25/08/2026. Three nodes, flush against
    /// each other, on one row at the top right.
    fn measured_counter_row() -> Vec<ElementBox> {
        vec![
            node(955.0, 195.0, 21.0, 39.0, "2"),
            node(976.0, 195.0, 30.0, 39.0, " / "),
            node(1006.0, 195.0, 21.0, 39.0, "2"),
        ]
    }

    #[test]
    fn the_measured_counter_row_reads_as_slide_two_of_two() {
        let row = measured_counter_row();
        let separators = vec![row[1].clone()];
        assert_eq!(carousel_index_from(&separators, &row), Some((2, 2)));
    }

    /// A digit that is nearer the separator but on another row must not be read as the index.
    ///
    /// Goes red the moment the row check is dropped: without it the only numeric node on the
    /// left is the one 400 px down the screen, and the function answers `Some((1, 2))` for a
    /// screen that never showed a first slide. This is the same rule the comment locator
    /// needs and for the same reason — a screen is full of numbers, and only the row makes
    /// two of them a pair.
    ///
    /// **The distractor is `1` on purpose.** It was `7` first, and that test passed with the
    /// row check deleted: `7 / 2` is refused by [`sane_pair`] anyway, so the assertion was
    /// proving the sanity bound rather than the geometry. A value the bound accepts is what
    /// makes the row the only thing standing between this screen and a wrong answer.
    #[test]
    fn a_digit_on_another_row_is_not_the_slide_index() {
        let row = measured_counter_row();
        let separator = row[1].clone();
        let elsewhere = vec![
            node(958.0, 600.0, 18.0, 39.0, "1"),
            separator.clone(),
            row[2].clone(),
        ];
        assert_eq!(
            carousel_index_from(&[separator], &elsewhere),
            None,
            "a counter with no left-hand digit on its own row is unread, not guessed"
        );
    }

    /// The action rail's own numbers sit on the same screen and must stay out of it.
    #[test]
    fn the_rail_counters_do_not_become_a_slide_index() {
        let mut screen = measured_counter_row();
        // Real bounds from the same dump: the Friends tab badge and the like count.
        screen.push(node(352.0, 97.0, 65.0, 39.0, "28"));
        screen.push(node(980.0, 1259.0, 41.0, 37.0, "28"));
        let separators = vec![screen[1].clone()];
        assert_eq!(carousel_index_from(&separators, &screen), Some((2, 2)));
    }

    #[test]
    fn a_one_node_counter_is_read_without_the_row() {
        assert_eq!(parse_carousel_pair("2 / 2"), Some((2, 2)));
        assert_eq!(parse_carousel_pair("3/8"), Some((3, 8)));
    }

    #[test]
    fn an_impossible_pair_is_refused() {
        assert_eq!(
            parse_carousel_pair("5 / 2"),
            None,
            "past the end of the post"
        );
        assert_eq!(parse_carousel_pair("0 / 2"), None, "slides are one-based");
        assert_eq!(
            parse_carousel_pair("1 / 999"),
            None,
            "a total past the sanity bound is a misread, not a very long post"
        );
        assert_eq!(parse_carousel_pair("1 / 2 / 3"), None);
        assert_eq!(parse_carousel_pair("Trả lời"), None);
    }

    #[tokio::test]
    async fn read_carousel_index_reads_the_split_counter_off_a_session() {
        let row = measured_counter_row();
        let session = DrawerSession::default()
            .with_many("/", vec![row[1].clone()])
            .with_many(CAROUSEL_COUNTER_CLASS, row);
        assert_eq!(read_carousel_index(&session).await, Some((2, 2)));
    }

    /// No counter on screen is `None`, and `None` is what stops a traversal.
    ///
    /// The alternative is what the measurement showed: one swipe past the last slide leaves
    /// the post for the author's profile page, where the Follow button is.
    #[tokio::test]
    async fn a_screen_without_a_counter_reads_none() {
        let session = DrawerSession::default();
        assert_eq!(read_carousel_index(&session).await, None);
    }

    /// A phone whose carousel behaves however the test needs it to.
    ///
    /// Everything here is keyed off the **swipe count**, not off a call counter, because
    /// `read_carousel_index` asks twice for one screen (the separator, then the row) and a
    /// fixture that advanced per call would answer the two halves of one read from two
    /// different slides.
    struct CarouselSession {
        /// What the counter says after swipe 1, 2, 3 ... `None` is an unreadable counter.
        counters: Vec<Option<(u32, u32)>>,
        /// Whether the comment rail is still findable after swipe 1, 2, 3 ...
        on_post: Vec<bool>,
        swipes: Mutex<usize>,
        backs: Mutex<usize>,
    }

    impl CarouselSession {
        fn new(counters: Vec<Option<(u32, u32)>>) -> Self {
            let on_post = vec![true; counters.len().max(1)];
            Self {
                counters,
                on_post,
                swipes: Mutex::new(0),
                backs: Mutex::new(0),
            }
        }

        fn leaving_the_post_after(mut self, swipe: usize) -> Self {
            self.on_post = (0..self.counters.len().max(swipe))
                .map(|index| index + 1 < swipe)
                .collect();
            self
        }

        fn after_swipe<T: Clone>(&self, table: &[T]) -> Option<T> {
            let index = self.swipes.lock().checked_sub(1)?;
            table.get(index).or_else(|| table.last()).cloned()
        }
    }

    #[async_trait::async_trait]
    impl UiSession for CarouselSession {
        async fn tap(&self, _point: crate::types::TapPoint) -> anyhow::Result<()> {
            Ok(())
        }
        async fn swipe(&self, _gesture: crate::types::SwipeGesture) -> anyhow::Result<()> {
            *self.swipes.lock() += 1;
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
        /// A measured pair, so `controls_for` answers and the walk is allowed to swipe at all.
        async fn ui_language(&self) -> Option<String> {
            Some("en".to_string())
        }
        async fn app_version(&self, _bundle_id: &str) -> Option<String> {
            Some("38.3.2".to_string())
        }
        async fn window_size(&self) -> anyhow::Result<(f64, f64)> {
            Ok((1080.0, 2220.0))
        }
        /// Stands in for the comment rail: present exactly while the walk is still on the post.
        async fn locate(&self, _query: ElementQuery<'_>) -> anyhow::Result<Option<ElementBox>> {
            Ok(self.after_swipe(&self.on_post).unwrap_or(true).then(|| {
                node(
                    922.0,
                    1296.0,
                    158.0,
                    171.0,
                    "Read or add comments. 152 comments",
                )
            }))
        }
        async fn locate_all(&self, query: ElementQuery<'_>) -> anyhow::Result<Vec<ElementBox>> {
            let Some(Some((current, total))) = self.after_swipe(&self.counters) else {
                return Ok(Vec::new());
            };
            // The measured three-node row, rebuilt at the measured coordinates.
            let row = vec![
                node(955.0, 195.0, 21.0, 39.0, &current.to_string()),
                node(976.0, 195.0, 30.0, 39.0, " / "),
                node(1006.0, 195.0, 21.0, 39.0, &total.to_string()),
            ];
            Ok(match query {
                ElementQuery::Text { value: "/", .. } => vec![row[1].clone()],
                ElementQuery::ClassName(CAROUSEL_COUNTER_CLASS) => row,
                _ => Vec::new(),
            })
        }
    }

    /// A post page whose caption can change under the walk, standing in for TikTok advancing
    /// to the next video when this one ends.
    struct VideoSession {
        /// The caption reported on read 1, 2, 3 ... the last entry repeats.
        captions: Vec<Option<String>>,
        /// Whether the comment rail is findable on check 1, 2, 3 ... the last entry repeats.
        rail: Vec<bool>,
        caption_reads: Mutex<usize>,
        rail_checks: Mutex<usize>,
    }

    impl VideoSession {
        /// A video that stays put for as long as anyone looks.
        fn playing() -> Self {
            Self {
                captions: vec![Some(VIDEO_CAPTION.to_string())],
                rail: vec![true],
                caption_reads: Mutex::new(0),
                rail_checks: Mutex::new(0),
            }
        }

        /// The caption becomes a different post just before sample `sample` is taken.
        ///
        /// Counted in samples rather than in reads because the walk reads the caption **once
        /// before the first sample** to establish its baseline, and an off-by-one here would
        /// silently test the wrong boundary.
        fn caption_changes_before_sample(mut self, sample: usize) -> Self {
            let read = sample + 1;
            self.captions = (1..=read)
                .map(|index| {
                    Some(if index < read {
                        VIDEO_CAPTION.to_string()
                    } else {
                        OTHER_CAPTION.to_string()
                    })
                })
                .collect();
            self
        }

        fn without_a_readable_caption(mut self) -> Self {
            self.captions = vec![None];
            self
        }

        fn leaves_the_post_on_check(mut self, check: usize) -> Self {
            self.rail = (1..=check).map(|index| index < check).collect();
            self
        }
    }

    /// Forty characters or more, or `read_post_caption` will not believe it is a caption.
    const VIDEO_CAPTION: &str = "một ngày ở Đà Lạt của mình, đi đâu ăn gì cho đủ";
    const OTHER_CAPTION: &str = "một bài hoàn toàn khác, cũng đủ dài để là một caption";

    fn png_picture(tone: u8, compression: CompressionType, filter: FilterType) -> Vec<u8> {
        let image = RgbImage::from_fn(64, 128, |x, y| {
            let value = tone.wrapping_add((x * 3 + y * 5) as u8);
            Rgb([value, value.wrapping_add(17), value.wrapping_add(43)])
        });
        let mut encoded = Vec::new();
        PngEncoder::new_with_quality(&mut encoded, compression, filter)
            .write_image(
                image.as_raw(),
                image.width(),
                image.height(),
                ColorType::Rgb8.into(),
            )
            .expect("encode fixture PNG");
        encoded
    }

    #[async_trait::async_trait]
    impl UiSession for VideoSession {
        async fn tap(&self, _point: crate::types::TapPoint) -> anyhow::Result<()> {
            Ok(())
        }
        async fn swipe(&self, _gesture: crate::types::SwipeGesture) -> anyhow::Result<()> {
            unreachable!("a video walk must never gesture");
        }
        async fn type_text(&self, _text: &str) -> anyhow::Result<()> {
            Ok(())
        }
        async fn home(&self) -> anyhow::Result<()> {
            Ok(())
        }
        async fn back(&self) -> anyhow::Result<()> {
            unreachable!("a video walk must never navigate");
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
        async fn ui_language(&self) -> Option<String> {
            Some("en".to_string())
        }
        async fn app_version(&self, _bundle_id: &str) -> Option<String> {
            Some("38.3.2".to_string())
        }
        async fn window_size(&self) -> anyhow::Result<(f64, f64)> {
            Ok((1080.0, 2220.0))
        }
        async fn locate(&self, _query: ElementQuery<'_>) -> anyhow::Result<Option<ElementBox>> {
            let mut checks = self.rail_checks.lock();
            *checks += 1;
            let present = self
                .rail
                .get(*checks - 1)
                .or_else(|| self.rail.last())
                .copied()
                .unwrap_or(true);
            Ok(present.then(|| node(922.0, 1296.0, 158.0, 171.0, "Read or add comments. 9")))
        }
        async fn locate_all(&self, query: ElementQuery<'_>) -> anyhow::Result<Vec<ElementBox>> {
            if !matches!(query, ElementQuery::ResourceIdSuffix(_)) {
                return Ok(Vec::new());
            }
            let mut reads = self.caption_reads.lock();
            *reads += 1;
            let caption = self
                .captions
                .get(*reads - 1)
                .or_else(|| self.captions.last())
                .cloned()
                .flatten();
            Ok(caption
                .map(|text| vec![node(44.0, 1700.0, 800.0, 120.0, &text)])
                .unwrap_or_default())
        }
    }

    /// A camera driven by a script of picture *identities*, so a frozen stream can be expressed.
    ///
    /// Real encoded PNGs rather than sentinel bytes: the walk deduplicates with
    /// `picture_digest_of`, which decodes, and a byte that is not an image would silently skip
    /// the very check being tested.
    struct ScriptedCamera {
        shades: Vec<u8>,
        taken: Mutex<usize>,
    }

    #[async_trait::async_trait]
    impl SlideCamera for ScriptedCamera {
        async fn capture(&self) -> Option<Vec<u8>> {
            let mut taken = self.taken.lock();
            let shade = *self.shades.get(*taken).or_else(|| self.shades.last())?;
            *taken += 1;
            let mut image = image::RgbImage::new(64, 128);
            for pixel in image.pixels_mut() {
                pixel.0 = [shade, shade, shade];
            }
            let mut out = std::io::Cursor::new(Vec::new());
            image::DynamicImage::ImageRgb8(image)
                .write_to(&mut out, image::ImageFormat::Png)
                .ok()?;
            Some(out.into_inner())
        }
    }

    async fn watch(
        session: &VideoSession,
        shades: Vec<u8>,
        duration: Option<Duration>,
    ) -> VideoWatch {
        photograph_video_post(
            session,
            &ScriptedCamera {
                shades,
                taken: Mutex::new(0),
            },
            "com.ss.android.ugc.trill",
            duration,
        )
        .await
    }

    /// **A playing video is sampled across itself, not at the instant it opened.**
    ///
    /// Four distinct pictures, and a span that says how much of the video they cover. The old
    /// pass took three samples 500 ms apart and saw the first second of whatever the post was.
    #[tokio::test(start_paused = true)]
    async fn a_playing_video_is_sampled_across_its_length() {
        let session = VideoSession::playing();
        let result = watch(
            &session,
            vec![10, 60, 110, 160],
            Some(Duration::from_secs(12)),
        )
        .await;
        assert_eq!(result.frames.len(), 4);
        // Exactly nine: a 12 s video gives `(12 - 1.2) / 3` = 3.6 s clamped to the 3 s ceiling,
        // and four keeps have three gaps between them. Pinned because the span is now read off
        // a clock — under a paused clock a free camera makes schedule and clock agree, which is
        // the fast path production actually runs (a scrcpy frame is already in memory). On real
        // hardware with `screencap` they do **not** agree, and that is the whole reason the
        // number stopped being derived from the schedule.
        assert_eq!(result.span_secs, 9);
    }

    /// **The post changing under the walk stops it, and the pictures already taken are kept.**
    ///
    /// This is the guard that matters: TikTok advances to the next video when one ends, and that
    /// screen has a comment rail on it too — so the rail alone cannot tell the difference. A
    /// picture of the next video filed as evidence for this one is worse than a short sheet,
    /// because it looks like evidence.
    #[tokio::test(start_paused = true)]
    async fn a_post_that_changes_under_the_walk_stops_it() {
        let session = VideoSession::playing().caption_changes_before_sample(3);
        let result = watch(
            &session,
            vec![10, 60, 110, 160],
            Some(Duration::from_secs(60)),
        )
        .await;
        assert_eq!(
            result.frames.len(),
            2,
            "the third sample was of a different post"
        );
    }

    /// Losing the post page stops the walk too, on the cheaper of the two checks.
    #[tokio::test(start_paused = true)]
    async fn leaving_the_post_page_stops_the_walk() {
        let session = VideoSession::playing().leaves_the_post_on_check(2);
        let result = watch(&session, vec![10, 60, 110, 160], None).await;
        assert_eq!(result.frames.len(), 1);
    }

    /// **A frozen stream ends the watch instead of running out the budget.**
    ///
    /// Measured as a real state on this fleet: one phone failed four assignments in a row with a
    /// stuck stream. From here that is indistinguishable from a paused video, and both should
    /// stop rather than spend ten seconds collecting the same picture.
    #[tokio::test(start_paused = true)]
    async fn a_frozen_stream_ends_the_watch_early() {
        let session = VideoSession::playing();
        let result = watch(
            &session,
            vec![10, 10, 10, 10],
            Some(Duration::from_secs(60)),
        )
        .await;
        assert_eq!(
            result.frames.len(),
            1,
            "one picture, however often it repeats"
        );
        assert_eq!(result.span_secs, 0, "one picture spans nothing");
    }

    /// **No readable caption is a thinner guard, not a refusal to photograph.**
    ///
    /// Falling back to the rail check alone is exactly the proof the old sampler had. Refusing
    /// instead would turn a build that renames the caption node into a campaign that comments
    /// on nothing.
    #[tokio::test(start_paused = true)]
    async fn an_unreadable_caption_still_allows_the_watch() {
        let session = VideoSession::playing().without_a_readable_caption();
        let result = watch(&session, vec![10, 60, 110, 160], None).await;
        assert_eq!(result.frames.len(), 4);
    }

    /// **The gap arithmetic, which has an off-by-one in it waiting to happen.**
    ///
    /// Four pictures need three gaps, so dividing the reach by four would sample past the
    /// budget on every video. The bounds are what keep a two-second clip from being sampled
    /// faster than the stream produces distinct frames, and a ten-minute one from spending the
    /// whole budget on two pictures.
    #[test]
    fn the_sample_gap_covers_a_short_video_and_caps_a_long_one() {
        // 12s: reach is 12 - 1.2 = 10.8s over three gaps = 3.6s, clamped to the 3s ceiling.
        assert_eq!(
            video_sample_gap(Some(Duration::from_secs(12)), 4),
            VIDEO_GAP_RANGE.1
        );
        // 6s: reach 4.8s over three gaps = 1.6s, inside the range and the whole clip is covered.
        assert_eq!(
            video_sample_gap(Some(Duration::from_secs(6)), 4),
            Duration::from_millis(1_600)
        );
        // A two-second clip cannot be sampled faster than the floor.
        assert_eq!(
            video_sample_gap(Some(Duration::from_secs(2)), 4),
            VIDEO_GAP_RANGE.0
        );
        // Long, and unknown, both spend the budget: 10s over three gaps, capped at 3s.
        for duration in [Some(Duration::from_secs(600)), None] {
            assert_eq!(video_sample_gap(duration, 4), VIDEO_GAP_RANGE.1);
        }
        // Three gaps, never four: the whole schedule has to fit the budget.
        let gap = video_sample_gap(None, 4);
        assert!(
            VIDEO_FIRST_SAMPLE + gap * 3 <= VIDEO_FIRST_SAMPLE + VIDEO_WATCH_BUDGET,
            "the schedule overruns the budget: {gap:?}"
        );
    }

    /// A camera that hands back a different picture every time it is asked.
    struct CountingCamera {
        taken: Mutex<u8>,
    }

    #[async_trait::async_trait]
    impl SlideCamera for CountingCamera {
        async fn capture(&self) -> Option<Vec<u8>> {
            let mut taken = self.taken.lock();
            *taken += 1;
            Some(vec![*taken])
        }
    }

    /// A camera whose successive frames are already encoded. This is distinct from
    /// `CountingCamera`: sentinel bytes cannot exercise `picture_digest_of`, because decoding
    /// them returns `None` and the walk deliberately retains undecodable evidence.
    struct EncodedCamera {
        frames: Vec<Vec<u8>>,
        taken: Mutex<usize>,
    }

    #[async_trait::async_trait]
    impl SlideCamera for EncodedCamera {
        async fn capture(&self) -> Option<Vec<u8>> {
            let mut taken = self.taken.lock();
            let frame = self
                .frames
                .get(*taken)
                .or_else(|| self.frames.last())?
                .clone();
            *taken += 1;
            Some(frame)
        }
    }

    async fn walk(session: &CarouselSession) -> (PhotoWalk, usize, usize) {
        let camera = CountingCamera {
            taken: Mutex::new(0),
        };
        let gestures = tokio::sync::Mutex::new(());
        let result = photograph_photo_post(
            session,
            &camera,
            "com.ss.android.ugc.trill",
            &gestures,
            Duration::from_millis(0),
        )
        .await;
        let swipes = *session.swipes.lock();
        let backs = *session.backs.lock();
        (result, swipes, backs)
    }

    /// A no-op carousel swipe can still produce a new encoded byte stream and a stale-looking
    /// `2 / 2` counter. Evidence is the decoded picture, not those bytes: the repeated slide
    /// must stay one frame even though the walk performed its single authorised swipe.
    #[tokio::test]
    async fn two_png_encodings_of_one_stuck_slide_are_one_evidence_frame() {
        let fast = png_picture(21, CompressionType::Fast, FilterType::NoFilter);
        let compact = png_picture(21, CompressionType::Best, FilterType::Paeth);
        assert_ne!(fast, compact, "the stream encodings must really differ");
        assert_eq!(
            crate::nurture::picture_digest_of(&fast),
            crate::nurture::picture_digest_of(&compact),
            "the decoded picture is the same"
        );

        let session = CarouselSession::new(vec![Some((2, 2))]);
        let camera = EncodedCamera {
            frames: vec![fast, compact],
            taken: Mutex::new(0),
        };
        let gestures = tokio::sync::Mutex::new(());
        let result = photograph_photo_post(
            &session,
            &camera,
            "com.ss.android.ugc.trill",
            &gestures,
            Duration::from_millis(0),
        )
        .await;

        assert_eq!(
            result.frames.len(),
            1,
            "one decoded slide, one evidence frame"
        );
        assert_eq!(
            *session.swipes.lock(),
            1,
            "the 2 / 2 counter permits one swipe"
        );
        assert_eq!(result.counters, vec![Some((2, 2))]);
    }

    /// A three-slide post is walked to its end and no further.
    ///
    /// Two swipes, not three: the counter says `3 / 3` after the second one, and the swipe that
    /// would follow is the one that leaves for the author's profile page.
    #[tokio::test]
    async fn a_three_slide_post_is_walked_to_its_last_slide() {
        let session = CarouselSession::new(vec![Some((2, 3)), Some((3, 3))]);
        let (result, swipes, backs) = walk(&session).await;
        assert_eq!(result.frames.len(), 3);
        assert_eq!(swipes, 2, "the last slide must not be swiped past");
        assert_eq!(backs, 0);
        assert_eq!(result.counters, vec![Some((2, 3)), Some((3, 3))]);
    }

    /// A post longer than the sheet can carry stops at the cap, and the cap is on gestures.
    #[tokio::test]
    async fn a_long_post_stops_at_the_slide_cap() {
        let session = CarouselSession::new(vec![Some((2, 9)), Some((3, 9)), Some((4, 9))]);
        let (result, swipes, _) = walk(&session).await;
        assert_eq!(result.frames.len(), CAROUSEL_SLIDE_CAP);
        assert_eq!(swipes, CAROUSEL_SLIDE_CAP - 1);
    }

    /// An unreadable counter ends the walk instead of guessing there is one more slide.
    ///
    /// This is the safety rule, not a nicety. The measurement that produced it: one swipe past
    /// the last slide of a photo post does not wrap and does not stay put — it opens the
    /// author's profile page, where the Follow button is, and a follow from an operator's
    /// account does not expire.
    #[tokio::test]
    async fn an_unreadable_counter_stops_the_walk() {
        let session = CarouselSession::new(vec![None]);
        let (result, swipes, backs) = walk(&session).await;
        assert_eq!(
            result.frames.len(),
            2,
            "the slide it did reach is still evidence"
        );
        assert_eq!(swipes, 1);
        assert_eq!(backs, 0, "still on the post, so nothing to undo");
        assert_eq!(result.counters, vec![None]);
    }

    /// A swipe that left the post throws its picture away and goes back.
    ///
    /// Keeping it would be worse than keeping nothing: a screenshot of somewhere else looks
    /// exactly like evidence about this post, and the comment written from it would be about
    /// whatever page the phone wandered onto.
    #[tokio::test]
    async fn a_swipe_that_leaves_the_post_keeps_no_picture() {
        let session = CarouselSession::new(vec![Some((2, 5))]).leaving_the_post_after(1);
        let (result, swipes, backs) = walk(&session).await;
        assert_eq!(
            result.frames.len(),
            1,
            "only the slide taken before the swipe"
        );
        assert_eq!(swipes, 1);
        assert_eq!(backs, 1, "the walk has to undo the navigation it caused");
    }
}
