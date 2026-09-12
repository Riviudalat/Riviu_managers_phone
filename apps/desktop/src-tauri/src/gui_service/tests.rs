use super::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[tokio::test]
async fn real_service_roundtrip_keeps_identity_and_budget_without_device_effects() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut input = Vec::new();
        let mut chunk = [0u8; 4096];
        loop {
            let n = socket.read(&mut chunk).await.unwrap();
            if n == 0 {
                break;
            }
            input.extend_from_slice(&chunk[..n]);
            if let Some(end) = input.windows(4).position(|s| s == b"\r\n\r\n") {
                let headers = String::from_utf8_lossy(&input[..end]);
                let length = headers
                    .lines()
                    .find_map(|line| {
                        line.to_lowercase()
                            .strip_prefix("content-length:")
                            .and_then(|v| v.trim().parse::<usize>().ok())
                    })
                    .unwrap_or(0);
                if input.len() >= end + 4 + length {
                    break;
                }
            }
        }
        let body=serde_json::json!({"choices":[{"message":{"content":"{\"status\":\"resolved\",\"nodeIds\":[3],\"reason\":\"Profile tab in current frame\"}"}}],"usage":{"prompt_tokens":17,"completion_tokens":8,"cost":0.001}}).to_string();
        socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).as_bytes()).await.unwrap();
    });
    let dir = std::env::temp_dir().join(format!("riviu-gui-integration-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let db = Arc::new(Database::open(dir.join("test.db")).unwrap());
    let settings = riviu_core::NurtureSettings {
        base_url: format!("http://{address}/v1"),
        model: "fixture-model".into(),
        api_key: "fixture-only-key".into(),
        ..Default::default()
    };
    db.save_nurture_settings(&settings).unwrap();
    let service = GuiService::new(db.clone(), None, dir.join("gui"));
    service
        .save(GuiConfig {
            max_requests: 1,
            ..Default::default()
        })
        .await
        .unwrap();
    let request: GuiRequest = serde_json::from_str(include_str!(
        "../../../../../crates/core/fixtures/gui-request.json"
    ))
    .unwrap();
    let result = service.resolve(request.clone()).await;
    service.stop().await;
    let response = result.unwrap();
    response.validate_binding(&request).unwrap();
    assert_eq!(response.candidates[0].node_id, Some(3));
    assert_eq!(response.prompt_tokens, Some(17));
    assert_eq!(response.cost_usd, Some(0.001));
    let mut second = request;
    second.request_id = "second".into();
    assert!(service
        .resolve(second)
        .await
        .unwrap_err()
        .to_string()
        .contains("budget"));
    server.await.unwrap();
}

#[test]
fn pack_import_keeps_effect_contract_and_requires_opposing_fixtures() {
    use riviu_core::app_automation::{AppAdapter, TikTokAdapter};
    let dir = std::env::temp_dir().join(format!("riviu-gui-packs-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let service = GuiService::new(
        Arc::new(Database::open(dir.join("test.db")).unwrap()),
        None,
        dir.join("gui"),
    );
    let mut pack = TikTokAdapter.pack();
    pack.revision = 2;
    assert!(service
        .import_pack(&serde_json::to_vec(&pack).unwrap())
        .is_ok());
    pack.targets
        .iter_mut()
        .find(|t| t.id == "post")
        .unwrap()
        .effectful = false;
    assert!(service
        .import_pack(&serde_json::to_vec(&pack).unwrap())
        .is_err());
}
