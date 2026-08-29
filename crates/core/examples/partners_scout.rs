//! Read the partner names out of every bundle in a publish folder, and say what was refused.
//!
//! ```text
//! cargo run -p riviu-core --example partners_scout -- "<folder>"
//! ```
//!
//! Read-only. Opens each `partners-*.xlsx` and prints the row it holds.
//!
//! This exists because those names end up in a spreadsheet **about a post that is already
//! live**, and the difference between a name read wrongly and a name read correctly is not
//! visible anywhere else until it is a wrong row somebody has to find by hand. The unit tests
//! pin the shapes; this is the instrument that says whether a real batch matches them.

use std::path::{Path, PathBuf};

use riviu_core::publish_partners::read_partner_row;

fn main() {
    let Some(root) = std::env::args().nth(1).map(PathBuf::from) else {
        eprintln!("usage: partners_scout <folder>");
        std::process::exit(2);
    };
    let mut bundles = 0usize;
    let mut read = 0usize;
    let mut refused = 0usize;
    let mut missing = 0usize;

    let mut entries: Vec<PathBuf> = match std::fs::read_dir(&root) {
        Ok(dir) => dir
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| path.is_dir())
            .collect(),
        Err(error) => {
            eprintln!("cannot read {}: {error}", root.display());
            std::process::exit(1);
        }
    };
    entries.sort();

    for bundle in entries {
        bundles += 1;
        let name = bundle
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_default();
        match workbook_in(&bundle) {
            None => {
                missing += 1;
                println!("{name}\n  (no partners-*.xlsx)");
            }
            Some(path) => match read_partner_row(&path) {
                Ok(row) => {
                    read += 1;
                    println!(
                        "{name}\n  {} tên: {}",
                        row.names.len(),
                        row.names.join(" | ")
                    );
                }
                Err(error) => {
                    refused += 1;
                    println!("{name}\n  REFUSED: {error}");
                }
            },
        }
    }

    println!(
        "\n{bundles} thư mục · {read} đọc được · {refused} bị từ chối · {missing} không có file"
    );
    if refused > 0 {
        std::process::exit(1);
    }
}

/// The one `partners-*.xlsx` in a bundle, or `None`.
///
/// More than one is not resolved here: `scan_publish_folder` is the place that counts partner
/// files, and a second one is its refusal to make, not this tool's.
fn workbook_in(bundle: &Path) -> Option<PathBuf> {
    std::fs::read_dir(bundle)
        .ok()?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|path| {
            let name = path
                .file_name()
                .map(|name| name.to_string_lossy().to_ascii_lowercase())
                .unwrap_or_default();
            name.starts_with("partner") && name.ends_with(".xlsx")
        })
}
