//! OpenAI-compatible chat client used for TikTok comments.
//!
//! The client targets an OpenAI-compatible gateway configured by `base_url`.
//! Production text comments
//! use [`prepare_grounded_comment`]: a three-frame contact sheet goes through a
//! grounded draft pass and an independent verification pass. Ambiguous or
//! unavailable context is skipped; the old pool helpers remain only for legacy
//! fixtures and are not a production fallback.
//!
//! Anything the model returns is treated as untrusted text: it is stripped of
//! reasoning blocks and quoting, collapsed to one line and word-capped before
//! it can be typed into someone's comment box.

use anyhow::{anyhow, Context};
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use image::{imageops::FilterType, GenericImage, Rgb, RgbImage};
use rand::seq::SliceRandom;
use serde::Deserialize;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::io::Cursor;

use crate::{interaction::CommentOcrObservation, types::NurtureSettings};

/// A comment longer than this is a malfunction, not a comment: real TikTok
/// comments are short, and a long reply means the model answered the prompt
/// instead of writing a comment.
const MAX_SANE_WORDS: usize = 30;

#[derive(Debug, Clone)]
pub struct VisionCommentResult {
    pub text: String,
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub usd: f64,
    pub model: String,
    pub base_url_host: String,
}

/// A comment that passed the contextual generation and independent relevance
/// checks. The sender is deliberately given this prepared value rather than
/// calling the model while the comment drawer is open.
#[derive(Debug, Clone)]
pub struct GroundedCommentResult {
    pub text: String,
    pub caption: Option<String>,
    pub context_confidence: u8,
    pub relevance: u8,
    pub evidence_support: u8,
    pub frame_sha256: String,
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub usd: f64,
    pub model: String,
    pub base_url_host: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct VerificationGate {
    overall: u8,
    instruction_fit: u8,
    genericity: u8,
    contradiction: bool,
    unsupported_claim: bool,
    ui_text_confusion: bool,
    formal_style: bool,
}

impl VerificationGate {
    fn accepts(self) -> bool {
        self.overall >= 80
            && self.instruction_fit >= 70
            && self.genericity <= 30
            && !self.contradiction
            && !self.unsupported_claim
            && !self.ui_text_confusion
            && !self.formal_style
    }

    fn retryable(self) -> bool {
        self.overall >= 60
            && !self.contradiction
            && !self.unsupported_claim
            && !self.ui_text_confusion
    }

    fn accepts_caption(self, context_confidence: u8) -> bool {
        // OCR captions are often short, hashtags, or partially visible. Keep
        // the hard safety flags and natural-style checks, but do not require
        // the same evidence score as the multi-frame vision path.
        context_confidence >= 70
            && self.overall >= 60
            && self.instruction_fit >= 70
            && self.genericity <= 35
            && !self.contradiction
            && !self.unsupported_claim
            && !self.ui_text_confusion
            && !self.formal_style
    }
}

#[derive(Debug, Deserialize)]
struct ChatResponse {
    choices: Vec<Choice>,
    usage: Option<Usage>,
    model: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Choice {
    message: Message,
}

#[derive(Debug, Deserialize)]
struct Message {
    content: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Usage {
    prompt_tokens: Option<u32>,
    completion_tokens: Option<u32>,
}

pub fn estimate_usd(settings: &NurtureSettings, prompt: u32, completion: u32) -> f64 {
    (prompt as f64 * settings.input_price_per_1m + completion as f64 * settings.output_price_per_1m)
        / 1_000_000.0
}

pub fn host_of(base_url: &str) -> String {
    base_url
        .trim()
        .trim_end_matches('/')
        .split("://")
        .nth(1)
        .unwrap_or(base_url)
        .split('/')
        .next()
        .unwrap_or(base_url)
        .to_string()
}

/// Whether the configured endpoint accepts image content parts.
///
/// Keyed on host, not model, and that is deliberate: measured against
/// `api.deepseek.com` on 09/08/2026, **both** `deepseek-v4-flash` and
/// `deepseek-v4-pro` reject an `image_url` part with
/// `unknown variant "image_url", expected "text"`. Serde names exactly one
/// variant there, so the content-part enum has no image case at all — the
/// limit is the endpoint's request schema, not the model's capability, and no
/// model string reaches vision through it. Whatever a DeepSeek model can do
/// elsewhere, this API surface cannot carry a picture to it.
///
/// A `false` here is not a refusal: callers fall back to a locally OCR'd
/// caption and the caption-scored gate (`accepts_caption`). Re-measure before
/// trusting this — the day DeepSeek ships an image part, this goes stale
/// silently.
pub fn provider_supports_vision(settings: &NurtureSettings) -> bool {
    !host_of(&settings.base_url).eq_ignore_ascii_case("api.deepseek.com")
}

fn client() -> anyhow::Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(45))
        .build()?)
}

async fn chat(
    settings: &NurtureSettings,
    body: serde_json::Value,
) -> anyhow::Result<(String, u32, u32, String)> {
    if settings.api_key.trim().is_empty() {
        return Err(anyhow!("API key trống — điền trong menu Nuôi TikTok"));
    }
    let base = settings.base_url.trim().trim_end_matches('/');
    let url = format!("{base}/chat/completions");
    let resp = client()?
        .post(&url)
        .bearer_auth(settings.api_key.trim())
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await
        .context("gọi API comment thất bại")?;
    let status = resp.status();
    let raw = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        let msg = serde_json::from_str::<serde_json::Value>(&raw)
            .ok()
            .and_then(|v| v["error"]["message"].as_str().map(|s| s.to_string()))
            .unwrap_or_else(|| raw.chars().take(200).collect());
        return Err(anyhow!("API {status}: {msg}"));
    }
    let parsed: ChatResponse = serde_json::from_str(&raw).context("parse API response")?;
    let text = parsed
        .choices
        .first()
        .and_then(|c| c.message.content.clone())
        .unwrap_or_default();
    let prompt_tokens = parsed
        .usage
        .as_ref()
        .and_then(|u| u.prompt_tokens)
        .unwrap_or(0);
    let completion_tokens = parsed
        .usage
        .as_ref()
        .and_then(|u| u.completion_tokens)
        .unwrap_or(0);
    let model = parsed.model.unwrap_or_else(|| settings.model.clone());
    Ok((text, prompt_tokens, completion_tokens, model))
}

/// One comment for the video currently on screen.
pub async fn generate_vision_comment(
    settings: &NurtureSettings,
    jpeg_bytes: &[u8],
    direction: Option<&str>,
) -> anyhow::Result<VisionCommentResult> {
    let b64 = B64.encode(jpeg_bytes);
    let max_words = settings.max_comment_words.max(4) as usize;
    let lang = language_label(&settings.comment_lang);
    let prompt = vision_prompt(&lang, max_words, direction);

    let body = json!({
        "model": settings.model,
        "temperature": 0.9,
        "max_tokens": 400,
        "stream": false,
        "messages": [{
            "role": "user",
            "content": [
                { "type": "image_url",
                  "image_url": { "url": format!("data:image/jpeg;base64,{b64}") } },
                { "type": "text", "text": prompt }
            ]
        }]
    });

    let (raw, prompt_tokens, completion_tokens, model) = chat(settings, body).await?;
    let text = sanitize_comment(&raw, max_words)
        .ok_or_else(|| anyhow!("model trả comment không dùng được: {:.60}", raw))?;
    Ok(VisionCommentResult {
        text,
        prompt_tokens,
        completion_tokens,
        usd: estimate_usd(settings, prompt_tokens, completion_tokens),
        model,
        base_url_host: host_of(&settings.base_url),
    })
}

/// Generate and independently validate one comment from a small frame set.
///
/// The frame set is turned into one contact sheet so OpenAI-compatible gateways
/// that only support the existing single-image request shape remain usable. The
/// second request receives the image and exact sanitized candidate, but not the
/// first request's self-reported facts, which keeps the check independent.
pub async fn prepare_grounded_comment(
    settings: &NurtureSettings,
    frames: &[Vec<u8>],
    direction: Option<&str>,
) -> anyhow::Result<GroundedCommentResult> {
    if settings.api_key.trim().is_empty() {
        return Err(anyhow!("ai_unavailable"));
    }
    if frames.is_empty() {
        return Err(anyhow!("no_usable_evidence"));
    }
    let sheet = make_contact_sheet(frames)?;
    let frame_sha256 = sha256_hex(&sheet);
    let max_words = settings.max_comment_words.clamp(4, 30) as usize;
    let lang = language_label(&settings.comment_lang);
    let direction = direction.map(str::trim).filter(|d| !d.is_empty());
    let mut total_prompt = 0u32;
    let mut total_completion = 0u32;
    let mut last_gate = None;

    // Two attempts, and **both failure kinds get to use the second one**. The retry existed
    // only for a draft the verifier disliked; a draft that came back unreadable — truncated
    // JSON, a markdown fence, an empty `comment` field — took the `?` straight out of the
    // loop and the post got nothing. Measured on six phones on 19/08/2026: of eleven
    // attempts, four posted, two were fairly rejected by the gate, and **five died on the
    // first unreadable draft** without ever asking again. Asking twice costs one more call
    // on the posts that need it and nothing at all on the ones that do not.
    let mut last_error: Option<String> = None;
    for attempt in 0..2 {
        let draft =
            match grounded_generate(settings, &sheet, &lang, max_words, direction, attempt > 0)
                .await
            {
                Ok(draft) => draft,
                Err(error) => {
                    last_error = Some(error.to_string());
                    if attempt == 0 {
                        continue;
                    }
                    break;
                }
            };
        total_prompt = total_prompt.saturating_add(draft.prompt_tokens);
        total_completion = total_completion.saturating_add(draft.completion_tokens);
        let Some(candidate) = sanitize_comment(&draft.comment, max_words) else {
            last_error = Some(format!("unusable_draft: {}", model_said(&draft.comment)));
            if attempt == 0 {
                continue;
            }
            break;
        };
        let verification = grounded_verify(settings, &sheet, &candidate, direction).await?;
        total_prompt = total_prompt.saturating_add(verification.prompt_tokens);
        total_completion = total_completion.saturating_add(verification.completion_tokens);
        let gate = VerificationGate {
            overall: draft
                .context_confidence
                .min(verification.relevance)
                .min(verification.evidence_support),
            instruction_fit: verification.instruction_fit,
            genericity: verification.genericity,
            contradiction: verification.contradiction,
            unsupported_claim: verification.unsupported_claim,
            ui_text_confusion: verification.ui_text_confusion,
            formal_style: sounds_like_report(&candidate),
        };
        if gate.accepts() {
            return Ok(GroundedCommentResult {
                text: candidate,
                caption: draft.caption,
                context_confidence: draft.context_confidence,
                relevance: verification.relevance,
                evidence_support: verification.evidence_support,
                frame_sha256,
                prompt_tokens: total_prompt,
                completion_tokens: total_completion,
                usd: estimate_usd(settings, total_prompt, total_completion),
                model: verification.model,
                base_url_host: host_of(&settings.base_url),
            });
        }
        last_gate = Some(gate);
        if attempt == 0 && gate.retryable() {
            continue;
        }
        break;
    }
    // A gate verdict is the more informative ending, so it wins when there is one. But a run
    // that never reached the gate has to say what actually stopped it, rather than reporting
    // a rejection that never happened — which is what `no_gate` used to do.
    if let Some(gate) = last_gate {
        return Err(anyhow!(
            "comment_context_rejected: context={} overall={} instruction={} genericity={}",
            gate.overall,
            gate.overall,
            gate.instruction_fit,
            gate.genericity
        ));
    }
    Err(anyhow!(
        "{}",
        last_error.unwrap_or_else(|| "no_gate".to_string())
    ))
}

/// Prepare a grounded comment for a text-only provider from caption text
/// extracted locally from the current frame. This is intentionally separate
/// from the image path: the provider never receives a fake image payload.
pub async fn prepare_caption_comment(
    settings: &NurtureSettings,
    caption: &str,
    frame_sha256: &str,
    direction: Option<&str>,
) -> anyhow::Result<GroundedCommentResult> {
    let caption = caption.trim();
    if caption.is_empty() {
        return Err(anyhow!("no_usable_evidence"));
    }
    let max_words = settings.max_comment_words.clamp(4, 30) as usize;
    let direction = direction.map(str::trim).filter(|value| !value.is_empty());
    let direction_text = direction.unwrap_or("tự nhiên");
    let mut total_prompt_tokens = 0u32;
    let mut total_completion_tokens = 0u32;
    let mut last_gate = None;

    for attempt in 0..2 {
        let retry_note = if attempt == 0 {
            ""
        } else {
            "Lượt trước bị chấm quá trang trọng; viết lại như phản ứng nói miệng ngắn của người vừa xem, không tóm tắt và không dùng giọng quảng cáo."
        };
        let draft_prompt = format!(
            "Bạn viết comment TikTok ngắn từ caption đã OCR ở frame hiện tại.\n\
             Caption OCR (bằng chứng duy nhất): {caption:?}\n\
             Định hướng giọng điệu: {direction_text:?}.\n\
             {retry_note}\n\
             Trả về JSON duy nhất, không markdown, theo schema: {{\"caption\":string,\"contextConfidence\":0..100,\"comment\":string}}.\n\
             Comment tiếng {lang}, tối đa {max_words} từ, thường 2-10 từ, thân mật như người vừa xem xong.\n\
             Chỉ phản hồi chi tiết có trong caption; không bịa địa điểm, người, giá, sản phẩm hoặc sự kiện. Không viết kiểu báo cáo, quảng cáo hay lời khen rỗng.\n\
             ",
            lang = language_label(&settings.comment_lang),
        );
        let (draft_raw, draft_prompt_tokens, draft_completion_tokens, _draft_model) =
            chat(settings, text_body(settings, draft_prompt, 0.7, 300)).await?;
        total_prompt_tokens = total_prompt_tokens.saturating_add(draft_prompt_tokens);
        total_completion_tokens = total_completion_tokens.saturating_add(draft_completion_tokens);
        let draft_value =
            json_object(&draft_raw).ok_or_else(|| anyhow!("malformed_model_output"))?;
        let candidate = sanitize_comment(
            draft_value
                .get("comment")
                .and_then(|value| value.as_str())
                .unwrap_or_default(),
            max_words,
        )
        .ok_or_else(|| anyhow!("malformed_model_output"))?;
        let context_confidence = score(
            draft_value
                .get("contextConfidence")
                .or_else(|| draft_value.get("confidence")),
        )?;

        let verify_prompt = format!(
            "Kiểm tra comment TikTok ứng viên dựa đúng trên caption OCR dưới đây.\n\
             Caption OCR: {caption:?}\n\
             Comment ứng viên: {candidate:?}\n\
             Định hướng: {direction_text:?}.\n\
             Trả về JSON duy nhất: {{\"relevance\":0..100,\"evidenceSupport\":0..100,\"instructionFit\":0..100,\"genericity\":0..100,\"contradiction\":boolean,\"unsupportedClaim\":boolean,\"uiTextConfusion\":boolean}}.\
             relevance/evidenceSupport chỉ chấm điều có thể đối chiếu với caption; instructionFit thấp nếu câu nghe như báo cáo; genericity cao nếu khen rỗng.",
        );
        let (verify_raw, verify_prompt_tokens, verify_completion_tokens, model) =
            chat(settings, text_body(settings, verify_prompt, 0.0, 240)).await?;
        total_prompt_tokens = total_prompt_tokens.saturating_add(verify_prompt_tokens);
        total_completion_tokens = total_completion_tokens.saturating_add(verify_completion_tokens);
        let value = json_object(&verify_raw).ok_or_else(|| anyhow!("malformed_model_output"))?;
        let relevance = score(value.get("relevance"))?;
        let evidence_support = score(value.get("evidenceSupport"))?;
        let gate = VerificationGate {
            overall: relevance.min(evidence_support),
            instruction_fit: score(value.get("instructionFit"))?,
            genericity: score(value.get("genericity"))?,
            contradiction: value
                .get("contradiction")
                .and_then(|item| item.as_bool())
                .unwrap_or(true),
            unsupported_claim: value
                .get("unsupportedClaim")
                .and_then(|item| item.as_bool())
                .unwrap_or(true),
            ui_text_confusion: value
                .get("uiTextConfusion")
                .and_then(|item| item.as_bool())
                .unwrap_or(true),
            formal_style: sounds_like_report(&candidate),
        };
        if gate.accepts_caption(context_confidence) {
            return Ok(GroundedCommentResult {
                text: candidate,
                caption: Some(caption.chars().take(240).collect()),
                context_confidence,
                relevance,
                evidence_support,
                frame_sha256: frame_sha256.to_string(),
                prompt_tokens: total_prompt_tokens,
                completion_tokens: total_completion_tokens,
                usd: estimate_usd(settings, total_prompt_tokens, total_completion_tokens),
                model,
                base_url_host: host_of(&settings.base_url),
            });
        }
        last_gate = Some(gate);
        if attempt == 0 && gate.retryable() {
            continue;
        }
        break;
    }

    let detail = last_gate
        .map(|gate| {
            format!(
                "context={} overall={} instruction={} genericity={}",
                gate.overall, gate.overall, gate.instruction_fit, gate.genericity
            )
        })
        .unwrap_or_else(|| "no_gate".to_string());
    Err(anyhow!("comment_context_rejected: {detail}"))
}

/// Keep only likely caption lines from platform OCR. Bottom navigation and
/// account chrome are deliberately excluded before text is sent to a model.
pub fn ocr_caption(observations: &[CommentOcrObservation]) -> Option<String> {
    let mut lines = observations
        .iter()
        .filter(|observation| {
            observation.confidence >= 0.4
                && (0.67..=0.91).contains(&observation.y)
                && observation.x < 0.8
                && observation.width >= 0.04
                && observation.text.trim().chars().count() >= 2
        })
        .filter_map(|observation| {
            let text = observation
                .text
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ");
            let normalized = text.to_lowercase();
            let metadata_line = normalized.starts_with("địa điểm này có")
                || normalized.contains("lượt thích")
                || normalized == "• ảnh"
                || normalized == "bóc tem"
                || normalized == "chia sẻ";
            (!metadata_line
                && !matches!(
                    normalized.as_str(),
                    "trang chủ" | "cửa hàng" | "hộp thư" | "hồ sơ" | "đã follow" | "bạn bè"
                ))
            .then_some((observation.y, observation.x, text))
        })
        .collect::<Vec<_>>();
    lines.sort_by(|left, right| {
        left.0
            .total_cmp(&right.0)
            .then_with(|| left.1.total_cmp(&right.1))
    });
    let mut unique = Vec::new();
    for (_, _, line) in lines {
        if unique.iter().all(|known: &String| known != &line) {
            unique.push(line);
        }
        if unique.len() >= 6 {
            break;
        }
    }
    (!unique.is_empty()).then(|| unique.join(" "))
}

#[derive(Debug)]
struct GroundedDraft {
    comment: String,
    caption: Option<String>,
    context_confidence: u8,
    prompt_tokens: u32,
    completion_tokens: u32,
}

#[derive(Debug)]
struct GroundedVerification {
    relevance: u8,
    evidence_support: u8,
    instruction_fit: u8,
    genericity: u8,
    contradiction: bool,
    unsupported_claim: bool,
    ui_text_confusion: bool,
    prompt_tokens: u32,
    completion_tokens: u32,
    model: String,
}

/// A bounded, single-line look at what a model actually said.
///
/// For error strings that an operator reads. `malformed_model_output` on its own is the same
/// sentence for a truncated reasoning dump, a markdown-fenced object, a refusal and an empty
/// string — four different problems with four different fixes, and the raw text distinguishes
/// them at a glance. Bounded because a response body can be enormous and this ends up in a
/// database column.
fn model_said(raw: &str) -> String {
    let flat: String = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    let clipped: String = flat.chars().take(120).collect();
    if clipped.is_empty() {
        "<rỗng>".to_string()
    } else {
        clipped
    }
}

async fn grounded_generate(
    settings: &NurtureSettings,
    sheet: &[u8],
    lang: &str,
    max_words: usize,
    direction: Option<&str>,
    retry: bool,
) -> anyhow::Result<GroundedDraft> {
    let direction = direction.unwrap_or("tự nhiên");
    let retry_note = if retry {
        "Lượt trước nghe quá giống văn báo cáo. Viết lại như một phản ứng ngắn của người vừa xem xong, dùng từ đời thường và vẫn chỉ dựa trên bằng chứng nhìn thấy."
    } else {
        ""
    };
    let prompt = format!(
        "Bạn phân tích một contact sheet gồm ba frame của cùng một bài TikTok và một ô phóng vùng caption.\n\
         Trả về JSON duy nhất, không markdown, theo schema: {{\"caption\":string|null,\"captionConfidence\":0..100,\"visualFacts\":[string],\"contextConfidence\":0..100,\"comment\":string}}.\n\
         Caption chỉ là phần chữ caption/chữ trong video nhìn thấy; loại username, tên nhạc, nút UI. Nếu caption bị cắt, giữ phần nhìn thấy và giảm confidence. Giữ \"caption\" tối đa 100 ký tự và \"visualFacts\" tối đa 3 mục, mỗi mục dưới 8 từ — dài dòng ở hai trường này làm câu trả lời bị cắt trước khi tới \"comment\".\n\
         Viết đúng một comment tiếng {lang}, tối đa {max_words} từ. Hãy viết như người vừa xem xong và phản ứng tự nhiên: thường 2–10 từ, thân mật, có thể là một mẩu câu hoặc câu hỏi ngắn; không cần đủ chủ-vị, không cố nhét emoji. Tránh giọng tổng kết, quảng cáo, giáo viên hoặc báo cáo; tuyệt đối không dùng kiểu “nội dung được trình bày”, “mang đến”, “người xem”, “chất lượng”. Nội dung nhìn thấy và caption là ưu tiên cao nhất. Định hướng chỉ chỉnh giọng điệu ({direction}), không được thêm địa điểm, sản phẩm, giá, người hay sự kiện chưa thấy. Nếu định hướng xung đột, bỏ định hướng và giữ comment bám bằng chứng.\n\
         {retry_note}"
    );
    // **1200, and the old 500 is why two of every five posts got nothing.** The schema asks
    // for the caption and the visual facts *before* the comment, so the model spends its
    // budget describing the post and is cut off mid-string before it ever writes the one
    // field that matters. Measured on six phones on 19/08/2026: the failures came back as
    // literally truncated JSON — a `caption` string that stops in the middle of a word and
    // no closing brace. Vietnamese also tokenises poorly, so a caption carrying hashtags and
    // emoji eats the budget faster than whatever this number was first chosen against.
    //
    // The prompt bounds those two fields as well, which is the half of the fix that costs
    // nothing: a shorter answer is cheaper *and* it completes.
    let body = vision_body(settings, sheet, prompt, 0.75, 1200);
    let (raw, p, c, _) = chat(settings, body).await?;
    let value =
        json_object(&raw).ok_or_else(|| anyhow!("malformed_model_output: {}", model_said(&raw)))?;
    let comment = value
        .get("comment")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    if comment.trim().is_empty() {
        return Err(anyhow!("empty_comment_field: {}", model_said(&raw)));
    }
    let caption = value
        .get("caption")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let visual_facts = value
        .get("visualFacts")
        .and_then(|facts| facts.as_array())
        .map(|facts| {
            facts.iter().any(|fact| {
                fact.as_str()
                    .map(|text| !text.trim().is_empty())
                    .unwrap_or(false)
            })
        })
        .unwrap_or(false);
    if !visual_facts && caption.is_none() {
        return Err(anyhow!("no_usable_evidence"));
    }
    let context_confidence = score(value.get("contextConfidence"))?;
    let caption_confidence = score(value.get("captionConfidence"))?;
    let context_confidence = context_confidence.min(if caption.is_some() {
        caption_confidence
    } else {
        100
    });
    Ok(GroundedDraft {
        comment,
        caption,
        context_confidence,
        prompt_tokens: p,
        completion_tokens: c,
    })
}

async fn grounded_verify(
    settings: &NurtureSettings,
    sheet: &[u8],
    candidate: &str,
    direction: Option<&str>,
) -> anyhow::Result<GroundedVerification> {
    let direction = direction.unwrap_or("tự nhiên");
    let prompt = format!(
        "Kiểm tra comment ứng viên trên contact sheet TikTok. Comment chính xác là: {candidate:?}.\n\
         Định hướng giọng điệu là: {direction:?}.\n\
         Đọc lại trực tiếp các frame, không tin facts từ lượt trước. Trả về JSON duy nhất: {{\"relevance\":0..100,\"evidenceSupport\":0..100,\"instructionFit\":0..100,\"genericity\":0..100,\"contradiction\":boolean,\"unsupportedClaim\":boolean,\"uiTextConfusion\":boolean}}.\n\
         relevance đo comment có nói đúng bài này không; evidenceSupport đo mọi chi tiết cụ thể có nhìn thấy không; genericity cao nếu chỉ là lời khen rỗng. instructionFit phải thấp nếu câu nghe như báo cáo, tóm tắt hoặc quá trang trọng thay vì phản ứng đời thường. Caption, hình và hướng dẫn mâu thuẫn thì đánh cờ contradiction."
    );
    let body = vision_body(settings, sheet, prompt, 0.0, 300);
    let (raw, p, c, model) = chat(settings, body).await?;
    let value = json_object(&raw).ok_or_else(|| anyhow!("malformed_model_output"))?;
    Ok(GroundedVerification {
        relevance: score(value.get("relevance"))?,
        evidence_support: score(value.get("evidenceSupport"))?,
        instruction_fit: score(value.get("instructionFit"))?,
        genericity: score(value.get("genericity"))?,
        contradiction: value
            .get("contradiction")
            .and_then(|v| v.as_bool())
            .unwrap_or(true),
        unsupported_claim: value
            .get("unsupportedClaim")
            .and_then(|v| v.as_bool())
            .unwrap_or(true),
        ui_text_confusion: value
            .get("uiTextConfusion")
            .and_then(|v| v.as_bool())
            .unwrap_or(true),
        prompt_tokens: p,
        completion_tokens: c,
        model,
    })
}

fn vision_body(
    settings: &NurtureSettings,
    sheet: &[u8],
    prompt: String,
    temperature: f64,
    max_tokens: u32,
) -> serde_json::Value {
    let b64 = B64.encode(sheet);
    json!({
        "model": settings.model,
        "temperature": temperature,
        "max_tokens": max_tokens,
        "stream": false,
        "messages": [{
            "role": "user",
            "content": [
                { "type": "image_url", "image_url": { "url": format!("data:image/jpeg;base64,{b64}") } },
                { "type": "text", "text": prompt }
            ]
        }]
    })
}

fn text_body(
    settings: &NurtureSettings,
    prompt: String,
    temperature: f64,
    max_tokens: u32,
) -> serde_json::Value {
    json!({
        "model": settings.model,
        "temperature": temperature,
        "max_tokens": max_tokens,
        "stream": false,
        "thinking": { "type": "disabled" },
        "response_format": { "type": "json_object" },
        "messages": [{
            "role": "user",
            "content": prompt
        }]
    })
}

fn json_object(raw: &str) -> Option<serde_json::Value> {
    let start = raw.find('{')?;
    let end = raw.rfind('}')?;
    serde_json::from_str(&raw[start..=end]).ok()
}

fn score(value: Option<&serde_json::Value>) -> anyhow::Result<u8> {
    let number = value
        .and_then(|v| v.as_f64())
        .ok_or_else(|| anyhow!("malformed_model_output"))?;
    if !number.is_finite() || !(0.0..=100.0).contains(&number) {
        return Err(anyhow!("malformed_model_output"));
    }
    let normalized = if number <= 1.0 {
        number * 100.0
    } else {
        number
    };
    Ok(normalized.round() as u8)
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

fn language_label(raw: &str) -> String {
    match raw.trim().to_ascii_lowercase().as_str() {
        "vi" | "vietnamese" | "tiếng việt" => "Việt".into(),
        "en" | "english" => "English".into(),
        "" => "Việt".into(),
        other => other.to_string(),
    }
}

fn make_contact_sheet(frames: &[Vec<u8>]) -> anyhow::Result<Vec<u8>> {
    let mut decoded = frames
        .iter()
        .take(3)
        .map(|bytes| image::load_from_memory(bytes).map(|image| image.to_rgb8()))
        .collect::<Result<Vec<_>, _>>()
        .context("decode comment frames")?;
    if decoded.is_empty() {
        return Err(anyhow!("no_usable_evidence"));
    }
    while decoded.len() < 3 {
        decoded.push(decoded.last().cloned().unwrap());
    }

    let mut sheet = RgbImage::from_pixel(750, 1334, Rgb([0, 0, 0]));
    for (index, frame) in decoded.iter().enumerate() {
        let thumb = image::imageops::resize(frame, 375, 667, FilterType::Lanczos3);
        let (x, y) = match index {
            0 => (0, 0),
            1 => (375, 0),
            _ => (0, 667),
        };
        sheet
            .copy_from(&thumb, x, y)
            .map_err(|_| anyhow!("compose comment frames"))?;
    }
    let source = decoded.last().unwrap();
    let crop_y = ((source.height() as f32) * 0.58) as u32;
    let crop_h = ((source.height() as f32) * 0.34) as u32;
    let crop_w = ((source.width() as f32) * 0.84) as u32;
    let crop = image::imageops::crop_imm(
        source,
        0,
        crop_y.min(source.height().saturating_sub(1)),
        crop_w.max(1).min(source.width()),
        crop_h.max(1).min(
            source
                .height()
                .saturating_sub(crop_y.min(source.height().saturating_sub(1))),
        ),
    )
    .to_image();
    let crop = image::imageops::resize(&crop, 375, 260, FilterType::Lanczos3);
    sheet
        .copy_from(&crop, 375, 870)
        .map_err(|_| anyhow!("compose caption crop"))?;

    let mut encoded = Cursor::new(Vec::new());
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut encoded, 85)
        .encode_image(&image::DynamicImage::ImageRgb8(sheet))
        .context("encode comment contact sheet")?;
    Ok(encoded.into_inner())
}

/// Legacy generic-comment helper kept for offline fixtures. The nurture engine
/// does not call it for production text comments because an ungrounded sentence
/// must never be posted under an account.
pub async fn generate_comment_pool(settings: &NurtureSettings, count: usize) -> (Vec<String>, f64) {
    let count = count.clamp(5, 60);
    let max_words = settings.max_comment_words.max(4) as usize;
    let directions: Vec<&str> = settings
        .ai_directions
        .split('|')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();

    let body = json!({
        "model": settings.model,
        "temperature": 1.0,
        "max_tokens": 1200,
        "stream": false,
        "messages": [{ "role": "user", "content": pool_prompt(count, max_words, &directions) }]
    });

    match chat(settings, body).await {
        Ok((raw, p, c, _)) => {
            let mut pool: Vec<String> = raw
                .lines()
                .filter_map(|line| sanitize_comment(line, max_words))
                .collect();
            pool.dedup();
            if pool.len() < 5 {
                (builtin_pool(), estimate_usd(settings, p, c))
            } else {
                (pool, estimate_usd(settings, p, c))
            }
        }
        Err(_) => (builtin_pool(), 0.0),
    }
}

/// A reaction the engine can actually post on this stack.
///
/// Stock WebDriverAgent gets a successful key ACK that TikTok ignores. Its
/// fallback uses this menu to pick a reaction; RT-MMO sessions take the text
/// path instead. Each entry is a position in the panel's grid, located from the
/// current frame rather than assumed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EmojiReaction {
    /// Row in the emoji grid, 0-based from the top of the full grid.
    pub row: usize,
    /// Column, 0-based from the left.
    pub col: usize,
    /// What it means, for the log.
    pub label: &'static str,
}

/// The menu the model chooses from. Positions measured on the live panel:
/// row 0 = 😀😄😁😆😅😂🤣, row 1 = ☺️😊😇🙂🙃😉😌, row 2 = 😍😋😛😝😜🤪🧐.
pub const EMOJI_MENU: [EmojiReaction; 6] = [
    EmojiReaction {
        row: 0,
        col: 5,
        label: "😂 buồn cười",
    },
    EmojiReaction {
        row: 2,
        col: 0,
        label: "😍 thích/đẹp",
    },
    EmojiReaction {
        row: 0,
        col: 1,
        label: "😄 vui",
    },
    EmojiReaction {
        row: 1,
        col: 1,
        label: "😊 dễ thương",
    },
    EmojiReaction {
        row: 2,
        col: 4,
        label: "😜 lầy/nghịch",
    },
    EmojiReaction {
        row: 1,
        col: 6,
        label: "😌 chill",
    },
];

/// Ask the model which reaction suits this video. Falls back to a random pick
/// so a failed call never blocks a session.
pub async fn choose_emoji_reaction(
    settings: &NurtureSettings,
    jpeg_bytes: &[u8],
) -> (EmojiReaction, f64) {
    let menu = EMOJI_MENU
        .iter()
        .enumerate()
        .map(|(i, e)| format!("{}. {}", i + 1, e.label))
        .collect::<Vec<_>>()
        .join("\n");
    let prompt = format!(
        "Bạn đang lướt TikTok. Nhìn ảnh và chọn ĐÚNG MỘT cảm xúc hợp với video:\n\n         {menu}\n\n         CHỈ trả về một chữ số từ 1 đến {}. Không giải thích.",
        EMOJI_MENU.len()
    );
    let b64 = B64.encode(jpeg_bytes);
    let body = json!({
        "model": settings.model,
        "temperature": 0.4,
        "max_tokens": 8,
        "stream": false,
        "messages": [{
            "role": "user",
            "content": [
                { "type": "image_url",
                  "image_url": { "url": format!("data:image/jpeg;base64,{b64}") } },
                { "type": "text", "text": prompt }
            ]
        }]
    });

    let fallback = || {
        let mut rng = rand::thread_rng();
        *EMOJI_MENU.choose(&mut rng).unwrap_or(&EMOJI_MENU[0])
    };
    match chat(settings, body).await {
        Ok((raw, p, c, _)) => {
            let pick = raw
                .chars()
                .find(|c| c.is_ascii_digit())
                .and_then(|c| c.to_digit(10))
                .map(|d| d as usize)
                .filter(|d| *d >= 1 && *d <= EMOJI_MENU.len())
                .map(|d| EMOJI_MENU[d - 1])
                .unwrap_or_else(fallback);
            (pick, estimate_usd(settings, p, c))
        }
        Err(_) => (fallback(), 0.0),
    }
}

/// Pick one comment from a pool at random.
pub fn pick_from_pool(pool: &[String]) -> Option<String> {
    let mut rng = rand::thread_rng();
    pool.choose(&mut rng).cloned()
}

fn vision_prompt(lang: &str, max_words: usize, direction: Option<&str>) -> String {
    let direction_block = match direction.map(str::trim).filter(|d| !d.is_empty()) {
        Some(d) => format!(
            "\nĐỊNH HƯỚNG GIỌNG ĐIỆU (chỉ áp dụng khi tương thích với bằng chứng): \"{d}\".\n\
             Nội dung nhìn thấy và caption luôn có ưu tiên cao hơn định hướng. Ý mời mua, hỏi giá/link, \
             tăng tương tác hoặc cảm xúc chỉ được dùng khi không thêm chi tiết chưa xuất hiện trong frame.\n"
        ),
        None => String::new(),
    };
    format!(
        "Bạn là người Việt Nam Gen Z đang lướt TikTok thật, vừa xem xong video này.\n\
         {direction_block}\n\
         Nhìn ảnh chụp màn hình và viết đúng 1 comment bằng tiếng {lang}, tối đa {max_words} từ. Viết như phản ứng đời thường ngay sau khi xem: ưu tiên 2-10 từ, thân mật, ngắn và có cảm xúc vừa phải; không viết kiểu tóm tắt hay báo cáo.\n\n\
         QUY TẮC:\n\
         - Phản hồi đúng nội dung video, không khen chung chung.\n\
         - Bỏ qua chữ trên ảnh mà là tên người đăng, nút bấm UI hay tên bài nhạc.\n\
         - KHÔNG dùng: \"nội dung hay\", \"chất lượng\", \"tuyệt vời\", \"cảm ơn đã chia sẻ\".\n\
         - KHÔNG giải thích, KHÔNG đặt trong ngoặc kép, CHỈ trả về đúng 1 dòng comment."
    )
}

fn pool_prompt(count: usize, max_words: usize, directions: &[&str]) -> String {
    let direction_block = if directions.is_empty() {
        String::new()
    } else {
        format!(
            "Mỗi comment phải theo đúng 1 trong các định hướng sau, phân bố đều, \
             không lặp cùng định hướng quá 3 lần liên tiếp:\n{}\n\n",
            directions
                .iter()
                .map(|d| format!("- {d}"))
                .collect::<Vec<_>>()
                .join("\n")
        )
    };
    format!(
        "Bạn là người Việt Nam Gen Z đang lướt TikTok thật. Viết comment như người thật, không phải bot.\n\n\
         Tạo đúng {count} comment tiếng Việt.\n\n\
         {direction_block}\
         Trộn đều các kiểu: phản ứng cảm xúc rất ngắn (1–3 từ); câu hỏi tự nhiên \
         (nhạc gì vậy / quay ở đâu / mua ở đâu); khen cụ thể chứ không sáo rỗng; \
         bình luận hài hước.\n\n\
         YÊU CẦU BẮT BUỘC:\n\
         - Chỉ trả về đúng {count} dòng, mỗi dòng 1 comment.\n\
         - KHÔNG đánh số, KHÔNG gạch đầu dòng, KHÔNG giải thích.\n\
         - KHÔNG dùng: \"nội dung hay\", \"chất lượng\", \"tuyệt vời\", \"cảm ơn đã chia sẻ\".\n\
         - Khoảng một nửa có emoji, một nửa không.\n\
         - Độ dài đa dạng, không quá {max_words} từ."
    )
}

/// Turn raw model output into something safe to type, or `None` if it cannot be
/// salvaged. Rejecting is the right answer when in doubt: a skipped comment
/// costs nothing, a garbage one is posted under the user's account.
fn sanitize_comment(raw: &str, max_words: usize) -> Option<String> {
    // Reasoning models occasionally leak a <think> block into the content.
    let mut text = raw.to_string();
    while let (Some(open), Some(close)) = (
        text.to_lowercase().find("<think>"),
        text.to_lowercase().find("</think>"),
    ) {
        if close < open {
            break;
        }
        text.replace_range(open..close + "</think>".len(), "");
    }

    let line = text
        .lines()
        .map(|l| {
            l.trim()
                .trim_start_matches(|c: char| c.is_ascii_digit() || c == '.' || c == ')')
                .trim_start_matches(['-', '*', '•'])
                .trim()
                .trim_matches(['"', '\'', '`', '“', '”'])
                .trim()
        })
        .find(|l| !l.is_empty())?;

    if line.is_empty() {
        return None;
    }
    let words: Vec<&str> = line.split_whitespace().collect();
    // Wildly long output means the model answered rather than commented.
    if words.len() > MAX_SANE_WORDS {
        return None;
    }
    let capped = words
        .into_iter()
        .take(max_words)
        .collect::<Vec<_>>()
        .join(" ");
    if capped.is_empty() {
        None
    } else {
        Some(capped)
    }
}

/// Model outputs sometimes satisfy the evidence check while sounding like a
/// report. Those sentences are technically relevant but feel wrong in a real
/// TikTok thread, so give the grounded pass one retry with a stronger casual
/// instruction.
fn sounds_like_report(text: &str) -> bool {
    let lower = text.to_lowercase();
    [
        "được trình bày",
        "mang đến",
        "người xem",
        "chất lượng",
        "truyền tải",
        "cung cấp",
        "nội dung về",
        "nội dung được",
    ]
    .iter()
    .any(|marker| lower.contains(marker))
}

/// Offline fallback so a session can still comment with no API at all.
fn builtin_pool() -> Vec<String> {
    [
        "Hay quá 🔥",
        "đỉnh thật",
        "xem lại lần 2 rồi 😭",
        "ôi trời ơi",
        "chill quá",
        "ủa thật không",
        "bao giờ ra phần 2",
        "lưu lại xem sau",
        "relate quá đi",
        "vibe quá",
        "ghim lại xem sau",
        "làm theo ngay thôi",
        "xem mãi không chán",
        "đúng quá bạn ơi",
        "clip này hay ghê",
        "mình cũng vậy 😭",
        "thích cái này",
        "không thể tin nổi",
        "đỉnh vậy 👏",
        "nhạc gì vậy bạn?",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

#[cfg(test)]
mod tests {
    use image::GenericImageView;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    use super::*;

    #[test]
    fn strips_quotes_bullets_and_numbering() {
        assert_eq!(sanitize_comment("\"hay quá\"", 12).unwrap(), "hay quá");
        assert_eq!(sanitize_comment("1. hay quá", 12).unwrap(), "hay quá");
        assert_eq!(sanitize_comment("- hay quá", 12).unwrap(), "hay quá");
        assert_eq!(sanitize_comment("  • hay quá  ", 12).unwrap(), "hay quá");
    }

    #[test]
    fn drops_reasoning_blocks() {
        let raw = "<think>the user wants a comment</think>\nhay quá";
        assert_eq!(sanitize_comment(raw, 12).unwrap(), "hay quá");
    }

    #[test]
    fn takes_the_first_non_empty_line_only() {
        assert_eq!(
            sanitize_comment("\n\n  \nhay quá\nthừa dòng sau", 12).unwrap(),
            "hay quá"
        );
    }

    /// A model that answers the prompt instead of commenting must be rejected,
    /// not truncated into something that looks like a comment.
    #[test]
    fn rejects_output_that_is_far_too_long_to_be_a_comment() {
        let essay = "từ ".repeat(40);
        assert!(sanitize_comment(&essay, 12).is_none());
    }

    #[test]
    fn caps_at_the_configured_word_count() {
        let out = sanitize_comment("một hai ba bốn năm sáu", 3).unwrap();
        assert_eq!(out, "một hai ba");
    }

    #[test]
    fn rejects_empty_and_whitespace_only_output() {
        assert!(sanitize_comment("", 12).is_none());
        assert!(sanitize_comment("   \n\t ", 12).is_none());
        assert!(sanitize_comment("\"\"", 12).is_none());
    }

    #[test]
    fn the_builtin_pool_is_usable_without_any_api() {
        let pool = builtin_pool();
        assert!(pool.len() >= 10);
        assert!(pick_from_pool(&pool).is_some());
        assert!(pool.iter().all(|c| !c.trim().is_empty()));
        assert!(pool.iter().all(|c| c.split_whitespace().count() <= 12));
    }

    #[test]
    fn the_vision_prompt_carries_the_direction_when_given() {
        let with = vision_prompt("Việt", 12, Some("bán hàng"));
        assert!(with.contains("bán hàng"));
        assert!(with.contains("tối đa 12 từ"));
        assert!(with.contains("Nội dung nhìn thấy và caption luôn có ưu tiên cao hơn"));
        assert!(with.contains("phản ứng đời thường"));
        let without = vision_prompt("Việt", 12, Some("   "));
        assert!(!without.contains("ĐỊNH HƯỚNG"));
    }

    #[test]
    fn formal_comment_style_is_retried_or_rejected() {
        assert!(sounds_like_report(
            "Nội dung về IELTS được trình bày rõ ràng quá ạ"
        ));
        assert!(sounds_like_report(
            "Chất lượng hình ảnh mang đến trải nghiệm tốt"
        ));
        assert!(!sounds_like_report("Ủa đoạn này cuốn quá trời 😭"));
        let accepted = VerificationGate {
            overall: 90,
            instruction_fit: 90,
            genericity: 10,
            contradiction: false,
            unsupported_claim: false,
            ui_text_confusion: false,
            formal_style: false,
        };
        assert!(!VerificationGate {
            formal_style: true,
            ..accepted
        }
        .accepts());
        assert!(VerificationGate {
            formal_style: true,
            ..accepted
        }
        .retryable());
    }

    #[test]
    fn the_pool_prompt_lists_every_direction() {
        let p = pool_prompt(30, 12, &["Gen z", "Tự nhiên"]);
        assert!(p.contains("- Gen z"));
        assert!(p.contains("- Tự nhiên"));
        assert!(p.contains("đúng 30 comment"));
    }

    #[test]
    fn host_of_extracts_the_api_host() {
        assert_eq!(host_of("https://api.vilao.ai/v1"), "api.vilao.ai");
        assert_eq!(host_of("https://api.openai.com/v1/"), "api.openai.com");
    }

    #[test]
    fn language_codes_are_rendered_as_prompt_language_names() {
        assert_eq!(language_label("vi"), "Việt");
        assert_eq!(language_label("ENGLISH"), "English");
        assert_eq!(language_label("  "), "Việt");
        assert_eq!(language_label("日本語"), "日本語");
    }

    #[test]
    fn ocr_caption_drops_photo_metadata_but_keeps_visible_caption_lines() {
        let observations = vec![
            CommentOcrObservation {
                text: "Đà Lạt Hotel • Phường 4".into(),
                confidence: 1.0,
                x: 0.1,
                y: 0.76,
                width: 0.4,
                height: 0.02,
            },
            CommentOcrObservation {
                text: "Địa điểm này có 19.7M lượt thích".into(),
                confidence: 1.0,
                x: 0.1,
                y: 0.78,
                width: 0.5,
                height: 0.02,
            },
            CommentOcrObservation {
                text: "Đà Lạt đi để trở về".into(),
                confidence: 1.0,
                x: 0.1,
                y: 0.83,
                width: 0.35,
                height: 0.02,
            },
            CommentOcrObservation {
                text: "• Ảnh".into(),
                confidence: 1.0,
                x: 0.4,
                y: 0.83,
                width: 0.1,
                height: 0.02,
            },
            CommentOcrObservation {
                text: "ĐÀ LẠT NHƯ ĐỊA NGỤC HAY THIÊN ĐƯỜNG?".into(),
                confidence: 1.0,
                x: 0.1,
                y: 0.86,
                width: 0.65,
                height: 0.03,
            },
        ];

        assert_eq!(
            ocr_caption(&observations).as_deref(),
            Some(
                "Đà Lạt Hotel • Phường 4 Đà Lạt đi để trở về ĐÀ LẠT NHƯ ĐỊA NGỤC HAY THIÊN ĐƯỜNG?"
            )
        );
    }

    #[test]
    fn grounded_score_accepts_fraction_or_percent_and_rejects_invalid_values() {
        assert_eq!(score(Some(&json!(0.8))).unwrap(), 80);
        assert_eq!(score(Some(&json!(80))).unwrap(), 80);
        assert!(score(Some(&json!(101))).is_err());
        assert!(score(Some(&json!("80"))).is_err());
    }

    #[test]
    fn grounded_gate_has_strict_boundaries_and_hard_flags_win() {
        let accepted = VerificationGate {
            overall: 80,
            instruction_fit: 70,
            genericity: 30,
            contradiction: false,
            unsupported_claim: false,
            ui_text_confusion: false,
            formal_style: false,
        };
        assert!(accepted.accepts());
        assert!(accepted.accepts_caption(70));
        assert!(!accepted.accepts_caption(69));
        assert!(VerificationGate {
            overall: 60,
            genericity: 35,
            ..accepted
        }
        .accepts_caption(70));
        assert!(!VerificationGate {
            overall: 59,
            ..accepted
        }
        .accepts_caption(70));
        assert!(!VerificationGate {
            genericity: 36,
            ..accepted
        }
        .accepts_caption(70));
        assert!(!VerificationGate {
            overall: 79,
            ..accepted
        }
        .accepts());
        assert!(!VerificationGate {
            contradiction: true,
            ..accepted
        }
        .accepts());
        assert!(VerificationGate {
            overall: 60,
            ..accepted
        }
        .retryable());
        assert!(!VerificationGate {
            overall: 59,
            ..accepted
        }
        .retryable());
    }

    #[test]
    fn contact_sheet_is_a_stable_portrait_jpeg_for_one_or_three_frames() {
        let frame = include_bytes!("../tests/fixtures/feed-iphone8.jpg").to_vec();
        let sheet =
            make_contact_sheet(std::slice::from_ref(&frame)).expect("one frame contact sheet");
        let image = image::load_from_memory(&sheet).expect("decode contact sheet");
        assert_eq!(image.dimensions(), (750, 1334));
        let sheet_three = make_contact_sheet(&[frame.clone(), frame.clone(), frame]).unwrap();
        assert_eq!(
            image::load_from_memory(&sheet_three).unwrap().dimensions(),
            (750, 1334)
        );
    }

    #[test]
    fn json_object_ignores_a_fenced_wrapper_but_requires_an_object() {
        let parsed = json_object("```json\n{\"comment\":\"hoa dep\"}\n```").unwrap();
        assert_eq!(parsed["comment"], "hoa dep");
        assert!(json_object("not json").is_none());
    }

    #[test]
    fn deepseek_uses_text_only_body_for_caption_preview() {
        let settings = NurtureSettings {
            base_url: "https://api.deepseek.com".into(),
            model: "deepseek-v4-flash".into(),
            ..NurtureSettings::default()
        };
        assert!(!provider_supports_vision(&settings));
        let body = text_body(&settings, "trả JSON".into(), 0.0, 100);
        assert!(body["messages"][0]["content"].is_string());
        assert_eq!(body["thinking"]["type"], "disabled");
        assert_eq!(body["response_format"]["type"], "json_object");
    }

    #[tokio::test]
    async fn grounded_comment_uses_two_pass_local_gateway_and_returns_evidence() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("mock gateway bind");
        let address = listener.local_addr().expect("mock gateway address");
        let draft = serde_json::json!({
            "choices": [{"message": {"content": "{\"caption\":\"trà cherry Đà Lạt\",\"captionConfidence\":95,\"visualFacts\":[\"ly trà màu đỏ\"],\"contextConfidence\":92,\"comment\":\"Trà cherry nhìn mê quá 😋\"}"}}],
            "usage": {"prompt_tokens": 31, "completion_tokens": 12},
            "model": "mock-draft"
        });
        let verification = serde_json::json!({
            "choices": [{"message": {"content": "{\"relevance\":94,\"evidenceSupport\":91,\"instructionFit\":88,\"genericity\":12,\"contradiction\":false,\"unsupportedClaim\":false,\"uiTextConfusion\":false}"}}],
            "usage": {"prompt_tokens": 27, "completion_tokens": 9},
            "model": "mock-verifier"
        });
        let server = tokio::spawn(async move {
            for response in [draft, verification] {
                let (mut socket, _) = listener.accept().await.expect("mock request");
                let mut request = Vec::new();
                let mut chunk = [0u8; 8192];
                loop {
                    let count = socket.read(&mut chunk).await.expect("mock request body");
                    if count == 0 {
                        break;
                    }
                    request.extend_from_slice(&chunk[..count]);
                    let Some(header_end) = request.windows(4).position(|w| w == b"\r\n\r\n") else {
                        continue;
                    };
                    let content_length = std::str::from_utf8(&request[..header_end])
                        .ok()
                        .and_then(|headers| {
                            headers.lines().find_map(|line| {
                                let (name, value) = line.split_once(':')?;
                                name.eq_ignore_ascii_case("content-length")
                                    .then(|| value.trim().parse().ok())
                                    .flatten()
                            })
                        })
                        .unwrap_or(0usize);
                    if request.len() >= header_end + 4 + content_length {
                        break;
                    }
                }
                let body = response.to_string();
                let head = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                socket
                    .write_all(format!("{head}{body}").as_bytes())
                    .await
                    .expect("mock response");
            }
        });

        let settings = NurtureSettings {
            api_key: "test-key".into(),
            base_url: format!("http://{address}/v1"),
            ..NurtureSettings::default()
        };
        let frame = include_bytes!("../tests/fixtures/feed-iphone8.jpg").to_vec();
        let result = prepare_grounded_comment(&settings, &[frame], Some("tự nhiên"))
            .await
            .expect("grounded comment");
        server.await.expect("mock gateway task");

        assert_eq!(result.text, "Trà cherry nhìn mê quá 😋");
        assert_eq!(result.caption.as_deref(), Some("trà cherry Đà Lạt"));
        assert_eq!(result.context_confidence, 92);
        assert_eq!(result.relevance, 94);
        assert_eq!(result.evidence_support, 91);
        assert_eq!(result.prompt_tokens, 58);
        assert_eq!(result.completion_tokens, 21);
        assert_eq!(result.model, "mock-verifier");
        assert_eq!(result.frame_sha256.len(), 64);
        assert!(result.usd > 0.0);
    }
}
