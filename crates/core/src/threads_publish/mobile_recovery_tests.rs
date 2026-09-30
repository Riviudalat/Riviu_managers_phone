use super::mobile::*;
use crate::{HierarchySourceSnapshot, SwipeGesture, TapPoint, UiSession};
use parking_lot::Mutex;

struct Session {
    stage: Mutex<u8>,
    disconnected: bool,
    composer: bool,
    fail_share: bool,
    account_probe: bool,
    taps: Mutex<Vec<TapPoint>>,
    clipboard: Mutex<Vec<u8>>,
}
impl Session {
    fn new() -> Self {
        Self {
            stage: Mutex::new(0),
            disconnected: false,
            composer: false,
            fail_share: false,
            account_probe: false,
            taps: Mutex::new(vec![]),
            clipboard: Mutex::new(b"operator clipboard".to_vec()),
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
        let xml = if self.composer {
            include_str!("../../fixtures/threads_446/composer.xml")
        } else {
            match *self.stage.lock() {
                0 => include_str!("../../fixtures/threads_446/profile.xml"),
                1 => include_str!("../../fixtures/threads_446/own_post.xml"),
                2 => include_str!("../../fixtures/threads_446/detail.xml"),
                _ => include_str!("../../fixtures/threads_446/share.xml"),
            }
        };
        Ok(HierarchySourceSnapshot {
            generation: 1,
            xml: xml.into(),
        })
    }
    async fn tap(&self, point: TapPoint) -> anyhow::Result<()> {
        self.taps.lock().push(point);
        if self.account_probe {
            *self.stage.lock() = 0;
            return Ok(());
        }
        if self.taps.lock().len() == 2 {
            *self.stage.lock() = 2;
        } else if self.taps.lock().len() == 3 {
            anyhow::ensure!(!self.fail_share, "connection lost after Share");
            *self.stage.lock() = 3;
        } else if self.taps.lock().len() == 4 {
            *self.clipboard.lock() =
                b"https://www.threads.com/@fixture.account/post/Fixture123".to_vec();
        }
        Ok(())
    }
    async fn swipe(&self, _: SwipeGesture) -> anyhow::Result<()> {
        *self.stage.lock() = 1;
        Ok(())
    }
    async fn set_clipboard(&self, _: &str, bytes: &[u8]) -> anyhow::Result<()> {
        *self.clipboard.lock() = bytes.to_vec();
        Ok(())
    }
    async fn get_clipboard(&self, _: usize) -> anyhow::Result<(String, Vec<u8>)> {
        Ok(("plaintext".into(), self.clipboard.lock().clone()))
    }
    async fn type_text(&self, _: &str) -> anyhow::Result<()> {
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
fn submitted() -> String {
    (chrono::Utc::now() - chrono::Duration::seconds(180)).to_rfc3339()
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
