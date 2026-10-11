use super::mobile::*;
use crate::{HierarchySourceSnapshot, SwipeGesture, TapPoint, UiSession};
use parking_lot::Mutex;

struct Session {
    stage: Mutex<u8>,
    disconnected: bool,
    composer: bool,
    caption_input_timeout: bool,
    fail_share: bool,
    account_probe: bool,
    profile_probe_449: bool,
    profile_449_text_edit: bool,
    initial_share_overlay: bool,
    login: bool,
    empty_composer: bool,
    delayed_profile_reads: Option<usize>,
    profile_reads: Mutex<usize>,
    swipes: Mutex<usize>,
    taps: Mutex<Vec<TapPoint>>,
    tap_times: Mutex<Vec<std::time::Instant>>,
    clipboard: Mutex<Vec<u8>>,
    unreadable_clipboard: bool,
    long_detail: bool,
    detail_scrolled: Mutex<bool>,
}
impl Session {
    fn new() -> Self {
        Self {
            stage: Mutex::new(0),
            disconnected: false,
            composer: false,
            caption_input_timeout: false,
            fail_share: false,
            account_probe: false,
            profile_probe_449: false,
            profile_449_text_edit: false,
            initial_share_overlay: false,
            login: false,
            empty_composer: false,
            delayed_profile_reads: None,
            profile_reads: Mutex::new(0),
            swipes: Mutex::new(0),
            taps: Mutex::new(vec![]),
            tap_times: Mutex::new(vec![]),
            clipboard: Mutex::new(b"operator clipboard".to_vec()),
            unreadable_clipboard: false,
            long_detail: false,
            detail_scrolled: Mutex::new(false),
        }
    }
}
#[async_trait::async_trait]
impl UiSession for Session {
    fn stream_url(&self) -> Option<String> {
        None
    }
    async fn active_app_bundle(&self) -> anyhow::Result<String> {
        anyhow::ensure!(!self.disconnected, "device disconnected");
        Ok(super::ANDROID_PACKAGE.into())
    }
    async fn hierarchy_source_snapshot(&self) -> anyhow::Result<HierarchySourceSnapshot> {
        let xml = if self.profile_probe_449 {
            match *self.stage.lock() {
                0 => include_str!("../../fixtures/threads_449/feed.xml"),
                1 | 3 if self.profile_449_text_edit => {
                    include_str!("../../fixtures/threads_449/profile_text_edit.xml")
                }
                1 | 3 => include_str!("../../fixtures/threads_449/profile_without_header.xml"),
                _ => include_str!("../../fixtures/threads_449/profile_editor_vi.xml"),
            }
        } else if self.login {
            include_str!("../../fixtures/threads_446/login.xml")
        } else if self.empty_composer && *self.stage.lock() == 4 {
            include_str!("../../fixtures/threads_446/empty_composer.xml")
        } else if self.composer {
            include_str!("../../fixtures/threads_446/composer.xml")
        } else {
            match *self.stage.lock() {
                0 => {
                    let mut reads = self.profile_reads.lock();
                    *reads += 1;
                    if self
                        .delayed_profile_reads
                        .is_some_and(|after| *reads >= after)
                    {
                        include_str!("../../fixtures/threads_446/own_post.xml")
                    } else {
                        include_str!("../../fixtures/threads_446/profile.xml")
                    }
                }
                1 => include_str!("../../fixtures/threads_446/own_post.xml"),
                2 => include_str!("../../fixtures/threads_446/detail.xml"),
                4 => include_str!("../../fixtures/threads_446/share_overlay.xml"),
                _ => include_str!("../../fixtures/threads_446/share.xml"),
            }
        };
        let xml = if self.long_detail && *self.stage.lock() == 2 && !*self.detail_scrolled.lock() {
            xml.replace("feed_post_ufi_share_button", "offscreen_share")
        } else { xml.to_string() };
        Ok(HierarchySourceSnapshot {
            generation: 1,
            xml: xml.into(),
        })
    }
    async fn tap(&self, point: TapPoint) -> anyhow::Result<()> {
        self.taps.lock().push(point);
        self.tap_times.lock().push(std::time::Instant::now());
        if self.profile_probe_449 {
            *self.stage.lock() += 1;
            return Ok(());
        }
        if self.account_probe {
            *self.stage.lock() = 0;
            return Ok(());
        }
        let tap_count = self.taps.lock().len();
        if self.initial_share_overlay && tap_count <= 2 {
            *self.stage.lock() = if tap_count == 1 { 2 } else { 0 };
            return Ok(());
        }
        let refresh_taps = if self.delayed_profile_reads == Some(usize::MAX) {
            if tap_count <= 3 { return Ok(()); }
            2
        } else { 0 };
        let publish_taps = tap_count - usize::from(self.initial_share_overlay) * 2 - refresh_taps;
        if publish_taps == 2 {
            *self.stage.lock() = 2;
        } else if publish_taps == 3 {
            anyhow::ensure!(!self.fail_share, "connection lost after Share");
            *self.stage.lock() = 3;
        } else if publish_taps == 4 {
            *self.clipboard.lock() =
                b"https://www.threads.com/@fixture.account/post/Fixture123".to_vec();
        }
        Ok(())
    }
    async fn swipe(&self, gesture: SwipeGesture) -> anyhow::Result<()> {
        if self.long_detail && *self.stage.lock() == 2 {
            *self.detail_scrolled.lock() = true;
            *self.swipes.lock() += 1;
            return Ok(());
        }
        if self.delayed_profile_reads == Some(usize::MAX) {
            anyhow::ensure!(gesture.to.y > gesture.from.y, "stale profile needs downward refresh before searching");
        }
        *self.swipes.lock() += 1;
        *self.stage.lock() = 1;
        Ok(())
    }
    async fn set_clipboard(&self, _: &str, bytes: &[u8]) -> anyhow::Result<()> {
        *self.clipboard.lock() = bytes.to_vec();
        Ok(())
    }
    async fn get_clipboard(&self, _: usize) -> anyhow::Result<(String, Vec<u8>)> {
        if self.unreadable_clipboard && !self.clipboard.lock().starts_with(b"https://") {
            anyhow::bail!("helper HTTP 400 clipboard_unavailable");
        }
        Ok(("plaintext".into(), self.clipboard.lock().clone()))
    }
    async fn type_text(&self, _: &str) -> anyhow::Result<()> {
        if self.caption_input_timeout { anyhow::bail!("set_text timed out"); }
        panic!("verification must not type")
    }
    async fn home(&self) -> anyhow::Result<()> {
        panic!("verification must not Home")
    }
    async fn find_and_tap(&self, _: &str) -> anyhow::Result<()> {
        panic!("verification must use observed bounds")
    }
    async fn assert_visible(&self, _: &str) -> anyhow::Result<()> {
        unreachable!()
    }
}

#[tokio::test]
async fn long_detail_and_unreadable_old_clipboard_still_copy_verified_link() {
    let session = Session { long_detail: true, unreadable_clipboard: true, ..Session::new() };
    let link = verify(&session, "Fixture caption", "fixture.account", &submitted()).await.unwrap();
    assert_eq!(link, "https://www.threads.com/@fixture.account/post/Fixture123");
    assert!(*session.detail_scrolled.lock());
    assert!(session.clipboard.lock().starts_with(b"https://"));
}

#[tokio::test]
async fn caption_input_timeout_continues_only_with_exact_ready_composer() {
    let session = Session { composer: true, caption_input_timeout: true, ..Session::new() };
    enter_caption_and_wait_ready(&session, "Fixture caption", "fixture.account", std::time::Duration::from_secs(2)).await.unwrap();
    assert!(session.taps.lock().is_empty());
    assert!(enter_caption_and_wait_ready(&session, "Wrong caption", "fixture.account", std::time::Duration::from_millis(100)).await.unwrap_err().to_string().contains("caption readback differs"));
    assert!(session.taps.lock().is_empty());
}

#[tokio::test]
async fn visible_profile_probe_reads_without_tapping_or_swiping() {
    let session = Session::new();
    assert_eq!(visible_profile_account(&session).await.unwrap(), "fixture.account");
    assert!(session.taps.lock().is_empty());
    assert_eq!(*session.swipes.lock(), 0);
}

#[tokio::test]
async fn threads_449_probe_opens_profile_and_reads_measured_editor_field() {
    let session = Session {
        profile_probe_449: true,
        ..Session::new()
    };
    assert_eq!(
        observed_449_profile_account(&session).await.unwrap(),
        "fixture.account"
    );
    assert_eq!(session.taps.lock().len(), 3);
    assert_eq!(*session.swipes.lock(), 0);
}

#[tokio::test]
async fn threads_449_probe_accepts_labeled_profile_edit_control() {
    let session = Session {
        profile_probe_449: true,
        profile_449_text_edit: true,
        ..Session::new()
    };
    assert_eq!(
        observed_449_profile_account(&session).await.unwrap(),
        "fixture.account"
    );
    assert_eq!(session.taps.lock().len(), 3);
}

#[tokio::test]
async fn threads_449_probe_leaves_existing_profile_edit_untouched() {
    let session = Session {
        stage: Mutex::new(2),
        profile_probe_449: true,
        ..Session::new()
    };
    let error = observed_449_profile_account(&session).await.unwrap_err();
    assert!(error.to_string().contains("editor is already open"));
    assert!(session.taps.lock().is_empty());
}

#[tokio::test]
async fn account_probe_returns_from_measured_post_detail_before_reading_profile() {
    let session = Session {
        stage: Mutex::new(2),
        account_probe: true,
        ..Session::new()
    };
    assert_eq!(
        observed_profile_account(&session).await.unwrap(),
        "fixture.account"
    );
    assert_eq!(session.taps.lock().len(), 2);
}

#[tokio::test]
async fn account_probe_reports_logged_out_threads_without_waiting_for_profile_tab() {
    let session = Session {
        login: true,
        ..Session::new()
    };
    assert!(observed_profile_account(&session)
        .await
        .unwrap_err()
        .to_string()
        .contains("not signed in"));
    assert!(session.taps.lock().is_empty());
}

#[tokio::test]
async fn account_probe_returns_from_empty_composer_without_touching_post() {
    let session = Session {
        stage: Mutex::new(4),
        account_probe: true,
        empty_composer: true,
        ..Session::new()
    };
    assert_eq!(
        observed_profile_account(&session).await.unwrap(),
        "fixture.account"
    );
    assert_eq!(session.taps.lock().len(), 2);
}

#[tokio::test]
async fn account_probe_keeps_existing_composer_draft() {
    let session = Session {
        composer: true,
        ..Session::new()
    };
    assert!(observed_profile_account(&session)
        .await
        .unwrap_err()
        .to_string()
        .contains("keep it for operator review"));
    assert!(session.taps.lock().is_empty());
}
fn submitted() -> String {
    (chrono::Utc::now() - chrono::Duration::seconds(180)).to_rfc3339()
}

#[tokio::test]
async fn reconciliation_refreshes_stale_profile_before_caption_search() {
    let session = Session { delayed_profile_reads: Some(usize::MAX), ..Session::new() };
    let link = verify_after_refresh(&session, "Fixture caption", "fixture.account", &submitted()).await.unwrap();
    assert_eq!(link, "https://www.threads.com/@fixture.account/post/Fixture123");
    assert_eq!(*session.swipes.lock(), 1);
    assert_eq!(session.taps.lock().len(), 6);
    let times = session.tap_times.lock();
    assert!(times[2].duration_since(times[1]) >= std::time::Duration::from_secs(10));
    assert_eq!(*session.clipboard.lock(), b"operator clipboard");
}

#[tokio::test]
async fn refresh_refuses_wrong_account_before_swiping() {
    let session = Session::new();
    assert!(verify_after_refresh(&session, "Fixture caption", "other.account", &submitted()).await.is_err());
    assert_eq!(*session.swipes.lock(), 0);
}

#[tokio::test]
async fn reconciliation_after_restart_only_reads_own_post_and_restores_clipboard() {
    for _ in 0..2 {
        let session = Session::new();
        let link = verify(&session, "Fixture caption", "fixture.account", &submitted())
            .await
            .unwrap();
        assert_eq!(
            link,
            "https://www.threads.com/@fixture.account/post/Fixture123"
        );
        assert_eq!(*session.clipboard.lock(), b"operator clipboard");
        assert_eq!(session.taps.lock().len(), 4);
    }
}

#[tokio::test]
async fn reconciliation_waits_for_new_profile_row_before_scrolling_into_history() {
    let session = Session {
        delayed_profile_reads: Some(5),
        ..Session::new()
    };
    let link = verify(&session, "Fixture caption", "fixture.account", &submitted())
        .await
        .unwrap();
    assert_eq!(
        link,
        "https://www.threads.com/@fixture.account/post/Fixture123"
    );
    assert_eq!(*session.swipes.lock(), 0);
}

#[tokio::test]
async fn reconciliation_closes_measured_share_overlay_before_reading_own_profile() {
    let session = Session {
        stage: Mutex::new(4),
        initial_share_overlay: true,
        ..Session::new()
    };
    assert_eq!(
        verify(&session, "Fixture caption", "fixture.account", &submitted())
            .await
            .unwrap(),
        "https://www.threads.com/@fixture.account/post/Fixture123"
    );
    assert_eq!(session.taps.lock().len(), 6);
    assert_eq!(*session.clipboard.lock(), b"operator clipboard");
}

#[tokio::test]
async fn disconnect_and_open_composer_never_dispatch_post_during_reconciliation() {
    for (disconnected, composer) in [(true, false), (false, true)] {
        let session = Session {
            disconnected,
            composer,
            ..Session::new()
        };
        assert!(
            verify(&session, "Fixture caption", "fixture.account", &submitted())
                .await
                .is_err()
        );
        assert!(session.taps.lock().is_empty());
    }
}

#[tokio::test]
async fn wrong_account_and_lost_share_response_remain_unverified() {
    let wrong = Session::new();
    assert!(
        verify(&wrong, "Fixture caption", "other.account", &submitted())
            .await
            .is_err()
    );
    assert_eq!(wrong.taps.lock().len(), 1);
    let lost = Session {
        fail_share: true,
        ..Session::new()
    };
    assert!(
        verify(&lost, "Fixture caption", "fixture.account", &submitted())
            .await
            .is_err()
    );
    assert_eq!(*lost.clipboard.lock(), b"operator clipboard");
    assert_eq!(lost.taps.lock().len(), 3);
}
