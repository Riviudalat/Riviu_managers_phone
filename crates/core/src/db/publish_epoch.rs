//! Closing a reporting epoch never erases publication evidence or fabricates an ACK.
use super::*;
use crate::publish_sheet::SheetDeliveryTarget;

impl Database {
    pub fn begin_publish_sheet_reset(
        &self,
        target: &SheetDeliveryTarget,
        new_epoch: &str,
    ) -> anyhow::Result<()> {
        target.validate()?;
        Uuid::parse_str(new_epoch)?;
        anyhow::ensure!(target.sheet_gid == 0, "Reset chỉ dành cho tab gid=0");
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let pending: Option<String>=tx.query_row("SELECT pending_epoch FROM publish_reporting_epochs WHERE spreadsheet_id=?1 AND sheet_gid=?2 AND paused=1",
            params![target.spreadsheet_id,i64::try_from(target.sheet_gid)?],|r|r.get(0)).optional()?.flatten();
        anyhow::ensure!(
            pending.as_deref().is_none_or(|v| v == new_epoch),
            "Một đợt dọn Sheet đang chờ hoàn tất"
        );
        tx.execute("INSERT INTO publish_reporting_epochs(spreadsheet_id,sheet_gid,epoch,paused,pending_epoch) VALUES(?1,?2,?3,1,?4)
            ON CONFLICT(spreadsheet_id,sheet_gid) DO UPDATE SET paused=1,pending_epoch=excluded.pending_epoch",
            params![target.spreadsheet_id,i64::try_from(target.sheet_gid)?,target.reporting_epoch.as_deref().unwrap_or("legacy"),new_epoch])?;
        tx.commit()?;
        Ok(())
    }

    pub fn publish_sheet_requests_drained(&self) -> anyhow::Result<bool> {
        self.conn()?.query_row("SELECT NOT EXISTS(SELECT 1 FROM publish_sheet_sync_state WHERE claim_token IS NOT NULL AND claim_until_ms>?1)",
            [Utc::now().timestamp_millis()],|r|r.get(0)).map_err(Into::into)
    }

    pub fn finish_publish_sheet_reset(
        &self,
        target: &SheetDeliveryTarget,
        new_epoch: &str,
        backup_id: &str,
    ) -> anyhow::Result<()> {
        target.validate()?;
        Uuid::parse_str(new_epoch)?;
        anyhow::ensure!(target.sheet_gid == 0, "Reset chỉ dành cho tab gid=0");
        anyhow::ensure!(
            !backup_id.trim().is_empty(),
            "Thiếu bản sao Sheet đã đọc lại"
        );
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let drained: bool = tx.query_row(
            "SELECT NOT EXISTS(SELECT 1 FROM publish_sheet_sync_state WHERE claim_token IS NOT NULL AND claim_until_ms>?1)",
            [Utc::now().timestamp_millis()], |row| row.get(0)
        )?;
        anyhow::ensure!(
            drained,
            "Request Sheet đang chạy; chờ kết thúc trước khi hoàn tất đợt dọn"
        );
        let pending:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM publish_reporting_epochs WHERE spreadsheet_id=?1 AND sheet_gid=?2 AND paused=1 AND pending_epoch=?3)",
            params![target.spreadsheet_id,i64::try_from(target.sheet_gid)?,new_epoch],|r|r.get(0))?;
        anyhow::ensure!(pending, "Reset Sheet đã thay đổi; đọc lại trạng thái");
        tx.execute("INSERT OR IGNORE INTO publish_sheet_sync_state(assignment_id) SELECT assignment_id FROM publish_sheet_outbox WHERE delivery_target_json IS NOT NULL",[])?;
        // Enroll every old assignment now, including those whose link arrives later.
        tx.execute("INSERT OR IGNORE INTO publish_sheet_sync_state(assignment_id)
            SELECT a.id FROM publish_assignments a JOIN publish_campaigns c ON c.id=a.campaign_id
            WHERE json_extract(c.request_json,'$.sheetDelivery.spreadsheetId')=?1 AND json_extract(c.request_json,'$.sheetDelivery.sheetGid')=?2
            AND COALESCE(json_extract(c.request_json,'$.sheetDelivery.reportingEpoch'),'legacy')<>?3",
            params![target.spreadsheet_id,i64::try_from(target.sheet_gid)?,new_epoch])?;
        tx.execute("UPDATE publish_sheet_sync_state SET superseded_epoch=?3,claim_token=NULL,claim_until_ms=NULL,claim_kind=NULL
            WHERE assignment_id IN (SELECT a.id FROM publish_assignments a JOIN publish_campaigns c ON c.id=a.campaign_id
              WHERE json_extract(c.request_json,'$.sheetDelivery.spreadsheetId')=?1 AND json_extract(c.request_json,'$.sheetDelivery.sheetGid')=?2
              AND COALESCE(json_extract(c.request_json,'$.sheetDelivery.reportingEpoch'),'legacy')<>?3)
            OR assignment_id IN(SELECT assignment_id FROM publish_sheet_outbox
              WHERE json_extract(delivery_target_json,'$.spreadsheetId')=?1 AND json_extract(delivery_target_json,'$.sheetGid')=?2
              AND COALESCE(json_extract(delivery_target_json,'$.reportingEpoch'),'legacy')<>?3)",
            params![target.spreadsheet_id,i64::try_from(target.sheet_gid)?,new_epoch])?;
        tx.execute("UPDATE publish_reporting_epochs SET epoch=?3,paused=0,pending_epoch=NULL WHERE spreadsheet_id=?1 AND sheet_gid=?2",
            params![target.spreadsheet_id,i64::try_from(target.sheet_gid)?,new_epoch])?;
        tx.execute("INSERT INTO settings(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            params![format!("publish.sheet.backup.{new_epoch}"),backup_id])?;
        let direct: Option<String> = tx
            .query_row(
                "SELECT value FROM settings WHERE key='google.sheets.connection.v1'",
                [],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(raw) = direct.filter(|s| !s.is_empty()) {
            let mut connection: super::GoogleSheetConnection = serde_json::from_str(&raw)?;
            if connection.target.spreadsheet_id == target.spreadsheet_id
                && connection.target.sheet_gid == target.sheet_gid
            {
                connection.target.reporting_epoch = Some(new_epoch.into());
                tx.execute(
                    "UPDATE settings SET value=?1 WHERE key='google.sheets.connection.v1'",
                    [serde_json::to_string(&connection)?],
                )?;
            }
        }
        let mappings: Option<String> = tx
            .query_row(
                "SELECT value FROM settings WHERE key='google.sheets.connections.v1'",
                [],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(raw) = mappings {
            let mut connections: Vec<super::GoogleSheetConnection> = serde_json::from_str(&raw)?;
            for connection in &mut connections {
                if connection.target.spreadsheet_id == target.spreadsheet_id
                    && connection.target.sheet_gid == target.sheet_gid
                {
                    connection.target.reporting_epoch = Some(new_epoch.into());
                }
            }
            tx.execute(
                "UPDATE settings SET value=?1 WHERE key='google.sheets.connections.v1'",
                [serde_json::to_string(&connections)?],
            )?;
        }
        tx.commit()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::super::publish_pipeline::tests::fixture;
    use super::*;
    #[test]
    fn reset_drain_supersedes_old_obligations_without_sending_or_erasing_history() {
        let (db, _, campaign, assignments) = fixture();
        let target = SheetDeliveryTarget {
            version: 2,
            spreadsheet_id: "fixture-book".into(),
            sheet_gid: 0,
            internal_reporting: true,
            reporting_epoch: Some("legacy".into()),
        };
        db.conn().unwrap().execute("UPDATE publish_campaigns SET request_json=json_set(request_json,'$.sheetDelivery',json(?2)) WHERE id=?1",
            params![campaign,serde_json::to_string(&target).unwrap()]).unwrap();
        let claim = db
            .claim_bound_sheet_delivery(
                SheetDeliveryKind::Report,
                None,
                Utc::now().timestamp_millis(),
            )
            .unwrap()
            .unwrap();
        let epoch = Uuid::new_v4().to_string();
        db.begin_publish_sheet_reset(&target, &epoch).unwrap();
        assert!(!db.publish_sheet_requests_drained().unwrap());
        assert!(db
            .finish_publish_sheet_reset(&target, &epoch, "verified-backup")
            .is_err());
        assert!(db
            .claim_bound_sheet_delivery(
                SheetDeliveryKind::Report,
                None,
                Utc::now().timestamp_millis()
            )
            .unwrap()
            .is_none());
        db.fail_bound_sheet_delivery(
            &claim,
            "interrupted response",
            true,
            Utc::now().timestamp_millis(),
        )
        .unwrap();
        assert!(db.publish_sheet_requests_drained().unwrap());
        db.finish_publish_sheet_reset(&target, &epoch, "verified-backup")
            .unwrap();
        assert!(matches!(
            db.settle_bound_sheet_delivery(&claim, None, None, Utc::now().timestamp_millis())
                .unwrap(),
            SheetOutboxSettlement::StaleRevision
        ));
        assert!(db
            .claim_bound_sheet_delivery(
                SheetDeliveryKind::Report,
                None,
                Utc::now().timestamp_millis()
            )
            .unwrap()
            .is_none());
        let detail = db.get_publish_campaign(&campaign).unwrap().unwrap();
        assert_eq!(detail.assignments.len(), assignments.len());
        assert!(detail
            .assignments
            .iter()
            .all(|a| a.sheet_delivery.as_ref().unwrap().state == "superseded"));
        assert_eq!(
            db.conn()
                .unwrap()
                .query_row(
                    "SELECT COUNT(*) FROM publish_sheet_outbox WHERE state='sent'",
                    [],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
            0
        );
    }
}
