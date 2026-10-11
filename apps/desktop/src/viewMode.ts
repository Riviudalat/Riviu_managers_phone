/**
 * The three wall views (PLAN §2): Tổng quan (short tiles, the default), Màn hình (the large
 * preview) and Danh sách (the table). The choice is remembered per user; the selection is the
 * same set of udids in every view.
 */
export type ViewMode = "overview" | "window" | "list";

const VIEW_MODE_KEY = "riviu.control.viewMode";

export function loadViewMode(): ViewMode {
  try {
    const raw = window.localStorage.getItem(VIEW_MODE_KEY);
    return raw === "window" || raw === "list" ? raw : "overview";
  } catch {
    return "overview";
  }
}

export function storeViewMode(mode: ViewMode): void {
  try {
    window.localStorage.setItem(VIEW_MODE_KEY, mode);
  } catch {
    // Remembering the view is a convenience; losing it must not break switching.
  }
}
