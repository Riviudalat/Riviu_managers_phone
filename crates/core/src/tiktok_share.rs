//! Reading back the link to a post we just published.
//!
//! # A post that went out and whose link could not be read is **not** a failed post
//!
//! The single rule this module exists to hold. These are two different outcomes and they
//! have to be recorded differently, because one of them is retryable and the other is a
//! carousel already sitting on a real account with no delete path on Android. So nothing
//! here returns an error that a caller could plausibly treat as "the post failed":
//! [`capture_post_link`] hands back a [`LinkCapture`], every unhappy variant of which means
//! *the post is fine and the link is missing*.
//!
//! The link's own destination is [`crate::db::publish_sheet`], which is likewise built so
//! that a missing row never reopens a published post.
//!
//! # The clipboard is shared state, and that is the hard part
//!
//! `Copy link` writes to the device clipboard, so reading it back is not proof of anything
//! by itself: if the tap missed, the clipboard still holds **whatever was there before** —
//! a link from the previous post in the same run, most likely — and a naive read would
//! record that stale URL against this post and paste it into the operator's sheet. The
//! sheet would then be wrong in the one way nobody would catch, because every row in it
//! looks like a valid TikTok link.
//!
//! So the clipboard is read **before** the tap as well, and the value must have changed.
//! That is the same shape as the comment drawer's disarm check: the evidence is a
//! transition, not a state.

use std::time::Duration;

use crate::driver::{ElementQuery, UiSession};
use crate::tiktok_labels::{TikTokControl, TikTokControls};

/// How long the share sheet may take to come up.
pub const SHEET_WINDOW: Duration = Duration::from_millis(6_000);
/// How long the clipboard may take to change after `Copy link` is tapped.
///
/// The write is local to the device, so this is short — but not zero: the app puts a toast
/// up and writes the clipboard on its own thread.
pub const CLIPBOARD_WINDOW: Duration = Duration::from_millis(4_000);
pub const POLL: Duration = Duration::from_millis(300);
/// Clipboard reads are capped; a TikTok link is far under this.
const CLIPBOARD_LIMIT: usize = 4_096;

/// The strings the copy row carries, lower-cased, across the builds seen so far.
///
/// Matched case-insensitively and as substrings because the row's label embeds nothing but
/// itself, and because a build this list does not cover should still have a chance rather
/// than failing on capitalisation. Adding a language here is not a measurement — it is a
/// guess that costs nothing if wrong, since a wrong guess simply finds no row and the
/// caller records "link not captured", which is safe.
pub const COPY_ROW_NEEDLES: [&str; 3] = ["copy link", "sao chép liên kết", "sao chép link"];

/// What reading the link achieved. **No variant here means the post failed.**
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkCapture {
    /// The clipboard changed and holds something shaped like a post link.
    Captured(String),
    /// This build has no measured Share control, so nothing was tapped.
    ShareUnmeasured,
    /// Share is measured and was not on screen.
    NoShareControl,
    /// Share was tapped and the sheet never showed a copy row.
    NoCopyRow,
    /// The copy row was tapped and the clipboard did not change.
    ///
    /// Deliberately **not** "read the clipboard anyway": what is in there is the previous
    /// post's link, and writing that into the operator's sheet is worse than writing
    /// nothing, because nothing in the sheet would look wrong.
    ClipboardUnchanged,
    /// The clipboard changed into something that is not a post link.
    NotALink(String),
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
            Self::NoCopyRow => "bảng chia sẻ không có dòng sao chép liên kết".into(),
            Self::ClipboardUnchanged => {
                "bấm sao chép mà clipboard không đổi — KHÔNG lấy link cũ, vì nó là link của \
                 bài trước"
                    .into()
            }
            Self::NotALink(value) => {
                format!("clipboard đổi nhưng không phải link bài: {value:.80}")
            }
        }
    }
}

/// Whether a clipboard value looks like a link to a TikTok post.
///
/// Deliberately loose about the *shape* of the path and strict about the host. The path
/// differs by build and by media kind — `/video/`, `/photo/`, and short `vt.tiktok.com`
/// links all appear — so pinning it would reject good links, while the host is what tells a
/// post link apart from the other things TikTok's share sheet copies (a sound page, a
/// profile, a plain caption) and from whatever the phone's clipboard held before.
pub fn looks_like_a_post_link(value: &str) -> bool {
    let value = value.trim();
    if !(value.starts_with("https://") || value.starts_with("http://")) {
        return false;
    }
    let host = value
        .split_once("://")
        .map(|(_, rest)| rest.split(['/', '?', '#']).next().unwrap_or_default())
        .unwrap_or_default()
        .to_ascii_lowercase();
    host == "tiktok.com" || host.ends_with(".tiktok.com")
}

/// Tap Share, tap the copy row, and read the link back off the clipboard.
///
/// Puts the share sheet away on every exit, including the unhappy ones — leaving it up
/// costs the next step its screen, which is how a probe run once measured the share sheet's
/// nodes and reported them as the comment drawer's.
pub async fn capture_post_link(
    session: &dyn UiSession,
    labels: &TikTokControls,
) -> anyhow::Result<LinkCapture> {
    let Some(share) = labels.label(TikTokControl::Share) else {
        // Nothing was opened, so there is nothing to close.
        return Ok(LinkCapture::ShareUnmeasured);
    };
    let outcome = read_through_sheet(session, share.to_query()).await;
    // Best effort, and never allowed to change the verdict: this runs on failure paths too.
    session.back().await.ok();
    outcome
}

async fn read_through_sheet(
    session: &dyn UiSession,
    share: ElementQuery<'_>,
) -> anyhow::Result<LinkCapture> {
    let Some(control) = session.locate(share).await? else {
        return Ok(LinkCapture::NoShareControl);
    };
    // Read first, so the comparison afterwards has something to compare against. A failure
    // here is treated as "empty": an unreadable clipboard before the tap must not stop the
    // capture, it just makes the change-check weaker in the safe direction.
    let before = session
        .get_clipboard(CLIPBOARD_LIMIT)
        .await
        .ok()
        .map(|(_kind, bytes)| String::from_utf8_lossy(&bytes).trim().to_string())
        .unwrap_or_default();

    session.tap(control.centre()).await?;

    let Some(row) = await_copy_row(session).await? else {
        return Ok(LinkCapture::NoCopyRow);
    };
    session.tap(row).await?;

    let deadline = tokio::time::Instant::now() + CLIPBOARD_WINDOW;
    loop {
        if let Ok((_kind, bytes)) = session.get_clipboard(CLIPBOARD_LIMIT).await {
            let now = String::from_utf8_lossy(&bytes).trim().to_string();
            if now != before && !now.is_empty() {
                return Ok(if looks_like_a_post_link(&now) {
                    LinkCapture::Captured(now)
                } else {
                    LinkCapture::NotALink(now)
                });
            }
        }
        if tokio::time::Instant::now() >= deadline {
            return Ok(LinkCapture::ClipboardUnchanged);
        }
        tokio::time::sleep(POLL).await;
    }
}

/// Find the copy row in the open share sheet, waiting for the sheet to arrive.
///
/// Reads `text` for every row rather than asking for one label, because the row's string is
/// not in the catalogue: it is matched against [`COPY_ROW_NEEDLES`] case-insensitively, and
/// the sheet is a short list so the per-row read costs little.
async fn await_copy_row(session: &dyn UiSession) -> anyhow::Result<Option<crate::types::TapPoint>> {
    let deadline = tokio::time::Instant::now() + SHEET_WINDOW;
    loop {
        let rows = session
            .locate_all_described(ElementQuery::ClassName("android.widget.TextView"))
            .await
            .unwrap_or_default();
        if let Some(row) = rows.iter().find(|row| {
            row.description
                .as_deref()
                .map(|label| {
                    let label = label.to_lowercase();
                    COPY_ROW_NEEDLES.iter().any(|needle| label.contains(needle))
                })
                .unwrap_or(false)
        }) {
            return Ok(Some(row.centre()));
        }
        if tokio::time::Instant::now() >= deadline {
            return Ok(None);
        }
        tokio::time::sleep(POLL).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::driver::ElementBox;
    use crate::tiktok_labels::{controls_for, nothing_measured};
    use crate::types::TapPoint;
    use parking_lot::Mutex;

    #[derive(Default)]
    struct FakeSession {
        rows: Vec<ElementBox>,
        share: Option<ElementBox>,
        /// Clipboard values handed out in order; the last one sticks.
        clipboard: Mutex<Vec<String>>,
        taps: Mutex<Vec<TapPoint>>,
        backs: Mutex<usize>,
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

    #[async_trait::async_trait]
    impl UiSession for FakeSession {
        async fn tap(&self, point: TapPoint) -> anyhow::Result<()> {
            self.taps.lock().push(point);
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
        async fn get_clipboard(&self, _limit: usize) -> anyhow::Result<(String, Vec<u8>)> {
            let mut values = self.clipboard.lock();
            let value = if values.len() > 1 {
                values.remove(0)
            } else {
                values.first().cloned().unwrap_or_default()
            };
            Ok(("text".into(), value.into_bytes()))
        }
        async fn locate(&self, _query: ElementQuery<'_>) -> anyhow::Result<Option<ElementBox>> {
            Ok(self.share.clone())
        }
        async fn locate_all_described(
            &self,
            _query: ElementQuery<'_>,
        ) -> anyhow::Result<Vec<ElementBox>> {
            Ok(self.rows.clone())
        }
    }

    fn english() -> TikTokControls {
        controls_for("com.ss.android.ugc.trill", "en", "").expect("a measured set")
    }

    /// The happy path: the clipboard changes into a link and that is what comes back.
    #[tokio::test(start_paused = true)]
    async fn a_changed_clipboard_holding_a_tiktok_link_is_the_captured_link() {
        let session = FakeSession {
            share: Some(labelled("Share", 1200.0)),
            rows: vec![labelled("Copy link", 1800.0)],
            clipboard: Mutex::new(vec![
                "https://www.tiktok.com/@a/photo/1".into(),
                "https://www.tiktok.com/@a/photo/2".into(),
            ]),
            ..Default::default()
        };
        assert_eq!(
            capture_post_link(&session, &english())
                .await
                .expect("no error"),
            LinkCapture::Captured("https://www.tiktok.com/@a/photo/2".into())
        );
        assert_eq!(*session.backs.lock(), 1, "the share sheet must be put away");
    }

    /// **A clipboard that did not change is refused, not read.**
    ///
    /// The failure this module is shaped around. The stale value is the *previous post's*
    /// link — a perfectly valid TikTok URL — so writing it into the sheet produces a row
    /// that nothing downstream could ever flag as wrong.
    #[tokio::test(start_paused = true)]
    async fn an_unchanged_clipboard_is_never_reported_as_this_posts_link() {
        let stale = "https://www.tiktok.com/@a/photo/1".to_string();
        let session = FakeSession {
            share: Some(labelled("Share", 1200.0)),
            rows: vec![labelled("Copy link", 1800.0)],
            clipboard: Mutex::new(vec![stale.clone()]),
            ..Default::default()
        };
        let capture = capture_post_link(&session, &english())
            .await
            .expect("no error");
        assert_eq!(capture, LinkCapture::ClipboardUnchanged);
        assert_eq!(
            capture.link(),
            None,
            "a stale link must not leak out as this post's"
        );
        assert!(!capture.reason().is_empty());
    }

    /// Something that is not a post link is named as that rather than silently stored.
    #[tokio::test(start_paused = true)]
    async fn a_clipboard_that_changed_into_something_else_is_not_a_link() {
        let session = FakeSession {
            share: Some(labelled("Share", 1200.0)),
            rows: vec![labelled("Copy link", 1800.0)],
            clipboard: Mutex::new(vec!["old".into(), "đi Đà Lạt thật đã".into()]),
            ..Default::default()
        };
        let capture = capture_post_link(&session, &english())
            .await
            .expect("no error");
        assert!(matches!(capture, LinkCapture::NotALink(_)));
        assert_eq!(capture.link(), None);
    }

    /// The row is matched case-insensitively, and in Vietnamese too.
    #[tokio::test(start_paused = true)]
    async fn the_copy_row_is_found_whatever_its_capitalisation_or_language() {
        for label in ["COPY LINK", "Copy Link", "Sao chép liên kết"] {
            let session = FakeSession {
                share: Some(labelled("Share", 1200.0)),
                rows: vec![labelled("Report", 1700.0), labelled(label, 1800.0)],
                clipboard: Mutex::new(vec!["old".into(), "https://vt.tiktok.com/ZS1/".into()]),
                ..Default::default()
            };
            assert_eq!(
                capture_post_link(&session, &english())
                    .await
                    .expect("no error")
                    .link(),
                Some("https://vt.tiktok.com/ZS1/"),
                "{label} was not recognised"
            );
            // Two taps: Share, then the row — and the row, not the first thing in the list.
            let taps = session.taps.lock();
            assert_eq!(taps.len(), 2);
            assert_eq!(taps[1].y, 1850.0, "tapped the wrong row");
        }
    }

    /// An unmeasured build taps nothing at all.
    #[tokio::test(start_paused = true)]
    async fn a_build_without_a_measured_share_control_taps_nothing() {
        let session = FakeSession::default();
        assert_eq!(
            capture_post_link(&session, &nothing_measured())
                .await
                .expect("no error"),
            LinkCapture::ShareUnmeasured
        );
        assert!(session.taps.lock().is_empty());
        assert_eq!(
            *session.backs.lock(),
            0,
            "nothing was opened, so nothing should be closed"
        );
    }

    /// **The host is what makes it a post link**, and the path deliberately is not.
    #[test]
    fn a_link_is_recognised_by_its_host_across_the_shapes_tiktok_uses() {
        for good in [
            "https://www.tiktok.com/@a/photo/7668947001618320660",
            "https://www.tiktok.com/@a/video/7668947001618320660",
            "https://vt.tiktok.com/ZS1abc/",
            "https://tiktok.com/@a/photo/1?is_from_webapp=1",
        ] {
            assert!(looks_like_a_post_link(good), "{good}");
        }
        for bad in [
            "",
            "đi Đà Lạt thật đã",
            "www.tiktok.com/@a/photo/1",
            // The one that matters: a lookalike host. `Contains("tiktok.com")` would take it.
            "https://tiktok.com.example.net/@a/photo/1",
            "https://nottiktok.com/@a/photo/1",
        ] {
            assert!(!looks_like_a_post_link(bad), "{bad}");
        }
    }
}
