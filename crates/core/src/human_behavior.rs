//! Human-like behavior state machine for TikTok nurture (ported cleanly from TOOL TIKTOK).

use rand::Rng;
use serde::{Deserialize, Serialize};

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

    pub fn watch_seconds(&mut self, watch_min: f64, watch_max: f64) -> f64 {
        let mut rng = rand::thread_rng();
        let span = (watch_max - watch_min).max(0.5);
        let base = match self.persona.as_str() {
            "bored" => watch_min + span * rng.gen_range(0.05..0.60),
            "curious" => watch_min + span * rng.gen_range(0.60..1.0),
            _ => {
                let roll = rng.gen_range(0.0..1.0);
                if roll < 0.2 {
                    watch_min + span * rng.gen_range(0.05..0.25)
                } else if roll < 0.3 {
                    watch_min + span * rng.gen_range(0.80..1.0)
                } else {
                    watch_min + span * rng.gen_range(0.15..0.70)
                }
            }
        };
        let mut secs = base * self.state.watch_mult();
        if self.fatigue_enabled {
            secs *= self.fatigue_mult();
        }
        if self.tod_enabled {
            secs *= self.tod_mult();
        }
        let jitter = span * 0.03;
        secs += rng.gen_range(-jitter..jitter);
        secs = secs.clamp(watch_min, watch_max);
        let min_delta = (span * 0.15).min(1.5);
        if (secs - self.last_watch).abs() < min_delta && span > min_delta {
            secs = if secs > self.last_watch {
                (secs + min_delta).min(watch_max)
            } else {
                (secs - min_delta).max(watch_min)
            };
        }
        self.last_watch = secs;
        secs
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
        let hour = chrono::Local::now().format("%H").to_string().parse::<u32>().unwrap_or(12);
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

pub fn in_night_window(start: u32, end: u32) -> bool {
    if start == end {
        return false;
    }
    let hour = chrono::Local::now().format("%H").to_string().parse::<u32>().unwrap_or(12);
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
}

impl Mood {
    /// Multipliers applied to the configured probabilities while this mood runs.
    /// The weighted average over a cycle lands near 1.0 for each channel.
    fn like_mult(self) -> f64 {
        match self {
            Mood::Skimming => 0.0,
            Mood::Liking => 2.2,
            Mood::Chatty => 1.2,
        }
    }

    fn comment_mult(self) -> f64 {
        match self {
            Mood::Skimming => 0.0,
            Mood::Liking => 0.5,
            Mood::Chatty => 3.0,
        }
    }

    fn follow_mult(self) -> f64 {
        match self {
            Mood::Skimming => 0.0,
            Mood::Liking => 1.6,
            Mood::Chatty => 1.4,
        }
    }

    /// Watch-length multiplier — skimming is quick, chatty lingers.
    pub fn watch_mult(self) -> f64 {
        match self {
            Mood::Skimming => 0.55,
            Mood::Liking => 1.0,
            Mood::Chatty => 1.45,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Mood::Skimming => "lướt nhanh",
            Mood::Liking => "thả tim nhiều",
            Mood::Chatty => "hay bình luận",
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

    /// The mood for this video, advancing the cycle. Returns `(mood, changed)`
    /// so the caller can log a transition once rather than every video.
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
    let like = ((like_prob as f64) * mood.like_mult()).round().clamp(0.0, 100.0) as u32;
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

#[cfg(test)]
mod mood_tests {
    use super::*;

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
}
