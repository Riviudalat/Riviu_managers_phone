use super::*;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupReceipt {
    pub backup_spreadsheet_id: String,
    pub source_fingerprint: String,
    pub verified: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResetReceipt {
    pub backup_spreadsheet_id: String,
    pub reporting_epoch: String,
    pub complete: bool,
}

impl DirectSheetsClient {
    async fn save_owner(&self, meta: &Metadata, owner: &Owner) -> Result<()> {
        self.batch(&meta.spreadsheet_id,vec![json!({"updateDeveloperMetadata":{"dataFilters":[{"developerMetadataLookup":{"metadataId":writer_metadata_id(meta.gid)}}],"developerMetadata":{"metadataValue":serde_json::to_string(owner).expect("owner JSON")},"fields":"metadataValue"}})]).await
    }

    async fn copy_workbook(&self, book: &str, reset_id: &str) -> Result<String> {
        let value=self.request_service(true,reqwest::Method::POST,&format!("{book}/copy"),&[("fields","id,mimeType".into())],Some(&json!({"name":format!("Riviu backup {reset_id}"),"appProperties":{"riviuBackupResetId":reset_id}}))).await?;
        let id = value["id"]
            .as_str()
            .ok_or_else(|| DirectSheetsError::invalid("Drive copy did not return an ID"))?;
        validate_target(id, 0)?;
        if value["mimeType"] != "application/vnd.google-apps.spreadsheet" {
            return Err(DirectSheetsError::invalid(
                "Drive backup is not a spreadsheet",
            ));
        }
        Ok(id.into())
    }

    /// Hash every copied tab's entered values, formulas, notes, formatting,
    /// validation and row/column geometry in bounded grids. Computed values and
    /// volatile object IDs are excluded; the complete Drive copy retains them.
    async fn workbook_fingerprint(&self, book: &str) -> Result<String> {
        let value=self.request(reqwest::Method::GET,book,&[("fields","spreadsheetId,sheets(properties(title,gridProperties),merges,conditionalFormats)".into())],None).await?;
        if value["spreadsheetId"] != book {
            return Err(DirectSheetsError::conflict(
                "Backup workbook identity mismatch",
            ));
        }
        let sheets = value["sheets"]
            .as_array()
            .ok_or_else(|| DirectSheetsError::invalid("Backup tabs missing"))?;
        let mut digest = Sha256::new();
        for sheet in sheets {
            let title = sheet["properties"]["title"]
                .as_str()
                .ok_or_else(|| DirectSheetsError::invalid("Backup tab title missing"))?;
            let rows = sheet["properties"]["gridProperties"]["rowCount"]
                .as_u64()
                .filter(|n| *n > 0 && *n <= MAX_SCAN_ROWS as u64)
                .ok_or_else(|| DirectSheetsError::invalid("Backup tab exceeds bounded row count"))?
                as u32;
            let cols = sheet["properties"]["gridProperties"]["columnCount"]
                .as_u64()
                .filter(|n| *n > 0 && *n <= 18_278)
                .ok_or_else(|| DirectSheetsError::invalid("Backup tab column count invalid"))?
                as u32;
            // Ignore extra fields a transport may return. Owner metadata changes
            // when the backup receipt is persisted and is not workbook content.
            let structure = json!({"properties":sheet["properties"],"merges":sheet["merges"],"conditionalFormats":sheet["conditionalFormats"]});
            digest.update(
                serde_json::to_vec(&stable_sheet_structure(structure)).expect("properties JSON"),
            );
            for start in (0..rows).step_by(PAGE_ROWS as usize) {
                for left in (0..cols).step_by(MAX_COLUMNS as usize) {
                    let range = format!(
                        "'{}'!{}{}:{}{}",
                        title.replace('\'', "''"),
                        column_letters(left + 1),
                        start + 1,
                        column_letters((left + MAX_COLUMNS).min(cols)),
                        (start + PAGE_ROWS).min(rows)
                    );
                    let page=self.request(reqwest::Method::GET,book,&[("ranges",range),("fields","sheets(data(startRow,startColumn,rowData(values(userEnteredValue,userEnteredFormat,note,dataValidation,textFormatRuns)),rowMetadata,columnMetadata))".into())],None).await?;
                    let data = page["sheets"]
                        .as_array()
                        .filter(|s| s.len() == 1)
                        .ok_or_else(|| DirectSheetsError::invalid("Backup page missing"))?;
                    digest.update(serde_json::to_vec(&data[0]["data"]).expect("page JSON"));
                }
            }
        }
        Ok(format!("{:x}", digest.finalize()))
    }

    async fn verify_backup(&self, source: &str, backup: &str) -> Result<BackupReceipt> {
        let before = self.workbook_fingerprint(source).await?;
        let copied = self.workbook_fingerprint(backup).await?;
        let after = self.workbook_fingerprint(source).await?;
        if before != copied || before != after {
            return Err(DirectSheetsError::conflict(
                "Full workbook backup readback differs; source was not cleared",
            ));
        }
        Ok(BackupReceipt {
            backup_spreadsheet_id: backup.into(),
            source_fingerprint: before,
            verified: true,
        })
    }

    pub async fn backup_target(
        &self,
        target: &SheetDeliveryTarget,
        writer_id: &str,
    ) -> Result<BackupReceipt> {
        tokio::time::timeout(
            Duration::from_secs(600),
            self.backup_inner(target, writer_id),
        )
        .await
        .map_err(|_| DirectSheetsError::transport())?
    }
    async fn backup_inner(
        &self,
        target: &SheetDeliveryTarget,
        writer_id: &str,
    ) -> Result<BackupReceipt> {
        let _guard = WRITER_LOCK.lock().await;
        let meta = self
            .metadata(&target.spreadsheet_id, target.sheet_gid)
            .await?;
        meta.owner_matches(
            writer_id,
            target.reporting_epoch.as_deref().unwrap_or("legacy"),
        )?;
        let backup = self
            .copy_workbook(&target.spreadsheet_id, &uuid::Uuid::new_v4().to_string())
            .await?;
        self.verify_backup(&target.spreadsheet_id, &backup).await
    }

    /// Caller pauses/drains its durable outbox first. The same reset UUID resumes
    /// a failed clear; the remote owner stays resetting until readback completes.
    pub async fn reset_target(
        &self,
        target: &SheetDeliveryTarget,
        writer_id: &str,
        reset_id: &str,
    ) -> Result<ResetReceipt> {
        tokio::time::timeout(
            Duration::from_secs(600),
            self.reset_inner(target, writer_id, reset_id),
        )
        .await
        .map_err(|_| DirectSheetsError::transport())?
    }
    async fn reset_inner(
        &self,
        target: &SheetDeliveryTarget,
        writer_id: &str,
        reset_id: &str,
    ) -> Result<ResetReceipt> {
        let _guard = WRITER_LOCK.lock().await;
        if target.sheet_gid != 0 || uuid::Uuid::parse_str(reset_id).is_err() {
            return Err(DirectSheetsError::invalid(
                "Reset requires gid0 and a UUID reset ID",
            ));
        }
        let meta = self
            .metadata(&target.spreadsheet_id, target.sheet_gid)
            .await?;
        Layout::from_header(&meta.header)?;
        let mut owner = meta
            .owner
            .clone()
            .ok_or_else(|| DirectSheetsError::conflict("Tab has no writer owner"))?;
        if owner.schema_version != 1 {
            return Err(DirectSheetsError::conflict(
                "Reset is disabled for shared-v2 tabs; no backup or clear was started",
            ));
        }
        if owner.writer_id != writer_id {
            return Err(DirectSheetsError::conflict(
                "Reset belongs to another installation",
            ));
        }
        if owner.state == "ready"
            && owner.reporting_epoch == reset_id
            && owner.reset_id.as_deref() == Some(reset_id)
        {
            return Ok(ResetReceipt {
                backup_spreadsheet_id: owner.backup_spreadsheet_id.ok_or_else(|| {
                    DirectSheetsError::conflict("Completed reset lost its backup receipt")
                })?,
                reporting_epoch: reset_id.into(),
                complete: true,
            });
        }
        if owner.state == "resetting" {
            if owner.reset_id.as_deref() != Some(reset_id) {
                return Err(DirectSheetsError::conflict(
                    "Resume the existing reset ID first",
                ));
            }
        } else {
            meta.owner_matches(
                writer_id,
                target.reporting_epoch.as_deref().unwrap_or("legacy"),
            )?;
            owner.state = "resetting".into();
            owner.reset_id = Some(reset_id.into());
            owner.backup_spreadsheet_id = None;
            owner.backup_fingerprint = None;
            self.save_owner(&meta, &owner).await?;
        }
        let backup = match owner.backup_spreadsheet_id.clone() {
            Some(id) => id,
            None => {
                let id = self.copy_workbook(&meta.spreadsheet_id, reset_id).await?;
                owner.backup_spreadsheet_id = Some(id.clone());
                self.save_owner(&meta, &owner).await?;
                id
            }
        };
        if let Some(fingerprint) = &owner.backup_fingerprint {
            if self.workbook_fingerprint(&backup).await? != *fingerprint {
                return Err(DirectSheetsError::conflict(
                    "Saved reset backup changed; clear remains paused",
                ));
            }
        } else {
            let verified = self.verify_backup(&meta.spreadsheet_id, &backup).await?;
            // Persist the verified backup BEFORE clear. A lost clear response
            // resumes against this saved backup, not the now-empty source.
            owner.backup_fingerprint = Some(verified.source_fingerprint);
            self.save_owner(&meta, &owner).await?;
        }
        let current = self.metadata(&meta.spreadsheet_id, 0).await?;
        if current.owner.as_ref().is_none_or(|o| {
            o.writer_id != writer_id
                || o.state != "resetting"
                || o.reset_id.as_deref() != Some(reset_id)
        }) {
            return Err(DirectSheetsError::conflict(
                "Reset owner changed before clear",
            ));
        }
        self.batch(&meta.spreadsheet_id,vec![json!({"updateCells":{"range":{"sheetId":0,"startRowIndex":1,"endRowIndex":meta.row_count,"startColumnIndex":0,"endColumnIndex":meta.column_count},"fields":"userEnteredValue,note"}})]).await?;
        for start in (1..meta.row_count).step_by(PAGE_ROWS as usize) {
            // Managed reports are at most128columns, but reset intentionally
            // clears every column of gid0 and verifies all those cells.
            let name = meta.title.replace('\'', "''");
            for left in (0..meta.column_count).step_by(MAX_COLUMNS as usize) {
                let range = format!(
                    "'{name}'!{}{}:{}{}",
                    column_letters(left + 1),
                    start + 1,
                    column_letters((left + MAX_COLUMNS).min(meta.column_count)),
                    (start + PAGE_ROWS).min(meta.row_count)
                );
                let value = self
                    .request(
                        reqwest::Method::GET,
                        &meta.spreadsheet_id,
                        &[
                            ("ranges", range),
                            (
                                "fields",
                                "sheets(data(rowData(values(userEnteredValue,note))))".into(),
                            ),
                        ],
                        None,
                    )
                    .await?;
                if has_entered_data(&value) {
                    return Err(DirectSheetsError::conflict(
                        "Cleared tab still contains values or notes",
                    ));
                }
            }
        }
        owner.reporting_epoch = reset_id.into();
        owner.state = "ready".into();
        self.save_owner(&meta, &owner).await?;
        let complete = self.metadata(&meta.spreadsheet_id, 0).await?;
        complete.owner_matches(writer_id, reset_id)?;
        Ok(ResetReceipt {
            backup_spreadsheet_id: backup,
            reporting_epoch: reset_id.into(),
            complete: true,
        })
    }
}

fn has_entered_data(value: &Value) -> bool {
    match value {
        Value::Object(o) => o.iter().any(|(key, v)| {
            (key == "userEnteredValue" && !v.is_null())
                || (key == "note" && v.as_str().is_some_and(|s| !s.is_empty()))
                || has_entered_data(v)
        }),
        Value::Array(rows) => rows.iter().any(has_entered_data),
        _ => false,
    }
}

fn stable_sheet_structure(mut value: Value) -> Value {
    match &mut value {
        Value::Object(map) => {
            map.remove("sheetId");
            for v in map.values_mut() {
                *v = stable_sheet_structure(v.take());
            }
        }
        Value::Array(rows) => {
            for v in rows {
                *v = stable_sheet_structure(v.take());
            }
        }
        _ => {}
    }
    value
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn backup_compares_formula_notes_formats_merges_and_rules_without_sheet_ids() {
        let a = json!({"properties":{"title":"Main","gridProperties":{"rowCount":20}},"merges":[{"sheetId":0,"startRowIndex":1}],"conditionalFormats":[{"ranges":[{"sheetId":0}],"booleanRule":{"condition":{"type":"NUMBER_GREATER"}}}]});
        let mut b = a.clone();
        b["merges"][0]["sheetId"] = json!(123);
        b["conditionalFormats"][0]["ranges"][0]["sheetId"] = json!(123);
        assert_eq!(
            stable_sheet_structure(a.clone()),
            stable_sheet_structure(b.clone())
        );
        b["merges"][0]["startRowIndex"] = json!(2);
        assert_ne!(stable_sheet_structure(a), stable_sheet_structure(b));
        for cell in [
            json!({"userEnteredValue":{"formulaValue":"=1+2"}}),
            json!({"note":"key"}),
        ] {
            assert!(has_entered_data(
                &json!({"sheets":[{"data":[{"rowData":[{"values":[cell]}]}]}]})
            ));
        }
        assert!(!has_entered_data(
            &json!({"sheets":[{"data":[{"rowData":[{"values":[{"userEnteredFormat":{"numberFormat":{"type":"DATE"}}}]}]}]}]})
        ));
    }
}
