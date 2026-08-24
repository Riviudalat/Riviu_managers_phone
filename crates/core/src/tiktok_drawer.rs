//! Driving TikTok's comment drawer through the accessibility hierarchy.
//!
//! Extracted from the nurture loop rather than copied, and the reason matters: this
//! is the most expensively measured thing in the project. Two copies of "the Send
//! button's `enabled` flag going false→true is the armed proof" would drift, and
//! drift there means either a silently dropped comment or the same comment posted
//! twice. One implementation, two callers.
//!
//! The steps are exposed **individually** because the two callers need different
//! endings. Nurture wants the drawer closed and the feed back. Interaction needs it
//! **left open** after Send — its evidence capture reads the posted comment back out
//! of the list, and its reply flow works inside the same drawer. A single
//! `post_comment` that always closed the drawer could not serve both, which is why
//! `leave` is the caller's decision.
//!
//! Measured on `com.ss.android.ugc.trill` 46.3.3 (Redmi Note 12):
//!
//! * the input is `android.widget.EditText` with an **empty** `content-desc` and a
//!   placeholder in `text`, so it is located by **class**;
//! * opening the drawer does **not** focus it — the field sits at y≈2127 with no
//!   keyboard until tapped, then jumps to y≈1175;
//! * the Send button is `android.widget.Button` with `content-desc="@2131823284"`,
//!   an unresolved resource id, and its `enabled` flips false→true when the field
//!   holds text. That transition is the hierarchy's answer to the pixel engine's
//!   `CommentDrawer::SendArmed`.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

// `tokio`'s clock, not `std`'s, so a `#[tokio::test(start_paused = true)]` moves these
// deadlines along with the sleeps. Behaviour in production is identical — the two only
// diverge under a paused runtime — and it is the difference between a test of the
// 5-second send window taking 5 seconds and taking none.
use tokio::time::Instant;

use crate::driver::{ElementBox, ElementQuery, UiSession};
use crate::tiktok_labels::{TikTokControl, TikTokControls};

/// The comment field's widget class.
///
/// Class, not label: the field's `content-desc` is empty and its `text` is the
/// placeholder (`Thêm bình luận...`), which changes with the app's own hint.
pub const EDIT_TEXT: &str = "android.widget.EditText";

/// How long to wait for the drawer, and for the Send control inside it.
///
/// Generous because the drawer loads comments over the network; the wait ends as
/// soon as the element appears, so a fast drawer costs nothing.
pub const DRAWER_WINDOW: Duration = Duration::from_millis(6_000);
/// How long the Send button may take to arm after the text is set.
pub const ARM_WINDOW: Duration = Duration::from_millis(3_000);
/// How long to wait for Send to disarm, which is what proves the comment left.
pub const SEND_WINDOW: Duration = Duration::from_millis(5_000);
pub const DRAWER_POLL: Duration = Duration::from_millis(350);

/// What a comment attempt actually achieved, named for the step that failed.
///
/// Every variant except [`Self::Sent`] means nothing was posted. The distinctions
/// matter to the operator: `SendUnmeasured` is a gap in the catalog that a person can
/// close, while `NotConfirmed` means a tap went out and the result is unknown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommentVerdict {
    /// Posted, and the Send button disarmed to prove it.
    Sent,
    /// Tapped Send, but the disarm could not be read. **Never retried** — the
    /// delivery state is ambiguous and a retry risks a duplicate comment.
    NotConfirmed,
    /// This build's Send control has never been measured, so there is nothing to
    /// aim at. Refuse rather than tap where it was on another build.
    SendUnmeasured,
    /// The drawer never produced an input field.
    NoDrawer,
    /// The drawer opened but the Send control was not in it.
    NoSendControl,
    /// The text went in and Send never armed.
    NotArmed,
    /// The text source had nothing worth saying about this post.
    ContextSkipped,
}

impl CommentVerdict {
    pub fn reason(self) -> &'static str {
        match self {
            Self::Sent => "đã gửi",
            Self::NotConfirmed => {
                "đã bấm Gửi nhưng không xác nhận được; không retry vì trạng thái giao nhận mơ hồ"
            }
            Self::SendUnmeasured => "chưa đo nút Gửi trên bản build này",
            Self::NoDrawer => "drawer bình luận không mở ra ô nhập",
            Self::NoSendControl => "không thấy nút Gửi trong drawer",
            Self::NotArmed => "đã nhập chữ nhưng nút Gửi không sáng",
            Self::ContextSkipped => "ngữ cảnh không đủ để nói gì",
        }
    }

    /// Whether the caller should treat this as "the comment is on the post".
    pub fn is_sent(self) -> bool {
        self == Self::Sent
    }
}

/// Where a tap should land inside a located control.
///
/// A closure rather than a fixed centre so the caller keeps its own jitter policy —
/// nurture threads its `TouchPointPlanner` through here, and a probe can pass the
/// plain centre.
///
/// Generic rather than `&mut dyn FnMut`: these sessions run inside spawned tasks, and
/// a trait object would have to name every auto trait the future needs. A generic
/// parameter lets the caller's own closure carry them.
pub trait TapPlanner: FnMut(&ElementBox) -> crate::types::TapPoint {}
impl<F: FnMut(&ElementBox) -> crate::types::TapPoint> TapPlanner for F {}

/// One open comment drawer, driven a step at a time.
pub struct CommentDrawer<'a, P: TapPlanner> {
    session: &'a dyn UiSession,
    labels: TikTokControls,
    plan_tap: P,
}

impl<'a, P: TapPlanner> CommentDrawer<'a, P> {
    pub fn new(session: &'a dyn UiSession, labels: TikTokControls, plan_tap: P) -> Self {
        Self {
            session,
            labels,
            plan_tap,
        }
    }

    /// The Send control's query, or `None` when this build never had it measured.
    pub fn send_query(&self) -> Option<ElementQuery<'static>> {
        self.labels
            .label(TikTokControl::CommentSend)
            .map(|label| label.to_query())
    }

    async fn tap_inside(&mut self, element: &ElementBox) -> anyhow::Result<()> {
        let point = (self.plan_tap)(element);
        self.session.tap(point).await
    }

    /// Open the drawer from the post's comment control, and return the input field.
    ///
    /// `Ok(None)` means the drawer never produced a field — a real observation, and
    /// the caller decides whether to back out.
    pub async fn open(&mut self, stop: &AtomicBool) -> anyhow::Result<Option<ElementBox>> {
        let Some(opener) = self
            .labels
            .label(TikTokControl::Comments)
            .map(|label| label.to_query())
        else {
            return Ok(None);
        };
        let Some(opener) = self.session.locate(opener).await? else {
            return Ok(None);
        };
        self.tap_inside(&opener).await?;
        // Wait for the field rather than sleeping a fixed time: the drawer loads
        // comments over the network and can take noticeably longer than the animation.
        self.await_element(DRAWER_WINDOW, ElementQuery::ClassName(EDIT_TEXT), stop)
            .await
    }

    /// Focus the field and set the text.
    ///
    /// Focusing is not optional: `type_text` targets the *focused* `EditText`, and an
    /// open drawer has two of them — the collapsed bar behind it and the real field
    /// inside. Setting text on the wrong one succeeds at the API level while the
    /// screen stays empty.
    pub async fn focus_and_type(
        &mut self,
        field: &ElementBox,
        text: &str,
        stop: &AtomicBool,
    ) -> anyhow::Result<bool> {
        self.tap_inside(field).await?;
        let Some(send) = self.send_query() else {
            return Ok(false);
        };
        if self
            .await_element(DRAWER_WINDOW, send, stop)
            .await?
            .is_none()
        {
            return Ok(false);
        }
        self.session.type_text(text).await?;
        Ok(true)
    }

    /// Wait for Send to arm, and return it so the caller can tap it.
    pub async fn await_armed(&self, stop: &AtomicBool) -> anyhow::Result<Option<ElementBox>> {
        let Some(send) = self.send_query() else {
            return Ok(None);
        };
        self.await_condition(ARM_WINDOW, send, stop, |element| element.enabled)
            .await
    }

    /// Tap Send and report whether the disarm was observed.
    ///
    /// The disarm is the proof: the button goes back to not-armed once the comment
    /// leaves. The button vanishing counts too — a drawer that closed is the same
    /// evidence.
    pub async fn tap_send_and_confirm_disarm(
        &mut self,
        send: &ElementBox,
        stop: &AtomicBool,
    ) -> anyhow::Result<bool> {
        self.tap_inside(send).await?;
        let Some(query) = self.send_query() else {
            return Ok(false);
        };
        let disarmed = self
            .await_condition(SEND_WINDOW, query, stop, |element| !element.enabled)
            .await?
            .is_some();
        if disarmed {
            return Ok(true);
        }
        Ok(self.session.locate(query).await?.is_none())
    }

    /// Back out until the feed tab is visible again.
    ///
    /// Best effort by design: this runs on failure paths, where returning an error
    /// would replace a precise verdict with a vague one. What it must not do is leave
    /// the session inside the drawer, so it presses Back several times and the
    /// caller's next check catches anything it missed.
    ///
    /// **The caller decides when to call this.** Interaction deliberately does not,
    /// until it has read its posted comment back out of the still-open list.
    pub async fn leave(&self, stop: &AtomicBool) {
        let feed = self
            .labels
            .label(TikTokControl::FeedTab)
            .map(|label| label.to_query());
        for _ in 0..3 {
            if let Some(feed) = feed {
                if self.session.locate(feed).await.ok().flatten().is_some() {
                    return;
                }
            }
            if self.session.back().await.is_err() {
                return;
            }
            sleep(DRAWER_POLL, stop).await;
        }
    }

    async fn await_element(
        &self,
        window: Duration,
        query: ElementQuery<'_>,
        stop: &AtomicBool,
    ) -> anyhow::Result<Option<ElementBox>> {
        self.await_condition(window, query, stop, |_| true).await
    }

    async fn await_condition(
        &self,
        window: Duration,
        query: ElementQuery<'_>,
        stop: &AtomicBool,
        ready: impl Fn(&ElementBox) -> bool,
    ) -> anyhow::Result<Option<ElementBox>> {
        let deadline = Instant::now() + window;
        loop {
            if let Some(element) = self.session.locate(query).await? {
                if ready(&element) {
                    return Ok(Some(element));
                }
            }
            if Instant::now() >= deadline || stop.load(Ordering::Relaxed) {
                return Ok(None);
            }
            sleep(DRAWER_POLL, stop).await;
        }
    }
}

/// Post one comment and close the drawer behind it.
///
/// The whole flow, for callers that just want a comment posted. Interaction uses the
/// steps directly instead, because it needs the drawer left open.
pub async fn post_comment(
    session: &dyn UiSession,
    labels: TikTokControls,
    plan_tap: impl TapPlanner,
    text: &str,
    stop: &AtomicBool,
) -> anyhow::Result<CommentVerdict> {
    let mut drawer = CommentDrawer::new(session, labels, plan_tap);
    if drawer.send_query().is_none() {
        // Nothing was opened, so there is nothing to close.
        return Ok(CommentVerdict::SendUnmeasured);
    }
    let outcome = post_into_drawer(&mut drawer, text, stop).await;
    // **Closed on the way out whatever happened, including on an error.** Every step below
    // can fail with `?`, and each of those failures used to skip `leave` — which leaves the
    // phone standing inside an open comment list, with the typed text still in the field.
    // The feed loop's next gesture is then spent scrolling comments instead of the feed,
    // which is the same way an interaction that wandered off the feed used to cost a whole
    // session. The last of those sites is the worst: it is reached *after* Send was tapped,
    // so the comment may well be posted and the drawer stays open on top of it.
    drawer.leave(stop).await;
    outcome
}

/// The steps that need an open drawer. Split out so [`post_comment`] can close it on every
/// exit without repeating the call at each early return, and without a `?` bypassing it.
async fn post_into_drawer<P: TapPlanner>(
    drawer: &mut CommentDrawer<'_, P>,
    text: &str,
    stop: &AtomicBool,
) -> anyhow::Result<CommentVerdict> {
    let Some(field) = drawer.open(stop).await? else {
        return Ok(CommentVerdict::NoDrawer);
    };
    if !drawer.focus_and_type(&field, text, stop).await? {
        return Ok(CommentVerdict::NoSendControl);
    }
    let Some(send) = drawer.await_armed(stop).await? else {
        return Ok(CommentVerdict::NotArmed);
    };
    Ok(if drawer.tap_send_and_confirm_disarm(&send, stop).await? {
        CommentVerdict::Sent
    } else {
        CommentVerdict::NotConfirmed
    })
}

/// Sleep unless the caller has asked to stop.
///
/// A local copy rather than reaching into `nurture`: this module must not depend on
/// the loop that used to own it.
async fn sleep(duration: Duration, stop: &AtomicBool) {
    let deadline = Instant::now() + duration;
    while Instant::now() < deadline {
        if stop.load(Ordering::Relaxed) {
            return;
        }
        let left = deadline.saturating_duration_since(Instant::now());
        tokio::time::sleep(left.min(Duration::from_millis(120))).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::tiktok_labels::controls_for;
    use crate::types::TapPoint;
    use parking_lot::Mutex;

    fn vietnamese() -> TikTokControls {
        controls_for("com.ss.android.ugc.trill", "vi", "46.3.3").expect("measured set")
    }

    fn english() -> TikTokControls {
        controls_for("com.zhiliaoapp.musically", "en", "").expect("measured set")
    }

    fn element(enabled: bool) -> ElementBox {
        ElementBox {
            x: 904.0,
            y: 1379.0,
            width: 143.0,
            height: 88.0,
            description: Some("@2131823284".into()),
            enabled,
        }
    }

    /// A session that answers from a script and records what was tapped.
    ///
    /// The point of this fake is the *state machine*: it is where a duplicate public
    /// comment would come from, so every verdict gets exercised without a phone.
    #[derive(Default)]
    struct FakeSession {
        /// Answers for `locate`, keyed by the query's value. Popped in order.
        answers: Mutex<Vec<(String, Option<ElementBox>)>>,
        /// Answers that persist once the queue is exhausted.
        ///
        /// Needed to express "the control stays there, unchanged". A queue alone
        /// cannot: once it empties the fake reports the element *absent*, and the
        /// flow correctly reads a vanished Send button as a closed drawer, i.e. sent.
        /// That is the code being right and the fixture being unable to say what it
        /// meant.
        sticky: Mutex<Vec<(String, ElementBox)>>,
        taps: Mutex<Vec<TapPoint>>,
        typed: Mutex<Vec<String>>,
        backs: Mutex<usize>,
        /// Make the typing step fail, so the error path can be exercised. A transport
        /// error here is the realistic one: the agent is reached, the drawer is open, and
        /// the request dies mid-gesture.
        typing_fails: bool,
    }

    impl FakeSession {
        fn with(answers: Vec<(&str, Option<ElementBox>)>) -> Self {
            Self {
                answers: Mutex::new(
                    answers
                        .into_iter()
                        .map(|(key, value)| (key.to_string(), value))
                        .collect(),
                ),
                ..Default::default()
            }
        }

        fn sticking(mut self, key: &str, element: ElementBox) -> Self {
            self.sticky = Mutex::new(vec![(key.to_string(), element)]);
            self
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
        async fn type_text(&self, text: &str) -> anyhow::Result<()> {
            if self.typing_fails {
                anyhow::bail!("agent went away mid-gesture");
            }
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
        async fn locate(&self, query: ElementQuery<'_>) -> anyhow::Result<Option<ElementBox>> {
            let wanted = match query {
                ElementQuery::Description { value, .. } => value,
                ElementQuery::ClassName(value) => value,
                ElementQuery::Text { value, .. } => value,
                // Registered by value like every other strategy, so a fixture can key on an id
                // suffix without this double having to model resource ids.
                ElementQuery::ResourceIdSuffix(value) => value,
            };
            let mut answers = self.answers.lock();
            if let Some(index) = answers.iter().position(|(key, _)| key == wanted) {
                return Ok(answers.remove(index).1);
            }
            drop(answers);
            let sticky = self.sticky.lock();
            Ok(sticky
                .iter()
                .find(|(key, _)| key == wanted)
                .map(|(_, element)| element.clone()))
        }
    }

    fn centre_planner() -> impl FnMut(&ElementBox) -> TapPoint {
        |element: &ElementBox| element.centre()
    }

    #[tokio::test(start_paused = true)]
    async fn an_unmeasured_send_control_refuses_before_opening_anything() {
        // The English set has no measured Send control. Opening the drawer anyway
        // would leave the phone in it with nothing to aim at.
        let session = FakeSession::default();
        let mut planner = centre_planner();
        let stop = AtomicBool::new(false);
        let verdict = post_comment(&session, english(), &mut planner, "hi", &stop)
            .await
            .expect("verdict");
        assert_eq!(verdict, CommentVerdict::SendUnmeasured);
        assert!(session.taps.lock().is_empty(), "nothing should be tapped");
    }

    #[tokio::test(start_paused = true)]
    async fn a_drawer_that_never_shows_a_field_is_not_a_comment() {
        let session = FakeSession::with(vec![("bình luận", Some(element(true)))]);
        let mut planner = centre_planner();
        let stop = AtomicBool::new(true); // stop immediately so the poll does not spin
        let verdict = post_comment(&session, vietnamese(), &mut planner, "hi", &stop)
            .await
            .expect("verdict");
        assert_eq!(verdict, CommentVerdict::NoDrawer);
        assert!(
            session.typed.lock().is_empty(),
            "must not type into nothing"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn text_is_never_typed_before_the_send_control_is_present() {
        // Opener and field answer, Send does not. Typing anyway would leave a draft
        // in somebody's comment box with no way to send or clear it.
        let session = FakeSession::with(vec![
            ("bình luận", Some(element(true))),
            ("android.widget.EditText", Some(element(true))),
        ]);
        let mut planner = centre_planner();
        let stop = AtomicBool::new(true);
        let verdict = post_comment(&session, vietnamese(), &mut planner, "hi", &stop)
            .await
            .expect("verdict");
        assert_eq!(verdict, CommentVerdict::NoSendControl);
        assert!(session.typed.lock().is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn a_send_control_that_never_arms_is_not_tapped() {
        // The Send button is there but stays disabled. Tapping a disabled button is
        // how a "sent" comment turns out never to have left.
        let session = FakeSession::with(vec![
            ("bình luận", Some(element(true))),
            ("android.widget.EditText", Some(element(true))),
            ("@2131823284", Some(element(false))), // present for focus_and_type
            ("@2131823284", Some(element(false))), // still not armed
        ]);
        let mut planner = centre_planner();
        let stop = AtomicBool::new(false);
        let verdict = post_comment(&session, vietnamese(), &mut planner, "hi", &stop)
            .await
            .expect("verdict");
        assert_eq!(verdict, CommentVerdict::NotArmed);
        assert_eq!(session.typed.lock().as_slice(), ["hi"]);
        // Two taps: the opener and the field. Not the Send button.
        assert_eq!(session.taps.lock().len(), 2);
    }

    #[tokio::test(start_paused = true)]
    async fn a_disarm_after_the_tap_is_what_proves_the_comment_left() {
        let session = FakeSession::with(vec![
            ("bình luận", Some(element(true))),
            ("android.widget.EditText", Some(element(true))),
            ("@2131823284", Some(element(false))), // focus_and_type presence check
            ("@2131823284", Some(element(true))),  // armed
            ("@2131823284", Some(element(false))), // disarmed after the tap
        ]);
        let mut planner = centre_planner();
        let stop = AtomicBool::new(false);
        let verdict = post_comment(&session, vietnamese(), &mut planner, "hay qua", &stop)
            .await
            .expect("verdict");
        assert_eq!(verdict, CommentVerdict::Sent);
        assert_eq!(session.typed.lock().as_slice(), ["hay qua"]);
        // Opener, field, Send.
        assert_eq!(session.taps.lock().len(), 3);
    }

    #[tokio::test(start_paused = true)]
    async fn a_send_whose_result_cannot_be_read_is_never_reported_as_sent() {
        // The genuinely ambiguous case: Send is tapped and then stays **present and
        // armed** for the whole window. A button that disappears instead is a closed
        // drawer, which the flow rightly counts as sent — so this needs a sticky
        // answer to hold the button in place.
        let session = FakeSession::with(vec![
            ("bình luận", Some(element(true))),
            ("android.widget.EditText", Some(element(true))),
        ])
        .sticking("@2131823284", element(true));
        let mut planner = centre_planner();
        let stop = AtomicBool::new(false);
        let verdict = post_comment(&session, vietnamese(), &mut planner, "hi", &stop)
            .await
            .expect("verdict");
        assert_eq!(verdict, CommentVerdict::NotConfirmed);
        assert!(!verdict.is_sent(), "ambiguous must never count as sent");
    }

    #[test]
    fn every_verdict_has_an_operator_facing_reason() {
        for verdict in [
            CommentVerdict::Sent,
            CommentVerdict::NotConfirmed,
            CommentVerdict::SendUnmeasured,
            CommentVerdict::NoDrawer,
            CommentVerdict::NoSendControl,
            CommentVerdict::NotArmed,
            CommentVerdict::ContextSkipped,
        ] {
            assert!(!verdict.reason().trim().is_empty(), "{verdict:?}");
            assert_eq!(verdict.is_sent(), verdict == CommentVerdict::Sent);
        }
    }

    #[tokio::test(start_paused = true)]
    async fn a_drawer_is_closed_even_when_the_gesture_that_failed_was_mid_comment() {
        // Every step between opening and sending can fail with `?`, and each of those
        // failures used to return straight out of `post_comment` without closing anything.
        // The phone is then standing inside an open comment list, and the feed loop's next
        // swipe scrolls comments instead of the feed — the same way a wandering interaction
        // used to cost a whole session.
        let session = FakeSession {
            typing_fails: true,
            ..FakeSession::with(vec![
                ("bình luận", Some(element(true))),
                ("android.widget.EditText", Some(element(true))),
                ("@2131823284", Some(element(false))), // present, so typing is attempted
            ])
        };
        let mut planner = centre_planner();
        let stop = AtomicBool::new(false);

        let outcome = post_comment(&session, vietnamese(), &mut planner, "chào", &stop).await;

        assert!(
            outcome.is_err(),
            "the failure itself must still be reported, not swallowed by the cleanup"
        );
        assert!(
            *session.backs.lock() > 0,
            "the drawer has to be closed on the way out, or the next feed swipe happens \
             inside the comment list"
        );
    }
}
