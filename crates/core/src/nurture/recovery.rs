//! Session outcome and the budgeted recovery ladder.
//!
//! The rule that matters: only a transport-class failure may touch the
//! transport. A rejected command means the agent is alive and just refused that
//! one call — recycling on it is what produced the 2–3 minute death spirals in
//! earlier live tests.

use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::device_control::UiWithStreamContext;
use crate::driver::{ui_error_kind, UiErrorKind, UiSession};
use crate::screen_watch::SessionHandle;
use crate::types::{InteractionSessionKind, NurtureSessionStatus};

use super::{NurtureEngine, TextCommentHealth};

/// Soft recovery budget: drop the session and make a new one.
pub(super) const SOFT_RECOVERY_BUDGET: Duration = Duration::from_secs(15);
/// Hard recovery budget: tear down relay + device runner and rebuild. Killing
/// the device-side runner and waiting for iOS to reap it before a fresh XCTest
/// start measured ~70–90 s, so a tighter budget would abort a recovery that was
/// about to succeed.
pub(super) const HARD_RECOVERY_BUDGET: Duration = Duration::from_secs(150);
/// Hard recycles allowed per session before the device is declared failed.
pub(super) const MAX_HARD_RECOVERIES: u32 = 1;

fn should_hard_recycle(kind: UiErrorKind) -> bool {
    kind == UiErrorKind::Transport
}

/// How a session ended. `done` is reserved for a session that actually did the
/// work: a run that timed out having processed nothing is `failed`, not `done`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Done,
    Partial,
    Failed,
    Stopped,
}

impl Outcome {
    pub fn as_str(&self) -> &'static str {
        match self {
            Outcome::Done => "done",
            Outcome::Partial => "partial",
            Outcome::Failed => "failed",
            Outcome::Stopped => "stopped",
        }
    }
}

/// Recovery spend for one session.
pub(super) struct Budget {
    pub(super) soft: u32,
    pub(super) hard: u32,
    pub(super) spent: Duration,
}

impl Budget {
    pub(super) fn new() -> Self {
        Self {
            soft: 0,
            hard: 0,
            spent: Duration::ZERO,
        }
    }

    pub(super) fn exhausted(&self) -> bool {
        self.hard > MAX_HARD_RECOVERIES
    }
}

impl NurtureEngine {
    /// Spend recovery budget on a failed gesture. Returns false when the
    /// session cannot continue.
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn recover(
        &self,
        _udid: &str,
        bundle_id: &str,
        fresh_text_session: bool,
        context: &mut UiWithStreamContext,
        session: &mut Arc<dyn UiSession>,
        handle: &SessionHandle,
        budget: &mut Budget,
        text_health: &mut TextCommentHealth,
        err: &anyhow::Error,
        status: &mut NurtureSessionStatus,
        on_status: &(impl Fn(NurtureSessionStatus) + Send + Sync),
    ) -> bool {
        let kind = ui_error_kind(err);
        // A rejected command means the runner is alive and only this command
        // was wrong. Recycling on that is what produced the old death spirals.
        if matches!(kind, UiErrorKind::Http | UiErrorKind::Other) {
            return true;
        }

        let started = Instant::now();
        budget.soft += 1;
        status.last_message = format!(
            "soft recovery {}/{} ({}s ngân sách)",
            budget.soft,
            budget.soft,
            SOFT_RECOVERY_BUDGET.as_secs()
        );
        on_status(status.clone());
        let session_kind = if fresh_text_session {
            InteractionSessionKind::FreshText
        } else {
            InteractionSessionKind::Ordinary
        };
        let soft = self
            .control
            .recover_streaming_session(context, bundle_id, session_kind, false)
            .await;
        let soft_failure = match soft {
            Ok(s) => {
                let stream: anyhow::Result<()> = Ok(());
                match stream {
                    Ok(()) => {
                        let recovered: Arc<dyn UiSession> = s;
                        *session = recovered.clone();
                        handle.set(recovered);
                        if fresh_text_session {
                            text_health.fresh_session_installed();
                        }
                        budget.spent += started.elapsed();
                        status.last_message = format!(
                            "soft recovery xong sau {:.0}s",
                            started.elapsed().as_secs_f64()
                        );
                        on_status(status.clone());
                        return true;
                    }
                    Err(e) => format!("không mở lại được stream: {e}"),
                }
            }
            Err(e) => e.to_string(),
        };
        budget.spent += started.elapsed();

        if !should_hard_recycle(kind) {
            status.last_message = format!(
                "soft recovery thất bại ({soft_failure}); giữ nguyên transport vì lỗi không thuộc transport"
            );
            on_status(status.clone());
            return false;
        }

        if budget.hard >= MAX_HARD_RECOVERIES {
            status.last_message = "đã dùng hết hard recovery — dừng thiết bị".into();
            on_status(status.clone());
            budget.hard += 1;
            return false;
        }

        let hard_started = Instant::now();
        budget.hard += 1;
        status.last_message = format!(
            "hard recovery 1/{} ({}s ngân sách)",
            MAX_HARD_RECOVERIES,
            HARD_RECOVERY_BUDGET.as_secs()
        );
        on_status(status.clone());
        let hard = self
            .control
            .recover_streaming_session(context, bundle_id, session_kind, true)
            .await;
        budget.spent += hard_started.elapsed();
        match hard {
            Ok(s) => {
                let recovered: Arc<dyn UiSession> = s;
                *session = recovered.clone();
                handle.set(recovered);
                if fresh_text_session {
                    text_health.fresh_session_installed();
                }
                status.last_message = format!(
                    "hard recovery xong sau {:.0}s",
                    hard_started.elapsed().as_secs_f64()
                );
                on_status(status.clone());
                true
            }
            Err(_) => {
                status.last_message = "hard recovery thất bại — thiết bị lỗi".into();
                on_status(status.clone());
                false
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    use async_trait::async_trait;

    use crate::db::Database;
    use crate::driver::{DeviceDriver, UiError};
    use crate::frame_source::NullFrameSource;
    use crate::types::{
        DeviceInfo, InteractionSessionKind, StreamHandoffProof, StreamStartProof, SwipeGesture,
        TapPoint,
    };
    use crate::StreamStopProof;

    struct RecoverySession;

    #[async_trait]
    impl UiSession for RecoverySession {
        async fn tap(&self, _point: TapPoint) -> anyhow::Result<()> {
            Ok(())
        }

        async fn swipe(&self, _gesture: SwipeGesture) -> anyhow::Result<()> {
            Ok(())
        }

        async fn type_text(&self, _text: &str) -> anyhow::Result<()> {
            Ok(())
        }

        async fn home(&self) -> anyhow::Result<()> {
            Ok(())
        }

        async fn find_and_tap(&self, _accessibility_id: &str) -> anyhow::Result<()> {
            Ok(())
        }

        async fn assert_visible(&self, _accessibility_id: &str) -> anyhow::Result<()> {
            Ok(())
        }

        fn stream_url(&self) -> Option<String> {
            None
        }
    }

    #[derive(Default)]
    struct RecoveryDriver {
        ordinary: AtomicUsize,
        fresh: AtomicUsize,
        streams: AtomicUsize,
        generation: AtomicUsize,
        fail_next_fresh: AtomicBool,
        events: std::sync::Mutex<Vec<&'static str>>,
        fresh_bundles: std::sync::Mutex<Vec<String>>,
    }

    #[async_trait]
    impl DeviceDriver for RecoveryDriver {
        async fn stop_owned_stream(&self, _udid: &str) -> anyhow::Result<StreamStopProof> {
            let old_generation = self.generation.fetch_add(1, Ordering::Relaxed) as u64;
            self.events.lock().expect("events").push("stop");
            Ok(StreamStopProof {
                old_generation,
                new_generation: old_generation + 1,
                child_stopped: true,
            })
        }

        async fn confirm_interaction_stream_stopped(
            &self,
            _udid: &str,
        ) -> anyhow::Result<StreamHandoffProof> {
            Ok(StreamHandoffProof {
                generation: self.generation.load(Ordering::Relaxed) as u64,
            })
        }

        async fn start_interaction_session(
            &self,
            udid: &str,
            bundle_id: &str,
            kind: InteractionSessionKind,
        ) -> anyhow::Result<Box<dyn UiSession>> {
            match kind {
                InteractionSessionKind::Ordinary => self.start_ui_session(udid).await,
                InteractionSessionKind::FreshText => {
                    self.start_fresh_text_session(udid, bundle_id).await
                }
            }
        }

        async fn start_stream_after_session(
            &self,
            _udid: &str,
        ) -> anyhow::Result<StreamStartProof> {
            self.streams.fetch_add(1, Ordering::Relaxed);
            self.events.lock().expect("events").push("stream");
            Ok(StreamStartProof {
                generation: self.generation.load(Ordering::Relaxed) as u64,
                first_frame_observed: true,
                stream_url: "fixture://stream".to_string(),
            })
        }

        async fn list_devices(&self) -> anyhow::Result<Vec<DeviceInfo>> {
            Ok(Vec::new())
        }

        async fn refresh_device(&self, _udid: &str) -> anyhow::Result<DeviceInfo> {
            anyhow::bail!("unused")
        }

        async fn install_app(&self, _udid: &str, _path: &Path) -> anyhow::Result<()> {
            Ok(())
        }

        async fn uninstall_app(&self, _udid: &str, _bundle_id: &str) -> anyhow::Result<()> {
            Ok(())
        }

        async fn screenshot(&self, _udid: &str, _dest: &Path) -> anyhow::Result<PathBuf> {
            anyhow::bail!("unused")
        }

        async fn syslog_tail(&self, _udid: &str, _lines: usize) -> anyhow::Result<String> {
            Ok(String::new())
        }

        async fn launch_app(&self, _udid: &str, _bundle_id: &str) -> anyhow::Result<()> {
            Ok(())
        }

        async fn terminate_app(
            &self,
            _udid: &str,
            bundle_id: &str,
        ) -> anyhow::Result<crate::ProcessAbsenceProof> {
            Ok(crate::ProcessAbsenceProof {
                bundle_id: bundle_id.to_string(),
                old_pid: None,
            })
        }

        async fn reboot(&self, _udid: &str) -> anyhow::Result<()> {
            Ok(())
        }

        async fn start_ui_session(&self, _udid: &str) -> anyhow::Result<Box<dyn UiSession>> {
            self.ordinary.fetch_add(1, Ordering::Relaxed);
            Ok(Box::new(RecoverySession))
        }

        async fn start_fresh_text_session(
            &self,
            _udid: &str,
            bundle_id: &str,
        ) -> anyhow::Result<Box<dyn UiSession>> {
            self.fresh.fetch_add(1, Ordering::Relaxed);
            self.events.lock().expect("events").push("fresh");
            if self.fail_next_fresh.swap(false, Ordering::Relaxed) {
                anyhow::bail!("fixture fresh session failure")
            }
            self.fresh_bundles
                .lock()
                .expect("fresh bundles")
                .push(bundle_id.to_string());
            Ok(Box::new(RecoverySession))
        }

        async fn ensure_stream(&self, _udid: &str) -> anyhow::Result<String> {
            self.streams.fetch_add(1, Ordering::Relaxed);
            self.events.lock().expect("events").push("stream");
            Ok(String::new())
        }

        async fn recycle_ui_transport(&self, _udid: &str) {
            self.events.lock().expect("events").push("recycle");
        }

        async fn prepare_device(&self, _udid: &str) -> anyhow::Result<()> {
            Ok(())
        }
    }

    fn recovery_control(driver: Arc<RecoveryDriver>) -> Arc<crate::DeviceControlPlane> {
        Arc::new(crate::DeviceControlPlane::new(
            driver,
            Arc::new(crate::DeviceWorkCoordinator::new()),
            Arc::new(crate::StreamBudgetManager::default()),
        ))
    }

    async fn recovery_context(
        control: &crate::DeviceControlPlane,
        udid: &str,
    ) -> UiWithStreamContext {
        let exclusive = control
            .try_acquire_exclusive(udid, crate::DeviceWorkOwner::Nurture)
            .await
            .expect("recovery fixture lease");
        let (exclusive, capacity) = control
            .reserve_ui_capacity(exclusive)
            .await
            .expect("recovery fixture capacity");
        let session = control
            .start_interaction_session(
                exclusive,
                "com.ss.iphone.ugc.Ame",
                InteractionSessionKind::FreshText,
            )
            .await
            .expect("recovery fixture session");
        control
            .start_reserved_stream(session, capacity)
            .await
            .expect("recovery fixture stream")
    }

    #[test]
    fn soft_recoveries_alone_never_stop_a_device() {
        let mut b = Budget::new();
        assert!(!b.exhausted());
        b.soft = 5;
        assert!(
            !b.exhausted(),
            "a device must not be given up on for soft recoveries — those are cheap"
        );
    }

    #[test]
    fn the_budget_runs_out_one_past_the_hard_recovery_cap() {
        let mut b = Budget::new();
        b.hard = MAX_HARD_RECOVERIES;
        assert!(
            !b.exhausted(),
            "the allowed hard recycle must still be usable"
        );
        b.hard = MAX_HARD_RECOVERIES + 1;
        assert!(b.exhausted());
    }

    /// A rejected command means the agent is alive; recycling on it is the
    /// mistake that produced 2–3 minute stalls in earlier builds.
    #[test]
    fn only_transport_class_failures_justify_touching_the_transport() {
        assert!(should_hard_recycle(UiErrorKind::Transport));
        assert!(!should_hard_recycle(UiErrorKind::Session));
        assert!(!should_hard_recycle(UiErrorKind::Timeout));
        assert!(!should_hard_recycle(UiErrorKind::Http));
        assert!(!should_hard_recycle(UiErrorKind::Other));
    }

    #[tokio::test]
    async fn comment_recovery_replaces_both_feed_and_watcher_with_a_fresh_session() {
        let driver = Arc::new(RecoveryDriver::default());
        let control = recovery_control(driver.clone());
        let db_path =
            std::env::temp_dir().join(format!("riviu-recovery-test-{}.db", uuid::Uuid::new_v4()));
        let engine = NurtureEngine::new(
            Arc::new(Database::open(&db_path).expect("test database")),
            control.clone(),
            Arc::new(NullFrameSource),
            std::env::temp_dir(),
        );
        let mut context = recovery_context(&control, "udid-a").await;
        let original = control
            .streaming_session(&context)
            .expect("fixture session");
        driver.fresh.store(0, Ordering::Relaxed);
        driver.streams.store(0, Ordering::Relaxed);
        driver.events.lock().expect("events").clear();
        driver.fresh_bundles.lock().expect("fresh bundles").clear();
        let mut current = original.clone();
        let handle = SessionHandle::new();
        handle.set(original.clone());
        let mut budget = Budget::new();
        let mut status = NurtureSessionStatus {
            udid: "udid-a".into(),
            running: true,
            videos_done: 0,
            swipe_attempts: 0,
            like_attempts: 0,
            comment_attempts: 0,
            follow_attempts: 0,
            likes: 0,
            comments: 0,
            follows: 0,
            last_message: String::new(),
            session_usd: 0.0,
        };
        let error =
            anyhow::Error::new(UiError::new(UiErrorKind::Session, "tap", "session expired"));
        let mut text_health = super::super::TextCommentHealth::default();
        assert_eq!(
            text_health.observe(super::super::CommentResult::TextNotArmed),
            super::super::CommentRecoveryAction::None
        );
        let settings = crate::types::NurtureSettings {
            bundle_id: "   ".into(),
            ..Default::default()
        };
        let bundle_id = NurtureEngine::tiktok_bundle(&settings);

        assert!(
            engine
                .recover(
                    "udid-a",
                    bundle_id,
                    true,
                    &mut context,
                    &mut current,
                    &handle,
                    &mut budget,
                    &mut text_health,
                    &error,
                    &mut status,
                    &|_| {},
                )
                .await
        );

        let watcher = handle.get().expect("watcher session");
        assert!(!Arc::ptr_eq(&original, &current));
        assert!(Arc::ptr_eq(&current, &watcher));
        assert_eq!(driver.fresh.load(Ordering::Relaxed), 1);
        assert_eq!(driver.ordinary.load(Ordering::Relaxed), 0);
        assert_eq!(driver.streams.load(Ordering::Relaxed), 1);
        assert_eq!(
            driver.events.lock().expect("events").as_slice(),
            ["stop", "fresh", "stream"]
        );
        assert_eq!(
            driver
                .fresh_bundles
                .lock()
                .expect("fresh bundles")
                .as_slice(),
            ["com.ss.iphone.ugc.Ame"]
        );
        assert_eq!(text_health.text_not_armed_streak(), 0);

        control
            .close_ui_context(context)
            .await
            .expect("close recovery fixture");
        drop(engine);
        let _ = std::fs::remove_file(db_path);
    }

    #[tokio::test]
    async fn hard_comment_recovery_rebuilds_fresh_session_before_restarting_stream() {
        let driver = Arc::new(RecoveryDriver::default());
        let control = recovery_control(driver.clone());
        let db_path =
            std::env::temp_dir().join(format!("riviu-hard-recovery-{}.db", uuid::Uuid::new_v4()));
        let engine = NurtureEngine::new(
            Arc::new(Database::open(&db_path).expect("test database")),
            control.clone(),
            Arc::new(NullFrameSource),
            std::env::temp_dir(),
        );
        let mut context = recovery_context(&control, "udid-hard").await;
        let original = control
            .streaming_session(&context)
            .expect("fixture session");
        driver.fresh.store(0, Ordering::Relaxed);
        driver.streams.store(0, Ordering::Relaxed);
        driver.events.lock().expect("events").clear();
        driver.fail_next_fresh.store(true, Ordering::Relaxed);
        let mut current = original.clone();
        let handle = SessionHandle::new();
        handle.set(original.clone());
        let mut budget = Budget::new();
        let mut text_health = super::super::TextCommentHealth::default();
        let mut status = NurtureSessionStatus {
            udid: "udid-hard".into(),
            running: true,
            videos_done: 0,
            swipe_attempts: 0,
            like_attempts: 0,
            comment_attempts: 0,
            follow_attempts: 0,
            likes: 0,
            comments: 0,
            follows: 0,
            last_message: String::new(),
            session_usd: 0.0,
        };
        let error = anyhow::Error::new(UiError::new(UiErrorKind::Transport, "tap", "relay reset"));

        assert!(
            engine
                .recover(
                    "udid-hard",
                    "com.ss.iphone.ugc.Ame",
                    true,
                    &mut context,
                    &mut current,
                    &handle,
                    &mut budget,
                    &mut text_health,
                    &error,
                    &mut status,
                    &|_| {},
                )
                .await
        );

        let watcher = handle.get().expect("watcher session");
        assert!(!Arc::ptr_eq(&original, &current));
        assert!(Arc::ptr_eq(&current, &watcher));
        assert_eq!(budget.hard, 1);
        assert_eq!(driver.streams.load(Ordering::Relaxed), 1);
        assert_eq!(
            driver.events.lock().expect("events").as_slice(),
            ["stop", "fresh", "stop", "recycle", "stop", "fresh", "stream"]
        );

        control
            .close_ui_context(context)
            .await
            .expect("close recovery fixture");
        drop(engine);
        let _ = std::fs::remove_file(db_path);
    }
}
