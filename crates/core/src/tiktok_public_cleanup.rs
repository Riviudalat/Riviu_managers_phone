//! Fail-closed state machines for undoing TikTok public effects.
//!
//! These contracts deliberately do not contain locators or coordinates. An Android adapter may
//! only implement them after the corresponding package/build/locale has been measured. Until
//! then it returns an unreadable observation or refuses preparation, and no public tap occurs.

use crate::types::TapPoint;
use serde::{Deserialize, Serialize};

/// Public effect an operator may ask the cleanup runtime to reverse.
///
/// This is deliberately wider than [`crate::InteractionActionKind`]: a published post is not an
/// Interaction action, but it still needs to fail closed through the same preflight surface while
/// its delete controls remain unmeasured.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PublicCleanupKind {
    Like,
    Save,
    Comment,
    Follow,
    Post,
}

impl PublicCleanupKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Like => "like",
            Self::Save => "save",
            Self::Comment => "comment",
            Self::Follow => "follow",
            Self::Post => "post",
        }
    }
}

/// Whether the production runtime may proceed beyond preflight.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PublicCleanupCapabilityStatus {
    /// A measured adapter exists. The executor must still re-open and re-prove the exact target.
    ReadyForTargetProof,
    /// The source action did not prove that this campaign created the public effect.
    SourceNotConfirmed,
    /// The source phone cannot provide the hierarchy state required by the measured adapter.
    HierarchyRequired,
    /// One or more irreversible controls have not been measured for the fleet.
    UnsupportedUnmeasured,
}

impl PublicCleanupCapabilityStatus {
    pub fn can_execute(self) -> bool {
        matches!(self, Self::ReadyForTargetProof)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PublicCleanupCapability {
    pub kind: PublicCleanupKind,
    pub status: PublicCleanupCapabilityStatus,
    pub reason: String,
    pub device_udid: Option<String>,
    pub effect_boundary_crossed: bool,
}

/// Static capability for paths whose missing proof is independent of the selected phone.
///
/// Unlike and Unsave continue to the source/device checks. Follow cannot: the catalog knows how
/// to identify an *unfollowed* author (`Follow <author>`) but has no positive Following/Unfollow
/// control on the same card. Comment and post deletion likewise have no measured menu/confirm
/// chain. Returning this value performs no device I/O.
pub fn static_public_cleanup_capability(kind: PublicCleanupKind) -> PublicCleanupCapability {
    let (status, reason) = match kind {
        PublicCleanupKind::Like => (
            PublicCleanupCapabilityStatus::ReadyForTargetProof,
            "Unlike requires a confirmed Like, hierarchy state and a fresh canonical-card proof",
        ),
        PublicCleanupKind::Save => (
            PublicCleanupCapabilityStatus::ReadyForTargetProof,
            "Unsave requires a confirmed Save, readable Saved state and a fresh canonical-card proof",
        ),
        PublicCleanupKind::Follow => (
            PublicCleanupCapabilityStatus::UnsupportedUnmeasured,
            "Following/Friends is measured only before effect on com.ss.android.ugc.trill en \
             38.3.2; the campaign-bound canonical source identity is not produced by the \
             Nurture engines and the post-unfollow Follow readback is unmeasured",
        ),
        PublicCleanupKind::Comment => (
            PublicCleanupCapabilityStatus::UnsupportedUnmeasured,
            "Owned-comment row, menu and Delete controls are measured only on \
             com.ss.android.ugc.trill en 38.3.2; durable campaign source binding and a \
             positive post-delete absence proof are not implemented",
        ),
        PublicCleanupKind::Post => (
            PublicCleanupCapabilityStatus::UnsupportedUnmeasured,
            "Delete navigation is measured only on com.ss.android.ugc.trill en 38.3.2; the \
             publish cleanup journal and strong canonical/grid absence readback are not implemented",
        ),
    };
    PublicCleanupCapability {
        kind,
        status,
        reason: reason.to_owned(),
        device_udid: None,
        effect_boundary_crossed: false,
    }
}

/// Durable state of one cleanup request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PublicCleanupRunState {
    Planned,
    Preparing,
    Armed,
    Cleared,
    AlreadyClear,
    FailedBeforeEffect,
    Uncertain,
}

impl PublicCleanupRunState {
    pub fn retry_is_safe(self) -> bool {
        matches!(self, Self::Planned | Self::FailedBeforeEffect)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PublicCleanupRunRecord {
    pub id: String,
    pub request_id: String,
    pub source_action_run_id: String,
    pub campaign_id: String,
    pub assignment_id: String,
    pub device_udid: String,
    pub kind: PublicCleanupKind,
    pub target_json: String,
    pub state: PublicCleanupRunState,
    pub revision: i64,
    pub effect_intent: Option<String>,
    pub evidence: Option<String>,
    pub error: Option<String>,
    pub updated_at: String,
}

/// Immutable source facts used to bind a reversal to the action that created the effect.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PublicCleanupSourceAction {
    pub action_run_id: String,
    pub campaign_id: String,
    pub assignment_id: String,
    pub device_udid: String,
    pub target_key: String,
    pub kind: PublicCleanupKind,
    pub source_confirmed: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PublicCleanupRecovery {
    pub retryable: u32,
    pub uncertain: u32,
}

/// Whether the public effect this cleanup owns is currently present.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PublicEffectState {
    Present,
    Absent,
    Unreadable,
}

/// Stable identity of the exact public object a cleanup attempt owns.
///
/// A post carries TikTok's immutable content id. A comment has no public id available in the
/// measured hierarchy, so it is bound to the post id, the campaign attempt id, the account and
/// the digest of the exact text that the campaign persisted before typing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum PublicCleanupIdentity {
    Post {
        content_id: String,
        author: String,
        caption_sha256: String,
    },
    Comment {
        content_id: String,
        attempt_id: String,
        author: String,
        text_sha256: String,
    },
    Toggle {
        /// Digest of the card identity read by the concrete adapter. A caller that has a
        /// canonical content id should include it in that identity before hashing.
        card_key: String,
        author: String,
        effect: PublicToggle,
    },
}

impl PublicCleanupIdentity {
    fn is_complete(&self) -> bool {
        match self {
            Self::Post {
                content_id,
                author,
                caption_sha256,
            } => nonempty(content_id) && nonempty(author) && valid_sha256(caption_sha256),
            Self::Comment {
                content_id,
                attempt_id,
                author,
                text_sha256,
            } => {
                nonempty(content_id)
                    && nonempty(attempt_id)
                    && nonempty(author)
                    && valid_sha256(text_sha256)
            }
            Self::Toggle {
                card_key, author, ..
            } => valid_sha256(card_key) && nonempty(author),
        }
    }
}

fn nonempty(value: &str) -> bool {
    !value.trim().is_empty() && value.trim() == value
}

fn valid_sha256(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PublicToggle {
    Like,
    Save,
    Follow,
}

/// One fresh observation of a Like/Save/Follow state on a specific card.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToggleCleanupObservation {
    pub identity: Option<PublicCleanupIdentity>,
    pub sequence: u64,
    pub state: PublicEffectState,
    pub tap_point: Option<TapPoint>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ToggleCleanupVerdict {
    Cleared,
    AlreadyClear,
    NoControl,
    StateUnreadable,
    TargetChangedBeforeEffect,
    FailedBeforeEffect,
    TargetChangedAfterEffect,
    NotConfirmed,
    UncertainAfterEffect,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToggleCleanupEvidence {
    pub verdict: ToggleCleanupVerdict,
    pub initial: Option<ToggleCleanupObservation>,
    pub final_observation: Option<ToggleCleanupObservation>,
    pub effect_boundary_crossed: bool,
    pub error: Option<String>,
}

impl ToggleCleanupEvidence {
    pub fn retry_is_safe(&self) -> bool {
        !self.effect_boundary_crossed
    }
}

#[async_trait::async_trait]
pub trait ToggleCleanupAdapter: Send {
    async fn observe(&mut self) -> anyhow::Result<ToggleCleanupObservation>;
    async fn tap(&mut self, point: TapPoint) -> anyhow::Result<()>;
}

fn toggle_evidence(
    verdict: ToggleCleanupVerdict,
    initial: Option<ToggleCleanupObservation>,
    final_observation: Option<ToggleCleanupObservation>,
    effect_boundary_crossed: bool,
    error: Option<String>,
) -> ToggleCleanupEvidence {
    ToggleCleanupEvidence {
        verdict,
        initial,
        final_observation,
        effect_boundary_crossed,
        error,
    }
}

fn toggle_refusal(observation: &ToggleCleanupObservation) -> Option<ToggleCleanupVerdict> {
    if observation
        .identity
        .as_ref()
        .is_none_or(|id| !id.is_complete())
        || observation.state == PublicEffectState::Unreadable
    {
        return Some(ToggleCleanupVerdict::StateUnreadable);
    }
    if observation.state == PublicEffectState::Absent {
        return Some(ToggleCleanupVerdict::AlreadyClear);
    }
    observation
        .tap_point
        .is_none()
        .then_some(ToggleCleanupVerdict::NoControl)
}

/// Reach the absent state for one Like, Save or Follow with at most one tap.
///
/// This is intentionally separate from the action primitives that create effects. A cleanup
/// adapter must positively read `Present`; "the Follow button is gone" does not prove a follow
/// exists and therefore cannot authorize an unfollow. `durable_intent` is the one-shot boundary
/// immediately before the tap.
pub async fn clear_public_toggle<A, F>(adapter: &mut A, durable_intent: F) -> ToggleCleanupEvidence
where
    A: ToggleCleanupAdapter + ?Sized,
    F: FnOnce(&ToggleCleanupObservation) -> anyhow::Result<()>,
{
    let initial = match adapter.observe().await {
        Ok(observation) => observation,
        Err(error) => {
            return toggle_evidence(
                ToggleCleanupVerdict::FailedBeforeEffect,
                None,
                None,
                false,
                Some(error.to_string()),
            )
        }
    };
    if let Some(verdict) = toggle_refusal(&initial) {
        return toggle_evidence(verdict, Some(initial), None, false, None);
    }

    let reproved = match adapter.observe().await {
        Ok(observation) => observation,
        Err(error) => {
            return toggle_evidence(
                ToggleCleanupVerdict::FailedBeforeEffect,
                Some(initial),
                None,
                false,
                Some(error.to_string()),
            )
        }
    };
    if reproved.sequence <= initial.sequence {
        return toggle_evidence(
            ToggleCleanupVerdict::FailedBeforeEffect,
            Some(initial),
            Some(reproved),
            false,
            Some("the immediate cleanup re-proof was not newer".to_owned()),
        );
    }
    if reproved.identity.is_none() || reproved.identity != initial.identity {
        return toggle_evidence(
            ToggleCleanupVerdict::TargetChangedBeforeEffect,
            Some(initial),
            Some(reproved),
            false,
            None,
        );
    }
    if let Some(verdict) = toggle_refusal(&reproved) {
        return toggle_evidence(verdict, Some(initial), Some(reproved), false, None);
    }

    if let Err(error) = durable_intent(&reproved) {
        return toggle_evidence(
            ToggleCleanupVerdict::FailedBeforeEffect,
            Some(initial),
            Some(reproved),
            false,
            Some(error.to_string()),
        );
    }
    let point = reproved
        .tap_point
        .clone()
        .expect("toggle_refusal returned for a missing cleanup control");
    if let Err(error) = adapter.tap(point).await {
        return toggle_evidence(
            ToggleCleanupVerdict::UncertainAfterEffect,
            Some(initial),
            Some(reproved),
            true,
            Some(error.to_string()),
        );
    }

    let confirmed = match adapter.observe().await {
        Ok(observation) => observation,
        Err(error) => {
            return toggle_evidence(
                ToggleCleanupVerdict::UncertainAfterEffect,
                Some(initial),
                Some(reproved),
                true,
                Some(error.to_string()),
            )
        }
    };
    let verdict = if confirmed.identity.is_none() || confirmed.identity != reproved.identity {
        ToggleCleanupVerdict::TargetChangedAfterEffect
    } else if confirmed.sequence <= reproved.sequence {
        ToggleCleanupVerdict::NotConfirmed
    } else if confirmed.state == PublicEffectState::Absent {
        ToggleCleanupVerdict::Cleared
    } else {
        ToggleCleanupVerdict::NotConfirmed
    };
    toggle_evidence(verdict, Some(initial), Some(confirmed), true, None)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum OwnershipProof {
    /// Positive ownership markers and campaign-bound identity both matched.
    Strong,
    /// The adapter only inferred ownership from an absent foreign-author control.
    NegativeOnly,
    Unreadable,
}

/// Observation of the exact comment/post the campaign intends to delete.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteTargetObservation {
    pub identity: Option<PublicCleanupIdentity>,
    pub sequence: u64,
    pub state: PublicEffectState,
    pub ownership: OwnershipProof,
}

/// UI prepared immediately before the irreversible confirmation tap.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PreparedDelete {
    pub target: DeleteTargetObservation,
    /// Must contain exactly one measured confirmation control.
    pub confirmation_points: Vec<TapPoint>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DeleteCleanupVerdict {
    Deleted,
    AlreadyAbsent,
    UnsupportedBeforeEffect,
    OwnershipUnproved,
    TargetChangedBeforeEffect,
    FailedBeforeEffect,
    TargetChangedAfterEffect,
    NotConfirmed,
    UncertainAfterEffect,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteCleanupEvidence {
    pub verdict: DeleteCleanupVerdict,
    pub initial: Option<DeleteTargetObservation>,
    pub final_observation: Option<DeleteTargetObservation>,
    pub effect_boundary_crossed: bool,
    pub error: Option<String>,
}

impl DeleteCleanupEvidence {
    pub fn retry_is_safe(&self) -> bool {
        !self.effect_boundary_crossed
    }
}

/// Backend contract for deleting an owned post/comment.
///
/// `prepare_delete` may open menus, but it must not tap the irreversible confirmation. It must
/// return the target still visible/re-readable in that UI and every matching confirmation
/// control. If the dialog hides identity, preparation has to refuse rather than manufacture it.
/// `observe_expected_target` after confirmation must query the expected identity itself; merely
/// seeing a different card is not absence proof.
#[async_trait::async_trait]
pub trait OwnedDeleteAdapter: Send {
    async fn observe_expected_target(&mut self) -> anyhow::Result<DeleteTargetObservation>;
    async fn prepare_delete(&mut self) -> anyhow::Result<PreparedDelete>;
    async fn confirm_delete(&mut self, point: TapPoint) -> anyhow::Result<()>;
}

fn delete_evidence(
    verdict: DeleteCleanupVerdict,
    initial: Option<DeleteTargetObservation>,
    final_observation: Option<DeleteTargetObservation>,
    effect_boundary_crossed: bool,
    error: Option<String>,
) -> DeleteCleanupEvidence {
    DeleteCleanupEvidence {
        verdict,
        initial,
        final_observation,
        effect_boundary_crossed,
        error,
    }
}

fn owned_target_refusal(observation: &DeleteTargetObservation) -> Option<DeleteCleanupVerdict> {
    if observation
        .identity
        .as_ref()
        .is_none_or(|id| !id.is_complete())
        || observation.state == PublicEffectState::Unreadable
    {
        return Some(DeleteCleanupVerdict::OwnershipUnproved);
    }
    if observation.state == PublicEffectState::Absent {
        return Some(DeleteCleanupVerdict::AlreadyAbsent);
    }
    if observation.ownership != OwnershipProof::Strong {
        return Some(DeleteCleanupVerdict::OwnershipUnproved);
    }
    None
}

/// Delete exactly one campaign-owned comment or post with one irreversible tap at most.
pub async fn delete_owned_content<A, F>(adapter: &mut A, durable_intent: F) -> DeleteCleanupEvidence
where
    A: OwnedDeleteAdapter + ?Sized,
    F: FnOnce(&DeleteTargetObservation) -> anyhow::Result<()>,
{
    let initial = match adapter.observe_expected_target().await {
        Ok(observation) => observation,
        Err(error) => {
            return delete_evidence(
                DeleteCleanupVerdict::FailedBeforeEffect,
                None,
                None,
                false,
                Some(error.to_string()),
            )
        }
    };
    if let Some(verdict) = owned_target_refusal(&initial) {
        return delete_evidence(verdict, Some(initial), None, false, None);
    }

    let prepared = match adapter.prepare_delete().await {
        Ok(prepared) => prepared,
        Err(error) => {
            return delete_evidence(
                DeleteCleanupVerdict::UnsupportedBeforeEffect,
                Some(initial),
                None,
                false,
                Some(error.to_string()),
            )
        }
    };
    if prepared.target.sequence <= initial.sequence
        || prepared.target.identity.is_none()
        || prepared.target.identity != initial.identity
    {
        return delete_evidence(
            DeleteCleanupVerdict::TargetChangedBeforeEffect,
            Some(initial),
            Some(prepared.target),
            false,
            None,
        );
    }
    if let Some(verdict) = owned_target_refusal(&prepared.target) {
        return delete_evidence(verdict, Some(initial), Some(prepared.target), false, None);
    }
    if prepared.confirmation_points.len() != 1 {
        return delete_evidence(
            DeleteCleanupVerdict::UnsupportedBeforeEffect,
            Some(initial),
            Some(prepared.target),
            false,
            Some(format!(
                "expected exactly one measured delete confirmation control, found {}",
                prepared.confirmation_points.len()
            )),
        );
    }
    if prepared
        .confirmation_points
        .iter()
        .any(|point| !point.x.is_finite() || !point.y.is_finite() || point.x < 0.0 || point.y < 0.0)
    {
        return delete_evidence(
            DeleteCleanupVerdict::UnsupportedBeforeEffect,
            Some(initial),
            Some(prepared.target),
            false,
            Some("delete confirmation control has invalid geometry".to_owned()),
        );
    }
    if let Err(error) = durable_intent(&prepared.target) {
        return delete_evidence(
            DeleteCleanupVerdict::FailedBeforeEffect,
            Some(initial),
            Some(prepared.target),
            false,
            Some(error.to_string()),
        );
    }

    let point = prepared.confirmation_points[0].clone();
    if let Err(error) = adapter.confirm_delete(point).await {
        return delete_evidence(
            DeleteCleanupVerdict::UncertainAfterEffect,
            Some(initial),
            Some(prepared.target),
            true,
            Some(error.to_string()),
        );
    }
    let confirmed = match adapter.observe_expected_target().await {
        Ok(observation) => observation,
        Err(error) => {
            return delete_evidence(
                DeleteCleanupVerdict::UncertainAfterEffect,
                Some(initial),
                Some(prepared.target),
                true,
                Some(error.to_string()),
            )
        }
    };
    let verdict = if confirmed.identity.is_none() || confirmed.identity != prepared.target.identity
    {
        DeleteCleanupVerdict::TargetChangedAfterEffect
    } else if confirmed.sequence <= prepared.target.sequence {
        DeleteCleanupVerdict::NotConfirmed
    } else if confirmed.state == PublicEffectState::Absent {
        DeleteCleanupVerdict::Deleted
    } else {
        DeleteCleanupVerdict::NotConfirmed
    };
    delete_evidence(verdict, Some(initial), Some(confirmed), true, None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    #[test]
    fn unmeasured_destructive_paths_are_typed_zero_effect_refusals() {
        for kind in [
            PublicCleanupKind::Comment,
            PublicCleanupKind::Follow,
            PublicCleanupKind::Post,
        ] {
            let capability = static_public_cleanup_capability(kind);
            assert_eq!(
                capability.status,
                PublicCleanupCapabilityStatus::UnsupportedUnmeasured
            );
            assert!(!capability.status.can_execute());
            assert!(!capability.effect_boundary_crossed);
            assert!(!capability.reason.trim().is_empty());
        }
        assert!(static_public_cleanup_capability(PublicCleanupKind::Like)
            .status
            .can_execute());
        assert!(static_public_cleanup_capability(PublicCleanupKind::Save)
            .status
            .can_execute());
    }

    #[test]
    fn cleanup_wire_names_and_retry_states_are_stable() {
        for (kind, wire) in [
            (PublicCleanupKind::Like, "like"),
            (PublicCleanupKind::Save, "save"),
            (PublicCleanupKind::Comment, "comment"),
            (PublicCleanupKind::Follow, "follow"),
            (PublicCleanupKind::Post, "post"),
        ] {
            assert_eq!(serde_json::to_value(kind).expect("kind"), wire);
        }
        assert!(PublicCleanupRunState::Planned.retry_is_safe());
        assert!(PublicCleanupRunState::FailedBeforeEffect.retry_is_safe());
        for state in [
            PublicCleanupRunState::Preparing,
            PublicCleanupRunState::Armed,
            PublicCleanupRunState::Cleared,
            PublicCleanupRunState::AlreadyClear,
            PublicCleanupRunState::Uncertain,
        ] {
            assert!(!state.retry_is_safe(), "{state:?} unexpectedly retryable");
        }
    }

    fn hash(byte: char) -> String {
        std::iter::repeat_n(byte, 64).collect()
    }

    fn toggle_identity(effect: PublicToggle, content: &str) -> PublicCleanupIdentity {
        PublicCleanupIdentity::Toggle {
            card_key: format!("{:0<64}", content),
            author: "@riviu_canary".to_owned(),
            effect,
        }
    }

    fn toggle_observation(
        sequence: u64,
        state: PublicEffectState,
        content: &str,
    ) -> ToggleCleanupObservation {
        ToggleCleanupObservation {
            identity: Some(toggle_identity(PublicToggle::Like, content)),
            sequence,
            state,
            tap_point: Some(TapPoint { x: 10.0, y: 20.0 }),
        }
    }

    struct ToggleFixture {
        observations: VecDeque<anyhow::Result<ToggleCleanupObservation>>,
        taps: usize,
        tap_error: Option<&'static str>,
    }

    #[async_trait::async_trait]
    impl ToggleCleanupAdapter for ToggleFixture {
        async fn observe(&mut self) -> anyhow::Result<ToggleCleanupObservation> {
            self.observations.pop_front().expect("fixture observation")
        }

        async fn tap(&mut self, _point: TapPoint) -> anyhow::Result<()> {
            self.taps += 1;
            match self.tap_error {
                Some(error) => anyhow::bail!(error),
                None => Ok(()),
            }
        }
    }

    #[tokio::test]
    async fn present_toggle_is_cleared_with_one_intent_and_one_tap() {
        let mut fixture = ToggleFixture {
            observations: VecDeque::from([
                Ok(toggle_observation(1, PublicEffectState::Present, "741")),
                Ok(toggle_observation(2, PublicEffectState::Present, "741")),
                Ok(toggle_observation(3, PublicEffectState::Absent, "741")),
            ]),
            taps: 0,
            tap_error: None,
        };
        let mut intents = 0;
        let evidence = clear_public_toggle(&mut fixture, |_| {
            intents += 1;
            Ok(())
        })
        .await;
        assert_eq!(evidence.verdict, ToggleCleanupVerdict::Cleared);
        assert_eq!((intents, fixture.taps), (1, 1));
        assert!(!evidence.retry_is_safe());
    }

    #[tokio::test]
    async fn no_op_and_unreadable_toggle_never_tap() {
        for (state, verdict) in [
            (
                PublicEffectState::Absent,
                ToggleCleanupVerdict::AlreadyClear,
            ),
            (
                PublicEffectState::Unreadable,
                ToggleCleanupVerdict::StateUnreadable,
            ),
        ] {
            let mut fixture = ToggleFixture {
                observations: VecDeque::from([Ok(toggle_observation(1, state, "741"))]),
                taps: 0,
                tap_error: None,
            };
            let evidence = clear_public_toggle(&mut fixture, |_| Ok(())).await;
            assert_eq!(evidence.verdict, verdict);
            assert_eq!(fixture.taps, 0);
            assert!(evidence.retry_is_safe());
        }
    }

    #[tokio::test]
    async fn card_a_to_b_before_tap_is_zero_effect() {
        let mut fixture = ToggleFixture {
            observations: VecDeque::from([
                Ok(toggle_observation(1, PublicEffectState::Present, "A")),
                Ok(toggle_observation(2, PublicEffectState::Present, "B")),
            ]),
            taps: 0,
            tap_error: None,
        };
        let evidence = clear_public_toggle(&mut fixture, |_| Ok(())).await;
        assert_eq!(
            evidence.verdict,
            ToggleCleanupVerdict::TargetChangedBeforeEffect
        );
        assert_eq!(fixture.taps, 0);
        assert!(evidence.retry_is_safe());
    }

    #[tokio::test]
    async fn transport_error_after_toggle_tap_is_uncertain_and_not_retryable() {
        let mut fixture = ToggleFixture {
            observations: VecDeque::from([
                Ok(toggle_observation(1, PublicEffectState::Present, "741")),
                Ok(toggle_observation(2, PublicEffectState::Present, "741")),
            ]),
            taps: 0,
            tap_error: Some("transport closed"),
        };
        let evidence = clear_public_toggle(&mut fixture, |_| Ok(())).await;
        assert_eq!(evidence.verdict, ToggleCleanupVerdict::UncertainAfterEffect);
        assert_eq!(fixture.taps, 1);
        assert!(!evidence.retry_is_safe());
    }

    fn post_identity(content: &str) -> PublicCleanupIdentity {
        PublicCleanupIdentity::Post {
            content_id: content.to_owned(),
            author: "@riviu_canary".to_owned(),
            caption_sha256: hash('a'),
        }
    }

    fn delete_observation(
        sequence: u64,
        state: PublicEffectState,
        content: &str,
        ownership: OwnershipProof,
    ) -> DeleteTargetObservation {
        DeleteTargetObservation {
            identity: Some(post_identity(content)),
            sequence,
            state,
            ownership,
        }
    }

    struct DeleteFixture {
        observations: VecDeque<anyhow::Result<DeleteTargetObservation>>,
        prepared: Option<anyhow::Result<PreparedDelete>>,
        confirms: usize,
        confirm_error: Option<&'static str>,
    }

    #[async_trait::async_trait]
    impl OwnedDeleteAdapter for DeleteFixture {
        async fn observe_expected_target(&mut self) -> anyhow::Result<DeleteTargetObservation> {
            self.observations.pop_front().expect("fixture observation")
        }

        async fn prepare_delete(&mut self) -> anyhow::Result<PreparedDelete> {
            self.prepared.take().expect("fixture preparation")
        }

        async fn confirm_delete(&mut self, _point: TapPoint) -> anyhow::Result<()> {
            self.confirms += 1;
            match self.confirm_error {
                Some(error) => anyhow::bail!(error),
                None => Ok(()),
            }
        }
    }

    fn prepared(content: &str, confirmations: usize) -> PreparedDelete {
        PreparedDelete {
            target: delete_observation(
                2,
                PublicEffectState::Present,
                content,
                OwnershipProof::Strong,
            ),
            confirmation_points: (0..confirmations)
                .map(|index| TapPoint {
                    x: index as f64 + 1.0,
                    y: 2.0,
                })
                .collect(),
        }
    }

    #[tokio::test]
    async fn owned_target_deletes_once_and_requires_absence_readback() {
        let mut fixture = DeleteFixture {
            observations: VecDeque::from([
                Ok(delete_observation(
                    1,
                    PublicEffectState::Present,
                    "741",
                    OwnershipProof::Strong,
                )),
                Ok(delete_observation(
                    3,
                    PublicEffectState::Absent,
                    "741",
                    OwnershipProof::Strong,
                )),
            ]),
            prepared: Some(Ok(prepared("741", 1))),
            confirms: 0,
            confirm_error: None,
        };
        let mut intents = 0;
        let evidence = delete_owned_content(&mut fixture, |_| {
            intents += 1;
            Ok(())
        })
        .await;
        assert_eq!(evidence.verdict, DeleteCleanupVerdict::Deleted);
        assert_eq!((intents, fixture.confirms), (1, 1));
        assert!(!evidence.retry_is_safe());
    }

    #[tokio::test]
    async fn ownership_inferred_only_from_absence_refuses_before_menu() {
        let mut fixture = DeleteFixture {
            observations: VecDeque::from([Ok(delete_observation(
                1,
                PublicEffectState::Present,
                "741",
                OwnershipProof::NegativeOnly,
            ))]),
            prepared: None,
            confirms: 0,
            confirm_error: None,
        };
        let evidence = delete_owned_content(&mut fixture, |_| Ok(())).await;
        assert_eq!(evidence.verdict, DeleteCleanupVerdict::OwnershipUnproved);
        assert_eq!(fixture.confirms, 0);
        assert!(evidence.retry_is_safe());
    }

    #[tokio::test]
    async fn zero_or_duplicate_delete_controls_refuse_before_effect() {
        for count in [0, 2] {
            let mut fixture = DeleteFixture {
                observations: VecDeque::from([Ok(delete_observation(
                    1,
                    PublicEffectState::Present,
                    "741",
                    OwnershipProof::Strong,
                ))]),
                prepared: Some(Ok(prepared("741", count))),
                confirms: 0,
                confirm_error: None,
            };
            let evidence = delete_owned_content(&mut fixture, |_| Ok(())).await;
            assert_eq!(
                evidence.verdict,
                DeleteCleanupVerdict::UnsupportedBeforeEffect
            );
            assert_eq!(fixture.confirms, 0);
            assert!(evidence.retry_is_safe());
        }
    }

    #[tokio::test]
    async fn target_a_to_b_while_opening_menu_refuses_before_confirm() {
        let mut fixture = DeleteFixture {
            observations: VecDeque::from([Ok(delete_observation(
                1,
                PublicEffectState::Present,
                "A",
                OwnershipProof::Strong,
            ))]),
            prepared: Some(Ok(prepared("B", 1))),
            confirms: 0,
            confirm_error: None,
        };
        let evidence = delete_owned_content(&mut fixture, |_| Ok(())).await;
        assert_eq!(
            evidence.verdict,
            DeleteCleanupVerdict::TargetChangedBeforeEffect
        );
        assert_eq!(fixture.confirms, 0);
    }

    #[tokio::test]
    async fn confirm_transport_error_is_uncertain_and_never_retryable() {
        let mut fixture = DeleteFixture {
            observations: VecDeque::from([Ok(delete_observation(
                1,
                PublicEffectState::Present,
                "741",
                OwnershipProof::Strong,
            ))]),
            prepared: Some(Ok(prepared("741", 1))),
            confirms: 0,
            confirm_error: Some("connection reset"),
        };
        let evidence = delete_owned_content(&mut fixture, |_| Ok(())).await;
        assert_eq!(evidence.verdict, DeleteCleanupVerdict::UncertainAfterEffect);
        assert_eq!(fixture.confirms, 1);
        assert!(!evidence.retry_is_safe());
    }

    #[tokio::test]
    async fn disappearance_without_expected_identity_is_not_absence_proof() {
        let mut disappeared =
            delete_observation(3, PublicEffectState::Absent, "741", OwnershipProof::Strong);
        disappeared.identity = None;
        let mut fixture = DeleteFixture {
            observations: VecDeque::from([
                Ok(delete_observation(
                    1,
                    PublicEffectState::Present,
                    "741",
                    OwnershipProof::Strong,
                )),
                Ok(disappeared),
            ]),
            prepared: Some(Ok(prepared("741", 1))),
            confirms: 0,
            confirm_error: None,
        };
        let evidence = delete_owned_content(&mut fixture, |_| Ok(())).await;
        assert_eq!(
            evidence.verdict,
            DeleteCleanupVerdict::TargetChangedAfterEffect
        );
        assert_eq!(fixture.confirms, 1);
        assert!(!evidence.retry_is_safe());
    }

    #[tokio::test]
    async fn invalid_confirmation_geometry_never_crosses_the_boundary() {
        let mut invalid = prepared("741", 1);
        invalid.confirmation_points[0].x = f64::NAN;
        let mut fixture = DeleteFixture {
            observations: VecDeque::from([Ok(delete_observation(
                1,
                PublicEffectState::Present,
                "741",
                OwnershipProof::Strong,
            ))]),
            prepared: Some(Ok(invalid)),
            confirms: 0,
            confirm_error: None,
        };
        let evidence = delete_owned_content(&mut fixture, |_| Ok(())).await;
        assert_eq!(
            evidence.verdict,
            DeleteCleanupVerdict::UnsupportedBeforeEffect
        );
        assert_eq!(fixture.confirms, 0);
        assert!(evidence.retry_is_safe());
    }

    #[test]
    fn incomplete_campaign_identity_never_authorizes_cleanup() {
        let identity = PublicCleanupIdentity::Comment {
            content_id: "741".to_owned(),
            attempt_id: String::new(),
            author: "@riviu_canary".to_owned(),
            text_sha256: hash('b'),
        };
        assert!(!identity.is_complete());
    }

    #[test]
    fn tagged_cleanup_identity_uses_camel_case_for_variant_fields() {
        let post = post_identity("741");
        let wire = serde_json::to_value(post).expect("serialize post identity");
        assert_eq!(wire["kind"], "post");
        assert_eq!(wire["contentId"], "741");
        assert!(wire.get("content_id").is_none());
        assert!(wire.get("captionSha256").is_some());

        let toggle = toggle_identity(PublicToggle::Save, "abc");
        let wire = serde_json::to_value(toggle).expect("serialize toggle identity");
        assert_eq!(wire["kind"], "toggle");
        assert!(wire.get("cardKey").is_some());
        assert!(wire.get("card_key").is_none());
    }
}
