import type { AutomationMode } from "./components/AutomationTabs";

type AutomationPage = "nurture" | "interaction";

let pending: { page: AutomationPage; mode: AutomationMode } | null = null;

/**
 * Asks the next mount of an automation page to open on `mode` instead of Thiết lập — how
 * "Tạo cấu hình để hẹn giờ" on Lịch chạy lands on Hẹn giờ in one action. One-shot: the
 * page clears it once mounted, and the caller clears it if the navigation was refused.
 */
export function requestAutomationMode(page: AutomationPage, mode: AutomationMode) {
  pending = { page, mode };
}

export function peekAutomationMode(page: AutomationPage): AutomationMode | null {
  return pending?.page === page ? pending.mode : null;
}

export function clearAutomationMode(page: AutomationPage) {
  if (pending?.page === page) pending = null;
}
