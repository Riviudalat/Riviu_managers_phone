//! The ordered ways back to the TikTok feed, in one place.
//!
//! A phone is left wherever the last session or the last person left it, and TikTok is
//! generous with places to leave it: a modal, an onboarding journey, another bottom tab, a
//! screen with no label on it at all. `nurture::hierarchy::await_feed` grew a ladder for
//! this one rung at a time, each rung measured on a real phone and each carrying the
//! reason it sits where it does.
//!
//! It lives here rather than there because there are now **two** callers. The session
//! ladder runs at the start of a nurture run; the idle sweeper runs the same ladder on
//! phones nobody is driving, which is what makes an onboarding page get cleared without
//! anyone starting a session. Two copies of "which button is safe to press, and in what
//! order" is exactly the drift this project has been bitten by before — a rung fixed in
//! one copy and not the other looks like a flaky phone, not like a bug.
//!
//! ## Why the order is the order
//!
//! 1. **[`TikTokControl::DialogDismiss`]** first, because a modal owns the whole
//!    accessibility tree. Measured 18/08/2026: a phone behind "Save login for next time?"
//!    dumped a single `content-desc` of `Dialog` — the feed tab, the Home tab and the
//!    journey's buttons were all equally invisible underneath it. Nothing below this rung
//!    can be *seen* until this one has fired, so probing them first only wastes round
//!    trips.
//! 2. **[`TikTokControl::JourneySkip`]**, the new-user journey's decline. Not a modal: it
//!    is a whole activity in front of the feed, so the rung above cannot reach it and
//!    Back does not close it. Two consecutive steps share the label, so it may fire twice.
//! 3. **[`TikTokControl::JourneyDone`]**, the journey's last page, which offers no decline
//!    at all. Below `JourneySkip` because it is only reachable once the two friend-sync
//!    steps are cleared, and probing it first would cost a round trip that cannot answer.
//! 4. **[`TikTokControl::HomeTab`]**, once. A phone parked on Profile, Shop or Inbox is one
//!    tap from the feed, but `FeedTab` is a tab *inside* the feed, so on any of those the
//!    loop sees nothing and waits out its whole window. Once rather than per poll: a phone
//!    that does not arrive after being sent there has something wrong that tapping again
//!    will not fix.
//! 5. **Back**, and only when every rung above has found nothing. This is the rung with no
//!    label, for the dialogs nobody has measured — measured itself on 18/08/2026, where
//!    "Get updates sent to your email?" dumped no `content-desc` anywhere and its only
//!    labelled button *accepted*. It is safe **only in this position**: Back on the feed
//!    leaves TikTok, and Back on another tab is worse than the Home tap above it. Both of
//!    those have already been ruled out by the time this is reached, which is the whole
//!    argument for its placement — see [`LadderSpend::allow_back`].
//!
//! ## What is deliberately not here
//!
//! No rung presses an affirmative. Every control above declines, dismisses or navigates,
//! and each was measured to change nothing on the account: `Not now` grants no permission,
//! `Skip` hands over no friend list, and `Done` was checked against the account's own
//! Following count rather than assumed (see [`TikTokControl::JourneyDone`]). A rung that
//! could say yes on a real account's behalf does not belong in an automatic ladder at all.

use crate::driver::UiSession;
use crate::tiktok_labels::{controls_for, TikTokControl, TikTokControls};
use crate::tiktok_target::is_measured_android_tiktok;

/// Why a phone cannot be put through the ladder right now.
///
/// A refusal is a value rather than an error string because every arm means something
/// different to the caller: two of them are "come back later", one is "this build needs
/// measuring", and one is "not this backend, ever".
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LadderRefusal {
    /// The backend cannot report element geometry — iOS, which locates by pixels instead
    /// and has [`crate::screen_watch`] for the same job.
    NoElementBounds,
    /// TikTok is not what is in front. **Nothing is touched in this state**, and that is
    /// the sweeper's central safety property rather than a nicety: the phone may be in
    /// Settings, in another account's app, or on a screen somebody left mid-task, and no
    /// rung here has been measured anywhere but inside TikTok.
    NotTikTok { package: String },
    /// TikTok, but on a build or in a language nobody has measured. Refuse rather than
    /// borrow another build's strings — the four phones that met an unmeasured dialog on
    /// 19/08/2026 are what that costs when it goes the other way.
    NoLabelSet {
        package: String,
        language: Option<String>,
    },
}

impl LadderRefusal {
    /// The operator-facing line, or `None` when this is not worth saying.
    ///
    /// [`Self::NotTikTok`] is silent on purpose: a phone doing something else is the
    /// normal state of an idle farm, and a line per device per sweep would bury the
    /// history this log exists to keep.
    pub fn says(&self) -> Option<String> {
        match self {
            Self::NoElementBounds | Self::NotTikTok { .. } => None,
            Self::NoLabelSet { package, language } => Some(format!(
                "chưa đo nhãn TikTok cho {package} + ngôn ngữ {} — không tự dọn popup máy này",
                language.as_deref().unwrap_or("không đọc được")
            )),
        }
    }
}

/// The measured labels for whatever TikTok build is in the foreground.
///
/// Reads the package off the screen rather than trusting configuration, for the reason
/// `nurture::hierarchy::HierarchyRun::prepare` gives: the regional builds differ
/// (`com.zhiliaoapp.musically` vs `com.ss.android.ugc.trill`) and their labels differ with
/// them, so the package actually in front is the one to key on. Unlike that function this
/// one has no configured bundle to fall back on, which is the right difference — a sweeper
/// that cannot read the foreground must do nothing rather than assume.
pub async fn foreground_labels(session: &dyn UiSession) -> Result<TikTokControls, LadderRefusal> {
    if !session.supports_element_bounds() {
        return Err(LadderRefusal::NoElementBounds);
    }
    let package = session.active_app_bundle().await.unwrap_or_default();
    if !is_measured_android_tiktok(&package) {
        return Err(LadderRefusal::NotTikTok { package });
    }
    let language = session.ui_language().await;
    let app_version = session.app_version(&package).await.unwrap_or_default();
    language
        .as_deref()
        .and_then(|language| controls_for(&package, language, &app_version))
        .ok_or(LadderRefusal::NoLabelSet { package, language })
}

/// One rung: a measured control, and what the operator is told when it fires.
#[derive(Debug, Clone, Copy)]
pub struct LadderRung {
    pub control: TikTokControl,
    /// The operator-facing line, in Vietnamese like the rest of the status stream.
    pub says: &'static str,
    /// Whether this rung may fire more than once while working back to the feed.
    ///
    /// `true` for the rungs that legitimately repeat — TikTok stacks modals, and the
    /// journey's decline clears a page and then its confirmation dialog. `false` for the
    /// Home tap, which is a navigation and not a dismissal.
    pub repeatable: bool,
}

/// The ladder, in the order it must be tried. Back is not a rung — see [`step`].
pub const FEED_LADDER: [LadderRung; 4] = [
    LadderRung {
        control: TikTokControl::DialogDismiss,
        says: "đóng hộp thoại TikTok chắn feed",
        repeatable: true,
    },
    LadderRung {
        control: TikTokControl::JourneySkip,
        says: "bỏ qua trang mời kết bạn của TikTok",
        repeatable: true,
    },
    LadderRung {
        control: TikTokControl::JourneyDone,
        says: "đóng trang gợi ý follow (không follow ai)",
        repeatable: true,
    },
    LadderRung {
        control: TikTokControl::HomeTab,
        says: "TikTok đang ở tab khác — bấm Home để về feed",
        repeatable: false,
    },
];

/// What one caller has already spent on getting this phone back.
///
/// Carried by the caller rather than owned here, because the two callers budget
/// differently: a session has a thirty-second window it may spend freely, and the idle
/// sweeper gets a couple of taps per visit and then leaves the phone alone until the next
/// sweep.
#[derive(Debug, Clone, Default)]
pub struct LadderSpend {
    /// Non-repeatable rungs already fired, by control ordinal position in [`FEED_LADDER`].
    fired: [bool; FEED_LADDER.len()],
    /// Back presses spent.
    pub backs: u32,
    /// How many Back presses this caller allows in total.
    pub back_limit: u32,
    /// Whether Back may be pressed *yet*.
    ///
    /// The caller's clock, not this module's. `await_feed` holds Back for ten seconds so a
    /// slow splash screen is not answered with a keypress; the idle sweeper holds it until
    /// it has seen the same unlabelled screen on two separate visits. Both are policies
    /// about *patience*, which is the caller's to set — the safety argument for Back's
    /// position in the order is this module's, and that one is not negotiable.
    pub allow_back: bool,
}

impl LadderSpend {
    /// A spend that allows `back_limit` Back presses, none of them yet.
    pub fn new(back_limit: u32) -> Self {
        Self {
            fired: [false; FEED_LADDER.len()],
            backs: 0,
            back_limit,
            allow_back: false,
        }
    }

    fn already_fired(&self, index: usize) -> bool {
        self.fired.get(index).copied().unwrap_or(false)
    }

    fn mark_fired(&mut self, index: usize) {
        if let Some(slot) = self.fired.get_mut(index) {
            *slot = true;
        }
    }

    fn backs_left(&self) -> bool {
        self.backs < self.back_limit
    }
}

/// What one pass over the ladder did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LadderStep {
    /// The feed tab is on screen. Nothing to do, and nothing was touched.
    OnFeed,
    /// A rung fired and its tap was accepted.
    Tapped {
        control: TikTokControl,
        says: &'static str,
    },
    /// A rung was found but the tap was refused. Reported rather than retried here: the
    /// caller owns the retry budget, and a refused tap is not evidence about the screen.
    TapFailed {
        control: TikTokControl,
        error: String,
    },
    /// Nothing on screen carries a label this build has measured, and Back was pressed.
    PressedBack { spent: u32, limit: u32 },
    /// Nothing matched and Back is either not allowed yet or used up. The caller waits.
    Stuck,
}

impl LadderStep {
    /// The operator-facing line for this step, or `None` when there is nothing to say.
    pub fn says(&self) -> Option<String> {
        match self {
            Self::OnFeed => None,
            Self::Tapped { says, .. } => Some((*says).to_string()),
            Self::TapFailed { control, error } => {
                Some(format!("bấm {control:?} không được: {error}"))
            }
            Self::PressedBack { spent, limit } => Some(format!(
                "màn hình bị chặn và không có nút nào đọc được — bấm Back ({spent}/{limit})"
            )),
            Self::Stuck => None,
        }
    }

    /// Whether the screen was touched. A caller that acted should wait before re-probing.
    pub fn acted(&self) -> bool {
        matches!(
            self,
            Self::Tapped { .. } | Self::TapFailed { .. } | Self::PressedBack { .. }
        )
    }
}

/// Probe the rungs in order and act on the first one that answers.
///
/// **Lazy on purpose.** Every probe is a round trip, and under a playing feed the measured
/// cost is 90–170 ms for geometry and up to ten seconds for a query the server has to wait
/// out (`docs/ANDROID_PROBE_REPORT_2026-08-09.md`). A version of this that collected the
/// whole screen state first and then decided would be tidier and would pay for four probes
/// on every poll where the first one already answered.
pub async fn step(
    session: &dyn UiSession,
    labels: TikTokControls,
    spend: &mut LadderSpend,
) -> LadderStep {
    if present(session, labels, TikTokControl::FeedTab).await {
        return LadderStep::OnFeed;
    }

    for (index, rung) in FEED_LADDER.iter().enumerate() {
        if !rung.repeatable && spend.already_fired(index) {
            continue;
        }
        let Some(element) = locate(session, labels, rung.control).await else {
            continue;
        };
        spend.mark_fired(index);
        return match session.tap(element).await {
            Ok(()) => LadderStep::Tapped {
                control: rung.control,
                says: rung.says,
            },
            Err(error) => LadderStep::TapFailed {
                control: rung.control,
                error: error.to_string(),
            },
        };
    }

    if spend.allow_back && spend.backs_left() {
        spend.backs += 1;
        // A refused Back is not worth its own arm: there is no second way to press it and
        // nothing the caller would do differently, so the count is spent either way and
        // the next poll re-probes the same screen.
        let _ = session.back().await;
        return LadderStep::PressedBack {
            spent: spend.backs,
            limit: spend.back_limit,
        };
    }

    LadderStep::Stuck
}

/// Whether TikTok's feed tab is on screen.
///
/// Public because the session ladder needs to ask **without acting** once its window has
/// expired: a tap it is about to abandon leaves the phone mid-transition for whoever
/// comes next. [`step`] answers the same question on its way past, so nothing should call
/// both in one pass.
pub async fn on_feed(session: &dyn UiSession, labels: TikTokControls) -> bool {
    present(session, labels, TikTokControl::FeedTab).await
}

/// The centre of a measured, on-screen control, or `None`.
async fn locate(
    session: &dyn UiSession,
    labels: TikTokControls,
    control: TikTokControl,
) -> Option<crate::types::TapPoint> {
    let label = labels.label(control)?;
    session
        .locate(label.to_query())
        .await
        .ok()
        .flatten()
        .map(|element| element.centre())
}

async fn present(session: &dyn UiSession, labels: TikTokControls, control: TikTokControl) -> bool {
    locate(session, labels, control).await.is_some()
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;

    use super::*;
    use crate::driver::{ElementBox, ElementQuery};
    use crate::tiktok_labels::controls_for;
    use crate::types::{SwipeGesture, TapPoint};

    fn measured() -> TikTokControls {
        controls_for("com.ss.android.ugc.trill", "en", "38.3.2").expect("measured set")
    }

    /// A phone that shows exactly the labels it is told to, and records what was tapped.
    #[derive(Default)]
    struct FakePhone {
        /// Text labels currently on screen.
        text: Mutex<Vec<&'static str>>,
        /// Description labels currently on screen.
        desc: Mutex<Vec<&'static str>>,
        tapped: Mutex<Vec<TapPoint>>,
        backs: AtomicUsize,
        refuse_taps: bool,
    }

    impl FakePhone {
        fn showing_text(labels: &[&'static str]) -> Self {
            Self {
                text: Mutex::new(labels.to_vec()),
                ..Default::default()
            }
        }

        fn showing_desc(labels: &[&'static str]) -> Self {
            Self {
                desc: Mutex::new(labels.to_vec()),
                ..Default::default()
            }
        }

        fn taps(&self) -> usize {
            self.tapped.lock().expect("taps").len()
        }
    }

    #[async_trait::async_trait]
    impl UiSession for FakePhone {
        async fn tap(&self, point: TapPoint) -> anyhow::Result<()> {
            if self.refuse_taps {
                anyhow::bail!("máy từ chối");
            }
            self.tapped.lock().expect("taps").push(point);
            Ok(())
        }

        async fn swipe(&self, _gesture: SwipeGesture) -> anyhow::Result<()> {
            Ok(())
        }

        async fn type_text(&self, _text: &str) -> anyhow::Result<()> {
            Ok(())
        }

        async fn home(&self) -> anyhow::Result<()> {
            Ok(())
        }

        async fn back(&self) -> anyhow::Result<()> {
            self.backs.fetch_add(1, Ordering::Relaxed);
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

        async fn locate(&self, query: ElementQuery<'_>) -> anyhow::Result<Option<ElementBox>> {
            let found = match query {
                ElementQuery::Text { value, .. } => {
                    self.text.lock().expect("text").contains(&value)
                }
                ElementQuery::Description { value, .. } => {
                    self.desc.lock().expect("desc").contains(&value)
                }
                ElementQuery::ClassName(_) => false,
            };
            Ok(found.then_some(ElementBox {
                x: 100.0,
                y: 1_900.0,
                width: 400.0,
                height: 100.0,
                description: None,
                enabled: true,
            }))
        }
    }

    #[tokio::test]
    async fn a_phone_on_the_feed_is_left_alone() {
        let phone = FakePhone::showing_desc(&["For You"]);
        let mut spend = LadderSpend::new(1);
        spend.allow_back = true;
        assert_eq!(
            step(&phone, measured(), &mut spend).await,
            LadderStep::OnFeed
        );
        assert_eq!(phone.taps(), 0, "nothing was touched");
        assert_eq!(phone.backs.load(Ordering::Relaxed), 0, "and no Back");
    }

    /// The case this whole change exists for: six phones sat on this page and no rung
    /// could reach them, because `Not now` is not on it.
    #[tokio::test]
    async fn the_friend_sync_page_is_skipped() {
        let phone = FakePhone::showing_text(&["Skip", "Sync"]);
        let mut spend = LadderSpend::new(1);
        let step = step(&phone, measured(), &mut spend).await;
        assert_eq!(
            step,
            LadderStep::Tapped {
                control: TikTokControl::JourneySkip,
                says: "bỏ qua trang mời kết bạn của TikTok",
            }
        );
        assert_eq!(phone.taps(), 1);
    }

    /// The confirmation dialog carries the same label, so one rung clears both steps —
    /// and it must be allowed to fire twice to do it.
    #[tokio::test]
    async fn the_skip_confirmation_is_cleared_by_the_same_rung() {
        let phone = FakePhone::showing_text(&["Skip", "Find friends"]);
        let mut spend = LadderSpend::new(1);
        for _ in 0..2 {
            assert!(matches!(
                step(&phone, measured(), &mut spend).await,
                LadderStep::Tapped {
                    control: TikTokControl::JourneySkip,
                    ..
                }
            ));
        }
        assert_eq!(phone.taps(), 2, "repeatable rungs fire again");
    }

    #[tokio::test]
    async fn the_suggested_follow_page_is_closed_with_done() {
        let phone = FakePhone::showing_text(&["Done"]);
        let mut spend = LadderSpend::new(1);
        assert!(matches!(
            step(&phone, measured(), &mut spend).await,
            LadderStep::Tapped {
                control: TikTokControl::JourneyDone,
                ..
            }
        ));
    }

    /// Ordering, stated as a test rather than as a comment: a modal hides everything, so
    /// when both are somehow visible the modal goes first.
    #[tokio::test]
    async fn a_modal_is_declined_before_the_journey_is_skipped() {
        let phone = FakePhone::showing_text(&["Not now", "Skip"]);
        let mut spend = LadderSpend::new(1);
        assert!(matches!(
            step(&phone, measured(), &mut spend).await,
            LadderStep::Tapped {
                control: TikTokControl::DialogDismiss,
                ..
            }
        ));
    }

    /// The journey outranks the Home tab. Both can be on screen — the bottom bar is drawn
    /// behind some journey pages — and tapping Home there does not leave the journey.
    #[tokio::test]
    async fn the_journey_outranks_the_home_tab() {
        let phone = FakePhone {
            text: Mutex::new(vec!["Skip"]),
            desc: Mutex::new(vec!["Home"]),
            ..Default::default()
        };
        let mut spend = LadderSpend::new(1);
        assert!(matches!(
            step(&phone, measured(), &mut spend).await,
            LadderStep::Tapped {
                control: TikTokControl::JourneySkip,
                ..
            }
        ));
    }

    #[tokio::test]
    async fn a_phone_parked_on_another_tab_is_nudged_home_once() {
        let phone = FakePhone::showing_desc(&["Home"]);
        let mut spend = LadderSpend::new(2);
        spend.allow_back = true;
        assert!(matches!(
            step(&phone, measured(), &mut spend).await,
            LadderStep::Tapped {
                control: TikTokControl::HomeTab,
                ..
            }
        ));
        // Second visit: the tab is still there (the tap did not work), and the ladder must
        // not keep tapping it — it falls through to Back instead.
        assert_eq!(
            step(&phone, measured(), &mut spend).await,
            LadderStep::PressedBack { spent: 1, limit: 2 }
        );
        assert_eq!(phone.taps(), 1, "Home was tapped once, not twice");
    }

    /// Back is the rung with no label, and it must stay last. A phone showing *nothing*
    /// gets it; a phone showing anything measured does not.
    #[tokio::test]
    async fn back_is_only_reached_when_no_rung_matched() {
        let blank = FakePhone::default();
        let mut spend = LadderSpend::new(1);
        spend.allow_back = true;
        assert_eq!(
            step(&blank, measured(), &mut spend).await,
            LadderStep::PressedBack { spent: 1, limit: 1 }
        );
        assert_eq!(blank.backs.load(Ordering::Relaxed), 1);
    }

    #[tokio::test]
    async fn back_is_withheld_until_the_caller_allows_it() {
        let blank = FakePhone::default();
        let mut spend = LadderSpend::new(1);
        assert_eq!(
            step(&blank, measured(), &mut spend).await,
            LadderStep::Stuck
        );
        assert_eq!(
            blank.backs.load(Ordering::Relaxed),
            0,
            "patience is the caller's to spend"
        );
    }

    #[tokio::test]
    async fn back_stops_at_its_limit() {
        let blank = FakePhone::default();
        let mut spend = LadderSpend::new(1);
        spend.allow_back = true;
        assert!(matches!(
            step(&blank, measured(), &mut spend).await,
            LadderStep::PressedBack { .. }
        ));
        assert_eq!(
            step(&blank, measured(), &mut spend).await,
            LadderStep::Stuck
        );
        assert_eq!(blank.backs.load(Ordering::Relaxed), 1);
    }

    /// A build with no measured journey labels must fall through the rungs it cannot see
    /// rather than refusing the whole ladder — the modal decline still works there.
    #[tokio::test]
    async fn an_unmeasured_build_still_gets_the_rungs_it_does_have() {
        let labels = controls_for("com.zhiliaoapp.musically", "en", "46.2.1")
            .expect("measured set without journey labels");
        let phone = FakePhone::showing_text(&["Skip"]);
        let mut spend = LadderSpend::new(1);
        assert_eq!(step(&phone, labels, &mut spend).await, LadderStep::Stuck);
        assert_eq!(
            phone.taps(),
            0,
            "an unmeasured label is not tapped by borrowing another build's string"
        );
    }

    #[tokio::test]
    async fn a_refused_tap_is_reported_rather_than_swallowed() {
        let phone = FakePhone {
            text: Mutex::new(vec!["Skip"]),
            refuse_taps: true,
            ..Default::default()
        };
        let mut spend = LadderSpend::new(1);
        let step = step(&phone, measured(), &mut spend).await;
        assert!(matches!(step, LadderStep::TapFailed { .. }));
        assert!(step.says().is_some_and(|line| line.contains("máy từ chối")));
    }

    /// No rung may press a button that grants something. Guards against a future edit
    /// adding `Sync`, `Find friends` or `Follow` to the ladder.
    #[test]
    fn no_rung_can_accept_on_the_accounts_behalf() {
        for rung in FEED_LADDER {
            assert!(
                matches!(
                    rung.control,
                    TikTokControl::DialogDismiss
                        | TikTokControl::JourneySkip
                        | TikTokControl::JourneyDone
                        | TikTokControl::HomeTab
                ),
                "{:?} is not one of the four controls measured as safe to press \
                 unattended — see this module's docs before adding a rung",
                rung.control
            );
        }
    }

    /// A phone whose foreground is not TikTok must be refused *before* any rung is probed
    /// — this is the sweeper's central safety property, so it gets a test and not a
    /// comment. `FakePhone` answers `None` to `active_app_bundle` by trait default.
    #[tokio::test]
    async fn a_phone_outside_tiktok_is_refused_without_being_touched() {
        let phone = FakePhone::showing_text(&["Skip"]);
        match foreground_labels(&phone).await {
            Err(LadderRefusal::NoElementBounds) => {}
            other => panic!("a backend with no element bounds must refuse first: {other:?}"),
        }
        assert_eq!(phone.taps(), 0);
    }

    #[test]
    fn only_the_unmeasured_build_refusal_is_worth_saying_out_loud() {
        assert!(LadderRefusal::NoElementBounds.says().is_none());
        assert!(LadderRefusal::NotTikTok {
            package: "com.android.settings".into()
        }
        .says()
        .is_none());
        let noisy = LadderRefusal::NoLabelSet {
            package: "com.zhiliaoapp.musically".into(),
            language: Some("th".into()),
        };
        assert!(noisy.says().is_some_and(|line| line.contains("chưa đo")));
    }

    /// The measured decline is an exact match, and it has to stay one: the confirmation
    /// dialog's own *title* contains the word.
    #[test]
    fn the_journey_decline_cannot_match_the_dialog_title() {
        let label = measured()
            .label(TikTokControl::JourneySkip)
            .expect("measured");
        assert!(
            !"Skip finding Facebook friends?".eq(label.value()),
            "sanity: the label is the button, not the title"
        );
        assert!(
            matches!(label, crate::tiktok_labels::LabelMatch::Text("Skip")),
            "exact text match — a substring would return the title TextView instead: {label:?}"
        );
    }
}
