import type { NurtureSettings } from "./types";

export interface NurtureSettingsIssue {
  field: keyof NurtureSettings;
  tab: "behaviour" | "ai";
  message: string;
}

/** Shared by readiness, profile saving and the final save-before-start check. */
export function validateNurtureSettings(s: NurtureSettings): NurtureSettingsIssue | null {
  if (!Number.isInteger(s.maxCommentWords) || s.maxCommentWords < 4 || s.maxCommentWords > 30) {
    return { field: "maxCommentWords", tab: "ai", message: "Giới hạn comment phải từ 4 đến 30 từ" };
  }
  if (!Number.isInteger(s.numVideos) || s.numVideos < 1 || s.numVideos > 10_000) {
    return { field: "numVideos", tab: "behaviour", message: "Giới hạn video phải từ 1 đến 10000" };
  }
  if (!Number.isInteger(s.numRounds) || s.numRounds < 1 || s.numRounds > 100) {
    return { field: "numRounds", tab: "behaviour", message: "Số vòng phải từ 1 đến 100" };
  }
  if (!Number.isFinite(s.watchMin) || s.watchMin <= 0 || s.watchMin > 120) {
    return { field: "watchMin", tab: "behaviour", message: "Thời gian xem tối thiểu phải lớn hơn 0 và không quá 120 giây" };
  }
  if (!Number.isFinite(s.watchMax) || s.watchMax < s.watchMin || s.watchMax > 120) {
    return { field: "watchMax", tab: "behaviour", message: "Thời gian xem tối đa phải từ mức tối thiểu đến 120 giây" };
  }
  if (!Number.isInteger(s.scheduleEveryMinutes) || s.scheduleEveryMinutes < 15 || s.scheduleEveryMinutes > 1440) {
    return { field: "scheduleEveryMinutes", tab: "behaviour", message: "Lịch phải cách nhau 15–1440 phút" };
  }
  if (!Number.isInteger(s.scheduleDurationMinutes) || s.scheduleDurationMinutes < 15 || s.scheduleDurationMinutes > 360) {
    return { field: "scheduleDurationMinutes", tab: "behaviour", message: "Thời lượng phiên phải từ 15 đến 360 phút" };
  }
  // A stored-key sentinel is nonempty; an explicitly cleared key must still block comments.
  if ((s.commentEnabled ?? true) && s.commentProb > 0 && !s.apiKey.trim()) {
    return { field: "apiKey", tab: "ai", message: "Đã bật bình luận: điền API key trong Cấu hình AI" };
  }
  return null;
}

export function nurtureFieldValidation(
  field: keyof NurtureSettings,
  issue: NurtureSettingsIssue | null | undefined,
  issueId: string | undefined,
) {
  const invalid = issue?.field === field;
  return {
    "data-nurture-field": field,
    "aria-invalid": invalid || undefined,
    "aria-describedby": invalid ? issueId : undefined,
  };
}
