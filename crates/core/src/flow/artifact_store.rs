use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};

use anyhow::{anyhow, bail, ensure, Context};
use image::ImageFormat;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use super::{validate_artifact_label, FlowArtifactRecord};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PreparedArtifact {
    pub id: Uuid,
    pub relative_path: PathBuf,
    pub kind: String,
    pub size: u64,
    pub sha256: String,
    #[serde(skip)]
    temp_path: PathBuf,
    #[serde(skip)]
    final_path: PathBuf,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub enum ArtifactReconciliationFailureCode {
    Missing,
    HashMismatch,
    QuarantinedOrphan,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactReconciliationFailure {
    pub artifact_id: Uuid,
    pub code: ArtifactReconciliationFailureCode,
}

#[derive(Clone)]
pub struct FlowArtifactStore {
    root: PathBuf,
    quarantine: PathBuf,
}

impl FlowArtifactStore {
    pub fn new(root: impl AsRef<Path>) -> anyhow::Result<Self> {
        fs::create_dir_all(root.as_ref())
            .with_context(|| format!("create Flow artifact root {}", root.as_ref().display()))?;
        let root = root
            .as_ref()
            .canonicalize()
            .context("canonicalize Flow artifact root")?;
        ensure!(root.is_dir(), "Flow artifact root is not a directory");

        let staging = root.join(".staging");
        let quarantine = root.join(".quarantine");
        fs::create_dir_all(&staging).context("create Flow artifact staging directory")?;
        fs::create_dir_all(&quarantine).context("create Flow artifact quarantine directory")?;
        let staging = staging
            .canonicalize()
            .context("canonicalize Flow artifact staging directory")?;
        let quarantine = quarantine
            .canonicalize()
            .context("canonicalize Flow artifact quarantine directory")?;
        ensure!(staging.starts_with(&root), "staging directory escaped root");
        ensure!(
            quarantine.starts_with(&root),
            "quarantine directory escaped root"
        );

        Ok(Self { root, quarantine })
    }

    pub fn validate_label(&self, label: &str, format: &str) -> anyhow::Result<()> {
        validate_artifact_label(label, format).map_err(|code| anyhow!(code))
    }

    pub fn prepare_image(
        &self,
        run_id: Uuid,
        device_run_id: Uuid,
        attempt_id: Uuid,
        label: &str,
        format: &str,
        bytes: &[u8],
    ) -> anyhow::Result<PreparedArtifact> {
        self.validate_label(label, format)?;
        ensure!(!bytes.is_empty(), "ArtifactImageEmpty");
        let (image_format, extension) = expected_image_format(format)?;
        ensure!(
            image::guess_format(bytes).context("detect artifact image format")? == image_format,
            "ArtifactImageFormatMismatch"
        );
        image::load_from_memory_with_format(bytes, image_format)
            .context("decode artifact image")?;

        let id = Uuid::new_v4();
        let relative_path = PathBuf::from(run_id.to_string())
            .join(device_run_id.to_string())
            .join(attempt_id.to_string())
            .join(format!("{id}.{extension}"));
        validate_generated_relative_path(&relative_path, id, Some(attempt_id), format)?;

        let final_parent =
            self.ensure_artifact_directory_chain(&[run_id, device_run_id, attempt_id])?;
        let final_path = self.root.join(&relative_path);
        ensure!(
            final_path.parent() == Some(final_parent.as_path()),
            "artifact final directory mismatch"
        );

        let temp_path = self.root.join(".staging").join(format!("{id}.tmp"));
        self.ensure_managed_directory(
            temp_path
                .parent()
                .context("artifact staging path has no parent")?,
        )?;
        let write_result = (|| -> anyhow::Result<()> {
            let mut file = OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&temp_path)
                .context("create staged artifact")?;
            file.write_all(bytes).context("write staged artifact")?;
            file.sync_all().context("sync staged artifact")?;
            Ok(())
        })();
        if let Err(error) = write_result {
            let _ = self.remove_managed_file_if_present(&temp_path);
            return Err(error);
        }

        Ok(PreparedArtifact {
            id,
            relative_path,
            kind: format.to_string(),
            size: u64::try_from(bytes.len()).context("artifact size exceeds u64")?,
            sha256: sha256_bytes(bytes),
            temp_path,
            final_path,
        })
    }

    pub fn publish_file(&self, artifact: &PreparedArtifact) -> anyhow::Result<PathBuf> {
        self.validate_prepared(artifact)?;
        ensure_path_missing(&artifact.final_path, "artifact final path already exists")?;
        let staged = self.ensure_managed_file(&artifact.temp_path)?;
        let (size, sha256) = file_identity(&staged)?;
        ensure!(size == artifact.size, "staged artifact size changed");
        ensure!(sha256 == artifact.sha256, "staged artifact hash changed");
        self.ensure_managed_directory(
            artifact
                .final_path
                .parent()
                .context("artifact final path has no parent")?,
        )?;

        fs::rename(&staged, &artifact.final_path).context("publish artifact atomically")?;
        OpenOptions::new()
            .read(true)
            .write(true)
            .open(&artifact.final_path)
            .context("open published artifact")?
            .sync_all()
            .context("sync published artifact")?;
        Ok(artifact.relative_path.clone())
    }

    pub fn rollback_file(&self, artifact: &PreparedArtifact) -> anyhow::Result<()> {
        self.validate_prepared(artifact)?;
        self.remove_managed_file_if_present(&artifact.final_path)?;
        self.remove_managed_file_if_present(&artifact.temp_path)?;
        Ok(())
    }

    pub fn reconcile(
        &self,
        rows: &[FlowArtifactRecord],
    ) -> anyhow::Result<Vec<ArtifactReconciliationFailure>> {
        self.remove_stale_staging_files()?;

        let mut committed = BTreeMap::new();
        let mut failures = Vec::new();
        for row in rows {
            self.validate_label(&row.label, &row.kind)?;
            ensure!(row.size > 0, "committed artifact has zero size");
            ensure!(
                is_lower_sha256(&row.sha256),
                "committed artifact hash is invalid"
            );
            let relative = PathBuf::from(&row.relative_path);
            validate_generated_relative_path(&relative, row.id, Some(row.attempt_id), &row.kind)?;
            ensure!(
                committed.insert(relative.clone(), row).is_none(),
                "duplicate committed artifact path"
            );

            let path = self.root.join(&relative);
            match fs::symlink_metadata(&path) {
                Ok(_) => {
                    let file = self.ensure_managed_file(&path)?;
                    let (size, sha256) = file_identity(&file)?;
                    if size != row.size || sha256 != row.sha256 {
                        failures.push(ArtifactReconciliationFailure {
                            artifact_id: row.id,
                            code: ArtifactReconciliationFailureCode::HashMismatch,
                        });
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    failures.push(ArtifactReconciliationFailure {
                        artifact_id: row.id,
                        code: ArtifactReconciliationFailureCode::Missing,
                    });
                }
                Err(error) => return Err(error).context("inspect committed artifact"),
            }
        }

        let mut final_files = Vec::new();
        self.collect_final_files(&self.root, &mut final_files)?;
        for file in final_files {
            let relative = file
                .strip_prefix(&self.root)
                .context("artifact file escaped managed root")?
                .to_path_buf();
            if committed.contains_key(&relative) {
                continue;
            }

            let artifact_id = file
                .file_stem()
                .and_then(|stem| stem.to_str())
                .and_then(|stem| Uuid::parse_str(stem).ok())
                .unwrap_or(Uuid::nil());
            let file_name = file
                .file_name()
                .and_then(|name| name.to_str())
                .context("orphan artifact name is not UTF-8")?;
            let destination = self
                .quarantine
                .join(format!("{}-{file_name}", Uuid::new_v4()));
            self.ensure_managed_file(&file)?;
            self.ensure_managed_directory(&self.quarantine)?;
            ensure_path_missing(&destination, "quarantine destination already exists")?;
            fs::rename(&file, &destination).context("quarantine orphan artifact")?;
            failures.push(ArtifactReconciliationFailure {
                artifact_id,
                code: ArtifactReconciliationFailureCode::QuarantinedOrphan,
            });
        }

        failures.sort_by_key(|failure| (failure.artifact_id, failure.code));
        Ok(failures)
    }

    fn validate_prepared(&self, artifact: &PreparedArtifact) -> anyhow::Result<()> {
        ensure!(artifact.size > 0, "prepared artifact has zero size");
        ensure!(
            is_lower_sha256(&artifact.sha256),
            "prepared artifact hash is invalid"
        );
        validate_generated_relative_path(
            &artifact.relative_path,
            artifact.id,
            None,
            &artifact.kind,
        )?;
        let expected_temp = self
            .root
            .join(".staging")
            .join(format!("{}.tmp", artifact.id));
        let expected_final = self.root.join(&artifact.relative_path);
        ensure!(
            artifact.temp_path == expected_temp,
            "prepared staging path mismatch"
        );
        ensure!(
            artifact.final_path == expected_final,
            "prepared final path mismatch"
        );
        Ok(())
    }

    fn remove_stale_staging_files(&self) -> anyhow::Result<()> {
        let staging = self.root.join(".staging");
        self.ensure_managed_directory(&staging)?;
        for entry in fs::read_dir(&staging).context("read artifact staging directory")? {
            let path = entry.context("read staged artifact entry")?.path();
            let metadata = fs::symlink_metadata(&path).context("inspect staged artifact")?;
            ensure!(
                metadata.file_type().is_file(),
                "unexpected staging entry type"
            );
            let managed = self.ensure_managed_file(&path)?;
            fs::remove_file(managed).context("remove stale staged artifact")?;
        }
        Ok(())
    }

    fn ensure_artifact_directory_chain(&self, ids: &[Uuid]) -> anyhow::Result<PathBuf> {
        let mut current = self.root.clone();
        for id in ids {
            let next = current.join(id.to_string());
            match fs::create_dir(&next) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(error).context("create artifact UUID directory"),
            }
            let metadata =
                fs::symlink_metadata(&next).context("inspect artifact UUID directory")?;
            ensure!(
                metadata.file_type().is_dir() && !metadata.file_type().is_symlink(),
                "artifact UUID path is not a real directory"
            );
            current = self.ensure_managed_directory(&next)?;
        }
        Ok(current)
    }

    fn collect_final_files(
        &self,
        directory: &Path,
        output: &mut Vec<PathBuf>,
    ) -> anyhow::Result<()> {
        self.ensure_managed_directory(directory)?;
        for entry in fs::read_dir(directory).context("read artifact directory")? {
            let entry = entry.context("read artifact entry")?;
            let path = entry.path();
            if directory == self.root
                && matches!(entry.file_name().to_str(), Some(".staging" | ".quarantine"))
            {
                continue;
            }
            let file_type = entry.file_type().context("inspect artifact entry type")?;
            ensure!(!file_type.is_symlink(), "artifact symlink is not allowed");
            if file_type.is_dir() {
                self.collect_final_files(&path, output)?;
            } else if file_type.is_file() {
                output.push(self.ensure_managed_file(&path)?);
            } else {
                bail!("unsupported artifact entry type");
            }
        }
        Ok(())
    }

    fn remove_managed_file_if_present(&self, path: &Path) -> anyhow::Result<()> {
        match fs::symlink_metadata(path) {
            Ok(metadata) => {
                ensure!(
                    metadata.file_type().is_file(),
                    "artifact path is not a file"
                );
                let managed = self.ensure_managed_file(path)?;
                fs::remove_file(managed).context("remove managed artifact")?;
                Ok(())
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error).context("inspect managed artifact"),
        }
    }

    fn ensure_managed_file(&self, path: &Path) -> anyhow::Result<PathBuf> {
        let canonical = path
            .canonicalize()
            .with_context(|| format!("canonicalize managed file {}", path.display()))?;
        ensure!(
            canonical.starts_with(&self.root),
            "managed file escaped root"
        );
        ensure!(canonical.is_file(), "managed path is not a file");
        self.ensure_managed_directory(
            canonical
                .parent()
                .context("managed file has no parent directory")?,
        )?;
        Ok(canonical)
    }

    fn ensure_managed_directory(&self, path: &Path) -> anyhow::Result<PathBuf> {
        let canonical = path
            .canonicalize()
            .with_context(|| format!("canonicalize managed directory {}", path.display()))?;
        ensure!(
            canonical.starts_with(&self.root),
            "managed directory escaped root"
        );
        ensure!(canonical.is_dir(), "managed path is not a directory");
        Ok(canonical)
    }
}

fn expected_image_format(format: &str) -> anyhow::Result<(ImageFormat, &'static str)> {
    match format {
        "jpeg" => Ok((ImageFormat::Jpeg, "jpeg")),
        "png" => Ok((ImageFormat::Png, "png")),
        _ => bail!("ArtifactFormat"),
    }
}

fn validate_generated_relative_path(
    relative: &Path,
    artifact_id: Uuid,
    expected_attempt_id: Option<Uuid>,
    kind: &str,
) -> anyhow::Result<()> {
    ensure!(!relative.is_absolute(), "artifact path must be relative");
    let components = relative
        .components()
        .map(|component| match component {
            Component::Normal(value) => value
                .to_str()
                .map(str::to_owned)
                .context("artifact path component is not UTF-8"),
            _ => bail!("artifact path contains a non-normal component"),
        })
        .collect::<anyhow::Result<Vec<_>>>()?;
    ensure!(components.len() == 4, "artifact path shape is invalid");
    let run_id = Uuid::parse_str(&components[0]).context("artifact run path is not a UUID")?;
    ensure!(
        run_id.to_string() == components[0],
        "artifact run path is not canonical"
    );
    let device_run_id =
        Uuid::parse_str(&components[1]).context("artifact device-run path is not a UUID")?;
    ensure!(
        device_run_id.to_string() == components[1],
        "artifact device-run path is not canonical"
    );
    let attempt_id =
        Uuid::parse_str(&components[2]).context("artifact attempt path is not a UUID")?;
    ensure!(
        attempt_id.to_string() == components[2],
        "artifact attempt path is not canonical"
    );
    if let Some(expected) = expected_attempt_id {
        ensure!(attempt_id == expected, "artifact attempt path mismatch");
    }
    let file = Path::new(&components[3]);
    let stem = file
        .file_stem()
        .and_then(|value| value.to_str())
        .context("artifact file stem is invalid")?;
    let extension = file
        .extension()
        .and_then(|value| value.to_str())
        .context("artifact file extension is invalid")?;
    ensure!(
        Uuid::parse_str(stem)? == artifact_id,
        "artifact file ID mismatch"
    );
    let (_, expected_extension) = expected_image_format(kind)?;
    ensure!(
        extension == expected_extension,
        "artifact extension mismatch"
    );
    ensure!(
        components[3] == format!("{artifact_id}.{expected_extension}"),
        "artifact file name is not canonical"
    );
    Ok(())
}

fn ensure_path_missing(path: &Path, message: &'static str) -> anyhow::Result<()> {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Ok(_) => bail!(message),
        Err(error) => Err(error).context("inspect destination path"),
    }
}

fn file_identity(path: &Path) -> anyhow::Result<(u64, String)> {
    let mut file = File::open(path).context("open artifact for hashing")?;
    let size = file.metadata().context("read artifact metadata")?.len();
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .context("read artifact for hashing")?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok((size, format!("{:x}", hasher.finalize())))
}

fn sha256_bytes(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn is_lower_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[cfg(test)]
mod tests {
    use std::fs;

    use chrono::Utc;
    use uuid::Uuid;

    use super::*;
    use crate::FlowArtifactRecord;

    const PNG: &[u8] = include_bytes!("../../tests/fixtures/feed-rail-variant.png");
    const JPEG: &[u8] = include_bytes!("../../tests/fixtures/feed-iphone8.jpg");

    fn temp_artifact_root() -> std::path::PathBuf {
        let root = std::env::temp_dir().join(format!("riviu-flow-artifacts-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).expect("create artifact root");
        root
    }

    fn prepare(
        store: &FlowArtifactStore,
        label: &str,
        format: &str,
        bytes: &[u8],
    ) -> PreparedArtifact {
        store
            .prepare_image(
                Uuid::from_u128(1),
                Uuid::from_u128(2),
                Uuid::from_u128(3),
                label,
                format,
                bytes,
            )
            .expect("prepare image")
    }

    fn record(artifact: &PreparedArtifact, label: &str) -> FlowArtifactRecord {
        FlowArtifactRecord {
            id: artifact.id,
            attempt_id: Uuid::from_u128(3),
            relative_path: artifact.relative_path.to_string_lossy().into_owned(),
            label: label.to_string(),
            kind: artifact.kind.clone(),
            size: artifact.size,
            sha256: artifact.sha256.clone(),
            created_at: Utc::now(),
        }
    }

    #[test]
    fn artifact_labels_never_become_paths() {
        let root = temp_artifact_root();
        let store = FlowArtifactStore::new(&root).expect("store");
        for label in ["../x", "a/b", "CON", "bad\u{0001}name", "shot.exe"] {
            assert!(
                store.validate_label(label, "jpeg").is_err(),
                "accepted {label:?}"
            );
        }

        let prepared = prepare(&store, "screen.png", "png", PNG);
        assert!(!prepared
            .relative_path
            .to_string_lossy()
            .contains("screen.png"));
        assert_eq!(prepared.sha256.len(), 64);
        assert_eq!(prepared.size, PNG.len() as u64);
        assert!(prepared.temp_path.exists());

        store.rollback_file(&prepared).expect("rollback staging");
        fs::remove_dir_all(root).expect("remove artifact root");
    }

    #[test]
    fn image_format_must_match_decoded_bytes() {
        let root = temp_artifact_root();
        let store = FlowArtifactStore::new(&root).expect("store");

        let png = prepare(&store, "screen.png", "png", PNG);
        let jpeg = prepare(&store, "screen.jpeg", "jpeg", JPEG);
        assert!(store
            .prepare_image(
                Uuid::new_v4(),
                Uuid::new_v4(),
                Uuid::new_v4(),
                "wrong.png",
                "png",
                JPEG,
            )
            .is_err());
        assert!(store
            .prepare_image(
                Uuid::new_v4(),
                Uuid::new_v4(),
                Uuid::new_v4(),
                "invalid.jpeg",
                "jpeg",
                b"not an image",
            )
            .is_err());

        store.rollback_file(&png).expect("rollback png");
        store.rollback_file(&jpeg).expect("rollback jpeg");
        fs::remove_dir_all(root).expect("remove artifact root");
    }

    #[test]
    fn publish_is_atomic_and_rollback_cleans_a_rejected_database_write() {
        let root = temp_artifact_root();
        let store = FlowArtifactStore::new(&root).expect("store");
        let prepared = prepare(&store, "screen.png", "png", PNG);

        let relative = store.publish_file(&prepared).expect("publish");
        assert_eq!(relative, prepared.relative_path);
        assert!(!prepared.temp_path.exists());
        assert!(root.join(&relative).exists());

        store
            .rollback_file(&prepared)
            .expect("database rejection rollback");
        assert!(!root.join(relative).exists());
        fs::remove_dir_all(root).expect("remove artifact root");
    }

    #[test]
    fn reconcile_removes_staging_and_quarantines_orphan_finals() {
        let root = temp_artifact_root();
        let store = FlowArtifactStore::new(&root).expect("store");
        fs::write(root.join(".staging").join("stale.tmp"), b"stale").expect("stale file");
        let prepared = prepare(&store, "screen.png", "png", PNG);
        let relative = store.publish_file(&prepared).expect("publish orphan");

        let failures = store.reconcile(&[]).expect("reconcile");

        assert!(!root.join(relative).exists());
        assert!(!root.join(".staging").join("stale.tmp").exists());
        assert_eq!(failures.len(), 1);
        assert_eq!(failures[0].artifact_id, prepared.id);
        assert_eq!(
            failures[0].code,
            ArtifactReconciliationFailureCode::QuarantinedOrphan
        );
        assert_eq!(
            fs::read_dir(root.join(".quarantine"))
                .expect("quarantine")
                .count(),
            1
        );
        fs::remove_dir_all(root).expect("remove artifact root");
    }

    #[test]
    fn reconcile_reports_missing_and_hash_mismatched_committed_files() {
        let root = temp_artifact_root();
        let store = FlowArtifactStore::new(&root).expect("store");

        let missing = prepare(&store, "missing.png", "png", PNG);
        let missing_record = record(&missing, "missing.png");
        store.rollback_file(&missing).expect("remove missing temp");

        let mismatch = prepare(&store, "mismatch.png", "png", PNG);
        let mismatch_record = record(&mismatch, "mismatch.png");
        let mismatch_relative = store.publish_file(&mismatch).expect("publish mismatch");
        fs::write(root.join(mismatch_relative), b"different bytes").expect("corrupt artifact");

        let failures = store
            .reconcile(&[missing_record, mismatch_record])
            .expect("reconcile committed rows");

        assert!(failures.iter().any(|failure| {
            failure.artifact_id == missing.id
                && failure.code == ArtifactReconciliationFailureCode::Missing
        }));
        assert!(failures.iter().any(|failure| {
            failure.artifact_id == mismatch.id
                && failure.code == ArtifactReconciliationFailureCode::HashMismatch
        }));
        fs::remove_dir_all(root).expect("remove artifact root");
    }

    #[test]
    fn reconcile_keeps_an_exact_committed_artifact() {
        let root = temp_artifact_root();
        let store = FlowArtifactStore::new(&root).expect("store");
        let prepared = prepare(&store, "screen.png", "png", PNG);
        let row = record(&prepared, "screen.png");
        let relative = store.publish_file(&prepared).expect("publish committed");

        assert!(store.reconcile(&[row]).expect("reconcile").is_empty());
        assert!(root.join(relative).is_file());

        store
            .rollback_file(&prepared)
            .expect("cleanup committed file");
        fs::remove_dir_all(root).expect("remove artifact root");
    }

    #[test]
    fn reconcile_rejects_database_paths_that_escape_the_managed_root() {
        let root = temp_artifact_root();
        let store = FlowArtifactStore::new(&root).expect("store");
        let artifact = prepare(&store, "screen.png", "png", PNG);
        let mut row = record(&artifact, "screen.png");
        row.relative_path = "../outside.png".to_string();

        assert!(store.reconcile(&[row]).is_err());

        store.rollback_file(&artifact).expect("rollback staging");
        fs::remove_dir_all(root).expect("remove artifact root");
    }
}
