//! Bounded latency books for adb lanes and agent routes, summarised once a minute.
//!
//! **Why this exists.** Before it, the only numbers were warnings: adb printed one line per
//! wait past 500 ms, labelled as the global cap even when the wait was the phone's own queue
//! or the transfer sub-cap, and the agent printed only calls past its slow budget. Percentiles
//! built from slow-only lines are the tail's percentiles, not the route's, so a host-side
//! queue and a slow phone could not be told apart (review F4, 14/10/2026). Every call is
//! counted here instead -- fast, slow, timed out, failed and cancelled -- per lane or route.
//!
//! **Bounded by construction.** A fixed bucket array per series, at most [`MAX_SERIES`]
//! series per window plus one overflow series per family, names normalised and truncated.
//! Nothing else is stored: no serial, no payload, no URL, no session id.
//!
//! The summary is emitted by the first record after the window closes, so a window with no
//! traffic prints nothing, and the window is reset after each summary so consecutive lines
//! compare runs rather than accumulate them.

use std::sync::OnceLock;
use std::time::{Duration, Instant};

/// Upper bounds of the histogram buckets, in milliseconds; one more bucket is open-ended.
///
/// Dense under one second where adb round trips and taps live, then spaced to resolve the
/// tree-read tail measured on the 30-phone fleet (r105/r107, 13-14/10/2026: `/source` p90
/// 12.7 s, header waits of 15.5 s).
const BUCKET_BOUNDS_MS: [u64; 21] = [
    5, 10, 20, 50, 100, 200, 350, 500, 750, 1_000, 1_500, 2_000, 3_000, 5_000, 8_000, 12_000,
    16_000, 20_000, 30_000, 60_000, 120_000,
];
const BUCKETS: usize = BUCKET_BOUNDS_MS.len() + 1;

/// Distinct named series kept in one window before new names fold into the overflow series.
///
/// Not measured; sized from what the code can produce: four adb lanes, ~20 adb verbs and
/// ~25 normalised agent routes fit with room to spare.
pub(crate) const MAX_SERIES: usize = 64;

/// The series new names fold into once [`MAX_SERIES`] is reached.
pub(crate) const OVERFLOW_NAME: &str = "(khác)";

/// Longest series name kept, in characters.
const MAX_NAME_CHARS: usize = 48;

/// How often the summary line is printed while there is traffic.
const SUMMARY_EVERY: Duration = Duration::from_secs(60);

/// Families, in the order the summary prints them.
pub(crate) const ADB_SERIAL_QUEUE: &str = "adb.serial_queue";
pub(crate) const ADB_TRANSFER_CAP: &str = "adb.transfer_cap";
pub(crate) const ADB_GLOBAL_SLOT: &str = "adb.global_slot";
pub(crate) const ADB_EXEC: &str = "adb.exec";
pub(crate) const AGENT: &str = "agent";
const FAMILY_ORDER: [&str; 5] = [
    ADB_SERIAL_QUEUE,
    ADB_TRANSFER_CAP,
    ADB_GLOBAL_SLOT,
    ADB_EXEC,
    AGENT,
];

/// How one call ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Outcome {
    /// Answered within its slow budget (or the wait completed).
    Ok,
    /// Answered, but past its slow budget.
    Slow,
    /// Its own deadline expired.
    Timeout,
    /// Failed for any other reason: transport, spawn, non-zero adb exit.
    Error,
    /// Dropped before it ended -- the caller's future was cancelled.
    Cancel,
}

impl Outcome {
    const ALL: [Outcome; 5] = [
        Outcome::Ok,
        Outcome::Slow,
        Outcome::Timeout,
        Outcome::Error,
        Outcome::Cancel,
    ];

    fn index(self) -> usize {
        match self {
            Outcome::Ok => 0,
            Outcome::Slow => 1,
            Outcome::Timeout => 2,
            Outcome::Error => 3,
            Outcome::Cancel => 4,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Outcome::Ok => "ok",
            Outcome::Slow => "chậm",
            Outcome::Timeout => "hết_giờ",
            Outcome::Error => "lỗi",
            Outcome::Cancel => "hủy",
        }
    }
}

/// Collapse anything that looks like an identifier, so one route is one series.
///
/// A path segment survives only if it is short and made of lowercase letters, `_` or `-`;
/// anything else (element UUIDs, numbers, ports) becomes `:id`. Queries are dropped. An HTTP
/// method before the first space is kept. This is both the cardinality bound and the reason
/// no identifier ever reaches a log line.
pub(crate) fn normalise_name(raw: &str) -> String {
    let path = raw.split(['?', '#']).next().unwrap_or_default();
    let (method, path) = match path.split_once(' ') {
        Some((method, rest))
            if !method.is_empty()
                && method.len() <= 7
                && method.chars().all(|c| c.is_ascii_uppercase()) =>
        {
            (Some(method), rest)
        }
        _ => (None, path),
    };
    let path = path
        .split('/')
        .map(|segment| {
            let plain = !segment.is_empty()
                && segment.len() <= 32
                && segment
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c == '_' || c == '-');
            if segment.is_empty() || plain {
                segment
            } else {
                ":id"
            }
        })
        .collect::<Vec<_>>()
        .join("/");
    let normalised = match method {
        Some(method) => format!("{method} {path}"),
        None => path,
    };
    normalised.chars().take(MAX_NAME_CHARS).collect()
}

#[derive(Debug, Clone)]
struct Series {
    family: &'static str,
    name: String,
    buckets: [u32; BUCKETS],
    outcomes: [u32; Outcome::ALL.len()],
    max_ms: u64,
}

impl Series {
    fn new(family: &'static str, name: String) -> Self {
        Self {
            family,
            name,
            buckets: [0; BUCKETS],
            outcomes: [0; Outcome::ALL.len()],
            max_ms: 0,
        }
    }

    fn count(&self) -> u64 {
        self.buckets.iter().map(|count| u64::from(*count)).sum()
    }

    /// The upper bound of the bucket holding the `q` quantile, never above the observed max.
    fn quantile_ms(&self, q: f64) -> u64 {
        let total = self.count();
        if total == 0 {
            return 0;
        }
        let rank = ((q * total as f64).ceil() as u64).clamp(1, total);
        let mut seen = 0u64;
        for (index, count) in self.buckets.iter().enumerate() {
            seen += u64::from(*count);
            if seen >= rank {
                let bound = BUCKET_BOUNDS_MS.get(index).copied().unwrap_or(self.max_ms);
                return bound.min(self.max_ms);
            }
        }
        self.max_ms
    }
}

/// One window of series. Pure apart from the clock the caller passes in.
#[derive(Debug)]
pub(crate) struct LatencyBook {
    window_started: Option<Instant>,
    series: Vec<Series>,
}

/// A closed window, ready to print.
#[derive(Debug)]
pub(crate) struct Summary {
    window: Duration,
    series: Vec<Series>,
}

impl LatencyBook {
    pub(crate) const fn new() -> Self {
        Self {
            window_started: None,
            series: Vec::new(),
        }
    }

    /// Count one call; returns the closed window when this record ends it.
    pub(crate) fn record(
        &mut self,
        family: &'static str,
        name: &str,
        elapsed: Duration,
        outcome: Outcome,
        now: Instant,
    ) -> Option<Summary> {
        let started = *self.window_started.get_or_insert(now);
        let name = normalise_name(name);
        let index = match self
            .series
            .iter()
            .position(|series| series.family == family && series.name == name)
        {
            Some(index) => index,
            None => {
                let name = if self.named_series() < MAX_SERIES {
                    name
                } else {
                    OVERFLOW_NAME.to_string()
                };
                match self
                    .series
                    .iter()
                    .position(|series| series.family == family && series.name == name)
                {
                    Some(index) => index,
                    None => {
                        self.series.push(Series::new(family, name));
                        self.series.len() - 1
                    }
                }
            }
        };
        let ms = u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX);
        let bucket = BUCKET_BOUNDS_MS
            .iter()
            .position(|bound| ms <= *bound)
            .unwrap_or(BUCKETS - 1);
        if let Some(series) = self.series.get_mut(index) {
            series.buckets[bucket] = series.buckets[bucket].saturating_add(1);
            series.outcomes[outcome.index()] = series.outcomes[outcome.index()].saturating_add(1);
            series.max_ms = series.max_ms.max(ms);
        }

        let window = now.saturating_duration_since(started);
        if window < SUMMARY_EVERY {
            return None;
        }
        self.window_started = Some(now);
        Some(Summary {
            window,
            series: std::mem::take(&mut self.series),
        })
    }

    fn named_series(&self) -> usize {
        self.series
            .iter()
            .filter(|series| series.name != OVERFLOW_NAME)
            .count()
    }

    #[cfg(test)]
    fn series_len(&self) -> usize {
        self.series.len()
    }

    #[cfg(test)]
    fn total(&self) -> u64 {
        self.series.iter().map(Series::count).sum()
    }
}

impl Summary {
    /// One line for the operator log: per lane/route `n`, p50, p90, max and non-ok outcomes.
    pub(crate) fn render(&self) -> String {
        let mut series: Vec<&Series> = self.series.iter().collect();
        series.sort_by_key(|series| {
            let rank = FAMILY_ORDER
                .iter()
                .position(|family| *family == series.family)
                .unwrap_or(FAMILY_ORDER.len());
            (rank, std::cmp::Reverse(series.count()), series.name.clone())
        });
        series
            .iter()
            .map(|series| {
                let mut line = if series.name.is_empty() {
                    series.family.to_string()
                } else {
                    format!("{} {}", series.family, series.name)
                };
                line.push_str(&format!(
                    ": n={} p50≤{}ms p90≤{}ms max={}ms",
                    series.count(),
                    series.quantile_ms(0.5),
                    series.quantile_ms(0.9),
                    series.max_ms
                ));
                for outcome in Outcome::ALL.iter().skip(1) {
                    let count = series.outcomes[outcome.index()];
                    if count > 0 {
                        line.push_str(&format!(" {}={count}", outcome.label()));
                    }
                }
                line
            })
            .collect::<Vec<_>>()
            .join("; ")
    }
}

fn book() -> &'static parking_lot::Mutex<LatencyBook> {
    static BOOK: OnceLock<parking_lot::Mutex<LatencyBook>> = OnceLock::new();
    BOOK.get_or_init(|| parking_lot::Mutex::new(LatencyBook::new()))
}

/// Count one call in the process-wide book, printing the summary when a window closes.
pub(crate) fn record(family: &'static str, name: &str, elapsed: Duration, outcome: Outcome) {
    let summary = book()
        .lock()
        .record(family, name, elapsed, outcome, Instant::now());
    // Formatted and logged outside the lock: the book is on every adb and agent call.
    if let Some(summary) = summary {
        tracing::info!(
            window_s = summary.window.as_secs(),
            series = summary.series.len(),
            "Độ trễ adb/agent {} s gần nhất (p50/p90 là cận trên ô histogram): {}",
            summary.window.as_secs(),
            summary.render()
        );
    }
}

/// Times one call from creation to drop; a call dropped before it ends counts as cancelled.
pub(crate) struct CallTimer {
    family: &'static str,
    name: String,
    slow_after: Option<Duration>,
    started: Instant,
    ended: Option<(Duration, Outcome)>,
}

impl CallTimer {
    /// `slow_after` splits answered calls into ok and slow; `None` never calls one slow.
    pub(crate) fn start(family: &'static str, name: &str, slow_after: Option<Duration>) -> Self {
        Self {
            family,
            name: normalise_name(name),
            slow_after,
            started: Instant::now(),
            ended: None,
        }
    }

    fn end(&mut self, outcome: Outcome) {
        if self.ended.is_none() {
            self.ended = Some((self.started.elapsed(), outcome));
        }
    }

    /// The far side answered; ok or slow by the budget.
    pub(crate) fn answered(&mut self) {
        let elapsed = self.started.elapsed();
        let slow = self.slow_after.is_some_and(|budget| elapsed >= budget);
        self.end(if slow { Outcome::Slow } else { Outcome::Ok });
    }

    pub(crate) fn timed_out(&mut self) {
        self.end(Outcome::Timeout);
    }

    pub(crate) fn failed(&mut self) {
        self.end(Outcome::Error);
    }
}

impl Drop for CallTimer {
    fn drop(&mut self) {
        let (elapsed, outcome) = self
            .ended
            .unwrap_or_else(|| (self.started.elapsed(), Outcome::Cancel));
        record(self.family, &self.name, elapsed, outcome);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Element UUIDs and numbers fold into one series; nothing identifying survives.
    #[test]
    fn identifiers_in_a_route_collapse_to_one_name() {
        assert_eq!(
            normalise_name("/element/5f0c1a2e-0000-4b1c-9a8e-1234567890ab/attribute/text"),
            "/element/:id/attribute/text"
        );
        assert_eq!(normalise_name("/source"), "/source");
        // The agent names its series `<METHOD> <route>`; the method survives, ids do not.
        assert_eq!(normalise_name("GET /source"), "GET /source");
        assert_eq!(
            normalise_name("GET /element/5f0c1a2e-0000-4b1c-9a8e-1234567890ab/rect"),
            "GET /element/:id/rect"
        );
        assert_eq!(normalise_name("get me/source"), ":id/source");
        assert_eq!(
            normalise_name("/window/current/size?x=1"),
            "/window/current/size"
        );
        assert_eq!(normalise_name("exec-out"), "exec-out");
        assert_eq!(normalise_name("ce0717171c2a64d50d"), ":id");
        assert!(normalise_name(&"/abc".repeat(40)).chars().count() <= MAX_NAME_CHARS);
    }

    /// However many names arrive, a window holds at most `MAX_SERIES` named series plus one
    /// overflow series per family, and no call goes uncounted.
    #[test]
    fn a_window_keeps_a_bounded_number_of_series_and_counts_every_call() {
        let mut book = LatencyBook::new();
        let now = Instant::now();
        for index in 0..1_000 {
            // Letters only, so normalisation does not merge them: the cap has to.
            let name: String = format!("{index:04}")
                .chars()
                .map(|digit| char::from(b'a' + digit as u8 - b'0'))
                .collect();
            for family in [ADB_EXEC, AGENT] {
                assert!(book
                    .record(family, &name, Duration::from_millis(7), Outcome::Ok, now)
                    .is_none());
            }
        }
        assert_eq!(book.series_len(), MAX_SERIES + 2);
        assert_eq!(book.total(), 2_000);
    }

    /// p50/p90 are bucket bounds clipped to the observed max; every outcome is counted.
    #[test]
    fn the_summary_reports_quantiles_and_every_outcome_then_resets() {
        let mut book = LatencyBook::new();
        let start = Instant::now();
        for ms in [40u64, 40, 40, 40, 40, 40, 40, 40, 40] {
            book.record(
                AGENT,
                "/source",
                Duration::from_millis(ms),
                Outcome::Ok,
                start,
            );
        }
        book.record(
            AGENT,
            "/source",
            Duration::from_millis(15_500),
            Outcome::Timeout,
            start,
        );
        book.record(
            AGENT,
            "/actions",
            Duration::from_millis(3),
            Outcome::Cancel,
            start,
        );
        book.record(
            ADB_SERIAL_QUEUE,
            "",
            Duration::from_millis(900),
            Outcome::Ok,
            start,
        );

        let summary = book
            .record(
                ADB_GLOBAL_SLOT,
                "",
                Duration::ZERO,
                Outcome::Ok,
                start + SUMMARY_EVERY,
            )
            .expect("the window closed");
        let line = summary.render();
        assert_eq!(
            line,
            "adb.serial_queue: n=1 p50≤900ms p90≤900ms max=900ms; \
             adb.global_slot: n=1 p50≤0ms p90≤0ms max=0ms; \
             agent /source: n=10 p50≤50ms p90≤50ms max=15500ms hết_giờ=1; \
             agent /actions: n=1 p50≤3ms p90≤3ms max=3ms hủy=1"
        );
        assert_eq!(book.series_len(), 0, "the next window starts empty");
    }
}
