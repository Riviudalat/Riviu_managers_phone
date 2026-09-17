import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { useEffect } from "react";
import { PublishQuickSetup } from "./PublishQuickSetup";
import type { PublishWizardProps } from "./PublishWizard";
import type { PublishDeviceGuards } from "../../types";
import { ConfirmHost } from "../ConfirmHost";
import { resetConfirms } from "../../confirmStore";

vi.mock("../../api", () => ({ publishImagePreview: vi.fn().mockResolvedValue("data:image/png;base64,AA==") }));
type Props = PublishWizardProps & { deviceGuards?: PublishDeviceGuards };
function props(): Props {
  const bundles = ["one", "two"].map(id => ({ id, name: id, sourcePath: id, mediaKind: "image" as const, images: [], captionPath: "caption", caption: `Caption ${id}`, captionSha256: id, totalBytes: 1, partners: [`Đối tác ${id}`] }));
  return { sourceRoot: "fixture", manifest: { sourceRoot: "fixture", scannedAt: "now", bundles, notices: [], ignoredPartnerFiles: 0, ignoredHiddenFiles: 0 }, selectedIds: ["one", "two"], assignments: { two: "b" }, captions: {}, devices: ["a", "b"].map(udid => ({ udid, name: udid, platform: "android", model: "test", osVersion: "9", connection: "usb", status: "ready", wdaReady: true })), metas: new Map(), eligible: ["a", "b"], busy: false, scanning: false, preflightLoading: false, preflight: null, preflightError: null, sound: { kind: "default" }, sheet: false, cleanup: false, runAt: "", onSource: vi.fn(), onScan: vi.fn(), onSelect: vi.fn(), onAssign: vi.fn(), onCaption: vi.fn(), onSheet: vi.fn(), onCleanup: vi.fn(), onRunAt: vi.fn(), onPreflight: vi.fn(), onExecute: vi.fn(), onHistory: vi.fn(), settings: null };
}
function mount(p: Props) { return render(<><PublishQuickSetup {...p}/><ConfirmHost/></>); }
const machineSelect = (id: string) => screen.getByRole("combobox", { name: `Máy nhận bài ${id}` });
const confirmReplacement = async () => { await act(async () => { fireEvent.click(screen.getByRole("button", { name: "Thay bài trong bản nháp" })); }); };
beforeEach(() => {
  if (!HTMLDialogElement.prototype.showModal) Object.defineProperty(HTMLDialogElement.prototype, "showModal", { configurable: true, writable: true, value() {} });
  if (!HTMLDialogElement.prototype.close) Object.defineProperty(HTMLDialogElement.prototype, "close", { configurable: true, writable: true, value() {} });
  vi.spyOn(HTMLDialogElement.prototype, "showModal").mockImplementation(function (this: HTMLDialogElement) { this.setAttribute("open", ""); });
  vi.spyOn(HTMLDialogElement.prototype, "close").mockImplementation(function (this: HTMLDialogElement) { this.removeAttribute("open"); });
});
afterEach(() => { cleanup(); resetConfirms(); vi.restoreAllMocks(); });

it("giữ bài active bị lọc ẩn, chặn cả handler khung phải và cho phép dropdown tường minh", () => {
  const p = props(); mount(p);
  fireEvent.change(screen.getByRole("textbox", { name: "Tìm bài đăng" }), { target: { value: "two" } });
  const assign = screen.getByRole("button", { name: "Gán one · Máy 1 · a" });
  expect(assign).toBeDisabled();
  // Bỏ thuộc tính DOM để chứng minh guard nằm trong handler, không chỉ ở nút.
  assign.removeAttribute("disabled"); fireEvent.click(assign);
  expect(p.onAssign).not.toHaveBeenCalled();
  fireEvent.change(machineSelect("one"), { target: { value: "a" } });
  expect(p.onAssign).toHaveBeenCalledExactlyOnceWith({ one: "a", two: "b" });
  fireEvent.click(screen.getByRole("button", { name: "Hiện bài" }));
  expect(screen.getByRole("textbox", { name: "Tìm bài đăng" })).toHaveValue("");
  expect(screen.getByRole("button", { name: "Chọn bài đang gán · one" })).toHaveFocus();
});

it("đổi chỗ khi bài active có máy cũ hợp lệ, không mở confirm hoặc dispatch", () => {
  const p = props(); p.assignments.one = "a"; mount(p);
  fireEvent.click(screen.getByRole("button", { name: "Đổi chỗ one · Máy 2 · b" }));
  expect(p.onAssign).toHaveBeenCalledExactlyOnceWith({ one: "b", two: "a" });
  expect(screen.queryByRole("alertdialog")).toBeNull();
  expect(p.onPreflight).not.toHaveBeenCalled(); expect(p.onExecute).not.toHaveBeenCalled();
});

it.each(["missing", "offline", "outside", "unknown", "pending"])("máy cũ %s không được đổi chỗ: thay bài cần xác nhận và đưa bài cũ về chờ", async kind => {
  const p = props();
  if (kind !== "missing") p.assignments.one = "a";
  if (kind === "offline") p.devices[0].status = "disconnected";
  if (kind === "outside") p.eligible = ["b"];
  if (kind === "unknown") p.deviceGuards = { b: { blocking: [], linkReview: [] } };
  if (kind === "pending") p.deviceGuards = { a: { blocking: [{ assignmentId: "x", campaignId: "c", updatedAt: "now", reason: "Đang xử lý" }], linkReview: [] }, b: { blocking: [], linkReview: [] } };
  mount(p);
  fireEvent.click(screen.getByRole("button", { name: "Thay bài one · Máy 2 · b" }));
  expect(screen.getByRole("alertdialog")).toHaveTextContent("two");
  expect(screen.getByRole("alertdialog")).toHaveTextContent("chờ ghép máy");
  expect(p.onAssign).not.toHaveBeenCalled();
  await confirmReplacement();
  expect(p.onAssign).toHaveBeenCalledExactlyOnceWith({ one: "b" });
});

it("hủy thay bài giữ nguyên phân công", async () => {
  const p = props(); mount(p);
  fireEvent.change(machineSelect("one"), { target: { value: "b" } });
  await act(async () => { fireEvent.click(screen.getByRole("button", { name: "Hủy" })); });
  expect(p.onAssign).not.toHaveBeenCalled(); expect(machineSelect("one")).toHaveValue("");
});

it.each(["source", "manifest", "selected", "mapping", "scope", "ready", "pending", "unknown", "busy", "scanning", "preflight", "inactive"])("confirm cũ không mutate khi snapshot %s đổi", async change => {
  const p = props(), view = mount(p);
  fireEvent.change(machineSelect("one"), { target: { value: "b" } });
  const next = { ...p };
  if (change === "source") next.sourceRoot = "other";
  if (change === "manifest") next.manifest = { ...p.manifest!, scannedAt: "later" };
  if (change === "selected") next.selectedIds = ["one"];
  if (change === "mapping") next.assignments = { one: "a", two: "b" };
  if (change === "scope") next.eligible = ["b"];
  if (change === "ready") next.devices = p.devices.map(d => d.udid === "b" ? { ...d, status: "disconnected" } : d);
  if (change === "pending") next.deviceGuards = { b: { blocking: [{ assignmentId: "x", campaignId: "c", updatedAt: "now", reason: "Đang xử lý" }], linkReview: [] } };
  if (change === "unknown") next.deviceGuards = {};
  if (change === "busy") next.busy = true;
  if (change === "scanning") next.scanning = true;
  if (change === "preflight") next.preflightLoading = true;
  if (change === "inactive") next.active = false;
  view.rerender(<><PublishQuickSetup {...next}/><ConfirmHost/></>);
  await confirmReplacement(); expect(p.onAssign).not.toHaveBeenCalled();
});

it.each(["query", "active"])("confirm từ khung phải bị hủy khi %s đổi", async change => {
  const p = props(); mount(p);
  fireEvent.click(screen.getByRole("button", { name: "Thay bài one · Máy 2 · b" }));
  if (change === "query") fireEvent.change(screen.getByRole("textbox", { name: "Tìm bài đăng" }), { target: { value: "two" } });
  else fireEvent.click(screen.getByRole("button", { name: "Chọn bài đang gán · two" }));
  await confirmReplacement(); expect(p.onAssign).not.toHaveBeenCalled();
});

it("confirm dropdown tường minh không phụ thuộc từ khóa khung trái", async () => {
  const p = props(); mount(p);
  fireEvent.change(machineSelect("one"), { target: { value: "b" } });
  fireEvent.change(screen.getByRole("textbox", { name: "Tìm bài đăng" }), { target: { value: "two" } });
  await confirmReplacement(); expect(p.onAssign).toHaveBeenCalledExactlyOnceWith({ one: "b" });
});

it.each(["busy", "scanning", "preflightLoading"] as const)("%s khóa thao tác ngay cả khi DOM bị mở nút", field => {
  const p = props(); p[field] = true; mount(p);
  const assign = screen.getByRole("button", { name: "Gán one · Máy 1 · a" });
  assign.removeAttribute("disabled"); fireEvent.click(assign);
  const quick = screen.getByRole("button", { name: "Chọn nhanh" });
  quick.removeAttribute("disabled"); fireEvent.click(quick);
  fireEvent.change(machineSelect("one"), { target: { value: "a" } });
  expect(p.onAssign).not.toHaveBeenCalled(); expect(p.onSelect).not.toHaveBeenCalled();
});

it("bulk bỏ chọn nói rõ toàn bộ và tác động cả bài/máy ngoài search", () => {
  const p = props(); mount(p);
  fireEvent.change(screen.getByRole("textbox", { name: "Tìm bài đăng" }), { target: { value: "one" } });
  fireEvent.click(screen.getByRole("button", { name: "Bỏ chọn toàn bộ bài" }));
  expect(p.onSelect).toHaveBeenCalledWith([]);
  fireEvent.change(screen.getByRole("textbox", { name: "Tìm số máy" }), { target: { value: "a" } });
  fireEvent.click(screen.getByText("Bộ lọc thiết bị"));
  fireEvent.click(screen.getByRole("button", { name: "Bỏ chọn toàn bộ máy" }));
  expect(p.onAssign).toHaveBeenCalledWith({});
});

it.each(["left", "middle"])("dialog caption ghim bundle và trả focus đúng trigger %s", async origin => {
  const p = props(), view = mount(p);
  const trigger = screen.getByRole("button", { name: origin === "left" ? "Xem ảnh và sửa caption · one" : "Sửa caption · one" });
  trigger.focus(); fireEvent.click(trigger);
  expect(screen.getByRole("dialog", { name: "Ảnh & caption · one" })).toHaveTextContent("Đối tác one");
  // Thay thứ tự nguồn và active, không được chuyển nội dung đang sửa sang two.
  fireEvent.click(screen.getByRole("button", { name: "Chọn bài đang gán · two" }));
  view.rerender(<><PublishQuickSetup {...p} manifest={{ ...p.manifest!, bundles: [...p.manifest!.bundles].reverse() }}/><ConfirmHost/></>);
  fireEvent.change(screen.getByRole("textbox", { name: "Nội dung bài đăng" }), { target: { value: "Bản sửa" } });
  expect(p.onCaption).toHaveBeenCalledExactlyOnceWith("one", "Bản sửa");
  fireEvent.click(screen.getByRole("button", { name: "Đóng" }));
  await waitFor(() => expect(trigger).toHaveFocus());
});

it.each(["source", "removed"])("đóng caption khi %s đổi, không rơi sang bundle khác", async change => {
  const p = props(), view = mount(p);
  fireEvent.click(screen.getByRole("button", { name: "Sửa caption · one" }));
  view.rerender(<><PublishQuickSetup {...p} sourceRoot={change === "source" ? "other" : p.sourceRoot} manifest={change === "removed" ? { ...p.manifest!, bundles: [p.manifest!.bundles[1]] } : p.manifest}/><ConfirmHost/></>);
  expect(screen.queryByRole("dialog")).toBeNull(); expect(p.onCaption).not.toHaveBeenCalled();
  await waitFor(() => expect(screen.getByLabelText("Các cặp bài và máy")).toHaveFocus());
});

it("dialog giữ đủ slide và video metadata, không dựng preview giả", async () => {
  const p = props();
  p.manifest!.bundles[0].images = [0, 1, 2].map(i => ({ path: `slide-${i}`, fileName: `Ảnh ${i + 1}.jpg`, order: i, sha256: `hash-${i}`, byteLen: 1, width: 10, height: 10 }));
  const view = mount(p);
  fireEvent.click(screen.getByRole("button", { name: "Sửa caption · one" }));
  fireEvent.click(screen.getByRole("button", { name: "Ảnh tiếp" })); fireEvent.click(screen.getByRole("button", { name: "Ảnh tiếp" }));
  expect(await screen.findByRole("img", { name: "Ảnh 3.jpg" })).toBeVisible();
  expect(screen.getByRole("button", { name: "Ảnh tiếp" })).toBeDisabled();
  fireEvent.click(screen.getByRole("button", { name: "Đóng" }));
  const video = { ...p.manifest!.bundles[1], mediaKind: "video" as const, video: { path: "two.mp4", fileName: "two.mp4", sha256: "video", byteLen: 20, durationMs: 4000, videoCodec: "h264Avc" as const } };
  view.rerender(<><PublishQuickSetup {...p} manifest={{ ...p.manifest!, bundles: [video] }}/><ConfirmHost/></>);
  fireEvent.click(screen.getByRole("button", { name: "Sửa caption · two" }));
  expect(screen.getByRole("dialog")).toHaveTextContent("two.mp4");
  expect(screen.queryByRole("button", { name: "Ảnh tiếp" })).toBeNull();
});

it("giữ settings mounted qua dialog, tab ẩn và chỉnh filter", () => {
  const unmount = vi.fn();
  function Settings() { useEffect(() => () => unmount(), []); return <input aria-label="Link Google Sheet" defaultValue="draft"/>; }
  const p = props(); p.settings = <Settings/>; const view = mount(p);
  fireEvent.change(screen.getByRole("textbox", { name: "Link Google Sheet" }), { target: { value: "Đang nhập" } });
  fireEvent.click(screen.getByRole("button", { name: "Sửa caption · one" }));
  view.rerender(<><PublishQuickSetup {...p} active={false}/><ConfirmHost/></>);
  view.rerender(<><PublishQuickSetup {...p}/><ConfirmHost/></>);
  expect(screen.getByRole("textbox", { name: "Link Google Sheet" })).toHaveValue("Đang nhập"); expect(unmount).not.toHaveBeenCalled();
});

it("giữ mapping offline/ra scope, bỏ tick máy mới gỡ cặp", () => {
  const p = props(); p.devices[1].status = "disconnected"; p.eligible = ["a"]; mount(p);
  expect(machineSelect("two")).toHaveValue("b");
  expect(screen.getByRole("button", { name: "Kiểm tra & đăng" })).toBeDisabled();
  expect(p.onAssign).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole("checkbox", { name: "Chọn Máy 2 · b" }));
  expect(p.onAssign).toHaveBeenCalledExactlyOnceWith({});
});
