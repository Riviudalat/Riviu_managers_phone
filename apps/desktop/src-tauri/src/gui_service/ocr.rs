//! Local OCR uses the same observation DTO as Flow; no provider account is needed.
use super::GuiService;
use anyhow::{anyhow, ensure};
use riviu_core::ui_automation::{OcrRequest, OcrResponse};
use std::time::Duration;
use tokio::{sync::Semaphore, time::Instant};

const MAX_CAPACITY_ATTEMPTS: usize = 30;

async fn post_ocr_with_backpressure(
    client: &reqwest::Client,
    url: &str,
    token: &str,
    request: &OcrRequest,
    capacity: &Semaphore,
    deadline: Instant,
) -> anyhow::Result<OcrResponse> {
    for attempt in 0..MAX_CAPACITY_ATTEMPTS {
        let permit = tokio::time::timeout_at(deadline, capacity.acquire())
            .await
            .map_err(|_| anyhow!("gui_deadline"))??;
        let remaining = deadline.saturating_duration_since(Instant::now());
        ensure!(!remaining.is_zero(), "gui_deadline");
        let mut current = request.clone();
        current.remaining_ms = u64::try_from(remaining.as_millis())
            .unwrap_or(u64::MAX)
            .max(1);
        let mut response = tokio::time::timeout_at(
            deadline,
            client.post(url).bearer_auth(token).json(&current).send(),
        )
        .await
        .map_err(|_| anyhow!("gui_deadline"))??;
        if response.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
            let retry_after = response
                .headers()
                .get(reqwest::header::RETRY_AFTER)
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.parse::<u64>().ok())
                .map(|seconds| Duration::from_secs(seconds.min(2)))
                .unwrap_or(Duration::from_secs(1));
            drop(response);
            drop(permit);
            if attempt + 1 == MAX_CAPACITY_ATTEMPTS {
                return Err(anyhow!("gui_ocr_http_429"));
            }
            tokio::time::timeout_at(deadline, tokio::time::sleep(retry_after))
                .await
                .map_err(|_| anyhow!("gui_deadline"))?;
            continue;
        }
        ensure!(
            response.status().is_success(),
            "gui_ocr_http_{}",
            response.status().as_u16()
        );
        let mut bytes = Vec::new();
        while let Some(chunk) = tokio::time::timeout_at(deadline, response.chunk())
            .await
            .map_err(|_| anyhow!("gui_deadline"))??
        {
            ensure!(
                bytes.len() + chunk.len() <= 128 * 1024,
                "gui_ocr_response_budget"
            );
            bytes.extend_from_slice(&chunk);
        }
        let result: OcrResponse = serde_json::from_slice(&bytes)?;
        result.validate_binding(request)?;
        return Ok(result);
    }
    unreachable!("capacity retry loop has a positive attempt count")
}

impl GuiService {
    pub async fn recognize_text(&self, request: OcrRequest) -> anyhow::Result<OcrResponse> {
        let duration = Duration::from_millis(request.remaining_ms.min(30_000));
        tokio::time::timeout(duration, async {
            let request = {
                let permit = self.capacity.clone().acquire_owned().await?;
                let (request, permit) = tokio::task::spawn_blocking(move || {
                    request.validate()?;
                    Ok::<_, anyhow::Error>((request, permit))
                })
                .await??;
                drop(permit);
                request
            };
            let (url, token) = self.connection_for(false).await?;
            let client = reqwest::Client::builder()
                .timeout(duration)
                .redirect(reqwest::redirect::Policy::none())
                .no_proxy()
                .build()?;
            post_ocr_with_backpressure(
                &client,
                &format!("{url}/v1/gui/ocr"),
                &token,
                &request,
                &self.capacity,
                Instant::now() + duration,
            )
            .await
        })
        .await
        .map_err(|_| anyhow::anyhow!("gui_deadline"))?
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    async fn read_request(socket: &mut tokio::net::TcpStream) -> OcrRequest {
        let mut input = Vec::new();
        let mut chunk = [0u8; 4096];
        loop {
            let n = socket.read(&mut chunk).await.unwrap();
            assert!(n > 0);
            input.extend_from_slice(&chunk[..n]);
            if let Some(end) = input.windows(4).position(|part| part == b"\r\n\r\n") {
                let headers = String::from_utf8_lossy(&input[..end]);
                let length = headers
                    .lines()
                    .find_map(|line| {
                        line.to_ascii_lowercase()
                            .strip_prefix("content-length:")
                            .and_then(|value| value.trim().parse::<usize>().ok())
                    })
                    .unwrap();
                if input.len() >= end + 4 + length {
                    return serde_json::from_slice(&input[end + 4..end + 4 + length]).unwrap();
                }
            }
        }
    }

    #[tokio::test]
    async fn local_ocr_capacity_retries_without_changing_observation() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let mut requests = Vec::new();
            for attempt in 0..2 {
                let (mut socket, _) = listener.accept().await.unwrap();
                let request = read_request(&mut socket).await;
                requests.push(request.clone());
                if attempt == 0 {
                    socket
                        .write_all(b"HTTP/1.1 429 Too Many Requests\r\nRetry-After: 1\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                        .await
                        .unwrap();
                } else {
                    let body = serde_json::json!({
                        "protocolVersion": request.protocol_version,
                        "requestId": request.request_id,
                        "observationId": request.observation_id,
                        "sessionEpoch": request.session_epoch,
                        "generation": request.generation,
                        "screenshotSha256": request.screenshot.sha256,
                        "status": "unresolved",
                        "text": "",
                        "lines": [],
                        "engine": "fixture",
                        "elapsedMs": 1
                    })
                    .to_string();
                    socket
                        .write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes())
                        .await
                        .unwrap();
                }
            }
            requests
        });
        let request: OcrRequest = serde_json::from_str(include_str!(
            "../../../../../crates/core/fixtures/ocr-request.json"
        ))
        .unwrap();
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let capacity = tokio::sync::Semaphore::new(1);
        let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
        let response = post_ocr_with_backpressure(
            &client,
            &format!("http://{address}/v1/gui/ocr"),
            "fixture-token",
            &request,
            &capacity,
            deadline,
        )
        .await
        .unwrap();
        assert_eq!(response.status, "unresolved");
        let requests = server.await.unwrap();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0].request_id, requests[1].request_id);
        assert_eq!(requests[0].observation_id, requests[1].observation_id);
        assert_eq!(requests[0].screenshot.sha256, requests[1].screenshot.sha256);
        assert!(requests[1].remaining_ms < requests[0].remaining_ms);
        assert_eq!(capacity.available_permits(), 1);
    }

    #[tokio::test]
    async fn local_ocr_capacity_wait_stops_at_deadline_without_sending() {
        let request: OcrRequest = serde_json::from_str(include_str!(
            "../../../../../crates/core/fixtures/ocr-request.json"
        ))
        .unwrap();
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let capacity = Semaphore::new(0);
        let error = post_ocr_with_backpressure(
            &client,
            "http://127.0.0.1:1/v1/gui/ocr",
            "fixture-token",
            &request,
            &capacity,
            Instant::now() + Duration::from_millis(30),
        )
        .await
        .unwrap_err();
        assert_eq!(error.to_string(), "gui_deadline");
    }

    #[tokio::test]
    async fn local_ocr_429_retry_after_cannot_extend_deadline() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let _request = read_request(&mut socket).await;
            socket
                .write_all(b"HTTP/1.1 429 Too Many Requests\r\nRetry-After: 2\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                .await
                .unwrap();
        });
        let request: OcrRequest = serde_json::from_str(include_str!(
            "../../../../../crates/core/fixtures/ocr-request.json"
        ))
        .unwrap();
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let capacity = Semaphore::new(1);
        let error = post_ocr_with_backpressure(
            &client,
            &format!("http://{address}/v1/gui/ocr"),
            "fixture-token",
            &request,
            &capacity,
            Instant::now() + Duration::from_millis(100),
        )
        .await
        .unwrap_err();
        assert_eq!(error.to_string(), "gui_deadline");
        assert_eq!(capacity.available_permits(), 1);
        server.await.unwrap();
    }
}
