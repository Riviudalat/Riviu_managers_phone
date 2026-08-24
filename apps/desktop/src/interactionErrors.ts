/**
 * Vietnamese for the codes an interaction campaign records against itself.
 *
 * The Monitor tab used to print these raw, so the most diagnostic string on screen was also
 * the least readable one: `parent_identity_not_confirmed_at_ordinal_3` and
 * `ai_comment_unavailable: ordinal 0 — comment_context_rejected: context=0 overall=0` sat in
 * a panel whose every other label was Vietnamese. There was a translation table for the
 * eleven *states* and none for the forty-odd *reasons*.
 *
 * Three things this deliberately does not do:
 *
 * - **It never throws the raw code away.** Every result carries it, and the UI keeps it
 *   behind a disclosure. The codes are what a bug report is written from, and the operator
 *   is not the only reader of this panel.
 * - **It never invents a translation for something it does not recognise.** The
 *   campaign-level `errorCode` is a free-text anyhow chain — much of it already Vietnamese,
 *   written by the engine — so an unknown string is passed through as the title rather than
 *   replaced by a guess.
 * - **It does not re-translate the detail.** The hierarchy refusals are stored by the engine
 *   as `code: câu tiếng Việt`, and that sentence is better than anything this file could
 *   write: it was authored next to the measurement it describes. The code becomes the title,
 *   the engine's sentence becomes the detail.
 */

export interface InteractionErrorView {
  /** Short Vietnamese headline for the failure. */
  title: string;
  /** Whatever the backend appended after the code, when it appended anything. */
  detail?: string;
  /** The original string, always, for diagnosis. */
  raw: string;
}

/**
 * Arrival and reply refusals from the hierarchy driver.
 *
 * These are the ones stored as `code: message`, so the map only has to name the code — see
 * the note about not re-translating the detail.
 */
const EXACT: Record<string, string> = {
  // Arrival: did the phone reach the post we meant?
  target_open_wrong_app: "Link mở sang app khác",
  target_open_no_post_page: "Không thấy trang bài viết",
  target_open_screen_unchanged: "Mở link xong màn hình không đổi",
  target_open_no_measured_label: "Bản TikTok này chưa được đo nhãn",
  target_open_no_baseline: "Không đọc được bài đang mở trước đó",
  target_open_cancelled: "Đã dừng khi đang mở bài",
  // Reply: did we find the right comment to answer?
  reply_control_unmeasured: "Chưa đo nút Trả lời trên bản này",
  reply_no_drawer: "Không mở được khay bình luận",
  reply_parent_not_found: "Không tìm thấy bình luận cha",
  reply_drawer_closed_by_scroll: "Cuộn làm đóng mất khay bình luận",
  reply_no_composer: "Không thấy ô nhập trả lời",
  reply_wrong_parent: "Bấm nhầm nút Trả lời của bình luận khác",
};

/** Families, so a refusal added to the engine later still reads as something. */
const FAMILIES: [string, string][] = [
  ["target_open_", "Không mở được bài"],
  ["reply_", "Không trả lời được"],
];

/**
 * Translate one recorded failure.
 *
 * The split is on the first `": "` because that is the shape the engine writes: a code, then
 * either a Vietnamese sentence (hierarchy refusals) or a nested cause chain (AI failures).
 */
export function interactionErrorVi(raw: string): InteractionErrorView {
  const trimmed = raw.trim();
  if (!trimmed) return { title: "Lỗi không rõ", raw };
  const at = trimmed.indexOf(": ");
  const head = at === -1 ? trimmed : trimmed.slice(0, at);
  const rest = at === -1 ? undefined : trimmed.slice(at + 2).trim() || undefined;

  const exact = EXACT[head];
  if (exact) return { title: exact, detail: rest, raw };

  // A skipped reply names the message it was waiting on. The ordinal is zero-based in the
  // code and one-based everywhere the operator reads it, which is why this is not a plain
  // string swap.
  const skipped = /^parent_identity_not_confirmed_at_ordinal_(\d+)$/.exec(head);
  if (skipped) {
    const parent = Number(skipped[1]) + 1;
    return {
      title: `Bỏ qua — bình luận thứ ${parent} chưa được xác nhận nên không có gì để trả lời`,
      detail: rest,
      raw,
    };
  }

  if (head === "ai_comment_unavailable") {
    return { title: "AI không viết được bình luận", detail: rest, raw };
  }
  if (head === "target_evidence_unavailable") {
    return { title: "Không chụp được bài cho AI đọc", detail: rest, raw };
  }

  for (const [prefix, title] of FAMILIES) {
    if (head.startsWith(prefix)) return { title, detail: rest, raw };
  }

  // Unknown: the campaign-level reason is free text and often already Vietnamese. Showing it
  // beats replacing it with "Lỗi".
  return { title: trimmed, raw };
}

/** Why one pasted line is not a usable link. */
const LINK_ERRORS: Record<string, string> = {
  empty: "Dòng trống",
  invalidUrl: "Không phải URL hợp lệ",
  unsupportedScheme: "Chỉ nhận link http/https",
  unsupportedHost: "Không phải link TikTok",
  userInfoNotAllowed: "Link có kèm thông tin đăng nhập",
  customPortNotAllowed: "Link dùng cổng lạ",
  unsupportedTargetKind: "Chỉ nhận link video hoặc ảnh",
  unresolvedShortLink: "Link rút gọn — bấm “Gỡ link rút gọn”",
};

export function linkErrorVi(code: string | null | undefined): string {
  if (!code) return "Không dùng được";
  return LINK_ERRORS[code] ?? code;
}

/** Campaign-level state. */
const CAMPAIGN_STATES: Record<string, string> = {
  queued: "Đang chờ",
  running: "Đang chạy",
  succeeded: "Hoàn tất",
  partial: "Xong một phần",
  failed: "Thất bại",
  cancelled: "Đã dừng",
};

export function campaignStateVi(state: string): string {
  return CAMPAIGN_STATES[state] ?? state;
}

/**
 * Per-message state.
 *
 * `uncertain` and `skippedParent` are spelled out rather than both reading "Chưa xác nhận":
 * one means the Send tap went out and its confirmation did not come back — so the comment may
 * be public and it will never be retried — and the other means nothing was typed at all.
 * Those need different reactions from the operator.
 */
const MESSAGE_STATES: Record<string, string> = {
  queued: "Đang chờ",
  preparing: "Đang soạn",
  ready: "Đã soạn",
  sending: "Đang gửi",
  succeeded: "Đã gửi",
  failed: "Lỗi",
  uncertain: "Đã gửi, chưa thấy lên",
  skippedParent: "Bỏ qua — thiếu bình luận cha",
};

export function assignmentStateVi(state: string): string {
  return MESSAGE_STATES[state] ?? state;
}

/** Chip colour for a state, in the tones the rest of the app uses. */
export function stateTone(state: string): "ok" | "warn" | "danger" | "info" {
  switch (state) {
    case "succeeded":
      return "ok";
    case "partial":
    case "uncertain":
    case "skippedParent":
      return "warn";
    // `failed` used to render in the neutral tone, so the one row the operator had to look
    // at was the one that looked like every other row.
    case "failed":
      return "danger";
    default:
      return "info";
  }
}
