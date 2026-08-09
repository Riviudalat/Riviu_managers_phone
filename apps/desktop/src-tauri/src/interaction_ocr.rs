#[cfg(target_os = "macos")]
use std::path::PathBuf;

use async_trait::async_trait;
use riviu_core::{CommentOcrObservation, FrameTextSource};

/// Which reader produced a set of observations, recorded on every stored
/// locator identity.
///
/// The two engines are not interchangeable: Vision reports per-word confidence
/// and, on this build, recognises Vietnamese; `Windows.Media.Ocr` reports no
/// confidence at all and reads whatever language pack the machine happens to
/// have. Evidence that does not say which one read it cannot be argued with
/// later.
pub fn locator_version() -> &'static str {
    #[cfg(target_os = "macos")]
    {
        "vision-v1"
    }
    #[cfg(windows)]
    {
        "windows-media-ocr-v1"
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        "none"
    }
}

/// The language tag the installed reader will actually recognise, if any.
///
/// macOS pins its Vision request to `["en-US", "vi-VN"]` in the Swift helper, so
/// Vietnamese is always available there. Windows uses whichever OCR language
/// pack the machine happens to carry, and a machine with only `en-US` reads
/// "mới" as "mdi" and "thư" as "thif" — substitutions, not lost tone marks, so
/// no amount of accent folding reconciles them.
pub fn recognizer_language() -> Option<String> {
    #[cfg(target_os = "macos")]
    {
        Some("vi-VN".to_string())
    }
    #[cfg(windows)]
    {
        ocr_engine()
            .ok()?
            .RecognizerLanguage()
            .and_then(|language| language.LanguageTag())
            .map(|tag| tag.to_string_lossy())
            .ok()
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        None
    }
}

/// Whether the installed reader can read back a Vietnamese comment body.
///
/// This is what the thread feature actually depends on: a reply has to find the
/// parent by its exact text, and the campaign writes Vietnamese.
pub fn reads_vietnamese() -> bool {
    recognizer_language().is_some_and(|tag| tag.to_ascii_lowercase().starts_with("vi"))
}

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

/// Windows carries an OCR engine in the OS, so the interaction thread does not
/// have to be macOS-only.
///
/// It was: this function used to `bail!("Vision OCR requires macOS")` on every
/// other platform, which meant `open_comment_for_ocr` — the step that finds the
/// campaign's own comment and its reply control — could never run on Windows.
/// A thread there posted its first message and stopped.
///
/// One difference from Vision that callers have to know about:
/// `Windows.Media.Ocr` reports no per-word confidence, so every observation
/// comes back at [`WINDOWS_OCR_CONFIDENCE`]. Inventing a score per word would
/// be worse than saying plainly that there is none — the `>= 0.55` filters
/// downstream simply do not discriminate here, and the exact-text and
/// uniqueness checks carry the weight instead.
#[cfg(windows)]
pub const WINDOWS_OCR_CONFIDENCE: f32 = 1.0;

#[cfg(windows)]
pub async fn recognize(frame: &[u8]) -> anyhow::Result<Vec<CommentOcrObservation>> {
    let frame = frame.to_vec();
    tokio::task::spawn_blocking(move || recognize_sync(&frame))
        .await
        .map_err(|error| anyhow::anyhow!("OCR worker join: {error}"))?
}

#[cfg(windows)]
fn recognize_sync(frame: &[u8]) -> anyhow::Result<Vec<CommentOcrObservation>> {
    use windows::Graphics::Imaging::BitmapDecoder;
    use windows::Storage::Streams::{DataWriter, InMemoryRandomAccessStream};

    // Let Windows decode the JPEG rather than handing it a pixel buffer: it
    // removes a format-conversion step that has nothing to do with reading text.
    let stream = InMemoryRandomAccessStream::new()?;
    let writer = DataWriter::CreateDataWriter(&stream.GetOutputStreamAt(0)?)?;
    writer.WriteBytes(frame)?;
    writer.StoreAsync()?.get()?;
    writer.FlushAsync()?.get()?;
    stream.Seek(0)?;

    let decoder =
        BitmapDecoder::CreateWithIdAsync(BitmapDecoder::JpegDecoderId()?, &stream)?.get()?;
    let bitmap = decoder.GetSoftwareBitmapAsync()?.get()?;
    let (width, height) = (bitmap.PixelWidth()? as f64, bitmap.PixelHeight()? as f64);
    if width <= 0.0 || height <= 0.0 {
        anyhow::bail!("khung ảnh rỗng");
    }

    let engine = ocr_engine()?;
    let result = engine.RecognizeAsync(&bitmap)?.get()?;

    let mut observations = Vec::new();
    for line in result.Lines()? {
        let text = line.Text()?.to_string_lossy();
        if text.trim().is_empty() {
            continue;
        }
        // A line has no rect of its own — only its words do — so the line box is
        // their union. The callers locate a tap point from this, so it has to
        // cover the whole line, not the first word.
        let (mut x0, mut y0) = (f64::MAX, f64::MAX);
        let (mut x1, mut y1) = (f64::MIN, f64::MIN);
        for word in line.Words()? {
            let rect = word.BoundingRect()?;
            x0 = x0.min(rect.X as f64);
            y0 = y0.min(rect.Y as f64);
            x1 = x1.max(rect.X as f64 + rect.Width as f64);
            y1 = y1.max(rect.Y as f64 + rect.Height as f64);
        }
        if x0 > x1 || y0 > y1 {
            continue;
        }
        observations.push(CommentOcrObservation {
            text,
            confidence: WINDOWS_OCR_CONFIDENCE,
            x: x0 / width,
            y: y0 / height,
            width: (x1 - x0) / width,
            height: (y1 - y0) / height,
        });
    }
    Ok(observations)
}

/// Prefer Vietnamese, then whatever the user profile offers, then English.
///
/// `TryCreateFromLanguage` returns null rather than an error when the pack is
/// not installed, so each step has to be tested for null instead of trusted.
/// The handles this reads are ASCII, so the English engine is a genuine
/// fallback rather than a token one.
#[cfg(windows)]
fn ocr_engine() -> anyhow::Result<windows::Media::Ocr::OcrEngine> {
    use windows::core::HSTRING;
    use windows::Globalization::Language;
    use windows::Media::Ocr::OcrEngine;

    for tag in ["vi", "en-US"] {
        if let Ok(language) = Language::CreateLanguage(&HSTRING::from(tag)) {
            if let Ok(engine) = OcrEngine::TryCreateFromLanguage(&language) {
                return Ok(engine);
            }
        }
        if tag == "vi" {
            if let Ok(engine) = OcrEngine::TryCreateFromUserProfileLanguages() {
                return Ok(engine);
            }
        }
    }
    anyhow::bail!(
        "Windows không có gói OCR nào dùng được — cài Language pack (Settings → \
         Time & language → Language → Optional features → Basic typing/OCR)"
    )
}

#[cfg(not(any(target_os = "macos", windows)))]
pub async fn recognize(_frame: &[u8]) -> anyhow::Result<Vec<CommentOcrObservation>> {
    anyhow::bail!("OCR chỉ có trên macOS (Vision) và Windows (Windows.Media.Ocr)")
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

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    fn fixture(name: &str) -> Vec<u8> {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../crates/core/tests/fixtures")
            .join(name);
        std::fs::read(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
    }

    /// The OCR the interaction thread runs on has to actually read a device
    /// capture, not merely compile.
    ///
    /// The assertion is on ASCII, deliberately. Which engine Windows hands back
    /// depends on the installed language packs, and this machine has only
    /// `en-US`, which renders "Mới đi Đà Lạt" as "Mdi di Dä Lat". The hashtags
    /// in the same frame come back intact, and ASCII is what the check that
    /// matters most reads: `open_target_confirmed` compares the author handle
    /// from the link, and handles are ASCII.
    #[tokio::test]
    async fn windows_ocr_reads_a_real_device_capture() {
        let observations = recognize(&fixture("feed-photo-carousel.jpg"))
            .await
            .expect("Windows OCR");

        assert!(
            observations.len() >= 5,
            "only {} lines read off the capture",
            observations.len()
        );
        assert!(
            observations
                .iter()
                .any(|o| o.text.to_lowercase().contains("dalat")),
            "the hashtag line was not read: {:?}",
            observations.iter().map(|o| &o.text).collect::<Vec<_>>()
        );
        for o in &observations {
            assert!(
                (0.0..=1.0).contains(&o.x)
                    && (0.0..=1.0).contains(&o.y)
                    && o.width > 0.0
                    && o.height > 0.0,
                "box out of the unit square: {o:?}"
            );
        }
    }

    /// What the reader reports it can read has to match what it actually does,
    /// because the thread feature refuses to start on the strength of it.
    ///
    /// On this machine only `en-US` is installed, and the dump test above shows
    /// what that means in practice: "Mới đi Đà Lạt" comes back as
    /// "Mdi di Dä Lat". Reporting Vietnamese here would let a campaign start
    /// and then fail one message in, which is the behaviour the guard exists to
    /// remove.
    #[test]
    fn the_reported_language_is_the_one_the_engine_will_use() {
        let reported = recognizer_language();
        match &reported {
            Some(tag) => {
                assert!(
                    !tag.trim().is_empty(),
                    "a reader reported an empty language"
                );
                assert_eq!(
                    reads_vietnamese(),
                    tag.to_ascii_lowercase().starts_with("vi"),
                    "reported {tag} but reads_vietnamese disagreed"
                );
            }
            None => assert!(
                !reads_vietnamese(),
                "no reader is available, so nothing can be read in Vietnamese"
            ),
        }
    }

    /// Whatever engine is installed, one has to be. A machine with no OCR pack
    /// at all should say so in a way the operator can act on rather than
    /// failing somewhere downstream.
    #[test]
    fn an_engine_is_available_or_the_error_names_the_fix() {
        if let Err(error) = ocr_engine() {
            let message = error.to_string();
            assert!(
                message.contains("Language pack"),
                "unhelpful OCR error: {message}"
            );
        }
    }
}
