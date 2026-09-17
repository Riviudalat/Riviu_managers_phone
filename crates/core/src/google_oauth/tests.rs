use super::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

fn config() -> GoogleOAuthClientConfig {
    GoogleOAuthClientConfig {
        client_id: "123-example.apps.googleusercontent.com".into(),
        client_secret: Some("private-client-secret".into()),
        picker_api_key: Some("private-picker-key".into()),
        project_number: Some("123456".into()),
    }
}
fn tokens() -> GoogleOAuthTokens {
    GoogleOAuthTokens {
        access_token: "private-access-token".into(),
        refresh_token: "private-refresh-token".into(),
        expires_at_ms: chrono::Utc::now().timestamp_millis() + 3600000,
        scope: GOOGLE_SHEETS_SCOPES.into(),
        account_id: "account-sub".into(),
        email: "fixture@example.com".into(),
    }
}

#[test]
fn config_and_token_debug_never_include_credentials() {
    let debug = format!("{:?} {:?}", config(), tokens());
    for secret in [
        "private-client-secret",
        "private-picker-key",
        "private-access-token",
        "private-refresh-token",
        "fixture@example.com",
    ] {
        assert!(!debug.contains(secret));
    }
    config().validate().unwrap();
    for id in [
        ".apps.googleusercontent.com",
        "123.apps.googleusercontent.com.evil",
        "123 apps.googleusercontent.com",
        "https://123.apps.googleusercontent.com",
    ] {
        let mut invalid = config();
        invalid.client_id = id.into();
        assert!(invalid.validate().is_err());
    }
}

#[tokio::test]
async fn pkce_sessions_have_distinct_state_verifier_and_ephemeral_loopback_ports() {
    let client = GoogleOAuthClient::new(config()).unwrap();
    let first = client.authorization_session().await.unwrap();
    let second = client.authorization_session().await.unwrap();
    assert_ne!(first.state, second.state);
    assert_ne!(first.verifier, second.verifier);
    assert_ne!(first.redirect_uri, second.redirect_uri);
    assert_eq!(first.verifier.len(), 43);
    let url = url::Url::parse(first.authorization_url()).unwrap();
    assert_eq!(url.host_str(), Some("accounts.google.com"));
    let fields: std::collections::HashMap<_, _> = url.query_pairs().collect();
    assert_eq!(fields["code_challenge_method"], "S256");
    assert_eq!(
        fields["code_challenge"],
        URL_SAFE_NO_PAD.encode(Sha256::digest(first.verifier.as_bytes()))
    );
    let scopes: Vec<_> = fields["scope"].split_whitespace().collect();
    assert!(scopes.contains(&"openid"));
    assert!(scopes.contains(&"email"));
    assert!(scopes.contains(&"https://www.googleapis.com/auth/spreadsheets"));
    assert!(!scopes.contains(&"https://www.googleapis.com/auth/drive"));
    let addr = first.listener.local_addr().unwrap();
    drop(first);
    assert!(tokio::net::TcpStream::connect(addr).await.is_err());
}

#[tokio::test]
async fn cancel_wait_closes_listener_before_any_token_exchange() {
    let session = GoogleOAuthClient::new(config())
        .unwrap()
        .authorization_session()
        .await
        .unwrap();
    let addr = session.listener.local_addr().unwrap();
    let cancel = session.cancel_handle();
    let task = tokio::spawn(session.wait());
    cancel.cancel();
    let result = tokio::time::timeout(Duration::from_secs(1), task)
        .await
        .unwrap()
        .unwrap();
    assert!(result.unwrap_err().to_string().contains("hủy"));
    assert!(tokio::net::TcpStream::connect(addr).await.is_err());
}

async fn callback_request(addr: std::net::SocketAddr, path: &str) -> String {
    let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
    stream
        .write_all(
            format!("GET {path} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n").as_bytes(),
        )
        .await
        .unwrap();
    let mut body = Vec::new();
    stream.read_to_end(&mut body).await.unwrap();
    String::from_utf8(body).unwrap()
}

#[tokio::test]
async fn callback_rejects_wrong_state_path_and_duplicate_fields_then_accepts_once() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        loopback::receive_authorization(&listener, &addr.to_string(), "expected").await
    });
    for path in [
        "/oauth/callback?code=secret&state=wrong",
        "/wrong?code=secret&state=expected",
        "/oauth/callback?code=secret&state=expected&state=expected",
    ] {
        let response = callback_request(addr, path).await;
        assert!(response.starts_with("HTTP/1.1 400"));
        assert!(!response.contains("secret"));
    }
    let response =
        callback_request(addr, "/oauth/callback?code=accepted-code&state=expected").await;
    assert!(response.starts_with("HTTP/1.1 200"));
    assert!(response.contains("Cache-Control: no-store"));
    assert!(!response.contains("accepted-code"));
    assert_eq!(task.await.unwrap().unwrap(), "accepted-code");
    assert!(tokio::net::TcpStream::connect(addr).await.is_err());
}

#[tokio::test]
async fn denied_callback_is_bound_to_state_and_cannot_exchange_a_code() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        loopback::receive_authorization(&listener, &addr.to_string(), "expected").await
    });
    callback_request(addr, "/oauth/callback?error=access_denied&state=expected").await;
    assert!(task
        .await
        .unwrap()
        .unwrap_err()
        .to_string()
        .contains("từ chối"));
}

#[test]
fn refresh_retains_original_refresh_token_and_rejects_invalid_grants_or_scopes() {
    let response = TokenResponse {
        access_token: "new-access".into(),
        refresh_token: None,
        token_type: "Bearer".into(),
        expires_in: 3600,
        scope: None,
    };
    let refreshed = response.into_tokens(Some(&tokens())).unwrap();
    assert_eq!(refreshed.refresh_token, "private-refresh-token");
    assert_eq!(refreshed.access_token, "new-access");
    assert!(!refreshed.needs_refresh());
    for value in [
        serde_json::json!({"access_token":"x","token_type":"Bearer","expires_in":0,"refresh_token":"r"}),
        serde_json::json!({"access_token":"x","token_type":"Bearer","expires_in":3600,"refresh_token":"r","scope":"openid email"}),
    ] {
        let response: TokenResponse = serde_json::from_value(value).unwrap();
        assert!(response.into_tokens(None).is_err());
    }
    let missing:TokenResponse=serde_json::from_value(serde_json::json!({"access_token":"x","token_type":"Bearer","expires_in":3600,"refresh_token":"r"})).unwrap();
    assert!(
        missing.into_tokens(None).is_err(),
        "initial scopes must be confirmed by Google"
    );
    let identity = GoogleIdentity {
        sub: "sub".into(),
        email: "fixture@example.com".into(),
        email_verified: Some(true),
    };
    identity.validate(Some("sub")).unwrap();
    assert!(identity.validate(Some("other")).is_err());
    let identity = GoogleIdentity {
        email_verified: Some(false),
        ..identity
    };
    assert!(identity.validate(None).is_err());
}

#[test]
fn initial_login_accepts_sheets_scope_without_drive_and_rejects_legacy_only_grant() {
    for (scope, accepted) in [
        ("openid email https://www.googleapis.com/auth/spreadsheets", true),
        ("openid https://www.googleapis.com/auth/userinfo.email https://www.googleapis.com/auth/spreadsheets", true),
        ("openid email https://www.googleapis.com/auth/drive.file", false),
        ("openid email https://www.googleapis.com/auth/spreadsheets.readonly", false),
    ] {
        let response: TokenResponse = serde_json::from_value(serde_json::json!({
            "access_token":"access", "refresh_token":"refresh", "token_type":"Bearer", "expires_in":3600, "scope":scope
        })).unwrap();
        assert_eq!(response.into_tokens(None).is_ok(), accepted, "{scope}");
    }
}

#[test]
fn refresh_legacy_scope_preserves_grant_without_silently_adding_sheets() {
    let mut previous = tokens();
    previous.scope = "openid email https://www.googleapis.com/auth/drive.file".into();
    let response: TokenResponse = serde_json::from_value(serde_json::json!({
        "access_token":"access", "token_type":"Bearer", "expires_in":3600
    }))
    .unwrap();
    let next = response.into_tokens(Some(&previous)).unwrap();
    assert_eq!(
        next.scope,
        "openid email https://www.googleapis.com/auth/drive.file"
    );
    assert_eq!(next.refresh_token, previous.refresh_token);
    assert!(!next.has_sheets_scope());
    assert!(tokens().has_sheets_scope());
}

#[tokio::test]
async fn picker_serves_no_store_nonce_page_and_requires_exact_origin_state_and_mime() {
    let session = GoogleOAuthClient::new(config())
        .unwrap()
        .picker_session(&tokens())
        .await
        .unwrap();
    let url = session.picker_url().to_string();
    let parsed = url::Url::parse(&url).unwrap();
    let origin = parsed.origin().ascii_serialization();
    let task = tokio::spawn(session.wait());
    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();
    let page = http.get(&url).send().await.unwrap();
    assert_eq!(page.status(), 200);
    assert!(page.headers()["cache-control"]
        .to_str()
        .unwrap()
        .contains("no-store"));
    assert!(page.headers()["content-security-policy"]
        .to_str()
        .unwrap()
        .contains("'nonce-"));
    let html = page.text().await.unwrap();
    assert!(html.contains("google.picker.ViewId.SPREADSHEETS"));
    let raw = html
        .split("const options=")
        .nth(1)
        .unwrap()
        .split(";let submitted")
        .next()
        .unwrap();
    let options: serde_json::Value = serde_json::from_str(raw).unwrap();
    let callback = format!("{origin}{}", options["callback"].as_str().unwrap());
    let file = serde_json::json!({"file":{"id":"sheet_123-abc","name":"Bảng nội dung","mimeType":"application/vnd.google-apps.spreadsheet"}});
    let bad = http
        .post(&callback)
        .header("Origin", "https://evil.invalid")
        .header("X-Riviu-Picker-State", options["csrf"].as_str().unwrap())
        .json(&file)
        .send()
        .await
        .unwrap();
    assert_eq!(bad.status(), 403);
    let bad = http
        .post(&callback)
        .header("Origin", &origin)
        .header("X-Riviu-Picker-State", "wrong")
        .json(&file)
        .send()
        .await
        .unwrap();
    assert_eq!(bad.status(), 403);
    let wrong = serde_json::json!({"file":{"id":"sheet_123","name":"document","mimeType":"application/pdf"}});
    let bad = http
        .post(&callback)
        .header("Origin", &origin)
        .header("X-Riviu-Picker-State", options["csrf"].as_str().unwrap())
        .json(&wrong)
        .send()
        .await
        .unwrap();
    assert_eq!(bad.status(), 403);
    let good = http
        .post(&callback)
        .header("Origin", &origin)
        .header("X-Riviu-Picker-State", options["csrf"].as_str().unwrap())
        .json(&file)
        .send()
        .await
        .unwrap();
    assert_eq!(good.status(), 200);
    let selected = task.await.unwrap().unwrap();
    assert_eq!(selected.id, "sheet_123-abc");
    assert_eq!(selected.name, "Bảng nội dung");
    assert!(http.get(&url).send().await.is_err());
}

#[tokio::test]
async fn picker_cancel_and_drop_close_only_their_own_session() {
    let client = GoogleOAuthClient::new(config()).unwrap();
    let first = client.picker_session(&tokens()).await.unwrap();
    let second = client.picker_session(&tokens()).await.unwrap();
    let second_url = second.picker_url().to_string();
    let cancel = first.cancel_handle();
    cancel.cancel();
    assert!(first.wait().await.is_err());
    let second_cancel = second.cancel_handle();
    let task = tokio::spawn(second.wait());
    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap();
    assert_eq!(http.get(second_url).send().await.unwrap().status(), 200);
    second_cancel.cancel();
    assert!(task.await.unwrap().is_err());
}

#[tokio::test]
async fn loopback_rejects_foreign_host_and_oversized_request_before_body_read() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        loopback::read_request(&mut stream, &addr.to_string())
            .await
            .is_err()
    });
    let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
    stream.write_all(b"POST /oauth/callback HTTP/1.1\r\nHost: evil.invalid\r\nContent-Length: 999999999\r\n\r\n").await.unwrap();
    assert!(task.await.unwrap());
}

#[tokio::test(start_paused = true)]
async fn abandoned_login_deadline_closes_the_original_listener() {
    let session = GoogleOAuthClient::new(config())
        .unwrap()
        .authorization_session()
        .await
        .unwrap();
    let addr = session.listener.local_addr().unwrap();
    let task = tokio::spawn(session.wait());
    tokio::task::yield_now().await;
    tokio::time::advance(SESSION_TIMEOUT + Duration::from_secs(1)).await;
    let error = task.await.unwrap().unwrap_err();
    assert_eq!(
        error.downcast_ref::<GoogleOAuthError>().unwrap().code,
        GoogleOAuthErrorCode::Timeout
    );
    assert!(tokio::net::TcpStream::connect(addr).await.is_err());
}

#[tokio::test]
async fn cancelling_slow_callback_interrupts_read_without_leaving_listener_alive() {
    let session = GoogleOAuthClient::new(config())
        .unwrap()
        .authorization_session()
        .await
        .unwrap();
    let addr = session.listener.local_addr().unwrap();
    let cancel = session.cancel_handle();
    let task = tokio::spawn(session.wait());
    let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
    stream
        .write_all(b"GET /oauth/callback HTTP/1.1\r\n")
        .await
        .unwrap();
    cancel.cancel();
    assert!(tokio::time::timeout(Duration::from_secs(1), task)
        .await
        .unwrap()
        .unwrap()
        .is_err());
    assert!(tokio::net::TcpStream::connect(addr).await.is_err());
}

#[tokio::test]
async fn token_error_response_is_typed_without_echoing_provider_secrets() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut request = [0u8; 2048];
        assert!(stream.read(&mut request).await.unwrap() > 0);
        let body = r#"{"error":"invalid_grant","error_description":"secret-refresh-token"}"#;
        stream.write_all(format!("HTTP/1.1 400 Bad Request\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).as_bytes()).await.unwrap();
    });
    let response = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap()
        .get(format!("http://{addr}/fixture"))
        .send()
        .await
        .unwrap();
    let error = decode_response::<TokenResponse>(response, "làm mới đăng nhập")
        .await
        .err()
        .unwrap();
    assert!(!error.to_string().contains("secret-refresh-token"));
    assert!(error
        .downcast_ref::<GoogleOAuthError>()
        .unwrap()
        .requires_reconnect());
    assert!(!error
        .downcast_ref::<GoogleOAuthError>()
        .unwrap()
        .is_retryable());
    server.await.unwrap();
}
