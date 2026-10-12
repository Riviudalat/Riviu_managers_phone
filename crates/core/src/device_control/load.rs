//! Phone load ("sức khỏe máy"): one read-only sample, the thresholds that call a phone
//! overloaded, and the tiered response's vocabulary.
//!
//! Measured 10/10/2026 on phone 44 (`ce031713d903001b01`, SM-G955F, Android 9): the sound sheet
//! was unreadable and taps landed after 110 s or more. `top -b -n 1` showed surfaceflinger at
//! 92% CPU (siblings 47/39 the same day: ~3%), `/proc/meminfo` showed 1.1 GB of swap in use
//! (siblings ~0.6 GB) and 1.1 GB MemAvailable (siblings 1.6-1.9 GB). `adb reboot` fixed it and
//! the next post verified on the first try. Nothing detected it, so each job spent 5-8 minutes
//! and failed before dispatch.
//!
//! Only the contract lives here. Reading a phone and cleaning it are the Android driver's job;
//! the lease that makes a cleanup safe is the control plane's; the ledger that makes a reboot
//! safe is the desktop's.

use serde::{Deserialize, Serialize};

/// The phrase every overload refusal carries. The Android driver writes it and publish recovery
/// reads it, so the typed reason survives `driver_error`'s flattening into text.
pub const DEVICE_OVERLOADED_MARKER: &str = "máy quá tải";

/// The recovery code an overload refusal settles as: nothing reached TikTok, and the same phone
/// can be retried once a fresh sample passes (after cleanup or a reboot).
pub const DEVICE_OVERLOADED_CODE: &str = "device_overloaded";

pub fn mentions_device_overloaded(message: &str) -> bool {
    message.to_lowercase().contains(DEVICE_OVERLOADED_MARKER)
}

/// surfaceflinger %CPU (as `top` prints it, per core) at or above which the compositor is
/// starving the app. Phone 44 measured 92% while siblings 47/39 sat at ~3% (10/10/2026); 60
/// leaves a wide margin on both sides.
///
/// It counts only together with swap use above [`SWAP_USED_CEILING_KB`] (phone 44: 1.1 GB).
/// On its own it also reads high on a working phone in the composer while Riviu captures its
/// screen. Measured 12/10/2026 (campaign `4a25d804`, SM-G955N `ce041714d44264230d`): 82% and
/// then 62%, four minutes apart, with 3087 MB MemAvailable, 468 MB swap, 3 h uptime and no
/// slow helper call in the run, and the phone was still refused. The phones refused the same
/// night for surfaceflinger with swap above 1 GB (`aadf361905` 1259 MB, `d903001b01`
/// 1075 MB, `715f321b05` 1444 MB) logged 7, 14 and 21 slow helper calls of up to 15-28 s.
/// On 11/10/2026, six phones at 68-102% read 6-54% one or two minutes later with no cleanup.
pub const SURFACEFLINGER_CPU_OVERLOAD_PERCENT: f32 = 60.0;

/// surfaceflinger must be high in this many consecutive samples: one spike during a scroll or
/// a video start is not an overloaded phone.
pub const SURFACEFLINGER_SUSTAINED_SAMPLES: usize = 2;

/// Two surfaceflinger samples further apart than this do not count as "sustained".
pub const SUSTAINED_WINDOW_MS: i64 = 3 * 60 * 1000;

/// MemAvailable below this is overloaded on its own. Phone 44 had 1.1 GB and siblings 1.6-1.9 GB
/// (10/10/2026, SM-G955F 4 GB); 600 MB is well below anything a working phone showed.
pub const MEM_AVAILABLE_FLOOR_KB: u64 = 600 * 1024;

/// Swap in use above this, together with MemAvailable below
/// [`SWAP_PAIRED_MEM_AVAILABLE_KB`], is overloaded. Phone 44 had 1.1 GB in use, siblings ~0.6 GB
/// (10/10/2026, SM-G955F).
pub const SWAP_USED_CEILING_KB: u64 = 1024 * 1024;

/// The MemAvailable that turns high swap use into overload. Siblings sat at 1.6-1.9 GB
/// (10/10/2026), so heavy swap with this much free memory is a phone that recovered, not one
/// that is thrashing.
pub const SWAP_PAIRED_MEM_AVAILABLE_KB: u64 = 900 * 1024;

/// A passing sample younger than this lets the next admission skip its own sample, so a job
/// that opens several sessions in a row does not pay one round trip per session.
pub const FRESH_PASS_MS: i64 = 60 * 1000;

/// Background re-sample period for phones that hold a running job (and phones marked
/// overloaded).
pub const BACKGROUND_SAMPLE_PERIOD_MS: i64 = 60 * 1000;

/// Tier 1 (light cleanup) at most this often per phone, so an idle phone that stays heavy is
/// not cleaned every minute.
pub const LIGHT_CLEANUP_MIN_INTERVAL_MS: i64 = 30 * 60 * 1000;

/// Tier 2 budget: at most one automatic reboot per phone per this window; then the operator.
pub const REBOOT_BUDGET_WINDOW_MS: i64 = 6 * 60 * 60 * 1000;

/// Background processes tier 1 force-stops. Reviewed, closed list; both seen in phone 44's
/// logcat (10/10/2026). Never TikTok, Trill, Riviu Helper, UiAutomator2 or a system package:
/// [`is_protected_package`] is checked again before any command is built.
pub const LIGHT_CLEANUP_FORCE_STOP: [&str; 2] =
    ["com.facebook.services", "com.facebook.appmanager"];

/// Packages tier 1 must never stop, by prefix.
pub const PROTECTED_PACKAGE_PREFIXES: [&str; 8] = [
    "com.ss.android.ugc.",
    "com.zhiliaoapp.musically",
    "com.riviu.",
    "io.appium.uiautomator2",
    "com.github.uiautomator",
    "android",
    "com.android.",
    "com.samsung.",
];

pub fn is_protected_package(package: &str) -> bool {
    PROTECTED_PACKAGE_PREFIXES
        .iter()
        .any(|prefix| package == prefix.trim_end_matches('.') || package.starts_with(prefix))
}

/// One read-only sample. Every field is `None` when the phone did not say: unknown, never zero.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct DeviceLoadSample {
    pub sampled_at_ms: i64,
    /// surfaceflinger %CPU from `top -m 10`. `None` when it was not among the top ten, which
    /// means it was below the tenth process and never means overloaded.
    pub surfaceflinger_cpu_percent: Option<f32>,
    pub target_package: Option<String>,
    /// The target app's %CPU summed over its processes in the top ten.
    pub target_app_cpu_percent: Option<f32>,
    /// Total idle as a share of all cores (0-100).
    pub cpu_idle_percent: Option<f32>,
    pub mem_available_kb: Option<u64>,
    pub swap_total_kb: Option<u64>,
    pub swap_free_kb: Option<u64>,
    pub uptime_secs: Option<f64>,
    /// Battery temperature in tenths of a degree C, when the phone exposes it.
    pub battery_temp_tenths_c: Option<i32>,
}

impl DeviceLoadSample {
    pub fn swap_used_kb(&self) -> Option<u64> {
        Some(self.swap_total_kb?.saturating_sub(self.swap_free_kb?))
    }

    /// Nothing that decides overload was read.
    pub fn is_blind(&self) -> bool {
        self.surfaceflinger_cpu_percent.is_none()
            && self.cpu_idle_percent.is_none()
            && self.mem_available_kb.is_none()
    }

    /// One Vietnamese line with the numbers the thresholds look at.
    pub fn summary(&self) -> String {
        let mb = |kb: Option<u64>| {
            kb.map(|kb| format!("{} MB", kb / 1024))
                .unwrap_or_else(|| "?".into())
        };
        let percent = |value: Option<f32>| {
            value
                .map(|value| format!("{value:.0}%"))
                .unwrap_or_else(|| "?".into())
        };
        let mut parts = vec![
            format!(
                "surfaceflinger {}",
                percent(self.surfaceflinger_cpu_percent)
            ),
            format!("RAM trống {}", mb(self.mem_available_kb)),
            format!("swap dùng {}", mb(self.swap_used_kb())),
            format!("CPU rảnh {}", percent(self.cpu_idle_percent)),
        ];
        if let (Some(package), Some(cpu)) = (&self.target_package, self.target_app_cpu_percent) {
            parts.push(format!("{package} {cpu:.0}%"));
        }
        if let Some(uptime) = self.uptime_secs {
            parts.push(format!("bật {:.1} giờ", uptime / 3600.0));
        }
        if let Some(temp) = self.battery_temp_tenths_c {
            parts.push(format!("pin {:.1}°C", f64::from(temp) / 10.0));
        }
        parts.join(", ")
    }
}

/// What a sample (with the one before it) says about the phone.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum LoadVerdict {
    Healthy,
    /// surfaceflinger is high in this sample only; take another before deciding.
    NeedsConfirmation,
    Overloaded {
        reasons: Vec<String>,
    },
    /// The sample said nothing that decides. Never treated as overloaded.
    Unknown,
}

impl LoadVerdict {
    pub fn is_overloaded(&self) -> bool {
        matches!(self, LoadVerdict::Overloaded { .. })
    }
}

/// Apply the thresholds. `previous` is the phone's last sample, used only for the surfaceflinger
/// "sustained" rule and only when it is within [`SUSTAINED_WINDOW_MS`].
pub fn evaluate(current: &DeviceLoadSample, previous: Option<&DeviceLoadSample>) -> LoadVerdict {
    if current.is_blind() {
        return LoadVerdict::Unknown;
    }
    let mut reasons = Vec::new();
    if let Some(available) = current.mem_available_kb {
        if available < MEM_AVAILABLE_FLOOR_KB {
            reasons.push(format!(
                "RAM trống {} MB < {} MB",
                available / 1024,
                MEM_AVAILABLE_FLOOR_KB / 1024
            ));
        } else if available < SWAP_PAIRED_MEM_AVAILABLE_KB {
            if let Some(used) = current.swap_used_kb() {
                if used > SWAP_USED_CEILING_KB {
                    reasons.push(format!(
                        "swap dùng {} MB > {} MB khi RAM trống chỉ {} MB",
                        used / 1024,
                        SWAP_USED_CEILING_KB / 1024,
                        available / 1024
                    ));
                }
            }
        }
    }
    // A busy compositor counts only on a phone that is also swapping heavily; see
    // [`SURFACEFLINGER_CPU_OVERLOAD_PERCENT`]. Unknown swap never corroborates.
    let sf_high = |sample: &DeviceLoadSample| {
        sample
            .surfaceflinger_cpu_percent
            .is_some_and(|cpu| cpu >= SURFACEFLINGER_CPU_OVERLOAD_PERCENT)
            && sample
                .swap_used_kb()
                .is_some_and(|used| used > SWAP_USED_CEILING_KB)
    };
    let mut needs_confirmation = false;
    if sf_high(current) {
        let sustained = previous.is_some_and(|previous| {
            sf_high(previous)
                && current.sampled_at_ms >= previous.sampled_at_ms
                && current.sampled_at_ms - previous.sampled_at_ms <= SUSTAINED_WINDOW_MS
        });
        if sustained {
            reasons.push(format!(
                "surfaceflinger {:.0}% CPU ≥ {:.0}% trong {} lần đo liên tiếp khi swap dùng {} MB",
                current.surfaceflinger_cpu_percent.unwrap_or_default(),
                SURFACEFLINGER_CPU_OVERLOAD_PERCENT,
                SURFACEFLINGER_SUSTAINED_SAMPLES,
                current.swap_used_kb().unwrap_or_default() / 1024
            ));
        } else {
            needs_confirmation = true;
        }
    }
    if !reasons.is_empty() {
        LoadVerdict::Overloaded { reasons }
    } else if needs_confirmation {
        LoadVerdict::NeedsConfirmation
    } else {
        LoadVerdict::Healthy
    }
}

/// Where a phone stands, for the device tile and the activity log.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DeviceLoadStatus {
    Healthy,
    /// Failed a sample; the automatic response (cleanup or reboot) has not finished yet.
    Overloaded,
    /// Failed after cleanup and a reboot is not allowed (uncertain work, a holder, or the 6 h
    /// budget is spent): "quá tải — chờ người vận hành".
    AwaitingOperator,
    Unknown,
}

/// One phone's load state as the UI reads it. In-memory only: the durable trail is the
/// activity log line each tier writes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceLoadState {
    pub udid: String,
    pub status: DeviceLoadStatus,
    /// Vietnamese, for the tile and the admission refusal.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_sample: Option<DeviceLoadSample>,
}

/// The answer to "Kiểm tra sức khỏe" and "Dọn máy".
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceLoadCheck {
    pub udid: String,
    pub verdict: LoadVerdict,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sample: Option<DeviceLoadSample>,
    /// Vietnamese summary for a toast.
    pub message: String,
}
