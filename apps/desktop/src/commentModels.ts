/**
 * Endpoints and models this project has actually put a TikTok contact sheet through.
 *
 * **A suggestion list, never a whitelist.** The comment path speaks plain
 * OpenAI-compatible `chat/completions`, so any gateway and any model string works and the
 * fields stay free text. What this adds is the one thing a free-text field cannot: the
 * measurement behind each entry, so choosing is not guesswork.
 *
 * The hard-won lesson behind the shape of this file: the vision capability used to be a
 * single hardcoded line — `host !== "api.deepseek.com"` — and it went stale silently. It is
 * detected at runtime now (`openai_client::provider_supports_vision`), so nothing here gates
 * anything. These are notes for a human, not a switch.
 */

export interface CommentModelSuggestion {
  /** The value for the Model field, verbatim. */
  model: string;
  /** The base URL it was measured against. */
  baseUrl: string;
  /** What was measured, in the operator's language. */
  note: string;
}

/**
 * Measured on this fleet, newest first.
 *
 * Every note is a number somebody read off a real run — no marketing claims, and no model
 * anybody merely expects to work.
 */
export const COMMENT_MODEL_SUGGESTIONS: readonly CommentModelSuggestion[] = [
  {
    model: "deepseek-v4-flash-vision-exp",
    baseUrl: "https://api.deepseek.com",
    note: "23/08/2026, tấm ghép 3 khung thật: 4/4 lần ra JSON dùng được, 475 token vào / 135 ra, p50 2,1s. Nhanh và rẻ token nhất trong ba cái ở đây. Là bản thực nghiệm (exp) nên có thể đổi hoặc biến mất.",
  },
  {
    model: "openai/gpt-5.6-luna",
    baseUrl: "https://openrouter.ai/api/v1",
    note: "Đang chạy trên farm. 19/08/2026, cả fleet 20 máy với bình luận thật: 14 comment gửi, 7 bị gate chặn. max_tokens phải là 1200 — ở 500 thì 2/5 bài không ra gì vì JSON bị cắt giữa chuỗi (đo trên 6 máy, 19/08/2026).",
  },
  {
    model: "deepseek-v4-flash",
    baseUrl: "https://api.deepseek.com",
    note: "KHÔNG đọc được ảnh: endpoint nhận image part nhưng model từ chối (`This model does not support image`), 23/08/2026. Chỉ dùng nếu chấp nhận đường OCR caption — mà Windows không có bộ OCR tiếng Việt, nên caption sẽ sai chữ.",
  },
];
