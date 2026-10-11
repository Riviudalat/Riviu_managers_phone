/**
 * The two wall views: Màn hình (GenFarmer's wall of live tiles, the default) and Danh sách (the
 * table). GF1 folded the old Tổng quan/Màn hình split into one wall, so a remembered
 * "overview" opens the wall. The choice is remembered per user; the selection is the same set
 * of udids in every view.
 */
export type ViewMode = "window" | "list";

const VIEW_MODE_KEY = "riviu.control.viewMode";
const PANEL_COLLAPSED_KEY = "riviu.control.panelCollapsed";

export function loadViewMode(): ViewMode {
  try {
    return window.localStorage.getItem(VIEW_MODE_KEY) === "list" ? "list" : "window";
  } catch {
    return "window";
  }
}

export function storeViewMode(mode: ViewMode): void {
  try {
    window.localStorage.setItem(VIEW_MODE_KEY, mode);
  } catch {
    // Remembering the view is a convenience; losing it must not break switching.
  }
}

/** Whether the left "Cài đặt" panel is folded («). Open unless the operator folded it. */
export function loadPanelCollapsed(): boolean {
  try {
    return window.localStorage.getItem(PANEL_COLLAPSED_KEY) === "1";
  } catch {
    return false;
  }
}

export function storePanelCollapsed(collapsed: boolean): void {
  try {
    window.localStorage.setItem(PANEL_COLLAPSED_KEY, collapsed ? "1" : "0");
  } catch {
    // Same as the view: a convenience only.
  }
}
