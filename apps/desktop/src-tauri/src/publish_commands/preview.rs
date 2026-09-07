use super::*;
use base64::Engine;
use sha2::{Digest, Sha256};
use std::io::Read;

static PREVIEW_SLOTS: std::sync::LazyLock<Arc<tokio::sync::Semaphore>> =
    std::sync::LazyLock::new(|| Arc::new(tokio::sync::Semaphore::new(2)));

/// Preview an image inside the selected bundle, matching its scanned bytes.
#[tauri::command]
pub async fn publish_image_preview(
    bundle_root: String,
    image_path: String,
    sha256: String,
) -> Result<String, CommandError> {
    let permit = PREVIEW_SLOTS
        .clone()
        .acquire_owned()
        .await
        .map_err(|error| CommandError::from(error.to_string()))?;
    tauri::async_runtime::spawn_blocking(move || {
        let _permit = permit;
        preview(&bundle_root, &image_path, &sha256)
    })
    .await
    .map_err(|error| CommandError::from(error.to_string()))?
    .map_err(|error| CommandError::from(error.to_string()))
}

fn preview(root: &str, file: &str, expected: &str) -> anyhow::Result<String> {
    let root = Path::new(root).canonicalize()?;
    let file = Path::new(file).canonicalize()?;
    anyhow::ensure!(
        file.parent() == Some(root.as_path()),
        "preview image is outside the selected bundle"
    );
    anyhow::ensure!(
        matches!(
            file.extension()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_ascii_lowercase()
                .as_str(),
            "png" | "jpg" | "jpeg"
        ),
        "unsupported preview image"
    );
    anyhow::ensure!(
        fs::metadata(&file)?.len() <= 64 * 1024 * 1024,
        "preview image is too large"
    );
    let mut data = Vec::new();
    fs::File::open(&file)?
        .take(64 * 1024 * 1024 + 1)
        .read_to_end(&mut data)?;
    anyhow::ensure!(
        data.len() <= 64 * 1024 * 1024,
        "preview image grew beyond its limit"
    );
    anyhow::ensure!(
        format!("{:x}", Sha256::digest(&data)) == expected.to_ascii_lowercase(),
        "preview image changed after scan"
    );
    let mut reader = image::ImageReader::new(std::io::Cursor::new(data)).with_guessed_format()?;
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(192 * 1024 * 1024);
    limits.max_image_width = Some(16384);
    limits.max_image_height = Some(16384);
    reader.limits(limits);
    let image = reader.decode()?.thumbnail(960, 960).to_rgb8();
    let mut jpeg = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, 82).encode_image(&image)?;
    Ok(format!(
        "data:image/jpeg;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(jpeg)
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preview_requires_bundle_ownership_and_scan_hash() {
        let root = std::env::temp_dir().join(format!("riviu-preview-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let image = root.join("ảnh 1.jpg");
        image::RgbImage::new(8, 8).save(&image).unwrap();
        let hash = format!("{:x}", Sha256::digest(fs::read(&image).unwrap()));
        assert!(
            preview(root.to_str().unwrap(), image.to_str().unwrap(), &hash)
                .unwrap()
                .starts_with("data:image/jpeg;base64,")
        );
        assert!(preview(root.to_str().unwrap(), image.to_str().unwrap(), "bad").is_err());
        let outside = root.join("other");
        fs::create_dir(&outside).unwrap();
        assert!(preview(outside.to_str().unwrap(), image.to_str().unwrap(), &hash).is_err());
        fs::remove_dir_all(root).unwrap();
    }
}
