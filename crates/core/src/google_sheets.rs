//! Direct Google Sheets transport. OAuth acquisition/refresh and durable claims
//! belong to the caller. A tab has one installation writer; Sheets has no CAS
//! against human edits, so every write verifies its exact row before and after.
use crate::publish_sheet::SheetDeliveryTarget;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{collections::BTreeSet, time::Duration};

mod planner;
mod reset;
use planner::{Cell, Layout, Row, Scan, WritePlan};
pub use reset::{BackupReceipt, ResetReceipt};
#[cfg(test)]
mod reset_http_tests;
#[cfg(test)]
mod tests;

pub const WRITER_METADATA_KEY: &str = "riviu.direct.writer.v1";
const PAGE_ROWS: u32 = 2_000;
const MAX_SCAN_ROWS: u32 = 100_000;
const MAX_COLUMNS: u32 = 128;
const BODY_LIMIT: usize = 8 * 1024 * 1024;
// Clients are recreated when tokens refresh. Keep serialization across clients
// and let the durable DB claims enforce the one-progress/two-delivery limits.
static WRITER_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
static REQUEST_START: tokio::sync::Mutex<Option<tokio::time::Instant>> =
    tokio::sync::Mutex::const_new(None);

async fn pace_requests(clock: &tokio::sync::Mutex<Option<tokio::time::Instant>>) {
    let mut last = clock.lock().await;
    if let Some(at) = *last {
        tokio::time::sleep_until(at + Duration::from_secs(1)).await;
    }
    *last = Some(tokio::time::Instant::now());
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum DirectSheetsErrorKind {
    Unauthorized,
    Forbidden,
    NotFound,
    RateLimited,
    Server,
    Transport,
    Conflict,
    Invalid,
}

#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub struct DirectSheetsError {
    pub kind: DirectSheetsErrorKind,
    pub status: Option<u16>,
    message: String,
}
impl DirectSheetsError {
    pub fn retryable(&self) -> bool {
        matches!(
            self.kind,
            DirectSheetsErrorKind::RateLimited
                | DirectSheetsErrorKind::Server
                | DirectSheetsErrorKind::Transport
        )
    }
    fn invalid(message: impl Into<String>) -> Self {
        Self {
            kind: DirectSheetsErrorKind::Invalid,
            status: None,
            message: message.into(),
        }
    }
    fn conflict(message: impl Into<String>) -> Self {
        Self {
            kind: DirectSheetsErrorKind::Conflict,
            status: None,
            message: message.into(),
        }
    }
    fn transport() -> Self {
        Self {
            kind: DirectSheetsErrorKind::Transport,
            status: None,
            message: "Google Sheets request did not complete; read the row before retrying".into(),
        }
    }
    fn http(status: u16) -> Self {
        let kind = match status {
            401 => DirectSheetsErrorKind::Unauthorized,
            403 => DirectSheetsErrorKind::Forbidden,
            404 => DirectSheetsErrorKind::NotFound,
            429 => DirectSheetsErrorKind::RateLimited,
            500..=599 => DirectSheetsErrorKind::Server,
            _ => DirectSheetsErrorKind::Invalid,
        };
        Self {
            kind,
            status: Some(status),
            message: format!("Google Sheets returned HTTP {status}"),
        }
    }
}
type Result<T> = std::result::Result<T, DirectSheetsError>;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DirectTargetCheck {
    pub spreadsheet_id: String,
    pub sheet_gid: u64,
    pub title: String,
    pub time_zone: String,
    pub columns: Vec<String>,
    pub layout: Option<String>,
    pub reporting_epoch: Option<String>,
    pub reporting_ready: bool,
    pub writer_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeliveryReceipt {
    pub publication_id: String,
    pub row: u32,
    pub revision: i64,
    pub reporting_epoch: String,
    pub post_url: String,
    pub duplicate: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DirectSheetTab {
    pub sheet_id: u64,
    pub title: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Owner {
    schema_version: u32,
    writer_id: String,
    reporting_epoch: String,
    state: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    reset_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    backup_spreadsheet_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    backup_fingerprint: Option<String>,
}

#[derive(Clone)]
struct Metadata {
    spreadsheet_id: String,
    gid: u64,
    title: String,
    time_zone: String,
    row_count: u32,
    column_count: u32,
    owner: Option<Owner>,
    header: Vec<Cell>,
}
impl Metadata {
    fn check(&self) -> DirectTargetCheck {
        let layout = Layout::from_header(&self.header).ok();
        DirectTargetCheck {
            spreadsheet_id: self.spreadsheet_id.clone(),
            sheet_gid: self.gid,
            title: self.title.clone(),
            time_zone: self.time_zone.clone(),
            columns: self.header.iter().map(Cell::display).collect(),
            layout: layout.map(|l| if l.internal { "internal" } else { "compact" }.into()),
            reporting_epoch: self.owner.as_ref().map(|o| o.reporting_epoch.clone()),
            reporting_ready: self.owner.as_ref().is_some_and(|o| o.state == "ready"),
            writer_id: self.owner.as_ref().map(|o| o.writer_id.clone()),
        }
    }
    fn owner_matches(&self, writer: &str, epoch: &str) -> Result<()> {
        let owner = self.owner.as_ref().ok_or_else(|| {
            DirectSheetsError::conflict("Tab has not been prepared for this installation")
        })?;
        if owner.writer_id != writer || owner.reporting_epoch != epoch || owner.state != "ready" {
            return Err(DirectSheetsError::conflict(
                "Tab writer or reporting epoch changed",
            ));
        }
        Ok(())
    }
}

pub struct DirectSheetsClient {
    token: String,
    http: reqwest::Client,
    #[cfg(test)]
    test_origin: Option<String>,
}

impl DirectSheetsClient {
    pub fn new(access_token: impl Into<String>) -> Result<Self> {
        let token = access_token.into();
        if token.is_empty() || token.len() > 8192 || token.chars().any(char::is_control) {
            return Err(DirectSheetsError::invalid(
                "Google access token is missing or invalid",
            ));
        }
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .connect_timeout(Duration::from_secs(7))
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .build()
            .map_err(|_| DirectSheetsError::transport())?;
        Ok(Self {
            token,
            http,
            #[cfg(test)]
            test_origin: None,
        })
    }

    async fn request(
        &self,
        method: reqwest::Method,
        path: &str,
        query: &[(&str, String)],
        body: Option<&Value>,
    ) -> Result<Value> {
        self.request_service(false, method, path, query, body).await
    }

    async fn request_service(
        &self,
        drive: bool,
        method: reqwest::Method,
        path: &str,
        query: &[(&str, String)],
        body: Option<&Value>,
    ) -> Result<Value> {
        #[cfg(not(test))]
        pace_requests(&REQUEST_START).await;
        #[cfg(test)]
        if self.test_origin.is_none() {
            pace_requests(&REQUEST_START).await;
        }
        let _permit = crate::publish_sheet::SHEET_HTTP_SLOTS
            .acquire()
            .await
            .map_err(|_| DirectSheetsError::transport())?;
        let origin = if drive {
            "https://drive.googleapis.com"
        } else {
            "https://sheets.googleapis.com"
        };
        #[cfg(test)]
        let origin = self.test_origin.as_deref().unwrap_or(origin);
        let mut request = self
            .http
            .request(
                method,
                if drive {
                    format!("{origin}/drive/v3/files/{path}")
                } else {
                    format!("{origin}/v4/spreadsheets/{path}")
                },
            )
            .bearer_auth(&self.token)
            .query(query);
        if let Some(body) = body {
            request = request.json(body);
        }
        let mut response = request
            .send()
            .await
            .map_err(|_| DirectSheetsError::transport())?;
        if !response.status().is_success() {
            return Err(DirectSheetsError::http(response.status().as_u16()));
        }
        if response
            .content_length()
            .is_some_and(|n| n > BODY_LIMIT as u64)
        {
            return Err(DirectSheetsError::invalid(
                "Google Sheets response exceeds bounded page size",
            ));
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| DirectSheetsError::transport())?
        {
            if bytes.len() + chunk.len() > BODY_LIMIT {
                return Err(DirectSheetsError::invalid(
                    "Google Sheets response exceeds bounded page size",
                ));
            }
            bytes.extend_from_slice(&chunk);
        }
        serde_json::from_slice(&bytes)
            .map_err(|_| DirectSheetsError::invalid("Google Sheets response is not JSON"))
    }

    async fn batch(&self, book: &str, requests: Vec<Value>) -> Result<()> {
        let response = self
            .request(
                reqwest::Method::POST,
                &format!("{book}:batchUpdate"),
                &[],
                Some(&json!({"requests":requests,"includeSpreadsheetInResponse":false})),
            )
            .await?;
        if response["spreadsheetId"] != book {
            return Err(DirectSheetsError::conflict(
                "Batch acknowledgement names another spreadsheet",
            ));
        }
        Ok(())
    }

    async fn metadata(&self, book: &str, gid: u64) -> Result<Metadata> {
        validate_target(book, gid)?;
        let value = self.request(reqwest::Method::GET,book,&[("fields","spreadsheetId,properties(timeZone),sheets(properties(sheetId,title,gridProperties(rowCount,columnCount)),developerMetadata),developerMetadata".into())],None).await?;
        if value["spreadsheetId"] != book {
            return Err(DirectSheetsError::conflict("Spreadsheet identity mismatch"));
        }
        let sheets = value["sheets"]
            .as_array()
            .ok_or_else(|| DirectSheetsError::invalid("Spreadsheet has no tabs"))?;
        let selected: Vec<_> = sheets
            .iter()
            .filter(|s| s["properties"]["sheetId"].as_u64() == Some(gid))
            .collect();
        let [sheet] = selected.as_slice() else {
            return Err(DirectSheetsError::invalid("Exact Sheet gid not found"));
        };
        let properties = &sheet["properties"];
        let row_count = properties["gridProperties"]["rowCount"]
            .as_u64()
            .filter(|n| *n > 0 && *n <= MAX_SCAN_ROWS as u64)
            .ok_or_else(|| {
                DirectSheetsError::invalid("Tab row count exceeds the bounded scan limit")
            })? as u32;
        let column_count = properties["gridProperties"]["columnCount"]
            .as_u64()
            .filter(|n| *n > 0 && *n <= 18_278)
            .ok_or_else(|| DirectSheetsError::invalid("Invalid tab column count"))?
            as u32;
        let title = properties["title"]
            .as_str()
            .ok_or_else(|| DirectSheetsError::invalid("Tab title missing"))?
            .to_owned();
        let mut owners = Vec::new();
        for entries in [
            value.get("developerMetadata"),
            sheet.get("developerMetadata"),
        ] {
            for entry in entries.and_then(Value::as_array).into_iter().flatten() {
                if entry["metadataKey"] == WRITER_METADATA_KEY
                    && entry["location"]["sheetId"].as_u64() == Some(gid)
                {
                    if entry["metadataId"].as_u64() != Some(writer_metadata_id(gid) as u64) {
                        return Err(DirectSheetsError::conflict(
                            "Writer metadata ID differs from this tab",
                        ));
                    }
                    let raw = entry["metadataValue"]
                        .as_str()
                        .ok_or_else(|| DirectSheetsError::conflict("Invalid writer metadata"))?;
                    if !owners.iter().any(|v: &String| v == raw) {
                        owners.push(raw.to_owned());
                    }
                }
            }
        }
        if owners.len() > 1 {
            return Err(DirectSheetsError::conflict("Multiple writer owners found"));
        }
        let owner = owners
            .first()
            .map(|raw| {
                serde_json::from_str::<Owner>(raw)
                    .map_err(|_| DirectSheetsError::conflict("Invalid writer metadata"))
            })
            .transpose()?;
        if owner.as_ref().is_some_and(|o| {
            o.schema_version != 1
                || uuid::Uuid::parse_str(&o.writer_id).is_err()
                || o.reporting_epoch.is_empty()
        }) {
            return Err(DirectSheetsError::conflict("Unsupported writer metadata"));
        }
        let mut result = Metadata {
            spreadsheet_id: book.into(),
            gid,
            title,
            time_zone: value["properties"]["timeZone"]
                .as_str()
                .unwrap_or("UTC")
                .into(),
            row_count,
            column_count,
            owner,
            header: vec![],
        };
        result.header = self
            .rows(&result, 0, 1, column_count.min(MAX_COLUMNS))
            .await?
            .remove(0)
            .cells;
        Ok(result)
    }

    async fn rows(&self, meta: &Metadata, start: u32, count: u32, width: u32) -> Result<Vec<Row>> {
        let name = meta.title.replace('\'', "''");
        let range = format!(
            "'{name}'!A{}:{}{}",
            start + 1,
            column_letters(width),
            start + count
        );
        let value=self.request(reqwest::Method::GET,&meta.spreadsheet_id,&[("ranges",range),("fields","spreadsheetId,sheets(properties(sheetId),data(startRow,startColumn,rowData(values(formattedValue,userEnteredValue,note))))".into())],None).await?;
        if value["spreadsheetId"] != meta.spreadsheet_id {
            return Err(DirectSheetsError::conflict("Readback spreadsheet mismatch"));
        }
        let selected = value["sheets"]
            .as_array()
            .ok_or_else(|| DirectSheetsError::invalid("Missing readback tab"))?;
        if selected.len() != 1 || selected[0]["properties"]["sheetId"].as_u64() != Some(meta.gid) {
            return Err(DirectSheetsError::conflict("Readback tab mismatch"));
        }
        let mut rows: Vec<_> = (start..start + count)
            .map(|index| Row {
                index,
                cells: vec![Cell::default(); width as usize],
            })
            .collect();
        for grid in selected[0]["data"].as_array().into_iter().flatten() {
            let top = grid["startRow"].as_u64().unwrap_or(0) as u32;
            let left = grid["startColumn"].as_u64().unwrap_or(0) as usize;
            for (y, row) in grid["rowData"].as_array().into_iter().flatten().enumerate() {
                if top + (y as u32) < start || top + (y as u32) >= start + count {
                    continue;
                }
                for (x, cell) in row["values"].as_array().into_iter().flatten().enumerate() {
                    if left + x < width as usize {
                        rows[(top + y as u32 - start) as usize].cells[left + x] =
                            serde_json::from_value(cell.clone())
                                .map_err(|_| DirectSheetsError::invalid("Malformed Sheet cell"))?;
                    }
                }
            }
        }
        Ok(rows)
    }

    async fn scan(&self, meta: &Metadata, width: usize, payload: Option<&Value>) -> Result<Scan> {
        let mut scan = Scan::default();
        let page_rows = PAGE_ROWS.min((16_000 / width.max(1)) as u32).max(1);
        for start in (1..meta.row_count).step_by(page_rows as usize) {
            for row in self
                .rows(
                    meta,
                    start,
                    page_rows.min(meta.row_count - start),
                    width as u32,
                )
                .await?
            {
                scan.observe(row, payload)?;
            }
        }
        Ok(scan)
    }

    pub async fn check_target(&self, spreadsheet_id: &str, gid: u64) -> Result<DirectTargetCheck> {
        Ok(self.metadata(spreadsheet_id, gid).await?.check())
    }

    pub async fn list_tabs(&self, spreadsheet_id: &str) -> Result<Vec<DirectSheetTab>> {
        validate_target(spreadsheet_id, 0)?;
        let value = self
            .request(
                reqwest::Method::GET,
                spreadsheet_id,
                &[(
                    "fields",
                    "spreadsheetId,sheets(properties(sheetId,title))".into(),
                )],
                None,
            )
            .await?;
        if value["spreadsheetId"] != spreadsheet_id {
            return Err(DirectSheetsError::conflict("Spreadsheet identity mismatch"));
        }
        let tabs = value["sheets"]
            .as_array()
            .ok_or_else(|| DirectSheetsError::invalid("Spreadsheet tabs missing"))?;
        tabs.iter()
            .map(|tab| {
                let sheet_id = tab["properties"]["sheetId"]
                    .as_u64()
                    .ok_or_else(|| DirectSheetsError::invalid("Tab ID missing"))?;
                let title = tab["properties"]["title"]
                    .as_str()
                    .ok_or_else(|| DirectSheetsError::invalid("Tab title missing"))?
                    .to_owned();
                Ok(DirectSheetTab { sheet_id, title })
            })
            .collect()
    }

    /// Explicit enrollment only, after the caller has retired any legacy writer.
    pub async fn prepare_target(
        &self,
        target: &SheetDeliveryTarget,
        writer_id: &str,
    ) -> Result<DirectTargetCheck> {
        tokio::time::timeout(
            Duration::from_secs(90),
            self.prepare_inner(target, writer_id),
        )
        .await
        .map_err(|_| DirectSheetsError::transport())?
    }

    async fn prepare_inner(
        &self,
        target: &SheetDeliveryTarget,
        writer_id: &str,
    ) -> Result<DirectTargetCheck> {
        let _guard = WRITER_LOCK.lock().await;
        uuid::Uuid::parse_str(writer_id)
            .map_err(|_| DirectSheetsError::invalid("Installation writer ID must be UUID"))?;
        target
            .validate()
            .map_err(|_| DirectSheetsError::invalid("Invalid Sheet target"))?;
        let meta = self
            .metadata(&target.spreadsheet_id, target.sheet_gid)
            .await?;
        if let Some(owner) = &meta.owner {
            if owner.writer_id != writer_id
                || target
                    .reporting_epoch
                    .as_ref()
                    .is_some_and(|e| e != &owner.reporting_epoch)
                || owner.state != "ready"
            {
                return Err(DirectSheetsError::conflict(
                    "Tab already belongs to another writer or epoch",
                ));
            }
            Layout::from_header(&meta.header)?;
            return Ok(meta.check());
        }
        let empty = meta.header.iter().all(Cell::empty);
        if empty && meta.column_count > MAX_COLUMNS {
            return Err(DirectSheetsError::invalid(
                "Blank-header preparation requires at most128 inspected columns",
            ));
        }
        let layout = if empty {
            Layout::standard(target.internal_reporting)
        } else {
            Layout::from_header(&meta.header)?
        };
        if layout.internal != target.internal_reporting {
            return Err(DirectSheetsError::conflict(
                "Configured reporting layout differs from the tab",
            ));
        }
        let scan = self
            .scan(
                &meta,
                if empty {
                    meta.column_count.min(MAX_COLUMNS) as usize
                } else {
                    layout.width
                },
                None,
            )
            .await?;
        if empty && scan.last_nonempty.is_some() {
            return Err(DirectSheetsError::conflict(
                "Blank header has existing data below it",
            ));
        }
        if scan.epochs.len() > 1 {
            return Err(DirectSheetsError::conflict(
                "Rows belong to multiple reporting epochs",
            ));
        }
        let epoch = target
            .reporting_epoch
            .clone()
            .or_else(|| scan.epochs.iter().next().cloned())
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        if scan.epochs.iter().any(|old| old != &epoch) {
            return Err(DirectSheetsError::conflict(
                "Existing row epoch differs from selected target",
            ));
        }
        let owner = Owner {
            schema_version: 1,
            writer_id: writer_id.into(),
            reporting_epoch: epoch,
            state: "ready".into(),
            reset_id: None,
            backup_spreadsheet_id: None,
            backup_fingerprint: None,
        };
        let mut requests = vec![
            json!({"createDeveloperMetadata":{"developerMetadata":{"metadataId":writer_metadata_id(meta.gid),"metadataKey":WRITER_METADATA_KEY,"metadataValue":serde_json::to_string(&owner).expect("owner serializes"),"location":{"sheetId":meta.gid},"visibility":"DOCUMENT"}}}),
        ];
        if empty {
            if meta.column_count < layout.width as u32 {
                requests.push(json!({"appendDimension":{"sheetId":meta.gid,"dimension":"COLUMNS","length":layout.width as u32-meta.column_count}}));
            }
            requests.push(json!({"updateCells":{"start":{"sheetId":meta.gid,"rowIndex":0,"columnIndex":0},"rows":[{"values":layout.headers.iter().map(|v|json!({"userEnteredValue":{"stringValue":v}})).collect::<Vec<_>>()}],"fields":"userEnteredValue"}}));
        }
        self.batch(&meta.spreadsheet_id, requests).await?;
        let checked = self.metadata(&meta.spreadsheet_id, meta.gid).await?;
        checked.owner_matches(writer_id, &owner.reporting_epoch)?;
        Layout::from_header(&checked.header)?;
        Ok(checked.check())
    }

    pub async fn deliver(
        &self,
        target: &SheetDeliveryTarget,
        payload: &Value,
        writer_id: &str,
    ) -> Result<DeliveryReceipt> {
        tokio::time::timeout(
            Duration::from_secs(90),
            self.deliver_inner(target, payload, writer_id),
        )
        .await
        .map_err(|_| DirectSheetsError::transport())?
    }

    async fn deliver_inner(
        &self,
        target: &SheetDeliveryTarget,
        payload: &Value,
        writer_id: &str,
    ) -> Result<DeliveryReceipt> {
        let _guard = WRITER_LOCK.lock().await;
        target
            .validate()
            .map_err(|_| DirectSheetsError::invalid("Invalid Sheet target"))?;
        let meta = self
            .metadata(&target.spreadsheet_id, target.sheet_gid)
            .await?;
        let epoch = target.reporting_epoch.as_deref().unwrap_or("legacy");
        meta.owner_matches(writer_id, epoch)?;
        let layout = Layout::from_header(&meta.header)?;
        if layout.internal != target.internal_reporting {
            return Err(DirectSheetsError::conflict(
                "Pinned reporting layout changed",
            ));
        }
        planner::validate_payload(payload, target, &layout)?;
        let scan = self.scan(&meta, layout.width, Some(payload)).await?;
        let plan = planner::plan(&meta, &layout, &scan, payload)?;
        // Reopen owner/header and the exact selected row before the effect. This
        // catches edits between scan and write; it is not a Google Sheets CAS.
        let before = self.metadata(&meta.spreadsheet_id, meta.gid).await?;
        before.owner_matches(writer_id, epoch)?;
        if before.header != meta.header {
            return Err(DirectSheetsError::conflict(
                "Header changed during row planning",
            ));
        }
        if plan.row < meta.row_count {
            let actual = self
                .rows(&before, plan.row, 1, layout.width as u32)
                .await?
                .remove(0);
            if actual.cells != plan.before.cells {
                return Err(DirectSheetsError::conflict(
                    "Row moved or changed before writing",
                ));
            }
        }
        if !plan.requests.is_empty() {
            self.batch(&meta.spreadsheet_id, plan.requests.clone())
                .await?;
        }
        self.read_receipt(&meta, payload, &plan).await
    }

    async fn read_receipt(
        &self,
        meta: &Metadata,
        payload: &Value,
        plan: &WritePlan,
    ) -> Result<DeliveryReceipt> {
        let actual = self
            .rows(meta, plan.row, 1, plan.width as u32)
            .await?
            .remove(0);
        planner::receipt(&actual, payload, plan.duplicate)
    }
}

pub fn writer_metadata_id(gid: u64) -> i32 {
    let hash = Sha256::digest(format!("{WRITER_METADATA_KEY}:{gid}").as_bytes());
    (i32::from_be_bytes([hash[0], hash[1], hash[2], hash[3]]) & 0x7fff_ffff).max(1)
}
fn validate_target(book: &str, gid: u64) -> Result<()> {
    if book.is_empty()
        || book.len() > 128
        || !book
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
        || gid > i32::MAX as u64
    {
        return Err(DirectSheetsError::invalid("Invalid spreadsheet ID or gid"));
    }
    Ok(())
}
fn column_letters(mut value: u32) -> String {
    let mut letters = Vec::new();
    while value > 0 {
        value -= 1;
        letters.push((b'A' + (value % 26) as u8) as char);
        value /= 26;
    }
    letters.into_iter().rev().collect()
}
