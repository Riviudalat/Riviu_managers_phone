use super::*;
use chrono::Datelike;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Cell {
    #[serde(default)]
    pub formatted_value: String,
    #[serde(default)]
    pub user_entered_value: Value,
    #[serde(default)]
    pub note: String,
}
impl Cell {
    pub fn display(&self) -> String {
        if !self.formatted_value.is_empty() {
            return self.formatted_value.clone();
        }
        if let Some(s) = self.user_entered_value["stringValue"].as_str() {
            return s.into();
        }
        if let Some(n) = self.user_entered_value["numberValue"].as_i64() {
            return n.to_string();
        }
        String::new()
    }
    pub fn empty(&self) -> bool {
        self.display().is_empty() && self.note.is_empty() && self.user_entered_value.is_null()
    }
    fn formula(&self) -> bool {
        self.user_entered_value.get("formulaValue").is_some()
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct Row {
    pub index: u32,
    pub cells: Vec<Cell>,
}
impl Row {
    fn values(&self) -> Vec<String> {
        self.cells.iter().map(Cell::display).collect()
    }
    pub(super) fn note(&self) -> Option<Value> {
        parse_note(self.cells.get(3)?.note.as_str())
    }
}
#[derive(Debug, Clone)]
pub(super) struct Layout {
    pub width: usize,
    pub internal: bool,
    pub headers: Vec<String>,
}
impl Layout {
    pub fn standard(internal: bool) -> Self {
        let mut headers = vec!["STT", "Người air", "Ngày", "Link"];
        if internal {
            headers.extend(["Máy", "Tài khoản TikTok", "Trạng thái", "Lỗi hoặc ghi chú"]);
        }
        headers.push("Đối tác");
        Self {
            width: headers.len(),
            internal,
            headers: headers.into_iter().map(str::to_owned).collect(),
        }
    }
    pub fn from_header(cells: &[Cell]) -> Result<Self> {
        let headers: Vec<_> = cells
            .iter()
            .map(|c| c.display().trim().to_owned())
            .collect();
        if cells.iter().take(9).any(Cell::formula)
            || headers.len() < 5
            || headers[..4] != ["STT", "Người air", "Ngày", "Link"]
        {
            return Err(DirectSheetsError::invalid(
                "Header must be STT, Người air, Ngày, Link",
            ));
        }
        let metadata = ["Máy", "Tài khoản TikTok", "Trạng thái", "Lỗi hoặc ghi chú"];
        let internal = headers.len() >= 9 && headers[4..8] == metadata;
        if !internal
            && metadata
                .iter()
                .enumerate()
                .any(|(n, v)| headers.get(n + 4).is_some_and(|s| s == v))
        {
            return Err(DirectSheetsError::invalid(
                "Internal report header is incomplete",
            ));
        }
        let start = if internal { 8 } else { 4 };
        let mut count = 0;
        while let Some(header) = headers.get(start + count) {
            let expected = if count == 0 {
                "Đối tác".into()
            } else {
                format!("Đối tác {}", count + 1)
            };
            if header != &expected {
                break;
            }
            count += 1;
        }
        if count == 0 || count > 100 {
            return Err(DirectSheetsError::invalid(
                "Consecutive partner headers are required (maximum100)",
            ));
        }
        let width = start + count;
        if cells.iter().take(width).any(Cell::formula) {
            return Err(DirectSheetsError::invalid(
                "Managed headers cannot be formulas",
            ));
        }
        Ok(Self {
            width,
            internal,
            headers: headers[..width].to_vec(),
        })
    }
    fn partner_start(&self) -> usize {
        if self.internal {
            8
        } else {
            4
        }
    }
}

#[derive(Default, Clone, Serialize, Deserialize)]
pub(super) struct Scan {
    pub epochs: BTreeSet<String>,
    pub last_nonempty: Option<u32>,
    pub first_empty: Option<Row>,
    max_stt: i64,
    pub matched: Option<Row>,
    linked: Option<Row>,
    ids: BTreeSet<String>,
}
impl Scan {
    pub fn observe(&mut self, row: Row, payload: Option<&Value>) -> Result<()> {
        if row.cells.iter().all(Cell::empty) {
            if self.first_empty.is_none() {
                self.first_empty = Some(row);
            }
            return Ok(());
        }
        self.last_nonempty = Some(row.index);
        let link_note = row
            .cells
            .get(3)
            .map(|c| c.note.as_str())
            .unwrap_or_default();
        if link_note.contains("riviu-publish") && row.note().is_none() {
            return Err(DirectSheetsError::conflict(
                "A publication note is damaged; repair its identity before adding rows",
            ));
        }
        if let Ok(n) = row.cells[0].display().parse::<i64>() {
            if n > self.max_stt {
                self.max_stt = n;
            }
        }
        if let Some(note) = row.note() {
            let id = note["assignmentId"].as_str().unwrap_or_default();
            if !self.ids.insert(id.into()) {
                return Err(DirectSheetsError::conflict(
                    "Multiple rows have the same publication ID",
                ));
            }
            self.epochs
                .insert(note["reportingEpoch"].as_str().unwrap_or("legacy").into());
            if payload.is_some_and(|p| p["publicationId"] == id) {
                self.matched = Some(row.clone());
            }
        }
        if payload.is_some_and(|p| {
            p["postUrl"]
                .as_str()
                .is_some_and(|url| !url.is_empty() && row.cells[3].display() == url)
        }) {
            if self.linked.is_some() {
                return Err(DirectSheetsError::conflict(
                    "Canonical URL exists on multiple rows",
                ));
            }
            self.linked = Some(row);
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub(super) struct WritePlan {
    pub row: u32,
    pub width: usize,
    pub before: Row,
    pub requests: Vec<Value>,
    pub duplicate: bool,
}
fn parse_note(raw: &str) -> Option<Value> {
    if let Some(id) = raw.strip_prefix("riviu-publish:v1:") {
        return Some(json!({"legacy":true,"assignmentId":id}));
    }
    let value: Value = serde_json::from_str(raw).ok()?;
    (value["kind"] == "riviu-publish"
        && (value["deliveryVersion"] == 2 || value["reportVersion"] == 1)
        && value["assignmentId"].is_string())
    .then_some(value)
}

pub(super) fn diagnose_row(
    row: &Row,
    assignment_id: &str,
    expected_revision: i64,
    spreadsheet_id: &str,
    gid: u64,
    epoch: &str,
    expected_content: (&str, Option<&str>),
) -> Option<super::SheetDiagnosticMatch> {
    let (expected_url, expected_posted_at) = expected_content;
    let note = row.note()?;
    if note["assignmentId"].as_str()? != assignment_id {
        return None;
    }
    let note_revision = note["canonicalDeliveryRevision"].as_i64();
    let values = row.values();
    let d_cell = values.get(3).map(String::as_str).unwrap_or_default();
    Some(super::SheetDiagnosticMatch {
        row: row.index + 1,
        note_revision,
        identity_matches: note["kind"] == "riviu-publish"
            && note["deliveryVersion"] == 2
            && note["publicationId"] == assignment_id,
        revision_matches: note_revision == Some(expected_revision),
        note_epoch_matches: note["reportingEpoch"] == epoch,
        note_target_matches: note["spreadsheetId"] == spreadsheet_id
            && note["sheetGid"].as_u64() == Some(gid),
        fingerprint_matches: !row.cells.iter().any(Cell::formula)
            && matches_fingerprint(&values, note["rowFingerprint"].as_str().unwrap_or_default()),
        url_matches: d_cell == expected_url,
        d_cell_empty: d_cell.is_empty(),
        d_cell_has_canonical_url: canonical(d_cell),
        posted_at_matches: expected_posted_at.is_some_and(|expected| {
            note["postedAt"].as_str() == Some(expected)
                && note
                    .get("canonicalPostedAt")
                    .is_none_or(|value| value.as_str() == Some(expected))
        }),
    })
}
fn fingerprint(value: &Value) -> String {
    format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(value).expect("JSON serializes"))
    )
}
fn row_fingerprint(values: &[String]) -> String {
    fingerprint(&json!(values))
}
fn matches_fingerprint(values: &[String], expected: &str) -> bool {
    let mut len = values.len();
    while len >= 5 {
        if row_fingerprint(&values[..len]) == expected {
            return true;
        }
        if !values[len - 1].is_empty() {
            break;
        }
        len -= 1;
    }
    false
}
fn field<'a>(p: &'a Value, key: &str) -> Result<&'a str> {
    p[key]
        .as_str()
        .ok_or_else(|| DirectSheetsError::invalid(format!("Missing string field {key}")))
}
fn revision(p: &Value, key: &str) -> Result<i64> {
    p[key]
        .as_i64()
        .filter(|n| (0..=9_007_199_254_740_991).contains(n))
        .ok_or_else(|| DirectSheetsError::invalid(format!("Invalid {key}")))
}
fn partners(p: &Value) -> Result<Vec<String>> {
    let rows = p["partners"]
        .as_array()
        .filter(|r| r.len() <= 100)
        .ok_or_else(|| DirectSheetsError::invalid("Invalid partner list"))?;
    rows.iter()
        .map(|v| {
            v.as_str()
                .filter(|s| s.len() <= 4096)
                .map(str::to_owned)
                .ok_or_else(|| DirectSheetsError::invalid("Invalid partner name"))
        })
        .collect()
}
fn canonical(url: &str) -> bool {
    let Ok(u) = url::Url::parse(url) else {
        return false;
    };
    let segments = u.path().split('/').collect::<Vec<_>>();
    u.scheme() == "https"
        && matches!(u.host_str(), Some("www.tiktok.com" | "tiktok.com"))
        && u.username().is_empty()
        && u.password().is_none()
        && u.port_or_known_default() == Some(443)
        && u.query().is_none()
        && u.fragment().is_none()
        && segments.len() == 4
        && segments[1].starts_with('@')
        && segments[1].len() > 1
        && matches!(segments[2], "photo" | "video")
        && !segments[3].is_empty()
        && segments[3].bytes().all(|b| b.is_ascii_digit())
}
pub(super) fn validate_payload(
    p: &Value,
    target: &SheetDeliveryTarget,
    layout: &Layout,
) -> Result<()> {
    let id = field(p, "publicationId")?;
    let by = field(p, "poster")?;
    let url = field(p, "postUrl")?;
    if id.trim() != id
        || id.is_empty()
        || id.len() > 128
        || p["assignmentId"] != id
        || by.trim().is_empty()
        || by.len() > 512
        || p["deliveryVersion"] != 2
        || p["spreadsheetId"] != target.spreadsheet_id
        || p["sheetGid"].as_u64() != Some(target.sheet_gid)
        || p["reportingEpoch"].as_str()
            != Some(target.reporting_epoch.as_deref().unwrap_or("legacy"))
    {
        return Err(DirectSheetsError::invalid(
            "Pinned publication or target identity mismatch",
        ));
    }
    revision(p, "deliveryRevision")?;
    partners(p)?;
    let kind = field(p, "rowKind")?;
    if !matches!(kind, "canonical" | "internalReport")
        || (!layout.internal && kind != "canonical")
        || (!url.is_empty() && !canonical(url))
        || (kind == "canonical" && url.is_empty())
    {
        return Err(DirectSheetsError::invalid(
            "Invalid row kind or canonical post URL",
        ));
    }
    if layout.internal {
        if p["reportVersion"] != 1 {
            return Err(DirectSheetsError::invalid(
                "Internal report metadata version required",
            ));
        }
        let r = revision(p, "rowRevision")?;
        if kind == "internalReport" && r != revision(p, "deliveryRevision")? {
            return Err(DirectSheetsError::invalid(
                "Report delivery revision differs",
            ));
        }
        let status = field(p, "status")?;
        if !matches!(
            status,
            "Chưa đăng" | "Đã gửi" | "Đã xác minh" | "Cần kiểm tra"
        ) || (status == "Đã xác minh") == url.is_empty()
        {
            return Err(DirectSheetsError::invalid(
                "Verified status and canonical URL disagree",
            ));
        }
        for key in ["machine", "tiktokAccount", "stateNotes"] {
            if field(p, key)?.len() > 4096 {
                return Err(DirectSheetsError::invalid("Report metadata exceeds limit"));
            }
        }
        let account = field(p, "tiktokAccount")?.trim_start_matches('@');
        if !url.is_empty()
            && (!account.is_empty()
                && !url::Url::parse(url)
                    .expect("validated URL")
                    .path()
                    .split('/')
                    .nth(1)
                    .unwrap_or("")
                    .trim_start_matches('@')
                    .eq_ignore_ascii_case(account))
        {
            return Err(DirectSheetsError::conflict(
                "Canonical URL belongs to another account",
            ));
        }
    }
    if !p["postedAt"].is_null() {
        chrono::DateTime::parse_from_rfc3339(field(p, "postedAt")?)
            .map_err(|_| DirectSheetsError::invalid("Invalid immutable submitted time"))?;
    } else if kind == "canonical" {
        return Err(DirectSheetsError::invalid(
            "Canonical row requires immutable submitted time",
        ));
    }
    Ok(())
}

fn date(p: &Value, zone: &str) -> Result<String> {
    if p["postedAt"].is_null() {
        return Ok(String::new());
    }
    let at = chrono::DateTime::parse_from_rfc3339(field(p, "postedAt")?)
        .map_err(|_| DirectSheetsError::invalid("Invalid submitted time"))?;
    let zone: chrono_tz::Tz = zone
        .parse()
        .map_err(|_| DirectSheetsError::invalid("Unsupported spreadsheet timezone"))?;
    let local = at.with_timezone(&zone);
    Ok(format!(
        "{}/{}/{}",
        local.day(),
        local.month(),
        local.year()
    ))
}
fn validate_stored(row: &Row, meta: &Metadata, epoch: &str) -> Result<Option<Value>> {
    if row.cells.iter().any(Cell::formula) {
        return Err(DirectSheetsError::conflict(
            "Managed row contains a formula",
        ));
    }
    let values = row.values();
    let note = row.note();
    if let Some(n) = &note {
        if n["deliveryVersion"] == 2
            && (n["spreadsheetId"] != meta.spreadsheet_id
                || n["sheetGid"].as_u64() != Some(meta.gid)
                || n["reportingEpoch"].as_str().unwrap_or("legacy") != epoch
                || n["publicationId"]
                    .as_str()
                    .is_some_and(|id| Some(id) != n["assignmentId"].as_str()))
        {
            return Err(DirectSheetsError::conflict(
                "Stored row identity or epoch changed",
            ));
        }
        if n["legacy"] != true
            && !matches_fingerprint(&values, n["rowFingerprint"].as_str().unwrap_or_default())
        {
            return Err(DirectSheetsError::conflict(
                "Stored row content or fingerprint changed",
            ));
        }
    }
    Ok(note)
}

pub(super) fn plan(meta: &Metadata, layout: &Layout, scan: &Scan, p: &Value) -> Result<WritePlan> {
    let id = field(p, "publicationId")?;
    let url = field(p, "postUrl")?;
    let epoch = field(p, "reportingEpoch")?;
    let by = field(p, "poster")?;
    let given = partners(p)?;
    let partner_count = given.len().max(layout.width - layout.partner_start());
    let width = layout.partner_start() + partner_count;
    let extra = width - layout.width;
    let mut padded = given;
    padded.resize(partner_count, String::new());
    let date = date(p, &meta.time_zone)?;
    let mut data = vec![by.into(), date, url.into()];
    let metadata = if layout.internal {
        vec![
            field(p, "machine")?.into(),
            field(p, "tiktokAccount")?.into(),
            field(p, "status")?.into(),
            field(p, "stateNotes")?.into(),
        ]
    } else {
        vec![]
    };
    data.extend(metadata.clone());
    data.extend(padded.clone());
    let row = scan
        .matched
        .clone()
        .or_else(|| scan.linked.clone())
        .or_else(|| scan.first_empty.clone())
        .unwrap_or_else(|| Row {
            index: scan.last_nonempty.map_or(1, |n| n + 1),
            cells: vec![Cell::default(); layout.width],
        });
    let before = row.clone();
    let original = row.values();
    let mut prior = validate_stored(&row, meta, epoch)?;
    let keyed = prior.as_ref().is_some_and(|n| n["assignmentId"] == id);
    if prior.is_some() && !keyed {
        return Err(DirectSheetsError::conflict(
            "Canonical URL or selected row belongs to another publication",
        ));
    }
    let occupied = !row.cells.iter().all(Cell::empty);
    let number = if occupied {
        original[0]
            .parse::<i64>()
            .ok()
            .filter(|n| *n > 0)
            .ok_or_else(|| DirectSheetsError::conflict("Existing STT is invalid"))?
    } else {
        scan.max_stt
            .checked_add(1)
            .filter(|n| *n <= 9_007_199_254_740_991)
            .ok_or_else(|| DirectSheetsError::invalid("STT exhausted"))?
    };
    if let Some(note) = &prior {
        if layout.internal
            && note["reportVersion"] == 1
            && p["rowKind"] == "internalReport"
            && revision(p, "rowRevision")? < revision(note, "rowRevision")?
        {
            if extra > 0 || (!url.is_empty() && original[3] != url) {
                return Err(DirectSheetsError::conflict(
                    "Stale report cannot change canonical identity or columns",
                ));
            }
            return Ok(WritePlan {
                row: row.index,
                width: layout.width,
                before,
                requests: vec![],
                duplicate: true,
            });
        }
    }
    if occupied {
        if original[1] != by
            || (!original[2].is_empty() && original[2] != data[1])
            || (!original[3].is_empty()
                && original[3] != url
                && !(keyed
                    && p["rowKind"] == "internalReport"
                    && prior
                        .as_ref()
                        .and_then(|n| n["rowRevision"].as_i64())
                        .is_some_and(|old| p["rowRevision"].as_i64().is_some_and(|new| new < old))))
        {
            return Err(DirectSheetsError::conflict(
                "Existing poster, date or canonical identity differs",
            ));
        }
        if !original[layout.partner_start()..]
            .iter()
            .zip(padded.iter())
            .all(|(a, b)| a == b)
        {
            return Err(DirectSheetsError::conflict("Partner identity changed"));
        }
        if !keyed && !row.cells[3].note.is_empty() {
            return Err(DirectSheetsError::conflict(
                "Existing link has unrelated notes",
            ));
        }
        if !keyed && original[3] != url {
            return Err(DirectSheetsError::conflict(
                "Unkeyed partial rows are not publication identity",
            ));
        }
    }
    if let Some(note) = &prior {
        if p["rowKind"] == "canonical" {
            if note.get("canonicalDeliveryRevision").is_some()
                && note["canonicalDeliveryRevision"] != p["deliveryRevision"]
            {
                return Err(DirectSheetsError::conflict(
                    "Canonical delivery revision changed",
                ));
            }
            if note.get("canonicalPostedAt").is_some() && note["canonicalPostedAt"] != p["postedAt"]
            {
                return Err(DirectSheetsError::conflict(
                    "Canonical submitted time changed",
                ));
            }
        }
        if layout.internal && note["reportVersion"] == 1 {
            let old = revision(note, "rowRevision")?;
            let new = revision(p, "rowRevision")?;
            if new <= old && extra > 0 {
                return Err(DirectSheetsError::conflict(
                    "Old report cannot expand partner columns",
                ));
            }
            if new < old && p["rowKind"] == "internalReport" {
                return Ok(WritePlan {
                    row: row.index,
                    width: layout.width,
                    before,
                    requests: vec![],
                    duplicate: true,
                });
            }
            if new < old && p["rowKind"] == "canonical" {
                if (!original[3].is_empty() && original[3] != url)
                    || note["postedAt"] != p["postedAt"]
                    || extra > 0
                {
                    return Err(DirectSheetsError::conflict(
                        "Canonical completion conflicts with newer stored report",
                    ));
                }
                let fill_link = original[3].is_empty();
                if fill_link {
                    let account = original[5].trim_start_matches('@');
                    let url_account = url::Url::parse(url)
                        .expect("validated canonical URL")
                        .path()
                        .split('/')
                        .nth(1)
                        .expect("validated canonical URL")
                        .trim_start_matches('@')
                        .to_owned();
                    if account.is_empty() || !account.eq_ignore_ascii_case(&url_account) {
                        return Err(DirectSheetsError::conflict(
                            "Canonical completion belongs to another account",
                        ));
                    }
                }
                let mut upgraded = note.clone();
                upgraded["canonicalDeliveryRevision"] = p["deliveryRevision"].clone();
                upgraded["canonicalPostedAt"] = p["postedAt"].clone();
                let mut requests = Vec::new();
                if fill_link {
                    let mut updated_values = original.clone();
                    updated_values[3] = url.into();
                    upgraded["rowFingerprint"] = json!(row_fingerprint(&updated_values));
                    requests.push(json!({"updateCells":{"start":{"sheetId":meta.gid,"rowIndex":row.index,"columnIndex":3},"rows":[{"values":[{"userEnteredValue":{"stringValue":url}}]}],"fields":"userEnteredValue"}}));
                }
                if fill_link || note.get("canonicalDeliveryRevision").is_none() {
                    requests.push(json!({"updateCells":{"start":{"sheetId":meta.gid,"rowIndex":row.index,"columnIndex":3},"rows":[{"values":[{"note":serde_json::to_string(&upgraded).expect("note JSON")}]}],"fields":"note"}}));
                }
                return Ok(WritePlan {
                    row: row.index,
                    width: layout.width,
                    before,
                    requests,
                    duplicate: true,
                });
            }
            if new == old {
                let mut count = padded.len();
                let mut equal = false;
                while count >= 1 {
                    if fingerprint(&json!([by, p["postedAt"], url, metadata, &padded[..count]]))
                        == note["payloadFingerprint"].as_str().unwrap_or_default()
                    {
                        equal = true;
                        break;
                    }
                    if !padded[count - 1].is_empty() {
                        break;
                    }
                    count -= 1;
                }
                if !equal {
                    return Err(DirectSheetsError::conflict(
                        "Same report revision has different content",
                    ));
                }
                if p["rowKind"] == "internalReport"
                    || note.get("canonicalDeliveryRevision").is_some()
                {
                    return Ok(WritePlan {
                        row: row.index,
                        width: layout.width,
                        before,
                        requests: vec![],
                        duplicate: true,
                    });
                }
            }
            if !note["postedAt"].is_null() && note["postedAt"] != p["postedAt"] {
                return Err(DirectSheetsError::conflict("Submitted time is immutable"));
            }
        } else if !layout.internal && occupied && original[1..] != data[..layout.width - 1] {
            return Err(DirectSheetsError::conflict("Existing compact row differs"));
        }
    }
    let mut display = vec![number.to_string()];
    display.extend(data.clone());
    let mut note = prior.take().unwrap_or_else(|| json!({}));
    note.as_object_mut()
        .expect("parsed note object")
        .remove("legacy");
    note["kind"] = json!("riviu-publish");
    note["deliveryVersion"] = json!(2);
    note["assignmentId"] = json!(id);
    note["publicationId"] = json!(id);
    note["reportingEpoch"] = json!(epoch);
    note["spreadsheetId"] = json!(meta.spreadsheet_id);
    note["sheetGid"] = json!(meta.gid);
    note["rowFingerprint"] = json!(row_fingerprint(&display));
    if layout.internal {
        note["reportVersion"] = json!(1);
        note["rowRevision"] = p["rowRevision"].clone();
        note["postedAt"] = p["postedAt"].clone();
        note["payloadFingerprint"] = json!(fingerprint(&json!([
            by,
            p["postedAt"],
            url,
            metadata,
            padded
        ])));
    }
    if p["rowKind"] == "canonical" {
        note["canonicalDeliveryRevision"] = p["deliveryRevision"].clone();
        note["canonicalPostedAt"] = p["postedAt"].clone();
    }
    let note = serde_json::to_string(&note)
        .map_err(|_| DirectSheetsError::invalid("Row note serialization failed"))?;
    let mut requests = Vec::new();
    if extra > 0 {
        requests.push(json!({"insertDimension":{"range":{"sheetId":meta.gid,"dimension":"COLUMNS","startIndex":layout.width,"endIndex":width},"inheritFromBefore":true}}));
        requests.push(json!({"updateCells":{"start":{"sheetId":meta.gid,"rowIndex":0,"columnIndex":layout.width},"rows":[{"values":(layout.width-layout.partner_start()+1..=partner_count).map(|n|json!({"userEnteredValue":{"stringValue":format!("Đối tác {n}")}})).collect::<Vec<_>>()}],"fields":"userEnteredValue"}}));
    }
    if row.index >= meta.row_count {
        requests.push(json!({"appendDimension":{"sheetId":meta.gid,"dimension":"ROWS","length":row.index+1-meta.row_count}}));
    }
    let mut values = vec![json!({"userEnteredValue":{"numberValue":number}})];
    values.extend(
        data.iter()
            .map(|v| json!({"userEnteredValue":{"stringValue":v}})),
    );
    requests.push(json!({"updateCells":{"start":{"sheetId":meta.gid,"rowIndex":row.index,"columnIndex":0},"rows":[{"values":values}],"fields":"userEnteredValue"}}));
    requests.push(json!({"updateCells":{"start":{"sheetId":meta.gid,"rowIndex":row.index,"columnIndex":3},"rows":[{"values":[{"note":note}]}],"fields":"note"}}));
    Ok(WritePlan {
        row: row.index,
        width,
        before,
        requests,
        duplicate: occupied,
    })
}

pub(super) fn receipt(row: &Row, p: &Value, duplicate: bool) -> Result<DeliveryReceipt> {
    let note = row
        .note()
        .ok_or_else(|| DirectSheetsError::conflict("Readback publication note missing"))?;
    let values = row.values();
    let id = field(p, "publicationId")?;
    if row.cells.iter().any(Cell::formula)
        || note["deliveryVersion"] != 2
        || note["assignmentId"] != id
        || note["publicationId"] != id
        || note["spreadsheetId"] != p["spreadsheetId"]
        || note["sheetGid"] != p["sheetGid"]
        || note["reportingEpoch"] != p["reportingEpoch"]
        || !matches_fingerprint(&values, note["rowFingerprint"].as_str().unwrap_or_default())
    {
        return Err(DirectSheetsError::conflict(
            "Readback row identity, epoch or content differs",
        ));
    }
    let rev = revision(
        &note,
        if p["rowKind"] == "canonical" {
            "canonicalDeliveryRevision"
        } else {
            "rowRevision"
        },
    )?;
    let expected = revision(p, "deliveryRevision")?;
    if (p["rowKind"] == "canonical" && rev != expected)
        || (p["rowKind"] == "internalReport" && rev < expected)
        || (!field(p, "postUrl")?.is_empty() && values[3] != p["postUrl"])
        || (!values[3].is_empty() && !canonical(&values[3]))
    {
        return Err(DirectSheetsError::conflict(
            "Readback revision or canonical URL differs",
        ));
    }
    Ok(DeliveryReceipt {
        publication_id: id.into(),
        row: row.index + 1,
        revision: rev,
        reporting_epoch: field(p, "reportingEpoch")?.into(),
        post_url: values[3].clone(),
        duplicate,
    })
}
