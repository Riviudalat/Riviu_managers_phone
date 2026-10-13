//! Retention for `artifacts/traces`: the per-step screenshots and hierarchies that
//! [`super::trace::TraceRecorder`] writes have no other owner and no other reaper.
//!
//! Measured 14/10/2026 on the operator host: `artifacts` held 76.7 GB, of which `traces`
//! was 53.2 GB, with 124 GB left on `C:`. Nothing had ever deleted a trace. The layout is
//! `traces/<run>/<device>/<attempt>/<id>.{json,xml,png}` where `<run>` is
//! [`super::trace::run_folder_id`] of the run id (the campaign id for publish), so a run
//! folder can be protected by name from the database without opening a single file.
//!
//! Two caps, both unmeasured defaults the operator doc records as such: a run older than
//! `max_age` goes, and when what remains is still over `max_total_bytes` the oldest
//! unprotected runs go until it is not. Protected runs never go, whatever the size. A run
//! newer than `size_cap_grace` is also never removed for size, because that is the
//! window in which a recorder can still be appending to it.
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TraceRetentionPolicy {
    /// Runs whose newest file is older than this are removed unless protected.
    pub max_age: Duration,
    /// After the age pass, the oldest unprotected runs are removed until the root is
    /// at or under this many bytes.
    pub max_total_bytes: u64,
    /// Runs newer than this are never removed by the size pass.
    pub size_cap_grace: Duration,
}

impl TraceRetentionPolicy {
    /// Seven days and 20 GiB: proposed 14/10/2026 against a 53 GB backlog, not measured
    /// against any diagnostic need. One night of the 30-phone fleet produced about 8 GB.
    pub const DEFAULT: Self = Self {
        max_age: Duration::from_secs(7 * 24 * 60 * 60),
        max_total_bytes: 20 * 1024 * 1024 * 1024,
        size_cap_grace: Duration::from_secs(24 * 60 * 60),
    };
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct TraceRetentionReport {
    pub scanned_runs: usize,
    pub protected_runs: usize,
    pub removed_runs: usize,
    pub removed_bytes: u64,
    pub kept_bytes: u64,
    /// Run folders that could not be measured or removed, with the error text.
    pub failures: Vec<(PathBuf, String)>,
}

struct RunFolder {
    path: PathBuf,
    id: Uuid,
    bytes: u64,
    newest: SystemTime,
}

/// Apply `policy` under `root`, never touching `protected` runs, the `.staging` and
/// `.quarantine` directories or anything whose name is not a run folder id.
pub fn prune_trace_runs(
    root: &Path,
    protected: &HashSet<Uuid>,
    policy: TraceRetentionPolicy,
    now: SystemTime,
) -> anyhow::Result<TraceRetentionReport> {
    let mut report = TraceRetentionReport::default();
    if !root.is_dir() {
        return Ok(report);
    }
    let mut runs = Vec::new();
    for entry in std::fs::read_dir(root)? {
        let entry = entry?;
        let path = entry.path();
        let Some(id) = entry
            .file_name()
            .to_str()
            .and_then(|name| Uuid::parse_str(name).ok())
        else {
            continue;
        };
        if !entry.file_type()?.is_dir() {
            continue;
        }
        report.scanned_runs += 1;
        match measure(&path) {
            Ok((bytes, newest)) => runs.push(RunFolder {
                path,
                id,
                bytes,
                newest,
            }),
            Err(error) => report.failures.push((path, format!("{error:#}"))),
        }
    }
    // Oldest first, so the size pass below removes in age order.
    runs.sort_by_key(|run| run.newest);

    let mut kept: Vec<RunFolder> = Vec::new();
    for run in runs {
        if protected.contains(&run.id) {
            report.protected_runs += 1;
            report.kept_bytes += run.bytes;
            continue;
        }
        let age = now.duration_since(run.newest).unwrap_or_default();
        if age > policy.max_age {
            remove(run, &mut report);
        } else {
            kept.push(run);
        }
    }
    let mut total: u64 = report.kept_bytes + kept.iter().map(|run| run.bytes).sum::<u64>();
    let mut index = 0;
    while total > policy.max_total_bytes && index < kept.len() {
        let age = now.duration_since(kept[index].newest).unwrap_or_default();
        if age < policy.size_cap_grace {
            // Sorted oldest first: nothing after this one is older.
            break;
        }
        let run = kept.remove(index);
        total = total.saturating_sub(run.bytes);
        remove(run, &mut report);
        // `remove` took the element, so `index` already points at the next one.
        index = index.min(kept.len());
    }
    report.kept_bytes += kept.iter().map(|run| run.bytes).sum::<u64>();
    Ok(report)
}

fn remove(run: RunFolder, report: &mut TraceRetentionReport) {
    match std::fs::remove_dir_all(&run.path) {
        Ok(()) => {
            report.removed_runs += 1;
            report.removed_bytes += run.bytes;
        }
        Err(error) => {
            report.kept_bytes += run.bytes;
            report.failures.push((run.path, error.to_string()));
        }
    }
}

/// Total bytes and newest file modification time under one run folder. Directory
/// mtimes are ignored: NTFS bumps them on every child change, so a folder created by
/// the recorder would look new forever. A run with no files at all dates from its own
/// folder. Symlinks are not followed: the recorder never creates one, and following
/// one could leave the root.
fn measure(path: &Path) -> anyhow::Result<(u64, SystemTime)> {
    let mut bytes = 0;
    let mut newest: Option<SystemTime> = None;
    let mut pending = vec![path.to_path_buf()];
    while let Some(directory) = pending.pop() {
        for entry in std::fs::read_dir(&directory)? {
            let entry = entry?;
            let metadata = entry.metadata()?;
            if metadata.file_type().is_symlink() {
                continue;
            }
            if metadata.is_dir() {
                pending.push(entry.path());
                continue;
            }
            bytes += metadata.len();
            if let Ok(modified) = metadata.modified() {
                newest = Some(newest.map_or(modified, |seen| seen.max(modified)));
            }
        }
    }
    let newest = match newest {
        Some(newest) => newest,
        None => std::fs::symlink_metadata(path)?
            .modified()
            .unwrap_or(SystemTime::UNIX_EPOCH),
    };
    Ok((bytes, newest))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy(max_age_secs: u64, max_bytes: u64) -> TraceRetentionPolicy {
        TraceRetentionPolicy {
            max_age: Duration::from_secs(max_age_secs),
            max_total_bytes: max_bytes,
            size_cap_grace: Duration::from_secs(60),
        }
    }

    fn run(root: &Path, id: Uuid, bytes: usize, age: Duration, now: SystemTime) -> PathBuf {
        let folder = root
            .join(id.to_string())
            .join(Uuid::new_v4().to_string())
            .join(Uuid::new_v4().to_string());
        std::fs::create_dir_all(&folder).unwrap();
        let file = folder.join("observation.json");
        std::fs::write(&file, vec![b'x'; bytes]).unwrap();
        std::fs::File::options()
            .write(true)
            .open(&file)
            .unwrap()
            .set_modified(now - age)
            .unwrap();
        root.join(id.to_string())
    }

    fn fresh_root() -> PathBuf {
        let root = std::env::temp_dir().join(format!("trace-retention-{}", Uuid::new_v4()));
        std::fs::create_dir_all(root.join(".staging")).unwrap();
        std::fs::write(root.join(".staging").join("x.tmp"), b"tmp").unwrap();
        std::fs::create_dir_all(root.join("not-a-run")).unwrap();
        std::fs::write(root.join("not-a-run").join("keep.txt"), b"keep").unwrap();
        root
    }

    #[test]
    fn old_runs_go_and_protected_or_young_runs_stay() {
        let root = fresh_root();
        let now = SystemTime::now();
        let day = Duration::from_secs(24 * 60 * 60);
        let old = run(&root, Uuid::new_v4(), 10, 9 * day, now);
        let protected_id = Uuid::new_v4();
        let protected = run(&root, protected_id, 10, 30 * day, now);
        let young = run(&root, Uuid::new_v4(), 10, day, now);
        let report = prune_trace_runs(
            &root,
            &HashSet::from([protected_id]),
            policy(7 * 24 * 60 * 60, u64::MAX),
            now,
        )
        .unwrap();
        assert!(!old.exists(), "older than max_age must be removed");
        assert!(protected.exists(), "protected runs never go");
        assert!(young.exists());
        assert!(root.join(".staging").join("x.tmp").exists());
        assert!(root.join("not-a-run").join("keep.txt").exists());
        assert_eq!(
            (
                report.scanned_runs,
                report.protected_runs,
                report.removed_runs,
                report.removed_bytes
            ),
            (3, 1, 1, 10)
        );
        assert_eq!(report.kept_bytes, 20);
        assert!(report.failures.is_empty());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn size_cap_removes_oldest_unprotected_first_and_spares_runs_in_grace() {
        let root = fresh_root();
        let now = SystemTime::now();
        let hour = Duration::from_secs(60 * 60);
        let oldest = run(&root, Uuid::new_v4(), 100, 5 * hour, now);
        let protected_id = Uuid::new_v4();
        let protected = run(&root, protected_id, 100, 4 * hour, now);
        let middle = run(&root, Uuid::new_v4(), 100, 3 * hour, now);
        let newest = run(&root, Uuid::new_v4(), 100, Duration::from_secs(10), now);
        // 400 bytes on disk, cap 250: the two oldest unprotected runs must go, the
        // protected one stays even though it is older than `middle`, and the newest is
        // inside the grace window even if the cap were still exceeded.
        let report = prune_trace_runs(
            &root,
            &HashSet::from([protected_id]),
            policy(u64::MAX / 4, 250),
            now,
        )
        .unwrap();
        assert!(!oldest.exists());
        assert!(!middle.exists());
        assert!(protected.exists());
        assert!(newest.exists());
        assert_eq!(report.removed_runs, 2);
        assert_eq!(report.kept_bytes, 200);

        // Cap still exceeded by protected + in-grace runs: nothing more may go.
        let report = prune_trace_runs(
            &root,
            &HashSet::from([protected_id]),
            policy(u64::MAX / 4, 50),
            now,
        )
        .unwrap();
        assert_eq!(report.removed_runs, 0);
        assert!(newest.exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_missing_root_is_an_empty_report() {
        let root = std::env::temp_dir().join(format!("trace-retention-missing-{}", Uuid::new_v4()));
        let report = prune_trace_runs(
            &root,
            &HashSet::new(),
            TraceRetentionPolicy::DEFAULT,
            SystemTime::now(),
        )
        .unwrap();
        assert_eq!(report, TraceRetentionReport::default());
    }
}
