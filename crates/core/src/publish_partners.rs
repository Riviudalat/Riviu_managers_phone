//! The partner venues a post promotes, read out of the bundle's `partners-*.xlsx`.
//!
//! These names are not decoration and they are not for the caption. After a post goes out, the
//! operator's record of it is a row in a spreadsheet: the post's link in one column, and from
//! column K onward **these names**. So a name read wrongly is a wrong row about a live post, and
//! a name read as a number — which is exactly what a shared-string table produces if you decode
//! the index instead of the string — is a wrong row that looks plausible.
//!
//! # Why a real zip reader and a hand-rolled XML scan
//!
//! The twenty-one workbooks this was written against store every part **uncompressed**, so a
//! Stored-only reader would work today and produce a confusing failure the first time anyone
//! opens one in Excel and saves it, because Excel deflates. `zip` is already in this workspace's
//! graph twice (`tools/rtmmo-re` and `tauri-plugin-updater`), so using it adds no crate and no
//! licence to approve.
//!
//! The XML, by contrast, is scanned rather than parsed, and deliberately: the question is only
//! "which strings does row 1 hold", the shape is one sheet with one row, and every way the file
//! could differ from that is something this module **refuses** rather than interprets. A parser
//! would add a dependency to be more permissive about a file whose permissiveness is the risk.

use std::io::Read;
use std::path::Path;

/// One row of partner names, in the workbook's own column order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PartnerRow {
    pub names: Vec<String>,
}

/// Every way a partner workbook can fail to be one row of names.
///
/// Each variant exists because interpreting that case would put wrong data into a spreadsheet
/// about a post that is already live and cannot be taken down from here.
#[derive(Debug, thiserror::Error)]
pub enum PartnerReadError {
    #[error("không mở được {path} như một file .xlsx: {source}")]
    NotAWorkbook {
        path: String,
        #[source]
        source: zip::result::ZipError,
    },
    #[error("{path} không có xl/worksheets/sheet1.xml")]
    NoSheet { path: String },
    #[error("{path} đọc không ra chữ: {source}")]
    NotText {
        path: String,
        #[source]
        source: std::string::FromUtf8Error,
    },
    #[error(
        "{path} dùng bảng chuỗi dùng chung (sharedStrings) mà bộ đọc này cố ý không đọc — \
         mở lại bằng Excel và lưu với inline strings, hoặc xuất lại từ công cụ sinh file"
    )]
    SharedStrings { path: String },
    #[error("{path} có dữ liệu ở {rows} hàng; hợp đồng là đúng một hàng tên đối tác")]
    NotOneRow { path: String, rows: usize },
    #[error("{path} không có tên đối tác nào")]
    Empty { path: String },
}

/// The single worksheet this reader will look at.
///
/// By name, never "the first sheet in the archive". These workbooks hold one sheet called
/// `Doi tac`, and guessing which sheet holds the partner list is not a guess a publish path
/// should make on the operator's behalf.
const SHEET_PART: &str = "xl/worksheets/sheet1.xml";

/// Read the one row of partner names out of a `partners-*.xlsx`.
pub fn read_partner_row(path: &Path) -> Result<PartnerRow, PartnerReadError> {
    let shown = path.display().to_string();
    let file = std::fs::File::open(path).map_err(|error| PartnerReadError::NotAWorkbook {
        path: shown.clone(),
        source: zip::result::ZipError::Io(error),
    })?;
    let mut archive =
        zip::ZipArchive::new(file).map_err(|source| PartnerReadError::NotAWorkbook {
            path: shown.clone(),
            source,
        })?;
    let mut sheet = archive
        .by_name(SHEET_PART)
        .map_err(|_| PartnerReadError::NoSheet {
            path: shown.clone(),
        })?;
    let mut bytes = Vec::new();
    sheet
        .read_to_end(&mut bytes)
        .map_err(|error| PartnerReadError::NotAWorkbook {
            path: shown.clone(),
            source: zip::result::ZipError::Io(error),
        })?;
    drop(sheet);
    let xml = String::from_utf8(bytes).map_err(|source| PartnerReadError::NotText {
        path: shown.clone(),
        source,
    })?;
    parse_partner_sheet(&xml, &shown)
}

/// The scan itself, split out so every refusal is testable without a file on disk.
pub fn parse_partner_sheet(xml: &str, shown: &str) -> Result<PartnerRow, PartnerReadError> {
    // Narrow to `<sheetData>` first. Outside it live `<cols>` width elements whose attributes
    // read enough like cells to be mistaken for them by a scan.
    let body = match (xml.find("<sheetData"), xml.find("</sheetData>")) {
        (Some(start), Some(end)) if end > start => &xml[start..end],
        // A sheet with no data section is a sheet with no partners, which is a refusal rather
        // than an empty list: the caller asked for this bundle's partners and there is a file.
        _ => {
            return Err(PartnerReadError::Empty {
                path: shown.to_string(),
            })
        }
    };

    let mut cells: Vec<(u32, u32, String)> = Vec::new();
    let mut rest = body;
    while let Some(open) = rest.find("<c ") {
        let after = &rest[open..];
        let Some(tag_end) = after.find('>') else {
            break;
        };
        let tag = &after[..tag_end];
        // A self-closing `<c … />` is an empty cell: it holds no `<t>`, so it contributes
        // nothing and must not swallow the next cell's contents.
        let self_closing = tag.ends_with('/');
        let (inner, consumed) = if self_closing {
            ("", tag_end + 1)
        } else {
            match after.find("</c>") {
                Some(close) => (&after[tag_end + 1..close], close + "</c>".len()),
                None => break,
            }
        };

        if cell_type(tag).as_deref() == Some("s") {
            // A shared-string cell holds an **index**, not a name. Decoding it as text writes
            // `3` where a venue belongs — wrong data that looks like data. None of the current
            // workbooks has one; Excel writes one the moment somebody re-saves.
            return Err(PartnerReadError::SharedStrings {
                path: shown.to_string(),
            });
        }
        if let Some((column, row)) = cell_reference(tag) {
            let text = collect_text(inner);
            if !text.is_empty() {
                cells.push((row, column, text));
            }
        }
        rest = &after[consumed..];
    }

    let mut rows: Vec<u32> = cells.iter().map(|(row, _, _)| *row).collect();
    rows.sort_unstable();
    rows.dedup();
    if rows.len() > 1 {
        // Two rows means the file changed shape. Taking the first would drop the rest silently,
        // and the operator would find out from a spreadsheet that is missing venues.
        return Err(PartnerReadError::NotOneRow {
            path: shown.to_string(),
            rows: rows.len(),
        });
    }

    // `r=` is the authority on order, not document order: a generator may emit cells in any
    // sequence, and column order is what "from column K onward" means.
    cells.sort_by_key(|(row, column, _)| (*row, *column));
    let names: Vec<String> = cells
        .into_iter()
        .map(|(_, _, text)| text.trim().to_string())
        .filter(|text| !text.is_empty())
        .collect();
    if names.is_empty() {
        return Err(PartnerReadError::Empty {
            path: shown.to_string(),
        });
    }
    Ok(PartnerRow { names })
}

/// `t="…"` off a `<c …` opening tag.
fn cell_type(tag: &str) -> Option<String> {
    attribute(tag, "t")
}

/// `r="K1"` split into a one-based `(column, row)`.
fn cell_reference(tag: &str) -> Option<(u32, u32)> {
    let reference = attribute(tag, "r")?;
    let split = reference.find(|c: char| c.is_ascii_digit())?;
    let (letters, digits) = reference.split_at(split);
    if letters.is_empty() {
        return None;
    }
    let mut column = 0u32;
    for letter in letters.chars() {
        if !letter.is_ascii_alphabetic() {
            return None;
        }
        column = column
            .checked_mul(26)?
            .checked_add(u32::from(letter.to_ascii_uppercase() as u8 - b'A') + 1)?;
    }
    Some((column, digits.parse().ok()?))
}

fn attribute(tag: &str, key: &str) -> Option<String> {
    let needle = format!("{key}=\"");
    let start = tag.find(&needle)? + needle.len();
    let rest = &tag[start..];
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

/// Every `<t …>…</t>` inside one cell, concatenated.
///
/// Concatenated rather than "the first one": `<is><r><t>` rich-text runs are what Excel writes
/// when a single name carries mixed formatting, and taking the first run would return half a
/// venue's name.
fn collect_text(inner: &str) -> String {
    let mut out = String::new();
    let mut rest = inner;
    while let Some(open) = rest.find("<t") {
        let after = &rest[open..];
        // `<t>` or `<t xml:space="preserve">`, but not `<text>`; the character after `t` is
        // either the tag's end or whitespace before an attribute.
        let Some(tag_end) = after.find('>') else {
            break;
        };
        let boundary = after.as_bytes().get(2).copied();
        if !matches!(boundary, Some(b'>') | Some(b' ')) {
            rest = &after[2..];
            continue;
        }
        let Some(close) = after.find("</t>") else {
            break;
        };
        if close < tag_end {
            break;
        }
        out.push_str(&unescape(&after[tag_end + 1..close]));
        rest = &after[close + "</t>".len()..];
    }
    out
}

/// XML entities, including the numeric forms.
///
/// `&amp;` and `&apos;` both occur in the operator's own files (`Hẻm - Book &amp; Coffee`,
/// `D&apos;Lart Garden`), so leaving them encoded would write the escape into the spreadsheet.
fn unescape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        let tail = &rest[at..];
        let Some(end) = tail.find(';') else {
            out.push_str(tail);
            return out;
        };
        let entity = &tail[1..end];
        match entity {
            "amp" => out.push('&'),
            "lt" => out.push('<'),
            "gt" => out.push('>'),
            "quot" => out.push('"'),
            "apos" => out.push('\''),
            _ => {
                let decoded = entity
                    .strip_prefix('#')
                    .and_then(|number| match number.strip_prefix(['x', 'X']) {
                        Some(hex) => u32::from_str_radix(hex, 16).ok(),
                        None => number.parse().ok(),
                    })
                    .and_then(char::from_u32);
                match decoded {
                    Some(character) => out.push(character),
                    // An entity this does not know stays as written. Dropping it would quietly
                    // shorten a name; leaving it is visible in the spreadsheet and fixable.
                    None => out.push_str(&tail[..=end]),
                }
            }
        }
        rest = &tail[end + 1..];
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The exact shape the operator's twenty-one workbooks have: one row, inline strings, no
    /// shared-string table, and XML entities inside the names.
    fn real_shape() -> String {
        concat!(
            r#"<worksheet><cols><col min="1" max="1" width="30"/></cols><sheetData>"#,
            r#"<row r="1"><c r="A1" t="inlineStr"><is><t>Cổ Làng Mơ</t></is></c>"#,
            r#"<c r="B1" t="inlineStr"><is><t>D&apos;Lart Garden</t></is></c>"#,
            r#"<c r="C1" t="inlineStr"><is><t>Hẻm - Book &amp; Coffee</t></is></c>"#,
            r#"</row></sheetData></worksheet>"#
        )
        .to_string()
    }

    #[test]
    fn reads_one_row_of_names_and_decodes_the_entities() {
        let row = parse_partner_sheet(&real_shape(), "fixture.xlsx").expect("one row");
        assert_eq!(
            row.names,
            vec![
                "Cổ Làng Mơ".to_string(),
                "D'Lart Garden".to_string(),
                "Hẻm - Book & Coffee".to_string(),
            ]
        );
    }

    /// `r=` decides the order, not the order the generator happened to emit.
    #[test]
    fn column_order_comes_from_the_reference_not_the_document() {
        let xml = concat!(
            r#"<sheetData><row r="1">"#,
            r#"<c r="C1" t="inlineStr"><is><t>third</t></is></c>"#,
            r#"<c r="A1" t="inlineStr"><is><t>first</t></is></c>"#,
            r#"<c r="B1" t="inlineStr"><is><t>second</t></is></c>"#,
            r#"</row></sheetData>"#
        );
        let row = parse_partner_sheet(xml, "fixture.xlsx").expect("one row");
        assert_eq!(row.names, vec!["first", "second", "third"]);
    }

    /// **The one that would write a number where a venue belongs.**
    #[test]
    fn a_shared_string_table_is_refused_rather_than_decoded_as_an_index() {
        let xml = r#"<sheetData><row r="1"><c r="A1" t="s"><v>3</v></c></row></sheetData>"#;
        let error = parse_partner_sheet(xml, "fixture.xlsx").expect_err("must refuse");
        assert!(matches!(error, PartnerReadError::SharedStrings { .. }));
        // And says how to fix it, because the operator's tool is Excel, not this code.
        assert!(format!("{error}").contains("inline"), "{error}");
    }

    #[test]
    fn a_second_row_is_refused_rather_than_silently_dropped() {
        let xml = concat!(
            r#"<sheetData>"#,
            r#"<row r="1"><c r="A1" t="inlineStr"><is><t>one</t></is></c></row>"#,
            r#"<row r="2"><c r="A2" t="inlineStr"><is><t>two</t></is></c></row>"#,
            r#"</sheetData>"#
        );
        assert!(matches!(
            parse_partner_sheet(xml, "fixture.xlsx"),
            Err(PartnerReadError::NotOneRow { rows: 2, .. })
        ));
    }

    #[test]
    fn rich_text_runs_are_joined_rather_than_truncated() {
        // One name, bolded halfway, is two `<t>` runs in one cell.
        let xml = concat!(
            r#"<sheetData><row r="1"><c r="A1" t="inlineStr"><is>"#,
            r#"<r><t>Tiệm </t></r><r><t>Chaiko</t></r>"#,
            r#"</is></c></row></sheetData>"#
        );
        let row = parse_partner_sheet(xml, "fixture.xlsx").expect("one row");
        assert_eq!(row.names, vec!["Tiệm Chaiko"]);
    }

    #[test]
    fn empty_cells_and_an_empty_sheet_are_told_apart_from_names() {
        let padded = concat!(
            r#"<sheetData><row r="1">"#,
            r#"<c r="A1" t="inlineStr"><is><t>only</t></is></c>"#,
            r#"<c r="B1"/><c r="C1" t="inlineStr"><is><t>   </t></is></c>"#,
            r#"</row></sheetData>"#
        );
        let row = parse_partner_sheet(padded, "fixture.xlsx").expect("one row");
        assert_eq!(row.names, vec!["only"]);

        assert!(matches!(
            parse_partner_sheet(r#"<sheetData></sheetData>"#, "fixture.xlsx"),
            Err(PartnerReadError::Empty { .. })
        ));
    }

    /// Column letters past Z, because "from column K onward" is a column index and a workbook
    /// with twelve partners reaches L, not K.
    #[test]
    fn column_letters_carry_past_z() {
        assert_eq!(cell_reference(r#"<c r="A1""#), Some((1, 1)));
        assert_eq!(cell_reference(r#"<c r="K1""#), Some((11, 1)));
        assert_eq!(cell_reference(r#"<c r="Z9""#), Some((26, 9)));
        assert_eq!(cell_reference(r#"<c r="AA1""#), Some((27, 1)));
        assert_eq!(cell_reference(r#"<c r="AB2""#), Some((28, 2)));
    }
}
