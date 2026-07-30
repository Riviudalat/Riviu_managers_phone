use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;

use chrono::Utc;
use parking_lot::Mutex;
use tokio::sync::Notify;
use tokio::task::JoinHandle;
use uuid::Uuid;

use crate::db::Database;
use crate::device_control::{DeviceControlPlane, UiSessionContext};
use crate::events::{AppEvent, EventBus};
use crate::registry::DeviceRegistry;
use crate::types::{
    AutomationScript, DeviceStatus, JobRecord, JobStatus, JobStepRecord, ScriptAction, StepStatus,
};
use crate::DeviceWorkOwner;

#[derive(Clone)]
pub struct JobQueue {
    db: Arc<Database>,
    events: EventBus,
    registry: DeviceRegistry,
    control: Arc<DeviceControlPlane>,
    artifacts_dir: PathBuf,
    cancelled: Arc<Mutex<HashSet<Uuid>>>,
    cancel_changed: Arc<Notify>,
    runtime: Arc<Mutex<JobQueueRuntime>>,
    shutdown_gate: Arc<tokio::sync::Mutex<()>>,
}

#[derive(Default)]
struct JobQueueRuntime {
    stopping: bool,
    tasks: HashMap<Uuid, JoinHandle<()>>,
}

impl JobQueue {
    pub fn new(
        db: Arc<Database>,
        events: EventBus,
        registry: DeviceRegistry,
        control: Arc<DeviceControlPlane>,
        artifacts_dir: PathBuf,
    ) -> Self {
        std::fs::create_dir_all(&artifacts_dir).ok();
        Self {
            db,
            events,
            registry,
            control,
            artifacts_dir,
            cancelled: Arc::new(Mutex::new(HashSet::new())),
            cancel_changed: Arc::new(Notify::new()),
            runtime: Arc::new(Mutex::new(JobQueueRuntime::default())),
            shutdown_gate: Arc::new(tokio::sync::Mutex::new(())),
        }
    }

    pub fn list_jobs(&self, limit: usize) -> anyhow::Result<Vec<JobRecord>> {
        self.db.list_jobs(limit)
    }

    pub fn cancel(&self, job_id: Uuid) {
        self.cancelled.lock().insert(job_id);
        self.cancel_changed.notify_waiters();
    }

    pub fn stop_all(&self) {
        let job_ids = {
            let mut runtime = self.runtime.lock();
            runtime.stopping = true;
            runtime.tasks.keys().copied().collect::<Vec<_>>()
        };
        self.cancelled.lock().extend(job_ids);
        self.cancel_changed.notify_waiters();
    }

    pub async fn shutdown(&self) -> anyhow::Result<()> {
        let _shutdown_guard = self.shutdown_gate.lock().await;
        self.stop_all();
        let tasks = {
            let mut runtime = self.runtime.lock();
            std::mem::take(&mut runtime.tasks)
        };
        let mut first_error = None;
        for (job_id, task) in tasks {
            if let Err(error) = task.await {
                first_error.get_or_insert_with(|| anyhow::anyhow!("job {job_id}: {error}"));
            }
        }
        if let Some(error) = first_error {
            return Err(error);
        }
        Ok(())
    }

    pub async fn enqueue(
        &self,
        script: AutomationScript,
        udids: Vec<String>,
    ) -> anyhow::Result<JobRecord> {
        let mut runtime = self.runtime.lock();
        if runtime.stopping {
            anyhow::bail!("job queue is shutting down");
        }
        runtime.tasks.retain(|_, task| !task.is_finished());
        let now = Utc::now();
        let steps: Vec<JobStepRecord> = script
            .steps
            .iter()
            .enumerate()
            .map(|(index, action)| JobStepRecord {
                index,
                action: action_name(action).to_string(),
                status: StepStatus::Pending,
                error: None,
                artifact_path: None,
            })
            .collect();

        let job = JobRecord {
            id: Uuid::new_v4(),
            script_name: script.name.clone(),
            udids: udids.clone(),
            status: JobStatus::Queued,
            created_at: now,
            updated_at: now,
            steps,
            error: None,
        };
        self.db.save_job(&job)?;
        self.events.emit(AppEvent::JobUpdated { job: job.clone() });

        let this = self.clone();
        let script_clone = script.clone();
        let job_id = job.id;
        let task = tokio::spawn(async move {
            if let Err(err) = this.run_job(job_id, script_clone, udids).await {
                tracing::error!("job {job_id} failed: {err:#}");
            }
        });
        runtime.tasks.insert(job_id, task);
        drop(runtime);

        Ok(job)
    }

    async fn run_job(
        &self,
        job_id: Uuid,
        script: AutomationScript,
        udids: Vec<String>,
    ) -> anyhow::Result<()> {
        let mut job = self
            .db
            .list_jobs(200)?
            .into_iter()
            .find(|j| j.id == job_id)
            .ok_or_else(|| anyhow::anyhow!("job missing"))?;

        job.status = JobStatus::Running;
        job.updated_at = Utc::now();
        self.persist(&job);

        let mut first_error: Option<String> = None;

        for udid in &udids {
            if self.is_cancelled(job_id) {
                job.status = JobStatus::Cancelled;
                job.updated_at = Utc::now();
                self.persist(&job);
                return Ok(());
            }

            let acquire = self
                .control
                .acquire_exclusive(udid, DeviceWorkOwner::Script);
            let cancelled = self.wait_until_cancelled(job_id);
            tokio::pin!(acquire);
            tokio::pin!(cancelled);
            let context = tokio::select! {
                biased;
                _ = &mut cancelled => {
                    job.status = JobStatus::Cancelled;
                    job.updated_at = Utc::now();
                    self.persist(&job);
                    return Ok(());
                }
                result = &mut acquire => result?,
            };
            self.registry.set_status(udid, DeviceStatus::Busy, None);

            match self.run_on_device(&mut job, &script, udid, context).await {
                Ok(()) => {
                    self.registry.set_status(udid, DeviceStatus::Ready, None);
                }
                Err(err) => {
                    if self.is_cancelled(job_id) {
                        self.registry.set_status(udid, DeviceStatus::Ready, None);
                        continue;
                    }
                    let msg = format!("{err:#}");
                    first_error.get_or_insert(msg.clone());
                    self.registry
                        .set_status(udid, DeviceStatus::Error, Some(msg));
                }
            }
        }

        job.status = if self.is_cancelled(job_id) {
            JobStatus::Cancelled
        } else if first_error.is_some() {
            JobStatus::Failed
        } else {
            JobStatus::Succeeded
        };
        job.error = first_error;
        job.updated_at = Utc::now();
        self.persist(&job);
        Ok(())
    }

    async fn run_on_device(
        &self,
        job: &mut JobRecord,
        script: &AutomationScript,
        udid: &str,
        context: crate::DeviceExclusiveContext,
    ) -> anyhow::Result<()> {
        self.control.repair_agent_install_only(&context).await?;
        let context = self.control.start_owned_ui_session(context).await?;

        let result: anyhow::Result<()> = async {
            let session = self.control.session(&context)?;
            let run_dir = self.artifacts_dir.join(job.id.to_string()).join(udid);
            std::fs::create_dir_all(&run_dir)?;
            for (index, action) in script.steps.iter().enumerate() {
                if self.is_cancelled(job.id) {
                    anyhow::bail!("cancelled");
                }
                if let Some(step) = job.steps.get_mut(index) {
                    step.status = StepStatus::Running;
                }
                job.updated_at = Utc::now();
                self.persist(job);

                let result = self
                    .execute_step(job.id, action, &context, session.as_ref(), &run_dir)
                    .await;

                match result {
                    Ok(artifact) => {
                        if let Some(step) = job.steps.get_mut(index) {
                            step.status = StepStatus::Succeeded;
                            step.artifact_path = artifact;
                            step.error = None;
                        }
                    }
                    Err(err) => {
                        if let Some(step) = job.steps.get_mut(index) {
                            if self.is_cancelled(job.id) {
                                step.status = StepStatus::Skipped;
                                step.error = None;
                            } else {
                                step.status = StepStatus::Failed;
                                step.error = Some(format!("{err:#}"));
                            }
                        }
                        job.updated_at = Utc::now();
                        self.persist(job);
                        return Err(err);
                    }
                }
                job.updated_at = Utc::now();
                self.persist(job);
            }
            Ok(())
        }
        .await;
        self.control.close_session_context(context)?;
        result
    }

    async fn execute_step(
        &self,
        job_id: Uuid,
        action: &ScriptAction,
        context: &UiSessionContext,
        session: &dyn crate::driver::UiSession,
        run_dir: &std::path::Path,
    ) -> anyhow::Result<Option<String>> {
        match action {
            ScriptAction::LaunchApp { bundle_id } => {
                self.control
                    .foreground_session_app(context, bundle_id)
                    .await?;
                Ok(None)
            }
            ScriptAction::TerminateApp { bundle_id } => {
                self.control
                    .terminate_session_app(context, bundle_id)
                    .await?;
                Ok(None)
            }
            ScriptAction::Wait { milliseconds } => {
                self.wait_or_cancel(job_id, *milliseconds).await?;
                Ok(None)
            }
            ScriptAction::Tap { selector, point } => {
                if let Some(p) = point {
                    session.tap(p.clone()).await?;
                } else if let Some(sel) = selector {
                    let id = sel
                        .accessibility_id
                        .as_deref()
                        .ok_or_else(|| anyhow::anyhow!("tap requires accessibilityId or point"))?;
                    session.find_and_tap(id).await?;
                } else {
                    anyhow::bail!("tap requires selector or point");
                }
                Ok(None)
            }
            ScriptAction::Swipe { gesture } => {
                session.swipe(gesture.clone()).await?;
                Ok(None)
            }
            ScriptAction::TypeText { value } => {
                session.type_text(value).await?;
                Ok(None)
            }
            ScriptAction::Screenshot { name } => {
                let dest = run_dir.join(format!("{name}.png"));
                let path = self.control.session_screenshot(context, &dest).await?;
                Ok(Some(path.display().to_string()))
            }
            ScriptAction::Home => {
                session.home().await?;
                Ok(None)
            }
            ScriptAction::AssertVisible { selector } => {
                let id = selector
                    .accessibility_id
                    .as_deref()
                    .ok_or_else(|| anyhow::anyhow!("assertVisible needs accessibilityId"))?;
                session.assert_visible(id).await?;
                Ok(None)
            }
        }
    }

    fn persist(&self, job: &JobRecord) {
        if let Err(err) = self.db.save_job(job) {
            tracing::error!("persist job: {err:#}");
        }
        self.events.emit(AppEvent::JobUpdated { job: job.clone() });
    }

    fn is_cancelled(&self, job_id: Uuid) -> bool {
        self.cancelled.lock().contains(&job_id)
    }

    async fn wait_until_cancelled(&self, job_id: Uuid) {
        loop {
            let changed = self.cancel_changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            if self.is_cancelled(job_id) {
                return;
            }
            changed.await;
        }
    }

    async fn wait_or_cancel(&self, job_id: Uuid, mut milliseconds: u64) -> anyhow::Result<()> {
        const MAX_WAIT_SLICE_MS: u64 = 1_000;
        while milliseconds > 0 {
            if self.is_cancelled(job_id) {
                anyhow::bail!("cancelled");
            }
            let slice = milliseconds.min(MAX_WAIT_SLICE_MS);
            tokio::select! {
                biased;
                _ = self.wait_until_cancelled(job_id) => anyhow::bail!("cancelled"),
                _ = tokio::time::sleep(std::time::Duration::from_millis(slice)) => {}
            }
            milliseconds -= slice;
        }
        Ok(())
    }
}

fn action_name(action: &ScriptAction) -> &'static str {
    match action {
        ScriptAction::LaunchApp { .. } => "launchApp",
        ScriptAction::TerminateApp { .. } => "terminateApp",
        ScriptAction::Wait { .. } => "wait",
        ScriptAction::Tap { .. } => "tap",
        ScriptAction::Swipe { .. } => "swipe",
        ScriptAction::TypeText { .. } => "typeText",
        ScriptAction::Screenshot { .. } => "screenshot",
        ScriptAction::Home => "home",
        ScriptAction::AssertVisible { .. } => "assertVisible",
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::Arc;

    use async_trait::async_trait;
    use tokio::time::{timeout, Duration};

    use super::*;
    use crate::{
        AgentInstallProof, DeviceDriver, DeviceInfo, DeviceWorkCoordinator, InstalledAgentIdentity,
        StreamBudgetManager, SwipeGesture, TapPoint, UiSession,
    };

    struct QueueTestSession;

    #[async_trait]
    impl UiSession for QueueTestSession {
        async fn tap(&self, _point: TapPoint) -> anyhow::Result<()> {
            Ok(())
        }

        async fn swipe(&self, _gesture: SwipeGesture) -> anyhow::Result<()> {
            Ok(())
        }

        async fn type_text(&self, _text: &str) -> anyhow::Result<()> {
            Ok(())
        }

        async fn home(&self) -> anyhow::Result<()> {
            Ok(())
        }

        async fn find_and_tap(&self, _accessibility_id: &str) -> anyhow::Result<()> {
            Ok(())
        }

        async fn assert_visible(&self, _accessibility_id: &str) -> anyhow::Result<()> {
            Ok(())
        }

        fn stream_url(&self) -> Option<String> {
            None
        }
    }

    #[derive(Default)]
    struct QueueTestDriver {
        require_install_only: AtomicBool,
        install_only_calls: AtomicUsize,
        session_calls: AtomicUsize,
        stream_calls: AtomicUsize,
        session_live: AtomicBool,
    }

    impl QueueTestDriver {
        fn requiring_install_only() -> Self {
            Self {
                require_install_only: AtomicBool::new(true),
                ..Self::default()
            }
        }
    }

    #[async_trait]
    impl DeviceDriver for QueueTestDriver {
        async fn repair_agent_install_only(
            &self,
            _udid: &str,
        ) -> anyhow::Result<AgentInstallProof> {
            if self.session_live.load(Ordering::Acquire) {
                anyhow::bail!("install-only readiness requires the prior session to be closed");
            }
            self.install_only_calls.fetch_add(1, Ordering::Relaxed);
            Ok(AgentInstallProof {
                installed: InstalledAgentIdentity {
                    bundle_id: "com.fixture.agent".to_string(),
                    version: "1.0".to_string(),
                    build: "1".to_string(),
                    executable_name: "FixtureRunner".to_string(),
                    signer_identity_sha256: "a".repeat(64),
                },
                artifact_sha256: "b".repeat(64),
                protected_auth_ready: true,
                session_created: false,
                stream_started: false,
            })
        }

        async fn list_devices(&self) -> anyhow::Result<Vec<DeviceInfo>> {
            Ok(Vec::new())
        }

        async fn refresh_device(&self, _udid: &str) -> anyhow::Result<DeviceInfo> {
            anyhow::bail!("unused")
        }

        async fn install_app(&self, _udid: &str, _path: &Path) -> anyhow::Result<()> {
            Ok(())
        }

        async fn uninstall_app(&self, _udid: &str, _bundle_id: &str) -> anyhow::Result<()> {
            Ok(())
        }

        async fn screenshot(&self, _udid: &str, dest: &Path) -> anyhow::Result<PathBuf> {
            Ok(dest.to_path_buf())
        }

        async fn syslog_tail(&self, _udid: &str, _lines: usize) -> anyhow::Result<String> {
            Ok(String::new())
        }

        async fn launch_app(&self, _udid: &str, _bundle_id: &str) -> anyhow::Result<()> {
            Ok(())
        }

        async fn terminate_app(
            &self,
            _udid: &str,
            bundle_id: &str,
        ) -> anyhow::Result<crate::ProcessAbsenceProof> {
            Ok(crate::ProcessAbsenceProof {
                bundle_id: bundle_id.to_string(),
                old_pid: None,
            })
        }

        async fn reboot(&self, _udid: &str) -> anyhow::Result<()> {
            Ok(())
        }

        async fn start_ui_session(&self, _udid: &str) -> anyhow::Result<Box<dyn UiSession>> {
            if self.require_install_only.load(Ordering::Relaxed)
                && self.install_only_calls.load(Ordering::Relaxed) == 0
            {
                anyhow::bail!("install-only readiness must precede the UI session");
            }
            self.session_calls.fetch_add(1, Ordering::Relaxed);
            self.session_live.store(true, Ordering::Release);
            Ok(Box::new(QueueTestSession))
        }

        fn invalidate_ui_session(&self, _udid: &str) {
            self.session_live.store(false, Ordering::Release);
        }

        async fn ensure_stream(&self, _udid: &str) -> anyhow::Result<String> {
            self.stream_calls.fetch_add(1, Ordering::Relaxed);
            Ok(String::new())
        }

        async fn prepare_device(&self, _udid: &str) -> anyhow::Result<()> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn shutdown_cancels_and_joins_an_unbounded_wait_without_leaking_the_owner() {
        let root = std::env::temp_dir().join(format!("riviu-job-shutdown-{}", Uuid::new_v4()));
        let db = Arc::new(Database::open(root.join("jobs.db")).expect("test database"));
        let events = EventBus::new(16);
        let work = Arc::new(DeviceWorkCoordinator::new());
        let control = Arc::new(DeviceControlPlane::new(
            Arc::new(QueueTestDriver::default()),
            work.clone(),
            Arc::new(StreamBudgetManager::new(1).expect("stream budget")),
        ));
        let queue = JobQueue::new(
            db,
            events.clone(),
            DeviceRegistry::new(events),
            control.clone(),
            root.join("artifacts"),
        );
        let job = queue
            .enqueue(
                AutomationScript {
                    version: 1,
                    name: "shutdown fixture".to_string(),
                    steps: vec![ScriptAction::Wait {
                        milliseconds: u64::MAX,
                    }],
                },
                vec!["iphone-a".to_string()],
            )
            .await
            .expect("queued job");
        timeout(Duration::from_secs(1), async {
            while work.current_owner("iphone-a") != Some(DeviceWorkOwner::Script) {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("script acquires the device");

        timeout(Duration::from_secs(1), queue.shutdown())
            .await
            .expect("cooperative shutdown is bounded")
            .expect("job queue shutdown");

        assert_eq!(work.current_owner("iphone-a"), None);
        let saved = queue
            .list_jobs(10)
            .expect("saved jobs")
            .into_iter()
            .find(|saved| saved.id == job.id)
            .expect("saved job");
        assert_eq!(saved.status, JobStatus::Cancelled);
        control.shutdown_cleanup().await.expect("control shutdown");
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn script_proves_install_only_readiness_without_starting_a_stream() {
        let root = std::env::temp_dir().join(format!("riviu-job-install-{}", Uuid::new_v4()));
        let db = Arc::new(Database::open(root.join("jobs.db")).expect("test database"));
        let events = EventBus::new(16);
        let driver = Arc::new(QueueTestDriver::requiring_install_only());
        let control = Arc::new(DeviceControlPlane::new(
            driver.clone(),
            Arc::new(DeviceWorkCoordinator::new()),
            Arc::new(StreamBudgetManager::new(1).expect("stream budget")),
        ));
        let queue = JobQueue::new(
            db,
            events.clone(),
            DeviceRegistry::new(events),
            control.clone(),
            root.join("artifacts"),
        );
        let job = queue
            .enqueue(
                AutomationScript {
                    version: 1,
                    name: "producer-free session fixture".to_string(),
                    steps: vec![ScriptAction::Wait { milliseconds: 1 }],
                },
                vec!["iphone-a".to_string()],
            )
            .await
            .expect("queued job");

        timeout(Duration::from_secs(1), async {
            loop {
                let status = queue
                    .list_jobs(10)
                    .expect("saved jobs")
                    .into_iter()
                    .find(|saved| saved.id == job.id)
                    .expect("saved job")
                    .status;
                if !matches!(status, JobStatus::Queued | JobStatus::Running) {
                    break status;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("script completes")
        .eq(&JobStatus::Succeeded)
        .then_some(())
        .expect("script succeeds after install-only readiness");

        assert_eq!(driver.install_only_calls.load(Ordering::Relaxed), 1);
        assert_eq!(driver.session_calls.load(Ordering::Relaxed), 1);
        assert_eq!(driver.stream_calls.load(Ordering::Relaxed), 0);
        assert!(!driver.session_live.load(Ordering::Acquire));
        queue.shutdown().await.expect("queue shutdown");
        control.shutdown_cleanup().await.expect("control shutdown");
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn artifact_directory_failure_closes_the_cached_session() {
        let root = std::env::temp_dir().join(format!("riviu-job-artifact-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&root).expect("test root");
        let blocked_artifacts = root.join("blocked-artifacts");
        std::fs::write(&blocked_artifacts, b"not a directory").expect("blocking file");
        let db = Arc::new(Database::open(root.join("jobs.db")).expect("test database"));
        let events = EventBus::new(16);
        let driver = Arc::new(QueueTestDriver::requiring_install_only());
        let control = Arc::new(DeviceControlPlane::new(
            driver.clone(),
            Arc::new(DeviceWorkCoordinator::new()),
            Arc::new(StreamBudgetManager::new(1).expect("stream budget")),
        ));
        let queue = JobQueue::new(
            db,
            events.clone(),
            DeviceRegistry::new(events),
            control.clone(),
            blocked_artifacts,
        );
        let job = queue
            .enqueue(
                AutomationScript {
                    version: 1,
                    name: "artifact failure fixture".to_string(),
                    steps: vec![ScriptAction::Wait { milliseconds: 1 }],
                },
                vec!["iphone-a".to_string()],
            )
            .await
            .expect("queued job");

        let status = timeout(Duration::from_secs(1), async {
            loop {
                let status = queue
                    .list_jobs(10)
                    .expect("saved jobs")
                    .into_iter()
                    .find(|saved| saved.id == job.id)
                    .expect("saved job")
                    .status;
                if !matches!(status, JobStatus::Queued | JobStatus::Running) {
                    break status;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("script failure is bounded");

        assert_eq!(status, JobStatus::Failed);
        assert_eq!(driver.session_calls.load(Ordering::Relaxed), 1);
        assert!(!driver.session_live.load(Ordering::Acquire));
        queue.shutdown().await.expect("queue shutdown");
        control.shutdown_cleanup().await.expect("control shutdown");
        let _ = std::fs::remove_dir_all(root);
    }
}
