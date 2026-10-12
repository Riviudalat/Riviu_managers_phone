//! Measured TikTok sound-picker support for the Android publish path.
//!
//! The picker is deliberately version keyed. Resource ids move between TikTok builds, and
//! selecting the wrong row is still a public-post input even though the selection itself is
//! reversible. Unknown packages, versions and locales therefore have no plan.

use std::collections::HashMap;
use std::time::Duration;

use anyhow::Context;
use std::sync::{
    atomic::{AtomicBool, AtomicU32, AtomicU8, Ordering},
    Arc,
};
use tokio::time::Instant;

use crate::driver::{ElementBox, ElementQuery, UiSession};
use crate::publish::SoundCandidate;
use unicode_normalization::UnicodeNormalization;

// TikTok can style the selected editor title (e.g. mathematical bold plus NBSP)
// while the picker exposes plain text. Preserve letters, case and punctuation.
fn same_editor_sound_title(observed: &str, expected: &str) -> bool {
    let normalize = |value: &str| {
        value
            .nfkc()
            .map(|character| {
                if character.is_whitespace() {
                    ' '
                } else {
                    character
                }
            })
            .collect::<String>()
    };
    normalize(observed.trim()) == normalize(expected.trim())
}

pub(crate) mod observation;
mod recent;
mod selection_recovery;
#[cfg(test)]
mod selection_recovery_tests;
mod snapshot;
mod visual;

const READBACK_WINDOW: Duration = Duration::from_secs(8);
const POLL: Duration = Duration::from_millis(250);
const SOUND_WINDOW: Duration = Duration::from_secs(180);
// #23/#28 (Global 45.7.3/en, 2026-10-08 20:34-20:50): every pinned /source on the sheet waited
// ~10.5 s for a root that never came, twelve times in a row, until the phase deadline. Two in a
// row (~21 s) is enough to stop asking that phase and let the measured visual route observe.
const ROOT_UNREADABLE_CAP: u32 = 2;

struct SoundBudget {
    deadline: Instant,
    stopped: Arc<AtomicBool>,
    popup: Arc<tokio::sync::Mutex<crate::app_automation::dialogs::PopupBudget>>,
    popup_package: Arc<tokio::sync::Mutex<Option<&'static str>>>,
    route: Arc<SoundRouteEvidence>,
}
tokio::task_local! { static SOUND_BUDGET: SoundBudget; }

/// What the sound stage observed about its own route. Timeline evidence only; it never
/// authorizes a tap or Post.
#[derive(Default)]
struct SoundRouteEvidence {
    unreadable_roots: AtomicU32,
    /// Unreadable roots since the last root that returned, across phases and the composer's
    /// inline retries. A phase that starts again must not reset what the stage already saw.
    consecutive_unreadable: AtomicU32,
    /// The visual route already recorded that it skips the XML popup check.
    popup_check_skipped: AtomicBool,
    selection: AtomicU8,
    /// A sheet tab tap was dispatched in this stage; see [`recycle_after_tab_switch`].
    tab_tapped: AtomicBool,
    /// `unreadable_roots` when the last tab tap was dispatched.
    roots_at_tab_tap: AtomicU32,
    /// Session replacements this stage asked for. Capped at one.
    tab_recycles: AtomicU32,
    /// Raised while a phase that can recycle and re-read the sheet is observing.
    recycle_armed: AtomicBool,
    /// The epoch this stage's own recycle produced, for owners that bound the old one.
    recycled_epoch: parking_lot::Mutex<Option<String>>,
    /// The last two-read late XML pool, its epoch and when it was proved.
    late_xml: parking_lot::Mutex<Option<(String, Instant, ObservedSoundPool)>>,
    /// Switches of the read session to every-window trees this stage asked for. Capped at one.
    multi_window_switches: AtomicU32,
    /// The sheet was read through every window at least once in this stage.
    multi_window_read: AtomicBool,
    /// A read-only `/source` on the sheet spent a request deadline this stage; see
    /// [`source_request_timed_out`]. The next unreadable root hands off without a second one.
    source_timed_out: AtomicBool,
    /// Visual row taps dispatched in this stage, across the composer's inline recovery.
    row_taps: AtomicU32,
    /// A visual observation ran out of time without a frame showing Hot selected.
    hot_unconfirmed: AtomicBool,
}

/// The stage's route evidence, kept past the end of its budget scope so the composer can say
/// why the sound stage ran out of time. It never authorizes a tap or Post.
#[derive(Default)]
pub(crate) struct SoundStageEvidence(Option<Arc<SoundRouteEvidence>>);

impl SoundStageEvidence {
    /// Called inside the stage's budget scope; outside one it holds nothing.
    pub(crate) fn capture() -> Self {
        Self(SOUND_BUDGET.try_with(|budget| budget.route.clone()).ok())
    }

    /// The operator message for a sound stage whose budget ran out. Run 72401bfd (other PC,
    /// 2026-10-12, Máy 33/40): the sheet root never answered on many phones and the visual
    /// route's Hot taps never showed Hot selected, yet the operator saw only a generic timeout.
    /// Phone 44 (box3150, 2026-10-10) showed the same blind sheet and swallowed taps while
    /// overloaded, and a reboot cleared it (docs/agents/09-fleet-android.md).
    /// A blind sheet in the sense of the visual hand-off, not one stray unreadable read. A
    /// stage that adopted a blind sheet remembered from the last attempt asks no root at all;
    /// its frames skip the XML popup check, and only a blind sheet reaches the frames.
    pub(crate) fn sheet_blind(&self) -> bool {
        self.0.as_ref().is_some_and(|route| {
            route.unreadable_roots.load(Ordering::Relaxed) >= ROOT_UNREADABLE_CAP
                || route.source_timed_out.load(Ordering::Relaxed)
                || route.popup_check_skipped.load(Ordering::Relaxed)
                || route.hot_unconfirmed.load(Ordering::Relaxed)
        })
    }

    pub(crate) fn budget_exhausted_message(&self) -> String {
        let Some(route) = self.0.as_ref().filter(|_| self.sheet_blind()) else {
            return SOUND_BUDGET_EXHAUSTED.into();
        };
        let hot_unconfirmed = route.hot_unconfirmed.load(Ordering::Relaxed);
        let hot = if hot_unconfirmed {
            " và bảng nhạc không chuyển sang tab Hot"
        } else {
            ""
        };
        format!(
            "Đã hết thời gian chọn nhạc; chưa bấm Đăng. TikTok trên máy không trả giao diện bảng nhạc{hot}: máy có thể đang quá tải hoặc có phần mềm khác (như GenFarmer) đang điều khiển máy. Dừng phần mềm đó điều khiển máy này, khởi động lại máy nếu máy không còn bài đăng chưa rõ, rồi chọn Thử lại."
        )
    }
}

const SOUND_BUDGET_EXHAUSTED: &str =
    "Đã hết thời gian tải và xác nhận nhạc; chưa bấm Đăng. Có thể chọn Thử lại để bắt đầu lượt mới.";

fn note_hot_unconfirmed() {
    let _ =
        SOUND_BUDGET.try_with(|budget| budget.route.hot_unconfirmed.store(true, Ordering::Relaxed));
}
const ROUTE_XML: u8 = 1;
const ROUTE_VISUAL: u8 = 2;
/// At most one session replacement per sound stage.
const TAB_SWITCH_RECYCLE_CAP: u32 = 1;
/// At most one switch to every-window reads per sound stage.
const MULTI_WINDOW_SWITCH_CAP: u32 = 1;

/// A sheet tab tap was dispatched. Recorded after the gesture, whatever it returned.
fn note_tab_tap() {
    let _ = SOUND_BUDGET.try_with(|budget| {
        let roots = budget.route.unreadable_roots.load(Ordering::Relaxed);
        budget
            .route
            .roots_at_tab_tap
            .store(roots, Ordering::Relaxed);
        budget.route.tab_tapped.store(true, Ordering::Relaxed);
    });
}

/// Claims one visual row tap for this stage, or `None` once `cap` taps were dispatched.
/// Outside a sound stage nothing is shared to count against, so the claim is `Some(1)`.
fn claim_row_tap(cap: u32) -> Option<u32> {
    SOUND_BUDGET
        .try_with(|budget| {
            budget
                .route
                .row_taps
                .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |taps| {
                    (taps < cap).then_some(taps + 1)
                })
                .ok()
                .map(|taps| taps + 1)
        })
        .unwrap_or(Some(1))
}

/// Visual row taps already dispatched in this stage.
fn row_taps() -> u32 {
    SOUND_BUDGET
        .try_with(|budget| budget.route.row_taps.load(Ordering::Relaxed))
        .unwrap_or(0)
}

/// Unreadable roots met since the last tab tap.
fn unreadable_since_tab_tap() -> u32 {
    SOUND_BUDGET
        .try_with(|budget| {
            budget
                .route
                .unreadable_roots
                .load(Ordering::Relaxed)
                .saturating_sub(budget.route.roots_at_tab_tap.load(Ordering::Relaxed))
        })
        .unwrap_or(0)
}

/// Phone 44 (Global 45.7.3/en, SM-G955F, 2026-10-10, `target/source-probe/44-*.jsonl`): once the
/// sheet went blind, every `/source` on that UiAutomator2 server waited ~10.5 s for a root that
/// never came, on Hot and For You alike, and a new POST /session on the same server stayed blind.
/// A restarted instrumentation read the same sheet 5/5 at ~1.9 s on either tab. The 2026-10-09
/// controller log shows 44's first currentXml read blind before any Hot tap, so opening the sheet
/// can cause it too. So an unreadable root after a tab tap, or the stage's first unreadable root
/// with no tab tap, ends the phase at its observation boundary, once per stage, and only inside a
/// phase that re-reads the sheet. Ask before noting the read that found it.
fn tab_switch_recycle_due(plan: SoundPickerPlan) -> bool {
    selection_recovery::measured(plan)
        && SOUND_BUDGET
            .try_with(|budget| {
                budget.route.recycle_armed.load(Ordering::Relaxed)
                    && (budget.route.tab_tapped.load(Ordering::Relaxed)
                        || budget.route.unreadable_roots.load(Ordering::Relaxed) == 0)
                    && budget.route.tab_recycles.load(Ordering::Relaxed) < TAB_SWITCH_RECYCLE_CAP
            })
            .unwrap_or(false)
}

/// Mark an unreadable sheet root for the arming phase. Every other error is unchanged,
/// and the original error stays in the chain for the existing classifiers.
fn blind_after_tab_switch(plan: SoundPickerPlan, error: anyhow::Error) -> anyhow::Error {
    if unreadable_root(&error) && tab_switch_recycle_due(plan) {
        error.context(SoundTabSwitchBlind)
    } else {
        error
    }
}

/// Lowers `recycle_armed` however the armed phase ends. A session that cannot recycle is never
/// armed, so its phases keep their existing route.
struct RecycleArmed;
impl RecycleArmed {
    fn arm(session: &dyn UiSession) -> Self {
        let armed = session.can_recycle_read_session();
        let _ = SOUND_BUDGET
            .try_with(|budget| budget.route.recycle_armed.store(armed, Ordering::Relaxed));
        Self
    }
}
impl Drop for RecycleArmed {
    fn drop(&mut self) {
        let _ = SOUND_BUDGET
            .try_with(|budget| budget.route.recycle_armed.store(false, Ordering::Relaxed));
    }
}

fn tab_recycles() -> u32 {
    SOUND_BUDGET
        .try_with(|budget| budget.route.tab_recycles.load(Ordering::Relaxed))
        .unwrap_or(0)
}

/// The epoch this stage's own recycle produced, if the session still carries it.
pub(crate) fn recycled_epoch(session: &dyn UiSession) -> Option<String> {
    let current = session.gui_session_epoch();
    SOUND_BUDGET
        .try_with(|budget| budget.route.recycled_epoch.lock().clone())
        .ok()
        .flatten()
        .filter(|epoch| !epoch.is_empty() && *epoch == current)
}

/// Replace the blind session once, at the caller's observation boundary. The caller holds no
/// target: every pool, tab and row bound to the old epoch is void, and the observation binding
/// starts over. Nothing is tapped here. The caller re-reads the sheet on the new session.
async fn recycle_after_tab_switch(
    session: &dyn UiSession,
    plan: SoundPickerPlan,
    step: &str,
) -> anyhow::Result<()> {
    let claimed = SOUND_BUDGET
        .try_with(|budget| {
            budget
                .route
                .tab_recycles
                .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |used| {
                    (used < TAB_SWITCH_RECYCLE_CAP).then_some(used + 1)
                })
                .is_ok()
        })
        .unwrap_or(false);
    anyhow::ensure!(claimed, "sound session recycle cap reached");
    let roots = unreadable_roots();
    let before = session.gui_session_epoch();
    let recycled = async {
        check_wait()?;
        read_sound(session.recycle_read_session()).await?;
        let after = session.gui_session_epoch();
        anyhow::ensure!(
            !after.is_empty() && after != before,
            "sound session recycle kept the old epoch"
        );
        anyhow::ensure!(
            read_sound(session.active_app_bundle()).await? == plan.package
                && session.gui_session_epoch() == after,
            "sound app/session changed during session recycle"
        );
        Ok(after)
    }
    .await;
    // Whatever happened, nothing read before this point may reach a tap.
    crate::tiktok_composer::observation::void_binding();
    let _ = SOUND_BUDGET.try_with(|budget| {
        *budget.route.late_xml.lock() = None;
        // A failed recycle keeps the unreadable run, so the existing route hands off as before.
        if let Ok(after) = &recycled {
            budget
                .route
                .consecutive_unreadable
                .store(0, Ordering::Relaxed);
            *budget.route.recycled_epoch.lock() = Some(after.clone());
        }
    });
    tracing::warn!(serial=?session.gui_scope().map(|scope| scope.device_id),
        previous=%before, current=%session.gui_session_epoch(), unreadableRoots=roots,
        recycled=recycled.is_ok(), step, "sound session recycled by an instrumentation restart");
    crate::publish_recovery::note_read(
        "sound",
        "sessionRecycledAfterTabSwitch",
        tab_recycles(),
        if recycled.is_ok() { "recycled" } else { "failed" },
        Some(&match &recycled {
            Ok(_) => format!(
                "Cây giao diện bảng nhạc không đọc được; đã khởi động lại UiAutomator một lần và đọc lại bảng nhạc từ đầu (bước: {step}; số lần không đọc được: {roots})"
            ),
            Err(error) => format!(
                "Không khởi động lại được UiAutomator để đọc bảng nhạc: {error:#} (bước: {step}; số lần không đọc được: {roots})"
            ),
        }),
    );
    recycled.map(|_| ())
}

fn note_unreadable_root() {
    let _ = SOUND_BUDGET.try_with(|budget| {
        budget
            .route
            .unreadable_roots
            .fetch_add(1, Ordering::Relaxed);
        budget
            .route
            .consecutive_unreadable
            .fetch_add(1, Ordering::Relaxed);
    });
}

/// Count an unreadable read, and remember a `/source` that spent the driver's request
/// deadline. The flag outlives a later readable root: the stage never spends a second one.
fn note_unreadable_read(error: &anyhow::Error) {
    if source_request_timed_out(error) {
        note_source_deadline();
    } else {
        note_unreadable_root();
    }
}

/// A read-only `/source` abandoned at a deadline, by the driver or by this stage. The server
/// is still waiting on the same root, and the next read queues behind it.
fn note_source_deadline() {
    note_unreadable_root();
    let _ = SOUND_BUDGET
        .try_with(|budget| budget.route.source_timed_out.store(true, Ordering::Relaxed));
}

fn source_timed_out() -> bool {
    SOUND_BUDGET
        .try_with(|budget| budget.route.source_timed_out.load(Ordering::Relaxed))
        .unwrap_or(false)
}

/// Enough unreadable roots in a row for a measured visual route to take over: ROOT_UNREADABLE_CAP
/// of them, or one once a `/source` deadline was spent. Phone 44 (box3150g, 2026-10-10 01:01): a
/// 30 s `agent_request_timeout` on the sheet was not counted, and the XML route then spent four
/// ~35 s pre-row attempts on the same blind root until the stage budget ran out. A second 30 s
/// read never buys a tree the first one did not, so one is the most this stage spends on them.
fn blind_handoff_reached(consecutive: u32) -> bool {
    consecutive >= ROOT_UNREADABLE_CAP || (consecutive >= 1 && source_timed_out())
}

/// A sound read returned a root; the next unreadable one starts a new run.
fn note_readable_root() {
    let _ = SOUND_BUDGET.try_with(|budget| {
        budget
            .route
            .consecutive_unreadable
            .store(0, Ordering::Relaxed)
    });
}

fn consecutive_unreadable_roots() -> u32 {
    SOUND_BUDGET
        .try_with(|budget| budget.route.consecutive_unreadable.load(Ordering::Relaxed))
        .unwrap_or(0)
}

/// #47 (Global 45.7.3/en, 2026-10-09): after the opening phase handed off, each inline retry
/// asked the same root twice more and the visual popup check asked it once per frame, ~10.5 s
/// each, until the 180 s budget ran out. Once the stage met ROOT_UNREADABLE_CAP in a row, a
/// phase with a measured visual route starts there. It never taps without a fresh frame.
fn visual_handoff_due(session: &dyn UiSession, plan: SoundPickerPlan) -> bool {
    blind_handoff_reached(consecutive_unreadable_roots())
        && selection_recovery::measured(plan)
        && session.gui_reasoner().is_some()
}

/// How long a blind sheet on one UiAutomator2 server is remembered across attempts. Finite: a
/// later attempt on the same server past this window asks the root again first.
const BLIND_SHEET_MEMORY: Duration = Duration::from_secs(600);
/// (device serial, UiAutomator2 server identity, when the sheet was found blind).
static BLIND_SHEETS: parking_lot::Mutex<Vec<(String, String, Instant)>> =
    parking_lot::const_mutex(Vec::new());

/// The device and the read server behind its session epoch (`<gui epoch>:<agent session>`).
/// A clean app restart keeps the server; an instrumentation restart replaces it.
fn read_server_key(session: &dyn UiSession) -> Option<(String, String)> {
    let serial = session.gui_scope()?.device_id;
    let epoch = session.gui_session_epoch();
    let (_, server) = epoch.rsplit_once(':')?;
    (!serial.is_empty() && !server.is_empty()).then(|| (serial, server.to_owned()))
}

/// The stage handed the sheet to the visual route. Phone 44 (box3150j, 2026-10-10 08:25:53):
/// the any-choice hand-off ended attempt 1, and attempt 2 on the same UiAutomator2 server spent
/// ~2 x 10.5 s of blind `/source` again before the same hand-off.
fn remember_blind_sheet(session: &dyn UiSession) {
    let Some((serial, server)) = read_server_key(session) else {
        return;
    };
    let mut sheets = BLIND_SHEETS.lock();
    sheets.retain(|(known, _, at)| known != &serial && at.elapsed() < BLIND_SHEET_MEMORY);
    sheets.push((serial, server, Instant::now()));
}

/// A recent stage on this device and read server already found the sheet blind: this stage
/// starts with the run of unreadable roots at ROOT_UNREADABLE_CAP, so it asks the sheet root
/// nothing and observes by sight. Every tap still needs fresh frames, and the editor XML proof
/// before Post is unchanged.
fn adopt_remembered_blind_sheet(session: &dyn UiSession, plan: SoundPickerPlan) {
    if !selection_recovery::measured(plan)
        || session.gui_reasoner().is_none()
        || blind_handoff_reached(consecutive_unreadable_roots())
    {
        return;
    }
    let Some((serial, server)) = read_server_key(session) else {
        return;
    };
    let remembered = BLIND_SHEETS.lock().iter().any(|(known, known_server, at)| {
        known == &serial && known_server == &server && at.elapsed() < BLIND_SHEET_MEMORY
    });
    if !remembered {
        return;
    }
    let seeded = SOUND_BUDGET
        .try_with(|budget| {
            budget
                .route
                .consecutive_unreadable
                .fetch_max(ROOT_UNREADABLE_CAP, Ordering::Relaxed);
        })
        .is_ok();
    if seeded {
        tracing::info!(serial=%serial, session=%session.gui_session_epoch(),
            "sound sheet was blind on this read server in a recent attempt; starting in the visual route");
        crate::publish_recovery::note_read(
            "sound",
            "rootUnreadable",
            unreadable_roots(),
            "visualHandoff",
            Some("Lượt trước trên cùng máy và cùng UiAutomator không đọc được cây giao diện bảng nhạc; lượt này đọc bảng nhạc bằng ảnh màn hình ngay, không đọc lại cây giao diện"),
        );
    }
}

/// Recorded once per stage: the visual route reads frames without the XML popup check.
fn note_popup_check_skipped(step: &str) {
    let first = SOUND_BUDGET
        .try_with(|budget| {
            !budget
                .route
                .popup_check_skipped
                .swap(true, Ordering::Relaxed)
        })
        .unwrap_or(false);
    if first {
        crate::publish_recovery::note_read(
            "sound",
            "rootUnreadable",
            unreadable_roots(),
            "visualHandoff",
            Some(&format!(
                "Không đọc được cây giao diện {} lần liên tiếp; bỏ kiểm tra hộp thoại bằng cây giao diện, chỉ bấm theo ảnh màn hình mới (bước: {step})",
                consecutive_unreadable_roots()
            )),
        );
    }
}

fn unreadable_roots() -> u32 {
    SOUND_BUDGET
        .try_with(|budget| budget.route.unreadable_roots.load(Ordering::Relaxed))
        .unwrap_or(0)
}

fn set_selection_route(route: u8) {
    let _ = SOUND_BUDGET.try_with(|budget| budget.route.selection.store(route, Ordering::Relaxed));
}

/// Record which route chose the confirmed sound and how many unreadable roots the stage met.
pub(crate) fn note_selection_route() {
    let Ok((route, roots, recycles, multi_window)) = SOUND_BUDGET.try_with(|budget| {
        (
            budget.route.selection.load(Ordering::Relaxed),
            budget.route.unreadable_roots.load(Ordering::Relaxed),
            budget.route.tab_recycles.load(Ordering::Relaxed),
            budget.route.multi_window_read.load(Ordering::Relaxed),
        )
    }) else {
        return;
    };
    let (outcome, detail) = match route {
        ROUTE_XML => ("xml", "Đã chọn nhạc bằng cây giao diện"),
        ROUTE_VISUAL => (
            "visual",
            "Đã chọn nhạc bằng ảnh màn hình (OCR); tên nhạc vẫn được kiểm lại trước khi Đăng",
        ),
        _ => (
            "priorSelection",
            "Nhạc đã có trên màn chỉnh sửa từ lượt trước; không rõ cách đã chọn",
        ),
    };
    crate::publish_recovery::note_read(
        "sound",
        "selectionRoute",
        1,
        outcome,
        Some(&format!(
            "{detail}; số lần không đọc được cây giao diện: {roots}; đọc bảng nhạc qua mọi cửa sổ (multiWindowRead): {multi_window}; số lần khởi động lại UiAutomator khi bảng nhạc không đọc được (sessionRecycledAfterTabSwitch): {recycles}"
        )),
    );
}

fn note_visual_handoff(step: &str, error: &anyhow::Error) {
    crate::publish_recovery::note_read(
        "sound",
        "rootUnreadable",
        unreadable_roots(),
        "visualHandoff",
        Some(&format!("{error:#} (bước: {step})")),
    );
}

/// A /source read waited for an accessibility root that never came. Unknown, never absent.
fn unreadable_root(error: &anyhow::Error) -> bool {
    error.is::<crate::driver::AccessibilityReadUnavailable>()
        || error
            .downcast_ref::<crate::driver::UiError>()
            .is_some_and(|error| {
                error.op == "hierarchySourceSnapshot"
                    && matches!(error.kind, crate::driver::UiErrorKind::Timeout)
            })
        || source_request_timed_out(error)
}

/// A read-only `GET /source` that reached the Android driver's request deadline. Phone 44
/// (box3150g, 2026-10-10 01:01:39): `agent_request_timeout ... GET /source ... elapsed_ms=30000`
/// on the sound sheet is the same blindness as the ~10.5 s root error, but the driver reports
/// it as a transport timeout, so the stage kept the XML route and never handed off. Only the
/// hierarchy route matches; no effectful request is classified here and nothing is replayed.
fn source_request_timed_out(error: &anyhow::Error) -> bool {
    error.chain().any(|source| {
        source
            .downcast_ref::<reqwest::Error>()
            .is_some_and(|transport| {
                transport.is_timeout()
                    && transport
                        .url()
                        .is_some_and(|url| url.path().ends_with("/source"))
            })
    })
}

/// Consecutive sound-sheet reads found no accessibility root. Only a phase with a measured
/// visual route to hand off to receives this, and it is returned before any tap.
#[derive(Debug, thiserror::Error)]
#[error("Không đọc được cây giao diện bảng nhạc {reads} lần liên tiếp; chuyển sang đọc ảnh màn hình, chưa bấm Đăng")]
pub(crate) struct SoundRootUnreadable {
    reads: u32,
}

/// A sheet read found no root, inside a phase armed to recycle the session once.
/// Returned before any further tap; the arming phase handles it.
#[derive(Debug, thiserror::Error)]
#[error("Không đọc được cây giao diện bảng nhạc; sẽ đọc qua mọi cửa sổ một lần, nếu vẫn không được thì chuyển sang đọc ảnh màn hình (không có OCR thì khởi động lại UiAutomator một lần rồi đọc lại), chưa bấm Đăng")]
pub(crate) struct SoundTabSwitchBlind;

/// The editor root stayed unreadable for a whole editor proof: no read returned a tree.
/// Distinct from a read that showed another title, which is never handed to OCR.
#[derive(Debug, thiserror::Error)]
#[error("Không đọc được màn chỉnh sửa để xác nhận nhạc đã chọn; chưa bấm Đăng")]
pub(crate) struct EditorRootUnreadable;

#[derive(Debug, thiserror::Error)]
#[error("Đã tạm dừng trong lúc chờ TikTok tải nhạc")]
pub(crate) struct SoundStopped;

#[derive(Debug, thiserror::Error)]
#[error("TikTok báo mạng không ổn định khi tải danh sách nhạc; chưa bấm Đăng")]
pub(crate) struct SoundNetworkUnavailable;

/// Only the OCR read failed; the caller must independently prove a measured
/// XML state before navigating. Invalid OCR bindings are not this error.
#[derive(Debug, thiserror::Error)]
#[error("local OCR read unavailable: {0}")]
pub(crate) struct OcrReadUnavailable(#[source] pub anyhow::Error);

pub(crate) async fn reopen_failed_sound_sheet(
    session: &dyn UiSession,
    plan: SoundPickerPlan,
) -> anyhow::Result<bool> {
    let _pin = hold_sound_session(session);
    anyhow::ensure!(
        selection_recovery::measured(plan),
        "sound network recovery unmeasured"
    );
    selection_recovery::prove_sheet(session, plan).await?;
    if visual::retry_network(session, plan).await? {
        return Ok(true);
    }
    check_wait()?;
    session.back().await?;
    check_wait()?;
    Ok(false)
}

/// One budget spans opening, loading, selecting and confirming. The scoped flag
/// lets nested observations honor cancellation without abandoning a live tap.
#[cfg(test)]
pub(crate) async fn with_sound_budget<T>(
    stop: &AtomicBool,
    work: impl std::future::Future<Output = anyhow::Result<T>>,
) -> anyhow::Result<T> {
    with_observation_budget(stop, SOUND_WINDOW, work).await
}

pub(crate) async fn with_readback_budget<T>(
    stop: &AtomicBool,
    work: impl std::future::Future<Output = anyhow::Result<T>>,
) -> anyhow::Result<T> {
    with_observation_budget(stop, READBACK_WINDOW, work).await
}

pub(crate) async fn with_observation_budget<T>(
    stop: &AtomicBool,
    window: Duration,
    work: impl std::future::Future<Output = anyhow::Result<T>>,
) -> anyhow::Result<T> {
    with_deadline_budget(stop, Instant::now() + window, work).await
}

pub(crate) async fn with_deadline_budget<T>(
    stop: &AtomicBool,
    deadline: Instant,
    work: impl std::future::Future<Output = anyhow::Result<T>>,
) -> anyhow::Result<T> {
    if let Ok(budget) = SOUND_BUDGET.try_with(|parent| SoundBudget {
        deadline: parent.deadline.min(deadline),
        stopped: parent.stopped.clone(),
        popup: parent.popup.clone(),
        popup_package: parent.popup_package.clone(),
        route: parent.route.clone(),
    }) {
        return SOUND_BUDGET
            .scope(budget, async {
                check_wait()?;
                finish_budget(Box::pin(work).await)
            })
            .await;
    }
    let stopped = Arc::new(AtomicBool::new(stop.load(Ordering::Relaxed)));
    let budget = SoundBudget {
        deadline,
        stopped: stopped.clone(),
        popup: Default::default(),
        popup_package: Default::default(),
        route: Default::default(),
    };
    let task = SOUND_BUDGET.scope(
        budget,
        crate::tiktok_composer::observation::with_binding(async {
            check_wait()?;
            finish_budget(Box::pin(work).await)
        }),
    );
    tokio::pin!(task);
    let mut poll = tokio::time::interval(POLL);
    loop {
        tokio::select! {
            result=&mut task => {
                if stop.load(Ordering::Relaxed) { return Err(SoundStopped.into()); }
                return result;
            },
            _=poll.tick()=>stopped.store(stop.load(Ordering::Relaxed),Ordering::Relaxed),
        }
    }
}

/// A shorter navigation stage shares the enclosing Stop flag and total deadline.
/// This scopes reads; it never times out the whole future containing a gesture.
pub(crate) async fn with_stage_budget<T>(
    window: Duration,
    work: impl std::future::Future<Output = anyhow::Result<T>>,
) -> anyhow::Result<T> {
    let stage_deadline = Instant::now() + window;
    let budget = SOUND_BUDGET
        .try_with(|parent| SoundBudget {
            deadline: parent.deadline.min(stage_deadline),
            stopped: parent.stopped.clone(),
            popup: parent.popup.clone(),
            popup_package: parent.popup_package.clone(),
            route: parent.route.clone(),
        })
        .unwrap_or_else(|_| SoundBudget {
            deadline: stage_deadline,
            stopped: Arc::new(AtomicBool::new(false)),
            popup: Default::default(),
            popup_package: Default::default(),
            route: Default::default(),
        });
    SOUND_BUDGET
        .scope(budget, async {
            check_wait()?;
            let result = Box::pin(work).await;
            // If a gesture consumed the remaining window, it has now drained.
            // Expiration still refuses later selection/navigation.
            if let Err(error) = check_wait() {
                let detail = error.to_string();
                crate::publish_recovery::note_read(
                    "sound",
                    "boundedStageObservation",
                    1,
                    "stageExhausted",
                    Some(&detail),
                );
                return finish_budget(result);
            }
            result
        })
        .await
}

pub(crate) fn phase_deadline(window: Duration) -> Instant {
    let deadline = Instant::now() + window;
    SOUND_BUDGET
        .try_with(|b| b.deadline.min(deadline))
        .unwrap_or(deadline)
}
pub(crate) fn check_wait() -> anyhow::Result<()> {
    SOUND_BUDGET
        .try_with(|b| {
            if b.stopped.load(Ordering::Relaxed) {
                return Err(SoundStopped.into());
            }
            if Instant::now() >= b.deadline {
                return Err(crate::publish_recovery::retryable_error(
                    "sound_load_timeout",
                    "Hết thời gian chờ tải hoặc xác nhận nhạc ở bước hiện tại; chưa bấm Đăng",
                ));
            }
            Ok(())
        })
        .unwrap_or(Ok(()))
}
fn finish_budget<T>(result: anyhow::Result<T>) -> anyhow::Result<T> {
    match check_wait() {
        Ok(()) => result,
        Err(wait)
            if !wait.is::<SoundStopped>()
                && result.as_ref().is_err_and(|error| {
                    error.is::<crate::driver::SessionEpochChanged>()
                        || crate::publish_recovery::describe(error).kind
                            != crate::publish_recovery::FailureKind::Retryable
                }) =>
        {
            result
        }
        Err(wait) => Err(wait),
    }
}

pub(crate) async fn read_sound<T>(
    work: impl std::future::Future<Output = anyhow::Result<T>>,
) -> anyhow::Result<T> {
    check_wait()?;
    let value = if let Ok(deadline) = SOUND_BUDGET.try_with(|b| b.deadline) {
        let task = tokio::time::timeout_at(deadline, work);
        tokio::pin!(task);
        let mut poll = tokio::time::interval(POLL);
        loop {
            tokio::select! {
                r=&mut task => break r.map_err(|_|crate::publish_recovery::retryable_error(
                    "sound_load_timeout",
                    "Hết thời gian chờ tải hoặc xác nhận nhạc ở bước hiện tại; chưa bấm Đăng",
                ))??,
                _=poll.tick()=>check_wait()?,
            }
        }
    } else {
        work.await?
    };
    check_wait()?;
    Ok(value)
}
/// All nested music waits share the operation's decline budget and Stop flag.
async fn clear_sound_popup(
    session: &dyn UiSession,
    deadline: Instant,
    expected: Option<&'static str>,
) -> anyhow::Result<()> {
    use crate::app_automation::dialogs::{step_before_deadline, PopupStep};
    use crate::ui_automation::runtime::ReadWaitResult;
    if !session.supports_accessibility_readback() {
        return Ok(());
    }
    let Ok((deadline, stopped, popup, bound_package)) = SOUND_BUDGET.try_with(|budget| {
        (
            deadline.min(budget.deadline),
            budget.stopped.clone(),
            budget.popup.clone(),
            budget.popup_package.clone(),
        )
    }) else {
        // Unscoped diagnostics are read-only: never create an independent popup controller.
        return Ok(());
    };
    let mut binding = bound_package.lock().await;
    if let Some(expected) = expected {
        anyhow::ensure!(
            (*binding).is_none_or(|bound| bound == expected),
            "sound popup target changed"
        );
        *binding = Some(expected);
    }
    let package = (*binding).context("sound popup target is not bound")?;
    drop(binding);
    let version =
        read_sound(async { Ok(session.app_version(package).await.unwrap_or_default()) }).await?;
    let language =
        read_sound(async { Ok(session.ui_language().await.unwrap_or_default()) }).await?;
    let labels = crate::tiktok_labels::controls_for_runtime(package, &language, &version)
        .context("sound popup profile unavailable")?;
    let mut popup = popup.lock().await;
    loop {
        let step =
            step_before_deadline(session, labels, &mut popup, deadline, &stopped, &stopped).await;
        match &step {
            Ok(ReadWaitResult::Ready(_)) => note_readable_root(),
            Err(error) if unreadable_root(error) => note_unreadable_read(error),
            _ => {}
        }
        match step? {
            ReadWaitResult::Ready(PopupStep::Clear) => return Ok(()),
            ReadWaitResult::Ready(PopupStep::Dismissed) => {
                tokio::time::sleep(POLL.min(deadline.saturating_duration_since(Instant::now())))
                    .await;
            }
            ReadWaitResult::Ready(PopupStep::Blocked) => {
                anyhow::bail!("sound popup requires operator review")
            }
            ReadWaitResult::Cancelled => return Err(SoundStopped.into()),
            ReadWaitResult::DeadlineExceeded => {
                return Err(crate::publish_recovery::observation_deadline())
            }
        }
    }
}

async fn sound_tap(session: &dyn UiSession, point: crate::TapPoint) -> anyhow::Result<()> {
    sound_tap_armed(session, point, &mut || {}).await
}
async fn sound_tap_armed(
    session: &dyn UiSession,
    point: crate::TapPoint,
    before_tap: &mut (dyn FnMut() + Send),
) -> anyhow::Result<()> {
    check_wait()?;
    crate::tiktok_composer::observation::check_session(session)?;
    before_tap();
    // The sheet's taps go through the OS shell like the visual route's: on 23 phones
    // (13/10/2026) `input swipe` at the measured Hot point switched the tab in 2 s every
    // time, while acknowledged helper `/actions` taps on a blind sheet never landed. Only
    // an unsupported session, which sent nothing, falls back to the helper.
    let contact_ms = {
        use rand::Rng;
        rand::thread_rng().gen_range(45..=130)
    };
    let shell_point = crate::TapPoint { x: point.x, y: point.y };
    match session.tap_outside_automation(shell_point, contact_ms).await {
        Err(error) if error.downcast_ref::<crate::driver::UnsupportedCapability>().is_some() => {
            session.tap(point).await?;
        }
        result => result?,
    }
    check_wait()
}
/// Every pool, tab and row this flow proves is bound to one session epoch, and the composer's
/// sound stage ends on any epoch change. A read-side session replacement can only end the
/// attempt here: #22/#29/#30 on Global 45.7.3 (2026-10-08) each lost the attempt to one, while
/// the replacement read the same unreadable root. So each public sound step holds its session,
/// and an unreadable root stays an unknown read that the existing loops observe again within
/// their unchanged deadlines. No target outlives a session change. The one exception is
/// [`recycle_after_tab_switch`], which the stage asks for itself, once, at an observation
/// boundary, and which re-reads the sheet from nothing on the new session.
fn hold_sound_session(session: &dyn UiSession) -> crate::driver::ReadSessionPin {
    session.pin_read_session()
}

fn transient_sound_read(error: &anyhow::Error) -> bool {
    matches!(
        crate::driver::classify_read_failure(error),
        crate::driver::ReadFailureKind::Transient | crate::driver::ReadFailureKind::Unavailable
    )
}

/// The exact hierarchy shape measured for one TikTok build.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SoundPickerPlan {
    dynamic: bool,
    package: &'static str,
    entry_id: &'static str,
    current_title_id: &'static str,
    section_label: &'static str,
    canonical_section: &'static str,
    row_id: &'static str,
    title_id: &'static str,
    artist_id: &'static str,
    choose_id: Option<&'static str>,
    layout: SoundPickerLayout,
    selection: SoundSelectionMode,
    post_back_id: Option<&'static str>,
    /// The unresolved `content-desc` the post-back control carries on this exact build.
    /// Checked wherever that control is read from a tree; see the 45.7.3 entry.
    post_back_description: Option<&'static str>,
    /// The clickable container wrapping each row, when measured. A row read next to
    /// such containers must sit inside exactly one clickable container.
    row_container_id: Option<&'static str>,
    provenance: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SoundPickerLayout {
    TabbedSnapshot(SoundSnapshotLayout),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SoundSnapshotLayout {
    tab_id: &'static str,
    viewport_id: &'static str,
    boundary_rows: SoundBoundaryRows,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SoundBoundaryRows {
    RequireCompleteText,
    ExcludeBottomEdge,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SoundSelectionMode {
    Inline { marker_ids: &'static [&'static str] },
}

struct MeasuredSoundPicker {
    version: &'static str,
    language: &'static str,
    plan: SoundPickerPlan,
}

// A measurement owns its behavior as well as its locators. Identical IDs across
// versions are not aliases, and an entry ID never chooses a parser or close route.
const MEASURED_SOUND_PICKERS: &[MeasuredSoundPicker] = &[
    MeasuredSoundPicker {
        version: "45.4.3", language: "en",
        plan: SoundPickerPlan {
            dynamic: false,
            package: "com.zhiliaoapp.musically",
            entry_id: ":id/dmk", current_title_id: ":id/zy1",
            section_label: "Hot", canonical_section: "trending",
            row_id: ":id/vertical_item_music_new_rl", title_id: ":id/title",
            artist_id: ":id/yq_", choose_id: None,
            layout: SoundPickerLayout::TabbedSnapshot(SoundSnapshotLayout {
                tab_id: ":id/wf9", viewport_id: ":id/viewpager_container",
                boundary_rows: SoundBoundaryRows::ExcludeBottomEdge,
            }),
            selection: SoundSelectionMode::Inline { marker_ids: &[":id/nd_"] },
            post_back_id: Some(":id/bgy"),
            post_back_description: None,
            row_container_id: None,
            provenance: "SM-G955F ce04171435f104080c, Android 9/en, 08/09/2026 code2024504030; isolated album, ordinal 1/2, Hot, selected sound, caption roundtrip, Send empty/typed/cleared",
        },
    },
    MeasuredSoundPicker {
        version: "46.1.3", language: "en",
        plan: SoundPickerPlan {
            dynamic: false,
            package: "com.zhiliaoapp.musically",
            entry_id: ":id/dta", current_title_id: ":id/tv_top_text",
            section_label: "Hot", canonical_section: "trending",
            row_id: ":id/vertical_item_music_new_rl", title_id: ":id/title",
            artist_id: ":id/zba", choose_id: None,
            layout: SoundPickerLayout::TabbedSnapshot(SoundSnapshotLayout {
                tab_id: ":id/wzs", viewport_id: ":id/viewpager_container",
                boundary_rows: SoundBoundaryRows::ExcludeBottomEdge,
            }),
            selection: SoundSelectionMode::Inline { marker_ids: &[":id/ntf"] },
            post_back_id: Some(":id/bn7"),
            post_back_description: None,
            row_container_id: None,
            provenance: "SM-G955F ce051715e15b2c2e02, Android 9/en, 08/09/2026 code2024601030; isolated album, ordinal 1/2, Hot, selected sound, caption roundtrip, Send empty/typed/cleared",
        },
    },
    MeasuredSoundPicker {
        version: "46.4.3", language: "en",
        plan: SoundPickerPlan {
            dynamic: false,
            package: "com.zhiliaoapp.musically",
            entry_id: ":id/dwh", current_title_id: ":id/tv_top_text",
            section_label: "Hot", canonical_section: "trending",
            row_id: ":id/vertical_item_music_new_rl", title_id: ":id/title",
            artist_id: ":id/zp2", choose_id: None,
            layout: SoundPickerLayout::TabbedSnapshot(SoundSnapshotLayout {
                tab_id: ":id/xbv", viewport_id: ":id/viewpager_container",
                boundary_rows: SoundBoundaryRows::ExcludeBottomEdge,
            }),
            selection: SoundSelectionMode::Inline { marker_ids: &[":id/o2k"] },
            post_back_id: Some(":id/bor"),
            post_back_description: None,
            row_container_id: None,
            provenance: "SM-G955F ce031713aadf361905, Android 9/en, 08/09/2026 code2024604030; isolated album, ordinal 1/2, Hot, selected sound, caption roundtrip, Send empty/typed/cleared",
        },
    },

    // 45.7.3/en, re-read on Riviu #44 (SM-G955F ce031713d903001b01) 09/10/2026 with
    // `uiautomator dump`; fixtures `real-20261009-*`:
    // - The sheet can open on For You, not Hot, so the `wrv` Hot tap stays required.
    // - Each row `vertical_item_music_new_rl` sits in the clickable container `opr`.
    // - `nms` is a Lottie shown only on the row whose preview is playing, so it marks
    //   the tapped row. Tapping that row again stops the preview AND deselects the
    //   sound (the editor chip shows no sound); there is no Pause control on this
    //   build, so a second tap is never a pause.
    // - `bix` (the caption page's Back) is declared in 61 layouts; its content-desc is
    //   the unresolved string reference `@2131823215` ("Back" in the APK's NxString
    //   table). The editor's own Back has the same id and desc, so the caption page
    //   itself stays the proof.
    MeasuredSoundPicker {
        version: "45.7.3",
        language: "en",
        plan: SoundPickerPlan {
            dynamic: false,
            package: "com.zhiliaoapp.musically",
            entry_id: ":id/dou",
            current_title_id: ":id/tv_top_text",
            section_label: "Hot",
            canonical_section: "trending",
            row_id: ":id/vertical_item_music_new_rl",
            title_id: ":id/title",
            artist_id: ":id/z3k",
            choose_id: None,
            layout: SoundPickerLayout::TabbedSnapshot(SoundSnapshotLayout {
                tab_id: ":id/wrv",
                viewport_id: ":id/viewpager_container",
                boundary_rows: SoundBoundaryRows::ExcludeBottomEdge,
            }),
            selection: SoundSelectionMode::Inline { marker_ids: &[":id/nms"] },
            post_back_id: Some(":id/bix"),
            post_back_description: Some("@2131823215"),
            row_container_id: Some(":id/opr"),
            provenance: "musically/en/45.7.3 code2024507030, measured 2026-09-08 on ce031713b0c610ab0c; three-photo selection, Hot, selected row/editor/caption return",
        },
    },
    MeasuredSoundPicker {
        version: "38.3.2",
        language: "en",
        plan: SoundPickerPlan {
            dynamic: false,
            package: "com.ss.android.ugc.trill",
            entry_id: ":id/c_4",
            current_title_id: ":id/so9",
            section_label: "Recommended",
            canonical_section: "recommended",
            row_id: ":id/ta8",
            title_id: ":id/title",
            artist_id: ":id/rr5",
            // dfu is the trim scissors, not a choose control (live 2026-09-06).
            choose_id: None,
            layout: SoundPickerLayout::TabbedSnapshot(SoundSnapshotLayout {
                tab_id: ":id/q_g", viewport_id: ":id/tka",
                boundary_rows: SoundBoundaryRows::RequireCompleteText,
            }),
            selection: SoundSelectionMode::Inline {
                marker_ids: &[":id/dfu", ":id/jk1"],
            },
            post_back_id: Some(":id/aun"),
            post_back_description: None,
            row_container_id: None,
            provenance: "trill/en/38.3.2, measured 2026-09-04 on 9889db374744474635",
        },
    },
    MeasuredSoundPicker {
        version: "46.2.1",
        language: "en",
        plan: SoundPickerPlan {
            dynamic: false,
            package: "com.zhiliaoapp.musically",
            entry_id: ":id/dvc",
            current_title_id: ":id/tv_top_text",
            section_label: "Hot",
            canonical_section: "trending",
            row_id: ":id/vertical_item_music_new_rl",
            title_id: ":id/title",
            artist_id: ":id/zgj",
            // This layout has no dedicated choose icon. The measured title area is the
            // row's stable selection target; the readback below still decides whether it
            // took.
            choose_id: None,
            layout: SoundPickerLayout::TabbedSnapshot(SoundSnapshotLayout {
                tab_id: ":id/x4y", viewport_id: ":id/viewpager_container",
                boundary_rows: SoundBoundaryRows::ExcludeBottomEdge,
            }),
            selection: SoundSelectionMode::Inline { marker_ids: &[":id/nx3"] },
            post_back_id: Some(":id/bot"),
            post_back_description: None,
            row_container_id: None,
            provenance: "musically/en/46.2.1, remeasured 08-09-2026 on ce04171411ae6a1504; Hot inline marker, Back, editor title and caption roundtrip; AGENTS.md ?9.189",
        },
    },
    MeasuredSoundPicker {
        version: "46.2.42",
        language: "en",
        plan: SoundPickerPlan {
            dynamic: false,
            package: "com.zhiliaoapp.musically",
            entry_id: ":id/dv3",
            current_title_id: ":id/tv_top_text",
            section_label: "Hot",
            canonical_section: "trending",
            row_id: ":id/vertical_item_music_new_rl",
            title_id: ":id/title",
            artist_id: ":id/zdw",
            choose_id: None,
            layout: SoundPickerLayout::TabbedSnapshot(SoundSnapshotLayout {
                tab_id: ":id/x2k",
                viewport_id: ":id/viewpager_container",
                boundary_rows: SoundBoundaryRows::RequireCompleteText,
            }),
            selection: SoundSelectionMode::Inline {
                marker_ids: &[":id/nve"],
            },
            post_back_id: Some(":id/bot"),
            post_back_description: None,
            row_container_id: None,
            provenance: "musically/en/46.2.42, measured 2026-09-04 on ce0517155ab38c390d",
        },
    },
    MeasuredSoundPicker {
        version: "46.0.41",
        language: "en",
        plan: SoundPickerPlan {
            dynamic: false,
            package: "com.zhiliaoapp.musically",
            entry_id: ":id/dsv",
            current_title_id: ":id/tv_top_text",
            section_label: "Hot",
            canonical_section: "trending",
            row_id: ":id/vertical_item_music_new_rl",
            title_id: ":id/title",
            artist_id: ":id/z_g",
            choose_id: None,
            layout: SoundPickerLayout::TabbedSnapshot(SoundSnapshotLayout {
                tab_id: ":id/wy5",
                viewport_id: ":id/viewpager_container",
                // The artist is visibly clipped despite both text nodes existing.
                boundary_rows: SoundBoundaryRows::ExcludeBottomEdge,
            }),
            selection: SoundSelectionMode::Inline { marker_ids: &[":id/nrm"] },
            post_back_id: Some(":id/bmy"),
            post_back_description: None,
            row_container_id: None,
            provenance: "musically/en/46.0.41 code2024600410, measured 2026-09-08 on ce011711c354be2005; selected row/editor/caption return verified before Post",
        },
    },
];

impl SoundPickerPlan {
    pub fn package(self) -> &'static str {
        self.package
    }
    pub fn resolve_runtime(package: &str, locale: &str, version: &str) -> Option<Self> {
        Self::resolve(package, locale, version).or_else(|| {
            let language = crate::tiktok_labels::normalise_language(locale);
            MEASURED_SOUND_PICKERS
                .iter()
                .find(|p| p.plan.package == package && p.language == language)
                .map(|p| {
                    let mut plan = p.plan;
                    plan.dynamic = true;
                    plan
                })
        })
    }
    /// The plan's own pool is the Hot tab (Global musically), so a For You row is never a
    /// publish sound on it (owner rule 2026-10-12, "Bắt buộc Hot").
    pub(crate) fn picks_from_hot(self) -> bool {
        self.section_label == "Hot"
    }
    pub(crate) fn post_back_query(self) -> Option<ElementQuery<'static>> {
        self.post_back_id.map(ElementQuery::ResourceIdSuffix)
    }
    pub(crate) fn post_back_description(self) -> Option<&'static str> {
        self.post_back_description
    }

    fn snapshot_layout(self) -> Option<SoundSnapshotLayout> {
        match self.layout {
            SoundPickerLayout::TabbedSnapshot(layout) => Some(layout),
        }
    }

    fn closes_with_back(self) -> bool {
        matches!(self.selection, SoundSelectionMode::Inline { .. })
    }

    fn selected_marker_ids(self) -> &'static [&'static str] {
        match self.selection {
            SoundSelectionMode::Inline { marker_ids } => marker_ids,
        }
    }

    /// Resolve only an exact build/locale tuple measured on the attached fleet.
    pub fn resolve(package: &str, locale: &str, version: &str) -> Option<Self> {
        let language = locale
            .trim()
            .split(['-', '_'])
            .next()
            .unwrap_or_default()
            .to_ascii_lowercase();
        MEASURED_SOUND_PICKERS
            .iter()
            .find(|entry| {
                entry.plan.package == package.trim()
                    && entry.language == language
                    && entry.version == version.trim()
            })
            .map(|entry| entry.plan)
    }

    pub fn provenance(self) -> &'static str {
        self.provenance
    }
}

/// One observed pool plus the exact row targets that produced it.
#[derive(Debug, Clone, PartialEq)]
pub struct ObservedSoundPool {
    visual: bool,
    effective_plan: Option<SoundPickerPlan>,
    pub candidates: Vec<SoundCandidate>,
    maximum_visible: usize,
    targets: Vec<ElementBox>,
    selected_index: Option<usize>,
}

impl ObservedSoundPool {
    fn stable_with(&self, other: &Self) -> bool {
        self.candidates == other.candidates
            && self.selected_index == other.selected_index
            && self.targets.len() == other.targets.len()
            && self.targets.iter().zip(&other.targets).all(|(a, b)| {
                a.enabled
                    && b.enabled
                    && a.width > 0.0
                    && a.height > 0.0
                    && b.width > 0.0
                    && b.height > 0.0
                    && (a.x - b.x).abs() <= 4.0
                    && (a.y - b.y).abs() <= 4.0
                    && (a.width - b.width).abs() <= 4.0
                    && (a.height - b.height).abs() <= 4.0
            })
    }
    pub fn effective_plan(&self, fallback: SoundPickerPlan) -> SoundPickerPlan {
        self.effective_plan.unwrap_or(fallback)
    }
    pub fn target(&self, index: usize) -> Option<&ElementBox> {
        self.targets.get(index)
    }
    pub fn selected_index(&self) -> Option<usize> {
        self.selected_index
    }
}

/// Open the measured picker and read at most `maximum_visible` rows without selecting one.
pub async fn open_and_observe_sounds(
    session: &dyn UiSession,
    plan: SoundPickerPlan,
    maximum_visible: usize,
) -> anyhow::Result<ObservedSoundPool> {
    open_and_observe_sounds_armed(session, plan, maximum_visible, &mut || {}).await
}

pub async fn open_and_observe_sounds_armed(
    session: &dyn UiSession,
    plan: SoundPickerPlan,
    maximum_visible: usize,
    before_open: &mut (dyn FnMut() + Send),
) -> anyhow::Result<ObservedSoundPool> {
    open_and_observe_sounds_armed_current(session, plan, maximum_visible, before_open, false).await
}

/// `allow_recommended` lets a measured sheet that opened on For You be read as a pool. The
/// publish composer never sets it: a publish sound comes from Hot only (owner rule 2026-10-12,
/// "Bắt buộc Hot"), through [`open_and_observe_sounds_armed`] and [`resume_open_sounds`].
pub async fn open_and_observe_sounds_armed_current(
    session: &dyn UiSession,
    plan: SoundPickerPlan,
    maximum_visible: usize,
    before_open: &mut (dyn FnMut() + Send),
    allow_recommended: bool,
) -> anyhow::Result<ObservedSoundPool> {
    let _pin = hold_sound_session(session);
    clear_sound_popup(session, phase_deadline(SOUND_WINDOW), Some(plan.package)).await?;
    anyhow::ensure!(
        (1..=5).contains(&maximum_visible),
        "sound observer limit must be within 1..=5"
    );
    if plan.dynamic {
        return open_dynamic_sounds(session, plan, maximum_visible, before_open).await;
    }
    if plan.package == "com.ss.android.ugc.trill"
        && plan.entry_id == ":id/c_4"
        && !session.gui_session_epoch().is_empty()
    {
        // Trill 38.3.2, ce031713dd735a1103, 28/09/2026: entry discovery
        // stalled in serial /elements + /rect before any sound tap. Read the
        // measured entry and its actual enabled/clickable flags in fresh XML.
        let epoch = session.gui_session_epoch();
        let entry = tokio::time::timeout(
            Duration::from_secs(30),
            prove_sound_entry_xml(session, plan, &epoch),
        )
        .await
        .context("sound entry XML proof timed out")??;
        check_wait()?;
        sound_tap_armed(session, entry.centre(), before_open).await?;
        return observe_sound_pool(session, plan, maximum_visible).await;
    }
    match visual::open_loading_entry(session, plan, before_open).await {
        Ok(true) => return observe_measured_sound_pool(session, plan, maximum_visible, allow_recommended).await,
        Err(error)
            if selection_recovery::measured(plan)
                && error.is::<crate::driver::ScreenshotReadUnavailable>() =>
        {
            return open_sound_without_image(session, plan, maximum_visible, before_open, allow_recommended).await;
        }
        Err(error) => return Err(error),
        Ok(false) => {}
    }
    let deadline = phase_deadline(SOUND_WINDOW);
    let entry = loop {
        check_wait()?;
        let entries =
            read_sound(session.locate_all(ElementQuery::ResourceIdSuffix(plan.entry_id))).await?;
        if let [only] = entries.as_slice() {
            if only.enabled && only.width > 0.0 && only.height > 0.0 {
                break only.clone();
            }
        }
        if entries.is_empty() && !session.gui_session_epoch().is_empty() {
            return open_dynamic_sounds(session, plan, maximum_visible, before_open).await;
        }
        // A fresh tree removes hidden ancestors and duplicate nested wrappers.
        let tree = crate::ui_automation::tree::Tree::parse(
            read_sound(session.hierarchy_source_snapshot()).await?,
        )?;
        if let Some(button) = unique_sound_entry(&tree, plan.package, &[plan.entry_id]) {
            break button;
        }
        anyhow::ensure!(
            Instant::now() < deadline,
            "Chưa tìm được duy nhất nút nhạc sẵn sàng; chưa bấm Đăng"
        );
        tokio::time::sleep(POLL).await;
    };
    sound_tap_armed(session, entry.centre(), before_open)
        .await
        .context("open sound picker")?;

    if session.gui_reasoner().is_none() {
        if let Some(pool) = observe_current_recommended(session, plan, maximum_visible, allow_recommended).await? {
            return Ok(pool);
        }
    }
    if selection_recovery::measured(plan) && session.gui_reasoner().is_some() {
        return observe_measured_sound_pool(session, plan, maximum_visible, allow_recommended).await;
    }
    if plan.snapshot_layout().is_some() {
        snapshot::select_section_tab(session, plan).await?;
    }

    observe_sound_pool(session, plan, maximum_visible).await
}

async fn open_sound_without_image(
    session: &dyn UiSession,
    plan: SoundPickerPlan,
    maximum_visible: usize,
    before_open: &mut (dyn FnMut() + Send),
    allow_recommended: bool,
) -> anyhow::Result<ObservedSoundPool> {
    let epoch = session.gui_session_epoch();
    anyhow::ensure!(!epoch.is_empty(), "sound sheet session missing");
    let entry = prove_sound_entry_xml(session, plan, &epoch).await?;
    sound_tap_armed(session, entry.centre(), before_open).await?;
    if let Some(pool) = observe_current_recommended(session, plan, maximum_visible, allow_recommended).await? {
        return checked_measured_sound_observation(session, plan, &epoch, Ok(pool)).await;
    }
    // With no screenshot, only an already-selected Hot tab in two fresh XML
    // pools can authorize a row. Do not enter the image-based tab recovery.
    let pool = async {
        snapshot::select_section_tab_xml_only(session, plan).await?;
        snapshot::observe_xml_only(session, plan, maximum_visible).await
    }
    .await;
    checked_measured_sound_observation(session, plan, &epoch, pool).await
}

async fn prove_sound_entry_xml(
    session: &dyn UiSession,
    plan: SoundPickerPlan,
    epoch: &str,
) -> anyhow::Result<ElementBox> {
    let deadline = phase_deadline(SOUND_WINDOW);
    let mut previous: Option<(u64, ElementBox)> = None;
    let mut stable = 0usize;
    loop {
        clear_sound_popup(session, deadline, Some(plan.package)).await?;
        check_wait()?;
        anyhow::ensure!(
            session.gui_session_epoch() == epoch
                && read_sound(session.active_app_bundle()).await? == plan.package,
            "sound app/session changed before opening picker"
        );
        let source = read_sound(session.hierarchy_source_snapshot()).await?;
        let tree = crate::ui_automation::tree::Tree::parse(source)?;
        let current = unique_sound_entry(&tree, plan.package, &[plan.entry_id]);
        // No entry tap has been armed. A changing editor can be observed again
        // inside this same phase budget; it never authorizes a stale target.
        if let Some(button) = current {
            let matched = previous.as_ref().is_some_and(|(generation, old)|
                tree.generation > *generation && old == &button);
            stable = if matched { stable + 1 } else { 1 };
            if let Some((_, old)) = &previous {
                if !matched {
                    tracing::info!(serial=?session.gui_scope().map(|scope|scope.device_id),
                        session=%epoch, generation=tree.generation,
                        geometry_unchanged=old.x==button.x && old.y==button.y
                            && old.width==button.width && old.height==button.height,
                        description_unchanged=old.description==button.description,
                        remaining_ms=deadline.saturating_duration_since(Instant::now()).as_millis() as u64,
                        "sound entry observation changed before dispatch");
                }
            }
            previous = Some((tree.generation, button.clone()));
            if stable >= 3 {
                check_wait()?;
                anyhow::ensure!(session.gui_session_epoch() == epoch
                    && read_sound(session.active_app_bundle()).await? == plan.package,
                    "sound app/session changed before opening picker");
                return Ok(button);
            }
        } else {
            previous = None;
            stable = 0;
        }
        anyhow::ensure!(Instant::now() < deadline,
            crate::publish_recovery::retryable_error("sound_entry_observation_changed",
                "sound entry did not stabilize before tap; no Post was sent"));
        tokio::time::sleep(POLL).await;
    }
}

/// Observe a sheet that this attempt may already have opened. This path never
/// taps the editor entry, so a lost open ACK cannot open a second surface.
pub async fn resume_open_sounds(
    session: &dyn UiSession,
    plan: SoundPickerPlan,
    maximum: usize,
) -> anyhow::Result<ObservedSoundPool> {
    resume_open_sounds_current(session, plan, maximum, false).await
}

pub async fn resume_open_sounds_current(
    session: &dyn UiSession,
    plan: SoundPickerPlan,
    maximum: usize,
    allow_recommended: bool,
) -> anyhow::Result<ObservedSoundPool> {
    let _pin = hold_sound_session(session);
    // After the hand-off the visual route below proves the sheet from fresh frames; asking the
    // same unreadable root for popups first would only spend another ~10.5 s per retry.
    if (!allow_recommended || !selection_recovery::measured(plan))
        && !visual_handoff_due(session, plan)
    {
        clear_sound_popup(session, phase_deadline(SOUND_WINDOW), Some(plan.package)).await?;
    }
    if selection_recovery::measured(plan) && session.gui_reasoner().is_some() {
        return observe_measured_sound_pool(session, plan, maximum, allow_recommended).await;
    }
    if let Some(pool) = observe_current_recommended(session, plan, maximum, allow_recommended).await? {
        return Ok(pool);
    }
    if allow_recommended && selection_recovery::measured(plan) {
        clear_sound_popup(session, phase_deadline(SOUND_WINDOW), Some(plan.package)).await?;
    }
    if let Some(layout) = plan.snapshot_layout() {
        if !read_sound(session.locate_all(ElementQuery::ResourceIdSuffix(layout.tab_id)))
            .await?
            .is_empty()
        {
            snapshot::select_section_tab(session, plan).await?;
            return observe_sound_pool(session, plan, maximum).await;
        }
    } else if !read_sound(session.locate_all(ElementQuery::ResourceIdSuffix(plan.row_id)))
        .await?
        .is_empty()
    {
        return observe_sound_pool(session, plan, maximum).await;
    }
    anyhow::bail!("sound sheet is not proven open; refusing to replay its editor entry")
}

pub(crate) async fn recover_frozen_sound_pool(
    session: &dyn UiSession,
    plan: SoundPickerPlan,
    selection: &crate::SoundSelectionEvidence,
) -> anyhow::Result<ObservedSoundPool> {
    let _pin = hold_sound_session(session);
    with_stage_budget(
        Duration::from_secs(60),
        recent::recover(session, plan, selection),
    )
    .await
    .map_err(classify_recent_observation_error)
    .context("frozen sound Recent recovery did not finish within its observation budget")
}

fn classify_recent_observation_error(error: anyhow::Error) -> anyhow::Error {
    if error.is::<recent::SessionChanged>() {
        error.context(crate::publish_recovery::RecoveryFailure::new(
            "sound_recent_observation_invalidated",
            crate::publish_recovery::FailureKind::Retryable,
            "Recent recovery observation session changed; prepare fresh without replaying selection",
        ))
    } else {
        error
    }
}

// Passive parent-route timings use the enclosing phase budget, never a new timeout.
struct SoundRouteDiagnostics {
    scope: Option<crate::ui_automation::GuiScope>,
    epoch: String,
    deadline: Option<Instant>,
    events: u8,
}
impl SoundRouteDiagnostics {
    fn emit(
        &mut self,
        route: &'static str,
        phase: &'static str,
        started: Instant,
        outcome: &'static str,
        error: Option<&anyhow::Error>,
    ) {
        if self.events >= 8 {
            return;
        }
        self.events += 1;
        let now = Instant::now();
        let error_code = error.map(|error| crate::publish_recovery::describe(error).code);
        tracing::info!(
            serial=?self.scope.as_ref().map(|scope| scope.device_id.as_str()),
            runId=?self.scope.as_ref().map(|scope| scope.run_id.as_str()),
            assignmentId=?self.scope.as_ref().and_then(|scope| scope.assignment_id.as_deref()),
            session=%self.epoch, route, phase, outcome, errorCode=?error_code,
            elapsedMs=now.saturating_duration_since(started).as_millis().min(u64::MAX as u128) as u64,
            remainingPhaseMs=?self.deadline.map(|deadline| deadline.saturating_duration_since(now)
                .as_millis().min(u64::MAX as u128) as u64),
            phaseBudgetSource="enclosingSoundBudget",
            "sound observation route diagnostic"
        );
    }
}

async fn observe_measured_sound_pool(
    session: &dyn UiSession,
    plan: SoundPickerPlan,
    maximum: usize,
    allow_recommended: bool,
) -> anyhow::Result<ObservedSoundPool> {
    let armed = RecycleArmed::arm(session);
    let observed =
        observe_measured_sound_pool_once(session, plan, maximum, allow_recommended).await;
    drop(armed);
    let blind = match observed {
        Err(error) if error.is::<SoundTabSwitchBlind>() && !error.is::<SoundStopped>() => error,
        observed => return observed,
    };
    // The blind phase returned before any further tap and holds no target. Phones 44/47
    // (Global 45.7.3/en, 2026-10-09 21:36): the active-window root stayed null on a fresh
    // server too, so a tree built from every window is asked first, on this same session.
    if let Some(observed) =
        reread_through_every_window(session, plan, maximum, allow_recommended).await
    {
        return observed;
    }
    // #47 (box3150e, 2026-10-10 00:17): the every-window tree was blind (unreadableRoots=3),
    // the restart finished ok=true in 6.6 s, and the fresh server answered the same sheet
    // with two more ~10.5 s unreadable roots. On this sheet the restart buys nothing; with
    // ROOT_UNREADABLE_CAP met, a session with a measured visual route hands off at once and
    // asks no further `/source`. The restart remains for sessions without that route.
    if visual_handoff_due(session, plan) {
        return observe_visual_after_blind_windows(session, plan, maximum).await;
    }
    match recycle_after_tab_switch(session, plan, "đọc bảng nhạc").await {
        Ok(()) => {
            observe_after_switch(
                session,
                plan,
                maximum,
                allow_recommended,
                "readAfterSessionRecycle",
            )
            .await
        }
        Err(recycle) if recycle.is::<SoundStopped>() => Err(recycle),
        // A refused or failed restart ends this attempt as an unreadable root, before any tap.
        // The cap is spent and the run of unreadable roots kept, so the next attempt starts on
        // the existing route and hands off to the visual route as before.
        Err(recycle) => {
            tracing::warn!(blind = %format!("{blind:#}"), recycle = %format!("{recycle:#}"),
                "sound session recycle unavailable; attempt ends before any tap");
            Err(anyhow::Error::new(SoundRootUnreadable {
                reads: consecutive_unreadable_roots(),
            })
            .context(format!("không khởi động lại được UiAutomator: {recycle:#}")))
        }
    }
}

/// The sheet observed by sight after the every-window tree was blind as well. No `/source`
/// is asked here: the frames skip the XML popup check past ROOT_UNREADABLE_CAP, Hot is proved
/// by the selected-tab underline on fresh frames, and the pool returned binds rows by OCR
/// only. The row tap and the editor proof stay with the existing visual `choose` path, so
/// nothing here selects a sound or reaches Post.
async fn observe_visual_after_blind_windows(
    session: &dyn UiSession,
    plan: SoundPickerPlan,
    maximum: usize,
) -> anyhow::Result<ObservedSoundPool> {
    let epoch = session.gui_session_epoch();
    anyhow::ensure!(!epoch.is_empty(), "sound sheet session missing");
    let blind = anyhow::Error::new(SoundRootUnreadable {
        reads: consecutive_unreadable_roots(),
    });
    tracing::warn!(serial=?session.gui_scope().map(|scope| scope.device_id), session=%epoch,
        unreadableRoots=unreadable_roots(), consecutive=consecutive_unreadable_roots(),
        "sound sheet blind through every window; handing off to the visual route without an instrumentation restart");
    note_visual_handoff("đọc bảng nhạc qua mọi cửa sổ", &blind);
    remember_blind_sheet(session);
    let mut diagnostics = SoundRouteDiagnostics {
        scope: session.gui_scope(),
        epoch: epoch.clone(),
        deadline: SOUND_BUDGET.try_with(|budget| budget.deadline).ok(),
        events: 0,
    };
    let started = Instant::now();
    diagnostics.emit("visual", "start", started, "pending", None);
    let visual = visual::observe_initial(session, plan, maximum).await;
    diagnostics.emit(
        "visual",
        "finish",
        started,
        if visual.is_ok() { "ready" } else { "error" },
        visual.as_ref().err(),
    );
    checked_measured_sound_observation(session, plan, &epoch, visual).await
}

/// Switch this session's tree reads to every window once per stage, then read the sheet again
/// on the same session and epoch. `None` means the switch is unsupported, spent, refused or
/// the every-window tree was blind as well; the caller falls back to the restart. `Some` is
/// the sheet read on the every-window tree, or an error that is not about readability.
async fn reread_through_every_window(
    session: &dyn UiSession,
    plan: SoundPickerPlan,
    maximum: usize,
    allow_recommended: bool,
) -> Option<anyhow::Result<ObservedSoundPool>> {
    let step = "đọc bảng nhạc";
    let note = |outcome: &str, detail: String| {
        crate::publish_recovery::note_read(
            "sound",
            "multiWindowRead",
            multi_window_switches(),
            outcome,
            Some(&format!(
                "{detail} (bước: {step}; số lần không đọc được: {})",
                unreadable_roots()
            )),
        );
    };
    if !session.can_read_multi_window() {
        note("unsupported", "Phiên này không đọc được cây giao diện qua mọi cửa sổ; chuyển sang khởi động lại UiAutomator".into());
        return None;
    }
    let claimed = SOUND_BUDGET
        .try_with(|budget| {
            budget
                .route
                .multi_window_switches
                .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |used| {
                    (used < MULTI_WINDOW_SWITCH_CAP).then_some(used + 1)
                })
                .is_ok()
        })
        .unwrap_or(false);
    if !claimed {
        note(
            "capReached",
            "Đã dùng hết lượt đọc qua mọi cửa sổ của bước nhạc này".into(),
        );
        return None;
    }
    if let Err(stopped) = check_wait() {
        return Some(Err(stopped));
    }
    let roots = unreadable_roots();
    let epoch = session.gui_session_epoch();
    let switch_started = Instant::now();
    let switched = read_sound(session.enable_multi_window_read(plan.package)).await;
    let switch_ms = switch_started.elapsed().as_millis() as u64;
    if let Err(error) = switched {
        if error.is::<SoundStopped>() {
            return Some(Err(error));
        }
        tracing::warn!(serial=?session.gui_scope().map(|scope| scope.device_id), session=%epoch,
            switchMs=switch_ms, unreadableRoots=roots, error=%format!("{error:#}"),
            "sound sheet every-window read unavailable; falling back to the instrumentation restart");
        note("failed", format!(
            "Không bật được chế độ đọc qua mọi cửa sổ (enableMultiWindows) sau {switch_ms} ms: {error:#}; chuyển sang khởi động lại UiAutomator"
        ));
        return None;
    }
    let _ = SOUND_BUDGET.try_with(|budget| {
        budget
            .route
            .multi_window_read
            .store(true, Ordering::Relaxed)
    });
    // The epoch is unchanged: nothing read before was ever a target, and the guards that bind
    // a tap to a fresh generation, this epoch and the foreground app run unchanged below.
    let read_started = Instant::now();
    let observed =
        observe_after_switch(session, plan, maximum, allow_recommended, "multiWindowRead").await;
    let read_ms = read_started.elapsed().as_millis() as u64;
    match observed {
        Ok(pool) => {
            tracing::info!(serial=?session.gui_scope().map(|scope| scope.device_id), session=%epoch,
                multiWindowRead=true, switchMs=switch_ms, readMs=read_ms, unreadableRoots=roots,
                candidates=pool.candidates.len(), "sound sheet read through every window");
            note("ready", format!(
                "multiWindowRead: true; bật chế độ đọc qua mọi cửa sổ mất {switch_ms} ms, đọc lại bảng nhạc mất {read_ms} ms: {} nhạc, tab {}",
                pool.candidates.len(),
                pool.effective_plan(plan).section_label
            ));
            Some(Ok(pool))
        }
        Err(error) if error.is::<SoundStopped>() => Some(Err(error)),
        Err(error) if error.is::<SoundRootUnreadable>() || unreadable_root(&error) => {
            let next = if visual_handoff_due(session, plan) {
                "visual"
            } else {
                "instrumentationRestart"
            };
            tracing::warn!(serial=?session.gui_scope().map(|scope| scope.device_id), session=%epoch,
                multiWindowRead=true, switchMs=switch_ms, readMs=read_ms, unreadableRoots=unreadable_roots(),
                next, error=%format!("{error:#}"),
                "sound sheet blind through every window too");
            note("unreadable", format!(
                "multiWindowRead: true; bật chế độ đọc qua mọi cửa sổ mất {switch_ms} ms, đọc lại {read_ms} ms vẫn không có cây giao diện của TikTok: {error:#}; {}",
                if next == "visual" {
                    "chuyển sang đọc ảnh màn hình ngay, không khởi động lại UiAutomator"
                } else {
                    "chuyển sang khởi động lại UiAutomator"
                }
            ));
            None
        }
        Err(error) => {
            note("error", format!(
                "multiWindowRead: true; bật chế độ đọc qua mọi cửa sổ mất {switch_ms} ms, đọc lại {read_ms} ms rồi lỗi: {error:#}"
            ));
            Some(Err(error))
        }
    }
}

fn multi_window_switches() -> u32 {
    SOUND_BUDGET
        .try_with(|budget| budget.route.multi_window_switches.load(Ordering::Relaxed))
        .unwrap_or(0)
}

/// Return the session to active-window reads when the sound stage ends, whatever the stage
/// returned. Idempotent: a session that never switched answers `false` and nothing is noted.
/// A restore that cannot be proven has sealed the session in the driver; the error returned
/// here must end the attempt before the editor or Post is read on that session.
pub(crate) async fn end_multi_window_read(session: &dyn UiSession) -> anyhow::Result<()> {
    let started = Instant::now();
    let restored = session.restore_single_window_read().await;
    let ms = started.elapsed().as_millis() as u64;
    match restored {
        Ok(false) => Ok(()),
        Ok(true) => {
            tracing::info!(serial=?session.gui_scope().map(|scope| scope.device_id),
                session=%session.gui_session_epoch(), restoreMs=ms,
                "sound stage restored active-window reads");
            crate::publish_recovery::note_read(
                "sound",
                "multiWindowRestore",
                1,
                "restored",
                Some(&format!("Đã tắt chế độ đọc qua mọi cửa sổ (enableMultiWindows: false) sau {ms} ms và đọc lại xác nhận")),
            );
            Ok(())
        }
        Err(error) => {
            tracing::warn!(serial=?session.gui_scope().map(|scope| scope.device_id),
                session=%session.gui_session_epoch(), restoreMs=ms, error=%format!("{error:#}"),
                "sound stage could not restore active-window reads; the session is quarantined");
            crate::publish_recovery::note_read(
                "sound",
                "multiWindowRestore",
                1,
                "failed",
                Some(&format!("Không tắt được chế độ đọc qua mọi cửa sổ sau {ms} ms; phiên bị cách ly, không dùng cho màn chỉnh sửa hay Đăng: {error:#}")),
            );
            Err(error.context(crate::publish_recovery::RecoveryFailure::new(
                "sound_multi_window_restore_failed",
                crate::publish_recovery::FailureKind::Retryable,
                "Không khôi phục được chế độ đọc một cửa sổ của UiAutomator sau khi chọn nhạc; phiên bị cách ly, chưa bấm Đăng. Lượt thử mới sẽ mở phiên khác.",
            )))
        }
    }
}

/// The sheet read again from nothing after the read session was switched or replaced. The
/// selected tab is proved from fresh reads, and Hot is tapped at most once, only when two
/// fresh reads prove it is not selected. A blind root here is not recycled again: the error
/// goes to the caller, whose next attempt starts on the existing visual route.
async fn observe_after_switch(
    session: &dyn UiSession,
    plan: SoundPickerPlan,
    maximum: usize,
    allow_recommended: bool,
    strategy: &'static str,
) -> anyhow::Result<ObservedSoundPool> {
    let epoch = session.gui_session_epoch();
    anyhow::ensure!(!epoch.is_empty(), "sound sheet session missing");
    let pool = async {
        // A policy that accepts the current list keeps it rather than switching tabs again.
        if let Some(pool) =
            observe_current_recommended_routed(session, plan, maximum, allow_recommended, true)
                .await?
        {
            return Ok(pool);
        }
        snapshot::select_section_tab_xml_only(session, plan).await?;
        snapshot::observe_xml_only(session, plan, maximum).await
    }
    .await;
    // The every-window read writes its own note with the timings.
    if strategy == "readAfterSessionRecycle" {
        crate::publish_recovery::note_read(
            "sound",
            strategy,
            tab_recycles(),
            if pool.is_ok() { "ready" } else { "unavailable" },
            Some(&match &pool {
                Ok(pool) => format!(
                    "Phiên đọc mới đọc được bảng nhạc: {} nhạc, tab {}",
                    pool.candidates.len(),
                    pool.effective_plan(plan).section_label
                ),
                Err(error) => format!("Phiên đọc mới vẫn chưa đọc được bảng nhạc: {error:#}"),
            }),
        );
    }
    checked_measured_sound_observation(session, plan, &epoch, pool).await
}

/// Item 5 of the round-2 plan (#44/#47): the late XML route just proved this pool from two
/// fresh reads. Within one POLL on the same epoch, the row tap reuses it instead of asking the
/// root twice more (~21 s on those phones). Taken once; anything older is read again.
fn take_fresh_late_xml(
    session: &dyn UiSession,
    pool: &ObservedSoundPool,
) -> Option<ObservedSoundPool> {
    let (epoch, proved, cached) = SOUND_BUDGET
        .try_with(|budget| budget.route.late_xml.lock().take())
        .ok()
        .flatten()?;
    (epoch == session.gui_session_epoch() && proved.elapsed() < POLL && cached == *pool)
        .then_some(cached)
}

async fn observe_measured_sound_pool_once(
    session: &dyn UiSession,
    plan: SoundPickerPlan,
    maximum: usize,
    allow_recommended: bool,
) -> anyhow::Result<ObservedSoundPool> {
    anyhow::ensure!(
        selection_recovery::measured(plan)
            && plan.package == "com.zhiliaoapp.musically"
            && plan.snapshot_layout().is_some(),
        "direct sound XML proof is restricted to measured Global 45.7.3"
    );
    let epoch = session.gui_session_epoch();
    anyhow::ensure!(!epoch.is_empty(), "sound sheet session missing");
    let mut diagnostics = SoundRouteDiagnostics {
        scope: session.gui_scope(), epoch: epoch.clone(),
        deadline: SOUND_BUDGET.try_with(|budget| budget.deadline).ok(), events: 0,
    };
    let started = Instant::now();
    adopt_remembered_blind_sheet(session, plan);
    // #44 (box3150j, 2026-10-10 08:29:32-08:31:08 and 08:32:30): each phase that started past
    // the cap still asked the sheet root once or twice more (~10.5 s each), or switched to
    // every-window reads first. Past the cap this phase goes straight to the frames.
    let root_unreadable = if visual_handoff_due(session, plan) {
        let error = anyhow::Error::new(SoundRootUnreadable {
            reads: consecutive_unreadable_roots(),
        });
        diagnostics.emit(
            "currentXml",
            "skip",
            started,
            "rootUnreadable",
            Some(&error),
        );
        note_visual_handoff("mở bảng nhạc", &error);
        true
    } else {
        match observe_current_recommended_routed(session, plan, maximum, allow_recommended, true)
            .await
        {
            Ok(Some(pool)) => {
                return checked_measured_sound_observation(session, plan, &epoch, Ok(pool)).await;
            }
            // Roots that went blind after a tab tap, in a phase that could not recycle: this
            // armed phase recycles before handing the attempt to OCR.
            Ok(None)
                if visual_handoff_due(session, plan)
                    && tab_switch_recycle_due(plan)
                    && unreadable_since_tab_tap() > 0 =>
            {
                let error = anyhow::Error::new(SoundRootUnreadable {
                    reads: consecutive_unreadable_roots(),
                });
                diagnostics.emit("directXml", "skip", started, "tabSwitchBlind", Some(&error));
                return Err(error.context(SoundTabSwitchBlind));
            }
            // The current list is not observed here; direct XML would ask the same root twice.
            Ok(None) if visual_handoff_due(session, plan) => {
                let error = anyhow::Error::new(SoundRootUnreadable {
                    reads: consecutive_unreadable_roots(),
                });
                diagnostics.emit("directXml", "skip", started, "rootUnreadable", Some(&error));
                note_visual_handoff("mở bảng nhạc", &error);
                true
            }
            Ok(None) => false,
            Err(error) if error.is::<SoundRootUnreadable>() => {
                diagnostics.emit(
                    "currentXml",
                    "finish",
                    started,
                    "rootUnreadable",
                    Some(&error),
                );
                note_visual_handoff("mở bảng nhạc, đọc danh sách nhạc", &error);
                true
            }
            Err(error) => return Err(error),
        }
    };
    // The measured 45.7.3 sheet can expose a complete Hot hierarchy even
    // when local OCR is unavailable. Require two fresh, stable XML pools from
    // this same app/session before returning; an incomplete root falls back
    // to the existing visual route and never authorizes a row tap.
    // After two unreadable roots in a row, a third wait here would only delay OCR.
    if root_unreadable {
        remember_blind_sheet(session);
        checked_measured_sound_observation(session, plan, &epoch, Ok(())).await?;
    } else {
        let started = Instant::now();
        diagnostics.emit("directXml", "start", started, "pending", None);
        let direct = snapshot::observe_direct(session, plan, maximum).await;
        diagnostics.emit("directXml", "finish", started,
            match &direct { Ok(Some(_)) => "ready", Ok(None) => "unavailable", Err(_) => "error" },
            direct.as_ref().err());
        match direct {
            Ok(Some(pool)) => {
                return checked_measured_sound_observation(session, plan, &epoch, Ok(pool)).await;
            }
            Err(error) if error.is::<SoundStopped>() || error.is::<SoundTabSwitchBlind>() => {
                return Err(error)
            }
            _ => checked_measured_sound_observation(session, plan, &epoch, Ok(())).await?,
        }
    }
    let started = Instant::now();
    diagnostics.emit("visual", "start", started, "pending", None);
    let visual = visual::observe_initial(session, plan, maximum).await;
    diagnostics.emit("visual", "finish", started,
        if visual.is_ok() { "ready" } else { "error" }, visual.as_ref().err());
    match checked_measured_sound_observation(session, plan, &epoch, visual).await {
        Ok(pool) => return Ok(pool),
        Err(error) if error.is::<visual::VisualSoundPoolUnavailable>() => {}
        Err(error)
            if error.is::<crate::driver::ScreenshotReadUnavailable>()
                || error.is::<OcrReadUnavailable>() =>
        {
            let started = Instant::now();
            diagnostics.emit("xmlOnly", "start", started, "pending", None);
            let pool = async {
                snapshot::select_section_tab_xml_only(session, plan).await?;
                snapshot::observe_xml_only(session, plan, maximum).await
            }
            .await
            .with_context(|| format!("independent sound XML proof after {error:#}"));
            diagnostics.emit("xmlOnly", "finish", started,
                if pool.is_ok() { "ready" } else { "error" }, pool.as_ref().err());
            crate::publish_recovery::note_read(
                "sound",
                "measuredXmlAfterImageUnavailable",
                1,
                if pool.is_ok() { "ready" } else { "unavailable" },
                Some(&error.to_string()),
            );
            return checked_measured_sound_observation(session, plan, &epoch, pool).await;
        }
        Err(error) => return Err(error),
    }

    // If the direct XML probe failed to stabilize, OCR must prove the same
    // sheet before a later XML read can take over.
    let started = Instant::now();
    diagnostics.emit("lateXml", "start", started, "pending", None);
    let pool = async {
        let sheet = selection_recovery::prove_sheet(session, plan).await;
        checked_measured_sound_observation(session, plan, &epoch, sheet).await?;
        let hierarchy_unavailable = || {
            crate::publish_recovery::retryable_error(
                "sound_hierarchy_unavailable",
                "TikTok chưa cung cấp hàng nhạc ổn định sau khi OCR không đọc được; chưa bấm Đăng",
            )
        };
        let pool = match tokio::time::timeout(
            Duration::from_secs(45),
            snapshot::observe(session, plan, maximum),
        )
        .await
        {
            Err(_) => return Err(hierarchy_unavailable()),
            // OCR already had its turn; an unreadable root ends this route the same way.
            Ok(Err(error)) if error.is::<SoundRootUnreadable>() => {
                return Err(hierarchy_unavailable());
            }
            Ok(pool) => pool,
        };
        let pool = checked_measured_sound_observation(session, plan, &epoch, pool).await?;
        let _ = SOUND_BUDGET.try_with(|budget| {
            *budget.route.late_xml.lock() = Some((epoch.clone(), Instant::now(), pool.clone()));
        });
        Ok(pool)
    }.await;
    diagnostics.emit("lateXml", "finish", started,
        if pool.is_ok() { "ready" } else { "error" }, pool.as_ref().err());
    pool
}

async fn observe_current_recommended(
    session: &dyn UiSession,
    plan: SoundPickerPlan,
    maximum: usize,
    allow_recommended: bool,
) -> anyhow::Result<Option<ObservedSoundPool>> {
    observe_current_recommended_routed(session, plan, maximum, allow_recommended, false).await
}

/// `hand_off` is set only by a caller that continues on the measured visual route.
async fn observe_current_recommended_routed(
    session: &dyn UiSession,
    plan: SoundPickerPlan,
    maximum: usize,
    allow_recommended: bool,
    hand_off: bool,
) -> anyhow::Result<Option<ObservedSoundPool>> {
    if !allow_recommended || !selection_recovery::measured(plan) {
        return Ok(None);
    }
    let current = SoundPickerPlan {
        section_label: "For You",
        canonical_section: "recommended",
        ..plan
    };
    let Some(mut pool) =
        snapshot::observe_current_recommended_until_loaded(session, current, maximum, hand_off)
            .await?
    else {
        return Ok(None);
    };
    pool.effective_plan = Some(current);
    Ok(Some(pool))
}

async fn checked_measured_sound_observation<T>(
    session: &dyn UiSession,
    plan: SoundPickerPlan,
    epoch: &str,
    observation: anyhow::Result<T>,
) -> anyhow::Result<T> {
    if observation
        .as_ref()
        .is_err_and(|error| error.is::<SoundStopped>())
    {
        return observation;
    }
    check_wait()?;
    anyhow::ensure!(
        read_sound(session.active_app_bundle()).await? == plan.package,
        "sound app changed; refusing selection"
    );
    if session.gui_session_epoch() != epoch {
        return Err(crate::publish_recovery::retryable_error(
            "sound_session_replaced",
            "Phiên điều khiển đổi khi đọc bảng nhạc; sẽ kiểm lại TikTok và bảng nhạc trước khi chọn, chưa bấm Đăng",
        ));
    }
    observation
}
pub async fn recover_sound_selection(
    session: &dyn UiSession,
    plan: SoundPickerPlan,
    pool: &ObservedSoundPool,
    index: usize,
) -> anyhow::Result<()> {
    let _pin = hold_sound_session(session);
    let plan = pool.effective_plan(plan);
    if recent::measured(plan) && plan.section_label == "Recent" {
        // The exact row was just proved on Recent; an editor read while that
        // sheet is open would consume the whole budget before selection.
        return match choose_and_confirm_sound(session, plan, pool, index).await {
            Err(error) if error.is::<recent::SessionChanged>() => {
                let changed = error
                    .downcast_ref::<recent::SessionChanged>()
                    .expect("typed epoch change");
                let expected = &pool
                    .candidates
                    .get(index)
                    .context("sound selection index missing")?
                    .title;
                if changed.previous.is_empty()
                    || changed.current.is_empty()
                    || changed.previous == changed.current
                    || session.gui_session_epoch() != changed.current
                {
                    return Err(error);
                }
                tracing::warn!(old_epoch=%changed.previous,new_epoch=%changed.current,"Recent session replaced; observing editor without replaying selection");
                match selection_recovery::confirm_editor_in_epoch(
                    session,
                    plan,
                    expected,
                    &changed.current,
                )
                .await
                {
                    Ok(()) => Ok(()),
                    Err(stopped) if stopped.is::<SoundStopped>() => Err(stopped),
                    Err(_) => Err(error),
                }
            }
            result => result,
        };
    }
    if pool.visual {
        set_selection_route(ROUTE_VISUAL);
        return visual::recover(session, plan, pool, index).await;
    }
    // A stage past its blind-root cap asks the editor and the sheet for no further XML: the
    // frozen identity goes to the visual route, which never taps a row shown selected.
    if visual_handoff_due(session, plan) {
        let blind = anyhow::Error::new(SoundRootUnreadable {
            reads: consecutive_unreadable_roots(),
        });
        note_visual_handoff("chọn lại nhạc sau khi đọc lỗi", &blind);
        set_selection_route(ROUTE_VISUAL);
        return hand_off_frozen_pool(session, plan, pool, index).await;
    }
    // The measured hierarchy path checks the current selected index before any tap.
    if confirm_sound(session, plan, &pool.candidates[index].title)
        .await
        .is_ok()
    {
        return Ok(());
    }
    choose_and_confirm_sound(session, plan, pool, index).await
}

/// The XML pool is frozen and the sheet root stopped answering before the row tap. Hot has
/// the measured visual twin (`visual::hand_off`). Any other tab has none: OCR rows are proved
/// under the Hot underline only, so the attempt ends before any tap and the bound choice waits
/// for a fresh attempt, exactly like a choice that left the pool.
async fn hand_off_frozen_pool(
    session: &dyn UiSession,
    plan: SoundPickerPlan,
    pool: &ObservedSoundPool,
    index: usize,
) -> anyhow::Result<()> {
    // Every caller met the blind-root cap. A fresh attempt on this read server starts by sight.
    remember_blind_sheet(session);
    if plan.section_label == "Hot" {
        return visual::hand_off(session, plan, pool, index).await;
    }
    Err(crate::publish_recovery::retryable_error(
        "sound_any_choice_requires_fresh_attempt",
        format!(
            "Không đọc được cây giao diện bảng nhạc trước khi bấm chọn ({} lần liên tiếp); tab {} chưa đo đường đọc ảnh màn hình nên không chuyển sang OCR được; cần lượt thử mới, chưa bấm Đăng",
            consecutive_unreadable_roots(),
            plan.section_label
        ),
    ))
}

fn unique_sound_entry(
    tree: &crate::ui_automation::tree::Tree,
    package: &str,
    ids: &[&str],
) -> Option<ElementBox> {
    let mut boxes: Vec<ElementBox> = Vec::new();
    for id in ids {
        for index in tree.matching(package, ElementQuery::ResourceIdSuffix(id)) {
            if !tree.nodes[index].visible(package) || !tree.ancestors_visible(index) {
                continue;
            }
            let Some(rect) = tree.nodes[index]
                .rect()
                .filter(|r| r.enabled && r.clickable && r.width > 0.0 && r.height > 0.0)
            else {
                continue;
            };
            if !boxes.iter().any(|b| {
                b.x == rect.x && b.y == rect.y && b.width == rect.width && b.height == rect.height
            }) {
                boxes.push(rect);
            }
        }
    }
    if boxes.len() == 1 {
        boxes.pop()
    } else {
        None
    }
}

async fn open_dynamic_sounds(
    session: &dyn UiSession,
    plan: SoundPickerPlan,
    maximum: usize,
    before_open: &mut (dyn FnMut() + Send),
) -> anyhow::Result<ObservedSoundPool> {
    let ids: Vec<_> = MEASURED_SOUND_PICKERS
        .iter()
        .filter(|p| p.plan.package == plan.package)
        .map(|p| p.plan.entry_id)
        .collect();
    let deadline = phase_deadline(SOUND_WINDOW);
    let button = loop {
        clear_sound_popup(session, deadline, Some(plan.package)).await?;
        let source = crate::ui_automation::tree::Tree::parse(
            read_sound(session.hierarchy_source_snapshot()).await?,
        )?;
        if let Some(button) = unique_sound_entry(&source, plan.package, &ids) {
            break button;
        }
        anyhow::ensure!(
            Instant::now() < deadline,
            "sound_entry_ambiguous: giao diện chưa có duy nhất nút nhạc"
        );
        tokio::time::sleep(POLL).await;
    };
    sound_tap_armed(session, button.centre(), before_open).await?;
    let deadline = phase_deadline(SOUND_WINDOW);
    let selected = loop {
        check_wait()?;
        let tree = crate::ui_automation::tree::Tree::parse(
            read_sound(session.hierarchy_source_snapshot()).await?,
        )?;
        let mut matches = Vec::new();
        for candidate in MEASURED_SOUND_PICKERS
            .iter()
            .filter(|p| p.plan.package == plan.package)
        {
            let p = candidate.plan;
            let exists = |id| {
                !tree
                    .matching(plan.package, ElementQuery::ResourceIdSuffix(id))
                    .is_empty()
            };
            if exists(p.row_id)
                && exists(p.title_id)
                && exists(p.artist_id)
                && p.snapshot_layout()
                    .is_none_or(|l| exists(l.tab_id) && exists(l.viewport_id))
            {
                matches.push(p);
            }
        }
        if let Some(first) = matches.first().copied() {
            anyhow::ensure!(
                matches.iter().all(|p| p.row_id == first.row_id
                    && p.title_id == first.title_id
                    && p.artist_id == first.artist_id
                    && p.layout == first.layout
                    && p.selection == first.selection
                    && p.choose_id == first.choose_id),
                "sound_layout_ambiguous"
            );
            break first;
        }
        anyhow::ensure!(Instant::now() < deadline, "sound_layout_unrecognized");
        tokio::time::sleep(POLL).await;
    };
    if selected.snapshot_layout().is_some() {
        snapshot::select_section_tab(session, selected).await?;
    }
    let mut pool = observe_sound_pool(session, selected, maximum).await?;
    pool.effective_plan = Some(selected);
    Ok(pool)
}

async fn observe_sound_pool(
    session: &dyn UiSession,
    plan: SoundPickerPlan,
    maximum_visible: usize,
) -> anyhow::Result<ObservedSoundPool> {
    Box::pin(observe_sound_pool_until(session, plan, maximum_visible, phase_deadline(SOUND_WINDOW))).await
}

async fn observe_sound_pool_until(
    session: &dyn UiSession,
    plan: SoundPickerPlan,
    maximum_visible: usize,
    deadline: Instant,
) -> anyhow::Result<ObservedSoundPool> {
    let deadline = deadline.min(phase_deadline(SOUND_WINDOW));
    check_wait()?;
    // Scope read deadlines without cancelling a popup dismissal already dispatched.
    let result = Box::pin(with_deadline_budget(&AtomicBool::new(false), deadline,
        Box::pin(observe_sound_pool_inner(session, plan, maximum_visible, deadline)))).await;
    check_wait()?;
    if Instant::now() >= deadline { return Err(crate::publish_recovery::observation_deadline()); }
    result
}

async fn observe_sound_pool_inner(
    session: &dyn UiSession,
    plan: SoundPickerPlan,
    maximum_visible: usize,
    deadline: Instant,
) -> anyhow::Result<ObservedSoundPool> {
    clear_sound_popup(session, deadline, Some(plan.package)).await?;
    if recent::measured(plan) && plan.section_label == "Recent" {
        return tokio::time::timeout(Duration::from_secs(30), recent::observe(session, plan))
            .await
            .context("Recent sound rows timed out")?
            .map_err(classify_recent_observation_error);
    }
    if plan.snapshot_layout().is_some() {
        return Box::pin(snapshot::observe_until(session, plan, maximum_visible, deadline)).await;
    }
    let mut previous: Option<ObservedSoundPool> = None;
    loop {
        clear_sound_popup(session, deadline, Some(plan.package)).await?;
        check_wait()?;
        let section = read_sound(session.locate_all_described(ElementQuery::Text {
            value: plan.section_label,
            exact: true,
        }))
        .await?;
        let rows =
            read_sound(session.locate_all(ElementQuery::ResourceIdSuffix(plan.row_id))).await?;
        let titles =
            read_sound(session.locate_all_described(ElementQuery::ResourceIdSuffix(plan.title_id)))
                .await?;
        let artists = read_sound(
            session.locate_all_described(ElementQuery::ResourceIdSuffix(plan.artist_id)),
        )
        .await?;
        let choices = match plan.choose_id {
            Some(id) => read_sound(session.locate_all(ElementQuery::ResourceIdSuffix(id))).await?,
            None => Vec::new(),
        };
        let mut markers = Vec::new();
        for id in plan.selected_marker_ids() {
            // An equalizer or trim control can prove the selected row without
            // becoming a tap target. The measured plan names both independently.
            markers
                .extend(read_sound(session.locate_all(ElementQuery::ResourceIdSuffix(id))).await?);
        }
        if section.len() == 1 && !rows.is_empty() && !titles.is_empty() {
            match assemble_pool(
                plan,
                rows,
                titles,
                artists,
                choices,
                markers,
                maximum_visible,
            ) {
                Ok(pool) => {
                    if previous.as_ref().is_some_and(|p| p.stable_with(&pool)) {
                        return Ok(pool);
                    }
                    previous = Some(pool);
                }
                Err(error) if Instant::now() >= deadline => return Err(error.context(
                    "sound_candidates_incomplete: bảng nhạc chưa tải đủ row/title/artist/choose",
                )),
                Err(_) => {
                    previous = None;
                    tokio::time::sleep(POLL).await;
                    continue;
                }
            }
        } else {
            previous = None;
        }
        if Instant::now() >= deadline {
            anyhow::bail!("sound picker did not expose one measured section with candidate rows");
        }
        tokio::time::sleep(POLL).await;
    }
}

/// Tap the selected row once and prove the editor now names the same sound.
pub(crate) fn provisional_visual_pool(plan: SoundPickerPlan, pool: &ObservedSoundPool) -> bool {
    pool.visual && selection_recovery::measured(pool.effective_plan(plan))
}

pub(crate) async fn choose_provisional_sound(
    session: &dyn UiSession,
    plan: SoundPickerPlan,
    pool: &ObservedSoundPool,
    index: usize,
    recovering: bool,
    row_proved: &mut bool,
) -> anyhow::Result<String> {
    let _pin = hold_sound_session(session);
    set_selection_route(ROUTE_VISUAL);
    visual::choose_provisional(
        session,
        pool.effective_plan(plan),
        pool,
        index,
        recovering,
        row_proved,
    )
    .await
}

pub async fn choose_and_confirm_sound(
    session: &dyn UiSession,
    plan: SoundPickerPlan,
    pool: &ObservedSoundPool,
    index: usize,
) -> anyhow::Result<()> {
    let _pin = hold_sound_session(session);
    let plan = pool.effective_plan.unwrap_or(plan);
    let serial = session.gui_scope().map(|scope| scope.device_id);
    let epoch = session.gui_session_epoch();
    let row_summary = |target: &ElementBox| serde_json::json!({
        "x": target.x, "y": target.y, "w": target.width, "h": target.height,
        "enabled": target.enabled, "clickable": target.clickable,
    });
    let trace_row = |phase: &'static str, observed: Option<&ObservedSoundPool>,
                     deadline: Option<Instant>, observation_ok: Option<bool>, observation: u8| {
        let row_target = observed.and_then(|p| p.target(index)).map(&row_summary);
        let boxes = observed.map(|p| p.targets.iter().take(5).map(&row_summary).collect::<Vec<_>>())
            .unwrap_or_default();
        let candidate_identity_equal = pool.candidates.get(index).is_some()
            && observed.is_some_and(|p| p.candidates.get(index) == pool.candidates.get(index));
        let remaining_phase_ms = deadline.map(|end| end.saturating_duration_since(Instant::now())
            .as_millis().min(u64::MAX as u128) as u64);
        let remaining_global_ms = SOUND_BUDGET.try_with(|budget| budget.deadline
            .saturating_duration_since(Instant::now()).as_millis().min(u64::MAX as u128) as u64).ok();
        tracing::info!(serial=?serial, session=%epoch, package=plan.package,
            phase, selectedIndex=?observed.and_then(|p| p.selected_index), targetIndex=index,
            candidateIdentityEqual=candidate_identity_equal,
            rowTarget=%row_target.unwrap_or(serde_json::Value::Null),
            poolTargets=%serde_json::Value::Array(boxes), poolCount=?observed.map(|p|p.candidates.len()),
            observation, observationOk=?observation_ok,
            remainingPhaseMs=?remaining_phase_ms, remainingGlobalMs=?remaining_global_ms,
            "sound row selection observation");
    };
    let mut phase = "preRow";
    let mut readback_deadline = None;
    let outcome = async {
        if pool.visual {
            phase = "visualRow";
            set_selection_route(ROUTE_VISUAL);
            return visual::choose(session, plan, pool, index).await;
        }
        set_selection_route(ROUTE_XML);
        let candidate = pool
            .candidates
            .get(index)
            .context("sound selection index is outside the observed pool")?;
        if visual_handoff_due(session, plan) {
            // The stage already met its blind-root cap. Asking the same root for popups and
            // rows again (~10.5 s each, 30 s on a request deadline) would only spend the
            // budget: the frozen identity is handed off before any read.
            phase = "visualHandoff";
            let blind = anyhow::Error::new(SoundRootUnreadable {
                reads: consecutive_unreadable_roots(),
            });
            note_visual_handoff("trước khi bấm chọn nhạc", &blind);
            set_selection_route(ROUTE_VISUAL);
            return hand_off_frozen_pool(session, plan, pool, index).await;
        }
        let reused = take_fresh_late_xml(session, pool);
        if reused.is_some() {
            crate::publish_recovery::note_read(
                "sound",
                "lateXmlPoolReused",
                1,
                "reused",
                Some("Dùng lại danh sách nhạc vừa đọc hai lần từ cây giao diện (cùng phiên, chưa quá một nhịp đọc); không đọc lại trước khi bấm chọn"),
            );
        }
        let observed = match reused {
            Some(fresh) => Ok(fresh),
            None => observe_sound_pool(session, plan, pool.maximum_visible).await,
        };
        let fresh = match observed {
            Ok(fresh) => fresh,
            Err(error)
                if error.is::<SoundRootUnreadable>()
                    || (unreadable_root(&error) && visual_handoff_due(session, plan)) =>
            {
                // No row was tapped in this call. The frozen XML identity moves to the
                // visual route, which taps only a row it sees unselected. A popup read that
                // met the cap on this same root is not asked again either.
                phase = "visualHandoff";
                note_visual_handoff("trước khi bấm chọn nhạc", &error);
                set_selection_route(ROUTE_VISUAL);
                return hand_off_frozen_pool(session, plan, pool, index).await;
            }
            Err(error) => return Err(error),
        };
        trace_row("preRow", Some(&fresh), None, Some(true), 0);
        let target = reproof_target(pool, &fresh, index)?;
        if fresh.selected_index != Some(index) {
            phase = "rowDispatch";
            trace_row(phase, Some(&fresh), None, None, 0);
            let row_result = sound_tap(session, target.centre()).await;
            phase = "rowAck";
            trace_row(phase, Some(&fresh), None, Some(row_result.is_ok()), 0);
            row_result.context("select observed sound")?;
        }
        if plan.closes_with_back() {
            // The measured Android sheet selects inline; Back closes only that sheet.
            // Prove the same pool remains before dismissing it, then prove the editor chip.
            let deadline = phase_deadline(READBACK_WINDOW);
            readback_deadline = Some(deadline);
            let mut recovered = false;
            let mut network_observation_used = false;
            let mut post_row_observations = 0u8;
            while fresh.selected_index != Some(index) {
                phase = "postRow";
                let observation = if selection_recovery::measured(plan) {
                    snapshot::observe_after_selection(session, plan, pool.maximum_visible, deadline).await
                } else {
                    observe_sound_pool_until(session, plan, pool.maximum_visible, deadline).await
                };
                if post_row_observations < 2 {
                    post_row_observations += 1;
                    trace_row(phase, observation.as_ref().ok(), Some(deadline),
                        Some(observation.is_ok()), post_row_observations);
                }
                let selected_pool = match observation {
                    Ok(pool) => pool,
                    Err(error)
                        if (transient_sound_read(&error) || error.is::<SoundRootUnreadable>())
                            && selection_recovery::measured(plan) =>
                    {
                        if error.is::<SoundRootUnreadable>() {
                            crate::publish_recovery::note_read(
                                "sound",
                                "postRowReadback",
                                unreadable_roots(),
                                "transientUnreadable",
                                Some(&format!(
                                    "Chưa đọc được bảng nhạc sau khi bấm chọn; kiểm tra bằng màn chỉnh sửa, không bấm chọn lại: {error:#}"
                                )),
                            );
                        }
                        // Some measured builds close the inline sheet themselves after
                        // selection. An exact, stable editor chip proves completion;
                        // otherwise the still-open sheet must be proven before Back.
                        phase = "editorFallback";
                        let editor = tokio::time::timeout_at(
                            phase_deadline(READBACK_WINDOW),
                            selection_recovery::confirm_editor(session, plan, &candidate.title),
                        )
                        .await;
                        check_wait()?;
                        if matches!(editor, Ok(Ok(()))) {
                            return Ok(());
                        }
                        phase = "sheetFallback";
                        selection_recovery::prove_sheet(session, plan)
                            .await
                            .with_context(|| format!("sound selection read unavailable: {error}"))?;
                        recovered = true;
                        break;
                    }
                    Err(error) => return Err(error),
                };
                if Instant::now() >= deadline { return Err(crate::publish_recovery::observation_deadline()); }
                reproof_target(pool, &selected_pool, index)?;
                if selected_pool.selected_index == Some(index) {
                    break;
                }
                // SM-G955F/Trill 38.3.2/en, 03/10/2026: a row contact ACK
                // leaves the sheet unselected while Android displays its exact
                // "No internet connection" toast. Observe this device condition
                // once inside the existing readback budget; never retap the row.
                if !network_observation_used
                    && MEASURED_SOUND_PICKERS.iter().any(|measured|
                        measured.version == "38.3.2" && measured.language == "en"
                            && measured.plan == plan)
                    && Instant::now() < deadline
                {
                    network_observation_used = true;
                    phase = "postRowNetworkObservation";
                    let network_unavailable = tokio::time::timeout_at(deadline, async {
                        check_wait()?;
                        anyhow::ensure!(session.gui_session_epoch() == epoch,
                            "sound session changed before network observation");
                        anyhow::ensure!(read_sound(session.active_app_bundle()).await? == plan.package,
                            "sound app changed before network observation");
                        let source = read_sound(session.hierarchy_source_snapshot()).await?;
                        anyhow::ensure!(session.gui_session_epoch() == epoch,
                            "sound session changed during network observation");
                        let tree = crate::ui_automation::tree::Tree::parse(source)?;
                        anyhow::ensure!(read_sound(session.active_app_bundle()).await? == plan.package
                            && session.gui_session_epoch() == epoch,
                            "sound app/session changed during network observation");
                        check_wait()?;
                        Ok::<bool, anyhow::Error>(tree.nodes.iter().any(|node|
                            node.attr("package") == "com.android.settings"
                                && node.attr("class") == "android.widget.Toast"
                                && node.attr("text") == "No internet connection"
                                && node.attr("displayed") == "true"))
                    }).await.map_err(|_| crate::publish_recovery::observation_deadline())??;
                    if network_unavailable {
                        return Err(anyhow::Error::new(crate::publish_recovery::RecoveryFailure::new(
                            "sound_network_unavailable",
                            crate::publish_recovery::FailureKind::Terminal,
                            "Máy báo không có kết nối Internet khi chọn nhạc; chưa bấm Đăng. Kiểm tra kết nối mạng rồi Thử lại.",
                        )));
                    }
                    phase = "postRow";
                }
                check_wait()?;
                if Instant::now() >= deadline {
                    return Err(anyhow::anyhow!("selected sound row not confirmed before closing picker")
                        .context(crate::publish_recovery::RecoveryFailure::new(
                            "sound_selection_readback_unavailable",
                            crate::publish_recovery::FailureKind::Retryable,
                            format!("Selected sound '{}' by '{}' has no confirmed readback; fresh preparation required before Post", candidate.title, candidate.artist),
                        )));
                }
                tokio::time::sleep(POLL).await;
            }
            check_wait()?;
            anyhow::ensure!(session.gui_session_epoch() == epoch,
                "sound session changed before closing picker");
            anyhow::ensure!(read_sound(session.active_app_bundle()).await? == plan.package,
                "sound app changed before closing picker");
            anyhow::ensure!(session.gui_session_epoch() == epoch,
                "sound session changed during close proof");
            check_wait()?;
            phase = "closePicker";
            session.back().await.context("close inline sound picker")?;
            if recovered {
                phase = "editorReadback";
                return selection_recovery::confirm_editor_or_chip(session, plan, candidate).await;
            }
        }
        phase = "editorReadback";
        confirm_sound(session, plan, &candidate.title)
            .await
            .context("initial sound selection readback")
    }.await;
    if let Err(error) = &outcome {
        let failure = crate::publish_recovery::describe(error);
        let remaining_phase_ms = readback_deadline.map(|end: Instant| end
            .saturating_duration_since(Instant::now()).as_millis().min(u64::MAX as u128) as u64);
        let remaining_global_ms = SOUND_BUDGET.try_with(|budget| budget.deadline
            .saturating_duration_since(Instant::now()).as_millis().min(u64::MAX as u128) as u64).ok();
        tracing::warn!(serial=?serial, session=%epoch, currentSession=%session.gui_session_epoch(),
            package=plan.package, phase, targetIndex=index, visual=pool.visual,
            failureCode=%failure.code, failureKind=failure.kind.as_str(),
            remainingPhaseMs=?remaining_phase_ms, remainingGlobalMs=?remaining_global_ms,
            "sound row selection returned an error");
    }
    outcome
}

fn reproof_target<'a>(
    expected: &ObservedSoundPool,
    fresh: &'a ObservedSoundPool,
    index: usize,
) -> anyhow::Result<&'a ElementBox> {
    anyhow::ensure!(
        expected.candidates.get(index).is_some()
            && expected.candidates.get(index) == fresh.candidates.get(index),
        "selected sound identity changed before selection"
    );
    let target = fresh
        .target(index)
        .context("sound selection target is missing")?;
    anyhow::ensure!(target.enabled, "sound selection control disabled");
    Ok(target)
}

/// Re-read the editor chip. The exact title and exactly one node are both required.
/// One additional observation-only window after an unreadable navigation transition.
/// Bound the request itself as well as polling, and honor Stop during slow reads.
pub(crate) async fn confirm_sound_after_transition(
    session: &dyn UiSession,
    plan: SoundPickerPlan,
    expected_title: &str,
    stop: &AtomicBool,
) -> anyhow::Result<()> {
    with_readback_budget(stop, confirm_sound(session, plan, expected_title)).await
}

/// Re-read the editor chip. The exact title and exactly one node are both required.
pub async fn confirm_sound(
    session: &dyn UiSession,
    plan: SoundPickerPlan,
    expected_title: &str,
) -> anyhow::Result<()> {
    observation::confirm(session, plan, expected_title).await
}

fn assemble_pool(
    plan: SoundPickerPlan,
    mut rows: Vec<ElementBox>,
    titles: Vec<ElementBox>,
    artists: Vec<ElementBox>,
    choices: Vec<ElementBox>,
    markers: Vec<ElementBox>,
    maximum_visible: usize,
) -> anyhow::Result<ObservedSoundPool> {
    rows.sort_by(|left, right| left.y.total_cmp(&right.y));
    let mut candidates = Vec::new();
    let mut targets = Vec::new();
    let mut selected_index = None;
    for (index, row) in rows.into_iter().take(maximum_visible).enumerate() {
        if plan.closes_with_back() && !inside(&row, &markers).is_empty() {
            anyhow::ensure!(selected_index.is_none(), "ambiguous selected sound row");
            selected_index = Some(index);
        }
        let title = exactly_one(inside(&row, &titles), "sound title inside candidate row")?;
        let title_text = title
            .description
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .context("sound title is empty")?;
        let artist = exactly_one(inside(&row, &artists), "sound artist inside candidate row")?
            .description
            .as_deref()
            .map(normalize_artist)
            .unwrap_or_default();
        if selection_recovery::measured(plan)
            && plan.section_label == "For You"
            && plan.canonical_section == "recommended"
        {
            anyhow::ensure!(!artist.is_empty(), "sound artist is empty");
        }
        let target = if plan.choose_id.is_some() {
            exactly_one(
                inside(&row, &choices),
                "sound choose control inside candidate row",
            )?
        } else {
            title.clone()
        };
        candidates.push(SoundCandidate {
            section: plan.canonical_section.to_string(),
            title: title_text.to_string(),
            artist,
        });
        targets.push(target);
    }
    anyhow::ensure!(
        !candidates.is_empty(),
        "sound picker exposed no complete candidate row"
    );
    // Skip every ambiguous title, retaining the original observation window for
    // reproof. One duplicate recommendation must not discard distinct usable songs.
    let mut counts = HashMap::new();
    for candidate in &candidates {
        *counts.entry(candidate.title.clone()).or_insert(0) += 1;
    }
    let mut unique_candidates = Vec::new();
    let mut unique_targets = Vec::new();
    let mut unique_selected = None;
    for (index, (candidate, target)) in candidates.into_iter().zip(targets).enumerate() {
        if counts[&candidate.title] != 1 || !complete_sound_title(&candidate.title) {
            continue;
        }
        if selected_index == Some(index) {
            unique_selected = Some(unique_candidates.len());
        }
        unique_candidates.push(candidate);
        unique_targets.push(target);
    }
    anyhow::ensure!(!unique_candidates.is_empty(), "sound picker contains only duplicate titles; the editor chip cannot prove which artist was selected");
    let candidates = unique_candidates;
    let targets = unique_targets;
    let selected_index = unique_selected;
    Ok(ObservedSoundPool {
        visual: false,
        effective_plan: None,
        candidates,
        maximum_visible,
        targets,
        selected_index,
    })
}

fn complete_sound_title(title: &str) -> bool {
    if title.contains('\u{2026}') || title.ends_with("...") {
        return false;
    }
    let mut brackets = Vec::new();
    for character in title.chars() {
        match character {
            '(' | '[' => brackets.push(character),
            ')' if brackets.pop() != Some('(') => return false,
            ']' if brackets.pop() != Some('[') => return false,
            _ => {}
        }
    }
    brackets.is_empty()
}

fn inside(row: &ElementBox, values: &[ElementBox]) -> Vec<ElementBox> {
    values
        .iter()
        .filter(|value| {
            let centre = value.centre();
            centre.x >= row.x
                && centre.x <= row.x + row.width
                && centre.y >= row.y
                && centre.y <= row.y + row.height
        })
        .cloned()
        .collect()
}

fn exactly_one(mut values: Vec<ElementBox>, what: &str) -> anyhow::Result<ElementBox> {
    if values.len() != 1 {
        anyhow::bail!("expected exactly one {what}, found {}", values.len());
    }
    Ok(values.remove(0))
}

fn normalize_artist(value: &str) -> String {
    value
        .trim()
        .split_once(" · ")
        .map_or_else(|| value.trim(), |(artist, _)| artist.trim())
        .trim_matches('\u{200e}')
        .trim()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    #[test]
    fn sound_entry_excludes_hidden_nodes_and_collapses_only_identical_bounds() {
        let package = "com.zhiliaoapp.musically";
        let node = |bounds: &str, displayed: bool| {
            format!(
                r#"<node package="{package}" resource-id="{package}:id/dou" bounds="{bounds}" enabled="true" clickable="true" displayed="{displayed}"/>"#
            )
        };
        let tree = |body: String| {
            crate::ui_automation::tree::Tree::parse(crate::HierarchySourceSnapshot {
                generation: 1,
                xml: format!("<hierarchy>{body}</hierarchy>"),
            })
            .unwrap()
        };
        let one = node("[0,0][40,40]", true);
        assert!(unique_sound_entry(
            &tree(one.clone() + &node("[50,0][90,40]", false)),
            package,
            &[":id/dou"]
        )
        .is_some());
        assert!(unique_sound_entry(&tree(one.clone() + &one), package, &[":id/dou"]).is_some());
        assert!(unique_sound_entry(
            &tree(one + &node("[50,0][90,40]", true)),
            package,
            &[":id/dou"]
        )
        .is_none());
    }

    struct OpeningSession {
        entry_ready: bool,
        taps: AtomicUsize,
        slow_trill: Option<&'static str>,
        snapshots: AtomicUsize,
    }

    #[async_trait::async_trait]
    impl UiSession for OpeningSession {
        async fn tap(&self, _: crate::TapPoint) -> anyhow::Result<()> {
            self.taps.fetch_add(1, Ordering::Relaxed);
            if self.slow_trill == Some("pool") {
                return Ok(());
            }
            anyhow::bail!("fixture loses the open acknowledgement")
        }
        async fn swipe(&self, _: crate::SwipeGesture) -> anyhow::Result<()> {
            unreachable!()
        }
        async fn type_text(&self, _: &str) -> anyhow::Result<()> {
            unreachable!()
        }
        async fn home(&self) -> anyhow::Result<()> {
            unreachable!()
        }
        async fn back(&self) -> anyhow::Result<()> {
            unreachable!()
        }
        async fn find_and_tap(&self, _: &str) -> anyhow::Result<()> {
            unreachable!()
        }
        async fn assert_visible(&self, _: &str) -> anyhow::Result<()> {
            unreachable!()
        }
        fn stream_url(&self) -> Option<String> {
            None
        }
        fn gui_session_epoch(&self) -> String {
            if self.slow_trill == Some("changed-session")
                && self.snapshots.load(Ordering::Relaxed) >= 4
            {
                return "replacement-session".into();
            }
            "opening-session".into()
        }
        async fn active_app_bundle(&self) -> anyhow::Result<String> {
            Ok(if self.slow_trill == Some("changed-app") {
                "other.app"
            } else {
                "com.ss.android.ugc.trill"
            }
            .into())
        }
        async fn hierarchy_source_snapshot(
            &self,
        ) -> anyhow::Result<crate::HierarchySourceSnapshot> {
            let n = self.snapshots.fetch_add(1, Ordering::Relaxed) + 1;
            if self.slow_trill == Some("stop") {
                tokio::time::sleep(Duration::from_secs(10)).await;
            }
            let xml = if n == 1 {
                "<hierarchy/>".into()
            } else {
                let bounds = if self.slow_trill == Some("changed-entry") && n >= 4 && n.is_multiple_of(2) {
                    "[700,100][900,216]"
                } else {
                    "[350,100][720,216]"
                };
                format!(
                    r#"<hierarchy><node package="com.ss.android.ugc.trill" resource-id="com.ss.android.ugc.trill:id/c_4" bounds="{bounds}" enabled="true" clickable="true" displayed="true"/></hierarchy>"#
                )
            };
            let xml = if self.slow_trill == Some("pool") && self.taps.load(Ordering::Relaxed) == 1 {
                inline_fixture_xml("Tung Zin Zin", None)
            } else { xml };
            let xml = if self.slow_trill == Some("ambiguous") {
                xml.replace("</hierarchy>", r#"<node package="com.ss.android.ugc.trill" resource-id="com.ss.android.ugc.trill:id/c_4" bounds="[700,100][900,216]" enabled="true" clickable="true" displayed="true"/></hierarchy>"#)
            } else {
                xml
            };
            Ok(crate::HierarchySourceSnapshot {
                generation: if self.slow_trill == Some("stale") {
                    1
                } else {
                    n as u64
                },
                xml,
            })
        }
        async fn locate_all(&self, query: ElementQuery<'_>) -> anyhow::Result<Vec<ElementBox>> {
            if self.slow_trill == Some("pool") && self.taps.load(Ordering::Relaxed) == 1 {
                return Ok(if query == ElementQuery::ResourceIdSuffix(":id/ta8") {
                    vec![ElementBox {
                        height: 200.0,
                        ..element(100.0, None)
                    }]
                } else {
                    vec![]
                });
            }
            if self.slow_trill.is_some() {
                std::future::pending::<()>().await;
            }
            if !self.entry_ready {
                anyhow::bail!("fixture read failed before the open tap")
            }
            Ok(match query {
                ElementQuery::ResourceIdSuffix(":id/dou") => vec![ElementBox {
                    x: 10.0,
                    y: 10.0,
                    width: 100.0,
                    height: 50.0,
                    description: None,
                    enabled: true,
                    clickable: true,
                }],
                _ => vec![],
            })
        }
        async fn locate_all_described(
            &self,
            query: ElementQuery<'_>,
        ) -> anyhow::Result<Vec<ElementBox>> {
            assert_eq!(self.taps.load(Ordering::Relaxed), 1);
            Ok(match query {
                ElementQuery::Text {
                    value: "Recommended",
                    ..
                } => vec![element(0.0, Some("Recommended"))],
                ElementQuery::ResourceIdSuffix(":id/title") => {
                    vec![element(120.0, Some("Tung Zin Zin"))]
                }
                ElementQuery::ResourceIdSuffix(":id/rr5") => vec![element(180.0, Some("Artist"))],
                _ => vec![],
            })
        }
    }

    #[tokio::test]
    async fn open_intent_is_armed_at_the_tap_boundary_not_during_entry_reads() {
        let plan = SoundPickerPlan::resolve("com.zhiliaoapp.musically", "en", "45.7.3")
            .expect("measured plan");
        for entry_ready in [false, true] {
            let session = OpeningSession {
                entry_ready,
                taps: AtomicUsize::new(0),
                slow_trill: None,
                snapshots: AtomicUsize::new(0),
            };
            let armed = AtomicBool::new(false);
            let result = open_and_observe_sounds_armed(&session, plan, 5, &mut || {
                armed.store(true, Ordering::Relaxed)
            })
            .await;
            assert!(result.is_err());
            assert_eq!(armed.load(Ordering::Relaxed), entry_ready);
            assert_eq!(
                session.taps.load(Ordering::Relaxed),
                usize::from(entry_ready)
            );
        }
    }

    #[tokio::test(start_paused = true)]
    async fn nested_readback_keeps_its_deadline_inside_the_sound_budget() {
        let stop = AtomicBool::new(false);
        let start = Instant::now();
        let result = with_sound_budget(&stop, async {
            with_readback_budget(
                &stop,
                read_sound(async {
                    tokio::time::sleep(READBACK_WINDOW + Duration::from_secs(1)).await;
                    Ok(())
                }),
            )
            .await
        })
        .await;
        assert!(
            result.is_err(),
            "outer loading budget extended a fresh readback deadline"
        );
        assert!(start.elapsed() <= READBACK_WINDOW);
        // An already-dispatched action must drain. Expiration refuses late
        // success but must not erase a definitive refusal into retryable timeout.
        for terminal in [false, true] {
            let result = with_readback_budget(&stop, async {
                tokio::time::sleep(READBACK_WINDOW + Duration::from_secs(1)).await;
                if terminal {
                    Err(crate::publish_recovery::RecoveryFailure::new(
                        "fixture_identity_mismatch",
                        crate::publish_recovery::FailureKind::Terminal,
                        "sound identity changed",
                    )
                    .into())
                } else {
                    Ok(())
                }
            })
            .await
            .expect_err("late completion never becomes success");
            let failure = crate::publish_recovery::describe(&result);
            assert_eq!(
                failure.kind,
                if terminal {
                    crate::publish_recovery::FailureKind::Terminal
                } else {
                    crate::publish_recovery::FailureKind::Retryable
                }
            );
            if terminal {
                assert_eq!(failure.code, "fixture_identity_mismatch");
            }
        }
    }

    #[tokio::test(start_paused = true)]
    async fn expired_budget_does_not_arm_or_dispatch_the_open_tap() {
        let session = OpeningSession {
            entry_ready: true,
            taps: AtomicUsize::new(0),
            slow_trill: None,
            snapshots: AtomicUsize::new(0),
        };
        let armed = AtomicBool::new(false);
        let budget = SoundBudget {
            deadline: Instant::now(),
            stopped: Arc::new(AtomicBool::new(false)),
            popup: Default::default(),
            popup_package: Default::default(),
            route: Default::default(),
        };
        let result = SOUND_BUDGET
            .scope(
                budget,
                sound_tap_armed(&session, crate::TapPoint { x: 10.0, y: 10.0 }, &mut || {
                    armed.store(true, Ordering::Relaxed)
                }),
            )
            .await;

        assert!(result.is_err());
        assert!(!armed.load(Ordering::Relaxed));
        assert_eq!(session.taps.load(Ordering::Relaxed), 0);
    }

    #[tokio::test(start_paused = true)]
    async fn slow_trill_entry_lookup_uses_fresh_xml_before_first_tap() {
        for mode in [
            "ready",
            "pool",
            "changed-app",
            "stale",
            "changed-entry",
            "changed-session",
            "ambiguous",
            "stop",
        ] {
            let session = OpeningSession {
                entry_ready: true,
                taps: AtomicUsize::new(0),
                slow_trill: Some(mode),
                snapshots: AtomicUsize::new(0),
            };
            let stop = AtomicBool::new(false);
            let armed = AtomicBool::new(false);
            let mut arm = || armed.store(true, Ordering::Relaxed);
            let start = Instant::now();
            let (result, ()) = tokio::join!(
                with_sound_budget(
                    &stop,
                    open_and_observe_sounds_armed(
                        &session,
                        SoundPickerPlan::resolve("com.ss.android.ugc.trill", "en", "38.3.2")
                            .unwrap(),
                        1,
                        &mut arm,
                    )
                ),
                async {
                    if mode == "stop" {
                        tokio::time::sleep(Duration::from_secs(2)).await;
                        stop.store(true, Ordering::Relaxed);
                    }
                },
            );
            if mode == "pool" {
                let pool = result.expect("Trill Recommended pool after first XML-proven entry tap");
                assert_eq!(pool.candidates.len(), 1);
                assert_eq!(pool.candidates[0].title, "Tung Zin Zin");
                assert_eq!(session.taps.load(Ordering::Relaxed), 1);
                assert!(armed.load(Ordering::Relaxed));
                continue;
            }
            let error = result.unwrap_err();
            if mode == "ready" {
                assert!(
                    format!("{error:#}").contains("fixture loses the open acknowledgement"),
                    "fresh XML must reach exactly one opening tap: {error:#}"
                );
            }
            assert_eq!(
                session.taps.load(Ordering::Relaxed),
                usize::from(mode == "ready")
            );
            assert_eq!(armed.load(Ordering::Relaxed), mode == "ready");
            assert!(
                start.elapsed() < Duration::from_secs(40),
                "entry lookup must leave budget for pool proof"
            );
            if mode == "stop" {
                assert!(error.is::<SoundStopped>());
            }
        }
    }

    fn inline_fixture_xml(title: &str, marker: Option<&str>) -> String {
        let package = "com.ss.android.ugc.trill";
        let mark = marker.map(|id| format!(r#"<node package="{package}" resource-id="{package}{id}" bounds="[10,110][20,120]" displayed="true" enabled="true"/>"#)).unwrap_or_default();
        format!(r#"<hierarchy><node package="{package}" resource-id="{package}:id/q_g" text="Recommended" selected="true" bounds="[0,0][100,40]" displayed="true" enabled="true"/><node package="{package}" resource-id="{package}:id/tka" bounds="[0,80][300,400]" displayed="true" enabled="true"><node package="{package}" resource-id="{package}:id/ta8" bounds="[0,100][300,300]" displayed="true" enabled="true"><node package="{package}" resource-id="{package}:id/title" text="{title}" bounds="[30,120][200,150]" displayed="true" enabled="true"/><node package="{package}" resource-id="{package}:id/rr5" text="Artist" bounds="[30,180][200,210]" displayed="true" enabled="true"/>{mark}</node></node></hierarchy>"#)
    }

    struct InlineSession {
        list_reads: AtomicUsize,
        list_read_limit: usize,
        selected: AtomicBool,
        closed: AtomicBool,
        taps: AtomicUsize,
        select_takes: bool,
        marker_id: &'static str,
        editor_title: &'static str,
    }

    #[async_trait::async_trait]
    impl UiSession for InlineSession {
        fn gui_session_epoch(&self) -> String { "inline-fixture".into() }
        async fn active_app_bundle(&self) -> anyhow::Result<String> { Ok("com.ss.android.ugc.trill".into()) }
        async fn hierarchy_source_snapshot(&self) -> anyhow::Result<crate::HierarchySourceSnapshot> {
            static GENERATION: AtomicUsize = AtomicUsize::new(1);
            let read = self.list_reads.fetch_add(1, Ordering::Relaxed);
            anyhow::ensure!(read < self.list_read_limit, "redundant list snapshot unavailable");
            Ok(crate::HierarchySourceSnapshot {
                generation: GENERATION.fetch_add(1, Ordering::Relaxed) as u64,
                xml: inline_fixture_xml("One", self.selected.load(Ordering::Relaxed).then_some(self.marker_id)),
            })
        }

        async fn tap(&self, _: crate::TapPoint) -> anyhow::Result<()> {
            self.taps.fetch_add(1, Ordering::Relaxed);
            if self.select_takes {
                self.selected.fetch_xor(true, Ordering::Relaxed);
            }
            Ok(())
        }
        async fn swipe(&self, _: crate::SwipeGesture) -> anyhow::Result<()> {
            unreachable!()
        }
        async fn type_text(&self, _: &str) -> anyhow::Result<()> {
            unreachable!()
        }
        async fn home(&self) -> anyhow::Result<()> {
            unreachable!()
        }
        async fn back(&self) -> anyhow::Result<()> {
            self.closed.store(true, Ordering::Relaxed);
            Ok(())
        }
        async fn find_and_tap(&self, _: &str) -> anyhow::Result<()> {
            unreachable!()
        }
        async fn assert_visible(&self, _: &str) -> anyhow::Result<()> {
            unreachable!()
        }
        fn stream_url(&self) -> Option<String> {
            None
        }
        async fn locate_all(&self, query: ElementQuery<'_>) -> anyhow::Result<Vec<ElementBox>> {
            Ok(match query {
                ElementQuery::ResourceIdSuffix(":id/ta8") => vec![ElementBox {
                    height: 200.0,
                    ..element(100.0, None)
                }],
                ElementQuery::ResourceIdSuffix(id)
                    if id == self.marker_id && self.selected.load(Ordering::Relaxed) =>
                {
                    vec![element(125.0, None)]
                }
                _ => vec![],
            })
        }
        async fn locate_all_described(
            &self,
            query: ElementQuery<'_>,
        ) -> anyhow::Result<Vec<ElementBox>> {
            Ok(match query {
                ElementQuery::Text {
                    value: "Recommended",
                    ..
                } => vec![element(0.0, Some("Recommended"))],
                ElementQuery::ResourceIdSuffix(":id/title") => vec![element(120.0, Some("One"))],
                ElementQuery::ResourceIdSuffix(":id/rr5") => vec![element(180.0, Some("Artist"))],
                ElementQuery::ResourceIdSuffix(":id/so9")
                    if self.closed.load(Ordering::Relaxed)
                        && self.selected.load(Ordering::Relaxed) =>
                {
                    vec![element(20.0, Some(self.editor_title))]
                }
                _ => vec![],
            })
        }
    }

    #[tokio::test(start_paused = true)]
    async fn inline_sound_desired_state_never_toggles_an_already_selected_track_off() {
        for selected in [false, true] {
            let session = InlineSession {
                list_reads: AtomicUsize::new(0),
                list_read_limit: if selected { 4 } else { usize::MAX },
                selected: AtomicBool::new(selected),
                closed: AtomicBool::new(false),
                taps: AtomicUsize::new(0),
                select_takes: true,
                marker_id: ":id/dfu",
                editor_title: "One",
            };
            let plan =
                SoundPickerPlan::resolve("com.ss.android.ugc.trill", "en", "38.3.2").unwrap();
            let pool = observe_sound_pool(&session, plan, 1).await.unwrap();
            choose_and_confirm_sound(&session, plan, &pool, 0)
                .await
                .unwrap();
            assert_eq!(session.taps.load(Ordering::Relaxed), usize::from(!selected));
            assert!(session.closed.load(Ordering::Relaxed));
            assert!(session.selected.load(Ordering::Relaxed));
        }
    }

    #[tokio::test(start_paused = true)]
    async fn unconfirmed_sound_selection_stops_without_another_tap_or_closing_picker() {
        let session = InlineSession {
                list_reads: AtomicUsize::new(0),
                list_read_limit: usize::MAX,
            selected: AtomicBool::new(false),
            closed: AtomicBool::new(false),
            taps: AtomicUsize::new(0),
            select_takes: false,
            marker_id: ":id/dfu",
            editor_title: "One",
        };
        let plan = SoundPickerPlan::resolve("com.ss.android.ugc.trill", "en", "38.3.2").unwrap();
        let pool = observe_sound_pool(&session, plan, 1).await.unwrap();
        assert!(choose_and_confirm_sound(&session, plan, &pool, 0)
            .await
            .is_err());
        assert_eq!(session.taps.load(Ordering::Relaxed), 1);
        assert!(!session.closed.load(Ordering::Relaxed));
    }

    #[tokio::test(start_paused = true)]
    async fn carousel_sound_equalizer_confirms_selected_track_without_trim_control() {
        // Measured carousel row on trill/en/38.3.2: jk1 appears inside the selected
        // ta8 row; dfu is absent. Editor so9 must still confirm the exact title.
        for selected in [false, true] {
            let session = InlineSession {
                list_reads: AtomicUsize::new(0),
                list_read_limit: usize::MAX,
                selected: AtomicBool::new(selected),
                closed: AtomicBool::new(false),
                taps: AtomicUsize::new(0),
                select_takes: true,
                marker_id: ":id/jk1",
                editor_title: "One",
            };
            let plan =
                SoundPickerPlan::resolve("com.ss.android.ugc.trill", "en", "38.3.2").unwrap();
            let pool = observe_sound_pool(&session, plan, 1).await.unwrap();
            choose_and_confirm_sound(&session, plan, &pool, 0)
                .await
                .unwrap();
            assert_eq!(session.taps.load(Ordering::Relaxed), usize::from(!selected));
            assert!(session.closed.load(Ordering::Relaxed));
            assert!(session.selected.load(Ordering::Relaxed));
        }
    }

    #[tokio::test(start_paused = true)]
    async fn carousel_marker_does_not_replace_editor_title_readback() {
        let session = InlineSession {
                list_reads: AtomicUsize::new(0),
                list_read_limit: usize::MAX,
            selected: AtomicBool::new(false),
            closed: AtomicBool::new(false),
            taps: AtomicUsize::new(0),
            select_takes: true,
            marker_id: ":id/jk1",
            editor_title: "Different sound",
        };
        let plan = SoundPickerPlan::resolve("com.ss.android.ugc.trill", "en", "38.3.2").unwrap();
        let pool = observe_sound_pool(&session, plan, 1).await.unwrap();
        let error = choose_and_confirm_sound(&session, plan, &pool, 0)
            .await
            .unwrap_err();
        assert!(
            error
                .downcast_ref::<crate::driver::UiError>()
                .is_some_and(|error| error.op == "observe"
                    && matches!(error.kind, crate::driver::UiErrorKind::Timeout)),
            "wrong editor title must not be accepted before the readback deadline: {error:#}"
        );
        assert_eq!(session.taps.load(Ordering::Relaxed), 1);
        assert!(session.closed.load(Ordering::Relaxed));
    }

    #[test]
    fn inline_markers_must_all_belong_to_one_candidate_row() {
        let plan = SoundPickerPlan::resolve("com.ss.android.ugc.trill", "en", "38.3.2").unwrap();
        let rows = vec![
            ElementBox {
                height: 100.0,
                ..element(100.0, None)
            },
            ElementBox {
                height: 100.0,
                ..element(200.0, None)
            },
        ];
        let titles = vec![element(115.0, Some("One")), element(215.0, Some("Two"))];
        let artists = vec![element(155.0, Some("A")), element(255.0, Some("B"))];
        let same_row = assemble_pool(
            plan,
            rows.clone(),
            titles.clone(),
            artists.clone(),
            vec![],
            vec![element(120.0, None), element(130.0, None)],
            5,
        )
        .unwrap();
        assert_eq!(same_row.selected_index, Some(0));
        let different_rows = assemble_pool(
            plan,
            rows,
            titles,
            artists,
            vec![],
            vec![element(120.0, None), element(220.0, None)],
            5,
        )
        .unwrap_err();
        assert!(different_rows
            .to_string()
            .contains("ambiguous selected sound row"));
    }

    #[test]
    fn sound_reproof_rejects_changed_pool_and_uses_fresh_position() {
        let expected = ObservedSoundPool {
            visual: false,
            effective_plan: None,
            maximum_visible: 5,
            selected_index: None,
            candidates: vec![SoundCandidate {
                section: "recommended".into(),
                title: "One".into(),
                artist: "Artist".into(),
            }],
            targets: vec![element(100.0, None)],
        };
        let mut fresh = expected.clone();
        fresh.targets[0].y = 200.0;
        assert_eq!(reproof_target(&expected, &fresh, 0).unwrap().y, 200.0);
        fresh.candidates.push(SoundCandidate {
            section: "recommended".into(),
            title: "Other loaded row".into(),
            artist: "Another".into(),
        });
        assert!(
            reproof_target(&expected, &fresh, 0).is_ok(),
            "unrelated rows loading cannot invalidate the exact chosen title/artist/section"
        );
        fresh.candidates[0].artist = "Other".into();
        assert!(reproof_target(&expected, &fresh, 0).is_err());
        fresh.candidates = expected.candidates.clone();
        fresh.targets[0].enabled = false;
        assert!(reproof_target(&expected, &fresh, 0).is_err());
    }

    fn element(y: f64, description: Option<&str>) -> ElementBox {
        ElementBox {
            x: 0.0,
            y,
            width: 500.0,
            height: 50.0,
            description: description.map(str::to_string),
            enabled: true,
            clickable: false,
        }
    }

    #[test]
    fn plans_are_exactly_version_and_locale_keyed() {
        let trill = SoundPickerPlan::resolve("com.ss.android.ugc.trill", "en", "38.3.2").unwrap();
        assert!(trill.choose_id.is_none() && trill.closes_with_back());
        assert!(SoundPickerPlan::resolve("com.ss.android.ugc.trill", "en-US", "38.3.2").is_some());
        assert!(SoundPickerPlan::resolve("com.zhiliaoapp.musically", "en", "46.2.1").is_some());
        assert!(SoundPickerPlan::resolve("com.zhiliaoapp.musically", "en", "46.2.42").is_some());
        assert!(SoundPickerPlan::resolve("com.ss.android.ugc.trill", "vi", "38.3.2").is_none());
        assert!(SoundPickerPlan::resolve("com.zhiliaoapp.musically", "en-US", "46.0.41").is_some());
        assert!(SoundPickerPlan::resolve("com.zhiliaoapp.musically", "en-US", "45.7.3").is_some());
        for version in [
            "45.7.2", "45.7.4", "46.1.4", "46.0.40", "46.0.42", "46.2", "",
        ] {
            assert!(SoundPickerPlan::resolve("com.zhiliaoapp.musically", "en", version).is_none());
        }
    }

    #[test]
    fn entry_ids_do_not_choose_layout_close_route_or_provenance() {
        for measurement in MEASURED_SOUND_PICKERS {
            let plan = measurement.plan;
            let moved = SoundPickerPlan {
                entry_id: ":id/new_entry_fixture",
                ..plan
            };
            assert_eq!(moved.snapshot_layout(), plan.snapshot_layout());
            assert_eq!(moved.closes_with_back(), plan.closes_with_back());
            assert_eq!(moved.post_back_query(), plan.post_back_query());
            assert_eq!(moved.provenance(), plan.provenance());
        }
        let inline = SoundPickerPlan::resolve("com.zhiliaoapp.musically", "en", "46.2.1").unwrap();
        let decoy = SoundPickerPlan {
            entry_id: ":id/dv3",
            ..inline
        };
        assert_eq!(decoy.snapshot_layout().unwrap().tab_id, ":id/x4y");
        assert!(decoy.closes_with_back());
        assert_eq!(
            decoy.post_back_query(),
            Some(ElementQuery::ResourceIdSuffix(":id/bot"))
        );
    }

    #[tokio::test(start_paused = true)]
    async fn inline_marker_locator_is_plan_data_not_a_trill_constant() {
        let base = SoundPickerPlan::resolve("com.ss.android.ugc.trill", "en", "38.3.2").unwrap();
        let plan = SoundPickerPlan {
            entry_id: ":id/fixture_entry",
            selection: SoundSelectionMode::Inline {
                marker_ids: &[":id/fixture_marker"],
            },
            ..base
        };
        let session = InlineSession {
                list_reads: AtomicUsize::new(0),
                list_read_limit: usize::MAX,
            selected: AtomicBool::new(true),
            closed: AtomicBool::new(false),
            taps: AtomicUsize::new(0),
            select_takes: true,
            marker_id: ":id/fixture_marker",
            editor_title: "One",
        };
        let pool = observe_sound_pool(&session, plan, 1).await.unwrap();
        assert_eq!(pool.selected_index, Some(0));
        choose_and_confirm_sound(&session, plan, &pool, 0)
            .await
            .unwrap();
        assert_eq!(session.taps.load(Ordering::Relaxed), 0);
        assert!(session.closed.load(Ordering::Relaxed));
    }

    #[test]
    fn choose_control_and_selected_marker_are_independent() {
        let base = SoundPickerPlan::resolve("com.ss.android.ugc.trill", "en", "38.3.2").unwrap();
        let plan = SoundPickerPlan {
            choose_id: Some(":id/fixture_choose"),
            ..base
        };
        let row = ElementBox {
            height: 100.0,
            ..element(100.0, None)
        };
        let choice = element(125.0, None);
        let pool = assemble_pool(
            plan,
            vec![row],
            vec![element(115.0, Some("One"))],
            vec![element(155.0, Some("Artist"))],
            vec![choice.clone()],
            vec![],
            1,
        )
        .unwrap();
        assert_eq!(pool.selected_index, None);
        assert_eq!(pool.target(0), Some(&choice));
    }

    #[test]
    fn row_assembly_keeps_title_artist_and_tap_target_bound_together() {
        let plan = SoundPickerPlan::resolve("com.ss.android.ugc.trill", "en", "38.3.2")
            .expect("measured plan");
        let rows = vec![
            ElementBox {
                height: 100.0,
                ..element(100.0, None)
            },
            ElementBox {
                height: 100.0,
                ..element(200.0, None)
            },
        ];
        let titles = vec![
            element(120.0, Some("First")),
            element(220.0, Some("Second")),
        ];
        let artists = vec![
            element(160.0, Some("Artist A · 10K posts")),
            element(260.0, Some("Artist B · 20K posts")),
        ];
        let markers = vec![element(125.0, None)];
        let pool = assemble_pool(plan, rows, titles, artists, vec![], markers, 5).expect("pool");
        assert_eq!(
            pool.candidates,
            vec![
                SoundCandidate {
                    section: "recommended".into(),
                    title: "First".into(),
                    artist: "Artist A".into(),
                },
                SoundCandidate {
                    section: "recommended".into(),
                    title: "Second".into(),
                    artist: "Artist B".into(),
                },
            ]
        );
        assert_eq!(pool.targets.len(), 2);
        assert_eq!(pool.selected_index, Some(0));
    }

    #[test]
    fn incomplete_or_ambiguous_rows_fail_closed() {
        let plan = SoundPickerPlan::resolve("com.ss.android.ugc.trill", "en", "38.3.2")
            .expect("measured plan");
        let row = ElementBox {
            height: 100.0,
            ..element(100.0, None)
        };
        assert!(assemble_pool(
            plan,
            vec![row],
            vec![element(120.0, Some("One")), element(130.0, Some("Two"))],
            vec![element(160.0, Some("Artist"))],
            vec![],
            vec![element(140.0, None)],
            5,
        )
        .is_err());
    }

    #[test]
    fn duplicate_recommendations_keep_unique_rows_and_the_original_read_window() {
        let plan = SoundPickerPlan::resolve("com.ss.android.ugc.trill", "en", "38.3.2").unwrap();
        let rows = [100.0, 200.0, 300.0]
            .into_iter()
            .map(|y| ElementBox {
                height: 90.0,
                ..element(y, None)
            })
            .collect();
        let pool = assemble_pool(
            plan,
            rows,
            vec![
                element(120.0, Some("Same")),
                element(220.0, Some("Same")),
                element(320.0, Some("Distinct")),
            ],
            vec![
                element(160.0, Some("A")),
                element(260.0, Some("B")),
                element(360.0, Some("C")),
            ],
            vec![],
            vec![element(325.0, None)],
            5,
        )
        .unwrap();
        assert_eq!(pool.candidates.len(), 1);
        assert_eq!(pool.candidates[0].title, "Distinct");
        assert_eq!(pool.selected_index, Some(0));
        assert_eq!(pool.maximum_visible, 5);
        assert_eq!(pool.target(0).unwrap().y, 320.0);
    }

    #[test]
    fn duplicate_titles_with_different_artists_fail_closed() {
        let plan = SoundPickerPlan::resolve("com.ss.android.ugc.trill", "en", "38.3.2")
            .expect("measured plan");
        let rows = vec![
            ElementBox {
                height: 100.0,
                ..element(100.0, None)
            },
            ElementBox {
                height: 100.0,
                ..element(200.0, None)
            },
        ];
        let result = assemble_pool(
            plan,
            rows,
            vec![
                element(120.0, Some("Same title")),
                element(220.0, Some("Same title")),
            ],
            vec![
                element(160.0, Some("Artist A")),
                element(260.0, Some("Artist B")),
            ],
            vec![],
            vec![element(125.0, None)],
            5,
        );
        assert!(result
            .expect_err("title-only readback cannot distinguish the two rows")
            .to_string()
            .contains("duplicate titles"));
    }
}
