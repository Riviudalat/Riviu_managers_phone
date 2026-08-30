//! Deterministic input planning for image publishing campaigns.
//!
//! This module deliberately stops at a validated, hashed manifest. Device
//! transfer and TikTok UI work consume this manifest later, so importing a
//! folder never has a side effect on a phone.

use std::cmp::Ordering;
use std::fs;
use std::path::Path;

use anyhow::{bail, Context};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// TikTok's own ceiling for a photo carousel.
///
/// **This used to be 11, and 11 was never a property of TikTok or of scanning.** It was the
/// iOS pixel composer's 3x4 tap grid — twelve cells, so a twelfth image would have indexed
/// `grid_y[4]` and panicked — expressed in the one place that could not know that. The cost
/// showed up the first time an operator pointed the scanner at a real batch: `scan_bundle`
/// returns `TooManyImages` and `scan_publish_folder` propagates it, so **one** thirteen-slide
/// folder made all twenty-one unscannable, with an error naming a post the operator had not
/// selected.
///
/// The composer's limit now lives with the grid that produces it
/// (`publish_commands::IOS_PIXEL_GRID_MAX_IMAGES`), and is checked before media leaves the
/// desktop rather than after it has been imported onto a phone.
const DEFAULT_MAX_IMAGES: usize = 35;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum PublishMediaKind {
    Image,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum PublishVisibility {
    Public,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum PublishCleanupPolicy {
    DeleteImportedAssetsAfterVerified,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum PublishCampaignState {
    Queued,
    Scheduled,
    Preparing,
    Ready,
    Transferring,
    Imported,
    Posting,
    Verifying,
    Succeeded,
    FailedBeforeDispatch,
    Uncertain,
    Cancelled,
    Missed,
}

impl PublishCampaignState {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Scheduled => "scheduled",
            Self::Preparing => "preparing",
            Self::Ready => "ready",
            Self::Transferring => "transferring",
            Self::Imported => "imported",
            Self::Posting => "posting",
            Self::Verifying => "verifying",
            Self::Succeeded => "succeeded",
            Self::FailedBeforeDispatch => "failed_before_dispatch",
            Self::Uncertain => "uncertain",
            Self::Cancelled => "cancelled",
            Self::Missed => "missed",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PublishCampaignRequest {
    pub request_id: String,
    pub source_root: String,
    pub bundle_ids: Vec<String>,
    pub udids: Vec<String>,
    pub run_at: Option<String>,
    pub visibility: PublishVisibility,
    pub cleanup_policy: PublishCleanupPolicy,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PublishAssignmentPlan {
    pub bundle_id: String,
    pub udid: String,
    pub ordinal: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PublishCampaignRecord {
    pub id: String,
    pub request_id: String,
    pub source_root: String,
    pub state: PublishCampaignState,
    pub run_at: Option<String>,
    pub visibility: PublishVisibility,
    pub cleanup_policy: PublishCleanupPolicy,
    pub assignments: Vec<PublishAssignmentPlan>,
    pub created_at: String,
    pub updated_at: String,
    pub error_code: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PublishAssignmentRecord {
    pub id: String,
    pub campaign_id: String,
    pub bundle_id: String,
    pub ordinal: u32,
    pub udid: String,
    pub state: PublishCampaignState,
    pub effect_intent: Option<String>,
    pub evidence_json: Option<String>,
    pub error_code: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PublishCampaignDetail {
    pub campaign: PublishCampaignRecord,
    pub bundles: Vec<PublishBundle>,
    pub assignments: Vec<PublishAssignmentRecord>,
    pub events: Vec<PublishEventRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PublishEventRecord {
    pub revision: u64,
    pub kind: String,
    pub payload_json: String,
    pub created_at: String,
}

#[derive(Debug, thiserror::Error)]
pub enum PublishPlanError {
    #[error("bundle selection and device selection must have the same non-zero length")]
    MappingLength,
    #[error("bundle selection contains duplicate id: {0}")]
    DuplicateBundle(String),
    #[error("device selection contains duplicate UDID: {0}")]
    DuplicateUdid(String),
    #[error("bundle id is empty")]
    EmptyBundleId,
    #[error("UDID is empty")]
    EmptyUdid,
}

/// Pair each bundle with the phone that will post it, or refuse.
///
/// **This is the only thing standing between bundle *i* and phone *j*, and it had no tests.**
/// The frontend test for the same pairing states the cost plainly: one account posting another
/// account's photographs under another account's caption, with no discrepancy to notice
/// afterwards and no delete path to undo it. That is the failure this function exists to
/// prevent, so the property worth pinning is not "it returns a Vec" but that **position is
/// identity**: the nth bundle goes to the nth phone, and `ordinal` records that n.
///
/// Everything it refuses, it refuses for the same reason: each one is a way for the mapping to
/// stop being a bijection between two lists read in parallel. Different lengths, an empty entry
/// on either side, or a repeat on either side all mean the operator's intent cannot be read off
/// the two lists with confidence -- and a publish is not the place to guess.
///
/// Empty is refused rather than treated as "nothing to do", because reaching here with no
/// selection means a caller lost the selection somewhere, and returning `Ok(vec![])` would turn
/// that into a campaign that silently posts nothing.
pub fn validate_publish_mapping(
    bundle_ids: &[String],
    udids: &[String],
) -> Result<Vec<PublishAssignmentPlan>, PublishPlanError> {
    if bundle_ids.is_empty() || bundle_ids.len() != udids.len() {
        return Err(PublishPlanError::MappingLength);
    }
    // **Deduplicated on the trimmed form, because that is what the emptiness check already
    // treats as the identity.** Rejecting `"   "` as empty says the surrounding whitespace is
    // not part of the id; deduplicating on the raw string then said the opposite, so
    // `"bundle-1"` and `"bundle-1 "` both passed as distinct bundles -- two phones posting one
    // bundle, which is the exact class of mis-pairing this function exists to refuse. Two
    // adjacent lines disagreeing about what a value *is* is a bug even when neither line is
    // wrong on its own.
    let mut seen_bundles = std::collections::BTreeSet::new();
    let mut seen_udids = std::collections::BTreeSet::new();
    for bundle_id in bundle_ids {
        let key = bundle_id.trim();
        if key.is_empty() {
            return Err(PublishPlanError::EmptyBundleId);
        }
        if !seen_bundles.insert(key) {
            return Err(PublishPlanError::DuplicateBundle(bundle_id.clone()));
        }
    }
    for udid in udids {
        let key = udid.trim();
        if key.is_empty() {
            return Err(PublishPlanError::EmptyUdid);
        }
        if !seen_udids.insert(key) {
            return Err(PublishPlanError::DuplicateUdid(udid.clone()));
        }
    }
    Ok(bundle_ids
        .iter()
        .zip(udids)
        .enumerate()
        .map(|(index, (bundle_id, udid))| PublishAssignmentPlan {
            bundle_id: bundle_id.clone(),
            udid: udid.clone(),
            ordinal: index as u32,
        })
        .collect())
}

/// What one auto-assignment dealt.
///
/// No cursor. An earlier version handed one back for the caller to persist, and the position
/// it encoded turned out to be a worse answer than the history it stood in for — see
/// [`auto_assign_bundles`]. Nothing has to be stored between runs now: the bundle ids that
/// already have assignments *are* the state.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AutoAssignment {
    pub plan: Vec<PublishAssignmentPlan>,
}

/// Why a deal was refused. None of these degrades into a smaller run.
///
/// Derives match [`PublishPlanError`], which it wraps: `Debug` and `Error`, nothing more.
/// Callers match on the shape rather than compare values.
#[derive(Debug, thiserror::Error)]
pub enum PublishAssignError {
    #[error("cần {wanted} máy rảnh, chỉ có {available}")]
    NotEnoughPhones { wanted: usize, available: usize },
    #[error("cần {wanted} bài khác nhau, thư mục chỉ có {available}")]
    NotEnoughBundles { wanted: usize, available: usize },
    /// The folder holds enough posts; not enough of them are **unpublished**.
    ///
    /// Kept apart from `NotEnoughBundles` because the operator's next move differs: this one
    /// means the run is finished, not that the folder is short.
    #[error("cần {wanted} bài chưa đăng, chỉ còn {available} bài chưa đăng")]
    NotEnoughFreshBundles { wanted: usize, available: usize },
    /// Two folders claim the same id, which is a scanner defect rather than a short run.
    #[error("hai thư mục cùng mang id {0} — một trong hai sắp bị đăng dưới caption của cái kia")]
    DuplicateBundleInSource(String),
    #[error("không chia được 0 bài")]
    Empty,
    #[error(transparent)]
    Mapping(#[from] PublishPlanError),
}

/// Deal `wanted` **not-yet-published** bundles onto `wanted` phones, one each.
///
/// **It ends by calling [`validate_publish_mapping`], and that is the point.** This function
/// only *chooses*; the bijection between a bundle and a phone stays enforced in the one place
/// that already earned nine tests for it. A second implementation of the pairing would be a
/// second thing to get wrong in the one function whose job is to stop one account's
/// photographs going out under another account's caption.
///
/// # Why the history, and not a cursor
///
/// The first version took a rotating cursor: deal `wanted` from position `cursor`, hand back
/// `cursor + wanted`, and two consecutive runs were disjoint. A review took it apart, and the
/// arithmetic was a **proxy for a fact** — which post has already gone out — that is worse
/// than the fact in three separate ways:
///
/// * with five bundles and `wanted = 5`, `next_cursor` was `5`, `5 % 5 == 0`, and the second
///   run dealt **exactly the same five**. The promise held only while the inventory was at
///   least twice the deal, and nothing said so;
/// * a duplicate id anywhere outside the selected window slipped past
///   [`validate_publish_mapping`], which only ever sees the chosen subset — so `["A","B","C","A"]`
///   dealt `A` twice across two runs;
/// * the cursor is positional. Inserting one folder ahead of it shifts every later window, and
///   `wrapping_add` at `u64::MAX` lands on a residue that is not the circular successor.
///
/// So the caller passes the bundle ids it has **already published**, and the pool is what is
/// left. That is keyed by identity rather than position, which makes all three of those
/// impossible rather than unlikely — and it is the thing the operator actually means.
///
/// # Refusing rather than dealing fewer
///
/// [`PublishAssignError::NotEnoughFreshBundles`] is the variant that matters, and it fires
/// when the *remaining* pool is short, not the whole folder. The operator asked for
/// *different* posts; reusing one puts the same carousel on two live accounts, which is the
/// shape a bot farm has and a person does not. Dealing four when five were asked for is a
/// silent change of plan, so too few phones refuses as well.
pub fn auto_assign_bundles(
    bundle_ids: &[String],
    already_published: &[String],
    udids: &[String],
    wanted: usize,
) -> Result<AutoAssignment, PublishAssignError> {
    if wanted == 0 {
        return Err(PublishAssignError::Empty);
    }
    if udids.len() < wanted {
        return Err(PublishAssignError::NotEnoughPhones {
            wanted,
            available: udids.len(),
        });
    }
    // A duplicate in the *source* is a scanner defect, and refusing it here is cheap insurance:
    // two folders claiming one id mean one of them is about to be published under the other's
    // caption, and nothing downstream would notice.
    let mut seen = std::collections::BTreeSet::new();
    for id in bundle_ids {
        if !seen.insert(id.as_str()) {
            return Err(PublishAssignError::DuplicateBundleInSource(id.clone()));
        }
    }

    let published: std::collections::BTreeSet<&str> =
        already_published.iter().map(String::as_str).collect();
    let fresh: Vec<String> = bundle_ids
        .iter()
        .filter(|id| !published.contains(id.as_str()))
        .cloned()
        .collect();
    if fresh.len() < wanted {
        return Err(PublishAssignError::NotEnoughFreshBundles {
            wanted,
            available: fresh.len(),
        });
    }

    let chosen_bundles: Vec<String> = fresh.into_iter().take(wanted).collect();
    // The phones as given: the caller filtered them to idle and eligible, and their order is
    // the operator's fleet order, which is the order the mapping preview renders.
    let chosen_udids: Vec<String> = udids.iter().take(wanted).cloned().collect();

    let plan = validate_publish_mapping(&chosen_bundles, &chosen_udids)?;
    Ok(AutoAssignment { plan })
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PublishImage {
    pub path: String,
    pub file_name: String,
    pub order: u32,
    pub sha256: String,
    pub byte_len: u64,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PublishBundle {
    pub id: String,
    pub source_path: String,
    pub name: String,
    pub media_kind: PublishMediaKind,
    pub images: Vec<PublishImage>,
    pub caption_path: String,
    pub caption: String,
    pub caption_sha256: String,
    pub total_bytes: u64,
    /// Partner names from the bundle's own `partners-*.xlsx`, in workbook order.
    ///
    /// Read at scan time and carried on the bundle, because post time is too late to go
    /// looking: the sheet row is queued in the same transaction that records the post, and
    /// a re-glob there would read whatever the folder holds *then*, not what the operator
    /// scanned and approved. `default` because manifests written before 31/08/2026 do not
    /// carry the field, and a bundle without a workbook legitimately has none — both read
    /// back as an empty list.
    #[serde(default)]
    pub partners: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum PublishScanSeverity {
    Warning,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PublishScanNotice {
    pub severity: PublishScanSeverity,
    pub path: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PublishFolderManifest {
    pub source_root: String,
    pub scanned_at: DateTime<Utc>,
    pub bundles: Vec<PublishBundle>,
    pub notices: Vec<PublishScanNotice>,
    pub ignored_partner_files: usize,
    pub ignored_hidden_files: usize,
}

#[derive(Debug, Clone, Copy)]
pub struct PublishScanOptions {
    pub max_images_per_bundle: usize,
}

impl Default for PublishScanOptions {
    fn default() -> Self {
        Self {
            max_images_per_bundle: DEFAULT_MAX_IMAGES,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum PublishScanError {
    #[error("publish folder does not exist: {0}")]
    MissingRoot(String),
    #[error("publish path is not a directory: {0}")]
    RootNotDirectory(String),
    #[error("bundle {bundle} has no supported images")]
    EmptyBundle { bundle: String },
    #[error("publish folder has no bundle directories")]
    NoBundles,
    #[error("publish folder contains duplicate bundle id: {0}")]
    DuplicateBundleId(String),
    #[error("bundle {bundle} has no caption*.txt file")]
    MissingCaption { bundle: String },
    #[error("bundle {bundle} has more than one caption*.txt file")]
    MultipleCaptions { bundle: String },
    #[error("bundle {bundle} has {count} images; maximum is {max}")]
    TooManyImages {
        bundle: String,
        count: usize,
        max: usize,
    },
    #[error("bundle {bundle} image order is invalid: {message}")]
    InvalidImageOrder { bundle: String, message: String },
    #[error("cannot read {path}: {source}")]
    Io {
        path: String,
        source: std::io::Error,
    },
    #[error("cannot decode image {path}: {source}")]
    Image {
        path: String,
        source: image::ImageError,
    },
    #[error("cannot hash {path}: {source}")]
    Hash {
        path: String,
        source: std::io::Error,
    },
    #[error("cannot read caption {path}: {source}")]
    Caption {
        path: String,
        source: std::io::Error,
    },
    #[error("caption is not valid UTF-8: {path}")]
    InvalidCaptionEncoding { path: String },
}

/// Scan one folder level and produce a stable, side-effect-free manifest.
pub fn scan_publish_folder(
    root: impl AsRef<Path>,
    options: PublishScanOptions,
) -> Result<PublishFolderManifest, PublishScanError> {
    let root = root.as_ref();
    if !root.exists() {
        return Err(PublishScanError::MissingRoot(root.display().to_string()));
    }
    if !root.is_dir() {
        return Err(PublishScanError::RootNotDirectory(
            root.display().to_string(),
        ));
    }

    let mut entries = read_dir_sorted(root)?;
    let mut bundles = Vec::new();
    let mut notices = Vec::new();
    let mut ignored_partner_files = 0;
    let mut ignored_hidden_files = 0;

    for entry in entries.drain(..) {
        let path = entry.path();
        if path.is_dir() {
            match scan_bundle(&path, options) {
                Ok((bundle, bundle_notices, partners, hidden)) => {
                    if bundles
                        .iter()
                        .any(|existing: &PublishBundle| existing.id == bundle.id)
                    {
                        return Err(PublishScanError::DuplicateBundleId(bundle.id));
                    }
                    bundles.push(bundle);
                    notices.extend(bundle_notices);
                    ignored_partner_files += partners;
                    ignored_hidden_files += hidden;
                }
                Err(error) => return Err(error),
            }
            continue;
        }

        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('.') {
            ignored_hidden_files += 1;
        } else if is_partner_file(&name) {
            ignored_partner_files += 1;
        } else {
            notices.push(PublishScanNotice {
                severity: PublishScanSeverity::Warning,
                path: path.display().to_string(),
                message: "file ở thư mục gốc bị bỏ qua; đặt vào một thư mục bài đăng".into(),
            });
        }
    }

    if bundles.is_empty() {
        return Err(PublishScanError::NoBundles);
    }

    Ok(PublishFolderManifest {
        source_root: root.display().to_string(),
        scanned_at: Utc::now(),
        bundles,
        notices,
        ignored_partner_files,
        ignored_hidden_files,
    })
}

fn scan_bundle(
    path: &Path,
    options: PublishScanOptions,
) -> Result<(PublishBundle, Vec<PublishScanNotice>, usize, usize), PublishScanError> {
    let bundle_name = path
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| path.display().to_string());
    let mut files = read_dir_sorted(path)?;
    let mut images = Vec::new();
    let mut captions = Vec::new();
    let mut partner_files: Vec<std::path::PathBuf> = Vec::new();
    let mut notices = Vec::new();
    let mut ignored_partner_files = 0;
    let mut ignored_hidden_files = 0;

    for entry in files.drain(..) {
        let file_path = entry.path();
        if file_path.is_dir() {
            notices.push(PublishScanNotice {
                severity: PublishScanSeverity::Warning,
                path: file_path.display().to_string(),
                message: "thư mục lồng bên trong bundle bị bỏ qua".into(),
            });
            continue;
        }
        let file_name = entry.file_name().to_string_lossy().to_string();
        if file_name.starts_with('.') {
            ignored_hidden_files += 1;
        } else if is_partner_file(&file_name) {
            // Still counted as "not an image or caption" — the manifest field keeps its
            // meaning — but no longer discarded: the path is kept so the names can ride
            // the bundle.
            ignored_partner_files += 1;
            partner_files.push(file_path);
        } else if is_caption_file(&file_name) {
            captions.push(file_path);
        } else if is_supported_image(&file_name) {
            images.push(file_path);
        } else {
            notices.push(PublishScanNotice {
                severity: PublishScanSeverity::Warning,
                path: file_path.display().to_string(),
                message: "file không thuộc ảnh/caption nên bị bỏ qua".into(),
            });
        }
    }

    if images.is_empty() {
        return Err(PublishScanError::EmptyBundle {
            bundle: bundle_name,
        });
    }
    if images.len() > options.max_images_per_bundle {
        return Err(PublishScanError::TooManyImages {
            bundle: bundle_name,
            count: images.len(),
            max: options.max_images_per_bundle,
        });
    }
    if captions.is_empty() {
        return Err(PublishScanError::MissingCaption {
            bundle: bundle_name,
        });
    }
    if captions.len() > 1 {
        return Err(PublishScanError::MultipleCaptions {
            bundle: bundle_name,
        });
    }

    let mut ordered = Vec::with_capacity(images.len());
    for (expected, image_path) in images.into_iter().enumerate() {
        let order =
            numeric_prefix(&image_path).ok_or_else(|| PublishScanError::InvalidImageOrder {
                bundle: bundle_name.clone(),
                message: format!("{} thiếu tiền tố số", image_path.display()),
            })?;
        let expected_order = (expected + 1) as u32;
        if order != expected_order {
            return Err(PublishScanError::InvalidImageOrder {
                bundle: bundle_name.clone(),
                message: format!("mong đợi {expected_order:02}, thấy {order:02}"),
            });
        }
        let metadata =
            image::image_dimensions(&image_path).map_err(|source| PublishScanError::Image {
                path: image_path.display().to_string(),
                source,
            })?;
        let bytes = read_file_retry(&image_path).map_err(|source| PublishScanError::Io {
            path: image_path.display().to_string(),
            source,
        })?;
        ordered.push(PublishImage {
            path: image_path.display().to_string(),
            file_name: image_path
                .file_name()
                .map(|value| value.to_string_lossy().to_string())
                .unwrap_or_default(),
            order,
            sha256: sha256_bytes(&bytes),
            byte_len: bytes.len() as u64,
            width: metadata.0,
            height: metadata.1,
        });
    }

    let caption_path = captions.pop().expect("caption checked above");
    let caption_bytes =
        read_file_retry(&caption_path).map_err(|source| PublishScanError::Caption {
            path: caption_path.display().to_string(),
            source,
        })?;
    let caption = String::from_utf8(caption_bytes)
        .map_err(|_| PublishScanError::InvalidCaptionEncoding {
            path: caption_path.display().to_string(),
        })?
        .replace("\r\n", "\n")
        .replace('\r', "\n")
        .trim_end_matches('\n')
        .to_string();
    if caption.trim().is_empty() {
        notices.push(PublishScanNotice {
            severity: PublishScanSeverity::Warning,
            path: caption_path.display().to_string(),
            message: "caption rỗng; bài sẽ bị chặn ở bước preview".into(),
        });
    }
    let total_bytes = ordered.iter().map(|image| image.byte_len).sum();
    let bundle_id = format!("{}-{}", slug(&bundle_name), short_hash(&ordered, &caption));

    // **Partner names are read here or never.** Exactly one workbook is believed; two is a
    // question for the operator, not a guess (the sheet writes these names across live
    // columns). An unreadable one degrades to an empty list with a notice — the names
    // decorate the sheet row, and decoration must never block a scan.
    let partners = match partner_files.as_slice() {
        [] => Vec::new(),
        [only] => match crate::publish_partners::read_partner_row(only) {
            Ok(row) => row.names,
            Err(error) => {
                notices.push(PublishScanNotice {
                    severity: PublishScanSeverity::Warning,
                    path: only.display().to_string(),
                    message: format!(
                        "không đọc được file đối tác, bundle đi tiếp không có tên: {error}"
                    ),
                });
                Vec::new()
            }
        },
        many => {
            notices.push(PublishScanNotice {
                severity: PublishScanSeverity::Warning,
                path: path.display().to_string(),
                message: format!(
                    "{} file đối tác trong một bundle — không đoán file nào là thật, bỏ qua cả",
                    many.len()
                ),
            });
            Vec::new()
        }
    };

    Ok((
        PublishBundle {
            id: bundle_id,
            source_path: path.display().to_string(),
            name: bundle_name,
            media_kind: PublishMediaKind::Image,
            images: ordered,
            caption_path: caption_path.display().to_string(),
            caption_sha256: sha256_bytes(caption.as_bytes()),
            caption,
            total_bytes,
            partners,
        },
        notices,
        ignored_partner_files,
        ignored_hidden_files,
    ))
}

fn read_dir_sorted(path: &Path) -> Result<Vec<fs::DirEntry>, PublishScanError> {
    // USB/DVT child processes and the desktop's signal handlers can interrupt
    // directory syscalls on macOS.  Treat EINTR as transient so a live scan
    // does not fail merely because a sidecar exited or emitted a signal.
    let mut directory = loop {
        match fs::read_dir(path) {
            Ok(directory) => break directory,
            Err(source) if source.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(source) => {
                return Err(PublishScanError::Io {
                    path: path.display().to_string(),
                    source,
                })
            }
        }
    };
    let mut entries = Vec::new();
    loop {
        match directory.next() {
            Some(Ok(entry)) => entries.push(entry),
            Some(Err(source)) if source.kind() == std::io::ErrorKind::Interrupted => continue,
            Some(Err(source)) => {
                return Err(PublishScanError::Io {
                    path: path.display().to_string(),
                    source,
                })
            }
            None => break,
        }
    }
    entries.sort_by(|left, right| {
        natural_cmp(
            &left.file_name().to_string_lossy(),
            &right.file_name().to_string_lossy(),
        )
    });
    Ok(entries)
}

fn read_file_retry(path: &Path) -> std::io::Result<Vec<u8>> {
    loop {
        match fs::read(path) {
            Ok(bytes) => return Ok(bytes),
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        }
    }
}

fn natural_cmp(left: &str, right: &str) -> Ordering {
    let left_key = natural_key(left);
    let right_key = natural_key(right);
    left_key.cmp(&right_key).then_with(|| left.cmp(right))
}

fn natural_key(value: &str) -> Vec<NaturalPart> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut numeric = false;
    for ch in value.chars() {
        let is_numeric = ch.is_ascii_digit();
        if !current.is_empty() && is_numeric != numeric {
            parts.push(if numeric {
                NaturalPart::Number(current.parse::<u64>().unwrap_or(u64::MAX))
            } else {
                NaturalPart::Text(current.to_lowercase())
            });
            current.clear();
        }
        numeric = is_numeric;
        current.push(ch);
    }
    if !current.is_empty() {
        parts.push(if numeric {
            NaturalPart::Number(current.parse::<u64>().unwrap_or(u64::MAX))
        } else {
            NaturalPart::Text(current.to_lowercase())
        });
    }
    parts
}

#[derive(Debug, PartialEq, Eq)]
enum NaturalPart {
    Text(String),
    Number(u64),
}

impl Ord for NaturalPart {
    fn cmp(&self, other: &Self) -> Ordering {
        match (self, other) {
            (Self::Text(left), Self::Text(right)) => left.cmp(right),
            (Self::Number(left), Self::Number(right)) => left.cmp(right),
            (Self::Text(_), Self::Number(_)) => Ordering::Less,
            (Self::Number(_), Self::Text(_)) => Ordering::Greater,
        }
    }
}

impl PartialOrd for NaturalPart {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

fn numeric_prefix(path: &Path) -> Option<u32> {
    let stem = path.file_stem()?.to_string_lossy();
    let digits: String = stem.chars().take_while(|ch| ch.is_ascii_digit()).collect();
    (!digits.is_empty()).then(|| digits.parse().ok()).flatten()
}

fn is_supported_image(name: &str) -> bool {
    matches!(
        Path::new(name)
            .extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| ext.to_ascii_lowercase())
            .as_deref(),
        Some("png" | "jpg" | "jpeg")
    )
}

fn is_caption_file(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.starts_with("caption") && lower.ends_with(".txt")
}

fn is_partner_file(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.starts_with("partner") && lower.ends_with(".xlsx")
}

fn sha256_bytes(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

fn short_hash(images: &[PublishImage], caption: &str) -> String {
    let mut hasher = Sha256::new();
    for image in images {
        hasher.update(image.sha256.as_bytes());
    }
    hasher.update(caption.as_bytes());
    let digest = format!("{:x}", hasher.finalize());
    digest[..12].to_string()
}

fn slug(value: &str) -> String {
    let mut out = String::new();
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    out.trim_matches('-').to_string()
}

/// Copy a scanned bundle into a managed campaign directory without changing
/// its logical hashes. The copy is intentionally separate from scanning so a
/// preview can remain side-effect free.
pub fn copy_bundle_to_managed(
    bundle: &PublishBundle,
    destination: &Path,
) -> anyhow::Result<PublishBundle> {
    fs::create_dir_all(destination)
        .with_context(|| format!("create managed bundle {}", destination.display()))?;
    for image in &bundle.images {
        let source = Path::new(&image.path);
        let target = destination.join(&image.file_name);
        fs::copy(source, &target).with_context(|| format!("copy {}", source.display()))?;
        let copied =
            fs::read(&target).with_context(|| format!("read copied {}", target.display()))?;
        if sha256_bytes(&copied) != image.sha256 || copied.len() as u64 != image.byte_len {
            bail!("managed copy verification failed for {}", target.display());
        }
    }
    let caption_target = destination.join("caption.txt");
    fs::write(&caption_target, bundle.caption.as_bytes())
        .with_context(|| format!("write managed caption {}", caption_target.display()))?;
    let copied = fs::read(&caption_target)?;
    if sha256_bytes(&copied) != bundle.caption_sha256 {
        bail!(
            "managed caption verification failed for {}",
            caption_target.display()
        );
    }
    let mut managed = bundle.clone();
    managed.source_path = destination.display().to_string();
    managed.caption_path = caption_target.display().to_string();
    for image in &mut managed.images {
        image.path = destination.join(&image.file_name).display().to_string();
    }
    Ok(managed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    struct TempDir(std::path::PathBuf);

    impl TempDir {
        fn new() -> Self {
            let path =
                std::env::temp_dir().join(format!("riviu-publish-test-{}", uuid::Uuid::new_v4()));
            fs::create_dir_all(&path).expect("temp dir");
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn write_png(path: &Path, color: [u8; 3]) {
        let image = image::RgbImage::from_pixel(8, 10, image::Rgb(color));
        image.save(path).expect("png");
    }

    fn ids(prefix: &str, count: usize) -> Vec<String> {
        (1..=count).map(|n| format!("{prefix}-{n:02}")).collect()
    }

    fn dealt(deal: &AutoAssignment) -> Vec<&str> {
        deal.plan.iter().map(|row| row.bundle_id.as_str()).collect()
    }

    /// The operator's run: twenty-one posts, five phones, five different posts.
    #[test]
    fn five_phones_get_five_different_posts() {
        let deal = auto_assign_bundles(&ids("bundle", 21), &[], &ids("phone", 5), 5).expect("deal");
        assert_eq!(deal.plan.len(), 5);
        let chosen: std::collections::BTreeSet<&str> = dealt(&deal).into_iter().collect();
        assert_eq!(
            chosen.len(),
            5,
            "five distinct posts, not one post five times"
        );
        // Positional identity is the whole invariant: row n's bundle goes to row n's phone.
        for (index, row) in deal.plan.iter().enumerate() {
            assert_eq!(row.ordinal, index as u32);
            assert_eq!(row.udid, format!("phone-{:02}", index + 1));
        }
    }

    /// **A second run never re-deals a post that already went out.**
    ///
    /// The old version of this test passed for the wrong reason. It rotated a cursor over
    /// twenty-one bundles, and twenty-one happens to be more than twice five — so the windows
    /// could not overlap and nothing was being proved. Five bundles and five phones is the
    /// case that broke it: `next_cursor` was 5, `5 % 5` is 0, and the second run dealt the
    /// same five carousels onto five live accounts.
    ///
    /// Keyed on what was published rather than on where a counter is, the case cannot arise:
    /// the pool is what is left, and when nothing is left the deal refuses.
    #[test]
    fn a_run_never_deals_a_post_that_already_went_out() {
        let bundles = ids("bundle", 21);
        let phones = ids("phone", 5);
        let first = auto_assign_bundles(&bundles, &[], &phones, 5).expect("first deal");
        let published: Vec<String> = dealt(&first).into_iter().map(str::to_string).collect();
        let second = auto_assign_bundles(&bundles, &published, &phones, 5).expect("second deal");
        let a: std::collections::BTreeSet<&str> = dealt(&first).into_iter().collect();
        let b: std::collections::BTreeSet<&str> = dealt(&second).into_iter().collect();
        assert!(
            a.is_disjoint(&b),
            "two consecutive runs share a post: {a:?} vs {b:?}"
        );

        // **The case the cursor got wrong**, at every inventory size where it could: an
        // inventory smaller than two deals.
        for total in 5..=9usize {
            let bundles = ids("bundle", total);
            let first = auto_assign_bundles(&bundles, &[], &phones, 5).expect("first deal");
            let published: Vec<String> = dealt(&first).into_iter().map(str::to_string).collect();
            let second = auto_assign_bundles(&bundles, &published, &phones, 5);
            match second {
                Ok(second) => {
                    let a: std::collections::BTreeSet<&str> = dealt(&first).into_iter().collect();
                    let b: std::collections::BTreeSet<&str> = dealt(&second).into_iter().collect();
                    assert!(
                        a.is_disjoint(&b),
                        "with {total} posts the second run re-published {:?}",
                        a.intersection(&b).collect::<Vec<_>>()
                    );
                }
                // Refusing is the right answer once there is nothing fresh left.
                Err(PublishAssignError::NotEnoughFreshBundles { wanted, available }) => {
                    assert_eq!(wanted, 5);
                    assert_eq!(available, total - 5);
                }
                Err(other) => panic!("unexpected refusal with {total} posts: {other:?}"),
            }
        }
    }

    /// The whole folder gets published exactly once across successive runs.
    #[test]
    fn every_post_goes_out_once_and_then_the_deals_run_out() {
        let bundles = ids("bundle", 21);
        let phones = ids("phone", 5);
        let mut published: Vec<String> = Vec::new();
        for _ in 0..4 {
            let deal = auto_assign_bundles(&bundles, &published, &phones, 5).expect("deal");
            published.extend(dealt(&deal).into_iter().map(str::to_string));
        }
        assert_eq!(published.len(), 20);
        let distinct: std::collections::BTreeSet<&String> = published.iter().collect();
        assert_eq!(distinct.len(), 20, "a post went out twice");
        // One left, so a fifth run of five refuses rather than reaching back.
        assert!(matches!(
            auto_assign_bundles(&bundles, &published, &phones, 5),
            Err(PublishAssignError::NotEnoughFreshBundles {
                wanted: 5,
                available: 1
            })
        ));
        // And a run of one still works.
        assert_eq!(
            auto_assign_bundles(&bundles, &published, &phones, 1)
                .expect("one left")
                .plan
                .len(),
            1
        );
    }

    /// **A duplicate id in the folder is refused, wherever it sits.**
    ///
    /// `validate_publish_mapping` only ever sees the chosen subset, so a duplicate outside the
    /// window used to slip past it entirely — `["A","B","C","A"]` dealt `A` on one run and `A`
    /// again on the next, each deal internally fine.
    #[test]
    fn two_folders_claiming_one_id_are_refused_rather_than_dealt_twice() {
        let bundles = vec![
            "A".to_string(),
            "B".to_string(),
            "C".to_string(),
            "A".to_string(),
        ];
        assert!(matches!(
            auto_assign_bundles(&bundles, &[], &ids("phone", 1), 1),
            Err(PublishAssignError::DuplicateBundleInSource(id)) if id == "A"
        ));
    }

    /// Refusing rather than dealing fewer, in every direction.
    #[test]
    fn too_few_of_either_side_is_refused_rather_than_shrunk() {
        // More posts than phones is normal — twenty-one posts, five phones.
        auto_assign_bundles(&ids("bundle", 21), &[], &ids("phone", 5), 5)
            .expect("more posts is fine");

        assert!(matches!(
            auto_assign_bundles(&ids("bundle", 21), &[], &ids("phone", 3), 5),
            Err(PublishAssignError::NotEnoughPhones {
                wanted: 5,
                available: 3
            })
        ));
        // The one that matters: four posts cannot fill five phones without repeating one, and
        // the same carousel on two live accounts is the thing this refuses to do.
        assert!(matches!(
            auto_assign_bundles(&ids("bundle", 4), &[], &ids("phone", 5), 5),
            Err(PublishAssignError::NotEnoughFreshBundles {
                wanted: 5,
                available: 4
            })
        ));
        assert!(matches!(
            auto_assign_bundles(&ids("bundle", 21), &[], &ids("phone", 5), 0),
            Err(PublishAssignError::Empty)
        ));
        // A published id that is not in the folder at all changes nothing.
        assert_eq!(
            auto_assign_bundles(
                &ids("bundle", 21),
                &["not-in-this-folder".to_string()],
                &ids("phone", 5),
                5
            )
            .expect("deal")
            .plan
            .len(),
            5
        );
    }

    /// The bijection stays enforced by `validate_publish_mapping`, not by a second copy here.
    #[test]
    fn a_duplicate_phone_is_still_refused_through_the_shared_check() {
        let phones = vec![
            "phone-01".to_string(),
            "phone-01".to_string(),
            "phone-03".to_string(),
        ];
        assert!(matches!(
            auto_assign_bundles(&ids("bundle", 21), &[], &phones, 3),
            Err(PublishAssignError::Mapping(
                PublishPlanError::DuplicateUdid(_)
            ))
        ));
    }

    #[test]
    fn scans_caption_images_in_numeric_order_and_ignores_partner_file() {
        let root = TempDir::new();
        let bundle = root.path().join("bo10 grid8");
        fs::create_dir(&bundle).expect("bundle");
        write_png(&bundle.join("02-two.png"), [2, 2, 2]);
        write_png(&bundle.join("01-cover.png"), [1, 1, 1]);
        fs::write(bundle.join("caption-set1.txt"), "Mở bài\n\n#tag").expect("caption");
        fs::write(bundle.join("partners-set1.xlsx"), b"fixture").expect("partner");
        fs::write(bundle.join(".DS_Store"), b"fixture").expect("hidden");

        let manifest =
            scan_publish_folder(root.path(), PublishScanOptions::default()).expect("scan");
        assert_eq!(manifest.bundles.len(), 1);
        assert_eq!(manifest.bundles[0].images[0].file_name, "01-cover.png");
        assert_eq!(manifest.bundles[0].images[1].file_name, "02-two.png");
        assert_eq!(manifest.bundles[0].caption, "Mở bài\n\n#tag");
        assert_eq!(manifest.ignored_partner_files, 1);
        assert_eq!(manifest.ignored_hidden_files, 1);
    }

    /// **Partner names ride the bundle out of the scan, or degrade loudly — never block.**
    ///
    /// Three folders in one scan: a real workbook (both names arrive, workbook order), a
    /// corrupt one (empty list plus a notice, scan survives), and two workbooks in one
    /// folder (a guess the sheet would print live, so: empty list plus a notice naming the
    /// count). `ignored_partner_files` keeps its old meaning — counted, not vanished.
    #[test]
    fn partner_names_ride_the_bundle_and_bad_workbooks_degrade_loudly() {
        fn write_workbook(path: &Path, first_name: &str) {
            let file = fs::File::create(path).expect("workbook file");
            let mut writer = zip::ZipWriter::new(file);
            let options: zip::write::FileOptions<'_, ()> = zip::write::FileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated);
            let sheet = format!(
                r#"<worksheet><sheetData><row r="1"><c r="A1" t="inlineStr"><is><t>{first_name}</t></is></c><c r="B1" t="inlineStr"><is><t>Quán B</t></is></c></row></sheetData></worksheet>"#
            );
            let parts: [(&str, &[u8]); 3] = [
                (
                    "xl/workbook.xml",
                    br#"<workbook><sheets><sheet name="Doi tac" sheetId="1" r:id="rId1"/></sheets></workbook>"#,
                ),
                (
                    "xl/_rels/workbook.xml.rels",
                    br#"<Relationships><Relationship Id="rId1" Target="worksheets/sheet1.xml"/></Relationships>"#,
                ),
                ("xl/worksheets/sheet1.xml", sheet.as_bytes()),
            ];
            for (name, bytes) in parts {
                writer.start_file(name, options).expect("start part");
                writer.write_all(bytes).expect("write part");
            }
            writer.finish().expect("finish archive");
        }
        fn folder(root: &Path, name: &str) -> std::path::PathBuf {
            let path = root.join(name);
            fs::create_dir(&path).expect("bundle");
            write_png(&path.join("01-cover.png"), [1, 1, 1]);
            fs::write(path.join("caption-set1.txt"), "Mở bài").expect("caption");
            path
        }

        let root = TempDir::new();
        let good = folder(root.path(), "bo1 co doi tac");
        write_workbook(&good.join("partners-set1.xlsx"), "Quán A");
        let corrupt = folder(root.path(), "bo2 file hong");
        fs::write(corrupt.join("partners-set1.xlsx"), b"not a workbook").expect("corrupt");
        let doubled = folder(root.path(), "bo3 hai file");
        write_workbook(&doubled.join("partners-set1.xlsx"), "Quán C");
        write_workbook(&doubled.join("partners-set2.xlsx"), "Quán D");

        let manifest =
            scan_publish_folder(root.path(), PublishScanOptions::default()).expect("scan");
        let by_name = |name: &str| {
            manifest
                .bundles
                .iter()
                .find(|bundle| bundle.name == name)
                .expect("bundle exists")
        };
        assert_eq!(
            by_name("bo1 co doi tac").partners,
            vec!["Quán A".to_string(), "Quán B".to_string()],
            "the names arrive in workbook order"
        );
        assert_eq!(
            by_name("bo2 file hong").partners,
            Vec::<String>::new(),
            "a corrupt workbook degrades to no names"
        );
        assert!(
            manifest.notices.iter().any(|notice| {
                notice.message.contains("không đọc được file đối tác")
                    && notice.path.contains("bo2 file hong")
            }),
            "and it degrades LOUDLY: {:?}",
            manifest.notices
        );
        assert_eq!(
            by_name("bo3 hai file").partners,
            Vec::<String>::new(),
            "two workbooks are a question, not a guess"
        );
        assert!(
            manifest
                .notices
                .iter()
                .any(|notice| notice.message.contains("2 file đối tác")),
            "{:?}",
            manifest.notices
        );
        assert_eq!(manifest.ignored_partner_files, 4, "counted, not vanished");
    }

    /// **One over-sized folder used to make every other folder unscannable.**
    ///
    /// `scan_bundle` returns `TooManyImages` and `scan_publish_folder` propagates it, so the
    /// operator's real batch — twenty-one posts, three of them thirteen slides — failed at the
    /// scan with an error naming a post they had not selected. The eleven was never TikTok's
    /// number nor the scanner's; it was the iOS composer's 3x4 tap grid, and it now lives with
    /// that grid.
    #[test]
    fn a_thirteen_slide_carousel_scans_and_does_not_take_its_neighbours_down() {
        let root = TempDir::new();

        let big = root.path().join("set1 19 spotlightv3");
        fs::create_dir(&big).expect("bundle");
        for order in 1..=13u32 {
            write_png(
                &big.join(format!("{order:02}-slide.png")),
                [order as u8, order as u8, order as u8],
            );
        }
        fs::write(big.join("caption-set1.txt"), "Mở bài\n\n#dalat").expect("caption");

        let small = root.path().join("set1 03 budget-72h-summary");
        fs::create_dir(&small).expect("bundle");
        write_png(&small.join("01-cover.png"), [1, 1, 1]);
        write_png(&small.join("02-tail.png"), [2, 2, 2]);
        fs::write(small.join("caption-set1.txt"), "Ngắn").expect("caption");

        let manifest =
            scan_publish_folder(root.path(), PublishScanOptions::default()).expect("scan");
        assert_eq!(manifest.bundles.len(), 2, "both folders survive the scan");
        let widest = manifest
            .bundles
            .iter()
            .map(|bundle| bundle.images.len())
            .max()
            .expect("bundles");
        assert_eq!(widest, 13);
    }

    /// The ceiling did not vanish, it moved: TikTok's own is 35, and it still refuses.
    #[test]
    fn a_carousel_past_tiktoks_own_ceiling_is_still_refused() {
        // **Both sides of the boundary, so the constant is pinned and not merely exceeded.**
        // Testing 13 and 36 left every value in between unclaimed — the suite stayed green
        // with the cap set anywhere from 13 to 35, including back at the old 11-slide iOS
        // grid limit, which is the value that once made all twenty-one folders unscannable.
        let build = |count: u32| {
            let root = TempDir::new();
            let bundle = root.path().join("carousel");
            fs::create_dir(&bundle).expect("bundle");
            for order in 1..=count {
                write_png(&bundle.join(format!("{order:02}-slide.png")), [1, 1, 1]);
            }
            fs::write(bundle.join("caption.txt"), "x").expect("caption");
            root
        };
        let at_the_cap = build(DEFAULT_MAX_IMAGES as u32);
        let scanned = scan_publish_folder(at_the_cap.path(), PublishScanOptions::default())
            .expect("exactly the cap must be accepted");
        assert_eq!(scanned.bundles[0].images.len(), DEFAULT_MAX_IMAGES);

        let one_past = build(DEFAULT_MAX_IMAGES as u32 + 1);
        assert!(matches!(
            scan_publish_folder(one_past.path(), PublishScanOptions::default()),
            Err(PublishScanError::TooManyImages { .. })
        ));

        // And the cap is TikTok's, not the iOS tap grid's. The 11 below is the number that
        // made a single 13-slide folder refuse all twenty-one.
        // 35 is TikTok's own ceiling. 11 was the iOS tap grid's, and it is the number that
        // once made a single 13-slide folder refuse all twenty-one.
        assert_eq!(DEFAULT_MAX_IMAGES, 35);
    }

    #[test]
    fn rejects_gapped_image_order_and_multiple_captions() {
        let root = TempDir::new();
        let bundle = root.path().join("bo");
        fs::create_dir(&bundle).expect("bundle");
        write_png(&bundle.join("01-cover.png"), [1, 1, 1]);
        write_png(&bundle.join("03-last.png"), [3, 3, 3]);
        fs::write(bundle.join("caption-a.txt"), b"a").expect("caption");
        assert!(matches!(
            scan_publish_folder(root.path(), PublishScanOptions::default()),
            Err(PublishScanError::InvalidImageOrder { .. })
        ));

        fs::remove_file(bundle.join("03-last.png")).expect("remove");
        write_png(&bundle.join("02-last.png"), [2, 2, 2]);
        fs::write(bundle.join("caption-b.txt"), b"b").expect("caption");
        assert!(matches!(
            scan_publish_folder(root.path(), PublishScanOptions::default()),
            Err(PublishScanError::MultipleCaptions { .. })
        ));
    }

    #[test]
    fn managed_copy_preserves_hashes() {
        let root = TempDir::new();
        let bundle_path = root.path().join("bo");
        let managed_path = root.path().join("managed");
        fs::create_dir(&bundle_path).expect("bundle");
        write_png(&bundle_path.join("01-cover.png"), [9, 8, 7]);
        let mut caption = fs::File::create(bundle_path.join("caption.txt")).expect("caption");
        writeln!(caption, "caption").expect("write");
        let manifest =
            scan_publish_folder(root.path(), PublishScanOptions::default()).expect("scan");
        let managed = copy_bundle_to_managed(&manifest.bundles[0], &managed_path).expect("copy");
        assert_eq!(
            managed.images[0].sha256,
            manifest.bundles[0].images[0].sha256
        );
        assert_eq!(managed.caption_sha256, manifest.bundles[0].caption_sha256);
        assert!(Path::new(&managed.images[0].path).is_file());
    }
}

#[cfg(test)]
mod mapping_tests {
    use super::{validate_publish_mapping, PublishPlanError};

    fn ids(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    /// **Position is identity, and this is the assertion the whole function exists for.**
    ///
    /// The nth bundle goes to the nth phone, and `ordinal` records that n. Get this wrong and one
    /// account posts another account's photographs under another account's caption -- with
    /// nothing to notice afterwards, because every phone did post *something*, and no delete
    /// path to undo it.
    #[test]
    fn the_nth_bundle_goes_to_the_nth_phone() {
        let plan = validate_publish_mapping(
            &ids(&["quan-an-1", "quan-an-2", "quan-an-3"]),
            &ids(&["10969614", "23021RAAEG", "a99f1234"]),
        )
        .expect("a clean mapping");

        assert_eq!(plan.len(), 3);
        assert_eq!(plan[0].bundle_id, "quan-an-1");
        assert_eq!(plan[0].udid, "10969614");
        assert_eq!(plan[1].bundle_id, "quan-an-2");
        assert_eq!(plan[1].udid, "23021RAAEG");
        assert_eq!(plan[2].bundle_id, "quan-an-3");
        assert_eq!(plan[2].udid, "a99f1234");
        // Ordinals are the position, in order, from zero -- not a re-derived index.
        assert_eq!(
            plan.iter().map(|entry| entry.ordinal).collect::<Vec<_>>(),
            vec![0, 1, 2]
        );
    }

    /// A single pair is the ordinary one-phone case and must work.
    #[test]
    fn one_bundle_and_one_phone_is_a_valid_plan() {
        let plan = validate_publish_mapping(&ids(&["b"]), &ids(&["u"])).expect("one pair");
        assert_eq!(plan.len(), 1);
        assert_eq!(plan[0].ordinal, 0);
    }

    /// **Different lengths cannot be read as an intent, so they are refused rather than zipped.**
    ///
    /// `zip` would silently truncate to the shorter list: three bundles and two phones would
    /// publish two and drop one, or two bundles and three phones would leave a phone out. Both
    /// look like success.
    #[test]
    fn a_length_mismatch_is_refused_in_both_directions() {
        assert!(matches!(
            validate_publish_mapping(&ids(&["a", "b", "c"]), &ids(&["u", "v"])),
            Err(PublishPlanError::MappingLength)
        ));
        assert!(matches!(
            validate_publish_mapping(&ids(&["a", "b"]), &ids(&["u", "v", "w"])),
            Err(PublishPlanError::MappingLength)
        ));
    }

    /// **Nothing selected is a lost selection, not a no-op.**
    ///
    /// `Ok(vec![])` here would become a campaign that runs and posts nothing, which reads to the
    /// operator as the phones having refused.
    #[test]
    fn an_empty_selection_is_refused_rather_than_treated_as_nothing_to_do() {
        assert!(matches!(
            validate_publish_mapping(&[], &[]),
            Err(PublishPlanError::MappingLength)
        ));
    }

    /// An empty entry on either side, including one that is only whitespace.
    #[test]
    fn an_empty_entry_on_either_side_is_refused() {
        assert!(matches!(
            validate_publish_mapping(&ids(&["a", ""]), &ids(&["u", "v"])),
            Err(PublishPlanError::EmptyBundleId)
        ));
        assert!(matches!(
            validate_publish_mapping(&ids(&["a", "   "]), &ids(&["u", "v"])),
            Err(PublishPlanError::EmptyBundleId)
        ));
        assert!(matches!(
            validate_publish_mapping(&ids(&["a", "b"]), &ids(&["u", ""])),
            Err(PublishPlanError::EmptyUdid)
        ));
        assert!(matches!(
            validate_publish_mapping(&ids(&["a", "b"]), &ids(&["u", "  "])),
            Err(PublishPlanError::EmptyUdid)
        ));
    }

    /// A repeat on either side, and the error **names which one** -- because "duplicate udid" with
    /// twenty phones selected is not something an operator can act on.
    #[test]
    fn a_repeat_on_either_side_is_refused_and_named() {
        match validate_publish_mapping(&ids(&["a", "a"]), &ids(&["u", "v"])) {
            Err(PublishPlanError::DuplicateBundle(named)) => assert_eq!(named, "a"),
            other => panic!("expected a named duplicate bundle, got {other:?}"),
        }
        match validate_publish_mapping(&ids(&["a", "b"]), &ids(&["u", "u"])) {
            Err(PublishPlanError::DuplicateUdid(named)) => assert_eq!(named, "u"),
            other => panic!("expected a named duplicate udid, got {other:?}"),
        }
    }

    /// **Two lines that disagreed about what an id *is*.**
    ///
    /// The emptiness check treats surrounding whitespace as not part of the id (`"   "` is
    /// empty). Deduplication used to run on the raw string, which says the opposite -- so
    /// `"bundle-1"` and `"bundle-1 "` both passed as distinct bundles, and two phones posted one
    /// bundle. Neither line was wrong on its own; the disagreement was the bug.
    #[test]
    fn a_trailing_space_does_not_smuggle_a_duplicate_past_the_check() {
        match validate_publish_mapping(&ids(&["bundle-1", "bundle-1 "]), &ids(&["u", "v"])) {
            Err(PublishPlanError::DuplicateBundle(_)) => {}
            other => panic!("a trailing space made one bundle look like two: {other:?}"),
        }
        match validate_publish_mapping(&ids(&["a", "b"]), &ids(&[" 10969614", "10969614"])) {
            Err(PublishPlanError::DuplicateUdid(_)) => {}
            other => panic!("a leading space made one phone look like two: {other:?}"),
        }
    }

    /// Twenty phones, which is the fleet this runs on. Guards against anything O(n) that only
    /// works for two, and against an ordinal that resets or repeats at scale.
    #[test]
    fn a_full_fleet_maps_cleanly_and_keeps_its_order() {
        let bundles: Vec<String> = (0..20).map(|index| format!("bundle-{index}")).collect();
        let phones: Vec<String> = (0..20).map(|index| format!("phone-{index}")).collect();

        let plan = validate_publish_mapping(&bundles, &phones).expect("twenty clean pairs");
        assert_eq!(plan.len(), 20);
        for (index, entry) in plan.iter().enumerate() {
            assert_eq!(entry.bundle_id, format!("bundle-{index}"));
            assert_eq!(entry.udid, format!("phone-{index}"));
            assert_eq!(entry.ordinal, index as u32);
        }
    }
}
