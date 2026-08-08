#[cfg(target_os = "macos")]
use std::path::PathBuf;

use async_trait::async_trait;
use riviu_core::{CommentOcrObservation, FrameTextSource};

#[derive(Debug, Default)]
pub struct DesktopFrameTextSource;

#[async_trait]
impl FrameTextSource for DesktopFrameTextSource {
    async fn recognize(&self, frame: &[u8]) -> anyhow::Result<Vec<CommentOcrObservation>> {
        recognize(frame).await
    }
}

/// Only the macOS `recognize` shells out to the Swift Vision helper; elsewhere
/// there is nothing to locate.
#[cfg(target_os = "macos")]
fn helper_candidates() -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        for ancestor in exe.ancestors().take(5) {
            candidates.push(ancestor.join("Resources/sidecars/wda/interaction_vision_ocr.swift"));
        }
    }
    // The source-tree paths are useful for `cargo run`, but must come after
    // bundled resources so an installed app does not trigger Desktop-folder
    // permission prompts by reading the checkout helper.
    candidates.extend([
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../../tools/interaction-gate0/vision_ocr.swift"),
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../../sidecars/wda/interaction_vision_ocr.swift"),
        PathBuf::from("tools/interaction-gate0/vision_ocr.swift"),
    ]);
    candidates
}

#[cfg(target_os = "macos")]
fn find_helper() -> Option<PathBuf> {
    helper_candidates().into_iter().find(|path| path.is_file())
}

#[cfg(target_os = "macos")]
pub async fn recognize(frame: &[u8]) -> anyhow::Result<Vec<CommentOcrObservation>> {
    let frame = frame.to_vec();
    tokio::task::spawn_blocking(move || recognize_sync(&frame))
        .await
        .map_err(|error| anyhow::anyhow!("OCR worker join: {error}"))?
}

#[cfg(not(target_os = "macos"))]
pub async fn recognize(_frame: &[u8]) -> anyhow::Result<Vec<CommentOcrObservation>> {
    anyhow::bail!("Vision OCR requires macOS")
}

#[cfg(target_os = "macos")]
fn recognize_sync(frame: &[u8]) -> anyhow::Result<Vec<CommentOcrObservation>> {
    let helper = find_helper().ok_or_else(|| anyhow::anyhow!("Vision OCR helper missing"))?;
    let id = format!(
        "{}-{}",
        std::process::id(),
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
    );
    let image_path = std::env::temp_dir().join(format!("riviu-interaction-{id}.jpg"));
    std::fs::write(&image_path, frame)?;
    let output = std::process::Command::new("xcrun")
        .args([
            "swift",
            helper.to_string_lossy().as_ref(),
            image_path.to_string_lossy().as_ref(),
        ])
        .output();
    let _ = std::fs::remove_file(&image_path);
    let output = output?;
    if !output.status.success() {
        anyhow::bail!(
            "Vision OCR failed ({})",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let parsed = serde_json::from_slice::<Vec<CommentOcrObservation>>(&output.stdout)?;
    Ok(parsed)
}
