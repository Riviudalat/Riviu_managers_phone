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
//! * an app build / UI language with no measured label set — [`controls_for`]
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
use super::live::{apply_live_settings, video_target, LiveSettings};
use crate::driver::{ElementBox, ElementQuery, UiSession};
use crate::human_behavior::{
    in_night_window, roll_bool, roll_feed_action_in_mood, roll_follow_in_mood, FeedAction,
    HumanBehavior, HumanSessionPolicy, MoodCycle, PolicyAction,
};
use crate::tiktok_drawer::CommentVerdict;
use crate::tiktok_labels::{controls_for, TikTokControl, TikTokControls};
use crate::tiktok_like::LikeVerdict;
use crate::types::{NurtureSessionStatus, NurtureSettings, TapPoint};

use super::recovery::Outcome;
use super::sleep_interruptible;
use super::touch::TouchPointPlanner;

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
/// The feed loop runs exactly `num_videos` passes, so it makes at most that many swipe
/// attempts: a session asked for two videos can never reach a counter of four. Measured
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

/// How many sideways turns in a row may go unproved before the traversal stops.
///
/// The page counter is a transient overlay, so an unreadable counter means "could not tell"
/// rather than "the post ended" — treating the two the same left a 15-image post at one
/// image (measured 12/08/2026). But an unproved turn is still a gesture on a card that may
/// have nothing left to show, so the stretch is bounded: three is enough to ride out an
/// overlay that fades between the swipe and the read, and short enough that a
/// single-image post costs three gestures rather than the whole ceiling.
const CAROUSEL_UNPROVEN_LIMIT: u32 = 3;

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
}

/// A comment the caller has already decided to post, and what it cost.
#[derive(Debug, Clone, PartialEq)]
pub struct PreparedComment {
    pub text: String,
    /// Provider spend for generating it, added to the session total.
    pub usd: f64,
    /// Opaque id for the caller's own audit row, echoed back to
    /// [`CommentTextSource::record_outcome`].
    pub attempt_id: Option<String>,
}

/// Where this loop gets the words for a comment.
///
/// A trait rather than the AI call itself, for two reasons. It keeps provider
/// plumbing — frames, OpenAI, pricing, audit rows — out of a module whose job is
/// driving a hierarchy; and it lets the G2 probe exercise the whole drawer flow
/// with a fixed string, no database or control plane behind it. The desktop app
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
    async fn comment_for_post(&self, settings: &NurtureSettings) -> Option<PreparedComment>;

    /// Record how the attempt ended. Default does nothing, for callers with no
    /// audit trail to keep.
    async fn record_outcome(&self, _prepared: &PreparedComment, _outcome: &str) {}
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
    session.locate(label.to_query()).await
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

async fn fingerprint(session: &dyn UiSession, labels: TikTokControls) -> PostFingerprint {
    let read = |control| async move {
        locate(session, labels, control)
            .await
            .ok()
            .flatten()
            .and_then(|element| element.description)
    };
    PostFingerprint {
        comments: read(TikTokControl::Comments).await,
        share: read(TikTokControl::Share).await,
        sound: read(TikTokControl::SoundLink).await,
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
        let package = session
            .active_app_bundle()
            .await
            .unwrap_or_else(|_| settings.bundle_id.clone());
        let language = session.ui_language().await;
        // Read once per session, not per locate: `dumpsys package` measured 1–2 s on
        // this fleet. An unread version is not fatal — it leaves only the resource-id
        // controls absent, so this loop can still like and read.
        let app_version = session.app_version(&package).await.unwrap_or_default();
        let labels = language
            .as_deref()
            .and_then(|language| controls_for(&package, language, &app_version))
            .ok_or_else(|| Unsupported::NoLabelSet {
                package: package.clone(),
                language: language.clone(),
            })?;
        Ok(Self {
            session,
            labels,
            screen,
            planner: TouchPointPlanner::new(screen),
        })
    }

    /// Provenance of the label set in use, for the session log.
    pub(super) fn labels_note(&self) -> String {
        format!("nhãn đã đo: {}", self.labels.provenance())
    }

    /// True when the feed tab is on screen — the only condition under which this
    /// loop will tap anything.
    async fn on_feed(&self) -> bool {
        present(self.session, self.labels, TikTokControl::FeedTab).await
    }

    /// Tap a jittered point inside a located control.
    async fn tap_inside(&mut self, element: &ElementBox) -> anyhow::Result<()> {
        let point = self.planner.next(element.centre(), element.jitter_radius());
        self.session.tap(point).await
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
    async fn like(&mut self, stop: &AtomicBool) -> anyhow::Result<LikeVerdict> {
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
    async fn comment(&mut self, text: &str, stop: &AtomicBool) -> anyhow::Result<CommentVerdict> {
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
        crate::tiktok_drawer::post_comment(self.session, self.labels, plan, text, stop).await
    }

    /// Follow the author, proved by the Follow control leaving the card.
    async fn follow(&mut self, stop: &AtomicBool) -> anyhow::Result<bool> {
        let Some(element) = locate(self.session, self.labels, TikTokControl::Follow).await? else {
            return Ok(false);
        };
        self.tap_inside(&element).await?;
        sleep_interruptible(Duration::from_millis(1_200), stop).await;
        // Following removes the button; its continued presence means the tap did
        // not take, and reporting that as a follow would inflate every count.
        Ok(locate(self.session, self.labels, TikTokControl::Follow)
            .await?
            .is_none())
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
    /// * When the digits never materialise, the traversal falls back to "keep turning until
    ///   a turn changes nothing", bounded by the ceiling. Safe, because the gate has already
    ///   established this is a photo post — just less precise, since a fraction of an
    ///   unknown total is not a fraction.
    ///
    /// Every turn is **proved** by the counter advancing, or in the fallback by the tree
    /// changing. A turn that proves nothing ends the traversal instead of being counted, the
    /// same rule [`Self::swipe_next`] applies to the feed.
    ///
    /// Stops at `current == total` rather than turning until the counter disappears. Both end
    /// the loop, but measured behaviour is that the counter vanishes only *after* the last
    /// image, so waiting for that spends one gesture past the end of the post.
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
                return 1;
            }
        }
        report(status, "gặp bài ảnh — vuốt ngang".into());

        let mut seen = 1u32;
        // Consecutive turns whose effect could not be read. Reset by any readable counter.
        let mut unproven = 0u32;
        let mut at = known_before.map(|(current, _)| current);
        let mut total = known_before.map(|(_, total)| total);
        // Recomputed as soon as the total is known, which on the feed is after the first
        // turn. Until then the ceiling is the only bound there is.
        let mut wanted = total.map_or(ceiling.max(1), |t| {
            carousel_target(t, portion_percent, ceiling)
        });

        while seen < wanted {
            if stop.load(Ordering::Relaxed) {
                break;
            }
            if let Err(error) = self.swipe_slide(stop).await {
                report(status, format!("vuốt ngang lỗi: {error}"));
                break;
            }
            match self.carousel_position().await {
                Some((now, post_total)) => {
                    unproven = 0;
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
                }
                // No digits. **Not the same as the end of the post**, and reading it that way
                // cost real coverage: a 15-image post was left at `1/15` because the
                // indicator faded between the swipe and the read (measured 12/08/2026). The
                // badge has already established this is a photo post, so the honest response
                // is to keep turning for a bounded stretch rather than to stop or to swipe
                // forever.
                //
                // **And there is nothing better to reach for, which is measured rather than
                // assumed.** The obvious idea is to fingerprint the tree instead — if the
                // page turned, something changed. It did not: on 38.3.2, across turns that
                // provably advanced the counter, the `ImageView` rectangles and the
                // `TextView` list were **identical** before and after (`probe
                // --measure-feed-carousel`, 18/08/2026). The counter is the only thing in the
                // hierarchy that can see a page turn, so on a post that never renders one,
                // four turns is the most this can honestly claim. On a whole-fleet run that
                // was 6 of 10 photo posts one time and 2 of 13 another — it is feed content,
                // not a phone or a build.
                None => {
                    unproven += 1;
                    if unproven > CAROUSEL_UNPROVEN_LIMIT {
                        break;
                    }
                    seen += 1;
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

    /// Swipe to the next post and report whether the change was provable.
    async fn swipe_next(
        &mut self,
        duration_ms: u64,
        before: &PostFingerprint,
        stop: &AtomicBool,
    ) -> anyhow::Result<bool> {
        // A vertical flick through the middle of the screen, clear of the rail on
        // the right and of the system gesture strip at the very bottom.
        //
        // These two are **targets**, not coordinates: `plan_swipe` moves the ends, bows the
        // path and shapes the velocity, so no two feed swipes are the same gesture. Before
        // that, every swipe in every session was the identical dead-straight line between
        // the identical two pixels, traversed at a constant speed — see `super::touch`.
        let from = TapPoint {
            x: self.screen.0 * 0.5,
            y: self.screen.1 * 0.72,
        };
        let to = TapPoint {
            x: self.screen.0 * 0.5,
            y: self.screen.1 * 0.28,
        };
        let path = self.planner.plan_swipe(from, to, duration_ms);
        self.session.swipe_path(path).await?;
        sleep_interruptible(SWIPE_SETTLE, stop).await;
        // Wait for the incoming card's rail rather than reading once and hoping.
        // A card genuinely without a rail (LIVE, a photo carousel mid-transition)
        // simply uses up the window, which costs a few seconds once and is the
        // price of not miscounting every ordinary swipe that lands slowly.
        let deadline = Instant::now() + RAIL_RETURN_WINDOW;
        loop {
            let after = fingerprint(self.session, self.labels).await;
            if !after.is_empty() {
                return Ok(after != *before);
            }
            if Instant::now() >= deadline || stop.load(Ordering::Relaxed) {
                // Still nothing to fingerprint. That proves nothing either way, so
                // it is not counted as a video — the same refusal the pixel engine
                // makes when a swipe only moves the screen.
                return Ok(false);
            }
            sleep_interruptible(RAIL_RETURN_POLL, stop).await;
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
    if let Ok(package) = session.active_app_bundle().await {
        if is_measured_tiktok(&package) {
            return Ok(false);
        }
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
    session.launch_app_foreground(bundle_id).await?;
    let deadline = Instant::now() + FOREGROUND_WINDOW;
    while Instant::now() < deadline {
        sleep_interruptible(FOREGROUND_POLL, stop).await;
        if stop.load(Ordering::Relaxed) {
            break;
        }
        if let Ok(package) = session.active_app_bundle().await {
            if is_measured_tiktok(&package) {
                return Ok(true);
            }
        }
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
    if !session.supports_element_bounds() {
        return HierarchySession::NotSupported;
    }
    match ensure_tiktok_foreground(session, bundle_id, stop).await {
        Ok(true) => report(status, "đã đưa TikTok lên foreground".into()),
        Ok(false) => {}
        Err(error) => {
            report(status, format!("failed — {error}"));
            return HierarchySession::Refused;
        }
    }
    let run = match HierarchyRun::prepare(session, screen, settings).await {
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
    await_first_rail(&run, stop).await;
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
async fn await_first_rail(run: &HierarchyRun<'_>, stop: &AtomicBool) {
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
        if attempt == 1 {
            let Some(element) = locate(run.session, run.labels, TikTokControl::FeedTab)
                .await
                .ok()
                .flatten()
            else {
                return;
            };
            if run.session.tap(element.centre()).await.is_err() {
                return;
            }
            sleep_interruptible(SWIPE_SETTLE, stop).await;
        }
        let deadline = Instant::now() + RAIL_RETURN_WINDOW;
        loop {
            if stop.load(Ordering::Relaxed) {
                return;
            }
            if !fingerprint(run.session, run.labels).await.is_empty() {
                return;
            }
            if Instant::now() >= deadline {
                break;
            }
            sleep_interruptible(RAIL_RETURN_POLL, stop).await;
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
    let started = Instant::now();
    let deadline = started + FEED_READY_WINDOW;
    let mut said = false;
    let mut nudged = false;
    let mut backed = false;
    loop {
        if stop.load(Ordering::Relaxed) {
            return false;
        }
        if run.on_feed().await {
            if said {
                report(status, "feed đã lên".into());
            }
            return true;
        }

        // **The window is checked here so every branch below sits inside it.**
        //
        // It used to sit at the bottom, past the recovery branches, which was safe only
        // while each of those could fire once: `nudged` guards the Home tap, `backed`
        // guards the Back press. The dialog decline deliberately has no such guard —
        // TikTok stacks prompts and each one hides the next — so a decline label that
        // stays on screen (a tap that misses, a screen that genuinely carries the word)
        // put this loop in a tap-sleep-continue cycle that never reached the check at
        // all. A session promising to give up after thirty seconds instead ran until
        // something else stopped it.
        //
        // Above the branches but below `on_feed`, so a feed arriving on the last poll
        // still counts. That costs one cheap read and is the answer the operator wanted.
        if Instant::now() >= deadline {
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
        // **A modal owns the whole tree, so nothing else on this screen is findable.**
        // Measured 18/08/2026: a phone held behind "Save login for next time?" dumped a
        // single `content-desc` of `Dialog`, which is why neither the feed tab nor the Home
        // tab could be seen and the session could only report that it never saw a feed.
        //
        // Declining is safe without knowing which dialog it is — `Not now` changes no
        // setting and no account, and the dialog comes back next time. Cleared on every
        // poll rather than once: TikTok stacks these, and each one hides the next.
        if let Some(element) = locate(run.session, run.labels, TikTokControl::DialogDismiss)
            .await
            .ok()
            .flatten()
        {
            report(status, "đóng hộp thoại TikTok chắn feed".into());
            let _ = run.session.tap(element.centre()).await;
            sleep_interruptible(FEED_READY_POLL, stop).await;
            continue;
        }

        // **Try to go there before giving up.** A phone is left wherever the last session
        // or the last person left it, and Profile / Shop / Inbox are each one tap from the
        // feed — but `FeedTab` is a tab *inside* the feed, so on any of them this loop saw
        // nothing and waited out the whole window. Measured 18/08/2026: phones parked on
        // Profile failed every session, with a message that guessed at a splash screen or a
        // login page while the app was logged in and perfectly healthy.
        //
        // Once, not on every poll. The tap is cheap but not free, and a phone that does not
        // reach the feed after being sent there has something wrong that tapping again will
        // not fix.
        if !nudged && present(run.session, run.labels, TikTokControl::HomeTab).await {
            nudged = true;
            report(
                status,
                "TikTok đang ở tab khác — bấm Home để về feed".into(),
            );
            if let Some(element) = locate(run.session, run.labels, TikTokControl::HomeTab)
                .await
                .ok()
                .flatten()
            {
                let _ = run.session.tap(element.centre()).await;
            }
            sleep_interruptible(FEED_READY_POLL, stop).await;
            continue;
        }

        // **Nothing on this screen carries a label, so use the way out that needs none.**
        //
        // Measured 18/08/2026 on ce0517155ab38c390d: TikTok held the phone behind "Get
        // updates sent to your email?", and the whole dump had *no* `content-desc` at all —
        // no feed tab, no bottom bar, nothing any branch above could find. The dialog's only
        // labelled control was its accept button, and accepting subscribes a real account to
        // marketing email; the decline is an unlabelled `ImageView`. So no label can reach
        // it. Back can, and did: one press dismissed it and the feed was underneath.
        //
        // This is why it is worth having as well as [`TikTokControl::DialogDismiss`] rather
        // than instead of it. A measured decline is better when it exists — it is the button
        // a person would choose. But dialogs arrive faster than anyone can measure them, in
        // languages nobody has dumped, and this one had no reachable decline at all.
        //
        // Safe *here specifically*, because every branch above has already failed. Back on
        // the feed would leave TikTok — but then `on_feed` would have returned. On Profile,
        // Inbox or Shop the Home tab would have been found. What is left is a screen with
        // neither, which is a dialog or a splash.
        if !backed && started.elapsed() >= MODAL_BACK_DELAY {
            backed = true;
            report(
                status,
                "màn hình bị chặn và không có nút nào đọc được — bấm Back".into(),
            );
            let _ = run.session.back().await;
            sleep_interruptible(FEED_READY_POLL, stop).await;
            continue;
        }

        if !said {
            said = true;
            report(status, "TikTok đang khởi động — chờ feed lên".into());
        }
        sleep_interruptible(FEED_READY_POLL, stop).await;
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
    let mut policy = HumanSessionPolicy::new(
        settings.like_prob,
        settings.comment_prob,
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

    let total_videos = video_target(&settings);

    let mut off_feed_streak = 0u32;
    let mut stuck_swipes = 0u32;
    let mut restarted = false;
    let mut last_interaction_at: Option<Instant> = None;
    let mut outcome = Outcome::Done;
    // Latched so an operator who switches comments on mid-run is told once why nothing
    // happens, instead of watching a switch that reports nothing at all.
    let mut said_comments_impossible = settings.comment_prob > 0 && !comment_capable;

    'feed: for _video in 0..total_videos {
        // Live tuning, once per post — same place, same function, same reason as the pixel
        // loop.
        if apply_live_settings(live, &mut settings, &mut human, &mut policy, &mut moods)
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
            if !await_feed(&run, stop, status, report).await {
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
        let no_rail = !present(run.session, run.labels, TikTokControl::Comments).await;
        if live_label || no_rail {
            report(
                status,
                if live_label {
                    "thẻ LIVE — chỉ vuốt qua, không tương tác".into()
                } else {
                    "thẻ không có thanh hành động (LIVE / đang chuyển) — chỉ vuốt tiếp".to_string()
                },
            );
            status.swipe_attempts += 1;
            let before = fingerprint(run.session, run.labels).await;
            if run
                .swipe_next(human.swipe_duration_ms(false), &before, stop)
                .await
                .unwrap_or(false)
            {
                status.videos_done += 1;
            }
            continue;
        }

        let watch = human.watch_seconds(settings.watch_min, settings.watch_max) * mood.watch_mult();
        report(status, format!("xem {watch:.1}s ({})", mood.label()));
        sleep_interruptible(Duration::from_secs_f64(watch.max(0.5)), stop).await;
        if stop.load(Ordering::Relaxed) {
            outcome = Outcome::Stopped;
            break;
        }

        let before = fingerprint(run.session, run.labels).await;
        human.note_action();

        match roll_feed_action_in_mood(settings.like_prob, settings.comment_prob, mood) {
            FeedAction::Like
                if !policy.can_interact_with_post() || !policy.can_attempt(PolicyAction::Like) =>
            {
                report(status, "bỏ qua tim: nhịp phiên hiện tại đã đủ".into());
            }
            FeedAction::Like => {
                if !wait_gap(&mut last_interaction_at, policy.min_action_gap(), stop).await {
                    outcome = Outcome::Stopped;
                    break 'feed;
                }
                policy.record_attempt(PolicyAction::Like);
                policy.mark_post_interacted();
                status.like_attempts += 1;
                report(status, "thả tim".into());
                match run.like(stop).await {
                    Ok(LikeVerdict::Liked) => {
                        status.likes += 1;
                        report(status, "tim thành công (nhãn đổi trạng thái)".into());
                    }
                    Ok(LikeVerdict::AlreadyLiked) => {
                        report(status, "video đã tim từ trước — bỏ qua".into())
                    }
                    Ok(LikeVerdict::NoControl) => {
                        report(status, "bỏ qua tim: thẻ này không có nút tim".into())
                    }
                    Ok(LikeVerdict::NotConfirmed) => report(
                        status,
                        "tim: tap gửi được nhưng nhãn không đổi — không tính là đã tim".into(),
                    ),
                    Err(error) => {
                        report(status, format!("tim thất bại: {error}"));
                    }
                }
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
                policy.record_attempt(PolicyAction::Comment);
                policy.mark_post_interacted();
                status.comment_attempts += 1;
                report(status, "bình luận".into());
                let source = comments.expect("comment_capable implies a text source");
                let prepared = source.comment_for_post(&settings).await;
                match prepared {
                    None => report(
                        status,
                        format!(
                            "bỏ qua bình luận: {}",
                            CommentVerdict::ContextSkipped.reason()
                        ),
                    ),
                    Some(prepared) => match run.comment(&prepared.text, stop).await {
                        Ok(CommentVerdict::Sent) => {
                            status.comments += 1;
                            status.session_usd += prepared.usd;
                            report(status, "đã gửi bình luận (nút Gửi tắt lại)".into());
                            source.record_outcome(&prepared, "sent").await;
                        }
                        Ok(verdict) => {
                            report(status, format!("bỏ qua bình luận: {}", verdict.reason()));
                            source.record_outcome(&prepared, "skipped").await;
                        }
                        Err(error) => {
                            report(status, format!("bình luận thất bại: {error}"));
                            source.record_outcome(&prepared, "failed").await;
                        }
                    },
                }
            }
            FeedAction::None => {}
        }

        // **An action with no measured label is not an action this phone can attempt.**
        //
        // `follow` finds nothing when `Follow` was never measured for this build, returns
        // false, and the arm below reports "nút Follow vẫn còn — chưa xác nhận" — blaming a
        // button that was never looked for, on a screen where nothing was tapped. Sixteen
        // of the eighteen phones on this fleet run `com.ss.android.ugc.trill`, whose set has
        // no `follow` label, so that is the majority case rather than an edge one.
        //
        // Worse than the wrong sentence: the block above it spends `record_attempt` and
        // `mark_post_interacted` first, so a roll that could never do anything still used
        // up the human-pacing budget for that post and blocked the like that might have
        // followed it.
        if can_follow(run.labels)
            && roll_follow_in_mood(settings.follow_prob, mood)
            && policy.can_interact_with_post()
            && policy.can_attempt(PolicyAction::Follow)
            && wait_gap(&mut last_interaction_at, policy.min_action_gap(), stop).await
        {
            policy.record_attempt(PolicyAction::Follow);
            policy.mark_post_interacted();
            status.follow_attempts += 1;
            report(status, "follow tác giả".into());
            match run.follow(stop).await {
                Ok(true) => {
                    status.follows += 1;
                    report(status, "follow thành công (nút Follow mất khỏi thẻ)".into());
                }
                Ok(false) => report(
                    status,
                    "bỏ qua follow: nút Follow vẫn còn — chưa xác nhận".into(),
                ),
                Err(error) => report(status, format!("follow thất bại: {error}")),
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
        let ceiling = settings.carousel_ceiling();
        let mut before = before;
        if ceiling > 0 {
            let slides = run
                .traverse_carousel(
                    settings.carousel_portion_percent,
                    ceiling,
                    stop,
                    status,
                    report,
                )
                .await;
            if stop.load(Ordering::Relaxed) {
                outcome = Outcome::Stopped;
                break 'feed;
            }
            if slides > 1 {
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
        if !run.on_feed().await {
            if !await_feed(&run, stop, status, report).await {
                report(status, "không quay lại được feed — dừng".into());
                outcome = Outcome::Partial;
                break;
            }
            before = fingerprint(run.session, run.labels).await;
        }

        status.swipe_attempts += 1;
        let advanced = run
            .swipe_next(
                human.swipe_duration_ms(roll_bool(settings.frenzy_prob)),
                &before,
                stop,
            )
            .await
            .unwrap_or(false);
        if advanced {
            stuck_swipes = 0;
            status.videos_done += 1;
            if let Some(rest) = policy.rest_after_video() {
                report(status, format!("nghỉ tự nhiên {}s", rest.as_secs()));
                sleep_interruptible(rest, stop).await;
            }
        } else {
            stuck_swipes += 1;
            report(
                status,
                format!("vuốt chưa chứng minh được đổi thẻ ({stuck_swipes}/{STUCK_SWIPE_LIMIT})"),
            );
            if stuck_swipes >= STUCK_RESTART_AFTER && !restarted {
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
                report(status, "feed không đổi thẻ — khởi động lại TikTok".into());
                let _ = run.session.restart_app(bundle_id).await;
                if await_feed(&run, stop, status, report).await {
                    // No need to re-fingerprint: the loop reads the card it is about to
                    // leave at the top of each pass, and after a restart that is a
                    // different card anyway.
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

    fn texts(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| (*s).to_string()).collect()
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
            comments: Some("Đọc hoặc viết bình luận. 697 bình luận".into()),
            share: Some("Chia sẻ video. 45,4K lượt chia sẻ".into()),
            sound: None,
        };
        let second = PostFingerprint {
            comments: Some("Đọc hoặc viết bình luận. 12 bình luận".into()),
            share: first.share.clone(),
            sound: None,
        };
        assert_ne!(first, second);
        assert!(!first.is_empty());
    }

    #[test]
    fn two_zero_engagement_posts_are_still_told_apart() {
        // Verbatim from two phones on 18/08/2026. Counts alone are identical on a
        // low-engagement feed, so a fingerprint without the sound strip reported every
        // swipe as unproven: no video was counted, and the session stopped at
        // `STUCK_SWIPE_LIMIT` while it was in fact watching and swiping correctly.
        let counts_only = |sound: Option<&str>| PostFingerprint {
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
            };
            Ok(found.then_some(ElementBox {
                x: 420.0,
                y: 1_600.0,
                width: 240.0,
                height: 96.0,
                description: None,
                enabled: true,
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
        };
        let stop = AtomicBool::new(false);
        let mut status = NurtureSessionStatus {
            udid: "modal-phone".into(),
            running: true,
            videos_done: 0,
            swipe_attempts: 0,
            like_attempts: 0,
            comment_attempts: 0,
            follow_attempts: 0,
            likes: 0,
            comments: 0,
            follows: 0,
            last_message: String::new(),
            session_usd: 0.0,
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
        cleared: std::sync::atomic::AtomicBool,
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
            self.cleared.store(true, Ordering::Relaxed);
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
            let found = self.cleared.load(Ordering::Relaxed)
                && matches!(query, ElementQuery::Description { value, .. } if value == "For You");
            Ok(found.then_some(ElementBox {
                x: 0.0,
                y: 0.0,
                width: 200.0,
                height: 80.0,
                description: None,
                enabled: true,
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
        };
        let stop = AtomicBool::new(false);
        let mut status = NurtureSessionStatus {
            udid: "unlabelled-modal".into(),
            running: true,
            videos_done: 0,
            swipe_attempts: 0,
            like_attempts: 0,
            comment_attempts: 0,
            follow_attempts: 0,
            likes: 0,
            comments: 0,
            follows: 0,
            last_message: String::new(),
            session_usd: 0.0,
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
            "once — a screen that Back did not fix is not fixed by Back again"
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
            // The decline is always there. The feed never is.
            Ok(
                matches!(query, ElementQuery::Text { .. }).then_some(ElementBox {
                    x: 0.0,
                    y: 0.0,
                    width: 100.0,
                    height: 40.0,
                    description: None,
                    enabled: true,
                }),
            )
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
        };
        let stop = AtomicBool::new(false);
        let mut status = NurtureSessionStatus {
            udid: "stuck-dialog".into(),
            running: true,
            videos_done: 0,
            swipe_attempts: 0,
            like_attempts: 0,
            comment_attempts: 0,
            follow_attempts: 0,
            likes: 0,
            comments: 0,
            follows: 0,
            last_message: String::new(),
            session_usd: 0.0,
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
    }

    impl StuckFeedPhone {
        fn card(&self, prefix: &str) -> Option<ElementBox> {
            let generation = self.generation.load(Ordering::Relaxed);
            Some(ElementBox {
                x: 900.0,
                y: 1_000.0,
                width: 80.0,
                height: 80.0,
                description: Some(format!("{prefix} {generation}")),
                enabled: true,
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
            Ok(match value {
                "For You" => Some(ElementBox {
                    x: 0.0,
                    y: 0.0,
                    width: 100.0,
                    height: 40.0,
                    description: None,
                    enabled: true,
                }),
                "comments" => self.card("comments"),
                "Share video" => self.card("Share video"),
                "Original sound by" => self.card("Original sound by"),
                _ => None,
            })
        }
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
            udid: "stuck-feed".into(),
            running: true,
            videos_done: 0,
            swipe_attempts: 0,
            like_attempts: 0,
            comment_attempts: 0,
            follow_attempts: 0,
            likes: 0,
            comments: 0,
            follows: 0,
            last_message: String::new(),
            session_usd: 0.0,
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
            udid: "wanders-off".into(),
            running: true,
            videos_done: 0,
            swipe_attempts: 0,
            like_attempts: 0,
            comment_attempts: 0,
            follow_attempts: 0,
            likes: 0,
            comments: 0,
            follows: 0,
            last_message: String::new(),
            session_usd: 0.0,
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
        };
        let stop = AtomicBool::new(false);
        let mut status = NurtureSessionStatus {
            udid: "pager-phone".into(),
            running: true,
            videos_done: 0,
            swipe_attempts: 0,
            like_attempts: 0,
            comment_attempts: 0,
            follow_attempts: 0,
            likes: 0,
            comments: 0,
            follows: 0,
            last_message: String::new(),
            session_usd: 0.0,
        };
        let said = std::sync::Mutex::new(Vec::<String>::new());
        let report = |status: &mut NurtureSessionStatus, message: String| {
            status.last_message = message.clone();
            said.lock().expect("messages").push(message);
        };

        let seen = run
            .traverse_carousel(100, 20, &stop, &mut status, &report)
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
}
