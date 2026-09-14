//! Local OCR uses the same observation DTO as Flow; no provider account is needed.
use super::GuiService;
use anyhow::ensure;
use riviu_core::ui_automation::{OcrRequest, OcrResponse};
use std::time::Duration;

impl GuiService {
    pub async fn recognize_text(&self, request: OcrRequest) -> anyhow::Result<OcrResponse> {
        let duration = Duration::from_millis(request.remaining_ms.min(30_000));
        tokio::time::timeout(duration, async {
            let permit = self.capacity.clone().acquire_owned().await?;
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
                .post(format!("{url}/v1/gui/ocr"))
                .bearer_auth(token)
                .json(&request)
                .send()
                .await?;
            ensure!(
                response.status().is_success(),
                "gui_ocr_http_{}",
                response.status().as_u16()
            );
            let mut bytes = Vec::new();
            while let Some(chunk) = response.chunk().await? {
                ensure!(
                    bytes.len() + chunk.len() <= 128 * 1024,
                    "gui_ocr_response_budget"
                );
                bytes.extend_from_slice(&chunk);
            }
            let result: OcrResponse = serde_json::from_slice(&bytes)?;
            result.validate_binding(&request)?;
            Ok(result)
        })
        .await
        .map_err(|_| anyhow::anyhow!("gui_deadline"))?
    }
}
