//! The fleet baseline of phone system settings ("Cài đặt máy").
//!
//! A baseline is a small, reviewed set of Android system settings the operator wants every phone
//! to hold: lock screen off and auto-rotate off first, because a phone that re-locks or turns
//! sideways is a phone no automation can drive (2026-10-08: a publish failed its foreground proof
//! with `mCurrentFocus=StatusBar` and `mDreamingLockscreen=true`).
//!
//! Only the contract lives here. Reading and writing a phone is the Android driver's job, and the
//! lease that makes a write safe is the control plane's. Nothing in the catalogue may touch
//! accounts, networking, USB-debugging authorisation or app data -- those are not "settings a
//! fleet can converge on", they are identity and access, and a wrong value locks the operator out.

use serde::{Deserialize, Serialize};

/// The phrase every lock-screen refusal carries. The Android driver writes it and
/// `publish_recovery` reads it, so the typed pre-Post reason does not depend on a paraphrase
/// surviving `driver_error`'s flattening of the error chain into text.
pub const SCREEN_LOCKED_MARKER: &str = "máy đang khóa màn hình";

/// The recovery code a lock-screen refusal settles as: nothing reached TikTok, and the same
/// phone can be retried once it is unlocked.
pub const SCREEN_LOCKED_CODE: &str = "device_screen_locked";

pub fn mentions_screen_locked(message: &str) -> bool {
    message.to_lowercase().contains(SCREEN_LOCKED_MARKER)
}

/// One setting the baseline can hold. The catalogue is closed on purpose: every member has an
/// exact read, an idempotent apply and a verify-by-re-read in the driver.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BaselineSetting {
    /// `locksettings set-disabled true` + `wm dismiss-keyguard`. Only for a phone without a
    /// PIN/pattern/password: a phone with a credential is reported, never attempted.
    LockScreenDisabled,
    /// `settings put system accelerometer_rotation 0` and `user_rotation 0`.
    AutoRotateOff,
    /// `settings put global stay_on_while_plugged_in 7` (AC | USB | wireless).
    StayAwakeWhileCharging,
    /// `settings put system screen_off_timeout 1800000` (30 min, the largest value stock
    /// Settings offers).
    ScreenOffTimeoutMax,
    /// `window_animation_scale`, `transition_animation_scale`, `animator_duration_scale` = 0.
    AnimationsOff,
}

impl BaselineSetting {
    pub const ALL: [BaselineSetting; 5] = [
        BaselineSetting::LockScreenDisabled,
        BaselineSetting::AutoRotateOff,
        BaselineSetting::StayAwakeWhileCharging,
        BaselineSetting::ScreenOffTimeoutMax,
        BaselineSetting::AnimationsOff,
    ];

    /// The settings a fresh install holds and auto-applies: the two the operator asked for.
    pub const DEFAULT_BASELINE: [BaselineSetting; 2] =
        [BaselineSetting::LockScreenDisabled, BaselineSetting::AutoRotateOff];

    pub fn key(self) -> &'static str {
        match self {
            BaselineSetting::LockScreenDisabled => "lockScreenDisabled",
            BaselineSetting::AutoRotateOff => "autoRotateOff",
            BaselineSetting::StayAwakeWhileCharging => "stayAwakeWhileCharging",
            BaselineSetting::ScreenOffTimeoutMax => "screenOffTimeoutMax",
            BaselineSetting::AnimationsOff => "animationsOff",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|setting| setting.key() == key)
    }
}

/// What a read of one setting proved. `Unknown` is the answer whenever the phone did not say:
/// a failed or unparseable read is never folded into `Ok` or `Drift`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BaselineStatus {
    Ok,
    Drift,
    Unknown,
    /// The setting cannot be applied by adb on this phone (lock screen with a credential, or a
    /// credential that could not be read). A person has to act.
    NeedsManual,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BaselineSettingReading {
    pub setting: BaselineSetting,
    pub status: BaselineStatus,
    /// The raw values the phone printed, e.g. `accelerometer_rotation=0 user_rotation=0`.
    pub observed: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceBaselineReading {
    pub udid: String,
    pub settings: Vec<BaselineSettingReading>,
    /// Set when the device could not be read at all (offline, not Android, adb error).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// The typed result of applying the baseline, per setting and rolled up per device.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BaselineOutcome {
    /// Written and proven by a re-read.
    Applied,
    /// Already held the baseline value; nothing was written.
    AlreadyOk,
    NeedsManual,
    /// The phone is held by other work (publish, nurture, interaction...). Refused, not
    /// preempted; nothing was written.
    RefusedBusy,
    Failed,
    /// Not an Android phone: iOS has no equivalent and gets no effect.
    Unsupported,
}

impl BaselineOutcome {
    /// Worst-first order used for a device roll-up.
    fn severity(self) -> u8 {
        match self {
            BaselineOutcome::Failed => 5,
            BaselineOutcome::RefusedBusy => 4,
            BaselineOutcome::NeedsManual => 3,
            BaselineOutcome::Unsupported => 2,
            BaselineOutcome::Applied => 1,
            BaselineOutcome::AlreadyOk => 0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BaselineItemResult {
    pub setting: BaselineSetting,
    pub outcome: BaselineOutcome,
    pub observed: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceBaselineResult {
    pub udid: String,
    pub outcome: BaselineOutcome,
    pub items: Vec<BaselineItemResult>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

impl DeviceBaselineResult {
    /// A device-level outcome with no per-setting work (busy, unsupported, unreachable).
    pub fn whole_device(udid: &str, outcome: BaselineOutcome, detail: impl Into<String>) -> Self {
        Self {
            udid: udid.to_string(),
            outcome,
            items: Vec::new(),
            detail: Some(detail.into()),
        }
    }

    pub fn from_items(udid: &str, items: Vec<BaselineItemResult>) -> Self {
        Self {
            udid: udid.to_string(),
            outcome: rollup_outcome(&items),
            items,
            detail: None,
        }
    }
}

/// The device outcome is its worst item. An empty plan is `AlreadyOk`: nothing was asked.
pub fn rollup_outcome(items: &[BaselineItemResult]) -> BaselineOutcome {
    items
        .iter()
        .map(|item| item.outcome)
        .max_by_key(|outcome| outcome.severity())
        .unwrap_or(BaselineOutcome::AlreadyOk)
}

/// One operator-readable line for the operation log, e.g.
/// `Cài đặt máy: đã áp dụng 1, đã đúng 1 · tắt khóa màn hình: Cần mở khóa bằng tay ...`.
pub fn summarize(result: &DeviceBaselineResult) -> String {
    let label = |outcome: BaselineOutcome| match outcome {
        BaselineOutcome::Applied => "đã áp dụng",
        BaselineOutcome::AlreadyOk => "đã đúng",
        BaselineOutcome::NeedsManual => "cần làm tay",
        BaselineOutcome::RefusedBusy => "máy đang bận",
        BaselineOutcome::Failed => "lỗi",
        BaselineOutcome::Unsupported => "chưa hỗ trợ",
    };
    if result.items.is_empty() {
        let detail = result.detail.as_deref().unwrap_or_default();
        return format!("Cài đặt máy: {} {detail}", label(result.outcome))
            .trim_end()
            .to_string();
    }
    let mut counts: Vec<(BaselineOutcome, usize)> = Vec::new();
    for item in &result.items {
        match counts.iter_mut().find(|(outcome, _)| *outcome == item.outcome) {
            Some((_, count)) => *count += 1,
            None => counts.push((item.outcome, 1)),
        }
    }
    let mut line = format!(
        "Cài đặt máy: {}",
        counts
            .iter()
            .map(|(outcome, count)| format!("{} {count}", label(*outcome)))
            .collect::<Vec<_>>()
            .join(", ")
    );
    for item in &result.items {
        if matches!(item.outcome, BaselineOutcome::NeedsManual | BaselineOutcome::Failed) {
            line.push_str(&format!(
                " · {}: {}",
                item.setting.key(),
                item.detail.as_deref().unwrap_or(item.observed.as_str())
            ));
        }
    }
    line
}

/// The persisted operator choice: which settings form the baseline, and whether a phone is
/// brought to it when it connects.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct DeviceBaselineConfig {
    pub settings: Vec<BaselineSetting>,
    pub auto_apply_on_connect: bool,
}

impl Default for DeviceBaselineConfig {
    fn default() -> Self {
        Self {
            settings: BaselineSetting::DEFAULT_BASELINE.to_vec(),
            auto_apply_on_connect: true,
        }
    }
}

impl DeviceBaselineConfig {
    /// Sorted, de-duplicated settings, so the stored form is canonical.
    pub fn normalized(mut self) -> Self {
        self.settings.sort();
        self.settings.dedup();
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(setting: BaselineSetting, outcome: BaselineOutcome) -> BaselineItemResult {
        BaselineItemResult {
            setting,
            outcome,
            observed: String::new(),
            detail: None,
        }
    }

    #[test]
    fn device_outcome_is_its_worst_item() {
        let items = vec![
            item(BaselineSetting::AutoRotateOff, BaselineOutcome::Applied),
            item(BaselineSetting::LockScreenDisabled, BaselineOutcome::NeedsManual),
        ];
        assert_eq!(rollup_outcome(&items), BaselineOutcome::NeedsManual);
        let items = vec![
            item(BaselineSetting::AutoRotateOff, BaselineOutcome::AlreadyOk),
            item(BaselineSetting::AnimationsOff, BaselineOutcome::Applied),
        ];
        assert_eq!(rollup_outcome(&items), BaselineOutcome::Applied);
        let items = vec![
            item(BaselineSetting::AutoRotateOff, BaselineOutcome::Failed),
            item(BaselineSetting::LockScreenDisabled, BaselineOutcome::NeedsManual),
        ];
        assert_eq!(rollup_outcome(&items), BaselineOutcome::Failed);
        assert_eq!(rollup_outcome(&[]), BaselineOutcome::AlreadyOk);
    }

    #[test]
    fn summary_counts_outcomes_and_names_what_needs_a_person() {
        let mut manual = item(BaselineSetting::LockScreenDisabled, BaselineOutcome::NeedsManual);
        manual.detail = Some("Cần mở khóa bằng tay: máy có mã PIN/mật khẩu/hình vẽ.".into());
        let result = DeviceBaselineResult::from_items(
            "ce0717171c2a64d50d",
            vec![item(BaselineSetting::AutoRotateOff, BaselineOutcome::Applied), manual],
        );
        let line = summarize(&result);
        assert!(line.starts_with("Cài đặt máy: đã áp dụng 1, cần làm tay 1"), "{line}");
        assert!(line.contains("lockScreenDisabled: Cần mở khóa bằng tay"), "{line}");
        let busy = DeviceBaselineResult::whole_device("x", BaselineOutcome::RefusedBusy, "Máy đang bận");
        assert_eq!(summarize(&busy), "Cài đặt máy: máy đang bận Máy đang bận");
    }

    #[test]
    fn screen_locked_marker_is_matched_case_insensitively() {
        assert!(mentions_screen_locked("X: Máy đang khóa màn hình (StatusBar)"));
        assert!(!mentions_screen_locked("did not reach the foreground"));
    }

    #[test]
    fn default_baseline_is_lock_screen_and_rotation_with_auto_apply() {
        let config = DeviceBaselineConfig::default();
        assert!(config.auto_apply_on_connect);
        assert_eq!(
            config.settings,
            vec![BaselineSetting::LockScreenDisabled, BaselineSetting::AutoRotateOff]
        );
    }

    #[test]
    fn wire_names_are_camel_case_and_round_trip() {
        for setting in BaselineSetting::ALL {
            let json = serde_json::to_value(setting).unwrap();
            assert_eq!(json, serde_json::json!(setting.key()));
            assert_eq!(BaselineSetting::from_key(setting.key()), Some(setting));
        }
        assert_eq!(
            serde_json::to_value(BaselineOutcome::RefusedBusy).unwrap(),
            serde_json::json!("refusedBusy")
        );
    }
}
