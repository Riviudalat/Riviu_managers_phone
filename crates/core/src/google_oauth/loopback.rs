use super::*;
use std::collections::BTreeMap;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
};

pub(super) struct Request {
    pub method: String,
    pub target: String,
    pub headers: BTreeMap<String, String>,
    pub body: Vec<u8>,
}
const HEADER_LIMIT: usize = 8192;
const REQUEST_BODY_LIMIT: usize = 16384;

pub(super) async fn read_request(stream: &mut TcpStream, host: &str) -> Result<Request> {
    tokio::time::timeout(Duration::from_secs(10), async {
        let mut bytes = Vec::new();
        let mut chunk = [0u8; 2048];
        let header_end = loop {
            if let Some(at) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                break at + 4;
            }
            anyhow::ensure!(bytes.len() < HEADER_LIMIT, "Loopback headers too large");
            let count = stream.read(&mut chunk).await?;
            anyhow::ensure!(count > 0, "Loopback request incomplete");
            bytes.extend_from_slice(&chunk[..count]);
        };
        anyhow::ensure!(header_end <= HEADER_LIMIT, "Loopback headers too large");
        let header = std::str::from_utf8(&bytes[..header_end])?;
        let mut lines = header.split("\r\n");
        let first = lines.next().context("Request line missing")?;
        let parts: Vec<_> = first.split(' ').collect();
        anyhow::ensure!(
            parts.len() == 3
                && parts[2] == "HTTP/1.1"
                && parts[1].starts_with('/')
                && !parts[1].starts_with("//"),
            "Invalid loopback request line"
        );
        let method = parts[0].to_owned();
        let target = parts[1].to_owned();
        let mut headers = BTreeMap::new();
        for line in lines.filter(|line| !line.is_empty()) {
            anyhow::ensure!(
                !line.starts_with(char::is_whitespace),
                "Invalid folded header"
            );
            let (name, value) = line.split_once(':').context("Invalid header")?;
            let name = name.to_ascii_lowercase();
            anyhow::ensure!(
                headers.insert(name, value.trim().to_owned()).is_none(),
                "Duplicate header"
            );
        }
        anyhow::ensure!(
            headers.get("host").is_some_and(|value| value == host),
            "Invalid loopback Host"
        );
        anyhow::ensure!(
            !headers.contains_key("transfer-encoding"),
            "Chunked callback is unsupported"
        );
        let length = headers
            .get("content-length")
            .map(|value| value.parse::<usize>())
            .transpose()?
            .unwrap_or(0);
        anyhow::ensure!(length <= REQUEST_BODY_LIMIT, "Loopback body too large");
        while bytes.len() < header_end + length {
            let count = stream.read(&mut chunk).await?;
            anyhow::ensure!(count > 0, "Loopback body incomplete");
            bytes.extend_from_slice(&chunk[..count]);
            anyhow::ensure!(
                bytes.len() <= header_end + REQUEST_BODY_LIMIT,
                "Loopback body too large"
            );
        }
        anyhow::ensure!(
            bytes.len() == header_end + length,
            "Unexpected pipelined callback"
        );
        Ok(Request {
            method,
            target,
            headers,
            body: bytes[header_end..].to_vec(),
        })
    })
    .await
    .context("Loopback request timed out")?
}

pub(super) async fn respond(
    stream: &mut TcpStream,
    status: u16,
    body: &str,
    csp: Option<&str>,
) -> Result<()> {
    let reason = match status {
        200 => "OK",
        400 => "Bad Request",
        403 => "Forbidden",
        404 => "Not Found",
        _ => "Error",
    };
    let policy = csp.unwrap_or(
        "default-src 'none'; frame-ancestors 'none'; base-uri 'none'; form-action 'none'",
    );
    let response=format!("HTTP/1.1 {status} {reason}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store, max-age=0\r\nPragma: no-cache\r\nX-Content-Type-Options: nosniff\r\nReferrer-Policy: strict-origin-when-cross-origin\r\nContent-Security-Policy: {policy}\r\n\r\n{body}",body.len());
    tokio::time::timeout(
        Duration::from_secs(5),
        stream.write_all(response.as_bytes()),
    )
    .await
    .context("Loopback response timed out")??;
    Ok(())
}

pub(super) fn secure_equal(a: &str, b: &str) -> bool {
    a.len() == b.len()
        && a.bytes()
            .zip(b.bytes())
            .fold(0u8, |difference, (a, b)| difference | (a ^ b))
            == 0
}

pub(super) fn callback_code(request: &Request, state: &str) -> Result<Option<String>> {
    anyhow::ensure!(
        request.method == "GET" && request.body.is_empty(),
        "Unexpected OAuth callback method"
    );
    anyhow::ensure!(
        request.target.split('?').next() == Some("/oauth/callback")
            && !request.target.contains('#'),
        "Unexpected OAuth callback path"
    );
    let url = url::Url::parse(&format!("http://127.0.0.1{}", request.target))?;
    anyhow::ensure!(
        url.path() == "/oauth/callback" && url.fragment().is_none(),
        "Unexpected OAuth callback path"
    );
    let mut fields = BTreeMap::new();
    for (key, value) in url.query_pairs() {
        anyhow::ensure!(
            fields
                .insert(key.into_owned(), value.into_owned())
                .is_none(),
            "Duplicate OAuth callback field"
        );
    }
    anyhow::ensure!(
        fields
            .get("state")
            .is_some_and(|returned| secure_equal(returned, state)),
        "OAuth state mismatch"
    );
    if fields.contains_key("error") {
        anyhow::ensure!(!fields.contains_key("code"), "Conflicting OAuth result");
        return Ok(None);
    }
    let code = fields.remove("code").context("OAuth code missing")?;
    anyhow::ensure!(
        !code.is_empty() && code.len() <= 4096 && !code.chars().any(char::is_control),
        "Invalid OAuth code"
    );
    Ok(Some(code))
}

pub(super) async fn receive_authorization(
    listener: &TcpListener,
    host: &str,
    state: &str,
) -> Result<String> {
    for _ in 0..64 {
        let (mut stream, remote) = listener.accept().await?;
        if !remote.ip().is_loopback() {
            continue;
        }
        let request = match read_request(&mut stream, host).await {
            Ok(request) => request,
            Err(_) => {
                let _ = respond(&mut stream, 400, "Yêu cầu không hợp lệ.", None).await;
                continue;
            }
        };
        match callback_code(&request, state) {
            Ok(Some(code)) => {
                let _ = respond(
                    &mut stream,
                    200,
                    "Đã nhận kết quả đăng nhập. Bạn có thể trở về Riviu Manager.",
                    None,
                )
                .await;
                return Ok(code);
            }
            Ok(None) => {
                let _ = respond(
                    &mut stream,
                    200,
                    "Bạn đã hủy cấp quyền. Trở về Riviu Manager để thử lại.",
                    None,
                )
                .await;
                return Err(oauth_error(
                    GoogleOAuthErrorCode::Denied,
                    "Bạn đã từ chối đăng nhập Google",
                ));
            }
            Err(_) => {
                let _ = respond(
                    &mut stream,
                    400,
                    "Kết quả đăng nhập không khớp phiên đang mở.",
                    None,
                )
                .await;
            }
        }
    }
    anyhow::bail!("Quá nhiều yêu cầu không khớp phiên đăng nhập Google")
}
