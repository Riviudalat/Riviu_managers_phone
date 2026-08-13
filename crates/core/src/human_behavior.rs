//! Human-like behavior state machine for TikTok nurture (ported cleanly from TOOL TIKTOK).

use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
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

    pub fn reset_swipe_streak(&mut self) {
        self.consecutive_swipes = 0;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FeedAction {
    Like,
    Comment,
    None,
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

pub fn pick_direction(raw: &str) -> Option<String> {
    let parts: Vec<_> = raw
        .split('|')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .collect();
    if parts.is_empty() {
        return None;
    }
    let mut rng = rand::thread_rng();
    Some(parts[rng.gen_range(0..parts.len())].to_string())
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

/// Internal pacing policy. It is deliberately not exposed as a user setting:
/// configured probabilities still decide whether an action is desired, while
/// this policy keeps the resulting session inside a human-sized rolling rate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PolicyAction {
    Like,
    Comment,
    Follow,
}

/// The gap left between two actions when the human pacing is switched off.
///
/// Not zero, and not a policy either — it is a **settle**. Every action here proves itself
/// by reading the screen back (a like by its label flipping, a comment by the Send button
/// disarming), and firing the next gesture into a screen that is still animating is how a
/// tap lands on whatever slides under it. 800 ms is below anything a human-pacing argument
/// would ask for and above the frame or two a transition needs.
const UNPACED_ACTION_GAP: Duration = Duration::from_millis(800);

#[derive(Debug, Clone)]
pub struct HumanSessionPolicy {
    like_cap: u32,
    comment_cap: u32,
    follow_cap: u32,
    attempts: VecDeque<(Instant, PolicyAction)>,
    last_action_at: Option<Instant>,
    recent_posts: VecDeque<bool>,
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
    /// of 8–16 likes / 1–3 comments / 1–2 follows; a rule that at most **two of the last
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
            like_cap: cap(like_prob, 8, 16, &mut rng),
            comment_cap: cap(comment_prob, 1, 3, &mut rng),
            follow_cap: cap(follow_prob, 1, 2, &mut rng),
            attempts: VecDeque::new(),
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
        self.limits = limits;
        Self::retune_cap(&mut self.like_cap, like_prob, 8, 16, &mut self.rng);
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
        self.recent_posts.push_back(false);
    }

    pub fn can_interact_with_post(&self) -> bool {
        if !self.limits {
            return true;
        }
        self.recent_posts.iter().filter(|&&used| used).count() < 2
    }

    pub fn mark_post_interacted(&mut self) {
        if let Some(last) = self.recent_posts.back_mut() {
            *last = true;
        }
    }

    fn prune(&mut self, now: Instant) {
        let window = Duration::from_secs(60 * 60);
        while self
            .attempts
            .front()
            .is_some_and(|(at, _)| now.duration_since(*at) >= window)
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
            PolicyAction::Comment => self.comment_cap,
            PolicyAction::Follow => self.follow_cap,
        };
        if cap == 0 {
            return false;
        }
        self.attempts
            .iter()
            .filter(|(_, kind)| *kind == action)
            .count()
            < cap as usize
    }

    /// Record an attempt before the gesture. A failed confirmation still
    /// consumes this slot, preventing retries from becoming a burst.
    pub fn record_attempt(&mut self, action: PolicyAction) {
        let now = Instant::now();
        self.prune(now);
        self.attempts.push_back((now, action));
        self.last_action_at = Some(now);
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
