//! Image-only local OCR contract shared by Flow and the desktop sidecar bridge.
use anyhow::{ensure, Context};
use base64::Engine;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OcrImage {
    pub bytes_base64: String,
    pub sha256: String,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OcrRect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl OcrRect {
    pub fn contains(&self, other: &Self) -> bool {
        other.width > 0
            && other.height > 0
            && other.x >= self.x
            && other.y >= self.y
            && u64::from(other.x) + u64::from(other.width)
                <= u64::from(self.x) + u64::from(self.width)
            && u64::from(other.y) + u64::from(other.height)
                <= u64::from(self.y) + u64::from(self.height)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OcrRequest {
    pub protocol_version: u32,
    pub request_id: String,
    pub observation_id: String,
    pub session_epoch: String,
    pub generation: u64,
    pub remaining_ms: u64,
    pub screenshot: OcrImage,
    pub roi: Option<OcrRect>,
    pub languages: Vec<String>,
    pub min_confidence: f64,
}

impl OcrRequest {
    pub fn region(&self) -> OcrRect {
        self.roi.unwrap_or(OcrRect {
            x: 0,
            y: 0,
            width: self.screenshot.width,
            height: self.screenshot.height,
        })
    }

    pub fn validate(&self) -> anyhow::Result<()> {
        ensure!(
            self.protocol_version == 1
                && self.generation > 0
                && (1..=30_000).contains(&self.remaining_ms)
                && [&self.request_id, &self.observation_id, &self.session_epoch]
                    .iter()
                    .all(|id| !id.is_empty() && id.len() <= 128)
                && self.min_confidence.is_finite()
                && (0.0..=1.0).contains(&self.min_confidence)
                && (1..=2).contains(&self.languages.len())
                && self.languages.iter().enumerate().all(|(i, lang)| matches!(
                    lang.as_str(),
                    "vi" | "en"
                ) && !self.languages[..i]
                    .contains(lang)),
            "gui_ocr_request_invalid"
        );
        let image = &self.screenshot;
        ensure!(
            (1..=8192).contains(&image.width)
                && (1..=8192).contains(&image.height)
                && u64::from(image.width) * u64::from(image.height) <= 8 * 1024 * 1024
                && image.bytes_base64.len() <= (8usize * 1024 * 1024).div_ceil(3) * 4,
            "gui_ocr_image_budget"
        );
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(&image.bytes_base64)
            .context("gui_ocr_image_invalid")?;
        ensure!(
            bytes.len() <= 8 * 1024 * 1024
                && super::profile::CompatibilityPack::sha256(&bytes) == image.sha256,
            "gui_ocr_image_hash"
        );
        let reader = image::ImageReader::new(std::io::Cursor::new(bytes)).with_guessed_format()?;
        ensure!(
            matches!(
                reader.format(),
                Some(image::ImageFormat::Png | image::ImageFormat::Jpeg)
            ),
            "gui_ocr_image_format"
        );
        ensure!(
            reader.into_dimensions()? == (image.width, image.height),
            "gui_ocr_image_dimensions"
        );
        ensure!(
            (OcrRect {
                x: 0,
                y: 0,
                width: image.width,
                height: image.height
            })
            .contains(&self.region()),
            "gui_ocr_roi_invalid"
        );
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OcrLine {
    pub text: String,
    pub bounds: OcrRect,
    pub confidence: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OcrResponse {
    pub protocol_version: u32,
    pub request_id: String,
    pub observation_id: String,
    pub session_epoch: String,
    pub generation: u64,
    pub screenshot_sha256: String,
    pub status: String,
    pub text: String,
    pub lines: Vec<OcrLine>,
    pub engine: String,
    pub elapsed_ms: u64,
}

impl OcrResponse {
    pub fn validate_binding(&self, request: &OcrRequest) -> anyhow::Result<()> {
        ensure!(
            self.protocol_version == request.protocol_version
                && self.request_id == request.request_id
                && self.observation_id == request.observation_id
                && self.session_epoch == request.session_epoch
                && self.generation == request.generation
                && self.screenshot_sha256 == request.screenshot.sha256,
            "gui_ocr_observation_mismatch"
        );
        ensure!(
            self.text.len() <= 4096
                && self.lines.len() <= 128
                && !self.engine.is_empty()
                && self.engine.len() <= 128
                && self.elapsed_ms <= 30_000
                && match self.status.as_str() {
                    "resolved" => !self.lines.is_empty(),
                    "unresolved" => self.lines.is_empty(),
                    _ => false,
                },
            "gui_ocr_response_invalid"
        );
        for line in &self.lines {
            ensure!(
                !line.text.trim().is_empty()
                    && line.text.len() <= 1024
                    && line.confidence.is_finite()
                    && (request.min_confidence..=1.0).contains(&line.confidence)
                    && request.region().contains(&line.bounds),
                "gui_ocr_line_invalid"
            );
        }
        ensure!(
            self.text
                == self
                    .lines
                    .iter()
                    .map(|line| line.text.as_str())
                    .collect::<Vec<_>>()
                    .join("\n"),
            "gui_ocr_text_mismatch"
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_python_request_fixture_roundtrips() {
        let fixture = include_str!("../../fixtures/ocr-request.json");
        let request: OcrRequest = serde_json::from_str(fixture).unwrap();
        request.validate().unwrap();
        let mut wire: serde_json::Value = serde_json::from_str(fixture).unwrap();
        wire["roi"] = serde_json::Value::Null;
        assert_eq!(serde_json::to_value(&request).unwrap(), wire);
    }

    fn fixture() -> OcrRequest {
        let bytes = include_bytes!(
            "../../../../sidecars/gui-service/tests/fixtures/ocr-vietnamese-english.png"
        );
        OcrRequest {
            protocol_version: 1,
            request_id: "ocr-request".into(),
            observation_id: "observed-frame".into(),
            session_epoch: "session".into(),
            generation: 1,
            remaining_ms: 30_000,
            screenshot: OcrImage {
                bytes_base64: base64::engine::general_purpose::STANDARD.encode(bytes),
                sha256: super::super::profile::CompatibilityPack::sha256(bytes),
                width: 900,
                height: 280,
            },
            roi: None,
            languages: vec!["vi".into(), "en".into()],
            min_confidence: 0.7,
        }
    }

    #[test]
    fn validates_real_image_and_rejects_wrong_hash_dimensions_and_roi() {
        let mut request = fixture();
        request.validate().unwrap();
        request.screenshot.width = 899;
        assert!(request.validate().is_err());
        request = fixture();
        request.screenshot.sha256 = "0".repeat(64);
        assert!(request.validate().is_err());
        request = fixture();
        request.roi = Some(OcrRect {
            x: u32::MAX,
            y: 0,
            width: 1,
            height: 1,
        });
        assert!(request.validate().is_err());
    }

    #[test]
    fn rejects_stale_binding_text_invention_low_confidence_and_bounds() {
        let request = fixture();
        let mut response = OcrResponse {
            protocol_version: 1,
            request_id: request.request_id.clone(),
            observation_id: request.observation_id.clone(),
            session_epoch: request.session_epoch.clone(),
            generation: 1,
            screenshot_sha256: request.screenshot.sha256.clone(),
            status: "resolved".into(),
            text: "Xin chào Việt Nam".into(),
            lines: vec![OcrLine {
                text: "Xin chào Việt Nam".into(),
                confidence: 0.95,
                bounds: OcrRect {
                    x: 30,
                    y: 35,
                    width: 340,
                    height: 45,
                },
            }],
            engine: "RapidOCR".into(),
            elapsed_ms: 450,
        };
        response.validate_binding(&request).unwrap();
        response.generation = 2;
        assert!(response.validate_binding(&request).is_err());
        response.generation = 1;
        response.text = "invented".into();
        assert!(response.validate_binding(&request).is_err());
        response.text = response.lines[0].text.clone();
        response.lines[0].confidence = 0.69;
        assert!(response.validate_binding(&request).is_err());
        response.lines[0].confidence = 0.95;
        response.lines[0].bounds.x = 1000;
        assert!(response.validate_binding(&request).is_err());
    }
}
