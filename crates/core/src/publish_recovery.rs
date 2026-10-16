//! Bounded pre-Post recovery. This never grants permission to repeat Post.
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};
use ts_rs::TS;

/// Pre-Post retries per step, shared by retries inside a step and whole fresh attempts.
///
/// r122 (15/10/2026, 20 SM-G955F/N phones, ten composing at once): eight rows spent the old
/// three retries within the first 23 minutes, while every read took 14-30 s, and ended
/// `failed_before_dispatch` with nothing posted. Rows whose next attempt came after that wave
/// posted at once (r117 phones 37/43/49 on one retry; r122 phone 47 on its sixth attempt).
/// A cap, not a target: nothing here ever repeats Post.
pub const PRE_POST_RETRY_BUDGET: u32 = 5;

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct PublishRecoveryState {
    pub step: String,
    pub checkpoint: String,
    pub state: String,
    pub retries_used: u32,
    pub max_retries: u32,
    pub next_retry_at: Option<f64>,
    pub reconnect_deadline: Option<f64>,
    pub last_error: Option<String>,
    #[serde(default)]
    pub last_error_code: Option<String>,
    #[serde(default)]
    pub last_error_kind: Option<String>,
    pub manual: bool,
    #[serde(default)]
    pub reconnect_retry: bool,
    #[serde(default)]
    pub expected_account: String,
    #[serde(default)]
    pub observed_account: Option<String>,
    #[serde(default)]
    pub counts: BTreeMap<String, u32>,
    #[serde(default)]
    #[ts(type = "unknown | null")]
    pub sound: Option<crate::SoundSelectionEvidence>,
    /// When this row first waited for an earlier post on its phone (`priorPostWaiting`), in
    /// epoch ms. The wait's deadline counts from here; an operator retry clears it.
    #[serde(default)]
    pub prior_post_wait_since: Option<f64>,
}
impl Default for PublishRecoveryState {
    fn default() -> Self {
        Self {
            step: "device".into(),
            checkpoint: "approved".into(),
            state: "running".into(),
            retries_used: 0,
            max_retries: PRE_POST_RETRY_BUDGET,
            next_retry_at: None,
            reconnect_deadline: None,
            last_error: None,
            last_error_code: None,
            last_error_kind: None,
            manual: false,
            reconnect_retry: false,
            expected_account: String::new(),
            observed_account: None,
            counts: BTreeMap::new(),
            sound: None,
            prior_post_wait_since: None,
        }
    }
}

/// A phone's earlier post of another campaign is not yet proven on TikTok or verified, or its
/// release did not complete, so this row must not send media or close TikTok yet. Requeued
/// without spending the retry budget; the prior-post wait (`PRIOR_POST_WAIT_MINUTES`) bounds it.
pub const PRIOR_POST_PENDING: &str = "prior_post_pending";

/// Typed Retryable failure for [`PRIOR_POST_PENDING`]: nothing was sent to the phone.
pub fn prior_post_pending(message: impl Into<String>) -> RecoveryFailure {
    RecoveryFailure::new(PRIOR_POST_PENDING, FailureKind::Retryable, message)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FailureKind {
    Disconnected,
    Retryable,
    Terminal,
}

impl FailureKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Disconnected => "disconnected",
            Self::Retryable => "retryable",
            Self::Terminal => "terminal",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecoveryFailure {
    pub code: String,
    pub kind: FailureKind,
    pub message: String,
}

impl std::fmt::Display for RecoveryFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for RecoveryFailure {}

impl RecoveryFailure {
    pub fn new(code: impl Into<String>, kind: FailureKind, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            kind,
            message: message.into(),
        }
    }

    /// Compatibility boundary for persisted jobs and drivers that still return text-only errors.
    pub fn legacy(message: impl Into<String>) -> Self {
        let message = message.into();
        let kind = classify(&message);
        let code = match kind {
            FailureKind::Disconnected => "legacy_device_disconnected",
            FailureKind::Retryable
                if crate::device_control::baseline::mentions_screen_locked(&message) =>
            {
                crate::device_control::baseline::SCREEN_LOCKED_CODE
            }
            FailureKind::Retryable
                if crate::device_control::load::mentions_device_overloaded(&message) =>
            {
                crate::device_control::load::DEVICE_OVERLOADED_CODE
            }
            FailureKind::Retryable if unavailable_android_agent_status(&message) => {
                "agent_status_unavailable"
            }
            FailureKind::Retryable if blind_android_agent(&message) => {
                "agent_accessibility_unavailable"
            }
            FailureKind::Retryable if adb_daemon_connect_timeout(&message) => {
                "adb_daemon_connect_timeout"
            }
            FailureKind::Retryable if adb_transfer_daemon_start_failed(&message) => {
                "adb_daemon_start_failed"
            }
            FailureKind::Retryable
                if foreground_read_failed_in_transport(&message.to_lowercase()) =>
            {
                "adb_foreground_read_failed"
            }
            FailureKind::Retryable => "legacy_retryable",
            FailureKind::Terminal => "legacy_terminal",
        };
        Self::new(code, kind, message)
    }

    /// A second attempt inside the driver's quiet window only burns the
    /// publication's bounded retry budget. The driver reports the remaining
    /// seconds; the first failed restart needs its whole measured window.
    pub fn minimum_retry_delay(&self) -> Option<Duration> {
        if self.code == "agent_status_unavailable" {
            return Some(Duration::from_secs(65));
        }
        if self.code != "agent_accessibility_unavailable" {
            return None;
        }
        let remaining = self
            .message
            .rsplit_once("Not restarting again for another ")
            .and_then(|(_, tail)| tail.split_once('s'))
            .and_then(|(seconds, _)| seconds.parse::<u64>().ok())
            .filter(|seconds| *seconds <= 120);
        Some(Duration::from_secs(
            remaining.map_or(65, |seconds| seconds + 1),
        ))
    }
}

fn blind_android_agent(message: &str) -> bool {
    let message = message.to_ascii_lowercase();
    message.contains("the agent on ")
        && message.contains("is listening but cannot read the accessibility tree")
}

fn unavailable_android_agent_status(message: &str) -> bool {
    let message = message.to_ascii_lowercase();
    message.contains("the agent on ") && message.contains("did not answer /status within")
}

fn adb_daemon_connect_timeout(message: &str) -> bool {
    let message = message.to_ascii_lowercase();
    (message.contains("adb.exe: cannot connect to daemon at tcp:5037:")
        || message.contains("error: cannot connect to daemon at tcp:5037:"))
        && message.contains("cannot connect to 127.0.0.1:5037:")
        && message.contains("(10060)")
}

/// TikTok re-rendered the caption field between finding it and `set_text` (r123 phone 49,
/// 15/10/2026; once before on 12/10). `ACTION_SET_TEXT` is one accessibility action, so a
/// detached element took no text, and the composer only types before Post.
fn caption_field_detached(lowered: &str) -> bool {
    lowered.contains("/value lỗi 404")
        && lowered.contains("the referenced element is no longer attached")
}

/// adb lost its server during the media transfer or the read-only photo-order proof that
/// follows it (r125 phone 32, 15/10/2026: `sha256sum` of an imported image). Both run before
/// any composer; a tap or Post with the same text stays terminal.
fn adb_transfer_daemon_start_failed(message: &str) -> bool {
    let message = message.to_ascii_lowercase();
    (message.contains("stagepublishmedia failed for device ")
        || message.contains("ensurepublishphotoorder failed for device "))
        && message.contains("* daemon not running; starting now at tcp:5037")
        && message.contains("could not read ok from adb server")
        && message.contains("* failed to start daemon")
        && message.contains("cannot connect to daemon")
}

/// Every source of the adb foreground read failed in transport: the adb server restarting
/// ("* daemon not running; starting now at tcp:5037") or `adb shell` dying with empty output.
/// r95 (12/10/2026, campaign 4a25d804, box 31-50): four rows composing when the server dropped
/// for 52 s ended `legacy_terminal` on their first read. It is a dumpsys read, never an
/// effect. A system window over the app (lock screen, system dialog) and an unreadable focus
/// line keep their verdict; `lowercase` is the message already lowercased.
fn foreground_read_failed_in_transport(lowercase: &str) -> bool {
    lowercase.contains("could not read the foreground package. tried: ")
        && !lowercase.contains("reported the system window")
        && !lowercase.contains("had no readable mcurrentfocus line")
}

#[derive(Debug)]
struct ClassifiedRecoveryError(RecoveryFailure);

impl std::fmt::Display for ClassifiedRecoveryError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0.message)
    }
}

impl std::error::Error for ClassifiedRecoveryError {}

pub fn retryable_error(code: &'static str, message: impl Into<String>) -> anyhow::Error {
    ClassifiedRecoveryError(RecoveryFailure::new(code, FailureKind::Retryable, message)).into()
}

/// A failure while staging media, before any composer. Nothing reached TikTok and a re-run
/// reuses the import identity, so an unrecognised adb or read error spends the assignment's
/// bounded retry budget instead of ending the row. Measured 14/10/2026: r112 phone 32
/// (`ce031713a8f5bd0805`) `adb push` "failed to read copy response: EOF" while its view
/// stream dropped in the same second; in r111 two rows read the TikTok version as unknown
/// under load and one failed the photo-order `content update`. All four settled
/// `legacy_terminal` on their first try. A deliberate refusal in the chain still ends it.
pub fn media_transfer_failure(error: &anyhow::Error) -> RecoveryFailure {
    let failure = describe(error);
    if failure.kind != FailureKind::Terminal || specific_refusal(&failure.message.to_lowercase()) {
        return failure;
    }
    RecoveryFailure::new(
        "media_transfer_failed",
        FailureKind::Retryable,
        failure.message,
    )
}

/// adb could not reach the local server while a publish session was opening, and failed to
/// start a second one. Measured r117, 14/10/2026, phone 49 (`ce071827715f321b05`): `wm size`
/// in startStreamAfterSession printed this while the server running since 05:14 kept serving
/// the other 19 phones, and the row ended `post_refused_before_dispatch` on its first try.
/// Opening never reaches TikTok's composer, so the row spends its bounded retry budget; the
/// same text from a tap or a Post stays terminal (`classify`).
pub fn session_open_adb_daemon_failure(error: &anyhow::Error) -> Option<RecoveryFailure> {
    let message = format!("{error:#}");
    let lowercase = message.to_lowercase();
    (lowercase.contains("* daemon not running; starting now at tcp:5037")
        && lowercase.contains("could not read ok from adb server")
        && lowercase.contains("* failed to start daemon")
        && lowercase.contains("cannot connect to daemon")
        && !specific_refusal(&lowercase))
    .then(|| RecoveryFailure::new("adb_daemon_start_failed", FailureKind::Retryable, message))
}

/// A read phase exhausted its existing budget; this never describes a gesture.
pub(crate) fn observation_deadline() -> anyhow::Error {
    crate::driver::UiError::new(
        crate::driver::UiErrorKind::Timeout,
        "observe",
        "publish observation deadline exceeded",
    )
    .into()
}

/// A pre-Post observation whose session (or foreground) changed while it was read, with no
/// driver proof of what replaced it.
pub const COMPOSER_OBSERVATION_INVALIDATED: &str = "composer_observation_invalidated";

/// [`COMPOSER_OBSERVATION_INVALIDATED`], naming what changed (`before` and `after`: the two
/// session epochs or foreground packages). The agent's stale-root recreate-and-replay swaps
/// the session in place and its replayed read succeeds untyped, so every target read under
/// the old epoch is void. Nothing is tapped from it: the attempt ends, and the dispatcher's
/// fresh attempt prepares account, media and sound again within [`PRE_POST_RETRY_BUDGET`]
/// ([`requires_fresh_preparation`]). It never claims the driver's proven repair
/// (`publish_observation_session_repaired`) and is raised only before the Post intent.
pub fn observation_invalidated(what: &str, before: &str, after: &str) -> RecoveryFailure {
    RecoveryFailure::new(
        COMPOSER_OBSERVATION_INVALIDATED,
        FailureKind::Retryable,
        format!(
            "{what} (trước: {before}; sau: {after}): màn hình đã đổi giữa lúc đọc nên bỏ mọi \
             mục tiêu đã đọc, không bấm gì thêm; lượt sau chuẩn bị lại từ đầu"
        ),
    )
}

pub fn describe(error: &anyhow::Error) -> RecoveryFailure {
    if let Some(failure) = error.downcast_ref::<RecoveryFailure>() {
        return failure.clone();
    }
    if let Some(diagnostic) = error.downcast_ref::<crate::tiktok_account::AccountDiagnostic>() {
        if let Some(failure) = diagnostic.failure() {
            return failure;
        }
    }
    if let Some(classified) = error.downcast_ref::<ClassifiedRecoveryError>() {
        return classified.0.clone();
    }
    // Android refused an injected event of a navigation gesture (r123/r124 phone 37,
    // 15/10/2026: `/actions` 400 "Unable to perform W3C actions" while the pre-Post account
    // read moved to the Profile). The Post tap never reaches here: the composer turns its
    // failure into PostNotConfirmed, and the intent gate still refuses any retry after Post.
    if error
        .chain()
        .any(|cause| cause.is::<crate::driver::GestureCancelled>())
    {
        return RecoveryFailure::new(
            "gesture_cancelled",
            FailureKind::Retryable,
            format!("{error:#}"),
        );
    }
    if error.is::<crate::tiktok_sound::SoundNetworkUnavailable>() {
        return RecoveryFailure::new(
            "sound_network_unavailable",
            FailureKind::Retryable,
            format!("{error:#}"),
        );
    }
    if error.is::<crate::tiktok_sound::SoundRootUnreadable>() {
        return RecoveryFailure::new(
            "sound_root_unreadable",
            FailureKind::Retryable,
            format!("{error:#}"),
        );
    }
    if error.is::<crate::tiktok_sound::SoundStopped>() {
        return RecoveryFailure::new(
            "sound_cancelled",
            FailureKind::Terminal,
            format!("{error:#}"),
        );
    }
    if error.is::<crate::driver::SessionEpochChanged>() {
        return RecoveryFailure::new(
            "publish_observation_session_repaired",
            FailureKind::Retryable,
            format!("{error:#}"),
        );
    }
    if let Some(ui) = error.downcast_ref::<crate::driver::UiError>() {
        use crate::driver::UiErrorKind;
        let classified = match (ui.op.as_str(), ui.kind) {
            ("observe", UiErrorKind::Timeout) => {
                Some(("publish_observation_deadline", FailureKind::Retryable))
            }
            ("observe", UiErrorKind::Transport) => {
                Some(("publish_observation_transient", FailureKind::Retryable))
            }
            (
                "tap" | "activate_element" | "typeText" | "type_text" | "swipe" | "back" | "home",
                UiErrorKind::Timeout,
            ) => Some(("publish_action_outcome_unknown", FailureKind::Terminal)),
            _ => None,
        };
        if let Some((code, kind)) = classified {
            return RecoveryFailure::new(code, kind, format!("{error:#}"));
        }
    }
    let failure = RecoveryFailure::legacy(format!("{error:#}"));
    // Drivers raise this type for reads only: no root answered, so the screen is unknown, not
    // refused. Run 72401bfd (other PC, 2026-10-12, ce0417141550322202 and ce031713f3d3ec1d0c):
    // the every-window wording "đọc qua mọi cửa sổ không có cửa sổ nào …" has no retry word and
    // settled `legacy_terminal` at the sound step's preRow, while the 500 wording of the same read
    // ("Timed out …") stayed retryable. A refusal word in the chain still ends the attempt, and
    // no retry is reserved once a Post intent exists.
    if failure.kind == FailureKind::Terminal
        && error.is::<crate::driver::AccessibilityReadUnavailable>()
        && !specific_refusal(&failure.message.to_lowercase())
    {
        return RecoveryFailure::new(
            "accessibility_read_unavailable",
            FailureKind::Retryable,
            failure.message,
        );
    }
    failure
}

/// These observations invalidate the local prepared targets. Only the admitted
/// controller may start fresh account/media preparation within its retry budget.
pub(crate) fn requires_fresh_preparation(error: &anyhow::Error) -> bool {
    matches!(
        describe(error).code.as_str(),
        "publish_observation_session_repaired"
            | "editor_observation_invalidated"
            | "sound_selection_readback_unavailable"
            | "sound_entry_observation_changed"
            | "sound_recent_observation_invalidated"
            | "sound_any_choice_requires_fresh_attempt"
            | SOUND_SHEET_BLIND_FRESH_ATTEMPT
            | SOUND_BUDGET_FRESH_ATTEMPT
            | COMPOSER_OBSERVATION_INVALIDATED
    )
}
/// The sound budget ran out on a blind sheet. Retried only as one fresh attempt per
/// assignment (TikTok relaunched, new helper session); the recovery ledger caps it.
pub const SOUND_SHEET_BLIND_FRESH_ATTEMPT: &str = "sound_sheet_blind_requires_fresh_attempt";
/// The sound budget ran out on a readable sheet, before Post. It shares the blind sheet's one
/// fresh attempt per assignment. Measured 12/10/2026: three of the five phones that ended r91
/// (`d8e672cc`) with it posted in r93 (`4a25d804`) two hours later, so it is not a property of
/// the phone; and 6c950c/993001 ended with it while 1-2 of their 3 retries were unused.
pub const SOUND_BUDGET_FRESH_ATTEMPT: &str = "publish_sound_budget_exhausted";
/// The sound stage's automatic fresh attempts, capped together at one per assignment.
pub(crate) fn is_sound_fresh_attempt(code: &str) -> bool {
    code == SOUND_SHEET_BLIND_FRESH_ATTEMPT || code == SOUND_BUDGET_FRESH_ATTEMPT
}
pub fn classify(message: &str) -> FailureKind {
    let s = message.to_lowercase();
    // A helper whose prior owner the driver could not supersede before media staging (r127
    // phone 35, r129 phone 29, 15-16/10/2026): nothing was copied or posted. Checked before the
    // specific refusals because its chain quotes the helper's own `401 Unauthorized`.
    if helper_recovery_before_media(&s) {
        return FailureKind::Retryable;
    }
    // The storage executor turned a request away before it ran, so nothing reached the phone.
    // r131 (16/10/2026, 30 phones): a controller restart's startup burst failed ten rows'
    // `resolveAppBinding` with it, and the wrappers ("composer ... chưa đo", "không mở được
    // phiên") ended them as unmeasured builds or refused sessions. Checked before the specific
    // refusals because the build wrapper says "chưa đo".
    if s.contains("storagebusy: ") {
        return FailureKind::Retryable;
    }
    // Specific refusals always win over words such as timeout in an error chain.
    if specific_refusal(&s) {
        return FailureKind::Terminal;
    }
    // A lock screen refused the app before anything reached TikTok. Retrying the same phone
    // is safe once someone unlocks it (or the "Cài đặt máy" baseline turns the lock off).
    if crate::device_control::baseline::mentions_screen_locked(message) {
        return FailureKind::Retryable;
    }
    // An overloaded phone ("máy quá tải") was refused at load admission before anything
    // reached TikTok. Retrying is safe once a fresh sample passes after cleanup or a reboot.
    if crate::device_control::load::mentions_device_overloaded(message) {
        return FailureKind::Retryable;
    }
    if s.contains("device offline")
        || s.contains("no devices/emulators found")
        || (s.contains("adb") && s.contains("device") && s.contains("not found"))
    {
        return FailureKind::Disconnected;
    }
    if unavailable_android_agent_status(message) {
        return FailureKind::Retryable;
    }
    // Windows ADB reports WSAETIMEDOUT without the words "timeout" or "timed
    // out". Match the measured loopback daemon failure, after specific terminal
    // refusals; the existing intent/revision gates still own permission to retry.
    if adb_daemon_connect_timeout(message) || adb_transfer_daemon_start_failed(message) {
        return FailureKind::Retryable;
    }
    // After the refusals and the Disconnected check, so "unauthorized" stays terminal and
    // "device … not found" keeps the reconnect window.
    if foreground_read_failed_in_transport(&s) {
        return FailureKind::Retryable;
    }
    if caption_field_detached(&s) {
        return FailureKind::Retryable;
    }
    if [
        "timeout",
        "timed out",
        "connection reset",
        "broken pipe",
        "error: closed",
        "network",
        "mạng không ổn định",
        "tải nhạc",
        "sound pool did not stabilize",
        "sound rows did not finish",
        "selected sound was not confirmed",
        "read unavailable",
        "cannot read the accessibility",
        "chưa chạy lại",
        "composer state lost",
        "composer không mở",
        "không thấy nút đăng ở màn cuối",
        "bước chỉnh sửa không mở",
        "màn đăng không mở",
        "không thấy ô caption ở màn cuối",
        "chưa xác nhận tải nhạc",
        // Picker selection not read back after a tap (r116, 14/10/2026: 3/10 phones once the
        // read grace let the last read finish): nothing was posted and a fresh attempt
        // rebuilds the picker. Its sentence once said "đã dừng", which read as a Stop.
        "chưa xác nhận chọn đủ ảnh",
        // Composer screens that did not show in time (r125, 15/10/2026: phone 49's camera
        // shutter on a slow camera screen ended the row on its first attempt). All are before
        // Post and a fresh attempt restarts the composer; "chưa đo nút Đăng", "số ảnh cần nhiều
        // hơn số ô lưới" and a Stop stay terminal.
        "không thấy nút chụp",
        "picker không mở",
        "không thấy album của chiến dịch",
        // The picker's album list never showed the campaign album within its search window
        // (r127 phone 31, 15/10/2026: legacy_terminal on attempt 1). Only the album list was
        // read and scrolled, before any selection or Post.
        "album_not_visible: chưa thấy album",
        "picker không chuyển sang đúng album",
        "không thấy hàng tab của picker",
        "nút tiếp không sáng",
        "không mọc nút tiếp",
        "gõ caption xong mà đọc lại không thấy",
        // The session was replaced while the pre-Post account proof read the Profile (r131
        // phone 37, 16/10/2026: the row ended `account_unreadable`). Nothing was posted and the
        // next attempt proves the account from nothing on a fresh session.
        "account_session_changed",
    ]
    .iter()
    .any(|x| s.contains(x))
    {
        return FailureKind::Retryable;
    }
    FailureKind::Terminal
}

/// A word that names a deliberate refusal; `lowercase` is the message already lowercased.
/// `stagePublishMedia` refused because the helper still holds an unreconciled prior owner. The
/// driver already tried its once-per-boot supersession; a later attempt finds it settled.
fn helper_recovery_before_media(lowercase: &str) -> bool {
    lowercase.contains("stagepublishmedia failed for device ")
        && lowercase.contains("chưa đăng bài")
        && lowercase.contains("helperrecoveryrequired: prior owner cleanup unresolved")
}

fn specific_refusal(lowercase: &str) -> bool {
    [
        "unauthorized",
        "account mismatch",
        "tài khoản đã đổi",
        "sai tài khoản",
        "unmeasured",
        "chưa hỗ trợ",
        "chưa đo",
        "còn máy khác điều khiển",
        "không khớp",
        "source changed",
        "nội dung đã thay đổi",
        "permission denied",
        "insufficient",
        "hash mismatch",
        "caption changed",
        "caption differs",
        "identity changed",
        "ambiguous",
        "không phân biệt",
        "rehearsal",
        "đã dừng",
        "đã tạm dừng",
        "stopped",
        "cancelled",
    ]
    .iter()
    .any(|x| lowercase.contains(x))
}

pub trait RecoveryJournal: Send + Sync {
    fn step(&self, step: &str, checkpoint: Option<&str>) -> anyhow::Result<()>;
    fn retry(&self, failure: &RecoveryFailure) -> anyhow::Result<Option<Duration>>;
    /// Diagnostics cannot alter a checkpoint or authorize retry of an effect.
    fn note_read(&self, _note: &ReadRecoveryNote) -> anyhow::Result<()> {
        Ok(())
    }
    fn sound(
        &self,
        selection: Option<&crate::SoundSelectionEvidence>,
    ) -> anyhow::Result<Option<crate::SoundSelectionEvidence>>;
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadRecoveryNote {
    pub stage: String,
    pub strategy: String,
    pub attempt: u32,
    pub outcome: String,
    pub error: Option<String>,
}

/// Record a strategy change in the existing per-device timeline, without turning a
/// telemetry write failure into permission to repeat a successfully dispatched Post.
pub fn note_read(stage: &str, strategy: &str, attempt: u32, outcome: &str, error: Option<&str>) {
    let note = ReadRecoveryNote {
        stage: stage.into(),
        strategy: strategy.into(),
        attempt,
        outcome: outcome.into(),
        error: error.map(str::to_owned),
    };
    if let Err(error) = JOURNAL
        .try_with(|journal| journal.note_read(&note))
        .unwrap_or(Ok(()))
    {
        tracing::warn!(
            stage,
            strategy,
            "read recovery telemetry incomplete: {error}"
        );
    }
}
tokio::task_local! { static JOURNAL: Arc<dyn RecoveryJournal>; }
pub fn active() -> bool {
    JOURNAL.try_with(|_| true).unwrap_or(false)
}
pub async fn scope<T>(
    journal: Arc<dyn RecoveryJournal>,
    work: impl std::future::Future<Output = T>,
) -> T {
    JOURNAL.scope(journal, work).await
}
pub fn step(name: &str, checkpoint: Option<&str>) -> anyhow::Result<()> {
    JOURNAL
        .try_with(|j| j.step(name, checkpoint))
        .unwrap_or(Ok(()))
}
pub fn bind_sound(
    selection: &crate::SoundSelectionEvidence,
) -> anyhow::Result<crate::SoundSelectionEvidence> {
    Ok(JOURNAL
        .try_with(|j| j.sound(Some(selection)))
        .unwrap_or(Ok(None))?
        .unwrap_or_else(|| selection.clone()))
}

pub fn stored_sound() -> anyhow::Result<Option<crate::SoundSelectionEvidence>> {
    JOURNAL
        .try_with(|journal| journal.sound(None))
        .unwrap_or(Ok(None))
}

pub async fn retry(error: &anyhow::Error, stop: &AtomicBool) -> anyhow::Result<bool> {
    retry_inner(error, stop, None).await
}

/// Pre-effect retry whose backoff consumes, rather than resets, the phase deadline.
pub async fn retry_until(
    error: &anyhow::Error,
    stop: &AtomicBool,
    deadline: tokio::time::Instant,
) -> anyhow::Result<bool> {
    retry_inner(error, stop, Some(deadline)).await
}

async fn retry_inner(
    error: &anyhow::Error,
    stop: &AtomicBool,
    deadline: Option<tokio::time::Instant>,
) -> anyhow::Result<bool> {
    let failure = describe(error);
    if stop.load(Ordering::Acquire)
        || failure.kind != FailureKind::Retryable
        // This repair needs fresh admitted account/session proof, not a local loop.
        || requires_fresh_preparation(error)
    {
        return Ok(false);
    }
    if deadline.is_some_and(|end| tokio::time::Instant::now() >= end) {
        return Err(observation_deadline());
    }
    let delay = JOURNAL
        .try_with(|j| j.retry(&failure))
        .unwrap_or(Ok(None))?;
    let Some(delay) = delay else { return Ok(false) };
    let retry_at = tokio::time::Instant::now() + delay;
    let until = deadline.map_or(retry_at, |end| retry_at.min(end));
    while tokio::time::Instant::now() < until {
        anyhow::ensure!(!stop.load(Ordering::Acquire), "Đã dừng; không thử lại");
        tokio::time::sleep(
            Duration::from_millis(100)
                .min(until.saturating_duration_since(tokio::time::Instant::now())),
        )
        .await;
    }
    anyhow::ensure!(!stop.load(Ordering::Acquire), "Đã dừng; không thử lại");
    if deadline.is_some_and(|end| tokio::time::Instant::now() >= end) {
        return Err(observation_deadline());
    }
    JOURNAL.try_with(|j| j.step("", None)).unwrap_or(Ok(()))?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn absent_transport_is_distinct_from_retryable_and_terminal_errors() {
        assert_eq!(classify("adb -s ce031713840038030c shell getprop service.adb.tcp.port failed: adb.exe: device 'ce031713840038030c' not found"),FailureKind::Disconnected);
        assert_eq!(classify("adb: connection reset"), FailureKind::Retryable);
        let daemon_connect = "stagePublishMedia failed for device ce11171be88ba30d01: query MediaStore for the campaign's rows: adb -s ce11171be88ba30d01 shell content query --uri content://media/external/video/media --projection _id:_data 2>&1 failed: * daemon still not running\r\nadb.exe: cannot connect to daemon at tcp:5037: cannot connect to 127.0.0.1:5037: A connection attempt failed because the connected party did not properly respond after a period of time, or established connection failed because connected host has failed to respond. (10060)";
        let daemon_failure = RecoveryFailure::legacy(daemon_connect);
        assert_eq!(daemon_failure.kind, FailureKind::Retryable);
        assert_eq!(daemon_failure.code, "adb_daemon_connect_timeout");
        // The shell-output readback wrapper reports the same measured WSA error
        // with `error:` instead of `adb.exe:`. Both must reach bounded recovery.
        let readback_connect = daemon_connect.replace("adb.exe:", "error:");
        let readback_failure = RecoveryFailure::legacy(&readback_connect);
        assert_eq!(readback_failure.kind, FailureKind::Retryable);
        assert_eq!(readback_failure.code, "adb_daemon_connect_timeout");
        let daemon_start = "stagePublishMedia failed for device fixture: push fixture.png to /sdcard/Pictures/.riviu-publish/fixture/fixture.png: adb -s fixture push fixture.png /sdcard/Pictures/.riviu-publish/fixture/fixture.png failed: * daemon not running; starting now at tcp:5037\r\ncould not read ok from ADB Server\r\n* failed to start daemon\r\nadb: error: failed to get feature set: cannot connect to daemon";
        let start_failure = RecoveryFailure::legacy(daemon_start);
        assert_eq!(start_failure.kind, FailureKind::Retryable);
        assert_eq!(start_failure.code, "adb_daemon_start_failed");
        // r125 phone 32: the same loss during the read-only photo-order proof.
        let order = "ce031713a8f5bd0805: chưa xác minh được thứ tự ảnh trước Đăng (ensurePublishPhotoOrder failed for device ce031713a8f5bd0805: reprove imported photo bytes: publish transfer operation=sha256sum path=/storage/emulated/0/Pictures/x/06-image.png reason=exitError exit=1 stdout= stderr=* daemon not running; starting now at tcp:5037\r\ncould not read ok from ADB Server\r\n* failed to start daemon\r\nadb.exe: cannot connect to daemon)";
        assert_eq!(
            RecoveryFailure::legacy(order).code,
            "adb_daemon_start_failed"
        );
        // r125 phone 49: the camera shutter did not show in time; nothing was posted.
        assert_eq!(
            classify("ce071827715f321b05: không thấy nút chụp — không có mốc để tìm ô thư viện"),
            FailureKind::Retryable
        );
        // r127 phone 31: the campaign album never showed in the picker's album list.
        assert_eq!(
            classify(
                "album_not_visible: chưa thấy album riviu-abc sau 3 lần cuộn; 2 lần đọc trống"
            ),
            FailureKind::Retryable
        );
        assert_eq!(
            classify("chưa đo nút Đăng trên bản build này — không bấm gì cả"),
            FailureKind::Terminal
        );
        // r129 phone 29: the helper's prior owner was still unreconciled; nothing was posted.
        assert_eq!(
            classify("stagePublishMedia failed for device ce081718512d0cd80b: Không chuẩn bị được Riviu Helper trên máy này; chưa chuyển ảnh/video, chưa đăng bài: helper owner resume unresolved (helper /v1/session/status HTTP 401 Unauthorized): HelperRecoveryRequired: prior owner cleanup unresolved; no claim or action replay"),
            FailureKind::Retryable
        );
        // r123 phone 49: the caption field was re-rendered under set_text.
        assert_eq!(
            classify("agent /element/00000000-0000-0c55-ffff-ffff00000dec/value lỗi 404 Not Found: The referenced element is no longer attached to the DOM"),
            FailureKind::Retryable
        );
        // r117 phone 49: the same daemon failure while the publish session opened.
        let session_open = anyhow::anyhow!("cleanAppSession failed for device ce071827715f321b05: startStreamAfterSession failed for device ce071827715f321b05: adb -s ce071827715f321b05 shell wm size failed: * daemon not running; starting now at tcp:5037\r\ncould not read ok from ADB Server\r\n* failed to start daemon\r\nadb.exe: cannot connect to daemon");
        let open_failure = session_open_adb_daemon_failure(&session_open).unwrap();
        assert_eq!(open_failure.kind, FailureKind::Retryable);
        assert_eq!(open_failure.code, "adb_daemon_start_failed");
        // r124 phone 37: a navigation gesture Android cancelled before Post.
        let cancelled = anyhow::Error::new(crate::driver::GestureCancelled {
            message: "agent /actions lỗi 400 Bad Request: Unable to perform W3C actions.".into(),
        })
        .context("navigate own profile");
        assert_eq!(describe(&cancelled).kind, FailureKind::Retryable);
        assert_eq!(describe(&cancelled).code, "gesture_cancelled");
        assert!(session_open_adb_daemon_failure(&anyhow::anyhow!(
            "device unauthorized: {session_open}"
        ))
        .is_none());
        for refusal in [
            format!("device unauthorized: {daemon_connect}"),
            daemon_connect.replace("127.0.0.1:5037", "192.0.2.1:5037"),
            daemon_connect.replace("(10060)", "(10013)"),
            readback_connect.replace("(10060)", "(10013)"),
            readback_connect.replace("127.0.0.1:5037", "192.0.2.1:5037"),
            format!("device unauthorized: {daemon_start}"),
            daemon_start.replace("tcp:5037", "tcp:5038"),
            daemon_start.replace("stagePublishMedia failed for device", "tap failed for device"),
        ] {
            assert_eq!(classify(&refusal), FailureKind::Terminal, "{refusal}");
        }
        assert_eq!(
            classify("TikTok báo mạng không ổn định khi tải danh sách nhạc"),
            FailureKind::Retryable
        );
        assert_eq!(
            classify("chưa xác nhận chọn đủ ảnh trong thư mục; chưa bấm Đăng"),
            FailureKind::Retryable
        );
        for msg in [
            "device unauthorized",
            "account mismatch after timeout",
            "sound identity changed",
            "unknown error",
            "rehearsal verified before public Post",
            "Đã tạm dừng trong lúc chờ TikTok tải nhạc",
        ] {
            assert_eq!(classify(msg), FailureKind::Terminal, "{msg}");
        }
    }

    #[test]
    fn media_staging_failures_retry_unless_refused() {
        // r112 phone 32, 14/10/2026: the push landed, the copy response did not.
        let push = anyhow::anyhow!("adb: error: failed to read copy response: EOF")
            .context("stagePublishMedia failed for device ce031713a8f5bd0805: push 01-cover.png");
        assert_eq!(describe(&push).kind, FailureKind::Terminal);
        let failure = media_transfer_failure(&push);
        assert_eq!(failure.kind, FailureKind::Retryable);
        assert_eq!(failure.code, "media_transfer_failed");
        // Typed and transport answers keep their own kind and code.
        let offline = anyhow::anyhow!("adb.exe: device 'x' not found");
        assert_eq!(
            media_transfer_failure(&offline).kind,
            FailureKind::Disconnected
        );
        let typed = retryable_error("adb_daemon_connect_timeout", "typed");
        assert_eq!(
            media_transfer_failure(&typed).code,
            "adb_daemon_connect_timeout"
        );
        for refusal in [
            "device unauthorized",
            "media hash mismatch",
            "publish cancelled",
        ] {
            let error = anyhow::anyhow!("{refusal}").context("stagePublishMedia failed");
            assert_eq!(
                media_transfer_failure(&error).kind,
                FailureKind::Terminal,
                "{refusal}"
            );
        }
    }

    #[test]
    fn typed_failure_wins_over_message_wording_and_legacy_remains_readable() {
        let error = retryable_error(
            "caption_navigation_timeout",
            "wording may change without changing retry policy",
        );
        let failure = describe(&error);
        assert_eq!(failure.code, "caption_navigation_timeout");
        assert_eq!(failure.kind, FailureKind::Retryable);
        assert_eq!(
            RecoveryFailure::legacy("adb: connection reset").kind,
            FailureKind::Retryable
        );
        assert_eq!(
            RecoveryFailure::legacy("account mismatch after timeout").kind,
            FailureKind::Terminal
        );
    }

    #[test]
    fn a_lock_screen_refusal_is_a_typed_retryable_pre_post_reason() {
        // 2026-10-08: publish failed its foreground proof with focus=StatusBar and
        // mDreamingLockscreen=true, and settled as the generic `legacy_terminal`.
        let message = "cleanAppSession failed for device ce0717171c2a64d50d: \
            startInteractionSession failed for device ce0717171c2a64d50d: ce0717171c2a64d50d: \
            máy đang khóa màn hình (StatusBar) nên com.ss.android.ugc.trill không lên \
            foreground được trong 40s; mở khóa máy (hoặc áp dụng Cài đặt máy) rồi chạy lại";
        let failure = RecoveryFailure::legacy(message);
        assert_eq!(failure.code, "device_screen_locked");
        assert_eq!(failure.kind, FailureKind::Retryable);
        assert_eq!(describe(&anyhow::anyhow!(message.to_string())).code, "device_screen_locked");
        // An operator stop still wins: a stopped run is not retried because a phone was locked.
        assert_eq!(
            classify(&format!("đã dừng; {message}")),
            FailureKind::Terminal
        );
    }

    #[test]
    fn blind_android_agent_waits_for_its_reported_cooldown() {
        let message = "cleanAppSession failed for device 9889db374744474635: startInteractionSession failed for device 9889db374744474635: the agent on 9889db374744474635 is listening but cannot read the accessibility tree, and its instrumentation was already restarted 52s ago without fixing it. Không thấy instrumentation khác đang hoạt động; cây UI của app có thể chưa phản hồi. Not restarting again for another 12s.";
        let failure = RecoveryFailure::legacy(message);
        assert_eq!(failure.kind, FailureKind::Retryable);
        assert_eq!(failure.code, "agent_accessibility_unavailable");
        assert_eq!(failure.minimum_retry_delay(), Some(Duration::from_secs(13)));
        let first = RecoveryFailure::legacy("the agent on 9889db374744474635 is listening but cannot read the accessibility tree even after a restart");
        assert_eq!(first.minimum_retry_delay(), Some(Duration::from_secs(65)));
        assert_eq!(
            RecoveryFailure::legacy("adb: connection reset").minimum_retry_delay(),
            None
        );
    }

    #[test]
    fn agent_status_timeout_waits_before_retrying_the_session() {
        let failure = RecoveryFailure::legacy(
            "openControlSession: the agent on 9889db374744474635 did not answer /status within 10 seconds",
        );
        assert_eq!(failure.kind, FailureKind::Retryable);
        assert_eq!(failure.code, "agent_status_unavailable");
        assert_eq!(failure.minimum_retry_delay(), Some(Duration::from_secs(65)));
    }
}

#[cfg(test)]
mod publish_read_recovery_tests {
    use super::*;
    use crate::driver::{UiError, UiErrorKind};

    #[test]
    fn observation_deadline_is_typed_without_turning_action_timeouts_into_retry() {
        let read = anyhow::Error::new(UiError::new(
            UiErrorKind::Timeout,
            "observe",
            "budget elapsed",
        ))
        .context("caption predicate");
        assert_eq!(describe(&read).code, "publish_observation_deadline");
        assert_eq!(describe(&read).kind, FailureKind::Retryable);
        for operation in ["tap", "activate_element", "typeText"] {
            let action = anyhow::Error::new(UiError::new(
                UiErrorKind::Timeout,
                operation,
                "budget elapsed",
            ));
            assert_eq!(
                describe(&action).kind,
                FailureKind::Terminal,
                "uncertain action must not become replayable: {operation}"
            );
        }
        assert_eq!(
            describe(&anyhow::anyhow!("observation_session_changed")).kind,
            FailureKind::Terminal,
            "unproven epoch changes must not get the driver-repair retry policy"
        );
    }

    #[derive(Default)]
    struct Journal {
        retries: std::sync::atomic::AtomicUsize,
        resumed: std::sync::atomic::AtomicUsize,
    }

    impl RecoveryJournal for Journal {
        fn step(&self, _: &str, _: Option<&str>) -> anyhow::Result<()> {
            self.resumed.fetch_add(1, Ordering::Relaxed);
            Ok(())
        }
        fn retry(&self, _: &RecoveryFailure) -> anyhow::Result<Option<Duration>> {
            self.retries.fetch_add(1, Ordering::Relaxed);
            Ok(Some(Duration::from_secs(10)))
        }
        fn sound(
            &self,
            _: Option<&crate::SoundSelectionEvidence>,
        ) -> anyhow::Result<Option<crate::SoundSelectionEvidence>> {
            Ok(None)
        }
    }

    #[tokio::test(start_paused = true)]
    async fn phase_deadline_caps_backoff_and_expired_phase_never_requests_retry() {
        let journal = Arc::new(Journal::default());
        let error = retryable_error("fixture_read", "transient read");
        let stop = AtomicBool::new(false);
        let started = tokio::time::Instant::now();
        scope(journal.clone(), async {
            let failure = retry_until(&error, &stop, started).await.unwrap_err();
            assert_eq!(describe(&failure).code, "publish_observation_deadline");
            assert_eq!(journal.retries.load(Ordering::Relaxed), 0);
            let failure = retry_until(&error, &stop, started + Duration::from_millis(200))
                .await
                .unwrap_err();
            assert_eq!(describe(&failure).code, "publish_observation_deadline");
        })
        .await;
        assert_eq!(journal.retries.load(Ordering::Relaxed), 1);
        assert_eq!(journal.resumed.load(Ordering::Relaxed), 0);
        assert_eq!(
            tokio::time::Instant::now() - started,
            Duration::from_millis(200)
        );
    }

    #[tokio::test(start_paused = true)]
    async fn repaired_session_does_not_spend_a_local_retry_or_resume_old_phase() {
        let journal = Arc::new(Journal::default());
        let stop = AtomicBool::new(false);
        let error = RecoveryFailure::new(
            "publish_observation_session_repaired",
            FailureKind::Retryable,
            "driver repaired observation",
        )
        .into();
        scope(journal.clone(), async {
            assert!(!retry(&error, &stop).await.unwrap());
            assert!(!retry_until(
                &error,
                &stop,
                tokio::time::Instant::now() + Duration::from_secs(1)
            )
            .await
            .unwrap());
        })
        .await;
        assert_eq!(journal.retries.load(Ordering::Relaxed), 0);
        assert_eq!(journal.resumed.load(Ordering::Relaxed), 0);
    }
}
