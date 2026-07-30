use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::io::{Cursor, Read, Seek};
use std::path::Path;

use anyhow::{bail, Context, Result};
use plist::{Dictionary, Value};
use sha2::{Digest, Sha256};
use zip::ZipArchive;

use crate::model::{BundleInfo, FileDigest};
use crate::redact;

const MAX_ENTRY_BYTES: u64 = 128 * 1024 * 1024;
const MAX_TOTAL_UNCOMPRESSED_BYTES: u64 = 512 * 1024 * 1024;
const MAX_ARCHIVE_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const MAX_ENTRIES: usize = 10_000;

#[derive(Clone, Copy)]
struct ArchiveLimits {
    archive_bytes: u64,
    entry_bytes: u64,
    total_uncompressed_bytes: u64,
    entries: usize,
}

const DEFAULT_LIMITS: ArchiveLimits = ArchiveLimits {
    archive_bytes: MAX_ARCHIVE_BYTES,
    entry_bytes: MAX_ENTRY_BYTES,
    total_uncompressed_bytes: MAX_TOTAL_UNCOMPRESSED_BYTES,
    entries: MAX_ENTRIES,
};

pub struct ArchiveData {
    pub artifact: FileDigest,
    pub entries: Vec<FileDigest>,
    pub bundles: Vec<BundleInfo>,
    pub redaction_count: usize,
    entry_bytes: BTreeMap<String, Vec<u8>>,
    executable_paths: BTreeSet<String>,
}

impl std::fmt::Debug for ArchiveData {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ArchiveData")
            .field("artifact", &self.artifact)
            .field("entries", &self.entries)
            .field("bundles", &self.bundles)
            .field("redaction_count", &self.redaction_count)
            .field("regular_entry_count", &self.entry_bytes.len())
            .field("executable_count", &self.executable_paths.len())
            .finish()
    }
}

impl ArchiveData {
    pub fn entry_bytes(&self, path: &str) -> Option<&[u8]> {
        self.entry_bytes.get(path).map(Vec::as_slice)
    }

    pub fn regular_entry_paths(&self) -> impl Iterator<Item = &str> {
        self.entry_bytes.keys().map(String::as_str)
    }
}

pub fn macho_candidates(archive: &ArchiveData) -> Vec<(&str, &[u8])> {
    let mut paths = BTreeSet::new();
    paths.extend(archive.executable_paths.iter().map(String::as_str));
    for path in archive.entry_bytes.keys() {
        if path.contains("/Contents/Resources/DWARF/") {
            paths.insert(path.as_str());
        }
    }

    paths
        .into_iter()
        .filter_map(|path| archive.entry_bytes(path).map(|bytes| (path, bytes)))
        .collect()
}

pub fn mobileprovision_candidates(archive: &ArchiveData) -> Vec<(&str, &[u8])> {
    archive
        .entry_bytes
        .iter()
        .filter(|(path, _)| {
            path.as_str() == "embedded.mobileprovision"
                || path.ends_with("/embedded.mobileprovision")
        })
        .map(|(path, bytes)| (path.as_str(), bytes.as_slice()))
        .collect()
}

pub fn read_ipa(path: &Path) -> Result<ArchiveData> {
    read_ipa_with_limits(path, DEFAULT_LIMITS)
}

fn read_ipa_with_limits(path: &Path, limits: ArchiveLimits) -> Result<ArchiveData> {
    let input_label = redact::all(&path.to_string_lossy()).0;
    let mut file = File::open(path).with_context(|| format!("open IPA: {input_label}"))?;
    let metadata = file
        .metadata()
        .with_context(|| format!("read IPA metadata: {input_label}"))?;
    if metadata.len() > limits.archive_bytes {
        bail!(
            "IPA is too large: {} bytes exceeds {}",
            metadata.len(),
            limits.archive_bytes
        );
    }

    let sha256 = hash_reader(&mut file)?;
    file.rewind().context("rewind IPA after hashing")?;
    let stable_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .context("IPA path must end in a UTF-8 filename")?;
    let (artifact_path, mut redaction_count) = redact::all(stable_name);
    let artifact = FileDigest {
        path: artifact_path,
        size: metadata.len(),
        sha256,
    };

    let mut archive = ZipArchive::new(file).context("parse IPA ZIP directory")?;
    if archive.len() > limits.entries {
        bail!(
            "IPA has too many ZIP entries: {} exceeds {}",
            archive.len(),
            limits.entries
        );
    }

    // Validate the complete directory before parsing or retaining any entry content.
    let mut planned_entries = Vec::with_capacity(archive.len());
    let mut seen_paths = BTreeSet::new();
    let mut declared_total = 0_u64;
    for index in 0..archive.len() {
        let entry = archive
            .by_index(index)
            .with_context(|| format!("read ZIP directory entry {index}"))?;
        let normalized = normalize_entry_path(entry.name())?;
        if !seen_paths.insert(normalized.clone()) {
            bail!(
                "duplicate normalized ZIP entry path: {}",
                redact::all(&normalized).0
            );
        }
        if entry.is_file() {
            if entry.size() > limits.entry_bytes {
                bail!(
                    "ZIP entry is too large: {} is {} bytes",
                    redact::all(&normalized).0,
                    entry.size()
                );
            }
            declared_total = declared_total
                .checked_add(entry.size())
                .context("ZIP declared size total overflow")?;
            if declared_total > limits.total_uncompressed_bytes {
                bail!(
                    "ZIP uncompressed data is too large: {declared_total} bytes exceeds {}",
                    limits.total_uncompressed_bytes
                );
            }
        }
        planned_entries.push((index, normalized, entry.is_file(), entry.size()));
    }

    let mut entries = Vec::new();
    let mut entry_bytes = BTreeMap::new();
    let mut reported_paths = BTreeSet::new();
    let mut actual_total = 0_u64;
    for (index, normalized, is_file, declared_size) in planned_entries {
        if !is_file {
            continue;
        }

        let mut entry = archive
            .by_index(index)
            .with_context(|| format!("open ZIP entry: {}", redact::all(&normalized).0))?;
        let mut bytes = Vec::new();
        (&mut entry)
            .take(limits.entry_bytes + 1)
            .read_to_end(&mut bytes)
            .with_context(|| format!("read ZIP entry: {}", redact::all(&normalized).0))?;
        if bytes.len() as u64 > limits.entry_bytes {
            bail!(
                "ZIP entry expanded past size limit: {}",
                redact::all(&normalized).0
            );
        }
        if bytes.len() as u64 != declared_size {
            bail!(
                "ZIP entry size mismatch: {} declared {declared_size}, read {}",
                redact::all(&normalized).0,
                bytes.len()
            );
        }
        actual_total = actual_total
            .checked_add(bytes.len() as u64)
            .context("ZIP actual size total overflow")?;
        if actual_total > limits.total_uncompressed_bytes {
            bail!("ZIP expanded past total uncompressed size limit");
        }

        let (reported_path, replacements) = redact::all(&normalized);
        redaction_count += replacements;
        if !reported_paths.insert(reported_path.clone()) {
            bail!("distinct ZIP entries collapse to one redacted report path");
        }
        entries.push(FileDigest {
            path: reported_path,
            size: bytes.len() as u64,
            sha256: sha256_bytes(&bytes),
        });
        entry_bytes.insert(normalized, bytes);
    }
    entries.sort_by(|left, right| left.path.cmp(&right.path));

    let mut source = archive.into_inner();
    let final_size = source.metadata().context("re-read IPA metadata")?.len();
    source.rewind().context("rewind IPA for mutation check")?;
    let final_sha256 = hash_reader(&mut source)?;
    if final_size != artifact.size || final_sha256 != artifact.sha256 {
        bail!("IPA changed while it was being inventoried");
    }

    let mut bundles = Vec::new();
    let mut executable_paths = BTreeSet::new();
    for (plist_path, bytes) in &entry_bytes {
        if plist_path != "Info.plist" && !plist_path.ends_with("/Info.plist") {
            continue;
        }
        let value = Value::from_reader(Cursor::new(bytes))
            .with_context(|| format!("parse plist: {}", redact::all(plist_path).0))?;
        let dictionary = value.as_dictionary().with_context(|| {
            format!(
                "root plist value is not a dictionary: {}",
                redact::all(plist_path).0
            )
        })?;
        bundles.push(bundle_info(
            plist_path,
            dictionary,
            &entry_bytes,
            &mut executable_paths,
            &mut redaction_count,
        )?);
    }
    bundles.sort_by(|left, right| left.plist_path.cmp(&right.plist_path));

    Ok(ArchiveData {
        artifact,
        entries,
        bundles,
        redaction_count,
        entry_bytes,
        executable_paths,
    })
}

fn bundle_info(
    plist_path: &str,
    dictionary: &Dictionary,
    entry_bytes: &BTreeMap<String, Vec<u8>>,
    executable_paths: &mut BTreeSet<String>,
    redaction_count: &mut usize,
) -> Result<BundleInfo> {
    let (reported_plist_path, replacements) = redact::all(plist_path);
    *redaction_count += replacements;

    let raw_executable = raw_plist_string(dictionary, "CFBundleExecutable", plist_path)?;
    let executable_path = match raw_executable {
        Some(executable) => {
            let executable = normalize_entry_path(executable)
                .context("CFBundleExecutable must be a safe relative filename")?;
            if executable.contains('/') {
                bail!("CFBundleExecutable must contain exactly one filename");
            }
            let parent = plist_path.rsplit_once('/').map_or("", |(parent, _)| parent);
            let resolved = if parent.is_empty() {
                executable
            } else {
                format!("{parent}/{executable}")
            };
            if !entry_bytes.contains_key(&resolved) {
                bail!(
                    "CFBundleExecutable does not resolve to a regular ZIP entry: {}",
                    redact::all(&resolved).0
                );
            }
            executable_paths.insert(resolved.clone());
            let (reported, replacements) = redact::all(&resolved);
            *redaction_count += replacements;
            Some(reported)
        }
        None => None,
    };

    Ok(BundleInfo {
        plist_path: reported_plist_path,
        bundle_id: plist_string(
            dictionary,
            "CFBundleIdentifier",
            plist_path,
            redaction_count,
        )?,
        executable_path,
        short_version: plist_string(
            dictionary,
            "CFBundleShortVersionString",
            plist_path,
            redaction_count,
        )?,
        build_version: plist_string(dictionary, "CFBundleVersion", plist_path, redaction_count)?,
        minimum_os_version: plist_string(
            dictionary,
            "MinimumOSVersion",
            plist_path,
            redaction_count,
        )?,
        dt_xcode: plist_string(dictionary, "DTXcode", plist_path, redaction_count)?,
        dt_sdk_name: plist_string(dictionary, "DTSDKName", plist_path, redaction_count)?,
    })
}

fn raw_plist_string<'a>(
    dictionary: &'a Dictionary,
    key: &str,
    plist_path: &str,
) -> Result<Option<&'a str>> {
    let Some(value) = dictionary.get(key) else {
        return Ok(None);
    };
    value.as_string().map(Some).with_context(|| {
        format!(
            "plist field {key} is not a string: {}",
            redact::all(plist_path).0
        )
    })
}

fn plist_string(
    dictionary: &Dictionary,
    key: &str,
    plist_path: &str,
    redaction_count: &mut usize,
) -> Result<Option<String>> {
    let Some(value) = raw_plist_string(dictionary, key, plist_path)? else {
        return Ok(None);
    };
    let (value, replacements) = redact::all(value);
    *redaction_count += replacements;
    Ok(Some(value))
}

fn normalize_entry_path(raw: &str) -> Result<String> {
    let reported = redact::all(raw).0;
    if raw.contains('\0') {
        bail!("unsafe ZIP entry path: NUL in {reported}");
    }
    let slash_path = raw.replace('\\', "/");
    if slash_path.is_empty() || slash_path.starts_with('/') {
        bail!("unsafe ZIP entry path: {reported}");
    }
    let slash_path = slash_path.strip_suffix('/').unwrap_or(&slash_path);
    if slash_path.is_empty() || slash_path.ends_with('/') {
        bail!("unsafe ZIP entry path: empty name");
    }
    let mut components = Vec::new();
    for (index, component) in slash_path.split('/').enumerate() {
        let drive_prefix = index == 0
            && component.as_bytes().get(1) == Some(&b':')
            && component.as_bytes()[0].is_ascii_alphabetic();
        if component.is_empty() || component == "." || component == ".." || drive_prefix {
            bail!("unsafe ZIP entry path: {reported}");
        }
        components.push(component);
    }
    Ok(components.join("/"))
}

fn hash_reader(mut reader: impl Read) -> Result<String> {
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = reader.read(&mut buffer).context("hash IPA")?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format_digest(hasher.finalize()))
}

fn sha256_bytes(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format_digest(hasher.finalize())
}

fn format_digest(bytes: impl AsRef<[u8]>) -> String {
    bytes
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use tempfile::TempDir;
    use zip::write::SimpleFileOptions;
    use zip::ZipWriter;

    use super::{read_ipa_with_limits, ArchiveLimits};

    #[test]
    fn rejects_cumulative_uncompressed_size_before_reading_entries() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("oversized.ipa");
        let mut writer = ZipWriter::new(std::fs::File::create(&path).unwrap());
        for name in ["one", "two"] {
            writer
                .start_file(name, SimpleFileOptions::default())
                .unwrap();
            writer.write_all(b"123456").unwrap();
        }
        writer.finish().unwrap();

        let error = read_ipa_with_limits(
            &path,
            ArchiveLimits {
                archive_bytes: 1024,
                entry_bytes: 10,
                total_uncompressed_bytes: 10,
                entries: 10,
            },
        )
        .unwrap_err();

        assert!(error
            .to_string()
            .contains("ZIP uncompressed data is too large"));
    }
}
