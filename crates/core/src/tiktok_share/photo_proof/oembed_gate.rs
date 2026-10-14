//! Process-wide pacing for TikTok's public oEmbed endpoint.
//!
//! Measured 14/10/2026 13:45-14:10 from one Windows host driving 20 phones: the TikTok
//! home page answered 200 in 0.3 s and short links 301 at once, while `/oembed` hung
//! 15-20 s or answered 429/503, with an occasional 200 on the same URL. Every verifying
//! row called on its own 2-minute cadence and manual checks added more, so the limit is
//! per host IP and the host, not each row, has to pace itself.
use std::sync::LazyLock;
use rand::Rng;
use std::time::Duration;
use tokio::sync::{Mutex, MutexGuard};
use tokio::time::Instant;

/// Minimum spacing between request starts. 20 phones' rows on the 2-minute cadence fit
/// in 100 s at this pace, without the bursts of every row calling on its own.
pub(super) const OEMBED_MIN_SPACING: Duration = Duration::from_secs(5);
/// First pause after TikTok throttles; it doubles with each throttled answer in a row.
pub(super) const OEMBED_BACKOFF_BASE: Duration = Duration::from_secs(30);
/// A cap on the backoff and on any `Retry-After`, so the host always probes again.
pub(super) const OEMBED_BACKOFF_CAP: Duration = Duration::from_secs(600);
/// Longest one attempt waits for its turn before it reports TikTok unavailable, on top
/// of the 15 s request timeout. The phone-side capture still holds its lease here.
pub(super) const OEMBED_MAX_TURN_WAIT: Duration = Duration::from_secs(20);

static OEMBED_GATE: LazyLock<OembedGate> = LazyLock::new(OembedGate::new);

pub(super) fn shared() -> &'static OembedGate {
    &OEMBED_GATE
}

/// What TikTok's answer says about its load, separate from the proof result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum OembedOutcome {
    /// TikTok answered, whatever the content; a 4xx other than 429 is an answer.
    Answered,
    /// 429, 5xx or no complete answer (timeout, connect failure, body cut off).
    Throttled { retry_after: Option<Duration> },
}

/// The next allowed start lies beyond the caller's wait; no request was sent.
#[derive(Debug, thiserror::Error)]
#[error("oEmbed turn not available within the caller's wait")]
pub(super) struct TurnUnavailable;

struct GateState {
    next_start: Option<Instant>,
    throttled_streak: u32,
}

pub(super) struct OembedGate {
    state: Mutex<GateState>,
}

/// Holding a turn is holding the only request in flight. Dropping it unfinished, as a
/// cancelled caller does, leaves the spacing in place and the backoff unchanged.
pub(super) struct OembedTurn<'a> {
    state: MutexGuard<'a, GateState>,
}

impl OembedGate {
    fn new() -> Self {
        Self {
            state: Mutex::new(GateState {
                next_start: None,
                throttled_streak: 0,
            }),
        }
    }

    /// Waits in FIFO order for the single request slot and the spacing or backoff.
    /// Cancel-safe: dropping the future gives up the place in line.
    pub(super) async fn turn(&self, max_wait: Duration) -> Result<OembedTurn<'_>, TurnUnavailable> {
        let deadline = Instant::now() + max_wait;
        let mut state = tokio::time::timeout_at(deadline, self.state.lock())
            .await
            .map_err(|_| TurnUnavailable)?;
        if let Some(next) = state.next_start {
            if next > deadline {
                return Err(TurnUnavailable);
            }
            tokio::time::sleep_until(next).await;
        }
        state.next_start = Some(Instant::now() + OEMBED_MIN_SPACING);
        Ok(OembedTurn { state })
    }
}

impl OembedTurn<'_> {
    pub(super) fn finish(mut self, outcome: OembedOutcome) {
        match outcome {
            // The measured throttle let an occasional 200 through, so one answer
            // steps the backoff down instead of clearing it.
            OembedOutcome::Answered => {
                self.state.throttled_streak = self.state.throttled_streak.saturating_sub(1);
            }
            OembedOutcome::Throttled { retry_after } => {
                self.state.throttled_streak = self.state.throttled_streak.saturating_add(1);
                let exponent = self.state.throttled_streak.saturating_sub(1).min(8);
                let jitter = rand::thread_rng().gen_range(0.8..=1.2);
                let backoff = OEMBED_BACKOFF_BASE.saturating_mul(1 << exponent).mul_f64(jitter);
                let pause = backoff.max(retry_after.unwrap_or_default()).min(OEMBED_BACKOFF_CAP);
                let until = Instant::now() + pause;
                self.state.next_start = Some(self.state.next_start.map_or(until, |next| next.max(until)));
                tracing::warn!(
                    "Kiểm tra công khai TikTok (oEmbed) bị giới hạn tần suất hoặc không phản hồi; tạm dừng gửi {} giây",
                    pause.as_secs()
                );
            }
        }
    }
}

/// `Retry-After` as delay-seconds or an HTTP date; the gate applies its own cap.
pub(super) fn retry_after(
    headers: &reqwest::header::HeaderMap,
    now: chrono::DateTime<chrono::Utc>,
) -> Option<Duration> {
    let value = headers.get(reqwest::header::RETRY_AFTER)?.to_str().ok()?.trim();
    if let Ok(seconds) = value.parse::<u64>() {
        return Some(Duration::from_secs(seconds));
    }
    let at = chrono::DateTime::parse_from_rfc2822(value).ok()?;
    (at.with_timezone(&chrono::Utc) - now).to_std().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(start_paused = true)]
    async fn concurrent_callers_never_overlap_and_keep_the_spacing() {
        let gate = std::sync::Arc::new(OembedGate::new());
        let spans = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let callers = (0..2)
            .map(|_| {
                let (gate, spans) = (gate.clone(), spans.clone());
                tokio::spawn(async move {
                    let turn = gate.turn(OEMBED_MAX_TURN_WAIT).await.unwrap();
                    let started = Instant::now();
                    // The request in flight.
                    tokio::time::sleep(Duration::from_secs(1)).await;
                    spans.lock().unwrap().push((started, Instant::now()));
                    turn.finish(OembedOutcome::Answered);
                })
            })
            .collect::<Vec<_>>();
        for caller in callers {
            caller.await.unwrap();
        }
        let mut spans = spans.lock().unwrap().clone();
        spans.sort();
        let [(first_start, first_end), (second_start, _)] = spans.as_slice() else {
            panic!("both callers must get a turn: {spans:?}");
        };
        assert!(first_end <= second_start, "requests overlapped");
        assert!(*second_start - *first_start >= OEMBED_MIN_SPACING);
    }

    #[tokio::test(start_paused = true)]
    async fn retry_after_delays_the_next_start_and_a_shorter_wait_sends_nothing() {
        let gate = OembedGate::new();
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(reqwest::header::RETRY_AFTER, "120".parse().unwrap());
        let retry_after = retry_after(&headers, chrono::Utc::now());
        assert_eq!(retry_after, Some(Duration::from_secs(120)));
        let throttled_at = Instant::now();
        gate.turn(OEMBED_MAX_TURN_WAIT)
            .await
            .unwrap()
            .finish(OembedOutcome::Throttled { retry_after });
        // A caller whose wait ends before TikTok's Retry-After is refused at once.
        assert!(gate.turn(OEMBED_MAX_TURN_WAIT).await.is_err());
        assert_eq!(Instant::now(), throttled_at);
        tokio::time::advance(Duration::from_secs(110)).await;
        let _turn = gate.turn(OEMBED_MAX_TURN_WAIT).await.unwrap();
        assert!(Instant::now() - throttled_at >= Duration::from_secs(120));
    }
}
