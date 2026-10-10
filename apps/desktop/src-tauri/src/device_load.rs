//! Background load monitor ("sức khỏe máy"): samples phones that hold a running job about
//! once a minute, and drives an idle phone marked overloaded through the tiered response.
//!
//! A child of `state`, ticked from the existing roster scan, so it adds no controller. Every
//! phone action goes through the control plane's lease and command admission; the job-time
//! gate itself is `DeviceControlPlane::ensure_load_admissible` at session start.
//!
//! - Running job: sample only (read-only). A phone that crosses a threshold is marked; it is
//!   never cleaned while an effect step may be running.
//! - Idle and marked: fresh sample; tier 1 (light cleanup) under a Repair lease; then tier 2
//!   (reboot) only when the durable ledger shows nothing uncertain or in flight on the phone,
//!   no job holds it and the 6 h budget allows. Otherwise "quá tải — chờ người vận hành".

use std::collections::HashMap;
use std::sync::Arc;

use riviu_android_driver::AndroidDriver;
use riviu_core::db::Database;
use riviu_core::device_control::load::{
    DeviceLoadStatus, LoadVerdict, BACKGROUND_SAMPLE_PERIOD_MS,
};
use riviu_core::{DeviceControlPlane, DeviceInfo, DevicePlatform, DeviceStatus, DeviceWorkOwner};
use tokio::task::{Id, JoinSet};
use tokio::time::{Duration, Instant};

use super::CommandAdmissionState;

/// How long after a reboot the monitor keeps sampling for the fresh verdict.
const POST_REBOOT_SAMPLE_WINDOW: Duration = Duration::from_secs(60);
/// Let a cleaned phone settle before the fresh sample decides.
const SETTLE_AFTER_CLEANUP: Duration = Duration::from_secs(3);

pub(super) struct DeviceLoadMonitor {
    control: Arc<DeviceControlPlane>,
    android: Arc<AndroidDriver>,
    admission: Arc<CommandAdmissionState>,
    db: Arc<Database>,
    last_visit: HashMap<String, Instant>,
    tasks: JoinSet<()>,
    running: HashMap<Id, String>,
    stopped: bool,
}

impl DeviceLoadMonitor {
    pub(super) fn new(
        control: Arc<DeviceControlPlane>,
        android: Arc<AndroidDriver>,
        admission: Arc<CommandAdmissionState>,
        db: Arc<Database>,
    ) -> Self {
        Self {
            control,
            android,
            admission,
            db,
            last_visit: HashMap::new(),
            tasks: JoinSet::new(),
            running: HashMap::new(),
            stopped: false,
        }
    }

    pub(super) fn tick(&mut self, devices: &[DeviceInfo]) {
        if self.stopped {
            return;
        }
        while let Some(result) = self.tasks.try_join_next_with_id() {
            let id = match &result {
                Ok((id, ())) => *id,
                Err(error) => error.id(),
            };
            self.running.remove(&id);
            if let Err(error) = result {
                if !error.is_cancelled() {
                    log::error!("device load task failed: {error}");
                }
            }
        }
        let period = Duration::from_millis(BACKGROUND_SAMPLE_PERIOD_MS.unsigned_abs());
        let connected: Vec<&DeviceInfo> = devices
            .iter()
            .filter(|device| {
                device.platform == DevicePlatform::Android
                    && !matches!(
                        device.status,
                        DeviceStatus::Disconnected | DeviceStatus::Pairing
                    )
            })
            .collect();
        self.last_visit
            .retain(|udid, _| connected.iter().any(|device| &device.udid == udid));
        for device in connected {
            let udid = device.udid.as_str();
            if self.running.values().any(|running| running == udid) {
                continue;
            }
            if self
                .last_visit
                .get(udid)
                .is_some_and(|at| at.elapsed() < period)
            {
                continue;
            }
            let owner = self.control.current_work_owner(udid);
            let job = matches!(
                owner,
                Some(
                    DeviceWorkOwner::Script
                        | DeviceWorkOwner::Interaction
                        | DeviceWorkOwner::Nurture
                )
            );
            let marked = matches!(
                self.android.load_status(udid),
                Some(DeviceLoadStatus::Overloaded | DeviceLoadStatus::AwaitingOperator)
            );
            let idle_and_marked = owner.is_none() && marked;
            if !job && !idle_and_marked {
                continue;
            }
            self.last_visit.insert(udid.to_string(), Instant::now());
            let udid = udid.to_string();
            let android = self.android.clone();
            let handle = if job {
                let task_udid = udid.clone();
                self.tasks.spawn(async move {
                    // Read-only: marks the phone if it crosses a threshold, never cleans it.
                    if let Err(error) = android.sample_load(&task_udid, None).await {
                        log::debug!("device load sample {task_udid}: {error:#}");
                    }
                })
            } else {
                let control = self.control.clone();
                let admission = self.admission.clone();
                let db = self.db.clone();
                let task_udid = udid.clone();
                self.tasks.spawn(async move {
                    recover_idle(&control, &android, &admission, &db, &task_udid).await;
                })
            };
            self.running.insert(handle.id(), udid);
        }
    }

    /// Shutdown: abort in-flight work. A sample is read-only; a cleanup or reboot already
    /// journaled its intent, and an aborted reboot wait leaves the phone to reconnect normally.
    pub(super) async fn drain(&mut self) {
        self.stopped = true;
        self.tasks.abort_all();
        while self.tasks.join_next().await.is_some() {}
        self.running.clear();
    }
}

/// The durable reason a reboot is not allowed on this phone now, or `None`.
async fn reboot_blocker(
    control: &DeviceControlPlane,
    android: &AndroidDriver,
    db: &Arc<Database>,
    udid: &str,
) -> Option<String> {
    if let Some(owner) = control.current_work_owner(udid) {
        if owner != DeviceWorkOwner::Repair {
            return Some(format!("máy đang được {owner:?} giữ"));
        }
    }
    if !android.load_reboot_budget_left(udid) {
        return Some("đã tự khởi động lại một lần trong 6 giờ qua".into());
    }
    ledger_blocker(db, udid).await
}

/// The durable ledger's reason this phone still has work, or `None`.
async fn ledger_blocker(db: &Arc<Database>, udid: &str) -> Option<String> {
    let ledger_udid = udid.to_string();
    // Conservative: any uncertain, in-flight or due Post/Send/Like/Comment/Follow/Flow/job
    // on this phone blocks, and an unreadable ledger blocks too.
    match db
        .storage_read(move |db| db.app_completion_block_reason(&ledger_udid))
        .await
    {
        Ok(None) => None,
        Ok(Some(reason)) => Some(format!("sổ ghi còn việc chưa rõ hoặc đang chạy ({reason})")),
        Err(error) => Some(format!("không đọc được sổ ghi thao tác ({error:#})")),
    }
}

async fn recover_idle(
    control: &DeviceControlPlane,
    android: &AndroidDriver,
    admission: &Arc<CommandAdmissionState>,
    db: &Arc<Database>,
    udid: &str,
) {
    let verdict = match android.sample_load_confirmed(udid, None).await {
        Ok((_, verdict)) => verdict,
        Err(error) => {
            log::debug!("device load sample {udid}: {error:#}");
            return;
        }
    };
    // Healthy: `record` already cleared the mark and wrote the line.
    let LoadVerdict::Overloaded { reasons } = verdict else {
        return;
    };
    let mut why = reasons.join("; ");

    // Tier 1, under a Repair lease so no job step is running. Conservative while idle: a phone
    // whose ledger still has uncertain or in-flight work (a post still uploading in the
    // background) is not cleaned either.
    if let Some(blocker) = ledger_blocker(db, udid).await {
        android.mark_load_awaiting_operator(
            udid,
            &format!("{why}; không tự dọn hay khởi động lại vì {blocker}"),
        );
        return;
    }
    if android.load_cleanup_allowed(udid) {
        let Ok(_admission) = admission.ensure_accepting_work() else {
            return;
        };
        let Ok(_lease) = control
            .try_acquire_exclusive_keeping_stream(udid, DeviceWorkOwner::Repair)
            .await
        else {
            return;
        };
        if android.light_cleanup(udid, &why).await.is_ok() {
            tokio::time::sleep(SETTLE_AFTER_CLEANUP).await;
            match android.sample_load_confirmed(udid, None).await {
                Ok((_, LoadVerdict::Overloaded { reasons })) => why = reasons.join("; "),
                Ok(_) => return,
                Err(error) => {
                    log::debug!("device load re-sample {udid}: {error:#}");
                    return;
                }
            }
        }
    }

    // Tier 2.
    if let Some(blocker) = reboot_blocker(control, android, db, udid).await {
        android.mark_load_awaiting_operator(
            udid,
            &format!("{why}; không tự khởi động lại vì {blocker}"),
        );
        return;
    }
    let Ok(admitted) = admission.ensure_accepting_work() else {
        return;
    };
    let Ok(lease) = control
        .try_acquire_exclusive_keeping_stream(udid, DeviceWorkOwner::Repair)
        .await
    else {
        return;
    };
    // Re-check under the lease: a job may have committed work between the read and now.
    if let Some(blocker) = reboot_blocker(control, android, db, udid).await {
        android.mark_load_awaiting_operator(
            udid,
            &format!("{why}; không tự khởi động lại vì {blocker}"),
        );
        return;
    }
    let reboot = match android.issue_load_reboot(udid, &why).await {
        Ok(reboot) => reboot,
        Err(error) => {
            android.mark_load_awaiting_operator(
                udid,
                &format!("{why}; khởi động lại thất bại: {error:#}"),
            );
            return;
        }
    };
    // The Repair lease stays held through the wait and the baseline, so no job claims a phone
    // that is booting or has auto-rotate back on. Command admission is released: a shutdown
    // must not wait out a boot.
    drop(admitted);
    if let Err(error) = android.wait_for_load_reboot(udid, &reboot).await {
        drop(lease);
        android.mark_load_awaiting_operator(udid, &format!("{why}; {error:#}"));
        return;
    }
    reapply_baseline(android, db, udid).await;
    drop(lease);
    // Helper: the phone left adb and came back, so `AndroidHelperSetup` sees a new connection
    // and runs the existing preparation/maintenance flow once this lease is released. The
    // phone takes no job until a fresh sample passes: `note_reboot` cleared its last pass.
    let deadline = Instant::now() + POST_REBOOT_SAMPLE_WINDOW;
    while Instant::now() < deadline {
        match android.sample_load_confirmed(udid, None).await {
            Ok((_, LoadVerdict::Overloaded { reasons })) => {
                android.mark_load_awaiting_operator(
                    udid,
                    &format!("vẫn quá tải sau khi khởi động lại: {}", reasons.join("; ")),
                );
                return;
            }
            Ok((_, LoadVerdict::Healthy)) => return,
            _ => tokio::time::sleep(Duration::from_secs(5)).await,
        }
    }
}

/// Re-apply the "Cài đặt máy" baseline after a reboot (auto-rotate comes back on), through the
/// same driver path as the page. The caller holds the phone's Repair lease.
async fn reapply_baseline(android: &AndroidDriver, db: &Database, udid: &str) {
    use riviu_core::device_control::baseline::{
        summarize, BaselineOutcome, BaselineSetting, DeviceBaselineResult,
    };
    let settings = match db.get_device_baseline_config() {
        Ok(config) if !config.settings.is_empty() => config.settings,
        Ok(_) => BaselineSetting::DEFAULT_BASELINE.to_vec(),
        Err(error) => {
            log::warn!("baseline config unreadable after reboot of {udid}: {error:#}");
            BaselineSetting::DEFAULT_BASELINE.to_vec()
        }
    };
    let result = match android.apply_baseline(udid, &settings).await {
        Ok(items) => DeviceBaselineResult::from_items(udid, items),
        Err(error) => DeviceBaselineResult::whole_device(
            udid,
            BaselineOutcome::Failed,
            format!("không đọc được máy, chưa thay đổi gì: {error:#}"),
        ),
    };
    android.journal_load(
        udid,
        "device.load.baseline",
        &format!(
            "áp lại Cài đặt máy sau khi khởi động lại: {}",
            summarize(&result)
        ),
    );
}
