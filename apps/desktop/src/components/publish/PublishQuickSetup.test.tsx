import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { deviceThreadsAccount, launchDeviceApp } from "../../api";
import { PublishQuickSetup } from "./PublishQuickSetup";
import type { PublishWizardProps } from "./PublishWizard";
import type { PublishDeviceGuards } from "../../types";
vi.mock("../../api", () => ({ publishImagePreview: vi.fn(), launchDeviceApp: vi.fn(), deviceThreadsAccount: vi.fn() }));
const row = { assignmentId: "pending-a", campaignId: "campaign-a", updatedAt: "2026-09-15T12:00:00Z", reason: "TikTok báo bài đang được xử lý" };
const guards: PublishDeviceGuards = { a: { blocking: [row], linkReview: [] }, b: { blocking: [], linkReview: [] } };
function props(): PublishWizardProps {
  const bundles = ["one", "two"].map(id => ({ id, name: id, sourcePath: id, mediaKind: "image" as const, images: [], captionPath: "caption", caption: "Caption", captionSha256: id, totalBytes: 1 }));
  return { sourceRoot: "fixture", manifest: { sourceRoot: "fixture", scannedAt: "now", bundles, notices: [], ignoredPartnerFiles: 0, ignoredHiddenFiles: 0 }, selectedIds: ["one"], assignments: { one: "a" }, captions: {}, devices: ["a", "b"].map(udid => ({ udid, name: udid, platform: "android", model: "test", osVersion: "9", connection: "usb", status: "ready", wdaReady: true })), metas: new Map(), eligible: ["a", "b"], busy: false, scanning: false, preflightLoading: false, preflight: null, preflightError: null, sound: { kind: "default" }, sheet: false, cleanup: false, runAt: "", onSource: vi.fn(), onScan: vi.fn(), onSelect: vi.fn(), onAssign: vi.fn(), onCaption: vi.fn(), onSheet: vi.fn(), onCleanup: vi.fn(), onRunAt: vi.fn(), onPreflight: vi.fn(), onExecute: vi.fn(), onHistory: vi.fn(), settings: null };
}
beforeEach(() => {
  vi.mocked(deviceThreadsAccount).mockReset();
  if (!HTMLDialogElement.prototype.showModal) Object.defineProperty(HTMLDialogElement.prototype, "showModal", { configurable: true, writable: true, value() {} });
  if (!HTMLDialogElement.prototype.close) Object.defineProperty(HTMLDialogElement.prototype, "close", { configurable: true, writable: true, value() {} });
  vi.spyOn(HTMLDialogElement.prototype, "showModal").mockImplementation(function (this: HTMLDialogElement) { this.setAttribute("open", ""); });
  vi.spyOn(HTMLDialogElement.prototype, "close").mockImplementation(function (this: HTMLDialogElement) { this.removeAttribute("open"); });
});
afterEach(() => { cleanup(); vi.restoreAllMocks(); });
it("opens Threads only on the chosen Android device without starting publication", async () => {
  vi.mocked(launchDeviceApp).mockResolvedValue(undefined);
  const p = props();
  const view = render(<PublishQuickSetup {...p} network="threads" />);
  fireEvent.click(screen.getByRole("button", { name: "Mở Threads trên Máy 2 · b" }));
  await waitFor(() => expect(launchDeviceApp).toHaveBeenCalledExactlyOnceWith("b", "com.instagram.barcelona"));
  expect(p.onPreflight).not.toHaveBeenCalled();
  expect(p.onExecute).not.toHaveBeenCalled();
  view.rerender(<PublishQuickSetup {...p} network="tiktok" />);
  expect(screen.queryByRole("button", { name: /Mở Threads trên/ })).toBeNull();
});
it("checks the selected phone's real Threads account without publishing", async () => {
  vi.mocked(deviceThreadsAccount).mockResolvedValue({ packageName: "com.instagram.barcelona", version: "446.0.0.32.78", locale: "en", account: "test.account", publishSupported: true });
  const p = props();
  render(<PublishQuickSetup {...p} network="threads" />);
  fireEvent.click(screen.getByRole("button", { name: "Kiểm tra Threads trên Máy 2 · b" }));
  await waitFor(() => expect(screen.getByText("Đã đăng nhập · @test.account")).toBeVisible());
  expect(deviceThreadsAccount).toHaveBeenCalledExactlyOnceWith("b");
  expect(p.onPreflight).not.toHaveBeenCalled();
  expect(p.onExecute).not.toHaveBeenCalled();
});
it("checks all connected Threads logins before any posts are selected", async () => {
  vi.mocked(deviceThreadsAccount).mockImplementation(async udid => {
    if (udid === "b") throw new Error("Threads account is not signed in");
    return { packageName: "com.instagram.barcelona", version: "446.0.0.32.78", locale: "en", account: "ready.account", publishSupported: true };
  });
  const p = props();
  render(<PublishQuickSetup {...p} network="threads" manifest={null} scanning={true} selectedIds={[]} assignments={{}} />);
  fireEvent.click(screen.getByRole("button", { name: "Kiểm tra đăng nhập Threads" }));
  await waitFor(() => expect(screen.getByText("1 đã đăng nhập · 1 hỗ trợ đăng / 2 đã kiểm tra")).toBeVisible());
  expect(deviceThreadsAccount).toHaveBeenNthCalledWith(1, "a");
  expect(deviceThreadsAccount).toHaveBeenNthCalledWith(2, "b");
  expect(screen.getByText("Đã đăng nhập · @ready.account")).toBeVisible();
  expect(screen.getByText("Chưa đăng nhập Threads")).toBeVisible();
  expect(p.onAssign).not.toHaveBeenCalled();
  expect(p.onPreflight).not.toHaveBeenCalled();
  expect(p.onExecute).not.toHaveBeenCalled();
});
it("shows a signed-in account on an unmeasured build without offering it for automatic posting", async () => {
  vi.mocked(deviceThreadsAccount).mockResolvedValue({ packageName: "com.instagram.barcelona", version: "447.0.0.0.0", locale: "en", account: "other.account", publishSupported: false });
  const p = props(), assign = vi.fn();
  render(<PublishQuickSetup {...p} network="threads" selectedIds={[]} assignments={{}} onAssignmentChange={assign} />);
  fireEvent.click(screen.getByRole("button", { name: "Kiểm tra đăng nhập Threads" }));
  await waitFor(() => expect(screen.getByText("2 đã đăng nhập · 0 hỗ trợ đăng / 2 đã kiểm tra")).toBeVisible());
  expect(screen.getAllByText(/@other.account · Threads 447.0.0.0.0 \(en\) chưa hỗ trợ đăng/)).toHaveLength(2);
  fireEvent.click(screen.getByRole("button", { name: "Chọn nhanh" }));
  await waitFor(() => expect(deviceThreadsAccount).toHaveBeenCalledTimes(4));
  expect(assign).not.toHaveBeenCalled();
});
it("quick assigns only freshly verified Threads accounts without pending posts", async () => {
  vi.mocked(deviceThreadsAccount).mockResolvedValue({ packageName: "com.instagram.barcelona", version: "446.0.0.32.78", locale: "en", account: "ready.account", publishSupported: true });
  const p = props(), assign = vi.fn();
  render(<PublishQuickSetup {...p} network="threads" selectedIds={[]} assignments={{}} deviceGuards={guards} onAssignmentChange={assign} />);
  fireEvent.click(screen.getByRole("button", { name: "Chọn nhanh" }));
  await waitFor(() => expect(assign).toHaveBeenCalledExactlyOnceWith(["one"], { one: "b" }));
  expect(deviceThreadsAccount).toHaveBeenCalledExactlyOnceWith("b");
  expect(p.onExecute).not.toHaveBeenCalled();
});
it("reuses released Threads phones with old link review while preserving the review", async () => {
  vi.mocked(deviceThreadsAccount).mockImplementation(async udid => ({ packageName: "com.instagram.barcelona", version: "446.0.0.32.78", locale: "en", account: `${udid}.account`, publishSupported: true }));
  const p = props(), assign = vi.fn(), open = vi.fn();
  const released = { a: { blocking: [], linkReview: [row] }, b: { blocking: [], linkReview: [] } };
  render(<PublishQuickSetup {...p} network="threads" selectedIds={[]} assignments={{}} deviceGuards={released} onAssignmentChange={assign} onPendingPublication={open} />);
  fireEvent.click(screen.getByRole("button", { name: "Chọn nhanh" }));
  await waitFor(() => expect(assign).toHaveBeenCalledExactlyOnceWith(["one", "two"], { one: "a", two: "b" }));
  expect(screen.getByText("Bài cũ còn cần kiểm tra link")).toBeVisible();
  fireEvent.click(screen.getByRole("button", { name: /Xem bài đang chờ/ }));
  expect(open).toHaveBeenCalledWith("campaign-a");
  expect(released.a.linkReview).toEqual([row]);
  expect(p.onExecute).not.toHaveBeenCalled();
});
it("quick assignment leaves posts unassigned when Threads account cannot be verified", async () => {
  vi.mocked(deviceThreadsAccount).mockRejectedValue(new Error("Threads chưa cài"));
  const p = props(), assign = vi.fn();
  render(<PublishQuickSetup {...p} network="threads" selectedIds={[]} assignments={{}} onAssignmentChange={assign} />);
  fireEvent.click(screen.getByRole("button", { name: "Chọn nhanh" }));
  await waitFor(() => expect(screen.getAllByText("Chưa xác minh: Threads chưa cài")).toHaveLength(2));
  expect(assign).not.toHaveBeenCalled();
  expect(p.onExecute).not.toHaveBeenCalled();
});
it("quick assignment uses only one machine per Threads account", async () => {
  vi.mocked(deviceThreadsAccount).mockResolvedValue({ packageName: "com.instagram.barcelona", version: "446.0.0.32.78", locale: "en", account: "shared.account", publishSupported: true });
  const p = props(), assign = vi.fn();
  render(<PublishQuickSetup {...p} network="threads" selectedIds={[]} assignments={{}} onAssignmentChange={assign} />);
  fireEvent.click(screen.getByRole("button", { name: "Chọn nhanh" }));
  await waitFor(() => expect(assign).toHaveBeenCalledExactlyOnceWith(["one"], { one: "a" }));
  expect(deviceThreadsAccount).toHaveBeenCalledTimes(2);
});
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
it("lists assigned machines as awaiting a result while preflight is running without implying posting started", () => {
  const p = props();
  const view = render(<PublishQuickSetup {...p} selectedIds={["one", "two"]} assignments={{ one: "a", two: "b" }} />);
  fireEvent.click(screen.getByRole("button", { name: "Kiểm tra & đăng" }));
  view.rerender(<PublishQuickSetup {...p} selectedIds={["one", "two"]} assignments={{ one: "a", two: "b" }} preflightLoading />);
  const pending = screen.getByRole("region", { name: "Máy đang chờ kết quả kiểm tra" });
  expect(within(pending).getByText("Máy 1 · a")).toBeVisible();
  expect(within(pending).getByText("Máy 2 · b")).toBeVisible();
  expect(screen.getByRole("button", { name: "Xác nhận đăng 2 bài" })).toBeDisabled();
  view.rerender(<PublishQuickSetup {...p} selectedIds={["one", "two"]} assignments={{ one: "a", two: "b" }} preflightLoading preflightStage="preparing" />);
  expect(screen.getByText("Đang chuẩn bị thiết bị…")).toBeVisible();
  view.rerender(<PublishQuickSetup {...p} selectedIds={["one", "two"]} assignments={{ one: "a", two: "b" }} preflightLoading preflightStage="checking" />);
  expect(screen.getByText("Đang kiểm tra từng máy…")).toBeVisible();
  expect(p.onExecute).not.toHaveBeenCalled();
});
