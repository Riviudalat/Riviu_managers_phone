use super::*;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicUsize;

const PACKAGE: &str = "com.ss.android.ugc.trill";

#[derive(Default)]
struct CompletionDriver {
    calls: parking_lot::Mutex<Vec<(String, String)>>,
    fail: AtomicUsize,
    wrong_package: AtomicBool,
}

#[async_trait::async_trait]
impl riviu_core::DeviceDriver for CompletionDriver {
    async fn list_devices(&self) -> anyhow::Result<Vec<riviu_core::DeviceInfo>> {
        Ok(Vec::new())
    }
    async fn refresh_device(&self, _: &str) -> anyhow::Result<riviu_core::DeviceInfo> {
        anyhow::bail!("unexpected refresh")
    }
    async fn install_app(&self, _: &str, _: &Path) -> anyhow::Result<()> {
        panic!("completion must not install")
    }
    async fn uninstall_app(&self, _: &str, _: &str) -> anyhow::Result<()> {
        panic!("completion must not uninstall")
    }
    async fn screenshot(&self, _: &str, _: &Path) -> anyhow::Result<PathBuf> {
        panic!("completion must not open screenshots")
    }
    async fn syslog_tail(&self, _: &str, _: usize) -> anyhow::Result<String> {
        panic!("completion must not read device logs")
    }
    async fn launch_app(&self, _: &str, _: &str) -> anyhow::Result<()> {
        panic!("completion must not launch")
    }
    async fn terminate_app(
        &self,
        udid: &str,
        package: &str,
    ) -> anyhow::Result<riviu_core::ProcessAbsenceProof> {
        self.calls.lock().push((udid.into(), package.into()));
        if self.fail.load(Ordering::SeqCst) > 0 {
            anyhow::bail!("fixture disconnected");
        }
        Ok(riviu_core::ProcessAbsenceProof {
            bundle_id: if self.wrong_package.load(Ordering::SeqCst) {
                "com.other.app"
            } else {
                package
            }
            .into(),
            old_pid: Some(412),
        })
    }
    async fn reboot(&self, _: &str) -> anyhow::Result<()> {
        panic!("completion must not reboot")
    }
    async fn start_ui_session(&self, _: &str) -> anyhow::Result<Box<dyn riviu_core::UiSession>> {
        panic!("completion must not start a UI session")
    }
    async fn ensure_stream(&self, _: &str) -> anyhow::Result<String> {
        panic!("completion must not start a stream")
    }
    async fn prepare_device(&self, _: &str) -> anyhow::Result<()> {
        panic!("completion must not prepare")
    }
}

fn fixture() -> (
    Database,
    Arc<CompletionDriver>,
    DeviceControlPlane,
    Arc<riviu_core::DeviceWorkCoordinator>,
) {
    let path = std::env::temp_dir().join(format!(
        "phone-completion-worker-{}.db",
        uuid::Uuid::new_v4()
    ));
    let db = Database::open(&path).unwrap();
    let driver = Arc::new(CompletionDriver::default());
    let work = Arc::new(riviu_core::DeviceWorkCoordinator::new());
    let control = DeviceControlPlane::new(
        driver.clone(),
        work.clone(),
        Arc::new(riviu_core::StreamBudgetManager::new(1).unwrap()),
    );
    (db, driver, control, work)
}

fn request(db: &Database, udid: &str) -> AppCompletionRecord {
    db.request_app_completion(udid, PACKAGE).unwrap();
    db.list_due_app_completions(32)
        .unwrap()
        .into_iter()
        .find(|row| row.udid == udid)
        .unwrap()
}

fn pending_publish(db: &Database, udid: &str, state: riviu_core::PublishCampaignState) {
    let bundle = riviu_core::PublishBundle {
        id: format!("bundle-{udid}"),
        source_path: "C:/fixture".into(),
        name: "bundle".into(),
        media_kind: riviu_core::PublishMediaKind::Image,
        images: Vec::new(),
        video: None,
        caption_path: "C:/fixture/caption.txt".into(),
        caption: "fixture".into(),
        caption_sha256: "a".repeat(64),
        total_bytes: 1,
        partners: Vec::new(),
    };
    let request = riviu_core::PublishCampaignRequest {
        sheet_delivery: None,
        verification_contract_version: Some(1),
        verification_builds: vec![],
        sheet_enabled: false,
        request_id: format!("request-{udid}"),
        source_root: "C:/fixture".into(),
        bundle_ids: vec![bundle.id.clone()],
        udids: vec![udid.into()],
        run_at: None,
        visibility: riviu_core::PublishVisibility::Public,
        cleanup_policy: riviu_core::PublishCleanupPolicy::KeepImportedAssets,
        network: riviu_core::SocialNetwork::TikTok,
        sound_policy: riviu_core::PublishSoundPolicy::Default,
        execution_confirmed: true,
        target_snapshot: None,
    };
    let campaign = db.create_publish_campaign(&request, &[bundle]).unwrap();
    let id = db
        .get_publish_campaign(&campaign.id)
        .unwrap()
        .unwrap()
        .assignments[0]
        .id
        .clone();
    let evidence = if state == riviu_core::PublishCampaignState::Queued {
        serde_json::json!({"post":{"state":"notStarted"}})
    } else {
        serde_json::json!({
            "post":{"state":"submitted","publicationVerified":false,"submittedAt":"2026-09-15T00:00:00Z","expectedAccount":"fixture"},
            "verificationStatus":{"state":"verifying","nextCheckAt":"2099-01-01T00:00:00Z"}
        })
    };
    db.update_publish_assignment_state(&id, state, None, Some(&evidence.to_string()))
        .unwrap();
}

#[tokio::test]
async fn idle_completion_terminates_exact_app_once_and_records_actual_proof() {
    let (db, driver, control, _) = fixture();
    let row = request(&db, "phone");
    close_if_finished(&control, &db, &row).await.unwrap();
    assert_eq!(
        driver.calls.lock().as_slice(),
        &[("phone".into(), PACKAGE.into())]
    );
    assert!(!db.app_completion_is_current(&row).unwrap());
    let completed = db.get_app_completion("phone", PACKAGE).unwrap().unwrap();
    assert_eq!(completed.state, "completed");
    let proof: riviu_core::ProcessAbsenceProof =
        serde_json::from_str(completed.proof_json.as_deref().unwrap()).unwrap();
    assert_eq!(proof.bundle_id, PACKAGE);
    assert_eq!(proof.old_pid, Some(412));
    assert!(db.list_due_app_completions(32).unwrap().is_empty());
    assert!(control.current_work_owner("phone").is_none());
    close_if_finished(&control, &db, &row).await.unwrap();
    assert_eq!(driver.calls.lock().len(), 1);
    assert!(db
        .try_publish_work("phone", "compose", "next-job")
        .unwrap()
        .is_some());
}

#[tokio::test]
async fn pending_post_link_and_queued_next_stage_prevent_termination() {
    for state in [
        riviu_core::PublishCampaignState::Verifying,
        riviu_core::PublishCampaignState::Queued,
    ] {
        let (db, driver, control, _) = fixture();
        pending_publish(&db, "phone", state);
        let row = request(&db, "phone");
        assert!(db.app_completion_block_reason("phone").unwrap().is_some());
        close_if_finished(&control, &db, &row).await.unwrap();
        assert!(driver.calls.lock().is_empty());
        assert!(db.app_completion_is_current(&row).unwrap());
        assert!(control.current_work_owner("phone").is_none());
    }
}

#[tokio::test]
async fn stale_request_generation_cannot_terminate_the_new_generation() {
    let (db, driver, control, _) = fixture();
    let old = request(&db, "phone");
    let new = request(&db, "phone");
    assert!(new.revision > old.revision);
    close_if_finished(&control, &db, &old).await.unwrap();
    assert!(driver.calls.lock().is_empty());
    assert!(db.app_completion_is_current(&new).unwrap());
    close_if_finished(&control, &db, &new).await.unwrap();
    assert_eq!(driver.calls.lock().len(), 1);
}

#[tokio::test]
async fn device_busy_and_device_failure_do_not_prevent_another_phone_completion() {
    let (db, driver, control, _) = fixture();
    let busy_row = request(&db, "busy-phone");
    let other_row = request(&db, "other-phone");
    let busy = control
        .try_acquire_exclusive_keeping_stream("busy-phone", DeviceWorkOwner::ManualControl)
        .await
        .unwrap();
    assert!(close_if_finished(&control, &db, &busy_row).await.is_err());
    close_if_finished(&control, &db, &other_row).await.unwrap();
    assert_eq!(
        driver.calls.lock().as_slice(),
        &[("other-phone".into(), PACKAGE.into())]
    );
    control.close_exclusive_context(busy).unwrap();
    driver.fail.store(1, Ordering::SeqCst);
    assert!(close_if_finished(&control, &db, &busy_row).await.is_err());
    assert!(db.app_completion_is_current(&busy_row).unwrap());
    assert!(control.current_work_owner("busy-phone").is_none());
    driver.fail.store(0, Ordering::SeqCst);
    close_if_finished(&control, &db, &busy_row).await.unwrap();
    assert!(!db.app_completion_is_current(&busy_row).unwrap());
}

#[tokio::test]
async fn queued_lease_owner_prevents_idle_closer_from_jumping_the_queue() {
    let (db, driver, control, work) = fixture();
    let row = request(&db, "phone");
    let held = work
        .try_acquire("phone", DeviceWorkOwner::ManualControl)
        .unwrap();
    let mut waiting = Box::pin(work.acquire("phone", DeviceWorkOwner::Nurture));
    std::future::poll_fn(|cx| {
        assert!(waiting.as_mut().poll(cx).is_pending());
        std::task::Poll::Ready(())
    })
    .await;
    drop(held);
    assert!(close_if_finished(&control, &db, &row).await.is_err());
    assert!(driver.calls.lock().is_empty());
    drop(waiting.await.unwrap());
    close_if_finished(&control, &db, &row).await.unwrap();
    assert_eq!(driver.calls.lock().len(), 1);
}

#[tokio::test]
async fn mismatched_process_proof_keeps_completion_pending_and_releases_capacity() {
    let (db, driver, control, _) = fixture();
    let row = request(&db, "phone");
    driver.wrong_package.store(true, Ordering::SeqCst);
    assert!(close_if_finished(&control, &db, &row).await.is_err());
    assert!(db.app_completion_is_current(&row).unwrap());
    assert!(db
        .get_app_completion("phone", PACKAGE)
        .unwrap()
        .unwrap()
        .proof_json
        .is_none());
    assert!(control.current_work_owner("phone").is_none());
    assert!(db
        .try_publish_work("phone", "compose", "next-job")
        .unwrap()
        .is_some());
}
