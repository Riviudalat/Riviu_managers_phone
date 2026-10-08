use super::*;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

const PACKAGE: &str = "com.zhiliaoapp.musically";

/// A phone with an app list. Every effect is recorded so a test can prove what was *not* sent.
#[derive(Default)]
struct RemovalDriver {
    installed: parking_lot::Mutex<HashSet<String>>,
    calls: parking_lot::Mutex<Vec<String>>,
    list_fails: AtomicBool,
    uninstall_fails: AtomicBool,
    uninstall_is_ignored: AtomicBool,
}

impl RemovalDriver {
    fn with_app() -> Arc<Self> {
        let driver = Self::default();
        driver.installed.lock().insert(PACKAGE.to_string());
        Arc::new(driver)
    }
    fn effects(&self) -> Vec<String> {
        self.calls
            .lock()
            .iter()
            .filter(|call| !call.starts_with("list"))
            .cloned()
            .collect()
    }
}

#[async_trait::async_trait]
impl riviu_core::DeviceDriver for RemovalDriver {
    async fn list_devices(&self) -> anyhow::Result<Vec<riviu_core::DeviceInfo>> {
        Ok(Vec::new())
    }
    async fn refresh_device(&self, _: &str) -> anyhow::Result<riviu_core::DeviceInfo> {
        anyhow::bail!("unexpected refresh")
    }
    async fn install_app(&self, _: &str, path: &Path) -> anyhow::Result<()> {
        self.calls
            .lock()
            .push(format!("install {}", path.display()));
        self.installed.lock().insert(PACKAGE.to_string());
        Ok(())
    }
    async fn uninstall_app(&self, _: &str, bundle_id: &str) -> anyhow::Result<()> {
        self.calls.lock().push(format!("uninstall {bundle_id}"));
        if self.uninstall_fails.load(Ordering::SeqCst) {
            anyhow::bail!("adb: device offline");
        }
        if !self.uninstall_is_ignored.load(Ordering::SeqCst) {
            self.installed.lock().remove(bundle_id);
        }
        Ok(())
    }
    async fn list_installed_apps(&self, _: &str) -> anyhow::Result<Vec<riviu_core::InstalledApp>> {
        self.calls.lock().push("list".into());
        if self.list_fails.load(Ordering::SeqCst) {
            anyhow::bail!("package service unavailable");
        }
        Ok(self
            .installed
            .lock()
            .iter()
            .map(|bundle_id| riviu_core::InstalledApp {
                bundle_id: bundle_id.clone(),
                kind: riviu_core::InstalledAppKind::User,
                label: None,
                icon_png_base64: None,
            })
            .collect())
    }
    async fn screenshot(&self, _: &str, _: &Path) -> anyhow::Result<PathBuf> {
        panic!("removal must not take screenshots")
    }
    async fn syslog_tail(&self, _: &str, _: usize) -> anyhow::Result<String> {
        panic!("removal must not read device logs")
    }
    async fn launch_app(&self, _: &str, _: &str) -> anyhow::Result<()> {
        panic!("removal must not launch apps")
    }
    async fn terminate_app(
        &self,
        _: &str,
        _: &str,
    ) -> anyhow::Result<riviu_core::ProcessAbsenceProof> {
        panic!("removal must not terminate apps")
    }
    async fn reboot(&self, _: &str) -> anyhow::Result<()> {
        panic!("removal must not reboot")
    }
    async fn start_ui_session(&self, _: &str) -> anyhow::Result<Box<dyn riviu_core::UiSession>> {
        panic!("removal must not start a UI session")
    }
    async fn ensure_stream(&self, _: &str) -> anyhow::Result<String> {
        panic!("removal must not start a stream")
    }
    async fn prepare_device(&self, _: &str) -> anyhow::Result<()> {
        panic!("removal must not prepare devices")
    }
}

fn fixture(driver: Arc<RemovalDriver>) -> (Database, DeviceControlPlane) {
    let path = std::env::temp_dir().join(format!("app-removal-{}.db", Uuid::new_v4()));
    let db = Database::open(&path).unwrap();
    let control = DeviceControlPlane::new(
        driver,
        Arc::new(riviu_core::DeviceWorkCoordinator::new()),
        Arc::new(riviu_core::StreamBudgetManager::new(1).unwrap()),
    );
    (db, control)
}

fn library_item(platform: AppLibraryPlatform, application_id: &str) -> AppLibraryItem {
    let (format, bundle_id) = match platform {
        AppLibraryPlatform::Android => (AppPackageFormat::Apk, String::new()),
        AppLibraryPlatform::Ios => (AppPackageFormat::Ipa, application_id.to_string()),
    };
    AppLibraryItem {
        id: "app-1".into(),
        name: "TikTok".into(),
        path: "C:/library/snapshot/tiktok.ipa".into(),
        bundle_id,
        version: "1".into(),
        platform,
        package_format: format,
        artifact_kind: format,
        application_id: application_id.into(),
        version_name: "1".into(),
        version_code: None,
        sha256: "a".repeat(64),
        size_bytes: 1,
        signer_sha256: String::new(),
        icon_png_base64: None,
        metadata_status: "ok".into(),
        metadata_error: None,
        created_at: "2026-10-08T00:00:00Z".into(),
    }
}

async fn run(
    control: &DeviceControlPlane,
    db: &Database,
    item: &AppLibraryItem,
    mode: AppRemovalMode,
) -> AppRemovalResult {
    let plan = RemovalPlan {
        item,
        package: PACKAGE,
        mode,
        scratch: None,
    };
    tokio::time::timeout(
        std::time::Duration::from_secs(5),
        remove_on_device(control, db, &plan, "phone".into()),
    )
    .await
    .expect("a busy phone is refused, never waited for")
}

/// Mirrors the publish fixture in `phone_app_completion_tests.rs`: one post submitted and
/// still waiting for its link.
fn pending_publish(db: &Database, udid: &str) {
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
    let evidence = serde_json::json!({
        "post":{"state":"submitted","publicationVerified":false,"submittedAt":"2026-09-15T00:00:00Z","expectedAccount":"fixture"},
        "verificationStatus":{"state":"verifying","nextCheckAt":"2099-01-01T00:00:00Z"}
    });
    db.update_publish_assignment_state(
        &id,
        riviu_core::PublishCampaignState::Verifying,
        None,
        Some(&evidence.to_string()),
    )
    .unwrap();
}

#[test]
fn only_a_well_formed_library_identity_that_is_not_riviu_itself_can_be_removed() {
    assert_eq!(
        removal_package_id(&library_item(AppLibraryPlatform::Android, PACKAGE)).unwrap(),
        PACKAGE
    );
    assert_eq!(
        removal_package_id(&library_item(
            AppLibraryPlatform::Ios,
            "com.zhiliaoapp.musically"
        ))
        .unwrap(),
        "com.zhiliaoapp.musically"
    );
    for (id, code) in [
        ("com.riviu.agent", "AppProtected"),
        ("com.riviu.managersphone.agent.xctrunner", "AppProtected"),
        ("io.appium.uiautomator2.server", "AppProtected"),
        ("io.appium.uiautomator2.server.test", "AppProtected"),
        (
            "com.facebook.WebDriverAgentRunner.xctrunner",
            "AppProtected",
        ),
        ("", "AppIdentityMissing"),
        ("-k", "AppIdentityInvalid"),
        ("com.example app", "AppIdentityInvalid"),
        ("com.example;reboot", "AppIdentityInvalid"),
    ] {
        let error =
            removal_package_id(&library_item(AppLibraryPlatform::Android, id)).expect_err(id);
        assert_eq!(error.code, code, "{id}");
    }
    // A name that merely starts with a protected root is someone else's app.
    assert!(removal_package_id(&library_item(
        AppLibraryPlatform::Android,
        "com.riviuhub.app"
    ))
    .is_ok());
}

#[tokio::test]
async fn a_phone_held_by_other_work_is_refused_without_waiting_or_dispatching() {
    let driver = RemovalDriver::with_app();
    let (db, control) = fixture(driver.clone());
    let nurture = control
        .try_acquire_exclusive("phone", DeviceWorkOwner::Nurture)
        .await
        .unwrap();
    let item = library_item(AppLibraryPlatform::Android, PACKAGE);
    for mode in [AppRemovalMode::Uninstall, AppRemovalMode::Reinstall] {
        let result = run(&control, &db, &item, mode).await;
        assert_eq!(result.outcome, AppRemovalOutcome::RefusedBusy);
        assert!(!result.removed_verified);
    }
    assert!(driver.calls.lock().is_empty());
    assert_eq!(
        control.current_work_owner("phone"),
        Some(DeviceWorkOwner::Nurture)
    );
    drop(nurture);
}

#[tokio::test]
async fn a_post_waiting_for_its_link_keeps_its_app() {
    let driver = RemovalDriver::with_app();
    let (db, control) = fixture(driver.clone());
    pending_publish(&db, "phone");
    let item = library_item(AppLibraryPlatform::Android, PACKAGE);
    let result = run(&control, &db, &item, AppRemovalMode::Uninstall).await;
    assert_eq!(result.outcome, AppRemovalOutcome::RefusedBusy);
    assert!(driver.effects().is_empty());
    assert!(control.current_work_owner("phone").is_none());
}

#[tokio::test]
async fn an_unreadable_app_list_refuses_before_the_uninstall_is_sent() {
    let driver = RemovalDriver::with_app();
    driver.list_fails.store(true, Ordering::SeqCst);
    let (db, control) = fixture(driver.clone());
    let item = library_item(AppLibraryPlatform::Android, PACKAGE);
    let result = run(&control, &db, &item, AppRemovalMode::Uninstall).await;
    assert_eq!(result.outcome, AppRemovalOutcome::FailedBeforeEffect);
    assert!(driver.effects().is_empty());
}

#[tokio::test]
async fn uninstall_is_sent_once_and_done_only_when_the_readback_no_longer_lists_the_app() {
    let driver = RemovalDriver::with_app();
    let (db, control) = fixture(driver.clone());
    let item = library_item(AppLibraryPlatform::Android, PACKAGE);
    let result = run(&control, &db, &item, AppRemovalMode::Uninstall).await;
    assert_eq!(result.outcome, AppRemovalOutcome::Done);
    assert!(result.removed_verified);
    assert!(result.install.is_none());
    assert_eq!(driver.effects(), vec![format!("uninstall {PACKAGE}")]);
    assert!(control.current_work_owner("phone").is_none());
    assert!(db
        .list_op_logs(10)
        .unwrap()
        .iter()
        .any(|entry| entry.action == "app.uninstall.dispatch"));

    // Already absent: nothing to send, and saying so is still a verified end state.
    let again = run(&control, &db, &item, AppRemovalMode::Uninstall).await;
    assert_eq!(again.outcome, AppRemovalOutcome::Done);
    assert!(again.removed_verified);
    assert_eq!(driver.effects().len(), 1);
}

#[tokio::test]
async fn an_unconfirmed_uninstall_is_unknown_and_never_sent_twice() {
    for ignored in [false, true] {
        let driver = RemovalDriver::with_app();
        if ignored {
            driver.uninstall_is_ignored.store(true, Ordering::SeqCst);
        } else {
            driver.uninstall_fails.store(true, Ordering::SeqCst);
        }
        let (db, control) = fixture(driver.clone());
        let item = library_item(AppLibraryPlatform::Ios, PACKAGE);
        let result = run(&control, &db, &item, AppRemovalMode::Reinstall).await;
        assert_eq!(result.outcome, AppRemovalOutcome::UnknownAfterDispatch);
        assert!(!result.removed_verified);
        assert!(
            result.install.is_none(),
            "no install after an unproven uninstall"
        );
        assert_eq!(driver.effects(), vec![format!("uninstall {PACKAGE}")]);
    }
}

#[tokio::test]
async fn reinstall_installs_the_library_artifact_only_after_the_app_is_proven_gone() {
    let driver = RemovalDriver::with_app();
    let (db, control) = fixture(driver.clone());
    let item = library_item(AppLibraryPlatform::Ios, PACKAGE);
    let result = run(&control, &db, &item, AppRemovalMode::Reinstall).await;
    assert_eq!(result.outcome, AppRemovalOutcome::Done);
    assert!(result.removed_verified);
    assert_eq!(
        result.install.as_ref().map(|install| &install.status),
        Some(&AppInstallStatus::Succeeded)
    );
    assert_eq!(
        driver.effects(),
        vec![
            format!("uninstall {PACKAGE}"),
            format!("install {}", item.path),
        ]
    );
    assert!(control.current_work_owner("phone").is_none());
}
