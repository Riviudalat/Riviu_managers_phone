//! Bounded native data connectors. The Flow attempt ledger owns intent and retries;
//! these functions dispatch once and return a read-back receipt, never schedule work.
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::db::Database;

pub const MAX_CONNECTOR_BYTES: usize = 16_384;
pub const MAX_CONNECTOR_CHARS: usize = 4096;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum FileDataFormat {
    Text,
    Json,
    Csv,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FileReadConfig {
    pub name: String,
    pub path: String,
    pub format: FileDataFormat,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FileWriteConfig {
    pub name: String,
    pub path: String,
    pub format: FileDataFormat,
    pub value: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum HttpMethod {
    GET,
    POST,
    PUT,
    PATCH,
    DELETE,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HttpRequestConfig {
    pub name: String,
    pub url: String,
    pub method: HttpMethod,
    #[serde(default)]
    pub body: Option<String>,
    #[serde(default)]
    pub secret_ref: Option<String>,
    pub timeout_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SheetReadConfig {
    pub name: String,
    pub spreadsheet_url: String,
    pub tab: String,
    pub range: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SheetWriteConfig {
    pub name: String,
    pub spreadsheet_url: String,
    pub tab: String,
    pub range: String,
    pub values: String,
}

#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub struct ConnectorError {
    pub code: &'static str,
    pub message: String,
    /// True after a write/request starts, including a lost or malformed response.
    pub may_have_applied: bool,
}

impl ConnectorError {
    fn new(code: &'static str, message: impl Into<String>, applied: bool) -> Self {
        Self {
            code,
            message: message.into(),
            may_have_applied: applied,
        }
    }
    fn validation(message: impl Into<String>) -> Self {
        Self::new("ConnectorConfig", message, false)
    }
}

fn validate_name(name: &str) -> Result<(), &'static str> {
    super::validate_flow_variable_name(name)
}

pub fn validate_relative_path(path: &str) -> Result<(), &'static str> {
    if path.is_empty()
        || path.len() > 240
        || path.contains([':', '\0', '\\'])
        || path.starts_with('/')
    {
        return Err("Use a relative path with forward slashes inside flow-data");
    }
    if path
        .split('/')
        .any(|part| part.is_empty() || part == "." || part == "..")
    {
        return Err("File path must not contain empty or relative segments");
    }
    for part in Path::new(path).components() {
        let Component::Normal(name) = part else {
            return Err("File path cannot escape flow-data");
        };
        let name = name.to_str().ok_or("Invalid file name")?;
        let stem = name.split('.').next().unwrap_or("").to_ascii_uppercase();
        if name.to_ascii_lowercase().starts_with(".riviu-")
            || name.ends_with([' ', '.'])
            || name.contains(['<', '>', '"', '|', '?', '*'])
            || name.chars().any(char::is_control)
            || matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
            || (stem.len() == 4
                && (stem.starts_with("COM") || stem.starts_with("LPT"))
                && stem.as_bytes()[3].is_ascii_digit())
        {
            return Err("File name is not portable");
        }
    }
    Ok(())
}

impl FileReadConfig {
    pub fn validate_template(&self) -> Result<(), &'static str> {
        let mut config = self.clone();
        config.path = template_sample(&config.path, "fixture", "fixture")?;
        config.validate()
    }
    pub fn validate(&self) -> Result<(), &'static str> {
        validate_name(&self.name)?;
        validate_relative_path(&self.path)
    }
}
impl FileWriteConfig {
    pub fn validate_template(&self) -> Result<(), &'static str> {
        let mut config = self.clone();
        config.path = template_sample(&config.path, "fixture", "fixture")?;
        template_sample(&config.value, "fixture", "fixture")?;
        config.validate()
    }
    pub fn validate(&self) -> Result<(), &'static str> {
        validate_name(&self.name)?;
        validate_relative_path(&self.path)?;
        if self.value.chars().count() > MAX_CONNECTOR_CHARS {
            return Err("File value exceeds 4096 characters");
        }
        Ok(())
    }
}
impl HttpRequestConfig {
    pub fn validate_template(&self) -> Result<(), &'static str> {
        let mut config = self.clone();
        config.url = template_sample(&config.url, "https://example.invalid", "fixture")?;
        if let Some(body) = &config.body {
            template_sample(body, "fixture", "fixture")?;
        }
        config.validate()
    }
    pub fn validate(&self) -> Result<(), &'static str> {
        validate_name(&self.name)?;
        let url = url::Url::parse(&self.url).map_err(|_| "Invalid HTTP URL")?;
        let loopback = matches!(url.host_str(), Some("127.0.0.1" | "localhost" | "[::1]"));
        if !(url.scheme() == "https" || (url.scheme() == "http" && loopback))
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.fragment().is_some()
            || self.url.len() > 2048
        {
            return Err("HTTP connector requires HTTPS, or an explicit localhost HTTP fixture; URL credentials/fragments are excluded");
        }
        if self.timeout_ms < 100 || self.timeout_ms > 30_000 {
            return Err("HTTP timeout must be 100..30000 ms");
        }
        if self
            .body
            .as_ref()
            .is_some_and(|v| v.chars().count() > MAX_CONNECTOR_CHARS)
        {
            return Err("HTTP body exceeds 4096 characters");
        }
        if self.method == HttpMethod::GET && self.body.as_ref().is_some_and(|v| !v.is_empty()) {
            return Err("GET requests have no body");
        }
        if let Some(reference) = &self.secret_ref {
            validate_name(reference)?;
        }
        Ok(())
    }
}

/// A finite rectangle; whole rows/columns, named ranges and another tab are excluded.
pub fn parse_sheet_range(range: &str) -> Result<(u32, u32, u32, u32), &'static str> {
    fn cell(value: &str) -> Option<(u32, u32)> {
        let split = value.find(|c: char| c.is_ascii_digit())?;
        let (column, row) = value.split_at(split);
        if column.is_empty()
            || column.len() > 3
            || !column.chars().all(|c| c.is_ascii_uppercase())
            || row.starts_with('0')
        {
            return None;
        }
        let col = column
            .bytes()
            .fold(0u32, |sum, b| sum * 26 + u32::from(b - b'A' + 1));
        let row = row.parse::<u32>().ok()?;
        (row > 0 && row <= 1_000_000 && col <= 18_278).then_some((row, col))
    }
    let mut parts = range.split(':');
    let start = cell(parts.next().ok_or("Invalid A1 range")?).ok_or("Invalid A1 range")?;
    let end = match parts.next() {
        Some(value) => cell(value).ok_or("Invalid A1 range")?,
        None => start,
    };
    if parts.next().is_some() || end.0 < start.0 || end.1 < start.1 {
        return Err("Invalid A1 rectangle");
    }
    let height = end.0 - start.0 + 1;
    let width = end.1 - start.1 + 1;
    if u64::from(height) * u64::from(width) > 1000 {
        return Err("Sheet range exceeds 1000 cells");
    }
    Ok((start.0, start.1, height, width))
}

pub fn spreadsheet_id(value: &str) -> Result<String, &'static str> {
    let parsed = url::Url::parse(value).map_err(|_| "Invalid Google Sheet URL")?;
    let pieces: Vec<_> = parsed.path().split('/').collect();
    if parsed.scheme() != "https"
        || parsed.host_str() != Some("docs.google.com")
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || pieces.len() < 4
        || pieces[1] != "spreadsheets"
        || pieces[2] != "d"
    {
        return Err("Use the full Google Sheet URL");
    }
    let id = pieces[3];
    if id.is_empty()
        || id.len() > 128
        || !id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return Err("Invalid spreadsheet ID");
    }
    Ok(id.into())
}

fn validate_sheet(name: &str, url: &str, tab: &str, range: &str) -> Result<(), &'static str> {
    validate_name(name)?;
    spreadsheet_id(url)?;
    parse_sheet_range(range)?;
    if tab.trim().is_empty() || tab.len() > 100 || tab.chars().any(char::is_control) {
        return Err("Sheet tab is required");
    }
    Ok(())
}
impl SheetReadConfig {
    pub fn validate_template(&self) -> Result<(), &'static str> {
        let mut config = self.clone();
        config.spreadsheet_url = template_sample(
            &config.spreadsheet_url,
            "https://docs.google.com/spreadsheets/d/fixture/edit",
            "fixture",
        )?;
        config.tab = template_sample(&config.tab, "Fixture", "Fixture")?;
        if config.range.contains("${") {
            template_sample(&config.range, "A1", "A1")?;
            config.range = "A1".into();
        }
        config.validate()
    }
    pub fn validate(&self) -> Result<(), &'static str> {
        validate_sheet(&self.name, &self.spreadsheet_url, &self.tab, &self.range)
    }
}
impl SheetWriteConfig {
    pub fn validate_template(&self) -> Result<(), &'static str> {
        SheetReadConfig {
            name: self.name.clone(),
            spreadsheet_url: self.spreadsheet_url.clone(),
            tab: self.tab.clone(),
            range: self.range.clone(),
        }
        .validate_template()?;
        template_sample(&self.values, "fixture", "fixture")?;
        if self.values.chars().count() > MAX_CONNECTOR_CHARS {
            return Err("Sheet values exceed 4096 characters");
        }
        Ok(())
    }
    pub fn validate(&self) -> Result<(), &'static str> {
        validate_sheet(&self.name, &self.spreadsheet_url, &self.tab, &self.range)?;
        if self.values.chars().count() > MAX_CONNECTOR_CHARS {
            return Err("Sheet values exceed 4096 characters");
        }
        Ok(())
    }
}

/// Compile-time substitution validates syntax without pretending that a future
/// variable is known. The executor validates each complete resolved config again.
fn template_sample(template: &str, first: &str, rest_value: &str) -> Result<String, &'static str> {
    let mut remaining = template;
    let mut output = String::new();
    while let Some((literal, suffix)) = remaining.split_once("${") {
        output.push_str(literal);
        let (name, after) = suffix
            .split_once('}')
            .ok_or("Unterminated connector variable reference")?;
        validate_name(name)?;
        output.push_str(if output.is_empty() { first } else { rest_value });
        remaining = after;
    }
    output.push_str(remaining);
    Ok(output)
}

fn value_result(
    operation: &str,
    name: &str,
    value: String,
    receipt: String,
    applied: bool,
) -> Result<Value, ConnectorError> {
    if value.chars().count() > MAX_CONNECTOR_CHARS {
        return Err(ConnectorError::new(
            "ConnectorOutputLimit",
            "Connector output exceeds 4096 characters",
            applied,
        ));
    }
    Ok(
        json!({"kind":"flowConnector","operation":operation,"name":name,"value":value,"receipt":receipt}),
    )
}
fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

/// Revalidate stored output before a resumed Flow consumes its value. Remote
/// responses never provide this receipt: each connector calculates it locally.
pub fn validate_output_receipt(
    config: &super::CompiledActionConfig,
    value: &str,
    receipt: &str,
) -> bool {
    let valid_hash = |text: &str| {
        text.len() == 64
            && text
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    };
    use super::CompiledActionConfig as C;
    match config {
        C::FileRead(c) => {
            valid_hash(receipt)
                && match c.format {
                    FileDataFormat::Text => receipt == hash(value.as_bytes()),
                    FileDataFormat::Json => serde_json::from_str::<Value>(value).is_ok(),
                    FileDataFormat::Csv => parse_table(value).is_ok(),
                }
        }
        C::FileWrite(c) => {
            encode_data(c.format, value).is_ok_and(|encoded| receipt == hash(encoded.as_bytes()))
        }
        C::HttpRequest(_) => receipt
            .strip_prefix("HTTP ")
            .and_then(|r| r.split_once("; sha256="))
            .is_some_and(|(status, digest)| {
                status
                    .parse::<u16>()
                    .is_ok_and(|status| (200..300).contains(&status))
                    && digest == hash(value.as_bytes())
            }),
        C::SheetRead(_) | C::SheetWrite(_) => {
            parse_table(value).is_ok() && receipt == hash(value.as_bytes())
        }
        _ => false,
    }
}

#[derive(Clone)]
pub struct FlowConnectorExecutor {
    root: PathBuf,
    database: Arc<Database>,
}

impl FlowConnectorExecutor {
    pub fn new(root: PathBuf, database: Arc<Database>) -> Result<Self, ConnectorError> {
        if root.exists() {
            reject_link(&root)?;
        }
        std::fs::create_dir_all(&root)
            .map_err(|_| ConnectorError::validation("Cannot create flow-data folder"))?;
        let root = root
            .canonicalize()
            .map_err(|_| ConnectorError::validation("Cannot resolve flow-data folder"))?;
        Ok(Self { root, database })
    }

    pub async fn file_read(&self, config: &FileReadConfig) -> Result<Value, ConnectorError> {
        config.validate().map_err(ConnectorError::validation)?;
        // A bounded local section has no detached worker that can outlive the
        // enclosing node's cancellation and release its evidence afterwards.
        {
            let path = confined_path(&self.root, &config.path, false)?;
            let _lock = lock_file(&self.root, &config.path)?;
            let bytes = read_file(&path)?;
            let text = String::from_utf8(bytes.clone())
                .map_err(|_| ConnectorError::validation("File is not UTF-8"))?;
            let value = decode_data(config.format, &text)?;
            value_result("fileRead", &config.name, value, hash(&bytes), false)
        }
    }

    pub async fn file_write(&self, config: &FileWriteConfig) -> Result<Value, ConnectorError> {
        config.validate().map_err(ConnectorError::validation)?;
        {
            let text = encode_data(config.format, &config.value)?;
            if text.len() > MAX_CONNECTOR_BYTES {
                return Err(ConnectorError::validation("Encoded file exceeds 16 KiB"));
            }
            let path = confined_path(&self.root, &config.path, true)?;
            let _lock = lock_file(&self.root, &config.path)?;
            let parent = path
                .parent()
                .ok_or_else(|| ConnectorError::validation("File has no parent"))?;
            let temp = parent.join(format!(".riviu-{}.tmp", uuid::Uuid::new_v4()));
            let result = (|| {
                let mut file = std::fs::OpenOptions::new()
                    .create_new(true)
                    .write(true)
                    .open(&temp)
                    .map_err(|_| {
                        ConnectorError::new("ConnectorFile", "Cannot prepare file write", false)
                    })?;
                file.write_all(text.as_bytes())
                    .and_then(|_| file.sync_all())
                    .map_err(|_| {
                        ConnectorError::new("ConnectorFile", "Cannot persist prepared file", false)
                    })?;
                drop(file);
                // Recheck existing path components before the atomic replacement.
                confined_path(&self.root, &config.path, true)?;
                std::fs::rename(&temp, &path).map_err(|_| {
                    ConnectorError::new("ConnectorFile", "File replacement failed", true)
                })?;
                let bytes = read_file(&path).map_err(|mut e| {
                    e.may_have_applied = true;
                    e
                })?;
                if bytes != text.as_bytes() {
                    return Err(ConnectorError::new(
                        "ConnectorReadback",
                        "File changed before readback",
                        true,
                    ));
                }
                value_result(
                    "fileWrite",
                    &config.name,
                    decode_data(config.format, &text)?,
                    hash(&bytes),
                    true,
                )
            })();
            if temp.exists() {
                let _ = std::fs::remove_file(temp);
            }
            result
        }
    }

    pub async fn http_request(&self, config: &HttpRequestConfig) -> Result<Value, ConnectorError> {
        config.validate().map_err(ConnectorError::validation)?;
        let method = match config.method {
            HttpMethod::GET => reqwest::Method::GET,
            HttpMethod::POST => reqwest::Method::POST,
            HttpMethod::PUT => reqwest::Method::PUT,
            HttpMethod::PATCH => reqwest::Method::PATCH,
            HttpMethod::DELETE => reqwest::Method::DELETE,
        };
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .timeout(Duration::from_millis(config.timeout_ms))
            .build()
            .map_err(|_| ConnectorError::validation("HTTP client initialization failed"))?;
        let mut request = client
            .request(method, &config.url)
            .header(reqwest::header::ACCEPT, "application/json, text/plain");
        let mut used_secret = None;
        if let Some(reference) = &config.secret_ref {
            let secret = self
                .database
                .flow_connector_secret(reference)
                .map_err(|_| ConnectorError::validation("Cannot read connector credential"))?
                .filter(|v| !v.is_empty())
                .ok_or_else(|| {
                    ConnectorError::validation("Connector credential has not been configured")
                })?;
            request = request.bearer_auth(&secret);
            used_secret = Some(secret);
        }
        if let Some(body) = &config.body {
            request = request
                .header(reqwest::header::CONTENT_TYPE, "application/json")
                .body(body.clone());
        }
        let response = request.send().await.map_err(|_| ConnectorError::new("ConnectorHttp", "HTTP request ended without a response; inspect the remote system before another run", true))?;
        let status = response.status();
        if !status.is_success() {
            return Err(ConnectorError::new(
                "ConnectorHttpStatus",
                format!("HTTP returned {status}; redirects are not followed"),
                true,
            ));
        }
        let bytes = limited_response(response).await?;
        let value = String::from_utf8(bytes.clone()).map_err(|_| {
            ConnectorError::new("ConnectorEncoding", "HTTP response is not UTF-8", true)
        })?;
        if used_secret
            .as_ref()
            .is_some_and(|secret| value.contains(secret))
        {
            return Err(ConnectorError::new("ConnectorCredentialEcho", "HTTP endpoint echoed its credential; the response was excluded from the Flow ledger", true));
        }
        value_result(
            "httpRequest",
            &config.name,
            value,
            format!("HTTP {}; sha256={}", status.as_u16(), hash(&bytes)),
            true,
        )
    }

    pub async fn sheet_read(&self, config: &SheetReadConfig) -> Result<Value, ConnectorError> {
        config.validate().map_err(ConnectorError::validation)?;
        self.sheet_request(
            "sheetRead",
            &config.name,
            &config.spreadsheet_url,
            &config.tab,
            &config.range,
            None,
        )
        .await
    }
    pub async fn sheet_write(&self, config: &SheetWriteConfig) -> Result<Value, ConnectorError> {
        config.validate().map_err(ConnectorError::validation)?;
        let values = parse_table(&config.values)?;
        let (_, _, height, width) =
            parse_sheet_range(&config.range).map_err(ConnectorError::validation)?;
        if values.len() != height as usize || values.iter().any(|row| row.len() != width as usize) {
            return Err(ConnectorError::validation(
                "Sheet values must exactly fill the requested rectangle",
            ));
        }
        self.sheet_request(
            "sheetWrite",
            &config.name,
            &config.spreadsheet_url,
            &config.tab,
            &config.range,
            Some(values),
        )
        .await
    }
    async fn sheet_request(
        &self,
        operation: &str,
        name: &str,
        url: &str,
        tab: &str,
        range: &str,
        values: Option<Vec<Vec<String>>>,
    ) -> Result<Value, ConnectorError> {
        let settings = self
            .database
            .publish_sheet_delivery_settings()
            .map_err(|_| ConnectorError::validation("Cannot read Sheet connection"))?;
        if !crate::publish_sheet::is_acceptable_webhook(&settings.webhook_url)
            || settings.token.is_empty()
        {
            return Err(ConnectorError::validation(
                "Configure the existing Sheet connection before using this node",
            ));
        }
        let id = spreadsheet_id(url).map_err(ConnectorError::validation)?;
        let payload = json!({"token":settings.token,"rowKind":if values.is_some(){"flowWrite"}else{"flowRead"},"connectorVersion":1,"spreadsheetId":id,"tab":tab,"range":range,"values":values});
        let applied = values.is_some();
        let ack = crate::publish_sheet::flow_connector_request(&settings.webhook_url,&payload).await
            .map_err(|_| ConnectorError::new("ConnectorSheet", "Sheet connector did not return a valid acknowledgement; update the bundled Apps Script deployment and inspect the target range", applied))?;
        if ack["ok"] != true
            || ack["connectorVersion"] != 1
            || ack["spreadsheetId"] != id
            || ack["tab"] != tab
            || ack["range"] != range
        {
            return Err(ConnectorError::new(
                "ConnectorSheetBinding",
                "Sheet reply does not match the requested table and range",
                applied,
            ));
        }
        let returned: Vec<Vec<String>> =
            serde_json::from_value(ack["values"].clone()).map_err(|_| {
                ConnectorError::new(
                    "ConnectorSheetReadback",
                    "Sheet returned an invalid table",
                    applied,
                )
            })?;
        let (_, _, height, width) = parse_sheet_range(range).map_err(ConnectorError::validation)?;
        if returned.len() != height as usize
            || returned.iter().any(|row| row.len() != width as usize)
            || values
                .as_ref()
                .is_some_and(|expected| *expected != returned)
        {
            return Err(ConnectorError::new(
                "ConnectorSheetReadback",
                "Sheet readback differs from the requested values",
                applied,
            ));
        }
        let value = serde_json::to_string(&returned).map_err(|_| {
            ConnectorError::new("ConnectorEncoding", "Cannot encode Sheet values", applied)
        })?;
        let receipt = hash(value.as_bytes());
        value_result(operation, name, value, receipt, applied)
    }
}

async fn limited_response(mut response: reqwest::Response) -> Result<Vec<u8>, ConnectorError> {
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| {
        ConnectorError::new("ConnectorHttpRead", "HTTP response was interrupted", true)
    })? {
        if bytes.len() + chunk.len() > MAX_CONNECTOR_BYTES {
            return Err(ConnectorError::new(
                "ConnectorOutputLimit",
                "HTTP response exceeds 16 KiB",
                true,
            ));
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

fn reject_link(path: &Path) -> Result<(), ConnectorError> {
    let metadata = std::fs::symlink_metadata(path)
        .map_err(|_| ConnectorError::validation("Cannot inspect file path"))?;
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return Err(ConnectorError::validation(
                "File path contains a reparse point",
            ));
        }
    }
    if metadata.file_type().is_symlink() {
        return Err(ConnectorError::validation(
            "File path contains a symbolic link",
        ));
    }
    Ok(())
}

fn lock_file(root: &Path, relative: &str) -> Result<std::fs::File, ConnectorError> {
    let lock = root.join(format!(
        ".riviu-lock-{}",
        hash(relative.to_lowercase().as_bytes())
    ));
    if lock.exists() {
        reject_link(&lock)?;
    }
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&lock)
        .map_err(|_| ConnectorError::validation("Cannot open connector file lock"))?;
    file.try_lock().map_err(|_| {
        ConnectorError::new(
            "ConnectorFileBusy",
            "Another Flow is using this file",
            false,
        )
    })?;
    Ok(file)
}
fn confined_path(
    root: &Path,
    relative: &str,
    create_parents: bool,
) -> Result<PathBuf, ConnectorError> {
    validate_relative_path(relative).map_err(ConnectorError::validation)?;
    reject_link(root)?;
    let pieces: Vec<_> = Path::new(relative).components().collect();
    let mut path = root.to_path_buf();
    for (index, part) in pieces.iter().enumerate() {
        path.push(part.as_os_str());
        match std::fs::symlink_metadata(&path) {
            Ok(_) => {
                reject_link(&path)?;
                let canonical = path
                    .canonicalize()
                    .map_err(|_| ConnectorError::validation("Cannot resolve file path"))?;
                if !canonical.starts_with(root) {
                    return Err(ConnectorError::validation("File path escaped flow-data"));
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                if index + 1 < pieces.len() {
                    if !create_parents {
                        return Err(ConnectorError::validation("File directory does not exist"));
                    }
                    std::fs::create_dir(&path)
                        .map_err(|_| ConnectorError::validation("Cannot create file directory"))?;
                }
            }
            Err(_) => return Err(ConnectorError::validation("Cannot inspect file path")),
        }
    }
    Ok(path)
}
fn read_file(path: &Path) -> Result<Vec<u8>, ConnectorError> {
    reject_link(path)?;
    let file = std::fs::File::open(path)
        .map_err(|_| ConnectorError::validation("Cannot open input file"))?;
    if !file
        .metadata()
        .map_err(|_| ConnectorError::validation("Cannot inspect input file"))?
        .is_file()
    {
        return Err(ConnectorError::validation("Input is not a regular file"));
    }
    let mut bytes = Vec::new();
    file.take((MAX_CONNECTOR_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| ConnectorError::validation("Cannot read input file"))?;
    if bytes.len() > MAX_CONNECTOR_BYTES {
        return Err(ConnectorError::validation("Input file exceeds 16 KiB"));
    }
    Ok(bytes)
}

pub fn parse_table(value: &str) -> Result<Vec<Vec<String>>, ConnectorError> {
    let rows: Vec<Vec<String>> = serde_json::from_str(value)
        .map_err(|_| ConnectorError::validation("Table must be a JSON array of string rows"))?;
    let width = rows.first().map_or(0, Vec::len);
    if width == 0 || rows.len() * width > 1000 || rows.iter().any(|row| row.len() != width) {
        return Err(ConnectorError::validation(
            "Table must be a nonempty rectangle with at most 1000 cells",
        ));
    }
    Ok(rows)
}

fn decode_data(format: FileDataFormat, value: &str) -> Result<String, ConnectorError> {
    match format {
        FileDataFormat::Text => Ok(value.into()),
        FileDataFormat::Json => serde_json::from_str::<Value>(value)
            .and_then(|v| serde_json::to_string(&v))
            .map_err(|_| ConnectorError::validation("File contains invalid JSON")),
        FileDataFormat::Csv => serde_json::to_string(&decode_csv(value)?)
            .map_err(|_| ConnectorError::validation("Cannot encode CSV table")),
    }
}
fn encode_data(format: FileDataFormat, value: &str) -> Result<String, ConnectorError> {
    match format {
        FileDataFormat::Text => Ok(value.into()),
        FileDataFormat::Json => decode_data(format, value),
        FileDataFormat::Csv => {
            let rows = parse_table(value)?;
            Ok(rows
                .into_iter()
                .map(|row| {
                    row.into_iter()
                        .map(|cell| format!("\"{}\"", cell.replace('"', "\"\"")))
                        .collect::<Vec<_>>()
                        .join(",")
                })
                .collect::<Vec<_>>()
                .join("\r\n"))
        }
    }
}
fn decode_csv(value: &str) -> Result<Vec<Vec<String>>, ConnectorError> {
    let mut rows = Vec::new();
    let mut row = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut closed = false;
    let mut chars = value.trim_start_matches('\u{feff}').chars().peekable();
    while let Some(c) = chars.next() {
        if quoted {
            if c == '"' {
                if chars.peek() == Some(&'"') {
                    chars.next();
                    field.push('"');
                } else {
                    quoted = false;
                    closed = true;
                }
            } else {
                field.push(c);
            }
        } else {
            match c {
                '"' if field.is_empty() && !closed => quoted = true,
                ',' => {
                    row.push(std::mem::take(&mut field));
                    closed = false;
                }
                '\n' | '\r' => {
                    if c == '\r' && chars.peek() == Some(&'\n') {
                        chars.next();
                    }
                    row.push(std::mem::take(&mut field));
                    rows.push(std::mem::take(&mut row));
                    closed = false;
                }
                '"' => return Err(ConnectorError::validation("CSV quote must start a field")),
                _ if closed => {
                    return Err(ConnectorError::validation(
                        "Unexpected text after CSV quote",
                    ))
                }
                _ => field.push(c),
            }
        }
    }
    if quoted {
        return Err(ConnectorError::validation("Unterminated CSV quote"));
    }
    if !field.is_empty() || !row.is_empty() || closed || rows.is_empty() {
        row.push(field);
        rows.push(row);
    }
    let width = rows[0].len();
    if rows.len() * width > 1000 || rows.iter().any(|row| row.len() != width) {
        return Err(ConnectorError::validation(
            "CSV must contain at most 1000 cells in equal-width rows",
        ));
    }
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use parking_lot::Mutex;
    use std::collections::BTreeMap;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[derive(Default)]
    struct Secrets(Mutex<BTreeMap<String, String>>);
    impl crate::db::SecretStore for Secrets {
        fn get_secret(&self, name: &str) -> anyhow::Result<Option<String>> {
            Ok(self.0.lock().get(name).cloned())
        }
        fn set_secret(&self, name: &str, value: &str) -> anyhow::Result<()> {
            self.0.lock().insert(name.into(), value.into());
            Ok(())
        }
    }
    struct Fixture {
        root: PathBuf,
        executor: FlowConnectorExecutor,
    }
    impl Fixture {
        fn new() -> Self {
            let root =
                std::env::temp_dir().join(format!("riviu-connectors-{}", uuid::Uuid::new_v4()));
            let db = Arc::new(
                Database::open(root.join("fixture.db"))
                    .unwrap()
                    .with_secrets(Arc::new(Secrets::default())),
            );
            let executor = FlowConnectorExecutor::new(db.flow_connector_root(), db).unwrap();
            Self { root, executor }
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    #[tokio::test]
    async fn text_json_csv_write_read_and_replace_have_exact_receipts() {
        let fixture = Fixture::new();
        for (format, value) in [
            (FileDataFormat::Text, "hello\nworld"),
            (FileDataFormat::Json, "{\"n\":2}"),
            (
                FileDataFormat::Csv,
                "[[\"a,b\",\"line\\nquote\\\"\"],[\"=literal\",\"\"]]",
            ),
        ] {
            let write = FileWriteConfig {
                name: "rows".into(),
                path: "nested/fixture.data".into(),
                format,
                value: value.into(),
            };
            let output = fixture.executor.file_write(&write).await.unwrap();
            let read = fixture
                .executor
                .file_read(&FileReadConfig {
                    name: "rows".into(),
                    path: write.path,
                    format,
                })
                .await
                .unwrap();
            assert_eq!(read["value"], value);
            assert_eq!(output["value"], value);
            assert_eq!(read["receipt"], output["receipt"]);
            assert_eq!(read["receipt"].as_str().unwrap().len(), 64);
        }
    }

    #[tokio::test]
    async fn invalid_data_is_rejected_before_file_replacement() {
        let fixture = Fixture::new();
        let path = fixture.executor.root.join("file.json");
        std::fs::write(&path, "original").unwrap();
        for (format, value) in [
            (FileDataFormat::Json, "{"),
            (FileDataFormat::Csv, "[[\"a\"],[]]"),
        ] {
            let error = fixture
                .executor
                .file_write(&FileWriteConfig {
                    name: "output".into(),
                    path: "file.json".into(),
                    format,
                    value: value.into(),
                })
                .await
                .unwrap_err();
            assert!(!error.may_have_applied);
            assert_eq!(std::fs::read_to_string(&path).unwrap(), "original");
        }
        std::fs::write(&path, "x".repeat(MAX_CONNECTOR_BYTES + 1)).unwrap();
        assert!(fixture
            .executor
            .file_read(&FileReadConfig {
                name: "out".into(),
                path: "file.json".into(),
                format: FileDataFormat::Text
            })
            .await
            .is_err());
    }

    #[tokio::test]
    async fn concurrent_file_writer_is_rejected_without_changing_the_target() {
        let fixture = Fixture::new();
        let path = fixture.executor.root.join("shared.txt");
        std::fs::write(&path, "original").unwrap();
        let lock = lock_file(&fixture.executor.root, "shared.txt").unwrap();
        let config = FileWriteConfig {
            name: "value".into(),
            path: "shared.txt".into(),
            format: FileDataFormat::Text,
            value: "changed".into(),
        };
        let error = fixture.executor.file_write(&config).await.unwrap_err();
        assert_eq!(error.code, "ConnectorFileBusy");
        assert!(!error.may_have_applied);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "original");
        drop(lock);
        fixture.executor.file_write(&config).await.unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "changed");
    }

    #[test]
    fn template_validation_accepts_dynamic_fields_but_runtime_checks_resolved_values() {
        let config = SheetWriteConfig {
            name: "sheet".into(),
            spreadsheet_url: "${sheet_url}".into(),
            tab: "${tab}".into(),
            range: "A${row}:B${row}".into(),
            values: "${values}".into(),
        };
        assert!(config.validate_template().is_ok());
        assert!(config.validate().is_err());
        let mut http = request("${endpoint}/rows/${row}".into());
        assert!(http.validate_template().is_ok());
        assert!(http.validate().is_err());
        http.url = "${broken".into();
        assert!(http.validate_template().is_err());
        let file = FileReadConfig {
            name: "in".into(),
            path: "../${file}".into(),
            format: FileDataFormat::Text,
        };
        assert!(file.validate_template().is_err());
    }

    #[test]
    fn stored_receipts_reject_changed_values_bad_status_and_wrong_digest() {
        let http =
            super::super::CompiledActionConfig::HttpRequest(request("https://example.com".into()));
        let value = "{\"ok\":true}";
        let receipt = format!("HTTP 200; sha256={}", hash(value.as_bytes()));
        assert!(validate_output_receipt(&http, value, &receipt));
        assert!(!validate_output_receipt(&http, "changed", &receipt));
        assert!(!validate_output_receipt(
            &http,
            value,
            &receipt.replace("200", "500")
        ));
        let sheet = super::super::CompiledActionConfig::SheetRead(SheetReadConfig {
            name: "rows".into(),
            spreadsheet_url: "https://docs.google.com/spreadsheets/d/fixture/edit".into(),
            tab: "tab".into(),
            range: "A1".into(),
        });
        let value = "[[\"hello\"]]";
        assert!(validate_output_receipt(
            &sheet,
            value,
            &hash(value.as_bytes())
        ));
        assert!(!validate_output_receipt(&sheet, "{}", &hash(b"{}")));
        let file = super::super::CompiledActionConfig::FileWrite(FileWriteConfig {
            name: "file".into(),
            path: "data.csv".into(),
            format: FileDataFormat::Csv,
            value: value.into(),
        });
        let encoded = encode_data(FileDataFormat::Csv, value).unwrap();
        assert!(validate_output_receipt(
            &file,
            value,
            &hash(encoded.as_bytes())
        ));
        assert!(!validate_output_receipt(&file, value, "pretend-receipt"));
    }

    #[test]
    fn paths_ranges_and_csv_reject_ambiguous_inputs() {
        for path in [
            "../secret",
            "/absolute",
            "C:/secret",
            "dir/../escape",
            "file:stream",
            "CON.txt",
            "folder./x",
            "bad\\path",
        ] {
            assert!(validate_relative_path(path).is_err(), "{path}");
        }
        assert_eq!(parse_sheet_range("B2:D4"), Ok((2, 2, 3, 3)));
        for range in [
            "A:A",
            "A0",
            "B2:A1",
            "A1:XFD1000000",
            "Sheet!A1",
            "a1",
            "A01",
            "A1:B2:C3",
        ] {
            assert!(parse_sheet_range(range).is_err(), "{range}");
        }
        for csv in ["\"unterminated", "\"closed\"garbage", "a,b\nc", "x\"y"] {
            assert!(decode_csv(csv).is_err(), "{csv}");
        }
        assert_eq!(
            decode_csv("\u{feff}\"a,b\",\"c\"\"d\"\r\n1,2\r\n").unwrap(),
            vec![vec!["a,b", "c\"d"], vec!["1", "2"]]
        );
    }

    #[test]
    fn connector_credentials_are_reference_only_and_not_in_database() {
        let fixture = Fixture::new();
        let db = &fixture.executor.database;
        db.set_flow_connector_secret("fixture", "private-bearer-value")
            .unwrap();
        assert_eq!(db.list_flow_connector_secrets().unwrap(), vec!["fixture"]);
        assert_eq!(
            db.flow_connector_secret("fixture").unwrap().as_deref(),
            Some("private-bearer-value")
        );
        let conn = rusqlite::Connection::open(fixture.root.join("fixture.db")).unwrap();
        let count: i64 = conn
            .query_row(
                "SELECT count(*) FROM settings WHERE value LIKE '%private-bearer-value%'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 0);
        drop(conn);
        db.set_flow_connector_secret("fixture", "").unwrap();
        assert!(db.list_flow_connector_secrets().unwrap().is_empty());
        assert!(db.flow_connector_secret("fixture").unwrap().is_none());
    }

    async fn serve(response: String) -> (String, tokio::task::JoinHandle<String>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut data = Vec::new();
            let mut buffer = [0u8; 4096];
            loop {
                let length = stream.read(&mut buffer).await.unwrap();
                if length == 0 {
                    break;
                }
                data.extend_from_slice(&buffer[..length]);
                let text = String::from_utf8_lossy(&data);
                if let Some(end) = text.find("\r\n\r\n") {
                    let content_length = text[..end]
                        .lines()
                        .find_map(|line| {
                            line.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .and_then(|v| v.trim().parse::<usize>().ok())
                        })
                        .unwrap_or(0);
                    if data.len() >= end + 4 + content_length {
                        break;
                    }
                }
            }
            stream.write_all(response.as_bytes()).await.unwrap();
            String::from_utf8(data).unwrap()
        });
        (format!("http://{address}/fixture"), task)
    }
    fn request(url: String) -> HttpRequestConfig {
        HttpRequestConfig {
            name: "http".into(),
            url,
            method: HttpMethod::POST,
            body: Some("{\"ready\":true}".into()),
            secret_ref: None,
            timeout_ms: 1000,
        }
    }

    #[tokio::test]
    async fn http_dispatches_actual_body_and_keyring_bearer_once() {
        let fixture = Fixture::new();
        fixture
            .executor
            .database
            .set_flow_connector_secret("api", "fixture-key")
            .unwrap();
        let (url, task) = serve(
            "HTTP/1.1 200 OK\r\nContent-Length: 11\r\nConnection: close\r\n\r\n{\"ok\":true}"
                .into(),
        )
        .await;
        let mut config = request(url);
        config.secret_ref = Some("api".into());
        let result = fixture.executor.http_request(&config).await.unwrap();
        assert_eq!(result["value"], "{\"ok\":true}");
        let sent = task.await.unwrap();
        assert!(sent.starts_with("POST /fixture HTTP/1.1"));
        assert!(sent
            .to_ascii_lowercase()
            .contains("authorization: bearer fixture-key"));
        assert!(sent.ends_with("{\"ready\":true}"));
        assert!(!result.to_string().contains("fixture-key"));
    }

    #[tokio::test]
    async fn redirects_server_errors_and_oversize_responses_are_uncertain_without_replay() {
        let fixture = Fixture::new();
        for response in [
            "HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:1/moved\r\nContent-Length: 0\r\n\r\n"
                .into(),
            "HTTP/1.1 500 Error\r\nContent-Length: 0\r\n\r\n".into(),
            format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{}",
                MAX_CONNECTOR_BYTES + 1,
                "x".repeat(MAX_CONNECTOR_BYTES + 1)
            ),
        ] {
            let (url, task) = serve(response).await;
            let error = fixture
                .executor
                .http_request(&request(url))
                .await
                .unwrap_err();
            assert!(error.may_have_applied);
            task.await.unwrap();
        }
    }

    #[tokio::test]
    async fn timeout_after_receiving_body_is_ambiguous_and_sends_once() {
        let fixture = Fixture::new();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mut config = request(format!("http://{}/slow", listener.local_addr().unwrap()));
        config.timeout_ms = 100;
        let task = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut bytes = [0; 4096];
            let count = stream.read(&mut bytes).await.unwrap();
            assert!(count > 0);
            assert!(
                tokio::time::timeout(Duration::from_millis(250), listener.accept())
                    .await
                    .is_err()
            );
        });
        let error = fixture.executor.http_request(&config).await.unwrap_err();
        assert!(error.may_have_applied);
        assert_eq!(error.code, "ConnectorHttp");
        task.await.unwrap();
    }

    #[tokio::test]
    async fn absent_secret_and_invalid_sheet_values_do_not_dispatch() {
        let fixture = Fixture::new();
        let mut config = request("http://127.0.0.1:1/fixture".into());
        config.secret_ref = Some("missing".into());
        assert!(
            !fixture
                .executor
                .http_request(&config)
                .await
                .unwrap_err()
                .may_have_applied
        );
        let invalid = SheetWriteConfig {
            name: "sheet".into(),
            spreadsheet_url: "https://docs.google.com/spreadsheets/d/fixture/edit".into(),
            tab: "Test".into(),
            range: "A1:B2".into(),
            values: "[[\"x\"]]".into(),
        };
        assert!(
            !fixture
                .executor
                .sheet_write(&invalid)
                .await
                .unwrap_err()
                .may_have_applied
        );
    }
}
