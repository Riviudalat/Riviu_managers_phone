//! Observation only. A read proof never substitutes for final writer admission.
use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SheetWritePermission {
    Verified,
    Unknown,
    Denied,
}

impl DirectSheetsClient {
    pub async fn check_target_readonly(
        &self,
        spreadsheet_id: &str,
        gid: u64,
        oauth_scope: &str,
    ) -> Result<(DirectTargetCheck, SheetWritePermission)> {
        let mut check = self.check_target(spreadsheet_id, gid).await?;
        let can_read_rights = oauth_scope.split_whitespace().any(|scope| {
            matches!(
                scope,
                "https://www.googleapis.com/auth/drive"
                    | "https://www.googleapis.com/auth/drive.file"
                    | "https://www.googleapis.com/auth/drive.metadata"
                    | "https://www.googleapis.com/auth/drive.metadata.readonly"
                    | "https://www.googleapis.com/auth/drive.readonly"
            )
        });
        // spreadsheets scope alone cannot query Drive capabilities. Do not
        // request broader OAuth scopes or turn a successful GET into edit proof.
        let mut permission = SheetWritePermission::Unknown;
        if can_read_rights {
            let rights = self
                .request_service(
                    true,
                    reqwest::Method::GET,
                    spreadsheet_id,
                    &[
                        ("fields", "id,capabilities(canEdit)".into()),
                        ("supportsAllDrives", "true".into()),
                    ],
                    None,
                )
                .await;
            match rights {
                Ok(rights) => {
                    if rights["id"] != spreadsheet_id {
                        return Err(DirectSheetsError::conflict("Drive identity mismatch"));
                    }
                    permission = match rights["capabilities"]["canEdit"].as_bool() {
                        Some(true) => SheetWritePermission::Verified,
                        Some(false) => SheetWritePermission::Denied,
                        None => SheetWritePermission::Unknown,
                    };
                }
                // A Drive permission/API/scope rejection is not evidence of
                // denial to edit Sheets, which was successfully read above.
                Err(error)
                    if matches!(
                        error.kind,
                        DirectSheetsErrorKind::Forbidden | DirectSheetsErrorKind::NotFound
                    ) => {}
                Err(error) => return Err(error),
            }
        }
        let protections = self.request(reqwest::Method::GET, spreadsheet_id,
            &[("fields", "spreadsheetId,sheets(properties(sheetId),protectedRanges(warningOnly,requestingUserCanEdit))".into())], None).await?;
        if protections["spreadsheetId"] != spreadsheet_id {
            return Err(DirectSheetsError::conflict(
                "Sheet protection identity mismatch",
            ));
        }
        let sheets = protections["sheets"]
            .as_array()
            .ok_or_else(|| DirectSheetsError::invalid("Sheet protections missing"))?;
        let selected: Vec<_> = sheets
            .iter()
            .filter(|s| s["properties"]["sheetId"].as_u64() == Some(gid))
            .collect();
        let [sheet] = selected.as_slice() else {
            return Err(DirectSheetsError::conflict(
                "Sheet protection target mismatch",
            ));
        };
        for range in sheet["protectedRanges"].as_array().into_iter().flatten() {
            if range["warningOnly"].as_bool() == Some(true) {
                continue;
            }
            if range["requestingUserCanEdit"].as_bool() != Some(true) {
                // Conservatively require final writer validation where any
                // protected range may overlap reporting. Never call this denied
                // without knowing the exact rows the next write will touch.
                if permission != SheetWritePermission::Denied {
                    permission = SheetWritePermission::Unknown;
                }
            }
        }
        check.writable = permission == SheetWritePermission::Verified;
        Ok((check, permission))
    }
}
