//! Observation records in the existing artifact store; no device commands or replay transport.
use super::GuiScope;
use crate::{ipc_contract::TraceArtifact, FlowArtifactStore, FrameSource};
use anyhow::{ensure, Context};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    path::Path,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
};
use tokio::sync::{mpsc, oneshot};
use uuid::Uuid;

const MAX_ARTIFACT_BYTES: u64 = 16 * 1024 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
pub struct DeviceTraceStep {
    pub run_id: String,
    pub device_id: String,
    pub assignment_id: Option<String>,
    pub session_id: String,
    pub sequence: u64,
    pub observed_at: String,
    pub elapsed_ms: u64,
    pub action: String,
    /// A transport acknowledgement is never a business postcondition.
    pub state: String,
    pub error: Option<String>,
    pub hierarchy_generation: Option<u64>,
    pub hierarchy: Option<TraceArtifact>,
    /// The currently cached stream frame, for diagnosis only; freshness is unverified.
    pub image: Option<TraceArtifact>,
}

#[derive(Clone)]
pub struct TraceRecorder {
    queue: mpsc::Sender<TraceMessage>,
    frames: Arc<dyn FrameSource>,
    failures: Arc<AtomicU64>,
}

struct TraceWrite {
    step: DeviceTraceStep,
    xml: Option<String>,
    image: Option<crate::frame_source::Frame>,
    done: Option<oneshot::Sender<anyhow::Result<()>>>,
}

enum TraceMessage {
    Record(Box<TraceWrite>),
    Flush(oneshot::Sender<anyhow::Result<()>>),
}

fn path_id(identity: &str) -> Uuid {
    let digest = Sha256::digest(identity.as_bytes());
    let mut bytes = [0; 16];
    bytes.copy_from_slice(&digest[..16]);
    Uuid::from_bytes(bytes)
}

impl TraceRecorder {
    pub fn new(root: impl AsRef<Path>, frames: Arc<dyn FrameSource>) -> anyhow::Result<Self> {
        let store = FlowArtifactStore::new(root.as_ref())?;
        let root = root.as_ref().canonicalize()?;
        let (queue, mut incoming) = mpsc::channel(64);
        let failures = Arc::new(AtomicU64::new(0));
        let worker_failures = failures.clone();
        tokio::runtime::Handle::try_current()?.spawn(async move {
            while let Some(message) = incoming.recv().await {
                match message {
                    TraceMessage::Record(write) => {
                        let TraceWrite {
                            step,
                            xml,
                            image,
                            done,
                        } = *write;
                        let (root, store) = (root.clone(), store.clone());
                        let result = tokio::task::spawn_blocking(move || {
                            persist(&root, &store, step, xml, image)
                        })
                        .await
                        .map_err(anyhow::Error::from)
                        .and_then(|value| value);
                        if let Err(error) = &result {
                            worker_failures.fetch_add(1, Ordering::Relaxed);
                            tracing::warn!("trace persistence incomplete: {error:#}");
                        }
                        if let Some(done) = done {
                            let _ = done.send(result);
                        }
                    }
                    TraceMessage::Flush(done) => {
                        let failures = worker_failures.load(Ordering::Relaxed);
                        let result = if failures == 0 {
                            Ok(())
                        } else {
                            Err(anyhow::anyhow!(
                                "TraceIncomplete: {failures} observations could not be persisted"
                            ))
                        };
                        let _ = done.send(result);
                    }
                }
            }
        });
        Ok(Self {
            queue,
            frames,
            failures,
        })
    }

    /// Admission is bounded before any blocking work. Capturing a trace cannot
    /// add disk latency or another device request to an input/readback deadline.
    pub fn enqueue(&self, step: DeviceTraceStep, xml: Option<String>) -> anyhow::Result<()> {
        self.submit(step, xml, None)
    }

    pub fn enqueue_image(&self, step: DeviceTraceStep, bytes: Vec<u8>) -> anyhow::Result<()> {
        self.submit_with_image(step, None, Some(Arc::new(bytes)), None)
    }

    fn submit(
        &self,
        step: DeviceTraceStep,
        xml: Option<String>,
        done: Option<oneshot::Sender<anyhow::Result<()>>>,
    ) -> anyhow::Result<()> {
        let image = self.frames.latest(&step.device_id);
        self.submit_with_image(step, xml, image, done)
    }

    fn submit_with_image(
        &self,
        step: DeviceTraceStep,
        xml: Option<String>,
        image: Option<crate::frame_source::Frame>,
        done: Option<oneshot::Sender<anyhow::Result<()>>>,
    ) -> anyhow::Result<()> {
        ensure!(
            !step.session_id.is_empty(),
            "Trace requires a real session identity"
        );
        self.queue
            .try_send(TraceMessage::Record(Box::new(TraceWrite {
                step,
                xml,
                image,
                done,
            })))
            .map_err(|_| {
                self.failures.fetch_add(1, Ordering::Relaxed);
                anyhow::anyhow!("TraceIncomplete: artifact queue full or closed")
            })
    }

    pub async fn record(&self, step: DeviceTraceStep, xml: Option<String>) -> anyhow::Result<()> {
        let (done, finished) = oneshot::channel();
        self.submit(step, xml, Some(done))?;
        finished.await?
    }

    /// Used before export and after device workers drain during shutdown.
    pub async fn flush(&self) -> anyhow::Result<()> {
        let (done, finished) = oneshot::channel();
        self.queue
            .send(TraceMessage::Flush(done))
            .await
            .map_err(|_| anyhow::anyhow!("Trace writer stopped"))?;
        finished.await?
    }
}

fn persist(
    root: &Path,
    store: &FlowArtifactStore,
    mut step: DeviceTraceStep,
    xml: Option<String>,
    image: Option<crate::frame_source::Frame>,
) -> anyhow::Result<()> {
    let run = path_id(&step.run_id);
    let device = path_id(&step.device_id);
    let attempt = Uuid::new_v4();
    let publish =
        |prepared: crate::flow::artifact_store::PreparedArtifact| -> anyhow::Result<TraceArtifact> {
            let relative = store.publish_file(&prepared)?;
            Ok(TraceArtifact {
                path: root.join(relative).to_string_lossy().into_owned(),
                sha256: prepared.sha256,
                bytes: prepared.size,
            })
        };
    if let Some(xml) = xml {
        step.hierarchy = Some(publish(
            store.prepare_hierarchy(run, device, attempt, &xml)?,
        )?);
    }
    if let Some(bytes) = image {
        ensure!(
            bytes.len() as u64 <= MAX_ARTIFACT_BYTES,
            "Trace image too large"
        );
        let kind = match image::guess_format(&bytes)? {
            image::ImageFormat::Png => "png",
            image::ImageFormat::Jpeg => "jpeg",
            _ => anyhow::bail!("Unsupported trace image"),
        };
        step.image = Some(publish(store.prepare_image(
            run,
            device,
            attempt,
            &format!("observation.{kind}"),
            kind,
            &bytes,
        )?)?);
    }
    publish(store.prepare_trace(run, device, attempt, &serde_json::to_value(step)?)?)?;
    Ok(())
}

pub fn new_step(
    scope: GuiScope,
    session_id: String,
    sequence: u64,
    action: &str,
    elapsed_ms: u64,
    error: Option<String>,
) -> DeviceTraceStep {
    DeviceTraceStep {
        run_id: scope.run_id,
        device_id: scope.device_id,
        assignment_id: scope.assignment_id,
        session_id,
        sequence,
        observed_at: chrono::Utc::now().to_rfc3339(),
        elapsed_ms,
        action: action.into(),
        state: if error.is_some() {
            "failed"
        } else if action == "hierarchy" {
            "observed"
        } else {
            "acknowledged"
        }
        .into(),
        error,
        hierarchy_generation: None,
        hierarchy: None,
        image: None,
    }
}

/// Reopen only this run/device's immutable records and verify every referenced byte.
pub fn read_run(
    root: &Path,
    run_id: &str,
    device_id: &str,
) -> anyhow::Result<Vec<DeviceTraceStep>> {
    if !root.exists() {
        return Ok(Vec::new());
    }
    let root = root.canonicalize()?;
    let folder = root
        .join(path_id(run_id).to_string())
        .join(path_id(device_id).to_string());
    if !folder.exists() {
        return Ok(Vec::new());
    }
    ensure!(
        folder.canonicalize()?.starts_with(&root),
        "Trace path escaped root"
    );
    let mut steps = Vec::new();
    for attempt in std::fs::read_dir(folder)? {
        let attempt = attempt?;
        ensure!(
            attempt.file_type()?.is_dir() && !attempt.file_type()?.is_symlink(),
            "Invalid trace directory"
        );
        for entry in std::fs::read_dir(attempt.path())? {
            let path = entry?.path();
            if path.extension().and_then(|s| s.to_str()) != Some("json") {
                continue;
            }
            let bytes = read_bounded(&root, &path)?;
            let step: DeviceTraceStep = serde_json::from_slice(&bytes)?;
            ensure!(
                step.run_id == run_id && step.device_id == device_id,
                "Trace scope mismatch"
            );
            for artifact in [&step.hierarchy, &step.image].into_iter().flatten() {
                let bytes = read_bounded(&root, Path::new(&artifact.path))?;
                ensure!(
                    bytes.len() as u64 == artifact.bytes
                        && format!("{:x}", Sha256::digest(bytes)) == artifact.sha256,
                    "Trace artifact changed"
                );
            }
            steps.push(step);
            ensure!(
                steps.len() <= 10_000,
                "Trace export exceeds 10000 observations"
            );
        }
    }
    steps.sort_by(|a, b| {
        a.observed_at
            .cmp(&b.observed_at)
            .then(a.session_id.cmp(&b.session_id))
            .then(a.sequence.cmp(&b.sequence))
    });
    Ok(steps)
}

fn read_bounded(root: &Path, path: &Path) -> anyhow::Result<Vec<u8>> {
    let metadata = std::fs::symlink_metadata(path)?;
    ensure!(
        metadata.is_file() && !metadata.file_type().is_symlink(),
        "Invalid trace file"
    );
    let path = path.canonicalize()?;
    ensure!(
        path.starts_with(root) && metadata.len() <= MAX_ARTIFACT_BYTES,
        "Trace file outside bounds"
    );
    std::fs::read(path).context("read trace artifact")
}

#[cfg(test)]
mod tests {
    use super::*;
    struct FixtureFrames(crate::frame_source::Frame);
    impl FrameSource for FixtureFrames {
        fn subscribe(&self, _: &str) -> Box<dyn crate::frame_source::FrameStream> {
            panic!("trace never opens a stream")
        }
        fn latest(&self, _: &str) -> Option<crate::frame_source::Frame> {
            Some(self.0.clone())
        }
    }
    #[tokio::test]
    async fn trace_reopens_exact_scope_and_rejects_modified_xml_without_device_io() {
        let root = std::env::temp_dir().join(format!("riviu-trace-fixture-{}", Uuid::new_v4()));
        let mut image = std::io::Cursor::new(Vec::new());
        image::DynamicImage::new_rgb8(2, 2)
            .write_to(&mut image, image::ImageFormat::Png)
            .unwrap();
        let recorder =
            TraceRecorder::new(&root, Arc::new(FixtureFrames(Arc::new(image.into_inner()))))
                .unwrap();
        let scope = GuiScope {
            run_id: "run-a".into(),
            device_id: "phone-a".into(),
            assignment_id: Some("step-3".into()),
            deadline_ms: None,
        };
        let mut step = new_step(scope, "session-a".into(), 4, "hierarchy", 17, None);
        step.hierarchy_generation = Some(6);
        recorder
            .record(
                step,
                Some("<hierarchy><node text=\"fixture\"/></hierarchy>".into()),
            )
            .await
            .unwrap();
        let read = read_run(&root, "run-a", "phone-a").unwrap();
        assert_eq!(read.len(), 1);
        assert_eq!(read[0].session_id, "session-a");
        assert_eq!(read[0].elapsed_ms, 17);
        assert_eq!(read[0].hierarchy_generation, Some(6));
        assert!(read[0].image.is_some());
        assert!(read_run(&root, "run-a", "phone-b").unwrap().is_empty());
        std::fs::write(&read[0].hierarchy.as_ref().unwrap().path, "changed").unwrap();
        assert!(read_run(&root, "run-a", "phone-a").is_err());
    }

    #[tokio::test]
    async fn explicit_screenshot_trace_uses_capture_instead_of_cached_frame() {
        let root = std::env::temp_dir().join(format!("riviu-trace-image-{}", Uuid::new_v4()));
        let recorder =
            TraceRecorder::new(&root, Arc::new(FixtureFrames(Arc::new(vec![0])))).unwrap();
        let mut png = std::io::Cursor::new(Vec::new());
        image::DynamicImage::new_rgb8(3, 2)
            .write_to(&mut png, image::ImageFormat::Png)
            .unwrap();
        let step = new_step(
            GuiScope {
                run_id: "r".into(),
                device_id: "d".into(),
                assignment_id: None,
                deadline_ms: None,
            },
            "s".into(),
            1,
            "screenshot",
            4,
            None,
        );
        recorder.enqueue_image(step, png.into_inner()).unwrap();
        recorder.flush().await.unwrap();
        let rows = read_run(&root, "r", "d").unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].action, "screenshot");
        assert!(rows[0].image.is_some());
    }
}
