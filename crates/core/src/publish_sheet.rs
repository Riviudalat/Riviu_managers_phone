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
    /// Who posted it. `bot` for everything this app publishes.
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

fn client() -> anyhow::Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .timeout(WEBHOOK_TIMEOUT)
        .build()?)
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
        !row.token.trim().is_empty(),
        "chưa đặt token cho webhook — URL Apps Script không tự xác thực người gọi, nên thiếu \
         token là bỏ ngỏ cả sheet"
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
    anyhow::ensure!(
        status.is_success(),
        "webhook Sheet trả {status}: {}",
        body.chars().take(200).collect::<String>()
    );
    let reply: SheetReply = serde_json::from_str(&body).map_err(|error| {
        anyhow::anyhow!(
            "webhook Sheet trả thứ không phải JSON ({error}) — thường là do URL trỏ vào bản \
             deploy cũ hoặc chưa đặt quyền truy cập: {}",
            body.chars().take(200).collect::<String>()
        )
    })?;
    if reply.duplicate {
        return Ok(());
    }
    anyhow::ensure!(
        reply.ok,
        "webhook Sheet từ chối: {}",
        reply.error.unwrap_or_else(|| "không nói lý do".into())
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

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
        // A URL that would fail loudly if it were ever reached.
        let error = push_row("http://127.0.0.1:1/never", &row)
            .await
            .expect_err("an empty token must refuse");
        assert!(error.to_string().contains("token"), "{error}");

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
            poster: "bot".into(),
            partners: vec!["Quán A".into(), "Quán B".into()],
            assignment_id: "assign-1".into(),
        };
        let json = serde_json::to_value(&row).expect("serialises");
        assert_eq!(json["postUrl"], "https://www.tiktok.com/@a/photo/1");
        assert_eq!(json["poster"], "bot");
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
