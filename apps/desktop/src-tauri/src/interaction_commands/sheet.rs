//! Google Visualization JSON is parsed as data, never evaluated as JavaScript.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{collections::HashMap, time::Duration};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SheetRow {
    pub row: usize,
    pub line: riviu_core::TikTokLinkLine,
    pub duplicate_of: Option<usize>,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SheetImport {
    pub source_url: String,
    pub column: String,
    pub digest: String,
    pub rows: Vec<SheetRow>,
}
#[derive(Deserialize)]
struct Response {
    status: String,
    table: Option<Table>,
}
#[derive(Deserialize)]
struct Table {
    rows: Vec<Row>,
}
#[derive(Deserialize)]
struct Row {
    c: Vec<Option<Cell>>,
}
#[derive(Deserialize)]
struct Cell {
    v: Option<serde_json::Value>,
}

fn query_url(raw: &str, column: &str) -> anyhow::Result<reqwest::Url> {
    let input = reqwest::Url::parse(raw.trim())?;
    anyhow::ensure!(
        input.scheme() == "https"
            && input.host_str() == Some("docs.google.com")
            && input.username().is_empty()
            && input.password().is_none()
            && input.port_or_known_default() == Some(443),
        "Chỉ nhận link Google Sheets https://docs.google.com"
    );
    let parts: Vec<_> = input.path_segments().into_iter().flatten().collect();
    anyhow::ensure!(
        parts.len() >= 3 && parts[0] == "spreadsheets" && parts[1] == "d",
        "Link Sheet không hợp lệ"
    );
    let id = parts[2];
    anyhow::ensure!(
        !id.is_empty()
            && id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-'),
        "ID Sheet không hợp lệ"
    );
    let gid = input
        .query_pairs()
        .find(|(key, _)| key == "gid")
        .map(|(_, v)| v.into_owned())
        .or_else(|| {
            let mut fragment = input.clone();
            fragment.set_query(input.fragment());
            fragment
                .query_pairs()
                .find(|(key, _)| key == "gid")
                .map(|(_, v)| v.into_owned())
        })
        .unwrap_or_else(|| "0".into());
    anyhow::ensure!(
        !gid.is_empty() && gid.bytes().all(|b| b.is_ascii_digit()),
        "Mã tab Sheet không hợp lệ"
    );
    anyhow::ensure!(
        !column.is_empty() && column.len() <= 3 && column.bytes().all(|b| b.is_ascii_uppercase()),
        "Cột link phải là A đến ZZZ"
    );
    let mut url = reqwest::Url::parse(&format!(
        "https://docs.google.com/spreadsheets/d/{id}/gviz/tq"
    ))?;
    url.query_pairs_mut()
        .append_pair("gid", &gid)
        .append_pair("headers", "0")
        .append_pair("tqx", "out:json")
        .append_pair("range", &format!("{column}1:{column}5001"));
    Ok(url)
}

fn parse_rows(source: &str) -> anyhow::Result<Vec<SheetRow>> {
    let json = source
        .trim()
        .strip_prefix("/*O_o*/")
        .unwrap_or(source.trim())
        .trim()
        .strip_prefix("google.visualization.Query.setResponse(")
        .and_then(|s| s.strip_suffix(");"))
        .ok_or_else(|| anyhow::anyhow!("Sheet không trả dữ liệu đọc được; kiểm tra quyền xem"))?;
    let response: Response = serde_json::from_str(json)?;
    anyhow::ensure!(response.status == "ok", "Google Sheets từ chối yêu cầu đọc");
    let table = response
        .table
        .ok_or_else(|| anyhow::anyhow!("Sheet thiếu bảng dữ liệu"))?;
    anyhow::ensure!(
        table.rows.len() <= 5000,
        "Sheet vượt 5.000 dòng; chia nguồn trước khi nhập"
    );
    let mut seen = HashMap::new();
    let mut rows = Vec::new();
    for (index, row) in table.rows.into_iter().enumerate() {
        let Some(value) = row
            .c
            .first()
            .and_then(Option::as_ref)
            .and_then(|c| c.v.as_ref())
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
        else {
            continue;
        };
        let mut parsed = riviu_core::parse_tiktok_links(value);
        if parsed.len() != 1 {
            rows.push(SheetRow {
                row: index + 1,
                line: riviu_core::TikTokLinkLine {
                    line_no: index + 1,
                    original: value.into(),
                    target: None,
                    error: Some(riviu_core::LinkErrorCode::InvalidUrl),
                },
                duplicate_of: None,
            });
            continue;
        }
        let mut line = parsed.remove(0);
        line.line_no = index + 1;
        let duplicate_of = line.target.as_ref().and_then(|t| {
            if let Some(first) = seen.get(&t.target_key) {
                Some(*first)
            } else {
                seen.insert(t.target_key.clone(), index + 1);
                None
            }
        });
        rows.push(SheetRow {
            row: index + 1,
            line,
            duplicate_of,
        });
    }
    Ok(rows)
}

pub(super) async fn import(source_url: &str, column: &str) -> anyhow::Result<SheetImport> {
    let column = column.trim().to_ascii_uppercase();
    let url = query_url(source_url, &column)?;
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(20))
        .build()?;
    let mut response = client.get(url).send().await?.error_for_status()?;
    anyhow::ensure!(
        response.status().is_success(),
        "Sheet yêu cầu đăng nhập; dùng nguồn đã có quyền xem bằng link"
    );
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        anyhow::ensure!(
            bytes.len() + chunk.len() <= 2 * 1024 * 1024,
            "Dữ liệu Sheet vượt 2 MiB"
        );
        bytes.extend_from_slice(&chunk);
    }
    let rows = parse_rows(std::str::from_utf8(&bytes)?)?;
    Ok(SheetImport {
        source_url: source_url.trim().into(),
        column,
        digest: format!("{:x}", Sha256::digest(&bytes)),
        rows,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[ignore = "Explicit read-only Google Sheet acceptance"]
    async fn live_sheet_read_only() -> anyhow::Result<()> {
        let source = std::env::var("RIVIU_SHEET_INSPECT_URL")?;
        let output = std::env::var("RIVIU_SHEET_INSPECT_REPORT")?;
        let result = import(&source, "D").await?;
        anyhow::ensure!(
            result.rows.iter().any(|row| row.line.target.is_some()),
            "no valid TikTok links"
        );
        println!(
            "rows={} valid={}",
            result.rows.len(),
            result
                .rows
                .iter()
                .filter(|row| row.line.target.is_some() && row.duplicate_of.is_none())
                .count()
        );
        std::fs::write(output, serde_json::to_vec_pretty(&result)?)?;
        Ok(())
    }
    #[test]
    fn sheet_url_is_host_bound_and_respects_fragment_gid() {
        let url = query_url(
            "https://docs.google.com/spreadsheets/d/abc/edit#gid=42",
            "D",
        )
        .unwrap();
        assert!(url.query_pairs().any(|(k, v)| k == "gid" && v == "42"));
        for url in [
            "http://docs.google.com/spreadsheets/d/abc",
            "https://docs.google.com.evil.test/spreadsheets/d/abc",
            "https://user@docs.google.com/spreadsheets/d/abc",
        ] {
            assert!(query_url(url, "D").is_err());
        }
        assert!(query_url("https://docs.google.com/spreadsheets/d/abc", "D;select").is_err());
    }
    #[test]
    fn rows_preserve_physical_numbers_and_duplicates_without_evaluating_code() {
        let value = serde_json::json!({"status":"ok","table":{"rows":[{"c":[null]},{"c":[{"v":"https://www.tiktok.com/@a/photo/123"}]},{"c":[{"v":"https://www.tiktok.com/@a/photo/123"}]},{"c":[{"v":"not a post"}]}]}});
        let rows = parse_rows(&format!(
            "/*O_o*/\ngoogle.visualization.Query.setResponse({value});"
        ))
        .unwrap();
        assert_eq!(rows[0].row, 2);
        assert_eq!(rows[1].duplicate_of, Some(2));
        assert!(rows[2].line.target.is_none());
        assert!(parse_rows("google.visualization.Query.setResponse(alert('x'));").is_err());
    }
}
