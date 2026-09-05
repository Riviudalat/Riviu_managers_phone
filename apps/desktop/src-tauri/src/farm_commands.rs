use crate::command_error::CommandError;
use crate::commands::GroupInstallResult;
use base64::Engine;
use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::io::Write;
use std::path::{Path, PathBuf};

use chrono::{Duration, Utc};
use futures_util::{stream, StreamExt};
use riviu_core::{
    resolve_target, AnalyticsSummary, AndroidInstallDeviceSpec, AppInstallBatchResponse,
    AppInstallProgress, AppInstallProgressPhase, AppInstallRequest, AppInstallResult,
    AppInstallStatus, AppLibraryItem, AppLibraryPlatform, AppPackageFormat,
    DeviceAppInstallRequest, DeviceGroup, DeviceMeta, DevicePlatform, DeviceWorkOwner,
    MaterialItem, MaterialPushBatchRequest, MaterialPushBatchResult, MaterialPushDeviceResult,
    MaterialPushStatus, OpLog, OperationRunKind, OperationRunState, ScheduleItem,
};
use sha2::{Digest, Sha256};
use tauri::State;
use uuid::Uuid;

use crate::state::AppState;

/// Resolve a flat library file and prove that it is a regular file directly
/// inside the expected managed root. Database paths are data, not authority to
/// read or delete elsewhere on the machine.
fn resolve_managed_file(managed_root: &Path, path: &str) -> Result<PathBuf, CommandError> {
    let canonical_root = std::fs::canonicalize(managed_root).map_err(err)?;
    let candidate = PathBuf::from(path);
    let metadata = std::fs::symlink_metadata(&candidate).map_err(err)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(CommandError::operation(
            "managed artifact is not a regular file",
        ));
    }
    let canonical = std::fs::canonicalize(&candidate).map_err(err)?;
    if canonical.parent() != Some(canonical_root.as_path()) {
        return Err(CommandError::operation(
            "managed artifact escaped its library root",
        ));
    }
    Ok(canonical)
}

/// Delete a file this app owns, and refuse if it is still there afterwards.
///
/// **A delete that did not delete is not a success.** Both callers used to write
/// `let _ = std::fs::remove_file(&item.path);` and then drop the database row
/// regardless, so a file held open by another process, marked read-only, or blocked
/// by antivirus left the library row gone and the bytes on disk forever — with the
/// UI reporting the delete as done. On a farm machine the orphans are APKs and
/// videos, so this is measured in gigabytes, not tidiness.
///
/// `NotFound` is success: the row is what the operator asked to remove, and a file
/// that is already gone has nothing left to fail at. Every other error is reported
/// with the path, because "which file" is the first thing anyone needs.
///
/// Found by an independent review on 27/08/2026.
fn remove_managed_file(managed_root: &Path, path: &str) -> Result<(), CommandError> {
    let candidate = PathBuf::from(path);
    let managed = match std::fs::symlink_metadata(&candidate) {
        Ok(_) => resolve_managed_file(managed_root, path)?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let canonical_root = std::fs::canonicalize(managed_root).map_err(err)?;
            let parent = candidate
                .parent()
                .ok_or_else(|| CommandError::operation("managed artifact has no parent"))?;
            let canonical_parent = std::fs::canonicalize(parent).map_err(err)?;
            if canonical_parent != canonical_root {
                return Err(CommandError::operation(
                    "managed artifact escaped its library root",
                ));
            }
            return Ok(());
        }
        Err(error) => return Err(err(error)),
    };
    match std::fs::remove_file(&managed) {
        Ok(()) if !managed.exists() => Ok(()),
        Ok(()) => Err(CommandError::operation(format!(
            "managed artifact still exists after delete: {}",
            managed.display()
        ))),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(CommandError::operation(format!(
            "không xoá được {}: {error}",
            managed.display()
        ))),
    }
}

fn digest_path_component(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))
}

fn managed_extension(path: &Path) -> Option<String> {
    let extension = path.extension()?.to_str()?;
    (!extension.is_empty()
        && extension.len() <= 12
        && extension.bytes().all(|byte| byte.is_ascii_alphanumeric()))
    .then(|| extension.to_ascii_lowercase())
}

fn material_storage_path(materials_root: &Path, id: &str, source: &Path) -> PathBuf {
    let stem = digest_path_component(id);
    match managed_extension(source) {
        Some(extension) => materials_root.join(format!("{stem}.{extension}")),
        None => materials_root.join(stem),
    }
}

fn material_staging_root(
    artifacts_dir: &Path,
    udid: &str,
    material_id: &str,
    nonce: Uuid,
) -> PathBuf {
    artifacts_dir
        .join("push-staging")
        .join(digest_path_component(udid))
        .join(digest_path_component(material_id))
        .join(nonce.to_string())
}

fn staged_material_path(staging_dir: &Path, managed_source: &Path) -> PathBuf {
    match managed_extension(managed_source) {
        Some(extension) => staging_dir.join(format!("payload.{extension}")),
        None => staging_dir.join("payload"),
    }
}

fn err(e: impl std::fmt::Display) -> CommandError {
    CommandError::operation(e)
}

fn log(state: &AppState, action: &str, detail: &str) {
    let _ = state.db.log_op(action, detail);
}

fn persist_library_outcome(
    state: &AppState,
    batch_id: &str,
    udid: &str,
    status: OperationRunState,
    code: Option<&str>,
    detail: Option<&str>,
    evidence: Option<&str>,
) -> Result<(), String> {
    state.db.settle_library_batch_item(batch_id,udid,status,code,detail,evidence).map_err(|error| {
        let message = format!("Chưa lưu được kết quả của máy; cần kiểm lại, không thực hiện lại thao tác: {error}");
        // Retrying only the journal is allowed. The driver action is never repeated.
        if let Err(recovery) = state.db.settle_library_batch_item(batch_id,udid,OperationRunState::Uncertain,
            Some("OutcomePersistenceUnavailable"),Some(&message),evidence) {
            log::error!("library batch outcome and uncertain journal both unavailable: {error}; {recovery}");
        }
        message
    })
}

fn install_dispatch_claim(claim: Result<bool, String>, udid: &str) -> Result<(), AppInstallResult> {
    let (status, detail) = match claim {
        Ok(true) => return Ok(()),
        Ok(false) => (
            AppInstallStatus::CancelledBeforeDispatch,
            "Đã dừng trước khi gửi lệnh tới máy".to_string(),
        ),
        Err(error) => (
            AppInstallStatus::BeforeEffect,
            format!("Chưa ghi được ý định cài đặt; chưa gửi lệnh tới máy: {error}"),
        ),
    };
    Err(AppInstallResult {
        udid: udid.to_string(),
        status,
        effect_started: false,
        observed_version_name: None,
        observed_version_code: None,
        detail: Some(detail),
    })
}

const MAX_SOURCE_BYTES: u64 = 4 * 1024 * 1024 * 1024;
const MAX_STANDALONE_APK_BYTES: u64 = 512 * 1024 * 1024;
const MAX_CONTAINER_ENTRIES: usize = 2_048;
const MAX_CONTAINER_APK_BYTES: u64 = 512 * 1024 * 1024;
const MAX_CONTAINER_TOTAL_APK_BYTES: u64 = 4 * 1024 * 1024 * 1024;
const MAX_INSTALL_CONCURRENCY: usize = 2;
const MAX_MATERIAL_PUSH_CONCURRENCY: usize = 2;

/// Owns one per-attempt desktop staging directory until the device readback finishes.
/// Drop covers success, transport failure and every early `?` after the copy.
struct MaterialStagingGuard {
    root: PathBuf,
}

impl MaterialStagingGuard {
    fn create(root: PathBuf) -> std::io::Result<Self> {
        std::fs::create_dir_all(&root)?;
        Ok(Self { root })
    }

    fn root(&self) -> &Path {
        &self.root
    }
}

impl Drop for MaterialStagingGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[derive(Default)]
struct ActiveInstallBatch {
    cancelled: bool,
    gates: Vec<riviu_core::InstallEffectGate>,
}

fn active_install_batches() -> &'static parking_lot::Mutex<HashMap<String, ActiveInstallBatch>> {
    static ACTIVE: std::sync::OnceLock<parking_lot::Mutex<HashMap<String, ActiveInstallBatch>>> =
        std::sync::OnceLock::new();
    ACTIVE.get_or_init(|| parking_lot::Mutex::new(HashMap::new()))
}

struct InstallBatchClaim {
    batch_id: String,
}

impl InstallBatchClaim {
    fn acquire(batch_id: &str) -> Result<Self, CommandError> {
        let mut active = active_install_batches().lock();
        if active.contains_key(batch_id) {
            return Err(CommandError::code(
                "AppInstallBatchActive",
                "an install batch with this ID is already active",
            ));
        }
        active.insert(batch_id.to_string(), ActiveInstallBatch::default());
        Ok(Self {
            batch_id: batch_id.to_string(),
        })
    }

    fn is_cancelled(&self) -> bool {
        active_install_batches()
            .lock()
            .get(&self.batch_id)
            .is_some_and(|batch| batch.cancelled)
    }

    fn register_gate(&self) -> riviu_core::InstallEffectGate {
        let gate = riviu_core::InstallEffectGate::new();
        let mut active = active_install_batches().lock();
        if let Some(batch) = active.get_mut(&self.batch_id) {
            if batch.cancelled {
                gate.cancel_before_effect();
            }
            batch.gates.push(gate.clone());
        } else {
            gate.cancel_before_effect();
        }
        gate
    }
}

impl Drop for InstallBatchClaim {
    fn drop(&mut self) {
        active_install_batches().lock().remove(&self.batch_id);
    }
}

fn install_batch_scratch_root(artifacts_dir: &Path, batch_id: &str, nonce: Uuid) -> PathBuf {
    let digest = Sha256::digest(batch_id.as_bytes());
    artifacts_dir
        .join("app-install-batches")
        .join(format!("{digest:x}"))
        .join(nonce.to_string())
}

struct BatchScratch(PathBuf);

impl BatchScratch {
    fn create(artifacts_dir: &Path, batch_id: &str) -> Result<Self, CommandError> {
        let path = install_batch_scratch_root(artifacts_dir, batch_id, Uuid::new_v4());
        std::fs::create_dir_all(&path).map_err(err)?;
        Ok(Self(path))
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for BatchScratch {
    fn drop(&mut self) {
        if let Err(error) = std::fs::remove_dir_all(&self.0) {
            if error.kind() != std::io::ErrorKind::NotFound {
                log::warn!(
                    "could not remove app-install scratch {}: {error}",
                    self.0.display()
                );
            }
        }
    }
}

async fn collect_bounded<T, U, I, F, Fut>(items: I, limit: usize, map: F) -> Vec<U>
where
    I: IntoIterator<Item = T>,
    F: FnMut(T) -> Fut,
    Fut: Future<Output = U>,
{
    stream::iter(items)
        .map(map)
        .buffer_unordered(limit)
        .collect()
        .await
}

fn package_kind(path: &Path) -> Result<(AppLibraryPlatform, AppPackageFormat), CommandError> {
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default();
    match extension.to_ascii_lowercase().as_str() {
        "ipa" => Ok((AppLibraryPlatform::Ios, AppPackageFormat::Ipa)),
        "apk" => Ok((AppLibraryPlatform::Android, AppPackageFormat::Apk)),
        "xapk" => Ok((AppLibraryPlatform::Android, AppPackageFormat::Xapk)),
        "apkm" => Ok((AppLibraryPlatform::Android, AppPackageFormat::Apkm)),
        "apks" => Ok((AppLibraryPlatform::Android, AppPackageFormat::Apks)),
        "aab" => Err(CommandError::code(
            "UnsupportedAndroidAppBundle",
            ".aab must be converted to a signed .apks artifact before it enters the library",
        )),
        "obb" => Err(CommandError::code(
            "UnsupportedObb",
            "standalone OBB expansion files are not supported",
        )),
        _ => Err(CommandError::invalid_argument(
            "app library only accepts .ipa, .apk, .xapk, .apkm, or .apks files",
        )),
    }
}

#[derive(Debug, Deserialize)]
struct XapkManifest {
    package_name: String,
    #[serde(default)]
    version_name: Option<String>,
    #[serde(default)]
    version_code: Option<serde_json::Value>,
    #[serde(default, alias = "split_configs")]
    split_apks: Vec<XapkSplit>,
}

#[derive(Debug, Deserialize)]
struct XapkSplit {
    #[serde(alias = "filename")]
    file: String,
}

#[derive(Debug, Deserialize)]
struct ApkmManifest {
    #[serde(default, alias = "package_name", alias = "package")]
    pname: Option<String>,
    #[serde(default, alias = "version_name")]
    release_version: Option<String>,
    #[serde(default, alias = "version_code")]
    versioncode: Option<serde_json::Value>,
    #[serde(default, alias = "apks", alias = "files")]
    split_apks: Vec<ApkmSplit>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum ApkmSplit {
    Name(String),
    Entry { file: String },
}

impl ApkmSplit {
    fn into_name(self) -> String {
        match self {
            Self::Name(name) => name,
            Self::Entry { file } => file,
        }
    }
}

#[derive(Debug, Clone)]
struct ContainerLayout {
    base: String,
    apks: Vec<String>,
    declared_application_id: Option<String>,
    declared_version_name: Option<String>,
    declared_version_code: Option<String>,
}

fn json_scalar_string(value: Option<serde_json::Value>) -> Option<String> {
    match value {
        Some(serde_json::Value::String(value)) => Some(value),
        Some(serde_json::Value::Number(value)) => Some(value.to_string()),
        _ => None,
    }
}

#[derive(Debug)]
enum ProtoValue<'a> {
    Varint,
    Bytes(&'a [u8]),
    Fixed,
}

fn read_proto_varint(input: &mut &[u8]) -> Result<u64, CommandError> {
    let mut value = 0_u64;
    for shift in (0..=63).step_by(7) {
        let (&byte, rest) = input
            .split_first()
            .ok_or_else(|| CommandError::invalid_argument("truncated APKS toc.pb varint"))?;
        *input = rest;
        value |= u64::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return Ok(value);
        }
    }
    Err(CommandError::invalid_argument("invalid APKS toc.pb varint"))
}

fn parse_proto_fields(mut input: &[u8]) -> Result<Vec<(u32, ProtoValue<'_>)>, CommandError> {
    let mut fields = Vec::new();
    while !input.is_empty() {
        let key = read_proto_varint(&mut input)?;
        let field = u32::try_from(key >> 3)
            .map_err(|_| CommandError::invalid_argument("APKS toc.pb field overflow"))?;
        if field == 0 {
            return Err(CommandError::invalid_argument(
                "APKS toc.pb contains field zero",
            ));
        }
        let value = match key & 7 {
            0 => {
                let _ = read_proto_varint(&mut input)?;
                ProtoValue::Varint
            }
            1 => {
                if input.len() < 8 {
                    return Err(CommandError::invalid_argument(
                        "truncated APKS toc.pb fixed64 field",
                    ));
                }
                input = &input[8..];
                ProtoValue::Fixed
            }
            2 => {
                let length = usize::try_from(read_proto_varint(&mut input)?)
                    .map_err(|_| CommandError::invalid_argument("APKS toc.pb length overflow"))?;
                if input.len() < length {
                    return Err(CommandError::invalid_argument(
                        "truncated APKS toc.pb byte field",
                    ));
                }
                let (bytes, rest) = input.split_at(length);
                input = rest;
                ProtoValue::Bytes(bytes)
            }
            5 => {
                if input.len() < 4 {
                    return Err(CommandError::invalid_argument(
                        "truncated APKS toc.pb fixed32 field",
                    ));
                }
                input = &input[4..];
                ProtoValue::Fixed
            }
            _ => {
                return Err(CommandError::invalid_argument(
                    "APKS toc.pb uses unsupported protobuf group encoding",
                ))
            }
        };
        fields.push((field, value));
    }
    Ok(fields)
}

fn child_messages<'a>(
    fields: &'a [(u32, ProtoValue<'a>)],
    number: u32,
) -> impl Iterator<Item = &'a [u8]> {
    fields.iter().filter_map(move |(field, value)| match value {
        ProtoValue::Bytes(bytes) if *field == number => Some(*bytes),
        _ => None,
    })
}

fn parse_apks_toc(bytes: &[u8]) -> Result<(String, Vec<String>), CommandError> {
    let root = parse_proto_fields(bytes)?;
    if root.iter().any(|(field, _)| matches!(field, 3 | 6)) {
        return Err(CommandError::code(
            "UnsupportedPlayAssetDelivery",
            "Bundletool .apks contains Play Asset Delivery metadata",
        ));
    }
    let package = child_messages(&root, 4)
        .next()
        .and_then(|bytes| std::str::from_utf8(bytes).ok())
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| CommandError::invalid_argument("APKS toc.pb has no package name"))?
        .to_string();
    let mut apks = Vec::new();
    for variant in child_messages(&root, 1) {
        let variant = parse_proto_fields(variant)?;
        for apk_set in child_messages(&variant, 2) {
            let apk_set = parse_proto_fields(apk_set)?;
            for description in child_messages(&apk_set, 2) {
                let description = parse_proto_fields(description)?;
                let path = child_messages(&description, 2)
                    .next()
                    .and_then(|bytes| std::str::from_utf8(bytes).ok())
                    .filter(|value| value.to_ascii_lowercase().ends_with(".apk"))
                    .ok_or_else(|| {
                        CommandError::invalid_argument(
                            "APKS toc.pb contains an APK description without a valid path",
                        )
                    })?;
                apks.push(path.to_string());
            }
        }
    }
    if apks.is_empty() {
        return Err(CommandError::invalid_argument(
            "APKS toc.pb declares no installable APK",
        ));
    }
    Ok((package, apks))
}

fn read_json_entry(
    archive: &mut zip::ZipArchive<std::fs::File>,
    name: &str,
) -> Result<Option<Vec<u8>>, CommandError> {
    match archive.by_name(name) {
        Ok(mut entry) => {
            if entry.size() > 16 * 1024 * 1024 {
                return Err(CommandError::invalid_argument(format!(
                    "Android app container metadata {name} exceeds 16 MiB"
                )));
            }
            let mut bytes = Vec::with_capacity(entry.size() as usize);
            std::io::Read::read_to_end(&mut entry, &mut bytes).map_err(err)?;
            Ok(Some(bytes))
        }
        Err(zip::result::ZipError::FileNotFound) => Ok(None),
        Err(error) => Err(err(error)),
    }
}

fn safe_container_layout(
    path: &Path,
    format: AppPackageFormat,
) -> Result<ContainerLayout, CommandError> {
    if std::fs::metadata(path).map_err(err)?.len() > MAX_SOURCE_BYTES {
        return Err(CommandError::invalid_argument(
            "Android app container exceeds 4 GiB",
        ));
    }
    let file = std::fs::File::open(path).map_err(err)?;
    let mut archive = zip::ZipArchive::new(file).map_err(|error| {
        CommandError::invalid_argument(format!("invalid Android app container: {error}"))
    })?;
    let mut names = Vec::new();
    let mut seen = HashSet::new();
    let mut total_apk_bytes = 0_u64;
    if archive.len() > MAX_CONTAINER_ENTRIES {
        return Err(CommandError::invalid_argument(
            "Android app container exceeds 2048 entries",
        ));
    }
    for index in 0..archive.len() {
        let entry = archive.by_index(index).map_err(err)?;
        let raw = entry.name();
        let normalized = raw.replace('\\', "/");
        let drive_absolute = normalized.as_bytes().get(1) == Some(&b':');
        if raw.contains('\\')
            || normalized.starts_with('/')
            || drive_absolute
            || normalized
                .split('/')
                .any(|part| part == ".." || part == ".")
        {
            return Err(CommandError::invalid_argument(format!(
                "Android app container has an unsafe entry: {raw}"
            )));
        }
        if !seen.insert(normalized) {
            return Err(CommandError::invalid_argument(format!(
                "Android app container has duplicate entry: {raw}"
            )));
        }
        if entry.is_symlink() {
            return Err(CommandError::invalid_argument(format!(
                "Android app container has a symlink entry: {raw}"
            )));
        }
        if entry.is_dir() {
            continue;
        }
        let lowered = raw.to_ascii_lowercase();
        if lowered.ends_with(".obb") {
            return Err(CommandError::code(
                "UnsupportedObbPayload",
                format!("Android app container has unsupported OBB payload: {raw}"),
            ));
        }
        if lowered.contains("/pad/")
            || lowered.starts_with("pad/")
            || lowered.contains("/asset-pack/")
            || lowered.starts_with("asset-pack/")
            || lowered.contains("/assetpacks/")
            || lowered.starts_with("assetpacks/")
        {
            return Err(CommandError::code(
                "UnsupportedPlayAssetDelivery",
                format!("Android app container has unsupported Play Asset Delivery entry: {raw}"),
            ));
        }
        if raw.to_ascii_lowercase().ends_with(".apk") {
            if entry.encrypted() {
                return Err(CommandError::invalid_argument(format!(
                    "Android app container APK is encrypted: {raw}"
                )));
            }
            if entry.size() > MAX_CONTAINER_APK_BYTES {
                return Err(CommandError::invalid_argument(
                    "Android app container has an APK larger than 512 MiB",
                ));
            }
            total_apk_bytes = total_apk_bytes
                .checked_add(entry.size())
                .ok_or_else(|| CommandError::invalid_argument("Android APK size overflow"))?;
            if total_apk_bytes > MAX_CONTAINER_TOTAL_APK_BYTES {
                return Err(CommandError::invalid_argument(
                    "Android app container APK payload exceeds 4 GiB",
                ));
            }
            names.push(raw.to_string());
        }
    }
    if names.is_empty() {
        return Err(CommandError::invalid_argument(
            "Android app container contains no APK files",
        ));
    }
    let names_set = names.iter().map(String::as_str).collect::<HashSet<_>>();
    let (base, declared, declared_application_id, declared_version_name, declared_version_code) =
        match format {
            AppPackageFormat::Xapk => {
                let manifest =
                    read_json_entry(&mut archive, "manifest.json")?.ok_or_else(|| {
                        CommandError::invalid_argument("XAPK is missing manifest.json")
                    })?;
                let manifest: XapkManifest =
                    serde_json::from_slice(&manifest).map_err(|error| {
                        CommandError::invalid_argument(format!(
                            "invalid XAPK manifest.json: {error}"
                        ))
                    })?;
                let base = format!("{}.apk", manifest.package_name);
                let declared_application_id = Some(manifest.package_name);
                let declared_version_name = manifest.version_name;
                let declared_version_code = json_scalar_string(manifest.version_code);
                let mut declared = manifest
                    .split_apks
                    .into_iter()
                    .map(|split| split.file)
                    .collect::<Vec<_>>();
                declared.insert(0, base.clone());
                (
                    base,
                    declared,
                    declared_application_id,
                    declared_version_name,
                    declared_version_code,
                )
            }
            AppPackageFormat::Apkm => {
                let base = "base.apk".to_string();
                let manifest = read_json_entry(&mut archive, "manifest.json")?
                    .or(read_json_entry(&mut archive, "info.json")?)
                    .ok_or_else(|| {
                        CommandError::invalid_argument("APKM is missing manifest.json or info.json")
                    })?;
                let manifest: ApkmManifest =
                    serde_json::from_slice(&manifest).map_err(|error| {
                        CommandError::invalid_argument(format!("invalid APKM manifest: {error}"))
                    })?;
                let mut declared = if manifest.split_apks.is_empty() {
                    names.clone()
                } else {
                    manifest
                        .split_apks
                        .into_iter()
                        .map(ApkmSplit::into_name)
                        .collect()
                };
                if !declared.iter().any(|entry| entry == &base) {
                    declared.insert(0, base.clone());
                }
                (
                    base,
                    declared,
                    manifest.pname,
                    manifest.release_version,
                    json_scalar_string(manifest.versioncode),
                )
            }
            AppPackageFormat::Apks => {
                let mut bases = names
                    .iter()
                    .filter(|name| {
                        let lower = name.to_ascii_lowercase();
                        lower == "base.apk"
                            || lower.ends_with("/base.apk")
                            || lower.ends_with("/base-master.apk")
                            || lower.ends_with("/universal.apk")
                            || lower.contains("/standalone-")
                    })
                    .cloned()
                    .collect::<Vec<_>>();
                bases.sort();
                if bases.is_empty() {
                    return Err(CommandError::invalid_argument(
                        ".apks has no base, universal, or standalone APK",
                    ));
                }
                // Bundletool's table of contents can legitimately describe several
                // variants. One canonical base provides immutable library metadata;
                // Bundletool selects and extracts the actual compatible set later.
                let toc = read_json_entry(&mut archive, "toc.pb")?.ok_or_else(|| {
                    CommandError::invalid_argument("Bundletool .apks is missing toc.pb")
                })?;
                let (package_name, toc_apks) = parse_apks_toc(&toc)?;
                (bases[0].clone(), toc_apks, Some(package_name), None, None)
            }
            _ => {
                return Err(CommandError::invalid_argument(
                    "container layout requested for a non-container artifact",
                ))
            }
        };
    let mut declared_set = HashSet::new();
    for declared_name in &declared {
        if !declared_set.insert(declared_name.as_str()) {
            return Err(CommandError::invalid_argument(format!(
                "Android app container manifest repeats APK {declared_name}"
            )));
        }
        if !names_set.contains(declared_name.as_str()) {
            return Err(CommandError::invalid_argument(format!(
                "Android app container manifest references missing APK {declared_name}"
            )));
        }
    }
    if !declared_set.contains(base.as_str()) {
        return Err(CommandError::invalid_argument(
            "Android app container manifest does not declare its base APK",
        ));
    }
    if declared_set.len() != names_set.len() {
        return Err(CommandError::invalid_argument(
            "Android app container has APK entries not declared by its manifest",
        ));
    }
    Ok(ContainerLayout {
        base,
        apks: declared,
        declared_application_id,
        declared_version_name,
        declared_version_code,
    })
}

struct MaterializedInstallSet {
    root: Option<PathBuf>,
    paths: Vec<PathBuf>,
}

impl Drop for MaterializedInstallSet {
    fn drop(&mut self) {
        if let Some(root) = self.root.take() {
            let _ = std::fs::remove_dir_all(root);
        }
    }
}

fn materialize_android_install_set(
    path: &Path,
    format: AppPackageFormat,
    scratch: &Path,
) -> Result<MaterializedInstallSet, CommandError> {
    if format == AppPackageFormat::Apk {
        return Ok(MaterializedInstallSet {
            root: None,
            paths: vec![path.to_path_buf()],
        });
    }
    let layout = safe_container_layout(path, format)?;
    let root = scratch.join(format!("android-install-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&root).map_err(err)?;
    let result = (|| {
        let file = std::fs::File::open(path).map_err(err)?;
        let mut archive = zip::ZipArchive::new(file).map_err(err)?;
        let mut paths = Vec::with_capacity(layout.apks.len());
        let ordered = std::iter::once(&layout.base)
            .chain(layout.apks.iter().filter(|name| *name != &layout.base))
            .collect::<Vec<_>>();
        for (ordinal, name) in ordered.into_iter().enumerate() {
            let mut entry = archive.by_name(name).map_err(err)?;
            let destination = root.join(format!("{ordinal:03}.apk"));
            let mut output = std::fs::File::create(&destination).map_err(err)?;
            std::io::copy(&mut entry, &mut output).map_err(err)?;
            output.flush().map_err(err)?;
            paths.push(destination);
        }
        Ok(MaterializedInstallSet {
            root: Some(root.clone()),
            paths,
        })
    })();
    if result.is_err() {
        let _ = std::fs::remove_dir_all(&root);
    }
    result
}

fn density_qualifier(density: u32) -> &'static str {
    match density {
        0..=139 => "ldpi",
        140..=186 => "mdpi",
        187..=226 => "tvdpi",
        227..=279 => "hdpi",
        280..=399 => "xhdpi",
        400..=559 => "xxhdpi",
        _ => "xxxhdpi",
    }
}

fn split_is_compatible(name: &str, spec: &AndroidInstallDeviceSpec) -> bool {
    let file = Path::new(name)
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or(name)
        .to_ascii_lowercase();
    let stem = file.strip_suffix(".apk").unwrap_or(&file);
    let qualifiers = stem
        .strip_prefix("config.")
        .or_else(|| stem.strip_prefix("split_config."))
        .or_else(|| stem.split_once(".config.").map(|(_, value)| value));
    let Some(qualifiers) = qualifiers else {
        return true;
    };
    let known_abis = [
        "arm64-v8a",
        "armeabi-v7a",
        "armeabi",
        "x86-64",
        "x86_64",
        "x86",
        "riscv64",
        "mips64",
        "mips",
    ];
    let densities = [
        "ldpi", "mdpi", "tvdpi", "hdpi", "xhdpi", "xxhdpi", "xxxhdpi",
    ];
    for raw in qualifiers.split('.') {
        let token = raw.replace('_', "-");
        if known_abis.contains(&token.as_str()) {
            if !spec
                .supported_abis
                .iter()
                .any(|abi| abi.to_ascii_lowercase().replace('_', "-") == token)
            {
                return false;
            }
            continue;
        }
        if densities.contains(&token.as_str()) {
            if token != density_qualifier(spec.screen_density) {
                return false;
            }
            continue;
        }
        if let Some(dpi) = token
            .strip_suffix("dpi")
            .and_then(|value| value.parse::<u32>().ok())
        {
            if dpi != spec.screen_density {
                return false;
            }
            continue;
        }
        if let Some(min_sdk) = token
            .strip_prefix("sdk")
            .or_else(|| token.strip_prefix('v'))
            .and_then(|value| value.parse::<u32>().ok())
        {
            if spec.sdk_version < min_sdk {
                return false;
            }
            continue;
        }
        let locale_token = token.replace("-r", "-").replace('+', "-");
        let language = locale_token.split('-').next().unwrap_or_default();
        if (2..=3).contains(&language.len()) && language.chars().all(|ch| ch.is_ascii_alphabetic())
        {
            let locale_matches = spec.supported_locales.iter().any(|locale| {
                let candidate = locale.to_ascii_lowercase().replace('_', "-");
                if locale_token.contains('-') {
                    candidate == locale_token
                } else {
                    candidate.split('-').next() == Some(language)
                }
            });
            if !locale_matches {
                return false;
            }
            continue;
        }
        return false;
    }
    true
}

fn materialize_android_install_set_for_spec(
    path: &Path,
    format: AppPackageFormat,
    scratch: &Path,
    spec: &AndroidInstallDeviceSpec,
) -> Result<MaterializedInstallSet, CommandError> {
    if format == AppPackageFormat::Apk {
        return materialize_android_install_set(path, format, scratch);
    }
    let layout = safe_container_layout(path, format)?;
    let selected = layout
        .apks
        .iter()
        .filter(|name| *name == &layout.base || split_is_compatible(name, spec))
        .cloned()
        .collect::<Vec<_>>();
    if !selected.iter().any(|name| name == &layout.base) {
        return Err(CommandError::invalid_argument(
            "compatible split selection omitted the base APK",
        ));
    }
    let root = scratch.join(format!("android-install-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&root).map_err(err)?;
    let result = (|| {
        let file = std::fs::File::open(path).map_err(err)?;
        let mut archive = zip::ZipArchive::new(file).map_err(err)?;
        let mut paths = Vec::with_capacity(selected.len());
        for (ordinal, name) in selected.iter().enumerate() {
            let mut entry = archive.by_name(name).map_err(err)?;
            let destination = root.join(format!("{ordinal:03}.apk"));
            let mut output = std::fs::File::create(&destination).map_err(err)?;
            std::io::copy(&mut entry, &mut output).map_err(err)?;
            output.flush().map_err(err)?;
            paths.push(destination);
        }
        Ok(MaterializedInstallSet {
            root: Some(root.clone()),
            paths,
        })
    })();
    if result.is_err() {
        let _ = std::fs::remove_dir_all(&root);
    }
    result
}

#[derive(Debug)]
struct AndroidMetadata {
    application_id: String,
    version_name: String,
    version_code: Option<String>,
    signer_sha256: String,
    application_label: Option<String>,
    icon_png_base64: Option<String>,
}

fn apk_info(path: &Path) -> Result<AndroidMetadata, CommandError> {
    let apk = apk_info::Apk::new(path).map_err(|error| {
        CommandError::invalid_argument(format!("cannot inspect Android package: {error}"))
    })?;
    let package = apk.get_package_name().unwrap_or_default();
    let version = apk.get_version_name().unwrap_or_default();
    let signatures = apk.get_signatures().map_err(|error| {
        CommandError::invalid_argument(format!("cannot read Android package signature: {error}"))
    })?;
    let signer_sha256 = canonical_signer_identity(
        &signatures
            .iter()
            .filter_map(|signature| serde_json::to_value(signature).ok())
            .collect::<Vec<_>>(),
    );
    if package.trim().is_empty() || version.trim().is_empty() || signer_sha256.is_empty() {
        return Err(CommandError::invalid_argument(
            "Android package must provide package name, version name, and signer",
        ));
    }
    let icon_png_base64 = apk.get_application_icon().and_then(|name| {
        apk.read(&name).ok().and_then(|(bytes, _)| {
            bytes
                .starts_with(b"\x89PNG\r\n\x1a\n")
                .then(|| base64::engine::general_purpose::STANDARD.encode(bytes))
        })
    });
    Ok(AndroidMetadata {
        application_id: package,
        version_name: version,
        version_code: apk.get_version_code(),
        signer_sha256,
        application_label: apk
            .get_application_label()
            .filter(|label| !label.trim().is_empty()),
        icon_png_base64,
    })
}

fn signer_fingerprints(value: &serde_json::Value, output: &mut Vec<String>) {
    match value {
        serde_json::Value::Object(values) => {
            if let Some(fingerprint) = values
                .get("sha256_fingerprint")
                .and_then(serde_json::Value::as_str)
            {
                output.push(fingerprint.to_ascii_lowercase());
            }
            for nested in values.values() {
                signer_fingerprints(nested, output);
            }
        }
        serde_json::Value::Array(values) => {
            for nested in values {
                signer_fingerprints(nested, output);
            }
        }
        _ => {}
    }
}

fn canonical_signer_identity(signatures: &[serde_json::Value]) -> String {
    let mut fingerprints = Vec::new();
    for signature in signatures {
        signer_fingerprints(signature, &mut fingerprints);
    }
    fingerprints.sort();
    fingerprints.dedup();
    fingerprints.join(",")
}

fn inspect_android_package(
    path: &Path,
    format: AppPackageFormat,
) -> Result<AndroidMetadata, CommandError> {
    if format == AppPackageFormat::Apk {
        return apk_info(path);
    }
    let layout = safe_container_layout(path, format)?;
    let temporary_root =
        std::env::temp_dir().join(format!("riviu-container-info-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&temporary_root).map_err(err)?;
    let result = (|| {
        let file = std::fs::File::open(path).map_err(err)?;
        let mut archive = zip::ZipArchive::new(file).map_err(err)?;
        let mut canonical: Option<AndroidMetadata> = None;
        let ordered = std::iter::once(&layout.base)
            .chain(layout.apks.iter().filter(|name| *name != &layout.base));
        for (ordinal, name) in ordered.enumerate() {
            let path = temporary_root.join(format!("{ordinal:04}.apk"));
            let mut entry = archive.by_name(name).map_err(err)?;
            let mut output = std::fs::File::create(&path).map_err(err)?;
            std::io::copy(&mut entry, &mut output).map_err(err)?;
            output.flush().map_err(err)?;
            let metadata = apk_info(&path)?;
            if let Some(expected) = &canonical {
                if metadata.application_id != expected.application_id
                    || metadata.version_name != expected.version_name
                    || metadata.version_code != expected.version_code
                    || metadata.signer_sha256 != expected.signer_sha256
                {
                    return Err(CommandError::invalid_argument(format!(
                        "split APK {name} has a different package, version, or signer"
                    )));
                }
            } else {
                canonical = Some(metadata);
            }
        }
        let canonical =
            canonical.ok_or_else(|| CommandError::invalid_argument("container has no base APK"))?;
        if layout
            .declared_application_id
            .as_deref()
            .is_some_and(|declared| declared != canonical.application_id)
        {
            return Err(CommandError::invalid_argument(
                "container declared package does not match its base APK",
            ));
        }
        if layout
            .declared_version_name
            .as_deref()
            .is_some_and(|declared| declared != canonical.version_name)
        {
            return Err(CommandError::invalid_argument(
                "container declared version name does not match its base APK",
            ));
        }
        if layout
            .declared_version_code
            .as_deref()
            .is_some_and(|declared| Some(declared) != canonical.version_code.as_deref())
        {
            return Err(CommandError::invalid_argument(
                "container declared version code does not match its base APK",
            ));
        }
        Ok(canonical)
    })();
    let _ = std::fs::remove_dir_all(&temporary_root);
    result
}

fn validate_install_set_identity(
    paths: &[PathBuf],
    item: &AppLibraryItem,
) -> Result<(), CommandError> {
    if paths.is_empty() {
        return Err(CommandError::invalid_argument(
            "compatible install set contains no APK files",
        ));
    }
    for path in paths {
        let metadata = apk_info(path)?;
        if metadata.application_id != item.application_id
            || metadata.version_name != item.version_name
            || metadata.version_code != item.version_code
            || metadata.signer_sha256 != item.signer_sha256
        {
            return Err(CommandError::invalid_argument(format!(
                "selected APK {} has a different package, version, or signer",
                path.display()
            )));
        }
    }
    Ok(())
}

fn stream_copy_with_sha256(
    source: &Path,
    destination: &Path,
) -> Result<(String, u64), CommandError> {
    let mut input = std::fs::File::open(source).map_err(err)?;
    let mut output = std::fs::File::create(destination).map_err(err)?;
    let mut hasher = Sha256::new();
    let mut total = 0_u64;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let count = std::io::Read::read(&mut input, &mut buffer).map_err(err)?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
        output.write_all(&buffer[..count]).map_err(err)?;
        total = total
            .checked_add(count as u64)
            .ok_or_else(|| CommandError::operation("artifact size overflow"))?;
    }
    output.flush().map_err(err)?;
    Ok((format!("{:x}", hasher.finalize()), total))
}

fn snapshot_managed_app_artifact(
    item: &AppLibraryItem,
    batch_root: &Path,
) -> Result<PathBuf, CommandError> {
    if item.sha256.len() != 64 || item.size_bytes == 0 {
        return Err(CommandError::operation(
            "app-library artifact has no verified content identity",
        ));
    }
    let extension = Path::new(&item.path)
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("package");
    let snapshot = batch_root.join(format!("source.{extension}"));
    let (sha256, size_bytes) = stream_copy_with_sha256(Path::new(&item.path), &snapshot)?;
    if sha256 != item.sha256 || size_bytes != item.size_bytes {
        let _ = std::fs::remove_file(&snapshot);
        return Err(CommandError::code(
            "AppArtifactChanged",
            "managed app package no longer matches its imported SHA-256 and size",
        ));
    }
    Ok(snapshot)
}

#[cfg(test)]
fn hash_file_with_sha256(path: &Path) -> Result<(String, u64), CommandError> {
    let mut input = std::fs::File::open(path).map_err(err)?;
    let mut hasher = Sha256::new();
    let mut total = 0_u64;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let count = std::io::Read::read(&mut input, &mut buffer).map_err(err)?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
        total = total
            .checked_add(count as u64)
            .ok_or_else(|| CommandError::operation("artifact size overflow"))?;
    }
    Ok((format!("{:x}", hasher.finalize()), total))
}

#[tauri::command]
pub fn get_device_meta(
    state: State<'_, AppState>,
    udid: String,
) -> Result<DeviceMeta, CommandError> {
    state.db.get_device_meta(&udid).map_err(err)
}

/// Every phone this app has a record for, in one call.
///
/// The grid reads it per refresh to label and order twenty tiles (alias, number). Per-device
/// reads would be twenty IPC round trips to draw one frame, and `get_device_meta` stays for
/// the one-phone editors that already use it.
#[tauri::command]
pub fn list_device_metas(state: State<'_, AppState>) -> Result<Vec<DeviceMeta>, CommandError> {
    state.db.list_device_metas().map_err(err)
}

#[tauri::command]
pub fn save_device_meta(state: State<'_, AppState>, meta: DeviceMeta) -> Result<(), CommandError> {
    let _admission = state.ensure_accepting_work()?;
    state.db.upsert_device_meta(&meta).map_err(err)?;
    log(&state, "device.meta", &meta.udid);
    Ok(())
}

#[tauri::command]
pub fn list_groups(state: State<'_, AppState>) -> Result<Vec<DeviceGroup>, CommandError> {
    state.db.list_groups().map_err(err)
}

#[tauri::command]
pub fn save_group(
    state: State<'_, AppState>,
    group: DeviceGroup,
) -> Result<DeviceGroup, CommandError> {
    let _admission = state.ensure_accepting_work()?;
    let mut g = group;
    if g.id.is_empty() {
        g.id = Uuid::new_v4().to_string();
        g.created_at = Utc::now().to_rfc3339();
    }
    if g.color.is_empty() {
        g.color = "#FF6A00".into();
    }
    state.db.upsert_group(&g).map_err(err)?;
    log(&state, "group.save", &g.name);
    Ok(g)
}

#[tauri::command]
pub fn delete_group(state: State<'_, AppState>, id: String) -> Result<(), CommandError> {
    let _admission = state.ensure_accepting_work()?;
    state.db.delete_group(&id).map_err(err)?;
    log(&state, "group.delete", &id);
    Ok(())
}

#[tauri::command]
pub fn list_materials(state: State<'_, AppState>) -> Result<Vec<MaterialItem>, CommandError> {
    state.db.list_materials().map_err(err)
}

#[tauri::command]
pub fn add_material(
    state: State<'_, AppState>,
    source_path: String,
    name: Option<String>,
) -> Result<MaterialItem, CommandError> {
    let _admission = state.ensure_accepting_work()?;
    let src = PathBuf::from(&source_path);
    if !src.is_file() {
        return Err(err(format!("file not found: {source_path}")));
    }
    let file_name = name
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| {
            src.file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| "material.bin".into())
        });
    let dest_dir = state.artifacts_dir.join("materials");
    std::fs::create_dir_all(&dest_dir).map_err(err)?;
    let id = Uuid::new_v4().to_string();
    let dest = material_storage_path(&dest_dir, &id, &src);
    std::fs::copy(&src, &dest).map_err(err)?;
    let size = std::fs::metadata(&dest).map(|m| m.len()).unwrap_or(0);
    let kind = match src
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase()
        .as_str()
    {
        "jpg" | "jpeg" | "png" | "gif" | "webp" | "heic" => "image",
        "mp4" | "mov" | "m4v" => "video",
        _ => "file",
    }
    .to_string();
    let item = MaterialItem {
        id,
        name: file_name,
        path: dest.display().to_string(),
        kind,
        size,
        created_at: Utc::now().to_rfc3339(),
    };
    if let Err(error) = state.db.add_material(&item) {
        let _ = std::fs::remove_file(&dest);
        return Err(err(error));
    }
    log(&state, "material.add", &item.name);
    Ok(item)
}

#[tauri::command]
pub fn delete_material(state: State<'_, AppState>, id: String) -> Result<(), CommandError> {
    let _admission = state.ensure_accepting_work()?;
    if let Some(item) = state
        .db
        .list_materials()
        .map_err(err)?
        .into_iter()
        .find(|m| m.id == id)
    {
        remove_managed_file(&state.artifacts_dir.join("materials"), &item.path)?;
    }
    state.db.delete_material(&id).map_err(err)?;
    log(&state, "material.delete", &id);
    Ok(())
}

#[tauri::command]
pub async fn push_material(
    state: State<'_, AppState>,
    udid: String,
    material_id: String,
) -> Result<String, CommandError> {
    let _admission = state.ensure_accepting_work()?;
    let item = state
        .db
        .list_materials()
        .map_err(err)?
        .into_iter()
        .find(|m| m.id == material_id)
        .ok_or_else(|| "material not found".to_string())?;
    let evidence = push_material_to_device(&state, &item, &udid, None).await?;
    let msg = format!(
        "Transferred {} to Agent sandbox on {udid}; readback={}",
        item.name, evidence
    );
    log(&state, "material.push", &format!("{udid}:{material_id}"));
    Ok(msg)
}

/// Stage one immutable material and return the driver's readback evidence.
/// Each caller owns admission; this helper owns the per-device lease.
async fn push_material_to_device(
    state: &AppState,
    item: &MaterialItem,
    udid: &str,
    batch_id: Option<&str>,
) -> Result<String, CommandError> {
    // Media never goes through installd. Stage it as a one-file campaign and
    // let the driver perform HouseArrest/AFC size+hash readback.
    let context = state
        .control
        .try_acquire_exclusive(udid, DeviceWorkOwner::Script)
        .await
        .map_err(err)?;
    let managed_source = resolve_managed_file(&state.artifacts_dir.join("materials"), &item.path)?;
    let staging_root = material_staging_root(&state.artifacts_dir, udid, &item.id, Uuid::new_v4());
    let staging = MaterialStagingGuard::create(staging_root).map_err(err)?;
    let staged = staging.root().join("material");
    std::fs::create_dir_all(&staged).map_err(err)?;
    let dest = staged_material_path(&staged, &managed_source);
    std::fs::copy(&managed_source, &dest).map_err(err)?;
    if let Some(batch_id) = batch_id {
        if !state
            .db
            .claim_library_batch_item(batch_id, udid)
            .map_err(err)?
        {
            return Err(CommandError::code(
                "CancelledBeforeDispatch",
                "batch item stopped before dispatch",
            ));
        }
    }
    let evidence = state
        .control
        .stage_publish_media(
            &context,
            &state.active_agent_bundle_id,
            &item.id,
            staging.root(),
        )
        .await
        .map_err(err)?;
    Ok(evidence.to_string())
}

/// Push one material to a semantic fleet target with per-device isolation.
/// At most two phones are active at once; one failed lease/readback does not
/// suppress results from its siblings.
#[tauri::command]
pub async fn push_material_batch(
    state: State<'_, AppState>,
    request: MaterialPushBatchRequest,
) -> Result<MaterialPushBatchResult, CommandError> {
    let _admission = state.ensure_accepting_work()?;
    if request.material_id.trim().is_empty() {
        return Err(CommandError::invalid_argument("materialId is required"));
    }
    let item = state
        .db
        .list_materials()
        .map_err(err)?
        .into_iter()
        .find(|item| item.id == request.material_id)
        .ok_or_else(|| CommandError::code("MaterialNotFound", "material not found"))?;
    let fleet_order = state
        .registry
        .list()
        .into_iter()
        .map(|device| device.udid)
        .collect::<Vec<_>>();
    let target = resolve_target(
        &request.target,
        &fleet_order,
        &state.db.list_device_metas().map_err(err)?,
        &state.db.list_groups().map_err(err)?,
    )
    .map_err(err)?;
    if target.included.is_empty() {
        return Err(CommandError::code(
            "MaterialTargetEmpty",
            "material target has no connected device",
        ));
    }

    let batch_id = request
        .batch_id
        .filter(|id| !id.trim().is_empty())
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    state
        .db
        .create_library_batch(
            &batch_id,
            OperationRunKind::MaterialTransfer,
            &item.id,
            &item.name,
            &target,
        )
        .map_err(err)?;

    let state_ref: &AppState = &state;
    let work = target
        .included
        .iter()
        .enumerate()
        .map(|(index, device)| (index, device.udid.clone()))
        .collect::<Vec<_>>();
    let mut results = collect_bounded(work, MAX_MATERIAL_PUSH_CONCURRENCY, |(index, udid)| {
        let item = &item;
        let batch_id = &batch_id;
        async move {
            let mut result =
                match push_material_to_device(state_ref, item, &udid, Some(batch_id)).await {
                    Ok(evidence) => {
                        log(state_ref, "material.push", &format!("{udid}:{}", item.id));
                        MaterialPushDeviceResult {
                            udid,
                            status: MaterialPushStatus::Succeeded,
                            evidence: Some(evidence),
                            error_code: None,
                            error: None,
                        }
                    }
                    Err(error) => {
                        let started = state_ref
                            .db
                            .get_library_batch(batch_id)
                            .map(|batch| {
                                batch.is_none_or(|batch| {
                                    batch.items.iter().any(|row| {
                                        row.udid.as_deref() == Some(&udid)
                                            && row.state == OperationRunState::Running
                                    })
                                })
                            })
                            .unwrap_or(true);
                        log(
                            state_ref,
                            "material.push.failed",
                            &format!("{udid}:{}:{}", item.id, error.code),
                        );
                        MaterialPushDeviceResult {
                            udid,
                            status: if error.code == "CancelledBeforeDispatch" {
                                MaterialPushStatus::CancelledBeforeDispatch
                            } else if started {
                                MaterialPushStatus::Uncertain
                            } else {
                                MaterialPushStatus::Failed
                            },
                            evidence: None,
                            error_code: Some(error.code),
                            error: Some(error.message.into_string()),
                        }
                    }
                };
            let status = match result.status {
                MaterialPushStatus::Succeeded => OperationRunState::Succeeded,
                MaterialPushStatus::Failed => OperationRunState::Failed,
                MaterialPushStatus::Uncertain => OperationRunState::Uncertain,
                MaterialPushStatus::CancelledBeforeDispatch => OperationRunState::Cancelled,
            };
            if let Err(error) = persist_library_outcome(
                state_ref,
                batch_id,
                &result.udid,
                status,
                result.error_code.as_deref(),
                result.error.as_deref(),
                result.evidence.as_deref(),
            ) {
                result.status = MaterialPushStatus::Uncertain;
                result.error_code = Some("OutcomePersistenceUnavailable".into());
                result.error = Some(error);
            }
            (index, result)
        }
    })
    .await;
    results.sort_by_key(|(index, _)| *index);
    let results = results.into_iter().map(|(_, result)| result).collect();
    log(
        &state,
        "material.push_batch",
        &format!("{batch_id}:{}:{}", item.id, target.included.len()),
    );
    Ok(MaterialPushBatchResult {
        batch_id,
        material_id: item.id,
        target,
        results,
    })
}

#[tauri::command]
pub fn list_apps_library(state: State<'_, AppState>) -> Result<Vec<AppLibraryItem>, CommandError> {
    state.db.list_apps_library().map_err(err)
}

#[tauri::command]
pub fn add_app_library(
    state: State<'_, AppState>,
    source_path: String,
    name: Option<String>,
    bundle_id: Option<String>,
    version: Option<String>,
) -> Result<AppLibraryItem, CommandError> {
    let _admission = state.ensure_accepting_work()?;
    let src = PathBuf::from(&source_path);
    if !src.is_file() {
        return Err(err(format!("app package not found: {source_path}")));
    }
    let (platform, package_format) = package_kind(&src)?;
    if std::fs::metadata(&src).map_err(err)?.len() > MAX_SOURCE_BYTES {
        return Err(CommandError::invalid_argument("app package exceeds 4 GiB"));
    }
    if package_format == AppPackageFormat::Apk
        && std::fs::metadata(&src).map_err(err)?.len() > MAX_STANDALONE_APK_BYTES
    {
        return Err(CommandError::invalid_argument(
            "standalone APK exceeds 512 MiB",
        ));
    }
    let dest_dir = state.artifacts_dir.join("apps");
    std::fs::create_dir_all(&dest_dir).map_err(err)?;
    let id = Uuid::new_v4().to_string();
    let extension = src
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("package")
        .to_ascii_lowercase();
    let dest = dest_dir.join(format!("{id}.{extension}"));
    let source_size_before = std::fs::metadata(&src).map_err(err)?.len();
    let (sha256, size_bytes) = stream_copy_with_sha256(&src, &dest)?;
    let source_size_after = std::fs::metadata(&src).map_err(err)?.len();
    if source_size_before != source_size_after || size_bytes != source_size_after {
        let _ = std::fs::remove_file(&dest);
        return Err(CommandError::operation(
            "app package changed while importing",
        ));
    }
    if let Some(existing) = state.db.find_app_library_by_sha256(&sha256).map_err(err)? {
        let _ = std::fs::remove_file(&dest);
        return Ok(existing);
    }
    let metadata_result = match platform {
        AppLibraryPlatform::Ios => Ok(AndroidMetadata {
            application_id: String::new(),
            version_name: String::new(),
            version_code: None,
            signer_sha256: String::new(),
            application_label: None,
            icon_png_base64: None,
        }),
        AppLibraryPlatform::Android => (|| {
            if package_format != AppPackageFormat::Apk {
                safe_container_layout(&dest, package_format)?;
            }
            inspect_android_package(&dest, package_format)
        })(),
    };
    let metadata = match metadata_result {
        Ok(metadata) => metadata,
        Err(error) => {
            let _ = std::fs::remove_file(&dest);
            return Err(error);
        }
    };
    let file_name = name
        .filter(|value| !value.trim().is_empty())
        .or_else(|| metadata.application_label.clone())
        .unwrap_or_else(|| {
            src.file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| "app-package".into())
        });
    let item = AppLibraryItem {
        id,
        name: file_name,
        path: dest.display().to_string(),
        bundle_id: bundle_id.unwrap_or_else(|| metadata.application_id.clone()),
        version: version.unwrap_or_else(|| metadata.version_name.clone()),
        platform,
        package_format,
        artifact_kind: package_format,
        application_id: metadata.application_id,
        version_name: metadata.version_name,
        version_code: metadata.version_code,
        sha256: sha256.clone(),
        size_bytes,
        signer_sha256: metadata.signer_sha256,
        icon_png_base64: metadata.icon_png_base64,
        metadata_status: if platform == AppLibraryPlatform::Android {
            "verified".into()
        } else {
            "legacy".into()
        },
        metadata_error: None,
        created_at: Utc::now().to_rfc3339(),
    };
    let inserted = match state.db.add_app_library_if_new(&item) {
        Ok(inserted) => inserted,
        Err(error) => {
            let _ = std::fs::remove_file(&dest);
            return Err(err(error));
        }
    };
    if !inserted {
        let _ = std::fs::remove_file(&dest);
        return state
            .db
            .find_app_library_by_sha256(&sha256)
            .map_err(err)?
            .ok_or_else(|| {
                CommandError::operation(
                    "duplicate app package was accepted without a readable existing row",
                )
            });
    }
    log(&state, "app.add", &item.name);
    Ok(item)
}

#[tauri::command]
pub fn delete_app_library(state: State<'_, AppState>, id: String) -> Result<(), CommandError> {
    let _admission = state.ensure_accepting_work()?;
    if let Some(item) = state
        .db
        .list_apps_library()
        .map_err(err)?
        .into_iter()
        .find(|a| a.id == id)
    {
        remove_managed_file(&state.artifacts_dir.join("apps"), &item.path)?;
    }
    state.db.delete_app_library(&id).map_err(err)?;
    log(&state, "app.delete", &id);
    Ok(())
}

#[tauri::command]
pub async fn install_library_app(
    state: State<'_, AppState>,
    udid: String,
    app_id: String,
) -> Result<(), CommandError> {
    let _admission = state.ensure_accepting_work()?;
    let response = install_library_app_batch(
        state,
        AppInstallRequest {
            batch_id: Uuid::new_v4().to_string(),
            app_id,
            udids: vec![udid],
            allow_downgrade: false,
        },
    )
    .await?;
    let result = response
        .results
        .into_iter()
        .next()
        .ok_or_else(|| CommandError::operation("install batch returned no device result"))?;
    if result.status == AppInstallStatus::Succeeded {
        Ok(())
    } else {
        Err(CommandError::code(
            format!("AppInstall{:?}", result.status),
            result
                .detail
                .unwrap_or_else(|| "app install did not succeed".to_string()),
        ))
    }
}

#[tauri::command]
pub fn cancel_app_install_batch(
    state: State<'_, AppState>,
    batch_id: String,
) -> Result<(), CommandError> {
    let _admission = state.ensure_accepting_work()?;
    state.db.cancel_library_batch(&batch_id).map_err(err)?;
    cancel_active_app_install_batch(&batch_id)
}

#[tauri::command]
pub fn operation_cancel_batch(
    state: State<'_, AppState>,
    operation_id: String,
) -> Result<(), CommandError> {
    let _admission = state.ensure_accepting_work()?;
    let (kind, id) = operation_id
        .split_once(':')
        .ok_or_else(|| CommandError::invalid_argument("batch operation ID is required"))?;
    let detail = state
        .db
        .get_library_batch(id)
        .map_err(err)?
        .filter(|detail| detail.summary.kind.as_key() == kind)
        .ok_or_else(|| CommandError::invalid_argument("library batch not found"))?;
    state.db.cancel_library_batch(id).map_err(err)?;
    if detail.summary.kind == OperationRunKind::AppInstall
        && active_install_batches().lock().contains_key(id)
    {
        cancel_active_app_install_batch(id)?;
    }
    Ok(())
}

fn cancel_active_app_install_batch(batch_id: &str) -> Result<(), CommandError> {
    if batch_id.trim().is_empty() {
        return Err(CommandError::invalid_argument("batchId is required"));
    }
    let mut active = active_install_batches().lock();
    let batch = active.get_mut(batch_id).ok_or_else(|| {
        CommandError::code("AppInstallBatchNotActive", "install batch is not active")
    })?;
    batch.cancelled = true;
    for gate in &batch.gates {
        gate.cancel_before_effect();
    }
    Ok(())
}

fn ensure_install_device_spec_unchanged(
    expected: &AndroidInstallDeviceSpec,
    observed: &AndroidInstallDeviceSpec,
) -> Result<(), String> {
    if expected == observed {
        Ok(())
    } else {
        Err(format!(
            "Android device spec changed after split selection: expected {expected:?}, observed {observed:?}"
        ))
    }
}

#[derive(Clone, Copy)]
struct InstallBatchDeviceContext<'a> {
    state: &'a AppState,
    request: &'a AppInstallRequest,
    item: &'a AppLibraryItem,
    paths: Option<&'a [PathBuf]>,
    expected_spec: Option<&'a AndroidInstallDeviceSpec>,
    batch: &'a InstallBatchClaim,
}

async fn install_one_batch_device(
    install: InstallBatchDeviceContext<'_>,
    udid: String,
    blocked: Option<AppInstallResult>,
) -> AppInstallResult {
    let InstallBatchDeviceContext {
        state,
        request,
        item,
        paths,
        expected_spec,
        batch,
    } = install;
    if let Some(result) = blocked {
        return result;
    }
    let cancelled = || AppInstallResult {
        udid: udid.clone(),
        status: AppInstallStatus::CancelledBeforeDispatch,
        effect_started: false,
        observed_version_name: None,
        observed_version_code: None,
        detail: Some("batch cancelled before this device started".to_string()),
    };
    if batch.is_cancelled() {
        return cancelled();
    }
    let context = match state
        .control
        .try_acquire_exclusive(&udid, DeviceWorkOwner::Repair)
        .await
    {
        Ok(context) => context,
        Err(error) => {
            return AppInstallResult {
                udid,
                status: AppInstallStatus::BeforeEffect,
                effect_started: false,
                observed_version_name: None,
                observed_version_code: None,
                detail: Some(error.to_string()),
            };
        }
    };
    if batch.is_cancelled() {
        return cancelled();
    }
    match item.platform {
        AppLibraryPlatform::Android => {
            let Some(paths) = paths else {
                return AppInstallResult {
                    udid,
                    status: AppInstallStatus::BeforeEffect,
                    effect_started: false,
                    observed_version_name: None,
                    observed_version_code: None,
                    detail: Some("no compatible APK set was prepared".to_string()),
                };
            };
            if item.package_format != AppPackageFormat::Apk {
                let Some(expected_spec) = expected_spec else {
                    return AppInstallResult {
                        udid,
                        status: AppInstallStatus::BeforeEffect,
                        effect_started: false,
                        observed_version_name: None,
                        observed_version_code: None,
                        detail: Some("no Android device spec was pinned for split install".into()),
                    };
                };
                let observed_spec = match state.control.android_install_device_spec(&context).await
                {
                    Ok(spec) => spec,
                    Err(error) => {
                        return AppInstallResult {
                            udid,
                            status: AppInstallStatus::BeforeEffect,
                            effect_started: false,
                            observed_version_name: None,
                            observed_version_code: None,
                            detail: Some(format!("device-spec reproof failed: {error}")),
                        };
                    }
                };
                if let Err(detail) =
                    ensure_install_device_spec_unchanged(expected_spec, &observed_spec)
                {
                    return AppInstallResult {
                        udid,
                        status: AppInstallStatus::BeforeEffect,
                        effect_started: false,
                        observed_version_name: None,
                        observed_version_code: None,
                        detail: Some(detail),
                    };
                }
            }
            let effect_gate = batch.register_gate();
            let checked = DeviceAppInstallRequest {
                apk_paths: paths.to_vec(),
                application_id: item.application_id.clone(),
                version_name: item.version_name.clone(),
                version_code: item.version_code.clone(),
                allow_downgrade: request.allow_downgrade,
                effect_gate: Some(effect_gate.clone()),
            };
            match state
                .control
                .install_app_set_checked(&context, &checked)
                .await
            {
                Ok(result) => result,
                Err(error) => {
                    let effect_started = effect_gate.effect_claimed();
                    AppInstallResult {
                        udid,
                        status: if effect_started {
                            AppInstallStatus::Uncertain
                        } else if effect_gate.is_cancelled() {
                            AppInstallStatus::CancelledBeforeDispatch
                        } else {
                            AppInstallStatus::BeforeEffect
                        },
                        effect_started,
                        observed_version_name: None,
                        observed_version_code: None,
                        detail: Some(error.to_string()),
                    }
                }
            }
        }
        AppLibraryPlatform::Ios => {
            let path = PathBuf::from(&item.path);
            let effect_gate = batch.register_gate();
            if !effect_gate.claim_effect() {
                return cancelled();
            }
            match state.control.install_app(&context, &path).await {
                Ok(()) => AppInstallResult {
                    udid,
                    status: AppInstallStatus::Succeeded,
                    effect_started: true,
                    observed_version_name: None,
                    observed_version_code: None,
                    detail: None,
                },
                Err(error) => AppInstallResult {
                    udid,
                    status: AppInstallStatus::Uncertain,
                    effect_started: true,
                    observed_version_name: None,
                    observed_version_code: None,
                    detail: Some(error.to_string()),
                },
            }
        }
    }
}

/// Install one immutable library artifact on a typed fleet request. Equal
/// Android device specs share one materialized split set; only the final ADB
/// install is per phone.
#[tauri::command]
pub async fn install_library_app_batch(
    state: State<'_, AppState>,
    request: AppInstallRequest,
) -> Result<AppInstallBatchResponse, CommandError> {
    let _admission = state.ensure_accepting_work()?;
    if request.batch_id.trim().is_empty()
        || request.app_id.trim().is_empty()
        || request.udids.is_empty()
    {
        return Err(CommandError::invalid_argument(
            "batchId, appId, and at least one device are required",
        ));
    }
    let unique = request.udids.iter().collect::<HashSet<_>>();
    if unique.len() != request.udids.len() {
        return Err(CommandError::invalid_argument(
            "an install batch cannot repeat a device",
        ));
    }
    let batch = InstallBatchClaim::acquire(&request.batch_id)?;
    let item = state
        .db
        .list_apps_library()
        .map_err(err)?
        .into_iter()
        .find(|item| item.id == request.app_id)
        .ok_or_else(|| CommandError::code("AppNotFound", "app not found"))?;
    let target = resolve_target(
        &riviu_core::TargetRef::Explicit {
            udids: request.udids.clone(),
        },
        &request.udids,
        &state.db.list_device_metas().map_err(err)?,
        &[],
    )
    .map_err(err)?;
    state
        .db
        .create_library_batch(
            &request.batch_id,
            OperationRunKind::AppInstall,
            &item.id,
            &item.name,
            &target,
        )
        .map_err(err)?;
    let prepared = (|| {
        let scratch = BatchScratch::create(&state.artifacts_dir, &request.batch_id)?;
        let source = snapshot_managed_app_artifact(&item, scratch.path())?;
        Ok::<_, CommandError>((scratch, source))
    })();
    let (batch_scratch, source) = match prepared {
        Ok(prepared) => prepared,
        Err(error) => {
            for udid in &request.udids {
                let _ = persist_library_outcome(
                    &state,
                    &request.batch_id,
                    udid,
                    OperationRunState::Failed,
                    Some("BeforeEffect"),
                    Some(error.message.as_ref()),
                    None,
                );
            }
            return Err(error);
        }
    };
    let item = AppLibraryItem {
        path: source.display().to_string(),
        ..item
    };
    let (connected, roster_error) = match state.control.list_devices().await {
        Ok(devices) => (devices, None),
        Err(error) => (Vec::new(), Some(error.to_string())),
    };
    let expected = match item.platform {
        AppLibraryPlatform::Ios => DevicePlatform::Ios,
        AppLibraryPlatform::Android => DevicePlatform::Android,
    };
    let mut blocked = HashMap::<String, AppInstallResult>::new();
    for udid in &request.udids {
        let detail = match roster_error.as_ref() {
            Some(error) => Some(format!("fleet roster unavailable: {error}")),
            None => match connected.iter().find(|device| &device.udid == udid) {
                None => Some(format!("device {udid} is not connected")),
                Some(device) if device.platform != expected => Some(format!(
                    "device {udid} has the wrong platform for {}",
                    item.name
                )),
                Some(_) => None,
            },
        };
        if let Some(detail) = detail {
            blocked.insert(
                udid.clone(),
                AppInstallResult {
                    udid: udid.clone(),
                    status: AppInstallStatus::BeforeEffect,
                    effect_started: false,
                    observed_version_name: None,
                    observed_version_code: None,
                    detail: Some(detail),
                },
            );
        }
    }

    let batch_root = batch_scratch.path();
    let mut specs = HashMap::<String, AndroidInstallDeviceSpec>::new();
    if item.platform == AppLibraryPlatform::Android {
        if item.package_format == AppPackageFormat::Apk {
            let direct = AndroidInstallDeviceSpec {
                sdk_version: 0,
                supported_abis: Vec::new(),
                screen_density: 0,
                supported_locales: Vec::new(),
            };
            for udid in &request.udids {
                if !blocked.contains_key(udid) {
                    specs.insert(udid.clone(), direct.clone());
                }
            }
        } else {
            for udid in &request.udids {
                if blocked.contains_key(udid) {
                    continue;
                }
                if batch.is_cancelled() {
                    break;
                }
                let context = match state
                    .control
                    .try_acquire_exclusive(udid, DeviceWorkOwner::Repair)
                    .await
                {
                    Ok(context) => context,
                    Err(error) => {
                        blocked.insert(
                            udid.clone(),
                            AppInstallResult {
                                udid: udid.clone(),
                                status: AppInstallStatus::BeforeEffect,
                                effect_started: false,
                                observed_version_name: None,
                                observed_version_code: None,
                                detail: Some(error.to_string()),
                            },
                        );
                        continue;
                    }
                };
                let spec = match state.control.android_install_device_spec(&context).await {
                    Ok(spec) => spec,
                    Err(error) => {
                        blocked.insert(
                            udid.clone(),
                            AppInstallResult {
                                udid: udid.clone(),
                                status: AppInstallStatus::BeforeEffect,
                                effect_started: false,
                                observed_version_name: None,
                                observed_version_code: None,
                                detail: Some(error.to_string()),
                            },
                        );
                        continue;
                    }
                };
                specs.insert(udid.clone(), spec);
            }
        }
    }
    let mut sets = HashMap::<AndroidInstallDeviceSpec, MaterializedInstallSet>::new();
    let mut set_failures = HashMap::<AndroidInstallDeviceSpec, String>::new();
    for spec in specs.values() {
        if sets.contains_key(spec) || set_failures.contains_key(spec) {
            continue;
        }
        let set = if item.package_format == AppPackageFormat::Apks {
            let representative = specs
                .iter()
                .find_map(|(udid, candidate)| (candidate == spec).then_some(udid))
                .expect("a spec came from at least one device");
            let context = match state
                .control
                .try_acquire_exclusive(representative, DeviceWorkOwner::Repair)
                .await
            {
                Ok(context) => context,
                Err(error) => {
                    set_failures.insert(spec.clone(), error.to_string());
                    continue;
                }
            };
            let root = batch_root.join(format!("apks-{}", Uuid::new_v4()));
            match state
                .control
                .extract_app_container_for_spec(&context, &source, spec, &root)
                .await
            {
                Ok(paths) => Ok(MaterializedInstallSet {
                    root: Some(root),
                    paths,
                }),
                Err(error) => {
                    let _ = std::fs::remove_dir_all(&root);
                    Err(error.to_string())
                }
            }
        } else {
            materialize_android_install_set_for_spec(&source, item.package_format, batch_root, spec)
                .map_err(|error| error.message.to_string())
        };
        match set.and_then(|set| {
            validate_install_set_identity(&set.paths, &item)
                .map_err(|error| error.message.to_string())?;
            Ok(set)
        }) {
            Ok(set) => {
                sets.insert(spec.clone(), set);
            }
            Err(detail) => {
                set_failures.insert(spec.clone(), detail);
            }
        }
    }

    let state_ref: &AppState = &state;
    let results = collect_bounded(
        request.udids.iter().cloned(),
        MAX_INSTALL_CONCURRENCY,
        |udid| {
            let mut device_block = blocked.get(&udid).cloned();
            let paths = specs
                .get(&udid)
                .and_then(|spec| sets.get(spec))
                .map(|set| set.paths.as_slice());
            let expected_spec = (item.platform == AppLibraryPlatform::Android
                && item.package_format != AppPackageFormat::Apk)
                .then(|| specs.get(&udid))
                .flatten();
            if device_block.is_none() {
                if let Some(detail) = specs.get(&udid).and_then(|spec| set_failures.get(spec)) {
                    device_block = Some(AppInstallResult {
                        udid: udid.clone(),
                        status: AppInstallStatus::BeforeEffect,
                        effect_started: false,
                        observed_version_name: None,
                        observed_version_code: None,
                        detail: Some(detail.clone()),
                    });
                }
            }
            let request = &request;
            let batch = &batch;
            let item = &item;
            async move {
                if let Err(mut result) = install_dispatch_claim(
                    state_ref
                        .db
                        .claim_library_batch_item(&request.batch_id, &udid)
                        .map_err(|error| error.to_string()),
                    &udid,
                ) {
                    if result.status == AppInstallStatus::BeforeEffect {
                        if let Err(error) = persist_library_outcome(
                            state_ref,
                            &request.batch_id,
                            &udid,
                            OperationRunState::Failed,
                            Some("IntentPersistenceUnavailable"),
                            result.detail.as_deref(),
                            None,
                        ) {
                            result.detail =
                                Some(format!("{}; {error}", result.detail.unwrap_or_default()));
                        }
                    }
                    return result;
                }
                let mut result = install_one_batch_device(
                    InstallBatchDeviceContext {
                        state: state_ref,
                        request,
                        item,
                        paths,
                        expected_spec,
                        batch,
                    },
                    udid,
                    device_block,
                )
                .await;
                let status = match result.status {
                    AppInstallStatus::Succeeded => OperationRunState::Succeeded,
                    AppInstallStatus::Uncertain => OperationRunState::Uncertain,
                    AppInstallStatus::CancelledBeforeDispatch => OperationRunState::Cancelled,
                    AppInstallStatus::BeforeEffect | AppInstallStatus::FailedVerified => {
                        OperationRunState::Failed
                    }
                };
                let evidence = serde_json::to_string(&result).ok();
                if let Err(error) = persist_library_outcome(
                    state_ref,
                    &request.batch_id,
                    &result.udid,
                    status,
                    None,
                    result.detail.as_deref(),
                    evidence.as_deref(),
                ) {
                    result.status = AppInstallStatus::Uncertain;
                    result.detail = Some(error);
                }
                result
            }
        },
    )
    .await;
    let mut progress = Vec::with_capacity(results.len() * 5);
    for result in &results {
        progress.push(AppInstallProgress {
            udid: result.udid.clone(),
            phase: AppInstallProgressPhase::Pending,
            detail: None,
        });
        progress.push(AppInstallProgress {
            udid: result.udid.clone(),
            phase: AppInstallProgressPhase::Preparing,
            detail: None,
        });
        if result.effect_started {
            progress.push(AppInstallProgress {
                udid: result.udid.clone(),
                phase: AppInstallProgressPhase::Installing,
                detail: None,
            });
            progress.push(AppInstallProgress {
                udid: result.udid.clone(),
                phase: AppInstallProgressPhase::Reconciling,
                detail: None,
            });
        }
        progress.push(AppInstallProgress {
            udid: result.udid.clone(),
            phase: if result.status == AppInstallStatus::CancelledBeforeDispatch {
                AppInstallProgressPhase::CancelledBeforeDispatch
            } else {
                AppInstallProgressPhase::Complete
            },
            detail: result.detail.clone(),
        });
    }
    log(
        &state,
        "app.install.batch",
        &format!("{}:{}", request.batch_id, results.len()),
    );
    Ok(AppInstallBatchResponse {
        batch_id: request.batch_id,
        target,
        progress,
        results,
    })
}

/// Install one library item across a group. Each phone gets its own lease and outcome, so one
/// unavailable phone never hides successful installs on the rest of the group.
#[tauri::command]
pub async fn install_library_app_to_group(
    state: State<'_, AppState>,
    group_id: String,
    app_id: String,
) -> Result<Vec<GroupInstallResult>, CommandError> {
    let _admission = state.ensure_accepting_work()?;
    let group = state
        .db
        .list_groups()
        .map_err(err)?
        .into_iter()
        .find(|group| group.id == group_id)
        .ok_or_else(|| {
            CommandError::code("GroupNotFound", format!("group {group_id} not found"))
        })?;
    if group.udids.is_empty() {
        return Err(CommandError::invalid_argument("group has no devices"));
    }
    let response = install_library_app_batch(
        state,
        AppInstallRequest {
            batch_id: Uuid::new_v4().to_string(),
            app_id,
            udids: group.udids,
            allow_downgrade: false,
        },
    )
    .await?;
    Ok(response
        .results
        .into_iter()
        .map(|result| GroupInstallResult {
            udid: result.udid,
            ok: result.status == AppInstallStatus::Succeeded,
            error: (result.status != AppInstallStatus::Succeeded).then(|| {
                result
                    .detail
                    .unwrap_or_else(|| format!("install ended as {:?}", result.status))
            }),
        })
        .collect())
}

#[tauri::command]
pub fn list_schedules(state: State<'_, AppState>) -> Result<Vec<ScheduleItem>, CommandError> {
    state.db.list_schedules().map_err(err)
}

#[tauri::command]
pub fn save_schedule(
    state: State<'_, AppState>,
    schedule: ScheduleItem,
) -> Result<ScheduleItem, CommandError> {
    let _admission = state.ensure_accepting_work()?;
    let mut s = schedule;
    if s.id.is_empty() {
        s.id = Uuid::new_v4().to_string();
    }
    if s.every_minutes == 0 {
        s.every_minutes = 60;
    }
    s.next_run_at = Some((Utc::now() + Duration::minutes(s.every_minutes as i64)).to_rfc3339());
    state.db.upsert_schedule(&s).map_err(err)?;
    log(&state, "schedule.save", &s.name);
    Ok(s)
}

#[tauri::command]
pub fn delete_schedule(state: State<'_, AppState>, id: String) -> Result<(), CommandError> {
    let _admission = state.ensure_accepting_work()?;
    state.db.delete_schedule(&id).map_err(err)?;
    log(&state, "schedule.delete", &id);
    Ok(())
}

#[tauri::command]
pub fn list_op_logs(
    state: State<'_, AppState>,
    limit: Option<usize>,
) -> Result<Vec<OpLog>, CommandError> {
    state.db.list_op_logs(limit.unwrap_or(100)).map_err(err)
}

#[tauri::command]
pub fn analytics_summary(state: State<'_, AppState>) -> Result<AnalyticsSummary, CommandError> {
    let devices = state.registry.list();
    let ready = devices
        .iter()
        .filter(|d| d.wda_ready || matches!(d.status, riviu_core::DeviceStatus::Ready))
        .count();
    state
        .db
        .analytics_summary(devices.len(), ready)
        .map_err(err)
}

#[tauri::command]
pub fn api_docs() -> String {
    r#"# Riviu Manager local API (Tauri invoke)

## Devices
- list_devices / refresh_devices / prepare_device / reboot_device / device_health
- device_tap / device_swipe / device_type_text / device_home / group_input
- resign_wda / bulk_resign_wda / screenshot / syslog

## Farm data
- list_groups / save_group / delete_group
- list_materials / add_material / delete_material / push_material / push_material_batch
- list_apps_library / add_app_library / delete_app_library / install_library_app / uninstall_app
- list_schedules / save_schedule / delete_schedule
- publish_scan_folder / publish_create_campaign / publish_list / publish_get / publish_readiness
- publish_sheet_get_config / publish_sheet_save_config
- publish_execute / publish_cancel
- list_op_logs / analytics_summary

## Operations
- operation_list_runs / operation_query_runs / operation_get_run / operation_cancel_batch

## Sidecar
- python riviu_pmd.py list|install|uninstall|media-stage|stream|start-wda|...
"#
    .into()
}

#[cfg(test)]
mod android_app_library_tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    struct TempFixture(PathBuf);

    impl TempFixture {
        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempFixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn fixture_dir() -> TempFixture {
        let path = std::env::temp_dir().join(format!("riviu-app-fixture-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&path).expect("fixture root");
        TempFixture(path)
    }

    #[test]
    fn install_intent_failure_is_before_effect_and_not_a_cancellation() {
        let failure = install_dispatch_claim(Err("fixture journal unavailable".into()), "a")
            .expect_err("an intent failure must block dispatch");
        assert_eq!(failure.status, AppInstallStatus::BeforeEffect);
        assert!(!failure.effect_started);
        assert!(failure
            .detail
            .unwrap()
            .contains("fixture journal unavailable"));
        let cancelled = install_dispatch_claim(Ok(false), "a").unwrap_err();
        assert_eq!(cancelled.status, AppInstallStatus::CancelledBeforeDispatch);
        assert!(!cancelled.effect_started);
        assert!(install_dispatch_claim(Ok(true), "a").is_ok());
    }

    fn bundled_apk(name: &str) -> Vec<u8> {
        std::fs::read(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../..")
                .join("sidecars/android/noarch")
                .join(name),
        )
        .expect("bundled APK fixture")
    }

    fn write_zip(path: &Path, entries: &[(&str, &[u8])]) {
        let file = std::fs::File::create(path).expect("fixture archive");
        let mut writer = zip::ZipWriter::new(file);
        let options: zip::write::FileOptions<'_, ()> = zip::write::FileOptions::default();
        for (name, bytes) in entries {
            writer.start_file(*name, options).expect("fixture entry");
            writer.write_all(bytes).expect("fixture bytes");
        }
        writer.finish().expect("finish fixture");
    }

    fn proto_varint(mut value: u64) -> Vec<u8> {
        let mut encoded = Vec::new();
        loop {
            let mut byte = (value & 0x7f) as u8;
            value >>= 7;
            if value != 0 {
                byte |= 0x80;
            }
            encoded.push(byte);
            if value == 0 {
                return encoded;
            }
        }
    }

    fn proto_bytes(field: u32, value: &[u8]) -> Vec<u8> {
        let mut encoded = proto_varint(u64::from(field) << 3 | 2);
        encoded.extend(proto_varint(value.len() as u64));
        encoded.extend(value);
        encoded
    }

    fn apks_toc(package: &str, paths: &[&str], include_asset_slice: bool) -> Vec<u8> {
        let mut apk_set = Vec::new();
        for path in paths {
            let description = proto_bytes(2, path.as_bytes());
            apk_set.extend(proto_bytes(2, &description));
        }
        let variant = proto_bytes(2, &apk_set);
        let mut root = proto_bytes(1, &variant);
        root.extend(proto_bytes(4, package.as_bytes()));
        if include_asset_slice {
            root.extend(proto_bytes(3, &[]));
        }
        root
    }

    #[test]
    fn xapk_manifest_and_every_split_identity_are_verified() {
        let root = fixture_dir();
        let base = bundled_apk("riviu-agent.apk");
        let apk_path = root.path().join("base.apk");
        std::fs::write(&apk_path, &base).expect("base fixture");
        let identity = apk_info(&apk_path).expect("fixture metadata");
        let manifest = serde_json::json!({
            "package_name": identity.application_id,
            "split_apks": [{"file": "config.en.apk"}]
        })
        .to_string();
        let xapk = root.path().join("fixture.xapk");
        write_zip(
            &xapk,
            &[
                ("manifest.json", manifest.as_bytes()),
                (&format!("{}.apk", identity.application_id), &base),
                ("config.en.apk", &base),
            ],
        );
        let inspected =
            inspect_android_package(&xapk, AppPackageFormat::Xapk).expect("matching split set");
        assert_eq!(inspected.application_id, identity.application_id);

        let mismatched = bundled_apk("minicap.apk");
        let bad = root.path().join("mismatch.xapk");
        write_zip(
            &bad,
            &[
                ("manifest.json", manifest.as_bytes()),
                (&format!("{}.apk", identity.application_id), &base),
                ("config.en.apk", &mismatched),
            ],
        );
        let error = inspect_android_package(&bad, AppPackageFormat::Xapk)
            .expect_err("mismatched split identity must fail");
        assert!(error
            .message
            .contains("different package, version, or signer"));
    }

    #[test]
    fn apks_multi_variant_rejects_any_mismatched_apk_identity() {
        let root = fixture_dir();
        let base = bundled_apk("riviu-agent.apk");
        let apk_path = root.path().join("base.apk");
        std::fs::write(&apk_path, &base).expect("base fixture");
        let identity = apk_info(&apk_path).expect("fixture metadata");
        let toc = apks_toc(
            &identity.application_id,
            &["splits/base-master.apk", "splits/config.en.apk"],
            false,
        );
        let good = root.path().join("good.apks");
        write_zip(
            &good,
            &[
                ("toc.pb", &toc),
                ("splits/config.en.apk", &base),
                ("splits/base-master.apk", &base),
            ],
        );
        inspect_android_package(&good, AppPackageFormat::Apks)
            .expect("multi-variant APKS with a non-base first entry");

        let mismatched = bundled_apk("minicap.apk");
        let bad = root.path().join("bad.apks");
        write_zip(
            &bad,
            &[
                ("toc.pb", &toc),
                ("splits/base-master.apk", &base),
                ("splits/config.en.apk", &mismatched),
            ],
        );
        let error = inspect_android_package(&bad, AppPackageFormat::Apks)
            .expect_err("APKS split with another identity must fail");
        assert!(error
            .message
            .contains("different package, version, or signer"));
    }

    #[test]
    fn apkm_info_manifest_and_split_identity_are_verified() {
        let root = fixture_dir();
        let base = bundled_apk("riviu-agent.apk");
        let apk_path = root.path().join("base.apk");
        std::fs::write(&apk_path, &base).expect("base fixture");
        let identity = apk_info(&apk_path).expect("fixture metadata");
        let info = serde_json::json!({
            "pname": identity.application_id,
            "release_version": identity.version_name,
            "versioncode": identity.version_code,
            "split_apks": ["config.en.apk"]
        })
        .to_string();
        let apkm = root.path().join("fixture.apkm");
        write_zip(
            &apkm,
            &[
                ("info.json", info.as_bytes()),
                ("base.apk", &base),
                ("config.en.apk", &base),
            ],
        );
        let inspected =
            inspect_android_package(&apkm, AppPackageFormat::Apkm).expect("valid APKM fixture");
        assert_eq!(inspected.application_id, identity.application_id);
    }

    #[test]
    fn apks_requires_a_well_formed_table_of_contents() {
        let root = fixture_dir();
        let base = bundled_apk("riviu-agent.apk");
        let missing = root.path().join("missing-toc.apks");
        write_zip(&missing, &[("splits/base-master.apk", &base)]);
        assert!(safe_container_layout(&missing, AppPackageFormat::Apks)
            .expect_err("toc.pb is mandatory")
            .message
            .contains("toc.pb"));

        let corrupt = root.path().join("corrupt-toc.apks");
        write_zip(
            &corrupt,
            &[("toc.pb", b"\x0a\xff"), ("splits/base-master.apk", &base)],
        );
        assert!(safe_container_layout(&corrupt, AppPackageFormat::Apks).is_err());
    }

    #[test]
    fn apks_toc_rejects_play_asset_delivery() {
        let toc = apks_toc("com.riviu.fixture", &["splits/base-master.apk"], true);
        assert_eq!(
            parse_apks_toc(&toc)
                .expect_err("asset_slice_set must be rejected")
                .code,
            "UnsupportedPlayAssetDelivery"
        );
    }

    #[test]
    fn container_declared_identity_must_match_base_apk() {
        let root = fixture_dir();
        let base = bundled_apk("riviu-agent.apk");
        let apk_path = root.path().join("base.apk");
        std::fs::write(&apk_path, &base).expect("base fixture");
        let identity = apk_info(&apk_path).expect("fixture metadata");

        let wrong_package = "com.riviu.deliberately.wrong";
        let xapk_manifest = serde_json::json!({
            "package_name": wrong_package,
            "version_name": identity.version_name,
            "version_code": identity.version_code,
            "split_apks": []
        })
        .to_string();
        let xapk = root.path().join("wrong-identity.xapk");
        let xapk_base = format!("{wrong_package}.apk");
        write_zip(
            &xapk,
            &[
                ("manifest.json", xapk_manifest.as_bytes()),
                (&xapk_base, &base),
            ],
        );
        assert!(inspect_android_package(&xapk, AppPackageFormat::Xapk)
            .expect_err("declared package must match the base APK")
            .message
            .contains("declared package"));

        let toc = apks_toc(wrong_package, &["splits/base-master.apk"], false);
        let apks = root.path().join("wrong-identity.apks");
        write_zip(
            &apks,
            &[("toc.pb", &toc), ("splits/base-master.apk", &base)],
        );
        assert!(inspect_android_package(&apks, AppPackageFormat::Apks)
            .expect_err("toc package must match the base APK")
            .message
            .contains("declared package"));
    }

    #[test]
    fn malformed_and_missing_container_manifests_fail_closed() {
        let root = fixture_dir();
        let malformed = root.path().join("malformed.xapk");
        write_zip(
            &malformed,
            &[("manifest.json", b"{"), ("base.apk", b"not-an-apk")],
        );
        assert!(safe_container_layout(&malformed, AppPackageFormat::Xapk).is_err());

        let missing = root.path().join("missing.xapk");
        let manifest = br#"{"package_name":"com.example.missing","split_apks":[]}"#;
        write_zip(&missing, &[("manifest.json", manifest)]);
        assert!(safe_container_layout(&missing, AppPackageFormat::Xapk).is_err());

        let apkm = root.path().join("missing-manifest.apkm");
        write_zip(&apkm, &[("base.apk", b"not-an-apk")]);
        assert!(safe_container_layout(&apkm, AppPackageFormat::Apkm).is_err());
    }

    #[test]
    fn unsupported_bundle_and_expansion_payloads_have_stable_codes() {
        for (name, code) in [
            ("fixture.aab", "UnsupportedAndroidAppBundle"),
            ("main.1.com.example.obb", "UnsupportedObb"),
        ] {
            assert_eq!(
                package_kind(Path::new(name))
                    .expect_err("unsupported artifact")
                    .code,
                code
            );
        }

        let root = fixture_dir();
        let xapk_manifest = br#"{"package_name":"com.example.fixture","split_apks":[]}"#;
        let obb = root.path().join("obb.xapk");
        write_zip(
            &obb,
            &[
                ("manifest.json", xapk_manifest),
                ("Android/obb/main.1.fixture.obb", b"fixture"),
            ],
        );
        assert_eq!(
            safe_container_layout(&obb, AppPackageFormat::Xapk)
                .expect_err("OBB payload")
                .code,
            "UnsupportedObbPayload"
        );

        let pad = root.path().join("pad.xapk");
        write_zip(
            &pad,
            &[
                ("manifest.json", xapk_manifest),
                ("asset-pack/level-one/data.bin", b"fixture"),
            ],
        );
        assert_eq!(
            safe_container_layout(&pad, AppPackageFormat::Xapk)
                .expect_err("PAD payload")
                .code,
            "UnsupportedPlayAssetDelivery"
        );
    }

    #[test]
    fn portable_traversal_and_normalized_duplicate_names_are_rejected() {
        let root = fixture_dir();
        for unsafe_name in ["../escape.apk", "..\\escape.apk", "C:\\escape.apk"] {
            let path = root.path().join(format!("{}.xapk", Uuid::new_v4()));
            write_zip(&path, &[(unsafe_name, b"fixture")]);
            assert!(
                safe_container_layout(&path, AppPackageFormat::Xapk).is_err(),
                "accepted {unsafe_name}"
            );
        }
        let duplicate = root.path().join("duplicate.xapk");
        write_zip(
            &duplicate,
            &[("dir/base.apk", b"one"), ("dir\\base.apk", b"two")],
        );
        assert!(safe_container_layout(&duplicate, AppPackageFormat::Xapk).is_err());
    }

    #[test]
    fn split_selection_uses_device_abi_density_and_locale() {
        let spec = AndroidInstallDeviceSpec {
            sdk_version: 35,
            supported_abis: vec!["arm64-v8a".to_string()],
            screen_density: 480,
            supported_locales: vec!["vi-VN".to_string(), "en-US".to_string()],
        };
        assert!(split_is_compatible("config.arm64_v8a.apk", &spec));
        assert!(!split_is_compatible("config.x86.apk", &spec));
        assert!(split_is_compatible("config.xxhdpi.apk", &spec));
        assert!(!split_is_compatible("config.mdpi.apk", &spec));
        assert!(split_is_compatible("config.vi.apk", &spec));
        assert!(!split_is_compatible("config.fr.apk", &spec));
        assert!(!split_is_compatible("config.arm64_v8a.mdpi.vi.apk", &spec));
        assert!(split_is_compatible("config.arm64_v8a.xxhdpi.vi.apk", &spec));
        assert!(split_is_compatible("config.480dpi.apk", &spec));
        assert!(!split_is_compatible("config.320dpi.apk", &spec));
        assert!(split_is_compatible("config.sdk26.apk", &spec));
        assert!(!split_is_compatible("config.sdk36.apk", &spec));
        assert!(split_is_compatible("config.en-rUS.apk", &spec));
        assert!(!split_is_compatible("config.en-rGB.apk", &spec));
        assert!(!split_is_compatible("config.futureQualifier.apk", &spec));
        assert!(split_is_compatible("feature-payments.apk", &spec));
    }

    #[test]
    fn install_device_spec_reproof_accepts_a_to_a_and_rejects_a_to_b() {
        let original = AndroidInstallDeviceSpec {
            sdk_version: 35,
            supported_abis: vec!["arm64-v8a".to_string()],
            screen_density: 480,
            supported_locales: vec!["vi-VN".to_string()],
        };
        assert!(ensure_install_device_spec_unchanged(&original, &original).is_ok());
        let mut changed = original.clone();
        changed.screen_density = 420;
        assert!(ensure_install_device_spec_unchanged(&original, &changed)
            .expect_err("changed device spec must stop before install")
            .contains("changed"));
    }

    #[test]
    fn install_batch_scratch_path_never_uses_caller_path_components() {
        let artifacts = Path::new("C:/Riviu data/artifacts");
        let nonce = Uuid::nil();
        let ordinary = install_batch_scratch_root(artifacts, "app-install-123", nonce);
        let traversal = install_batch_scratch_root(artifacts, "../../outside", nonce);
        let absolute = install_batch_scratch_root(artifacts, "C:\\outside", nonce);

        for path in [&ordinary, &traversal, &absolute] {
            assert_eq!(
                path.parent().and_then(Path::parent),
                Some(artifacts.join("app-install-batches").as_path())
            );
            let digest = path
                .parent()
                .and_then(Path::file_name)
                .and_then(|value| value.to_str())
                .expect("hashed batch directory name");
            assert_eq!(digest.len(), 64);
            assert!(digest.bytes().all(|byte| byte.is_ascii_hexdigit()));
            let leaf = path
                .file_name()
                .and_then(|value| value.to_str())
                .expect("nonce directory name");
            assert_eq!(leaf, Uuid::nil().to_string());
        }
        assert_ne!(ordinary, traversal);
        assert_ne!(traversal, absolute);
    }

    #[test]
    fn managed_artifact_must_still_match_the_imported_hash_and_size() {
        let root = fixture_dir();
        let path = root.path().join("managed.apk");
        std::fs::write(&path, b"original bytes").expect("fixture artifact");
        let (sha256, size_bytes) = hash_file_with_sha256(&path).expect("fixture identity");
        let item = AppLibraryItem {
            id: "app-1".into(),
            name: "Managed".into(),
            path: path.display().to_string(),
            bundle_id: "com.example.managed".into(),
            version: "1".into(),
            platform: AppLibraryPlatform::Android,
            package_format: AppPackageFormat::Apk,
            artifact_kind: AppPackageFormat::Apk,
            application_id: "com.example.managed".into(),
            version_name: "1".into(),
            version_code: Some("1".into()),
            sha256,
            size_bytes,
            signer_sha256: "signer".into(),
            icon_png_base64: None,
            metadata_status: "verified".into(),
            metadata_error: None,
            created_at: "2026-09-02T00:00:00Z".into(),
        };
        let scratch = root.path().join("scratch");
        std::fs::create_dir_all(&scratch).expect("scratch");
        snapshot_managed_app_artifact(&item, &scratch).expect("unchanged managed artifact");

        std::fs::write(&path, b"mutated bytes!").expect("mutate fixture artifact");
        let error = snapshot_managed_app_artifact(&item, &scratch)
            .expect_err("mutated managed artifact must fail before device work");
        assert_eq!(error.code, "AppArtifactChanged");
    }

    #[test]
    fn signer_identity_contains_the_complete_sorted_certificate_set() {
        let first = serde_json::json!({
            "v3": [
                {"sha256_fingerprint": "BB"},
                {"sha256_fingerprint": "aa"}
            ]
        });
        let repeated_other_scheme = serde_json::json!({
            "v1": [{"sha256_fingerprint": "AA"}]
        });
        assert_eq!(
            canonical_signer_identity(&[first, repeated_other_scheme]),
            "aa,bb"
        );
        assert_ne!(
            canonical_signer_identity(&[serde_json::json!({
                "v2": [{"sha256_fingerprint": "aa"}]
            })]),
            canonical_signer_identity(&[serde_json::json!({
                "v2": [
                    {"sha256_fingerprint": "aa"},
                    {"sha256_fingerprint": "cc"}
                ]
            })])
        );
    }

    #[tokio::test]
    async fn fleet_install_work_never_exceeds_two_concurrent_devices() {
        let active = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let results = collect_bounded(0..8, MAX_INSTALL_CONCURRENCY, |value| {
            let active = Arc::clone(&active);
            let peak = Arc::clone(&peak);
            async move {
                let now = active.fetch_add(1, Ordering::SeqCst) + 1;
                peak.fetch_max(now, Ordering::SeqCst);
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
                active.fetch_sub(1, Ordering::SeqCst);
                value
            }
        })
        .await;
        assert_eq!(results.len(), 8);
        assert_eq!(peak.load(Ordering::SeqCst), MAX_INSTALL_CONCURRENCY);
    }

    #[tokio::test]
    async fn material_push_work_never_exceeds_two_concurrent_devices() {
        let active = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let results = collect_bounded(0..9, MAX_MATERIAL_PUSH_CONCURRENCY, |value| {
            let active = Arc::clone(&active);
            let peak = Arc::clone(&peak);
            async move {
                let now = active.fetch_add(1, Ordering::SeqCst) + 1;
                peak.fetch_max(now, Ordering::SeqCst);
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
                active.fetch_sub(1, Ordering::SeqCst);
                value
            }
        })
        .await;
        assert_eq!(results.len(), 9);
        assert_eq!(peak.load(Ordering::SeqCst), MAX_MATERIAL_PUSH_CONCURRENCY);
    }

    #[test]
    fn material_staging_guard_removes_copied_bytes_on_every_exit() {
        let parent = std::env::temp_dir().join(format!("riviu-material-stage-{}", Uuid::new_v4()));
        let attempt = parent.join("attempt");
        {
            let guard = MaterialStagingGuard::create(attempt.clone()).expect("create stage");
            let material = guard.root().join("material");
            std::fs::create_dir_all(&material).expect("create material directory");
            std::fs::write(material.join("fixture.mp4"), b"fixture bytes").expect("copy bytes");
            assert!(attempt.exists());
        }
        assert!(
            !attempt.exists(),
            "attempt bytes survived the staging guard"
        );
        let _ = std::fs::remove_dir_all(parent);
    }

    #[test]
    fn material_paths_never_use_operator_or_device_text_as_components() {
        let artifacts = Path::new("C:/Riviu data/artifacts");
        let materials = artifacts.join("materials");
        let source = Path::new("C:/nguon co dau/video thử.MP4");
        let stored = material_storage_path(&materials, "../../material:C:ads", source);
        assert_eq!(stored.parent(), Some(materials.as_path()));
        assert_eq!(
            stored.extension().and_then(|value| value.to_str()),
            Some("mp4")
        );
        assert!(!stored.to_string_lossy().contains("material:C:ads"));

        for serial in ["192.168.1.20:5555", "../../outside", "C:\\Windows\\Startup"] {
            let staged =
                material_staging_root(artifacts, serial, "..\\material:stream", Uuid::nil());
            assert_eq!(
                staged
                    .parent()
                    .and_then(Path::parent)
                    .and_then(Path::parent),
                Some(artifacts.join("push-staging").as_path())
            );
            for component in staged
                .strip_prefix(artifacts.join("push-staging"))
                .expect("staging remains under its root")
                .components()
                .take(2)
            {
                let component = component.as_os_str().to_string_lossy();
                assert_eq!(component.len(), 64);
                assert!(component.bytes().all(|byte| byte.is_ascii_hexdigit()));
            }
        }

        let staged_file = staged_material_path(Path::new("C:/stage"), source);
        assert_eq!(staged_file, Path::new("C:/stage/payload.mp4"));
    }

    #[test]
    fn managed_delete_is_confined_to_the_declared_library_root() {
        let fixture = fixture_dir();
        let root = fixture.path().join("materials");
        std::fs::create_dir_all(&root).expect("managed root");
        let managed = root.join("managed.mp4");
        let outside = fixture.path().join("outside.mp4");
        std::fs::write(&managed, b"managed").expect("managed fixture");
        std::fs::write(&outside, b"outside").expect("outside fixture");

        let error = remove_managed_file(&root, &outside.display().to_string())
            .expect_err("an external DB path must never be deleted");
        assert!(error.message.contains("escaped"));
        assert!(outside.exists());

        remove_managed_file(&root, &managed.display().to_string()).expect("managed delete");
        assert!(!managed.exists());
    }

    #[test]
    fn material_push_wire_contract_is_camel_case_and_carries_the_target_snapshot() {
        let request: MaterialPushBatchRequest = serde_json::from_value(serde_json::json!({
            "materialId": "material-a",
            "target": { "type": "explicit", "udids": ["phone-1", "phone-2"] }
        }))
        .expect("request");
        assert_eq!(request.material_id, "material-a");

        let response = MaterialPushBatchResult {
            batch_id: "batch-a".into(),
            material_id: request.material_id,
            target: resolve_target(&request.target, &[String::from("phone-1")], &[], &[])
                .expect("target"),
            results: vec![MaterialPushDeviceResult {
                udid: "phone-1".into(),
                status: MaterialPushStatus::Succeeded,
                evidence: Some("sha256=ok".into()),
                error_code: None,
                error: None,
            }],
        };
        let value = serde_json::to_value(response).expect("response");
        assert_eq!(value["batchId"], "batch-a");
        assert_eq!(value["materialId"], "material-a");
        assert_eq!(value["target"]["included"][0]["udid"], "phone-1");
        assert_eq!(value["results"][0]["status"], "succeeded");
    }

    #[test]
    fn cancel_is_one_shot_at_the_effect_gate_and_batch_ids_can_be_reused() {
        let batch_id = format!("cancel-fixture-{}", Uuid::new_v4());
        let claim = InstallBatchClaim::acquire(&batch_id).expect("claim batch ID");
        assert!(InstallBatchClaim::acquire(&batch_id).is_err());
        let started = claim.register_gate();
        let queued = claim.register_gate();
        assert!(started.claim_effect());

        cancel_active_app_install_batch(&batch_id).expect("cancel batch");
        assert!(started.effect_claimed());
        assert!(!started.is_cancelled());
        assert!(queued.is_cancelled());
        assert!(!queued.claim_effect());
        assert!(claim.register_gate().is_cancelled());

        drop(claim);
        assert!(cancel_active_app_install_batch(&batch_id).is_err());
        let reused = InstallBatchClaim::acquire(&batch_id).expect("reuse completed batch ID");
        assert!(!reused.is_cancelled());
    }
}
