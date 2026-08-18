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
use crate::tiktok_labels::{TikTokControl, TikTokControls};

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
                && bottom(candidate) <= body.y + ABOVE_SLACK
                && candidate.y >= body.y - AUTHOR_REACH
                && (candidate.x - body.x).abs() <= body.width.max(1.0)
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
                && bottom(candidate) <= body.y + ABOVE_SLACK
                && candidate.y >= body.y - AUTHOR_REACH
                && (candidate.x - body.x).abs() <= body.width.max(1.0)
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
    // Compared as **contiguous word runs**, not as one string, because the label carries a
    // prefix: it is `Follow <nickname>`, so squashing the whole thing yields
    // `followmongquynh`, which is inside nothing. Stripping the literal `Follow` instead
    // would hard-code a translated word that the catalogue already owns.
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
    /// The parent was only reachable after expanding `View folded comments`.
    ///
    /// A reply under a folded comment is posted, confirmed, and **seen by nobody** —
    /// TikTok folded the parent away from everyone but the account that wrote it. The send
    /// succeeded and the thread is invisible, and those two facts have to travel together
    /// or the operator reads a success and believes something untrue.
    pub parent_was_folded: bool,
}

/// The `locator_version` stamped on identities this module produces.
///
/// Distinct from the OCR path's on purpose: a stored identity has to say which reader
/// created it, because the two read the author label from different places and only one
/// of them can find it again.
pub const HIERARCHY_LOCATOR_VERSION: &str = "android-hierarchy-v1";

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
    stop: &AtomicBool,
    mut frame_sha: F,
) -> anyhow::Result<HierarchySendOutcome>
where
    F: FnMut() -> String,
{
    use crate::tiktok_drawer::{CommentDrawer, CommentVerdict};

    let outcome = |verdict, armed: String, cleared: String, identity| HierarchySendOutcome {
        verdict,
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
    let Some(field) = drawer.open(stop).await? else {
        return Ok(outcome(
            CommentVerdict::NoDrawer,
            String::new(),
            String::new(),
            None,
        ));
    };
    if !drawer.focus_and_type(&field, text, stop).await? {
        return Ok(outcome(
            CommentVerdict::NoSendControl,
            String::new(),
            String::new(),
            None,
        ));
    }
    let Some(send) = drawer.await_armed(stop).await? else {
        return Ok(outcome(
            CommentVerdict::NotArmed,
            String::new(),
            String::new(),
            None,
        ));
    };
    let armed = frame_sha();
    let confirmed = drawer.tap_send_and_confirm_disarm(&send, stop).await?;
    let cleared = frame_sha();
    if !confirmed {
        // `NotConfirmed` and never retried: the tap went out, so a retry is how a post
        // ends up with two identical comments on it. The frames are still returned —
        // they are the only way a person can settle what happened.
        return Ok(outcome(CommentVerdict::NotConfirmed, armed, cleared, None));
    }

    // Read the comment back out of the list that is still on screen. A failure here
    // does not un-send anything, so it downgrades the identity rather than the verdict.
    let identity = read_back_identity(session, text, &cleared).await;
    Ok(outcome(CommentVerdict::Sent, armed, cleared, identity))
}

/// How many times the comment list is scrolled looking for the parent.
///
/// Same budget and same reason as the pixel path's: every reply is sent from a
/// *different* device that re-opens the link fresh, so TikTok re-ranks the list and the
/// campaign's own comment is under no obligation to still be near the top.
/// How long the feed gets to render an author label after the phone is sent Home.
///
/// Bounded, and a miss is not a failure: the baseline read before the tap is kept, which is
/// exactly as good as it was before any of this existed.
const BASELINE_SETTLE: Duration = Duration::from_secs(4);

const PARENT_SCROLL_ATTEMPTS: u32 = 4;

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
async fn expand_folded_comments(session: &dyn UiSession, labels: TikTokControls) -> bool {
    let Some(label) = labels.label(TikTokControl::FoldedComments) else {
        return false;
    };
    let Ok(Some(control)) = session.locate(label.to_query()).await else {
        return false;
    };
    if session.tap(control.centre()).await.is_err() {
        return false;
    }
    tokio::time::sleep(PARENT_SCROLL_SETTLE).await;
    true
}
const PARENT_SCROLL_DURATION_MS: u64 = 320;
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
    ParentNotFound { scrolls: u32, unfolded: bool },
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
            Self::ParentNotFound { scrolls, unfolded } => {
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
    mut frame_sha: F,
) -> anyhow::Result<Result<HierarchySendOutcome, ReplyRefusal>>
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
            parent_was_folded: false,
            armed_frame_sha256: String::new(),
            cleared_frame_sha256: String::new(),
            identity: None,
        }));
    }
    // Opening the drawer also gives the field, whose top edge bounds the list — the
    // measured field jumps from y≈2127 to y≈1175 when the keyboard comes up, so a fixed
    // screen fraction can land inside the composer instead of the list.
    let Some(field) = drawer.open(stop).await? else {
        return Ok(Err(ReplyRefusal::NoDrawer));
    };

    let mut scrolls = 0u32;
    let mut unfolded = false;
    let target = loop {
        if let Some(found) = find_parent(session, reply_label, parent).await {
            break found;
        }
        if scrolls >= PARENT_SCROLL_ATTEMPTS || stop.load(Ordering::Relaxed) {
            return Ok(Err(ReplyRefusal::ParentNotFound { scrolls, unfolded }));
        }
        // Anchors before the swipe, so "the list did not move" is observable rather than
        // assumed. Reply controls are the cheapest anchor: geometry only, no text.
        let before = anchor_positions(session, reply_label).await;
        let rows_before = visible_rows(session).await;
        scroll_comment_list(session, screen, &field).await?;
        tokio::time::sleep(PARENT_SCROLL_SETTLE).await;
        scrolls += 1;
        // A swipe that closed the drawer is a different failure from one that hit the
        // end of the list, and the pixel path checks for exactly this.
        if session
            .locate(ElementQuery::ClassName(EDIT_TEXT))
            .await
            .ok()
            .flatten()
            .is_none()
        {
            return Ok(Err(ReplyRefusal::DrawerClosedByScroll));
        }
        let after = anchor_positions(session, reply_label).await;
        if !moved(&before, &after) {
            // The cheap anchor says stopped. It is wrong often enough to matter — evenly
            // spaced rows alias — so the expensive one gets the final word, and only here,
            // where the alternative is refusing a reply whose parent is further down.
            if visible_rows(session).await != rows_before {
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
            if !unfolded && expand_folded_comments(session, labels).await {
                unfolded = true;
                scrolls = 0;
                continue;
            }
            return Ok(Err(ReplyRefusal::ParentNotFound { scrolls, unfolded }));
        }
    };

    // The field already carries a **non-empty** placeholder before anything is tapped —
    // measured as `Thêm bình luận...`. So "wait for an EditText with some text in it" is
    // satisfied by the state we are already in, and would return instantly with the root
    // drawer's own hint. Read that hint first and wait for it to *change*.
    let before = read_placeholder(session).await.unwrap_or_default();

    // Tap this comment's own Reply control.
    let point = {
        let mut planner = crate::nurture::touch::TouchPointPlanner::new(screen);
        planner.next(target.reply.centre(), target.reply.jitter_radius())
    };
    session.tap(point).await?;

    // The composer replaces the field rather than stacking on it (measured: still
    // exactly one `EditText`), and its *text* is the placeholder — which `locate` cannot
    // see, because `content-desc` is empty there.
    let Some(placeholder) = await_composer(session, &before, stop).await else {
        return Ok(Err(ReplyRefusal::NoComposer));
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
    let Some(field) = session.locate(ElementQuery::ClassName(EDIT_TEXT)).await? else {
        return Ok(Err(ReplyRefusal::NoComposer));
    };
    if !drawer.focus_and_type(&field, text, stop).await? {
        return Ok(Ok(HierarchySendOutcome {
            verdict: CommentVerdict::NoSendControl,
            parent_was_folded: false,
            armed_frame_sha256: String::new(),
            cleared_frame_sha256: String::new(),
            identity: None,
        }));
    }
    let Some(send) = drawer.await_armed(stop).await? else {
        return Ok(Ok(HierarchySendOutcome {
            verdict: CommentVerdict::NotArmed,
            parent_was_folded: unfolded,
            armed_frame_sha256: String::new(),
            cleared_frame_sha256: String::new(),
            identity: None,
        }));
    };
    let armed = frame_sha();
    let confirmed = drawer.tap_send_and_confirm_disarm(&send, stop).await?;
    let cleared = frame_sha();
    if !confirmed {
        return Ok(Ok(HierarchySendOutcome {
            verdict: CommentVerdict::NotConfirmed,
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
    if let Some(placeholder) = read_placeholder(session).await {
        let wanted = parent.author_label.trim();
        if !wanted.is_empty() && placeholder.contains(wanted) {
            let _ = session.back().await;
            sleep_poll().await;
        }
    }
    let identity = read_back_identity(session, text, &cleared).await;
    Ok(Ok(HierarchySendOutcome {
        verdict: CommentVerdict::Sent,
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
) -> Option<ElementReplyTarget> {
    let wanted = parent.text.trim();
    if wanted.is_empty() {
        return None;
    }
    let bodies: Vec<ElementBox> = session
        .locate_all(ElementQuery::Text {
            value: wanted,
            exact: true,
        })
        .await
        .ok()?
        .into_iter()
        .map(|mut body| {
            // Known exactly: it is what the query matched on.
            body.description = Some(wanted.to_string());
            body
        })
        .collect();
    if bodies.is_empty() {
        return None;
    }
    let replies: Vec<ElementBox> = session
        .locate_all(reply_label.to_query())
        .await
        .ok()?
        .into_iter()
        .map(|mut reply| {
            reply.description = Some(reply_label.value().to_string());
            reply
        })
        .collect();
    // The expensive read, and only reached once a candidate body is on screen.
    let authors = session
        .locate_all_described(ElementQuery::ClassName(COMMENT_AUTHOR_CLASS))
        .await
        .ok()?;
    locate_parent_in_elements(&bodies, &replies, &authors, parent)
}

/// Y positions of the reply controls, as a cheap "did the list move" fingerprint.
async fn anchor_positions(
    session: &dyn UiSession,
    reply_label: crate::tiktok_labels::LabelMatch,
) -> Vec<f64> {
    session
        .locate_all(reply_label.to_query())
        .await
        .map(|found| found.into_iter().map(|element| element.y).collect())
        .unwrap_or_default()
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
async fn visible_rows(session: &dyn UiSession) -> Vec<String> {
    session
        .locate_all_described(ElementQuery::ClassName("android.widget.TextView"))
        .await
        .unwrap_or_default()
        .into_iter()
        .filter_map(|element| element.description)
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty())
        .collect()
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
async fn read_placeholder(session: &dyn UiSession) -> Option<String> {
    let fields = session
        .locate_all_described(ElementQuery::ClassName(crate::tiktok_drawer::EDIT_TEXT))
        .await
        .ok()?;
    fields
        .iter()
        .find_map(|field| field.description.as_deref())
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty())
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
) -> Option<String> {
    let deadline = Instant::now() + crate::tiktok_drawer::DRAWER_WINDOW;
    loop {
        if stop.load(Ordering::Relaxed) {
            return None;
        }
        if let Some(text) = read_placeholder(session).await {
            if text != before {
                return Some(text);
            }
        }
        if Instant::now() >= deadline {
            return None;
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

/// The author label of whatever post is on screen.
///
/// Measured: the only labels carrying an author string are `Follow <name>` and
/// `Hồ sơ <name>`, and both carry the **nickname**. That is enough to tell one post from
/// another — which is what the arrival check needs — even when it cannot tell which
/// account it is.
async fn read_author_label(session: &dyn UiSession, labels: TikTokControls) -> Option<String> {
    let label = labels.label(TikTokControl::Follow)?;
    let found = session.locate(label.to_query()).await.ok().flatten()?;
    let observed = found.description.as_deref().unwrap_or_default().trim();
    (!observed.is_empty()).then(|| observed.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::driver::ElementQuery;
    use crate::tiktok_labels::controls_for;
    use crate::types::{SwipeGesture, TapPoint};
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
        /// Label values that answer `locate`, and what they answer with.
        present: Mutex<Vec<(String, ElementBox)>>,
        opened: Mutex<Vec<String>>,
        taps: Mutex<Vec<TapPoint>>,
        typed: Mutex<Vec<String>>,
        open_url_fails: bool,
        /// Installed as the `Follow` label once the link has been opened.
        after_open: Mutex<Option<String>>,
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
                match present.iter_mut().find(|(key, _)| key == follow_key()) {
                    Some(slot) => slot.1 = node,
                    None => present.push((follow_key().to_string(), node)),
                }
            }
            Ok(())
        }
        async fn locate(&self, query: ElementQuery<'_>) -> anyhow::Result<Option<ElementBox>> {
            let wanted = match query {
                ElementQuery::Description { value, .. } => value,
                ElementQuery::ClassName(value) => value,
                ElementQuery::Text { value, .. } => value,
            };
            if wanted == follow_key() {
                let mut reads = self.follow_reads.lock();
                *reads += 1;
                if *reads <= *self.follow_hidden_for.lock() {
                    return Ok(None);
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
                author_label: "Follow Bích Vân".into()
            }
        );
        assert_eq!(refusal.code(), "target_open_screen_unchanged");
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
                author_label: "Follow Mộng Quỳnh".into()
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
                author_label: "Follow nguyenvantoan8584".into()
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
        /// `locate_all` / `locate_all_described` answers, by query value.
        multiples: Mutex<Vec<(String, Vec<ElementBox>)>>,
        /// Replacement answers installed after the first read of a key.
        after_first: Mutex<Vec<(String, Vec<ElementBox>)>>,
        taps: Mutex<Vec<TapPoint>>,
        typed: Mutex<Vec<String>>,
        swipes: Mutex<usize>,
        backs: Mutex<usize>,
    }

    impl DrawerSession {
        fn with_single(self, key: &str, element: ElementBox) -> Self {
            self.singles.lock().push((key.to_string(), element, None));
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
        }
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
            // Stays armed forever: tapped, never disarmed, never vanished.
            .with_single(SEND_ID, send_button(true));

        let mut shas = vec!["armed".to_string(), "cleared".to_string()].into_iter();
        let outcome = send_root_by_hierarchy(
            &session,
            vietnamese(),
            (1080.0, 2400.0),
            "hello",
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

    #[tokio::test(start_paused = true)]
    async fn a_root_send_refuses_before_opening_the_drawer_when_send_is_unmeasured() {
        let english = controls_for("com.zhiliaoapp.musically", "en", "").expect("set");
        let session = DrawerSession::default();
        let outcome = send_root_by_hierarchy(
            &session,
            english,
            (1080.0, 2400.0),
            "hello",
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
    fn node(x: f64, y: f64, width: f64, height: f64, label: &str) -> ElementBox {
        ElementBox {
            x,
            y,
            width,
            height,
            description: Some(label.to_string()),
            enabled: true,
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
}
