//! Durable, observation-only reconciliation for Android comments.
use serde::{Deserialize, Serialize};

pub mod search;
pub mod worker;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum VerificationState {
    Pending,
    Verified,
    NeedsReview,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommentVerification {
    pub state: VerificationState,
    pub attempts: u32,
    pub next_check_at_ms: Option<i64>,
    pub deadline_ms: Option<i64>,
    pub reason: Option<String>,
    pub evidence: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VerificationContext {
    pub target: crate::ResolvedTikTokTarget,
    pub device_id: String,
    pub account: String,
    pub text: String,
    pub mentions: Vec<String>,
    pub parent: Option<crate::CommentLocatorIdentity>,
    pub root: Option<crate::CommentLocatorIdentity>,
}

#[derive(Debug, Clone)]
pub struct VerificationJob {
    pub assignment_id: String,
    pub campaign_id: String,
    pub context: VerificationContext,
    pub owner: String,
    pub revision: i64,
    pub deadline_ms: i64,
    pub attempts: u32,
}
