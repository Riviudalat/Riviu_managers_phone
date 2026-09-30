//! Inspect the assets actually embedded in the app, before installer delivery.
use anyhow::Context as _;
use sha2::{Digest, Sha256};
use std::path::Path;

pub(crate) fn inspect(context: &tauri::Context<tauri::Wry>) -> anyhow::Result<serde_json::Value> {
    anyhow::ensure!(
        matches!(context.config().build.frontend_dist,
            Some(tauri::utils::config::FrontendDist::Directory(_))),
        "Packaged frontend must be a directory, not a URL. Use a relative frontendDist; Windows C:/ paths are parsed as URLs."
    );
    let index = context.assets().get(&"index.html".into())
        .context("Packaged frontend is missing index.html")?;
    let html = std::str::from_utf8(&index).context("Packaged index.html is not UTF-8")?;
    anyhow::ensure!(html.contains("id=\"root\""), "Packaged frontend is missing the React root");
    let mut files = Vec::new();
    for (path, _) in context.assets().iter() {
        let bytes = context.assets().get(&path.as_ref().into())
            .with_context(|| format!("Packaged asset cannot be decoded: {path}"))?;
        files.push(serde_json::json!({"path":path,"bytes":bytes.len(),
            "sha256":format!("{:x}",Sha256::digest(&bytes))}));
    }
    let mut references = 0;
    for marker in ["src=\"./assets/", "href=\"./assets/"] {
        for tail in html.split(marker).skip(1) {
            let name = tail.split('"').next().context("Malformed frontend asset reference")?;
            let path = format!("assets/{name}");
            anyhow::ensure!(context.assets().get(&path.into()).is_some(),
                "Packaged frontend references a missing asset: {name}");
            references += 1;
        }
    }
    anyhow::ensure!(references > 0, "Packaged frontend contains no application asset references");
    files.sort_by(|a,b|a["path"].as_str().cmp(&b["path"].as_str()));
    Ok(serde_json::json!({"schemaVersion":1,"status":"ready",
        "appVersion":context.config().version,"frontendKind":"embeddedDirectory",
        "indexSha256":format!("{:x}",Sha256::digest(&index)),
        "assetReferences":references,"files":files}))
}

/// Console-free installer diagnostics; no WebView, device, DB or credential access.
pub fn write_report(path: &Path) -> anyhow::Result<()> {
    anyhow::ensure!(path.is_absolute(), "Frontend report path must be absolute");
    let context = tauri::generate_context!();
    let report = inspect(&context)?;
    std::fs::write(path, serde_json::to_vec_pretty(&report)?)?;
    Ok(())
}
