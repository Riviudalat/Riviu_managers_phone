use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

#[derive(Default)]
struct CleanupDriver {
    deletes: AtomicUsize,
    fail: AtomicUsize,
}
#[async_trait::async_trait]
impl riviu_core::DeviceDriver for CleanupDriver {
    async fn list_devices(&self) -> anyhow::Result<Vec<riviu_core::DeviceInfo>> {
        Ok(Vec::new())
    }
    async fn refresh_device(&self, _: &str) -> anyhow::Result<riviu_core::DeviceInfo> {
        anyhow::bail!("unexpected refresh")
    }
    async fn install_app(&self, _: &str, _: &Path) -> anyhow::Result<()> {
        anyhow::bail!("unexpected install")
    }
    async fn uninstall_app(&self, _: &str, _: &str) -> anyhow::Result<()> {
        anyhow::bail!("unexpected uninstall")
    }
    async fn screenshot(&self, _: &str, _: &Path) -> anyhow::Result<PathBuf> {
        anyhow::bail!("unexpected screenshot")
    }
    async fn syslog_tail(&self, _: &str, _: usize) -> anyhow::Result<String> {
        anyhow::bail!("unexpected syslog")
    }
    async fn launch_app(&self, _: &str, _: &str) -> anyhow::Result<()> {
        panic!("cleanup must not launch TikTok")
    }
    async fn terminate_app(
        &self,
        _: &str,
        _: &str,
    ) -> anyhow::Result<riviu_core::ProcessAbsenceProof> {
        panic!("cleanup must not terminate TikTok")
    }
    async fn reboot(&self, _: &str) -> anyhow::Result<()> {
        panic!("cleanup must not reboot")
    }
    async fn start_ui_session(&self, _: &str) -> anyhow::Result<Box<dyn riviu_core::UiSession>> {
        panic!("cleanup must not open composer/session")
    }
    async fn ensure_stream(&self, _: &str) -> anyhow::Result<String> {
        panic!("cleanup must not restart preview")
    }
    async fn prepare_device(&self, _: &str) -> anyhow::Result<()> {
        panic!("cleanup must not prepare device")
    }
    async fn cleanup_publish_media(
        &self,
        _: &str,
        import: &str,
    ) -> anyhow::Result<serde_json::Value> {
        assert_eq!(import, "riviu-fixture-aa");
        self.deletes.fetch_add(1, Ordering::SeqCst);
        if self.fail.load(Ordering::SeqCst) > 0 {
            anyhow::bail!("fixture disconnected");
        }
        Ok(serde_json::json!({"state":"cleaned","importId":import}))
    }
}

fn fixture() -> (Database, PathBuf, String) {
    let path = std::env::temp_dir().join(format!("deferred-cleanup-{}.db", Uuid::new_v4()));
    let db = Database::open(&path).unwrap();
    let bundle = riviu_core::PublishBundle {
        id: "bundle".into(),
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
    let request = PublishCampaignRequest {
        sheet_delivery: None,
        verification_contract_version: Some(1),
        verification_builds: vec![],
        sheet_enabled: false,
        request_id: "request".into(),
        source_root: "C:/fixture".into(),
        bundle_ids: vec!["bundle".into()],
        udids: vec!["phone".into()],
        run_at: None,
        visibility: PublishVisibility::Public,
        cleanup_policy: PublishCleanupPolicy::DeleteImportedAssetsAfterVerified,
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
    db.update_publish_assignment_state(&id,riviu_core::PublishCampaignState::Succeeded,None,Some(&serde_json::json!({
        "post":{"state":"posted","publicationVerified":true,"postUrl":"https://www.tiktok.com/@fixture/photo/123","importId":"riviu-fixture-aa"},
        "cleanup":{"state":"kept","reason":"post_verification_pending","importId":"riviu-fixture-aa"}
    }).to_string())).unwrap();
    (db, path, id)
}

fn control(driver: Arc<CleanupDriver>) -> DeviceControlPlane {
    DeviceControlPlane::new(
        driver,
        Arc::new(riviu_core::DeviceWorkCoordinator::new()),
        Arc::new(riviu_core::StreamBudgetManager::new(1).unwrap()),
    )
}

#[test]
fn native_cleanup_requires_cleaned_state_for_exact_import() {
    assert_eq!(
        cleanup_result(
            "riviu-a",
            Ok(serde_json::json!({"state":"cleaned","importId":"riviu-b"}))
        )["state"],
        "not_cleaned"
    );
    assert_eq!(
        cleanup_result("riviu-a", Ok(serde_json::json!({"state":"cleaned"})))["state"],
        "not_cleaned"
    );
    assert_eq!(
        cleanup_result("riviu-a", Err("lost response".into()))["state"],
        "not_cleaned"
    );
    assert_eq!(
        cleanup_result(
            "riviu-a",
            Ok(serde_json::json!({"value":{"state":"cleaned","importId":"riviu-a"}}))
        )["state"],
        "cleaned"
    );
}

#[tokio::test]
async fn deferred_cleanup_retains_warm_app_and_resumes_failed_import_after_restart() {
    let (db, path, id) = fixture();
    let driver = Arc::new(CleanupDriver::default());
    let control = control(driver.clone());
    let events = riviu_core::events::EventBus::new(16);
    let busy = control
        .try_acquire_exclusive_keeping_stream("phone", DeviceWorkOwner::ManualControl)
        .await
        .unwrap();
    assert_eq!(
        cleanup_verified_assignments(&control, &db, &events, 1)
            .await
            .unwrap(),
        0
    );
    assert_eq!(driver.deletes.load(Ordering::SeqCst), 0);
    control.close_exclusive_context(busy).unwrap();
    driver.fail.store(1, Ordering::SeqCst);
    assert_eq!(
        cleanup_verified_assignments(&control, &db, &events, 1)
            .await
            .unwrap(),
        0
    );
    assert_eq!(control.current_work_owner("phone"), None);
    assert_eq!(
        db.pending_publish_cleanup(&id).unwrap().unwrap().import_id,
        "riviu-fixture-aa"
    );
    drop(db);
    let db = Database::open(&path).unwrap();
    driver.fail.store(0, Ordering::SeqCst);
    assert_eq!(
        cleanup_verified_assignments(&control, &db, &events, 1)
            .await
            .unwrap(),
        1
    );
    assert_eq!(driver.deletes.load(Ordering::SeqCst), 2);
    assert_eq!(control.current_work_owner("phone"), None);
    assert!(db.pending_publish_cleanup(&id).unwrap().is_none());
    assert_eq!(
        cleanup_verified_assignments(&control, &db, &events, 1)
            .await
            .unwrap(),
        0
    );
    assert_eq!(driver.deletes.load(Ordering::SeqCst), 2);
    drop(db);
    let _ = std::fs::remove_file(path);
}

#[tokio::test]
async fn stale_or_unverified_assignment_never_deletes_media() {
    let (db, path, id) = fixture();
    let driver = Arc::new(CleanupDriver::default());
    let control = control(driver.clone());
    let events = riviu_core::events::EventBus::new(16);
    let candidate = db.pending_publish_cleanup(&id).unwrap().unwrap();
    db.update_publish_assignment_state(
        &id,
        riviu_core::PublishCampaignState::Succeeded,
        None,
        Some(&candidate.evidence_json),
    )
    .unwrap();
    assert!(
        !cleanup_verified_assignment(&control, &db, &events, &candidate)
            .await
            .unwrap()
    );
    assert_eq!(driver.deletes.load(Ordering::SeqCst), 0);
    let candidate = db.pending_publish_cleanup(&id).unwrap().unwrap();
    db.update_publish_assignment_state(
        &id,
        riviu_core::PublishCampaignState::Uncertain,
        None,
        Some(&candidate.evidence_json),
    )
    .unwrap();
    assert!(
        !cleanup_verified_assignment(&control, &db, &events, &candidate)
            .await
            .unwrap()
    );
    assert_eq!(driver.deletes.load(Ordering::SeqCst), 0);
    assert_eq!(control.current_work_owner("phone"), None);
    drop(db);
    let _ = std::fs::remove_file(path);
}
