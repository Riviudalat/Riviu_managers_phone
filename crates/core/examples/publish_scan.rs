//! Offline manifest scan. Reads source files and writes only a JSON summary to stdout.
//! cargo run --locked -p riviu-core --example publish_scan -- "<folder>"

use std::path::PathBuf;

use riviu_core::{scan_publish_folder, PublishScanOptions};

fn main() -> std::process::ExitCode {
    let mut arguments = std::env::args_os().skip(1);
    let Some(root) = arguments.next().map(PathBuf::from) else {
        eprintln!("usage: publish_scan <folder>");
        return std::process::ExitCode::from(2);
    };
    if arguments.next().is_some() {
        eprintln!("usage: publish_scan <folder>");
        return std::process::ExitCode::from(2);
    }
    let (summary, code) = match scan_publish_folder(&root, PublishScanOptions::default()) {
        Ok(manifest) => {
            let bundles = manifest
                .bundles
                .iter()
                .map(|bundle| {
                    serde_json::json!({
                        "id": bundle.id,
                        "name": bundle.name,
                        "sourcePath": bundle.source_path,
                        "mediaKind": bundle.media_kind,
                        "imageCount": bundle.images.len(),
                        "videoPresent": bundle.video.is_some(),
                        "captionPresent": !bundle.caption.trim().is_empty(),
                        "captionSha256": bundle.caption_sha256,
                        "totalBytes": bundle.total_bytes,
                        "partnerCount": bundle.partners.len(),
                    })
                })
                .collect::<Vec<_>>();
            (
                serde_json::json!({
                    "sourceRoot": manifest.source_root,
                    "bundleCount": bundles.len(),
                    "bundles": bundles,
                    "noticeCount": manifest.notices.len(),
                    "ignoredPartnerFiles": manifest.ignored_partner_files,
                    "ignoredHiddenFiles": manifest.ignored_hidden_files,
                    "error": null,
                }),
                0,
            )
        }
        Err(error) => (
            serde_json::json!({
                "sourceRoot": root,
                "bundleCount": 0,
                "error": error.to_string(),
            }),
            1,
        ),
    };
    match serde_json::to_string_pretty(&summary) {
        Ok(json) => println!("{json}"),
        Err(error) => {
            eprintln!("serialize scan summary: {error}");
            return std::process::ExitCode::from(3);
        }
    }
    std::process::ExitCode::from(code)
}
