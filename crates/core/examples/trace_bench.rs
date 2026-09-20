use riviu_core::{
    ui_automation::{
        trace::{new_step, TraceRecorder},
        GuiScope,
    },
    FrameSource,
};
use std::{path::PathBuf, sync::Arc, time::Instant};
struct Frames(Arc<Vec<u8>>);
impl FrameSource for Frames {
    fn subscribe(&self, _: &str) -> Box<dyn riviu_core::frame_source::FrameStream> {
        panic!("benchmark does not subscribe")
    }
    fn latest(&self, _: &str) -> Option<Arc<Vec<u8>>> {
        Some(self.0.clone())
    }
}
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    anyhow::ensure!(args.len() == 2, "usage: trace_bench INPUT_IMAGE OUTPUT_DIR");
    let bytes = std::fs::read(&args[0])?;
    let root = PathBuf::from(&args[1]);
    let start = Instant::now();
    for _ in 0..10 {
        image::load_from_memory(&bytes)?;
    }
    let decode_ms = start.elapsed().as_millis();
    let recorder = TraceRecorder::new(&root, Arc::new(Frames(Arc::new(bytes))))?;
    let start = Instant::now();
    for sequence in 0..10 {
        let step = new_step(
            GuiScope {
                run_id: "bench".into(),
                device_id: "fixture".into(),
                assignment_id: None,
                deadline_ms: None,
            },
            "fixture-session".into(),
            sequence,
            "hierarchy",
            0,
            None,
        );
        recorder
            .record(
                step,
                Some("<hierarchy><node text=\"fixture\"/></hierarchy>".into()),
            )
            .await?;
    }
    recorder.flush().await?;
    println!(
        "{}",
        serde_json::json!({"samples":10,"decodeMs":decode_ms,"persistMs":start.elapsed().as_millis()})
    );
    Ok(())
}
