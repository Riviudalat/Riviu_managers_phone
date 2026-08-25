//! What each phone said, kept where somebody can still read it.
//!
//! [`crate::types::NurtureSessionStatus`] carries `last_message`, one string, overwritten
//! on every change. That is enough to render a live row and useless for the question an
//! operator actually asks about a phone that went wrong — *what happened before this?*
//! The history existed only as `tracing` output, which in the desktop app has no
//! subscriber and in the harness scrolls past.
//!
//! So this is a small ring per device, written at the one point every status already
//! funnels through, and read back by udid. Two properties are deliberate:
//!
//! * **Consecutive identical lines collapse into a count.** A session polling for the feed
//!   emits the same sentence every second; without this, two hundred slots hold one
//!   message and the interesting line before it is already gone. `×14` says the same
//!   thing in one slot.
//! * **It is not persisted.** A ring in memory survives closing the panel and reloading
//!   the webview, which is what the frontend could not do on its own; it does not survive
//!   restarting the app, and nothing here pretends otherwise. Anything that must outlive
//!   the process belongs in the database, and the audit rows that do already live there.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use chrono::{DateTime, Utc};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

/// Lines kept per device.
///
/// Two hundred is sized against a real session rather than picked round: the messages
/// worth reading are the ladder ones — opening the app, declining a dialog, the feed
/// arriving, a swipe going unproven, a recovery — and a ten-minute session on this fleet
/// produces a few dozen of those once repeats collapse. Enough to hold a whole session,
/// small enough that twenty devices cost kilobytes.
pub const SESSION_LOG_CAPACITY: usize = 200;

/// One line, and when it was first said.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionLogEntry {
    /// When this line was **first** recorded. A collapsed run keeps its first timestamp
    /// so the reader can see when the phone entered that state, not when it last
    /// repeated itself — "stuck here since 14:22" is the useful reading.
    pub at: DateTime<Utc>,
    /// When the most recent repeat landed. Equal to [`Self::at`] for a line said once.
    pub last_at: DateTime<Utc>,
    pub text: String,
    /// How many times in a row this same text was recorded. `1` for an ordinary line.
    pub repeats: u32,
}

/// One device that has said something, for building the row list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionLogSummary {
    pub udid: String,
    pub lines: usize,
    /// The most recent line. `None` cannot happen for a device that appears here — a ring
    /// is only created by a recorded line — but the type says so rather than unwrapping.
    pub last: Option<SessionLogEntry>,
}

/// The per-device rings.
///
/// Cloning is free and every clone shares one book — the nurture runtime and the idle
/// sweeper both write to it, and the command that reads it holds a third clone.
#[derive(Clone, Default)]
pub struct SessionLogBook {
    inner: Arc<Mutex<HashMap<String, VecDeque<SessionLogEntry>>>>,
}

impl SessionLogBook {
    pub fn new() -> Self {
        Self::default()
    }

    /// Append a line for `udid`, collapsing it into the previous one if identical.
    ///
    /// Blank text is dropped rather than stored. An empty `last_message` is the initial
    /// value of a status struct, not something the phone said, and a log of blank rows
    /// costs the ring exactly as much as a log of real ones.
    pub fn record(&self, udid: &str, text: &str) {
        let text = text.trim();
        if text.is_empty() {
            return;
        }
        let now = Utc::now();
        let mut books = self.inner.lock();
        let ring = books.entry(udid.to_string()).or_default();
        if let Some(last) = ring.back_mut() {
            if last.text == text {
                last.repeats = last.repeats.saturating_add(1);
                last.last_at = now;
                return;
            }
        }
        ring.push_back(SessionLogEntry {
            at: now,
            last_at: now,
            text: text.to_string(),
            repeats: 1,
        });
        while ring.len() > SESSION_LOG_CAPACITY {
            ring.pop_front();
        }
    }

    /// Everything kept for `udid`, oldest first. Empty for a device that has said nothing.
    pub fn entries(&self, udid: &str) -> Vec<SessionLogEntry> {
        self.inner
            .lock()
            .get(udid)
            .map(|ring| ring.iter().cloned().collect())
            .unwrap_or_default()
    }

    /// Every device with history, and the last thing each one said.
    ///
    /// Exists because the panel's rows used to come from the live nurture statuses alone,
    /// and the idle sweeper produces neither a session nor a status — so a phone it had
    /// just unstuck had a full history and no row anywhere to open it from. The whole
    /// point of writing the sweeper's lines into this book is that somebody can read
    /// them, and that needs a way to find out which phones have any.
    ///
    /// Sorted by udid so the row order does not shuffle between polls.
    pub fn summaries(&self) -> Vec<SessionLogSummary> {
        let mut out: Vec<SessionLogSummary> = self
            .inner
            .lock()
            .iter()
            .map(|(udid, ring)| SessionLogSummary {
                udid: udid.clone(),
                lines: ring.len(),
                last: ring.back().cloned(),
            })
            .collect();
        out.sort_by(|a, b| a.udid.cmp(&b.udid));
        out
    }

    /// Drop one device's history — used when the operator asks to clear it.
    pub fn clear(&self, udid: &str) {
        self.inner.lock().remove(udid);
    }

    /// How many devices have any history. For tests and diagnostics.
    pub fn tracked_devices(&self) -> usize {
        self.inner.lock().len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_come_back_in_the_order_they_were_said() {
        let book = SessionLogBook::new();
        book.record("a", "mở TikTok");
        book.record("a", "đóng hộp thoại");
        book.record("a", "feed đã lên");
        let texts: Vec<_> = book.entries("a").into_iter().map(|e| e.text).collect();
        assert_eq!(texts, ["mở TikTok", "đóng hộp thoại", "feed đã lên"]);
    }

    #[test]
    fn devices_do_not_share_a_ring() {
        let book = SessionLogBook::new();
        book.record("a", "chỉ của a");
        book.record("b", "chỉ của b");
        assert_eq!(book.entries("a").len(), 1);
        assert_eq!(book.entries("b").len(), 1);
        assert_eq!(book.entries("a")[0].text, "chỉ của a");
        assert_eq!(book.tracked_devices(), 2);
    }

    #[test]
    fn a_device_that_never_spoke_reads_empty_rather_than_erroring() {
        assert!(SessionLogBook::new().entries("nobody").is_empty());
    }

    /// The property the ring exists for: a poll loop repeating one sentence must not push
    /// the line before it out of the history.
    #[test]
    fn a_repeated_line_collapses_instead_of_filling_the_ring() {
        let book = SessionLogBook::new();
        book.record("a", "mở TikTok");
        for _ in 0..(SESSION_LOG_CAPACITY * 3) {
            book.record("a", "TikTok đang khởi động — chờ feed lên");
        }
        let entries = book.entries("a");
        assert_eq!(entries.len(), 2, "the run collapsed into one slot");
        assert_eq!(entries[0].text, "mở TikTok", "the earlier line survived");
        assert_eq!(entries[1].repeats, (SESSION_LOG_CAPACITY * 3) as u32);
    }

    /// A collapsed run keeps the moment it *started*, because "stuck since" is the reading
    /// that matters. It also records the latest repeat, so a stale run is visible as such.
    #[test]
    fn a_collapsed_run_keeps_its_first_timestamp_and_tracks_the_latest() {
        let book = SessionLogBook::new();
        book.record("a", "chờ feed");
        let first = book.entries("a")[0].at;
        book.record("a", "chờ feed");
        let entry = book.entries("a").remove(0);
        assert_eq!(entry.at, first, "start of the run is preserved");
        assert!(entry.last_at >= first);
        assert_eq!(entry.repeats, 2);
    }

    /// Interleaving matters: the same sentence said again *after* something else is a new
    /// event, not a repeat of the old one.
    #[test]
    fn only_consecutive_lines_collapse() {
        let book = SessionLogBook::new();
        book.record("a", "chờ feed");
        book.record("a", "đóng hộp thoại");
        book.record("a", "chờ feed");
        let entries = book.entries("a");
        assert_eq!(entries.len(), 3);
        assert!(entries.iter().all(|e| e.repeats == 1));
    }

    #[test]
    fn distinct_lines_past_the_cap_drop_the_oldest() {
        let book = SessionLogBook::new();
        for i in 0..(SESSION_LOG_CAPACITY + 25) {
            book.record("a", &format!("dòng {i}"));
        }
        let entries = book.entries("a");
        assert_eq!(entries.len(), SESSION_LOG_CAPACITY);
        assert_eq!(
            entries[0].text, "dòng 25",
            "the oldest 25 fell off the front"
        );
    }

    #[test]
    fn blank_lines_are_not_stored() {
        let book = SessionLogBook::new();
        book.record("a", "");
        book.record("a", "   ");
        assert!(book.entries("a").is_empty());
        assert_eq!(book.tracked_devices(), 0, "and no ring is created for them");
    }

    #[test]
    fn text_is_trimmed_so_padding_does_not_defeat_collapsing() {
        let book = SessionLogBook::new();
        book.record("a", "chờ feed");
        book.record("a", "  chờ feed  ");
        let entries = book.entries("a");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].repeats, 2);
    }

    #[test]
    fn clearing_one_device_leaves_the_others_alone() {
        let book = SessionLogBook::new();
        book.record("a", "của a");
        book.record("b", "của b");
        book.clear("a");
        assert!(book.entries("a").is_empty());
        assert_eq!(book.entries("b").len(), 1);
    }

    /// The sweeper writes for phones that never ran a session, so "which phones have
    /// history" cannot be answered from the nurture status list. This is that answer.
    #[test]
    fn summaries_name_every_device_with_history_and_its_last_line() {
        let book = SessionLogBook::new();
        book.record("b", "cũ");
        book.record("b", "mới nhất");
        book.record("a", "một dòng");
        let summaries = book.summaries();
        assert_eq!(
            summaries
                .iter()
                .map(|s| s.udid.as_str())
                .collect::<Vec<_>>(),
            ["a", "b"],
            "sorted, so the row order does not shuffle between polls"
        );
        assert_eq!(summaries[1].lines, 2);
        assert_eq!(
            summaries[1].last.as_ref().map(|e| e.text.as_str()),
            Some("mới nhất")
        );
    }

    #[test]
    fn a_cleared_device_leaves_the_summary_list() {
        let book = SessionLogBook::new();
        book.record("a", "một dòng");
        book.clear("a");
        assert!(book.summaries().is_empty());
    }

    #[test]
    fn clones_share_one_book() {
        let book = SessionLogBook::new();
        let other = book.clone();
        other.record("a", "viết qua bản sao");
        assert_eq!(book.entries("a").len(), 1);
    }
}
