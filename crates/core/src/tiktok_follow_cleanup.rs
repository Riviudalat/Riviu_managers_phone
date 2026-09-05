//! Measured, fail-closed proofs for reversing a Follow created by Nurture.
//!
//! The Following-list row shape is measured for `com.ss.android.ugc.trill` 38.3.2/en. This
//! module only binds a campaign-owned identity to that row. Production execution remains
//! disabled until the state after the unfollow tap has also been measured.

use crate::driver::HierarchySourceSnapshot;
use crate::tiktok_public_cleanup::{PublicCleanupIdentity, PublicToggle};
use crate::types::TapPoint;
use quick_xml::events::{BytesStart, Event};
use quick_xml::{Reader, XmlVersion};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

pub const MEASURED_FOLLOW_PACKAGE: &str = "com.ss.android.ugc.trill";
pub const MEASURED_FOLLOW_VERSION: &str = "38.3.2";
pub const MEASURED_FOLLOW_LOCALE: &str = "en";
const FOLLOWING_HANDLE_ID: &str = "com.ss.android.ugc.trill:id/ss9";
const FOLLOWING_HANDLE_CLASS: &str = "android.widget.TextView";
const FOLLOWING_RELATIONSHIP_ID: &str = "com.ss.android.ugc.trill:id/n_1";
const FOLLOWING_RELATIONSHIP_CLASS: &str = "android.widget.Button";
const FOLLOWING_ROW_CLASS: &str = "android.widget.LinearLayout";
const SOURCE_PROFILE_ID: &str = "com.ss.android.ugc.trill:id/t40";
const SOURCE_PROFILE_CLASS: &str = "android.widget.ImageView";
const SOURCE_FOLLOW_ID: &str = "com.ss.android.ugc.trill:id/fm1";
const SOURCE_FOLLOW_CLASS: &str = "android.widget.Button";
const SOURCE_CARD_ID: &str = "com.ss.android.ugc.trill:id/cv2";
const SOURCE_CARD_CLASS: &str = "android.widget.FrameLayout";
const SOURCE_RAIL_ID: &str = "com.ss.android.ugc.trill:id/hfp";
const SOURCE_RAIL_CLASS: &str = "android.widget.LinearLayout";
const SOURCE_PAGER_ID: &str = "com.ss.android.ugc.trill:id/tod";
const SOURCE_PAGER_CLASS: &str = "androidx.viewpager.widget.ViewPager";
const FEED_TAB_DESCRIPTION: &str = "For You";
const FEED_TAB_CLASS: &str = "android.widget.LinearLayout";
const MAX_HIERARCHY_BYTES: usize = 16 * 1024 * 1024;
const MAX_HIERARCHY_NODES: usize = 32_768;
const MAX_HIERARCHY_DEPTH: usize = 256;

#[derive(Debug, Clone)]
pub struct ProvedNurtureFollowSource {
    identity: NurtureFollowSourceIdentity,
    follow_tap_point: TapPoint,
    hierarchy_generation: u64,
    snapshot_sha256: String,
}

impl ProvedNurtureFollowSource {
    pub fn identity(&self) -> &NurtureFollowSourceIdentity {
        &self.identity
    }

    pub fn follow_tap_point(&self) -> TapPoint {
        self.follow_tap_point.clone()
    }

    pub fn hierarchy_generation(&self) -> u64 {
        self.hierarchy_generation
    }

    pub fn snapshot_sha256(&self) -> &str {
        &self.snapshot_sha256
    }

    pub fn into_parts(self) -> (NurtureFollowSourceIdentity, TapPoint) {
        (self.identity, self.follow_tap_point)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum FollowSourceProofError {
    #[error("the TikTok package/version/locale tuple is not measured")]
    UnmeasuredTuple,
    #[error("the hierarchy source is malformed")]
    InvalidHierarchy,
    #[error("the source card exposes no canonical @handle")]
    NonCanonicalAuthor,
    #[error("the exact measured source-card proof is missing")]
    Missing,
    #[error("more than one exact measured source-card proof matches")]
    Ambiguous,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NurtureFollowReadbackVerdict {
    FollowAbsent,
    FollowPresent,
    CardChanged,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NurtureFollowReadback {
    verdict: NurtureFollowReadbackVerdict,
    hierarchy_generation: u64,
    snapshot_sha256: String,
}

impl NurtureFollowReadback {
    pub fn verdict(&self) -> NurtureFollowReadbackVerdict {
        self.verdict
    }

    pub fn hierarchy_generation(&self) -> u64 {
        self.hierarchy_generation
    }

    pub fn snapshot_sha256(&self) -> &str {
        &self.snapshot_sha256
    }

    pub(crate) fn validate_confirmation(
        &self,
        source: &NurtureFollowSourceIdentity,
    ) -> anyhow::Result<()> {
        source.validate()?;
        anyhow::ensure!(
            self.verdict == NurtureFollowReadbackVerdict::FollowAbsent,
            "Nurture Follow confirmation requires an absent exact Follow control"
        );
        anyhow::ensure!(
            self.hierarchy_generation > source.author_profile_proof.hierarchy_generation,
            "Nurture Follow confirmation readback is stale"
        );
        anyhow::ensure!(
            valid_sha256(&self.snapshot_sha256),
            "Nurture Follow confirmation snapshot hash is invalid"
        );
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum FollowReadbackError {
    #[error("the TikTok package/version/locale tuple changed after Follow")]
    SourceTupleChanged,
    #[error("the persisted Follow source identity is invalid")]
    InvalidSourceIdentity,
    #[error("the Follow readback hierarchy is malformed")]
    InvalidHierarchy,
    #[error("the Follow readback hierarchy is not newer than the source proof")]
    StaleGeneration,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct FollowHierarchyTuple {
    pub(crate) package: String,
    pub(crate) version_name: String,
    pub(crate) locale: String,
}

impl FollowHierarchyTuple {
    pub(crate) fn measured() -> Self {
        Self {
            package: MEASURED_FOLLOW_PACKAGE.to_owned(),
            version_name: MEASURED_FOLLOW_VERSION.to_owned(),
            locale: MEASURED_FOLLOW_LOCALE.to_owned(),
        }
    }

    fn is_measured(&self) -> bool {
        self.package == MEASURED_FOLLOW_PACKAGE
            && self.version_name == MEASURED_FOLLOW_VERSION
            && self.locale == MEASURED_FOLLOW_LOCALE
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FollowHierarchyBounds {
    pub(crate) left: i32,
    pub(crate) top: i32,
    pub(crate) right: i32,
    pub(crate) bottom: i32,
}

impl FollowHierarchyBounds {
    fn valid(self) -> bool {
        self.left >= 0 && self.top >= 0 && self.right > self.left && self.bottom > self.top
    }

    fn centre(self) -> TapPoint {
        TapPoint {
            x: (f64::from(self.left) + f64::from(self.right)) / 2.0,
            y: (f64::from(self.top) + f64::from(self.bottom)) / 2.0,
        }
    }

    fn vertical_overlap(self, other: Self) -> i32 {
        self.bottom.min(other.bottom) - self.top.max(other.top)
    }

    fn contains(self, other: Self) -> bool {
        self.left <= other.left
            && self.top <= other.top
            && self.right >= other.right
            && self.bottom >= other.bottom
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FollowHierarchyNode {
    /// Index of the parent in the same snapshot. A missing parent marks the document root.
    pub(crate) parent: Option<usize>,
    pub(crate) package_name: String,
    pub(crate) class_name: String,
    pub(crate) resource_id: String,
    pub(crate) text: String,
    pub(crate) content_description: String,
    pub(crate) bounds: FollowHierarchyBounds,
    pub(crate) enabled: bool,
    pub(crate) clickable: bool,
    pub(crate) selected: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct FollowCardContinuityNode {
    package_name: String,
    class_name: String,
    resource_id: String,
    text: String,
    content_description: String,
    bounds: FollowHierarchyBounds,
    enabled: bool,
    clickable: bool,
    selected: bool,
    children: Vec<FollowCardContinuityNode>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FollowHierarchySnapshot {
    pub(crate) generation: u64,
    pub(crate) tuple: FollowHierarchyTuple,
    pub(crate) nodes: Vec<FollowHierarchyNode>,
}

impl FollowHierarchySnapshot {
    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.generation > 0,
            "Follow hierarchy generation is missing"
        );
        anyhow::ensure!(
            self.tuple.is_measured(),
            "Follow hierarchy tuple is unmeasured"
        );
        anyhow::ensure!(!self.nodes.is_empty(), "Follow hierarchy is empty");
        anyhow::ensure!(
            self.nodes
                .iter()
                .filter(|node| node.parent.is_none())
                .count()
                == 1,
            "Follow hierarchy must have one document root"
        );
        for (index, node) in self.nodes.iter().enumerate() {
            if let Some(parent) = node.parent {
                anyhow::ensure!(
                    parent < self.nodes.len(),
                    "Follow hierarchy parent is missing"
                );
                anyhow::ensure!(parent != index, "Follow hierarchy node parents itself");
            }
            ancestry(self, index).ok_or_else(|| anyhow::anyhow!("Follow hierarchy has a cycle"))?;
        }
        Ok(())
    }

    pub fn sha256(&self) -> anyhow::Result<String> {
        self.validate()?;
        Ok(hex_sha256(&serde_json::to_vec(self)?))
    }

    fn card_continuity_node(
        &self,
        index: usize,
        excluded_follow_description: &str,
    ) -> anyhow::Result<FollowCardContinuityNode> {
        let node = self
            .nodes
            .get(index)
            .ok_or_else(|| anyhow::anyhow!("Follow continuity node is missing"))?;
        let children = self
            .nodes
            .iter()
            .enumerate()
            .filter(|(_, child)| child.parent == Some(index))
            .filter(|(_, child)| {
                !(child.package_name == MEASURED_FOLLOW_PACKAGE
                    && child.resource_id == SOURCE_FOLLOW_ID
                    && child.class_name == SOURCE_FOLLOW_CLASS
                    && child.text.is_empty()
                    && child.content_description == excluded_follow_description)
            })
            .map(|(child_index, _)| {
                self.card_continuity_node(child_index, excluded_follow_description)
            })
            .collect::<anyhow::Result<Vec<_>>>()?;
        Ok(FollowCardContinuityNode {
            package_name: node.package_name.clone(),
            class_name: node.class_name.clone(),
            resource_id: node.resource_id.clone(),
            text: node.text.clone(),
            content_description: node.content_description.clone(),
            bounds: node.bounds,
            enabled: node.enabled,
            clickable: node.clickable,
            selected: node.selected,
            children,
        })
    }

    fn card_continuity_key(
        &self,
        card_node_index: usize,
        excluded_follow_description: &str,
    ) -> anyhow::Result<String> {
        self.validate()?;
        let card = self.card_continuity_node(card_node_index, excluded_follow_description)?;
        Ok(hex_sha256(&serde_json::to_vec(&card)?))
    }

    /// Build the immutable author witness from the exact source-card snapshot.
    pub fn author_profile_proof(
        &self,
        profile_node_index: usize,
    ) -> anyhow::Result<FollowAuthorProfileProof> {
        self.validate()?;
        let profile = self
            .nodes
            .get(profile_node_index)
            .ok_or_else(|| anyhow::anyhow!("author profile node is missing"))?;
        let canonical_handle = canonical_profile_description(&profile.content_description)
            .ok_or_else(|| anyhow::anyhow!("author profile node has no exact canonical handle"))?;
        anyhow::ensure!(
            profile.package_name == MEASURED_FOLLOW_PACKAGE
                && profile.resource_id == SOURCE_PROFILE_ID
                && profile.class_name == SOURCE_PROFILE_CLASS
                && profile.text.is_empty()
                && profile.enabled
                && profile.clickable
                && profile.bounds.valid(),
            "author profile node does not match the measured control"
        );
        let parent_chain = ancestry(self, profile_node_index)
            .ok_or_else(|| anyhow::anyhow!("author profile ancestry is invalid"))?;
        anyhow::ensure!(parent_chain.len() >= 3, "author profile node is unbound");
        let card_node_index = parent_chain[0];
        let rail_node_index = parent_chain[1];
        let pager_node_index = parent_chain[2];
        ensure_exact_ancestor(
            self,
            card_node_index,
            SOURCE_CARD_ID,
            SOURCE_CARD_CLASS,
            true,
            "source card",
        )?;
        ensure_exact_ancestor(
            self,
            rail_node_index,
            SOURCE_RAIL_ID,
            SOURCE_RAIL_CLASS,
            true,
            "source rail",
        )?;
        ensure_exact_ancestor(
            self,
            pager_node_index,
            SOURCE_PAGER_ID,
            SOURCE_PAGER_CLASS,
            false,
            "source pager",
        )?;
        anyhow::ensure!(
            self.nodes[card_node_index].parent == Some(rail_node_index)
                && self.nodes[rail_node_index].parent == Some(pager_node_index)
                && self.nodes[card_node_index].bounds.contains(profile.bounds)
                && self.nodes[rail_node_index]
                    .bounds
                    .contains(self.nodes[card_node_index].bounds)
                && self.nodes[pager_node_index]
                    .bounds
                    .contains(self.nodes[rail_node_index].bounds),
            "author profile ancestry skips the measured source-card chain"
        );
        let mut follow_nodes = self.nodes.iter().enumerate().filter(|(_, node)| {
            node.parent == Some(card_node_index)
                && node.package_name == MEASURED_FOLLOW_PACKAGE
                && node.resource_id == SOURCE_FOLLOW_ID
                && node.class_name == SOURCE_FOLLOW_CLASS
                && node.text.is_empty()
                && node.content_description == format!("Follow {canonical_handle}")
                && node.enabled
                && node.clickable
                && self.nodes[card_node_index].bounds.contains(node.bounds)
        });
        let (follow_node_index, follow) = follow_nodes
            .next()
            .ok_or_else(|| anyhow::anyhow!("exact Follow sibling is missing"))?;
        anyhow::ensure!(
            follow_nodes.next().is_none(),
            "exact Follow sibling is ambiguous"
        );
        let mut feed_tabs = self.nodes.iter().enumerate().filter(|(_, node)| {
            node.package_name == MEASURED_FOLLOW_PACKAGE
                && node.class_name == FEED_TAB_CLASS
                && node.resource_id.is_empty()
                && node.text.is_empty()
                && node.content_description == FEED_TAB_DESCRIPTION
                && node.enabled
                && !node.clickable
                && node.selected
                && node.bounds.valid()
        });
        let (feed_tab_node_index, _) = feed_tabs
            .next()
            .ok_or_else(|| anyhow::anyhow!("selected For You feed tab is missing"))?;
        anyhow::ensure!(
            feed_tabs.next().is_none(),
            "selected For You feed tab is ambiguous"
        );
        let profile_root = parent_chain
            .last()
            .copied()
            .ok_or_else(|| anyhow::anyhow!("source card has no document root"))?;
        let feed_root = ancestry(self, feed_tab_node_index)
            .and_then(|chain| chain.last().copied())
            .ok_or_else(|| anyhow::anyhow!("selected For You tab has no document root"))?;
        anyhow::ensure!(
            profile_root == feed_root,
            "selected For You tab belongs to another hierarchy root"
        );
        Ok(FollowAuthorProfileProof {
            tuple: self.tuple.clone(),
            hierarchy_generation: self.generation,
            snapshot_sha256: self.sha256()?,
            profile_node_index,
            profile_resource_id: profile.resource_id.clone(),
            profile_class_name: profile.class_name.clone(),
            profile_content_description: profile.content_description.clone(),
            profile_enabled: profile.enabled,
            profile_clickable: profile.clickable,
            follow_node_index,
            follow_resource_id: follow.resource_id.clone(),
            follow_class_name: follow.class_name.clone(),
            follow_content_description: follow.content_description.clone(),
            follow_enabled: follow.enabled,
            follow_clickable: follow.clickable,
            card_node_index,
            card_resource_id: self.nodes[card_node_index].resource_id.clone(),
            card_class_name: self.nodes[card_node_index].class_name.clone(),
            card_enabled: self.nodes[card_node_index].enabled,
            card_clickable: self.nodes[card_node_index].clickable,
            rail_node_index,
            rail_resource_id: self.nodes[rail_node_index].resource_id.clone(),
            rail_class_name: self.nodes[rail_node_index].class_name.clone(),
            rail_enabled: self.nodes[rail_node_index].enabled,
            rail_clickable: self.nodes[rail_node_index].clickable,
            pager_node_index,
            pager_resource_id: self.nodes[pager_node_index].resource_id.clone(),
            pager_class_name: self.nodes[pager_node_index].class_name.clone(),
            pager_enabled: self.nodes[pager_node_index].enabled,
            pager_clickable: self.nodes[pager_node_index].clickable,
            feed_tab_node_index,
            feed_tab_class_name: self.nodes[feed_tab_node_index].class_name.clone(),
            feed_tab_content_description: self.nodes[feed_tab_node_index]
                .content_description
                .clone(),
            feed_tab_enabled: self.nodes[feed_tab_node_index].enabled,
            feed_tab_clickable: self.nodes[feed_tab_node_index].clickable,
            feed_tab_selected: self.nodes[feed_tab_node_index].selected,
            parent_chain,
            canonical_handle,
            card_continuity_key: self
                .card_continuity_key(card_node_index, &follow.content_description)?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct FollowAuthorProfileProof {
    pub(crate) tuple: FollowHierarchyTuple,
    pub(crate) hierarchy_generation: u64,
    pub(crate) snapshot_sha256: String,
    pub(crate) profile_node_index: usize,
    pub(crate) profile_resource_id: String,
    pub(crate) profile_class_name: String,
    pub(crate) profile_content_description: String,
    pub(crate) profile_enabled: bool,
    pub(crate) profile_clickable: bool,
    pub(crate) follow_node_index: usize,
    pub(crate) follow_resource_id: String,
    pub(crate) follow_class_name: String,
    pub(crate) follow_content_description: String,
    pub(crate) follow_enabled: bool,
    pub(crate) follow_clickable: bool,
    pub(crate) card_node_index: usize,
    pub(crate) card_resource_id: String,
    pub(crate) card_class_name: String,
    pub(crate) card_enabled: bool,
    pub(crate) card_clickable: bool,
    pub(crate) rail_node_index: usize,
    pub(crate) rail_resource_id: String,
    pub(crate) rail_class_name: String,
    pub(crate) rail_enabled: bool,
    pub(crate) rail_clickable: bool,
    pub(crate) pager_node_index: usize,
    pub(crate) pager_resource_id: String,
    pub(crate) pager_class_name: String,
    pub(crate) pager_enabled: bool,
    pub(crate) pager_clickable: bool,
    pub(crate) feed_tab_node_index: usize,
    pub(crate) feed_tab_class_name: String,
    pub(crate) feed_tab_content_description: String,
    pub(crate) feed_tab_enabled: bool,
    pub(crate) feed_tab_clickable: bool,
    pub(crate) feed_tab_selected: bool,
    pub(crate) parent_chain: Vec<usize>,
    pub(crate) canonical_handle: String,
    pub(crate) card_continuity_key: String,
}

impl FollowAuthorProfileProof {
    fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.tuple.is_measured(),
            "author profile tuple is unmeasured"
        );
        anyhow::ensure!(
            self.hierarchy_generation > 0,
            "author profile generation is missing"
        );
        anyhow::ensure!(
            valid_sha256(&self.snapshot_sha256),
            "author snapshot key is invalid"
        );
        anyhow::ensure!(
            self.profile_resource_id == SOURCE_PROFILE_ID
                && self.profile_class_name == SOURCE_PROFILE_CLASS
                && self.profile_content_description == format!("{} profile", self.canonical_handle)
                && self.profile_enabled
                && self.profile_clickable,
            "author profile witness is not the measured control"
        );
        anyhow::ensure!(
            self.follow_resource_id == SOURCE_FOLLOW_ID
                && self.follow_class_name == SOURCE_FOLLOW_CLASS
                && self.follow_content_description == format!("Follow {}", self.canonical_handle)
                && self.follow_enabled
                && self.follow_clickable,
            "Follow witness is not the measured sibling control"
        );
        anyhow::ensure!(
            self.parent_chain.len() >= 3
                && self.parent_chain[0] == self.card_node_index
                && self.parent_chain[1] == self.rail_node_index
                && self.parent_chain[2] == self.pager_node_index
                && self.profile_node_index != self.follow_node_index
                && self.feed_tab_node_index != self.profile_node_index
                && self.feed_tab_node_index != self.follow_node_index,
            "author profile parent path is not the measured card/rail/pager chain"
        );
        anyhow::ensure!(
            self.card_resource_id == SOURCE_CARD_ID
                && self.card_class_name == SOURCE_CARD_CLASS
                && self.card_enabled
                && self.card_clickable
                && self.rail_resource_id == SOURCE_RAIL_ID
                && self.rail_class_name == SOURCE_RAIL_CLASS
                && self.rail_enabled
                && self.rail_clickable
                && self.pager_resource_id == SOURCE_PAGER_ID
                && self.pager_class_name == SOURCE_PAGER_CLASS
                && self.pager_enabled
                && !self.pager_clickable,
            "author profile ancestors are not the measured card/rail/pager controls"
        );
        anyhow::ensure!(
            self.feed_tab_class_name == FEED_TAB_CLASS
                && self.feed_tab_content_description == FEED_TAB_DESCRIPTION
                && self.feed_tab_enabled
                && !self.feed_tab_clickable
                && self.feed_tab_selected,
            "author profile witness is not bound to the selected For You feed"
        );
        anyhow::ensure!(
            canonical_handle(&self.canonical_handle).as_deref()
                == Some(self.canonical_handle.as_str()),
            "author profile proof has no canonical handle"
        );
        anyhow::ensure!(
            valid_sha256(&self.card_continuity_key),
            "Follow card continuity key is invalid"
        );
        Ok(())
    }

    fn key(&self) -> anyhow::Result<String> {
        self.validate()?;
        Ok(hex_sha256(&serde_json::to_vec(self)?))
    }

    fn card_key(&self) -> anyhow::Result<String> {
        self.validate()?;
        Ok(self.card_continuity_key.clone())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NurtureFollowSourceIdentity {
    /// Exact canonical TikTok handle, including `@`, normalized to lowercase.
    pub(crate) canonical_handle: String,
    /// SHA-256 of the card identity proved immediately before the original Follow.
    pub(crate) card_key: String,
    /// SHA-256 of the author-profile identity read from that card/profile transition.
    pub(crate) author_profile_key: String,
    /// Full witness whose canonical serialization produces `author_profile_key`.
    pub(crate) author_profile_proof: FollowAuthorProfileProof,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PersistedNurtureFollowSourceIdentity {
    canonical_handle: String,
    card_key: String,
    author_profile_key: String,
    author_profile_proof: FollowAuthorProfileProof,
}

impl NurtureFollowSourceIdentity {
    pub(crate) fn from_profile_snapshot(
        card_key: String,
        snapshot: &FollowHierarchySnapshot,
        profile_node_index: usize,
    ) -> anyhow::Result<Self> {
        anyhow::ensure!(valid_sha256(&card_key), "Follow card key is invalid");
        let author_profile_proof = snapshot.author_profile_proof(profile_node_index)?;
        anyhow::ensure!(
            card_key == author_profile_proof.card_key()?,
            "Follow card key is not bound to the source-card ancestry"
        );
        let canonical_handle = author_profile_proof.canonical_handle.clone();
        let author_profile_key = author_profile_proof.key()?;
        Ok(Self {
            canonical_handle,
            card_key,
            author_profile_key,
            author_profile_proof,
        })
    }

    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            canonical_handle(&self.canonical_handle).as_deref()
                == Some(self.canonical_handle.as_str()),
            "Follow source requires a lowercase canonical @handle"
        );
        anyhow::ensure!(valid_sha256(&self.card_key), "Follow card key is invalid");
        self.author_profile_proof.validate()?;
        anyhow::ensure!(
            self.author_profile_proof.canonical_handle == self.canonical_handle,
            "Follow author-profile witness belongs to another handle"
        );
        anyhow::ensure!(
            self.author_profile_key == self.author_profile_proof.key()?,
            "Follow author-profile key is not bound to its witness"
        );
        anyhow::ensure!(
            self.card_key == self.author_profile_proof.card_key()?,
            "Follow card key is not bound to its source-card witness"
        );
        Ok(())
    }

    pub fn canonical_handle(&self) -> &str {
        &self.canonical_handle
    }

    pub fn card_key(&self) -> &str {
        &self.card_key
    }

    pub fn author_profile_key(&self) -> &str {
        &self.author_profile_key
    }

    pub(crate) fn from_persisted_json(raw: &str) -> anyhow::Result<Self> {
        let wire = serde_json::from_str::<PersistedNurtureFollowSourceIdentity>(raw)
            .map_err(|error| anyhow::anyhow!("source Follow identity is weak: {error}"))?;
        let identity = Self {
            canonical_handle: wire.canonical_handle,
            card_key: wire.card_key,
            author_profile_key: wire.author_profile_key,
            author_profile_proof: wire.author_profile_proof,
        };
        identity.validate()?;
        Ok(identity)
    }
}

/// Parse one immutable hierarchy read and bind the exact author-profile and Follow controls.
///
/// The returned tap point and identity are derived from the same XML generation. Callers cannot
/// provide independently located elements or manufacture the persisted proof structure.
pub fn prove_nurture_follow_source(
    package: &str,
    version_name: &str,
    locale: &str,
    source: &HierarchySourceSnapshot,
) -> Result<ProvedNurtureFollowSource, FollowSourceProofError> {
    if package != MEASURED_FOLLOW_PACKAGE
        || version_name != MEASURED_FOLLOW_VERSION
        || locale != MEASURED_FOLLOW_LOCALE
    {
        return Err(FollowSourceProofError::UnmeasuredTuple);
    }
    if source.generation == 0
        || source.xml.trim().is_empty()
        || source.xml.len() > MAX_HIERARCHY_BYTES
    {
        return Err(FollowSourceProofError::InvalidHierarchy);
    }
    let snapshot = parse_source_hierarchy(source)?;
    snapshot
        .validate()
        .map_err(|_| FollowSourceProofError::InvalidHierarchy)?;

    let profile_indices: Vec<usize> = snapshot
        .nodes
        .iter()
        .enumerate()
        .filter(|(_, node)| {
            node.package_name == MEASURED_FOLLOW_PACKAGE
                && node.resource_id == SOURCE_PROFILE_ID
                && node.class_name == SOURCE_PROFILE_CLASS
        })
        .map(|(index, _)| index)
        .collect();
    if profile_indices.iter().any(|index| {
        canonical_profile_description(&snapshot.nodes[*index].content_description).is_none()
    }) {
        return Err(FollowSourceProofError::NonCanonicalAuthor);
    }

    let mut proofs = profile_indices
        .into_iter()
        .filter_map(|index| snapshot.author_profile_proof(index).ok());
    let proof = proofs.next().ok_or(FollowSourceProofError::Missing)?;
    if proofs.next().is_some() {
        return Err(FollowSourceProofError::Ambiguous);
    }
    let follow_tap_point = snapshot
        .nodes
        .get(proof.follow_node_index)
        .ok_or(FollowSourceProofError::InvalidHierarchy)?
        .bounds
        .centre();
    let card_key = proof
        .card_key()
        .map_err(|_| FollowSourceProofError::InvalidHierarchy)?;
    let identity = NurtureFollowSourceIdentity::from_profile_snapshot(
        card_key,
        &snapshot,
        proof.profile_node_index,
    )
    .map_err(|_| FollowSourceProofError::InvalidHierarchy)?;
    Ok(ProvedNurtureFollowSource {
        identity,
        follow_tap_point,
        hierarchy_generation: source.generation,
        snapshot_sha256: snapshot
            .sha256()
            .map_err(|_| FollowSourceProofError::InvalidHierarchy)?,
    })
}

/// Classify one fresh hierarchy read after a Follow tap.
///
/// Card continuity, author continuity, selected feed state, and disappearance of the exact
/// Follow control are derived from this one snapshot. A caller cannot combine a missing Follow
/// result from one tree with an author result from another tree.
pub fn readback_nurture_follow_source(
    source: &NurtureFollowSourceIdentity,
    package: &str,
    version_name: &str,
    locale: &str,
    hierarchy: &HierarchySourceSnapshot,
) -> Result<NurtureFollowReadback, FollowReadbackError> {
    source
        .validate()
        .map_err(|_| FollowReadbackError::InvalidSourceIdentity)?;
    if package != MEASURED_FOLLOW_PACKAGE
        || version_name != MEASURED_FOLLOW_VERSION
        || locale != MEASURED_FOLLOW_LOCALE
        || source.author_profile_proof.tuple.package != package
        || source.author_profile_proof.tuple.version_name != version_name
        || source.author_profile_proof.tuple.locale != locale
    {
        return Err(FollowReadbackError::SourceTupleChanged);
    }
    if hierarchy.generation <= source.author_profile_proof.hierarchy_generation {
        return Err(FollowReadbackError::StaleGeneration);
    }
    let snapshot =
        parse_source_hierarchy(hierarchy).map_err(|_| FollowReadbackError::InvalidHierarchy)?;
    snapshot
        .validate()
        .map_err(|_| FollowReadbackError::InvalidHierarchy)?;
    let snapshot_sha256 = snapshot
        .sha256()
        .map_err(|_| FollowReadbackError::InvalidHierarchy)?;
    let proof = &source.author_profile_proof;

    let matching_profiles = snapshot
        .nodes
        .iter()
        .enumerate()
        .filter(|(_, node)| {
            node.package_name == MEASURED_FOLLOW_PACKAGE
                && node.resource_id == proof.profile_resource_id
                && node.class_name == proof.profile_class_name
                && node.text.is_empty()
                && node.content_description == proof.profile_content_description
                && node.enabled
                && node.clickable
                && node.bounds.valid()
        })
        .map(|(index, _)| index)
        .collect::<Vec<_>>();

    let verdict = if matching_profiles.len() != 1 {
        NurtureFollowReadbackVerdict::CardChanged
    } else {
        classify_follow_readback(&snapshot, source, matching_profiles[0])
    };
    Ok(NurtureFollowReadback {
        verdict,
        hierarchy_generation: hierarchy.generation,
        snapshot_sha256,
    })
}

fn classify_follow_readback(
    snapshot: &FollowHierarchySnapshot,
    source: &NurtureFollowSourceIdentity,
    profile_node_index: usize,
) -> NurtureFollowReadbackVerdict {
    let Some(parent_chain) = ancestry(snapshot, profile_node_index) else {
        return NurtureFollowReadbackVerdict::CardChanged;
    };
    if parent_chain.len() < 3 {
        return NurtureFollowReadbackVerdict::CardChanged;
    }
    let card_node_index = parent_chain[0];
    let rail_node_index = parent_chain[1];
    let pager_node_index = parent_chain[2];
    let exact_chain = ensure_exact_ancestor(
        snapshot,
        card_node_index,
        SOURCE_CARD_ID,
        SOURCE_CARD_CLASS,
        true,
        "readback card",
    )
    .and_then(|_| {
        ensure_exact_ancestor(
            snapshot,
            rail_node_index,
            SOURCE_RAIL_ID,
            SOURCE_RAIL_CLASS,
            true,
            "readback rail",
        )
    })
    .and_then(|_| {
        ensure_exact_ancestor(
            snapshot,
            pager_node_index,
            SOURCE_PAGER_ID,
            SOURCE_PAGER_CLASS,
            false,
            "readback pager",
        )
    });
    if exact_chain.is_err()
        || snapshot.nodes[card_node_index].parent != Some(rail_node_index)
        || snapshot.nodes[rail_node_index].parent != Some(pager_node_index)
        || !snapshot.nodes[card_node_index]
            .bounds
            .contains(snapshot.nodes[profile_node_index].bounds)
        || !snapshot.nodes[rail_node_index]
            .bounds
            .contains(snapshot.nodes[card_node_index].bounds)
        || !snapshot.nodes[pager_node_index]
            .bounds
            .contains(snapshot.nodes[rail_node_index].bounds)
    {
        return NurtureFollowReadbackVerdict::CardChanged;
    }

    let profile_root = parent_chain.last().copied();
    let selected_feed_roots = snapshot
        .nodes
        .iter()
        .enumerate()
        .filter(|(_, node)| {
            node.package_name == MEASURED_FOLLOW_PACKAGE
                && node.class_name == FEED_TAB_CLASS
                && node.resource_id.is_empty()
                && node.text.is_empty()
                && node.content_description == FEED_TAB_DESCRIPTION
                && node.enabled
                && !node.clickable
                && node.selected
                && node.bounds.valid()
        })
        .filter_map(|(index, _)| ancestry(snapshot, index)?.last().copied())
        .collect::<Vec<_>>();
    if selected_feed_roots.len() != 1 || selected_feed_roots.first().copied() != profile_root {
        return NurtureFollowReadbackVerdict::CardChanged;
    }

    let continuity_key = snapshot.card_continuity_key(
        card_node_index,
        &source.author_profile_proof.follow_content_description,
    );
    if !matches!(continuity_key, Ok(ref key) if key == source.card_key()) {
        return NurtureFollowReadbackVerdict::CardChanged;
    }

    let follow_nodes = snapshot
        .nodes
        .iter()
        .filter(|node| {
            node.parent == Some(card_node_index)
                && node.package_name == MEASURED_FOLLOW_PACKAGE
                && node.resource_id == SOURCE_FOLLOW_ID
        })
        .collect::<Vec<_>>();
    if follow_nodes.is_empty() {
        return NurtureFollowReadbackVerdict::FollowAbsent;
    }
    if follow_nodes.len() == 1 {
        let follow = follow_nodes[0];
        if follow.class_name == source.author_profile_proof.follow_class_name
            && follow.text.is_empty()
            && follow.content_description == source.author_profile_proof.follow_content_description
            && follow.enabled
            && follow.clickable
            && snapshot.nodes[card_node_index]
                .bounds
                .contains(follow.bounds)
        {
            return NurtureFollowReadbackVerdict::FollowPresent;
        }
    }
    NurtureFollowReadbackVerdict::CardChanged
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfirmedNurtureFollowSource {
    pub action_run_id: String,
    pub owner_id: String,
    pub device_udid: String,
    pub identity: NurtureFollowSourceIdentity,
    pub readback_hierarchy_generation: u64,
    pub readback_snapshot_sha256: String,
    pub confirmed_at: String,
}

/// The durable phase of an effect that could have happened before its worker disappeared.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum NurtureFollowPossibleEffectState {
    /// The effect intent was persisted and the process may disappear before settlement.
    Armed,
    /// Restart/session recovery quarantined the armed row because the effect is unknowable.
    Uncertain,
}

/// Immutable provenance for a Follow which may have reached TikTok.
///
/// This is deliberately distinct from [`ConfirmedNurtureFollowSource`]. It authorizes a later
/// cleanup preflight to inspect the exact source identity, but does not claim the Follow happened
/// and does not make cleanup executable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PossibleNurtureFollowSource {
    pub action_run_id: String,
    pub owner_id: String,
    pub device_udid: String,
    pub identity: NurtureFollowSourceIdentity,
    pub state: NurtureFollowPossibleEffectState,
    /// Timestamp captured in the immutable arm witness.
    pub armed_at: String,
    /// Present only after recovery/runtime settlement moved the row to `uncertain`.
    pub uncertain_at: Option<String>,
}

/// The only two provenance classes that can reach a future Follow cleanup preflight.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "confidence", content = "source", rename_all = "camelCase")]
pub enum NurtureFollowCleanupOrigin {
    Confirmed(ConfirmedNurtureFollowSource),
    PossibleEffect(PossibleNurtureFollowSource),
}

/// Counts from one idempotent orphan-action recovery transaction.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct NurtureFollowRecovery {
    pub failed_before_effect: usize,
    pub uncertain: usize,
    /// Armed/uncertain witnesses still requiring conservative cleanup visibility in this scope.
    pub possible_effects: usize,
}

impl NurtureFollowRecovery {
    pub fn is_empty(self) -> bool {
        self.failed_before_effect == 0 && self.uncertain == 0
    }

    pub fn has_possible_effect(self) -> bool {
        self.possible_effects > 0
    }
}

/// Stable prefix shared by the Nurture runtime and its recovery queries.
pub(crate) fn nurture_follow_owner_prefix(run_id: Uuid) -> String {
    format!("nurture-run:{run_id}:follow:")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FollowRelationshipProof {
    Following,
    Friends,
}

#[derive(Debug, Clone)]
pub struct BoundFollowingRow {
    pub source: NurtureFollowSourceIdentity,
    pub cleanup_identity: PublicCleanupIdentity,
    pub relationship: FollowRelationshipProof,
    pub tap_point: TapPoint,
    pub hierarchy_generation: u64,
    pub snapshot_sha256: String,
    pub row_node_index: usize,
    pub handle_node_index: usize,
    pub relationship_node_index: usize,
    pub row_proof_key: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FollowRowRefusal {
    InvalidSourceIdentity,
    InvalidSnapshot,
    SourceTupleChanged,
    ExpectedHandleMissing,
    DuplicateExpectedHandle,
    RelationshipMissing,
    DuplicateRelationship,
    UnexpectedRelationship,
    InvalidGeometry,
    ControlNotActionable,
}

/// Bind one exact Following-list handle row to its measured relationship control.
///
/// The handle (`:id/ss9`) and relationship (`:id/n_1`) must come from the same immutable
/// snapshot and have a non-root common ancestor. Geometry is only a corroborating signal;
/// it can never join nodes from independent queries or adjacent rows.
pub fn prove_following_cleanup_row(
    source: &NurtureFollowSourceIdentity,
    package: &str,
    version_name: &str,
    locale: &str,
    hierarchy: &HierarchySourceSnapshot,
) -> Result<BoundFollowingRow, FollowRowRefusal> {
    if package != MEASURED_FOLLOW_PACKAGE
        || version_name != MEASURED_FOLLOW_VERSION
        || locale != MEASURED_FOLLOW_LOCALE
    {
        return Err(FollowRowRefusal::SourceTupleChanged);
    }
    let snapshot =
        parse_source_hierarchy(hierarchy).map_err(|_| FollowRowRefusal::InvalidSnapshot)?;
    bind_following_row(source, &snapshot)
}

pub(crate) fn bind_following_row(
    source: &NurtureFollowSourceIdentity,
    snapshot: &FollowHierarchySnapshot,
) -> Result<BoundFollowingRow, FollowRowRefusal> {
    source
        .validate()
        .map_err(|_| FollowRowRefusal::InvalidSourceIdentity)?;
    snapshot
        .validate()
        .map_err(|_| FollowRowRefusal::InvalidSnapshot)?;
    if snapshot.tuple != source.author_profile_proof.tuple {
        return Err(FollowRowRefusal::SourceTupleChanged);
    }

    let mut matching_handles = snapshot.nodes.iter().enumerate().filter(|(_, node)| {
        node.package_name == MEASURED_FOLLOW_PACKAGE
            && node.resource_id == FOLLOWING_HANDLE_ID
            && node.class_name == FOLLOWING_HANDLE_CLASS
            && node.content_description.is_empty()
            && node.enabled
            && !node.clickable
            && !node.selected
            && canonical_handle(&node.text).as_deref() == Some(source.canonical_handle.as_str())
    });
    let (handle_index, handle) = matching_handles
        .next()
        .ok_or(FollowRowRefusal::ExpectedHandleMissing)?;
    if matching_handles.next().is_some() {
        return Err(FollowRowRefusal::DuplicateExpectedHandle);
    }
    if !handle.bounds.valid() {
        return Err(FollowRowRefusal::InvalidGeometry);
    }

    let mut same_row = snapshot
        .nodes
        .iter()
        .enumerate()
        .filter_map(|(control_index, control)| {
            if control.package_name != MEASURED_FOLLOW_PACKAGE
                || control.resource_id != FOLLOWING_RELATIONSHIP_ID
                || control.class_name != FOLLOWING_RELATIONSHIP_CLASS
                || !control.content_description.is_empty()
                || control.selected
            {
                return None;
            }
            let row = measured_following_row(snapshot, handle_index, control_index)?;
            let enough_overlap = handle.bounds.vertical_overlap(control.bounds)
                >= (handle.bounds.bottom - handle.bounds.top)
                    .min(control.bounds.bottom - control.bounds.top)
                    / 4;
            (control.bounds.valid() && control.bounds.left >= handle.bounds.right && enough_overlap)
                .then_some((control_index, control, row))
        });
    let (relationship_node_index, control, row_node_index) = same_row
        .next()
        .ok_or(FollowRowRefusal::RelationshipMissing)?;
    if same_row.next().is_some() {
        return Err(FollowRowRefusal::DuplicateRelationship);
    }
    let relationship = match control.text.trim() {
        "Following" => FollowRelationshipProof::Following,
        "Friends" => FollowRelationshipProof::Friends,
        _ => return Err(FollowRowRefusal::UnexpectedRelationship),
    };
    if !control.enabled || !control.clickable {
        return Err(FollowRowRefusal::ControlNotActionable);
    }
    let snapshot_sha256 = snapshot
        .sha256()
        .map_err(|_| FollowRowRefusal::InvalidSnapshot)?;
    let row_proof_key = hex_sha256(
        format!(
            "{}\n{}\n{}\n{}\n{}\n{}",
            source.author_profile_key,
            snapshot_sha256,
            snapshot.generation,
            row_node_index,
            handle_index,
            relationship_node_index
        )
        .as_bytes(),
    );

    Ok(BoundFollowingRow {
        source: source.clone(),
        cleanup_identity: PublicCleanupIdentity::Toggle {
            card_key: source.card_key.clone(),
            author: source.canonical_handle.clone(),
            effect: PublicToggle::Follow,
        },
        relationship,
        tap_point: control.bounds.centre(),
        hierarchy_generation: snapshot.generation,
        snapshot_sha256,
        row_node_index,
        handle_node_index: handle_index,
        relationship_node_index,
        row_proof_key,
    })
}

impl BoundFollowingRow {
    pub fn is_fresh_reproof_of(&self, earlier: &Self) -> bool {
        self.source == earlier.source
            && self.hierarchy_generation > earlier.hierarchy_generation
            && self.snapshot_sha256 != earlier.snapshot_sha256
            && self.row_proof_key != earlier.row_proof_key
    }
}

fn ensure_exact_ancestor(
    snapshot: &FollowHierarchySnapshot,
    index: usize,
    resource_id: &str,
    class_name: &str,
    clickable: bool,
    label: &str,
) -> anyhow::Result<()> {
    let node = snapshot
        .nodes
        .get(index)
        .ok_or_else(|| anyhow::anyhow!("{label} node is missing"))?;
    anyhow::ensure!(
        node.package_name == MEASURED_FOLLOW_PACKAGE
            && node.resource_id == resource_id
            && node.class_name == class_name
            && node.text.is_empty()
            && node.content_description.is_empty()
            && node.enabled
            && node.clickable == clickable
            && node.bounds.valid(),
        "{label} does not match the measured source-card ancestor"
    );
    Ok(())
}

fn canonical_profile_description(value: &str) -> Option<String> {
    let raw = value.strip_suffix(" profile")?;
    let canonical = canonical_handle(raw)?;
    (raw.starts_with('@') && raw == canonical).then_some(canonical)
}

fn xml_attribute(
    reader: &Reader<&[u8]>,
    start: &BytesStart<'_>,
    name: &[u8],
) -> Result<Option<String>, FollowSourceProofError> {
    for attribute in start.attributes().with_checks(true) {
        let attribute = attribute.map_err(|_| FollowSourceProofError::InvalidHierarchy)?;
        if attribute.key.as_ref() == name {
            return attribute
                .decoded_and_normalized_value(XmlVersion::Implicit1_0, reader.decoder())
                .map(|value| Some(value.into_owned()))
                .map_err(|_| FollowSourceProofError::InvalidHierarchy);
        }
    }
    Ok(None)
}

fn xml_bool(
    reader: &Reader<&[u8]>,
    start: &BytesStart<'_>,
    name: &[u8],
) -> Result<bool, FollowSourceProofError> {
    match xml_attribute(reader, start, name)?.as_deref() {
        Some("true") => Ok(true),
        Some("false") | None => Ok(false),
        Some(_) => Err(FollowSourceProofError::InvalidHierarchy),
    }
}

fn xml_bounds(value: &str) -> Result<FollowHierarchyBounds, FollowSourceProofError> {
    let values = value
        .split(|character: char| !character.is_ascii_digit() && character != '-')
        .filter(|part| !part.is_empty())
        .map(str::parse::<i32>)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| FollowSourceProofError::InvalidHierarchy)?;
    match values.as_slice() {
        [left, top, right, bottom] => Ok(FollowHierarchyBounds {
            left: *left,
            top: *top,
            right: *right,
            bottom: *bottom,
        }),
        _ => Err(FollowSourceProofError::InvalidHierarchy),
    }
}

fn parse_source_node(
    reader: &Reader<&[u8]>,
    start: &BytesStart<'_>,
    parent: Option<usize>,
) -> Result<Option<FollowHierarchyNode>, FollowSourceProofError> {
    for attribute in start.attributes().with_checks(true) {
        let attribute = attribute.map_err(|_| FollowSourceProofError::InvalidHierarchy)?;
        attribute
            .decoded_and_normalized_value(XmlVersion::Implicit1_0, reader.decoder())
            .map_err(|_| FollowSourceProofError::InvalidHierarchy)?;
    }
    let class_name = xml_attribute(reader, start, b"class")?.unwrap_or_default();
    if class_name.is_empty() || class_name == "hierarchy" {
        return Ok(None);
    }
    let bounds = xml_bounds(
        &xml_attribute(reader, start, b"bounds")?
            .ok_or(FollowSourceProofError::InvalidHierarchy)?,
    )?;
    Ok(Some(FollowHierarchyNode {
        parent,
        package_name: xml_attribute(reader, start, b"package")?.unwrap_or_default(),
        class_name,
        resource_id: xml_attribute(reader, start, b"resource-id")?.unwrap_or_default(),
        text: xml_attribute(reader, start, b"text")?.unwrap_or_default(),
        content_description: xml_attribute(reader, start, b"content-desc")?.unwrap_or_default(),
        bounds,
        enabled: xml_bool(reader, start, b"enabled")?,
        clickable: xml_bool(reader, start, b"clickable")?,
        selected: xml_bool(reader, start, b"selected")?,
    }))
}

fn parse_source_hierarchy(
    source: &HierarchySourceSnapshot,
) -> Result<FollowHierarchySnapshot, FollowSourceProofError> {
    if source.generation == 0
        || source.xml.trim().is_empty()
        || source.xml.len() > MAX_HIERARCHY_BYTES
    {
        return Err(FollowSourceProofError::InvalidHierarchy);
    }
    let mut reader = Reader::from_str(&source.xml);
    let mut nodes = Vec::new();
    let mut parents: Vec<Option<usize>> = Vec::new();
    loop {
        match reader.read_event() {
            Ok(Event::Start(start)) => {
                if parents.len() >= MAX_HIERARCHY_DEPTH {
                    return Err(FollowSourceProofError::InvalidHierarchy);
                }
                let parent = parents.iter().rev().flatten().next().copied();
                let index = parse_source_node(&reader, &start, parent)?.map(|node| {
                    let index = nodes.len();
                    nodes.push(node);
                    index
                });
                if nodes.len() > MAX_HIERARCHY_NODES {
                    return Err(FollowSourceProofError::InvalidHierarchy);
                }
                parents.push(index);
            }
            Ok(Event::Empty(start)) => {
                let parent = parents.iter().rev().flatten().next().copied();
                if let Some(node) = parse_source_node(&reader, &start, parent)? {
                    nodes.push(node);
                    if nodes.len() > MAX_HIERARCHY_NODES {
                        return Err(FollowSourceProofError::InvalidHierarchy);
                    }
                }
            }
            Ok(Event::End(_)) => {
                parents
                    .pop()
                    .ok_or(FollowSourceProofError::InvalidHierarchy)?;
            }
            Ok(Event::Eof) if parents.is_empty() => break,
            Ok(Event::Eof) | Err(_) => return Err(FollowSourceProofError::InvalidHierarchy),
            Ok(_) => {}
        }
    }
    Ok(FollowHierarchySnapshot {
        generation: source.generation,
        tuple: FollowHierarchyTuple::measured(),
        nodes,
    })
}

fn ancestry(snapshot: &FollowHierarchySnapshot, index: usize) -> Option<Vec<usize>> {
    let mut seen = vec![false; snapshot.nodes.len()];
    let mut chain = Vec::new();
    let mut cursor = snapshot.nodes.get(index)?.parent;
    while let Some(parent) = cursor {
        if parent >= snapshot.nodes.len() || seen[parent] {
            return None;
        }
        seen[parent] = true;
        chain.push(parent);
        cursor = snapshot.nodes[parent].parent;
    }
    Some(chain)
}

fn measured_following_row(
    snapshot: &FollowHierarchySnapshot,
    handle_index: usize,
    control_index: usize,
) -> Option<usize> {
    let handle = snapshot.nodes.get(handle_index)?;
    let metadata_index = handle.parent?;
    let metadata = snapshot.nodes.get(metadata_index)?;
    let row_index = metadata.parent?;
    let row = snapshot.nodes.get(row_index)?;
    let control = snapshot.nodes.get(control_index)?;
    (control.parent == Some(row_index)
        && row.parent.is_some()
        && metadata.package_name == MEASURED_FOLLOW_PACKAGE
        && metadata.class_name == FOLLOWING_ROW_CLASS
        && metadata.resource_id.is_empty()
        && metadata.text.is_empty()
        && metadata.content_description.is_empty()
        && metadata.enabled
        && !metadata.clickable
        && !metadata.selected
        && row.package_name == MEASURED_FOLLOW_PACKAGE
        && row.class_name == FOLLOWING_ROW_CLASS
        && row.resource_id.is_empty()
        && row.text.is_empty()
        && row.content_description.is_empty()
        && row.enabled
        && row.clickable
        && !row.selected
        && metadata.bounds.contains(handle.bounds)
        && row.bounds.contains(metadata.bounds)
        && row.bounds.contains(control.bounds))
    .then_some(row_index)
}

fn canonical_handle(value: &str) -> Option<String> {
    let value = value.trim();
    let raw = value.strip_prefix('@').unwrap_or(value);
    if !(2..=32).contains(&raw.len())
        || !raw
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_'))
    {
        return None;
    }
    Some(format!("@{}", raw.to_ascii_lowercase()))
}

fn valid_sha256(value: &str) -> bool {
    value.len() == 64
        && value == value.to_ascii_lowercase()
        && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn hex_sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;

    use super::*;
    use crate::tiktok_public_cleanup::{
        clear_public_toggle, PublicEffectState, ToggleCleanupAdapter, ToggleCleanupObservation,
        ToggleCleanupVerdict,
    };

    fn source(handle: &str) -> NurtureFollowSourceIdentity {
        let snapshot = profile_snapshot(handle, 1);
        let proof = snapshot.author_profile_proof(4).expect("author proof");
        NurtureFollowSourceIdentity::from_profile_snapshot(
            proof.card_key().expect("card key"),
            &snapshot,
            4,
        )
        .expect("source profile proof")
    }

    fn bounds(left: i32, top: i32, right: i32, bottom: i32) -> FollowHierarchyBounds {
        FollowHierarchyBounds {
            left,
            top,
            right,
            bottom,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn full_node(
        parent: Option<usize>,
        class_name: &str,
        resource_id: &str,
        text: &str,
        content_description: &str,
        bounds: FollowHierarchyBounds,
        actionable: bool,
        selected: bool,
    ) -> FollowHierarchyNode {
        FollowHierarchyNode {
            parent,
            package_name: MEASURED_FOLLOW_PACKAGE.to_owned(),
            class_name: class_name.to_owned(),
            resource_id: resource_id.to_owned(),
            text: text.to_owned(),
            content_description: content_description.to_owned(),
            bounds,
            enabled: true,
            clickable: actionable,
            selected,
        }
    }

    fn profile_snapshot(handle: &str, generation: u64) -> FollowHierarchySnapshot {
        FollowHierarchySnapshot {
            generation,
            tuple: FollowHierarchyTuple::measured(),
            nodes: vec![
                full_node(
                    None,
                    "android.widget.FrameLayout",
                    "root",
                    "",
                    "",
                    bounds(0, 0, 1080, 2160),
                    false,
                    false,
                ),
                full_node(
                    Some(0),
                    SOURCE_PAGER_CLASS,
                    SOURCE_PAGER_ID,
                    "",
                    "",
                    bounds(0, 0, 1080, 2000),
                    false,
                    false,
                ),
                full_node(
                    Some(1),
                    SOURCE_RAIL_CLASS,
                    SOURCE_RAIL_ID,
                    "",
                    "",
                    bounds(0, 0, 1080, 1900),
                    true,
                    false,
                ),
                full_node(
                    Some(2),
                    SOURCE_CARD_CLASS,
                    SOURCE_CARD_ID,
                    "",
                    "",
                    bounds(0, 100, 1080, 1900),
                    true,
                    false,
                ),
                full_node(
                    Some(3),
                    SOURCE_PROFILE_CLASS,
                    SOURCE_PROFILE_ID,
                    "",
                    &format!("{handle} profile"),
                    bounds(24, 1500, 580, 1560),
                    true,
                    false,
                ),
                full_node(
                    Some(3),
                    SOURCE_FOLLOW_CLASS,
                    SOURCE_FOLLOW_ID,
                    "",
                    &format!("Follow {handle}"),
                    bounds(870, 1450, 1030, 1530),
                    true,
                    false,
                ),
                full_node(
                    Some(0),
                    FEED_TAB_CLASS,
                    "",
                    "",
                    FEED_TAB_DESCRIPTION,
                    bounds(540, 2020, 1080, 2160),
                    false,
                    true,
                ),
            ],
        }
    }

    fn following_snapshot(
        handle: &str,
        relationship: &str,
        generation: u64,
    ) -> FollowHierarchySnapshot {
        FollowHierarchySnapshot {
            generation,
            tuple: FollowHierarchyTuple::measured(),
            nodes: vec![
                full_node(
                    None,
                    "android.widget.FrameLayout",
                    "root",
                    "",
                    "",
                    bounds(0, 0, 1080, 2160),
                    false,
                    false,
                ),
                full_node(
                    Some(0),
                    FOLLOWING_ROW_CLASS,
                    "",
                    "",
                    "",
                    bounds(0, 539, 1080, 728),
                    true,
                    false,
                ),
                full_node(
                    Some(1),
                    FOLLOWING_ROW_CLASS,
                    "",
                    "",
                    "",
                    bounds(221, 585, 684, 682),
                    false,
                    false,
                ),
                full_node(
                    Some(2),
                    FOLLOWING_HANDLE_CLASS,
                    FOLLOWING_HANDLE_ID,
                    handle,
                    "",
                    bounds(221, 640, 684, 682),
                    false,
                    false,
                ),
                full_node(
                    Some(1),
                    FOLLOWING_RELATIONSHIP_CLASS,
                    FOLLOWING_RELATIONSHIP_ID,
                    relationship,
                    "",
                    bounds(723, 596, 954, 670),
                    true,
                    false,
                ),
            ],
        }
    }

    fn profile_source_xml(handle: &str) -> String {
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<hierarchy rotation="0">
  <node package="{package}" class="android.widget.FrameLayout" resource-id="root" text="" content-desc="" bounds="[0,0][1080,2160]" enabled="true" clickable="false" selected="false">
    <node package="{package}" class="{pager_class}" resource-id="{pager_id}" text="" content-desc="" bounds="[0,0][1080,2000]" enabled="true" clickable="false" selected="false">
      <node package="{package}" class="{rail_class}" resource-id="{rail_id}" text="" content-desc="" bounds="[0,0][1080,1900]" enabled="true" clickable="true" selected="false">
        <node package="{package}" class="{card_class}" resource-id="{card_id}" text="" content-desc="" bounds="[0,100][1080,1900]" enabled="true" clickable="true" selected="false">
          <node package="{package}" class="{profile_class}" resource-id="{profile_id}" text="" content-desc="{handle} profile" bounds="[24,1500][580,1560]" enabled="true" clickable="true" selected="false" />
          <node package="{package}" class="{follow_class}" resource-id="{follow_id}" text="" content-desc="Follow {handle}" bounds="[870,1450][1030,1530]" enabled="true" clickable="true" selected="false" />
        </node>
      </node>
    </node>
    <node package="{package}" class="{feed_class}" resource-id="" text="" content-desc="{feed_desc}" bounds="[540,2020][1080,2160]" enabled="true" clickable="false" selected="true" />
  </node>
</hierarchy>"#,
            package = MEASURED_FOLLOW_PACKAGE,
            pager_class = SOURCE_PAGER_CLASS,
            pager_id = SOURCE_PAGER_ID,
            rail_class = SOURCE_RAIL_CLASS,
            rail_id = SOURCE_RAIL_ID,
            card_class = SOURCE_CARD_CLASS,
            card_id = SOURCE_CARD_ID,
            profile_class = SOURCE_PROFILE_CLASS,
            profile_id = SOURCE_PROFILE_ID,
            follow_class = SOURCE_FOLLOW_CLASS,
            follow_id = SOURCE_FOLLOW_ID,
            feed_class = FEED_TAB_CLASS,
            feed_desc = FEED_TAB_DESCRIPTION,
        )
    }

    fn without_follow_control(xml: String) -> String {
        xml.lines()
            .filter(|line| !line.contains(&format!("resource-id=\"{SOURCE_FOLLOW_ID}\"")))
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn with_card_caption(xml: String, caption: &str) -> String {
        let profile = format!(
            "          <node package=\"{MEASURED_FOLLOW_PACKAGE}\" class=\"{SOURCE_PROFILE_CLASS}\" resource-id=\"{SOURCE_PROFILE_ID}\" text=\"\" content-desc=\"@exact.author profile\" bounds=\"[24,1500][580,1560]\" enabled=\"true\" clickable=\"true\" selected=\"false\" />"
        );
        let caption_node = format!(
            "{profile}\n          <node package=\"{MEASURED_FOLLOW_PACKAGE}\" class=\"android.widget.TextView\" resource-id=\"caption\" text=\"{caption}\" content-desc=\"\" bounds=\"[24,1600][700,1660]\" enabled=\"true\" clickable=\"false\" selected=\"false\" />"
        );
        xml.replace(&profile, &caption_node)
    }

    #[test]
    fn one_source_snapshot_produces_bound_identity_and_follow_tap() {
        let source = HierarchySourceSnapshot {
            generation: 73,
            xml: profile_source_xml("@exact.author"),
        };
        let proved = prove_nurture_follow_source(
            MEASURED_FOLLOW_PACKAGE,
            MEASURED_FOLLOW_VERSION,
            MEASURED_FOLLOW_LOCALE,
            &source,
        )
        .expect("exact source proof");
        assert_eq!(proved.identity().canonical_handle(), "@exact.author");
        assert_eq!(proved.hierarchy_generation(), 73);
        assert!(valid_sha256(proved.snapshot_sha256()));
        assert_eq!(
            (proved.follow_tap_point().x, proved.follow_tap_point().y),
            (950.0, 1490.0)
        );
        assert!(proved.identity().validate().is_ok());
    }

    #[test]
    fn one_fresh_snapshot_atomically_confirms_follow_absent_on_same_card() {
        let source = HierarchySourceSnapshot {
            generation: 73,
            xml: profile_source_xml("@exact.author"),
        };
        let proved = prove_nurture_follow_source(
            MEASURED_FOLLOW_PACKAGE,
            MEASURED_FOLLOW_VERSION,
            MEASURED_FOLLOW_LOCALE,
            &source,
        )
        .expect("source proof");
        let readback = readback_nurture_follow_source(
            proved.identity(),
            MEASURED_FOLLOW_PACKAGE,
            MEASURED_FOLLOW_VERSION,
            MEASURED_FOLLOW_LOCALE,
            &HierarchySourceSnapshot {
                generation: 74,
                xml: without_follow_control(profile_source_xml("@exact.author")),
            },
        )
        .expect("fresh readback");
        assert_eq!(
            readback.verdict(),
            NurtureFollowReadbackVerdict::FollowAbsent
        );
        assert_eq!(readback.hierarchy_generation(), 74);
        assert!(valid_sha256(readback.snapshot_sha256()));
    }

    #[test]
    fn follow_readback_distinguishes_present_changed_and_stale() {
        let source = HierarchySourceSnapshot {
            generation: 73,
            xml: profile_source_xml("@exact.author"),
        };
        let proved = prove_nurture_follow_source(
            MEASURED_FOLLOW_PACKAGE,
            MEASURED_FOLLOW_VERSION,
            MEASURED_FOLLOW_LOCALE,
            &source,
        )
        .expect("source proof");
        let read = |generation, xml| {
            readback_nurture_follow_source(
                proved.identity(),
                MEASURED_FOLLOW_PACKAGE,
                MEASURED_FOLLOW_VERSION,
                MEASURED_FOLLOW_LOCALE,
                &HierarchySourceSnapshot { generation, xml },
            )
        };
        assert_eq!(
            read(74, profile_source_xml("@exact.author"))
                .expect("same-card readback")
                .verdict(),
            NurtureFollowReadbackVerdict::FollowPresent
        );
        assert_eq!(
            read(
                74,
                without_follow_control(profile_source_xml("@another.author"))
            )
            .expect("changed-card readback")
            .verdict(),
            NurtureFollowReadbackVerdict::CardChanged
        );
        assert_eq!(
            read(
                73,
                without_follow_control(profile_source_xml("@exact.author"))
            )
            .unwrap_err(),
            FollowReadbackError::StaleGeneration
        );
    }

    #[test]
    fn follow_readback_refuses_same_author_when_card_content_changed() {
        let source = HierarchySourceSnapshot {
            generation: 73,
            xml: with_card_caption(profile_source_xml("@exact.author"), "caption-a"),
        };
        let proved = prove_nurture_follow_source(
            MEASURED_FOLLOW_PACKAGE,
            MEASURED_FOLLOW_VERSION,
            MEASURED_FOLLOW_LOCALE,
            &source,
        )
        .expect("source proof");
        let readback = readback_nurture_follow_source(
            proved.identity(),
            MEASURED_FOLLOW_PACKAGE,
            MEASURED_FOLLOW_VERSION,
            MEASURED_FOLLOW_LOCALE,
            &HierarchySourceSnapshot {
                generation: 74,
                xml: without_follow_control(with_card_caption(
                    profile_source_xml("@exact.author"),
                    "caption-b",
                )),
            },
        )
        .expect("changed-card readback");
        assert_eq!(
            readback.verdict(),
            NurtureFollowReadbackVerdict::CardChanged
        );
    }

    #[test]
    fn source_parser_refuses_noncanonical_author_and_unmeasured_tuple() {
        let display_name = HierarchySourceSnapshot {
            generation: 73,
            xml: profile_source_xml("Display Name"),
        };
        assert_eq!(
            prove_nurture_follow_source(
                MEASURED_FOLLOW_PACKAGE,
                MEASURED_FOLLOW_VERSION,
                MEASURED_FOLLOW_LOCALE,
                &display_name,
            )
            .unwrap_err(),
            FollowSourceProofError::NonCanonicalAuthor
        );
        let canonical = HierarchySourceSnapshot {
            generation: 73,
            xml: profile_source_xml("@exact.author"),
        };
        assert_eq!(
            prove_nurture_follow_source(
                MEASURED_FOLLOW_PACKAGE,
                "38.3.3",
                MEASURED_FOLLOW_LOCALE,
                &canonical,
            )
            .unwrap_err(),
            FollowSourceProofError::UnmeasuredTuple
        );
    }

    #[test]
    fn source_parser_refuses_wrong_card_ancestry_and_stale_feed_tab() {
        let wrong_card = HierarchySourceSnapshot {
            generation: 73,
            xml: profile_source_xml("@exact.author").replace(SOURCE_CARD_ID, ":id/other_card"),
        };
        assert_eq!(
            prove_nurture_follow_source(
                MEASURED_FOLLOW_PACKAGE,
                MEASURED_FOLLOW_VERSION,
                MEASURED_FOLLOW_LOCALE,
                &wrong_card,
            )
            .unwrap_err(),
            FollowSourceProofError::Missing
        );
        let stale_tab = HierarchySourceSnapshot {
            generation: 74,
            xml: profile_source_xml("@exact.author").replace(
                "content-desc=\"For You\" bounds=\"[540,2020][1080,2160]\" enabled=\"true\" clickable=\"false\" selected=\"true\"",
                "content-desc=\"For You\" bounds=\"[540,2020][1080,2160]\" enabled=\"true\" clickable=\"false\" selected=\"false\"",
            ),
        };
        assert_eq!(
            prove_nurture_follow_source(
                MEASURED_FOLLOW_PACKAGE,
                MEASURED_FOLLOW_VERSION,
                MEASURED_FOLLOW_LOCALE,
                &stale_tab,
            )
            .unwrap_err(),
            FollowSourceProofError::Missing
        );
    }

    #[test]
    fn source_parser_requires_exact_profile_and_follow_controls() {
        for xml in [
            profile_source_xml("@exact.author").replace(
                &format!("class=\"{SOURCE_PROFILE_CLASS}\" resource-id=\"{SOURCE_PROFILE_ID}\""),
                &format!(
                    "class=\"android.view.View\" resource-id=\"{SOURCE_PROFILE_ID}\""
                ),
            ),
            profile_source_xml("@exact.author").replace(
                "content-desc=\"Follow @exact.author\"",
                "content-desc=\"Follow another.author\"",
            ),
            profile_source_xml("@exact.author").replace(
                &format!(
                    "resource-id=\"{SOURCE_FOLLOW_ID}\" text=\"\" content-desc=\"Follow @exact.author\" bounds=\"[870,1450][1030,1530]\" enabled=\"true\" clickable=\"true\""
                ),
                &format!(
                    "resource-id=\"{SOURCE_FOLLOW_ID}\" text=\"\" content-desc=\"Follow @exact.author\" bounds=\"[870,1450][1030,1530]\" enabled=\"true\" clickable=\"false\""
                ),
            ),
        ] {
            assert_eq!(
                prove_nurture_follow_source(
                    MEASURED_FOLLOW_PACKAGE,
                    MEASURED_FOLLOW_VERSION,
                    MEASURED_FOLLOW_LOCALE,
                    &HierarchySourceSnapshot {
                        generation: 75,
                        xml,
                    },
                )
                .unwrap_err(),
                FollowSourceProofError::Missing
            );
        }
    }

    #[test]
    fn measured_following_and_friends_rows_bind_exact_handle() {
        for (label, expected) in [
            ("Following", FollowRelationshipProof::Following),
            ("Friends", FollowRelationshipProof::Friends),
        ] {
            let bound = bind_following_row(
                &source("@user40802176074960"),
                &following_snapshot("user40802176074960", label, 2),
            )
            .expect("measured row");
            assert_eq!(bound.relationship, expected);
            assert_eq!((bound.tap_point.x, bound.tap_point.y), (838.5, 633.0));
            assert_eq!((bound.row_node_index, bound.handle_node_index), (1, 3));
            assert_eq!(bound.relationship_node_index, 4);
            assert!(valid_sha256(&bound.snapshot_sha256));
            assert!(valid_sha256(&bound.row_proof_key));
        }
    }

    #[test]
    fn wrong_row_follow_back_duplicates_and_a_to_b_fail_closed() {
        let expected = source("@campaign.author");
        assert_eq!(
            bind_following_row(
                &expected,
                &following_snapshot("other.author", "Following", 2),
            )
            .unwrap_err(),
            FollowRowRefusal::ExpectedHandleMissing
        );
        assert_eq!(
            bind_following_row(
                &expected,
                &following_snapshot("campaign.author", "Follow back", 2),
            )
            .unwrap_err(),
            FollowRowRefusal::UnexpectedRelationship
        );
        let mut duplicate_handle = following_snapshot("campaign.author", "Following", 2);
        duplicate_handle.nodes.push(full_node(
            Some(2),
            FOLLOWING_HANDLE_CLASS,
            FOLLOWING_HANDLE_ID,
            "@CAMPAIGN.AUTHOR",
            "",
            bounds(221, 580, 684, 622),
            false,
            false,
        ));
        assert_eq!(
            bind_following_row(&expected, &duplicate_handle).unwrap_err(),
            FollowRowRefusal::DuplicateExpectedHandle
        );
        let mut duplicate_relationship = following_snapshot("campaign.author", "Following", 2);
        duplicate_relationship.nodes.push(full_node(
            Some(1),
            FOLLOWING_RELATIONSHIP_CLASS,
            FOLLOWING_RELATIONSHIP_ID,
            "Friends",
            "",
            bounds(723, 610, 954, 684),
            true,
            false,
        ));
        assert_eq!(
            bind_following_row(&expected, &duplicate_relationship).unwrap_err(),
            FollowRowRefusal::DuplicateRelationship
        );
        let mut adjacent_row = following_snapshot("campaign.author", "", 2);
        adjacent_row.nodes[4].resource_id = ":id/not_relationship".into();
        adjacent_row.nodes.push(full_node(
            Some(0),
            FOLLOWING_ROW_CLASS,
            "",
            "",
            "",
            bounds(0, 730, 1080, 870),
            true,
            false,
        ));
        adjacent_row.nodes.push(full_node(
            Some(5),
            FOLLOWING_ROW_CLASS,
            "",
            "",
            "",
            bounds(0, 500, 1080, 1600),
            false,
            false,
        ));
        adjacent_row.nodes.push(full_node(
            Some(5),
            FOLLOWING_RELATIONSHIP_CLASS,
            FOLLOWING_RELATIONSHIP_ID,
            "Following",
            "",
            bounds(723, 746, 954, 820),
            true,
            false,
        ));
        assert_eq!(
            bind_following_row(&expected, &adjacent_row).unwrap_err(),
            FollowRowRefusal::RelationshipMissing
        );
    }

    #[test]
    fn source_witness_tuple_generation_and_content_are_cryptographically_bound() {
        let source = source("@campaign.author");
        assert!(source.validate().is_ok());

        let mut tampered = source.clone();
        tampered.author_profile_proof.profile_resource_id = ":id/other_author".into();
        assert!(tampered.validate().is_err());

        let mut other_card = source.clone();
        other_card.card_key = "d".repeat(64);
        assert!(other_card.validate().is_err());

        let mut wrong_tuple = following_snapshot("campaign.author", "Following", 2);
        wrong_tuple.tuple.version_name = "38.3.3".into();
        assert_eq!(
            bind_following_row(&source, &wrong_tuple).unwrap_err(),
            FollowRowRefusal::InvalidSnapshot
        );

        let first = bind_following_row(
            &source,
            &following_snapshot("campaign.author", "Following", 2),
        )
        .expect("first proof");
        let fresh = bind_following_row(
            &source,
            &following_snapshot("campaign.author", "Following", 3),
        )
        .expect("fresh proof");
        assert!(fresh.is_fresh_reproof_of(&first));
        assert!(!first.is_fresh_reproof_of(&fresh));
    }

    #[test]
    fn weak_source_identity_never_binds_a_row() {
        let snapshot = following_snapshot("campaign.author", "Following", 2);
        let mut weak = source("@campaign.author");
        weak.canonical_handle = "display name".into();
        assert_eq!(
            bind_following_row(&weak, &snapshot).unwrap_err(),
            FollowRowRefusal::InvalidSourceIdentity
        );
        weak.canonical_handle = "@campaign.author".into();
        weak.author_profile_key = "not-a-digest".into();
        assert_eq!(
            bind_following_row(&weak, &snapshot).unwrap_err(),
            FollowRowRefusal::InvalidSourceIdentity
        );
    }

    fn observation(
        sequence: u64,
        source: &NurtureFollowSourceIdentity,
    ) -> ToggleCleanupObservation {
        ToggleCleanupObservation {
            identity: Some(PublicCleanupIdentity::Toggle {
                card_key: source.card_key.clone(),
                author: source.canonical_handle.clone(),
                effect: PublicToggle::Follow,
            }),
            sequence,
            state: PublicEffectState::Present,
            tap_point: Some(TapPoint { x: 838.5, y: 633.0 }),
        }
    }

    struct FollowFixture {
        observations: VecDeque<anyhow::Result<ToggleCleanupObservation>>,
        taps: usize,
        tap_error: Option<&'static str>,
    }

    #[async_trait::async_trait]
    impl ToggleCleanupAdapter for FollowFixture {
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
    async fn follow_a_to_b_during_fresh_reproof_never_arms_or_taps() {
        let source_a = source("@author.a");
        let source_b = source("@author.b");
        let mut fixture = FollowFixture {
            observations: VecDeque::from([
                Ok(observation(1, &source_a)),
                Ok(observation(2, &source_b)),
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
        assert_eq!(
            evidence.verdict,
            ToggleCleanupVerdict::TargetChangedBeforeEffect
        );
        assert_eq!((intents, fixture.taps), (0, 0));
        assert!(evidence.retry_is_safe());
    }

    #[tokio::test]
    async fn follow_transport_failure_after_arm_is_uncertain_and_not_retryable() {
        let expected = source("@exact.author");
        let mut fixture = FollowFixture {
            observations: VecDeque::from([
                Ok(observation(1, &expected)),
                Ok(observation(2, &expected)),
            ]),
            taps: 0,
            tap_error: Some("transport closed"),
        };
        let mut intents = 0;
        let evidence = clear_public_toggle(&mut fixture, |_| {
            intents += 1;
            Ok(())
        })
        .await;
        assert_eq!(evidence.verdict, ToggleCleanupVerdict::UncertainAfterEffect);
        assert_eq!((intents, fixture.taps), (1, 1));
        assert!(!evidence.retry_is_safe());
    }
}
