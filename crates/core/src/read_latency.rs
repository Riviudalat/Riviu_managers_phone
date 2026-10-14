//! Per-phone tree-read latency and the one function that sizes a pre-Post observation window
//! from it.
//!
//! **Why this exists.** The composer's observation windows were constants chosen on idle
//! phones. Run r111 (`9e5e7340`, 14/10/2026, 28 phones, 10 phone jobs at once, QuietView)
//! reached Post on 2 of 68 compose attempts in its first 23 minutes: almost every row stopped
//! on `publish_observation_deadline` at transfer/media/device/prePost, while the agent's
//! `GET /source` ran p50 1.5 s, p90 3-8 s, max 14-17 s and the host's adb slots stayed under
//! 750 ms at p90 (`controller-r111.stdout.txt`, `riviu_android_driver::telemetry`). A step
//! that needs three to six reads cannot finish inside 8-10 s when one read takes 1.5-3 s.
//!
//! **What it does.** The Android agent client records every completed `/source` read and
//! `/element`/`/elements` find against the phone's serial ([`record_tree_read`]); r112/r113
//! (14/10/2026) showed steps dying on a slow find as often as on a slow `/source`. [`window`]
//! is computed once when a step starts:
//! `clamp(max(base, reads × q90 + margin), base, ceiling)`, with the ceiling at most four
//! times the base and never above [`WINDOW_CEILING`]. Every caller still bounds the result by
//! the enclosing sound/compose budget (`phase_deadline`), so a wider step never outlives the
//! stage it belongs to. The window is never extended while the step runs; only the one read
//! still in flight when it closes may be finished by the read grace
//! ([`crate::ui_automation::runtime::READ_GRACE`]).
//!
//! **What it never does.** It is for reads that wait for a screen. It never sizes the fresh
//! proof before Post (`final_reproof_window`), a gesture or an effect, and it never turns an
//! unread screen into a tap: an expired window still ends with the same error code. After
//! Post it sizes only the wait for the feed (`postConfirm`), which observes and never
//! repeats Post: an expired wait is still `PostNotConfirmed`.
//!
//! **Bounded by construction.** At most [`MAX_SERIALS`] phones, [`SAMPLES_PER_SERIAL`] reads
//! each, and reads older than [`SAMPLE_MAX_AGE`] are ignored, so a phone that was slow an
//! hour ago is sized by what it does now.

use std::collections::{HashMap, VecDeque};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

/// Recent completed reads kept per phone.
///
/// Not measured; sized so one q90 rests on enough reads to mean something: a compose step
/// makes three to six reads, so 32 reads span the last several steps of the same attempt.
const SAMPLES_PER_SERIAL: usize = 32;
/// Fewest recent reads before the window trusts the phone's q90; below it, the base window.
///
/// Not measured: with fewer than five reads the 90th percentile is just the maximum, and one
/// cold first read (instrumentation warm-up) would widen every step of the attempt.
const MIN_SAMPLES: usize = 5;
/// Reads older than this no longer describe the phone.
///
/// Not measured; ten minutes is about two compose attempts at r111's pace, long enough to
/// carry a phone's load from one attempt into its retry and short enough that a fleet that
/// has gone quiet returns to the idle-measured windows.
const SAMPLE_MAX_AGE: Duration = Duration::from_secs(600);
/// Phones remembered at once; the least recently read is forgotten first.
///
/// Not measured; the largest fleet on one controller so far is 50 phones (Riviu 1-50).
const MAX_SERIALS: usize = 128;
/// Added to the reads' total for the work between them: the step's poll sleeps, the popup
/// check's decode and the tap's settle.
///
/// Not measured as such. `POLL` is 350 ms and a step polls between two and four times, so
/// two seconds covers the sleeps of a step whose reads alone already fit.
const SETTLE_MARGIN: Duration = Duration::from_secs(2);
/// No step window grows past this, whatever the phone measures.
///
/// r111's longest completed `/source` was 14-17 s; a step of five such reads would need more
/// than a minute, and a phone that slow is better retried than waited on. 90 s keeps the
/// whole compose inside its 300 s sound stage and the assignment's retry budget.
pub const WINDOW_CEILING: Duration = Duration::from_secs(90);
/// No step window grows past this many times its base.
///
/// A step's base was measured on idle phones; four times it is still finite and below
/// [`WINDOW_CEILING`] for every base the composer uses (2-20 s).
const CEILING_FACTOR: u32 = 4;

#[derive(Default)]
struct Book {
    serials: HashMap<String, Series>,
}

struct Series {
    reads: VecDeque<(Instant, Duration)>,
    last: Instant,
}

impl Book {
    fn record(&mut self, serial: &str, elapsed: Duration, now: Instant) {
        if !self.serials.contains_key(serial) && self.serials.len() >= MAX_SERIALS {
            if let Some(oldest) = self
                .serials
                .iter()
                .min_by_key(|(_, series)| series.last)
                .map(|(serial, _)| serial.clone())
            {
                self.serials.remove(&oldest);
            }
        }
        let series = self
            .serials
            .entry(serial.to_owned())
            .or_insert_with(|| Series {
                reads: VecDeque::with_capacity(SAMPLES_PER_SERIAL),
                last: now,
            });
        if series.reads.len() >= SAMPLES_PER_SERIAL {
            series.reads.pop_front();
        }
        series.reads.push_back((now, elapsed));
        series.last = now;
    }

    /// Nearest-rank 90th percentile of the phone's recent completed reads.
    fn q90(&self, serial: &str, now: Instant) -> Option<Duration> {
        let series = self.serials.get(serial)?;
        let mut recent: Vec<Duration> = series
            .reads
            .iter()
            .filter(|(at, _)| now.saturating_duration_since(*at) <= SAMPLE_MAX_AGE)
            .map(|(_, elapsed)| *elapsed)
            .collect();
        if recent.len() < MIN_SAMPLES {
            return None;
        }
        recent.sort_unstable();
        let rank = (recent.len() * 9).div_ceil(10).clamp(1, recent.len());
        recent.get(rank - 1).copied()
    }
}

fn book() -> &'static parking_lot::Mutex<Book> {
    static BOOK: OnceLock<parking_lot::Mutex<Book>> = OnceLock::new();
    BOOK.get_or_init(|| parking_lot::Mutex::new(Book::default()))
}

/// Count one tree read (`GET /source`, or a `POST /element`/`/elements` find) on `serial`.
///
/// Completed reads, plus reads cut off after at least five seconds as a lower bound (the
/// Android agent's `UnansweredTreeRead`): r122/r123 (15/10/2026) showed every 12-30 s read
/// ending interrupted, which left the book holding only fast reads. A read cut off sooner
/// measured its deadline, not the phone, and is not counted.
pub fn record_tree_read(serial: &str, elapsed: Duration) {
    if serial.is_empty() {
        return;
    }
    book().lock().record(serial, elapsed, Instant::now());
}

/// The phone's recent tree-read q90, if it has enough recent reads to say.
pub fn tree_read_q90(serial: &str) -> Option<Duration> {
    book().lock().q90(serial, Instant::now())
}

/// The finite ceiling for a step whose idle-measured window is `base`.
pub fn ceiling(base: Duration) -> Duration {
    base.saturating_mul(CEILING_FACTOR)
        .min(WINDOW_CEILING)
        .max(base)
}

/// `clamp(max(base, reads × q90 + margin), base, ceiling(base))`; `base` without a q90.
pub fn sized(base: Duration, expected_reads: u32, q90: Option<Duration>) -> Duration {
    let Some(q90) = q90 else {
        return base;
    };
    let needed = q90
        .saturating_mul(expected_reads)
        .saturating_add(SETTLE_MARGIN);
    needed.max(base).min(ceiling(base))
}

/// The window for one observation step that expects `expected_reads` tree reads on the phone
/// this task is publishing from. Computed once at the start of the step; callers still bound
/// it by the enclosing stage budget. Logs one line when it is wider than `base`.
pub fn window(step: &'static str, base: Duration, expected_reads: u32) -> Duration {
    let Some(serial) = crate::tiktok_composer::stage_udid() else {
        return base;
    };
    window_for(step, base, expected_reads, &serial)
}

/// [`window`] for an explicit phone.
pub fn window_for(
    step: &'static str,
    base: Duration,
    expected_reads: u32,
    serial: &str,
) -> Duration {
    let q90 = tree_read_q90(serial);
    let window = sized(base, expected_reads, q90);
    if window > base {
        tracing::info!(
            serial = %serial,
            step,
            base_ms = base.as_millis() as u64,
            q90_ms = q90.map(|q90| q90.as_millis() as u64),
            reads = expected_reads,
            window_ms = window.as_millis() as u64,
            "Nới cửa sổ chờ đọc màn hình theo độ trễ đo được của máy"
        );
    }
    window
}

#[cfg(test)]
mod tests {
    use super::*;

    /// q90 needs enough recent reads, forgets stale ones, and memory stays bounded.
    #[test]
    fn the_book_reports_a_recent_q90_and_stays_bounded() {
        let mut book = Book::default();
        let start = Instant::now();
        for ms in [100u64, 200, 300, 400] {
            book.record("a", Duration::from_millis(ms), start);
        }
        assert_eq!(book.q90("a", start), None, "four reads are not enough");
        for ms in 500..=1_000u64 {
            book.record("a", Duration::from_millis(ms), start);
        }
        assert_eq!(book.serials["a"].reads.len(), SAMPLES_PER_SERIAL);
        // The last 32 reads are 969..=1000 ms; their nearest-rank q90 is the 29th.
        assert_eq!(book.q90("a", start), Some(Duration::from_millis(997)));
        assert_eq!(
            book.q90("a", start + SAMPLE_MAX_AGE + Duration::from_secs(1)),
            None
        );

        for index in 0..(MAX_SERIALS + 10) {
            book.record(&format!("s{index}"), Duration::from_millis(1), start);
        }
        assert_eq!(book.serials.len(), MAX_SERIALS);
    }

    /// The window grows with the phone's reads, never below the base or past the ceiling.
    #[test]
    fn the_window_is_clamped_between_base_and_a_finite_ceiling() {
        let base = Duration::from_secs(10);
        assert_eq!(sized(base, 4, None), base);
        assert_eq!(sized(base, 4, Some(Duration::from_millis(500))), base);
        assert_eq!(
            sized(base, 4, Some(Duration::from_secs(3))),
            Duration::from_secs(14)
        );
        assert_eq!(
            sized(base, 4, Some(Duration::from_secs(30))),
            Duration::from_secs(40)
        );
        assert_eq!(
            sized(Duration::from_secs(30), 6, Some(Duration::from_secs(30))),
            WINDOW_CEILING
        );
    }
}
