//! Phone load ("sức khỏe máy"): the read-only sample, the light cleanup and the per-phone book
//! the admission gate and the background monitor share. Thresholds and vocabulary live in
//! [`riviu_core::device_control::load`].
//!
//! The sample is one shell round trip with a deadline and never `uiautomator dump`: it must stay
//! cheap on exactly the phones that are struggling.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use riviu_core::device_control::load::{
    self, DeviceLoadSample, DeviceLoadState, DeviceLoadStatus, LoadVerdict,
};

/// The whole sample in one round trip. `-d 1` bounds toybox's first-frame delay; `;` and not
/// `&&` so a failing section leaves only its own marker empty.
pub const LOAD_SAMPLE_SCRIPT: &str = "echo '@@top'; top -b -n 1 -m 10 -d 1 2>/dev/null; \
echo '@@meminfo'; cat /proc/meminfo; \
echo '@@uptime'; cat /proc/uptime; \
echo '@@battery_temp'; cat /sys/class/power_supply/battery/temp 2>/dev/null; \
echo '@@end'";

/// The sample's own deadline, after its adb slot is granted.
pub const LOAD_SAMPLE_DEADLINE: Duration = Duration::from_secs(2);

/// Tier 1 deadline: `pm trim-caches` walks every app's cache.
pub const LIGHT_CLEANUP_DEADLINE: Duration = Duration::from_secs(30);

/// How long a phone gets to come back from `adb reboot` with `sys.boot_completed=1`.
pub const REBOOT_DEADLINE: Duration = Duration::from_secs(120);

/// Read during a reboot wait: boot completion, the boot id and uptime, one line each.
pub const BOOT_PROBE: &str = "getprop sys.boot_completed; cat /proc/sys/kernel/random/boot_id; \
                              cut -d' ' -f1 /proc/uptime";

pub const BOOT_PROBE_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Default, PartialEq)]
pub struct BootProbe {
    pub completed: bool,
    pub boot_id: Option<String>,
    pub uptime_secs: Option<f64>,
}

pub fn parse_boot_probe(stdout: &str) -> BootProbe {
    let mut lines = stdout.lines().map(str::trim);
    BootProbe {
        completed: lines.next() == Some("1"),
        boot_id: lines
            .next()
            .filter(|line| !line.is_empty())
            .map(str::to_string),
        uptime_secs: lines.next().and_then(|line| line.parse::<f64>().ok()),
    }
}

/// A reboot that was issued: what the phone's boot looked like before it.
#[derive(Debug, Clone)]
pub struct LoadReboot {
    pub boot_id_before: Option<String>,
    pub issued: std::time::Instant,
}

impl LoadReboot {
    /// A new boot that has completed: the boot id changed, or (when it could not be read
    /// before) the phone has been up for less time than has passed since the command.
    pub fn finished(&self, probe: &BootProbe) -> bool {
        let new_boot = match (&self.boot_id_before, &probe.boot_id) {
            (Some(before), Some(now)) => before != now,
            _ => probe
                .uptime_secs
                .is_some_and(|up| up < self.issued.elapsed().as_secs_f64() + 5.0),
        };
        probe.completed && new_boot
    }
}

/// Parse [`LOAD_SAMPLE_SCRIPT`] output. Missing sections leave their fields `None`.
pub fn parse_sample(stdout: &str, target_package: Option<&str>, now_ms: i64) -> DeviceLoadSample {
    let sections = crate::baseline::split_sections(stdout);
    let mut sample = DeviceLoadSample {
        sampled_at_ms: now_ms,
        target_package: target_package.map(str::to_string),
        ..DeviceLoadSample::default()
    };
    if let Some(top) = sections.get("top") {
        let parsed = parse_top(top, target_package);
        sample.surfaceflinger_cpu_percent = parsed.surfaceflinger;
        sample.target_app_cpu_percent = parsed.target_app;
        sample.cpu_idle_percent = parsed.idle;
    }
    if let Some(meminfo) = sections.get("meminfo") {
        sample.mem_available_kb = meminfo_kb(meminfo, "MemAvailable");
        sample.swap_total_kb = meminfo_kb(meminfo, "SwapTotal");
        sample.swap_free_kb = meminfo_kb(meminfo, "SwapFree");
    }
    sample.uptime_secs = sections
        .get("uptime")
        .and_then(|uptime| uptime.split_whitespace().next())
        .and_then(|value| value.parse::<f64>().ok())
        .filter(|value| value.is_finite() && *value >= 0.0);
    sample.battery_temp_tenths_c = sections
        .get("battery_temp")
        .and_then(|temp| temp.trim().parse::<i32>().ok())
        // -200 (-20.0 °C) is the "no reading" value some phones in the 2026-10-10 fleet sample
        // returned, so the range starts above it and that reading stays unknown.
        .filter(|temp| (-199..=1000).contains(temp));
    sample
}

#[derive(Debug, Default, PartialEq)]
struct TopReading {
    surfaceflinger: Option<f32>,
    target_app: Option<f32>,
    idle: Option<f32>,
}

/// toybox `top -b` (Android 8+): a `800%cpu ... 708%idle` summary line, then a header whose
/// sort column is bracketed (`S[%CPU]`), then rows. Older `top` prints `CPU%` and no summary.
fn parse_top(text: &str, target_package: Option<&str>) -> TopReading {
    let mut reading = TopReading::default();
    let mut header: Option<(usize, usize)> = None;
    let mut target_total: Option<f32> = None;
    for raw in text.lines() {
        let line = strip_escapes(raw);
        let tokens: Vec<&str> = line.split_whitespace().collect();
        if reading.idle.is_none() {
            let total = tokens.iter().find_map(|t| percent_with_suffix(t, "%cpu"));
            let idle = tokens.iter().find_map(|t| percent_with_suffix(t, "%idle"));
            if let (Some(total), Some(idle)) = (total, idle) {
                if total > 0.0 {
                    reading.idle = Some((idle / total * 100.0).clamp(0.0, 100.0));
                }
                continue;
            }
        }
        match header {
            None => {
                let normalized = line.replace(['[', ']'], " ");
                let columns: Vec<&str> = normalized.split_whitespace().collect();
                if !columns.contains(&"PID") {
                    continue;
                }
                let Some(cpu) = columns.iter().position(|c| *c == "%CPU" || *c == "CPU%") else {
                    continue;
                };
                let name = columns
                    .iter()
                    .position(|c| matches!(*c, "ARGS" | "NAME" | "Name" | "CMD" | "COMMAND"))
                    .unwrap_or(columns.len().saturating_sub(1));
                header = Some((cpu, name));
            }
            Some((cpu_index, name_index)) => {
                let Some(cpu) = tokens
                    .get(cpu_index)
                    .and_then(|value| value.trim_end_matches('%').parse::<f32>().ok())
                else {
                    continue;
                };
                let name = if name_index < tokens.len() {
                    tokens[name_index]
                } else {
                    match tokens.last() {
                        Some(last) => last,
                        None => continue,
                    }
                };
                let process = name.rsplit('/').next().unwrap_or(name);
                if process == "surfaceflinger" {
                    reading.surfaceflinger = Some(reading.surfaceflinger.unwrap_or(0.0).max(cpu));
                }
                if let Some(package) = target_package {
                    if process.split(':').next() == Some(package) {
                        target_total = Some(target_total.unwrap_or(0.0) + cpu);
                    }
                }
            }
        }
    }
    reading.target_app = target_total;
    reading
}

fn percent_with_suffix(token: &str, suffix: &str) -> Option<f32> {
    token.strip_suffix(suffix)?.parse::<f32>().ok()
}

fn strip_escapes(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            // CSI: ESC [ ... final byte in @..~
            if chars.next() == Some('[') {
                for c in chars.by_ref() {
                    if ('@'..='~').contains(&c) {
                        break;
                    }
                }
            }
            continue;
        }
        if c != '\r' {
            out.push(c);
        }
    }
    out
}

fn meminfo_kb(meminfo: &str, key: &str) -> Option<u64> {
    meminfo.lines().find_map(|line| {
        let rest = line.trim().strip_prefix(key)?.strip_prefix(':')?;
        rest.split_whitespace().next()?.parse::<u64>().ok()
    })
}

/// Tier 1 as one shell script: `am kill-all` (cached background processes only), `am force-stop`
/// for the reviewed hogs that pass [`load::is_protected_package`], `pm trim-caches 2G`. Never
/// `pm clear`: it logs the account out.
pub fn light_cleanup_script() -> String {
    let mut steps = vec!["am kill-all".to_string()];
    steps.extend(
        load::LIGHT_CLEANUP_FORCE_STOP
            .iter()
            .filter(|package| !load::is_protected_package(package))
            .filter(|package| crate::adb::validate_package_name(package).is_ok())
            .map(|package| format!("am force-stop {package}")),
    );
    steps.push("pm trim-caches 2G".to_string());
    steps.join("; ")
}

/// `(serial, action, Vietnamese message)` -> the desktop's durable activity log.
pub type LoadJournal = Arc<dyn Fn(&str, &str, &str) + Send + Sync>;

/// The desktop's tier 2 for a job's own load gate: `(serial, why)` reboots the phone when the
/// durable ledger and the 6 h budget allow, waits for the new boot and re-applies the "Cài đặt
/// máy" baseline. `Err` is the Vietnamese reason it did not reboot or the boot did not finish.
/// The caller holds the job's exclusive lease and no effect of the job has run.
pub type LoadRebootRepair = Arc<
    dyn Fn(String, String) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send>>
        + Send
        + Sync,
>;

/// How long after a gate reboot the gate keeps sampling for a decisive verdict.
pub const POST_REBOOT_SAMPLE_WINDOW: Duration = Duration::from_secs(60);

/// adb `offline` without a break for this long after a Riviu reboot earns the one per-device
/// `adb -s <serial> reconnect`. A cap, not measured: on 13/10/2026 an SM-G955F (Android 9)
/// stayed offline ~20 min after a load reboot until `adb reconnect offline` cleared it.
pub const OFFLINE_RECONNECT_AFTER_MS: i64 = 60 * 1000;

/// How long after a Riviu reboot an offline phone is still that reboot's business. Not
/// measured: the reboot deadline plus a margin; later offline is the cable or the operator's.
pub const OFFLINE_RECONNECT_WINDOW_MS: i64 = 15 * 60 * 1000;

/// Deadline for `adb -s <serial> reconnect`, a host-side transport reset that returns at once.
pub const OFFLINE_RECONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// How long the job's reboot path waits for the phone to show `device` again before helper
/// preparation: the offline cap, then the reconnect and a margin. Not measured.
pub const POST_REBOOT_ONLINE_WAIT: Duration = Duration::from_secs(100);

#[derive(Default)]
struct PhoneLoad {
    last: Option<DeviceLoadSample>,
    status: Option<DeviceLoadStatus>,
    reason: Option<String>,
    last_pass_ms: Option<i64>,
    last_cleanup_ms: Option<i64>,
    reboots_ms: Vec<i64>,
    /// First adb `offline` reading of the current unbroken run, after a Riviu reboot.
    offline_since_ms: Option<i64>,
    /// The latest reboot's one `adb reconnect` was spent.
    reconnect_spent: bool,
}

/// Per-phone load history shared by the admission gate and the background monitor. In memory:
/// the durable trail is the journal line each change writes.
#[derive(Default)]
pub struct LoadBook {
    phones: Mutex<HashMap<String, PhoneLoad>>,
    journal: Mutex<Option<LoadJournal>>,
    reboot_repair: Mutex<Option<LoadRebootRepair>>,
}

pub fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

impl LoadBook {
    pub fn set_journal(&self, journal: LoadJournal) {
        *self.journal.lock() = Some(journal);
    }

    pub fn set_reboot_repair(&self, repair: LoadRebootRepair) {
        *self.reboot_repair.lock() = Some(repair);
    }

    pub fn reboot_repair(&self) -> Option<LoadRebootRepair> {
        self.reboot_repair.lock().clone()
    }

    pub fn journal(&self, serial: &str, action: &str, message: &str) {
        let journal = self.journal.lock().clone();
        match journal {
            Some(journal) => journal(serial, action, message),
            None => tracing::info!(serial, action, message, "device load"),
        }
    }

    /// Evaluate against the phone's previous sample, store it, and journal a status change.
    pub fn record(&self, serial: &str, sample: DeviceLoadSample) -> LoadVerdict {
        let mut change: Option<(&'static str, String)> = None;
        let verdict = {
            let mut phones = self.phones.lock();
            let phone = phones.entry(serial.to_string()).or_default();
            let verdict = load::evaluate(&sample, phone.last.as_ref());
            match &verdict {
                LoadVerdict::Healthy => {
                    if matches!(
                        phone.status,
                        Some(DeviceLoadStatus::Overloaded | DeviceLoadStatus::AwaitingOperator)
                    ) {
                        change = Some((
                            "device.load.recovered",
                            format!("hết quá tải, đo lại đạt ({})", sample.summary()),
                        ));
                    }
                    phone.status = Some(DeviceLoadStatus::Healthy);
                    phone.reason = None;
                    phone.last_pass_ms = Some(sample.sampled_at_ms);
                }
                LoadVerdict::Overloaded { reasons } => {
                    let reason =
                        format!("{}: {}", load::DEVICE_OVERLOADED_MARKER, reasons.join("; "));
                    if phone.status != Some(DeviceLoadStatus::AwaitingOperator) {
                        if phone.status != Some(DeviceLoadStatus::Overloaded) {
                            change = Some((
                                "device.load.overloaded",
                                format!("phát hiện {reason} ({})", sample.summary()),
                            ));
                        }
                        phone.status = Some(DeviceLoadStatus::Overloaded);
                        phone.reason = Some(reason);
                    }
                    phone.last_pass_ms = None;
                }
                LoadVerdict::NeedsConfirmation => {
                    phone.last_pass_ms = None;
                }
                LoadVerdict::Unknown => {
                    if phone.status.is_none() {
                        phone.status = Some(DeviceLoadStatus::Unknown);
                    }
                }
            }
            phone.last = Some(sample);
            verdict
        };
        if let Some((action, message)) = change {
            self.journal(serial, action, &message);
        }
        verdict
    }

    /// "quá tải — chờ người vận hành". Journals only on entering this state: the reason carries
    /// fresh numbers each minute, and a phone that stays blocked must not write a line each time.
    pub fn mark_awaiting_operator(&self, serial: &str, why: &str) {
        let message = {
            let mut phones = self.phones.lock();
            let phone = phones.entry(serial.to_string()).or_default();
            let reason = format!(
                "{} — chờ người vận hành: {why}",
                load::DEVICE_OVERLOADED_MARKER
            );
            let entering = phone.status != Some(DeviceLoadStatus::AwaitingOperator);
            phone.status = Some(DeviceLoadStatus::AwaitingOperator);
            phone.reason = Some(reason.clone());
            phone.last_pass_ms = None;
            if !entering {
                return;
            }
            reason
        };
        self.journal(serial, "device.load.awaiting_operator", &message);
    }

    pub fn status(&self, serial: &str) -> Option<DeviceLoadStatus> {
        self.phones
            .lock()
            .get(serial)
            .and_then(|phone| phone.status)
    }

    pub fn reason(&self, serial: &str) -> Option<String> {
        self.phones
            .lock()
            .get(serial)
            .and_then(|phone| phone.reason.clone())
    }

    /// A passing sample younger than [`load::FRESH_PASS_MS`] and no mark since.
    pub fn fresh_pass(&self, serial: &str, now: i64) -> bool {
        self.phones.lock().get(serial).is_some_and(|phone| {
            phone.status == Some(DeviceLoadStatus::Healthy)
                && phone
                    .last_pass_ms
                    .is_some_and(|at| now >= at && now - at <= load::FRESH_PASS_MS)
        })
    }

    pub fn cleanup_allowed(&self, serial: &str, now: i64) -> bool {
        self.phones.lock().get(serial).is_none_or(|phone| {
            phone
                .last_cleanup_ms
                .is_none_or(|at| now - at >= load::LIGHT_CLEANUP_MIN_INTERVAL_MS)
        })
    }

    pub fn note_cleanup(&self, serial: &str, now: i64) {
        self.phones
            .lock()
            .entry(serial.to_string())
            .or_default()
            .last_cleanup_ms = Some(now);
    }

    pub fn reboot_budget_left(&self, serial: &str, now: i64) -> bool {
        self.phones.lock().get(serial).is_none_or(|phone| {
            !phone
                .reboots_ms
                .iter()
                .any(|at| now - at < load::REBOOT_BUDGET_WINDOW_MS)
        })
    }

    pub fn note_reboot(&self, serial: &str, now: i64) {
        let mut phones = self.phones.lock();
        let phone = phones.entry(serial.to_string()).or_default();
        phone
            .reboots_ms
            .retain(|at| now - at < load::REBOOT_BUDGET_WINDOW_MS);
        phone.reboots_ms.push(now);
        // A rebooted phone takes no job until a fresh sample passes.
        phone.last_pass_ms = None;
        phone.last = None;
        phone.offline_since_ms = None;
        phone.reconnect_spent = false;
    }

    /// One adb state reading of `serial`. True exactly once per Riviu reboot: when the phone
    /// has read `offline` without a break for [`OFFLINE_RECONNECT_AFTER_MS`] within
    /// [`OFFLINE_RECONNECT_WINDOW_MS`] of that reboot. The caller then runs the reconnect.
    pub fn offline_reconnect_due(&self, serial: &str, offline: bool, now: i64) -> bool {
        let mut phones = self.phones.lock();
        let Some(phone) = phones.get_mut(serial) else {
            return false;
        };
        let rebooted = phone
            .reboots_ms
            .last()
            .is_some_and(|at| now - at < OFFLINE_RECONNECT_WINDOW_MS);
        if !offline || !rebooted {
            phone.offline_since_ms = None;
            return false;
        }
        let since = *phone.offline_since_ms.get_or_insert(now);
        if phone.reconnect_spent || now - since < OFFLINE_RECONNECT_AFTER_MS {
            return false;
        }
        phone.reconnect_spent = true;
        true
    }

    pub fn snapshot(&self) -> Vec<DeviceLoadState> {
        let phones = self.phones.lock();
        let mut states: Vec<DeviceLoadState> = phones
            .iter()
            .map(|(udid, phone)| DeviceLoadState {
                udid: udid.clone(),
                status: phone.status.unwrap_or(DeviceLoadStatus::Unknown),
                reason: phone.reason.clone(),
                last_sample: phone.last.clone(),
            })
            .collect();
        states.sort_by(|a, b| a.udid.cmp(&b.udid));
        states
    }
}
