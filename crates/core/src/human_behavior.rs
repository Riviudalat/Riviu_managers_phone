//! Human-like behavior state machine for TikTok nurture (ported cleanly from TOOL TIKTOK).

use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BehaviorState {
    Active,
    Distracted,
    Passive,
    Curious,
    Focused,
    Fatigued,
}

impl BehaviorState {
    pub fn latency_mult(self) -> f64 {
        match self {
            Self::Active => 1.0,
            Self::Distracted => 1.6,
            Self::Passive => 1.3,
            Self::Curious => 0.85,
            Self::Focused => 0.9,
            Self::Fatigued => 1.8,
        }
    }

    pub fn watch_mult(self) -> f64 {
        match self {
            Self::Active => 1.0,
            Self::Distracted => 0.7,
            Self::Passive => 0.85,
            Self::Curious => 1.4,
            Self::Focused => 1.5,
            Self::Fatigued => 0.6,
        }
    }
}

pub struct HumanBehavior {
    pub state: BehaviorState,
    pub persona: String,
    action_count: u32,
    next_transition_at: u32,
    consecutive_swipes: u32,
    pause_after_swipes: u32,
    last_watch: f64,
    session_start: std::time::Instant,
    fatigue_enabled: bool,
    tod_enabled: bool,
    pause_swipe: bool,
}

impl HumanBehavior {
    pub fn new(persona: &str, fatigue: bool, tod: bool, pause_swipe: bool) -> Self {
        let mut rng = rand::thread_rng();
        Self {
            state: BehaviorState::Active,
            persona: persona.to_string(),
            action_count: 0,
            next_transition_at: rng.gen_range(5..=15),
            consecutive_swipes: 0,
            pause_after_swipes: rng.gen_range(3..=6),
            last_watch: 0.0,
            session_start: std::time::Instant::now(),
            fatigue_enabled: fatigue,
            tod_enabled: tod,
            pause_swipe,
        }
    }

    /// Pick up the three rhythm switches an operator can flip while a session runs.
    ///
    /// Assignment rather than a rebuild, and that is the whole point: `session_start`
    /// is what fatigue is measured from, and `action_count` / `pause_after_swipes`
    /// are where the session is in its own arc. `HumanBehavior::new` would reset all
    /// of them, so re-enabling fatigue two hours in would restart its clock at zero
    /// and the run would act freshly rested — the opposite of what was asked for.
    ///
    /// `persona` is deliberately absent: it seeds the behaviour model itself, so it
    /// stays on the restart-required list in
    /// [`NurtureSettings::absorb_live_changes`](crate::types::NurtureSettings::absorb_live_changes).
    pub fn retune(&mut self, fatigue: bool, tod: bool, pause_swipe: bool) {
        self.fatigue_enabled = fatigue;
        self.tod_enabled = tod;
        self.pause_swipe = pause_swipe;
    }

    /// Read the three switches back.
    ///
    /// Test-only, and deliberately not public API: the switches are inputs, and code that
    /// wanted to branch on them would be second-guessing this type. They exist so
    /// `nurture::live` can assert that a saved change actually arrived here, which is the
    /// half of live tuning that was broken while looking correct.
    #[cfg(test)]
    pub(crate) fn fatigue_is_on(&self) -> bool {
        self.fatigue_enabled
    }

    #[cfg(test)]
    pub(crate) fn time_of_day_is_on(&self) -> bool {
        self.tod_enabled
    }

    #[cfg(test)]
    pub(crate) fn pause_swipe_is_on(&self) -> bool {
        self.pause_swipe
    }

    pub fn note_action(&mut self) {
        self.action_count += 1;
        if self.action_count >= self.next_transition_at {
            self.transition();
            let mut rng = rand::thread_rng();
            self.action_count = 0;
            self.next_transition_at = rng.gen_range(5..=15);
        }
    }

    fn transition(&mut self) {
        let mut rng = rand::thread_rng();
        self.state = match self.state {
            BehaviorState::Active => match rng.gen_range(0..4) {
                0 | 1 => BehaviorState::Active,
                2 => BehaviorState::Curious,
                _ => BehaviorState::Distracted,
            },
            BehaviorState::Distracted => match rng.gen_range(0..3) {
                0 => BehaviorState::Distracted,
                1 => BehaviorState::Passive,
                _ => BehaviorState::Active,
            },
            BehaviorState::Passive => match rng.gen_range(0..3) {
                0 => BehaviorState::Passive,
                1 => BehaviorState::Active,
                _ => BehaviorState::Fatigued,
            },
            BehaviorState::Curious => match rng.gen_range(0..4) {
                0 | 1 => BehaviorState::Curious,
                2 => BehaviorState::Focused,
                _ => BehaviorState::Active,
            },
            BehaviorState::Focused => match rng.gen_range(0..3) {
                0 => BehaviorState::Focused,
                1 => BehaviorState::Active,
                _ => BehaviorState::Passive,
            },
            BehaviorState::Fatigued => {
                if rng.gen_bool(2.0 / 3.0) {
                    BehaviorState::Fatigued
                } else {
                    BehaviorState::Passive
                }
            }
        };
    }

    /// How long to sit on one post.
    ///
    /// **Two things here were the same mistake the tap points made**, and both showed up in
    /// a real log as a pattern rather than as a person:
    ///
    /// * It forced consecutive dwells apart — `min_delta` of 15 % of the window, so two
    ///   posts in a row could not be watched for a similar length. A live run with a 3–5 s
    ///   window produced `2,5 · 3,6 · 2,8 · 3,2 · 2,7 · 2,3 · 2,9 · 2,0` — alternating, never
    ///   close twice. People watch two clips for the same length all the time.
    /// * The draw was three disjoint uniform bands (20 % low, 10 % high, 70 % middle), so
    ///   the shape had hard edges at the band boundaries and was flat inside each.
    ///
    /// Now it is one continuous draw, skewed to the short end, which is the shape watch time
    /// actually has: most posts get a glance, a few hold attention. Repeats are allowed
    /// because they happen.
    ///
    /// `persona` moves the skew rather than switching between bands — one dial instead of
    /// three hard-coded ranges, and `bored` / `curious` still mean what they meant.
    pub fn watch_seconds(&mut self, watch_min: f64, watch_max: f64) -> f64 {
        let mut rng = rand::thread_rng();
        let span = (watch_max - watch_min).max(0.5);
        // Above 1 pulls mass toward the short end, below 1 toward the long end.
        let skew = match self.persona.as_str() {
            "bored" => 2.6,
            "curious" => 0.6,
            _ => 1.7,
        };
        let unit = rng.gen_range(0.0..1.0_f64).powf(skew);
        let mut secs = (watch_min + span * unit) * self.state.watch_mult();
        if self.fatigue_enabled {
            secs *= self.fatigue_mult();
        }
        if self.tod_enabled {
            secs *= self.tod_mult();
        }
        self.last_watch = secs.clamp(watch_min, watch_max);
        self.last_watch
    }

    fn fatigue_mult(&self) -> f64 {
        let mins = self.session_start.elapsed().as_secs_f64() / 60.0;
        if mins <= 15.0 {
            1.0
        } else if mins <= 30.0 {
            1.0 + (mins - 15.0) / 15.0 * 0.15
        } else if mins <= 60.0 {
            1.15 + (mins - 30.0) / 30.0 * 0.20
        } else {
            1.6_f64.min(1.35 + (mins - 60.0) / 60.0 * 0.25)
        }
    }

    fn tod_mult(&self) -> f64 {
        let hour = chrono::Local::now()
            .format("%H")
            .to_string()
            .parse::<u32>()
            .unwrap_or(12);
        match hour {
            6..=8 => 1.2,
            9..=11 => 0.95,
            12..=13 => 1.1,
            14..=17 => 1.0,
            18..=21 => 0.9,
            _ => 1.4,
        }
    }

    pub fn think_pause_ms(&self) -> u64 {
        let mut rng = rand::thread_rng();
        let base = rng.gen_range(400.0..1400.0) * self.state.latency_mult();
        base as u64
    }

    pub fn after_swipe_pause_ms(&mut self) -> u64 {
        let mut rng = rand::thread_rng();
        self.consecutive_swipes += 1;
        let mut ms = rng.gen_range(800.0..1600.0) * self.state.latency_mult();
        if self.pause_swipe && self.consecutive_swipes >= self.pause_after_swipes {
            ms += rng.gen_range(1000.0..3500.0);
            self.consecutive_swipes = 0;
            self.pause_after_swipes = rng.gen_range(3..=6);
        }
        ms as u64
    }

    /// Sample a swipe duration from a human-looking mixture. Fast swipes are
    /// rare, ordinary swipes dominate, and distracted/fatigued moments create
    /// occasional slow drags. `frenzy` is the explicit rare fast-scroll mode.
    /// How long one feed flick takes.
    ///
    /// **The old version left a hole in the distribution.** It picked one of three disjoint
    /// ranges — 190–280, 300–520, 520–820 — so no swipe ever lasted 281–299 ms. A histogram
    /// with a gap in it is a stronger signal than any single value, and nothing about a
    /// finger produces one.
    ///
    /// One continuous range now, skewed short, which keeps the old shape's intent — most
    /// flicks brisk, some slow and deliberate — without the seams.
    pub fn swipe_duration_ms(&mut self, frenzy: bool) -> u64 {
        let mut rng = rand::thread_rng();
        let (low, high, skew) = if frenzy {
            // The bounds are the ones that were already here — 150–240 — because nothing
            // measured says otherwise and the fault being fixed was the *hole* in the
            // non-frenzy range, not its edges. `mood_tests::swipe_duration_profile_stays_
            // inside_human_bounds` pins them, and it is right to.
            (150.0, 240.0, 1.3)
        } else {
            (190.0, 820.0, 1.8)
        };
        let unit = rng.gen_range(0.0..1.0_f64).powf(skew);
        (low + (high - low) * unit).round() as u64
    }

    /// Photo carousels use a horizontal drag, usually slower than a feed swipe while a
    /// person reads the image, with occasional brisk changes.
    ///
    /// Skewed the other way — below 1 — because this one leans *long*: the reason to drag a
    /// carousel slowly is that there is something to look at.
    pub fn photo_swipe_duration_ms(&mut self) -> u64 {
        let mut rng = rand::thread_rng();
        let unit = rng.gen_range(0.0..1.0_f64).powf(0.7);
        (280.0 + 480.0 * unit).round() as u64
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FeedAction {
    Like,
    Save,
    Comment,
    None,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FeedActionPlan {
    pub like: bool,
    pub save: bool,
    pub comment: bool,
    pub follow: bool,
}

impl FeedActionPlan {
    pub fn ordered(self) -> Vec<PolicyAction> {
        [
            (self.like, PolicyAction::Like),
            (self.save, PolicyAction::Save),
            (self.comment, PolicyAction::Comment),
            (self.follow, PolicyAction::Follow),
        ]
        .into_iter()
        .filter_map(|(selected, action)| selected.then_some(action))
        .collect()
    }
}

pub fn roll_feed_action(like_prob: u32, comment_prob: u32) -> FeedAction {
    let like = like_prob.min(100);
    let comment = comment_prob.min(100_u32.saturating_sub(like));
    let mut rng = rand::thread_rng();
    let roll = rng.gen_range(0..100);
    if roll < like {
        FeedAction::Like
    } else if roll < like + comment {
        FeedAction::Comment
    } else {
        FeedAction::None
    }
}

pub fn roll_bool(prob_percent: u32) -> bool {
    let mut rng = rand::thread_rng();
    rng.gen_range(0..100) < prob_percent.min(100)
}

/// Deterministic direction selection for a prepared comment. A retry for the
/// same frame fingerprint must keep the same style instruction.
pub fn pick_direction_seeded(raw: &str, seed: u64) -> Option<String> {
    let parts: Vec<_> = raw
        .split('|')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();
    if parts.is_empty() {
        return None;
    }
    let mut rng = StdRng::seed_from_u64(seed);
    Some(parts[rng.gen_range(0..parts.len())].to_string())
}

pub fn in_night_window(start: u32, end: u32) -> bool {
    if start == end {
        return false;
    }
    let hour = chrono::Local::now()
        .format("%H")
        .to_string()
        .parse::<u32>()
        .unwrap_or(12);
    if start < end {
        hour >= start && hour < end
    } else {
        hour >= start || hour < end
    }
}

/// A stretch of consistent intent, so a session reads like one person rather
/// than an independent coin flip per video.
///
/// Real users do not interact at a flat 40 % rate: they skim a dozen clips
/// without touching anything, then hit a run of videos they like, then get
/// chatty on one topic. [`MoodCycle`] reproduces that shape while keeping the
/// session average close to the configured probabilities, so the settings still
/// mean what they say.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Mood {
    /// Scrolling past things: short watches, no interaction.
    Skimming,
    /// Enjoying a run: normal watches, plenty of likes, the odd follow.
    Liking,
    /// Invested in a topic: long watches, comments, likes.
    Chatty,
    /// No mood at all: every multiplier is 1.0, so the configured probability applies to
    /// every post and the configured watch window is the real window.
    ///
    /// This is what `human_limits = false` runs, and it exists because the mood cycle was
    /// the **third** layer quietly overriding the panel. `Skimming::like_mult()` is
    /// `0.0` — not reduced, off — and Skimming is about 60 % of videos, so a session that
    /// happened to stay in it liked nothing at all at any setting. Measured on 12/08/2026:
    /// a twelve-video run with `like_prob = 100` reported `tim 0/0`, every post logged
    /// `(lướt)`.
    ///
    /// The per-mood multipliers are still right for what they are for — they average to 1.0
    /// across a long session, which keeps the *setting* honest over an hour. They are simply
    /// not what an operator asking for "100 % means every post" is asking for.
    Neutral,
}

impl Mood {
    /// Multipliers applied to the configured probabilities while this mood runs.
    ///
    /// Skimming never interacts, so the like/comment/follow expectation is
    /// carried entirely by the Liking and Chatty runs. Videos are shared across
    /// moods ≈ 60.4 % Skimming / 30.5 % Liking / 9.1 % Chatty (each mood's video
    /// share = P(mood) × mean run length), so these multipliers are scaled to
    /// make the video-weighted session average land at 1.0 per channel before
    /// per-mood clamping — the setting still means what it says. Verified by
    /// `mood_multipliers_average_near_one_over_a_long_session`.
    fn like_mult(self) -> f64 {
        match self {
            Mood::Neutral => 1.0,
            Mood::Skimming => 0.0,
            Mood::Liking => 2.82,
            Mood::Chatty => 1.54,
        }
    }

    fn comment_mult(self) -> f64 {
        match self {
            Mood::Neutral => 1.0,
            Mood::Skimming => 0.0,
            Mood::Liking => 1.18,
            Mood::Chatty => 7.07,
        }
    }

    fn save_mult(self) -> f64 {
        self.like_mult()
    }

    fn follow_mult(self) -> f64 {
        match self {
            Mood::Neutral => 1.0,
            Mood::Skimming => 0.0,
            Mood::Liking => 2.60,
            Mood::Chatty => 2.28,
        }
    }

    /// Watch-length multiplier — skimming is quick, chatty lingers.
    pub fn watch_mult(self) -> f64 {
        match self {
            Mood::Neutral => 1.0,
            Mood::Skimming => 0.55,
            Mood::Liking => 1.0,
            Mood::Chatty => 1.45,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Mood::Skimming => "lướt",
            Mood::Liking => "xem kỹ",
            Mood::Chatty => "tương tác",
            Mood::Neutral => "theo đúng tỉ lệ đặt",
        }
    }
}

/// Runs of one mood at a time, with the run length drawn per mood.
pub struct MoodCycle {
    current: Mood,
    remaining: u32,
}

impl Default for MoodCycle {
    fn default() -> Self {
        Self::new()
    }
}

impl MoodCycle {
    /// A cycle pinned to one mood. Used by feature tests that need every video
    /// to attempt the same action instead of being diluted by skim runs.
    pub fn fixed(mood: Mood) -> Self {
        Self {
            current: mood,
            remaining: u32::MAX,
        }
    }

    pub fn new() -> Self {
        let mut c = Self {
            current: Mood::Skimming,
            remaining: 0,
        };
        c.roll();
        c
    }

    /// A cycle that never varies anything: [`Mood::Neutral`] forever.
    ///
    /// What `human_limits = false` uses, so the configured probabilities and watch window
    /// apply to every post rather than to a session average.
    pub fn neutral() -> Self {
        Self::fixed(Mood::Neutral)
    }

    /// Switch between the varying cycle and [`Self::neutral`] mid-session.
    ///
    /// Needed because the pacing switch is live-tunable: an operator who turns it off while
    /// a run is going has to get the plain probabilities from the next post, not the next
    /// run — the same rule every other live switch follows.
    pub fn retune(&mut self, limits: bool) {
        match (limits, self.current) {
            // Leaving full control: start varying again from a fresh run.
            (true, Mood::Neutral) => {
                self.remaining = 0;
                self.roll();
            }
            // Entering full control: stop varying immediately.
            (false, _) => *self = Self::neutral(),
            _ => {}
        }
    }

    /// The mood for this video, advancing the cycle. Returns `(mood, changed)`
    /// so the caller can log a transition once rather than every video.
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> (Mood, bool) {
        if self.remaining == 0 {
            self.roll();
            (self.current, true)
        } else {
            self.remaining -= 1;
            (self.current, false)
        }
    }

    pub fn current(&self) -> Mood {
        self.current
    }

    fn roll(&mut self) {
        let mut rng = rand::thread_rng();
        // Skimming dominates by design: most of a real feed session is scrolling.
        let (mood, span) = match rng.gen_range(0..100) {
            0..=49 => (Mood::Skimming, rng.gen_range(4..=12)),
            50..=84 => (Mood::Liking, rng.gen_range(3..=8)),
            _ => (Mood::Chatty, rng.gen_range(2..=5)),
        };
        self.current = mood;
        self.remaining = span;
    }
}

/// Roll a feed action using the configured probabilities scaled by mood.
pub fn roll_feed_action_in_mood(like_prob: u32, comment_prob: u32, mood: Mood) -> FeedAction {
    let like = ((like_prob as f64) * mood.like_mult())
        .round()
        .clamp(0.0, 100.0) as u32;
    let comment = ((comment_prob as f64) * mood.comment_mult())
        .round()
        .clamp(0.0, 100.0) as u32;
    roll_feed_action(like, comment)
}

/// Roll a follow using the configured probability scaled by mood.
pub fn roll_follow_in_mood(follow_prob: u32, mood: Mood) -> bool {
    let p = ((follow_prob as f64) * mood.follow_mult())
        .round()
        .clamp(0.0, 100.0) as u32;
    roll_bool(p)
}

fn scaled_probability(probability: u32, multiplier: f64) -> u32 {
    ((probability as f64) * multiplier)
        .round()
        .clamp(0.0, 100.0) as u32
}

pub fn feed_action_plan_from_rolls(
    like_prob: u32,
    comment_prob: u32,
    save_prob: u32,
    follow_prob: u32,
    mood: Mood,
    rolls: [u32; 4],
) -> FeedActionPlan {
    FeedActionPlan {
        like: rolls[0].min(99) < scaled_probability(like_prob, mood.like_mult()),
        save: rolls[1].min(99) < scaled_probability(save_prob, mood.save_mult()),
        comment: rolls[2].min(99) < scaled_probability(comment_prob, mood.comment_mult()),
        follow: rolls[3].min(99) < scaled_probability(follow_prob, mood.follow_mult()),
    }
}

pub fn roll_feed_actions_in_mood(
    like_prob: u32,
    comment_prob: u32,
    save_prob: u32,
    follow_prob: u32,
    mood: Mood,
) -> FeedActionPlan {
    let mut rng = rand::thread_rng();
    feed_action_plan_from_rolls(
        like_prob,
        comment_prob,
        save_prob,
        follow_prob,
        mood,
        [
            rng.gen_range(0..100),
            rng.gen_range(0..100),
            rng.gen_range(0..100),
            rng.gen_range(0..100),
        ],
    )
}

/// Internal pacing policy. It is deliberately not exposed as a user setting:
/// configured probabilities still decide whether an action is desired, while
/// this policy keeps the resulting session inside a human-sized rolling rate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PolicyAction {
    Like,
    Save,
    Comment,
    Follow,
}

/// A write-ahead policy charge that can be returned only when the action proves it touched
/// nothing. It is deliberately neither `Copy` nor `Clone`: settling it consumes the only
/// token a caller can hold. The policy identity prevents an id from another session matching
/// by accident, while the post generation keeps a delayed refund attached to its original
/// card after the feed advances.
#[derive(Debug, PartialEq, Eq)]
pub struct AttemptReservation {
    policy_id: u64,
    attempt_id: u64,
    post_generation: Option<u64>,
}

#[derive(Debug, Clone)]
struct AttemptRecord {
    at: Instant,
    action: PolicyAction,
    attempt_id: u64,
    post_generation: Option<u64>,
    outstanding: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RecentPost {
    generation: u64,
    interactions: u32,
}

static NEXT_POLICY_ID: AtomicU64 = AtomicU64::new(1);

/// The gap left between two actions when the human pacing is switched off.
///
/// Not zero, and not a policy either — it is a **settle**. Every action here proves itself
/// by reading the screen back (a like by its label flipping, a comment by the Send button
/// disarming), and firing the next gesture into a screen that is still animating is how a
/// tap lands on whatever slides under it. 800 ms is below anything a human-pacing argument
/// would ask for and above the frame or two a transition needs.
const UNPACED_ACTION_GAP: Duration = Duration::from_millis(800);

#[derive(Debug)]
pub struct HumanSessionPolicy {
    policy_id: u64,
    like_cap: u32,
    save_cap: u32,
    comment_cap: u32,
    follow_cap: u32,
    attempts: VecDeque<AttemptRecord>,
    next_attempt_id: u64,
    next_post_generation: u64,
    last_action_at: Option<Instant>,
    /// Number of committed or outstanding reservations on each recent card. A count rather
    /// than a bool lets one no-op reservation be refunded without erasing another real action
    /// performed on the same card.
    recent_posts: VecDeque<RecentPost>,
    videos_since_break: u32,
    next_rest_at: u32,
    block_started: Instant,
    block_length: Duration,
    cold_restart_used: bool,
    /// Whether this type is allowed to override the operator at all.
    ///
    /// `false` is **full control**: every ceiling, every enforced gap and every imposed
    /// rest below stops applying, and the configured probabilities become exactly what
    /// happens. That is the shipped default, by an explicit operator decision on
    /// 12/08/2026 — see [`Self::new`].
    limits: bool,
    rng: StdRng,
}

impl HumanSessionPolicy {
    /// `limits` decides whether anything in this type is allowed to bind.
    ///
    /// **What it holds back, measured against what an operator sets:** a per-hour ceiling
    /// of 8–16 likes / 8–16 saves / 1–3 comments / 1–2 follows; a rule that at most **two of the last
    /// five** cards may be interacted with at all; a **12–35 s** wait after every action;
    /// a 15–90 s rest every 7–13 videos; and 20–45 minute block breaks. Together those
    /// meant that "Thích 100%" produced likes on well under half the posts — the ceiling
    /// and the two-of-five rule bound long before the probability did, which is exactly
    /// the surprise that led to this parameter existing.
    ///
    /// `false` — the default — makes the configured numbers the real numbers. The cost is
    /// real and belongs on the record: this pacing is what makes a session look like a
    /// person, so a run without it is faster, denser and more distinguishable. The
    /// operator asked for the numbers to mean what they say and owns that trade.
    pub fn new(like_prob: u32, comment_prob: u32, follow_prob: u32, limits: bool) -> Self {
        Self::new_with_save(like_prob, comment_prob, 0, follow_prob, limits)
    }

    pub fn new_with_save(
        like_prob: u32,
        comment_prob: u32,
        save_prob: u32,
        follow_prob: u32,
        limits: bool,
    ) -> Self {
        let seed = rand::thread_rng().gen::<u64>();
        let mut rng = StdRng::seed_from_u64(seed);
        let cap = |prob: u32, low: u32, high: u32, rng: &mut StdRng| {
            if prob == 0 {
                0
            } else {
                rng.gen_range(low..=high)
            }
        };
        let next_rest_at = rng.gen_range(7..=13);
        Self {
            policy_id: NEXT_POLICY_ID.fetch_add(1, Ordering::Relaxed),
            like_cap: cap(like_prob, 8, 16, &mut rng),
            save_cap: cap(save_prob, 8, 16, &mut rng),
            comment_cap: cap(comment_prob, 1, 3, &mut rng),
            follow_cap: cap(follow_prob, 1, 2, &mut rng),
            attempts: VecDeque::new(),
            next_attempt_id: 1,
            next_post_generation: 1,
            last_action_at: None,
            recent_posts: VecDeque::with_capacity(5),
            videos_since_break: 0,
            next_rest_at,
            block_started: Instant::now(),
            block_length: Duration::from_secs(rng.gen_range(20..=45) * 60),
            cold_restart_used: false,
            limits,
            rng,
        }
    }

    /// Re-open a per-hour ceiling that was closed because the feature was off when the
    /// session started.
    ///
    /// The ceilings themselves stay out of the operator's hands — that is this type's
    /// stated design, and a run whose ceiling could be raised mid-session would not be
    /// human-paced any more. What this fixes is narrower and was a real dead end: a cap
    /// is `0` when its probability was `0` at construction, and `can_attempt` reads `0`
    /// as *never*. So switching a feature on while a session ran left it switched on
    /// everywhere in the UI and still unable to fire, forever, with no message saying so.
    ///
    /// Hence only the transitions are acted on. A cap that is already open keeps the
    /// number it was given, so this is safe to call once per post: re-rolling the ceiling
    /// every post would make "at most 8–16 an hour" mean nothing.
    pub fn retune(&mut self, like_prob: u32, comment_prob: u32, follow_prob: u32, limits: bool) {
        self.retune_with_save(like_prob, comment_prob, 0, follow_prob, limits);
    }

    pub fn retune_with_save(
        &mut self,
        like_prob: u32,
        comment_prob: u32,
        save_prob: u32,
        follow_prob: u32,
        limits: bool,
    ) {
        self.limits = limits;
        Self::retune_cap(&mut self.like_cap, like_prob, 8, 16, &mut self.rng);
        Self::retune_cap(&mut self.save_cap, save_prob, 8, 16, &mut self.rng);
        Self::retune_cap(&mut self.comment_cap, comment_prob, 1, 3, &mut self.rng);
        Self::retune_cap(&mut self.follow_cap, follow_prob, 1, 2, &mut self.rng);
    }

    /// The like ceiling as it stands. Test-only: it exists to prove that calling
    /// [`Self::retune`] once per post leaves an already-open ceiling alone.
    #[cfg(test)]
    pub(crate) fn like_ceiling(&self) -> u32 {
        self.like_cap
    }

    fn retune_cap(cap: &mut u32, prob: u32, low: u32, high: u32, rng: &mut StdRng) {
        match (prob, *cap) {
            // Switched off: shut the ceiling, so the change takes effect on this post
            // rather than only on the probability roll.
            (0, _) => *cap = 0,
            // Switched on after starting off: it has no ceiling yet, give it one.
            (_, 0) => *cap = rng.gen_range(low..=high),
            // Already open. Leave it exactly as it was.
            _ => {}
        }
    }

    /// Begin a new feed card. At most two of the last five cards can receive
    /// an interaction; a card skipped by the engine remains false.
    pub fn begin_post(&mut self) {
        if self.recent_posts.len() == 5 {
            self.recent_posts.pop_front();
        }
        let generation = self.next_post_generation;
        self.next_post_generation = self.next_post_generation.wrapping_add(1).max(1);
        self.recent_posts.push_back(RecentPost {
            generation,
            interactions: 0,
        });
    }

    pub fn can_interact_with_post(&self) -> bool {
        if !self.limits {
            return true;
        }
        if self
            .recent_posts
            .back()
            .is_some_and(|post| post.interactions > 0)
        {
            return true;
        }
        self.recent_posts
            .iter()
            .filter(|post| post.interactions > 0)
            .count()
            < 2
    }

    pub fn mark_post_interacted(&mut self) {
        if let Some(last) = self.recent_posts.back_mut() {
            last.interactions = last.interactions.saturating_add(1);
        }
    }

    fn prune(&mut self, now: Instant) {
        let window = Duration::from_secs(60 * 60);
        while self
            .attempts
            .front()
            .is_some_and(|attempt| now.duration_since(attempt.at) >= window)
        {
            self.attempts.pop_front();
        }
    }

    pub fn can_attempt(&mut self, action: PolicyAction) -> bool {
        if !self.limits {
            return true;
        }
        let now = Instant::now();
        self.prune(now);
        let cap = match action {
            PolicyAction::Like => self.like_cap,
            PolicyAction::Save => self.save_cap,
            PolicyAction::Comment => self.comment_cap,
            PolicyAction::Follow => self.follow_cap,
        };
        if cap == 0 {
            return false;
        }
        self.attempts
            .iter()
            .filter(|attempt| attempt.action == action)
            .count()
            < cap as usize
    }

    /// Record an attempt before the gesture. A failed confirmation still
    /// consumes this slot, preventing retries from becoming a burst.
    pub fn record_attempt(&mut self, action: PolicyAction) {
        let now = Instant::now();
        self.prune(now);
        let id = self.next_attempt_id;
        self.next_attempt_id = self.next_attempt_id.wrapping_add(1).max(1);
        self.attempts.push_back(AttemptRecord {
            at: now,
            action,
            attempt_id: id,
            post_generation: self.recent_posts.back().map(|post| post.generation),
            outstanding: false,
        });
        self.last_action_at = Some(now);
    }

    /// Reserve one rate slot and mark the current card before any gesture can leave the
    /// process. Call [`Self::cancel_no_effect`] only for a verdict that proves no tap or text
    /// injection occurred; an unconfirmed or transport-ambiguous gesture must be committed.
    pub fn reserve_attempt(&mut self, action: PolicyAction) -> AttemptReservation {
        let now = Instant::now();
        self.prune(now);
        let id = self.next_attempt_id;
        self.next_attempt_id = self.next_attempt_id.wrapping_add(1).max(1);
        let post_generation = self.recent_posts.back().map(|post| post.generation);
        let reservation = AttemptReservation {
            policy_id: self.policy_id,
            attempt_id: id,
            post_generation,
        };
        self.attempts.push_back(AttemptRecord {
            at: now,
            action,
            attempt_id: id,
            post_generation,
            outstanding: true,
        });
        self.last_action_at = Some(now);
        self.mark_post_interacted();
        reservation
    }

    /// Keep a reservation. Returns `false` for a stale, already-settled, or foreign token.
    /// The write-ahead attempt remains in the rolling window; only its outstanding marker is
    /// cleared so even a forged replay cannot later refund it.
    pub fn commit_attempt(&mut self, reservation: AttemptReservation) -> bool {
        if reservation.policy_id != self.policy_id {
            return false;
        }
        let Some(attempt) = self.attempts.iter_mut().find(|attempt| {
            attempt.outstanding
                && attempt.attempt_id == reservation.attempt_id
                && attempt.post_generation == reservation.post_generation
        }) else {
            return false;
        };
        attempt.outstanding = false;
        true
    }

    /// Return a reservation whose action proved it touched nothing. Returns `false` and makes
    /// no state change when the token is stale, already settled, or belongs to another policy.
    pub fn cancel_no_effect(&mut self, reservation: AttemptReservation) -> bool {
        if reservation.policy_id != self.policy_id {
            return false;
        }
        if let Some(index) = self.attempts.iter().position(|attempt| {
            attempt.outstanding
                && attempt.attempt_id == reservation.attempt_id
                && attempt.post_generation == reservation.post_generation
        }) {
            self.attempts.remove(index);
            if let Some(post_generation) = reservation.post_generation {
                if let Some(post) = self
                    .recent_posts
                    .iter_mut()
                    .find(|post| post.generation == post_generation)
                {
                    post.interactions = post.interactions.saturating_sub(1);
                }
            }
            self.last_action_at = self.attempts.back().map(|attempt| attempt.at);
            true
        } else {
            false
        }
    }

    #[cfg(test)]
    pub(crate) fn pin_cap_for_test(&mut self, action: PolicyAction, cap: u32) {
        match action {
            PolicyAction::Like => self.like_cap = cap,
            PolicyAction::Save => self.save_cap = cap,
            PolicyAction::Comment => self.comment_cap = cap,
            PolicyAction::Follow => self.follow_cap = cap,
        }
    }

    #[cfg(test)]
    fn current_post_interactions(&self) -> u32 {
        self.recent_posts.back().map_or(0, |post| post.interactions)
    }

    #[cfg(test)]
    fn interacted_card_count(&self) -> usize {
        self.recent_posts
            .iter()
            .filter(|post| post.interactions > 0)
            .count()
    }

    /// Return the next gap after the previous action. The selected range is
    /// intentionally broad enough to avoid a metronomic cadence.
    pub fn min_action_gap(&mut self) -> Duration {
        if !self.limits {
            return UNPACED_ACTION_GAP;
        }
        Duration::from_secs(self.rng.gen_range(12..=35))
    }

    pub fn rest_after_video(&mut self) -> Option<Duration> {
        if !self.limits {
            return None;
        }
        self.videos_since_break = self.videos_since_break.saturating_add(1);
        if self.videos_since_break < self.next_rest_at {
            return None;
        }
        self.videos_since_break = 0;
        self.next_rest_at = self.rng.gen_range(7..=13);
        Some(Duration::from_secs(self.rng.gen_range(15..=90)))
    }

    pub fn should_take_home_break(&mut self) -> bool {
        self.limits && self.rng.gen_ratio(1, 18)
    }

    pub fn should_enter_live(&mut self) -> bool {
        self.rng.gen_ratio(1, 6)
    }

    pub fn should_take_block_break(&self) -> bool {
        self.limits && self.block_started.elapsed() >= self.block_length
    }

    pub fn reset_block(&mut self) {
        self.block_started = Instant::now();
        self.block_length = Duration::from_secs(self.rng.gen_range(20..=45) * 60);
    }

    pub fn home_break_duration(&mut self) -> Duration {
        Duration::from_secs(self.rng.gen_range(60..=240))
    }

    pub fn should_cold_restart(&mut self) -> bool {
        if !self.limits || self.cold_restart_used {
            return false;
        }
        let selected = self.rng.gen_ratio(1, 240);
        if selected {
            self.cold_restart_used = true;
        }
        selected
    }
}

#[cfg(test)]
mod mood_tests {
    use super::*;

    /// The per-mood multipliers must make the video-weighted session average
    /// land at ~1.0 per channel, measured against the real MoodCycle roll
    /// distribution and run lengths (not a hand-computed share). Regression
    /// guard for the skew that used to deliver ≈0.42× on comments.
    #[test]
    fn mood_multipliers_average_near_one_over_a_long_session() {
        let mut cycle = MoodCycle::new();
        let iterations = 300_000;
        let (mut like, mut comment, mut follow) = (0.0_f64, 0.0_f64, 0.0_f64);
        for _ in 0..iterations {
            let (mood, _) = cycle.next();
            like += mood.like_mult();
            comment += mood.comment_mult();
            follow += mood.follow_mult();
        }
        let n = iterations as f64;
        let (like, comment, follow) = (like / n, comment / n, follow / n);
        assert!(
            (like - 1.0).abs() < 0.07,
            "like average {like:.3} drifted off 1.0"
        );
        assert!(
            (comment - 1.0).abs() < 0.07,
            "comment average {comment:.3} drifted off 1.0"
        );
        assert!(
            (follow - 1.0).abs() < 0.07,
            "follow average {follow:.3} drifted off 1.0"
        );
    }

    #[test]
    fn skimming_never_interacts() {
        for _ in 0..200 {
            assert_eq!(
                roll_feed_action_in_mood(100, 100, Mood::Skimming),
                FeedAction::None
            );
            assert!(!roll_follow_in_mood(100, Mood::Skimming));
        }
    }

    #[test]
    fn chatty_comments_far_more_often_than_skimming() {
        let count = |mood| {
            (0..400)
                .filter(|_| roll_feed_action_in_mood(20, 20, mood) == FeedAction::Comment)
                .count()
        };
        assert!(count(Mood::Chatty) > count(Mood::Liking));
        assert_eq!(count(Mood::Skimming), 0);
    }

    /// A cycle must actually change mood over a session rather than locking in.
    #[test]
    fn the_cycle_visits_more_than_one_mood() {
        let mut cycle = MoodCycle::new();
        let mut seen = std::collections::HashSet::new();
        let mut changes = 0;
        for _ in 0..400 {
            let (mood, changed) = cycle.next();
            seen.insert(mood);
            if changed {
                changes += 1;
            }
        }
        assert!(seen.len() >= 2, "session stuck in one mood: {seen:?}");
        assert!(changes > 5, "only {changes} mood changes in 400 videos");
    }

    /// Runs must have length — a mood that changes every video is just noise.
    #[test]
    fn a_mood_persists_for_several_videos() {
        let mut cycle = MoodCycle::new();
        let mut run = 0;
        let mut longest = 0;
        for _ in 0..200 {
            let (_, changed) = cycle.next();
            if changed {
                longest = longest.max(run);
                run = 1;
            } else {
                run += 1;
            }
        }
        assert!(longest >= 3, "longest run was only {longest} videos");
    }

    #[test]
    fn watch_length_follows_the_mood() {
        assert!(Mood::Skimming.watch_mult() < Mood::Liking.watch_mult());
        assert!(Mood::Liking.watch_mult() < Mood::Chatty.watch_mult());
    }

    #[test]
    fn internal_policy_caps_attempts_and_post_bursts() {
        let mut policy = HumanSessionPolicy::new(100, 100, 100, true);
        policy.begin_post();
        assert!(policy.can_attempt(PolicyAction::Like));
        policy.record_attempt(PolicyAction::Like);
        policy.mark_post_interacted();
        policy.begin_post();
        assert!(policy.can_attempt(PolicyAction::Comment));
        policy.record_attempt(PolicyAction::Comment);
        policy.mark_post_interacted();
        policy.begin_post();
        assert!(!policy.can_interact_with_post());
    }

    #[test]
    fn a_cancelled_no_effect_reservation_spends_no_hourly_or_post_budget() {
        let mut policy = HumanSessionPolicy::new(100, 100, 100, true);
        policy.begin_post();
        let cap = policy.like_ceiling();

        for _ in 0..=cap {
            let reservation = policy.reserve_attempt(PolicyAction::Like);
            policy.cancel_no_effect(reservation);
        }

        assert!(
            policy.can_attempt(PolicyAction::Like),
            "a missing control or unchanged card must not close the hourly ceiling"
        );
        assert!(
            policy.can_interact_with_post(),
            "a no-op must not mark the card as publicly interacted"
        );
    }

    #[test]
    fn one_ambiguous_effect_closes_a_one_attempt_cap() {
        let mut policy = HumanSessionPolicy::new(100, 100, 100, true);
        // Pin a deliberately small cap: the production cap is randomized, while this test
        // needs one ambiguous tap/type to prove that a retained reservation closes it.
        policy.comment_cap = 1;
        policy.begin_post();

        let ambiguous = policy.reserve_attempt(PolicyAction::Comment);
        policy.commit_attempt(ambiguous);

        assert!(
            !policy.can_attempt(PolicyAction::Comment),
            "an unconfirmed gesture must retain its reservation and close the cap"
        );
        policy.begin_post();
        policy.mark_post_interacted();
        policy.begin_post();
        assert!(
            !policy.can_interact_with_post(),
            "the ambiguous gesture must remain a real interaction on its card"
        );
    }

    #[test]
    fn cancelling_one_reservation_keeps_another_real_action_on_the_post() {
        let mut policy = HumanSessionPolicy::new(100, 100, 100, true);
        policy.begin_post();
        let real = policy.reserve_attempt(PolicyAction::Like);
        policy.commit_attempt(real);
        let no_effect = policy.reserve_attempt(PolicyAction::Follow);
        policy.cancel_no_effect(no_effect);

        policy.begin_post();
        policy.mark_post_interacted();
        policy.begin_post();
        assert!(
            !policy.can_interact_with_post(),
            "refunding follow must not erase the like already performed on that card"
        );
    }

    #[test]
    fn cancelling_overlapping_reservations_cannot_restore_a_cancelled_timestamp() {
        let mut policy = HumanSessionPolicy::new(100, 100, 100, true);
        policy.begin_post();
        let first = policy.reserve_attempt(PolicyAction::Like);
        let second = policy.reserve_attempt(PolicyAction::Comment);

        policy.cancel_no_effect(first);
        policy.cancel_no_effect(second);

        assert!(policy.attempts.is_empty());
        assert_eq!(policy.last_action_at, None);
        assert!(policy.can_interact_with_post());
    }

    #[test]
    fn reservation_from_another_policy_cannot_cancel_a_local_attempt() {
        let mut source = HumanSessionPolicy::new(100, 100, 100, true);
        source.begin_post();
        let foreign = source.reserve_attempt(PolicyAction::Like);

        let mut target = HumanSessionPolicy::new(100, 100, 100, true);
        target.begin_post();
        let local = target.reserve_attempt(PolicyAction::Like);

        target.cancel_no_effect(foreign);
        assert_eq!(
            target.attempts.len(),
            1,
            "a reservation from another policy must not match the same local id"
        );
        assert_eq!(
            target.recent_posts.back().map(|post| post.interactions),
            Some(1)
        );

        target.cancel_no_effect(local);
        assert!(target.attempts.is_empty());
        assert_eq!(
            target.recent_posts.back().map(|post| post.interactions),
            Some(0)
        );
    }

    #[test]
    fn cancellation_after_feed_advance_refunds_the_original_post() {
        let mut policy = HumanSessionPolicy::new(100, 100, 100, true);
        policy.begin_post();
        let first_post = policy.reserve_attempt(PolicyAction::Like);

        policy.begin_post();
        policy.mark_post_interacted();
        policy.cancel_no_effect(first_post);

        assert_eq!(
            policy
                .recent_posts
                .iter()
                .map(|post| post.interactions)
                .collect::<Vec<_>>(),
            vec![0, 1],
            "the refund must follow its post instead of decrementing the current card"
        );
    }

    #[test]
    fn a_settled_reservation_cannot_be_replayed() {
        let mut policy = HumanSessionPolicy::new(100, 100, 100, true);
        policy.begin_post();
        let reservation = policy.reserve_attempt(PolicyAction::Like);
        let replay = AttemptReservation {
            policy_id: reservation.policy_id,
            attempt_id: reservation.attempt_id,
            post_generation: reservation.post_generation,
        };

        assert!(policy.commit_attempt(reservation));
        assert!(!policy.cancel_no_effect(replay));
        assert_eq!(policy.attempts.len(), 1);
        assert_eq!(
            policy.recent_posts.back().map(|post| post.interactions),
            Some(1)
        );
    }

    #[test]
    fn policy_does_not_schedule_actions_that_are_disabled() {
        let mut policy = HumanSessionPolicy::new(0, 0, 0, true);
        assert!(!policy.can_attempt(PolicyAction::Like));
        assert!(!policy.can_attempt(PolicyAction::Comment));
        assert!(!policy.can_attempt(PolicyAction::Follow));
    }

    #[test]
    fn swipe_duration_profile_stays_inside_human_bounds() {
        let mut behavior = HumanBehavior::new("casual", false, false, true);
        for _ in 0..500 {
            assert!((150..=820).contains(&behavior.swipe_duration_ms(false)));
            assert!((150..=240).contains(&behavior.swipe_duration_ms(true)));
            assert!((280..=760).contains(&behavior.photo_swipe_duration_ms()));
        }
    }

    #[test]
    fn policy_rest_threshold_stays_between_seven_and_thirteen_videos() {
        let mut policy = HumanSessionPolicy::new(0, 0, 0, true);
        let mut since_rest = 0;
        let mut rests = 0;
        for _ in 0..200 {
            since_rest += 1;
            if policy.rest_after_video().is_some() {
                assert!((7..=13).contains(&since_rest));
                since_rest = 0;
                rests += 1;
            }
        }
        assert!(rests >= 10, "policy did not schedule enough rests: {rests}");
    }
}

#[cfg(test)]
mod task4_independent_action_tests {
    use super::*;

    #[test]
    fn zero_and_hundred_percent_are_exact_and_all_actions_keep_order() {
        let none = feed_action_plan_from_rolls(0, 0, 0, 0, Mood::Neutral, [0; 4]);
        assert!(none.ordered().is_empty());

        let all = feed_action_plan_from_rolls(100, 100, 100, 100, Mood::Neutral, [99; 4]);
        assert_eq!(
            all.ordered(),
            vec![
                PolicyAction::Like,
                PolicyAction::Save,
                PolicyAction::Comment,
                PolicyAction::Follow,
            ]
        );
    }

    #[test]
    fn each_roll_is_independent_instead_of_sharing_one_probability_budget() {
        let plan = feed_action_plan_from_rolls(40, 40, 40, 40, Mood::Neutral, [39, 40, 1, 99]);
        assert_eq!(
            plan.ordered(),
            vec![PolicyAction::Like, PolicyAction::Comment]
        );
    }

    #[test]
    fn save_has_its_own_cap_and_noops_refund_without_marking_the_card() {
        let mut policy = HumanSessionPolicy::new_with_save(0, 0, 100, 0, true);
        policy.save_cap = 1;
        policy.begin_post();

        for _ in 0..10 {
            let no_effect = policy.reserve_attempt(PolicyAction::Save);
            assert!(policy.cancel_no_effect(no_effect));
            assert!(policy.can_attempt(PolicyAction::Save));
            assert!(policy.can_interact_with_post());
        }

        let ambiguous = policy.reserve_attempt(PolicyAction::Save);
        assert!(policy.commit_attempt(ambiguous));
        assert!(!policy.can_attempt(PolicyAction::Save));
        assert_eq!(policy.current_post_interactions(), 1);
    }

    #[test]
    fn multiple_actions_on_one_card_use_independent_caps_but_one_density_marker() {
        let mut policy = HumanSessionPolicy::new_with_save(100, 100, 100, 100, true);
        policy.begin_post();
        let prior = policy.reserve_attempt(PolicyAction::Like);
        assert!(policy.commit_attempt(prior));

        policy.begin_post();
        for action in [
            PolicyAction::Like,
            PolicyAction::Save,
            PolicyAction::Comment,
            PolicyAction::Follow,
        ] {
            let reservation = policy.reserve_attempt(action);
            assert!(policy.commit_attempt(reservation));
        }
        assert_eq!(policy.current_post_interactions(), 4);
        assert_eq!(policy.interacted_card_count(), 2);

        policy.begin_post();
        assert!(!policy.can_interact_with_post());
    }
}
