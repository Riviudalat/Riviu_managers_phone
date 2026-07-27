//! OpenAI-compatible chat client used for TikTok comments.
//!
//! Two jobs, one provider (Vilao AI by default — an OpenAI-compatible gateway,
//! so any other compatible endpoint works by changing `base_url`):
//!
//! * [`generate_vision_comment`] — one comment for the video on screen, from a
//!   frame. Best quality, but it costs a round trip per comment and can fail.
//! * [`generate_comment_pool`] — a batch of generic comments generated once at
//!   session start. This is the fallback that keeps a session running when the
//!   API is slow, rate-limited or unreachable, so a network problem never
//!   leaves the phone sitting in an open comment box.
//!
//! Anything the model returns is treated as untrusted text: it is stripped of
//! reasoning blocks and quoting, collapsed to one line and word-capped before
//! it can be typed into someone's comment box.

use anyhow::{anyhow, Context};
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use rand::seq::SliceRandom;
use serde::Deserialize;
use serde_json::json;

use crate::types::NurtureSettings;

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
    let lang = if settings.comment_lang.trim().is_empty() {
        "Việt"
    } else {
        settings.comment_lang.trim()
    };
    let prompt = vision_prompt(lang, max_words, direction);

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

/// A batch of generic comments, generated once per session as the fallback for
/// when the vision call fails. Returns the built-in pool if the API is
/// unavailable, so this never fails the session.
pub async fn generate_comment_pool(
    settings: &NurtureSettings,
    count: usize,
) -> (Vec<String>, f64) {
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
/// TikTok's comment box refuses every synthesized keystroke (see AGENTS.md), so
/// text cannot be typed with a stock WebDriverAgent. The emoji panel *is*
/// reachable, so the model picks a reaction that fits the video instead and the
/// engine taps that cell. Each entry is a position in the panel's grid, which
/// is located per frame rather than assumed.
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
    EmojiReaction { row: 0, col: 5, label: "😂 buồn cười" },
    EmojiReaction { row: 2, col: 0, label: "😍 thích/đẹp" },
    EmojiReaction { row: 0, col: 1, label: "😄 vui" },
    EmojiReaction { row: 1, col: 1, label: "😊 dễ thương" },
    EmojiReaction { row: 2, col: 4, label: "😜 lầy/nghịch" },
    EmojiReaction { row: 1, col: 6, label: "😌 chill" },
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
            "\nĐỊNH HƯỚNG (bắt buộc, ưu tiên cao hơn việc bám nội dung video nếu xung đột): \"{d}\".\n\
             Áp dụng đúng tinh thần của định hướng — ý mời mua thì khéo hỏi giá/link; ý tăng tương tác \
             thì đặt câu hỏi hoặc rủ tag bạn bè; ý cảm xúc (tích cực/hoài niệm/hài hước…) thì chọn từ ngữ \
             mang đúng cảm xúc đó. Luôn lồng được định hướng vào comment.\n"
        ),
        None => String::new(),
    };
    format!(
        "Bạn là người Việt Nam Gen Z đang lướt TikTok thật, vừa xem xong video này.\n\
         {direction_block}\n\
         Nhìn ảnh chụp màn hình và viết đúng 1 comment bằng tiếng {lang}, tối đa {max_words} từ.\n\n\
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

/// Offline fallback so a session can still comment with no API at all.
fn builtin_pool() -> Vec<String> {
    [
        "Hay quá 🔥", "đỉnh thật", "xem lại lần 2 rồi 😭", "ôi trời ơi",
        "chill quá", "ủa thật không", "bao giờ ra phần 2", "lưu lại xem sau",
        "relate quá đi", "vibe quá", "ghim lại xem sau", "làm theo ngay thôi",
        "xem mãi không chán", "đúng quá bạn ơi", "clip này hay ghê",
        "mình cũng vậy 😭", "thích cái này", "không thể tin nổi", "đỉnh vậy 👏",
        "nhạc gì vậy bạn?",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

#[cfg(test)]
mod tests {
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
        let without = vision_prompt("Việt", 12, Some("   "));
        assert!(!without.contains("ĐỊNH HƯỚNG"));
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
}
