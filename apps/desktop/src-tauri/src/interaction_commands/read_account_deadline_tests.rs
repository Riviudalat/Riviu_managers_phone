use super::*;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

/// A phone whose interaction session never finishes opening: the r108 stall.
#[derive(Default)]
struct StalledOpenDriver {
    starts: AtomicUsize,
}

#[async_trait::async_trait]
impl riviu_core::DeviceDriver for StalledOpenDriver {
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
        anyhow::bail!("unexpected launch")
    }
    async fn terminate_app(
        &self,
        _: &str,
        _: &str,
    ) -> anyhow::Result<riviu_core::ProcessAbsenceProof> {
        anyhow::bail!("unexpected terminate")
    }
    async fn start_ui_session(&self, _: &str) -> anyhow::Result<Box<dyn riviu_core::UiSession>> {
        anyhow::bail!("unexpected manual session")
    }
    async fn ensure_stream(&self, _: &str) -> anyhow::Result<String> {
        anyhow::bail!("unexpected stream")
    }
    async fn reboot(&self, _: &str) -> anyhow::Result<()> {
        anyhow::bail!("unexpected reboot")
    }
    async fn prepare_device(&self, _: &str) -> anyhow::Result<()> {
        anyhow::bail!("unexpected prepare")
    }
    async fn resolve_tiktok_package(&self, _: &str) -> anyhow::Result<String> {
        Ok("com.zhiliaoapp.musically".into())
    }
    async fn confirm_interaction_stream_stopped(
        &self,
        _: &str,
    ) -> anyhow::Result<riviu_core::StreamHandoffProof> {
        Ok(riviu_core::StreamHandoffProof { generation: 0 })
    }
    async fn start_interaction_session(
        &self,
        _: &str,
        _: &str,
        _: riviu_core::InteractionSessionKind,
    ) -> anyhow::Result<Box<dyn riviu_core::UiSession>> {
        self.starts.fetch_add(1, Ordering::SeqCst);
        std::future::pending().await
    }
}

/// r108 (13/10/2026): two account reads whose session never opened held both foreground
/// slots, every other read queued behind them, and none of those IPC calls ever answered.
/// Each read must now answer within its budget, release its phone, and let the queued one
/// through, while the rest of the control plane keeps answering.
#[tokio::test(start_paused = true)]
async fn a_stalled_account_read_answers_within_its_budget_and_frees_the_slot_it_held() {
    let driver = Arc::new(StalledOpenDriver::default());
    let control = Arc::new(DeviceControlPlane::new(
        driver.clone(),
        Arc::new(riviu_core::DeviceWorkCoordinator::new()),
        Arc::new(riviu_core::StreamBudgetManager::new(1).unwrap()),
    ));
    let path = std::env::temp_dir().join(format!("read-account-{}.db", uuid::Uuid::new_v4()));
    let db = Arc::new(riviu_core::db::Database::open(&path).unwrap());
    let started = tokio::time::Instant::now();
    let reads: Vec<_> = ["phone-a", "phone-b"]
        .into_iter()
        .map(|udid| {
            let (control, db) = (control.clone(), db.clone());
            tokio::spawn(async move {
                let result = inspection::read_account(&control, &db, udid).await;
                (result, started.elapsed())
            })
        })
        .collect();
    while driver.starts.load(Ordering::SeqCst) == 0 {
        tokio::task::yield_now().await;
    }
    // Another command is not held up by the stalled open.
    tokio::time::timeout(Duration::from_secs(1), control.list_devices())
        .await
        .expect("the control plane answers while a read is stalled")
        .unwrap();

    let ceiling =
        inspection::ACCOUNT_READ_QUEUE_BUDGET + inspection::ACCOUNT_READ_DEVICE_BUDGET * 3;
    for read in reads {
        let (result, elapsed) = tokio::time::timeout(ceiling, read)
            .await
            .expect("a stalled account read must answer, not wait forever")
            .unwrap();
        let error = result.expect_err("nothing was read");
        assert!(
            error.to_string().contains("đã nhả máy, chưa đọc được nick"),
            "{error:#}"
        );
        // The second read waited only for the first one's budget, then spent its own.
        assert!(
            elapsed <= inspection::ACCOUNT_READ_DEVICE_BUDGET * 2 + Duration::from_secs(5),
            "{elapsed:?}"
        );
    }
    assert_eq!(driver.starts.load(Ordering::SeqCst), 2);
    for udid in ["phone-a", "phone-b"] {
        control
            .try_acquire_exclusive(udid, riviu_core::DeviceWorkOwner::ManualControl)
            .await
            .expect("the timed-out read released its lease");
    }
    drop(db);
    let _ = std::fs::remove_file(path);
}
