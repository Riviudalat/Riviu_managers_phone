//! Latency and outcome recording for every WDA request and recovery step.
//!
//! Live-test post-mortems kept stalling on "which call actually hung, and for
//! how long?" — the logs only showed the eventual failure. Every request now
//! lands here with its endpoint, duration and typed outcome, so a run can be
//! summarised (p50/p95 per endpoint, failures by class) without re-reading raw
//! logs, and `RIVIU_WDA_TRACE=<path>` streams the same records as JSONL for
//! offline analysis.
//!
//! Records are keyed by UDID so a multi-device farm reports per device.

use std::collections::HashMap;
use std::io::Write;
use std::sync::OnceLock;
use std::time::Instant;

use parking_lot::Mutex;

/// How a request ended. Distinguishing these is the point: a rejected command
/// (`Http`) means the runner is alive and well, while `Transport` means the
/// relay itself broke — only the latter justifies recycling anything.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Outcome {
    Ok,
    /// Socket never completed: connection refused/reset, relay wedged.
    Transport,
    /// Accepted but did not answer within its deadline.
    Timeout,
    /// WDA reports the session is gone; a new session fixes it.
    Session,
    /// WDA answered with an error status — command rejected, runner healthy.
    Http,
}

impl Outcome {
    pub fn as_str(&self) -> &'static str {
        match self {
            Outcome::Ok => "ok",
            Outcome::Transport => "transport",
            Outcome::Timeout => "timeout",
            Outcome::Session => "session",
            Outcome::Http => "http",
        }
    }
}

#[derive(Default)]
struct Bucket {
    samples: Vec<u32>,
    failures: HashMap<Outcome, u32>,
}

struct Inner {
    started: Instant,
    per_endpoint: HashMap<String, Bucket>,
    events: Vec<(String, String, u32)>,
    trace: Option<std::fs::File>,
}

fn inner() -> &'static Mutex<Inner> {
    static INNER: OnceLock<Mutex<Inner>> = OnceLock::new();
    INNER.get_or_init(|| {
        let trace = std::env::var("RIVIU_WDA_TRACE").ok().and_then(|p| {
            std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(p)
                .ok()
        });
        Mutex::new(Inner {
            started: Instant::now(),
            per_endpoint: HashMap::new(),
            events: Vec::new(),
            trace,
        })
    })
}

/// Record one completed WDA request.
pub fn record(udid: &str, endpoint: &str, ms: u32, outcome: Outcome) {
    let mut g = inner().lock();
    let elapsed = g.started.elapsed().as_millis() as u64;
    if let Some(f) = g.trace.as_mut() {
        let _ = writeln!(
            f,
            r#"{{"t_ms":{elapsed},"udid":"{udid}","kind":"request","endpoint":"{endpoint}","ms":{ms},"outcome":"{}"}}"#,
            outcome.as_str()
        );
    }
    let bucket = g.per_endpoint.entry(endpoint.to_string()).or_default();
    bucket.samples.push(ms);
    if outcome != Outcome::Ok {
        *bucket.failures.entry(outcome).or_insert(0) += 1;
    }
}

/// Record a non-request milestone: a recovery step, a relay spawn, a launch.
pub fn record_event(udid: &str, kind: &str, ms: u32, detail: &str) {
    let mut g = inner().lock();
    let elapsed = g.started.elapsed().as_millis() as u64;
    if let Some(f) = g.trace.as_mut() {
        let escaped = detail.replace('\\', "\\\\").replace('"', "\\\"");
        let _ = writeln!(
            f,
            r#"{{"t_ms":{elapsed},"udid":"{udid}","kind":"event","event":"{kind}","ms":{ms},"detail":"{escaped}"}}"#
        );
    }
    g.events.push((kind.to_string(), detail.to_string(), ms));
}

/// p50 / p95 / max / count for one endpoint.
#[derive(Debug, Clone, Copy, Default)]
pub struct Percentiles {
    pub n: usize,
    pub p50: u32,
    pub p95: u32,
    pub max: u32,
}

pub fn percentiles(endpoint: &str) -> Percentiles {
    let g = inner().lock();
    let Some(bucket) = g.per_endpoint.get(endpoint) else {
        return Percentiles::default();
    };
    pct(&bucket.samples)
}

fn pct(samples: &[u32]) -> Percentiles {
    if samples.is_empty() {
        return Percentiles::default();
    }
    let mut xs = samples.to_vec();
    xs.sort_unstable();
    // Nearest-rank: the smallest value at or below which at least q of the
    // samples fall. Unambiguous for even-sized sets, unlike interpolation.
    let idx = |q: f64| -> u32 {
        let rank = (q * xs.len() as f64).ceil().max(1.0) as usize;
        xs[(rank - 1).min(xs.len() - 1)]
    };
    Percentiles {
        n: xs.len(),
        p50: idx(0.50),
        p95: idx(0.95),
        max: *xs.last().unwrap_or(&0),
    }
}

/// Counts of each failure class across all endpoints.
pub fn failure_counts() -> HashMap<Outcome, u32> {
    let g = inner().lock();
    let mut out: HashMap<Outcome, u32> = HashMap::new();
    for bucket in g.per_endpoint.values() {
        for (k, v) in &bucket.failures {
            *out.entry(*k).or_insert(0) += v;
        }
    }
    out
}

/// Human-readable per-endpoint table, newest run only.
pub fn summary_lines() -> Vec<String> {
    let g = inner().lock();
    let mut names: Vec<&String> = g.per_endpoint.keys().collect();
    names.sort();
    let mut out = Vec::new();
    for name in names {
        let bucket = &g.per_endpoint[name];
        let p = pct(&bucket.samples);
        let mut fails: Vec<String> = bucket
            .failures
            .iter()
            .map(|(k, v)| format!("{}={v}", k.as_str()))
            .collect();
        fails.sort();
        out.push(format!(
            "{name:<26} n={:<4} p50={:<6} p95={:<6} max={:<6} {}",
            p.n,
            format!("{}ms", p.p50),
            format!("{}ms", p.p95),
            format!("{}ms", p.max),
            if fails.is_empty() {
                "ok".to_string()
            } else {
                fails.join(" ")
            }
        ));
    }
    out
}

/// Recovery/launch milestones recorded this run.
pub fn events() -> Vec<(String, String, u32)> {
    inner().lock().events.clone()
}

/// Slowest single request seen, for the "no request may hang over N s" check.
pub fn slowest_request() -> (String, u32) {
    let g = inner().lock();
    let mut worst = (String::new(), 0u32);
    for (name, bucket) in &g.per_endpoint {
        if let Some(m) = bucket.samples.iter().max() {
            if *m > worst.1 {
                worst = (name.clone(), *m);
            }
        }
    }
    worst
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentiles_of_a_known_series() {
        let p = pct(&[10, 20, 30, 40, 50, 60, 70, 80, 90, 100]);
        assert_eq!(p.n, 10);
        assert_eq!(p.max, 100);
        assert_eq!(p.p50, 50);
        assert_eq!(p.p95, 100);
    }

    #[test]
    fn percentiles_of_nothing_are_zero_not_a_panic() {
        let p = pct(&[]);
        assert_eq!(p.n, 0);
        assert_eq!(p.p95, 0);
    }
}
