import { cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import { PublishQuickSetup } from "./PublishQuickSetup";
import type { PublishWizardProps } from "./PublishWizard";
import type { PublishDeviceGuards } from "../../types";
vi.mock("../../api", () => ({ publishImagePreview: vi.fn() }));
const row = { assignmentId: "pending-a", campaignId: "campaign-a", updatedAt: "2026-09-15T12:00:00Z", reason: "TikTok báo bài đang được xử lý" };
const guards: PublishDeviceGuards = { a: { blocking: [row], linkReview: [] }, b: { blocking: [], linkReview: [] } };
function props(): PublishWizardProps {
  const bundles = ["one", "two"].map(id => ({ id, name: id, sourcePath: id, mediaKind: "image" as const, images: [], captionPath: "caption", caption: "Caption", captionSha256: id, totalBytes: 1 }));
  return { sourceRoot: "fixture", manifest: { sourceRoot: "fixture", scannedAt: "now", bundles, notices: [], ignoredPartnerFiles: 0, ignoredHiddenFiles: 0 }, selectedIds: ["one"], assignments: { one: "a" }, captions: {}, devices: ["a", "b"].map(udid => ({ udid, name: udid, platform: "android", model: "test", osVersion: "9", connection: "usb", status: "ready", wdaReady: true })), metas: new Map(), eligible: ["a", "b"], busy: false, scanning: false, preflightLoading: false, preflight: null, preflightError: null, sound: { kind: "default" }, sheet: false, cleanup: false, runAt: "", onSource: vi.fn(), onScan: vi.fn(), onSelect: vi.fn(), onAssign: vi.fn(), onCaption: vi.fn(), onSheet: vi.fn(), onCleanup: vi.fn(), onRunAt: vi.fn(), onPreflight: vi.fn(), onExecute: vi.fn(), onHistory: vi.fn(), settings: null };
}
afterEach(cleanup);
it("shows source partner warnings without blocking valid publishing or changing source data", () => {
  const p = props();
  p.manifest!.notices = [{ severity: "warning", path: "fixture/one/partners.xlsx", message: "File đối tác rỗng; bài vẫn có thể đăng" }];
  render(<PublishQuickSetup {...p} />);
  fireEvent.click(screen.getByText("Cảnh báo nguồn (1)"));
  expect(screen.getByText("File đối tác rỗng; bài vẫn có thể đăng")).toBeVisible();
  expect(screen.getByText("fixture/one/partners.xlsx")).toBeVisible();
  expect(screen.getByRole("button", { name: "Kiểm tra & đăng" })).toBeEnabled();
  expect(p.onCaption).not.toHaveBeenCalled();
  expect(p.onExecute).not.toHaveBeenCalled();
});
it("warns about duplicate selected captions using edits without changing the manifest", () => {
  const p = props();
  const view = render(<PublishQuickSetup {...p} selectedIds={["one", "two"]} assignments={{ one: "a", two: "b" }} />);
  expect(screen.getByText(/2 bài đã chọn có caption trùng/)).toBeVisible();
  expect(screen.getByRole("button", { name: "Kiểm tra & đăng" })).toBeEnabled();
  view.rerender(<PublishQuickSetup {...p} selectedIds={["one", "two"]} assignments={{ one: "a", two: "b" }} captions={{ two: "Caption khác" }} />);
  expect(screen.queryByText(/caption trùng/)).toBeNull();
  expect(p.manifest!.bundles[1].caption).toBe("Caption");
  expect(p.onCaption).not.toHaveBeenCalled();
});
it("immediately names the blocked machine, disables posting and opens its exact pending campaign", () => {
  const p = props(), open = vi.fn(); const view = render(<PublishQuickSetup {...p} deviceGuards={guards} onPendingPublication={open} />);
  const card = screen.getByRole("checkbox", { name: "Chọn Máy 1 · a" }).closest("article")!;
  expect(within(card).getByText("Máy còn bài chưa lấy được link")).toBeVisible();
  expect(within(card).getByText(row.reason)).toBeVisible();
  expect(screen.getByRole("button", { name: "Kiểm tra & đăng" })).toBeEnabled();
  fireEvent.click(within(card).getByRole("button", { name: /Xem bài đang chờ/ })); expect(open).toHaveBeenCalledExactlyOnceWith("campaign-a");
  view.rerender(<PublishQuickSetup {...p} deviceGuards={{ ...guards, a: { blocking: [], linkReview: [] } }} />);
  expect(screen.getByRole("button", { name: "Kiểm tra & đăng" })).toBeEnabled();
});
it("quick assignment retains pending machines for explicit stop-before-preflight", () => {
  const p = props(), assign = vi.fn(); const view = render(<PublishQuickSetup {...p} selectedIds={[]} assignments={{}} deviceGuards={guards} onAssignmentChange={assign} />);
  fireEvent.click(screen.getByRole("button", { name: "Chọn nhanh" })); expect(assign).toHaveBeenCalledWith(["one", "two"], { one: "a", two: "b" });
  view.rerender(<PublishQuickSetup {...p} deviceGuards={{ ...guards, a: { blocking: [], linkReview: [row] } }} />);
  expect(screen.getByText("Bài cũ còn cần kiểm tra link")).toBeVisible(); expect(screen.getByRole("button", { name: "Kiểm tra & đăng" })).toBeEnabled();
});
it("unknown guard data keeps the existing assignment and prevents new posting", () => {
  const p = props(); render(<PublishQuickSetup {...p} deviceGuards={{}} />);
  expect(screen.getByRole("combobox", { name: "Máy nhận bài one" })).toHaveValue("a");
  expect(screen.getByRole("button", { name: "Kiểm tra & đăng" })).toBeDisabled(); expect(screen.getAllByText("Chưa kiểm tra được bài đang chờ").length).toBeGreaterThan(0);
});
