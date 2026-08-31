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

use std::time::Duration;

use serde::{Deserialize, Serialize};

/// Settings key holding the deployed web app URL.
pub const WEBHOOK_URL_SETTING: &str = "publish_sheet_webhook_url";
/// Settings key holding the shared token the script checks.
pub const WEBHOOK_TOKEN_SETTING: &str = "publish_sheet_webhook_token";

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
    /// Who posted it: the device account's handle when the fleet has one on file
    /// (`device_meta.handle`, typed in by the operator), else the literal `bot`.
    ///
    /// Two legal values, not one, since 31/08/2026 — twenty accounts publish through this
    /// app, and a column that always reads `bot` cannot tell the operator whose post a row
    /// is. The script keeps its own `|| 'bot'` fallback, and migration 18's CHECK refuses an
    /// empty poster, which is why the fallback lives on this side too rather than trusting
    /// every handle to have been typed in.
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
        // **One measured hop, and it may not carry the body.**
        //
        // The token is a bearer credential in the body, so a redirect that *preserves* the
        // method — 307 and 308 — would hand it to whoever answers. Those are refused
        // outright. A 302/303 turns the follow-up into a GET with no body, which is exactly
        // how Apps Script's own answer is fetched, and is why this hop costs the credential
        // nothing.
        //
        // Bounded at one: `/exec` → content host is the whole protocol, and a chain longer
        // than that is not Apps Script answering.
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            let body_preserving = matches!(attempt.status().as_u16(), 307 | 308);
            let target_ok = attempt.url().scheme() == "https"
                && attempt.url().host_str().is_some_and(is_script_content_host);
            if attempt.previous().len() > 1 {
                attempt.error("webhook Sheet chuyển hướng quá nhiều lần")
            } else if body_preserving {
                attempt.error(
                    "webhook Sheet chuyển hướng kiểu giữ nguyên body (307/308) — token nằm \
                     trong body nên KHÔNG đi theo",
                )
            } else if target_ok {
                attempt.follow()
            } else {
                attempt.error(
                    "webhook Sheet chuyển hướng ra ngoài script.googleusercontent.com — \
                     không đi theo",
                )
            }
        }))
        .build()?)
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
    let response = client()?
        .post(webhook_url)
        .json(row)
        .send()
        .await
        .map_err(|error| anyhow::anyhow!("không gọi được webhook Sheet: {error}"))?;

    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    // Apps Script answers 200 with a JSON body for its own refusals — a non-2xx here is
    // Google's infrastructure, not the script, and the body is an HTML error page. Quoting a
    // slice of it is what tells the two apart in a log.
    //
    // **The slice is redacted first, and that is not paranoia about our own script.** The
    // body comes from whatever host the URL points at; an endpoint that echoes the request
    // — a debug handler, a proxy, a mistyped host — puts the bearer token in it, and this
    // string is stored in `last_error` and written to the app log. A credential that
    // reaches a log has to be re-issued, so the quoted evidence never carries it.
    anyhow::ensure!(
        status.is_success(),
        "webhook Sheet trả {status}: {}",
        redact_token(&body, &row.token)
    );
    let reply: SheetReply = serde_json::from_str(&body).map_err(|error| {
        anyhow::anyhow!(
            "webhook Sheet trả thứ không phải JSON ({error}) — thường là do URL trỏ vào bản \
             deploy cũ hoặc chưa đặt quyền truy cập: {}",
            redact_token(&body, &row.token)
        )
    })?;
    interpret(reply)
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
        })
        .expect("the payload serialises");
        let sent: Vec<String> = sent
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
