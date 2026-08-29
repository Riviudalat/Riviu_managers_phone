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
    /// The archive is readable and the worksheet part inside it is not.
    ///
    /// Split from [`Self::NoSheet`] because the two send the operator to different places: a
    /// missing sheet is a workbook-layout question, and this is corruption. Every error from
    /// `by_name` used to become `NoSheet`, including a truncated local header on a member the
    /// central directory lists.
    #[error("{path} có phần {part} nhưng đọc không được: {source}")]
    BrokenPart {
        path: String,
        part: String,
        #[source]
        source: zip::result::ZipError,
    },
    /// The worksheet expands past what a one-row partner list could possibly need.
    ///
    /// A `.xlsx` is a zip, and a small one can hold a worksheet that inflates to gigabytes.
    /// Without a ceiling the reader calls `read_to_end` into an unbounded `Vec` and the
    /// process dies of memory exhaustion before any shape check runs — which on a desktop app
    /// is not an error message, it is the app disappearing.
    #[error("{path} có sheet giải nén ra hơn {limit} byte — một hàng tên quán không thể lớn thế")]
    SheetTooLarge { path: String, limit: usize },
    /// The workbook does not name exactly one partner sheet.
    #[error("{path} không chỉ ra được sheet đối tác: {detail}")]
    NoNamedSheet { path: String, detail: String },
    /// The scan hit XML it could not finish reading.
    ///
    /// A refusal rather than "the names found so far", which is what this used to do: a cell
    /// truncated before its `</c>` ended the loop and the workbook was reported as a success
    /// holding every name **before** the break. The operator's record of a live post then
    /// silently omits partners.
    #[error("{path} có XML hỏng ở {detail} — không đọc một phần rồi báo là xong")]
    BrokenXml { path: String, detail: String },
}

/// The sheet name these workbooks carry.
///
/// Preferred, not required: a workbook holding exactly one sheet is unambiguous whatever it is
/// called, and pinning a generated name would break the day the generator changes it. What is
/// refused is *ambiguity* — several sheets and none of them this one.
const PARTNER_SHEET_NAME: &str = "Doi tac";

/// The largest worksheet this reader will inflate.
///
/// One row of venue names is a few kilobytes; a megabyte is four hundred times that and still
/// far under what a decompression bomb needs to hurt. See
/// [`PartnerReadError::SheetTooLarge`].
const MAX_SHEET_BYTES: usize = 1_048_576;

/// Find the worksheet part that actually holds the partner row.
///
/// **The part number is not the sheet name**, and hard-coding `xl/worksheets/sheet1.xml` was
/// wrong in both directions: a workbook whose only sheet is related to `sheet2.xml` — which
/// happens after a sheet is deleted and recreated — read as "no sheet", and a workbook with a
/// cover sheet in `sheet1.xml` had its cover row read as partner names. The second is the
/// dangerous one: plausible data about a live post.
///
/// OOXML resolves this in two hops. `xl/workbook.xml` maps a visible name to a relationship
/// id; `xl/_rels/workbook.xml.rels` maps that id to a part. Both are scanned the same way the
/// worksheet is, and for the same reason.
fn worksheet_part(workbook: &str, rels: &str, shown: &str) -> Result<String, PartnerReadError> {
    let sheets: Vec<(String, String)> = workbook
        .split("<sheet ")
        .skip(1)
        .filter_map(|chunk| {
            let tag = chunk.split('>').next()?;
            Some((attribute(tag, "name")?, attribute(tag, "r:id")?))
        })
        .collect();
    let chosen = match sheets.len() {
        0 => {
            return Err(PartnerReadError::NoNamedSheet {
                path: shown.to_string(),
                detail: "workbook.xml không liệt kê sheet nào".into(),
            })
        }
        1 => &sheets[0],
        _ => sheets
            .iter()
            .find(|(name, _)| name == PARTNER_SHEET_NAME)
            .ok_or_else(|| PartnerReadError::NoNamedSheet {
                path: shown.to_string(),
                detail: format!(
                    "có {} sheet và không sheet nào tên `{PARTNER_SHEET_NAME}`: {}",
                    sheets.len(),
                    sheets
                        .iter()
                        .map(|(name, _)| name.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            })?,
    };
    let target = rels
        .split("<Relationship ")
        .skip(1)
        .filter_map(|chunk| {
            let tag = chunk.split('>').next()?;
            (attribute(tag, "Id")? == chosen.1).then(|| attribute(tag, "Target"))?
        })
        .next()
        .ok_or_else(|| PartnerReadError::NoNamedSheet {
            path: shown.to_string(),
            detail: format!("không có Relationship nào mang Id {}", chosen.1),
        })?;
    // Targets are relative to `xl/`, and may or may not say so.
    let target = target.trim_start_matches('/');
    Ok(if target.starts_with("xl/") {
        target.to_string()
    } else {
        format!("xl/{target}")
    })
}

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
    let workbook = read_part(&mut archive, "xl/workbook.xml", &shown)?;
    let rels = read_part(&mut archive, "xl/_rels/workbook.xml.rels", &shown)?;
    let part = worksheet_part(&workbook, &rels, &shown)?;
    let xml = read_part(&mut archive, &part, &shown)?;
    parse_partner_sheet(&xml, &shown)
}

/// Read one archive member as text, refusing anything that inflates past the ceiling.
///
/// `take(limit + 1)` rather than checking `ZipFile::size()`: the declared size is a number in
/// the archive, written by whoever built it, and a bomb declares whatever it likes. Reading one
/// byte past the ceiling and refusing on it is the only bound that does not trust the file.
fn read_part(
    archive: &mut zip::ZipArchive<std::fs::File>,
    part: &str,
    shown: &str,
) -> Result<String, PartnerReadError> {
    let mut member = match archive.by_name(part) {
        Ok(member) => member,
        Err(zip::result::ZipError::FileNotFound) => {
            return Err(PartnerReadError::NoSheet {
                path: shown.to_string(),
            })
        }
        // Anything else is a readable archive with an unreadable member: corruption, not a
        // layout question, and the operator is sent somewhere different for each.
        Err(source) => {
            return Err(PartnerReadError::BrokenPart {
                path: shown.to_string(),
                part: part.to_string(),
                source,
            })
        }
    };
    let mut bytes = Vec::new();
    member
        .by_ref()
        .take(MAX_SHEET_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| PartnerReadError::NotAWorkbook {
            path: shown.to_string(),
            source: zip::result::ZipError::Io(error),
        })?;
    if bytes.len() > MAX_SHEET_BYTES {
        return Err(PartnerReadError::SheetTooLarge {
            path: shown.to_string(),
            limit: MAX_SHEET_BYTES,
        });
    }
    String::from_utf8(bytes).map_err(|source| PartnerReadError::NotText {
        path: shown.to_string(),
        source,
    })
}

/// The scan itself, split out so every refusal is testable without a file on disk.
pub fn parse_partner_sheet(xml: &str, shown: &str) -> Result<PartnerRow, PartnerReadError> {
    // Narrow to `<sheetData>` first. Outside it live `<cols>` width elements whose attributes
    // read enough like cells to be mistaken for them by a scan.
    let body = match (xml.find("<sheetData"), xml.find("</sheetData>")) {
        (Some(start), Some(end)) if end > start => &xml[start..end],
        // **An opened `<sheetData>` that never closes is truncation, not emptiness**, and the
        // two send the operator to different places: one is a workbook with no partners in it,
        // the other is a file that stopped mid-write. `<sheetData/>` is the legitimate empty
        // form and is not truncation.
        (Some(start), None) if !is_self_closing_sheet_data(&xml[start..]) => {
            return Err(PartnerReadError::BrokenXml {
                path: shown.to_string(),
                detail: "<sheetData> mở mà không đóng".into(),
            })
        }
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
            // Truncated mid-tag. Returning the cells found so far would report a workbook as
            // read while silently dropping every partner after this point.
            return Err(PartnerReadError::BrokenXml {
                path: shown.to_string(),
                detail: "một thẻ <c ...> không đóng".into(),
            });
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
                None => {
                    return Err(PartnerReadError::BrokenXml {
                        path: shown.to_string(),
                        detail: "một ô thiếu </c>".into(),
                    })
                }
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
            let text = collect_text(inner, shown)?;
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

/// Whether `<sheetData` opens the self-closing, legitimately empty form.
///
/// `<sheetData/>` and `<sheetData />` are how a generator writes a sheet with no rows. Without
/// this they would be read as a file truncated mid-element.
fn is_self_closing_sheet_data(from_open: &str) -> bool {
    from_open
        .find('>')
        .is_some_and(|end| from_open[..end].trim_end().ends_with('/'))
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

/// One attribute off an opening tag, in either XML quoting.
///
/// Single quotes and whitespace around `=` are both valid XML and neither was recognised, so a
/// `t='s'` shared-string cell read as an ordinary one — writing an index where a venue belongs.
/// The key is matched only at a boundary, so `r=` does not match inside `xr:uid=`.
fn attribute(tag: &str, key: &str) -> Option<String> {
    let bytes = tag.as_bytes();
    let mut from = 0usize;
    while let Some(found) = tag[from..].find(key) {
        let at = from + found;
        from = at + key.len();
        // A key starts the tag or follows whitespace; otherwise this is a suffix of a longer
        // attribute name.
        if at > 0 && !bytes[at - 1].is_ascii_whitespace() {
            continue;
        }
        let mut cursor = at + key.len();
        while bytes.get(cursor).is_some_and(u8::is_ascii_whitespace) {
            cursor += 1;
        }
        if bytes.get(cursor) != Some(&b'=') {
            continue;
        }
        cursor += 1;
        while bytes.get(cursor).is_some_and(u8::is_ascii_whitespace) {
            cursor += 1;
        }
        let quote = match bytes.get(cursor) {
            Some(&b'"') => '"',
            Some(&b'\'') => '\'',
            _ => continue,
        };
        let rest = &tag[cursor + 1..];
        let end = rest.find(quote)?;
        return Some(rest[..end].to_string());
    }
    None
}

/// Every `<t …>…</t>` inside one cell, concatenated.
///
/// Concatenated rather than "the first one": `<is><r><t>` rich-text runs are what Excel writes
/// when a single name carries mixed formatting, and taking the first run would return half a
/// venue's name.
fn collect_text(inner: &str, shown: &str) -> Result<String, PartnerReadError> {
    let mut out = String::new();
    let mut rest = inner;
    while let Some(open) = rest.find("<t") {
        let after = &rest[open..];
        // `<t>` or `<t xml:space="preserve">`, but not `<text>`. **Any** whitespace counts as
        // the boundary, not only a space: a tab or a newline before the attributes made the
        // whole run vanish, and a vanished run is half a venue's name.
        let boundary = after.as_bytes().get(2).copied();
        if !matches!(boundary, Some(b'>'))
            && !boundary.is_some_and(|byte| byte.is_ascii_whitespace())
        {
            rest = &after[2..];
            continue;
        }
        let Some(tag_end) = after.find('>') else {
            return Err(PartnerReadError::BrokenXml {
                path: shown.to_string(),
                detail: "một thẻ <t ...> không đóng".into(),
            });
        };
        let Some(close) = after.find("</t>") else {
            return Err(PartnerReadError::BrokenXml {
                path: shown.to_string(),
                detail: "một <t> thiếu </t>".into(),
            });
        };
        if close < tag_end {
            return Err(PartnerReadError::BrokenXml {
                path: shown.to_string(),
                detail: "</t> đứng trước khi thẻ <t> mở xong".into(),
            });
        }
        out.push_str(&unescape(&after[tag_end + 1..close], shown)?);
        rest = &after[close + "</t>".len()..];
    }
    Ok(out)
}

/// XML entities, including the numeric forms.
///
/// `&amp;` and `&apos;` both occur in the operator's own files (`Hẻm - Book &amp; Coffee`,
/// `D&apos;Lart Garden`), so leaving them encoded would write the escape into the spreadsheet.
fn unescape(text: &str, shown: &str) -> Result<String, PartnerReadError> {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        let tail = &rest[at..];
        let Some(end) = tail.find(';') else {
            // A bare `&` is not valid XML. Passing it through wrote a half-decoded name into a
            // spreadsheet about a live post.
            return Err(PartnerReadError::BrokenXml {
                path: shown.to_string(),
                detail: "một `&` không phải là entity".into(),
            });
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
                    // A control character is not a venue name, and `&#0;` in particular puts a
                    // NUL into a string that goes on to a spreadsheet.
                    Some(character) if !character.is_control() => out.push(character),
                    // An entity this does not know used to be passed through as written, on the
                    // reasoning that a visible escape is fixable. It is not: nobody reads the
                    // spreadsheet against the workbook, so what actually happens is a wrong
                    // name in the operator's record of a post that is already live.
                    _ => {
                        return Err(PartnerReadError::BrokenXml {
                            path: shown.to_string(),
                            detail: format!("entity không đọc được: &{entity};"),
                        })
                    }
                }
            }
        }
        rest = &tail[end + 1..];
    }
    out.push_str(rest);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Write a real `.xlsx` to a temporary path, so the archive half is exercised too.
    ///
    /// The scan is testable on a `&str` and the *archive* is not: how the worksheet part is
    /// found, and how much of it is allowed to inflate, both live above
    /// `parse_partner_sheet`. Two reversals — hard-coding `sheet1.xml` again, and removing the
    /// decompression ceiling — stayed green against a suite that only ever fed it strings.
    fn write_workbook(parts: &[(&str, &[u8])]) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!(
            "riviu-partner-fixture-{}.xlsx",
            uuid::Uuid::new_v4()
        ));
        let file = std::fs::File::create(&path).expect("fixture file");
        let mut writer = zip::ZipWriter::new(file);
        let options: zip::write::FileOptions<'_, ()> =
            zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        for (name, bytes) in parts {
            use std::io::Write;
            writer.start_file(*name, options).expect("start part");
            writer.write_all(bytes).expect("write part");
        }
        writer.finish().expect("finish archive");
        path
    }

    fn workbook_xml(sheet_name: &str, rel: &str) -> Vec<u8> {
        format!(
            r#"<workbook><sheets><sheet name="{sheet_name}" sheetId="1" r:id="{rel}"/></sheets></workbook>"#
        )
        .into_bytes()
    }

    fn rels_xml(rel: &str, target: &str) -> Vec<u8> {
        format!(r#"<Relationships><Relationship Id="{rel}" Target="{target}"/></Relationships>"#)
            .into_bytes()
    }

    fn one_row_sheet(name: &str) -> Vec<u8> {
        format!(
            r#"<worksheet><sheetData><row r="1"><c r="A1" t="inlineStr"><is><t>{name}</t></is></c></row></sheetData></worksheet>"#
        )
        .into_bytes()
    }

    /// **The worksheet is found through the relationship, in a real archive.**
    ///
    /// Not `sheet1.xml`. A workbook whose only sheet lives in `sheet7.xml` — which is what a
    /// deleted-and-recreated sheet leaves behind — read as "no sheet" before this.
    #[test]
    fn a_real_workbook_whose_sheet_is_not_sheet1_still_reads() {
        let path = write_workbook(&[
            ("xl/workbook.xml", &workbook_xml("Doi tac", "rId1")),
            (
                "xl/_rels/workbook.xml.rels",
                &rels_xml("rId1", "worksheets/sheet7.xml"),
            ),
            ("xl/worksheets/sheet7.xml", &one_row_sheet("Quán Bảy")),
            // A decoy in the conventional place, holding something else entirely.
            (
                "xl/worksheets/sheet1.xml",
                &one_row_sheet("KHÔNG PHẢI ĐỐI TÁC"),
            ),
        ]);
        let row = read_partner_row(&path).expect("reads through the relationship");
        assert_eq!(
            row.names,
            vec!["Quán Bảy".to_string()],
            "read the decoy in sheet1 instead of the related sheet"
        );
        let _ = std::fs::remove_file(path);
    }

    /// **A worksheet that inflates past the ceiling is refused, not read.**
    ///
    /// A `.xlsx` is a zip: a few kilobytes on disk can hold a part that expands to gigabytes.
    /// Without a ceiling this called `read_to_end` into an unbounded `Vec`, and on a desktop
    /// app that is not an error message — it is the app disappearing.
    #[test]
    fn a_worksheet_that_inflates_past_the_ceiling_is_refused() {
        // Two megabytes of one repeated byte: tiny deflated, over the limit inflated.
        let mut oversized = Vec::from(&b"<worksheet><sheetData>"[..]);
        oversized.resize(MAX_SHEET_BYTES * 2, b' ');
        oversized.extend_from_slice(b"</sheetData></worksheet>");
        let path = write_workbook(&[
            ("xl/workbook.xml", &workbook_xml("Doi tac", "rId1")),
            (
                "xl/_rels/workbook.xml.rels",
                &rels_xml("rId1", "worksheets/sheet1.xml"),
            ),
            ("xl/worksheets/sheet1.xml", &oversized),
        ]);
        let error = read_partner_row(&path).expect_err("must refuse to inflate it");
        assert!(
            matches!(error, PartnerReadError::SheetTooLarge { .. }),
            "{error:?}"
        );
        // And the file on disk is far smaller than what it would have expanded to, which is
        // the whole shape of the hazard.
        let on_disk = std::fs::metadata(&path).expect("stat").len() as usize;
        assert!(
            on_disk < MAX_SHEET_BYTES,
            "the fixture is not compressed, so it does not model the hazard: {on_disk} bytes"
        );
        let _ = std::fs::remove_file(path);
    }

    /// A workbook with no worksheet part at all is a missing sheet, not corruption.
    #[test]
    fn a_workbook_missing_its_worksheet_says_so() {
        let path = write_workbook(&[
            ("xl/workbook.xml", &workbook_xml("Doi tac", "rId1")),
            (
                "xl/_rels/workbook.xml.rels",
                &rels_xml("rId1", "worksheets/sheet1.xml"),
            ),
        ]);
        assert!(matches!(
            read_partner_row(&path),
            Err(PartnerReadError::NoSheet { .. })
        ));
        let _ = std::fs::remove_file(path);
    }

    /// **Truncated XML is a refusal, not a shorter partner list.**
    ///
    /// The failure this closes is the quiet one: a cell cut off before its `</c>` ended the
    /// scan, and the workbook was reported as read successfully holding every name *before*
    /// the break. The operator's record of a live post then silently omits partners, and
    /// nothing anywhere says so.
    #[test]
    fn a_truncated_cell_refuses_rather_than_returning_the_names_before_it() {
        let truncated = concat!(
            r#"<sheetData><row r="1">"#,
            r#"<c r="A1" t="inlineStr"><is><t>Quán A</t></is></c>"#,
            r#"<c r="B1" t="inlineStr"><is><t>Quán B</t></is>"#,
            r#"</sheetData>"#,
        );
        let error = parse_partner_sheet(truncated, "x.xlsx").expect_err("must refuse");
        assert!(
            matches!(error, PartnerReadError::BrokenXml { .. }),
            "{error:?}"
        );

        // A tag cut off mid-attribute, with the section left open too.
        let mid_tag = r#"<sheetData><row r="1"><c r="A1" t="inlineStr"#;
        assert!(matches!(
            parse_partner_sheet(mid_tag, "x.xlsx"),
            Err(PartnerReadError::BrokenXml { .. })
        ));
        // And the same cut with the section closed, so the cell scan is what catches it.
        let closed = r#"<sheetData><row r="1"><c r="A1" t="inlineStr</sheetData>"#;
        assert!(matches!(
            parse_partner_sheet(closed, "x.xlsx"),
            Err(PartnerReadError::BrokenXml { .. })
        ));
        // A legitimately empty sheet is empty, not broken.
        for empty in [r#"<sheetData/>"#, r#"<sheetData />"#] {
            assert!(
                matches!(
                    parse_partner_sheet(empty, "x.xlsx"),
                    Err(PartnerReadError::Empty { .. })
                ),
                "{empty} was read as truncation"
            );
        }
    }

    /// **Single quotes are valid XML, and `t='s'` is still a shared string.**
    ///
    /// The attribute reader only understood double quotes, so this cell read as an ordinary
    /// one and its *index* went into the spreadsheet as a venue name.
    #[test]
    fn a_single_quoted_shared_string_is_refused_like_a_double_quoted_one() {
        for tag in [
            r#"<c r="A1" t='s'><v>3</v></c>"#,
            r#"<c r="A1" t = "s"><v>3</v></c>"#,
        ] {
            let xml = format!(r#"<sheetData><row r="1">{tag}</row></sheetData>"#);
            assert!(
                matches!(
                    parse_partner_sheet(&xml, "x.xlsx"),
                    Err(PartnerReadError::SharedStrings { .. })
                ),
                "{tag} was not recognised as a shared string"
            );
        }
    }

    /// An attribute name is matched at a boundary, not as a suffix of a longer one.
    #[test]
    fn a_longer_attribute_name_ending_in_the_key_is_not_the_key() {
        // `xr:uid` ends in no key here, but `sr` does end in `r`; without a boundary check
        // the reference would be read out of the wrong attribute.
        let tag = r#"<c sr="ZZ9" r="B1" t="inlineStr""#;
        assert_eq!(attribute(tag, "r").as_deref(), Some("B1"));
        assert_eq!(attribute(tag, "t").as_deref(), Some("inlineStr"));
        assert_eq!(attribute(tag, "nothing"), None);
    }

    /// **A run whose tag has a newline before its attributes is still a run.**
    ///
    /// Only a literal space counted as the boundary after `<t`, so a workbook formatted with
    /// tabs or newlines lost the text entirely — half a venue's name, or all of it.
    #[test]
    fn whitespace_inside_the_text_tag_does_not_hide_the_text() {
        for opener in [
            "<t>",
            "<t xml:space=\"preserve\">",
            "<t\n xml:space=\"preserve\">",
            "<t\txml:space=\"preserve\">",
        ] {
            let xml = format!(
                r#"<sheetData><row r="1"><c r="A1" t="inlineStr"><is>{opener}Quán A</t></is></c></row></sheetData>"#
            );
            let row = parse_partner_sheet(&xml, "x.xlsx")
                .unwrap_or_else(|error| panic!("{opener:?} lost its text: {error:?}"));
            assert_eq!(row.names, vec!["Quán A".to_string()]);
        }
        // And `<text>` is still not `<t>`.
        let decoy = r#"<sheetData><row r="1"><c r="A1" t="inlineStr"><is><text>x</text><t>Quán A</t></is></c></row></sheetData>"#;
        assert_eq!(
            parse_partner_sheet(decoy, "x.xlsx").expect("reads").names,
            vec!["Quán A".to_string()]
        );
    }

    /// **An entity the reader cannot decode is a refusal, not a name with an escape in it.**
    ///
    /// Passing it through was justified as "visible in the spreadsheet and fixable". Nobody
    /// reads the spreadsheet against the workbook, so what actually happened was a wrong name
    /// in the operator's record of a post that is already live. `&#0;` was worse still: a NUL
    /// inside a string on its way to a sheet.
    #[test]
    fn an_entity_that_cannot_be_decoded_refuses_rather_than_travelling() {
        for body in ["Qu&nbsp;án", "Qu&#0;án", "Qu&án", "Qu&#xZZ;án"] {
            let xml = format!(
                r#"<sheetData><row r="1"><c r="A1" t="inlineStr"><is><t>{body}</t></is></c></row></sheetData>"#
            );
            let error = parse_partner_sheet(&xml, "x.xlsx")
                .err()
                .unwrap_or_else(|| panic!("{body} was accepted"));
            assert!(
                matches!(error, PartnerReadError::BrokenXml { .. }),
                "{body}: {error:?}"
            );
        }
        // The entities the operator's own files actually carry still decode.
        let real = r#"<sheetData><row r="1"><c r="A1" t="inlineStr"><is><t>H&#7867;m - Book &amp; Coffee</t></is></c></row></sheetData>"#;
        assert_eq!(
            parse_partner_sheet(real, "x.xlsx").expect("reads").names,
            vec!["Hẻm - Book & Coffee".to_string()]
        );
    }

    /// **The part number is not the sheet name.**
    ///
    /// Hard-coding `sheet1.xml` failed both ways: a workbook whose only sheet lives in
    /// `sheet2.xml` read as "no sheet", and a workbook with a cover sheet first had the
    /// cover's row read as partner names — plausible data about a live post.
    #[test]
    fn the_worksheet_is_resolved_through_the_relationship_not_by_its_number() {
        let rels = concat!(
            r#"<Relationships>"#,
            r#"<Relationship Id="rId1" Target="worksheets/sheet2.xml"/>"#,
            r#"<Relationship Id="rId2" Target="styles.xml"/>"#,
            r#"</Relationships>"#,
        );
        // One sheet, whatever it is called, living in sheet2.
        let one = r#"<workbook><sheets><sheet name="Doi tac" sheetId="1" r:id="rId1"/></sheets></workbook>"#;
        assert_eq!(
            worksheet_part(one, rels, "x.xlsx").expect("resolves"),
            "xl/worksheets/sheet2.xml"
        );

        // Two sheets: the named one wins, and the cover is not read.
        let two = concat!(
            r#"<workbook><sheets>"#,
            r#"<sheet name="Bìa" sheetId="1" r:id="rId9"/>"#,
            r#"<sheet name="Doi tac" sheetId="2" r:id="rId1"/>"#,
            r#"</sheets></workbook>"#,
        );
        assert_eq!(
            worksheet_part(two, rels, "x.xlsx").expect("resolves"),
            "xl/worksheets/sheet2.xml"
        );

        // Two sheets and neither is the partner sheet: refuse, and name what was found.
        let ambiguous = concat!(
            r#"<workbook><sheets>"#,
            r#"<sheet name="Bìa" sheetId="1" r:id="rId9"/>"#,
            r#"<sheet name="Ghi chú" sheetId="2" r:id="rId1"/>"#,
            r#"</sheets></workbook>"#,
        );
        let error = worksheet_part(ambiguous, rels, "x.xlsx").expect_err("must refuse");
        let message = error.to_string();
        assert!(
            message.contains("Bìa") && message.contains("Ghi chú"),
            "{message}"
        );

        // A relationship id nothing maps is a refusal, not a fallback to sheet1.
        assert!(worksheet_part(one, r#"<Relationships/>"#, "x.xlsx").is_err());
    }

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
