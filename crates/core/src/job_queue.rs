use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;

use chrono::Utc;
use parking_lot::Mutex;
use tokio::sync::Semaphore;
use uuid::Uuid;

use crate::db::Database;
use crate::driver::DeviceDriver;
use crate::events::{AppEvent, EventBus};
use crate::registry::DeviceRegistry;
use crate::types::{
    AutomationScript, DeviceStatus, JobRecord, JobStatus, JobStepRecord, ScriptAction, StepStatus,
};

#[derive(Clone)]
pub struct JobQueue {
    db: Arc<Database>,
    events: EventBus,
    registry: DeviceRegistry,
    driver: Arc<dyn DeviceDriver>,
    artifacts_dir: PathBuf,
    cancelled: Arc<Mutex<HashSet<Uuid>>>,
    device_locks: Arc<Mutex<HashMap<String, Arc<Semaphore>>>>,
}

impl JobQueue {
    pub fn new(
        db: Arc<Database>,
        events: EventBus,
        registry: DeviceRegistry,
        driver: Arc<dyn DeviceDriver>,
        artifacts_dir: PathBuf,
    ) -> Self {
        std::fs::create_dir_all(&artifacts_dir).ok();
        Self {
            db,
            events,
            registry,
            driver,
            artifacts_dir,
            cancelled: Arc::new(Mutex::new(HashSet::new())),
            device_locks: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    fn device_sem(&self, udid: &str) -> Arc<Semaphore> {
        let mut map = self.device_locks.lock();
        map.entry(udid.to_string())
            .or_insert_with(|| Arc::new(Semaphore::new(1)))
            .clone()
    }

    pub fn list_jobs(&self, limit: usize) -> anyhow::Result<Vec<JobRecord>> {
        self.db.list_jobs(limit)
    }

    pub fn cancel(&self, job_id: Uuid) {
        self.cancelled.lock().insert(job_id);
    }

    pub async fn enqueue(
        &self,
        script: AutomationScript,
        udids: Vec<String>,
    ) -> anyhow::Result<JobRecord> {
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
        tokio::spawn(async move {
            if let Err(err) = this.run_job(job_id, script_clone, udids).await {
                tracing::error!("job {job_id} failed: {err:#}");
            }
        });

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
            if self.cancelled.lock().contains(&job_id) {
                job.status = JobStatus::Cancelled;
                job.updated_at = Utc::now();
                self.persist(&job);
                return Ok(());
            }

            let sem = self.device_sem(udid);
            let _permit = sem.acquire().await?;
            self.registry
                .set_status(udid, DeviceStatus::Busy, None);

            match self.run_on_device(&mut job, &script, udid).await {
                Ok(()) => {
                    self.registry
                        .set_status(udid, DeviceStatus::Ready, None);
                }
                Err(err) => {
                    let msg = format!("{err:#}");
                    first_error.get_or_insert(msg.clone());
                    self.registry
                        .set_status(udid, DeviceStatus::Error, Some(msg));
                }
            }
        }

        job.status = if self.cancelled.lock().contains(&job_id) {
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
    ) -> anyhow::Result<()> {
        let session = self.driver.start_ui_session(udid).await?;
        let run_dir = self
            .artifacts_dir
            .join(job.id.to_string())
            .join(udid);
        std::fs::create_dir_all(&run_dir)?;

        for (index, action) in script.steps.iter().enumerate() {
            if self.cancelled.lock().contains(&job.id) {
                anyhow::bail!("cancelled");
            }
            if let Some(step) = job.steps.get_mut(index) {
                step.status = StepStatus::Running;
            }
            job.updated_at = Utc::now();
            self.persist(job);

            let result = self
                .execute_step(udid, action, session.as_ref(), &run_dir)
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
                        step.status = StepStatus::Failed;
                        step.error = Some(format!("{err:#}"));
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

    async fn execute_step(
        &self,
        udid: &str,
        action: &ScriptAction,
        session: &dyn crate::driver::UiSession,
        run_dir: &std::path::Path,
    ) -> anyhow::Result<Option<String>> {
        match action {
            ScriptAction::LaunchApp { bundle_id } => {
                self.driver.launch_app(udid, bundle_id).await?;
                Ok(None)
            }
            ScriptAction::TerminateApp { bundle_id } => {
                self.driver.terminate_app(udid, bundle_id).await?;
                Ok(None)
            }
            ScriptAction::Wait { milliseconds } => {
                tokio::time::sleep(std::time::Duration::from_millis(*milliseconds)).await;
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
                let path = self.driver.screenshot(udid, &dest).await?;
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
