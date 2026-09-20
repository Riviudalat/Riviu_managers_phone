//! Compare existing and batched read-only projections on a backed-up database.
use riviu_core::{
    db::Database, project_publish_detail_with_target, OperationRunKind, OperationRunState,
};
use std::time::Instant;

fn main() -> anyhow::Result<()> {
    let path = std::env::args()
        .nth(1)
        .ok_or_else(|| anyhow::anyhow!("usage: monitor_bench DB_COPY"))?;
    let db = Database::open(path)?;
    let ids = db
        .operation_source_ids(None, Some(OperationRunKind::Publish))?
        .into_iter()
        .filter_map(|id| id.strip_prefix("publish:").map(str::to_owned))
        .collect::<Vec<_>>();
    let started = Instant::now();
    let mut baseline = Vec::new();
    for id in &ids {
        let Some(detail) = db.get_publish_campaign(id)? else {
            continue;
        };
        let snapshot = db.get_publish_execution_snapshot(id)?;
        let request = db.publish_campaign_request(id)?;
        let mut summary = project_publish_detail_with_target(
            &detail,
            snapshot.as_ref(),
            request.as_ref().and_then(|r| r.target_snapshot.as_ref()),
        )
        .summary;
        if db.publish_operation_stopped(id)? {
            summary.state = OperationRunState::Cancelled;
        }
        baseline.push(summary);
    }
    let baseline_ms = started.elapsed().as_millis();
    let started = Instant::now();
    let modified = db.publish_operation_summaries(&ids)?;
    let modified_ms = started.elapsed().as_millis();
    anyhow::ensure!(baseline == modified, "batched monitor projection differs");
    println!(
        "{}",
        serde_json::json!({"campaigns":ids.len(),"baselineMs":baseline_ms,"modifiedMs":modified_ms,"identicalProjection":true})
    );
    Ok(())
}
