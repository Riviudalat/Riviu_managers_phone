//! A fresh visual notice after Copy is a pending observation, never publication proof.
use super::*;
use crate::ui_automation::{OcrImage, OcrRequest, OcrResponse};
use base64::Engine;
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessingNotice {
    pub text: String,
    pub before_sha256: String,
    pub after_sha256: String,
}

pub(super) async fn frame(session: &dyn UiSession) -> Option<OcrImage> {
    session.gui_reasoner()?;
    if !session.supports_accessibility_readback() {
        return None;
    }
    let png = tokio::time::timeout(Duration::from_secs(5), session.screenshot_png())
        .await
        .ok()?
        .ok()?;
    if png.len() > 8 * 1024 * 1024 {
        return None;
    }
    tokio::task::spawn_blocking(move || {
        let image = image::load_from_memory(&png).ok()?;
        // Galaxy S8 Trill 38.3.2, machines 1/7, 14/09/2026: processing toast
        // appears above the photo, inside the upper quarter, not in its caption.
        let crop = image.crop_imm(0, 0, image.width(), (image.height() / 4).max(1));
        let mut buffer = std::io::Cursor::new(Vec::new());
        crop.write_to(&mut buffer, image::ImageFormat::Png).ok()?;
        let bytes = buffer.into_inner();
        Some(OcrImage {
            sha256: format!("{:x}", Sha256::digest(&bytes)),
            bytes_base64: base64::engine::general_purpose::STANDARD.encode(bytes),
            width: crop.width(),
            height: crop.height(),
        })
    })
    .await
    .ok()
    .flatten()
}

/// Retain only three fresh upper-quarter observations around the measured toast
/// onset. Capture precedes clipboard/IME reads, which can cover a transient toast.
pub(super) async fn frames_after_copy(
    session: &dyn UiSession,
    started: tokio::time::Instant,
) -> Vec<OcrImage> {
    let deadline = started + Duration::from_millis(2000);
    let mut frames = Vec::with_capacity(3);
    for delay_ms in [150, 450, 900] {
        if tokio::time::Instant::now() >= deadline {
            break;
        }
        tokio::time::sleep_until(started + Duration::from_millis(delay_ms)).await;
        if let Ok(Some(image)) = tokio::time::timeout_at(deadline, frame(session)).await {
            if frames
                .iter()
                .all(|prior: &OcrImage| prior.sha256 != image.sha256)
            {
                frames.push(image);
            }
        }
    }
    frames
}

fn notice(response: &OcrResponse) -> Option<&str> {
    response
        .lines
        .iter()
        .find(|line| {
            line.confidence >= 0.9
                && matches!(
                    line.text
                        .trim()
                        .trim_end_matches('.')
                        .to_lowercase()
                        .as_str(),
                    "post is being processed"
                        | "bài đăng đang được xử lý"
                        | "bài viết đang được xử lý"
                )
        })
        .map(|line| line.text.as_str())
}

pub(super) async fn observe_frames(
    session: &dyn UiSession,
    before: Option<OcrImage>,
    after: Vec<OcrImage>,
) -> Option<ProcessingNotice> {
    let before = before?;
    let reasoner = session.gui_reasoner()?;
    let epoch = session.gui_session_epoch();
    if epoch.is_empty() {
        return None;
    }
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    let observe_id = uuid::Uuid::new_v4().to_string();
    let request = |image, generation| OcrRequest {
        protocol_version: 1,
        request_id: uuid::Uuid::new_v4().to_string(),
        observation_id: observe_id.clone(),
        session_epoch: epoch.clone(),
        generation,
        remaining_ms: 15_000,
        screenshot: image,
        roi: None,
        languages: vec!["en".into(), "vi".into()],
        min_confidence: 0.9,
    };
    let mut detected = None;
    for (index, image) in after.into_iter().take(3).enumerate() {
        if before.sha256 == image.sha256 || session.gui_session_epoch() != epoch {
            continue;
        }
        let after_request = request(image, index as u64 + 2);
        let result = tokio::time::timeout_at(
            deadline.min(tokio::time::Instant::now() + Duration::from_secs(7)),
            reasoner.ocr(after_request.clone()),
        )
        .await;
        let Ok(Ok(result)) = result else {
            continue;
        };
        if result.validate_binding(&after_request).is_err() {
            continue;
        }
        if let Some(text) = notice(&result) {
            detected = Some((text.to_owned(), after_request.screenshot.sha256));
            break;
        }
    }
    let (text, after_sha256) = detected?;
    let before_request = request(before, 1);
    let prior = tokio::time::timeout_at(deadline, reasoner.ocr(before_request.clone()))
        .await
        .ok()?
        .ok()?;
    prior.validate_binding(&before_request).ok()?;
    if notice(&prior).is_some() || session.gui_session_epoch() != epoch {
        return None;
    }
    Some(ProcessingNotice {
        text,
        before_sha256: before_request.screenshot.sha256,
        after_sha256,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui_automation::{GuiReasoner, GuiRequest, GuiResponse, OcrLine, OcrRect};
    use std::sync::Arc;

    struct Reasoner {
        before_has_notice: bool,
        corrupt_binding: bool,
        calls: parking_lot::Mutex<Vec<String>>,
    }
    #[async_trait::async_trait]
    impl GuiReasoner for Reasoner {
        async fn resolve(&self, _: GuiRequest) -> anyhow::Result<GuiResponse> {
            unreachable!()
        }
        async fn ocr(&self, r: OcrRequest) -> anyhow::Result<OcrResponse> {
            self.calls.lock().push(r.screenshot.sha256.clone());
            let present = r.screenshot.sha256 == "toast"
                || (r.screenshot.sha256 == "before" && self.before_has_notice);
            let text = if present {
                "Post is being processed"
            } else {
                ""
            }
            .to_owned();
            Ok(OcrResponse {
                protocol_version: r.protocol_version,
                request_id: r.request_id,
                observation_id: r.observation_id,
                session_epoch: r.session_epoch,
                generation: r.generation,
                screenshot_sha256: if self.corrupt_binding {
                    "stale".into()
                } else {
                    r.screenshot.sha256
                },
                status: if present { "resolved" } else { "unresolved" }.into(),
                text: text.clone(),
                lines: if present {
                    vec![OcrLine {
                        text,
                        bounds: OcrRect {
                            x: 0,
                            y: 0,
                            width: 100,
                            height: 10,
                        },
                        confidence: 0.99,
                    }]
                } else {
                    vec![]
                },
                engine: "fixture".into(),
                elapsed_ms: 1,
            })
        }
    }
    struct Session(Arc<Reasoner>);
    #[async_trait::async_trait]
    impl UiSession for Session {
        fn gui_reasoner(&self) -> Option<crate::ui_automation::SharedReasoner> {
            Some(self.0.clone())
        }
        fn gui_session_epoch(&self) -> String {
            "session".into()
        }
        async fn tap(&self, _: crate::TapPoint) -> anyhow::Result<()> {
            unreachable!()
        }
        async fn swipe(&self, _: crate::SwipeGesture) -> anyhow::Result<()> {
            unreachable!()
        }
        async fn type_text(&self, _: &str) -> anyhow::Result<()> {
            unreachable!()
        }
        async fn home(&self) -> anyhow::Result<()> {
            unreachable!()
        }
        async fn find_and_tap(&self, _: &str) -> anyhow::Result<()> {
            unreachable!()
        }
        async fn assert_visible(&self, _: &str) -> anyhow::Result<()> {
            unreachable!()
        }
        fn stream_url(&self) -> Option<String> {
            None
        }
    }
    fn image(hash: &str) -> OcrImage {
        OcrImage {
            sha256: hash.into(),
            bytes_base64: String::new(),
            width: 1080,
            height: 555,
        }
    }

    #[tokio::test]
    async fn later_fresh_frame_finds_transient_notice_but_old_or_mismatched_frames_do_not() {
        for (before_has_notice, corrupt_binding, expected) in [
            (false, false, true),
            (true, false, false),
            (false, true, false),
        ] {
            let reasoner = Arc::new(Reasoner {
                before_has_notice,
                corrupt_binding,
                calls: parking_lot::Mutex::new(vec![]),
            });
            let session = Session(reasoner.clone());
            let result = observe_frames(
                &session,
                Some(image("before")),
                vec![image("early"), image("toast"), image("late")],
            )
            .await;
            assert_eq!(result.is_some(), expected);
            if let Some(notice) = result {
                assert_eq!(notice.before_sha256, "before");
                assert_eq!(notice.after_sha256, "toast");
                assert_eq!(
                    reasoner.calls.lock().as_slice(),
                    ["early", "toast", "before"]
                );
            }
        }
        let reasoner = Arc::new(Reasoner {
            before_has_notice: false,
            corrupt_binding: false,
            calls: parking_lot::Mutex::new(vec![]),
        });
        assert!(observe_frames(
            &Session(reasoner.clone()),
            Some(image("before")),
            vec![image("before")]
        )
        .await
        .is_none());
        assert!(reasoner.calls.lock().is_empty());
    }
    #[test]
    fn processing_notice_requires_exact_high_confidence_text() {
        let mut response: OcrResponse = serde_json::from_value(serde_json::json!({
            "protocolVersion":1,"requestId":"r","observationId":"o","sessionEpoch":"s",
            "generation":1,"screenshotSha256":"x","status":"resolved","text":"Post is being processed",
            "lines":[{"text":"Post is being processed","bounds":{"x":0,"y":0,"width":100,"height":10},"confidence":0.99}],
            "engine":"fixture","elapsedMs":1
        })).unwrap();
        assert!(notice(&response).is_some());
        response.lines[0].confidence = 0.89;
        assert!(notice(&response).is_none());
        response.lines[0].confidence = 0.99;
        response.lines[0].text = "My caption says Post is being processed".into();
        assert!(notice(&response).is_none());
    }
}
