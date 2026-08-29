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
    ClipboardUnwritable,
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
            Self::ClipboardUnwritable => {
                "không ghi được clipboard nên không có mốc để so — KHÔNG đọc bừa cái đang có, \
                 vì đó là link của bài trước"
                    .into()
            }
            Self::NoCopyRow => "bảng chia sẻ không có dòng sao chép liên kết".into(),
            Self::AmbiguousCopyRow => {
                "bảng chia sẻ có nhiều hơn một dòng giống 'sao chép liên kết' — không đoán".into()
            }
            Self::CopyDidNotLand => {
                "bấm sao chép mà clipboard vẫn là mốc đã ghi — cú bấm trượt".into()
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

/// Tap Share, tap the copy row, and read the link back off the clipboard.
///
/// Puts the share sheet away **only when it opened one**, and never returns `Err`.
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
        Ok(None) => return LinkCapture::NoShareControl,
        Err(error) => return LinkCapture::ReadFailed(error.to_string()),
    };

    // **Written, not observed.** See the module docs: an unreadable prior value used to
    // become an empty baseline, after which any stale link counted as a change.
    let mark = sentinel();
    if session
        .set_clipboard("plaintext", mark.as_bytes())
        .await
        .is_err()
    {
        return LinkCapture::ClipboardUnwritable;
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
    let Ok(rows) = session
        .locate_all_described(ElementQuery::ClassName("android.widget.TextView"))
        .await
    else {
        return CopyRow::Missing;
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
        set_clipboard_fails: bool,
        share_tap_fails: bool,
        /// Once the sheet is dismissed the rows go away, like the real one.
        dismissed: Mutex<bool>,
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
        async fn tap(&self, point: TapPoint) -> anyhow::Result<()> {
            if self.share_tap_fails {
                anyhow::bail!("the agent went away mid-gesture");
            }
            let hit = self.hits_copy_row(&point);
            self.taps.lock().push(point);
            if hit && !self.copies.is_empty() {
                *self.clipboard.lock() = Some((self.kind.clone(), self.copies.clone()));
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
            _query: ElementQuery<'_>,
        ) -> anyhow::Result<Vec<ElementBox>> {
            if *self.dismissed.lock() {
                return Ok(Vec::new());
            }
            Ok(self.rows.clone())
        }
    }

    fn english() -> TikTokControls {
        controls_for("com.ss.android.ugc.trill", "en", "").expect("a measured set")
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
            LinkCapture::ClipboardUnwritable
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
}
