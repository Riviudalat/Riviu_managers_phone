//! Pushing a published carousel's link to the operator's partner sheet.
//!
//! The transport half of [`crate::db::publish_sheet`]. That module explains why the row is
//! written to the database first; this one is what carries it, and it is built so that
//! everything it can do wrong is confined to the sheet.
//!
//! # Why a webhook and not the Sheets API
//!
//! The operator chose an Apps Script webhook. It is the right choice for this: the Sheets
//! API needs an OAuth client, a consent flow and a refresh token living on a desktop
//! machine, and the app currently carries no Google integration at all. A web app bound to
//! the sheet needs a URL and a shared secret, and the script runs as the sheet's owner —
//! so nothing here ever holds a credential that can reach anything but this one sheet.
//!
//! The script itself is `docs/apps-script/publish-sheet.gs`, which the operator pastes into
//! the sheet. The layout it writes is theirs, verbatim: the post link in **column D**, the
//! poster as `bot`, and the partner names from the workbook spread **from column K**.
//!
//! # Wired since 31/08/2026, and proved against a live sheet
//!
//! This module spent a while as finished code waiting on a measurement: the chain — publish,
//! read the link back, queue the row, sweep — was complete except for its middle, and that
//! middle needed the caller to be standing on the post it had just published. That route is
//! measured now ([`crate::tiktok_share::capture_own_post_link`], §9.136), the desktop's
//! background sweeper carries the rows, and the whole path has been run end to end against a
//! real deployed script: a row landed in the operator's sheet, and re-sending the same
//! assignment left it at one row.
//!
//! What it must never become is a path that reports a failed *sheet write* as a failed
//! *post* — see [`crate::db::publish_sheet`] for the half that guarantees that.
//!
//! # The redirect is part of the protocol, and refusing it was a wrong answer
//!
//! Apps Script answers `POST /exec` with **302** to `script.googleusercontent.com`; the reply
//! body lives only there. The first version of [`client`] refused every redirect to keep the
//! token out of a stranger's hands, which sounded right and was measured wrong: the POST had
//! already reached the script, the script had already written the row, and the client called
//! it a failure. See the policy in [`client`] for what is allowed instead — one hop, one
//! host, and never a body-preserving 307/308.
//!
//! # The secret is a real one
//!
//! An Apps Script web app deployed so the desktop can reach it is reachable by anyone who
//! has the URL — Google does not authenticate the caller. So the URL alone is not a
//! credential and must not be treated as one: the script compares a shared token and
//! refuses without it. Both live in `settings`, which is the same SQLite file the rest of
//! the app uses; that is fine here in a way it is not for an API key that can spend money,
//! because the worst this token can do is write rows to one spreadsheet.

use anyhow::Context;
use std::time::Duration;

use serde::{Deserialize, Serialize};

/// Settings key holding the deployed web app URL.
pub const WEBHOOK_URL_SETTING: &str = "publish_sheet_webhook_url";
/// Settings key holding the shared token the script checks.
pub const WEBHOOK_TOKEN_SETTING: &str = "publish_sheet_webhook_token";
pub const INTERNAL_REPORTING_SETTING: &str = "publish_sheet_internal_reporting";

/// How long to wait on the webhook.
///
/// Generous because Apps Script cold-starts: a script that has not run for a while takes
/// seconds to answer the first request. Short enough that a sweep cannot wedge — this runs
/// on a background pass, and a hung request there would hold the pass open indefinitely.
pub const WEBHOOK_TIMEOUT: Duration = Duration::from_secs(30);

/// One row, as the script receives it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SheetRow {
    /// The shared token. Checked by the script before anything is written.
    pub token: String,
    /// The link to the published post — column D.
    pub post_url: String,
    /// Who posted it — column B, and always `bot`.
    ///
    /// **One legal value, and the operator's sheet is the reason.** For one day this carried
    /// the device's handle, on the argument that a column always reading `bot` cannot say
    /// whose post a row is. Reading the real sheet refuted it: column B is `Nhân Viên`, a
    /// staff column holding eleven people's names, so `bot` is what distinguishes the app's
    /// rows from a person's. The account is still identifiable — the canonical link in
    /// column D carries `@handle`. See `publish_commands::poster_identity`.
    ///
    /// The script keeps its own `|| 'bot'` fallback, and migration 18's CHECK refuses an
    /// empty poster, so this can never arrive blank from either side.
    pub poster: String,
    /// Partner names in workbook order, written from column K onward.
    pub partners: Vec<String>,
    /// The assignment this came from.
    ///
    /// Not used for placement — it is the **idempotency key**. The script refuses to write
    /// a row whose key it has already seen, which is what makes a retry after an ambiguous
    /// response safe: a timeout on a request the script actually processed would otherwise
    /// paste the same link into column D twice, and nothing on the desktop can tell that
    /// case apart from a request that never arrived.
    pub assignment_id: String,
    /// Immutable time the Post intent was recorded, formatted by the destination timezone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub posted_at: Option<String>,
}

/// Read-only report facts; these fields never create a classic outbox obligation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct InternalReportMetadata {
    pub report_version: u32,
    pub row_revision: i64,
    pub machine: String,
    pub tiktok_account: String,
    pub status: String,
    pub state_notes: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct InternalSheetReportRow {
    pub row_kind: String,
    #[serde(flatten)]
    pub metadata: InternalReportMetadata,
    pub assignment_id: String,
    pub post_url: String,
    pub poster: String,
    pub partners: Vec<String>,
    pub posted_at: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SheetDeliverySettings {
    pub webhook_url: String,
    pub token: String,
    pub internal_reporting: bool,
}

/// What the script answers.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SheetReply {
    pub ok: bool,
    /// Set when the script recognised the assignment and wrote nothing.
    #[serde(default)]
    pub duplicate: bool,
    #[serde(default)]
    pub error: Option<String>,
}

/// Whether the script's answer means the sheet now holds the row.
///
/// A named function rather than three lines inside `push_row`, because what it decides cannot
/// be tested through an HTTP call without standing up a server — and the mistake it exists to
/// prevent was invisible to every test that did not.
///
/// **`duplicate` only counts alongside `ok`.** Checking it first accepted
/// `{"ok":false,"duplicate":true,"error":"write failed"}` as a success: a reply the current
/// script never sends, but an older deployment or a proxy in front of it can, and the row
/// would then be marked delivered against a sheet holding nothing.
fn interpret(reply: SheetReply) -> anyhow::Result<()> {
    anyhow::ensure!(
        reply.ok,
        "webhook Sheet từ chối: {}",
        reply.error.unwrap_or_else(|| "không nói lý do".into())
    );
    Ok(())
}

/// The `LIMIT` a sweep may ask SQLite for.
///
/// Clamped, and the reason is not tidiness: `usize::MAX as i64` is `-1`, and SQLite reads a
/// negative `LIMIT` as **no limit** — so the argument meant to bound the sweep is the one that
/// would have unbounded it, and a long outage's whole backlog would materialise at once.
pub fn sweep_limit(limit: usize) -> i64 {
    limit.min(1_000) as i64
}

/// The first 200 characters of a response, with the bearer token taken out.
///
/// **The body belongs to whoever answers the URL.** An endpoint that echoes the request —
/// a debug handler, a proxy, a host typed one character wrong — puts the token in it, and
/// this slice is stored in the outbox's `last_error` and written to the app log. A
/// credential in a log has to be re-issued, so it is removed before the evidence is kept
/// rather than after somebody notices.
///
/// The redaction runs before the truncation: a token straddling the 200-character boundary
/// would otherwise survive in halves. An empty token redacts nothing — replacing the empty
/// string would rewrite the whole body.
fn redact_token(body: &str, token: &str) -> String {
    let cleaned = if token.trim().is_empty() {
        body.to_string()
    } else {
        body.replace(token, "«token»")
    };
    cleaned.chars().take(200).collect()
}

/// Whether a redirect target is the content host Apps Script hands its answer to.
///
/// Measured 31/08/2026 against a live deployment: `POST /exec` on `script.google.com`
/// answers **302** with `Location: https://script.googleusercontent.com/macros/echo?…`, and
/// the answer only exists at that URL. The old policy refused every redirect, so the client
/// could not talk to Apps Script *at all* — it read the 302 as the endpoint's reply and
/// failed every row. The refusal was written from a belief about how Apps Script answers,
/// and the belief was wrong; the endpoint being "direct" was never measured.
///
/// So the hop is allowed, and nothing else is: one host family, HTTPS only. `www.google.com`
/// is not on it, and neither is anything that merely ends in `google.com` — the point is the
/// single host the protocol names, not Google in general.
fn is_script_content_host(host: &str) -> bool {
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    host == "script.googleusercontent.com"
}

fn client() -> anyhow::Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .timeout(WEBHOOK_TIMEOUT)
        .connect_timeout(Duration::from_secs(7))
        .redirect(reqwest::redirect::Policy::none())
        .build()?)
}

#[derive(Debug)]
struct SheetTransportError {
    message: String,
    retryable: bool,
}

fn sheet_redirect(status: reqwest::StatusCode, location: &str) -> anyhow::Result<reqwest::Url> {
    anyhow::ensure!(
        matches!(status.as_u16(), 302 | 303),
        "Sheet chuyển hướng không hỗ trợ (HTTP {status})"
    );
    let url = reqwest::Url::parse(location)
        .map_err(|_| anyhow::anyhow!("Sheet trả địa chỉ chuyển hướng không hợp lệ"))?;
    anyhow::ensure!(
        url.scheme() == "https"
            && url.host_str().is_some_and(is_script_content_host)
            && url.username().is_empty()
            && url.password().is_none()
            && url.port_or_known_default() == Some(443),
        "Sheet chuyển hướng ngoài máy chủ nội dung Google"
    );
    Ok(url)
}

fn retry_sheet_status(status: reqwest::StatusCode, content_hop: bool) -> bool {
    status == reqwest::StatusCode::TOO_MANY_REQUESTS
        || status.is_server_error()
        || (content_hop && status == reqwest::StatusCode::NOT_FOUND)
}

fn sheet_network_error(error: reqwest::Error, stage: &str) -> SheetTransportError {
    let retryable =
        error.is_timeout() || error.is_connect() || error.is_request() || error.is_body();
    // reqwest's Display can include user_content_key and the complete ephemeral URL.
    SheetTransportError {
        message: format!("Kết nối Sheet lỗi tại {stage}: {}", error.without_url()),
        retryable,
    }
}

async fn sheet_post_once(
    http: &reqwest::Client,
    webhook: &str,
    payload: &serde_json::Value,
) -> Result<String, SheetTransportError> {
    let mut response = http
        .post(webhook)
        .json(payload)
        .send()
        .await
        .map_err(|error| sheet_network_error(error, "webhook"))?;
    let mut content_hop = false;
    if response.status().is_redirection() {
        let location = response
            .headers()
            .get(reqwest::header::LOCATION)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("");
        let url =
            sheet_redirect(response.status(), location).map_err(|error| SheetTransportError {
                message: error.to_string(),
                retryable: false,
            })?;
        // A fresh GET contains neither the token-bearing POST body nor its credentials.
        response = http
            .get(url)
            .send()
            .await
            .map_err(|error| sheet_network_error(error, "nội dung Google"))?;
        content_hop = true;
    }
    let status = response.status();
    if !status.is_success() {
        return Err(SheetTransportError {
            message: format!(
                "Kết nối ghi trả HTTP {status} tại {}",
                if content_hop {
                    "nội dung Google"
                } else {
                    "webhook"
                }
            ),
            retryable: retry_sheet_status(status, content_hop),
        });
    }
    limited_body(response).await.map_err(|error| {
        let retryable = error
            .downcast_ref::<reqwest::Error>()
            .is_some_and(|error| error.is_body() || error.is_timeout());
        SheetTransportError {
            message: error.to_string(),
            retryable,
        }
    })
}

async fn sheet_post_body(
    webhook: &str,
    payload: &serde_json::Value,
    budget: Duration,
    attempts: u32,
) -> anyhow::Result<String> {
    let http = client()?;
    let deadline = tokio::time::Instant::now() + budget;
    for attempt in 0..attempts {
        // Reserve the remaining backoffs; all retries start at the original webhook.
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        let delay_reserve = Duration::from_secs((attempt + 1..attempts).map(u64::from).sum());
        let allowance = remaining.saturating_sub(delay_reserve) / (attempts - attempt);
        let result = tokio::time::timeout(allowance, sheet_post_once(&http, webhook, payload))
            .await
            .unwrap_or_else(|_| {
                Err(SheetTransportError {
                    message: "Kết nối Sheet quá thời gian chờ".into(),
                    retryable: true,
                })
            });
        match result {
            Ok(body) => return Ok(body),
            Err(error) => {
                if !error.retryable || attempt + 1 == attempts {
                    anyhow::bail!("{} (lượt {}/{})", error.message, attempt + 1, attempts);
                }
                tracing::warn!(attempt = attempt + 1, stage_error = %error.message, "Sheet connection will retry from webhook");
                let delay = Duration::from_secs(u64::from(attempt + 1));
                if tokio::time::Instant::now() + delay >= deadline {
                    anyhow::bail!("{}; hết thời gian kết nối", error.message);
                }
                tokio::time::sleep(delay).await;
            }
        }
    }
    anyhow::bail!("Sheet chưa có lượt kết nối")
}

/// Whether a webhook URL is one this client will send a credential to.
///
/// **HTTPS only.** The token, the post link, the poster and every partner name travel in the
/// body; over `http://` they travel in the clear, and whoever reads the token can write
/// arbitrary rows into the operator's sheet from then on. A settings field is exactly the
/// place a `http://` typo survives unnoticed, which is why this is checked rather than
/// documented.
///
/// The host is not pinned to Google: a proxy in front of the script is a reasonable setup and
/// refusing it would push the operator toward turning the check off entirely.
pub fn is_acceptable_webhook(url: &str) -> bool {
    url::Url::parse(url.trim()).is_ok_and(|parsed| {
        parsed.scheme() == "https" && parsed.host_str().is_some_and(|host| !host.is_empty())
    })
}

/// Push one row, and say plainly whether the sheet now has it.
///
/// # A duplicate is a success
///
/// `duplicate: true` means the script had already written this assignment — which is what a
/// retry after a timeout looks like from the far side — so it returns `Ok(())`. Treating it
/// as a failure would leave the row in the outbox forever, retrying something that is
/// already done.
pub async fn push_row(webhook_url: &str, row: &SheetRow) -> anyhow::Result<()> {
    push_row_with_metadata(webhook_url, row, None).await
}

pub async fn push_row_with_metadata(
    webhook_url: &str,
    row: &SheetRow,
    metadata: Option<&InternalReportMetadata>,
) -> anyhow::Result<()> {
    anyhow::ensure!(
        !webhook_url.trim().is_empty(),
        "chưa đặt webhook Apps Script — điền URL trong cài đặt trước khi đẩy link lên Sheet"
    );
    anyhow::ensure!(
        is_acceptable_webhook(webhook_url),
        "webhook Sheet phải là https:// — token và link bài đi trong thân request, và qua \
         http:// thì đi công khai: {webhook_url}"
    );
    anyhow::ensure!(
        !row.token.trim().is_empty(),
        "chưa đặt token cho webhook — URL Apps Script không tự xác thực người gọi, nên thiếu \
         token là bỏ ngỏ cả sheet"
    );
    // A blank link or key is a row the script rejects on every attempt, forever. Refusing here
    // says so once, in the place that can name which field is missing.
    anyhow::ensure!(
        !row.post_url.trim().is_empty() && !row.assignment_id.trim().is_empty(),
        "thiếu link bài hoặc assignmentId — script sẽ từ chối mãi mà không ai biết vì sao"
    );
    let mut payload = serde_json::to_value(row)?;
    if let Some(metadata) = metadata {
        let fields = serde_json::to_value(metadata)?;
        payload
            .as_object_mut()
            .context("Sheet payload object")?
            .extend(fields.as_object().context("Sheet metadata object")?.clone());
        payload["rowKind"] = serde_json::json!("canonical");
    }
    send_sheet_payload(webhook_url, &row.token, &payload, None).await
}

pub async fn push_internal_report(
    webhook_url: &str,
    token: &str,
    row: &InternalSheetReportRow,
) -> anyhow::Result<()> {
    anyhow::ensure!(
        is_acceptable_webhook(webhook_url) && !token.trim().is_empty(),
        "Sheet nội bộ cần webhook HTTPS và token"
    );
    anyhow::ensure!(
        row.row_kind == "internalReport"
            && row.metadata.report_version == 1
            && !row.assignment_id.trim().is_empty()
            && row.metadata.row_revision >= 0,
        "internal report identity/version missing"
    );
    let mut payload = serde_json::to_value(row)?;
    payload["token"] = serde_json::json!(token);
    send_sheet_payload(
        webhook_url,
        token,
        &payload,
        Some((&row.assignment_id, row.metadata.row_revision)),
    )
    .await
}

async fn send_sheet_payload(
    webhook_url: &str,
    token: &str,
    payload: &serde_json::Value,
    internal_ack: Option<(&str, i64)>,
) -> anyhow::Result<()> {
    let body = sheet_post_body(webhook_url, payload, WEBHOOK_TIMEOUT, 1).await?;
    let reply: SheetReply = serde_json::from_str(&body).map_err(|error| {
        anyhow::anyhow!(
            "webhook Sheet trả thứ không phải JSON ({error}) — thường là do URL trỏ vào bản \
             deploy cũ hoặc chưa đặt quyền truy cập: {}",
            redact_token(&body, token)
        )
    })?;
    interpret(reply)?;
    if let Some((assignment_id, revision)) = internal_ack {
        validate_internal_ack(&body, assignment_id, revision)?;
    }
    Ok(())
}

fn validate_internal_ack(body: &str, assignment_id: &str, revision: i64) -> anyhow::Result<()> {
    let ack: serde_json::Value = serde_json::from_str(body)?;
    anyhow::ensure!(
        ack["ok"] == true
            && ack["reportVersion"] == 1
            && ack["assignmentId"].as_str() == Some(assignment_id)
            && ack["rowRevision"]
                .as_i64()
                .is_some_and(|value| value >= revision),
        "Sheet nội bộ chưa xác nhận đúng hàng/phiên bản; kiểm tra bản Apps Script đã triển khai"
    );
    Ok(())
}

/// The user-facing Google Sheet address, independent of the Apps Script endpoint.
pub const SHEET_URL_SETTING: &str = "publish_sheet_url";
const CHECK_TIMEOUT: Duration = Duration::from_secs(20);
const CHECK_BODY_LIMIT: usize = 2 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SheetCheckResult {
    pub sheet_url: String,
    pub spreadsheet_id: String,
    pub sheet_gid: u64,
    pub readable: bool,
    pub connection_verified: bool,
    pub layout: Option<String>,
    pub columns: Vec<String>,
    pub message: String,
}

fn parse_sheet_url(value: &str) -> anyhow::Result<SheetCheckResult> {
    let parsed = url::Url::parse(value.trim()).context("Link Google Sheet không hợp lệ")?;
    anyhow::ensure!(
        parsed.scheme() == "https"
            && parsed.host_str() == Some("docs.google.com")
            && parsed.username().is_empty()
            && parsed.password().is_none()
            && parsed.port_or_known_default() == Some(443),
        "Dùng link HTTPS của docs.google.com/spreadsheets/d/..."
    );
    let parts: Vec<_> = parsed
        .path_segments()
        .context("Đường dẫn Sheet không hợp lệ")?
        .collect();
    anyhow::ensure!(
        parts.len() >= 3 && parts[0] == "spreadsheets" && parts[1] == "d",
        "Dùng link Google Sheet có mã bảng sau /spreadsheets/d/"
    );
    let id = parts[2];
    anyhow::ensure!(
        !id.is_empty()
            && id.len() <= 128
            && id
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-')),
        "Mã Google Sheet không hợp lệ"
    );
    anyhow::ensure!(
        parts[3..]
            .iter()
            .all(|part| part.is_empty() || *part == "edit"),
        "Dùng link chỉnh sửa Google Sheet, không dùng link xuất bản hoặc tải xuống"
    );
    let mut gids = parsed
        .query_pairs()
        .filter(|(k, _)| k == "gid")
        .map(|(_, v)| v.into_owned())
        .collect::<Vec<_>>();
    if let Some(fragment) = parsed.fragment() {
        gids.extend(
            url::form_urlencoded::parse(fragment.as_bytes())
                .filter(|(k, _)| k == "gid")
                .map(|(_, v)| v.into_owned()),
        );
    }
    let mut gid = None;
    for value in gids {
        let next = value
            .parse::<u64>()
            .context("gid của tab phải là số không âm")?;
        anyhow::ensure!(next <= i32::MAX as u64, "gid của tab vượt giới hạn");
        anyhow::ensure!(
            gid.is_none_or(|old| old == next),
            "Link có nhiều gid khác nhau"
        );
        gid = Some(next);
    }
    let gid = gid.unwrap_or(0);
    Ok(SheetCheckResult {
        sheet_url: format!("https://docs.google.com/spreadsheets/d/{id}/edit#gid={gid}"),
        spreadsheet_id: id.into(),
        sheet_gid: gid,
        readable: false,
        connection_verified: false,
        layout: None,
        columns: vec![],
        message: String::new(),
    })
}

fn check_columns(columns: &[String]) -> anyhow::Result<String> {
    let normalized = columns
        .iter()
        .map(|s| s.trim().trim_start_matches('\u{feff}').to_lowercase())
        .collect::<Vec<_>>();
    anyhow::ensure!(
        normalized.len() >= 4,
        "Sheet thiếu các cột STT, Người air, Ngày, Link"
    );
    let c = |n| normalized.get(n).map(String::as_str).unwrap_or("");
    if c(0) == "stt" && c(1) == "người air" && c(2) == "ngày" && c(3) == "link" {
        if c(4) == "máy"
            && c(5) == "tài khoản tiktok"
            && c(6) == "trạng thái"
            && c(7) == "lỗi hoặc ghi chú"
        {
            anyhow::ensure!(
                c(8) == "đối tác",
                "Sheet nội bộ thiếu cột Đối tác sau bốn cột báo cáo"
            );
            return Ok("internal".into());
        }
        anyhow::ensure!(c(4) == "đối tác", "Sheet thiếu cột Đối tác ngay sau Link");
        return Ok("compact".into());
    }
    if matches!(c(1), "nhân viên" | "người đăng") && c(3).contains("link") && c(10) == "đối tác"
    {
        return Ok("legacy".into());
    }
    anyhow::bail!("Tiêu đề bảng chưa khớp mẫu Đăng bài")
}

fn csv_columns(body: &str, content_type: &str) -> anyhow::Result<Vec<String>> {
    anyhow::ensure!(body.len() <= CHECK_BODY_LIMIT, "Phản hồi Sheet vượt 2 MiB");
    let clean = body.trim_start_matches('\u{feff}').trim_start();
    anyhow::ensure!(
        !content_type.to_ascii_lowercase().contains("text/html") && !clean.starts_with('<'),
        "Google yêu cầu đăng nhập hoặc chưa chia sẻ quyền đọc bảng"
    );
    let mut fields = Vec::new();
    let mut value = String::new();
    let mut quoted = false;
    let mut chars = clean.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '"' {
            if quoted && chars.peek() == Some(&'"') {
                chars.next();
                value.push('"');
            } else {
                quoted = !quoted;
            }
        } else if c == ',' && !quoted {
            fields.push(std::mem::take(&mut value));
        } else if matches!(c, '\r' | '\n') && !quoted {
            break;
        } else {
            value.push(c);
        }
    }
    anyhow::ensure!(!quoted, "Hàng tiêu đề CSV chưa kết thúc dấu nháy");
    fields.push(value);
    check_columns(&fields)?;
    Ok(fields)
}

fn public_check_client() -> anyhow::Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .timeout(CHECK_TIMEOUT)
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            let host = attempt.url().host_str().unwrap_or("");
            if attempt.previous().len() > 3
                || attempt.url().scheme() != "https"
                || !(host == "docs.google.com"
                    || host == "drive.google.com"
                    || host.ends_with(".googleusercontent.com"))
            {
                attempt.error("Sheet chuyển hướng đến trang đăng nhập hoặc host khác")
            } else {
                attempt.follow()
            }
        }))
        .build()?)
}

async fn limited_body(mut response: reqwest::Response) -> anyhow::Result<String> {
    anyhow::ensure!(
        response
            .content_length()
            .is_none_or(|length| length <= CHECK_BODY_LIMIT as u64),
        "Phản hồi Sheet vượt 2 MiB"
    );
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|error| anyhow::Error::new(error.without_url()))?
    {
        anyhow::ensure!(
            bytes.len() + chunk.len() <= CHECK_BODY_LIMIT,
            "Phản hồi Sheet vượt 2 MiB"
        );
        bytes.extend_from_slice(&chunk);
    }
    String::from_utf8(bytes).context("Phản hồi Sheet không phải UTF-8")
}

fn validate_sheet_check_ack(
    body: &str,
    expected: &SheetCheckResult,
    token: &str,
) -> anyhow::Result<(String, Vec<String>)> {
    let ack: serde_json::Value = serde_json::from_str(body).map_err(|_| {
        anyhow::anyhow!(
            "Kết nối ghi chưa hỗ trợ kiểm tra; cập nhật Apps Script. {}",
            redact_token(body, token)
        )
    })?;
    anyhow::ensure!(
        ack["ok"] == true,
        "Kết nối ghi từ chối: {}",
        redact_token(
            ack["error"]
                .as_str()
                .unwrap_or("cần cập nhật Apps Script hoặc kiểm tra token"),
            token
        )
    );
    anyhow::ensure!(
        ack["checkVersion"] == 1,
        "Cập nhật Apps Script để hỗ trợ kiểm tra kết nối không ghi dữ liệu"
    );
    anyhow::ensure!(
        ack["spreadsheetId"].as_str() == Some(expected.spreadsheet_id.as_str())
            && ack["sheetGid"].as_u64() == Some(expected.sheet_gid),
        "Kết nối ghi hiện trỏ tới bảng hoặc tab khác; link mới chưa được kết nối"
    );
    let columns = ack["columns"]
        .as_array()
        .context("Kết nối không trả tiêu đề bảng")?
        .iter()
        .map(|v| {
            v.as_str()
                .map(str::to_owned)
                .context("Tiêu đề bảng không hợp lệ")
        })
        .collect::<anyhow::Result<Vec<_>>>()?;
    let layout = check_columns(&columns)?;
    anyhow::ensure!(
        ack["layout"].as_str() == Some(layout.as_str()),
        "Bố cục kiểm tra không khớp tiêu đề"
    );
    Ok((layout, columns))
}

/// Check read access and the configured writer's actual target without writing a Sheet row.
pub async fn check_sheet(
    value: &str,
    settings: &SheetDeliverySettings,
) -> anyhow::Result<SheetCheckResult> {
    let mut result = parse_sheet_url(value)?;
    let read = async {
        let url = format!(
            "https://docs.google.com/spreadsheets/d/{}/export?format=csv&gid={}",
            result.spreadsheet_id, result.sheet_gid
        );
        let response = public_check_client()?.get(url).send().await?;
        anyhow::ensure!(
            response.status().is_success(),
            "Google chưa cho phép đọc bảng (HTTP {})",
            response.status()
        );
        let kind = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_owned();
        let body = limited_body(response).await?;
        let columns = csv_columns(&body, &kind)?;
        Ok::<_, anyhow::Error>((check_columns(&columns)?, columns))
    }
    .await;
    let mut details = Vec::new();
    match read {
        Ok((layout, columns)) => {
            result.readable = true;
            result.layout = Some(layout);
            result.columns = columns;
        }
        Err(error) => details.push(error.to_string()),
    };
    if is_acceptable_webhook(&settings.webhook_url) && !settings.token.trim().is_empty() {
        let checked=async {
            let payload=serde_json::json!({"rowKind":"check","checkVersion":1,"token":settings.token,"spreadsheetId":result.spreadsheet_id,"sheetGid":result.sheet_gid});
            let body = sheet_post_body(&settings.webhook_url, &payload, CHECK_TIMEOUT, 3).await?;
            validate_sheet_check_ack(&body, &result, &settings.token)
        }.await;
        match checked {
            Ok((layout, columns)) => {
                result.readable = true;
                result.connection_verified = true;
                result.layout = Some(layout);
                result.columns = columns;
            }
            Err(error) => details.push(redact_token(&error.to_string(), &settings.token)),
        }
    } else {
        details.push("Chưa cấu hình kết nối ghi cho bảng này".into());
    }
    result.message = if result.connection_verified {
        "Đã kiểm tra đúng bảng và tab qua kết nối ghi; không thêm dữ liệu thử".into()
    } else if result.readable {
        format!(
            "Đọc được bảng; chưa xác minh kết nối ghi. {}",
            details.join(". ")
        )
    } else {
        format!("Chưa kiểm tra được bảng. {}", details.join(". "))
    };
    Ok(result)
}

/// Explicit initialization endpoint, separate from the read-only connection check.
pub async fn prepare_sheet(
    value: &str,
    settings: &SheetDeliverySettings,
) -> anyhow::Result<SheetCheckResult> {
    let mut result = parse_sheet_url(value)?;
    anyhow::ensure!(
        is_acceptable_webhook(&settings.webhook_url) && !settings.token.trim().is_empty(),
        "Chưa có kết nối ghi cho Sheet này"
    );
    let payload = serde_json::json!({"rowKind":"prepare","checkVersion":1,"token":settings.token,"spreadsheetId":result.spreadsheet_id,"sheetGid":result.sheet_gid});
    let body = sheet_post_body(&settings.webhook_url, &payload, Duration::from_secs(45), 3).await?;
    let (layout, columns) = validate_sheet_check_ack(&body, &result, &settings.token)?;
    result.readable = true;
    result.connection_verified = true;
    result.layout = Some(layout);
    result.columns = columns;
    result.message = "Sheet đã sẵn sàng ghi kết quả".into();
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **The two halves of this wire are in different languages, and nothing else checks
    /// they agree.**
    ///
    /// The payload is built in Rust and read in Apps Script. No compiler sees both, no type
    /// spans them, and the failure mode is silent on the worst possible schedule: a renamed
    /// field means the script writes a blank cell — or refuses every row — the first time a
    /// real campaign publishes, which is exactly when nobody is watching a log.
    ///
    /// So the script itself is the fixture. Both directions are checked, and the second is
    /// the one that catches drift: a field the script reads that this struct never sends
    /// would be `undefined` on arrival, and `String(undefined || '')` is an empty string,
    /// which the script happily writes into a cell.
    #[test]
    fn every_field_the_apps_script_reads_is_a_field_this_payload_sends() {
        let script = include_str!("../../../docs/apps-script/publish-sheet.gs");
        let sent = serde_json::to_value(SheetRow {
            token: "t".into(),
            post_url: "https://vt.tiktok.com/ZSVcq8mha/".into(),
            poster: "@cn.qut.lt4".into(),
            partners: vec!["Quán A".into()],
            assignment_id: "a-1".into(),
            posted_at: Some("2026-09-08T19:46:32Z".into()),
        })
        .expect("the payload serialises");
        let mut sent: Vec<String> = sent
            .as_object()
            .expect("a JSON object")
            .keys()
            .cloned()
            .collect();

        for field in &sent {
            assert!(
                script.contains(&format!("payload.{field}")),
                "the app sends `{field}` and the Apps Script never reads it — the value is \
                 dropped on arrival, silently"
            );
        }
        let internal = serde_json::to_value(internal_fixture()).unwrap();
        sent.extend(internal.as_object().unwrap().keys().cloned());
        sent.extend(["checkVersion", "spreadsheetId", "sheetGid"].map(str::to_owned));

        // Every `payload.<name>` the script mentions, harvested from its own text.
        let mut read: Vec<String> = script
            .match_indices("payload.")
            .map(|(at, _)| {
                script[at + "payload.".len()..]
                    .chars()
                    .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                    .collect::<String>()
            })
            .filter(|name| !name.is_empty())
            .collect();
        read.sort();
        read.dedup();
        assert!(!read.is_empty(), "the script fixture parsed to nothing");
        for field in &read {
            assert!(
                sent.contains(field),
                "the Apps Script reads `payload.{field}`, which this payload does not send — \
                 it arrives `undefined` and lands in the sheet as an empty cell"
            );
        }
    }

    fn internal_fixture() -> InternalSheetReportRow {
        InternalSheetReportRow {
            row_kind: "internalReport".into(),
            metadata: InternalReportMetadata {
                report_version: 1,
                row_revision: 7,
                machine: "Máy 17".into(),
                tiktok_account: String::new(),
                status: "Chưa đăng".into(),
                state_notes: String::new(),
            },
            assignment_id: "assignment-fixture".into(),
            post_url: String::new(),
            poster: "bot".into(),
            partners: vec!["Partner A".into()],
            posted_at: None,
        }
    }

    #[test]
    fn sheet_check_parses_only_normal_google_sheet_targets_and_unambiguous_gid() {
        let target = parse_sheet_url(
            "https://docs.google.com/spreadsheets/d/fixture_Ab-12/edit?gid=37#gid=37",
        )
        .unwrap();
        assert_eq!(target.sheet_gid, 37);
        assert_eq!(target.spreadsheet_id, "fixture_Ab-12");
        assert_eq!(
            target.sheet_url,
            "https://docs.google.com/spreadsheets/d/fixture_Ab-12/edit#gid=37"
        );
        for url in [
            "http://docs.google.com/spreadsheets/d/abc/edit",
            "https://docs.google.com.evil.test/spreadsheets/d/abc/edit",
            "https://secret@docs.google.com/spreadsheets/d/abc/edit",
            "https://docs.google.com:444/spreadsheets/d/abc/edit",
            "https://docs.google.com/spreadsheets/d/abc/edit?gid=1#gid=2",
            "https://docs.google.com/spreadsheets/d/%2e%2e/edit",
            "https://docs.google.com/spreadsheets/d/abc/edit#gid=-1",
        ] {
            assert!(parse_sheet_url(url).is_err(), "accepted {url}");
        }
    }

    #[test]
    fn sheet_check_rejects_login_html_and_damaged_headers_and_reads_quoted_csv() {
        for body in [
            "<!DOCTYPE html><html>Sign in</html>",
            "<html>STT,Người air,Ngày,Link,Đối tác</html>",
            "STT,broken,Ngày,Link,Đối tác",
        ] {
            assert!(csv_columns(body, "text/csv").is_err());
        }
        let cols = csv_columns(
            "\u{feff}\"STT\",\"Người air\",Ngày,Link,Đối tác\r\n1,bot",
            "text/csv",
        )
        .unwrap();
        assert_eq!(check_columns(&cols).unwrap(), "compact");
        assert!(csv_columns("STT,Người air,Ngày,Link,Đối tác", "text/html").is_err());
        assert!(csv_columns(&"x".repeat(CHECK_BODY_LIMIT + 1), "text/csv").is_err());
    }

    #[test]
    fn sheet_check_requires_actual_target_version_and_redacts_token_in_errors() {
        let target =
            parse_sheet_url("https://docs.google.com/spreadsheets/d/fixture/edit#gid=7").unwrap();
        let ack = serde_json::json!({"ok":true,"checkVersion":1,"spreadsheetId":"fixture","sheetGid":7,"layout":"compact","columns":["STT","Người air","Ngày","Link","Đối tác"]});
        assert!(validate_sheet_check_ack(&ack.to_string(), &target, "token-secret").is_ok());
        for field in ["spreadsheetId", "sheetGid", "checkVersion"] {
            let mut wrong = ack.clone();
            wrong[field] = serde_json::json!("wrong");
            assert!(validate_sheet_check_ack(&wrong.to_string(), &target, "token-secret").is_err());
        }
        let error = validate_sheet_check_ack(
            r#"{"ok":false,"error":"token-secret rejected"}"#,
            &target,
            "token-secret",
        )
        .unwrap_err();
        assert!(!error.to_string().contains("token-secret"));
        assert!(validate_sheet_check_ack(r#"{"ok":true}"#, &target, "token-secret").is_err());
    }

    #[test]
    fn internal_report_wire_supports_blank_link_and_requires_a_revision_ack() {
        let row = internal_fixture();
        let value = serde_json::to_value(&row).unwrap();
        assert_eq!(value["postUrl"], "");
        assert!(value["postedAt"].is_null());
        assert_eq!(value["status"], "Chưa đăng");
        assert_eq!(value["rowRevision"], 7);
        assert_eq!(value["partners"][0], "Partner A");
        for reply in [
            r#"{"ok":true}"#,
            r#"{"ok":true,"reportVersion":1,"assignmentId":"wrong","rowRevision":7}"#,
            r#"{"ok":true,"reportVersion":1,"assignmentId":"assignment-fixture","rowRevision":6}"#,
            r#"{"ok":false,"reportVersion":1,"assignmentId":"assignment-fixture","rowRevision":9}"#,
        ] {
            assert!(validate_internal_ack(reply, "assignment-fixture", 7).is_err());
        }
        assert!(validate_internal_ack(
            r#"{"ok":true,"reportVersion":1,"assignmentId":"assignment-fixture","rowRevision":8}"#,
            "assignment-fixture",
            7
        )
        .is_ok());
    }

    /// **The shipped script's column numbers depend on each other, and nothing else checks
    /// they agree.**
    ///
    /// Measured 31/08/2026 against the operator's real sheet: the template shipped
    /// `KEY_COLUMN: 26`, which is column Z, which on that sheet is `Đối tác 16`. Every
    /// delivery would have overwritten a partner name — and each number was individually
    /// legal, so nothing anywhere said so. `PARTNERS_MAX: 12` was wrong the same way
    /// (23 partner columns exist, and rows use all 23).
    ///
    /// The script grew its own runtime guard (`assertConfigIsSane`). This is the build-time
    /// half: the file is data, not code, so the only way to hold it is to read it.
    #[test]
    fn the_shipped_apps_script_config_cannot_overwrite_a_partner_column() {
        let script = include_str!("../../../docs/apps-script/publish-sheet.gs");
        let number = |key: &str| -> i64 {
            let at = script
                .find(&format!("{key}:"))
                .unwrap_or_else(|| panic!("the script still defines {key}"));
            script[at + key.len() + 1..]
                .trim_start()
                .chars()
                .take_while(char::is_ascii_digit)
                .collect::<String>()
                .parse()
                .unwrap_or_else(|_| panic!("{key} is not a plain number"))
        };
        let first = number("PARTNERS_START_COLUMN");
        let max = number("PARTNERS_MAX");
        let key = number("KEY_COLUMN");
        let link = number("LINK_COLUMN");
        let poster = number("POSTER_COLUMN");
        let last = first + max - 1;

        assert!(max >= 1, "PARTNERS_MAX must be at least one column");
        for (name, column) in [
            ("KEY_COLUMN", key),
            ("LINK_COLUMN", link),
            ("POSTER_COLUMN", poster),
        ] {
            assert!(
                column < first || column > last,
                "{name} = {column} sits inside the partner block {first}..={last}; every \
                 delivery would overwrite a partner name, and the sheet cannot say which"
            );
        }
        assert_ne!(key, link, "the key and the link cannot share a column");
        assert_ne!(key, poster, "the key and the poster cannot share a column");

        // The runtime guard has to still be wired, or the script ships without the check
        // this test is the twin of.
        assert!(
            script.contains("assertConfigIsSane()"),
            "the script's own config guard is no longer called from doPost"
        );
    }

    /// **The one host the Apps Script protocol names, and nothing that merely looks like it.**
    ///
    /// Measured 31/08/2026 against the live deployment: `POST /exec` answers 302 to
    /// `script.googleusercontent.com`, and the reply body exists only there. Refusing every
    /// redirect meant refusing Apps Script itself — and worse than plainly: the POST had
    /// already reached the script, which had already written the row, so the client reported
    /// a failure for a delivery that succeeded. A wrong answer, not a missing one.
    ///
    /// The allowance is exactly one host. `googleusercontent.com` without the `script.`
    /// prefix is user content on Google's domain, and `script.googleusercontent.com.evil.tld`
    /// is somebody else's host that ends in the right characters — both refused.
    #[test]
    fn only_the_apps_script_content_host_is_followed() {
        assert!(is_script_content_host("script.googleusercontent.com"));
        assert!(
            is_script_content_host("SCRIPT.GoogleUserContent.COM"),
            "hosts are case-insensitive"
        );
        assert!(
            is_script_content_host("script.googleusercontent.com."),
            "a trailing dot is the same host in DNS"
        );
        for other in [
            "googleusercontent.com",
            "lh3.googleusercontent.com",
            "script.googleusercontent.com.evil.tld",
            "evil.tld",
            "script.google.com",
            "www.google.com",
            "",
        ] {
            assert!(
                !is_script_content_host(other),
                "{other} must not be followed with the token's request"
            );
        }
    }

    /// **A credential never goes out over plaintext, and never follows a redirect.**
    ///
    /// The token, the post link and every partner name are in the body. Over `http://` they
    /// are readable by anything on the path, and whoever reads the token can write arbitrary
    /// rows into the operator's sheet from then on — which is a settings-field typo away.
    #[test]
    fn only_an_https_webhook_is_acceptable() {
        assert!(is_acceptable_webhook(
            "https://script.google.com/macros/s/AKf/exec"
        ));
        assert!(is_acceptable_webhook("  https://proxy.example/hook  "));
        for bad in [
            "http://script.google.com/macros/s/AKf/exec",
            "script.google.com/macros/s/AKf/exec",
            "ftp://example/x",
            "https://",
            "",
            "javascript:alert(1)",
        ] {
            assert!(!is_acceptable_webhook(bad), "{bad} should be refused");
        }
    }

    /// **`duplicate` does not override an explicit failure.**
    ///
    /// Checking it first accepted `{"ok":false,"duplicate":true,"error":"write failed"}` as a
    /// success, and the row would be marked delivered against a sheet holding nothing. Tested
    /// through `interpret` rather than through `push_row`, because the decision is otherwise
    /// only reachable behind an HTTP call — which is exactly why no test saw it.
    #[test]
    fn a_reply_that_claims_a_duplicate_and_a_failure_is_a_failure() {
        let contradictory: SheetReply =
            serde_json::from_str(r#"{"ok":false,"duplicate":true,"error":"write failed"}"#)
                .expect("parses");
        let error = interpret(contradictory).expect_err("a failed write is not a success");
        assert!(error.to_string().contains("write failed"), "{error}");

        // The shape the current script sends for a row it already has.
        let real: SheetReply =
            serde_json::from_str(r#"{"ok":true,"duplicate":true}"#).expect("parses");
        interpret(real).expect("an already-written row is done, not owed");

        let refused: SheetReply =
            serde_json::from_str(r#"{"ok":false,"error":"token sai"}"#).expect("parses");
        assert!(interpret(refused)
            .expect_err("a refusal is a refusal")
            .to_string()
            .contains("token sai"));
    }

    /// **A limit that wraps negative would unbound the sweep, not bound it.**
    #[test]
    fn a_sweep_limit_is_never_negative_however_absurd_the_request() {
        assert_eq!(sweep_limit(5), 5);
        assert_eq!(sweep_limit(0), 0);
        assert!(sweep_limit(usize::MAX) > 0, "usize::MAX became `no limit`");
        assert!(sweep_limit(usize::MAX) <= 1_000);
        for request in [1usize, 999, 1_000, 1_001, usize::MAX / 2, usize::MAX] {
            assert!(
                sweep_limit(request) >= 0,
                "{request} produced a negative LIMIT, which SQLite reads as unbounded"
            );
        }
    }

    /// A blank link is refused before a request that would be rejected forever.
    #[tokio::test]
    async fn a_blank_link_or_key_refuses_rather_than_queueing_a_permanent_rejection() {
        let row = SheetRow {
            token: "t".into(),
            post_url: "   ".into(),
            poster: "bot".into(),
            partners: vec![],
            assignment_id: "assign-1".into(),
            posted_at: Some("2026-09-08T19:46:32Z".into()),
        };
        let error = push_row("https://example/hook", &row)
            .await
            .expect_err("must refuse");
        assert!(error.to_string().contains("link bài"), "{error}");
    }

    /// **A row cannot travel without a token, and the check is not the URL's job.**
    ///
    /// An Apps Script web app reachable from this desktop is reachable by anyone holding
    /// the URL — Google authenticates nothing. So refusing here on an empty token is the
    /// only thing standing between a mistyped setting and an open write endpoint on the
    /// operator's sheet.
    #[tokio::test]
    async fn a_missing_token_or_url_refuses_before_any_request_goes_out() {
        let row = SheetRow {
            token: String::new(),
            post_url: "https://www.tiktok.com/@a/photo/1".into(),
            poster: "bot".into(),
            partners: vec!["Quán A".into()],
            assignment_id: "assign-1".into(),
            posted_at: Some("2026-09-08T19:46:32Z".into()),
        };
        // **https**, and a port nothing listens on: the scheme check now runs first, so an
        // `http://` fixture here would fail for the wrong reason — and that message happens to
        // contain the word `token`, so the assertion below would have passed anyway.
        let error = push_row("https://127.0.0.1:1/never", &row)
            .await
            .expect_err("an empty token must refuse");
        assert!(
            error.to_string().contains("chưa đặt token"),
            "refused for some other reason: {error}"
        );

        let with_token = SheetRow {
            token: "t".into(),
            ..row
        };
        let error = push_row("   ", &with_token)
            .await
            .expect_err("an empty URL must refuse");
        assert!(error.to_string().contains("webhook"), "{error}");
    }

    /// The wire shape is the script's contract, so it is pinned here rather than only there.
    #[test]
    fn the_payload_names_the_fields_the_script_reads() {
        let row = SheetRow {
            token: "secret".into(),
            post_url: "https://www.tiktok.com/@a/photo/1".into(),
            // A device handle travels verbatim; `bot` is only the fallback for a phone whose
            // handle was never typed in (see the field's doc — the script falls back too).
            poster: "@cn.qut.lt4".into(),
            partners: vec!["Quán A".into(), "Quán B".into()],
            assignment_id: "assign-1".into(),
            posted_at: Some("2026-09-08T19:46:32Z".into()),
        };
        let json = serde_json::to_value(&row).expect("serialises");
        assert_eq!(json["postUrl"], "https://www.tiktok.com/@a/photo/1");
        assert_eq!(json["poster"], "@cn.qut.lt4");
        assert_eq!(json["assignmentId"], "assign-1");
        assert_eq!(json["partners"][1], "Quán B");
        // Order is meaning here: the names go across columns K, L, M… in this order, so a
        // set or a sorted list would silently rearrange the operator's sheet.
        assert_eq!(
            json["partners"].as_array().expect("array").len(),
            2,
            "partners travel as an ordered array"
        );
    }

    /// **A duplicate is a success**, or the row retries forever against a sheet that has it.
    #[test]
    fn the_reply_shape_treats_an_already_written_row_as_done() {
        let reply: SheetReply =
            serde_json::from_str(r#"{"ok":false,"duplicate":true}"#).expect("parses");
        assert!(reply.duplicate);
        // And the fields the script may omit default rather than failing the parse: a
        // parse error here would be reported as "the sheet refused", which is a different
        // problem from the one an operator would then go looking for.
        let minimal: SheetReply = serde_json::from_str(r#"{"ok":true}"#).expect("parses");
        assert!(minimal.ok);
        assert!(!minimal.duplicate);
        assert!(minimal.error.is_none());
    }
}

#[cfg(test)]
mod connection_recovery_tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    async fn endpoint(statuses: Vec<u16>) -> (String, tokio::task::JoinHandle<Vec<String>>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/exec", listener.local_addr().unwrap());
        let task = tokio::spawn(async move {
            let mut requests = Vec::new();
            for status in statuses {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut data = Vec::new();
                loop {
                    let mut part = [0; 4096];
                    let n = socket.read(&mut part).await.unwrap();
                    if n == 0 {
                        break;
                    }
                    data.extend_from_slice(&part[..n]);
                    let text = String::from_utf8_lossy(&data);
                    if let Some(end) = text.find("\r\n\r\n") {
                        let length = text[..end]
                            .lines()
                            .find_map(|line| {
                                line.to_ascii_lowercase()
                                    .strip_prefix("content-length:")
                                    .and_then(|value| value.trim().parse::<usize>().ok())
                            })
                            .unwrap_or(0);
                        if data.len() >= end + 4 + length {
                            break;
                        }
                    }
                }
                requests.push(String::from_utf8(data).unwrap());
                let body = r#"{"ok":true}"#;
                let reply = format!("HTTP/1.1 {status} Result\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                socket.write_all(reply.as_bytes()).await.unwrap();
            }
            requests
        });
        (url, task)
    }

    #[tokio::test]
    async fn retry_uses_the_original_post_and_stops_after_success() {
        let (url, requests) = endpoint(vec![503, 429, 200]).await;
        let body = sheet_post_body(
            &url,
            &serde_json::json!({"rowKind":"prepare","token":"fixture"}),
            Duration::from_secs(10),
            3,
        )
        .await
        .unwrap();
        assert_eq!(body, r#"{"ok":true}"#);
        let requests = requests.await.unwrap();
        assert_eq!(requests.len(), 3);
        assert!(requests.iter().all(
            |request| request.starts_with("POST /exec HTTP/1.1") && request.contains("fixture")
        ));
    }

    #[tokio::test]
    async fn original_webhook_404_is_not_retried() {
        let (url, requests) = endpoint(vec![404]).await;
        let error = sheet_post_body(&url, &serde_json::json!({}), Duration::from_secs(10), 3)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("404"));
        assert!(error.to_string().contains("lượt 1/3"));
        assert_eq!(requests.await.unwrap().len(), 1);
    }

    #[test]
    fn only_content_404_is_transient_and_redirect_never_preserves_a_body() {
        assert!(retry_sheet_status(reqwest::StatusCode::NOT_FOUND, true));
        assert!(!retry_sheet_status(reqwest::StatusCode::NOT_FOUND, false));
        assert!(!retry_sheet_status(reqwest::StatusCode::FORBIDDEN, true));
        let good = "https://script.googleusercontent.com/macros/echo?user_content_key=fixture";
        for status in [302, 303] {
            assert!(sheet_redirect(reqwest::StatusCode::from_u16(status).unwrap(), good).is_ok());
        }
        for status in [301, 307, 308] {
            assert!(sheet_redirect(reqwest::StatusCode::from_u16(status).unwrap(), good).is_err());
        }
        for url in [
            "https://example.com/?user_content_key=fixture",
            "http://script.googleusercontent.com/",
            "https://script.googleusercontent.com:444/",
        ] {
            let error = sheet_redirect(reqwest::StatusCode::FOUND, url)
                .unwrap_err()
                .to_string();
            assert!(!error.contains("fixture"));
        }
    }
}
