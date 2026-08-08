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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceInfo {
    pub udid: String,
    pub name: String,
    pub model: String,
    pub ios_version: String,
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
            "iosVersion": "fixture",
            "connection": "mock",
            "status": "ready",
            "battery": null,
            "wdaReady": true,
            "wdaExpiresAt": null,
            "streamUrl": null,
            "lastError": null
        }))
        .expect("decode legacy device payload");

        assert_eq!(decoded.tile_stream_state, TileStreamState::Parked);
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TileSize {
    Thumbnail,
    Medium,
    Large,
    ExtraLarge,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StreamQuality {
    Low,
    Medium,
    High,
    Extra,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamSettings {
    pub fps: u32,
    pub tile_size: TileSize,
    pub grid_quality: StreamQuality,
    pub focus_quality: StreamQuality,
}

impl Default for StreamSettings {
    fn default() -> Self {
        Self {
            fps: STREAM_FPS,
            tile_size: TileSize::Medium,
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
pub struct LocalUser {
    pub id: String,
    pub email: String,
    pub role: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthSession {
    pub show_auth_ui: bool,
    pub bypassed: bool,
    pub user: Option<LocalUser>,
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
}

impl Default for NurtureSettings {
    fn default() -> Self {
        Self {
            // Vilao AI is an OpenAI-compatible gateway; any other compatible
            // endpoint works by changing these two fields. The key is never
            // stored here — it comes from the settings row or RIVIU_AI_API_KEY.
            base_url: "https://api.deepseek.com".into(),
            model: "deepseek-v4-flash".into(),
            api_key: String::new(),
            input_price_per_1m: 1.25,
            output_price_per_1m: 10.0,
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
        }
    }
}

impl NurtureSettings {
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
}

#[cfg(test)]
mod nurture_settings_tests {
    use super::NurtureSettings;

    #[test]
    fn defaults_allow_a_first_run_without_ai_credentials() {
        let settings = NurtureSettings::default();
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
