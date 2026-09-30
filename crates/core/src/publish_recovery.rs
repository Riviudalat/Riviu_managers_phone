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
}
impl Default for PublishRecoveryState {
    fn default() -> Self {
        Self {
            step: "device".into(),
            checkpoint: "approved".into(),
            state: "running".into(),
            retries_used: 0,
            max_retries: 3,
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
        }
    }
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
            FailureKind::Retryable if unavailable_android_agent_status(&message) => {
                "agent_status_unavailable"
            }
            FailureKind::Retryable if blind_android_agent(&message) => {
                "agent_accessibility_unavailable"
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

/// A read phase exhausted its existing budget; this never describes a gesture.
pub(crate) fn observation_deadline() -> anyhow::Error {
    crate::driver::UiError::new(
        crate::driver::UiErrorKind::Timeout,
        "observe",
        "publish observation deadline exceeded",
    )
    .into()
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
    if error.is::<crate::tiktok_sound::SoundNetworkUnavailable>() {
        return RecoveryFailure::new(
            "sound_network_unavailable",
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
    RecoveryFailure::legacy(format!("{error:#}"))
}
pub fn classify(message: &str) -> FailureKind {
    let s = message.to_lowercase();
    // Specific refusals always win over words such as timeout in an error chain.
    if [
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
    .any(|x| s.contains(x))
    {
        return FailureKind::Terminal;
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
    ]
    .iter()
    .any(|x| s.contains(x))
    {
        return FailureKind::Retryable;
    }
    FailureKind::Terminal
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
        || failure.code == "publish_observation_session_repaired"
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
        assert_eq!(
            classify("TikTok báo mạng không ổn định khi tải danh sách nhạc"),
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
