use super::*;
use planner::*;

fn meta(internal: bool) -> Metadata {
    let layout = Layout::standard(internal);
    Metadata {
        spreadsheet_id: "fixture-book".into(),
        spreadsheet_name: "Fixture book".into(),
        gid: 0,
        title: "Fixture".into(),
        time_zone: "Asia/Ho_Chi_Minh".into(),
        row_count: 20,
        column_count: layout.width as u32,
        owner: Some(Owner {
            schema_version: 1,
            writer_id: "550e8400-e29b-41d4-a716-446655440000".into(),
            reporting_epoch: "fixture-epoch".into(),
            state: "ready".into(),
            reset_id: None,
            backup_spreadsheet_id: None,
            backup_fingerprint: None,
        }),
        owner_raw: None,
        header: layout
            .headers
            .iter()
            .map(|s| Cell {
                formatted_value: s.clone(),
                user_entered_value: json!({"stringValue":s}),
                note: String::new(),
            })
            .collect(),
    }
}
pub(super) fn payload(id: &str, revision: i64, url: &str) -> Value {
    json!({"publicationId":id,"assignmentId":id,"deliveryVersion":2,"deliveryRevision":revision,"spreadsheetId":"fixture-book","sheetGid":0,"reportingEpoch":"fixture-epoch","rowKind":"internalReport","reportVersion":1,"rowRevision":revision,"poster":"bot","postUrl":url,"postedAt":"2026-09-15T18:30:00Z","machine":"Máy 1","tiktokAccount":"@fixture","status":if url.is_empty(){"Đã gửi"}else{"Đã xác minh"},"stateNotes":"","partners":["Quán A"]})
}
fn scan(rows: &[Row], p: &Value) -> Scan {
    let mut scan = Scan::default();
    for row in rows {
        scan.observe(row.clone(), Some(p)).unwrap();
    }
    scan
}
fn apply(rows: &mut Vec<Row>, plan: &WritePlan) {
    while rows.len() <= plan.row as usize {
        rows.push(Row {
            index: rows.len() as u32,
            cells: vec![Cell::default(); plan.width],
        });
    }
    for request in &plan.requests {
        if let Some(update) = request.get("updateCells") {
            let index = update["start"]["rowIndex"].as_u64().unwrap() as usize;
            if index == 0 {
                continue;
            }
            let left = update["start"]["columnIndex"].as_u64().unwrap() as usize;
            rows[index].cells.resize(plan.width, Cell::default());
            for (offset, value) in update["rows"][0]["values"]
                .as_array()
                .unwrap()
                .iter()
                .enumerate()
            {
                let cell = &mut rows[index].cells[left + offset];
                if update["fields"] == "userEnteredValue" {
                    cell.user_entered_value = value["userEnteredValue"].clone();
                    cell.formatted_value = String::new();
                }
                if update["fields"] == "note" {
                    cell.note = value["note"].as_str().unwrap().into();
                }
            }
        }
    }
}
fn fixture() -> (Metadata, Layout, Vec<Row>) {
    let m = meta(true);
    let l = Layout::from_header(&m.header).unwrap();
    let rows = vec![
        Row {
            index: 0,
            cells: m.header.clone(),
        },
        Row {
            index: 1,
            cells: vec![Cell::default(); l.width],
        },
    ];
    (m, l, rows)
}

// Break caught: a legacy path must never accept the shared schema, even when
// its diagnostic writer UUID happens to match this installation.
#[test]
fn legacy_owner_match_refuses_shared_schema_even_for_original_writer() {
    let mut m = meta(true);
    m.owner.as_mut().unwrap().schema_version = 2;
    assert!(m
        .owner_matches("550e8400-e29b-41d4-a716-446655440000", "fixture-epoch")
        .is_err());
}

#[test]
fn committed_timeout_retry_and_restart_select_the_same_row() {
    let (m, l, mut rows) = fixture();
    let p = payload("publication-a", 1, "");
    validate_payload(
        &p,
        &crate::publish_sheet::SheetDeliveryTarget {
            version: 2,
            spreadsheet_id: m.spreadsheet_id.clone(),
            sheet_gid: 0,
            internal_reporting: true,
            reporting_epoch: Some("fixture-epoch".into()),
        },
        &l,
    )
    .unwrap();
    let plan = planner::plan(&m, &l, &scan(&rows[1..], &p), &p).unwrap();
    apply(&mut rows, &plan);
    // Response loss: no client receipt/cache is retained. Rebuild from persisted
    // authoritative cells/notes exactly as a restarted client scans Google.
    let retry = planner::plan(&m, &l, &scan(&rows[1..], &p), &p).unwrap();
    assert!(retry.requests.is_empty());
    assert_eq!(retry.row, 1);
    assert_eq!(receipt(&rows[1], &p, true).unwrap().row, 2);
    assert_eq!(rows[1].cells[0].display(), "1");
    assert_eq!(rows[1].cells[2].display(), "16/9/2026");
    // Independent Node JSON.stringify/SHA-256 oracle from the Apps Script
    // wire. STT is a numberValue in Google but a STRING in the row fingerprint.
    let note: Value = serde_json::from_str(&rows[1].cells[3].note).unwrap();
    assert_eq!(
        note["rowFingerprint"],
        "7237a0b0987d6518d0838b3aa596c0338287ee7bc2840de08211c2d34ecd9bab"
    );
    assert_eq!(
        note["payloadFingerprint"],
        "eaabe0c10cbfd91db8a8779529242f0fca5692df66b4103433f65501febe36d3"
    );
}

#[test]
fn failed_diagnostic_projects_only_exact_publication_note_without_raw_cell_data() {
    let (m, l, mut rows) = fixture();
    let url = "https://www.tiktok.com/@fixture/photo/111";
    let mut p = payload("diagnostic-id", 7, url);
    p["rowKind"] = json!("canonical");
    let write = planner::plan(&m, &l, &scan(&rows[1..], &p), &p).unwrap();
    apply(&mut rows, &write);
    let projection = planner::diagnose_row(
        &rows[1],
        "diagnostic-id",
        7,
        "fixture-book",
        0,
        "fixture-epoch",
        (url, Some("2026-09-15T18:30:00Z")),
    )
    .unwrap();
    assert_eq!(projection.row, 2);
    assert_eq!(projection.note_revision, Some(7));
    assert!(projection.identity_matches);
    assert!(projection.url_matches);
    assert!(!projection.d_cell_empty);
    assert!(projection.d_cell_has_canonical_url);
    assert!(projection.posted_at_matches);
    assert!(!serde_json::to_string(&projection).unwrap().contains(url));
    assert!(planner::diagnose_row(
        &rows[1],
        "other-id",
        7,
        "fixture-book",
        0,
        "fixture-epoch",
        (url, Some("2026-09-15T18:30:00Z"))
    )
    .is_none());
    let stale = planner::diagnose_row(
        &rows[1],
        "diagnostic-id",
        8,
        "fixture-book",
        0,
        "fixture-epoch",
        (url, Some("2026-09-15T18:30:00Z")),
    )
    .unwrap();
    assert_eq!(stale.note_revision, Some(7));
    assert!(!stale.revision_matches);
    let wrong_time = planner::diagnose_row(
        &rows[1],
        "diagnostic-id",
        7,
        "fixture-book",
        0,
        "fixture-epoch",
        (url, Some("2026-09-15T18:31:00Z")),
    )
    .unwrap();
    assert!(!wrong_time.posted_at_matches);
    let mut mismatched_note = rows[1].clone();
    let mut note: Value = serde_json::from_str(&mismatched_note.cells[3].note).unwrap();
    note["canonicalPostedAt"] = json!("2026-09-15T18:31:00Z");
    mismatched_note.cells[3].note = note.to_string();
    let mismatched = planner::diagnose_row(
        &mismatched_note,
        "diagnostic-id",
        7,
        "fixture-book",
        0,
        "fixture-epoch",
        (url, Some("2026-09-15T18:30:00Z")),
    )
    .unwrap();
    assert!(!mismatched.posted_at_matches);
}

#[test]
fn failed_diagnostic_distinguishes_empty_d_from_a_different_canonical_url() {
    let (m, l, mut rows) = fixture();
    let mut report = payload("diagnostic-id", 7, "");
    report["status"] = json!("Đã gửi");
    let write = planner::plan(&m, &l, &scan(&rows[1..], &report), &report).unwrap();
    apply(&mut rows, &write);
    let expected = "https://www.tiktok.com/@fixture/photo/111";
    let inspect = |row: &Row| {
        planner::diagnose_row(
            row,
            "diagnostic-id",
            7,
            "fixture-book",
            0,
            "fixture-epoch",
            (expected, Some("2026-09-15T18:30:00Z")),
        )
        .unwrap()
    };
    let empty = inspect(&rows[1]);
    assert!(empty.d_cell_empty);
    assert!(!empty.d_cell_has_canonical_url);
    assert!(empty.posted_at_matches);
    assert_eq!(empty.note_revision, None);
    rows[1].cells[3].formatted_value = "https://www.tiktok.com/@other/photo/222".into();
    let other = inspect(&rows[1]);
    assert!(!other.d_cell_empty);
    assert!(other.d_cell_has_canonical_url);
    assert!(!other.url_matches);
    assert!(!other.fingerprint_matches);
}

#[test]
fn late_links_and_reverse_revision_arrival_never_create_new_rows_or_clear_links() {
    let (m, l, mut rows) = fixture();
    for id in ["one", "two"] {
        let p = payload(id, 1, "");
        let write = planner::plan(&m, &l, &scan(&rows[1..], &p), &p).unwrap();
        apply(&mut rows, &write);
    }
    for (id, url) in [
        ("two", "https://www.tiktok.com/@fixture/video/222"),
        ("one", "https://www.tiktok.com/@fixture/photo/111"),
    ] {
        let mut p = payload(id, 7, url);
        p["rowKind"] = json!("canonical");
        p["deliveryRevision"] = json!(0);
        let write = planner::plan(&m, &l, &scan(&rows[1..], &p), &p).unwrap();
        apply(&mut rows, &write);
        assert_eq!(
            receipt(&rows[write.row as usize], &p, false)
                .unwrap()
                .post_url,
            url
        );
        let old = payload(id, 2, "");
        let retry = planner::plan(&m, &l, &scan(&rows[1..], &old), &old).unwrap();
        assert!(retry.requests.is_empty());
        assert_eq!(rows[write.row as usize].cells[3].display(), url);
    }
    assert_eq!(rows.len(), 3);
}

#[test]
fn presend_report_late_arrival_and_older_canonical_completion_preserve_newer_projection() {
    let (m, l, mut rows) = fixture();
    let initial = payload("one", 1, "");
    let write = planner::plan(&m, &l, &scan(&rows[1..], &initial), &initial).unwrap();
    apply(&mut rows, &write);
    let url = "https://www.tiktok.com/@fixture/video/111";
    let current = payload("one", 9, url);
    let write = planner::plan(&m, &l, &scan(&rows[1..], &current), &current).unwrap();
    apply(&mut rows, &write);
    let mut old = payload("one", 0, "");
    old["postedAt"] = Value::Null;
    old["status"] = json!("Chưa đăng");
    let plan = planner::plan(&m, &l, &scan(&rows[1..], &old), &old).unwrap();
    assert!(plan.requests.is_empty());
    let mut canonical = payload("one", 7, url);
    canonical["rowKind"] = json!("canonical");
    canonical["deliveryRevision"] = json!(0);
    let plan = planner::plan(&m, &l, &scan(&rows[1..], &canonical), &canonical).unwrap();
    assert_eq!(plan.requests.len(), 1);
    assert_eq!(plan.requests[0]["updateCells"]["fields"], "note");
    apply(&mut rows, &plan);
    assert_eq!(receipt(&rows[1], &current, true).unwrap().revision, 9);
    assert_eq!(receipt(&rows[1], &canonical, true).unwrap().revision, 0);
}

#[test]
fn older_canonical_completion_fills_empty_link_without_reverting_newer_report() {
    let (m, l, mut rows) = fixture();
    let mut report = payload("one", 9, "");
    report["machine"] = json!("Machine 9");
    report["stateNotes"] = json!("Newer report note");
    let write = planner::plan(&m, &l, &scan(&rows[1..], &report), &report).unwrap();
    apply(&mut rows, &write);
    let before = rows[1].clone();
    let before_note = before.note().unwrap();

    let url = "https://www.tiktok.com/@fixture/video/111";
    let mut canonical = payload("one", 7, url);
    canonical["rowKind"] = json!("canonical");
    canonical["deliveryRevision"] = json!(0);
    let write = planner::plan(&m, &l, &scan(&rows[1..], &canonical), &canonical).unwrap();
    assert_eq!(write.row, 1);
    assert_eq!(write.requests.len(), 2);
    assert_eq!(write.requests[0]["updateCells"]["start"]["columnIndex"], 3);
    assert_eq!(
        write.requests[0]["updateCells"]["fields"],
        "userEnteredValue"
    );
    assert_eq!(write.requests[1]["updateCells"]["start"]["columnIndex"], 3);
    assert_eq!(write.requests[1]["updateCells"]["fields"], "note");
    apply(&mut rows, &write);

    let after = &rows[1];
    assert_eq!(after.cells[3].display(), url);
    for column in (0..l.width).filter(|column| *column != 3) {
        assert_eq!(after.cells[column], before.cells[column]);
    }
    let after_note = after.note().unwrap();
    assert_eq!(after_note["rowRevision"], before_note["rowRevision"]);
    assert_eq!(
        after_note["payloadFingerprint"],
        before_note["payloadFingerprint"]
    );
    assert_eq!(after_note["canonicalDeliveryRevision"], 0);
    assert_ne!(after_note["rowFingerprint"], before_note["rowFingerprint"]);
    assert_eq!(receipt(after, &canonical, true).unwrap().post_url, url);
    assert_eq!(receipt(after, &report, true).unwrap().revision, 9);
    let retry = planner::plan(&m, &l, &scan(&rows[1..], &canonical), &canonical).unwrap();
    assert!(retry.requests.is_empty());
}

#[test]
fn older_canonical_completion_rejects_a_different_stored_account() {
    let (m, l, mut rows) = fixture();
    let mut report = payload("one", 9, "");
    report["tiktokAccount"] = json!("@other");
    let write = planner::plan(&m, &l, &scan(&rows[1..], &report), &report).unwrap();
    apply(&mut rows, &write);

    let mut canonical = payload("one", 7, "https://www.tiktok.com/@fixture/video/111");
    canonical["rowKind"] = json!("canonical");
    canonical["deliveryRevision"] = json!(0);
    assert!(planner::plan(&m, &l, &scan(&rows[1..], &canonical), &canonical).is_err());
}

#[test]
fn immutable_identity_and_manual_changes_fail_before_writes() {
    let (m, l, mut rows) = fixture();
    let p = payload("one", 1, "");
    let write = planner::plan(&m, &l, &scan(&rows[1..], &p), &p).unwrap();
    apply(&mut rows, &write);
    for key in ["poster", "postedAt", "partners"] {
        let mut changed = p.clone();
        changed["rowRevision"] = json!(2);
        changed["deliveryRevision"] = json!(2);
        changed[key] = if key == "partners" {
            json!(["different"])
        } else {
            json!("different")
        };
        assert!(planner::plan(&m, &l, &scan(&rows[1..], &changed), &changed).is_err());
    }
    let mut changed = rows.clone();
    changed[1].cells[1].user_entered_value = json!({"formulaValue":"=IMPORTXML(\"x\")"});
    assert!(planner::plan(&m, &l, &scan(&changed[1..], &p), &p).is_err());
    let mut changed = rows.clone();
    changed[1].cells[8].formatted_value = "human edit".into();
    assert!(planner::plan(&m, &l, &scan(&changed[1..], &p), &p).is_err());
    let mut duplicate = Scan::default();
    duplicate.observe(rows[1].clone(), Some(&p)).unwrap();
    assert!(duplicate.observe(rows[1].clone(), Some(&p)).is_err());
    let mut broken = rows[1].clone();
    broken.cells[3].note = "{\"kind\":\"riviu-publish\",broken".into();
    assert!(Scan::default().observe(broken, Some(&p)).is_err());
}

#[test]
fn partner_expansion_preserves_unmanaged_columns_and_notes() {
    let (m, l, mut rows) = fixture();
    rows[1].cells[0].note = "user annotation".into();
    let mut p = payload("one", 1, "");
    p["partners"] = json!(["=literal", "Quán B", "Quán C"]);
    let write = planner::plan(&m, &l, &scan(&rows[1..], &p), &p).unwrap();
    assert_eq!(write.row, 2, "annotated empty row is not silently adopted");
    assert_eq!(
        write.requests[0]["insertDimension"]["range"]["startIndex"],
        9
    );
    assert_eq!(
        write.requests[0]["insertDimension"]["range"]["endIndex"],
        11
    );
    assert!(write
        .requests
        .iter()
        .filter_map(|r| r.get("updateCells"))
        .all(|r| r["fields"] == "userEnteredValue" || r["fields"] == "note"));
    apply(&mut rows, &write);
    assert_eq!(
        rows[2].cells[8].user_entered_value["stringValue"],
        "=literal"
    );
    assert_eq!(rows[1].cells[0].note, "user annotation");
}

#[test]
fn canonical_account_epoch_and_layout_mismatches_reject() {
    let (m, l, _) = fixture();
    let t = crate::publish_sheet::SheetDeliveryTarget {
        version: 2,
        spreadsheet_id: m.spreadsheet_id.clone(),
        sheet_gid: 0,
        internal_reporting: true,
        reporting_epoch: Some("fixture-epoch".into()),
    };
    let mut p = payload("one", 1, "https://www.tiktok.com/@wrong/video/111");
    assert!(validate_payload(&p, &t, &l).is_err());
    p["postUrl"] = json!("");
    p["status"] = json!("Đã gửi");
    p["reportingEpoch"] = json!("old");
    assert!(validate_payload(&p, &t, &l).is_err());
    assert!(m
        .owner_matches("another-installation", "fixture-epoch")
        .is_err());
    assert!(m
        .owner_matches("550e8400-e29b-41d4-a716-446655440000", "old")
        .is_err());
    let mut header = m.header;
    header[4].formatted_value = "wrong".into();
    assert!(Layout::from_header(&header).is_err());
}

#[test]
fn direct_http_errors_are_typed_without_reflecting_credentials() {
    for (status, kind, retry) in [
        (401, DirectSheetsErrorKind::Unauthorized, false),
        (403, DirectSheetsErrorKind::Forbidden, false),
        (404, DirectSheetsErrorKind::NotFound, false),
        (429, DirectSheetsErrorKind::RateLimited, true),
        (503, DirectSheetsErrorKind::Server, true),
    ] {
        let e = DirectSheetsError::http(status);
        assert_eq!(e.kind, kind);
        assert_eq!(e.retryable(), retry);
        assert_eq!(e.status, Some(status));
    }
    assert_ne!(writer_metadata_id(0), writer_metadata_id(1));
    assert!(writer_metadata_id(0) > 0);
}

#[tokio::test(start_paused = true)]
async fn quota_spacing_serializes_concurrent_starts_without_using_http_slots() {
    let clock = tokio::sync::Mutex::new(None);
    let start = tokio::time::Instant::now();
    let ((), (), ()) = tokio::join!(
        pace_requests(&clock),
        pace_requests(&clock),
        pace_requests(&clock)
    );
    assert_eq!(start.elapsed(), Duration::from_secs(2));
}

struct Server {
    origin: String,
    rows: std::sync::Arc<parking_lot::Mutex<Vec<Row>>>,
    writes: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    lose_response: std::sync::Arc<std::sync::atomic::AtomicBool>,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl Server {
    async fn start() -> Self {
        use std::sync::{
            atomic::{AtomicBool, AtomicUsize, Ordering},
            Arc,
        };
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let (m, l, rows) = fixture();
        let rows = Arc::new(parking_lot::Mutex::new(rows));
        let shared = rows.clone();
        let writes = Arc::new(AtomicUsize::new(0));
        let written = writes.clone();
        let lose_response = Arc::new(AtomicBool::new(false));
        let lost = lose_response.clone();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let task = tokio::spawn(async move {
            loop {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut data = Vec::new();
                let (body_start, length) = loop {
                    let mut buffer = [0; 8192];
                    let n = socket.read(&mut buffer).await.unwrap();
                    if n == 0 {
                        break (0, 0);
                    }
                    data.extend_from_slice(&buffer[..n]);
                    if let Some(end) = data.windows(4).position(|p| p == b"\r\n\r\n") {
                        let header = String::from_utf8_lossy(&data[..end]);
                        let length = header
                            .lines()
                            .find_map(|s| {
                                s.to_lowercase()
                                    .strip_prefix("content-length:")
                                    .and_then(|v| v.trim().parse::<usize>().ok())
                            })
                            .unwrap_or(0);
                        if data.len() >= end + 4 + length {
                            break (end + 4, length);
                        }
                    }
                };
                if body_start == 0 {
                    continue;
                }
                let header = String::from_utf8_lossy(&data[..body_start]);
                assert!(header
                    .to_lowercase()
                    .contains("authorization: bearer fixture-token"));
                let target = header
                    .lines()
                    .next()
                    .unwrap()
                    .split_whitespace()
                    .nth(1)
                    .unwrap();
                let url = url::Url::parse(&format!("http://localhost{target}")).unwrap();
                let range = url
                    .query_pairs()
                    .find(|(k, _)| k == "ranges")
                    .map(|(_, v)| v.into_owned());
                let response = if header.starts_with("POST ") {
                    let body: Value =
                        serde_json::from_slice(&data[body_start..body_start + length]).unwrap();
                    let requests = body["requests"].as_array().unwrap().clone();
                    let index = requests
                        .iter()
                        .filter_map(|r| r["updateCells"]["start"]["rowIndex"].as_u64())
                        .find(|n| *n > 0)
                        .unwrap() as u32;
                    let write = WritePlan {
                        row: index,
                        width: l.width,
                        before: Row {
                            index,
                            cells: vec![],
                        },
                        requests,
                        duplicate: false,
                    };
                    apply(&mut shared.lock(), &write);
                    written.fetch_add(1, Ordering::SeqCst);
                    if lost.swap(false, Ordering::SeqCst) {
                        continue;
                    }
                    json!({"spreadsheetId":"fixture-book","replies":[{},{}]})
                } else if let Some(range) = range {
                    let coords = range.split('!').next_back().unwrap();
                    let nums: Vec<u32> = coords
                        .split(|c: char| !c.is_ascii_digit())
                        .filter(|v| !v.is_empty())
                        .map(|s| s.parse().unwrap())
                        .collect();
                    let start = nums[0] - 1;
                    let end = nums[1];
                    let row_data = (start..end)
                        .map(|n| {
                            let cells = shared
                                .lock()
                                .get(n as usize)
                                .map(|r| r.cells.clone())
                                .unwrap_or_else(|| vec![Cell::default(); l.width]);
                            json!({"values":cells})
                        })
                        .collect::<Vec<_>>();
                    json!({"spreadsheetId":"fixture-book","sheets":[{"properties":{"sheetId":0},"data":[{"startRow":start,"startColumn":0,"rowData":row_data}]}]})
                } else {
                    json!({"spreadsheetId":"fixture-book","properties":{"timeZone":"Asia/Ho_Chi_Minh"},"sheets":[{"properties":{"sheetId":0,"title":"Fixture","gridProperties":{"rowCount":20,"columnCount":9}},"developerMetadata":[{"metadataId":writer_metadata_id(0),"metadataKey":WRITER_METADATA_KEY,"metadataValue":serde_json::to_string(m.owner.as_ref().unwrap()).unwrap(),"location":{"sheetId":0}}]}]})
                };
                let body = response.to_string();
                let reply=format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len());
                socket.write_all(reply.as_bytes()).await.unwrap();
            }
        });
        Self {
            origin,
            rows,
            writes,
            lose_response,
            task,
        }
    }
    fn client(&self) -> DirectSheetsClient {
        let mut c = DirectSheetsClient::new("fixture-token").unwrap();
        c.test_origin = Some(self.origin.clone());
        c
    }
}

#[tokio::test]
async fn actual_transport_lost_commit_response_restarts_and_reads_existing_receipt() {
    use std::sync::atomic::Ordering;
    let server = Server::start().await;
    let target = SheetDeliveryTarget {
        version: 2,
        spreadsheet_id: "fixture-book".into(),
        sheet_gid: 0,
        internal_reporting: true,
        reporting_epoch: Some("fixture-epoch".into()),
    };
    let p = payload("publication-network", 1, "");
    let writer = "550e8400-e29b-41d4-a716-446655440000";
    server.lose_response.store(true, Ordering::SeqCst);
    let error = server
        .client()
        .deliver(&target, &p, writer)
        .await
        .unwrap_err();
    assert_eq!(error.kind, DirectSheetsErrorKind::Transport);
    assert_eq!(server.writes.load(Ordering::SeqCst), 1);
    let receipt = server.client().deliver(&target, &p, writer).await.unwrap();
    assert_eq!(receipt.row, 2);
    assert!(receipt.duplicate);
    assert_eq!(server.writes.load(Ordering::SeqCst), 1);
    // Moving the row between calls invalidates any remembered ordinal. Every
    // fresh client rebinds the note and acknowledges its actual new row.
    {
        let mut rows = server.rows.lock();
        let mut moved = rows[1].clone();
        moved.index = 3;
        rows[1] = Row {
            index: 1,
            cells: vec![Cell::default(); 9],
        };
        rows.push(Row {
            index: 2,
            cells: vec![Cell::default(); 9],
        });
        rows.push(moved);
    }
    let moved = server.client().deliver(&target, &p, writer).await.unwrap();
    assert_eq!(moved.row, 4);
    assert_eq!(server.writes.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn failed_diagnostic_reads_only_google_rows_and_reports_exact_d_cell_match() {
    use std::sync::atomic::Ordering;
    let server = Server::start().await;
    let (m, l, mut rows) = fixture();
    let url = "https://www.tiktok.com/@fixture/photo/111";
    let mut p = payload("diagnostic-id", 7, url);
    p["rowKind"] = json!("canonical");
    let write = planner::plan(&m, &l, &scan(&rows[1..], &p), &p).unwrap();
    apply(&mut rows, &write);
    *server.rows.lock() = rows;
    let target = SheetDeliveryTarget {
        version: 2,
        spreadsheet_id: "fixture-book".into(),
        sheet_gid: 0,
        internal_reporting: true,
        reporting_epoch: Some("fixture-epoch".into()),
    };
    let result = server
        .client()
        .diagnose_failed(
            &target,
            "diagnostic-id",
            7,
            url,
            Some("2026-09-15T18:30:00Z"),
            2,
        )
        .await
        .unwrap();
    assert!(result.complete);
    assert_eq!(result.matches.len(), 1);
    assert!(result.matches[0].url_matches);
    assert_eq!(server.writes.load(Ordering::SeqCst), 0);

    {
        let mut duplicate = server.rows.lock();
        let mut second = duplicate[1].clone();
        second.index = 2;
        duplicate.push(second);
    }
    let result = server
        .client()
        .diagnose_failed(
            &target,
            "diagnostic-id",
            7,
            url,
            Some("2026-09-15T18:30:00Z"),
            2,
        )
        .await
        .unwrap();
    assert_eq!(result.matches.len(), 2);
    assert!(result.duplicate_found);
    assert_eq!(server.writes.load(Ordering::SeqCst), 0);
}
