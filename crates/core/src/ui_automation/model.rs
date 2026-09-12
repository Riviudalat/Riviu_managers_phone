use serde::{Deserialize, Serialize};
use std::sync::Arc;

pub const GUI_PROTOCOL_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GuiScope {
    pub run_id: String,
    pub assignment_id: Option<String>,
    pub device_id: String,
    pub deadline_ms: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppContext {
    pub package: String,
    pub version: String,
    pub system_locale: String,
    pub observed_language: Option<String>,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}
impl Rect {
    pub fn valid(&self, width: u32, height: u32) -> bool {
        [self.x, self.y, self.width, self.height]
            .iter()
            .all(|v| v.is_finite())
            && self.x >= 0.0
            && self.y >= 0.0
            && self.width > 0.0
            && self.height > 0.0
            && self.x + self.width <= f64::from(width)
            && self.y + self.height <= f64::from(height)
    }
    pub fn contains(&self, other: &Self) -> bool {
        other.x >= self.x
            && other.y >= self.y
            && other.x + other.width <= self.x + self.width
            && other.y + other.height <= self.y + self.height
    }
}
impl From<&crate::ElementBox> for Rect {
    fn from(b: &crate::ElementBox) -> Self {
        Self {
            x: b.x,
            y: b.y,
            width: b.width,
            height: b.height,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GuiNode {
    pub id: usize,
    pub parent: Option<usize>,
    pub text: String,
    pub description: String,
    pub resource_id: String,
    pub class_name: String,
    pub bounds: Rect,
    pub enabled: Option<bool>,
    pub clickable: Option<bool>,
    #[serde(default)]
    pub visible: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GuiRequest {
    pub scope: Option<GuiScope>,
    pub protocol_version: u32,
    pub request_id: String,
    pub observation_id: String,
    pub session_epoch: String,
    pub generation: u64,
    pub app: AppContext,
    pub target: String,
    pub expected_screen: String,
    pub remaining_ms: u64,
    pub nodes: Vec<GuiNode>,
    pub screenshot: String,
    pub screenshot_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ResolutionStatus {
    Resolved,
    Unresolved,
    Ambiguous,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TargetCandidate {
    pub node_id: Option<usize>,
    pub bounds: Rect,
    pub method: String,
    pub evidence: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GuiResponse {
    pub protocol_version: u32,
    pub request_id: String,
    pub observation_id: String,
    pub session_epoch: String,
    pub generation: u64,
    pub status: ResolutionStatus,
    pub candidates: Vec<TargetCandidate>,
    pub reason: String,
    pub model: Option<String>,
    pub elapsed_ms: u64,
    pub prompt_tokens: Option<u64>,
    pub completion_tokens: Option<u64>,
    #[serde(default)]
    pub cost_usd: Option<f64>,
}
impl GuiResponse {
    pub fn validate_binding(&self, request: &GuiRequest) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.protocol_version == GUI_PROTOCOL_VERSION
                && self.request_id == request.request_id
                && self.observation_id == request.observation_id
                && self.session_epoch == request.session_epoch
                && self.generation == request.generation,
            "gui_response_stale: phản hồi không thuộc quan sát hiện tại"
        );
        anyhow::ensure!(
            self.candidates.len() <= 8,
            "gui_response_invalid: quá nhiều ứng viên"
        );
        anyhow::ensure!(
            self.status != ResolutionStatus::Resolved || self.candidates.len() == 1,
            "gui_response_ambiguous: mục tiêu chưa duy nhất"
        );
        for candidate in &self.candidates {
            anyhow::ensure!(
                candidate
                    .bounds
                    .valid(request.app.width, request.app.height),
                "gui_bounds_invalid"
            );
            if let Some(id) = candidate.node_id {
                let node = request
                    .nodes
                    .iter()
                    .find(|n| n.id == id)
                    .ok_or_else(|| anyhow::anyhow!("gui_node_missing"))?;
                anyhow::ensure!(
                    node.bounds == candidate.bounds
                        && node.enabled == Some(true)
                        && node.clickable == Some(true),
                    "gui_node_changed"
                );
            }
        }
        Ok(())
    }
}

#[async_trait::async_trait]
pub trait GuiReasoner: Send + Sync {
    async fn resolve(&self, request: GuiRequest) -> anyhow::Result<GuiResponse>;
    fn compatibility_pack(&self, _package: &str) -> Option<super::profile::CompatibilityPack> {
        None
    }
    fn compatibility_pack_for(
        &self,
        package: &str,
        _scope: Option<&GuiScope>,
    ) -> Option<super::profile::CompatibilityPack> {
        self.compatibility_pack(package)
    }
    fn record(&self, _request: &GuiRequest, _response: Option<&GuiResponse>, _reason: &str) {}
}
pub type SharedReasoner = Arc<dyn GuiReasoner>;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum CheckStatus {
    Pass,
    Blocked,
    Unknown,
    NotApplicable,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AutomationCheck {
    pub id: String,
    pub label: String,
    pub status: CheckStatus,
    pub reason: Option<String>,
}
