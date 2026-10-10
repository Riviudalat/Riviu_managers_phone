//! Nurture for backends that can report where a control is.
//!
//! The iOS engine in [`super`] finds controls by pixels because it has to:
//! `snapshotMaxDepth` is pinned at 1 on that transport (AGENTS.md 2.3), so the
//! accessibility tree is not readable and a calibrated screen layout is the only
//! way to know where the like heart sits. Android is the opposite — it answers
//! hierarchy queries — and AGENTS.md §9 says plainly not to port `screen.rs`
//! there. This module is what that instruction implies: the same session policy,
//! a different way of seeing.
//!
//! What is deliberately shared with the pixel engine, not re-decided here:
//! [`HumanBehavior`] (dwell, swipe duration, fatigue), [`HumanSessionPolicy`]
//! (per-post and per-session caps, action gaps, rests), [`MoodCycle`], the action
//! roll, and [`TouchPointPlanner`] for jitter. Only observation and the proof of
//! each action are new. Duplicating the pacing would have let the two backends
//! drift into behaving like two different users.
//!
//! One property falls out of locating before tapping, and it is worth naming
//! because the pixel engine had to work for it: **every tap here comes from a
//! rectangle the device reported**, so a card this loop does not understand
//! produces no tap at all. There is no equivalent of tapping a rail position that
//! turned out not to exist — the position comes from the rail or the action is
//! skipped.
//!
//! What it refuses rather than fakes:
//!
//! * an app build / UI language with no measured label set — [`crate::tiktok_labels::controls_for`]
//!   returns `None` and the session stops, the same fail-closed rule
//!   `screen::CALIBRATED_LAYOUTS` applies to uncalibrated screens;
//! * commenting, on any build whose Send control has not been measured. The
//!   session says so once and keeps running its other actions instead of
//!   pretending a comment went out.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

// Settings an operator changed after this session started, and the two objects that have
// to be told. Both feed loops go through the same function; [`super::live`] says why that
// is load-bearing rather than tidiness.
use super::live::{apply_live_settings, video_target, LiveSettings, LiveSettingsRefresh};
use crate::db::Database;
use crate::driver::{ElementBox, ElementQuery, UiSession};
use crate::feed_ladder::{self, LadderSpend, LadderStep};
use crate::human_behavior::{
    in_night_window, roll_bool, roll_nurture_actions, AttemptReservation, FeedAction,
    HumanBehavior, HumanSessionPolicy, MoodCycle, PolicyAction,
};
use crate::tiktok_drawer::CommentVerdict;
use crate::tiktok_follow_cleanup::{
    prove_nurture_follow_source, readback_nurture_follow_source, NurtureFollowReadbackVerdict,
};
use crate::tiktok_labels::{TikTokControl, TikTokControls};
use crate::tiktok_like::LikeVerdict;
use crate::tiktok_save::{
    hierarchy_save_observation, tiktok_save, SaveAdapter, SaveCardIdentity, SaveEvidence,
    SaveObservation,
};
use crate::types::{
    NurtureCommentAttempt, NurturePhase, NurtureSessionStatus, NurtureSettings, TapPoint,
};
use crate::ui_automation::runtime::{read_before_deadline, ReadWaitResult};
use crate::ActionFailure;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use super::recovery::Outcome;
use super::touch::TouchPointPlanner;
use super::{sleep_interruptible, NurtureFollowJournal, NurtureSaveJournal, NurtureSaveLease};

#[cfg(test)]
mod review_stop_during_save {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    struct Phone {
        stop: AtomicBool,
        taps: AtomicUsize,
    }
    #[async_trait::async_trait]
    impl UiSession for Phone {
        async fn tap(&self, _: TapPoint) -> anyhow::Result<()> {
            assert!(
                self.stop.load(Ordering::Acquire),
                "fixture must stop before tap"
            );
            self.taps.fetch_add(1, Ordering::Relaxed);
            Ok(())
        }
        async fn swipe(&self, _: crate::SwipeGesture) -> anyhow::Result<()> {
            panic!("unexpected swipe")
        }
        async fn type_text(&self, _: &str) -> anyhow::Result<()> {
            panic!("unexpected typing")
        }
        async fn home(&self) -> anyhow::Result<()> {
            panic!("unexpected home")
        }
        async fn find_and_tap(&self, _: &str) -> anyhow::Result<()> {
            panic!("unexpected tap")
        }
        async fn assert_visible(&self, _: &str) -> anyhow::Result<()> {
            Ok(())
        }
        fn stream_url(&self) -> Option<String> {
            None
        }
        async fn locate(&self, query: ElementQuery<'_>) -> anyhow::Result<Option<ElementBox>> {
            let ElementQuery::Description { value, .. } = query else {
                return Ok(None);
            };
            let description = match value {
                " profile" => "author_a profile",
                "Sound:" => "Sound: fixture",
                "comments" => "Read or add comments. 12 comments",
                "Share video" => "Share video. 3 shares",
                _ => return Ok(None),
            };
            Ok(Some(ElementBox {
                x: 10.0,
                y: 10.0,
                width: 10.0,
                height: 10.0,
                enabled: true,
                clickable: true,
                description: Some(description.into()),
            }))
        }
        async fn hierarchy_source_snapshot(
            &self,
        ) -> anyhow::Result<crate::HierarchySourceSnapshot> {
            // Operator presses Stop while the first observation is in progress.
            self.stop.store(true, Ordering::Release);
            Ok(crate::HierarchySourceSnapshot {
                generation: 1,
                xml: if self.taps.load(Ordering::Relaxed) > 0 {
                    include_str!("../../../../fixtures/tiktok/bookmark-trill-38.3.2-en-saved.xml")
                } else {
                    include_str!("../../../../fixtures/tiktok/bookmark-trill-38.3.2-en-unsaved.xml")
                }
                .into(),
            })
        }
    }
    struct Journal(AtomicUsize);
    impl NurtureSaveJournal for Journal {
        fn arm(&self, _: &str, _: &SaveObservation) -> anyhow::Result<NurtureSaveLease> {
            self.0.fetch_add(1, Ordering::Relaxed);
            Ok(NurtureSaveLease {
                owner: crate::TikTokActionOwner {
                    kind: crate::TikTokActionOwnerKind::Nurture,
                    owner_id: "review".into(),
                    device_udid: "fixture".into(),
                    card_identity: None,
                },
                armed_revision: 1,
            })
        }
        fn settle(&self, _: Option<NurtureSaveLease>, _: &SaveEvidence) -> anyhow::Result<()> {
            Ok(())
        }
    }
    #[tokio::test]
    async fn regression_nurture_save_cancels_before_intent() {
        let phone = Phone {
            stop: AtomicBool::new(false),
            taps: AtomicUsize::new(0),
        };
        let journal = Journal(AtomicUsize::new(0));
        let mut run = HierarchyRun {
            session: &phone,
            labels: crate::tiktok_labels::controls_for("com.ss.android.ugc.trill", "en", "38.3.2")
                .unwrap(),
            screen: (1080.0, 2220.0),
            planner: TouchPointPlanner::new((1080.0, 2220.0)),
            search_keyword: None,
        };
        assert!(wait_gap(&mut None, Duration::ZERO, &phone.stop).await);
        let (evidence, _) = run.save(Some(&journal), "card-0", &phone.stop).await;
        println!(
            "stop={} journal_armed={} taps={} verdict={:?}",
            phone.stop.load(Ordering::Relaxed),
            journal.0.load(Ordering::Relaxed),
            phone.taps.load(Ordering::Relaxed),
            evidence.verdict
        );
        assert!(phone.stop.load(Ordering::Relaxed));
        assert_eq!(journal.0.load(Ordering::Relaxed), 0);
        assert_eq!(phone.taps.load(Ordering::Relaxed), 0);
        assert!(!evidence.effect_boundary_crossed);
    }
}

/// How many consecutive cards may lack the feed tab before the loop gives up.
///
/// Same intent as the pixel engine's off-feed limit: a few cards can legitimately
/// be mid-transition, but a run that never sees the feed again is stuck somewhere
/// blind taps would only make worse.
const OFF_FEED_LIMIT: u32 = 6;

/// How many consecutive swipes may fail to change the post before stopping.
const STUCK_SWIPE_LIMIT: u32 = 4;

/// Unproven swipes before TikTok is restarted — **below** the give-up limit, deliberately.
///
/// The feed loop then ran exactly `num_videos` passes (now at most half as many again), so a
/// session asked for two videos could never reach a counter of four. Measured
/// 18/08/2026 on the fleet, where every stuck phone ended on `2/4` and the recovery
/// behind that four could not have fired even if it had existed. Two is the smallest
/// number that is still evidence rather than one disappointment.
const STUCK_RESTART_AFTER: u32 = 2;

/// Minimum settle time after a swipe before the first hierarchy read.
///
/// The tree is queryable immediately but reports the outgoing card for a beat, so
/// reading too early fingerprints the post we just left and the swipe looks stuck.
const SWIPE_SETTLE: Duration = Duration::from_millis(700);

/// How long to keep waiting for the incoming card's rail after the settle.
///
/// Measured, not guessed. With a single read at 900 ms a real 300-second run
/// reported 6 of its 34 swipes as unproved, and every one was followed by a card
/// that appeared to have no action rail — the same mid-transition frame seen
/// twice. Waiting for the rail to come back turns both readings into one correct
/// one.
const RAIL_RETURN_WINDOW: Duration = Duration::from_millis(2_600);
const RAIL_RETURN_POLL: Duration = Duration::from_millis(300);

/// How long a railless card that directly follows an unproven swipe gets to draw its rail
/// before it is swiped past.
///
/// Observed 09/10/2026 on box 31–50 (`ce0317139ca344520c`, `ce071827715f321b05`,
/// `ce011711c354be2005`, `ce031713b064841e0c` and `ce0417141d4decde0c`; Galaxy S8+,
/// `com.zhiliaoapp.musically` / en): fourteen times a swipe stayed unproven through
/// [`SWIPE_SETTLE`] + [`RAIL_RETURN_WINDOW`], and the next pass, about 2.2 s later, found
/// the feed tab but no rail. It swiped that card away unwatched, the swipe after it was
/// proven, and one card cost two passes. Five of six phones spent all 15 swipes and ended on
/// 8–11 videos. The logs only show the rail still missing at least 5.5 s after the swipe.
/// The 6 s bound is the round-2 fix plan's figure, not a measured return time. A card that
/// has still not drawn by then is swiped past as before.
const LATE_RAIL_WINDOW: Duration = Duration::from_millis(6_000);

/// Read `(current, total)` out of a photo post's page counter.
///
/// The counter is **three adjacent nodes** — `"1"`, `" / "`, `"5"` — not one string, which
/// is why an earlier search for a node containing a slash concluded there was no counter
/// and left the Android carousel unimplemented. Free function so the measured strings can
/// be asserted without a phone.
///
/// Both neighbours must parse as whole numbers, which is what keeps an unrelated slash from
/// matching: a date renders as one node (`"12/08"`), never as a bare `"/"` between two
/// numeric nodes.
fn parse_carousel_counter(texts: &[String]) -> Option<(u32, u32)> {
    texts.windows(3).find_map(|window| {
        if window[1].trim() != "/" {
            return None;
        }
        let current = window[0].trim().parse::<u32>().ok()?;
        let total = window[2].trim().parse::<u32>().ok()?;
        (current >= 1 && total >= current).then_some((current, total))
    })
}

/// How many images of a photo post to view.
///
/// Rounded **up**, so half of a three-image post is two: an operator asking for half of an
/// odd post means "most of the way", not "less than half". Never above the post's own
/// total — a percentage cannot invent images — and never below one, because the card is
/// already showing its first image before any swipe happens.
fn carousel_target(total: u32, portion_percent: u32, ceiling: u32) -> u32 {
    let wanted = (u64::from(total) * u64::from(portion_percent.min(100))).div_ceil(100) as u32;
    wanted.clamp(1, total).min(ceiling.max(1))
}

/// How long one screen capture used as page-turn evidence may take.
///
/// The capture is `screencap` over adb (the driver's own ceiling is 60 s), and it is only
/// asked for on a photo post whose counter never renders. A capture that outlives this is
/// "could not tell", which ends the traversal: a turn nobody could see is never counted, and
/// a traversal must never wait a minute on one picture while the card sits still.
const CAROUSEL_FRAME_BUDGET: Duration = Duration::from_secs(4);

/// How many consecutive railless passes the loop may spend before it stops.
///
/// Box 31–50 acceptance round 4, 10/10/2026 (`ce031713b064841e0c`, Galaxy S8+,
/// `com.zhiliaoapp.musically` / en): after one unproven swipe the feed tab stayed on screen
/// and no card showed a rail again — twenty passes of `thẻ không có thanh hành động … chỉ
/// vuốt tiếp` until the whole pass budget was gone, `3/15`. A LIVE card or a card still
/// drawing is one railless pass, two at most; three in a row is a screen this loop does
/// not understand, and the honest answer is to say so rather than swipe it blind.
const NO_RAIL_STREAK_LIMIT: u32 = 3;

/// How many consecutive railless passes the loop may spend **on a proven feed** before it
/// stops.
///
/// Box 31–50 acceptance round 5, 10/10/2026 (`ce031713b064841e0c`, Galaxy S8+,
/// `com.zhiliaoapp.musically` / en, 01:16:29–01:16:55): [`NO_RAIL_STREAK_LIMIT`] fired on a
/// card whose rail was hidden by an ad end-card while the For You tab and the Home tab were
/// on screen the whole time — the feed *was* proven, the message said it was not, and the
/// phone stopped at 5/15. A railless card on a proven feed is an ad end-card or a LIVE card:
/// it is left by swiping, and by one Close on the end-card. Twice the other limit, because
/// the swipe and the Close each get a pass; a feed that is still on a railless card after
/// this many is reported rather than swiped blind.
const NO_RAIL_ON_FEED_LIMIT: u32 = 6;

/// How long the feed tab gets to come back after the one Back this loop presses itself.
///
/// Back out of the author's profile is a single activity pop. The ladder in
/// [`await_feed`] waits [`MODAL_BACK_DELAY`] before it will press Back at all, and that is
/// right for a splash screen; measured on box 31–50 round 3, 09/10/2026
/// (`ce0317139ca344520c` 21:39:13–21:39:28), it cost fifteen seconds per visit to a
/// profile page the loop had walked onto itself. Four seconds covers the pop and the
/// feed's first draw, and a feed that is not back by then goes to that ladder as before.
const BACK_RETURN_WINDOW: Duration = Duration::from_secs(4);
const BACK_RETURN_POLL: Duration = Duration::from_millis(500);

/// How long TikTok gets to reach the foreground after being launched here.
///
/// **The same measurement that corrected `FOREGROUND_PROOF_TIMEOUT` in the Android driver,
/// applied to the second place that waits for the same thing.** Cold starts on an
/// SM-N950F took 15,9 / 19,7 / 19,4 s from `am force-stop`, and 26,9 s once. This waited
/// 10 × 800 ms = 8 s, so a session started against a closed TikTok refused with
/// `đã gọi mở … nhưng nó không lên foreground` while the app was opening perfectly well —
/// reproduced through the G2 gate on 12/08/2026.
///
/// Through the app this loop usually returns before the first poll, because
/// `start_interaction_session` has already foregrounded and proved the app. This window is
/// for every other caller: the gate example, and any path that has not.
const FOREGROUND_WINDOW: Duration = Duration::from_secs(40);
const FOREGROUND_POLL: Duration = Duration::from_millis(800);

/// How long a sideways swipe gets to land before the counter is read again.
///
/// The measurement loop used 900 ms and never once read a stale counter across seven
/// swipes on two posts; 700 ms is what a vertical feed swipe already waits
/// ([`SWIPE_SETTLE`]) and a page turn inside a card is a smaller animation than a card
/// change. Kept at the measured 900 ms rather than the smaller number that was not tried.
const CAROUSEL_SETTLE: Duration = Duration::from_millis(900);

/// What a post looks like to a hierarchy reader, used only to tell two posts
/// apart.
///
/// Comment and share labels embed their own counts (`… 697 bình luận`,
/// `… 45,4K lượt chia sẻ`), so the pair changes from card to card. Two adjacent
/// posts *could* carry identical counts, which is why an unchanged fingerprint
/// means "not proved to have advanced" rather than "definitely did not" — the
/// same honesty the pixel engine's `SwipeOutcome` keeps.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct PostFingerprint {
    /// Stable author link beside the card. Counts can change after our own comment; the
    /// author is therefore the proof a later follow still targets the card we selected.
    author: Option<String>,
    comments: Option<String>,
    share: Option<String>,
    /// The sound strip's whole description, which is the only part of this that reliably
    /// differs between two ordinary posts.
    ///
    /// **Measured 18/08/2026, and the reason this field exists.** Comments and shares both
    /// read zero on a low-engagement feed — `Read or add comments. 0 comments` and
    /// `Share video.  shares` — so two different cards produced an identical fingerprint,
    /// every swipe was recorded as unproven, no video was ever counted, and the session
    /// stopped at `STUCK_SWIPE_LIMIT` believing it was stuck. On a six-phone run the
    /// sessions were watching and swiping correctly the whole time and reported `0/2 video`.
    ///
    /// `None` on a build whose sound strip has not been measured, and on a post using
    /// licensed music rather than an original sound — in both cases this degrades to
    /// exactly the previous behaviour rather than to something worse.
    sound: Option<String>,
}

impl PostFingerprint {
    /// Whether the card showed an action rail at all.
    ///
    /// Deliberately unchanged by `sound`: this answers "is there a rail here", which is what
    /// distinguishes an ordinary post from a LIVE card, and the sound strip is not part of
    /// that question.
    fn is_empty(&self) -> bool {
        self.comments.is_none() && self.share.is_none()
    }

    fn has_same_author(&self, other: &Self) -> bool {
        self.author.is_some() && self.author == other.author
    }

    /// Whether two reads conservatively describe the card that already received an action.
    ///
    /// Comment/share counts are intentionally absent: our own comment changes one of them,
    /// and either can move between hierarchy reads while the feed stays on the same card.
    /// Author plus sound is the strongest stable identity this hierarchy exposes. When sound
    /// disappears transiently but the author still agrees, treating the card as the same costs
    /// at most one skipped action; treating it as new can post a duplicate that cannot be
    /// undone. Full [`PartialEq`] remains the swipe-change proof elsewhere.
    fn same_card_for_duplicate_guard(&self, other: &Self) -> bool {
        if self.is_empty() || other.is_empty() {
            return false;
        }
        match (self.author.as_deref(), other.author.as_deref()) {
            (Some(left), Some(right)) if left != right => false,
            (Some(_), Some(_)) => match (self.sound.as_deref(), other.sound.as_deref()) {
                (Some(left), Some(right)) => left == right,
                _ => true,
            },
            _ => match (self.sound.as_deref(), other.sound.as_deref()) {
                (Some(left), Some(right)) => left == right,
                _ => self == other,
            },
        }
    }
}

/// Extract the author embedded around a measured control needle. Vietnamese profile labels
/// put the name after `Hồ sơ `, English profile labels put it before ` profile`, and Follow
/// puts it after `Follow `. Removing the catalogued needle rather than guessing a language
/// keeps all three shapes on one comparison.
fn embedded_author(description: &str, label: crate::tiktok_labels::LabelMatch) -> Option<String> {
    let observed = crate::interaction::normalize_locator_text(description);
    let needle = crate::interaction::normalize_locator_text(label.value());
    let at = observed.find(&needle)?;
    let author = crate::interaction::normalize_locator_text(&format!(
        "{} {}",
        &observed[..at],
        &observed[at + needle.len()..]
    ));
    (!author.is_empty()).then_some(author)
}

fn follow_control_matches_author(
    labels: TikTokControls,
    expected: &PostFingerprint,
    follow: &ElementBox,
) -> bool {
    let Some(expected_description) = expected.author.as_deref() else {
        return false;
    };
    let Some(follow_description) = follow.description.as_deref() else {
        return false;
    };
    let Some(profile_label) = labels.label(TikTokControl::AuthorProfileLink) else {
        return false;
    };
    let Some(follow_label) = labels.label(TikTokControl::Follow) else {
        return false;
    };
    embedded_author(expected_description, profile_label)
        .zip(embedded_author(follow_description, follow_label))
        .is_some_and(|(expected, actual)| expected == actual)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FollowVerdict {
    Followed,
    NoControl,
    SourceUnavailable,
    CardChanged,
    NotConfirmed,
    Cancelled,
}

impl FollowVerdict {
    fn did_act(self) -> bool {
        matches!(self, Self::Followed | Self::NotConfirmed)
    }

    fn latch_session_outcome(self, outcome: &mut Outcome) {
        if self == Self::NotConfirmed && *outcome == Outcome::Done {
            *outcome = Outcome::Partial;
        }
    }
}

/// Proof that the comment's write-ahead audit row exists.
///
/// The field is private and the only public constructor performs the INSERT itself. A source
/// therefore cannot hand the hierarchy driver a bare string and claim it is durable. The
/// boundary checks the marker again immediately before any comment UI is touched.
#[derive(Debug, PartialEq, Eq)]
pub struct CommentAuditToken {
    id: String,
    text_sha256: String,
    persisted: bool,
    consumed: bool,
}

impl CommentAuditToken {
    pub fn persist(database: &Database, attempt: &NurtureCommentAttempt) -> anyhow::Result<Self> {
        Self::persist_for_text(database, attempt, &attempt.preview)
    }

    pub fn persist_for_text(
        database: &Database,
        attempt: &NurtureCommentAttempt,
        text: &str,
    ) -> anyhow::Result<Self> {
        anyhow::ensure!(
            !attempt.id.trim().is_empty(),
            "comment audit attempt id is empty"
        );
        anyhow::ensure!(
            attempt.id.trim() == attempt.id,
            "comment audit attempt id has surrounding whitespace"
        );
        Uuid::parse_str(&attempt.id)
            .map_err(|error| anyhow::anyhow!("comment audit attempt id is not a UUID: {error}"))?;
        anyhow::ensure!(
            attempt.preview == text.chars().take(160).collect::<String>(),
            "comment audit preview does not match the persisted payload"
        );
        database.add_nurture_comment_attempt(attempt)?;
        Ok(Self {
            id: attempt.id.clone(),
            text_sha256: comment_text_sha256(text),
            persisted: true,
            consumed: false,
        })
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    fn is_verified(&self) -> bool {
        self.persisted
            && !self.consumed
            && !self.id.trim().is_empty()
            && self.id.trim() == self.id
            && Uuid::parse_str(&self.id).is_ok()
    }

    fn consume_for(&mut self, text: &str) -> bool {
        if !self.is_verified() {
            return false;
        }
        self.consumed = true;
        self.text_sha256 == comment_text_sha256(text)
    }

    #[cfg(test)]
    fn verified_fixture(text: &str) -> Self {
        Self {
            id: "00000000-0000-4000-8000-000000000001".into(),
            text_sha256: comment_text_sha256(text),
            persisted: true,
            consumed: false,
        }
    }

    #[cfg(test)]
    fn unverified_fixture(id: impl Into<String>, text: &str) -> Self {
        Self {
            id: id.into(),
            text_sha256: comment_text_sha256(text),
            persisted: false,
            consumed: false,
        }
    }
}

fn comment_text_sha256(text: &str) -> String {
    format!("{:x}", Sha256::digest(text.as_bytes()))
}

/// A comment the caller has already decided to post, and what it cost.
#[derive(Debug, PartialEq)]
pub struct PreparedComment {
    pub text: String,
    /// What the API reported spending to generate it, added to the session total.
    ///
    /// Tokens rather than money: the price this used to carry was two hand-typed numbers
    /// the app multiplied itself, and they matched no configured model.
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    /// Typed proof of the caller's durable audit row, echoed back to
    /// [`CommentTextSource::record_outcome`].
    pub audit_attempt: CommentAuditToken,
}

/// A comment source can decline ordinary context with `Ok(None)`, but failure to persist the
/// required write-ahead row is a different, operator-actionable result. Keeping it typed stops
/// the hierarchy loop from reporting a broken audit store as an AI/context skip.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommentSourceError {
    AuditUnavailable,
}

/// Where this loop gets the words for a comment.
///
/// A trait rather than the AI call itself, for two reasons. It keeps provider
/// plumbing — frames, OpenAI, pricing, audit rows — out of a module whose job is
/// driving a hierarchy; and it lets the G2 probe exercise the whole drawer flow
/// with a fixed string and its own audit database, no control plane behind it. The desktop app
/// supplies the *same* grounded generator the iOS engine uses, so both backends
/// comment from the same evidence and write the same audit trail.
#[async_trait::async_trait]
pub trait CommentTextSource: Send + Sync {
    /// Words for the post on screen right now, or `None` to skip this post.
    ///
    /// `None` is the ordinary answer when the context is not good enough to say
    /// anything — an unread caption, an unusable frame. It is not an error.
    ///
    /// `settings` is the loop's **current** copy, not the one the session started
    /// with, so a corrected API key or a reworded direction reaches the next comment.
    /// It is passed in rather than held by the implementor because the loop owns the
    /// only live copy; a borrow captured at session start would be a second, stale
    /// answer to the same question.
    async fn comment_for_post(
        &self,
        settings: &NurtureSettings,
    ) -> Result<Option<PreparedComment>, CommentSourceError>;

    /// Record how the attempt ended. There is deliberately no default: every source capable of
    /// returning prepared public text must make an explicit outcome decision.
    async fn record_outcome(&self, prepared: &PreparedComment, outcome: &str);

    /// Offer whatever is on screen right now as evidence for the comment that follows.
    ///
    /// Called once per slide of a photo post, from inside [`HierarchyRun::traverse_carousel`],
    /// at the settled moment just after the page counter was read. **The comment used to be
    /// written before the first sideways gesture existed**, so a six-image post was commented
    /// on from three samples of image one — and since a still card publishes the same picture
    /// every time, that was one picture, three times.
    ///
    /// Sync, and cheap by contract: the implementor is expected to read a frame cache, not to
    /// ask the device for anything. A slide costs a flick, a 900 ms settle and a hierarchy
    /// dump already; this must add nothing to that.
    fn note_slide(&self) {}

    /// Record a comment the post's budget was charged for that never reached the drawer.
    ///
    /// The deferred path spends `record_attempt` and the action gap *before* the slides are
    /// paged, so a card that changes underneath the traversal leaves an attempt with no row.
    /// Default does nothing, for callers with no audit trail to keep.
    async fn record_skip(&self, _settings: &NurtureSettings, _reason: &str) {}
}

/// Locate a control, or `None` when the label for it was never measured.
///
/// The two `None`s are different and both matter: no measured label means *do not
/// look*, while a measured label that finds nothing means the control is not on
/// this card.
async fn locate(
    session: &dyn UiSession,
    labels: TikTokControls,
    control: TikTokControl,
) -> anyhow::Result<Option<ElementBox>> {
    let Some(label) = labels.label(control) else {
        return Ok(None);
    };
    let found = session.locate(label.to_query()).await?;
    if found.is_some() {
        return Ok(found);
    }
    let target = match control {
        TikTokControl::Comments => "comments",
        TikTokControl::Share => "share",
        TikTokControl::ProfileTab => "profile",
        _ => return Ok(None),
    };
    crate::ui_automation::runtime::resolve_navigation_local(
        session,
        target,
        std::time::Duration::from_secs(30),
    )
    .await
}

/// Whether this build can be asked to page a photo carousel at all.
///
/// A named check beside [`can_follow`] for the same reason that one exists: the answer was
/// *no* for three of the fourteen phones on this farm, and the consequence of not asking was
/// not a missing gesture — it was **silence**. `traverse_carousel` returns 0 on an unmeasured
/// badge without reporting anything, so the panel showed a "Bài ảnh" group promising a
/// feature those phones could never perform, and the session-start provenance line read
/// perfectly healthy. Worse, that line is *inverted*: `TIKTOK_RESOURCE_SETS` has no entry for
/// `trill` 38.3.2, so the eleven phones that **can** page carousels print "CHƯA đo resource id
/// cho phiên bản app này" while the three that could not were the only ones printing a
/// reassuring version.
///
/// Costs nothing at runtime — the badge lookup is a table read, not a device query.
fn can_page_carousel(labels: TikTokControls) -> bool {
    labels.label(TikTokControl::PhotoBadge).is_some()
}

/// Whether this build can be asked to follow at all.
///
/// A named check rather than an inline `is_some()`, because the answer is *no* for most
/// of this fleet and the consequence of not asking is not a missing follow — it is a
/// wrong sentence and a spent budget. See the call site.
fn can_follow(labels: TikTokControls) -> bool {
    labels.label(TikTokControl::Follow).is_some()
}
/// True when the label is measured *and* on screen.
async fn present(session: &dyn UiSession, labels: TikTokControls, control: TikTokControl) -> bool {
    matches!(locate(session, labels, control).await, Ok(Some(_)))
}

/// The card identity, never failing: a failed snapshot falls back to the per-control reads,
/// which keep whatever they can still see, as this did before the snapshot existed. Read from
/// the same source as [`read_fingerprint`], so a swipe is judged against a `before` and an
/// `after` that were described the same way.
async fn fingerprint(session: &dyn UiSession, labels: TikTokControls) -> PostFingerprint {
    fingerprint_and_surface(session, labels).await.0
}

/// What one card read says about the card besides its identity.
#[derive(Debug, Clone, PartialEq)]
enum CardSurface {
    /// A snapshot was read and carried no sponsored marker.
    Ordinary,
    /// A snapshot was read and carried one; see [`super::ad_card::sponsored_card`].
    Sponsored(super::ad_card::SponsoredCard),
    /// The backend has no snapshots, so the markers can never be looked for there. The
    /// per-control path reads four rail labels and nothing else; it keeps its old behaviour.
    Unsupported,
    /// A snapshot was asked for and the read failed. Unknown, not ordinary.
    Unread,
}

/// [`fingerprint`] plus, from the **same** snapshot, whether the card is a sponsored one.
/// No second read: the sponsored markers are on the tree the identity was just read from.
async fn fingerprint_and_surface(
    session: &dyn UiSession,
    labels: TikTokControls,
) -> (PostFingerprint, CardSurface) {
    match read_card(session, labels).await {
        Ok(read) => read,
        Err(error) => {
            tracing::debug!(%error, "nurture card snapshot unread; per-control fallback");
            let read = |control| async move {
                locate(session, labels, control)
                    .await
                    .ok()
                    .flatten()
                    .and_then(|element| element.description)
            };
            (
                PostFingerprint {
                    author: read(TikTokControl::AuthorProfileLink).await,
                    comments: read(TikTokControl::Comments).await,
                    share: read(TikTokControl::Share).await,
                    sound: read(TikTokControl::SoundLink).await,
                },
                CardSurface::Unread,
            )
        }
    }
}

/// The card identity from **one** hierarchy snapshot. A wait must retain a failed read rather
/// than treating it as an empty rail.
///
/// This used to be four sequential `locate`s, each its own `/element` round trip plus the
/// rectangle and attribute reads, and a railless card added the navigation resolver's own
/// snapshots on top. Box 31–50 acceptance run, 10/10/2026 (Galaxy S8+,
/// `com.zhiliaoapp.musically` / en): single agent reads took up to 6 380 ms for `/element`
/// and 5 468–10 690 ms for `/source`, so one fingerprint outlived the whole swipe window, no
/// read completed, and 22 swipes were reported unproven. One snapshot is one round trip, and
/// all four descriptions come from the same frame, so they cannot straddle two cards either.
///
/// A backend that refuses snapshots keeps the per-control reads.
async fn read_fingerprint(
    session: &dyn UiSession,
    labels: TikTokControls,
) -> anyhow::Result<PostFingerprint> {
    Ok(read_card(session, labels).await?.0)
}

/// [`read_fingerprint`] with the card's surface from the same snapshot.
async fn read_card(
    session: &dyn UiSession,
    labels: TikTokControls,
) -> anyhow::Result<(PostFingerprint, CardSurface)> {
    let epoch = session.gui_session_epoch();
    let observed = match session.hierarchy_source_snapshot().await {
        Ok(snapshot) => {
            let tree = crate::ui_automation::tree::Tree::parse(snapshot)?;
            (
                tree_fingerprint(&tree, labels),
                super::ad_card::sponsored_card(&tree, labels)
                    .map_or(CardSurface::Ordinary, CardSurface::Sponsored),
            )
        }
        Err(error)
            if error
                .downcast_ref::<crate::driver::UnsupportedCapability>()
                .is_some() =>
        {
            let read = |control| async move {
                Ok::<_, anyhow::Error>(
                    locate(session, labels, control)
                        .await?
                        .and_then(|element| element.description),
                )
            };
            (
                PostFingerprint {
                    author: read(TikTokControl::AuthorProfileLink).await?,
                    comments: read(TikTokControl::Comments).await?,
                    share: read(TikTokControl::Share).await?,
                    sound: read(TikTokControl::SoundLink).await?,
                },
                CardSurface::Unsupported,
            )
        }
        Err(error) => return Err(error),
    };
    anyhow::ensure!(
        session.gui_session_epoch() == epoch,
        "rail session changed during read"
    );
    Ok(observed)
}

/// How long a card read already running when its wait's deadline passes may still finish.
///
/// Cutting it at the deadline threw away the only evidence the wait had: on the box 31–50
/// acceptance run of 10/10/2026 (Galaxy S8+), `/source` took 5 468–10 690 ms under load,
/// longer than the whole swipe window, so a swipe that had advanced was reported unproven
/// with no completed read, and the late-rail wait timed out 10 times out of 10. Sized just
/// above that 10 690 ms maximum. No read *starts* after the deadline; a read still running
/// at the end of this grace is cut as before, so the wait stays finite.
const CARD_READ_GRACE: Duration = Duration::from_millis(11_000);

/// The page-indicator band of a photo post on the global 45.7.3 build: a full-width
/// `FrameLayout` 180 px tall (child `q9w` holding the dots `q_b`/`q9x`, layout `eb0`) that sits
/// on the bottom edge of the image, so its height on screen follows the image's aspect ratio.
/// Measured on 45.7.3 feed dumps (1080×2094 window, Galaxy S8+): [1363,1543]
/// (`share-deeplink-probe/raw/B2h_photo_url.xml`), [1481,1661]
/// (`acceptance-20261010/phaseA/riviu43/07-after-7.xml`), and [986,1166] up to [1260,1440]
/// on others. The usual feed swipe starts at 72 % of that window, y≈1508 ± the planner's
/// jitter, which is inside the band on a tall image and clear of it on a short one. Box
/// 31–50 round 8 (10/10/2026) left about half of its paged photo posts and stuck on the
/// rest with both the flick and the bowed path — `ce011711c354be2005` 14:42:41 and
/// 14:47:14–14:48:08, `ce031713b064841e0c` 14:41:00, `ce0317139ca344520c` 14:41:18 — while
/// the same start left every video. The band reports `clickable=false` and the tree does
/// not say what it does with a drag; it is avoided, not interpreted. The id is an obfuscated
/// name, so it is bound to [`PHOTO_INDICATOR_BUILD`] the way [`super::ad_card`] binds its own.
const PHOTO_INDICATOR_ID: &str = ":id/q9t";
const PHOTO_INDICATOR_BUILD: &str = "45.7.3";
const PHOTO_INDICATOR_PACKAGE: &str = "com.zhiliaoapp.musically";

/// Evidence bundles of unproven vertical swipes one session may write: one per card, this
/// many in all. See [`capture_unproven_swipe`].
const UNPROVEN_SWIPE_EVIDENCE_LIMIT: usize = 3;

/// The visible photo page-indicator bands on `tree`, empty on a build whose band is not
/// measured.
fn photo_indicator_bands(
    tree: &crate::ui_automation::tree::Tree,
    labels: TikTokControls,
) -> Vec<ElementBox> {
    if labels.package() != PHOTO_INDICATOR_PACKAGE
        || labels.resource_version() != Some(PHOTO_INDICATOR_BUILD)
    {
        return Vec::new();
    }
    tree.nodes
        .iter()
        .enumerate()
        .filter(|(index, node)| {
            node.visible(PHOTO_INDICATOR_PACKAGE)
                && tree.ancestors_visible(*index)
                && node.attr("resource-id").ends_with(PHOTO_INDICATOR_ID)
        })
        .filter_map(|(_, node)| node.rect())
        .collect()
}

fn fingerprint_json(card: &PostFingerprint) -> serde_json::Value {
    serde_json::json!({
        "author": card.author,
        "comments": card.comments,
        "share": card.share,
        "sound": card.sound,
    })
}

/// One redacted evidence bundle for a vertical swipe that is unproven for the second time in
/// a row, through the popup-artifact path ([`crate::app_automation::popup_evidence`]): the
/// screen, the hierarchy, and in `meta.json` the swipe's ends, the card before it and the card
/// read now, the carousel this card had and the photo indicator band. Answers the folder for
/// the operator line; `None` when nothing was written. Bounded by [`CARD_READ_GRACE`].
async fn capture_unproven_swipe(
    run: &HierarchyRun<'_>,
    before: &PostFingerprint,
    reads: &CardReads,
    carousel_pages: Option<u32>,
    stop: &AtomicBool,
) -> Option<String> {
    let deadline = tokio::time::Instant::now() + CARD_READ_GRACE;
    let snapshot =
        match read_before_deadline(run.session.hierarchy_source_snapshot(), deadline, stop).await {
            Ok(ReadWaitResult::Ready(snapshot)) => snapshot,
            _ => return None,
        };
    let tree = crate::ui_automation::tree::Tree::parse(snapshot).ok()?;
    let after = tree_fingerprint(&tree, run.labels);
    let bands: Vec<_> = photo_indicator_bands(&tree, run.labels)
        .iter()
        .map(|band| {
            [
                band.x.round() as i64,
                band.y.round() as i64,
                (band.x + band.width).round() as i64,
                (band.y + band.height).round() as i64,
            ]
        })
        .collect();
    let detail = serde_json::json!({
        "screen": [run.screen.0, run.screen.1],
        "swipe": reads.swipe.map(|[fx, fy, tx, ty]| serde_json::json!({
            "from": [fx, fy],
            "to": [tx, ty],
        })),
        "gestureCancelled": reads.gesture_cancelled,
        "fingerprintBefore": fingerprint_json(before),
        "fingerprintAfter": fingerprint_json(&after),
        "sameCard": before.same_card_for_duplicate_guard(&after),
        "carousel": {
            "ran": carousel_pages.is_some(),
            "page": carousel_pages,
        },
        "photoIndicatorBands": bands,
    });
    crate::app_automation::popup_evidence::capture_with(
        run.session,
        &tree,
        run.labels,
        "nurture-unproven-swipe",
        "vuốt dọc chưa chứng minh được đổi thẻ hai lần liên tiếp",
        detail,
        deadline,
        stop,
    )
    .await
}

/// What the card reads inside one bounded wait cost, for the unproven-swipe and late-rail
/// lines, so the windows can be sized from measurement rather than guessed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct CardReads {
    completed: u32,
    last: Option<Duration>,
    /// A read still running when even [`CARD_READ_GRACE`] had run out.
    cut_after: Option<Duration>,
    /// The OS cancelled the swipe part-way ([`crate::driver::GestureCancelled`]); the judgement
    /// comes from the re-read that followed it, not from the gesture.
    gesture_cancelled: bool,
    /// What the shared popup step did on that re-read, when it did anything.
    popup: Option<&'static str>,
    /// The swipe as sent, `[from x, from y, to x, to y]` in screen pixels, for the evidence
    /// bundle of an unproven swipe.
    swipe: Option<[i64; 4]>,
}

impl CardReads {
    fn says(&self) -> String {
        let mut line = match self.last {
            Some(last) => format!(
                "đọc thẻ {} lần, lần cuối {} ms",
                self.completed,
                last.as_millis()
            ),
            None => "chưa đọc xong thẻ lần nào".to_string(),
        };
        if let Some(cut) = self.cut_after {
            line.push_str(&format!(", một lần đọc bị cắt sau {} ms", cut.as_millis()));
        }
        line
    }

    /// The operator line for a cancelled swipe, `None` when the swipe was not cancelled.
    fn cancelled_says(&self) -> Option<String> {
        self.gesture_cancelled.then(|| {
            let mut line =
                "vuốt bị hệ thống huỷ giữa chừng (cửa sổ bị chạm đã đóng) — đọc lại màn hình"
                    .to_string();
            if let Some(popup) = self.popup {
                line.push_str(&format!("; {popup}"));
            }
            line
        })
    }
}

/// One card read inside a wait. None starts at or after `deadline`; one already running then
/// may finish within [`CARD_READ_GRACE`], and the caller judges what it read before the
/// deadline ends the wait.
async fn read_card_in_window(
    session: &dyn UiSession,
    labels: TikTokControls,
    deadline: tokio::time::Instant,
    stop: &AtomicBool,
    reads: &mut CardReads,
) -> anyhow::Result<ReadWaitResult<PostFingerprint>> {
    if stop.load(Ordering::Relaxed) {
        return Ok(ReadWaitResult::Cancelled);
    }
    if tokio::time::Instant::now() >= deadline {
        return Ok(ReadWaitResult::DeadlineExceeded);
    }
    let started = tokio::time::Instant::now();
    let result = read_before_deadline(
        read_fingerprint(session, labels),
        deadline + CARD_READ_GRACE,
        stop,
    )
    .await?;
    match &result {
        ReadWaitResult::Ready(_) => {
            reads.completed += 1;
            reads.last = Some(started.elapsed());
        }
        ReadWaitResult::DeadlineExceeded => reads.cut_after = Some(started.elapsed()),
        ReadWaitResult::Cancelled => {}
    }
    Ok(result)
}

/// The dwell is read at most this often. Each checkpoint takes one fresh snapshot that the
/// popup step and the card identity both use; the last stretch before the deadline is
/// slept, never read again to "catch up".
const WATCH_CHECKPOINT: Duration = Duration::from_secs(5);

/// How one watched card ended.
#[derive(Debug, Clone, PartialEq, Eq)]
enum WatchEnd {
    /// The whole dwell elapsed and every checkpoint read the same card, or none was due.
    Watched,
    Stopped,
    /// A checkpoint read another card than the first checkpoint: the watched card is gone.
    CardChanged,
    /// A popup was dismissed and no clean read came after it; nothing proves which card is
    /// now on screen.
    Unproven,
    /// A popup only the operator may answer.
    Blocked(String),
}

/// Card identity from one snapshot: the four controls [`read_fingerprint`] reads.
fn tree_fingerprint(
    tree: &crate::ui_automation::tree::Tree,
    labels: TikTokControls,
) -> PostFingerprint {
    let package = labels.package();
    let read = |control| {
        let query = labels.label(control)?.to_query();
        let nodes = if let ElementQuery::Semantic(role) = query {
            crate::app_automation::tiktok_roles::indices(tree, package, role)
        } else {
            tree.matching(package, query)
        };
        let [index] = nodes.as_slice() else {
            return None;
        };
        let node = &tree.nodes[*index];
        [node.attr("content-desc"), node.attr("text")]
            .into_iter()
            .find(|value| !value.is_empty())
            .map(str::to_owned)
    };
    PostFingerprint {
        author: read(TikTokControl::AuthorProfileLink),
        comments: read(TikTokControl::Comments),
        share: read(TikTokControl::Share),
        sound: read(TikTokControl::SoundLink),
    }
}

/// Watch the card for `dwell`, interruptible by Stop, with a popup/identity checkpoint every
/// [`WATCH_CHECKPOINT`]. One popup budget serves the whole card. Every read is bounded by the
/// watch deadline, so the dwell never grows; a failed read is not evidence and the watch goes
/// on until the next checkpoint.
async fn watch_card(
    session: &dyn UiSession,
    labels: TikTokControls,
    dwell: Duration,
    stop: &AtomicBool,
    status: &mut NurtureSessionStatus,
    report: &(dyn Fn(&mut NurtureSessionStatus, String) + Send + Sync),
) -> WatchEnd {
    use crate::app_automation::dialogs::{step_before_deadline_with_tree, PopupBudget, PopupStep};
    let deadline = tokio::time::Instant::now() + dwell;
    let mut popup = PopupBudget::default();
    let mut card: Option<PostFingerprint> = None;
    let mut unproven = false;
    loop {
        let left = deadline.saturating_duration_since(tokio::time::Instant::now());
        if left.is_zero() {
            break;
        }
        sleep_interruptible(left.min(WATCH_CHECKPOINT), stop).await;
        if stop.load(Ordering::Relaxed) {
            return WatchEnd::Stopped;
        }
        if tokio::time::Instant::now() >= deadline || !session.supports_accessibility_readback() {
            continue;
        }
        let step =
            step_before_deadline_with_tree(session, labels, &mut popup, deadline, stop, stop).await;
        match step {
            Ok(ReadWaitResult::Ready((PopupStep::Clear, Some(tree)))) => {
                unproven = false;
                let seen = tree_fingerprint(&tree, labels);
                match &card {
                    Some(first)
                        if !first.is_empty()
                            && !seen.is_empty()
                            && !first.same_card_for_duplicate_guard(&seen) =>
                    {
                        return WatchEnd::CardChanged;
                    }
                    Some(_) => {}
                    None => card = Some(seen),
                }
            }
            Ok(ReadWaitResult::Ready((PopupStep::Clear, None))) => {}
            Ok(ReadWaitResult::Ready((PopupStep::Dismissed, _))) => {
                // The card under the popup is re-grounded by the next clean read.
                card = None;
                unproven = true;
                report(
                    status,
                    popup
                        .says()
                        .unwrap_or("đóng một hộp thoại tùy chọn đã đo; chờ đọc lại màn hình")
                        .to_string(),
                );
            }
            Ok(ReadWaitResult::Ready((PopupStep::Blocked, _))) => {
                return WatchEnd::Blocked(
                    popup
                        .says()
                        .unwrap_or("màn hình đang bị một lớp phủ chặn — Riviu không bấm gì; cần người vận hành kiểm tra")
                        .to_string(),
                );
            }
            Ok(ReadWaitResult::Cancelled) => return WatchEnd::Stopped,
            Ok(ReadWaitResult::DeadlineExceeded) => break,
            Err(error) => {
                tracing::debug!(%error, "nurture watch checkpoint unread; keeps watching");
            }
        }
    }
    if unproven {
        WatchEnd::Unproven
    } else {
        WatchEnd::Watched
    }
}

struct HierarchyNurtureSaveAdapter<'a> {
    session: &'a dyn UiSession,
    labels: TikTokControls,
    sequence: u64,
}

#[async_trait::async_trait]
impl SaveAdapter for HierarchyNurtureSaveAdapter<'_> {
    async fn observe(&mut self) -> anyhow::Result<SaveObservation> {
        self.sequence = self.sequence.saturating_add(1);
        let before = fingerprint(self.session, self.labels).await;
        let control =
            crate::tiktok_save::hierarchy::read_bookmark_control(self.session, self.labels).await?;
        let card = fingerprint(self.session, self.labels).await;
        if !before.same_card_for_duplicate_guard(&card) {
            let mut observation = hierarchy_save_observation(
                SaveCardIdentity::Hierarchy {
                    author: String::new(),
                    sound: None,
                },
                self.sequence,
                control,
            );
            observation.identity = None;
            return Ok(observation);
        }
        let Some(author) = card.author else {
            let mut observation = hierarchy_save_observation(
                SaveCardIdentity::Hierarchy {
                    author: String::new(),
                    sound: card.sound,
                },
                self.sequence,
                control,
            );
            observation.identity = None;
            return Ok(observation);
        };
        Ok(hierarchy_save_observation(
            SaveCardIdentity::Hierarchy {
                author,
                sound: card.sound,
            },
            self.sequence,
            control,
        ))
    }

    async fn tap(&mut self, point: TapPoint) -> anyhow::Result<()> {
        self.session.tap(point).await
    }
}

/// One hierarchy-driven nurture session.
///
/// Takes the session by reference and owns nothing the caller needs back, so it
/// can be dropped mid-run without leaving the device in a special state — every
/// action it performs is a tap or a swipe that a person could have made.
pub(super) struct HierarchyRun<'a> {
    session: &'a dyn UiSession,
    labels: TikTokControls,
    screen: (f64, f64),
    planner: TouchPointPlanner,
    search_keyword: Option<String>,
}

/// Why a device cannot run this loop, phrased for the operator.
///
/// A refusal is a first-class result rather than an error string, because the
/// caller has to decide whether to fall through to the pixel engine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Unsupported {
    /// The backend cannot report element geometry (iOS).
    NoElementBounds,
    /// Geometry works, but nobody has measured labels for this build/language.
    /// The build is outside the fleet allowlist (`crate::tiktok_fleet_policy`).
    BuildNotAllowed(String),
    NoLabelSet {
        package: String,
        language: Option<String>,
    },
}

impl Unsupported {
    /// The message shown in the session status, in the operator's language.
    pub(super) fn message(&self) -> String {
        match self {
            Self::NoElementBounds => {
                "máy này không đọc được vị trí phần tử — dùng đường nhận dạng ảnh".into()
            }
            Self::BuildNotAllowed(refusal) => format!("failed — {refusal}"),
            Self::NoLabelSet { package, language } => format!(
                "failed — chưa đo nhãn TikTok cho {package} + ngôn ngữ {}; \
                 đã đo: {}. Đo nhãn bằng `cargo run -p riviu-android-driver --example probe` \
                 trước khi chạy máy này",
                language.as_deref().unwrap_or("không đọc được"),
                crate::tiktok_labels::TIKTOK_LABEL_SETS
                    .iter()
                    .map(|set| format!("{} / {}", set.package, set.language))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        }
    }
}

impl<'a> HierarchyRun<'a> {
    /// Decide whether this device can be driven by hierarchy, and with which
    /// labels.
    ///
    /// Reads the foreground package rather than trusting the configured bundle
    /// id: the regional builds differ (`com.zhiliaoapp.musically` vs
    /// `com.ss.android.ugc.trill`) and their labels differ with them, so the
    /// package that is *actually* on screen is the one to key on.
    pub(super) async fn prepare(
        session: &'a dyn UiSession,
        screen: (f64, f64),
        settings: &NurtureSettings,
    ) -> Result<Self, Unsupported> {
        if !session.supports_element_bounds() {
            return Err(Unsupported::NoElementBounds);
        }
        let foreground = session
            .active_app_bundle()
            .await
            .unwrap_or_else(|_| settings.bundle_id.clone());
        let package = tiktok_behind_permission_window(session, &settings.bundle_id, &foreground)
            .await
            .unwrap_or(foreground);
        let language = session.ui_language().await;
        // Read once per session, not per locate: `dumpsys package` measured 1–2 s on
        // this fleet. An unread version is not fatal — it leaves only the resource-id
        // controls absent, so this loop can still like and read.
        let app_version = session.app_version(&package).await.unwrap_or_default();
        // Admission checked the build at planning; TikTok can update itself before the
        // session opens, so the session that is about to act asks again, before any tap.
        if let Err(refusal) = crate::tiktok_fleet_policy::require_fleet_build(
            &package,
            &app_version,
            language.as_deref().unwrap_or_default(),
        ) {
            return Err(Unsupported::BuildNotAllowed(refusal.to_string()));
        }
        let labels = language
            .as_deref()
            .and_then(|language| {
                crate::tiktok_labels::controls_for_runtime(&package, language, &app_version)
            })
            .ok_or_else(|| Unsupported::NoLabelSet {
                package: package.clone(),
                language: language.clone(),
            })?;
        Ok(Self {
            session,
            labels,
            screen,
            planner: TouchPointPlanner::new(screen),
            search_keyword: None,
        })
    }

    /// Provenance of the label set in use, for the session log.
    pub(super) fn labels_note(&self) -> String {
        format!("nhãn đã đo: {}", self.labels.provenance())
    }

    /// True when the feed tab is on screen — the only condition under which this
    /// loop will tap anything.
    async fn on_feed(&self) -> bool {
        if let Some(keyword) = &self.search_keyword {
            return super::search::on_video(self.session, self.labels.package(), keyword)
                .await
                .unwrap_or(false);
        }
        present(self.session, self.labels, TikTokControl::FeedTab).await
    }

    /// Like the current post and say what was actually proved.
    ///
    /// Confirmation without a measured liked-label: the *not-liked* label is an
    /// exact match, so once the state flips that exact string is gone. Requiring
    /// the comment control to still be present at the same time rules out the
    /// other reason the label could vanish — the card having moved on. That is a
    /// real proof, not an assumption, and it is why a build whose `Liked` label is
    /// unmeasured can still like a post honestly.
    /// Like the post, proved by the label state changing.
    ///
    /// One line, because the contract lives in [`crate::tiktok_like`] now — the Interaction
    /// campaign needs the same proof on a post opened from a link, and two copies of "the
    /// liked label appearing is the evidence" would drift into reporting different things.
    /// The tap placement stays here: the touch history and this device's hand belong to the
    /// run, not to the like.
    async fn like(&mut self, stop: &AtomicBool) -> Result<LikeVerdict, ActionFailure> {
        // The session reference is copied out before the planner is borrowed mutably: they
        // are disjoint fields, and taking them in this order is what lets both be held.
        let session = self.session;
        let labels = self.labels;
        let planner = &mut self.planner;
        crate::tiktok_like::like_post(
            session,
            labels,
            &mut |element: &ElementBox| {
                planner.next(
                    TapPoint {
                        x: element.x + element.width / 2.0,
                        y: element.y + element.height / 2.0,
                    },
                    (element.width / 2.0, element.height / 2.0),
                )
            },
            stop,
        )
        .await
    }

    async fn save(
        &mut self,
        journal: Option<&dyn NurtureSaveJournal>,
        card_key: &str,
        stop: &AtomicBool,
    ) -> (SaveEvidence, Option<NurtureSaveLease>) {
        let mut adapter = HierarchyNurtureSaveAdapter {
            session: self.session,
            labels: self.labels,
            sequence: 0,
        };
        let mut lease = None;
        let evidence = tiktok_save(&mut adapter, |observation| {
            anyhow::ensure!(
                !stop.load(Ordering::Acquire),
                "nurture Save cancelled before intent"
            );
            match journal {
                Some(journal) => {
                    lease = Some(journal.arm(card_key, observation)?);
                    anyhow::ensure!(
                        !stop.load(Ordering::Acquire),
                        "nurture Save cancelled before tap"
                    );
                    Ok(())
                }
                None => anyhow::bail!("nurture Save has no durable journal"),
            }
        })
        .await;
        (evidence, lease)
    }

    /// Post a comment, or say precisely which step could not be proved.
    ///
    /// Every stage has evidence, mirroring the iOS engine's states rather than
    /// inventing new ones:
    ///
    /// 1. **drawer open** — an `EditText` exists (the field's `content-desc` is
    ///    empty and its `text` is a placeholder, so class is the only handle);
    /// 2. **field focused** — tapping it brings the keyboard, which moves the field
    ///    and makes the Send button appear `enabled=false`;
    /// 3. **armed** — after `type_text`, that same Send button reads
    ///    `enabled=true`. This is the measured false→true transition, and it is the
    ///    hierarchy's answer to the pixel engine's `CommentDrawer::SendArmed`;
    /// 4. **sent** — after tapping Send the button goes back to not-armed and the
    ///    field no longer holds the text.
    ///
    /// A step without its evidence returns the matching [`CommentVerdict`] and does
    /// **not** retry: a tapped Send whose result cannot be read is ambiguous, and
    /// re-sending an ambiguous comment is how a post ends up with two of them.
    async fn comment(
        &mut self,
        text: &str,
        stop: &AtomicBool,
    ) -> Result<CommentVerdict, ActionFailure> {
        // Delegated to `crate::tiktok_drawer`, which owns the measured flow. The
        // planner is threaded through so the taps keep this loop's jitter history —
        // the drawer module deliberately has no opinion about where inside a control
        // to touch.
        let screen = self.screen;
        let planner = &mut self.planner;
        let plan = move |element: &ElementBox| {
            let _ = screen;
            planner.next(element.centre(), element.jitter_radius())
        };
        if let Some(keyword) = &self.search_keyword {
            let mut drawer =
                crate::tiktok_drawer::CommentDrawer::new(self.session, self.labels, plan);
            if drawer.send_query().is_none() {
                return Ok(CommentVerdict::SendUnmeasured);
            }
            let result = crate::tiktok_drawer::post_into_drawer(&mut drawer, text, stop).await;
            // Return only to the search viewer. Generic drawer cleanup targets FYP
            // and would leave the keyword source after every comment.
            for _ in 0..3 {
                if super::search::on_video(self.session, self.labels.package(), keyword)
                    .await
                    .unwrap_or(false)
                {
                    break;
                }
                if stop.load(Ordering::Relaxed) {
                    break;
                }
                if self
                    .session
                    .press_hardware_key(crate::HardwareKey::Back)
                    .await
                    .is_err()
                {
                    break;
                }
                sleep_interruptible(SWIPE_SETTLE, stop).await;
            }
            result
        } else {
            crate::tiktok_drawer::post_comment(self.session, self.labels, plan, text, stop).await
        }
    }

    /// Follow the author, proved by the Follow control leaving the card.
    async fn follow(
        &mut self,
        expected: &PostFingerprint,
        stop: &AtomicBool,
        journal: Option<&dyn NurtureFollowJournal>,
    ) -> Result<FollowVerdict, ActionFailure> {
        if !self.on_feed().await {
            return Ok(FollowVerdict::CardChanged);
        }
        let current = fingerprint(self.session, self.labels).await;
        if !current.has_same_author(expected) {
            return Ok(FollowVerdict::CardChanged);
        }
        let Some(element) = locate(self.session, self.labels, TikTokControl::Follow)
            .await
            .map_err(ActionFailure::before)?
        else {
            return Ok(FollowVerdict::NoControl);
        };
        // A mixed/stale hierarchy can briefly expose AuthorProfileLink from card A beside a
        // Follow node from card B. Geometry alone cannot bind those nodes; the embedded names
        // must agree before the tap leaves this process.
        if !follow_control_matches_author(self.labels, expected, &element) {
            return Ok(FollowVerdict::CardChanged);
        }

        let Some(journal) = journal else {
            return Ok(FollowVerdict::SourceUnavailable);
        };
        let package = self
            .session
            .active_app_bundle()
            .await
            .map_err(ActionFailure::before)?;
        let Some(version) = self.session.app_version(&package).await else {
            return Ok(FollowVerdict::SourceUnavailable);
        };
        let Some(locale) = self.session.ui_language().await else {
            return Ok(FollowVerdict::SourceUnavailable);
        };
        let source_snapshot = match self.session.hierarchy_source_snapshot().await {
            Ok(snapshot) => snapshot,
            Err(_) => return Ok(FollowVerdict::SourceUnavailable),
        };
        let proof = match prove_nurture_follow_source(&package, &version, &locale, &source_snapshot)
        {
            Ok(proof) => proof,
            Err(_) => return Ok(FollowVerdict::SourceUnavailable),
        };
        let Some(profile_label) = self.labels.label(TikTokControl::AuthorProfileLink) else {
            return Ok(FollowVerdict::SourceUnavailable);
        };
        let Some(expected_handle) = expected
            .author
            .as_deref()
            .and_then(|author| embedded_author(author, profile_label))
            .filter(|author| author.starts_with('@'))
            .map(|author| author.to_ascii_lowercase())
        else {
            return Ok(FollowVerdict::SourceUnavailable);
        };
        if expected_handle != proof.identity().canonical_handle() {
            return Ok(FollowVerdict::CardChanged);
        }
        let mut evidence = serde_json::json!({
            "canonicalHandle": proof.identity().canonical_handle(),
            "cardKey": proof.identity().card_key(),
            "authorProfileKey": proof.identity().author_profile_key(),
            "hierarchyGeneration": proof.hierarchy_generation(),
            "snapshotSha256": proof.snapshot_sha256(),
        });
        let follow_tap_point = proof.follow_tap_point();
        if stop.load(Ordering::Relaxed) {
            return Ok(FollowVerdict::Cancelled);
        }
        // The database arm is deliberately the last operation before the tap. The point and
        // immutable identity came from this same hierarchy generation; no stale locator is kept.
        let lease = journal
            .arm(proof.identity())
            .map_err(ActionFailure::before)?;
        if stop.load(Ordering::Relaxed) {
            evidence["tap"] = serde_json::json!("cancelled_before_dispatch");
            journal
                .settle_failed_before_effect(
                    lease,
                    &evidence.to_string(),
                    "follow_cancelled_before_tap",
                )
                .map_err(ActionFailure::before)?;
            return Ok(FollowVerdict::Cancelled);
        }
        if let Err(error) = self.session.tap(follow_tap_point).await {
            evidence["tap"] = serde_json::json!("transport_error_after_arm");
            let serialized = evidence.to_string();
            let settlement =
                journal.settle_uncertain(lease, &serialized, "follow_tap_transport_uncertain");
            return Err(ActionFailure::after(match settlement {
                Ok(()) => anyhow::anyhow!("Follow tap failed after durable arm: {error}"),
                Err(settle) => anyhow::anyhow!(
                    "Follow tap failed after durable arm: {error}; uncertain settlement failed: {settle}"
                ),
            }));
        }
        sleep_interruptible(Duration::from_millis(1_200), stop).await;
        let readback_snapshot = match self.session.hierarchy_source_snapshot().await {
            Ok(value) => value,
            Err(error) => {
                evidence["readback"] = serde_json::json!("transport_error");
                let serialized = evidence.to_string();
                let settlement = journal.settle_uncertain(
                    lease,
                    &serialized,
                    "follow_readback_transport_uncertain",
                );
                return Err(ActionFailure::after(match settlement {
                    Ok(()) => anyhow::anyhow!("Follow readback failed after tap: {error}"),
                    Err(settle) => anyhow::anyhow!(
                        "Follow readback failed after tap: {error}; uncertain settlement failed: {settle}"
                    ),
                }));
            }
        };
        let readback = match readback_nurture_follow_source(
            proof.identity(),
            &package,
            &version,
            &locale,
            &readback_snapshot,
        ) {
            Ok(value) => value,
            Err(error) => {
                evidence["readback"] = serde_json::json!(error.to_string());
                journal
                    .settle_uncertain(lease, &evidence.to_string(), "follow_readback_unproved")
                    .map_err(ActionFailure::after)?;
                return Ok(FollowVerdict::NotConfirmed);
            }
        };
        evidence["readbackHierarchyGeneration"] =
            serde_json::json!(readback.hierarchy_generation());
        evidence["readbackSnapshotSha256"] = serde_json::json!(readback.snapshot_sha256());
        evidence["readbackVerdict"] = serde_json::json!(format!("{:?}", readback.verdict()));
        match readback.verdict() {
            NurtureFollowReadbackVerdict::FollowAbsent => {
                journal
                    .settle_confirmed(lease, &readback, &evidence.to_string())
                    .map_err(ActionFailure::after)?;
                Ok(FollowVerdict::Followed)
            }
            NurtureFollowReadbackVerdict::FollowPresent
            | NurtureFollowReadbackVerdict::CardChanged => {
                journal
                    .settle_uncertain(lease, &evidence.to_string(), "follow_not_confirmed")
                    .map_err(ActionFailure::after)?;
                Ok(FollowVerdict::NotConfirmed)
            }
        }
    }

    /// Where we are inside a photo post: `(current, total)`, or `None` when this card is
    /// not one.
    ///
    /// **Measured, and it corrects an earlier wrong conclusion of mine.** A previous look
    /// reported "no `N / M` counter anywhere in the TextViews" and the Android carousel was
    /// left unimplemented on the strength of that. The counter is there — it is simply
    /// **split across three nodes**: `"1"`, `" / "`, `"5"`. Searching for one node
    /// containing a slash finds nothing, which is exactly what happened.
    ///
    /// Measured on an SM-N950F, 12/08/2026, on two of the operator's own photo posts
    /// (`/photo/7668950377680735508`, five images, and `/photo/7668954054680136967`, two):
    /// the counter advances `1 / 5` → `2 / 5` → … → `5 / 5` and then **disappears** once
    /// the last image is passed. `ImageView` rectangles stay byte-identical throughout —
    /// 22 of them, never moving — so geometry is not the signal, and the comment label
    /// never changes, which is how the traversal knows it is still on the same post.
    ///
    /// Is this card a photo post? One cheap `locate`, and the gate for everything below.
    ///
    /// **The badge, not the page counter, and that took two wrong attempts to establish.**
    /// The `1 / 7` indicator sits in the top-right corner and is a *transient overlay* on
    /// the feed: measured on an SM-N950F on 12/08/2026 at 3 of 14 cards in one sweep and 0
    /// of 14 in another over the same kind of feed, because it fades a moment after the
    /// card arrives. A gate reading it fires only if it looks while the overlay is up, and
    /// in a real session it never does — the loop reaches here after the watch dwell and
    /// the interactions. Two full 30-card runs paged exactly nothing before this was
    /// understood. The badge sits beside the caption and stays.
    ///
    /// The cost of the badge is that it is a **translation**, so it lives in the catalog and
    /// a build whose language has not been measured simply does not page carousels. That is
    /// the right failure: a sideways swipe on a *video* card is TikTok's
    /// open-the-author's-profile gesture, so guessing here walks the session off the feed.
    async fn looks_like_photo_post(&self) -> bool {
        locate(self.session, self.labels, TikTokControl::PhotoBadge)
            .await
            .ok()
            .flatten()
            .is_some()
    }

    /// `(current, total)` from the page counter, when the digits are readable.
    ///
    /// **They are not always readable, and that is measured rather than assumed.** On a
    /// link-opened post page the three nodes `"1"`, `" / "`, `"5"` are all `TextView`s from
    /// the moment the page settles. On the **feed** the slash is some other class with an
    /// empty `content-desc`, no digits appear in the `TextView` list at all, and the three
    /// nodes only materialise **after the first sideways swipe** — measured as
    /// `+ "2" + " / " + "7"` arriving together on the first turn, then advancing `3 / 7`.
    ///
    /// So a caller must not treat `None` as "not a photo post". That is what
    /// [`Self::looks_like_photo_post`] is for, and conflating the two is precisely why a
    /// 25-card run paged nothing at all: the counter was read before the first swipe, on
    /// the one surface where it is not there yet.
    async fn carousel_position(&self) -> Option<(u32, u32)> {
        let request = crate::ui_automation::ObservationRequest {
            query: Default::default(),
            scope: Some(crate::ui_automation::ObservationScope {
                package: Some(self.labels.package().into()),
                root: None,
            }),
            fields: Default::default(),
            remaining_ms: 4_000,
        };
        match self.session.observe(&request).await {
            Ok(observation) => {
                if observation.session_epoch != self.session.gui_session_epoch()
                    || observation.app.package.as_deref() != Some(self.labels.package())
                {
                    return None;
                }
                let texts: Vec<_> = observation
                    .matches
                    .into_iter()
                    .filter(|node| {
                        node.class_name.as_deref() == Some("android.widget.TextView")
                            && node.visible == Some(true)
                            && node.password != Some(true)
                    })
                    .filter_map(|node| node.text)
                    .collect();
                return parse_carousel_counter(&texts);
            }
            Err(error)
                if error
                    .downcast_ref::<crate::driver::UnsupportedCapability>()
                    .is_some() => {}
            Err(_) => return None,
        }
        let texts: Vec<String> = self
            .session
            .locate_all_described(ElementQuery::ClassName("android.widget.TextView"))
            .await
            .ok()?
            .into_iter()
            .filter_map(|element| element.description)
            .collect();
        parse_carousel_counter(&texts)
    }

    /// One sideways page turn, through the image area.
    ///
    /// Clear of the action rail on the right and well above the caption, so the gesture
    /// reaches the pager and not a control.
    ///
    /// **A flick, not a swipe, and that is the whole difference between this working and
    /// not.** TikTok's image pager acts on a thrown finger and ignores a dragged one, so a
    /// `plan_swipe` path — bowed into the vertical axis, decelerating to a crawl, then held
    /// still for 12–45 ms before the lift — leaves the page where it was. The counter then
    /// reads the same number after the turn as before it, the loop concludes "no further
    /// image", and a photo post ends the session at two. Measured on the feed on 38.3.2 with
    /// one component changed at a time; the numbers are in [`TouchPointPlanner::plan_flick`].
    async fn swipe_slide(&mut self, stop: &AtomicBool) -> anyhow::Result<()> {
        // Planned, not two fixed points: the same targets every time produced the same
        // pixel-perfect straight line every time. A flick still jitters its ends and still
        // varies its leg spacing — it gives up only what the pager objects to.
        let path = self.planner.plan_flick(
            TapPoint {
                x: self.screen.0 * 0.78,
                y: self.screen.1 * 0.40,
            },
            TapPoint {
                x: self.screen.0 * 0.22,
                y: self.screen.1 * 0.40,
            },
            320,
        );
        self.session.swipe_path(path).await?;
        sleep_interruptible(CAROUSEL_SETTLE, stop).await;
        Ok(())
    }

    /// Page through a photo post, and report how many images were actually seen.
    ///
    /// The shape follows what the two surfaces actually do, not what would be tidy:
    ///
    /// * The **gate** is the slash node, which is there before anything is touched. Only a
    ///   card that passes it is ever swiped sideways — on a video card that gesture opens
    ///   the author's profile.
    /// * The **first turn happens before the total is known**, because on the feed the
    ///   digits only materialise once paging starts. So the post's own image count arrives
    ///   with the first turn and the portion is applied from there.
    /// * When the digits never materialise, a turn is proved by the **picture** changing
    ///   instead — a screen capture before and after, hashed by [`super::picture_digest_of`],
    ///   which ignores the status bar — and the card has to still be a photo post with its
    ///   rail afterwards. The tree cannot see a page turn (the `ImageView` rectangles and the
    ///   `TextView` list were identical across turns that provably advanced the counter,
    ///   `probe --measure-feed-carousel`, 18/08/2026), so the picture is the only other
    ///   witness there is. Bounded by the ceiling, since a fraction of an unknown total is
    ///   not a fraction.
    ///
    /// Every turn is **proved**, by the counter advancing or by the picture changing. A turn
    /// that proves nothing ends the traversal instead of being counted, the same rule
    /// [`Self::swipe_next`] applies to the feed.
    ///
    /// **Never swipes left from the last image, and never on a counter that has gone.** One
    /// swipe past the last slide does not stay put and does not wrap: it leaves for the
    /// author's profile page, Follow button and all (measured 25/08/2026 on
    /// `/photo/7668948504827448583`, see `interaction_hierarchy::photograph_photo_post`).
    /// This loop used to read an unreadable counter as "could not tell, keep turning" for up
    /// to three more gestures, and on box 31–50 (round 3, 09/10/2026, `ce0317139ca344520c`
    /// 21:39:02–21:39:28; round 4, 10/10/2026, `ce031713aadf361905` 00:10:49–00:12:07) a
    /// second pass over a post already on its last image did exactly that: four unproven
    /// sideways gestures, `TikTok đang khởi động — chờ feed lên`, Back, and the session
    /// ended on unproven vertical swipes. So the loop stops at `current == total`, and once
    /// the counter has rendered on a post, a read that no longer finds it is the end of the
    /// post, not an invitation.
    ///
    /// One consequence worth stating plainly: on the feed the floor is **two** images
    /// whenever the feature is on, because the total cannot be read until the first turn has
    /// already happened. A portion of 1% does not get a session out of turning once.
    async fn traverse_carousel(
        &mut self,
        portion_percent: u32,
        ceiling: u32,
        stop: &AtomicBool,
        status: &mut NurtureSessionStatus,
        report: &(dyn Fn(&mut NurtureSessionStatus, String) + Send + Sync),
        // Where each slide is offered as evidence for the comment that follows. `None` from
        // callers with no comment to write — the probe, and the tests that switch commenting
        // off — and then this behaves exactly as it did.
        evidence: Option<&dyn CommentTextSource>,
    ) -> u32 {
        if !self.looks_like_photo_post().await {
            return 0;
        }
        // The counter, if this surface shows it before any turn. A post page does; the feed
        // does not.
        let known_before = self.carousel_position().await;
        if let Some((_, total)) = known_before {
            let wanted = carousel_target(total, portion_percent, ceiling);
            if wanted <= 1 {
                report(
                    status,
                    format!("bài ảnh {total} ảnh — chỉ xem ảnh đầu ({portion_percent}%)"),
                );
                // One slide is still one slide of evidence, and a deferred comment is waiting
                // on it. Returning without offering it would leave the sheet empty and the
                // post silently uncommented for a reason the operator never asked for.
                if let Some(evidence) = evidence {
                    evidence.note_slide();
                }
                return 1;
            }
        }
        report(status, "gặp bài ảnh — vuốt ngang".into());
        // Slide one, before any gesture: the picture the card arrived on, and the one the
        // fingerprint at the top of the post describes.
        if let Some(evidence) = evidence {
            evidence.note_slide();
        }

        let mut seen = 1u32;
        let mut at = known_before.map(|(current, _)| current);
        let mut total = known_before.map(|(_, total)| total);
        // Recomputed as soon as the total is known, which on the feed is after the first
        // turn. Until then the ceiling is the only bound there is.
        let mut wanted = total.map_or(ceiling.max(1), |t| {
            carousel_target(t, portion_percent, ceiling)
        });
        // The picture before the next turn, the witness for a turn whose counter cannot be
        // read. Taken once up front — one capture per photo post — and refreshed only by a
        // turn the picture itself proved, so it is never stale against the turn it judges.
        // `None` on a backend that cannot photograph the screen, and then an unreadable
        // counter ends the traversal rather than being guessed at.
        let mut picture = self.picture_digest(stop).await;

        while seen < wanted {
            if stop.load(Ordering::Relaxed) {
                break;
            }
            if let (Some(now), Some(post_total)) = (at, total) {
                if now >= post_total {
                    // The gate above lets a post that is already on its last image in — the
                    // feed hides the counter until a turn, so the first turn is the first
                    // reading — and that turn is the one that leaves the post.
                    report(
                        status,
                        format!("bài ảnh: đang ở ảnh cuối ({now}/{post_total}) — không vuốt tiếp"),
                    );
                    break;
                }
            }
            if let Err(error) = self.swipe_slide(stop).await {
                report(status, format!("vuốt ngang lỗi: {error}"));
                break;
            }
            let position = self.carousel_position().await;
            // **After `CAROUSEL_SETTLE` and a full hierarchy dump**, which is the most settled
            // this loop ever is, so it needs no new wait and costs no new round trip.
            //
            // The slide is offered to the comment evidence only by an arm that holds proof the
            // page turned. Offering it before reading the counter spent that proof: the same
            // image went to the comment evidence twice and `seen` counted a turn that
            // provably did not happen, so a comment could be published as carousel-grounded
            // over one duplicated image.
            match position {
                Some((now, post_total)) => {
                    if total.is_none() {
                        // First sight of the total: the operator's fraction becomes a real
                        // number of images now, not a fraction of a ceiling.
                        total = Some(post_total);
                        wanted = carousel_target(post_total, portion_percent, ceiling);
                        report(
                            status,
                            format!("bài ảnh {post_total} ảnh — xem {wanted} ({portion_percent}%)"),
                        );
                    }
                    if at == Some(now) {
                        // The counter did not move: no further image on this post.
                        break;
                    }
                    at = Some(now);
                    seen += 1;
                    if let Some(evidence) = evidence {
                        evidence.note_slide();
                    }
                }
                // The counter rendered on this post and a read no longer finds it. Measured
                // 12/08/2026 on an SM-N950F: the counter advances to `N / N` and then
                // **disappears once the last image is passed**. A gesture here would be the
                // one that leaves the post, so this is the end of it — not "could not tell".
                None if at.is_some() => {
                    report(
                        status,
                        "bài ảnh: số ảnh không còn đọc được sau lượt vuốt — coi là hết ảnh, không vuốt tiếp"
                            .into(),
                    );
                    break;
                }
                // No digits, on a post whose counter has never rendered — 6 of 10 photo posts
                // on one fleet run, 2 of 13 on another; it is feed content, not a phone or a
                // build. **Not the same as the end of the post**: a 15-image post was once
                // left at `1/15` for reading it that way (measured 12/08/2026). The picture is
                // the witness instead, and a changed picture still has to be *this card* —
                // the swipe that leaves a post changes the picture too, into the author's
                // profile page, which has no photo badge beside a caption and no rail.
                None => {
                    let now_picture = self.picture_digest(stop).await;
                    let turned = match (picture, now_picture) {
                        (Some(before), Some(after)) => Some(before != after),
                        _ => None,
                    };
                    match turned {
                        Some(true) => {
                            let still_this_card = self.looks_like_photo_post().await
                                && present(self.session, self.labels, TikTokControl::Comments)
                                    .await;
                            if !still_this_card {
                                report(
                                    status,
                                    "bài ảnh: màn hình đổi nhưng không còn thẻ bài ảnh — có thể đã rời thẻ, dừng vuốt ngang"
                                        .into(),
                                );
                                break;
                            }
                            picture = now_picture;
                            seen += 1;
                            if let Some(evidence) = evidence {
                                evidence.note_slide();
                            }
                        }
                        Some(false) => {
                            report(
                                status,
                                "bài ảnh: ảnh không đổi sau lượt vuốt — hết ảnh".into(),
                            );
                            break;
                        }
                        None => {
                            report(
                                status,
                                "bài ảnh: không đọc được số ảnh và không chụp được màn hình — dừng vuốt ngang"
                                    .into(),
                            );
                            break;
                        }
                    }
                }
            }
        }
        report(
            status,
            match total {
                Some(total) => format!("bài ảnh: đã xem {seen}/{total} ảnh"),
                None => format!("bài ảnh: đã xem {seen} ảnh (bản build không hiện số ảnh)"),
            },
        );
        seen
    }

    /// A hash of what is on screen right now, ignoring the status bar, or `None` when the
    /// screen could not be photographed inside [`CAROUSEL_FRAME_BUDGET`].
    ///
    /// `None` is "could not tell", never "unchanged": the callers stop on it.
    async fn picture_digest(&self, stop: &AtomicBool) -> Option<u64> {
        let deadline = tokio::time::Instant::now() + CAROUSEL_FRAME_BUDGET;
        match read_before_deadline(self.session.screenshot_png(), deadline, stop).await {
            Ok(ReadWaitResult::Ready(frame)) => super::picture_digest_of(&frame),
            Ok(ReadWaitResult::Cancelled | ReadWaitResult::DeadlineExceeded) => None,
            Err(error) => {
                tracing::debug!(%error, "nurture carousel screen capture unread");
                None
            }
        }
    }

    /// The default start of the feed swipe in [`Self::swipe_next`].
    fn feed_swipe_from(&self) -> TapPoint {
        TapPoint {
            x: self.screen.0 * 0.5,
            y: self.screen.1 * 0.72,
        }
    }

    /// The start of the vertical swipe off a photo post: [`Self::feed_swipe_from`] when it is
    /// clear of the page-indicator band ([`PHOTO_INDICATOR_ID`]), otherwise the clear point
    /// nearest it in the same column, kept between 50 % and 80 % of the screen like the ad's
    /// start ([`super::ad_card::swipe_start`], with its 36 px clearance for the planner's
    /// scatter). On the [1363,1543] band that is y=1580, in the caption, and on [1481,1661]
    /// it is y=1444, on the image. `None` keeps the usual start: an unmeasured build, a tree
    /// that could not be read inside [`CARD_READ_GRACE`], or a band with no clear point. The
    /// line says what happened when the start moved or could not be checked.
    async fn photo_swipe_from(&self, stop: &AtomicBool) -> (Option<TapPoint>, Option<String>) {
        if self.labels.package() != PHOTO_INDICATOR_PACKAGE
            || self.labels.resource_version() != Some(PHOTO_INDICATOR_BUILD)
        {
            return (None, None);
        }
        let deadline = tokio::time::Instant::now() + CARD_READ_GRACE;
        let tree =
            match read_before_deadline(self.session.hierarchy_source_snapshot(), deadline, stop)
                .await
            {
                Ok(ReadWaitResult::Ready(snapshot)) => {
                    crate::ui_automation::tree::Tree::parse(snapshot).ok()
                }
                _ => None,
            };
        let Some(tree) = tree else {
            return (
                None,
                Some("bài ảnh: không đọc được dải chấm trang ảnh — vuốt dọc từ điểm thường".into()),
            );
        };
        let bands = photo_indicator_bands(&tree, self.labels);
        let target = self.feed_swipe_from();
        let zone = (self.screen.1 * 0.50, self.screen.1 * 0.80);
        match super::ad_card::swipe_start(&bands, &target, zone) {
            Some(from) if (from.y - target.y).abs() < 0.5 => (None, None),
            Some(from) => {
                let line = format!(
                    "bài ảnh: vuốt dọc từ y={:.0} thay vì y={:.0} để tránh dải chấm trang ảnh ({})",
                    from.y,
                    target.y,
                    bands
                        .iter()
                        .map(|band| format!("y {:.0}–{:.0}", band.y, band.y + band.height))
                        .collect::<Vec<_>>()
                        .join(", ")
                );
                (Some(from), Some(line))
            }
            None => (
                None,
                Some(
                    "bài ảnh: không có điểm trống ngoài dải chấm trang ảnh — vuốt dọc từ điểm thường"
                        .into(),
                ),
            ),
        }
    }

    /// Swipe to the next post and report whether the change was provable, with what the
    /// card reads that judged it cost.
    ///
    /// `straight_flick` is set when the card being left handles the gesture itself: a photo
    /// card that was just paged sideways, or an ad end-card. A photo card's own image pager
    /// watches the horizontal axis, and the bow `plan_swipe` puts on a vertical gesture is
    /// horizontal — into exactly that axis — so a bowed, decelerating vertical swipe can be
    /// claimed by the image pager as a short sideways drag, which it ignores (a drag turns
    /// nothing, see [`Self::swipe_slide`]) and the feed never sees. The ad end-card is a Lynx
    /// mask over the card that consumes the touch and forwards only what its own swipe-up
    /// recogniser accepts, as `action_ad_swipe_up_video`, to the feed pager
    /// (`CommerceVideoDelegateKt` in the 45.7.3 APK); see [`super::ad_card`].
    /// Box 31–50 rounds 3–4 (09–10/10/2026): the vertical swipe straight after a paged
    /// carousel was unproven on `ce031713aadf361905` at 00:10:39 and 00:11:13 and on
    /// `ce0317139ca344520c` at 21:38:54, and proven on `ce031713b064841e0c` at 00:09:11
    /// and `ce0317139ca344520c` at 21:35:43 — the odds of a gesture whose bow direction is
    /// drawn per swipe. Leaving such a card therefore uses the flick: jittered ends and
    /// varied legs, no bow and no pause, which is the gesture the pager measured as acting
    /// 19 of 19 times, here sent up the feed axis instead of across it. **Not yet proved on
    /// a phone** as of this change; the live check is the vertical swipe after a carousel
    /// and the swipe off an ad end-card.
    async fn swipe_next(
        &mut self,
        duration_ms: u64,
        before: &PostFingerprint,
        straight_flick: bool,
        // A start the caller has cleared of the card's own touch targets (a sponsored card,
        // see [`super::ad_card::swipe_start`]); `None` keeps [`Self::feed_swipe_from`].
        from: Option<TapPoint>,
        stop: &AtomicBool,
    ) -> anyhow::Result<(bool, CardReads)> {
        use crate::app_automation::dialogs::{
            step_before_deadline_with_tree, PopupBudget, PopupStep,
        };
        let mut reads = CardReads::default();
        if stop.load(Ordering::Relaxed) {
            return Ok((false, reads));
        }
        // A vertical flick through the middle of the screen, clear of the rail on
        // the right and of the system gesture strip at the very bottom.
        //
        // These two are **targets**, not coordinates: `plan_swipe` moves the ends, bows the
        // path and shapes the velocity, so no two feed swipes are the same gesture. Before
        // that, every swipe in every session was the identical dead-straight line between
        // the identical two pixels, traversed at a constant speed — see `super::touch`.
        let from = from.unwrap_or_else(|| self.feed_swipe_from());
        let to = TapPoint {
            x: self.screen.0 * 0.5,
            y: self.screen.1 * 0.28,
        };
        let path = if straight_flick {
            self.planner.plan_flick(from, to, duration_ms)
        } else {
            self.planner.plan_swipe(from, to, duration_ms)
        };
        reads.swipe = path.steps.last().map(|last| {
            [
                path.start.x.round() as i64,
                path.start.y.round() as i64,
                last.point.x.round() as i64,
                last.point.y.round() as i64,
            ]
        });
        let sent = self.session.swipe_path(path).await;
        // Keep the former settle + arrival budget, but an outgoing, non-empty rail
        // is still a pending transition. Read until the card actually changes. A read still
        // running at the deadline finishes and is judged (see [`CARD_READ_GRACE`]).
        let deadline = tokio::time::Instant::now() + SWIPE_SETTLE + RAIL_RETURN_WINDOW;
        if let Err(error) = sent {
            // A swipe the OS cancelled part-way is a navigation that may or may not have
            // moved the feed, never a dead session: the window its DOWN touched closed under
            // it (see [`crate::driver::GestureCancelled`]). One fresh snapshot through the
            // shared popup step, then the same fingerprint judgement as any swipe; an
            // unproven result costs the caller's `stuck_swipes`, so the budget stays the one
            // it was. Any other error, and a failing re-read, stay errors.
            if !error.is::<crate::driver::GestureCancelled>() {
                return Err(error);
            }
            reads.gesture_cancelled = true;
            if !self.session.supports_accessibility_readback() {
                return Ok((false, reads));
            }
            let mut popup = PopupBudget::default();
            let started = tokio::time::Instant::now();
            match step_before_deadline_with_tree(
                self.session,
                self.labels,
                &mut popup,
                deadline,
                stop,
                stop,
            )
            .await?
            {
                ReadWaitResult::Ready((step, tree)) => {
                    reads.completed += 1;
                    reads.last = Some(started.elapsed());
                    reads.popup = popup.says();
                    match (step, tree) {
                        (PopupStep::Clear, Some(tree)) => {
                            let after = tree_fingerprint(&tree, self.labels);
                            if !after.is_empty() && after != *before {
                                return Ok((true, reads));
                            }
                        }
                        (PopupStep::Blocked, _) => return Ok((false, reads)),
                        (PopupStep::Clear | PopupStep::Dismissed, _) => {}
                    }
                }
                ReadWaitResult::Cancelled | ReadWaitResult::DeadlineExceeded => {
                    return Ok((false, reads))
                }
            }
        }
        loop {
            match read_card_in_window(self.session, self.labels, deadline, stop, &mut reads).await?
            {
                ReadWaitResult::Ready(after) if !after.is_empty() && after != *before => {
                    return Ok((true, reads));
                }
                ReadWaitResult::Ready(_) => {}
                ReadWaitResult::Cancelled | ReadWaitResult::DeadlineExceeded => {
                    return Ok((false, reads))
                }
            }
            sleep_interruptible(
                RAIL_RETURN_POLL
                    .min(deadline.saturating_duration_since(tokio::time::Instant::now())),
                stop,
            )
            .await;
        }
    }
}

/// Any TikTok build this project has measured labels for.
///
/// Derived from the catalog rather than written out again: a build we cannot read
/// labels off is not a build this loop can drive, so the two lists must not be
/// able to disagree. The regional packages genuinely differ — the global build is
/// `com.zhiliaoapp.musically`, the South-East Asian one
/// `com.ss.android.ugc.trill`.
fn is_measured_tiktok(package: &str) -> bool {
    crate::tiktok_target::is_measured_android_tiktok(package)
}

/// Make sure a measured TikTok build is in front, and say whether we had to act.
///
/// Reads before it launches. The phone is often already on TikTok — the ordinary
/// case when a device is handed over mid-feed — and relaunching would throw away
/// the feed position for nothing.
///
/// The configured `bundle_id` defaults to the *iOS* bundle
/// (`com.ss.iphone.ugc.Ame`), which no Android phone can launch. That is why a
/// failure here names the package: the fix is a setting, not a retry.
pub(super) async fn ensure_tiktok_foreground(
    session: &dyn UiSession,
    bundle_id: &str,
    stop: &AtomicBool,
) -> anyhow::Result<bool> {
    let deadline = tokio::time::Instant::now() + FOREGROUND_WINDOW;
    match read_before_deadline(session.active_app_bundle(), deadline, stop).await {
        Ok(ReadWaitResult::Ready(package)) if is_measured_tiktok(&package) => return Ok(false),
        Ok(ReadWaitResult::Ready(package))
            if permission_over_tiktok(session, bundle_id, &package, deadline, stop).await =>
        {
            return Ok(false)
        }
        Ok(ReadWaitResult::Ready(_)) => {}
        Ok(ReadWaitResult::Cancelled) => anyhow::bail!("đã dừng trước khi mở TikTok"),
        Ok(ReadWaitResult::DeadlineExceeded) => {
            anyhow::bail!("hết thời gian kiểm tra TikTok foreground")
        }
        Err(error)
            if error
                .downcast_ref::<crate::driver::UnsupportedCapability>()
                .is_some() => {}
        Err(error) => return Err(error),
    }
    if !is_measured_tiktok(bundle_id) {
        anyhow::bail!(
            "TikTok chưa ở foreground và package cấu hình `{bundle_id}` không phải bản Android \
             đã đo nhãn. Đặt bundle id thành một trong: {}",
            crate::tiktok_labels::TIKTOK_LABEL_SETS
                .iter()
                .map(|set| set.package)
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
    anyhow::ensure!(!stop.load(Ordering::Relaxed), "đã dừng trước khi mở TikTok");
    anyhow::ensure!(
        tokio::time::Instant::now() < deadline,
        "hết thời gian mở TikTok"
    );
    session.launch_app_foreground(bundle_id).await?;
    loop {
        match read_before_deadline(session.active_app_bundle(), deadline, stop).await? {
            ReadWaitResult::Ready(package) if is_measured_tiktok(&package) => return Ok(true),
            ReadWaitResult::Ready(package)
                if permission_over_tiktok(session, bundle_id, &package, deadline, stop).await =>
            {
                return Ok(true)
            }
            ReadWaitResult::Ready(_) => {}
            ReadWaitResult::Cancelled => anyhow::bail!("đã dừng khi chờ TikTok foreground"),
            ReadWaitResult::DeadlineExceeded => break,
        }
        sleep_interruptible(
            FOREGROUND_POLL.min(deadline.saturating_duration_since(tokio::time::Instant::now())),
            stop,
        )
        .await;
    }
    // Naming the likeliest cause, because the symptom is indistinguishable from a slow
    // launch and the fix is on the phone rather than in the settings. Hit on 12/08/2026:
    // the SM-N950F had locked its own screen after a long test session, `dumpsys window`
    // showed `isStatusBarKeyguard=true`, and TikTok was running the whole time — `monkey`
    // reports success against a keyguard and nothing moves.
    anyhow::bail!(
        "đã gọi mở {bundle_id} nhưng nó không lên foreground trong {}s. \
         Máy đang khoá màn hình thì đúng ra kết quả này — mở khoá rồi chạy lại",
        FOREGROUND_WINDOW.as_secs()
    )
}

/// The measured runtime permission question over the configured TikTok counts as TikTok in
/// front (see [`crate::tiktok_target::runtime_permission_over_measured_tiktok`]): the launch
/// worked, and the shared popup step owns the question and may only Deny it. Any other
/// foreign window, an unreadable activity or a backend that cannot name one still blocks.
///
/// A permission package caught mid-transition (right after the step's own Deny, before the
/// next question or TikTok has settled) is re-read by the shared classifier for a bounded
/// window rather than relaunched; TikTok coming back during that window counts too. The
/// window is bounded by `deadline` and by the classifier's own cap; `stop` is honoured
/// before it opens and by the callers' next read.
async fn permission_over_tiktok(
    session: &dyn UiSession,
    bundle_id: &str,
    package: &str,
    deadline: tokio::time::Instant,
    stop: &AtomicBool,
) -> bool {
    if package != crate::tiktok_target::RUNTIME_PERMISSION_PACKAGE || stop.load(Ordering::Relaxed) {
        return false;
    }
    matches!(
        crate::tiktok_target::foreground_is_tiktok_or_permission_window(
            session, bundle_id, package, deadline,
        )
        .await,
        crate::tiktok_target::TikTokForeground::TikTok
            | crate::tiktok_target::TikTokForeground::PermissionWindow
    )
}

/// The TikTok build standing behind the measured runtime permission question, for the build
/// check. Riviu #42, box 31-50, 10/10/2026: the contacts question was in front when the
/// session opened, and the check read `com.google.android.packageinstaller 9-7028680` as
/// the TikTok build. The build is the device's app selection when it is a measured TikTok,
/// otherwise the one fleet build, and only when the focused window is
/// `GrantPermissionsActivity` over it. Its installed version is read next, so a build that
/// is not installed is still refused. Any other foreground answers `None`.
async fn tiktok_behind_permission_window(
    session: &dyn UiSession,
    configured: &str,
    foreground: &str,
) -> Option<String> {
    if foreground != crate::tiktok_target::RUNTIME_PERMISSION_PACKAGE {
        return None;
    }
    let configured = configured.trim();
    let target = if is_measured_tiktok(configured) {
        configured
    } else if let [build] = crate::tiktok_fleet_policy::TIKTOK_FLEET_BUILDS {
        build.package
    } else {
        return None;
    };
    let deadline = tokio::time::Instant::now() + FOREGROUND_WINDOW;
    (crate::tiktok_target::foreground_is_tiktok_or_permission_window(
        session, target, foreground, deadline,
    )
    .await
        == crate::tiktok_target::TikTokForeground::PermissionWindow)
        .then(|| target.to_string())
}

/// What one attempt at a hierarchy-driven session came to.
///
/// Three answers rather than two, because the caller has to distinguish "this is
/// an iPhone, use pixels" from "this is an Android nobody has measured" — the
/// first is routine, the second must stop the session rather than fall through to
/// a pixel engine whose only calibrated screen is an iPhone 8.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HierarchySession {
    /// It ran. The outcome is the session verdict.
    Ran(Outcome),
    /// The backend cannot report element geometry; use the pixel path.
    NotSupported,
    /// It could have run, but something measured is missing. The reason is already
    /// in `status.last_message`.
    Refused,
}

/// Drive a whole nurture session by hierarchy, from foregrounding to the last
/// swipe.
///
/// Public because it is the only way to exercise this loop without the control
/// plane, the database, and a stream behind it: the Android G2 probe drives a real
/// phone through exactly this call, so the numbers in the docs come from the
/// shipped code path rather than from a shell transcript.
#[allow(clippy::too_many_arguments)]
pub async fn run_hierarchy_session(
    session: &dyn UiSession,
    screen: (f64, f64),
    settings: &NurtureSettings,
    bundle_id: &str,
    started: Instant,
    max_duration: Option<Duration>,
    stop: &AtomicBool,
    status: &mut NurtureSessionStatus,
    report: &(dyn Fn(&mut NurtureSessionStatus, String) + Send + Sync),
    comments: Option<&dyn CommentTextSource>,
    live: Option<&dyn LiveSettings>,
) -> HierarchySession {
    run_hierarchy_session_inner(
        session,
        screen,
        settings,
        bundle_id,
        started,
        max_duration,
        stop,
        status,
        report,
        comments,
        live,
        None,
        None,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn run_hierarchy_session_with_save_intent(
    session: &dyn UiSession,
    screen: (f64, f64),
    settings: &NurtureSettings,
    bundle_id: &str,
    started: Instant,
    max_duration: Option<Duration>,
    stop: &AtomicBool,
    status: &mut NurtureSessionStatus,
    report: &(dyn Fn(&mut NurtureSessionStatus, String) + Send + Sync),
    comments: Option<&dyn CommentTextSource>,
    live: Option<&dyn LiveSettings>,
    save_journal: &dyn NurtureSaveJournal,
    follow_journal: &dyn NurtureFollowJournal,
) -> HierarchySession {
    run_hierarchy_session_inner(
        session,
        screen,
        settings,
        bundle_id,
        started,
        max_duration,
        stop,
        status,
        report,
        comments,
        live,
        Some(save_journal),
        Some(follow_journal),
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn run_hierarchy_session_inner(
    session: &dyn UiSession,
    screen: (f64, f64),
    settings: &NurtureSettings,
    bundle_id: &str,
    started: Instant,
    max_duration: Option<Duration>,
    stop: &AtomicBool,
    status: &mut NurtureSessionStatus,
    report: &(dyn Fn(&mut NurtureSessionStatus, String) + Send + Sync),
    comments: Option<&dyn CommentTextSource>,
    live: Option<&dyn LiveSettings>,
    save_journal: Option<&dyn NurtureSaveJournal>,
    follow_journal: Option<&dyn NurtureFollowJournal>,
) -> HierarchySession {
    if !session.supports_element_bounds() {
        return HierarchySession::NotSupported;
    }
    let effective_settings = settings.clone().into_effective();
    let settings = &effective_settings;
    match ensure_tiktok_foreground(session, bundle_id, stop).await {
        Ok(true) => report(status, "đã đưa TikTok lên foreground".into()),
        Ok(false) => {}
        Err(error) => {
            report(status, format!("failed — {error}"));
            return HierarchySession::Refused;
        }
    }
    let mut run = match HierarchyRun::prepare(session, screen, settings).await {
        Ok(run) => run,
        Err(Unsupported::NoElementBounds) => return HierarchySession::NotSupported,
        Err(refusal) => {
            report(status, refusal.message());
            return HierarchySession::Refused;
        }
    };
    report(status, run.labels_note());
    if !await_feed(&run, stop, status, report).await {
        return HierarchySession::Refused;
    }
    if settings.feed_source == crate::types::NurtureFeedSource::Search {
        let deadline = max_duration.map_or_else(
            || Instant::now() + Duration::from_secs(75),
            |max| started + max,
        );
        match super::search::open(session, bundle_id, &settings.search_keyword, stop, deadline)
            .await
        {
            Ok(()) => {
                run.search_keyword = Some(settings.search_keyword.trim().into());
            }
            Err(error) => {
                report(status, format!("Không mở được video theo từ khóa: {error}"));
                return HierarchySession::Refused;
            }
        }
    }
    if let Err(error) = await_first_rail(&run, stop).await {
        report(
            status,
            format!("failed — không đọc được rail TikTok: {error}"),
        );
        return HierarchySession::Refused;
    }
    let outcome = run_feed(
        run,
        bundle_id,
        settings,
        started,
        max_duration,
        stop,
        status,
        report,
        comments,
        live,
        save_journal,
        follow_journal,
    )
    .await;
    HierarchySession::Ran(outcome)
}

/// How long the feed gets to appear before the loop is refused, and how often to look.
///
/// Measured on the SM-N950F (Android 8), 12/08/2026: after `am force-stop`, the feed tab
/// `Đề xuất` first became readable **23,8 s** after the launch intent, while the package
/// itself reached the foreground at 16–27 s. So "TikTok is in front" and "the feed is up"
/// are seconds apart on this phone, and the gap is spent on the splash screen.
///
/// That gap had a visible cost. A session started right after a cold launch reported
/// `partial — 0/0 video, 14s`: the loop found no feed tab, took the off-feed branch, and
/// spent its whole `OFF_FEED_LIMIT` of 6 swipes on the splash before giving up — 14 s,
/// which is less than the phone needs. The off-feed branch is right for what it was built
/// for (an ad card, a LIVE card, a card mid-transition); it is the wrong tool for an app
/// that has not finished starting.
///
/// Waiting is bounded and ends in a refusal, not a guess: a phone parked on TikTok's
/// interest-picker onboarding never shows a feed, and saying so beats six blind swipes.
const FEED_READY_WINDOW: Duration = Duration::from_secs(30);
const FEED_READY_POLL: Duration = Duration::from_millis(1_000);

/// How long a blocked screen is given to resolve itself before Back is pressed on it.
///
/// A third of the window, and the reason is the one case Back would make worse: TikTok's
/// splash shows neither a feed tab nor a bottom bar either, and Back at a splash closes the
/// app rather than a dialog. Ten seconds is long enough that a phone still starting is past
/// it, and short enough to leave two thirds of the window for the feed to arrive afterwards.
const MODAL_BACK_DELAY: Duration = Duration::from_secs(10);

/// How many times Back may be pressed at a screen that no label can reach.
///
/// One was enough for a dialog, which is what this rung was built for, and one is not
/// enough for a *stack*. Measured 19/08/2026 on ce0417145199e0490c: a whole-fleet run
/// lost one phone, and it was sitting on TikTok's `SearchResultActivity` — left there by
/// whatever ran last. Search results are two screens deep, so one Back reached the search
/// page, which has no feed tab and no Home tab either, and the session ended at zero
/// videos with `chờ 30s mà chưa thấy tab feed`.
///
/// Bounded, and each press is re-judged from the top of the loop rather than fired in a
/// burst: the moment one of them lands on the feed, `on_feed` returns and the rest are
/// never spent. Three covers the stacks TikTok actually builds; a screen that Back cannot
/// leave still ends inside [`FEED_READY_WINDOW`] instead of looping on it.
const MODAL_BACK_LIMIT: u32 = 3;

/// Give the first card's action rail the same grace every later card already gets.
///
/// **The feed's chrome renders before its first card does.** `await_feed` is satisfied by the
/// `For You` tab, which is part of the tab bar and appears while the video underneath is
/// still loading — so the loop's first read found no rail, called the card railless, swiped,
/// and did it again until `OFF_FEED_LIMIT` ran out. Measured 18/08/2026 on a six-phone run:
/// three sessions ended `0/2 video` in about fifteen seconds, having swiped past six cards
/// that were never given time to draw, on phones that showed a complete rail when scouted a
/// minute later.
///
/// `swipe_next` has waited for the incoming rail since the day it was measured
/// ([`RAIL_RETURN_WINDOW`]); only the card the session *starts* on was missing it, because no
/// swipe precedes it.
///
/// Absence is tolerated rather than treated as failure, for the same reason as there: a LIVE
/// card or a photo carousel genuinely has no rail, and it simply uses up the window.
async fn await_first_rail(run: &HierarchyRun<'_>, stop: &AtomicBool) -> anyhow::Result<()> {
    // Twice: once for a card that is merely slow, and once more after selecting the For-You
    // tab, for a phone that is on the feed but not on *that* part of it.
    //
    // **`For You` being on screen does not mean it is the tab in front.** It sits in a strip
    // beside `Following`, `Friends` and `Explore`, all four visible whichever is selected —
    // and Explore is a grid with no per-post rail at all. So `on_feed` was satisfied,
    // the loop found no rail, and it swiped through its whole off-feed budget on a screen
    // that has nothing to swipe. Measured 18/08/2026: three phones of six failed this way in
    // about twenty seconds each, and the same phones showed a complete rail a minute later
    // once something had tapped For You.
    for attempt in 0..2 {
        let deadline = tokio::time::Instant::now()
            + RAIL_RETURN_WINDOW
            + if attempt == 1 {
                SWIPE_SETTLE
            } else {
                Duration::ZERO
            };
        if attempt == 1 {
            let element = match read_before_deadline(
                locate(run.session, run.labels, TikTokControl::FeedTab),
                deadline,
                stop,
            )
            .await?
            {
                ReadWaitResult::Ready(Some(element)) => element,
                ReadWaitResult::Ready(None) | ReadWaitResult::DeadlineExceeded => return Ok(()),
                ReadWaitResult::Cancelled => anyhow::bail!("đã dừng khi chờ rail đầu tiên"),
            };
            anyhow::ensure!(
                !stop.load(Ordering::Relaxed),
                "đã dừng trước khi chọn tab feed"
            );
            run.session.tap(element.centre()).await?;
        }
        let mut reads = CardReads::default();
        loop {
            match read_card_in_window(run.session, run.labels, deadline, stop, &mut reads).await? {
                ReadWaitResult::Ready(card) if !card.is_empty() => return Ok(()),
                ReadWaitResult::Ready(_) => {}
                ReadWaitResult::Cancelled => anyhow::bail!("đã dừng khi chờ rail đầu tiên"),
                ReadWaitResult::DeadlineExceeded => break,
            }
            sleep_interruptible(
                RAIL_RETURN_POLL
                    .min(deadline.saturating_duration_since(tokio::time::Instant::now())),
                stop,
            )
            .await;
        }
    }
    Ok(())
}

/// Leave a railless card: one vertical swipe with the two `HumanBehavior` pauses every feed
/// swipe takes, judged by [`HierarchyRun::swipe_next`] against `before`. The caller decides
/// what an unproven departure costs and whether the card counts as a video.
async fn leave_railless_card(
    run: &mut HierarchyRun<'_>,
    human: &mut HumanBehavior,
    before: &PostFingerprint,
    straight_flick: bool,
    stop: &AtomicBool,
    status: &mut NurtureSessionStatus,
    report: &(dyn Fn(&mut NurtureSessionStatus, String) + Send + Sync),
) -> anyhow::Result<(bool, CardReads)> {
    status.swipe_attempts += 1;
    sleep_interruptible(Duration::from_millis(human.think_pause_ms()), stop).await;
    let judged = run
        .swipe_next(
            human.swipe_duration_ms(false),
            before,
            straight_flick,
            None,
            stop,
        )
        .await?;
    if let Some(line) = judged.1.cancelled_says() {
        report(status, line);
    }
    if !stop.load(Ordering::Relaxed) {
        sleep_interruptible(Duration::from_millis(human.after_swipe_pause_ms()), stop).await;
    }
    Ok(judged)
}

/// Wait for the rail of a card that came up railless right after an unproven swipe.
///
/// Bounded by [`LATE_RAIL_WINDOW`] (plus one read in flight, [`CARD_READ_GRACE`]) and the
/// Stop flag. A read error stays an error, never an empty rail. `reads` is filled either way,
/// for the line the caller writes.
async fn await_late_rail(
    run: &HierarchyRun<'_>,
    stop: &AtomicBool,
    reads: &mut CardReads,
) -> anyhow::Result<ReadWaitResult<PostFingerprint>> {
    let deadline = tokio::time::Instant::now() + LATE_RAIL_WINDOW;
    loop {
        sleep_interruptible(
            RAIL_RETURN_POLL.min(deadline.saturating_duration_since(tokio::time::Instant::now())),
            stop,
        )
        .await;
        match read_card_in_window(run.session, run.labels, deadline, stop, reads).await? {
            ReadWaitResult::Ready(card) if !card.is_empty() => {
                return Ok(ReadWaitResult::Ready(card));
            }
            ReadWaitResult::Ready(_) => {}
            ended @ (ReadWaitResult::Cancelled | ReadWaitResult::DeadlineExceeded) => {
                return Ok(ended);
            }
        }
    }
}

/// Wait for the feed to be on screen. `false` means it never was.
///
/// Cheap in the ordinary case — TikTok already on the feed answers on the first query and
/// nothing is slept — so this is not a fixed cost paid by every session.
async fn await_feed(
    run: &HierarchyRun<'_>,
    stop: &AtomicBool,
    status: &mut NurtureSessionStatus,
    report: &(dyn Fn(&mut NurtureSessionStatus, String) + Send + Sync),
) -> bool {
    if run.search_keyword.is_some() {
        report(
            status,
            "Đã rời video tìm kiếm — dừng để giữ đúng nguồn từ khóa".into(),
        );
        return false;
    }
    let started = tokio::time::Instant::now();
    let deadline = started + FEED_READY_WINDOW;
    let mut said = false;
    // The rungs, their order and the argument for each live in [`crate::feed_ladder`]; the
    // idle sweeper drives the same ones. What stays here is this caller's *budget*: a
    // thirty-second window, a poll every second, and Back held back for ten of those.
    let mut spend = LadderSpend::new(MODAL_BACK_LIMIT);
    loop {
        if stop.load(Ordering::Relaxed) {
            return false;
        }

        // No unbounded final probe: a late tree cannot extend the caller's window
        // or authorize another dismissal after this session has ended.
        if tokio::time::Instant::now() >= deadline {
            report(
                status,
                format!(
                    "failed — chờ {}s mà chưa thấy tab feed. TikTok có thể còn ở màn khởi \
                     động, hoặc đang ở trang chọn chủ đề / đăng nhập — dừng thay vì vuốt mù",
                    FEED_READY_WINDOW.as_secs()
                ),
            );
            return false;
        }

        // Patience is this caller's to spend, not the ladder's. Ten seconds before Back
        // becomes available, so a slow splash screen is waited out rather than answered
        // with a keypress — see [`MODAL_BACK_DELAY`].
        spend.allow_back = started.elapsed() >= MODAL_BACK_DELAY;
        let step = match feed_ladder::step_before_deadline(
            run.session,
            run.labels,
            &mut spend,
            deadline,
            stop,
        )
        .await
        {
            Ok(ReadWaitResult::Ready(step)) => step,
            Ok(ReadWaitResult::Cancelled) => return false,
            Ok(ReadWaitResult::DeadlineExceeded) => continue,
            Err(error) => {
                report(
                    status,
                    format!("failed — không đọc được màn hình feed: {error}"),
                );
                return false;
            }
        };
        if stop.load(Ordering::Relaxed) {
            return false;
        }
        if step == LadderStep::OnFeed {
            if said {
                report(status, "feed đã lên".into());
            }
            return true;
        }
        if let Some(line) = step.says() {
            report(status, line);
        } else if !said {
            said = true;
            report(status, "TikTok đang khởi động — chờ feed lên".into());
        }
        sleep_interruptible(
            FEED_READY_POLL.min(deadline.saturating_duration_since(tokio::time::Instant::now())),
            stop,
        )
        .await;
    }
}

/// What the screen is, judged only by measured shapes, for the one recovery this loop
/// performs itself: a single Back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FeedScreen {
    /// The feed tab is on screen.
    Feed,
    /// TikTok's main shell on another tab: the bottom bar is there and the feed tab is not.
    /// The ladder's Home rung owns this one; Back here would be a guess.
    OtherTab,
    /// A TikTok screen that one Back is known to leave, named for the operator.
    NonFeed(&'static str),
    /// Nothing measured matched, or the screen could not be read. Nothing is pressed.
    Unknown,
}

/// Classify the screen from one hierarchy snapshot.
///
/// The shapes, in the order they are asked, each measured on the Global build:
///
/// * the feed tab (`For You` / `Đề xuất`, the catalogued [`TikTokControl::FeedTab`]);
/// * the bottom bar's Home tab ([`TikTokControl::HomeTab`]) — **selected**, it proves the
///   feed as well, because the Home tab is only selected while the feed pager is in front
///   (`selected="true"` on `…:id/nr_` in
///   `fixtures/tiktok-publish/musically-45.7.3-en/real-20261009-home-feed.xml`, read off
///   a real phone). Neither proof needs the action rail: an ad end-card hides the whole
///   interact area and leaves both tabs on screen (box 31–50 round 5, 10/10/2026,
///   `ce031713b064841e0c`, `evidence-h/b064-norail.png`). An unselected Home tab is some
///   other bottom tab;
/// * the author's profile page — `Followers`, `Following` and `Likes` as visible text, the
///   header the 45.7.3/en fixtures `tiktok-follow/global-profile-{before,after}.fixture`
///   carry, or the Vietnamese pair `comment_verification::search` reads on a profile;
/// * the expanded photo viewer, [`crate::tiktok_share::photo_proof::expanded_photo_surface`],
///   measured per build from its own fixtures;
/// * failing those, the build's Back control ([`TikTokControls::back_descriptions`]) with no
///   tab bar at all — a sub-screen inside TikTok, whatever it is.
///
/// A backend without snapshots answers only the first question, through `on_feed`.
async fn classify_screen(run: &HierarchyRun<'_>) -> FeedScreen {
    match screen_tree(run).await {
        Some(tree) => classify_tree(&tree, run.labels),
        None if run.on_feed().await => FeedScreen::Feed,
        None => FeedScreen::Unknown,
    }
}

/// One parsed snapshot, or `None` when the backend has none or it did not parse. The callers
/// fall back to the feed-tab query, which is all a snapshot-less backend can answer.
async fn screen_tree(run: &HierarchyRun<'_>) -> Option<crate::ui_automation::tree::Tree> {
    match run.session.hierarchy_source_snapshot().await {
        Ok(snapshot) => match crate::ui_automation::tree::Tree::parse(snapshot) {
            Ok(tree) => Some(tree),
            Err(error) => {
                tracing::debug!(%error, "nurture screen snapshot unparsed");
                None
            }
        },
        Err(_) => None,
    }
}

/// [`classify_screen`] on a tree already in hand.
fn classify_tree(tree: &crate::ui_automation::tree::Tree, labels: TikTokControls) -> FeedScreen {
    let package = labels.package();
    let found = |control: TikTokControl| -> Vec<usize> {
        labels
            .label(control)
            .map(|label| match label.to_query() {
                ElementQuery::Semantic(role) => {
                    crate::app_automation::tiktok_roles::indices(tree, package, role)
                }
                query => tree.matching(package, query),
            })
            .unwrap_or_default()
    };
    if !found(TikTokControl::FeedTab).is_empty() {
        return FeedScreen::Feed;
    }
    let home = found(TikTokControl::HomeTab);
    if !home.is_empty() {
        return if home
            .iter()
            .any(|index| tree.nodes[*index].attr("selected") == "true")
        {
            FeedScreen::Feed
        } else {
            FeedScreen::OtherTab
        };
    }
    let visible = |attribute: &str, value: &str| {
        tree.nodes.iter().enumerate().any(|(index, node)| {
            node.visible(package) && tree.ancestors_visible(index) && node.attr(attribute) == value
        })
    };
    let profile = ["Followers", "Following", "Likes"]
        .iter()
        .all(|text| visible("text", text))
        || ["Người theo dõi", "Đang Follow"]
            .iter()
            .all(|text| visible("text", text));
    if profile {
        return FeedScreen::NonFeed("trang hồ sơ tác giả");
    }
    if labels.resource_version().is_some_and(|version| {
        crate::tiktok_share::photo_proof::expanded_photo_surface(tree, package, version)
    }) {
        return FeedScreen::NonFeed("trình xem ảnh phóng to");
    }
    if labels
        .back_descriptions()
        .iter()
        .any(|value| visible("content-desc", value))
    {
        return FeedScreen::NonFeed("màn hình con của TikTok (có nút Back, không có tab feed)");
    }
    FeedScreen::Unknown
}

/// Press Back once, from a screen [`classify_screen`] proved is a TikTok screen Back
/// leaves, and wait [`BACK_RETURN_WINDOW`] for the feed tab. `false` when nothing was
/// pressed or the feed did not come back; the caller decides what that costs.
///
/// One press, never a burst: the second press on a screen that did not move is the one
/// that closes TikTok from its root, and a session that has to be restarted costs more
/// than the ladder in [`await_feed`] this hands over to.
async fn back_to_feed(
    run: &HierarchyRun<'_>,
    stop: &AtomicBool,
    status: &mut NurtureSessionStatus,
    report: &(dyn Fn(&mut NurtureSessionStatus, String) + Send + Sync),
    why: &str,
) -> bool {
    if stop.load(Ordering::Relaxed) {
        return false;
    }
    // The same foreground proof the ladder takes before its own Back: a key sent to
    // whatever is in front is only ours to send when that is TikTok.
    if run.session.supports_accessibility_readback() {
        match run.session.active_app_bundle().await {
            Ok(bundle) if bundle == run.labels.package() => {}
            _ => {
                report(
                    status,
                    format!("đang ở {why} nhưng không chứng minh được TikTok ở foreground — không bấm Back"),
                );
                return false;
            }
        }
    }
    report(
        status,
        format!("đang ở {why} — bấm Back một lần để về feed"),
    );
    if let Err(error) = run.session.back().await {
        report(status, format!("bấm Back không được: {error}"));
        return false;
    }
    let deadline = tokio::time::Instant::now() + BACK_RETURN_WINDOW;
    // The feed Back returns to can come up under a TikTok dialog: box 31-50 round 6,
    // 10/10/2026 08:50:53-57, ce071827715f321b05 (Global 45.7.3/en), where "Link email"
    // opened over For You right after Back from the author's profile and the snapshot held
    // only the dialog's window. Each poll therefore asks the shared popup step first, as
    // the ladder does, so a catalogued dialog is answered here instead of being read as
    // "no feed tab". A blocking or unreadable screen ends the wait; [`await_feed`] owns it.
    let screened = run.search_keyword.is_none() && run.session.supports_accessibility_readback();
    let mut popup = crate::app_automation::dialogs::PopupBudget::default();
    loop {
        sleep_interruptible(
            BACK_RETURN_POLL.min(deadline.saturating_duration_since(tokio::time::Instant::now())),
            stop,
        )
        .await;
        if stop.load(Ordering::Relaxed) {
            return false;
        }
        let on_feed = if screened {
            use crate::app_automation::dialogs::{step_before_deadline_with_tree, PopupStep};
            match step_before_deadline_with_tree(
                run.session,
                run.labels,
                &mut popup,
                deadline,
                stop,
                stop,
            )
            .await
            {
                Ok(ReadWaitResult::Ready((PopupStep::Clear, Some(tree)))) => {
                    classify_tree(&tree, run.labels) == FeedScreen::Feed
                }
                Ok(ReadWaitResult::Ready((PopupStep::Clear, None))) => run.on_feed().await,
                Ok(ReadWaitResult::Ready((PopupStep::Dismissed, _))) => {
                    report(
                        status,
                        popup
                            .says()
                            .unwrap_or("đóng một hộp thoại tùy chọn đã đo; chờ đọc lại màn hình")
                            .to_string(),
                    );
                    false
                }
                // No operator line: the read predates the dismissal just sent (or a spent
                // decline); the next poll reads again within the same window.
                Ok(ReadWaitResult::Ready((PopupStep::Blocked, _))) if popup.says().is_none() => {
                    false
                }
                Ok(ReadWaitResult::Ready((PopupStep::Blocked, _))) | Err(_) => return false,
                Ok(ReadWaitResult::Cancelled) => return false,
                Ok(ReadWaitResult::DeadlineExceeded) => false,
            }
        } else {
            run.on_feed().await
        };
        if on_feed {
            report(status, "feed đã lên sau Back".into());
            return true;
        }
        if tokio::time::Instant::now() >= deadline {
            report(
                status,
                format!(
                    "chờ {}s sau Back mà chưa thấy tab feed",
                    BACK_RETURN_WINDOW.as_secs()
                ),
            );
            return false;
        }
    }
}

/// Get the feed back on screen: one Back from a proved non-feed screen first, the ladder
/// in [`await_feed`] for everything else, including a Back that did not work.
///
/// Cheap when the feed is already there — one snapshot — which is the ordinary case after a
/// carousel. The reason it exists is the other case: box 31–50 round 3, 09/10/2026
/// (`ce0317139ca344520c` 21:39:13–21:39:28), where the ladder's own route back from the
/// author's profile page was "TikTok đang khởi động — chờ feed lên", ten seconds of
/// waiting for a splash that was not there, then Back. The screen was readable the whole
/// time; nothing had asked it what it was.
async fn recover_feed(
    run: &HierarchyRun<'_>,
    stop: &AtomicBool,
    status: &mut NurtureSessionStatus,
    report: &(dyn Fn(&mut NurtureSessionStatus, String) + Send + Sync),
) -> bool {
    match classify_screen(run).await {
        FeedScreen::Feed => true,
        FeedScreen::NonFeed(why) => {
            back_to_feed(run, stop, status, report, why).await
                || await_feed(run, stop, status, report).await
        }
        FeedScreen::OtherTab | FeedScreen::Unknown => await_feed(run, stop, status, report).await,
    }
}

/// Run a hierarchy-driven session, updating `status` as it goes.
///
/// `bundle_id` is TikTok's app id **on this device**, resolved by the caller rather than
/// read from `settings.bundle_id`. That field is one global row for the whole fleet holding
/// an *iOS* bundle by default, so on Android it produced `monkey -p com.ss.iphone.ugc.Ame`
/// and failed on a phone whose TikTok was installed and working — measured through the app
/// on 12/08/2026. `NurtureEngine::tiktok_bundle_for` resolves it per device.
///
/// Mirrors the shape of the pixel engine's loop — dwell, decide, act, swipe,
/// rest — using the same policy objects, so a session on Android paces like a
/// session on iOS rather than like a script.
/// Write the comment for the post in front of us and send it, recording whichever way it
/// ended.
///
/// Lifted verbatim out of the `FeedAction::Comment` arm when the deferral gave it a second
/// caller. The outcome distinguishes a real gesture from a refusal before any UI was touched,
/// so the caller can refund only a provable no-op.
///
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RolledCommentOutcome {
    sent: bool,
    did_act: bool,
}

fn settle_attempt(
    policy: &mut HumanSessionPolicy,
    reservation: AttemptReservation,
    did_act: bool,
    attempt_counter: &mut u32,
) -> bool {
    if did_act {
        policy.commit_attempt(reservation);
        *attempt_counter += 1;
        true
    } else {
        policy.cancel_no_effect(reservation);
        false
    }
}

fn settle_action_failure(
    policy: &mut HumanSessionPolicy,
    reservation: AttemptReservation,
    failure: &ActionFailure,
    attempt_counter: &mut u32,
) -> bool {
    settle_attempt(
        policy,
        reservation,
        failure.effect_may_have_gone_out(),
        attempt_counter,
    )
}

async fn post_rolled_comment(
    run: &mut HierarchyRun<'_>,
    source: &dyn CommentTextSource,
    settings: &NurtureSettings,
    grounded_on: &PostFingerprint,
    stop: &AtomicBool,
    status: &mut NurtureSessionStatus,
    report: &(dyn Fn(&mut NurtureSessionStatus, String) + Send + Sync),
) -> RolledCommentOutcome {
    let mut prepared = match source.comment_for_post(settings).await {
        Ok(Some(prepared)) => prepared,
        Err(CommentSourceError::AuditUnavailable) => {
            report(
                status,
                "bỏ qua bình luận: không ghi được audit trước hành động".into(),
            );
            return RolledCommentOutcome {
                sent: false,
                did_act: false,
            };
        }
        Ok(None) => {
            report(
                status,
                format!(
                    "bỏ qua bình luận: {}",
                    CommentVerdict::ContextSkipped.reason()
                ),
            );
            return RolledCommentOutcome {
                sent: false,
                did_act: false,
            };
        }
    };
    if !prepared.audit_attempt.is_verified() {
        report(
            status,
            "bỏ qua bình luận: audit attempt không có bằng chứng đã ghi bền trước hành động".into(),
        );
        return RolledCommentOutcome {
            sent: false,
            did_act: false,
        };
    }
    // **The comment is grounded on the card that was on screen when we started; only that
    // card may receive it.** Drafting collects frames and then calls a vision model, which
    // takes seconds, and a feed can advance underneath a slow draft — so a comment written
    // about card A would otherwise be typed into card B's drawer, publicly and undoably, on
    // a real account. The deferred (photo) path has re-read the card for exactly this reason
    // since the carousel work; the immediate path drafted and typed without ever looking
    // again. `is_empty()` (no rail — mid-transition or a LIVE card) is treated as "changed":
    // if we cannot see the card we grounded on, we do not type. Found by the 31/08/2026
    // whole-file review; the deferred path's guard is the sibling of this one.
    let now = fingerprint(run.session, run.labels).await;
    if now.is_empty() || now != *grounded_on {
        report(
            status,
            "bỏ qua bình luận: thẻ đã đổi giữa lúc soạn và lúc gõ — không bình luận nhầm bài"
                .into(),
        );
        source
            .record_outcome(&prepared, "skipped: card_changed")
            .await;
        return RolledCommentOutcome {
            sent: false,
            did_act: false,
        };
    }
    if !prepared.audit_attempt.consume_for(&prepared.text) {
        report(
            status,
            "bỏ qua bình luận: audit attempt đã dùng hoặc không khớp nội dung đã ghi bền".into(),
        );
        return RolledCommentOutcome {
            sent: false,
            did_act: false,
        };
    }
    match run.comment(&prepared.text, stop).await {
        Ok(CommentVerdict::Sent) => {
            status.comments += 1;
            status.session_prompt_tokens += prepared.prompt_tokens;
            status.session_completion_tokens += prepared.completion_tokens;
            report(status, "đã gửi bình luận (nút Gửi tắt lại)".into());
            source.record_outcome(&prepared, "sent").await;
            RolledCommentOutcome {
                sent: true,
                did_act: true,
            }
        }
        Ok(verdict) => {
            report(status, format!("bỏ qua bình luận: {}", verdict.reason()));
            // **The verdict, not just "skipped".** A comment that was written, scored and
            // then not posted is a device-level failure, and the four ways it happens want
            // four different answers: the drawer never opened, the Send control was not
            // there, it never armed, or the tap could not be proved. The audit row said the
            // same word for all of them, so the only place the distinction survived was a
            // status line nobody stores.
            source
                .record_outcome(&prepared, &format!("skipped: {verdict:?}"))
                .await;
            RolledCommentOutcome {
                sent: false,
                did_act: !matches!(
                    verdict,
                    CommentVerdict::SendUnmeasured
                        | CommentVerdict::NoControl
                        | CommentVerdict::ContextSkipped
                ),
            }
        }
        Err(error) => {
            report(status, format!("bình luận thất bại: {error}"));
            source
                .record_outcome(&prepared, &format!("failed: {error}"))
                .await;
            // A transport error does not prove the drawer or Send were untouched.
            RolledCommentOutcome {
                sent: false,
                did_act: error.effect_may_have_gone_out(),
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn run_feed(
    mut run: HierarchyRun<'_>,
    // TikTok's package on *this* phone, for the one recovery that has to restart it.
    bundle_id: &str,
    settings: &NurtureSettings,
    started: Instant,
    max_duration: Option<Duration>,
    stop: &AtomicBool,
    status: &mut NurtureSessionStatus,
    // `Send + Sync` because the whole session runs inside a spawned task: without
    // the bounds the returned future is not `Send` and the Tauri command that
    // drives nurture will not compile.
    report: &(dyn Fn(&mut NurtureSessionStatus, String) + Send + Sync),
    comments: Option<&dyn CommentTextSource>,
    live: Option<&dyn LiveSettings>,
    save_journal: Option<&dyn NurtureSaveJournal>,
    follow_journal: Option<&dyn NurtureFollowJournal>,
) -> Outcome {
    // The loop's own copy, because from here on the operator can change it. Everything
    // below reads `settings` per post already, so owning it is all that was missing.
    let mut settings = settings.clone();
    let mut human = HumanBehavior::new(
        &settings.persona,
        settings.fatigue,
        settings.time_of_day,
        settings.pause_swipe,
    );
    let mut policy = HumanSessionPolicy::new_with_save(
        settings.like_prob,
        settings.comment_prob,
        settings.save_prob,
        settings.follow_prob,
        settings.human_limits,
    );
    // With the pacing switched off the mood cycle has to stop scaling the probabilities
    // too, or `Skimming`'s `like_mult = 0.0` silently keeps overriding a 100 % setting.
    let mut moods = if !settings.human_limits {
        MoodCycle::neutral()
    } else {
        match settings.steady_mood.as_str() {
            "chatty" => MoodCycle::fixed(crate::human_behavior::Mood::Chatty),
            "liking" => MoodCycle::fixed(crate::human_behavior::Mood::Liking),
            "skimming" => MoodCycle::fixed(crate::human_behavior::Mood::Skimming),
            _ => MoodCycle::new(),
        }
    };

    // Whether commenting is *possible* is fixed for the session — a text source that was
    // not wired and a Gửi button nobody measured cannot appear mid-run. Whether it is
    // *wanted* is `settings.comment_prob`, which the operator can change, so that half is
    // re-read per post below. Splitting them is what keeps a live "bật Bình luận" from
    // being silently ignored by a flag computed before it was flipped.
    let comment_capable =
        comments.is_some() && run.labels.label(TikTokControl::CommentSend).is_some();
    // Said once at the start rather than per post, and only when the session opened
    // wanting comments it cannot post.
    if settings.comment_prob > 0 && !comment_capable {
        let why = if comments.is_none() {
            "chưa nối nguồn chữ"
        } else {
            "chưa đo nút Gửi của bản build này"
        };
        report(
            status,
            format!("bỏ qua bình luận cả phiên: {why}. Phiên vẫn chạy tim/follow/xem"),
        );
    }
    // Same shape as the line above, for the same reason: a feature the operator switched on
    // that this build cannot perform must say so once, not fail quietly for a whole session.
    // Only when the carousel is actually wanted — `carousel_ceiling()` is 0 with the switch
    // off, and announcing a gap in a feature nobody asked for is noise.
    if settings.carousel_ceiling() > 0 && !can_page_carousel(run.labels) {
        report(
            status,
            "bỏ qua vuốt ngang cả phiên: chưa đo nhãn bài ảnh cho bản build này. \
             Bài ảnh vẫn được xem và tương tác như video, chỉ không lướt qua từng ảnh"
                .into(),
        );
    }

    let total_videos = video_target(&settings);
    // **Count videos, not passes.** The loop used to run exactly `total_videos` passes, so
    // every pass that counted nothing — an unproven swipe, a railless card swiped past, an
    // off-feed walk back, a restart — lowered the result: on the box 31–50 acceptance run of
    // 10/10/2026 one phone watched 15 cards and counted 12 videos. It now runs until the
    // target is counted, inside a finite budget of half as many passes again. The target
    // stays a cap, and the duration and night caps below still end the session first.
    let pass_budget = total_videos.saturating_add(total_videos.div_ceil(2));
    let videos_at_start = status.videos_done;
    let mut passes = 0u32;

    let mut off_feed_streak = 0u32;
    let mut stuck_swipes = 0u32;
    let mut restarted = false;
    let mut last_interaction_at: Option<Instant> = None;
    // The card an interaction was last spent on. A swallowed swipe leaves that same card on
    // screen, and the next pass would `begin_post()` and comment/like/follow it a SECOND
    // time — a duplicate public action, on a real account, with no delete path. So a pass
    // whose card fingerprint matches this one spends no interaction; it only tries to leave.
    // Cleared the moment a swipe is proven to have changed the card.
    let mut last_acted_fingerprint: Option<PostFingerprint> = None;
    // The railed card the previous pass swiped away from without proof. Only the very next
    // pass may use it: it waits for a railless card's rail instead of spending the pass on
    // swiping it past, and counts the earlier swipe only when the rail returns on another card.
    let mut unproven_swipe_from: Option<PostFingerprint> = None;
    // Consecutive passes that found no action rail. See [`NO_RAIL_STREAK_LIMIT`] and
    // [`NO_RAIL_ON_FEED_LIMIT`].
    let mut no_rail_streak = 0u32;
    // Whether the one Close this loop may press on an ad end-card has been pressed during
    // the current railless streak. One per card: the streak ends with the next railed card.
    let mut ad_close_pressed = false;
    // Sponsored cards passed without an interaction, for the operator lines.
    let mut ads_skipped = 0u32;
    // The last card whose images were paged sideways. A swallowed vertical swipe leaves it
    // on screen, and paging it again starts blind from its last image — the feed hides the
    // counter until a turn — so the first turn is the one that leaves the post for the
    // author's profile. Box 31–50 round 4, 10/10/2026 (`ce031713aadf361905`
    // 00:10:49–00:12:07) and round 3, 09/10/2026 (`ce0317139ca344520c` 21:39:02): the
    // second pass over a 14- and an 11-image post did exactly that.
    let mut last_paged_card: Option<PostFingerprint> = None;
    // The pages the carousel traversal reached on `last_paged_card`, for the evidence bundle.
    let mut last_paged_pages: Option<u32> = None;
    // Cards an unproven-swipe evidence bundle was attempted for; see
    // [`UNPROVEN_SWIPE_EVIDENCE_LIMIT`].
    let mut evidenced_cards: Vec<PostFingerprint> = Vec::new();
    let mut outcome = Outcome::Done;
    // Latched so an operator who switches comments on mid-run is told once why nothing
    // happens, instead of watching a switch that reports nothing at all.
    let mut said_comments_impossible = settings.comment_prob > 0 && !comment_capable;
    let mut said_live_settings_failed = false;

    // The feed loop proper. Set once outside the loop, for the same reason the pixel path
    // does it: a phase that flickered per post would be noise, not signal.
    if status.phase != NurturePhase::Watching {
        status.phase = NurturePhase::Watching;
        report(status, status.last_message.clone());
    }
    'feed: loop {
        if status.videos_done.saturating_sub(videos_at_start) >= total_videos {
            break;
        }
        if passes >= pass_budget {
            report(
                status,
                format!(
                    "đã dùng hết {pass_budget} lượt cho {total_videos} video — dừng ở {}/{total_videos}",
                    status.videos_done.saturating_sub(videos_at_start)
                ),
            );
            break;
        }
        passes += 1;
        // Live tuning, once per post — same place, same function, same reason as the pixel
        // loop.
        let live_refresh =
            apply_live_settings(live, &mut settings, &mut human, &mut policy, &mut moods);
        match &live_refresh {
            LiveSettingsRefresh::Applied => said_live_settings_failed = false,
            LiveSettingsRefresh::Failed(error) if !said_live_settings_failed => {
                said_live_settings_failed = true;
                report(
                    status,
                    format!(
                        "không đọc được cài đặt mới ({error}) — khóa like/comment/save/follow ở lượt này"
                    ),
                );
            }
            LiveSettingsRefresh::NoSource | LiveSettingsRefresh::Failed(_) => {}
        }
        if matches!(live_refresh, LiveSettingsRefresh::Applied)
            && settings.comment_prob > 0
            && !comment_capable
            && !said_comments_impossible
        {
            said_comments_impossible = true;
            report(
                status,
                "đã bật bình luận nhưng phiên này không đăng được: \
                 chưa nối nguồn chữ hoặc chưa đo nút Gửi. Cần dừng và chạy lại"
                    .into(),
            );
        }
        if stop.load(Ordering::Relaxed) {
            outcome = Outcome::Stopped;
            break;
        }
        if max_duration.is_some_and(|max| started.elapsed() >= max) {
            break;
        }
        if in_night_window(settings.night_start, settings.night_end) {
            report(status, "giờ nghỉ đêm — dừng".into());
            break;
        }
        policy.begin_post();
        let mut left_unproven = unproven_swipe_from.take();

        let (mood, mood_changed) = moods.next();
        if mood_changed {
            report(status, format!("chuyển nhịp: {}", mood.label()));
        }

        if !run.on_feed().await {
            off_feed_streak += 1;
            if off_feed_streak >= OFF_FEED_LIMIT {
                report(
                    status,
                    format!("kẹt ngoài FYP {off_feed_streak} lượt — dừng thay vì tap mù"),
                );
                outcome = Outcome::Partial;
                break;
            }
            // **Use the ladder that already knows how to find the feed.**
            //
            // This swiped, blindly, and a swipe is the one gesture that cannot help: being
            // off the feed means being on some *other* screen, and swiping a profile or a
            // search page moves that page. Measured 18/08/2026 on ce0717171c2a64d50d, which
            // left the feed after its first like and then reported "chưa thấy tab feed"
            // six times in a row while swiping a screen that was never going to become the
            // feed — a session that had already started successfully, ending at zero.
            //
            // `await_feed` is the same ladder the session opens with: tap the feed tab,
            // decline a modal, tap Home, and press Back when the screen carries no label at
            // all. Reusing it means there is one way back rather than two, and the second
            // one working was never more than luck.
            report(
                status,
                format!("rời khỏi feed ({off_feed_streak}/{OFF_FEED_LIMIT}) — tìm đường về"),
            );
            if !recover_feed(&run, stop, status, report).await {
                report(status, "không quay lại được feed — dừng".into());
                outcome = Outcome::Partial;
                break;
            }
            continue;
        }
        off_feed_streak = 0;

        // A LIVE card has no action rail at all. The pixel engine learned this by
        // spending videos tapping rail positions that did not exist.
        //
        // Two ways to notice, and the second is the one that carries the weight:
        // the LIVE label when the build has been measured for it, and otherwise the
        // rail simply not being there. The Vietnamese build has no measured LIVE
        // label — no LIVE post has come past to read it off — so absence of the
        // comment control is what identifies these cards there.
        let live_label = present(run.session, run.labels, TikTokControl::LiveRoom).await;
        let mut no_rail = !present(run.session, run.labels, TikTokControl::Comments).await;
        // **Right after an unproven swipe, a railless card is usually the incoming one still
        // drawing.** The feed tab is on screen, so the tree is not empty; only the rail is
        // missing. Swiping it past charged a second attempt for one card and left it unwatched
        // (see [`LATE_RAIL_WINDOW`]). Wait for its rail first, within that finite window.
        if no_rail && !live_label {
            // Taken, not borrowed: judged here either way, and the railed path below must
            // not judge it a second time against the same card. When no rail comes at all the
            // pass ends in the railless branch, where it was never read.
            if let Some(left) = left_unproven.take() {
                report(
                    status,
                    "vuốt trước chưa chứng minh được, thẻ hiện tại chưa có thanh hành động — chờ thanh hiện lại trước khi vuốt tiếp".into(),
                );
                let mut reads = CardReads::default();
                match await_late_rail(&run, stop, &mut reads).await {
                    Ok(ReadWaitResult::Ready(card)) => {
                        no_rail = false;
                        // Unproven is not advanced: the earlier swipe counts only now that a
                        // railed card reads differently from the one it left, by the full
                        // fingerprint and by the stable author/sound identity.
                        if card != left && !left.same_card_for_duplicate_guard(&card) {
                            stuck_swipes = 0;
                            status.videos_done += 1;
                            report(
                                status,
                                format!(
                                    "thẻ mới đã hiện thanh hành động — lượt vuốt trước đã đổi thẻ ({})",
                                    reads.says()
                                ),
                            );
                            if let Some(rest) = policy.rest_after_video() {
                                report(status, format!("nghỉ tự nhiên {}s", rest.as_secs()));
                                sleep_interruptible(rest, stop).await;
                            }
                            // A cap, not a target: the card this proved is not watched past it.
                            if status.videos_done.saturating_sub(videos_at_start) >= total_videos {
                                break;
                            }
                        } else {
                            report(
                                status,
                                format!(
                                    "thanh hành động đã hiện nhưng vẫn là thẻ cũ — lượt vuốt trước không được tính ({})",
                                    reads.says()
                                ),
                            );
                        }
                    }
                    Ok(ReadWaitResult::Cancelled) => {
                        outcome = Outcome::Stopped;
                        break;
                    }
                    Ok(ReadWaitResult::DeadlineExceeded) => {
                        report(
                            status,
                            format!(
                                "chờ {}s mà thẻ vẫn chưa có thanh hành động ({})",
                                LATE_RAIL_WINDOW.as_secs(),
                                reads.says()
                            ),
                        );
                    }
                    Err(error) => {
                        report(
                            status,
                            format!("không đọc được thanh hành động: {error} ({})", reads.says()),
                        );
                    }
                }
                if stop.load(Ordering::Relaxed) {
                    outcome = Outcome::Stopped;
                    break;
                }
            }
        }
        if live_label || no_rail {
            no_rail_streak += 1;
            // **A railless card must not consume the whole pass budget, and a proven feed must
            // not stop the session.** The screen is read once per railless pass (a measured
            // LIVE label is its own answer and skips the read):
            //
            // * a proved non-feed screen gets one Back from the second railless pass, and the
            //   feed is re-proved by a railed card;
            // * a proven feed — the For You tab or a selected Home tab, never the rail — with
            //   an ad end-card over the card is left by one straight flick, then by one Close
            //   and another flick; nothing on it is ever tapped but Close ([`super::ad_card`]);
            // * a proven feed with any other railless card (LIVE, still drawing) is swiped on,
            //   straight from the second pass, up to [`NO_RAIL_ON_FEED_LIMIT`];
            // * a screen that is none of those reaches [`NO_RAIL_STREAK_LIMIT`] and stops, as
            //   round 4 taught (see the constant). Round 5 (`ce031713b064841e0c`,
            //   01:16:37–01:16:55, 10/10/2026) then stopped there *on* the feed: the tabs were
            //   on screen on every pass — `on_feed` at the top of the pass had said so — and
            //   the old line claimed the feed was unproven because the rail was.
            let mut feed_proven = live_label;
            let mut end_card: Option<super::ad_card::AdEndCard> = None;
            // The card as the same snapshot read it: what the swipe below is judged against,
            // without a second read.
            let mut railless_card: Option<PostFingerprint> = None;
            if !live_label {
                match screen_tree(&run).await {
                    Some(tree) => match classify_tree(&tree, run.labels) {
                        FeedScreen::Feed => {
                            feed_proven = true;
                            end_card = super::ad_card::end_card(&tree, run.labels, run.screen);
                            railless_card = Some(tree_fingerprint(&tree, run.labels));
                        }
                        FeedScreen::NonFeed(why) if no_rail_streak >= 2 => {
                            report(
                                status,
                                format!("{no_rail_streak} thẻ liên tiếp không có thanh hành động — màn hình là {why}"),
                            );
                            if back_to_feed(&run, stop, status, report, why).await {
                                let mut reads = CardReads::default();
                                match await_late_rail(&run, stop, &mut reads).await {
                                    Ok(ReadWaitResult::Ready(_)) => {
                                        report(
                                            status,
                                            format!(
                                                "thẻ có thanh hành động đã hiện lại sau Back ({})",
                                                reads.says()
                                            ),
                                        );
                                        no_rail_streak = 0;
                                        ad_close_pressed = false;
                                        continue;
                                    }
                                    Ok(ReadWaitResult::Cancelled) => {
                                        outcome = Outcome::Stopped;
                                        break;
                                    }
                                    Ok(ReadWaitResult::DeadlineExceeded) => {}
                                    Err(error) => {
                                        report(
                                            status,
                                            format!(
                                                "không đọc được thanh hành động sau Back: {error}"
                                            ),
                                        );
                                    }
                                }
                            }
                        }
                        FeedScreen::NonFeed(_) | FeedScreen::OtherTab | FeedScreen::Unknown => {}
                    },
                    // No snapshot to read: the feed-tab query is the only proof left, and it is
                    // the one the top of this pass already passed.
                    None => feed_proven = run.on_feed().await,
                }
                if stop.load(Ordering::Relaxed) {
                    outcome = Outcome::Stopped;
                    break;
                }
            }
            if feed_proven && no_rail_streak >= NO_RAIL_ON_FEED_LIMIT {
                report(
                    status,
                    format!(
                        "{no_rail_streak} thẻ liên tiếp không có thanh hành động dù feed đã chứng minh (tab For You / Home){} — \
                         thẻ không đổi sau nhiều lượt vuốt; dừng thay vì vuốt hết lượt, cần kiểm tra tay",
                        if end_card.is_some() {
                            ", vẫn là thẻ kết thúc quảng cáo"
                        } else {
                            ""
                        }
                    ),
                );
                outcome = Outcome::Partial;
                break;
            }
            if let Some(ad) = end_card {
                ads_skipped += 1;
                report(
                    status,
                    format!(
                        "quảng cáo — bỏ qua: thẻ kết thúc quảng cáo (nút «{}»), không tương tác, \
                         không bấm nút quảng cáo; vuốt thẳng qua (đã bỏ qua {ads_skipped} quảng cáo)",
                        ad.cta
                    ),
                );
                // The rail is hidden under the mask, so `before` is empty and the swipe is
                // proven by the rail of whatever arrives. A skipped card, never a watched
                // video: `videos_done` does not move in this branch.
                let before = railless_card.take().unwrap_or_default();
                let (advanced, reads) = match leave_railless_card(
                    &mut run, &mut human, &before, true, stop, status, report,
                )
                .await
                {
                    Ok(judged) => judged,
                    Err(error) => {
                        report(status, format!("không đọc được kết quả vuốt: {error}"));
                        outcome = Outcome::Failed;
                        break;
                    }
                };
                if stop.load(Ordering::Relaxed) {
                    outcome = Outcome::Stopped;
                    break;
                }
                if advanced {
                    report(status, format!("đã rời thẻ quảng cáo ({})", reads.says()));
                    no_rail_streak = 0;
                    ad_close_pressed = false;
                    continue;
                }
                if ad_close_pressed {
                    report(
                        status,
                        format!(
                            "vuốt chưa chứng minh được rời quảng cáo ({}); Close đã dùng cho thẻ này — lượt sau vuốt tiếp",
                            reads.says()
                        ),
                    );
                    continue;
                }
                // One Close per card, and only on an end-card that is still there after the
                // swipe, read again: a stale match never authorises a tap. Close is the mask's
                // own `action_ad_hide_lynx_mask`: it hides the overlay and shows the card's rail
                // again, nothing else — and that rail belongs to the ad, so what follows is the
                // flick, never an interaction.
                let still_up = match screen_tree(&run).await {
                    Some(tree) => super::ad_card::end_card(&tree, run.labels, run.screen),
                    None => None,
                };
                match still_up {
                    Some(super::ad_card::AdEndCard {
                        close: Some(close), ..
                    }) => {
                        ad_close_pressed = true;
                        report(
                            status,
                            format!(
                                "vuốt chưa rời được quảng cáo ({}) — bấm Close một lần (chỉ đóng lớp phủ \
                                 quảng cáo, không mở trang quảng cáo) rồi vuốt tiếp",
                                reads.says()
                            ),
                        );
                        if let Err(error) = run.session.tap(close.centre()).await {
                            report(status, format!("bấm Close không được: {error}"));
                            continue;
                        }
                        let mut reads = CardReads::default();
                        match await_late_rail(&run, stop, &mut reads).await {
                            Ok(ReadWaitResult::Ready(card)) => {
                                report(
                                    status,
                                    format!(
                                        "thanh hành động hiện lại trên thẻ quảng cáo sau Close ({}) — không tương tác, vuốt tiếp",
                                        reads.says()
                                    ),
                                );
                                match leave_railless_card(
                                    &mut run, &mut human, &card, true, stop, status, report,
                                )
                                .await
                                {
                                    Ok((true, reads)) => {
                                        report(
                                            status,
                                            format!("đã rời thẻ quảng cáo sau Close ({})", reads.says()),
                                        );
                                        no_rail_streak = 0;
                                        ad_close_pressed = false;
                                    }
                                    Ok((false, reads)) => report(
                                        status,
                                        format!(
                                            "vuốt sau Close chưa chứng minh được đổi thẻ ({}) — lượt sau vuốt tiếp",
                                            reads.says()
                                        ),
                                    ),
                                    Err(error) => {
                                        report(status, format!("không đọc được kết quả vuốt: {error}"));
                                        outcome = Outcome::Failed;
                                        break;
                                    }
                                }
                            }
                            Ok(ReadWaitResult::Cancelled) => {
                                outcome = Outcome::Stopped;
                                break;
                            }
                            Ok(ReadWaitResult::DeadlineExceeded) => report(
                                status,
                                format!(
                                    "sau Close vẫn chưa thấy thanh hành động ({}) — lượt sau vuốt tiếp",
                                    reads.says()
                                ),
                            ),
                            Err(error) => report(
                                status,
                                format!("không đọc được thanh hành động sau Close: {error}"),
                            ),
                        }
                    }
                    Some(_) => report(
                        status,
                        format!(
                            "vuốt chưa rời được quảng cáo ({}); thấy nút Close nhưng cây không nhận nó là nút bấm được — không bấm, lượt sau vuốt tiếp",
                            reads.says()
                        ),
                    ),
                    None => report(
                        status,
                        format!(
                            "vuốt chưa chứng minh được đổi thẻ ({}) và không còn đọc thấy thẻ kết thúc quảng cáo — đọc lại ở lượt sau",
                            reads.says()
                        ),
                    ),
                }
                if stop.load(Ordering::Relaxed) {
                    outcome = Outcome::Stopped;
                    break;
                }
                continue;
            }
            if !feed_proven && no_rail_streak >= NO_RAIL_STREAK_LIMIT {
                report(
                    status,
                    format!(
                        "{no_rail_streak} thẻ liên tiếp không có thanh hành động và không chứng minh được feed — dừng thay vì vuốt hết lượt. \
                         Máy có thể đang ở LIVE, tab Khám phá hoặc một màn hình con của TikTok; cần kiểm tra tay"
                    ),
                );
                outcome = Outcome::Partial;
                break;
            }
            // From the second railless pass on a proven feed the bowed swipe has failed once
            // on this card; the straight flick is the gesture the overlays accept.
            let straight_flick = feed_proven && no_rail_streak >= 2;
            report(
                status,
                if live_label {
                    "thẻ LIVE — chỉ vuốt qua, không tương tác".into()
                } else if straight_flick {
                    format!(
                        "{no_rail_streak} thẻ liên tiếp không có thanh hành động nhưng feed đã chứng minh (tab For You / Home) — không dừng, vuốt thẳng tiếp"
                    )
                } else {
                    "thẻ không có thanh hành động (LIVE / đang chuyển) — chỉ vuốt tiếp".to_string()
                },
            );
            let before = match railless_card.take() {
                Some(card) => card,
                None => fingerprint(run.session, run.labels).await,
            };
            let (advanced, _) = match leave_railless_card(
                &mut run,
                &mut human,
                &before,
                straight_flick,
                stop,
                status,
                report,
            )
            .await
            {
                Ok(judged) => judged,
                Err(error) => {
                    report(status, format!("không đọc được kết quả vuốt: {error}"));
                    outcome = Outcome::Failed;
                    break;
                }
            };
            if stop.load(Ordering::Relaxed) {
                outcome = Outcome::Stopped;
                break;
            }
            if advanced {
                status.videos_done += 1;
            }
            continue;
        }
        no_rail_streak = 0;
        ad_close_pressed = false;

        let watch = human.watch_seconds(settings.watch_min, settings.watch_max) * mood.watch_mult();
        report(status, format!("xem {watch:.1}s ({})", mood.label()));
        let watched = watch_card(
            run.session,
            run.labels,
            Duration::from_secs_f64(watch.max(0.5)),
            stop,
            status,
            report,
        )
        .await;
        if stop.load(Ordering::Relaxed) || watched == WatchEnd::Stopped {
            outcome = Outcome::Stopped;
            break;
        }
        match watched {
            WatchEnd::Watched | WatchEnd::Stopped => {}
            WatchEnd::CardChanged => {
                report(
                    status,
                    "thẻ đã đổi trong lúc xem — không tương tác với thẻ chưa xem, xem lại từ đầu"
                        .into(),
                );
                continue;
            }
            WatchEnd::Unproven => {
                report(
                    status,
                    "vừa đóng hộp thoại và chưa đọc lại được thẻ — không tương tác, xem lại từ đầu"
                        .into(),
                );
                continue;
            }
            WatchEnd::Blocked(says) => {
                report(status, says);
                outcome = Outcome::Partial;
                break;
            }
        }

        let (before, surface) = fingerprint_and_surface(run.session, run.labels).await;
        // **A swipe proven one pass late, by a railed card.** The late-rail wait above counts
        // the previous swipe only when the next card arrived *without* a rail. When it
        // arrived with one, the swipe that produced it was dropped unjudged: the card the
        // session had just watched to the end of its dwell was never counted, and
        // `stuck_swipes` kept the strike. The card watched here is the proof — a railed card
        // that reads as another post than the one the swipe left.
        if let Some(left) = left_unproven.take() {
            if !before.is_empty() && before != left && !left.same_card_for_duplicate_guard(&before)
            {
                stuck_swipes = 0;
                status.videos_done += 1;
                report(
                    status,
                    "thẻ vừa xem khác thẻ trước — lượt vuốt trước đã đổi thẻ, tính một video"
                        .into(),
                );
                // A cap, not a target.
                if status.videos_done.saturating_sub(videos_at_start) >= total_videos {
                    break;
                }
            }
        }
        human.note_action();
        // Hoisted above the roll because the Comment arm needs it to decide whether to wait.
        // Safe to read here: `apply_live_settings` runs only at the top of the loop, so this
        // is the same value the carousel block below would have read.
        let ceiling = settings.carousel_ceiling();
        // Set when the comment was charged for but held until the slides have been paged.
        let mut deferred_comment: Option<AttemptReservation> = None;

        // Whether this post's comment went out on the *immediate* path. The deferred path
        // refreshes `before` itself; this one could not, because `before` is not mutable until
        // the shadow below.
        let mut comment_sent = false;

        // **Do not interact with a card we already interacted with.** A swallowed swipe
        // leaves the acted card on screen, and this pass would otherwise `begin_post()` and
        // spend a fresh like/comment/follow on it — a duplicate public action on a real
        // account, undoable by nobody. The fingerprint match is deliberately conservative:
        // on the rare chance two different cards read identically it costs one skipped
        // interaction, never a second comment. `is_empty()` cards (no rail) are excluded so
        // a LIVE/transition card never latches this.
        let still_on_acted_card = last_acted_fingerprint
            .as_ref()
            .is_some_and(|acted| acted.same_card_for_duplicate_guard(&before));
        if still_on_acted_card {
            report(
                status,
                "vẫn ở thẻ vừa tương tác (vuốt trước chưa ăn) — KHÔNG tương tác lại, chỉ tìm cách rời đi"
                    .into(),
            );
        }
        // Set by any arm that actually spends an interaction on this card, so the card's
        // post-action fingerprint can be remembered before the swipe.
        let mut acted_this_pass = false;

        // **Never interact with a sponsored card.** No like, save, comment or follow, and the
        // CTA is never a target of anything here. Read from the same snapshot as `before`
        // ([`CardSurface`]): the CTA strip `ad_feed_card_container_v2` or the `Sponsored` label
        // (see [`super::ad_card::sponsored_card`]). A snapshot that failed to read is unknown,
        // not ordinary, so that pass watches and leaves; a backend without snapshots cannot
        // tell and keeps its rolls, which is what it did before.
        // The ad's own touch targets, kept for the swipe that leaves it; `Some` marks the card
        // as a skipped ad for the carousel, the gesture and the count below.
        let mut sponsored: Option<Vec<ElementBox>> = None;
        let plan = match surface {
            CardSurface::Sponsored(card) => {
                ads_skipped += 1;
                report(
                    status,
                    format!(
                        "quảng cáo — bỏ qua ({}): không tim/lưu/bình luận/follow, không vuốt ngang, \
                         không bấm nút quảng cáo; vuốt thẳng qua (đã bỏ qua {ads_skipped} quảng cáo)",
                        card.evidence
                    ),
                );
                sponsored = Some(card.touch_owners);
                crate::human_behavior::FeedActionPlan::default()
            }
            CardSurface::Unread if !still_on_acted_card => {
                report(
                    status,
                    "không đọc được cây màn hình để loại quảng cáo — không tương tác với thẻ này"
                        .into(),
                );
                crate::human_behavior::FeedActionPlan::default()
            }
            CardSurface::Ordinary | CardSurface::Unsupported | CardSurface::Unread
                if still_on_acted_card =>
            {
                crate::human_behavior::FeedActionPlan::default()
            }
            CardSurface::Ordinary | CardSurface::Unsupported | CardSurface::Unread => {
                roll_nurture_actions(&settings, mood)
            }
        };
        for selected in
            crate::human_behavior::ordered_feed_actions(&plan, &settings.workflow_action_order)
        {
            match selected {
                FeedAction::Like
                    if !policy.can_interact_with_post()
                        || !policy.can_attempt(PolicyAction::Like) =>
                {
                    report(status, "bỏ qua tim: nhịp phiên hiện tại đã đủ".into());
                }
                FeedAction::Like => {
                    if !wait_gap(&mut last_interaction_at, policy.min_action_gap(), stop).await {
                        outcome = Outcome::Stopped;
                        break 'feed;
                    }
                    let reservation = policy.reserve_attempt(PolicyAction::Like);
                    report(status, "thả tim".into());
                    match run.like(stop).await {
                        Ok(LikeVerdict::Liked) => {
                            policy.commit_attempt(reservation);
                            acted_this_pass = true;
                            status.like_attempts += 1;
                            status.likes += 1;
                            report(status, "tim thành công (nhãn đổi trạng thái)".into());
                        }
                        Ok(LikeVerdict::AlreadyLiked) => {
                            policy.cancel_no_effect(reservation);
                            report(status, "video đã tim từ trước — bỏ qua".into())
                        }
                        Ok(LikeVerdict::NoControl) => {
                            policy.cancel_no_effect(reservation);
                            report(status, "bỏ qua tim: thẻ này không có nút tim".into())
                        }
                        Ok(LikeVerdict::NotConfirmed) => {
                            policy.commit_attempt(reservation);
                            acted_this_pass = true;
                            status.like_attempts += 1;
                            report(
                                status,
                                "tim: tap gửi được nhưng nhãn không đổi — không tính là đã tim"
                                    .into(),
                            )
                        }
                        Ok(LikeVerdict::StateUnreadable) => {
                            policy.cancel_no_effect(reservation);
                            report(
                            status,
                            "bỏ qua tim: không đọc được trạng thái đã-tim (không tap để khỏi gỡ tim)"
                                .into(),
                        )
                        }
                        Err(error) => {
                            acted_this_pass |= settle_action_failure(
                                &mut policy,
                                reservation,
                                &error,
                                &mut status.like_attempts,
                            );
                            report(status, format!("tim thất bại: {error}"));
                        }
                    }
                }
                FeedAction::Save
                    if !policy.can_interact_with_post()
                        || !policy.can_attempt(PolicyAction::Save) =>
                {
                    report(status, "bỏ qua lưu: nhịp phiên hiện tại đã đủ".into());
                }
                FeedAction::Save => {
                    if !wait_gap(&mut last_interaction_at, policy.min_action_gap(), stop).await {
                        outcome = Outcome::Stopped;
                        break 'feed;
                    }
                    let reservation = policy.reserve_attempt(PolicyAction::Save);
                    report(status, "lưu video".into());
                    let card_key = format!("card-{}", status.videos_done);
                    let (evidence, lease) = run.save(save_journal, &card_key, stop).await;
                    match super::settle_journaled_save(
                        &mut policy,
                        reservation,
                        status,
                        save_journal,
                        lease,
                        &evidence,
                    ) {
                        Ok(acted) => acted_this_pass |= acted,
                        Err(error) => {
                            report(
                                status,
                                format!("lưu: không chốt được sổ hành động ({error})"),
                            );
                            outcome = Outcome::Failed;
                            break 'feed;
                        }
                    }
                    report(status, super::save_verdict_message(&evidence));
                }
                FeedAction::Comment
                    if !comment_capable
                        || !policy.can_interact_with_post()
                        || !policy.can_attempt(PolicyAction::Comment) =>
                {
                    // The startup line already said why comments are off for the whole
                    // session; per-post silence there is deliberate.
                    if comment_capable {
                        report(status, "bỏ qua bình luận: nhịp phiên hiện tại đã đủ".into());
                    }
                }
                FeedAction::Comment => {
                    if !wait_gap(&mut last_interaction_at, policy.min_action_gap(), stop).await {
                        outcome = Outcome::Stopped;
                        break 'feed;
                    }
                    let reservation = policy.reserve_attempt(PolicyAction::Comment);
                    report(status, "bình luận".into());
                    let source = comments.expect("comment_capable implies a text source");
                    // **Wait for the slides on a post that has them.** The comment used to be
                    // written right here, before a single sideways gesture existed, so a
                    // six-image post was commented on from image one — and because a still card
                    // publishes the same picture on every sample, from one picture sampled three
                    // times. The traversal below already pays for every flick, its settle and a
                    // hierarchy dump per slide; the comment simply never saw any of it.
                    //
                    // A video keeps this exact order. There is nothing to wait for, and the
                    // follow block below can navigate off the feed — which a deferred comment has
                    // to survive, and an immediate one never meets.
                    if ceiling > 0
                        && can_page_carousel(run.labels)
                        && run.looks_like_photo_post().await
                    {
                        report(
                            status,
                            "bài ảnh — soạn bình luận sau khi xem hết ảnh".into(),
                        );
                        deferred_comment = Some(reservation);
                    } else {
                        let comment = post_rolled_comment(
                            &mut run, source, &settings, &before, stop, status, report,
                        )
                        .await;
                        comment_sent = comment.sent;
                        acted_this_pass |= settle_attempt(
                            &mut policy,
                            reservation,
                            comment.did_act,
                            &mut status.comment_attempts,
                        );
                    }
                }
                FeedAction::None => {}
            }
        }

        // Photo posts, before leaving the card. After the interactions on purpose: a like
        // belongs to the post, not to an image, and the rail is where it was when the card
        // arrived. `carousel_ceiling()` returns 0 when the operator has the feature switched
        // off, and then not even the cheap gate query runs.
        //
        // The **ceiling**, not `carousel_slide_budget()`: that one has the portion already
        // folded in for the pixel engine, and this path applies the portion to the post's
        // real image count. Using it here would apply the percentage twice.
        let mut before = before;
        if comment_sent {
            // **A sent comment changes this card's own comment count, and that count is part
            // of the fingerprint the next vertical swipe is judged against.** The deferred
            // path already refreshes for exactly this reason (see below); the immediate path
            // discarded `post_rolled_comment`'s answer and never did.
            //
            // Left stale, the sequence is: card reads Comments=12 into `before`, the comment
            // posts and the card now reads 13, the vertical swipe is swallowed by the UI, and
            // `swipe_next` compares 13 against 12 and reports **advanced**. So `videos_done`
            // counts a post the session never left — and worse, `stuck_swipes` resets to 0, so
            // the stuck-swipe recovery that exists to catch exactly this never fires. The loop
            // can keep watching and acting on the same post while reporting progress.
            //
            // The comment forty lines up says the inverse of this: "judging a swipe against a
            // stale card is how a working swipe reads as stuck". This is the other direction —
            // a stuck swipe reading as working — and it is the more expensive one.
            //
            // Found by an independent review on 28/08/2026.
            before = fingerprint(run.session, run.labels).await;
        }
        // Whether this pass paged a photo post, which decides the gesture that leaves it.
        let mut paged_this_pass = false;
        // Never on an ad: its photos are the advertiser's Lynx body, and paging it is touching it.
        if ceiling > 0 && sponsored.is_none() {
            // Only a deferred comment has a use for the slides. Handing the sink over on every
            // photo post would fill it on posts nobody is going to comment on, and the next
            // comment would then be grounded on the wrong card.
            let evidence = deferred_comment.is_some().then_some(comments).flatten();
            let already_paged = last_paged_card
                .as_ref()
                .is_some_and(|paged| paged.same_card_for_duplicate_guard(&before));
            let traversed = !(already_paged || still_on_acted_card);
            let slides = if !traversed {
                // The card never left (see `last_paged_card`). Its images were paged on an
                // earlier pass, and paging again would start blind from its last image.
                if already_paged {
                    report(
                        status,
                        "vẫn ở bài ảnh vừa vuốt ngang — không vuốt ngang lại".into(),
                    );
                }
                // Slide one is still one slide of evidence for a comment deferred on it.
                if let Some(evidence) = evidence {
                    evidence.note_slide();
                }
                1
            } else {
                run.traverse_carousel(
                    settings.carousel_portion_percent,
                    ceiling,
                    stop,
                    status,
                    report,
                    evidence,
                )
                .await
            };
            paged_this_pass = slides > 1;
            // **The deferred comment, before the stop check below.** A session told to stop
            // mid-traversal has already spent this post's comment budget, so the row has to
            // be written either way — that is the same hole `no_api_key` used to leave, and
            // the reason `record_skip` exists.
            if let Some(reservation) = deferred_comment.take() {
                let source = comments.expect("deferred implies a text source");
                // Let the pager finish, then re-read the card. `before` was taken at the top
                // of the post, several sideways gestures ago.
                sleep_interruptible(SWIPE_SETTLE, stop).await;
                let after = fingerprint(run.session, run.labels).await;
                if stop.load(Ordering::Relaxed) {
                    policy.cancel_no_effect(reservation);
                    source.record_skip(&settings, "deferred_stopped").await;
                } else if after.is_empty() {
                    policy.cancel_no_effect(reservation);
                    // No rail at all means the card is gone or mid-transition. Either way
                    // the drawer would open on the wrong screen.
                    report(status, "bỏ qua bình luận: không còn thấy thẻ bài".into());
                    source.record_skip(&settings, "deferred_no_rail").await;
                } else if after != before {
                    policy.cancel_no_effect(reservation);
                    // The card changed underneath the traversal. Posting now would comment on
                    // whatever arrived, which is worse than not commenting.
                    report(
                        status,
                        "bỏ qua bình luận: thẻ đã đổi khi đang xem ảnh".into(),
                    );
                    source.record_skip(&settings, "deferred_card_changed").await;
                } else {
                    let comment = post_rolled_comment(
                        &mut run, source, &settings, &after, stop, status, report,
                    )
                    .await;
                    acted_this_pass |= settle_attempt(
                        &mut policy,
                        reservation,
                        comment.did_act,
                        &mut status.comment_attempts,
                    );
                    if comment.sent {
                        // A sent comment changes this card's own comment count, and that count
                        // is part of the next vertical swipe proof.
                        before = fingerprint(run.session, run.labels).await;
                    } else {
                        before = after;
                    }
                }
            }
            if stop.load(Ordering::Relaxed) {
                outcome = Outcome::Stopped;
                break 'feed;
            }
            if slides > 1 && deferred_comment.is_none() {
                // Re-read the card we are about to leave, and let the pager finish.
                //
                // `before` was taken at the top of the post, several sideways gestures ago,
                // and it is the evidence the *next* vertical swipe is judged against. Using
                // the stale one made every swipe after a carousel read as unproved:
                // measured on 12/08/2026, a run that paged four photo posts reported
                // `vuốt chưa chứng minh được đổi thẻ` four times and stopped at
                // `1/5 video` — the feature reporting the feed as stuck was the carousel
                // traversal it had just finished.
                sleep_interruptible(SWIPE_SETTLE, stop).await;
                before = fingerprint(run.session, run.labels).await;
            }
            if slides >= 1 && !before.is_empty() {
                last_paged_card = Some(before.clone());
                if traversed {
                    last_paged_pages = Some(slides);
                }
            }
        }

        // Follow is last in the public-action order. On photo cards the selected comment is
        // deliberately delayed until carousel evidence is collected, so this block belongs
        // after that delay rather than beside the initial rolls.
        if !still_on_acted_card
            && can_follow(run.labels)
            && plan.follow
            && policy.can_interact_with_post()
            && policy.can_attempt(PolicyAction::Follow)
            && wait_gap(&mut last_interaction_at, policy.min_action_gap(), stop).await
        {
            if !run.on_feed().await {
                report(
                    status,
                    "bỏ qua follow: không còn ở feed (hành động trước đã rời thẻ) — không follow nhầm tác giả"
                        .into(),
                );
            } else {
                let reservation = policy.reserve_attempt(PolicyAction::Follow);
                report(status, "follow tác giả".into());
                match run.follow(&before, stop, follow_journal).await {
                    Ok(verdict) => {
                        verdict.latch_session_outcome(&mut outcome);
                        if verdict.did_act() {
                            policy.commit_attempt(reservation);
                            acted_this_pass = true;
                            status.follow_attempts += 1;
                        } else {
                            policy.cancel_no_effect(reservation);
                        }
                        match verdict {
                            FollowVerdict::Followed => {
                                status.follows += 1;
                                report(
                                    status,
                                    "follow thành công (nút Follow mất khỏi thẻ)".into(),
                                );
                            }
                            FollowVerdict::NoControl => {
                                report(status, "bỏ qua follow: thẻ không có nút Follow".into())
                            }
                            FollowVerdict::SourceUnavailable => report(
                                status,
                                "bỏ qua follow: chưa chứng minh được hồ sơ tác giả để hoàn tác"
                                    .into(),
                            ),
                            FollowVerdict::CardChanged => report(
                                status,
                                "bỏ qua follow: thẻ hoặc tác giả đã đổi trước cú tap".into(),
                            ),
                            FollowVerdict::NotConfirmed => report(
                                status,
                                "follow đã chạm nhưng readback không chắc chắn".into(),
                            ),
                            FollowVerdict::Cancelled => {
                                report(status, "đã dừng trước cú tap Follow".into());
                                outcome = Outcome::Stopped;
                                break 'feed;
                            }
                        }
                    }
                    Err(error) => {
                        let possible_effect = error.effect_may_have_gone_out();
                        acted_this_pass |= settle_action_failure(
                            &mut policy,
                            reservation,
                            &error,
                            &mut status.follow_attempts,
                        );
                        if possible_effect {
                            outcome = Outcome::Partial;
                        }
                        report(status, format!("follow thất bại: {error}"));
                    }
                }
            }
        }

        // **An interaction can leave the feed, and a swipe spent elsewhere is a post lost.**
        //
        // Measured 18/08/2026 on ce0717171c2a64d50d: every post went watch → like → the
        // like landed somewhere that opened another screen → swipe, on that screen → the
        // card could not have changed, so the swipe read as unproven and the post counted
        // for nothing. The walk back happened on the *next* pass, by which time the same
        // thing was about to happen again, and the session ended at zero videos having
        // recovered its way through every one of them.
        //
        // Checking here costs one cheap read per post and turns that into a post that
        // counts. The fingerprint is retaken because the walk back is a navigation: the
        // card in front of us afterwards is not necessarily the one `before` describes,
        // and judging a swipe against a stale card is how a working swipe reads as stuck.
        // Remember the card we just acted on, while still standing on it — before any
        // walk-back navigation re-reads `before`. If the swipe below is swallowed, the next
        // pass will read this same fingerprint and decline to act on it a second time.
        if acted_this_pass {
            last_acted_fingerprint = Some(before.clone());
        }

        // After a carousel this is also where an overshoot is caught: the screen is
        // classified, a proved profile page or photo viewer gets one Back, and the feed is
        // re-proved before the swipe. See [`recover_feed`].
        if !run.on_feed().await {
            if !recover_feed(&run, stop, status, report).await {
                report(status, "không quay lại được feed — dừng".into());
                outcome = Outcome::Partial;
                break;
            }
            before = fingerprint(run.session, run.labels).await;
        }

        // A photo post is left from a start clear of its page-indicator band: see
        // [`PHOTO_INDICATOR_ID`]. Every pass on the card, not only the one that paged it — the
        // bowed swipe of a later pass starts on the same band.
        let photo_card = sponsored.is_none()
            && last_paged_card
                .as_ref()
                .is_some_and(|paged| paged.same_card_for_duplicate_guard(&before));
        let photo_from = if photo_card {
            let (from, line) = run.photo_swipe_from(stop).await;
            if let Some(line) = line {
                report(status, line);
            }
            from
        } else {
            None
        };
        status.swipe_attempts += 1;
        // **The two `HumanBehavior` pauses this module's own header promises are shared.**
        //
        // It claimed `HumanBehavior` was taken whole from the pixel engine so the two backends
        // could not "drift into behaving like two different users" — and then never called
        // `think_pause_ms` before the swipe or `after_swipe_pause_ms` after it, both of which the
        // pixel engine does call. Android sessions went straight from the last action into the
        // swipe and straight out of the swipe into the next watch.
        //
        // The after-swipe one matters beyond its own duration: it takes `&mut self` and increments
        // `consecutive_swipes`, which is what arms the persona's long rest after
        // `pause_after_swipes` swipes. Never calling it left that setting **inert on Android** —
        // switched on in the UI, doing nothing on the twenty phones that actually run.
        sleep_interruptible(Duration::from_millis(human.think_pause_ms()), stop).await;
        // An ad leaves by the flick, like the end-card, from a clear start: see `ad_swipe_from`.
        let ad_from = sponsored
            .as_deref()
            .map(|owners| ad_swipe_from(&run, owners, status, report));
        let (advanced, reads) = match run
            .swipe_next(
                human.swipe_duration_ms(roll_bool(settings.frenzy_prob)),
                &before,
                paged_this_pass || sponsored.is_some(),
                ad_from.or(photo_from),
                stop,
            )
            .await
        {
            Ok(judged) => judged,
            Err(error) => {
                report(status, format!("không đọc được kết quả vuốt: {error}"));
                outcome = Outcome::Failed;
                break;
            }
        };
        if stop.load(Ordering::Relaxed) {
            outcome = Outcome::Stopped;
            break;
        }
        sleep_interruptible(Duration::from_millis(human.after_swipe_pause_ms()), stop).await;
        if let Some(line) = reads.cancelled_says() {
            report(status, line);
        }
        if advanced && sponsored.is_some() {
            // A skipped card, never a watched video: the count is the operator's videos.
            stuck_swipes = 0;
            report(status, format!("đã rời thẻ quảng cáo ({})", reads.says()));
        } else if advanced {
            stuck_swipes = 0;
            // Keep the duplicate guard until the next pass reads a different stable identity.
            // Full fingerprint inequality proves enough to count a swipe, but our own delayed
            // comment-count update can also create that inequality on a swallowed swipe.
            // Clearing here would turn that useful full proof into permission to comment twice.
            status.videos_done += 1;
            if let Some(rest) = policy.rest_after_video() {
                report(status, format!("nghỉ tự nhiên {}s", rest.as_secs()));
                sleep_interruptible(rest, stop).await;
            }
        } else {
            stuck_swipes += 1;
            if !before.is_empty() {
                unproven_swipe_from = Some(before.clone());
            }
            report(
                status,
                format!(
                    "vuốt chưa chứng minh được đổi thẻ ({stuck_swipes}/{STUCK_SWIPE_LIMIT}; {})",
                    reads.says()
                ),
            );
            // The second unproven swipe in a row: keep one bundle of what the phone showed,
            // once per card and at most [`UNPROVEN_SWIPE_EVIDENCE_LIMIT`] per session. Before
            // the restart below, which would replace the screen it is about.
            if stuck_swipes >= 2
                && !before.is_empty()
                && evidenced_cards.len() < UNPROVEN_SWIPE_EVIDENCE_LIMIT
                && !evidenced_cards
                    .iter()
                    .any(|card| card.same_card_for_duplicate_guard(&before))
            {
                evidenced_cards.push(before.clone());
                let carousel_pages = if photo_card { last_paged_pages } else { None };
                match capture_unproven_swipe(&run, &before, &reads, carousel_pages, stop).await {
                    Some(folder) => report(
                        status,
                        format!("đã lưu bằng chứng vuốt chưa chứng minh: popups/{folder}"),
                    ),
                    None => report(
                        status,
                        "không lưu được bằng chứng vuốt chưa chứng minh (không đọc được màn hình hoặc đã đủ giới hạn)"
                            .into(),
                    ),
                }
            }
            if stuck_swipes >= STUCK_RESTART_AFTER && !restarted && run.search_keyword.is_none() {
                // **Restart TikTok once before believing the feed is over.**
                //
                // A feed that will not advance is not a broken swipe. Measured 18/08/2026
                // on ce051715081fe20f03, whose card would not change for the nurture loop,
                // for the agent, or for a plain `adb shell input swipe` — and which moved
                // on the very first swipe after the app was force-stopped and reopened.
                // Launching it again is not enough: `launch_app_foreground` only raises a
                // process that is already in front, which is why repeated launches kept
                // showing the same card and made this look like a dead phone.
                //
                // Once, and only after four honest attempts: restarting costs the watch
                // history of the current session and a few seconds of startup, and a feed
                // that is still stuck afterwards is telling us something a second restart
                // will not change.
                restarted = true;
                // The card after a restart is no evidence about the swipe that preceded it.
                unproven_swipe_from = None;
                report(status, "feed không đổi thẻ — khởi động lại TikTok".into());
                let _ = run.session.restart_app(bundle_id).await;
                if await_feed(&run, stop, status, report).await {
                    // No need to re-fingerprint: the loop reads the card it is about to
                    // leave at the top of each pass, and after a restart that is a
                    // different card anyway.
                    //
                    // `last_acted_fingerprint` is deliberately NOT cleared here: a restart is
                    // only *supposed* to change the card, and when it does not (the card is
                    // genuinely stuck) clearing it would let the next pass act on the same
                    // post again. The next pass re-reads the card and the fingerprint guard
                    // decides — a real new card differs and is acted on, a stuck one matches
                    // and is left alone. Only a proven swipe clears it.
                    stuck_swipes = 0;
                    continue;
                }
            }
            if stuck_swipes >= STUCK_SWIPE_LIMIT {
                report(
                    status,
                    "feed không đổi thẻ sau nhiều lượt vuốt — dừng".into(),
                );
                outcome = Outcome::Partial;
                break;
            }
        }
    }

    outcome
}

/// The start of the vertical swipe off a railed sponsored card, cleared of the ad's own touch
/// targets ([`super::ad_card::swipe_start`]) inside the feed's swipe zone, said in the log.
///
/// Box 31–50, 10/10/2026, `ce0417141d4decde0c` 08:57:15–08:58:21 (Galaxy S8+, 45.7.3): four
/// swipes off one Shopee photo ad were all unproven and the session stopped at 9/15; the card
/// on screen at the stop was the same ad, already liked (`Video liked`), so the gesture never
/// reached the feed pager. All four were the bowed `plan_swipe` from (0.5, 0.72) of the
/// screen: y=1508 on a 2094 px window, 56 px above the `Get` CTA strip [32,1564][870,1701]
/// and inside the bow's reach of it, or y=1598 on the 2220 px display, on the strip itself
/// (the log does not record which). The whole card also sits under the ad's Lynx
/// `fullscreen_container` — the same surface family as the end-card mask, which forwards a
/// flick and not a drag (see [`HierarchyRun::swipe_next`]). So an ad is left by the flick, from
/// the clear point nearest the usual start, kept between 50 % and 80 % of the screen: below
/// the swipe's end (28 %) by a full flick's travel and above the bottom bar (88.5 % on that
/// phone). **Not yet proved on a phone.**
fn ad_swipe_from(
    run: &HierarchyRun<'_>,
    owners: &[ElementBox],
    status: &mut NurtureSessionStatus,
    report: &(dyn Fn(&mut NurtureSessionStatus, String) + Send + Sync),
) -> TapPoint {
    let target = run.feed_swipe_from();
    let zone = (run.screen.1 * 0.50, run.screen.1 * 0.80);
    match super::ad_card::swipe_start(owners, &target, zone) {
        Some(from) => {
            report(
                status,
                format!(
                    "quảng cáo: vuốt thẳng từ y={:.0} (tránh {} vùng bấm của quảng cáo; mặc định y={:.0})",
                    from.y,
                    owners.len(),
                    target.y
                ),
            );
            from
        }
        None => {
            report(
                status,
                format!(
                    "quảng cáo: không có điểm vuốt trống trong vùng feed — vuốt thẳng từ y={:.0} như mặc định",
                    target.y
                ),
            );
            target
        }
    }
}

/// Hold off until the configured gap since the last interaction has passed.
///
/// Returns false when the stop flag fired while waiting, so the caller ends the
/// session instead of acting after an interrupted pause.
async fn wait_gap(last: &mut Option<Instant>, gap: Duration, stop: &AtomicBool) -> bool {
    if let Some(previous) = *last {
        let elapsed = previous.elapsed();
        if elapsed < gap {
            sleep_interruptible(gap - elapsed, stop).await;
        }
    }
    if stop.load(Ordering::Relaxed) {
        return false;
    }
    *last = Some(Instant::now());
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tiktok_labels::controls_for;

    fn vietnamese() -> TikTokControls {
        controls_for("com.ss.android.ugc.trill", "vi", "46.3.3").expect("measured set")
    }

    fn english_38() -> TikTokControls {
        controls_for("com.ss.android.ugc.trill", "en", "38.3.2").expect("measured set")
    }

    fn texts(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| (*s).to_string()).collect()
    }

    struct FollowCardPhone {
        author: &'static str,
        follow_author: &'static str,
        taps: std::sync::atomic::AtomicUsize,
        followed: std::sync::atomic::AtomicBool,
    }

    #[async_trait::async_trait]
    impl UiSession for FollowCardPhone {
        async fn tap(&self, _point: TapPoint) -> anyhow::Result<()> {
            self.taps.fetch_add(1, Ordering::Relaxed);
            self.followed.store(true, Ordering::Relaxed);
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
            let ElementQuery::Description { value, .. } = query else {
                return Ok(None);
            };
            let card = |description: String| {
                Some(ElementBox {
                    x: 900.0,
                    y: 1_000.0,
                    width: 80.0,
                    height: 80.0,
                    description: Some(description),
                    enabled: true,
                    clickable: true,
                })
            };
            Ok(match value {
                "Đề xuất" => card("Đề xuất".into()),
                "Hồ sơ " => card(format!("Hồ sơ {}", self.author)),
                "bình luận" => card("12 bình luận".into()),
                "Chia sẻ video" => card("Chia sẻ video. 3".into()),
                "Follow " if !self.followed.load(Ordering::Relaxed) => {
                    card(format!("Follow {}", self.follow_author))
                }
                _ => None,
            })
        }
    }

    #[tokio::test]
    async fn hierarchy_follow_refuses_author_b_when_roll_was_for_author_a() {
        let phone = FollowCardPhone {
            author: "author_b",
            follow_author: "author_b",
            taps: std::sync::atomic::AtomicUsize::new(0),
            followed: std::sync::atomic::AtomicBool::new(false),
        };
        let screen = (1_080.0, 2_220.0);
        let mut run = HierarchyRun {
            session: &phone,
            labels: vietnamese(),
            screen,
            planner: TouchPointPlanner::new(screen),
            search_keyword: None,
        };
        let expected = PostFingerprint {
            author: Some("Hồ sơ author_a".into()),
            comments: Some("12 bình luận".into()),
            share: Some("Chia sẻ video. 3".into()),
            sound: None,
        };

        assert_eq!(
            run.follow(&expected, &AtomicBool::new(false), None)
                .await
                .expect("classified follow"),
            FollowVerdict::CardChanged
        );
        assert_eq!(phone.taps.load(Ordering::Relaxed), 0);
    }

    #[tokio::test]
    async fn hierarchy_follow_refuses_a_follow_control_for_another_author() {
        let phone = FollowCardPhone {
            author: "author_a",
            follow_author: "author_b",
            taps: std::sync::atomic::AtomicUsize::new(0),
            followed: std::sync::atomic::AtomicBool::new(false),
        };
        let screen = (1_080.0, 2_220.0);
        let mut run = HierarchyRun {
            session: &phone,
            labels: vietnamese(),
            screen,
            planner: TouchPointPlanner::new(screen),
            search_keyword: None,
        };
        let expected = PostFingerprint {
            author: Some("Hồ sơ author_a".into()),
            comments: Some("12 bình luận".into()),
            share: Some("Chia sẻ video. 3".into()),
            sound: None,
        };

        assert_eq!(
            run.follow(&expected, &AtomicBool::new(false), None)
                .await
                .expect("classified follow"),
            FollowVerdict::CardChanged
        );
        assert_eq!(phone.taps.load(Ordering::Relaxed), 0);
    }

    #[tokio::test]
    async fn hierarchy_follow_without_a_durable_source_journal_never_taps() {
        let phone = FollowCardPhone {
            author: "author_a",
            follow_author: "author_a",
            taps: std::sync::atomic::AtomicUsize::new(0),
            followed: std::sync::atomic::AtomicBool::new(false),
        };
        let screen = (1_080.0, 2_220.0);
        let mut run = HierarchyRun {
            session: &phone,
            labels: vietnamese(),
            screen,
            planner: TouchPointPlanner::new(screen),
            search_keyword: None,
        };
        let expected = PostFingerprint {
            author: Some("Hồ sơ author_a".into()),
            comments: Some("12 bình luận".into()),
            share: Some("Chia sẻ video. 3".into()),
            sound: None,
        };

        assert_eq!(
            run.follow(&expected, &AtomicBool::new(false), None)
                .await
                .expect("follow"),
            FollowVerdict::SourceUnavailable
        );
        assert_eq!(phone.taps.load(Ordering::Relaxed), 0);
    }

    struct ProvedFollowPhone {
        handle: &'static str,
        confirm: bool,
        taps: std::sync::atomic::AtomicUsize,
        followed: std::sync::atomic::AtomicBool,
    }

    impl ProvedFollowPhone {
        fn source_xml(&self) -> String {
            let handle = self.handle;
            let follow_control = if self.followed.load(Ordering::Relaxed) {
                String::new()
            } else {
                format!(
                    r#"          <node package="com.ss.android.ugc.trill" class="android.widget.Button" resource-id="com.ss.android.ugc.trill:id/fm1" text="" content-desc="Follow {handle}" bounds="[870,1450][1030,1530]" enabled="true" clickable="true" selected="false" />"#
                )
            };
            format!(
                r#"<?xml version="1.0" encoding="UTF-8"?>
<hierarchy rotation="0">
  <node package="com.ss.android.ugc.trill" class="android.widget.FrameLayout" resource-id="root" text="" content-desc="" bounds="[0,0][1080,2160]" enabled="true" clickable="false" selected="false">
    <node package="com.ss.android.ugc.trill" class="androidx.viewpager.widget.ViewPager" resource-id="com.ss.android.ugc.trill:id/tod" text="" content-desc="" bounds="[0,0][1080,2000]" enabled="true" clickable="false" selected="false">
      <node package="com.ss.android.ugc.trill" class="android.widget.LinearLayout" resource-id="com.ss.android.ugc.trill:id/hfp" text="" content-desc="" bounds="[0,0][1080,1900]" enabled="true" clickable="true" selected="false">
        <node package="com.ss.android.ugc.trill" class="android.widget.FrameLayout" resource-id="com.ss.android.ugc.trill:id/cv2" text="" content-desc="" bounds="[0,100][1080,1900]" enabled="true" clickable="true" selected="false">
          <node package="com.ss.android.ugc.trill" class="android.widget.ImageView" resource-id="com.ss.android.ugc.trill:id/t40" text="" content-desc="{handle} profile" bounds="[24,1500][580,1560]" enabled="true" clickable="true" selected="false" />
{follow_control}
        </node>
      </node>
    </node>
    <node package="com.ss.android.ugc.trill" class="android.widget.LinearLayout" resource-id="" text="" content-desc="For You" bounds="[540,2020][1080,2160]" enabled="true" clickable="false" selected="true" />
  </node>
</hierarchy>"#
            )
        }
    }

    #[async_trait::async_trait]
    impl UiSession for ProvedFollowPhone {
        async fn tap(&self, _point: TapPoint) -> anyhow::Result<()> {
            self.taps.fetch_add(1, Ordering::Relaxed);
            self.followed.store(self.confirm, Ordering::Relaxed);
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

        async fn find_and_tap(&self, _accessibility_id: &str) -> anyhow::Result<()> {
            Ok(())
        }

        async fn assert_visible(&self, _accessibility_id: &str) -> anyhow::Result<()> {
            Ok(())
        }

        async fn active_app_bundle(&self) -> anyhow::Result<String> {
            Ok("com.ss.android.ugc.trill".into())
        }

        async fn ui_language(&self) -> Option<String> {
            Some("en".into())
        }

        async fn app_version(&self, _bundle_id: &str) -> Option<String> {
            Some("38.3.2".into())
        }

        async fn hierarchy_source_snapshot(
            &self,
        ) -> anyhow::Result<crate::HierarchySourceSnapshot> {
            Ok(crate::HierarchySourceSnapshot {
                generation: if self.taps.load(Ordering::Relaxed) > 0 {
                    2
                } else {
                    1
                },
                xml: self.source_xml(),
            })
        }

        async fn locate(&self, query: ElementQuery<'_>) -> anyhow::Result<Option<ElementBox>> {
            let ElementQuery::Description { value, .. } = query else {
                return Ok(None);
            };
            let found = match value {
                "For You" => Some("For You".to_owned()),
                " profile" => Some(format!("{} profile", self.handle)),
                "comments" => Some("Read or add comments. 12 comments".to_owned()),
                "Share video" => Some("Share video. 3".to_owned()),
                "Sound:" => Some("Sound: fixture".to_owned()),
                "Follow " if !self.followed.load(Ordering::Relaxed) => {
                    Some(format!("Follow {}", self.handle))
                }
                _ => None,
            };
            Ok(found.map(|description| ElementBox {
                x: 870.0,
                y: 1_450.0,
                width: 160.0,
                height: 80.0,
                description: Some(description),
                enabled: true,
                clickable: true,
            }))
        }

        fn stream_url(&self) -> Option<String> {
            None
        }
    }

    fn follow_fixture() -> (Database, std::path::PathBuf) {
        let path = std::env::temp_dir().join(format!(
            "riviu-nurture-follow-runtime-test-{}.db",
            Uuid::new_v4()
        ));
        (Database::open(&path).expect("follow database"), path)
    }

    #[test]
    fn unconfirmed_follow_latches_partial_session_outcome() {
        let mut outcome = Outcome::Done;
        FollowVerdict::NotConfirmed.latch_session_outcome(&mut outcome);
        let attempts = u32::from(FollowVerdict::NotConfirmed.did_act());
        let confirmed = u32::from(FollowVerdict::NotConfirmed == FollowVerdict::Followed);
        assert_eq!(outcome, Outcome::Partial);
        assert_eq!((attempts, confirmed), (1, 0));

        FollowVerdict::NoControl.latch_session_outcome(&mut outcome);
        assert_eq!(outcome, Outcome::Partial);
    }

    struct StopOnFollowArm<'a> {
        inner: crate::nurture::NurtureFollowLedger<'a>,
        stop: &'a AtomicBool,
    }

    impl NurtureFollowJournal for StopOnFollowArm<'_> {
        fn arm(
            &self,
            identity: &crate::tiktok_follow_cleanup::NurtureFollowSourceIdentity,
        ) -> anyhow::Result<crate::nurture::NurtureFollowLease> {
            let lease = self.inner.arm(identity)?;
            self.stop.store(true, Ordering::Relaxed);
            Ok(lease)
        }

        fn settle_failed_before_effect(
            &self,
            lease: crate::nurture::NurtureFollowLease,
            evidence: &str,
            error_code: &str,
        ) -> anyhow::Result<()> {
            self.inner
                .settle_failed_before_effect(lease, evidence, error_code)
        }

        fn settle_confirmed(
            &self,
            lease: crate::nurture::NurtureFollowLease,
            readback: &crate::tiktok_follow_cleanup::NurtureFollowReadback,
            evidence: &str,
        ) -> anyhow::Result<()> {
            self.inner.settle_confirmed(lease, readback, evidence)
        }

        fn settle_uncertain(
            &self,
            lease: crate::nurture::NurtureFollowLease,
            evidence: &str,
            error_code: &str,
        ) -> anyhow::Result<()> {
            self.inner.settle_uncertain(lease, evidence, error_code)
        }
    }

    #[tokio::test]
    async fn hierarchy_follow_stop_after_arm_settles_before_effect_without_tap() {
        let phone = ProvedFollowPhone {
            handle: "@exact.author",
            confirm: true,
            taps: std::sync::atomic::AtomicUsize::new(0),
            followed: std::sync::atomic::AtomicBool::new(false),
        };
        let (db, path) = follow_fixture();
        let stop = AtomicBool::new(false);
        let journal = StopOnFollowArm {
            inner: crate::nurture::NurtureFollowLedger::new(&db, "session-stop", "device-a"),
            stop: &stop,
        };
        let screen = (1_080.0, 2_220.0);
        let mut run = HierarchyRun {
            session: &phone,
            labels: english_38(),
            screen,
            planner: TouchPointPlanner::new(screen),
            search_keyword: None,
        };
        let expected = PostFingerprint {
            author: Some("@exact.author profile".into()),
            comments: Some("Read or add comments. 12 comments".into()),
            share: Some("Share video. 3".into()),
            sound: Some("Sound: fixture".into()),
        };

        assert_eq!(
            run.follow(&expected, &stop, Some(&journal))
                .await
                .expect("cancelled Follow"),
            FollowVerdict::Cancelled
        );
        assert_eq!(phone.taps.load(Ordering::Relaxed), 0);
        let connection = rusqlite::Connection::open(&path).expect("read follow database");
        let row: (String, String, i64, i64) = connection
            .query_row(
                "SELECT action.state, action.error_code,
                        (SELECT COUNT(*) FROM nurture_follow_armed_witnesses),
                        (SELECT COUNT(*) FROM nurture_follow_source_identities)
                 FROM tiktok_action_runs AS action WHERE action.action_kind='follow'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .expect("settled Follow row");
        assert_eq!(
            row,
            (
                "failed_before_effect".into(),
                "follow_cancelled_before_tap".into(),
                1,
                0,
            )
        );
        drop(connection);
        drop(db);
        std::fs::remove_file(path).expect("remove follow database");
    }

    #[tokio::test]
    async fn hierarchy_follow_persists_atomic_source_after_confirmation() {
        let phone = ProvedFollowPhone {
            handle: "@exact.author",
            confirm: true,
            taps: std::sync::atomic::AtomicUsize::new(0),
            followed: std::sync::atomic::AtomicBool::new(false),
        };
        let (db, path) = follow_fixture();
        let ledger = crate::nurture::NurtureFollowLedger::new(&db, "session-a", "device-a");
        let screen = (1_080.0, 2_220.0);
        let mut run = HierarchyRun {
            session: &phone,
            labels: english_38(),
            screen,
            planner: TouchPointPlanner::new(screen),
            search_keyword: None,
        };
        let expected = PostFingerprint {
            author: Some("@exact.author profile".into()),
            comments: Some("Read or add comments. 12 comments".into()),
            share: Some("Share video. 3".into()),
            sound: Some("Sound: fixture".into()),
        };

        assert_eq!(
            run.follow(&expected, &AtomicBool::new(false), Some(&ledger))
                .await
                .expect("follow with durable source"),
            FollowVerdict::Followed
        );
        assert_eq!(phone.taps.load(Ordering::Relaxed), 1);
        let connection = rusqlite::Connection::open(&path).expect("read follow database");
        let row: (String, String, i64) = connection
            .query_row(
                "SELECT action.state,source.canonical_handle,COUNT(*)
                 FROM tiktok_action_runs AS action
                 JOIN nurture_follow_source_identities AS source
                   ON source.action_run_id=action.id
                 WHERE action.owner_kind='nurture' AND action.action_kind='follow'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .expect("confirmed source row");
        assert_eq!(row, ("confirmed".into(), "@exact.author".into(), 1));
        drop(connection);
        drop(db);
        std::fs::remove_file(path).expect("remove follow database");
    }

    #[tokio::test]
    async fn hierarchy_follow_uncertain_has_no_cleanup_source() {
        let phone = ProvedFollowPhone {
            handle: "@exact.author",
            confirm: false,
            taps: std::sync::atomic::AtomicUsize::new(0),
            followed: std::sync::atomic::AtomicBool::new(false),
        };
        let (db, path) = follow_fixture();
        let ledger = crate::nurture::NurtureFollowLedger::new(&db, "session-b", "device-a");
        let screen = (1_080.0, 2_220.0);
        let mut run = HierarchyRun {
            session: &phone,
            labels: english_38(),
            screen,
            planner: TouchPointPlanner::new(screen),
            search_keyword: None,
        };
        let expected = PostFingerprint {
            author: Some("@exact.author profile".into()),
            comments: Some("Read or add comments. 12 comments".into()),
            share: Some("Share video. 3".into()),
            sound: Some("Sound: fixture".into()),
        };

        assert_eq!(
            run.follow(&expected, &AtomicBool::new(false), Some(&ledger))
                .await
                .expect("uncertain follow"),
            FollowVerdict::NotConfirmed
        );
        assert_eq!(phone.taps.load(Ordering::Relaxed), 1);
        let connection = rusqlite::Connection::open(&path).expect("read follow database");
        let action_state: String = connection
            .query_row(
                "SELECT state FROM tiktok_action_runs WHERE action_kind='follow'",
                [],
                |row| row.get(0),
            )
            .expect("uncertain action row");
        let source_count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM nurture_follow_source_identities",
                [],
                |row| row.get(0),
            )
            .expect("source count");
        assert_eq!(action_state, "uncertain");
        assert_eq!(source_count, 0);
        drop(connection);
        drop(db);
        std::fs::remove_file(path).expect("remove follow database");
    }

    #[tokio::test]
    async fn hierarchy_follow_noncanonical_author_is_noop_without_action_source() {
        let phone = ProvedFollowPhone {
            handle: "Display Name",
            confirm: true,
            taps: std::sync::atomic::AtomicUsize::new(0),
            followed: std::sync::atomic::AtomicBool::new(false),
        };
        let (db, path) = follow_fixture();
        let ledger = crate::nurture::NurtureFollowLedger::new(&db, "session-c", "device-a");
        let screen = (1_080.0, 2_220.0);
        let mut run = HierarchyRun {
            session: &phone,
            labels: english_38(),
            screen,
            planner: TouchPointPlanner::new(screen),
            search_keyword: None,
        };
        let expected = PostFingerprint {
            author: Some("Display Name profile".into()),
            comments: Some("Read or add comments. 12 comments".into()),
            share: Some("Share video. 3".into()),
            sound: Some("Sound: fixture".into()),
        };

        assert_eq!(
            run.follow(&expected, &AtomicBool::new(false), Some(&ledger))
                .await
                .expect("typed source refusal"),
            FollowVerdict::SourceUnavailable
        );
        assert_eq!(phone.taps.load(Ordering::Relaxed), 0);
        let connection = rusqlite::Connection::open(&path).expect("read follow database");
        let action_count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM tiktok_action_runs WHERE action_kind='follow'",
                [],
                |row| row.get(0),
            )
            .expect("action count");
        let source_count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM nurture_follow_source_identities",
                [],
                |row| row.get(0),
            )
            .expect("source count");
        assert_eq!((action_count, source_count), (0, 0));
        drop(connection);
        drop(db);
        std::fs::remove_file(path).expect("remove follow database");
    }

    struct PhaseFailurePhone {
        fail_locate: Option<TikTokControl>,
        comments_absent: bool,
        tap_fails: bool,
        taps: std::sync::atomic::AtomicUsize,
        typed: std::sync::atomic::AtomicUsize,
    }

    #[async_trait::async_trait]
    impl UiSession for PhaseFailurePhone {
        async fn tap(&self, _point: TapPoint) -> anyhow::Result<()> {
            self.taps.fetch_add(1, Ordering::Relaxed);
            if self.tap_fails {
                anyhow::bail!("tap response was lost");
            }
            Ok(())
        }

        async fn swipe(&self, _gesture: crate::types::SwipeGesture) -> anyhow::Result<()> {
            Ok(())
        }

        async fn type_text(&self, _text: &str) -> anyhow::Result<()> {
            self.typed.fetch_add(1, Ordering::Relaxed);
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

        async fn locate(&self, query: ElementQuery<'_>) -> anyhow::Result<Option<ElementBox>> {
            let labels = vietnamese();
            let matches = |control| {
                labels
                    .label(control)
                    .is_some_and(|label| label.to_query() == query)
            };
            if self.fail_locate.is_some_and(matches) {
                anyhow::bail!("scripted locate failed before its control was returned");
            }
            let element = |description: &str| {
                Some(ElementBox {
                    x: 900.0,
                    y: 1_000.0,
                    width: 80.0,
                    height: 80.0,
                    description: Some(description.into()),
                    enabled: true,
                    clickable: true,
                })
            };
            Ok(if matches(TikTokControl::Liked) {
                None
            } else if matches(TikTokControl::Like) {
                element("Thích")
            } else if matches(TikTokControl::FeedTab) {
                element("Đề xuất")
            } else if matches(TikTokControl::AuthorProfileLink) {
                element("Hồ sơ author_a")
            } else if matches(TikTokControl::Comments) && !self.comments_absent {
                element("12 bình luận")
            } else if matches(TikTokControl::Share) {
                element("Chia sẻ video. 3")
            } else if matches(TikTokControl::Follow) {
                element("Follow author_a")
            } else {
                None
            })
        }
    }

    fn phase_failure_run(phone: &PhaseFailurePhone) -> HierarchyRun<'_> {
        let screen = (1_080.0, 2_220.0);
        HierarchyRun {
            session: phone,
            labels: vietnamese(),
            screen,
            planner: TouchPointPlanner::new(screen),
            search_keyword: None,
        }
    }

    fn one_attempt_policy(action: PolicyAction) -> HumanSessionPolicy {
        let mut policy = HumanSessionPolicy::new(100, 100, 100, true);
        policy.pin_cap_for_test(action, 1);
        policy.begin_post();
        policy
    }

    fn assert_before_effect_refunds(
        action: PolicyAction,
        failure: &ActionFailure,
        taps: usize,
        typed: usize,
    ) {
        let mut policy = one_attempt_policy(action);
        let reservation = policy.reserve_attempt(action);
        let mut attempts = 0;
        assert!(!settle_action_failure(
            &mut policy,
            reservation,
            failure,
            &mut attempts,
        ));
        assert_eq!(taps, 0);
        assert_eq!(typed, 0);
        assert_eq!(attempts, 0);
        assert!(policy.can_attempt(action));
        assert!(policy.can_interact_with_post());
    }

    #[tokio::test]
    async fn hierarchy_like_locate_failure_refunds_before_effect() {
        let phone = PhaseFailurePhone {
            fail_locate: Some(TikTokControl::Like),
            comments_absent: false,
            tap_fails: false,
            taps: 0.into(),
            typed: 0.into(),
        };
        let failure = phase_failure_run(&phone)
            .like(&AtomicBool::new(false))
            .await
            .expect_err("scripted Like locate must fail");
        assert_before_effect_refunds(
            PolicyAction::Like,
            &failure,
            phone.taps.load(Ordering::Relaxed),
            phone.typed.load(Ordering::Relaxed),
        );
    }

    #[tokio::test]
    async fn hierarchy_comment_locate_failure_refunds_before_effect() {
        let phone = PhaseFailurePhone {
            fail_locate: Some(TikTokControl::Comments),
            comments_absent: false,
            tap_fails: false,
            taps: 0.into(),
            typed: 0.into(),
        };
        let failure = phase_failure_run(&phone)
            .comment("không được gõ", &AtomicBool::new(false))
            .await
            .expect_err("scripted comment opener locate must fail");
        assert_before_effect_refunds(
            PolicyAction::Comment,
            &failure,
            phone.taps.load(Ordering::Relaxed),
            phone.typed.load(Ordering::Relaxed),
        );
    }

    #[tokio::test]
    async fn hierarchy_follow_locate_failure_refunds_before_effect() {
        let phone = PhaseFailurePhone {
            fail_locate: Some(TikTokControl::Follow),
            comments_absent: false,
            tap_fails: false,
            taps: 0.into(),
            typed: 0.into(),
        };
        let expected = PostFingerprint {
            author: Some("Hồ sơ author_a".into()),
            comments: Some("12 bình luận".into()),
            share: Some("Chia sẻ video. 3".into()),
            sound: None,
        };
        let failure = phase_failure_run(&phone)
            .follow(&expected, &AtomicBool::new(false), None)
            .await
            .expect_err("scripted Follow locate must fail");
        assert_before_effect_refunds(
            PolicyAction::Follow,
            &failure,
            phone.taps.load(Ordering::Relaxed),
            phone.typed.load(Ordering::Relaxed),
        );
    }

    #[tokio::test]
    async fn an_ambiguous_hierarchy_tap_failure_closes_the_cap() {
        let phone = PhaseFailurePhone {
            fail_locate: None,
            comments_absent: false,
            tap_fails: true,
            taps: 0.into(),
            typed: 0.into(),
        };
        let failure = phase_failure_run(&phone)
            .like(&AtomicBool::new(false))
            .await
            .expect_err("scripted tap response must fail");
        let mut policy = one_attempt_policy(PolicyAction::Like);
        let reservation = policy.reserve_attempt(PolicyAction::Like);
        let mut attempts = 0;

        assert!(settle_action_failure(
            &mut policy,
            reservation,
            &failure,
            &mut attempts,
        ));
        assert_eq!(phone.taps.load(Ordering::Relaxed), 1);
        assert_eq!(attempts, 1);
        assert!(!policy.can_attempt(PolicyAction::Like));
        policy.begin_post();
        policy.mark_post_interacted();
        policy.begin_post();
        assert!(!policy.can_interact_with_post());
    }

    #[test]
    fn every_measured_build_can_be_asked_to_follow() {
        // The guard this exercises exists because `follow` finds nothing when the label
        // was never measured, returns false, and the loop then reported "nút Follow vẫn
        // còn — chưa xác nhận" about a screen where nothing was tapped — after spending
        // `record_attempt` and `mark_post_interacted`, so a roll that could do nothing
        // blocked the like that might have used the budget.
        //
        // Sixteen of twenty phones were in that state this morning. The label was there
        // the whole time: `Follow <author>` appears only on cards whose author is not
        // followed yet, so the first dump anyone took happened not to show it. It is
        // measured now, and this asserts the state that follows from that — while the
        // guard stays, because the next build added here may arrive half-measured too.
        for set in crate::tiktok_labels::TIKTOK_LABEL_SETS {
            let controls =
                controls_for(set.package, set.language, "").expect("a declared set resolves");
            assert!(
                can_follow(controls),
                "{} / {} cannot follow: read `Follow <author>` off a card whose author is \
                 not followed yet — it is absent from the others",
                set.package,
                set.language
            );
        }
    }
    #[test]
    fn the_photo_counter_is_read_out_of_three_adjacent_nodes() {
        // The strings are the measured ones, in tree order, from an SM-N950F on
        // 12/08/2026 (`/photo/7668950377680735508`, five images).
        assert_eq!(
            parse_carousel_counter(&texts(&["Ảnh", "1", " / ", "5", "huongthao.dalat"])),
            Some((1, 5))
        );
        // Two-image post, and the fourth image of five, both seen during the same run.
        assert_eq!(
            parse_carousel_counter(&texts(&["1", " / ", "2"])),
            Some((1, 2))
        );
        assert_eq!(
            parse_carousel_counter(&texts(&["4", " / ", "5"])),
            Some((4, 5))
        );
    }

    #[test]
    fn a_card_without_a_counter_is_not_treated_as_a_photo_post() {
        // This is the safety property, not a tidiness one: a sideways swipe on a *video*
        // is TikTok's open-the-author's-profile gesture, so a false positive here walks the
        // session off the feed. Measured: once the last image is passed the counter
        // disappears entirely, which is this case too.
        assert_eq!(parse_carousel_counter(&[]), None);
        assert_eq!(
            parse_carousel_counter(&texts(&["Ảnh", "huongthao.dalat", "3 bình luận"])),
            None
        );
        // A slash that is part of one node — a date — must not look like a counter.
        assert_eq!(parse_carousel_counter(&texts(&["12/08", "2026"])), None);
        // A bare slash whose neighbours are not numbers.
        assert_eq!(
            parse_carousel_counter(&texts(&["Trang", " / ", "chủ"])),
            None
        );
        // Nonsense ordering: a counter cannot be past its own total.
        assert_eq!(parse_carousel_counter(&texts(&["7", " / ", "5"])), None);
        assert_eq!(parse_carousel_counter(&texts(&["0", " / ", "5"])), None);
    }

    #[test]
    fn the_portion_is_computed_from_the_post_s_own_image_count() {
        // What the operator asked for: "lướt hết" and "lướt 1 nửa". The total comes from
        // the card, so half means half of *this* post rather than a guess — the pixel
        // engine cannot do this, it has no counter to read.
        assert_eq!(carousel_target(5, 100, 12), 5);
        assert_eq!(carousel_target(5, 50, 12), 3, "half of five rounds up");
        assert_eq!(carousel_target(2, 50, 12), 1);
        assert_eq!(carousel_target(8, 25, 12), 2);
        // The first image is on screen before any swipe, so a run can never view zero.
        assert_eq!(carousel_target(5, 1, 12), 1);
        assert_eq!(carousel_target(5, 0, 12), 1);
        // A percentage cannot invent images, and the safety ceiling still bites.
        assert_eq!(carousel_target(20, 100, 12), 12);
        assert_eq!(carousel_target(3, 100, 0), 1);
    }

    #[test]
    fn a_control_with_no_measured_label_is_not_looked_for() {
        // `LiveRoom` is unmeasured on the Vietnamese build. The loop must treat
        // that as "do not query", which is what keeps it from inventing a
        // translation — and it is why a LIVE card there is handled by the card
        // having no rail rather than by recognising it.
        assert!(vietnamese().label(TikTokControl::LiveRoom).is_none());
        assert!(vietnamese().label(TikTokControl::Like).is_some());
        assert!(vietnamese().label(TikTokControl::Liked).is_some());
    }

    #[test]
    fn an_unmeasured_build_refuses_with_the_measured_sets_listed() {
        let refusal = Unsupported::NoLabelSet {
            package: "com.example.fake".into(),
            language: Some("th".into()),
        };
        let message = refusal.message();
        assert!(message.contains("com.example.fake"));
        assert!(message.contains("th"));
        // The operator needs to know which sets *do* exist to act on this.
        assert!(message.contains("com.ss.android.ugc.trill"));
        assert!(message.contains("com.zhiliaoapp.musically"));
    }

    #[test]
    fn a_missing_language_says_so_rather_than_printing_none() {
        let message = Unsupported::NoLabelSet {
            package: "com.ss.android.ugc.trill".into(),
            language: None,
        }
        .message();
        assert!(message.contains("không đọc được"), "{message}");
        assert!(!message.contains("None"), "{message}");
    }

    #[tokio::test]
    async fn the_session_rechecks_the_fleet_build_before_the_first_tap() {
        use crate::tiktok_fleet_policy::fixtures::BuildSession;
        let settings = NurtureSettings::default();
        for (package, version, language) in [
            ("com.zhiliaoapp.musically", "46.4.3", "en"),
            ("com.ss.android.ugc.trill", "38.3.2", "en"),
            ("com.zhiliaoapp.musically", "45.7.3", "vi"),
        ] {
            let session = BuildSession {
                package,
                language: Some(language),
                version: Some(version),
            };
            match HierarchyRun::prepare(&session, (1080.0, 2220.0), &settings).await {
                Err(refusal @ Unsupported::BuildNotAllowed(_)) => {
                    let message = refusal.message();
                    assert!(
                        message.starts_with("failed — TikTok trên máy đang là"),
                        "{message}"
                    );
                    assert!(message.contains(&format!("{package} {version} ({language})")));
                }
                Err(other) => panic!("{package} {version}: wrong refusal {other:?}"),
                Ok(_) => panic!("{package} {version} {language}: build outside the fleet opened"),
            }
        }
        let allowed = BuildSession {
            package: "com.zhiliaoapp.musically",
            language: Some("en"),
            version: Some("45.7.3"),
        };
        assert!(HierarchyRun::prepare(&allowed, (1080.0, 2220.0), &settings)
            .await
            .is_ok());
    }

    #[test]
    fn a_pixel_only_backend_is_directed_at_the_image_path() {
        let message = Unsupported::NoElementBounds.message();
        assert!(message.contains("nhận dạng ảnh"), "{message}");
        // Not a failure — the caller falls through to the pixel engine.
        assert!(!message.starts_with("failed"), "{message}");
    }

    #[test]
    fn only_measured_builds_count_as_drivable_tiktok() {
        assert!(is_measured_tiktok("com.ss.android.ugc.trill"));
        assert!(is_measured_tiktok("com.zhiliaoapp.musically"));
        // The iOS bundle is the default in `NurtureSettings`, and no Android phone
        // can launch it — the loop must not try.
        assert!(!is_measured_tiktok("com.ss.iphone.ugc.Ame"));
        assert!(!is_measured_tiktok(""));
    }

    #[test]
    fn fingerprints_differ_when_the_counts_differ() {
        let first = PostFingerprint {
            author: None,
            comments: Some("Đọc hoặc viết bình luận. 697 bình luận".into()),
            share: Some("Chia sẻ video. 45,4K lượt chia sẻ".into()),
            sound: None,
        };
        let second = PostFingerprint {
            author: None,
            comments: Some("Đọc hoặc viết bình luận. 12 bình luận".into()),
            share: first.share.clone(),
            sound: None,
        };
        assert_ne!(first, second);
        assert!(!first.is_empty());
    }

    #[test]
    fn duplicate_guard_uses_stable_identity_without_weakening_swipe_equality() {
        let acted = PostFingerprint {
            author: Some("author_a profile".into()),
            comments: Some("12 comments".into()),
            share: Some("Share video. 3".into()),
            sound: Some("Sound: track_a".into()),
        };
        let counts_changed = PostFingerprint {
            comments: Some("13 comments".into()),
            share: Some("Share video. 4".into()),
            ..acted.clone()
        };
        assert_ne!(
            acted, counts_changed,
            "full equality remains the sensitive swipe/card-change proof"
        );
        assert!(acted.same_card_for_duplicate_guard(&counts_changed));

        let different_sound = PostFingerprint {
            sound: Some("Sound: track_b".into()),
            ..counts_changed.clone()
        };
        assert!(!acted.same_card_for_duplicate_guard(&different_sound));

        let author_only = PostFingerprint {
            sound: None,
            ..acted.clone()
        };
        let author_only_with_new_counts = PostFingerprint {
            comments: Some("99 comments".into()),
            share: Some("Share video. 100".into()),
            ..author_only.clone()
        };
        assert!(
            author_only.same_card_for_duplicate_guard(&author_only_with_new_counts),
            "only-author identity must fail closed: one skipped action is cheaper than a duplicate"
        );
    }

    #[test]
    fn two_zero_engagement_posts_are_still_told_apart() {
        // Verbatim from two phones on 18/08/2026. Counts alone are identical on a
        // low-engagement feed, so a fingerprint without the sound strip reported every
        // swipe as unproven: no video was counted, and the session stopped at
        // `STUCK_SWIPE_LIMIT` while it was in fact watching and swiping correctly.
        let counts_only = |sound: Option<&str>| PostFingerprint {
            author: None,
            comments: Some("Read or add comments. 0 comments".into()),
            share: Some("Share video.  shares".into()),
            sound: sound.map(str::to_string),
        };

        assert_eq!(
            counts_only(None),
            counts_only(None),
            "this equality is the defect: two different cards, one fingerprint"
        );
        assert_ne!(
            counts_only(Some("Original sound by Jacketkat")),
            counts_only(Some("Original sound by BapMidnight"))
        );
        // A post on licensed music has no `Original sound by` strip. That degrades to the
        // old behaviour rather than to something worse, and the rail is still detected.
        assert!(!counts_only(None).is_empty());
    }

    #[test]
    fn an_empty_fingerprint_never_counts_as_a_new_post() {
        // Both sides blank happens off-feed. Counting it would inflate
        // `videos_done` with cards that were never seen.
        let blank = PostFingerprint::default();
        assert!(blank.is_empty());
        assert_eq!(blank, PostFingerprint::default());
    }

    #[test]
    fn element_geometry_taps_inside_the_control() {
        let element = ElementBox {
            x: 1_000.0,
            y: 1_400.0,
            width: 120.0,
            height: 120.0,
            description: Some("Thích".into()),
            enabled: true,
            clickable: true,
        };
        let centre = element.centre();
        assert_eq!((centre.x, centre.y), (1_060.0, 1_460.0));
        let (rx, ry) = element.jitter_radius();
        // Well inside the 60pt half-extent, so jitter cannot reach a neighbour.
        assert_eq!((rx, ry), (24.0, 24.0));

        let mut planner = TouchPointPlanner::new((1_080.0, 2_400.0));
        for _ in 0..64 {
            let point = planner.next(element.centre(), element.jitter_radius());
            assert!((1_036.0..=1_084.0).contains(&point.x), "{point:?}");
            assert!((1_436.0..=1_484.0).contains(&point.y), "{point:?}");
        }
    }

    #[test]
    fn a_degenerate_rectangle_still_yields_a_usable_radius() {
        // A zero-height element would otherwise ask the planner for radius 0 and
        // pin every tap to one coordinate.
        let sliver = ElementBox {
            x: 10.0,
            y: 10.0,
            width: 0.0,
            height: 0.0,
            description: None,
            enabled: true,
            clickable: true,
        };
        assert_eq!(sliver.jitter_radius(), (1.0, 1.0));
    }

    #[tokio::test]
    async fn the_action_gap_is_not_waited_out_twice() {
        let stop = AtomicBool::new(false);
        let mut last = None;
        let started = Instant::now();
        // First call has nothing to wait for.
        assert!(wait_gap(&mut last, Duration::from_millis(200), &stop).await);
        assert!(started.elapsed() < Duration::from_millis(150));
        assert!(last.is_some());
    }

    #[tokio::test]
    async fn a_stop_during_the_gap_ends_the_session() {
        let stop = AtomicBool::new(true);
        let mut last = Some(Instant::now());
        assert!(!wait_gap(&mut last, Duration::from_millis(50), &stop).await);
    }

    /// A phone held behind a modal — and behind it, nothing else at all.
    ///
    /// The shape is measured rather than invented: the dump taken from
    /// `ce0517155ab38c390d` on 18/08/2026, while it sat behind TikTok's save-login prompt,
    /// offered the dialog and neither the feed tab nor the Home tab. That is what makes
    /// this case worth its own test — every *other* way out of [`await_feed`] looks for a
    /// control to tap, and behind a modal there is none to find, so the loop could only
    /// wait out its whole window and then blame a splash screen.
    #[derive(Default)]
    struct ModalPhone {
        dismissed: std::sync::atomic::AtomicBool,
        taps: std::sync::atomic::AtomicUsize,
    }

    #[async_trait::async_trait]
    impl UiSession for ModalPhone {
        async fn tap(&self, _point: TapPoint) -> anyhow::Result<()> {
            self.taps.fetch_add(1, Ordering::Relaxed);
            self.dismissed.store(true, Ordering::Relaxed);
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
            let dismissed = self.dismissed.load(Ordering::Relaxed);
            let found = match query {
                // While the dialog is up it is the only thing on the tree, decline button
                // included. `Not now` is a `Text` match because that is how it was measured.
                ElementQuery::Text { value, .. } => !dismissed && value == "Not now",
                // The feed tab — and the Home tab, which shares this arm — only exist once
                // the dialog is gone. Both being absent is the whole difficulty.
                ElementQuery::Description { value, .. } => dismissed && value == "For You",
                ElementQuery::ClassName(_) => false,
                // This path looks nothing up by id; answering it would be inventing a screen.
                ElementQuery::ResourceIdSuffix(_) | ElementQuery::Semantic(_) => false,
            };
            Ok(found.then_some(ElementBox {
                x: 420.0,
                y: 1_600.0,
                width: 240.0,
                height: 96.0,
                description: None,
                enabled: true,
                clickable: true,
            }))
        }
    }

    #[tokio::test]
    async fn a_modal_that_owns_the_screen_is_declined_instead_of_waited_out() {
        let phone = ModalPhone::default();
        let screen = (1_080.0, 2_220.0);
        let run = HierarchyRun {
            session: &phone,
            labels: controls_for("com.ss.android.ugc.trill", "en", "38.3.2").expect("measured set"),
            screen,
            planner: TouchPointPlanner::new(screen),
            search_keyword: None,
        };
        let stop = AtomicBool::new(false);
        let mut status = NurtureSessionStatus {
            running: true,
            last_message: String::new(),
            ..NurtureSessionStatus::new("modal-phone")
        };
        let said = std::sync::Mutex::new(Vec::<String>::new());
        let report = |status: &mut NurtureSessionStatus, message: String| {
            status.last_message = message.clone();
            said.lock().expect("messages").push(message);
        };

        // Without the decline this returns false, thirty seconds later.
        assert!(await_feed(&run, &stop, &mut status, &report).await);
        assert_eq!(
            phone.taps.load(Ordering::Relaxed),
            1,
            "declined once — the dialog is gone, so the next poll must not tap again"
        );
        let said = said.lock().expect("messages").clone();
        assert!(
            said.iter().any(|line| line.contains("hộp thoại")),
            "the operator is told what was in the way, not left with a guess: {said:?}"
        );
    }

    /// A phone behind a dialog that offers nothing anybody could tap.
    ///
    /// Measured on ce0517155ab38c390d, 18/08/2026: "Get updates sent to your email?" dumped
    /// **no** `content-desc` at all, and its one labelled button was the one that subscribes
    /// the account. So this fake answers nothing, whatever it is asked — which is exactly
    /// the state every label-driven recovery is blind to, including the measured decline.
    #[derive(Default)]
    struct UnlabelledModalPhone {
        /// How many screens deep the phone is. Zero reads as one, so the default is the
        /// single dialog this fake was built for; two is a search-results stack.
        depth: std::sync::atomic::AtomicUsize,
        backs: std::sync::atomic::AtomicUsize,
        taps: std::sync::atomic::AtomicUsize,
    }

    #[async_trait::async_trait]
    impl UiSession for UnlabelledModalPhone {
        async fn tap(&self, _point: TapPoint) -> anyhow::Result<()> {
            self.taps.fetch_add(1, Ordering::Relaxed);
            Ok(())
        }

        async fn back(&self) -> anyhow::Result<()> {
            self.backs.fetch_add(1, Ordering::Relaxed);
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
            // Behind the dialog: nothing, for any query. Underneath it: the feed.
            let out =
                self.backs.load(Ordering::Relaxed) >= self.depth.load(Ordering::Relaxed).max(1);
            let found = out
                && matches!(query, ElementQuery::Description { value, .. } if value == "For You");
            Ok(found.then_some(ElementBox {
                x: 0.0,
                y: 0.0,
                width: 200.0,
                height: 80.0,
                description: None,
                enabled: true,
                clickable: true,
            }))
        }
    }

    /// Takes about eleven seconds on purpose: [`MODAL_BACK_DELAY`] is most of it, and the
    /// delay is the part being asserted as much as the keypress is. A splash screen looks
    /// identical to this one — no feed tab, no bottom bar — and Back at a splash closes
    /// TikTok, so pressing it immediately would trade one broken session for a worse one.
    #[tokio::test]
    async fn a_dialog_with_no_decline_is_backed_out_of_rather_than_waited_out() {
        let phone = UnlabelledModalPhone::default();
        let screen = (1_080.0, 2_220.0);
        let run = HierarchyRun {
            session: &phone,
            // The set *with* a measured decline, to prove this path is not that one: the
            // dialog carries no text at all, so `Not now` finds nothing either.
            labels: controls_for("com.ss.android.ugc.trill", "en", "38.3.2").expect("measured set"),
            screen,
            planner: TouchPointPlanner::new(screen),
            search_keyword: None,
        };
        let stop = AtomicBool::new(false);
        let mut status = NurtureSessionStatus {
            running: true,
            last_message: String::new(),
            ..NurtureSessionStatus::new("unlabelled-modal")
        };
        let report = |status: &mut NurtureSessionStatus, message: String| {
            status.last_message = message;
        };

        let started = Instant::now();
        assert!(await_feed(&run, &stop, &mut status, &report).await);
        assert!(
            started.elapsed() >= MODAL_BACK_DELAY,
            "Back must not be pressed before a starting app has had its chance"
        );
        assert_eq!(
            phone.backs.load(Ordering::Relaxed),
            1,
            "one is enough *here*: the feed was directly underneath. A screen that Back \n             cannot leave is bounded by MODAL_BACK_LIMIT, not by stopping after one"
        );
        assert_eq!(
            phone.taps.load(Ordering::Relaxed),
            0,
            "nothing on that screen was safe to tap, so nothing was tapped"
        );
    }

    /// A dialog whose decline is always on screen and never does anything.
    ///
    /// Not a contrived shape: the decline is matched by label, and a tap can miss, a
    /// prompt can re-arm, and a screen can simply carry the word "Not now" somewhere the
    /// loop can find it. What made that fatal rather than merely useless is that the
    /// decline branch is the one recovery with no once-guard — deliberately, because
    /// TikTok stacks prompts and each hides the next.
    #[derive(Default)]
    struct StuckDialogPhone {
        taps: std::sync::atomic::AtomicUsize,
        stop_on_decline_read: Option<std::sync::Arc<AtomicBool>>,
        read_delay: Option<Duration>,
    }

    #[async_trait::async_trait]
    impl UiSession for StuckDialogPhone {
        async fn tap(&self, _point: TapPoint) -> anyhow::Result<()> {
            self.taps.fetch_add(1, Ordering::Relaxed);
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
            if let Some(delay) = self.read_delay {
                tokio::time::sleep(delay).await;
            }
            if matches!(query, ElementQuery::Text { .. }) {
                if let Some(stop) = &self.stop_on_decline_read {
                    stop.store(true, Ordering::Relaxed);
                }
            }
            // The decline is always there. The feed never is.
            Ok(
                matches!(query, ElementQuery::Text { .. }).then_some(ElementBox {
                    x: 0.0,
                    y: 0.0,
                    width: 100.0,
                    height: 40.0,
                    description: None,
                    enabled: true,
                    clickable: true,
                }),
            )
        }
    }

    #[tokio::test(start_paused = true)]
    async fn nurture_feed_wait_rejects_stop_during_decline_read_and_stalled_tree() {
        for cancelled in [true, false] {
            let stop = std::sync::Arc::new(AtomicBool::new(false));
            let phone = StuckDialogPhone {
                stop_on_decline_read: cancelled.then(|| stop.clone()),
                read_delay: (!cancelled).then_some(FEED_READY_WINDOW + Duration::from_secs(1)),
                ..Default::default()
            };
            let screen = (1_080.0, 2_220.0);
            let run = HierarchyRun {
                session: &phone,
                labels: english_38(),
                screen,
                planner: TouchPointPlanner::new(screen),
                search_keyword: None,
            };
            let mut status = NurtureSessionStatus::new("feed-wait");
            let report = |status: &mut NurtureSessionStatus, message: String| {
                status.last_message = message;
            };
            let started = tokio::time::Instant::now();
            let result = tokio::time::timeout(
                FEED_READY_WINDOW + Duration::from_secs(1),
                await_feed(&run, &stop, &mut status, &report),
            )
            .await;
            assert_eq!(result.ok(), Some(false));
            assert!(started.elapsed() <= FEED_READY_WINDOW);
            assert_eq!(phone.taps.load(Ordering::Relaxed), 0);
        }
    }

    /// Costs the full `FEED_READY_WINDOW` on purpose: the property is that the window is
    /// respected, so there is nothing to assert until it has passed. The outer timeout is
    /// what makes the failure legible — without the deadline in front of the branches this
    /// loop does not run long, it runs forever, and a hanging test says less than a failing
    /// one.
    #[tokio::test]
    async fn a_decline_that_never_works_still_ends_inside_the_window() {
        let phone = StuckDialogPhone::default();
        let screen = (1_080.0, 2_220.0);
        let run = HierarchyRun {
            session: &phone,
            labels: controls_for("com.ss.android.ugc.trill", "en", "38.3.2").expect("measured set"),
            screen,
            planner: TouchPointPlanner::new(screen),
            search_keyword: None,
        };
        let stop = AtomicBool::new(false);
        let mut status = NurtureSessionStatus {
            running: true,
            last_message: String::new(),
            ..NurtureSessionStatus::new("stuck-dialog")
        };
        let report = |status: &mut NurtureSessionStatus, message: String| {
            status.last_message = message;
        };

        let outcome = tokio::time::timeout(
            FEED_READY_WINDOW + Duration::from_secs(15),
            await_feed(&run, &stop, &mut status, &report),
        )
        .await;

        assert_eq!(
            outcome.ok(),
            Some(false),
            "await_feed never returned: the decline branch is outrunning its own deadline"
        );
        assert!(
            status.last_message.contains("chưa thấy tab feed"),
            "the operator is told the window ran out: {:?}",
            status.last_message
        );
        assert!(
            phone.taps.load(Ordering::Relaxed) > 0,
            "the decline was never attempted, so this proves nothing"
        );
    }

    /// A phone whose feed will not move until the app is restarted.
    ///
    /// Measured shape, from ce051715081fe20f03 on 18/08/2026: the same card for the nurture
    /// loop, for the agent, and for a plain `adb shell input swipe` — and a different card
    /// on the first swipe after a force-stop and relaunch. Every rail label is present the
    /// whole time, so nothing here looks broken; only the *values* refuse to change, which
    /// is exactly what a fingerprint is for.
    #[derive(Default)]
    struct StuckFeedPhone {
        generation: std::sync::atomic::AtomicUsize,
        restarts: std::sync::atomic::AtomicUsize,
        raises: std::sync::atomic::AtomicUsize,
        swipes: std::sync::atomic::AtomicUsize,
        incoming_at: Option<tokio::time::Instant>,
        rail_read_delay: Option<Duration>,
        stop_on_rail_read: Option<std::sync::Arc<AtomicBool>>,
        rail_read_error: bool,
    }

    impl StuckFeedPhone {
        fn card(&self, prefix: &str) -> Option<ElementBox> {
            let generation = self.generation.load(Ordering::Relaxed)
                + usize::from(
                    self.incoming_at
                        .is_some_and(|at| tokio::time::Instant::now() >= at),
                );
            Some(ElementBox {
                x: 900.0,
                y: 1_000.0,
                width: 80.0,
                height: 80.0,
                description: Some(format!("{prefix} {generation}")),
                enabled: true,
                clickable: true,
            })
        }
    }

    #[async_trait::async_trait]
    impl UiSession for StuckFeedPhone {
        async fn tap(&self, _point: TapPoint) -> anyhow::Result<()> {
            Ok(())
        }

        // The swipe lands and changes nothing — until the app has been restarted, after
        // which the feed behaves. That order is the measured shape: the gesture was never
        // the problem, the app's state was.
        async fn swipe(&self, _gesture: crate::types::SwipeGesture) -> anyhow::Result<()> {
            self.swipes.fetch_add(1, Ordering::Relaxed);
            if self.restarts.load(Ordering::Relaxed) > 0 {
                self.generation.fetch_add(1, Ordering::Relaxed);
            }
            Ok(())
        }

        async fn type_text(&self, _text: &str) -> anyhow::Result<()> {
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

        async fn launch_app_foreground(&self, _bundle_id: &str) -> anyhow::Result<()> {
            // Raising a process that is already in front is what the old code could do, and
            // it is counted here to prove it is not what unsticks this.
            self.raises.fetch_add(1, Ordering::Relaxed);
            Ok(())
        }

        async fn restart_app(&self, _bundle_id: &str) -> anyhow::Result<()> {
            self.restarts.fetch_add(1, Ordering::Relaxed);
            self.generation.fetch_add(1, Ordering::Relaxed);
            Ok(())
        }

        async fn locate(&self, query: ElementQuery<'_>) -> anyhow::Result<Option<ElementBox>> {
            let ElementQuery::Description { value, .. } = query else {
                return Ok(None);
            };
            if value == "comments" {
                if let Some(delay) = self.rail_read_delay {
                    tokio::time::sleep(delay).await;
                }
                if let Some(stop) = &self.stop_on_rail_read {
                    stop.store(true, Ordering::Relaxed);
                }
                anyhow::ensure!(!self.rail_read_error, "fixture rail transport timeout");
            }
            Ok(match value {
                "For You" => Some(ElementBox {
                    x: 0.0,
                    y: 0.0,
                    width: 100.0,
                    height: 40.0,
                    description: None,
                    enabled: true,
                    clickable: true,
                }),
                "comments" => self.card("comments"),
                "Share video" => self.card("Share video"),
                "Original sound by" => self.card("Original sound by"),
                _ => None,
            })
        }
    }

    fn original_stuck_feed_fingerprint() -> PostFingerprint {
        PostFingerprint {
            author: None,
            comments: Some("comments 0".into()),
            share: Some("Share video 0".into()),
            sound: None,
        }
    }

    #[tokio::test(start_paused = true)]
    async fn nurture_swipe_waits_for_changed_card_without_dispatching_another_swipe() {
        let phone = StuckFeedPhone {
            incoming_at: Some(tokio::time::Instant::now() + Duration::from_millis(1_200)),
            ..Default::default()
        };
        let screen = (1_080.0, 2_220.0);
        let mut run = HierarchyRun {
            session: &phone,
            labels: english_38(),
            screen,
            planner: TouchPointPlanner::new(screen),
            search_keyword: None,
        };
        assert!(
            run.swipe_next(
                320,
                &original_stuck_feed_fingerprint(),
                false,
                None,
                &AtomicBool::new(false)
            )
            .await
            .expect("readable incoming card")
            .0
        );
        assert_eq!(phone.swipes.load(Ordering::Relaxed), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn nurture_swipe_rejects_cancelled_late_and_failed_rail_reads() {
        // The late read outlives even the grace one read in flight gets at the deadline
        // ([`CARD_READ_GRACE`]); a read that finishes inside that grace is judged instead.
        for (delay, cancelled, failed) in [
            (Duration::from_millis(100), true, false),
            (
                SWIPE_SETTLE + RAIL_RETURN_WINDOW + CARD_READ_GRACE + Duration::from_secs(1),
                false,
                false,
            ),
            (Duration::ZERO, false, true),
        ] {
            let stop = std::sync::Arc::new(AtomicBool::new(false));
            let phone = StuckFeedPhone {
                generation: 1.into(),
                rail_read_delay: Some(delay),
                stop_on_rail_read: cancelled.then(|| stop.clone()),
                rail_read_error: failed,
                ..Default::default()
            };
            let screen = (1_080.0, 2_220.0);
            let mut run = HierarchyRun {
                session: &phone,
                labels: english_38(),
                screen,
                planner: TouchPointPlanner::new(screen),
                search_keyword: None,
            };
            let started = tokio::time::Instant::now();
            let result = run
                .swipe_next(320, &original_stuck_feed_fingerprint(), false, None, &stop)
                .await;
            if failed {
                assert!(result
                    .expect_err("read failure must stay a failure")
                    .to_string()
                    .contains("rail transport timeout"));
            } else {
                assert!(!result.expect("unproved cancelled/expired swipe").0);
            }
            assert!(started.elapsed() <= SWIPE_SETTLE + RAIL_RETURN_WINDOW + CARD_READ_GRACE);
            assert_eq!(phone.swipes.load(Ordering::Relaxed), 1);
        }
    }

    #[tokio::test]
    async fn stopped_nurture_does_not_launch_tiktok() {
        let phone = StuckFeedPhone::default();
        ensure_tiktok_foreground(&phone, english_38().package(), &AtomicBool::new(true))
            .await
            .expect_err("stopped before launch");
        assert_eq!(phone.raises.load(Ordering::Relaxed), 0);
    }

    /// Runs for real seconds: four honest swipe attempts have to happen before the restart
    /// is earned, and each one waits for the card to settle. That delay is the design —
    /// restarting costs the session its place in the feed, so it is not something to reach
    /// for on the first disappointment.
    #[tokio::test]
    async fn a_feed_that_will_not_advance_gets_the_app_restarted_once() {
        let phone = StuckFeedPhone::default();
        let screen = (1_080.0, 2_220.0);
        let run = HierarchyRun {
            session: &phone,
            labels: controls_for("com.ss.android.ugc.trill", "en", "38.3.2").expect("measured set"),
            screen,
            planner: TouchPointPlanner::new(screen),
            search_keyword: None,
        };
        let settings = NurtureSettings {
            num_videos: 4,
            num_rounds: 1,
            like_prob: 0,
            comment_prob: 0,
            follow_prob: 0,
            frenzy_prob: 0,
            watch_min: 0.1,
            watch_max: 0.1,
            ..Default::default()
        };
        let stop = AtomicBool::new(false);
        let mut status = NurtureSessionStatus {
            running: true,
            last_message: String::new(),
            ..NurtureSessionStatus::new("stuck-feed")
        };
        let said = std::sync::Mutex::new(Vec::<String>::new());
        let report = |status: &mut NurtureSessionStatus, message: String| {
            status.last_message = message.clone();
            said.lock().expect("messages").push(message);
        };

        run_feed(
            run,
            "com.ss.android.ugc.trill",
            &settings,
            Instant::now(),
            Some(Duration::from_secs(120)),
            &stop,
            &mut status,
            &report,
            None,
            None,
            None,
            None,
        )
        .await;

        let said = said.lock().expect("messages").clone();
        assert_eq!(
            phone.restarts.load(Ordering::Relaxed),
            1,
            "once — a feed still stuck after a restart is saying something a second will not \
             change: {said:?}"
        );
        assert!(
            said.iter()
                .any(|line| line.contains("khởi động lại TikTok")),
            "the operator is told why the app went away and came back: {said:?}"
        );
        assert!(
            status.videos_done > 0,
            "the restart is only worth doing if the feed moves afterwards: {said:?}"
        );
    }

    /// A card that never changes and never registers the like — the exact shape where a
    /// swallowed swipe would let the same post be acted on twice.
    ///
    /// The `Like` control is present and `Video liked` never appears, so the already-liked
    /// guard cannot self-limit; only the duplicate-action guard can. Every query answers
    /// with the SAME card, and the swipe changes nothing, so without the guard the loop
    /// would like this one post once per pass.
    #[derive(Default)]
    struct StuckCardPhone {
        likes: std::sync::atomic::AtomicUsize,
    }

    #[async_trait::async_trait]
    impl UiSession for StuckCardPhone {
        async fn tap(&self, _point: TapPoint) -> anyhow::Result<()> {
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
        async fn find_and_tap(&self, _accessibility_id: &str) -> anyhow::Result<()> {
            Ok(())
        }
        async fn assert_visible(&self, _accessibility_id: &str) -> anyhow::Result<()> {
            Ok(())
        }
        fn stream_url(&self) -> Option<String> {
            None
        }
        async fn restart_app(&self, _bundle_id: &str) -> anyhow::Result<()> {
            // A restart does not unstick this card, so the session simply ends — after the
            // guard has already capped the likes at one.
            Ok(())
        }
        async fn locate(&self, query: ElementQuery<'_>) -> anyhow::Result<Option<ElementBox>> {
            let ElementQuery::Description { value, .. } = query else {
                return Ok(None);
            };
            let card = |desc: &str| {
                Some(ElementBox {
                    x: 900.0,
                    y: 1_000.0,
                    width: 80.0,
                    height: 80.0,
                    description: Some(desc.to_string()),
                    enabled: true,
                    clickable: true,
                })
            };
            Ok(match value {
                "For You" => card("For You"),
                "comments" => card("one steady card"),
                "Share video" => card("one steady card"),
                "Sound:" => card("one steady card"),
                "Like" => {
                    self.likes.fetch_add(1, Ordering::Relaxed);
                    card("Like")
                }
                // Never becomes liked, so `AlreadyLiked` can never be the thing that stops a
                // second like — the duplicate-action guard has to.
                "Video liked" => None,
                _ => None,
            })
        }
    }

    /// **A card the swipe cannot leave is liked at most once.** Without the
    /// duplicate-action guard, `begin_post()` runs every pass and the same post is liked
    /// again and again from a real account. The `Like` node is present in both states here,
    /// so a re-tap would toggle a real like off — irreversibly, publicly.
    #[tokio::test]
    async fn a_card_a_swallowed_swipe_cannot_leave_is_acted_on_once() {
        let phone = StuckCardPhone::default();
        let screen = (1_080.0, 2_220.0);
        let run = HierarchyRun {
            session: &phone,
            labels: controls_for("com.ss.android.ugc.trill", "en", "38.3.2").expect("measured set"),
            screen,
            planner: TouchPointPlanner::new(screen),
            search_keyword: None,
        };
        let settings = NurtureSettings {
            num_videos: 6,
            num_rounds: 1,
            like_prob: 100,
            comment_prob: 0,
            follow_prob: 0,
            frenzy_prob: 0,
            watch_min: 0.1,
            watch_max: 0.1,
            ..Default::default()
        };
        let stop = AtomicBool::new(false);
        let mut status = NurtureSessionStatus {
            running: true,
            last_message: String::new(),
            ..NurtureSessionStatus::new("stuck-card")
        };
        let report = |status: &mut NurtureSessionStatus, message: String| {
            status.last_message = message;
        };
        run_feed(
            run,
            "com.ss.android.ugc.trill",
            &settings,
            Instant::now(),
            Some(Duration::from_secs(120)),
            &stop,
            &mut status,
            &report,
            None,
            None,
            None,
            None,
        )
        .await;
        assert_eq!(
            status.like_attempts, 1,
            "a stuck card must be liked exactly once; got {} — the swallowed swipe let the \
             same post be acted on again",
            status.like_attempts
        );
    }

    /// A stuck feed card whose own comment count becomes visible one hierarchy read after Send.
    /// The delayed count is the important part: the immediate refresh still sees the old value,
    /// then the swallowed swipe sees the increment and the full swipe fingerprint calls that an
    /// advance. The duplicate-action identity must nevertheless remember this is the same card.
    #[derive(Default)]
    struct StuckCommentCardPhone {
        pending_text: std::sync::atomic::AtomicBool,
        sent_comments: std::sync::atomic::AtomicUsize,
        comment_reads_after_send: std::sync::atomic::AtomicUsize,
    }

    impl StuckCommentCardPhone {
        fn element(description: Option<String>, x: f64, y: f64, enabled: bool) -> ElementBox {
            ElementBox {
                x,
                y,
                width: 80.0,
                height: 80.0,
                description,
                enabled,
                clickable: true,
            }
        }

        fn comments_description(&self) -> String {
            let sent = self.sent_comments.load(Ordering::Relaxed);
            let visible = if sent == 0 {
                0
            } else if self
                .comment_reads_after_send
                .fetch_add(1, Ordering::Relaxed)
                == 0
            {
                sent - 1
            } else {
                sent
            };
            format!("{} comments", 12 + visible)
        }
    }

    #[async_trait::async_trait]
    impl UiSession for StuckCommentCardPhone {
        async fn tap(&self, point: TapPoint) -> anyhow::Result<()> {
            // The fake gives Send its own lower-right rectangle. Opener and field taps do not
            // cross this boundary; a Send tap consumes the pending text and arms the delayed
            // count update described above.
            if point.x > 800.0
                && point.y > 1_500.0
                && self.pending_text.swap(false, Ordering::Relaxed)
            {
                self.sent_comments.fetch_add(1, Ordering::Relaxed);
                self.comment_reads_after_send.store(0, Ordering::Relaxed);
            }
            Ok(())
        }

        async fn swipe(&self, _gesture: crate::types::SwipeGesture) -> anyhow::Result<()> {
            // Swallowed: the same author, sound and post remain on screen.
            Ok(())
        }

        async fn type_text(&self, _text: &str) -> anyhow::Result<()> {
            self.pending_text.store(true, Ordering::Relaxed);
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

        async fn restart_app(&self, _bundle_id: &str) -> anyhow::Result<()> {
            // A restart is not proof of movement; this fixture deliberately comes back to the
            // same card so the duplicate guard remains the only thing preventing another send.
            Ok(())
        }

        async fn locate(&self, query: ElementQuery<'_>) -> anyhow::Result<Option<ElementBox>> {
            let element = match query {
                ElementQuery::Description { value, .. } => match value {
                    "For You" => Some(Self::element(Some("For You".into()), 100.0, 2_000.0, true)),
                    "comments" => Some(Self::element(
                        Some(self.comments_description()),
                        900.0,
                        1_000.0,
                        true,
                    )),
                    "Share video" => Some(Self::element(
                        Some("Share video. 3".into()),
                        900.0,
                        1_150.0,
                        true,
                    )),
                    "Sound:" => Some(Self::element(
                        Some("Sound: one steady card".into()),
                        300.0,
                        1_500.0,
                        true,
                    )),
                    " profile" => Some(Self::element(
                        Some("author_a profile".into()),
                        300.0,
                        300.0,
                        true,
                    )),
                    "Post comment" => Some(Self::element(
                        Some("Post comment".into()),
                        900.0,
                        1_800.0,
                        self.pending_text.load(Ordering::Relaxed),
                    )),
                    _ => None,
                },
                ElementQuery::ClassName(crate::tiktok_drawer::EDIT_TEXT) => {
                    Some(Self::element(None, 100.0, 1_800.0, true))
                }
                _ => None,
            };
            Ok(element)
        }
    }

    /// A sent comment may update the mutable count after the immediate refresh. Even when the
    /// full swipe proof mistakes that late update for a new card, a swallowed swipe must not let
    /// the loop type a second public comment into the same author+sound identity.
    #[tokio::test]
    async fn a_delayed_comment_count_cannot_reopen_a_stuck_card_for_another_comment() {
        let phone = StuckCommentCardPhone::default();
        let screen = (1_080.0, 2_220.0);
        let run = HierarchyRun {
            session: &phone,
            labels: controls_for("com.ss.android.ugc.trill", "en", "38.3.2").expect("measured set"),
            screen,
            planner: TouchPointPlanner::new(screen),
            search_keyword: None,
        };
        let settings = NurtureSettings {
            num_videos: 6,
            num_rounds: 1,
            like_prob: 0,
            comment_prob: 100,
            follow_prob: 0,
            frenzy_prob: 0,
            watch_min: 0.1,
            watch_max: 0.1,
            human_limits: false,
            ..Default::default()
        };
        let stop = AtomicBool::new(false);
        let mut status = NurtureSessionStatus {
            running: true,
            last_message: String::new(),
            ..NurtureSessionStatus::new("stuck-comment-card")
        };
        let report = |status: &mut NurtureSessionStatus, message: String| {
            status.last_message = message;
        };

        run_feed(
            run,
            "com.ss.android.ugc.trill",
            &settings,
            Instant::now(),
            Some(Duration::from_secs(120)),
            &stop,
            &mut status,
            &report,
            Some(&AlwaysHasWords),
            None,
            None,
            None,
        )
        .await;

        assert_eq!(
            phone.sent_comments.load(Ordering::Relaxed),
            1,
            "the delayed count made the same stuck card look new and receive another comment"
        );
        assert_eq!(status.comment_attempts, 1);
    }

    /// A phone that keeps falling off the feed, and comes back only via the Home tab.
    ///
    /// Measured on ce0717171c2a64d50d, 18/08/2026: it started cleanly, watched, liked, and
    /// then left the feed — after which the loop swiped six times at whatever screen it had
    /// landed on and ended the session at zero videos. Swiping is the one gesture that
    /// cannot help there: being off the feed means being on some *other* page, and swiping
    /// scrolls that page.
    #[derive(Default)]
    struct WandersOffPhone {
        generation: std::sync::atomic::AtomicUsize,
        on_feed: std::sync::atomic::AtomicBool,
        home_taps: std::sync::atomic::AtomicUsize,
        started: std::sync::atomic::AtomicBool,
    }

    impl WandersOffPhone {
        /// The bottom bar sits here, and nothing else does — which is how `tap` can tell a
        /// Home press from any other tap without being handed a label.
        const HOME_Y: f64 = 2_100.0;

        fn here(&self) -> bool {
            // First read arms it: `Default` gives `false` and the phone starts on the feed.
            if !self.started.swap(true, Ordering::Relaxed) {
                self.on_feed.store(true, Ordering::Relaxed);
            }
            self.on_feed.load(Ordering::Relaxed)
        }
    }

    #[async_trait::async_trait]
    impl UiSession for WandersOffPhone {
        /// A tap on the rail wanders off; a tap on the bottom bar comes back.
        ///
        /// This is the measured order and it matters: the *like* is what left the feed on
        /// ce0717171c2a64d50d, not the swipe. Modelling it the other way round makes the
        /// loop refuse the swipe for a good reason — a rail that vanishes mid-gesture means
        /// the card went somewhere, which is not an advance — and tests the wrong thing.
        async fn tap(&self, point: TapPoint) -> anyhow::Result<()> {
            if point.y >= Self::HOME_Y {
                self.home_taps.fetch_add(1, Ordering::Relaxed);
                self.on_feed.store(true, Ordering::Relaxed);
            } else if self.here() {
                self.on_feed.store(false, Ordering::Relaxed);
            }
            Ok(())
        }

        async fn swipe(&self, _gesture: crate::types::SwipeGesture) -> anyhow::Result<()> {
            if self.here() {
                self.generation.fetch_add(1, Ordering::Relaxed);
            }
            Ok(())
        }

        async fn type_text(&self, _text: &str) -> anyhow::Result<()> {
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

        async fn locate(&self, query: ElementQuery<'_>) -> anyhow::Result<Option<ElementBox>> {
            let ElementQuery::Description { value, .. } = query else {
                return Ok(None);
            };
            let generation = self.generation.load(Ordering::Relaxed);
            let card = |prefix: &str| {
                Some(ElementBox {
                    x: 900.0,
                    y: 1_000.0,
                    width: 80.0,
                    height: 80.0,
                    description: Some(format!("{prefix} {generation}")),
                    enabled: true,
                    clickable: true,
                })
            };
            // The bottom bar is on screen wherever we are; the rail is only on the feed.
            if value == "Home" {
                return Ok(Some(ElementBox {
                    x: 100.0,
                    y: Self::HOME_Y,
                    width: 60.0,
                    height: 60.0,
                    description: None,
                    enabled: true,
                    clickable: true,
                }));
            }
            if !self.here() {
                return Ok(None);
            }
            Ok(match value {
                "For You" => Some(ElementBox {
                    x: 0.0,
                    y: 0.0,
                    width: 100.0,
                    height: 40.0,
                    description: None,
                    enabled: true,
                    clickable: true,
                }),
                "Like" => card("Like"),
                "comments" => card("comments"),
                "Share video" => card("Share video"),
                "Original sound by" => card("Original sound by"),
                _ => None,
            })
        }
    }

    #[tokio::test]
    async fn a_session_that_falls_off_the_feed_walks_back_instead_of_swiping() {
        let phone = WandersOffPhone::default();
        let screen = (1_080.0, 2_220.0);
        let run = HierarchyRun {
            session: &phone,
            labels: controls_for("com.ss.android.ugc.trill", "en", "38.3.2").expect("measured set"),
            screen,
            planner: TouchPointPlanner::new(screen),
            search_keyword: None,
        };
        let settings = NurtureSettings {
            num_videos: 4,
            num_rounds: 1,
            like_prob: 100,
            comment_prob: 0,
            follow_prob: 0,
            frenzy_prob: 0,
            watch_min: 0.1,
            watch_max: 0.1,
            ..Default::default()
        };
        let stop = AtomicBool::new(false);
        let mut status = NurtureSessionStatus {
            running: true,
            last_message: String::new(),
            ..NurtureSessionStatus::new("wanders-off")
        };
        let said = std::sync::Mutex::new(Vec::<String>::new());
        let report = |status: &mut NurtureSessionStatus, message: String| {
            status.last_message = message.clone();
            said.lock().expect("messages").push(message);
        };

        run_feed(
            run,
            "com.ss.android.ugc.trill",
            &settings,
            Instant::now(),
            Some(Duration::from_secs(180)),
            &stop,
            &mut status,
            &report,
            None,
            None,
            None,
            None,
        )
        .await;

        let said = said.lock().expect("messages").clone();
        assert!(
            phone.home_taps.load(Ordering::Relaxed) > 0,
            "the way back was never taken: {said:?}"
        );
        assert!(
            said.iter().any(|line| line.contains("bấm Home để về feed")),
            "the operator is told the session left the feed and went back: {said:?}"
        );
        assert!(
            !said.iter().any(|line| line.contains("vuốt chưa chứng minh")),
            "a swipe was spent off the feed, which is the post this fix exists to save:              {said:?}"
        );
        assert!(
            status.videos_done >= 1,
            "falling off the feed must not end the session: {} watched, {said:?}",
            status.videos_done
        );
    }

    /// A photo post whose pager turns for a fling and ignores a drag.
    ///
    /// **Measured, not invented.** On TikTok 38.3.2, on the feed, a sideways `plan_swipe`
    /// path advanced the page counter on 3 of 16 turns across three phones, while the same
    /// path straightened and stripped of its trailing pause turned the page. Android reads
    /// the release velocity out of the recent motion history, so the pause describes a
    /// finger that had already stopped; and the bow is perpendicular to travel, which on a
    /// horizontal gesture means vertical — into the axis the feed's own pager watches.
    ///
    /// A phone that answers a drag by doing nothing is the whole failure: the counter reads
    /// the same number after the turn as before it, the loop concludes "no further image",
    /// and a photo post ends the session at two.
    #[derive(Default)]
    struct PagerPhone {
        /// Which image is showing. Zero means the counter has not appeared yet, which is
        /// what the feed does — the digits only materialise once paging starts.
        at: std::sync::atomic::AtomicU32,
        /// Gestures this pager refused to act on.
        drags: std::sync::atomic::AtomicUsize,
    }

    impl PagerPhone {
        const TOTAL: u32 = 7;

        fn node(text: &str) -> ElementBox {
            ElementBox {
                x: 940.0,
                y: 195.0,
                width: 30.0,
                height: 39.0,
                description: Some(text.to_string()),
                enabled: true,
                clickable: true,
            }
        }
    }

    #[async_trait::async_trait]
    impl UiSession for PagerPhone {
        async fn tap(&self, _point: TapPoint) -> anyhow::Result<()> {
            Ok(())
        }

        async fn swipe(&self, _gesture: crate::types::SwipeGesture) -> anyhow::Result<()> {
            Ok(())
        }

        async fn swipe_path(&self, path: crate::types::SwipePath) -> anyhow::Result<()> {
            let end = path.end();
            let (dx, dy) = (end.x - path.start.x, end.y - path.start.y);
            let span = dx * dx + dy * dy;
            let straight = span > f64::EPSILON
                && path.steps.iter().all(|step| {
                    let along = (((step.point.x - path.start.x) * dx
                        + (step.point.y - path.start.y) * dy)
                        / span)
                        .clamp(0.0, 1.0);
                    let (lx, ly) = (path.start.x + dx * along, path.start.y + dy * along);
                    ((step.point.x - lx).powi(2) + (step.point.y - ly).powi(2)).sqrt() < 0.5
                });
            if path.settle_ms == 0 && straight {
                // Measured: the counter arrives with the first turn already reading two.
                let at = self.at.load(Ordering::Relaxed);
                self.at
                    .store((at.max(1) + 1).min(Self::TOTAL), Ordering::Relaxed);
            } else {
                self.drags.fetch_add(1, Ordering::Relaxed);
            }
            Ok(())
        }

        async fn type_text(&self, _text: &str) -> anyhow::Result<()> {
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

        /// Only the badge, and only on `text` — where it actually lives.
        async fn locate(&self, query: ElementQuery<'_>) -> anyhow::Result<Option<ElementBox>> {
            match query {
                ElementQuery::Text { value: "Ảnh", .. } => Ok(Some(Self::node("Ảnh"))),
                _ => Ok(None),
            }
        }

        /// The counter, as three adjacent nodes, exactly as a phone renders it.
        async fn locate_all_described(
            &self,
            query: ElementQuery<'_>,
        ) -> anyhow::Result<Vec<ElementBox>> {
            let ElementQuery::ClassName(name) = query else {
                return Ok(Vec::new());
            };
            if name != "android.widget.TextView" {
                return Ok(Vec::new());
            }
            let at = self.at.load(Ordering::Relaxed);
            if at == 0 {
                return Ok(Vec::new());
            }
            Ok(vec![
                Self::node(&at.to_string()),
                Self::node(" / "),
                Self::node(&Self::TOTAL.to_string()),
            ])
        }
    }

    #[tokio::test]
    async fn a_photo_post_is_paged_with_a_flick_because_a_drag_turns_nothing() {
        let phone = PagerPhone::default();
        let screen = (1_080.0, 2_220.0);
        let mut run = HierarchyRun {
            session: &phone,
            labels: vietnamese(),
            screen,
            planner: TouchPointPlanner::new(screen),
            search_keyword: None,
        };
        let stop = AtomicBool::new(false);
        let mut status = NurtureSessionStatus {
            running: true,
            last_message: String::new(),
            ..NurtureSessionStatus::new("pager-phone")
        };
        let said = std::sync::Mutex::new(Vec::<String>::new());
        let report = |status: &mut NurtureSessionStatus, message: String| {
            status.last_message = message.clone();
            said.lock().expect("messages").push(message);
        };

        let seen = run
            .traverse_carousel(100, 20, &stop, &mut status, &report, None)
            .await;

        let said = said.lock().expect("messages").clone();
        assert_eq!(
            phone.drags.load(Ordering::Relaxed),
            0,
            "every sideways gesture the loop sends has to be one this pager acts on: {said:?}"
        );
        assert_eq!(
            seen,
            PagerPhone::TOTAL,
            "a seven-image post asked for in full is seven images: {said:?}"
        );
        assert!(
            said.iter().any(|line| line.contains("7/7")),
            "and the operator is told the post was finished, not abandoned: {said:?}"
        );
    }

    /// A sink that only counts, so the traversal can be asked what it offered.
    #[derive(Default)]
    struct SlideCounter {
        slides: std::sync::atomic::AtomicUsize,
        skips: std::sync::Mutex<Vec<String>>,
    }

    #[async_trait::async_trait]
    impl CommentTextSource for SlideCounter {
        async fn comment_for_post(
            &self,
            _settings: &NurtureSettings,
        ) -> Result<Option<PreparedComment>, CommentSourceError> {
            Ok(None)
        }

        async fn record_outcome(&self, _prepared: &PreparedComment, _outcome: &str) {
            panic!("a source that returns no prepared comment has no outcome to record");
        }

        fn note_slide(&self) {
            self.slides.fetch_add(1, Ordering::Relaxed);
        }

        async fn record_skip(&self, _settings: &NurtureSettings, reason: &str) {
            self.skips.lock().expect("skips").push(reason.to_string());
        }
    }

    /// **Every slide, including the ones whose counter never rendered.**
    ///
    /// The comment used to be written before this traversal began, so a seven-image post was
    /// commented on from image one — and because a still card publishes the same picture on
    /// every sample, from one picture sampled three times. The count here is what the sheet
    /// gets to choose from.
    #[tokio::test]
    async fn every_slide_of_a_photo_post_is_offered_as_evidence() {
        let phone = PagerPhone::default();
        let screen = (1_080.0, 2_220.0);
        let mut run = HierarchyRun {
            session: &phone,
            labels: vietnamese(),
            screen,
            planner: TouchPointPlanner::new(screen),
            search_keyword: None,
        };
        let stop = AtomicBool::new(false);
        let mut status = NurtureSessionStatus {
            running: true,
            last_message: String::new(),
            ..NurtureSessionStatus::new("pager-phone")
        };
        let report = |status: &mut NurtureSessionStatus, message: String| {
            status.last_message = message;
        };
        let sink = SlideCounter::default();

        let seen = run
            .traverse_carousel(100, 20, &stop, &mut status, &report, Some(&sink))
            .await;

        assert_eq!(seen, PagerPhone::TOTAL);
        // One per slide: the card it arrived on, then one per turn. Not `seen + 1` — slide one
        // is offered before the loop, and the loop offers one per turn it takes.
        assert_eq!(
            sink.slides.load(Ordering::Relaxed),
            PagerPhone::TOTAL as usize,
            "a seven-image post offers seven slides"
        );
    }

    /// A pager whose counter renders and never moves: TikTok ignored the flick.
    #[derive(Default)]
    struct StuckPagerPhone {
        drags: std::sync::atomic::AtomicUsize,
    }

    #[async_trait::async_trait]
    impl UiSession for StuckPagerPhone {
        async fn tap(&self, _point: TapPoint) -> anyhow::Result<()> {
            Ok(())
        }

        async fn swipe(&self, _gesture: crate::types::SwipeGesture) -> anyhow::Result<()> {
            Ok(())
        }

        async fn swipe_path(&self, _path: crate::types::SwipePath) -> anyhow::Result<()> {
            self.drags.fetch_add(1, Ordering::Relaxed);
            Ok(())
        }

        async fn type_text(&self, _text: &str) -> anyhow::Result<()> {
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

        async fn locate(&self, query: ElementQuery<'_>) -> anyhow::Result<Option<ElementBox>> {
            match query {
                ElementQuery::Text { value: "Ảnh", .. } => Ok(Some(PagerPhone::node("Ảnh"))),
                _ => Ok(None),
            }
        }

        /// Readable, and stuck on image two of seven no matter how often it is flicked.
        async fn locate_all_described(
            &self,
            query: ElementQuery<'_>,
        ) -> anyhow::Result<Vec<ElementBox>> {
            let ElementQuery::ClassName(name) = query else {
                return Ok(Vec::new());
            };
            if name != "android.widget.TextView" {
                return Ok(Vec::new());
            }
            Ok(vec![
                PagerPhone::node("2"),
                PagerPhone::node(" / "),
                PagerPhone::node(&PagerPhone::TOTAL.to_string()),
            ])
        }
    }

    /// A readable counter that does not move is **proof** the page did not turn, and the
    /// traversal must not spend that proof by having already offered the slide.
    #[tokio::test]
    async fn a_carousel_that_did_not_turn_offers_no_further_slide() {
        let phone = StuckPagerPhone::default();
        let screen = (1_080.0, 2_220.0);
        let mut run = HierarchyRun {
            session: &phone,
            labels: vietnamese(),
            screen,
            planner: TouchPointPlanner::new(screen),
            search_keyword: None,
        };
        let stop = AtomicBool::new(false);
        let mut status = NurtureSessionStatus {
            running: true,
            last_message: String::new(),
            ..NurtureSessionStatus::new("stuck-pager")
        };
        let report = |status: &mut NurtureSessionStatus, message: String| {
            status.last_message = message;
        };
        let sink = SlideCounter::default();

        let seen = run
            .traverse_carousel(100, 20, &stop, &mut status, &report, Some(&sink))
            .await;

        // Slide one arrived with the card and is offered before the loop. The turn did not
        // happen, and the counter says so, so nothing further is claimed: not another `seen`,
        // and not another image handed to the comment evidence. Offering it anyway published
        // comments as carousel-grounded over one duplicated picture.
        assert_eq!(seen, 1, "one image was actually observed");
        assert_eq!(
            sink.slides.load(Ordering::Relaxed),
            1,
            "a flick the counter says was ignored is not another slide"
        );
    }

    /// **The module header promises `HumanBehavior` is shared; this is the part that was not.**
    ///
    /// `think_pause_ms` (before the feed swipe) and `after_swipe_pause_ms` (after it) are both
    /// consumed by the pixel engine and were consumed by nothing here, so Android sessions went
    /// straight from the last action into the swipe and straight out of it into the next watch.
    /// The second one is worse than its own duration: it takes `&mut self` and increments
    /// `consecutive_swipes`, which is what arms the persona's long rest after `pause_after_swipes`
    /// swipes — so that setting was switched on in the UI and inert on the twenty phones that run.
    ///
    /// A source scan, because the values are randomised sleeps: what needs guarding is that the
    /// calls exist at all, which is what silently went missing.
    #[test]
    fn the_hierarchy_loop_consumes_the_same_human_pauses_as_the_pixel_loop() {
        let source = include_str!("hierarchy.rs").replace("\r\n", "\n");
        let production = source
            .split_once("\n#[cfg(test)]\nmod tests {")
            .map(|(before, _)| before)
            .unwrap_or(&source);
        // **Counted per site, not per file.** This loop swipes in two places -- the live-card
        // shortcut and the main feed step -- and two gates fell to the same arithmetic in
        // turn: presence stayed green with one site stripped, and then a whole-file COUNT
        // stayed green with both pauses doubled at one site and removed from the other. So
        // the assertion now stands at each swipe: within the lines around every
        // `.swipe_next(` call, both pause calls must appear.
        // `.swipe_next(` counts calls; a bare `swipe_next(` would also count the definition.
        let lines: Vec<&str> = production.lines().collect();
        let swipe_sites: Vec<usize> = lines
            .iter()
            .enumerate()
            .filter(|(_, line)| line.contains(".swipe_next("))
            .map(|(idx, _)| idx)
            .collect();
        assert!(
            swipe_sites.len() >= 2,
            "expected at least two swipe sites in the hierarchy loop, found {}",
            swipe_sites.len()
        );
        for site in swipe_sites {
            let from = site.saturating_sub(20);
            let to = (site + 21).min(lines.len());
            let window = lines[from..to].join("\n");
            for call in ["human.think_pause_ms()", "human.after_swipe_pause_ms()"] {
                assert!(
                    window.contains(call),
                    "the swipe at line {} has no {call} within twenty lines; the pixel loop \
                     consumes it at every swipe, and this module's header claims the pacing \
                     is shared",
                    site + 1
                );
            }
        }
        let pixel = include_str!("mod.rs").replace("\r\n", "\n");
        for call in ["human.think_pause_ms()", "human.after_swipe_pause_ms()"] {
            assert!(
                pixel.contains(call),
                "the pixel loop stopped calling {call}; this gate compares the two backends, so \
                 it has to fail rather than quietly compare nothing"
            );
        }
    }

    /// A card that reads as one post, plus a source that always has words ready. The card
    /// the phone shows differs from the one the caller says it grounded on, which is the
    /// state a feed advancing during a slow draft leaves behind.
    #[derive(Default)]
    struct DifferentCardPhone {
        taps: std::sync::atomic::AtomicUsize,
        type_texts: std::sync::atomic::AtomicUsize,
    }

    #[async_trait::async_trait]
    impl UiSession for DifferentCardPhone {
        async fn tap(&self, _point: TapPoint) -> anyhow::Result<()> {
            self.taps.fetch_add(1, Ordering::Relaxed);
            Ok(())
        }
        async fn swipe(&self, _gesture: crate::types::SwipeGesture) -> anyhow::Result<()> {
            Ok(())
        }
        async fn type_text(&self, _text: &str) -> anyhow::Result<()> {
            self.type_texts.fetch_add(1, Ordering::Relaxed);
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
        async fn restart_app(&self, _bundle_id: &str) -> anyhow::Result<()> {
            Ok(())
        }
        async fn locate(&self, query: ElementQuery<'_>) -> anyhow::Result<Option<ElementBox>> {
            let labels = vietnamese();
            let matches = |control| {
                labels
                    .label(control)
                    .is_some_and(|label| label.to_query() == query)
            };
            let card = |desc: &str| {
                Some(ElementBox {
                    x: 900.0,
                    y: 1_000.0,
                    width: 80.0,
                    height: 80.0,
                    description: Some(desc.to_string()),
                    enabled: true,
                    clickable: true,
                })
            };
            // A rail is present — so the card is not "gone", it is a *different* card. Its
            // author and comment count both differ from the one the draft was grounded on.
            Ok(if matches(TikTokControl::AuthorProfileLink) {
                card("Hồ sơ author_b")
            } else if matches(TikTokControl::Comments) {
                card("Xem hoặc thêm bình luận. 87 bình luận")
            } else if matches(TikTokControl::Share) {
                card("Chia sẻ video. 12")
            } else {
                None
            })
        }
    }

    /// A source that always hands back a real comment, so the only thing that can stop a
    /// send is the caller's own card check.
    struct AlwaysHasWords;

    #[async_trait::async_trait]
    impl CommentTextSource for AlwaysHasWords {
        async fn comment_for_post(
            &self,
            _settings: &NurtureSettings,
        ) -> Result<Option<PreparedComment>, CommentSourceError> {
            Ok(Some(PreparedComment {
                text: "một bình luận đã soạn".into(),
                prompt_tokens: 10,
                completion_tokens: 10,
                audit_attempt: CommentAuditToken::verified_fixture("một bình luận đã soạn"),
            }))
        }

        async fn record_outcome(&self, _prepared: &PreparedComment, _outcome: &str) {}
    }

    #[derive(Default)]
    struct RecordingWords {
        outcomes: std::sync::Mutex<Vec<String>>,
    }

    #[async_trait::async_trait]
    impl CommentTextSource for RecordingWords {
        async fn comment_for_post(
            &self,
            settings: &NurtureSettings,
        ) -> Result<Option<PreparedComment>, CommentSourceError> {
            AlwaysHasWords.comment_for_post(settings).await
        }

        async fn record_outcome(&self, _prepared: &PreparedComment, outcome: &str) {
            self.outcomes
                .lock()
                .expect("recorded outcome lock")
                .push(outcome.to_string());
        }
    }

    struct AuditUnavailableSource;

    #[async_trait::async_trait]
    impl CommentTextSource for AuditUnavailableSource {
        async fn comment_for_post(
            &self,
            _settings: &NurtureSettings,
        ) -> Result<Option<PreparedComment>, CommentSourceError> {
            Err(CommentSourceError::AuditUnavailable)
        }

        async fn record_outcome(&self, _prepared: &PreparedComment, _outcome: &str) {
            panic!("an unavailable audit source cannot have an outcome");
        }
    }

    struct InvalidAuditWords(&'static str);

    #[async_trait::async_trait]
    impl CommentTextSource for InvalidAuditWords {
        async fn comment_for_post(
            &self,
            _settings: &NurtureSettings,
        ) -> Result<Option<PreparedComment>, CommentSourceError> {
            Ok(Some(PreparedComment {
                text: "không được chạm UI".into(),
                prompt_tokens: 0,
                completion_tokens: 0,
                audit_attempt: CommentAuditToken::unverified_fixture(self.0, "không được chạm UI"),
            }))
        }

        async fn record_outcome(&self, _prepared: &PreparedComment, _outcome: &str) {
            panic!("an unverified audit attempt must be rejected before outcome handling");
        }
    }

    struct MismatchedAuditWords;

    #[async_trait::async_trait]
    impl CommentTextSource for MismatchedAuditWords {
        async fn comment_for_post(
            &self,
            _settings: &NurtureSettings,
        ) -> Result<Option<PreparedComment>, CommentSourceError> {
            Ok(Some(PreparedComment {
                text: "payload đã bị thay đổi".into(),
                prompt_tokens: 0,
                completion_tokens: 0,
                audit_attempt: CommentAuditToken::verified_fixture("payload đã ghi bền"),
            }))
        }

        async fn record_outcome(&self, _prepared: &PreparedComment, _outcome: &str) {
            panic!("a mismatched audit token must be rejected before outcome handling");
        }
    }

    struct ReplayedAuditWords;

    #[async_trait::async_trait]
    impl CommentTextSource for ReplayedAuditWords {
        async fn comment_for_post(
            &self,
            _settings: &NurtureSettings,
        ) -> Result<Option<PreparedComment>, CommentSourceError> {
            let text = "payload chỉ được dùng một lần";
            let mut token = CommentAuditToken::verified_fixture(text);
            assert!(token.consume_for(text));
            Ok(Some(PreparedComment {
                text: text.into(),
                prompt_tokens: 0,
                completion_tokens: 0,
                audit_attempt: token,
            }))
        }

        async fn record_outcome(&self, _prepared: &PreparedComment, _outcome: &str) {
            panic!("a replayed audit token must be rejected before outcome handling");
        }
    }

    fn audit_attempt_row(id: &str) -> NurtureCommentAttempt {
        NurtureCommentAttempt {
            id: id.into(),
            udid: "audit-token-test".into(),
            outcome: "prepared".into(),
            source: "test".into(),
            model: "none".into(),
            base_url_host: "local".into(),
            prompt_tokens: 0,
            completion_tokens: 0,
            cost_usd: None,
            preview: "fixture".into(),
            caption_preview: String::new(),
            frame_sha256: String::new(),
            context_confidence: None,
            relevance: None,
            evidence_support: None,
            distinct_frames: None,
            carousel_slides: None,
            created_at: chrono::Utc::now().to_rfc3339(),
        }
    }

    #[test]
    fn comment_audit_token_exists_only_after_a_valid_row_is_persisted() {
        let path =
            std::env::temp_dir().join(format!("riviu-comment-audit-token-{}.db", Uuid::new_v4()));
        let database = Database::open(&path).expect("open audit fixture");

        for invalid in ["", "fabricated-not-persisted"] {
            assert!(CommentAuditToken::persist(&database, &audit_attempt_row(invalid)).is_err());
        }
        assert!(database
            .list_nurture_comment_attempts(10)
            .expect("list after invalid attempts")
            .is_empty());

        let valid_id = Uuid::new_v4().to_string();
        let mut attempt = audit_attempt_row(&valid_id);
        attempt.preview = "payload đã ghi bền".into();
        let mut token =
            CommentAuditToken::persist_for_text(&database, &attempt, "payload đã ghi bền")
                .expect("persisted row produces token");
        assert_eq!(token.id(), valid_id);
        assert!(token.is_verified());
        assert!(token.consume_for("payload đã ghi bền"));
        assert!(
            !token.consume_for("payload đã ghi bền"),
            "a persisted capability is one-shot"
        );
        assert_eq!(
            database
                .list_nurture_comment_attempts(10)
                .expect("list persisted attempt")
                .len(),
            1
        );

        drop(database);
        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{}", path.display(), suffix));
        }
    }

    fn phase_card() -> PostFingerprint {
        PostFingerprint {
            author: Some("Hồ sơ author_a".into()),
            comments: None,
            share: Some("Chia sẻ video. 3".into()),
            sound: None,
        }
    }

    fn fully_read_phase_card() -> PostFingerprint {
        PostFingerprint {
            author: Some("Hồ sơ author_a".into()),
            comments: Some("12 bình luận".into()),
            share: Some("Chia sẻ video. 3".into()),
            sound: None,
        }
    }

    async fn assert_invalid_audit_attempt_is_refused(attempt_id: &'static str) {
        assert_audit_source_is_refused(&InvalidAuditWords(attempt_id)).await;
    }

    async fn assert_audit_source_is_refused(source: &dyn CommentTextSource) {
        let phone = PhaseFailurePhone {
            fail_locate: None,
            comments_absent: false,
            tap_fails: false,
            taps: 0.into(),
            typed: 0.into(),
        };
        let mut run = phase_failure_run(&phone);
        let mut status = NurtureSessionStatus::new("invalid-audit-token");
        let report = |status: &mut NurtureSessionStatus, message: String| {
            status.last_message = message;
        };

        let outcome = post_rolled_comment(
            &mut run,
            source,
            &NurtureSettings::default(),
            &fully_read_phase_card(),
            &AtomicBool::new(false),
            &mut status,
            &report,
        )
        .await;

        assert!(!outcome.sent);
        assert!(!outcome.did_act);
        assert_eq!(phone.taps.load(Ordering::Relaxed), 0);
        assert_eq!(phone.typed.load(Ordering::Relaxed), 0);
        assert!(status.last_message.contains("audit"));
    }

    #[tokio::test]
    async fn hierarchy_rejects_an_empty_audit_attempt_before_comment_ui() {
        assert_invalid_audit_attempt_is_refused("").await;
    }

    #[tokio::test]
    async fn hierarchy_rejects_a_fabricated_audit_attempt_before_comment_ui() {
        assert_invalid_audit_attempt_is_refused("fabricated-not-persisted").await;
    }

    #[tokio::test]
    async fn hierarchy_rejects_a_text_mismatch_before_comment_ui() {
        assert_audit_source_is_refused(&MismatchedAuditWords).await;
    }

    #[tokio::test]
    async fn hierarchy_rejects_an_audit_token_replay_before_comment_ui() {
        assert_audit_source_is_refused(&ReplayedAuditWords).await;
    }

    #[tokio::test]
    async fn rolled_comment_opener_failure_refunds_attempt_and_cap() {
        let phone = PhaseFailurePhone {
            fail_locate: Some(TikTokControl::Comments),
            comments_absent: false,
            tap_fails: false,
            taps: 0.into(),
            typed: 0.into(),
        };
        let mut run = phase_failure_run(&phone);
        let mut status = NurtureSessionStatus::new("comment-before-effect");
        let report = |status: &mut NurtureSessionStatus, message: String| {
            status.last_message = message;
        };
        let outcome = post_rolled_comment(
            &mut run,
            &AlwaysHasWords,
            &NurtureSettings::default(),
            &phase_card(),
            &AtomicBool::new(false),
            &mut status,
            &report,
        )
        .await;
        let mut policy = one_attempt_policy(PolicyAction::Comment);
        let reservation = policy.reserve_attempt(PolicyAction::Comment);

        assert!(!settle_attempt(
            &mut policy,
            reservation,
            outcome.did_act,
            &mut status.comment_attempts,
        ));
        assert_eq!(phone.taps.load(Ordering::Relaxed), 0);
        assert_eq!(phone.typed.load(Ordering::Relaxed), 0);
        assert_eq!(status.comment_attempts, 0);
        assert!(policy.can_attempt(PolicyAction::Comment));
        assert!(policy.can_interact_with_post());
    }

    #[tokio::test]
    async fn rolled_comment_without_a_control_refunds_attempt_and_cap() {
        let phone = PhaseFailurePhone {
            fail_locate: None,
            comments_absent: true,
            tap_fails: false,
            taps: 0.into(),
            typed: 0.into(),
        };
        let mut run = phase_failure_run(&phone);
        let mut status = NurtureSessionStatus::new("comment-no-control");
        let report = |status: &mut NurtureSessionStatus, message: String| {
            status.last_message = message;
        };
        let outcome = post_rolled_comment(
            &mut run,
            &AlwaysHasWords,
            &NurtureSettings::default(),
            &phase_card(),
            &AtomicBool::new(false),
            &mut status,
            &report,
        )
        .await;
        let mut policy = one_attempt_policy(PolicyAction::Comment);
        let reservation = policy.reserve_attempt(PolicyAction::Comment);

        assert!(!settle_attempt(
            &mut policy,
            reservation,
            outcome.did_act,
            &mut status.comment_attempts,
        ));
        assert_eq!(phone.taps.load(Ordering::Relaxed), 0);
        assert_eq!(phone.typed.load(Ordering::Relaxed), 0);
        assert_eq!(status.comment_attempts, 0);
        assert!(policy.can_attempt(PolicyAction::Comment));
        assert!(policy.can_interact_with_post());
    }

    /// **A comment grounded on card A is never typed into card B.** Drafting takes seconds and
    /// the feed can advance under it; typing the words anyway posts them, publicly and
    /// undoably, on whatever card arrived. The guard re-reads the card after the draft and
    /// declines when it does not match — here the phone shows a card whose comment count is
    /// not the one we grounded on, and no `type_text` may go out.
    #[tokio::test]
    async fn a_comment_is_not_typed_when_the_card_changed_during_drafting() {
        let phone = DifferentCardPhone::default();
        let source = RecordingWords::default();
        let screen = (1_080.0, 2_220.0);
        let mut run = HierarchyRun {
            session: &phone,
            labels: vietnamese(),
            screen,
            planner: TouchPointPlanner::new(screen),
            search_keyword: None,
        };
        let settings = NurtureSettings::default();
        let stop = AtomicBool::new(false);
        let mut status = NurtureSessionStatus {
            running: true,
            last_message: String::new(),
            ..NurtureSessionStatus::new("card-changed")
        };
        let report = |status: &mut NurtureSessionStatus, message: String| {
            status.last_message = message;
        };
        // The card the draft was grounded on: a different comment count from what the phone
        // now shows.
        let grounded_on = PostFingerprint {
            author: Some("Hồ sơ author_a".into()),
            comments: Some("Xem hoặc thêm bình luận. 3 bình luận".into()),
            share: None,
            sound: None,
        };
        let outcome = post_rolled_comment(
            &mut run,
            &source,
            &settings,
            &grounded_on,
            &stop,
            &mut status,
            &report,
        )
        .await;
        assert!(
            !outcome.sent,
            "a changed card must not be reported as commented"
        );
        assert!(
            !outcome.did_act,
            "a changed card must refund its no-effect reservation"
        );
        assert_eq!(
            phone.taps.load(Ordering::Relaxed),
            0,
            "no control may be tapped on a card we did not ground on"
        );
        assert_eq!(
            phone.type_texts.load(Ordering::Relaxed),
            0,
            "not one character may be typed into a card we did not ground on: {}",
            status.last_message
        );
        assert!(
            status.last_message.contains("thẻ đã đổi"),
            "the operator is told why the comment was withheld: {}",
            status.last_message
        );
        assert_eq!(
            source
                .outcomes
                .lock()
                .expect("recorded outcome lock")
                .as_slice(),
            ["skipped: card_changed"],
            "the durable attempt must explain why no UI action followed"
        );
    }

    #[test]
    fn public_hierarchy_entry_applies_effective_settings_before_prepare_and_feed() {
        let source = include_str!("hierarchy.rs").replace("\r\n", "\n");
        let entry = source
            .split("pub async fn run_hierarchy_session(")
            .nth(1)
            .expect("public hierarchy entry")
            .split("\n}\n\n/// How long the feed gets to appear")
            .next()
            .expect("public hierarchy entry body");
        let effective = entry
            .find("settings.clone().into_effective()")
            .expect("entrypoint must fold disabled public actions");
        let prepare = entry
            .find("HierarchyRun::prepare")
            .expect("hierarchy prepare call");
        let feed = entry.find("run_feed(").expect("hierarchy feed call");

        assert!(effective < prepare);
        assert!(effective < feed);
    }

    #[tokio::test]
    async fn audit_unavailable_is_typed_and_never_reaches_comment_ui() {
        let phone = DifferentCardPhone::default();
        let screen = (1_080.0, 2_220.0);
        let mut run = HierarchyRun {
            session: &phone,
            labels: vietnamese(),
            screen,
            planner: TouchPointPlanner::new(screen),
            search_keyword: None,
        };
        let settings = NurtureSettings::default();
        let stop = AtomicBool::new(false);
        let mut status = NurtureSessionStatus {
            running: true,
            last_message: String::new(),
            ..NurtureSessionStatus::new("audit-unavailable")
        };
        let report = |status: &mut NurtureSessionStatus, message: String| {
            status.last_message = message;
        };

        let outcome = post_rolled_comment(
            &mut run,
            &AuditUnavailableSource,
            &settings,
            &PostFingerprint::default(),
            &stop,
            &mut status,
            &report,
        )
        .await;

        assert!(!outcome.sent);
        assert!(!outcome.did_act);
        assert_eq!(status.comment_attempts, 0);
        assert_eq!(phone.type_texts.load(Ordering::Relaxed), 0);
        assert!(status.last_message.contains("audit"));
    }

    /// A sink that hands back a fixed comment and remembers when it was asked.
    struct OrderedComment {
        slides_when_asked: std::sync::Mutex<Option<usize>>,
        slides: std::sync::atomic::AtomicUsize,
    }

    #[async_trait::async_trait]
    impl CommentTextSource for OrderedComment {
        async fn comment_for_post(
            &self,
            _settings: &NurtureSettings,
        ) -> Result<Option<PreparedComment>, CommentSourceError> {
            *self.slides_when_asked.lock().expect("asked") =
                Some(self.slides.load(Ordering::Relaxed));
            Ok(None)
        }

        async fn record_outcome(&self, _prepared: &PreparedComment, _outcome: &str) {
            panic!("a source that returns no prepared comment has no outcome to record");
        }

        fn note_slide(&self) {
            self.slides.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// The property the whole reorder exists for: **the words are written after the pictures**.
    ///
    /// Driven through `traverse_carousel` plus the guard rather than through `run_feed`, because
    /// the guard is where the decision lives and `run_feed` needs a phone that can also serve a
    /// feed, a drawer and a rail. What this pins is the ordering: at the moment the source is
    /// asked for words, it has already been offered every slide.
    #[tokio::test]
    async fn the_words_are_written_after_the_slides_not_before_them() {
        let phone = PagerPhone::default();
        let screen = (1_080.0, 2_220.0);
        let mut run = HierarchyRun {
            session: &phone,
            labels: vietnamese(),
            screen,
            planner: TouchPointPlanner::new(screen),
            search_keyword: None,
        };
        let stop = AtomicBool::new(false);
        let mut status = NurtureSessionStatus {
            running: true,
            last_message: String::new(),
            ..NurtureSessionStatus::new("pager-phone")
        };
        let report = |status: &mut NurtureSessionStatus, message: String| {
            status.last_message = message;
        };
        let source = OrderedComment {
            slides_when_asked: std::sync::Mutex::new(None),
            slides: std::sync::atomic::AtomicUsize::new(0),
        };

        run.traverse_carousel(100, 20, &stop, &mut status, &report, Some(&source))
            .await;
        let settings = NurtureSettings::default();
        let grounded_on = fingerprint(run.session, run.labels).await;
        post_rolled_comment(
            &mut run,
            &source,
            &settings,
            &grounded_on,
            &stop,
            &mut status,
            &report,
        )
        .await;

        assert_eq!(
            *source.slides_when_asked.lock().expect("asked"),
            Some(PagerPhone::TOTAL as usize),
            "the source must have seen every slide before it was asked for words"
        );
    }

    /// A traversal a caller has no comment for must behave exactly as it did.
    #[tokio::test]
    async fn a_post_nobody_is_commenting_on_offers_nothing() {
        let phone = PagerPhone::default();
        let screen = (1_080.0, 2_220.0);
        let mut run = HierarchyRun {
            session: &phone,
            labels: vietnamese(),
            screen,
            planner: TouchPointPlanner::new(screen),
            search_keyword: None,
        };
        let stop = AtomicBool::new(false);
        let mut status = NurtureSessionStatus {
            running: true,
            last_message: String::new(),
            ..NurtureSessionStatus::new("pager-phone")
        };
        let report = |status: &mut NurtureSessionStatus, message: String| {
            status.last_message = message;
        };
        let sink = SlideCounter::default();

        // `None`, which is what the roll site passes on a post it did not defer a comment for.
        let seen = run
            .traverse_carousel(100, 20, &stop, &mut status, &report, None)
            .await;

        assert_eq!(seen, PagerPhone::TOTAL);
        assert_eq!(
            sink.slides.load(Ordering::Relaxed),
            0,
            "the sink was not handed over, so nothing may reach it"
        );
    }

    /// A post whose first image is all the operator asked for still hands that image over.
    ///
    /// The early return used to leave with the sink empty, which would have left a deferred
    /// comment with no evidence at all and skipped the post for a reason nobody chose.
    #[tokio::test]
    async fn the_one_slide_case_still_offers_its_one_slide() {
        let phone = PagerPhone::default();
        // Start with the counter already showing, which is what a post page does — that is the
        // surface where `wanted <= 1` is reachable at all.
        phone.at.store(1, Ordering::Relaxed);
        let screen = (1_080.0, 2_220.0);
        let mut run = HierarchyRun {
            session: &phone,
            labels: vietnamese(),
            screen,
            planner: TouchPointPlanner::new(screen),
            search_keyword: None,
        };
        let stop = AtomicBool::new(false);
        let mut status = NurtureSessionStatus {
            running: true,
            last_message: String::new(),
            ..NurtureSessionStatus::new("pager-phone")
        };
        let report = |status: &mut NurtureSessionStatus, message: String| {
            status.last_message = message;
        };
        let sink = SlideCounter::default();

        // 1% of seven images rounds to one, so this takes the early return.
        let seen = run
            .traverse_carousel(1, 20, &stop, &mut status, &report, Some(&sink))
            .await;

        assert_eq!(seen, 1);
        assert_eq!(
            sink.slides.load(Ordering::Relaxed),
            1,
            "the one image the operator asked for is still evidence"
        );
        assert_eq!(phone.at.load(Ordering::Relaxed), 1, "and nothing was paged");
    }

    #[tokio::test(start_paused = true)]
    async fn a_phone_two_screens_deep_is_backed_out_of_both_of_them() {
        // Measured 19/08/2026 on ce0417145199e0490c: a whole-fleet run lost one phone, and it
        // was on TikTok's `SearchResultActivity` — left there by whatever ran last, not by a
        // dialog. Search results are two screens deep, so the single Back this rung used to
        // allow reached the search page, which has no feed tab and no Home tab either, and
        // the session ended at zero videos.
        let phone = UnlabelledModalPhone {
            depth: std::sync::atomic::AtomicUsize::new(2),
            ..Default::default()
        };
        let screen = (1_080.0, 2_220.0);
        let run = HierarchyRun {
            session: &phone,
            labels: controls_for("com.ss.android.ugc.trill", "en", "38.3.2").expect("measured set"),
            screen,
            planner: TouchPointPlanner::new(screen),
            search_keyword: None,
        };
        let stop = AtomicBool::new(false);
        let mut status = NurtureSessionStatus {
            running: true,
            last_message: String::new(),
            ..NurtureSessionStatus::new("deep-stack")
        };
        let report = |status: &mut NurtureSessionStatus, message: String| {
            status.last_message = message;
        };

        assert!(
            await_feed(&run, &stop, &mut status, &report).await,
            "two screens deep is still a way back to the feed"
        );
        assert_eq!(
            phone.backs.load(Ordering::Relaxed),
            2,
            "exactly as many as it took — the loop re-judges after each one, so the moment \
             the feed appears the rest are never spent"
        );
    }
}

#[cfg(test)]
mod carousel_capability_tests {
    use super::*;
    use crate::tiktok_labels::controls_for;

    /// The three phones this check exists for, and the eleven it must stay quiet about.
    ///
    /// Measured 23/08/2026: the fleet ran eleven `trill` 38.3.2 phones that could page
    /// carousels and three `musically` 46.2.1 that could not, and nothing anywhere said so —
    /// `traverse_carousel` returned 0 without a word while the panel's "Bài ảnh" group
    /// promised the feature. The badge has since been measured on `musically` too, which is
    /// exactly why this test asserts the *rule* against the set that refuses everything: the
    /// gap will come back with the next unmeasured build, and it must announce itself then.
    #[test]
    fn a_build_without_a_measured_badge_is_known_to_be_unable_to_page() {
        assert!(
            !can_page_carousel(crate::tiktok_labels::nothing_measured()),
            "an unmeasured badge means this build cannot page carousels"
        );
        for (package, language) in [
            ("com.ss.android.ugc.trill", "en"),
            ("com.ss.android.ugc.trill", "vi"),
            ("com.zhiliaoapp.musically", "en"),
        ] {
            let labels = controls_for(package, language, "").expect("measured set");
            assert!(
                can_page_carousel(labels),
                "{package}/{language} has a measured badge, so it must not be warned about"
            );
        }
    }

    /// The check must cost nothing: it reads the label table, never the device.
    ///
    /// Stated as a test because the reason the missing-carousel case was silent in the first
    /// place is that nobody wanted to pay a device round trip per session to announce it.
    #[test]
    fn the_check_is_a_table_read_and_needs_no_session() {
        // No `UiSession` argument exists to pass, which is the property itself: this cannot
        // touch a phone even by accident.
        let labels = controls_for("com.zhiliaoapp.musically", "en", "").expect("measured set");
        assert!(can_page_carousel(labels));
    }
}
