//! Cooperative Stop at each verifier primitive, including nested account/Copy helpers.
//! An in-flight driver primitive finishes its own clipboard/IME cleanup. Once
//! revoked, this view never issues another device command or regains authority.
use riviu_core::{
    ElementBox, ElementQuery, HierarchySourceSnapshot, SwipeGesture, TapPoint, UiSession,
};
use std::sync::atomic::{AtomicBool, Ordering};

pub(super) struct VerificationSession<'a> {
    inner: &'a dyn UiSession,
    authorize: &'a (dyn Fn() -> anyhow::Result<()> + Send + Sync),
    revoked: AtomicBool,
}

impl<'a> VerificationSession<'a> {
    pub(super) fn new(
        inner: &'a dyn UiSession,
        authorize: &'a (dyn Fn() -> anyhow::Result<()> + Send + Sync),
    ) -> Self {
        Self {
            inner,
            authorize,
            revoked: AtomicBool::new(false),
        }
    }

    pub(super) fn check(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            !self.revoked.load(Ordering::Acquire),
            "Lượt lấy link đã dừng để nhả máy"
        );
        if let Err(error) = (self.authorize)() {
            self.revoked.store(true, Ordering::Release);
            return Err(error);
        }
        Ok(())
    }
}

#[async_trait::async_trait]
impl UiSession for VerificationSession<'_> {
    async fn tap(&self, point: TapPoint) -> anyhow::Result<()> {
        self.check()?;
        self.inner.tap(point).await
    }
    async fn activate_element(&self, query: ElementQuery<'_>) -> anyhow::Result<()> {
        self.check()?;
        self.inner.activate_element(query).await
    }
    async fn swipe(&self, gesture: SwipeGesture) -> anyhow::Result<()> {
        self.check()?;
        self.inner.swipe(gesture).await
    }
    async fn back(&self) -> anyhow::Result<()> {
        self.check()?;
        self.inner.back().await
    }
    async fn type_text(&self, _: &str) -> anyhow::Result<()> {
        anyhow::bail!("link verification cannot type")
    }
    async fn home(&self) -> anyhow::Result<()> {
        anyhow::bail!("link verification cannot leave the app")
    }
    async fn find_and_tap(&self, _: &str) -> anyhow::Result<()> {
        anyhow::bail!("link verification requires an observed target")
    }
    async fn assert_visible(&self, id: &str) -> anyhow::Result<()> {
        self.check()?;
        self.inner.assert_visible(id).await
    }
    async fn active_app_bundle(&self) -> anyhow::Result<String> {
        self.check()?;
        self.inner.active_app_bundle().await
    }
    async fn hierarchy_source_snapshot(&self) -> anyhow::Result<HierarchySourceSnapshot> {
        self.check()?;
        let snapshot = self.inner.hierarchy_source_snapshot().await?;
        self.check()?;
        Ok(snapshot)
    }
    async fn locate(&self, query: ElementQuery<'_>) -> anyhow::Result<Option<ElementBox>> {
        self.check()?;
        self.inner.locate(query).await
    }
    async fn locate_all(&self, query: ElementQuery<'_>) -> anyhow::Result<Vec<ElementBox>> {
        self.check()?;
        self.inner.locate_all(query).await
    }
    async fn locate_all_described(
        &self,
        query: ElementQuery<'_>,
    ) -> anyhow::Result<Vec<ElementBox>> {
        self.check()?;
        self.inner.locate_all_described(query).await
    }
    async fn set_clipboard(&self, kind: &str, bytes: &[u8]) -> anyhow::Result<()> {
        self.check()?;
        self.inner.set_clipboard(kind, bytes).await
    }
    async fn get_clipboard(&self, limit: usize) -> anyhow::Result<(String, Vec<u8>)> {
        self.check()?;
        self.inner.get_clipboard(limit).await
    }
    async fn window_size(&self) -> anyhow::Result<(f64, f64)> {
        self.check()?;
        self.inner.window_size().await
    }
    async fn screenshot_png(&self) -> anyhow::Result<Vec<u8>> {
        self.check()?;
        self.inner.screenshot_png().await
    }
    async fn ui_language(&self) -> Option<String> {
        self.check().ok()?;
        self.inner.ui_language().await
    }
    async fn app_version(&self, package: &str) -> Option<String> {
        self.check().ok()?;
        self.inner.app_version(package).await
    }
    fn supports_accessibility_readback(&self) -> bool {
        self.inner.supports_accessibility_readback()
    }
    fn supports_element_bounds(&self) -> bool {
        self.inner.supports_element_bounds()
    }
    fn stream_url(&self) -> Option<String> {
        self.inner.stream_url()
    }
    fn gui_scope(&self) -> Option<riviu_core::ui_automation::GuiScope> {
        self.inner.gui_scope()
    }
    fn gui_session_epoch(&self) -> String {
        self.inner.gui_session_epoch()
    }
    fn gui_reasoner(&self) -> Option<riviu_core::ui_automation::SharedReasoner> {
        self.inner.gui_reasoner()
    }
    fn gui_compatibility_pack(
        &self,
        package: &str,
    ) -> Option<riviu_core::ui_automation::profile::CompatibilityPack> {
        self.inner.gui_compatibility_pack(package)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    struct Phone {
        stopped: AtomicBool,
        reads: AtomicUsize,
        taps: AtomicUsize,
        finished: AtomicBool,
    }
    impl Phone {
        fn new() -> Self {
            Self {
                stopped: AtomicBool::new(false),
                reads: AtomicUsize::new(0),
                taps: AtomicUsize::new(0),
                finished: AtomicBool::new(false),
            }
        }
        fn authorize(&self) -> anyhow::Result<()> {
            anyhow::ensure!(!self.stopped.load(Ordering::SeqCst), "operator stopped");
            Ok(())
        }
    }
    #[async_trait::async_trait]
    impl UiSession for Phone {
        async fn tap(&self, _: TapPoint) -> anyhow::Result<()> {
            self.taps.fetch_add(1, Ordering::SeqCst);
            self.stopped.store(true, Ordering::SeqCst);
            tokio::task::yield_now().await;
            self.finished.store(true, Ordering::SeqCst);
            Ok(())
        }
        async fn hierarchy_source_snapshot(&self) -> anyhow::Result<HierarchySourceSnapshot> {
            self.reads.fetch_add(1, Ordering::SeqCst);
            self.stopped.store(true, Ordering::SeqCst);
            Ok(HierarchySourceSnapshot {
                generation: 1,
                xml: "<hierarchy/>".into(),
            })
        }
        async fn active_app_bundle(&self) -> anyhow::Result<String> {
            Ok("com.zhiliaoapp.musically".into())
        }
        async fn swipe(&self, _: SwipeGesture) -> anyhow::Result<()> {
            panic!("unexpected swipe")
        }
        async fn type_text(&self, _: &str) -> anyhow::Result<()> {
            panic!("unexpected type")
        }
        async fn home(&self) -> anyhow::Result<()> {
            panic!("unexpected home")
        }
        async fn find_and_tap(&self, _: &str) -> anyhow::Result<()> {
            panic!("unexpected input")
        }
        async fn assert_visible(&self, _: &str) -> anyhow::Result<()> {
            Ok(())
        }
        fn stream_url(&self) -> Option<String> {
            None
        }
    }

    #[tokio::test]
    async fn stop_during_snapshot_prevents_next_tap_and_cannot_rearm() {
        let phone = Phone::new();
        let authorize = || phone.authorize();
        let session = VerificationSession::new(&phone, &authorize);
        assert!(session.hierarchy_source_snapshot().await.is_err());
        phone.stopped.store(false, Ordering::SeqCst);
        assert!(session.tap(TapPoint { x: 1.0, y: 1.0 }).await.is_err());
        assert_eq!(phone.taps.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn stop_during_primitive_drains_cleanup_before_refusing_next_input() {
        let phone = Phone::new();
        let authorize = || phone.authorize();
        let session = VerificationSession::new(&phone, &authorize);
        session.tap(TapPoint { x: 1.0, y: 1.0 }).await.unwrap();
        assert!(phone.finished.load(Ordering::SeqCst));
        assert!(session.tap(TapPoint { x: 1.0, y: 1.0 }).await.is_err());
        assert_eq!(phone.taps.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn capture_stopped_in_nested_photo_read_exits_without_navigation() {
        let phone = Phone::new();
        let authorize = || phone.authorize();
        let session = VerificationSession::new(&phone, &authorize);
        let plan = riviu_core::tiktok_share::PublishVerificationPlan::for_build(
            "com.zhiliaoapp.musically",
            "en",
            "45.7.3",
        )
        .unwrap();
        let identity = riviu_core::tiktok_share::SubmissionIdentity {
            prepared_at: None,
            submitted_at: chrono::Utc::now().to_rfc3339(),
            account: "fixture.account".into(),
        };
        let capture = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            riviu_core::tiktok_share::capture_submission_link(
                &session,
                &plan,
                "Fixture caption identifies the intended publication",
                &identity,
            ),
        )
        .await
        .unwrap();
        assert!(capture.outcome.link().is_none());
        assert_eq!(phone.reads.load(Ordering::SeqCst), 1);
        assert_eq!(phone.taps.load(Ordering::SeqCst), 0);
    }
}
