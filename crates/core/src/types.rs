use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const STREAM_FPS: u32 = 24;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum InteractionSessionKind {
    Ordinary,
    FreshText,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ActiveAppIdentity {
    pub bundle_id: String,
    pub pid: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct StreamStartProof {
    pub generation: u64,
    pub first_frame_observed: bool,
    pub stream_url: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct StreamHandoffProof {
    pub generation: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ConnectionKind {
    Usb,
    Wifi,
    Mock,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum DeviceStatus {
    Disconnected,
    Pairing,
    Connected,
    Preparing,
    Ready,
    Busy,
    Error,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum TileStreamState {
    Live,
    Sampling,
    #[default]
    Parked,
    Stale,
    Error,
}

/// Which mobile OS a device runs.
///
/// Stamped by the backend that listed the device. Never inferred from the udid —
/// see the rules at the top of `crate::driver_multiplex`.
///
/// Deliberately has **no `Default`** and no `#[serde(default)]`. The only plausible
/// default is `Ios`, which is exactly the bug this field exists to remove: an
/// Android phone rendered as "iOS 15". A backend that cannot answer must fail to
/// compile, and a payload that omits the key must fail to decode.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum DevicePlatform {
    Ios,
    Android,
}

/// A physical-looking key the operator can press from the desktop overlay.
///
/// These are the keys a farm console like GenFarmer puts beside the big phone
/// preview. Android maps each one to a real `KEYCODE_*`. iOS only has Home;
/// the rest must stay disabled in the UI rather than inventing HID.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum HardwareKey {
    Home,
    Back,
    Recents,
    VolumeUp,
    VolumeDown,
    Power,
    Notification,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceInfo {
    pub udid: String,
    pub name: String,
    pub model: String,
    pub platform: DevicePlatform,
    /// The OS release, as the platform words it: `16.7.15` on iOS, `15` on
    /// Android. Read with [`Self::platform`], never alone — it used to be called
    /// `ios_version` and the UI printed a hardcoded "iOS" beside it.
    pub os_version: String,
    pub connection: ConnectionKind,
    pub status: DeviceStatus,
    pub battery: Option<u8>,
    pub wda_ready: bool,
    pub wda_expires_at: Option<DateTime<Utc>>,
    pub stream_url: Option<String>,
    #[serde(default)]
    pub tile_stream_state: TileStreamState,
    pub last_error: Option<String>,
}

#[cfg(test)]
mod device_info_tests {
    use super::*;

    #[test]
    fn tile_stream_state_serializes_in_camel_case_and_defaults_to_parked() {
        assert_eq!(
            serde_json::to_value(TileStreamState::Sampling).expect("serialize tile state"),
            serde_json::json!("sampling")
        );
        assert_eq!(TileStreamState::default(), TileStreamState::Parked);

        let decoded: DeviceInfo = serde_json::from_value(serde_json::json!({
            "udid": "fixture",
            "name": "fixture",
            "model": "fixture",
            "platform": "ios",
            "osVersion": "fixture",
            "connection": "mock",
            "status": "ready",
            "battery": null,
            "wdaReady": true,
            "wdaExpiresAt": null,
            "streamUrl": null,
            "lastError": null
        }))
        .expect("decode a device payload without a tile state");

        assert_eq!(decoded.tile_stream_state, TileStreamState::Parked);
    }

    fn payload_without(omit: &str) -> serde_json::Value {
        let mut payload = serde_json::json!({
            "udid": "fixture",
            "name": "fixture",
            "model": "fixture",
            "platform": "android",
            "osVersion": "15",
            "connection": "usb",
            "status": "ready",
            "battery": null,
            "wdaReady": true,
            "wdaExpiresAt": null,
            "streamUrl": null,
            "lastError": null
        });
        payload
            .as_object_mut()
            .expect("object")
            .remove(omit)
            .expect("the key being omitted must have been there");
        payload
    }

    #[test]
    fn a_payload_with_no_platform_is_refused_rather_than_defaulted_to_ios() {
        // The whole reason `DevicePlatform` has no `Default`. A silent `Ios` here
        // is the "iOS 15 on an Android phone" bug coming back through the wire.
        let error = serde_json::from_value::<DeviceInfo>(payload_without("platform"))
            .expect_err("platform is required");
        assert!(error.to_string().contains("platform"), "{error}");
    }

    #[test]
    fn the_old_ios_version_key_no_longer_decodes() {
        // Proves the rename is a rename, not an alias. `DeviceInfo` is built fresh
        // on every listing and never persisted, so there is exactly one producer
        // and a second accepted spelling would only hide a stale one.
        let mut payload = payload_without("osVersion");
        payload
            .as_object_mut()
            .expect("object")
            .insert("iosVersion".into(), serde_json::json!("15"));
        assert!(serde_json::from_value::<DeviceInfo>(payload).is_err());
    }

    #[test]
    fn the_platform_travels_as_a_lowercase_tag() {
        // Matches `ConnectionKind`'s `"usb"`/`"wifi"`/`"mock"` convention, which is
        // what the hand-written TS mirror is written against.
        assert_eq!(
            serde_json::to_value(DevicePlatform::Android).expect("serialize"),
            serde_json::json!("android")
        );
        assert_eq!(
            serde_json::to_value(DevicePlatform::Ios).expect("serialize"),
            serde_json::json!("ios")
        );
    }

    #[test]
    fn hardware_keys_travel_as_camel_case_tags() {
        assert_eq!(
            serde_json::to_value(HardwareKey::VolumeUp).expect("serialize"),
            serde_json::json!("volumeUp")
        );
        assert_eq!(
            serde_json::from_value::<HardwareKey>(serde_json::json!("recents")).expect("decode"),
            HardwareKey::Recents
        );
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TapPoint {
    pub x: f64,
    pub y: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SwipeGesture {
    pub from: TapPoint,
    pub to: TapPoint,
    pub duration_ms: u64,
}

/// One leg of a swipe: move to this point, taking this long.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SwipeStep {
    pub point: TapPoint,
    pub duration_ms: u64,
}

/// A swipe as a **path** rather than as two endpoints.
///
/// Exists because [`SwipeGesture`] cannot describe a human gesture. It becomes one
/// `pointerMove` on the wire, which is a perfectly straight line traversed at a perfectly
/// constant velocity, from the same two coordinates every time — three properties a finger
/// does not have, and all three are trivially measurable by anything watching.
///
/// A path carries what the straight line cannot: slight curvature, a velocity that builds
/// and then eases, and a lift that happens a beat after the movement stops. The W3C pointer
/// protocol the Android agent speaks takes an arbitrary number of moves with individual
/// durations, so this costs nothing extra on the wire — the old shape was simply the
/// simplest thing that worked.
///
/// Backends that cannot express a path collapse it to first-point → last-point through the
/// default [`UiSession::swipe_path`](crate::driver::UiSession::swipe_path), so nothing has
/// to implement this to keep working.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SwipePath {
    /// Where the finger lands, before any movement.
    pub start: TapPoint,
    /// Where it goes, in order. Never empty.
    pub steps: Vec<SwipeStep>,
    /// How long the finger stays down after the last move, before lifting.
    ///
    /// A real flick does not end at the instant the finger stops: there is a few
    /// milliseconds of contact after the motion, and on Android that gap is part of how the
    /// framework decides between a fling and a drag.
    pub settle_ms: u64,
}

impl SwipePath {
    /// The last point, which is where the gesture ends.
    pub fn end(&self) -> TapPoint {
        self.steps
            .last()
            .map(|step| step.point.clone())
            .unwrap_or_else(|| self.start.clone())
    }

    /// Total time the finger is in contact and moving.
    pub fn travel_ms(&self) -> u64 {
        self.steps.iter().map(|step| step.duration_ms).sum()
    }

    /// The same gesture as two endpoints, for a backend that cannot draw a path.
    pub fn as_gesture(&self) -> SwipeGesture {
        SwipeGesture {
            from: self.start.clone(),
            to: self.end(),
            duration_ms: self.travel_ms().max(1),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ElementSelector {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accessibility_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub xpath: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub predicate: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "camelCase")]
pub enum ScriptAction {
    #[serde(rename_all = "camelCase")]
    LaunchApp {
        bundle_id: String,
    },
    #[serde(rename_all = "camelCase")]
    TerminateApp {
        bundle_id: String,
    },
    #[serde(rename_all = "camelCase")]
    Wait {
        milliseconds: u64,
    },
    #[serde(rename_all = "camelCase")]
    Tap {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        selector: Option<ElementSelector>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        point: Option<TapPoint>,
    },
    #[serde(rename_all = "camelCase")]
    Swipe {
        gesture: SwipeGesture,
    },
    #[serde(rename_all = "camelCase")]
    TypeText {
        value: String,
    },
    #[serde(rename_all = "camelCase")]
    Screenshot {
        name: String,
    },
    Home,
    #[serde(rename_all = "camelCase")]
    AssertVisible {
        selector: ElementSelector,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AutomationScript {
    pub version: u32,
    pub name: String,
    pub steps: Vec<ScriptAction>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum JobStatus {
    Queued,
    Running,
    Succeeded,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum StepStatus {
    Pending,
    Running,
    Succeeded,
    Failed,
    Skipped,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JobStepRecord {
    pub index: usize,
    pub action: String,
    pub status: StepStatus,
    pub error: Option<String>,
    pub artifact_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JobRecord {
    pub id: Uuid,
    pub script_name: String,
    pub udids: Vec<String>,
    pub status: JobStatus,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub steps: Vec<JobStepRecord>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StreamQuality {
    Low,
    Medium,
    High,
    Extra,
}

/// What the operator asked the view path for. Persisted under `stream.settings.v1`.
///
/// `#[serde(default)]` is not decoration: this round-trips through the database now, so a
/// blob written by an older build has to keep loading when a field is added. Without it the
/// first new field would make every stored row fail to deserialize — and the load happens at
/// startup, so that is a boot failure over a quality setting.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct StreamSettings {
    pub fps: u32,
    /// What a grid tile encodes at.
    pub grid_quality: StreamQuality,
    /// What the overlay encodes at, which is a separate choice because it is a separate
    /// picture: one phone filling a window rather than one of twenty tiles.
    pub focus_quality: StreamQuality,
}

impl Default for StreamSettings {
    fn default() -> Self {
        Self {
            fps: STREAM_FPS,
            grid_quality: StreamQuality::Medium,
            focus_quality: StreamQuality::High,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppleIdConfig {
    pub email: String,
    pub has_password: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentSettings {
    #[serde(default = "default_true")]
    pub auto_repair: bool,
}

fn default_true() -> bool {
    true
}

impl Default for AgentSettings {
    fn default() -> Self {
        Self { auto_repair: true }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum AgentState {
    Unknown,
    Missing,
    RepairRequired,
    Starting,
    Ready,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentStatus {
    pub udid: String,
    pub state: AgentState,
    pub artifact_id: String,
    pub artifact_version: String,
    pub bundle_id: String,
    pub protocol_version: u32,
    pub features: Vec<String>,
    pub installed_version: Option<String>,
    pub installed_build: Option<String>,
    pub auth_ready: bool,
    pub mjpeg_ready: bool,
    pub session_ready: bool,
    pub message: Option<String>,
}

impl AgentStatus {
    pub fn unknown(udid: impl Into<String>) -> Self {
        Self {
            udid: udid.into(),
            state: AgentState::Unknown,
            artifact_id: String::new(),
            artifact_version: String::new(),
            bundle_id: String::new(),
            protocol_version: 0,
            features: Vec::new(),
            installed_version: None,
            installed_build: None,
            auth_ready: false,
            mjpeg_ready: false,
            session_ready: false,
            message: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WdaStatus {
    pub udid: String,
    pub installed: bool,
    pub running: bool,
    pub expires_at: Option<DateTime<Utc>>,
    pub days_remaining: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceMeta {
    pub udid: String,
    pub notes: String,
    pub tags: Vec<String>,
    pub group_id: Option<String>,
    pub proxy_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceGroup {
    pub id: String,
    pub name: String,
    pub color: String,
    pub udids: Vec<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProxyConfig {
    pub id: String,
    pub name: String,
    pub proxy_type: String,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: String,
    pub notes: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MaterialItem {
    pub id: String,
    pub name: String,
    pub path: String,
    pub kind: String,
    pub size: u64,
    pub created_at: String,
}

/// What one operator-typed shell command produced.
///
/// All three fields, because a non-zero exit is a normal answer here rather than a
/// failure: `ls` on a missing path, `grep` with no match and `dumpsys` on an unknown
/// service all exit non-zero and put the useful text on stderr. Returning only stdout, or
/// turning a non-zero exit into an error, would hide the answer the operator asked for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShellOutcome {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
}

/// Whether an app came with the phone or was installed onto it.
///
/// **Tagged, never inferred, and never used as a filter.** Listing only third-party
/// packages would hide the very app the rest of this product drives: TikTok ships
/// preinstalled on some phones, so a `-3`-only listing can disagree with
/// `resolve_tiktok_package` about what is on the same device. Both partitions are read
/// and each row says which one it came from; hiding system apps is the UI's choice to
/// make visibly, not the driver's to make silently.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum InstalledAppKind {
    /// Installed onto the phone: `cmd package list packages -3`.
    User,
    /// Shipped with the phone or its ROM: `cmd package list packages -s`.
    System,
}

/// One app present on a phone, as the phone itself reports it.
///
/// `label` is `Option` because on Android it is **not obtainable over adb at all**, and
/// this type refuses to pretend otherwise. Measured 14/08/2026 on both attached phones:
/// `cmd package query-activities` returns the label as a resource id
/// (`labelRes=0x7f14026a nonLocalizedLabel=null`), which needs the APK's resource table
/// plus the device locale to resolve; 257 of 273 records on the Redmi carried a null
/// `nonLocalizedLabel`, neither phone has `aapt`/`aapt2`, and pulling APKs to read them
/// is absurd at the measured sizes (one base.apk was 261 MB). So a listing shows package
/// names, and a `None` here means "this phone cannot tell us", not "unnamed".
///
/// The route that *would* work is the on-device `com.riviu.agent` helper calling
/// `PackageManager.getApplicationLabel`, one HTTP call for the whole list. That is a
/// separate piece of work; the field exists so adding it later changes no shape.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstalledApp {
    /// `bundle_id` and not `package`: the house term across `uninstall_app`,
    /// `install_library_app` and the whole iOS path, and the same string both platforms
    /// use to name an app.
    pub bundle_id: String,
    pub kind: InstalledAppKind,
    /// Human-readable name where the platform gives one for free. Always `None` on
    /// Android — see the type doc.
    pub label: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppLibraryItem {
    pub id: String,
    pub name: String,
    pub path: String,
    pub bundle_id: String,
    pub version: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScheduleItem {
    pub id: String,
    pub name: String,
    pub script_name: String,
    pub udids: Vec<String>,
    pub every_minutes: u32,
    pub enabled: bool,
    pub last_run_at: Option<String>,
    pub next_run_at: Option<String>,
    /// Why the last due tick did not enqueue anything, or `None` if it did.
    ///
    /// Exists because the runner used to fail in complete silence: a schedule whose script
    /// had been renamed or deleted advanced both timestamps on every tick and enqueued
    /// nothing, so the page showed a healthy job that had never run.
    #[serde(default)]
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PublishTask {
    pub id: String,
    pub name: String,
    pub script_name: String,
    pub material_ids: Vec<String>,
    pub udids: Vec<String>,
    pub status: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpLog {
    pub id: String,
    pub action: String,
    pub detail: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalyticsSummary {
    pub device_total: usize,
    pub device_ready: usize,
    pub jobs_total: usize,
    pub jobs_succeeded: usize,
    pub jobs_failed: usize,
    pub jobs_running: usize,
    pub scripts_total: usize,
    pub materials_total: usize,
    pub apps_total: usize,
    pub schedules_enabled: usize,
    pub recent_logs: Vec<OpLog>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct NurtureSettings {
    pub base_url: String,
    pub model: String,
    pub api_key: String,
    pub input_price_per_1m: f64,
    pub output_price_per_1m: f64,
    pub bundle_id: String,
    pub num_videos: u32,
    pub num_rounds: u32,
    pub like_prob: u32,
    pub comment_prob: u32,
    pub follow_prob: u32,
    pub frenzy_prob: u32,
    pub watch_min: f64,
    pub watch_max: f64,
    pub persona: String,
    pub fatigue: bool,
    pub time_of_day: bool,
    pub pause_swipe: bool,
    pub night_start: u32,
    pub night_end: u32,
    pub recover_delay_min: u32,
    pub recover_delay_max: u32,
    pub stagger_delay_min: u32,
    pub stagger_delay_max: u32,
    pub comment_lang: String,
    pub ai_directions: String,
    pub max_comment_words: u32,
    pub schedule_enabled: bool,
    pub schedule_every_minutes: u32,
    pub schedule_duration_minutes: u32,
    #[serde(default)]
    pub schedule_udids: Vec<String>,
    /// Pin the behaviour cycle to one mood (`chatty` / `liking` / `skimming`).
    /// Empty means the normal varied cycle. Only for isolating a feature during
    /// a test — a real session should vary.
    #[serde(default)]
    pub steady_mood: String,

    // ---- per-feature switches ----
    //
    // Separate from the probabilities on purpose. `like_prob = 0` already turns liking
    // off, but it does so by **destroying the tuned number** — the operator who wants to
    // pause liking for one run has to remember what 35 was. A switch keeps the number and
    // stops the behaviour, which is what "bật tắt riêng biệt" actually asks for.
    //
    // Every one defaults to `true` so an existing stored profile keeps behaving exactly as
    // it did: `#[serde(default)]` on a bool would give `false` and silently switch
    // everything off on upgrade.
    #[serde(default = "yes")]
    pub like_enabled: bool,
    #[serde(default = "yes")]
    pub comment_enabled: bool,
    #[serde(default = "yes")]
    pub follow_enabled: bool,
    #[serde(default = "yes")]
    pub frenzy_enabled: bool,
    /// Whether to page through a photo carousel at all, rather than treating it as one
    /// card and swiping past.
    #[serde(default = "yes")]
    pub carousel_enabled: bool,
    /// Whether the built-in human pacing is allowed to override the configured numbers.
    ///
    /// Defaults to **off**, which is a deliberate change of shipped behaviour made on
    /// 12/08/2026: the operator asked for the panel's numbers to be the real numbers.
    ///
    /// What switching it on restores is listed on
    /// [`HumanSessionPolicy::new`](crate::human_behavior::HumanSessionPolicy::new) — a
    /// per-hour ceiling of 8–16 likes, a two-of-the-last-five-cards rule, a 12–35 s wait
    /// after every action, rests, and block breaks. Those are what make a run look like a
    /// person rather than a script, so off is faster, denser and more distinguishable. It
    /// is the operator's trade to make, and `false` is what they chose.
    ///
    /// `#[serde(default)]` rather than `default = "yes"`, so a stored row written before
    /// this existed reads as off too — the same answer the operator gave.
    #[serde(default)]
    pub human_limits: bool,

    // ---- photo carousel ----
    /// Hard ceiling on slides paged through in one carousel.
    ///
    /// A ceiling rather than a target: the traversal stops as soon as a swipe **fails to
    /// change the screen**, which is the real end of the carousel. This only bounds a post
    /// that never stops changing — a video misread as a photo, or a card that animates.
    #[serde(default = "default_carousel_max_slides")]
    pub carousel_max_slides: u32,
    /// How much of a photo post to look at, as a percentage. 100 means "to the end".
    ///
    /// **What 50 means depends on what the backend can see, and that is measured rather
    /// than a design choice:**
    ///
    /// * Android — half of *this post*. The page counter is on screen and readable, so the
    ///   total is known before the first sideways swipe.
    /// * iOS — half of the **ceiling**, because the pixel engine has no counter to read and
    ///   learns the end of a carousel only from a swipe that fails to change the frame.
    ///
    /// The earlier note here said no counter existed anywhere — "a dump of every `TextView`
    /// on a photo post contains no `1 / 7` counter", measured 11/08/2026 — and the Android
    /// traversal was left unbuilt on that basis. **It was wrong.** The counter is split
    /// across three sibling nodes, `"1"`, `" / "`, `"5"`, so a search for one node
    /// containing a slash finds nothing. Re-measured on the same SM-N950F on 12/08/2026
    /// against two real photo posts: it advances per image and disappears after the last.
    #[serde(default = "default_carousel_portion_percent")]
    pub carousel_portion_percent: u32,
}

fn yes() -> bool {
    true
}

fn default_carousel_max_slides() -> u32 {
    12
}

fn default_carousel_portion_percent() -> u32 {
    100
}

impl Default for NurtureSettings {
    fn default() -> Self {
        Self {
            // OpenRouter chat/completions + Luna vision. The operator only
            // fills the key. A custom gateway still works by changing these.
            base_url: "https://openrouter.ai/api/v1".into(),
            model: "openai/gpt-5.6-luna".into(),
            api_key: String::new(),
            // OpenRouter's OpenAI route listed $0.10 / $0.60 on 14/08/2026
            // (50% off the $0.20 / $1.20 list). Display only; the panel can
            // edit these if the promo ends.
            input_price_per_1m: 0.10,
            output_price_per_1m: 0.60,
            bundle_id: "com.ss.iphone.ugc.Ame".into(),
            // Manual runs use a varied 2–3 hour horizon; this remains the
            // legacy fixture ceiling for callers that do not pass a duration.
            num_videos: 120,
            num_rounds: 1,
            like_prob: 35,
            // Comments are opt-in because a fresh install has no AI key. Once
            // a key is configured, the operator can enable a small comment rate.
            comment_prob: 0,
            follow_prob: 3,
            frenzy_prob: 6,
            watch_min: 3.0,
            watch_max: 18.0,
            persona: "casual".into(),
            fatigue: true,
            time_of_day: true,
            pause_swipe: true,
            night_start: 0,
            night_end: 0,
            recover_delay_min: 2,
            recover_delay_max: 4,
            stagger_delay_min: 5,
            stagger_delay_max: 15,
            comment_lang: "vi".into(),
            ai_directions: "Tự nhiên|Thân mật|Hơi hài|Ngắn gọn".into(),
            max_comment_words: 12,
            schedule_enabled: false,
            // Scheduling is disabled by default. If enabled, use human-sized
            // blocks instead of short fixed bursts on the hour.
            schedule_every_minutes: 240,
            schedule_duration_minutes: 150,
            schedule_udids: Vec::new(),
            steady_mood: String::new(),
            like_enabled: true,
            comment_enabled: true,
            follow_enabled: true,
            frenzy_enabled: true,
            carousel_enabled: true,
            human_limits: false,
            carousel_max_slides: default_carousel_max_slides(),
            carousel_portion_percent: default_carousel_portion_percent(),
        }
    }
}

impl NurtureSettings {
    /// Fold the per-feature switches **into** the probabilities.
    ///
    /// Applied once when a session takes its snapshot and again on every live refresh, so
    /// the loop only ever sees effective values. That is deliberate and it is the whole
    /// safety argument for the switches: there are a dozen places that read `like_prob`,
    /// `comment_prob`, `follow_prob` and `frenzy_prob`, and asking each of them to also
    /// check a flag is asking for one of them to be forgotten — which would be a feature
    /// the operator switched off still happening.
    ///
    /// The switch flags are left as they are, so the UI still round-trips what the operator
    /// chose; only the working copy's probabilities are zeroed.
    pub fn into_effective(mut self) -> Self {
        if !self.like_enabled {
            self.like_prob = 0;
        }
        if !self.comment_enabled {
            self.comment_prob = 0;
        }
        if !self.follow_enabled {
            self.follow_prob = 0;
        }
        if !self.frenzy_enabled {
            self.frenzy_prob = 0;
        }
        if !self.carousel_enabled {
            self.carousel_max_slides = 0;
        }
        self
    }

    /// The safety ceiling on its own, with the portion **not** folded in.
    ///
    /// For a backend that can read how many images a post has, which is Android: the
    /// portion is applied to the post's real total there, so folding it into the ceiling
    /// as well would apply it twice. `0` still means the switch is off.
    ///
    /// [`Self::carousel_slide_budget`] is the other half of this split and belongs to the
    /// pixel engine, which has no total to take a fraction of.
    pub fn carousel_ceiling(&self) -> u32 {
        if !self.carousel_enabled {
            return 0;
        }
        self.carousel_max_slides
    }

    /// How many slides one carousel may be paged through, **portion already folded in**.
    ///
    /// `0` means "do not page at all" — the carousel switch is off, and the caller should
    /// treat the post as a single card. Otherwise at least one slide, so a portion rounded
    /// down to nothing still looks at something.
    ///
    /// This is the pixel engine's number, and folding the portion into the ceiling is a
    /// concession to what that engine can see: it learns the end of a carousel only by a
    /// swipe failing to change the frame, so "half" cannot mean half of anything it knows.
    /// A backend that reads the counter should take [`Self::carousel_ceiling`] instead.
    pub fn carousel_slide_budget(&self) -> u32 {
        if !self.carousel_enabled || self.carousel_max_slides == 0 {
            return 0;
        }
        let portion = self.carousel_portion_percent.min(100);
        let budget = (u64::from(self.carousel_max_slides) * u64::from(portion) / 100) as u32;
        budget.max(1)
    }

    /// Copy the fields a **running** session is allowed to pick up, leaving the rest.
    ///
    /// The split is the answer to "which parameters can change mid-run", and it is not
    /// arbitrary. What is copied is a knob: a probability, a duration, a switch — reading a
    /// new value on the next post is indistinguishable from the operator having set it
    /// before that post.
    ///
    /// What is **not** copied is anything the session already built something out of:
    ///
    /// * `num_videos` / `num_rounds` — the session's target was computed at start; moving
    ///   it underneath a run in progress makes "42 of 120" meaningless.
    /// * `persona` — `HumanBehavior` is constructed from it once, so a change would need
    ///   the behaviour model rebuilt mid-session.
    /// * `steady_mood` — the mood cycle is already built.
    /// * `bundle_id` — the app is already open, and on Android the package is resolved per
    ///   device anyway.
    /// * `schedule_*` and `stagger_delay_*` — they act *between* sessions, not inside one.
    ///
    /// The UI marks those as needing a restart, so this list and that list are the same
    /// list, and this doc is where it is written down.
    pub fn absorb_live_changes(&mut self, fresh: &NurtureSettings) {
        // Behaviour knobs.
        self.like_prob = fresh.like_prob;
        self.comment_prob = fresh.comment_prob;
        self.follow_prob = fresh.follow_prob;
        self.frenzy_prob = fresh.frenzy_prob;
        self.like_enabled = fresh.like_enabled;
        self.comment_enabled = fresh.comment_enabled;
        self.follow_enabled = fresh.follow_enabled;
        self.frenzy_enabled = fresh.frenzy_enabled;
        self.watch_min = fresh.watch_min;
        self.watch_max = fresh.watch_max;
        self.fatigue = fresh.fatigue;
        self.time_of_day = fresh.time_of_day;
        self.pause_swipe = fresh.pause_swipe;
        self.human_limits = fresh.human_limits;
        self.night_start = fresh.night_start;
        self.night_end = fresh.night_end;
        self.recover_delay_min = fresh.recover_delay_min;
        self.recover_delay_max = fresh.recover_delay_max;
        // Carousel.
        self.carousel_enabled = fresh.carousel_enabled;
        self.carousel_max_slides = fresh.carousel_max_slides;
        self.carousel_portion_percent = fresh.carousel_portion_percent;
        // What the AI is told, and where it is asked. Read per comment, so a corrected key
        // or a reworded direction takes effect on the next one — which is the point.
        self.base_url = fresh.base_url.clone();
        self.model = fresh.model.clone();
        self.api_key = fresh.api_key.clone();
        self.input_price_per_1m = fresh.input_price_per_1m;
        self.output_price_per_1m = fresh.output_price_per_1m;
        self.comment_lang = fresh.comment_lang.clone();
        self.ai_directions = fresh.ai_directions.clone();
        self.max_comment_words = fresh.max_comment_words;
    }

    /// Upgrade the values shipped by the pre-human-v2 profile once.
    ///
    /// Only exact legacy defaults are replaced, so an operator's deliberate
    /// overrides (API endpoint, persona, language, or custom probabilities)
    /// remain intact. Unknown legacy fields are removed when the normalized
    /// profile is serialized back to the settings store.
    pub(crate) fn migrate_legacy_defaults(&mut self) -> bool {
        let defaults = Self::default();
        let mut changed = false;

        macro_rules! replace_if {
            ($field:ident, $legacy:expr) => {
                if self.$field == $legacy {
                    self.$field = defaults.$field.clone();
                    changed = true;
                }
            };
        }

        replace_if!(num_videos, 50);
        replace_if!(like_prob, 40);
        replace_if!(comment_prob, 25);
        replace_if!(follow_prob, 5);
        replace_if!(frenzy_prob, 8);
        if (self.watch_min, self.watch_max) == (5.0, 20.0) {
            self.watch_min = defaults.watch_min;
            self.watch_max = defaults.watch_max;
            changed = true;
        }
        replace_if!(schedule_every_minutes, 60);
        replace_if!(schedule_duration_minutes, 20);

        changed
    }

    /// One-time remap of the *shipped* DeepSeek pair onto OpenRouter Luna.
    ///
    /// Only `api.deepseek.com` + `deepseek-v4-flash` — the values a fresh
    /// install used to write. A custom model or host stays put. The API key
    /// is left alone; a DeepSeek key will not work on OpenRouter and the
    /// operator replaces it.
    pub(crate) fn adopt_openrouter_luna_if_still_shipped_deepseek(&mut self) -> bool {
        let host = crate::openai_client::host_of(&self.base_url);
        if !host.eq_ignore_ascii_case("api.deepseek.com")
            || self.model.trim() != "deepseek-v4-flash"
        {
            return false;
        }
        let defaults = Self::default();
        self.base_url = defaults.base_url;
        self.model = defaults.model;
        if (self.input_price_per_1m - 1.25).abs() < f64::EPSILON
            && (self.output_price_per_1m - 10.0).abs() < f64::EPSILON
        {
            self.input_price_per_1m = defaults.input_price_per_1m;
            self.output_price_per_1m = defaults.output_price_per_1m;
        }
        true
    }
}

#[cfg(test)]
mod nurture_settings_tests {
    use super::NurtureSettings;

    #[test]
    fn the_ceiling_and_the_portion_are_two_different_numbers() {
        // They are split because the two backends can see different things, and conflating
        // them applied the percentage twice on Android: once folded into the ceiling here,
        // once against the post's real image count in the traversal.
        let mut settings = NurtureSettings {
            carousel_max_slides: 12,
            carousel_portion_percent: 50,
            carousel_enabled: true,
            ..NurtureSettings::default()
        };
        assert_eq!(settings.carousel_ceiling(), 12, "the ceiling is untouched");
        assert_eq!(
            settings.carousel_slide_budget(),
            6,
            "the pixel engine's number has the portion folded in"
        );
        // The switch closes both doors.
        settings.carousel_enabled = false;
        assert_eq!(settings.carousel_ceiling(), 0);
        assert_eq!(settings.carousel_slide_budget(), 0);
    }

    #[test]
    fn defaults_allow_a_first_run_without_ai_credentials() {
        let settings = NurtureSettings::default();
        assert_eq!(settings.base_url, "https://openrouter.ai/api/v1");
        assert_eq!(settings.model, "openai/gpt-5.6-luna");
        assert!(settings.api_key.is_empty());
        assert_eq!(settings.comment_prob, 0);
        assert_eq!(settings.like_prob, 35);
        assert_eq!(settings.follow_prob, 3);
        assert_eq!(settings.frenzy_prob, 6);
        assert_eq!((settings.watch_min, settings.watch_max), (3.0, 18.0));
        assert!(!settings.schedule_enabled);
        assert_eq!(settings.schedule_every_minutes, 240);
        assert_eq!(settings.schedule_duration_minutes, 150);
    }

    #[test]
    fn legacy_defaults_migrate_without_touching_custom_fields() {
        let mut settings = NurtureSettings {
            api_key: "fixture-key".into(),
            model: "custom-model".into(),
            persona: "custom-persona".into(),
            num_videos: 50,
            like_prob: 40,
            comment_prob: 25,
            follow_prob: 5,
            frenzy_prob: 8,
            watch_min: 5.0,
            watch_max: 20.0,
            schedule_every_minutes: 60,
            schedule_duration_minutes: 20,
            ..Default::default()
        };

        assert!(settings.migrate_legacy_defaults());
        assert_eq!(settings.num_videos, 120);
        assert_eq!(settings.like_prob, 35);
        assert_eq!(settings.comment_prob, 0);
        assert_eq!(settings.follow_prob, 3);
        assert_eq!(settings.frenzy_prob, 6);
        assert_eq!((settings.watch_min, settings.watch_max), (3.0, 18.0));
        assert_eq!(settings.schedule_every_minutes, 240);
        assert_eq!(settings.schedule_duration_minutes, 150);
        assert_eq!(settings.api_key, "fixture-key");
        assert_eq!(settings.model, "custom-model");
        assert_eq!(settings.persona, "custom-persona");
        assert!(!settings.migrate_legacy_defaults());
    }

    #[test]
    fn shipped_deepseek_defaults_move_to_openrouter_luna_and_keep_the_key() {
        let mut settings = NurtureSettings {
            base_url: "https://api.deepseek.com/".into(),
            model: "deepseek-v4-flash".into(),
            api_key: "sk-or-keep-me".into(),
            input_price_per_1m: 1.25,
            output_price_per_1m: 10.0,
            like_prob: 80,
            ..NurtureSettings::default()
        };
        assert!(settings.adopt_openrouter_luna_if_still_shipped_deepseek());
        assert_eq!(settings.base_url, "https://openrouter.ai/api/v1");
        assert_eq!(settings.model, "openai/gpt-5.6-luna");
        assert_eq!(settings.api_key, "sk-or-keep-me");
        assert_eq!(settings.like_prob, 80);
        assert!((settings.input_price_per_1m - 0.10).abs() < f64::EPSILON);
        assert!((settings.output_price_per_1m - 0.60).abs() < f64::EPSILON);
        assert!(!settings.adopt_openrouter_luna_if_still_shipped_deepseek());
    }

    #[test]
    fn a_custom_deepseek_model_is_left_alone() {
        let mut settings = NurtureSettings {
            base_url: "https://api.deepseek.com".into(),
            model: "deepseek-v4-pro".into(),
            api_key: "ds-key".into(),
            ..NurtureSettings::default()
        };
        assert!(!settings.adopt_openrouter_luna_if_still_shipped_deepseek());
        assert_eq!(settings.model, "deepseek-v4-pro");
        assert_eq!(settings.api_key, "ds-key");
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NurtureCommentCost {
    pub id: String,
    pub udid: String,
    pub model: String,
    pub base_url_host: String,
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub usd: f64,
    pub preview: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NurtureCommentAttempt {
    pub id: String,
    pub udid: String,
    pub outcome: String,
    pub source: String,
    pub model: String,
    pub base_url_host: String,
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub usd: f64,
    pub preview: String,
    pub caption_preview: String,
    pub frame_sha256: String,
    pub context_confidence: Option<u8>,
    pub relevance: Option<u8>,
    pub evidence_support: Option<u8>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NurtureSessionStatus {
    pub udid: String,
    pub running: bool,
    /// Swipes that were *proven* to reach a new card. Every other counter here
    /// already comes in an attempted/succeeded pair; this one did not, so a
    /// swipe that went nowhere was indistinguishable from one that worked.
    pub videos_done: u32,
    /// Vertical feed swipes sent, confirmed or not. Photo-carousel slides are
    /// not counted: they move within one card, not to the next one.
    #[serde(default)]
    pub swipe_attempts: u32,
    #[serde(default)]
    pub like_attempts: u32,
    #[serde(default)]
    pub comment_attempts: u32,
    #[serde(default)]
    pub follow_attempts: u32,
    pub likes: u32,
    pub comments: u32,
    pub follows: u32,
    pub last_message: String,
    pub session_usd: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NurtureCostSummary {
    pub today_usd: f64,
    pub total_usd: f64,
    pub today_comments: u32,
    pub total_comments: u32,
}

/// Tests for the per-feature switches, the carousel budget, and the live-refresh split.
///
/// A module of their own rather than folded into `nurture_settings_tests`, which predates
/// them and covers defaults and the legacy migration.
#[cfg(test)]
mod nurture_tuning_tests {
    use super::NurtureSettings;

    #[test]
    fn an_old_stored_profile_upgrades_with_every_feature_still_on() {
        // Deliberately missing every field added for the switches and the carousel.
        let legacy = r#"{
            "baseUrl":"https://api.deepseek.com","model":"m","apiKey":"",
            "inputPricePer1m":1.0,"outputPricePer1m":2.0,
            "bundleId":"com.ss.iphone.ugc.Ame","numVideos":10,"numRounds":1,
            "likeProb":35,"commentProb":4,"followProb":3,"frenzyProb":6,
            "watchMin":3.0,"watchMax":18.0,"persona":"casual",
            "fatigue":true,"timeOfDay":true,"pauseSwipe":true,
            "nightStart":0,"nightEnd":0,
            "recoverDelayMin":2,"recoverDelayMax":4,
            "staggerDelayMin":5,"staggerDelayMax":15,
            "commentLang":"vi","aiDirections":"x","maxCommentWords":12,
            "scheduleEnabled":false,"scheduleEveryMinutes":240,
            "scheduleDurationMinutes":150
        }"#;
        let settings: NurtureSettings = serde_json::from_str(legacy).expect("legacy profile");
        assert!(settings.like_enabled);
        assert!(settings.comment_enabled);
        assert!(settings.follow_enabled);
        assert!(settings.frenzy_enabled);
        assert!(settings.carousel_enabled);
        assert_eq!(settings.carousel_max_slides, 12);
        assert_eq!(settings.carousel_portion_percent, 100);
        // And the tuned numbers survive untouched.
        let effective = settings.into_effective();
        assert_eq!(effective.like_prob, 35);
        assert_eq!(effective.comment_prob, 4);
    }

    #[test]
    fn a_switch_keeps_the_tuned_number_while_stopping_the_behaviour() {
        // The entire reason a switch exists separately from the probability: pausing a
        // feature must not make the operator remember what 35 was.
        let mut settings = NurtureSettings {
            like_prob: 35,
            comment_prob: 4,
            follow_prob: 3,
            frenzy_prob: 6,
            like_enabled: false,
            comment_enabled: false,
            follow_enabled: false,
            frenzy_enabled: false,
            ..Default::default()
        };
        let stored = settings.clone();
        settings = settings.into_effective();
        // The loop sees zero, so no call site has to know the switch exists.
        assert_eq!(settings.like_prob, 0);
        assert_eq!(settings.comment_prob, 0);
        assert_eq!(settings.follow_prob, 0);
        assert_eq!(settings.frenzy_prob, 0);
        // The stored profile still carries the numbers, so the UI round-trips them.
        assert_eq!(stored.like_prob, 35);
    }

    #[test]
    fn the_carousel_budget_is_a_portion_of_the_ceiling_and_never_zero_when_on() {
        let settings = |max, portion, on| NurtureSettings {
            carousel_max_slides: max,
            carousel_portion_percent: portion,
            carousel_enabled: on,
            ..Default::default()
        };
        assert_eq!(settings(12, 100, true).carousel_slide_budget(), 12);
        assert_eq!(settings(12, 50, true).carousel_slide_budget(), 6);
        assert_eq!(settings(7, 50, true).carousel_slide_budget(), 3);
        // A portion that rounds to nothing still looks at one slide — "half" must not mean
        // "none" on a short ceiling.
        assert_eq!(settings(1, 10, true).carousel_slide_budget(), 1);
        // Over 100 is clamped rather than trusted.
        assert_eq!(settings(4, 250, true).carousel_slide_budget(), 4);
        // Off means off, and the loop reads 0 as "treat it as one card".
        assert_eq!(settings(12, 100, false).carousel_slide_budget(), 0);
        assert_eq!(settings(0, 100, true).carousel_slide_budget(), 0);
        // And `into_effective` agrees with the accessor, so the loop cannot get two
        // different answers depending on which one it asked.
        assert_eq!(
            settings(12, 100, false)
                .into_effective()
                .carousel_max_slides,
            0
        );
    }

    #[test]
    fn a_live_refresh_moves_the_knobs_and_leaves_the_structure_alone() {
        // The split that answers "what can change mid-run". Everything asserted as
        // unchanged here is something the session already built on: a target it counts
        // against, a behaviour model it constructed, or an app it already opened.
        let mut running = NurtureSettings {
            like_prob: 10,
            num_videos: 120,
            num_rounds: 2,
            persona: "casual".into(),
            steady_mood: "chatty".into(),
            bundle_id: "com.ss.iphone.ugc.Ame".into(),
            stagger_delay_min: 5,
            schedule_every_minutes: 240,
            ..Default::default()
        };
        let fresh = NurtureSettings {
            like_prob: 80,
            comment_prob: 9,
            watch_max: 30.0,
            carousel_portion_percent: 50,
            follow_enabled: false,
            max_comment_words: 20,
            api_key: "corrected".into(),
            // Structural: all different, all of which must be ignored.
            num_videos: 1,
            num_rounds: 99,
            persona: "hype".into(),
            steady_mood: "skimming".into(),
            bundle_id: "com.zhiliaoapp.musically".into(),
            stagger_delay_min: 999,
            schedule_every_minutes: 1,
            ..Default::default()
        };
        running.absorb_live_changes(&fresh);

        assert_eq!(running.like_prob, 80, "a probability is a knob");
        assert_eq!(running.comment_prob, 9);
        assert_eq!(running.watch_max, 30.0);
        assert_eq!(running.carousel_portion_percent, 50);
        assert!(!running.follow_enabled, "a switch is a knob");
        assert_eq!(running.max_comment_words, 20);
        assert_eq!(
            running.api_key, "corrected",
            "a corrected key must take effect"
        );

        assert_eq!(
            running.num_videos, 120,
            "the session's target must not move"
        );
        assert_eq!(running.num_rounds, 2);
        assert_eq!(
            running.persona, "casual",
            "HumanBehavior was built from this"
        );
        assert_eq!(
            running.steady_mood, "chatty",
            "the mood cycle is already built"
        );
        assert_eq!(
            running.bundle_id, "com.ss.iphone.ugc.Ame",
            "the app is already open"
        );
        assert_eq!(
            running.stagger_delay_min, 5,
            "acts between sessions, not inside one"
        );
        assert_eq!(running.schedule_every_minutes, 240);
    }
}
