//! Local template perception: bounded image-only IPC, no device or campaign effects.
use super::{CompatibilityPack, GuiService};
use anyhow::{ensure, Context};
use base64::Engine;
use serde::{Deserialize, Serialize};
use std::{io::Cursor, time::Duration};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EncodedImage {
    pub bytes_base64: String,
    pub sha256: String,
    pub width: u32,
    pub height: u32,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PixelRect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TemplateMatchRequest {
    pub protocol_version: u32,
    pub request_id: String,
    pub observation_id: String,
    pub session_epoch: String,
    pub generation: u64,
    pub remaining_ms: u64,
    pub screenshot: EncodedImage,
    pub template: EncodedImage,
    pub roi: Option<PixelRect>,
    pub scales: Vec<f64>,
    pub threshold: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TemplateCandidate {
    pub bounds: PixelRect,
    pub score: f64,
    pub scale: f64,
    pub method: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TemplateMatchResponse {
    pub protocol_version: u32,
    pub request_id: String,
    pub observation_id: String,
    pub session_epoch: String,
    pub generation: u64,
    pub screenshot_sha256: String,
    pub template_sha256: String,
    pub status: String,
    pub candidates: Vec<TemplateCandidate>,
    pub reason: String,
    pub elapsed_ms: u64,
    pub searched_scales: Vec<f64>,
}

impl PixelRect {
    fn contains(&self, other: &Self) -> bool {
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

impl EncodedImage {
    fn validate(&self, byte_limit: usize, pixel_limit: u64) -> anyhow::Result<()> {
        ensure!(
            (1..=8192).contains(&self.width)
                && (1..=8192).contains(&self.height)
                && u64::from(self.width) * u64::from(self.height) <= pixel_limit
                && self.bytes_base64.len() <= byte_limit.div_ceil(3) * 4,
            "template_image_budget"
        );
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(&self.bytes_base64)
            .context("template_image_invalid")?;
        ensure!(
            bytes.len() <= byte_limit && CompatibilityPack::sha256(&bytes) == self.sha256,
            "template_image_hash"
        );
        let reader = image::ImageReader::new(Cursor::new(bytes)).with_guessed_format()?;
        ensure!(
            matches!(
                reader.format(),
                Some(image::ImageFormat::Png | image::ImageFormat::Jpeg)
            ),
            "template_image_format"
        );
        ensure!(
            reader.into_dimensions()? == (self.width, self.height),
            "template_image_dimensions"
        );
        Ok(())
    }
}

impl TemplateMatchRequest {
    fn region(&self) -> PixelRect {
        self.roi.unwrap_or(PixelRect {
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
                && self.threshold.is_finite()
                && (0.5..=1.0).contains(&self.threshold)
                && (1..=9).contains(&self.scales.len()),
            "template_request_invalid"
        );
        self.screenshot.validate(8 * 1024 * 1024, 8 * 1024 * 1024)?;
        self.template.validate(2 * 1024 * 1024, 1024 * 1024)?;
        let region = self.region();
        ensure!(
            (PixelRect {
                x: 0,
                y: 0,
                width: self.screenshot.width,
                height: self.screenshot.height
            })
            .contains(&region),
            "template_roi_invalid"
        );
        let mut positions = 0u64;
        for (index, scale) in self.scales.iter().enumerate() {
            ensure!(
                scale.is_finite()
                    && (0.5..=2.0).contains(scale)
                    && !self.scales[..index].contains(scale),
                "template_scale_invalid"
            );
            let width = (f64::from(self.template.width) * scale)
                .round_ties_even()
                .max(1.0) as u32;
            let height = (f64::from(self.template.height) * scale)
                .round_ties_even()
                .max(1.0) as u32;
            if width <= region.width && height <= region.height {
                positions +=
                    u64::from(region.width - width + 1) * u64::from(region.height - height + 1);
            }
        }
        ensure!(positions <= 32 * 1024 * 1024, "template_search_budget");
        Ok(())
    }
}

impl TemplateMatchResponse {
    pub fn validate_binding(&self, request: &TemplateMatchRequest) -> anyhow::Result<()> {
        ensure!(
            self.protocol_version == request.protocol_version
                && self.request_id == request.request_id
                && self.observation_id == request.observation_id
                && self.session_epoch == request.session_epoch
                && self.generation == request.generation
                && self.screenshot_sha256 == request.screenshot.sha256
                && self.template_sha256 == request.template.sha256,
            "template_observation_mismatch"
        );
        ensure!(
            self.reason.len() <= 256 && self.candidates.len() <= 8,
            "template_response_invalid"
        );
        ensure!(
            match self.status.as_str() {
                "resolved" => self.candidates.len() == 1,
                "unresolved" => self.candidates.is_empty(),
                "ambiguous" => !self.candidates.is_empty(),
                _ => false,
            },
            "template_status_invalid"
        );
        ensure!(
            self.searched_scales.len() <= request.scales.len(),
            "template_scale_invalid"
        );
        for (index, scale) in self.searched_scales.iter().enumerate() {
            ensure!(
                request.scales.contains(scale) && !self.searched_scales[..index].contains(scale),
                "template_scale_invalid"
            );
        }
        for (index, candidate) in self.candidates.iter().enumerate() {
            ensure!(
                candidate.method == "templateCorrelation"
                    && candidate.score.is_finite()
                    && (request.threshold..=1.0).contains(&candidate.score)
                    && self.searched_scales.contains(&candidate.scale)
                    && request.region().contains(&candidate.bounds)
                    && candidate.bounds.width
                        == (f64::from(request.template.width) * candidate.scale)
                            .round_ties_even()
                            .max(1.0) as u32
                    && candidate.bounds.height
                        == (f64::from(request.template.height) * candidate.scale)
                            .round_ties_even()
                            .max(1.0) as u32
                    && !self.candidates[..index]
                        .iter()
                        .any(|prior| prior.bounds == candidate.bounds),
                "template_candidate_invalid"
            );
        }
        Ok(())
    }
}

impl GuiService {
    pub async fn template_match(
        &self,
        request: TemplateMatchRequest,
    ) -> anyhow::Result<TemplateMatchResponse> {
        let duration = Duration::from_millis(request.remaining_ms.min(30_000));
        tokio::time::timeout(duration, async {
            let permit = self.capacity.clone().acquire_owned().await?;
            // Hashing and image-header parsing must not block Tauri's async executor.
            let (request, _permit) = tokio::task::spawn_blocking(move || {
                request.validate()?;
                Ok::<_, anyhow::Error>((request, permit))
            })
            .await??;
            let (url, token) = self.connection_for(false).await?;
            let mut response = reqwest::Client::builder()
                .timeout(duration)
                .redirect(reqwest::redirect::Policy::none())
                .no_proxy()
                .build()?
                .post(format!("{url}/v1/gui/template-match"))
                .bearer_auth(token)
                .json(&request)
                .send()
                .await?;
            ensure!(
                response.status().is_success(),
                "gui_template_http_{}",
                response.status().as_u16()
            );
            let mut bytes = Vec::new();
            while let Some(chunk) = response.chunk().await? {
                ensure!(
                    bytes.len() + chunk.len() <= 32 * 1024,
                    "template_response_too_large"
                );
                bytes.extend_from_slice(&chunk);
            }
            let response: TemplateMatchResponse = serde_json::from_slice(&bytes)?;
            response.validate_binding(&request)?;
            Ok::<_, anyhow::Error>(response)
        })
        .await
        .map_err(|_| anyhow::anyhow!("gui_deadline"))?
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request() -> TemplateMatchRequest {
        serde_json::from_str(include_str!(
            "../../../../../sidecars/gui-service/tests/fixtures/template-request.json"
        ))
        .unwrap()
    }

    fn response(request: &TemplateMatchRequest) -> TemplateMatchResponse {
        TemplateMatchResponse {
            protocol_version: 1,
            request_id: request.request_id.clone(),
            observation_id: request.observation_id.clone(),
            session_epoch: request.session_epoch.clone(),
            generation: request.generation,
            screenshot_sha256: request.screenshot.sha256.clone(),
            template_sha256: request.template.sha256.clone(),
            status: "resolved".into(),
            reason: "template_unique_match".into(),
            elapsed_ms: 1,
            searched_scales: vec![1.0],
            candidates: vec![TemplateCandidate {
                bounds: PixelRect {
                    x: 27,
                    y: 39,
                    width: 23,
                    height: 19,
                },
                score: 1.0,
                scale: 1.0,
                method: "templateCorrelation".into(),
            }],
        }
    }

    #[test]
    fn shared_python_contract_validates_image_headers_hashes_and_limits() {
        let valid = request();
        valid.validate().unwrap();
        let mut invalid = valid.clone();
        invalid.screenshot.sha256 = "0".repeat(64);
        assert!(invalid.validate().unwrap_err().to_string().contains("hash"));
        let mut invalid = valid.clone();
        invalid.screenshot.width += 1;
        assert!(invalid
            .validate()
            .unwrap_err()
            .to_string()
            .contains("dimensions"));
        let mut invalid = valid.clone();
        invalid.roi = Some(PixelRect {
            x: u32::MAX,
            y: 0,
            width: 2,
            height: 2,
        });
        assert!(invalid.validate().unwrap_err().to_string().contains("roi"));
        let mut invalid = valid;
        invalid.scales = vec![1.0, 1.0];
        assert!(invalid
            .validate()
            .unwrap_err()
            .to_string()
            .contains("scale"));
    }

    #[test]
    fn mismatched_observation_hash_and_epoch_never_reach_ui_as_results() {
        let request = request();
        for field in [
            "requestId",
            "observationId",
            "sessionEpoch",
            "screenshotSha256",
            "templateSha256",
        ] {
            let mut raw = serde_json::to_value(response(&request)).unwrap();
            raw[field] = "stale".into();
            let invalid: TemplateMatchResponse = serde_json::from_value(raw).unwrap();
            assert!(invalid.validate_binding(&request).is_err(), "{field}");
        }
        let mut invalid = response(&request);
        invalid.generation += 1;
        assert!(invalid.validate_binding(&request).is_err());
    }

    #[test]
    fn invented_out_of_roi_wrong_size_and_below_threshold_candidates_fail() {
        let request = request();
        response(&request).validate_binding(&request).unwrap();
        for change in 0..6 {
            let mut invalid = response(&request);
            match change {
                0 => invalid.candidates[0].bounds.x = 79,
                1 => invalid.candidates[0].score = 0.1,
                2 => invalid.candidates[0].scale = 1.5,
                3 => invalid.candidates[0].bounds.width = 24,
                4 => invalid.candidates[0].method = "invented".into(),
                _ => invalid.candidates.push(invalid.candidates[0].clone()),
            }
            assert!(invalid.validate_binding(&request).is_err(), "{change}");
        }
    }

    #[tokio::test]
    async fn local_service_roundtrip_works_without_provider_credentials_or_ai_enabled() {
        let directory = std::env::temp_dir().join(format!(
            "riviu-local-template-test-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&directory).unwrap();
        let db =
            std::sync::Arc::new(super::super::Database::open(directory.join("test.db")).unwrap());
        let service = GuiService::new(db, None, directory.join("gui"));
        service
            .save(super::super::GuiConfig {
                enabled: false,
                ..Default::default()
            })
            .await
            .unwrap();
        let request = request();
        let result = service.template_match(request.clone()).await;
        service.stop().await;
        let result = result.unwrap();
        result.validate_binding(&request).unwrap();
        assert_eq!(result.status, "resolved");
        assert_eq!(
            result.candidates[0].bounds,
            PixelRect {
                x: 27,
                y: 39,
                width: 23,
                height: 19
            }
        );
    }
}
