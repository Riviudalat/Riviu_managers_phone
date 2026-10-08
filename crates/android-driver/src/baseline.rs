//! Read, apply and verify the fleet's baseline of Android system settings ("Cài đặt máy").
//!
//! Every function here runs over an injected `shell` closure so the whole decision -- what was
//! read, what is written, what the re-read proves -- is testable against recorded adb output.
//! The driver wires the closure to [`crate::adb::AdbProgram::shell_output`] with a deadline.
//!
//! Three rules, each one a way this could hurt a phone:
//!
//! - **Unknown is never folded into OK.** A read that failed or printed something unexpected is
//!   `BaselineStatus::Unknown`, and an apply is only `Applied` when a re-read proves the value.
//! - **A phone with a lock credential is never sent `locksettings`.** On Android 9-11 every
//!   `locksettings` verb, `get-disabled` included, first checks the (empty) old credential, so
//!   on a PIN phone it is a *wrong-PIN attempt* that Gatekeeper counts and throttles. The
//!   credential is therefore read from `dumpsys lock_settings` (no authentication), and
//!   `locksettings` runs only after that dump proved there is no credential. A dump that does not
//!   say is `NeedsManual`, not a guess.
//! - **Writes are idempotent and narrow.** Each setting writes only its own keys, a setting
//!   already at its value is not written, and nothing here touches accounts, networking,
//!   USB-debugging authorisation or app data.

use std::collections::HashMap;
use std::future::Future;

use riviu_core::device_control::baseline::{
    BaselineItemResult, BaselineOutcome, BaselineSetting, BaselineSettingReading, BaselineStatus,
};

/// The value `screen_off_timeout` is set to: 30 minutes, the longest choice stock Settings and
/// MIUI offer. Larger raw values are accepted by some builds and clamped or ignored by others,
/// so the baseline stays inside what the phone's own UI can show.
pub const SCREEN_OFF_TIMEOUT_MS: &str = "1800000";

/// `BatteryManager.BATTERY_PLUGGED_AC | USB | WIRELESS` = 1 | 2 | 4.
pub const STAY_ON_ALL_PLUGS: &str = "7";

const ANIMATION_KEYS: [&str; 3] = [
    "window_animation_scale",
    "transition_animation_scale",
    "animator_duration_scale",
];

/// One read of everything, in a single shell round trip. `;` and not `&&`: a failing section
/// leaves its own marker empty (→ unknown) instead of hiding every section after it.
///
/// No `locksettings` here -- see the module docs; it runs only in [`LOCK_DISABLED_READ`] once
/// the credential is proven absent.
pub const BASELINE_READ_SCRIPT: &str = "echo '@@credential'; dumpsys lock_settings | grep -E 'CredentialType|^ *User [0-9]+$|^ *SID = '; \
echo '@@accelerometer_rotation'; settings get system accelerometer_rotation; \
echo '@@user_rotation'; settings get system user_rotation; \
echo '@@stay_on_while_plugged_in'; settings get global stay_on_while_plugged_in; \
echo '@@screen_off_timeout'; settings get system screen_off_timeout; \
echo '@@window_animation_scale'; settings get global window_animation_scale; \
echo '@@transition_animation_scale'; settings get global transition_animation_scale; \
echo '@@animator_duration_scale'; settings get global animator_duration_scale; \
echo '@@keyguard'; dumpsys window | grep -E 'isKeyguardShowing|mKeyguardShowing|mDreamingLockscreen'; \
echo '@@end'";

pub const LOCK_DISABLED_READ: &str = "locksettings get-disabled";
pub const LOCK_DISABLE_WRITE: &str = "locksettings set-disabled true";
pub const DISMISS_KEYGUARD: &str = "wm dismiss-keyguard";

/// Split [`BASELINE_READ_SCRIPT`] output into its marked sections.
pub fn split_sections(stdout: &str) -> HashMap<String, String> {
    let mut sections = HashMap::new();
    let mut current: Option<String> = None;
    let mut body = String::new();
    for line in stdout.lines() {
        let line = line.trim_end_matches('\r');
        if let Some(name) = line.trim().strip_prefix("@@") {
            if let Some(previous) = current.take() {
                sections.insert(previous, std::mem::take(&mut body));
            }
            current = (name != "end").then(|| name.to_string());
            body.clear();
            continue;
        }
        if current.is_some() {
            body.push_str(line);
            body.push('\n');
        }
    }
    if let Some(previous) = current {
        sections.insert(previous, body);
    }
    sections
}

/// A `settings get` answer. `null` is the phone saying the key was never written -- the
/// effective value is a build default nobody here knows, so it is `NotSet`, not "0".
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SettingValue {
    Value(String),
    NotSet,
    Unreadable,
}

impl SettingValue {
    fn display(&self) -> String {
        match self {
            SettingValue::Value(value) => value.clone(),
            SettingValue::NotSet => "null".to_string(),
            SettingValue::Unreadable => "?".to_string(),
        }
    }
}

pub fn parse_setting_value(section: Option<&str>) -> SettingValue {
    let Some(section) = section else {
        return SettingValue::Unreadable;
    };
    let mut lines = section.lines().map(str::trim).filter(|line| !line.is_empty());
    let (Some(value), None) = (lines.next(), lines.next()) else {
        return SettingValue::Unreadable;
    };
    if value == "null" {
        return SettingValue::NotSet;
    }
    // A shell error ("settings: not found", "Exception ...") is not a value.
    if value.contains(' ') || value.contains(':') {
        return SettingValue::Unreadable;
    }
    SettingValue::Value(value.to_string())
}

/// Whether a value equals the target numerically ("0" == "0.0" for animation scales).
fn value_is(value: &SettingValue, target: &str) -> Option<bool> {
    match value {
        SettingValue::Value(value) => match (value.parse::<f64>(), target.parse::<f64>()) {
            (Ok(left), Ok(right)) => Some((left - right).abs() < f64::EPSILON),
            _ => Some(value == target),
        },
        SettingValue::NotSet => Some(false),
        SettingValue::Unreadable => None,
    }
}

/// Whether the phone has a lock credential, from `dumpsys lock_settings`.
///
/// `CredentialType:` is printed as a name on Android 12+ (`NONE`, `PIN`, `PASSWORD`, `PATTERN`,
/// `PASSWORD_OR_PIN`, `UNKNOWN_<n>`) and as the raw constant on Android 10-11 (`-1` none,
/// `1` pattern, `2` password, `3` pin). Every user is checked and **any** credential counts:
/// a false "has credential" costs a manual step, a false "none" costs a wrong-PIN attempt.
///
/// Android 9 prints no `CredentialType` (SM-G955F/N fleet, 08/10/2026). Its dump lists each
/// `User <id>` with `SID = <hex>`, the Gatekeeper secure user id, which is enrolled with a
/// credential and cleared when the credential is removed. So a zero SID for **every** listed
/// user proves "none", any non-zero SID is a credential, and a user whose SID did not print (or
/// did not parse) leaves the answer unknown.
///
/// `None` when nothing says -- unknown, never a guess.
pub fn parse_lock_credential(dump: &str) -> Option<bool> {
    let mut seen_none = false;
    let mut users = 0usize;
    let mut zero_sids = 0usize;
    let mut sid_pending = false;
    for line in dump.lines() {
        let trimmed = line.trim();
        if let Some(id) = trimmed.strip_prefix("User ") {
            if !id.is_empty() && id.bytes().all(|b| b.is_ascii_digit()) {
                users += 1;
                sid_pending = true;
            }
            continue;
        }
        if let Some(sid) = trimmed.strip_prefix("SID = ") {
            if !sid_pending {
                continue;
            }
            sid_pending = false;
            match u64::from_str_radix(sid.trim(), 16) {
                Ok(0) => zero_sids += 1,
                Ok(_) => return Some(true),
                Err(_) => {}
            }
            continue;
        }
        let Some(index) = line.find("CredentialType:") else {
            continue;
        };
        let value = line[index + "CredentialType:".len()..].trim();
        let value = value.split_whitespace().next().unwrap_or_default();
        match value.to_ascii_uppercase().as_str() {
            "NONE" | "-1" => seen_none = true,
            "" => {}
            // PIN/PASSWORD/PATTERN, numeric types and UNKNOWN_<n> are all "a person must act".
            _ => return Some(true),
        }
    }
    if seen_none {
        return Some(false);
    }
    (users > 0 && zero_sids == users).then_some(false)
}

/// `locksettings get-disabled` prints exactly `true` or `false`.
pub fn parse_lock_disabled(stdout: &str) -> Option<bool> {
    match stdout.trim() {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

/// Everything one read learned, before it is turned into per-setting readings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BaselineSnapshot {
    pub credential: Option<bool>,
    pub lock_disabled: Option<bool>,
    pub keyguard_locked: Option<bool>,
    pub values: HashMap<&'static str, SettingValue>,
}

const SETTING_KEYS: [&str; 7] = [
    "accelerometer_rotation",
    "user_rotation",
    "stay_on_while_plugged_in",
    "screen_off_timeout",
    "window_animation_scale",
    "transition_animation_scale",
    "animator_duration_scale",
];

pub fn parse_snapshot(stdout: &str, lock_disabled: Option<bool>) -> BaselineSnapshot {
    let sections = split_sections(stdout);
    let values = SETTING_KEYS
        .into_iter()
        .map(|key| (key, parse_setting_value(sections.get(key).map(String::as_str))))
        .collect();
    BaselineSnapshot {
        credential: sections
            .get("credential")
            .and_then(|section| parse_lock_credential(section)),
        lock_disabled,
        keyguard_locked: sections
            .get("keyguard")
            .and_then(|section| crate::adb::parse_keyguard_locked(section)),
        values,
    }
}

impl BaselineSnapshot {
    fn value(&self, key: &str) -> SettingValue {
        self.values
            .get(key)
            .cloned()
            .unwrap_or(SettingValue::Unreadable)
    }

    /// Several keys that must all hold `target`.
    fn keys_reading(
        &self,
        setting: BaselineSetting,
        keys: &[&str],
        target: &str,
    ) -> BaselineSettingReading {
        let observed = keys
            .iter()
            .map(|key| format!("{key}={}", self.value(key).display()))
            .collect::<Vec<_>>()
            .join(" ");
        let mut all_ok = true;
        let mut unknown = false;
        for key in keys {
            match value_is(&self.value(key), target) {
                Some(true) => {}
                Some(false) => all_ok = false,
                None => unknown = true,
            }
        }
        let status = if !all_ok {
            BaselineStatus::Drift
        } else if unknown {
            BaselineStatus::Unknown
        } else {
            BaselineStatus::Ok
        };
        BaselineSettingReading {
            setting,
            status,
            observed,
            detail: None,
        }
    }

    pub fn reading(&self, setting: BaselineSetting) -> BaselineSettingReading {
        match setting {
            BaselineSetting::LockScreenDisabled => self.lock_reading(),
            BaselineSetting::AutoRotateOff => self.keys_reading(
                setting,
                &["accelerometer_rotation", "user_rotation"],
                "0",
            ),
            BaselineSetting::StayAwakeWhileCharging => {
                self.keys_reading(setting, &["stay_on_while_plugged_in"], STAY_ON_ALL_PLUGS)
            }
            BaselineSetting::ScreenOffTimeoutMax => {
                self.keys_reading(setting, &["screen_off_timeout"], SCREEN_OFF_TIMEOUT_MS)
            }
            BaselineSetting::AnimationsOff => self.keys_reading(setting, &ANIMATION_KEYS, "0"),
        }
    }

    fn lock_reading(&self) -> BaselineSettingReading {
        let keyguard = match self.keyguard_locked {
            Some(true) => "đang khóa",
            Some(false) => "đang mở",
            None => "?",
        };
        let credential = match self.credential {
            Some(true) => "có",
            Some(false) => "không",
            None => "?",
        };
        let disabled = match self.lock_disabled {
            Some(value) => value.to_string(),
            None => "?".to_string(),
        };
        let observed =
            format!("mã khóa={credential} lockscreen.disabled={disabled} màn hình={keyguard}");
        let (status, detail) = match (self.credential, self.lock_disabled) {
            (Some(true), _) => (
                BaselineStatus::NeedsManual,
                Some("Cần mở khóa bằng tay: máy có mã PIN/mật khẩu/hình vẽ.".to_string()),
            ),
            (None, _) => (
                BaselineStatus::NeedsManual,
                Some(
                    "Không đọc được loại khóa màn hình; không gửi locksettings để tránh tính \
                     một lần nhập sai mã. Kiểm tra và tắt khóa bằng tay."
                        .to_string(),
                ),
            ),
            (Some(false), Some(true)) => (BaselineStatus::Ok, None),
            (Some(false), Some(false)) => (BaselineStatus::Drift, None),
            (Some(false), None) => (BaselineStatus::Unknown, None),
        };
        BaselineSettingReading {
            setting: BaselineSetting::LockScreenDisabled,
            status,
            observed,
            detail,
        }
    }
}

/// The reason no key can open this phone's lock screen, for publish preflight.
///
/// Only a lock screen that is **up** and a credential that is **proven** count. A swipe-only lock
/// is `None` because session start dismisses it, and so is a credential the dump did not print:
/// Android 9 builds may not print `CredentialType`, and refusing every locked Android 9 phone in
/// preflight would turn away phones the session start can still open. Those reach the session
/// and, if the lock really holds, settle as `device_screen_locked` there.
pub fn lock_blocker(snapshot: &BaselineSnapshot) -> Option<String> {
    (snapshot.keyguard_locked == Some(true) && snapshot.credential == Some(true)).then(|| {
        format!(
            "{} và máy có mã PIN/mật khẩu/hình vẽ; mở khóa máy bằng tay rồi kiểm tra lại",
            riviu_core::device_control::baseline::SCREEN_LOCKED_MARKER
        )
    })
}

/// The `settings put` commands that bring one non-lock setting to its baseline value.
pub fn apply_commands(setting: BaselineSetting) -> Vec<String> {
    match setting {
        BaselineSetting::LockScreenDisabled => {
            vec![LOCK_DISABLE_WRITE.to_string(), DISMISS_KEYGUARD.to_string()]
        }
        BaselineSetting::AutoRotateOff => vec![
            "settings put system accelerometer_rotation 0".to_string(),
            "settings put system user_rotation 0".to_string(),
        ],
        BaselineSetting::StayAwakeWhileCharging => vec![format!(
            "settings put global stay_on_while_plugged_in {STAY_ON_ALL_PLUGS}"
        )],
        BaselineSetting::ScreenOffTimeoutMax => vec![format!(
            "settings put system screen_off_timeout {SCREEN_OFF_TIMEOUT_MS}"
        )],
        BaselineSetting::AnimationsOff => ANIMATION_KEYS
            .iter()
            .map(|key| format!("settings put global {key} 0"))
            .collect(),
    }
}

/// Read every catalogue setting. `Err` only when the phone could not be asked at all.
pub async fn read_snapshot_with<F, Fut>(shell: &mut F) -> anyhow::Result<BaselineSnapshot>
where
    F: FnMut(String) -> Fut,
    Fut: Future<Output = anyhow::Result<String>>,
{
    let stdout = shell(BASELINE_READ_SCRIPT.to_string()).await?;
    let mut snapshot = parse_snapshot(&stdout, None);
    // Only a phone proven to have no credential is asked: see the module docs.
    if snapshot.credential == Some(false) {
        snapshot.lock_disabled = match shell(LOCK_DISABLED_READ.to_string()).await {
            Ok(stdout) => parse_lock_disabled(&stdout),
            Err(error) => {
                tracing::warn!(%error, "đọc locksettings get-disabled thất bại");
                None
            }
        };
    }
    Ok(snapshot)
}

pub async fn read_baseline_with<F, Fut>(shell: &mut F) -> anyhow::Result<Vec<BaselineSettingReading>>
where
    F: FnMut(String) -> Fut,
    Fut: Future<Output = anyhow::Result<String>>,
{
    let snapshot = read_snapshot_with(shell).await?;
    Ok(BaselineSetting::ALL
        .into_iter()
        .map(|setting| snapshot.reading(setting))
        .collect())
}

/// Bring the phone to `plan`, verifying every write by a fresh read.
///
/// Per setting: `Ok` → `AlreadyOk` (nothing written); `NeedsManual` → `NeedsManual` (nothing
/// written); `Drift`/`Unknown` → write, re-read, `Applied` only when the re-read is `Ok`.
pub async fn apply_baseline_with<F, Fut>(
    shell: &mut F,
    plan: &[BaselineSetting],
) -> anyhow::Result<Vec<BaselineItemResult>>
where
    F: FnMut(String) -> Fut,
    Fut: Future<Output = anyhow::Result<String>>,
{
    let before = read_snapshot_with(shell).await?;
    let mut pending = Vec::new();
    let mut results: Vec<(BaselineSetting, BaselineItemResult)> = Vec::new();
    for &setting in plan {
        let reading = before.reading(setting);
        let outcome = match reading.status {
            BaselineStatus::Ok => Some(BaselineOutcome::AlreadyOk),
            BaselineStatus::NeedsManual => Some(BaselineOutcome::NeedsManual),
            BaselineStatus::Drift | BaselineStatus::Unknown => None,
        };
        match outcome {
            Some(outcome) => results.push((
                setting,
                BaselineItemResult {
                    setting,
                    outcome,
                    observed: reading.observed,
                    detail: reading.detail,
                },
            )),
            None => pending.push(setting),
        }
    }
    if pending.is_empty() {
        return Ok(order(plan, results));
    }

    let mut write_errors: HashMap<BaselineSetting, String> = HashMap::new();
    for &setting in &pending {
        for command in apply_commands(setting) {
            if let Err(error) = shell(command.clone()).await {
                // `wm dismiss-keyguard` only hides a keyguard that is up; the setting itself is
                // proven by the re-read, so its failure is logged, not fatal.
                if command == DISMISS_KEYGUARD {
                    tracing::warn!(%error, "wm dismiss-keyguard bị từ chối");
                    continue;
                }
                write_errors
                    .entry(setting)
                    .or_insert_with(|| format!("`{command}`: {error}"));
            }
        }
    }

    let after = match read_snapshot_with(shell).await {
        Ok(snapshot) => Some(snapshot),
        Err(error) => {
            tracing::warn!(%error, "đọc lại cài đặt máy sau khi áp dụng thất bại");
            None
        }
    };
    for setting in pending {
        let item = match &after {
            Some(after) => {
                let reading = after.reading(setting);
                if reading.status == BaselineStatus::Ok {
                    BaselineItemResult {
                        setting,
                        outcome: BaselineOutcome::Applied,
                        observed: reading.observed,
                        detail: None,
                    }
                } else {
                    BaselineItemResult {
                        setting,
                        outcome: BaselineOutcome::Failed,
                        observed: reading.observed,
                        detail: Some(write_errors.remove(&setting).unwrap_or_else(|| {
                            "Đã gửi lệnh nhưng đọc lại chưa đúng giá trị chuẩn.".to_string()
                        })),
                    }
                }
            }
            None => BaselineItemResult {
                setting,
                outcome: BaselineOutcome::Failed,
                observed: "?".to_string(),
                detail: Some(
                    "Đã gửi lệnh nhưng không đọc lại được; trạng thái chưa xác định.".to_string(),
                ),
            },
        };
        results.push((setting, item));
    }
    Ok(order(plan, results))
}

fn order(
    plan: &[BaselineSetting],
    mut results: Vec<(BaselineSetting, BaselineItemResult)>,
) -> Vec<BaselineItemResult> {
    results.sort_by_key(|(setting, _)| plan.iter().position(|planned| planned == setting));
    results.into_iter().map(|(_, item)| item).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex};

    /// A fake phone: holds setting values, answers the read script from them, applies
    /// `settings put` / `locksettings set-disabled` to them, and records every command.
    #[derive(Clone)]
    struct FakePhone {
        state: Arc<Mutex<FakeState>>,
    }

    struct FakeState {
        credential_line: Option<&'static str>,
        lock_disabled: bool,
        keyguard: &'static str,
        values: HashMap<String, String>,
        commands: Vec<String>,
        /// Puts that the phone accepts but silently ignores (a ROM that pins a value).
        ignored_keys: Vec<&'static str>,
        fail_reads: VecDeque<bool>,
    }

    impl FakePhone {
        fn new(credential_line: Option<&'static str>) -> Self {
            let values = [
                ("accelerometer_rotation", "1"),
                ("user_rotation", "1"),
                ("stay_on_while_plugged_in", "0"),
                ("screen_off_timeout", "60000"),
                ("window_animation_scale", "1.0"),
                ("transition_animation_scale", "1.0"),
                ("animator_duration_scale", "1.0"),
            ]
            .into_iter()
            .map(|(key, value)| (key.to_string(), value.to_string()))
            .collect();
            Self {
                state: Arc::new(Mutex::new(FakeState {
                    credential_line,
                    lock_disabled: false,
                    // The 2026-10-08 failure: StatusBar focus with the dreaming lockscreen up.
                    keyguard: "    mShowingDream=false mDreamingLockscreen=true",
                    values,
                    commands: Vec::new(),
                    ignored_keys: Vec::new(),
                    fail_reads: VecDeque::new(),
                })),
            }
        }

        fn commands(&self) -> Vec<String> {
            self.state.lock().unwrap().commands.clone()
        }

        fn run(&self, command: String) -> anyhow::Result<String> {
            let mut state = self.state.lock().unwrap();
            state.commands.push(command.clone());
            if command == BASELINE_READ_SCRIPT {
                if state.fail_reads.pop_front().unwrap_or(false) {
                    anyhow::bail!("adb shell timed out after 20s");
                }
                let mut out = String::from("@@credential\r\n");
                if let Some(line) = state.credential_line {
                    out.push_str(line);
                    out.push_str("\r\n");
                }
                for key in SETTING_KEYS {
                    out.push_str(&format!("@@{key}\r\n{}\r\n", state.values[key]));
                }
                out.push_str(&format!("@@keyguard\r\n{}\r\n@@end\r\n", state.keyguard));
                return Ok(out);
            }
            if command == LOCK_DISABLED_READ {
                return Ok(format!("{}\n", state.lock_disabled));
            }
            if command == LOCK_DISABLE_WRITE {
                state.lock_disabled = true;
                return Ok("Lock screen disabled set to true\n".to_string());
            }
            if command == DISMISS_KEYGUARD {
                state.keyguard = "    mShowingDream=false mDreamingLockscreen=false";
                return Ok(String::new());
            }
            if let Some(rest) = command.strip_prefix("settings put ") {
                let parts: Vec<&str> = rest.split_whitespace().collect();
                let (key, value) = (parts[1], parts[2]);
                if !state.ignored_keys.contains(&key) {
                    state.values.insert(key.to_string(), value.to_string());
                }
                return Ok(String::new());
            }
            anyhow::bail!("unexpected command {command}")
        }

        fn shell(&self) -> impl FnMut(String) -> std::future::Ready<anyhow::Result<String>> + '_ {
            move |command| std::future::ready(self.run(command))
        }
    }

    fn outcome_of(items: &[BaselineItemResult], setting: BaselineSetting) -> BaselineOutcome {
        items
            .iter()
            .find(|item| item.setting == setting)
            .map(|item| item.outcome)
            .unwrap()
    }

    #[tokio::test]
    async fn locked_phone_without_credential_is_unlocked_and_verified() {
        let phone = FakePhone::new(Some("CredentialType: NONE"));
        let items = apply_baseline_with(
            &mut phone.shell(),
            &BaselineSetting::DEFAULT_BASELINE,
        )
        .await
        .unwrap();
        assert_eq!(outcome_of(&items, BaselineSetting::LockScreenDisabled), BaselineOutcome::Applied);
        assert_eq!(outcome_of(&items, BaselineSetting::AutoRotateOff), BaselineOutcome::Applied);
        let commands = phone.commands();
        assert!(commands.contains(&LOCK_DISABLE_WRITE.to_string()));
        assert!(commands.contains(&DISMISS_KEYGUARD.to_string()));
        assert!(commands.contains(&"settings put system accelerometer_rotation 0".to_string()));
        assert!(commands.contains(&"settings put system user_rotation 0".to_string()));
    }

    #[tokio::test]
    async fn phone_with_credential_is_reported_and_never_sent_locksettings() {
        for line in ["CredentialType: PIN", "CredentialType: 3", "CredentialType: PASSWORD"] {
            let phone = FakePhone::new(Some(line));
            let items = apply_baseline_with(
                &mut phone.shell(),
                &BaselineSetting::DEFAULT_BASELINE,
            )
            .await
            .unwrap();
            let lock = items
                .iter()
                .find(|item| item.setting == BaselineSetting::LockScreenDisabled)
                .unwrap();
            assert_eq!(lock.outcome, BaselineOutcome::NeedsManual, "{line}");
            assert!(lock.detail.as_deref().unwrap().contains("mã PIN"), "{line}");
            // Rotation is independent of the lock and is still brought to baseline.
            assert_eq!(outcome_of(&items, BaselineSetting::AutoRotateOff), BaselineOutcome::Applied);
            assert!(
                phone.commands().iter().all(|command| !command.starts_with("locksettings")),
                "{line}: a credential phone must never see locksettings: {:?}",
                phone.commands()
            );
        }
    }

    #[tokio::test]
    async fn unreadable_credential_is_needs_manual_not_a_guess() {
        let phone = FakePhone::new(None);
        let items = apply_baseline_with(&mut phone.shell(), &[BaselineSetting::LockScreenDisabled])
            .await
            .unwrap();
        assert_eq!(items[0].outcome, BaselineOutcome::NeedsManual);
        assert!(phone.commands().iter().all(|c| !c.starts_with("locksettings")));
    }

    #[tokio::test]
    async fn unlocked_phone_at_baseline_is_already_ok_and_nothing_is_written() {
        let phone = FakePhone::new(Some("CredentialType: -1"));
        {
            let mut state = phone.state.lock().unwrap();
            state.lock_disabled = true;
            state.keyguard = "    isKeyguardShowing=false";
            state.values.insert("accelerometer_rotation".into(), "0".into());
            state.values.insert("user_rotation".into(), "0".into());
        }
        let items = apply_baseline_with(&mut phone.shell(), &BaselineSetting::DEFAULT_BASELINE)
            .await
            .unwrap();
        assert!(items.iter().all(|item| item.outcome == BaselineOutcome::AlreadyOk));
        assert!(
            phone.commands().iter().all(|c| !c.contains(" put ") && c != LOCK_DISABLE_WRITE),
            "{:?}",
            phone.commands()
        );
    }

    #[tokio::test]
    async fn a_write_the_phone_ignores_is_failed_not_applied() {
        let phone = FakePhone::new(Some("CredentialType: NONE"));
        phone.state.lock().unwrap().ignored_keys.push("user_rotation");
        let items = apply_baseline_with(&mut phone.shell(), &[BaselineSetting::AutoRotateOff])
            .await
            .unwrap();
        assert_eq!(items[0].outcome, BaselineOutcome::Failed);
        assert!(items[0].observed.contains("user_rotation=1"), "{}", items[0].observed);
    }

    #[tokio::test]
    async fn a_failed_re_read_is_failed_with_unknown_state() {
        let phone = FakePhone::new(Some("CredentialType: NONE"));
        phone.state.lock().unwrap().fail_reads = VecDeque::from([false, true]);
        let items = apply_baseline_with(&mut phone.shell(), &[BaselineSetting::AnimationsOff])
            .await
            .unwrap();
        assert_eq!(items[0].outcome, BaselineOutcome::Failed);
        assert!(items[0].detail.as_deref().unwrap().contains("chưa xác định"));
    }

    #[tokio::test]
    async fn read_failure_is_an_error_not_a_reading() {
        let phone = FakePhone::new(Some("CredentialType: NONE"));
        phone.state.lock().unwrap().fail_reads = VecDeque::from([true]);
        assert!(read_baseline_with(&mut phone.shell()).await.is_err());
    }

    #[test]
    fn credential_parser_covers_named_numeric_and_missing_forms() {
        assert_eq!(parse_lock_credential("  CredentialType: NONE\n"), Some(false));
        assert_eq!(parse_lock_credential("CredentialType: -1"), Some(false));
        assert_eq!(parse_lock_credential("CredentialType: PATTERN"), Some(true));
        assert_eq!(parse_lock_credential("CredentialType: UNKNOWN_7"), Some(true));
        // Any user with a credential counts, whatever order the users print in.
        assert_eq!(
            parse_lock_credential("CredentialType: NONE\nCredentialType: PIN\n"),
            Some(true)
        );
        assert_eq!(parse_lock_credential("Quality: 0\nSID: 0\n"), None);
        assert_eq!(parse_lock_credential(""), None);
    }

    #[test]
    fn android_9_dump_without_credential_type_reads_the_gatekeeper_sid_of_every_user() {
        // SM-G955F, Android 9, swipe lock disabled (Riviu #24, 08/10/2026), as filtered by
        // BASELINE_READ_SCRIPT: no `CredentialType` line at all.
        let no_credential = "    User 0\r\n        SID = 0\r\n";
        assert_eq!(parse_lock_credential(no_credential), Some(false));
        // Gatekeeper enrolls a secure user id with the credential and clears it on removal.
        assert_eq!(parse_lock_credential("    User 0\n        SID = 5a1f03c2e9b7d410\n"), Some(true));
        assert_eq!(
            parse_lock_credential("    User 0\n        SID = 0\n    User 150\n        SID = 3e8\n"),
            Some(true),
            "a work profile's credential counts"
        );
        // A user whose SID was not printed (the dump's RemoteException branch) stays unknown.
        assert_eq!(parse_lock_credential("    User 0\n        SID = 0\n    User 150\n"), None);
        assert_eq!(parse_lock_credential("    User 0\n        SID = zz\n"), None);
        // A printed CredentialType still decides on builds that have it.
        assert_eq!(
            parse_lock_credential("    User 0\n        SID = 0\nCredentialType: PIN\n"),
            Some(true)
        );
    }

    #[test]
    fn setting_values_keep_null_and_errors_distinct_from_zero() {
        assert_eq!(parse_setting_value(Some("0\r\n")), SettingValue::Value("0".into()));
        assert_eq!(parse_setting_value(Some("null\n")), SettingValue::NotSet);
        assert_eq!(parse_setting_value(Some("")), SettingValue::Unreadable);
        assert_eq!(parse_setting_value(None), SettingValue::Unreadable);
        assert_eq!(
            parse_setting_value(Some("/system/bin/sh: settings: not found\n")),
            SettingValue::Unreadable
        );
        assert_eq!(value_is(&SettingValue::Value("0.0".into()), "0"), Some(true));
        assert_eq!(value_is(&SettingValue::NotSet, "0"), Some(false));
        assert_eq!(value_is(&SettingValue::Unreadable, "0"), None);
    }

    #[test]
    fn a_missing_section_reads_unknown_never_ok() {
        let snapshot = parse_snapshot("@@credential\nCredentialType: NONE\n@@end\n", Some(true));
        assert_eq!(snapshot.reading(BaselineSetting::AutoRotateOff).status, BaselineStatus::Unknown);
        assert_eq!(snapshot.reading(BaselineSetting::LockScreenDisabled).status, BaselineStatus::Ok);
    }

    #[test]
    fn preflight_blocks_only_a_proven_credential_behind_a_raised_lock_screen() {
        let locked_pin = "@@credential\nCredentialType: PIN\n@@keyguard\n    mDreamingLockscreen=true\n@@end\n";
        let reason = lock_blocker(&parse_snapshot(locked_pin, None)).unwrap();
        assert!(riviu_core::device_control::baseline::mentions_screen_locked(&reason), "{reason}");
        // Swipe-only lock: session start dismisses it, so preflight does not refuse.
        let locked_swipe = "@@credential\nCredentialType: NONE\n@@keyguard\n    mDreamingLockscreen=true\n@@end\n";
        assert_eq!(lock_blocker(&parse_snapshot(locked_swipe, None)), None);
        // Unlocked PIN phone, and a dump that does not say: neither is reported as locked.
        let open_pin = "@@credential\nCredentialType: PIN\n@@keyguard\n    isKeyguardShowing=false\n@@end\n";
        assert_eq!(lock_blocker(&parse_snapshot(open_pin, None)), None);
        let unknown = "@@credential\n@@keyguard\n    mDreamingLockscreen=true\n@@end\n";
        assert_eq!(lock_blocker(&parse_snapshot(unknown, None)), None);
    }

    #[test]
    fn catalogue_writes_only_its_own_keys() {
        for setting in BaselineSetting::ALL {
            for command in apply_commands(setting) {
                assert!(
                    command.starts_with("settings put system ")
                        || command.starts_with("settings put global ")
                        || command == LOCK_DISABLE_WRITE
                        || command == DISMISS_KEYGUARD,
                    "{command}"
                );
                for forbidden in ["adb_enabled", "account", "wifi", "airplane", "pm clear", "secure"] {
                    assert!(!command.contains(forbidden), "{command}");
                }
            }
        }
    }
}
