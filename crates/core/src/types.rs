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
    pub last_error: Option<String>,
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
    LaunchApp { bundle_id: String },
    #[serde(rename_all = "camelCase")]
    TerminateApp { bundle_id: String },
    #[serde(rename_all = "camelCase")]
    Wait { milliseconds: u64 },
    #[serde(rename_all = "camelCase")]
    Tap {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        selector: Option<ElementSelector>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        point: Option<TapPoint>,
    },
    #[serde(rename_all = "camelCase")]
    Swipe { gesture: SwipeGesture },
    #[serde(rename_all = "camelCase")]
    TypeText { value: String },
    #[serde(rename_all = "camelCase")]
    Screenshot { name: String },
    Home,
    #[serde(rename_all = "camelCase")]
    AssertVisible { selector: ElementSelector },
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
#[serde(rename_all = "camelCase")]
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
            base_url: "https://api.vilao.ai/v1".into(),
            model: "cd/gpt-5.5".into(),
            api_key: String::new(),
            input_price_per_1m: 1.25,
            output_price_per_1m: 10.0,
            bundle_id: "com.ss.iphone.ugc.Ame".into(),
            num_videos: 50,
            num_rounds: 1,
            like_prob: 40,
            comment_prob: 25,
            follow_prob: 5,
            frenzy_prob: 8,
            watch_min: 5.0,
            watch_max: 20.0,
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
            ai_directions: "Gen z|Tự nhiên|Ngắn gọn".into(),
            max_comment_words: 12,
            schedule_enabled: false,
            schedule_every_minutes: 60,
            schedule_duration_minutes: 20,
            schedule_udids: Vec::new(),
            steady_mood: String::new(),
        }
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
pub struct NurtureSessionStatus {
    pub udid: String,
    pub running: bool,
    pub videos_done: u32,
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
