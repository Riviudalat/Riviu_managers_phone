//! Owned diagnostic tasks and fsynced receipts; never a campaign dispatcher.
use anyhow::{ensure, Context};
use parking_lot::Mutex;
use serde::Serialize;
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};
use tokio::time::Instant;

#[derive(Clone)]
pub(crate) struct RunContext {
    pub request_id: String,
    pub udid: String,
    pub deadline: Instant,
    pub stop: Arc<AtomicBool>,
    pub report_dir: PathBuf,
    record: Arc<Mutex<RunRecord>>,
    fence_path: PathBuf,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct RunRecord {
    request_id: String,
    activation_id: String,
    udid: String,
    kind: String,
    input_digest: String,
    phase: String,
    state: String,
    outcome: Option<Value>,
    public_effects_allowed: bool,
}
#[derive(Default)]
pub(crate) struct Registry {
    runs: Mutex<HashMap<String, RunContext>>,
    tasks: Mutex<Vec<tokio::task::JoinHandle<()>>>,
    closing: AtomicBool,
}
fn atomic_record(path: &std::path::Path, record: &impl Serialize) -> anyhow::Result<()> {
    let temp = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp)?;
    use std::io::Write;
    file.write_all(&serde_json::to_vec_pretty(record)?)?;
    file.sync_all()?;
    std::fs::rename(&temp, path).or_else(|error| {
        temp.exists().then(|| std::fs::remove_file(&temp));
        Err(error)
    })?;
    Ok(())
}
impl RunContext {
    pub fn check(&self) -> anyhow::Result<()> {
        ensure!(!self.stop.load(Ordering::Relaxed), "diagnostic cancelled");
        ensure!(
            Instant::now() < self.deadline,
            "diagnostic deadline exhausted"
        );
        Ok(())
    }
    pub fn remaining_ms(&self) -> u64 {
        self.deadline
            .saturating_duration_since(Instant::now())
            .as_millis()
            .min(u64::MAX as u128) as u64
    }
    fn update(&self, change: impl FnOnce(&mut RunRecord)) -> anyhow::Result<()> {
        let mut record = self.record.lock();
        if record.state != "running" {
            return Ok(());
        }
        let mut candidate = record.clone();
        change(&mut candidate);
        if let Err(error) = atomic_record(&self.report_dir.join("status.json"), &candidate) {
            record.state = "needsAttention".into();
            self.stop.store(true, Ordering::Release);
            return Err(error);
        }
        *record = candidate;
        // Keep the durable fence on every ambiguous or failed settlement. A clean
        // run can permit another request only after both status and fence settle.
        if matches!(record.state.as_str(), "preparedAndCleaned" | "blocked") {
            if let Err(error) = std::fs::remove_file(&self.fence_path) {
                record.state = "needsAttention".into();
                self.stop.store(true, Ordering::Release);
                return Err(error.into());
            }
        }
        Ok(())
    }
    pub fn record_phase(&self, phase: &str, detail: &Value) -> anyhow::Result<()> {
        self.update(|record| {
            record.phase = phase.into();
            record.outcome = Some(detail.clone());
        })
    }
    pub fn finish(&self, state: &str, outcome: Value) -> anyhow::Result<()> {
        if self.record.lock().state != "running" { return Ok(()); }
        self.update(|record| {
            record.state = state.into();
            record.outcome = Some(outcome);
        })
    }
    pub fn status(&self) -> Value {
        serde_json::to_value(&*self.record.lock()).expect("diagnostic DTO")
    }
}
impl Registry {
    pub fn begin(
        &self,
        scope: &crate::no_public::Session,
        udid: &str,
        id: &str,
        kind: &str,
        digest: &str,
        budget: Duration,
    ) -> anyhow::Result<(RunContext, bool)> {
        ensure!(
            !self.closing.load(Ordering::Acquire),
            "diagnostic shutdown in progress"
        );
        uuid::Uuid::parse_str(id)?;
        scope.device(udid)?;
        let mut runs = self.runs.lock();
        ensure!(
            !self.closing.load(Ordering::Acquire),
            "diagnostic shutdown in progress"
        );
        if let Some(run) = runs.get(id) {
            let existing = run.record.lock();
            ensure!(
                existing.udid == udid && existing.kind == kind && existing.input_digest == digest,
                "same request ID has different scope/input"
            );
            drop(existing);
            return Ok((run.clone(), false));
        }
        ensure!(
            !runs.values().any(|run| run.udid == udid
                && matches!(
                    run.record.lock().state.as_str(),
                    "running" | "needsAttention"
                )),
            "device has an active or unresolved diagnostic"
        );
        let fence_path = scope.fence_root.join(format!(
            "{}.json",
            riviu_core::frame_sha256(udid.as_bytes())
        ));
        // Stable cross-activation fence: partial/corrupt/existing claims all refuse.
        let mut fence = std::fs::OpenOptions::new().write(true).create_new(true).open(&fence_path)
            .context("device has an unresolved prior diagnostic fence; do not retry under a new activation")?;
        use std::io::Write;
        fence.write_all(&serde_json::to_vec(&json!({"requestId":id,"activationId":scope.activation,"udid":udid,"inputDigest":digest,"state":"claimed"}))?)?;
        fence.sync_all()?;
        let directory = scope.root.join(format!("run-{id}"));
        std::fs::create_dir(&directory).context("existing run directory refuses replay")?;
        let record = RunRecord {
            request_id: id.into(),
            activation_id: scope.activation.clone(),
            udid: udid.into(),
            kind: kind.into(),
            input_digest: digest.into(),
            phase: "claimed".into(),
            state: "running".into(),
            outcome: None,
            public_effects_allowed: false,
        };
        // Create claim before any task or lease; a partial claim is never auto replayed.
        let path = directory.join("intent.json");
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)?;
        file.write_all(&serde_json::to_vec(&record)?)?;
        file.sync_all()?;
        let run = RunContext {
            request_id: id.into(),
            udid: udid.into(),
            deadline: Instant::now() + budget,
            stop: Arc::new(AtomicBool::new(false)),
            report_dir: directory,
            record: Arc::new(Mutex::new(record)),
            fence_path,
        };
        run.record_phase("claimed", &json!({"noPublicIntent":true}))?;
        runs.insert(id.into(), run.clone());
        Ok((run, true))
    }
    pub fn spawn(
        &self,
        run: &RunContext,
        work: impl std::future::Future<Output = ()> + Send + 'static,
    ) -> anyhow::Result<()> {
        let _runs = self.runs.lock();
        ensure!(
            !self.closing.load(Ordering::Acquire),
            "diagnostic shutdown in progress"
        );
        let owned = run.clone();
        let task = tokio::spawn(async move {
            use futures_util::FutureExt;
            if std::panic::AssertUnwindSafe(work)
                .catch_unwind()
                .await
                .is_err()
            {
                let _ = owned.finish(
                    "needsAttention",
                    json!({"error":"diagnostic task panicked"}),
                );
                owned.stop.store(true, Ordering::Release);
            }
        });
        self.tasks.lock().push(task);
        Ok(())
    }
    pub fn status(&self, id: &str) -> Option<Value> {
        self.runs.lock().get(id).map(RunContext::status)
    }
    pub fn cancel(&self, id: &str) -> anyhow::Result<Value> {
        let runs = self.runs.lock();
        let run = runs.get(id).context("diagnostic not found")?;
        if run.record.lock().state == "running" {
            run.stop.store(true, Ordering::Release);
        }
        Ok(run.status())
    }
    pub fn cancel_all(&self) {
        let runs = self.runs.lock();
        self.closing.store(true, Ordering::Release);
        for run in runs.values() {
            run.stop.store(true, Ordering::Release);
        }
    }
    pub async fn drain(&self) -> anyhow::Result<()> {
        let tasks = std::mem::take(&mut *self.tasks.lock());
        let mut failures = Vec::new();
        for task in tasks {
            if let Err(error) = task.await {
                failures.push(error.to_string());
            }
        }
        ensure!(failures.is_empty(), "diagnostic task drain failed: {}", failures.join("; "));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn duplicate_request_is_status_only_and_changed_scope_refuses() {
        let root = std::env::temp_dir().join(format!("riviu-runs-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        let scope = crate::no_public::Session {
            root: root.clone(),
            activation: uuid::Uuid::new_v4().to_string(),
            fence_root: root.clone(),
            devices: vec![crate::no_public::DeviceScope {
                udid: "fixture".into(),
                package: "com.zhiliaoapp.musically".into(),
                expected_account: "fixture".into(),
                target_url: None,
                draft_text: None,
                source_root: None,
                bundle_id: None,
                publish_fingerprint: None,
                sound_policy: None,
                helper_canary: false,
                allow_warm_launch: false,
                allow_installed_runner: false,
            }]
            .into(),
        };
        let registry = Registry::default();
        let id = uuid::Uuid::new_v4().to_string();
        let (first, inserted) = registry
            .begin(
                &scope,
                "fixture",
                &id,
                "interaction",
                "digest",
                Duration::from_secs(1),
            )
            .unwrap();
        assert!(inserted);
        assert!(
            !registry
                .begin(
                    &scope,
                    "fixture",
                    &id,
                    "interaction",
                    "digest",
                    Duration::from_secs(1)
                )
                .unwrap()
                .1
        );
        assert!(registry
            .begin(
                &scope,
                "fixture",
                &id,
                "interaction",
                "changed",
                Duration::from_secs(1)
            )
            .is_err());
        registry.cancel(&id).unwrap();
        assert!(first.check().is_err());
        first
            .finish("needsAttention", json!({"cleanup":false}))
            .unwrap();
        assert!(registry
            .begin(
                &scope,
                "fixture",
                &uuid::Uuid::new_v4().to_string(),
                "interaction",
                "digest",
                Duration::from_secs(1)
            )
            .is_err());
        // A new process/activation must still refuse this device's durable fence.
        assert!(Registry::default().begin(&scope, "fixture", &uuid::Uuid::new_v4().to_string(), "inspect", "digest", Duration::from_secs(1)).is_err());
        let second_root = root.join("second"); std::fs::create_dir(&second_root).unwrap();
        let mut clean_scope = scope.clone(); clean_scope.root = second_root.clone(); clean_scope.fence_root = second_root;
        let clean_registry = Registry::default();
        let (clean, _) = clean_registry.begin(&clean_scope, "fixture", &uuid::Uuid::new_v4().to_string(), "inspect", "digest", Duration::from_secs(1)).unwrap();
        let status_path = clean.report_dir.join("status.json"); std::fs::remove_file(&status_path).unwrap(); std::fs::create_dir(&status_path).unwrap();
        assert!(clean.finish("preparedAndCleaned", json!({})).is_err());
        assert_eq!(clean.status()["state"], "needsAttention"); assert!(clean.fence_path.exists());
        clean.record_phase("lateUnsafeUpdate", &json!({})).unwrap();
        assert_eq!(clean.status()["state"], "needsAttention");
        assert!(clean.fence_path.exists());
        clean_registry.cancel_all(); assert!(clean_registry.begin(&clean_scope, "fixture", &uuid::Uuid::new_v4().to_string(), "inspect", "digest", Duration::from_secs(1)).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }
}
