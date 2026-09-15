//! Offline maintenance: stop the desktop, then supply its database and local connection file.
//! Never writes a test row. Reuse reset-id after an interrupted/ambiguous response.
use anyhow::Context;
use riviu_core::{db::Database, publish_sheet::*};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    anyhow::ensure!(
        args.len() == 4,
        "usage: reset_publication_reporting DB CONNECTION_JSON RESET_ID"
    );
    uuid::Uuid::parse_str(&args[3])?;
    let db = Database::open(&args[1])?;
    let local: serde_json::Value = serde_json::from_slice(&std::fs::read(&args[2])?)?;
    let settings = SheetDeliverySettings {
        webhook_url: local["webhookUrl"]
            .as_str()
            .context("webhookUrl missing")?
            .into(),
        token: local["token"].as_str().context("token missing")?.into(),
        internal_reporting: true,
    };
    let url = db
        .get_setting(SHEET_URL_SETTING)?
        .filter(|s| !s.is_empty())
        .context("Sheet URL missing in database")?;
    let check = check_sheet(&url, &settings).await?;
    anyhow::ensure!(check.connection_verified, "{}", check.message);
    let target = SheetDeliveryTarget {
        version: 2,
        spreadsheet_id: check.spreadsheet_id,
        sheet_gid: check.sheet_gid,
        internal_reporting: check.layout.as_deref() == Some("internal"),
        reporting_epoch: check.reporting_epoch,
    };
    db.begin_publish_sheet_reset(&target, &args[3])?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(125);
    while !db.publish_sheet_requests_drained()? {
        anyhow::ensure!(
            std::time::Instant::now() < deadline,
            "Sheet requests still active; resume same reset ID"
        );
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    }
    let backup = reset_reporting_sheet(&settings, &target, &args[3]).await?;
    db.finish_publish_sheet_reset(&target, &args[3], &backup)?;
    println!(
        "{}",
        serde_json::json!({"complete":true,"reportingEpoch":args[3],"backupSpreadsheetId":backup,"sheetGid":0})
    );
    Ok(())
}
