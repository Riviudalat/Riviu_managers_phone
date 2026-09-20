//! Offline evidence replay. No driver crates or device transport are linked here.
use riviu_core::ipc_contract::OperationTrace;
use sha2::{Digest, Sha256};

#[derive(Default)]
struct FakeDriver {
    steps: usize,
}
impl FakeDriver {
    fn observe(&mut self, step: &riviu_core::operation::OperationDeviceLogEntry) {
        self.steps += 1;
        println!(
            "{}",
            serde_json::json!({"step":step.id,"at":step.at,"state":step.state,"action":step.action,"deviceEffects":0})
        );
    }
}
fn main() -> anyhow::Result<()> {
    let path = std::env::args()
        .nth(1)
        .ok_or_else(|| anyhow::anyhow!("usage: trace_replay TRACE_JSON"))?;
    let trace: OperationTrace = serde_json::from_slice(&std::fs::read(path)?)?;
    for artifact in &trace.artifacts {
        let bytes = std::fs::read(&artifact.path)?;
        anyhow::ensure!(
            bytes.len() as u64 == artifact.bytes
                && format!("{:x}", Sha256::digest(&bytes)) == artifact.sha256,
            "Trace artifact changed"
        );
    }
    let mut driver = FakeDriver::default();
    for step in &trace.steps {
        driver.observe(step);
    }
    for observation in &trace.observations {
        anyhow::ensure!(
            observation.device_id == trace.device_id,
            "Trace device mismatch"
        );
        if let Some(xml) = &observation.hierarchy {
            riviu_core::ui_automation::tree::Tree::parse(riviu_core::HierarchySourceSnapshot {
                generation: observation
                    .hierarchy_generation
                    .ok_or_else(|| anyhow::anyhow!("missing hierarchy generation"))?,
                xml: std::fs::read_to_string(&xml.path)?,
            })?;
        }
        driver.steps += 1;
        println!(
            "{}",
            serde_json::json!({"session":observation.session_id,"step":observation.sequence,
            "action":observation.action,"elapsedMs":observation.elapsed_ms,"state":observation.state,"deviceEffects":0})
        );
    }
    println!(
        "{}",
        serde_json::json!({"run":trace.run_id,"device":trace.device_id,"observations":driver.steps,"driver":"fake","deviceEffects":0})
    );
    Ok(())
}
