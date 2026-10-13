use super::*;
use parking_lot::Mutex;
use std::path::{Path, PathBuf};

use riviu_core::db::publish_device_scope;
use riviu_core::{PublishCampaignState, PublishCleanupPolicy, PublishMediaLeftovers};

#[derive(Default)]
struct SweepDriver {
    leftovers: Mutex<PublishMediaLeftovers>,
    deleted: Mutex<Vec<(PublishMediaSweepKind, String)>>,
    fail_names: Mutex<Vec<String>>,
}

#[async_trait::async_trait]
impl riviu_core::DeviceDriver for SweepDriver {
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
        panic!("the sweep must not launch TikTok")
    }
    async fn terminate_app(
        &self,
        _: &str,
        _: &str,
    ) -> anyhow::Result<riviu_core::ProcessAbsenceProof> {
        panic!("the sweep must not terminate TikTok")
    }
    async fn start_ui_session(&self, _: &str) -> anyhow::Result<Box<dyn riviu_core::UiSession>> {
        panic!("the sweep must not open a session")
    }
    async fn ensure_stream(&self, _: &str) -> anyhow::Result<String> {
        panic!("the sweep must not restart the preview")
    }
    async fn reboot(&self, _: &str) -> anyhow::Result<()> {
        panic!("the sweep must not reboot")
    }
    async fn prepare_device(&self, _: &str) -> anyhow::Result<()> {
        panic!("the sweep must not prepare the device")
    }
    async fn list_publish_media_leftovers(&self, _: &str) -> anyhow::Result<PublishMediaLeftovers> {
        Ok(self.leftovers.lock().clone())
    }
    async fn cleanup_publish_media(
        &self,
        _: &str,
        import: &str,
    ) -> anyhow::Result<serde_json::Value> {
        self.deleted
            .lock()
            .push((PublishMediaSweepKind::Album, import.to_owned()));
        if self.fail_names.lock().iter().any(|name| name == import) {
            anyhow::bail!("fixture: {import} is busy");
        }
        self.leftovers.lock().albums.retain(|name| name != import);
        Ok(serde_json::json!({"state":"cleaned","importId":import}))
    }
    async fn cleanup_publish_stage(
        &self,
        _: &str,
        scope: &str,
    ) -> anyhow::Result<serde_json::Value> {
        self.deleted
            .lock()
            .push((PublishMediaSweepKind::Stage, scope.to_owned()));
        self.leftovers.lock().stages.retain(|name| name != scope);
        Ok(serde_json::json!({"state":"cleaned","stageScope":scope}))
    }
}

fn control(driver: Arc<SweepDriver>) -> DeviceControlPlane {
    DeviceControlPlane::new(
        driver,
        Arc::new(riviu_core::DeviceWorkCoordinator::new()),
        Arc::new(riviu_core::StreamBudgetManager::new(1).unwrap()),
    )
}

/// One campaign with one assignment on `phone`, left in `state`. Returns its scope.
fn seed(db: &Database, state: PublishCampaignState, run_at: Option<&str>) -> String {
    let id = uuid::Uuid::new_v4().to_string();
    let bundle = riviu_core::PublishBundle {
        id: id.clone(),
        source_path: "C:/fixture".into(),
        name: id.clone(),
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
        request_id: id.clone(),
        source_root: "C:/fixture".into(),
        bundle_ids: vec![id],
        udids: vec!["phone".into()],
        run_at: run_at.map(str::to_owned),
        visibility: riviu_core::PublishVisibility::Public,
        cleanup_policy: PublishCleanupPolicy::DeleteImportedAssetsAfterVerified,
        network: riviu_core::SocialNetwork::TikTok,
        sound_policy: riviu_core::PublishSoundPolicy::Default,
        execution_confirmed: true,
        target_snapshot: None,
        approval: None,
    };
    let campaign = db.create_publish_campaign(&request, &[bundle]).unwrap();
    let assignment = db
        .get_publish_campaign(&campaign.id)
        .unwrap()
        .unwrap()
        .assignments[0]
        .clone();
    db.update_publish_assignment_state(&assignment.id, state, None, None)
        .unwrap();
    publish_device_scope(&campaign.id, assignment.ordinal)
}

fn fixture() -> (Database, PathBuf) {
    let path = std::env::temp_dir().join(format!("media-sweep-{}.db", uuid::Uuid::new_v4()));
    (Database::open(&path).unwrap(), path)
}

fn album(scope: &str) -> String {
    format!("riviu-{scope}-aaaaaaaaaaaa")
}

#[tokio::test]
async fn sweep_deletes_only_scopes_without_a_living_assignment_and_records_each_effect() {
    let (db, path) = fixture();
    let failed = seed(&db, PublishCampaignState::FailedBeforeDispatch, None);
    let cancelled = seed(&db, PublishCampaignState::Cancelled, None);
    // Scheduled for 2099: a living assignment that does not make the phone busy today.
    let scheduled = seed(
        &db,
        PublishCampaignState::Scheduled,
        Some("2099-01-01T00:00:00"),
    );
    let driver = Arc::new(SweepDriver::default());
    *driver.leftovers.lock() = PublishMediaLeftovers {
        albums: vec![
            album(&failed),
            album(&scheduled),
            album("unknown-campaign-3"),
        ],
        stages: vec![cancelled.clone(), scheduled.clone(), "lost-stage-9".into()],
    };
    let control = control(driver.clone());

    let outcome = sweep_orphan_publish_media(&control, &db, "phone", 3)
        .await
        .unwrap();
    assert_eq!(outcome.skipped, None);
    assert_eq!((outcome.albums_listed, outcome.stages_listed), (3, 3));
    assert_eq!(
        (outcome.deleted, outcome.failed, outcome.remaining),
        (3, 0, 1)
    );
    assert_eq!(
        *driver.deleted.lock(),
        vec![
            (PublishMediaSweepKind::Album, album(&failed)),
            (PublishMediaSweepKind::Album, album("unknown-campaign-3")),
            (PublishMediaSweepKind::Stage, cancelled.clone()),
        ],
        "albums first, the living scope untouched, the cap honoured"
    );
    assert_eq!(control.current_work_owner("phone"), None);

    let outcome = sweep_orphan_publish_media(&control, &db, "phone", DELETIONS_PER_VISIT)
        .await
        .unwrap();
    assert_eq!((outcome.deleted, outcome.remaining), (1, 0));
    assert_eq!(driver.deleted.lock().last().unwrap().1, "lost-stage-9");
    for name in [album(&scheduled), scheduled.clone()] {
        assert!(
            !driver
                .deleted
                .lock()
                .iter()
                .any(|(_, deleted)| *deleted == name),
            "{name} belongs to a scheduled assignment"
        );
    }

    let rows: Vec<(String, String, String)> = rusqlite::Connection::open(&path)
        .unwrap()
        .prepare("SELECT kind,name,state FROM publish_media_sweeps ORDER BY claimed_at_ms,rowid")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(rows.len(), 4);
    assert!(rows.iter().all(|(_, _, state)| state == "cleaned"));
    assert_eq!(rows[0], ("album".into(), album(&failed), "cleaned".into()));
    drop(db);
    let _ = std::fs::remove_file(path);
}

#[tokio::test]
async fn sweep_never_touches_a_busy_or_pending_phone() {
    let (db, path) = fixture();
    let failed = seed(&db, PublishCampaignState::FailedBeforeDispatch, None);
    let driver = Arc::new(SweepDriver::default());
    *driver.leftovers.lock() = PublishMediaLeftovers {
        albums: vec![album(&failed)],
        stages: vec![],
    };
    let control = control(driver.clone());

    let busy = control
        .try_acquire_exclusive_keeping_stream("phone", DeviceWorkOwner::ManualControl)
        .await
        .unwrap();
    let outcome = sweep_orphan_publish_media(&control, &db, "phone", DELETIONS_PER_VISIT)
        .await
        .unwrap();
    assert!(outcome.skipped.is_some());
    control.close_exclusive_context(busy).unwrap();

    // A post still verifying on this phone keeps every sweep off it, orphans included.
    let _verifying = seed(&db, PublishCampaignState::Verifying, None);
    let outcome = sweep_orphan_publish_media(&control, &db, "phone", DELETIONS_PER_VISIT)
        .await
        .unwrap();
    assert!(outcome.skipped.is_some());
    assert!(driver.deleted.lock().is_empty());
    assert_eq!(control.current_work_owner("phone"), None);
    let rows: i64 = rusqlite::Connection::open(&path)
        .unwrap()
        .query_row("SELECT COUNT(*) FROM publish_media_sweeps", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(rows, 0, "a skipped visit writes no intent");
    drop(db);
    let _ = std::fs::remove_file(path);
}

#[tokio::test]
async fn a_failed_deletion_is_receipted_and_backed_off_until_tomorrow() {
    let (db, path) = fixture();
    let failed = seed(&db, PublishCampaignState::FailedBeforeDispatch, None);
    let driver = Arc::new(SweepDriver::default());
    *driver.leftovers.lock() = PublishMediaLeftovers {
        albums: vec![album(&failed)],
        stages: vec![],
    };
    driver.fail_names.lock().push(album(&failed));
    let control = control(driver.clone());

    let outcome = sweep_orphan_publish_media(&control, &db, "phone", DELETIONS_PER_VISIT)
        .await
        .unwrap();
    assert_eq!((outcome.deleted, outcome.failed), (0, 1));
    let outcome = sweep_orphan_publish_media(&control, &db, "phone", DELETIONS_PER_VISIT)
        .await
        .unwrap();
    assert_eq!(
        (outcome.deleted, outcome.failed, outcome.remaining),
        (0, 0, 0)
    );
    assert_eq!(driver.deleted.lock().len(), 1, "backed off, not retried");
    let (state, proof): (String, String) = rusqlite::Connection::open(&path)
        .unwrap()
        .query_row(
            "SELECT state,proof_json FROM publish_media_sweeps",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(state, "failed");
    assert!(proof.contains("is busy"));
    drop(db);
    let _ = std::fs::remove_file(path);
}

#[test]
fn a_cleaned_readback_must_name_the_same_directory() {
    let claim = PublishMediaSweepClaim {
        id: "x".into(),
        udid: "phone".into(),
        kind: PublishMediaSweepKind::Album,
        name: "riviu-a-aaaaaaaaaaaa".into(),
    };
    assert!(
        sweep_receipt(
            &claim,
            Ok(serde_json::json!({"state":"cleaned","importId":"riviu-a-aaaaaaaaaaaa"}))
        )
        .0
    );
    assert!(
        sweep_receipt(
            &claim,
            Ok(serde_json::json!({"value":{"state":"cleaned","importId":"riviu-a-aaaaaaaaaaaa"}}))
        )
        .0
    );
    assert!(
        !sweep_receipt(
            &claim,
            Ok(serde_json::json!({"state":"cleaned","importId":"riviu-b-bbbbbbbbbbbb"}))
        )
        .0
    );
    assert!(!sweep_receipt(&claim, Ok(serde_json::json!({"state":"cleaned"}))).0);
    let (cleaned, proof) = sweep_receipt(&claim, Err("lost".into()));
    assert!(!cleaned);
    assert_eq!(proof["message"], "lost");
}

#[test]
fn orphan_selection_skips_living_scopes_backoff_and_unparseable_names() {
    let leftovers = PublishMediaLeftovers {
        albums: vec![
            "riviu-live-1-aaaaaaaaaaaa".into(),
            "riviu-gone-2-bbbbbbbbbbbb".into(),
            "riviu-slow-3-cccccccccccc".into(),
            "riviu-notanid".into(),
        ],
        stages: vec!["live-1".into(), "gone-4".into(), "slow-3".into()],
    };
    let live = BTreeSet::from(["live-1".to_string()]);
    let backoff = BTreeSet::from([
        "riviu-slow-3-cccccccccccc".to_string(),
        "slow-3".to_string(),
    ]);
    assert_eq!(
        orphan_names(&leftovers, &live, &backoff),
        vec![
            (
                PublishMediaSweepKind::Album,
                "riviu-gone-2-bbbbbbbbbbbb".to_string()
            ),
            (PublishMediaSweepKind::Stage, "gone-4".to_string()),
        ]
    );
}
