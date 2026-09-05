//! Measured, fail-closed recognition for TikTok's owned-post delete sheets.
//!
//! This module deliberately stops at an armed intent. The current runtime cannot yet prove that
//! a canonical post is absent after the final tap, and the cleanup journal is interaction-only.
//! Consequently nothing here taps a device or advertises post cleanup as executable.

use crate::tiktok_labels::normalise_language;
use crate::tiktok_public_cleanup::{OwnershipProof, PublicCleanupIdentity};
use crate::types::TapPoint;
use quick_xml::events::{BytesStart, Event};
use quick_xml::{Reader, XmlVersion};
use serde::Serialize;
use sha2::{Digest, Sha256};
use url::Url;

const PACKAGE: &str = "com.ss.android.ugc.trill";
const LANGUAGE: &str = "en";
const VERSION: &str = "38.3.2";
const MAX_HIERARCHY_BYTES: usize = 16 * 1024 * 1024;
const MAX_HIERARCHY_NODES: usize = 32_768;
const MAX_HIERARCHY_DEPTH: usize = 256;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum PostDeleteCatalogIdentity {
    TrillEn3832,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PostDeleteCatalog {
    identity: PostDeleteCatalogIdentity,
    package: &'static str,
    language: &'static str,
    version: &'static str,
    share_delete_description: &'static str,
    dialog_description: &'static str,
    dialog_title_id_suffix: &'static str,
    reedit_label_id_suffix: &'static str,
    final_label_id_suffix: &'static str,
    final_parent_id_suffix: &'static str,
}

const MEASURED: PostDeleteCatalog = PostDeleteCatalog {
    identity: PostDeleteCatalogIdentity::TrillEn3832,
    package: PACKAGE,
    language: LANGUAGE,
    version: VERSION,
    share_delete_description: "Delete",
    dialog_description: "Bottom sheet",
    dialog_title_id_suffix: ":id/kdu",
    reedit_label_id_suffix: ":id/n7y",
    final_label_id_suffix: ":id/dlu",
    final_parent_id_suffix: ":id/dld",
};

impl PostDeleteCatalog {
    pub fn identity(&self) -> PostDeleteCatalogIdentity {
        self.identity
    }

    pub fn package(&self) -> &'static str {
        self.package
    }

    pub fn language(&self) -> &'static str {
        self.language
    }

    pub fn version(&self) -> &'static str {
        self.version
    }
}

/// Return a delete catalog only for the exact measured package/build/language tuple.
pub fn post_delete_catalog(
    package: &str,
    language: &str,
    version: &str,
) -> Option<&'static PostDeleteCatalog> {
    (package == PACKAGE && normalise_language(language) == LANGUAGE && version == VERSION)
        .then_some(&MEASURED)
}

#[derive(Debug, Clone, PartialEq)]
struct PostDeleteHierarchyNode {
    parent: Option<usize>,
    package_name: String,
    class_name: String,
    text: String,
    content_description: String,
    resource_id: String,
    clickable: bool,
    enabled: bool,
    heading: bool,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

impl PostDeleteHierarchyNode {
    fn valid_tap_geometry(&self) -> bool {
        [self.x, self.y, self.width, self.height]
            .iter()
            .all(|value| value.is_finite())
            && self.x >= 0.0
            && self.y >= 0.0
            && self.width > 0.0
            && self.height > 0.0
    }

    fn centre(&self) -> TapPoint {
        TapPoint {
            x: self.x + self.width / 2.0,
            y: self.y + self.height / 2.0,
        }
    }
}

#[derive(Debug, Clone)]
pub struct MeasuredPostDeleteControl {
    point: TapPoint,
    node_index: usize,
    observation_sequence: u64,
    hierarchy_sha256: String,
    catalog: PostDeleteCatalogIdentity,
    stage: PostDeleteControlStage,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PostDeleteControlStage {
    Navigation,
    Confirmation,
}

impl MeasuredPostDeleteControl {
    pub fn point(&self) -> &TapPoint {
        &self.point
    }

    pub fn node_index(&self) -> usize {
        self.node_index
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum PostDeleteLocateError {
    #[error("the measured delete control is absent")]
    Missing,
    #[error("more than one measured delete control matches")]
    Ambiguous,
    #[error("the measured delete control has invalid geometry or ancestry")]
    Invalid,
    #[error("the measured delete dialog shape is not present")]
    WrongDialog,
    #[error("the hierarchy snapshot is malformed")]
    InvalidHierarchy,
}

fn snapshot_sha256(source: &str) -> String {
    format!("{:x}", Sha256::digest(source.as_bytes()))
}

fn attribute(reader: &Reader<&[u8]>, start: &BytesStart<'_>, name: &[u8]) -> Option<String> {
    start
        .attributes()
        .with_checks(true)
        .filter_map(Result::ok)
        .find(|attribute| attribute.key.as_ref() == name)
        .and_then(|attribute| {
            attribute
                .decoded_and_normalized_value(XmlVersion::Implicit1_0, reader.decoder())
                .ok()
        })
        .map(|value| value.into_owned())
}

fn parse_bool(reader: &Reader<&[u8]>, start: &BytesStart<'_>, name: &[u8]) -> bool {
    attribute(reader, start, name).as_deref() == Some("true")
}

fn parse_bounds(value: &str) -> Option<(f64, f64, f64, f64)> {
    let values: Vec<f64> = value
        .split(|character: char| {
            !character.is_ascii_digit() && character != '-' && character != '.'
        })
        .filter(|part| !part.is_empty())
        .map(str::parse)
        .collect::<Result<_, _>>()
        .ok()?;
    match values.as_slice() {
        [left, top, right, bottom] if right > left && bottom > top => {
            Some((*left, *top, right - left, bottom - top))
        }
        _ => None,
    }
}

fn parse_node(
    reader: &Reader<&[u8]>,
    start: &BytesStart<'_>,
    parent: Option<usize>,
) -> Result<Option<PostDeleteHierarchyNode>, PostDeleteLocateError> {
    let class_name = attribute(reader, start, b"class").unwrap_or_default();
    if class_name.is_empty() || class_name == "hierarchy" {
        return Ok(None);
    }
    let (x, y, width, height) = parse_bounds(
        &attribute(reader, start, b"bounds").ok_or(PostDeleteLocateError::InvalidHierarchy)?,
    )
    .ok_or(PostDeleteLocateError::InvalidHierarchy)?;
    Ok(Some(PostDeleteHierarchyNode {
        parent,
        package_name: attribute(reader, start, b"package").unwrap_or_default(),
        class_name,
        text: attribute(reader, start, b"text").unwrap_or_default(),
        content_description: attribute(reader, start, b"content-desc").unwrap_or_default(),
        resource_id: attribute(reader, start, b"resource-id").unwrap_or_default(),
        clickable: parse_bool(reader, start, b"clickable"),
        enabled: parse_bool(reader, start, b"enabled"),
        heading: parse_bool(reader, start, b"heading"),
        x,
        y,
        width,
        height,
    }))
}

fn parse_hierarchy(source: &str) -> Result<Vec<PostDeleteHierarchyNode>, PostDeleteLocateError> {
    if source.len() > MAX_HIERARCHY_BYTES {
        return Err(PostDeleteLocateError::InvalidHierarchy);
    }
    let mut reader = Reader::from_str(source);
    let mut nodes = Vec::new();
    let mut parents: Vec<Option<usize>> = Vec::new();
    loop {
        match reader.read_event() {
            Ok(Event::Start(start)) => {
                if parents.len() >= MAX_HIERARCHY_DEPTH {
                    return Err(PostDeleteLocateError::InvalidHierarchy);
                }
                let parent = parents.iter().rev().flatten().next().copied();
                let index = parse_node(&reader, &start, parent)?.map(|node| {
                    let index = nodes.len();
                    nodes.push(node);
                    index
                });
                if nodes.len() > MAX_HIERARCHY_NODES {
                    return Err(PostDeleteLocateError::InvalidHierarchy);
                }
                parents.push(index);
            }
            Ok(Event::Empty(start)) => {
                let parent = parents.iter().rev().flatten().next().copied();
                if let Some(node) = parse_node(&reader, &start, parent)? {
                    nodes.push(node);
                    if nodes.len() > MAX_HIERARCHY_NODES {
                        return Err(PostDeleteLocateError::InvalidHierarchy);
                    }
                }
            }
            Ok(Event::End(_)) => {
                parents
                    .pop()
                    .ok_or(PostDeleteLocateError::InvalidHierarchy)?;
            }
            Ok(Event::Eof) if parents.is_empty() => return Ok(nodes),
            Ok(Event::Eof) | Err(_) => return Err(PostDeleteLocateError::InvalidHierarchy),
            Ok(_) => {}
        }
    }
}

fn unique(indices: Vec<usize>) -> Result<usize, PostDeleteLocateError> {
    match indices.as_slice() {
        [index] => Ok(*index),
        [] => Err(PostDeleteLocateError::Missing),
        _ => Err(PostDeleteLocateError::Ambiguous),
    }
}

fn has_ancestor(nodes: &[PostDeleteHierarchyNode], child: usize, ancestor: usize) -> bool {
    let mut next = nodes.get(child).and_then(|node| node.parent);
    for _ in 0..nodes.len() {
        match next {
            Some(index) if index == ancestor => return true,
            Some(index) => next = nodes.get(index).and_then(|node| node.parent),
            None => return false,
        }
    }
    false
}

/// Locate the exact share-rail navigation button revealed by a horizontal rail swipe.
pub fn locate_post_delete_navigation(
    hierarchy_source: &str,
    catalog: &PostDeleteCatalog,
    observation_sequence: u64,
) -> Result<MeasuredPostDeleteControl, PostDeleteLocateError> {
    let nodes = parse_hierarchy(hierarchy_source)?;
    let index = unique(
        nodes
            .iter()
            .enumerate()
            .filter(|(_, node)| {
                node.class_name == "android.widget.Button"
                    && node.package_name == catalog.package
                    && node.text.is_empty()
                    && node.content_description == catalog.share_delete_description
                    && node.clickable
                    && node.enabled
            })
            .map(|(index, _)| index)
            .collect(),
    )?;
    let node = &nodes[index];
    if !node.valid_tap_geometry() {
        return Err(PostDeleteLocateError::Invalid);
    }
    Ok(MeasuredPostDeleteControl {
        point: node.centre(),
        node_index: index,
        observation_sequence,
        hierarchy_sha256: snapshot_sha256(hierarchy_source),
        catalog: catalog.identity,
        stage: PostDeleteControlStage::Navigation,
    })
}

/// Locate the exact final `Delete` row, excluding the `Delete` heading and `Delete and re-edit`.
pub fn locate_post_delete_confirmation(
    hierarchy_source: &str,
    catalog: &PostDeleteCatalog,
    observation_sequence: u64,
) -> Result<MeasuredPostDeleteControl, PostDeleteLocateError> {
    let nodes = parse_hierarchy(hierarchy_source)?;
    let sheet = unique(
        nodes
            .iter()
            .enumerate()
            .filter(|(_, node)| {
                node.package_name == catalog.package
                    && node.content_description == catalog.dialog_description
            })
            .map(|(index, _)| index)
            .collect(),
    )
    .map_err(|_| PostDeleteLocateError::WrongDialog)?;

    let title_count = nodes
        .iter()
        .enumerate()
        .filter(|(index, node)| {
            has_ancestor(&nodes, *index, sheet)
                && node.package_name == catalog.package
                && node.text == "Delete"
                && node.heading
                && node.resource_id.ends_with(catalog.dialog_title_id_suffix)
        })
        .count();
    let reedit_count = nodes
        .iter()
        .enumerate()
        .filter(|(index, node)| {
            has_ancestor(&nodes, *index, sheet)
                && node.package_name == catalog.package
                && node.text == "Delete and re-edit"
                && node.resource_id.ends_with(catalog.reedit_label_id_suffix)
        })
        .count();
    if title_count != 1 || reedit_count != 1 {
        return Err(PostDeleteLocateError::WrongDialog);
    }

    let label = unique(
        nodes
            .iter()
            .enumerate()
            .filter(|(index, node)| {
                has_ancestor(&nodes, *index, sheet)
                    && node.package_name == catalog.package
                    && node.class_name == "android.widget.TextView"
                    && node.text == "Delete"
                    && !node.heading
                    && node.resource_id.ends_with(catalog.final_label_id_suffix)
            })
            .map(|(index, _)| index)
            .collect(),
    )?;
    let parent = nodes[label].parent.ok_or(PostDeleteLocateError::Invalid)?;
    let row = nodes.get(parent).ok_or(PostDeleteLocateError::Invalid)?;
    if !has_ancestor(&nodes, parent, sheet)
        || row.package_name != catalog.package
        || !row.resource_id.ends_with(catalog.final_parent_id_suffix)
        || !row.clickable
        || !row.enabled
        || !row.valid_tap_geometry()
    {
        return Err(PostDeleteLocateError::Invalid);
    }
    Ok(MeasuredPostDeleteControl {
        point: row.centre(),
        node_index: parent,
        observation_sequence,
        hierarchy_sha256: snapshot_sha256(hierarchy_source),
        catalog: catalog.identity,
        stage: PostDeleteControlStage::Confirmation,
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OwnedPostContinuityProof {
    catalog: PostDeleteCatalogIdentity,
    device_udid: String,
    session_id: String,
    sequence: u64,
    canonical_url: String,
    identity: PublicCleanupIdentity,
    ownership: OwnershipProof,
    hierarchy_sha256: String,
    frame_sha256: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PostDeleteNavigationProof {
    source: OwnedPostContinuityProof,
    observation_sequence: u64,
    hierarchy_sha256: String,
    frame_sha256: String,
    control: TapPoint,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArmedPostDeleteIntent {
    navigation: PostDeleteNavigationProof,
    navigation_tap_sequence: u64,
    dialog_sequence: u64,
    dialog_hierarchy_sha256: String,
    confirmation: TapPoint,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum PostDeleteContinuityError {
    #[error("owned post identity or ownership is incomplete")]
    WeakSource,
    #[error("canonical URL does not identify the same post and author")]
    CanonicalMismatch,
    #[error("hierarchy or frame digest is missing or invalid")]
    InvalidDigest,
    #[error("post identity changed before the delete sheet")]
    TargetChanged,
    #[error("delete observations are stale or not immediately consecutive")]
    StaleSequence,
    #[error("the owned-post hierarchy or screenshot is not a valid measured snapshot")]
    InvalidSnapshot,
}

fn valid_digest(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn canonical_post(url: &str) -> Option<(String, String)> {
    let parsed = Url::parse(url).ok()?;
    if parsed.scheme() != "https"
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.port().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
    {
        return None;
    }
    let host = parsed.host_str()?.to_ascii_lowercase();
    if !matches!(host.as_str(), "tiktok.com" | "www.tiktok.com") {
        return None;
    }
    let parts: Vec<&str> = parsed.path_segments()?.collect();
    let (author, kind, content_id) = match parts.as_slice() {
        [author, kind, content_id] => (*author, *kind, *content_id),
        [author, kind, content_id, ""] => (*author, *kind, *content_id),
        _ => return None,
    };
    if !matches!(kind, "video" | "photo")
        || content_id.is_empty()
        || !content_id.bytes().all(|byte| byte.is_ascii_digit())
    {
        return None;
    }
    let handle = author.strip_prefix('@')?;
    if !(2..=32).contains(&handle.len())
        || !handle
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_'))
    {
        return None;
    }
    Some((handle.to_string(), content_id.to_string()))
}

fn frame_digest(frame_png: &[u8]) -> Result<String, PostDeleteContinuityError> {
    const PNG_SIGNATURE: &[u8; 8] = b"\x89PNG\r\n\x1a\n";
    if !frame_png.starts_with(PNG_SIGNATURE) {
        return Err(PostDeleteContinuityError::InvalidSnapshot);
    }
    Ok(format!("{:x}", Sha256::digest(frame_png)))
}

fn strong_owned_markers(nodes: &[PostDeleteHierarchyNode], catalog: &PostDeleteCatalog) -> bool {
    nodes
        .iter()
        .filter(|node| {
            node.class_name == "android.widget.Button"
                && node.package_name == catalog.package
                && node.text == "Privacy settings"
                && node.clickable
                && node.enabled
        })
        .count()
        == 1
        && nodes
            .iter()
            .filter(|node| {
                node.class_name == "android.widget.TextView"
                    && node.package_name == catalog.package
                    && node.text.ends_with(" views")
                    && {
                        let count = node.text.trim_end_matches(" views");
                        !count.is_empty()
                            && count.chars().all(|character| {
                                character.is_ascii_digit() || matches!(character, ',' | '.')
                            })
                    }
            })
            .count()
            == 1
}

/// Build a strong source proof only from the exact measured owned-post hierarchy and frame.
#[allow(clippy::too_many_arguments)]
pub fn prove_owned_post_snapshot(
    catalog: &PostDeleteCatalog,
    device_udid: impl Into<String>,
    session_id: impl Into<String>,
    sequence: u64,
    canonical_url: impl Into<String>,
    identity: PublicCleanupIdentity,
    hierarchy_source: &str,
    frame_png: &[u8],
) -> Result<OwnedPostContinuityProof, PostDeleteContinuityError> {
    let nodes = parse_hierarchy(hierarchy_source)
        .map_err(|_| PostDeleteContinuityError::InvalidSnapshot)?;
    if !strong_owned_markers(&nodes, catalog) {
        return Err(PostDeleteContinuityError::WeakSource);
    }
    let proof = OwnedPostContinuityProof {
        catalog: catalog.identity,
        device_udid: device_udid.into(),
        session_id: session_id.into(),
        sequence,
        canonical_url: canonical_url.into(),
        identity,
        ownership: OwnershipProof::Strong,
        hierarchy_sha256: snapshot_sha256(hierarchy_source),
        frame_sha256: frame_digest(frame_png)?,
    };
    if !source_is_strong(&proof) {
        return Err(PostDeleteContinuityError::WeakSource);
    }
    let (author, content_id) =
        canonical_post(&proof.canonical_url).ok_or(PostDeleteContinuityError::CanonicalMismatch)?;
    match &proof.identity {
        PublicCleanupIdentity::Post {
            content_id: expected_id,
            author: expected_author,
            ..
        } if *expected_id == content_id
            && expected_author
                .trim_start_matches('@')
                .eq_ignore_ascii_case(&author) =>
        {
            Ok(proof)
        }
        _ => Err(PostDeleteContinuityError::CanonicalMismatch),
    }
}

impl ArmedPostDeleteIntent {
    pub fn confirmation_point(&self) -> &TapPoint {
        &self.confirmation
    }

    pub fn catalog_identity(&self) -> PostDeleteCatalogIdentity {
        self.navigation.source.catalog
    }

    pub fn device_udid(&self) -> &str {
        &self.navigation.source.device_udid
    }
}

fn source_is_strong(source: &OwnedPostContinuityProof) -> bool {
    source.catalog == PostDeleteCatalogIdentity::TrillEn3832
        && !source.device_udid.trim().is_empty()
        && !source.session_id.trim().is_empty()
        && source.ownership == OwnershipProof::Strong
        && valid_digest(&source.hierarchy_sha256)
        && valid_digest(&source.frame_sha256)
        && matches!(
            &source.identity,
            PublicCleanupIdentity::Post {
                content_id,
                author,
                caption_sha256,
            } if !content_id.trim().is_empty()
                && !author.trim().is_empty()
                && valid_digest(caption_sha256)
        )
}

/// Bind the share-rail Delete navigation to a fresh re-proof of the owned canonical post.
pub fn bind_post_delete_navigation(
    expected: &OwnedPostContinuityProof,
    reproved: OwnedPostContinuityProof,
    navigation_frame_png: &[u8],
    control: MeasuredPostDeleteControl,
) -> Result<PostDeleteNavigationProof, PostDeleteContinuityError> {
    if !source_is_strong(expected) || !source_is_strong(&reproved) {
        return Err(PostDeleteContinuityError::WeakSource);
    }
    if expected.catalog != reproved.catalog
        || expected.device_udid != reproved.device_udid
        || expected.session_id != reproved.session_id
        || expected.identity != reproved.identity
    {
        return Err(PostDeleteContinuityError::TargetChanged);
    }
    if control.stage != PostDeleteControlStage::Navigation
        || control.catalog != reproved.catalog
        || expected.sequence.checked_add(1) != Some(reproved.sequence)
        || reproved.sequence.checked_add(1) != Some(control.observation_sequence)
    {
        return Err(PostDeleteContinuityError::StaleSequence);
    }
    if !valid_digest(&control.hierarchy_sha256) {
        return Err(PostDeleteContinuityError::InvalidDigest);
    }
    let frame_sha256 = frame_digest(navigation_frame_png)?;
    Ok(PostDeleteNavigationProof {
        source: reproved,
        observation_sequence: control.observation_sequence,
        hierarchy_sha256: control.hierarchy_sha256,
        frame_sha256,
        control: control.point,
    })
}

/// Bind the modal's final Delete row to the immediately preceding navigation tap.
///
/// The dialog hides the post, so identity is intentionally carried from `navigation` rather than
/// fabricated from modal contents. Exact consecutive sequence numbers make that inheritance valid
/// only for the same in-memory session transition.
pub fn arm_post_delete_intent(
    navigation: PostDeleteNavigationProof,
    navigation_tap_sequence: u64,
    confirmation: MeasuredPostDeleteControl,
) -> Result<ArmedPostDeleteIntent, PostDeleteContinuityError> {
    if confirmation.stage != PostDeleteControlStage::Confirmation
        || confirmation.catalog != navigation.source.catalog
        || navigation.observation_sequence.checked_add(1) != Some(navigation_tap_sequence)
        || navigation_tap_sequence.checked_add(1) != Some(confirmation.observation_sequence)
    {
        return Err(PostDeleteContinuityError::StaleSequence);
    }
    if !valid_digest(&confirmation.hierarchy_sha256) {
        return Err(PostDeleteContinuityError::InvalidDigest);
    }
    Ok(ArmedPostDeleteIntent {
        navigation,
        navigation_tap_sequence,
        dialog_sequence: confirmation.observation_sequence,
        dialog_hierarchy_sha256: confirmation.hierarchy_sha256,
        confirmation: confirmation.point,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const NAVIGATION_XML: &str = r#"<hierarchy class="hierarchy">
      <node package="com.ss.android.ugc.trill" class="android.widget.Button" text="" content-desc="Delete" resource-id=""
            clickable="true" enabled="true" heading="false" bounds="[894,1852][996,2036]" />
    </hierarchy>"#;

    const DIALOG_XML: &str = r#"<hierarchy class="hierarchy">
      <node package="com.ss.android.ugc.trill" class="android.widget.FrameLayout" text="" content-desc="Bottom sheet"
            resource-id="com.ss.android.ugc.trill:id/dox" clickable="false" enabled="true"
            heading="false" bounds="[0,1433][996,2094]">
        <node package="com.ss.android.ugc.trill" class="android.widget.Button" text="Delete" content-desc=""
              resource-id="com.ss.android.ugc.trill:id/kdu" clickable="true" enabled="true"
              heading="true" bounds="[469,1473][611,1531]" />
        <node package="com.ss.android.ugc.trill" class="android.widget.LinearLayout" text="" content-desc=""
              resource-id="com.ss.android.ugc.trill:id/n7w" clickable="true" enabled="true"
              heading="false" bounds="[32,1586][996,1745]">
          <node package="com.ss.android.ugc.trill" class="android.widget.TextView" text="Delete and re-edit" content-desc=""
                resource-id="com.ss.android.ugc.trill:id/n7y" clickable="false" enabled="true"
                heading="false" bounds="[180,1639][541,1692]" />
        </node>
        <node package="com.ss.android.ugc.trill" class="android.view.ViewGroup" text="" content-desc=""
              resource-id="com.ss.android.ugc.trill:id/dld" clickable="true" enabled="true"
              heading="false" bounds="[32,1746][996,1999]">
          <node package="com.ss.android.ugc.trill" class="android.widget.TextView" text="Delete" content-desc=""
                resource-id="com.ss.android.ugc.trill:id/dlu" clickable="false" enabled="true"
                heading="false" bounds="[180,1799][309,1852]" />
        </node>
      </node>
    </hierarchy>"#;

    const OWN_POST_XML: &str = r#"<hierarchy class="hierarchy">
      <node package="com.ss.android.ugc.trill" class="android.widget.FrameLayout" text="" content-desc="" resource-id=""
            clickable="false" enabled="true" heading="false" bounds="[0,0][1080,2094]">
        <node package="com.ss.android.ugc.trill" class="android.widget.TextView" text="135 views" content-desc=""
              resource-id="com.ss.android.ugc.trill:id/tjz" clickable="false" enabled="true"
              heading="false" bounds="[111,2006][277,2053]" />
        <node package="com.ss.android.ugc.trill" class="android.widget.Button" text="Privacy settings" content-desc=""
              resource-id="com.ss.android.ugc.trill:id/ma9" clickable="true" enabled="true"
              heading="false" bounds="[706,1982][1047,2077]" />
      </node>
    </hierarchy>"#;

    const FRAME_PNG: &[u8] = b"\x89PNG\r\n\x1a\nfixture";

    fn catalog() -> &'static PostDeleteCatalog {
        post_delete_catalog(PACKAGE, "en-US", VERSION).unwrap()
    }

    fn navigation(sequence: u64) -> MeasuredPostDeleteControl {
        locate_post_delete_navigation(NAVIGATION_XML, catalog(), sequence).unwrap()
    }

    fn confirmation(sequence: u64) -> MeasuredPostDeleteControl {
        locate_post_delete_confirmation(DIALOG_XML, catalog(), sequence).unwrap()
    }

    fn source(sequence: u64, content_id: &str) -> OwnedPostContinuityProof {
        source_on(sequence, content_id, "device-1", "session-1")
    }

    fn source_on(
        sequence: u64,
        content_id: &str,
        device_udid: &str,
        session_id: &str,
    ) -> OwnedPostContinuityProof {
        prove_owned_post_snapshot(
            catalog(),
            device_udid,
            session_id,
            sequence,
            format!("https://www.tiktok.com/@owner/video/{content_id}"),
            PublicCleanupIdentity::Post {
                content_id: content_id.into(),
                author: "owner".into(),
                caption_sha256: "a".repeat(64),
            },
            OWN_POST_XML,
            FRAME_PNG,
        )
        .unwrap()
    }

    #[test]
    fn catalog_is_pinned_to_the_measured_tuple() {
        assert_eq!(catalog().identity(), PostDeleteCatalogIdentity::TrillEn3832);
        assert_eq!(catalog().package(), PACKAGE);
        assert_eq!(catalog().language(), LANGUAGE);
        assert_eq!(catalog().version(), VERSION);
        assert!(post_delete_catalog(PACKAGE, "en", "38.3.3").is_none());
        assert!(post_delete_catalog("com.zhiliaoapp.musically", "en", VERSION).is_none());
    }

    #[test]
    fn canonical_post_requires_the_exact_numeric_tiktok_path() {
        assert_eq!(
            canonical_post("https://www.tiktok.com/@owner_1/video/7400000000000000001"),
            Some(("owner_1".into(), "7400000000000000001".into()))
        );
        assert_eq!(
            canonical_post("https://tiktok.com/@owner.1/photo/7400000000000000002/"),
            Some(("owner.1".into(), "7400000000000000002".into()))
        );
        for invalid in [
            "https://www.tiktok.com/prefix/@owner/video/7400000000000000001",
            "https://www.tiktok.com/@owner/video/7400000000000000001/extra",
            "https://www.tiktok.com/@owner/video/not-a-number",
            "https://www.tiktok.com/@bad-handle/video/7400000000000000001",
            "https://www.tiktok.example/@owner/video/7400000000000000001",
            "https://m.tiktok.com/@owner/video/7400000000000000001",
            "http://www.tiktok.com/@owner/video/7400000000000000001",
            "https://www.tiktok.com/@owner/video/7400000000000000001?lang=en",
            "https://www.tiktok.com/@owner/video/7400000000000000001#comments",
        ] {
            assert_eq!(canonical_post(invalid), None, "accepted {invalid}");
        }
    }

    #[test]
    fn frame_digest_requires_the_complete_png_signature() {
        assert!(frame_digest(FRAME_PNG).is_ok());
        assert_eq!(
            frame_digest(b"\x89PNG").unwrap_err(),
            PostDeleteContinuityError::InvalidSnapshot
        );
        assert_eq!(
            frame_digest(b"\x89PNG\r\n\x1a").unwrap_err(),
            PostDeleteContinuityError::InvalidSnapshot
        );
    }

    #[test]
    fn hierarchy_parser_rejects_sources_over_sixteen_mib() {
        let source = "x".repeat(MAX_HIERARCHY_BYTES + 1);
        assert_eq!(
            parse_hierarchy(&source).unwrap_err(),
            PostDeleteLocateError::InvalidHierarchy
        );
    }

    #[test]
    fn hierarchy_parser_rejects_more_than_256_levels() {
        let mut source = String::from(r#"<hierarchy class="hierarchy">"#);
        let node = r#"<node class="android.view.View" bounds="[0,0][1,1]">"#;
        for _ in 0..MAX_HIERARCHY_DEPTH {
            source.push_str(node);
        }
        for _ in 0..MAX_HIERARCHY_DEPTH {
            source.push_str("</node>");
        }
        source.push_str("</hierarchy>");

        assert_eq!(
            parse_hierarchy(&source).unwrap_err(),
            PostDeleteLocateError::InvalidHierarchy
        );
    }

    #[test]
    fn hierarchy_parser_rejects_more_than_32768_nodes() {
        let mut source = String::from(r#"<hierarchy class="hierarchy">"#);
        let node = r#"<node class="android.view.View" bounds="[0,0][1,1]" />"#;
        for _ in 0..=MAX_HIERARCHY_NODES {
            source.push_str(node);
        }
        source.push_str("</hierarchy>");

        assert_eq!(
            parse_hierarchy(&source).unwrap_err(),
            PostDeleteLocateError::InvalidHierarchy
        );
    }

    #[test]
    fn share_locator_requires_one_exact_clickable_button() {
        let found = navigation(3);
        assert_eq!(found.node_index(), 0);
        assert_eq!(found.hierarchy_sha256, snapshot_sha256(NAVIGATION_XML));

        let decoy = NAVIGATION_XML.replace(
            "content-desc=\"Delete\"",
            "content-desc=\"Delete and re-edit\"",
        );
        assert_eq!(
            locate_post_delete_navigation(&decoy, catalog(), 3).unwrap_err(),
            PostDeleteLocateError::Missing
        );
        let duplicate = NAVIGATION_XML.replace(
            "</hierarchy>",
            r#"<node package="com.ss.android.ugc.trill" class="android.widget.Button" text="" content-desc="Delete" resource-id=""
                       clickable="true" enabled="true" heading="false" bounds="[10,10][20,20]" />
                </hierarchy>"#,
        );
        assert_eq!(
            locate_post_delete_navigation(&duplicate, catalog(), 3).unwrap_err(),
            PostDeleteLocateError::Ambiguous
        );
        let invalid = NAVIGATION_XML.replace("[894,1852][996,2036]", "[10,10][10,20]");
        assert_eq!(
            locate_post_delete_navigation(&invalid, catalog(), 3).unwrap_err(),
            PostDeleteLocateError::InvalidHierarchy
        );
        let wrong_package = NAVIGATION_XML.replace(
            "package=\"com.ss.android.ugc.trill\"",
            "package=\"com.zhiliaoapp.musically\"",
        );
        assert_eq!(
            locate_post_delete_navigation(&wrong_package, catalog(), 3).unwrap_err(),
            PostDeleteLocateError::Missing
        );
    }

    #[test]
    fn dialog_locator_ignores_heading_and_reedit_and_taps_the_final_parent() {
        let found = confirmation(5);
        assert_eq!(found.node_index(), 4);
        assert_eq!(found.hierarchy_sha256, snapshot_sha256(DIALOG_XML));

        let duplicate = DIALOG_XML.replace(
            r#"heading="false" bounds="[180,1799][309,1852]" />"#,
            r#"heading="false" bounds="[180,1799][309,1852]" />
          <node package="com.ss.android.ugc.trill" class="android.widget.TextView" text="Delete" content-desc=""
                resource-id="com.ss.android.ugc.trill:id/dlu" clickable="false" enabled="true"
                heading="false" bounds="[400,1800][500,1850]" />"#,
        );
        assert_eq!(
            locate_post_delete_confirmation(&duplicate, catalog(), 5).unwrap_err(),
            PostDeleteLocateError::Ambiguous
        );
    }

    #[test]
    fn continuity_accepts_a_to_a_and_rejects_a_to_b() {
        let expected = source(1, "111");
        assert!(
            bind_post_delete_navigation(&expected, source(2, "111"), FRAME_PNG, navigation(3),)
                .is_ok()
        );
        assert_eq!(
            bind_post_delete_navigation(&expected, source(2, "222"), FRAME_PNG, navigation(3),)
                .unwrap_err(),
            PostDeleteContinuityError::TargetChanged
        );
    }

    #[test]
    fn continuity_rejects_another_session_or_device() {
        let expected = source(1, "111");
        assert_eq!(
            bind_post_delete_navigation(
                &expected,
                source_on(2, "111", "device-1", "session-2"),
                FRAME_PNG,
                navigation(3),
            )
            .unwrap_err(),
            PostDeleteContinuityError::TargetChanged
        );

        assert_eq!(
            bind_post_delete_navigation(
                &expected,
                source_on(2, "111", "device-2", "session-1"),
                FRAME_PNG,
                navigation(3),
            )
            .unwrap_err(),
            PostDeleteContinuityError::TargetChanged
        );
    }

    #[test]
    fn continuity_rejects_a_canonical_url_for_another_post() {
        assert_eq!(
            prove_owned_post_snapshot(
                catalog(),
                "device-1",
                "session-1",
                2,
                "https://www.tiktok.com/@owner/video/222",
                PublicCleanupIdentity::Post {
                    content_id: "111".into(),
                    author: "owner".into(),
                    caption_sha256: "a".repeat(64),
                },
                OWN_POST_XML,
                FRAME_PNG,
            )
            .unwrap_err(),
            PostDeleteContinuityError::CanonicalMismatch
        );
    }

    #[test]
    fn modal_identity_is_only_inherited_from_an_immediate_same_session_navigation() {
        let expected = source(1, "111");
        let navigation =
            bind_post_delete_navigation(&expected, source(2, "111"), FRAME_PNG, navigation(3))
                .unwrap();
        let armed = arm_post_delete_intent(navigation.clone(), 4, confirmation(5)).unwrap();
        assert_eq!(
            armed.catalog_identity(),
            PostDeleteCatalogIdentity::TrillEn3832
        );
        assert_eq!(armed.device_udid(), "device-1");
        assert!(armed.confirmation_point().x.is_finite());
        assert_eq!(
            arm_post_delete_intent(navigation.clone(), 5, confirmation(6)).unwrap_err(),
            PostDeleteContinuityError::StaleSequence
        );
        assert_eq!(
            arm_post_delete_intent(navigation, 4, confirmation(6)).unwrap_err(),
            PostDeleteContinuityError::StaleSequence
        );
    }

    #[test]
    fn sequence_overflow_refuses_instead_of_saturating() {
        let expected = source(u64::MAX - 2, "111");
        let navigation = bind_post_delete_navigation(
            &expected,
            source(u64::MAX - 1, "111"),
            FRAME_PNG,
            navigation(u64::MAX),
        )
        .unwrap();
        assert_eq!(
            arm_post_delete_intent(navigation, u64::MAX, confirmation(0)).unwrap_err(),
            PostDeleteContinuityError::StaleSequence
        );
    }
}
