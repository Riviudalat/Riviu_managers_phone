use riviu_core::db::{Database, SheetDeliveryKind};
use rusqlite::Connection;
use std::{path::PathBuf, time::Instant};

fn main() -> anyhow::Result<()> {
    let root = PathBuf::from(
        std::env::args()
            .nth(1)
            .ok_or_else(|| anyhow::anyhow!("usage: storage_bench DISPOSABLE_DIRECTORY"))?,
    );
    std::fs::create_dir_all(&root)?;
    println!(
        "sqlite_version={} sqlite_version_number={}",
        rusqlite::version(),
        rusqlite::version_number()
    );
    anyhow::ensure!(
        rusqlite::version_number() >= 3_051_003,
        "SQLite must include the WAL-reset fix"
    );
    for rows in [10_000, 50_000] {
        let path = root.join(format!("history-{rows}-{}.db", uuid::Uuid::new_v4()));
        let db = Database::open(&path)?;
        let mut conn = Connection::open(&path)?;
        let tx = conn.transaction()?;
        tx.execute("WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<?1)
            INSERT INTO publish_sheet_sync_state(assignment_id,report_acked_revision,report_revision,report_enabled) SELECT 'history-'||x,5,5,1 FROM n", [rows])?;
        tx.commit()?;
        let started = Instant::now();
        for _ in 0..100 {
            anyhow::ensure!(db
                .claim_bound_sheet_delivery(
                    SheetDeliveryKind::Report,
                    None,
                    chrono::Utc::now().timestamp_millis()
                )?
                .is_none());
        }
        let plan: String = conn.query_row("EXPLAIN QUERY PLAN SELECT assignment_id FROM publish_sheet_sync_state INDEXED BY sheet_report_due WHERE superseded_epoch IS NULL AND report_due_at_ms IS NOT NULL AND report_due_at_ms<=0 ORDER BY report_due_at_ms,assignment_id LIMIT 1", [], |r| r.get(3))?;
        println!(
            "{}",
            serde_json::json!({"historyRows":rows,"idleQueries":100,"elapsedMs":started.elapsed().as_millis(),"queryPlan":plan})
        );
    }
    Ok(())
}
