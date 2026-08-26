//! What to do when `TcpListener::accept` returns an error, which both listeners in this app
//! used to answer with a bare `continue`.
//!
//! **A bare `continue` is right for a transient error and catastrophic for a persistent one.**
//! Most `accept` failures are per-connection — the peer went away between the SYN and the
//! accept — and retrying at once is exactly correct. But `EMFILE` and `ENFILE` are not per
//! connection: the process (or the machine) is out of file descriptors, and every retry fails
//! the same way, immediately. That turns the loop into a tight spin that burns a core and
//! writes one log line per iteration, for as long as the condition lasts.
//!
//! File-descriptor exhaustion is a realistic state here rather than a theoretical one: twenty
//! phones times (an adb forward, a scrcpy socket, a control socket) plus one WebSocket per
//! visible tile, on a process that also shells out to `adb` constantly.
//!
//! GenFarmer's survey in `docs/re/genfarmer/README.md` §12.2 describes the general shape —
//! every recovery action gets a windowed cooldown, and log emission to the UI is throttled to
//! 120 ms — precisely to stop a fleet-wide failure from feeding on itself. This is that idea at
//! the smallest scale it applies to.

use std::time::Duration;

/// The longest an accept loop waits before trying again.
///
/// One second, because the loop has to stay responsive: whatever exhausted the descriptors will
/// be released by the tasks that hold them, and the listener has to pick up again promptly when
/// it is. Long enough that a spin becomes one attempt per second instead of thousands.
pub const ACCEPT_BACKOFF_MAX: Duration = Duration::from_secs(1);

/// The first step after the free retry. Doubles from here to [`ACCEPT_BACKOFF_MAX`].
const ACCEPT_BACKOFF_BASE: Duration = Duration::from_millis(10);

/// How many failures in a row are reported in full before the log starts thinning out.
const ACCEPT_REPORT_FIRST: u32 = 3;

/// Once thinned, one line per this many failures. At the cap above that is roughly one line
/// every two minutes, which is enough to show the condition is ongoing without burying the
/// rest of the log.
const ACCEPT_REPORT_EVERY: u32 = 100;

/// What the loop should do about one failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AcceptRetry {
    /// Sleep this long before accepting again. Zero for the first failure.
    pub delay: Duration,
    /// Whether this one is worth a log line.
    pub report: bool,
}

/// Consecutive `accept` failures, and what to do about the next one.
#[derive(Debug, Default, Clone, Copy)]
pub struct AcceptFailures {
    consecutive: u32,
}

impl AcceptFailures {
    /// A connection came through. Whatever was wrong is over.
    pub fn succeeded(&mut self) {
        self.consecutive = 0;
    }

    /// `accept` failed. Says how long to wait and whether to say so.
    pub fn failed(&mut self) -> AcceptRetry {
        self.consecutive = self.consecutive.saturating_add(1);
        AcceptRetry {
            delay: backoff_for(self.consecutive),
            report: self.consecutive <= ACCEPT_REPORT_FIRST
                || self.consecutive.is_multiple_of(ACCEPT_REPORT_EVERY),
        }
    }

    /// How many in a row, for the log line.
    pub fn consecutive(&self) -> u32 {
        self.consecutive
    }
}

/// **The first failure retries immediately, and that is deliberate.**
///
/// A peer that vanished between the SYN and the accept is an ordinary event, and making the
/// next real connection wait 10 ms for it would be a latency cost paid for nothing. The delay
/// only appears once failures start repeating, which is the signal that the cause is the
/// process rather than one client.
fn backoff_for(consecutive: u32) -> Duration {
    match consecutive {
        0 | 1 => Duration::ZERO,
        n => {
            let steps = n - 2;
            let multiplier = 1_u32.checked_shl(steps.min(16)).unwrap_or(u32::MAX);
            ACCEPT_BACKOFF_BASE
                .saturating_mul(multiplier)
                .min(ACCEPT_BACKOFF_MAX)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One failure costs nothing; a run of them backs off to the cap and stays there.
    #[test]
    fn the_first_failure_retries_at_once_and_a_run_of_them_slows_down() {
        let mut failures = AcceptFailures::default();

        assert_eq!(failures.failed().delay, Duration::ZERO, "first is free");
        assert_eq!(failures.failed().delay, ACCEPT_BACKOFF_BASE);
        assert_eq!(failures.failed().delay, ACCEPT_BACKOFF_BASE * 2);
        assert_eq!(failures.failed().delay, ACCEPT_BACKOFF_BASE * 4);

        for _ in 0..40 {
            failures.failed();
        }
        assert_eq!(
            failures.failed().delay,
            ACCEPT_BACKOFF_MAX,
            "a persistent condition must not be retried faster than the cap"
        );
    }

    /// **A tight spin is what this exists to prevent, so the cap has to be a real pause.**
    #[test]
    fn the_cap_turns_a_spin_into_about_one_attempt_a_second() {
        assert!(ACCEPT_BACKOFF_MAX >= Duration::from_millis(500));
        assert!(
            ACCEPT_BACKOFF_MAX <= Duration::from_secs(2),
            "the listener still has to come back promptly once descriptors free up"
        );
    }

    /// A success wipes the slate: the next stray failure is free again.
    #[test]
    fn one_good_connection_resets_the_backoff() {
        let mut failures = AcceptFailures::default();
        for _ in 0..10 {
            failures.failed();
        }
        assert_eq!(failures.consecutive(), 10);

        failures.succeeded();
        assert_eq!(failures.consecutive(), 0);
        assert_eq!(failures.failed().delay, Duration::ZERO);
    }

    /// The log thins out, because the point is a readable log rather than a complete one.
    ///
    /// At the cap that is one line roughly every two minutes: enough to show the condition is
    /// still there, not enough to bury everything else — the same reasoning that cut 83% of one
    /// release log when a tree read was being held to a tap's budget.
    #[test]
    fn a_long_run_of_failures_stops_writing_a_line_each_time() {
        let mut failures = AcceptFailures::default();
        let reported: u32 = (1..=1_000).filter(|_| failures.failed().report).count() as u32;
        assert!(
            reported < 20,
            "a thousand failures produced {reported} lines; the log has to stay readable"
        );
        assert!(reported >= 3, "and the first few must still be visible");
    }

    /// Overflow is not a failure mode. A listener that has been failing for days must not wrap
    /// its counter into a zero delay.
    #[test]
    fn an_absurd_number_of_failures_still_waits_the_cap() {
        let mut failures = AcceptFailures {
            consecutive: u32::MAX - 1,
        };
        assert_eq!(failures.failed().delay, ACCEPT_BACKOFF_MAX);
        assert_eq!(failures.failed().delay, ACCEPT_BACKOFF_MAX);
    }
}
