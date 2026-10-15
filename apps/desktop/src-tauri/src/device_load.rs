//! Background load monitor ("sức khỏe máy"): samples phones that hold a running job about
//! once a minute, and drives an idle phone marked overloaded through the tiered response.
//!
//! A child of `state`, ticked from the existing roster scan, so it adds no controller. Every
//! phone action goes through the control plane's lease and command admission; the job-time
//! gate itself is `DeviceControlPlane::ensure_load_admissible` at session start.
//!
//! - Running job: sample only (read-only). A phone that crosses a threshold is marked; it is
//!   never cleaned while an effect step may be running.
//! - Idle and unmarked: one read-only sample about every 10 minutes, a few phones at a time,
//!   so an overloaded phone is marked (and handled below) before a job claims it.
//! - Idle and marked: fresh sample; tier 1 (light cleanup) under a Repair lease; then tier 2
//!   (reboot) only when the durable ledger shows nothing uncertain or in flight on the phone,
//!   no job holds it and the 6 h budget allows; the budget is read back from the durable
//!   activity log, so a controller restart does not renew it. Otherwise "quá tải — chờ người
//!   vận hành".
//! - A job's own load gate: [`reboot_for_job`] is the driver's tier 2 inside the job's turn,
//!   under its exclusive lease and before any of its effects. It is not a background worker,
//!   so it also runs while acceptance mode freezes this monitor (12/10/2026: frozen in every
//!   r91/r93 controller, so no overloaded phone was ever rebooted).
//!
//! Publish work that only waits for the phone (queued, due, transferring, imported) blocks
//! neither tier: it has sent nothing to TikTok, and its job waits out the reboot.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use riviu_android_driver::AndroidDriver;
use riviu_core::db::Database;
use riviu_core::device_control::load::{
    DeviceLoadStatus, LoadVerdict, BACKGROUND_SAMPLE_PERIOD_MS, REBOOT_BUDGET_WINDOW_MS,
};
use riviu_core::{DeviceControlPlane, DeviceInfo, DevicePlatform, DeviceStatus, DeviceWorkOwner};
use tokio::task::{Id, JoinSet};
use tokio::time::{Duration, Instant};

use super::CommandAdmissionState;

/// How long after a reboot the monitor keeps sampling for the fresh verdict.
const POST_REBOOT_SAMPLE_WINDOW: Duration = Duration::from_secs(60);
/// Let a cleaned phone settle before the fresh sample decides.
const SETTLE_AFTER_CLEANUP: Duration = Duration::from_secs(3);
/// How often an idle phone nobody has marked gets one read-only sample, so an overloaded
/// phone is known (and cleaned while idle) before a job claims it. Not measured: 30 phones
/// at one shell each per 10 min is ~3 shells a minute beside the ~160 an idle fleet runs
/// (r111, `helper_setup.rs`).
const IDLE_SAMPLE_PERIOD: Duration = Duration::from_secs(10 * 60);
/// Idle samples in flight at once. Not measured: keeps the sweep a trickle on the adb server.
const MAX_IDLE_SAMPLES: usize = 3;

pub(super) struct DeviceLoadMonitor {
    control: Arc<DeviceControlPlane>,
    android: Arc<AndroidDriver>,
    admission: Arc<CommandAdmissionState>,
    db: Arc<Database>,
    last_visit: HashMap<String, Instant>,
    tasks: JoinSet<()>,
    running: HashMap<Id, String>,
    /// The tasks in `running` that are idle samples.
    idle_samples: HashSet<Id>,
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
            idle_samples: HashSet::new(),
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
            self.idle_samples.remove(&id);
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
            let visited = self.last_visit.get(udid).map(Instant::elapsed);
            if visited.is_some_and(|elapsed| elapsed < period) {
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
            let idle_sample = owner.is_none()
                && !marked
                && visited.is_none_or(|elapsed| elapsed >= IDLE_SAMPLE_PERIOD)
                && self.idle_samples.len() < MAX_IDLE_SAMPLES;
            if !job && !idle_and_marked && !idle_sample {
                continue;
            }
            self.last_visit.insert(udid.to_string(), Instant::now());
            let udid = udid.to_string();
            let android = self.android.clone();
            let handle = if job || idle_sample {
                let task_udid = udid.clone();
                let handle = self.tasks.spawn(async move {
                    // Read-only: marks the phone if it crosses a threshold, never cleans it.
                    // An idle phone marked here takes the idle tier on a later tick.
                    if let Err(error) = android.sample_load(&task_udid, None).await {
                        log::debug!("device load sample {task_udid}: {error:#}");
                    }
                });
                if idle_sample {
                    self.idle_samples.insert(handle.id());
                }
                handle
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
        self.idle_samples.clear();
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
    if let Some(spent) = reboot_budget_blocker(android, db, udid).await {
        return Some(spent);
    }
    ledger_blocker(db, udid, false).await
}

/// The reason this phone's 6 h reboot budget allows no reboot now, or `None`. This process's
/// own record first, then the durable activity log, so a controller restart grants no new
/// reboot; an unreadable log grants none either. Load and language reboots share it.
pub(super) async fn reboot_budget_blocker(
    android: &AndroidDriver,
    db: &Arc<Database>,
    udid: &str,
) -> Option<String> {
    const SPENT: &str = "đã tự khởi động lại một lần trong 6 giờ qua";
    if !android.load_reboot_budget_left(udid) {
        return Some(SPENT.into());
    }
    let serial = udid.to_string();
    let since = chrono::Utc::now() - chrono::Duration::milliseconds(REBOOT_BUDGET_WINDOW_MS);
    match db
        .storage_read(move |db| db.phone_rebooted_since(&serial, since))
        .await
    {
        Ok(false) => None,
        Ok(true) => Some(format!("{SPENT} (theo nhật ký thao tác)")),
        Err(error) => Some(format!(
            "không đọc được nhật ký khởi động lại trong 6 giờ qua ({error:#})"
        )),
    }
}

/// The durable ledger's reason this phone still has work, or `None`. `job_holds_phone`: the
/// caller is a job at its load gate, under its own exclusive lease.
async fn ledger_blocker(db: &Arc<Database>, udid: &str, job_holds_phone: bool) -> Option<String> {
    let ledger_udid = udid.to_string();
    // Conservative: any uncertain or in-flight Post/Send/Like/Comment/Follow, any live
    // nurture/interaction/Flow/job/orchestration work on this phone blocks, and an unreadable
    // ledger blocks too. Publish work that has not reached its Post intent does not.
    match db
        .storage_read(move |db| db.load_reboot_block_reason(&ledger_udid, job_holds_phone))
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
    if let Some(blocker) = ledger_blocker(db, udid, false).await {
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
    // A phone stuck `offline` gets this reboot's one per-device reconnect (13/10/2026).
    android.wait_for_adb_online_after_reboot(udid).await;
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

/// Tier 2 inside a job's turn, installed as the driver's reboot repair (see the module
/// docs). The caller holds the job's exclusive lease and has run none of its effects. Reboots
/// only when the 6 h budget and the ledger allow: anything uncertain or in flight on the phone
/// still blocks; publish work waiting for this phone does not. `Err` is the Vietnamese reason.
pub(super) async fn reboot_for_job(
    android: &AndroidDriver,
    db: &Arc<Database>,
    udid: &str,
    why: &str,
) -> Result<(), String> {
    if let Some(spent) = reboot_budget_blocker(android, db, udid).await {
        return Err(spent);
    }
    if let Some(blocker) = ledger_blocker(db, udid, true).await {
        return Err(blocker);
    }
    let reboot = android
        .issue_load_reboot(udid, why)
        .await
        .map_err(|error| format!("khởi động lại thất bại: {error:#}"))?;
    android
        .wait_for_load_reboot(udid, &reboot)
        .await
        .map_err(|error| format!("{error:#}"))?;
    // 13/10/2026: an SM-G955F (Android 9) finished booting, then read `offline` from ~10 s
    // later until a manual reconnect. Each wait spends the reboot's one per-device reconnect if
    // the phone stays offline past the cap; it returns at once for a phone that is online.
    android.wait_for_adb_online_after_reboot(udid).await;
    reapply_baseline(android, db, udid).await;
    // The helper process ended with the boot, and automatic helper setup may be frozen. Bring
    // it back now so a composer opened right after the gate finds it; the transfer step still
    // qualifies it, and a failure here is left to the job's own step. A phone that dropped off
    // adb meanwhile gets one more try once it is back.
    let mut prepared = android.prepare_helper_runtime(udid).await;
    if prepared
        .as_ref()
        .is_err_and(riviu_android_driver::adb::is_transport_lost)
        && android.wait_for_adb_online_after_reboot(udid).await
    {
        reapply_baseline(android, db, udid).await;
        prepared = android.prepare_helper_runtime(udid).await;
    }
    if let Err(error) = prepared {
        log::warn!("helper not ready after load reboot of {udid}: {error:#}");
    }
    Ok(())
}

/// Re-apply the "Cài đặt máy" baseline after a reboot (auto-rotate comes back on), through the
/// same driver path as the page. The caller holds the phone's Repair lease or a job's lease.
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
