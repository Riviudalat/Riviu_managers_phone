//! Effect-aware orchestration for one publish assignment.
//!
//! The device adapter owns locators and gestures. This module owns the order and, more
//! importantly, the point after which a failed call may have created a public post. It never
//! retries a post. A confirmed post may resume only at link capture or the idempotent Sheet
//! write.

use std::collections::BTreeMap;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::publish::{
    select_sound_candidate, PublishBundle, PublishMediaKind, PublishSoundPolicy, SoundCandidate,
    SoundSelectionEvidence,
};

/// The externally visible result of one assignment execution.
#[derive(Debug, Clone, Copy, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum PublishExecutionStatus {
    Complete,
    Partial,
    Uncertain,
}

/// The only portion a caller is allowed to resume automatically.
#[derive(Debug, Clone, Copy, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum PublishRetryScope {
    FullPipeline,
    LinkAndSheet,
    SheetOnly,
    None,
}

#[derive(Debug, Clone, Copy, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum PublishExecutionPhase {
    Preflight,
    Transfer,
    SoundSelection,
    Post,
    Confirmation,
    LinkCapture,
    Sheet,
    Cleanup,
}

#[derive(Debug, Clone, Copy, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum PublishPhaseStatus {
    Complete,
    Failed,
    Skipped,
}

#[derive(Debug, Clone, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PublishPhaseResult {
    pub phase: PublishExecutionPhase,
    pub status: PublishPhaseStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// Durable checkpoint supplied when completing work for a post already proven live.
#[derive(Debug, Clone, Deserialize, PartialEq, Serialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum PublishResumePoint {
    Full,
    ConfirmedPost {
        post_evidence: Value,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        canonical_link: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        sound_selection: Option<SoundSelectionEvidence>,
    },
}

#[derive(Debug, Clone, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PublishExecutionInput {
    pub assignment_id: String,
    pub bundle: PublishBundle,
    pub sound_policy: PublishSoundPolicy,
    /// This is the one operator confirmation for the whole pipeline.
    pub confirmed: bool,
    pub resume: PublishResumePoint,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PublishExecutionResult {
    pub assignment_id: String,
    pub media_kind: PublishMediaKind,
    pub status: PublishExecutionStatus,
    pub retry_scope: PublishRetryScope,
    pub phases: Vec<PublishPhaseResult>,
    pub post_confirmed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub post_evidence: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub canonical_link: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sound_selection: Option<SoundSelectionEvidence>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cleanup_warning: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PublishExecutionIssue {
    pub code: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assignment_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub udid: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bundle_id: Option<String>,
    pub message: String,
}

/// The immutable operator input covered by one publish preflight digest.
///
/// A sorted map makes caption override ordering deterministic before a caller hashes the
/// serialized request. Device observations belong in [`PublishPreflightReport`], not here.
#[derive(Debug, Clone, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PublishPreflightRequest {
    pub source_root: String,
    pub bundle_ids: Vec<String>,
    pub udids: Vec<String>,
    /// Semantic operator selection. Older clients supplied only `udids`; those requests are
    /// interpreted as an explicit target so persisted payloads remain readable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_ref: Option<crate::TargetRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_at: Option<String>,
    #[serde(default)]
    pub caption_overrides: BTreeMap<String, String>,
    #[serde(default)]
    pub sound_policy: PublishSoundPolicy,
}

/// Result of a bounded, read-only preflight check.
#[derive(Debug, Clone, Copy, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum PublishPreflightCheck {
    Pass,
    Fail,
}

/// Preflight evidence for the exact bundle-to-device assignment at `ordinal`.
#[derive(Debug, Clone, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PublishPreflightAssignmentReport {
    pub ordinal: u32,
    pub bundle_id: String,
    pub udid: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub package_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub locale: Option<String>,
    pub media: PublishPreflightCheck,
    pub composer: PublishPreflightCheck,
    pub sound_picker: PublishPreflightCheck,
    pub storage: PublishPreflightCheck,
    pub required_bytes: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub available_bytes: Option<u64>,
    #[serde(default)]
    pub issues: Vec<PublishExecutionIssue>,
}

/// Read-only result whose digest must still match when a campaign is created.
#[derive(Debug, Clone, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PublishPreflightReport {
    pub input_digest: String,
    pub target_snapshot: crate::ResolvedTargetSnapshot,
    pub can_execute: bool,
    pub assignments: Vec<PublishPreflightAssignmentReport>,
    #[serde(default)]
    pub issues: Vec<PublishExecutionIssue>,
    pub sheet_configured: bool,
}

/// Latest durable execution projection for one publish campaign.
///
/// This is a replacement snapshot, not an append-only event. The campaign's existing event
/// stream remains the history; this record is the restart-safe answer to "what may resume?".
#[derive(Debug, Clone, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PublishExecutionSnapshot {
    pub campaign_id: String,
    pub input_digest: String,
    pub status: PublishExecutionStatus,
    pub retry_scope: PublishRetryScope,
    pub report_json: Value,
    pub updated_at: String,
}

/// Initial restart projection committed atomically with a new publish campaign.
#[derive(Debug, Clone, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PublishExecutionSnapshotDraft {
    pub input_digest: String,
    pub status: PublishExecutionStatus,
    pub retry_scope: PublishRetryScope,
    pub report_json: Value,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PublishCampaignExecutionResult {
    pub campaign_id: String,
    pub status: PublishExecutionStatus,
    pub retry_scope: PublishRetryScope,
    pub issues: Vec<PublishExecutionIssue>,
    pub detail: crate::publish::PublishCampaignDetail,
}

/// Device and persistence operations required by the orchestrator.
///
/// `dispatch_post` must call `before_post` immediately before the first Post gesture. It must
/// not dispatch when the callback fails and must never call it a second time. Sheet writes are
/// keyed by `assignment_id` and must be idempotent.
#[async_trait]
pub trait PublishRuntimePort: Send {
    async fn preflight(&mut self, input: &PublishExecutionInput) -> Result<(), String>;
    async fn transfer(&mut self, bundle: &PublishBundle) -> Result<(), String>;
    async fn observe_sound_candidates(
        &mut self,
        maximum_visible: usize,
    ) -> Result<Vec<SoundCandidate>, String>;
    async fn choose_sound(&mut self, selection: &SoundSelectionEvidence) -> Result<(), String>;
    async fn confirm_sound(&mut self, selection: &SoundSelectionEvidence) -> Result<bool, String>;
    async fn dispatch_post(
        &mut self,
        before_post: &mut (dyn FnMut() -> Result<(), String> + Send),
        selection: &SoundSelectionEvidence,
    ) -> Result<(), String>;
    async fn confirm_post(&mut self) -> Result<Value, String>;
    async fn capture_canonical_link(&mut self, bundle: &PublishBundle) -> Result<String, String>;
    async fn write_sheet(
        &mut self,
        assignment_id: &str,
        canonical_link: &str,
        bundle: &PublishBundle,
    ) -> Result<(), String>;
    async fn cleanup(&mut self) -> Result<(), String>;
}

/// Run a fresh assignment or complete only the downstream obligations of a confirmed post.
pub async fn run_publish_pipeline<P, F>(
    input: PublishExecutionInput,
    port: &mut P,
    mut record_effect_intent: F,
) -> PublishExecutionResult
where
    P: PublishRuntimePort,
    F: FnMut(&SoundSelectionEvidence) -> Result<(), String> + Send,
{
    let mut result = PublishExecutionResult {
        assignment_id: input.assignment_id.clone(),
        media_kind: input.bundle.media_kind.clone(),
        status: PublishExecutionStatus::Partial,
        retry_scope: PublishRetryScope::FullPipeline,
        phases: Vec::new(),
        post_confirmed: false,
        post_evidence: None,
        canonical_link: None,
        sound_selection: None,
        cleanup_warning: None,
    };

    if let PublishResumePoint::ConfirmedPost {
        post_evidence,
        canonical_link,
        sound_selection,
    } = &input.resume
    {
        result.post_confirmed = true;
        result.post_evidence = Some(post_evidence.clone());
        result.canonical_link = canonical_link.clone();
        result.sound_selection = sound_selection.clone();
    }

    if !input.confirmed {
        let retry_scope = match &input.resume {
            PublishResumePoint::Full => PublishRetryScope::FullPipeline,
            PublishResumePoint::ConfirmedPost {
                canonical_link: Some(_),
                ..
            } => PublishRetryScope::SheetOnly,
            PublishResumePoint::ConfirmedPost { .. } => PublishRetryScope::LinkAndSheet,
        };
        fail_phase(
            &mut result,
            PublishExecutionPhase::Preflight,
            "operator confirmation is required".into(),
            retry_scope,
        );
        return result;
    }
    if let Err(error) = port.preflight(&input).await {
        fail_phase(
            &mut result,
            PublishExecutionPhase::Preflight,
            error,
            match &input.resume {
                PublishResumePoint::Full => PublishRetryScope::FullPipeline,
                PublishResumePoint::ConfirmedPost {
                    canonical_link: Some(_),
                    ..
                } => PublishRetryScope::SheetOnly,
                PublishResumePoint::ConfirmedPost { .. } => PublishRetryScope::LinkAndSheet,
            },
        );
        return result;
    }
    complete_phase(&mut result, PublishExecutionPhase::Preflight);

    let (post_evidence, sound_selection, existing_link) = match input.resume.clone() {
        PublishResumePoint::Full => {
            if let Err(error) = port.transfer(&input.bundle).await {
                fail_phase(
                    &mut result,
                    PublishExecutionPhase::Transfer,
                    error,
                    PublishRetryScope::FullPipeline,
                );
                return finish_with_cleanup(port, result).await;
            }
            complete_phase(&mut result, PublishExecutionPhase::Transfer);

            let requested_pool = match input.sound_policy.pool_size() {
                Ok(pool) => pool,
                Err(error) => {
                    fail_phase(
                        &mut result,
                        PublishExecutionPhase::SoundSelection,
                        error.to_string(),
                        PublishRetryScope::FullPipeline,
                    );
                    return finish_with_cleanup(port, result).await;
                }
            };
            // Runtime selection is deliberately narrower than the reusable domain contract:
            // only the first five rows currently visible in the in-app picker are eligible.
            let visible_pool = requested_pool.min(5);
            let candidates = match port.observe_sound_candidates(visible_pool).await {
                Ok(candidates) => candidates
                    .into_iter()
                    .take(visible_pool)
                    .collect::<Vec<_>>(),
                Err(error) => {
                    fail_phase(
                        &mut result,
                        PublishExecutionPhase::SoundSelection,
                        error,
                        PublishRetryScope::FullPipeline,
                    );
                    return finish_with_cleanup(port, result).await;
                }
            };
            let bounded_policy = match input.sound_policy {
                PublishSoundPolicy::Default => PublishSoundPolicy::Default,
                PublishSoundPolicy::TrendingAny { seed, .. } => PublishSoundPolicy::TrendingAny {
                    pool_size: visible_pool,
                    seed,
                },
            };
            let mut selection = match select_sound_candidate(&bounded_policy, &candidates) {
                Ok(selection) => selection,
                Err(error) => {
                    fail_phase(
                        &mut result,
                        PublishExecutionPhase::SoundSelection,
                        error.to_string(),
                        PublishRetryScope::FullPipeline,
                    );
                    return finish_with_cleanup(port, result).await;
                }
            };
            if let Err(error) = port.choose_sound(&selection).await {
                fail_phase(
                    &mut result,
                    PublishExecutionPhase::SoundSelection,
                    error,
                    PublishRetryScope::FullPipeline,
                );
                return finish_with_cleanup(port, result).await;
            }

            // No device operation may sit between this readback and `dispatch_post`. The
            // adapter invokes the callback at the final line before its actual Post gesture.
            match port.confirm_sound(&selection).await {
                Ok(true) => selection.confirmed = true,
                Ok(false) => {
                    fail_phase(
                        &mut result,
                        PublishExecutionPhase::SoundSelection,
                        "selected sound did not match immediately before Post".into(),
                        PublishRetryScope::FullPipeline,
                    );
                    return finish_with_cleanup(port, result).await;
                }
                Err(error) => {
                    fail_phase(
                        &mut result,
                        PublishExecutionPhase::SoundSelection,
                        error,
                        PublishRetryScope::FullPipeline,
                    );
                    return finish_with_cleanup(port, result).await;
                }
            }
            complete_phase(&mut result, PublishExecutionPhase::SoundSelection);
            result.sound_selection = Some(selection.clone());

            let mut callback_calls = 0_u8;
            let mut effect_boundary_crossed = false;
            let effect_evidence = selection.clone();
            let dispatch = {
                let mut one_shot = || {
                    callback_calls = callback_calls.saturating_add(1);
                    if callback_calls != 1 {
                        return Err("Post effect-intent callback was invoked more than once".into());
                    }
                    record_effect_intent(&effect_evidence)?;
                    effect_boundary_crossed = true;
                    Ok(())
                };
                port.dispatch_post(&mut one_shot, &selection).await
            };
            if let Err(error) = dispatch {
                fail_phase(
                    &mut result,
                    PublishExecutionPhase::Post,
                    error,
                    if effect_boundary_crossed {
                        PublishRetryScope::None
                    } else {
                        PublishRetryScope::FullPipeline
                    },
                );
                if effect_boundary_crossed {
                    result.status = PublishExecutionStatus::Uncertain;
                }
                return finish_with_cleanup(port, result).await;
            }
            if !effect_boundary_crossed {
                fail_phase(
                    &mut result,
                    PublishExecutionPhase::Post,
                    "adapter returned from Post without recording effect intent".into(),
                    PublishRetryScope::None,
                );
                result.status = PublishExecutionStatus::Uncertain;
                return finish_with_cleanup(port, result).await;
            }
            complete_phase(&mut result, PublishExecutionPhase::Post);

            let post_evidence = match port.confirm_post().await {
                Ok(evidence) => evidence,
                Err(error) => {
                    fail_phase(
                        &mut result,
                        PublishExecutionPhase::Confirmation,
                        error,
                        PublishRetryScope::None,
                    );
                    result.status = PublishExecutionStatus::Uncertain;
                    return finish_with_cleanup(port, result).await;
                }
            };
            complete_phase(&mut result, PublishExecutionPhase::Confirmation);
            result.post_confirmed = true;
            result.post_evidence = Some(post_evidence.clone());
            (post_evidence, Some(selection), None)
        }
        PublishResumePoint::ConfirmedPost {
            post_evidence,
            canonical_link,
            sound_selection,
        } => {
            result.post_confirmed = true;
            result.post_evidence = Some(post_evidence.clone());
            result.sound_selection = sound_selection.clone();
            skip_phase(&mut result, PublishExecutionPhase::Transfer);
            skip_phase(&mut result, PublishExecutionPhase::SoundSelection);
            skip_phase(&mut result, PublishExecutionPhase::Post);
            skip_phase(&mut result, PublishExecutionPhase::Confirmation);
            (post_evidence, sound_selection, canonical_link)
        }
    };

    let canonical_link = match existing_link {
        Some(link) if crate::tiktok_share::looks_like_a_post_link(link.trim()) => {
            skip_phase(&mut result, PublishExecutionPhase::LinkCapture);
            link.trim().to_string()
        }
        Some(_) => {
            fail_phase(
                &mut result,
                PublishExecutionPhase::LinkCapture,
                "stored link is not a canonical TikTok post link".into(),
                PublishRetryScope::LinkAndSheet,
            );
            result.post_confirmed = true;
            result.post_evidence = Some(post_evidence);
            result.sound_selection = sound_selection;
            return finish_without_cleanup(result);
        }
        None => match port.capture_canonical_link(&input.bundle).await {
            Ok(link) if crate::tiktok_share::looks_like_a_post_link(link.trim()) => {
                complete_phase(&mut result, PublishExecutionPhase::LinkCapture);
                link.trim().to_string()
            }
            Ok(_) => {
                fail_phase(
                    &mut result,
                    PublishExecutionPhase::LinkCapture,
                    "captured value is not a canonical TikTok post link".into(),
                    PublishRetryScope::LinkAndSheet,
                );
                result.post_confirmed = true;
                return finish_after_full_if_needed(port, result, &input.resume).await;
            }
            Err(error) => {
                fail_phase(
                    &mut result,
                    PublishExecutionPhase::LinkCapture,
                    error,
                    PublishRetryScope::LinkAndSheet,
                );
                result.post_confirmed = true;
                return finish_after_full_if_needed(port, result, &input.resume).await;
            }
        },
    };
    result.canonical_link = Some(canonical_link.clone());

    if let Err(error) = port
        .write_sheet(&input.assignment_id, &canonical_link, &input.bundle)
        .await
    {
        fail_phase(
            &mut result,
            PublishExecutionPhase::Sheet,
            error,
            PublishRetryScope::SheetOnly,
        );
        return finish_after_full_if_needed(port, result, &input.resume).await;
    }
    complete_phase(&mut result, PublishExecutionPhase::Sheet);
    result.status = PublishExecutionStatus::Complete;
    result.retry_scope = PublishRetryScope::None;
    finish_after_full_if_needed(port, result, &input.resume).await
}

fn complete_phase(result: &mut PublishExecutionResult, phase: PublishExecutionPhase) {
    result.phases.push(PublishPhaseResult {
        phase,
        status: PublishPhaseStatus::Complete,
        detail: None,
    });
}

fn skip_phase(result: &mut PublishExecutionResult, phase: PublishExecutionPhase) {
    result.phases.push(PublishPhaseResult {
        phase,
        status: PublishPhaseStatus::Skipped,
        detail: None,
    });
}

fn fail_phase(
    result: &mut PublishExecutionResult,
    phase: PublishExecutionPhase,
    detail: String,
    retry_scope: PublishRetryScope,
) {
    result.status = PublishExecutionStatus::Partial;
    result.retry_scope = retry_scope;
    result.phases.push(PublishPhaseResult {
        phase,
        status: PublishPhaseStatus::Failed,
        detail: Some(detail),
    });
}

async fn finish_after_full_if_needed<P: PublishRuntimePort>(
    port: &mut P,
    result: PublishExecutionResult,
    resume: &PublishResumePoint,
) -> PublishExecutionResult {
    if matches!(resume, PublishResumePoint::Full) {
        finish_with_cleanup(port, result).await
    } else {
        finish_without_cleanup(result)
    }
}

async fn finish_with_cleanup<P: PublishRuntimePort>(
    port: &mut P,
    mut result: PublishExecutionResult,
) -> PublishExecutionResult {
    match port.cleanup().await {
        Ok(()) => complete_phase(&mut result, PublishExecutionPhase::Cleanup),
        Err(error) => {
            result.cleanup_warning = Some(error.clone());
            result.phases.push(PublishPhaseResult {
                phase: PublishExecutionPhase::Cleanup,
                status: PublishPhaseStatus::Failed,
                detail: Some(error),
            });
        }
    }
    result
}

fn finish_without_cleanup(mut result: PublishExecutionResult) -> PublishExecutionResult {
    skip_phase(&mut result, PublishExecutionPhase::Cleanup);
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::publish::{
        PublishAudioCodec, PublishImage, PublishVideo, PublishVideoCodec, SoundSectionKind,
    };
    use parking_lot::Mutex;
    use std::collections::HashSet;
    use std::sync::Arc;

    struct FakePort {
        calls: Arc<Mutex<Vec<String>>>,
        candidates: Vec<SoundCandidate>,
        post_error: Option<String>,
        invoke_intent: bool,
        sound_confirmed: bool,
        link: Result<String, String>,
        sheet_error: Option<String>,
        cleanup_error: Option<String>,
        written: HashSet<String>,
        preflight_error: Option<String>,
    }

    impl FakePort {
        fn happy(calls: Arc<Mutex<Vec<String>>>) -> Self {
            Self {
                calls,
                candidates: (0..7)
                    .map(|index| SoundCandidate {
                        section: if index % 2 == 0 {
                            "Trending"
                        } else {
                            "Recommended"
                        }
                        .into(),
                        title: format!("track-{index}"),
                        artist: format!("artist-{index}"),
                    })
                    .collect(),
                post_error: None,
                invoke_intent: true,
                sound_confirmed: true,
                link: Ok("https://www.tiktok.com/@fixture/video/7400000000000000001".into()),
                sheet_error: None,
                cleanup_error: None,
                written: HashSet::new(),
                preflight_error: None,
            }
        }

        fn note(&self, value: impl Into<String>) {
            self.calls.lock().push(value.into());
        }
    }

    #[async_trait]
    impl PublishRuntimePort for FakePort {
        async fn preflight(&mut self, _input: &PublishExecutionInput) -> Result<(), String> {
            self.note("preflight");
            match self.preflight_error.clone() {
                Some(error) => Err(error),
                None => Ok(()),
            }
        }

        async fn transfer(&mut self, bundle: &PublishBundle) -> Result<(), String> {
            self.note(format!("transfer:{:?}", bundle.media_kind));
            Ok(())
        }

        async fn observe_sound_candidates(
            &mut self,
            maximum_visible: usize,
        ) -> Result<Vec<SoundCandidate>, String> {
            self.note(format!("observe:{maximum_visible}"));
            Ok(self.candidates[..self.candidates.len().min(maximum_visible)].to_vec())
        }

        async fn choose_sound(&mut self, selection: &SoundSelectionEvidence) -> Result<(), String> {
            self.note(format!("choose:{}", selection.index));
            Ok(())
        }

        async fn confirm_sound(
            &mut self,
            _selection: &SoundSelectionEvidence,
        ) -> Result<bool, String> {
            self.note("reconfirm-sound");
            Ok(self.sound_confirmed)
        }

        async fn dispatch_post(
            &mut self,
            before_post: &mut (dyn FnMut() -> Result<(), String> + Send),
            _selection: &SoundSelectionEvidence,
        ) -> Result<(), String> {
            self.note("post-ready");
            if self.invoke_intent {
                before_post()?;
            }
            self.note("post-dispatch");
            match self.post_error.take() {
                Some(error) => Err(error),
                None => Ok(()),
            }
        }

        async fn confirm_post(&mut self) -> Result<Value, String> {
            self.note("confirm-post");
            Ok(serde_json::json!({"state":"posted"}))
        }

        async fn capture_canonical_link(
            &mut self,
            _bundle: &PublishBundle,
        ) -> Result<String, String> {
            self.note("capture-link");
            self.link.clone()
        }

        async fn write_sheet(
            &mut self,
            assignment_id: &str,
            _canonical_link: &str,
            _bundle: &PublishBundle,
        ) -> Result<(), String> {
            self.note("sheet");
            if let Some(error) = self.sheet_error.clone() {
                return Err(error);
            }
            self.written.insert(assignment_id.to_string());
            Ok(())
        }

        async fn cleanup(&mut self) -> Result<(), String> {
            self.note("cleanup");
            match self.cleanup_error.clone() {
                Some(error) => Err(error),
                None => Ok(()),
            }
        }
    }

    fn image_bundle() -> PublishBundle {
        PublishBundle {
            id: "bundle-1".into(),
            source_path: "/managed/bundle-1".into(),
            name: "bundle-1".into(),
            media_kind: PublishMediaKind::Image,
            images: vec![PublishImage {
                path: "/managed/bundle-1/01.jpg".into(),
                file_name: "01.jpg".into(),
                order: 1,
                sha256: "11".repeat(32),
                byte_len: 20,
                width: 1080,
                height: 1920,
            }],
            video: None,
            caption_path: "/managed/bundle-1/caption.txt".into(),
            caption: "fixture caption".into(),
            caption_sha256: "22".repeat(32),
            total_bytes: 35,
            partners: vec![],
        }
    }

    fn video_bundle() -> PublishBundle {
        PublishBundle {
            media_kind: PublishMediaKind::Video,
            images: vec![],
            video: Some(PublishVideo {
                path: "/managed/bundle-1/video.mp4".into(),
                file_name: "video.mp4".into(),
                sha256: "33".repeat(32),
                byte_len: 1_024,
                duration_ms: 5_000,
                video_codec: PublishVideoCodec::H264Avc,
                audio_codec: Some(PublishAudioCodec::Aac),
            }),
            ..image_bundle()
        }
    }

    fn input(bundle: PublishBundle) -> PublishExecutionInput {
        PublishExecutionInput {
            assignment_id: "assignment-1".into(),
            bundle,
            sound_policy: PublishSoundPolicy::TrendingAny {
                pool_size: 10,
                seed: 44,
            },
            confirmed: true,
            resume: PublishResumePoint::Full,
        }
    }

    #[test]
    fn preflight_and_snapshot_wire_contracts_are_camel_case_and_typed() {
        let request = PublishPreflightRequest {
            source_root: "C:/fixture".into(),
            bundle_ids: vec!["bundle-1".into()],
            udids: vec!["phone-1".into()],
            target_ref: Some(crate::TargetRef::Group {
                group_id: "morning".into(),
            }),
            run_at: None,
            caption_overrides: [("bundle-1".to_string(), "caption".to_string())]
                .into_iter()
                .collect(),
            sound_policy: PublishSoundPolicy::Default,
        };
        let request_json = serde_json::to_value(&request).expect("serialize request");
        assert_eq!(request_json["sourceRoot"], "C:/fixture");
        assert_eq!(request_json["targetRef"]["type"], "group");
        assert_eq!(request_json["captionOverrides"]["bundle-1"], "caption");
        assert_eq!(request_json["soundPolicy"]["kind"], "default");
        assert!(request_json.get("runAt").is_none());

        let report = PublishPreflightReport {
            input_digest: "a".repeat(64),
            target_snapshot: crate::resolve_target(
                &crate::TargetRef::Explicit {
                    udids: vec!["phone-1".into()],
                },
                &["phone-1".into()],
                &[],
                &[],
            )
            .expect("resolve target fixture"),
            can_execute: false,
            assignments: vec![PublishPreflightAssignmentReport {
                ordinal: 0,
                bundle_id: "bundle-1".into(),
                udid: "phone-1".into(),
                package_name: None,
                version: Some("38.3.2".into()),
                locale: None,
                media: PublishPreflightCheck::Pass,
                composer: PublishPreflightCheck::Pass,
                sound_picker: PublishPreflightCheck::Fail,
                storage: PublishPreflightCheck::Pass,
                required_bytes: 1024,
                available_bytes: Some(4096),
                issues: vec![PublishExecutionIssue {
                    code: "sound_picker_unmeasured".into(),
                    assignment_id: None,
                    udid: Some("phone-1".into()),
                    bundle_id: Some("bundle-1".into()),
                    message: "sound picker is not measured".into(),
                }],
            }],
            issues: Vec::new(),
            sheet_configured: false,
        };
        let report_json = serde_json::to_value(&report).expect("serialize report");
        assert_eq!(
            report_json["targetSnapshot"]["included"][0]["udid"],
            "phone-1"
        );
        assert_eq!(
            report_json["targetSnapshot"]["targetRef"]["type"],
            "explicit"
        );
        assert_eq!(report_json["assignments"][0]["soundPicker"], "fail");
        assert_eq!(report_json["assignments"][0]["media"], "pass");
        assert_eq!(report_json["assignments"][0]["storage"], "pass");
        assert_eq!(report_json["assignments"][0]["availableBytes"], 4096);
        assert!(report_json["assignments"][0].get("packageName").is_none());
        assert_eq!(report_json["sheetConfigured"], false);

        let snapshot = PublishExecutionSnapshot {
            campaign_id: "campaign-1".into(),
            input_digest: "a".repeat(64),
            status: PublishExecutionStatus::Partial,
            retry_scope: PublishRetryScope::LinkAndSheet,
            report_json: serde_json::json!({"canExecute": false}),
            updated_at: "2026-09-04T00:00:00Z".into(),
        };
        let snapshot_json = serde_json::to_value(&snapshot).expect("serialize snapshot");
        assert_eq!(snapshot_json["status"], "partial");
        assert_eq!(snapshot_json["retryScope"], "linkAndSheet");
        assert_eq!(snapshot_json["reportJson"]["canExecute"], false);
        assert_eq!(
            serde_json::from_value::<PublishExecutionSnapshot>(snapshot_json)
                .expect("restore snapshot"),
            snapshot
        );
    }

    #[tokio::test]
    async fn full_pipeline_bounds_visible_music_reconfirms_then_crosses_effect_once() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let mut port = FakePort::happy(calls.clone());
        let callback_calls = calls.clone();
        let result = run_publish_pipeline(input(image_bundle()), &mut port, move |selection| {
            callback_calls
                .lock()
                .push(format!("intent:{}", selection.index));
            Ok(())
        })
        .await;

        assert_eq!(result.status, PublishExecutionStatus::Complete);
        assert_eq!(result.retry_scope, PublishRetryScope::None);
        assert!(result.post_confirmed);
        let selection = result.sound_selection.expect("selection evidence");
        assert!(selection.confirmed);
        assert!(selection.index < 5, "only five visible rows are eligible");
        let calls = calls.lock();
        assert_eq!(
            calls
                .iter()
                .filter(|call| call.starts_with("intent:"))
                .count(),
            1
        );
        assert_eq!(
            calls.as_slice(),
            [
                "preflight",
                "transfer:Image",
                "observe:5",
                &format!("choose:{}", selection.index),
                "reconfirm-sound",
                "post-ready",
                &format!("intent:{}", selection.index),
                "post-dispatch",
                "confirm-post",
                "capture-link",
                "sheet",
                "cleanup",
            ]
        );
    }

    #[tokio::test]
    async fn mp4_uses_the_same_one_confirmation_pipeline() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let mut port = FakePort::happy(calls.clone());
        let result = run_publish_pipeline(input(video_bundle()), &mut port, |_| Ok(())).await;
        assert_eq!(result.status, PublishExecutionStatus::Complete);
        assert_eq!(result.media_kind, PublishMediaKind::Video);
        assert!(calls.lock().contains(&"transfer:Video".to_string()));
    }

    #[tokio::test]
    async fn error_before_intent_is_retryable_but_after_intent_is_uncertain() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let mut before = FakePort::happy(calls.clone());
        before.invoke_intent = false;
        before.post_error = Some("transport failed before Post".into());
        let result = run_publish_pipeline(input(image_bundle()), &mut before, |_| Ok(())).await;
        assert_eq!(result.status, PublishExecutionStatus::Partial);
        assert_eq!(result.retry_scope, PublishRetryScope::FullPipeline);
        assert!(!result.post_confirmed);

        let mut after = FakePort::happy(Arc::new(Mutex::new(Vec::new())));
        after.post_error = Some("transport failed after Post".into());
        let result = run_publish_pipeline(input(image_bundle()), &mut after, |_| Ok(())).await;
        assert_eq!(result.status, PublishExecutionStatus::Uncertain);
        assert_eq!(result.retry_scope, PublishRetryScope::None);
        assert!(!result.post_confirmed);
    }

    #[tokio::test]
    async fn sound_must_match_immediately_before_post() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let mut port = FakePort::happy(calls.clone());
        port.sound_confirmed = false;
        let intent_count = Arc::new(Mutex::new(0));
        let count = intent_count.clone();
        let result = run_publish_pipeline(input(image_bundle()), &mut port, move |_| {
            *count.lock() += 1;
            Ok(())
        })
        .await;
        assert_eq!(result.status, PublishExecutionStatus::Partial);
        assert_eq!(result.retry_scope, PublishRetryScope::FullPipeline);
        assert_eq!(*intent_count.lock(), 0);
        assert!(!calls.lock().contains(&"post-ready".to_string()));
    }

    #[tokio::test]
    async fn confirmed_post_resumes_at_link_or_sheet_without_posting_again() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let mut port = FakePort::happy(calls.clone());
        let mut resume = input(image_bundle());
        resume.resume = PublishResumePoint::ConfirmedPost {
            post_evidence: serde_json::json!({"state":"posted"}),
            canonical_link: None,
            sound_selection: None,
        };
        let result = run_publish_pipeline(resume, &mut port, |_| {
            panic!("a confirmed post must never cross Post again")
        })
        .await;
        assert_eq!(result.status, PublishExecutionStatus::Complete);
        assert_eq!(
            calls.lock().as_slice(),
            ["preflight", "capture-link", "sheet"]
        );

        let calls = Arc::new(Mutex::new(Vec::new()));
        let mut port = FakePort::happy(calls.clone());
        let mut resume = input(image_bundle());
        resume.resume = PublishResumePoint::ConfirmedPost {
            post_evidence: serde_json::json!({"state":"posted"}),
            canonical_link: Some(
                "https://www.tiktok.com/@fixture/video/7400000000000000001".into(),
            ),
            sound_selection: None,
        };
        let result = run_publish_pipeline(resume, &mut port, |_| unreachable!()).await;
        assert_eq!(result.status, PublishExecutionStatus::Complete);
        assert_eq!(calls.lock().as_slice(), ["preflight", "sheet"]);
    }

    #[tokio::test]
    async fn confirmed_post_with_missing_link_is_partial_not_retryable_at_post() {
        let mut port = FakePort::happy(Arc::new(Mutex::new(Vec::new())));
        port.link = Err("own post was not found".into());
        let mut resume = input(image_bundle());
        resume.resume = PublishResumePoint::ConfirmedPost {
            post_evidence: serde_json::json!({"state":"posted"}),
            canonical_link: None,
            sound_selection: None,
        };
        let result = run_publish_pipeline(resume, &mut port, |_| unreachable!()).await;
        assert_eq!(result.status, PublishExecutionStatus::Partial);
        assert_eq!(result.retry_scope, PublishRetryScope::LinkAndSheet);
        assert!(result.post_confirmed);
    }

    #[tokio::test]
    async fn confirmed_resume_keeps_its_durable_evidence_when_preflight_stops() {
        let mut port = FakePort::happy(Arc::new(Mutex::new(Vec::new())));
        port.preflight_error = Some("Sheet is not configured".into());
        let mut resume = input(image_bundle());
        let evidence = serde_json::json!({"state":"posted","proof":"frame-7"});
        let link = "https://www.tiktok.com/@fixture/video/7400000000000000001";
        let sound = SoundSelectionEvidence {
            section: SoundSectionKind::Trending,
            title: "A".into(),
            artist: "B".into(),
            index: 1,
            candidates_digest: "aa".repeat(32),
            confirmed: true,
        };
        resume.resume = PublishResumePoint::ConfirmedPost {
            post_evidence: evidence.clone(),
            canonical_link: Some(link.into()),
            sound_selection: Some(sound.clone()),
        };

        let result = run_publish_pipeline(resume, &mut port, |_| unreachable!()).await;

        assert_eq!(result.status, PublishExecutionStatus::Partial);
        assert_eq!(result.retry_scope, PublishRetryScope::SheetOnly);
        assert!(result.post_confirmed);
        assert_eq!(result.post_evidence, Some(evidence));
        assert_eq!(result.canonical_link.as_deref(), Some(link));
        assert_eq!(result.sound_selection, Some(sound));
    }

    #[tokio::test]
    async fn declining_a_confirmed_resume_never_offers_the_full_pipeline() {
        for (link, expected) in [
            (None, PublishRetryScope::LinkAndSheet),
            (
                Some("https://www.tiktok.com/@fixture/video/7400000000000000001".to_string()),
                PublishRetryScope::SheetOnly,
            ),
        ] {
            let calls = Arc::new(Mutex::new(Vec::new()));
            let mut port = FakePort::happy(calls.clone());
            let mut resume = input(image_bundle());
            resume.confirmed = false;
            resume.resume = PublishResumePoint::ConfirmedPost {
                post_evidence: serde_json::json!({"state":"posted"}),
                canonical_link: link,
                sound_selection: None,
            };

            let result = run_publish_pipeline(resume, &mut port, |_| {
                panic!("a confirmed post must never cross Post again")
            })
            .await;

            assert_eq!(result.retry_scope, expected);
            assert!(result.post_confirmed);
            assert!(calls.lock().is_empty());
        }
    }

    #[tokio::test]
    async fn sheet_failure_retries_only_the_idempotent_sheet_write() {
        let mut port = FakePort::happy(Arc::new(Mutex::new(Vec::new())));
        port.sheet_error = Some("webhook offline".into());
        let result = run_publish_pipeline(input(image_bundle()), &mut port, |_| Ok(())).await;
        assert_eq!(result.status, PublishExecutionStatus::Partial);
        assert_eq!(result.retry_scope, PublishRetryScope::SheetOnly);
        assert!(result.post_confirmed);
        assert!(result.canonical_link.is_some());
    }

    #[tokio::test]
    async fn cleanup_failure_is_a_warning_and_does_not_downgrade_completion() {
        let mut port = FakePort::happy(Arc::new(Mutex::new(Vec::new())));
        port.cleanup_error = Some("media remains".into());
        let result = run_publish_pipeline(input(image_bundle()), &mut port, |_| Ok(())).await;
        assert_eq!(result.status, PublishExecutionStatus::Complete);
        assert_eq!(result.cleanup_warning.as_deref(), Some("media remains"));
    }

    #[test]
    fn evidence_shape_keeps_the_observed_section() {
        let value = SoundSelectionEvidence {
            section: SoundSectionKind::Trending,
            title: "A".into(),
            artist: "B".into(),
            index: 1,
            candidates_digest: "aa".repeat(32),
            confirmed: true,
        };
        assert_eq!(value.section, SoundSectionKind::Trending);
    }
}
