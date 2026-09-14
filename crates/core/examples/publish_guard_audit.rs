//! Inspect a copied database without controlling a device or dispatching publication.
use riviu_core::db::Database;

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let snapshot = args
        .next()
        .ok_or_else(|| anyhow::anyhow!("usage: publish_guard_audit SNAPSHOT_DB UDID"))?;
    let udid = args
        .next()
        .ok_or_else(|| anyhow::anyhow!("UDID required"))?;
    let db = Database::open(snapshot)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&db.publish_device_guard(&udid)?)?
    );
    Ok(())
}
