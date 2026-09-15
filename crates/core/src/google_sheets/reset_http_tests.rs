use super::*;
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Arc,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

struct Fixture {
    origin: String,
    copies: Arc<AtomicUsize>,
    clears: Arc<AtomicUsize>,
    lose_clear: Arc<AtomicBool>,
    state: Arc<parking_lot::Mutex<State>>,
    worker: tokio::task::JoinHandle<()>,
}
struct State {
    owner: Option<Value>,
    rows: Vec<Vec<Value>>,
    backup: Option<Vec<Vec<Value>>>,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.worker.abort();
    }
}
impl Fixture {
    async fn start() -> Self {
        let headers = Layout::standard(true).headers;
        let rows = vec![
            headers
                .iter()
                .map(|s| json!({"formattedValue":s,"userEnteredValue":{"stringValue":s}}))
                .collect(),
            vec![
                json!({"formattedValue":"41","userEnteredValue":{"numberValue":41},"note":"user note"}),
                json!({"formattedValue":"value","userEnteredValue":{"formulaValue":"=1+2"},"userEnteredFormat":{"backgroundColor":{"red":1}}}),
            ],
        ];
        let state = Arc::new(parking_lot::Mutex::new(State {
            owner: None,
            rows,
            backup: None,
        }));
        let data = state.clone();
        let copies = Arc::new(AtomicUsize::new(0));
        let nc = copies.clone();
        let clears = Arc::new(AtomicUsize::new(0));
        let cl = clears.clone();
        let lose_clear = Arc::new(AtomicBool::new(false));
        let loss = lose_clear.clone();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let worker = tokio::spawn(async move {
            loop {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut input = Vec::new();
                let (end, len) = loop {
                    let mut b = [0; 8192];
                    let n = stream.read(&mut b).await.unwrap();
                    if n == 0 {
                        break (0, 0);
                    }
                    input.extend_from_slice(&b[..n]);
                    if let Some(i) = input.windows(4).position(|w| w == b"\r\n\r\n") {
                        let h = String::from_utf8_lossy(&input[..i]);
                        let len = h
                            .lines()
                            .find_map(|l| {
                                l.to_lowercase()
                                    .strip_prefix("content-length:")
                                    .and_then(|v| v.trim().parse::<usize>().ok())
                            })
                            .unwrap_or(0);
                        if input.len() >= i + 4 + len {
                            break (i + 4, len);
                        }
                    }
                };
                if end == 0 {
                    continue;
                }
                let header = String::from_utf8_lossy(&input[..end]);
                let uri = header
                    .lines()
                    .next()
                    .unwrap()
                    .split_whitespace()
                    .nth(1)
                    .unwrap();
                let url = url::Url::parse(&format!("http://localhost{uri}")).unwrap();
                assert!(header
                    .to_lowercase()
                    .contains("authorization: bearer fixture-token"));
                let mut status = 200;
                let mut drop_response = false;
                let response = if url.path().starts_with("/drive/") {
                    let mut data = data.lock();
                    data.backup = Some(data.rows.clone());
                    nc.fetch_add(1, Ordering::SeqCst);
                    json!({"id":"backup-book","mimeType":"application/vnd.google-apps.spreadsheet"})
                } else if header.starts_with("POST ") {
                    let body: Value = serde_json::from_slice(&input[end..end + len]).unwrap();
                    let mut data = data.lock();
                    for request in body["requests"].as_array().unwrap() {
                        if let Some(v) = request.get("createDeveloperMetadata") {
                            if data.owner.is_some() {
                                status = 400;
                                break;
                            }
                            data.owner = Some(v["developerMetadata"].clone());
                        } else if let Some(v) = request.get("updateDeveloperMetadata") {
                            data.owner.as_mut().unwrap()["metadataValue"] =
                                v["developerMetadata"]["metadataValue"].clone();
                        } else if request["updateCells"].get("range").is_some() {
                            assert_eq!(request["updateCells"]["range"]["sheetId"], 0);
                            assert_eq!(request["updateCells"]["range"]["startRowIndex"], 1);
                            for row in data.rows.iter_mut().skip(1) {
                                for cell in row {
                                    if let Some(o) = cell.as_object_mut() {
                                        o.remove("userEnteredValue");
                                        o.remove("formattedValue");
                                        o.remove("note");
                                    }
                                }
                            }
                            cl.fetch_add(1, Ordering::SeqCst);
                            drop_response = loss.swap(false, Ordering::SeqCst);
                        } else {
                            panic!("unexpected fixture request {request}");
                        }
                    }
                    json!({"spreadsheetId":"fixture-book","replies":[{}]})
                } else {
                    let backup = url.path().contains("backup-book");
                    let book = if backup {
                        "backup-book"
                    } else {
                        "fixture-book"
                    };
                    let data = data.lock();
                    let rows = if backup {
                        data.backup.as_ref().unwrap()
                    } else {
                        &data.rows
                    };
                    let ranges = url
                        .query_pairs()
                        .find(|(k, _)| k == "ranges")
                        .map(|(_, v)| v.into_owned());
                    if let Some(range) = ranges {
                        let coords = range.split('!').next_back().unwrap();
                        let ns = coords
                            .split(|c: char| !c.is_ascii_digit())
                            .filter(|s| !s.is_empty())
                            .map(|s| s.parse::<usize>().unwrap())
                            .collect::<Vec<_>>();
                        let start = ns[0] - 1;
                        let end = ns[1];
                        json!({"spreadsheetId":book,"sheets":[{"properties":{"sheetId":0},"data":[{"startRow":start,"startColumn":0,"rowData":(start..end).map(|i|json!({"values":rows.get(i).cloned().unwrap_or_default()})).collect::<Vec<_>>()}]}]})
                    } else {
                        let fields = url
                            .query_pairs()
                            .find(|(k, _)| k == "fields")
                            .map(|(_, v)| v.into_owned())
                            .unwrap_or_default();
                        let mut properties = json!({"title":"Fixture","gridProperties":{"rowCount":4,"columnCount":9}});
                        if fields.contains("sheetId") {
                            properties["sheetId"] = json!(0);
                        }
                        json!({"spreadsheetId":book,"properties":{"timeZone":"UTC"},"sheets":[{"properties":properties,"developerMetadata":data.owner.iter().cloned().collect::<Vec<_>>()}]})
                    }
                };
                if drop_response {
                    continue;
                }
                let body = response.to_string();
                stream.write_all(format!("HTTP/1.1 {status} Status\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).as_bytes()).await.unwrap();
            }
        });
        Self {
            origin,
            copies,
            clears,
            lose_clear,
            state,
            worker,
        }
    }
    fn client(&self) -> DirectSheetsClient {
        let mut c = DirectSheetsClient::new("fixture-token").unwrap();
        c.test_origin = Some(self.origin.clone());
        c
    }
}
fn target() -> SheetDeliveryTarget {
    SheetDeliveryTarget {
        version: 2,
        spreadsheet_id: "fixture-book".into(),
        sheet_gid: 0,
        internal_reporting: true,
        reporting_epoch: Some("fixture-epoch".into()),
    }
}
const WRITER: &str = "550e8400-e29b-41d4-a716-446655440000";

#[tokio::test]
async fn prepare_then_lost_clear_response_resumes_same_backup_and_fences_old_epoch() {
    let f = Fixture::start().await;
    let client = f.client();
    let t = target();
    let prepared = client.prepare_target(&t, WRITER).await.unwrap();
    assert!(prepared.reporting_ready);
    assert_eq!(prepared.reporting_epoch.as_deref(), Some("fixture-epoch"));
    assert!(client
        .prepare_target(&t, "650e8400-e29b-41d4-a716-446655440000")
        .await
        .is_err());
    let reset = "750e8400-e29b-41d4-a716-446655440000";
    f.lose_clear.store(true, Ordering::SeqCst);
    let err = client.reset_target(&t, WRITER, reset).await.unwrap_err();
    assert_eq!(err.kind, DirectSheetsErrorKind::Transport);
    assert_eq!(f.copies.load(Ordering::SeqCst), 1);
    assert_eq!(f.clears.load(Ordering::SeqCst), 1);
    assert!(
        !client
            .check_target("fixture-book", 0)
            .await
            .unwrap()
            .reporting_ready
    );
    assert!(client
        .reset_target(&t, WRITER, "850e8400-e29b-41d4-a716-446655440000")
        .await
        .is_err());
    let done = f.client().reset_target(&t, WRITER, reset).await.unwrap();
    assert!(done.complete);
    assert_eq!(done.backup_spreadsheet_id, "backup-book");
    assert_eq!(f.copies.load(Ordering::SeqCst), 1);
    let checks = client.check_target("fixture-book", 0).await.unwrap();
    assert!(checks.reporting_ready);
    assert_eq!(checks.reporting_epoch.as_deref(), Some(reset));
    let old = super::tests::payload("old", 1, "");
    assert_eq!(
        client.deliver(&t, &old, WRITER).await.unwrap_err().kind,
        DirectSheetsErrorKind::Conflict
    );
    let before = f.clears.load(Ordering::SeqCst);
    client.reset_target(&t, WRITER, reset).await.unwrap();
    assert_eq!(f.clears.load(Ordering::SeqCst), before);
    let state = f.state.lock();
    assert_eq!(state.rows[0][0]["formattedValue"], "STT");
    assert_eq!(
        state.rows[1][1]["userEnteredFormat"]["backgroundColor"]["red"],
        1
    );
    assert_eq!(
        state.backup.as_ref().unwrap()[1][1]["userEnteredValue"]["formulaValue"],
        "=1+2"
    );
}
