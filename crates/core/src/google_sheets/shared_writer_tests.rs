use super::*;
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Arc,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const WRITER: &str = "550e8400-e29b-41d4-a716-446655440000";
const OTHER: &str = "550e8400-e29b-41d4-a716-446655440001";

#[tokio::test]
async fn same_database_writers_wait_without_abandoning_the_live_operation() {
    let server = Server::start(2).await;
    server.hold_scan.store(true, Ordering::SeqCst);
    let path = std::env::temp_dir().join(format!("shared-local-gate-{}.db", uuid::Uuid::new_v4()));
    let first_db = crate::db::Database::open(&path).unwrap();
    let second_db = crate::db::Database::open(&path).unwrap();
    let first_client = server.client();
    let first = tokio::spawn(async move {
        first_client
            .deliver_shared(
                &target(),
                &tests::payload("first-local", 1, ""),
                WRITER,
                &first_db,
            )
            .await
    });
    tokio::time::timeout(Duration::from_secs(5), server.scan_entered.notified())
        .await
        .unwrap();
    let second_client = server.client();
    let mut second = tokio::spawn(async move {
        second_client
            .deliver_shared(
                &target(),
                &tests::payload("second-local", 1, ""),
                WRITER,
                &second_db,
            )
            .await
    });
    let concurrent = tokio::select! {
        result = &mut second => Some(format!("second completed while first scan was held: {result:?}")),
        _ = server.scan_entered.notified() => Some("second abandoned the live lock and started a concurrent scan".to_owned()),
        _ = tokio::time::sleep(Duration::from_millis(300)) => None,
    };
    server.hold_scan.store(false, Ordering::SeqCst);
    server.resume_scan.notify_waiters();
    if let Some(reason) = concurrent {
        first.abort();
        second.abort();
        panic!("{reason}");
    }
    assert_eq!(first.await.unwrap().unwrap().row, 2);
    assert_eq!(second.await.unwrap().unwrap().row, 3);
    assert_eq!(server.mutations.load(Ordering::SeqCst), 2);
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
fn database() -> crate::db::Database {
    crate::db::Database::open(
        std::env::temp_dir().join(format!("shared-http-{}.db", uuid::Uuid::new_v4())),
    )
    .unwrap()
}
#[derive(Clone)]
struct Remote {
    metadata: Vec<Value>,
    rows: Vec<Vec<Cell>>,
    columns: usize,
}
struct Server {
    origin: String,
    state: Arc<parking_lot::Mutex<Remote>>,
    lose_acquire: Arc<AtomicBool>,
    lose_mutation: Arc<AtomicBool>,
    lose_release: Arc<AtomicBool>,
    reject_mutation: Arc<AtomicBool>,
    reject_status: Arc<AtomicUsize>,
    reject_acquire: Arc<AtomicUsize>,
    mutations: Arc<AtomicUsize>,
    hold_scan: Arc<AtomicBool>,
    scan_entered: Arc<tokio::sync::Notify>,
    resume_scan: Arc<tokio::sync::Notify>,
    gate_acquires: Arc<AtomicBool>,
    creates: Arc<AtomicUsize>,
    lose_reconcile: Arc<AtomicBool>,
    replay_delete: Arc<parking_lot::Mutex<Option<Value>>>,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl Server {
    async fn start(schema: u32) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let layout = Layout::standard(true);
        let state = Arc::new(parking_lot::Mutex::new(Remote {
            metadata: vec![
                json!({"metadataId":writer_metadata_id(0),"metadataKey":WRITER_METADATA_KEY,"metadataValue":json!({"schemaVersion":schema,"writerId":WRITER,"reportingEpoch":"fixture-epoch","state":"ready"}).to_string(),"location":{"sheetId":0},"visibility":"DOCUMENT"}),
            ],
            rows: vec![layout
                .headers
                .iter()
                .map(|s| Cell {
                    user_entered_value: json!({"stringValue":s}),
                    ..Cell::default()
                })
                .collect()],
            columns: 9,
        }));
        let lose_acquire = Arc::new(AtomicBool::new(false));
        let lose_mutation = Arc::new(AtomicBool::new(false));
        let lose_release = Arc::new(AtomicBool::new(false));
        let reject_mutation = Arc::new(AtomicBool::new(false));
        let reject_status = Arc::new(AtomicUsize::new(0));
        let rs = reject_status.clone();
        let reject_acquire = Arc::new(AtomicUsize::new(0));
        let ra = reject_acquire.clone();
        let mutations = Arc::new(AtomicUsize::new(0));
        let hold_scan = Arc::new(AtomicBool::new(false));
        let scan_entered = Arc::new(tokio::sync::Notify::new());
        let resume_scan = Arc::new(tokio::sync::Notify::new());
        let acquire_gate = Arc::new(tokio::sync::Barrier::new(2));
        let gate_acquires = Arc::new(AtomicBool::new(false));
        let creates = Arc::new(AtomicUsize::new(0));
        let lose_reconcile = Arc::new(AtomicBool::new(false));
        let lr = lose_reconcile.clone();
        let replay_delete = Arc::new(parking_lot::Mutex::new(None::<Value>));
        let (g, ga, cc, rd) = (
            acquire_gate.clone(),
            gate_acquires.clone(),
            creates.clone(),
            replay_delete.clone(),
        );
        let (s, a, m, r, x, n, h, e, c) = (
            state.clone(),
            lose_acquire.clone(),
            lose_mutation.clone(),
            lose_release.clone(),
            reject_mutation.clone(),
            mutations.clone(),
            hold_scan.clone(),
            scan_entered.clone(),
            resume_scan.clone(),
        );
        let task = tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else {
                    break;
                };
                let (s, a, m, r, x, n, h, e, c) = (
                    s.clone(),
                    a.clone(),
                    m.clone(),
                    r.clone(),
                    x.clone(),
                    n.clone(),
                    h.clone(),
                    e.clone(),
                    c.clone(),
                );
                let (g, ga, cc, rd) = (g.clone(), ga.clone(), cc.clone(), rd.clone());
                let lr = lr.clone();
                let rs = rs.clone();
                let ra = ra.clone();
                tokio::spawn(async move {
                    let mut bytes = Vec::new();
                    let (body_start, len) = loop {
                        let mut buf = [0; 8192];
                        let Ok(size) = socket.read(&mut buf).await else {
                            return;
                        };
                        if size == 0 {
                            return;
                        }
                        bytes.extend_from_slice(&buf[..size]);
                        if let Some(end) = bytes.windows(4).position(|b| b == b"\r\n\r\n") {
                            let headers = String::from_utf8_lossy(&bytes[..end]).to_lowercase();
                            let len = headers
                                .lines()
                                .find_map(|l| {
                                    l.strip_prefix("content-length:")
                                        .and_then(|v| v.trim().parse::<usize>().ok())
                                })
                                .unwrap_or(0);
                            if bytes.len() >= end + 4 + len {
                                break (end + 4, len);
                            }
                        }
                    };
                    let headers = String::from_utf8_lossy(&bytes[..body_start]);
                    let url = url::Url::parse(&format!(
                        "http://localhost{}",
                        headers
                            .lines()
                            .next()
                            .unwrap()
                            .split_whitespace()
                            .nth(1)
                            .unwrap()
                    ))
                    .unwrap();
                    if headers.starts_with("GET ")
                        && lr.load(Ordering::SeqCst)
                        && (n.load(Ordering::SeqCst) > 0
                            || cc.load(Ordering::SeqCst) > 0
                                && ra.load(Ordering::SeqCst) == usize::MAX)
                    {
                        return;
                    }
                    let range = url
                        .query_pairs()
                        .find(|(k, _)| k == "ranges")
                        .map(|(_, v)| v.into_owned());
                    if range.as_ref().is_some_and(|r| r.contains("!A2:"))
                        && h.swap(false, Ordering::SeqCst)
                    {
                        e.notify_one();
                        c.notified().await;
                    }
                    let mut status = 200;
                    let mut lose = false;
                    let response = if headers.starts_with("POST ") {
                        let body: Value =
                            serde_json::from_slice(&bytes[body_start..body_start + len]).unwrap();
                        let requests = body["requests"].as_array().unwrap();
                        let acquire = requests.iter().any(|r| {
                            r["createDeveloperMetadata"]["developerMetadata"]["metadataKey"]
                                == shared_writer::LOCK_KEY
                        });
                        let release = requests
                            .iter()
                            .any(|r| r.get("deleteDeveloperMetadata").is_some());
                        let mutation = !acquire && !release;
                        if acquire && ga.load(Ordering::SeqCst) {
                            g.wait().await;
                            ga.store(false, Ordering::SeqCst);
                        }
                        if acquire {
                            cc.fetch_add(1, Ordering::SeqCst);
                        }
                        let mut state = s.lock();
                        let mut next = state.clone();
                        let rejection = if mutation {
                            rs.swap(0, Ordering::SeqCst)
                        } else if acquire {
                            let code = ra.load(Ordering::SeqCst);
                            if code > 0 && code < 600 {
                                ra.store(usize::MAX, Ordering::SeqCst);
                                if code != 400 {
                                    lr.store(true, Ordering::SeqCst);
                                }
                                code
                            } else {
                                0
                            }
                        } else {
                            0
                        };
                        if rejection != 0 {
                            status = rejection;
                        } else if mutation && x.swap(false, Ordering::SeqCst) {
                            lose = true;
                        } else {
                            for request in requests {
                                if let Some(v) = request.get("createDeveloperMetadata") {
                                    let md = &v["developerMetadata"];
                                    if next
                                        .metadata
                                        .iter()
                                        .any(|old| old["metadataId"] == md["metadataId"])
                                    {
                                        status = 400;
                                        break;
                                    }
                                    next.metadata.push(md.clone());
                                } else if let Some(v) = request.get("updateDeveloperMetadata") {
                                    for old in &mut next.metadata {
                                        if matches_filter(
                                            old,
                                            &v["dataFilters"][0]["developerMetadataLookup"],
                                        ) {
                                            old["metadataValue"] =
                                                v["developerMetadata"]["metadataValue"].clone();
                                        }
                                    }
                                } else if let Some(v) = request.get("deleteDeveloperMetadata") {
                                    *rd.lock() = Some(v["dataFilter"].clone());
                                    next.metadata.retain(|old| {
                                        !matches_filter(
                                            old,
                                            &v["dataFilter"]["developerMetadataLookup"],
                                        )
                                    });
                                } else if let Some(v) = request.get("insertDimension") {
                                    let start = v["range"]["startIndex"].as_u64().unwrap() as usize;
                                    let end = v["range"]["endIndex"].as_u64().unwrap() as usize;
                                    next.columns += end - start;
                                    for row in &mut next.rows {
                                        row.resize(start.max(row.len()), Cell::default());
                                        for _ in start..end {
                                            row.insert(start, Cell::default());
                                        }
                                    }
                                } else if let Some(v) = request.get("appendDimension") {
                                    if v["dimension"] == "COLUMNS" {
                                        next.columns += v["length"].as_u64().unwrap() as usize;
                                    }
                                } else if let Some(v) = request.get("updateCells") {
                                    let top = v["start"]["rowIndex"].as_u64().unwrap() as usize;
                                    let left = v["start"]["columnIndex"].as_u64().unwrap() as usize;
                                    for (dy, rr) in v["rows"].as_array().unwrap().iter().enumerate()
                                    {
                                        while next.rows.len() <= top + dy {
                                            next.rows.push(vec![])
                                        }
                                        let values = rr["values"].as_array().unwrap();
                                        let row = &mut next.rows[top + dy];
                                        row.resize(
                                            row.len().max(left + values.len()),
                                            Cell::default(),
                                        );
                                        for (dx, cell) in values.iter().enumerate() {
                                            if v["fields"] == "userEnteredValue" {
                                                row[left + dx].user_entered_value =
                                                    cell["userEnteredValue"].clone();
                                                row[left + dx].formatted_value.clear();
                                            }
                                            if v["fields"] == "note" {
                                                row[left + dx].note =
                                                    cell["note"].as_str().unwrap().into();
                                            }
                                        }
                                    }
                                } else {
                                    panic!("unhandled batch request {request}")
                                }
                            }
                            if status == 200 {
                                if acquire {
                                    if let Some(filter) = rd.lock().take() {
                                        next.metadata.retain(|md| {
                                            !matches_filter(md, &filter["developerMetadataLookup"])
                                        });
                                    }
                                }
                                *state = next;
                                if mutation {
                                    n.fetch_add(1, Ordering::SeqCst);
                                }
                            }
                            lose = if acquire {
                                a.swap(false, Ordering::SeqCst)
                            } else if release {
                                r.swap(false, Ordering::SeqCst)
                            } else {
                                m.swap(false, Ordering::SeqCst)
                            };
                        }
                        json!({"spreadsheetId":"fixture-book"})
                    } else if let Some(range) = range {
                        let nums: Vec<usize> = range
                            .split('!')
                            .next_back()
                            .unwrap()
                            .split(|c: char| !c.is_ascii_digit())
                            .filter(|v| !v.is_empty())
                            .map(|v| v.parse().unwrap())
                            .collect();
                        let state = s.lock();
                        let rows=(nums[0]-1..nums[1]).map(|i|json!({"values":state.rows.get(i).cloned().unwrap_or_default()})).collect::<Vec<_>>();
                        json!({"spreadsheetId":"fixture-book","sheets":[{"properties":{"sheetId":0},"data":[{"startRow":nums[0]-1,"startColumn":0,"rowData":rows}]}]})
                    } else {
                        let state = s.lock();
                        json!({"spreadsheetId":"fixture-book","properties":{"title":"Shared fixture","timeZone":"Asia/Ho_Chi_Minh"},"sheets":[{"properties":{"sheetId":0,"title":"Fixture","gridProperties":{"rowCount":20,"columnCount":state.columns}},"developerMetadata":state.metadata}]})
                    };
                    if lose {
                        return;
                    }
                    let text = response.to_string();
                    let _=socket.write_all(format!("HTTP/1.1 {status} Fixture\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{text}",text.len()).as_bytes()).await;
                });
            }
        });
        Self {
            origin,
            state,
            lose_acquire,
            lose_mutation,
            lose_release,
            reject_mutation,
            reject_status,
            reject_acquire,
            mutations,
            hold_scan,
            scan_entered,
            resume_scan,
            gate_acquires,
            creates,
            lose_reconcile,
            replay_delete,
            task,
        }
    }
    fn client(&self) -> DirectSheetsClient {
        let mut c = DirectSheetsClient::new("fixture-token").unwrap();
        c.test_origin = Some(self.origin.clone());
        c
    }
}
// Break caught: validation before any mutation must not leave a permanent
// remote mutex which wedges every other installation.
#[tokio::test]
async fn invalid_payload_before_dispatch_releases_lock_for_other_writer() {
    let server = Server::start(2).await;
    let db = database();
    let mut bad = tests::payload("bad", 1, "");
    bad["spreadsheetId"] = json!("wrong-book");
    assert!(server
        .client()
        .deliver_shared(&target(), &bad, WRITER, &db)
        .await
        .is_err());
    assert_eq!(
        server
            .client()
            .deliver_shared(
                &target(),
                &tests::payload("good", 1, ""),
                OTHER,
                &database()
            )
            .await
            .unwrap()
            .row,
        2
    );
}

#[tokio::test]
async fn foreign_metadata_id_collision_is_not_overwritten_or_stolen() {
    let server = Server::start(2).await;
    server.hold_scan.store(true, Ordering::SeqCst);
    let client = server.client();
    let db = database();
    let first = tokio::spawn(async move {
        client
            .deliver_shared(&target(), &tests::payload("first", 1, ""), WRITER, &db)
            .await
    });
    tokio::time::timeout(Duration::from_secs(5), server.scan_entered.notified())
        .await
        .unwrap();
    {
        let mut state = server.state.lock();
        let md = state
            .metadata
            .iter_mut()
            .find(|v| v["metadataKey"] == shared_writer::LOCK_KEY)
            .unwrap();
        md["metadataKey"] = json!("other-app-key");
    }
    let error = server
        .client()
        .deliver_shared(
            &target(),
            &tests::payload("second", 1, ""),
            OTHER,
            &database(),
        )
        .await
        .unwrap_err();
    assert_eq!(error.kind, DirectSheetsErrorKind::Conflict);
    assert_eq!(server.mutations.load(Ordering::SeqCst), 0);
    assert!(server
        .state
        .lock()
        .metadata
        .iter()
        .any(|v| v["metadataKey"] == "other-app-key"));
    server.resume_scan.notify_one();
    assert!(first.await.unwrap().is_err());
}

#[tokio::test]
async fn simultaneous_create_collision_has_one_winner_and_delayed_delete_preserves_next_lock() {
    let server = Server::start(2).await;
    server.gate_acquires.store(true, Ordering::SeqCst);
    let db1 = database();
    let db2 = database();
    let c1 = server.client();
    let c2 = server.client();
    let t = target();
    let p1 = tests::payload("one", 1, "");
    let p2 = tests::payload("two", 1, "");
    let (one, two) = tokio::time::timeout(Duration::from_secs(8), async {
        tokio::join!(
            c1.deliver_shared(&t, &p1, WRITER, &db1),
            c2.deliver_shared(&t, &p2, OTHER, &db2)
        )
    })
    .await
    .expect("independent acquire requests must run concurrently");
    assert_ne!(one.is_ok(), two.is_ok());
    assert!(one
        .as_ref()
        .err()
        .or_else(|| two.as_ref().err())
        .unwrap()
        .retryable());
    assert_eq!(server.mutations.load(Ordering::SeqCst), 1);
    assert_eq!(server.creates.load(Ordering::SeqCst), 2);
    assert!(server.replay_delete.lock().is_some());
    // The fixture replays the prior writer's exact delete after next acquire.
    let receipt = if one.is_ok() {
        c2.deliver_shared(&t, &p2, OTHER, &db2).await
    } else {
        c1.deliver_shared(&t, &p1, WRITER, &db1).await
    };
    assert_eq!(receipt.unwrap().row, 3);
    assert_eq!(server.mutations.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn coalesced_revision_reconciles_frozen_previous_commit_before_new_write() {
    let server = Server::start(2).await;
    let db = database();
    server.lose_reconcile.store(true, Ordering::SeqCst);
    assert!(server
        .client()
        .deliver_shared(&target(), &tests::payload("one", 1, ""), WRITER, &db)
        .await
        .is_err());
    server.lose_reconcile.store(false, Ordering::SeqCst);
    let p = tests::payload("one", 2, "");
    let receipt = server
        .client()
        .deliver_shared(&target(), &p, WRITER, &db)
        .await
        .unwrap();
    assert_eq!(receipt.revision, 2);
    assert_eq!(receipt.row, 2);
    assert_eq!(server.mutations.load(Ordering::SeqCst), 2);
    assert_eq!(server.state.lock().rows.len(), 2);
}

#[tokio::test]
async fn definite_mutation_rejection_releases_mutex_and_preserves_error_kind() {
    for (status, kind) in [
        (403, DirectSheetsErrorKind::Forbidden),
        (429, DirectSheetsErrorKind::RateLimited),
    ] {
        let server = Server::start(2).await;
        let db = database();
        server.reject_status.store(status, Ordering::SeqCst);
        let error = server
            .client()
            .deliver_shared(&target(), &tests::payload("rejected", 1, ""), WRITER, &db)
            .await
            .unwrap_err();
        assert_eq!(error.kind, kind);
        assert_eq!(
            server
                .client()
                .deliver_shared(
                    &target(),
                    &tests::payload("other", 1, ""),
                    OTHER,
                    &database()
                )
                .await
                .unwrap()
                .row,
            2
        );
    }
}

#[tokio::test]
async fn rejected_create_after_winner_releases_is_still_retryable() {
    let server = Server::start(2).await;
    let db = database();
    // Simulate duplicate-create 400 followed by an already empty mutex read.
    server.reject_acquire.store(400, Ordering::SeqCst);
    let p = tests::payload("one", 1, "");
    let error = server
        .client()
        .deliver_shared(&target(), &p, WRITER, &db)
        .await
        .unwrap_err();
    assert!(error.retryable());
    server.lose_reconcile.store(false, Ordering::SeqCst);
    assert_eq!(
        server
            .client()
            .deliver_shared(&target(), &p, WRITER, &db)
            .await
            .unwrap()
            .row,
        2
    );
}

#[tokio::test]
async fn acquire_rejection_survives_failed_followup_read_without_pending_wedge() {
    let server = Server::start(2).await;
    let db = database();
    server.reject_acquire.store(429, Ordering::SeqCst);
    let p = tests::payload("one", 1, "");
    assert_eq!(
        server
            .client()
            .deliver_shared(&target(), &p, WRITER, &db)
            .await
            .unwrap_err()
            .kind,
        DirectSheetsErrorKind::RateLimited
    );
    server.lose_reconcile.store(false, Ordering::SeqCst);
    assert_eq!(
        server
            .client()
            .deliver_shared(&target(), &p, WRITER, &db)
            .await
            .unwrap()
            .row,
        2
    );
}

#[tokio::test]
async fn timeout_http_status_is_ambiguous_and_never_releases_or_replays() {
    for status in [408, 499, 503] {
        let server = Server::start(2).await;
        let db = database();
        server.reject_status.store(status, Ordering::SeqCst);
        let p = tests::payload("timeout", 1, "");
        assert_eq!(
            server
                .client()
                .deliver_shared(&target(), &p, WRITER, &db)
                .await
                .unwrap_err()
                .kind,
            DirectSheetsErrorKind::Busy
        );
        assert!(server
            .client()
            .deliver_shared(&target(), &p, WRITER, &db)
            .await
            .is_err());
        assert!(server
            .client()
            .deliver_shared(
                &target(),
                &tests::payload("other", 1, ""),
                OTHER,
                &database()
            )
            .await
            .is_err());
        assert_eq!(server.mutations.load(Ordering::SeqCst), 0);
        assert!(server
            .state
            .lock()
            .metadata
            .iter()
            .any(|m| m["metadataKey"] == shared_writer::LOCK_KEY));
    }
}

fn matches_filter(md: &Value, filter: &Value) -> bool {
    ["metadataId", "metadataKey", "metadataValue"]
        .iter()
        .all(|key| filter.get(*key).is_none_or(|value| md[*key] == *value))
}

// Break caught: local static serialization can hide the race; we hold the first
// client's scan while a distinct DB/client must promptly return remote Busy.
#[tokio::test]
async fn independent_databases_contend_before_scan_then_keep_both_rows_and_headers() {
    let server = Server::start(2).await;
    server.hold_scan.store(true, Ordering::SeqCst);
    let client = server.client();
    let db = database();
    let first = tokio::spawn(async move {
        client
            .deliver_shared(&target(), &tests::payload("first", 1, ""), WRITER, &db)
            .await
    });
    tokio::time::timeout(Duration::from_secs(5), server.scan_entered.notified())
        .await
        .unwrap();
    let db2 = database();
    let mut p = tests::payload("second", 1, "");
    p["partners"] = json!(["A", "B", "C"]);
    let error = tokio::time::timeout(
        Duration::from_secs(3),
        server.client().deliver_shared(&target(), &p, OTHER, &db2),
    )
    .await
    .unwrap()
    .unwrap_err();
    assert!(error.retryable());
    assert_eq!(error.kind, DirectSheetsErrorKind::Busy);
    server.resume_scan.notify_one();
    assert_eq!(first.await.unwrap().unwrap().row, 2);
    assert_eq!(
        server
            .client()
            .deliver_shared(&target(), &p, OTHER, &db2)
            .await
            .unwrap()
            .row,
        3
    );
    let state = server.state.lock();
    assert_eq!(state.rows[1][0].display(), "1");
    assert_eq!(state.rows[2][0].display(), "2");
    assert_eq!(state.rows[0][10].display(), "Đối tác 3");
    assert!(state.rows[1][3].note.contains("first"));
    assert!(state.rows[2][3].note.contains("second"));
}

#[tokio::test]
async fn committed_lost_ack_reconciles_after_restart_without_mutation_replay() {
    let server = Server::start(2).await;
    let db_path = std::env::temp_dir().join(format!("shared-restart-{}.db", uuid::Uuid::new_v4()));
    let db = crate::db::Database::open(&db_path).unwrap();
    let p = tests::payload("one", 1, "");
    server.lose_mutation.store(true, Ordering::SeqCst);
    let _ = server
        .client()
        .deliver_shared(&target(), &p, WRITER, &db)
        .await;
    drop(db);
    let db = crate::db::Database::open(db_path).unwrap();
    let receipt = server
        .client()
        .deliver_shared(&target(), &p, WRITER, &db)
        .await
        .unwrap();
    assert_eq!(receipt.row, 2);
    assert_eq!(server.mutations.load(Ordering::SeqCst), 1);
}
#[tokio::test]
async fn absent_commit_marker_is_uncertain_not_permission_to_replay_or_steal() {
    let server = Server::start(2).await;
    let db = database();
    let p = tests::payload("one", 1, "");
    server.reject_mutation.store(true, Ordering::SeqCst);
    assert!(server
        .client()
        .deliver_shared(&target(), &p, WRITER, &db)
        .await
        .unwrap_err()
        .retryable());
    assert!(server
        .client()
        .deliver_shared(&target(), &p, WRITER, &db)
        .await
        .unwrap_err()
        .retryable());
    assert!(server
        .client()
        .deliver_shared(&target(), &tests::payload("two", 1, ""), OTHER, &database())
        .await
        .unwrap_err()
        .retryable());
    assert_eq!(server.mutations.load(Ordering::SeqCst), 0);
    assert_eq!(server.state.lock().rows.len(), 1);
}
#[tokio::test]
async fn migration_requires_drain_confirmation_and_shared_reset_has_no_remote_write() {
    let server = Server::start(1).await;
    let db = database();
    let error = server
        .client()
        .prepare_shared_target(&target(), OTHER, false, &db)
        .await
        .unwrap_err();
    assert_eq!(error.kind, DirectSheetsErrorKind::SharedUpgradeRequired);
    assert_eq!(server.mutations.load(Ordering::SeqCst), 0);
    let check = server
        .client()
        .prepare_shared_target(&target(), OTHER, true, &db)
        .await
        .unwrap();
    assert_eq!(check.writer_schema_version, Some(2));
    assert!(check.writable);
    assert_eq!(check.reporting_epoch.as_deref(), Some("fixture-epoch"));
    let before = server.mutations.load(Ordering::SeqCst);
    assert!(server
        .client()
        .reset_target(&target(), OTHER, &uuid::Uuid::new_v4().to_string())
        .await
        .is_err());
    assert_eq!(server.mutations.load(Ordering::SeqCst), before);
    assert!(server
        .client()
        .deliver(&target(), &tests::payload("old", 1, ""), WRITER)
        .await
        .is_err());
}
#[tokio::test]
async fn lost_acquire_and_release_ack_are_resolved_by_exact_operation() {
    let server = Server::start(2).await;
    let db = database();
    let p = tests::payload("one", 1, "");
    server.lose_acquire.store(true, Ordering::SeqCst);
    server.lose_release.store(true, Ordering::SeqCst);
    let _ = server
        .client()
        .deliver_shared(&target(), &p, WRITER, &db)
        .await;
    assert_eq!(
        server
            .client()
            .deliver_shared(&target(), &p, WRITER, &db)
            .await
            .unwrap()
            .row,
        2
    );
    assert_eq!(server.mutations.load(Ordering::SeqCst), 1);
    assert!(server
        .state
        .lock()
        .metadata
        .iter()
        .all(|m| m["metadataKey"] != shared_writer::LOCK_KEY));
}
