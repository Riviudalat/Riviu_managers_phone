//! Riviu core types, device registry, job queue, and persistence.

pub mod db;
pub mod device_capabilities;
pub mod device_control;
pub mod device_work;
pub mod driver;
pub mod driver_multiplex;
pub mod events;
pub mod feed_ladder;
pub mod flow;
pub mod frame_source;
pub mod frame_text;
pub mod group_sync;
pub mod human_behavior;
pub mod interaction;
pub mod interaction_campaign;
pub mod interaction_hierarchy;
pub mod interaction_lifecycle;
pub mod interaction_target;
pub mod interaction_threshold;
pub mod job_queue;
pub mod nurture;
pub mod openai_client;
pub mod publish;
pub mod publish_partners;
pub mod publish_sheet;
pub mod registry;
pub mod screen;
pub mod screen_match;
pub mod screen_watch;
pub mod session_log;
pub mod stream_budget;
pub mod tiktok_composer;
pub mod tiktok_drawer;
pub mod tiktok_labels;
pub mod tiktok_like;
pub mod tiktok_share;
pub mod tiktok_target;
pub mod tiktok_web;
pub mod types;

pub use device_capabilities::*;
pub use device_control::{
    ContextReleaseProof, DeviceControlError, DeviceControlPlane, DeviceExclusiveContext,
    DeviceLeaseRef, DeviceReleaseProof, ForegroundAppProof, InteractionAcquireResult,
    UiCapacityReservation, UiSessionContext, UiWithStreamContext,
};
pub use device_work::{
    DeviceBusy, DeviceWorkAcquireError, DeviceWorkCoordinator, DeviceWorkLease, DeviceWorkOwner,
    DeviceWorkTokenError,
};
pub use driver::{
    ui_error_kind, AppProcessState, DeviceDriver, ElementBox, ElementQuery,
    GuardedClipboardOperation, GuardedClipboardOutput, GuardedClipboardProgress,
    GuardedClipboardTransition, MediaPullReport, ProcessAbsenceProof, UiError, UiErrorKind,
    UiSession, UnsupportedCapability,
};
pub use events::{AppEvent, EventBus};
pub use feed_ladder::{step as feed_ladder_step, LadderSpend, LadderStep, FEED_LADDER};
pub use flow::*;
pub use frame_source::{
    decodes_as_jpeg, Frame, FrameSink, FrameSource, FrameStream, GenerationFrame,
    GenerationFrameEvent, GenerationFrameSource, GenerationFrameStream, NullFrameSource,
};
pub use frame_text::{FrameTextSource, NullFrameTextSource};
pub use group_sync::{apply_offset, DelayPolicy, DevicePlan, GroupSyncPolicy, OffsetPolicy};
pub use interaction::*;
pub use interaction_hierarchy::{
    discover_identity_in_elements, locate_parent_in_elements, ElementReplyTarget,
};
pub use interaction_lifecycle::{
    InteractionLifecycleRegistry, InteractionSessionReservation, InteractionStreamReservation,
};
pub use job_queue::JobQueue;
pub use nurture::{NurtureEngine, Outcome};
pub use publish::*;
pub use registry::DeviceRegistry;
pub use screen::{ScreenKind, ScreenObservation};
pub use screen_watch::{ScreenWatcher, WatchStats};
pub use session_log::{SessionLogBook, SessionLogEntry, SessionLogSummary, SESSION_LOG_CAPACITY};
pub use stream_budget::{
    BackgroundStreamLease, ForegroundStreamReservation, ForegroundTransfer, StreamBudgetError,
    StreamBudgetManager, StreamStopRequest,
};
pub use types::*;

/// Every tuned number carries its reason, and the reason is checked.
///
/// `AGENTS.md` has said since the beginning that a geometric constant must carry a real
/// measurement naming the phone it was taken on. It is the rule the repo breaks most often,
/// and nothing enforced it — so the check is here rather than in a review checklist.
///
/// Scoped to the two kinds of number that are *choices*: screen fractions and durations.
/// A protocol tag (`VIEW_KIND_H264 = 1`), a keycode, a schema version and a byte offset are
/// not tuned — they are dictated from outside, and demanding a rationale for them would fill
/// the tree with `/// One.` and teach everyone to ignore the rule.
#[cfg(test)]
mod tuned_constant_tests {
    /// A constant is explained if a comment sits above it, or above the block it belongs to.
    ///
    /// Block-aware on purpose: `ALERT_SEARCH_Y` / `ALERT_BAND_H` / `ALERT_PANEL_X` are one
    /// idea written as four numbers, and one comment over the group says more than four
    /// comments repeating each other.
    fn undocumented(source: &str, kinds: &[&str]) -> Vec<String> {
        let lines: Vec<&str> = source.lines().collect();
        let is_const = |line: &str| {
            let t = line.trim_start();
            t.starts_with("const ")
                || t.starts_with("pub const ")
                || t.starts_with("pub(crate) const ")
        };
        let mut out = Vec::new();
        let mut test_from = usize::MAX;
        for (i, line) in lines.iter().enumerate() {
            if line.trim_start().starts_with("#[cfg(test)]") && test_from == usize::MAX {
                test_from = i;
            }
            if i >= test_from || !is_const(line) {
                continue;
            }
            let Some((decl, _)) = line.split_once('=') else {
                continue;
            };
            let Some((name, ty)) = decl.split_once(':') else {
                continue;
            };
            let ty = ty.trim();
            if !kinds.contains(&ty) {
                continue;
            }
            let name = name.rsplit(' ').next().unwrap_or(name).trim();

            let mut j = i;
            let documented = loop {
                if j == 0 {
                    break false;
                }
                j -= 1;
                let prev = lines[j].trim();
                if prev.starts_with("#[") {
                    continue;
                }
                if prev.starts_with("//") {
                    break true;
                }
                if is_const(lines[j]) {
                    continue;
                }
                break false;
            };
            if !documented {
                out.push(format!("{name}: {ty}"));
            }
        }
        out
    }

    /// Files worth checking. Not every file in the crate — the point is coverage of the
    /// modules that drive phones, which is where a bare number does damage.
    const TUNED_SOURCES: &[(&str, &str)] = &[
        (
            "interaction_hierarchy.rs",
            include_str!("interaction_hierarchy.rs"),
        ),
        (
            "interaction_threshold.rs",
            include_str!("interaction_threshold.rs"),
        ),
        (
            "interaction_campaign.rs",
            include_str!("interaction_campaign.rs"),
        ),
        ("openai_client.rs", include_str!("openai_client.rs")),
        ("screen.rs", include_str!("screen.rs")),
        ("tiktok_drawer.rs", include_str!("tiktok_drawer.rs")),
        ("tiktok_like.rs", include_str!("tiktok_like.rs")),
        ("nurture/actions.rs", include_str!("nurture/actions.rs")),
        ("nurture/hierarchy.rs", include_str!("nurture/hierarchy.rs")),
        ("nurture/mod.rs", include_str!("nurture/mod.rs")),
        ("flow/executor.rs", include_str!("flow/executor.rs")),
        ("flow/runtime.rs", include_str!("flow/runtime.rs")),
    ];

    #[test]
    fn a_geometric_constant_says_where_its_number_came_from() {
        // AGENTS.md: "Mọi hằng số hình học phải kèm số đo thật ... ghi rõ đo trên máy nào."
        // A screen fraction with no note is a number nobody can re-derive: the next person
        // cannot tell a measurement from a guess, so they will not touch it, and it stays
        // wrong on whichever phone it was never checked against.
        let mut bare = Vec::new();
        for (file, source) in TUNED_SOURCES {
            for c in undocumented(source, &["f32", "f64", "(f32, f32)", "(f64, f64)"]) {
                bare.push(format!("{file}: {c}"));
            }
        }
        assert!(
            bare.is_empty(),
            "geometric constants with no measurement:\n  {}",
            bare.join("\n  ")
        );
    }

    #[test]
    fn a_timing_constant_says_what_it_is_waiting_for() {
        // The same argument one step weaker: a duration cannot carry a measurement the way a
        // coordinate can, but it can say what it bounds and what goes wrong on each side of
        // it. Ten of these had nothing at all, including the two that decide whether a
        // comment is typed into a field that has focus yet.
        let mut bare = Vec::new();
        for (file, source) in TUNED_SOURCES {
            for c in undocumented(source, &["Duration"]) {
                bare.push(format!("{file}: {c}"));
            }
        }
        assert!(
            bare.is_empty(),
            "timing constants with no explanation:\n  {}",
            bare.join("\n  ")
        );
    }

    #[test]
    fn the_scan_can_actually_see_the_constants_it_checks() {
        // Every assertion above passes trivially if the scanner finds nothing, and a silent
        // zero is exactly how a source-scanning test rots. So make it prove it reads them.
        let found: usize = TUNED_SOURCES
            .iter()
            .map(|(_, s)| {
                s.lines()
                    .filter(|l| {
                        let t = l.trim_start();
                        (t.starts_with("const ") || t.starts_with("pub const "))
                            && (t.contains(": f64") || t.contains(": Duration"))
                    })
                    .count()
            })
            .sum();
        assert!(found > 40, "the scanner sees only {found} tuned constants");
    }
}
