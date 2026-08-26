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

const DEFAULT_MAX_IMAGES: usize = 11;

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
            ignored_partner_files += 1;
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
