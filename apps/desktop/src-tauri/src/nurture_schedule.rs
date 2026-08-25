//! What a nurture schedule tick should do, decided away from the loop that does it.
//!
//! The decision used to live inline inside the `tauri::async_runtime::spawn` in `state.rs`,
//! and the source-scan test that guards it says why that was a problem in its own words:
//! *"there is no seam a unit test can drive the spawned scheduler through"*. So the only
//! thing anybody could check was the **order of two calls in the text of the file**. Whether
//! the schedule arms, when it decides a tick is due, what it does when the mark in the
//! database is missing or corrupt, and which phones it picks were all unmeasured.
//!
//! This module is that seam. It holds no clock, no database and no device: `now`, the stored
//! marks and the connected phones are arguments, so every branch is reachable from a test.
//! The loop keeps the effects — read the rows, write the marks, start the sessions.

use std::collections::BTreeMap;

use chrono::{DateTime, Duration, FixedOffset, Timelike, Utc};
use riviu_core::{NurtureSettings, NurtureWindow, NurtureWindowBehaviour};

/// Where the single-cadence schedule keeps "when is the next run due".
pub(crate) const LEGACY_MARK: &str = "nurture.schedule.next_run_at";

/// Each window keeps its own mark, under this prefix plus the window id.
///
/// **Not one shared mark.** With one, a long cadence in the morning window writes a mark
/// hours ahead, and the afternoon window — which the operator configured separately, with its
/// own cadence — sits out the rest of the day waiting for a number that has nothing to do
/// with it. A mark answers "when is *this* window next due", so there is one per window.
pub(crate) fn window_mark(id: &str) -> String {
    format!("{LEGACY_MARK}.{id}")
}

/// What the tick found, and what the caller must therefore do.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Tick {
    /// Do nothing. The schedule is off, the clock is outside every window, or the mark has
    /// not been reached yet.
    Wait,
    /// Nothing to run on, but the mark still has to move.
    ///
    /// Without this the tick would find itself due every thirty seconds forever, and each
    /// pass would re-ask a question whose answer has not changed.
    Rearm {
        mark_key: String,
        next_run_at: DateTime<Utc>,
    },
    /// Start a session on these phones, then move this window's mark.
    Run {
        mark_key: String,
        udids: Vec<String>,
        next_run_at: DateTime<Utc>,
        duration_minutes: u32,
        /// `None` means the session runs on the panel's own settings.
        behaviour: Option<NurtureWindowBehaviour>,
    },
}

/// Whether a local wall-clock minute falls inside a window.
///
/// `end <= start` is a window that wraps past midnight — `22:00` to `02:00` is written
/// exactly that way, and reading it as an empty window would silently drop the night shift.
/// Both ends are inclusive: a window `08:00`–`11:00` is due at 11:00.
fn covers(window: &NurtureWindow, minute: u32) -> bool {
    if window.start_minute <= window.end_minute {
        minute >= window.start_minute && minute <= window.end_minute
    } else {
        minute >= window.start_minute || minute <= window.end_minute
    }
}

/// Whether a stored mark says a run is due at `now`.
///
/// **An unreadable mark counts as due, and that is the safer of the two mistakes.** Treating
/// it as "not due" would leave a schedule the operator switched on silently never running,
/// with nothing to read anywhere; treating it as due costs one early session and then
/// re-arms to a mark that parses.
fn due(mark: Option<&String>, now: DateTime<Utc>) -> bool {
    match mark {
        Some(raw) => DateTime::parse_from_rfc3339(raw)
            .map(|mark| mark.with_timezone(&Utc) <= now)
            .unwrap_or(true),
        None => true,
    }
}

/// Decide a tick.
///
/// `connected` is the fleet minus whatever the registry calls disconnected or errored; that
/// filter stays at the call site because it speaks the registry's types and this does not.
/// `offset` is the operator's UTC offset: windows are wall-clock hours *they* typed, and the
/// marks are instants, so both clocks have to be in the room and neither may be guessed.
///
/// **The empty-list fallback is reproduced here deliberately, not endorsed.** An empty phone
/// list means the whole connected fleet, because that is what the loop has always done. For a
/// window the editor prints that as the word "tất cả", so the operator can see it; for the
/// legacy single-cadence path it stays the trap it always was — the panel's tooltip says
/// "chỉ chạy trên những máy đã chọn khi lưu" while saving with nothing selected arms
/// everything. Changing that is the operator's call, not something to slip into a refactor.
/// See `the_empty_list_means_the_whole_fleet_not_no_phones`.
pub(crate) fn decide(
    settings: &NurtureSettings,
    marks: &BTreeMap<String, String>,
    connected: &[String],
    now: DateTime<Utc>,
    offset: FixedOffset,
) -> Tick {
    if !settings.schedule_enabled {
        return Tick::Wait;
    }
    if settings.schedule_windows.is_empty() {
        return decide_single_cadence(settings, marks, connected, now);
    }

    let local = now.with_timezone(&offset);
    let minute = local.hour() * 60 + local.minute();
    // First match wins. Overlapping windows are not refused — the operator may well mean
    // "20:00-22:00 on everything, and 20:30-21:00 harder on four phones" — but only one
    // session can start at a given minute, so the order in the list is the tie-break, and
    // the editor is what shows that order.
    let Some(window) = settings
        .schedule_windows
        .iter()
        .find(|window| covers(window, minute))
    else {
        // Outside every window the mark is left exactly where it is. That is what makes a
        // window fire on its own opening minute: its mark is already in the past by then,
        // so the first tick inside the window is due.
        return Tick::Wait;
    };

    let mark_key = window_mark(&window.id);
    if !due(marks.get(&mark_key), now) {
        return Tick::Wait;
    }
    let next_run_at = now + Duration::minutes(window.every_minutes.max(1) as i64);
    let udids = if window.udids.is_empty() {
        connected.to_vec()
    } else {
        window.udids.clone()
    };
    if udids.is_empty() {
        return Tick::Rearm {
            mark_key,
            next_run_at,
        };
    }
    Tick::Run {
        mark_key,
        udids,
        next_run_at,
        duration_minutes: window.duration_minutes.max(1),
        behaviour: window.behaviour.clone(),
    }
}

/// The schedule as it was before windows existed: one cadence, all day.
///
/// Every database written before `schedule_windows` came along deserializes with an empty
/// list, and an operator who never opens the window editor keeps this. It is the current
/// behaviour of the product, not a migration step.
fn decide_single_cadence(
    settings: &NurtureSettings,
    marks: &BTreeMap<String, String>,
    connected: &[String],
    now: DateTime<Utc>,
) -> Tick {
    // `max(1)` so a stored zero cannot make the next mark equal to now and spin the tick.
    let next_run_at = now + Duration::minutes(settings.schedule_every_minutes.max(1) as i64);
    let mark_key = LEGACY_MARK.to_string();
    if !due(marks.get(LEGACY_MARK), now) {
        return Tick::Wait;
    }
    let udids = if settings.schedule_udids.is_empty() {
        connected.to_vec()
    } else {
        settings.schedule_udids.clone()
    };
    if udids.is_empty() {
        return Tick::Rearm {
            mark_key,
            next_run_at,
        };
    }
    Tick::Run {
        mark_key,
        udids,
        next_run_at,
        duration_minutes: settings.schedule_duration_minutes.max(1),
        behaviour: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// UTC+7. Every window test states the offset rather than reading the machine's, so the
    /// suite means the same thing on a laptop that travels.
    fn saigon() -> FixedOffset {
        FixedOffset::east_opt(7 * 3600).expect("a valid offset")
    }

    fn armed() -> NurtureSettings {
        NurtureSettings {
            schedule_enabled: true,
            schedule_every_minutes: 240,
            schedule_duration_minutes: 150,
            schedule_udids: vec!["A".into(), "B".into()],
            ..NurtureSettings::default()
        }
    }

    fn window(id: &str, start_h: u32, end_h: u32) -> NurtureWindow {
        NurtureWindow {
            id: id.into(),
            start_minute: start_h * 60,
            end_minute: end_h * 60,
            every_minutes: 60,
            duration_minutes: 20,
            udids: vec!["A".into()],
            behaviour: None,
        }
    }

    fn at(iso: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(iso)
            .expect("fixture instant")
            .with_timezone(&Utc)
    }

    fn marks(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect()
    }

    // ───────────────────────── the switch and the mark ─────────────────────────

    #[test]
    fn a_schedule_that_is_off_never_runs() {
        let settings = NurtureSettings {
            schedule_enabled: false,
            ..armed()
        };
        assert_eq!(
            decide(
                &settings,
                &marks(&[]),
                &["A".into()],
                at("2026-08-24T10:00:00Z"),
                saigon()
            ),
            Tick::Wait
        );
    }

    #[test]
    fn a_mark_in_the_future_is_not_due() {
        assert_eq!(
            decide(
                &armed(),
                &marks(&[(LEGACY_MARK, "2026-08-24T14:00:00Z")]),
                &["A".into()],
                at("2026-08-24T10:00:00Z"),
                saigon()
            ),
            Tick::Wait
        );
    }

    /// The mark is a deadline, not a window: reaching it exactly is due.
    #[test]
    fn a_mark_exactly_now_is_due() {
        let now = at("2026-08-24T10:00:00Z");
        assert_eq!(
            decide(
                &armed(),
                &marks(&[(LEGACY_MARK, "2026-08-24T10:00:00Z")]),
                &[],
                now,
                saigon()
            ),
            Tick::Run {
                mark_key: LEGACY_MARK.into(),
                udids: vec!["A".into(), "B".into()],
                next_run_at: at("2026-08-24T14:00:00Z"),
                duration_minutes: 150,
                behaviour: None,
            }
        );
    }

    /// A mark written in another timezone is the same instant, and must not read as overdue.
    ///
    /// The loop stores `to_rfc3339()` of a UTC instant so it always writes `+00:00`, but a
    /// database restored from elsewhere, or a hand edit, does not have to. Comparing the
    /// strings, or parsing naively, would make `11:00+07:00` — which is 04:00 UTC — look like
    /// a mark four hours in the future when it is six hours in the past.
    #[test]
    fn a_mark_with_an_offset_is_compared_as_an_instant() {
        let now = at("2026-08-24T10:00:00Z");
        let tick = decide(
            &armed(),
            &marks(&[(LEGACY_MARK, "2026-08-24T11:00:00+07:00")]),
            &[],
            now,
            saigon(),
        );
        assert!(
            matches!(tick, Tick::Run { .. }),
            "11:00+07:00 is 04:00Z, six hours before now, so the tick is overdue: {tick:?}"
        );
    }

    /// A mark that does not parse fires rather than hangs.
    #[test]
    fn an_unreadable_mark_is_treated_as_due() {
        let now = at("2026-08-24T10:00:00Z");
        for raw in ["", "soon", "2026-08-24", "not-a-date"] {
            let tick = decide(&armed(), &marks(&[(LEGACY_MARK, raw)]), &[], now, saigon());
            assert!(
                matches!(tick, Tick::Run { .. }),
                "an unparseable mark ({raw:?}) must not park the schedule forever"
            );
        }
    }

    /// **The contradiction, pinned as it is.**
    ///
    /// Empty `schedule_udids` means the whole connected fleet, while `NurtureScheduleTab.tsx`
    /// tells the operator the schedule runs only on the phones picked when it was saved.
    /// Saving with nothing selected on the grid therefore arms twenty phones. This test
    /// exists so the behaviour cannot change by accident before somebody decides which of the
    /// two is wrong — and so whoever changes it has to come here and read this.
    #[test]
    fn the_empty_list_means_the_whole_fleet_not_no_phones() {
        let settings = NurtureSettings {
            schedule_udids: Vec::new(),
            ..armed()
        };
        let fleet: Vec<String> = vec!["A".into(), "B".into(), "C".into()];
        assert_eq!(
            decide(
                &settings,
                &marks(&[]),
                &fleet,
                at("2026-08-24T10:00:00Z"),
                saigon()
            ),
            Tick::Run {
                mark_key: LEGACY_MARK.into(),
                udids: fleet,
                next_run_at: at("2026-08-24T14:00:00Z"),
                duration_minutes: 150,
                behaviour: None,
            }
        );
    }

    /// Due with nothing to run on still moves the mark, or the tick spins every 30 s.
    #[test]
    fn nothing_to_run_on_rearms_instead_of_retrying_forever() {
        let settings = NurtureSettings {
            schedule_udids: Vec::new(),
            ..armed()
        };
        assert_eq!(
            decide(
                &settings,
                &marks(&[]),
                &[],
                at("2026-08-24T10:00:00Z"),
                saigon()
            ),
            Tick::Rearm {
                mark_key: LEGACY_MARK.into(),
                next_run_at: at("2026-08-24T14:00:00Z"),
            }
        );
    }

    /// A stored zero period cannot make the next mark equal now.
    #[test]
    fn a_zero_period_still_advances_the_mark() {
        let settings = NurtureSettings {
            schedule_every_minutes: 0,
            ..armed()
        };
        let now = at("2026-08-24T10:00:00Z");
        let Tick::Run { next_run_at, .. } = decide(&settings, &marks(&[]), &[], now, saigon())
        else {
            panic!("a due schedule with phones must run");
        };
        assert!(
            next_run_at > now,
            "a mark at or before now makes the tick fire every thirty seconds"
        );
    }

    // ───────────────────────────── windows ─────────────────────────────

    fn windowed(windows: Vec<NurtureWindow>) -> NurtureSettings {
        NurtureSettings {
            schedule_windows: windows,
            ..armed()
        }
    }

    /// Outside every window nothing runs, however overdue the mark is.
    ///
    /// This is the point of the feature: before it, "lịch tự chạy" meant *all day*, and a
    /// fleet nurturing at four in the morning is the behaviour the operator asked to be able
    /// to stop.
    #[test]
    fn outside_every_window_the_schedule_is_quiet() {
        let settings = windowed(vec![window("morning", 8, 11)]);
        // 05:00Z is 12:00 in Saigon — after the morning window closed.
        assert_eq!(
            decide(
                &settings,
                &marks(&[]),
                &["A".into()],
                at("2026-08-24T05:00:00Z"),
                saigon()
            ),
            Tick::Wait
        );
    }

    /// The hours are the operator's wall clock, not UTC.
    ///
    /// 02:00Z is 09:00 in Saigon, which is inside `08:00-11:00`. Reading the window against
    /// UTC would put this run at two in the morning — the exact hour a window exists to
    /// avoid — and the mistake would be invisible: the phones would nurture at the wrong time
    /// of day, forever, with every log line agreeing.
    #[test]
    fn windows_are_matched_against_the_operators_local_clock() {
        let settings = windowed(vec![window("morning", 8, 11)]);
        let tick = decide(
            &settings,
            &marks(&[]),
            &["A".into()],
            at("2026-08-24T02:00:00Z"),
            saigon(),
        );
        assert_eq!(
            tick,
            Tick::Run {
                mark_key: window_mark("morning"),
                udids: vec!["A".into()],
                next_run_at: at("2026-08-24T03:00:00Z"),
                duration_minutes: 20,
                behaviour: None,
            }
        );
    }

    /// A window that ends before it starts runs through midnight.
    #[test]
    fn a_window_may_wrap_past_midnight() {
        let settings = windowed(vec![window("night", 22, 2)]);
        // 18:00Z is 01:00 next day in Saigon — inside 22:00-02:00.
        assert!(matches!(
            decide(
                &settings,
                &marks(&[]),
                &["A".into()],
                at("2026-08-24T18:00:00Z"),
                saigon()
            ),
            Tick::Run { .. }
        ));
        // 10:00Z is 17:00 in Saigon — outside it.
        assert_eq!(
            decide(
                &settings,
                &marks(&[]),
                &["A".into()],
                at("2026-08-24T10:00:00Z"),
                saigon()
            ),
            Tick::Wait
        );
    }

    /// Each window carries its own mark, so one window's cadence cannot mute another.
    ///
    /// The morning window running every 240 minutes writes a mark hours ahead. With a single
    /// shared mark that number would also gag the afternoon window, which the operator
    /// configured separately and which has nothing to do with it.
    #[test]
    fn one_windows_mark_does_not_gag_another() {
        let mut morning = window("morning", 8, 11);
        morning.every_minutes = 240;
        let afternoon = window("afternoon", 14, 17);
        let settings = windowed(vec![morning, afternoon]);

        // 08:00Z is 15:00 in Saigon: inside the afternoon window. The morning window's mark
        // is far in the future and must not matter.
        let tick = decide(
            &settings,
            &marks(&[(&window_mark("morning"), "2026-08-24T20:00:00Z")]),
            &["A".into()],
            at("2026-08-24T08:00:00Z"),
            saigon(),
        );
        assert_eq!(
            tick,
            Tick::Run {
                mark_key: window_mark("afternoon"),
                udids: vec!["A".into()],
                next_run_at: at("2026-08-24T09:00:00Z"),
                duration_minutes: 20,
                behaviour: None,
            }
        );
    }

    /// A window opens with its mark already in the past, so the first tick inside it fires.
    ///
    /// That is why `Tick::Wait` outside a window leaves the mark alone: re-arming out there
    /// would push the mark past the opening minute and the window would start late by however
    /// long its cadence is.
    #[test]
    fn a_window_fires_on_its_opening_minute() {
        let settings = windowed(vec![window("morning", 8, 11)]);
        let stale = marks(&[(&window_mark("morning"), "2026-08-23T00:00:00Z")]);
        // 01:00Z is exactly 08:00 in Saigon.
        assert!(matches!(
            decide(
                &settings,
                &stale,
                &["A".into()],
                at("2026-08-24T01:00:00Z"),
                saigon()
            ),
            Tick::Run { .. }
        ));
    }

    /// The window's own cadence and cap are what the run uses, not the panel's.
    #[test]
    fn a_window_runs_on_its_own_cadence_and_cap() {
        let mut evening = window("evening", 20, 22);
        evening.every_minutes = 45;
        evening.duration_minutes = 15;
        let settings = windowed(vec![evening]);
        // 13:00Z is 20:00 in Saigon.
        let tick = decide(
            &settings,
            &marks(&[]),
            &["A".into()],
            at("2026-08-24T13:00:00Z"),
            saigon(),
        );
        let Tick::Run {
            next_run_at,
            duration_minutes,
            ..
        } = tick
        else {
            panic!("inside the window with a phone, this runs");
        };
        assert_eq!(next_run_at, at("2026-08-24T13:45:00Z"));
        assert_eq!(
            duration_minutes, 15,
            "the panel's 150 is not this window's cap"
        );
    }

    /// An empty phone list in a window means every connected phone.
    #[test]
    fn a_window_with_no_phones_named_takes_the_whole_fleet() {
        let mut all = window("evening", 20, 22);
        all.udids = Vec::new();
        let settings = windowed(vec![all]);
        let fleet: Vec<String> = vec!["A".into(), "B".into(), "C".into()];
        let tick = decide(
            &settings,
            &marks(&[]),
            &fleet,
            at("2026-08-24T13:00:00Z"),
            saigon(),
        );
        let Tick::Run { udids, .. } = tick else {
            panic!("inside the window with a fleet, this runs");
        };
        assert_eq!(udids, fleet);
    }

    /// A window's behaviour block reaches the caller; without one, the panel's applies.
    #[test]
    fn a_window_carries_its_behaviour_override_through() {
        let heavy = NurtureWindowBehaviour {
            num_videos: 40,
            num_rounds: 2,
            like_prob: 25,
            comment_prob: 5,
            follow_prob: 0,
        };
        let mut evening = window("evening", 20, 22);
        evening.behaviour = Some(heavy.clone());
        let settings = windowed(vec![evening]);
        let tick = decide(
            &settings,
            &marks(&[]),
            &["A".into()],
            at("2026-08-24T13:00:00Z"),
            saigon(),
        );
        let Tick::Run { behaviour, .. } = tick else {
            panic!("inside the window with a phone, this runs");
        };
        assert_eq!(behaviour, Some(heavy));
    }

    /// An empty window list keeps the schedule exactly as it behaved before windows existed.
    #[test]
    fn no_windows_means_the_old_all_day_cadence() {
        let settings = windowed(Vec::new());
        // 19:00Z is 02:00 in Saigon — an hour no window would cover, and the old schedule ran
        // at it, because the old schedule ran at every hour.
        assert!(matches!(
            decide(
                &settings,
                &marks(&[]),
                &["A".into()],
                at("2026-08-24T19:00:00Z"),
                saigon()
            ),
            Tick::Run {
                duration_minutes: 150,
                ..
            }
        ));
    }
}
