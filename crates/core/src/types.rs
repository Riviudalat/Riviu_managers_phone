use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::nurture::Outcome;

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
pub struct DeviceMeta {
    pub udid: String,
    pub notes: String,
    pub tags: Vec<String>,
    pub group_id: Option<String>,
    /// The TikTok @handle this phone is logged into, without the leading `@`.
    ///
    /// Entered by the operator (there is no reliable on-device read for it). Empty when
    /// unknown. It is what lets an interaction @-mention resolve to the phone that owns the
    /// account, so tagging `@name` can bring that phone into the same post to reply — see
    /// `interaction::ThreadCampaignRequest::mentions`.
    #[serde(default)]
    pub handle: String,
    /// What the operator calls this phone (xiaowei "Change Name"). Empty means "use the name
    /// the phone reports", which is what the grid showed before this existed.
    ///
    /// Deliberately not written back to the device. Renaming the *phone* needs root on
    /// Android and is a fingerprint change; what an operator wants when they rename a tile is
    /// to tell twenty identical SM-G955Fs apart, and that is a fact about this app's records.
    #[serde(default)]
    pub alias: String,
    /// The number written on the phone, on the shelf, in the operator's notes (xiaowei
    /// "Change Number").
    ///
    /// `None` means unnumbered, and the grid then shows the tile's position instead — which
    /// is what every tile showed before, and is exactly the thing a number is for replacing:
    /// a position changes when the fleet list changes, so it cannot be written on a sticker.
    #[serde(default)]
    pub number: Option<u32>,
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

/// What one row of a device directory listing is.
///
/// `Other` is not a failure and not a leftover: a phone's filesystem holds sockets, fifos
/// and block devices, and a browser that dropped every row it did not recognise would show
/// a directory as empty when it is not. The UI can refuse to *open* one; it must still say
/// it is there.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DeviceFileKind {
    File,
    Directory,
    /// A symlink. Which of the two it points at is deliberately not resolved here — that
    /// costs a second `ls` per row — so the browser follows it by listing its path and
    /// letting the phone answer.
    Symlink,
    Other,
}

/// One entry in a phone's own directory listing (xiaowei "Preview Mobile Files").
///
/// Every field is what the phone printed, never something computed here. `modified` is the
/// phone's own `YYYY-MM-DD HH:MM` string rather than a parsed timestamp, and that is
/// deliberate: `ls` prints in the *device's* timezone with no offset, so turning it into an
/// instant here would invent a precision the source does not have. Measured on
/// 23021RAAEG (Android 15) and SM-G955F (Android 9): both print exactly that shape.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceFileEntry {
    /// The bare name, not the path — `Download`, not `/sdcard/Download`.
    pub name: String,
    pub kind: DeviceFileKind,
    /// Bytes for a file; for a directory this is the size of the directory *inode*
    /// (3452 on this fleet's sdcard), which is why the UI shows it only for files.
    pub size: u64,
    /// `None` when the phone printed `?` for it, which happens on rows it cannot stat —
    /// a dangling symlink under `/`, measured on 23021RAAEG.
    pub modified: Option<String>,
    /// Present only for `Symlink`, and exactly the text after `->`.
    pub link_target: Option<String>,
}

/// One directory as the phone described it, including what it would not describe.
///
/// **`incomplete` exists because a short list used to read as a whole one.** `ls -la` on a
/// directory it can only partly read prints the rows it managed and complains about the rest;
/// the browser drew those rows with nothing to say more existed. A folder that looks complete
/// and is not is worse than an error: an operator deletes from it, exports from it, and
/// concludes things about it.
///
/// A refusal is not represented here at all -- it comes back as an `Err`, because there is no
/// listing to draw. That distinction is the whole point: an empty `entries` with `incomplete:
/// None` means the directory really is empty.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceDirListing {
    pub entries: Vec<DeviceFileEntry>,
    /// What the phone said that the rows do not show. `Some` means **incomplete**.
    pub incomplete: Option<String>,
}

/// The two separate answers to "is this phone rooted", because they are not the same
/// question and this fleet disagrees on nine of twenty phones.
///
/// Measured 27/08/2026: the nine SM-G950F have `adbd` running as uid 0 (`context=u:r:su:s0`)
/// and **no `su` binary at all**; the eleven SM-G955F/N/U1 run as uid 2000 and also have no
/// `su`. So "can run a privileged command" is true on nine of them while "has `su`" is true on
/// none — and a single boolean had to pick one meaning and mislead about the other.
///
/// `factory_reset` is gated on `has_su` specifically, so the UI must not present a phone as
/// simply "rooted" when the wipe button will still refuse it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceRootStatus {
    /// A `su` that grants uid 0. This is what `factory_reset` requires.
    pub has_su: bool,
    /// The adb shell is already uid 0, so a privileged command runs without `su`.
    pub shell_is_root: bool,
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
/// **That route is now built** (21/08/2026): the helper answers `/v1/apps/describe` with the
/// label and a rendered icon for a list of packages, and the Android driver joins it onto the
/// adb listing. The paragraph above still describes what happens on a phone *without* the
/// helper, which is why `label` stays an `Option` — the fallback is a package name, never an
/// invented one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstalledApp {
    /// `bundle_id` and not `package`: the house term across `uninstall_app`,
    /// `install_library_app` and the whole iOS path, and the same string both platforms
    /// use to name an app.
    pub bundle_id: String,
    pub kind: InstalledAppKind,
    /// Human-readable name where the platform gives one, or the helper can read one.
    /// `None` means "nothing on this phone could tell us", not "unnamed".
    pub label: Option<String>,
    /// The app's icon as a base64 PNG, at the size the helper rendered it (48 px edge).
    ///
    /// `None` is the ordinary case for a phone with no helper, and also for the handful of
    /// system packages that genuinely have no icon — never a placeholder image, so the UI can
    /// draw its own neutral square instead of showing one the phone did not give.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon_png_base64: Option<String>,
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

/// The part of a window that overrides how a session behaves, when it overrides it at all.
///
/// **All five or none**, rather than five independent `Option`s. A window that carries a
/// half-set of rates would leave the other half inheriting a global the operator edited
/// later, and the four rates here share one 100% budget (`nurtureBudget.ts`) — a budget
/// assembled from two sources is a budget nobody can read off the screen. One switch, one
/// complete block: either this window behaves like the panel above it, or it behaves like
/// exactly what is written in it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct NurtureWindowBehaviour {
    pub num_videos: u32,
    pub num_rounds: u32,
    pub like_prob: u32,
    pub comment_prob: u32,
    pub follow_prob: u32,
}

impl Default for NurtureWindowBehaviour {
    fn default() -> Self {
        let base = NurtureSettings::default();
        Self {
            num_videos: base.num_videos,
            num_rounds: base.num_rounds,
            like_prob: base.like_prob,
            comment_prob: base.comment_prob,
            follow_prob: base.follow_prob,
        }
    }
}

/// One stretch of the local day the schedule may run in.
///
/// Times are **minutes from local midnight**, because that is the number the operator is
/// thinking in when they type `08:00` — the mark that says when the next run is due stays a
/// UTC instant. `end_minute <= start_minute` means the window wraps past midnight, which is
/// how "22:00 tới 02:00" is written.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct NurtureWindow {
    /// Stable across edits, so the mark for "when is this window next due" survives the
    /// operator reordering the list or changing the hours.
    pub id: String,
    pub start_minute: u32,
    pub end_minute: u32,
    /// How often a run starts *inside* the window.
    pub every_minutes: u32,
    /// The cap handed to each session this window starts.
    pub duration_minutes: u32,
    /// Empty means every connected phone, and the editor says so in words rather than
    /// leaving a blank that reads as "none".
    pub udids: Vec<String>,
    /// `None` means "behave like the panel above".
    pub behaviour: Option<NurtureWindowBehaviour>,
}

impl Default for NurtureWindow {
    fn default() -> Self {
        Self {
            id: String::new(),
            start_minute: 8 * 60,
            end_minute: 11 * 60,
            every_minutes: 60,
            duration_minutes: 20,
            udids: Vec::new(),
            behaviour: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct NurtureSettings {
    pub base_url: String,
    pub model: String,
    pub api_key: String,
    /// Whether a key is stored, for a form that must not receive the key itself.
    ///
    /// `#[serde(skip_serializing_if)]`-free on purpose: the frontend reads it on every load.
    /// Derived at the command boundary, never persisted — `save_nurture_settings` writes the
    /// key to the OS credential store and the settings blob keeps neither it nor this flag
    /// meaningfully.
    #[serde(default)]
    pub has_api_key: bool,
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
    /// Stretches of the day the schedule may run in, each with its own cadence.
    ///
    /// **Empty keeps the old single-cadence behaviour**, which is what every database written
    /// before this field existed contains: run every `schedule_every_minutes`, all day, on
    /// `schedule_udids`. That fallback is not a deprecation shim to be removed quietly — it is
    /// what an operator who never opens the window editor still gets.
    #[serde(default)]
    pub schedule_windows: Vec<NurtureWindow>,
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
            has_api_key: false,
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
            schedule_windows: Vec::new(),
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
            like_prob: 80,
            ..NurtureSettings::default()
        };
        assert!(settings.adopt_openrouter_luna_if_still_shipped_deepseek());
        assert_eq!(settings.base_url, "https://openrouter.ai/api/v1");
        assert_eq!(settings.model, "openai/gpt-5.6-luna");
        assert_eq!(settings.api_key, "sk-or-keep-me");
        assert_eq!(settings.like_prob, 80);
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
pub struct NurtureCommentAttempt {
    pub id: String,
    pub udid: String,
    pub outcome: String,
    pub source: String,
    pub model: String,
    pub base_url_host: String,
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    /// What the gateway said this attempt cost, in USD, summed over every call it took.
    ///
    /// **`None` means the gateway did not say, and that is not the same as free.** The column
    /// this feeds is nullable for exactly that reason — see `apply_migration_15`, and see
    /// migration 11 for the fabricated column whose zeroes this must never become.
    pub cost_usd: Option<f64>,
    pub preview: String,
    pub caption_preview: String,
    pub frame_sha256: String,
    pub context_confidence: Option<u8>,
    pub relevance: Option<u8>,
    pub evidence_support: Option<u8>,
    /// How many *different* frames the model was shown. `Some(1)` on a photo post, where the
    /// three samples were one byte-identical picture; `Some(0)` on the caption-only path,
    /// which sends no picture at all; `None` on rows written before this was recorded.
    ///
    /// It is here so `evidence_support` can be read. A low score next to `1` means there was
    /// only ever one frame of evidence; the same score next to `3` means the model read three
    /// and still could not ground the comment. Those are different problems.
    pub distinct_frames: Option<u8>,
    /// Slides the carousel traversal paged before this comment was written, duplicates
    /// included. `Some(0)` on a post that was never paged; `None` on rows from before it was
    /// recorded.
    ///
    /// Read it next to [`Self::distinct_frames`], which is the pair that says anything:
    /// `carousel_slides = 7, distinct_frames = 1` means the pager turned seven times and the
    /// stream handed back one picture every time — a parked stream, and the comment is
    /// grounded on a seventh of the post. `7` and `2` means it is working.
    pub carousel_slides: Option<u32>,
    pub created_at: String,
}

/// Where one device is in its session, as a value rather than as a sentence.
///
/// **Every phase below already existed — as Vietnamese prose in `last_message`.** A bar
/// drawn from `videos_done` alone reads 0% for the first minute of a healthy run (up to 40 s
/// waiting for TikTok to reach the foreground, then up to 30 s waiting for the feed), and it
/// reads exactly the same 0% for a phone that failed to open the app at all. That is the
/// case this enum exists for: the two lock-screen phones on 23/08/2026 died inside that
/// window, and no number could have told them apart from a phone that was merely starting.
///
/// Ordered roughly as a session passes through them, so a UI can render them as a track.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum NurturePhase {
    /// Accepted, waiting out its stagger delay. Not yet touching the phone.
    #[default]
    Queued,
    /// Opening a control session and bringing TikTok to the front.
    Opening,
    /// Session up, working back to a usable feed — declining dialogs, skipping the
    /// onboarding journey, finding the action rail.
    AwaitingFeed,
    /// The feed loop proper: watching, liking, commenting, swiping.
    Watching,
    /// Spending recovery budget after a failure. Distinct from [`Self::AwaitingFeed`]
    /// because it means something already went wrong, which is worth seeing.
    Recovering,
    /// Terminal. Pair with [`NurtureSessionStatus::outcome`] for the verdict.
    Finished,
}

impl NurturePhase {
    /// Whether nothing more will happen on this device.
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Finished)
    }
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
    /// What the comment model actually reported spending on this device, in tokens.
    ///
    /// **Tokens and not money, because money was fabricated.** This used to be `session_usd`,
    /// the product of two hand-typed per-million prices that were never sent to the API and
    /// existed in three different values at once — `types.rs` said $0.10/$0.60, `db.rs` said
    /// $1.25/$10.00, and a migration rewrote the second back to the first. No UI could edit
    /// them, so after any model change every USD figure in the database was silently wrong.
    /// Tokens come from the API's own `usage` object, which means they are true of whatever
    /// model is configured. Multiply by the provider's real rate outside the app.
    #[serde(default)]
    pub session_prompt_tokens: u32,
    #[serde(default)]
    pub session_completion_tokens: u32,
    /// Which run this row belongs to.
    ///
    /// **Without this there is no such thing as "the current run".** `set_status` inserts by
    /// udid and nothing ever removes an entry, so the status list accumulates every phone
    /// that has run since the process started — a fleet total summed over it already
    /// includes finished phones from earlier runs, and restarting one phone makes a fleet
    /// bar go *backwards* because that row's counters reset to zero while the others keep
    /// their finished values. Flow runs already carry a `run_id` for the same reason.
    #[serde(default)]
    pub run_id: Option<Uuid>,
    /// How many devices were started together in this run.
    ///
    /// The denominator for an overall bar, and it must be this rather than the number of
    /// rows present: a phone that failed before producing a second status still occupies a
    /// slot, and one that never produced a row at all must not shrink the total.
    #[serde(default)]
    pub run_size: u32,
    #[serde(default)]
    pub phase: NurturePhase,
    /// The verdict, once there is one. See [`crate::Outcome`].
    #[serde(default)]
    pub outcome: Option<Outcome>,
    /// Posts this session is aiming for — `num_videos × num_rounds`, snapshotted at start.
    ///
    /// **The denominator has to travel with the numerator.** `num_videos` is deliberately
    /// not absorbed by a running session, so a frontend dividing by the *live* settings row
    /// would rescale the bar under a session that never changed: lower "Giới hạn video"
    /// from 120 to 15 mid-run and the loop keeps counting to 120 while the UI divides by 15,
    /// which reads 800%.
    #[serde(default)]
    pub video_target: u32,
    /// When this device's session actually began — after its stagger, before the app opened.
    #[serde(default)]
    pub started_at: Option<DateTime<Utc>>,
    /// When the wall clock will end this session regardless of the video count.
    ///
    /// A run ends at **whichever bound arrives first**, and for a manual start this one is a
    /// randomised 2–3 hour horizon that was previously invisible to the UI entirely. A bar
    /// drawn from the video count alone stalls at 40% on a run that is about to finish on
    /// time and reads as hung.
    #[serde(default)]
    pub deadline_at: Option<DateTime<Utc>>,
}

impl Default for NurtureSessionStatus {
    fn default() -> Self {
        Self {
            udid: String::new(),
            running: false,
            videos_done: 0,
            swipe_attempts: 0,
            like_attempts: 0,
            comment_attempts: 0,
            follow_attempts: 0,
            likes: 0,
            comments: 0,
            follows: 0,
            last_message: String::new(),
            session_prompt_tokens: 0,
            session_completion_tokens: 0,
            run_id: None,
            run_size: 0,
            phase: NurturePhase::Queued,
            outcome: None,
            video_target: 0,
            started_at: None,
            deadline_at: None,
        }
    }
}

impl NurtureSessionStatus {
    /// A fresh row for one device, with everything else at its default.
    ///
    /// Exists so the eight construction sites that spelled out all twelve fields — four of
    /// them in one file, identical apart from `last_message` — do not each have to grow a
    /// line every time a field is added. That churn is how a field ends up set in three
    /// places and forgotten in the fourth.
    pub fn new(udid: impl Into<String>) -> Self {
        Self {
            udid: udid.into(),
            ..Self::default()
        }
    }

    /// Close this row out: terminal phase, `running` down, and the verdict *beside* the
    /// summary sentence rather than only inside it.
    ///
    /// One method rather than three assignments at ten sites, and that is not tidiness. The
    /// verdict used to be stringified into the first token of a Vietnamese sentence and
    /// then dropped, so a phone that finished 47 videos and one that never opened the app
    /// were both a grey row with prose in it. Ten separate exits each setting
    /// `running = false` is exactly the shape where the eleventh forgets — which is why
    /// `SessionCtx::push` now debug-asserts that a stopped row carries a verdict.
    pub fn finish(&mut self, outcome: Outcome) {
        self.running = false;
        self.phase = NurturePhase::Finished;
        self.outcome = Some(outcome);
    }

    /// How far along this device is, as a fraction in `0.0..=1.0`.
    ///
    /// **The maximum of the two bounds, because the session ends at whichever arrives
    /// first.** A count-only reading under-reports every timed run — it sits at 40% on a
    /// session with ten minutes left — and a clock-only reading under-reports a run that is
    /// about to hit its video cap in the first twenty minutes of a three-hour horizon.
    /// Taking the larger is the only reading that cannot promise more time than remains.
    ///
    /// Monotone by construction: `videos_done` only ever increments and the clock only ever
    /// advances, so this never goes backwards for a given run. A terminal phase reads 1.0
    /// whatever the counters say — a session that stopped at 40 of 120 videos is finished,
    /// and leaving its bar short would read as still working.
    ///
    /// `None` when there is nothing to divide by yet: a queued device with no target and no
    /// deadline is honestly *unknown*, not zero, and a bar should say so rather than draw an
    /// empty track that looks like a stall.
    pub fn progress_fraction(&self, now: DateTime<Utc>) -> Option<f64> {
        if self.phase.is_terminal() {
            return Some(1.0);
        }
        let (by_videos, by_clock) = self.bounds(now);
        let best = match (by_videos, by_clock) {
            (Some(a), Some(b)) => Some(a.max(b)),
            (Some(a), None) | (None, Some(a)) => Some(a),
            (None, None) => None,
        };
        best.map(|value| value.clamp(0.0, 1.0))
    }

    /// Which bound is currently governing, for a label beside the bar.
    ///
    /// Named rather than inferred by the UI: "42/120 video" and "còn 18 phút" are different
    /// sentences and only one of them is true of any given moment.
    ///
    /// A terminal row has no governing bound — it is over, and naming the bound that would
    /// have ended it next invites a label like "còn 18 phút" beside a finished session.
    pub fn governing_bound(&self, now: DateTime<Utc>) -> Option<NurtureBound> {
        if self.phase.is_terminal() {
            return None;
        }
        match self.bounds(now) {
            (Some(videos), Some(clock)) if clock > videos + CLOCK_LABEL_LEAD => {
                Some(NurtureBound::Clock)
            }
            (Some(_), _) => Some(NurtureBound::Videos),
            (None, Some(_)) => Some(NurtureBound::Clock),
            (None, None) => None,
        }
    }

    /// The two fractions, each `None` when its bound is not known yet.
    ///
    /// One function because the two callers above had fourteen identical lines each, and a
    /// rule fixed in one copy and not the other is exactly the drift this repo keeps paying
    /// for. `None` rather than `0.0` throughout: a queued device with no target is *unknown*,
    /// and a bar that draws unknown as empty looks like a stall.
    fn bounds(&self, now: DateTime<Utc>) -> (Option<f64>, Option<f64>) {
        let by_videos =
            (self.video_target > 0).then(|| self.videos_done as f64 / self.video_target as f64);
        let by_clock = match (self.started_at, self.deadline_at) {
            (Some(started), Some(deadline)) => {
                let total = (deadline - started).num_seconds();
                // A non-positive window is nonsense, not zero progress: a deadline at or
                // before the start would divide by zero or invert the fraction.
                (total > 0).then(|| (now - started).num_seconds().max(0) as f64 / total as f64)
            }
            _ => None,
        };
        (by_videos, by_clock)
    }
}

/// How far ahead the clock must be before it is called the governing bound.
///
/// Without it the clock wins the instant a run starts — `videos_done` is 0, so any elapsed
/// second beats it — and the operator's first reading was "còn ~154 phút" rather than the
/// "0/5 video" they had just typed. Measured on the live fleet on 23/08/2026. The *fraction*
/// still takes the plain maximum; this only decides which sentence is printed.
const CLOCK_LABEL_LEAD: f64 = 0.05;

/// Which of a session's two bounds is closer to ending it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum NurtureBound {
    /// The video count is ahead — the run will end on posts watched.
    Videos,
    /// The wall clock is ahead — the run will end on time, short of its video target.
    Clock,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NurtureCostSummary {
    /// Tokens the comment model reported for today's and all-time attempts.
    ///
    /// Was `today_usd`/`total_usd`, computed from prices the app could not know. See
    /// [`NurtureSessionStatus::session_prompt_tokens`].
    pub today_prompt_tokens: u64,
    pub today_completion_tokens: u64,
    pub total_prompt_tokens: u64,
    pub total_completion_tokens: u64,
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

    /// Rust field names `absorb_live_changes` copies, in the frontend's spelling.
    fn fields_absorbed_live() -> std::collections::BTreeSet<String> {
        // **Normalised, because a CRLF checkout makes this scan silently unmatchable.**
        //
        // `rustc` folds CRLF to LF inside string literals, so the needle this splits on is
        // LF-terminated whatever line endings this file has on disk — but `include_str!`
        // hands back the bytes as they sit. On a machine with `core.autocrlf=true` the two
        // stop agreeing, the split finds nothing, and the panic reads as
        // *absorb_live_changes was renamed* on a tree where nothing was renamed.
        //
        // **CI never sees it.** The workflow checks out with `core.autocrlf false`, so this
        // fails only on a developer's own clone — measured 26/08/2026, right after a merge
        // rewrote the tree, on a change that touched neither this file nor the one it scans.
        let source = &include_str!("types.rs").replace("\r\n", "\n");
        let body = source
            .split_once("pub fn absorb_live_changes(&mut self, fresh: &NurtureSettings) {")
            .expect("absorb_live_changes is still declared with that signature")
            .1
            .split_once(
                "
    }
",
            )
            .expect("absorb_live_changes ends at a top-level brace")
            .0;
        body.lines()
            .filter_map(|line| {
                let assignment = line.trim().strip_prefix("self.")?;
                let (field, rest) = assignment.split_once(" = ")?;
                // Only the plain `self.x = fresh.x` copies; anything computed is not a
                // straight absorption and would need reading, not pattern-matching.
                rest.starts_with("fresh.").then(|| camel(field))
            })
            .collect()
    }

    /// `recover_delay_min` -> `recoverDelayMin`.
    fn camel(snake: &str) -> String {
        let mut out = String::with_capacity(snake.len());
        let mut upper_next = false;
        for ch in snake.chars() {
            if ch == '_' {
                upper_next = true;
            } else if upper_next {
                out.extend(ch.to_uppercase());
                upper_next = false;
            } else {
                out.push(ch);
            }
        }
        out
    }

    #[test]
    fn the_form_promises_exactly_what_a_running_session_absorbs() {
        // `absorb_live_changes` is documented as "this list and that list are the same
        // list". They were not: this function copies `human_limits`, and the frontend's
        // LIVE_TUNABLE_FIELDS had never listed it. Nothing showed, because nothing read the
        // frontend list -- three restart badges hardcoded their reason strings instead.
        //
        // **The old version of this comment then claimed "now the badges read the list", and
        // that is not what happened.** The badges read `RESTART_REQUIRED_REASONS`, a separate
        // map keyed by field. So this test is still the *only* consumer of
        // LIVE_TUNABLE_FIELDS on the Rust side, which is exactly why it looks deletable from
        // TypeScript. `nurtureLiveFields.test.ts` now reads it from that side as well, and
        // asserts the two frontend lists never name the same field -- a field in both would
        // show a restart badge for a value this function had already absorbed.
        let types_ts = include_str!("../../../apps/desktop/src/types.ts");
        let declared: std::collections::BTreeSet<String> = types_ts
            .lines()
            .skip_while(|line| !line.contains("export const LIVE_TUNABLE_FIELDS"))
            .skip(1)
            .take_while(|line| !line.contains("]);"))
            .filter_map(|line| {
                let t = line.trim().trim_end_matches(',');
                t.strip_prefix('"')?.strip_suffix('"').map(str::to_owned)
            })
            .collect();
        assert!(
            !declared.is_empty(),
            "types.ts no longer declares LIVE_TUNABLE_FIELDS in a shape this test can read"
        );

        let absorbed = fields_absorbed_live();
        assert!(
            absorbed.len() > 20,
            "only found {} absorbed fields -- the parser lost the function body",
            absorbed.len()
        );
        assert_eq!(
            absorbed, declared,
            "the loop absorbs one set of fields and the form promises another"
        );
    }

    #[test]
    fn a_field_needing_a_restart_is_never_also_promised_as_live() {
        // The two lists are shown to the operator as opposites; an overlap would put a
        // "needs restart" badge on a field that does take effect immediately.
        let types_ts = include_str!("../../../apps/desktop/src/types.ts");
        let restart: Vec<&str> = types_ts
            .lines()
            .skip_while(|line| !line.contains("export const RESTART_REQUIRED_REASONS"))
            .skip(1)
            .take_while(|line| !line.starts_with('}'))
            .filter_map(|line| line.trim().split_once(':').map(|(k, _)| k.trim()))
            .collect();
        assert!(
            !restart.is_empty(),
            "RESTART_REQUIRED_REASONS is unreadable"
        );
        let live = fields_absorbed_live();
        for field in restart {
            assert!(
                !live.contains(field),
                "`{field}` is marked restart-required and is absorbed live"
            );
        }
    }

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

/// The frontend's copy of these types has to describe the same wire.
///
/// `apps/desktop/src/types.ts` is 1,300 lines written by hand against `types.rs`, and nothing
/// checked the two. Renaming a serde field is therefore not a compile error and not a test
/// failure — the frontend simply reads `undefined` at runtime, on whichever screen happens to
/// use that field, and the value renders as blank rather than as an error.
///
/// This does not replace the hand-written file with generated code; it holds it to the shape
/// of the Rust side, which is the part that was missing. Twenty-four types carry the same name
/// on both sides and all twenty-four agree today, so this starts green and stays useful.
#[cfg(test)]
mod wire_shape_tests {
    use std::collections::BTreeSet;

    /// `recover_delay_min` -> `recoverDelayMin`.
    fn camel(snake: &str) -> String {
        let mut out = String::with_capacity(snake.len());
        let mut upper = false;
        for ch in snake.chars() {
            if ch == '_' {
                upper = true;
            } else if upper {
                out.extend(ch.to_uppercase());
                upper = false;
            } else {
                out.push(ch);
            }
        }
        out
    }

    /// Field names each `#[derive(Serialize)]` struct puts on the wire.
    ///
    /// A line scanner rather than a parser, and deliberately conservative: a struct it cannot
    /// read is one it does not report, which the count assertion below then catches.
    fn rust_structs(source: &str) -> Vec<(String, BTreeSet<String>)> {
        let lines: Vec<&str> = source.lines().collect();
        let mut out = Vec::new();
        let mut i = 0;
        while i < lines.len() {
            let line = lines[i].trim_start();
            if !line.starts_with("pub struct ") || !line.ends_with('{') {
                i += 1;
                continue;
            }
            let name = line
                .trim_start_matches("pub struct ")
                .split([' ', '<', '{'])
                .next()
                .unwrap_or("")
                .to_string();

            // Attributes sit directly above the declaration.
            let mut attrs = String::new();
            let mut j = i;
            while j > 0 {
                let prev = lines[j - 1].trim();
                if prev.starts_with("#[") {
                    attrs.push_str(prev);
                    j -= 1;
                } else if prev.starts_with("///") || prev.starts_with("//") {
                    j -= 1;
                } else {
                    break;
                }
            }
            if !attrs.contains("Serialize") && !attrs.contains("Deserialize") {
                i += 1;
                continue;
            }
            let camel_all = attrs.contains("rename_all = \"camelCase\"");

            let mut fields = BTreeSet::new();
            let mut k = i + 1;
            let mut pending = String::new();
            while k < lines.len() && lines[k].trim() != "}" {
                let f = lines[k].trim();
                if f.starts_with("#[") {
                    pending.push_str(f);
                } else if let Some(rest) = f.strip_prefix("pub ") {
                    if let Some((field, _)) = rest.split_once(':') {
                        let field = field.trim();
                        let skipped =
                            pending.contains("skip") && !pending.contains("skip_serializing_if");
                        if !skipped && !field.is_empty() {
                            let renamed = pending
                                .split_once("rename = \"")
                                .and_then(|(_, r)| r.split_once('"'))
                                .map(|(v, _)| v.to_string());
                            fields.insert(match renamed {
                                Some(v) => v,
                                None if camel_all => camel(field),
                                None => field.to_string(),
                            });
                        }
                    }
                    pending.clear();
                } else if !f.is_empty() && !f.starts_with("//") {
                    pending.clear();
                }
                k += 1;
            }
            out.push((name, fields));
            i = k;
        }
        out
    }

    /// Field names each `export interface` declares.
    fn ts_interfaces(source: &str) -> Vec<(String, BTreeSet<String>)> {
        let lines: Vec<&str> = source.lines().collect();
        let mut out = Vec::new();
        let mut i = 0;
        while i < lines.len() {
            let line = lines[i].trim_start();
            if !line.starts_with("export interface ") || !line.ends_with('{') {
                i += 1;
                continue;
            }
            let name = line
                .trim_start_matches("export interface ")
                .split([' ', '<', '{'])
                .next()
                .unwrap_or("")
                .to_string();
            let mut fields = BTreeSet::new();
            let mut k = i + 1;
            while k < lines.len() && lines[k] != "}" {
                let f = lines[k].trim();
                let is_comment = f.starts_with("//") || f.starts_with("/*") || f.starts_with('*');
                if !is_comment {
                    if let Some((field, _)) = f.split_once(':') {
                        let field = field.trim().trim_end_matches('?');
                        if !field.is_empty()
                            && field.chars().all(|c| c.is_alphanumeric() || c == '_')
                        {
                            fields.insert(field.to_string());
                        }
                    }
                }
                k += 1;
            }
            out.push((name, fields));
            i = k;
        }
        out
    }

    /// **The same gate again, for the Flow models — which also had none.**
    ///
    /// `the_frontend_types_describe_the_same_fields_the_backend_sends` reads this file and
    /// `..._the_interaction_types_too` reads `interaction.rs`; the Flow wire shapes live in
    /// `flow/model.rs` and were compared by nothing. Two fields had already drifted when this was
    /// written — `CompiledFlowPlanV2.successors` and `FlowNodeAttemptRecord.chosenPort`, both live
    /// on the Rust side and both missing from TypeScript, which is why the run monitor had no typed
    /// way to say which branch an `IfVision` actually took.
    ///
    /// A third sibling rather than an extra `include_str!` in one of the others, for the reason
    /// given above: a single test reading three files reports drift without saying which half is
    /// ungated.
    #[test]
    fn the_frontend_types_match_the_flow_models_too() {
        /// Pairs whose two halves carry different names, so matching by name alone skips them.
        /// Listed rather than ignored: each is a shape that really does cross the wire, and an
        /// unmatched pair means this gate covers less than its count suggests.
        const RENAMED: [(&str, &str); 1] = [("ContextPlan", "FlowContextPlan")];

        let rust = rust_structs(&include_str!("flow/model.rs").replace("\r\n", "\n"));
        let ts = ts_interfaces(
            &include_str!("../../../apps/desktop/src/types.ts").replace("\r\n", "\n"),
        );

        let mut shared = 0;
        let mut drift = Vec::new();
        for (name, rust_fields) in &rust {
            let counterpart = RENAMED
                .iter()
                .find(|(from, _)| from == name)
                .map(|(_, to)| *to)
                .unwrap_or(name.as_str());
            let Some((_, ts_fields)) = ts.iter().find(|(n, _)| n == counterpart) else {
                continue;
            };
            shared += 1;
            let only_rust: Vec<_> = rust_fields.difference(ts_fields).cloned().collect();
            let only_ts: Vec<_> = ts_fields.difference(rust_fields).cloned().collect();
            if !only_rust.is_empty() || !only_ts.is_empty() {
                drift.push(format!(
                    "{name}: only in Rust {only_rust:?}, only in TypeScript {only_ts:?}"
                ));
            }
        }

        // 22 matched when this was written, of 24 serde structs in the file. A scanner that reads
        // nothing passes the assertion below it.
        assert!(
            shared >= 20,
            "only {shared} Flow models matched a TypeScript interface; the scanner has stopped \
             reading one of the two files (Rust structs seen: {}, TS interfaces seen: {})",
            rust.len(),
            ts.len()
        );
        assert!(
            drift.is_empty(),
            "the two halves of the Flow wire disagree:\n  {}",
            drift.join("\n  ")
        );
    }

    /// **The same gate, for the Interaction types — which had none at all.**
    ///
    /// `the_frontend_types_describe_the_same_fields_the_backend_sends` scans only this file, and
    /// every Interaction wire type lives in `interaction.rs`. So a field added on one side and
    /// forgotten on the other rendered as `undefined` with nothing to catch it: the campaign
    /// summary, the assignment record, the plan, the previews and the target note are all on
    /// that side of the line.
    ///
    /// Split from the test below rather than folded into it, because the two read different
    /// files and one test reading both would report drift without saying which half is ungated.
    #[test]
    fn the_frontend_types_match_the_interaction_types_too() {
        let rust = rust_structs(&include_str!("interaction.rs").replace("\r\n", "\n"));
        let ts = ts_interfaces(
            &include_str!("../../../apps/desktop/src/types.ts").replace("\r\n", "\n"),
        );

        let mut shared = 0;
        let mut drift = Vec::new();
        for (name, rust_fields) in &rust {
            let Some((_, ts_fields)) = ts.iter().find(|(n, _)| n == name) else {
                continue;
            };
            shared += 1;
            let only_rust: Vec<_> = rust_fields.difference(ts_fields).cloned().collect();
            let only_ts: Vec<_> = ts_fields.difference(rust_fields).cloned().collect();
            if !only_rust.is_empty() || !only_ts.is_empty() {
                drift.push(format!(
                    "{name}: only in Rust {only_rust:?}, only in TypeScript {only_ts:?}"
                ));
            }
        }

        // A scanner that reads nothing passes every assertion below it.
        assert!(
            shared >= 6,
            "only {shared} interaction types matched by name; the scanner has stopped reading one \
             of the two files (Rust structs seen: {}, TS interfaces seen: {})",
            rust.len(),
            ts.len()
        );
        assert!(
            drift.is_empty(),
            "the two halves of the Interaction wire disagree:\n  {}",
            drift.join("\n  ")
        );
    }

    /// **A field Rust may omit must be a field TypeScript knows can be missing.**
    ///
    /// `the_frontend_types_describe_the_same_fields_the_backend_sends` compares field *names*.
    /// That catches a field added on one side and forgotten on the other, and it cannot see
    /// the other half of the contract: whether the two halves agree that a value may be
    /// absent. `Option<String>` on the wire against a required `string` in TypeScript passes
    /// the name check and then renders `undefined` into the UI, or reads it as a value.
    ///
    /// Measured 28/08/2026, after an independent review pointed at the gap: **31** `Option<>`
    /// fields in the 25 shared shapes have a TypeScript counterpart, and **all 31 already
    /// declare it nullable**. So this gate lands green and its job is to keep it that way —
    /// which is the only moment a gate like this is cheap to add.
    ///
    /// The **other** direction is deliberately not checked. Fourteen fields are optional in
    /// TypeScript and required in Rust — every one of them backed by `#[serde(default)]`,
    /// which is exactly what makes omitting them on the wire correct. Asserting symmetry there
    /// would flag fourteen correct declarations, and a gate that has to be argued with is a
    /// gate people switch off.
    #[test]
    fn a_field_rust_can_omit_is_nullable_in_typescript() {
        /// `Option<T>` fields, by struct, from a `pub struct` block.
        fn rust_optional_fields(source: &str) -> Vec<(String, String)> {
            let mut out = Vec::new();
            let lines: Vec<&str> = source.lines().collect();
            let mut current: Option<String> = None;
            for line in &lines {
                let trimmed = line.trim();
                if let Some(rest) = trimmed.strip_prefix("pub struct ") {
                    if let Some(name) = rest.split_whitespace().next() {
                        current = Some(name.trim_end_matches('{').trim().to_string());
                    }
                    continue;
                }
                if trimmed == "}" {
                    current = None;
                    continue;
                }
                let Some(struct_name) = current.as_ref() else {
                    continue;
                };
                let Some(rest) = trimmed.strip_prefix("pub ") else {
                    continue;
                };
                let Some((field, ty)) = rest.split_once(':') else {
                    continue;
                };
                // The fully qualified spellings count too: `std::option::Option<T>` is the
                // same wire shape, and a review pointed out the bare-prefix check read it
                // as required — one rename away from a silent `undefined` on the frontend.
                let ty = ty.trim();
                let ty = ty
                    .strip_prefix("std::option::")
                    .or_else(|| ty.strip_prefix("core::option::"))
                    .unwrap_or(ty);
                if ty.starts_with("Option<") {
                    out.push((struct_name.clone(), field.trim().to_string()));
                }
            }
            out
        }

        /// Field name -> whether TypeScript says it can be absent, by interface.
        fn ts_nullable_fields(source: &str) -> Vec<(String, String, bool)> {
            let mut out = Vec::new();
            let mut current: Option<String> = None;
            for line in source.lines() {
                let trimmed = line.trim();
                if let Some(rest) = trimmed.strip_prefix("export interface ") {
                    if let Some(name) = rest.split_whitespace().next() {
                        current = Some(name.trim_end_matches('{').trim().to_string());
                    }
                    continue;
                }
                if trimmed == "}" {
                    current = None;
                    continue;
                }
                let Some(interface) = current.as_ref() else {
                    continue;
                };
                let code = trimmed.split("//").next().unwrap_or(trimmed).trim();
                if code.is_empty() || code.starts_with('*') || code.starts_with('/') {
                    continue;
                }
                let Some((name, ty)) = code.split_once(':') else {
                    continue;
                };
                let optional = name.trim().ends_with('?');
                let field = name.trim().trim_end_matches('?').trim();
                if field.is_empty() || field.contains(' ') {
                    continue;
                }
                let nullable = optional
                    || ty.contains("| null")
                    || ty.contains("|null")
                    || ty.contains("| undefined");
                out.push((interface.clone(), field.to_string(), nullable));
            }
            out
        }

        let rust = rust_optional_fields(&include_str!("types.rs").replace("\r\n", "\n"));
        let ts = ts_nullable_fields(
            &include_str!("../../../apps/desktop/src/types.ts").replace("\r\n", "\n"),
        );

        let mut compared = 0usize;
        let mut drift = Vec::new();
        for (struct_name, field) in &rust {
            let wanted = camel(field);
            let Some((_, _, nullable)) = ts
                .iter()
                .find(|(interface, name, _)| interface == struct_name && *name == wanted)
            else {
                // No TypeScript counterpart at all is the *other* gate's business.
                continue;
            };
            compared += 1;
            if !nullable {
                drift.push(format!(
                    "{struct_name}.{field} is Option<..> in Rust but required in TypeScript"
                ));
            }
        }

        // A scanner that matched nothing would pass the assertion below it.
        assert!(
            compared >= 25,
            "only {compared} Option<> fields matched a TypeScript field; the parse is broken \
             (31 matched when this was written)"
        );
        assert!(
            drift.is_empty(),
            "TypeScript will read these as always present, and Rust may send null:\n  {}",
            drift.join("\n  ")
        );
    }
    #[test]
    fn the_frontend_types_describe_the_same_fields_the_backend_sends() {
        // Same reason as `fields_absorbed_live`: a line-anchored scan against bytes that may
        // carry CR sees a different shape than the pattern compiled from this source.
        let rust = rust_structs(&include_str!("types.rs").replace("\r\n", "\n"));
        let ts = ts_interfaces(include_str!("../../../apps/desktop/src/types.ts"));

        let mut shared = 0;
        let mut drift = Vec::new();
        for (name, rust_fields) in &rust {
            let Some((_, ts_fields)) = ts.iter().find(|(n, _)| n == name) else {
                continue;
            };
            shared += 1;
            let only_rust: Vec<_> = rust_fields.difference(ts_fields).cloned().collect();
            let only_ts: Vec<_> = ts_fields.difference(rust_fields).cloned().collect();
            if !only_rust.is_empty() || !only_ts.is_empty() {
                drift.push(format!(
                    "{name}: only in Rust {only_rust:?}, only in TypeScript {only_ts:?}"
                ));
            }
        }

        // A scanner that reads nothing passes every assertion below it.
        assert!(
            shared >= 24,
            "only {shared} types matched by name; the scanner has stopped reading one of the \
             two files (Rust structs seen: {}, TS interfaces seen: {})",
            rust.len(),
            ts.len()
        );
        assert!(
            drift.is_empty(),
            "the two halves of the wire disagree:\n  {}",
            drift.join("\n  ")
        );
    }
}

/// The two pure decisions behind a progress bar.
///
/// They encode policy — which bound wins, what a terminal row reads, what "unknown" means —
/// and policy without tests is how a bar ends up reporting 800% on a run nobody changed.
/// Sibling to `nurture::recovery::session_verdict`, which this repo already treats as a
/// pure, unit-tested function.
#[cfg(test)]
mod progress_tests {
    use super::*;

    fn at(seconds: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(1_800_000_000 + seconds, 0).expect("fixed timestamp")
    }

    /// A row mid-run: 120 posts wanted, a three-hour horizon, nothing done yet.
    fn running(videos_done: u32) -> NurtureSessionStatus {
        NurtureSessionStatus {
            running: true,
            phase: NurturePhase::Watching,
            videos_done,
            video_target: 120,
            started_at: Some(at(0)),
            deadline_at: Some(at(3 * 3600)),
            ..NurtureSessionStatus::new("fixture")
        }
    }

    #[test]
    fn a_queued_row_with_nothing_to_divide_by_is_unknown_not_zero() {
        // The distinction the `Option` exists for: an empty track reads as a stall, and a
        // phone that has not started is not stalled.
        let queued = NurtureSessionStatus::new("fixture");
        assert_eq!(queued.progress_fraction(at(0)), None);
        assert_eq!(queued.governing_bound(at(0)), None);
    }

    #[test]
    fn the_video_count_governs_when_it_is_ahead_of_the_clock() {
        // 60 of 120 posts = 50%, against 6 minutes of a 180-minute horizon = 3.3%.
        let status = running(60);
        assert_eq!(status.progress_fraction(at(6 * 60)), Some(0.5));
        assert_eq!(
            status.governing_bound(at(6 * 60)),
            Some(NurtureBound::Videos)
        );
    }

    /// The reading a count-only bar gets wrong, and the reason for taking the maximum: this
    /// session is twelve minutes from ending and a video bar would call it 40%.
    #[test]
    fn the_clock_governs_when_it_is_ahead_and_the_bar_follows_it() {
        let status = running(48); // 40% of the posts…
        let nearly_over = at(168 * 60); // …but 93% of the horizon.
        let fraction = status.progress_fraction(nearly_over).expect("known");
        assert!(
            (fraction - 168.0 / 180.0).abs() < 1e-9,
            "the closer bound wins: {fraction}"
        );
        assert_eq!(
            status.governing_bound(nearly_over),
            Some(NurtureBound::Clock)
        );
    }

    #[test]
    fn a_terminal_row_reads_full_whatever_its_counters_say() {
        // A run that stopped at 40 of 120 is finished, not 33% done. Leaving its bar short
        // would read as still working — which is the whole reason `phase` exists.
        let mut status = running(40);
        status.finish(crate::Outcome::Partial);
        assert_eq!(status.progress_fraction(at(60)), Some(1.0));
        assert_eq!(
            status.governing_bound(at(60)),
            None,
            "a finished session has no bound left to name"
        );
    }

    #[test]
    fn a_failed_row_also_reads_full_because_it_is_over() {
        // Deliberate: the bar means "this slot is settled", and the *colour* carries the
        // verdict. A failed phone frozen at 0% would look like one that never started.
        let mut status = running(0);
        status.finish(crate::Outcome::Failed);
        assert_eq!(status.progress_fraction(at(0)), Some(1.0));
        assert_eq!(status.outcome, Some(crate::Outcome::Failed));
    }

    #[test]
    fn the_fraction_never_exceeds_one_even_past_the_deadline() {
        let status = running(0);
        assert_eq!(status.progress_fraction(at(10 * 3600)), Some(1.0));
    }

    #[test]
    fn a_clock_before_the_start_reads_zero_rather_than_negative() {
        // Clocks are not monotone across a machine's time changes; a negative fraction would
        // render as a bar growing leftwards.
        let status = running(0);
        assert_eq!(status.progress_fraction(at(-600)), Some(0.0));
    }

    #[test]
    fn a_deadline_at_or_before_the_start_is_ignored_rather_than_dividing_by_zero() {
        let status = NurtureSessionStatus {
            running: true,
            phase: NurturePhase::Watching,
            videos_done: 30,
            video_target: 120,
            started_at: Some(at(0)),
            deadline_at: Some(at(0)),
            ..NurtureSessionStatus::new("fixture")
        };
        assert_eq!(status.progress_fraction(at(60)), Some(0.25));
        assert_eq!(status.governing_bound(at(60)), Some(NurtureBound::Videos));
    }

    #[test]
    fn a_run_with_no_deadline_still_tracks_its_video_count() {
        let status = NurtureSessionStatus {
            running: true,
            phase: NurturePhase::Watching,
            videos_done: 15,
            video_target: 60,
            started_at: Some(at(0)),
            deadline_at: None,
            ..NurtureSessionStatus::new("fixture")
        };
        assert_eq!(status.progress_fraction(at(60)), Some(0.25));
        assert_eq!(status.governing_bound(at(60)), Some(NurtureBound::Videos));
    }

    /// The lead threshold: a clock barely ahead of a zero count must not steal the label.
    /// Measured on the live fleet — the first thing shown was "còn ~154 phút" on a run whose
    /// operator had just typed 5 into the video limit.
    #[test]
    fn the_video_count_keeps_the_label_until_the_clock_is_meaningfully_ahead() {
        let just_started = running(0);
        assert_eq!(
            just_started.governing_bound(at(2 * 60)),
            Some(NurtureBound::Videos),
            "two minutes into three hours is not the clock governing"
        );
        assert_eq!(
            just_started.governing_bound(at(20 * 60)),
            Some(NurtureBound::Clock),
            "twenty minutes in with nothing watched, it is"
        );
    }

    #[test]
    fn the_fraction_still_follows_the_clock_while_the_label_says_videos() {
        // The fill takes the plain maximum; only the sentence waits for the lead.
        let just_started = running(0);
        let fraction = just_started.progress_fraction(at(2 * 60)).expect("known");
        assert!((fraction - 2.0 / 180.0).abs() < 1e-9, "{fraction}");
        assert_eq!(
            just_started.governing_bound(at(2 * 60)),
            Some(NurtureBound::Videos)
        );
    }

    #[test]
    fn a_run_with_no_video_target_still_tracks_the_clock() {
        let status = NurtureSessionStatus {
            running: true,
            phase: NurturePhase::Watching,
            video_target: 0,
            started_at: Some(at(0)),
            deadline_at: Some(at(100)),
            ..NurtureSessionStatus::new("fixture")
        };
        assert_eq!(status.progress_fraction(at(25)), Some(0.25));
        assert_eq!(status.governing_bound(at(25)), Some(NurtureBound::Clock));
    }

    /// Monotone, which is what stops a bar jittering backwards while an operator watches it.
    #[test]
    fn the_fraction_never_goes_backwards_as_a_run_proceeds() {
        let mut last = 0.0;
        for minute in 0..180 {
            let status = running(minute / 2);
            let value = status
                .progress_fraction(at(minute as i64 * 60))
                .expect("known");
            assert!(
                value >= last,
                "went backwards at minute {minute}: {value} < {last}"
            );
            last = value;
        }
    }

    #[test]
    fn finish_marks_the_row_terminal_and_carries_the_verdict() {
        let mut status = running(5);
        assert!(status.running);
        assert!(!status.phase.is_terminal());
        status.finish(crate::Outcome::Done);
        assert!(!status.running);
        assert!(status.phase.is_terminal());
        assert_eq!(status.outcome, Some(crate::Outcome::Done));
    }
}
