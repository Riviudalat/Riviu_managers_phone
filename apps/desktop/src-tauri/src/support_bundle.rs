//! One zip that holds everything needed to diagnose this installation from another machine:
//! the logs, a consistent copy of the database, recent evidence files and the live fleet state.
//!
//! Secrets live in the OS credential store, not here, and nothing in this module reads that
//! store. What can still leak is text that happened to carry one — a header in a log line, a
//! legacy settings blob — so logs are redacted line by line here and the database copy is
//! masked by `Database::support_snapshot_into` before it is zipped.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant, SystemTime};

use anyhow::Context;

/// Evidence written within this window is included; older files are only counted.
const RECENT_ARTIFACTS: Duration = Duration::from_secs(48 * 60 * 60);
/// Upper bound on evidence bytes in one bundle. The artifact tree on the box PC passed 87 GB,
/// so "everything" is not a file anyone can send.
const RECENT_ARTIFACT_BYTES: u64 = 1_500 * 1024 * 1024;
/// A single evidence file larger than this is a video or a dump, not a frame or a JSON receipt.
const ARTIFACT_FILE_MAX: u64 = 32 * 1024 * 1024;
/// Walking the artifact tree is bounded. A tree with a million files still yields a bundle,
/// and the manifest says the inventory stopped early.
const ARTIFACT_SCAN_BUDGET: Duration = Duration::from_secs(90);
/// Small state next to the database (driver-state, completion journal) is copied up to this.
const DATA_FILES_BYTES: u64 = 64 * 1024 * 1024;
const ERRORS_KEPT: usize = 200;
const REDACTED: &str = riviu_core::db::SUPPORT_REDACTED;

static EXPORTING: AtomicBool = AtomicBool::new(false);

pub(crate) struct BundleInputs {
    pub db: std::sync::Arc<riviu_core::db::Database>,
    pub data_dir: PathBuf,
    pub artifacts_dir: PathBuf,
    pub log_dir: Option<PathBuf>,
    pub destination_dir: PathBuf,
    pub app: serde_json::Value,
    pub live: serde_json::Value,
}

pub(crate) struct BundleReceipt {
    pub path: PathBuf,
    pub bytes: u64,
    pub artifact_files: usize,
    pub inventory_complete: bool,
}

/// Build the bundle. One export at a time: a second click while one is writing is refused
/// instead of doubling a multi-hundred-megabyte snapshot.
pub(crate) fn export(inputs: BundleInputs) -> anyhow::Result<BundleReceipt> {
    anyhow::ensure!(
        !EXPORTING.swap(true, Ordering::SeqCst),
        "Đang xuất một gói log khác, chờ gói đó xong rồi bấm lại."
    );
    let result = write_bundle(&inputs);
    EXPORTING.store(false, Ordering::SeqCst);
    result
}

fn write_bundle(inputs: &BundleInputs) -> anyhow::Result<BundleReceipt> {
    std::fs::create_dir_all(&inputs.destination_dir)
        .with_context(|| format!("tạo thư mục {}", inputs.destination_dir.display()))?;
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
    let final_path = inputs
        .destination_dir
        .join(format!("riviu-log-{stamp}.zip"));
    let partial = final_path.with_extension("zip.partial");
    let snapshot = inputs
        .destination_dir
        .join(format!(".riviu-snapshot-{}.db", uuid::Uuid::new_v4()));
    let outcome = write_archive(inputs, &partial, &snapshot);
    let _ = std::fs::remove_file(&snapshot);
    let receipt = match outcome {
        Ok(receipt) => receipt,
        Err(error) => {
            let _ = std::fs::remove_file(&partial);
            return Err(error);
        }
    };
    std::fs::rename(&partial, &final_path).context("đặt tên gói log")?;
    Ok(BundleReceipt {
        path: final_path,
        ..receipt
    })
}

fn write_archive(
    inputs: &BundleInputs,
    partial: &Path,
    snapshot: &Path,
) -> anyhow::Result<BundleReceipt> {
    let mut errors = Vec::new();
    let mut archive = zip::ZipWriter::new(std::fs::File::create(partial)?);
    let deflate = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .large_file(true);
    let stored = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Stored)
        .large_file(true);

    // Database first: it is the one part a bundle is useless without.
    let schema_version = inputs.db.support_snapshot_into(snapshot)?;
    archive.start_file("db/riviu.db", deflate)?;
    std::io::copy(&mut std::fs::File::open(snapshot)?, &mut archive)?;

    let mut logs = Vec::new();
    if let Some(log_dir) = &inputs.log_dir {
        // Both builds' log folders sit side by side under %LOCALAPPDATA%; an operator who
        // switched between the base and Full installer has history in the other one.
        let mut folders = vec![(String::from("logs"), log_dir.clone())];
        if let Some(identifiers) = log_dir.parent().and_then(Path::parent) {
            let current = log_dir.parent().and_then(Path::file_name);
            for other in ["com.riviu.manager", "com.riviu.manager.full"] {
                if current.is_some_and(|name| name == other) {
                    continue;
                }
                folders.push((
                    format!("logs-other/{other}"),
                    identifiers.join(other).join("logs"),
                ));
            }
        }
        for (prefix, folder) in folders {
            let Ok(entries) = std::fs::read_dir(&folder) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if !path.is_file() {
                    continue;
                }
                let name = entry.file_name().to_string_lossy().into_owned();
                match std::fs::read(&path) {
                    Ok(bytes) => {
                        archive.start_file(format!("{prefix}/{name}"), deflate)?;
                        archive
                            .write_all(redact_text(&String::from_utf8_lossy(&bytes)).as_bytes())?;
                        logs.push(format!("{prefix}/{name}"));
                    }
                    Err(error) => note(&mut errors, format!("{}: {error}", path.display())),
                }
            }
        }
    }

    // Small state beside the database. Old pre-migration backups are listed, not copied:
    // each is a full older database and the snapshot above already supersedes them.
    let mut data_listing = Vec::new();
    let mut data_bytes = 0u64;
    if let Ok(entries) = std::fs::read_dir(&inputs.data_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
            data_listing
                .push(serde_json::json!({"name": name, "bytes": size, "dir": path.is_dir()}));
            let skip = name == "artifacts"
                || name.starts_with("riviu.db")
                || name.ends_with(".db")
                || name.ends_with(".db-wal")
                || name.ends_with(".db-shm")
                || name.starts_with('.');
            if skip {
                continue;
            }
            for (file, relative) in
                walk_files(&path, &name, Instant::now() + Duration::from_secs(10)).0
            {
                let Ok(meta) = file.metadata() else { continue };
                if data_bytes + meta.len() > DATA_FILES_BYTES {
                    note(&mut errors, format!("bỏ qua (vượt giới hạn): {relative}"));
                    continue;
                }
                data_bytes += meta.len();
                archive.start_file(format!("data/{relative}"), deflate)?;
                if let Err(error) = std::io::copy(&mut std::fs::File::open(&file)?, &mut archive) {
                    note(&mut errors, format!("{relative}: {error}"));
                }
            }
        }
    }

    let (artifact_files, artifact_bytes, inventory, inventory_complete) = add_recent_artifacts(
        &mut archive,
        &inputs.artifacts_dir,
        deflate,
        stored,
        &mut errors,
    )?;

    let manifest = serde_json::json!({
        "format": 1,
        "createdAt": chrono::Local::now().to_rfc3339(),
        "app": inputs.app,
        "os": {"family": std::env::consts::OS, "arch": std::env::consts::ARCH},
        "paths": {
            "data": inputs.data_dir.display().to_string(),
            "artifacts": inputs.artifacts_dir.display().to_string(),
            "logs": inputs.log_dir.as_ref().map(|p| p.display().to_string()),
        },
        "database": {"schemaVersion": schema_version, "entry": "db/riviu.db"},
        "logs": logs,
        "dataDir": data_listing,
        "artifacts": {
            "includedWindowHours": RECENT_ARTIFACTS.as_secs() / 3600,
            "includedFiles": artifact_files,
            "includedBytes": artifact_bytes,
            "byteCap": RECENT_ARTIFACT_BYTES,
            "inventoryComplete": inventory_complete,
            "inventory": inventory,
        },
        "live": inputs.live,
        "errors": errors,
    });
    archive.start_file("manifest.json", deflate)?;
    archive.write_all(serde_json::to_string_pretty(&manifest)?.as_bytes())?;
    archive.start_file("README.txt", deflate)?;
    archive.write_all(README.as_bytes())?;
    let file = archive.finish()?;
    file.sync_all()?;
    let bytes = file.metadata()?.len();
    Ok(BundleReceipt {
        path: partial.to_path_buf(),
        bytes,
        artifact_files,
        inventory_complete,
    })
}

const README: &str = "Gói log Riviu Manager\r\n\
\r\n\
manifest.json  phiên bản app, đường dẫn, danh sách máy lúc xuất, thống kê thư mục bằng chứng\r\n\
db/riviu.db    bản chụp nhất quán của database (chiến dịch, lượt đăng, từng lần thử, xác minh, Sheet, lịch, nhật ký máy)\r\n\
logs/          log của app (đã che chuỗi giống mật khẩu, cookie, token)\r\n\
data/          trạng thái nhỏ cạnh database (driver-state, ...)\r\n\
artifacts/     ảnh và file bằng chứng ghi trong 48 giờ gần nhất (có giới hạn dung lượng)\r\n\
\r\n\
Không có trong gói: mật khẩu, cookie, token Google/API (nằm trong kho bảo mật Windows, không bao giờ được đọc khi xuất).\r\n";

type Inventory = std::collections::BTreeMap<String, serde_json::Value>;

fn add_recent_artifacts(
    archive: &mut zip::ZipWriter<std::fs::File>,
    root: &Path,
    deflate: zip::write::SimpleFileOptions,
    stored: zip::write::SimpleFileOptions,
    errors: &mut Vec<String>,
) -> anyhow::Result<(usize, u64, Inventory, bool)> {
    let cutoff = SystemTime::now()
        .checked_sub(RECENT_ARTIFACTS)
        .unwrap_or(SystemTime::UNIX_EPOCH);
    let (files, complete) = walk_files(root, "", Instant::now() + ARTIFACT_SCAN_BUDGET);
    let mut totals: std::collections::BTreeMap<String, (u64, u64)> = Default::default();
    let mut recent = Vec::new();
    for (path, relative) in files {
        let Ok(meta) = path.metadata() else { continue };
        let top = relative.split('/').next().unwrap_or("").to_string();
        let entry = totals.entry(top).or_default();
        entry.0 += 1;
        entry.1 += meta.len();
        let modified = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
        if modified >= cutoff && meta.len() <= ARTIFACT_FILE_MAX && !is_bulk_media(&path) {
            recent.push((modified, meta.len(), path, relative));
        }
    }
    recent.sort_by_key(|entry| std::cmp::Reverse(entry.0));
    let mut included = 0usize;
    let mut bytes = 0u64;
    for (_, size, path, relative) in recent {
        if bytes + size > RECENT_ARTIFACT_BYTES {
            note(
                errors,
                format!("dừng thêm bằng chứng ở {relative}: đã đủ giới hạn dung lượng"),
            );
            break;
        }
        let options = if is_compressed(&path) {
            stored
        } else {
            deflate
        };
        archive.start_file(format!("artifacts/{relative}"), options)?;
        match std::fs::File::open(&path).and_then(|mut f| std::io::copy(&mut f, archive)) {
            Ok(_) => {
                included += 1;
                bytes += size;
            }
            Err(error) => note(errors, format!("{relative}: {error}")),
        }
    }
    let inventory = totals
        .into_iter()
        .map(|(top, (files, bytes))| (top, serde_json::json!({"files": files, "bytes": bytes})))
        .collect();
    Ok((included, bytes, inventory, complete))
}

/// Files under `root`, depth-first, with their `/`-joined path relative to it. Returns
/// `false` alongside when the deadline cut the walk short. Symlinks are not followed.
fn walk_files(root: &Path, prefix: &str, deadline: Instant) -> (Vec<(PathBuf, String)>, bool) {
    let mut out = Vec::new();
    if root.is_file() {
        out.push((root.to_path_buf(), prefix.to_string()));
        return (out, true);
    }
    let mut stack = vec![(root.to_path_buf(), prefix.to_string())];
    while let Some((dir, relative)) = stack.pop() {
        if Instant::now() > deadline {
            return (out, false);
        }
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            let name = entry.file_name().to_string_lossy().into_owned();
            let child = if relative.is_empty() {
                name
            } else {
                format!("{relative}/{name}")
            };
            if kind.is_dir() {
                stack.push((entry.path(), child));
            } else if kind.is_file() {
                out.push((entry.path(), child));
            }
        }
    }
    (out, true)
}

fn extension(path: &Path) -> String {
    path.extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default()
}

fn is_bulk_media(path: &Path) -> bool {
    matches!(
        extension(path).as_str(),
        "mp4" | "mov" | "mkv" | "webm" | "apk" | "ipa" | "zip" | "db"
    )
}

fn is_compressed(path: &Path) -> bool {
    matches!(
        extension(path).as_str(),
        "png" | "jpg" | "jpeg" | "webp" | "gz"
    )
}

fn note(errors: &mut Vec<String>, message: String) {
    if errors.len() < ERRORS_KEPT {
        errors.push(message);
    }
}

/// Mask what follows a credential marker in free text, and well-known token shapes anywhere.
fn redact_text(text: &str) -> String {
    const MARKERS: &[&str] = &[
        "bearer ",
        "authorization: ",
        "authorization=",
        "cookie: ",
        "cookie=",
        "password=",
        "password: ",
        "\"password\":\"",
        "\"password\": \"",
        "access_token=",
        "refresh_token=",
        "\"access_token\":\"",
        "\"refresh_token\":\"",
        "\"accesstoken\":\"",
        "\"refreshtoken\":\"",
        "api_key=",
        "apikey=",
        "\"apikey\":\"",
        "x-api-key: ",
        "sessionid=",
    ];
    const PREFIXES: &[&str] = &["ya29.", "sk-or-v1-", "sk-ant-", "aiza", "-----begin"];
    let lower = text.to_ascii_lowercase();
    // (start, end) byte ranges to mask; ASCII lowercasing keeps offsets aligned.
    let mut ranges = Vec::new();
    for marker in MARKERS {
        for (at, _) in lower.match_indices(marker) {
            let start = at + marker.len();
            ranges.push((start, secret_end(text, start)));
        }
    }
    for prefix in PREFIXES {
        for (at, _) in lower.match_indices(prefix) {
            let end = if *prefix == "-----begin" {
                lower[at..]
                    .find("-----end")
                    .map_or(text.len(), |offset| at + offset)
            } else {
                secret_end(text, at)
            };
            ranges.push((at, end));
        }
    }
    if ranges.is_empty() {
        return text.to_string();
    }
    ranges.sort_unstable();
    let mut out = String::with_capacity(text.len());
    let mut cursor = 0;
    for (start, end) in ranges {
        if end <= start || end <= cursor {
            continue;
        }
        let start = start.max(cursor);
        out.push_str(&text[cursor..start]);
        out.push_str(REDACTED);
        cursor = end;
    }
    out.push_str(&text[cursor..]);
    out
}

fn secret_end(text: &str, start: usize) -> usize {
    text[start..]
        .find(|c: char| {
            c.is_whitespace() || matches!(c, '"' | '\'' | ',' | ';' | '&' | '}' | ']' | ')')
        })
        .map_or(text.len(), |offset| start + offset)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redaction_masks_credentials_and_keeps_the_diagnosis() {
        let line = "WARN sheet write failed Authorization: Bearer ya29.abc-DEF_1 status=401 \
                    body={\"password\":\"hunter2\",\"claimToken\":\"c1\"} key=sk-or-v1-xyz";
        let redacted = redact_text(line);
        for secret in ["ya29.abc", "hunter2", "sk-or-v1-xyz"] {
            assert!(!redacted.contains(secret), "{secret} leaked: {redacted}");
        }
        for kept in ["sheet write failed", "status=401", "\"claimToken\":\"c1\""] {
            assert!(redacted.contains(kept), "{kept} lost: {redacted}");
        }
    }
}
