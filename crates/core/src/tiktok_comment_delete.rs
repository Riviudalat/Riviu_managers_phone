//! Measured, fail-closed recognition for one campaign-owned TikTok comment.
//!
//! The hierarchy shape in this module was measured on
//! `com.ss.android.ugc.trill` 38.3.2/en. It deliberately stops at an armed Delete-row
//! intent. A virtualized Comment-history list cannot prove that an off-screen row was
//! deleted, so post-effect readback never reports successful deletion.

use crate::tiktok_labels::normalise_language;
use crate::types::TapPoint;
use quick_xml::events::{BytesStart, Event};
use quick_xml::{Reader, XmlVersion};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::str;
use url::Url;

const PACKAGE: &str = "com.ss.android.ugc.trill";
const LANGUAGE: &str = "en";
const VERSION: &str = "38.3.2";
const PNG_SIGNATURE: &[u8; 8] = b"\x89PNG\r\n\x1a\n";
const MAX_HIERARCHY_BYTES: usize = 16 * 1024 * 1024;
const MAX_HIERARCHY_NODES: usize = 32_768;
const MAX_HIERARCHY_DEPTH: usize = 256;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum CommentDeleteCatalogIdentity {
    TrillEn3832,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CommentDeleteCatalog {
    identity: CommentDeleteCatalogIdentity,
    package: &'static str,
    language: &'static str,
    version: &'static str,
    card_anchor_description: &'static str,
    card_anchor_id_suffix: &'static str,
    comment_list_id_suffix: &'static str,
    comment_row_id_suffix: &'static str,
    author_id_suffix: &'static str,
    reply_target_id_suffix: &'static str,
    body_id_suffix: &'static str,
    sheet_description: &'static str,
    sheet_id_suffix: &'static str,
    delete_label: &'static str,
    delete_label_id_suffix: &'static str,
    delete_parent_id_suffix: &'static str,
}

const MEASURED: CommentDeleteCatalog = CommentDeleteCatalog {
    identity: CommentDeleteCatalogIdentity::TrillEn3832,
    package: PACKAGE,
    language: LANGUAGE,
    version: VERSION,
    card_anchor_description: "Video",
    card_anchor_id_suffix: ":id/long_press_layout",
    comment_list_id_suffix: ":id/n6s",
    comment_row_id_suffix: ":id/il1",
    author_id_suffix: ":id/title",
    reply_target_id_suffix: ":id/seu",
    body_id_suffix: ":id/d0h",
    sheet_description: "Bottom sheet",
    sheet_id_suffix: ":id/dox",
    delete_label: "Delete",
    delete_label_id_suffix: ":id/cm5",
    delete_parent_id_suffix: ":id/cm4",
};

impl CommentDeleteCatalog {
    pub fn identity(&self) -> CommentDeleteCatalogIdentity {
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

pub fn comment_delete_catalog(
    package: &str,
    language: &str,
    version: &str,
) -> Option<&'static CommentDeleteCatalog> {
    (package == PACKAGE && normalise_language(language) == LANGUAGE && version == VERSION)
        .then_some(&MEASURED)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommentDetailIdentity {
    pub post_id: String,
    pub comment_id: String,
}

pub fn parse_comment_detail_identity(detail_uri: &str) -> Option<CommentDetailIdentity> {
    let parsed = Url::parse(detail_uri).ok()?;
    if parsed.scheme() != "aweme"
        || parsed.host_str() != Some("aweme")
        || parsed.path().trim_matches('/') != "detail"
    {
        return None;
    }
    let ids: Vec<String> = parsed
        .query_pairs()
        .filter(|(key, _)| key == "id")
        .map(|(_, value)| value.into_owned())
        .collect();
    let comment_ids: Vec<String> = parsed
        .query_pairs()
        .filter(|(key, _)| key == "cid")
        .map(|(_, value)| value.into_owned())
        .collect();
    match (ids.as_slice(), comment_ids.as_slice()) {
        ([post_id], [comment_id])
            if numeric_identifier(post_id) && numeric_identifier(comment_id) =>
        {
            Some(CommentDetailIdentity {
                post_id: post_id.clone(),
                comment_id: comment_id.clone(),
            })
        }
        _ => None,
    }
}

fn numeric_identifier(value: &str) -> bool {
    !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit())
}

pub fn comment_body_sha256(body: &str) -> String {
    hex_sha256(body.as_bytes())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OwnedCommentFingerprint {
    pub post_id: String,
    pub comment_id: String,
    pub author: String,
    pub reply_target: String,
    pub body_sha256: String,
}

impl OwnedCommentFingerprint {
    pub fn from_body(
        detail: CommentDetailIdentity,
        author: impl Into<String>,
        reply_target: impl Into<String>,
        body: &str,
    ) -> Self {
        Self {
            post_id: detail.post_id,
            comment_id: detail.comment_id,
            author: author.into(),
            reply_target: reply_target.into(),
            body_sha256: comment_body_sha256(body),
        }
    }

    fn is_complete(&self) -> bool {
        numeric_identifier(&self.post_id)
            && numeric_identifier(&self.comment_id)
            && exact_nonempty(&self.author)
            && exact_nonempty(&self.reply_target)
            && valid_sha256(&self.body_sha256)
    }
}

fn exact_nonempty(value: &str) -> bool {
    !value.is_empty() && value.trim() == value
}

fn valid_sha256(value: &str) -> bool {
    value.len() == 64
        && value == value.to_ascii_lowercase()
        && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn hex_sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn binding_sha256(parts: &[&[u8]]) -> String {
    let mut digest = Sha256::new();
    for part in parts {
        digest.update((part.len() as u64).to_be_bytes());
        digest.update(part);
    }
    format!("{:x}", digest.finalize())
}

fn frame_sha256(frame_png: &[u8]) -> Result<String, CommentDeleteContinuityError> {
    if !frame_png.starts_with(PNG_SIGNATURE) || frame_png.len() == PNG_SIGNATURE.len() {
        return Err(CommentDeleteContinuityError::InvalidSnapshot);
    }
    Ok(hex_sha256(frame_png))
}

#[derive(Debug, Clone, PartialEq)]
struct CommentDeleteHierarchyNode {
    parent: Option<usize>,
    package_name: String,
    class_name: String,
    text: String,
    content_description: String,
    resource_id: String,
    clickable: bool,
    long_clickable: bool,
    enabled: bool,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

impl CommentDeleteHierarchyNode {
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
) -> Result<Option<CommentDeleteHierarchyNode>, CommentDeleteSnapshotError> {
    let class_name = attribute(reader, start, b"class").unwrap_or_default();
    if class_name.is_empty() || class_name == "hierarchy" {
        return Ok(None);
    }
    let (x, y, width, height) = parse_bounds(
        &attribute(reader, start, b"bounds").ok_or(CommentDeleteSnapshotError::InvalidHierarchy)?,
    )
    .ok_or(CommentDeleteSnapshotError::InvalidHierarchy)?;
    Ok(Some(CommentDeleteHierarchyNode {
        parent,
        package_name: attribute(reader, start, b"package").unwrap_or_default(),
        class_name,
        text: attribute(reader, start, b"text").unwrap_or_default(),
        content_description: attribute(reader, start, b"content-desc").unwrap_or_default(),
        resource_id: attribute(reader, start, b"resource-id").unwrap_or_default(),
        clickable: parse_bool(reader, start, b"clickable"),
        long_clickable: parse_bool(reader, start, b"long-clickable"),
        enabled: parse_bool(reader, start, b"enabled"),
        x,
        y,
        width,
        height,
    }))
}

fn parse_hierarchy(
    hierarchy_xml: &[u8],
) -> Result<Vec<CommentDeleteHierarchyNode>, CommentDeleteSnapshotError> {
    if hierarchy_xml.is_empty() || hierarchy_xml.len() > MAX_HIERARCHY_BYTES {
        return Err(CommentDeleteSnapshotError::InvalidHierarchy);
    }
    let source =
        str::from_utf8(hierarchy_xml).map_err(|_| CommentDeleteSnapshotError::InvalidHierarchy)?;
    let mut reader = Reader::from_str(source);
    let mut nodes = Vec::new();
    let mut parents: Vec<Option<usize>> = Vec::new();
    loop {
        match reader.read_event() {
            Ok(Event::Start(start)) => {
                if parents.len() >= MAX_HIERARCHY_DEPTH {
                    return Err(CommentDeleteSnapshotError::InvalidHierarchy);
                }
                let parent = parents.iter().rev().flatten().next().copied();
                let index = parse_node(&reader, &start, parent)?.map(|node| {
                    let index = nodes.len();
                    nodes.push(node);
                    index
                });
                if nodes.len() > MAX_HIERARCHY_NODES {
                    return Err(CommentDeleteSnapshotError::InvalidHierarchy);
                }
                parents.push(index);
            }
            Ok(Event::Empty(start)) => {
                let parent = parents.iter().rev().flatten().next().copied();
                if let Some(node) = parse_node(&reader, &start, parent)? {
                    nodes.push(node);
                    if nodes.len() > MAX_HIERARCHY_NODES {
                        return Err(CommentDeleteSnapshotError::InvalidHierarchy);
                    }
                }
            }
            Ok(Event::End(_)) => {
                parents
                    .pop()
                    .ok_or(CommentDeleteSnapshotError::InvalidHierarchy)?;
            }
            Ok(Event::Eof) if parents.is_empty() && !nodes.is_empty() => return Ok(nodes),
            Ok(Event::Eof) | Err(_) => return Err(CommentDeleteSnapshotError::InvalidHierarchy),
            Ok(_) => {}
        }
    }
}

fn has_ancestor(nodes: &[CommentDeleteHierarchyNode], child: usize, ancestor: usize) -> bool {
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

fn direct_children_with<F>(
    nodes: &[CommentDeleteHierarchyNode],
    parent: usize,
    predicate: F,
) -> Vec<(usize, &CommentDeleteHierarchyNode)>
where
    F: Fn(&CommentDeleteHierarchyNode) -> bool,
{
    nodes
        .iter()
        .enumerate()
        .filter(|(_, node)| node.parent == Some(parent) && predicate(node))
        .collect()
}

fn unique(indices: Vec<usize>) -> Option<usize> {
    match indices.as_slice() {
        [index] => Some(*index),
        _ => None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum CommentDeleteSnapshotError {
    #[error("the Comment history detail URI is invalid")]
    InvalidDetailUri,
    #[error("the owned-comment fingerprint is incomplete")]
    WeakFingerprint,
    #[error("the hierarchy snapshot is malformed")]
    InvalidHierarchy,
}

#[derive(Debug, Clone)]
struct BoundOwnedCommentRow {
    fingerprint: OwnedCommentFingerprint,
    row_node_index: usize,
    author_node_index: usize,
    reply_target_node_index: usize,
    body_node_index: usize,
    long_press_point: TapPoint,
}

#[derive(Debug, Clone)]
enum OwnedCommentPresence {
    Present(BoundOwnedCommentRow),
    TargetChanged,
    CardUnreadable,
    Ambiguous,
}

#[derive(Debug, Clone)]
struct OwnedCommentObservation {
    detail: CommentDetailIdentity,
    presence: OwnedCommentPresence,
}

fn observe_owned_comment(
    nodes: &[CommentDeleteHierarchyNode],
    catalog: &CommentDeleteCatalog,
    detail_uri: &str,
    expected: &OwnedCommentFingerprint,
) -> Result<OwnedCommentObservation, CommentDeleteSnapshotError> {
    if !expected.is_complete() {
        return Err(CommentDeleteSnapshotError::WeakFingerprint);
    }
    let detail = parse_comment_detail_identity(detail_uri)
        .ok_or(CommentDeleteSnapshotError::InvalidDetailUri)?;
    if detail.post_id != expected.post_id || detail.comment_id != expected.comment_id {
        return Ok(OwnedCommentObservation {
            detail,
            presence: OwnedCommentPresence::TargetChanged,
        });
    }
    let anchors: Vec<usize> = nodes
        .iter()
        .enumerate()
        .filter(|(_, node)| {
            node.package_name == catalog.package
                && node.resource_id.ends_with(catalog.card_anchor_id_suffix)
                && node.content_description == catalog.card_anchor_description
                && node.enabled
        })
        .map(|(index, _)| index)
        .collect();
    let lists: Vec<usize> = nodes
        .iter()
        .enumerate()
        .filter(|(_, node)| {
            node.package_name == catalog.package
                && node.resource_id.ends_with(catalog.comment_list_id_suffix)
                && node.enabled
        })
        .map(|(index, _)| index)
        .collect();
    let (Some(_card_anchor), Some(comment_list)) = (unique(anchors), unique(lists)) else {
        return Ok(OwnedCommentObservation {
            detail,
            presence: OwnedCommentPresence::CardUnreadable,
        });
    };

    let mut matches = Vec::new();
    for (row_index, _) in nodes.iter().enumerate().filter(|(index, node)| {
        node.package_name == catalog.package
            && node.resource_id.ends_with(catalog.comment_row_id_suffix)
            && has_ancestor(nodes, *index, comment_list)
    }) {
        let authors = direct_children_with(nodes, row_index, |node| {
            node.package_name == catalog.package
                && node.resource_id.ends_with(catalog.author_id_suffix)
                && node.text == expected.author
        });
        let targets = direct_children_with(nodes, row_index, |node| {
            node.package_name == catalog.package
                && node.resource_id.ends_with(catalog.reply_target_id_suffix)
                && node.text == expected.reply_target
        });
        let bodies = direct_children_with(nodes, row_index, |node| {
            node.package_name == catalog.package
                && node.resource_id.ends_with(catalog.body_id_suffix)
                && comment_body_sha256(&node.text) == expected.body_sha256
        });
        let ([author], [target], [body]) =
            (authors.as_slice(), targets.as_slice(), bodies.as_slice())
        else {
            continue;
        };
        if !body.1.clickable
            || !body.1.long_clickable
            || !body.1.enabled
            || !body.1.valid_tap_geometry()
        {
            continue;
        }
        matches.push(BoundOwnedCommentRow {
            fingerprint: expected.clone(),
            row_node_index: row_index,
            author_node_index: author.0,
            reply_target_node_index: target.0,
            body_node_index: body.0,
            long_press_point: body.1.centre(),
        });
    }
    let presence = match matches.as_slice() {
        [found] => OwnedCommentPresence::Present(found.clone()),
        [] => OwnedCommentPresence::CardUnreadable,
        _ => OwnedCommentPresence::Ambiguous,
    };
    Ok(OwnedCommentObservation { detail, presence })
}

#[derive(Debug)]
pub struct OwnedCommentContinuityProof {
    catalog: CommentDeleteCatalogIdentity,
    device_udid: String,
    session_id: String,
    sequence: u64,
    detail: CommentDetailIdentity,
    fingerprint: OwnedCommentFingerprint,
    hierarchy_sha256: String,
    frame_sha256: String,
    row: BoundOwnedCommentRow,
    binding_sha256: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum CommentDeleteContinuityError {
    #[error("the owned-comment proof is incomplete")]
    WeakSource,
    #[error("the comment or card changed before the Delete row")]
    TargetChanged,
    #[error("the observations are stale or not immediately consecutive")]
    StaleSequence,
    #[error("the hierarchy or screenshot is not a valid measured snapshot")]
    InvalidSnapshot,
    #[error("the Delete control or readback belongs to another snapshot")]
    SnapshotMismatch,
}

fn proof_binding(proof: &OwnedCommentContinuityProof) -> String {
    binding_sha256(&[
        b"comment-delete-source-v1",
        proof.catalog_key(),
        proof.device_udid.as_bytes(),
        proof.session_id.as_bytes(),
        &proof.sequence.to_be_bytes(),
        proof.detail.post_id.as_bytes(),
        proof.detail.comment_id.as_bytes(),
        proof.fingerprint.author.as_bytes(),
        proof.fingerprint.reply_target.as_bytes(),
        proof.fingerprint.body_sha256.as_bytes(),
        proof.hierarchy_sha256.as_bytes(),
        proof.frame_sha256.as_bytes(),
        &proof.row.row_node_index.to_be_bytes(),
        &proof.row.author_node_index.to_be_bytes(),
        &proof.row.reply_target_node_index.to_be_bytes(),
        &proof.row.body_node_index.to_be_bytes(),
        &proof.row.long_press_point.x.to_bits().to_be_bytes(),
        &proof.row.long_press_point.y.to_bits().to_be_bytes(),
    ])
}

fn catalog_identity_key(identity: CommentDeleteCatalogIdentity) -> &'static [u8] {
    match identity {
        CommentDeleteCatalogIdentity::TrillEn3832 => b"com.ss.android.ugc.trill/en/38.3.2",
    }
}

impl OwnedCommentContinuityProof {
    fn catalog_key(&self) -> &'static [u8] {
        catalog_identity_key(self.catalog)
    }

    fn is_strong(&self) -> bool {
        self.catalog == CommentDeleteCatalogIdentity::TrillEn3832
            && exact_nonempty(&self.device_udid)
            && exact_nonempty(&self.session_id)
            && self.fingerprint.is_complete()
            && self.detail.post_id == self.fingerprint.post_id
            && self.detail.comment_id == self.fingerprint.comment_id
            && valid_sha256(&self.hierarchy_sha256)
            && valid_sha256(&self.frame_sha256)
            && self.row.fingerprint == self.fingerprint
            && self.binding_sha256 == proof_binding(self)
    }
}

#[allow(clippy::too_many_arguments)]
pub fn prove_owned_comment_snapshot(
    catalog: &CommentDeleteCatalog,
    device_udid: impl Into<String>,
    session_id: impl Into<String>,
    sequence: u64,
    detail_uri: &str,
    expected: OwnedCommentFingerprint,
    hierarchy_xml: &[u8],
    frame_png: &[u8],
) -> Result<OwnedCommentContinuityProof, CommentDeleteContinuityError> {
    let nodes = parse_hierarchy(hierarchy_xml)
        .map_err(|_| CommentDeleteContinuityError::InvalidSnapshot)?;
    let observation = observe_owned_comment(&nodes, catalog, detail_uri, &expected)
        .map_err(|_| CommentDeleteContinuityError::InvalidSnapshot)?;
    let row = match observation.presence {
        OwnedCommentPresence::Present(row) => row,
        OwnedCommentPresence::TargetChanged => {
            return Err(CommentDeleteContinuityError::TargetChanged)
        }
        OwnedCommentPresence::CardUnreadable | OwnedCommentPresence::Ambiguous => {
            return Err(CommentDeleteContinuityError::WeakSource)
        }
    };
    let mut proof = OwnedCommentContinuityProof {
        catalog: catalog.identity,
        device_udid: device_udid.into(),
        session_id: session_id.into(),
        sequence,
        detail: observation.detail,
        fingerprint: expected,
        hierarchy_sha256: hex_sha256(hierarchy_xml),
        frame_sha256: frame_sha256(frame_png)?,
        row,
        binding_sha256: String::new(),
    };
    proof.binding_sha256 = proof_binding(&proof);
    proof
        .is_strong()
        .then_some(proof)
        .ok_or(CommentDeleteContinuityError::WeakSource)
}

#[derive(Debug)]
pub struct CommentLongPressProof {
    source: OwnedCommentContinuityProof,
    long_press_point: TapPoint,
    source_binding_sha256: String,
}

impl CommentLongPressProof {
    pub fn long_press_point(&self) -> &TapPoint {
        &self.long_press_point
    }
}

pub fn bind_comment_long_press(
    expected: &OwnedCommentContinuityProof,
    reproved: OwnedCommentContinuityProof,
) -> Result<CommentLongPressProof, CommentDeleteContinuityError> {
    if !expected.is_strong() || !reproved.is_strong() {
        return Err(CommentDeleteContinuityError::WeakSource);
    }
    if expected.catalog != reproved.catalog
        || expected.device_udid != reproved.device_udid
        || expected.session_id != reproved.session_id
        || expected.detail != reproved.detail
        || expected.fingerprint != reproved.fingerprint
    {
        return Err(CommentDeleteContinuityError::TargetChanged);
    }
    if expected.sequence.checked_add(1) != Some(reproved.sequence) {
        return Err(CommentDeleteContinuityError::StaleSequence);
    }
    let long_press_point = reproved.row.long_press_point.clone();
    let source_binding_sha256 = reproved.binding_sha256.clone();
    Ok(CommentLongPressProof {
        source: reproved,
        long_press_point,
        source_binding_sha256,
    })
}

#[derive(Debug)]
pub struct MeasuredCommentDeleteControl {
    point: TapPoint,
    node_index: usize,
    catalog: CommentDeleteCatalogIdentity,
    device_udid: String,
    session_id: String,
    source_binding_sha256: String,
    long_press_sequence: u64,
    menu_sequence: u64,
    menu_hierarchy_sha256: String,
    menu_frame_sha256: String,
    binding_sha256: String,
}

fn control_binding(control: &MeasuredCommentDeleteControl) -> String {
    binding_sha256(&[
        b"comment-delete-control-v1",
        catalog_identity_key(control.catalog),
        control.device_udid.as_bytes(),
        control.session_id.as_bytes(),
        control.source_binding_sha256.as_bytes(),
        &control.long_press_sequence.to_be_bytes(),
        &control.menu_sequence.to_be_bytes(),
        control.menu_hierarchy_sha256.as_bytes(),
        control.menu_frame_sha256.as_bytes(),
        &control.node_index.to_be_bytes(),
        &control.point.x.to_bits().to_be_bytes(),
        &control.point.y.to_bits().to_be_bytes(),
    ])
}

impl MeasuredCommentDeleteControl {
    pub fn point(&self) -> &TapPoint {
        &self.point
    }

    pub fn node_index(&self) -> usize {
        self.node_index
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum CommentDeleteLocateError {
    #[error("the measured Delete label is absent")]
    Missing,
    #[error("more than one measured Delete label matches")]
    Ambiguous,
    #[error("the measured Delete row has invalid geometry or ancestry")]
    Invalid,
    #[error("the measured comment action sheet is not present")]
    WrongSheet,
    #[error("the hierarchy snapshot is malformed")]
    InvalidHierarchy,
    #[error("the screenshot snapshot is malformed")]
    InvalidFrame,
    #[error("the source proof is invalid")]
    WeakSource,
    #[error("the observations are stale or not immediately consecutive")]
    StaleSequence,
}

pub fn locate_comment_delete_control(
    hierarchy_xml: &[u8],
    frame_png: &[u8],
    long_press: &CommentLongPressProof,
    long_press_sequence: u64,
    menu_sequence: u64,
) -> Result<MeasuredCommentDeleteControl, CommentDeleteLocateError> {
    if !long_press.source.is_strong()
        || long_press.source_binding_sha256 != long_press.source.binding_sha256
    {
        return Err(CommentDeleteLocateError::WeakSource);
    }
    if long_press.source.sequence.checked_add(1) != Some(long_press_sequence)
        || long_press_sequence.checked_add(1) != Some(menu_sequence)
    {
        return Err(CommentDeleteLocateError::StaleSequence);
    }
    let nodes =
        parse_hierarchy(hierarchy_xml).map_err(|_| CommentDeleteLocateError::InvalidHierarchy)?;
    let menu_frame_sha256 =
        frame_sha256(frame_png).map_err(|_| CommentDeleteLocateError::InvalidFrame)?;
    let catalog = &MEASURED;
    let sheets: Vec<usize> = nodes
        .iter()
        .enumerate()
        .filter(|(_, node)| {
            node.package_name == catalog.package
                && node.content_description == catalog.sheet_description
                && node.resource_id.ends_with(catalog.sheet_id_suffix)
        })
        .map(|(index, _)| index)
        .collect();
    let sheet = unique(sheets).ok_or(CommentDeleteLocateError::WrongSheet)?;
    let labels: Vec<usize> = nodes
        .iter()
        .enumerate()
        .filter(|(index, node)| {
            has_ancestor(&nodes, *index, sheet)
                && node.package_name == catalog.package
                && node.class_name == "android.widget.TextView"
                && node.text == catalog.delete_label
                && node.resource_id.ends_with(catalog.delete_label_id_suffix)
        })
        .map(|(index, _)| index)
        .collect();
    let label = match labels.as_slice() {
        [index] => *index,
        [] => return Err(CommentDeleteLocateError::Missing),
        _ => return Err(CommentDeleteLocateError::Ambiguous),
    };
    let parent = nodes[label]
        .parent
        .ok_or(CommentDeleteLocateError::Invalid)?;
    let row = nodes.get(parent).ok_or(CommentDeleteLocateError::Invalid)?;
    if !has_ancestor(&nodes, parent, sheet)
        || row.package_name != catalog.package
        || !row.resource_id.ends_with(catalog.delete_parent_id_suffix)
        || !row.clickable
        || !row.enabled
        || !row.valid_tap_geometry()
    {
        return Err(CommentDeleteLocateError::Invalid);
    }
    let mut control = MeasuredCommentDeleteControl {
        point: row.centre(),
        node_index: parent,
        catalog: long_press.source.catalog,
        device_udid: long_press.source.device_udid.clone(),
        session_id: long_press.source.session_id.clone(),
        source_binding_sha256: long_press.source_binding_sha256.clone(),
        long_press_sequence,
        menu_sequence,
        menu_hierarchy_sha256: hex_sha256(hierarchy_xml),
        menu_frame_sha256,
        binding_sha256: String::new(),
    };
    control.binding_sha256 = control_binding(&control);
    Ok(control)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum CommentDeleteEffectBoundary {
    DeleteRowTap,
}

#[derive(Debug)]
pub struct ArmedCommentDeleteIntent {
    source: OwnedCommentContinuityProof,
    source_binding_sha256: String,
    long_press_sequence: u64,
    menu_sequence: u64,
    menu_hierarchy_sha256: String,
    menu_frame_sha256: String,
    delete_control: TapPoint,
    effect_boundary: CommentDeleteEffectBoundary,
}

impl ArmedCommentDeleteIntent {
    pub fn delete_control(&self) -> &TapPoint {
        &self.delete_control
    }

    pub fn effect_boundary(&self) -> CommentDeleteEffectBoundary {
        self.effect_boundary
    }

    pub fn catalog_identity(&self) -> CommentDeleteCatalogIdentity {
        self.source.catalog
    }

    pub fn device_udid(&self) -> &str {
        &self.source.device_udid
    }
}

pub fn arm_comment_delete_intent(
    long_press: CommentLongPressProof,
    delete: MeasuredCommentDeleteControl,
) -> Result<ArmedCommentDeleteIntent, CommentDeleteContinuityError> {
    if !long_press.source.is_strong()
        || long_press.source_binding_sha256 != long_press.source.binding_sha256
    {
        return Err(CommentDeleteContinuityError::WeakSource);
    }
    if delete.catalog != long_press.source.catalog
        || delete.device_udid != long_press.source.device_udid
        || delete.session_id != long_press.source.session_id
        || delete.source_binding_sha256 != long_press.source_binding_sha256
        || long_press.source.sequence.checked_add(1) != Some(delete.long_press_sequence)
        || delete.long_press_sequence.checked_add(1) != Some(delete.menu_sequence)
        || !valid_sha256(&delete.menu_hierarchy_sha256)
        || !valid_sha256(&delete.menu_frame_sha256)
        || delete.binding_sha256 != control_binding(&delete)
    {
        return Err(CommentDeleteContinuityError::SnapshotMismatch);
    }
    Ok(ArmedCommentDeleteIntent {
        source: long_press.source,
        source_binding_sha256: long_press.source_binding_sha256,
        long_press_sequence: delete.long_press_sequence,
        menu_sequence: delete.menu_sequence,
        menu_hierarchy_sha256: delete.menu_hierarchy_sha256,
        menu_frame_sha256: delete.menu_frame_sha256,
        delete_control: delete.point,
        effect_boundary: CommentDeleteEffectBoundary::DeleteRowTap,
    })
}

#[derive(Debug)]
pub struct CommentDeleteReadback {
    catalog: CommentDeleteCatalogIdentity,
    device_udid: String,
    session_id: String,
    source_binding_sha256: String,
    sequence: u64,
    hierarchy_sha256: String,
    frame_sha256: String,
    observation: OwnedCommentObservation,
}

pub fn read_comment_delete_snapshot(
    armed: &ArmedCommentDeleteIntent,
    sequence: u64,
    detail_uri: &str,
    hierarchy_xml: &[u8],
    frame_png: &[u8],
) -> Result<CommentDeleteReadback, CommentDeleteContinuityError> {
    if !armed.source.is_strong()
        || armed.source_binding_sha256 != armed.source.binding_sha256
        || !valid_sha256(&armed.menu_hierarchy_sha256)
        || !valid_sha256(&armed.menu_frame_sha256)
        || armed.source.sequence.checked_add(1) != Some(armed.long_press_sequence)
        || armed.long_press_sequence.checked_add(1) != Some(armed.menu_sequence)
    {
        return Err(CommentDeleteContinuityError::WeakSource);
    }
    if armed.menu_sequence.checked_add(1) != Some(sequence) {
        return Err(CommentDeleteContinuityError::StaleSequence);
    }
    let nodes = parse_hierarchy(hierarchy_xml)
        .map_err(|_| CommentDeleteContinuityError::InvalidSnapshot)?;
    let observation =
        observe_owned_comment(&nodes, &MEASURED, detail_uri, &armed.source.fingerprint)
            .map_err(|_| CommentDeleteContinuityError::InvalidSnapshot)?;
    Ok(CommentDeleteReadback {
        catalog: armed.source.catalog,
        device_udid: armed.source.device_udid.clone(),
        session_id: armed.source.session_id.clone(),
        source_binding_sha256: armed.source_binding_sha256.clone(),
        sequence,
        hierarchy_sha256: hex_sha256(hierarchy_xml),
        frame_sha256: frame_sha256(frame_png)?,
        observation,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum CommentDeletePostEffectVerdict {
    NotConfirmed,
    TargetChanged,
    Unreadable,
}

pub fn confirm_comment_absence(
    armed: &ArmedCommentDeleteIntent,
    readback: &CommentDeleteReadback,
) -> Result<CommentDeletePostEffectVerdict, CommentDeleteContinuityError> {
    if !armed.source.is_strong()
        || readback.catalog != armed.source.catalog
        || readback.device_udid != armed.source.device_udid
        || readback.session_id != armed.source.session_id
        || readback.source_binding_sha256 != armed.source_binding_sha256
        || !valid_sha256(&readback.hierarchy_sha256)
        || !valid_sha256(&readback.frame_sha256)
    {
        return Err(CommentDeleteContinuityError::SnapshotMismatch);
    }
    if armed.menu_sequence.checked_add(1) != Some(readback.sequence) {
        return Err(CommentDeleteContinuityError::StaleSequence);
    }
    if readback.observation.detail.post_id != armed.source.detail.post_id
        || readback.observation.detail.comment_id != armed.source.detail.comment_id
    {
        return Ok(CommentDeletePostEffectVerdict::TargetChanged);
    }
    Ok(match &readback.observation.presence {
        OwnedCommentPresence::Present(row) if row.fingerprint == armed.source.fingerprint => {
            CommentDeletePostEffectVerdict::NotConfirmed
        }
        OwnedCommentPresence::TargetChanged => CommentDeletePostEffectVerdict::TargetChanged,
        OwnedCommentPresence::Present(_)
        | OwnedCommentPresence::CardUnreadable
        | OwnedCommentPresence::Ambiguous => CommentDeletePostEffectVerdict::Unreadable,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const DETAIL_A: &str =
        "aweme://aweme/detail?id=7679311681721306389&cid=7681527250248090376&refer=comment_history";
    const DETAIL_B: &str = "aweme://aweme/detail?id=999&cid=888&refer=comment_history";
    const FRAME_PNG: &[u8] = b"\x89PNG\r\n\x1a\ncomment-frame";
    const PRESENT_XML: &str = r#"<hierarchy rotation="0">
  <node package="com.ss.android.ugc.trill" class="android.view.View" text="" content-desc="Video" resource-id="com.ss.android.ugc.trill:id/long_press_layout" clickable="false" long-clickable="false" enabled="true" bounds="[0,0][1080,1200]" />
  <node package="com.ss.android.ugc.trill" class="androidx.recyclerview.widget.RecyclerView" text="" content-desc="" resource-id="com.ss.android.ugc.trill:id/n6s" clickable="false" long-clickable="false" enabled="true" bounds="[0,1200][1080,2100]">
    <node package="com.ss.android.ugc.trill" class="android.widget.FrameLayout" text="" content-desc="" resource-id="com.ss.android.ugc.trill:id/il1" clickable="false" long-clickable="false" enabled="true" bounds="[0,1417][1080,1642]">
      <node package="com.ss.android.ugc.trill" class="android.widget.Button" text="Ghiền Đà Lạt Sương Sương" content-desc="" resource-id="com.ss.android.ugc.trill:id/title" clickable="true" long-clickable="false" enabled="true" bounds="[230,1430][800,1480]" />
      <node package="com.ss.android.ugc.trill" class="android.widget.Button" text="Mì Gánh 🍜🍜" content-desc="" resource-id="com.ss.android.ugc.trill:id/seu" clickable="true" long-clickable="false" enabled="true" bounds="[230,1480][800,1495]" />
      <node package="com.ss.android.ugc.trill" class="android.widget.TextView" text="ok nha" content-desc="" resource-id="com.ss.android.ugc.trill:id/d0h" clickable="true" long-clickable="true" enabled="true" bounds="[230,1496][1048,1557]" />
    </node>
  </node>
</hierarchy>"#;
    const OTHER_ROW_XML: &str = r#"<hierarchy rotation="0">
  <node package="com.ss.android.ugc.trill" class="android.view.View" text="" content-desc="Video" resource-id="com.ss.android.ugc.trill:id/long_press_layout" clickable="false" long-clickable="false" enabled="true" bounds="[0,0][1080,1200]" />
  <node package="com.ss.android.ugc.trill" class="androidx.recyclerview.widget.RecyclerView" text="" content-desc="" resource-id="com.ss.android.ugc.trill:id/n6s" clickable="false" long-clickable="false" enabled="true" bounds="[0,1200][1080,2100]">
    <node package="com.ss.android.ugc.trill" class="android.widget.FrameLayout" text="" content-desc="" resource-id="com.ss.android.ugc.trill:id/il1" clickable="false" long-clickable="false" enabled="true" bounds="[0,1417][1080,1642]">
      <node package="com.ss.android.ugc.trill" class="android.widget.Button" text="Another author" content-desc="" resource-id="com.ss.android.ugc.trill:id/title" clickable="true" long-clickable="false" enabled="true" bounds="[230,1430][800,1480]" />
      <node package="com.ss.android.ugc.trill" class="android.widget.Button" text="Another target" content-desc="" resource-id="com.ss.android.ugc.trill:id/seu" clickable="true" long-clickable="false" enabled="true" bounds="[230,1480][800,1495]" />
      <node package="com.ss.android.ugc.trill" class="android.widget.TextView" text="another comment" content-desc="" resource-id="com.ss.android.ugc.trill:id/d0h" clickable="true" long-clickable="true" enabled="true" bounds="[230,1496][1048,1557]" />
    </node>
  </node>
</hierarchy>"#;
    const EMPTY_VIRTUAL_LIST_XML: &str = r#"<hierarchy rotation="0">
  <node package="com.ss.android.ugc.trill" class="android.view.View" text="" content-desc="Video" resource-id="com.ss.android.ugc.trill:id/long_press_layout" clickable="false" long-clickable="false" enabled="true" bounds="[0,0][1080,1200]" />
  <node package="com.ss.android.ugc.trill" class="androidx.recyclerview.widget.RecyclerView" text="" content-desc="" resource-id="com.ss.android.ugc.trill:id/n6s" clickable="false" long-clickable="false" enabled="true" bounds="[0,1200][1080,2100]" />
</hierarchy>"#;
    const MENU_XML: &str = r#"<hierarchy rotation="0">
  <node package="com.ss.android.ugc.trill" class="android.widget.FrameLayout" text="" content-desc="Bottom sheet" resource-id="com.ss.android.ugc.trill:id/dox" clickable="false" long-clickable="false" enabled="true" bounds="[0,877][1080,2094]">
    <node package="com.ss.android.ugc.trill" class="android.widget.FrameLayout" text="" content-desc="" resource-id="com.ss.android.ugc.trill:id/cm4" clickable="true" long-clickable="false" enabled="true" bounds="[32,1474][1048,1627]">
      <node package="com.ss.android.ugc.trill" class="android.widget.TextView" text="Delete" content-desc="" resource-id="com.ss.android.ugc.trill:id/cm5" clickable="false" long-clickable="false" enabled="true" bounds="[148,1524][270,1574]" />
    </node>
  </node>
</hierarchy>"#;

    fn catalog() -> &'static CommentDeleteCatalog {
        comment_delete_catalog(PACKAGE, "en-US", VERSION).unwrap()
    }

    fn fingerprint(detail_uri: &str) -> OwnedCommentFingerprint {
        OwnedCommentFingerprint::from_body(
            parse_comment_detail_identity(detail_uri).unwrap(),
            "Ghiền Đà Lạt Sương Sương",
            "Mì Gánh 🍜🍜",
            "ok nha",
        )
    }

    fn proof_on(
        sequence: u64,
        detail_uri: &str,
        device_udid: &str,
        session_id: &str,
    ) -> OwnedCommentContinuityProof {
        prove_owned_comment_snapshot(
            catalog(),
            device_udid,
            session_id,
            sequence,
            detail_uri,
            fingerprint(detail_uri),
            PRESENT_XML.as_bytes(),
            FRAME_PNG,
        )
        .unwrap()
    }

    fn proof(sequence: u64, detail_uri: &str) -> OwnedCommentContinuityProof {
        proof_on(sequence, detail_uri, "device-1", "session-1")
    }

    fn long_press(detail_uri: &str) -> CommentLongPressProof {
        bind_comment_long_press(&proof(1, detail_uri), proof(2, detail_uri)).unwrap()
    }

    fn armed() -> ArmedCommentDeleteIntent {
        let long_press = long_press(DETAIL_A);
        let control =
            locate_comment_delete_control(MENU_XML.as_bytes(), FRAME_PNG, &long_press, 3, 4)
                .unwrap();
        arm_comment_delete_intent(long_press, control).unwrap()
    }

    #[test]
    fn hierarchy_parser_rejects_oversized_input() {
        let oversized = vec![b' '; MAX_HIERARCHY_BYTES + 1];
        assert_eq!(
            parse_hierarchy(&oversized).unwrap_err(),
            CommentDeleteSnapshotError::InvalidHierarchy
        );
    }

    #[test]
    fn hierarchy_parser_rejects_excessive_depth() {
        let node = r#"<node class="android.view.View" bounds="[0,0][1,1]">"#;
        let mut hierarchy = String::from("<hierarchy>");
        hierarchy.push_str(&node.repeat(MAX_HIERARCHY_DEPTH + 1));
        hierarchy.push_str(&"</node>".repeat(MAX_HIERARCHY_DEPTH + 1));
        hierarchy.push_str("</hierarchy>");
        assert_eq!(
            parse_hierarchy(hierarchy.as_bytes()).unwrap_err(),
            CommentDeleteSnapshotError::InvalidHierarchy
        );
    }

    #[test]
    fn hierarchy_parser_rejects_excessive_node_count() {
        let node = r#"<node class="android.view.View" bounds="[0,0][1,1]"/>"#;
        let hierarchy = format!(
            "<hierarchy>{}</hierarchy>",
            node.repeat(MAX_HIERARCHY_NODES + 1)
        );
        assert_eq!(
            parse_hierarchy(hierarchy.as_bytes()).unwrap_err(),
            CommentDeleteSnapshotError::InvalidHierarchy
        );
    }

    #[test]
    fn catalog_and_detail_are_exactly_pinned() {
        assert_eq!(
            catalog().identity(),
            CommentDeleteCatalogIdentity::TrillEn3832
        );
        assert_eq!(catalog().package(), PACKAGE);
        assert_eq!(catalog().language(), LANGUAGE);
        assert_eq!(catalog().version(), VERSION);
        assert!(comment_delete_catalog(PACKAGE, "vi", VERSION).is_none());
        assert!(comment_delete_catalog(PACKAGE, "en", "38.3.3").is_none());
        assert!(comment_delete_catalog("com.zhiliaoapp.musically", "en", VERSION).is_none());
        assert!(parse_comment_detail_identity("aweme://aweme/detail?id=1").is_none());
        assert!(parse_comment_detail_identity("aweme://aweme/detail?id=1&id=2&cid=3").is_none());
        assert!(parse_comment_detail_identity("aweme://aweme/detail?id=a&cid=3").is_none());
    }

    #[test]
    fn exact_xml_and_frame_bytes_create_an_opaque_source_proof() {
        let source = proof(1, DETAIL_A);
        assert!(source.is_strong());
        assert_eq!(source.hierarchy_sha256, hex_sha256(PRESENT_XML.as_bytes()));
        assert_eq!(source.frame_sha256, hex_sha256(FRAME_PNG));
        assert_eq!(source.row.row_node_index, 2);
        assert_eq!(source.row.author_node_index, 3);
        assert_eq!(source.row.reply_target_node_index, 4);
        assert_eq!(source.row.body_node_index, 5);
        assert_eq!(
            (source.row.long_press_point.x, source.row.long_press_point.y),
            (639.0, 1526.5)
        );
        assert_eq!(
            prove_owned_comment_snapshot(
                catalog(),
                "device-1",
                "session-1",
                1,
                DETAIL_A,
                fingerprint(DETAIL_A),
                OTHER_ROW_XML.as_bytes(),
                FRAME_PNG
            )
            .unwrap_err(),
            CommentDeleteContinuityError::WeakSource
        );
        assert_eq!(
            prove_owned_comment_snapshot(
                catalog(),
                "device-1",
                "session-1",
                1,
                DETAIL_A,
                fingerprint(DETAIL_A),
                PRESENT_XML.as_bytes(),
                b"not-png"
            )
            .unwrap_err(),
            CommentDeleteContinuityError::InvalidSnapshot
        );
    }

    #[test]
    fn source_proof_cannot_be_mixed_or_fabricated() {
        assert_eq!(
            bind_comment_long_press(
                &proof_on(1, DETAIL_A, "device-1", "session-1"),
                proof_on(2, DETAIL_A, "device-2", "session-1")
            )
            .unwrap_err(),
            CommentDeleteContinuityError::TargetChanged
        );
        assert_eq!(
            bind_comment_long_press(
                &proof_on(1, DETAIL_A, "device-1", "session-1"),
                proof_on(2, DETAIL_A, "device-1", "session-2")
            )
            .unwrap_err(),
            CommentDeleteContinuityError::TargetChanged
        );
        let mut fabricated = proof(2, DETAIL_A);
        fabricated.hierarchy_sha256 = "f".repeat(64);
        assert_eq!(
            bind_comment_long_press(&proof(1, DETAIL_A), fabricated).unwrap_err(),
            CommentDeleteContinuityError::WeakSource
        );
    }

    #[test]
    fn continuity_requires_exact_consecutive_a_to_a_sequences() {
        assert!(bind_comment_long_press(&proof(1, DETAIL_A), proof(2, DETAIL_A)).is_ok());
        assert_eq!(
            bind_comment_long_press(&proof(1, DETAIL_A), proof(2, DETAIL_B)).unwrap_err(),
            CommentDeleteContinuityError::TargetChanged
        );
        assert_eq!(
            bind_comment_long_press(&proof(1, DETAIL_A), proof(3, DETAIL_A)).unwrap_err(),
            CommentDeleteContinuityError::StaleSequence
        );
        assert_eq!(
            bind_comment_long_press(&proof(u64::MAX, DETAIL_A), proof(u64::MAX, DETAIL_A))
                .unwrap_err(),
            CommentDeleteContinuityError::StaleSequence
        );
    }

    #[test]
    fn delete_control_is_parent_aware_and_bound_to_the_source() {
        let long_press = long_press(DETAIL_A);
        let found =
            locate_comment_delete_control(MENU_XML.as_bytes(), FRAME_PNG, &long_press, 3, 4)
                .unwrap();
        assert_eq!(found.node_index(), 1);
        assert_eq!((found.point().x, found.point().y), (540.0, 1550.5));
        assert_eq!(found.menu_hierarchy_sha256, hex_sha256(MENU_XML.as_bytes()));
        assert_eq!(found.menu_frame_sha256, hex_sha256(FRAME_PNG));
        let wrong_package = MENU_XML.replace(PACKAGE, "com.example.other");
        assert_eq!(
            locate_comment_delete_control(wrong_package.as_bytes(), FRAME_PNG, &long_press, 3, 4,)
                .unwrap_err(),
            CommentDeleteLocateError::WrongSheet
        );
        assert_eq!(
            locate_comment_delete_control(MENU_XML.as_bytes(), FRAME_PNG, &long_press, 3, 5,)
                .unwrap_err(),
            CommentDeleteLocateError::StaleSequence
        );
        assert_eq!(
            locate_comment_delete_control(MENU_XML.as_bytes(), b"not-png", &long_press, 3, 4)
                .unwrap_err(),
            CommentDeleteLocateError::InvalidFrame
        );
    }

    #[test]
    fn arming_rejects_a_control_from_another_source_or_snapshot() {
        let source_a = long_press(DETAIL_A);
        let source_b = long_press(DETAIL_B);
        let control_b =
            locate_comment_delete_control(MENU_XML.as_bytes(), FRAME_PNG, &source_b, 3, 4).unwrap();
        assert_eq!(
            arm_comment_delete_intent(source_a, control_b).unwrap_err(),
            CommentDeleteContinuityError::SnapshotMismatch
        );
        let source = long_press(DETAIL_A);
        let mut fabricated =
            locate_comment_delete_control(MENU_XML.as_bytes(), FRAME_PNG, &source, 3, 4).unwrap();
        fabricated.menu_hierarchy_sha256 = "f".repeat(64);
        assert_eq!(
            arm_comment_delete_intent(source, fabricated).unwrap_err(),
            CommentDeleteContinuityError::SnapshotMismatch
        );
    }

    #[test]
    fn arming_names_the_delete_row_tap_as_the_effect_boundary() {
        let armed = armed();
        assert_eq!(
            armed.effect_boundary(),
            CommentDeleteEffectBoundary::DeleteRowTap
        );
        assert_eq!(
            armed.catalog_identity(),
            CommentDeleteCatalogIdentity::TrillEn3832
        );
        assert_eq!(armed.device_udid(), "device-1");
        assert_eq!(
            (armed.delete_control().x, armed.delete_control().y),
            (540.0, 1550.5)
        );
    }

    #[test]
    fn virtualized_history_absence_never_becomes_deleted() {
        for hierarchy in [OTHER_ROW_XML.as_bytes(), EMPTY_VIRTUAL_LIST_XML.as_bytes()] {
            let armed = armed();
            let readback =
                read_comment_delete_snapshot(&armed, 5, DETAIL_A, hierarchy, FRAME_PNG).unwrap();
            assert_eq!(
                confirm_comment_absence(&armed, &readback).unwrap(),
                CommentDeletePostEffectVerdict::Unreadable
            );
        }
    }

    #[test]
    fn readback_only_distinguishes_present_changed_and_unreadable() {
        let armed = armed();
        let present =
            read_comment_delete_snapshot(&armed, 5, DETAIL_A, PRESENT_XML.as_bytes(), FRAME_PNG)
                .unwrap();
        assert_eq!(
            confirm_comment_absence(&armed, &present).unwrap(),
            CommentDeletePostEffectVerdict::NotConfirmed
        );
        let changed =
            read_comment_delete_snapshot(&armed, 5, DETAIL_B, OTHER_ROW_XML.as_bytes(), FRAME_PNG)
                .unwrap();
        assert_eq!(
            confirm_comment_absence(&armed, &changed).unwrap(),
            CommentDeletePostEffectVerdict::TargetChanged
        );
        assert_eq!(
            read_comment_delete_snapshot(&armed, 6, DETAIL_A, PRESENT_XML.as_bytes(), FRAME_PNG)
                .unwrap_err(),
            CommentDeleteContinuityError::StaleSequence
        );
    }

    #[test]
    fn fabricated_readback_is_rejected() {
        let armed = armed();
        let mut readback =
            read_comment_delete_snapshot(&armed, 5, DETAIL_A, PRESENT_XML.as_bytes(), FRAME_PNG)
                .unwrap();
        readback.source_binding_sha256 = "f".repeat(64);
        assert_eq!(
            confirm_comment_absence(&armed, &readback).unwrap_err(),
            CommentDeleteContinuityError::SnapshotMismatch
        );
    }
}
